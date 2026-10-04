//! Today's weather in the daily greeting (docs/protocol.md, "Today's
//! weather, for places the owner chose", #72): the places field the family's
//! owner fills in beside the greeting switch.
//!
//! Two keys decide whether it is drawn at all — the server's
//! `assistant.greeting_weather` (absent on a server that predates it, which
//! reads as false) and the caller being the owner ([`offered`]). What the
//! owner types is held to the server's own rules before it is sent, so a
//! PATCH this client makes is never refused for a name ([`typed`],
//! [`write`]):
//!
//! - a field keeps at most [`MAX_CHARS`] characters (characters, not bytes
//!   and not UTF-16 units, as the server counts) and no control character
//!   that is not whitespace — the server refuses one, and nobody means one;
//! - a field left empty is not a place: it is dropped, never sent as `""`
//!   (which the server refuses);
//! - each name is trimmed and its inner whitespace folded to one space, and
//!   a repeat of an earlier name once both are lower-cased is dropped,
//!   keeping the first spelling — exactly what the server keeps, so the list
//!   it answers with is the list this client sent;
//! - at most [`MAX_PLACES`], which the field enforces by offering no fourth
//!   row ([`can_add`]).
//!
//! The answer, not what was sent, is what is then shown: a server older
//! than this ignores the key, and its answer carries no list.

use crate::i18n::t;

/// The most places a family may name (`greeting_places`, protocol limits).
pub const MAX_PLACES: usize = 3;

/// The most characters one name may hold, counted after it is folded.
pub const MAX_CHARS: usize = 80;

/// Whether the places field is drawn: for the family's OWNER only — every
/// member may read the list, and nobody else may change it, so a member's
/// screen shows nothing, as it shows none of the owner's assistant
/// switches — and only where the server will use it. `greeting_weather` is
/// true only where `greetings_enabled` is, and both are asked so that a
/// server answering one without the other draws no field that does nothing.
pub fn offered(owner: bool, greetings_enabled: bool, greeting_weather: bool) -> bool {
    owner && greetings_enabled && greeting_weather
}

/// What a field keeps of what was typed or pasted into it: no control
/// character that is not whitespace (a tab is whitespace, and folds to a
/// space when it is sent), and at most [`MAX_CHARS`] characters — the
/// server's limit, applied while typing so a name is never refused for its
/// length. Counted before folding, so this is at most as generous as the
/// server.
pub fn typed(raw: &str) -> String {
    raw.chars()
        .filter(|c| !c.is_control() || c.is_whitespace())
        .take(MAX_CHARS)
        .collect()
}

/// One name as the server keeps it: trimmed, every run of whitespace inside
/// it one space.
pub fn fold(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Why a list cannot be sent as it stands. Positions count the rows as
/// drawn, from 1. With [`typed`] applied to every field none of these can
/// happen; they are the server's rules, checked once more on the way out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// A control character the server refuses.
    Control { position: usize },
    /// Over [`MAX_CHARS`] once folded.
    TooLong { position: usize },
    /// More than [`MAX_PLACES`] once repeats are dropped.
    TooMany,
}

/// The rows as `greeting_places` will carry them, by the server's rules:
/// blank rows dropped, names folded, repeats dropped (Unicode lower-casing,
/// first spelling kept), at most [`MAX_PLACES`].
pub fn places(rows: &[String]) -> Result<Vec<String>, Refusal> {
    let mut kept: Vec<String> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let position = index + 1;
        let name = fold(row);
        if name.is_empty() {
            continue;
        }
        if name.chars().any(char::is_control) {
            return Err(Refusal::Control { position });
        }
        if name.chars().count() > MAX_CHARS {
            return Err(Refusal::TooLong { position });
        }
        let lower = name.to_lowercase();
        if seen.contains(&lower) {
            continue;
        }
        seen.push(lower);
        kept.push(name);
    }
    if kept.len() > MAX_PLACES {
        return Err(Refusal::TooMany);
    }
    Ok(kept)
}

/// What committing the rows does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Write {
    /// Nothing: they say what the family already has (an empty row added,
    /// a name retyped with an extra space, a row emptied that was never
    /// saved).
    Nothing,
    /// `PATCH /families/mine {"greeting_places": [...]}` with this list.
    Send(Vec<String>),
    /// Not sendable as it stands.
    Refused(Refusal),
}

/// What committing `rows` does, given the list the family has (`saved`,
/// as the server last answered it).
pub fn write(rows: &[String], saved: &[String]) -> Write {
    match places(rows) {
        Err(refusal) => Write::Refused(refusal),
        Ok(list) if list == saved => Write::Nothing,
        Ok(list) => Write::Send(list),
    }
}

