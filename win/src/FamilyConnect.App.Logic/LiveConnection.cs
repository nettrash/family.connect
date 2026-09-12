using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic;

/// <summary>What the window draws in the corner, and nothing more subtle than this.</summary>
public enum Link
{
    /// <summary>No socket, and not trying: signed out, or at the family gate.</summary>
    Down,

    /// <summary>Trying. The loop is running and the connection is not up.</summary>
    Connecting,

    /// <summary>Up, and the pass that follows a connection has been started.</summary>
    Up,
}

/// <summary>
/// The live client: the socket, the reconnect resync, the outbox and the frame router under one
/// policy — which is the only thing here that is a decision rather than a wire.
/// </summary>
/// <remarks>
/// <para>
/// <b>THE SOCKET RUNS ONLY WHILE THERE IS SOMETHING TO LISTEN TO.</b> Signed in AND in a family:
/// an account at the gate has no chats, and a loop that reconnected on its behalf would be a
/// client hammering a door with no room behind it. The gate moving is what starts and stops it —
/// a sign-in, a sign-out, a join approved, a removal.
/// </para>
/// <para>
/// <b>A CONNECTION MEANS A RESYNC, AND RESYNCS DO NOT STACK.</b> Every connection starts one, not
/// just the first: what a client missed while the socket was down is exactly what the pass reads.
/// But a flapping proxy can raise three connections in a second, and three overlapping passes
/// would page the same chats three times and fight over the cursors — so a pass in flight
/// COALESCES the connections that arrive during it into one more pass, and no more than one.
/// </para>
/// <para>
/// <b>AN EXPIRED SESSION IS NOT A NETWORK ERROR.</b> The socket closing with <c>4401</c> lands
/// exactly where a REST <c>401</c> lands, and it lands once — otherwise a device whose account was
/// deleted sits behind a "Connecting…" banner looking signed in until something makes a REST call.
/// </para>
/// <para>
/// <b>THE OUTBOX HAS THREE TRIGGERS AND THE SOCKET IS ONLY ONE OF THEM.</b> A returning network
/// and a window coming forward flush it too, because waiting for the next reconnect to come round
/// on its own is what makes a message sit unsent under a full signal bar.
/// </para>
/// </remarks>
public sealed class LiveConnection : IAsyncDisposable
{
    private readonly AppSession session;
    private readonly ILiveSocket socket;
    private readonly Resync resync;
    private readonly SendPipeline sending;
    private readonly FrameRouter router;

    // Released once per connection, and never more than once at a time: the coalescing rule IS
    // this semaphore's capacity.
    private readonly SemaphoreSlim due = new(0, 1);
    private readonly object gate = new();

    /// <summary>
    /// STARTING AND STOPPING ARE SERIALISED, and it took a review round to notice why. A stop
    /// clears the fields under the lock and only THEN waits for the socket loop to wind down, so
    /// a start arriving in that window found `life` null, believed nothing was running, and put a
    /// SECOND <c>RunAsync</c> on the same socket — which is one socket object with one live
    /// connection field, so the two would have fought over it. A gate movement away and back
    /// (signed out, signed in; removed, re-approved) is exactly that window.
    /// </summary>
    private readonly SemaphoreSlim lifecycle = new(1, 1);

    private CancellationTokenSource? life;
    private Task? loop;
    private Task? passes;
    private Link link = Link.Down;

    public LiveConnection(
        AppSession session,
        ILiveSocket socket,
        Resync resync,
        SendPipeline sending,
        FrameRouter router)
    {
        this.session = session;
        this.socket = socket;
        this.resync = resync;
        this.sending = sending;
        this.router = router;

        socket.Frame += router.Hear;
        socket.Connected += OnConnected;
        socket.Disconnected += OnDisconnected;
        socket.SessionExpired += session.SessionGone;
        session.Changed += OnGateMoved;
    }

    /// <summary>What the corner of the window says.</summary>
    public Link Link
    {
        get
        {
            lock (gate)
            {
                return link;
            }
        }
    }

    /// <summary>The link changed.</summary>
    public event Action<Link>? LinkChanged;

    /// <summary>A reconnect pass finished, and what it managed.</summary>
    public event Action<Resync.Report>? Resynced;

    /// <summary>A pass could not finish. Transient means it will be tried again on the next one.</summary>
    public event Action<ApiError>? ResyncFailed;

