namespace FamilyConnect.Core.Board;

/// <summary>
/// THE TEXT FITS THE NOTE (docs/protocol.md, "Board"): the type scales down from the size's own
/// ceiling until the whole note is inside the card, and only past the floor is anything cut.
/// </summary>
/// <remarks>
/// Web counterpart: <c>fc_text::board::fitted_scale</c> / <c>lines_that_fit</c>. Apple does it
/// with <c>minimumScaleFactor</c>, Android with its own measure pass — the FLOOR and the number of
/// halvings are shared so the same note reaches the same size everywhere.
/// </remarks>
public static class NoteFitting
{
    /// <summary>
    /// How far the type may shrink before the text is cut instead. Below it the note truncates,
    /// which is the one case a reader has to open it for.
    /// </summary>
    public const double MinTextScale = 0.6;

    /// <summary>
    /// How many halvings the search takes: a hundredth of the range from the floor to the ceiling
    /// is well under a pixel at every size.
    /// </summary>
    public const int FitSteps = 7;

    /// <summary>
    /// The largest scale, from <paramref name="floor"/> up to 1, at which the text fits — or null
    /// when it does not fit even at the floor and must be cut.
    /// </summary>
    /// <remarks>
    /// <paramref name="fits"/> answers for one scale. Smaller type never fits worse, which is what
    /// lets this halve the range rather than walk it.
    /// </remarks>
    public static double? FittedScale(Func<double, bool> fits, double floor, int steps)
    {
        if (fits(1.0))
        {
            return 1.0;
        }
        if (!fits(floor))
        {
            return null;
        }
        // `low` always fits and `high` never does.
        var (low, high) = (floor, 1.0);
        for (var step = 0; step < steps; step++)
        {
            var middle = (low + high) / 2.0;
            if (fits(middle))
            {
                low = middle;
            }
            else
            {
                high = middle;
            }
        }
        return low;
    }

    /// <summary>
    /// The lines a box <paramref name="height"/> tall holds at <paramref name="lineHeight"/>, at
    /// least one — how far text cut at the floor may run before its ellipsis.
    /// </summary>
    public static int LinesThatFit(double height, double lineHeight) =>
        lineHeight <= 0 || double.IsNaN(height) || double.IsInfinity(height)
            ? 1
            : Math.Max(1, (int)Math.Floor(height / lineHeight));
}