/// The rows drawn when nothing is being edited: the family's list, then
/// the empty rows the owner added and has not typed in yet — never more
/// than [`MAX_PLACES`] in all (a list longer than that, from a server
/// breaking its own rule, is drawn whole, with no empty row beside it).
pub fn rows(saved: &[String], blanks: usize) -> Vec<String> {
    let room = MAX_PLACES.saturating_sub(saved.len());
    let mut rows = saved.to_vec();
    rows.extend(std::iter::repeat_n(String::new(), blanks.min(room)));
    rows
}

/// How many rows hold no name — the ones kept, empty, when an answer
/// replaces the rest with the list the server kept.
pub fn blanks(rows: &[String]) -> usize {
    rows.iter().filter(|row| fold(row).is_empty()).count()
}

/// Whether "Add place" is offered: while fewer than [`MAX_PLACES`] rows
/// are drawn. At three, [`limit_note`] stands in its place.
pub fn can_add(rows: usize) -> bool {
    rows < MAX_PLACES
}

/// The field's heading, under "Daily greeting".
pub fn heading() -> &'static str {
    t("Weather in the greeting")
}

/// Each row's placeholder.
pub fn placeholder() -> &'static str {
    t("City or town")
}

/// The button that adds a row.
pub fn add_label() -> &'static str {
    t("Add place")
}

/// The accessible name of a row's remove button.
pub fn remove_label() -> &'static str {
    t("Remove place")
}

/// Shown instead of "Add place" once three rows are drawn.
pub fn limit_note() -> &'static str {
    t("Up to 3 places.")
}

