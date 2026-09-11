//! Time as the reader knows it: their clock, their time zone, their
//! language's way of writing a date.
//!
//! The wire carries RFC 3339 UTC. Everything here goes through the browser's
//! own `Date` and `Intl`, which already know the reader's zone and locale —
//! the same thing the apps get from the operating system, and the reason
//! this client carries no date library of its own.

use fc_text::i18n::t;
use js_sys::{Date, Object, Reflect};
use wasm_bindgen::JsValue;

/// A calendar day in the reader's own time zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Day {
    pub year: u32,
    /// 0-based, as JavaScript counts months.
    pub month: u32,
    pub day: u32,
}

fn date_of(rfc3339: &str) -> Option<Date> {
    let date = Date::new(&JsValue::from_str(rfc3339));
    (!date.get_time().is_nan()).then_some(date)
}

fn day_of_date(date: &Date) -> Day {
    Day {
        year: date.get_full_year(),
        month: date.get_month(),
        day: date.get_date(),
    }
}

/// The local day a timestamp falls on.
pub fn day(rfc3339: &str) -> Option<Day> {
    date_of(rfc3339).map(|date| day_of_date(&date))
}

/// The day before `day`, by the calendar rather than by 24 hours — which
/// on the night a clock changes is a different answer.
fn previous(day: Day) -> Day {
    let date = Date::new_with_year_month_day(day.year, day.month as i32, day.day as i32 - 1);
    day_of_date(&date)
}

fn today(now_ms: f64) -> Day {
    day_of_date(&Date::new(&JsValue::from_f64(now_ms)))
}

/// The reader's language, as the browser reports it.
fn locale() -> String {
    web_sys::window()
        .map(|window| window.navigator().language().unwrap_or_default())
        .filter(|tag| !tag.is_empty())
        .unwrap_or_else(|| "en".to_string())
}

fn options(pairs: &[(&str, &str)]) -> JsValue {
    let object = Object::new();
    for (key, value) in pairs {
        let _ = Reflect::set(&object, &JsValue::from_str(key), &JsValue::from_str(value));
    }
    object.into()
}

/// An event's date as a CALENDAR BLOCK: the day's number and its short
/// month, both in the reader's own language (docs/protocol.md, "Board").
///
/// Two strings rather than one, because they are drawn one over the other —
/// which is the whole point of the block — and because a joined "24 Dec"
/// would put them in an order some languages do not use.
pub fn date_block(rfc3339: &str) -> Option<(String, String)> {
    let date = date_of(rfc3339)?;
    let day: String = date
        .to_locale_date_string(&locale(), &options(&[("day", "numeric")]))
        .into();
    let month: String = date
        .to_locale_date_string(&locale(), &options(&[("month", "short")]))
        .into();
    Some((day, month))
}

/// The event's TIME, beside the block that already says the date: "16:00",
/// "16:00 – 20:00", or "16:00 – 25 Dec 02:00" when it ends on another day.
pub fn event_clock(starts_at: &str, ends_at: Option<&str>) -> String {
    let Some(starts) = date_of(starts_at) else {
        return String::new();
    };
    let from = clock(starts_at);
    let Some(ends) = ends_at.and_then(date_of) else {
        return from;
    };
    let to = clock(ends_at.unwrap_or_default());
    if day_of_date(&ends) == day_of_date(&starts) {
        format!("{from} – {to}")
    } else {
        let end_day: String = ends
            .to_locale_date_string(
                &locale(),
                &options(&[("month", "short"), ("day", "numeric")]),
            )
            .into();
        format!("{from} – {end_day} {to}")
    }
}

/// A wire instant as iCalendar spells one: `YYYYMMDDTHHMMSSZ`, in UTC.
///
/// Through the platform's own date maths (`toISOString`) rather than string
/// arithmetic: the wire's instant may carry fractional seconds or an
/// offset, and only a real parse gets every shape right.
pub fn ics_stamp(rfc3339: &str) -> Option<String> {
    let iso: String = date_of(rfc3339)?.to_iso_string().into();
    // "2026-12-24T16:00:00.000Z" → "20261224T160000Z"
    let (date, rest) = iso.split_once('T')?;
    let clock = rest.split('.').next().unwrap_or(rest).trim_end_matches('Z');
    Some(format!(
        "{}T{}Z",
        date.replace('-', ""),
        clock.replace(':', "")
    ))
}

