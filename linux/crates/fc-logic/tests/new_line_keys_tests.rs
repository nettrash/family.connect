//! `NewLineKeysTests`: #95 — Enter sends; Enter with Shift, Ctrl or Alt is a new line — and a new
//! line goes out as a line feed.
//!
//! `ANoteDraftHoldsLineFeeds`, whose subject is `NoteDraft`, is `note_draft_new_line_keys.rs`.

use std::borrow::Cow;

use fc_logic::new_line_keys::{enter_makes_line, for_wire};

#[test]
fn enter_alone_sends_and_any_modifier_makes_a_line() {
    for (shift, control, alt, line) in [
        (false, false, false, false),
        (true, false, false, true),
        (false, true, false, true),
        (false, false, true, true),
        (true, true, true, true),
    ] {
        assert_eq!(
            enter_makes_line(shift, control, alt),
            line,
            "{shift} {control} {alt}"
        );
    }
}

#[test]
fn a_text_box_line_break_goes_out_as_a_line_feed() {
    for (typed, sent) in [
        ("one\rtwo", "one\ntwo"),
        ("one\r\ntwo", "one\ntwo"),
        ("one\ntwo", "one\ntwo"),
        ("a\r\rb", "a\n\nb"),
        ("a\r\n\r\nb\r", "a\n\nb\n"),
        ("", ""),
        ("no lines", "no lines"),
    ] {
        assert_eq!(for_wire(typed), sent, "{typed:?}");
    }
}

/// `Assert.Same`: the very string back, borrowed, not a copy.
#[test]
fn text_without_a_carriage_return_is_the_same_string() {
    let text = "hello\nworld";
    let wired = for_wire(text);
    assert!(matches!(wired, Cow::Borrowed(_)));
    assert!(std::ptr::eq(wired.as_ref(), text));
}
