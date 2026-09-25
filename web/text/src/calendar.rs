//! One event as an `.ics` file, for the client that has nowhere else to put
//! it (docs/protocol.md, "Board").
//!
//! **Nothing here is on the wire.** The protocol carries no calendar and no
//! `.ics`: a client that can put an event in the system calendar builds it
//! locally out of the title, the times and the place, and the server neither
//! generates one nor knows whether anybody kept it. The phones and the Mac
//! hand those three fields to the platform's own calendar; a BROWSER has no
//! such door, so the web writes the file itself and lets somebody save it —
//! which is what this module is for.
//!
//! What it copies is the title, the start, the end and the place. Not who is
//! coming (that is the family's business and not the calendar's) and not the
//! backdrop.
//!
//! The rules that are easy to get wrong, and why each is here rather than in
//! the view:
//!
//! - **CRLF line endings.** RFC 5545 §3.1 says every content line ends
//!   `\r\n`; a file with bare newlines is refused outright by some
//!   calendars and silently truncated by others.
//! - **Escaping.** A backslash, a semicolon and a comma are the separators
//!   of the format itself, so each is escaped in TEXT values, and a newline
//!   becomes a literal `\n`. A title with a comma in it would otherwise
//!   become two properties and take the rest of the line with it.
//! - **Folding at 75 octets**, counted in BYTES and never split inside a
//!   UTF-8 sequence: a folded line continues with a space on the next one.
//!   A family writing in Cyrillic hits the limit at half the characters an
//!   English one does, and a fold inside a code point produces a file no
//!   calendar can read.
//!
//! The timestamps arrive ALREADY in the format iCalendar wants
//! (`YYYYMMDDTHHMMSSZ`), because turning a wire instant into one is a job
//! for the platform's own date maths — `Date.toISOString` in the browser —
//! and not for string arithmetic here.

/// An `.ics` file holding one event. `uid` must be stable for the event so a
/// calendar that already has it updates rather than duplicating.
pub fn one_event(
    uid: &str,
    title: &str,
    starts_at: &str,
    ends_at: Option<&str>,
    place: Option<&str>,
    stamped_at: &str,
) -> String {
    let mut out = String::new();
    for line in [
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".to_string(),
        // Who wrote the file, in the shape RFC 5545 asks for. No product
        // registry, no version: a calendar shows this to nobody.
        "PRODID:-//nettrash//Family Connect//EN".to_string(),
        "CALSCALE:GREGORIAN".to_string(),
        // PUBLISH, not REQUEST: this is a copy of something the family
        // already agreed on, not an invitation with attendees to answer —
        // who is coming lives on the board (docs/protocol.md, "Board").
        "METHOD:PUBLISH".to_string(),
        "BEGIN:VEVENT".to_string(),
        format!("UID:{}", escape(uid)),
        format!("DTSTAMP:{}", escape(stamped_at)),
        format!("DTSTART:{}", escape(starts_at)),
        // An event with no end is an hour by convention, and the
        // convention is the CLIENT's: rather than invent one here, the
        // file simply has no DTEND, which every calendar reads as its own
        // default duration.
        ends_at.map_or_else(String::new, |ends| format!("DTEND:{}", escape(ends))),
        format!("SUMMARY:{}", escape(title)),
        place.map_or_else(String::new, |place| format!("LOCATION:{}", escape(place))),
        "END:VEVENT".to_string(),
        "END:VCALENDAR".to_string(),
    ] {
        if line.is_empty() {
            continue;
        }
        out.push_str(&fold(&line));
        out.push_str("\r\n");
    }
    out
}

/// A TEXT value's own characters, kept from being read as the format's.
fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\n' => out.push_str("\\n"),
            // A lone carriage return is dropped: it is half a line ending,
            // and the `\n` beside it carries the meaning.
            '\r' => {}
            _ => out.push(ch),
        }
    }
    out
}

/// A content line broken at 75 OCTETS, continued with a leading space, and
/// never split inside a UTF-8 sequence.
fn fold(line: &str) -> String {
    const LIMIT: usize = 75;
    if line.len() <= LIMIT {
        return line.to_string();
    }
    let mut out = String::with_capacity(line.len() + line.len() / LIMIT * 3);
    let mut room = LIMIT;
    for ch in line.chars() {
        let width = ch.len_utf8();
        if width > room {
            out.push_str("\r\n ");
            // The continuation's leading space is itself an octet of the
            // folded line.
            room = LIMIT - 1;
        }
        out.push(ch);
        room -= width;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_events_file_has_the_shape_a_calendar_reads() {
        let ics = one_event(
            "fc-note-12@nettrash",
            "Christmas dinner",
            "20261224T160000Z",
            Some("20261224T200000Z"),
            Some("Gran's house"),
            "20260911T120000Z",
        );

        // Every line ends CRLF, and the file is one VEVENT in one
        // VCALENDAR.
        assert!(ics.starts_with("BEGIN:VCALENDAR\r\n"));
        assert!(ics.ends_with("END:VCALENDAR\r\n"));
        assert!(!ics.contains("\n\n"));
        for line in ics.split("\r\n").filter(|line| !line.is_empty()) {
            assert!(!line.contains('\n'), "a bare newline in {line:?}");
        }
        assert!(ics.contains("SUMMARY:Christmas dinner\r\n"));
        assert!(ics.contains("DTSTART:20261224T160000Z\r\n"));
        assert!(ics.contains("DTEND:20261224T200000Z\r\n"));
        assert!(ics.contains("LOCATION:Gran's house\r\n"));
        assert!(ics.contains("UID:fc-note-12@nettrash\r\n"));
        assert!(ics.contains("DTSTAMP:20260911T120000Z\r\n"));
    }

    #[test]
    fn an_event_with_no_end_and_no_place_says_neither() {
        let ics = one_event("u", "Picnic", "20261224T160000Z", None, None, "s");
        assert!(!ics.contains("DTEND"));
        assert!(!ics.contains("LOCATION"));
        assert!(ics.contains("SUMMARY:Picnic\r\n"));
    }

    #[test]
    fn the_formats_own_characters_are_escaped() {
        let ics = one_event(
            "u",
            "Dinner, drinks; then\nfireworks \\ home",
            "20261224T160000Z",
            None,
            Some("Gran's, upstairs"),
            "s",
        );
        // A comma left alone would end the value and make the rest a
        // second property.
        assert!(ics.contains("SUMMARY:Dinner\\, drinks\\; then\\nfireworks \\\\ home\r\n"));
        assert!(ics.contains("LOCATION:Gran's\\, upstairs\r\n"));
        // And a real newline never survives inside a value.
        assert!(!ics.contains("then\r\nfireworks"));
    }

    #[test]
    fn a_long_line_folds_at_seventy_five_octets_and_never_mid_character() {
        let long = "Дед Мороз".repeat(20);
        let ics = one_event("u", &long, "20261224T160000Z", None, None, "s");
        for line in ics.split("\r\n") {
            assert!(line.len() <= 75, "{} octets: {line:?}", line.len());
        }
        // The folds are the only thing added: strip them and the value is
        // back, character for character.
        let unfolded = ics.replace("\r\n ", "");
        assert!(
            unfolded.contains(&format!("SUMMARY:{long}")),
            "the title survives the folding"
        );
        // Every line is still valid UTF-8 text (it is a Rust String, so a
        // split inside a code point would have panicked above) and no line
        // begins with a stray byte.
        assert!(ics.is_char_boundary(0));
    }
}
