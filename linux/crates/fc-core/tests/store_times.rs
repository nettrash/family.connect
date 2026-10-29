//! `store::times` against what the C# `Times` answers — not a port of a C# test (`win/` has none for
//! `Times.cs`), but the helper `linux/docs/port-plan.md` asks for wherever the C# formats or parses a
//! time. Every expected value below is what `Times.Instant` / `Times.Rfc3339` answered on .NET 10
//! (the C# files built in the session scratchpad, 2026-10-28).

use fc_core::store::times::{from_milliseconds, instant, milliseconds, rfc3339};
use time::macros::datetime;

/// 2026-09-12T10:00:00Z.
const TEN_O_CLOCK: i64 = 1_789_207_200_000;

/// Each stamp and what .NET read it as.
#[track_caller]
fn reads(cases: &[(&str, Option<i64>)]) {
    for &(stamp, expected) in cases {
        assert_eq!(instant(Some(stamp)), expected, "{stamp:?}");
    }
}

#[test]
fn what_the_server_writes_is_read_to_the_millisecond_rounded_down() {
    reads(&[
        ("2026-09-12T10:00:00Z", Some(TEN_O_CLOCK)),
        ("2026-09-12T10:00:00.123Z", Some(TEN_O_CLOCK + 123)),
        ("2026-09-12T10:00:00.1239Z", Some(TEN_O_CLOCK + 123)),
        ("2026-09-12T10:00:00.123456789Z", Some(TEN_O_CLOCK + 123)),
        ("2026-09-12T10:00:00.1234567Z", Some(TEN_O_CLOCK + 123)),
        ("2026-09-12T10:00:00.9999999Z", Some(TEN_O_CLOCK + 999)),
        // .NET keeps seven digits, and an eighth of 5 or more rounds the seventh up.
        ("2026-09-12T10:00:00.99999999Z", Some(TEN_O_CLOCK + 1000)),
        ("2026-09-12T10:00:00+02:00", Some(TEN_O_CLOCK - 7_200_000)),
        ("2026-09-12T10:00:00-00:00", Some(TEN_O_CLOCK)),
        (
            "2026-09-12T10:00:00.5+01:00",
            Some(TEN_O_CLOCK - 3_600_000 + 500),
        ),
        // Before 1970 the millisecond is still the one at or before the instant.
        ("1969-12-31T23:59:59.9995Z", Some(-1)),
    ]);
}

#[test]
fn the_spellings_dotnet_also_reads_are_read() {
    reads(&[
        ("2026-09-12t10:00:00z", Some(TEN_O_CLOCK)),
        ("2026-09-12 10:00:00Z", Some(TEN_O_CLOCK)),
        // No zone is UTC (`AssumeUniversal`).
        ("2026-09-12T10:00:00", Some(TEN_O_CLOCK)),
        ("2026-09-12T10:00:00.5", Some(TEN_O_CLOCK + 500)),
        ("2026-09-12T10:00", Some(TEN_O_CLOCK)),
        ("2026-09-12", Some(1_789_171_200_000)),
        ("2026-09-12Z", Some(1_789_171_200_000)),
        ("2026-09-12 Z", Some(1_789_171_200_000)),
        (" 2026-09-12T10:00:00Z ", Some(TEN_O_CLOCK)),
        ("\t2026-09-12T10:00:00Z", Some(TEN_O_CLOCK)),
        ("2026-09-12T10:00:00 Z", Some(TEN_O_CLOCK)),
        ("2026-09-12T10:00:00 +02:00", Some(TEN_O_CLOCK - 7_200_000)),
        ("2026-09-12T10:00:00,5Z", Some(TEN_O_CLOCK + 500)),
        ("2026-09-12T10:00:00+0200", Some(TEN_O_CLOCK - 7_200_000)),
        ("2026-09-12T10:00:00+02", Some(TEN_O_CLOCK - 7_200_000)),
        ("2026-09-12T10:00:00+14:00", Some(TEN_O_CLOCK - 50_400_000)),
    ]);
}

#[test]
fn what_dotnet_refuses_is_refused() {
    reads(&[
        ("", None),
        ("garbage", None),
        ("2026-02-30T00:00:00Z", None),
        // A leap second: .NET has none (off Windows), and neither has this.
        ("2016-12-31T23:59:60Z", None),
        ("2026-09-12T25:00:00Z", None),
        ("2026-09-12T24:00:00Z", None),
        ("+2026-09-12T10:00:00Z", None),
        ("2026-09-12T10:00:00.Z", None),
        ("20260912T100000Z", None),
        ("2026-09-12T1:2:3Z", None),
        ("2026-09-12T10Z", None),
        ("2026-09-12TZ", None),
        ("2026-09-12T10:00:00ZZ", None),
        ("2026-09-12T10:00:00+15:00", None),
        ("2026-09-12T10:00:00+14:01", None),
    ]);
    assert_eq!(instant(None), None);
}

#[test]
fn the_range_is_dotnets() {
    reads(&[
        ("0001-01-01T00:00:00Z", Some(-62_135_596_800_000)),
        ("0001-01-01T00:00:00+01:00", None),
        ("9999-12-31T23:59:59.999Z", Some(253_402_300_799_999)),
        ("9999-12-31T23:59:59-01:00", None),
        ("0000-06-01T00:00:00Z", None),
    ]);
}

/// Where this port and .NET part: spellings `DateTimeOffset.TryParse` reads and no server writes.
/// Pinned so that reading any of them is a decision, not an accident.
#[test]
fn the_spellings_no_server_writes_are_not_read() {
    reads(&[
        // .NET: 1789207200000.
        ("Sat, 12 Sep 2026 10:00:00 GMT", None),
        // .NET: 1789171200000, by the invariant culture's month-first order.
        ("9/12/2026", None),
        // .NET: 1788343200000 and 1789207200000.
        ("2026-9-2T10:00:00Z", None),
        ("02026-09-12T10:00:00Z", None),
        // .NET: 1789200000000.
        ("2026-09-12T10:00:00+2:00", None),
    ]);
}

#[test]
fn back_to_the_wire_is_utc_to_the_millisecond() {
    for (milliseconds, expected) in [
        (None, None),
        // 0 is this cache's spelling of absent, not 1970.
        (Some(0), None),
        (Some(1), Some("1970-01-01T00:00:00.001Z")),
        (Some(-1), Some("1969-12-31T23:59:59.999Z")),
        (Some(1_757_671_200_000), Some("2025-09-12T10:00:00.000Z")),
        (Some(1_757_671_200_123), Some("2025-09-12T10:00:00.123Z")),
        (Some(-62_135_596_800_000), Some("0001-01-01T00:00:00.000Z")),
        (Some(253_402_300_799_999), Some("9999-12-31T23:59:59.999Z")),
        // Outside .NET's range the C# throws; a damaged cache reads as no stamp here.
        (Some(-62_135_596_800_001), None),
        (Some(253_402_300_800_000), None),
    ] {
        assert_eq!(
            rfc3339(milliseconds).as_deref(),
            expected,
            "{milliseconds:?}"
        );
    }
}

#[test]
fn an_instant_and_its_milliseconds_round_trip() {
    let at = datetime!(2026-09-12 10:00:00.123 UTC);
    assert_eq!(milliseconds(at), TEN_O_CLOCK + 123);
    assert_eq!(from_milliseconds(TEN_O_CLOCK + 123), Some(at));
    // Rounded down, before 1970 too: `ToUnixTimeMilliseconds`.
    assert_eq!(milliseconds(datetime!(1969-12-31 23:59:59.9995 UTC)), -1);
}
