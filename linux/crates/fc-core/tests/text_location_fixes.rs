//! `LocationFixesTests`: the bar a shared place clears — web `location.rs`'s own tests, and the
//! edges they leave out. (`APlacesLabelRidesEscapedAfterItsNumbers` is `UploadQueries.Location`,
//! the `protocol` port's.)

use std::time::Duration;

use fc_core::text::location_fixes::{
    FRESH_ENOUGH, FixVerdict, GOOD_ENOUGH_METRES, LocationFix, LocationHunt, TIMEOUT, better,
    judge, usable_accuracy,
};
use time::OffsetDateTime;
use time::macros::datetime;

const NOW: OffsetDateTime = datetime!(2026-09-14 12:00:00 UTC);

fn fix(accuracy: Option<f64>, age_ms: i64) -> LocationFix {
    LocationFix {
        latitude: 55.75,
        longitude: 37.61,
        accuracy_m: accuracy,
        taken: NOW - time::Duration::milliseconds(age_ms),
    }
}

#[test]
fn a_fix_is_sent_only_while_it_is_fresh() {
    assert_eq!(judge(&fix(Some(12.0), 1_000), NOW), FixVerdict::Accept);
    assert_eq!(judge(&fix(Some(100.0), 0), NOW), FixVerdict::Accept);
    // Coarse is held, not refused — and so is a fix that does not know how good it is.
    assert_eq!(judge(&fix(Some(101.0), 0), NOW), FixVerdict::Hold);
    assert_eq!(judge(&fix(None, 0), NOW), FixVerdict::Hold);
    // Two minutes to the millisecond is still fresh; a millisecond more is not, however precise.
    assert_eq!(judge(&fix(Some(5.0), 120_000), NOW), FixVerdict::Accept);
    assert_eq!(judge(&fix(Some(5.0), 120_001), NOW), FixVerdict::Stale);
    assert_eq!(judge(&fix(None, 120_001), NOW), FixVerdict::Stale);
    assert_eq!(judge(&fix(Some(500.0), 3_600_000), NOW), FixVerdict::Stale);
}

#[test]
fn the_best_fresh_fix_is_the_smallest_circle() {
    let coarse = fix(Some(900.0), 0);
    let finer = fix(Some(300.0), 0);
    let unknown = fix(None, 0);
    assert_eq!(better(None, coarse), coarse);
    assert_eq!(better(Some(coarse), finer), finer);
    assert_eq!(better(Some(finer), coarse), finer);
    // Known beats unknown, both ways round.
    assert_eq!(better(Some(unknown), coarse), coarse);
    assert_eq!(better(Some(coarse), unknown), coarse);
    // A tie keeps what was held.
    let same_circle = LocationFix {
        latitude: 1.0,
        ..coarse
    };
    assert_eq!(better(Some(coarse), same_circle), coarse);
    assert_eq!(
        better(
            Some(unknown),
            LocationFix {
                latitude: 1.0,
                ..unknown
            }
        ),
        unknown
    );
}

#[test]
fn an_accuracy_is_known_only_when_it_is_a_measurement() {
    assert_eq!(usable_accuracy(12.5), Some(12.5));
    assert_eq!(usable_accuracy(0.0), Some(0.0));
    assert_eq!(usable_accuracy(-1.0), None);
    assert_eq!(usable_accuracy(f64::NAN), None);
    assert_eq!(usable_accuracy(f64::INFINITY), None);
}

/// A good fix ends the wait; a coarse fresh one is held for the deadline; a stale one is nothing.
#[test]
fn a_hunt_ends_on_a_good_fix_and_holds_the_best_coarse_one() {
    let hunt = LocationHunt::new();
    assert_eq!(hunt.offer(fix(Some(5.0), 600_000), NOW), None);
    assert_eq!(hunt.best(), None);

    assert_eq!(hunt.offer(fix(Some(900.0), 0), NOW), None);
    assert_eq!(hunt.best().and_then(|best| best.accuracy_m), Some(900.0));
    assert_eq!(hunt.offer(fix(Some(300.0), 0), NOW), None);
    assert_eq!(hunt.offer(fix(Some(900.0), 0), NOW), None);
    assert_eq!(hunt.best().and_then(|best| best.accuracy_m), Some(300.0));

    let good = fix(Some(40.0), 0);
    assert_eq!(hunt.offer(good, NOW), Some(good));

    assert_eq!(GOOD_ENOUGH_METRES, 100.0);
    assert_eq!(FRESH_ENOUGH, Duration::from_secs(120));
    assert_eq!(TIMEOUT, Duration::from_secs(20));
}
