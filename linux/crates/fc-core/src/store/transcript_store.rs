//! The text of recordings this member asked for, kept on this device (docs/protocol.md,
//! "Transcripts on request") — so reopening a chat shows what was already given without asking the
//! server again. A port of `TranscriptStore.cs`.
//!
//! **THIS DEVICE'S, AND NOBODY ELSE'S.** A transcript is the answer to one member's request: it is
//! in no `Attachment`, no history page and no frame, so nothing the wire delivers can fill or empty
//! this table. It goes with everything else at sign-out ([`Database::wipe_all`]), because the text
//! of somebody's voice belongs to the account that asked for it.
//!
//! **AN ANSWER FROM THE SERVER'S STORED BYTES IS NEVER REPLACED BY ONE FROM SUPPLIED SOUND.** The
//! stored one is the answer every member gets; the server itself returns it instead of reading
//! supplied sound once it exists.

use std::rc::Rc;

use rusqlite::{OptionalExtension, named_params};
use time::OffsetDateTime;

use super::database::{Database, StoreResult};
use super::times;

/// One recording's text, as this device was given it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KeptTranscript {
    /// The recording: an attachment id never names other bytes, so this cannot go stale.
    pub attachment_id: i64,
    /// What was said. `""` is silence — an answer, drawn as "No speech".
    pub text: String,
    /// The language the provider named, as it spelled it, or `None` when it named none.
    pub language: Option<String>,
    /// Made from sound THIS DEVICE sent, which the server never kept: this row is the only copy.
    pub supplied: bool,
}

impl KeptTranscript {
    /// `new KeptTranscript(attachmentId, text)`: no language, from the server's stored bytes.
    pub fn new(attachment_id: i64, text: &str) -> Self {
        KeptTranscript {
            attachment_id,
            text: text.to_string(),
            ..KeptTranscript::default()
        }
    }
}

/// The transcripts this device keeps.
#[derive(Debug)]
pub struct TranscriptStore {
    database: Rc<Database>,
}

impl TranscriptStore {
    pub fn new(database: &Rc<Database>) -> Self {
        TranscriptStore {
            database: Rc::clone(database),
        }
    }

    /// The text kept for this recording, or `None` when this device was never given any.
    pub fn find(&self, attachment_id: i64) -> StoreResult<Option<KeptTranscript>> {
        let _held = self.database.hold();
        Ok(self
            .database
            .connection()
            .prepare_cached(
                "SELECT text, language, supplied FROM transcripts WHERE attachment_id = $id",
            )?
            .query_row(named_params! { "$id": attachment_id }, |row| {
                Ok(KeptTranscript {
                    attachment_id,
                    text: row.get(0)?,
                    language: row.get(1)?,
                    supplied: row.get::<_, i64>(2)? != 0,
                })
            })
            .optional()?)
    }

    /// Keep this answer. An answer from the stored bytes already held is not replaced by a
    /// supplied one.
    pub fn keep(&self, transcript: &KeptTranscript, at: OffsetDateTime) -> StoreResult<()> {
        let _held = self.database.hold();
        self.database
            .connection()
            .prepare_cached(
                "INSERT INTO transcripts (attachment_id, text, language, supplied, kept_at) VALUES ($id, $text, $language, $supplied, $at) ON CONFLICT(attachment_id) DO UPDATE SET text = excluded.text, language = excluded.language, supplied = excluded.supplied, kept_at = excluded.kept_at WHERE transcripts.supplied = 1 OR excluded.supplied = 0",
            )?
            .execute(named_params! {
                "$id": transcript.attachment_id,
                "$text": transcript.text,
                "$language": transcript.language,
                "$supplied": transcript.supplied,
                "$at": times::milliseconds(at),
            })?;
        Ok(())
    }
}
