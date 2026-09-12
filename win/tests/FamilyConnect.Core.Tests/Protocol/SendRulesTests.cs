using FamilyConnect.Core.Protocol;

namespace FamilyConnect.Core.Tests.Protocol;

/// <summary>
/// A send has THREE outcomes, not two: delivered, refused, and unknown — and unknown, which is by
/// far the most common on a bad network, must never be shown as a refusal (docs/protocol.md,
/// "Sending on an unreliable network").
/// </summary>
public class SendRulesTests
{
    private static readonly DateTimeOffset Now = new(2026, 9, 12, 10, 0, 0, TimeSpan.Zero);

    /// <summary>The ceiling, not a sample: a random source that always returns its bound.</summary>
    private static ReconnectBackoff Ceilings() => new(random: ceiling => ceiling);

    [Fact]
    public void TheDeadlinesAndTheBudgetAreTheApps()
    {
        Assert.Equal(TimeSpan.FromSeconds(10), SendRules.AckDeadline);
        Assert.Equal(6, SendRules.MaxAttempts);
    }

    [Fact]
    public void OnlyATerminalCodeMarksAMessageFailed()
    {
        foreach (var code in SendRules.TerminalSendCodes)
        {
            var (what, next) = SendRules.Verdict(
                new ApiError(code, "…", 400), attempts: 1, Now, Ceilings());
            Assert.Equal(SendRules.Outcome.Failed, what);
            Assert.Null(next);
        }
    }

    [Fact]
    public void EverythingElseLeavesTheRowQueued()
    {
        // A transport failure says nothing about the request at all.
        var (transport, when) = SendRules.Verdict(
            ApiError.Transport("connection reset"), attempts: 1, Now, Ceilings());
        Assert.Equal(SendRules.Outcome.Retry, transport);
        Assert.NotNull(when);
        // A 5xx, a 429 and an `error` FRAME carrying a transient code are all "no answer".
        foreach (var error in new[]
        {
            new ApiError(ErrorCodes.Internal, "…", 500),
            new ApiError(ErrorCodes.TooManyRequests, "…", 429),
            new ApiError("", "…", 502),
            new ApiError(ErrorCodes.Internal, "…", 0),
        })
        {
            Assert.Equal(
                SendRules.Outcome.Retry,
                SendRules.Verdict(error, attempts: 1, Now, Ceilings()).What);
        }
    }

    [Fact]
    public void TheRetryScheduleIsTheSharedJitterShapePositionedOnTheRow()
    {
        // Positioned at the row's own attempt count, so the schedule survives a relaunch: the
        // ceilings are 1, 2, 4, 8, 16, 30 — and the last is the cap, not 32.
        double[] expected = [1, 2, 4, 8, 16, 30];
        for (var attempts = 0; attempts < expected.Length; attempts++)
        {
            var (what, next) = SendRules.Verdict(
                ApiError.Transport("offline"), attempts, Now, Ceilings());
            Assert.Equal(SendRules.Outcome.Retry, what);
            Assert.Equal(Now.AddSeconds(expected[attempts]), next);
        }
    }

    [Fact]
    public void WhenTheBudgetIsSpentItGivesUpVisibly()
    {
        var (what, next) = SendRules.Verdict(
            ApiError.Transport("offline"), attempts: SendRules.MaxAttempts, Now, Ceilings());
        // A failed row with a retry affordance is a promise that nothing else is coming.
        Assert.Equal(SendRules.Outcome.Failed, what);
        Assert.Null(next);
    }

    [Fact]
    public void TheServersOwnRetryAfterWinsOverOurJitterAndIsCapped()
    {
        var (what, next) = SendRules.Verdict(
            new ApiError(ErrorCodes.TooManyRequests, "…", 429, TimeSpan.FromSeconds(2)),
            attempts: 3, Now, Ceilings());
        Assert.Equal(SendRules.Outcome.Retry, what);
        Assert.Equal(Now.AddSeconds(2), next);
        // Honoured on every method, POST included — but capped at something sane.
        var (_, far) = SendRules.Verdict(
            new ApiError(ErrorCodes.TooManyRequests, "…", 429, TimeSpan.FromHours(3)),
            attempts: 0, Now, Ceilings());
        Assert.Equal(Now + SendRules.RetryAfterCap, far);
        // A nonsense negative wait is not a trip back in time.
        var (_, never) = SendRules.Verdict(
            new ApiError(ErrorCodes.TooManyRequests, "…", 429, TimeSpan.FromSeconds(-5)),
            attempts: 0, Now, Ceilings());
        Assert.Equal(Now, never);
    }

