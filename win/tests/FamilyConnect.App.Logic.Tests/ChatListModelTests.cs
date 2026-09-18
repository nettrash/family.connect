using FamilyConnect.App.Logic;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// The chat list: the order, the counts, and the one line under each name.
/// </summary>
public class ChatListModelTests : IDisposable
{
    private const long Me = 7;

    private static readonly DateTimeOffset Now =
        new(2026, 9, 12, 12, 0, 0, TimeSpan.Zero);

    private readonly Database cache = Database.OpenInMemory();
    private readonly ChatStore chats;
    private readonly ChatListModel list;

    public ChatListModelTests()
    {
        chats = new ChatStore(cache);
        list = new ChatListModel(chats, () => Me);
    }

    public void Dispose() => cache.Dispose();

    private static MessageDto Message(
        long id,
        long chat = 42,
        string body = "Dinner at 7?",
        long sender = 9,
        DateTimeOffset? at = null,
        AttachmentDto[]? media = null,
        CallRecordDto? call = null) =>
        new(
            id, chat, sender, null, body,
            // Through the product's OWN writer, and therefore invariantly: a fixture that
            // formatted the wire's instants in the machine's language would hand the product
            // `16.00.00` on a Finnish computer and fail for the one reason that is not a bug.
            Times.Rfc3339((at ?? Now.AddMinutes(-5)).ToUnixTimeMilliseconds())!,
            Attachments: media,
            Call: call);

    private ChatRow Row(long chatId) =>
        list.Rows(Now).Single(row => row.Chat.Id == chatId);

    [Fact]
    public void TheRowsAreInTheOrderSomethingLastHappenedAndTheCountsAreTheServersOwn()
    {
        chats.Replace([
            new ChatRowDto(
                new ChatDto(42, "family", "The Smiths"),
                LastMessage: Message(1338, at: Now.AddHours(-3)),
                UnreadCount: 3,
                Mentioned: true),
            new ChatRowDto(
                new ChatDto(43, "direct", "Bob", PeerUserId: 11),
                LastMessage: Message(1400, chat: 43, at: Now.AddMinutes(-2)),
                UnreadCount: 0),
        ]);
        chats.Apply([
            Message(1338, at: Now.AddHours(-3)),
            Message(1400, chat: 43, at: Now.AddMinutes(-2)),
        ]);

        var rows = list.Rows(Now);

        // THE FAMILY CHAT IS FIRST even though the direct chat is the busier one: it is the room
        // everybody is in, and Apple and the web both pin it there.
        Assert.Equal([42L, 43L], rows.Select(row => row.Chat.Id));
        var family = rows[0];
        Assert.Equal("The Smiths", family.Title);
        Assert.True(family.IsFamily);
        // Counted by the server, not by this device.
        Assert.Equal(3, family.Unread);
        Assert.True(family.Mentioned);
        Assert.Equal(RowTimeKind.Clock, family.When);
    }

    /// <summary>
    /// Everything below the family chat goes newest conversation first, by the ID of its newest
    /// message: ids are monotonic, where a stamp can be moved by a server's clock and is
    /// deliberately not moved by an edit.
    /// </summary>
    [Fact]
    public void BelowTheFamilyChatTheNewestConversationIsFirst()
    {
        chats.Replace([
            new ChatRowDto(new ChatDto(42, "family", "The Smiths")),
            new ChatRowDto(new ChatDto(43, "direct", "Bob", PeerUserId: 11)),
            new ChatRowDto(new ChatDto(44, "direct", "Gran", PeerUserId: 14)),
            new ChatRowDto(new ChatDto(45, "ai", "Assistant")),
        ]);
        chats.Apply([
            Message(100, chat: 43, at: Now.AddDays(-1)),
            Message(300, chat: 44, at: Now.AddMinutes(-1)),
        ]);

        // 44 has the newest message, 43 an older one, and the assistant's chat has none at all.
        Assert.Equal([42L, 44L, 43L, 45L], list.Rows(Now).Select(row => row.Chat.Id));
    }

    [Fact]
    public void AChatWithNothingInItSaysSoRatherThanShowingAnEmptyLine()
    {
        chats.Replace([new ChatRowDto(new ChatDto(42, "family", "The Smiths"))]);

        var row = Row(42);

        Assert.Equal("No messages yet", row.Preview);
        Assert.Equal(RowTimeKind.None, row.When);
        Assert.Null(row.At);
    }

    [Fact]
    public void ThePreviewIsTheFirstLineAndNoMore()
    {
        chats.Replace([new ChatRowDto(new ChatDto(42, "family", "The Smiths"))]);
        chats.Apply(Message(1338, body: "first line\nsecond line\nthird"));

        // A row is one line high; a body with newlines in it would push every row below it down.
        Assert.Equal("first line", Row(42).Preview);
    }

    [Fact]
    public void ACaptionlessAttachmentReadsAsWhatItIs()
    {
        chats.Replace([new ChatRowDto(new ChatDto(42, "family", "The Smiths"))]);

        void Newest(params AttachmentDto[] media) =>
            chats.Apply(Message(1338, body: string.Empty, media: media));

        Newest(new AttachmentDto(34, "photo", Width: 1600, Height: 1200));
        Assert.Equal("Photo", Row(42).Preview);

        Newest(
            new AttachmentDto(34, "photo"),
            new AttachmentDto(35, "photo"));
        Assert.Equal("2 Photos", Row(42).Preview);

        Newest(new AttachmentDto(36, "video"));
        Assert.Equal("Video", Row(42).Preview);

        // The row for a message that IS the recording says so, where the attachment's own name
        // would only say "Audio".
        Newest(new AttachmentDto(37, "audio"));
        Assert.Equal("Voice message", Row(42).Preview);

        Newest(new AttachmentDto(38, "location", Latitude: 55.7558, Longitude: 37.6173));
        Assert.Equal("Location", Row(42).Preview);

        // A file IS its name.
        Newest(new AttachmentDto(39, "file", Name: "receipts.pdf"));
        Assert.Equal("receipts.pdf", Row(42).Preview);

        // And a kind this build never heard of still says something happened.
        Newest(new AttachmentDto(40, "hologram"));
        Assert.Equal("Photo", Row(42).Preview);
    }

