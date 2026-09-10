//! Time as the reader knows it: their clock, their time zone, their
//! language's way of writing a date.
//!
//! The wire carries RFC 3339 UTC. Everything here goes through the browser's
//! own `Date` and `Intl`, which already know the reader's zone and locale —
//! the same thing the apps get from the operating system, and the reason
//! this client carries no date library of its own.

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
        return "Today".to_string();
    }
    if day == previous(today) {
        return "Yesterday".to_string();
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
        return "Yesterday".to_string();
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

    /// The first of the month's yesterday is the last day of the month
    /// before — by the calendar, not by subtracting a day's milliseconds.
    #[wasm_bindgen_test]
    fn yesterday_crosses_a_month_boundary() {
        let now = Date::new_with_year_month_day_hr_min(2026, 2, 1, 9, 0).get_time();
        let last_of_february = day(&local_iso(2026, 1, 28, 21, 0)).expect("a day");
        assert_eq!(day_label(last_of_february, now), "Yesterday");
    }
}
