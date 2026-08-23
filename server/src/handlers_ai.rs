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
            .bind(user_id)
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
pub fn spawn_reply(state: AppState, chat_id: i64, user_id: i64, language: Option<String>) {
    tokio::spawn(async move {
        if let Err(error) = reply(&state, chat_id, user_id, language.as_deref()).await {
            warn!(%chat_id, %error, "assistant reply failed");
        }
    });
}

/// Turn an `Accept-Language` value into something worth telling a model.
///
/// Only the FIRST tag matters and only its primary subtag: the header can
/// be a whole weighted list (`ru-RU,ru;q=0.9,en;q=0.8`), and what is wanted
/// is "the language this device is in", not a negotiation.
///
/// Named rather than tagged where the name is known — "Russian" is a
/// clearer instruction to a model than "ru", and for anything unrecognised
/// the tag itself is still a better hint than nothing. Returns None for a
/// missing or unusable header, in which case nothing is added to the prompt
/// at all and the model simply answers in the language it was asked in.
pub fn language_instruction(header: Option<&str>) -> Option<String> {
    let tag = header?
        .split(',')
        .next()?
        .split(';')
        .next()?
        .trim()
        .to_ascii_lowercase();
    if tag.is_empty() || tag == "*" {
        return None;
    }
    let primary = tag.split('-').next().unwrap_or(&tag);
    let named = match primary {
        "en" => "English",
        "de" => "German",
        "es" => "Spanish",
        "fr" => "French",
        "ja" => "Japanese",
        "ru" => "Russian",
        "zh" => "Chinese",
        "sr" => "Serbian",
        _ => {
            return Some(format!(
                "Answer in the language with IETF tag \"{primary}\"."
            ));
        }
    };
    Some(format!("Answer in {named}."))
}

async fn reply(state: &AppState, chat_id: i64, user_id: i64, language: Option<&str>) -> Result<()> {
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
    // The family writes ONE system prompt; the language is appended per
    // question. That is deliberately not eight prompts to maintain: what
    // the assistant is FOR is the same in every language, and only the
    // language to answer in differs.
    let mut cfg = state.cfg.ai.clone();
    if let Some(instruction) = language_instruction(language) {
        if cfg.system_prompt.trim().is_empty() {
            cfg.system_prompt = instruction;
        } else {
            cfg.system_prompt = format!("{}\n\n{instruction}", cfg.system_prompt.trim_end());
        }
    }

    let outcome = ai::stream_reply(&state.http, &cfg, &turns, |delta| {
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
            .bind(user_id)
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

#[cfg(test)]
mod tests {
    use super::language_instruction;

    #[test]
    fn a_weighted_header_yields_only_the_first_language() {
        // Browsers and OSes send a whole negotiation list; what is wanted
        // is "the language this device is in", not the negotiation.
        assert_eq!(
            language_instruction(Some("ru-RU,ru;q=0.9,en-US;q=0.8,en;q=0.7")).as_deref(),
            Some("Answer in Russian.")
        );
        assert_eq!(
            language_instruction(Some("de-CH")).as_deref(),
            Some("Answer in German.")
        );
        // Serbian in either script is still Serbian.
        assert_eq!(
            language_instruction(Some("sr-Latn-RS")).as_deref(),
            Some("Answer in Serbian.")
        );
    }

    #[test]
    fn an_unknown_language_still_gets_its_tag() {
        // A family speaking something the app is not translated into should
        // still be answered in it — the tag is a better hint than nothing.
        assert_eq!(
            language_instruction(Some("pt-BR")).as_deref(),
            Some("Answer in the language with IETF tag \"pt\".")
        );
    }

    #[test]
    fn nothing_usable_adds_nothing_to_the_prompt() {
        // With no instruction the model simply answers in the language it
        // was asked in, which is the right fallback.
        assert_eq!(language_instruction(None), None);
        assert_eq!(language_instruction(Some("")), None);
        assert_eq!(language_instruction(Some("   ")), None);
        assert_eq!(language_instruction(Some("*")), None);
    }
}
