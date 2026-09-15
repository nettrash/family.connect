using FamilyConnect.App.Logic;

namespace FamilyConnect.App.Services;

/// <summary>
/// OpenStreetMap's tiles for the maps under shared locations: fetched once, kept on disk, and asked for only while the reader
/// has map previews on.
/// </summary>
/// <remarks>
/// <para>
/// <b>PRIVACY, UP FRONT.</b> A tile is a square of map around a place a family member shared, so asking for one tells
/// OpenStreetMap roughly where that place is — the trade the Apple apps make with Apple for MapKit, and the reason for the
/// switch. Requests carry no cookies and no referrer, and nothing about a tile is ever written to the log.
/// </para>
/// <para>
/// <b>OPENSTREETMAP'S TILE POLICY, KEPT.</b> The app says who it is in its User-Agent, every tile is kept on disk and not
/// fetched again for a month, a tile that failed is not asked for again for five minutes, and every map drawn carries the
/// attribution (https://operations.osmfoundation.org/policies/tiles/).
/// </para>
/// </remarks>
internal sealed class MapTiles : IDisposable
{
    public const string UserAgent = "FamilyConnect-Windows/1.0 (+https://github.com/nettrash/family.connect)";

    private static readonly TimeSpan Keep = TimeSpan.FromDays(30);
    private static readonly TimeSpan RetryAfter = TimeSpan.FromMinutes(5);

    private readonly HttpClient http;
    private readonly object gate = new();
    private readonly Dictionary<string, Task<byte[]?>> loading = new(StringComparer.Ordinal);
    private readonly Dictionary<string, DateTime> failed = new(StringComparer.Ordinal);

    public MapTiles()
    {
        var handler = new SocketsHttpHandler
        {
            UseCookies = false,
            Credentials = null,
            AllowAutoRedirect = false,
            ConnectTimeout = TimeSpan.FromSeconds(10),
        };
        http = new HttpClient(handler, disposeHandler: true) { Timeout = TimeSpan.FromSeconds(20) };
        http.DefaultRequestHeaders.TryAddWithoutValidation("User-Agent", UserAgent);
    }

    private static string Folder => Path.Combine(AppFolders.Root, "maps");

    /// <summary>A tile's bytes, or null when there are none to be had right now. One fetch per tile however many ask.</summary>
    public Task<byte[]?> BytesAsync(MapTile tile)
    {
        var key = tile.Key;
        lock (gate)
        {
            if (loading.TryGetValue(key, out var running))
            {
                return running;
            }
            if (failed.TryGetValue(key, out var at) && DateTime.UtcNow - at < RetryAfter)
            {
                return Task.FromResult<byte[]?>(null);
            }
            var task = Task.Run(() => LoadAsync(key, tile));
            loading[key] = task;
            return task;
        }
    }

    private async Task<byte[]?> LoadAsync(string key, MapTile tile)
    {
        var path = Path.Combine(Folder, key + ".png");
        byte[]? bytes = null;
        try
        {
            if (File.Exists(path) && DateTime.UtcNow - File.GetLastWriteTimeUtc(path) < Keep)
            {
                bytes = await File.ReadAllBytesAsync(path).ConfigureAwait(false);
                return bytes;
            }
            using var response = await http.GetAsync(MapView.TileUrl(tile)).ConfigureAwait(false);
            if (response.IsSuccessStatusCode)
            {
                var body = await response.Content.ReadAsByteArrayAsync().ConfigureAwait(false);
                if (body.Length is > 0 and < 1_000_000)
                {
                    Directory.CreateDirectory(Folder);
                    var part = path + ".part";
                    await File.WriteAllBytesAsync(part, body).ConfigureAwait(false);
                    File.Move(part, path, overwrite: true);
                    bytes = body;
                    return bytes;
                }
            }
            else
            {
                Diagnostics.Write($"map tile refused: {(int)response.StatusCode}");
            }
        }
        catch (Exception e) when (e is HttpRequestException or TaskCanceledException or IOException or UnauthorizedAccessException)
        {
            Diagnostics.Write($"map tile: {e.GetType().Name}");
        }
        finally
        {
            lock (gate)
            {
                loading.Remove(key);
                if (bytes is null)
                {
                    failed[key] = DateTime.UtcNow;
                }
                else
                {
                    failed.Remove(key);
                }
            }
        }
        // Out of date but on disk beats nothing: a month-old street is still the street.
        try
        {
            if (File.Exists(path))
            {
                bytes = await File.ReadAllBytesAsync(path).ConfigureAwait(false);
            }
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            Diagnostics.Write($"map tile from disk: {e.GetType().Name}");
        }
        return bytes;
    }

    public void Dispose() => http.Dispose();
}
