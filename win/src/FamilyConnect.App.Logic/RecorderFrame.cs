namespace FamilyConnect.App.Logic;

// Where the video-message recorder's parts stand in the conversation pane (docs/audio-video-messages-2026-10-04.md, S3.3,
// decision 41 of 2026-10-06). On the owner's iPhone the composer showed through the recorder and its controls sat on top
// of the composer's own buttons; the fix, the same on every platform: the composer is not drawn and takes no hits while the
// recorder is open, the controls stand on their own solid bar, the status sits in its own capsule above the circle, and the
// circle is sized so that it never overlaps the status, the controls or any caption, at any size. RoundRecorderLayer
// measures its parts and places them where this says; RecorderFrameTests prove nothing intersects.

/// <summary>A rectangle in the conversation pane's own effective pixels, its origin at the pane's top-left.</summary>
public readonly record struct PaneBox(double X, double Y, double Width, double Height)
{
    public double Right => X + Width;

    public double Bottom => Y + Height;

    /// <summary>Whether the two share any area — touching edges do not.</summary>
    public bool Intersects(PaneBox other) =>
        Width > 0 && Height > 0 && other.Width > 0 && other.Height > 0
        && X < other.Right && other.X < Right && Y < other.Bottom && other.Y < Bottom;

    /// <summary>Whether <paramref name="inner"/> lies wholly inside this one.</summary>
    public bool Contains(PaneBox inner) =>
        inner.X >= X - Epsilon && inner.Y >= Y - Epsilon && inner.Right <= Right + Epsilon && inner.Bottom <= Bottom + Epsilon;

    private const double Epsilon = 1e-9;
}

/// <summary>
/// What the recorder measured of its parts, in effective pixels at the reader's text size: the pane, the status capsule,
/// the reply banner (zero without one), the controls as they are arranged now (zero before they are measured), and the
/// composer's Send button — whose centre the slot stands under, so the pointer does not move sideways (S3.3).
/// </summary>
/// <param name="SendCentreX">Send's centre, from the pane's left edge; NaN when it is not laid out.</param>
public readonly record struct RecorderMeasures(
    double PaneWidth,
    double PaneHeight,
    double StatusWidth,
    double StatusHeight,
    double BannerWidth,
    double BannerHeight,
    double ControlsWidth,
    double ControlsHeight,
    double SendCentreX = double.NaN);

/// <summary>
/// The recorder laid out in one pane: its circle's diameter and the boxes the status capsule, the circle (with its ring),
/// the reply banner, the solid bar and the controls on it occupy. In a column the banner rides in the controls where they
/// have the height for it, and <see cref="Banner"/> is null; where they do not, it stands under the circle beside the bar.
/// </summary>
public sealed record RecorderFrame(RecorderLayout Layout, double Diameter, PaneBox Bar, PaneBox Controls, PaneBox Status, PaneBox Circle, PaneBox? Banner);

/// <summary>The recorder's frame: the solid bar first, then the circle in the room that is left.</summary>
public static class RecorderFrames
{
    /// <summary>The bar's padding round its controls, top and bottom (and in a column, each side).</summary>
    public const double BarPadding = 12;

    /// <summary>The space kept clear at the pane's sides, and between the bar and what stands over it.</summary>
    public const double Gutter = 16;

    /// <summary>The space between the status capsule, the circle and the banner.</summary>
    public const double Gap = 12;

    /// <summary>A column bar is never narrower than this inside its padding — the controls' own width before 2026-10-06.</summary>
    public const double ColumnControlsWidth = 184;

    /// <summary>The space between the lines of the controls (the grid's RowSpacing) — in a column, the banner's line too.</summary>
    public const double ControlsLineGap = 8;

    /// <summary>The status capsule's padding at each side; its line of text gets the rest of the capsule's width.</summary>
    public const double StatusPaddingX = 14;

    /// <summary>The red dot before the clock and the space after it (8 + 8).</summary>
    public const double StatusDotRoom = 16;

    /// <summary>
    /// Where the room runs out — a tiny pane, the largest text and a notice of several lines — the circle keeps at least this
    /// much and the status capsule gives up its last lines instead (it is clipped to <see cref="RecorderFrame.Status"/>): the
    /// camera is what the recorder is for.
    /// </summary>
    public const double ShortestCircle = 96;

    /// <summary>What the ring and its clearance add round the circle, each side (SizeCircle's outer box).</summary>
    public const double RingClearance = RoundVideoRules.RingWidth + 2;

