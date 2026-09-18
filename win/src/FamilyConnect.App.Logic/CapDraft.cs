using FamilyConnect.Core;
using FamilyConnect.Core.Protocol;

namespace FamilyConnect.App.Logic;

/// <summary>
/// The member limit while its write waits: what is drawn, and the ONE write a run of changes becomes.
/// </summary>
/// <remarks>
/// <para>
/// <b>A RUN OF CLICKS IS ONE WRITE, OF THE VALUE IT ENDED ON.</b> Every change restarts a short wait,
/// and only the last one sends — stepped from what is drawn NOW, so two clicks before the next redraw
/// are two steps and not the same step twice.
/// </para>
/// <para>
/// <b>TURNED OFF STAYS OFF WHILE ITS NULL WAITS.</b> The Mac's springs back on for the wait, and a
/// second click then cancels the removal.
/// </para>
/// <para>
/// <b>ONLY THE ANSWER TO THE LAST WRITE CLEARS WHAT IS DRAWN.</b> An older answer arriving after a newer
/// change must not wipe the newer one, still on its way (the web client's tests, ported).
/// </para>
/// </remarks>
/// <param name="send">The write — which should also bring the family back, so the value drawn after it is the server's.</param>
/// <param name="wait">The pause: <see cref="Task.Delay(TimeSpan, CancellationToken)"/> in the app, a gate in a test.</param>
public sealed class CapDraft(Func<int?, Task<ApiError?>> send, Func<TimeSpan, CancellationToken, Task> wait)
{
    public static readonly TimeSpan Debounce = TimeSpan.FromMilliseconds(600);

    private readonly object gate = new();
    private int? draft;
    private bool touched;
    private long generation;
    private CancellationTokenSource? waiting;

    /// <summary>Whether the LAST write was refused or failed.</summary>
    public bool Failed { get; private set; }

    /// <summary>Something drawn changed: raised on whatever thread noticed.</summary>
    public event Action? Changed;

    /// <summary>What to draw: the change on its way, or the family's own cap when nothing is.</summary>
    public int? Drawn(int? held)
    {
        lock (gate)
        {
            return touched ? draft : held;
        }
    }

    public void Toggle(bool on, int members, int ceiling) => Change(on ? HouseRules.SeedCap(members, ceiling) : null);

    public void Step(int by, int? held, int ceiling)
    {
        if (Drawn(held) is { } now)
        {
            Change(HouseRules.ClampCap(now + by, ceiling));
        }
    }

    public void Typed(int value, int ceiling) => Change(HouseRules.ClampCap(value, ceiling));

    private void Change(int? to)
    {
        long mine;
        CancellationTokenSource source;
        lock (gate)
        {
            mine = ++generation;
            draft = to;
            touched = true;
            Failed = false;
            waiting?.Cancel();
            waiting = source = new CancellationTokenSource();
        }
        _ = SendAfterAsync(mine, to, source.Token);
        Changed?.Invoke();
    }

    private async Task SendAfterAsync(long mine, int? to, CancellationToken token)
    {
        try
        {
            await wait(Debounce, token).ConfigureAwait(true);
        }
        catch (OperationCanceledException)
        {
            // A newer change took over the wait.
            return;
        }
        ApiError? error;
        try
        {
            error = await send(to).ConfigureAwait(true);
        }
        catch (Exception e)
        {
            error = ApiError.Transport(e.GetType().Name);
        }
        lock (gate)
        {
            if (mine != generation)
            {
                return;
            }
            touched = false;
            Failed = error is not null;
        }
        Changed?.Invoke();
    }
}
