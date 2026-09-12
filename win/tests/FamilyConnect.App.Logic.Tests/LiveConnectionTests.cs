using System.Net;
using FamilyConnect.App.Logic;
using FamilyConnect.Core.Protocol;
using FamilyConnect.Core.Store;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// The policy over the socket: when it runs, what a connection means, and what a connection
/// that keeps flapping is not allowed to cost.
/// </summary>
public class LiveConnectionTests : IDisposable
{
    /// <summary>Every wait here is bounded: a mutant that hangs costs the whole harness.</summary>
    private static readonly TimeSpan Patience = TimeSpan.FromSeconds(5);

    private readonly Database cache = Database.OpenInMemory();

    public void Dispose() => cache.Dispose();

    /// <summary>A socket that is told what to do rather than connecting to anything.</summary>
    private sealed class Wire : ILiveSocket
    {
        private readonly TaskCompletionSource running = new();

        public int Runs { get; private set; }

        /// <summary>The most <see cref="RunAsync"/> calls that were ever in flight at once.</summary>
        public int Overlap { get; private set; }

        public int Live { get; private set; }

        public bool Stopped { get; private set; }

        public bool IsConnected { get; set; }

        public List<string> Sent { get; } = [];

        /// <summary>Completes once <see cref="RunAsync"/> has actually been entered.</summary>
        public Task Running => running.Task;

        public event Action<ServerFrame>? Frame;

        public event Action? Connected;

        public event Action? Disconnected;

        public event Action? SessionExpired;

        public async Task RunAsync(CancellationToken ct)
        {
            Runs++;
            Live++;
            Overlap = Math.Max(Overlap, Live);
            running.TrySetResult();
            try
            {
                await Task.Delay(Timeout.Infinite, ct).ConfigureAwait(false);
            }
            catch (OperationCanceledException)
            {
                Stopped = true;
            }
            finally
            {
                Live--;
            }
        }

        public Task<bool> TrySend(string frame, CancellationToken ct = default)
        {
            Sent.Add(frame);
            return Task.FromResult(IsConnected);
        }

        public void Comes()
        {
            IsConnected = true;
            Connected?.Invoke();
        }

        public void Goes()
        {
            IsConnected = false;
            Disconnected?.Invoke();
        }

        public void Expires() => SessionExpired?.Invoke();

        public void Hear(ServerFrame frame) => Frame?.Invoke(frame);
    }

    private const string Token =
        """{"token": "t0ken", "user": {"id": 7, "username": "anna", "display_name": "Anna"}}""";

    private const string Me =
        """
        {"user": {"id": 7, "username": "anna", "display_name": "Anna"},
         "family": {"id": 3, "name": "The Smiths", "ai_history": true}, "role": "member",
         "blocked_user_ids": []}
        """;

    private const string Family =
        """
        {"family": {"id": 3, "name": "The Smiths", "ai_history": true},
         "members": [{"id": 7, "username": "anna", "display_name": "Anna", "role": "owner"}],
         "blocked_user_ids": []}
        """;

    private sealed record Rig(
        AppSession Session,
        LiveConnection Live,
        Wire Wire,
        ChatStore Chats,
        OutboxStore Outbox,
        SendPipeline Sending,
        Server Handler,
        MemoryTokenStore Tokens,
        List<int> Posts);

    private Rig Build(Server? server = null, Func<Task>? beforeRead = null)
    {
        server ??= new Server()
            .On("/auth/login", Token)
            .On("/me", Me)
            .On("/families/mine", Family)
            .On("/chats", """{"chats": []}""");
        var tokens = new MemoryTokenStore();
        var api = new ApiClient(
            new HttpClient(server), ServerUrl.Normalise("chat.example.com")!, tokens);
        var chats = new ChatStore(cache, () => 7);
        var board = new BoardStore(cache);
        var outbox = new OutboxStore(cache);
        var wire = new Wire();
        var posts = new List<int>();
        var sending = new SendPipeline(
            wire, outbox, chats,
            post: (row, _) =>
            {
                posts.Add(1);
                return Task.FromResult(ApiResult<MessageResponse>.Success(
                    new MessageResponse(new MessageDto(
                        1338, row.ChatId, 7, row.ClientMsgId, row.Body,
                        "2026-09-12T10:00:00Z"))));
            },
            wait: (_, _) => Task.CompletedTask);
        var session = new AppSession(api, tokens, cache);
        var resync = new Resync(api, chats, board, sending);
        var router = new FrameRouter(chats, board);
        var live = new LiveConnection(session, wire, resync, sending, router);
        return new Rig(session, live, wire, chats, outbox, sending, server, tokens, posts);
    }

