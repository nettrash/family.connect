using FamilyConnect.Core;

namespace FamilyConnect.Core.Tests.Text;

/// <summary>
/// The owner's places for the greeting's weather, under the server's own rules for a name (docs/protocol.md, "Today's
/// weather, for places the owner chose"; <c>server/src/handlers_family.rs</c>, <c>validate_greeting_places</c>): trimmed and
/// folded, never empty, no control character, at most 80 characters — Rust's characters, counted as Rust counts them.
/// </summary>
public class GreetingPlacesTests
{
    [Fact]
    public void TheLimitsAreTheServers()
    {
        Assert.Equal(3, GreetingPlaces.MaxPlaces);
        Assert.Equal(80, GreetingPlaces.MaxChars);
    }

    /// <summary><c>split_whitespace().join(" ")</c>: every run of Unicode whitespace, inside or at the ends.</summary>
    [Theory]
    [InlineData("  Moscow  ", "Moscow")]
    [InlineData("New   York", "New York")]
    [InlineData("\tSan\u00A0\u2003Francisco\n", "San Francisco")]
    [InlineData("Rio\u3000de\u2028Janeiro", "Rio de Janeiro")]
    [InlineData("   ", "")]
    [InlineData("", "")]
    [InlineData(null, "")]
    // Not whitespace to Rust, so not folded: a zero-width space and the information separators.
    [InlineData("A\u200BB", "A\u200BB")]
    [InlineData("A\u001FB", "A\u001FB")]
    public void ANameIsFoldedAsTheServerFoldsIt(string? raw, string folded) =>
        Assert.Equal(folded, GreetingPlaces.Fold(raw));

    [Fact]
    public void EachNameIsJudgedAsTheServerJudgesIt()
    {
        Assert.Equal(GreetingPlaces.Problem.None, GreetingPlaces.Judge("Belgrade"));
        Assert.Equal(GreetingPlaces.Problem.Empty, GreetingPlaces.Judge(" \t "));
        // A control character that is not whitespace stays after folding, and is refused.
        Assert.Equal(GreetingPlaces.Problem.Control, GreetingPlaces.Judge("Mos\u0001cow"));
        Assert.Equal(GreetingPlaces.Problem.Control, GreetingPlaces.Judge("Mos\u007Fcow"));
        // A line break is whitespace, folded away before the check.
        Assert.Equal(GreetingPlaces.Problem.None, GreetingPlaces.Judge("Mos\ncow"));

        // Eighty characters after folding is the most; the spaces folded away do not count.
        Assert.Equal(GreetingPlaces.Problem.None, GreetingPlaces.Judge(new string('a', 80)));
        Assert.Equal(GreetingPlaces.Problem.TooLong, GreetingPlaces.Judge(new string('a', 81)));
        Assert.Equal(GreetingPlaces.Problem.None, GreetingPlaces.Judge("   " + new string('a', 40) + "      " + new string('b', 39)));
    }

    /// <summary>Characters are Unicode scalars: eighty astral characters are 160 UTF-16 units and still a valid name.</summary>
    [Fact]
    public void CharactersAreScalarsNotUtf16Units()
    {
        var astral = string.Concat(Enumerable.Repeat("\U0002000B", 80));
        Assert.Equal(160, astral.Length);
        Assert.Equal(80, GreetingPlaces.Length(astral));
        Assert.Equal(GreetingPlaces.Problem.None, GreetingPlaces.Judge(astral));
        Assert.Equal(GreetingPlaces.Problem.TooLong, GreetingPlaces.Judge(astral + "x"));
        Assert.Equal(80, GreetingPlaces.Length(new string('Ж', 80)));
    }

    /// <summary>
    /// The fields as a request: folded, blank fields left out (a field nobody typed in is no place), at most three, and the
    /// first field the server would refuse named by its position.
    /// </summary>
    [Fact]
    public void TheFieldsBecomeARequestOrNameTheFirstBadField()
    {
        var ok = GreetingPlaces.Check(["  Moscow ", "", "Belgrade"]);
        Assert.True(ok.Ok);
        Assert.Equal(["Moscow", "Belgrade"], ok.Names);

        var none = GreetingPlaces.Check(["", "   "]);
        Assert.True(none.Ok);
        Assert.Empty(none.Names);

        var control = GreetingPlaces.Check(["Moscow", "Bel\u0002grade"]);
        Assert.False(control.Ok);
        Assert.Equal(2, control.BadField);
        Assert.Equal(GreetingPlaces.Problem.Control, control.Why);

        var tooLong = GreetingPlaces.Check([new string('x', 81)]);
        Assert.Equal(1, tooLong.BadField);
        Assert.Equal(GreetingPlaces.Problem.TooLong, tooLong.Why);

        var four = GreetingPlaces.Check(["A", "B", "", "C", "D"]);
        Assert.Equal(5, four.BadField);
        Assert.Equal(GreetingPlaces.Problem.TooMany, four.Why);
    }

