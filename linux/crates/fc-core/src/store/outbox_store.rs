//! What has not been said yet: the send queue, which is the one table holding something the
//! server has never seen — a port of `OutboxStore.cs`.
//!
//! It survives the app closing, on purpose — that is the whole point of writing a send down before
//! the first byte moves, so an interrupted send is a bubble that can be finished rather than
//! nothing at all.

use std::rc::Rc;

use rusqlite::{Connection, Row, named_params};
use time::OffsetDateTime;

use super::database::{Database, StoreResult};
use super::times;
use crate::protocol::send_rules::{self, Outcome};
use crate::protocol::{ApiError, MentionDto, ReconnectBackoff, wire};

/// One message this device has been given and the server has not acknowledged.
///
/// The `client_msg_id` is the row's identity AND the wire's dedup key, which is what makes a repeat
/// safe. `next_attempt_at` `None` means due now — a row somebody has just asked to retry.
/// `failed_code` is set only on a TERMINAL refusal, and a row with one is the only kind the user is
/// told about (docs/protocol.md, "Sending on an unreliable network").
#[derive(Debug, Clone, PartialEq, Default)]
pub struct OutboxRow {
    pub client_msg_id: String,
    pub chat_id: i64,
    pub body: String,
    pub reply_to_message_id: Option<i64>,
    pub attachment_ids: Option<Vec<i64>>,
    pub pending_files: Option<Vec<String>>,
    /// Every file this send's media came from. NEVER SHRINKS, unlike
    /// [`pending_files`](Self::pending_files): `attachment_expired` means upload it again, and a
    /// row that had forgotten where the bytes came from could only give up.
    pub staged_files: Option<Vec<String>>,
    pub poll_options: Option<Vec<String>>,
    pub mentions: Option<Vec<MentionDto>>,
    /// When the send was written down. `None` (the C#'s `default`) is "now" to
    /// [`OutboxStore::queue`]; a row read back always has one.
    pub queued_at: Option<OffsetDateTime>,
    pub attempts: i32,
    pub next_attempt_at: Option<OffsetDateTime>,
    pub failed_code: Option<String>,
    /// This send is a STICKER (docs/protocol.md, "Sending one"): its one attachment goes up as the
    /// bytes it is — never prepared, never with a preview — and the message says `sticker: true`.
    /// Written down with the row, because a sticker tapped while offline is still a sticker when
    /// the app has been closed and opened again before it lands.
    pub sticker: bool,
    /// This send is a VIDEO MESSAGE (docs/protocol.md, "Video messages"): its one video goes up
    /// with its square poster, and the message says `round: true`. Written down with the row for
    /// the sticker's reason — a circle recorded offline is still a circle when the app has been
    /// closed and opened again before it lands, never a square video.
    pub round: bool,
    /// This send is an ANIMATION (docs/protocol.md, "Animations"): its one video is an animated
    /// picture made into a silent looping MP4, and the message says `animation: true`. Written down
    /// with the row for the sticker's reason — an animation queued offline is still one when the
    /// app has been closed and opened again before it lands.
    pub animation: bool,
}

impl OutboxRow {
    /// `new OutboxRow(clientMsgId, chatId, body)`, the rest at their defaults.
    pub fn new(client_msg_id: &str, chat_id: i64, body: &str) -> Self {
        OutboxRow {
            client_msg_id: client_msg_id.to_string(),
            chat_id,
            body: body.to_string(),
            ..OutboxRow::default()
        }
    }

    /// Shown as failed, with a retry affordance — and nothing else is coming.
    pub fn failed(&self) -> bool {
        self.failed_code.is_some()
    }

    /// Whether this row still owes an upload. Until every one has landed the row must NEVER be
    /// posted: a message claiming no attachments is a text message, and the server would accept it
    /// happily, leaving a delivered bubble with the pictures gone.
    pub fn owes_uploads(&self) -> bool {
        self.pending_files
            .as_ref()
            .is_some_and(|files| !files.is_empty())
    }

    /// Whether this device can still recover this row's media — which is what decides whether an
    /// expired upload is "push it again" or "tell the person it is gone".
    pub fn holds_bytes(&self) -> bool {
        self.staged_files
            .as_ref()
            .is_some_and(|files| !files.is_empty())
            || self.attachment_ids.is_none()
    }
}

/// The send queue.
#[derive(Debug)]
pub struct OutboxStore {
    database: Rc<Database>,
}

impl OutboxStore {
    pub fn new(database: &Rc<Database>) -> Self {
        OutboxStore {
            database: Rc::clone(database),
        }
    }

    fn connection(&self) -> &Connection {
        self.database.connection()
    }

