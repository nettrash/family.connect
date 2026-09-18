using System.Collections.Concurrent;
using System.Net.WebSockets;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.Core.Tests.Protocol;

/// <summary>
/// The realtime loop: what it does with frames, with a dropped connection, and with a session
/// that expired under it (docs/protocol.md, "WebSocket protocol").
/// </summary>
public class ChatSocketTests
{
    /// <summary>
    /// A socket that answers a script. Each entry is one connection's worth: the frames it hands
    /// over, then how it ends.
    /// </summary>
    private sealed class Scripted : IWebSocket
    {
        private readonly Queue<string> frames;
        private readonly Exception? fails;
        private readonly int? closeCode;
        private readonly TaskCompletionSource<bool>? holdOpen;

        public Scripted(
            IEnumerable<string>? frames = null,
            Exception? fails = null,
            int? closeCode = null,
            TaskCompletionSource<bool>? holdOpen = null)
        {
            this.frames = new Queue<string>(frames ?? []);
            this.fails = fails;
            this.closeCode = closeCode;
            this.holdOpen = holdOpen;
        }

        public Uri? Opened { get; private set; }
        public string? Bearer { get; private set; }
        public List<string> Sent { get; } = [];
        public int? CloseCode => closeCode;
        public bool Disposed { get; private set; }

        public Task ConnectAsync(Uri url, string? bearer, CancellationToken ct)
        {
            Opened = url;
            Bearer = bearer;
            return fails is null ? Task.CompletedTask : Task.FromException(fails);
        }

        public Task SendAsync(string text, CancellationToken ct)
        {
            Sent.Add(text);
            return Task.CompletedTask;
        }

        public async Task<string?> ReceiveAsync(CancellationToken ct)
        {
            if (frames.Count > 0)
            {
                // A real receive yields; a fake that returns synchronously invents an ordering
                // the transport cannot have.
                await Task.Yield();
                return frames.Dequeue();
            }
            if (holdOpen is not null)
            {
                await holdOpen.Task.WaitAsync(ct).ConfigureAwait(false);
            }
            return null;
        }

        public void Dispose() => Disposed = true;
    }

    private static readonly Uri Where = new("wss://chat.example.com/api/v1/ws");

    private const string OneMessage =
        """
        {"type": "message", "message": {"id": 1338, "chat_id": 42, "sender_id": 7,
         "client_msg_id": null, "body": "Dinner at 7?", "created_at": "…"}}
        """;

    private sealed class Loop
    {
        public List<TimeSpan> Waits { get; } = [];
        public List<ServerFrame> Frames { get; } = [];
        public int Connects { get; set; }
        public int Disconnects { get; set; }
        public int Expired { get; set; }
        public DateTimeOffset Clock { get; set; } = new(2026, 9, 12, 10, 0, 0, TimeSpan.Zero);
    }

    /// <summary>
    /// One loop over a script of connections. The waits are recorded rather than slept, and the
    /// clock is moved by <paramref name="lasted"/> for each connection that came up — which is
    /// what decides whether the ceiling resets.
    /// </summary>
    private static async Task<(Loop Log, List<Scripted> Sockets)> Run(
        IEnumerable<Func<Scripted>> connections,
        TimeSpan lasted = default,
        int stopAfter = int.MaxValue)
    {
        var log = new Loop();
        var sockets = new List<Scripted>();
        var queue = new Queue<Func<Scripted>>(connections);
        using var stop = new CancellationTokenSource();
        var socket = new ChatSocket(
            open: () =>
            {
                if (queue.Count == 0)
                {
                    stop.Cancel();
                    var idle = new Scripted();
                    sockets.Add(idle);
                    return idle;
                }
                var next = queue.Dequeue()();
                sockets.Add(next);
                return next;
            },
            url: () => Where,
            tokens: new MemoryTokenStore("t0ken"),
            backoff: new ReconnectBackoff(random: ceiling => ceiling),
            wait: (delay, _) =>
            {
                log.Waits.Add(delay);
                if (log.Waits.Count >= stopAfter)
                {
                    stop.Cancel();
                }
                return Task.CompletedTask;
            },
            now: () => log.Clock);
        socket.Frame += frame => log.Frames.Add(frame);
        socket.Connected += () =>
        {
            log.Connects++;
            // Each connection "lasts" this long before it drops.
            log.Clock += lasted;
        };
        socket.Disconnected += () => log.Disconnects++;
        socket.SessionExpired += () => log.Expired++;
        await socket.RunAsync(stop.Token);
        return (log, sockets);
    }

    [Fact]
    public async Task TheUpgradeCarriesTheTokenInAHeaderAndNowhereElse()
    {
        var (_, sockets) = await Run([() => new Scripted([OneMessage])], stopAfter: 1);
        var first = sockets[0];
        Assert.Equal(Where, first.Opened);
        Assert.Equal("t0ken", first.Bearer);
        Assert.DoesNotContain("t0ken", first.Opened!.ToString(), StringComparison.Ordinal);
        // And the socket is disposed when the connection ends, every time.
        Assert.True(first.Disposed);
    }

    [Fact]
    public async Task FramesArriveInOrderAndUnknownOnesAreSkipped()
    {
        var (log, _) = await Run(
            [() => new Scripted([
                OneMessage,
                """{"type": "fireworks", "colour": "green"}""",
                """{"type": "typing", "chat_id": 42, "user_id": 9}""",
                "not json at all",
                """{"type": "pong"}""",
            ])],
            stopAfter: 1);
        Assert.Collection(
            log.Frames,
            frame => Assert.IsType<ServerFrame.Message>(frame),
            frame => Assert.IsType<ServerFrame.Typing>(frame),
            frame => Assert.IsType<ServerFrame.Pong>(frame));
    }

