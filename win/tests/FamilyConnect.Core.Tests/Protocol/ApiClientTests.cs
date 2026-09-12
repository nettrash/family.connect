using System.Net;
using System.Net.Http.Headers;
using System.Text;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.Core.Tests.Protocol;

/// <summary>
/// The REST transport: the token on every call, the error shape read as the protocol writes it,
/// and ONE retry on a read that hit a transient status (docs/protocol.md, "Transport",
/// "Error shape").
/// </summary>
/// <remarks>
/// Driven through a fake handler, which is what makes the transport testable at all — no server
/// and no network, and every request the client made is kept for inspection.
/// </remarks>
public class ApiClientTests
{
    /// <summary>Answers a queue of replies and remembers every request it was given.</summary>
    private sealed class Fake : HttpMessageHandler
    {
        private readonly Queue<Func<HttpRequestMessage, HttpResponseMessage>> replies = new();

        public List<HttpRequestMessage> Sent { get; } = [];
        public List<string?> Bodies { get; } = [];

        public Fake Then(HttpStatusCode status, string? json = null, string? retryAfter = null)
        {
            replies.Enqueue(_ =>
            {
                var response = new HttpResponseMessage(status);
                if (json is not null)
                {
                    response.Content = new StringContent(json, Encoding.UTF8, "application/json");
                }
                if (retryAfter is not null)
                {
                    response.Headers.TryAddWithoutValidation("Retry-After", retryAfter);
                }
                return response;
            });
            return this;
        }

        /// <summary>A request that never got an answer: the transport failure case.</summary>
        public Fake ThenUnreachable()
        {
            replies.Enqueue(_ => throw new HttpRequestException("connection reset"));
            return this;
        }

        protected override async Task<HttpResponseMessage> SendAsync(
            HttpRequestMessage request, CancellationToken cancellationToken)
        {
            Sent.Add(request);
            Bodies.Add(request.Content is null
                ? null
                : await request.Content.ReadAsStringAsync(cancellationToken));
            if (replies.Count == 0)
            {
                throw new InvalidOperationException($"no reply queued for {request.RequestUri}");
            }
            return replies.Dequeue()(request);
        }
    }

    private static (ApiClient Client, Fake Handler) Client(Fake handler, string? token = "t0ken")
    {
        var http = new HttpClient(handler);
        var url = ServerUrl.Normalise("chat.example.com")!;
        return (new ApiClient(http, url, new MemoryTokenStore(token)), handler);
    }

    [Fact]
    public async Task EveryCallGoesUnderApiV1WithTheBearerToken()
    {
        var (client, handler) = Client(new Fake().Then(HttpStatusCode.OK, """{"notes": [], "max_board_seq": 0}"""));
        var answer = await client.Board();
        Assert.True(answer.Ok);
        var request = Assert.Single(handler.Sent);
        Assert.Equal("https://chat.example.com/api/v1/families/mine/board", request.RequestUri?.ToString());
        Assert.Equal("Bearer", request.Headers.Authorization?.Scheme);
        Assert.Equal("t0ken", request.Headers.Authorization?.Parameter);
        // And the token is NOWHERE in the URL: "a token in the query string is not a token at all".
        Assert.DoesNotContain("t0ken", request.RequestUri!.ToString(), StringComparison.Ordinal);
    }

    [Fact]
    public async Task SigningInSendsNoTokenBecauseThereIsNotOneYet()
    {
        var (client, handler) = Client(
            new Fake().Then(HttpStatusCode.OK,
                """{"token": "fresh", "user": {"id": 7, "username": "anna", "display_name": "Anna"}}"""),
            token: null);
        var answer = await client.LogIn("anna", "hunter2");
        Assert.True(answer.Ok);
        Assert.Equal("fresh", answer.Value!.Token);
        Assert.Equal("Anna", answer.Value.User.DisplayName);
        Assert.Null(Assert.Single(handler.Sent).Headers.Authorization);
        Assert.Contains("\"username\":\"anna\"", Assert.Single(handler.Bodies), StringComparison.Ordinal);
    }

    [Fact]
    public async Task A204IsASuccessWithNothingInIt()
    {
        var (client, _) = Client(new Fake().Then(HttpStatusCode.NoContent));
        var answer = await client.DeleteNote(12);
        Assert.True(answer.Ok);
        Assert.Null(answer.Error);
    }

