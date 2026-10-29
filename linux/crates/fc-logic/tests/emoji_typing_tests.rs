//! `EmojiTypingTests`: EMOJI AS YOU TYPE, held to `fc_text::emoji_typing` case for case (issue
//! #101; docs/emoji-as-you-type-2026-10-18.md). `fixtures/emoji-typing-vectors.json` was printed by
//! the Rust the web client runs (`win/tools/board-oracle`, `cargo run -- emoji-typing`): the
//! emoticon table, and every key sequence typed into an empty field with what the field then holds.
//! The same bytes are held by the Apple, Android and Windows ports.
//!
//! The rule is `fc_text`'s own, so the vectors here check THIS port's door to it: the UTF-16
//! offsets of `EmojiSwap`, `replace` given a string, the port's own `typing` — and then what is
//! the field's and so not in the vectors: the rule driven through a field's events (`CapitalField`,
//! by `TypingBox`) — capitals first on a multi-line field, emoji only on a single-line one, the
//! take-back, everything that drops the live swap, and the field's maximum length.

mod common;

use std::collections::HashMap;

use common::typing_box::{TypingBox, units};
use fc_logic::capital_field::{CapitalField, CapitalKey};
use fc_logic::emoji_typing::{self, EmojiSwap};

struct Vectors {
    emoticons: Vec<(String, String)>,
    typing: Vec<(String, String)>,
}

fn vectors() -> Vectors {
    let root = common::fixture_json("emoji-typing-vectors.json");
    let rows = |name: &str, first: &str, second: &str| -> Vec<(String, String)> {
        root[name]
            .as_array()
            .unwrap_or_else(|| panic!("no {name}"))
            .iter()
            .map(|row| {
                let get = |field: &str| {
                    row[field]
                        .as_str()
                        .unwrap_or_else(|| panic!("a vector without {field}"))
                        .to_string()
                };
                (get(first), get(second))
            })
            .collect()
    };
    Vectors {
        emoticons: rows("emoticons", "text", "emoji"),
        typing: rows("typing", "keys", "text"),
    }
}

fn swap(at: usize, put: &str, typed: &str) -> Option<EmojiSwap> {
    Some(EmojiSwap::new(at, put, typed))
}

/// Rust's `typing`, written out again here rather than called, so the vectors check
/// `emoji_typing::replace` and `EmojiSwap` through a second reading of the original: the caret
/// always at the end; `⌫` takes back the live swap or else removes the last code point; `→` drops
/// the live swap; anything else is typed — replaced, or appended.
fn simulate(keys: &str) -> String {
    let mut text = String::new();
    let mut live: Option<EmojiSwap> = None;
    for key in keys.chars() {
        if key == '⌫' {
            if let Some((undone, _)) = live
                .as_ref()
                .and_then(|swap| swap.backspace(&text, units(&text)))
            {
                text = undone;
            } else {
                text.pop();
            }
            live = None;
        } else if key == '→' {
            live = None;
        } else if let Some(swap) = emoji_typing::replace(&text, &key.to_string(), live.as_ref()) {
            text = swap.apply(&text, "").0;
            live = Some(swap);
        } else {
            text.push(key);
            live = None;
        }
    }
    text
}

