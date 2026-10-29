//! The caps, counted the way the SERVER counts them — Unicode scalars, not UTF-16 units and not
//! graphemes. This is where a port is most likely to diverge, so it is pinned with the same awkward
//! strings the other ports use. A port of
//! `win/tests/FamilyConnect.Core.Tests/Board/NoteTextTests.cs`, against `fc_text::board` where the
//! rule lives there; the C#'s `NoteText.Scalars(text)` is Rust's own `text.chars().count()`.

use fc_core::board::note_text::done_of;
use fc_text::board::{
    COUNTER_FROM, MAX_PLACE_CHARS, MAX_TASK_ITEM_CHARS, MAX_TASK_ITEMS, MAX_TEXT_CHARS,
    cap_at_caret, capped, remaining, shows_counter,
};
use fc_text::i18n::{Lang, use_lang};

#[test]
fn the_caps_are_the_protocols_own() {
    assert_eq!(MAX_TEXT_CHARS, 280);
    assert_eq!(MAX_PLACE_CHARS, 200);
    assert_eq!(MAX_TASK_ITEM_CHARS, 100);
    assert_eq!(MAX_TASK_ITEMS, 20);
    assert_eq!(COUNTER_FROM, 40);
}

#[test]
fn the_cap_counts_scalars_the_way_the_server_does() {
    assert_eq!("Milk and eggs".chars().count(), 13);
    // An emoji is ONE scalar to the server and two UTF-16 units to .NET: counting units would give
    // an emoji note half its allowance.
    assert_eq!("🎂".chars().count(), 1);
    assert_eq!("🎂".encode_utf16().count(), 2);
    // A family emoji is several scalars joined by zero-width joiners — MORE than the one grapheme
    // a reader sees, which is the server's count and therefore ours.
    assert_eq!(
        "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}"
            .chars()
            .count(),
        7
    );
    // A letter typed with a COMBINING mark is two scalars and one grapheme — so a grapheme count
    // would be smaller than the server's, and a note that looked under the cap would come back
    // refused. (The precomposed spelling of the same letter is one of each.)
    assert_eq!("e\u{0301}".chars().count(), 2);
    assert_eq!("\u{00e9}".chars().count(), 1);

    let long = "a".repeat(300);
    assert_eq!(capped(&long, MAX_TEXT_CHARS).chars().count(), 280);
    assert_eq!(capped("Milk and eggs", 4), "Milk");
    assert_eq!(capped("Milk and eggs", 280), "Milk and eggs");
    assert_eq!(capped("Milk", 0), "");
    // A cap must never cut a character in half.
    assert_eq!(capped("🎂🎂", 1), "🎂");
    assert_eq!(capped("🎂", 0), "");
}

#[test]
fn the_counter_only_shows_in_the_last_forty() {
    assert_eq!(remaining(""), 280);
    assert!(!shows_counter(&"a".repeat(239)));
    assert!(shows_counter(&"a".repeat(240)));
    assert_eq!(remaining(&"a".repeat(400)), 0);
    assert!(shows_counter(&"a".repeat(400)));
}

#[test]
fn what_went_past_the_cap_is_cut_where_it_was_typed_not_off_the_end() {
    // Typing in the MIDDLE of a full note: the overflow comes out of what was just typed, and the
    // caret stays where the author was writing.
    let value = format!("{}xyz{}", "a".repeat(279), "b".repeat(20));
    let (kept, caret) = cap_at_caret(&value, 282, MAX_TEXT_CHARS);
    assert_eq!(kept.chars().count(), MAX_TEXT_CHARS);
    assert!(kept.ends_with(&"b".repeat(20)));
    // The caret FOLLOWS the cut: it sits where the deleted run began, which is the end of what
    // the author's typing left behind — not at the end of the note, which is where the old "cut
    // the tail" rule used to dump it.
    assert_eq!(caret, 260);
    assert_eq!(kept, format!("{}{}", "a".repeat(260), "b".repeat(20)));
    // Under the cap: nothing moves at all.
    assert_eq!(cap_at_caret("Milk", 2, 280), ("Milk".to_string(), 2));
    // A caret at the very start has nothing before it to take, so the END is cut, as before.
    let (from_start, start_caret) = cap_at_caret(&"a".repeat(300), 0, 280);
    assert_eq!(from_start.chars().count(), 280);
    assert_eq!(start_caret, 0);
}

#[test]
fn a_list_says_how_much_of_it_is_done() {
    // `EnglishCatalog.Instance`: the reader's language is English.
    use_lang(Lang::En);
    assert_eq!(done_of(2, 5), "2 of 5 done");
}
