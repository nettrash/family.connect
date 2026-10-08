using System.Net;
using System.Text;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.Core.Tests.Protocol;

/// <summary>
/// Stickers and video messages, from the server's own bytes to the row the window draws — and the cache an OLDER build
/// wrote, which kept a sticker a grey photo tile and a circle a square, for good, until schema step 8's repair read it
/// again (docs/audio-video-messages-2026-10-04.md, S5.8; iOS <c>RoundCacheRepairTests</c>, Android
/// <c>RoundFlagMigrationTest</c>).
/// </summary>
public class FlagRepairTests : IDisposable
{
    private readonly Database database = Database.OpenInMemory();
    private readonly ChatStore chats;

    public FlagRepairTests()
    {
        chats = new ChatStore(database, () => 7);
        chats.Replace([new ChatRowDto(new ChatDto(42, "family", "The Smiths"))]);
    }

    public void Dispose() => database.Dispose();

    // ---- the server's bytes, verbatim -------------------------------------------------------------

    // What server/src/models.rs serializes: `body` ALWAYS present ("" for a sticker and a circle), a real uuid for
    // client_msg_id, BOTH `attachment` and `attachments`, `has_preview` always, and each flag only when true — never
    // `"round": false` on a sticker, never `"sticker": false` on a circle (server/tests/round_flow.rs, pack_flow.rs).
    private const string StickerJson =
        """
        {"id": 1340, "chat_id": 42, "sender_id": 9, "client_msg_id": "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01", "body": "",
         "created_at": "2026-10-05T10:00:00Z",
         "attachment": {"id": 90, "kind": "photo", "mime": "image/webp", "size": 40960, "width": 512, "height": 512,
                        "has_preview": false, "sticker": true},
         "attachments": [{"id": 90, "kind": "photo", "mime": "image/webp", "size": 40960, "width": 512, "height": 512,
                          "has_preview": false, "sticker": true}]}
        """;

    private const string RoundJson =
        """
        {"id": 1341, "chat_id": 42, "sender_id": 9, "client_msg_id": "4f9e21c0-ceea-4e17-a91c-0d9f8e7b2a01", "body": "",
         "created_at": "2026-10-05T10:01:00Z",
         "attachment": {"id": 91, "kind": "video", "mime": "video/mp4", "size": 812345, "width": 480, "height": 480,
                        "duration_ms": 23400, "has_preview": true, "round": true},
         "attachments": [{"id": 91, "kind": "video", "mime": "video/mp4", "size": 812345, "width": 480, "height": 480,
                          "duration_ms": 23400, "has_preview": true, "round": true}]}
        """;

