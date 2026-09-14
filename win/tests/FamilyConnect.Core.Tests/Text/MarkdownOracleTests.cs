using System.Text.Json;
using System.Text.Json.Nodes;
using FamilyConnect.Core;

namespace FamilyConnect.Core.Tests.Text;

/// <summary>
/// THE THIRD DIFFERENTIAL ORACLE: a message body as a bubble draws it. Every vector in <c>Fixtures/markdown-vectors.json</c>
/// was produced by <c>fc_text::markdown</c>, <c>fc_text::links</c> and <c>fc_text::assistant</c> themselves, over every string
/// literal in those modules' own tests and a fixed-seed run of markup-heavy bodies; this suite holds the C# port to them.
/// </summary>
/// <remarks>
/// Regenerate with <c>python3 win/tools/markdown/transcribe.py</c> (tables and corpus), then <c>cd win/tools/board-oracle
/// &amp;&amp; cargo run --quiet -- markdown corpus/markdown.json &gt; ../../tests/FamilyConnect.Core.Tests/Fixtures/markdown-vectors.json</c>.
/// </remarks>
public class MarkdownOracleTests
{
    private static readonly JsonArray Cases = (JsonArray)JsonNode.Parse(
        File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "Fixtures", "markdown-vectors.json")))!["cases"]!;

    private static JsonNode? Link(MarkdownLink? link) =>
        link is null ? null : new JsonObject { ["destination"] = link.Destination, ["title"] = link.Title };

    private static JsonArray Text(MarkdownText text) =>
    [
        .. text.Runs.Select(run => (JsonNode)new JsonObject
        {
            ["text"] = run.Text,
            ["emphasis"] = run.Style.Emphasis,
            ["strong"] = run.Style.Strong,
            ["code"] = run.Style.Code,
            ["strikethrough"] = run.Style.Strikethrough,
            ["line_break"] = run.Style.LineBreak,
            ["html"] = run.Style.Html,
            ["link"] = Link(run.Style.Link),
            ["image"] = Link(run.Style.Image),
            ["font"] = run.Style.Face switch
            {
                MarkdownFace.Heading1 => "h1",
                MarkdownFace.Heading2 => "h2",
                MarkdownFace.Heading3 => "h3",
                MarkdownFace.Monospaced => "mono",
                _ => "body",
            },
        }),
    ];

    private static JsonArray Blocks(string body) =>
    [
        .. Markdown.Blocks(body).Select(block => (JsonNode)(block switch
        {
            MarkdownTableBlock { Table: var table } => new JsonObject
            {
                ["table"] = new JsonObject
                {
                    ["alignments"] = new JsonArray([.. table.Alignments.Select(alignment => (JsonNode)(alignment switch
                    {
                        MarkdownAlignment.Center => "center",
                        MarkdownAlignment.Trailing => "trailing",
                        _ => "leading",
                    }))]),
                    ["header"] = new JsonArray([.. table.Header.Select(cell => (JsonNode)Text(cell))]),
                    ["rows"] = new JsonArray([.. table.Rows.Select(row => (JsonNode)new JsonArray([.. row.Select(cell => (JsonNode)Text(cell))]))]),
                },
            },
            MarkdownTextBlock { Text: var text } => new JsonObject { ["text"] = Text(text) },
            _ => throw new InvalidOperationException(),
        })),
    ];

    private static JsonArray Spans(IEnumerable<LinkSpan> spans) =>
    [
        .. spans.Select(span => (JsonNode)new JsonObject
        {
            ["start"] = span.Start,
            ["end"] = span.End,
            ["text"] = span.Text,
            ["target"] = span.Target,
        }),
    ];

    private static JsonArray? Range((int Start, int End)? range) => range is { } found ? [found.Start, found.End] : null;

    /// <summary>Structural equality, numbers and booleans by their JSON spelling.</summary>
    private static bool Same(JsonNode? expected, JsonNode? actual) => (expected, actual) switch
    {
        (null, null) => true,
        (JsonObject x, JsonObject y) => x.Count == y.Count && x.All(pair => y.TryGetPropertyValue(pair.Key, out var value) && Same(pair.Value, value)),
        (JsonArray x, JsonArray y) => x.Count == y.Count && x.Zip(y).All(pair => Same(pair.First, pair.Second)),
        (JsonValue x, JsonValue y) => x.GetValueKind() == y.GetValueKind()
            && (x.GetValueKind() == JsonValueKind.String ? x.GetValue<string>() == y.GetValue<string>() : x.ToJsonString() == y.ToJsonString()),
        _ => false,
    };

    /// <summary>Every case, the port against the original; the first differences named so a failure says what broke.</summary>
    private static void Check(string aspect, Func<JsonNode, string, (JsonNode? Expected, JsonNode? Actual)> pick)
    {
        Assert.True(Cases.Count > 2000, "the corpus is missing");
        var failures = new List<string>();
        foreach (var item in Cases)
        {
            var body = item!["body"]!.GetValue<string>();
            var (expected, actual) = pick(item, body);
            if (!Same(expected, actual))
            {
                failures.Add($"{JsonValue.Create(body).ToJsonString()}\n  expected {expected?.ToJsonString() ?? "null"}\n  actual   {actual?.ToJsonString() ?? "null"}");
            }
        }
        Assert.True(failures.Count == 0, $"{aspect}: {failures.Count} of {Cases.Count} differ\n{string.Join("\n", failures.Take(12))}");
    }

    /// <summary>The corpus reaches what it claims to: tables, links, the assistant's tokens.</summary>
    [Fact]
    public void TheCorpusCoversEveryPart()
    {
        Assert.True(Cases.Count(item => item!["blocks"]!.AsArray().Any(block => block!["table"] is not null)) >= 20, "tables");
        Assert.True(Cases.Count(item => item!["merged"]!.AsArray().Count > 0) >= 500, "links");
        Assert.True(Cases.Count(item => item!["render"]!.AsArray().Any(run => run!["link"] is not null)) >= 100, "markdown links");
        Assert.True(Cases.Count(item => item!["assistant_ranges"]!.AsArray().Count > 0) >= 100, "@ai");
        Assert.True(Cases.Count(item => item!["draw_range"] is not null) >= 10, "/draw");
    }

    /// <summary>The blocks a bubble draws: runs and their styles, headings, bullets, fences and tables.</summary>
    [Fact]
    public void BlocksAreTheOriginals() => Check("blocks", (item, body) => (item["blocks"], Blocks(body)));

    /// <summary>The flat render the preview card and the link pass index.</summary>
    [Fact]
    public void TheFlatRenderIsTheOriginals() => Check("render", (item, body) => (item["render"], Text(Markdown.Render(body))));

    /// <summary>Links, addresses and phone numbers, over the raw body and over the rendered text.</summary>
    [Fact]
    public void DetectionIsTheOriginals()
    {
        Check("detect (raw)", (item, body) => (item["detect_body"], Spans(Links.Detect(body))));
        Check("detect (rendered)", (item, body) => (item["detect"], Spans(Links.Detect(Markdown.Render(body).Plain))));
    }

    /// <summary>The author's links merged over the detector's, and the one https link the preview card describes.</summary>
    [Fact]
    public void MergedLinksAndThePreviewLinkAreTheOriginals()
    {
        static IReadOnlyList<LinkSpan> Merged(string body)
        {
            var rendered = Markdown.Render(body);
            return Links.Merge(MessageBody.Declared(rendered), Links.Detect(rendered.Plain));
        }
        Check("merged", (item, body) => (item["merged"], Spans(Merged(body))));
        Check("first web link", (item, body) => (item["first_web"], Links.FirstWebLink(Merged(body)) is { } first ? JsonValue.Create(first.Target) : null));
    }

    /// <summary>Every markdown destination, normalised, and whether it may be opened.</summary>
    [Fact]
    public void DestinationsNormaliseAsTheOriginal() => Check("normalized", (item, body) => (item["normalized"],
        new JsonArray([.. Markdown.Render(body).Runs.Where(run => run.Style.Link is not null).Select(run => (JsonNode)new JsonObject
        {
            ["destination"] = run.Style.Link!.Destination,
            ["normalized"] = Links.NormalizeDestination(run.Style.Link.Destination),
            ["openable"] = Links.IsOpenable(Links.NormalizeDestination(run.Style.Link.Destination)),
        })])));

    /// <summary><c>@ai</c> ranges and the <c>/draw</c> token over the rendered text, and whether the raw body asks.</summary>
    [Fact]
    public void AssistantTokensAreTheOriginals()
    {
        Check("assistant ranges", (item, body) => (item["assistant_ranges"],
            new JsonArray([.. AssistantText.Ranges(Markdown.Render(body).Plain).Select(range => (JsonNode)new JsonArray(range.Start, range.End))])));
        Check("draw range", (item, body) => (item["draw_range"], Range(AssistantText.DrawTokenRange(Markdown.Render(body).Plain))));
        Check("draw asked", (item, body) => (item["draw_asked"], JsonValue.Create(AssistantText.DrawTokenRange(body) is not null)));
    }
}
