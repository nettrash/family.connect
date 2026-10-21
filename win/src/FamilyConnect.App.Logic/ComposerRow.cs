using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic;

// The message row and the wide window (docs/composer-and-wide-layout-2026-10-20.md, issue #106): one row of three
// controls — the paperclip, the field, the slot — with the field's own glyphs inside its trailing edge; the conversation
// as a column no wider than 1024 epx, centred, its bubbles no wider than 560; and the least window that still leaves the
// field a run of text to type in. What is decided here is WHICH glyphs and HOW WIDE; the view only draws it.

/// <summary>A control inside the field's trailing edge, in the order it is drawn, leading to trailing.</summary>
public enum FieldGlyph
{
    /// <summary>The family's stickers (☺): wherever a sticker can be sent, empty or typing — it never moves under the pointer.</summary>
    Sticker,

    /// <summary>"Ask for a picture" (the paintbrush): the assistant's own chat, where it draws — still a button, beside the stickers.</summary>
    Picture,

    /// <summary>The video button (S1.4 of docs/audio-video-messages-2026-10-04.md): only while the field is empty and the slot a microphone.</summary>
    VideoDoor,
}

/// <summary>
/// What sits in the composer's row, per chat (section 2.1): left of the field the paperclip alone; in the field, at its
/// trailing edge, the stickers always, the paintbrush in the assistant's chat, the video button where S1.4 puts it; right
/// of it the slot (S1.3, unchanged). <c>@ai</c> has no button any more: the paperclip's "Ask the Assistant" and the first
/// line of the <c>@</c> strip reach it (<see cref="AttachMenu"/>, <see cref="ComposerMentions.Choices"/>).
/// </summary>
/// <remarks>
/// <para>
/// <b>THE FIELD MAKES ROOM FOR WHAT IT CARRIES</b>: its trailing padding grows by one target per glyph drawn in it, so text
/// never runs under a button (<see cref="TrailingRoom"/>).
/// </para>
/// <para>
/// <b>THE THREAD'S COMPOSER IS THE SAME FIELD</b>: the sticker glyph inside it and the same Send disc, disabled while the
/// reply is blank — no paperclip and no microphone, as before.
/// </para>
/// </remarks>
public static class ComposerRow
{
    /// <summary>Every control of the row is a target this size, in effective pixels; glyphs keep their own size inside it.</summary>
    public const int TargetEpx = ComposerButton.MinTargetWindowsEpx;

    /// <summary>
    /// What a field adds to its style's padding above and below, so one line stands as tall as the targets beside it and
    /// centred on them — the field bottom-aligned with the controls, its last line level with theirs as it grows.
    /// </summary>
    public const int FieldLiftEpx = 5;

    // Segoe Fluent Icons — line glyphs for every control of the row, never colour emoji.
    public const int AttachGlyph = 0xE723;
    public const int StickerGlyph = 0xE899;
    public const int PictureGlyph = 0xE771;
    public const int AssistantGlyph = 0xE99A;

    /// <summary>
    /// The glyphs inside the conversation's field, leading to trailing: the stickers where <see cref="PackSending.Offered"/>
    /// has them, the paintbrush where <see cref="AssistantButtons.Offered"/> does, and the video button where
    /// <paramref name="videoDoor"/> (<see cref="ComposerButton.VideoDoor"/>) shows it — never the paintbrush and the video
    /// button together, which are the assistant's chat and the others'.
    /// </summary>
    public static IReadOnlyList<FieldGlyph> InField(
        string? chatKind, bool editing, bool packOffered, AssistantDto? assistant, bool videoDoor)
    {
        List<FieldGlyph> glyphs = [];
        if (PackSending.Offered(chatKind, editing, packOffered))
        {
            glyphs.Add(FieldGlyph.Sticker);
        }
        if (AssistantButtons.Offered(chatKind, assistant, editing).AskPicture)
        {
            glyphs.Add(FieldGlyph.Picture);
        }
        else if (videoDoor)
        {
            glyphs.Add(FieldGlyph.VideoDoor);
        }
        return glyphs;
    }

    /// <summary>The glyphs inside the thread's field: the stickers, wherever that composer can send.</summary>
    public static IReadOnlyList<FieldGlyph> InThreadField(bool hasRoot, bool packOffered) =>
        PackSending.OfferedInThread(hasRoot, packOffered) ? [FieldGlyph.Sticker] : [];

    /// <summary>The trailing room a field gives its glyphs, over its own padding.</summary>
    public static int TrailingRoom(IReadOnlyList<FieldGlyph> glyphs) => glyphs.Count * TargetEpx;

    /// <summary>The thread's Send: a reply needs a root to answer and something to say.</summary>
    public static bool ThreadSendEnabled(bool hasRoot, string? draft) => hasRoot && !string.IsNullOrWhiteSpace(draft);
}

/// <summary>Where the chat list, the conversation and the thread sit across the window (<see cref="ConversationColumn.Arrange"/>).</summary>
/// <param name="ListWidth">The chat list's width, between <see cref="ConversationColumn.ListMinEpx"/> and <see cref="ConversationColumn.ListEpx"/>.</param>
/// <param name="ThreadBeside">The thread sits beside the conversation; false is OVER it, when beside would starve the field.</param>
/// <param name="PaneWidth">What the conversation's pane is left with.</param>
public readonly record struct PaneArrangement(double ListWidth, bool ThreadBeside, double PaneWidth);

