//! Polls: voting, closing, hydration and the catch-up feed
//! (docs/protocol.md, "Polls").
//!
//! A poll is not a kind of message — it is an ordinary `messages` row with a
//! `polls` row beside it, and its QUESTION is the message body. Creating one
//! therefore lives in `handlers_chat::create_message`, inside the same
//! transaction as the message insert, and not here: there is one write path
//! for "post a message" and a poll must not become a second. What lives here
//! is everything that happens to a poll AFTER it exists.
//!
//! Every change to a poll — a vote, a retraction, a close — takes the next
//! value of `message_poll_seq` and stamps it on the poll, with the chat's
//! `last_poll_seq` following GREATEST-guarded. That is the fourth cursor of
//! the same shape as reactions, edits and the board, for the fourth time the
//! same reason: `after_id` is `WHERE id > cursor` and can never see a change
//! to an older row, and a poll is nothing but changes to an older row.
//!
//! Both the frame and the feed carry a poll's FULL CURRENT STATE, never a
//! delta, so ordering races resolve client-side under the `poll_seq` guard.

use std::collections::HashMap;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;
use sqlx::{PgPool, Row};

use crate::auth::AuthUser;
use crate::error::{ApiError, AppJson, codes};
use crate::events;
use crate::handlers_chat::{clamp_limit, ensure_chat_access, parse_pagination_param};
use crate::models::{Message, Poll, PollOption};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct VoteRequest {
    pub option_id: i64,
}

/// A poll's full current state after a mutation — what the REST response
/// returns and what the `poll` frame carries. `changed` tells the caller
/// whether anything actually moved (no fan-out on no-ops).
pub struct PollState {
    pub chat_id: i64,
    pub message_id: i64,
    pub poll: Poll,
    pub changed: bool,
}

