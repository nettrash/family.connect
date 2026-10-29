//! Recordings THIS DEVICE has played, by attachment, in one table per kind — what the small accent
//! dot beside an unplayed one is drawn from, and nothing else (docs/audio-video-messages-2026-10-04.md,
//! S5.2; the voice note's dot is the same rule, drawn from the approved design of 2026-10-05) — a
//! port of `PlayedRoundStore.cs`.
//!
//! **THE DEVICE'S OWN KNOWLEDGE.** Never sent: whether somebody listened to or watched something is
//! theirs, and the wire has no field for it. Kept in the cache file, so it is this account's and
//! goes with everything else at sign-out ([`Database::wipe_all`]).
//!
//! **THE NEWEST [`PlayedStore::KEPT`], AND NO MORE.** Attachment ids only grow, so the highest ids
//! are the newest; marking one more past the limit lets the oldest go — which draws a dot again on
//! something nobody will scroll back to.
//!
//! The C#'s abstract `PlayedStore` and its two sealed kinds are one [`PlayedStore`] over a table
//! and two thin types that name the table and dereference to it.

use std::fmt;
use std::ops::Deref;
use std::rc::Rc;

use rusqlite::{OptionalExtension, named_params};

use super::database::{Database, StoreResult};

/// Recordings of one kind this device has played.
pub struct PlayedStore {
    database: Rc<Database>,
    /// The table: one of the two below, never anything a caller supplies.
    table: &'static str,
}

impl fmt::Debug for PlayedStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PlayedStore")
            .field("table", &self.table)
            .finish_non_exhaustive()
    }
}

impl PlayedStore {
    /// How many played recordings of one kind are remembered (S5.2).
    pub const KEPT: usize = 5000;

    fn over(database: &Rc<Database>, table: &'static str) -> Self {
        PlayedStore {
            database: Rc::clone(database),
            table,
        }
    }

    /// Whether this device has played this recording.
    pub fn played(&self, attachment_id: i64) -> StoreResult<bool> {
        let _held = self.database.hold();
        Ok(self
            .database
            .connection()
            .prepare_cached(&format!(
                "SELECT 1 FROM {} WHERE attachment_id = $id",
                self.table
            ))?
            .query_row(named_params! { "$id": attachment_id }, |_| Ok(()))
            .optional()?
            .is_some())
    }

    /// Remember that this recording was played here, and forget the oldest past
    /// [`KEPT`](Self::KEPT). Answers whether it was news — false when it had been played already,
    /// so a caller redraws only what changed.
    pub fn mark_played(&self, attachment_id: i64) -> StoreResult<bool> {
        let _held = self.database.hold();
        let transaction = self.database.connection().unchecked_transaction()?;
        let added = transaction
            .prepare_cached(&format!(
                "INSERT OR IGNORE INTO {} (attachment_id) VALUES ($id)",
                self.table
            ))?
            .execute(named_params! { "$id": attachment_id })?
            > 0;
        if added {
            transaction
                .prepare_cached(&format!(
                    "DELETE FROM {table} WHERE attachment_id NOT IN (SELECT attachment_id FROM {table} ORDER BY attachment_id DESC LIMIT $kept)",
                    table = self.table
                ))?
                .execute(named_params! { "$kept": PlayedStore::KEPT })?;
        }
        transaction.commit()?;
        Ok(added)
    }

    /// How many are remembered — never more than [`KEPT`](Self::KEPT).
    pub fn count(&self) -> StoreResult<usize> {
        let _held = self.database.hold();
        let count: i64 = self
            .database
            .connection()
            .prepare_cached(&format!("SELECT COUNT(*) FROM {}", self.table))?
            .query_row([], |row| row.get(0))?;
        Ok(usize::try_from(count).unwrap_or(0))
    }
}

/// The video messages this device has played: the dot beside an unplayed circle (S5.2).
#[derive(Debug)]
pub struct PlayedRoundStore(PlayedStore);

impl PlayedRoundStore {
    pub fn new(database: &Rc<Database>) -> Self {
        PlayedRoundStore(PlayedStore::over(database, "played_rounds"))
    }
}

impl Deref for PlayedRoundStore {
    type Target = PlayedStore;

    fn deref(&self) -> &PlayedStore {
        &self.0
    }
}

/// The voice messages this device has played: the dot beside an unplayed voice bubble — never on
/// the reader's own, which the caller decides (`VoiceLook.ShowsDot`). A table of its own, so a busy
/// chat of circles cannot push the voice notes out.
#[derive(Debug)]
pub struct PlayedVoiceStore(PlayedStore);

impl PlayedVoiceStore {
    pub fn new(database: &Rc<Database>) -> Self {
        PlayedVoiceStore(PlayedStore::over(database, "played_voice"))
    }
}

impl Deref for PlayedVoiceStore {
    type Target = PlayedStore;

    fn deref(&self) -> &PlayedStore {
        &self.0
    }
}
