namespace FamilyConnect.Core.Protocol;

/// <summary>
/// Full-jitter exponential backoff — <c>delay = random(0, min(cap, base·2ⁿ))</c> — for the socket's
/// reconnect loop and, borrowed, for the send outbox's own per-message schedule.
/// </summary>
/// <remarks>
/// <para>
/// FULL jitter rather than a deterministic delay, because reconnects here are a thundering-herd
/// problem: a family's devices all lose the same server at the same moment (a restart, a router
/// blip) and would otherwise stampede back in lockstep.
/// </para>
/// <para>
/// The random source is injectable so a test can pin the exact ceiling sequence — 1, 2, 4, … 30,
/// 30 — without statistics. Apple counterpart: <c>ReconnectBackoff</c>.
/// </para>
/// </remarks>
public sealed class ReconnectBackoff(
    double baseSeconds = 1.0,
    double capSeconds = 30.0,
    Func<double, double>? random = null)
{
    private readonly Func<double, double> random = random ?? (ceiling => Random.Shared.NextDouble() * ceiling);

    /// <summary>Consecutive failures since the last <see cref="Reset"/>.</summary>
    public int Attempt { get; private set; }

    /// <summary>
    /// The delay before the next attempt, advancing the counter. Uniform in
    /// <c>[0, min(cap, base·2^attempt)]</c>.
    /// </summary>
    public TimeSpan NextDelay()
    {
        var ceiling = Ceiling(Attempt);
        // Saturate rather than overflow if a socket flaps for days.
        if (Attempt < 62)
        {
            Attempt++;
        }
        return TimeSpan.FromSeconds(random(ceiling));
    }

    /// <summary>Call on a connection that lasted, so the next drop starts cheap again.</summary>
    public void Reset() => Attempt = 0;

    /// <summary>
    /// Position the counter, for a caller whose tally lives somewhere else — the outbox keeps its
    /// own per-message count on the row so a retry schedule survives a relaunch, then borrows this
    /// shape to turn it into a delay.
    /// </summary>
    public void AdvanceTo(int attempt) => Attempt = Math.Clamp(attempt, 0, 62);

    private double Ceiling(int attempt) => Math.Min(capSeconds, baseSeconds * Math.Pow(2, attempt));

    /// <summary>
    /// Whether a finished connection earned its reset. A COMPLETED HANDSHAKE IS NOT ENOUGH, and
    /// that distinction is the whole point: a proxy can accept the upgrade and drop the connection
    /// immediately, and so can this server when a send queue overflows. Forgiving the ceiling at
    /// handshake made the ceiling never climb — the socket reconnected about twice a second
    /// indefinitely, each cycle firing a full resync. Judging by DURATION tells the two apart
    /// without asking the endpoint anything.
    /// </summary>
    public static bool EarnsReset(DateTimeOffset? connectedAt, DateTimeOffset now, TimeSpan durableAfter) =>
        connectedAt is { } since && now - since >= durableAfter;
}
