//! Emoji as you type (#101): Telegram's emoticons — `:-)`, `<3`, `;-P` — with
//! their no-nose forms, and `:name:` shortcodes, become emoji in the field the
//! moment the last character goes in. A Backspace straight after takes the
//! replacement back. docs/emoji-as-you-type-2026-10-18.md is the design; this
//! module is the original every other port is held to (board-oracle's
//! `emoji-typing` vectors).
//!
//! Every client runs it on every field #97's capitals run on, for one typed
//! character at a time: never on a paste, never in code. The text that is
//! sent holds the emoji itself, so nothing about it is on the wire.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::composer::in_code;

/// The emoticons, as typed, and the emoji each becomes.
///
/// The first 27 are Telegram Desktop's own table (desktop-app/codegen,
/// emoji/data.cpp, and lib_ui's `:o ` and `xD `), emoji for emoji. The rest
/// are the owner's addition (2026-10-18): the no-nose forms of the same faces,
/// so `:)` works as the issue asks, plus the case twins Telegram's
/// case-sensitive table misses. `8)` and `B)` are left out on purpose — "plan
/// B)" and a list's "8)" are far commoner than either face.
///
/// An entry ending in a space is replaced when that space is typed, and the
/// space stays: `:o` and `:p` would otherwise eat the start of `:ok_hand:` and
/// `:pizza:`, and `xD` of `xDDD`.
pub const EMOTICONS: &[(&str, &str)] = &[
    // Telegram's.
    (":-)", "\u{1F60A}"),
    ("8-)", "\u{1F60D}"),
    ("<3", "\u{2764}\u{FE0F}"),
    (":-*", "\u{1F61A}"),
    ("B-)", "\u{1F60E}"),
    (":-D", "\u{1F603}"),
    (";-)", "\u{1F609}"),
    (";-P", "\u{1F61C}"),
    (":-p", "\u{1F60B}"),
    ("3(", "\u{1F614}"),
    (":-(", "\u{1F61E}"),
    (":]", "\u{1F60F}"),
    (":'(", "\u{1F622}"),
    (":_(", "\u{1F62D}"),
    (":((", "\u{1F629}"),
    (":|", "\u{1F610}"),
    ("3-)", "\u{1F60C}"),
    (">(", "\u{1F620}"),
    (">((", "\u{1F621}"),
    ("O:)", "\u{1F607}"),
    (";o", "\u{1F630}"),
    ("8|", "\u{1F633}"),
    ("8o", "\u{1F632}"),
    (":X", "\u{1F637}"),
    ("}:)", "\u{1F608}"),
    (":o ", "\u{1F628}"),
    ("xD ", "\u{1F606}"),
    // The no-nose forms and the case twins.
    (":)", "\u{1F60A}"),
    (":D", "\u{1F603}"),
    (";)", "\u{1F609}"),
    (":(", "\u{1F61E}"),
    (":*", "\u{1F61A}"),
    (":P", "\u{1F60B}"),
    (":p ", "\u{1F60B}"),
    (":-P", "\u{1F60B}"),
    (";P", "\u{1F61C}"),
    (";p", "\u{1F61C}"),
    (";-p", "\u{1F61C}"),
    ("O:-)", "\u{1F607}"),
    ("}:-)", "\u{1F608}"),
    ("XD ", "\u{1F606}"),
];

/// The shortcode table: JoyPixels' names plus Telegram's four
/// (tools/emoji/make_names.py writes it; every client ships a copy).
const NAMES_TSV: &str = include_str!("../data/emoji-names.tsv");

/// The emoji `name` (no colons) stands for, if any. Names are matched exactly:
/// `:Fire:` is not `:fire:`.
pub fn emoji_named(name: &str) -> Option<&'static str> {
    static NAMES: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    NAMES
        .get_or_init(|| {
            NAMES_TSV
                .lines()
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .filter_map(|line| line.split_once('\t'))
                .collect()
        })
        .get(name)
        .copied()
}

