using System.Diagnostics;
using System.Text;
using FamilyConnect.Core;

namespace FamilyConnect.Core.Tests.Text;

/// <summary>
/// How a page turns into the card under a link — ios <c>LinkPreviewParserTests</c> and <c>LinkPreviewYouTubeTests</c>, vector
/// for vector (Android mirrors the same ones): the Open Graph → Twitter → plain HTML ladder, the attribute forms real pages use,
/// entities, whitespace, clamping, relative images, no card without a title, and the head stop.
/// </summary>
public class LinkPreviewParserTests
{
    private static readonly Uri Page = new("https://example.com/articles/1");

    private static LinkPreview? Parse(string html) => LinkPreviewParser.Parse(html, Page);

    [Fact]
    public void OpenGraphTagsWin()
    {
        var preview = Parse("""
            <html><head>
            <title>Ignored</title>
            <meta property="og:title" content="The Real Title">
            <meta property="og:site_name" content="Example News">
            <meta property="og:description" content="A short summary.">
            <meta property="og:image" content="https://cdn.example.com/a.jpg">
            </head><body>…</body></html>
            """);
        Assert.Equal("The Real Title", preview?.Title);
        Assert.Equal("Example News", preview?.SiteName);
        Assert.Equal("A short summary.", preview?.Description);
        Assert.Equal("https://cdn.example.com/a.jpg", preview?.ImageUrl?.AbsoluteUri);
    }

    [Fact]
    public void TwitterCardsAreTheSecondChoicePlainHtmlTheThird()
    {
        var twitter = Parse("""
            <head><meta name="twitter:title" content="Twitter Title">
            <meta name="twitter:description" content="Twitter summary">
            <meta name="twitter:image" content="https://cdn.example.com/t.png"></head>
            """);
        Assert.Equal("Twitter Title", twitter?.Title);
        Assert.Equal("Twitter summary", twitter?.Description);
        Assert.Equal("https://cdn.example.com/t.png", twitter?.ImageUrl?.AbsoluteUri);

        var plain = Parse("""
            <head><title>Plain Title</title>
            <meta name="description" content="Plain summary"></head>
            """);
        Assert.Equal("Plain Title", plain?.Title);
        Assert.Equal("Plain summary", plain?.Description);
        Assert.Null(plain?.ImageUrl);
    }

    /// <summary>With both on the page, Open Graph wins every field — whichever comes first in the markup.</summary>
    [Fact]
    public void OpenGraphBeatsTwitterFieldForField()
    {
        var preview = Parse("""
            <head>
            <meta name="twitter:title" content="Twitter Title">
            <meta name="twitter:description" content="Twitter summary">
            <meta name="twitter:image" content="https://cdn.example.com/t.png">
            <meta name="description" content="Plain summary">
            <meta property="og:title" content="OG Title">
            <meta property="og:description" content="OG summary">
            <meta property="og:image" content="https://cdn.example.com/o.png">
            </head>
            """);
        Assert.Equal("OG Title", preview?.Title);
        Assert.Equal("OG summary", preview?.Description);
        Assert.Equal("https://cdn.example.com/o.png", preview?.ImageUrl?.AbsoluteUri);
        Assert.Equal("Twitter summary", Parse("""<head><title>T</title><meta name="description" content="Plain"><meta name="twitter:description" content="Twitter summary"></head>""")?.Description);
    }

    [Fact]
    public void NoTitleMeansNoCard()
    {
        Assert.Null(Parse("<head></head>"));
        Assert.Null(Parse("<head><title>   </title></head>"));
        Assert.Null(Parse(""));
    }

    [Fact]
    public void TheSiteNameFallsBackToTheHostWithoutWww()
    {
        Assert.Equal("example.com", LinkPreviewParser.Parse("<head><title>T</title></head>", new Uri("https://www.example.com/x"))?.SiteName);
        // Android's own vector: a host a strict parser refuses is still the label.
        Assert.Equal("my_host.example.com", LinkPreviewParser.DisplayHost(new Uri("https://my_host.example.com/x")));
    }

    [Fact]
    public void AttributeFormsRealPagesUse()
    {
        // Single quotes, reversed order, self-closing, extra attributes.
        var preview = Parse("""
            <head>
            <meta content='Reversed Order' property='og:title' />
            <meta charset="utf-8">
            <meta data-rh="true" property="og:description" content="Desc" />
            </head>
            """);
        Assert.Equal("Reversed Order", preview?.Title);
        Assert.Equal("Desc", preview?.Description);
    }

