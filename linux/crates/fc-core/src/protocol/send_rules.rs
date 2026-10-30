//! What a client owes a message it has been given (docs/protocol.md, "Sending on an unreliable
//! network") — a port of `win/src/FamilyConnect.Core/Protocol/SendRules.cs`. None of this is on
//! the wire; all of it is written down because two ports that each invented their own answer
//! produced two different apps.
//!
//! The failure this prevents is the worst one the product has: **a message the sender believes
//! they sent**.

use std::time::Duration;

use time::OffsetDateTime;

use super::api_error::{ApiError, error_codes};
use super::reconnect_backoff::ReconnectBackoff;

/// A `send` frame with no `ack` and no `error` inside this window is unanswered, and the client
/// re-sends over REST with the same `client_msg_id`. The frame WRITE gets the same deadline,
/// because a socket whose TCP connection is dead absorbs a write and the keepalive horizon is far
/// longer than a person's patience.
pub const ACK_DEADLINE: Duration = Duration::from_secs(10);

/// How many tries a queued row gets before the client gives up VISIBLY — a failed row with a
/// retry affordance is a promise to the user that nothing else is coming. The apps' own six.
pub const MAX_ATTEMPTS: i32 = 6;

/// Longest a client waits on a server's `Retry-After`, however large it is. Signed, like the
/// `Retry-After` it caps ([`ApiError::retry_after`]).
pub const RETRY_AFTER_CAP: time::Duration = time::Duration::minutes(2);

/// The refusals: the message will never be accepted as it stands, and the user has to be told.
/// EVERYTHING ELSE leaves the row queued.
pub const TERMINAL_SEND_CODES: [&str; 10] = [
    error_codes::VALIDATION,
    error_codes::MESSAGE_EMPTY,
    error_codes::MESSAGE_TOO_LONG,
    error_codes::NOT_CHAT_MEMBER,
    error_codes::CHAT_NOT_FOUND,
    error_codes::BLOCKED,
    error_codes::INVALID_POLL,
    error_codes::INVALID_ATTACHMENT,
    error_codes::ATTACHMENT_NOT_FOUND,
    error_codes::ATTACHMENT_ALREADY_USED,
];

/// What this client should DO about a failed send.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Outcome {
    /// Keep it queued and try again at the time [`verdict`] gives.
    Retry,

    /// The uploads it names are gone from the server. Drop the dead ids, upload the bytes again
    /// and re-send the SAME `client_msg_id` — `attachment_expired` means upload it again, not
    /// give up. A client that no longer holds the bytes tells the person the media is gone
    /// instead, which is [`Failed`](Self::Failed).
    ReuploadAndRetry,

    /// Show it failed, with a retry affordance. Nothing else is coming.
    Failed,
}

/// The verdict on one failure: what to do, and when to try if trying is the answer.
///
/// - `error`: what came back — or a transport failure, which says nothing at all.
/// - `attempts`: how many attempts this row has now had, THIS FAILURE INCLUDED — so the first
///   failure is 1, and the delay after it is the first step of the backoff. The budget is spent
///   when this reaches [`MAX_ATTEMPTS`]: six tries and then a visible failure, which is five
///   waits and not six.
/// - `now`: the clock, passed in so this stays pure and testable.
/// - `backoff`: the shared jitter shape, positioned at this row's own attempt count so a schedule
///   survives a relaunch.
/// - `holds_bytes`: whether this client still has the source bytes of every attachment it owes
///   (`true` in `win/` unless said otherwise). It must, until the message is acked — an
///   attachment id is only valid while the server still holds the upload it names, and a client
///   that threw its copy away has no way to recover.
pub fn verdict(
    error: &ApiError,
    attempts: i32,
    now: OffsetDateTime,
    backoff: &mut ReconnectBackoff,
    holds_bytes: bool,
) -> (Outcome, Option<OffsetDateTime>) {
    // An upload the sweep has already taken is the one refusal with a way out.
    if error.code == error_codes::ATTACHMENT_EXPIRED {
        return if holds_bytes {
            (Outcome::ReuploadAndRetry, Some(now))
        } else {
            (Outcome::Failed, None)
        };
    }
    if TERMINAL_SEND_CODES.contains(&error.code.as_str()) {
        return (Outcome::Failed, None);
    }
    // Not a refusal — including an `error` FRAME carrying a transient code, which is treated
    // exactly as no answer at all.
    if attempts >= MAX_ATTEMPTS {
        return (Outcome::Failed, None);
    }
    // The server's own answer wins over our jitter when it gave one: a 429 carries Retry-After
    // in delta-seconds, and a client honours it on every method, POST included.
    if let Some(wait) = error.retry_after {
        let capped = wait.clamp(time::Duration::ZERO, RETRY_AFTER_CAP);
        return (Outcome::Retry, Some(now + capped));
    }
    // The FIRST failure waits the first step, so the shape is positioned one behind the count:
    // attempts 1 → base, 2 → base·2, and so on.
    backoff.advance_to((attempts - 1).max(0));
    (Outcome::Retry, Some(now + backoff.next_delay()))
}

/// Whether a queued row is due. A row with no time at all is due now: that is a row somebody has
/// just asked to retry, and the whole point of the button is that it does not wait.
pub fn is_due(next_attempt_at: Option<OffsetDateTime>, now: OffsetDateTime) -> bool {
    next_attempt_at.is_none_or(|at| at <= now)
}

/// Why the outbox is being flushed. A RETURNING NETWORK IS A TRIGGER: waiting for the next
/// reconnect to come round on its own is what makes a message sit unsent under a full signal bar.
/// All of these flush, and the flush runs FIRST in a resync rather than last.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlushTrigger {
    ConnectivityRestored,
    SocketConnected,
    WindowActivated,
    Timer,
    UserRetried,

    /// A message was just written into the outbox, and there is no reason to wait.
    Queued,
}
