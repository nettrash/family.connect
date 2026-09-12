using System.Net;
using FamilyConnect.App.Logic;
using FamilyConnect.Core.Board;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// The family's wall: what is on it, where, the badge over it, and the writes that change it.
/// </summary>
public class BoardModelTests : IDisposable
{
    private const long Me = 7;

    private readonly Database cache = Database.OpenInMemory();
    private readonly BoardStore board;
    private readonly ChatStore chats;

    public BoardModelTests()
    {
        board = new BoardStore(cache);
        chats = new ChatStore(cache, () => Me);
    }

    public void Dispose() => cache.Dispose();

    private BoardModel Model(Server server) =>
        new(board, chats, new ApiClient(
            new HttpClient(server), ServerUrl.Normalise("chat.example.com")!,
            new MemoryTokenStore("t0ken")));

    private static NoteDto Note(
        long id,
        long seq,
        string kind = "text",
        string? text = "Milk",
        long author = 7,
        double x = 0.25,
        double y = 0.25,
        long? contentSeq = null,
        AttachmentDto? attachment = null,
        string size = "medium") =>
        new(
            id, author, kind, text, "yellow", size, "plain", x, y,
            BoardSeq: seq, ContentSeq: contentSeq ?? seq, Attachment: attachment);

    private static string NoteJson(long id, long seq, string text) =>
        $$$"""
        {"note": {"id": {{{id}}}, "author_id": 7, "kind": "text", "text": "{{{text}}}",
                  "color": "yellow", "size": "medium", "font": "plain", "x": 0.5, "y": 0.5,
                  "board_seq": {{{seq}}}, "content_seq": {{{seq}}}}}
        """;

    [Fact]
    public void AStickerIsPlacedAndSizedByTheSharedArithmetic()
    {
        board.Replace([Note(12, 11, x: 0.5, y: 0.25)], 11);
        var model = Model(new Server());

        var wall = BoardModel.WallSize(1000, 800);
        var sticker = Assert.Single(model.Stickers(1000, 800));

        // The same module the oracle pins to the web client's Rust decides all of this.
        var card = BoardWall.Card(NoteSize.Medium, BoardWall.IsCompact(wall.Width));
        var (x, y) = BoardWall.Origin((0.5, 0.25), card, wall);
        Assert.Equal(x, sticker.X);
        Assert.Equal(y, sticker.Y);
        Assert.Equal(card.Width, sticker.Width);
        Assert.Equal(card.Height, sticker.Height);
        Assert.Equal(BoardWall.TiltDegrees(12), sticker.TiltDegrees);
        Assert.Equal(NoteKind.Text, sticker.Kind);
        Assert.True(sticker.Mine);
    }

    /// <summary>
    /// A stored fraction is CLAMPED as it is drawn: 0.98 hugs the edge rather than hanging off
    /// it, which is what a wall that has been resized smaller since the drop leaves behind.
    /// </summary>
    [Fact]
    public void ANotePinnedAtTheEdgeIsDrawnInsideTheWall()
    {
        board.Replace([Note(12, 11, x: 0.98, y: 0.99)], 11);
        var model = Model(new Server());

        var wall = BoardModel.WallSize(1000, 800);
        var sticker = Assert.Single(model.Stickers(1000, 800));

        Assert.Equal(wall.Width - sticker.Width, sticker.X);
        Assert.Equal(wall.Height - sticker.Height, sticker.Y);
        // Fully on the wall, which is the whole of the rule.
        Assert.True(sticker.X + sticker.Width <= wall.Width);
        Assert.True(sticker.Y + sticker.Height <= wall.Height);
    }

    [Fact]
    public void SomebodyElsesNoteIsNotYours()
    {
        board.Replace([Note(12, 11), Note(13, 12, author: 11, x: 0.1)], 12);
        var model = Model(new Server());

        var stickers = model.Stickers(1000, 800);

        Assert.True(stickers.Single(sticker => sticker.Note.Id == 12).Mine);
        Assert.False(stickers.Single(sticker => sticker.Note.Id == 13).Mine);
    }

