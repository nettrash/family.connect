using FamilyConnect.App.Logic;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// THE RECORDER'S PARTS NEVER OVERLAP (decision 41, 2026-10-06). On the owner's iPhone the composer showed through the
/// video recorder and its controls sat on the composer's buttons, the status floated over messages and the circle competed
/// with the chat. Windows' layer is switched off until the trials, so the bounds are proved here, from what the layer would
/// measure: the status capsule, the circle with its ring, the reply banner, the solid bar and the controls on it — the
/// captions under the round buttons are inside the controls' measured height — never intersect, at phone-narrow widths
/// (320, 375, 430), in a short "landscape" pane, in a desktop pane, and at 100 % to 225 % text.
/// </summary>
public sealed class RecorderFrameTests
{
    /// <summary>What the layer measures at a text scale: Windows' 11-epx captions, the 14-epx status line, the round buttons.</summary>
    private sealed record Parts(double Scale, int BelowLines, bool Replying, int RoundButtons)
    {
        public double Caption => Math.Ceiling(15 * Scale);

        /// <summary>A captioned button's column: 56 at least, or as wide as its caption — "Camera" is about 36 at 11 epx.</summary>
        public double CaptionedColumn => Math.Max(56, Math.Ceiling(36 * Scale));

        public double StatusHeight => 12 + Math.Ceiling(20 * Scale) + BelowLines * (2 + Math.Ceiling(16 * Scale));

        public double StatusWidth => Math.Min(344, 24 + Math.Ceiling(150 * Scale));

        public double BannerHeight => Replying ? Math.Max(40, 12 + Math.Ceiling(20 * Scale)) : 0;

        public double BannerWidth => Replying ? 420 : 0;

        public double SlotColumnHeight => RecorderLook.SlotTarget + RecorderLook.CaptionGap + Caption;

        public double SideColumnHeight => RecorderLook.Side + RecorderLook.CaptionGap + Caption;

        /// <summary>One row: the 6-epx lead, the round buttons in their captioned columns, the slot, 8 between each.</summary>
        public double SingleRowWidth => 6 + RoundButtons * CaptionedColumn + RecorderLook.SlotTarget + RoundButtons * 8;

        public double WrappedWidth => 6 + Math.Max(RoundButtons * CaptionedColumn + (RoundButtons - 1) * 8, RecorderLook.SlotTarget);

        public double WrappedHeight => SideColumnHeight + 8 + SlotColumnHeight;

        /// <summary>
        /// The column's rows as the layer measures them — WITHOUT the reply banner, which the frame places: the leading row,
        /// the middle row, the slot, 8 between each.
        /// </summary>
        public double ColumnHeight => 2 * SideColumnHeight + 8 + 8 + SlotColumnHeight;

        public double ColumnWidth => Math.Max(RecorderFrames.ColumnControlsWidth, 2 * CaptionedColumn + 8);
    }

    private static readonly double[] Scales = [1.0, 1.5, 2.0, 2.25];

    private static (RecorderFrame Frame, double ControlsWidth) Lay(double width, double height, Parts parts, double sendInset = 40)
    {
        var fit = RoundVideoRules.Fit(width, height, parts.BannerHeight);
        var send = width - sendInset;
        double controlsWidth, controlsHeight;
        if (fit.Layout == RecorderLayout.Column)
        {
            (controlsWidth, controlsHeight) = (parts.ColumnWidth, parts.ColumnHeight);
        }
        else if (RecorderFrames.RowWraps(width, parts.SingleRowWidth, send))
        {
            (controlsWidth, controlsHeight) = (parts.WrappedWidth, parts.WrappedHeight);
        }
        else
        {
            (controlsWidth, controlsHeight) = (parts.SingleRowWidth, parts.SlotColumnHeight);
        }
        var frame = RecorderFrames.Frame(fit, new RecorderMeasures(
            width, height, parts.StatusWidth, parts.StatusHeight, parts.BannerWidth, parts.BannerHeight,
            controlsWidth, controlsHeight, send));
        return (frame, controlsWidth);
    }

