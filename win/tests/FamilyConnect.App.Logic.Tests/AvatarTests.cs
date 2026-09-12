using System.Net;
using System.Text;
using FamilyConnect.App.Logic;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// Profile pictures: what may be sent, and the version that decides what is drawn.
/// </summary>
public class AvatarTests
{
    private sealed class Blobs : IBlobStore
    {
        public Dictionary<string, byte[]> Held { get; } = [];

        public byte[]? Read(string key) => Held.TryGetValue(key, out var bytes) ? bytes : null;

        public void Write(string key, ReadOnlyMemory<byte> bytes) => Held[key] = bytes.ToArray();
    }

    private sealed class Faces(HttpStatusCode status = HttpStatusCode.OK, string body = "JPEG")
        : HttpMessageHandler
    {
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

    private static (AvatarCache Cache, Faces Handler, Blobs Blobs) Build(Faces? handler = null)
    {
        handler ??= new Faces();
        var blobs = new Blobs();
        return (
            new AvatarCache(
                new ApiClient(
                    new HttpClient(handler), ServerUrl.Normalise("chat.example.com")!,
                    new MemoryTokenStore("t0ken")),
                blobs),
            handler,
            blobs);
    }

    [Fact]
    public void APictureIsShrunkToFiveHundredAndTwelveAndNeverBlownUp()
    {
        Assert.True(AvatarRules.NeedsDownscale(4032, 3024));
        Assert.Equal((512, 384), AvatarRules.Fit(4032, 3024));
        Assert.Equal((384, 512), AvatarRules.Fit(3024, 4032));
        Assert.Equal((512, 512), AvatarRules.Fit(1024, 1024));

        // Already small enough: sent as it is, rather than blown up to look worse.
        Assert.False(AvatarRules.NeedsDownscale(200, 120));
        Assert.Equal((200, 120), AvatarRules.Fit(200, 120));

        // A picture the platform could not measure is not a picture to send.
        Assert.Equal((0, 0), AvatarRules.Fit(0, 0));
    }

    [Fact]
    public void OnlyTheTwoTypesTheServerTakesAreOffered()
    {
        Assert.True(AvatarRules.IsAllowed("image/jpeg"));
        Assert.True(AvatarRules.IsAllowed("IMAGE/PNG"));
        Assert.False(AvatarRules.IsAllowed("image/heic"));
        Assert.False(AvatarRules.IsAllowed("image/gif"));
        Assert.Equal("image/jpeg", AvatarRules.SendAs);
    }

    /// <summary>
    /// THE VERSION IS PART OF THE KEY. No frame carries a picture — only the number — so a cache
    /// keyed on the user alone shows a face the family replaced weeks ago.
    /// </summary>
    [Fact]
    public async Task APictureIsKeptPerVersionSoAChangeIsDrawn()
    {
        var (cache, handler, blobs) = Build();

        Assert.Equal("JPEG", Encoding.UTF8.GetString((await cache.BytesAsync(11, 3)).Bytes!));
        Assert.Single(handler.Asked);

        // The same version again asks nobody.
        await cache.BytesAsync(11, 3);
        Assert.Single(handler.Asked);

        // A new version is a new picture, and it is fetched.
        await cache.BytesAsync(11, 4);
        Assert.Equal(2, handler.Asked.Count);
        Assert.Equal(2, blobs.Held.Count);
        Assert.Equal("avatar-11-4", AvatarCache.KeyFor(11, 4));
    }

    [Fact]
    public async Task VersionZeroIsNoPictureAndAsksNobody()
    {
        var (cache, handler, _) = Build();

        var (bytes, error) = await cache.BytesAsync(11, 0);

        Assert.Null(bytes);
        Assert.Null(error);
        Assert.Empty(handler.Asked);
    }

    /// <summary>
    /// "NO PICTURE" IS AN ANSWER AND IT IS CACHED: a 404 is stable for that version, and asking
    /// again on every redraw would be a request per row per frame.
    /// </summary>
    [Fact]
    public async Task AMissingPictureIsRememberedRatherThanAskedForAgain()
    {
        var (cache, handler, _) = Build(new Faces(
            HttpStatusCode.NotFound,
            """{"error": {"code": "user_not_found", "message": "no"}}"""));

        var (bytes, error) = await cache.BytesAsync(11, 3);
        Assert.Null(bytes);
        // Not a failure to show: the caller draws initials.
        Assert.Null(error);

        var (again, alsoNoError) = await cache.BytesAsync(11, 3);
        Assert.Null(again);
        Assert.Null(alsoNoError);
        Assert.Single(handler.Asked);
    }

    [Fact]
    public async Task ATransientFailureIsNotRememberedAsNoPicture()
    {
        var (cache, handler, blobs) = Build(
            new Faces(HttpStatusCode.BadGateway, "<html>502</html>"));

        var (bytes, error) = await cache.BytesAsync(11, 3);

        Assert.Null(bytes);
        Assert.True(error!.Transient);
        Assert.Empty(blobs.Held);

        await cache.BytesAsync(11, 3);
        Assert.Equal(2, handler.Asked.Count);
    }
}
