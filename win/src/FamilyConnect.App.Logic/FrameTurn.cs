using FamilyConnect.Core;

namespace FamilyConnect.App.Logic;

/// <summary>What a frame of a transcode looks like beside the same frame of its source.</summary>
public enum FrameVerdict
{
    /// <summary>The same picture, the same way up.</summary>
    Same,

    /// <summary>The source's picture on its side or upside down.</summary>
    Turned,

    /// <summary>A picture, but not the source's — squashed into bars, or something else altogether.</summary>
    Unlike,

    /// <summary>Nothing to go by: a black frame, a fade, a flat wall.</summary>
    CannotTell,
}

/// <summary>
/// Whether a transcode still LOOKS like what it came from — the half of the read-back that the container's numbers
/// cannot give (<see cref="MediaEncoding.Matches"/> is the other half).
/// </summary>
/// <remarks>
/// <para>
/// <b>THE NUMBERS CAN ALL BE RIGHT AND THE PICTURE WRONG.</b> A portrait phone clip is asked for in its STORED sides
/// with its turn as metadata. If the transcoder also turns the PIXELS — or the turn that was read and the turn that was
/// written mean opposite directions — the result has exactly the sides and the turn that were asked for, and plays on
/// its side, squashed. Nothing on the Mac this port is written on can watch Media Foundation run, so the result is
/// looked at instead: one frame of the source and the same frame of the result, read by the SAME reader and compared.
/// Whatever that reader does with a turn it does to both, so the comparison holds whichever convention it has.
/// </para>
/// <para>
/// <b>A FRAME IS SQUASHED TO A SQUARE GRID OF BRIGHTNESS</b>, <see cref="Side"/> each way, because a square can be
/// turned and compared cell for cell; and what is compared is how the cells VARY together (a correlation), not their
/// values, so an HDR source beside its tone-mapped result — darker or lighter all over — is still the same picture.
/// </para>
/// <para>
/// <b>ONLY A FRAME WITH SOMETHING IN IT CAN SAY ANYTHING.</b> A clip that opens on black correlates with nothing, turned
/// or not; that is <see cref="FrameVerdict.CannotTell"/>, and another moment of the clip is asked.
/// </para>
/// </remarks>
public static class FrameTurn
{
    /// <summary>The grid's side: fine enough to tell a picture from its quarter turn, coarse enough to ignore the encoder.</summary>
    public const int Side = 16;

    /// <summary>Brightness that varies by less than this (of 255) across a frame is no picture to compare.</summary>
    private const double Flat = 8;

    /// <summary>How alike two grids must be to be the same picture — a re-encode of the same frame is far above it.</summary>
    private const double Alike = 0.7;

    /// <summary>How much more alike than the upright reading a turned one must be before the result is called turned.</summary>
    private const double Clearly = 0.2;

    /// <summary>
    /// The grid of a frame already scaled to <see cref="Side"/> × <see cref="Side"/> BGRA pixels: each cell's brightness
    /// (BT.601's weights — which ones matters little, since only how cells differ is compared). Null when it is not
    /// that many pixels.
    /// </summary>
    public static double[]? Grid(ReadOnlySpan<byte> bgra)
    {
        if (bgra.Length != 4 * Side * Side)
        {
            return null;
        }
        var grid = new double[Side * Side];
        for (var cell = 0; cell < grid.Length; cell++)
        {
            grid[cell] = (0.114 * bgra[4 * cell]) + (0.587 * bgra[(4 * cell) + 1]) + (0.299 * bgra[(4 * cell) + 2]);
        }
        return grid;
    }

    /// <summary>
    /// <paramref name="result"/> beside <paramref name="source"/>. Turned only when a turned reading is CLEARLY the
    /// better one — a picture that looks the same upside down (a horizon, a corridor) is the same picture.
    /// </summary>
    public static FrameVerdict Judge(double[] source, double[] result)
    {
        if (source.Length != Side * Side || result.Length != Side * Side || IsFlat(source) || IsFlat(result))
        {
            return FrameVerdict.CannotTell;
        }
        var upright = Likeness(source, result);
        var turned = new[] { 1, 2, 3 }.Max(quarters => Likeness(Turn(source, quarters), result));
        return turned >= Alike && turned > upright + Clearly ? FrameVerdict.Turned
            : upright >= Alike ? FrameVerdict.Same
            : FrameVerdict.Unlike;
    }

    /// <summary>
    /// What the frames of one transcode add up to, asked in order (<see cref="MediaPrep.PosterSeekSeconds"/>): true when
    /// it may go. One turned frame fails it and one that is the same passes it; when no frame is the same, one unlike
    /// its source fails it too. Frames that say nothing at all — every one black, or none readable — let it go: the
    /// numbers were right, and a clip of a dark room must not be unsendable.
    /// </summary>
    public static bool LooksRight(IEnumerable<FrameVerdict> frames)
    {
        var unlike = false;
        foreach (var frame in frames)
        {
            switch (frame)
            {
                case FrameVerdict.Turned:
                    return false;
                case FrameVerdict.Same:
                    return true;
                case FrameVerdict.Unlike:
                    unlike = true;
                    break;
            }
        }
        return !unlike;
    }

    /// <summary>The grid turned clockwise by <paramref name="quarters"/> quarter turns.</summary>
    public static double[] Turn(double[] grid, int quarters)
    {
        var turned = grid;
        for (var turn = 0; turn < ((quarters % 4) + 4) % 4; turn++)
        {
            var next = new double[Side * Side];
            for (var row = 0; row < Side; row++)
            {
                for (var column = 0; column < Side; column++)
                {
                    // Clockwise: the left column, read bottom to top, becomes the top row.
                    next[(row * Side) + column] = turned[((Side - 1 - column) * Side) + row];
                }
            }
            turned = next;
        }
        return turned;
    }

    private static bool IsFlat(double[] grid)
    {
        var mean = grid.Average();
        return Math.Sqrt(grid.Sum(cell => (cell - mean) * (cell - mean)) / grid.Length) < Flat;
    }

    /// <summary>Pearson's correlation of two grids that are neither of them flat: 1 for the same picture, about 0 for unrelated ones.</summary>
    private static double Likeness(double[] one, double[] other)
    {
        var oneMean = one.Average();
        var otherMean = other.Average();
        double together = 0, oneSpread = 0, otherSpread = 0;
        for (var cell = 0; cell < one.Length; cell++)
        {
            var a = one[cell] - oneMean;
            var b = other[cell] - otherMean;
            together += a * b;
            oneSpread += a * a;
            otherSpread += b * b;
        }
        return together / Math.Sqrt(oneSpread * otherSpread);
    }
}
