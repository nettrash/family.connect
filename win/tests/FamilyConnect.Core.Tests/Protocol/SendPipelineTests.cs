using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.Core.Tests.Protocol;

/// <summary>
/// Everything between "somebody pressed send" and "the server has it" — and the case that matters
/// most, which is the one in between (docs/protocol.md, "Sending on an unreliable network").
/// </summary>
public class SendPipelineTests : IDisposable
{
    private static readonly DateTimeOffset Now = new(2026, 9, 12, 10, 0, 0, TimeSpan.Zero);

    private readonly Database database = Database.OpenInMemory();

    public void Dispose() => database.Dispose();

    /// <summary>A socket whose answers the test decides, frame by frame.</summary>
    private sealed class Fake : IFrameSender
    {
        public bool IsConnected { get; set; } = true;
        public bool WritesGoThrough { get; set; } = true;
        public List<string> Sent { get; } = [];

        /// <summary>What to answer a send with, if anything. Null = silence.</summary>
        public Func<string, ServerFrame?>? Answers { get; set; }

        public event Action<ServerFrame>? Frame;

        public Task<bool> TrySend(string frame, CancellationToken ct = default)
        {
            Sent.Add(frame);
            if (!WritesGoThrough)
            {
                return Task.FromResult(false);
            }
            if (Answers?.Invoke(frame) is { } answer)
            {
                // Answered on this turn, which is deterministic and is an ordering the wire can
                // produce: the waiter is registered before the frame goes, so an answer that
                // arrives before the write returns is handled by the same code as a later one.
                Frame?.Invoke(answer);
            }
            return Task.FromResult(true);
        }

        public void Raise(ServerFrame frame) => Frame?.Invoke(frame);
    }

    private static string IdOf(string frame) =>
        System.Text.Json.JsonDocument.Parse(frame).RootElement
            .GetProperty("client_msg_id").GetString()!;

    private static MessageDto Delivered(string clientMsgId, long id = 1338) =>
        new(id, 42, 7, clientMsgId, "Dinner at 7?", "2026-09-12T10:00:00Z");

    private sealed record Harness(
        SendPipeline Pipeline,
        Fake Socket,
        OutboxStore Outbox,
        ChatStore Chats,
        List<(OutboxRow Row, ApiError Error)> Refusals,
        List<OutboxRow> Posted)
    {
        public Func<OutboxRow, ApiResult<MessageResponse>>? Rest { get; set; }
    }

    private Harness Build(Func<OutboxRow, ApiResult<MessageResponse>>? rest = null)
    {
        var socket = new Fake();
        var outbox = new OutboxStore(database);
        var chats = new ChatStore(database);
        chats.Replace([new ChatRowDto(new ChatDto(42, "family", "The Smiths"))]);
        var refusals = new List<(OutboxRow, ApiError)>();
        var posted = new List<OutboxRow>();
        Harness? harness = null;
        var pipeline = new SendPipeline(
            socket, outbox, chats,
            post: (row, _) =>
            {
                posted.Add(row);
                var answer = harness?.Rest?.Invoke(row)
                    ?? ApiResult<MessageResponse>.Failure(ApiError.Transport("no server here"));
                return Task.FromResult(answer);
            },
            backoff: new ReconnectBackoff(random: ceiling => ceiling),
            now: () => Now,
            nextId: () => "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01",
            // The deadline, not slept: a silent socket answers "unanswered" at once, which is
            // the same decision ten seconds later and a suite that finishes.
            wait: (_, _) => Task.CompletedTask);
        harness = new Harness(pipeline, socket, outbox, chats, refusals, posted) { Rest = rest };
        pipeline.Refused += (row, error) => refusals.Add((row, error));
        return harness;
    }

    [Fact]
    public void ASendIsWrittenDownBeforeAnythingMoves()
    {
        var harness = Build();
        var row = harness.Pipeline.Enqueue(42, "Dinner at 7?");
        // The bubble is drawn from this, and the id is the dedup key the server will take.
        Assert.Equal("8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01", row.ClientMsgId);
        Assert.Equal(Now, row.QueuedAt);
        Assert.Single(harness.Outbox.All());
        // And nothing has been sent yet: the flush is a separate act, which is what lets it run
        // first in a resync and on every trigger.
        Assert.Empty(harness.Socket.Sent);
    }

    [Fact]
    public async Task AnAckedFrameIsDeliveredAndTheRowGoes()
    {
        var harness = Build();
        harness.Socket.Answers = frame => new ServerFrame.Ack(IdOf(frame), Delivered(IdOf(frame)));
        harness.Pipeline.Enqueue(42, "Dinner at 7?");

        Assert.Equal(1, await harness.Pipeline.FlushAsync());
        Assert.Empty(harness.Outbox.All());
        // The message went into the cache through the same path a page does.
        Assert.Equal("Dinner at 7?", harness.Chats.Message(1338)!.Body);
        // Over the SOCKET: no REST call was made.
        Assert.Single(harness.Socket.Sent);
        Assert.Empty(harness.Posted);
    }

