using FamilyConnect.Core;

namespace FamilyConnect.Core.Tests.Text;

/// <summary>
/// The footer the server appends to an answer that looked something up (docs/protocol.md, "How sources are shown: a footer
/// the server writes"), recognised by its shape and its fixed words — so the client can keep a lookup answer's links out
/// of preview cards (design decision 7).
/// </summary>
/// <remarks>
/// The bodies are built the way <c>server/src/lookups.rs</c>'s <c>footer</c> builds them: the answer, one blank line, the
/// sources line, the credit line.
/// </remarks>
public class SourcesFooterTests
{
    private const string Weather = "[Weather data by Open-Meteo.com](https://open-meteo.com/)";
    private const string Wikipedia = "Wikipedia, [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/)";
    private const string Brave = "Powered by Brave";

    /// <summary>protocol.md's own example, verbatim.</summary>
    public const string ProtocolExample =
        "Tomorrow in Tromsø: snow showers, around −2 °C …\n\n"
        + "Sources: [Tromsø – Wikipedia](https://en.wikipedia.org/wiki/Troms%C3%B8) · [Weather in Tromsø](https://example.org/tromso)\n"
        + Weather + " · " + Wikipedia + " · " + Brave;

    [Fact]
    public void TheProtocolsExampleIsAFooter() => Assert.True(SourcesFooter.EndsWithFooter(ProtocolExample));

    /// <summary>A weather-only answer has no sources line, and a SearXNG answer no credit line: both are footers.</summary>
    [Fact]
    public void EitherLineAloneIsAFooter()
    {
        Assert.True(SourcesFooter.EndsWithFooter("Snow tomorrow.\n\n" + Weather));
        Assert.True(SourcesFooter.EndsWithFooter("Here is what I found.\n\nSources: [A page](https://a.example/x)"));
        Assert.True(SourcesFooter.EndsWithFooter("Brave says so.\n\nSources: [A](https://a.example/) · [B](http://b.example/b)\n" + Brave));
    }

    /// <summary>The sources word in every language the server writes it in, English for anything else.</summary>
    [Fact]
    public void EveryLanguagesSourcesWordIsKnown()
    {
        foreach (var word in new[] { "Sources", "Quellen", "Fuentes", "出典", "Источники", "Извори", "Izvori", "来源" })
        {
            Assert.True(SourcesFooter.EndsWithFooter($"…\n\n{word}: [Title](https://a.example/)"), word);
        }
        Assert.Equal(8, SourcesFooter.SourcesWords.Count);
        // The credit words are translated; the links and Brave's words are not.
        Assert.True(SourcesFooter.EndsWithFooter(
            "…\n\nИсточники: [Тромсё](https://ru.wikipedia.org/wiki/%D0%A2)\n"
            + "[Данные о погоде: Open-Meteo.com](https://open-meteo.com/) · Википедия, [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/)"));
        Assert.True(SourcesFooter.EndsWithFooter("…\n\n维基百科, [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/)"));
    }

    [Fact]
    public void AnOrdinaryAnswerIsNot()
    {
        Assert.False(SourcesFooter.EndsWithFooter(null));
        Assert.False(SourcesFooter.EndsWithFooter(""));
        Assert.False(SourcesFooter.EndsWithFooter("Paris is the capital of France."));
        Assert.False(SourcesFooter.EndsWithFooter("See [the docs](https://example.com/docs)."));
        Assert.False(SourcesFooter.EndsWithFooter("Read more:\n\nhttps://example.com"));
        // A sources word with no links, or with prose after the links, is somebody's sentence.
        Assert.False(SourcesFooter.EndsWithFooter("…\n\nSources: my memory"));
        Assert.False(SourcesFooter.EndsWithFooter("…\n\nSources: [A](https://a.example/) and that is all"));
        Assert.False(SourcesFooter.EndsWithFooter("…\n\nReferences: [A](https://a.example/)"));
    }

    /// <summary>Only the END: a footer followed by anything else is not the server's, which appends it last.</summary>
    [Fact]
    public void TheFooterIsTheLastParagraph()
    {
        Assert.False(SourcesFooter.EndsWithFooter("…\n\nSources: [A](https://a.example/)\n\nAnd one more thing."));
        Assert.False(SourcesFooter.EndsWithFooter("…\n\nSources: [A](https://a.example/)\nAnd one more thing."));
        // Trailing whitespace and Windows line ends are not "anything else".
        Assert.True(SourcesFooter.EndsWithFooter("…\r\n\r\nSources: [A](https://a.example/)\r\n" + Weather + "\n "));
    }

    /// <summary>Three links at most, three known credits at most, each once.</summary>
    [Fact]
    public void TheShapeIsTheServersAndNoLooser()
    {
        Assert.True(SourcesFooter.EndsWithFooter("…\n\nSources: [A](https://a.example/) · [B](https://b.example/) · [C](https://c.example/)"));
        Assert.False(SourcesFooter.EndsWithFooter(
            "…\n\nSources: [A](https://a.example/) · [B](https://b.example/) · [C](https://c.example/) · [D](https://d.example/)"));
        Assert.False(SourcesFooter.EndsWithFooter("…\n\nSources: [A](javascript:alert)"));
        Assert.False(SourcesFooter.EndsWithFooter("…\n\n" + Weather + " · " + Weather));
        Assert.False(SourcesFooter.EndsWithFooter("…\n\nPowered by Bravo"));
        Assert.False(SourcesFooter.EndsWithFooter("…\n\n[Weather](https://open-meteo.example/)"));
        // The credit line belongs AFTER the sources line, never before it.
        Assert.False(SourcesFooter.EndsWithFooter("…\n\n" + Weather + "\nSources: [A](https://a.example/)"));
        Assert.False(SourcesFooter.EndsWithFooter("…\n\nSources: [A](https://a.example/)\nSources: [B](https://b.example/)"));
    }
}
