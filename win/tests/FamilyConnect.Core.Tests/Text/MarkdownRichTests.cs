using FamilyConnect.Core;

namespace FamilyConnect.Core.Tests.Text;

/// <summary>
/// Formulas, diagrams and graphs (issue #97): docs/capitals-and-rich-markdown-2026-10-17.md, "The cases every parser's tests
/// hold", row for row — the same tables <c>fc_text::markdown</c>'s tests hold (the oracle vectors hold the rest), plus what a
/// bubble's composition must leave alone around a formula.
/// </summary>
public class MarkdownRichTests
{
    /// <summary>A body's blocks as words: <c>text &lt;plain&gt;</c>, <c>table</c>, or <c>rich &lt;kind&gt; &lt;engine&gt; &lt;source&gt;</c>.</summary>
    private static string[] Shape(string body) =>
    [
        .. Markdown.Blocks(body).Select(block => block switch
        {
            MarkdownTextBlock { Text: var text } => $"text {text.Plain}",
            MarkdownTableBlock => "table",
            MarkdownRichBlock rich => $"rich {rich.Kind.Word()} {rich.Engine} {rich.Source}",
            _ => throw new InvalidOperationException(),
        }),
    ];

    /// <summary>Every formula a body's text and table cells carry, in order.</summary>
    private static string[] Formulas(string body)
    {
        var output = new List<string>();
        void Of(MarkdownText text) => output.AddRange(text.Runs.Select(run => run.Style.Math).OfType<string>());
        foreach (var block in Markdown.Blocks(body))
        {
            switch (block)
            {
                case MarkdownTextBlock { Text: var text }:
                    Of(text);
                    break;
                case MarkdownTableBlock { Table: var table }:
                    foreach (var cell in table.Header.Concat(table.Rows.SelectMany(row => row)))
                    {
                        Of(cell);
                    }
                    break;
            }
        }
        return [.. output];
    }

    [Theory]
    [InlineData("```math\nx^2\n```", new[] { "rich math  x^2" })]
    // The tag is case-blind.
    [InlineData("```Mermaid\ngraph TD\nA-->B\n```", new[] { "rich mermaid  graph TD\nA-->B" })]
    // The first word decides.
    [InlineData("```mermaid extra words\nA\n```", new[] { "rich mermaid  A" })]
    [InlineData("```neato\ngraph {a--b}\n```", new[] { "rich graphviz neato graph {a--b}" })]
    [InlineData("```gv\ndigraph{a->b}\n```", new[] { "rich graphviz dot digraph{a->b}" })]
    [InlineData("```plot\nsin(x)\n```", new[] { "rich plot  sin(x)" })]
    // Any other tag is a code block, as before.
    [InlineData("```js\nx\n```", new[] { "text x" })]
    // Never closed: text, exactly as typed.
    [InlineData("```mermaid\nA", new[] { "text ```mermaid\nA" })]
    [InlineData("Look:\n```math\nx\n```\nDone", new[] { "text Look:", "rich math  x", "text Done" })]
    [InlineData("$$\nx^2\n$$", new[] { "rich math  x^2" })]
    [InlineData("$$x^2$$", new[] { "rich math  x^2" })]
    [InlineData("\\[\na\n\\]", new[] { "rich math  a" })]
    [InlineData("  $$ y $$  ", new[] { "rich math  y" })]
    [InlineData("$$x$$ is it", new[] { "text $$x$$ is it" })]
    [InlineData("Area $$x$$", new[] { "text Area $$x$$" })]
    [InlineData("$$\nx", new[] { "text $$\nx" })]
    [InlineData("$$ $$", new[] { "text $$ $$" })]
    // Nothing inside a fence is math.
    [InlineData("```\n$$x$$\n```", new[] { "text $$x$$" })]
    public void RichBlocksTheDesignTable(string body, string[] expected) => Assert.Equal(expected, Shape(body));

    [Fact]
    public void TheBlocksThatStayTextCarryTheirFormulaInline()
    {
        Assert.Equal(["x"], Formulas("$$x$$ is it"));
        Assert.Equal(["x"], Formulas("Area $$x$$"));
        Assert.Empty(Formulas("```\n$$x$$\n```"));
        Assert.Empty(Formulas("$$ $$"));
        var code = Assert.IsType<MarkdownTextBlock>(Assert.Single(Markdown.Blocks("```js\nx\n```"))).Text;
        Assert.Equal(MarkdownFace.Monospaced, code.Runs[0].Style.Face);
    }

