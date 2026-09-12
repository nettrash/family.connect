namespace FamilyConnect.Core.Board;

/// <summary>
/// What the board's badge counts (docs/protocol.md, "Board"): notes with something NEW TO READ,
/// which is not the same as notes that have changed.
/// </summary>
/// <remarks>
/// <para>
/// A note carries two sequences: <c>board_seq</c> moves on every change, and <c>content_seq</c>
/// only when what the note SAYS changes. The badge counts the second — a note somebody dragged
/// across the wall, recoloured, resized or answered is not a note with news in it, and a badge
/// claiming otherwise is a lie the family learns to ignore.
/// </para>
/// <para>
/// The marks are what this DEVICE has shown its reader, which is not the sync cursor. They move
/// only when the board has actually been on screen.
/// </para>
/// <para>
/// Web counterpart: <c>fc_text::board::Marks</c> / <c>is_unread</c> /
/// <c>marks_after_showing</c> / <c>later</c>. Apple: <c>BoardBadge</c>.
/// </para>
/// </remarks>
public readonly record struct BoardMarks(long NoteId, long ContentSeq)
{
    public static readonly BoardMarks Zero = new(0, 0);

    /// <summary>
    /// The later of two sets of marks, field by field — two windows of one account both write,
    /// and neither may walk the other's back.
    /// </summary>
    public static BoardMarks Later(BoardMarks a, BoardMarks b) =>
        new(Math.Max(a.NoteId, b.NoteId), Math.Max(a.ContentSeq, b.ContentSeq));
}

public static class BoardBadge
{
    /// <summary>
    /// One note's verdict: judged by its content seq when it has one, and by its id when it does
    /// not — the rule for a note from a server that predates the field.
    /// </summary>
    public static bool IsUnread(long noteId, long? contentSeq, BoardMarks marks) =>
        contentSeq is > 0 ? contentSeq > marks.ContentSeq : noteId > marks.NoteId;

    /// <summary>
    /// The marks after the board has been on screen: everything on it has been shown. Monotonic in
    /// both fields — a board that has just lost its newest note to somebody's delete must not
    /// bring a cleared badge back.
    /// </summary>
    public static BoardMarks AfterShowing(
        IEnumerable<(long NoteId, long? ContentSeq)> notes, BoardMarks marks)
    {
        foreach (var (noteId, contentSeq) in notes)
        {
            marks = new BoardMarks(
                Math.Max(marks.NoteId, noteId),
                Math.Max(marks.ContentSeq, contentSeq ?? 0));
        }
        return marks;
    }

    /// <summary>How many notes on this board have something new to read.</summary>
    public static int Count(IEnumerable<(long NoteId, long? ContentSeq)> notes, BoardMarks marks) =>
        notes.Count(note => IsUnread(note.NoteId, note.ContentSeq, marks));
}
