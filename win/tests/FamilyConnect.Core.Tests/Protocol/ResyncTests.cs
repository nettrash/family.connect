using System.Net;
using System.Text;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.Core.Tests.Protocol;

/// <summary>
/// What a client does on every (re)connect, in the protocol's own order — and the two rules in it
/// that only look small (docs/protocol.md, "Best-effort delivery").
/// </summary>
public class ResyncTests : IDisposable
{
    private readonly Database database = Database.OpenInMemory();

    public void Dispose() => database.Dispose();

    /// <summary>Answers by path, and remembers every path it was asked for, in order.</summary>
    private sealed class Server : HttpMessageHandler
    {
        private readonly List<Func<string, (HttpStatusCode Status, string? Json)?>> routes = [];

        public List<string> Asked { get; } = [];

        public Server On(Func<string, (HttpStatusCode, string?)?> route)
        {
            routes.Add(route);
            return this;
        }

        /// <summary>A fixed answer for the one endpoint <paramref name="what"/> names.</summary>
        /// <remarks>
        /// The match is the WHOLE path, query aside, and it is anchored for a reason: a route that
        /// matched a substring answered `/chats/42/messages` with `/me`'s body — `/messages`
        /// contains `/me` — and a fake that lies about which endpoint was asked can only teach the
        /// wrong lesson. It did: four tests failed inside the product, on an answer no server
        /// would send.
        /// </remarks>
        public Server Always(string what, string json, HttpStatusCode status = HttpStatusCode.OK) =>
            On(path => Endpoint(path) == "/api/v1" + what ? (status, json) : null);

        private static string Endpoint(string pathAndQuery)
        {
            var query = pathAndQuery.IndexOf('?', StringComparison.Ordinal);
            return query < 0 ? pathAndQuery : pathAndQuery[..query];
        }

        protected override Task<HttpResponseMessage> SendAsync(
            HttpRequestMessage request, CancellationToken cancellationToken)
        {
            var path = request.RequestUri!.PathAndQuery;
            Asked.Add(path);
            foreach (var route in routes)
            {
                if (route(path) is { } answer)
                {
                    var response = new HttpResponseMessage(answer.Status);
                    if (answer.Json is not null)
                    {
                        response.Content = new StringContent(answer.Json, Encoding.UTF8, "application/json");
                    }
                    return Task.FromResult(response);
                }
            }
            throw new InvalidOperationException($"no route for {path}");
        }
    }

    private const string Me =
        """
        {"user": {"id": 7, "username": "anna", "display_name": "Anna"},
         "family": {"id": 3, "name": "The Smiths"}, "role": "owner",
         "blocked_user_ids": []}
        """;

    private static string FamilyWall(string mark) =>
        $$"""
        {"family": {"id": 3, "name": "The Smiths"},
         "members": [{"id": 7, "username": "anna", "display_name": "Anna", "role": "owner"}]
         {{mark}}}
        """;

    /// <summary>A family whose wall has been written on — `max_board_seq` present.</summary>
    private static readonly string Family = FamilyWall(""", "max_board_seq": 11""");

    /// <summary>A family whose wall is empty and UNTOUCHED: the mark is absent.</summary>
    private static readonly string FamilyWithNoWall = FamilyWall("");

    private static string Message(long id, long chat = 42, string body = "hi") =>
        $$"""
        {"id": {{id}}, "chat_id": {{chat}}, "sender_id": 7, "client_msg_id": null,
         "body": "{{body}}", "created_at": "2026-09-12T10:00:00Z"}
        """;

    private (Resync Resync, Server Handler, ChatStore Chats, BoardStore Board) Build(Server server)
    {
        var api = new ApiClient(
            new HttpClient(server), ServerUrl.Normalise("chat.example.com")!,
            new MemoryTokenStore("t0ken"));
        var chats = new ChatStore(database);
        var board = new BoardStore(database);
        return (new Resync(api, chats, board), server, chats, board);
    }

    [Fact]
    public async Task ThePassGoesInTheDocumentedOrder()
    {
        var server = new Server()
            .Always("/me", Me)
            .Always("/families/mine/board/changes", """{"notes": []}""")
            .Always("/families/mine/board", """{"notes": [], "max_board_seq": 0}""")
            .Always("/families/mine", Family)
            .Always("/chats/42/messages", """{"messages": []}""")
            .Always("/chats", """{"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths"}}]}""");
        var (resync, handler, _, _) = Build(server);

        var report = await resync.RunAsync();
        Assert.True(report.Complete);
        Assert.True(report.Signed);
        Assert.True(report.HasFamily);
        Assert.Equal(1, report.Chats);
        // A chat holding nothing has no hole to fill, and so no message catch-up: it is read from its newest page when
        // it is opened.
        Assert.Equal(
            [
                "/api/v1/me",
                "/api/v1/families/mine",
                "/api/v1/chats",
                "/api/v1/families/mine/board",
            ],
            handler.Asked);
    }

