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

        /// <summary>Each request body's own type header, read while it is still open — a multipart one carries its boundary.</summary>
        public List<MediaTypeHeaderValue?> BodyTypes { get; } = [];

        /// <summary>Each multipart body's parts, read while it is still open: name, file name, type and bytes.</summary>
        public List<List<(string? Name, string? FileName, string? Type, byte[] Bytes)>> Parts { get; } = [];

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

        /// <summary>How long each request waits before it is answered, in order; a missing entry answers at once.</summary>
        public Queue<TimeSpan> Delays { get; } = new();

        protected override async Task<HttpResponseMessage> SendAsync(
            HttpRequestMessage request, CancellationToken cancellationToken)
        {
            Sent.Add(request);
            Bodies.Add(request.Content is null
                ? null
                : await request.Content.ReadAsStringAsync(cancellationToken));
            BodyTypes.Add(request.Content?.Headers.ContentType);
            if (request.Content is MultipartContent multipart)
            {
                var parts = new List<(string?, string?, string?, byte[])>();
                foreach (var part in multipart)
                {
                    parts.Add((
                        part.Headers.ContentDisposition?.Name,
                        part.Headers.ContentDisposition?.FileName,
                        part.Headers.ContentType?.MediaType,
                        await part.ReadAsByteArrayAsync(cancellationToken)));
                }
                Parts.Add(parts);
            }
            if (Delays.TryDequeue(out var delay))
            {
                // A slow server: the deadline, when it passes, cancels this wait exactly as it would the socket.
                await Task.Delay(delay, cancellationToken);
            }
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

    /// <summary>
    /// A backdrop the author has not agreed to (docs/protocol.md, "Consenting to the assistant", amended 2026-09-30):
    /// a refusal with nothing sent, read as its code — the one the board answers with the consent question.
    /// </summary>
    [Fact]
    public async Task ABackdropWithoutConsentIsRefusedByItsCode()
    {
        var (client, handler) = Client(new Fake().Then(
            HttpStatusCode.Forbidden,
            """{"error": {"code": "assistant_consent_required", "message": "agree first"}}"""));
        var answer = await client.DrawBackdrop(12);
        Assert.False(answer.Ok);
        Assert.Equal(ErrorCodes.AssistantConsentRequired, answer.Error!.Code);
        Assert.Equal(403, answer.Error.Status);
        Assert.False(answer.Error.Transient);
        // A write, and a refusal: asked once, never repeated here.
        Assert.Single(handler.Sent);
    }

    /// <summary>
    /// SLOW (docs/protocol.md, "Board", amended 2026-09-30): the backdrop gets "a timeout of its own, no shorter than
    /// 90 s … never its ordinary request timeout" — and the HttpClient the app builds must not cap it, because
    /// <see cref="HttpClient.Timeout"/> applies to every request sent through it.
    /// </summary>
    [Fact]
    public void TheBackdropHasADeadlineOfItsOwnThatNothingCaps()
    {
        Assert.True(ApiClient.BackdropTimeout >= TimeSpan.FromSeconds(90));
        Assert.True(ApiClient.BackdropTimeout > ApiClient.OrdinaryTimeout);
        using var http = ApiClient.NewHttpClient();
        Assert.Equal(Timeout.InfiniteTimeSpan, http.Timeout);
        var client = new ApiClient(http, ServerUrl.Normalise("chat.example.com")!, new MemoryTokenStore("t0ken"));
        Assert.Equal(ApiClient.BackdropTimeout, client.BackdropDeadline);
        Assert.Equal(ApiClient.OrdinaryTimeout, client.RequestDeadline);
    }

    /// <summary>
    /// The same rule, run: a server slower than the ORDINARY deadline fails an ordinary request, and still answers a
    /// backdrop — the backdrop does not ride on the timeout every other call gets.
    /// </summary>
    [Fact]
    public async Task ABackdropOutlastsTheOrdinaryDeadline()
    {
        // One reply only: a request whose deadline passes never takes its reply off the queue.
        var handler = new Fake()
            .Then(HttpStatusCode.OK,
                """{"note": {"id": 12, "author_id": 7, "kind": "event", "text": "Lunch", "board_seq": 12}}""");
        var slow = TimeSpan.FromMilliseconds(400);
        handler.Delays.Enqueue(slow);
        handler.Delays.Enqueue(slow);
        handler.Delays.Enqueue(slow);
        var client = new ApiClient(
            new HttpClient(handler), ServerUrl.Normalise("chat.example.com")!, new MemoryTokenStore("t0ken"))
        {
            RequestDeadline = TimeSpan.FromMilliseconds(50),
            BackdropDeadline = TimeSpan.FromSeconds(30),
        };
        // A read, so it is tried twice — and both run out of time.
        var board = await client.Board();
        Assert.False(board.Ok);
        Assert.Equal(ErrorCodes.Transport, board.Error!.Code);
        var drawn = await client.DrawBackdrop(12);
        Assert.True(drawn.Ok);
        Assert.Equal(12, drawn.Value!.Note.Id);
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

    /// <summary>
    /// A voice note's waveform (docs/protocol.md, "A voice note's waveform") rides in the query on AUDIO only, and only
    /// when it is exactly the wire; the answer's echo lands on the attachment, and an answer without one (a server from
    /// before waveforms) is still a success.
    /// </summary>
    [Fact]
    public async Task AVoiceNotesWaveformGoesInTheQueryOnAudioOnly()
    {
        const string wire = "0123456789abcdef0123456789abcdef0123456789abcdef";
        var (client, handler) = Client(new Fake()
            .Then(HttpStatusCode.Created, $$$"""{"attachment": {"id": 40, "kind": "audio", "mime": "audio/mp4", "duration_ms": 4200, "waveform": "{{{wire}}}"}}""")
            .Then(HttpStatusCode.Created, """{"attachment": {"id": 41, "kind": "audio", "mime": "audio/mp4", "duration_ms": 4200}}""")
            .Then(HttpStatusCode.Created, """{"attachment": {"id": 42, "kind": "audio", "mime": "audio/mp4"}}""")
            .Then(HttpStatusCode.Created, """{"attachment": {"id": 43, "kind": "video", "mime": "video/mp4"}}"""));
        var echoed = await client.Upload("audio", "audio/mp4", new byte[] { 1 }, durationMs: 4200, waveform: wire);
        Assert.Equal(wire, echoed.Value!.Attachment.Waveform);
        var old = await client.Upload("audio", "audio/mp4", new byte[] { 1 }, durationMs: 4200, waveform: wire);
        Assert.True(old.Ok);
        Assert.Null(old.Value!.Attachment.Waveform);
        await client.Upload("audio", "audio/mp4", new byte[] { 1 }, waveform: wire.ToUpperInvariant());
        await client.Upload("video", "video/mp4", new byte[] { 1 }, waveform: wire);
        Assert.Equal(
            $"https://chat.example.com/api/v1/attachments?kind=audio&duration_ms=4200&waveform={wire}",
            handler.Sent[0].RequestUri?.ToString());
        Assert.Equal("https://chat.example.com/api/v1/attachments?kind=audio", handler.Sent[2].RequestUri?.ToString());
        Assert.Equal("https://chat.example.com/api/v1/attachments?kind=video", handler.Sent[3].RequestUri?.ToString());
    }

    /// <summary>
    /// A preview is PUT as raw JPEG to its attachment, and the server's 204 is success — the empty
    /// answer that a reader expecting JSON would call unreadable.
    /// </summary>
    [Fact]
    public async Task APreviewIsPutAsRawJpegAndA204IsSuccess()
    {
        var (client, handler) = Client(new Fake().Then(HttpStatusCode.NoContent));
        var answer = await client.UploadPreview(34, "jpeg"u8.ToArray());
        Assert.True(answer.Ok);
        var request = Assert.Single(handler.Sent);
        Assert.Equal(HttpMethod.Put, request.Method);
        Assert.Equal("https://chat.example.com/api/v1/attachments/34/preview", request.RequestUri?.ToString());
        Assert.Equal("image/jpeg", request.Content?.Headers.ContentType?.MediaType);
        Assert.Equal("jpeg", Assert.Single(handler.Bodies));
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
             "members": [{"id": 7, "username": "anna", "display_name": "Anna", "role": "owner"}],
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

    /// <summary>
    /// A vote PUTs the option, a retraction DELETEs and a close POSTs, neither with a body — and each is
    /// answered with the poll's whole state. The open polls are a plain read of messages.
    /// </summary>
    [Fact]
    public async Task APollIsVotedRetractedClosedAndListedOnItsOwnPaths()
    {
        const string State = """{"message_id": 5, "poll": {"poll_seq": 12, "closed": false, "options": [{"id": 1, "text": "Pizza", "votes": [7, 9]}, {"id": 2, "text": "Pasta", "votes": []}]}}""";
        var (client, handler) = Client(new Fake()
            .Then(HttpStatusCode.OK, State)
            .Then(HttpStatusCode.OK, State)
            .Then(HttpStatusCode.OK, State)
            .Then(HttpStatusCode.OK, """{"messages": [{"id": 5, "chat_id": 42, "sender_id": 7, "body": "Dinner?", "created_at": "2026-09-14T10:00:00Z", "poll": {"poll_seq": 12, "closed": false, "options": [{"id": 1, "text": "Pizza", "votes": [7]}]}}]}"""));

        var voted = await client.Vote(42, 5, 1);
        Assert.Equal(5, voted.Value!.MessageId);
        Assert.Equal(12, voted.Value.Poll.PollSeq);
        Assert.Equal([7L, 9L], voted.Value.Poll.Options[0].Votes);
        Assert.True((await client.Unvote(42, 5)).Ok);
        Assert.True((await client.ClosePoll(42, 5)).Ok);
        var open = await client.OpenPolls(42);
        Assert.Equal("Dinner?", Assert.Single(open.Value!.Messages!).Body);

        Assert.Equal(
            [
                (HttpMethod.Put, "https://chat.example.com/api/v1/chats/42/messages/5/vote"),
                (HttpMethod.Delete, "https://chat.example.com/api/v1/chats/42/messages/5/vote"),
                (HttpMethod.Post, "https://chat.example.com/api/v1/chats/42/messages/5/poll/close"),
                (HttpMethod.Get, "https://chat.example.com/api/v1/chats/42/polls/open"),
            ],
            handler.Sent.Select(request => (request.Method, request.RequestUri!.ToString())));
        Assert.Equal(["{\"option_id\":1}", null, null, null], handler.Bodies);
    }

    [Fact]
    public async Task ADirectChatIsAskedForByTheMembersId()
    {
        var (client, handler) = Client(new Fake().Then(
            HttpStatusCode.OK, """{"chat": {"id": 42, "kind": "direct", "title": "Bob", "peer_user_id": 9}}"""));
        var answer = await client.DirectChat(9);
        Assert.True(answer.Ok);
        Assert.Equal(42, answer.Value!.Chat.Id);
        Assert.Equal(9, answer.Value.Chat.PeerUserId);
        var request = Assert.Single(handler.Sent);
        Assert.Equal(HttpMethod.Post, request.Method);
        Assert.Equal("https://chat.example.com/api/v1/chats/direct", request.RequestUri?.ToString());
        Assert.Equal("{\"user_id\":9}", Assert.Single(handler.Bodies));
    }

    /// <summary>An event's end taken off is a NULL on the wire; a field left out leaves it alone.</summary>
    [Fact]
    public async Task TakingAnEventsEndOffSendsANullAndLeavingItAloneSendsNothing()
    {
        var (client, handler) = Client(new Fake()
            .Then(HttpStatusCode.OK, """{"note": {"id": 12, "author_id": 7, "board_seq": 9}}""")
            .Then(HttpStatusCode.OK, """{"note": {"id": 12, "author_id": 7, "board_seq": 10}}"""));
        Assert.True((await client.PatchNote(12, new NotePatch(Place: "") { ClearsEnd = true })).Ok);
        Assert.True((await client.PatchNote(12, new NotePatch(Place: "Park"))).Ok);

        using var cleared = System.Text.Json.JsonDocument.Parse(handler.Bodies[0]!);
        Assert.Equal(System.Text.Json.JsonValueKind.Null, cleared.RootElement.GetProperty("ends_at").ValueKind);
        Assert.Equal(string.Empty, cleared.RootElement.GetProperty("place").GetString());
        Assert.False(cleared.RootElement.TryGetProperty("clears_end", out _));
        Assert.False(cleared.RootElement.TryGetProperty("text", out _));
        using var alone = System.Text.Json.JsonDocument.Parse(handler.Bodies[1]!);
        Assert.False(alone.RootElement.TryGetProperty("ends_at", out _));
        Assert.Equal(HttpMethod.Patch, handler.Sent[1].Method);
    }

    // ---- the sticker pack (docs/protocol.md, "Sticker pack") ---------------------------------

    private const string PackItemJson =
        """
        {"id": 5, "added_by": 7,
         "attachment": {"id": 71, "kind": "photo", "mime": "image/webp", "size": 40960, "width": 512, "height": 512},
         "created_at": "2026-09-13T10:00:00Z", "pack_seq": 12}
        """;

    [Fact]
    public async Task ThePackIsReadWholeAndCaughtUpOnByItsOwnSequence()
    {
        var (client, handler) = Client(new Fake()
            .Then(HttpStatusCode.OK, $$"""{"items": [{{PackItemJson}}], "max_pack_seq": 12}""")
            .Then(HttpStatusCode.OK, """{"items": [{"id": 5, "deleted": true, "pack_seq": 14}]}"""));

        var whole = await client.Pack();
        Assert.Equal(12, whole.Value!.MaxPackSeq);
        Assert.Equal(71, Assert.Single(whole.Value.Items!).Attachment!.Id);

        var changes = await client.PackChanges(12, 50);
        Assert.True(Assert.Single(changes.Value!.Items!).Deleted);

        Assert.Equal("https://chat.example.com/api/v1/families/mine/pack", handler.Sent[0].RequestUri!.ToString());
        Assert.Equal(
            "https://chat.example.com/api/v1/families/mine/pack/changes?after_seq=12&limit=50",
            handler.Sent[1].RequestUri!.ToString());
    }

    /// <summary>
    /// A claim: the attachment's id and, only when one was given, a label. Both <c>201</c> and
    /// <c>200</c> answer an item and both are success — a <c>200</c> is the pack already holding it.
    /// </summary>
    [Fact]
    public async Task AddingToThePackClaimsAnUploadAndTakesEitherSuccess()
    {
        var (client, handler) = Client(new Fake()
            .Then(HttpStatusCode.Created, $$"""{"item": {{PackItemJson}}}""")
            .Then(HttpStatusCode.OK, $$"""{"item": {{PackItemJson}}}""")
            .Then(HttpStatusCode.OK, $$"""{"item": {{PackItemJson}}}"""));

        var added = await client.AddToPack(71, "  party cat ");
        var again = await client.AddToPack(99);
        var blank = await client.AddToPack(99, "   ");

        Assert.True(added.Ok);
        Assert.True(again.Ok);
        // WHICH success it was is the status, and the only thing that says it: 201 is a new item, 200 is one the pack
        // already held. The cache cannot answer that — this device's own frame may land before the claim does.
        Assert.Equal(201, added.Status);
        Assert.Equal(200, again.Status);
        // The item that came back carries the id the PACK holds, which is not the one a duplicate named.
        Assert.Equal(71, again.Value!.Item.Attachment!.Id);
        Assert.Equal(HttpMethod.Post, handler.Sent[0].Method);
        Assert.Equal("https://chat.example.com/api/v1/families/mine/pack", handler.Sent[0].RequestUri!.ToString());
        Assert.Equal("""{"attachment_id":71,"label":"party cat"}""", handler.Bodies[0]);
        // No label is no key — and an empty one is no label.
        Assert.Equal("""{"attachment_id":99}""", handler.Bodies[1]);
        Assert.Equal("""{"attachment_id":99}""", handler.Bodies[2]);
        Assert.True(blank.Ok);
    }

    [Fact]
    public async Task ThePacksRefusalsAreReadAsTheProtocolWritesThem()
    {
        var (client, handler) = Client(new Fake()
            .Then(HttpStatusCode.RequestEntityTooLarge, """{"error": {"code": "pack_item_too_large", "message": "…"}}""")
            .Then(HttpStatusCode.Conflict, """{"error": {"code": "pack_full", "message": "…"}}""")
            .Then(HttpStatusCode.Forbidden, """{"error": {"code": "not_pack_item_author", "message": "…"}}""")
            .Then(HttpStatusCode.NotFound, """{"error": {"code": "pack_item_not_found", "message": "…"}}""")
            .Then(HttpStatusCode.NoContent));

        Assert.Equal(ErrorCodes.PackItemTooLarge, (await client.AddToPack(71)).Error!.Code);
        var full = await client.AddToPack(71);
        Assert.Equal(ErrorCodes.PackFull, full.Error!.Code);
        // Refusals, every one: the ceiling is the family's, and trying again would refuse again.
        Assert.False(full.Error.Transient);
        Assert.Equal(ErrorCodes.NotPackItemAuthor, (await client.RemoveFromPack(5)).Error!.Code);
        Assert.Equal(ErrorCodes.PackItemNotFound, (await client.RemoveFromPack(5)).Error!.Code);
        // Removing is idempotent, and a 204 is a success with nothing in it.
        Assert.True((await client.RemoveFromPack(5)).Ok);
        Assert.Equal(HttpMethod.Delete, handler.Sent[4].Method);
        Assert.Equal("https://chat.example.com/api/v1/families/mine/pack/5", handler.Sent[4].RequestUri!.ToString());
        // A write is never retried by the transport.
        Assert.Equal(5, handler.Sent.Count);
    }

    [Fact]
    public async Task ARestSendSaysStickerOnlyWhenItIsOne()
    {
        const string Answer =
            """
            {"message": {"id": 1340, "chat_id": 42, "sender_id": 7, "client_msg_id": "k", "body": "",
             "created_at": "2026-09-13T10:00:00Z",
             "attachments": [{"id": 90, "kind": "photo", "mime": "image/webp", "sticker": true}]}}
            """;
        var (client, handler) = Client(new Fake()
            .Then(HttpStatusCode.Created, Answer)
            .Then(HttpStatusCode.Created, Answer));

        var sent = await client.SendMessage(42, "k", "", attachmentIds: [90], sticker: true);
        await client.SendMessage(42, "k2", "Dinner at 7?", attachmentIds: [90]);

        Assert.True(sent.Value!.Message.Media[0].Sticker);
        Assert.Equal("""{"client_msg_id":"k","body":"","attachment_ids":[90],"sticker":true}""", handler.Bodies[0]);
        // Absent otherwise — never false.
        Assert.DoesNotContain("sticker", handler.Bodies[1], StringComparison.Ordinal);
    }

    [Fact]
    public async Task ARestSendSaysRoundOnlyWhenItIsAVideoMessage()
    {
        const string Answer =
            """
            {"message": {"id": 1341, "chat_id": 42, "sender_id": 7, "client_msg_id": "k", "body": "",
             "created_at": "2026-10-05T10:00:00Z",
             "attachments": [{"id": 91, "kind": "video", "mime": "video/mp4", "size": 1649700, "width": 480,
                              "height": 480, "duration_ms": 23400, "has_preview": true, "round": true}]}}
            """;
        var (client, handler) = Client(new Fake()
            .Then(HttpStatusCode.Created, Answer)
            .Then(HttpStatusCode.Created, Answer));

        var sent = await client.SendMessage(42, "k", "", replyToMessageId: 41, attachmentIds: [91], round: true);
        await client.SendMessage(42, "k2", "", attachmentIds: [91]);

        Assert.True(sent.Value!.Message.Media[0].Round);
        Assert.Equal(91, sent.Value.Message.RoundVideo!.Id);
        Assert.Equal(
            """{"client_msg_id":"k","body":"","reply_to_message_id":41,"attachment_ids":[91],"round":true}""",
            handler.Bodies[0]);
        // Absent otherwise — never false — so a server that predates video messages reads what it always read.
        Assert.DoesNotContain("round", handler.Bodies[1], StringComparison.Ordinal);
        Assert.DoesNotContain("sticker", handler.Bodies[0], StringComparison.Ordinal);
    }

    /// <summary>
    /// The family's own document: the pack's mark, omitted while the pack is untouched, and its
    /// two ceilings — whose ABSENCE is how a client knows the server predates packs.
    /// </summary>
    [Fact]
    public async Task TheFamilysDocumentCarriesThePacksMarkAndCeilingsOrNeither()
    {
        var (client, _) = Client(new Fake()
            .Then(HttpStatusCode.OK,
                """
                {"family": {"id": 3, "name": "The Smiths"}, "members": [],
                 "max_pack_seq": 14, "max_pack_items": 200, "max_pack_item_bytes": 524288}
                """)
            .Then(HttpStatusCode.OK,
                """{"family": {"id": 3, "name": "The Smiths"}, "members": [], "max_pack_items": 200, "max_pack_item_bytes": 524288}""")
            .Then(HttpStatusCode.OK, """{"family": {"id": 3, "name": "The Smiths"}, "members": []}"""));

        var touched = (await client.Family()).Value!;
        Assert.Equal(14, touched.MaxPackSeq);
        Assert.Equal(200, touched.MaxPackItems);
        Assert.Equal(524_288, touched.MaxPackItemBytes);

        var untouched = (await client.Family()).Value!;
        Assert.Null(untouched.MaxPackSeq);
        Assert.Equal(200, untouched.MaxPackItems);

        var older = (await client.Family()).Value!;
        Assert.Null(older.MaxPackItems);
        Assert.Null(older.MaxPackItemBytes);
    }

    /// <summary>
    /// The video message's two limits on the family's document (docs/protocol.md, "Video messages"): read as numbers, and
    /// their ABSENCE — an older server — read as nothing to record with.
    /// </summary>
    [Fact]
    public async Task TheFamilysDocumentCarriesTheVideoMessagesLimitsOrNeither()
    {
        var (client, _) = Client(new Fake()
            .Then(HttpStatusCode.OK,
                """
                {"family": {"id": 3, "name": "The Smiths"}, "members": [],
                 "max_round_video_ms": 60000, "max_round_video_bytes": 4194304}
                """)
            .Then(HttpStatusCode.OK, """{"family": {"id": 3, "name": "The Smiths"}, "members": []}"""));

        var current = (await client.Family()).Value!;
        Assert.Equal(60_000, current.MaxRoundVideoMs);
        Assert.Equal(4_194_304, current.MaxRoundVideoBytes);
        Assert.Equal(new RoundVideoLimits(60_000, 4_194_304), RoundVideoLimits.Of(current));

        var older = (await client.Family()).Value!;
        Assert.Null(older.MaxRoundVideoMs);
        Assert.Null(older.MaxRoundVideoBytes);
        Assert.Null(RoundVideoLimits.Of(older));
    }

    /// <summary>
    /// Both keys, or nothing: one alone — or a number no server sends — is a server this client cannot record for, and a
    /// video entry that ends in a refusal is worse than none.
    /// </summary>
    [Theory]
    [InlineData(60_000L, null)]
    [InlineData(null, 12_582_912L)]
    [InlineData(0L, 12_582_912L)]
    [InlineData(60_000L, 0L)]
    [InlineData(-1L, 12_582_912L)]
    [InlineData(60_000L, -5L)]
    public void TheLimitsAreBothOrNothing(long? ms, long? bytes)
    {
        var family = new FamilyResponse(new FamilyDto(3, "The Smiths"), MaxRoundVideoMs: ms, MaxRoundVideoBytes: bytes);
        Assert.Null(RoundVideoLimits.Of(family));
    }

    // ---- transcripts on request (#62) -------------------------------------------------------------

    /// <summary>
    /// The STORED-BYTES form (docs/protocol.md, "Transcripts on request"): a POST to the attachment's own path with NO
    /// BODY — the form whose answer the server keeps — and the text read back with the language the provider named.
    /// </summary>
    [Fact]
    public async Task ATranscriptIsAskedForWithNoBodyAtTheAttachmentsOwnPath()
    {
        var (client, handler) = Client(new Fake().Then(
            HttpStatusCode.OK, """{"transcript": {"text": "Мы будем в шесть", "language": "ru"}}"""));
        var answer = await client.Transcript(42, 1338, 34);
        Assert.True(answer.Ok);
        Assert.Equal("Мы будем в шесть", answer.Value!.Transcript!.Text);
        Assert.Equal("ru", answer.Value.Transcript.Language);
        var request = Assert.Single(handler.Sent);
        Assert.Equal(HttpMethod.Post, request.Method);
        Assert.Equal(
            "https://chat.example.com/api/v1/chats/42/messages/1338/attachments/34/transcript",
            request.RequestUri?.ToString());
        // No body at all: a multipart one would be the OTHER form, whose answer is never kept.
        Assert.Null(Assert.Single(handler.Bodies));
        Assert.Equal("t0ken", request.Headers.Authorization?.Parameter);
    }

    /// <summary>SILENCE IS AN ANSWER: the empty text, and no language, read as a success.</summary>
    [Fact]
    public async Task SilenceIsASuccessWithEmptyText()
    {
        var (client, _) = Client(new Fake().Then(HttpStatusCode.OK, """{"transcript": {"text": ""}}"""));
        var answer = await client.Transcript(42, 1338, 34);
        Assert.True(answer.Ok);
        Assert.Equal(string.Empty, answer.Value!.Transcript!.Text);
        Assert.Null(answer.Value.Transcript.Language);
    }

    /// <summary>
    /// Every refusal the endpoint answers, read as its code and status — the ones the bubble branches on — and only
    /// <c>internal</c> transient. A write, so nothing here repeats it: not even the 500.
    /// </summary>
    [Theory]
    [InlineData("transcripts_unavailable", 403, false)]
    [InlineData("assistant_consent_required", 403, false)]
    [InlineData("transcript_not_allowed", 403, false)]
    [InlineData("not_transcribable", 400, false)]
    [InlineData("transcript_refused", 400, false)]
    [InlineData("message_not_found", 404, false)]
    [InlineData("internal", 500, true)]
    public async Task ATranscriptRefusalIsReadAsItsCode(string code, int status, bool transient)
    {
        var (client, handler) = Client(new Fake().Then(
            (HttpStatusCode)status, $$$"""{"error": {"code": "{{{code}}}", "message": "no"}}"""));
        var answer = await client.Transcript(42, 1338, 34);
        Assert.False(answer.Ok);
        Assert.Equal(code, answer.Error!.Code);
        Assert.Equal(status, answer.Error.Status);
        Assert.Equal(transient, answer.Error.Transient);
        Assert.Single(handler.Sent);
    }

    /// <summary>
    /// SLOW: "a timeout of its OWN, no shorter than 90 s … never its ordinary request timeout" — and longer than the
    /// ordinary one, with nothing in the HttpClient the app builds to cap it.
    /// </summary>
    [Fact]
    public void ATranscriptHasADeadlineOfItsOwn()
    {
        Assert.True(ApiClient.TranscriptTimeout >= TimeSpan.FromSeconds(90));
        Assert.True(ApiClient.TranscriptTimeout > ApiClient.OrdinaryTimeout);
        using var http = ApiClient.NewHttpClient();
        var client = new ApiClient(http, ServerUrl.Normalise("chat.example.com")!, new MemoryTokenStore("t0ken"));
        Assert.Equal(ApiClient.TranscriptTimeout, client.TranscriptDeadline);
    }

    /// <summary>
    /// The deadline covers the whole request, the upload included: a supplied sound of up to 25 MiB goes up BEFORE the
    /// provider's own wait (the server's 180 s) begins, and the reference proxy waits 300 s on this route. A client that
    /// gives up first never gets an answer for supplied sound — the server drops that call with the connection and
    /// keeps nothing — so "try again" would fail the same way every time. Past the proxy's wait, as iOS (310 s), Android
    /// (310 s) and the web (300 s) wait: the server, or the proxy answering for it, ends the wait.
    /// </summary>
    [Fact]
    public void ATranscriptWaitsPastTheProxy()
    {
        Assert.True(ApiClient.TranscriptTimeout > TimeSpan.FromSeconds(300));
        Assert.True(ApiClient.TranscriptTimeout > TimeSpan.FromSeconds(180) + TimeSpan.FromSeconds(60));
    }

    /// <summary>An M4A's first bytes: a size, then <c>ftyp</c>, then its brand — what the server checks the part for.</summary>
    private static readonly byte[] M4a = [0, 0, 0, 0x18, (byte)'f', (byte)'t', (byte)'y', (byte)'p', (byte)'M', (byte)'4', (byte)'A', (byte)' ', 0, 0, 2, 0, 0xFF, 0x00];

    /// <summary>
    /// THE SUPPLIED-SOUND FORM (docs/protocol.md, "Transcripts on request"): the same path, as
    /// <c>multipart/form-data</c> with ONE part named <c>audio</c> — the device's M4A, typed <c>audio/mp4</c>, its bytes
    /// exactly as made — and the answer read as the stored form's is.
    /// </summary>
    [Fact]
    public async Task SuppliedSoundGoesAsOneMultipartAudioPart()
    {
        var (client, handler) = Client(new Fake().Then(
            HttpStatusCode.OK, """{"transcript": {"text": "Back at six", "language": "en"}}"""));
        var answer = await client.Transcript(42, 1338, 34, M4a);
        Assert.True(answer.Ok);
        Assert.Equal("Back at six", answer.Value!.Transcript!.Text);
        var request = Assert.Single(handler.Sent);
        Assert.Equal(HttpMethod.Post, request.Method);
        Assert.Equal(
            "https://chat.example.com/api/v1/chats/42/messages/1338/attachments/34/transcript",
            request.RequestUri?.ToString());
        Assert.Equal("t0ken", request.Headers.Authorization?.Parameter);
        var type = Assert.Single(handler.BodyTypes);
        Assert.Equal("multipart/form-data", type?.MediaType);
        Assert.Contains(type!.Parameters, parameter => parameter.Name == "boundary" && !string.IsNullOrEmpty(parameter.Value));
        var part = Assert.Single(Assert.Single(handler.Parts));
        Assert.Equal("audio", part.Name?.Trim('"'));
        Assert.Equal("audio.m4a", part.FileName?.Trim('"'));
        Assert.Equal("audio/mp4", part.Type);
        Assert.Equal(M4a, part.Bytes);
    }

    /// <summary>A write like the stored form: a 500 is said once, never repeated here — and so is a lost connection.</summary>
    [Fact]
    public async Task SuppliedSoundIsNeverRepeatedHere()
    {
        var (client, handler) = Client(new Fake()
            .Then(HttpStatusCode.InternalServerError, """{"error": {"code": "internal", "message": "provider"}}"""));
        var answer = await client.Transcript(42, 1338, 34, M4a);
        Assert.False(answer.Ok);
        Assert.True(answer.Error!.Transient);
        Assert.Single(handler.Sent);

        var (unreached, gone) = Client(new Fake().ThenUnreachable());
        Assert.Equal(ErrorCodes.Transport, (await unreached.Transcript(42, 1338, 34, M4a)).Error!.Code);
        Assert.Single(gone.Sent);
    }

    /// <summary>The server's verdict on supplied sound is read as its code: not MPEG-4, too big or missing — terminal.</summary>
    [Fact]
    public async Task SuppliedSoundTheServerRefusesIsReadAsItsCode()
    {
        var (client, _) = Client(new Fake().Then(
            HttpStatusCode.BadRequest, """{"error": {"code": "not_transcribable", "message": "not mpeg-4"}}"""));
        var answer = await client.Transcript(42, 1338, 34, M4a);
        Assert.Equal(ErrorCodes.NotTranscribable, answer.Error!.Code);
        Assert.False(answer.Error.Transient);
    }

    /// <summary>It runs under the transcript's deadline too, never the ordinary one.</summary>
    [Fact]
    public async Task SuppliedSoundOutlastsTheOrdinaryDeadline()
    {
        var handler = new Fake().Then(HttpStatusCode.OK, """{"transcript": {"text": ""}}""");
        handler.Delays.Enqueue(TimeSpan.FromMilliseconds(300));
        var client = new ApiClient(
            new HttpClient(handler), ServerUrl.Normalise("chat.example.com")!, new MemoryTokenStore("t0ken"))
        {
            RequestDeadline = TimeSpan.FromMilliseconds(50),
            TranscriptDeadline = TimeSpan.FromSeconds(30),
        };
        var answer = await client.Transcript(42, 1338, 34, M4a);
        Assert.True(answer.Ok);
        Assert.Equal(string.Empty, answer.Value!.Transcript!.Text);
    }

    /// <summary>The rule, run: a server slower than the ordinary deadline still answers a transcript.</summary>
    [Fact]
    public async Task ATranscriptOutlastsTheOrdinaryDeadline()
    {
        var handler = new Fake().Then(HttpStatusCode.OK, """{"transcript": {"text": "Back at six"}}""");
        handler.Delays.Enqueue(TimeSpan.FromMilliseconds(300));
        var client = new ApiClient(
            new HttpClient(handler), ServerUrl.Normalise("chat.example.com")!, new MemoryTokenStore("t0ken"))
        {
            RequestDeadline = TimeSpan.FromMilliseconds(50),
            TranscriptDeadline = TimeSpan.FromSeconds(30),
        };
        var answer = await client.Transcript(42, 1338, 34);
        Assert.True(answer.Ok);
        Assert.Equal("Back at six", answer.Value!.Transcript!.Text);
    }

    /// <summary>And its own deadline does end it: a transport failure, transient, which the member may retry.</summary>
    [Fact]
    public async Task ATranscriptPastItsOwnDeadlineIsATransportFailure()
    {
        var handler = new Fake().Then(HttpStatusCode.OK, """{"transcript": {"text": "too late"}}""");
        handler.Delays.Enqueue(TimeSpan.FromSeconds(5));
        var client = new ApiClient(
            new HttpClient(handler), ServerUrl.Normalise("chat.example.com")!, new MemoryTokenStore("t0ken"))
        {
            TranscriptDeadline = TimeSpan.FromMilliseconds(50),
        };
        var answer = await client.Transcript(42, 1338, 34);
        Assert.False(answer.Ok);
        Assert.Equal(ErrorCodes.Transport, answer.Error!.Code);
        Assert.True(answer.Error.Transient);
        // Never repeated here: a write.
        Assert.Single(handler.Sent);
    }

    /// <summary>
    /// The server's capability and the owner's switch, as <c>GET /families/mine</c> carries them — and an older server,
    /// which carries neither, reads as "no".
    /// </summary>
    [Fact]
    public async Task TheTranscriptCapabilityAndTheOwnersSwitchArriveWithTheFamily()
    {
        var (client, _) = Client(new Fake()
            .Then(HttpStatusCode.OK, """
                {"family": {"id": 3, "name": "The Smiths", "ai_transcripts": true},
                 "assistant": {"user_id": 1, "display_name": "Assistant", "mention": "@ai",
                               "processor": "Azure OpenAI", "transcribe": true, "transcribe_max_bytes": 26214400}}
                """)
            .Then(HttpStatusCode.OK, """
                {"family": {"id": 3, "name": "The Smiths"},
                 "assistant": {"user_id": 1, "display_name": "Assistant", "mention": "@ai"}}
                """));
        var newer = (await client.Family()).Value!;
        Assert.True(newer.Family.AiTranscripts);
        Assert.True(newer.Assistant!.Transcribe);
        Assert.Equal(26_214_400, newer.Assistant.TranscribeMaxBytes);
        var older = (await client.Family()).Value!;
        Assert.False(older.Family.AiTranscripts);
        Assert.False(older.Assistant!.Transcribe);
        Assert.Null(older.Assistant.TranscribeMaxBytes);
    }

    /// <summary>The owner's switch goes out as its own key, and only when it is in the patch.</summary>
    [Fact]
    public async Task TheTranscriptSwitchIsPatchedAsItsOwnKey()
    {
        var (client, handler) = Client(new Fake()
            .Then(HttpStatusCode.OK, """{"family": {"id": 3, "name": "The Smiths", "ai_transcripts": true}}""")
            .Then(HttpStatusCode.OK, """{"family": {"id": 3, "name": "The Smiths", "ai_vision": true}}"""));
        var on = await client.PatchFamily(new FamilyPatch { AiTranscripts = true });
        Assert.True(on.Value!.Family.AiTranscripts);
        Assert.Equal("{\"ai_transcripts\":true}", handler.Bodies[0]);
        await client.PatchFamily(new FamilyPatch { AiVision = true });
        Assert.DoesNotContain("ai_transcripts", handler.Bodies[1], StringComparison.Ordinal);
    }

    /// <summary>The statistics' two new numbers, read where they are and zero where an older server sends neither.</summary>
    [Fact]
    public async Task TheRecordingsAsTextAreInTheStatistics()
    {
        var (client, _) = Client(new Fake().Then(HttpStatusCode.OK, """
            {"generated_at": "2026-10-02T10:00:00Z",
             "totals": {"members": 2, "messages": 10, "board_notes": 0,
                        "ai": {"questions": 3, "transcripts": 2, "transcript_duration_ms": 95000}},
             "members": [{"user_id": 7, "display_name": "Anna", "messages": 6, "ai": {"questions": 1}}]}
            """));
        var stats = (await client.Stats()).Value!;
        Assert.Equal(2, stats.Totals.Ai!.Transcripts);
        Assert.Equal(95_000, stats.Totals.Ai.TranscriptDurationMs);
        Assert.Equal(0, stats.Members![0].Ai!.Transcripts);
    }

    /// <summary>
    /// Looking things up, as <c>GET /families/mine</c> and <c>GET /me</c> carry it (docs/protocol.md, "Looking things up"):
    /// the providers by name in the server's order, the owner's switch, and the member's stamp — and an older server, which
    /// carries none of them, reads as "no lookups here".
    /// </summary>
    [Fact]
    public async Task TheLookupProvidersSwitchAndConsentArriveTolerantly()
    {
        var (client, _) = Client(new Fake()
            .Then(HttpStatusCode.OK, """
                {"family": {"id": 3, "name": "The Smiths", "ai_lookups": true},
                 "assistant": {"user_id": 1, "display_name": "Assistant", "mention": "@ai", "processor": "Azure OpenAI",
                               "lookups": ["Brave Search", "Open-Meteo", "Wikipedia"]}}
                """)
            .Then(HttpStatusCode.OK, """
                {"family": {"id": 3, "name": "The Smiths"},
                 "assistant": {"user_id": 1, "display_name": "Assistant", "mention": "@ai"}}
                """)
            .Then(HttpStatusCode.OK, """
                {"user": {"id": 7, "username": "anna", "display_name": "Anna"},
                 "assistant_consent_at": "2026-09-19T08:12:04Z", "assistant_lookup_consent_at": "2026-10-03T09:30:00Z"}
                """)
            .Then(HttpStatusCode.OK, """
                {"user": {"id": 7, "username": "anna", "display_name": "Anna"}, "assistant_consent_at": null}
                """));
        var newer = (await client.Family()).Value!;
        Assert.True(newer.Family.AiLookups);
        Assert.Equal(["Brave Search", "Open-Meteo", "Wikipedia"], newer.Assistant!.Lookups!);
        var older = (await client.Family()).Value!;
        Assert.False(older.Family.AiLookups);
        Assert.Null(older.Assistant!.Lookups);
        Assert.Equal("2026-10-03T09:30:00Z", (await client.Me()).Value!.AssistantLookupConsentAt);
        Assert.Null((await client.Me()).Value!.AssistantLookupConsentAt);
    }

    /// <summary>The owner's switch goes out as its own key, and only when it is in the patch.</summary>
    [Fact]
    public async Task TheLookupSwitchIsPatchedAsItsOwnKey()
    {
        var (client, handler) = Client(new Fake()
            .Then(HttpStatusCode.OK, """{"family": {"id": 3, "name": "The Smiths", "ai_lookups": false}}""")
            .Then(HttpStatusCode.OK, """{"family": {"id": 3, "name": "The Smiths", "ai_history": true}}"""));
        var off = await client.PatchFamily(new FamilyPatch { AiLookups = false });
        Assert.False(off.Value!.Family.AiLookups);
        Assert.Equal("{\"ai_lookups\":false}", handler.Bodies[0]);
        Assert.Equal(HttpMethod.Patch, handler.Sent[0].Method);
        await client.PatchFamily(new FamilyPatch { AiHistory = true });
        Assert.DoesNotContain("ai_lookups", handler.Bodies[1], StringComparison.Ordinal);
    }

    /// <summary>
    /// The greeting's weather, as <c>GET /families/mine</c> carries it (docs/protocol.md, "Today's weather, for places the
    /// owner chose"): the owner's places on the family, the server's ability on the assistant — and an older server, which
    /// carries neither, reads as no places and no greeting weather.
    /// </summary>
    [Fact]
    public async Task TheGreetingPlacesAndWeatherArriveTolerantly()
    {
        var (client, _) = Client(new Fake()
            .Then(HttpStatusCode.OK, """
                {"family": {"id": 3, "name": "The Smiths", "ai_greeting": true, "greeting_places": ["Moscow", "Белград"]},
                 "assistant": {"user_id": 1, "display_name": "Assistant", "mention": "@ai", "greeting_weather": true}}
                """)
            .Then(HttpStatusCode.OK, """
                {"family": {"id": 3, "name": "The Smiths", "greeting_places": []},
                 "assistant": {"user_id": 1, "display_name": "Assistant", "mention": "@ai", "greeting_weather": false}}
                """)
            .Then(HttpStatusCode.OK, """
                {"family": {"id": 3, "name": "The Smiths"},
                 "assistant": {"user_id": 1, "display_name": "Assistant", "mention": "@ai"}}
                """));
        var newer = (await client.Family()).Value!;
        Assert.Equal(["Moscow", "Белград"], newer.Family.GreetingPlaces!);
        Assert.True(newer.Assistant!.GreetingWeather);
        var off = (await client.Family()).Value!;
        Assert.Empty(off.Family.GreetingPlaces!);
        Assert.False(off.Assistant!.GreetingWeather);
        var older = (await client.Family()).Value!;
        Assert.Null(older.Family.GreetingPlaces);
        Assert.False(older.Assistant!.GreetingWeather);
    }

    /// <summary>
    /// The places go out as one list that REPLACES the stored one; <c>[]</c> clears it, and a patch without them never
    /// carries the key — never a <c>null</c>, which the server refuses. The answer is the list as the server KEPT it.
    /// </summary>
    [Fact]
    public async Task TheGreetingPlacesArePatchedAsOneListAndNeverAsNull()
    {
        var (client, handler) = Client(new Fake()
            .Then(HttpStatusCode.OK, """{"family": {"id": 3, "name": "The Smiths", "greeting_places": ["Moscow"]}}""")
            .Then(HttpStatusCode.OK, """{"family": {"id": 3, "name": "The Smiths", "greeting_places": []}}""")
            .Then(HttpStatusCode.OK, """{"family": {"id": 3, "name": "The Smiths", "greeting_places": []}}""")
            .Then(HttpStatusCode.OK, """{"family": {"id": 3, "name": "The Smiths", "greeting_places": ["Белград"]}}"""));

        var kept = await client.PatchFamily(new FamilyPatch { GreetingPlaces = ["Moscow", "moscow"] });
        Assert.Equal(HttpMethod.Patch, handler.Sent[0].Method);
        Assert.EndsWith("/families/mine", handler.Sent[0].RequestUri!.AbsolutePath, StringComparison.Ordinal);
        Assert.Equal("{\"greeting_places\":[\"Moscow\",\"moscow\"]}", handler.Bodies[0]);
        // What the server kept, not what was sent.
        Assert.Equal(["Moscow"], kept.Value!.Family.GreetingPlaces!);

        await client.PatchFamily(new FamilyPatch { GreetingPlaces = [] });
        Assert.Equal("{\"greeting_places\":[]}", handler.Bodies[1]);

        await client.PatchFamily(new FamilyPatch { AiGreeting = true });
        Assert.DoesNotContain("greeting_places", handler.Bodies[2], StringComparison.Ordinal);

        // Any script: the body is JSON whatever the escaping, and it reads back as what was typed.
        await client.PatchFamily(new FamilyPatch { GreetingPlaces = ["Белград"] });
        using var sent = System.Text.Json.JsonDocument.Parse(handler.Bodies[3]!);
        Assert.Equal("Белград", sent.RootElement.GetProperty("greeting_places")[0].GetString());
        Assert.Single(sent.RootElement.EnumerateObject());
    }

    /// <summary>
    /// The member's own lookup consent: its own endpoint, the same shape as the assistant's, the stamp read back — and
    /// the refusal for granting it without the first one read as its code, asked once.
    /// </summary>
    [Fact]
    public async Task TheLookupConsentIsItsOwnEndpoint()
    {
        var (client, handler) = Client(new Fake()
            .Then(HttpStatusCode.OK, """{"assistant_lookup_consent_at": "2026-10-03T09:30:00Z"}""")
            .Then(HttpStatusCode.OK, """{"assistant_lookup_consent_at": null}""")
            .Then(HttpStatusCode.Forbidden, """{"error": {"code": "assistant_consent_required", "message": "agree first"}}"""));
        var granted = await client.SetAssistantLookupConsent(true);
        Assert.Equal("2026-10-03T09:30:00Z", granted.Value!.AssistantLookupConsentAt);
        Assert.Equal(HttpMethod.Post, handler.Sent[0].Method);
        Assert.Equal("https://chat.example.com/api/v1/me/assistant-lookup-consent", handler.Sent[0].RequestUri?.ToString());
        Assert.Equal("{\"granted\":true}", handler.Bodies[0]);

        var withdrawn = await client.SetAssistantLookupConsent(false);
        Assert.True(withdrawn.Ok);
        Assert.Null(withdrawn.Value!.AssistantLookupConsentAt);
        Assert.Equal("{\"granted\":false}", handler.Bodies[1]);

        var refused = await client.SetAssistantLookupConsent(true);
        Assert.Equal(ErrorCodes.AssistantConsentRequired, refused.Error!.Code);
        Assert.Equal(3, handler.Sent.Count);
    }

    /// <summary>The paid web searches, read per member and in the totals — and zero where an older server sends none.</summary>
    [Fact]
    public async Task TheWebSearchesAreInTheStatistics()
    {
        var (client, _) = Client(new Fake().Then(HttpStatusCode.OK, """
            {"generated_at": "2026-10-03T10:00:00Z",
             "totals": {"members": 2, "messages": 10, "board_notes": 0, "ai": {"questions": 3, "searches": 9}},
             "members": [{"user_id": 7, "display_name": "Anna", "messages": 6, "ai": {"questions": 2, "searches": 4}},
                         {"user_id": 11, "display_name": "Bob", "messages": 4, "ai": {"questions": 1}}]}
            """));
        var stats = (await client.Stats()).Value!;
        Assert.Equal(9, stats.Totals.Ai!.Searches);
        Assert.Equal(4, stats.Members![0].Ai!.Searches);
        Assert.Equal(0, stats.Members[1].Ai!.Searches);
    }
}