fn all_hold(typing: impl Fn(&str) -> String, spell: impl Fn(&str) -> String) {
    let file = vectors();
    let failures: Vec<String> = file
        .typing
        .iter()
        .filter_map(|(keys, text)| {
            let typed = typing(&spell(keys));
            (typed != spell(text)).then(|| format!("{keys:?} -> {typed:?} (expected {text:?})"))
        })
        .collect();
    assert!(
        failures.is_empty(),
        "{} of {} vectors fail:\n{}",
        failures.len(),
        file.typing.len(),
        failures
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

fn as_typed(text: &str) -> String {
    text.to_string()
}

#[test]
fn the_vectors_are_the_whole_file() {
    let file = vectors();
    assert_eq!(file.emoticons.len(), 41);
    assert!(
        file.typing.len() >= 1600,
        "{} typing vectors",
        file.typing.len()
    );
}

/// (a) The table, entry for entry and in the original's order: the order is part of the port.
#[test]
fn the_emoticons_are_the_originals() {
    let table: Vec<(String, String)> = emoji_typing::EMOTICONS
        .iter()
        .map(|&(text, emoji)| (text.to_string(), emoji.to_string()))
        .collect();
    assert_eq!(vectors().emoticons, table);
}

/// (b) Every key sequence, through a simulator that is Rust's `typing` line for line.
#[test]
fn every_vector_holds() {
    all_hold(simulate, as_typed);
}

/// The port's own `emoji_typing::typing` agrees with every vector too.
#[test]
fn the_ports_typing_agrees() {
    all_hold(emoji_typing::typing, as_typed);
}

/// Every vector again, typed into a single-line field through the window's whole sequence — the
/// key, the change before it lands, the selection it leaves, the high-priority turn, the Backspace:
/// the integration says what the rule says.
#[test]
fn every_vector_holds_through_the_windows_events() {
    all_hold(
        |keys| {
            let mut typing_box = TypingBox::new(false);
            typing_box.type_all(keys);
            assert_eq!(typing_box.start, units(&typing_box.text));
            typing_box.text
        },
        as_typed,
    );
}

/// A WinUI `TextBox` keeps a line break as a lone carriage return: the fences and code spans are
/// found by it too.
#[test]
fn every_vector_holds_with_the_text_boxs_line_breaks() {
    all_hold(
        |keys| {
            let mut typing_box = TypingBox::new(false);
            typing_box.type_all(keys);
            typing_box.text
        },
        |text| text.replace('\n', "\r"),
    );
}

/// The names: `fc_text`'s table is private, so the C#'s `NameCount` is counted here off the very
/// file `fc_text` embeds (`web/text/data/emoji-names.tsv`, byte-identical to `win/`'s copy) — and
/// every name in it must reach its emoji (its last line, for a name listed twice) through this
/// port's `emoji_named`.
#[test]
fn the_names_load() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../web/text/data/emoji-names.tsv");
    let tsv = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let mut names: HashMap<&str, &str> = HashMap::new();
    for line in tsv.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((name, emoji)) = line.split_once('\t') {
            names.insert(name, emoji);
        }
    }
    assert!(names.len() > 6000, "{} names", names.len());
    for (name, emoji) in &names {
        assert_eq!(emoji_typing::emoji_named(name), Some(*emoji), "{name}");
    }
    assert_eq!(emoji_typing::emoji_named("fire"), Some("🔥"));
    assert_eq!(emoji_typing::emoji_named("piñata"), Some("🪅"));
    assert_eq!(emoji_typing::emoji_named("like"), Some("👍"));
    assert_eq!(emoji_typing::emoji_named(""), None);
    assert_eq!(emoji_typing::emoji_named(":fire:"), None);
    assert_eq!(emoji_typing::emoji_named("Fire"), None);
    // No comment line became a name.
    assert_eq!(emoji_typing::emoji_named("#"), None);
}

/// (c) A swap in the middle of the text: `apply` keeps what is after the caret, and the Backspace
/// puts back what was typed.
#[test]
fn a_swap_lands_between_before_and_after() {
    let made = emoji_typing::replace("ok :", ")", None);
    assert_eq!(made, swap(3, "😊", ":)"));
    let made = made.expect("a swap");
    let (text, caret) = made.apply("ok :", " bye");
    assert_eq!(text, "ok 😊 bye");
    // UTF-16 units: the emoji is a surrogate pair.
    assert_eq!(caret, 5);
    assert_eq!(
        made.backspace(&text, caret),
        Some(("ok :) bye".to_string(), 5))
    );
    // Not right behind it: an ordinary Backspace.
    assert_eq!(made.backspace(&text, caret + 1), None);
    assert_eq!(made.backspace(&text, caret - 1), None);
    assert_eq!(made.backspace("ok x bye", caret), None);
    assert_eq!(made.backspace("ok", caret), None);
    // A space-ended emoticon keeps its space, and gives back all three characters.
    let spaced = emoji_typing::replace("a :o", " ", None);
    assert_eq!(spaced, swap(2, "😨 ", ":o "));
    let spaced = spaced.expect("a swap");
    let (with_space, after) = spaced.apply("a :o", "b");
    assert_eq!(with_space, "a 😨 b");
    assert_eq!(
        spaced.backspace(&with_space, after),
        Some(("a :o b".to_string(), 5))
    );
}

#[test]
fn a_swap_grows_only_from_where_it_started() {
    // A shortcode's closing colon and a `(` are not a `:(`.
    let fire = emoji_typing::replace(":fire", ":", None).expect("a shortcode");
    let text = fire.apply(":fire", "").0;
    assert_eq!(emoji_typing::replace(&text, "(", Some(&fire)), None);
    // `:(` then `(` is one swap from the same place.
    let sad = emoji_typing::replace("hi :", "(", None);
    assert_eq!(sad, swap(3, "😞", ":("));
    let sad = sad.expect("a face");
    assert_eq!(
        emoji_typing::replace(&sad.apply("hi :", "").0, "(", Some(&sad)),
        swap(3, "😩", ":((")
    );
    // Not right behind it any more: no growing (and a lone `(` is nothing).
    assert_eq!(
        emoji_typing::replace(&(sad.apply("hi :", "").0 + "x"), "(", Some(&sad)),
        None
    );
}