    /// <summary>
    /// THE CASE THAT MATTERS. A frame with no answer inside the deadline says nothing about
    /// whether the message landed — so the client re-sends over REST with the SAME dedup key,
    /// which is what makes the repeat safe.
    /// </summary>
    [Fact]
    public async Task AnUnansweredFrameFallsBackToRestWithTheSameDedupKey()
    {
        var harness = Build();
        // Silence from the socket.
        harness.Socket.Answers = _ => null;
        harness.Rest = row => ApiResult<MessageResponse>.Success(
            new MessageResponse(Delivered(row.ClientMsgId, 1339)));
        var row = harness.Pipeline.Enqueue(42, "Dinner at 7?");

        Assert.Equal(1, await harness.Pipeline.FlushAsync());
        Assert.Single(harness.Socket.Sent);
        Assert.Equal(row.ClientMsgId, Assert.Single(harness.Posted).ClientMsgId);
        Assert.Empty(harness.Outbox.All());
        Assert.NotNull(harness.Chats.Message(1339));
    }

    [Fact]
    public async Task AWriteThatDidNotGoIsTheSameAsNoAnswer()
    {
        var harness = Build();
        harness.Socket.WritesGoThrough = false;
        harness.Rest = row => ApiResult<MessageResponse>.Success(
            new MessageResponse(Delivered(row.ClientMsgId)));
        harness.Pipeline.Enqueue(42, "Dinner at 7?");

        Assert.Equal(1, await harness.Pipeline.FlushAsync());
        // A socket whose connection is dead absorbs a write, so this is as much of an answer as
        // the deadline expiring: REST decides it.
        Assert.Single(harness.Posted);
    }

    [Fact]
    public async Task WithNoSocketAtAllItGoesStraightToRest()
    {
        var harness = Build();
        harness.Socket.IsConnected = false;
        harness.Rest = row => ApiResult<MessageResponse>.Success(
            new MessageResponse(Delivered(row.ClientMsgId)));
        harness.Pipeline.Enqueue(42, "Dinner at 7?");

        Assert.Equal(1, await harness.Pipeline.FlushAsync(SendRules.FlushTrigger.WindowActivated));
        Assert.Empty(harness.Socket.Sent);
        Assert.Single(harness.Posted);
    }

    /// <summary>
    /// An <c>error</c> frame carrying a TERMINAL code is a refusal: the message will never be
    /// accepted as it stands, so it is shown failed and REST is not tried.
    /// </summary>
    [Fact]
    public async Task ATerminalRefusalOverTheSocketIsFinal()
    {
        var harness = Build();
        harness.Socket.Answers = frame =>
            new ServerFrame.Error(ErrorCodes.MessageTooLong, "too long", IdOf(frame), null);
        harness.Pipeline.Enqueue(42, new string('a', 5000));

        Assert.Equal(0, await harness.Pipeline.FlushAsync());
        Assert.Empty(harness.Posted);
        var row = Assert.Single(harness.Outbox.All());
        Assert.True(row.Failed);
        Assert.Equal(ErrorCodes.MessageTooLong, row.FailedCode);
        // The user is told exactly once, and only about this.
        var (refused, error) = Assert.Single(harness.Refusals);
        Assert.Equal(row.ClientMsgId, refused.ClientMsgId);
        Assert.Equal(ErrorCodes.MessageTooLong, error.Code);
    }

    /// <summary>
    /// An <c>error</c> frame with a TRANSIENT code is treated exactly as no answer at all — so
    /// REST still gets its turn.
    /// </summary>
    [Fact]
    public async Task ATransientErrorFrameIsNotARefusal()
    {
        var harness = Build();
        harness.Socket.Answers = frame =>
            new ServerFrame.Error(ErrorCodes.Internal, "busy", IdOf(frame), null);
        harness.Rest = row => ApiResult<MessageResponse>.Success(
            new MessageResponse(Delivered(row.ClientMsgId)));
        harness.Pipeline.Enqueue(42, "Dinner at 7?");

        Assert.Equal(1, await harness.Pipeline.FlushAsync());
        Assert.Single(harness.Posted);
        Assert.Empty(harness.Refusals);
    }

