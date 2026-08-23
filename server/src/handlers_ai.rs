//! The assistant's half of a send (docs/protocol.md, "The assistant").
//!
//! Everything here runs AFTER the member's own message has been stored and
//! fanned out. The shape is deliberately boring:
//!
//!   1. store an EMPTY assistant message and fan it out, so every one of the
//!      member's devices has a row to fill;
//!   2. stream fragments to those devices as `ai_delta`;
//!   3. write the finished text to the row, which takes an `edit_seq` and
//!      fans out `message_edited`.
//!
//! Step 3 is what makes the deltas optional. A client that was asleep, on
//! another screen, or connected halfway through gets the whole reply from
//! the edits feed it already speaks — no special path, no resumable stream,
//! no "what did I miss" question to answer.

use anyhow::Result;
use sqlx::Row;
use tracing::warn;
use uuid::Uuid;

use crate::ai::{self, ChatTurn};
use crate::events;
use crate::models::Message;
use crate::state::AppState;
use crate::ws::ServerFrame;

/// The reserved account the assistant sends under (migration 0015). Looked
/// up rather than hard-coded: the id is whatever the sequence gave it.
pub async fn assistant_user_id(state: &AppState) -> Result<Option<i64>> {
    let id = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM users WHERE lower(username) = 'assistant' AND family_id IS NULL",
    )
    .fetch_optional(&state.pool)
    .await?;
    Ok(id)
}

/// Make sure this member has their assistant chat, and answer with its id.
///
/// Created on demand rather than at registration: a server that turns the
/// assistant on later should not need every member to re-register, and one
/// that never turns it on should not carry an empty chat in every list.
pub async fn ensure_ai_chat(state: &AppState, user_id: i64) -> Result<Option<i64>> {
    if !state.cfg.ai.is_usable() {
        return Ok(None);
    }
    let Some(family_id) =
        sqlx::query_scalar::<_, Option<i64>>("SELECT family_id FROM users WHERE id = $1")
            .fetch_optional(&state.pool)
            .await?
            .flatten()
    else {
        // No family, no chats of any kind — the assistant is not an
        // exception to that.
        return Ok(None);
    };

    let mut tx = state.pool.begin().await?;
    let existing =
        sqlx::query_scalar::<_, i64>("SELECT id FROM chats WHERE kind = 'ai' AND user_a_id = $1")
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?;
    if let Some(id) = existing {
        tx.commit().await?;
        return Ok(Some(id));
    }

    let chat_id: i64 = sqlx::query_scalar(
        "INSERT INTO chats (family_id, kind, user_a_id) VALUES ($1, 'ai', $2) RETURNING id",
    )
    .bind(family_id)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;
    // Only the owner is a member. That is what makes `GET /chats` return it
    // to them and nobody else, using the membership check every other chat
    // already goes through.
    sqlx::query("INSERT INTO chat_members (chat_id, user_id) VALUES ($1, $2)")
        .bind(chat_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Some(chat_id))
}

/// Answer a member's question. Spawned; never blocks their send.
pub fn spawn_reply(state: AppState, chat_id: i64, user_id: i64) {
    tokio::spawn(async move {
        if let Err(error) = reply(&state, chat_id, user_id).await {
            warn!(%chat_id, %error, "assistant reply failed");
        }
    });
}

