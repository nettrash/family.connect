using System.Globalization;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Text.RegularExpressions;
using FamilyConnect.Core;

namespace FamilyConnect.App.Logic;

/// <summary>
/// How one bubble wants its formulas, diagrams and graphs drawn (issue #97; docs/capitals-and-rich-markdown-2026-10-17.md, "The
/// native host"): the theme, the bubble's own text colour — an own bubble's white is not another's near-black — and its body text
/// size, in DIPs, which on Windows are the CSS pixels <c>fc-rich.js</c> measures in.
/// </summary>
/// <param name="TextColor">A CSS colour (<see cref="RichJob.CssColor"/>).</param>
public sealed record RichLook(bool Dark, string TextColor, double FontSize);

/// <summary>
/// One job for <c>fc-rich.js</c>, exactly the object its header describes, built from a bubble's block and its
/// <see cref="RichLook"/> by the design doc's job table — and the key a drawing of it is kept under.
/// </summary>
/// <param name="Kind"><c>math</c>, <c>math-inline</c>, <c>mermaid</c>, <c>graphviz</c> or <c>plot</c>.</param>
/// <param name="Engine">Graphviz's layout; empty for every other kind.</param>
/// <param name="Source">What to draw, exactly as written — never the delimiters a bubble shows around it.</param>
/// <param name="Color">The ink: the bubble's text colour for a formula, the card's for a diagram or a graph.</param>
/// <param name="Background"><c>transparent</c> for a formula; the card's colour for a diagram or a graph.</param>
public sealed record RichJob(string Kind, string Engine, string Source, bool Dark, string Color, string Background, double FontSize)
{
    /// <summary>The card a diagram or a graph is drawn on, light and dark — fixed, whoever's bubble it is in (the job table).</summary>
    public const string CardLight = "#FFFFFF";

    public const string CardDark = "#1C1C1E";

    /// <summary>The ink on that card.</summary>
    public const string CardInkLight = "#1C1C1E";

    public const string CardInkDark = "#F2F2F7";

    /// <summary>A formula has no ground of its own: it sits on the bubble's.</summary>
    public const string Transparent = "transparent";

    /// <summary>The job for a block standing on its own: a display formula, a diagram or a graph.</summary>
    public static RichJob ForBlock(RichKind kind, string engine, string source, RichLook look) =>
        kind == RichKind.Math
            ? new RichJob(kind.Word(), string.Empty, source, look.Dark, look.TextColor, Transparent, look.FontSize)
            : new RichJob(
                kind.Word(),
                kind == RichKind.Graphviz ? engine : string.Empty,
                source,
                look.Dark,
                look.Dark ? CardInkDark : CardInkLight,
                look.Dark ? CardDark : CardLight,
                look.FontSize);

    /// <summary>The job for a formula inside a line of text.</summary>
    public static RichJob ForInline(string source, RichLook look) =>
        new("math-inline", string.Empty, source, look.Dark, look.TextColor, Transparent, look.FontSize);

    /// <summary>A formula, drawn in the bubble's ink on nothing; anything else is a diagram or a graph on a card.</summary>
    public bool IsFormula => Kind is "math" or "math-inline";

    /// <summary>
    /// The cache key: SHA-256, in lower-case hex, over <c>fcRich.version</c>, the kind, the engine, the theme, both colours, the
    /// text size and the source — the doc's list, nothing more. Each field goes in with its length in front, so no source can
    /// end one field and start the next (<c>a</c>+<c>bc</c> is never <c>ab</c>+<c>c</c>). The version is what retires every
    /// drawing an older engine made.
    /// </summary>
    public string Key(string version)
    {
        var text = new StringBuilder();
        foreach (var field in new[]
        {
            version, Kind, Engine, Dark ? "1" : "0", Color, Background, FontSize.ToString("R", CultureInfo.InvariantCulture), Source,
        })
        {
            text.Append(field.Length.ToString(CultureInfo.InvariantCulture)).Append(':').Append(field).Append('\n');
        }
        return Convert.ToHexStringLower(SHA256.HashData(Encoding.UTF8.GetBytes(text.ToString())));
    }