/// Now, in the same shape — what an `.ics` stamps itself with.
pub fn ics_now() -> String {
    let iso: String = Date::new_0().to_iso_string().into();
    iso.split_once('T')
        .and_then(|(date, rest)| {
            let clock = rest.split('.').next()?.trim_end_matches('Z');
            Some(format!(
                "{}T{}Z",
                date.replace('-', ""),
                clock.replace(':', "")
            ))
        })
        .unwrap_or_default()
}

/// "17:03", or "5:03 PM" — hours and minutes the way the reader's language
/// writes them. Empty for a timestamp that cannot be read.
pub fn clock(rfc3339: &str) -> String {
    let Some(date) = date_of(rfc3339) else {
        return String::new();
    };
    date.to_locale_time_string_with_options(
        &locale(),
        &options(&[("hour", "numeric"), ("minute", "2-digit")]),
    )
    .into()
}

/// The pill between day sections: "Today", "Yesterday", otherwise the
/// weekday, month and day abbreviated — "Mon, Aug 17" — with no year, the
/// same three labels the apps draw (ios MacDayPill).
pub fn day_label(day: Day, now_ms: f64) -> String {
    let today = today(now_ms);
    if day == today {
        return t("Today").to_string();
    }
    if day == previous(today) {
        return t("Yesterday").to_string();
    }
    let date = Date::new_with_year_month_day(day.year, day.month as i32, day.day as i32);
    date.to_locale_date_string(
        &locale(),
        &options(&[("weekday", "short"), ("month", "short"), ("day", "numeric")]),
    )
    .into()
}

/// The short time on a chat row: the clock for today, "Yesterday", the
/// weekday within the last week, and a short date before that.
pub fn row_time(rfc3339: &str, now_ms: f64) -> String {
    let Some(date) = date_of(rfc3339) else {
        return String::new();
    };
    let when = day_of_date(&date);
    let today = today(now_ms);
    if when == today {
        return clock(rfc3339);
    }
    if when == previous(today) {
        return t("Yesterday").to_string();
    }
    let age_days = (now_ms - date.get_time()) / 86_400_000.0;
    if (0.0..7.0).contains(&age_days) {
        return date
            .to_locale_date_string(&locale(), &options(&[("weekday", "short")]))
            .into();
    }
    date.to_locale_date_string(
        &locale(),
        &options(&[("month", "short"), ("day", "numeric")]),
    )
    .into()
}

/// When an event is, for its sticker and its note: "Sat, Sep 12, 11:00",
/// with the end after a dash — its time alone on the same day, its date too
/// on another. In the reader's zone and language, deliberately: the wire
/// carries an instant, and a family in two countries each sees the moment
/// in their own (ios EventFormat.when).
pub fn event_when(starts_at: &str, ends_at: Option<&str>) -> String {
    let Some(starts) = date_of(starts_at) else {
        return String::new();
    };
    let day: String = starts
        .to_locale_date_string(
            &locale(),
            &options(&[("weekday", "short"), ("month", "short"), ("day", "numeric")]),
        )
        .into();
    let from = clock(starts_at);
    let Some(ends) = ends_at.and_then(date_of) else {
        return format!("{day}, {from}");
    };
    let to = clock(ends_at.unwrap_or_default());
    if day_of_date(&ends) == day_of_date(&starts) {
        format!("{day}, {from} – {to}")
    } else {
        let end_day: String = ends
            .to_locale_date_string(
                &locale(),
                &options(&[("month", "short"), ("day", "numeric")]),
            )
            .into();
        format!("{day}, {from} – {end_day} {to}")
    }
}

/// A birthday as the reader's language writes a day of the year — "March
/// 14", "14 марта", "3月14日" — and never a year: a birthday has none on the
/// wire, and 2024 is only there so the 29th of February resolves (ios
/// Birthday.formatted).
pub fn birthday(month: u32, day: u32) -> String {
    let date = Date::new(&JsValue::from_f64(
        Date::utc(2024.0, f64::from(month) - 1.0) + f64::from(day.saturating_sub(1)) * 86_400_000.0,
    ));
    date.to_locale_date_string(
        &locale(),
        &options(&[("month", "long"), ("day", "numeric"), ("timeZone", "UTC")]),
    )
    .into()
}

