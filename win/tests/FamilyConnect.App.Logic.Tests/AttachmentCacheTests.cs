using System.Net;
using System.Text;
using FamilyConnect.App.Logic;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// One attachment's bytes, and the preview rule three clients have each had to learn.
/// </summary>
public class AttachmentCacheTests
{
    private sealed class Blobs : IBlobStore
    {
        public Dictionary<string, byte[]> Held { get; } = [];
        public List<string> Reads { get; } = [];

        public byte[]? Read(string key)
        {
            Reads.Add(key);
            return Held.TryGetValue(key, out var bytes) ? bytes : null;
        }

        public void Write(string key, ReadOnlyMemory<byte> bytes) => Held[key] = bytes.ToArray();
    }

    /// <summary>Answers bytes for any attachment path, and says what it was asked.</summary>
    private sealed class Pictures : HttpMessageHandler
    {
        private readonly HttpStatusCode status;
        private readonly string body;

        public Pictures(string body = "JPEG", HttpStatusCode status = HttpStatusCode.OK)
        {
            this.body = body;
            this.status = status;
        }

        public List<string> Asked { get; } = [];

        protected override Task<HttpResponseMessage> SendAsync(
            HttpRequestMessage request, CancellationToken cancellationToken)
        {
            Asked.Add(request.RequestUri!.PathAndQuery);
            return Task.FromResult(new HttpResponseMessage(status)
            {
                Content = new StringContent(body, Encoding.UTF8, "image/jpeg"),
            });
        }
    }

    private static (AttachmentCache Cache, Pictures Handler, Blobs Blobs) Build(
        Pictures? handler = null)
    {
        handler ??= new Pictures();
        var blobs = new Blobs();
        return (
            new AttachmentCache(
                new ApiClient(
                    new HttpClient(handler), ServerUrl.Normalise("chat.example.com")!,
                    new MemoryTokenStore("t0ken")),
                blobs),
            handler,
            blobs);
    }

    private static AttachmentDto Photo(bool hasPreview) =>
        new(34, "photo", "image/jpeg", Width: 1600, Height: 1200, HasPreview: hasPreview);

    [Fact]
    public async Task BytesAreDownloadedOnceAndKept()
    {
        var (cache, handler, blobs) = Build();
        var photo = Photo(hasPreview: true);

        var (bytes, error) = await cache.BytesAsync(photo);

        Assert.Null(error);
        Assert.Equal("JPEG", Encoding.UTF8.GetString(bytes!));
        Assert.Equal("/api/v1/attachments/34", Assert.Single(handler.Asked));

        // The second look asks nobody.
        Assert.True(cache.Holds(photo));
        Assert.NotNull((await cache.BytesAsync(photo)).Bytes);
        Assert.Single(handler.Asked);
        Assert.Single(blobs.Held);
    }

    [Fact]
    public async Task ThePreviewAndTheOriginalAreDifferentThings()
    {
        var (cache, handler, blobs) = Build();
        var photo = Photo(hasPreview: true);

        await cache.BytesAsync(photo, preview: true);
        await cache.BytesAsync(photo);

        Assert.Equal(
            ["/api/v1/attachments/34/preview", "/api/v1/attachments/34"], handler.Asked);
        Assert.Equal(2, blobs.Held.Count);
    }

    /// <summary>
    /// A PREVIEW IS ONLY ASKED FOR WHEN THE ATTACHMENT SAYS IT HAS ONE. The server generates none
    /// for a picture the assistant drew; asking anyway answers 404, and a client that reads that
    /// as "no picture" draws an empty frame for ever — which is exactly how the Android board's
    /// backdrops failed.
    /// </summary>
    [Fact]
    public async Task APictureWithNoPreviewIsFetchedWholeRatherThanAskedForTwice()
    {
        var (cache, handler, _) = Build();
        var drawn = Photo(hasPreview: false);

        var (bytes, error) = await cache.BytesAsync(drawn, preview: true);

        Assert.Null(error);
        Assert.NotNull(bytes);
        // The ORIGINAL, and nothing else: no 404 on a preview that was never generated.
        Assert.Equal("/api/v1/attachments/34", Assert.Single(handler.Asked));
        Assert.True(cache.Holds(drawn, preview: true));
    }

    /// <summary>
    /// A FAILED DOWNLOAD IS NOT CACHED: a transient failure must not become a permanently blank
    /// picture.
    /// </summary>
    [Fact]
    public async Task AFailedDownloadIsNotRememberedAsAnAnswer()
    {
        var (cache, handler, blobs) = Build(
            new Pictures("<html>502</html>", HttpStatusCode.BadGateway));
        var photo = Photo(hasPreview: true);

        var (bytes, error) = await cache.BytesAsync(photo);

        Assert.Null(bytes);
        Assert.True(error!.Transient);
        Assert.Empty(blobs.Held);
        Assert.False(cache.Holds(photo));

        // And it asks again rather than drawing nothing for ever.
        await cache.BytesAsync(photo);
        Assert.Equal(2, handler.Asked.Count);
    }

    [Fact]
    public async Task LocalReadsTrackTheSelectedAlbumPageWithoutAdditionalRequests()
    {
        var (cache, handler, blobs) = Build();
        var first = Photo(hasPreview: true);
        var second = first with { Id = 59 };
        var album = new MediaAlbum([first, second], 0);
        await cache.BytesAsync(first, preview: true);
        await cache.BytesAsync(first);
        cache.Remember(second, new byte[] { 4, 5, 6 });
        blobs.Reads.Clear();

        Assert.Equal("JPEG", Encoding.UTF8.GetString(cache.ReadOriginal(album.PhotoForCopy!)!));
        Assert.True(album.Step(1));
        Assert.Equal(new byte[] { 4, 5, 6 }, cache.ReadOriginal(album.PhotoForCopy!));

        Assert.Equal(["34", "59"], blobs.Reads);
        Assert.Equal(["/api/v1/attachments/34/preview", "/api/v1/attachments/34"], handler.Asked);
    }

    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public void MissingOrEvictedOriginalNeverFallsBackToAPreviewOrDownloads(bool evicted)
    {
        var (cache, handler, blobs) = Build();
        var photo = Photo(hasPreview: true);
        if (evicted)
        {
            cache.Remember(photo, new byte[] { 1 });
            Assert.True(blobs.Held.Remove("34"));
        }
        cache.RememberPreview(photo, new byte[] { 2 });

        Assert.Null(cache.ReadOriginal(photo));

        Assert.Equal("34", Assert.Single(blobs.Reads));
        Assert.Empty(handler.Asked);
    }

    [Fact]
    public void TheKeysSayWhichCopyTheyAre()
    {
        Assert.Equal("34", AttachmentCache.KeyFor(34, preview: false));
        Assert.Equal("34.preview", AttachmentCache.KeyFor(34, preview: true));
    }
}
