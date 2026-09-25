using System.Net;
using FamilyConnect.App.Logic;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>A chain on its own surface: rooted, read whole, kept out of the cache, and counted once per copy.</summary>
public sealed class ThreadModelTests : IDisposable
{
    private const long Me = 7;
    private const long Chat = 42;

    private readonly Database cache = Database.OpenInMemory();
    private readonly ChatStore chats;
    private readonly OutboxStore outbox;

    public ThreadModelTests()
    {
        chats = new ChatStore(cache, () => Me);
        outbox = new OutboxStore(cache);
        chats.Replace([new ChatRowDto(new ChatDto(Chat, "family", "The Smiths"))]);
    }

    public void Dispose() => cache.Dispose();

    private static MessageDto Message(long id, long? root = null, int? replies = null, long sender = 9, string body = "x", long? editSeq = null) =>
        new(id, Chat, sender, null, body, "2026-09-12T10:00:00Z", ThreadRootId: root, ReplyCount: replies, EditSeq: editSeq);

    private static string Json(MessageDto message) =>
        $"{{\"id\": {message.Id}, \"chat_id\": {message.ChatId}, \"sender_id\": {message.SenderId}, \"client_msg_id\": null, " +
        $"\"body\": \"{message.Body}\", \"created_at\": \"2026-09-12T10:00:00Z\"" +
        (message.ThreadRootId is { } root ? $", \"thread_root_id\": {root}" : string.Empty) +
        (message.ReplyCount is { } count ? $", \"reply_count\": {count}" : string.Empty) +
        (message.EditSeq is { } seq ? $", \"edit_seq\": {seq}, \"edited_at\": \"2026-09-12T11:00:00Z\"" : string.Empty) +
        "}";

    private static (HttpStatusCode, string?) Page(params MessageDto[] messages) =>
        (HttpStatusCode.OK, "{\"messages\": [" + string.Join(", ", messages.Select(Json)) + "]}");

    private ThreadModel Build(Server server, long named) =>
        new(Chat, named, chats,
            new ApiClient(new HttpClient(server), ServerUrl.Normalise("chat.example.com")!, new MemoryTokenStore("t0ken")),
            outbox);

    private static long[] Ids(IEnumerable<Bubble> bubbles) => [.. bubbles.Select(bubble => bubble.Message.Id)];

    /// <summary>Opened from a held reply, the chain is rooted before anything is asked, and draws what is held.</summary>
    [Fact]
    public void AChainOpensAtOnceOnWhatIsHeld()
    {
        chats.Apply([Message(10, replies: 1), Message(11, root: 10), Message(12)], SeqRoute.Evidence);

        var thread = Build(new Server(), named: 11);

        Assert.Equal(10, thread.RootId);
        Assert.Equal([10L, 11L], Ids(thread.Bubbles()));
        Assert.Equal(1, thread.Replies(thread.Bubbles()));
        Assert.Equal(12, Build(new Server(), named: 12).RootId);
    }

    /// <summary>Named by a reply the device does not hold, the first page answers with the root first — and re-roots.</summary>
    [Fact]
    public async Task TheFirstPageRootsTheChain()
    {
        var server = new Server().Then("/chats/42/messages/11/thread", Page(Message(10, replies: 1), Message(11, root: 10)));
        var thread = Build(server, named: 11);
        Assert.Equal(11, thread.RootId);

        Assert.Null(await thread.LoadAsync());

        Assert.True(thread.Loaded);
        Assert.Equal(10, thread.RootId);
        Assert.Equal([10L, 11L], Ids(thread.Bubbles()));
        Assert.Equal("/api/v1/chats/42/messages/11/thread?limit=50", Assert.Single(server.Asked));
    }

    /// <summary>A full page asks again after the newest row it held, and a short one ends the read.</summary>
    [Fact]
    public async Task AChainIsReadPageByPageUntilAShortOne()
    {
        var first = new[] { Message(10, replies: 50) }.Concat(Enumerable.Range(11, 49).Select(id => Message(id, root: 10))).ToArray();
        var server = new Server().Then("/chats/42/messages/10/thread", Page(first), Page(Message(60, root: 10)));
        var thread = Build(server, named: 10);

        await thread.LoadAsync();

        Assert.Equal(["/api/v1/chats/42/messages/10/thread?limit=50", "/api/v1/chats/42/messages/10/thread?limit=50&after_id=59"], server.Asked);
        Assert.Equal(51, thread.Bubbles().Count);
        Assert.Equal(10, thread.RootId);
    }

