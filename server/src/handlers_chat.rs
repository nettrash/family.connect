//! Chat and message endpoints, plus the message-write core shared with the
//! WebSocket path.
//!
//! `create_message` and `apply_read_marker` are the single implementations
//! of "post a message" and "advance a read marker" — REST handlers and WS
//! frame handlers both call them, so validation, dedup, and monotonicity
//! semantics cannot drift between transports. Idempotency is enforced by
//! the database (`ON CONFLICT ... DO NOTHING` on the dedup index), never by
//! an application-level existence check.

use std::collections::HashMap;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::error::{ApiError, AppJson, codes};
use crate::events;
use crate::handlers_call::attach_calls;
use crate::handlers_poll::attach_polls;
use crate::models::{
    Attachment, Chat, ChatListEntry, Message, Poll, QuotedParent, Reaction, ReplyTo,
};
use crate::state::AppState;

/// Upper bound on a reaction emoji, in UTF-8 bytes. Fixed by protocol.md's
/// Limits table — generous enough for ZWJ family sequences, tight enough
/// that reactions stay reactions.
const MAX_EMOJI_BYTES: usize = 32;

#[derive(Debug, Deserialize)]
pub struct DirectChatRequest {
    pub user_id: i64,
}

#[derive(Debug, Deserialize)]
pub struct EditMessageRequest {
    pub body: String,
}

#[derive(Debug, Deserialize)]
pub struct PostMessageRequest {
    pub client_msg_id: Uuid,
    pub body: String,
    /// Optional: the message being answered. Must be in this same chat.
    #[serde(default)]
    pub reply_to_message_id: Option<i64>,
    /// Optional: the legacy spelling of a one-element `attachment_ids`,
    /// still accepted. Sending BOTH is `validation`.
    #[serde(default)]
    pub attachment_id: Option<i64>,
    /// Optional: attachments this caller uploaded, claimed by this message
    /// in the order given — 1 to `limits.max_attachments_per_message` of
    /// them. A message carrying any may have an empty body.
    #[serde(default)]
    pub attachment_ids: Option<Vec<i64>>,
    /// Optional: makes this message a poll. The body is then the QUESTION
    /// and may NOT be empty, and a poll excludes an attachment.
    #[serde(default)]
    pub poll: Option<NewPoll>,
}

/// The poll half of a `POST /chats/{id}/messages` body, and of a `send`
/// frame: `{"options": ["Pizza", "Pasta"]}`.
///
/// Options only. The question is the message body (protocol.md, "Polls"),
/// and there is no `multiple_choice` — a v1 poll asks one question and takes
/// one answer, which the primary key of `poll_votes` enforces rather than a
/// field here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NewPoll {
    pub options: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct ReadRequest {
    pub last_read_message_id: i64,
}

#[derive(Debug, Deserialize)]
pub struct ReactionRequest {
    pub emoji: String,
}

/// A message's full reaction state after a mutation — what the REST
/// response returns and what the `reaction` WS frame carries. `changed`
/// tells the caller whether anything actually moved (no fan-out on no-ops).
pub struct ReactionState {
    pub chat_id: i64,
    pub message_id: i64,
    pub reaction_seq: i64,
    pub reactions: Vec<Reaction>,
    pub changed: bool,
}

/// Order a user-id pair for the `direct` chat shape constraint
/// (`user_a_id < user_b_id`), making (a,b) and (b,a) the same chat.
pub fn ordered_pair(a: i64, b: i64) -> (i64, i64) {
    if a < b { (a, b) } else { (b, a) }
}

/// Clamp a requested page size into `[1, max_limit]`, defaulting when the
/// client sent none.
pub fn clamp_limit(requested: Option<i64>, default_limit: i64, max_limit: i64) -> i64 {
    requested.unwrap_or(default_limit).clamp(1, max_limit)
}

/// Verify the chat exists and the caller is a member.
///
/// 404 when the chat does not exist at all, 403 when it exists but the
/// caller is not in it — a member that left sees 403, not a phantom 404.
pub async fn ensure_chat_access(
    state: &AppState,
    chat_id: i64,
    user_id: i64,
) -> Result<(), ApiError> {
    let row = sqlx::query(
        "SELECT (cm.user_id IS NOT NULL) AS is_member
         FROM chats c
         LEFT JOIN chat_members cm ON cm.chat_id = c.id AND cm.user_id = $2
         WHERE c.id = $1",
    )
    .bind(chat_id)
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await?;
    match row {
        None => Err(ApiError::not_found(codes::CHAT_NOT_FOUND, "no such chat")),
        Some(row) if !row.get::<bool, _>("is_member") => Err(ApiError::forbidden(
            codes::NOT_CHAT_MEMBER,
            "you are not a member of this chat",
        )),
        Some(_) => Ok(()),
    }
}

/// The columns every full message read returns, and the self-join that
/// resolves a reply's quote.
///
/// Two self-joins, so every reply carries its quote AND what that quote was
/// itself answering, without the client having to hold either message (they
/// may be pages back, or never fetched at all). Each is a primary-key lookup
/// per row.
///
/// It stops at two by construction — there is no third join, and
/// `QuotedParent` has no `parent` field for a third level to land in
/// (protocol.md, "Replies").
///
/// The join is scoped to the same chat as a second line of defence. The
/// write path already refuses a cross-chat target (protocol.md makes that a
/// security property), so this can only matter if that check is ever
/// weakened — at which point a quote must still not leak text out of a chat
/// the reader cannot see.
///
/// ATTACHMENTS ARE DELIBERATELY NOT JOINED HERE. A message may carry up to
/// ten, and a LEFT JOIN would multiply every page row by the size of its
/// album — under a LIMIT that either truncates the page or repeats its
/// messages. They are hydrated after the fact by `attach_attachments`,
/// exactly as polls and reactions are.
const MESSAGE_COLS: &str = "m.id, m.chat_id, m.sender_id, m.client_msg_id, m.body, m.created_at, \
                            m.reaction_seq, m.edit_seq, m.edited_at, m.reply_to_message_id, \
                            p.sender_id AS reply_sender_id, p.body AS reply_body, \
                            p.reply_to_message_id AS reply_parent_message_id, \
                            g.sender_id AS reply_parent_sender_id, g.body AS reply_parent_body";
const MESSAGE_FROM: &str = "FROM messages m LEFT JOIN messages p \
                            ON p.id = m.reply_to_message_id AND p.chat_id = m.chat_id \
                            LEFT JOIN messages g \
                            ON g.id = p.reply_to_message_id AND g.chat_id = m.chat_id";

/// Delete messages past the retention age, and the attachment files they
/// leave behind.
///
/// What goes: the message row, and by cascade its reactions and its
/// attachment ROW. What survives: a newer reply that quoted it (the FK is
/// ON DELETE SET NULL — migration 0012 — so the reply keeps existing and
/// simply loses its quote), and the family board, which is a wall rather
/// than history.
///
/// The FILE is the careful part. Since 0011 a family's identical uploads
/// share one blob, so a file may only be removed once no row names it any
/// more — the keys are collected BEFORE the delete, and each is checked
/// afterwards, when the cascade has already taken the rows that were
/// going.
///
/// Returns how many messages were deleted. `retention_days = 0` disables
/// the sweep, which is why it is a state rather than a very large number.
pub async fn sweep_expired_messages(state: &AppState) -> Result<u64, ApiError> {
    let days = state.cfg.limits.retention_days;
    if days <= 0 {
        return Ok(0);
    }

    // Collected first: after the delete these rows are gone, and with them
    // any way to know which files to consider.
    let keys: Vec<String> = sqlx::query_scalar(
        "SELECT a.storage_key
         FROM attachments a
         JOIN messages m ON m.id = a.message_id
         WHERE m.created_at < now() - make_interval(days => $1)",
    )
    .bind(days as i32)
    .fetch_all(&state.pool)
    .await?;

    let deleted =
        sqlx::query("DELETE FROM messages WHERE created_at < now() - make_interval(days => $1)")
            .bind(days as i32)
            .execute(&state.pool)
            .await?
            .rows_affected();

    for key in &keys {
        crate::handlers_attachment::remove_if_unreferenced(state, key).await?;
    }
    Ok(deleted)
}

