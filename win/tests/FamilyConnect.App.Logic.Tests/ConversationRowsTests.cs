using FamilyConnect.App.Logic;
using FamilyConnect.Core.Protocol;
using Xunit;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// Which rows a conversation's redraw keeps, and how the panel is turned into the rows wanted: only what changed is built,
/// nothing kept ever moves, and anything that cannot be done that way falls back to building it all, as before.
/// </summary>
public sealed class ConversationRowsTests
{
    private static RowMark Mark(string key, string signature = "s") => new(key, signature);

    /// <summary>A panel that records what was done to it — the window's UIElementCollection, played by a list.</summary>
    private sealed class Panel(IEnumerable<string> children)
    {
        public List<string> Children { get; } = [.. children];

        public List<string> Done { get; } = [];

        public void RemoveAt(int index)
        {
            Done.Add($"-{Children[index]}");
            Children.RemoveAt(index);
        }

        public void Insert(int index, string element)
        {
            Assert.DoesNotContain(element, Children);
            Done.Add($"+{element}@{index}");
            Children.Insert(index, element);
        }
    }

    private static (int Changes, Panel Panel, List<string> Shown) Merge(string[] before, string[] after)
    {
        // Distinct instances, so the merge's reference comparison is what is tested and not string interning.
        var panel = new Panel(before);
        var shown = new List<string>(panel.Children);
        var changes = ConversationRows.Merge(shown, after, panel.RemoveAt, panel.Insert);
        return (changes, panel, shown);
    }

    [Fact]
    public void AnUnchangedRowIsKeptAndAChangedOneIsBuilt()
    {
        var plan = ConversationRows.Plan(
            [Mark("m1"), Mark("m2"), Mark("m3")],
            [Mark("m1"), Mark("m2", "edited"), Mark("m3")]);
        Assert.Equal([0, -1, 2], plan);
    }

    /// <summary>What arrived goes below, a page of history above, and every row already drawn is kept where it is.</summary>
    [Fact]
    public void ArrivalsAndHistoryKeepEveryRowAlreadyDrawn()
    {
        var plan = ConversationRows.Plan(
            [Mark("m5"), Mark("m6")],
            [Mark("m3"), Mark("m4"), Mark("m5"), Mark("m6"), Mark("m7"), Mark("p-a")]);
        Assert.Equal([-1, -1, 0, 1, -1, -1], plan);
    }

    /// <summary>A row whose drawing never finished is built again — the retry every redraw used to give it.</summary>
    [Fact]
    public void ARowThatNeverFinishedDrawingIsBuiltAgain()
    {
        var plan = ConversationRows.Plan(
            [Mark("m1"), Mark("m2"), Mark("m3")],
            [Mark("m1"), Mark("m2"), Mark("m3")],
            keepable: index => index != 1);
        Assert.Equal([0, -1, 2], plan);
    }

    /// <summary>Kept rows never move: one that would have to jump back over another is built again instead.</summary>
    [Fact]
    public void ARowThatWouldHaveToMoveIsBuiltAgain()
    {
        var plan = ConversationRows.Plan(
            [Mark("a"), Mark("b"), Mark("c")],
            [Mark("c"), Mark("a"), Mark("b")]);
        Assert.Equal([2, -1, -1], plan);
    }

    [Fact]
    public void AKeyThatAppearsTwiceIsNeverKept()
    {
        Assert.Equal([-1, 1], ConversationRows.Plan([Mark("a"), Mark("b"), Mark("a")], [Mark("a"), Mark("b")]));
        Assert.Equal([-1, -1, 1], ConversationRows.Plan([Mark("a"), Mark("b")], [Mark("a"), Mark("a"), Mark("b")]));
    }

    [Fact]
    public void NothingDrawnBuildsEverything() =>
        Assert.Equal([-1, -1], ConversationRows.Plan([], [Mark("a"), Mark("b")]));

    /// <summary>The window slides: what fell off the top goes, what arrived is added, and the rows in between are untouched.</summary>
    [Fact]
    public void TheMergeRemovesAndInsertsAndNeverTouchesWhatItKeeps()
    {
        var (changes, panel, shown) = Merge(["day", "a", "b", "c"], ["b", "c", "d", "pending"]);
        Assert.Equal(4, changes);
        Assert.Equal(["b", "c", "d", "pending"], panel.Children);
        Assert.Equal(panel.Children, shown);
        Assert.Equal(["-a", "-day", "+d@2", "+pending@3"], panel.Done);
    }

    [Fact]
    public void APageOfHistoryIsInsertedAboveAndAChangedRowInPlace()
    {
        var (changes, panel, _) = Merge(["b", "c-old", "d"], ["pill", "a", "b", "c-new", "d"]);
        Assert.Equal(4, changes);
        Assert.Equal(["pill", "a", "b", "c-new", "d"], panel.Children);
        Assert.Equal(["-c-old", "+pill@0", "+a@1", "+c-new@3"], panel.Done);
    }