/// A replacement the rule just made, kept so the Backspace pressed straight
/// after it can take it back — and so the next key can grow it, `:(` and
/// another `(` being `:((`.
///
/// The UI keeps one from the moment the rule replaces until anything else
/// happens — a key the rule does not act on, a click, a paste, a selection —
/// and then drops it, exactly like #97's capital. Offsets are BYTE offsets
/// into the field's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Swap {
    /// Where the replacement starts.
    pub at: usize,
    /// What was put in: the emoji, and the space after it for the emoticons
    /// that end in one.
    pub put: String,
    /// What was typed, all of it: what a Backspace puts back.
    pub typed: String,
}

impl Swap {
    /// The field's text and caret after the swap, given what was before the
    /// caret (without the typed character) and what is after it.
    pub fn apply(&self, before: &str, after: &str) -> (String, usize) {
        let mut text = String::with_capacity(self.at + self.put.len() + after.len());
        text.push_str(&before[..self.at]);
        text.push_str(&self.put);
        let caret = text.len();
        text.push_str(after);
        (text, caret)
    }

    /// A Backspace with the caret at `caret` and nothing selected: the text
    /// with what was typed put back, and the caret right behind it — or `None`
    /// when the caret is not right behind the replacement or the replacement
    /// is no longer there, and the Backspace is an ordinary one.
    pub fn backspace(&self, text: &str, caret: usize) -> Option<(String, usize)> {
        let end = self.at + self.put.len();
        if caret != end || text.get(self.at..end)? != self.put {
            return None;
        }
        let mut undone = String::with_capacity(text.len() + self.typed.len());
        undone.push_str(&text[..self.at]);
        undone.push_str(&self.typed);
        undone.push_str(&text[end..]);
        Some((undone, self.at + self.typed.len()))
    }
}

/// What typing `typed` turns into: `before` is the field's text before the
/// caret, without `typed`; `live` is the swap the UI still holds, if any.
/// `None` when the character goes in as it is.
///
/// In order:
///
/// 1. nothing in code (the test #97's capitals use);
/// 2. GROWING: the caret is right behind `live`'s replacement, and what was
///    typed then plus `typed` is a longer emoticon starting at the same
///    place — `:(` then `(` is `:((`, one swap whose Backspace puts back all
///    three characters;
/// 3. a SHORTCODE, when `typed` is `:` — `before` ends in `:name` with no
///    space or colon in the name, the opening colon is at the start or
///    after something that is not an ASCII letter or digit (so `10:30:` and
///    `a:b:` are left alone), and the table knows the name;
/// 4. an EMOTICON: `before` plus `typed` ends in one, the longest whose
///    guard passes — one starting with `:` or `;` must not follow an ASCII
///    digit (`1:]`), any other must not follow an ASCII letter or digit
///    (`x<3`, `item3(`). `HELLO:)` is `:)` after `HELL`'s `O`, because the
///    longer `O:)` fails its guard.
pub fn replace(before: &str, typed: char, live: Option<&Swap>) -> Option<Swap> {
    // 1. Code is typed as it is.
    if in_code(before) {
        return None;
    }
    // 2. Growing the swap just made.
    if let Some(live) = live {
        if live.at + live.put.len() == before.len() && before.get(live.at..) == Some(&live.put) {
            let mut grown = live.typed.clone();
            grown.push(typed);
            if let Some(&(_, emoji)) = EMOTICONS.iter().find(|(text, _)| *text == grown) {
                return Some(Swap {
                    at: live.at,
                    put: put_for(&grown, emoji),
                    typed: grown,
                });
            }
        }
    }
    // 3. `:name:`.
    if typed == ':' {
        if let Some(colon) = before.rfind(':') {
            let name = &before[colon + 1..];
            if !name.is_empty()
                && !name.chars().any(char::is_whitespace)
                && !before[..colon].ends_with(|c: char| c.is_ascii_alphanumeric())
            {
                if let Some(emoji) = emoji_named(name) {
                    return Some(Swap {
                        at: colon,
                        put: emoji.to_owned(),
                        typed: format!(":{name}:"),
                    });
                }
            }
        }
    }
    // 4. An emoticon, the longest that may stand here.
    let mut typed_text = String::with_capacity(before.len() + typed.len_utf8());
    typed_text.push_str(before);
    typed_text.push(typed);
    EMOTICONS
        .iter()
        .filter(|(text, _)| typed_text.ends_with(text) && text.len() <= typed_text.len())
        .filter(|(text, _)| guard_passes(&typed_text[..typed_text.len() - text.len()], text))
        .max_by_key(|(text, _)| text.len())
        .map(|&(text, emoji)| Swap {
            at: typed_text.len() - text.len(),
            put: put_for(text, emoji),
            typed: text.to_owned(),
        })
}

