//! Full-jitter exponential backoff — `delay = random(0, min(cap, base·2ⁿ))` — for the socket's
//! reconnect loop and, borrowed, for the send outbox's own per-message schedule. A port of
//! `win/src/FamilyConnect.Core/Protocol/ReconnectBackoff.cs`.
//!
//! FULL jitter rather than a deterministic delay, because reconnects here are a thundering-herd
//! problem: a family's devices all lose the same server at the same moment (a restart, a router
//! blip) and would otherwise stampede back in lockstep.
//!
//! The random source is injectable so a test can pin the exact ceiling sequence — 1, 2, 4, … 30,
//! 30 — without statistics. Apple counterpart: `ReconnectBackoff`.

use std::collections::hash_map::RandomState;
use std::fmt;
use std::hash::{BuildHasher, Hasher};
use std::time::Duration;

use time::OffsetDateTime;

/// The highest the counter goes: 2⁶² seconds is far past any cap, and saturating here keeps the
/// exponent finite if a socket flaps for days.
const MOST_ATTEMPTS: i32 = 62;

/// Given a ceiling, a delay in seconds in `[0, ceiling]`.
pub type RandomSource = Box<dyn FnMut(f64) -> f64>;

pub struct ReconnectBackoff {
    base_seconds: f64,
    cap_seconds: f64,
    random: RandomSource,
    attempt: i32,
}

impl fmt::Debug for ReconnectBackoff {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ReconnectBackoff")
            .field("base_seconds", &self.base_seconds)
            .field("cap_seconds", &self.cap_seconds)
            .field("attempt", &self.attempt)
            .finish_non_exhaustive()
    }
}

impl Default for ReconnectBackoff {
    /// One second, doubling to thirty, uniformly jittered.
    fn default() -> Self {
        ReconnectBackoff::new(1.0, 30.0, None)
    }
}

impl ReconnectBackoff {
    /// `new ReconnectBackoff(baseSeconds, capSeconds, random)`; `None` is the system's own
    /// randomness.
    pub fn new(base_seconds: f64, cap_seconds: f64, random: Option<RandomSource>) -> Self {
        ReconnectBackoff {
            base_seconds,
            cap_seconds,
            random: random.unwrap_or_else(|| Box::new(uniform)),
            attempt: 0,
        }
    }

    /// The default shape (1 s doubling to 30 s) with the random source a test pins.
    pub fn with_random(random: impl FnMut(f64) -> f64 + 'static) -> Self {
        ReconnectBackoff::new(1.0, 30.0, Some(Box::new(random)))
    }

    /// Consecutive failures since the last [`reset`](Self::reset).
    pub fn attempt(&self) -> i32 {
        self.attempt
    }

    /// The delay before the next attempt, advancing the counter. Uniform in
    /// `[0, min(cap, base·2^attempt)]`.
    pub fn next_delay(&mut self) -> Duration {
        let ceiling = self.ceiling(self.attempt);
        // Saturate rather than overflow if a socket flaps for days.
        if self.attempt < MOST_ATTEMPTS {
            self.attempt += 1;
        }
        seconds((self.random)(ceiling))
    }

    /// Call on a connection that lasted, so the next drop starts cheap again.
    pub fn reset(&mut self) {
        self.attempt = 0;
    }

    /// Position the counter, for a caller whose tally lives somewhere else — the outbox keeps its
    /// own per-message count on the row so a retry schedule survives a relaunch, then borrows this
    /// shape to turn it into a delay.
    pub fn advance_to(&mut self, attempt: i32) {
        self.attempt = attempt.clamp(0, MOST_ATTEMPTS);
    }

    fn ceiling(&self, attempt: i32) -> f64 {
        self.cap_seconds.min(self.base_seconds * 2f64.powi(attempt))
    }

    /// Whether a finished connection earned its reset. A COMPLETED HANDSHAKE IS NOT ENOUGH, and
    /// that distinction is the whole point: a proxy can accept the upgrade and drop the connection
    /// immediately, and so can this server when a send queue overflows. Forgiving the ceiling at
    /// handshake made the ceiling never climb — the socket reconnected about twice a second
    /// indefinitely, each cycle firing a full resync. Judging by DURATION tells the two apart
    /// without asking the endpoint anything.
    pub fn earns_reset(
        connected_at: Option<OffsetDateTime>,
        now: OffsetDateTime,
        durable_after: Duration,
    ) -> bool {
        connected_at.is_some_and(|since| now - since >= durable_after)
    }
}

/// Seconds as a wait. A source that answers a negative number or not a number waits not at all —
/// where .NET made a negative `TimeSpan` (which `Task.Delay` then refused) or threw.
fn seconds(value: f64) -> Duration {
    if value.is_nan() || value <= 0.0 {
        Duration::ZERO
    } else {
        Duration::try_from_secs_f64(value).unwrap_or(Duration::MAX)
    }
}

/// The system's randomness, uniform in `[0, ceiling)` (`Random.Shared.NextDouble() * ceiling`).
/// No crate: std's `RandomState` is seeded from the operating system once per thread and each new
/// one hashes with fresh keys, which is all a jitter needs — it is not a secret.
fn uniform(ceiling: f64) -> f64 {
    let bits = RandomState::new().build_hasher().finish();
    // The top 53 bits, as a fraction of 2⁵³: exactly representable, and below 1.
    let fraction = (bits >> 11) as f64 / (1u64 << 53) as f64;
    fraction * ceiling
}