    [Fact]
    public void NothingChangedIsNoChangeAtAll()
    {
        var (changes, panel, _) = Merge(["a", "b"], ["a", "b"]);
        Assert.Equal(0, changes);
        Assert.Empty(panel.Done);
    }

    /// <summary>
    /// Kept elements out of order would have to MOVE, and inserting one still in the panel throws: the merge says so before
    /// it touches anything, and the caller builds the whole panel as it used to.
    /// </summary>
    [Fact]
    public void KeptElementsOutOfOrderChangeNothingAndAskForAWholeRebuild()
    {
        var (changes, panel, shown) = Merge(["a", "b", "c"], ["c", "a"]);
        Assert.Equal(-1, changes);
        Assert.Empty(panel.Done);
        Assert.Equal(["a", "b", "c"], shown);
    }

    [Fact]
    public void TheSameElementWantedTwiceAsksForAWholeRebuild()
    {
        var twice = "x";
        var panel = new Panel([]);
        var shown = new List<string>();
        Assert.Equal(-1, ConversationRows.Merge(shown, [twice, twice], panel.RemoveAt, panel.Insert));
        Assert.Empty(panel.Done);
    }

    /// <summary>Elements are compared by identity: an equal string that is another element is another element.</summary>
    [Fact]
    public void ElementsAreTheSameOnlyWhenTheyAreTheSameObject()
    {
        var kept = new string('a', 1);
        var lookalike = new string('a', 1);
        var panel = new Panel([]);
        panel.Children.Add(kept);
        var shown = new List<string> { kept };
        var changes = ConversationRows.Merge(shown, [lookalike], panel.RemoveAt, panel.Insert);
        Assert.Equal(2, changes);
        Assert.Same(lookalike, shown[0]);
    }

    private static TimelineRow Row(MessageDto message, bool hidden = false, bool revealed = false, bool runEnd = true) =>
        new(new Bubble(message, hidden, revealed, false), null, false, false, true, runEnd);

    private static MessageDto Message => new(5, 42, 9, null, "Dinner at 7?", "2026-09-12T10:00:00Z");

    /// <summary>
    /// The signature changes with EVERYTHING a row is drawn from — every field of the message (a trimmed preview made whole
    /// carries no new sequence number), whether it reads, its place in the timeline and what the view adds — and with
    /// nothing else: the same message read twice from the store signs the same.
    /// </summary>
    [Fact]
    public void TheSignatureFollowsEverythingTheRowIsDrawnFrom()
    {
        var plain = ConversationRows.Signature(Row(Message), "seen");
        Assert.Equal(plain, ConversationRows.Signature(Row(Message with { }), "seen"));
        Assert.Equal(
            ConversationRows.Signature(Row(Message with { Reactions = [new ReactionDto(7, "👍")] }), "seen"),
            ConversationRows.Signature(Row(Message with { Reactions = [new ReactionDto(7, "👍")] }), "seen"));

        string[] changed =
        [
            ConversationRows.Signature(Row(Message with { Body = "Dinner at 8?" }), "seen"),
            ConversationRows.Signature(Row(Message with { ReplyCount = 2 }), "seen"),
            ConversationRows.Signature(Row(Message with { Reactions = [new ReactionDto(7, "👍")] }), "seen"),
            ConversationRows.Signature(Row(Message with { ReplyTo = new ReplyToDto(3, 7, "hi") }), "seen"),
            ConversationRows.Signature(Row(Message with { Attachments = [new AttachmentDto(8, "photo")] }), "seen"),
            ConversationRows.Signature(Row(Message, hidden: true), "seen"),
            ConversationRows.Signature(Row(Message, hidden: true, revealed: true), "seen"),
            ConversationRows.Signature(Row(Message, runEnd: false), "seen"),
            ConversationRows.Signature(Row(Message) with { DayAbove = new DateOnly(2026, 9, 12) }, "seen"),
            ConversationRows.Signature(Row(Message) with { UnreadDividerAbove = true }, "seen"),
            ConversationRows.Signature(Row(Message) with { ShowsSender = true }, "seen"),
            ConversationRows.Signature(Row(Message), "sent"),
        ];
        Assert.DoesNotContain(plain, changed);
        Assert.Equal(changed.Length, changed.Distinct().Count());
    }

    [Fact]
    public void KeysNameTheirKindOfRow()
    {
        Assert.Equal("m12", ConversationRows.MessageKey(12));
        Assert.Equal("pabc", ConversationRows.PendingKey("abc"));
        Assert.NotEqual(ConversationRows.MessageKey(12), ConversationRows.PendingKey("12"));
    }
}
