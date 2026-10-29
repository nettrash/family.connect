//! `CapitalsTests`: capital letters (issue #97) — docs/capitals-and-rich-markdown-2026-10-17.md,
//! "The table every port's tests hold", verbatim (the same table as the web's
//! `composer::capitalizes` and the Mac's `CapitalsTests`) — and the take-back.
//!
//! The rule itself is `fc_text::composer::capitalizes`, whose own `capitals_the_design_table` and
//! `capitals_edges` hold the same rows; they are ported here all the same, because they go through
//! this port's own door — a `typed` that is a STRING (one character, or not), where `fc_text` takes
//! a `char` — and the field's offsets, which are UTF-16 units here and bytes there.
//!
//! Not portable: the C#'s lone-surrogate inputs (`"\uD801"`), which a Rust string cannot hold —
//! GTK hands the shell UTF-8, so no field can produce one.

use fc_logic::capitals::{self, CapitalTakeBack};

#[test]
fn the_design_table() {
    let rows: &[(&str, &str, bool)] = &[
        ("", "h", true),
        ("Hi. ", "h", true),
        // The word is not starting.
        ("Hi.", "h", false),
        ("hi ", "h", false),
        ("Really? ", "y", true),
        ("Wow! ", "s", true),
        // An ellipsis.
        ("Wait... ", "w", false),
        // An abbreviation.
        ("See e.g. ", "t", false),
        ("It's at example.com. ", "t", true),
        ("Line\n", "n", true),
        ("He said \"stop.\" ", "t", true),
        ("(Done.) ", "n", true),
        ("\"", "h", true),
        ("Hi. (", "s", true),
        ("- ", "m", true),
        ("List:\n1. ", "b", true),
        ("12) ", "x", true),
        ("# ", "t", true),
        ("> ", "q", true),
        // Inside a fence.
        ("```\n", "l", false),
        // The fence closed.
        ("```\ncode\n```\n", "n", true),
        // Inside a code span.
        ("Use `x. ", "y", false),
        // Already upper case.
        ("Hi. ", "H", false),
        // Not a letter.
        ("Hi. ", "1", false),
        // Its upper case is two letters.
        ("", "ß", false),
        // Two spaces.
        ("Hi.  ", "h", true),
        // A tab.
        ("Hi.\t", "h", true),
        ("", "é", true),
        ("Привет. ", "к", true),
        ("Ok.\n\n", "n", true),
        ("@ann ", "h", false),
    ];
    for &(before, typed, capitalizes) in rows {
        assert_eq!(
            capitals::capitalizes(before, typed),
            capitalizes,
            "{before:?} + {typed:?}"
        );
    }
}

/// The edges of each condition, beyond the table — the same as the web's `capitals_edges`.
#[test]
fn the_edges() {
    let rows: &[(&str, bool)] = &[
        // Every opening and closing mark.
        ("Hi. «", true),
        ("Hi. “", true),
        ("Hi. ‘", true),
        ("Hi. [", true),
        ("Hi. '", true),
        ("((", true),
        ("Done.» ", true),
        ("Done.” ", true),
        ("Done.’ ", true),
        ("Done.] ", true),
        ("Done!'' ", true),
        // A lone carriage return (what a WinUI TextBox keeps) is a line break.
        ("Line\r", true),
        ("```\r", false),
        ("```\rcode\r```\r", true),
        ("```\r\ncode\r\n```\r\n", true),
        // Markers: indented, every heading depth, and near misses.
        ("  - ", true),
        ("###### ", true),
        ("####### ", false),
        ("-- ", false),
        ("1.2 ", false),
        ("Hi. - ", false),
        // Abbreviations: the pieces must be one or two ASCII letters.
        ("a.m. ", false),
        ("in the U.S. ", false),
        ("v1.2. ", true),
        ("abc.d. ", true),
        ("é.g. ", true),
        ("Plan B. ", true),
        ("What?! ", true),
        ("Hi, ", false),
        ("Note: ", false),
        // A fence line counts only at the start of a line; a closed code span is not code.
        ("x ```\n", true),
        ("Use `x`. ", true),
    ];
    for &(before, capitalizes) in rows {
        assert_eq!(
            capitals::capitalizes(before, "a"),
            capitalizes,
            "{before:?}"
        );
    }
}

#[test]
fn which_characters_have_a_capital() {
    assert_eq!(capitals::capital_of("a").as_deref(), Some("A"));
    assert_eq!(capitals::capital_of("ж").as_deref(), Some("Ж"));
    assert_eq!(capitals::capital_of("ß"), None);
    assert_eq!(capitals::capital_of("A"), None);
    // Rust's full mapping, not .NET's simple one: ᾳ's capital is two letters, so it has none here —
    // as on the web.
    assert_eq!(capitals::capital_of("ᾳ"), None);
    // (The C#'s list also holds a lone high surrogate, "\uD801", which no Rust string can.)
    for typed in ["ŉ", "ǰ", "ﬀ", "ª", "日", " ", ".", "", "ab"] {
        assert_eq!(capitals::capital_of(typed), None, "{typed:?}");
        assert!(!capitals::capitalizes("", typed), "{typed:?}");
    }
    // Outside the Basic Multilingual Plane, a surrogate pair is one character: Deseret.
    assert_eq!(
        capitals::capital_of("\u{10428}").as_deref(),
        Some("\u{10400}")
    );
    assert!(capitals::capitalizes("Hi. ", "\u{10428}"));
}