    /// Write a send down — before the first byte of it moves.
    pub fn queue(&self, row: &OutboxRow) -> StoreResult<()> {
        let _held = self.database.hold();
        let queued = row.queued_at.unwrap_or_else(OffsetDateTime::now_utc);
        // Staged is what was written down; pending is what is still owed. A caller that names only
        // the pending files at queue time is naming all of them.
        let staged = row.staged_files.as_ref().or(row.pending_files.as_ref());
        self.connection()
            .prepare_cached(
                "INSERT INTO outbox (client_msg_id, chat_id, body, reply_to_id, attachment_ids,
                    pending_files, staged_files, poll_json, mentions_json, queued_at,
                    attempts, next_attempt_at, failed_code, sticker, round, animation)
VALUES ($id, $chat, $body, $reply, $attachments, $files, $staged, $poll, $mentions,
        $queued, $attempts, $next, $failed, $sticker, $round, $animation)
ON CONFLICT(client_msg_id) DO UPDATE SET
    body = excluded.body, attachment_ids = excluded.attachment_ids,
    pending_files = excluded.pending_files, attempts = excluded.attempts,
    next_attempt_at = excluded.next_attempt_at, failed_code = excluded.failed_code",
            )?
            .execute(named_params! {
                "$id": row.client_msg_id,
                "$chat": row.chat_id,
                "$body": row.body,
                "$reply": row.reply_to_message_id,
                "$attachments": row.attachment_ids.as_ref().map(wire::encode),
                "$files": row.pending_files.as_ref().map(wire::encode),
                "$staged": staged.map(wire::encode),
                "$poll": row.poll_options.as_ref().map(wire::encode),
                "$mentions": row.mentions.as_ref().map(wire::encode),
                "$queued": times::milliseconds(queued),
                "$attempts": row.attempts,
                "$next": row.next_attempt_at.map(times::milliseconds),
                "$failed": row.failed_code,
                "$sticker": row.sticker,
                "$round": row.round,
                "$animation": row.animation,
            })?;
        Ok(())
    }

    /// Every queued row, oldest first — the order they were written in.
    pub fn all(&self) -> StoreResult<Vec<OutboxRow>> {
        let _held = self.database.hold();
        let rows = self
            .connection()
            .prepare_cached("SELECT * FROM outbox ORDER BY queued_at, rowid")?
            .query_map([], read)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// One chat's queue, for the pending bubbles under its last message.
    pub fn for_chat(&self, chat_id: i64) -> StoreResult<Vec<OutboxRow>> {
        let _held = self.database.hold();
        let rows = self
            .connection()
            .prepare_cached("SELECT * FROM outbox WHERE chat_id = $chat ORDER BY queued_at, rowid")?
            .query_map(named_params! { "$chat": chat_id }, read)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// What to send now: the rows that are due and not failed, oldest first. A row that owes
    /// uploads is included — the caller finishes the uploads and then posts it, which is the one
    /// order that cannot deliver a message without its pictures.
    pub fn due(&self, now: OffsetDateTime) -> StoreResult<Vec<OutboxRow>> {
        let _held = self.database.hold();
        Ok(self
            .all()?
            .into_iter()
            .filter(|row| !row.failed() && send_rules::is_due(row.next_attempt_at, now))
            .collect())
    }

    /// One queued row, or `None`.
    pub fn find(&self, client_msg_id: &str) -> StoreResult<Option<OutboxRow>> {
        let _held = self.database.hold();
        let rows = self
            .connection()
            .prepare_cached("SELECT * FROM outbox WHERE client_msg_id = $id")?
            .query_map(named_params! { "$id": client_msg_id }, read)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows.into_iter().next())
    }

    /// The send landed: the row goes, and the message itself arrives through the ack or the feed.
    /// Idempotent — an ack that crossed a REST answer for the same id is ordinary.
    pub fn delivered(&self, client_msg_id: &str) -> StoreResult<bool> {
        let _held = self.database.hold();
        let deleted = self
            .connection()
            .prepare_cached("DELETE FROM outbox WHERE client_msg_id = $id")?
            .execute(named_params! { "$id": client_msg_id })?;
        Ok(deleted > 0)
    }

    /// One failed attempt, judged by [`send_rules::verdict`]: queued again with a delay, or shown
    /// failed. Answers what was decided, so the caller can tell the user only when it is final.
    /// (The C#'s default `holds_bytes` is `true`.)
    pub fn failed(
        &self,
        client_msg_id: &str,
        error: &ApiError,
        now: OffsetDateTime,
        backoff: &mut ReconnectBackoff,
        holds_bytes: bool,
    ) -> StoreResult<Outcome> {
        let _held = self.database.hold();
        let Some(row) = self.find(client_msg_id)? else {
            // Delivered by the other path while this attempt was in flight. Nothing to do — and
            // certainly not a failure to show.
            return Ok(Outcome::Retry);
        };
        let attempts = row.attempts + 1;
        let (what, next) = send_rules::verdict(error, attempts, now, backoff, holds_bytes);
        self.connection()
            .prepare_cached(
                "UPDATE outbox SET attempts = $attempts, next_attempt_at = $next, failed_code = $failed WHERE client_msg_id = $id",
            )?
            .execute(named_params! {
                "$attempts": attempts,
                "$next": next.map(times::milliseconds),
                "$failed": (what == Outcome::Failed).then_some(error.code.as_str()),
                "$id": client_msg_id,
            })?;
        Ok(what)
    }

    /// The ids that DID land are kept and reused within the grace, so a retry pushes only the
    /// remainder — and the files still owed shrink as they go.
    pub fn uploaded(&self, client_msg_id: &str, attachment_id: i64, file: &str) -> StoreResult<()> {
        let _held = self.database.hold();
        let Some(row) = self.find(client_msg_id)? else {
            return Ok(());
        };
        let mut ids = row.attachment_ids.unwrap_or_default();
        ids.push(attachment_id);
        // `Distinct()`: the first of each, in order.
        let mut seen = std::collections::HashSet::new();
        ids.retain(|id| seen.insert(*id));
        let files: Vec<String> = row
            .pending_files
            .unwrap_or_default()
            .into_iter()
            .filter(|pending| pending != file)
            .collect();
        self.connection()
            .prepare_cached(
                "UPDATE outbox SET attachment_ids = $ids, pending_files = $files WHERE client_msg_id = $id",
            )?
            .execute(named_params! {
                "$ids": wire::encode(&ids),
                "$files": wire::encode(&files),
                "$id": client_msg_id,
            })?;
        Ok(())
    }

    /// Fail a row outright, without a verdict: the caller already knows nothing else is coming.
    /// The one honest use is media this device can no longer find — an id cannot recover a
    /// picture, so there is nothing for a backoff to wait for.
    pub fn refuse(&self, client_msg_id: &str, code: &str) -> StoreResult<()> {
        let _held = self.database.hold();
        self.connection()
            .prepare_cached(
                "UPDATE outbox SET failed_code = $code, next_attempt_at = NULL WHERE client_msg_id = $id",
            )?
            .execute(named_params! { "$code": code, "$id": client_msg_id })?;
        Ok(())
    }

    /// The server has swept the uploads this row names: `attachment_expired`, which means UPLOAD IT
    /// AGAIN and re-send the same `client_msg_id`. The dead ids go and every staged file is owed
    /// again — the ids were only ever valid while the server still held them. Answers whether
    /// there was anything to push again.
    pub fn reupload(&self, client_msg_id: &str) -> StoreResult<bool> {
        let _held = self.database.hold();
        let Some(staged) = self
            .find(client_msg_id)?
            .and_then(|row| row.staged_files)
            .filter(|staged| !staged.is_empty())
        else {
            return Ok(false);
        };
        self.connection()
            .prepare_cached(
                "UPDATE outbox SET attachment_ids = NULL, pending_files = $files WHERE client_msg_id = $id",
            )?
            .execute(named_params! { "$files": wire::encode(&staged), "$id": client_msg_id })?;
        Ok(true)
    }

    /// Somebody pressed retry: the budget starts again and the row is due at once. That is what
    /// the affordance promises, and a row that waited out a backoff after being asked would be a
    /// button that did nothing.
    pub fn retry(&self, client_msg_id: &str) -> StoreResult<()> {
        let _held = self.database.hold();
        self.connection()
            .prepare_cached(
                "UPDATE outbox SET attempts = 0, next_attempt_at = NULL, failed_code = NULL WHERE client_msg_id = $id",
            )?
            .execute(named_params! { "$id": client_msg_id })?;
        Ok(())
    }

    /// The user gave up on it. The bytes are the caller's to clean up.
    pub fn discard(&self, client_msg_id: &str) -> StoreResult<bool> {
        self.delivered(client_msg_id)
    }
}