    private static void AssertApart(RecorderFrame frame, double width, double height, double controlsWidth, string what)
    {
        var pane = new PaneBox(0, 0, width, height);
        var parts = new List<(string Name, PaneBox Box)> { ("status", frame.Status), ("circle", frame.Circle), ("bar", frame.Bar) };
        if (frame.Banner is { } banner)
        {
            parts.Add(("banner", banner));
        }
        for (var i = 0; i < parts.Count; i++)
        {
            Assert.True(pane.Contains(parts[i].Box), $"{what}: {parts[i].Name} {parts[i].Box} leaves the pane");
            for (var j = i + 1; j < parts.Count; j++)
            {
                Assert.False(parts[i].Box.Intersects(parts[j].Box), $"{what}: {parts[i].Name} {parts[i].Box} overlaps {parts[j].Name} {parts[j].Box}");
            }
        }
        // The controls — every round button and every caption under it — stand on the bar, whole, and nothing else does.
        Assert.True(frame.Bar.Contains(frame.Controls), $"{what}: controls {frame.Controls} off the bar {frame.Bar}");
        Assert.True(frame.Controls.Width >= controlsWidth - 1e-9, $"{what}: controls cut to {frame.Controls.Width} of {controlsWidth}");
        foreach (var (name, box) in parts.Where(part => part.Name != "bar"))
        {
            Assert.False(box.Intersects(frame.Controls), $"{what}: {name} {box} overlaps the controls {frame.Controls}");
        }
        // The circle box is the circle and its ring, exactly.
        Assert.Equal(frame.Diameter + 2 * RecorderFrames.RingClearance, frame.Circle.Width, 9);
        Assert.Equal(frame.Circle.Width, frame.Circle.Height, 9);
    }

    /// <summary>Phone-narrow panes, portrait: 320, 375 and 430 wide, every text size, a reply or none, two status lines.</summary>
    [Theory]
    [InlineData(320, 568)]
    [InlineData(375, 667)]
    [InlineData(430, 932)]
    [InlineData(320, 480)]
    [InlineData(375, 520)]
    [InlineData(700, 900)]
    [InlineData(1000, 1000)]
    public void NothingOverlapsInARow(double width, double height)
    {
        foreach (var scale in Scales)
        {
            foreach (var replying in new[] { false, true })
            {
                foreach (var below in new[] { 0, 1, 3 })
                {
                    foreach (var buttons in new[] { 1, 3 })
                    {
                        var parts = new Parts(scale, below, replying, buttons);
                        var (frame, controlsWidth) = Lay(width, height, parts);
                        Assert.Equal(RecorderLayout.Row, frame.Layout);
                        AssertApart(frame, width, height, controlsWidth, $"{width}x{height} at {scale:P0}, {parts}");
                    }
                }
            }
        }
    }

    /// <summary>A short pane — a phone or a small window turned "landscape" — stands the bar at the trailing edge.</summary>
    [Theory]
    [InlineData(568, 320)]
    [InlineData(667, 375)]
    [InlineData(932, 430)]
    [InlineData(1000, 479)]
    [InlineData(480, 400)]
    [InlineData(420, 360)]
    public void NothingOverlapsInAColumn(double width, double height)
    {
        // Every text size, the largest in the shortest pane too: until 2026-10-06 the column kept the banner in the controls
        // whatever it cost, and at 568 × 320 and 200 % with a reply the slot ended 24 below the pane, out of reach.
        foreach (var scale in Scales)
        {
            foreach (var replying in new[] { false, true })
            {
                foreach (var below in new[] { 0, 1, 2 })
                {
                    var parts = new Parts(scale, below, replying, 3);
                    var (frame, controlsWidth) = Lay(width, height, parts);
                    var what = $"{width}x{height} at {scale:P0}, {parts}";
                    Assert.Equal(RecorderLayout.Column, frame.Layout);
                    AssertApart(frame, width, height, controlsWidth, what);
                    // The banner rides at the column's top where the bar has the height for it, and stands under the circle
                    // where it has not; without a reply there is none.
                    var rides = replying && RecorderFrames.BannerRidesInColumn(height, parts.ColumnHeight, parts.BannerHeight);
                    Assert.True((frame.Banner is null) == (!replying || rides), $"{what}: banner {frame.Banner}");
                    Assert.Equal(parts.ColumnHeight + (rides ? 8 + parts.BannerHeight : 0), frame.Controls.Height, 9);
                    // The controls — the slot last — whole inside the bar's padding, so the slot is always in reach.
                    Assert.True(frame.Controls.Y >= RecorderFrames.BarPadding - 1e-9 && frame.Controls.Bottom <= height - RecorderFrames.BarPadding + 1e-9,
                        $"{what}: controls {frame.Controls}");
                    // The status, the circle and (when it is there) the banner are centred top to bottom beside the bar, a
                    // gutter clear of both edges.
                    Assert.True(frame.Status.Y >= RecorderFrames.Gutter - 1e-9, $"{what}: status at {frame.Status.Y}");
                    Assert.Equal(frame.Status.Y, height - (frame.Banner?.Bottom ?? frame.Circle.Bottom), 6);
                    Assert.True(frame.Banner is not { } under || under.Right <= frame.Bar.X - RecorderFrames.Gutter + 1e-9, $"{what}: banner reaches the bar");
                }
            }
        }
    }

