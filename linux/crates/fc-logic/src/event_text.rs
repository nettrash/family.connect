//! When an event is, as the wall writes it — a CALENDAR ENTRY: the day's number over its short
//! month, and the time beside it (docs/protocol.md, "Board"). The reader's own language and zone:
//! the wire carries an instant, and "16:00" is only true somewhere (`EventText.cs`).
//!
//! The C#'s `CultureInfo` and `TimeZoneInfo` are [`DateCulture`] and [`TimeZone`]
//! (`date_culture`), which the shell builds from the desktop's locale and zone.

use time::{Date, Month, OffsetDateTime, PrimitiveDateTime, Time, UtcOffset};

use crate::date_culture::{DateCulture, TimeZone, local};

/// An instant off the wire, or `None` for one that cannot be read.
///
/// The C# asks .NET's `DateTimeOffset.TryParse` (invariant culture, a missing offset read as
/// UTC), which reads far more than the wire ever sends. This reads what an RFC 3339 instant can
/// look like and what that parser measured to accept around it (.NET 10, the oracle): a date
/// `YYYY-MM-DD`; then optionally `T`, `t` or spaces and `HH:MM`, `:SS` and any fraction (kept to
/// the nanosecond, where .NET rounds to the 100 ns tick); then optionally `Z`/`z` or an offset
/// `±HH`, `±HHMM` or `±HH:MM` of at most 14 hours; spaces around it ignored. No time is midnight
/// and no offset is UTC, as there. Refused, as there: hour 24, second 60, a day the month does not
/// have, an offset past 14 hours, `20261224T160000Z`. NOT read, unlike there: the culture-free
/// prose .NET also takes ("12/24/2026", "Dec 24 2026") — the protocol's instants are RFC 3339.
pub fn instant(rfc3339: Option<&str>) -> Option<OffsetDateTime> {
    let text = rfc3339?.trim_matches(|c: char| c.is_whitespace());
    if text.is_empty() {
        return None;
    }
    let bytes = text.as_bytes();
    let mut reader = Reader { bytes, at: 0 };
    let year = reader.number(4)?;
    reader.expect(b'-')?;
    let month = reader.number(2)?;
    reader.expect(b'-')?;
    let day = reader.number(2)?;
    let date = Date::from_calendar_date(
        i32::try_from(year).ok()?,
        Month::try_from(u8::try_from(month).ok()?).ok()?,
        u8::try_from(day).ok()?,
    )
    .ok()?;
    let mut time = Time::MIDNIGHT;
    if matches!(reader.peek(), Some(b'T' | b't' | b' ')) {
        while matches!(reader.peek(), Some(b'T' | b't' | b' ')) {
            reader.at += 1;
        }
        let hour = reader.number(2)?;
        reader.expect(b':')?;
        let minute = reader.number(2)?;
        let mut second = 0;
        let mut nanosecond = 0u32;
        if reader.peek() == Some(b':') {
            reader.at += 1;
            second = reader.number(2)?;
            if reader.peek() == Some(b'.') {
                reader.at += 1;
                let start = reader.at;
                let mut scale = 100_000_000u32;
                while let Some(digit) = reader.peek().filter(u8::is_ascii_digit) {
                    nanosecond += u32::from(digit - b'0') * scale;
                    scale /= 10;
                    reader.at += 1;
                }
                if reader.at == start {
                    return None;
                }
            }
        }
        time = Time::from_hms_nano(
            u8::try_from(hour).ok()?,
            u8::try_from(minute).ok()?,
            u8::try_from(second).ok()?,
            nanosecond,
        )
        .ok()?;
    }
    let offset = match reader.peek() {
        None => UtcOffset::UTC,
        Some(b'Z' | b'z') => {
            reader.at += 1;
            UtcOffset::UTC
        }
        Some(sign @ (b'+' | b'-')) => {
            reader.at += 1;
            let hours = reader.number(2)?;
            let minutes = match reader.peek() {
                Some(b':') => {
                    reader.at += 1;
                    reader.number(2)?
                }
                Some(digit) if digit.is_ascii_digit() => reader.number(2)?,
                _ => 0,
            };
            let total = hours * 60 + minutes;
            if minutes > 59 || total > 14 * 60 {
                return None;
            }
            let seconds = i32::try_from(total * 60).ok()?;
            UtcOffset::from_whole_seconds(if sign == b'-' { -seconds } else { seconds }).ok()?
        }
        Some(_) => return None,
    };
    if reader.at != bytes.len() {
        return None;
    }
    Some(PrimitiveDateTime::new(date, time).assume_offset(offset))
}

/// The block: the day's number and its short month.
pub fn date_block(
    starts_at: &str,
    culture: &DateCulture,
    zone: &dyn TimeZone,
) -> Option<(String, String)> {
    let at = wall(Some(starts_at), zone)?;
    Some((culture.format(at, "%d"), culture.format(at, "MMM")))
}

/// The time, beside the block that already says the date: "16:00", "16:00 – 20:00", or
/// "16:00 – 25 Dec 02:00" when it ends on another day.
pub fn clock(
    starts_at: &str,
    ends_at: Option<&str>,
    culture: &DateCulture,
    zone: &dyn TimeZone,
) -> String {
    let Some(starts) = wall(Some(starts_at), zone) else {
        return String::new();
    };
    let from = culture.format(starts, "t");
    let Some(ends) = wall(ends_at, zone) else {
        return from;
    };
    let to = culture.format(ends, "t");
    if ends.date() == starts.date() {
        format!("{from} – {to}")
    } else {
        format!(
            "{from} – {} {to}",
            culture.format(ends, &short_month_day(culture))
        )
    }
}

/// The whole of it in one line, weekday included — for a label that stands in for the card.
pub fn when(
    starts_at: &str,
    ends_at: Option<&str>,
    culture: &DateCulture,
    zone: &dyn TimeZone,
) -> String {
    match wall(Some(starts_at), zone) {
        Some(starts) => format!(
            "{}, {}",
            culture.format(starts, &format!("ddd, {}", short_month_day(culture))),
            clock(starts_at, ends_at, culture, zone)
        ),
        None => String::new(),
    }
}

/// Over: its end has passed, or — with no end — its start.
pub fn is_past(starts_at: &str, ends_at: Option<&str>, now: OffsetDateTime) -> bool {
    instant(ends_at)
        .or_else(|| instant(Some(starts_at)))
        .is_some_and(|at| at < now)
}

/// The instant on the reader's wall clock, as a date and a time with the offset dropped.
fn wall(rfc3339: Option<&str>, zone: &dyn TimeZone) -> Option<PrimitiveDateTime> {
    let at = local(instant(rfc3339)?, zone);
    Some(PrimitiveDateTime::new(at.date(), at.time()))
}

/// The culture's own month-and-day, with the month abbreviated: "Dec 25", "25 Dec".
fn short_month_day(culture: &DateCulture) -> String {
    culture.month_day_pattern.replace("MMMM", "MMM")
}

/// ASCII digits read from the front of an instant.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn expect(&mut self, byte: u8) -> Option<()> {
        (self.peek()? == byte).then(|| self.at += 1)
    }

    /// Exactly `width` ASCII digits.
    fn number(&mut self, width: usize) -> Option<u32> {
        let digits = self.bytes.get(self.at..self.at + width)?;
        if !digits.iter().all(u8::is_ascii_digit) {
            return None;
        }
        self.at += width;
        Some(
            digits
                .iter()
                .fold(0, |value, digit| value * 10 + u32::from(digit - b'0')),
        )
    }
}
