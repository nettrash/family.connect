using FamilyConnect.Core.Board;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.Core.Tests.Store;

/// <summary>
/// The wall as this device holds it: the two protocol rules that decide what a cache may keep
/// (docs/protocol.md, "Board").
/// </summary>
public class BoardStoreTests : IDisposable
{
    private readonly Database database = Database.OpenInMemory();

    public void Dispose() => database.Dispose();

    private BoardStore Store() => new(database);

    private static NoteDto Note(
        long id,
        long seq,
        string text = "Milk",
        string kind = "text",
        long? contentSeq = null,
        bool deleted = false) =>
        new(
            Id: id,
            AuthorId: 7,
            Kind: kind,
            Text: text,
            Color: "yellow",
            Size: "medium",
            Font: "plain",
            X: 0.25,
            Y: 0.5,
            CreatedAt: "2026-09-12T08:00:00Z",
            UpdatedAt: "2026-09-12T08:00:00Z",
            BoardSeq: seq,
            ContentSeq: contentSeq,
            Deleted: deleted);

    [Fact]
    public void ANoteGoesInAndComesBackTheSame()
    {
        var store = Store();
        store.Apply(Note(12, 88, contentSeq: 84));
        var held = store.Note(12);
        Assert.NotNull(held);
        Assert.Equal("Milk", held!.Text);
        Assert.Equal(0.25, held.X);
        Assert.Equal(88, held.BoardSeq);
        Assert.Equal(84, held.ContentSeq);
        Assert.Equal("2026-09-12T08:00:00.000Z", held.CreatedAt);
        Assert.Equal(88, store.Cursor);
    }

    /// <summary>
    /// A FULL READ REPLACES what is held. A note somebody took down while this device was away
    /// would otherwise stay on the wall for ever.
    /// </summary>
    [Fact]
    public void AWholeBoardReadIsTheWall()
    {
        var store = Store();
        store.Apply(Note(1, 10));
        store.Apply(Note(2, 11));
        store.Replace([Note(2, 11), Note(3, 12)], maxBoardSeq: 12);
        Assert.Equal([3L, 2L], store.Notes().Select(note => note.Id));
        Assert.Null(store.Note(1));
        Assert.Equal(12, store.Cursor);
    }

    /// <summary>The changes feed carries TOMBSTONES, and a tombstone is a delete.</summary>
    [Fact]
    public void ATombstoneTakesTheNoteDown()
    {
        var store = Store();
        store.Apply(Note(12, 88));
        Assert.True(store.Apply(Note(12, 90, deleted: true)));
        Assert.Null(store.Note(12));
        Assert.Empty(store.Notes());
        // And the cursor still moves: the delete is a change like any other.
        Assert.Equal(90, store.Cursor);
        // Idempotent — a tombstone this device has already applied changes nothing.
        Assert.False(store.Apply(Note(12, 91, deleted: true)));
    }

    /// <summary>
    /// SEQS COMMIT OUT OF ORDER, so a slower answer must never overwrite a newer one it crossed
    /// on the wire.
    /// </summary>
    [Fact]
    public void AnOlderAnswerIsDropped()
    {
        var store = Store();
        store.Apply(Note(12, 90, text: "the newer one"));
        Assert.False(store.Apply(Note(12, 88, text: "the older one")));
        Assert.Equal("the newer one", store.Note(12)!.Text);
        // The same seq is not newer either: a re-read of what is held changes nothing.
        Assert.False(store.Apply(Note(12, 90, text: "again")));
        Assert.Equal("the newer one", store.Note(12)!.Text);
    }

    /// <summary>
    /// A TOMBSTONE IS THE LAST WORD. A delete is the last thing that happens to a note, but seqs
    /// commit out of order and a history page carries the pre-delete copy — so a note that has
    /// been taken down must not come back when an older answer crosses the tombstone on the wire.
    /// </summary>
    [Fact]
    public void ANoteTakenDownStaysDownHoweverLateAnOlderCopyArrives()
    {
        var store = Store();
        store.Apply([Note(12, 11), Note(13, 12)]);

        Assert.True(store.Apply(Note(12, 13, deleted: true)));
        Assert.Equal([13L], store.Notes().Select(note => note.Id));

        // The copy that was already travelling when the delete happened — a catch-up page, or a
        // frame that crossed it.
        Assert.False(store.Apply(Note(12, 99, text: "back from the dead")));
        Assert.Equal([13L], store.Notes().Select(note => note.Id));

        // And a full read that still lists it puts nothing back either.
        store.Replace([Note(12, 11), Note(13, 12)], 12);
        Assert.Equal([13L], store.Notes().Select(note => note.Id));
    }

