using FamilyConnect.App.Logic;
using Xunit;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// A frame of a transcode beside the same frame of its source — the check for a result whose NUMBERS are all as asked
/// and whose picture is on its side. The frames here are drawn by this test: a scene with a bright sky over dark
/// ground and one lit window off-centre, which looks like itself in exactly one of its four turns.
/// </summary>
public sealed class FrameTurnTests
{
    private const int Side = FrameTurn.Side;

    /// <summary>Sky above, ground below, a window up and to the left — brightness, row by row.</summary>
    private static double[] Scene()
    {
        var grid = new double[Side * Side];
        for (var row = 0; row < Side; row++)
        {
            for (var column = 0; column < Side; column++)
            {
                var sky = row < Side / 2 ? 190 - (row * 4) : 60 + column;
                var window = row is >= 2 and < 5 && column is >= 3 and < 6 ? 60 : 0;
                grid[(row * Side) + column] = sky + window;
            }
        }
        return grid;
    }

    /// <summary>What an encoder does to a frame: every cell a little off, by a pattern that is no picture.</summary>
    private static double[] Encoded(double[] grid) =>
        [.. grid.Select((cell, at) => cell + (((at * 7919) % 11) - 5))];

    private static byte[] Pixels(double[] grid)
    {
        var bgra = new byte[4 * grid.Length];
        for (var cell = 0; cell < grid.Length; cell++)
        {
            var value = (byte)Math.Clamp(Math.Round(grid[cell]), 0, 255);
            (bgra[4 * cell], bgra[(4 * cell) + 1], bgra[(4 * cell) + 2], bgra[(4 * cell) + 3]) = (value, value, value, 255);
        }
        return bgra;
    }

    [Fact]
    public void AGridIsEachCellsBrightness()
    {
        var grey = FrameTurn.Grid(Pixels(Scene()))!;
        Assert.Equal(Side * Side, grey.Length);
        Assert.Equal(Scene()[0], grey[0], 3);
        Assert.Equal(Scene()[^1], grey[^1], 3);

        // Green weighs most and blue least, as an eye has it.
        var colours = new byte[4 * Side * Side];
        (colours[0], colours[5], colours[10]) = (255, 255, 255);
        var weighed = FrameTurn.Grid(colours)!;
        Assert.True(weighed[1] > weighed[2] && weighed[2] > weighed[0] && weighed[0] > 0);

        Assert.Null(FrameTurn.Grid(new byte[4 * Side]));
        Assert.Null(FrameTurn.Grid([]));
    }

    [Fact]
    public void FourQuarterTurnsAreNoTurnAndOneMovesTheTopLeftToTheTopRight()
    {
        var scene = Scene();
        Assert.Equal(scene, FrameTurn.Turn(scene, 0));
        Assert.Equal(scene, FrameTurn.Turn(scene, 4));
        Assert.Equal(scene, FrameTurn.Turn(FrameTurn.Turn(scene, 1), 3));
        Assert.Equal(FrameTurn.Turn(scene, 2), FrameTurn.Turn(FrameTurn.Turn(scene, 1), 1));
        Assert.Equal(FrameTurn.Turn(scene, 3), FrameTurn.Turn(scene, -1));

        var corner = new double[Side * Side];
        corner[0] = 1;
        Assert.Equal(1, FrameTurn.Turn(corner, 1)[Side - 1]);
        Assert.Equal(1, FrameTurn.Turn(corner, 2)[(Side * Side) - 1]);
        Assert.Equal(1, FrameTurn.Turn(corner, 3)[(Side - 1) * Side]);
    }

    [Fact]
    public void AReEncodeOfTheSameFrameIsTheSamePicture()
    {
        Assert.Equal(FrameVerdict.Same, FrameTurn.Judge(Scene(), Scene()));
        Assert.Equal(FrameVerdict.Same, FrameTurn.Judge(Scene(), Encoded(Scene())));
    }

    /// <summary>An HDR source beside its tone-mapped result: darker all over and flatter, and still the same picture.</summary>
    [Fact]
    public void ADarkerFlatterCopyIsTheSamePicture()
    {
        double[] toneMapped = [.. Scene().Select(cell => 20 + (0.6 * cell))];
        Assert.Equal(FrameVerdict.Same, FrameTurn.Judge(Scene(), Encoded(toneMapped)));
    }

    /// <summary>
    /// THE case: a portrait clip whose pixels the transcoder turned as well as carrying the turn. Every number read back
    /// is what was asked for; the picture is on its side — or, where two conventions disagree, upside down.
    /// </summary>
    [Theory]
    [InlineData(1)]
    [InlineData(2)]
    [InlineData(3)]
    public void AResultOnItsSideOrUpsideDownIsTurned(int quarters)
    {
        Assert.Equal(FrameVerdict.Turned, FrameTurn.Judge(Scene(), Encoded(FrameTurn.Turn(Scene(), quarters))));
    }

