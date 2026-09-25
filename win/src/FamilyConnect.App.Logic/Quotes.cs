using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic;

/// <summary>Which level of a quote something is about — the answered message, or its own quote.</summary>
public enum QuoteLevel
{
    /// <summary>The message this reply answers.</summary>
    Reply,

    /// <summary>What THAT message was itself answering: one level under, and no deeper.</summary>
    Parent,
}

/// <summary>
/// One level of the quote over a reply: who wrote it, what they said, and which message it is —
/// the id being how a click on it finds the message (docs/protocol.md, "Replies").
/// </summary>
public sealed record QuoteLine(long MessageId, string Name, string Excerpt, bool Hidden);

/// <summary>The quote over a reply, as drawn: the message it answers, and one level under that.</summary>
public sealed record Quote(QuoteLine Reply, QuoteLine? Parent);

/// <summary>What a click on a quote does — which is not one thing.</summary>
public enum QuoteClick
{
    /// <summary>Show the masked level naming the answered message.</summary>
    RevealReply,

    /// <summary>Show the masked level under that one.</summary>
    RevealParent,

    /// <summary>Nothing left masked: go to the message this reply answers.</summary>
    GoToMessage,
}

/// <summary>
/// The two places a reply names what it answers: the quote over a bubble, and the banner over the
/// composer while a reply is being written.
/// </summary>
/// <remarks>
/// <para>
/// <b>BOTH LEVELS, BECAUSE THE SERVER SENDS BOTH.</b> A quote of a reply carries that reply's own
/// quote — one level, structurally capped — so answering an answer shows both halves of the
/// exchange rather than a snippet with no idea what it was responding to (docs/protocol.md,
/// "Replies"). The excerpts are the server's, cut the way <see cref="Excerpt.Cut"/> cuts.
/// </para>
/// <para>
/// <b>A BLOCKED MEMBER'S WORDS ARE NOT QUOTED EITHER — AT EITHER LEVEL.</b> A level whose sender
/// is blocked draws as "Replying to a hidden message" — the same rule as their own bubble, one
/// step removed, because a quote is their words in somebody else's bubble — and reveals under the
/// same one-tap rule, each level on its own (docs/protocol.md, "Blocking a member"). The quoting
/// message above it is untouched and stays readable.
/// </para>
/// </remarks>
public static class Quotes
{
    /// <summary>The quote over a bubble, or null when it answers nothing.</summary>
    /// <param name="revealed">
    /// Which hidden levels this reader has asked to see. Held by the surface, not by the bubble,
    /// so a reveal outlives the next redraw.
    /// </param>
    public static Quote? Of(
        MessageDto message,
        ChatStore chats,
        IStringCatalog say,
        Func<QuoteLevel, bool>? revealed = null)
    {
        if (message.ReplyTo is not { } quoted)
        {
            return null;
        }
        var shows = revealed ?? (_ => false);
        return new Quote(
            Line(quoted.MessageId, quoted.SenderId, quoted.Excerpt, QuoteLevel.Reply, shows, chats, say),
            quoted.Parent is { } parent
                ? Line(parent.MessageId, parent.SenderId, parent.Excerpt, QuoteLevel.Parent, shows, chats, say)
                : null);
    }

    /// <summary>
    /// What a click on the quote asks for. A MASKED LEVEL IS SHOWN BEFORE ANYTHING MOVES —
    /// outermost first, then the one under it — which is the one-tap reveal the blocking rule
    /// gives every surface that draws a quote (docs/protocol.md, "Blocking a member"); the same
    /// staged click the apps use. Only a quote with nothing left masked goes to the message, so a
    /// click never jumps to a bubble whose words the reader has chosen not to see.
    /// </summary>
    public static QuoteClick ClickOn(Quote quote) =>
        quote switch
        {
            { Reply.Hidden: true } => QuoteClick.RevealReply,
            { Parent.Hidden: true } => QuoteClick.RevealParent,
            _ => QuoteClick.GoToMessage,
        };

    /// <summary>The banner over the composer while replying to <paramref name="message"/>.</summary>
    public static string Banner(MessageDto message, ChatStore chats, IStringCatalog say) =>
        Hides(message.SenderId, chats)
            ? say.Get("Replying to a hidden message")
            : say.Format("Replying to %@", NameOf(message.SenderId, chats, say));

    private static QuoteLine Line(
        long messageId,
        long senderId,
        string excerpt,
        QuoteLevel level,
        Func<QuoteLevel, bool> revealed,
        ChatStore chats,
        IStringCatalog say) =>
        Hides(senderId, chats) && !revealed(level)
            ? new QuoteLine(
                messageId,
                string.Empty,
                level == QuoteLevel.Reply
                    ? say.Get("Replying to a hidden message")
                    : say.Get("which replied to a hidden message"),
                Hidden: true)
            : new QuoteLine(messageId, NameOf(senderId, chats, say), excerpt, Hidden: false);

    private static bool Hides(long senderId, ChatStore chats) =>
        senderId != chats.Reader && chats.IsBlocked(senderId);

    private static string NameOf(long userId, ChatStore chats, IStringCatalog say)
    {
        if (userId == chats.Reader)
        {
            return say.Get("You");
        }
        return chats.Member(userId) switch
        {
            { Deleted: true } => say.Get("Deleted account"),
            { DisplayName: { Length: > 0 } name } => name,
            _ => say.Get("Someone"),
        };
    }
}
