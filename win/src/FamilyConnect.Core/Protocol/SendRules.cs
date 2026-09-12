namespace FamilyConnect.Core.Protocol;

/// <summary>
/// What a client owes a message it has been given (docs/protocol.md, "Sending on an unreliable
/// network"). None of this is on the wire; all of it is written down because two ports that each
/// invented their own answer produced two different apps.
/// </summary>
/// <remarks>
/// The failure this prevents is the worst one the product has: <b>a message the sender believes
/// they sent</b>.
/// </remarks>
public static class SendRules
{
    /// <summary>
    /// A <c>send</c> frame with no <c>ack</c> and no <c>error</c> inside this window is
    /// unanswered, and the client re-sends over REST with the same <c>client_msg_id</c>. The frame
    /// WRITE gets the same deadline, because a socket whose TCP connection is dead absorbs a write
    /// and the keepalive horizon is far longer than a person's patience.
    /// </summary>
    public static readonly TimeSpan AckDeadline = TimeSpan.FromSeconds(10);

    /// <summary>
    /// How many tries a queued row gets before the client gives up VISIBLY — a failed row with a
    /// retry affordance is a promise to the user that nothing else is coming. The apps' own six.
    /// </summary>
    public const int MaxAttempts = 6;

    /// <summary>Longest a client waits on a server's <c>Retry-After</c>, however large it is.</summary>
    public static readonly TimeSpan RetryAfterCap = TimeSpan.FromMinutes(2);

    /// <summary>
    /// The refusals: the message will never be accepted as it stands, and the user has to be told.
    /// EVERYTHING ELSE leaves the row queued.
    /// </summary>
    public static readonly string[] TerminalSendCodes =
    [
        ErrorCodes.Validation,
        ErrorCodes.MessageEmpty,
        ErrorCodes.MessageTooLong,
        ErrorCodes.NotChatMember,
        ErrorCodes.ChatNotFound,
        ErrorCodes.Blocked,
        ErrorCodes.InvalidPoll,
        ErrorCodes.InvalidAttachment,
        ErrorCodes.AttachmentNotFound,
        ErrorCodes.AttachmentAlreadyUsed,
    ];

    /// <summary>What this client should DO about a failed send.</summary>
    public enum Outcome
    {
        /// <summary>Keep it queued and try again after <see cref="Next"/>.</summary>
        Retry,

        /// <summary>
        /// The uploads it names are gone from the server. Drop the dead ids, upload the bytes
        /// again and re-send the SAME <c>client_msg_id</c> — <c>attachment_expired</c> means
        /// upload it again, not give up. A client that no longer holds the bytes tells the person
        /// the media is gone instead, which is <see cref="Failed"/>.
        /// </summary>
        ReuploadAndRetry,

        /// <summary>Show it failed, with a retry affordance. Nothing else is coming.</summary>
        Failed,
    }

    /// <summary>
    /// The verdict on one failure: what to do, and when to try if trying is the answer.
    /// </summary>
    /// <param name="error">What came back — or a transport failure, which says nothing at all.</param>
    /// <param name="attempts">
    /// How many attempts this row has now had, THIS FAILURE INCLUDED — so the first failure is 1,
    /// and the delay after it is the first step of the backoff. The budget is spent when this
    /// reaches <see cref="MaxAttempts"/>: six tries and then a visible failure, which is five
    /// waits and not six.
    /// </param>
    /// <param name="now">The clock, passed in so this stays pure and testable.</param>
    /// <param name="backoff">
    /// The shared jitter shape, positioned at this row's own attempt count so a schedule survives
    /// a relaunch.
    /// </param>
    /// <param name="holdsBytes">
    /// Whether this client still has the source bytes of every attachment it owes. It must, until
    /// the message is acked — an attachment id is only valid while the server still holds the
    /// upload it names, and a client that threw its copy away has no way to recover.
    /// </param>
    public static (Outcome What, DateTimeOffset? Next) Verdict(
        ApiError error,
        int attempts,
        DateTimeOffset now,
        ReconnectBackoff backoff,
        bool holdsBytes = true)
    {
        // An upload the sweep has already taken is the one refusal with a way out.
        if (error.Code == ErrorCodes.AttachmentExpired)
        {
            return holdsBytes
                ? (Outcome.ReuploadAndRetry, now)
                : (Outcome.Failed, null);
        }
        if (TerminalSendCodes.Contains(error.Code))
        {
            return (Outcome.Failed, null);
        }
        // Not a refusal — including an `error` FRAME carrying a transient code, which is treated
        // exactly as no answer at all.
        if (attempts >= MaxAttempts)
        {
            return (Outcome.Failed, null);
        }
        // The server's own answer wins over our jitter when it gave one: a 429 carries
        // Retry-After in delta-seconds, and a client honours it on every method, POST included.
        if (error.RetryAfter is { } wait)
        {
            var capped = wait > RetryAfterCap ? RetryAfterCap : wait;
            return (Outcome.Retry, now + (capped < TimeSpan.Zero ? TimeSpan.Zero : capped));
        }
        // The FIRST failure waits the first step, so the shape is positioned one behind the
        // count: attempts 1 → base, 2 → base·2, and so on.
        backoff.AdvanceTo(Math.Max(0, attempts - 1));
        return (Outcome.Retry, now + backoff.NextDelay());
    }

    /// <summary>
    /// Whether a queued row is due. A row with no time at all is due now: that is a row somebody
    /// has just asked to retry, and the whole point of the button is that it does not wait.
    /// </summary>
    public static bool IsDue(DateTimeOffset? nextAttemptAt, DateTimeOffset now) =>
        nextAttemptAt is null || nextAttemptAt <= now;

    /// <summary>
    /// Why the outbox is being flushed. A RETURNING NETWORK IS A TRIGGER: waiting for the next
    /// reconnect to come round on its own is what makes a message sit unsent under a full signal
    /// bar. All of these flush, and the flush runs FIRST in a resync rather than last.
    /// </summary>
    public enum FlushTrigger
    {
        ConnectivityRestored,
        SocketConnected,
        WindowActivated,
        Timer,
        UserRetried,
    }
}
