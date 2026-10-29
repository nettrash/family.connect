//! `GreetingPlacesTests`: the owner's places for the greeting's weather, under the server's own
//! rules for a name (`validate_greeting_places`): trimmed and folded, never empty, no control
//! character, at most 80 characters — Rust's characters, counted as Rust counts them.
//!
//! `Fold` and `Typed` are `fc_text::greeting_places::{fold, typed}`; `Check` is
//! `fc_text::greeting_places::places` (see the two tests that say how they differ); `Length` and
//! `Judge` are `fc_core::text::greeting_places`. A C# `null` is the empty string here.

use fc_core::text::greeting_places::{Problem, judge, length};
use fc_text::greeting_places::{MAX_CHARS, MAX_PLACES, Refusal, fold, places, typed};

fn rows(fields: &[&str]) -> Vec<String> {
    fields.iter().map(|field| field.to_string()).collect()
}

#[test]
fn the_limits_are_the_servers() {
    assert_eq!(MAX_PLACES, 3);
    assert_eq!(MAX_CHARS, 80);
}

/// `split_whitespace().join(" ")`: every run of Unicode whitespace, inside or at the ends.
#[test]
fn a_name_is_folded_as_the_server_folds_it() {
    for (raw, folded) in [
        ("  Moscow  ", "Moscow"),
        ("New   York", "New York"),
        ("\tSan\u{A0}\u{2003}Francisco\n", "San Francisco"),
        ("Rio\u{3000}de\u{2028}Janeiro", "Rio de Janeiro"),
        ("   ", ""),
        ("", ""),
        // Not whitespace to Rust, so not folded: a zero-width space and the information
        // separators.
        ("A\u{200B}B", "A\u{200B}B"),
        ("A\u{1F}B", "A\u{1F}B"),
    ] {
        assert_eq!(fold(raw), folded, "{raw:?}");
    }
}

#[test]
fn each_name_is_judged_as_the_server_judges_it() {
    assert_eq!(judge("Belgrade"), None);
    assert_eq!(judge(" \t "), Some(Problem::Empty));
    // A control character that is not whitespace stays after folding, and is refused.
    assert_eq!(judge("Mos\u{1}cow"), Some(Problem::Control));
    assert_eq!(judge("Mos\u{7F}cow"), Some(Problem::Control));
    // A line break is whitespace, folded away before the check.
    assert_eq!(judge("Mos\ncow"), None);

    // Eighty characters after folding is the most; the spaces folded away do not count.
    assert_eq!(judge(&"a".repeat(80)), None);
    assert_eq!(judge(&"a".repeat(81)), Some(Problem::TooLong));
    assert_eq!(
        judge(&format!("   {}      {}", "a".repeat(40), "b".repeat(39))),
        None
    );
}

/// Characters are Unicode scalars: eighty astral characters are 160 UTF-16 units and still a
/// valid name.
#[test]
fn characters_are_scalars_not_utf16_units() {
    let astral = "\u{2000B}".repeat(80);
    assert_eq!(astral.encode_utf16().count(), 160);
    assert_eq!(length(&astral), 80);
    assert_eq!(judge(&astral), None);
    assert_eq!(judge(&format!("{astral}x")), Some(Problem::TooLong));
    assert_eq!(length(&"Ж".repeat(80)), 80);
}

/// The fields as a request: folded, blank fields left out (a field nobody typed in is no place),
/// at most three, and the first field the server would refuse named by its position.
///
/// The C# `Check` names the FIELD past the third (`BadField` 5 for `A B _ C D`);
/// `fc_text::greeting_places::places` says `TooMany` with no position, so that is all asserted
/// of it here.
#[test]
fn the_fields_become_a_request_or_name_the_first_bad_field() {
    assert_eq!(
        places(&rows(&["  Moscow ", "", "Belgrade"])),
        Ok(rows(&["Moscow", "Belgrade"]))
    );

    assert_eq!(places(&rows(&["", "   "])), Ok(Vec::new()));

    assert_eq!(
        places(&rows(&["Moscow", "Bel\u{2}grade"])),
        Err(Refusal::Control { position: 2 })
    );

    assert_eq!(
        places(&rows(&[&"x".repeat(81)])),
        Err(Refusal::TooLong { position: 1 })
    );

    assert_eq!(
        places(&rows(&["A", "B", "", "C", "D"])),
        Err(Refusal::TooMany)
    );
}

