using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.Core.Tests.Store;

/// <summary>
/// The chats and what was said in them: the guards that decide whether an answer may overwrite
/// what this device holds (docs/protocol.md, "Editing", "Reactions", "Threads").
/// </summary>
public class ChatStoreTests : IDisposable
{
    private readonly Database database = Database.OpenInMemory();

    public void Dispose() => database.Dispose();

    private ChatStore Store() => new(database);

    private static ChatDto Family => new(42, "family", "The Smiths");

    private static ChatRowDto Row(
        ChatDto? chat = null,
        MessageDto? last = null,
        int unread = 0,
        long read = 0,
        long? reactionSeq = null,
        long? editSeq = null,
        long? pollSeq = null,
        bool? mentioned = null) =>
        new(chat ?? Family, last, unread, read, reactionSeq, editSeq, pollSeq, mentioned);

    private static MessageDto Message(
        long id,
        long chat = 42,
        string body = "Dinner at 7?",
        long sender = 7,
        string created = "2026-09-12T10:00:00Z",
        long? editSeq = null,
        MentionDto[]? mentions = null,
        long? threadRoot = null) =>
        new(
            Id: id,
            ChatId: chat,
            SenderId: sender,
            ClientMsgId: null,
            Body: body,
            CreatedAt: created,
            EditSeq: editSeq,
            EditedAt: editSeq is null ? null : "2026-09-12T10:05:00Z",
            Mentions: mentions,
            ThreadRootId: threadRoot);

    [Fact]
    public void TheListIsWhatTheServerAnswered()
    {
        var store = Store();
        store.Replace([
            Row(),
            Row(new ChatDto(43, "direct", "Anna", PeerUserId: 9)),
        ]);
        Assert.Equal([42L, 43L], store.Chats().Select(row => row.Chat.Id).Order());

        // A direct chat that is no longer listed is one the reader has BLOCKED. It leaves the
        // LIST and nothing else: the messages stay, the marker stays, and an unblock brings the
        // whole conversation back rather than an empty chat.
        store.Apply(Message(1, chat: 43, body: "hello"));
        store.MarkRead(43, 1);
        store.Replace([Row()]);
        Assert.Single(store.Chats());
        Assert.False(store.IsListed(43));
        Assert.Equal("hello", store.Message(1)!.Body);
        Assert.Equal(1, store.Chat(43)!.LastReadMessageId);
        // And it counts towards nothing while it is hidden.
        store.Apply(Message(2, chat: 43, body: "and another"));
        Assert.Equal(0, store.Unread());

        // Unblocked: the list carries it again, whole.
        store.Replace([Row(), Row(new ChatDto(43, "direct", "Anna", PeerUserId: 9))]);
        Assert.True(store.IsListed(43));
        Assert.Equal(2, store.Chats().Count);
        Assert.Equal("and another", store.Chat(43)!.LastMessage!.Body);
    }

    [Fact]
    public void AMessageGoesInAndComesBackWhole()
    {
        var store = Store();
        store.Replace([Row()]);
        store.Apply(Message(1338) with
        {
            Attachments = [new AttachmentDto(34, "photo", "image/jpeg", Width: 600, Height: 1200)],
            Attachment = new AttachmentDto(34, "photo", "image/jpeg"),
            Mentions = [new MentionDto(9, "Anna")],
            Reactions = [new ReactionDto(9, "❤️")],
            ReactionSeq = 123,
        });
        var held = store.Message(1338)!;
        Assert.Equal("Dinner at 7?", held.Body);
        Assert.Equal("2026-09-12T10:00:00.000Z", held.CreatedAt);
        Assert.Equal(34, Assert.Single(held.Media).Id);
        Assert.Equal("Anna", Assert.Single(held.Mentions!).Name);
        Assert.Equal(123, held.ReactionSeq);
        // A message with no poll and no call has neither field, not empty ones.
        Assert.Null(held.Poll);
        Assert.Null(held.Call);
    }

