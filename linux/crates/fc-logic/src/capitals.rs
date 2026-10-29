//! Capital letters at the start of a sentence (issue #97; docs/capitals-and-rich-markdown-2026-10-17.md,
//! "The rule") — `Capitals.cs`. No desktop keyboard capitalises a sentence, and GNOME has no switch
//! for it either, so the app does it itself, always on, in the multi-line prose fields — the
//! composer, the thread reply, a note, a report's note (shell-design §7).
//!
//! THE RULE IS `fc_text::composer`'s, called rather than copied (port-plan: `fc-text` is a
//! dependency, not a source): [`capital_of`] is `composer::capital_of` and [`capitalizes`] is
//! `composer::capitalizes`, the original the Mac's `Capitals.swift` and Windows' `Capitals.cs` hold
//! to the same table. What this module adds is what the C# adds on top: the field's side of it —
//! which change was ONE typed character ([`typed_character`]), what the field holds instead
//! ([`after_typing`]) and the Backspace that takes the capital back ([`CapitalTakeBack`]).
//!
//! A field wires it in two places. When the text is about to change, [`after_typing`] says whether
//! the change was one typed letter that becomes a capital, and if so what to put in the field
//! instead; the field keeps the [`CapitalTakeBack`] it returns. When Backspace is pressed,
//! [`CapitalTakeBack::backspace`] says whether it takes that capital back. Anything else — another
//! key, a click, a paste, a selection — forgets the take-back.
//!
//! OFFSETS ARE UTF-16 UNITS into the field's own text, as the C# counts them and as every `fc-text`
//! caret rule the shell feeds does (carry-forward, "Caret units"): GTK counts characters, and the
//! shell converts at its edge. A text is a Rust `&str`, so it can never hold the lone surrogate a
//! WinUI `TextBox` can; an offset that falls inside a surrogate pair or past the end is a change
//! the rule does not recognise, exactly as the C#'s unit-by-unit comparison would not. A line break
//! may be a lone carriage return (`fc_text` reads one as a line break).

/// What a field holds after the rule capitalised a letter: its text, where the caret goes (UTF-16
/// units, right behind the capital), and how to take it back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapitalEdit {
    pub text: String,
    pub caret: usize,
    pub take_back: CapitalTakeBack,
}

/// A capital the rule just put in, remembered so the Backspace pressed straight after it can put
/// the lower-case letter back (docs/capitals-and-rich-markdown-2026-10-17.md, "Taking it back").
/// The field keeps one from the moment it capitalises until anything else happens — another key, a
/// click, a paste, a selection — and then drops it.
///
/// `fc_text::composer::TakeBack` is the same thing in BYTE offsets for the web's own text; this one
/// counts UTF-16 units, as the field's caret does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapitalTakeBack {
    /// Where the capital starts, in UTF-16 units.
    pub at: usize,
    /// The capital that was put in.
    pub capital: String,
    /// What was typed: the lower-case letter a Backspace puts back.
    pub typed: String,
}

impl CapitalTakeBack {
    /// Remember that `typed` went in at `at` as its capital, or `None` when it has none.
    pub fn after(at: usize, typed: &str) -> Option<CapitalTakeBack> {
        capital_of(typed).map(|capital| CapitalTakeBack {
            at,
            capital,
            typed: typed.to_string(),
        })
    }

    /// A Backspace with the caret at `caret`: the text with the lower-case letter back and the caret
    /// right behind it — or `None` when something is selected, the caret is not right behind the
    /// capital, or the capital is no longer there, and the Backspace is an ordinary one.
    pub fn backspace(
        &self,
        text: &str,
        caret: usize,
        selection_length: usize,
    ) -> Option<(String, usize)> {
        let end = self.at + utf16_len(&self.capital);
        if selection_length != 0 || caret != end {
            return None;
        }
        let (head, rest) = split_at_units(text, self.at)?;
        let tail = rest.strip_prefix(self.capital.as_str())?;
        let mut undone = String::with_capacity(text.len() + self.typed.len());
        undone.push_str(head);
        undone.push_str(&self.typed);
        undone.push_str(tail);
        Some((undone, self.at + utf16_len(&self.typed)))
    }
}

/// The capital the rule would put in place of `typed`, or `None`. `typed` must be ONE character —
/// one Unicode scalar (in the C#, one UTF-16 unit or a surrogate pair) — that is a lower-case letter
/// whose upper case (Rust's full mapping, the invariant one, never a locale's) is exactly one
/// character, and a different one. `ß` (upper case `SS`) has none. `fc_text::composer::capital_of`.
pub fn capital_of(typed: &str) -> Option<String> {
    one_character(typed)
        .and_then(fc_text::composer::capital_of)
        .map(String::from)
}

