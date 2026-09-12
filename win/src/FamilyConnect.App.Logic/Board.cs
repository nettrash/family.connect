using FamilyConnect.Core.Board;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic;

/// <summary>
/// One note as the wall draws it: the note, its look resolved, where it goes and how big it is.
/// </summary>
public sealed record Sticker(
    NoteDto Note,
    NoteKind Kind,
    NoteSize Size,
    NoteFont Font,
    string Color,
    double X,
    double Y,
    double Width,
    double Height,
    int TiltDegrees,
    /// <summary>Something NEW TO READ — not merely something that changed.</summary>
    bool Unread,
    /// <summary>Written by this reader, which is who may edit and delete it.</summary>
    bool Mine)
{
    public bool IsEvent => Kind == NoteKind.Event;

    public bool IsPhoto => Kind == NoteKind.Photo;

    public bool IsTasks => Kind == NoteKind.Tasks;

    /// <summary>The backdrop or the photograph — one attachment either way.</summary>
    public AttachmentDto? Picture => Note.Attachment;
}

/// <summary>
/// The family's wall: the stickers on it, the badge over it, and the writes that change it.
/// </summary>
/// <remarks>
/// <para>
/// <b>THE ARITHMETIC IS NOT THIS LAYER'S.</b> Card sizes, the wall's own size, the clamp, the
/// fraction and the tilt all come from <see cref="BoardWall"/> — the shared module the differential
/// oracle pins to the web client's Rust — so a Windows wall lays out exactly where the other three
/// lay out. What lives here is when to ask and what to do with the answer.
/// </para>
/// <para>
/// <b>A POSITION IS A FRACTION OF THE WALL, read back from where the card actually is.</b> A
/// fraction derived from a raw drag delta stores a position the note was never at, and a note
/// stored at 0.98 must hug the edge rather than hang off it — which is why the clamp is applied
/// before the fraction is taken, and again when it is drawn.
/// </para>
/// <para>
/// <b>THE BADGE COUNTS WHAT IS NEW TO READ.</b> A note somebody dragged, recoloured, resized or
/// answered is not news; <c>content_seq</c> is, and the marks only move when the wall has actually
/// been on screen.
/// </para>
/// <para>
/// <b>A REDRAWN BACKDROP IS A NEW ATTACHMENT.</b> The answer carries the whole note with a new
/// attachment id, so a client that caches a picture by NOTE shows the old one for ever — which is
/// exactly how this was found on three platforms (issue #70). Callers redraw by attachment id.
/// </para>
/// </remarks>
public sealed class BoardModel(BoardStore board, ChatStore chats, ApiClient api)
{
    /// <summary>The wall's own size for a viewport: taller than the screen, so it can be pinned.</summary>
    public static (double Width, double Height) WallSize(double visibleWidth, double visibleHeight) =>
        BoardWall.Size(visibleWidth, visibleHeight);

    /// <summary>How many notes have something new to read.</summary>
    public int Badge => board.Unread();

    /// <summary>The wall has been on screen: everything on it has been shown.</summary>
    public void Shown() => board.MarkShown();

    /// <summary>
    /// The stickers, newest first as the store answers them, each placed and sized for a wall of
    /// this size.
    /// </summary>
    public IReadOnlyList<Sticker> Stickers(double visibleWidth, double visibleHeight)
    {
        var wall = WallSize(visibleWidth, visibleHeight);
        var compact = BoardWall.IsCompact(wall.Width);
        var marks = board.Marks;
        var me = chats.Reader;
        var stickers = new List<Sticker>();
        foreach (var note in board.Notes())
        {
            var kind = Notes.KindFrom(note.Kind);
            var size = Notes.SizeFrom(note.Size);
            var card = BoardWall.Card(size, compact);
            // A BARE PHOTO'S CARD IS THE PICTURE, fitted into the card it would otherwise have
            // had — the rule issue #71 settled: a photo is drawn WHOLE (both dimensions), and a
            // card with nothing else on it hugs it (docs/protocol.md, "A photo is drawn whole").
            if (kind == NoteKind.Photo
                && string.IsNullOrEmpty(note.Text)
                && note.Attachment is { } picture)
            {
                card = BoardPicture.Fitted(
                    card.Width, card.Height, picture.Width, picture.Height);
            }
            var (x, y) = BoardWall.Origin((note.X, note.Y), card, wall);
            stickers.Add(new Sticker(
                note, kind, size, Notes.FontFrom(note.Font), note.Color ?? "yellow",
                x, y, card.Width, card.Height,
                BoardWall.TiltDegrees(note.Id),
                BoardBadge.IsUnread(note.Id, note.ContentSeq, marks),
                note.AuthorId == me));
        }
        return stickers;
    }

