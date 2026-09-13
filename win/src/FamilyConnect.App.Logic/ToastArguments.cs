using System.Globalization;
using FamilyConnect.Core.Board;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic;

/// <summary>What a clicked notification opens: a chat, or — with no chat — the wall.</summary>
public sealed record ToastTarget(long? ChatId)
{
    public bool IsBoard => ChatId is null;
}

/// <summary>
/// What a notification carries back to the app when it is clicked, and the one question the wall's
/// notifications ask before they are raised.
/// </summary>
/// <remarks>
/// <para>
/// <b>AN ARGUMENT IS UNTRUSTED INPUT.</b> It comes back from the operating system, and a launch from a
/// notification the app never raised — or from an older build that wrote something else — must open
/// nothing rather than throw or open the wrong chat. Anything that is not a positive id is ignored.
/// </para>
/// <para>
/// <b>A NOTE IS NEWS BY THE BADGE'S OWN RULE</b> (<see cref="BoardBadge.IsUnread"/>): the badge and the
/// notification answer the same question and must not disagree. A tombstone is never news.
/// </para>
/// </remarks>
public static class ToastArguments
{
    public const string ChatKey = "chat";
    public const string BoardKey = "board";

    public static IReadOnlyList<KeyValuePair<string, string>> For(Toast toast) =>
        toast.ChatId is { } chatId
            ? [new(ChatKey, chatId.ToString(CultureInfo.InvariantCulture))]
            : [new(BoardKey, "1")];

    public static ToastTarget? Parse(IReadOnlyDictionary<string, string> arguments)
    {
        if (arguments.TryGetValue(ChatKey, out var text)
            && long.TryParse(text, NumberStyles.None, CultureInfo.InvariantCulture, out var chatId)
            && chatId > 0)
        {
            return new ToastTarget(chatId);
        }
        return arguments.ContainsKey(BoardKey) ? new ToastTarget(null) : null;
    }

    public static bool IsNews(NoteDto note, BoardMarks marks) =>
        !note.Deleted && BoardBadge.IsUnread(note.Id, note.ContentSeq, marks);
}