    [Fact]
    public async Task AnUnknownOutcomeStaysQueuedAndIsNotShownAsFailed()
    {
        var harness = Build();
        harness.Socket.Answers = _ => null;
        harness.Rest = _ => ApiResult<MessageResponse>.Failure(
            new ApiError("", "gateway", 502));
        harness.Pipeline.Enqueue(42, "Dinner at 7?");

        Assert.Equal(0, await harness.Pipeline.FlushAsync());
        var row = Assert.Single(harness.Outbox.All());
        Assert.False(row.Failed);
        Assert.Equal(1, row.Attempts);
        Assert.Equal(Now.AddSeconds(1), row.NextAttemptAt);
        // NOTHING is shown to the user: unknown is not a refusal.
        Assert.Empty(harness.Refusals);
    }

    [Fact]
    public async Task ARowThatIsNotDueIsLeftAlone()
    {
        var harness = Build();
        harness.Socket.Answers = _ => null;
        harness.Rest = _ => ApiResult<MessageResponse>.Failure(ApiError.Transport("offline"));
        harness.Pipeline.Enqueue(42, "Dinner at 7?");
        await harness.Pipeline.FlushAsync();
        var sentOnce = harness.Socket.Sent.Count;

        // The clock has not moved, so the row's delay has not passed.
        Assert.Equal(0, await harness.Pipeline.FlushAsync(SendRules.FlushTrigger.SocketConnected));
        Assert.Equal(sentOnce, harness.Socket.Sent.Count);
    }

    /// <summary>
    /// A row that still owes uploads must NEVER be posted: a message claiming no attachments is a
    /// text message, and the server would take it happily — a delivered bubble with the pictures
    /// gone.
    /// </summary>
    [Fact]
    public async Task ARowThatOwesUploadsIsNotSentAtAll()
    {
        var harness = Build();
        harness.Socket.Answers = frame => new ServerFrame.Ack(IdOf(frame), Delivered(IdOf(frame)));
        harness.Pipeline.Enqueue(42, "", pendingFiles: ["/tmp/a.jpg"]);

        Assert.Equal(0, await harness.Pipeline.FlushAsync());
        Assert.Empty(harness.Socket.Sent);
        Assert.Empty(harness.Posted);
        // Still queued, and not failed: it is waiting for its bytes, not refused.
        Assert.False(Assert.Single(harness.Outbox.All()).Failed);
    }

    /// <summary>
    /// An ack that arrived after the deadline — or after a relaunch, with nobody waiting — is
    /// still a delivery.
    /// </summary>
    [Fact]
    public void ALateAckStillDeliversTheRow()
    {
        var harness = Build();
        var row = harness.Pipeline.Enqueue(42, "Dinner at 7?");
        harness.Socket.Raise(new ServerFrame.Ack(row.ClientMsgId, Delivered(row.ClientMsgId)));
        Assert.Empty(harness.Outbox.All());
        Assert.NotNull(harness.Chats.Message(1338));
    }

    /// <summary>
    /// And this device's own send arriving as an ORDINARY message — the ack lost on the way back
    /// — is a delivery too, not a second bubble.
    /// </summary>
    [Fact]
    public void OwnMessageArrivingWithoutItsAckIsStillADelivery()
    {
        var harness = Build();
        var row = harness.Pipeline.Enqueue(42, "Dinner at 7?");
        harness.Socket.Raise(new ServerFrame.Message(Delivered(row.ClientMsgId)));
        Assert.Empty(harness.Outbox.All());
        Assert.NotNull(harness.Chats.Message(1338));
    }

    [Fact]
    public async Task TheSameSendNeverRunsTwiceAtOnce()
    {
        var harness = Build();
        var gate = new TaskCompletionSource<bool>();
        var posts = 0;
        harness.Socket.IsConnected = false;
        var pipeline = new SendPipeline(
            harness.Socket, harness.Outbox, harness.Chats,
            post: async (row, _) =>
            {
                posts++;
                await gate.Task;
                return ApiResult<MessageResponse>.Success(
                    new MessageResponse(Delivered(row.ClientMsgId)));
            },
            now: () => Now,
            nextId: () => "one",
            wait: (_, _) => Task.CompletedTask);
        pipeline.Enqueue(42, "Dinner at 7?");

        var first = pipeline.FlushAsync();
        var second = pipeline.FlushAsync(SendRules.FlushTrigger.ConnectivityRestored);
        // The second flush finds the first still working and does nothing: two passes over one
        // row would each upload the remainder and post it twice.
        //
        // BOUNDED on purpose. Without the guard the second flush joins the first behind the
        // gate and this await never returns — a test that hangs for ever instead of failing,
        // which is worse than the bug it is watching for.
        Assert.Equal(0, await second.WaitAsync(TimeSpan.FromSeconds(5)));
        gate.SetResult(true);
        Assert.Equal(1, await first.WaitAsync(TimeSpan.FromSeconds(5)));
        Assert.Equal(1, posts);
    }
}