    /// <summary>A new note. The answer is the note the server made of it.</summary>
    public Task<ApiError?> CreateAsync(NoteRequest note, CancellationToken ct = default) =>
        Applied(() => api.CreateNote(note, ct));

    /// <summary>
    /// A change. ONLY WHAT CHANGED is sent — which fields are present decides what changes, and
    /// a patch that sent everything would overwrite what somebody else edited in between.
    /// </summary>
    public Task<ApiError?> PatchAsync(long noteId, NotePatch patch, CancellationToken ct = default) =>
        Applied(() => api.PatchNote(noteId, patch, ct));

    /// <summary>
    /// Where a note was DROPPED, as a fraction of the wall. The corner is where the card actually
    /// is, clamped inside the wall before the fraction is taken.
    /// </summary>
    public Task<ApiError?> MoveAsync(
        long noteId,
        (double X, double Y) corner,
        (double Width, double Height) card,
        (double Width, double Height) wall,
        CancellationToken ct = default)
    {
        var held = BoardWall.Clamp(corner.X, corner.Y, card, wall);
        var (x, y) = BoardWall.FractionOf(held, wall);
        return PatchAsync(noteId, new NotePatch(X: x, Y: y), ct);
    }

    /// <summary>Take it down. A delete is a tombstone in the feed, not a silence.</summary>
    public async Task<ApiError?> DeleteAsync(long noteId, CancellationToken ct = default)
    {
        var answer = await api.DeleteNote(noteId, ct).ConfigureAwait(false);
        if (!answer.Ok)
        {
            return answer.Error;
        }
        // The frame and the catch-up both carry the tombstone; applying it here as well is what
        // makes the note leave the wall as the tap lands rather than a round trip later.
        board.Apply(board.Note(noteId) is { } held
            ? held with { Deleted = true, BoardSeq = held.BoardSeq + 1 }
            : new NoteDto(noteId, 0, Deleted: true, BoardSeq: long.MaxValue));
        return null;
    }

    /// <summary>
    /// Going, or not. An idempotent STATE-SET rather than a toggle, and any member may send it —
    /// the note's author has no special standing over who is coming.
    /// </summary>
    public Task<ApiError?> AnswerAsync(
        long noteId, RsvpAnswer answer, CancellationToken ct = default) =>
        Applied(() => api.Answer(noteId, Notes.NameOf(answer), ct));

    /// <summary>Unsay it: back to no answer at all, which is not the same as "not going".</summary>
    public Task<ApiError?> RetractAsync(long noteId, CancellationToken ct = default) =>
        Applied(() => api.RetractAnswer(noteId, ct));

    /// <summary>One line of a list, ticked or unticked. The server records WHO ticked it.</summary>
    public Task<ApiError?> TickAsync(
        long noteId, long itemId, bool done, CancellationToken ct = default) =>
        Applied(() => api.TickTask(noteId, itemId, done, ct));

    /// <summary>
    /// Ask the assistant to draw this event's backdrop. Answers the NEW attachment — a redraw is
    /// a new attachment id, and a caller that caches by note rather than by attachment shows the
    /// old picture for ever.
    /// </summary>
    public async Task<(AttachmentDto? Drawn, ApiError? Error)> DrawBackdropAsync(
        long noteId, CancellationToken ct = default)
    {
        var answer = await api.DrawBackdrop(noteId, ct).ConfigureAwait(false);
        if (!answer.Ok || answer.Value is null)
        {
            return (null, answer.Error ?? ApiError.Transport("no answer"));
        }
        board.Apply(answer.Value.Note);
        return (answer.Value.Note.Attachment, null);
    }

    private async Task<ApiError?> Applied(Func<Task<ApiResult<NoteResponse>>> write)
    {
        var answer = await write().ConfigureAwait(false);
        if (!answer.Ok || answer.Value is null)
        {
            return answer.Error ?? ApiError.Transport("no answer");
        }
        // Applied under the board_seq guard, exactly as a frame is: this answer and the frame the
        // same write raised can arrive in either order.
        board.Apply(answer.Value.Note);
        return null;
    }
}
