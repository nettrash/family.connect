using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic;

/// <summary>The quote above a reply, as drawn: who, and what they said — or that it is hidden.</summary>
public sealed record Quote(string Name, string Excerpt, bool Hidden);

/// <summary>
/// The two places a reply names what it answers: the quote over a bubble, and the banner over the
/// composer while a reply is being written.
/// </summary>
/// <remarks>
/// <b>A BLOCKED MEMBER'S WORDS ARE NOT QUOTED EITHER.</b> A reply to them is drawn as "Replying to a
/// hidden message" — the same rule as their own bubble, one step removed, because a quote is their
/// words in somebody else's bubble (docs/protocol.md, "Blocking a member"). The excerpt itself is
/// the server's (<see cref="ReplyToDto.Excerpt"/>), cut the way <see cref="Excerpt.Cut"/> cuts.
/// </remarks>
public static class Quotes
{
    /// <summary>The quote over a bubble, or null when it answers nothing.</summary>
    public static Quote? Of(MessageDto message, ChatStore chats, IStringCatalog say)
    {
        if (message.ReplyTo is not { } quoted)
        {
            return null;
        }
        return Hides(quoted.SenderId, chats)
            ? new Quote(string.Empty, say.Get("Replying to a hidden message"), Hidden: true)
            : new Quote(NameOf(quoted.SenderId, chats, say), quoted.Excerpt, Hidden: false);
    }

    /// <summary>The banner over the composer while replying to <paramref name="message"/>.</summary>
    public static string Banner(MessageDto message, ChatStore chats, IStringCatalog say) =>
        Hides(message.SenderId, chats)
            ? say.Get("Replying to a hidden message")
            : say.Format("Replying to %@", NameOf(message.SenderId, chats, say));

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