    /// <summary>
    /// A FRESH CACHE IS RIGHT ON EVERY ROUTE: the history page, the live frame, the ack, the edits feed and the chat list's
    /// trimmed preview all decode the same record through the same <see cref="Wire.Options"/>, and the store gives both
    /// flags back as it was given them. Whatever drew a sticker as a photo on Windows did not start here.
    /// </summary>
    [Fact]
    public void EveryRouteFromTheServerKeepsBothFlagsThroughTheStore()
    {
        var sticker = Wire.Decode<MessageDto>(StickerJson)!;
        var round = Wire.Decode<MessageDto>(RoundJson)!;
        Assert.Equal(90, sticker.StickerPicture!.Id);
        Assert.Equal(91, round.RoundVideo!.Id);

        var frame = Assert.IsType<ServerFrame.Message>(ServerFrame.Parse($$"""{"type": "message", "message": {{StickerJson}}}"""));
        Assert.Equal(90, frame.Value.StickerPicture!.Id);
        var ack = Assert.IsType<ServerFrame.Ack>(ServerFrame.Parse(
            $$"""{"type": "ack", "client_msg_id": "4f9e21c0-ceea-4e17-a91c-0d9f8e7b2a01", "message": {{RoundJson}}}"""));
        Assert.Equal(91, ack.Value.RoundVideo!.Id);
        var page = Wire.Decode<MessagesResponse>($$"""{"messages": [{{StickerJson}}, {{RoundJson}}]}""")!;

        chats.Apply(page.Messages!, SeqRoute.Evidence);
        Assert.Equal(90, chats.Message(1340)!.StickerPicture!.Id);
        Assert.Equal(91, chats.Message(1341)!.RoundVideo!.Id);
        // Written by this build, so nothing to repair.
        Assert.Empty(chats.FlagRepairCandidates(Resync.FlagRepairBatch));

        // The chat list's preview: no dimensions, the flags kept (handlers_chat.rs's own comment says why).
        using var other = Database.OpenInMemory();
        var listed = new ChatStore(other, () => 7);
        var list = Wire.Decode<ChatsResponse>(
            """
            {"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths"}, "unread_count": 0,
              "last_message": {"id": 1341, "chat_id": 42, "sender_id": 9, "client_msg_id": "4f9e21c0-ceea-4e17-a91c-0d9f8e7b2a01",
                               "body": "", "created_at": "2026-10-05T10:01:00Z",
                               "attachment": {"id": 91, "kind": "video", "mime": "video/mp4", "size": 812345, "has_preview": true, "round": true},
                               "attachments": [{"id": 91, "kind": "video", "mime": "video/mp4", "size": 812345, "has_preview": true, "round": true}]}}]}
            """)!;
        listed.Replace(list.Chats!);
        Assert.Equal(91, listed.Message(1341)!.RoundVideo!.Id);
    }

    // ---- what an older build left behind --------------------------------------------------------

    /// <summary>
    /// A row exactly as a build from before #58/#79 wrote it: the set decoded into a record that had no
    /// <c>sticker</c> and no <c>round</c>, encoded back with only the fields it knew, IN SEQUENCE — and schema step 8's
    /// column at its default, because nothing says which build wrote it.
    /// </summary>
    private void HeldByAnOlderBuild(long id, string? attachments, string body = "", long chat = 42)
    {
        using var command = database.Connection.CreateCommand();
        command.CommandText =
            """
            INSERT INTO messages (message_id, chat_id, sender_id, client_msg_id, body, created_at, attachments_json, sequenced)
            VALUES ($id, $chat, 9, NULL, $body, 0, $attachments, 1)
            """;
        command.Parameters.AddWithValue("$id", id);
        command.Parameters.AddWithValue("$chat", chat);
        command.Parameters.AddWithValue("$body", body);
        command.Parameters.AddWithValue("$attachments", (object?)attachments ?? DBNull.Value);
        command.ExecuteNonQuery();
    }

    private const string OldSticker =
        """[{"id":90,"kind":"photo","mime":"image/webp","size":40960,"width":512,"height":512,"has_preview":false}]""";

    private const string OldRound =
        """[{"id":91,"kind":"video","mime":"video/mp4","size":812345,"width":480,"height":480,"duration_ms":23400,"has_preview":true}]""";

    /// <summary>Answers by path, remembering every path, in order.</summary>
    private sealed class Server(Func<string, (HttpStatusCode Status, string? Json)?> route) : HttpMessageHandler
    {
        public List<string> Asked { get; } = [];

        protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken)
        {
            var path = request.RequestUri!.PathAndQuery;
            Asked.Add(path);
            if (route(path) is not { } answer)
            {
                throw new HttpRequestException($"no route for {path}");
            }
            var response = new HttpResponseMessage(answer.Status);
            if (answer.Json is not null)
            {
                response.Content = new StringContent(answer.Json, Encoding.UTF8, "application/json");
            }
            return Task.FromResult(response);
        }
    }

    private const string Me =
        """{"user": {"id": 7, "username": "anna", "display_name": "Anna"}, "blocked_user_ids": []}""";

    /// <summary>A server with no family to read and one chat, whose catch-up has nothing new — and the per-message reads.</summary>
    private static Func<string, (HttpStatusCode, string?)?> Pass(Func<long, (HttpStatusCode, string?)?> single) =>
        path =>
        {
            if (path == "/api/v1/me")
            {
                return (HttpStatusCode.OK, Me);
            }
            if (path == "/api/v1/chats")
            {
                return (HttpStatusCode.OK, """{"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths"}}]}""");
            }
            if (path.Contains("after_id=", StringComparison.Ordinal))
            {
                return (HttpStatusCode.OK, """{"messages": []}""");
            }
            var marker = "/messages?before_id=";
            var at = path.IndexOf(marker, StringComparison.Ordinal);
            if (at >= 0 && path.EndsWith("&limit=1", StringComparison.Ordinal))
            {
                var before = long.Parse(path[(at + marker.Length)..path.IndexOf('&', at)], System.Globalization.CultureInfo.InvariantCulture);
                return single(before - 1);
            }
            return null;
        };

    private (Resync Resync, Server Handler) Build(Func<long, (HttpStatusCode, string?)?> single)
    {
        var server = new Server(Pass(single));
        var api = new ApiClient(new HttpClient(server), ServerUrl.Normalise("chat.example.com")!, new MemoryTokenStore("t0ken"));
        return (new Resync(api, chats, new BoardStore(database)), server);
    }

    private static (HttpStatusCode, string?) Page(params string[] messages) =>
        (HttpStatusCode.OK, $$"""{"messages": [{{string.Join(",", messages)}}]}""");

    /// <summary>
    /// THE BUG, AND ITS REPAIR. A sticker and a circle an older build cached are a photo and a square video to this one —
    /// the record now has the flags, the bytes on disk never did — and the chat is held IN SEQUENCE, so opening it reads
    /// nothing (<c>ConversationModel.OpenAsync</c>) and the catch-up only ever asks for what is newer: nothing anywhere
    /// would ever read them again. The pass reads each once, by <c>before_id = id + 1, limit = 1</c>, and puts the server's
    /// set back — and with it the sticker and the circle.
    /// </summary>
    [Fact]
    public async Task APassReadsAgainWhatAnOlderBuildCachedAndPutsTheFlagsBack()
    {
        HeldByAnOlderBuild(1340, OldSticker);
        HeldByAnOlderBuild(1341, OldRound);
        Assert.Null(chats.Message(1340)!.StickerPicture);
        Assert.Null(chats.Message(1341)!.RoundVideo);
        Assert.Equal(1341, chats.CatchUpCursor(42));

        var (resync, server) = Build(id => id switch
        {
            1340 => Page(StickerJson),
            1341 => Page(RoundJson),
            _ => null,
        });
        var report = await resync.RunAsync();

        Assert.True(report.Complete);
        Assert.Equal(2, report.FlagsRepaired);
        Assert.Equal(90, chats.Message(1340)!.StickerPicture!.Id);
        Assert.Equal(91, chats.Message(1341)!.RoundVideo!.Id);
        // Newest first, one message each.
        Assert.Equal(
            ["/api/v1/chats/42/messages?before_id=1342&limit=1", "/api/v1/chats/42/messages?before_id=1341&limit=1"],
            server.Asked.Where(path => path.Contains("before_id", StringComparison.Ordinal)));
        // The repair moved no cursor and touched nothing else about the rows.
        Assert.Equal(1341, chats.CatchUpCursor(42));
        Assert.Equal(string.Empty, chats.Message(1340)!.Body);

        // And once is enough: the next pass asks about neither.
        server.Asked.Clear();
        Assert.Equal(0, (await resync.RunAsync()).FlagsRepaired);
        Assert.DoesNotContain(server.Asked, path => path.Contains("before_id", StringComparison.Ordinal));
    }

    /// <summary>
    /// ONLY WHAT COULD HAVE LOST A FLAG IS ASKED ABOUT. A message with words, an album, a voice note, a file, a message with
    /// no attachment and a set that already carries its flag draw the same either way, and are settled with no request; a
    /// caption-less photo or video could be a sticker or a circle, and is asked once.
    /// </summary>
    [Fact]
    public void OnlyABodylessSinglePhotoOrVideoWithNoFlagIsACandidate()
    {
        HeldByAnOlderBuild(1, OldSticker, body: "look at this");
        HeldByAnOlderBuild(2, """[{"id":1,"kind":"photo","mime":"image/jpeg","size":1,"has_preview":true},{"id":2,"kind":"photo","mime":"image/jpeg","size":1,"has_preview":true}]""");
        HeldByAnOlderBuild(3, """[{"id":3,"kind":"audio","mime":"audio/mp4","size":1,"has_preview":false}]""");
        HeldByAnOlderBuild(4, """[{"id":4,"kind":"file","mime":"application/pdf","size":1,"has_preview":false,"name":"a.pdf"}]""");
        HeldByAnOlderBuild(5, null);
        HeldByAnOlderBuild(6, """[{"id":6,"kind":"photo","mime":"image/webp","size":1,"has_preview":false,"sticker":true}]""");
        HeldByAnOlderBuild(7, OldSticker);
        HeldByAnOlderBuild(8, OldRound);

        Assert.Equal([8, 7], chats.FlagRepairCandidates(Resync.FlagRepairBatch).Select(message => message.Id));
        // Settled for good: asking again finds the same two and nothing else.
        Assert.Equal([8, 7], chats.FlagRepairCandidates(Resync.FlagRepairBatch).Select(message => message.Id));
    }

    /// <summary>At most a batch per pass, newest first — however many non-candidates lie between them.</summary>
    [Fact]
    public void CandidatesAreABatchNewestFirst()
    {
        for (var id = 1; id <= 60; id++)
        {
            HeldByAnOlderBuild(id, id % 2 == 0 ? OldSticker : OldRound, body: id % 3 == 0 ? "words" : "");
        }
        var found = chats.FlagRepairCandidates(Resync.FlagRepairBatch);
        var expected = Enumerable.Range(1, 60).Reverse().Where(id => id % 3 != 0).Take(Resync.FlagRepairBatch).Select(id => (long)id);
        Assert.Equal(expected, found.Select(message => message.Id));
    }

    /// <summary>
    /// Every set this build writes knows the flags, and a copy that brings a set settles a row an older build wrote — so a
    /// history page that happens to cover it repairs it without the pass ever asking.
    /// </summary>
    [Fact]
    public void ASetThisBuildWritesIsNeverACandidate()
    {
        HeldByAnOlderBuild(1340, OldSticker);
        chats.Apply(Wire.Decode<MessageDto>(StickerJson)!, SeqRoute.Evidence);
        Assert.Equal(90, chats.Message(1340)!.StickerPicture!.Id);
        Assert.Empty(chats.FlagRepairCandidates(Resync.FlagRepairBatch));

        // A copy with NO set (an edit feed's answer never lacks one, but absent is not empty) leaves the row unknown.
        HeldByAnOlderBuild(1341, OldRound);
        chats.Apply(new MessageDto(1341, 42, 9, null, "", "2026-10-05T10:01:00Z"), SeqRoute.Evidence, inSequence: false);
        Assert.Equal([1341], chats.FlagRepairCandidates(Resync.FlagRepairBatch).Select(message => message.Id));
    }

    /// <summary>
    /// ONE FAILED READ IS ABOUT ONE MESSAGE (iOS's and Android's rules): a refusal and an unreadable answer settle it, a
    /// message the server no longer has settles it, a 5xx leaves it for the next pass — and the pass goes on to the others.
    /// </summary>
    [Fact]
    public async Task ARefusalSettlesOneMessageAndA5xxIsAskedAgain()
    {
        HeldByAnOlderBuild(10, OldRound); // 403: a chat this device still caches and may no longer read
        HeldByAnOlderBuild(11, OldRound); // 404: lost
        HeldByAnOlderBuild(12, OldRound); // 502: no answer yet
        HeldByAnOlderBuild(13, OldRound); // the server no longer has it: an empty page
        HeldByAnOlderBuild(14, OldRound); // 200 with a body this build cannot read
        HeldByAnOlderBuild(1341, OldRound); // fine
        var (resync, server) = Build(id => id switch
        {
            10 => (HttpStatusCode.Forbidden, """{"error": {"code": "forbidden", "message": "no"}}"""),
            11 => (HttpStatusCode.NotFound, """{"error": {"code": "not_found", "message": "no"}}"""),
            12 => (HttpStatusCode.BadGateway, "<html>502</html>"),
            13 => Page(),
            14 => (HttpStatusCode.OK, "not json"),
            1341 => Page(RoundJson),
            _ => null,
        });

        var report = await resync.RunAsync();
        Assert.True(report.Complete);
        Assert.Equal(1, report.FlagsRepaired);
        Assert.Equal(91, chats.Message(1341)!.RoundVideo!.Id);
        Assert.Equal([12], chats.FlagRepairCandidates(Resync.FlagRepairBatch).Select(message => message.Id));
        // Every one of them was asked, the 5xx one twice (the client's one retry of a read).
        Assert.Equal(7, server.Asked.Count(path => path.Contains("before_id", StringComparison.Ordinal)));
    }

    /// <summary>
    /// A failure that says nothing about the one message — no network, a lost session, a server asking us to slow down —
    /// ENDS the pass: every further read would fail the same way. Nothing is settled, and the pass itself is not failed.
    /// </summary>
    [Theory]
    [InlineData(HttpStatusCode.Unauthorized)]
    [InlineData(HttpStatusCode.TooManyRequests)]
    [InlineData((HttpStatusCode)0)]
    public async Task ANetworkOrSessionFailureEndsThePassAndSettlesNothing(HttpStatusCode status)
    {
        HeldByAnOlderBuild(1340, OldSticker);
        HeldByAnOlderBuild(1341, OldRound);
        var (resync, server) = Build(_ => status == 0
            ? null // the fake throws HttpRequestException: a transport failure
            : (status, status == HttpStatusCode.Unauthorized
                ? """{"error": {"code": "unauthorized", "message": "no"}}"""
                : null));

        var report = await resync.RunAsync();
        Assert.True(report.Complete);
        Assert.Equal(0, report.FlagsRepaired);
        // The newest was asked (twice, where the client retries a read once); the older one never was.
        var asked = server.Asked.Where(path => path.Contains("before_id", StringComparison.Ordinal)).ToList();
        Assert.NotEmpty(asked);
        Assert.All(asked, path => Assert.Equal("/api/v1/chats/42/messages?before_id=1342&limit=1", path));
        Assert.Equal([1341, 1340], chats.FlagRepairCandidates(Resync.FlagRepairBatch).Select(message => message.Id));
    }

    [Theory]
    [InlineData("__transport", 0, Resync.FlagRepairOutcome.EndPass)]
    [InlineData("unauthorized", 401, Resync.FlagRepairOutcome.EndPass)]
    [InlineData("too_many_requests", 429, Resync.FlagRepairOutcome.EndPass)]
    [InlineData("", 429, Resync.FlagRepairOutcome.EndPass)]
    [InlineData("internal", 500, Resync.FlagRepairOutcome.AskAgainLater)]
    [InlineData("", 502, Resync.FlagRepairOutcome.AskAgainLater)]
    [InlineData("", 408, Resync.FlagRepairOutcome.AskAgainLater)]
    [InlineData("forbidden", 403, Resync.FlagRepairOutcome.Settled)]
    [InlineData("not_found", 404, Resync.FlagRepairOutcome.Settled)]
    [InlineData("conflict", 409, Resync.FlagRepairOutcome.Settled)]
    [InlineData("validation", 200, Resync.FlagRepairOutcome.Settled)]
    public void WhatOneFailedReadMeans(string code, int status, Resync.FlagRepairOutcome expected) =>
        Assert.Equal(expected, Resync.RepairOutcome(new ApiError(code, "", status)));
}