fn typed(at: usize, text: &str) -> Option<(usize, String)> {
    Some((at, text.to_string()))
}

/// Only one character typed over the old selection counts — never a deletion that happens to look
/// like one.
#[test]
fn what_counts_as_typed() {
    assert_eq!(
        capitals::typed_character("Hi. ", 4, 0, "Hi. h"),
        typed(4, "h")
    );
    assert_eq!(
        capitals::typed_character("ello", 0, 0, "hello"),
        typed(0, "h")
    );
    // Typed over a selection.
    assert_eq!(
        capitals::typed_character("Hi. xyz", 4, 3, "Hi. t"),
        typed(4, "t")
    );
    // A surrogate pair is one character. (Half of one — the C#'s "\uD801" — no Rust string holds.)
    assert_eq!(
        capitals::typed_character("", 0, 0, "\u{10428}"),
        typed(0, "\u{10428}")
    );
    // A paste of two, a deletion, a Backspace that looks like typing, an unchanged text.
    assert_eq!(capitals::typed_character("", 0, 0, "hi"), None);
    assert_eq!(capitals::typed_character("hi", 2, 0, "h"), None);
    assert_eq!(capitals::typed_character("aab", 1, 0, "ab"), None);
    assert_eq!(capitals::typed_character("ab", 1, 0, "ab"), None);
    // The change is somewhere else than the selection.
    assert_eq!(capitals::typed_character("ab", 0, 0, "abx"), None);
    // A selection outside the text is nothing.
    assert_eq!(capitals::typed_character("ab", 3, 0, "abx"), None);
}

#[test]
fn after_typing_puts_in_the_capital_and_remembers_it() {
    let edit = capitals::after_typing("Hi. ", 4, 0, "Hi. h").expect("a capital");
    assert_eq!(edit.text, "Hi. H");
    assert_eq!(edit.caret, 5);
    assert_eq!(
        edit.take_back,
        CapitalTakeBack {
            at: 4,
            capital: "H".into(),
            typed: "h".into()
        }
    );

    // In the middle of the text, with what follows the caret kept.
    assert_eq!(
        capitals::after_typing("Ok. hen go", 4, 0, "Ok. then go").map(|edit| edit.text),
        Some("Ok. Then go".to_string())
    );
    assert_eq!(
        capitals::after_typing("Привет. ", 8, 0, "Привет. к").map(|edit| edit.text),
        Some("Привет. К".to_string())
    );
    assert_eq!(
        capitals::after_typing("", 0, 0, "\u{10428}").map(|edit| edit.text),
        Some("\u{10400}".to_string())
    );
    assert_eq!(
        capitals::after_typing("", 0, 0, "\u{10428}").map(|edit| edit.caret),
        Some(2)
    );

    // Not a sentence's start, not a letter, not one character: the change stands.
    assert_eq!(capitals::after_typing("hi ", 3, 0, "hi h"), None);
    assert_eq!(capitals::after_typing("Hi. ", 4, 0, "Hi. 1"), None);
    assert_eq!(capitals::after_typing("Hi. ", 4, 0, "Hi. hi"), None);
}

#[test]
fn a_backspace_straight_after_takes_it_back() {
    let take = CapitalTakeBack {
        at: 4,
        capital: "H".into(),
        typed: "h".into(),
    };
    assert_eq!(
        take.backspace("Hi. H", 5, 0),
        Some(("Hi. h".to_string(), 5))
    );
    assert_eq!(
        take.backspace("Hi. Hello", 5, 0),
        Some(("Hi. hello".to_string(), 5))
    );
    // Not right behind it, something selected, or the capital gone: an ordinary Backspace.
    assert_eq!(take.backspace("Hi. H", 4, 0), None);
    assert_eq!(take.backspace("Hi. H", 5, 1), None);
    assert_eq!(take.backspace("Hi. Ho", 6, 0), None);
    assert_eq!(take.backspace("Hi. x", 5, 0), None);
    assert_eq!(take.backspace("Hi.", 5, 0), None);

    let deseret = CapitalTakeBack::after(0, "\u{10428}").expect("Deseret has a capital");
    assert_eq!(
        deseret.backspace("\u{10400}!", 2, 0),
        Some(("\u{10428}!".to_string(), 2))
    );
    assert_eq!(deseret.backspace("\u{10400}!", 1, 0), None);
    assert_eq!(CapitalTakeBack::after(0, "ß"), None);
}
