using System.Reflection;
using FamilyConnect.Core.Board;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;
using Microsoft.Data.Sqlite;

namespace FamilyConnect.Core.Tests.Store;

/// <summary>
/// ONE CACHE, MANY THREADS. In the app the socket writes frames into the cache on its own thread,
/// the resync applies pages on the thread pool, and the window reads on the UI thread — all through
/// the one connection the cache owns. Every operation holds the cache for its whole length, so a
/// read can never land inside somebody else's transaction.
/// </summary>
public class CacheConcurrencyTests : IDisposable
{
    private const string Sent = "2026-09-13T10:00:00Z";

    private readonly string path =
        Path.Combine(Path.GetTempPath(), $"fc-concurrency-{Guid.NewGuid():N}.db");

    private readonly Database cache;
    private readonly ChatStore chats;
    private readonly BoardStore board;
    private readonly OutboxStore outbox;
    private readonly PackStore pack;
    private readonly TranscriptStore transcripts;

    public CacheConcurrencyTests()
    {
        cache = Database.Open(path);
        chats = new ChatStore(cache, () => 7);
        board = new BoardStore(cache);
        outbox = new OutboxStore(cache);
        pack = new PackStore(cache);
        transcripts = new TranscriptStore(cache);
        transcripts.Keep(new KeptTranscript(34, "See you at six"), DateTimeOffset.UnixEpoch);
        pack.Replace([PackItem(1, 5)], 5);
        chats.Replace([new ChatRowDto(new ChatDto(42, "family", "The Smiths"))]);
        chats.Replace([new MemberDto(7, "anna", "Anna", Role: "owner"), new MemberDto(11, "bob", "Bob")]);
        chats.Apply(new MessageDto(1, 42, 11, "c-1", "hello", Sent));
        board.Replace([new NoteDto(1, 7, "text", "Milk", BoardSeq: 5, ContentSeq: 5)], 5);
        outbox.Queue(new OutboxRow(
            "c-9", 42, "queued", PendingFiles: ["photo.jpg"], StagedFiles: ["photo.jpg"],
            QueuedAt: DateTimeOffset.UnixEpoch));
    }

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

    private static PackItemDto PackItem(long id, long seq) =>
        new(id, 7, new AttachmentDto(70 + id, "photo", "image/webp", 4096, 512, 512), Sent, seq);

