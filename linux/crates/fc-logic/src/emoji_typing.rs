//! Emoji as you type (issue #101; docs/emoji-as-you-type-2026-10-18.md) — `EmojiTyping.cs`:
//! Telegram's emoticons — `:-)`, `<3`, `;-P` — with their no-nose forms, and `:name:` shortcodes,
//! become emoji in the field the moment the last character goes in, and a Backspace straight after
//! takes the replacement back.
//!
//! THE RULE IS `fc_text::emoji_typing`'s — the original every port is held to (board-oracle's
//! `emoji-typing` vectors) — called, not copied: its table ([`EMOTICONS`]), its shortcode names
//! ([`emoji_named`]) and [`replace`]'s four steps. What this module adds is what the C# adds: the
//! field's offsets in UTF-16 units, where the Rust counts UTF-8 bytes (carry-forward, "Caret
//! units"), so the shell hands the same carets to this rule as to #97's capitals.
//!
//! Every prose field runs it, single-line ones included, for one typed character at a time: never
//! on a paste, never in code. The text that is sent holds the emoji itself, so nothing about it is
//! on the wire. Every guard is ASCII on purpose: the four languages' "is a letter" disagree past it.

use crate::capitals::{byte_at, split_at_units, utf16_len};

/// The emoticons, as typed, and the emoji each becomes — in the original's order
/// (`fc_text::emoji_typing::EMOTICONS`). The first 27 are Telegram Desktop's own table, emoji for
/// emoji; the rest are the owner's addition (2026-10-18): the no-nose forms of the same faces, so
/// `:)` works, and the case twins Telegram's case-sensitive table misses. `8)` and `B)` are left
/// out on purpose. An entry ending in a space is replaced when that space is typed, and the space
/// stays: `:o` and `:p` would otherwise eat the start of `:ok_hand:` and `:pizza:`, and `xD` of
/// `xDDD`.
pub const EMOTICONS: &[(&str, &str)] = fc_text::emoji_typing::EMOTICONS;

/// The emoji `name` (no colons) stands for, or `None`. Names are matched exactly: `:Fire:` is not
/// `:fire:`. `fc_text::emoji_typing::emoji_named` — the very table the web client embeds
/// (`web/text/data/emoji-names.tsv`), so there is no copy here to drift; a name listed twice keeps
/// its last line, as that table's `HashMap` collect does.
///
/// The C#'s `NameCount` (there for its tests) has no counterpart: `fc_text` keeps the table
/// private, so the tests count the file it embeds instead.
pub fn emoji_named(name: &str) -> Option<&'static str> {
    fc_text::emoji_typing::emoji_named(name)
}

/// A replacement the rule just made ([`replace`]), kept so the Backspace pressed straight after it
/// can take it back — and so the next key can grow it, `:(` and another `(` being `:((`. A field
/// keeps one from the moment the rule replaces until anything else happens — a key the rule does
/// not act on, a click, a paste, a selection — and then drops it, exactly like #97's capital.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmojiSwap {
    /// Where the replacement starts, in UTF-16 units.
    pub at: usize,
    /// What was put in: the emoji, and the space after it for the emoticons that end in one.
    pub put: String,
    /// What was typed, all of it: what a Backspace puts back.
    pub typed: String,
}

impl EmojiSwap {
    pub fn new(at: usize, put: &str, typed: &str) -> Self {
        EmojiSwap {
            at,
            put: put.to_string(),
            typed: typed.to_string(),
        }
    }

    /// The field's text and caret (UTF-16 units) after the swap, given what was before the caret
    /// (without the typed character) and what is after it.
    ///
    /// A swap is only ever applied to the `before` it was made from. Should `at` not fall on a
    /// character of `before` (the C# would throw past its end), the swap starts at the nearest
    /// character boundary before it rather than panicking.
    pub fn apply(&self, before: &str, after: &str) -> (String, usize) {
        let head = &before[..floor_byte_at(before, self.at)];
        let mut text = String::with_capacity(head.len() + self.put.len() + after.len());
        text.push_str(head);
        text.push_str(&self.put);
        let caret = utf16_len(&text);
        text.push_str(after);
        (text, caret)
    }