/// <summary>
/// The wide rule (section 2.2) and the least window (section 3, fix 8): the conversation column — messages AND composer —
/// is <c>min(pane − 2×16, 1024)</c> and centred, a bubble at most <c>min(column − 48, 560)</c>, and the window never so
/// narrow that the field's run of text drops under <see cref="MinTextRunEpx"/>.
/// </summary>
/// <remarks>
/// <para>
/// <b>THE BUBBLE'S RULE IS ITS MARGIN AND ITS CAP</b>: a bubble keeps <see cref="BubbleSpacerEpx"/> clear on the side it does
/// not lean to and is capped at <see cref="BubbleMaxEpx"/>, which inside a column is exactly <see cref="BubbleMax"/> — so a
/// resize moves no bubble by hand, and the incremental list (<see cref="ConversationRows"/>) is never redrawn for one.
/// </para>
/// <para>
/// <b>THE FIELD'S FLOOR IS ARITHMETIC</b>, not a guess: the pane's insets, the card's border and padding, the paperclip,
/// the slot and the spacing between them, the field's own chrome, the two glyphs a field carries at most, and the run of
/// text. Below it the chat list narrows to <see cref="ListMinEpx"/>, the thread goes over the conversation instead of
/// beside it, and the window will not be made narrower than <see cref="MinimumWindowWidthEpx"/>.
/// </para>
/// </remarks>
public static class ConversationColumn
{
    /// <summary>The clear margin either side of the column.</summary>
    public const double PaneInsetEpx = 16;

    /// <summary>The widest a column grows.</summary>
    public const double MaxColumnEpx = 1024;

    /// <summary>The widest a bubble grows — a readable line on a wide window.</summary>
    public const double BubbleMaxEpx = 560;

    /// <summary>What a bubble leaves clear on the side it does not lean to.</summary>
    public const double BubbleSpacerEpx = 48;

    /// <summary>The chat list as wide as it is drawn, and the least it narrows to before anything else gives.</summary>
    public const double ListEpx = 300;
    public const double ListMinEpx = 260;

    /// <summary>The line between the list and the conversation.</summary>
    public const double DividerEpx = 1;

    /// <summary>The thread panel beside the conversation, its own left line included.</summary>
    public const double ThreadEpx = 400;

    /// <summary>The navigation rail on the window's left edge (LeftCompact).</summary>
    public const double RailEpx = 48;

    /// <summary>The window's frame and resize borders, which the outer size counts and the content never gets.</summary>
    public const double FrameEpx = 16;

    /// <summary>The least run of text the conversation's field keeps.</summary>
    public const double MinTextRunEpx = 200;

    /// <summary>The composer card's border and padding, either side.</summary>
    public const double CardChromeEpx = 1 + 6;

    /// <summary>The spacing between the paperclip, the field and the slot.</summary>
    public const double RowSpacingEpx = 6;

    /// <summary>The field's own border and its style's padding (TextControlThemePadding: 10 leading, 6 trailing).</summary>
    public const double FieldChromeEpx = 1 + 10 + 6 + 1;

    /// <summary>The most glyphs a field carries at once: the stickers and the paintbrush, or the stickers and the video button.</summary>
    public const int MostFieldGlyphs = 2;

    /// <summary>The least pane that leaves the field <see cref="MinTextRunEpx"/>.</summary>
    public const double MinPaneEpx =
        (2 * PaneInsetEpx) + (2 * CardChromeEpx)
        + ComposerRow.TargetEpx + RowSpacingEpx + RowSpacingEpx + ComposerRow.TargetEpx
        + FieldChromeEpx + (MostFieldGlyphs * ComposerRow.TargetEpx)
        + MinTextRunEpx;

    /// <summary>The least window, outer size, with the list at its narrowest and the thread over the conversation.</summary>
    public const double MinimumWindowWidthEpx = RailEpx + ListMinEpx + DividerEpx + MinPaneEpx + FrameEpx;

    /// <summary>The conversation column for a pane this wide: <c>min(pane − 2×16, 1024)</c>, never below nothing.</summary>
    public static double Width(double pane) => Math.Max(0, Math.Min(pane - (2 * PaneInsetEpx), MaxColumnEpx));

    /// <summary>How far inside the column's trailing edge the jump-to-newest button stands.</summary>
    public const double JumpInsideEpx = 12;

    /// <summary>
    /// What the jump-to-newest button leaves clear of the pane's trailing edge: the column's margin there and
    /// <see cref="JumpInsideEpx"/> more, so it stands inside the column on any window — 28 epx wherever the column fills the
    /// pane, as it always stood.
    /// </summary>
    public static double JumpInset(double pane) => (Math.Max(0, pane - Width(pane)) / 2) + JumpInsideEpx;

    /// <summary>The widest bubble in a column this wide: <c>min(column − 48, 560)</c>, never below nothing.</summary>
    public static double BubbleMax(double column) => Math.Max(0, Math.Min(column - BubbleSpacerEpx, BubbleMaxEpx));

    /// <summary>
    /// The list, the pane and the thread for a view this wide: the thread beside the conversation while that leaves the pane
    /// <see cref="MinPaneEpx"/> with the list at its narrowest, and over it otherwise; the list as wide as it is drawn while
    /// the pane keeps its floor, narrowing to <see cref="ListMinEpx"/> before the pane goes under it.
    /// </summary>
    public static PaneArrangement Arrange(double view, bool threadOpen)
    {
        var beside = threadOpen && view - ListMinEpx - DividerEpx - ThreadEpx >= MinPaneEpx;
        var thread = beside ? ThreadEpx : 0;
        var list = Math.Clamp(view - DividerEpx - thread - MinPaneEpx, ListMinEpx, ListEpx);
        return new PaneArrangement(list, beside, Math.Max(0, view - list - DividerEpx - thread));
    }
}
