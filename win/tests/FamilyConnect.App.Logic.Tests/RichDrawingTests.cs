using System.Text.Json;
using FamilyConnect.Core;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// The pure half of drawing formulas, diagrams and graphs on Windows (issue #97; docs/capitals-and-rich-markdown-2026-10-17.md,
/// "The native host"): the job table, the cache key, the page's answers, the photograph's scale, and the two caches.
/// </summary>
public sealed class RichDrawingTests : IDisposable
{
    private const string Version = "1;katex-0.17.0;mermaid-11.16.0;viz-3.24.0;plot-md-1.6";

    private static readonly RichLook Mine = new(Dark: false, "#FFFFFF", 14);

    private static readonly RichLook TheirsDark = new(Dark: true, "#FFFFFF", 14);

    private readonly string folder = Path.Combine(Path.GetTempPath(), "fc-rich-" + Guid.NewGuid().ToString("N"));

    public void Dispose()
    {
        if (Directory.Exists(folder))
        {
            Directory.Delete(folder, recursive: true);
        }
    }

    /// <summary>The design doc's job table, row for row: a formula in the bubble's ink on nothing; a diagram on the fixed card.</summary>
    [Fact]
    public void JobsFollowTheTable()
    {
        Assert.Equal(new RichJob("math", "", "x^2", false, "#FFFFFF", "transparent", 14), RichJob.ForBlock(RichKind.Math, "", "x^2", Mine));
        Assert.Equal(new RichJob("math-inline", "", "x", false, "#FFFFFF", "transparent", 14), RichJob.ForInline("x", Mine));
        Assert.Equal(new RichJob("mermaid", "", "graph TD", false, "#1C1C1E", "#FFFFFF", 14), RichJob.ForBlock(RichKind.Mermaid, "", "graph TD", Mine));
        Assert.Equal(new RichJob("graphviz", "neato", "graph{}", true, "#F2F2F7", "#1C1C1E", 14), RichJob.ForBlock(RichKind.Graphviz, "neato", "graph{}", TheirsDark));
        // An engine on anything but Graphviz is nobody's business.
        Assert.Equal("", RichJob.ForBlock(RichKind.Plot, "dot", "sin(x)", Mine).Engine);
        Assert.True(RichJob.ForInline("x", Mine).IsFormula);
        Assert.False(RichJob.ForBlock(RichKind.Plot, "", "x", Mine).IsFormula);
    }

    /// <summary>The key moves with every field the doc lists, and with nothing else.</summary>
    [Fact]
    public void TheKeyCoversEveryField()
    {
        var job = RichJob.ForBlock(RichKind.Math, "", "x^2", Mine);
        var key = job.Key(Version);
        Assert.Matches("^[0-9a-f]{64}$", key);
        Assert.Equal(key, RichJob.ForBlock(RichKind.Math, "", "x^2", Mine).Key(Version));
        RichJob[] others =
        [
            job with { Kind = "math-inline" },
            job with { Engine = "dot" },
            job with { Dark = true },
            job with { Color = "#000000" },
            job with { Background = "#FFFFFF" },
            job with { FontSize = 15 },
            job with { Source = "x^3" },
        ];
        Assert.All(others, other => Assert.NotEqual(key, other.Key(Version)));
        Assert.NotEqual(key, job.Key(Version + "x"));
    }

    /// <summary>Fields go in with their lengths, so moving a character from one field to the next is a different key.</summary>
    [Fact]
    public void TheKeyCannotBeShiftedBetweenFields()
    {
        var a = new RichJob("math", "a", "bc", false, "#FFF", "transparent", 14);
        var b = new RichJob("math", "ab", "c", false, "#FFF", "transparent", 14);
        Assert.NotEqual(a.Key(Version), b.Key(Version));
    }

