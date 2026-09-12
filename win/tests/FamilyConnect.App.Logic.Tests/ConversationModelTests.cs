using System.Net;
using FamilyConnect.App.Logic;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// One open chat: the window, paging back, the read marker, typing and sending.
/// </summary>
public class ConversationModelTests : IDisposable
{
    private const long Me = 7;
    private const long Chat = 42;

    private static readonly DateTimeOffset Start =
        new(2026, 9, 12, 12, 0, 0, TimeSpan.Zero);

    private readonly Database cache = Database.OpenInMemory();
    private readonly ChatStore chats;
    private readonly OutboxStore outbox;
    private DateTimeOffset now = Start;

    public ConversationModelTests()
    {
        chats = new ChatStore(cache, () => Me);
        outbox = new OutboxStore(cache);
        chats.Replace([new ChatRowDto(new ChatDto(Chat, "family", "The Smiths"))]);
    }

    public void Dispose() => cache.Dispose();

    /// <summary>A socket that is told whether it is up, and remembers what went.</summary>
    private sealed class Wire : IFrameSender
    {
        public bool IsConnected { get; set; } = true;

        public List<string> Sent { get; } = [];

        public Task<bool> TrySend(string frame, CancellationToken ct = default)
        {
            Sent.Add(frame);
            return Task.FromResult(IsConnected);
        }

        public event Action<ServerFrame>? Frame;

        public void Unused() => Frame?.Invoke(new ServerFrame.Pong());
    }

    private static string Wire_(long id, long sender = 9, string body = "Dinner at 7?") =>
        $$"""
        {"id": {{id}}, "chat_id": 42, "sender_id": {{sender}}, "client_msg_id": null,
         "body": "{{body}}", "created_at": "2026-09-12T10:00:00Z"}
        """;

    private static MessageDto Message(long id, long sender = 9, string body = "Dinner at 7?") =>
        new(id, Chat, sender, null, body, "2026-09-12T10:00:00Z");

    private sealed record Rig(
        ConversationModel Chat, Server Handler, Wire Socket, List<string> Posted);

    private Rig Build(Server server)
    {
        var api = new ApiClient(
            new HttpClient(server), ServerUrl.Normalise("chat.example.com")!,
            new MemoryTokenStore("t0ken"));
        var socket = new Wire();
        var posted = new List<string>();
        var sending = new SendPipeline(
            socket, outbox, chats,
            post: (row, _) =>
            {
                posted.Add(row.Body);
                return Task.FromResult(ApiResult<MessageResponse>.Success(
                    new MessageResponse(new MessageDto(
                        2000 + posted.Count, Chat, Me, row.ClientMsgId, row.Body,
                        "2026-09-12T12:00:00Z"))));
            },
            wait: (_, _) => Task.CompletedTask);
        var model = new ConversationModel(Chat, chats, api, sending, socket, () => now);
        return new Rig(model, server, socket, posted);
    }

    private static string PageOf(int count, long from) =>
        $$"""{"messages": [{{string.Join(",", Enumerable.Range(0, count).Select(at => Wire_(from + at)))}}]}""";

    [Fact]
    public async Task OpeningAChatWithNothingHeldReadsItsNewestPage()
    {
        var rig = Build(new Server().On("/chats/42/messages", PageOf(50, 1)));

        Assert.Null(await rig.Chat.OpenAsync());

        Assert.Equal(50, rig.Chat.Bubbles().Count);
        // Oldest first: a conversation reads the other way round from the index.
        Assert.Equal(1, rig.Chat.Bubbles()[0].Message.Id);
        Assert.Equal(50, rig.Chat.Bubbles()[^1].Message.Id);
        Assert.Contains("/api/v1/chats/42/messages?limit=50", rig.Handler.Asked);
        // A full page says there may be more behind it.
        Assert.True(rig.Chat.MayHaveOlder);
    }