    [Theory]
    [InlineData("math", "math", "")]
    [InlineData("latex", "math", "")]
    [InlineData("tex", "math", "")]
    [InlineData("mermaid", "mermaid", "")]
    [InlineData("dot", "graphviz", "dot")]
    [InlineData("graphviz", "graphviz", "dot")]
    [InlineData("gv", "graphviz", "dot")]
    [InlineData("neato", "graphviz", "neato")]
    [InlineData("circo", "graphviz", "circo")]
    [InlineData("fdp", "graphviz", "fdp")]
    [InlineData("sfdp", "graphviz", "sfdp")]
    [InlineData("twopi", "graphviz", "twopi")]
    [InlineData("osage", "graphviz", "osage")]
    [InlineData("patchwork", "graphviz", "patchwork")]
    [InlineData("plot", "plot", "")]
    public void EveryRichTag(string tag, string kind, string engine)
    {
        Assert.Equal([$"rich {kind} {engine} src"], Shape($"```{tag}\nsrc\n```"));
        Assert.Equal([$"rich {kind} {engine} src"], Shape($"```{tag.ToUpperInvariant()}\nsrc\n```"));
    }

    /// <summary>A tag is a whole word, lower-cased in ASCII only; the fence's extra backticks and blanks are not part of it.</summary>
    [Fact]
    public void TagNearMisses()
    {
        foreach (var body in new[] { "```mathx\nsrc\n```", "```ma th\nsrc\n```", "```\U0001D426ath\nsrc\n```", "```{math}\nsrc\n```", "```plots\nsrc\n```" })
        {
            Assert.Equal(["text src"], Shape(body));
        }
        Assert.Equal(["rich math  src"], Shape("```  math\t\nsrc\n```"));
        Assert.Equal(["rich math  src"], Shape("````math\nsrc\n```"));
        Assert.Equal(["rich math  src"], Shape("\u1FEF\u1FEF\u1FEFmath\nsrc\n```"));
        Assert.Equal(["rich math  "], Shape("```math\n```"));
    }

    /// <summary>A rich block splits the text as a table does, and display math comes before tables.</summary>
    [Fact]
    public void RichBlocksSplitLikeTables()
    {
        Assert.Equal(["text a\nx", "rich math  y", "text b"], Shape("a\n```js\nx\n```\n```math\ny\n```\nb"));
        Assert.Equal(["text one\n", "rich math  x", "text \ntwo"], Shape("one\n\n$$x$$\n\ntwo"));
        Assert.Equal(["table", "rich math  x | y"], Shape("| a | b |\n|---|---|\n$$ x | y $$"));
        Assert.Equal(["text $$a$$b$$"], Shape("$$a$$b$$"));
        Assert.Equal(["text $$a", "rich math  b"], Shape("$$a\n$$b$$"));
        Assert.Equal(["rich math  a +\n\tb"], Shape("$$ a +\n\tb \n$$"));
    }

    /// <summary>
    /// What a bubble shows until a block is drawn: a fence's source, as a code block shows it, but a display formula's lines as
    /// typed — delimiters, spaces and all — which the source, trimmed and without them, cannot give back.
    /// </summary>
    [Theory]
    [InlineData("```math\nx^2\n```", "x^2")]
    [InlineData("```mermaid\ngraph TD\nA-->B\n```", "graph TD\nA-->B")]
    [InlineData("$$x^2$$", "$$x^2$$")]
    [InlineData("$$\nx^2\n$$", "$$\nx^2\n$$")]
    [InlineData("\\[\na\n\\]", "\\[\na\n\\]")]
    [InlineData("  $$ y $$  ", "$$ y $$")]
    [InlineData("\t\\[ x \\]\t", "\\[ x \\]")]
    [InlineData("$$ a +\n\tb \n$$ ", "$$ a +\n\tb \n$$")]
    [InlineData("Look:\n$$ a +\n\tb \n$$\nDone", "$$ a +\n\tb \n$$")]
    [InlineData("€ before\n$$ é $$", "$$ é $$")]
    public void ARichBlockShowsWhatWasTyped(string body, string shown) =>
        Assert.Equal(shown, Assert.Single(Markdown.Blocks(body).OfType<MarkdownRichBlock>()).Shown);

    /// <summary>The flat render recognises no rich block — and an inline formula is still a run.</summary>
    [Fact]
    public void TheFlatRenderDrawsNoRichBlock()
    {
        Assert.Equal("Look:\nx\nDone", Markdown.Render("Look:\n```math\nx\n```\nDone").Plain);
        Assert.Equal("$$\nx\n$$", Markdown.Render("$$\nx\n$$").Plain);
        Assert.Equal("x", Assert.Single(Markdown.Render("$$x$$").Runs).Style.Math);
    }