/// The emoji, and the space after it for an emoticon that ends in one.
fn put_for(text: &str, emoji: &str) -> String {
    if text.ends_with(' ') {
        format!("{emoji} ")
    } else {
        emoji.to_owned()
    }
}

/// Rule 4's guard: what may stand before `emoticon`.
fn guard_passes(head: &str, emoticon: &str) -> bool {
    let Some(previous) = head.chars().next_back() else {
        return true;
    };
    if emoticon.starts_with([':', ';']) {
        !previous.is_ascii_digit()
    } else {
        !previous.is_ascii_alphanumeric()
    }
}

/// Types `keys` into an empty field with the caret always at the end, the way
/// every port's vectors are checked: `⌫` is a Backspace, `→` is "something
/// else happened" (a click, a caret move — the live swap is dropped), and
/// every other character is typed. An ordinary Backspace removes one Unicode
/// scalar; the vectors only press one after ASCII.
pub fn typing(keys: &str) -> String {
    let mut text = String::new();
    let mut live: Option<Swap> = None;
    for key in keys.chars() {
        match key {
            '⌫' => {
                let caret = text.len();
                if let Some((undone, _)) =
                    live.as_ref().and_then(|swap| swap.backspace(&text, caret))
                {
                    text = undone;
                } else {
                    text.pop();
                }
                live = None;
            }
            '→' => live = None,
            _ => match replace(&text, key, live.as_ref()) {
                Some(swap) => {
                    text = swap.apply(&text, "").0;
                    live = Some(swap);
                }
                None => {
                    text.push(key);
                    live = None;
                }
            },
        }
    }
    text
}

