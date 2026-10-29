//! The wire carries RFC 3339 instants; the cache stores milliseconds since the epoch — a port of
//! `Times.cs`.
//!
//! An INSTANT either way, never a local time: the wire stores the moment and every client draws it
//! in the reader's own zone, which is the whole reason a family spread across two countries sees
//! the same event at the time it is for them (docs/protocol.md, "Board").
//!
//! THE C# READ IS `DateTimeOffset.TryParse` (invariant culture, assume and adjust to UTC), which
//! takes far more than the wire ever writes. What this port reads, each pinned against .NET 10 in
//! `tests/store_times.rs`: RFC 3339 (`T` or `t` or a space between date and time, `Z`/`z` or a
//! numeric offset), a stamp with NO offset, which is UTC (the C#'s `AssumeUniversal`), a time with
//! no seconds, a date alone (midnight UTC), a `,` before the fraction, offsets written `+hh:mm`,
//! `+hhmm` or `+hh` up to ±14:00, and white space around it all. Not read — `None` here where .NET
//! answers an instant: one-digit or five-digit fields (`2026-9-2`, `02026-…`), RFC 1123
//! (`Sat, 12 Sep 2026 10:00:00 GMT`) and culture spellings (`9/12/2026`). The server writes none
//! of them (`time`'s own `Rfc3339`, `server/src/models.rs`).
//!
//! .NET's range is kept on both sides: 0001-01-01 to 9999-12-31, UTC.

use time::{Date, Duration, Month, OffsetDateTime, PrimitiveDateTime, Time, UtcOffset};

/// 0001-01-01T00:00:00Z, the first instant .NET can hold.
const FIRST_MS: i64 = -62_135_596_800_000;

/// 9999-12-31T23:59:59.999Z, the last millisecond .NET can hold.
const LAST_MS: i64 = 253_402_300_799_999;

/// Milliseconds since the epoch, or `None` for an absent, empty or unreadable stamp (`Instant`).
pub fn instant(rfc3339: Option<&str>) -> Option<i64> {
    let text = rfc3339.filter(|text| !text.is_empty())?;
    let when = parse(text)?;
    let milliseconds = i64::try_from(when.unix_timestamp_nanos().div_euclid(1_000_000)).ok()?;
    (FIRST_MS..=LAST_MS)
        .contains(&milliseconds)
        .then_some(milliseconds)
}

/// Back to the wire's spelling: UTC, with a `Z`, to the millisecond — `yyyy-MM-ddTHH:mm:ss.fffZ`
/// (`Rfc3339`). `None` and 0 both mean "no stamp": 0 is this cache's spelling of absent, not 1970.
///
/// THE C#'S INVARIANT CULTURE IS LOAD-BEARING THERE — in a .NET format string `:` is the culture's
/// TIME SEPARATOR, which Finnish spells `.` — and this has no culture to get wrong: the digits are
/// written by hand.
///
/// A value outside .NET's range (only a damaged cache holds one) is `None` here, where the C#
/// throws `ArgumentOutOfRangeException`.
pub fn rfc3339(milliseconds: Option<i64>) -> Option<String> {
    let milliseconds = milliseconds.filter(|&value| value != 0)?;
    if !(FIRST_MS..=LAST_MS).contains(&milliseconds) {
        return None;
    }
    let when = from_milliseconds(milliseconds)?;
    Some(format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        when.year(),
        u8::from(when.month()),
        when.day(),
        when.hour(),
        when.minute(),
        when.second(),
        when.millisecond()
    ))
}

/// `DateTimeOffset.ToUnixTimeMilliseconds`: whole milliseconds, rounded DOWN (towards the past,
/// for an instant before 1970 too).
pub fn milliseconds(at: OffsetDateTime) -> i64 {
    // An OffsetDateTime spans ±9999 years, about 3.2e20 ns: a whole millisecond count fits an i64.
    i64::try_from(at.unix_timestamp_nanos().div_euclid(1_000_000)).unwrap_or(i64::MAX)
}

/// `DateTimeOffset.FromUnixTimeMilliseconds`, in UTC — `None` where `time` cannot hold it (the C#
/// throws there).
pub fn from_milliseconds(milliseconds: i64) -> Option<OffsetDateTime> {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(milliseconds) * 1_000_000).ok()
}