    [Fact]
    public async Task TheErrorShapeIsReadAsTheProtocolWritesIt()
    {
        var (client, _) = Client(new Fake().Then(
            HttpStatusCode.Forbidden,
            """{"error": {"code": "pictures_unavailable", "message": "this server cannot draw"}}"""));
        var answer = await client.DrawBackdrop(12);
        Assert.False(answer.Ok);
        Assert.Equal(ErrorCodes.PicturesUnavailable, answer.Error!.Code);
        Assert.Equal("this server cannot draw", answer.Error.Message);
        Assert.Equal(403, answer.Error.Status);
        // A refusal, so nothing retries it.
        Assert.False(answer.Error.Transient);
    }

    /// <summary>
    /// nginx answers its own rate limit with an HTML body, so the status alone has to be enough.
    /// </summary>
    [Fact]
    public async Task A429WithNoJsonAtAllIsStillATransientFailureWithItsWait()
    {
        var (client, _) = Client(new Fake()
            .Then(HttpStatusCode.TooManyRequests, "<html>429 Too Many Requests</html>", retryAfter: "2")
            .Then(HttpStatusCode.TooManyRequests, "<html>429 Too Many Requests</html>", retryAfter: "2"));
        var answer = await client.Board();
        Assert.False(answer.Ok);
        Assert.Equal(ErrorCodes.TooManyRequests, answer.Error!.Code);
        Assert.True(answer.Error.Transient);
        Assert.Equal(TimeSpan.FromSeconds(2), answer.Error.RetryAfter);
    }

    /// <summary>
    /// ONE retry, and only on a read: a GET is safe to repeat, and the alternative is a board
    /// that empties itself because a proxy hiccuped.
    /// </summary>
    [Fact]
    public async Task AReadIsTriedOnceMoreAfterATransientFailure()
    {
        var (client, handler) = Client(new Fake()
            .Then(HttpStatusCode.BadGateway, "<html>502</html>")
            .Then(HttpStatusCode.OK, """{"notes": [], "max_board_seq": 4}"""));
        var answer = await client.Board();
        Assert.True(answer.Ok);
        Assert.Equal(4, answer.Value!.MaxBoardSeq);
        Assert.Equal(2, handler.Sent.Count);
    }

    [Fact]
    public async Task AndOnlyOnce()
    {
        var (client, handler) = Client(new Fake()
            .Then(HttpStatusCode.BadGateway, "<html>502</html>")
            .Then(HttpStatusCode.BadGateway, "<html>502</html>"));
        var answer = await client.Board();
        Assert.False(answer.Ok);
        Assert.True(answer.Error!.Transient);
        Assert.Equal(2, handler.Sent.Count);
    }

    /// <summary>
    /// A WRITE is never retried here, whatever went wrong: only the outbox knows whether the
    /// request carries a dedup key, and a retried POST without one is a second message.
    /// </summary>
    [Fact]
    public async Task AWriteIsNeverRetried()
    {
        var (client, handler) = Client(new Fake().Then(HttpStatusCode.BadGateway, "<html>502</html>"));
        var answer = await client.SendMessage(42, "8f14e45f-…", "Dinner at 7?");
        Assert.False(answer.Ok);
        Assert.True(answer.Error!.Transient, "still transient — the outbox will try again");
        Assert.Single(handler.Sent);
    }

    [Fact]
    public async Task ARequestThatNeverArrivedIsATransportFailure()
    {
        var (client, handler) = Client(new Fake().ThenUnreachable().ThenUnreachable());
        var answer = await client.Board();
        Assert.False(answer.Ok);
        Assert.Equal(ErrorCodes.Transport, answer.Error!.Code);
        Assert.True(answer.Error.Transient);
        // A read: tried once more, and then reported.
        Assert.Equal(2, handler.Sent.Count);
    }

    [Fact]
    public async Task ASendCarriesItsDedupKeyAndOnlyTheFieldsThatApply()
    {
        var (client, handler) = Client(new Fake().Then(
            HttpStatusCode.Created,
            """{"message": {"id": 1338, "chat_id": 42, "sender_id": 7, "client_msg_id": "k", "body": "hi", "created_at": "…"}}"""));
        var answer = await client.SendMessage(42, "k", "hi");
        Assert.True(answer.Ok);
        Assert.Equal(1338, answer.Value!.Message.Id);
        var body = Assert.Single(handler.Bodies)!;
        Assert.Contains("\"client_msg_id\":\"k\"", body, StringComparison.Ordinal);
        // Absent, not null: a field this message does not have is a field the server never sees.
        Assert.DoesNotContain("reply_to_message_id", body, StringComparison.Ordinal);
        Assert.DoesNotContain("attachment_ids", body, StringComparison.Ordinal);
        Assert.DoesNotContain("poll", body, StringComparison.Ordinal);
        Assert.DoesNotContain("mentions", body, StringComparison.Ordinal);
    }