    /// <summary>The job reaches the page as the object fc-rich.js reads, and a source can never break out of it.</summary>
    [Fact]
    public void TheJobIsSafeJson()
    {
        var hostile = "</script><script>alert('x')</script>\"\u2028`${x}`";
        var job = RichJob.ForBlock(RichKind.Graphviz, "", hostile, Mine);
        var json = job.Json();
        Assert.DoesNotContain("<", json);
        Assert.DoesNotContain("'", json);
        Assert.DoesNotContain("\u2028", json);
        using var parsed = JsonDocument.Parse(json);
        var root = parsed.RootElement;
        Assert.Equal("graphviz", root.GetProperty("kind").GetString());
        Assert.Equal(hostile, root.GetProperty("source").GetString());
        // Graphviz with no layout named is dot.
        Assert.Equal("dot", root.GetProperty("engine").GetString());
        Assert.False(root.GetProperty("dark").GetBoolean());
        Assert.Equal(14, root.GetProperty("fontSize").GetDouble());
        Assert.False(root.TryGetProperty("mathml", out _));
        Assert.False(JsonDocument.Parse(RichJob.ForInline("x", Mine).Json()).RootElement.TryGetProperty("engine", out _));
        Assert.StartsWith("fcRich.drawAndPost(7,{", job.Script(7));
        Assert.EndsWith("})", job.Script(7));
    }

    [Theory]
    [InlineData(0xFF, 0xFF, 0xFF, 0xFF, "#FFFFFF")]
    [InlineData(0xE4, 0x00, 0x00, 0x00, "#000000E4")]
    [InlineData(0xFF, 0x1C, 0x1C, 0x1E, "#1C1C1E")]
    public void ColoursAreCss(byte a, byte r, byte g, byte b, string css) => Assert.Equal(css, RichJob.CssColor(a, r, g, b));

    /// <summary>The page's answers: a drawing, a failure in the engine's words, and the shapes that are neither.</summary>
    [Fact]
    public void AnswersParse()
    {
        Assert.Equal((3, new RichResult(true, 120, 40, 30)),
            RichAnswer.Parse("{\"id\":3,\"result\":{\"ok\":true,\"width\":120,\"height\":40,\"baseline\":30}}"));
        var failed = RichAnswer.Parse("{\"id\":4,\"result\":{\"ok\":false,\"error\":\"KaTeX parse error\"}}");
        Assert.Equal(4, failed?.Id);
        Assert.False(failed?.Result.Ok);
        Assert.Equal("KaTeX parse error", failed?.Result.Error);
        // Drew, it says, but nothing to photograph.
        Assert.Equal("empty", RichAnswer.Parse("{\"id\":5,\"result\":{\"ok\":true,\"width\":0,\"height\":40}}")?.Result.Error);
        // No baseline (a diagram) is its foot; one outside the box is put back inside it.
        Assert.Equal(40, RichAnswer.Parse("{\"id\":6,\"result\":{\"ok\":true,\"width\":10,\"height\":40}}")?.Result.Baseline);
        Assert.Equal(40, RichAnswer.Parse("{\"id\":6,\"result\":{\"ok\":true,\"width\":10,\"height\":40,\"baseline\":90}}")?.Result.Baseline);
        Assert.Equal(0, RichAnswer.Parse("{\"id\":6,\"result\":{\"ok\":true,\"width\":10,\"height\":40,\"baseline\":-3}}")?.Result.Baseline);
        Assert.Null(RichAnswer.Parse("not json"));
        Assert.Null(RichAnswer.Parse("{\"event\":\"ready\"}"));
        Assert.Null(RichAnswer.Parse("{\"id\":\"3\",\"result\":{}}"));
        Assert.Null(RichAnswer.Parse("[1,2]"));
    }

    [Fact]
    public void MetricsRoundTrip()
    {
        var drawn = new RichResult(true, 120.5, 40, 31.25);
        Assert.Equal(drawn, RichAnswer.ReadMetrics(RichAnswer.Metrics(drawn)));
        Assert.Null(RichAnswer.ReadMetrics("{\"width\":0,\"height\":1}"));
        Assert.Null(RichAnswer.ReadMetrics("garbage"));
    }

    /// <summary>Only the page's own folder, and data: URLs — nothing that leaves the machine.</summary>
    [Theory]
    [InlineData("https://rich.familyconnect.invalid/rich.html", true)]
    [InlineData("https://rich.familyconnect.invalid/fonts/KaTeX_Main-Regular.woff2", true)]
    [InlineData("https://RICH.familyconnect.invalid/katex.min.js", true)]
    [InlineData("data:image/png;base64,AAAA", true)]
    [InlineData("http://rich.familyconnect.invalid/rich.html", false)]
    [InlineData("https://rich.familyconnect.invalid:8443/rich.html", false)]
    [InlineData("https://call.familyconnect.invalid/call.html", false)]
    [InlineData("https://fc.nettrash.me/", false)]
    [InlineData("https://rich.familyconnect.invalid.example.com/x", false)]
    [InlineData("blob:https://rich.familyconnect.invalid/1", false)]
    [InlineData("file:///C:/Windows/win.ini", false)]
    [InlineData("", false)]
    [InlineData(null, false)]
    public void ThePageReachesOnlyItsOwnFolder(string? uri, bool allowed) => Assert.Equal(allowed, RichPage.Allows(uri));