    /// <summary>
    /// Whether the controls, laid out in a single row, are wider than the bar has room for — a narrow pane, large text or
    /// long captions — so the row wraps: the slot under Send on its own line, the other controls on the line above it.
    /// </summary>
    public static bool RowWraps(double paneWidth, double singleRowWidth, double sendCentreX = double.NaN) =>
        singleRowWidth > RowRoom(paneWidth, sendCentreX).Width;

    /// <summary>
    /// The recorder in a pane (S3.3, decision 41): <paramref name="fit"/>'s layout and its diameter as the most the circle
    /// may be, shrunk — below <see cref="RoundVideoRules.SmallestCircle"/> if it must, to nothing at worst — until the
    /// circle overlaps neither the status capsule nor the bar. In a row, the bar runs along the bottom, the controls on it
    /// with the slot under Send, and the circle, the status above it and the banner below it stand just over it; in a
    /// column the bar stands at the trailing edge, the reply banner in its controls where they have the height for it (else
    /// under the circle), and the status and circle are centred in the room beside it.
    /// </summary>
    public static RecorderFrame Frame(RecorderFit fit, RecorderMeasures m)
    {
        var width = Finite(m.PaneWidth);
        var height = Finite(m.PaneHeight);
        var statusHeight = Finite(m.StatusHeight);
        var controlsHeight = Finite(m.ControlsHeight);
        var preferred = Math.Max(0, Finite(fit.Diameter));
        if (fit.Layout == RecorderLayout.Column)
        {
            // ControlsHeight is the column measured WITHOUT the reply banner: the banner rides at its top where the bar has
            // the height for both, and otherwise stands under the circle — a short pane at large text would push the slot
            // off the bottom of the pane, out of reach, before it gave up the banner's line.
            var barWidth = ColumnBarWidth(width, m.ControlsWidth);
            var bar = new PaneBox(width - barWidth, 0, barWidth, height);
            var bannerHeight = Finite(m.BannerHeight);
            var inControls = bannerHeight > 0 && BannerRidesInColumn(height, controlsHeight, bannerHeight);
            var drawnControls = inControls ? controlsHeight + ControlsLineGap + bannerHeight : controlsHeight;
            var controls = new PaneBox(bar.X + BarPadding, Math.Max(BarPadding, (height - drawnControls) / 2),
                Math.Max(0, barWidth - 2 * BarPadding), drawnControls);
            var roomWidth = Math.Max(0, bar.X);
            var under = bannerHeight > 0 && !inControls ? Gap + bannerHeight : 0;
            var (diameter, drawnStatus) = Share(preferred, roomWidth - 2 * Gutter - 2 * RingClearance, height - 2 * Gutter - under, statusHeight);
            statusHeight = drawnStatus;
            var outer = diameter + 2 * RingClearance;
            var stack = statusHeight + Gap + outer + under;
            var top = Math.Max(Gutter, (height - stack) / 2);
            var status = Centred(roomWidth, m.StatusWidth, top, statusHeight);
            var circle = new PaneBox((roomWidth - outer) / 2, status.Bottom + Gap, outer, outer);
            PaneBox? banner = under > 0 ? Centred(roomWidth, m.BannerWidth, circle.Bottom + Gap, bannerHeight) : null;
            return new(RecorderLayout.Column, diameter, bar, controls, status, circle, banner);
        }
        else
        {
            var barHeight = Math.Min(height, controlsHeight + 2 * BarPadding);
            var bar = new PaneBox(0, height - barHeight, width, barHeight);
            var room = RowRoom(width, m.SendCentreX);
            var controls = new PaneBox(room.X, bar.Y + BarPadding, room.Width, controlsHeight);
            var replying = Finite(m.BannerHeight) > 0;
            var bannerHeight = replying ? Finite(m.BannerHeight) : 0;
            // From the bar up: the banner, the circle, the status — so a status that grows a line mid-take grows upward
            // and never moves the circle under the person's eyes.
            var circleBottom = bar.Y - Gutter - (replying ? bannerHeight + Gap : 0);
            var (diameter, drawnStatus) = Share(preferred, width - 2 * Gutter - 2 * RingClearance, circleBottom - Gutter, statusHeight);
            statusHeight = drawnStatus;
            var outer = diameter + 2 * RingClearance;
            var circle = new PaneBox((width - outer) / 2, circleBottom - outer, outer, outer);
            var status = Centred(width, m.StatusWidth, circle.Y - Gap - statusHeight, statusHeight);
            PaneBox? banner = replying ? Centred(width, m.BannerWidth, circleBottom + Gap, bannerHeight) : null;
            return new(RecorderLayout.Row, diameter, bar, controls, status, circle, banner);
        }
    }

