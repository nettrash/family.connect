using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.Core.Tests.Protocol;

/// <summary>
/// What a live frame does to the cache (docs/protocol.md, "Server → client", "Semantics").
/// </summary>
public class FrameRouterTests : IDisposable
{
    private const long Me = 7;

    private readonly Database database = Database.OpenInMemory();
    private readonly ChatStore chats;
    private readonly BoardStore board;
    private readonly FrameRouter router;

    public FrameRouterTests()
    {
        chats = new ChatStore(database, () => Me);
        board = new BoardStore(database);
        router = new FrameRouter(chats, board);
        chats.Replace([
            new ChatRowDto(new ChatDto(42, "family", "The Smiths")),
            new ChatRowDto(new ChatDto(43, "direct", "Bob", PeerUserId: 11)),
        ]);
    }

    public void Dispose() => database.Dispose();

    private static MessageDto Message(
        long id, long chat = 42, string body = "Dinner at 7?", long sender = 9,
        long? editSeq = null) =>
        new(id, chat, sender, null, body, "2026-09-12T10:00:00Z",
            EditSeq: editSeq, EditedAt: editSeq is null ? null : "2026-09-12T10:05:00Z");

    /// <summary>
    /// A message of the reader's OWN — the fan-out reaches their other devices as an ordinary
    /// `message` frame — is stored and announced and counts against nobody.
    /// </summary>
    [Fact]
    public void YourOwnMessageFromYourOtherDeviceCountsAgainstNobody()
    {
        router.Hear(new ServerFrame.Message(Message(1338, sender: Me)));

        Assert.Equal("Dinner at 7?", chats.Message(1338)!.Body);
        Assert.Equal(0, chats.Chat(42)!.UnreadCount);
    }

    /// <summary>
    /// A PAGE MAY NOT RAISE THE COUNT: the list read that preceded it already counted every
    /// message on it, so counting them again doubles the badge.
    /// </summary>
    [Fact]
    public void APageOfMessagesDoesNotCountWhatTheListAlreadyCounted()
    {
        chats.Replace([
            new ChatRowDto(new ChatDto(42, "family", "The Smiths"), UnreadCount: 2),
        ]);

        chats.Apply([Message(1338), Message(1339)]);

        Assert.Equal(2, chats.Chat(42)!.UnreadCount);
    }

    [Fact]
    public void AMessageIsStoredAndAnnounced()
    {
        MessageDto? arrived = null;
        var touched = new List<long>();
        router.Arrived += message => arrived = message;
        router.ChatChanged += chat => touched.Add(chat);

        router.Hear(new ServerFrame.Message(Message(1338)));

        Assert.Equal(1338, arrived!.Id);
        Assert.Equal("Dinner at 7?", chats.Message(1338)!.Body);
        Assert.Equal([42L], touched);
        Assert.Equal(1, chats.Chat(42)!.UnreadCount);
    }

    /// <summary>
    /// AN EDIT IS NOT A MESSAGE: it raises no arrival, and it moves the chat's edit cursor, which
    /// nothing else may. It also carries the WHOLE message — the assistant's picture answer
    /// arrives as an attachment added by exactly this frame.
    /// </summary>
    [Fact]
    public void AnEditAnnouncesItselfDifferentlyAndMovesTheEditCursor()
    {
        chats.Apply(Message(1338, body: "Dinner at 7?"));
        var arrivals = 0;
        MessageDto? edited = null;
        router.Arrived += _ => arrivals++;
        router.Edited += message => edited = message;

        router.Hear(new ServerFrame.MessageEdited(
            Message(1338, body: "Dinner at 8?", editSeq: 91) with
            {
                Attachments = [new AttachmentDto(34, "photo", Width: 1600, Height: 1200)],
            }));

        Assert.Equal(0, arrivals);
        Assert.Equal("Dinner at 8?", edited!.Body);
        Assert.Equal("Dinner at 8?", chats.Message(1338)!.Body);
        // The whole message, not the body alone.
        Assert.Equal(34, Assert.Single(chats.Message(1338)!.Media).Id);
        Assert.Equal(91, chats.Chat(42)!.MaxEditSeq);
    }