async fn reply(state: &AppState, chat_id: i64, user_id: i64) -> Result<()> {
    let Some(assistant_id) = assistant_user_id(state).await? else {
        warn!("the assistant account is missing; run migrations");
        return Ok(());
    };

    // ONLY this member's own assistant thread. Not the family chat, not
    // another member's thread — this is the query that enforces what the
    // protocol promises, and it is the only place a request is built from.
    let history = sqlx::query(
        "SELECT sender_id, body FROM messages
         WHERE chat_id = $1 AND body <> ''
         ORDER BY id DESC
         LIMIT $2",
    )
    .bind(chat_id)
    .bind(state.cfg.ai.history_messages)
    .fetch_all(&state.pool)
    .await?;

    let mut turns: Vec<ChatTurn> = history
        .iter()
        .rev()
        .map(|row| {
            let body: String = row.get("body");
            if row.get::<i64, _>("sender_id") == assistant_id {
                ChatTurn::assistant(body)
            } else {
                ChatTurn::user(body)
            }
        })
        .collect();
    if turns.is_empty() {
        return Ok(());
    }
    // The newest turn must be the member's question; if the last row is the
    // assistant's own (a resend, a race) there is nothing to answer.
    if turns.last().map(|turn| turn.role) != Some("user") {
        return Ok(());
    }
    turns.truncate(state.cfg.ai.history_messages.max(1) as usize);

    // The empty row the reply will fill. Created BEFORE the call, so the
    // member sees the bubble appear immediately and every one of their
    // devices has the same id to stream into.
    let placeholder: Message = sqlx::query(
        "INSERT INTO messages (chat_id, sender_id, client_msg_id, body)
         VALUES ($1, $2, $3, '')
         RETURNING id, chat_id, sender_id, client_msg_id, body, created_at,
                   reaction_seq, edit_seq, edited_at, reply_to_message_id",
    )
    .bind(chat_id)
    .bind(assistant_id)
    .bind(Uuid::new_v4())
    .fetch_one(&state.pool)
    .await
    .map(|row| Message::from_row(&row))?;

    events::log_fanout_error(
        "ai_placeholder",
        events::deliver_new_message(state, &placeholder, None).await,
    );

    let message_id = placeholder.id;
    let owner = [user_id];
    let outcome = ai::stream_reply(&state.http, &state.cfg.ai, &turns, |delta| {
        let state = state.clone();
        let text = delta.to_string();
        // Fan-out is async and the callback is not; spawning keeps the
        // stream reading while frames go out, which is what stops a slow
        // socket from throttling the whole reply.
        tokio::spawn(async move {
            state
                .registry
                .fan_out(
                    &[user_id],
                    &ServerFrame::AiDelta {
                        chat_id,
                        message_id,
                        text,
                    },
                    None,
                )
                .await;
        });
    })
    .await;

    let (text, usage) = match outcome {
        Ok(result) => result,
        Err(error) => {
            warn!(%chat_id, %error, "assistant stream failed");
            state
                .registry
                .fan_out(
                    &owner,
                    &ServerFrame::AiError {
                        chat_id,
                        message_id,
                    },
                    None,
                )
                .await;
            return Ok(());
        }
    };

    if text.trim().is_empty() {
        state
            .registry
            .fan_out(
                &owner,
                &ServerFrame::AiError {
                    chat_id,
                    message_id,
                },
                None,
            )
            .await;
        return Ok(());
    }

    // The finished text goes on through the EDIT path, so catch-up carries
    // it for free (protocol.md, "Editing").
    let seq: i64 = sqlx::query_scalar("SELECT nextval('message_edit_seq')")
        .fetch_one(&state.pool)
        .await?;
    let updated: Message = sqlx::query(
        "UPDATE messages
         SET body = $1, edit_seq = $2, edited_at = now()
         WHERE id = $3
         RETURNING id, chat_id, sender_id, client_msg_id, body, created_at,
                   reaction_seq, edit_seq, edited_at, reply_to_message_id",
    )
    .bind(&text)
    .bind(seq)
    .bind(message_id)
    .fetch_one(&state.pool)
    .await
    .map(|row| Message::from_row(&row))?;

    sqlx::query("UPDATE chats SET last_edit_seq = $1 WHERE id = $2")
        .bind(seq)
        .bind(chat_id)
        .execute(&state.pool)
        .await?;

    // What it cost, for Family Statistics. Best effort: a reply the member
    // has already read must not fail because a counter did not save.
    let family_id =
        sqlx::query_scalar::<_, Option<i64>>("SELECT family_id FROM users WHERE id = $1")
            .fetch_optional(&state.pool)
            .await?
            .flatten();
    if let Some(family_id) = family_id
        && let Err(error) = sqlx::query(
            "INSERT INTO ai_usage
                 (user_id, family_id, message_id, prompt_tokens, completion_tokens)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(user_id)
        .bind(family_id)
        .bind(message_id)
        .bind(usage.prompt_tokens)
        .bind(usage.completion_tokens)
        .execute(&state.pool)
        .await
    {
        warn!(%error, "could not record assistant usage");
    }

    events::log_fanout_error(
        "ai_reply",
        events::deliver_message_edited(state, &updated, None).await,
    );
    Ok(())
}