    [Fact]
    public async Task AnAccountWithNoFamilyAsksForNeitherTheRosterNorTheWall()
    {
        var server = new Server()
            .Always("/me", """{"user": {"id": 7, "username": "anna", "display_name": "Anna"}, "blocked_user_ids": []}""")
            .Always("/chats", """{"chats": []}""");
        var (resync, handler, _, _) = Build(server);

        var report = await resync.RunAsync();
        Assert.True(report.Complete);
        Assert.False(report.HasFamily);
        // `GET /families/mine` would answer `not_in_family`, and the wall belongs to a family.
        Assert.Equal(["/api/v1/me", "/api/v1/chats"], handler.Asked);
    }

    /// <summary>
    /// THE CURSOR BELONGS TO THE LOOP: read once, then advanced by the largest id each page
    /// actually returned. Re-reading the store between pages lets a live message arriving mid-loop
    /// jump the cursor, and every message between the last page and it is skipped — permanently,
    /// because `after_id` can never look back.
    /// </summary>
    [Fact]
    public async Task TheMessageCursorFollowsThePagesAndNotTheStore()
    {
        var pages = 0;
        ChatStore? store = null;
        var server = new Server()
            .Always("/me", Me)
            .Always("/families/mine/board", """{"notes": [], "max_board_seq": 0}""")
            .Always("/families/mine", Family)
            .Always("/chats", """{"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths"}}]}""")
            .On(path =>
            {
                if (!path.Contains("/chats/42/messages", StringComparison.Ordinal))
                {
                    return null;
                }
                pages++;
                if (pages == 1)
                {
                    // A full page of fifty, ids 2..51 — and, as it is answered, a LIVE message
                    // with id 900 lands from the socket that started this resync. The store's
                    // max(id) is 900 from here on; the loop's cursor must still be 51.
                    store!.Apply(Wire.Decode<MessageDto>(Message(900))!, SeqRoute.LiveFrame);
                    var fifty = string.Join(",", Enumerable.Range(2, 50).Select(id => Message(id)));
                    return (HttpStatusCode.OK, $$"""{"messages": [{{fifty}}]}""");
                }
                // The second page must be asked for from 51 — the largest id page one RETURNED —
                // and not from 900, which is what the store now says.
                return path.Contains("after_id=51", StringComparison.Ordinal)
                    ? (HttpStatusCode.OK, $$"""{"messages": [{{Message(52)}}]}""")
                    : (HttpStatusCode.OK, """{"messages": []}""");
            });
        var (resync, handler, chats, _) = Build(server);
        store = chats;
        chats.Replace([new ChatRowDto(new ChatDto(42, "family", "The Smiths"))]);
        // The one message this device already held, so that there is a cursor to follow.
        chats.Apply(Wire.Decode<MessageDto>(Message(1))!);

        var report = await resync.RunAsync();
        Assert.True(report.Complete);
        Assert.Equal(51, report.Messages);
        Assert.Contains("/api/v1/chats/42/messages?after_id=1&limit=50", handler.Asked);
        Assert.Contains("/api/v1/chats/42/messages?after_id=51&limit=50", handler.Asked);
        // Nothing was skipped: 52 is held, which is the message the cursor would have jumped over.
        Assert.NotNull(chats.Message(52));
        Assert.NotNull(chats.Message(900));
    }

    [Fact]
    public async Task TheMessageLoopStopsOnAShortPage()
    {
        var server = new Server()
            .Always("/me", Me)
            .Always("/families/mine/board", """{"notes": [], "max_board_seq": 0}""")
            .Always("/families/mine", Family)
            .Always("/chats", """{"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths"}}]}""")
            .Always("/chats/42/messages", $$"""{"messages": [{{Message(2)}}, {{Message(3)}}]}""");
        var (resync, handler, chats, _) = Build(server);
        chats.Replace([new ChatRowDto(new ChatDto(42, "family", "The Smiths"))]);
        chats.Apply(Wire.Decode<MessageDto>(Message(1))!);

        var report = await resync.RunAsync();
        Assert.Equal(2, report.Messages);
        // Two of fifty is short, so one request only.
        Assert.Single(handler.Asked, path => path.Contains("messages", StringComparison.Ordinal));
    }

