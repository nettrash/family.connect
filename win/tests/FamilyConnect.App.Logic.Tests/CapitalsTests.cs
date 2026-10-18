using FamilyConnect.App.Logic;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// Capital letters (issue #97): docs/capitals-and-rich-markdown-2026-10-17.md, "The table every port's tests hold", verbatim
/// — the same table as the web's <c>composer::capitalizes</c> and the Mac's <c>CapitalsTests</c> — and the take-back.
/// </summary>
public sealed class CapitalsTests
{
    [Theory]
    [InlineData("", "h", true)]
    [InlineData("Hi. ", "h", true)]
    // The word is not starting.
    [InlineData("Hi.", "h", false)]
    [InlineData("hi ", "h", false)]
    [InlineData("Really? ", "y", true)]
    [InlineData("Wow! ", "s", true)]
    // An ellipsis.
    [InlineData("Wait... ", "w", false)]
    // An abbreviation.
    [InlineData("See e.g. ", "t", false)]
    [InlineData("It's at example.com. ", "t", true)]
    [InlineData("Line\n", "n", true)]
    [InlineData("He said \"stop.\" ", "t", true)]
    [InlineData("(Done.) ", "n", true)]
    [InlineData("\"", "h", true)]
    [InlineData("Hi. (", "s", true)]
    [InlineData("- ", "m", true)]
    [InlineData("List:\n1. ", "b", true)]
    [InlineData("12) ", "x", true)]
    [InlineData("# ", "t", true)]
    [InlineData("> ", "q", true)]
    // Inside a fence.
    [InlineData("```\n", "l", false)]
    // The fence closed.
    [InlineData("```\ncode\n```\n", "n", true)]
    // Inside a code span.
    [InlineData("Use `x. ", "y", false)]
    // Already upper case.
    [InlineData("Hi. ", "H", false)]
    // Not a letter.
    [InlineData("Hi. ", "1", false)]
    // Its upper case is two letters.
    [InlineData("", "ß", false)]
    // Two spaces.
    [InlineData("Hi.  ", "h", true)]
    // A tab.
    [InlineData("Hi.\t", "h", true)]
    [InlineData("", "é", true)]
    [InlineData("Привет. ", "к", true)]
    [InlineData("Ok.\n\n", "n", true)]
    [InlineData("@ann ", "h", false)]
    public void TheDesignTable(string before, string typed, bool capitalizes) =>
        Assert.Equal(capitalizes, Capitals.Capitalizes(before, typed));

    /// <summary>The edges of each condition, beyond the table — the same as the web's <c>capitals_edges</c>.</summary>
    [Theory]
    // Every opening and closing mark.
    [InlineData("Hi. «", true)]
    [InlineData("Hi. “", true)]
    [InlineData("Hi. ‘", true)]
    [InlineData("Hi. [", true)]
    [InlineData("Hi. '", true)]
    [InlineData("((", true)]
    [InlineData("Done.» ", true)]
    [InlineData("Done.” ", true)]
    [InlineData("Done.’ ", true)]
    [InlineData("Done.] ", true)]
    [InlineData("Done!'' ", true)]
    // A TextBox's lone carriage return is a line break.
    [InlineData("Line\r", true)]
    [InlineData("```\r", false)]
    [InlineData("```\rcode\r```\r", true)]
    [InlineData("```\r\ncode\r\n```\r\n", true)]
    // Markers: indented, every heading depth, and near misses.
    [InlineData("  - ", true)]
    [InlineData("###### ", true)]
    [InlineData("####### ", false)]
    [InlineData("-- ", false)]
    [InlineData("1.2 ", false)]
    [InlineData("Hi. - ", false)]
    // Abbreviations: the pieces must be one or two ASCII letters.
    [InlineData("a.m. ", false)]
    [InlineData("in the U.S. ", false)]
    [InlineData("v1.2. ", true)]
    [InlineData("abc.d. ", true)]
    [InlineData("é.g. ", true)]
    [InlineData("Plan B. ", true)]
    [InlineData("What?! ", true)]
    [InlineData("Hi, ", false)]
    [InlineData("Note: ", false)]
    // A fence line counts only at the start of a line; a closed code span is not code.
    [InlineData("x ```\n", true)]
    [InlineData("Use `x`. ", true)]
    public void TheEdges(string before, bool capitalizes) => Assert.Equal(capitalizes, Capitals.Capitalizes(before, "a"));

