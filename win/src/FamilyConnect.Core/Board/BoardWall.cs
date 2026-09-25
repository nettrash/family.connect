namespace FamilyConnect.Core.Board;

/// <summary>
/// Where a note sits, and how big it is drawn (docs/protocol.md, "Board").
/// </summary>
/// <remarks>
/// <para>
/// The numbers here are shared by all four clients even though the wire says nothing about them:
/// <c>x</c> and <c>y</c> are fractions of the WALL, so a note two thirds of the way down has to be
/// two thirds of the way down on the phone, on the Mac, in a browser and here.
/// </para>
/// <para>
/// Web counterpart: <c>fc_text::board</c> (<c>WALL_SCREENS</c>, <c>COMPACT_BELOW</c>,
/// <c>Size::frame</c>, <c>clamp_corner</c>, <c>origin</c>, <c>fraction_of</c>,
/// <c>tilt_degrees</c>). Apple: <c>BoardWall</c> + <c>NoteSize.frame</c>. Android:
/// <c>BoardWall</c> + <c>NoteSizes</c>.
/// </para>
/// </remarks>
public static class BoardWall
{
    /// <summary>
    /// How many windows tall the wall is. A wall the size of the window is a wall that fills up,
    /// and then a family has to take something down before it can say anything.
    /// </summary>
    public const double Screens = 1.6;

    /// <summary>
    /// The wall's height for a window of <paramref name="visible"/> height — never shorter than
    /// the window, or fractions of the wall would sit behind its edges.
    /// </summary>
    public static double Height(double visible) => Math.Max(visible * Screens, visible);

    /// <summary>The wall's own size, for the fractions to be read against.</summary>
    public static (double Width, double Height) Size(double visibleWidth, double visibleHeight) =>
        (visibleWidth, Height(visibleHeight));

    /// <summary>
    /// Below this width a wall takes the phone's square stickers: the landscape cards would cover
    /// most of a narrow board between them. A desktop window can be dragged this narrow, so the
    /// rule is a width test and not a platform one.
    /// </summary>
    public const double CompactBelow = 640.0;

    public static bool IsCompact(double boardWidth) => boardWidth < CompactBelow;

    /// <summary>
    /// The card, in device-independent pixels: the desktop's landscape card on a wall wide enough
    /// for it, and the phone's square sticker on a narrow one — the same step drawn at the idiom
    /// of the window it is in.
    /// </summary>
    public static (double Width, double Height) Card(NoteSize size, bool compact) => (size, compact) switch
    {
        (NoteSize.Small, false) => (120.0, 88.0),
        (NoteSize.Medium, false) => (150.0, 110.0),
        (NoteSize.Large, false) => (280.0, 200.0),
        (NoteSize.Small, true) => (100.0, 100.0),
        (NoteSize.Medium, true) => (132.0, 132.0),
        _ => (220.0, 220.0),
    };

    /// <summary>
    /// The type size the text starts from — the CEILING, which fitting scales down from until the
    /// whole note is inside the card. It climbs with the step: a large note is meant to be read
    /// from across the room, not to hold more of the same small print.
    /// </summary>
    public static double TypePx(NoteSize size) => size switch
    {
        NoteSize.Small => 12.0,
        NoteSize.Large => 15.0,
        _ => 14.0,
    };

    /// <summary>
    /// A card's top-left corner held inside the board, so no part of it is off screen — which
    /// depends on the CARD, a large one running out of room sooner, and a bare photo's being the
    /// shape of the photograph. A board smaller than the card pins it to the top-left.
    /// </summary>
    public static (double X, double Y) Clamp(
        double x, double y, (double Width, double Height) card, (double Width, double Height) board) =>
        (Math.Max(0.0, Math.Min(x, board.Width - card.Width)),
         Math.Max(0.0, Math.Min(y, board.Height - card.Height)));

    /// <summary>
    /// Where a stored position puts a card: the fraction is its TOP-LEFT corner, as the protocol
    /// says and every client draws it — clamped, so a stored 0.98 hugs the edge rather than
    /// hanging off it.
    /// </summary>
    public static (double X, double Y) Origin(
        (double X, double Y) fraction, (double Width, double Height) card, (double Width, double Height) board) =>
        Clamp(fraction.X * board.Width, fraction.Y * board.Height, card, board);

    /// <summary>
    /// Where a card sits while it is in hand: its origin moved by the drag, and held inside again.
    /// Clamping only on release would let a note be dragged off the edge and then snap back.
    /// </summary>
    public static (double X, double Y) Dragged(
        (double X, double Y) fraction,
        (double X, double Y) offset,
        (double Width, double Height) card,
        (double Width, double Height) board)
    {
        var (x, y) = Origin(fraction, card, board);
        return Clamp(x + offset.X, y + offset.Y, card, board);
    }

    /// <summary>
    /// The fraction to store for a card DRAWN at <paramref name="corner"/> — read back from where
    /// it is, so what was dropped is what gets stored, clamped as the server would clamp it. A
    /// fraction derived from the raw drag delta stores a position the note was never at.
    /// </summary>
    public static (double X, double Y) FractionOf(
        (double X, double Y) corner, (double Width, double Height) board)
    {
        static double Unit(double value, double extent) =>
            Math.Clamp(value / Math.Max(extent, 1.0), 0.0, 1.0);
        return (Unit(corner.X, board.Width), Unit(corner.Y, board.Height));
    }

    /// <summary>
    /// A few degrees of tilt, derived from the id so a note keeps the same angle for everyone and
    /// across launches — a wall of perfectly square notes reads as a table, not a pinboard.
    /// </summary>
    public static int TiltDegrees(long noteId) => (int)(((noteId % 7) + 7) % 7) - 3;
}
