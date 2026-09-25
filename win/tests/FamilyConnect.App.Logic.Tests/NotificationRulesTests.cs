using FamilyConnect.App.Logic;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// When this client speaks up, and what it says when it does.
/// </summary>
public class NotificationRulesTests : IDisposable
{
    private const long Me = 7;
    private const long Assistant = 1;

    private readonly Database cache = Database.OpenInMemory();
    private readonly ChatStore chats;
    private readonly NotificationRules rules;

    public NotificationRulesTests()
    {
        chats = new ChatStore(cache, () => Me);
        chats.Replace([
            new ChatRowDto(new ChatDto(42, "family", "The Smiths")),
            new ChatRowDto(new ChatDto(43, "direct", "Bob", PeerUserId: 11)),
        ]);
        chats.Replace([
            new MemberDto(7, "anna", "Anna", Role: "owner"),
            new MemberDto(11, "bob", "Bob", Role: "member"),
        ]);
        rules = new NotificationRules(chats)
        {
            Wanted = true,
            InFront = false,
            FamilyName = "The Smiths",
            AssistantUserId = Assistant,
        };
    }

    public void Dispose() => cache.Dispose();

    private static MessageDto Message(
        long id,
        long chat = 42,
        long sender = 11,
        string body = "Dinner at 7?",
        MentionDto[]? mentions = null,
        ReplyToDto? replyTo = null) =>
        new(id, chat, sender, null, body, "2026-09-12T10:00:00Z",
            Mentions: mentions, ReplyTo: replyTo);

    [Fact]
    public void TheFamilyChatNamesTheFamilyFirstAndTheBodyIsNeverTheMessage()
    {
        var toast = rules.ForMessage(Message(1338));

        // A notification centre shows a list, and "Bob" alone does not say where he said it.
        Assert.Equal("The Smiths — Bob", toast!.Title);
        Assert.Equal("New message", toast.Body);
        Assert.Equal("chat-42", toast.Tag);
        Assert.Equal(42, toast.ChatId);
    }

    [Fact]
    public void ADirectChatIsTheSenderAlone()
    {
        var toast = rules.ForMessage(Message(1338, chat: 43));

        Assert.Equal("Bob", toast!.Title);
    }

    [Fact]
    public void AMessageThatNamesTheReaderSaysSoInTheTitle()
    {
        var toast = rules.ForMessage(
            Message(1338, mentions: [new MentionDto(Me, "Anna")]));

        // The same notification, a different title — never a second one.
        Assert.Equal("The Smiths — Bob mentioned you", toast!.Title);
    }

    [Fact]
    public void NothingIsSaidWhileTheReaderIsLookingOrHasNotAsked()
    {
        rules.InFront = true;
        Assert.Null(rules.ForMessage(Message(1338)));

        rules.InFront = false;
        rules.Wanted = false;
        Assert.Null(rules.ForMessage(Message(1338)));

        rules.Wanted = true;
        Assert.NotNull(rules.ForMessage(Message(1338)));
    }

    [Fact]
    public void YourOwnMessageFromYourOtherDeviceSaysNothing()
    {
        Assert.Null(rules.ForMessage(Message(1338, sender: Me)));
    }

    /// <summary>
    /// THE BLOCK REACHES ONE STEP FURTHER THAN THE SENDER: the assistant's answer to a blocked
    /// member's question would light up a notification for a thread its reader cannot read.
    /// </summary>
    [Fact]
    public void ABlockedMembersMessageAndTheAssistantsAnswerToItBothSayNothing()
    {
        chats.SetBlocked(11, true);

        Assert.Null(rules.ForMessage(Message(1338, sender: 11)));
        Assert.Null(rules.ForMessage(Message(
            1339, sender: Assistant, replyTo: new ReplyToDto(1338, 11, "what is for dinner"))));

        // The assistant answering anybody else still speaks.
        Assert.NotNull(rules.ForMessage(Message(
            1340, sender: Assistant, replyTo: new ReplyToDto(1337, Me, "what is for dinner"))));
    }

    [Fact]
    public void AMessageInAChatThisDeviceDoesNotHoldSaysNothing()
    {
        // A notification that cannot be opened is worse than none: the list read that names this
        // chat has not landed yet.
        Assert.Null(rules.ForMessage(Message(1338, chat: 99)));
    }

    [Fact]
    public void ASenderTheRosterCannotNameIsSomebody()
    {
        var toast = rules.ForMessage(Message(1338, sender: 14));

        Assert.Equal("The Smiths — Someone", toast!.Title);
    }

    /// <summary>
    /// A NOTE IS NEWS ONLY WHEN IT SAYS SOMETHING NEW. The badge and the notification answer the
    /// same question, so the board's own verdict decides both.
    /// </summary>
    [Fact]
    public void ANoteSpeaksOnlyWhenTheBoardSaysItIsNews()
    {
        var note = new NoteDto(12, 11, "text", "Bins on Tuesday", BoardSeq: 11, ContentSeq: 11);

        var toast = rules.ForNote(note, isNews: true);
        Assert.Equal("The Smiths — Bob", toast!.Title);
        Assert.Equal("New note", toast.Body);
        Assert.Equal("board", toast.Tag);
        Assert.True(toast.IsBoard);

        // Dragged across the wall: no badge, and nothing said.
        Assert.Null(rules.ForNote(note, isNews: false));
        // Mine, and a blocked member's.
        Assert.Null(rules.ForNote(note with { AuthorId = Me }, isNews: true));
        chats.SetBlocked(11, true);
        Assert.Null(rules.ForNote(note, isNews: true));
    }

    [Fact]
    public void TheWindowTitleCarriesTheCount()
    {
        Assert.Equal("Family Connect", rules.WindowTitle("Family Connect"));

        chats.Apply(Message(1338), SeqRoute.LiveFrame);
        chats.Apply(Message(1339), SeqRoute.LiveFrame);
        Assert.Equal("(2) Family Connect", rules.WindowTitle("Family Connect"));
    }
}
