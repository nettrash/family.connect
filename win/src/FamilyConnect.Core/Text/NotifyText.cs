namespace FamilyConnect.Core;

/// <summary>
/// What a notification says, and what the window's own title carries — ported from
/// <c>fc_text::notify</c> and pinned to it by the oracle.
/// </summary>
/// <remarks>
/// The TITLE is the push's title, because a family that uses a phone and a desktop should read the
/// same words in both places. The BODY is the one a server with <c>include_message_body = false</c>
/// would send: a desktop client cannot know whether the operator wanted family text on a lock
/// screen, so it never puts it there.
/// </remarks>
public static class NotifyText
{
    /// <summary>The body of a message notification — never the message.</summary>
    public static string NewMessage(IStringCatalog? words = null) =>
        (words ?? EnglishCatalog.Instance).Get("New message");

    /// <summary>The body of a board-note notification.</summary>
    public static string NewNote(IStringCatalog? words = null) =>
        (words ?? EnglishCatalog.Instance).Get("New note");

    /// <summary>
    /// Who a notification is from. A direct chat is the sender alone; the family chat names the
    /// family FIRST, because a notification centre shows a list and "Anna" alone does not say
    /// where she said it. A message that names the reader says so in the title — the same
    /// notification, a different title, never a second one.
    /// </summary>
    public static string Title(
        string? family, string sender, bool mentioned, IStringCatalog? words = null)
    {
        var say = words ?? EnglishCatalog.Instance;
        return family switch
        {
            null => sender,
            _ when mentioned => say.Format("%@ — %@ mentioned you", family, sender),
            _ => say.Format("%@ — %@", family, sender),
        };
    }

    /// <summary>
    /// The count in the window's title: "(3) Family Connect", and the bare name at zero. A
    /// hundred and more is "99+" — the exact number stops being the point.
    /// </summary>
    public static string WindowTitle(string name, long unread) => unread switch
    {
        <= 0 => name,
        > 99 => $"(99+) {name}",
        _ => $"({unread}) {name}",
    };
}