/// Offsets are UTF-16: an emoji or a Cyrillic word before the face moves it by its units, never
/// splits a pair.
#[test]
fn offsets_are_utf16_units() {
    assert_eq!(emoji_typing::replace("😀:", ")", None), swap(2, "😊", ":)"));
    assert_eq!(
        emoji_typing::replace("Привет:fire", ":", None),
        swap(6, "🔥", ":fire:")
    );
    // A typed surrogate pair is one character, and makes nothing.
    assert_eq!(emoji_typing::replace(":", "😀", None), None);
    assert_eq!(
        emoji_typing::replace(":(", "😀", Some(&EmojiSwap::new(0, "😞", ":("))),
        None
    );
    // The guards are ASCII: a Cyrillic letter is no letter to them.
    assert_eq!(emoji_typing::replace("я<", "3", None), swap(1, "❤️", "<3"));
    assert_eq!(emoji_typing::replace("я3", "(", None), swap(1, "😔", "3("));
    assert_eq!(emoji_typing::replace("z3", "(", None), None);
    // A full-width digit is no ASCII digit either.
    assert_eq!(emoji_typing::replace("１:", ")", None), swap(1, "😊", ":)"));
}

// ---- through a field's events (CapitalField) ---------------------------------------------------

fn typed(keys: &str, capitals: bool) -> TypingBox {
    let mut typing_box = TypingBox::new(capitals);
    typing_box.type_all(keys);
    typing_box
}

/// Capitals decide the typed character first (#97), then the emoji rule sees it: `xD` at a
/// sentence's start is `XD`.
#[test]
fn capitals_first_then_emoji() {
    let mut typing_box = typed("xD ", true);
    assert_eq!(typing_box.text, "😆 ");
    assert_eq!(typing_box.start, 3);
    // The Backspace puts back what the field held: the capital the rule made, and the rest as typed.
    typing_box.backspace();
    assert_eq!(typing_box.text, "XD ");
    assert_eq!(typing_box.start, 3);

    assert_eq!(typed("hi :)", true).text, "Hi 😊");
    assert_eq!(typed("ok. :fire: yes", true).text, "Ok. 🔥 yes");
    // An emoji ends no sentence.
    assert_eq!(typed("привет:) как дела", true).text, "Привет😊 как дела");
}

/// A single-line field gets emoji and no capitals.
#[test]
fn a_single_line_field_gets_emoji_only() {
    assert_eq!(typed("hi :)", false).text, "hi 😊");
    assert_eq!(typed("xD ", false).text, "😆 ");
    assert_eq!(typed("xD ⌫", false).text, "xD ");
}

/// The characters go in as typed and the emoji replaces them: its own undo step, so Ctrl+Z gives
/// the face back.
#[test]
fn the_emoji_is_its_own_undo_step() {
    let typing_box = typed(":)", true);
    assert_eq!(typing_box.text, "😊");
    assert_eq!(typing_box.undo_units, 3);
    assert_eq!(typing_box.start, 2);
}

#[test]
fn growing_and_taking_back() {
    let mut typing_box = typed(":((", true);
    assert_eq!(typing_box.text, "😩");
    typing_box.backspace();
    assert_eq!(typing_box.text, ":((");
    assert_eq!(typing_box.start, 3);
    // Once: the next Backspace is an ordinary one.
    typing_box.backspace();
    assert_eq!(typing_box.text, ":(");
    assert_eq!(typed(":fire:⌫", true).text, ":fire:");
    assert_eq!(typed(":fire::)", true).text, "🔥😊");
}

/// In the middle of the text: the face is found behind the caret, what is after it stays, and the
/// caret lands right behind the emoji.
#[test]
fn in_the_middle_of_the_text() {
    let mut typing_box = TypingBox::holding("ab", 1, 0);
    typing_box.type_all(":)");
    assert_eq!(typing_box.text, "a😊b");
    assert_eq!(typing_box.start, 3);
    typing_box.backspace();
    assert_eq!(typing_box.text, "a:)b");
    assert_eq!(typing_box.start, 3);
}

