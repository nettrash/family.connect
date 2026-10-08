using FamilyConnect.App.Logic;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// What the WINDOW reads for a sticker and a circle — <see cref="ConversationModel.Bubbles"/>, the record
/// <c>ChatsView.BubbleElement</c> branches on — when the cache was written by a build that predates the flags: the bug
/// the owner saw on Windows (a sticker in a grey tile, a circle square, while every other device drew them right), and
/// schema step 8's repair of it.
/// </summary>
public sealed class CachedFlagsTests : IDisposable
{
    private const long Chat = 42;

    private readonly Database cache = Database.OpenInMemory();
    private readonly ChatStore chats;
    private readonly OutboxStore outbox;

    public CachedFlagsTests()
    {
        chats = new ChatStore(cache, () => 7);
        outbox = new OutboxStore(cache);
        chats.Replace([new ChatRowDto(new ChatDto(Chat, "family", "The Smiths"))]);
    }

    public void Dispose() => cache.Dispose();

    private const string StickerJson =
        """
        {"id": 1340, "chat_id": 42, "sender_id": 9, "client_msg_id": "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01", "body": "",
         "created_at": "2026-10-05T10:00:00Z",
         "attachment": {"id": 90, "kind": "photo", "mime": "image/webp", "size": 40960, "width": 512, "height": 512, "has_preview": false, "sticker": true},
         "attachments": [{"id": 90, "kind": "photo", "mime": "image/webp", "size": 40960, "width": 512, "height": 512, "has_preview": false, "sticker": true}]}
        """;

    private const string RoundJson =
        """
        {"id": 1341, "chat_id": 42, "sender_id": 9, "client_msg_id": "4f9e21c0-ceea-4e17-a91c-0d9f8e7b2a01", "body": "",
         "created_at": "2026-10-05T10:01:00Z",
         "attachment": {"id": 91, "kind": "video", "mime": "video/mp4", "size": 812345, "width": 480, "height": 480, "duration_ms": 23400, "has_preview": true, "round": true},
         "attachments": [{"id": 91, "kind": "video", "mime": "video/mp4", "size": 812345, "width": 480, "height": 480, "duration_ms": 23400, "has_preview": true, "round": true}]}
        """;

    /// <summary>The two rows exactly as a build before #58/#79 left them: in sequence, the set without either flag.</summary>
    private void CachedByAnOlderBuild()
    {
        foreach (var (id, attachments) in new[]
                 {
                     (1340L, """[{"id":90,"kind":"photo","mime":"image/webp","size":40960,"width":512,"height":512,"has_preview":false}]"""),
                     (1341L, """[{"id":91,"kind":"video","mime":"video/mp4","size":812345,"width":480,"height":480,"duration_ms":23400,"has_preview":true}]"""),
                 })
        {
            using var command = cache.Connection.CreateCommand();
            command.CommandText =
                """
                INSERT INTO messages (message_id, chat_id, sender_id, body, created_at, attachments_json, sequenced)
                VALUES ($id, 42, 9, '', 0, $attachments, 1)
                """;
            command.Parameters.AddWithValue("$id", id);
            command.Parameters.AddWithValue("$attachments", attachments);
            command.ExecuteNonQuery();
        }
    }

    private static ApiClient Api(Server server) =>
        new(new HttpClient(server), ServerUrl.Normalise("chat.example.com")!, new MemoryTokenStore("t0ken"));

    private ConversationModel Open(Server server) =>
        new(Chat, chats, Api(server),
            new SendPipeline(new NoSocket(), outbox, chats, post: (_, _) => throw new InvalidOperationException("no sends here"),
                wait: (_, _) => Task.CompletedTask),
            new NoSocket(), () => DateTimeOffset.UtcNow);

    private sealed class NoSocket : IFrameSender
    {
        public bool IsConnected => true;

        public Task<bool> TrySend(string frame, CancellationToken ct = default) => Task.FromResult(true);

#pragma warning disable CS0067 // Never raised: nothing arrives on this socket.
        public event Action<ServerFrame>? Frame;
#pragma warning restore CS0067
    }

    /// <summary>
    /// A FRESH CACHE DRAWS THEM RIGHT: the newest page, as the server sends it, reaches the bubbles with both flags — the
    /// sticker branch and the circle branch, not the photo tile.
    /// </summary>
    [Fact]
    public async Task AFreshCacheHandsTheWindowAStickerAndACircle()
    {
        var conversation = Open(new Server().On("/chats/42/messages", $$"""{"messages": [{{StickerJson}}, {{RoundJson}}]}"""));
        Assert.Null(await conversation.OpenAsync());
        var bubbles = conversation.Bubbles();
        Assert.Equal(90, bubbles[0].Message.StickerPicture!.Id);
        Assert.Equal(91, bubbles[1].Message.RoundVideo!.Id);
        Assert.Equal("s90", BubbleRules.MediaMark(bubbles[0].Message));
        Assert.Equal("r91", BubbleRules.MediaMark(bubbles[1].Message));
    }

    /// <summary>
    /// THE OWNER'S SCREEN, AND ITS REPAIR. Held in sequence, the chat is not read on open — so the window is handed a
    /// photo and a plain video, and draws a tile and a square. One resync pass reads each once and the window is handed a
    /// sticker and a circle; the redraw key changes with them, so the conversation is drawn again.
    /// </summary>
    [Fact]
    public async Task ACacheAnOlderBuildWroteIsDrawnAsPhotoAndSquareUntilAPassRepairsIt()
    {
        CachedByAnOlderBuild();
        var server = new Server()
            .On("/me", """{"user": {"id": 7, "username": "anna", "display_name": "Anna"}, "blocked_user_ids": []}""")
            .On("/chats", """{"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths"}}]}""")
            .Then("/chats/42/messages",
                (System.Net.HttpStatusCode.OK, """{"messages": []}"""), // the catch-up: after_id=1341, nothing newer
                (System.Net.HttpStatusCode.OK, $$"""{"messages": [{{RoundJson}}]}"""), // before_id=1342&limit=1
                (System.Net.HttpStatusCode.OK, $$"""{"messages": [{{StickerJson}}]}""")); // before_id=1341&limit=1
        var conversation = Open(server);

        Assert.Null(await conversation.OpenAsync());
        Assert.Empty(server.Asked);
        var before = conversation.Bubbles();
        Assert.Null(before[0].Message.StickerPicture);
        Assert.Null(before[1].Message.RoundVideo);
        var marks = before.Select(bubble => BubbleRules.MediaMark(bubble.Message)).ToList();
        Assert.Equal(["m90photo-", "m91video+"], marks);

        var report = await new Resync(Api(server), chats, new BoardStore(cache)).RunAsync();
        Assert.True(report.Complete);
        Assert.Equal(2, report.FlagsRepaired);
        Assert.Equal(
            [
                "/api/v1/me", "/api/v1/chats", "/api/v1/chats/42/messages?after_id=1341&limit=50",
                "/api/v1/chats/42/messages?before_id=1342&limit=1", "/api/v1/chats/42/messages?before_id=1341&limit=1",
            ],
            server.Asked);

        var after = conversation.Bubbles();
        Assert.Equal(90, after[0].Message.StickerPicture!.Id);
        Assert.Equal(91, after[1].Message.RoundVideo!.Id);
        Assert.Equal(["s90", "r91"], after.Select(bubble => BubbleRules.MediaMark(bubble.Message)));
    }
}
