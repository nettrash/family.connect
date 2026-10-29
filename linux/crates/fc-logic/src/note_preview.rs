//! The sticker the note sheet shows under its fields: the draft as the wall will draw it, type
//! already fitted, so a size or a face is a choice with its result in front of the author (the web
//! client's sheet preview, docs/protocol.md "Board") — `NotePreview.cs`.
//!
//! IT IS A [`Sticker`], drawn by the wall's own code: a second, simpler renderer is a second place
//! for the rules to drift.
//!
//! Words not written yet read as the kind's placeholder. A photo has none, so an uncaptioned photo
//! is the bare picture with its card fitted to it — the shape the wall draws, or the author would
//! be shown one it never does. An event's picture is its backdrop; the preview is where its author
//! sees what arrived.

use fc_core::board::board_picture;
use fc_core::protocol::NoteDto;
use fc_core::text::string_catalog::StringCatalog;
use fc_text::board::Kind;

use crate::board::Sticker;
use crate::note_draft::{NoteDraft, wire_time};

/// The draft as the wall will draw it: `current` is the note being edited (`None` for a new one),
/// `reader` who is writing, `compact` whether the wall draws the phone's square cards.
pub fn of(
    draft: &NoteDraft,
    kind: Kind,
    current: Option<&NoteDto>,
    reader: i64,
    compact: bool,
    say: &dyn StringCatalog,
) -> Sticker {
    let text = if draft.text().trim().is_empty() {
        placeholder(kind, say)
    } else {
        draft.text().to_string()
    };
    let picture = if matches!(kind, Kind::Photo | Kind::Event) {
        current.and_then(|note| note.attachment.clone())
    } else {
        None
    };
    let mut card = draft.size.frame(compact);
    if kind == Kind::Photo
        && text.is_empty()
        && let Some(picture) = &picture
    {
        card = board_picture::fitted(card.0, card.1, picture.width, picture.height);
    }
    let is_event = kind == Kind::Event;
    let place = draft.place.trim();
    let note = NoteDto {
        kind: Some(kind.name().to_string()),
        text: Some(text),
        color: Some(draft.color.clone()),
        size: Some(draft.size.name().to_string()),
        font: Some(draft.font.name().to_string()),
        attachment: picture,
        starts_at: if is_event {
            wire_time(draft.starts)
        } else {
            None
        },
        ends_at: if is_event && draft.has_end {
            wire_time(draft.ends)
        } else {
            None
        },
        place: (is_event && !place.is_empty()).then(|| place.to_string()),
        rsvps: if is_event {
            current.and_then(|note| note.rsvps.clone())
        } else {
            None
        },
        ..NoteDto::new(
            current.map_or(0, |note| note.id),
            current.map_or(reader, |note| note.author_id),
        )
    };
    // Upright: a preview is looked at, not pinned.
    Sticker {
        note,
        kind,
        size: draft.size,
        font: draft.font,
        color: draft.color.clone(),
        x: 0.0,
        y: 0.0,
        width: card.0,
        height: card.1,
        tilt_degrees: 0,
        unread: false,
        mine: true,
    }
}

/// What an unwritten note says in its preview — nothing, for a photo.
pub fn placeholder(kind: Kind, say: &dyn StringCatalog) -> String {
    match kind {
        Kind::Photo => String::new(),
        Kind::Event => say.get("Your event").to_string(),
        Kind::Tasks => say.get("Your list").to_string(),
        Kind::Text => say.get("Your note").to_string(),
    }
}
