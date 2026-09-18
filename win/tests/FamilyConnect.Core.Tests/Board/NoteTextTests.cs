using FamilyConnect.Core;
using FamilyConnect.Core.Board;

namespace FamilyConnect.Core.Tests.Board;

/// <summary>
/// The caps, counted the way the SERVER counts them — Unicode scalars, not UTF-16 units and not
/// graphemes. This is where a .NET port is most likely to diverge, so it is pinned with the same
/// awkward strings the other ports use.
/// </summary>
public class NoteTextTests
{
    [Fact]
    public void TheCapsAreTheProtocolsOwn()
    {
        Assert.Equal(280, NoteText.MaxTextChars);
        Assert.Equal(200, NoteText.MaxPlaceChars);
        Assert.Equal(100, NoteText.MaxTaskItemChars);
        Assert.Equal(20, NoteText.MaxTaskItems);
        Assert.Equal(40, NoteText.CounterFrom);
    }

    [Fact]
    public void TheCapCountsScalarsTheWayTheServerDoes()
    {
        Assert.Equal(13, NoteText.Scalars("Milk and eggs"));
        // An emoji is ONE scalar to the server and two UTF-16 units to .NET: counting units would
        // give an emoji note half its allowance.
        Assert.Equal(1, NoteText.Scalars("🎂"));
        Assert.Equal(2, "🎂".Length);
        // A family emoji is several scalars joined by zero-width joiners — MORE than the one
        // grapheme a reader sees, which is the server's count and therefore ours.
        Assert.Equal(7, NoteText.Scalars("👨‍👩‍👧‍👦"));
        // A letter typed with a COMBINING mark is two scalars and one grapheme — so a grapheme
        // count would be smaller than the server's, and a note that looked under the cap would
        // come back refused. (The precomposed spelling of the same letter is one of each.)
        Assert.Equal(2, NoteText.Scalars("e\u0301"));
        Assert.Equal(1, NoteText.Scalars("\u00e9"));

        var long_ = new string('a', 300);
        Assert.Equal(280, NoteText.Scalars(NoteText.Capped(long_, NoteText.MaxTextChars)));
        Assert.Equal("Milk", NoteText.Capped("Milk and eggs", 4));
        Assert.Equal("Milk and eggs", NoteText.Capped("Milk and eggs", 280));
        Assert.Equal(string.Empty, NoteText.Capped("Milk", 0));
        // A cap must never cut a surrogate pair in half.
        Assert.Equal("🎂", NoteText.Capped("🎂🎂", 1));
        Assert.Equal(string.Empty, NoteText.Capped("🎂", 0));
    }

    [Fact]
    public void TheCounterOnlyShowsInTheLastForty()
    {
        Assert.Equal(280, NoteText.Remaining(string.Empty));
        Assert.False(NoteText.ShowsCounter(new string('a', 239)));
        Assert.True(NoteText.ShowsCounter(new string('a', 240)));
        Assert.Equal(0, NoteText.Remaining(new string('a', 400)));
        Assert.True(NoteText.ShowsCounter(new string('a', 400)));
    }

    [Fact]
    public void WhatWentPastTheCapIsCutWhereItWasTypedNotOffTheEnd()
    {
        // Typing in the MIDDLE of a full note: the overflow comes out of what was just typed,
        // and the caret stays where the author was writing.
        var value = new string('a', 279) + "xyz" + new string('b', 20);
        var (kept, caret) = NoteText.CapAtCaret(value, 282, NoteText.MaxTextChars);
        Assert.Equal(NoteText.MaxTextChars, NoteText.Scalars(kept));
        Assert.EndsWith(new string('b', 20), kept, StringComparison.Ordinal);
        // The caret FOLLOWS the cut: it sits where the deleted run began, which is the end of
        // what the author's typing left behind — not at the end of the note, which is where the
        // old "cut the tail" rule used to dump it.
        Assert.Equal(260, caret);
        Assert.Equal(new string('a', 260) + new string('b', 20), kept);
        // Under the cap: nothing moves at all.
        Assert.Equal(("Milk", 2), NoteText.CapAtCaret("Milk", 2, 280));
        // A caret at the very start has nothing before it to take, so the END is cut, as before.
        var (fromStart, startCaret) = NoteText.CapAtCaret(new string('a', 300), 0, 280);
        Assert.Equal(280, NoteText.Scalars(fromStart));
        Assert.Equal(0, startCaret);
    }

    [Fact]
    public void AListSaysHowMuchOfItIsDone()
    {
        Assert.Equal("2 of 5 done", NoteText.DoneOf(2, 5, EnglishCatalog.Instance));
    }
}