    /// <summary>
    /// A FULL READ REPLACES THE WALL — except what is NEWER than the read's own mark. A frame
    /// that landed while the read was in flight is not on that answer, and wiping it would take
    /// a note off the wall seconds after somebody pinned it.
    /// </summary>
    [Fact]
    public void AFullReadKeepsWhatArrivedAfterTheReadWasTaken()
    {
        var store = Store();
        // A frame, ahead of the read that is about to land.
        store.Apply(Note(20, 30, text: "just pinned"), SeqRoute.LiveFrame);

        store.Replace([Note(12, 11), Note(13, 12)], 12);

        Assert.Equal([20L, 13L, 12L], store.Notes().Select(note => note.Id));
        Assert.Equal("just pinned", store.Note(20)!.Text);
        // The cursor is the read's mark: the server read it BEFORE the notes, so it is never
        // past a change they missed.
        Assert.Equal(12, store.Cursor);
    }

    /// <summary>
    /// THE CURSOR IS NOT MOVED BY EVERY WRITE. A live frame and a page of the feed move it; the
    /// answer to this client's own write is evidence about one note and says nothing about what
    /// else has happened, so a cursor moved by it steps the feed past somebody else's change.
    /// </summary>
    [Fact]
    public void OnlyAFrameOrAPageMovesTheBoardsCursor()
    {
        var store = Store();

        store.Apply(Note(12, 11), SeqRoute.Evidence);
        Assert.Equal("Milk", store.Note(12)!.Text);
        Assert.Equal(0, store.Cursor);

        // A FRAME BEFORE THIS DEVICE HAS EVER READ THE BOARD moves nothing either: the cursor
        // being 0 is what tells the resync to read the whole wall, and a frame that jumped that
        // queue would leave it above changes nobody had read.
        store.Apply(Note(13, 12), SeqRoute.LiveFrame);
        Assert.Equal("Milk", store.Note(13)!.Text);
        Assert.Equal(0, store.Cursor);

        // The whole wall, read once…
        store.Replace([Note(12, 11), Note(13, 12)], 12);
        Assert.Equal(12, store.Cursor);

        // …and from here a frame does move it.
        store.Apply(Note(14, 20), SeqRoute.LiveFrame);
        Assert.Equal(20, store.Cursor);

        store.Apply([Note(15, 21)]);
        Assert.Equal(21, store.Cursor);

        // And a page applied as evidence — the answer to our own write, which never comes as a
        // page in practice — moves nothing either.
        store.Apply([Note(16, 22)], SeqRoute.Evidence);
        Assert.Equal(21, store.Cursor);
    }

    /// <summary>
    /// TWO FULL READS CAN LAND IN EITHER ORDER, and an older one landing second must change
    /// nothing: it would drop every note written between the two and set the cursor back.
    /// </summary>
    [Fact]
    public void AFullReadOlderThanOneAlreadyAppliedIsIgnored()
    {
        var store = Store();
        store.Replace([Note(12, 11), Note(13, 12)], 12);

        // The slower read, taken before the faster one and arriving after it.
        store.Replace([Note(12, 11)], 10);

        Assert.Equal([13L, 12L], store.Notes().Select(note => note.Id));
        Assert.Equal(12, store.Cursor);
    }

    [Fact]
    public void APageOfChangesIsAppliedInOrderAndMovesTheCursorOnce()
    {
        var store = Store();
        var changed = store.Apply([
            Note(1, 10),
            Note(2, 11),
            Note(1, 12, text: "rewritten"),
            Note(3, 13, deleted: true),
        ]);
        // Two notes written, one rewritten, and a tombstone for a note this device never had.
        Assert.Equal(3, changed);
        Assert.Equal("rewritten", store.Note(1)!.Text);
        Assert.Equal(13, store.Cursor);
    }

    /// <summary>
    /// `[]` and ABSENT are different answers all the way down: an event nobody has answered sends
    /// `[]`, a list with nothing on it sends `[]`, and every other kind sends nothing at all.
    /// </summary>
    [Fact]
    public void AnEmptyListIsNotAMissingOne()
    {
        var store = Store();
        store.Apply(Note(1, 10, kind: "tasks") with { Items = [] });
        store.Apply(Note(2, 11, kind: "event") with { Rsvps = [], StartsAt = "2026-12-24T17:00:00Z" });
        store.Apply(Note(3, 12));
        Assert.Empty(store.Note(1)!.Items!);
        Assert.NotNull(store.Note(1)!.Items);
        Assert.NotNull(store.Note(2)!.Rsvps);
        Assert.Empty(store.Note(2)!.Rsvps!);
        // A text note is not a list and not an event: no items, no answers.
        Assert.Null(store.Note(3)!.Items);
        Assert.Null(store.Note(3)!.Rsvps);
        Assert.Empty(store.Note(3)!.TaskList);
    }

