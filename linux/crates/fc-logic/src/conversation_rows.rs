//! Which of a conversation's drawn rows a redraw may KEEP, and how to turn the widgets on screen
//! into the ones wanted with the fewest insertions and removals (`ConversationRows.cs`; issue: the
//! conversation was rebuilt whole on every change).
//!
//! WHY. The conversation used to clear its stack and build every bubble again on any change it
//! noticed — a peer's read marker, a link's card landing, every chunk of an assistant answer being
//! streamed, each page of a catch-up. A chat paged back a few hundred messages was hundreds of
//! bubbles, their stickers, animation players and pictures, made again for one tick mark. Now a row
//! whose signature has not changed keeps the widgets it has, and only the rows that changed are
//! built. On Linux the timeline is a bounded, non-lazy `gtk::Box` of rows (shell-design §5.1), so
//! this is the same saving there.
//!
//! A ROW IS KEPT ONLY WHEN NOTHING IT WAS DRAWN FROM HAS CHANGED — the signature is the whole of
//! it, and the view decides what goes in (the message itself, its place in the timeline, the tick,
//! the card, the streamed words). What is not per row — the theme, the language, the block list,
//! the roster — is not in any row's signature: a change to it rebuilds every row.
//!
//! KEPT ROWS NEVER MOVE. A widget moved from one place in a box to another is unparented and
//! parented again, which for a sticker, a circle or an animation is a teardown and a start — the
//! very cost this is here to save. So the plan keeps only rows that stay in the order they were in,
//! and builds again any that would have to jump over another (which, for a conversation ordered by
//! message id, a redraw never asks for). New rows are inserted where they belong: below for what
//! arrived, above for a page of history, in place for a row that changed.
//!
//! No window in here: the widgets are a type parameter and the box is two callbacks, so the rule is
//! tested where no window exists.

use std::collections::{HashMap, HashSet};
use std::hash::Hash;

use fc_core::protocol::dtos::wire;

use crate::timeline::TimelineRow;

/// One row of a drawn conversation, as the redraw compares it: what it is, and everything its
/// drawing was made from.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RowMark {
    /// Which row: a message's, a pending send's, the empty chat's line. Unique within one drawing.
    pub key: String,
    /// Everything the row's widgets were built from. Two equal signatures draw the same widgets.
    pub signature: String,
}

impl RowMark {
    pub fn new(key: impl Into<String>, signature: impl Into<String>) -> Self {
        RowMark {
            key: key.into(),
            signature: signature.into(),
        }
    }
}

/// For each row wanted, the index of the drawn row it may keep, or `None` where it must be built
/// (the C#'s `-1`).
///
/// `keepable` says whether a drawn row may be kept at all, by its index: false for one whose
/// drawing never finished — a picture whose bytes did not come, a sticker that did not decode yet —
/// which, built again, would try again, as every redraw used to. `None` keeps any.
///
/// A key that appears twice on either side is never kept: which of the two a kept widget belonged
/// to could not be told.
pub fn plan(
    drawn: &[RowMark],
    wanted: &[RowMark],
    keepable: Option<&dyn Fn(usize) -> bool>,
) -> Vec<Option<usize>> {
    let mut plan = vec![None; wanted.len()];
    let mut at: HashMap<&str, usize> = HashMap::with_capacity(drawn.len());
    let mut twice: HashSet<&str> = HashSet::new();
    for (index, row) in drawn.iter().enumerate() {
        if at.contains_key(row.key.as_str()) {
            twice.insert(&row.key);
        } else {
            at.insert(&row.key, index);
        }
    }
    let mut wanted_keys: HashSet<&str> = HashSet::with_capacity(wanted.len());
    for row in wanted {
        if !wanted_keys.insert(&row.key) {
            twice.insert(&row.key);
        }
    }
    let mut last: Option<usize> = None;
    for (index, row) in wanted.iter().enumerate() {
        if twice.contains(row.key.as_str()) {
            continue;
        }
        let Some(&old) = at.get(row.key.as_str()) else {
            continue;
        };
        if drawn[old].signature != row.signature
            // Kept rows keep their order: one that would jump back over a row already kept is
            // built again instead.
            || last.is_some_and(|last| old <= last)
            || keepable.is_some_and(|keepable| !keepable(old))
        {
            continue;
        }
        plan[index] = Some(old);
        last = Some(old);
    }
    plan
}

