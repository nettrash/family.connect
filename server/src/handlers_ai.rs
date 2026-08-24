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

use std::time::Duration;

use anyhow::Result;
use sqlx::Row;
use tokio::sync::mpsc;
use tokio::time::sleep;
use tracing::warn;
use uuid::Uuid;

use crate::ai::{self, ChatTurn};
use crate::events;
use crate::handlers_chat;
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
        // Existing is not the same as reachable. Leaving a family deletes
        // the caller's row from `chat_members` for EVERY chat of that
        // family, this one included (handlers_family::remove_membership) —
        // so without restoring it here a member who left and came back is
        // locked out of their own assistant thread forever, by a 403 on a
        // chat nobody else can even see. protocol.md promises history
        // "resurfaces on rejoin"; this is what makes that true here.
        //
        // The family_id is re-pointed for the same reason the direct-chat
        // upsert re-points its own: the thread belongs to the member, not
        // to the family they were in when they started it, and a stale
        // family id would take it down with a family they have left.
        sqlx::query("UPDATE chats SET family_id = $1 WHERE id = $2 AND family_id <> $1")
            .bind(family_id)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO chat_members (chat_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        )
        .bind(id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
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

/// Answer a member's question in their own assistant thread. Spawned;
/// never blocks their send.
pub fn spawn_reply(state: AppState, chat_id: i64, user_id: i64, language: Option<String>) {
    tokio::spawn(async move {
        if let Err(error) = reply(&state, chat_id, user_id, language.as_deref()).await {
            warn!(%chat_id, %error, "assistant reply failed");
        }
    });
}

/// Answer an `@ai` mention in the family chat. Spawned, like the private
/// one, and for the same reason: a reply takes seconds and the member's
/// send must return at once.
pub fn spawn_mention_reply(
    state: AppState,
    chat_id: i64,
    user_id: i64,
    message_id: i64,
    language: Option<String>,
) {
    tokio::spawn(async move {
        if let Err(error) =
            mention_reply(&state, chat_id, user_id, message_id, language.as_deref()).await
        {
            warn!(%chat_id, %message_id, %error, "assistant mention reply failed");
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

/// The system prompt for one question: the family's own, plus the notes
/// this particular question needs, plus the language to answer in.
///
/// The family writes ONE prompt — what the assistant is FOR does not change
/// with the situation, only what it should know about THIS question does —
/// and the language goes LAST on purpose. Everything above it is English
/// (the configured prompt usually is, and the mention note certainly is),
/// and a model handed a paragraph of English instructions will answer in
/// English unless the last thing it reads says otherwise.
///
/// Its own function so the composition can be tested. It used to be inline
/// in `answer`, where "does a family-chat mention carry the asker's
/// language?" was a question nothing could answer without a live provider.
pub fn compose_system_prompt(configured: &str, notes: &[String], language: Option<&str>) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(notes.len() + 2);
    if !configured.trim().is_empty() {
        parts.push(configured.trim_end().to_string());
    }
    parts.extend(notes.iter().cloned());
    if let Some(instruction) = language_instruction(language) {
        parts.push(instruction);
    }
    parts.join("\n\n")
}

/// What the assistant is told about being mentioned in a group chat.
///
/// Worth saying explicitly, because the honest description of what it can
/// see is unusual enough that a model will otherwise assume it is missing
/// context it ought to have and answer as if it had read the thread.
/// Everything after "you cannot see" is what stops it inventing one.
pub const MENTION_INSTRUCTION: &str = "You have been mentioned with @ai in a family group chat. \
     You can see ONLY the message that mentioned you, and the message it quotes when there is one; \
     you cannot see the rest of the conversation, who else is in it, or anything said earlier. \
     If answering needs context you were not given, say so in one line and ask for it. \
     Keep the answer short — this is a group chat, not a document. \
     Answer in the language the message you were sent is written in, whatever language these \
     instructions are in.";

/// One question, prepared: what goes to the model, who watches it arrive,
/// and what the answer should quote.
///
/// Both entry points build one of these and hand it to `answer`, so the
/// half that talks to the provider and writes the row exists once. The
/// half that decides WHAT LEAVES THE SERVER is deliberately the half that
/// differs — see `thread_prompt` and `mention_prompt`.
struct Prompt {
    turns: Vec<ChatTurn>,
    /// Appended to the configured system prompt for this request only.
    notes: Vec<String>,
    /// The message the answer quotes, if any.
    reply_to: Option<i64>,
    /// Who receives the streaming fragments and any error frame.
    audience: Vec<i64>,
}

async fn reply(state: &AppState, chat_id: i64, user_id: i64, language: Option<&str>) -> Result<()> {
    let Some(assistant_id) = assistant_user_id(state).await? else {
        warn!("the assistant account is missing; run migrations");
        return Ok(());
    };
    let Some(prompt) = thread_prompt(state, chat_id, user_id, assistant_id).await? else {
        return Ok(());
    };
    answer(state, chat_id, user_id, assistant_id, prompt, language).await
}

async fn mention_reply(
    state: &AppState,
    chat_id: i64,
    user_id: i64,
    message_id: i64,
    language: Option<&str>,
) -> Result<()> {
    let Some(assistant_id) = assistant_user_id(state).await? else {
        warn!("the assistant account is missing; run migrations");
        return Ok(());
    };
    let Some(prompt) = mention_prompt(state, chat_id, message_id).await? else {
        return Ok(());
    };
    answer(state, chat_id, user_id, assistant_id, prompt, language).await
}

/// The private thread: the last N turns of THIS member's own assistant chat.
///
/// ONLY that. Not the family chat, not another member's thread — this is the
/// query that enforces what the protocol promises for a `kind = 'ai'` chat,
/// and it is the only place a thread request is built from.
async fn thread_prompt(
    state: &AppState,
    chat_id: i64,
    user_id: i64,
    assistant_id: i64,
) -> Result<Option<Prompt>> {
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
        return Ok(None);
    }
    // The newest turn must be the member's question; if the last row is the
    // assistant's own (a resend, a race) there is nothing to answer.
    if turns.last().map(|turn| turn.role) != Some("user") {
        return Ok(None);
    }
    turns.truncate(state.cfg.ai.history_messages.max(1) as usize);

    Ok(Some(Prompt {
        turns,
        notes: Vec::new(),
        reply_to: None,
        // A private thread is private in both directions: nobody else sees
        // it arrive, so nobody else is streamed to.
        audience: vec![user_id],
    }))
}

/// A mention in the family chat: **one message**, and the message it quotes
/// when the member deliberately replied to one.
///
/// This is the whole of what leaves the server, and it is a different query
/// from `thread_prompt` on purpose rather than the same one with a different
/// bind. Reusing that query here — the obvious edit — would ship the entire
/// family chat to the provider on the first mention, silently, with nothing
/// failing. Keeping them apart is what makes the scope reviewable at a
/// glance (docs/protocol.md, "Mentioning the assistant in the family chat").
///
/// The quoted message is included because the member CHOSE it by replying:
/// "@ai what does this mean?" is not answerable without it, and the choice
/// is an explicit act by someone in the chat. Nothing else — not the
/// surrounding messages, not the roster, not other members' names — goes.
async fn mention_prompt(state: &AppState, chat_id: i64, message_id: i64) -> Result<Option<Prompt>> {
    let row = sqlx::query(
        "SELECT m.body,
                p.body AS quoted_body,
                qu.display_name AS quoted_author
         FROM messages m
         LEFT JOIN messages p ON p.id = m.reply_to_message_id AND p.chat_id = m.chat_id
         LEFT JOIN users qu ON qu.id = p.sender_id
         WHERE m.id = $1 AND m.chat_id = $2",
    )
    .bind(message_id)
    .bind(chat_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some(row) = row else {
        // Swept by retention, or deleted between the send and this task.
        return Ok(None);
    };

    let body: String = row.get("body");
    if body.trim().is_empty() {
        return Ok(None);
    }
    let quoted: Option<String> = row.get("quoted_body");
    let quoted_author: Option<String> = row.get("quoted_author");

    let content = match (quoted, quoted_author) {
        (Some(quoted), Some(author)) if !quoted.trim().is_empty() => format!(
            "[The member replied to this message from {author}]\n{quoted}\n[End of quoted message]\n\n{body}"
        ),
        _ => body,
    };

    let audience = events::chat_member_ids(&state.pool, chat_id)
        .await
        .unwrap_or_default();

    Ok(Some(Prompt {
        turns: vec![ChatTurn::user(content)],
        notes: vec![MENTION_INSTRUCTION.to_string()],
        // The answer quotes the question. In a chat where several
        // conversations run at once, an unattached answer belongs to
        // nobody — and the quote is how a member scrolling back sees what
        // was asked.
        reply_to: Some(message_id),
        // Everyone in the family chat watches it arrive, because everyone
        // can already read the answer once it lands.
        audience,
    }))
}

/// How often streaming fragments may leave the server, per reply.
///
/// The provider emits fragments far faster than this — a word at a time —
/// and each one used to become its own frame to each recipient. In a
/// private thread that was one member's own socket; in the family chat it
/// is every connected member's, and a socket whose outbound queue fills is
/// KICKED (registry.rs) rather than buffered. Coalescing whatever arrived
/// during one tick into a single frame bounds that at ~8 frames a second
/// per reply, which still reads as typing.
const DELTA_INTERVAL: Duration = Duration::from_millis(120);

/// The half that is the same for both kinds of question: create the row,
/// stream into it, and finish it through the edit path.
async fn answer(
    state: &AppState,
    chat_id: i64,
    user_id: i64,
    assistant_id: i64,
    prompt: Prompt,
    language: Option<&str>,
) -> Result<()> {
    // The empty row the reply will fill. Created BEFORE the call, so the
    // bubble appears immediately and every device has the same id to stream
    // into.
    let inserted: Message = sqlx::query(
        "INSERT INTO messages (chat_id, sender_id, client_msg_id, body, reply_to_message_id)
         VALUES ($1, $2, $3, '', $4)
         RETURNING id, chat_id, sender_id, client_msg_id, body, created_at,
                   reaction_seq, edit_seq, edited_at, reply_to_message_id",
    )
    .bind(chat_id)
    .bind(assistant_id)
    .bind(Uuid::new_v4())
    .bind(prompt.reply_to)
    .fetch_one(&state.pool)
    .await
    .map(|row| Message::from_row(&row))?;

    let message_id = inserted.id;
    // Re-read through the joined reader when the row quotes something: the
    // RETURNING clause above cannot join, so the `reply_to` snippet would
    // serialize as absent and no client would draw the quote.
    let placeholder = match prompt.reply_to {
        None => inserted,
        Some(_) => handlers_chat::fetch_message(state, chat_id, message_id)
            .await
            .ok()
            .flatten()
            .unwrap_or(inserted),
    };

    // Delivered WITHOUT a notification. The row is empty — an alert saying
    // the assistant sent a blank line is worse than no alert — and the
    // finished text arrives as an edit, which never pushes. The alert is
    // raised at the end instead, by `push_message_late`.
    events::log_fanout_error(
        "ai_placeholder",
        events::deliver_message_without_push(state, &placeholder, None).await,
    );

    let mut cfg = state.cfg.ai.clone();
    cfg.system_prompt = compose_system_prompt(&cfg.system_prompt, &prompt.notes, language);

    // One ordered pump, rather than a task per fragment.
    //
    // Two things were wrong with spawning per fragment, and the family chat
    // makes both worse: nothing ordered the spawned tasks, so "Sure — the"
    // could reach a device after " weather", and nothing bounded the frame
    // rate. Draining one channel from one task fixes the order by
    // construction, and coalescing whatever is queued fixes the rate.
    let (deltas, mut queue) = mpsc::unbounded_channel::<String>();
    let pump = {
        let state = state.clone();
        let audience = prompt.audience.clone();
        tokio::spawn(async move {
            while let Some(first) = queue.recv().await {
                let mut text = first;
                while let Ok(more) = queue.try_recv() {
                    text.push_str(&more);
                }
                if !text.is_empty() {
                    state
                        .registry
                        .fan_out(
                            &audience,
                            &ServerFrame::AiDelta {
                                chat_id,
                                message_id,
                                text,
                            },
                            None,
                        )
                        .await;
                }
                // Sleep AFTER sending, so the first fragment is not held
                // back and everything that arrives meanwhile rides the
                // next frame.
                sleep(DELTA_INTERVAL).await;
            }
        })
    };

    let outcome = ai::stream_reply(&state.http, &cfg, &prompt.turns, |delta| {
        // A closed channel means the pump is gone; the row is still the
        // truth, so losing a cosmetic fragment is not worth an error.
        let _ = deltas.send(delta.to_string());
    })
    .await;

    // Drop the sender, then WAIT. Every fragment must be on its way before
    // the finished body goes out: a delta arriving after `message_edited`
    // would be appended to the complete text by clients that treat the row
    // as an accumulator, duplicating a phrase in the answer.
    drop(deltas);
    let _ = pump.await;

    let (text, usage) = match outcome {
        Ok(result) => result,
        Err(error) => {
            warn!(%chat_id, %error, "assistant stream failed");
            state
                .registry
                .fan_out(
                    &prompt.audience,
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
                &prompt.audience,
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
    sqlx::query(
        "UPDATE messages
         SET body = $1, edit_seq = $2, edited_at = now()
         WHERE id = $3",
    )
    .bind(&text)
    .bind(seq)
    .bind(message_id)
    .execute(&state.pool)
    .await?;

    sqlx::query("UPDATE chats SET last_edit_seq = $1 WHERE id = $2")
        .bind(seq)
        .bind(chat_id)
        .execute(&state.pool)
        .await?;

    // Read the finished row back through the joined reader, so the quote
    // rides along with the edit exactly as it does on any other read.
    let updated = match handlers_chat::fetch_message(state, chat_id, message_id).await {
        Ok(Some(message)) => message,
        _ => {
            let mut fallback = placeholder;
            fallback.body = text;
            fallback.edit_seq = Some(seq);
            fallback
        }
    };

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
    // NOW the alert, with something in it to read. Suppressed on the
    // placeholder and skipped by the edit frame, this is the only push the
    // assistant ever raises — one per answer, never two.
    events::log_fanout_error(
        "ai_reply_push",
        events::push_message_late(state, &updated).await,
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{MENTION_INSTRUCTION, compose_system_prompt, language_instruction};

    /// A family-chat mention must come back in the language it was asked
    /// in, exactly as a question in the private thread does.
    ///
    /// The wiring is easy to have and easy to lose — `Accept-Language`
    /// reaches `create_message` for EVERY chat, gets handed to
    /// `spawn_mention_reply`, and has to survive all the way into the
    /// prompt. This is the assertion that says it did, without a provider.
    #[test]
    fn a_family_mention_carries_the_askers_language() {
        let prompt = compose_system_prompt(
            "You are a helpful assistant in a family's private chat app.",
            &[MENTION_INSTRUCTION.to_string()],
            Some("ru-RU,ru;q=0.9,en;q=0.8"),
        );
        assert!(
            prompt.ends_with("Answer in Russian."),
            "the language instruction goes LAST, after the English notes: {prompt}"
        );
        assert!(prompt.contains("mentioned with @ai"), "{prompt}");
        assert!(prompt.starts_with("You are a helpful"), "{prompt}");
    }

    /// The private thread has no mention note, and still gets the language.
    #[test]
    fn a_private_question_carries_it_too() {
        let prompt = compose_system_prompt("Be brief.", &[], Some("ja"));
        assert_eq!(prompt, "Be brief.\n\nAnswer in Japanese.");
    }

    /// A device that sent no usable header adds nothing at all, and the
    /// model simply mirrors the language it was written to.
    #[test]
    fn no_header_adds_nothing() {
        assert_eq!(compose_system_prompt("Be brief.", &[], None), "Be brief.");
        assert_eq!(
            compose_system_prompt("Be brief.", &[], Some("*")),
            "Be brief."
        );
    }

    /// An empty configured prompt must not leave a blank line in front of
    /// the notes — a prompt starting with whitespace is a prompt that has
    /// been built by string concatenation and not looked at.
    #[test]
    fn an_empty_configured_prompt_leaves_no_blank_line() {
        let prompt = compose_system_prompt("   ", &["Only this.".to_string()], Some("de"));
        assert_eq!(prompt, "Only this.\n\nAnswer in German.");
    }

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
