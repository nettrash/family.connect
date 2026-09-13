using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;
using Microsoft.Data.Sqlite;

namespace FamilyConnect.Core.Tests.Store;

/// <summary>
/// ONE CACHE, MANY THREADS. In the app the socket writes frames into the cache on its own thread,
/// the resync applies pages on the thread pool, and the window reads on the UI thread — all through
/// the one connection the cache owns. A connection is not safe to share without a rule about it.
/// </summary>
public class CacheConcurrencyTests : IDisposable
{
    private readonly string path =
        Path.Combine(Path.GetTempPath(), $"fc-concurrency-{Guid.NewGuid():N}.db");

    private readonly Database cache;

    public CacheConcurrencyTests() => cache = Database.Open(path);

    public void Dispose()
    {
        cache.Dispose();
        SqliteConnection.ClearAllPools();
        foreach (var file in new[] { path, path + "-wal", path + "-shm" })
        {
            try
            {
                File.Delete(file);
            }
            catch (IOException)
            {
            }
        }
    }

    [Fact]
    public async Task TheSocketWritesWhileTheWindowReads()
    {
        var chats = new ChatStore(cache, () => 7);
        chats.Replace([new ChatRowDto(new ChatDto(42, "family", "The Smiths"))]);
        var until = DateTime.UtcNow.AddSeconds(3);
        long written = 0;

        var writer = Task.Run(() =>
        {
            var id = 1L;
            while (DateTime.UtcNow < until)
            {
                var page = new List<MessageDto>();
                for (var at = 0; at < 20; at++, id++)
                {
                    page.Add(new MessageDto(id, 42, 11, null, $"message {id}", "2026-09-13T10:00:00Z"));
                }
                chats.Apply(page);
                Interlocked.Exchange(ref written, id - 1);
            }
        });
        var reader = Task.Run(() =>
        {
            while (DateTime.UtcNow < until)
            {
                _ = chats.Messages(42);
                _ = chats.Chats();
            }
        });

        await Task.WhenAll(writer, reader);

        Assert.Equal(Interlocked.Read(ref written), chats.Newest(42)?.Id);
    }
}
