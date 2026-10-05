//! An attachment on its way out: prepared in the composer, queued with its
//! message, uploaded by the outbox, and claimed by the send.
//!
//! Three shapes, because the three stages keep different things. A
//! [`Prepared`] item is what the composer holds — the bytes and what the
//! server will be told about them. Once its message is queued, the outbox
//! row keeps an [`OutgoingItem`] — the metadata, and the server's id once
//! the bytes have landed — and the store keeps the bytes beside it as
//! [`StagedBytes`], in this tab's memory only. The row survives a reload
//! (session.rs); the bytes deliberately do not (docs/protocol.md, "A
//! browser is a client too").

use serde::{Deserialize, Serialize};
use web_sys::Blob;

use crate::model::Attachment;

/// One attachment, prepared to send (the web's MediaPrep.Prepared).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Prepared {
    /// `photo` | `video` | `audio` | `file` | `location`.
    pub kind: String,
    /// What the upload declares — the type the server checks for a photo,
    /// a video or audio, and mere metadata for a file.
    pub mime: String,
    pub size: i64,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration_ms: Option<i64>,
    /// A file's name, which is its identity; a track's title; a place's
    /// label. A voice note has none.
    pub name: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub accuracy_m: Option<f64>,
    /// The bytes to upload. None for a location, which has none.
    pub file: Option<Blob>,
    /// The small JPEG a bubble draws: a photo's preview, a video's poster.
    pub preview: Option<Blob>,
    /// For a STICKER being sent: the pack picture these bytes are a copy of
    /// (docs/protocol.md, "Sending one"). Not sent anywhere — it is where a
    /// reload, which keeps no bytes, can fetch them from again.
    pub source_attachment_id: Option<i64>,
}

impl Prepared {
    /// A voice note recorded here — audio with no name, since its length is
    /// its identity (`prep::recording`) — as opposed to a sound file picked
    /// from disk, which keeps its name. The one the chip calls a voice note
    /// (views::attach::label), and the one a chat left with it in review
    /// keeps as not sent (store::Store::park_review).
    pub fn is_voice_note(&self) -> bool {
        self.kind == "audio" && self.name.is_none()
    }

    /// A place, decided now. No bytes: it IS its three numbers.
    pub fn location(latitude: f64, longitude: f64, accuracy_m: Option<f64>) -> Self {
        Prepared {
            kind: "location".into(),
            mime: String::new(),
            latitude: Some(latitude),
            longitude: Some(longitude),
            accuracy_m,
            ..Prepared::default()
        }
    }
}

/// One attachment of a queued message: what the upload will say, and the
/// server's id once the bytes are up. Kept across a reload with its row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutgoingItem {
    /// The id the pending bubble draws it under until the server's own
    /// arrives — negative, so it can never be mistaken for one, and never
    /// sent anywhere.
    pub provisional_id: i64,
    pub kind: String,
    pub mime: String,
    pub size: i64,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration_ms: Option<i64>,
    pub name: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub accuracy_m: Option<f64>,
    /// Whether a preview was made to go with it.
    pub has_preview: bool,
    /// The server's id, once the upload landed. An id is good for the
    /// server's unclaimed grace, so a retry never uploads it again.
    pub attachment_id: Option<i64>,
    /// A sticker's own source: the pack picture it is a copy of. The one
    /// kind of attachment whose bytes a reload does not lose for good —
    /// they are the family's, and the server still has them — so a queued
    /// sticker survives a reload where a queued photo cannot. None on
    /// everything else, and on a row kept by a build from before stickers.
    #[serde(default)]
    pub source_attachment_id: Option<i64>,
}

impl OutgoingItem {
    pub fn new(prepared: &Prepared, provisional_id: i64) -> Self {
        OutgoingItem {
            provisional_id,
            kind: prepared.kind.clone(),
            mime: prepared.mime.clone(),
            size: prepared.size,
            width: prepared.width,
            height: prepared.height,
            duration_ms: prepared.duration_ms,
            name: prepared.name.clone(),
            latitude: prepared.latitude,
            longitude: prepared.longitude,
            accuracy_m: prepared.accuracy_m,
            has_preview: prepared.preview.is_some(),
            attachment_id: None,
            source_attachment_id: prepared.source_attachment_id,
        }
    }

    /// Whether the bytes this item still owes can be had again after the
    /// tab has lost them: a location needs none, and a sticker's are the
    /// pack's.
    pub fn survives_reload(&self) -> bool {
        self.is_location() || self.source_attachment_id.is_some()
    }

    pub fn is_location(&self) -> bool {
        self.kind == "location"
    }

    /// What the pending bubble draws: this item, under its provisional id.
    pub fn as_attachment(&self) -> Attachment {
        Attachment {
            id: self.provisional_id,
            kind: self.kind.clone(),
            mime: (!self.mime.is_empty()).then(|| self.mime.clone()),
            size: Some(self.size),
            width: self.width,
            height: self.height,
            duration_ms: self.duration_ms,
            has_preview: self.has_preview,
            name: self.name.clone(),
            latitude: self.latitude,
            longitude: self.longitude,
            accuracy_m: self.accuracy_m,
            // The row's to say, not the item's: see `Store::enqueue`.
            sticker: false,
            round: false,
        }
    }
}

/// The bytes a queued attachment still has to upload. Held by the store,
/// by provisional id, for as long as the row is queued — and never written
/// anywhere.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StagedBytes {
    pub file: Option<Blob>,
    pub preview: Option<Blob>,
}
