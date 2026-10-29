//! One event as an `.ics` file (`Calendar.cs`): the file itself is `fc_text::calendar::one_event`,
//! called. What `fc_text` leaves to the platform is the one thing here — turning a wire instant
//! into the stamp the file is written with, because "turning a wire instant into one is a job for
//! the platform's own date maths".
//!
//! NOTHING HERE IS ON THE WIRE (docs/protocol.md, "Board"): a client that can put an event in the
//! system calendar builds the file locally, and the server never knows whether anybody kept it.

use time::{OffsetDateTime, UtcOffset};

/// A content line's octet budget, from RFC 5545 §3.1 — `fc_text::calendar` folds at it.
pub const FOLD_AT: usize = 75;

/// An instant in the shape iCalendar wants, `YYYYMMDDTHHMMSSZ`. The wire carries an instant and
/// every client draws it in the reader's own zone, but a calendar file wants UTC and says so with
/// the Z. Digits and two letters only: nothing in it depends on the reader's language (the C#
/// pins its invariant culture; Rust has none to get wrong).
pub fn stamp(when: OffsetDateTime) -> String {
    let utc = when.to_offset(UtcOffset::UTC);
    format!(
        "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
        utc.year(),
        u8::from(utc.month()),
        utc.day(),
        utc.hour(),
        utc.minute(),
        utc.second()
    )
}