    /// <summary>
    /// A BARE PHOTO'S CARD IS THE PICTURE, fitted into the card it would otherwise have had —
    /// the rule issue #71 settled. A photo with a caption keeps the ordinary card.
    /// </summary>
    [Fact]
    public void ABarePhotosCardIsThePictureAndACaptionedOnesIsNot()
    {
        var tall = new AttachmentDto(34, "photo", Width: 600, Height: 1200);
        board.Replace([
            Note(12, 11, kind: "photo", text: string.Empty, attachment: tall),
            Note(13, 12, kind: "photo", text: "at the lake", attachment: tall, x: 0.1, y: 0.1),
        ], 12);
        var model = Model(new Server());

        var wall = BoardModel.WallSize(1000, 800);
        var card = BoardWall.Card(NoteSize.Medium, BoardWall.IsCompact(wall.Width));
        var stickers = model.Stickers(1000, 800);
        var bare = stickers.Single(sticker => sticker.Note.Id == 12);
        var captioned = stickers.Single(sticker => sticker.Note.Id == 13);

        var fitted = BoardPicture.Fitted(card.Width, card.Height, 600, 1200);
        Assert.Equal(fitted.Width, bare.Width);
        Assert.Equal(fitted.Height, bare.Height);
        // Fitted in BOTH dimensions, so a tall photograph comes back narrow and never cropped.
        Assert.True(bare.Width < card.Width);

        Assert.Equal(card.Width, captioned.Width);
        Assert.Equal(card.Height, captioned.Height);
    }

    /// <summary>
    /// THE BADGE COUNTS WHAT IS NEW TO READ. A note somebody dragged, recoloured or answered is
    /// not news — `content_seq` is — and the marks move only when the wall has been on screen.
    /// </summary>
    [Fact]
    public void TheBadgeCountsNewsAndShowingTheWallClearsIt()
    {
        board.Replace([Note(12, 11), Note(13, 12)], 12);
        var model = Model(new Server());
        Assert.Equal(2, model.Badge);
        Assert.All(model.Stickers(1000, 800), sticker => Assert.True(sticker.Unread));

        model.Shown();
        Assert.Equal(0, model.Badge);
        Assert.All(model.Stickers(1000, 800), sticker => Assert.False(sticker.Unread));

        // Somebody drags it across the wall: board_seq moves, content_seq does not, and the
        // badge stays quiet — on the wall as well as in the count.
        board.Apply(Note(12, 20, x: 0.9, contentSeq: 11));
        Assert.Equal(0, model.Badge);
        Assert.All(model.Stickers(1000, 800), sticker => Assert.False(sticker.Unread));

        // Somebody rewrites what it says, and that IS news.
        board.Apply(Note(12, 21, text: "Milk and eggs", contentSeq: 21));
        Assert.Equal(1, model.Badge);
        Assert.True(model.Stickers(1000, 800).Single(s => s.Note.Id == 12).Unread);
        Assert.False(model.Stickers(1000, 800).Single(s => s.Note.Id == 13).Unread);
    }

    [Fact]
    public async Task WritingANoteAppliesTheAnswerTheServerMadeOfIt()
    {
        var model = Model(new Server().On("/families/mine/board/notes", NoteJson(12, 11, "Milk")));

        Assert.Null(await model.CreateAsync(new NoteRequest("Milk", "yellow", 0.5, 0.5)));

        Assert.Equal("Milk", Assert.Single(board.Notes()).Text);
        // The answer to our OWN write moves no cursor: it is one note's news and says nothing
        // about what else has happened, and a cursor moved by it would step the changes feed
        // past somebody else's change.
        Assert.Equal(0, board.Cursor);
    }

