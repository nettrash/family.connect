//! A send has THREE outcomes, not two: delivered, refused, and unknown — and unknown, which is by
//! far the most common on a bad network, must never be shown as a refusal (docs/protocol.md,
//! "Sending on an unreliable network"). A port of
//! `win/tests/FamilyConnect.Core.Tests/Protocol/SendRulesTests.cs` — its two classes,
//! `SendRulesTests` and `ReconnectBackoffTests` — test for test.

use std::time::Duration;

use fc_core::protocol::send_rules::{self, Outcome};
use fc_core::protocol::{ApiError, ReconnectBackoff, error_codes};
use time::OffsetDateTime;
use time::macros::datetime;

const NOW: OffsetDateTime = datetime!(2026-09-12 10:00:00 UTC);

/// The ceiling, not a sample: a random source that always returns its bound.
fn ceilings() -> ReconnectBackoff {
    ReconnectBackoff::with_random(|ceiling| ceiling)
}

fn verdict(error: &ApiError, attempts: i32) -> (Outcome, Option<OffsetDateTime>) {
    send_rules::verdict(error, attempts, NOW, &mut ceilings(), true)
}

fn seconds(value: f64) -> time::Duration {
    time::Duration::seconds_f64(value)
}

// ---- SendRulesTests -------------------------------------------------------------------------

#[test]
fn the_deadlines_and_the_budget_are_the_apps() {
    assert_eq!(send_rules::ACK_DEADLINE, Duration::from_secs(10));
    assert_eq!(send_rules::MAX_ATTEMPTS, 6);
}

/// The budget is SIX TRIES, counting the failure being judged — so the sixth failure is the one
/// that says so, and a client that waited a seventh delay would be promising something that
/// never comes.
#[test]
fn the_sixth_failure_is_the_visible_one() {
    for failures in 1..send_rules::MAX_ATTEMPTS {
        assert_eq!(
            verdict(&ApiError::transport("offline"), failures).0,
            Outcome::Retry
        );
    }
    assert_eq!(
        verdict(&ApiError::transport("offline"), send_rules::MAX_ATTEMPTS).0,
        Outcome::Failed
    );
}

#[test]
fn only_a_terminal_code_marks_a_message_failed() {
    for code in send_rules::TERMINAL_SEND_CODES {
        let (what, next) = verdict(&ApiError::new(code, "…", 400), 1);
        assert_eq!(what, Outcome::Failed, "{code}");
        assert_eq!(next, None, "{code}");
    }
}

#[test]
fn everything_else_leaves_the_row_queued() {
    // A transport failure says nothing about the request at all.
    let (transport, when) = verdict(&ApiError::transport("connection reset"), 1);
    assert_eq!(transport, Outcome::Retry);
    assert!(when.is_some());
    // A 5xx, a 429 and an `error` FRAME carrying a transient code are all "no answer".
    for error in [
        ApiError::new(error_codes::INTERNAL, "…", 500),
        ApiError::new(error_codes::TOO_MANY_REQUESTS, "…", 429),
        ApiError::new("", "…", 502),
        ApiError::new(error_codes::INTERNAL, "…", 0),
    ] {
        assert_eq!(verdict(&error, 1).0, Outcome::Retry, "{error:?}");
    }
}

#[test]
fn the_retry_schedule_is_the_shared_jitter_shape_positioned_on_the_row() {
    // Positioned at the row's own attempt count, so the schedule survives a relaunch. The FIRST
    // failure waits the first step: 1, 2, 4, 8, 16 — five waits inside a budget of six tries,
    // and the sixth failure is the visible one (see below).
    let expected = [1.0, 2.0, 4.0, 8.0, 16.0];
    for failures in 1..=expected.len() {
        let (what, next) = verdict(&ApiError::transport("offline"), failures as i32);
        assert_eq!(what, Outcome::Retry);
        assert_eq!(next, Some(NOW + seconds(expected[failures - 1])));
    }
    // And the cap binds rather than the doubling, for a row whose count somehow ran on.
    let (_, capped) = verdict(&ApiError::transport("offline"), 40);
    assert_eq!(capped, None);
}

#[test]
fn when_the_budget_is_spent_it_gives_up_visibly() {
    let (what, next) = verdict(&ApiError::transport("offline"), send_rules::MAX_ATTEMPTS);
    // A failed row with a retry affordance is a promise that nothing else is coming.
    assert_eq!(what, Outcome::Failed);
    assert_eq!(next, None);
}

#[test]
fn the_servers_own_retry_after_wins_over_our_jitter_and_is_capped() {
    let slow = ApiError::new(error_codes::TOO_MANY_REQUESTS, "…", 429)
        .with_retry_after(time::Duration::seconds(2));
    let (what, next) = verdict(&slow, 3);
    assert_eq!(what, Outcome::Retry);
    assert_eq!(next, Some(NOW + seconds(2.0)));
    // Honoured on every method, POST included — but capped at something sane.
    let far = ApiError::new(error_codes::TOO_MANY_REQUESTS, "…", 429)
        .with_retry_after(time::Duration::hours(3));
    let (_, far) = verdict(&far, 0);
    assert_eq!(far, Some(NOW + send_rules::RETRY_AFTER_CAP));
    // A nonsense negative wait is not a trip back in time.
    let never = ApiError::new(error_codes::TOO_MANY_REQUESTS, "…", 429)
        .with_retry_after(time::Duration::seconds(-5));
    let (_, never) = verdict(&never, 0);
    assert_eq!(never, Some(NOW));
}