    /// <summary>
    /// The job as the JSON object <c>fc-rich.js</c> reads. <c>engine</c> only for Graphviz, which is the only engine that reads
    /// it; never <c>mathml</c>, which would draw a snapshotted formula twice. Written by <see cref="Utf8JsonWriter"/>, whose
    /// default encoder escapes every character that could close a script or a string, so the object is also a safe JavaScript
    /// literal (<see cref="Script"/>).
    /// </summary>
    public string Json()
    {
        using var buffer = new MemoryStream();
        using (var writer = new Utf8JsonWriter(buffer))
        {
            writer.WriteStartObject();
            writer.WriteString("kind", Kind);
            writer.WriteString("source", Source);
            if (Kind == "graphviz")
            {
                writer.WriteString("engine", Engine.Length > 0 ? Engine : "dot");
            }
            writer.WriteBoolean("dark", Dark);
            writer.WriteString("color", Color);
            writer.WriteString("background", Background);
            writer.WriteNumber("fontSize", FontSize);
            writer.WriteEndObject();
        }
        return Encoding.UTF8.GetString(buffer.ToArray());
    }

    /// <summary>
    /// The script that starts the job in the page: <c>fcRich.drawAndPost(id, job)</c>, whose answer comes back as a web message
    /// (<see cref="RichAnswer.Parse"/>) — a WebView2 script call cannot wait for a promise. The job is JSON, which is a JavaScript
    /// expression, so nothing in a source is ever read as code.
    /// </summary>
    public string Script(int id) => string.Create(CultureInfo.InvariantCulture, $"fcRich.drawAndPost({id},{Json()})");

    /// <summary>
    /// A colour as CSS: <c>#RRGGBB</c>, or <c>#RRGGBBAA</c> when it is not opaque — WinUI's own text brushes are (the light theme's
    /// primary text is black at 0xE4), and the formula must be the same ink as the words beside it.
    /// </summary>
    public static string CssColor(byte alpha, byte red, byte green, byte blue) =>
        alpha == 0xFF
            ? string.Create(CultureInfo.InvariantCulture, $"#{red:X2}{green:X2}{blue:X2}")
            : string.Create(CultureInfo.InvariantCulture, $"#{red:X2}{green:X2}{blue:X2}{alpha:X2}");
}

/// <summary>
/// What <c>fc-rich.js</c> answered for one job: the rectangle it drew at the page's top left, in CSS pixels, and for a formula the
/// distance from its top to its baseline — or that it could not draw it.
/// </summary>
/// <param name="Error">
/// The engine's own words. NEVER LOGGED: KaTeX quotes the source it choked on, and no log carries a message's text.
/// </param>
public sealed record RichResult(bool Ok, double Width, double Height, double Baseline, string? Error = null)
{
    public static RichResult Failed(string error) => new(false, 0, 0, 0, error);
}

/// <summary>The answers the page posts back, and the metrics a drawing is kept with on disk.</summary>
public static class RichAnswer
{
    /// <summary>
    /// The page's <c>{id, result}</c> message (<c>fcRich.drawAndPost</c>), or null for anything else. A result that says it drew
    /// but names no positive, finite size is a failure — there is nothing to photograph — and a baseline outside the drawing is
    /// put back inside it: a formula that sank below its own box would sit off the line.
    /// </summary>
    public static (int Id, RichResult Result)? Parse(string json)
    {
        try
        {
            using var document = JsonDocument.Parse(json);
            var root = document.RootElement;
            if (root.ValueKind != JsonValueKind.Object
                || !root.TryGetProperty("id", out var id) || id.ValueKind != JsonValueKind.Number || !id.TryGetInt32(out var number)
                || !root.TryGetProperty("result", out var result) || result.ValueKind != JsonValueKind.Object)
            {
                return null;
            }
            return (number, Result(result));
        }
        catch (JsonException)
        {
            return null;
        }
    }

    /// <summary>A drawing's metrics as they are kept beside its PNG: <c>{"width":…,"height":…,"baseline":…}</c>, CSS pixels.</summary>
    public static string Metrics(RichResult result) =>
        string.Create(CultureInfo.InvariantCulture, $"{{\"width\":{result.Width:R},\"height\":{result.Height:R},\"baseline\":{result.Baseline:R}}}");

    /// <summary>Those metrics read back, or null when the file is not what <see cref="Metrics"/> wrote.</summary>
    public static RichResult? ReadMetrics(string json)
    {
        try
        {
            using var document = JsonDocument.Parse(json);
            var root = document.RootElement;
            if (root.ValueKind != JsonValueKind.Object)
            {
                return null;
            }
            var read = Result(root, assumeOk: true);
            return read.Ok ? read : null;
        }
        catch (JsonException)
        {
            return null;
        }
    }