    [Theory]
    [InlineData("$x$", new[] { "x" })]
    [InlineData("$5 and $10", new string[0])]
    [InlineData("costs $5, or $6.", new string[0])]
    // A word character before the opener.
    [InlineData("a$b$c", new string[0])]
    // A word character after the closer.
    [InlineData("$x$s", new string[0])]
    [InlineData("$ x$", new string[0])]
    [InlineData("$x $", new string[0])]
    // The opener is escaped (the text reads `$x$`).
    [InlineData("\\$x$", new string[0])]
    // The escaped `$` cannot close.
    [InlineData("$a\\$b$", new[] { "a\\$b" })]
    [InlineData("see $$x$$ now", new[] { "x" })]
    [InlineData("\\(x^2\\)", new[] { "x^2" })]
    // Code.
    [InlineData("`$x$`", new string[0])]
    // Inside the emphasis.
    [InlineData("*see $a*b$*", new[] { "a*b" })]
    // A formula does not cross a line.
    [InlineData("$x\ny$", new string[0])]
    // A link's destination.
    [InlineData("[a](http://x.y/$b$)", new string[0])]
    [InlineData("| $a$ | b |\n|---|---|\n| $c$ | d |", new[] { "a", "c" })]
    // `€` is not an ASCII word character.
    [InlineData("€$x$", new[] { "x" })]
    // `2` is.
    [InlineData("2$x$", new string[0])]
    public void InlineMathTheDesignTable(string body, string[] expected) => Assert.Equal(expected, Formulas(body));

    [Fact]
    public void AFormulaIsARunOfItsOwnAndNothingInsideIsMarkdown()
    {
        Assert.Equal("$x$", Markdown.Render("\\$x$").Plain);
        var emphasis = Assert.IsType<MarkdownTextBlock>(Assert.Single(Markdown.Blocks("*see $a*b$*"))).Text;
        Assert.All(emphasis.Runs, run => Assert.True(run.Style.Emphasis));
        Assert.Equal("$a*b$", Assert.Single(emphasis.Runs, run => run.Style.Math is not null).Text);

        var runs = Markdown.Render("Area $\\pi r^2$, or \\(a\\)\\(a\\).").Runs.Select(run => (run.Text, run.Style.Math)).ToArray();
        Assert.Equal(
            [("Area ", null), ("$\\pi r^2$", "\\pi r^2"), (", or ", null), ("\\(a\\)", "a"), ("\\(a\\)", "a"), (".", null)],
            runs);
        foreach (var body in new[] { "$**x**$", "$<b>x</b>$", "$&amp;$", "$https://example.com$", "$me@example.com$", "\\(a`b\\)" })
        {
            var run = Assert.Single(Markdown.Render(body).Runs);
            Assert.Equal(body, run.Text);
            Assert.Null(run.Style.Link);
        }
        // Code spans come first, and a formula never reaches across one.
        Assert.Empty(Formulas("$a `b$` c$"));
        Assert.Equal(["a`b"], Formulas("$a`b$"));
    }

    /// <summary>Non-ASCII around and inside: never a byte offset off a character boundary.</summary>
    [Fact]
    public void FormulasOverNonAscii()
    {
        Assert.Equal(["α+β"], Formulas("Привет $α+β$ мир"));
        Assert.Equal(["привет"], Formulas("\\(привет\\)"));
        Assert.Equal(["😀"], Formulas("😀 $$😀$$ 😀"));
        Assert.Equal(["rich math  Привет"], Shape("$$ Привет $$"));
        Assert.Equal(["x\u0301"], Formulas("$x\u0301$"));
    }

    /// <summary>Nothing is laid over a formula: no detected link, no <c>@ai</c>, no mention — it reaches the bubble whole.</summary>
    [Fact]
    public void ABubbleLaysNothingOverAFormula()
    {
        IReadOnlyList<BodyPiece> Pieces(string body, params Named[] named) =>
            MessageBody.Pieces(Assert.IsType<MarkdownTextBlock>(Markdown.Blocks(body)[0]).Text, body, named);

        var linked = Pieces("see $https://e.com/x$ and https://e.com/y");
        var formula = Assert.Single(linked, piece => piece.Style.Math is not null);
        Assert.Equal("$https://e.com/x$", formula.Text);
        Assert.Null(formula.Link);
        Assert.Equal("https://e.com/y", Assert.Single(linked, piece => piece.Link is not null).Link);

        var marked = Pieces("$@ai$ @ai $@Anna$ @Anna", new Named(7, "Anna"));
        Assert.All(marked.Where(piece => piece.Style.Math is not null), piece => Assert.Null(piece.Mark));
        Assert.Equal(2, marked.Count(piece => piece.Style.Math is not null));
        Assert.Equal(2, marked.Count(piece => piece.Mark is not null));

        // And a URL drawn only inside a formula is not what the preview card describes.
        Assert.Null(MessageBody.FirstWebLink("$https://e.com/x$"));
        Assert.Equal("https://e.com/y", MessageBody.FirstWebLink("$https://e.com/x$ https://e.com/y"));
    }
}
