//! "Add to Calendar" (§11): an event as one `.ics` file — `fc_text::calendar::one_event`
//! (byte-exact, CRLF, folded at 75 octets, never inside a code point), stamped with
//! `fc_core::text::calendar::stamp` — written into the app's temporary folder under the event's
//! title and handed to `gtk::FileLauncher`, which opens the desktop's calendar on it (GNOME
//! Calendar's import). Nothing here is on the wire. Pure but for [`write`].

use std::path::{Path, PathBuf};

use fc_core::protocol::NoteDto;
use fc_core::text::calendar::stamp;
use fc_logic::event_text;
use fc_logic::wall_text;
use time::OffsetDateTime;

/// The event's file, or `None` for a note that is not an event with a readable start. The uid is
/// the other clients' (`fc-note-<id>@family.connect`), stable, so a calendar that already holds
/// the event updates it rather than keeping two.
pub fn ics(note: &NoteDto, now: OffsetDateTime) -> Option<String> {
    let starts = event_text::instant(note.starts_at.as_deref())?;
    let ends = event_text::instant(note.ends_at.as_deref()).map(stamp);
    let place = note
        .place
        .as_deref()
        .map(str::trim)
        .filter(|place| !place.is_empty());
    Some(fc_text::calendar::one_event(
        &format!("fc-note-{}@family.connect", note.id),
        &wall_text::caption_of(note),
        &stamp(starts),
        ends.as_deref(),
        place,
        &stamp(now),
    ))
}

/// A title as a file name (the web client's `file_stem`, which the Mac's `fileStem` is): the
/// letters, digits, spaces, dashes and underscores it has, runs of anything else one space —
/// and "event" when nothing is left.
pub fn file_stem(title: &str) -> String {
    let kept: String = title
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || ch == ' ' || ch == '-' || ch == '_' {
                ch
            } else {
                ' '
            }
        })
        .collect();
    let trimmed = kept.split_whitespace().collect::<Vec<_>>().join(" ");
    if trimmed.is_empty() {
        "event".to_string()
    } else {
        trimmed
    }
}

/// Writes the event's file into `folder` (created when missing) as `<title>.ics`. Answers its path.
pub fn write(folder: &Path, note: &NoteDto, now: OffsetDateTime) -> Option<PathBuf> {
    let text = ics(note, now)?;
    std::fs::create_dir_all(folder).ok()?;
    let path = folder.join(format!("{}.ics", file_stem(&wall_text::caption_of(note))));
    std::fs::write(&path, text.as_bytes()).ok()?;
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn event() -> NoteDto {
        NoteDto {
            kind: Some("event".into()),
            text: Some("  Dinner at Gran's, then cake  ".into()),
            starts_at: Some("2026-12-24T16:00:00Z".into()),
            ends_at: Some("2026-12-24T20:30:00+01:00".into()),
            place: Some(" Gran's ".into()),
            ..NoteDto::new(42, 7)
        }
    }

    #[test]
    fn an_event_becomes_one_crlf_calendar_file() {
        let text = ics(&event(), datetime!(2026-12-01 09:05:03 UTC)).unwrap();
        assert!(text.starts_with("BEGIN:VCALENDAR\r\n"));
        assert!(text.contains("UID:fc-note-42@family.connect\r\n"));
        assert!(text.contains("DTSTART:20261224T160000Z\r\n"));
        // The end's offset is turned into UTC.
        assert!(text.contains("DTEND:20261224T193000Z\r\n"));
        assert!(text.contains("DTSTAMP:20261201T090503Z\r\n"));
        // Trimmed, and the format's own separators escaped.
        assert!(text.contains("SUMMARY:Dinner at Gran's\\, then cake\r\n"));
        assert!(text.contains("LOCATION:Gran's\r\n"));
        assert!(text.ends_with("END:VCALENDAR\r\n"));
        assert!(!text.replace("\r\n", "").contains('\n'));
    }

    #[test]
    fn no_end_and_no_place_are_left_out_and_a_note_without_a_start_has_no_file() {
        let note = NoteDto {
            ends_at: None,
            place: Some("   ".into()),
            ..event()
        };
        let text = ics(&note, datetime!(2026-12-01 00:00 UTC)).unwrap();
        assert!(!text.contains("DTEND"));
        assert!(!text.contains("LOCATION"));
        let plain = NoteDto {
            starts_at: None,
            ..event()
        };
        assert_eq!(ics(&plain, datetime!(2026-12-01 00:00 UTC)), None);
        let unreadable = NoteDto {
            starts_at: Some("tomorrow".into()),
            ..event()
        };
        assert_eq!(ics(&unreadable, datetime!(2026-12-01 00:00 UTC)), None);
    }

    #[test]
    fn a_title_becomes_a_file_name() {
        assert_eq!(
            file_stem("Dinner at Gran's, then cake"),
            "Dinner at Gran s then cake"
        );
        assert_eq!(file_stem("Ёлка / 2026"), "Ёлка 2026");
        assert_eq!(file_stem("?!"), "event");
        assert_eq!(file_stem(""), "event");
    }

    #[test]
    fn the_file_is_written_under_its_title() {
        let folder = std::env::temp_dir().join(format!("fc-ics-{}", std::process::id()));
        let path = write(&folder, &event(), datetime!(2026-12-01 00:00 UTC)).unwrap();
        assert_eq!(
            path.file_name().and_then(|n| n.to_str()),
            Some("Dinner at Gran s then cake.ics")
        );
        let read = std::fs::read_to_string(&path).unwrap();
        assert!(read.contains("DTSTART:20261224T160000Z\r\n"));
        let _ = std::fs::remove_dir_all(&folder);
    }
}
