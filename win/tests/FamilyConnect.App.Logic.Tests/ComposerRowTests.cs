using FamilyConnect.App.Logic;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// The message row and the wide window (docs/composer-and-wide-layout-2026-10-20.md, issue #106): which glyphs sit inside
/// the field for each chat, the column and bubble widths, and the least window that keeps the field a run of text.
/// </summary>
public sealed class ComposerRowTests
{
    private static readonly AssistantDto Talks = new(99, "Assistant", "@ai", Processor: "Microsoft — Azure OpenAI");
    private static readonly AssistantDto Draws = new(99, "Assistant", "@ai", Images: true, Processor: "Microsoft — Azure OpenAI");

    /// <summary>The family and direct chats: the stickers always — empty or typing — and the video button only where S1.4 shows it.</summary>
    [Fact]
    public void TheStickersSitInTheFieldInEveryChatAndTheVideoButtonBesideThem()
    {
        Assert.Equal([FieldGlyph.Sticker], ComposerRow.InField("family", editing: false, packOffered: true, Talks, videoDoor: false));
        Assert.Equal([FieldGlyph.Sticker, FieldGlyph.VideoDoor], ComposerRow.InField("family", false, true, Talks, videoDoor: true));
        Assert.Equal([FieldGlyph.Sticker, FieldGlyph.VideoDoor], ComposerRow.InField("direct", false, true, null, videoDoor: true));
        Assert.Equal([FieldGlyph.VideoDoor], ComposerRow.InField("direct", false, packOffered: false, null, videoDoor: true));
        Assert.Empty(ComposerRow.InField("direct", false, packOffered: false, null, videoDoor: false));
    }

    /// <summary>The assistant's chat: the paintbrush stays a button, inside the field beside the stickers, where it draws.</summary>
    [Fact]
    public void TheAssistantsChatCarriesThePaintbrushBesideTheStickers()
    {
        Assert.Equal([FieldGlyph.Sticker, FieldGlyph.Picture], ComposerRow.InField("ai", false, true, Draws, videoDoor: false));
        Assert.Equal([FieldGlyph.Sticker], ComposerRow.InField("ai", false, true, Talks, videoDoor: false));
        // A server that names no processor offers no paintbrush (docs/protocol.md, "Consenting to the assistant").
        Assert.Equal([FieldGlyph.Sticker], ComposerRow.InField("ai", false, true, Draws with { Processor = null }, videoDoor: false));
        // No @ai button in the family chat any more: the paperclip's menu and the @ strip reach it.
        Assert.DoesNotContain(FieldGlyph.Picture, ComposerRow.InField("family", false, true, Draws, videoDoor: false));
    }

    /// <summary>An edit carries no sticker and no paintbrush — a sticker is its own message, and an edit cannot become a request.</summary>
    [Fact]
    public void AnEditCarriesNothingInTheField()
    {
        Assert.Empty(ComposerRow.InField("family", editing: true, true, Talks, videoDoor: false));
        Assert.Empty(ComposerRow.InField("ai", editing: true, true, Draws, videoDoor: false));
        Assert.Empty(ComposerRow.InField(null, false, true, Talks, videoDoor: false));
    }

    /// <summary>The thread's field: the stickers where it can send; its Send disc lit only with a root and something to say.</summary>
    [Fact]
    public void TheThreadsFieldIsTheSameFieldAndItsSendNeedsWords()
    {
        Assert.Equal([FieldGlyph.Sticker], ComposerRow.InThreadField(hasRoot: true, packOffered: true));
        Assert.Empty(ComposerRow.InThreadField(hasRoot: false, packOffered: true));
        Assert.Empty(ComposerRow.InThreadField(hasRoot: true, packOffered: false));
        Assert.True(ComposerRow.ThreadSendEnabled(hasRoot: true, "Sunday works"));
        Assert.False(ComposerRow.ThreadSendEnabled(hasRoot: true, "  \n "));
        Assert.False(ComposerRow.ThreadSendEnabled(hasRoot: true, null));
        Assert.False(ComposerRow.ThreadSendEnabled(hasRoot: false, "Sunday works"));
    }

    /// <summary>The field makes one 44-epx target of room per glyph it carries, so text never runs under a button.</summary>
    [Fact]
    public void TheFieldMakesRoomForEveryGlyph()
    {
        Assert.Equal(44, ComposerRow.TargetEpx);
        Assert.Equal(0, ComposerRow.TrailingRoom([]));
        Assert.Equal(44, ComposerRow.TrailingRoom([FieldGlyph.Sticker]));
        Assert.Equal(88, ComposerRow.TrailingRoom([FieldGlyph.Sticker, FieldGlyph.Picture]));
        Assert.True(ComposerRow.TrailingRoom(ComposerRow.InField("ai", false, true, Draws, false)) <= ConversationColumn.MostFieldGlyphs * ComposerRow.TargetEpx);
        Assert.True(ComposerRow.TrailingRoom(ComposerRow.InField("family", false, true, Talks, true)) <= ConversationColumn.MostFieldGlyphs * ComposerRow.TargetEpx);
    }