/// `attachment_expired` means UPLOAD IT AGAIN, not give up — and only a client that threw its
/// bytes away has to tell the person the media is gone.
#[test]
fn an_expired_upload_is_the_one_refusal_with_a_way_out() {
    let expired = ApiError::new(error_codes::ATTACHMENT_EXPIRED, "…", 404);
    let (again, now) = verdict(&expired, 2);
    assert_eq!(again, Outcome::ReuploadAndRetry);
    assert_eq!(now, Some(NOW));
    // Without the bytes there is no way back, and retrying for ever would be a lie.
    let (gone, never) = send_rules::verdict(&expired, 2, NOW, &mut ceilings(), false);
    assert_eq!(gone, Outcome::Failed);
    assert_eq!(never, None);
    // Its two neighbours stay terminal.
    assert_eq!(
        verdict(
            &ApiError::new(error_codes::ATTACHMENT_NOT_FOUND, "…", 404),
            1
        )
        .0,
        Outcome::Failed
    );
    assert_eq!(
        verdict(
            &ApiError::new(error_codes::ATTACHMENT_ALREADY_USED, "…", 409),
            1
        )
        .0,
        Outcome::Failed
    );
}

#[test]
fn a_row_with_no_time_at_all_is_due_now() {
    // That is a row somebody has just asked to retry, and the point of the button is that it
    // does not wait.
    assert!(send_rules::is_due(None, NOW));
    assert!(send_rules::is_due(Some(NOW - seconds(1.0)), NOW));
    assert!(send_rules::is_due(Some(NOW), NOW));
    assert!(!send_rules::is_due(Some(NOW + seconds(1.0)), NOW));
}

// ---- ReconnectBackoffTests ------------------------------------------------------------------

#[test]
fn the_ceilings_double_and_then_stop_at_the_cap() {
    let mut backoff = ReconnectBackoff::with_random(|ceiling| ceiling);
    for seconds in [1.0, 2.0, 4.0, 8.0, 16.0, 30.0, 30.0, 30.0] {
        assert_eq!(backoff.next_delay().as_secs_f64(), seconds);
    }
    // A connection that lasted starts the next drop cheap again.
    backoff.reset();
    assert_eq!(backoff.next_delay().as_secs_f64(), 1.0);
}

#[test]
fn the_delay_is_uniform_below_the_ceiling_not_the_ceiling_itself() {
    // Full jitter, because a family's devices lose the same server at the same moment and would
    // otherwise stampede back in lockstep.
    let mut half = ReconnectBackoff::with_random(|ceiling| ceiling / 2.0);
    assert_eq!(half.next_delay().as_secs_f64(), 0.5);
    assert_eq!(half.next_delay().as_secs_f64(), 1.0);
    let mut zero = ReconnectBackoff::with_random(|_| 0.0);
    assert_eq!(zero.next_delay(), Duration::ZERO);
}

#[test]
fn a_flapping_socket_saturates_rather_than_overflowing() {
    let mut backoff = ReconnectBackoff::with_random(|ceiling| ceiling);
    let delays: Vec<f64> = (0..200)
        .map(|_| backoff.next_delay().as_secs_f64())
        .collect();
    // Never past the cap, and never a negative or infinite delay from an overflowed exponent: a
    // socket may flap for days.
    assert!(delays.iter().all(|delay| (0.0..=30.0).contains(delay)));
    assert_eq!(delays[delays.len() - 1], 30.0);
    assert_eq!(backoff.attempt(), 62);
}

/// A COMPLETED HANDSHAKE IS NOT ENOUGH: a proxy can accept the upgrade and drop the connection
/// at once, and forgiving the ceiling there made the socket reconnect about twice a second for
/// ever, each cycle firing a full resync.
#[test]
fn only_a_connection_that_lasted_earns_its_reset() {
    let now = datetime!(2026-09-12 10:00:00 UTC);
    let durable = Duration::from_secs(5);
    assert!(!ReconnectBackoff::earns_reset(None, now, durable));
    assert!(!ReconnectBackoff::earns_reset(
        Some(now - seconds(1.0)),
        now,
        durable
    ));
    assert!(ReconnectBackoff::earns_reset(
        Some(now - seconds(5.0)),
        now,
        durable
    ));
    assert!(ReconnectBackoff::earns_reset(
        Some(now - time::Duration::minutes(30)),
        now,
        durable
    ));
}

#[test]
fn an_outbox_rows_own_tally_can_position_the_shape() {
    let mut backoff = ReconnectBackoff::with_random(|ceiling| ceiling);
    backoff.advance_to(4);
    assert_eq!(backoff.next_delay().as_secs_f64(), 16.0);
    backoff.advance_to(-3);
    assert_eq!(backoff.next_delay().as_secs_f64(), 1.0);
    backoff.advance_to(9_999);
    assert_eq!(backoff.attempt(), 62);
}

// ---- this port's own: what .NET did that a Rust `Duration` cannot ----------------------------

/// A random source that answers nonsense waits not at all: .NET made a negative `TimeSpan` of a
/// negative answer and threw on `NaN`; a `Duration` has neither, so both are a zero wait.
#[test]
fn a_nonsense_random_answer_is_no_wait_at_all() {
    let mut negative = ReconnectBackoff::with_random(|_| -1.0);
    assert_eq!(negative.next_delay(), Duration::ZERO);
    let mut nan = ReconnectBackoff::with_random(|_| f64::NAN);
    assert_eq!(nan.next_delay(), Duration::ZERO);
}

/// The system's own randomness stays under the ceiling and is not a constant.
#[test]
fn the_default_source_is_jitter_below_the_ceiling() {
    let mut backoff = ReconnectBackoff::default();
    backoff.advance_to(5);
    let delays: Vec<Duration> = (0..64)
        .map(|_| {
            backoff.advance_to(5);
            backoff.next_delay()
        })
        .collect();
    assert!(delays.iter().all(|delay| *delay < Duration::from_secs(30)));
    assert!(delays.iter().any(|delay| *delay != delays[0]));
}