    [Fact]
    public void AttachmentsRideOnACaptionRatherThanReplacingIt()
    {
        chats.Replace([new ChatRowDto(new ChatDto(42, "family", "The Smiths"))]);
        chats.Apply(Message(
            1338, body: "look at this", media: [new AttachmentDto(34, "photo")]));

        Assert.Equal("look at this", Row(42).Preview);
    }

    /// <summary>
    /// A CALL READS AS A CALL. The server writes an English placeholder into a record's body for
    /// clients that predate calls, and a client that knows the object never shows it.
    /// </summary>
    [Fact]
    public void ACallRecordReadsAsACallAndNeverAsItsPlaceholderBody()
    {
        chats.Replace([new ChatRowDto(new ChatDto(43, "direct", "Bob", PeerUserId: 11))]);

        // A record is immutable and each call is its own message, so each one gets its own id:
        // an upsert never rewrites a message's sender, and nor does a real server.
        void Record(long id, string outcome, int? seconds, long sender) =>
            chats.Apply(Message(
                id, chat: 43, body: "Voice call", sender: sender,
                call: new CallRecordDto(outcome, seconds)));

        Record(1500, "completed", 222, sender: 11);
        Assert.Equal("Voice call · 3:42", Row(43).Preview);

        // Whose call it was changes the sentence: mine that nobody answered…
        Record(1501, "missed", null, sender: Me);
        Assert.Equal("No answer", Row(43).Preview);

        // …and theirs that I never answered.
        Record(1502, "missed", null, sender: 11);
        Assert.Equal("Missed voice call", Row(43).Preview);
    }

    /// <summary>
    /// A BLOCKED MEMBER'S MESSAGE IS THE HIDDEN ROW HERE TOO, with no name and no reveal: a list
    /// is not where a person peeks. In the family chat it still counts, because the count is the
    /// other half of the read marker.
    /// </summary>
    [Fact]
    public void ABlockedSendersPreviewIsTheHiddenRowAndTheCountStands()
    {
        chats.Replace([
            new ChatRowDto(new ChatDto(42, "family", "The Smiths"), UnreadCount: 2),
        ]);
        chats.Apply(Message(1338, body: "something unpleasant", sender: 11));
        chats.SetBlocked(11, true);

        var row = Row(42);

        Assert.True(row.Hidden);
        Assert.Equal("Hidden — blocked member", row.Preview);
        Assert.Equal(2, row.Unread);

        // My own words are never hidden from me, whoever else I have blocked.
        chats.Apply(Message(1339, body: "mine", sender: Me));
        Assert.False(Row(42).Hidden);
        Assert.Equal("mine", Row(42).Preview);

        // Not even if this device somehow held its OWN id on the block list. The server refuses
        // a self-block (`cannot_block_self`), so this is insurance against a cache rather than a
        // reachable state — and insurance that is never checked is a comment.
        chats.SetBlocked(Me, true);
        Assert.False(Row(42).Hidden);
        Assert.Equal("mine", Row(42).Preview);
    }

    [Fact]
    public void ADirectChatWithNoTitleFallsBackToTheRosterAndThenToAWord()
    {
        chats.Replace([new ChatRowDto(new ChatDto(43, "direct", string.Empty, PeerUserId: 11))]);
        Assert.Equal("Chat", Row(43).Title);

        chats.Replace([new MemberDto(11, "bob", "Bob", Role: "member")]);
        Assert.Equal("Bob", Row(43).Title);

        // And the wire's own title wins when it has one: the server recomputes it per read.
        chats.Replace([new ChatRowDto(new ChatDto(43, "direct", "Bobby", PeerUserId: 11))]);
        Assert.Equal("Bobby", Row(43).Title);
    }

    /// <summary>
    /// Days are compared in the READER's zone: a message sent at 23:30 last night is "Yesterday"
    /// to the person reading it, whatever the clock says in Greenwich.
    /// </summary>
    [Fact]
    public void TheRowTimeIsADecisionAndNotAFormat()
    {
        var now = new DateTimeOffset(2026, 9, 12, 12, 0, 0, TimeSpan.Zero).ToLocalTime();
        Assert.Equal(RowTimeKind.Clock, ChatListModel.When(now.AddHours(-2), now));
        Assert.Equal(RowTimeKind.Yesterday, ChatListModel.When(now.AddDays(-1), now));
        Assert.Equal(RowTimeKind.Weekday, ChatListModel.When(now.AddDays(-3), now));
        Assert.Equal(RowTimeKind.Date, ChatListModel.When(now.AddDays(-20), now));
        // A stamp from the future is not "in six days": it is a date.
        Assert.Equal(RowTimeKind.Date, ChatListModel.When(now.AddDays(1), now));
    }

    [Fact]
    public void AHiddenChatIsNotOnTheListAtAll()
    {
        chats.Replace([
            new ChatRowDto(new ChatDto(42, "family", "The Smiths")),
            new ChatRowDto(new ChatDto(43, "direct", "Bob", PeerUserId: 11)),
        ]);
        chats.SetBlocked(11, true);

        Assert.Equal([42L], list.Rows(Now).Select(row => row.Chat.Id));
    }
}