/// Fill in reaction state for messages whose `reaction_seq` says they have
/// any. `Some(vec![])` (not `None`) when the seq is set but every reaction
/// was removed — clients must see "cleared", not "no data".
async fn attach_reactions(state: &AppState, messages: &mut [Message]) -> Result<(), ApiError> {
    let reacted_ids: Vec<i64> = messages
        .iter()
        .filter(|m| m.reaction_seq.is_some_and(|seq| seq > 0))
        .map(|m| m.id)
        .collect();
    if reacted_ids.is_empty() {
        return Ok(());
    }
    for message in messages.iter_mut() {
        if message.reaction_seq.is_some_and(|seq| seq > 0) {
            message.reactions = Some(Vec::new());
        }
    }
    let reaction_rows = sqlx::query(
        "SELECT message_id, user_id, emoji FROM message_reactions
         WHERE message_id = ANY($1)
         ORDER BY created_at, user_id",
    )
    .bind(&reacted_ids)
    .fetch_all(&state.pool)
    .await?;
    let mut by_message: HashMap<i64, Vec<Reaction>> = HashMap::new();
    for row in &reaction_rows {
        by_message
            .entry(row.get("message_id"))
            .or_default()
            .push(Reaction {
                user_id: row.get("user_id"),
                emoji: row.get("emoji"),
            });
    }
    for message in messages.iter_mut() {
        if let Some(list) = by_message.remove(&message.id) {
            message.reactions = Some(list);
        }
    }
    Ok(())
}

/// Fill the attachment list in on every message of a page that carries one.
///
/// One query for the whole page, exactly as `attach_polls` does it — never
/// one per message, and never a JOIN on the message SELECT, which under
/// plurality would multiply every row by the size of its album (see the
/// note on `MESSAGE_COLS`).
///
/// Sets BOTH fields the protocol ties together: `attachments` in the
/// sender's order (`position`, stamped at claim time, with `id` as a
/// tiebreak so the order is total), and the legacy `attachment` as its
/// first element. Never one without the other, and neither on a message
/// that carries none — absent, not an empty array, on the wire.
pub async fn attach_attachments(
    pool: &sqlx::PgPool,
    messages: &mut [Message],
) -> Result<(), ApiError> {
    if messages.is_empty() {
        return Ok(());
    }
    let message_ids: Vec<i64> = messages.iter().map(|message| message.id).collect();
    let rows = sqlx::query(
        "SELECT message_id, id, kind, mime, size_bytes, width, height, duration_ms,
                has_preview, name, latitude, longitude, accuracy_m
         FROM attachments
         WHERE message_id = ANY($1)
         ORDER BY message_id, position, id",
    )
    .bind(&message_ids)
    .fetch_all(pool)
    .await?;
    let mut by_message: HashMap<i64, Vec<Attachment>> = HashMap::new();
    for row in &rows {
        by_message
            .entry(row.get("message_id"))
            .or_default()
            .push(Attachment::from_row(row));
    }
    for message in messages.iter_mut() {
        if let Some(list) = by_message.remove(&message.id) {
            message.attachment = list.first().cloned();
            message.attachments = Some(list);
        }
    }
    Ok(())
}

/// Trim and bounds-check a message body. Shared by send and edit on
/// purpose: the protocol gives them the same rules, and two copies would
/// eventually disagree about which one is authoritative.
fn validate_body(state: &AppState, body: &str) -> Result<String, ApiError> {
    let body = body.trim();
    if body.is_empty() {
        return Err(ApiError::bad_request(
            codes::MESSAGE_EMPTY,
            "message body is empty",
        ));
    }
    let max_chars = state.cfg.limits.max_message_chars;
    if body.chars().count() > max_chars {
        return Err(ApiError::bad_request(
            codes::MESSAGE_TOO_LONG,
            format!("message body exceeds {max_chars} characters"),
        ));
    }
    Ok(body.to_string())
}

/// Trim and bounds-check a poll's options (protocol.md, "Polls").
///
/// Options are fixed at creation — a poll can never gain, lose or rename
/// one, because the votes already cast were cast against the list as it was
/// read — so this is the only place they are ever checked.
///
/// Characters, not bytes, exactly as `max_message_chars` is counted: a
/// family that writes in Cyrillic gets the same ten options everyone else
/// does. The case-insensitive duplicate check is what stops "Pizza" and
/// "pizza" sitting on the same poll as two things to choose between.
fn validate_poll_options(state: &AppState, options: &[String]) -> Result<Vec<String>, ApiError> {
    let trimmed: Vec<String> = options.iter().map(|o| o.trim().to_string()).collect();
    if trimmed.iter().any(|option| option.is_empty()) {
        return Err(ApiError::bad_request(
            codes::INVALID_POLL,
            "a poll option cannot be empty",
        ));
    }
    let max_options = state.cfg.limits.max_poll_options;
    if trimmed.len() < Poll::MIN_OPTIONS || trimmed.len() > max_options {
        return Err(ApiError::bad_request(
            codes::INVALID_POLL,
            format!(
                "a poll needs between {} and {max_options} options",
                Poll::MIN_OPTIONS
            ),
        ));
    }
    let max_chars = state.cfg.limits.max_poll_option_chars;
    if trimmed.iter().any(|o| o.chars().count() > max_chars) {
        return Err(ApiError::bad_request(
            codes::INVALID_POLL,
            format!("a poll option exceeds {max_chars} characters"),
        ));
    }
    let mut seen = std::collections::HashSet::new();
    if !trimmed.iter().all(|o| seen.insert(o.to_lowercase())) {
        return Err(ApiError::bad_request(
            codes::INVALID_POLL,
            "two poll options are the same",
        ));
    }
    Ok(trimmed)
}

