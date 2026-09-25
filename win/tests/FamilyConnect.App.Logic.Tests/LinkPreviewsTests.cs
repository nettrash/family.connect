using System.Net;
using System.Text;
using FamilyConnect.App.Logic;
using FamilyConnect.Core;
using Xunit;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>The card fetch: https only, once per link, failures kept, nothing while switched off, and the table's oldest half evicted.</summary>
public sealed class LinkPreviewsTests
{
    /// <summary>Linked pages by absolute address, every request recorded.</summary>
    private sealed class Pages : HttpMessageHandler
    {
        private readonly Dictionary<string, Func<HttpRequestMessage, Task<HttpResponseMessage>>> answers = new(StringComparer.Ordinal);

        public List<HttpRequestMessage> Asked { get; } = [];

        public int Count(string url)
        {
            lock (Asked)
            {
                return Asked.Count(request => request.RequestUri!.AbsoluteUri == url);
            }
        }

        public Pages On(string url, Func<HttpRequestMessage, Task<HttpResponseMessage>> answer)
        {
            answers[new Uri(url).AbsoluteUri] = answer;
            return this;
        }

        public Pages Html(string url, string html, string contentType = "text/html; charset=utf-8") =>
            On(url, _ => Task.FromResult(Answer(HttpStatusCode.OK, Encoding.UTF8.GetBytes(html), contentType)));

        public static HttpResponseMessage Answer(HttpStatusCode status, byte[] body, string? contentType)
        {
            var content = new ByteArrayContent(body);
            if (contentType is not null)
            {
                content.Headers.TryAddWithoutValidation("Content-Type", contentType);
            }
            return new HttpResponseMessage(status) { Content = content };
        }