    /// <summary>
    /// APPLYING AN EDIT IS GUARDED. A history page fetched BEFORE an edit but delivered after it
    /// would otherwise restore the old text, and two devices in a family disagree about what was
    /// said.
    /// </summary>
    [Fact]
    public void AnOlderCopyNeverRestoresTheOldText()
    {
        var store = Store();
        store.Replace([Row()]);
        store.Apply(Message(1338, body: "Dinner at 7?"));
        Assert.True(store.Apply(Message(1338, body: "Dinner at 8?", editSeq: 88)));
        Assert.Equal("Dinner at 8?", store.Message(1338)!.Body);

        // The page that was already in flight, carrying the pre-edit body and no seq.
        Assert.False(store.Apply(Message(1338, body: "Dinner at 7?")));
        Assert.Equal("Dinner at 8?", store.Message(1338)!.Body);
        // And an edit at the SAME seq still applies: the guard is "at least", because the same
        // edit may arrive by two paths and the whole message comes with it.
        Assert.True(store.Apply(Message(1338, body: "Dinner at 8?", editSeq: 88)));
    }

    /// <summary>
    /// The one edit that changes more than the body: the assistant's reply gains its picture as
    /// an attachment through exactly this path, so the WHOLE message is applied.
    /// </summary>
    [Fact]
    public void AnEditCanAddAnAttachmentBecauseTheWholeMessageIsApplied()
    {
        var store = Store();
        store.Replace([Row()]);
        store.Apply(Message(1339, sender: 1, body: "Here it is"));
        Assert.Empty(store.Message(1339)!.Media);

        store.Apply(Message(1339, sender: 1, body: "Here it is", editSeq: 90) with
        {
            Attachments = [new AttachmentDto(900, "photo", "image/png")],
        });
        Assert.Equal(900, Assert.Single(store.Message(1339)!.Media).Id);
    }

    /// <summary>
    /// A PREVIEW carries less than a page does — no reactions, no poll, no quote — and absent
    /// means "not included here", never "cleared".
    /// </summary>
    [Fact]
    public void APreviewDoesNotStripWhatAFullerCopyHad()
    {
        var store = Store();
        store.Replace([Row()]);
        store.Apply(Message(1340) with
        {
            Reactions = [new ReactionDto(9, "❤️")],
            ReactionSeq = 5,
            Poll = new PollDto(7, false, [new PollOptionDto(1, "Pizza", [9])]),
        });
        // The list read arrives with the trimmed preview of the same message.
        store.Replace([Row(last: Message(1340), unread: 1)]);
        var held = store.Message(1340)!;
        Assert.Equal("❤️", Assert.Single(held.Reactions!).Emoji);
        Assert.Equal(5, held.ReactionSeq);
        Assert.NotNull(held.Poll);

        // AND THE CASE THAT ACTUALLY WRITES: a preview carrying a NEWER edit seq — an edit
        // happened, and the list read brought the trimmed copy before a page did. The body is
        // taken, and the reactions and the poll it does not carry are NOT cleared by it.
        store.Replace([Row(last: Message(1340, body: "Dinner at 8?", editSeq: 91), unread: 1)]);
        var edited = store.Message(1340)!;
        Assert.Equal("Dinner at 8?", edited.Body);
        Assert.Equal(91, edited.EditSeq);
        Assert.Equal("❤️", Assert.Single(edited.Reactions!).Emoji);
        Assert.Equal(5, edited.ReactionSeq);
        Assert.NotNull(edited.Poll);
    }

    [Fact]
    public void ReactionsAreCompleteStateUnderTheirOwnSeq()
    {
        var store = Store();
        store.Replace([Row()]);
        store.Apply(Message(1341));
        Assert.True(store.ApplyReactions(1341, 10, [new ReactionDto(9, "❤️")]));
        Assert.Equal("❤️", Assert.Single(store.Message(1341)!.Reactions!).Emoji);
        // Cleared is `[]` and the field STAYS: a client tells "cleared" from "no data".
        Assert.True(store.ApplyReactions(1341, 11, []));
        Assert.NotNull(store.Message(1341)!.Reactions);
        Assert.Empty(store.Message(1341)!.Reactions!);
        // And an older frame that crossed a newer one is dropped.
        Assert.False(store.ApplyReactions(1341, 9, [new ReactionDto(9, "👍")]));
        Assert.Empty(store.Message(1341)!.Reactions!);
        // The chat's cursor followed the newest.
        Assert.Equal(11, store.Chat(42)!.MaxReactionSeq);
    }

