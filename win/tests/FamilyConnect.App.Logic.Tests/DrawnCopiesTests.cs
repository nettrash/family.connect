using FamilyConnect.App.Logic;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// Every copy drawn of one voice message: the conversation's bubble and the thread panel's both hear that it was played,
/// and a redraw's new copy never displaces the other surface's.
/// </summary>
public sealed class DrawnCopiesTests
{
    private sealed class Row(string where)
    {
        public string Where { get; } = where;
    }

    /// <summary>Two copies of one note — the conversation's and the thread panel's — are both kept, not the last drawn.</summary>
    [Fact]
    public void BothCopiesOfOneNoteAreKept()
    {
        var copies = new DrawnCopies<long, Row>();
        var conversation = new Row("conversation");
        var thread = new Row("thread");
        copies.Add(40, conversation);
        copies.Add(40, thread);
        Assert.Equal([conversation, thread], copies.Of(40));
        Assert.Equal(2, copies.All.Count);
        // The same copy drawn again (back on screen) is still one.
        copies.Add(40, thread);
        Assert.Equal(2, copies.Of(40).Count);
    }

    /// <summary>A copy that leaves the screen goes by reference; the other surface's stays, and an empty key is gone.</summary>
    [Fact]
    public void ACopyLeavingTakesOnlyItself()
    {
        var copies = new DrawnCopies<long, Row>();
        var old = new Row("conversation, before the redraw");
        var redrawn = new Row("conversation, after");
        var thread = new Row("thread");
        copies.Add(40, old);
        copies.Add(40, thread);
        copies.Add(40, redrawn);
        Assert.True(copies.Remove(40, old));
        Assert.Equal([thread, redrawn], copies.Of(40));
        Assert.False(copies.Remove(40, old));
        Assert.False(copies.Remove(41, old));
        copies.Remove(40, thread);
        copies.Remove(40, redrawn);
        Assert.Empty(copies.Of(40));
        Assert.Empty(copies.All);
        copies.Add(7, thread);
        copies.Clear();
        Assert.Empty(copies.All);
    }
}