    [Fact]
    public void AnEventKeepsItsWhenWhereWhoAndItsBackdrop()
    {
        var store = Store();
        store.Apply(Note(13, 20, text: "Gran's birthday", kind: "event") with
        {
            StartsAt = "2026-12-24T17:00:00Z",
            EndsAt = "2026-12-24T19:30:00Z",
            Place = "Gran's",
            Rsvps = [new RsvpDto(7, "going"), new RsvpDto(9, "maybe")],
            Attachment = new AttachmentDto(900, "photo", "image/png", Width: 1024, Height: 1024),
        });
        var held = store.Note(13)!;
        Assert.Equal("2026-12-24T17:00:00.000Z", held.StartsAt);
        Assert.Equal("2026-12-24T19:30:00.000Z", held.EndsAt);
        Assert.Equal("Gran's", held.Place);
        Assert.Equal(1, held.Count("going"));
        Assert.Equal("maybe", held.MyAnswer(9));
        // The BACKDROP, with the flag that decides which bytes a card asks for.
        Assert.Equal(900, held.Attachment!.Id);
        Assert.False(held.Attachment.HasPreview);
        Assert.Equal(1.0, held.Attachment.AspectRatio);
    }

    /// <summary>
    /// An unknown kind, colour, size or face is kept AS SENT: a fourth size from a newer server
    /// draws as medium and must not be written back as medium by an edit.
    /// </summary>
    [Fact]
    public void AnUnknownNameIsKeptRatherThanNormalised()
    {
        var store = Store();
        store.Apply(Note(1, 10) with { Size = "huge", Color = "chartreuse", Font = "comic", Kind = "hologram" });
        var held = store.Note(1)!;
        Assert.Equal("huge", held.Size);
        Assert.Equal("chartreuse", held.Color);
        Assert.Equal("comic", held.Font);
        Assert.Equal("hologram", held.Kind);
        // And the drawing rules still resolve them to the fallbacks.
        Assert.Equal(NoteSize.Medium, Notes.SizeFrom(held.Size));
        Assert.Equal("#fff2b3", Notes.ColorHex(held.Color));
        Assert.Equal(NoteKind.Text, Notes.KindFrom(held.Kind));
        // An untouched picker sends nothing, so the name survives an edit.
        Assert.Null(Notes.PatchSize(NoteSize.Medium, held.Size));
    }

    /// <summary>
    /// The badge counts what a note SAYS, and the marks are what this DEVICE has shown — moved
    /// only when the board has been on screen, and never backwards.
    /// </summary>
    [Fact]
    public void TheBadgeCountsWhatIsNewToReadAndTheMarksNeverGoBack()
    {
        var store = Store();
        store.Apply(Note(1, 10, contentSeq: 10));
        store.Apply(Note(2, 11, contentSeq: 11));
        Assert.Equal(2, store.Unread());
        store.MarkShown();
        Assert.Equal(0, store.Unread());
        Assert.Equal(new BoardMarks(2, 11), store.Marks);

        // A move is not news: a new board_seq with the same content_seq raises nothing.
        store.Apply(Note(1, 12, contentSeq: 10) with { X = 0.9 });
        Assert.Equal(0, store.Unread());
        // A rewrite is.
        store.Apply(Note(1, 13, text: "Milk and eggs", contentSeq: 13));
        Assert.Equal(1, store.Unread());

        // And the newest note going away must not bring a cleared badge back.
        store.MarkShown();
        store.Apply(Note(1, 14, deleted: true));
        Assert.Equal(0, store.Unread());
        Assert.Equal(13, store.Marks.ContentSeq);

        // TWO WINDOWS OF ONE ACCOUNT both write these, and neither may walk the other's back:
        // marks that arrive BEHIND what is held change nothing. `MarkShown` cannot show this on
        // its own — it hands in the maximum it just computed — so the stale set is written
        // directly, which is what a second window's older snapshot looks like.
        store.Mark(new BoardMarks(1, 5));
        // Both fields stand: the note id is still 2 (a note that has since been deleted, which is
        // exactly why the mark may not go back) and the content seq is still 13.
        Assert.Equal(new BoardMarks(2, 13), store.Marks);
        Assert.Equal(0, store.Unread());
    }

    [Fact]
    public void ANoteFromAServerWithNoContentSeqIsJudgedByItsId()
    {
        var store = Store();
        store.Apply(Note(5, 10, contentSeq: null));
        // 0 is this cache's spelling of "the server never said", which reads back as absent.
        Assert.Null(store.Note(5)!.ContentSeq);
        Assert.Equal(1, store.Unread());
        store.Mark(new BoardMarks(5, 0));
        Assert.Equal(0, store.Unread());
    }
}
