using FamilyConnect.Core;
using FamilyConnect.Core.Board;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic;

/// <summary>
/// What a sticker on the wall says and hides — the web client's board pane (<c>web/src/views/board.rs</c>),
/// which is the Mac's.
/// </summary>
/// <remarks>
/// <para>
/// <b>A NOTE A BLOCK HIDES stays on the wall</b> — its slot is part of the family's shared layout — but
/// says nothing of what it holds, not even who wrote it, until the reader peeks.
/// </para>
/// <para>
/// <b>A BARE PHOTO IS THE PICTURE</b>: no paper under it, so no author line and no words.
/// </para>
/// </remarks>
public static class WallText
{
    public static string CaptionOf(NoteDto note) => (note.Text ?? string.Empty).Trim();

    /// <summary>Hidden while its author is somebody this reader blocked, until they peek — never their own.</summary>
    public static bool IsHidden(NoteDto note, long me, Func<long, bool> blocked, IReadOnlySet<long> revealed) =>
        note.AuthorId != me && blocked(note.AuthorId) && !revealed.Contains(note.Id);

    /// <summary>Who wrote it, as the sticker signs it: "You", the name the roster holds, or "Someone".</summary>
    public static string AuthorName(NoteDto note, long me, Func<long, string?> nameOf, IStringCatalog say) =>
        note.AuthorId == me ? say.Get("You")
        : nameOf(note.AuthorId) is { Length: > 0 } name ? name
        : say.Get("Someone");

    public static bool ShowsText(NoteKind kind, string caption, bool hidden) =>
        hidden || kind != NoteKind.Photo || caption.Length > 0;

    public static bool ShowsAuthor(NoteKind kind, string caption, bool hidden) =>
        !(hidden || (kind == NoteKind.Photo && caption.Length == 0));

    /// <summary>
    /// What the card holds, in words — its accessibility label stands in for the content, so an event's
    /// when, where and who is coming have to be in it too.
    /// </summary>
    public static string What(NoteKind kind, string caption, string? when, string? place, string? going, IStringCatalog say)
    {
        var what = kind == NoteKind.Photo && caption.Length == 0 ? say.Get("a photo") : caption;
        if (kind == NoteKind.Event && when is { Length: > 0 })
        {
            what = $"{what}, {when}";
            if (place is { Length: > 0 })
            {
                what = $"{what}, {place}";
            }
            if (going is { Length: > 0 })
            {
                what = $"{what}, {going}";
            }
        }
        return what;
    }

    public static string Label(bool hidden, bool mine, string author, string what, IStringCatalog say) =>
        hidden ? say.Get("Hidden note from a blocked member")
        : mine ? say.Format("Your note: %@", what)
        : say.Format("Note from %@: %@", author, what);

    /// <summary>How many lines of a list the sticker had to leave off.</summary>
    public static string MoreLine(int left, IStringCatalog say) => say.Plural("+%lld more", left, left);

    /// <summary>
    /// Where each sticker stacks: the most recently changed highest. Drawn in a STABLE order (by id) with
    /// the recency as a z-index, so a note somebody else touched does not reshuffle the wall under a reader.
    /// </summary>
    public static IReadOnlyDictionary<long, int> Layers(IReadOnlyList<Sticker> newestFirst)
    {
        var layers = new Dictionary<long, int>();
        for (var at = 0; at < newestFirst.Count; at++)
        {
            layers[newestFirst[at].Note.Id] = newestFirst.Count - at;
        }
        return layers;
    }
}