    /// <summary>
    /// THE LIST'S PREVIEW IS NOT WHERE THE CATCH-UP STARTS. Step 2 stores each chat's newest message, trimmed, beside the
    /// ones pages delivered; counted in `max(id)` it made `after_id` the server's newest id, the page came back empty, and
    /// the Windows client lost a day of a family chat for good (docs/protocol.md, "Best-effort delivery", step 3).
    /// </summary>
    [Fact]
    public async Task TheListsPreviewIsNotWhereTheCatchUpStarts()
    {
        var ten = string.Join(",", Enumerable.Range(11, 10).Select(id => Message(id)));
        var server = new Server()
            .Always("/me", Me)
            .Always("/families/mine/board", """{"notes": [], "max_board_seq": 0}""")
            .Always("/families/mine", Family)
            .Always("/chats", $$"""
                {"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths"},
                            "last_message": {{Message(20)}}}]}
                """)
            .On(path => !path.Contains("/chats/42/messages", StringComparison.Ordinal)
                ? null
                : path.Contains("after_id=10&", StringComparison.Ordinal)
                    ? (HttpStatusCode.OK, $$"""{"messages": [{{ten}}]}""")
                    : (HttpStatusCode.OK, """{"messages": []}"""));
        var (resync, handler, chats, _) = Build(server);
        chats.Replace([new ChatRowDto(new ChatDto(42, "family", "The Smiths"))]);
        chats.Apply(Enumerable.Range(1, 10).Select(id => Wire.Decode<MessageDto>(Message(id))!).ToList());

        var report = await resync.RunAsync();
        Assert.True(report.Complete);
        Assert.Contains("/api/v1/chats/42/messages?after_id=10&limit=50", handler.Asked);
        Assert.Equal(10, report.Messages);
        Assert.All(Enumerable.Range(11, 10), id => Assert.NotNull(chats.Message(id)));

        // Once a page has delivered it, the preview IS held in sequence, and the next pass starts from it.
        handler.Asked.Clear();
        Assert.True((await resync.RunAsync()).Complete);
        Assert.Contains("/api/v1/chats/42/messages?after_id=20&limit=50", handler.Asked);
    }

    /// <summary>
    /// A chat that holds nothing but its preview has no hole to fill, so it has no message catch-up: opening it reads the
    /// newest page down, and a request here would page a family's whole history from `after_id=0` on a new device.
    /// </summary>
    [Fact]
    public async Task AChatHoldingOnlyItsPreviewIsLeftToHistoryPaging()
    {
        var server = new Server()
            .Always("/me", Me)
            .Always("/families/mine/board", """{"notes": [], "max_board_seq": 0}""")
            .Always("/families/mine", Family)
            .Always("/chats", $$"""
                {"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths"},
                            "last_message": {{Message(20)}}}]}
                """);
        var (resync, handler, chats, _) = Build(server);

        Assert.True((await resync.RunAsync()).Complete);
        Assert.DoesNotContain(handler.Asked, path => path.Contains("/messages", StringComparison.Ordinal));
        // The row still draws itself from it.
        Assert.Equal(20, chats.Newest(42)!.Id);
        Assert.Null(chats.CatchUpCursor(42));
    }

    /// <summary>
    /// THE CURSORS ARE TAKEN WHEN THE CONNECTION OPENS. The pass reaches the messages several round trips later, and a live
    /// message landing in between must not become where it starts: everything missed while the socket was down is below
    /// it. A second opening before the pass keeps the EARLIER cursors, and a pass that could not finish hands its own on.
    /// </summary>
    [Fact]
    public async Task TheCursorsAreTheOnesTakenWhenTheConnectionOpened()
    {
        var failing = true;
        var server = new Server()
            .Always("/me", Me)
            .Always("/families/mine/board", """{"notes": [], "max_board_seq": 0}""")
            .Always("/families/mine", Family)
            .Always("/chats", """
                {"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths"}},
                           {"chat": {"id": 43, "kind": "direct", "title": "Bob"}}]}
                """)
            .Always("/chats/43/messages", """{"messages": []}""")
            .On(path => !path.Contains("/chats/42/messages", StringComparison.Ordinal)
                ? null
                : failing
                    ? (HttpStatusCode.InternalServerError, null)
                    : (HttpStatusCode.OK, """{"messages": []}"""));
        var (resync, handler, chats, _) = Build(server);
        chats.Replace([
            new ChatRowDto(new ChatDto(42, "family", "The Smiths")),
            new ChatRowDto(new ChatDto(43, "direct", "Bob")),
        ]);
        chats.Apply(Enumerable.Range(1, 10).Select(id => Wire.Decode<MessageDto>(Message(id))!).ToList());

        // A connection opens at 10; frames on it move the store to 15 — and give chat 43, which held nothing, its first
        // message — and it opens AGAIN before the pass runs. The earlier of the two is kept, per chat...
        resync.Snapshot();
        chats.Apply(Enumerable.Range(11, 5).Select(id => Wire.Decode<MessageDto>(Message(id))!).ToList(), SeqRoute.LiveFrame);
        chats.Apply(Wire.Decode<MessageDto>(Message(50, chat: 43))!, SeqRoute.LiveFrame);
        resync.Snapshot();
        // ...and a live message lands before the pass reaches the messages.
        chats.Apply(Wire.Decode<MessageDto>(Message(900))!, SeqRoute.LiveFrame);

        Assert.False((await resync.RunAsync()).Complete);
        Assert.Contains("/api/v1/chats/42/messages?after_id=10&limit=50", handler.Asked);

        // The pass that could not finish handed its cursors on: the next one does not start from 900 either.
        handler.Asked.Clear();
        failing = false;
        Assert.True((await resync.RunAsync()).Complete);
        Assert.Contains("/api/v1/chats/42/messages?after_id=10&limit=50", handler.Asked);
        // And the chat only the second opening had a cursor for is caught up from it, not forgotten by the merge.
        Assert.Contains("/api/v1/chats/43/messages?after_id=50&limit=50", handler.Asked);

        // A pass that finished hands nothing on: the one after it takes its own, from what is held now.
        handler.Asked.Clear();
        Assert.True((await resync.RunAsync()).Complete);
        Assert.Contains("/api/v1/chats/42/messages?after_id=900&limit=50", handler.Asked);
    }