    private static RichResult Result(JsonElement result, bool assumeOk = false)
    {
        var ok = assumeOk || (result.TryGetProperty("ok", out var said) && said.ValueKind == JsonValueKind.True);
        if (!ok)
        {
            return RichResult.Failed(
                result.TryGetProperty("error", out var error) && error.ValueKind == JsonValueKind.String ? error.GetString() ?? "error" : "error");
        }
        var width = Number(result, "width");
        var height = Number(result, "height");
        if (!(width > 0 && height > 0))
        {
            return RichResult.Failed("empty");
        }
        var baseline = Number(result, "baseline");
        baseline = double.IsNaN(baseline) ? height : Math.Clamp(baseline, 0, height);
        return new RichResult(true, width, height, baseline);
    }

    /// <summary>A finite number, or NaN for anything else.</summary>
    private static double Number(JsonElement owner, string name) =>
        owner.TryGetProperty(name, out var value) && value.ValueKind == JsonValueKind.Number && value.TryGetDouble(out var number)
            && double.IsFinite(number)
            ? number
            : double.NaN;
}

/// <summary>The page the hidden web view draws in, and the one place it may reach (docs/protocol.md, "How it is drawn").</summary>
public static class RichPage
{
    /// <summary>
    /// The virtual host the app's copy of <c>web/rich</c> is served from. <c>.invalid</c> can never resolve (RFC 2606), so a
    /// request that slipped past the host's filter would still reach nobody — as <c>call.familyconnect.invalid</c> does for calls.
    /// </summary>
    public const string Host = "rich.familyconnect.invalid";

    /// <summary>The page itself, and the only address the web view is ever allowed to show.</summary>
    public const string Url = "https://" + Host + "/rich.html";

    /// <summary>The layout viewport the page is pinned to, CSS pixels: wider than <c>fc-rich.js</c>'s 1200-pixel diagram cap, so nothing is wrapped by the window.</summary>
    public const int ViewportWidth = 1280;

    public const int ViewportHeight = 800;

    /// <summary>Fifteen seconds without an answer is a failure (the design doc).</summary>
    public static readonly TimeSpan Timeout = TimeSpan.FromSeconds(15);

    /// <summary>Whether a navigation is to the page — exactly it; a fragment or a query is another document's business.</summary>
    public static bool IsPage(string? uri) => string.Equals(uri, Url, StringComparison.OrdinalIgnoreCase);

    /// <summary>
    /// Whether the page may load <paramref name="uri"/>: a file of its own folder over https on <see cref="Host"/>, or a
    /// <c>data:</c> URL, which the page's content policy already allows for images and which never leaves the process. Everything
    /// else is refused — no network, ever.
    /// </summary>
    public static bool Allows(string? uri) =>
        Uri.TryCreate(uri, UriKind.Absolute, out var parsed)
        && (parsed.Scheme == "data"
            || (parsed.Scheme == Uri.UriSchemeHttps && string.Equals(parsed.Host, Host, StringComparison.OrdinalIgnoreCase) && parsed.IsDefaultPort));

    /// <summary>
    /// <c>fcRich.version</c> read out of <c>fc-rich.js</c> itself, so a drawing kept on disk can be found again before the web view
    /// — a browser process — has been started at all. Null when the file does not say, and nothing is then drawn.
    /// </summary>
    public static string? VersionOf(string script) =>
        Regex.Match(script, @"\bvar\s+VERSION\s*=\s*'([^'\r\n]+)'") is { Success: true } found ? found.Groups[1].Value : null;
}

/// <summary>Turning what the page drew into an image of it, through the DevTools protocol (md.win's <c>CaptureRegionPngAsync</c>).</summary>
public static class RichCapture
{
    /// <summary>The longest side, in pixels, a drawing is photographed at (the design doc): past it the scale gives way.</summary>
    public const int LongestSide = 4096;

    /// <summary>
    /// The scale to photograph a <paramref name="width"/> × <paramref name="height"/> CSS-pixel drawing at: the screen's own, so it
    /// is crisp at the size it is shown, unless that would put the longest side past <paramref name="longest"/> pixels. A screen
    /// that cannot be asked counts as 1.
    /// </summary>
    public static double ScaleFor(double screen, double width, double height, int longest = LongestSide)
    {
        var scale = double.IsFinite(screen) && screen > 0 ? screen : 1;
        var side = Math.Max(width, height);
        return side * scale > longest && side > 0 ? longest / side : scale;
    }