    [Fact]
    public void APollIsCompleteStateTooAndTheCursorFollowsIt()
    {
        var store = Store();
        store.Replace([Row()]);
        store.Apply(Message(1342, body: "Pizza or pasta?"));
        Assert.True(store.ApplyPoll(1342, new PollDto(88, false, [
            new PollOptionDto(5, "Pizza", [7]),
            new PollOptionDto(6, "Pasta", []),
        ])));
        Assert.Equal([7L], store.Message(1342)!.Poll!.Options[0].Votes);
        Assert.False(store.ApplyPoll(1342, new PollDto(87, true, [])));
        Assert.False(store.Message(1342)!.Poll!.Closed);
        Assert.Equal(88, store.Chat(42)!.MaxPollSeq);
    }

    /// <summary>
    /// ONLY A LIVE FRAME AND A CATCH-UP PAGE MAY MOVE A CHAT CURSOR. A state that arrives as
    /// EVIDENCE — embedded on a fetched message, or in the answer to this client's own reaction —
    /// is applied to the message and moves nothing: advancing the cursor from evidence would step
    /// the chat past states for messages this device has never seen, and `after_seq` can never
    /// look back.
    /// </summary>
    [Fact]
    public void EvidenceAppliesToTheMessageAndMovesNoCursor()
    {
        var store = Store();
        store.Replace([Row()]);
        store.Apply(Message(1));
        store.Apply(Message(2, body: "Pizza or pasta?"));

        // The answer to our own reaction: applied, cursor untouched.
        Assert.True(store.ApplyReactions(1, 300, [new ReactionDto(7, "❤️")], SeqRoute.Evidence));
        Assert.Equal("❤️", Assert.Single(store.Message(1)!.Reactions!).Emoji);
        Assert.Null(store.Chat(42)!.MaxReactionSeq);

        // The answer to our own vote: the same.
        Assert.True(store.ApplyPoll(
            2, new PollDto(400, false, [new PollOptionDto(5, "Pizza", [7])]), SeqRoute.Evidence));
        Assert.Equal([7L], store.Message(2)!.Poll!.Options[0].Votes);
        Assert.Null(store.Chat(42)!.MaxPollSeq);

        // A live frame and a catch-up page both move it.
        store.ApplyReactions(1, 301, [], SeqRoute.LiveFrame);
        Assert.Equal(301, store.Chat(42)!.MaxReactionSeq);
        store.ApplyPoll(2, new PollDto(401, true, []), SeqRoute.CatchUpPage);
        Assert.Equal(401, store.Chat(42)!.MaxPollSeq);
    }

    /// <summary>
    /// The three cursors are HIGH-WATER MARKS: absent on the wire is 0 here, and a 0 never lowers
    /// one — a chat whose polls retention has swept still reports its maximum.
    /// </summary>
    [Fact]
    public void TheCatchUpCursorsNeverGoBackDown()
    {
        var store = Store();
        store.Replace([Row(reactionSeq: 124, editSeq: 88, pollSeq: 89)]);
        store.Replace([Row()]);
        var row = store.Chat(42)!;
        Assert.Equal(124, row.MaxReactionSeq);
        Assert.Equal(88, row.MaxEditSeq);
        Assert.Equal(89, row.MaxPollSeq);
        // Absent reads back as absent rather than as 0, which is the wire's own shape.
        store.Replace([Row(chat: new ChatDto(44, "direct", "Bob", 11))]);
        Assert.Null(store.Chat(44)!.MaxReactionSeq);
    }

    /// <summary>
    /// The read marker is MONOTONIC: a list read still in flight while the reader is reading must
    /// never walk it backwards.
    /// </summary>
    [Fact]
    public void TheReadMarkerOnlyEverMovesForward()
    {
        var store = Store();
        store.Replace([Row(read: 1000)]);
        store.MarkRead(42, 1337);
        Assert.Equal(1337, store.Chat(42)!.LastReadMessageId);
        // The stale answer.
        store.Replace([Row(read: 1000)]);
        Assert.Equal(1337, store.Chat(42)!.LastReadMessageId);
        store.MarkRead(42, 900);
        Assert.Equal(1337, store.Chat(42)!.LastReadMessageId);
    }