    /// <summary>
    /// <c>attachment_expired</c> means UPLOAD IT AGAIN, not give up — and only a client that threw
    /// its bytes away has to tell the person the media is gone.
    /// </summary>
    [Fact]
    public void AnExpiredUploadIsTheOneRefusalWithAWayOut()
    {
        var expired = new ApiError(ErrorCodes.AttachmentExpired, "…", 404);
        var (again, now) = SendRules.Verdict(expired, attempts: 2, Now, Ceilings());
        Assert.Equal(SendRules.Outcome.ReuploadAndRetry, again);
        Assert.Equal(Now, now);
        // Without the bytes there is no way back, and retrying for ever would be a lie.
        var (gone, never) = SendRules.Verdict(
            expired, attempts: 2, Now, Ceilings(), holdsBytes: false);
        Assert.Equal(SendRules.Outcome.Failed, gone);
        Assert.Null(never);
        // Its two neighbours stay terminal.
        Assert.Equal(
            SendRules.Outcome.Failed,
            SendRules.Verdict(new ApiError(ErrorCodes.AttachmentNotFound, "…", 404), 1, Now, Ceilings()).What);
        Assert.Equal(
            SendRules.Outcome.Failed,
            SendRules.Verdict(new ApiError(ErrorCodes.AttachmentAlreadyUsed, "…", 409), 1, Now, Ceilings()).What);
    }

    [Fact]
    public void ARowWithNoTimeAtAllIsDueNow()
    {
        // That is a row somebody has just asked to retry, and the point of the button is that it
        // does not wait.
        Assert.True(SendRules.IsDue(null, Now));
        Assert.True(SendRules.IsDue(Now.AddSeconds(-1), Now));
        Assert.True(SendRules.IsDue(Now, Now));
        Assert.False(SendRules.IsDue(Now.AddSeconds(1), Now));
    }
}

public class ReconnectBackoffTests
{
    [Fact]
    public void TheCeilingsDoubleAndThenStopAtTheCap()
    {
        var backoff = new ReconnectBackoff(random: ceiling => ceiling);
        double[] expected = [1, 2, 4, 8, 16, 30, 30, 30];
        foreach (var seconds in expected)
        {
            Assert.Equal(seconds, backoff.NextDelay().TotalSeconds);
        }
        // A connection that lasted starts the next drop cheap again.
        backoff.Reset();
        Assert.Equal(1, backoff.NextDelay().TotalSeconds);
    }

    [Fact]
    public void TheDelayIsUniformBelowTheCeilingNotTheCeilingItself()
    {
        // Full jitter, because a family's devices lose the same server at the same moment and
        // would otherwise stampede back in lockstep.
        var half = new ReconnectBackoff(random: ceiling => ceiling / 2);
        Assert.Equal(0.5, half.NextDelay().TotalSeconds);
        Assert.Equal(1, half.NextDelay().TotalSeconds);
        var zero = new ReconnectBackoff(random: _ => 0);
        Assert.Equal(TimeSpan.Zero, zero.NextDelay());
    }

    [Fact]
    public void AFlappingSocketSaturatesRatherThanOverflowing()
    {
        var backoff = new ReconnectBackoff(random: ceiling => ceiling);
        var delays = new List<double>();
        for (var attempt = 0; attempt < 200; attempt++)
        {
            delays.Add(backoff.NextDelay().TotalSeconds);
        }
        // Never past the cap, and never a negative or infinite delay from an overflowed
        // exponent: a socket may flap for days.
        Assert.All(delays, delay => Assert.InRange(delay, 0, 30));
        Assert.Equal(30, delays[^1]);
        Assert.Equal(62, backoff.Attempt);
    }

    /// <summary>
    /// A COMPLETED HANDSHAKE IS NOT ENOUGH: a proxy can accept the upgrade and drop the connection
    /// at once, and forgiving the ceiling there made the socket reconnect about twice a second for
    /// ever, each cycle firing a full resync.
    /// </summary>
    [Fact]
    public void OnlyAConnectionThatLastedEarnsItsReset()
    {
        var now = new DateTimeOffset(2026, 9, 12, 10, 0, 0, TimeSpan.Zero);
        var durable = TimeSpan.FromSeconds(5);
        Assert.False(ReconnectBackoff.EarnsReset(null, now, durable));
        Assert.False(ReconnectBackoff.EarnsReset(now.AddSeconds(-1), now, durable));
        Assert.True(ReconnectBackoff.EarnsReset(now.AddSeconds(-5), now, durable));
        Assert.True(ReconnectBackoff.EarnsReset(now.AddMinutes(-30), now, durable));
    }

    [Fact]
    public void AnOutboxRowsOwnTallyCanPositionTheShape()
    {
        var backoff = new ReconnectBackoff(random: ceiling => ceiling);
        backoff.AdvanceTo(4);
        Assert.Equal(16, backoff.NextDelay().TotalSeconds);
        backoff.AdvanceTo(-3);
        Assert.Equal(1, backoff.NextDelay().TotalSeconds);
        backoff.AdvanceTo(9_999);
        Assert.Equal(62, backoff.Attempt);
    }
}
