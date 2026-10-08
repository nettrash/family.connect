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
    /// The slot's target is the disc and its halo. (Until 2026-10-06 its column was also lifted so the slot sat centred on
    /// Send's row; the controls now stand on their own bar — decision 41 — with the slot under Send's centre across, and
    /// RecorderFrameTests holds where it stands.)
    /// </summary>
    [Fact]
    public void TheSlotsTargetIsItsDiscAndHalo() => Assert.Equal(RecorderLook.Slot + 2 * RecorderLook.Halo, RecorderLook.SlotTarget);
}