    /// <summary>
    /// The read stays OUT of the cache: a row it does not hold stays out, so no page starts from the wrong place — while a
    /// row it holds takes the true count and the edit.
    /// </summary>
    [Fact]
    public async Task TheReadRefreshesHeldRowsAndAddsNothingToTheCache()
    {
        chats.Apply([Message(10, replies: 1), Message(50)], SeqRoute.Evidence);
        var server = new Server().Then("/chats/42/messages/10/thread",
            Page(Message(10, replies: 3, body: "edited", editSeq: 2), Message(11, root: 10), Message(300, root: 10)));
        var thread = Build(server, named: 10);

        await thread.LoadAsync();

        Assert.Equal((3, "edited"), (chats.Message(10)!.ReplyCount, chats.Message(10)!.Body));
        Assert.Null(chats.Message(11));
        Assert.Null(chats.Message(300));
        Assert.Equal([50L, 10L], chats.Messages(Chat).Select(message => message.Id));
        Assert.Equal([10L, 11L, 300L], Ids(thread.Bubbles()));
    }

    /// <summary>A reply that reaches the cache after the read joins the surface and raises the surface's root — once.</summary>
    [Fact]
    public async Task ALaterReplyRaisesTheSurfacesRootOnce()
    {
        var server = new Server().Then("/chats/42/messages/10/thread", Page(Message(10, replies: 1), Message(11, root: 10)));
        var thread = Build(server, named: 10);
        await thread.LoadAsync();

        chats.Apply(Message(12, root: 10), SeqRoute.LiveFrame);

        Assert.Equal([10L, 11L, 12L], Ids(thread.Bubbles()));
        Assert.Equal(2, thread.Bubbles()[0].Message.ReplyCount);
        Assert.False(thread.Heard(Message(13, root: 99)));
        Assert.True(thread.Heard(Message(12, root: 10)));
        Assert.Equal(2, thread.Bubbles()[0].Message.ReplyCount);
    }

    /// <summary>A root the cache holds is counted by the cache; the surface draws that copy, and does not count it again.</summary>
    [Fact]
    public async Task AHeldRootIsNotCountedTwice()
    {
        chats.Apply([Message(10, replies: 1), Message(11, root: 10)], SeqRoute.Evidence);
        var server = new Server().Then("/chats/42/messages/10/thread", Page(Message(10, replies: 1), Message(11, root: 10)));
        var thread = Build(server, named: 10);
        await thread.LoadAsync();

        chats.Apply(Message(12, root: 10), SeqRoute.LiveFrame);

        Assert.Equal(2, chats.Message(10)!.ReplyCount);
        Assert.Equal(2, thread.Bubbles()[0].Message.ReplyCount);
    }

    /// <summary>An older copy never undoes an edit, but brings the true count.</summary>
    [Fact]
    public async Task AnOlderCopyKeepsTheEditAndTakesTheCount()
    {
        var server = new Server().Then("/chats/42/messages/10/thread", Page(Message(10, replies: 1, body: "new", editSeq: 4)));
        var thread = Build(server, named: 10);
        await thread.LoadAsync();

        Assert.True(thread.Heard(Message(10, replies: 5, body: "old", editSeq: 3)));

        var root = Assert.Single(thread.Bubbles()).Message;
        Assert.Equal(("new", 4L, 5), (root.Body, root.EditSeq, root.ReplyCount));
    }

    /// <summary>A read that fails says why and keeps what was drawn.</summary>
    [Fact]
    public async Task AFailedReadKeepsWhatIsHeld()
    {
        chats.Apply([Message(10, replies: 1), Message(11, root: 10)], SeqRoute.Evidence);
        var thread = Build(new Server().Then("/chats/42/messages/10/thread",
            // A refusal, not a 502: a read is safe to repeat, and the client would try a 502 again by itself.
            (HttpStatusCode.Forbidden, """{"error": {"code": "not_in_family", "message": "no"}}"""),
            Page(Message(10, replies: 1), Message(11, root: 10))), named: 10);

        Assert.NotNull(await thread.LoadAsync());

        Assert.False(thread.Loaded);
        Assert.NotNull(thread.Failure);
        Assert.Equal([10L, 11L], Ids(thread.Bubbles()));

        // The next read that answers clears it.
        Assert.Null(await thread.LoadAsync());
        Assert.Null(thread.Failure);
        Assert.True(thread.Loaded);
    }