    private static async Task<Resync.Report> NextPass(LiveConnection live, Action act)
    {
        var pass = new TaskCompletionSource<Resync.Report>();
        void Heard(Resync.Report report) => pass.TrySetResult(report);
        live.Resynced += Heard;
        try
        {
            act();
            return await pass.Task.WaitAsync(Patience);
        }
        finally
        {
            live.Resynced -= Heard;
        }
    }

    /// <summary>
    /// THE SOCKET RUNS ONLY WHILE THERE IS SOMETHING TO LISTEN TO: an account at the family gate
    /// has no chats, and a loop reconnecting on its behalf is a client hammering a door with no
    /// room behind it.
    /// </summary>
    [Fact]
    public async Task NothingListensUntilThereIsAFamilyToListenTo()
    {
        var server = new Server()
            .On("/auth/login", Token)
            .Then("/me",
                (HttpStatusCode.OK,
                 """{"user": {"id": 7, "username": "anna", "display_name": "Anna"}, "blocked_user_ids": []}"""),
                (HttpStatusCode.OK, Me))
            .On("/families/mine", Family)
            .On("/chats", """{"chats": []}""");
        var rig = Build(server);

        await rig.Session.SignInAsync("anna", "hunter2");
        Assert.Equal(Gate.NoFamily, rig.Session.State.Gate);
        rig.Live.Start();
        Assert.Equal(0, rig.Wire.Runs);
        Assert.Equal(Link.Down, rig.Live.Link);

        // The join is approved: the gate moves, and the socket follows it without being asked.
        await rig.Session.RefreshAsync();
        await rig.Wire.Running.WaitAsync(Patience);
        Assert.Equal(1, rig.Wire.Runs);
        Assert.Equal(Link.Connecting, rig.Live.Link);

        // And starting again is not starting a second one: the window may say this on every
        // appearance, and two loops on one socket would be two sockets and two resyncs. A loop
        // starts on the thread pool, so give a second one every chance to appear before
        // insisting there is none.
        rig.Live.Start();
        rig.Live.Start();
        await Task.Delay(TimeSpan.FromMilliseconds(200));
        Assert.Equal(1, rig.Wire.Runs);

        await rig.Live.DisposeAsync();
    }

    /// <summary>
    /// A pass that THREW is a pass that failed, and no more than that: the next connection starts
    /// another. A loop that stopped reading because one answer was nonsense would be a client
    /// that never recovers.
    /// </summary>
    [Fact]
    public async Task APassThatThrowsDoesNotStopTheNextOne()
    {
        var routed = 0;
        var rig = Build(new Server()
            .On("/auth/login", Token)
            .On("/me", Me)
            .On("/families/mine", Family)
            // The first pass asks for a chat list and gets an exception instead of an answer;
            // the second gets one.
            .OnAsync("/chats", () =>
            {
                if (Interlocked.Increment(ref routed) == 1)
                {
                    throw new InvalidOperationException("nothing there");
                }
                return Task.FromResult<(HttpStatusCode, string?)>(
                    (HttpStatusCode.OK, """{"chats": []}"""));
            }));
        await rig.Session.SignInAsync("anna", "hunter2");
        await rig.Wire.Running.WaitAsync(Patience);

        var reports = new List<Resync.Report>();
        rig.Live.Resynced += report => reports.Add(report);
        rig.Wire.Comes();
        rig.Wire.Goes();
        var second = await NextPass(rig.Live, rig.Wire.Comes);

        Assert.True(second.Complete);
        // The pass that threw reported nothing at all — it never reached the end of the method —
        // and the one after it did.
        Assert.Equal(2, routed);
        await rig.Live.DisposeAsync();
    }