    /// <summary>Every public operation of the stores and the cache, by name.</summary>
    private Dictionary<string, Action> Operations() => new()
    {
        ["chats.Replace(rows)"] = () => chats.Replace([new ChatRowDto(new ChatDto(42, "family", "The Smiths"))]),
        ["chats.Chats"] = () => chats.Chats(),
        ["chats.Chat"] = () => chats.Chat(42),
        ["chats.IsListed"] = () => chats.IsListed(42),
        ["chats.Unread"] = () => chats.Unread(),
        ["chats.MarkRead"] = () => chats.MarkRead(42, 1),
        ["chats.Advance"] = () => chats.Advance(42, reactionSeq: 3),
        ["chats.Apply(message)"] = () => chats.Apply(new MessageDto(2, 42, 11, "c-2", "again", Sent)),
        ["chats.Apply(page)"] = () => chats.Apply([new MessageDto(3, 42, 11, "c-3", "and again", Sent)]),
        ["chats.ApplyReactions"] = () => chats.ApplyReactions(42, 1, 4, [new ReactionDto(11, "+1")]),
        ["chats.ApplyPoll"] = () => chats.ApplyPoll(42, 1, new PollDto(5, false, [new PollOptionDto(1, "Pizza", [])])),
        ["chats.Message"] = () => chats.Message(1),
        ["chats.Messages"] = () => chats.Messages(42),
        ["chats.Thread"] = () => chats.Thread(1),
        ["chats.Refresh"] = () => chats.Refresh([]),
        ["chats.Polls"] = () => chats.Polls(42),
        ["chats.Newest"] = () => chats.Newest(42),
        ["chats.CatchUpCursor"] = () => chats.CatchUpCursor(42),
        ["chats.CatchUpCursors"] = () => chats.CatchUpCursors(),
        ["chats.FlagRepairCandidates"] = () => chats.FlagRepairCandidates(25),
        ["chats.SettleFlags"] = () => chats.SettleFlags(1),
        ["chats.RepairMedia"] = () => chats.RepairMedia(new MessageDto(1, 42, 11, "c-1", "", Sent)),
        ["chats.Replace(members)"] = () => chats.Replace([new MemberDto(7, "anna", "Anna", Role: "owner")]),
        ["chats.Members"] = () => chats.Members(),
        ["chats.Member"] = () => chats.Member(11),
        ["chats.Joined"] = () => chats.Joined(new UserDto(12, "carl", "Carl")),
        ["chats.Left"] = () => chats.Left(11),
        ["chats.Deleted"] = () => chats.Deleted(new MemberDto(11, "bob", "Bob", Deleted: true)),
        ["chats.SetOwner"] = () => chats.SetOwner(11),
        ["chats.ReplaceBlocked"] = () => chats.ReplaceBlocked([11]),
        ["chats.SetBlocked"] = () => chats.SetBlocked(11, true),
        ["chats.Blocked"] = () => chats.Blocked(),
        ["chats.IsBlocked"] = () => chats.IsBlocked(11),
        ["board.Notes"] = () => board.Notes(),
        ["board.Note"] = () => board.Note(1),
        ["board.Replace"] = () => board.Replace([new NoteDto(2, 7, "text", "Eggs", BoardSeq: 6, ContentSeq: 6)], 6),
        ["board.Apply(note)"] = () => board.Apply(new NoteDto(3, 7, "text", "Bread", BoardSeq: 7, ContentSeq: 7)),
        ["board.Apply(page)"] = () => board.Apply([new NoteDto(4, 7, "text", "Tea", BoardSeq: 8, ContentSeq: 8)]),
        ["board.Cursor"] = () => _ = board.Cursor,
        ["board.Marks"] = () => _ = board.Marks,
        ["board.Mark"] = () => board.Mark(new BoardMarks(1, 5)),
        ["board.MarkShown"] = () => board.MarkShown(),
        ["board.Unread"] = () => board.Unread(),
        ["pack.Items"] = () => pack.Items(),
        ["pack.Item"] = () => pack.Item(1),
        ["pack.Count"] = () => pack.Count(),
        ["pack.Replace"] = () => pack.Replace([PackItem(2, 6)], 6),
        ["pack.Apply(item)"] = () => pack.Apply(PackItem(3, 7)),
        ["pack.Apply(page)"] = () => pack.Apply([PackItem(4, 8)]),
        ["pack.Removed"] = () => pack.Removed(1),
        ["pack.Cursor"] = () => _ = pack.Cursor,
        ["pack.Reconnected"] = () => pack.Reconnected(),
        ["pack.Connection"] = () => _ = pack.Connection,
        ["pack.CaughtUp"] = () => pack.CaughtUp(0),
        ["pack.IsCaughtUp"] = () => _ = pack.IsCaughtUp,
        ["pack.Limits"] = () => _ = pack.Limits,
        ["pack.SetLimits"] = () => pack.SetLimits(new PackLimits(200, 524_288)),
        ["pack.Used"] = () => pack.Used(1, DateTimeOffset.UnixEpoch),
        ["pack.Recents"] = () => pack.Recents(),
        ["transcripts.Find"] = () => transcripts.Find(34),
        ["transcripts.Keep"] = () => transcripts.Keep(new KeptTranscript(35, ""), DateTimeOffset.UnixEpoch),
        ["outbox.Queue"] = () => outbox.Queue(new OutboxRow("c-10", 42, "more", QueuedAt: DateTimeOffset.UnixEpoch)),
        ["outbox.All"] = () => outbox.All(),
        ["outbox.ForChat"] = () => outbox.ForChat(42),
        ["outbox.Due"] = () => outbox.Due(DateTimeOffset.UtcNow),
        ["outbox.Find"] = () => outbox.Find("c-9"),
        ["outbox.Delivered"] = () => outbox.Delivered("c-9"),
        ["outbox.Failed"] = () => outbox.Failed("c-9", ApiError.Transport("down"), DateTimeOffset.UtcNow, new ReconnectBackoff()),
        ["outbox.Uploaded"] = () => outbox.Uploaded("c-9", 34, "photo.jpg"),
        ["outbox.Refuse"] = () => outbox.Refuse("c-9", ErrorCodes.Blocked),
        ["outbox.Reupload"] = () => outbox.Reupload("c-9"),
        ["outbox.Retry"] = () => outbox.Retry("c-9"),
        ["outbox.Discard"] = () => outbox.Discard("c-9"),
        ["cache.Tables"] = () => cache.Tables(),
        ["cache.Columns"] = () => cache.Columns("messages"),
        ["cache.WipeAll"] = () => cache.WipeAll(),
        ["cache.UserVersion"] = () => _ = cache.UserVersion,
        ["cache.Migrate"] = () => cache.Migrate(),
    };