    /// <summary>
    /// Nothing is fetched when the cache already holds this chat: the resync's own catch-up is
    /// what brings it up to date, and a read here would race it for no gain.
    /// </summary>
    [Fact]
    public async Task OpeningAChatThatIsAlreadyHeldAsksForNothing()
    {
        chats.Apply([Message(1), Message(2)]);
        var rig = Build(new Server());

        Assert.Null(await rig.Chat.OpenAsync());

        Assert.Empty(rig.Handler.Asked);
        Assert.Equal(2, rig.Chat.Bubbles().Count);
    }

    /// <summary>
    /// A short page IS the beginning of the chat. Saying otherwise leaves a button that asks the
    /// server for nothing for ever.
    /// </summary>
    [Fact]
    public async Task AShortPageIsTheBeginningOfTheChat()
    {
        var rig = Build(new Server().On("/chats/42/messages", PageOf(3, 1)));

        await rig.Chat.OpenAsync();

        Assert.Equal(3, rig.Chat.Bubbles().Count);
        Assert.False(rig.Chat.MayHaveOlder);
    }

    /// <summary>
    /// Paging back widens the WINDOW first: what the cache already holds below the oldest drawn
    /// message is paged in without a request at all, which is what makes reopening a chat cheap.
    /// </summary>
    [Fact]
    public async Task PagingBackReadsTheCacheBeforeItReadsTheServer()
    {
        chats.Apply([.. Enumerable.Range(1, 60).Select(id => Message(id))]);
        var rig = Build(new Server());
        await rig.Chat.OpenAsync();
        Assert.Equal(50, rig.Chat.Bubbles().Count);

        Assert.Null(await rig.Chat.OlderAsync());

        Assert.Equal(60, rig.Chat.Bubbles().Count);
        Assert.Empty(rig.Handler.Asked);
    }

    [Fact]
    public async Task PagingPastWhatIsHeldAsksForOlderThanTheOldest()
    {
        chats.Apply([.. Enumerable.Range(100, 50).Select(id => Message(id))]);
        var rig = Build(new Server().On("/chats/42/messages", PageOf(2, 98)));
        await rig.Chat.OpenAsync();

        Assert.Null(await rig.Chat.OlderAsync());

        Assert.Contains("/api/v1/chats/42/messages?before_id=100&limit=50", rig.Handler.Asked);
        Assert.Equal(52, rig.Chat.Bubbles().Count);
        // Two of fifty: there is nothing older.
        Assert.False(rig.Chat.MayHaveOlder);
    }

    /// <summary>
    /// THE READ MARKER GOES FORWARD ONLY, AND ONLY WHEN THERE IS SOMETHING TO REPORT: the server
    /// keeps the maximum ever reported, so an older id buys a request and nothing else, and a
    /// report per redraw costs the family's server a request per redraw.
    /// </summary>
    [Fact]
    public async Task ReadingReportsTheNewestOnceAndNotAgain()
    {
        chats.Apply(Message(1338), SeqRoute.LiveFrame);
        var rig = Build(new Server().On("/chats/42/read", null, HttpStatusCode.NoContent));
        Assert.Equal(1, chats.Chat(Chat)!.UnreadCount);

        Assert.True(await rig.Chat.ReadAsync());

        // The badge goes out at once, whatever the network is doing…
        Assert.Equal(0, chats.Chat(Chat)!.UnreadCount);
        Assert.Equal(1338, chats.Chat(Chat)!.LastReadMessageId);
        Assert.Single(rig.Handler.Asked);

        // …and looking again reports nothing, because nothing has been said since.
        Assert.False(await rig.Chat.ReadAsync());
        Assert.Single(rig.Handler.Asked);

        // A new message, and there is something to report again.
        chats.Apply(Message(1339), SeqRoute.LiveFrame);
        Assert.True(await rig.Chat.ReadAsync());
        Assert.Equal(2, rig.Handler.Asked.Count);
    }

