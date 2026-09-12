using System.Net;
using FamilyConnect.App.Logic;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// The uploads a queued message owes, and the four ways they end.
/// </summary>
public class MediaOutboxTests : IDisposable
{
    private readonly Database cache = Database.OpenInMemory();
    private readonly OutboxStore outbox;

    public MediaOutboxTests()
    {
        outbox = new OutboxStore(cache);
        new ChatStore(cache, () => 7).Replace([
            new ChatRowDto(new ChatDto(42, "family", "The Smiths")),
        ]);
    }

    public void Dispose() => cache.Dispose();

    /// <summary>Staged bytes, and what a test says has gone missing.</summary>
    private sealed class Staging : IMediaStore
    {
        public Dictionary<string, StagedMedia> Files { get; } = [];

        public List<IReadOnlySet<string>> Sweeps { get; } = [];

        public StagedMedia? Read(string handle) =>
            Files.TryGetValue(handle, out var staged) ? staged : null;

        public void Sweep(IReadOnlySet<string> keep)
        {
            Sweeps.Add(keep);
            foreach (var handle in Files.Keys.Where(handle => !keep.Contains(handle)).ToList())
            {
                Files.Remove(handle);
            }
        }

        public Staging With(string handle, string kind = "photo", string mime = "image/jpeg")
        {
            Files[handle] = new StagedMedia(kind, mime, new byte[] { 1, 2, 3 }, 1600, 1200);
            return this;
        }
    }

    private static string Uploaded(long id) =>
        $$$"""{"attachment": {"id": {{{id}}}, "kind": "photo", "width": 1600, "height": 1200}}""";

    private (MediaOutbox Media, Server Handler) Build(Server server, IMediaStore staging) =>
        (new MediaOutbox(
            outbox,
            new ApiClient(
                new HttpClient(server), ServerUrl.Normalise("chat.example.com")!,
                new MemoryTokenStore("t0ken")),
            staging),
         server);

    private OutboxRow Queue(params string[] files)
    {
        var row = new OutboxRow(
            Guid.NewGuid().ToString(), 42, "look at this",
            PendingFiles: files, StagedFiles: files);
        outbox.Queue(row);
        return row;
    }

    [Fact]
    public async Task EveryOwedFileGoesAndEachLandingIsRememberedAsItHappens()
    {
        var row = Queue("a.jpg", "b.jpg");
        var (media, handler) = Build(
            new Server().Then("/attachments",
                (HttpStatusCode.OK, Uploaded(34)),
                (HttpStatusCode.OK, Uploaded(35))),
            new Staging().With("a.jpg").With("b.jpg"));

        Assert.Equal(2, await media.PushAsync());

        var held = outbox.Find(row.ClientMsgId)!;
        Assert.Equal([34L, 35L], held.AttachmentIds);
        Assert.False(held.OwesUploads);
        // The facts ride in the query, the bytes in the body.
        Assert.All(handler.Asked, path => Assert.Contains("kind=photo", path));
        Assert.All(handler.Asked, path => Assert.Contains("width=1600", path));
    }

    /// <summary>
    /// A crash halfway through a four-photo message must cost the remaining three, not all four:
    /// an id that landed is kept and reused within the server's grace.
    /// </summary>
    [Fact]
    public async Task AFileThatFailedTransientlyLeavesTheRestOwedAndTheLandedOnesKept()
    {
        var row = Queue("a.jpg", "b.jpg", "c.jpg");
        var (media, _) = Build(
            new Server().Then("/attachments",
                (HttpStatusCode.OK, Uploaded(34)),
                (HttpStatusCode.BadGateway, "<html>502</html>")),
            new Staging().With("a.jpg").With("b.jpg").With("c.jpg"));

        Assert.Equal(1, await media.PushAsync());

        var held = outbox.Find(row.ClientMsgId)!;
        Assert.Equal([34L], held.AttachmentIds);
        Assert.Equal(["b.jpg", "c.jpg"], held.PendingFiles);
        // Still queued, not failed: a transient upload failure is not a refusal.
        Assert.False(held.Failed);
    }