/// The footnote under the field: where the names go, and that nothing else
/// does (the protocol's disclosure, in the apps' words).
pub fn footnote() -> &'static str {
    t("The daily greeting will also mention today's weather in these places. Only the place names are sent to Open-Meteo to fetch the forecast. Nothing else is sent.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{use_lang, Lang};

    fn list(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    /// The owner, on a server that posts greetings AND can fetch weather —
    /// and nobody else, nowhere else. An old server sends neither key,
    /// which reads as false.
    #[test]
    fn the_field_is_offered_to_the_owner_where_the_server_uses_it() {
        assert!(offered(true, true, true));
        assert!(!offered(false, true, true), "a member sees nothing");
        assert!(!offered(true, true, false), "no weather on this server");
        assert!(!offered(true, false, true), "no greetings on this server");
        assert!(!offered(true, false, false), "an old server");
    }

    /// Kept as the server keeps them, so the answer is the list sent.
    #[test]
    fn places_are_trimmed_folded_and_deduplicated_as_the_server_does() {
        assert_eq!(
            places(&list(&["  Moscow ", "Novi\t  Sad", "MOSCOW"])),
            Ok(list(&["Moscow", "Novi Sad"]))
        );
        // Unicode lower-casing, not ASCII.
        assert_eq!(
            places(&list(&["Москва", "МОСКВА", "Белград"])),
            Ok(list(&["Москва", "Белград"]))
        );
        assert_eq!(places(&[]), Ok(Vec::new()));
        // Three counted AFTER repeats are dropped.
        assert_eq!(
            places(&list(&["Moscow", "moscow", "Belgrade", "Paris"])),
            Ok(list(&["Moscow", "Belgrade", "Paris"]))
        );
    }

    /// An empty field is not a place: never sent as "", which the server
    /// refuses.
    #[test]
    fn an_empty_row_is_dropped_not_sent() {
        assert_eq!(
            places(&list(&["", "Belgrade", "   "])),
            Ok(list(&["Belgrade"]))
        );
        assert_eq!(places(&list(&["  ", "\t"])), Ok(Vec::new()));
    }

    /// The server's refusals, by position and never by name.
    #[test]
    fn what_the_server_would_refuse_is_refused_here_first() {
        assert_eq!(
            places(&list(&["Moscow", "Bel\u{0}grade"])),
            Err(Refusal::Control { position: 2 })
        );
        assert_eq!(
            places(&list(&["Moscow\u{7f}"])),
            Err(Refusal::Control { position: 1 })
        );
        let long = "x".repeat(MAX_CHARS + 1);
        assert_eq!(
            places(std::slice::from_ref(&long)),
            Err(Refusal::TooLong { position: 1 })
        );
        assert_eq!(
            places(&list(&["Moscow", "Belgrade", "Paris", "Tokyo"])),
            Err(Refusal::TooMany)
        );
        // 80 exactly, in characters and not bytes; and 80 after folding
        // even when the field held more.
        let exact = "ж".repeat(MAX_CHARS);
        assert_eq!(
            places(std::slice::from_ref(&exact)),
            Ok(vec![exact.clone()])
        );
        let spaced = format!("   {}   ", "a b".repeat(26));
        assert_eq!(fold(&spaced).chars().count(), 78);
        assert!(places(&[spaced]).is_ok());
    }

    /// A field keeps what the server would accept: control characters out
    /// (whitespace stays, to be folded), and no more than 80 characters —
    /// characters, so a Cyrillic or emoji name is not cut at 40.
    #[test]
    fn a_field_keeps_at_most_eighty_characters_and_no_control_characters() {
        assert_eq!(typed("Bel\u{0}gr\u{1b}ade"), "Belgrade");
        assert_eq!(typed("Novi\tSad"), "Novi\tSad");
        assert_eq!(typed(&"ж".repeat(90)), "ж".repeat(MAX_CHARS));
        assert_eq!(typed(&"🌤".repeat(81)).chars().count(), MAX_CHARS);
        assert_eq!(typed("Moscow"), "Moscow");
        // Whatever a field keeps, the server accepts.
        for raw in ["\u{0}\u{1}x", &"y ".repeat(60), &"🇷🇸".repeat(50)] {
            assert!(places(&[typed(raw)]).is_ok(), "{raw:?}");
        }
    }

    /// Committing sends the whole list when it differs from the family's,
    /// and nothing when it does not.
    #[test]
    fn a_commit_sends_only_a_change() {
        let saved = list(&["Moscow", "Belgrade"]);
        assert_eq!(
            write(&list(&["Moscow", "Belgrade", ""]), &saved),
            Write::Nothing
        );
        assert_eq!(
            write(&list(&[" Moscow", "Belgrade  "]), &saved),
            Write::Nothing
        );
        assert_eq!(
            write(&list(&["Moscow", "Belgrade", "Paris"]), &saved),
            Write::Send(list(&["Moscow", "Belgrade", "Paris"]))
        );
        // Removing a row, and emptying one, are both a smaller list.
        assert_eq!(
            write(&list(&["Belgrade"]), &saved),
            Write::Send(list(&["Belgrade"]))
        );
        assert_eq!(
            write(&list(&["", "Belgrade"]), &saved),
            Write::Send(list(&["Belgrade"]))
        );
        // Emptying the last is [], which clears the list.
        assert_eq!(
            write(&list(&[""]), &list(&["Moscow"])),
            Write::Send(Vec::new())
        );
        assert_eq!(write(&[], &[]), Write::Nothing);
        assert_eq!(
            write(&list(&["a\u{0}"]), &[]),
            Write::Refused(Refusal::Control { position: 1 })
        );
    }

    /// Rows drawn: the list, then the empty rows added — three in all.
    #[test]
    fn rows_are_the_list_then_the_empty_rows_added_three_in_all() {
        assert_eq!(rows(&[], 0), Vec::<String>::new());
        assert_eq!(rows(&[], 1), list(&[""]));
        assert_eq!(rows(&list(&["Moscow"]), 1), list(&["Moscow", ""]));
        assert_eq!(
            rows(&list(&["Moscow", "Belgrade"]), 5),
            list(&["Moscow", "Belgrade", ""])
        );
        assert_eq!(rows(&list(&["a", "b", "c"]), 1), list(&["a", "b", "c"]));
        assert_eq!(
            rows(&list(&["a", "b", "c", "d"]), 1).len(),
            4,
            "drawn whole"
        );
        assert_eq!(blanks(&list(&["Moscow", "", "  "])), 2);
        assert!(can_add(0) && can_add(2));
        assert!(!can_add(3) && !can_add(4));
    }

    /// The six words are the apps' own, translated.
    #[test]
    fn the_fields_words_are_translated() {
        use_lang(Lang::De);
        assert_eq!(heading(), "Wetter im Gruß");
        assert_eq!(add_label(), "Ort hinzufügen");
        assert!(footnote().contains("Open-Meteo"));
        use_lang(Lang::Ru);
        assert!(footnote().contains("Open-Meteo"), "never translated");
        assert_ne!(placeholder(), "City or town");
        assert_ne!(remove_label(), "Remove place");
        assert_ne!(limit_note(), "Up to 3 places.");
        use_lang(Lang::En);
        assert_eq!(limit_note(), "Up to 3 places.");
        assert!(footnote().starts_with("The daily greeting will also mention today's weather"));
    }

    /// The footnote says where the names go and promises nothing about how
    /// often: a greeting retried after a model failure fetches again once
    /// the caches expire (protocol.md, "The disclosure"), so "once a day"
    /// would not be true — in any of the nine languages.
    #[test]
    fn the_footnote_promises_no_frequency() {
        let promises = [
            "once a day",
            "einmal am Tag",
            "una vez al día",
            "une fois par jour",
            "1日1回",
            "раз в день",
            "једном дневно",
            "jednom dnevno",
            "每天",
        ];
        for lang in [
            Lang::En,
            Lang::De,
            Lang::Es,
            Lang::Fr,
            Lang::Ja,
            Lang::Ru,
            Lang::Sr,
            Lang::SrLatn,
            Lang::ZhHans,
        ] {
            use_lang(lang);
            let text = footnote();
            assert!(text.contains("Open-Meteo"), "{lang:?}: {text}");
            for promise in promises {
                assert!(!text.contains(promise), "{lang:?}: {text}");
            }
        }
        use_lang(Lang::En);
    }
}