    /// <summary>
    /// The other way a double turn comes out: the upright frame fitted INTO the landscape one with bars either side, and
    /// then turned. It is not the source's picture in any of its four turns.
    /// </summary>
    [Fact]
    public void AResultSquashedBetweenBarsIsUnlikeItsSource()
    {
        var scene = Scene();
        var turned = FrameTurn.Turn(scene, 1);
        var barred = new double[Side * Side];
        for (var row = 0; row < Side; row++)
        {
            for (var column = 0; column < Side; column++)
            {
                // The middle five rows hold the whole turned picture, squeezed; the rest is black.
                var inside = row is >= 5 and < 10;
                barred[(row * Side) + column] = inside ? turned[((row - 5) * 3 * Side) + column] : 0;
            }
        }
        Assert.Equal(FrameVerdict.Unlike, FrameTurn.Judge(scene, barred));
    }

    /// <summary>A horizon, a corridor: the same upside down. No turned reading is CLEARLY better, so it is the same picture.</summary>
    [Fact]
    public void APictureThatLooksTheSameTurnedIsNotCalledTurned()
    {
        var rings = new double[Side * Side];
        for (var row = 0; row < Side; row++)
        {
            for (var column = 0; column < Side; column++)
            {
                var fromCentre = Math.Max(Math.Abs(row - 7.5), Math.Abs(column - 7.5));
                rings[(row * Side) + column] = 40 + (fromCentre * 25);
            }
        }
        Assert.Equal(FrameVerdict.Same, FrameTurn.Judge(rings, Encoded(FrameTurn.Turn(rings, 1))));
        Assert.Equal(FrameVerdict.Same, FrameTurn.Judge(rings, Encoded(FrameTurn.Turn(rings, 2))));
    }

    /// <summary>A clip that opens on black: there is nothing to compare, turned or not.</summary>
    [Fact]
    public void ABlackOrFlatFrameSaysNothing()
    {
        var black = new double[Side * Side];
        double[] dim = [.. Enumerable.Range(0, Side * Side).Select(at => 12.0 + (at % 5))];
        Assert.Equal(FrameVerdict.CannotTell, FrameTurn.Judge(black, black));
        Assert.Equal(FrameVerdict.CannotTell, FrameTurn.Judge(dim, Scene()));
        Assert.Equal(FrameVerdict.CannotTell, FrameTurn.Judge(Scene(), black));
        Assert.Equal(FrameVerdict.CannotTell, FrameTurn.Judge(Scene(), new double[4]));
    }

    [Fact]
    public void ADifferentPictureIsUnlike()
    {
        double[] stripes = [.. Enumerable.Range(0, Side * Side).Select(at => at % 2 == 0 ? 200.0 : 30.0)];
        Assert.Equal(FrameVerdict.Unlike, FrameTurn.Judge(Scene(), stripes));
    }

    /// <summary>
    /// What a clip's frames add up to: a turned one fails it at once, a same one passes it, an unlike one fails it unless
    /// another moment is the same, and frames that say nothing let it go — a clip of a dark room must stay sendable.
    /// </summary>
    [Fact]
    public void TheFramesOfOneClipAddUp()
    {
        Assert.True(FrameTurn.LooksRight([FrameVerdict.Same]));
        Assert.True(FrameTurn.LooksRight([FrameVerdict.CannotTell, FrameVerdict.Same]));
        // A cut exactly at the first moment asked: the next moment settles it.
        Assert.True(FrameTurn.LooksRight([FrameVerdict.Unlike, FrameVerdict.Same]));
        Assert.True(FrameTurn.LooksRight([FrameVerdict.CannotTell, FrameVerdict.CannotTell, FrameVerdict.CannotTell]));
        Assert.True(FrameTurn.LooksRight([]));

        Assert.False(FrameTurn.LooksRight([FrameVerdict.Turned]));
        Assert.False(FrameTurn.LooksRight([FrameVerdict.CannotTell, FrameVerdict.Turned]));
        Assert.False(FrameTurn.LooksRight([FrameVerdict.Unlike, FrameVerdict.Turned, FrameVerdict.Same]));
        Assert.False(FrameTurn.LooksRight([FrameVerdict.Unlike, FrameVerdict.CannotTell, FrameVerdict.Unlike]));
        Assert.False(FrameTurn.LooksRight([FrameVerdict.CannotTell, FrameVerdict.Unlike]));
    }
}