    [Fact]
    public async Task APatchSendsOnlyWhatChanged()
    {
        var (client, handler) = Client(new Fake().Then(
            HttpStatusCode.OK,
            """{"note": {"id": 12, "author_id": 7, "text": "Milk", "board_seq": 9}}"""));
        var answer = await client.PatchNote(12, new NotePatch(X: 0.4, Y: 0.6));
        Assert.True(answer.Ok);
        var body = Assert.Single(handler.Bodies)!;
        Assert.Equal("{\"x\":0.4,\"y\":0.6}", body);
        Assert.Equal(HttpMethod.Patch, Assert.Single(handler.Sent).Method);
    }

    [Fact]
    public async Task ANewListSendsItsLinesWithoutIds()
    {
        var (client, handler) = Client(new Fake().Then(
            HttpStatusCode.Created,
            """{"note": {"id": 13, "author_id": 7, "kind": "tasks", "text": "Saturday", "board_seq": 10, "items": []}}"""));
        var answer = await client.CreateNote(new NoteRequest(
            "Saturday", "green", 0.1, 0.1, Kind: "tasks",
            Items: [new TaskLineRequest("Milk"), new TaskLineRequest("Bread")]));
        Assert.True(answer.Ok);
        var body = Assert.Single(handler.Bodies)!;
        Assert.Contains("\"items\":[{\"text\":\"Milk\"},{\"text\":\"Bread\"}]", body, StringComparison.Ordinal);
        // "An `id` on a created item is refused: ids are the server's."
        Assert.DoesNotContain("\"id\"", body, StringComparison.Ordinal);
    }

    [Fact]
    public async Task ARewrittenLineKeepsItsIdAndThereforeItsTick()
    {
        var (client, handler) = Client(new Fake().Then(
            HttpStatusCode.OK,
            """{"note": {"id": 13, "author_id": 7, "kind": "tasks", "text": "Saturday", "board_seq": 11}}"""));
        await client.PatchNote(13, new NotePatch(
            Items: [new TaskLineRequest("Milk and eggs", Id: 11), new TaskLineRequest("Jam")]));
        var body = Assert.Single(handler.Bodies)!;
        Assert.Contains("{\"text\":\"Milk and eggs\",\"id\":11}", body, StringComparison.Ordinal);
        Assert.Contains("{\"text\":\"Jam\"}", body, StringComparison.Ordinal);
    }

    [Fact]
    public async Task TheBackdropAsksWithNoBodyAtAll()
    {
        var (client, handler) = Client(new Fake().Then(
            HttpStatusCode.OK,
            """{"note": {"id": 12, "author_id": 7, "kind": "event", "text": "Lunch", "board_seq": 12}}"""));
        var answer = await client.DrawBackdrop(12);
        Assert.True(answer.Ok);
        // The prompt is the note's TITLE: a request body would be a second way to send words to a
        // model from a screen that is not the assistant's chat.
        Assert.Null(Assert.Single(handler.Bodies));
        Assert.Equal(
            "https://chat.example.com/api/v1/families/mine/board/notes/12/backdrop",
            Assert.Single(handler.Sent).RequestUri?.ToString());
    }

    [Fact]
    public async Task TicksAndAnswersAreStatesAndNotToggles()
    {
        var (client, handler) = Client(new Fake()
            .Then(HttpStatusCode.OK, """{"note": {"id": 13, "author_id": 7, "board_seq": 13}}""")
            .Then(HttpStatusCode.OK, """{"note": {"id": 12, "author_id": 7, "board_seq": 14}}"""));
        await client.TickTask(13, 11, done: true);
        await client.Answer(12, "going");
        Assert.Equal("{\"done\":true}", handler.Bodies[0]);
        Assert.Equal("{\"answer\":\"going\"}", handler.Bodies[1]);
        Assert.Equal(HttpMethod.Put, handler.Sent[0].Method);
        Assert.Equal(
            "https://chat.example.com/api/v1/families/mine/board/notes/13/tasks/11",
            handler.Sent[0].RequestUri?.ToString());
    }