    /// <summary>Where there is room the circle is the plan's (Fit's) size; where there is not it shrinks, never overlaps.</summary>
    [Fact]
    public void TheCircleKeepsItsSizeWhereThereIsRoomAndShrinksWhereThereIsNot()
    {
        var roomy = Lay(1000, 1000, new Parts(1, 0, false, 3)).Frame;
        Assert.Equal(RoundVideoRules.LargestCircle, roomy.Diameter);
        // A 320 × 480 pane at 200 % with a reply and a wrapped row: Fit asks for more than there is room for — the old
        // layout drew it anyway, over the status — and the frame draws less, still a circle.
        var parts = new Parts(2, 2, true, 3);
        var tight = Lay(320, 480, parts).Frame;
        var asked = RoundVideoRules.Fit(320, 480, parts.BannerHeight).Diameter;
        Assert.True(tight.Diameter < asked, $"{tight.Diameter} of {asked}");
        Assert.True(tight.Diameter > 0);
    }

    /// <summary>
    /// The old Row layout's arithmetic (before 2026-10-06), kept to show what the frame fixed: the stack — status, circle at
    /// Fit's diameter, banner, 12 apart — grew up from 16 over the slot's column on the composer's own row, sized without the
    /// status's height, the text size or a bar. At 320 × 480 and 200 % with a reply and a two-line notice its status line was
    /// pushed off the top of the pane; the frame keeps every part inside and apart.
    /// </summary>
    [Fact]
    public void TheOldLayoutOverflowedWhereTheFrameDoesNot()
    {
        const double width = 320, height = 480;
        var parts = new Parts(2, 2, true, 3);
        var oldDiameter = RoundVideoRules.Fit(width, height, parts.BannerHeight).Diameter;
        var oldStackBottom = height - (parts.SlotColumnHeight + 16);
        var oldCircleTop = oldStackBottom - parts.BannerHeight - 12 - (oldDiameter + 2 * RecorderFrames.RingClearance);
        var oldStatusTop = oldCircleTop - 12 - parts.StatusHeight;
        Assert.True(oldStatusTop < 0, $"the old status line began at {oldStatusTop}");
        var (frame, controlsWidth) = Lay(width, height, parts);
        AssertApart(frame, width, height, controlsWidth, "320x480 at 200 %");
        Assert.True(frame.Status.Y >= RecorderFrames.Gutter);
        Assert.True(frame.Diameter < oldDiameter, $"{frame.Diameter} of {oldDiameter}");
    }

    /// <summary>
    /// Where the room runs out — 320 × 480 at 225 %, a three-line notice and a reply — the camera keeps a circle of at least
    /// <see cref="RecorderFrames.ShortestCircle"/> and the status capsule is cut to what is left, still apart from it.
    /// </summary>
    [Fact]
    public void WhereTheRoomRunsOutTheCircleKeepsItsShortestAndTheStatusIsCut()
    {
        var parts = new Parts(2.25, 3, true, 3);
        var (frame, controlsWidth) = Lay(320, 480, parts);
        AssertApart(frame, 320, 480, controlsWidth, "320x480 at 225 %");
        Assert.Equal(RecorderFrames.ShortestCircle, frame.Diameter);
        Assert.True(frame.Status.Height < parts.StatusHeight, $"{frame.Status.Height}");
        Assert.True(frame.Status.Height > 0);
        // With a one-line status and no reply the same pane draws the whole status and a larger circle.
        var roomier = Lay(320, 480, new Parts(2.25, 0, false, 3)).Frame;
        Assert.Equal(new Parts(2.25, 0, false, 3).StatusHeight, roomier.Status.Height);
        Assert.True(roomier.Diameter > RecorderFrames.ShortestCircle, $"{roomier.Diameter}");
    }

    /// <summary>The slot stands under Send: the controls end where a 72-epx slot centred on Send would.</summary>
    [Fact]
    public void TheSlotStandsUnderSend()
    {
        var frame = RecorderFrames.Frame(new RecorderFit(320, RecorderLayout.Row),
            new RecorderMeasures(1000, 900, 200, 32, 0, 0, 270, 89, SendCentreX: 950));
        Assert.Equal(950 + RecorderLook.SlotTarget / 2, frame.Controls.Right, 9);
        Assert.Equal(RecorderFrames.Gutter, frame.Controls.X, 9);
        // Not laid out yet: the gutter on both sides.
        var unknown = RecorderFrames.Frame(new RecorderFit(320, RecorderLayout.Row), new RecorderMeasures(1000, 900, 200, 32, 0, 0, 270, 89));
        Assert.Equal(1000 - RecorderFrames.Gutter, unknown.Controls.Right, 9);
    }

