namespace FamilyConnect.App.Logic;

/// <summary>How one way of asking for a transcode ended.</summary>
public enum AttemptEnd
{
    /// <summary>It ran and came out as asked.</summary>
    Taken,

    /// <summary>This way of asking was not taken — refused, or answered with something else. The next is tried.</summary>
    Refused,

    /// <summary>It ran and came out WRONG in a way no other way of asking would change. Nothing more is tried.</summary>
    Wrong,
}

/// <summary>One attempt's end, and what it made when it was taken.</summary>
public readonly record struct Attempted<TResult>(AttemptEnd End, TResult? Result)
    where TResult : class
{
    public static Attempted<TResult> Taken(TResult result) => new(AttemptEnd.Taken, result);

    public static Attempted<TResult> Refused { get; } = new(AttemptEnd.Refused, null);

    public static Attempted<TResult> Wrong { get; } = new(AttemptEnd.Wrong, null);
}

/// <summary>
/// The order a transcode is asked for in, and what ends the asking — the loop around Media Foundation that needs no
/// Media Foundation (<c>MediaPreparing</c> runs each attempt; <see cref="MediaEncoding"/> says what the attempts are).
/// </summary>
/// <remarks>
/// <para>
/// <b>A REFUSAL IS A REFUSAL WHENEVER IT ARRIVES.</b> An encoder may take a profile when it is prepared and reject the
/// type only once the topology starts, which surfaces as an exception out of the transcode itself. That is the same
/// answer as "cannot transcode" given late, so the next way of asking is tried — the nearest AAC rate, the Main profile
/// — exactly as if it had been refused up front. It first ended the whole loop instead, and the fallbacks the attempts
/// exist for were never reached.
/// </para>
/// <para>
/// <b>ONE CEILING FOR ALL OF THEM.</b> Attempts that each fail late could otherwise cost their ceiling apiece. When
/// the clock runs out nothing more is tried: a transcode that stalled will stall again.
/// </para>
/// <para>
/// <b>THE PERSON'S CANCEL IS NOT A FAILURE.</b> It comes out as <see cref="OperationCanceledException"/>, not as null —
/// null is rule C, which would read the original in and stage it for somebody who has just said they do not want it.
/// </para>
/// </remarks>
public static class TranscodeAttempts
{
    /// <summary>
    /// The first of <paramref name="attempts"/> that is taken, or null when none is: every one refused, one that came
    /// out wrong, or the ceiling reached. <paramref name="say"/> is told why, for the diagnostics log.
    /// </summary>
    /// <exception cref="OperationCanceledException"><paramref name="cancel"/> was cancelled.</exception>
    public static async Task<TResult?> FirstTakenAsync<T, TResult>(
        IReadOnlyList<T> attempts,
        Func<T, CancellationToken, Task<Attempted<TResult>>> attemptAsync,
        TimeSpan ceiling,
        Action<string> say,
        CancellationToken cancel)
        where TResult : class
    {
        using var clock = new CancellationTokenSource(ceiling);
        using var either = CancellationTokenSource.CreateLinkedTokenSource(cancel, clock.Token);
        foreach (var attempt in attempts)
        {
            cancel.ThrowIfCancellationRequested();
            if (clock.IsCancellationRequested)
            {
                say("a transcode ran past its ceiling");
                return null;
            }
            Attempted<TResult> ended;
            try
            {
                ended = await attemptAsync(attempt, either.Token).ConfigureAwait(false);
            }
            catch (Exception e)
            {
                cancel.ThrowIfCancellationRequested();
                if (clock.IsCancellationRequested)
                {
                    say("a transcode ran past its ceiling");
                    return null;
                }
                // The number is what tells one Media Foundation refusal from another; the type is always the same.
                say($"a transcode failed: {e.GetType().Name} 0x{e.HResult:X8} ({attempt})");
                continue;
            }
            switch (ended.End)
            {
                case AttemptEnd.Taken when ended.Result is { } result:
                    return result;
                case AttemptEnd.Wrong:
                    return null;
                default:
                    continue;
            }
        }
        say("no transcode profile was taken");
        return null;
    }
}
