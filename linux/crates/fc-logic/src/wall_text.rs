//! What a sticker on the wall says and hides — the web client's board pane
//! (`web/src/views/board.rs`), which is the Mac's (`WallText.cs`).
//!
//! A NOTE A BLOCK HIDES stays on the wall — its slot is part of the family's shared layout — but
//! says nothing of what it holds, not even who wrote it, until the reader peeks.
//!
//! A BARE PHOTO IS THE PICTURE: no paper under it, so no author line and no words.

use std::collections::{HashMap, HashSet};

use fc_core::protocol::NoteDto;
use fc_core::text::string_catalog::StringCatalog;
use fc_text::board::Kind;

use crate::board::Sticker;

/// A note's words, trimmed (`str::trim` — the same White_Space set as .NET's `Trim`).
pub fn caption_of(note: &NoteDto) -> String {
    note.text.as_deref().unwrap_or("").trim().to_string()
}

/// Hidden while its author is somebody this reader blocked, until they peek — never their own.
pub fn is_hidden(
    note: &NoteDto,
    me: i64,
    blocked: impl Fn(i64) -> bool,
    revealed: &HashSet<i64>,
) -> bool {
    note.author_id != me && blocked(note.author_id) && !revealed.contains(&note.id)
}

/// Who wrote it, as the sticker signs it: "You", the name the roster holds, or "Someone".
pub fn author_name(
    note: &NoteDto,
    me: i64,
    name_of: impl Fn(i64) -> Option<String>,
    say: &dyn StringCatalog,
) -> String {
    if note.author_id == me {
        return say.get("You").to_string();
    }
    match name_of(note.author_id) {
        Some(name) if !name.is_empty() => name,
        _ => say.get("Someone").to_string(),
    }
}

pub fn shows_text(kind: Kind, caption: &str, hidden: bool) -> bool {
    hidden || kind != Kind::Photo || !caption.is_empty()
}

pub fn shows_author(kind: Kind, caption: &str, hidden: bool) -> bool {
    !(hidden || (kind == Kind::Photo && caption.is_empty()))
}

/// What the card holds, in words — its accessibility label stands in for the content, so an
/// event's when, where and who is coming have to be in it too.
pub fn what(
    kind: Kind,
    caption: &str,
    when: Option<&str>,
    place: Option<&str>,
    going: Option<&str>,
    say: &dyn StringCatalog,
) -> String {
    let mut what = if kind == Kind::Photo && caption.is_empty() {
        say.get("a photo").to_string()
    } else {
        caption.to_string()
    };
    if kind == Kind::Event
        && let Some(when) = when.filter(|when| !when.is_empty())
    {
        what = format!("{what}, {when}");
        if let Some(place) = place.filter(|place| !place.is_empty()) {
            what = format!("{what}, {place}");
        }
        if let Some(going) = going.filter(|going| !going.is_empty()) {
            what = format!("{what}, {going}");
        }
    }
    what
}

pub fn label(
    hidden: bool,
    mine: bool,
    author: &str,
    what: &str,
    say: &dyn StringCatalog,
) -> String {
    if hidden {
        say.get("Hidden note from a blocked member").to_string()
    } else if mine {
        say.format("Your note: %@", &[&what])
    } else {
        say.format("Note from %@: %@", &[&author, &what])
    }
}

/// How many lines of a list the sticker had to leave off.
pub fn more_line(left: i64, say: &dyn StringCatalog) -> String {
    say.plural("+%lld more", left, &[&left])
}

/// Where each sticker stacks: the most recently changed highest. Drawn in a STABLE order (by id)
/// with the recency as a z-index, so a note somebody else touched does not reshuffle the wall under
/// a reader.
pub fn layers(newest_first: &[Sticker]) -> HashMap<i64, usize> {
    newest_first
        .iter()
        .enumerate()
        .map(|(at, sticker)| (sticker.note.id, newest_first.len() - at))
        .collect()
}