    [Fact]
    public void WhichCharactersHaveACapital()
    {
        Assert.Equal("A", Capitals.CapitalOf("a"));
        Assert.Equal("Ж", Capitals.CapitalOf("ж"));
        Assert.Null(Capitals.CapitalOf("ß"));
        Assert.Null(Capitals.CapitalOf("A"));
        // Rust's full mapping, not .NET's simple one: ᾳ's capital is two letters, so it has none here — as on the web.
        Assert.Null(Capitals.CapitalOf("ᾳ"));
        foreach (var typed in new[] { "ŉ", "ǰ", "ﬀ", "ª", "日", " ", ".", "", "ab", "\uD801" })
        {
            Assert.Null(Capitals.CapitalOf(typed));
            Assert.False(Capitals.Capitalizes("", typed));
        }
        // Outside the Basic Multilingual Plane, a surrogate pair is one character: Deseret.
        Assert.Equal("\U00010400", Capitals.CapitalOf("\U00010428"));
        Assert.True(Capitals.Capitalizes("Hi. ", "\U00010428"));
    }

    /// <summary>Only one character typed over the old selection counts — never a deletion that happens to look like one.</summary>
    [Fact]
    public void WhatCountsAsTyped()
    {
        Assert.Equal((4, "h"), Capitals.TypedCharacter("Hi. ", 4, 0, "Hi. h"));
        Assert.Equal((0, "h"), Capitals.TypedCharacter("ello", 0, 0, "hello"));
        // Typed over a selection.
        Assert.Equal((4, "t"), Capitals.TypedCharacter("Hi. xyz", 4, 3, "Hi. t"));
        // A surrogate pair is one character; half of one is not.
        Assert.Equal((0, "\U00010428"), Capitals.TypedCharacter("", 0, 0, "\U00010428"));
        Assert.Null(Capitals.TypedCharacter("", 0, 0, "\uD801"));
        // A paste of two, a deletion, a Backspace that looks like typing, an unchanged text.
        Assert.Null(Capitals.TypedCharacter("", 0, 0, "hi"));
        Assert.Null(Capitals.TypedCharacter("hi", 2, 0, "h"));
        Assert.Null(Capitals.TypedCharacter("aab", 1, 0, "ab"));
        Assert.Null(Capitals.TypedCharacter("ab", 1, 0, "ab"));
        // The change is somewhere else than the selection.
        Assert.Null(Capitals.TypedCharacter("ab", 0, 0, "abx"));
        // A selection outside the text is nothing.
        Assert.Null(Capitals.TypedCharacter("ab", 3, 0, "abx"));
    }

    [Fact]
    public void AfterTypingPutsInTheCapitalAndRemembersIt()
    {
        var edit = Capitals.AfterTyping("Hi. ", 4, 0, "Hi. h");
        Assert.NotNull(edit);
        Assert.Equal("Hi. H", edit.Text);
        Assert.Equal(5, edit.Caret);
        Assert.Equal(new CapitalTakeBack(4, "H", "h"), edit.TakeBack);

        // In the middle of the text, with what follows the caret kept.
        Assert.Equal("Ok. Then go", Capitals.AfterTyping("Ok. hen go", 4, 0, "Ok. then go")?.Text);
        Assert.Equal("Привет. К", Capitals.AfterTyping("Привет. ", 8, 0, "Привет. к")?.Text);
        Assert.Equal("\U00010400", Capitals.AfterTyping("", 0, 0, "\U00010428")?.Text);
        Assert.Equal(2, Capitals.AfterTyping("", 0, 0, "\U00010428")?.Caret);

        // Not a sentence's start, not a letter, not one character: the change stands.
        Assert.Null(Capitals.AfterTyping("hi ", 3, 0, "hi h"));
        Assert.Null(Capitals.AfterTyping("Hi. ", 4, 0, "Hi. 1"));
        Assert.Null(Capitals.AfterTyping("Hi. ", 4, 0, "Hi. hi"));
    }

    [Fact]
    public void ABackspaceStraightAfterTakesItBack()
    {
        var take = new CapitalTakeBack(4, "H", "h");
        Assert.Equal(("Hi. h", 5), take.Backspace("Hi. H", 5, 0));
        Assert.Equal(("Hi. hello", 5), take.Backspace("Hi. Hello", 5, 0));
        // Not right behind it, something selected, or the capital gone: an ordinary Backspace.
        Assert.Null(take.Backspace("Hi. H", 4, 0));
        Assert.Null(take.Backspace("Hi. H", 5, 1));
        Assert.Null(take.Backspace("Hi. Ho", 6, 0));
        Assert.Null(take.Backspace("Hi. x", 5, 0));
        Assert.Null(take.Backspace("Hi.", 5, 0));

        var deseret = CapitalTakeBack.After(0, "\U00010428");
        Assert.NotNull(deseret);
        Assert.Equal(("\U00010428!", 2), deseret.Backspace("\U00010400!", 2, 0));
        Assert.Null(deseret.Backspace("\U00010400!", 1, 0));
        Assert.Null(CapitalTakeBack.After(0, "ß"));
    }
}