    [Fact]
    public void NavigationIsToThePageAlone()
    {
        Assert.True(RichPage.IsPage("https://rich.familyconnect.invalid/rich.html"));
        Assert.False(RichPage.IsPage("https://rich.familyconnect.invalid/fc-rich.js"));
        Assert.False(RichPage.IsPage("https://rich.familyconnect.invalid/rich.html?x"));
        Assert.False(RichPage.IsPage("about:blank"));
        Assert.False(RichPage.IsPage(null));
    }

    /// <summary>The version is read out of the very file the app ships — so every copy gets the same key as web/rich.</summary>
    [Fact]
    public void TheVersionIsReadFromTheShippedScript()
    {
        Assert.Equal("1;a", RichPage.VersionOf("  var VERSION = '1;a';\n"));
        Assert.Null(RichPage.VersionOf("var VERSIONS = 'x';"));
        Assert.Null(RichPage.VersionOf("nothing here"));
        var script = File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "Fixtures", "fc-rich.js"));
        Assert.Equal(Version, RichPage.VersionOf(script));
    }

    /// <summary>The screen's scale, until the longest side would pass 4096 pixels.</summary>
    [Theory]
    [InlineData(1.5, 200, 100, 1.5)]
    [InlineData(2, 1200, 3000, 4096.0 / 3000)]
    [InlineData(1, 4096, 10, 1)]
    [InlineData(1, 5000, 10, 4096.0 / 5000)]
    [InlineData(0, 100, 100, 1)]
    [InlineData(double.NaN, 100, 100, 1)]
    public void TheScaleIsCapped(double screen, double width, double height, double expected) =>
        Assert.Equal(expected, RichCapture.ScaleFor(screen, width, height), 9);

    [Fact]
    public void TheCaptureAsksForTheDrawingAtTheTopLeft()
    {
        using var arguments = JsonDocument.Parse(RichCapture.Arguments(120, 40, 1.5));
        var root = arguments.RootElement;
        Assert.Equal("png", root.GetProperty("format").GetString());
        Assert.True(root.GetProperty("captureBeyondViewport").GetBoolean());
        var clip = root.GetProperty("clip");
        Assert.Equal((0d, 0d, 120d, 40d, 1.5d),
            (clip.GetProperty("x").GetDouble(), clip.GetProperty("y").GetDouble(), clip.GetProperty("width").GetDouble(),
                clip.GetProperty("height").GetDouble(), clip.GetProperty("scale").GetDouble()));
        using var metrics = JsonDocument.Parse(RichCapture.Metrics());
        Assert.Equal(1, metrics.RootElement.GetProperty("deviceScaleFactor").GetDouble());
        using var clear = JsonDocument.Parse(RichCapture.ClearBackground);
        Assert.Equal(0, clear.RootElement.GetProperty("color").GetProperty("a").GetDouble());
        Assert.Equal(new byte[] { 1, 2, 3 }, RichCapture.Png("{\"data\":\"AQID\"}"));
        Assert.Null(RichCapture.Png("{\"data\":\"***\"}"));
        Assert.Null(RichCapture.Png("{}"));
        Assert.Null(RichCapture.Png("nope"));
    }

    /// <summary>An inline formula drops by what hangs below its baseline, and a too-wide one shrinks whole, baseline with it.</summary>
    [Fact]
    public void InlineFormulasSitOnTheBaseline()
    {
        Assert.Equal((40d, 20d, 5d), RichFit.Inline(40, 20, 15, 500));
        Assert.Equal((500d, 10d, 2.5d), RichFit.Inline(1000, 20, 15, 500));
        Assert.Equal((10d, 20d, 0d), RichFit.Inline(10, 20, 20, 500));
    }

    [Fact]
    public void MemoryLetsTheLeastRecentlyUsedGo()
    {
        var memory = new RichMemory<int>(2);
        memory.Put("a", 1);
        memory.Put("b", 2);
        Assert.True(memory.TryGet("a", out _));
        memory.Put("c", 3);
        Assert.False(memory.TryGet("b", out _));
        Assert.True(memory.TryGet("a", out var a));
        Assert.Equal(1, a);
        memory.Put("a", 9);
        Assert.Equal(2, memory.Count);
        Assert.True(memory.TryGet("a", out a));
        Assert.Equal(9, a);
        memory.Clear();
        Assert.Equal(0, memory.Count);
        Assert.Throws<ArgumentOutOfRangeException>(() => new RichMemory<int>(0));
    }

    /// <summary>Pixels are the budget too: a heavy drawing pushes the old ones out, and the newest always stays.</summary>
    [Fact]
    public void MemoryKeepsToItsWeight()
    {
        var memory = new RichMemory<string>(10, budget: 100);
        memory.Put("a", "a", 40);
        memory.Put("b", "b", 40);
        memory.Put("failed", "failed", 0);
        memory.Put("c", "c", 40);
        Assert.False(memory.TryGet("a", out _));
        Assert.True(memory.TryGet("b", out _));
        Assert.Equal(80, memory.Weight);
        memory.Put("huge", "huge", 500);
        Assert.Equal(1, memory.Count);
        Assert.True(memory.TryGet("huge", out _));
        memory.Put("huge", "smaller", 10);
        Assert.Equal(10, memory.Weight);
    }

    [Fact]
    public void TheDiskKeepsDrawingsWithTheirMetrics()
    {
        var store = new RichStore(folder);
        var key = RichJob.ForInline("x", Mine).Key(Version);
        Assert.Null(store.Read(key));
        store.Write(key, [1, 2, 3], new RichResult(true, 10, 20, 15));
        var read = store.Read(key);
        Assert.Equal(new byte[] { 1, 2, 3 }, read?.Png);
        Assert.Equal(new RichResult(true, 10, 20, 15), read?.Metrics);
        // A failure is never kept on disk, nor is a key that is not one.
        var other = RichJob.ForInline("y", Mine).Key(Version);
        store.Write(other, [1], RichResult.Failed("bad"));
        Assert.Null(store.Read(other));
        store.Write("../escape", [1], new RichResult(true, 1, 1, 1));
        Assert.Empty(Directory.EnumerateFiles(Path.GetDirectoryName(folder)!, "escape*"));
        Assert.Null(store.Read("../escape"));
    }

    [Fact]
    public void ABrokenDrawingIsDrawnAgain()
    {
        var store = new RichStore(folder);
        var key = RichJob.ForInline("x", Mine).Key(Version);
        store.Write(key, [1, 2, 3], new RichResult(true, 10, 20, 15));
        File.WriteAllText(Path.Combine(folder, key + ".json"), "{broken");
        Assert.Null(store.Read(key));
        Assert.False(File.Exists(Path.Combine(folder, key + ".png")));
    }

    [Fact]
    public void TrimKeepsTheMostRecentlyUsed()
    {
        var store = new RichStore(folder);
        var keys = Enumerable.Range(0, 4).Select(index => RichJob.ForInline($"x{index}", Mine).Key(Version)).ToArray();
        for (var index = 0; index < keys.Length; index++)
        {
            store.Write(keys[index], [1], new RichResult(true, 1, 1, 1));
            File.SetLastWriteTimeUtc(Path.Combine(folder, keys[index] + ".json"), DateTime.UtcNow.AddMinutes(index - 10));
        }
        // Reading the oldest makes it the newest.
        Assert.NotNull(store.Read(keys[0]));
        File.WriteAllBytes(Path.Combine(folder, "stray.png.part"), [1]);
        File.WriteAllBytes(Path.Combine(folder, new string('a', 64) + ".png"), [1]);
        store.Trim(keep: 2);
        Assert.NotNull(store.Read(keys[0]));
        Assert.NotNull(store.Read(keys[3]));
        Assert.Null(store.Read(keys[1]));
        Assert.Null(store.Read(keys[2]));
        Assert.Equal(4, Directory.EnumerateFiles(folder).Count());
    }
}
