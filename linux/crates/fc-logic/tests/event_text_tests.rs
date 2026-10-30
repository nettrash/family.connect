//! `EventTextTests`: when an event is, in the reader's language and zone. The C#'s
//! `CultureInfo.GetCultureInfo("en-GB")` is `DateCulture::named("en-GB")` (the table measured from
//! that very culture), and its zones — UTC and a custom +3 — are fixed offsets.

use fc_logic::date_culture::DateCulture;
use fc_logic::event_text::{clock, date_block, instant, is_past, when};
use time::UtcOffset;
use time::macros::datetime;

fn british() -> DateCulture {
    DateCulture::named("en-GB").expect("en-GB is measured")
}

const UTC: UtcOffset = UtcOffset::UTC;

fn plus_three() -> UtcOffset {
    UtcOffset::from_hms(3, 0, 0).unwrap()
}

fn pair(day: &str, month: &str) -> Option<(String, String)> {
    Some((day.to_string(), month.to_string()))
}

#[test]
fn the_block_is_the_days_number_over_its_short_month_in_the_readers_zone() {
    assert_eq!(
        date_block("2026-12-24T21:30:00Z", &british(), &UTC),
        pair("24", "Dec")
    );
    // Half past nine on Christmas Eve in London is half past midnight on Christmas Day three hours
    // east.
    assert_eq!(
        date_block("2026-12-24T21:30:00Z", &british(), &plus_three()),
        pair("25", "Dec")
    );
    assert_eq!(date_block("not a time", &british(), &UTC), None);
    assert_eq!(
        date_block("2026-12-05T10:00:00Z", &british(), &UTC),
        pair("5", "Dec")
    );
}

#[test]
fn the_clock_says_the_end_and_the_ends_day_when_it_is_another_day() {
    assert_eq!(
        clock("2026-12-24T16:00:00Z", None, &british(), &UTC),
        "16:00"
    );
    assert_eq!(
        clock(
            "2026-12-24T16:00:00Z",
            Some("2026-12-24T20:00:00Z"),
            &british(),
            &UTC
        ),
        "16:00 – 20:00"
    );
    assert_eq!(
        clock(
            "2026-12-24T16:00:00Z",
            Some("2026-12-25T02:00:00Z"),
            &british(),
            &UTC
        ),
        "16:00 – 25 Dec 02:00"
    );
    // The same pair, three hours east, starts and ends on the same local day.
    assert_eq!(
        clock(
            "2026-12-24T22:00:00Z",
            Some("2026-12-24T23:30:00Z"),
            &british(),
            &plus_three()
        ),
        "01:00 – 02:30"
    );
    assert_eq!(
        clock("2026-12-24T16:00:00Z", Some("garbage"), &british(), &UTC),
        "16:00"
    );
    assert_eq!(
        clock("", Some("2026-12-24T20:00:00Z"), &british(), &UTC),
        ""
    );
    let american = DateCulture::named("en-US").expect("en-US is measured");
    assert!(clock("2026-12-24T16:00:00Z", None, &american, &UTC).contains("PM"));
}

#[test]
fn the_whole_of_it_has_the_weekday_too() {
    assert_eq!(
        when(
            "2026-12-24T16:00:00Z",
            Some("2026-12-24T20:00:00Z"),
            &british(),
            &UTC
        ),
        "Thu, 24 Dec, 16:00 – 20:00"
    );
    assert_eq!(when("never", None, &british(), &UTC), "");
}

#[test]
fn an_event_is_over_when_its_end_has_passed_or_with_no_end_its_start() {
    let now = datetime!(2026-12-24 18:00:00 UTC);
    assert!(is_past("2026-12-24T16:00:00Z", None, now));
    assert!(!is_past(
        "2026-12-24T16:00:00Z",
        Some("2026-12-24T20:00:00Z"),
        now
    ));
    assert!(is_past(
        "2026-12-24T12:00:00Z",
        Some("2026-12-24T17:00:00Z"),
        now
    ));
    assert!(!is_past("2026-12-25T10:00:00Z", None, now));
    assert!(!is_past("broken", None, now));
    // Ending this very minute is not over yet.
    assert!(!is_past(
        "2026-12-24T16:00:00Z",
        Some("2026-12-24T18:00:00Z"),
        now
    ));
    assert_eq!(
        instant(Some("2026-12-24T18:00:00+02:00")),
        Some(datetime!(2026-12-24 16:00:00 UTC))
    );
}

/// Not in the C# suite: what `DateTimeOffset.TryParse` (invariant, a missing offset as UTC) was
/// measured to answer on .NET 10 for the shapes an instant takes — the port answers the same.
#[test]
fn an_instant_is_read_as_dotnet_reads_the_wires_shapes() {
    let utc = |text: &str| instant(Some(text));
    for (text, expected) in [
        (
            "2026-12-24T16:00:00Z",
            Some(datetime!(2026-12-24 16:00:00 UTC)),
        ),
        (
            "2026-12-24t16:00:00z",
            Some(datetime!(2026-12-24 16:00:00 UTC)),
        ),
        (
            "2026-12-24 16:00:00Z",
            Some(datetime!(2026-12-24 16:00:00 UTC)),
        ),
        (
            "2026-12-24T16:00:00",
            Some(datetime!(2026-12-24 16:00:00 UTC)),
        ),
        ("2026-12-24", Some(datetime!(2026-12-24 00:00:00 UTC))),
        (
            "2026-12-24T16:00Z",
            Some(datetime!(2026-12-24 16:00:00 UTC)),
        ),
        (
            "2026-12-24T16:00:00+0200",
            Some(datetime!(2026-12-24 14:00:00 UTC)),
        ),
        (
            "2026-12-24T16:00:00+02",
            Some(datetime!(2026-12-24 14:00:00 UTC)),
        ),
        (
            " 2026-12-24T16:00:00Z ",
            Some(datetime!(2026-12-24 16:00:00 UTC)),
        ),
        (
            "2026-12-24T16:00:00-00:00",
            Some(datetime!(2026-12-24 16:00:00 UTC)),
        ),
        (
            "2026-12-24T16:00:00+14:00",
            Some(datetime!(2026-12-24 02:00:00 UTC)),
        ),
        ("2026-12-24T24:00:00Z", None),
        ("2026-12-24T23:59:60Z", None),
        ("2026-02-30T00:00:00Z", None),
        ("20261224T160000Z", None),
        ("2026-12-24T16Z", None),
        ("+002026-12-24T16:00:00Z", None),
        ("2026-12-24T16:00:00+15:00", None),
        ("not a time", None),
        ("", None),
    ] {
        assert_eq!(utc(text), expected, "{text:?}");
    }
    assert_eq!(instant(None), None);
    // A fraction is kept (to the nanosecond, where .NET keeps 100 ns ticks).
    assert_eq!(
        utc("2026-12-24T16:00:00.123456789Z"),
        Some(datetime!(2026-12-24 16:00:00.123456789 UTC))
    );
}