/// Whether `typed`, the one character the keyboard just produced, goes in as its capital. `before`
/// is the field's text before the caret (the selection's start). YES only when all four hold:
///
/// 1. `typed` has a capital ([`capital_of`]);
/// 2. a word is starting: with any opening marks typed just before it set aside, `before` is empty
///    or ends in a space, a tab or a line break;
/// 3. a sentence is starting: `before`, without those marks and trailing spaces and tabs, is empty,
///    ends in a line break, is nothing on its line but a list, heading or quote marker, or ends in
///    `.`, `!` or `?` (closing marks after it allowed) — where a `.` after a `.` (an ellipsis) or
///    ending an abbreviation (`e.g.`, `U.S.`) does not count;
/// 4. the caret is not in code: an even number of fence lines before it, and an even number of
///    backticks on its own line.
///
/// `fc_text::composer::capitalizes`, for a `typed` that is one character; anything else is not.
pub fn capitalizes(before: &str, typed: &str) -> bool {
    one_character(typed).is_some_and(|typed| fc_text::composer::capitalizes(before, typed))
}

/// The one character a change typed, as `(start, typed)` with `start` in UTF-16 units — or `None`
/// when it was anything else. Call it BEFORE the change lands, with the old text and selection
/// still at hand: the change typed `typed` at `start` exactly when `new_text` is `old_text` with its
/// selection replaced by one character.
///
/// The old selection is what makes the answer sound. Judged from the two texts and the caret alone,
/// a Backspace that takes one `a` out of `aab` looks exactly like typing `a` over a selected `aa` —
/// and would be capitalised.
pub fn typed_character(
    old_text: &str,
    selection_start: usize,
    selection_length: usize,
    new_text: &str,
) -> Option<(usize, String)> {
    // Unit for unit, as the C# compares them: the two texts as UTF-16, never sliced as UTF-8.
    let old: Vec<u16> = old_text.encode_utf16().collect();
    let new: Vec<u16> = new_text.encode_utf16().collect();
    let selection_end = selection_start.checked_add(selection_length)?;
    if selection_end > old.len() {
        return None;
    }
    let tail = old.len() - selection_end;
    let typed_length = new.len().checked_sub(selection_start + tail)?;
    if !matches!(typed_length, 1 | 2)
        || new[..selection_start] != old[..selection_start]
        || new[new.len() - tail..] != old[old.len() - tail..]
    {
        return None;
    }
    // One character: one unit that is not half of a pair, or a whole surrogate pair. A Rust string
    // cannot hold half of one, so a typed slice that splits a pair simply does not decode — the
    // C#'s `char.IsSurrogate` check, by another road.
    let typed = String::from_utf16(&new[selection_start..selection_start + typed_length]).ok()?;
    one_character(&typed)?;
    Some((selection_start, typed))
}

/// What a field holds instead of `new_text` when the change the rule watches for happened — one
/// typed letter, at a sentence's start — or `None` when the change stands as it is. Arguments as
/// [`typed_character`]. The edit's `caret` is right behind the capital; its `take_back` is what the
/// field keeps for the next Backspace.
pub fn after_typing(
    old_text: &str,
    selection_start: usize,
    selection_length: usize,
    new_text: &str,
) -> Option<CapitalEdit> {
    let (start, typed) = typed_character(old_text, selection_start, selection_length, new_text)?;
    let (before, rest) = split_at_units(new_text, start)?;
    if !capitalizes(before, &typed) {
        return None;
    }
    let take_back = CapitalTakeBack::after(start, &typed)?;
    let after = rest.strip_prefix(typed.as_str())?;
    let mut text = String::with_capacity(new_text.len() + 4);
    text.push_str(before);
    text.push_str(&take_back.capital);
    text.push_str(after);
    let caret = start + utf16_len(&take_back.capital);
    Some(CapitalEdit {
        text,
        caret,
        take_back,
    })
}

/// The one scalar `text` is, or `None` for none or several.
fn one_character(text: &str) -> Option<char> {
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(only), None) => Some(only),
        _ => None,
    }
}

// ---- UTF-16 offsets over UTF-8 text (shared with emoji_typing and capital_field) ----------------

/// How many UTF-16 units `text` is: the length a WinUI `TextBox` — and every caret this module
/// takes — counts.
pub(crate) fn utf16_len(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

/// The byte offset of the UTF-16 offset `units` in `text`, found by a char walk — `None` when it
/// falls inside a surrogate pair or past the end, where no character boundary is.
pub(crate) fn byte_at(text: &str, units: usize) -> Option<usize> {
    let mut counted = 0;
    for (byte, character) in text.char_indices() {
        if counted == units {
            return Some(byte);
        }
        if counted > units {
            return None;
        }
        counted += character.len_utf16();
    }
    (counted == units).then_some(text.len())
}

/// `text` cut at the UTF-16 offset `units`, or `None` when that is no character boundary.
pub(crate) fn split_at_units(text: &str, units: usize) -> Option<(&str, &str)> {
    byte_at(text, units).map(|byte| text.split_at(byte))
}