/// A month's name standing alone, as a picker lists it — "January",
/// "январь" — in the reader's language.
pub fn month_name(month: u32) -> String {
    let date = Date::new(&JsValue::from_f64(Date::utc(
        2024.0,
        f64::from(month) - 1.0,
    )));
    date.to_locale_date_string(
        &locale(),
        &options(&[("month", "long"), ("timeZone", "UTC")]),
    )
    .into()
}

/// Whether an event has been and gone — its end, or its start when it has
/// none, is behind `now_ms`. A past event is drawn quieter, never removed:
/// clearing the wall is the family's call.
pub fn is_past(starts_at: &str, ends_at: Option<&str>, now_ms: f64) -> bool {
    ends_at
        .and_then(date_of)
        .or_else(|| date_of(starts_at))
        .is_some_and(|date| date.get_time() < now_ms)
}

/// An instant as a `datetime-local` input writes it — "2026-09-12T11:00" in
/// the reader's own zone. Empty for a timestamp that cannot be read.
pub fn local_input(rfc3339: &str) -> String {
    let Some(date) = date_of(rfc3339) else {
        return String::new();
    };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}",
        date.get_full_year(),
        date.get_month() + 1,
        date.get_date(),
        date.get_hours(),
        date.get_minutes()
    )
}

/// What a `datetime-local` input holds, as the wire's RFC 3339 UTC — whole
/// seconds, `Z`. The input's value is local time with no zone, which is
/// exactly the form `Date` reads as local. None for an empty or broken one.
pub fn from_local_input(value: &str) -> Option<String> {
    if value.trim().is_empty() {
        return None;
    }
    let date = Date::new(&JsValue::from_str(value));
    if date.get_time().is_nan() {
        return None;
    }
    let iso: String = date.to_iso_string().into();
    // "2026-09-12T09:00:00.000Z" → "2026-09-12T09:00:00Z".
    Some(match iso.split_once('.') {
        Some((whole, _)) => format!("{whole}Z"),
        None => iso,
    })
}

/// The next round hour after `now_ms`, as the wire writes an instant: a
/// family event is PLANNED, so a new one starts on something somebody might
/// actually have meant rather than on the second the button was pressed.
pub fn next_round_hour(now_ms: f64) -> String {
    let date = Date::new(&JsValue::from_f64(now_ms + 3_600_000.0));
    date.set_minutes(0);
    date.set_seconds(0);
    date.set_milliseconds(0);
    let iso: String = date.to_iso_string().into();
    match iso.split_once('.') {
        Some((whole, _)) => format!("{whole}Z"),
        None => iso,
    }
}

/// A wire timestamp moved by `ms`, as the wire writes it.
pub fn shifted(rfc3339: &str, ms: f64) -> Option<String> {
    let at = instant(rfc3339)?;
    let iso: String = Date::new(&JsValue::from_f64(at + ms))
        .to_iso_string()
        .into();
    Some(match iso.split_once('.') {
        Some((whole, _)) => format!("{whole}Z"),
        None => iso,
    })
}