    [Fact]
    public void TheUnreadCountIsTheOtherHalfOfTheMarker()
    {
        var store = Store();
        store.Replace([Row()]);
        store.Apply([Message(1), Message(2), Message(3)]);
        Assert.Equal(3, store.Chat(42)!.UnreadCount);
        Assert.Equal(3, store.Unread());

        store.MarkRead(42, 2);
        Assert.Equal(1, store.Chat(42)!.UnreadCount);
        // An EDIT moves neither the count nor the ordering: it never re-notifies.
        store.Apply(Message(1, body: "rewritten", editSeq: 5));
        Assert.Equal(1, store.Chat(42)!.UnreadCount);

        store.MarkRead(42, 3);
        Assert.Equal(0, store.Unread());
    }

    [Fact]
    public void AMentionMarksTheChatUntilItIsRead()
    {
        var store = Store();
        store.Replace([Row()]);
        store.Apply(Message(10, body: "@Anna are you in?", mentions: [new MentionDto(9, "Anna")]));
        // Absent, never false — which is the wire's shape and what a list row draws its mark from.
        Assert.True(store.Chat(42)!.Mentioned);
        store.MarkRead(42, 10);
        Assert.Null(store.Chat(42)!.Mentioned);
    }

    [Fact]
    public void APageComesBackNewestFirstAndPagesBackwards()
    {
        var store = Store();
        store.Replace([Row()]);
        store.Apply([.. Enumerable.Range(1, 10).Select(id => Message(id))]);
        Assert.Equal([10L, 9L, 8L], store.Messages(42, limit: 3).Select(message => message.Id));
        Assert.Equal([7L, 6L], store.Messages(42, beforeId: 8, limit: 2).Select(m => m.Id));
        // The newest is the row's preview.
        Assert.Equal(10, store.Newest(42)!.Id);
        Assert.Equal(10, store.Chat(42)!.LastMessage!.Id);
    }

    /// <summary>
    /// A CHAIN IS ROOTED AT THE TOP and read far more often than it is written, which is why the
    /// root is stored rather than walked.
    /// </summary>
    [Fact]
    public void AChainIsTheRootAndEverythingThatNamesIt()
    {
        var store = Store();
        store.Replace([Row()]);
        store.Apply([
            Message(38, body: "Where shall we eat?"),
            Message(39, body: "The Italian place", threadRoot: 38),
            Message(40, body: "somewhere else entirely"),
            Message(41, body: "Six works", threadRoot: 38),
        ]);
        Assert.Equal([38L, 39L, 41L], store.Thread(38).Select(message => message.Id));
        // A message nobody answered is a chain of one.
        Assert.Equal([40L], store.Thread(40).Select(message => message.Id));
    }

    [Fact]
    public void AMemberWhoLeftKeepsTheirRowBecauseTheirMessagesKeepTheirAuthor()
    {
        var store = Store();
        store.Replace(
            [new MemberDto(7, "anna", "Anna", Owner: true)],
            [new MemberDto(11, "junior", "Junior", HasLeft: true, Deleted: true)]);
        Assert.Equal(2, store.Members().Count);
        var gone = store.Member(11)!;
        Assert.True(gone.HasLeft);
        Assert.True(gone.Deleted);
        Assert.True(store.Member(7)!.Owner);
        // An id this device has never heard of is null, and naming it is the caller's job —
        // "Deleted account" is a client's word, not the server's.
        Assert.Null(store.Member(99));
    }

    [Fact]
    public void TheListIsOrderedByWhenSomethingLastHappened()
    {
        var store = Store();
        store.Replace([Row(), Row(new ChatDto(43, "direct", "Anna", 9))]);
        store.Apply(Message(1, chat: 43, created: "2026-09-12T09:00:00Z"));
        store.Apply(Message(2, chat: 42, created: "2026-09-12T11:00:00Z"));
        Assert.Equal([42L, 43L], store.Chats().Select(row => row.Chat.Id));
        store.Apply(Message(3, chat: 43, created: "2026-09-12T12:00:00Z"));
        Assert.Equal([43L, 42L], store.Chats().Select(row => row.Chat.Id));

        // AN EDIT NEVER MOVES THE CHAT'S ORDERING, whatever its original timestamp: it is not a
        // new message, it never re-notifies and it never bumps a count. A list that jumped when
        // somebody fixed a typo three pages back would be a list nobody could keep their place in.
        store.Apply(Message(2, chat: 42, body: "rewritten", created: "2026-09-12T13:00:00Z",
            editSeq: 7));
        Assert.Equal([43L, 42L], store.Chats().Select(row => row.Chat.Id));
    }
}
