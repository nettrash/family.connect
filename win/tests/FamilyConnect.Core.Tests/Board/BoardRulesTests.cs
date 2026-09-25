using FamilyConnect.Core;
using FamilyConnect.Core.Board;

namespace FamilyConnect.Core.Tests.Board;

/// <summary>
/// The board's shared arithmetic, held to the numbers the other three clients use: a wall that
/// measured its own way would not be the same wall (docs/protocol.md, "Board").
/// </summary>
public class BoardRulesTests
{
    [Fact]
    public void TheWallIsTallerThanTheWindowAndNeverShorter()
    {
        Assert.Equal(1.6, BoardWall.Screens);
        Assert.Equal(800.0, BoardWall.Height(500.0));
        Assert.True(BoardWall.Height(1000.0) > 1000.0);
        // And never SHORTER, whatever a window does: a wall smaller than the window would put
        // fractions of it behind the edges.
        Assert.Equal(0.0, BoardWall.Height(0.0));
        Assert.True(BoardWall.Height(1.0) >= 1.0);
    }

    [Fact]
    public void ANarrowWindowTakesThePhonesSquareStickers()
    {
        Assert.Equal(640.0, BoardWall.CompactBelow);
        Assert.True(BoardWall.IsCompact(639.0));
        Assert.False(BoardWall.IsCompact(640.0));
        // The desktop's landscape cards, and the same three steps squared on a narrow wall.
        Assert.Equal((150.0, 110.0), BoardWall.Card(NoteSize.Medium, compact: false));
        Assert.Equal((120.0, 88.0), BoardWall.Card(NoteSize.Small, compact: false));
        Assert.Equal((280.0, 200.0), BoardWall.Card(NoteSize.Large, compact: false));
        Assert.Equal((132.0, 132.0), BoardWall.Card(NoteSize.Medium, compact: true));
        // Type climbs with the step.
        Assert.True(BoardWall.TypePx(NoteSize.Small) < BoardWall.TypePx(NoteSize.Medium));
        Assert.True(BoardWall.TypePx(NoteSize.Medium) < BoardWall.TypePx(NoteSize.Large));
    }

    [Fact]
    public void AStoredFractionIsTheTopLeftCornerHeldInsideTheWall()
    {
        var card = (150.0, 110.0);
        var board = (900.0, 600.0);
        Assert.Equal((450.0, 150.0), BoardWall.Origin((0.5, 0.25), card, board));
        // A stored 0.98 hugs the edge rather than hanging off it.
        Assert.Equal((750.0, 490.0), BoardWall.Origin((0.98, 0.98), card, board));
        Assert.Equal((0.0, 0.0), BoardWall.Origin((-1.0, -1.0), card, board));
        // A board smaller than the card pins it to the top-left.
        Assert.Equal((0.0, 0.0), BoardWall.Origin((0.5, 0.5), card, (100.0, 80.0)));
        // In hand: the origin moved, and held inside again.
        Assert.Equal((470.0, 130.0), BoardWall.Dragged((0.5, 0.25), (20.0, -20.0), card, board));
        // Read back from where it is DRAWN, so what was dropped is what gets stored.
        var (x, y) = BoardWall.FractionOf((450.0, 150.0), board);
        Assert.Equal(0.5, x, 6);
        Assert.Equal(0.25, y, 6);
        Assert.Equal((1.0, 1.0), BoardWall.FractionOf((2000.0, 2000.0), board));
        Assert.Equal((0.0, 0.0), BoardWall.FractionOf((-5.0, -5.0), board));
    }

    [Fact]
    public void ANoteKeepsItsTiltForEverybodyAndAcrossLaunches()
    {
        // The Mac's -3…3, derived from the id: a wall of perfectly square notes reads as a table.
        for (long id = -20; id <= 20; id++)
        {
            var tilt = BoardWall.TiltDegrees(id);
            Assert.InRange(tilt, -3, 3);
            Assert.Equal(tilt, BoardWall.TiltDegrees(id));
        }
        Assert.Equal(-3, BoardWall.TiltDegrees(0));
        Assert.Equal(3, BoardWall.TiltDegrees(6));
        Assert.Equal(-3, BoardWall.TiltDegrees(7));
        // A negative id — no server sends one, but nothing here may throw on it.
        Assert.Equal(-3, BoardWall.TiltDegrees(-7));
    }

    [Fact]
    public void AStickerDrawsTheFirstLinesOfAListAndSaysHowManyAreLeft()
    {
        Assert.Equal(5, BoardTasks.OnWall);
        Assert.Equal((0, 0), BoardTasks.Drawn(0));
        Assert.Equal((3, 0), BoardTasks.Drawn(3));
        // A list of exactly the cap says nothing extra.
        Assert.Equal((5, 0), BoardTasks.Drawn(5));
        Assert.Equal((5, 1), BoardTasks.Drawn(6));
        Assert.Equal((5, 15), BoardTasks.Drawn(20));
    }

