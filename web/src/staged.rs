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
}

impl Prepared {
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
        }
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