/// Insert a message, idempotently.
///
/// Returns `(message, created)`: `created == false` means the
/// `(chat, sender, client_msg_id)` triple already existed and the original
/// message is returned — the caller must not fan out again.
// Nine arguments, two over clippy's threshold. Grouping them into a struct
// would be a parameter object nothing else ever constructs — the call sites
// are two, both in this crate, and each argument is a distinct thing the
// caller genuinely has.
#[allow(clippy::too_many_arguments)]
pub async fn create_message(
    state: &AppState,
    chat_id: i64,
    sender_id: i64,
    client_msg_id: Uuid,
    body: &str,
    reply_to_message_id: Option<i64>,
    // Already merged from the two spellings the wire accepts (see
    // `merge_attachment_ids`); empty means the message carries none.
    attachment_ids: &[i64],
    // Optional: makes this message a poll (protocol.md, "Polls"). A poll is
    // created HERE, inside the message transaction, rather than by a second
    // endpoint — a poll IS a message, and a second write path is exactly
    // what would let the two drift apart.
    poll: Option<&NewPoll>,
    // Which language to answer an assistant question in, from the sending
    // device (docs/protocol.md, "The assistant"). None outside an assistant
    // chat, and harmless there too.
    language: Option<&str>,
) -> Result<(Message, bool), ApiError> {
    ensure_chat_access(state, chat_id, sender_id).await?;

    // Read once, up here: the assistant trigger at the end needs it, and
    // `ensure_chat_access` has already proved the chat exists.
    let chat_kind: String = sqlx::query_scalar("SELECT kind FROM chats WHERE id = $1")
        .bind(chat_id)
        .fetch_optional(&state.pool)
        .await?
        .unwrap_or_default();

    // Where a poll may live, checked before anything else about it: a poll
    // is a family deciding something together, and between two people it is
    // a question whose answer is the next message (protocol.md, "Polls").
    // The assistant's chat refuses one for the same reason.
    if poll.is_some() {
        if chat_kind != "family" {
            return Err(ApiError::bad_request(
                codes::INVALID_POLL,
                "polls are only allowed in the family chat",
            ));
        }
        if !attachment_ids.is_empty() {
            return Err(ApiError::bad_request(
                codes::INVALID_POLL,
                "a message carries a poll or an attachment, not both",
            ));
        }
    }

    // The set rules, checked before any id is looked up: the ceiling and
    // the no-duplicates rule are about the ARRAY, not about what it names,
    // so a send that breaks them is refused for the real reason rather
    // than for whatever the first bad id happens to be.
    let max_attachments = state.cfg.limits.max_attachments_per_message;
    if attachment_ids.len() > max_attachments {
        return Err(ApiError::bad_request(
            codes::INVALID_ATTACHMENT,
            format!("a message carries at most {max_attachments} attachments"),
        ));
    }
    {
        let mut seen = std::collections::HashSet::new();
        if !attachment_ids.iter().all(|id| seen.insert(*id)) {
            return Err(ApiError::bad_request(
                codes::INVALID_ATTACHMENT,
                "the same attachment id appears twice in one message",
            ));
        }
    }

    // A photo needs no caption: an empty body is allowed when — and only
    // when — the message carries an attachment. A poll gets NO such
    // relaxation — its body is the question, and a poll with no question is
    // `message_empty` — and the exclusion just above means the two
    // conditions can never both hold anyway.
    let body = if !attachment_ids.is_empty() && poll.is_none() && body.trim().is_empty() {
        String::new()
    } else {
        validate_body(state, body)?
    };
    let body = body.as_str();

    // Last, so an empty question is answered `message_empty` rather than
    // `invalid_poll`: the body is the more basic thing to have got wrong.
    let poll_options = match poll {
        None => None,
        Some(poll) => Some(validate_poll_options(state, &poll.options)?),
    };

    // The quote is resolved BEFORE the insert, both to reject a bad target
    // without writing anything and because the row we read here is exactly
    // the snippet the response carries — no second query, and no chance of
    // the two disagreeing.
    //
    // Scoped to this chat on purpose: a real message in another chat must
    // be indistinguishable from one that does not exist, or the endpoint
    // would confirm ids the caller cannot otherwise see.
    let reply_to = match reply_to_message_id {
        None => None,
        Some(target_id) => {
            // ONE self-joined query, not two: the comment above promises
            // that the row read here IS the snippet the response carries,
            // and splitting the second level into its own query would give
            // the two a chance to disagree.
            let parent = sqlx::query(
                "SELECT p.id, p.sender_id, p.body, \
                        g.id AS g_id, g.sender_id AS g_sender_id, g.body AS g_body \
                 FROM messages p \
                 LEFT JOIN messages g ON g.id = p.reply_to_message_id AND g.chat_id = p.chat_id \
                 WHERE p.id = $1 AND p.chat_id = $2",
            )
            .bind(target_id)
            .bind(chat_id)
            .fetch_optional(&state.pool)
            .await?;
            let Some(parent) = parent else {
                return Err(ApiError::not_found(
                    codes::MESSAGE_NOT_FOUND,
                    "no such message in this chat",
                ));
            };
            let grandparent = match (
                parent.get::<Option<i64>, _>("g_id"),
                parent.get::<Option<i64>, _>("g_sender_id"),
                parent.get::<Option<String>, _>("g_body"),
            ) {
                (Some(id), Some(sender_id), Some(body)) => {
                    Some(QuotedParent::new(id, sender_id, &body))
                }
                _ => None,
            };
            Some(
                ReplyTo::new(
                    parent.get("id"),
                    parent.get("sender_id"),
                    parent.get::<String, _>("body").as_str(),
                )
                .with_parent(grandparent),
            )
        }
    };

    // The insert and the attachment claim are ONE transaction, and that is
    // load-bearing rather than tidy. The claim can fail for reasons the
    // sender cannot control — the sweeper removed an upload the client
    // retried past its 24 h grace, or the id was already used — and with
    // two autocommitted statements the message would already be in the
    // chat by then. Since a message carrying an attachment is allowed an
    // empty body, what everyone would be left with is a permanent, blank,
    // undeletable bubble that is also the chat's newest message.
    let mut tx = state.pool.begin().await?;

    let inserted = sqlx::query(
        "INSERT INTO messages (chat_id, sender_id, client_msg_id, body, reply_to_message_id)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (chat_id, sender_id, client_msg_id) DO NOTHING
         RETURNING id, chat_id, sender_id, client_msg_id, body, created_at",
    )
    .bind(chat_id)
    .bind(sender_id)
    .bind(client_msg_id)
    .bind(body)
    .bind(reply_to_message_id)
    .fetch_optional(&mut *tx)
    .await?;

    if let Some(row) = inserted {
        let mut message = Message::from_row(&row);
        // RETURNING cannot join, so the snippet resolved above is attached
        // here rather than re-read.
        message.reply_to = reply_to;
        if !attachment_ids.is_empty() {
            // Claimed one by one, IN THE SENDER'S ORDER, stamping the
            // array index as `position` — which is the order every read
            // returns. The `?` rolls the transaction back on drop: ANY id
            // failing takes the message and every earlier claim with it
            // (all-or-nothing, protocol.md), and the sender gets the real
            // reason.
            // LOCKS ARE TAKEN IN ID ORDER, positions stay the sender's.
            // Claiming in array order took row locks in whatever order the
            // sender listed the ids, and two concurrent sends from the
            // same uploader naming overlapping ids in opposite orders
            // ([34, 61] vs [61, 34]) formed a lock cycle — Postgres broke
            // it with 40P01, which surfaced as a 500 where the loser's
            // truthful answer is `attachment_already_used`. Sorting a COPY
            // for acquisition makes every transaction lock in one global
            // order, so the loser now blocks, sees the winner's claim, and
            // gets the 409. The `position` stamped is still the index in
            // the SENDER'S array, and the claimed list is put back into
            // that order below — the wire order never changes.
            let mut lock_order: Vec<(usize, i64)> =
                attachment_ids.iter().copied().enumerate().collect();
            lock_order.sort_unstable_by_key(|(_, id)| *id);
            let mut claimed_by_index = Vec::with_capacity(lock_order.len());
            for (position, attachment_id) in lock_order {
                claimed_by_index.push((
                    position,
                    claim_attachment(
                        &mut tx,
                        attachment_id,
                        sender_id,
                        message.id,
                        position as i16,
                    )
                    .await?,
                ));
            }
            claimed_by_index.sort_unstable_by_key(|(position, _)| *position);
            let claimed: Vec<_> = claimed_by_index
                .into_iter()
                .map(|(_, attachment)| attachment)
                .collect();
            // A LOCATION is always alone (protocol.md) — a place is a
            // statement, not a page of an album. Checked AFTER the claims
            // because only the claims reveal the kinds, and the order costs
            // nothing: returning here drops the transaction, so nothing is
            // written either way. The protocol does not promise WHICH
            // refusal wins when several apply — only that nothing lands.
            if claimed.len() > 1
                && claimed
                    .iter()
                    .any(|attachment| attachment.kind == Attachment::KIND_LOCATION)
            {
                return Err(ApiError::bad_request(
                    codes::INVALID_ATTACHMENT,
                    "a location is always a message's only attachment",
                ));
            }
            // Both fields, always together: the legacy `attachment` is the
            // first element, for clients that predate plurality.
            message.attachment = claimed.first().cloned();
            message.attachments = Some(claimed);
        }
        if let Some(options) = &poll_options {
            // Same transaction, same reason: a poll written after an
            // autocommitted message could fail and leave a permanent
            // options-less bubble that is also the chat's newest message.
            // `chat_id` is taken from the row just inserted, not from the
            // request — nothing in the schema ties the two together.
            message.poll = Some(
                crate::handlers_poll::create_poll(&mut tx, message.chat_id, message.id, options)
                    .await?,
            );
        }
        tx.commit().await?;

        // An assistant chat answers back, and this lives HERE rather than in
        // the REST handler because both transports come through this
        // function: a client with a live socket sends over it, and wiring
        // the trigger into `post_message` alone meant asking a question in
        // the app produced no reply at all while curl worked perfectly.
        //
        // Spawned, so the sender's send returns at once: a reply takes
        // seconds, and holding the send open for it would make asking a
        // question feel like a failure.
        if state.cfg.ai.is_usable() {
            match chat_kind.as_str() {
                "ai" => crate::handlers_ai::spawn_reply(
                    state.clone(),
                    chat_id,
                    sender_id,
                    language.map(str::to_string),
                ),
                // The family chat answers only when it is ASKED to
                // (protocol.md, "Mentioning the assistant in the family
                // chat"). The mention is the consent: without it nothing
                // about the message leaves the server, which is what keeps
                // an ordinary family conversation ordinary.
                //
                // Family only, deliberately. A direct chat is two people
                // who each already have a private assistant, and an
                // assistant answering inside someone's one-to-one is a
                // third party in a conversation that had two.
                "family" if crate::mentions::mentions_assistant(body) => {
                    crate::handlers_ai::spawn_mention_reply(
                        state.clone(),
                        chat_id,
                        sender_id,
                        message.id,
                        language.map(str::to_string),
                    )
                }
                _ => {}
            }
        }
        return Ok((message, true));
    }
    // Dedup: nothing was written, so there is nothing to commit.
    drop(tx);

    // Dedup hit: a retry of a message that already landed. Return the
    // original — possibly with a different body if the client mutated the
    // retry, which is the client's bug; the first write wins.
    let row = sqlx::query(&format!(
        "SELECT {MESSAGE_COLS} {MESSAGE_FROM}
         WHERE m.chat_id = $1 AND m.sender_id = $2 AND m.client_msg_id = $3"
    ))
    .bind(chat_id)
    .bind(sender_id)
    .bind(client_msg_id)
    .fetch_one(&state.pool)
    .await?;
    let mut message = Message::from_row(&row);
    // A retry re-acks the ORIGINAL message, which is a `Message` like any
    // other and must be hydrated like one. Both of these are load-bearing
    // rather than tidy:
    //
    //   * reactions, because `reaction_seq` is read from the row and is
    //     therefore already on the wire — answering with a seq and no
    //     `reactions` key is the one state protocol.md's Objects block does
    //     not define, since the two are named together and an empty list is
    //     how "cleared" is said. Dedup exists so an offline client can
    //     replay its outbound queue, and a message sitting in that queue is
    //     exactly the one somebody has had time to react to;
    //   * the poll, because a retry does NOT create a second one — the
    //     first write won, poll and all — and a client that retried would
    //     otherwise draw the question with no options.
    attach_reactions(state, std::slice::from_mut(&mut message)).await?;
    attach_attachments(&state.pool, std::slice::from_mut(&mut message)).await?;
    attach_polls(&state.pool, std::slice::from_mut(&mut message)).await?;
    attach_calls(&state.pool, std::slice::from_mut(&mut message)).await?;
    Ok((message, false))
}