    /// <summary>
    /// A POSITION IS A FRACTION OF THE WALL, read back from where the card actually is and
    /// clamped first: a note dropped past the edge is stored hugging it, not hanging off it.
    /// </summary>
    [Fact]
    public async Task MovingANoteStoresTheFractionItWasDroppedAt()
    {
        board.Replace([Note(12, 11)], 11);
        var server = new Server()
            .On("/families/mine/board/notes/12", NoteJson(12, 12, "Milk"));
        var model = Model(server);
        var wall = BoardModel.WallSize(1000, 800);
        var card = BoardWall.Card(NoteSize.Medium, BoardWall.IsCompact(wall.Width));

        // Dropped well past the right-hand edge.
        Assert.Null(await model.MoveAsync(12, (wall.Width + 500, 0), card, wall));

        var sent = Assert.Single(server.Bodies);
        var patch = Wire.Decode<NotePatch>(sent)!;
        // Clamped to the edge and then turned into a fraction, so the note is fully on the wall.
        Assert.Equal((wall.Width - card.Width) / wall.Width, patch.X!.Value, 6);
        Assert.Equal(0, patch.Y!.Value);
        // ONLY what changed: a patch carrying the text would overwrite an edit made in between.
        Assert.Null(patch.Text);
        Assert.Null(patch.Color);
    }

    [Fact]
    public async Task TakingANoteDownTakesItOffTheWallAtOnce()
    {
        board.Replace([Note(12, 11), Note(13, 12)], 12);
        var model = Model(new Server()
            .On("/families/mine/board/notes/12", null, HttpStatusCode.NoContent));

        Assert.Null(await model.DeleteAsync(12));

        Assert.Equal([13L], board.Notes().Select(note => note.Id));
        // And the board's cursor did not move: the seq this delete was given is the server's to
        // announce, and a cursor moved to a number nobody issued steps the feed past whatever
        // really holds it.
        Assert.Equal(12, board.Cursor);
        // It stays down, too, even if an older copy of it is still travelling.
        board.Apply(Note(12, 99));
        Assert.Equal([13L], board.Notes().Select(note => note.Id));
    }

    [Fact]
    public async Task ARefusedDeleteLeavesTheNoteWhereItIs()
    {
        board.Replace([Note(12, 11)], 11);
        var model = Model(new Server().On(
            "/families/mine/board/notes/12",
            """{"error": {"code": "not_note_author", "message": "no"}}""",
            HttpStatusCode.Forbidden));

        var refused = await model.DeleteAsync(12);

        Assert.Equal("not_note_author", refused!.Code);
        Assert.Single(board.Notes());
    }

    [Fact]
    public async Task AnsweringAnEventIsAStateSetAndRetractingIsNotANo()
    {
        board.Replace([Note(12, 11, kind: "event", text: "Gran's birthday")], 11);
        var server = new Server().On("/families/mine/board/notes/12/rsvp", """
            {"note": {"id": 12, "author_id": 7, "kind": "event", "text": "Gran's birthday",
                      "board_seq": 12, "content_seq": 11,
                      "rsvps": [{"user_id": 7, "answer": "going"}]}}
            """);
        var model = Model(server);

        Assert.Null(await model.AnswerAsync(12, RsvpAnswer.Going));

        Assert.Equal("going", board.Note(12)!.MyAnswer(Me));
        Assert.Contains("\"answer\":\"going\"", Assert.Single(server.Bodies));
        // Answering is not news: the words did not change, so the badge stays where it was.
        Assert.Equal(11, board.Note(12)!.ContentSeq);
    }

    [Fact]
    public async Task TickingALineIsPerLineAndTheAnswerIsTheWholeNote()
    {
        board.Replace([Note(12, 11, kind: "tasks", text: "Shopping")], 11);
        var server = new Server().On("/families/mine/board/notes/12/tasks/5", """
            {"note": {"id": 12, "author_id": 7, "kind": "tasks", "text": "Shopping",
                      "board_seq": 12, "content_seq": 12,
                      "items": [{"id": 5, "text": "Milk", "done": true, "done_by": 7},
                                {"id": 6, "text": "Eggs", "done": false}]}}
            """);
        var model = Model(server);

        Assert.Null(await model.TickAsync(12, 5, done: true));

        var items = board.Note(12)!.TaskList;
        Assert.True(items[0].Done);
        Assert.Equal(7, items[0].DoneBy);
        Assert.False(items[1].Done);
        Assert.Contains("\"done\":true", Assert.Single(server.Bodies));
    }

