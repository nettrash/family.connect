using FamilyConnect.App.Logic;
using FamilyConnect.Core;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>The video-message recorder's one big slot (the approved design): Record → Stop → Send, captioned, 64 across.</summary>
public sealed class RecorderLookTests
{
    private static readonly IStringCatalog Say = EnglishCatalog.Instance;

    [Theory]
    [InlineData(RecorderStage.Opening, SlotShape.RecordDisc, "Record", false)]
    [InlineData(RecorderStage.Refused, SlotShape.RecordDisc, "Record", false)]
    [InlineData(RecorderStage.Unavailable, SlotShape.RecordDisc, "Record", false)]
    [InlineData(RecorderStage.Preview, SlotShape.RecordDisc, "Record", true)]
    [InlineData(RecorderStage.Recording, SlotShape.StopSquare, "Stop", true)]
    [InlineData(RecorderStage.Review, SlotShape.SendArrow, "Send", true)]
    public void TheSlotGoesRecordStopSend(RecorderStage stage, SlotShape shape, string caption, bool track)
    {
        Assert.Equal(shape, RecorderLook.Shape(stage));
        Assert.Equal(caption, RecorderLook.Caption(stage, Say));
        Assert.Equal(track, RecorderLook.ShowsTrack(stage));
    }

    [Fact]
    public void TheSlotIsTheBigButtonAndTheOthersAreSmaller()
    {
        Assert.True(RecorderLook.Slot >= 64);
        Assert.True(RecorderLook.Side >= ComposerButton.MinTargetWindowsEpx);
        Assert.True(RecorderLook.Side < RecorderLook.Slot);
        Assert.True(RecorderLook.StopSquare < RecorderLook.Slot / 2);
    }

    /// <summary>
    /// The slot's column sits so the SLOT is centred on Send at any text size: its caption's measured height is taken off,
    /// not a fixed 18 — at 100 % an 11-px caption is about 15 tall, and a fixed 18 put the slot 3 below Send's centre.
    /// </summary>
    [Theory]
    [InlineData(14.6)]
    [InlineData(18)]
    [InlineData(22)]
    [InlineData(33)]
    public void TheSlotIsCentredOnSendWhateverTheCaptionsHeight(double caption)
    {
        const double sendCentre = 40;
        var bottom = RecorderLook.SlotColumnBottom(sendCentre, caption);
        // The column, from the bottom up: the caption, the gap, then the slot's target.
        var slotCentre = bottom + caption + RecorderLook.CaptionGap + RecorderLook.SlotTarget / 2;
        Assert.Equal(sendCentre, slotCentre, 6);
        Assert.Equal(RecorderLook.Slot + 2 * RecorderLook.Halo, RecorderLook.SlotTarget);
    }
}