    /// <summary>The bar is solid along the whole bottom of the pane, as tall as the controls and their padding.</summary>
    [Fact]
    public void TheBarRunsTheWidthOfThePaneAtItsBottom()
    {
        var frame = RecorderFrames.Frame(new RecorderFit(320, RecorderLayout.Row), new RecorderMeasures(800, 900, 200, 32, 0, 0, 270, 89, 760));
        Assert.Equal(new PaneBox(0, 900 - 89 - 2 * RecorderFrames.BarPadding, 800, 89 + 2 * RecorderFrames.BarPadding), frame.Bar);
        // The controls stand inside its padding, top and bottom.
        Assert.Equal(frame.Bar.Y + RecorderFrames.BarPadding, frame.Controls.Y, 9);
        Assert.Equal(frame.Bar.Bottom - RecorderFrames.BarPadding, frame.Controls.Bottom, 9);
        // And the circle and status stand just over it, the circle centred.
        Assert.Equal(frame.Bar.Y - RecorderFrames.Gutter, frame.Circle.Bottom, 9);
        Assert.Equal(400, frame.Circle.X + frame.Circle.Width / 2, 9);
        Assert.Equal(frame.Circle.Y - RecorderFrames.Gap, frame.Status.Bottom, 9);
    }

    /// <summary>A status that grows a line mid-take grows upward: the circle stays where it was.</summary>
    [Fact]
    public void AGrowingStatusNeverMovesTheCircle()
    {
        var fit = new RecorderFit(320, RecorderLayout.Row);
        var one = RecorderFrames.Frame(fit, new RecorderMeasures(800, 900, 200, 32, 0, 0, 270, 89, 760));
        var two = RecorderFrames.Frame(fit, new RecorderMeasures(800, 900, 200, 50, 0, 0, 270, 89, 760));
        Assert.Equal(one.Circle, two.Circle);
        Assert.True(two.Status.Y < one.Status.Y);
    }

    /// <summary>A single row wraps where it would not fit beside Send: the slot on its own line.</summary>
    [Fact]
    public void TheRowWrapsWhenItWouldNotFit()
    {
        Assert.False(RecorderFrames.RowWraps(800, 270, 760));
        Assert.True(RecorderFrames.RowWraps(320, 342, 280));
        // Room is the pane less the gutter on the left and, on the right, what puts the slot's centre on Send.
        var room = 375 - RecorderFrames.Gutter - (375 - 300 - RecorderLook.SlotTarget / 2);
        Assert.False(RecorderFrames.RowWraps(375, room, 300));
        Assert.True(RecorderFrames.RowWraps(375, room + 1, 300));
        // Send hard against the edge: the bar's padding is kept on the right all the same.
        Assert.False(RecorderFrames.RowWraps(375, 375 - RecorderFrames.Gutter - RecorderFrames.BarPadding, 370));
        Assert.True(RecorderFrames.RowWraps(375, 375 - RecorderFrames.Gutter - RecorderFrames.BarPadding + 1, 370));
    }

    /// <summary>
    /// A short "landscape" pane: at 100 % the reply banner rides at the top of the column; at 200 % the column would run off
    /// the pane with it, so it stands under the circle instead and the slot stays whole on the bar.
    /// </summary>
    [Fact]
    public void AShortPaneGivesUpTheBannersPlaceInTheColumnNotTheSlot()
    {
        var small = new Parts(1, 0, true, 3);
        var (riding, _) = Lay(568, 320, small);
        Assert.Null(riding.Banner);
        Assert.Equal(small.ColumnHeight + RecorderFrames.ControlsLineGap + small.BannerHeight, riding.Controls.Height, 9);
        var large = new Parts(2, 0, true, 3);
        var (under, controlsWidth) = Lay(568, 320, large);
        AssertApart(under, 568, 320, controlsWidth, "568x320 at 200 %");
        Assert.NotNull(under.Banner);
        Assert.Equal(under.Circle.Bottom + RecorderFrames.Gap, under.Banner!.Value.Y, 9);
        Assert.Equal(large.ColumnHeight, under.Controls.Height, 9);
        Assert.True(under.Controls.Bottom <= 320 - RecorderFrames.BarPadding, $"{under.Controls}");
        Assert.True(RecorderFrames.BannerRidesInColumn(320, 200, 40));
        Assert.True(RecorderFrames.BannerRidesInColumn(320, 320 - 24 - 8 - 40, 40));
        Assert.False(RecorderFrames.BannerRidesInColumn(320, 320 - 24 - 8 - 40 + 1, 40));
    }