/// GREATEST, not plain SET: two polls in one chat can commit out of seq
/// order, and the chat's cursor must never move backwards. A cursor that
/// went backwards would make a client's `max_poll_seq > local` guard read
/// false and stop it reading the feed for good.
pub async fn advance_chat_poll_seq(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    chat_id: i64,
    seq: i64,
) -> Result<(), ApiError> {
    sqlx::query("UPDATE chats SET last_poll_seq = GREATEST(last_poll_seq, $2) WHERE id = $1")
        .bind(chat_id)
        .bind(seq)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Write the poll rows for a message that has just been inserted, and
/// return the state the ack and the fan-out carry.
///
/// Runs inside the caller's transaction, which is load-bearing for the same
/// reason `claim_attachment` is: a poll insert that failed after an
/// autocommitted message would leave a permanent options-less bubble that is
/// also the chat's newest message.
///
/// `chat_id` comes from the message row that was just written, never from
/// the request — nothing in the schema ties `polls.chat_id` to
/// `messages.chat_id`, and the catch-up feed reads by `chat_id`.
pub async fn create_poll(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    chat_id: i64,
    message_id: i64,
    options: &[String],
) -> Result<Poll, ApiError> {
    let poll_seq: i64 = sqlx::query_scalar("SELECT nextval('message_poll_seq')")
        .fetch_one(&mut **tx)
        .await?;
    sqlx::query("INSERT INTO polls (message_id, chat_id, poll_seq) VALUES ($1, $2, $3)")
        .bind(message_id)
        .bind(chat_id)
        .bind(poll_seq)
        .execute(&mut **tx)
        .await?;
    // One statement for every option rather than a round trip each:
    // WITH ORDINALITY turns the array's order into `position`, which is what
    // fixes creation order for the life of the poll.
    let rows = sqlx::query(
        "INSERT INTO poll_options (poll_id, position, text)
         SELECT $1, (opt.ord - 1)::smallint, opt.text
         FROM unnest($2::text[]) WITH ORDINALITY AS opt(text, ord)
         RETURNING id, position, text",
    )
    .bind(message_id)
    .bind(options)
    .fetch_all(&mut **tx)
    .await?;
    advance_chat_poll_seq(tx, chat_id, poll_seq).await?;

    // Sorted here rather than trusted from RETURNING: `position` is the
    // authority on order everywhere else a poll is read, and this is the one
    // place the two could quietly disagree.
    let mut options: Vec<(i16, PollOption)> = rows
        .iter()
        .map(|row| {
            (
                row.get::<i16, _>("position"),
                PollOption {
                    id: row.get("id"),
                    text: row.get("text"),
                    votes: Vec::new(),
                },
            )
        })
        .collect();
    options.sort_by_key(|(position, _)| *position);

    Ok(Poll {
        poll_seq,
        closed: false,
        options: options.into_iter().map(|(_, option)| option).collect(),
    })
}

/// One poll's full current state, in ONE query, so it can run inside the
/// caller's transaction and see what that transaction has just written.
///
/// Options come back in `position` order and each option's votes in
/// `(created_at, user_id)` order — a poll rendered twice must not shuffle,
/// and the frame is compared field by field by clients that hold it.
pub async fn fetch_poll<'e, E>(executor: E, message_id: i64) -> Result<Poll, ApiError>
where
    E: sqlx::PgExecutor<'e>,
{
    let rows = sqlx::query(
        "SELECT p.poll_seq,
                (p.closed_at IS NOT NULL) AS closed,
                o.id   AS option_id,
                o.text AS option_text,
                ARRAY(SELECT v.user_id FROM poll_votes v
                       WHERE v.option_id = o.id
                       ORDER BY v.created_at, v.user_id) AS votes
         FROM polls p
         LEFT JOIN poll_options o ON o.poll_id = p.message_id
         WHERE p.message_id = $1
         ORDER BY o.position",
    )
    .bind(message_id)
    .fetch_all(executor)
    .await?;

    // Unreachable in practice: every caller has already located the poll
    // row, under `FOR UPDATE` in the mutating ones. A 404 is still the
    // honest answer if the row vanished under us.
    let Some(first) = rows.first() else {
        return Err(ApiError::not_found(
            codes::MESSAGE_NOT_FOUND,
            "no such poll in this chat",
        ));
    };
    // LEFT JOIN, so a poll with no options at all still yields its header
    // row. The schema does not forbid that state; validation does.
    let options = rows
        .iter()
        .filter_map(|row| {
            row.get::<Option<i64>, _>("option_id").map(|id| PollOption {
                id,
                text: row.get("option_text"),
                votes: row.get("votes"),
            })
        })
        .collect();
    Ok(Poll {
        poll_seq: first.get("poll_seq"),
        closed: first.get("closed"),
        options,
    })
}

/// A poll's header: everything about it that is not its options.
struct PollHeader {
    message_id: i64,
    poll_seq: i64,
    closed: bool,
}

fn header_from_row(row: &sqlx::postgres::PgRow) -> PollHeader {
    PollHeader {
        message_id: row.get("message_id"),
        poll_seq: row.get("poll_seq"),
        closed: row.get("closed"),
    }
}

/// Fill options and votes in for a whole page of poll headers: TWO `= ANY`
/// queries for the page, never one per poll.
async fn hydrate_headers(
    pool: &PgPool,
    headers: &[PollHeader],
) -> Result<HashMap<i64, Poll>, ApiError> {
    if headers.is_empty() {
        return Ok(HashMap::new());
    }
    let poll_ids: Vec<i64> = headers.iter().map(|header| header.message_id).collect();

    let vote_rows = sqlx::query(
        "SELECT option_id, user_id FROM poll_votes
         WHERE poll_id = ANY($1)
         ORDER BY created_at, user_id",
    )
    .bind(&poll_ids)
    .fetch_all(pool)
    .await?;
    let mut votes_by_option: HashMap<i64, Vec<i64>> = HashMap::new();
    for row in &vote_rows {
        votes_by_option
            .entry(row.get("option_id"))
            .or_default()
            .push(row.get("user_id"));
    }

    let option_rows = sqlx::query(
        "SELECT poll_id, id, text FROM poll_options
         WHERE poll_id = ANY($1)
         ORDER BY poll_id, position",
    )
    .bind(&poll_ids)
    .fetch_all(pool)
    .await?;
    let mut options_by_poll: HashMap<i64, Vec<PollOption>> = HashMap::new();
    for row in &option_rows {
        let id: i64 = row.get("id");
        options_by_poll
            .entry(row.get("poll_id"))
            .or_default()
            .push(PollOption {
                id,
                text: row.get("text"),
                votes: votes_by_option.remove(&id).unwrap_or_default(),
            });
    }

    Ok(headers
        .iter()
        .map(|header| {
            (
                header.message_id,
                Poll {
                    poll_seq: header.poll_seq,
                    closed: header.closed,
                    options: options_by_poll
                        .remove(&header.message_id)
                        .unwrap_or_default(),
                },
            )
        })
        .collect())
}

/// Fill the `poll` object in on every message of a page that has one.
///
/// Three queries for the whole page — headers, options, votes — never one
/// per message, exactly as `attach_reactions` does it. Messages that are not
/// polls are left alone, and the key stays absent on the wire.
pub async fn attach_polls(pool: &PgPool, messages: &mut [Message]) -> Result<(), ApiError> {
    if messages.is_empty() {
        return Ok(());
    }
    let message_ids: Vec<i64> = messages.iter().map(|message| message.id).collect();
    let header_rows = sqlx::query(
        "SELECT message_id, poll_seq, (closed_at IS NOT NULL) AS closed FROM polls
         WHERE message_id = ANY($1)",
    )
    .bind(&message_ids)
    .fetch_all(pool)
    .await?;
    let headers: Vec<PollHeader> = header_rows.iter().map(header_from_row).collect();
    let mut polls = hydrate_headers(pool, &headers).await?;
    for message in messages.iter_mut() {
        if let Some(poll) = polls.remove(&message.id) {
            message.poll = Some(poll);
        }
    }
    Ok(())
}

/// Set (`Some(option_id)`) or retract (`None`) the caller's vote — an
/// idempotent state-set, one choice per member, never a toggle.
///
/// The transaction locks the poll row *first*, and that is deliberate: it
/// serializes concurrent voters so the state read below sees every committed
/// vote (without it, two READ COMMITTED writers each ship a frame missing
/// the other's row), and it doubles as the exists-and-belongs-to-this-chat
/// check. Lock order is uniform — poll row, then chat row — and nothing else
/// takes both, so there is no deadlock.
pub async fn apply_vote(
    state: &AppState,
    chat_id: i64,
    message_id: i64,
    user_id: i64,
    option_id: Option<i64>,
) -> Result<PollState, ApiError> {
    ensure_chat_access(state, chat_id, user_id).await?;

    let mut tx = state.pool.begin().await?;
    let closed = lock_poll(&mut tx, chat_id, message_id).await?;
    if closed {
        return Err(ApiError::conflict(
            codes::POLL_CLOSED,
            "this poll is closed",
        ));
    }

    // The option must belong to THIS poll. Without the `poll_id` half a
    // member could vote for an option on a poll in a chat they cannot see,
    // and the tally would name them there.
    if let Some(option_id) = option_id {
        let belongs: Option<i32> =
            sqlx::query_scalar("SELECT 1 FROM poll_options WHERE id = $1 AND poll_id = $2")
                .bind(option_id)
                .bind(message_id)
                .fetch_optional(&mut *tx)
                .await?;
        if belongs.is_none() {
            return Err(ApiError::bad_request(
                codes::INVALID_POLL,
                "no such option on this poll",
            ));
        }
    }

    let current: Option<i64> =
        sqlx::query_scalar("SELECT option_id FROM poll_votes WHERE poll_id = $1 AND user_id = $2")
            .bind(message_id)
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?;
    // Re-PUT of the choice already held, or a retraction of nothing: no
    // sequence value burned and no fan-out raised.
    let changed = current != option_id;
    if changed {
        // Delete-then-insert rather than an upsert: it is the one shape that
        // says "set my choice to this" and "clear my choice" with the same
        // two statements, and `created_at` should be when this vote was
        // cast, not when the member first voted for something else.
        sqlx::query("DELETE FROM poll_votes WHERE poll_id = $1 AND user_id = $2")
            .bind(message_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        if let Some(option_id) = option_id {
            sqlx::query("INSERT INTO poll_votes (poll_id, option_id, user_id) VALUES ($1, $2, $3)")
                .bind(message_id)
                .bind(option_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
        }
        stamp_new_seq(&mut tx, chat_id, message_id).await?;
    }

    // Read from INSIDE the transaction, so the state that is broadcast is
    // the state that was committed — and, in the unchanged case, carries the
    // poll's CURRENT seq rather than a zero.
    let poll = fetch_poll(&mut *tx, message_id).await?;
    tx.commit().await?;

    Ok(PollState {
        chat_id,
        message_id,
        poll,
        changed,
    })
}

/// Close a poll: the author's act, and one-way (protocol.md, "Polls").
///
/// The family owner does not outrank the author here, for the same reason
/// they cannot edit or delete anybody else's message anywhere else in this
/// protocol. Closing a closed poll is a no-op.
pub async fn close_poll(
    state: &AppState,
    chat_id: i64,
    message_id: i64,
    user_id: i64,
) -> Result<PollState, ApiError> {
    ensure_chat_access(state, chat_id, user_id).await?;

    let mut tx = state.pool.begin().await?;
    let already_closed = lock_poll(&mut tx, chat_id, message_id).await?;

    let sender_id: Option<i64> = sqlx::query_scalar("SELECT sender_id FROM messages WHERE id = $1")
        .bind(message_id)
        .fetch_optional(&mut *tx)
        .await?;
    if sender_id != Some(user_id) {
        return Err(ApiError::forbidden(
            codes::NOT_MESSAGE_AUTHOR,
            "only the author can close this poll",
        ));
    }

    let changed = !already_closed;
    if changed {
        sqlx::query("UPDATE polls SET closed_at = now() WHERE message_id = $1")
            .bind(message_id)
            .execute(&mut *tx)
            .await?;
        stamp_new_seq(&mut tx, chat_id, message_id).await?;
    }

    let poll = fetch_poll(&mut *tx, message_id).await?;
    tx.commit().await?;

    Ok(PollState {
        chat_id,
        message_id,
        poll,
        changed,
    })
}

/// Lock the poll row and report whether it is closed.
///
/// `FOR UPDATE` before anything else: it serializes concurrent writers so
/// the state read afterwards sees every committed change, and it doubles as
/// the exists-and-belongs-to-this-chat check. A poll in another chat is
/// answered exactly like one that does not exist — the endpoint never
/// confirms an id the caller cannot otherwise see.
async fn lock_poll(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    chat_id: i64,
    message_id: i64,
) -> Result<bool, ApiError> {
    let locked = sqlx::query(
        "SELECT (closed_at IS NOT NULL) AS closed FROM polls
         WHERE message_id = $1 AND chat_id = $2 FOR UPDATE",
    )
    .bind(message_id)
    .bind(chat_id)
    .fetch_optional(&mut **tx)
    .await?;
    let Some(locked) = locked else {
        return Err(ApiError::not_found(
            codes::MESSAGE_NOT_FOUND,
            "no such poll in this chat",
        ));
    };
    Ok(locked.get("closed"))
}

/// Stamp the next `message_poll_seq` on the poll and advance the chat's
/// cursor with it. Every change to a poll takes one — a vote, a retraction,
/// a close — because the catch-up feed is ordered by nothing else.
///
/// `pub(crate)` for the account deletion, which retracts a departing
/// member's votes from every still-open poll inside its own transaction
/// (protocol.md, "Deleting an account"). That path cannot come through
/// `apply_vote`: `apply_vote` opens by checking the VOTER's access to the
/// chat, and a member whose `chat_members` rows have just been deleted has
/// none.
pub(crate) async fn stamp_new_seq(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    chat_id: i64,
    message_id: i64,
) -> Result<(), ApiError> {
    let seq: i64 = sqlx::query_scalar("SELECT nextval('message_poll_seq')")
        .fetch_one(&mut **tx)
        .await?;
    sqlx::query("UPDATE polls SET poll_seq = $2 WHERE message_id = $1")
        .bind(message_id)
        .bind(seq)
        .execute(&mut **tx)
        .await?;
    advance_chat_poll_seq(tx, chat_id, seq).await
}

/// `PUT /chats/{id}/messages/{mid}/vote` — set the caller's choice. The 200
/// body is the caller's ack; the `poll` frame goes to every member
/// connection (the caller's other devices included).
pub async fn put_vote(
    auth: AuthUser,
    State(state): State<AppState>,
    Path((chat_id, message_id)): Path<(i64, i64)>,
    AppJson(req): AppJson<VoteRequest>,
) -> Result<Response, ApiError> {
    let poll_state = apply_vote(
        &state,
        chat_id,
        message_id,
        auth.user_id,
        Some(req.option_id),
    )
    .await?;
    respond_with_poll_state(&state, poll_state).await
}

/// `DELETE /chats/{id}/messages/{mid}/vote` — retract the caller's vote;
/// idempotent (retracting nothing returns the state unchanged).
pub async fn delete_vote(
    auth: AuthUser,
    State(state): State<AppState>,
    Path((chat_id, message_id)): Path<(i64, i64)>,
) -> Result<Response, ApiError> {
    let poll_state = apply_vote(&state, chat_id, message_id, auth.user_id, None).await?;
    respond_with_poll_state(&state, poll_state).await
}

/// `POST /chats/{id}/messages/{mid}/poll/close` — author only, one-way.
pub async fn close_poll_handler(
    auth: AuthUser,
    State(state): State<AppState>,
    Path((chat_id, message_id)): Path<(i64, i64)>,
) -> Result<Response, ApiError> {
    let poll_state = close_poll(&state, chat_id, message_id, auth.user_id).await?;
    respond_with_poll_state(&state, poll_state).await
}

async fn respond_with_poll_state(
    state: &AppState,
    poll_state: PollState,
) -> Result<Response, ApiError> {
    if poll_state.changed {
        // No-ops fan nothing out: re-voting for the option you already hold
        // is not an event anyone needs to hear about.
        events::log_fanout_error("poll", events::deliver_poll(state, &poll_state).await);
    }
    Ok((
        StatusCode::OK,
        Json(json!({
            "message_id": poll_state.message_id,
            "poll": poll_state.poll,
        })),
    )
        .into_response())
}

/// `GET /chats/{id}/polls?after_seq=` — the poll catch-up: every poll of the
/// chat whose state changed after `after_seq`, oldest change first, full
/// current state per poll. Clients loop until a short page, exactly as with
/// `after_id`.
pub async fn get_polls(
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

    let header_rows = sqlx::query(
        "SELECT message_id, poll_seq, (closed_at IS NOT NULL) AS closed FROM polls
         WHERE chat_id = $1 AND poll_seq > $2
         ORDER BY poll_seq ASC LIMIT $3",
    )
    .bind(chat_id)
    .bind(after_seq)
    .bind(limit)
    .fetch_all(&state.pool)
    .await?;

    let headers: Vec<PollHeader> = header_rows.iter().map(header_from_row).collect();
    let mut polls = hydrate_headers(&state.pool, &headers).await?;
    let entries: Vec<serde_json::Value> = headers
        .iter()
        .filter_map(|header| {
            polls.remove(&header.message_id).map(|poll| {
                json!({
                    "message_id": header.message_id,
                    "poll": poll,
                })
            })
        })
        .collect();

    Ok((StatusCode::OK, Json(json!({"polls": entries}))).into_response())
}
