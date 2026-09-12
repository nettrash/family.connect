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
        Assert.True(store.ApplyReactions(42, 1341, 10, [new ReactionDto(9, "❤️")]));
        Assert.Equal("❤️", Assert.Single(store.Message(1341)!.Reactions!).Emoji);
        // Cleared is `[]` and the field STAYS: a client tells "cleared" from "no data".
        Assert.True(store.ApplyReactions(42, 1341, 11, []));
        Assert.NotNull(store.Message(1341)!.Reactions);
        Assert.Empty(store.Message(1341)!.Reactions!);
        // And an older frame that crossed a newer one is dropped.
        Assert.False(store.ApplyReactions(42, 1341, 9, [new ReactionDto(9, "👍")]));
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
        Assert.True(store.ApplyPoll(42, 1342, new PollDto(88, false, [
            new PollOptionDto(5, "Pizza", [7]),
            new PollOptionDto(6, "Pasta", []),
        ])));
        Assert.Equal([7L], store.Message(1342)!.Poll!.Options[0].Votes);
        Assert.False(store.ApplyPoll(42, 1342, new PollDto(87, true, [])));
        Assert.False(store.Message(1342)!.Poll!.Closed);
        Assert.Equal(88, store.Chat(42)!.MaxPollSeq);
    }

    /// <summary>
    /// A STATE FOR A MESSAGE THIS DEVICE DOES NOT HOLD IS DROPPED, AND THE CURSOR STILL MOVES.
    /// History paging re-delivers such a state embedded on the message itself, while a cursor
    /// left behind asks for a page that answers the same nothing — for ever, because the chat's
    /// mark never comes back down.
    /// </summary>
    [Fact]
    public void AStateForAnUnknownMessageIsDroppedAndTheCursorMovesAnyway()
    {
        var store = Store();
        store.Replace([Row()]);

        Assert.False(store.ApplyReactions(42, 9999, 124, [new ReactionDto(9, "❤️")]));
        Assert.Null(store.Message(9999));
        Assert.Equal(124, store.Chat(42)!.MaxReactionSeq);

        Assert.False(store.ApplyPoll(42, 9998, new PollDto(88, false, [])));
        Assert.Equal(88, store.Chat(42)!.MaxPollSeq);

        // Evidence about a message this device has never seen is evidence about nothing.
        Assert.False(store.ApplyReactions(
            42, 9997, 300, [new ReactionDto(9, "👍")], SeqRoute.Evidence));
        Assert.Equal(124, store.Chat(42)!.MaxReactionSeq);
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
        Assert.True(store.ApplyReactions(42, 1, 300, [new ReactionDto(7, "❤️")], SeqRoute.Evidence));
        Assert.Equal("❤️", Assert.Single(store.Message(1)!.Reactions!).Emoji);
        Assert.Null(store.Chat(42)!.MaxReactionSeq);

        // The answer to our own vote: the same.
        Assert.True(store.ApplyPoll(
            42, 2, new PollDto(400, false, [new PollOptionDto(5, "Pizza", [7])]),
            SeqRoute.Evidence));
        Assert.Equal([7L], store.Message(2)!.Poll!.Options[0].Votes);
        Assert.Null(store.Chat(42)!.MaxPollSeq);

        // A live frame and a catch-up page both move it.
        store.ApplyReactions(42, 1, 301, [], SeqRoute.LiveFrame);
        Assert.Equal(301, store.Chat(42)!.MaxReactionSeq);
        store.ApplyPoll(42, 2, new PollDto(401, true, []), SeqRoute.CatchUpPage);
        Assert.Equal(401, store.Chat(42)!.MaxPollSeq);
    }

    /// <summary>
    /// THE THREE CURSORS ARE THIS DEVICE'S, NOT THE SERVER'S. A <c>GET /chats</c> row carries
    /// <c>max_*_seq</c> too, with the same names and a different meaning — the server's maximum —
    /// and a list read must not write them anywhere near these columns: step 3's catch-up runs
    /// only "when the chat's max_reaction_seq from step 2 exceeds the locally stored reaction
    /// cursor", so a device that stored the server's mark as its own would never ask for a page
    /// again. They move under <see cref="ChatStore.Advance"/> alone, and there as high-water
    /// marks: a 0 that arrives after a 124 is not a reset.
    /// </summary>
    [Fact]
    public void TheCatchUpCursorsAreLocalAndAListReadDoesNotWriteThem()
    {
        var store = Store();
        store.Replace([Row(reactionSeq: 124, editSeq: 88, pollSeq: 89)]);
        var fresh = store.Chat(42)!;
        // The server says 124; this device has applied nothing, and says so.
        Assert.Null(fresh.MaxReactionSeq);
        Assert.Null(fresh.MaxEditSeq);
        Assert.Null(fresh.MaxPollSeq);

        // What this device applies is what its cursors read back as…
        store.Advance(42, reactionSeq: 124, editSeq: 88, pollSeq: 89);
        Assert.Equal(124, store.Chat(42)!.MaxReactionSeq);
        Assert.Equal(88, store.Chat(42)!.MaxEditSeq);
        Assert.Equal(89, store.Chat(42)!.MaxPollSeq);

        // …and another list read neither lowers them nor raises them.
        store.Replace([Row()]);
        Assert.Equal(124, store.Chat(42)!.MaxReactionSeq);
        store.Replace([Row(reactionSeq: 900)]);
        Assert.Equal(124, store.Chat(42)!.MaxReactionSeq);

        // A high-water mark: an older page cannot walk one back.
        store.Advance(42, reactionSeq: 5);
        Assert.Equal(124, store.Chat(42)!.MaxReactionSeq);

        // And a chat this device has just learned of starts level with nothing.
        store.Replace([Row(chat: new ChatDto(44, "direct", "Bob", 11), reactionSeq: 300)]);
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

    /// <summary>
    /// THE COUNT IS INCREMENTED BY A LIVE FRAME AND NEVER RECOMPUTED FROM WHAT IS HELD. Local
    /// history is not the whole of it — retention has swept some of it, paging has not fetched
    /// the rest — so a recount is a badge that silently falls to one the moment anything arrives:
    /// a device told "12 unread" by the list, then handed one live message, would draw 1.
    /// </summary>
    [Fact]
    public void TheUnreadCountIsTheOtherHalfOfTheMarker()
    {
        var store = Store();
        store.Replace([Row()]);
        store.Apply(Message(1), SeqRoute.LiveFrame);
        store.Apply(Message(2), SeqRoute.LiveFrame);
        store.Apply(Message(3), SeqRoute.LiveFrame);
        Assert.Equal(3, store.Chat(42)!.UnreadCount);
        Assert.Equal(3, store.Unread());

        store.MarkRead(42, 2);
        Assert.Equal(1, store.Chat(42)!.UnreadCount);
        // An EDIT moves neither the count nor the ordering: it never re-notifies.
        store.Apply(Message(1, body: "rewritten", editSeq: 5), SeqRoute.LiveFrame);
        Assert.Equal(1, store.Chat(42)!.UnreadCount);

        store.MarkRead(42, 3);
        Assert.Equal(0, store.Unread());
    }

    /// <summary>
    /// The four conditions on raising the count, each of which is a badge bug on its own.
    /// </summary>
    [Fact]
    public void OnlySomebodyElsesNewLiveMessageAboveTheMarkerCounts()
    {
        // Somebody who is NOT the fixture's default sender, so that "somebody else's message"
        // and "my own" are told apart by the store and not by the test's luck.
        var mine = 99L;
        var store = new ChatStore(database, () => mine);
        store.Replace([Row(read: 10)]);

        // A page: the list read that preceded it already counted these.
        store.Apply([Message(11), Message(12)]);
        Assert.Equal(0, store.Chat(42)!.UnreadCount);

        // A live frame from somebody else, above the marker: one.
        store.Apply(Message(13), SeqRoute.LiveFrame);
        Assert.Equal(1, store.Chat(42)!.UnreadCount);

        // The same message again — a lost ack re-delivered as an ordinary message — is not two.
        store.Apply(Message(13), SeqRoute.LiveFrame);
        Assert.Equal(1, store.Chat(42)!.UnreadCount);

        // This reader's own send, arriving at their other device: nothing.
        store.Apply(Message(14, sender: mine), SeqRoute.LiveFrame);
        Assert.Equal(1, store.Chat(42)!.UnreadCount);

        // Something from BELOW the marker, arriving late: read before it got here.
        store.Apply(Message(5), SeqRoute.LiveFrame);
        Assert.Equal(1, store.Chat(42)!.UnreadCount);

        // An EDIT never counts, not even for a message this device has never held: the frame
        // exists precisely so that a rewrite bumps nothing and notifies nobody.
        store.Apply(Message(20, body: "rewritten", editSeq: 7), SeqRoute.LiveFrame);
        Assert.Equal("rewritten", store.Message(20)!.Body);
        Assert.Equal(1, store.Chat(42)!.UnreadCount);
    }

    /// <summary>
    /// A list read's count is the SERVER's and is never lowered to what this device can see —
    /// but neither is it allowed to be lower than that: a message that arrived while the read
    /// was in flight is not on that answer, and the two numbers are both lower bounds.
    /// </summary>
    [Fact]
    public void AListReadTakesTheServersCountAndNeverLessThanWhatIsVisible()
    {
        var store = new ChatStore(database, () => 99);
        store.Replace([Row(unread: 12)]);
        Assert.Equal(12, store.Chat(42)!.UnreadCount);

        // A frame lands while the next list read is in flight; that read says 12 again.
        store.Apply(Message(99), SeqRoute.LiveFrame);
        Assert.Equal(13, store.Chat(42)!.UnreadCount);
        store.Replace([Row(unread: 12)]);
        Assert.Equal(13, store.Chat(42)!.UnreadCount);

        // And a read that reports the message as read brings it back down.
        store.Replace([Row(unread: 0, read: 99)]);
        Assert.Equal(0, store.Chat(42)!.UnreadCount);

        // A message of the READER'S OWN, racing the same read, adds nothing: it was never
        // unread, and the server did not count it either.
        store.Apply(Message(150, sender: 99), SeqRoute.LiveFrame);
        store.Replace([Row(unread: 0, read: 99, last: Message(120))]);
        Assert.Equal(0, store.Chat(42)!.UnreadCount);
    }

    /// <summary>
    /// READING PART OF A RUN IS SUBTRACTION, not a recount. A device told "12 unread" that holds
    /// the last three of them and reads two is ten behind; a recount from local history would say
    /// ONE, which is the badge quietly going out on a conversation nine messages deep.
    /// </summary>
    /// <remarks>
    /// The marker is an id THRESHOLD, so reading up to the newest message this device holds means
    /// everything at or below it is read — that is the other branch, and it is 0. Where the two
    /// disagree is a PARTIAL read, and there the subtraction can leave the count too HIGH when
    /// local history is sparse (the server had more messages in the range than this device holds).
    /// Too high is the safe direction: the next list read replaces it.
    /// </remarks>
    [Fact]
    public void ReadingPartOfARunSubtractsRatherThanRecounting()
    {
        var store = new ChatStore(database, () => 99);
        store.Replace([Row(unread: 12)]);
        store.Apply([Message(498), Message(499), Message(500)]);

        store.MarkRead(42, 499);

        Assert.Equal(10, store.Chat(42)!.UnreadCount);
        Assert.Equal(499, store.Chat(42)!.LastReadMessageId);

        // And reading to the bottom of what is held is the other branch: the threshold covers
        // everything this device could know about.
        store.MarkRead(42, 500);
        Assert.Equal(0, store.Chat(42)!.UnreadCount);
    }

    /// <summary>
    /// A MENTION THIS DEVICE RAISED IS NOT CLEARED BY AN ANSWER THAT DID NOT SEE IT. `mentioned`
    /// rides on a list row only while an unread message names the caller, and a read that crossed
    /// the frame carrying one would otherwise take the mark off a message still unread.
    /// </summary>
    [Fact]
    public void AMentionRaisedByAFrameSurvivesAListReadThatMissedIt()
    {
        var store = new ChatStore(database, () => 99);
        store.Replace([Row()]);
        store.Apply(
            Message(600, mentions: [new MentionDto(99, "Anna")]),
            SeqRoute.LiveFrame);
        Assert.True(store.Chat(42)!.Mentioned);

        // The list read was in flight when the frame landed, so it says nothing about a mention.
        store.Replace([Row(unread: 1)]);
        Assert.True(store.Chat(42)!.Mentioned);

        // Reading past it is what takes the mark off.
        store.MarkRead(42, 600);
        Assert.Null(store.Chat(42)!.Mentioned);
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
            [new MemberDto(7, "anna", "Anna", Role: "owner")],
            [new MemberDto(11, "junior", "Junior", HasLeft: true, Deleted: true)]);
        Assert.Equal(2, store.Members().Count);
        var gone = store.Member(11)!;
        Assert.True(gone.HasLeft);
        Assert.True(gone.Deleted);
        Assert.True(gone.IsFormer);
        // The ROLE is the wire's own word, and a former member has none — which is not the same
        // answer as "a member who is not the owner".
        Assert.Null(gone.Role);
        Assert.True(store.Member(7)!.Owner);
        Assert.Equal("owner", store.Member(7)!.Role);
        // An id this device has never heard of is null, and naming it is the caller's job —
        // "Deleted account" is a client's word, not the server's.
        Assert.Null(store.Member(99));
    }

    /// <summary>
    /// THE BLOCK LIST IS COMPLETE STATE. Both reads that carry it carry all of it, there is no
    /// catch-up feed to cursor and nothing to miss, and an absent list means nobody rather than
    /// "leave what you hold alone". An id on it may name somebody the roster cannot.
    /// </summary>
    [Fact]
    public void TheBlockListIsCompleteStateAndNeedNotResolveToAName()
    {
        var store = Store();
        store.Replace([new MemberDto(11, "bob", "Bob", Role: "member")]);
        store.ReplaceBlocked([11, 14, 11]);
        Assert.Equal([11L, 14L], store.Blocked());
        Assert.True(store.IsBlocked(11));
        // 14 is nobody this device has heard of — a blocked member who left, or whose account is
        // gone — and the list carries them anyway.
        Assert.True(store.IsBlocked(14));
        Assert.Null(store.Member(14));

        store.ReplaceBlocked([]);
        Assert.Empty(store.Blocked());
        Assert.False(store.IsBlocked(11));
    }

    /// <summary>
    /// The `member_blocked` frame is a STATE-SET, and it is the one place a client applies the
    /// consequence itself: the frame reaches the blocker's own devices and no list read is
    /// coming. The chat leaves the list and comes back WHOLE, because nothing about it is ever
    /// deleted (docs/protocol.md, "Blocking a member").
    /// </summary>
    [Fact]
    public void BlockingAMemberHidesTheirChatAndUnblockingBringsItBackWhole()
    {
        var store = Store();
        var direct = new ChatDto(43, "direct", "Bob", PeerUserId: 11);
        store.Replace([Row(), Row(direct)]);
        store.Apply(Message(500, chat: 43, body: "Are we still on?", sender: 11));

        store.SetBlocked(11, true);
        Assert.Equal([42L], store.Chats().Select(row => row.Chat.Id));
        Assert.False(store.IsListed(43));
        // Hidden, not deleted: the marker, the cursors and every word are still here.
        Assert.NotNull(store.Chat(43));
        Assert.Equal("Are we still on?", store.Message(500)!.Body);

        store.SetBlocked(11, false);
        Assert.Equal([42L, 43L], store.Chats().Select(row => row.Chat.Id).Order());
        Assert.Equal("Are we still on?", store.Message(500)!.Body);

        // The family chat is NOT hidden by a block: a blocked member's message still arrives,
        // still counts and may still be the preview — the count is the other half of the read
        // marker, and projecting one without the other desynchronises them.
        store.SetBlocked(11, true);
        Assert.Contains(42L, store.Chats().Select(row => row.Chat.Id));
    }

    /// <summary>
    /// A complete-state read does NOT decide what is on the list — the list read that follows it
    /// does, and two writers for one fact would fight over it.
    /// </summary>
    [Fact]
    public void ReadingTheBlockListDoesNotRelistAChatTheServerLeftOut()
    {
        var store = Store();
        var direct = new ChatDto(43, "direct", "Bob", PeerUserId: 11);
        store.Replace([Row(), Row(direct)]);
        // The server stopped listing it: this reader has blocked Bob.
        store.Replace([Row()]);
        Assert.False(store.IsListed(43));

        // Reading `/me` again says the same thing about Bob and must not undo it.
        store.ReplaceBlocked([11]);
        Assert.False(store.IsListed(43));
        // Nor does a list that no longer names him relist the chat on its own.
        store.ReplaceBlocked([]);
        Assert.False(store.IsListed(43));
    }

    /// <summary>
    /// A birthday is a MONTH AND A DAY, arriving as one object — and it survives the store,
    /// because the family calendar is drawn from the roster and nothing else replays it.
    /// </summary>
    [Fact]
    public void ABirthdayIsAMonthAndADayAndItSurvivesTheRoster()
    {
        var store = Store();
        var anna = Wire.Decode<MemberDto>(
            """
            {"id": 7, "username": "anna", "display_name": "Anna", "role": "owner",
             "birthday": {"month": 3, "day": 14}}
            """);
        Assert.True(anna!.Owner);
        // A field whose type is wrong does not degrade — it takes the whole answer with it.
        Assert.NotNull(anna);
        Assert.Equal(new BirthdayDto(3, 14), anna!.Birthday);

        store.Replace([anna, new MemberDto(11, "bob", "Bob")]);
        Assert.Equal(new BirthdayDto(3, 14), store.Member(7)!.Birthday);
        // Nobody's birthday is not everybody's: absent stays absent.
        Assert.Null(store.Member(11)!.Birthday);

        // 29 February is a birthday here: there is no year for it to fail to exist in.
        store.Replace([anna with { Birthday = new BirthdayDto(2, 29) }]);
        Assert.Equal(new BirthdayDto(2, 29), store.Member(7)!.Birthday);
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
