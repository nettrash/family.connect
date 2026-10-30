//! What the board's badge counts (docs/protocol.md, "Board"): notes with something NEW TO READ,
//! which is not the same as notes that have changed — a port of `BoardBadge.cs`.
//!
//! A note carries two sequences: `board_seq` moves on every change, and `content_seq` only when
//! what the note SAYS changes. The badge counts the second — a note somebody dragged across the
//! wall, recoloured, resized or answered is not a note with news in it, and a badge claiming
//! otherwise is a lie the family learns to ignore.
//!
//! The marks are what this DEVICE has shown its reader, which is not the sync cursor. They move
//! only when the board has actually been on screen.
//!
//! The rules themselves are `fc_text::board`'s: [`Marks`] (`BoardMarks`, `Marks::default()` its
//! `Zero`), [`is_unread`], `marks_after_showing` (`AfterShowing`) and `later` (`BoardMarks.Later`).
//! Only the count is here. Apple: `BoardBadge`.

use fc_text::board::{Marks, is_unread};

/// How many notes on this board have something new to read: each `(note_id, content_seq)` judged
/// by [`is_unread`] — by its content seq when it has one, and by its id when it does not.
pub fn count(notes: impl IntoIterator<Item = (i64, Option<i64>)>, marks: Marks) -> usize {
    notes
        .into_iter()
        .filter(|&(note_id, content_seq)| is_unread(note_id, content_seq, marks))
        .count()
}
