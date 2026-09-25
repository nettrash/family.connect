using FamilyConnect.Core;

namespace FamilyConnect.App.Logic;

/// <summary>What a bubble knows about its link's card.</summary>
public enum PreviewStatus
{
    Loading,
    Loaded,
    Unavailable,
}

/// <summary>
/// Fetches the card under a message's first web link (ios <c>LinkPreviewLoader</c>, android <c>LinkPreviewRepository</c>).
/// </summary>
/// <remarks>
/// <para>
/// PRIVACY, up front: the one place the app talks to a host the family does not own. Every device that shows a linked
/// message contacts that link — switchable off in Settings, which is why <see cref="State"/> asks first. Requests are
/// stripped down: https only, no cookies, no credentials, no referrer, and a read that stops at the end of the page's
/// <c>&lt;head&gt;</c> or 1 MB. A redirect to plain http is not followed.
/// </para>
/// <para>
/// One fetch per link however many bubbles draw it. Results — failures too, so a dead link is not retried on every redraw —
/// live in memory only: a preview is derived data. The oldest half goes when the table fills, never all of it, or every card
/// on screen would vanish at once. A hidden row asks exactly as a visible one does (docs/protocol.md, "A hidden row still
/// fetches, and draws none of it"), and nothing here knows who sent the link.
/// </para>
/// </remarks>
public sealed class LinkPreviews : IDisposable
{
    public const int MaxEntries = 200;

    public const string UserAgent = "FamilyConnect/1.0 (+link-preview; like WhatsApp)";

    public const string Accept = "text/html,application/xhtml+xml";

    /// <summary>Connecting may take this long, and a whole fetch — page or image — twice it.</summary>
    public static readonly TimeSpan ConnectTimeout = TimeSpan.FromSeconds(10);

    public static readonly TimeSpan FetchTimeout = TimeSpan.FromSeconds(20);

    private readonly HttpClient http;
    private readonly Func<bool> enabled;
    private readonly object gate = new();
    private readonly Dictionary<string, (PreviewStatus Status, LinkPreview? Preview)> states = new(StringComparer.Ordinal);
    private readonly HashSet<string> inFlight = new(StringComparer.Ordinal);
    private readonly Dictionary<string, byte[]> images = new(StringComparer.Ordinal);
    /// <summary>Settled links, oldest first, so eviction drops the oldest rather than everything.</summary>
    private readonly List<string> order = [];
    private int generation;

    /// <param name="enabled">The reader's Link Previews switch, asked every time.</param>
    /// <param name="handler">The network; a stripped-down <see cref="SocketsHttpHandler"/> when none is given.</param>
    public LinkPreviews(Func<bool> enabled, HttpMessageHandler? handler = null)
    {
        this.enabled = enabled;
        handler ??= new SocketsHttpHandler
        {
            UseCookies = false,
            Credentials = null,
            PreAuthenticate = false,
            AllowAutoRedirect = true,
            ConnectTimeout = ConnectTimeout,
        };
        http = new HttpClient(handler, disposeHandler: true) { Timeout = Timeout.InfiniteTimeSpan };
        http.DefaultRequestHeaders.TryAddWithoutValidation("Accept", Accept);
        // Bots get the metadata without the consent walls and personalisation a browser's User-Agent invites.
        http.DefaultRequestHeaders.TryAddWithoutValidation("User-Agent", UserAgent);
    }

    /// <summary>A fetch settled — a card may have appeared, and a bubble changed height. Raised off the UI thread.</summary>
    public event Action? Landed;

    /// <summary>Ticks every time a fetch lands.</summary>
    public int Generation => Volatile.Read(ref generation);

    /// <summary>
    /// What is known about <paramref name="url"/>, starting its one fetch the first time it is asked for — or null while the
    /// reader has previews switched off, when nothing is fetched at all.
    /// </summary>
    public (PreviewStatus Status, LinkPreview? Preview)? State(string url)
    {
        if (!enabled())
        {
            return null;
        }
        lock (gate)
        {
            if (states.TryGetValue(url, out var known))
            {
                return known;
            }
            // https only, as on the phones: fetching a cleartext address somebody else chose is the wrong default for the one
            // request this app makes off the family's own server.
            if (!Uri.TryCreate(url, UriKind.Absolute, out var uri) || uri.Scheme != Uri.UriSchemeHttps)
            {
                Settle(url, (PreviewStatus.Unavailable, null));
                return states[url];
            }
            if (inFlight.Add(url))
            {
                states[url] = (PreviewStatus.Loading, null);
                _ = Task.Run(() => LoadAsync(url, uri));
            }
            return (PreviewStatus.Loading, null);
        }
    }