    /// <summary>
    /// A REDRAWN BACKDROP IS A NEW ATTACHMENT — the answer carries a new id, and a caller that
    /// cached the picture by NOTE would show the old one for ever (issue #70, on three platforms).
    /// </summary>
    [Fact]
    public async Task ADrawnBackdropComesBackAsANewAttachment()
    {
        board.Replace([
            Note(12, 11, kind: "event", text: "Gran's birthday",
                attachment: new AttachmentDto(34, "photo", Width: 1024, Height: 512)),
        ], 11);
        var model = Model(new Server().On("/families/mine/board/notes/12/backdrop", """
            {"note": {"id": 12, "author_id": 7, "kind": "event", "text": "Gran's birthday",
                      "board_seq": 12, "content_seq": 11,
                      "attachment": {"id": 77, "kind": "photo", "width": 1024, "height": 512}}}
            """));

        var (drawn, error) = await model.DrawBackdropAsync(12);

        Assert.Null(error);
        // WHAT THE ASSISTANT DREW, from the answer itself rather than read back out of the store
        // — the caller redraws by attachment id, and an answer that crossed a newer change on
        // the wire would otherwise hand back the picture already on the wall.
        Assert.Equal(77, drawn!.Id);
        Assert.Equal(77, board.Note(12)!.Attachment!.Id);
        // The picture changed and the words did not: a redraw is not news either.
        Assert.Equal(11, board.Note(12)!.ContentSeq);
    }

    [Fact]
    public async Task ARefusedDrawSaysWhyAndChangesNothing()
    {
        board.Replace([Note(12, 11, kind: "event", text: "Gran's birthday")], 11);
        var model = Model(new Server().On(
            "/families/mine/board/notes/12/backdrop",
            """{"error": {"code": "ai_unavailable", "message": "no"}}""",
            HttpStatusCode.ServiceUnavailable));

        var (drawn, error) = await model.DrawBackdropAsync(12);

        Assert.Null(drawn);
        Assert.Equal("ai_unavailable", error!.Code);
        Assert.Null(board.Note(12)!.Attachment);
    }

    /// <summary>
    /// A backdrop answer that crossed a newer change still says what was DRAWN: the store keeps
    /// the newer note, and the caller is told the attachment it asked about.
    /// </summary>
    [Fact]
    public async Task ADrawAnswerSaysWhatWasDrawnEvenWhenTheStoreKeepsSomethingNewer()
    {
        board.Replace([
            Note(12, 30, kind: "event", text: "Gran's birthday", contentSeq: 30,
                attachment: new AttachmentDto(34, "photo", Width: 1024, Height: 512)),
        ], 30);
        var model = Model(new Server().On("/families/mine/board/notes/12/backdrop", """
            {"note": {"id": 12, "author_id": 7, "kind": "event", "text": "Gran's birthday",
                      "board_seq": 12, "content_seq": 11,
                      "attachment": {"id": 77, "kind": "photo", "width": 1024, "height": 512}}}
            """));

        var (drawn, error) = await model.DrawBackdropAsync(12);

        Assert.Null(error);
        Assert.Equal(77, drawn!.Id);
        // The wall kept the newer note, guard and all.
        Assert.Equal(34, board.Note(12)!.Attachment!.Id);
    }

    /// <summary>
    /// An answer and the frame the same write raised can arrive in either order, so the answer is
    /// applied under the `board_seq` guard like everything else.
    /// </summary>
    [Fact]
    public async Task AnAnswerThatIsOlderThanWhatIsHeldDoesNotUndoIt()
    {
        board.Replace([Note(12, 30, text: "Milk and eggs", contentSeq: 30)], 30);
        var model = Model(new Server()
            .On("/families/mine/board/notes/12", NoteJson(12, 12, "Milk")));

        Assert.Null(await model.PatchAsync(12, new NotePatch(Color: "pink")));

        Assert.Equal("Milk and eggs", board.Note(12)!.Text);
    }
}