    /// <summary>
    /// The reaction catch-up is asked for only when the chat says there is something to catch up
    /// to, and its cursor advances per PAGE — even for messages this device does not hold, whose
    /// states are dropped.
    /// </summary>
    [Fact]
    public async Task TheReactionCatchUpRunsOnlyWhenTheChatSaysSoAndAdvancesPerPage()
    {
        var server = new Server()
            .Always("/me", Me)
            .Always("/families/mine/board", """{"notes": [], "max_board_seq": 0}""")
            .Always("/families/mine", Family)
            .Always("/chats", """
                {"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths"},
                            "max_reaction_seq": 124}]}
                """)
            .Always("/chats/42/messages", """{"messages": []}""")
            .Always("/chats/42/reactions", """
                {"message_reactions": [
                    {"message_id": 1338, "reaction_seq": 123, "reactions": [{"user_id": 9, "emoji": "❤️"}]},
                    {"message_id": 9999, "reaction_seq": 124, "reactions": [{"user_id": 9, "emoji": "👍"}]}]}
                """);
        var (resync, handler, chats, _) = Build(server);
        chats.Replace([new ChatRowDto(new ChatDto(42, "family", "The Smiths"))]);
        chats.Apply(Wire.Decode<MessageDto>(Message(1338))!);

        var report = await resync.RunAsync();
        Assert.Equal(2, report.Reactions);
        Assert.Contains("/api/v1/chats/42/reactions?after_seq=0&limit=50", handler.Asked);
        // The state for the message this device holds was applied…
        Assert.Equal("❤️", Assert.Single(chats.Message(1338)!.Reactions!).Emoji);
        // …the one for a message it has never seen was dropped, and the cursor still moved past
        // both, which is what stops the next pass asking for them again.
        Assert.Null(chats.Message(9999));
        Assert.Equal(124, chats.Chat(42)!.MaxReactionSeq);

        // And a second pass asks for nothing: the chat's maximum is no longer above the cursor.
        handler.Asked.Clear();
        await resync.RunAsync();
        Assert.DoesNotContain(
            handler.Asked, path => path.Contains("reactions", StringComparison.Ordinal));
    }

    /// <summary>
    /// The edit catch-up is the ONLY step that learns of a change to a message this device
    /// already holds: step 3's `after_id` is `WHERE id > cursor` and can never look at an older
    /// row, so without this feed a device that slept through an edit shows the old words until
    /// something else happens to that message.
    /// </summary>
    [Fact]
    public async Task TheEditCatchUpIsTheOnlyWayAnOlderMessageIsKnownToHaveChanged()
    {
        var server = new Server()
            .Always("/me", Me)
            .Always("/families/mine/board", """{"notes": [], "max_board_seq": 0}""")
            .Always("/families/mine", Family)
            .Always("/chats", """
                {"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths"},
                            "max_edit_seq": 5}]}
                """)
            // Nothing NEWER than what is held — the message that changed is three pages back.
            .Always("/chats/42/messages", """{"messages": []}""")
            .Always("/chats/42/edits", $$"""
                {"messages": [{"id": 10, "chat_id": 42, "sender_id": 7, "body": "Bins on Tuesday",
                               "created_at": "2026-09-12T10:00:00Z",
                               "edited_at": "2026-09-12T11:00:00Z", "edit_seq": 5}]}
                """);
        var (resync, handler, chats, _) = Build(server);
        chats.Replace([new ChatRowDto(new ChatDto(42, "family", "The Smiths"))]);
        chats.Apply(Wire.Decode<MessageDto>(Message(10, body: "Bins"))!);

        var report = await resync.RunAsync();
        Assert.Equal(1, report.Edits);
        Assert.Contains("/api/v1/chats/42/edits?after_seq=0&limit=50", handler.Asked);
        // The whole message was applied, through the path a page of history goes through.
        Assert.Equal("Bins on Tuesday", chats.Message(10)!.Body);
        Assert.Equal(5, chats.Chat(42)!.MaxEditSeq);

        // And the next pass asks for nothing: the chat's maximum is no longer above the cursor.
        handler.Asked.Clear();
        await resync.RunAsync();
        Assert.DoesNotContain(handler.Asked, path => path.Contains("edits", StringComparison.Ordinal));
    }