    /// <summary>
    /// STARTING AND STOPPING ARE SERIALISED. A stop used to clear its fields and only then wait
    /// for the socket loop to wind down, so a start arriving in that window found nothing running
    /// and put a SECOND <c>RunAsync</c> on the same socket — one object with one live connection
    /// field, and the two would have fought over it. A gate that moves away and back (signed out,
    /// signed in; removed, re-approved) is exactly that window.
    /// </summary>
    [Fact]
    public async Task AGateThatMovesAwayAndBackNeverRunsTwoSocketsAtOnce()
    {
        var rig = Build();
        await rig.Session.SignInAsync("anna", "hunter2");
        await rig.Wire.Running.WaitAsync(Patience);

        for (var again = 0; again < 5; again++)
        {
            // Not awaited, on purpose: this is the window handler's own shape — the gate moved
            // on somebody else's thread and a window event may not block on a socket.
            var stopping = rig.Live.StopAsync();
            rig.Live.Start();
            await stopping.WaitAsync(Patience);
            await rig.Live.StartAsync();
        }
        await Task.Delay(TimeSpan.FromMilliseconds(200));

        Assert.Equal(1, rig.Wire.Overlap);
        Assert.Equal(Link.Connecting, rig.Live.Link);

        await rig.Live.DisposeAsync();
        Assert.Equal(0, rig.Wire.Live);
    }

    /// <summary>
    /// EVERY connection starts a pass, not just the first: what a client missed while the socket
    /// was down is exactly what the pass reads.
    /// </summary>
    [Fact]
    public async Task EveryConnectionStartsAPass()
    {
        var rig = Build();
        await rig.Session.SignInAsync("anna", "hunter2");
        await rig.Wire.Running.WaitAsync(Patience);

        var first = await NextPass(rig.Live, rig.Wire.Comes);
        Assert.True(first.Complete);
        Assert.True(first.Signed);
        Assert.Equal(Link.Up, rig.Live.Link);

        rig.Wire.Goes();
        Assert.Equal(Link.Connecting, rig.Live.Link);

        var second = await NextPass(rig.Live, rig.Wire.Comes);
        Assert.True(second.Complete);

        await rig.Live.DisposeAsync();
        Assert.Equal(Link.Down, rig.Live.Link);
        Assert.True(rig.Wire.Stopped);
    }

    /// <summary>
    /// AND RESYNCS DO NOT STACK. A flapping proxy can raise three connections in a second; three
    /// overlapping passes would page the same chats three times and fight over the cursors. A
    /// pass in flight coalesces what arrives during it into ONE more pass.
    /// </summary>
    [Fact]
    public async Task ConnectionsThatArriveDuringAPassBecomeOneMorePass()
    {
        var held = new TaskCompletionSource();
        var reads = 0;
        // `/me` is the first read of a pass: the SECOND one (the first belongs to the sign-in)
        // is held open, so every connection that follows arrives while a pass is in flight.
        var rig = Build(new Server()
            .On("/auth/login", Token)
            .On("/families/mine", Family)
            .On("/chats", """{"chats": []}""")
            .OnAsync("/me", async () =>
            {
                if (Interlocked.Increment(ref reads) == 2)
                {
                    await held.Task.WaitAsync(Patience).ConfigureAwait(false);
                }
                return (HttpStatusCode.OK, Me);
            }));
        await rig.Session.SignInAsync("anna", "hunter2");
        await rig.Wire.Running.WaitAsync(Patience);

        var passes = 0;
        var second = new TaskCompletionSource();
        rig.Live.Resynced += _ =>
        {
            if (Interlocked.Increment(ref passes) == 2)
            {
                second.TrySetResult();
            }
        };

        // The first connection starts the pass that is now stuck on `/me`…
        rig.Wire.Comes();
        // …and three more arrive while it is stuck.
        rig.Wire.Goes();
        rig.Wire.Comes();
        rig.Wire.Goes();
        rig.Wire.Comes();
        rig.Wire.Goes();
        rig.Wire.Comes();
        held.SetResult();

        await second.Task.WaitAsync(Patience);
        // Give a third pass every chance to appear, and then insist there is none.
        await Task.Delay(TimeSpan.FromMilliseconds(200));
        Assert.Equal(2, passes);

        await rig.Live.DisposeAsync();
    }