    [Fact]
    public async Task AnUploadPutsItsBytesInTheBodyAndItsFactsInTheQuery()
    {
        var (client, handler) = Client(new Fake().Then(
            HttpStatusCode.Created,
            """{"attachment": {"id": 34, "kind": "photo", "mime": "image/jpeg", "width": 600, "height": 1200}}"""));
        var answer = await client.Upload("photo", "image/jpeg", new byte[] { 1, 2, 3 }, 600, 1200);
        Assert.True(answer.Ok);
        Assert.Equal(0.5, answer.Value!.Attachment.AspectRatio);
        var request = Assert.Single(handler.Sent);
        Assert.Equal(
            "https://chat.example.com/api/v1/attachments?kind=photo&width=600&height=1200",
            request.RequestUri?.ToString());
        Assert.Equal("image/jpeg", request.Content?.Headers.ContentType?.MediaType);
    }

    [Fact]
    public async Task AnAttachmentComesBackAsBytes()
    {
        var handler = new Fake();
        handler.Then(HttpStatusCode.OK, null);
        var http = new HttpClient(handler);
        var client = new ApiClient(
            http, ServerUrl.Normalise("chat.example.com")!, new MemoryTokenStore("t0ken"));
        var answer = await client.Download(34, preview: true);
        Assert.True(answer.Ok);
        Assert.Equal(
            "https://chat.example.com/api/v1/attachments/34/preview",
            Assert.Single(handler.Sent).RequestUri?.ToString());
    }

    /// <summary>
    /// A 401 sends the client back to login — EXCEPT <c>invalid_credentials</c>, which is a
    /// password that was wrong and leaves the session alone.
    /// </summary>
    [Fact]
    public async Task AnExpiredSessionAndAWrongPasswordAreTheSameStatusAndNotTheSameThing()
    {
        var (client, _) = Client(new Fake().Then(
            HttpStatusCode.Unauthorized,
            """{"error": {"code": "unauthorized", "message": "session gone"}}"""));
        var gone = await client.Me();
        Assert.True(gone.Error!.SessionGone);

        var (again, _) = Client(new Fake().Then(
            HttpStatusCode.Unauthorized,
            """{"error": {"code": "invalid_credentials", "message": "wrong password"}}"""), token: null);
        var wrong = await again.LogIn("anna", "nope");
        Assert.False(wrong.Error!.SessionGone);
    }

    [Fact]
    public async Task ThisServersCapabilitiesArriveWithTheAnswerRatherThanAsErrorsLater()
    {
        var (client, _) = Client(new Fake().Then(HttpStatusCode.OK, """
            {"family": {"id": 3, "name": "The Smiths", "join_policy": "open", "ai_history": true},
             "members": [{"id": 7, "username": "anna", "display_name": "Anna", "owner": true}],
             "blocked_user_ids": [],
             "assistant": {"user_id": 1, "display_name": "Assistant", "mention": "@ai",
                           "draw": "/draw", "vision": true, "images": true}}
            """));
        var answer = await client.Family();
        Assert.True(answer.Ok);
        Assert.Equal("The Smiths", answer.Value!.Family.Name);
        Assert.True(answer.Value.HasAssistant);
        // What the board's backdrop button hangs on.
        Assert.True(answer.Value.CanDrawPictures);
        Assert.Empty(answer.Value.BlockedUserIds!);
        // The owner's cap is ABSENT here, which is not the same as the operator's ceiling.
        Assert.Null(answer.Value.Family.MaxMembers);
    }

    [Fact]
    public async Task AServerWithNoAssistantOffersNone()
    {
        var (client, _) = Client(new Fake().Then(
            HttpStatusCode.OK, """{"family": {"id": 3, "name": "The Smiths"}}"""));
        var answer = await client.Family();
        Assert.True(answer.Ok);
        Assert.False(answer.Value!.HasAssistant);
        Assert.False(answer.Value.CanDrawPictures);
    }

    [Fact]
    public async Task A2xxWhoseBodyCannotBeReadIsNotASuccess()
    {
        var (client, _) = Client(new Fake().Then(HttpStatusCode.OK, "<html>a proxy's idea of JSON</html>"));
        var answer = await client.Board();
        Assert.False(answer.Ok);
        // Terminal: repeating the call produces the same body.
        Assert.False(answer.Error!.Transient);
    }

    [Fact]
    public async Task TheSocketUrlFollowsTheSameServer()
    {
        var (client, _) = Client(new Fake());
        Assert.Equal("wss://chat.example.com/api/v1/ws", client.SocketUrl.ToString());
        Assert.Equal("https://chat.example.com/", client.BaseUrl.ToString());
    }
}
