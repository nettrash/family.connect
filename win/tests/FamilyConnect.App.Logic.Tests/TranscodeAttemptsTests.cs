using FamilyConnect.App.Logic;
using Xunit;

namespace FamilyConnect.App.Logic.Tests;

/// <summary>
/// The loop around Media Foundation's transcoder — which way of asking is tried after which, and what ends the asking.
/// The attempts here are strings and the "transcoder" a lambda, because the order is the whole of what is decided.
/// </summary>
public sealed class TranscodeAttemptsTests
{
    private static readonly TimeSpan Long = TimeSpan.FromMinutes(5);

    private sealed record Made(string By);

    private static Task<Made?> RunAsync(
        IReadOnlyList<string> attempts,
        Func<string, CancellationToken, Task<Attempted<Made>>> attempt,
        List<string>? said = null,
        TimeSpan? ceiling = null,
        CancellationToken cancel = default) =>
        TranscodeAttempts.FirstTakenAsync(attempts, attempt, ceiling ?? Long, line => said?.Add(line), cancel);

    [Fact]
    public async Task TheFirstWayThatIsTakenIsTheResultAndNothingAfterItIsAsked()
    {
        var asked = new List<string>();
        var made = await RunAsync(["high", "main"], (attempt, _) =>
        {
            asked.Add(attempt);
            return Task.FromResult(Attempted<Made>.Taken(new Made(attempt)));
        });

        Assert.Equal(new Made("high"), made);
        Assert.Equal(["high"], asked);
    }

    [Fact]
    public async Task ARefusedProfileIsFollowedByTheNext()
    {
        var made = await RunAsync(["exact", "nearest", "main"], (attempt, _) =>
            Task.FromResult(attempt == "main" ? Attempted<Made>.Taken(new Made(attempt)) : Attempted<Made>.Refused));

        Assert.Equal(new Made("main"), made);
    }

    /// <summary>
    /// The encoder takes the profile when it is prepared and rejects the type once the topology starts: the transcode
    /// itself throws. That once ended the loop, so a mono clip whose 64 000 was refused late never reached 96 000 or Main.
    /// </summary>
    [Fact]
    public async Task ARefusalThatArrivesAsAnExceptionMidTranscodeIsStillFollowedByTheNext()
    {
        var asked = new List<string>();
        var said = new List<string>();
        var made = await RunAsync(
            ["64000", "96000", "main"],
            (attempt, _) =>
            {
                asked.Add(attempt);
                return attempt == "64000"
                    ? Task.FromException<Attempted<Made>>(new InvalidOperationException("MF_E_INVALIDMEDIATYPE"))
                    : Task.FromResult(Attempted<Made>.Taken(new Made(attempt)));
            },
            said);

        Assert.Equal(new Made("96000"), made);
        Assert.Equal(["64000", "96000"], asked);
        // Said with its number and with what was asked for — and never with the exception's own words.
        var line = Assert.Single(said);
        Assert.Contains("InvalidOperationException 0x", line, StringComparison.Ordinal);
        Assert.Contains("(64000)", line, StringComparison.Ordinal);
        Assert.DoesNotContain("MF_E_INVALIDMEDIATYPE", line, StringComparison.Ordinal);
    }

    /// <summary>A thrown refusal is caught whether it is thrown before the first await or after it.</summary>
    [Fact]
    public async Task AnAttemptThatThrowsSynchronouslyIsARefusalToo()
    {
        var made = await RunAsync(["a", "b"], (attempt, _) =>
            attempt == "a" ? throw new IOException("disk") : Task.FromResult(Attempted<Made>.Taken(new Made(attempt))));

        Assert.Equal(new Made("b"), made);
    }

    /// <summary>Sideways, squashed, too fast: asking again under Main would make the same picture, minutes later.</summary>
    [Fact]
    public async Task AResultThatCameOutWrongEndsTheAsking()
    {
        var asked = new List<string>();
        var made = await RunAsync(["high", "main"], (attempt, _) =>
        {
            asked.Add(attempt);
            return Task.FromResult(Attempted<Made>.Wrong);
        });

        Assert.Null(made);
        Assert.Equal(["high"], asked);
    }