    [Fact]
    public void TheFirstOccurrenceOfAKeyWins() => Assert.Equal("First", Parse("""
        <head><meta property="og:title" content="First">
        <meta property="og:title" content="Second"></head>
        """)?.Title);

    [Fact]
    public void EntitiesAreDecodedAndWhitespaceCollapsed() => Assert.Equal("Tom & Jerry's big day …", Parse("""
        <head><meta property="og:title"
        content="Tom &amp; Jerry&#39;s
             big   day &hellip;"></head>
        """)?.Title);

    [Fact]
    public void OverLongTextIsClampedWithAnEllipsis()
    {
        var title = Parse($"<head><meta property=\"og:title\" content=\"{new string('a', 400)}\"></head>")?.Title ?? "";
        Assert.Equal(LinkPreviewParser.MaxTitleLength + 1, title.Length);
        Assert.EndsWith("…", title, StringComparison.Ordinal);
        var description = Parse($"<head><title>T</title><meta name=\"description\" content=\"{new string('d', 301)}\"></head>")?.Description ?? "";
        Assert.Equal(LinkPreviewParser.MaxDescriptionLength + 1, description.Length);
        Assert.Equal(new string('d', 300), Parse($"<head><title>T</title><meta name=\"description\" content=\"{new string('d', 300)}\"></head>")?.Description);
    }

    [Fact]
    public void RelativeAndProtocolRelativeImagesResolveAgainstThePage()
    {
        Assert.Equal("https://example.com/img/a.png",
            Parse("<head><title>T</title><meta property=\"og:image\" content=\"/img/a.png\"></head>")?.ImageUrl?.AbsoluteUri);
        Assert.Equal("https://cdn.example.com/b.png",
            Parse("<head><title>T</title><meta property=\"og:image\" content=\"//cdn.example.com/b.png\"></head>")?.ImageUrl?.AbsoluteUri);
    }

    [Fact]
    public void NonHttpImageSchemesAreRefused()
    {
        Assert.Null(Parse("<head><title>T</title><meta property=\"og:image\" content=\"file:///etc/passwd\"></head>")?.ImageUrl);
        Assert.Null(Parse("<head><title>T</title><meta property=\"og:image\" content=\"javascript:alert(1)\"></head>")?.ImageUrl);
    }

    [Fact]
    public void ATagWhoseNameMerelyStartsWithMetaIsNotAMetaTag() =>
        Assert.Equal("Real", Parse("<head><metadata property=\"og:title\" content=\"Nope\"><title>Real</title></head>")?.Title);

    /// <summary>İ lowercases to two scalars, ẞ to "ss", the Kelvin sign to "k": nothing may drift.</summary>
    [Fact]
    public void UnicodeThatLowercasesToADifferentLengthDoesNotCorruptOrCrash()
    {
        Assert.Equal("İstanbul Haber", Parse("<head><title>İstanbul Haber</title></head>")?.Title);
        Assert.Equal(new string('İ', 10), Parse($"<head><title>{new string('İ', 10)}</title></head>")?.Title);
        Assert.Equal("GROẞE STRAẞE", Parse("<head><title>GROẞE STRAẞE</title></head>")?.Title);
        Assert.Equal("Real", Parse("<head><p>KKK</p><title>Real</title></head>")?.Title);
        Assert.Equal("Found", Parse($"<head><p>{new string('İ', 8)}</p><meta property=\"og:title\" content=\"Found\"></head>")?.Title);
        Assert.Equal("Real", Parse($"<head><meta property=\"og:description\" content=\"{new string('İ', 16)}\"><title>Real</title></head>")?.Title);
    }

    [Fact]
    public void AStrayAmpersandPageStaysLinear()
    {
        var clock = Stopwatch.StartNew();
        _ = Parse("<head><title>" + new string('&', 30_000) + "x;</title></head>");
        Assert.True(clock.Elapsed < TimeSpan.FromSeconds(2));
    }

    [Fact]
    public void AQuotedAngleBracketDoesNotTruncateTheTag() =>
        Assert.Equal("A > B", Parse("<head><meta property=\"og:title\" content=\"A > B\"><title>Fallback</title></head>")?.Title);

    [Fact]
    public void ClampingCountsScalarsAndNeverSplitsAPair()
    {
        var title = Parse($"<head><meta property=\"og:title\" content=\"{string.Concat(Enumerable.Repeat("😀", 200))}\"></head>")?.Title ?? "";
        Assert.Equal(LinkPreviewParser.MaxTitleLength + 1, title.EnumerateRunes().Count());
        Assert.EndsWith("…", title, StringComparison.Ordinal);
        Assert.Equal(string.Concat(Enumerable.Repeat("😀", 140)), title[..^1]);
    }