    /// <summary>
    /// A connection that opens WHILE a pass is failing and the pass's own starting cursors are MERGED, per chat, the earlier
    /// kept — neither replaces the other. Chat 42's pass started from 10 although frame 900 was already held above a gap:
    /// the reconnection's 900 alone would skip that gap. Chat 43 held nothing when the pass began; a frame gave it 50
    /// before the socket dropped, and the reconnection took that: the pass's cursors alone would skip the chat, and
    /// everything sent to it while the socket was down would sit for good below the frames that came after.
    /// </summary>
    [Fact]
    public async Task AConnectionThatOpensDuringAFailedPassIsNotForgotten()
    {
        var failing = true;
        Resync? running = null;
        ChatStore? store = null;
        var server = new Server()
            .Always("/me", Me)
            .Always("/families/mine/board", """{"notes": [], "max_board_seq": 0}""")
            .Always("/families/mine", Family)
            .Always("/chats", """
                {"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths"}},
                           {"chat": {"id": 43, "kind": "direct", "title": "Bob"}}]}
                """)
            .Always("/chats/43/messages", """{"messages": []}""")
            .On(path =>
            {
                if (!path.Contains("/chats/42/messages", StringComparison.Ordinal))
                {
                    return null;
                }
                if (!failing)
                {
                    return (HttpStatusCode.OK, """{"messages": []}""");
                }
                // Mid-pass: a frame for chat 43, the socket drops and opens again — and then this read fails.
                store!.Apply(Wire.Decode<MessageDto>(Message(50, chat: 43))!, SeqRoute.LiveFrame);
                running!.Snapshot();
                return (HttpStatusCode.InternalServerError, null);
            });
        var (resync, handler, chats, _) = Build(server);
        running = resync;
        store = chats;
        chats.Replace([
            new ChatRowDto(new ChatDto(42, "family", "The Smiths")),
            new ChatRowDto(new ChatDto(43, "direct", "Bob")),
        ]);
        chats.Apply(Wire.Decode<MessageDto>(Message(10))!);
        // A connection opens at 10, and a live message lands on it before the pass.
        resync.Snapshot();
        chats.Apply(Wire.Decode<MessageDto>(Message(900))!, SeqRoute.LiveFrame);

        Assert.False((await resync.RunAsync()).Complete);
        // A frame on the new connection, before the next pass.
        chats.Apply(Wire.Decode<MessageDto>(Message(60, chat: 43))!, SeqRoute.LiveFrame);

        handler.Asked.Clear();
        failing = false;
        Assert.True((await resync.RunAsync()).Complete);
        Assert.Contains("/api/v1/chats/42/messages?after_id=10&limit=50", handler.Asked);
        Assert.Contains("/api/v1/chats/43/messages?after_id=50&limit=50", handler.Asked);
    }

    /// <summary>
    /// Cursors a connection handed over are held to what the cache still holds: a cache wiped since — a sign-out, another
    /// account, another server — is never paged from a stranger's cursor, past what its own messages reach.
    /// </summary>
    [Fact]
    public async Task HandedOverCursorsAreHeldToWhatTheCacheStillHolds()
    {
        var server = new Server()
            .Always("/me", Me)
            .Always("/families/mine/board", """{"notes": [], "max_board_seq": 0}""")
            .Always("/families/mine", Family)
            .Always("/chats", """
                {"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths"}},
                           {"chat": {"id": 43, "kind": "direct", "title": "Bob"}}]}
                """)
            .Always("/chats/42/messages", """{"messages": []}""")
            .Always("/chats/43/messages", """{"messages": []}""");
        var (resync, handler, chats, _) = Build(server);
        ChatRowDto[] rows =
        [
            new ChatRowDto(new ChatDto(42, "family", "The Smiths")),
            new ChatRowDto(new ChatDto(43, "direct", "Bob")),
        ];
        chats.Replace(rows);
        chats.Apply([Wire.Decode<MessageDto>(Message(1200))!, Wire.Decode<MessageDto>(Message(70, chat: 43))!]);
        resync.Snapshot();

        // The cache is wiped; the next copy of chat 42 reaches only 1000, and of chat 43 it holds nothing.
        database.WipeAll();
        chats.Replace(rows);
        chats.Apply(Wire.Decode<MessageDto>(Message(1000))!);

        Assert.True((await resync.RunAsync()).Complete);
        Assert.Contains("/api/v1/chats/42/messages?after_id=1000&limit=50", handler.Asked);
        Assert.DoesNotContain(handler.Asked, path => path.Contains("/chats/43/messages", StringComparison.Ordinal));
    }