    /// A Backspace with the caret at `caret` and nothing selected: the text with what was typed put
    /// back, and the caret right behind it — or `None` when the caret is not right behind the
    /// replacement or the replacement is no longer there, and the Backspace is an ordinary one.
    pub fn backspace(&self, text: &str, caret: usize) -> Option<(String, usize)> {
        if caret != self.at + utf16_len(&self.put) {
            return None;
        }
        let (head, rest) = split_at_units(text, self.at)?;
        let tail = rest.strip_prefix(self.put.as_str())?;
        let mut undone = String::with_capacity(text.len() + self.typed.len());
        undone.push_str(head);
        undone.push_str(&self.typed);
        undone.push_str(tail);
        Some((undone, self.at + utf16_len(&self.typed)))
    }
}

/// What typing `typed` turns into, or `None` when the character goes in as it is. `before` is the
/// field's text before the caret, without `typed`; `typed` is the ONE character the keyboard
/// produced — after capitals (#97) decided it; `live` is the swap the field still holds, if any. In
/// order (`fc_text::emoji_typing::replace`):
///
/// 1. nothing in code (the test #97's capitals use);
/// 2. GROWING: the caret is right behind `live`'s replacement, and what was typed then plus `typed`
///    is a longer emoticon starting at the same place — `:(` then `(` is `:((`, one swap whose
///    Backspace puts back all three characters;
/// 3. a SHORTCODE, when `typed` is `:` — `before` ends in `:name` with no whitespace or colon in
///    the name, the opening colon is at the start or after something that is not an ASCII letter or
///    digit (so `10:30:` and `a:b:` are left alone), and the table knows the name;
/// 4. an EMOTICON: `before` plus `typed` ends in one, the longest whose guard passes — one starting
///    with `:` or `;` must not follow an ASCII digit (`1:]`), any other must not follow an ASCII
///    letter or digit (`x<3`, `item3(`). `HELLO:)` is `:)` after `HELL`'s `O`, because the longer
///    `O:)` fails its guard.
///
/// A `typed` that is not exactly one character makes nothing (the C# documents one character and
/// is only ever given one).
pub fn replace(before: &str, typed: &str, live: Option<&EmojiSwap>) -> Option<EmojiSwap> {
    let mut chars = typed.chars();
    let (Some(typed), None) = (chars.next(), chars.next()) else {
        return None;
    };
    // The live swap in the original's bytes. One whose start is no character of `before` cannot be
    // right behind the caret, so it grows nothing — the C#'s unit comparison fails the same way.
    let live = live.and_then(|live| {
        Some(fc_text::emoji_typing::Swap {
            at: byte_at(before, live.at)?,
            put: live.put.clone(),
            typed: live.typed.clone(),
        })
    });
    let swap = fc_text::emoji_typing::replace(before, typed, live.as_ref())?;
    // Its start is a byte offset into `before` + `typed`, on a character boundary of it.
    let mut typed_text = String::with_capacity(before.len() + typed.len_utf8());
    typed_text.push_str(before);
    typed_text.push(typed);
    let at = utf16_len(typed_text.get(..swap.at)?);
    Some(EmojiSwap {
        at,
        put: swap.put,
        typed: swap.typed,
    })
}

/// Types `keys` into an empty field with the caret always at the end, the way every port's vectors
/// are checked: `⌫` is a Backspace (the live swap taken back, or else the last character removed),
/// `→` is "something else happened" (a click, a caret move — the live swap is dropped), and every
/// other character is typed. Written over this module's own UTF-16 API — not
/// `fc_text::emoji_typing::typing` — so the vectors check the offsets the field uses.
pub fn typing(keys: &str) -> String {
    let mut text = String::new();
    let mut live: Option<EmojiSwap> = None;
    for key in keys.chars() {
        match key {
            '⌫' => {
                let caret = utf16_len(&text);
                if let Some((undone, _)) =
                    live.as_ref().and_then(|swap| swap.backspace(&text, caret))
                {
                    text = undone;
                } else {
                    // One character: what a surrogate pair is in the C#.
                    text.pop();
                }
                live = None;
            }
            '→' => live = None,
            _ => {
                let mut typed = [0; 4];
                match replace(&text, key.encode_utf8(&mut typed), live.as_ref()) {
                    Some(swap) => {
                        text = swap.apply(&text, "").0;
                        live = Some(swap);
                    }
                    None => {
                        text.push(key);
                        live = None;
                    }
                }
            }
        }
    }
    text
}

/// The byte offset of the UTF-16 offset `units` in `text`, or of the last character boundary
/// before it when it falls inside a pair or past the end.
fn floor_byte_at(text: &str, units: usize) -> usize {
    let mut counted = 0;
    for (byte, character) in text.char_indices() {
        if counted + character.len_utf16() > units {
            return byte;
        }
        counted += character.len_utf16();
    }
    text.len()
}