    /// <summary>
    /// The status is measured across the span it is drawn across — the room beside a column's bar, not the whole pane, where
    /// a notice would wrap fewer times than it is drawn and lose its last line to the capsule's clip — and its line of text
    /// wraps inside the capsule.
    /// </summary>
    [Fact]
    public void TheStatusIsMeasuredAcrossTheSpanItIsDrawnIn()
    {
        foreach (var (width, height) in new[] { (568.0, 320.0), (420.0, 360.0), (1000.0, 479.0), (375.0, 667.0), (1000.0, 1000.0) })
        {
            var parts = new Parts(1.5, 1, false, 3);
            var (frame, controlsWidth) = Lay(width, height, parts);
            var span = RecorderFrames.StatusSpan(frame.Layout, width, controlsWidth);
            Assert.Equal(frame.Layout == RecorderLayout.Column ? frame.Bar.X : width, span, 9);
            // The capsule drawn never wider than the span less its gutters, and centred in it.
            Assert.True(frame.Status.Width <= span - 2 * RecorderFrames.Gutter + 1e-9);
            Assert.Equal(span / 2, frame.Status.X + frame.Status.Width / 2, 9);
        }
        Assert.Equal(400 - 32 - 28, RecorderFrames.StatusLineWidth(400, dot: false), 9);
        Assert.Equal(400 - 32 - 28 - 16, RecorderFrames.StatusLineWidth(400, dot: true), 9);
        Assert.Equal(0, RecorderFrames.StatusLineWidth(40, dot: true));
    }

    /// <summary>
    /// A new line lays the recorder out again — "Starting camera…" turning into a longer sentence would otherwise be cut to
    /// the old capsule — but the clock ticking while recording does not: every second would re-measure the whole layer.
    /// </summary>
    [Fact]
    public void ANewStatusLineLaysOutAndTheClockDoesNot()
    {
        Assert.Equal(RecorderFrames.LineThatLaysOut(RecorderStage.Recording, "0:01"), RecorderFrames.LineThatLaysOut(RecorderStage.Recording, "0:42"));
        Assert.NotEqual(RecorderFrames.LineThatLaysOut(RecorderStage.Preview, "Starting camera…"), RecorderFrames.LineThatLaysOut(RecorderStage.Preview, "Not recording"));
        Assert.NotEqual(RecorderFrames.LineThatLaysOut(RecorderStage.Review, "Video message · 0:09"), RecorderFrames.LineThatLaysOut(RecorderStage.Review, "Video message · 0:10"));
        Assert.Equal("Starting camera…", RecorderFrames.LineThatLaysOut(RecorderStage.Opening, "Starting camera…"));
    }

    [Fact]
    public void BoxesTouchingAtAnEdgeDoNotOverlap()
    {
        var a = new PaneBox(0, 0, 10, 10);
        Assert.False(a.Intersects(new PaneBox(10, 0, 10, 10)));
        Assert.False(a.Intersects(new PaneBox(0, 10, 10, 10)));
        Assert.False(new PaneBox(10, 0, 10, 10).Intersects(a));
        Assert.False(new PaneBox(0, 10, 10, 10).Intersects(a));
        Assert.True(a.Intersects(new PaneBox(9.5, 9.5, 10, 10)));
        Assert.False(a.Intersects(new PaneBox(5, 5, 0, 10)));
        Assert.True(a.Contains(new PaneBox(0, 0, 10, 10)));
        Assert.False(a.Contains(new PaneBox(0, 0, 10.5, 10)));
    }

    /// <summary>A pane not yet laid out draws nothing at all rather than something negative.</summary>
    [Fact]
    public void APaneNotYetLaidOutIsHarmless()
    {
        var frame = RecorderFrames.Frame(RoundVideoRules.Fit(0, 0, 0), new RecorderMeasures(0, 0, 0, 0, 0, 0, 0, 0));
        Assert.Equal(0, frame.Diameter);
        Assert.True(frame.Bar.Width >= 0 && frame.Bar.Height >= 0);
        var nan = RecorderFrames.Frame(new RecorderFit(double.NaN, RecorderLayout.Row), new RecorderMeasures(double.NaN, 900, 0, 0, 0, 0, 0, 0));
        Assert.Equal(0, nan.Diameter);
    }
}