    /// <summary>
    /// AN EXPIRED SESSION IS NOT A NETWORK ERROR: the socket's <c>4401</c> lands exactly where a
    /// REST <c>401</c> lands, and it lands once — otherwise a device whose account was deleted
    /// sits behind a "Connecting…" banner looking signed in.
    /// </summary>
    [Fact]
    public async Task TheSocketSayingTheSessionIsGoneEndsItOnceAndStopsListening()
    {
        var rig = Build();
        await rig.Session.SignInAsync("anna", "hunter2");
        await rig.Wire.Running.WaitAsync(Patience);
        var endings = new List<SessionEnd>();
        rig.Session.Ended += end => endings.Add(end);
        var stopped = new TaskCompletionSource();
        rig.Live.LinkChanged += link =>
        {
            if (link == Link.Down)
            {
                stopped.TrySetResult();
            }
        };

        rig.Wire.Expires();
        rig.Wire.Expires();

        await stopped.Task.WaitAsync(Patience);
        Assert.Equal([SessionEnd.Expired], endings);
        Assert.Null(rig.Tokens.Token);
        Assert.Equal(Gate.SignedOut, rig.Session.State.Gate);
        Assert.True(rig.Wire.Stopped);

        await rig.Live.DisposeAsync();
    }

    [Fact]
    public async Task FramesReachTheCacheThroughTheRouter()
    {
        var rig = Build();
        await rig.Session.SignInAsync("anna", "hunter2");
        rig.Chats.Replace([new ChatRowDto(new ChatDto(42, "family", "The Smiths"))]);

        rig.Wire.Hear(new ServerFrame.Message(new MessageDto(
            1338, 42, 9, null, "Dinner at 7?", "2026-09-12T10:00:00Z")));

        Assert.Equal("Dinner at 7?", rig.Chats.Message(1338)!.Body);

        await rig.Live.DisposeAsync();
        // And a disposed connection is not still listening to a socket somebody else holds.
        rig.Wire.Hear(new ServerFrame.Message(new MessageDto(
            1339, 42, 9, null, "…or 8?", "2026-09-12T10:01:00Z")));
        Assert.Null(rig.Chats.Message(1339));
    }

    /// <summary>
    /// THE OUTBOX HAS THREE TRIGGERS AND THE SOCKET IS ONLY ONE OF THEM: a returning network and
    /// a window coming forward flush it too, or a message sits unsent under a full signal bar.
    /// </summary>
    [Fact]
    public async Task AReturningNetworkFlushesTheOutboxWithoutWaitingForASocket()
    {
        var rig = Build();
        await rig.Session.SignInAsync("anna", "hunter2");
        rig.Chats.Replace([new ChatRowDto(new ChatDto(42, "family", "The Smiths"))]);
        rig.Sending.Enqueue(42, "Dinner at 7?");

        // No socket at all: the trigger is the network coming back.
        Assert.Equal(1, await rig.Live.FlushAsync(SendRules.FlushTrigger.ConnectivityRestored));
        Assert.Equal(1, rig.Posts.Count);
        // Posted, delivered, and the row is gone.
        Assert.Empty(rig.Outbox.All());

        await rig.Live.DisposeAsync();
    }

    [Fact]
    public async Task AFailedPassSaysSoAndTheNextConnectionTriesAgain()
    {
        var rig = Build(new Server()
            .On("/auth/login", Token)
            // TWICE, because a GET is tried once more after a transient failure — one 502 is a
            // read that succeeds on the retry, which is the point of the retry.
            .Then("/me",
                (HttpStatusCode.OK, Me),
                (HttpStatusCode.BadGateway, "<html>502</html>"),
                (HttpStatusCode.BadGateway, "<html>502</html>"),
                (HttpStatusCode.OK, Me))
            .On("/families/mine", Family)
            .On("/chats", """{"chats": []}"""));
        await rig.Session.SignInAsync("anna", "hunter2");
        await rig.Wire.Running.WaitAsync(Patience);
        var failures = new List<ApiError>();
        rig.Live.ResyncFailed += error => failures.Add(error);

        var failed = await NextPass(rig.Live, rig.Wire.Comes);
        Assert.False(failed.Complete);
        Assert.True(Assert.Single(failures).Transient);
        // The flush ran anyway: it is not a step.
        Assert.True(failed.Flushed);

        rig.Wire.Goes();
        var worked = await NextPass(rig.Live, rig.Wire.Comes);
        Assert.True(worked.Complete);

        await rig.Live.DisposeAsync();
    }

    [Fact]
    public async Task ARefreshOnDemandIsAPassAndTwoAtOnceAreStillOne()
    {
        var rig = Build();
        await rig.Session.SignInAsync("anna", "hunter2");
        await rig.Wire.Running.WaitAsync(Patience);

        var report = await NextPass(rig.Live, () =>
        {
            rig.Live.Refresh();
            rig.Live.Refresh();
            rig.Live.Refresh();
        });
        Assert.True(report.Complete);

        await rig.Live.DisposeAsync();
    }
}