/// The table every port's tests hold (docs/emoji-as-you-type-2026-10-18.md,
/// "The table"): keys typed, and the field's text after them.
pub const TYPING_TABLE: &[(&str, &str)] = &[
    // Telegram's emoticons, as Telegram maps them.
    (":-)", "😊"),
    ("8-)", "😍"),
    ("<3", "❤️"),
    (":-*", "😚"),
    ("B-)", "😎"),
    (":-D", "😃"),
    (";-)", "😉"),
    (";-P", "😜"),
    (":-p", "😋"),
    ("3(", "😔"),
    (":-(", "😞"),
    (":]", "😏"),
    (":'(", "😢"),
    (":_(", "😭"),
    ("3-)", "😌"),
    (":|", "😐"),
    (">(", "😠"),
    ("O:)", "😇"),
    (";o", "😰"),
    ("8|", "😳"),
    ("8o", "😲"),
    (":X", "😷"),
    ("}:)", "😈"),
    (":o ", "😨 "),
    ("xD ", "😆 "),
    // The no-nose forms and case twins.
    (":)", "😊"),
    (":D", "😃"),
    (";)", "😉"),
    (":(", "😞"),
    (":*", "😚"),
    (":P", "😋"),
    (":p ", "😋 "),
    (":-P", "😋"),
    (";P", "😜"),
    (";p", "😜"),
    (";-p", "😜"),
    ("O:-)", "😇"),
    ("}:-)", "😈"),
    ("XD ", "😆 "),
    // In a sentence.
    ("Hi :)", "Hi 😊"),
    ("thanks:)", "thanks😊"),
    ("Привет:)", "Привет😊"),
    ("i <3 u", "i ❤️ u"),
    ("ok :) bye", "ok 😊 bye"),
    // Growing: the second character of `((` makes the longer face, and one
    // Backspace puts back everything typed.
    (":((", "😩"),
    (">((", "😡"),
    (":((⌫", ":(("),
    (":(→(", "😞("),
    (":)(", "😊("),
    (":))", "😊)"),
    // The guards.
    ("x<3", "x<3"),
    ("item3(", "item3("),
    ("plan B-)", "plan 😎"),
    ("planB-)", "planB-)"),
    ("1:]", "1:]"),
    ("a[1:]", "a[1:]"),
    ("10:30", "10:30"),
    ("HELLO:)", "HELLO😊"),
    ("hi O:)", "hi 😇"),
    ("LOLO:)", "LOLO😊"),
    ("x}:)", "x}😊"),
    // The space-ended ones wait for the space.
    (":o", ":o"),
    (":ok_hand:", "👌"),
    (":pizza:", "🍕"),
    ("xDDD", "xDDD"),
    ("XD", "XD"),
    // Shortcodes.
    (":fire:", "🔥"),
    (":smile:", "😄"),
    (":+1:", "👍"),
    (":-1:", "👎"),
    (":100:", "💯"),
    (":piñata:", "🪅"),
    (":like:", "👍"),
    (":dislike:", "👎"),
    (":hmm:", "🤔"),
    (":party:", "🥳"),
    (":heart:", "❤️"),
    ("so :fire: today", "so 🔥 today"),
    ("Привет:fire:", "Привет🔥"),
    ("(:fire:)", "(🔥)"),
    ("::fire:", ":🔥"),
    (":Fire:", ":Fire:"),
    (":nope_not_a_name:", ":nope_not_a_name:"),
    (":fire :", ":fire :"),
    ("a:fire:", "a:fire:"),
    ("10:100:", "10:100:"),
    ("::", "::"),
    // A face right after a shortcode is its own swap.
    (":fire::)", "🔥😊"),
    // Taking it back.
    (":)⌫", ":)"),
    (":-)⌫", ":-)"),
    (":fire:⌫", ":fire:"),
    (":o ⌫", ":o "),
    (":)⌫⌫", ":"),
    (":)→⌫", ""),
    (":)⌫)", ":))"),
    // Code is typed as it is.
    ("`:)", "`:)"),
    ("`x` :)", "`x` 😊"),
    ("```\n:fire:", "```\n:fire:"),
    ("```\nx\n```\n:)", "```\nx\n```\n😊"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_design_table() {
        for &(keys, text) in TYPING_TABLE {
            assert_eq!(typing(keys), text, "typing {keys:?}");
        }
    }

    #[test]
    fn every_emoticon_is_in_the_table_once() {
        for (index, (text, _)) in EMOTICONS.iter().enumerate() {
            assert!(
                !EMOTICONS[..index]
                    .iter()
                    .any(|(earlier, _)| earlier == text),
                "{text:?} twice"
            );
            assert!(
                TYPING_TABLE.iter().any(|(keys, _)| keys == text),
                "{text:?} has no row in the design table"
            );
        }
    }

    #[test]
    fn the_names_load() {
        let count = NAMES_TSV
            .lines()
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .count();
        assert!(count > 6000, "{count} names");
        assert_eq!(emoji_named("fire"), Some("🔥"));
        assert_eq!(emoji_named(""), None);
        assert_eq!(emoji_named(":fire:"), None);
    }

    #[test]
    fn a_swap_lands_between_before_and_after() {
        let swap = replace("ok :", ')', None).unwrap();
        assert_eq!(
            swap,
            Swap {
                at: 3,
                put: "😊".into(),
                typed: ":)".into()
            }
        );
        assert_eq!(
            swap.apply("ok :", " bye"),
            ("ok 😊 bye".into(), 3 + "😊".len())
        );
        let (text, caret) = swap.apply("ok :", " bye");
        assert_eq!(swap.backspace(&text, caret), Some(("ok :) bye".into(), 5)));
        // Not right behind it: an ordinary Backspace.
        assert_eq!(swap.backspace(&text, caret + 1), None);
        assert_eq!(swap.backspace("ok x bye", caret), None);
    }

    #[test]
    fn a_swap_grows_only_from_where_it_started() {
        // A shortcode's closing colon and a `(` are not a `:(`.
        let fire = replace(":fire", ':', None).unwrap();
        let text = fire.apply(":fire", "").0;
        assert_eq!(replace(&text, '(', Some(&fire)), None);
    }
}