    /// <summary>
    /// A PHOTO IS DRAWN WHOLE (issue #71): fitted in both dimensions, never cropped — the same
    /// numbers <c>fc_text::board::fitted_picture</c>, Apple's and Android's pin.
    /// </summary>
    [Fact]
    public void APictureIsFittedInBothDimensionsAndNeverCropped()
    {
        // The 600x1200 portrait the issue was reported with, on the medium card.
        var (width, height) = BoardPicture.Fitted(150, 110, 600, 1200);
        Assert.Equal(55.0, width, 2);
        Assert.Equal(110.0, height, 2);
        Assert.Equal(0.5, width / height, 3);
        // A wide one comes back SHORT, by the same rule.
        var (wide, short_) = BoardPicture.Fitted(150, 110, 1600, 900);
        Assert.Equal(150.0, wide, 2);
        Assert.Equal(84.375, short_, 3);
        // A picture the shape of its space fills it exactly, and neither side ever grows past it.
        Assert.Equal((150.0, 110.0), BoardPicture.Fitted(150, 110, 300, 220));
        var (small, smaller) = BoardPicture.Fitted(150, 110, 15, 11);
        Assert.True(small <= 150 && smaller <= 110);
        // Dimensions the server never recorded take the whole space: a margin at worst.
        Assert.Equal((150.0, 110.0), BoardPicture.Fitted(150, 110, null, null));
        Assert.Equal((150.0, 110.0), BoardPicture.Fitted(150, 110, 600, null));
        Assert.Equal((150.0, 110.0), BoardPicture.Fitted(150, 110, 0, 1200));
        Assert.Equal((150.0, 110.0), BoardPicture.Fitted(150, 110, -4, 8));
        // And a panorama still leaves something to click.
        var (_, hairline) = BoardPicture.Fitted(150, 110, 20_000, 10);
        Assert.True(hairline >= 1.0);
        // FITTED, never filled: the one word #71 was, named so a test can hold it.
        Assert.Equal("Uniform", BoardPicture.Stretch);
    }

    [Fact]
    public void TheTextFitsTheNoteDownToTheFloorAndIsCutBelowIt()
    {
        Assert.Equal(0.6, NoteFitting.MinTextScale);
        // Fits at full size: nothing to search for.
        Assert.Equal(1.0, NoteFitting.FittedScale(_ => true, 0.6, 7));
        // Never fits: it has to be cut instead.
        Assert.Null(NoteFitting.FittedScale(_ => false, 0.6, 7));
        // Fits only below 0.8: the search lands just under it and never above.
        var scale = NoteFitting.FittedScale(candidate => candidate <= 0.8, 0.6, 7);
        Assert.NotNull(scale);
        Assert.True(scale <= 0.8);
        Assert.True(scale > 0.79);
        // Lines are at least one, whatever the arithmetic says.
        Assert.Equal(4, NoteFitting.LinesThatFit(80, 20));
        Assert.Equal(1, NoteFitting.LinesThatFit(10, 20));
        Assert.Equal(1, NoteFitting.LinesThatFit(80, 0));
        Assert.Equal(1, NoteFitting.LinesThatFit(double.NaN, 20));
    }

    [Fact]
    public void TheBadgeCountsWhatANoteSAYSAndNeverAMove()
    {
        var marks = new BoardMarks(NoteId: 10, ContentSeq: 100);
        // Judged by its content seq when it has one…
        Assert.True(BoardBadge.IsUnread(3, 101, marks));
        Assert.False(BoardBadge.IsUnread(99, 100, marks));
        // …and by its id when it does not (an older server).
        Assert.True(BoardBadge.IsUnread(11, null, marks));
        Assert.False(BoardBadge.IsUnread(10, null, marks));
        // A zero seq is "no data", not "seq 0".
        Assert.True(BoardBadge.IsUnread(11, 0, marks));
        // Showing the board marks everything on it, monotonically in both fields: a board that
        // has just lost its newest note must not bring a cleared badge back.
        var shown = BoardBadge.AfterShowing([(12, 130), (4, null)], marks);
        Assert.Equal(new BoardMarks(12, 130), shown);
        Assert.Equal(shown, BoardBadge.AfterShowing([(2, 5)], shown));
        // Two windows of one account: neither walks the other's marks back.
        Assert.Equal(new BoardMarks(12, 130), BoardMarks.Later(shown, marks));
        Assert.Equal(2, BoardBadge.Count([(11, null), (3, 101), (2, 7)], marks));
    }
}