    /// <summary>
    /// A copy from the edits feed is held and drawn, and it is not where the next catch-up starts: an edited message may be
    /// one this device never held, and it says nothing about the messages before it (docs/protocol.md, "Best-effort
    /// delivery", step 3).
    /// </summary>
    [Fact]
    public async Task AnEditedCopyIsNotWhereTheNextCatchUpStarts()
    {
        var server = new Server()
            .Always("/me", Me)
            .Always("/families/mine/board", """{"notes": [], "max_board_seq": 0}""")
            .Always("/families/mine", Family)
            .Always("/chats", """
                {"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths"},
                            "max_edit_seq": 5}]}
                """)
            // A server whose pages have not caught up with its edits feed: whatever the reason, the copy the feed carries
            // must not move the cursor past what the pages have not delivered.
            .Always("/chats/42/messages", """{"messages": []}""")
            .Always("/chats/42/edits", """
                {"messages": [{"id": 30, "chat_id": 42, "sender_id": 9, "body": "Bins on Tuesday",
                               "created_at": "2026-09-12T10:00:00Z",
                               "edited_at": "2026-09-12T11:00:00Z", "edit_seq": 5}]}
                """);
        var (resync, _, chats, _) = Build(server);
        chats.Replace([new ChatRowDto(new ChatDto(42, "family", "The Smiths"))]);
        chats.Apply(Wire.Decode<MessageDto>(Message(10))!);

        Assert.True((await resync.RunAsync()).Complete);
        Assert.Equal("Bins on Tuesday", chats.Message(30)!.Body);
        Assert.Equal(10, chats.CatchUpCursor(42));
    }

    /// <summary>
    /// The messages catch-up delivers only what is newer than everything held, so a reply on it raises its root; the edits
    /// catch-up delivers copies the root's recomputed count already includes, and raises nothing (docs/protocol.md,
    /// "Threads").
    /// </summary>
    [Fact]
    public async Task OnlyTheMessagesCatchUpRaisesARootsCount()
    {
        var server = new Server()
            .Always("/me", Me)
            .Always("/families/mine/board", """{"notes": [], "max_board_seq": 0}""")
            .Always("/families/mine", Family)
            .Always("/chats", """
                {"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths"},
                            "max_edit_seq": 5}]}
                """)
            .Always("/chats/42/messages", """
                {"messages": [{"id": 12, "chat_id": 42, "sender_id": 9, "body": "at 8",
                               "created_at": "2026-09-12T10:02:00Z", "thread_root_id": 9}]}
                """)
            .Always("/chats/42/edits", """
                {"messages": [{"id": 11, "chat_id": 42, "sender_id": 9, "body": "at 7!",
                               "created_at": "2026-09-12T10:01:00Z", "thread_root_id": 9,
                               "edited_at": "2026-09-12T11:00:00Z", "edit_seq": 5}]}
                """);
        var (resync, _, chats, _) = Build(server);
        chats.Replace([new ChatRowDto(new ChatDto(42, "family", "The Smiths"))]);
        chats.Apply(Wire.Decode<MessageDto>(Message(9, body: "Dinner?"))! with { ReplyCount = 1 }, FamilyConnect.Core.Store.SeqRoute.Evidence);

        await resync.RunAsync();

        Assert.Equal("at 7!", chats.Message(11)!.Body);
        Assert.Equal(2, chats.Message(9)!.ReplyCount);
    }

    /// <summary>
    /// The three feeds are asked for in the protocol's own order, after the messages and before
    /// the wall — and each only when its own mark is above its own cursor.
    /// </summary>
    [Fact]
    public async Task TheThreeFeedsFollowTheMessagesInTheDocumentedOrder()
    {
        var server = new Server()
            .Always("/me", Me)
            .Always("/families/mine/board", """{"notes": [], "max_board_seq": 0}""")
            .Always("/families/mine", Family)
            .Always("/chats", """
                {"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths"},
                            "max_reaction_seq": 1, "max_edit_seq": 2, "max_poll_seq": 3}]}
                """)
            .Always("/chats/42/messages", """{"messages": []}""")
            .Always("/chats/42/reactions", """{"message_reactions": []}""")
            .Always("/chats/42/edits", """{"messages": []}""")
            .Always("/chats/42/polls", """{"polls": []}""");
        var (resync, handler, chats, _) = Build(server);
        chats.Replace([new ChatRowDto(new ChatDto(42, "family", "The Smiths"))]);
        chats.Apply(Wire.Decode<MessageDto>(Message(1))!);

        Assert.True((await resync.RunAsync()).Complete);
        Assert.Equal(
            [
                "/api/v1/me",
                "/api/v1/families/mine",
                "/api/v1/chats",
                "/api/v1/chats/42/messages?after_id=1&limit=50",
                "/api/v1/chats/42/reactions?after_seq=0&limit=50",
                "/api/v1/chats/42/edits?after_seq=0&limit=50",
                "/api/v1/chats/42/polls?after_seq=0&limit=50",
                "/api/v1/families/mine/board",
            ],
            handler.Asked);
    }