    /// <summary>
    /// <c>Page.captureScreenshot</c>'s arguments: the drawing's rectangle at the page's top left, as PNG (which keeps the alpha),
    /// at <paramref name="scale"/> — which re-renders rather than upscales — and <c>captureBeyondViewport</c>, so a drawing taller
    /// than the viewport is photographed whole.
    /// </summary>
    public static string Arguments(double width, double height, double scale) =>
        string.Create(CultureInfo.InvariantCulture,
            $"{{\"format\":\"png\",\"captureBeyondViewport\":true,\"clip\":{{\"x\":0,\"y\":0,\"width\":{Math.Max(width, 1):R},\"height\":{Math.Max(height, 1):R},\"scale\":{scale:R}}}}}");

    /// <summary>
    /// <c>Emulation.setDeviceMetricsOverride</c>: the viewport pinned in CSS pixels at a device scale of 1, so the window's own
    /// scale never reflows a drawing — the photograph's sharpness comes from the clip's scale instead.
    /// </summary>
    public static string Metrics() =>
        string.Create(CultureInfo.InvariantCulture,
            $"{{\"width\":{RichPage.ViewportWidth},\"height\":{RichPage.ViewportHeight},\"deviceScaleFactor\":1,\"mobile\":false}}");

    /// <summary>
    /// <c>Emulation.setDefaultBackgroundColorOverride</c> to clear: without it Chromium photographs a page with no background on
    /// white, and a formula would carry a white box onto a blue bubble (what Puppeteer's <c>omitBackground</c> does).
    /// </summary>
    public const string ClearBackground = "{\"color\":{\"r\":0,\"g\":0,\"b\":0,\"a\":0}}";

    /// <summary>The PNG in <c>Page.captureScreenshot</c>'s answer, or null when it holds none.</summary>
    public static byte[]? Png(string answer)
    {
        try
        {
            using var document = JsonDocument.Parse(answer);
            return document.RootElement.ValueKind == JsonValueKind.Object
                && document.RootElement.TryGetProperty("data", out var data) && data.ValueKind == JsonValueKind.String
                && data.GetString() is { Length: > 0 } base64
                ? Convert.FromBase64String(base64)
                : null;
        }
        catch (Exception e) when (e is JsonException or FormatException)
        {
            return null;
        }
    }
}

/// <summary>How a drawing sits in a line of text.</summary>
public static class RichFit
{
    /// <summary>
    /// An inline formula at its natural size — or, when it is wider than <paramref name="maxWidth"/>, scaled down whole, baseline
    /// with it — and how far to drop it so its baseline sits on the line's: a picture in a run of text stands ON the baseline, and
    /// the part of a formula below its own baseline (a fraction's denominator, a subscript) has to hang under it.
    /// </summary>
    public static (double Width, double Height, double Drop) Inline(double width, double height, double baseline, double maxWidth)
    {
        var scale = width > maxWidth && maxWidth > 0 ? maxWidth / width : 1;
        var shownHeight = height * scale;
        return (width * scale, shownHeight, Math.Max(0, shownHeight - Math.Clamp(baseline, 0, height) * scale));
    }
}

/// <summary>
/// The drawings a window keeps in memory, most recently used first — and the failures, which are kept only here, so a newer
/// engine gets another try at them (the design doc's cache). Bounded twice: by how many it holds and by their total weight — a
/// drawing's weight is its pixels, and one tall diagram photographed at 4096 pixels is worth a hundred formulas. Not thread-safe:
/// the window's thread owns it.
/// </summary>
public sealed class RichMemory<T>
{
    private readonly int capacity;
    private readonly long budget;
    private readonly LinkedList<(string Key, T Value, long Weight)> order = new();
    private readonly Dictionary<string, LinkedListNode<(string Key, T Value, long Weight)>> index = new(StringComparer.Ordinal);

    /// <param name="capacity">The most entries it holds.</param>
    /// <param name="budget">The most total weight it holds; the newest entry is always kept, whatever it weighs.</param>
    public RichMemory(int capacity, long budget = long.MaxValue)
    {
        ArgumentOutOfRangeException.ThrowIfLessThan(capacity, 1);
        ArgumentOutOfRangeException.ThrowIfLessThan(budget, 0);
        this.capacity = capacity;
        this.budget = budget;
    }

    public int Count => index.Count;

    /// <summary>The weight of everything held.</summary>
    public long Weight { get; private set; }

    public bool TryGet(string key, out T value)
    {
        if (index.TryGetValue(key, out var node))
        {
            order.Remove(node);
            order.AddFirst(node);
            value = node.Value.Value;
            return true;
        }
        value = default!;
        return false;
    }