/// The stamp, as the module's notes say what is read.
fn parse(text: &str) -> Option<OffsetDateTime> {
    let mut rest = Cursor(
        text.trim_matches(|c: char| c.is_ascii_whitespace())
            .as_bytes(),
    );
    let year = rest.digits(4)?;
    rest.literal(b'-')?;
    let month = Month::try_from(u8::try_from(rest.digits(2)?).ok()?).ok()?;
    rest.literal(b'-')?;
    let day = u8::try_from(rest.digits(2)?).ok()?;
    let date = Date::from_calendar_date(i32::try_from(year).ok()?, month, day).ok()?;

    let mut time = Time::MIDNIGHT;
    // Ticks past the second, as .NET counts them: a fraction is cut to seven digits, and an eighth
    // of 5 or more rounds the seventh up (`.99999999` is the next second).
    let mut ticks: i64 = 0;
    // A space separates a time only when one follows: `2026-09-12 Z` is a date and its zone.
    let spaced_time =
        rest.0.first() == Some(&b' ') && rest.0.get(1).is_some_and(u8::is_ascii_digit);
    if spaced_time || rest.take_if(|byte| matches!(byte, b'T' | b't')) {
        if spaced_time {
            rest.skip_spaces();
        }
        let hour = u8::try_from(rest.digits(2)?).ok()?;
        rest.literal(b':')?;
        let minute = u8::try_from(rest.digits(2)?).ok()?;
        let mut second = 0;
        if rest.take_if(|byte| byte == b':') {
            second = u8::try_from(rest.digits(2)?).ok()?;
            if rest.take_if(|byte| matches!(byte, b'.' | b',')) {
                let fraction = rest.run_of_digits();
                if fraction.is_empty() {
                    return None;
                }
                for (place, digit) in fraction.iter().take(7).enumerate() {
                    ticks += i64::from(digit - b'0') * 10_i64.pow(6 - place as u32);
                }
                if fraction.get(7).is_some_and(|&digit| digit >= b'5') {
                    ticks += 1;
                }
            }
        }
        time = Time::from_hms(hour, minute, second).ok()?;
    }

    rest.skip_spaces();
    let offset = if rest.take_if(|byte| matches!(byte, b'Z' | b'z')) {
        UtcOffset::UTC
    } else if let Some(sign) = rest.take_sign() {
        let hours = rest.digits(2)?;
        let minutes = if rest.take_if(|byte| byte == b':') {
            rest.digits(2)?
        } else {
            rest.digits(2).unwrap_or(0)
        };
        if minutes > 59 || hours * 60 + minutes > 14 * 60 {
            return None;
        }
        let hours = i8::try_from(hours).ok()? * sign;
        let minutes = i8::try_from(minutes).ok()? * sign;
        UtcOffset::from_hms(hours, minutes, 0).ok()?
    } else {
        // No offset at all: UTC, as `AssumeUniversal` reads it.
        UtcOffset::UTC
    };
    if !rest.0.is_empty() {
        return None;
    }
    Some(
        PrimitiveDateTime::new(date, time).assume_offset(offset)
            + Duration::nanoseconds(ticks * 100),
    )
}

/// What is left of the stamp, read from the front.
struct Cursor<'a>(&'a [u8]);

impl<'a> Cursor<'a> {
    /// Exactly `count` ASCII digits, as a number.
    fn digits(&mut self, count: usize) -> Option<u32> {
        let (head, tail) = self.0.split_at_checked(count)?;
        if !head.iter().all(u8::is_ascii_digit) {
            return None;
        }
        self.0 = tail;
        Some(
            head.iter()
                .fold(0, |value, digit| value * 10 + u32::from(digit - b'0')),
        )
    }

    /// Every ASCII digit at the front, however many.
    fn run_of_digits(&mut self) -> &'a [u8] {
        let length = self
            .0
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        let (head, tail) = self.0.split_at(length);
        self.0 = tail;
        head
    }

    fn literal(&mut self, expected: u8) -> Option<()> {
        self.take_if(|byte| byte == expected).then_some(())
    }

    fn take_if(&mut self, accept: impl Fn(u8) -> bool) -> bool {
        match self.0.split_first() {
            Some((&first, tail)) if accept(first) => {
                self.0 = tail;
                true
            }
            _ => false,
        }
    }

    fn take_sign(&mut self) -> Option<i8> {
        if self.take_if(|byte| byte == b'+') {
            Some(1)
        } else if self.take_if(|byte| byte == b'-') {
            Some(-1)
        } else {
            None
        }
    }

    fn skip_spaces(&mut self) {
        while self.take_if(|byte| byte == b' ') {}
    }
}