    /// <summary>
    /// A `read` FRAME IS TWO DIFFERENT FACTS. One naming YOURSELF is your own marker from your
    /// other device; one naming somebody else is roster data, and clearing your unread count from
    /// it would be reading somebody else's reading as your own.
    /// </summary>
    [Fact]
    public void AReadFrameNamingYouIsYourMarkerAndOneNamingAnybodyElseIsNot()
    {
        // As frames, because a live frame is the only route that may raise the count.
        router.Hear(new ServerFrame.Message(Message(1338)));
        router.Hear(new ServerFrame.Message(Message(1339)));
        Assert.Equal(2, chats.Chat(42)!.UnreadCount);

        // Somebody else read it: nothing of yours moves.
        router.Hear(new ServerFrame.Read(42, UserId: 9, LastReadMessageId: 1339));
        Assert.Equal(2, chats.Chat(42)!.UnreadCount);
        Assert.Equal(0, chats.Chat(42)!.LastReadMessageId);

        // You read it, on your laptop: the marker moves and the count follows.
        router.Hear(new ServerFrame.Read(42, UserId: Me, LastReadMessageId: 1339));
        Assert.Equal(0, chats.Chat(42)!.UnreadCount);
        Assert.Equal(1339, chats.Chat(42)!.LastReadMessageId);

        // And it is MONOTONIC: a frame still in flight while you read on cannot walk it back.
        router.Hear(new ServerFrame.Read(42, UserId: Me, LastReadMessageId: 1000));
        Assert.Equal(1339, chats.Chat(42)!.LastReadMessageId);
    }

    /// <summary>
    /// A peer's read marker is drawn in a DIRECT chat only: in the family chat a per-member seen
    /// state over N members is a row of faces nobody asked for, and a per-member ABSENCE is
    /// exactly what a blocker's suppressed inward frames would put on screen.
    /// </summary>
    [Fact]
    public void APeersReadMarkerIsSeenInADirectChatAndNowhereElse()
    {
        var seen = new List<long>();
        router.PeerRead += (chat, _, _) => seen.Add(chat);

        router.Hear(new ServerFrame.Read(42, UserId: 9, LastReadMessageId: 1339));
        router.Hear(new ServerFrame.Read(43, UserId: 11, LastReadMessageId: 500));

        Assert.Equal([43L], seen);
    }

    /// <summary>
    /// A live frame is a complete statement about that chat's sequence, so the cursor follows it
    /// EVEN FOR A MESSAGE THIS DEVICE DOES NOT HOLD — the state is dropped, the sequence still
    /// happened, and a cursor left behind asks for a page that answers the same nothing.
    /// </summary>
    [Fact]
    public void AReactionFrameMovesTheCursorEvenWhenItsMessageIsUnknown()
    {
        router.Hear(new ServerFrame.Reactions(42, MessageId: 9999, ReactionSeq: 124,
            Value: [new ReactionDto(9, "❤️")]));
        Assert.Null(chats.Message(9999));
        Assert.Equal(124, chats.Chat(42)!.MaxReactionSeq);

        chats.Apply(Message(1338));
        router.Hear(new ServerFrame.Reactions(42, 1338, 125, [new ReactionDto(9, "👍")]));
        Assert.Equal("👍", Assert.Single(chats.Message(1338)!.Reactions!).Emoji);
        Assert.Equal(125, chats.Chat(42)!.MaxReactionSeq);
    }