    /// <summary>
    /// Repeats are the SERVER's to drop: its lower-casing is Rust's, and a second answer here could disagree with it. With
    /// three fields at most, a repeat never pushes the list over the limit.
    /// </summary>
    [Fact]
    public void RepeatsAreSentForTheServerToDrop()
    {
        var check = GreetingPlaces.Check(["Moscow", "moscow"]);
        Assert.True(check.Ok);
        Assert.Equal(["Moscow", "moscow"], check.Names);
    }

    /// <summary>
    /// What a field holds as somebody types or pastes: no control character that is not whitespace, at most 80 characters,
    /// never cut inside a surrogate pair — and the spaces left alone, or the space before the next word would be eaten.
    /// </summary>
    [Fact]
    public void TypingKeepsWhatTheServerWouldKeep()
    {
        Assert.Equal("New ", GreetingPlaces.Typed("New "));
        Assert.Equal("  Rio  de", GreetingPlaces.Typed("  Rio  de"));
        Assert.Equal("Moscow", GreetingPlaces.Typed("Mos\u0000c\u0007ow\u009B"));
        Assert.Equal("Mos\tcow", GreetingPlaces.Typed("Mos\tcow"));
        Assert.Equal(string.Empty, GreetingPlaces.Typed(null));
        Assert.Equal(new string('a', 80), GreetingPlaces.Typed(new string('a', 100)));

        var astral = string.Concat(Enumerable.Repeat("\U0001F600", 81));
        var typed = GreetingPlaces.Typed(astral);
        Assert.Equal(80, GreetingPlaces.Length(typed));
        Assert.Equal(160, typed.Length);
        Assert.False(char.IsHighSurrogate(typed[^1]));

        // Whatever typing leaves, folded, is never refused for its characters or its length.
        foreach (var text in new[] { new string(' ', 200), "a\u0001b", astral, "x" + new string('\u0085', 90) + "y" })
        {
            Assert.NotEqual(GreetingPlaces.Problem.Control, GreetingPlaces.Judge(GreetingPlaces.Typed(text)));
            Assert.NotEqual(GreetingPlaces.Problem.TooLong, GreetingPlaces.Judge(GreetingPlaces.Typed(text)));
        }
    }

    [Fact]
    public void TypingCountsTheLengthTheServerCountsAfterFolding()
    {
        // The Copilot case: 79 leading spaces cost nothing, so all of "Paris" is kept, whitespace and all.
        var padded = new string(' ', 79) + "Paris";
        Assert.Equal(padded, GreetingPlaces.Typed(padded));
        Assert.Equal(GreetingPlaces.Problem.None, GreetingPlaces.Judge(GreetingPlaces.Typed(padded)));

        // A run of whitespace between two words costs ONE, as its folded space does.
        var gapped = new string('a', 39) + " \t\u3000\u2028  " + new string('b', 40);
        Assert.Equal(gapped, GreetingPlaces.Typed(gapped));
        Assert.Equal(80, GreetingPlaces.Length(GreetingPlaces.Fold(gapped)));
        Assert.Equal(gapped, GreetingPlaces.Typed(gapped + "c"));

        // The 81st folded character stops it — and so does the gap that would make it the 81st.
        Assert.Equal(new string('a', 79) + "  ", GreetingPlaces.Typed(new string('a', 79) + "  b"));
        Assert.Equal(new string('a', 78) + "  b", GreetingPlaces.Typed(new string('a', 78) + "  b"));

        // Trailing whitespace past the limit is kept (the space somebody is typing), and costs nothing.
        var full = new string('a', 80) + "   ";
        Assert.Equal(full, GreetingPlaces.Typed(full));

        // Astral and multi-byte text: one rune, one character, whatever its UTF-16 or UTF-8 length.
        var emoji = new string(' ', 40) + string.Concat(Enumerable.Repeat("\U0001F600", 77)) + "  \U0001F1F7\U0001F1F8!";
        var typedEmoji = GreetingPlaces.Typed(emoji);
        Assert.Equal(new string(' ', 40) + string.Concat(Enumerable.Repeat("\U0001F600", 77)) + "  \U0001F1F7\U0001F1F8", typedEmoji);
        Assert.Equal(80, GreetingPlaces.Length(GreetingPlaces.Fold(typedEmoji)));
        Assert.False(char.IsHighSurrogate(typedEmoji[^1]));

        var cyrillic = "   " + string.Concat(Enumerable.Repeat("Москва ", 10)) + "Санкт-Петербург";
        var typedCyrillic = GreetingPlaces.Typed(cyrillic);
        Assert.Equal(80, GreetingPlaces.Length(GreetingPlaces.Fold(typedCyrillic)));
        Assert.StartsWith("   " + string.Concat(Enumerable.Repeat("Москва ", 10)), typedCyrillic);
        Assert.EndsWith("Москва Санкт-Пете", typedCyrillic);

        // A control character dropped costs nothing either, and does not open a gap.
        Assert.Equal(new string('a', 80), GreetingPlaces.Typed(new string('a', 40) + "\u0001\u009B" + new string('a', 41)));
    }
}
