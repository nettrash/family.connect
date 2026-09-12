using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic;

/// <summary>
/// One notification to raise: what it says, what it is FOR, and the tag that makes a second one
/// about the same thing replace the first rather than stack on it.
/// </summary>
public sealed record Toast(string Tag, string Title, string Body, long? ChatId = null)
{
    /// <summary>Whether this one opens the board rather than a chat.</summary>
    public bool IsBoard => ChatId is null;
}

/// <summary>
/// When this client raises a notification of its own, and what it says.
/// </summary>
/// <remarks>
/// <para>
/// <b>THE BODY IS NEVER THE MESSAGE.</b> It is the one a server with
/// <c>include_message_body = false</c> would send: a desktop client cannot know whether the
/// operator wanted family text on a lock screen, so it never puts it there.
/// </para>
/// <para>
/// <b>THE BLOCK REACHES ONE STEP FURTHER THAN THE SENDER.</b> A blocked member's message raises
/// nothing, and neither does the ASSISTANT'S ANSWER TO ONE — that would light up a notification
/// for a thread its reader cannot read.
/// </para>
/// <para>
/// <b>A NOTE IS NEWS ONLY WHEN IT SAYS SOMETHING NEW.</b> A note somebody dragged across the wall
/// raises no badge, and notifies nobody either — the badge and the notification answer the same
/// question and must not disagree.
/// </para>
/// <para>
/// Nothing is raised while the window is in FRONT (the reader is looking at it) or while the
/// reader has not asked for notifications at all.
/// </para>
/// </remarks>
public sealed class NotificationRules(ChatStore chats, IStringCatalog? words = null)
{
    private readonly IStringCatalog say = words ?? EnglishCatalog.Instance;

    /// <summary>Whether the reader has asked for these at all.</summary>
    public bool Wanted { get; set; }

    /// <summary>Whether the window is in front, in which case there is nothing to tell.</summary>
    public bool InFront { get; set; }

    /// <summary>The family's name, for the title of anything from the family chat or the wall.</summary>
    public string? FamilyName { get; set; }

    /// <summary>The assistant's user id, so its answers can be gated with the member they answer.</summary>
    public long? AssistantUserId { get; set; }

    /// <summary>
    /// The toast for an arriving message, or null when there is nothing to say.
    /// </summary>
    public Toast? ForMessage(MessageDto message)
    {
        if (!Speaking)
        {
            return null;
        }
        var me = chats.Reader;
        if (message.SenderId == me || chats.IsBlocked(message.SenderId))
        {
            return null;
        }
        // The block, one step further: the assistant answering somebody this reader cannot see.
        if (AssistantUserId == message.SenderId
            && message.ReplyTo is { } quote
            && chats.IsBlocked(quote.SenderId))
        {
            return null;
        }
        var chat = chats.Chat(message.ChatId);
        if (chat is null)
        {
            // A chat this device does not hold: the list read that names it has not landed, and
            // a notification that cannot be opened is worse than none.
            return null;
        }
        var mentioned = (message.Mentions ?? []).Any(mention => mention.UserId == me);
        return new Toast(
            $"chat-{message.ChatId}",
            NotifyText.Title(
                chat.Chat.Kind == "family" ? FamilyName : null, NameOf(message.SenderId),
                mentioned, say),
            NotifyText.NewMessage(say),
            message.ChatId);
    }

    /// <summary>
    /// The toast for a note that has just said something new, or null. <paramref name="isNews"/>
    /// is the board's own verdict — the badge's — so the two cannot disagree.
    /// </summary>
    public Toast? ForNote(NoteDto note, bool isNews)
    {
        if (!Speaking || !isNews)
        {
            return null;
        }
        if (note.AuthorId == chats.Reader || chats.IsBlocked(note.AuthorId))
        {
            return null;
        }
        return new Toast(
            "board",
            NotifyText.Title(FamilyName, NameOf(note.AuthorId), mentioned: false, say),
            NotifyText.NewNote(say));
    }

    /// <summary>
    /// What the window's own title says: the count, then the name. A hundred and more is "99+" —
    /// the exact number stops being the point.
    /// </summary>
    public string WindowTitle(string brand) => NotifyText.WindowTitle(brand, chats.Unread());

    /// <summary>Whether anything is worth saying at all.</summary>
    private bool Speaking => Wanted && !InFront;

    /// <summary>
    /// Whoever this is, as the roster names them — and "Someone" when it cannot, which happens
    /// and is nobody's fault: a member who joined while this device slept is a name it has not
    /// read yet.
    /// </summary>
    private string NameOf(long userId) =>
        chats.Member(userId)?.DisplayName is { Length: > 0 } name ? name : say.Get("Someone");
}
