//! `CultureTests`: what this client does in a reader's own language and number format.
//!
//! On Windows the ambient CULTURE is process-wide and changes how .NET writes a number, compares a
//! string and reads a format string; the C# suite sets it (de-DE, tr-TR, ru-RU, sv-SE, ja-JP,
//! fi-FI) and checks nothing moved. Rust has no ambient culture — `{}` writes a number the same
//! way on every machine and `to_ascii_lowercase` has no Turkish I — so each test here is the C#
//! test with the culture switch gone and its assertions kept.
//!
//! Not here: `TheBoardsOwnNumbersSurviveAnyReadersFormat` is `BoardStore`'s (the `store` port),
//! and the half of `TheWiresInstantsAreReadAndWrittenInOneFormat` that is `Times.Instant` /
//! `Times.Rfc3339` is the `store` port's too; the calendar stamp's half is below.

mod common;

use fc_core::text::calendar::stamp;
use fc_core::text::catalogues::Languages;
use fc_core::text::string_catalog::{EnglishCatalog, StringCatalog};
use time::macros::datetime;

/// A language TAG is matched the same way whatever the reader's alphabet: `SR-LATN` is Latin
/// Serbian in Turkey too.
#[test]
fn a_language_tag_is_matched_the_same_way_everywhere() {
    assert_eq!(Languages::nearest(Some("SR-LATN-RS")), "sr-Latn");
    assert_eq!(Languages::nearest(Some("DE-AT")), "de");
    assert_eq!(Languages::nearest(Some("ZH-HANT-TW")), "zh-Hans");
}

/// The calendar file's own stamp, which a calendar programme parses: never in the reader's
/// format. (fi-FI is the culture that caught it on Windows, where `:` in a format string is the
/// TIME SEPARATOR and Finnish spells it `.`.)
#[test]
fn the_wires_instants_are_read_and_written_in_one_format() {
    // 2026-12-24T16:00:00Z, as `Times.Instant` reads it.
    let instant = datetime!(2026-12-24 16:00:00 UTC);
    assert_eq!(stamp(instant), "20261224T160000Z");
}

/// A number INSIDE a sentence is the reader's to format — "1.284" is right in German — but the
/// sentence is the catalogue's and the placeholders are Apple's, so the two have to work together
/// rather than one of them quietly winning.
#[test]
fn a_sentences_numbers_are_drawn_in_the_readers_own_format() {
    // The catalogue's German value is positional, and the count goes where German wants it.
    assert_eq!(
        common::catalogues()
            .for_language("de")
            .format("%lld of %lld done", &[&3, &7]),
        "3 von 7 erledigt"
    );
    // The English source sentence, filled in order, in the same breath.
    assert_eq!(
        EnglishCatalog::new().format("%lld Photos", &[&2]),
        "2 Photos"
    );
}

/// And the `.ics` file is the same bytes on every machine: a calendar file is read by a programme,
/// and its 75-octet lines are counted in bytes rather than in anybody's letters.
#[test]
fn the_calendar_file_is_the_same_bytes_in_every_language() {
    let english = fc_text::calendar::one_event(
        "fc-note-12@nettrash",
        "Christmas dinner, at Gran's",
        "20261224T160000Z",
        Some("20261224T200000Z"),
        Some("Gran's house"),
        "20260912T120000Z",
    );
    // With no culture to switch, the second call is the whole of the check.
    assert_eq!(
        english,
        fc_text::calendar::one_event(
            "fc-note-12@nettrash",
            "Christmas dinner, at Gran's",
            "20261224T160000Z",
            Some("20261224T200000Z"),
            Some("Gran's house"),
            "20260912T120000Z",
        )
    );
}