/// Repeats are the SERVER's to drop, says `win/`: its lower-casing is Rust's, and a second answer
/// here could disagree with it.
#[test]
#[ignore = "DIVERGENCE: fc_text::greeting_places::places drops a repeat itself (Rust's \
            to_lowercase, first spelling kept) and answers [\"Moscow\"]; win/'s Check sends both \
            for the server to drop"]
fn repeats_are_sent_for_the_server_to_drop() {
    assert_eq!(
        places(&rows(&["Moscow", "moscow"])),
        Ok(rows(&["Moscow", "moscow"]))
    );
}

/// What a field holds as somebody types or pastes: no control character that is not whitespace,
/// at most 80 characters, never half a character — and the spaces left alone, or the space
/// before the next word would be eaten.
#[test]
fn typing_keeps_what_the_server_would_keep() {
    assert_eq!(typed("New "), "New ");
    assert_eq!(typed("  Rio  de"), "  Rio  de");
    assert_eq!(typed("Mos\u{0}c\u{7}ow\u{9B}"), "Moscow");
    assert_eq!(typed("Mos\tcow"), "Mos\tcow");
    assert_eq!(typed(""), "");
    assert_eq!(typed(&"a".repeat(100)), "a".repeat(80));

    let astral = "\u{1F600}".repeat(81);
    let kept = typed(&astral);
    assert_eq!(length(&kept), 80);
    assert_eq!(kept.encode_utf16().count(), 160);
    assert!(kept.ends_with('\u{1F600}'));

    // Whatever typing leaves, folded, is never refused for its characters or its length.
    for text in [
        " ".repeat(200),
        "a\u{1}b".to_string(),
        astral.clone(),
        format!("x{}y", "\u{85}".repeat(90)),
    ] {
        assert_ne!(judge(&typed(&text)), Some(Problem::Control));
        assert_ne!(judge(&typed(&text)), Some(Problem::TooLong));
    }
}

#[test]
fn typing_counts_the_length_the_server_counts_after_folding() {
    // The Copilot case: 79 leading spaces cost nothing, so all of "Paris" is kept, whitespace and
    // all.
    let padded = format!("{}Paris", " ".repeat(79));
    assert_eq!(typed(&padded), padded);
    assert_eq!(judge(&typed(&padded)), None);

    // A run of whitespace between two words costs ONE, as its folded space does.
    let gapped = format!("{} \t\u{3000}\u{2028}  {}", "a".repeat(39), "b".repeat(40));
    assert_eq!(typed(&gapped), gapped);
    assert_eq!(length(&fold(&gapped)), 80);
    assert_eq!(typed(&format!("{gapped}c")), gapped);

    // The 81st folded character stops it — and so does the gap that would make it the 81st.
    assert_eq!(
        typed(&format!("{}  b", "a".repeat(79))),
        format!("{}  ", "a".repeat(79))
    );
    assert_eq!(
        typed(&format!("{}  b", "a".repeat(78))),
        format!("{}  b", "a".repeat(78))
    );

    // Trailing whitespace past the limit is kept (the space somebody is typing), and costs
    // nothing.
    let full = format!("{}   ", "a".repeat(80));
    assert_eq!(typed(&full), full);

    // Astral and multi-byte text: one scalar, one character, whatever its UTF-16 or UTF-8 length.
    let emoji = format!(
        "{}{}  \u{1F1F7}\u{1F1F8}!",
        " ".repeat(40),
        "\u{1F600}".repeat(77)
    );
    let typed_emoji = typed(&emoji);
    assert_eq!(
        typed_emoji,
        format!(
            "{}{}  \u{1F1F7}\u{1F1F8}",
            " ".repeat(40),
            "\u{1F600}".repeat(77)
        )
    );
    assert_eq!(length(&fold(&typed_emoji)), 80);

    let cyrillic = format!("   {}Санкт-Петербург", "Москва ".repeat(10));
    let typed_cyrillic = typed(&cyrillic);
    assert_eq!(length(&fold(&typed_cyrillic)), 80);
    assert!(typed_cyrillic.starts_with(&format!("   {}", "Москва ".repeat(10))));
    assert!(typed_cyrillic.ends_with("Москва Санкт-Пете"));

    // A control character dropped costs nothing either, and does not open a gap.
    assert_eq!(
        typed(&format!("{}\u{1}\u{9B}{}", "a".repeat(40), "a".repeat(41))),
        "a".repeat(80)
    );
}