    [Fact]
    public async Task ADroppedConnectionComesBackWithTheBackoffsOwnDelays()
    {
        var (log, sockets) = await Run(
            [
                () => new Scripted([OneMessage]),
                () => new Scripted([OneMessage]),
                () => new Scripted([OneMessage]),
            ],
            stopAfter: 3);
        // Three connections, three drops, and the ceilings climbing: none of them lasted.
        Assert.Equal(3, log.Connects);
        Assert.Equal(3, log.Disconnects);
        Assert.Equal([TimeSpan.FromSeconds(1), TimeSpan.FromSeconds(2), TimeSpan.FromSeconds(4)],
            log.Waits);
        Assert.Equal(3, log.Frames.Count);
        Assert.All(sockets.Take(3), socket => Assert.True(socket.Disposed));
    }

    /// <summary>
    /// A COMPLETED HANDSHAKE IS NOT ENOUGH. A proxy that accepts the upgrade and drops it at once
    /// used to reset the ceiling every cycle, so the socket reconnected about twice a second for
    /// ever and each cycle fired a full resync.
    /// </summary>
    [Fact]
    public async Task AConnectionThatLastedResetsTheCeilingAndOneThatDidNotDoesNot()
    {
        var lasting = await Run(
            [() => new Scripted([OneMessage]), () => new Scripted([OneMessage])],
            lasted: ChatSocket.DurableAfter,
            stopAfter: 2);
        // Both connections carried traffic for long enough, so both start again from the floor.
        Assert.Equal([TimeSpan.FromSeconds(1), TimeSpan.FromSeconds(1)], lasting.Log.Waits);

        var flapping = await Run(
            [() => new Scripted([OneMessage]), () => new Scripted([OneMessage])],
            lasted: TimeSpan.FromSeconds(1),
            stopAfter: 2);
        Assert.Equal([TimeSpan.FromSeconds(1), TimeSpan.FromSeconds(2)], flapping.Log.Waits);
    }

    [Fact]
    public async Task AnUpgradeThatNeverCompletedEarnsNothingAndIsNotAConnection()
    {
        var (log, _) = await Run(
            [
                () => new Scripted(fails: new WebSocketException("refused")),
                () => new Scripted(fails: new WebSocketException("refused")),
            ],
            stopAfter: 2);
        Assert.Equal(0, log.Connects);
        // Nothing came up, so nothing went down: a refused dial is not a disconnection to show.
        Assert.Equal(0, log.Disconnects);
        Assert.Equal([TimeSpan.FromSeconds(1), TimeSpan.FromSeconds(2)], log.Waits);
    }

    /// <summary>
    /// <c>4401</c> is not a network error: the session expired mid-connection, so the client
    /// signs out rather than retrying a token that will never work again.
    /// </summary>
    [Fact]
    public async Task ASessionThatExpiredMidConnectionSignsOutAndStopsTrying()
    {
        var (log, sockets) = await Run(
            [
                () => new Scripted([OneMessage], closeCode: ChatSocket.SessionExpiredCode),
                () => new Scripted([OneMessage]),
            ]);
        Assert.Equal(1, log.Expired);
        // One connection only: the loop returned instead of reconnecting.
        Assert.Single(sockets);
        Assert.Empty(log.Waits);
    }

    [Fact]
    public async Task AnOrdinaryCloseIsNotASignOut()
    {
        var (log, _) = await Run(
            [() => new Scripted([OneMessage], closeCode: (int)WebSocketCloseStatus.NormalClosure)],
            stopAfter: 1);
        Assert.Equal(0, log.Expired);
        Assert.Single(log.Waits);
    }

    [Fact]
    public async Task AFrameIsWrittenOnlyWhileThereIsAConnection()
    {
        var holdOpen = new TaskCompletionSource<bool>();
        var connected = new TaskCompletionSource<bool>();
        var scripted = new Scripted([OneMessage], holdOpen: holdOpen);
        using var stop = new CancellationTokenSource();
        var socket = new ChatSocket(
            open: () => scripted,
            url: () => Where,
            tokens: new MemoryTokenStore("t0ken"),
            backoff: new ReconnectBackoff(random: _ => 0),
            wait: (_, _) => Task.CompletedTask);
        socket.Connected += () => connected.TrySetResult(true);
        var loop = socket.RunAsync(stop.Token);

        await connected.Task.WaitAsync(TimeSpan.FromSeconds(5));
        Assert.True(socket.IsConnected);
        Assert.True(await socket.TrySend(ClientFrames.Typing(42)));
        Assert.Equal("""{"type":"typing","chat_id":42}""", Assert.Single(scripted.Sent));

        // And once it is gone, a write reports that it did not go rather than throwing: the
        // outbox decides what to do about it.
        holdOpen.TrySetResult(true);
        stop.Cancel();
        await loop;
        Assert.False(socket.IsConnected);
        Assert.False(await socket.TrySend(ClientFrames.Typing(42)));
    }

    [Fact]
    public async Task CancellingStopsTheLoopWithoutWaitingOrReconnecting()
    {
        var log = new Loop();
        using var stop = new CancellationTokenSource();
        var opens = 0;
        var socket = new ChatSocket(
            open: () =>
            {
                opens++;
                stop.Cancel();
                return new Scripted([OneMessage]);
            },
            url: () => Where,
            tokens: new MemoryTokenStore("t0ken"),
            wait: (delay, _) =>
            {
                log.Waits.Add(delay);
                return Task.CompletedTask;
            });
        await socket.RunAsync(stop.Token);
        Assert.Equal(1, opens);
        Assert.Empty(log.Waits);
    }
}
