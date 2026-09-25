using FamilyConnect.Core;

namespace FamilyConnect.Core.Tests.Text;

/// <summary>A text cut into words and names, on the right CHARACTERS whatever the bytes before them.</summary>
public class MentionRunsTests
{
    [Fact]
    public void ATextIsCutIntoItsWordsAndTheNamesItSays()
    {
        var runs = Mentions.Runs("hi @Anna Lee and @Bob!", [new Named(1, "Anna"), new Named(2, "Anna Lee"), new Named(3, "Bob")]);

        Assert.Equal([("hi ", (long?)null), ("@Anna Lee", 2), (" and ", null), ("@Bob", 3), ("!", null)], runs);
    }

    [Fact]
    public void NamesAfterWideCharactersLandOnTheRightCharacters()
    {
        const string family = "👨‍👩‍👧";
        var mark = ((char)0x301).ToString();
        var text = $"{family} @Анна{mark} ok";

        var runs = Mentions.Runs(text, [new Named(5, "Анна")]);

        Assert.Equal([($"{family} ", (long?)null), ($"@Анна{mark}", 5), (" ok", null)], runs);
    }

    [Fact]
    public void ATextWithNoNamesIsOneRun()
    {
        Assert.Equal([("plain", (long?)null)], Mentions.Runs("plain", []));
        Assert.Equal([("@Anna", (long?)7)], Mentions.Runs("@Anna", [new Named(7, "Anna")]));
    }
}