    /// <summary>The column: min(pane − 32, 1024), centred — the composer exactly as wide as the messages above it.</summary>
    [Fact]
    public void TheColumnIsThePaneLessItsInsetsUpTo1024()
    {
        Assert.Equal(1024, ConversationColumn.Width(1620));
        Assert.Equal(1024, ConversationColumn.Width(1056));
        Assert.Equal(968, ConversationColumn.Width(1000));
        Assert.Equal(420, ConversationColumn.Width(452));
        Assert.Equal(0, ConversationColumn.Width(20));
        Assert.Equal(0, ConversationColumn.Width(0));
    }

    /// <summary>The jump button: 12 epx inside the column's trailing edge — 28 from the pane's while the column fills it, then further in.</summary>
    [Fact]
    public void TheJumpButtonStandsInsideTheColumn()
    {
        Assert.Equal(28, ConversationColumn.JumpInset(452));
        Assert.Equal(28, ConversationColumn.JumpInset(1056));
        Assert.Equal(310, ConversationColumn.JumpInset(1620));
        Assert.Equal(22, ConversationColumn.JumpInset(20));
        Assert.Equal(12, ConversationColumn.JumpInset(0));
        foreach (var pane in new double[] { 452, 700, 1056, 1300, 1620, 2400 })
        {
            var columnRight = ((pane - ConversationColumn.Width(pane)) / 2) + ConversationColumn.Width(pane);
            Assert.Equal(columnRight - ConversationColumn.JumpInsideEpx, pane - ConversationColumn.JumpInset(pane));
        }
    }

    /// <summary>A bubble: min(column − 48, 560) — and the margin-plus-cap the view draws it with is exactly that.</summary>
    [Fact]
    public void ABubbleIsTheColumnLess48UpTo560()
    {
        Assert.Equal(560, ConversationColumn.BubbleMax(1024));
        Assert.Equal(560, ConversationColumn.BubbleMax(608));
        Assert.Equal(500, ConversationColumn.BubbleMax(548));
        Assert.Equal(0, ConversationColumn.BubbleMax(30));
        foreach (var column in new double[] { 0, 30, 200, 420, 560, 608, 700, 968, 1024 })
        {
            var drawn = Math.Max(0, Math.Min(column - ConversationColumn.BubbleSpacerEpx, ConversationColumn.BubbleMaxEpx));
            Assert.Equal(ConversationColumn.BubbleMax(column), drawn);
        }
    }

    /// <summary>The field's floor is arithmetic: at the least pane the run of text is exactly 200 epx, with two glyphs in the field.</summary>
    [Fact]
    public void TheLeastPaneLeavesTheFieldItsRunOfText()
    {
        Assert.Equal(452, ConversationColumn.MinPaneEpx);
        var column = ConversationColumn.Width(ConversationColumn.MinPaneEpx);
        var field = column - (2 * ConversationColumn.CardChromeEpx) - (2 * ComposerRow.TargetEpx) - (2 * ConversationColumn.RowSpacingEpx);
        var run = field - ConversationColumn.FieldChromeEpx - (ConversationColumn.MostFieldGlyphs * ComposerRow.TargetEpx);
        Assert.Equal(ConversationColumn.MinTextRunEpx, run);
        Assert.Equal(777, ConversationColumn.MinimumWindowWidthEpx);
    }

    /// <summary>
    /// Wide: the list as drawn, the thread beside. Narrower: the list gives first, down to its least; then the thread goes
    /// over the conversation instead of beside it — and the pane never drops under its floor at the least window.
    /// </summary>
    [Fact]
    public void TheListNarrowsThenTheThreadGoesOverTheConversation()
    {
        Assert.Equal(new PaneArrangement(300, ThreadBeside: false, 1619), ConversationColumn.Arrange(1920, threadOpen: false));
        Assert.Equal(new PaneArrangement(300, ThreadBeside: true, 1219), ConversationColumn.Arrange(1920, threadOpen: true));
        // 1150 wide: the list narrows to keep the pane its floor beside the thread.
        var tight = ConversationColumn.Arrange(1150, threadOpen: true);
        Assert.True(tight.ThreadBeside);
        Assert.Equal(1150 - 1 - 400 - 452, tight.ListWidth);
        Assert.Equal(452, tight.PaneWidth);
        // Too narrow for both: the thread goes over the conversation, and the list keeps what it can.
        var over = ConversationColumn.Arrange(1000, threadOpen: true);
        Assert.False(over.ThreadBeside);
        Assert.Equal(300, over.ListWidth);
        Assert.Equal(699, over.PaneWidth);
        // The least window's content: the rail and the frame aside, the list at its least and the pane at its floor.
        var least = ConversationColumn.MinimumWindowWidthEpx - ConversationColumn.RailEpx - ConversationColumn.FrameEpx;
        foreach (var thread in new[] { false, true })
        {
            var arranged = ConversationColumn.Arrange(least, thread);
            Assert.Equal(ConversationColumn.ListMinEpx, arranged.ListWidth);
            Assert.Equal(ConversationColumn.MinPaneEpx, arranged.PaneWidth);
            Assert.False(arranged.ThreadBeside);
        }
        // Every width from the least window up keeps the pane at its floor or above.
        for (var view = least; view <= 2400; view += 7)
        {
            Assert.True(ConversationColumn.Arrange(view, true).PaneWidth >= ConversationColumn.MinPaneEpx);
            Assert.True(ConversationColumn.Arrange(view, false).PaneWidth >= ConversationColumn.MinPaneEpx);
        }
    }
}