    /// <summary>
    /// A row whose bytes this device can no longer find is FAILED rather than retried: an id
    /// cannot recover a picture, and a backoff has nothing to wait for.
    /// </summary>
    [Fact]
    public async Task MediaThisDeviceCanNoLongerFindFailsTheRowOutright()
    {
        var row = Queue("gone.jpg");
        var refusals = new List<ApiError>();
        var (media, handler) = Build(new Server(), new Staging());
        media.Refused += (_, error) => refusals.Add(error);

        Assert.Equal(0, await media.PushAsync());

        Assert.Equal(ErrorCodes.MediaMissing, outbox.Find(row.ClientMsgId)!.FailedCode);
        Assert.Equal(ErrorCodes.MediaMissing, Assert.Single(refusals).Code);
        // Nothing was even attempted: there was nothing to send.
        Assert.Empty(handler.Asked);
    }

    /// <summary>
    /// A refusal about the FILE ITSELF — too large, not the kind it claimed — would refuse again
    /// with the same bytes, so the row fails and the person is told.
    /// </summary>
    [Fact]
    public async Task AFileTheServerRefusesFailsTheRowAndSaysWhy()
    {
        var row = Queue("huge.jpg");
        var refusals = new List<ApiError>();
        var (media, _) = Build(
            new Server().On(
                "/attachments",
                """{"error": {"code": "attachment_too_large", "message": "no"}}""",
                HttpStatusCode.RequestEntityTooLarge),
            new Staging().With("huge.jpg"));
        media.Refused += (_, error) => refusals.Add(error);

        Assert.Equal(0, await media.PushAsync());

        Assert.Equal(ErrorCodes.AttachmentTooLarge, outbox.Find(row.ClientMsgId)!.FailedCode);
        Assert.Equal(ErrorCodes.AttachmentTooLarge, Assert.Single(refusals).Code);
    }

    [Fact]
    public async Task ARowThatOwesNothingAndOneAlreadyFailedArePassedOver()
    {
        outbox.Queue(new OutboxRow(Guid.NewGuid().ToString(), 42, "just words"));
        var failed = Queue("a.jpg");
        outbox.Refuse(failed.ClientMsgId, ErrorCodes.AttachmentTooLarge);
        var (media, handler) = Build(new Server(), new Staging().With("a.jpg"));

        Assert.Equal(0, await media.PushAsync());

        Assert.Empty(handler.Asked);
    }

    /// <summary>
    /// The staging area cannot outlive the sends it is for: what no queued row names any more is
    /// forgotten, and what a row still owes is kept.
    /// </summary>
    [Fact]
    public void SweepingForgetsTheBytesOfSendsThatAreOver()
    {
        var row = Queue("a.jpg");
        var staging = new Staging().With("a.jpg").With("orphan.jpg");
        var (media, _) = Build(new Server(), staging);

        media.Sweep();

        Assert.True(staging.Files.ContainsKey("a.jpg"));
        Assert.False(staging.Files.ContainsKey("orphan.jpg"));

        // The send lands, the row goes, and its bytes go with it on the next sweep.
        outbox.Delivered(row.ClientMsgId);
        media.Sweep();
        Assert.Empty(staging.Files);
    }

    /// <summary>
    /// A SWEEP KEEPS WHAT IS STAGED, NOT WHAT IS OWED. A row two of whose three uploads have
    /// landed still needs those two files: the server can sweep an unclaimed upload, and
    /// `attachment_expired` means push it again.
    /// </summary>
    [Fact]
    public async Task SweepingKeepsTheFilesOfUploadsThatHaveAlreadyLanded()
    {
        Queue("a.jpg", "b.jpg");
        var staging = new Staging().With("a.jpg").With("b.jpg");
        var (media, _) = Build(
            new Server().Then("/attachments", (HttpStatusCode.OK, Uploaded(34))),
            staging);
        await media.PushAsync();

        media.Sweep();

        Assert.True(staging.Files.ContainsKey("a.jpg"));
        Assert.True(staging.Files.ContainsKey("b.jpg"));
    }