    /// <summary>This reader's replies on their way belong to the chain when they answer the root or any row of it.</summary>
    [Fact]
    public async Task PendingRepliesToTheChainAreItsOwn()
    {
        var server = new Server().Then("/chats/42/messages/10/thread", Page(Message(10, replies: 1), Message(11, root: 10)));
        var thread = Build(server, named: 10);
        await thread.LoadAsync();
        outbox.Queue(new OutboxRow("a", Chat, "8?", ReplyToMessageId: 11));
        outbox.Queue(new OutboxRow("b", Chat, "sure", ReplyToMessageId: 10));
        outbox.Queue(new OutboxRow("c", Chat, "elsewhere", ReplyToMessageId: 99));
        outbox.Queue(new OutboxRow("d", Chat, "no reply"));

        Assert.Equal(["a", "b"], thread.Pending().Select(row => row.ClientMsgId).Order());
    }

    /// <summary>A blocked member's row is the hidden row here too, and revealing it is this surface's.</summary>
    [Fact]
    public void ABlockedMembersRowIsHiddenOnTheSurface()
    {
        chats.Apply([Message(10, replies: 1), Message(11, root: 10, sender: 5)], SeqRoute.Evidence);
        chats.ReplaceBlocked([5]);
        var thread = Build(new Server(), named: 10);

        Assert.True(thread.Bubbles()[1].Hidden);
        Assert.False(thread.Bubbles()[1].Revealed);
        thread.Reveal(11);
        Assert.True(thread.Bubbles()[1].Revealed);
        thread.Hide(11);
        Assert.False(thread.Bubbles()[1].Revealed);
    }

    /// <summary>The chain is this chat's: a row rooted at the same id in another chat is neither held here nor heard.</summary>
    [Fact]
    public void AChainIsItsOwnChats()
    {
        chats.Replace([new ChatRowDto(new ChatDto(Chat, "family", "The Smiths")), new ChatRowDto(new ChatDto(43, "direct", "Bob"))]);
        chats.Apply([Message(10, replies: 1), Message(11, root: 10), Message(20, root: 10) with { ChatId = 43 }], SeqRoute.Evidence);
        var thread = Build(new Server(), named: 10);

        var bubbles = thread.Bubbles();

        Assert.Equal([10L, 11L], Ids(bubbles));
        Assert.Equal(1, bubbles[0].Message.ReplyCount);
        Assert.False(thread.Heard(Message(21, root: 10) with { ChatId = 43 }));
    }

    /// <summary>A row the cache holds is drawn from the fresher copy: a later edit and a later poll state reach the surface.</summary>
    [Fact]
    public async Task AHeldRowDrawsTheCachesFresherState()
    {
        var poll = new PollDto(1, false, [new PollOptionDto(1, "Pizza", []), new PollOptionDto(2, "Pasta", [])]);
        chats.Apply([Message(10, replies: 1) with { Poll = poll }, Message(11, root: 10, body: "a", editSeq: 1)], SeqRoute.Evidence);
        var server = new Server().Then("/chats/42/messages/10/thread", Page(Message(10, replies: 1), Message(11, root: 10, body: "a", editSeq: 1)));
        var thread = Build(server, named: 10);
        await thread.LoadAsync();

        chats.Apply(Message(11, root: 10, body: "b", editSeq: 2), SeqRoute.Evidence);
        chats.ApplyPoll(Chat, 10, poll with { PollSeq = 2, Closed = true });

        var bubbles = thread.Bubbles();
        Assert.Equal("b", bubbles[1].Message.Body);
        Assert.True(bubbles[0].Message.Poll!.Closed);
    }

    /// <summary>Reactions come from the copy with the newer reaction sequence, whichever copy that is.</summary>
    [Fact]
    public void ReactionsComeFromTheNewerSequence()
    {
        var thread = Build(new Server(), named: 10);

        Assert.True(thread.Heard(Message(10) with { ReactionSeq = 5, Reactions = [new ReactionDto(9, "👍")] }));
        Assert.True(thread.Heard(Message(10) with { ReactionSeq = 3, Reactions = [new ReactionDto(9, "❤️")] }));
        Assert.Equal("👍", Assert.Single(Assert.Single(thread.Bubbles()).Message.Reactions!).Emoji);

        Assert.True(thread.Heard(Message(10) with { ReactionSeq = 6, Reactions = [new ReactionDto(9, "😂")] }));
        Assert.Equal("😂", Assert.Single(thread.Bubbles()[0].Message.Reactions!).Emoji);
        // The root itself is no reply to itself.
        Assert.Null(thread.Bubbles()[0].Message.ReplyCount);
    }

