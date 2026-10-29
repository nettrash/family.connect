//! The bar a shared place must clear (`LocationFixes.cs`; docs/protocol.md, "Locations"; web
//! `location.rs`, ios `LocationProvider`): a fix older than two minutes is never sent, however
//! quickly the device hands it over; 100 m is good enough to send at once; and after twenty
//! seconds the best FRESH fix seen goes, coarse or not, with its accuracy saying so honestly.

use std::cell::Cell;
use std::time::Duration;

use time::OffsetDateTime;

/// One reading of where this device is: the numbers, how sure it was, and when it was taken.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LocationFix {
    pub latitude: f64,
    pub longitude: f64,
    /// Metres, or `None` when the device did not report a usable one — drawn as a plain pin, never
    /// as perfect precision.
    pub accuracy_m: Option<f64>,
    pub taken: OffsetDateTime,
}

/// What a fix that just arrived means for the wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixVerdict {
    /// Fresh and good enough: send it.
    Accept,
    /// Fresh but coarse: keep it, in case nothing better comes.
    Hold,
    /// Too old to send at all.
    Stale,
}

/// Good enough to stop looking, in metres: a street, which is what "where are you?" asks.
pub const GOOD_ENOUGH_METRES: f64 = 100.0;

/// The oldest fix that may be sent.
pub const FRESH_ENOUGH: Duration = Duration::from_secs(2 * 60);

/// How long to look — once looking is allowed, never counting somebody reading the prompt.
pub const TIMEOUT: Duration = Duration::from_secs(20);

/// How old, and how good. Staleness is decided FIRST: a pinpoint fix from an hour ago is precisely
/// wrong, which is worse than roughly right.
pub fn judge(fix: &LocationFix, now: OffsetDateTime) -> FixVerdict {
    if now - fix.taken > FRESH_ENOUGH {
        FixVerdict::Stale
    } else if fix
        .accuracy_m
        .is_some_and(|accuracy| accuracy <= GOOD_ENOUGH_METRES)
    {
        FixVerdict::Accept
    } else {
        FixVerdict::Hold
    }
}

/// The better of two fresh fixes: the smaller circle, and a known one over an unknown one. A tie
/// keeps what was held.
pub fn better(held: Option<LocationFix>, fix: LocationFix) -> LocationFix {
    match held {
        None => fix,
        Some(kept) => match (kept.accuracy_m, fix.accuracy_m) {
            (Some(was), Some(now)) if now < was => fix,
            (None, Some(_)) => fix,
            _ => kept,
        },
    }
}

/// An accuracy the device reported, if it is a measurement at all: finite and not negative.
pub fn usable_accuracy(accuracy: f64) -> Option<f64> {
    (accuracy.is_finite() && accuracy >= 0.0).then_some(accuracy)
}

/// One wait for a fix: a good one ends it, a coarse fresh one is held for the deadline, and a
/// stale one is nothing. Per wait, never per device — the next "where are you?" must not be
/// answered with a fix held from the last one.
///
/// The C# holds a lock, because the platform offers fixes on its own thread and the deadline reads
/// on another. Here both happen on the one thread the client runs on (the shell posts the
/// platform's callbacks to the main context), so a `Cell` is the whole of it.
#[derive(Debug, Default)]
pub struct LocationHunt {
    best: Cell<Option<LocationFix>>,
}

impl LocationHunt {
    pub fn new() -> Self {
        LocationHunt::default()
    }

    /// The best fresh fix held so far — what the deadline sends, or `None` for "could not find".
    pub fn best(&self) -> Option<LocationFix> {
        self.best.get()
    }

    /// One delivery: the fix itself when it ends the wait, otherwise `None`.
    pub fn offer(&self, fix: LocationFix, now: OffsetDateTime) -> Option<LocationFix> {
        match judge(&fix, now) {
            FixVerdict::Accept => Some(fix),
            FixVerdict::Hold => {
                self.best.set(Some(better(self.best.get(), fix)));
                None
            }
            FixVerdict::Stale => None,
        }
    }
}