        protected override async Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken cancellationToken)
        {
            lock (Asked)
            {
                Asked.Add(request);
            }
            if (!answers.TryGetValue(request.RequestUri!.AbsoluteUri, out var answer))
            {
                throw new HttpRequestException($"no such page {request.RequestUri}");
            }
            var response = await answer(request).ConfigureAwait(false);
            response.RequestMessage ??= request;
            return response;
        }
    }

    private static LinkPreviews Loader(Pages pages, Func<bool>? enabled = null) => new(enabled ?? (() => true), pages);

    private static async Task<(PreviewStatus Status, LinkPreview? Preview)> SettledAsync(LinkPreviews previews, string url)
    {
        for (var attempt = 0; attempt < 500; attempt++)
        {
            if (previews.State(url) is { Status: not PreviewStatus.Loading } settled)
            {
                return settled;
            }
            await Task.Delay(10);
        }
        throw new TimeoutException(url);
    }

    private const string Card = """
        <html><head><title>Ignored</title>
        <meta property="og:title" content="The Real Title">
        <meta property="og:image" content="/img/a.png">
        </head><body><p>and megabytes more</p></body></html>
        """;

    [Fact]
    public Task APageBecomesACardWithItsImageInOneStep() => Task.Run(async () =>
    {
        var picture = new byte[] { 0x89, 0x50, 0x4E, 0x47, 1, 2, 3 };
        var pages = new Pages()
            .Html("https://example.com/a", Card)
            .On("https://example.com/img/a.png", _ => Task.FromResult(Pages.Answer(HttpStatusCode.OK, picture, "image/png")));
        using var previews = Loader(pages);
        var landed = 0;
        previews.Landed += () => Interlocked.Increment(ref landed);

        Assert.Equal((PreviewStatus.Loading, (LinkPreview?)null), previews.State("https://example.com/a"));
        var (status, preview) = await SettledAsync(previews, "https://example.com/a");

        Assert.Equal(PreviewStatus.Loaded, status);
        Assert.Equal("The Real Title", preview!.Title);
        Assert.Equal("example.com", preview.SiteName);
        Assert.Equal(picture, previews.Image(preview.Url));
        Assert.Equal(1, landed);
        Assert.Equal(1, previews.Generation);
    });

    [Fact]
    public Task RequestsSayWhoTheyAreAndCarryNothingElse() => Task.Run(async () =>
    {
        var pages = new Pages().Html("https://example.com/a", Card);
        using var previews = Loader(pages);
        _ = previews.State("https://example.com/a");
        _ = await SettledAsync(previews, "https://example.com/a");

        var request = pages.Asked[0];
        Assert.Equal(HttpMethod.Get, request.Method);
        Assert.Equal("FamilyConnect/1.0 (+link-preview; like WhatsApp)", string.Join(' ', request.Headers.GetValues("User-Agent")));
        Assert.Equal("text/html,application/xhtml+xml", string.Join(',', request.Headers.GetValues("Accept")));
        Assert.False(request.Headers.Contains("Cookie"));
        Assert.Null(request.Headers.Referrer);
        Assert.Null(request.Headers.Authorization);
    });

    /// <summary>A cleartext page is refused without a request, and so is a cleartext card image.</summary>
    [Fact]
    public Task OnlyHttpsIsEverFetched() => Task.Run(async () =>
    {
        var pages = new Pages()
            .Html("https://example.com/a", "<head><title>T</title><meta property=\"og:image\" content=\"http://cdn.example.com/a.png\"></head>")
            .Html("http://example.com/plain", Card);
        using var previews = Loader(pages);

        Assert.Equal(PreviewStatus.Unavailable, previews.State("http://example.com/plain")?.Status);
        Assert.Equal(PreviewStatus.Unavailable, previews.State("not a url")?.Status);
        // Refused on sight, while a bubble is drawn: nothing landed, so nothing ticks.
        Assert.Equal(0, previews.Generation);
        var (status, preview) = await SettledAsync(previews, "https://example.com/a");

        Assert.Equal(PreviewStatus.Loaded, status);
        Assert.Null(previews.Image(preview!.Url));
        Assert.Single(pages.Asked);
    });

    /// <summary>Every bubble drawing a link shares one fetch, and a settled answer — a failure included — is never fetched again.</summary>
    [Fact]
    public Task ALinkIsFetchedOnceAndAFailureIsKept() => Task.Run(async () =>
    {
        var release = new TaskCompletionSource();
        var pages = new Pages()
            .On("https://example.com/slow", async _ =>
            {
                await release.Task;
                return Pages.Answer(HttpStatusCode.OK, Encoding.UTF8.GetBytes(Card), "text/html");
            })
            .On("https://example.com/gone", _ => Task.FromResult(Pages.Answer(HttpStatusCode.NotFound, [], "text/html")));
        using var previews = Loader(pages);

        for (var bubble = 0; bubble < 3; bubble++)
        {
            Assert.Equal(PreviewStatus.Loading, previews.State("https://example.com/slow")?.Status);
        }
        release.SetResult();
        Assert.Equal(PreviewStatus.Loaded, (await SettledAsync(previews, "https://example.com/slow")).Status);
        _ = previews.State("https://example.com/slow");
        Assert.Equal(1, pages.Count("https://example.com/slow"));

        Assert.Equal(PreviewStatus.Unavailable, (await SettledAsync(previews, "https://example.com/gone")).Status);
        _ = previews.State("https://example.com/gone");
        await Task.Delay(50);
        Assert.Equal(1, pages.Count("https://example.com/gone"));
    });

    /// <summary>Not OK, not HTML, or not reachable: no card. A page that declares no type is still read.</summary>
    [Fact]
    public Task OnlyAnOkHtmlPageMakesACard() => Task.Run(async () =>
    {
        var html = Encoding.UTF8.GetBytes(Card);
        var pages = new Pages()
            .On("https://example.com/error", _ => Task.FromResult(Pages.Answer(HttpStatusCode.InternalServerError, html, "text/html")))
            .On("https://example.com/moved", _ => Task.FromResult(Pages.Answer(HttpStatusCode.Found, html, "text/html")))
            .On("https://example.com/file.pdf", _ => Task.FromResult(Pages.Answer(HttpStatusCode.OK, html, "application/pdf")))
            .On("https://example.com/untyped", _ => Task.FromResult(Pages.Answer(HttpStatusCode.OK, html, null)))
            .On("https://example.com/xhtml", _ => Task.FromResult(Pages.Answer(HttpStatusCode.OK, html, "application/xhtml+xml")))
            .On("https://example.com/throws", _ => throw new HttpRequestException("reset"));
        using var previews = Loader(pages);

        Assert.Equal(PreviewStatus.Unavailable, (await SettledAsync(previews, "https://example.com/error")).Status);
        Assert.Equal(PreviewStatus.Unavailable, (await SettledAsync(previews, "https://example.com/moved")).Status);
        Assert.Equal(PreviewStatus.Unavailable, (await SettledAsync(previews, "https://example.com/file.pdf")).Status);
        Assert.Equal(PreviewStatus.Unavailable, (await SettledAsync(previews, "https://example.com/unreachable")).Status);
        Assert.Equal(PreviewStatus.Unavailable, (await SettledAsync(previews, "https://example.com/throws")).Status);
        Assert.Equal(PreviewStatus.Loaded, (await SettledAsync(previews, "https://example.com/untyped")).Status);
        Assert.Equal(PreviewStatus.Loaded, (await SettledAsync(previews, "https://example.com/xhtml")).Status);
    });

    [Fact]
    public Task SwitchedOffNothingIsAskedForAtAll() => Task.Run(async () =>
    {
        var on = false;
        var pages = new Pages().Html("https://example.com/a", Card);
        using var previews = Loader(pages, () => on);

        Assert.Null(previews.State("https://example.com/a"));
        await Task.Delay(50);
        Assert.Empty(pages.Asked);

        on = true;
        Assert.Equal(PreviewStatus.Loaded, (await SettledAsync(previews, "https://example.com/a")).Status);
        on = false;
        Assert.Null(previews.State("https://example.com/a"));
    });

    /// <summary>The label and a relative image resolve against where a redirect landed, and the image is kept under that address.</summary>
    [Fact]
    public Task ARedirectedPageNamesWhereItLanded() => Task.Run(async () =>
    {
        var picture = new byte[] { 1, 2, 3 };
        var pages = new Pages()
            .On("https://short.example/x", _ =>
            {
                var landed = Pages.Answer(HttpStatusCode.OK, Encoding.UTF8.GetBytes(Card), "text/html");
                landed.RequestMessage = new HttpRequestMessage(HttpMethod.Get, "https://www.landed.example/final/page");
                return Task.FromResult(landed);
            })
            .On("https://www.landed.example/img/a.png", _ => Task.FromResult(Pages.Answer(HttpStatusCode.OK, picture, "image/png")));
        using var previews = Loader(pages);

        var (_, preview) = await SettledAsync(previews, "https://short.example/x");

        Assert.Equal("landed.example", preview!.SiteName);
        Assert.Equal("https://www.landed.example/final/page", preview.Url.AbsoluteUri);
        Assert.Equal(picture, previews.Image(preview.Url));
        Assert.Null(previews.Image(new Uri("https://short.example/x")));
    });

    [Fact]
    public Task AReadStopsAtTheHeadAndAPageSpeaksItsOwnCharset() => Task.Run(async () =>
    {
        // "Привет" in windows-1251.
        byte[] cyrillic = [.. "<head><title>"u8, 0xCF, 0xF0, 0xE8, 0xE2, 0xE5, 0xF2, .. "</title></head><body>"u8];
        var pages = new Pages()
            .Html("https://example.com/late", "<html><head><meta name=\"x\" content=\"y\"></head><body><meta property=\"og:title\" content=\"Too Late\"></body>")
            .On("https://example.com/ru", _ => Task.FromResult(Pages.Answer(HttpStatusCode.OK, cyrillic, "text/html; charset=windows-1251")));
        using var previews = Loader(pages);

        Assert.Equal(PreviewStatus.Unavailable, (await SettledAsync(previews, "https://example.com/late")).Status);
        Assert.Equal("Привет", (await SettledAsync(previews, "https://example.com/ru")).Preview?.Title);
    });

    /// <summary>A full table drops its oldest half, not everything: the newest cards stay, the oldest are fetched again when asked.</summary>
    [Fact]
    public Task AFullTableForgetsItsOldestHalf() => Task.Run(async () =>
    {
        var pages = new Pages();
        for (var index = 0; index <= LinkPreviews.MaxEntries; index++)
        {
            pages.Html($"https://example.com/{index}", $"<head><title>{index}</title></head>");
        }
        using var previews = Loader(pages);
        for (var index = 0; index <= LinkPreviews.MaxEntries; index++)
        {
            Assert.Equal(PreviewStatus.Loaded, (await SettledAsync(previews, $"https://example.com/{index}")).Status);
        }

        // The newest half is still known — answered at once, not fetched again.
        Assert.Equal(PreviewStatus.Loaded, previews.State($"https://example.com/{LinkPreviews.MaxEntries / 2}")?.Status);
        Assert.Equal(PreviewStatus.Loaded, previews.State($"https://example.com/{LinkPreviews.MaxEntries - 1}")?.Status);
        Assert.Equal(PreviewStatus.Loaded, previews.State($"https://example.com/{LinkPreviews.MaxEntries}")?.Status);
        Assert.Equal(1, pages.Count($"https://example.com/{LinkPreviews.MaxEntries / 2}"));
        Assert.Equal(1, pages.Count($"https://example.com/{LinkPreviews.MaxEntries - 1}"));
        // The oldest half was forgotten, and asking again fetches again.
        Assert.Equal(PreviewStatus.Loaded, (await SettledAsync(previews, "https://example.com/0")).Status);
        Assert.Equal(2, pages.Count("https://example.com/0"));
        Assert.Equal(PreviewStatus.Loaded, (await SettledAsync(previews, $"https://example.com/{LinkPreviews.MaxEntries / 2 - 1}")).Status);
        Assert.Equal(2, pages.Count($"https://example.com/{LinkPreviews.MaxEntries / 2 - 1}"));
    });
}