    /// <summary>
    /// A reaction on a row the cache does not hold is decided against the surface's copy, and its answer lands there —
    /// unless the surface already holds a newer reaction sequence.
    /// </summary>
    [Fact]
    public async Task AReactionOnARowTheCacheDoesNotHoldLandsOnTheSurface()
    {
        var server = new Server()
            .Then("/chats/42/messages/10/thread", Page(Message(10, replies: 3), Message(11, root: 10), Message(12, root: 10), Message(13, root: 10)))
            .On("/chats/42/messages/11/reaction", """{"message_id": 11, "reaction_seq": 3, "reactions": [{"user_id": 7, "emoji": "👍"}]}""")
            .On("/chats/42/messages/12/reaction", """{"message_id": 12, "reaction_seq": 3, "reactions": [{"user_id": 7, "emoji": "👍"}]}""")
            .On("/chats/42/messages/13/reaction", """{"message_id": 13, "reaction_seq": 5, "reactions": [{"user_id": 7, "emoji": "👍"}]}""");
        var thread = Build(server, named: 10);
        await thread.LoadAsync();
        thread.Heard(Message(12, root: 10) with { ReactionSeq = 5, Reactions = [new ReactionDto(9, "❤️")] });
        thread.Heard(Message(13, root: 10) with { ReactionSeq = 5, Reactions = [new ReactionDto(9, "❤️")] });

        Assert.Null(await thread.ReactAsync(11, "👍"));
        Assert.Null(await thread.ReactAsync(12, "👍"));
        Assert.Null(await thread.ReactAsync(13, "👍"));
        Assert.Null(await thread.ReactAsync(99, "👍"));

        var bubbles = thread.Bubbles();
        Assert.Equal(("👍", 3L), (Assert.Single(bubbles[1].Message.Reactions!).Emoji, bubbles[1].Message.ReactionSeq!.Value));
        Assert.Equal(("❤️", 5L), (Assert.Single(bubbles[2].Message.Reactions!).Emoji, bubbles[2].Message.ReactionSeq!.Value));
        Assert.Equal(("👍", 5L), (Assert.Single(bubbles[3].Message.Reactions!).Emoji, bubbles[3].Message.ReactionSeq!.Value));
        Assert.Null(chats.Message(11));
        Assert.Equal(4, server.Asked.Count);
    }

    /// <summary>The root arriving is no reply to itself: it raises nothing, and the first reply after it raises one.</summary>
    [Fact]
    public void TheRootArrivingRaisesNothing()
    {
        var thread = Build(new Server(), named: 10);

        Assert.True(thread.Heard(Message(10)));
        Assert.Null(Assert.Single(thread.Bubbles()).Message.ReplyCount);

        Assert.True(thread.Heard(Message(11, root: 10)));
        Assert.Equal(1, thread.Bubbles()[0].Message.ReplyCount);
    }

    /// <summary>Before anything is read or held, a reply to the root is still the chain's.</summary>
    [Fact]
    public void AReplyToTheRootIsTheChainsBeforeAnythingIsRead()
    {
        var thread = Build(new Server(), named: 10);
        outbox.Queue(new OutboxRow("a", Chat, "first!", ReplyToMessageId: 10));

        Assert.Equal("a", Assert.Single(thread.Pending()).ClientMsgId);
    }

    /// <summary>A full page that cannot move the cursor ends the read, rather than asking the same thing for ever.</summary>
    [Fact]
    public async Task AFullPageThatCannotMoveTheCursorEndsTheRead()
    {
        var full = Enumerable.Range(10, 50).Select(id => Message(id, root: id == 10 ? null : 10)).ToArray();
        var server = new Server().Then("/chats/42/messages/10/thread", Page(full));
        var thread = Build(server, named: 10);

        Assert.Null(await thread.LoadAsync());

        Assert.Equal(2, server.Asked.Count);
        Assert.True(thread.Loaded);
    }
}