fn read(row: &Row<'_>) -> rusqlite::Result<OutboxRow> {
    fn json<T: serde::de::DeserializeOwned>(
        row: &Row<'_>,
        column: &str,
    ) -> rusqlite::Result<Option<T>> {
        Ok(row
            .get::<_, Option<String>>(column)?
            .and_then(|text| wire::decode(&text)))
    }
    Ok(OutboxRow {
        client_msg_id: row.get("client_msg_id")?,
        chat_id: row.get("chat_id")?,
        body: row.get::<_, Option<String>>("body")?.unwrap_or_default(),
        reply_to_message_id: row.get("reply_to_id")?,
        attachment_ids: json(row, "attachment_ids")?,
        pending_files: json(row, "pending_files")?,
        staged_files: json(row, "staged_files")?,
        poll_options: json(row, "poll_json")?,
        mentions: json(row, "mentions_json")?,
        queued_at: times::from_milliseconds(row.get("queued_at")?),
        attempts: row.get("attempts")?,
        next_attempt_at: row
            .get::<_, Option<i64>>("next_attempt_at")?
            .and_then(times::from_milliseconds),
        failed_code: row.get("failed_code")?,
        sticker: row.get::<_, i64>("sticker")? != 0,
        round: row.get::<_, i64>("round")? != 0,
        animation: row.get::<_, i64>("animation")? != 0,
    })
}