    /// <summary>The entities the scanner knows, and the numeric ones that are not characters.</summary>
    [Fact]
    public void OnlyRealCharactersDecode()
    {
        Assert.Equal("’ “x” — – ‘ < > \" '   é 😀", LinkPreviewParser.DecodeEntities(
            "&#8217; &ldquo;x&rdquo; &mdash; &ndash; &lsquo; &LT; &gt; &quot; &apos; &nbsp; &#xE9; &#128512;"));
        Assert.Equal("&#xD800; &#1114112; &#; &#x; &bogus; &#-1;", LinkPreviewParser.DecodeEntities("&#xD800; &#1114112; &#; &#x; &bogus; &#-1;"));
        // The lookahead is ten characters, and that is a rule, not only a speed: an eleven-character entity stays as typed.
        Assert.Equal("A", LinkPreviewParser.DecodeEntities("&#000000065;"));
        Assert.Equal("&#0000000065;", LinkPreviewParser.DecodeEntities("&#0000000065;"));
    }

    // ---- the YouTube page (#50) ----------------------------------------------------------------------------------

    private static readonly Uri Watch = new("https://www.youtube.com/watch?v=dQw4w9WgXcQ");

    private const int TitleTagOffset = 704_923;
    private const int OgTitleOffset = 706_842;
    private const int HeadEndOffset = 715_108;