    [Fact]
    public async Task EveryWayRefusedIsNoTranscodeAndSaysSo()
    {
        var said = new List<string>();
        var made = await RunAsync(["a", "b"], (_, _) => Task.FromResult(Attempted<Made>.Refused), said);

        Assert.Null(made);
        Assert.Equal(["no transcode profile was taken"], said);
        Assert.Null(await RunAsync([], (_, _) => Task.FromResult(Attempted<Made>.Refused)));
    }

    /// <summary>
    /// The person took the file back. That is not rule C — null would send the original they no longer want — so it
    /// comes out as a cancellation, and the attempt in hand was told to stop through its own token.
    /// </summary>
    [Fact]
    public async Task ACancelStopsTheAttemptInHandAndIsNotAFailure()
    {
        using var cancel = new CancellationTokenSource();
        var asked = new List<string>();
        var running = new TaskCompletionSource();
        var run = RunAsync(
            ["high", "main"],
            async (attempt, token) =>
            {
                asked.Add(attempt);
                running.SetResult();
                await Task.Delay(Timeout.Infinite, token);
                return Attempted<Made>.Refused;
            },
            cancel: cancel.Token);

        await running.Task;
        cancel.Cancel();

        // Bounded: a cancel that never reached the attempt must FAIL here, not hang the suite on its endless transcode.
        await Assert.ThrowsAnyAsync<OperationCanceledException>(() => run.WaitAsync(TimeSpan.FromSeconds(30)));
        Assert.Equal(["high"], asked);
    }

    [Fact]
    public async Task AnAlreadyCancelledRunAsksNothing()
    {
        using var cancel = new CancellationTokenSource();
        cancel.Cancel();
        var asked = new List<string>();

        await Assert.ThrowsAnyAsync<OperationCanceledException>(() => RunAsync(
            ["high"],
            (attempt, _) =>
            {
                asked.Add(attempt);
                return Task.FromResult(Attempted<Made>.Refused);
            },
            cancel: cancel.Token));
        Assert.Empty(asked);
    }

    /// <summary>
    /// A cancel that an attempt swallowed and answered over — "refused", or even a result — is still a cancel: nothing
    /// more is asked.
    /// </summary>
    [Fact]
    public async Task ACancelAnAttemptAnsweredOverStillStopsTheRest()
    {
        using var cancel = new CancellationTokenSource();
        var asked = new List<string>();

        await Assert.ThrowsAnyAsync<OperationCanceledException>(() => RunAsync(
            ["high", "main"],
            (attempt, _) =>
            {
                asked.Add(attempt);
                cancel.Cancel();
                return Task.FromResult(Attempted<Made>.Refused);
            },
            cancel: cancel.Token));
        Assert.Equal(["high"], asked);
    }

    /// <summary>
    /// ONE ceiling for every way of asking together: a stalled transcode is stopped through its token, is a failure
    /// (rule C — not a cancellation), and the next is not tried, because it would stall too.
    /// </summary>
    [Fact]
    public async Task TheCeilingStopsAStalledTranscodeAndNothingMoreIsTried()
    {
        var asked = new List<string>();
        var said = new List<string>();
        var made = await RunAsync(
            ["high", "main"],
            async (attempt, token) =>
            {
                asked.Add(attempt);
                await Task.Delay(Timeout.Infinite, token);
                return Attempted<Made>.Taken(new Made(attempt));
            },
            said,
            ceiling: TimeSpan.FromMilliseconds(50));

        Assert.Null(made);
        Assert.Equal(["high"], asked);
        Assert.Equal(["a transcode ran past its ceiling"], said);
    }

    /// <summary>The ceiling is not per attempt: one that used it up and was only REFUSED leaves none for the next.</summary>
    [Fact]
    public async Task AnAttemptThatUsedUpTheCeilingLeavesNoneForTheNext()
    {
        var asked = new List<string>();
        var made = await RunAsync(
            ["high", "main"],
            async (attempt, token) =>
            {
                asked.Add(attempt);
                try
                {
                    await Task.Delay(Timeout.Infinite, token);
                }
                catch (OperationCanceledException)
                {
                    // Swallowed, as a platform call might: the loop must still see the clock has run out.
                }
                return Attempted<Made>.Refused;
            },
            ceiling: TimeSpan.FromMilliseconds(50));

        Assert.Null(made);
        Assert.Equal(["high"], asked);
    }
}
