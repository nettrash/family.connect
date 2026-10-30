//! `CapitalFieldTests`: a prose field's capital letters as a text field drives them (issue #97;
//! `CapitalField`): a fake field (`TypingBox`) that raises the same calls in the same order the
//! window makes them — the key, the change before it lands, the turn that applies the capital, the
//! Backspace — so the wiring is held to its sequence here. The same field's emoji (#101) are
//! `emoji_typing_tests`'. Keys are Win32 virtual keys, as in the C#; the Linux keysym classifier is
//! `capital_field_keysyms`'.

mod common;

use common::typing_box::{TypingBox, units};
use fc_logic::capital_field::{CapitalField, CapitalKey};

const VK_A: i32 = 0x41;
const VK_BACK: i32 = 0x08;
const VK_SHIFT: i32 = 0x10;
const VK_V: i32 = 0x56;
const VK_LEFT: i32 = 0x25;

#[test]
fn a_sentence_starts_with_a_capital() {
    let mut typing_box = TypingBox::new(true);
    typing_box.type_all("hi. how are you? fine");
    assert_eq!(typing_box.text, "Hi. How are you? Fine");
    assert_eq!(typing_box.start, units(&typing_box.text));
}

/// The letter goes in as typed and is then replaced: two undo units, so Ctrl+Z gives the lower-case
/// letter back.
#[test]
fn the_capital_is_its_own_undo_step() {
    let mut typing_box = TypingBox::new(true);
    typing_box.type_text("h");
    assert_eq!(typing_box.text, "H");
    assert_eq!(typing_box.undo_units, 2);
}

#[test]
fn backspace_straight_after_takes_it_back() {
    let mut typing_box = TypingBox::new(true);
    typing_box.type_text("h");
    typing_box.backspace();
    assert_eq!(typing_box.text, "h");
    assert_eq!(typing_box.start, 1);
    // Once: the next Backspace is an ordinary one.
    typing_box.backspace();
    assert_eq!(typing_box.text, "");
}

#[test]
fn anything_else_forgets_the_take_back() {
    let mut typing_box = TypingBox::new(true);
    typing_box.type_text("h");
    typing_box.type_text("i");
    typing_box.backspace();
    typing_box.backspace();
    assert_eq!(typing_box.text, "");

    // An arrow.
    let mut typing_box = TypingBox::new(true);
    typing_box.type_text("h");
    typing_box
        .field
        .key(CapitalField::classify(VK_LEFT, false, false));
    let (text, start, length) = (typing_box.text.clone(), typing_box.start, typing_box.length);
    assert_eq!(typing_box.field.backspace(&text, start, length), None);

    // A click elsewhere.
    let mut typing_box = TypingBox::new(true);
    typing_box.type_text("h");
    typing_box.field.selection_moved(0, 0);
    assert_eq!(typing_box.field.take_back(), None);

    // Losing the focus.
    let mut typing_box = TypingBox::new(true);
    typing_box.type_text("h");
    typing_box.field.focus_lost();
    assert_eq!(typing_box.field.take_back(), None);
}

/// Shift on its own is not "another key": Shift+Backspace still takes the capital back.
#[test]
fn a_modifier_forgets_nothing() {
    let mut typing_box = TypingBox::new(true);
    typing_box.type_text("h");
    typing_box
        .field
        .key(CapitalField::classify(VK_SHIFT, false, false));
    typing_box.field.selection_moved(1, 0);
    typing_box.backspace();
    assert_eq!(typing_box.text, "h");
}

/// The window's own text — a draft put back, a message opened for editing — is never capitalised.
#[test]
fn only_typing_is_judged() {
    let mut typing_box = TypingBox::new(true);
    typing_box.change("h", 1);
    typing_box.turn();
    assert_eq!(typing_box.text, "h");
}

#[test]
fn a_paste_is_not_typing() {
    let mut typing_box = TypingBox::new(true);
    typing_box
        .field
        .key(CapitalField::classify(VK_V, true, false));
    typing_box.field.pasting();
    typing_box.change("h", 1);
    typing_box.turn();
    assert_eq!(typing_box.text, "h");
}

/// AltGr is Ctrl+Alt on Windows and TYPES: `ą` on a Polish keyboard starts a sentence like any
/// letter.
#[test]
fn alt_gr_types() {
    let mut typing_box = TypingBox::new(true);
    typing_box.type_key("ą", VK_A, true, true);
    assert_eq!(typing_box.text, "Ą");
    assert_eq!(CapitalField::classify(VK_V, true, false), CapitalKey::Chord);
    assert_eq!(
        CapitalField::classify(VK_BACK, true, false),
        CapitalKey::Chord
    );
    assert_eq!(
        CapitalField::classify(VK_BACK, false, false),
        CapitalKey::Backspace
    );
    assert_eq!(
        CapitalField::classify(0xA2, true, false),
        CapitalKey::Modifier
    );
    assert_eq!(
        CapitalField::classify(VK_LEFT, false, false),
        CapitalKey::Moving
    );
    assert_eq!(
        CapitalField::classify(0x09, false, false),
        CapitalKey::Moving
    );
    assert_eq!(
        CapitalField::classify(0x0D, false, false),
        CapitalKey::Typing
    );
    assert_eq!(
        CapitalField::classify(0x20, false, false),
        CapitalKey::Typing
    );
}

/// A key that types nothing makes nothing after it typing: a name accepted on Tab, a draft put back
/// on an arrow.
#[test]
fn a_key_that_types_nothing_types_nothing() {
    let mut typing_box = TypingBox::new(true);
    typing_box
        .field
        .key(CapitalField::classify(0x09, false, false));
    typing_box.change("h", 1);
    typing_box.turn();
    assert_eq!(typing_box.text, "h");
}

#[test]
fn a_composition_is_left_alone() {
    let mut typing_box = TypingBox::new(true);
    typing_box.field.composition_started();
    typing_box.type_text("k");
    assert_eq!(typing_box.text, "k");
    typing_box.field.composition_ended();
    typing_box.text = String::new();
    typing_box.start = 0;
    typing_box.type_text("k");
    assert_eq!(typing_box.text, "K");
}

/// A change that went somewhere else before the turn — the control refused it, or more was typed —
/// is left as typed.
#[test]
fn a_stale_capital_is_not_applied() {
    let mut typing_box = TypingBox::new(true);
    typing_box.field.key(CapitalKey::Typing);
    let plan = typing_box
        .field
        .before_change("", 0, 0, "h")
        .expect("a plan");
    assert!(!typing_box.field.still_applies(&plan, "hx", 2, 0, 0));
    assert!(!typing_box.field.still_applies(&plan, "h", 0, 0, 0));
    assert!(!typing_box.field.still_applies(&plan, "h", 1, 1, 0));
    assert!(typing_box.field.still_applies(&plan, "h", 1, 0, 0));
}

/// A letter typed over a selection is typed: the rule reads the text before the selection's start.
#[test]
fn typing_over_a_selection() {
    let mut typing_box = TypingBox::holding("Ok. word", 4, 4);
    typing_box.type_text("n");
    assert_eq!(typing_box.text, "Ok. N");
}

/// Enter's new line is typing too (a CR, as a WinUI TextBox writes one), and the next letter starts
/// a line.
#[test]
fn a_new_line_starts_a_sentence() {
    let mut typing_box = TypingBox::new(true);
    typing_box.type_all("ok");
    typing_box.type_key("\r", 0x0D, false, false);
    typing_box.type_text("n");
    assert_eq!(typing_box.text, "Ok\rN");
}