    /// <summary>
    /// Whether, in a column, the reply banner rides at the top of the controls: the column measured without it, the line gap
    /// and the banner fit inside the bar's padding in a pane <paramref name="paneHeight"/> tall.
    /// </summary>
    public static bool BannerRidesInColumn(double paneHeight, double controlsHeight, double bannerHeight) =>
        Finite(controlsHeight) + ControlsLineGap + Finite(bannerHeight) + 2 * BarPadding <= Finite(paneHeight);

    /// <summary>
    /// The span the status capsule (and the circle) stand across, known before the frame so the capsule is measured at the
    /// width it is drawn at: the whole pane in a row; in a column, the room beside the bar, whose width the controls set.
    /// </summary>
    public static double StatusSpan(RecorderLayout layout, double paneWidth, double controlsWidth)
    {
        var width = Finite(paneWidth);
        return layout == RecorderLayout.Column ? Math.Max(0, width - ColumnBarWidth(width, controlsWidth)) : width;
    }

    private static double ColumnBarWidth(double width, double controlsWidth) =>
        Math.Min(width, Math.Max(ColumnControlsWidth, Finite(controlsWidth)) + 2 * BarPadding);

    /// <summary>
    /// How wide the status capsule's line of text may be in a span (the pane in a row, the room beside the bar in a column):
    /// the span less its gutters, the capsule's padding and, while recording, the red dot — so a long sentence wraps inside
    /// the capsule, which is measured with it, instead of running out of it.
    /// </summary>
    public static double StatusLineWidth(double spanWidth, bool dot) =>
        Math.Max(0, Finite(spanWidth) - 2 * Gutter - 2 * StatusPaddingX - (dot ? StatusDotRoom : 0));

    /// <summary>
    /// What of the status line lays the recorder out again when it changes (with the stage, the notice and the controls):
    /// the line itself — "Starting camera…" becoming "Not recording", a review's length landing — but not the clock that
    /// ticks while recording, whose digits keep one width.
    /// </summary>
    public static string? LineThatLaysOut(RecorderStage stage, string line) => stage == RecorderStage.Recording ? null : line;

    /// <summary>
    /// The circle's diameter and the status capsule's drawn height in <paramref name="room"/> — the height the two and the gap
    /// between them share: the preferred circle where it fits beside the whole status; else the largest that does, down to
    /// <see cref="ShortestCircle"/>; and below that the circle keeps the shortest (or what the width allows) and the status
    /// is cut to what is left.
    /// </summary>
    private static (double Diameter, double Status) Share(double preferred, double widthLimit, double room, double status)
    {
        var limit = Math.Max(0, Math.Min(preferred, widthLimit));
        var beside = room - status - Gap - 2 * RingClearance;
        if (beside >= Math.Min(limit, ShortestCircle))
        {
            return (Math.Floor(Math.Min(limit, beside)), status);
        }
        var diameter = Math.Floor(Math.Max(0, Math.Min(Math.Min(limit, ShortestCircle), room - Gap - 2 * RingClearance)));
        return (diameter, Math.Max(0, Math.Min(status, room - Gap - 2 * RingClearance - diameter)));
    }

    /// <summary>The span a row's controls get: the gutter on the left; on the right, so the slot's centre is Send's.</summary>
    private static PaneBox RowRoom(double paneWidth, double sendCentreX)
    {
        var width = Finite(paneWidth);
        var right = double.IsFinite(sendCentreX)
            ? Math.Clamp(width - sendCentreX - RecorderLook.SlotTarget / 2, BarPadding, Math.Max(BarPadding, width / 2))
            : Gutter;
        return new PaneBox(Gutter, 0, Math.Max(0, width - Gutter - right), 0);
    }

    /// <summary>A box centred across a span, never wider than the span less its gutters.</summary>
    private static PaneBox Centred(double spanWidth, double width, double y, double height)
    {
        var drawn = Math.Max(0, Math.Min(Finite(width), spanWidth - 2 * Gutter));
        return new PaneBox((spanWidth - drawn) / 2, y, drawn, height);
    }

    private static double Finite(double value) => double.IsFinite(value) ? Math.Max(0, value) : 0;
}