/// Milliseconds since the epoch for a wire timestamp.
pub fn instant(rfc3339: &str) -> Option<f64> {
    date_of(rfc3339).map(|date| date.get_time())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    fn local_iso(year: u32, month: i32, day: i32, hour: i32, minute: i32) -> String {
        let date = Date::new_with_year_month_day_hr_min(year, month, day, hour, minute);
        date.to_iso_string().into()
    }

    #[wasm_bindgen_test]
    fn a_timestamp_is_read_in_the_readers_own_zone() {
        // Built as a LOCAL time and sent through the wire's UTC form: the
        // day that comes back is the local one, whatever this machine's zone.
        let wire = local_iso(2026, 7, 19, 23, 30);
        assert_eq!(
            day(&wire),
            Some(Day {
                year: 2026,
                month: 7,
                day: 19
            })
        );
        assert!(clock(&wire).contains("30"), "{}", clock(&wire));
    }

    #[wasm_bindgen_test]
    fn an_unreadable_timestamp_is_nothing_rather_than_a_panic() {
        assert_eq!(day(""), None);
        assert_eq!(day("not a date"), None);
        assert_eq!(clock(""), "");
        assert_eq!(row_time("", 0.0), "");
    }

    #[wasm_bindgen_test]
    fn today_and_yesterday_are_words_and_older_days_are_dates() {
        let now = Date::new_with_year_month_day_hr_min(2026, 8, 10, 12, 0).get_time();
        let today = day(&local_iso(2026, 8, 10, 0, 5)).expect("a day");
        let yesterday = day(&local_iso(2026, 8, 9, 23, 55)).expect("a day");
        let older = day(&local_iso(2026, 7, 17, 9, 0)).expect("a day");
        assert_eq!(day_label(today, now), "Today");
        assert_eq!(day_label(yesterday, now), "Yesterday");
        let label = day_label(older, now);
        assert!(label != "Today" && label != "Yesterday" && !label.is_empty());
        assert!(!label.contains("2026"), "no year on a day pill: {label}");
    }

    /// The input's local time goes out as the same instant in UTC, and
    /// comes back to the input unchanged — whatever this machine's zone.
    #[wasm_bindgen_test]
    fn a_local_input_round_trips_through_the_wire() {
        let wire = from_local_input("2026-09-12T11:00").expect("an instant");
        assert!(wire.ends_with('Z') && !wire.contains('.'), "{wire}");
        assert_eq!(local_input(&wire), "2026-09-12T11:00");
        assert_eq!(
            instant(&wire),
            Some(Date::new_with_year_month_day_hr_min(2026, 8, 12, 11, 0).get_time())
        );
        assert_eq!(from_local_input(""), None);
        assert_eq!(from_local_input("tomorrow"), None);
        assert_eq!(local_input("nonsense"), "");
    }

    #[wasm_bindgen_test]
    fn a_new_event_starts_on_the_next_round_hour() {
        let now = Date::new_with_year_month_day_hr_min(2026, 8, 10, 14, 37).get_time();
        let next = next_round_hour(now);
        assert_eq!(local_input(&next), "2026-09-10T15:00");
        let late = Date::new_with_year_month_day_hr_min(2026, 8, 10, 23, 5).get_time();
        assert_eq!(local_input(&next_round_hour(late)), "2026-09-11T00:00");
    }

    /// The same day writes the end as a time; another day, as a date too;
    /// and no end, no dash.
    #[wasm_bindgen_test]
    fn an_event_says_when_in_one_line() {
        let starts = local_iso(2026, 8, 12, 11, 0);
        let same_day = local_iso(2026, 8, 12, 13, 30);
        let next_day = local_iso(2026, 8, 13, 10, 0);
        let alone = event_when(&starts, None);
        assert!(!alone.contains('–'), "{alone}");
        assert!(alone.contains(&clock(&starts)), "{alone}");
        let short = event_when(&starts, Some(&same_day));
        assert!(short.ends_with(&clock(&same_day)), "{short}");
        assert!(short.contains(" – "), "{short}");
        let long = event_when(&starts, Some(&next_day));
        assert!(long.len() > short.len(), "{long} vs {short}");
        assert!(long.contains("13"), "the end's own day: {long}");
        assert_eq!(event_when("", None), "");
    }

    #[wasm_bindgen_test]
    fn an_event_is_past_once_its_end_is() {
        let now = Date::new_with_year_month_day_hr_min(2026, 8, 12, 12, 0).get_time();
        let started = local_iso(2026, 8, 12, 11, 0);
        let later = local_iso(2026, 8, 12, 13, 0);
        assert!(is_past(&started, None, now), "no end: its start");
        assert!(!is_past(&started, Some(&later), now), "still going");
        assert!(!is_past(&later, None, now));
        assert!(!is_past("", None, now), "unreadable is not past");
    }

    /// The first of the month's yesterday is the last day of the month
    /// before — by the calendar, not by subtracting a day's milliseconds.
    #[wasm_bindgen_test]
    fn yesterday_crosses_a_month_boundary() {
        let now = Date::new_with_year_month_day_hr_min(2026, 2, 1, 9, 0).get_time();
        let last_of_february = day(&local_iso(2026, 1, 28, 21, 0)).expect("a day");
        assert_eq!(day_label(last_of_february, now), "Yesterday");
    }
}