    /// <summary>
    /// A FLUSH PUSHES WHAT IS OWED BEFORE IT POSTS ANYTHING. A row that owes a byte is passed
    /// over by the pipeline, and nothing else is going to move those bytes — so a flush that
    /// skipped the uploads would pass over every media message for ever.
    /// </summary>
    [Fact]
    public async Task AFlushPushesTheUploadsBeforeItPostsTheMessage()
    {
        var row = Queue("a.jpg");
        var (media, _) = Build(
            new Server().On("/attachments", Uploaded(34)),
            new Staging().With("a.jpg"));
        var posted = new List<long[]?>();
        var pipeline = new SendPipeline(
            new NeverConnected(), outbox, new ChatStore(cache, () => 7),
            post: (sending, _) =>
            {
                posted.Add(sending.AttachmentIds);
                return Task.FromResult(ApiResult<MessageResponse>.Success(
                    new MessageResponse(new MessageDto(
                        1338, 42, 7, sending.ClientMsgId, sending.Body,
                        "2026-09-12T12:00:00Z"))));
            },
            wait: (_, _) => Task.CompletedTask,
            uploads: media.PushAsync);

        Assert.Equal(1, await pipeline.FlushAsync(SendRules.FlushTrigger.SocketConnected));

        // One flush: the bytes went, and then the message went WITH the id they became.
        Assert.Equal([34L], Assert.Single(posted));
        Assert.Empty(outbox.All());
    }

    /// <summary>
    /// `attachment_expired` means UPLOAD IT AGAIN. The dead ids go, every staged file is owed
    /// again, and the same `client_msg_id` is re-sent — which is what makes the repeat safe.
    /// </summary>
    [Fact]
    public async Task AnExpiredUploadIsPushedAgainRatherThanGivenUpOn()
    {
        var row = Queue("a.jpg");
        var staging = new Staging().With("a.jpg");
        var (media, _) = Build(
            new Server().Then("/attachments",
                (HttpStatusCode.OK, Uploaded(34)),
                (HttpStatusCode.OK, Uploaded(77))),
            staging);
        await media.PushAsync();
        Assert.Equal([34L], outbox.Find(row.ClientMsgId)!.AttachmentIds);

        // The send is refused because the server swept the upload before the message named it.
        var pipeline = new SendPipeline(
            new NeverConnected(), outbox, new ChatStore(cache, () => 7),
            post: (_, _) => Task.FromResult(ApiResult<MessageResponse>.Failure(
                new ApiError(ErrorCodes.AttachmentExpired, "gone", 409))),
            wait: (_, _) => Task.CompletedTask);
        await pipeline.FlushAsync(SendRules.FlushTrigger.UserRetried);

        var owing = outbox.Find(row.ClientMsgId)!;
        Assert.False(owing.Failed);
        Assert.Null(owing.AttachmentIds);
        Assert.Equal(["a.jpg"], owing.PendingFiles);

        // And the pump pushes it again, to a new id.
        Assert.Equal(1, await media.PushAsync());
        Assert.Equal([77L], outbox.Find(row.ClientMsgId)!.AttachmentIds);
    }

    /// <summary>
    /// …unless the bytes are gone, and then it is the one refusal with no way out.
    /// </summary>
    [Fact]
    public async Task AnExpiredUploadWhoseBytesAreGoneFailsInstead()
    {
        var row = new OutboxRow(
            Guid.NewGuid().ToString(), 42, "look at this", AttachmentIds: [34]);
        outbox.Queue(row);
        var pipeline = new SendPipeline(
            new NeverConnected(), outbox, new ChatStore(cache, () => 7),
            post: (_, _) => Task.FromResult(ApiResult<MessageResponse>.Failure(
                new ApiError(ErrorCodes.AttachmentExpired, "gone", 409))),
            wait: (_, _) => Task.CompletedTask);

        await pipeline.FlushAsync(SendRules.FlushTrigger.UserRetried);

        Assert.Equal(ErrorCodes.AttachmentExpired, outbox.Find(row.ClientMsgId)!.FailedCode);
    }

    private sealed class NeverConnected : IFrameSender
    {
        public bool IsConnected => false;

        public Task<bool> TrySend(string frame, CancellationToken ct = default) =>
            Task.FromResult(false);

        public event Action<ServerFrame>? Frame;

        public void Unused() => Frame?.Invoke(new ServerFrame.Pong());
    }
}