/// Turn what a box holds into `wanted`: every widget not wanted removed, every wanted one not there
/// inserted where it belongs, and every widget kept left where it is — never removed and put back.
/// Answers how many insertions and removals that took, or `None` — the C#'s `-1` — having changed
/// nothing, when the widgets kept are not in the order wanted (the caller then clears the box and
/// adds them all, as it used to).
///
/// `T`'s equality must be IDENTITY: the same widget, not an equal one — what `glib::Object`'s `==`
/// and `Hash` are (the object pointer), and what the C# asked of `ReferenceEqualityComparer`.
///
/// `shown` is what the box holds, in order — the caller's own copy, kept in step here; `wanted` is
/// what it should hold, no widget twice; `remove_at` removes the box's child at an index and
/// `insert` inserts a widget as its child at an index.
pub fn merge<T: Clone + Eq + Hash>(
    shown: &mut Vec<T>,
    wanted: &[T],
    mut remove_at: impl FnMut(usize),
    mut insert: impl FnMut(usize, &T),
) -> Option<usize> {
    let mut wanted_set: HashSet<&T> = HashSet::with_capacity(wanted.len());
    for element in wanted {
        if !wanted_set.insert(element) {
            // The same widget twice cannot be in a box twice.
            return None;
        }
    }
    // Asked BEFORE anything is touched: what stays must already be in the wanted order, or keeping
    // it would mean moving it — and inserting a widget that is still in the box elsewhere fails.
    let mut next = 0;
    for element in shown.iter() {
        if !wanted_set.contains(element) {
            continue;
        }
        while next < wanted.len() && wanted[next] != *element {
            next += 1;
        }
        if next == wanted.len() {
            return None;
        }
        next += 1;
    }
    let mut changes = 0;
    // From the end, so the indices still to visit do not shift.
    for index in (0..shown.len()).rev() {
        if !wanted_set.contains(&shown[index]) {
            remove_at(index);
            shown.remove(index);
            changes += 1;
        }
    }
    for (index, element) in wanted.iter().enumerate() {
        if shown.get(index) == Some(element) {
            continue;
        }
        insert(index, element);
        shown.insert(index, element.clone());
        changes += 1;
    }
    Some(changes)
}

/// The day number .NET's `DateOnly.DayNumber` gives 0001-01-01.
const JULIAN_DAY_OF_DAY_NUMBER_ZERO: i32 = 1_721_426;

/// A character no message's JSON contains unescaped: a control character is escaped by the writer.
const SEPARATOR: char = '\u{1F}';

/// The part of a message row's signature that is the row's own: the message as the store holds it
/// — every field, so a trimmed list preview overwritten by the full message, an edit, a reaction, a
/// vote, a new reply count all change it — whether it is hidden, revealed or the reader's own, its
/// place in the timeline, and what the view adds (`drawn_with`: the tick, the card, the streamed
/// words — what the view reads to draw it).
///
/// The message is compared as the WIRE writes it, because a hand-picked list of fields is one field
/// short the day the protocol grows one.
pub fn signature(row: &TimelineRow, drawn_with: &str) -> String {
    let bubble = &row.bubble;
    let flag = |on: bool| if on { "1" } else { "0" };
    let day = row
        .day_above
        .map(|day| (day.to_julian_day() - JULIAN_DAY_OF_DAY_NUMBER_ZERO).to_string())
        .unwrap_or_default();
    let parts: [&str; 10] = [
        &wire::encode(&bubble.message),
        flag(bubble.hidden),
        flag(bubble.revealed),
        flag(bubble.mine),
        &day,
        flag(row.unread_divider_above),
        flag(row.shows_sender),
        flag(row.run_start),
        flag(row.run_end),
        drawn_with,
    ];
    parts.join(&SEPARATOR.to_string())
}

/// The key of a message's row.
pub fn message_key(message_id: i64) -> String {
    format!("m{message_id}")
}

/// The key of a send still on its way.
pub fn pending_key(client_msg_id: &str) -> String {
    format!("p{client_msg_id}")
}

/// The key of the line an empty chat draws.
pub const EMPTY_KEY: &str = "empty";