/// The outcome of an edit: the message as it now stands, plus whether
/// anything actually changed. Re-sending the identical body burns no
/// sequence value and raises no fan-out, exactly like re-PUTing the
/// reaction you already have.
pub struct EditOutcome {
    pub message: Message,
    pub changed: bool,
}

/// Replace a message's body. Author only, no time limit; nothing else about
/// the message moves (protocol.md, "Editing").
pub async fn apply_edit(
    state: &AppState,
    chat_id: i64,
    message_id: i64,
    user_id: i64,
    body: &str,
) -> Result<EditOutcome, ApiError> {
    ensure_chat_access(state, chat_id, user_id).await?;
    // A call record's body is a placeholder other clients render as "Voice
    // call"; the author-only edit path would otherwise let a caller write
    // anything into that line (protocol.md, "The record").
    let is_call: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM calls WHERE message_id = $1)")
            .bind(message_id)
            .fetch_one(&state.pool)
            .await?;
    if is_call {
        return Err(ApiError::validation("a call record cannot be edited"));
    }
    let body = validate_body(state, body)?;

    let mut tx = state.pool.begin().await?;
    // FOR UPDATE: two devices of the same author can edit at once, and the
    // seq must be stamped against a body nobody else is rewriting.
    let locked = sqlx::query(
        "SELECT sender_id, body, edit_seq FROM messages WHERE id = $1 AND chat_id = $2 FOR UPDATE",
    )
    .bind(message_id)
    .bind(chat_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(locked) = locked else {
        // Same answer as a message in another chat — the endpoint never
        // confirms that an id exists elsewhere.
        return Err(ApiError::not_found(
            codes::MESSAGE_NOT_FOUND,
            "no such message in this chat",
        ));
    };

    let sender_id: i64 = locked.get("sender_id");
    if sender_id != user_id {
        return Err(ApiError::forbidden(
            codes::NOT_MESSAGE_AUTHOR,
            "only the author can edit this message",
        ));
    }

    let current_body: String = locked.get("body");
    let changed = current_body != body;
    if changed {
        let seq: i64 = sqlx::query_scalar("SELECT nextval('message_edit_seq')")
            .fetch_one(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE messages SET body = $2, edit_seq = $3, edited_at = now() WHERE id = $1",
        )
        .bind(message_id)
        .bind(&body)
        .bind(seq)
        .execute(&mut *tx)
        .await?;
        // GREATEST, not plain SET: two messages in one chat can commit out
        // of seq order, and the chat cursor must never move backwards.
        sqlx::query("UPDATE chats SET last_edit_seq = GREATEST(last_edit_seq, $2) WHERE id = $1")
            .bind(chat_id)
            .bind(seq)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;

    let message = fetch_message(state, chat_id, message_id)
        .await?
        .ok_or_else(|| {
            ApiError::not_found(codes::MESSAGE_NOT_FOUND, "no such message in this chat")
        })?;
    Ok(EditOutcome { message, changed })
}

/// One whole message — quote joined, reactions attached — as every read
/// path returns it.
pub async fn fetch_message(
    state: &AppState,
    chat_id: i64,
    message_id: i64,
) -> Result<Option<Message>, ApiError> {
    let row = sqlx::query(&format!(
        "SELECT {MESSAGE_COLS} {MESSAGE_FROM} WHERE m.chat_id = $1 AND m.id = $2"
    ))
    .bind(chat_id)
    .bind(message_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some(row) = row else { return Ok(None) };
    let mut message = Message::from_row(&row);
    attach_reactions(state, std::slice::from_mut(&mut message)).await?;
    attach_attachments(&state.pool, std::slice::from_mut(&mut message)).await?;
    attach_polls(&state.pool, std::slice::from_mut(&mut message)).await?;
    attach_calls(&state.pool, std::slice::from_mut(&mut message)).await?;
    Ok(Some(message))
}

/// One whole message found by its `(chat_id, sender_id, client_msg_id)` —
/// the dedup key. Used when a call's record already exists (a call that
/// ended twice) and the existing row must be returned rather than a second
/// one written (docs/protocol.md, "The record").
pub async fn fetch_message_by_client_id(
    state: &AppState,
    chat_id: i64,
    sender_id: i64,
    client_msg_id: Uuid,
) -> Result<Option<Message>, ApiError> {
    let row = sqlx::query(&format!(
        "SELECT {MESSAGE_COLS} {MESSAGE_FROM}
         WHERE m.chat_id = $1 AND m.sender_id = $2 AND m.client_msg_id = $3"
    ))
    .bind(chat_id)
    .bind(sender_id)
    .bind(client_msg_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some(row) = row else { return Ok(None) };
    let mut message = Message::from_row(&row);
    attach_reactions(state, std::slice::from_mut(&mut message)).await?;
    attach_attachments(&state.pool, std::slice::from_mut(&mut message)).await?;
    attach_polls(&state.pool, std::slice::from_mut(&mut message)).await?;
    attach_calls(&state.pool, std::slice::from_mut(&mut message)).await?;
    Ok(Some(message))
}

/// The one list of attachment ids a send names, merged from the two
/// spellings the protocol accepts: `attachment_ids` (the array), and the
/// legacy `attachment_id`, which is exactly a one-element array and is
/// still accepted. Sending BOTH is `validation` — two spellings that could
/// disagree are a client bug worth hearing about, not something to pick a
/// winner between. An empty array is the same statement as neither.
pub fn merge_attachment_ids(
    legacy: Option<i64>,
    ids: Option<Vec<i64>>,
) -> Result<Vec<i64>, ApiError> {
    match (legacy, ids) {
        (Some(_), Some(_)) => Err(ApiError::validation(
            "send attachment_ids or the legacy attachment_id, not both",
        )),
        (Some(id), None) => Ok(vec![id]),
        (None, Some(ids)) => Ok(ids),
        (None, None) => Ok(Vec::new()),
    }
}

/// Bind an uploaded attachment to the message that just claimed it,
/// stamping `position` — the index into the sender's `attachment_ids`
/// array, which is the order every read returns.
///
/// Claimable once, by its uploader only. The `message_id IS NULL` guard in
/// the UPDATE is the real guarantee (0025 removed the unique index that
/// used to forbid a second attachment per MESSAGE — claiming stays
/// once-per-ATTACHMENT); the check below exists to answer with the
/// protocol's error rather than silence.
/// Runs inside the caller's transaction so that a refusal takes the
/// message and every sibling claim with it — see the note at the insert.
async fn claim_attachment(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    attachment_id: i64,
    uploader_id: i64,
    message_id: i64,
    position: i16,
) -> Result<Attachment, ApiError> {
    let row = sqlx::query(
        "UPDATE attachments SET message_id = $3, position = $4
         WHERE id = $1 AND uploader_id = $2 AND message_id IS NULL
         RETURNING id, kind, mime, size_bytes, width, height, duration_ms, has_preview, name,
                   latitude, longitude, accuracy_m",
    )
    .bind(attachment_id)
    .bind(uploader_id)
    .bind(message_id)
    .bind(position)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(row) = row {
        return Ok(Attachment::from_row(&row));
    }

    // Tell "already claimed" apart from "not yours / no such thing" — the
    // first is worth retrying differently, the second is not.
    let exists: Option<i64> =
        sqlx::query_scalar("SELECT message_id FROM attachments WHERE id = $1 AND uploader_id = $2")
            .bind(attachment_id)
            .bind(uploader_id)
            .fetch_optional(&mut **tx)
            .await?
            .flatten();
    if exists.is_some() {
        Err(ApiError::conflict(
            codes::ATTACHMENT_ALREADY_USED,
            "that attachment is already on another message",
        ))
    } else {
        Err(ApiError::not_found(
            codes::ATTACHMENT_NOT_FOUND,
            "no such attachment",
        ))
    }
}

/// Advance a read marker, monotonically (the max ever reported wins).
/// Returns the effective marker after the write.
pub async fn apply_read_marker(
    state: &AppState,
    chat_id: i64,
    user_id: i64,
    last_read_message_id: i64,
) -> Result<i64, ApiError> {
    ensure_chat_access(state, chat_id, user_id).await?;
    if last_read_message_id < 0 {
        return Err(ApiError::validation("last_read_message_id must be >= 0"));
    }
    let effective: i64 = sqlx::query_scalar(
        "INSERT INTO chat_reads (chat_id, user_id, last_read_message_id)
         VALUES ($1, $2, $3)
         ON CONFLICT (chat_id, user_id) DO UPDATE
         SET last_read_message_id =
                 GREATEST(chat_reads.last_read_message_id, EXCLUDED.last_read_message_id),
             updated_at = now()
         RETURNING last_read_message_id",
    )
    .bind(chat_id)
    .bind(user_id)
    .bind(last_read_message_id)
    .fetch_one(&state.pool)
    .await?;
    Ok(effective)
}

/// Set (`Some(emoji)`) or remove (`None`) the caller's reaction on a
/// message — an idempotent state-set, one reaction per user per message.
///
/// The transaction locks the message row *first*: that serializes
/// concurrent reactions to the same message so the state SELECT below
/// always sees every committed reaction (without it, two READ COMMITTED
/// writers each ship a frame missing the other's row), and it doubles as
/// the existence + belongs-to-this-chat check. Lock order is uniform
/// (message row, then chat row) and nothing else takes both — no deadlock.
pub async fn apply_reaction(
    state: &AppState,
    chat_id: i64,
    message_id: i64,
    user_id: i64,
    emoji: Option<&str>,
) -> Result<ReactionState, ApiError> {
    ensure_chat_access(state, chat_id, user_id).await?;

    let emoji = match emoji {
        Some(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() || trimmed.len() > MAX_EMOJI_BYTES {
                return Err(ApiError::bad_request(
                    codes::INVALID_EMOJI,
                    format!("emoji must be non-empty and at most {MAX_EMOJI_BYTES} bytes"),
                ));
            }
            Some(trimmed.to_string())
        }
        None => None,
    };

    let mut tx = state.pool.begin().await?;
    let locked =
        sqlx::query("SELECT reaction_seq FROM messages WHERE id = $1 AND chat_id = $2 FOR UPDATE")
            .bind(message_id)
            .bind(chat_id)
            .fetch_optional(&mut *tx)
            .await?;
    let Some(locked) = locked else {
        return Err(ApiError::not_found(
            codes::MESSAGE_NOT_FOUND,
            "no such message in this chat",
        ));
    };
    let current_seq: i64 = locked.get("reaction_seq");

    // RETURNING yields a row only when something was actually written, so
    // a re-PUT of the same emoji or a DELETE of nothing is detected here
    // and burns neither a sequence value nor a fan-out.
    let changed = match &emoji {
        Some(emoji) => sqlx::query(
            "INSERT INTO message_reactions (message_id, user_id, emoji)
             VALUES ($1, $2, $3)
             ON CONFLICT (message_id, user_id) DO UPDATE
             SET emoji = EXCLUDED.emoji, created_at = now()
             WHERE message_reactions.emoji IS DISTINCT FROM EXCLUDED.emoji
             RETURNING message_id",
        )
        .bind(message_id)
        .bind(user_id)
        .bind(emoji)
        .fetch_optional(&mut *tx)
        .await?
        .is_some(),
        None => sqlx::query(
            "DELETE FROM message_reactions
             WHERE message_id = $1 AND user_id = $2
             RETURNING message_id",
        )
        .bind(message_id)
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await?
        .is_some(),
    };

    let reaction_seq = if changed {
        let seq: i64 = sqlx::query_scalar("SELECT nextval('message_reaction_seq')")
            .fetch_one(&mut *tx)
            .await?;
        sqlx::query("UPDATE messages SET reaction_seq = $2 WHERE id = $1")
            .bind(message_id)
            .bind(seq)
            .execute(&mut *tx)
            .await?;
        // GREATEST, not plain SET: two messages in one chat can commit out
        // of seq order, and the chat cursor must never move backwards.
        sqlx::query(
            "UPDATE chats SET last_reaction_seq = GREATEST(last_reaction_seq, $2) WHERE id = $1",
        )
        .bind(chat_id)
        .bind(seq)
        .execute(&mut *tx)
        .await?;
        seq
    } else {
        current_seq
    };

    let reactions = fetch_reaction_rows(&mut *tx, message_id).await?;
    tx.commit().await?;

    Ok(ReactionState {
        chat_id,
        message_id,
        reaction_seq,
        reactions,
        changed,
    })
}

/// A message's current reactions, in stable (created_at, user_id) order.
async fn fetch_reaction_rows<'e, E>(executor: E, message_id: i64) -> Result<Vec<Reaction>, ApiError>
where
    E: sqlx::PgExecutor<'e>,
{
    let rows = sqlx::query(
        "SELECT user_id, emoji FROM message_reactions
         WHERE message_id = $1
         ORDER BY created_at, user_id",
    )
    .bind(message_id)
    .fetch_all(executor)
    .await?;
    Ok(rows
        .iter()
        .map(|row| Reaction {
            user_id: row.get("user_id"),
            emoji: row.get("emoji"),
        })
        .collect())
}

/// `GET /chats` — every chat the caller is a member of, with a last-message
/// preview and the authoritative unread count.
pub async fn list_chats(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Response, ApiError> {
    // Created on demand rather than at registration, so turning the
    // assistant on later needs nobody to re-register and leaving it off
    // costs an empty chat in nobody's list. A no-op when it already exists,
    // or when the server is not configured for it.
    if let Err(error) = crate::handlers_ai::ensure_ai_chat(&state, auth.user_id).await {
        tracing::warn!(%error, "could not ensure the assistant chat");
    }
    let rows = sqlx::query(
        "SELECT c.id AS chat_id, c.kind,
                f.name AS family_name,
                CASE WHEN c.kind = 'direct' THEN
                    CASE WHEN c.user_a_id = $1 THEN c.user_b_id ELSE c.user_a_id END
                END AS peer_user_id,
                pu.display_name AS peer_display_name,
                lm.id AS last_id, lm.sender_id AS last_sender_id,
                lm.client_msg_id AS last_client_msg_id, lm.body AS last_body,
                lm.created_at AS last_created_at,
                lm.call_outcome, lm.call_duration_secs,
                uc.unread AS unread_count,
                -- The caller's own marker, off the SAME chat_reads row the
                -- unread count is measured against: it costs no extra join
                -- and no extra query, and 0 (never read anything here) is
                -- the answer the count already assumes.
                COALESCE(cr.last_read_message_id, 0) AS last_read_message_id,
                c.last_reaction_seq,
                c.last_edit_seq,
                c.last_poll_seq
         FROM chat_members m
         JOIN chats c ON c.id = m.chat_id
         JOIN families f ON f.id = c.family_id
         LEFT JOIN users pu
                ON c.kind = 'direct'
               AND pu.id = CASE WHEN c.user_a_id = $1 THEN c.user_b_id ELSE c.user_a_id END
         LEFT JOIN chat_reads cr ON cr.chat_id = c.id AND cr.user_id = $1
         LEFT JOIN LATERAL (
             -- The BARE last message. Its attachments are hydrated after
             -- the fact with one query for the whole list — joined here
             -- they would multiply the candidate rows before the LIMIT 1,
             -- and a three-photo album would repeat as three last messages.
             SELECT m2.id, m2.sender_id, m2.client_msg_id, m2.body, m2.created_at,
                    -- The call record's outcome comes with the preview: a
                    -- client that knows the object draws its own wording
                    -- rather than the English placeholder body.
                    cl.outcome AS call_outcome, cl.duration_secs AS call_duration_secs
             FROM messages m2
             LEFT JOIN calls cl ON cl.message_id = m2.id
             WHERE m2.chat_id = c.id ORDER BY m2.id DESC LIMIT 1
         ) lm ON TRUE
         LEFT JOIN LATERAL (
             SELECT count(*) AS unread
             FROM messages msg
             WHERE msg.chat_id = c.id
               AND msg.id > COALESCE(cr.last_read_message_id, 0)
               AND msg.sender_id <> $1
         ) uc ON TRUE
         WHERE m.user_id = $1
         ORDER BY COALESCE(lm.id, 0) DESC, c.id",
    )
    .bind(auth.user_id)
    .fetch_all(&state.pool)
    .await?;

    // The preview attachments, hydrated with ONE page-sized query rather
    // than joined inside the lateral (see the note there). Trimmed exactly
    // as the preview always was — kind and name, plus the mime, size and
    // preview flag the row carried before plurality; no dimensions, and NO
    // COORDINATES: a chat-list row is one line of text — "Location" — and
    // shipping a family member's position on every list read to draw
    // nothing with it would be a wider answer than the question.
    let preview_ids: Vec<i64> = rows
        .iter()
        .filter_map(|row| row.get::<Option<i64>, _>("last_id"))
        .collect();
    let mut preview_attachments: HashMap<i64, Vec<Attachment>> = HashMap::new();
    if !preview_ids.is_empty() {
        let attachment_rows = sqlx::query(
            "SELECT message_id, id, kind, mime, size_bytes, has_preview, name
             FROM attachments
             WHERE message_id = ANY($1)
             ORDER BY message_id, position, id",
        )
        .bind(&preview_ids)
        .fetch_all(&state.pool)
        .await?;
        for row in &attachment_rows {
            preview_attachments
                .entry(row.get("message_id"))
                .or_default()
                .push(Attachment {
                    id: row.get("id"),
                    kind: row.get("kind"),
                    mime: row.get("mime"),
                    size: row.get("size_bytes"),
                    width: None,
                    height: None,
                    duration_ms: None,
                    has_preview: row.get("has_preview"),
                    name: row.get("name"),
                    latitude: None,
                    longitude: None,
                    accuracy_m: None,
                });
        }
    }

    let chats: Vec<ChatListEntry> = rows
        .iter()
        .map(|row| {
            let kind: String = row.get("kind");
            let peer_user_id: Option<i64> = row.get("peer_user_id");
            let title: String = match kind.as_str() {
                "family" => row.get("family_name"),
                // The assistant has no peer to name it after — and its
                // reserved account is not in the roster, so a lookup would
                // find nothing anyway.
                "ai" => state.cfg.ai.title.clone(),
                _ => row.get("peer_display_name"),
            };
            let chat_id: i64 = row.get("chat_id");
            let last_message = row.get::<Option<i64>, _>("last_id").map(|last_id| {
                // The attachments DO come along: without them a client has
                // nothing to write on the row for a caption-less photo.
                // Hydrated above; the legacy `attachment` is the first
                // element, exactly as on a full message.
                let attachments = preview_attachments.remove(&last_id);
                Message {
                    id: last_id,
                    chat_id,
                    sender_id: row.get("last_sender_id"),
                    client_msg_id: row.get("last_client_msg_id"),
                    body: row.get("last_body"),
                    created_at: row.get("last_created_at"),
                    reactions: None,
                    reaction_seq: None,
                    // Previews carry neither reactions nor the quote: the chat
                    // list draws one line of text, not a bubble.
                    reply_to: None,
                    // Nor the edit stamps — the preview is the current text,
                    // and whether it was edited is a bubble's business.
                    edited_at: None,
                    edit_seq: None,
                    attachment: attachments.as_ref().and_then(|list| list.first().cloned()),
                    attachments,
                    // Nor the poll. A poll's QUESTION is the body, which the
                    // preview already carries — the options are a bubble's
                    // business and three tables' worth of reads per chat.
                    poll: None,
                    // The call record DOES come along: its placeholder body is
                    // not something a client should show, so a preview needs the
                    // outcome to draw its own line ("Missed voice call", "5:12").
                    call: row.get::<Option<String>, _>("call_outcome").map(|outcome| {
                        crate::models::CallRecord {
                            outcome,
                            duration_secs: row.get("call_duration_secs"),
                        }
                    }),
                }
            });
            let last_reaction_seq: i64 = row.get("last_reaction_seq");
            let last_edit_seq: i64 = row.get("last_edit_seq");
            let last_poll_seq: i64 = row.get("last_poll_seq");
            ChatListEntry {
                chat: Chat {
                    id: chat_id,
                    kind,
                    title,
                    peer_user_id,
                },
                last_message,
                unread_count: row.get("unread_count"),
                last_read_message_id: row.get("last_read_message_id"),
                max_reaction_seq: (last_reaction_seq > 0).then_some(last_reaction_seq),
                max_edit_seq: (last_edit_seq > 0).then_some(last_edit_seq),
                max_poll_seq: (last_poll_seq > 0).then_some(last_poll_seq),
            }
        })
        .collect();

    Ok((StatusCode::OK, Json(json!({"chats": chats}))).into_response())
}

/// `POST /chats/direct` — get-or-create the direct chat with another family
/// member. Idempotent by the `(user_a, user_b)` unique index; re-creating
/// after a leave/rejoin also re-attaches both members.
pub async fn direct_chat(
    auth: AuthUser,
    State(state): State<AppState>,
    AppJson(req): AppJson<DirectChatRequest>,
) -> Result<Response, ApiError> {
    if req.user_id == auth.user_id {
        return Err(ApiError::bad_request(
            codes::CANNOT_DM_SELF,
            "cannot open a direct chat with yourself",
        ));
    }
    let Some(family_id) = auth.family_id else {
        // Without a family there is no shared family to be in.
        return Err(ApiError::conflict(
            codes::NOT_IN_FAMILY,
            "you do not belong to a family",
        ));
    };
    let peer = sqlx::query("SELECT id, family_id, display_name FROM users WHERE id = $1")
        .bind(req.user_id)
        .fetch_optional(&state.pool)
        .await?;
    let Some(peer) = peer else {
        return Err(ApiError::not_found(codes::USER_NOT_FOUND, "no such user"));
    };
    if peer.get::<Option<i64>, _>("family_id") != Some(family_id) {
        return Err(ApiError::conflict(
            codes::NOT_SAME_FAMILY,
            "this user is not in your family",
        ));
    }

    let (user_a, user_b) = ordered_pair(auth.user_id, req.user_id);
    let mut tx = state.pool.begin().await?;
    // On conflict the family_id is refreshed: a pair that left family X and
    // reunited in family Y keeps one chat (and its history) across moves.
    let chat_id: i64 = sqlx::query_scalar(
        "INSERT INTO chats (family_id, kind, user_a_id, user_b_id)
         VALUES ($1, 'direct', $2, $3)
         ON CONFLICT (user_a_id, user_b_id) WHERE kind = 'direct'
         DO UPDATE SET family_id = EXCLUDED.family_id
         RETURNING id",
    )
    .bind(family_id)
    .bind(user_a)
    .bind(user_b)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO chat_members (chat_id, user_id)
         VALUES ($1, $2), ($1, $3)
         ON CONFLICT DO NOTHING",
    )
    .bind(chat_id)
    .bind(user_a)
    .bind(user_b)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    let chat = Chat {
        id: chat_id,
        kind: "direct".to_string(),
        title: peer.get("display_name"),
        peer_user_id: Some(req.user_id),
    };
    Ok((StatusCode::OK, Json(json!({"chat": chat}))).into_response())
}

/// `GET /chats/{id}/messages` — keyset pagination on the message id.
pub async fn get_messages(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(chat_id): Path<i64>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, ApiError> {
    ensure_chat_access(&state, chat_id, auth.user_id).await?;

    let before_id = parse_pagination_param(&params, "before_id")?;
    let after_id = parse_pagination_param(&params, "after_id")?;
    let requested_limit = parse_pagination_param(&params, "limit")?;
    if before_id.is_some() && after_id.is_some() {
        return Err(ApiError::bad_request(
            codes::INVALID_PAGINATION,
            "before_id and after_id are mutually exclusive",
        ));
    }
    let limit = clamp_limit(
        requested_limit,
        state.cfg.limits.default_page_size,
        state.cfg.limits.max_page_size,
    );

    let rows = if let Some(before) = before_id {
        // History paging: strictly older, newest first.
        sqlx::query(&format!(
            "SELECT {MESSAGE_COLS} {MESSAGE_FROM} WHERE m.chat_id = $1 AND m.id < $2 ORDER BY m.id DESC LIMIT $3"
        ))
        .bind(chat_id)
        .bind(before)
        .bind(limit)
        .fetch_all(&state.pool)
        .await?
    } else if let Some(after) = after_id {
        // Reconnect catch-up: strictly newer, oldest first.
        sqlx::query(&format!(
            "SELECT {MESSAGE_COLS} {MESSAGE_FROM} WHERE m.chat_id = $1 AND m.id > $2 ORDER BY m.id ASC LIMIT $3"
        ))
        .bind(chat_id)
        .bind(after)
        .bind(limit)
        .fetch_all(&state.pool)
        .await?
    } else {
        sqlx::query(&format!(
            "SELECT {MESSAGE_COLS} {MESSAGE_FROM} WHERE m.chat_id = $1 ORDER BY m.id DESC LIMIT $2"
        ))
        .bind(chat_id)
        .bind(limit)
        .fetch_all(&state.pool)
        .await?
    };

    let mut messages: Vec<Message> = rows.iter().map(Message::from_row).collect();
    attach_reactions(&state, &mut messages).await?;
    attach_attachments(&state.pool, &mut messages).await?;
    attach_polls(&state.pool, &mut messages).await?;
    attach_calls(&state.pool, &mut messages).await?;

    Ok((StatusCode::OK, Json(json!({"messages": messages}))).into_response())
}

/// `POST /chats/{id}/messages` — 201 on first write, 200 on a dedup retry.
pub async fn post_message(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(chat_id): Path<i64>,
    headers: HeaderMap,
    AppJson(req): AppJson<PostMessageRequest>,
) -> Result<Response, ApiError> {
    // Which language to answer a question in. Read here rather than stored
    // on the account, because it is a property of the DEVICE asking: the
    // same person may run the app in Russian on their phone and English on
    // a work Mac, and each question should come back in the language it was
    // asked from.
    let language = headers
        .get(header::ACCEPT_LANGUAGE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let attachment_ids = merge_attachment_ids(req.attachment_id, req.attachment_ids)?;
    let (message, created) = create_message(
        &state,
        chat_id,
        auth.user_id,
        req.client_msg_id,
        &req.body,
        req.reply_to_message_id,
        &attachment_ids,
        req.poll.as_ref(),
        language.as_deref(),
    )
    .await?;
    if created {
        // REST has no originating WS connection: every connection of every
        // member — the sender's own devices included — receives `message`;
        // this HTTP response is the sender's ack.
        events::log_fanout_error(
            "new_message",
            events::deliver_new_message(&state, &message, None).await,
        );
    }
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(json!({"message": message}))).into_response())
}

/// `POST /chats/{id}/read`
pub async fn mark_read(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(chat_id): Path<i64>,
    AppJson(req): AppJson<ReadRequest>,
) -> Result<Response, ApiError> {
    let effective =
        apply_read_marker(&state, chat_id, auth.user_id, req.last_read_message_id).await?;
    // Relay the effective (post-GREATEST) marker so receivers never observe
    // a regression even when the client reported a stale value.
    events::log_fanout_error(
        "read",
        events::deliver_read(&state, chat_id, auth.user_id, effective).await,
    );
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `PUT /chats/{id}/messages/{mid}/reaction` — set/replace the caller's
/// reaction. The 200 body is the caller's ack; the `reaction` frame goes to
/// every member connection (the caller's other devices included).
pub async fn put_reaction(
    auth: AuthUser,
    State(state): State<AppState>,
    Path((chat_id, message_id)): Path<(i64, i64)>,
    AppJson(req): AppJson<ReactionRequest>,
) -> Result<Response, ApiError> {
    let reaction_state =
        apply_reaction(&state, chat_id, message_id, auth.user_id, Some(&req.emoji)).await?;
    respond_with_reaction_state(&state, reaction_state).await
}

/// `DELETE /chats/{id}/messages/{mid}/reaction` — remove the caller's
/// reaction; idempotent (removing nothing returns the state unchanged).
pub async fn delete_reaction(
    auth: AuthUser,
    State(state): State<AppState>,
    Path((chat_id, message_id)): Path<(i64, i64)>,
) -> Result<Response, ApiError> {
    let reaction_state = apply_reaction(&state, chat_id, message_id, auth.user_id, None).await?;
    respond_with_reaction_state(&state, reaction_state).await
}

async fn respond_with_reaction_state(
    state: &AppState,
    reaction_state: ReactionState,
) -> Result<Response, ApiError> {
    if reaction_state.changed {
        events::log_fanout_error(
            "reaction",
            events::deliver_reaction(state, &reaction_state).await,
        );
    }
    Ok((
        StatusCode::OK,
        Json(json!({
            "message_id": reaction_state.message_id,
            "reaction_seq": reaction_state.reaction_seq,
            "reactions": reaction_state.reactions,
        })),
    )
        .into_response())
}

/// `GET /chats/{id}/reactions` — reaction catch-up: every message of the
/// chat whose reaction state changed after `after_seq`, oldest change
/// first, full current state per message. Clients loop until a short page,
/// exactly as with `after_id`.
/// `PATCH /chats/{id}/messages/{mid}` — replace the body. The 200 is the
/// author's ack; `message_edited` goes to every member connection, the
/// author's other devices included.
pub async fn patch_message(
    auth: AuthUser,
    State(state): State<AppState>,
    Path((chat_id, message_id)): Path<(i64, i64)>,
    AppJson(req): AppJson<EditMessageRequest>,
) -> Result<Response, ApiError> {
    let outcome = apply_edit(&state, chat_id, message_id, auth.user_id, &req.body).await?;
    if outcome.changed {
        // No-ops fan nothing out: re-sending the body it already has is
        // not an event anyone needs to hear about.
        events::log_fanout_error(
            "message_edited",
            events::deliver_message_edited(&state, &outcome.message, None).await,
        );
    }
    Ok((StatusCode::OK, Json(json!({"message": outcome.message}))).into_response())
}

/// `GET /chats/{id}/edits?after_seq=` — the edit catch-up. Whole messages,
/// ordered by edit_seq, so a client applies them through exactly the path a
/// page of history takes.
pub async fn get_edits(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(chat_id): Path<i64>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, ApiError> {
    ensure_chat_access(&state, chat_id, auth.user_id).await?;

    let after_seq = parse_pagination_param(&params, "after_seq")?.unwrap_or(0);
    let requested_limit = parse_pagination_param(&params, "limit")?;
    let limit = clamp_limit(
        requested_limit,
        state.cfg.limits.default_page_size,
        state.cfg.limits.max_page_size,
    );

    let rows = sqlx::query(&format!(
        "SELECT {MESSAGE_COLS} {MESSAGE_FROM}
         WHERE m.chat_id = $1 AND m.edit_seq > $2
         ORDER BY m.edit_seq ASC LIMIT $3"
    ))
    .bind(chat_id)
    .bind(after_seq)
    .bind(limit)
    .fetch_all(&state.pool)
    .await?;

    let mut messages: Vec<Message> = rows.iter().map(Message::from_row).collect();
    attach_reactions(&state, &mut messages).await?;
    attach_attachments(&state.pool, &mut messages).await?;
    attach_polls(&state.pool, &mut messages).await?;
    attach_calls(&state.pool, &mut messages).await?;

    Ok((StatusCode::OK, Json(json!({"messages": messages}))).into_response())
}

pub async fn get_reactions(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(chat_id): Path<i64>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, ApiError> {
    ensure_chat_access(&state, chat_id, auth.user_id).await?;

    let after_seq = parse_pagination_param(&params, "after_seq")?.unwrap_or(0);
    let requested_limit = parse_pagination_param(&params, "limit")?;
    let limit = clamp_limit(
        requested_limit,
        state.cfg.limits.default_page_size,
        state.cfg.limits.max_page_size,
    );

    // The literal `reaction_seq > 0` predicate keeps the partial index
    // usable under a generic plan; `> $2` alone would not prove it.
    let message_rows = sqlx::query(
        "SELECT id, reaction_seq FROM messages
         WHERE chat_id = $1 AND reaction_seq > 0 AND reaction_seq > $2
         ORDER BY reaction_seq ASC LIMIT $3",
    )
    .bind(chat_id)
    .bind(after_seq)
    .bind(limit)
    .fetch_all(&state.pool)
    .await?;

    let ids: Vec<i64> = message_rows.iter().map(|row| row.get("id")).collect();
    let mut by_message: HashMap<i64, Vec<Reaction>> = HashMap::new();
    if !ids.is_empty() {
        let reaction_rows = sqlx::query(
            "SELECT message_id, user_id, emoji FROM message_reactions
             WHERE message_id = ANY($1)
             ORDER BY created_at, user_id",
        )
        .bind(&ids)
        .fetch_all(&state.pool)
        .await?;
        for row in &reaction_rows {
            by_message
                .entry(row.get("message_id"))
                .or_default()
                .push(Reaction {
                    user_id: row.get("user_id"),
                    emoji: row.get("emoji"),
                });
        }
    }

    let entries: Vec<serde_json::Value> = message_rows
        .iter()
        .map(|row| {
            let id: i64 = row.get("id");
            json!({
                "message_id": id,
                "reaction_seq": row.get::<i64, _>("reaction_seq"),
                "reactions": by_message.remove(&id).unwrap_or_default(),
            })
        })
        .collect();
    Ok((StatusCode::OK, Json(json!({"message_reactions": entries}))).into_response())
}

pub fn parse_pagination_param(
    params: &HashMap<String, String>,
    name: &'static str,
) -> Result<Option<i64>, ApiError> {
    match params.get(name) {
        None => Ok(None),
        Some(raw) => raw.parse::<i64>().map(Some).map_err(|_| {
            ApiError::bad_request(
                codes::INVALID_PAGINATION,
                format!("{name} must be an integer"),
            )
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordered_pair_sorts_both_argument_orders_the_same_way() {
        assert_eq!(ordered_pair(2, 9), (2, 9));
        assert_eq!(ordered_pair(9, 2), (2, 9));
    }

    #[test]
    fn clamp_limit_defaults_and_clamps_into_range() {
        assert_eq!(clamp_limit(None, 50, 200), 50, "default when unset");
        assert_eq!(
            clamp_limit(Some(75), 50, 200),
            75,
            "in-range passes through"
        );
        assert_eq!(clamp_limit(Some(1000), 50, 200), 200, "clamped to max");
        assert_eq!(clamp_limit(Some(0), 50, 200), 1, "floor of 1");
        assert_eq!(clamp_limit(Some(-5), 50, 200), 1, "negatives floor to 1");
    }

    #[test]
    fn pagination_params_parse_integers_and_reject_garbage() {
        let mut params = HashMap::new();
        params.insert("before_id".to_string(), "42".to_string());
        params.insert("limit".to_string(), "abc".to_string());
        assert_eq!(
            parse_pagination_param(&params, "before_id").expect("parses"),
            Some(42)
        );
        assert_eq!(
            parse_pagination_param(&params, "after_id").expect("absent is fine"),
            None
        );
        assert!(parse_pagination_param(&params, "limit").is_err());
    }

    /// The two spellings of "this message claims attachments" merge into
    /// one list — and sending BOTH is `validation`, pinned here so nobody
    /// ever picks a precedence for two values that could disagree.
    #[test]
    fn attachment_ids_and_the_legacy_spelling_merge_but_never_together() {
        assert_eq!(
            merge_attachment_ids(None, None).expect("neither is fine"),
            Vec::<i64>::new()
        );
        assert_eq!(
            merge_attachment_ids(Some(34), None).expect("the legacy spelling"),
            vec![34]
        );
        assert_eq!(
            merge_attachment_ids(None, Some(vec![34, 61])).expect("the array"),
            vec![34, 61]
        );
        // An empty array is the same statement as neither: no attachments.
        assert_eq!(
            merge_attachment_ids(None, Some(Vec::new())).expect("an empty array"),
            Vec::<i64>::new()
        );
        let err = merge_attachment_ids(Some(34), Some(vec![34])).expect_err("both is refused");
        assert!(
            matches!(err, ApiError::BadRequest { code, .. } if code == codes::VALIDATION),
            "both spellings at once must be `validation`: {err:?}"
        );
    }
}