    private static readonly string[] Names =
    [
        "chats.Replace(rows)", "chats.Chats", "chats.Chat", "chats.IsListed", "chats.Unread",
        "chats.MarkRead", "chats.Advance", "chats.Apply(message)", "chats.Apply(page)",
        "chats.ApplyReactions", "chats.ApplyPoll", "chats.Message", "chats.Messages", "chats.Thread", "chats.Refresh", "chats.Polls",
        "chats.Newest", "chats.CatchUpCursor", "chats.CatchUpCursors", "chats.FlagRepairCandidates", "chats.SettleFlags", "chats.RepairMedia", "chats.Replace(members)", "chats.Members", "chats.Member", "chats.Joined",
        "chats.Left", "chats.Deleted", "chats.SetOwner", "chats.ReplaceBlocked", "chats.SetBlocked",
        "chats.Blocked", "chats.IsBlocked",
        "board.Notes", "board.Note", "board.Replace", "board.Apply(note)", "board.Apply(page)",
        "board.Cursor", "board.Marks", "board.Mark", "board.MarkShown", "board.Unread",
        "pack.Items", "pack.Item", "pack.Count", "pack.Replace", "pack.Apply(item)", "pack.Apply(page)", "pack.Removed",
        "pack.Cursor", "pack.Reconnected", "pack.Connection", "pack.CaughtUp", "pack.IsCaughtUp", "pack.Limits", "pack.SetLimits",
        "pack.Used", "pack.Recents",
        "transcripts.Find", "transcripts.Keep",
        "outbox.Queue", "outbox.All", "outbox.ForChat", "outbox.Due", "outbox.Find",
        "outbox.Delivered", "outbox.Failed", "outbox.Uploaded", "outbox.Refuse", "outbox.Reupload",
        "outbox.Retry", "outbox.Discard",
        "cache.Tables", "cache.Columns", "cache.WipeAll", "cache.UserVersion", "cache.Migrate",
    ];

    public static TheoryData<string> Named => new(Names);

    /// <summary>
    /// EVERY OPERATION WAITS FOR THE ONE IN FLIGHT. The test holds the cache itself, starts the
    /// operation on another thread, and insists it has not finished while the cache is held — then
    /// lets go and insists it does. An operation that forgot to hold the cache runs straight through
    /// and fails here every time, rather than one run in three under load.
    /// </summary>
    [Theory]
    [MemberData(nameof(Named))]
    public async Task AnOperationWaitsForTheOneInFlight(string name)
    {
        var operation = Operations()[name];
        using var started = new ManualResetEventSlim();
        var finished = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        Task running = finished.Task;
        // Entered and left on THIS thread with no await in between: the cache's lock belongs to the
        // thread that took it.
        using (cache.Hold())
        {
            // A thread of its own, not the pool's: on a busy CI runner (windows-2025, 2026-10-07) a
            // Task.Run waited more than five seconds for a pool thread while other test classes held
            // theirs, and "never started" was the pool's answer, not the cache's.
            var worker = new Thread(() =>
            {
                started.Set();
                try
                {
                    operation();
                    finished.SetResult();
                }
                catch (Exception e)
                {
                    finished.SetException(e);
                }
            })
            {
                IsBackground = true,
                Name = $"cache-concurrency {name}",
            };
            worker.Start();
            Assert.True(started.Wait(TimeSpan.FromSeconds(5)), $"{name} never started");
            Thread.Sleep(150);
            Assert.False(running.IsCompleted, $"{name} used the cache while another operation held it");
        }
        await running.WaitAsync(TimeSpan.FromSeconds(5));
    }

    /// <summary>
    /// And the list above is ALL of them: a store operation added later without being named here
    /// would be an operation nobody checked holds the cache.
    /// </summary>
    [Fact]
    public void EveryStoreOperationIsNamed()
    {
        var named = Operations().Keys.Select(key => key.Split('(')[0]).ToHashSet();
        Assert.Equal(Names.OrderBy(name => name), Operations().Keys.OrderBy(name => name));
        foreach (var (prefix, type) in new[]
                 {
                     ("chats", typeof(ChatStore)), ("board", typeof(BoardStore)), ("outbox", typeof(OutboxStore)),
                     ("pack", typeof(PackStore)), ("transcripts", typeof(TranscriptStore)),
                 })
        {
            var members = type
                .GetMethods(BindingFlags.Public | BindingFlags.Instance | BindingFlags.DeclaredOnly)
                .Where(method => !method.IsSpecialName)
                .Select(method => method.Name)
                .Concat(type
                    .GetProperties(BindingFlags.Public | BindingFlags.Instance | BindingFlags.DeclaredOnly)
                    .Select(property => property.Name))
                // The reader is a function the store was given, not the cache.
                .Where(member => member != nameof(ChatStore.Reader))
                .Distinct();
            foreach (var member in members)
            {
                Assert.Contains($"{prefix}.{member}", named);
            }
        }
    }

    /// <summary>The same, the way the app does it: a writer and a reader, flat out, at once.</summary>
    [Fact]
    public async Task TheSocketWritesWhileTheWindowReads()
    {
        var until = DateTime.UtcNow.AddSeconds(2);
        long written = 0;

        var writer = Task.Run(() =>
        {
            var id = 100L;
            while (DateTime.UtcNow < until)
            {
                var page = new List<MessageDto>();
                for (var at = 0; at < 20; at++, id++)
                {
                    page.Add(new MessageDto(id, 42, 11, null, $"message {id}", Sent));
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
                _ = board.Unread();
                _ = outbox.Due(DateTimeOffset.UtcNow);
            }
        });

        await Task.WhenAll(writer, reader);

        Assert.Equal(Interlocked.Read(ref written), chats.Newest(42)?.Id);
    }
}
