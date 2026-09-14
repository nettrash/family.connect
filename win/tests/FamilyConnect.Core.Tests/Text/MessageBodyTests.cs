using FamilyConnect.Core;

namespace FamilyConnect.Core.Tests.Text;

/// <summary>A body composed for drawing — the web client's <c>views/body.rs</c> tests, case for case.</summary>
public class MessageBodyTests
{
    private static IReadOnlyList<BodyPiece> LaidOut(string body, params Named[] named) =>
        Markdown.Blocks(body)[0] is MarkdownTextBlock { Text: var text } ? MessageBody.Pieces(text, body, named) : [];

    private static BodyPiece Find(IReadOnlyList<BodyPiece> pieces, string text) =>
        pieces.FirstOrDefault(piece => piece.Text == text)
        ?? throw new Xunit.Sdk.XunitException($"no piece \"{text}\" in [{string.Join(" | ", pieces.Select(piece => piece.Text))}]");

    [Fact]
    public void MarkdownStylesReachThePieces()
    {
        var pieces = LaidOut("**bold** and `code` and ~~gone~~");
        Assert.True(Find(pieces, "bold").Style.Strong);
        Assert.True(Find(pieces, "code").Style.Code);
        Assert.True(Find(pieces, "gone").Style.Strikethrough);
    }

    /// <summary>A detected link is a link; the author's markdown destination wins over anything detected in its label.</summary>
    [Fact]
    public void LinksAreFoundAndTheAuthorsDestinationWins()
    {
        Assert.Equal("https://example.com/x", Find(LaidOut("see https://example.com/x now"), "https://example.com/x").Link);
        Assert.Equal("https://b.com", LaidOut("[https://a.com](https://b.com)")[0].Link);
        Assert.Equal("https://example.com", LaidOut("[shop](example.com)")[0].Link);
    }

    /// <summary>A link that is not http(s), mail or a phone number never becomes one.</summary>
    [Fact]
    public void OnlyWebMailAndPhoneLinksAreLinks()
    {
        foreach (var body in new[] { "[click](javascript:alert(1))", "javascript://%0Aalert(1)", "[x](data:text/html,hi)" })
        {
            Assert.All(LaidOut(body), piece => Assert.Null(piece.Link));
        }
        Assert.Equal("mailto:me@example.com", Find(LaidOut("write to me@example.com"), "me@example.com").Link);
        Assert.Equal("tel:5551234567", Find(LaidOut("ring 555-123-4567"), "555-123-4567").Link);
    }

    /// <summary>The assistant's tokens are bold and REPLACE the run's style — and <c>/draw</c> is marked only as the body's first word.</summary>
    [Fact]
    public void TheAssistantsTokensAreMarkedTheAppsWay()
    {
        var ai = Find(LaidOut("ask *@ai* please"), "@ai");
        Assert.Equal(BodyMark.Assistant, ai.Mark);
        Assert.True(ai.Style.Strong);
        Assert.False(ai.Style.Emphasis);

        // Swift's own vectors (MessageLinksTests.drawMarkComesFromTheRawBody).
        (string Body, string[] Marked)[] vectors =
        [
            ("**/draw** a cat", []),
            ("*/draw* a cat", []),
            ("`/draw` a cat", []),
            ("~~/draw~~ a cat", []),
            ("# /draw a cat", []),
            ("- /draw a cat", []),
            ("/draw a cat", ["/draw"]),
            ("/draw a **fluffy** cat", ["/draw"]),
            ("@ai /draw a cat", ["@ai", "/draw"]),
        ];
        foreach (var (body, marked) in vectors)
        {
            Assert.Equal(marked, LaidOut(body).Where(piece => piece.Mark == BodyMark.Assistant).Select(piece => piece.Text).ToArray());
        }
    }

    /// <summary>The heading's face survives the mark: only the inline style is replaced.</summary>
    [Fact]
    public void AMarkKeepsTheFace()
    {
        var ai = Find(LaidOut("# ask @ai"), "@ai");
        Assert.Equal(MarkdownFace.Heading1, ai.Style.Face);
        Assert.True(ai.Style.Strong);
    }

    /// <summary>Only the first block of a table-split body may carry <c>/draw</c>.</summary>
    [Fact]
    public void OnlyTheFirstBlockCarriesThePictureMark()
    {
        const string Body = "| a |\n| --- |\n| 1 |\n/draw a cat";
        var blocks = Markdown.Blocks(Body);
        Assert.Contains(blocks, block => block is MarkdownTableBlock);
        for (var index = 0; index < blocks.Count; index++)
        {
            if (blocks[index] is MarkdownTextBlock { Text: var text })
            {
                Assert.All(MessageBody.Pieces(text, index == 0 ? Body : null, []), piece => Assert.Null(piece.Mark));
            }
        }
    }

    /// <summary>Mentions come last and never inside a link.</summary>
    [Fact]
    public void MentionsAreMarkedUnlessALinkCoversThem()
    {
        var anna = Find(LaidOut("**@Anna** dinner?", new Named(9, "Anna")), "@Anna");
        Assert.Equal(BodyMark.Member(9), anna.Mark);
        Assert.True(anna.Style.Strong);

        Assert.All(LaidOut("[@Anna](https://x.com)", new Named(9, "Anna")), piece => Assert.Null(piece.Mark));
    }

    /// <summary>A label split into runs is still one link.</summary>
    [Fact]
    public void ALabelSplitIntoRunsIsOneLink()
    {
        var declared = MessageBody.Declared(Markdown.Render("*[a](https://e.com)*[b](https://e.com) [c](https://f.com)"));
        Assert.Equal([("ab", "https://e.com"), ("c", "https://f.com")], declared.Select(span => (span.Text, span.Target)).ToArray());
        Assert.Equal((0, 2), (declared[0].Start, declared[0].End));
    }

    /// <summary>Every piece boundary is a character boundary, in any script.</summary>
    [Fact]
    public void PiecesNeverCutInsideACharacter()
    {
        const string Body = "Привет @Ана́, **смотри** https://пример.рф/путь 😀 @ai 🇷🇸";
        var pieces = LaidOut(Body, new Named(9, "Ана"));
        Assert.Equal(Markdown.Render(Body).Plain, string.Concat(pieces.Select(piece => piece.Text)));
        // Past an emoji, a flag and a combining mark, the offsets still land on the tokens themselves.
        Assert.Equal(BodyMark.Assistant, Find(pieces, "@ai").Mark);
        Assert.Equal(BodyMark.Member(9), Find(pieces, "@Ана́").Mark);
        // The parser's own autolink made it a markdown link, so its destination — as typed — is the one that stands.
        Assert.Equal("https://пример.рф/путь", Find(pieces, "https://пример.рф/путь").Link);
    }
}