#[test]
fn anything_else_drops_the_swap() {
    // A click elsewhere, even back again.
    let mut typing_box = typed(":)", true);
    typing_box.click(0, 0);
    typing_box.click(2, 0);
    typing_box.backspace();
    assert_eq!(typing_box.text, "");

    // A selection.
    let mut typing_box = typed(":)", true);
    typing_box.click(0, 2);
    assert_eq!(typing_box.field.swap(), None);

    // An arrow: no growing after it, and an ordinary Backspace.
    assert_eq!(typed(":(→(", true).text, "😞(");
    assert_eq!(typed(":)→⌫", true).text, "");

    // A Ctrl chord.
    let mut typing_box = typed(":)", true);
    typing_box
        .field
        .key(CapitalField::classify(0x5A, true, false));
    assert_eq!(typing_box.field.swap(), None);

    // A paste.
    let mut typing_box = typed(":)", true);
    typing_box.paste("x");
    typing_box.backspace();
    assert_eq!(typing_box.text, "😊");

    // The focus leaving.
    let mut typing_box = typed(":)", true);
    typing_box.field.focus_lost();
    assert_eq!(typing_box.field.swap(), None);

    // An input method's composition starting.
    let mut typing_box = typed(":)", true);
    typing_box.field.composition_started();
    assert_eq!(typing_box.field.swap(), None);
}

/// A typing key that typed nothing — Enter in a single-line field, a dead key, a character the
/// field refused — is something else happening: the Backspace after it is ordinary, and the
/// character after it grows nothing.
#[test]
fn a_key_that_typed_nothing_drops_the_swap() {
    let mut typing_box = typed(":)", false);
    typing_box.type_nothing(0x0D);
    typing_box.backspace();
    assert_eq!(typing_box.text, "");

    let mut typing_box = typed(":(", false);
    typing_box.type_nothing(0xDE);
    typing_box.type_text("(");
    assert_eq!(typing_box.text, "😞(");
}

/// A Shift on its own is not "another key": Shift+Backspace still takes the emoji back, and Shift+9
/// still grows it.
#[test]
fn a_modifier_drops_nothing() {
    let mut typing_box = typed(":(", true);
    typing_box
        .field
        .key(CapitalField::classify(0x10, false, false));
    typing_box.type_text("(");
    assert_eq!(typing_box.text, "😩");
    typing_box
        .field
        .key(CapitalField::classify(0x10, false, false));
    typing_box.backspace();
    assert_eq!(typing_box.text, ":((");
}

/// Only typing is judged: a paste, the window's own text and an input method's text are left as
/// they are.
#[test]
fn only_typing_is_judged() {
    let mut typing_box = TypingBox::new(true);
    typing_box.paste(":)");
    assert_eq!(typing_box.text, ":)");

    let mut typing_box = TypingBox::holding(":", 1, 0);
    typing_box.change(":)", 2);
    typing_box.turn();
    assert_eq!(typing_box.text, ":)");

    let mut typing_box = TypingBox::holding(":", 1, 0);
    typing_box.field.composition_started();
    typing_box.type_text(")");
    assert_eq!(typing_box.text, ":)");
}

#[test]
fn code_is_typed_as_it_is() {
    assert_eq!(typed("`:)", true).text, "`:)");
    assert_eq!(typed("```\r:fire:", true).text, "```\r:fire:");
    assert_eq!(typed("```\rx\r```\r:)", false).text, "```\rx\r```\r😊");
}

/// A field's maximum length: the few shortcodes shorter than their emoji (`:wales:` is 7 units, its
/// flag 14) stay as typed when the emoji would not fit, rather than being cut off by the field.
#[test]
fn the_emoji_must_fit_the_box() {
    let wales = emoji_typing::emoji_named("wales").expect("the flag of Wales");
    assert_eq!(typed(":wales:", true).text, wales);
    let mut typing_box = TypingBox::new(false);
    typing_box.max_length = 10;
    typing_box.type_all(":wales:");
    assert_eq!(typing_box.text, ":wales:");
    let mut typing_box = TypingBox::new(false);
    typing_box.max_length = 14;
    typing_box.type_all(":wales:");
    assert_eq!(typing_box.text, wales);
    // A face is never longer than what was typed.
    let mut typing_box = TypingBox::new(false);
    typing_box.max_length = 2;
    typing_box.type_all(":)");
    assert_eq!(typing_box.text, "😊");
}

/// A capital's own take-back still works beside the emoji, and a change that went elsewhere before
/// the turn is left as typed.
#[test]
fn the_capital_still_works() {
    let mut typing_box = typed("h", true);
    assert_eq!(typing_box.text, "H");
    typing_box.backspace();
    assert_eq!(typing_box.text, "h");

    let mut typing_box = TypingBox::new(true);
    typing_box.field.key(CapitalKey::Typing);
    let plan = typing_box
        .field
        .before_change(":", 1, 0, ":)")
        .expect("a plan");
    assert!(plan.swap.is_some());
    assert_eq!(plan.take_back, None);
    assert!(!typing_box.field.still_applies(&plan, ":)x", 3, 0, 0));
    assert!(!typing_box.field.still_applies(&plan, ":)", 1, 0, 0));
    assert!(typing_box.field.still_applies(&plan, ":)", 2, 0, 0));
}