    /// <summary>Keep <paramref name="value"/> under <paramref name="key"/>, letting the least recently used go once past either bound.</summary>
    public void Put(string key, T value, long weight = 1)
    {
        weight = Math.Max(0, weight);
        if (index.Remove(key, out var old))
        {
            order.Remove(old);
            Weight -= old.Value.Weight;
        }
        index[key] = order.AddFirst((key, value, weight));
        Weight += weight;
        while ((index.Count > capacity || Weight > budget) && order.Count > 1 && order.Last is { } last)
        {
            order.RemoveLast();
            index.Remove(last.Value.Key);
            Weight -= last.Value.Weight;
        }
    }

    public void Clear()
    {
        order.Clear();
        index.Clear();
        Weight = 0;
    }
}

/// <summary>
/// The drawings kept on disk, in the app's own cache folder: <c>&lt;key&gt;.png</c> and its metrics beside it in
/// <c>&lt;key&gt;.json</c>, written last, so a drawing is there only once both are. A file that cannot be read is a drawing that is
/// not kept — it is drawn again — and every failure here is swallowed: a cache never stops a bubble.
/// </summary>
public sealed class RichStore(string folder)
{
    /// <summary>How many drawings stay on disk; the ones read least recently go first (<see cref="Trim"/>).</summary>
    public const int Keep = 1000;

    /// <summary>The drawing kept under <paramref name="key"/>, or null. Reading one marks it as used, which is what keeps it.</summary>
    public (byte[] Png, RichResult Metrics)? Read(string key)
    {
        if (!IsKey(key))
        {
            return null;
        }
        try
        {
            var metricsPath = Path.Combine(folder, key + ".json");
            var pngPath = Path.Combine(folder, key + ".png");
            if (!File.Exists(metricsPath) || !File.Exists(pngPath))
            {
                return null;
            }
            if (RichAnswer.ReadMetrics(File.ReadAllText(metricsPath)) is not { } metrics)
            {
                Forget(key);
                return null;
            }
            var png = File.ReadAllBytes(pngPath);
            if (png.Length == 0)
            {
                Forget(key);
                return null;
            }
            File.SetLastWriteTimeUtc(metricsPath, DateTime.UtcNow);
            return (png, metrics);
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            return null;
        }
    }

    /// <summary>Keep a drawing. Each file goes in under a temporary name and is moved into place, so a reader never sees half of one.</summary>
    public void Write(string key, byte[] png, RichResult metrics)
    {
        if (!IsKey(key) || !metrics.Ok || png.Length == 0)
        {
            return;
        }
        try
        {
            Directory.CreateDirectory(folder);
            Place(Path.Combine(folder, key + ".png"), path => File.WriteAllBytes(path, png));
            Place(Path.Combine(folder, key + ".json"), path => File.WriteAllText(path, RichAnswer.Metrics(metrics)));
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            Forget(key);
        }
    }

    /// <summary>Let all but the <paramref name="keep"/> most recently used drawings go, and anything a crash left half-written.</summary>
    public void Trim(int keep = Keep)
    {
        try
        {
            if (!Directory.Exists(folder))
            {
                return;
            }
            foreach (var stray in Directory.EnumerateFiles(folder, "*.part"))
            {
                File.Delete(stray);
            }
            var kept = Directory.EnumerateFiles(folder, "*.json")
                .Select(path => new FileInfo(path))
                .OrderByDescending(file => file.LastWriteTimeUtc)
                .ToList();
            foreach (var file in kept.Skip(Math.Max(0, keep)))
            {
                Forget(Path.GetFileNameWithoutExtension(file.Name));
            }
            // A PNG whose metrics never landed is not a drawing.
            foreach (var png in Directory.EnumerateFiles(folder, "*.png"))
            {
                if (!File.Exists(Path.ChangeExtension(png, ".json")))
                {
                    File.Delete(png);
                }
            }
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            // Trimmed next launch, then.
        }
    }

    /// <summary>A key is 64 lower-case hex digits (<see cref="RichJob.Key"/>) and nothing else can name a file here.</summary>
    private static bool IsKey(string key) => key.Length == 64 && key.All(c => c is >= '0' and <= '9' or >= 'a' and <= 'f');

    private static void Place(string path, Action<string> write)
    {
        var part = path + ".part";
        write(part);
        File.Move(part, path, overwrite: true);
    }

    private void Forget(string key)
    {
        try
        {
            File.Delete(Path.Combine(folder, key + ".json"));
            File.Delete(Path.Combine(folder, key + ".png"));
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            // Left for Trim.
        }
    }
}