    /// <summary>
    /// A FULL PAGE THAT CANNOT MOVE THE CURSOR ENDS THE LOOP. Every feed answers strictly newer
    /// rows, so this cannot happen against a server keeping its word — and "cannot happen" is
    /// exactly the shape of a loop that asks the same question until the process dies.
    /// </summary>
    [Fact]
    public async Task APageThatCannotAdvanceTheCursorStopsTheLoopRatherThanSpinningIt()
    {
        var pages = 0;
        // Fifty states, every one of them at seq 7: a full page, and one that says nothing new
        // after the first. The fake gives up after six so that a client which does not stop
        // fails this test instead of hanging it.
        var fifty = string.Join(",", Enumerable.Range(1, 50).Select(id =>
            $$"""{"message_id": {{id}}, "reaction_seq": 7, "reactions": []}"""));
        var server = new Server()
            .Always("/me", Me)
            .Always("/families/mine/board", """{"notes": [], "max_board_seq": 0}""")
            .Always("/families/mine", Family)
            .Always("/chats", """
                {"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths"},
                            "max_reaction_seq": 9}]}
                """)
            .Always("/chats/42/messages", """{"messages": []}""")
            .On(path => !path.Contains("/reactions", StringComparison.Ordinal)
                ? null
                : ++pages > 6
                    ? (HttpStatusCode.OK, """{"message_reactions": []}""")
                    : (HttpStatusCode.OK, $$"""{"message_reactions": [{{fifty}}]}"""));
        var (resync, _, chats, _) = Build(server);

        var report = await resync.RunAsync();
        Assert.True(report.Complete);
        // Twice: once from 0, once from 7 — and the second answer moved nothing, so that is that.
        Assert.Equal(2, pages);
        Assert.Equal(7, chats.Chat(42)!.MaxReactionSeq);
    }

    [Fact]
    public async Task TheWallIsReadWholeOnceAndThenAsChanges()
    {
        var server = new Server()
            .Always("/me", Me)
            .Always("/families/mine/board/changes", """
                {"notes": [{"id": 13, "author_id": 7, "text": "Bins", "board_seq": 12}]}
                """)
            .Always("/families/mine/board", """
                {"notes": [{"id": 12, "author_id": 7, "text": "Milk", "board_seq": 11}],
                 "max_board_seq": 11}
                """)
            .Always("/families/mine", FamilyWall(""", "max_board_seq": 12"""))
            .Always("/chats", """{"chats": []}""");
        var (resync, handler, _, board) = Build(server);

        var first = await resync.RunAsync();
        Assert.Equal(1, first.Notes);
        Assert.Equal(11, board.Cursor);
        Assert.Contains("/api/v1/families/mine/board", handler.Asked);

        // With a cursor in hand the next pass asks for CHANGES, from where it left off.
        handler.Asked.Clear();
        var second = await resync.RunAsync();
        Assert.Equal(1, second.Notes);
        Assert.Contains("/api/v1/families/mine/board/changes?after_seq=11&limit=50", handler.Asked);
        Assert.Equal(12, board.Cursor);
        Assert.Equal(2, board.Notes().Count);
    }

    /// <summary>
    /// The block list is read in FULL on every pass, from both reads that carry it, and stored as
    /// complete state — there is no catch-up feed for it and so nothing to miss. An absent list
    /// means nobody, which is the one read where absence is not "leave what you hold alone".
    /// </summary>
    [Fact]
    public async Task TheBlockListIsReadInFullOnEveryPass()
    {
        var server = new Server()
            .Always("/me", """
                {"user": {"id": 7, "username": "anna", "display_name": "Anna"},
                 "family": {"id": 3, "name": "The Smiths"}, "role": "owner",
                 "blocked_user_ids": [11, 14]}
                """)
            .Always("/families/mine", FamilyWithNoWall)
            .Always("/chats", """{"chats": []}""");
        var (resync, _, chats, _) = Build(server);
        chats.ReplaceBlocked([99]);

        Assert.True((await resync.RunAsync()).Complete);
        // Replaced whole: the id this device held and the server no longer names is gone.
        Assert.Equal([11L, 14L], chats.Blocked());
        Assert.False(chats.IsBlocked(99));

        // The family's own document carries the same list, and when it is THERE it applies —
        // when it is not, it is an older server and not an empty list, so the value `/me` just
        // gave stands rather than being cleared by a document that never mentioned it.
        var loud = new Server()
            .Always("/me", """
                {"user": {"id": 7, "username": "anna", "display_name": "Anna"},
                 "family": {"id": 3, "name": "The Smiths"}, "role": "owner",
                 "blocked_user_ids": [11, 14]}
                """)
            .Always("/families/mine", FamilyWall(""", "blocked_user_ids": [14]"""))
            .Always("/chats", """{"chats": []}""")
            .Always("/families/mine/board", """{"notes": [], "max_board_seq": 0}""");
        var (again, _, held, _) = Build(loud);
        Assert.True((await again.RunAsync()).Complete);
        Assert.Equal([14L], held.Blocked());
    }