    /// <summary>
    /// Start listening, if the session says there is anything to listen to. Idempotent: the
    /// window may call it on every appearance.
    /// </summary>
    public void Start() => _ = StartAsync();

    /// <summary>
    /// The same start, awaitable — which is what a test wants and what the fire-and-forget
    /// <see cref="Start"/> is.
    /// </summary>
    public async Task StartAsync()
    {
        await lifecycle.WaitAsync().ConfigureAwait(false);
        try
        {
            if (!session.State.CanChat || life is not null)
            {
                return;
            }
            life = new CancellationTokenSource();
            var ct = life.Token;
            loop = Task.Run(() => socket.RunAsync(ct), CancellationToken.None);
            passes = Task.Run(() => PassesAsync(ct), CancellationToken.None);
        }
        finally
        {
            lifecycle.Release();
        }
        // Outside the gate: a subscriber is somebody else's code, and holding a gate across it is
        // how a window handler ends up waiting on a socket.
        Publish(Link.Connecting);
    }

    /// <summary>
    /// Stop, and wait for both halves to finish — so that a sign-out cannot leave a pass writing
    /// into a cache the next account is about to use.
    /// </summary>
    public async Task StopAsync()
    {
        await lifecycle.WaitAsync().ConfigureAwait(false);
        try
        {
            var stopping = life;
            Task?[] running = [loop, passes];
            if (stopping is null)
            {
                return;
            }
            await stopping.CancelAsync().ConfigureAwait(false);
            foreach (var task in running)
            {
                if (task is not null)
                {
                    try
                    {
                        await task.ConfigureAwait(false);
                    }
                    catch (OperationCanceledException)
                    {
                        // Asked for.
                    }
                }
            }
            stopping.Dispose();
            // Cleared only once the loop has actually WOUND DOWN, and inside the gate, so a
            // start that follows cannot find a socket that looks free while it is still running.
            life = null;
            loop = null;
            passes = null;
        }
        finally
        {
            lifecycle.Release();
        }
        Publish(Link.Down);
    }

    /// <summary>
    /// The network came back, or the window came forward. Flushes the outbox without waiting for
    /// a reconnect to come round on its own.
    /// </summary>
    /// <returns>How many rows went.</returns>
    public Task<int> FlushAsync(
        SendRules.FlushTrigger trigger, CancellationToken ct = default) =>
        sending.FlushAsync(trigger, ct);

    /// <summary>A pass, on demand — what a pull-to-refresh is.</summary>
    public void Refresh()
    {
        try
        {
            due.Release();
        }
        catch (SemaphoreFullException)
        {
            // One is already due. That is the whole point.
        }
    }

    private void OnConnected()
    {
        Publish(Link.Up);
        Refresh();
    }

    private void OnDisconnected() => Publish(Link.Connecting);

    private void OnGateMoved(SessionState state)
    {
        if (state.CanChat)
        {
            Start();
            return;
        }
        // Signed out, removed, or waiting at the gate: nothing to listen to. Not awaited — the
        // gate moved on somebody else's thread, and a window event may not block on a socket.
        _ = StopAsync();
    }

    private async Task PassesAsync(CancellationToken ct)
    {
        while (!ct.IsCancellationRequested)
        {
            try
            {
                await due.WaitAsync(ct).ConfigureAwait(false);
            }
            catch (OperationCanceledException)
            {
                return;
            }
            try
            {
                var report = await resync.RunAsync(ct).ConfigureAwait(false);
                if (report.Stopped is { } error)
                {
                    ResyncFailed?.Invoke(error);
                }
                Resynced?.Invoke(report);
            }
            catch (OperationCanceledException)
            {
                return;
            }
            catch (Exception)
            {
                // A pass that threw is a pass that failed. The next connection starts another,
                // and a client that stopped reading because one page went wrong would be a
                // client that never recovers.
            }
        }
    }

    private void Publish(Link next)
    {
        lock (gate)
        {
            if (link == next)
            {
                return;
            }
            link = next;
        }
        LinkChanged?.Invoke(next);
    }

    public async ValueTask DisposeAsync()
    {
        socket.Frame -= router.Hear;
        socket.Connected -= OnConnected;
        socket.Disconnected -= OnDisconnected;
        socket.SessionExpired -= session.SessionGone;
        session.Changed -= OnGateMoved;
        await StopAsync().ConfigureAwait(false);
        due.Dispose();
        lifecycle.Dispose();
    }
}