    [Fact]
    public async Task AReadThatTheNetworkRefusesStillTakesTheBadgeOut()
    {
        chats.Apply(Message(1338), SeqRoute.LiveFrame);
        var rig = Build(new Server()
            .On("/chats/42/read", "<html>502</html>", HttpStatusCode.BadGateway));

        Assert.True(await rig.Chat.ReadAsync());

        // Locally read; the server keeps the maximum ever reported, so the next report fixes it.
        Assert.Equal(0, chats.Chat(Chat)!.UnreadCount);
        Assert.Equal(1338, chats.Chat(Chat)!.LastReadMessageId);
    }

    /// <summary>
    /// TYPING IS MOMENTARY: one frame every four seconds at most, and never while the socket is
    /// down — by the time a queued one went it would be a lie.
    /// </summary>
    [Fact]
    public async Task TypingIsThrottledAndNeverQueued()
    {
        var rig = Build(new Server());

        Assert.True(await rig.Chat.TypingAsync());
        Assert.Single(rig.Socket.Sent);
        Assert.Contains("\"type\":\"typing\"", rig.Socket.Sent[0]);

        // Every keystroke after it, for four seconds, says nothing.
        now = Start.AddSeconds(3);
        Assert.False(await rig.Chat.TypingAsync());
        Assert.Single(rig.Socket.Sent);

        now = Start.AddSeconds(4);
        Assert.True(await rig.Chat.TypingAsync());
        Assert.Equal(2, rig.Socket.Sent.Count);

        // And with the socket down, nothing at all: not queued, not tried.
        rig.Socket.IsConnected = false;
        now = Start.AddMinutes(1);
        Assert.False(await rig.Chat.TypingAsync());
        Assert.Equal(2, rig.Socket.Sent.Count);
    }

    [Fact]
    public void SendingWritesTheRowDownFirstAndTheOutboxOwnsIt()
    {
        var rig = Build(new Server());
        rig.Socket.IsConnected = false;

        var row = rig.Chat.Send("Dinner at 7?");

        // Written down before anything was tried: an interrupted send is a bubble that can be
        // finished rather than nothing at all.
        Assert.Equal("Dinner at 7?", Assert.Single(outbox.All()).Body);
        Assert.Equal(Chat, row.ChatId);
        // And the dedup key is a real UUID, which is what makes a repeat safe.
        Assert.True(Guid.TryParse(row.ClientMsgId, out _));

        rig.Chat.Send("…or 8?");
        Assert.Equal(2, outbox.All().Count);
    }

    /// <summary>
    /// A REVEAL BELONGS TO THE CONVERSATION: held here, it outlives the bubble being drawn again,
    /// which is the difference between tapping "show" once and tapping it after every arriving
    /// message.
    /// </summary>
    [Fact]
    public void AHiddenBubbleCanBeRevealedAndStaysRevealed()
    {
        chats.Apply([Message(1338, sender: 11, body: "something unpleasant"), Message(1339)]);
        chats.SetBlocked(11, true);
        var rig = Build(new Server());

        var blocked = rig.Chat.Bubbles()[0];
        Assert.True(blocked.Hidden);
        Assert.False(blocked.Reads);
        Assert.False(rig.Chat.Bubbles()[1].Hidden);

        rig.Chat.Reveal(1338);
        Assert.True(rig.Chat.Bubbles()[0].Revealed);
        Assert.True(rig.Chat.Bubbles()[0].Reads);

        // Another message arrives and the bubbles are drawn again: the reveal is still there.
        chats.Apply(Message(1340), SeqRoute.LiveFrame);
        Assert.True(rig.Chat.Bubbles()[0].Revealed);

        rig.Chat.Hide(1338);
        Assert.False(rig.Chat.Bubbles()[0].Revealed);
    }

    [Fact]
    public void MyOwnBubblesAreMineAndNeverHidden()
    {
        chats.Apply([Message(1338, sender: Me, body: "mine"), Message(1339, sender: 11)]);
        chats.SetBlocked(11, true);
        var rig = Build(new Server());

        var bubbles = rig.Chat.Bubbles();

        Assert.True(bubbles[0].Mine);
        Assert.False(bubbles[0].Hidden);
        Assert.False(bubbles[1].Mine);
        Assert.True(bubbles[1].Hidden);
    }
}