    [Fact]
    public void APollFrameIsCompleteStateAndMovesItsOwnCursor()
    {
        router.Hear(new ServerFrame.Message(Message(1340, body: "Pizza or pasta?")));
        Assert.Equal(1, chats.Chat(42)!.UnreadCount);

        router.Hear(new ServerFrame.Poll(42, 1340, new PollDto(88, false, [
            new PollOptionDto(5, "Pizza", [7]),
            new PollOptionDto(6, "Pasta", []),
        ])));
        Assert.Equal([7L], chats.Message(1340)!.Poll!.Options[0].Votes);
        Assert.Equal(88, chats.Chat(42)!.MaxPollSeq);
        // Never an unread count and never a notification: a vote is not a message, so the count
        // the message itself raised is all there is.
        Assert.Equal(1, chats.Chat(42)!.UnreadCount);

        // And a vote on a poll this device has never seen moves the cursor all the same.
        router.Hear(new ServerFrame.Poll(42, 9999, new PollDto(89, true, [])));
        Assert.Null(chats.Message(9999));
        Assert.Equal(89, chats.Chat(42)!.MaxPollSeq);
    }

    [Fact]
    public void ANoteFrameIsAppliedUnderItsOwnSequence()
    {
        NoteDto? changed = null;
        router.BoardChanged += note => changed = note;

        router.Hear(new ServerFrame.BoardNote(new NoteDto(12, 7, "text", "Milk", BoardSeq: 11)));
        Assert.Equal("Milk", Assert.Single(board.Notes()).Text);
        Assert.Equal(12, changed!.Id);

        // An out-of-order frame cannot undo a newer move.
        router.Hear(new ServerFrame.BoardNote(new NoteDto(12, 7, "text", "Bread", BoardSeq: 10)));
        Assert.Equal("Milk", Assert.Single(board.Notes()).Text);

        // And a tombstone takes it down.
        router.Hear(new ServerFrame.BoardNote(new NoteDto(12, 7, BoardSeq: 12, Deleted: true)));
        Assert.Empty(board.Notes());
    }

    [Fact]
    public void TheRosterFollowsItsFourFrames()
    {
        var changes = 0;
        router.RosterChanged += () => changes++;
        chats.Replace([
            new MemberDto(7, "anna", "Anna", Role: "owner"),
            new MemberDto(11, "bob", "Bob", Role: "member", Birthday: new BirthdayDto(3, 14)),
        ]);

        // A join, carrying a User — so a member, never an owner by arriving.
        router.Hear(new ServerFrame.MemberJoined(3, new UserDto(12, "kid", "Kid")));
        Assert.Equal("member", chats.Member(12)!.Role);

        // A frame that says nothing about a birthday leaves the one this device knows alone.
        router.Hear(new ServerFrame.MemberJoined(3, new UserDto(11, "bob", "Bobby")));
        Assert.Equal("Bobby", chats.Member(11)!.DisplayName);
        Assert.Equal(new BirthdayDto(3, 14), chats.Member(11)!.Birthday);

        // A leave keeps the row: their messages keep their author.
        router.Hear(new ServerFrame.MemberLeft(3, 12));
        Assert.True(chats.Member(12)!.HasLeft);
        Assert.Null(chats.Member(12)!.Role);

        // And a REJOIN takes the flag back off — as a member, because arriving is not owning:
        // the family passes on by its own frame and never by somebody walking back in.
        router.Hear(new ServerFrame.MemberJoined(3, new UserDto(12, "kid", "Kid")));
        Assert.False(chats.Member(12)!.HasLeft);
        Assert.Equal("member", chats.Member(12)!.Role);
        Assert.False(chats.Member(12)!.Owner);
        // The owner is still the owner: one join did not move it.
        Assert.True(chats.Member(7)!.Owner);

        // A deleted account is the one write whose job is to WIPE.
        router.Hear(new ServerFrame.MemberDeleted(
            3, new MemberDto(11, "", "Deleted account", Deleted: true)));
        var gone = chats.Member(11)!;
        Assert.True(gone.Deleted);
        Assert.True(gone.HasLeft);
        Assert.Equal(0, gone.AvatarVersion);
        Assert.Null(gone.Birthday);

        // And the family passes on: one owner, so whoever held it stops holding it.
        chats.Joined(new UserDto(14, "gran", "Gran"));
        router.Hear(new ServerFrame.FamilyOwner(3, 14));
        Assert.True(chats.Member(14)!.Owner);
        Assert.False(chats.Member(7)!.Owner);
        // Six frames, six announcements: three joins, a leave, a deletion and a new owner.
        Assert.Equal(6, changes);
    }

