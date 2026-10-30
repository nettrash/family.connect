//! What a note may SAY (docs/protocol.md, "Board"): the caps, counted the way the server counts
//! them, and the counter that only shows when it is worth showing — a port of `NoteText.cs`.
//!
//! The server counts UNICODE SCALARS (Rust's `chars()`), which is what a Rust `str` counts too:
//! the C# had to walk runes to stop `string.Length` (UTF-16 units) giving an emoji note half its
//! allowance, and a grapheme count would be SMALLER than the server's for a family emoji or a
//! combining mark — a note that looked under the cap would come back refused.
//!
//! The caps and the cuts are `fc_text::board`'s: `MAX_TEXT_CHARS`, `MAX_PLACE_CHARS`,
//! `MAX_TASK_ITEM_CHARS`, `MAX_TASK_ITEMS`, `COUNTER_FROM`, `capped`, `remaining`,
//! `shows_counter` and `cap_at_caret`. Apple: `NoteText`. Android: `NoteTexts`.
//!
//! `cap_at_caret`'s caret, in and out, is in UTF-16 units — what a browser's `selectionStart` and a
//! Windows text box report. A GTK editable's position and a text iter's offset count CHARACTERS
//! (scalars), so a GTK caller converts both ways; passed through as it is, the caret is off by one
//! for every emoji before it.

use fc_text::i18n::t2;

/// How a list says what is done of it, under its title — the sentence every client shows, in the
/// reader's language (`fc_text::i18n`).
pub fn done_of(done: usize, total: usize) -> String {
    t2("%lld of %lld done", &done.to_string(), &total.to_string())
}