    /// <summary>The card image's bytes once they arrived, kept under the preview's own url — where a redirect landed.</summary>
    public byte[]? Image(Uri previewUrl)
    {
        lock (gate)
        {
            return images.GetValueOrDefault(previewUrl.AbsoluteUri);
        }
    }

    private async Task LoadAsync(string url, Uri uri)
    {
        LinkPreview? preview = null;
        byte[]? image = null;
        try
        {
            preview = await FetchPreviewAsync(uri).ConfigureAwait(false);
            // The image BEFORE publishing, so a card appears in one step instead of growing its bubble twice. https only.
            if (preview?.ImageUrl is { Scheme: "https" } imageUrl)
            {
                image = await FetchImageAsync(imageUrl).ConfigureAwait(false);
            }
        }
        catch (Exception e) when (e is HttpRequestException or OperationCanceledException or IOException or InvalidOperationException)
        {
            // A dead or slow link is simply no card; the text keeps whatever the page gave before the image failed.
        }
        lock (gate)
        {
            inFlight.Remove(url);
            if (preview is not null && image is not null)
            {
                images[preview.Url.AbsoluteUri] = image;
            }
            Settle(url, preview is null ? (PreviewStatus.Unavailable, null) : (PreviewStatus.Loaded, preview));
            // Only a fetch that LANDED ticks: a link refused on sight is answered while its bubble is being drawn, and a tick
            // then would make the very next draw rebuild everything for nothing.
            Interlocked.Increment(ref generation);
        }
        Landed?.Invoke();
    }

    /// <summary>Record a settled link — making room first by dropping the oldest half when the table is full.</summary>
    private void Settle(string url, (PreviewStatus, LinkPreview?) state)
    {
        if (states.Count >= MaxEntries)
        {
            var dropping = order.Take(MaxEntries / 2).ToList();
            foreach (var old in dropping)
            {
                states.Remove(old);
                images.Remove(old);
            }
            order.RemoveRange(0, dropping.Count);
        }
        states[url] = state;
        order.Add(url);
    }

    private async Task<LinkPreview?> FetchPreviewAsync(Uri uri)
    {
        using var deadline = new CancellationTokenSource(FetchTimeout);
        using var request = new HttpRequestMessage(HttpMethod.Get, uri);
        using var response = await http.SendAsync(request, HttpCompletionOption.ResponseHeadersRead, deadline.Token).ConfigureAwait(false);
        if (!response.IsSuccessStatusCode)
        {
            return null;
        }
        // Only HTML can carry the tags; a PDF or a download would be bytes burned.
        var mime = response.Content.Headers.ContentType?.MediaType?.ToLowerInvariant() ?? string.Empty;
        if (mime.Length > 0 && !mime.Contains("html", StringComparison.Ordinal))
        {
            return null;
        }
        byte[] bytes;
        await using (var stream = await response.Content.ReadAsStreamAsync(deadline.Token).ConfigureAwait(false))
        {
            bytes = await PageReader.ReadAsync(stream, PageReader.MaxPageBytes, stoppingAtEndOfHead: true, deadline.Token).ConfigureAwait(false);
        }
        var html = PageReader.Decode(bytes, response.Content.Headers.ContentType?.CharSet);
        // Relative images and the host label resolve against where a redirect landed, not what was asked for.
        return LinkPreviewParser.Parse(html, response.RequestMessage?.RequestUri ?? uri);
    }

    private async Task<byte[]?> FetchImageAsync(Uri uri)
    {
        using var deadline = new CancellationTokenSource(FetchTimeout);
        using var request = new HttpRequestMessage(HttpMethod.Get, uri);
        using var response = await http.SendAsync(request, HttpCompletionOption.ResponseHeadersRead, deadline.Token).ConfigureAwait(false);
        if (!response.IsSuccessStatusCode)
        {
            return null;
        }
        await using var stream = await response.Content.ReadAsStreamAsync(deadline.Token).ConfigureAwait(false);
        var bytes = await PageReader.ReadAsync(stream, PageReader.MaxImageBytes, stoppingAtEndOfHead: false, deadline.Token).ConfigureAwait(false);
        return bytes.Length > 0 ? bytes : null;
    }

    public void Dispose() => http.Dispose();
}