    /// <summary>
    /// `member_blocked` carries FULL CURRENT STATE rather than an event, so an unblock is the
    /// same frame with <c>false</c> — and it is the one frame about another member that reaches
    /// one person's own devices and stops there.
    /// </summary>
    [Fact]
    public void BlockingIsAStateSetAndTheChatFollowsIt()
    {
        chats.Apply(Message(500, chat: 43, body: "Are we still on?", sender: 11));
        var states = new List<bool>();
        router.BlockChanged += (_, blocked) => states.Add(blocked);

        router.Hear(new ServerFrame.MemberBlocked(11, true));
        Assert.True(chats.IsBlocked(11));
        Assert.False(chats.IsListed(43));

        router.Hear(new ServerFrame.MemberBlocked(11, false));
        Assert.False(chats.IsBlocked(11));
        Assert.True(chats.IsListed(43));
        // Nothing about the chat was ever deleted.
        Assert.Equal("Are we still on?", chats.Message(500)!.Body);
        Assert.Equal([true, false], states);
    }

    /// <summary>
    /// The assistant's deltas are NOT stored: the finished answer lands as `message_edited`, and
    /// a half-written body written into the cache is what a relaunch mid-answer would draw for
    /// ever.
    /// </summary>
    [Fact]
    public void TheAssistantsDeltasArePassedOnAndNotStored()
    {
        chats.Apply(Message(1341, body: ""));
        var text = "";
        router.AiDelta += (_, _, piece) => text += piece;
        var stopped = 0;
        router.AiStopped += (_, _) => stopped++;

        router.Hear(new ServerFrame.AiDelta(42, 1341, "Once "));
        router.Hear(new ServerFrame.AiDelta(42, 1341, "upon"));
        router.Hear(new ServerFrame.AiError(42, 1341));

        Assert.Equal("Once upon", text);
        Assert.Equal(1, stopped);
        Assert.Equal("", chats.Message(1341)!.Body);
    }

    /// <summary>
    /// An error that answers a send belongs to the pipeline, which is waiting for it; one that
    /// answers a call belongs to the call layer; anything else is nobody's, and is spoken.
    /// </summary>
    [Fact]
    public void AnErrorGoesToWhoeverIsWaitingForIt()
    {
        var refusals = new List<string>();
        var calls = new List<ServerFrame>();
        router.Refused += error => refusals.Add(error.Code);
        router.Call += frame => calls.Add(frame);

        router.Hear(new ServerFrame.Error("not_chat_member", "no", "8f14e45f", null));
        router.Hear(new ServerFrame.Error("peer_busy", "busy", null, "6a1f0c3e"));
        router.Hear(new ServerFrame.Error("internal", "oh dear", null, null));
        router.Hear(new ServerFrame.Pong());

        Assert.Equal(["internal"], refusals);
        Assert.Single(calls);
    }

    [Fact]
    public void ACallFrameIsPassedOnWholeBecauseSignallingIsNotACache()
    {
        var heard = new List<ServerFrame>();
        router.Call += frame => heard.Add(frame);

        router.Hear(new ServerFrame.CallOffer("6a1f0c3e", 43, 11, "v=0", Video: false));
        router.Hear(new ServerFrame.CallRinging("6a1f0c3e"));
        router.Hear(new ServerFrame.CallAnswer("6a1f0c3e", "v=0"));
        router.Hear(new ServerFrame.CallIce("6a1f0c3e", new IceCandidate("candidate:1")));
        router.Hear(new ServerFrame.CallEnd("6a1f0c3e", "hangup"));

        Assert.Equal(5, heard.Count);
    }
}