    /// <summary>The captured page with its elided player script put back at its exact byte count, so every tag sits where it did.</summary>
    private static string YouTubeWatchPage()
    {
        const string Marker = "<!--FAMILY-CONNECT-FIXTURE: ";
        var raw = File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "Fixtures", "youtube-watch.html"), Encoding.UTF8);
        var open = raw.IndexOf(Marker, StringComparison.Ordinal);
        var close = raw.IndexOf("-->", open, StringComparison.Ordinal);
        var count = int.Parse(string.Concat(raw[(open + Marker.Length)..].TakeWhile(char.IsAsciiDigit)), System.Globalization.CultureInfo.InvariantCulture);
        var script = "<script>var elided=\"" + new string('a', count - 31) + "\";</script>";
        Assert.Equal(count, Encoding.UTF8.GetByteCount(script));
        return raw[..open] + script + raw[(close + 3)..];
    }

    [Fact]
    public void TheCapturedPageHidesItsTagsPastAQuarterMegabyte()
    {
        var bytes = Encoding.UTF8.GetBytes(YouTubeWatchPage());
        Assert.Equal(TitleTagOffset, bytes.AsSpan().IndexOf("<title"u8));
        Assert.Equal(OgTitleOffset, bytes.AsSpan().IndexOf("og:title"u8));
        Assert.Equal(HeadEndOffset, bytes.AsSpan().IndexOf("</head"u8));
        Assert.True(OgTitleOffset > 256 * 1024);
    }

    [Fact]
    public void AWatchPageYieldsTheFullCard()
    {
        var preview = LinkPreviewParser.Parse(YouTubeWatchPage(), Watch);
        Assert.NotNull(preview);
        Assert.Equal("Rick Astley - Never Gonna Give You Up (Official Video) (4K Remaster)", preview.Title);
        Assert.Equal("YouTube", preview.SiteName);
        Assert.Equal("https://i.ytimg.com/vi/dQw4w9WgXcQ/maxresdefault.jpg", preview.ImageUrl?.AbsoluteUri);
        Assert.StartsWith("The official video for", preview.Description, StringComparison.Ordinal);
    }

    [Fact]
    public void TheOldScanLimitIsWhatMadeTheCardVanish()
    {
        var html = YouTubeWatchPage();
        Assert.Null(LinkPreviewParser.Parse(html[..200_000], Watch));
        Assert.NotNull(LinkPreviewParser.Parse(html[..Math.Min(html.Length, LinkPreviewParser.ScanLimit)], Watch));
        Assert.True(LinkPreviewParser.ScanLimit >= PageReader.MaxPageBytes);
    }

    private static int? StopIndex(byte[] bytes)
    {
        var detector = new HeadEndDetector();
        for (var index = 0; index < bytes.Length; index++)
        {
            if (detector.Consume(bytes[index]))
            {
                return index;
            }
        }
        return null;
    }

    [Fact]
    public void TheHeadStopLandsOnTheRealHeadEnd() =>
        Assert.Equal(HeadEndOffset + 6, StopIndex(Encoding.UTF8.GetBytes(YouTubeWatchPage())));

    [Fact]
    public void TheHeadStopIgnoresLookalikesFoldsCaseAndFallsBackToBody()
    {
        Assert.Null(StopIndex("<header><bodyguard>"u8.ToArray()));
        Assert.Null(StopIndex("<head><meta><bodyx"u8.ToArray()));
        Assert.Equal(27, StopIndex("<head><title>x</title><body class=\"a\">"u8.ToArray()));
        Assert.Equal(28, StopIndex("<HEAD><TITLE>x</TITLE></HEAD>"u8.ToArray()));
        Assert.Null(StopIndex("<html><meta name=\"a\" content=\"b\">"u8.ToArray()));
    }

    /// <summary>A stream that hands over a few bytes at a time, so a head end can straddle two reads.</summary>
    private sealed class Trickle(byte[] bytes, int chunk) : Stream
    {
        private int position;

        public override bool CanRead => true;
        public override bool CanSeek => false;
        public override bool CanWrite => false;
        public override long Length => bytes.Length;
        public override long Position { get => position; set => throw new NotSupportedException(); }
        public int Handed => position;

        public override int Read(byte[] buffer, int offset, int count)
        {
            var take = Math.Min(Math.Min(count, chunk), bytes.Length - position);
            Array.Copy(bytes, position, buffer, offset, take);
            position += take;
            return take;
        }

        public override void Flush() { }
        public override long Seek(long offset, SeekOrigin origin) => throw new NotSupportedException();
        public override void SetLength(long value) => throw new NotSupportedException();
        public override void Write(byte[] buffer, int offset, int count) => throw new NotSupportedException();
    }

    [Fact]
    public async Task TheReadStopsAtTheHeadAndTagsBelowItAreNotRead()
    {
        var page = Encoding.UTF8.GetBytes("<html><head><meta name=\"nothing\" content=\"x\"></head><body><meta property=\"og:title\" content=\"Too Late\"></body></html>");
        var stream = new Trickle(page, 5);
        var read = await Task.Run(() => PageReader.ReadAsync(stream, PageReader.MaxPageBytes, stoppingAtEndOfHead: true, CancellationToken.None));
        Assert.EndsWith("</head>", Encoding.UTF8.GetString(read), StringComparison.Ordinal);
        Assert.True(stream.Handed < page.Length, "the rest of the page was never taken");
        Assert.Null(LinkPreviewParser.Parse(Encoding.UTF8.GetString(read), Page));
    }

    [Fact]
    public async Task TheCeilingHoldsWhenAPageNeverClosesItsHead()
    {
        var page = Encoding.UTF8.GetBytes("<html><head><script>" + new string('a', 1_100_000) + "</script><meta property=\"og:title\" content=\"Past The Cap\">");
        var read = await Task.Run(() => PageReader.ReadAsync(new MemoryStream(page), PageReader.MaxPageBytes, stoppingAtEndOfHead: true, CancellationToken.None));
        Assert.Equal(PageReader.MaxPageBytes, read.Length);
        Assert.Null(LinkPreviewParser.Parse(Encoding.UTF8.GetString(read), Page));
    }

    [Fact]
    public async Task TheWatchPageReadInChunksStillBuildsTheCard()
    {
        var bytes = Encoding.UTF8.GetBytes(YouTubeWatchPage());
        var read = await Task.Run(() => PageReader.ReadAsync(new Trickle(bytes, 7_919), PageReader.MaxPageBytes, stoppingAtEndOfHead: true, CancellationToken.None));
        Assert.Equal(HeadEndOffset + 7, read.Length);
        Assert.Equal("YouTube", LinkPreviewParser.Parse(Encoding.UTF8.GetString(read), Watch)?.SiteName);
    }

    [Fact]
    public void APageDecodesInItsDeclaredCharsetOrUtf8()
    {
        Assert.Equal("Привет", PageReader.Decode([0xCF, 0xF0, 0xE8, 0xE2, 0xE5, 0xF2], "windows-1251"));
        Assert.Equal("Привет", PageReader.Decode(Encoding.UTF8.GetBytes("Привет"), "\"utf-8\""));
        Assert.Equal("Привет", PageReader.Decode(Encoding.UTF8.GetBytes("Привет"), "x-no-such-charset"));
        Assert.Equal("Привет", PageReader.Decode(Encoding.UTF8.GetBytes("Привет"), null));
    }
}