    /// <summary>
    /// `max_board_seq` is the one published mark for the wall, and it is ABSENT while the wall is
    /// empty and untouched — which is what it is for: a client reads it to know whether a board
    /// request is worth making. Level with it, or no mark at all, and nothing is asked for.
    /// </summary>
    [Fact]
    public async Task AWallNobodyHasTouchedIsNotAskedForAndNeitherIsOneAlreadyLevel()
    {
        var server = new Server()
            .Always("/me", Me)
            .Always("/families/mine", FamilyWithNoWall)
            .Always("/chats", """{"chats": []}""");
        var (resync, handler, _, _) = Build(server);

        // No mark, nothing held: no request at all — and there is no route for one, so a client
        // that asked would fail this test rather than quietly cost a round trip.
        Assert.True((await resync.RunAsync()).Complete);
        Assert.Equal(["/api/v1/me", "/api/v1/families/mine", "/api/v1/chats"], handler.Asked);

        // Now the same family with a wall, read once…
        var wall = new Server()
            .Always("/me", Me)
            .Always("/families/mine", Family)
            .Always("/chats", """{"chats": []}""")
            .Always("/families/mine/board", """
                {"notes": [{"id": 12, "author_id": 7, "text": "Milk", "board_seq": 11}],
                 "max_board_seq": 11}
                """);
        var (again, asked, _, held) = Build(wall);
        Assert.Equal(1, (await again.RunAsync()).Notes);
        Assert.Equal(11, held.Cursor);

        // …and a second pass asks for neither the wall nor its changes: the mark it just read
        // says this device has applied everything there has been.
        asked.Asked.Clear();
        Assert.True((await again.RunAsync()).Complete);
        Assert.DoesNotContain(asked.Asked, path => path.Contains("board", StringComparison.Ordinal));
    }

    /// <summary>
    /// THE FLUSH IS NOT A STEP. A client that could not even sign in must still flush what it
    /// holds: putting the one operation that recovers a stuck message behind the reads most likely
    /// to fail on the network that stuck it is exactly the bug both of this app's ports shipped.
    /// </summary>
    [Fact]
    public async Task TheOutboxIsFlushedEvenWhenEveryReadFails()
    {
        var server = new Server().On(_ => (HttpStatusCode.BadGateway, "<html>502</html>"));
        var api = new ApiClient(
            new HttpClient(server), ServerUrl.Normalise("chat.example.com")!,
            new MemoryTokenStore("t0ken"));
        var chats = new ChatStore(database);
        var board = new BoardStore(database);
        var outbox = new OutboxStore(database);
        var posts = 0;
        var pipeline = new SendPipeline(
            new NeverConnected(), outbox, chats,
            post: (row, _) =>
            {
                posts++;
                return Task.FromResult(ApiResult<MessageResponse>.Failure(
                    ApiError.Transport("still down")));
            },
            wait: (_, _) => Task.CompletedTask);
        chats.Replace([new ChatRowDto(new ChatDto(42, "family", "The Smiths"))]);
        pipeline.Enqueue(42, "Dinner at 7?");

        var report = await new Resync(api, chats, board, pipeline).RunAsync();

        // The reads got nowhere…
        Assert.False(report.Complete);
        Assert.False(report.Signed);
        // …and the flush happened anyway, first.
        Assert.True(report.Flushed);
        Assert.Equal(1, posts);
        // The row is still queued, not failed: one transient attempt is not a refusal.
        Assert.False(Assert.Single(outbox.All()).Failed);
    }

    private sealed class NeverConnected : IFrameSender
    {
        public bool IsConnected => false;

        public Task<bool> TrySend(string frame, CancellationToken ct = default) =>
            Task.FromResult(false);

        public event Action<ServerFrame>? Frame;

        /// <summary>Never raised; declared so the interface is satisfied without a warning.</summary>
        public void Unused() => Frame?.Invoke(new ServerFrame.Pong());
    }

    /// <summary>
    /// A 200 is not a promise that the body is the documented body: a rewriting proxy or a moved
    /// path answers readable JSON with no `messages` key at all. That is an empty page — nothing
    /// to apply — and not a crash, which is what it was before every one of these lists was read
    /// with `?? []`.
    /// </summary>
    [Fact]
    public async Task AnAnswerWithNoListAtAllIsAnEmptyPageAndNotACrash()
    {
        var server = new Server()
            .Always("/me", Me)
            .Always("/families/mine/board", """{"max_board_seq": 0}""")
            .Always("/families/mine", Family)
            .Always("/chats", """{"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths"}}]}""")
            .Always("/chats/42/messages", "{}");
        var (resync, _, _, board) = Build(server);

        var report = await resync.RunAsync();
        Assert.True(report.Complete);
        Assert.Equal(0, report.Messages);
        Assert.Equal(0, report.Notes);
        Assert.Empty(board.Notes());
    }

    [Fact]
    public async Task AReadThatFailsStopsTheReadsAndSaysWhere()
    {
        var server = new Server()
            .Always("/me", Me)
            .Always("/families/mine", Family)
            .Always("/chats", """{"error": {"code": "internal", "message": "no"}}""",
                HttpStatusCode.InternalServerError);
        var (resync, _, _, _) = Build(server);

        var report = await resync.RunAsync();
        Assert.False(report.Complete);
        Assert.True(report.Signed);
        Assert.Equal(ErrorCodes.Internal, report.Stopped!.Code);
        // Transient, so the caller knows to come back rather than to show a failure.
        Assert.True(report.Stopped.Transient);
    }
}
