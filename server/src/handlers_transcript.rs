//! The text of a recording, on request (docs/protocol.md, "Transcripts on
//! request").
//!
//! `POST /chats/{chat_id}/messages/{message_id}/attachments/{attachment_id}/transcript`
//!
//! A member asks for the text of ONE voice note, audio file or video, and the
//! answer goes back to them in the response. Nothing is transcribed unasked,
//! nothing is added to the message, no frame is sent and nothing is pushed.
//!
//! Two forms, decided by the request's `Content-Type` alone:
//!
//! - **no body** (anything that is not `multipart/form-data`): the server
//!   sends its OWN stored bytes, for `kind=audio` in a type the provider
//!   reads and within `[ai.transcribe] max_bytes`. That answer is KEPT — one
//!   `transcripts` row per attachment — and handed to every later asker the
//!   rule allows, with no second provider call. Concurrent asks share one
//!   call, and the call finishes and is kept even if the asker's connection
//!   closes: it runs on a spawned task, not on the request.
//! - **one multipart part, `audio`**: sound the asking device took out of the
//!   file itself (a video's sound track, an Ogg file re-encoded), AAC in
//!   MPEG-4. Returned and NEVER kept: the server cannot check that uploaded
//!   sound is really this recording's, and a kept answer is handed to other
//!   members — one member could put words into another's video.
//!
//! Who may ask is decided HERE, before any byte leaves, in a fixed order
//! with one code per reason — see [`transcript`].
//!
//! **Never logged**: the text, the language, the sound. The log line says
//! which of five outcomes happened — `stored`, `supplied`, `shared`,
//! `refused`, `failed` — beside the ids, and a provider error goes through
//! `ai::loggable_detail`'s allow-list like every other.

use std::collections::HashMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::{FromRequest, Multipart, Path, Request, State};
use axum::http::StatusCode;
use axum::http::header::CONTENT_TYPE;
use axum::response::{IntoResponse, Response};
use futures_util::FutureExt;
use futures_util::future::{BoxFuture, Shared};
use serde_json::json;
use sqlx::Row;
use tokio::sync::{Mutex, Semaphore};
use tracing::{info, warn};

use crate::ai::{self, Recording, Transcript};
use crate::auth::AuthUser;
use crate::config::ModelRoute;
use crate::error::{ApiError, codes};
use crate::handlers_chat::ensure_chat_access;
use crate::state::AppState;

/// How many transcription calls may be in flight on one server at once.
///
/// Fixed, and small, because each one holds up to 25 MiB of sound in memory
/// twice over (the bytes read, and the multipart body built from them) on a
/// unit capped at 768 MB. A request beyond it waits for a slot — and a
/// request carrying sound reads its body only once it HAS one, so a queue
/// of uploads waits in the socket rather than in this process's memory.
pub const TRANSCRIPTION_SLOTS: usize = 4;

/// The stored types the provider reads, and the file name each is sent
/// under — the OpenAI transcription surface takes a file's format from its
/// EXTENSION, so the name is load-bearing.
///
/// The intersection of two lists: what this server accepts as `kind=audio`
/// (`models::Attachment::ACCEPTED`) and what Azure's transcription contract
/// lists (mp3, mp4, mpeg, mpga, m4a, wav, webm). `audio/ogg` is accepted on
/// upload and is NOT here: the provider refuses Ogg, so a device sends its
/// own sound track for one instead.
pub const STORED_AUDIO: [(&str, &str); 4] = [
    ("audio/mp4", "audio.m4a"),
    ("audio/m4a", "audio.m4a"),
    ("audio/mpeg", "audio.mp3"),
    ("audio/wav", "audio.wav"),
];

/// The one shape a device may SUPPLY: AAC in an MPEG-4 container.
const SUPPLIED_MIME: &str = "audio/mp4";
const SUPPLIED_FILENAME: &str = "audio.m4a";
/// The multipart part that carries it.
const SUPPLIED_PART: &str = "audio";

/// How one provider call ended, as every waiter on it sees it. `Clone`
/// because a shared call hands the same outcome to each of them.
#[derive(Debug, Clone)]
enum Outcome {
    Answered(Transcript),
    /// The provider's own filter said no — terminal.
    Refused,
    /// Anything else — transient, `internal`.
    Failed,
}

/// One call in flight for one attachment's stored bytes, awaitable by every
/// request that arrives while it runs.
type Flight = Shared<BoxFuture<'static, Outcome>>;

/// The server's transcription state: the calls in flight per attachment,
/// and the slots that bound how many run at once.
pub struct Transcriptions {
    /// Attachment id → the call for its stored bytes. An entry exists
    /// exactly while that call runs; the call removes it itself, AFTER its
    /// answer is written, under this same lock — so a request that takes
    /// the lock and finds no entry is guaranteed to find the stored row
    /// instead, if there is one. That is what makes "one call per
    /// attachment" hold without a window between the two.
    flights: Mutex<HashMap<i64, Flight>>,
    slots: Arc<Semaphore>,
}

impl Default for Transcriptions {
    fn default() -> Self {
        Self {
            flights: Mutex::new(HashMap::new()),
            slots: Arc::new(Semaphore::new(TRANSCRIPTION_SLOTS)),
        }
    }
}

/// Everything about the attachment that the checks and the call need, read
/// once.
struct Held {
    sender_id: i64,
    chat_kind: String,
    kind: String,
    mime: String,
    size_bytes: i64,
    duration_ms: Option<i32>,
    storage_key: String,
}

/// What a call is FOR — handed to the spawned task, which outlives the
/// request that started it.
struct Ask {
    route: ModelRoute,
    asker: i64,
    family_id: Option<i64>,
    message_id: i64,
    attachment_id: i64,
    duration_ms: Option<i32>,
    language: Option<String>,
}

/// `POST /chats/{chat_id}/messages/{message_id}/attachments/{attachment_id}/transcript`
///
/// The checks run in this order, each with its own answer, and nothing
/// leaves the server until all of them pass:
///
/// 1. the chat, and the caller in it — `ensure_chat_access`, exactly as
///    every chat endpoint answers (`chat_not_found`, `not_chat_member`,
///    `blocked`);
/// 2. the message in that chat — `message_not_found`;
/// 3. the attachment on that message — `attachment_not_found`;
/// 4. a transcription deployment — `transcripts_unavailable`;
/// 5. the ASKER's consent — `assistant_consent_required`, asked even when an
///    answer is already stored;
/// 6. the rule — `transcript_not_allowed`: the asker's own message anywhere;
///    another member's only in the family chat, only with `ai_transcripts`
///    on, and only when that SENDER has consented too;
/// 7. the recording in this form — `not_transcribable`;
/// 8. the provider — `transcript_refused` for its filter, `internal` for the
///    rest.
pub async fn transcript(
    auth: AuthUser,
    State(state): State<AppState>,
    Path((chat_id, message_id, attachment_id)): Path<(i64, i64, i64)>,
    request: Request,
) -> Result<Response, ApiError> {
    // 1–3: here, and addressed correctly.
    ensure_chat_access(&state, chat_id, auth.user_id).await?;
    let held = load(&state, chat_id, message_id, attachment_id).await?;

    // 4: this server can transcribe at all.
    let Some(route) = state.cfg.ai.transcribe_route() else {
        return Err(ApiError::forbidden(
            codes::TRANSCRIPTS_UNAVAILABLE,
            "this server has no transcription model configured",
        ));
    };

    // 5: NOTHING REACHES THE MODEL WITHOUT THE ASKER'S OWN PERMISSION
    // (protocol.md, "Consenting to the assistant") — the member asking is
    // the member sending the sound. Asked before the stored answer is
    // looked up, so the answer to "may I?" never depends on whether
    // somebody else already paid for this one.
    if !crate::handlers_ai::has_assistant_consent(&state, auth.user_id).await? {
        return Err(ApiError::forbidden(
            codes::ASSISTANT_CONSENT_REQUIRED,
            "you have not agreed that your words may be sent to the assistant",
        ));
    }

    // 6: whose voice it is.
    let family = sqlx::query(
        "SELECT u.family_id, f.language, COALESCE(f.ai_transcripts, false) AS ai_transcripts
         FROM users u LEFT JOIN families f ON f.id = u.family_id
         WHERE u.id = $1",
    )
    .bind(auth.user_id)
    .fetch_one(&state.pool)
    .await?;
    let family_id: Option<i64> = family.get("family_id");
    let family_language: Option<String> = family.get("language");
    let switch_on: bool = family.get("ai_transcripts");
    if !allowed(&state, auth.user_id, &held, switch_on).await? {
        return Err(ApiError::forbidden(
            codes::TRANSCRIPT_NOT_ALLOWED,
            "you may not ask for the text of this recording",
        ));
    }

    // 7, first half: the form is decided by the Content-Type alone, and a
    // body that is not multipart is ignored rather than refused — a client
    // that always sends `{}` is asking for the stored bytes.
    let supplied = request
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .trim_start()
                .to_ascii_lowercase()
                .starts_with("multipart/form-data")
        });
    let max_bytes = state.cfg.ai.transcribe.max_bytes;
    let ask = Ask {
        route,
        asker: auth.user_id,
        family_id,
        message_id,
        attachment_id,
        duration_ms: held.duration_ms,
        language: family_language
            .as_deref()
            .and_then(ai::transcription_language),
    };

    if supplied {
        if held.kind != "audio" && held.kind != "video" {
            return Err(not_transcribable(
                "only a voice note, an audio file or a video has sound to transcribe",
            ));
        }
        // An answer from the server's own bytes outranks any sound a device
        // sends: it is the one answer that is known to be this recording's,
        // and it costs nothing. The supplied body is then never read.
        if let Some(kept) = stored_answer(&state, attachment_id).await? {
            log_outcome(&ask, "shared");
            return Ok(answer(&kept));
        }
        return supplied_sound(&state, ask, request, max_bytes).await;
    }

    // 7, second half: the stored bytes are something the provider reads.
    let filename = STORED_AUDIO
        .iter()
        .find(|(mime, _)| *mime == held.mime)
        .map(|(_, filename)| *filename);
    let Some(filename) = filename.filter(|_| held.kind == "audio") else {
        return Err(not_transcribable(
            "only a voice note or an audio file in a type the provider reads can be sent as \
             stored; send the sound track instead",
        ));
    };
    if held.size_bytes < 0 || held.size_bytes as u64 > max_bytes as u64 {
        return Err(not_transcribable(format!(
            "this recording is over the {max_bytes} bytes one transcript may send; send a \
             smaller sound track instead"
        )));
    }

    stored_bytes(&state, ask, held, filename).await
}

/// Steps 2 and 3, with the columns everything after them needs.
async fn load(
    state: &AppState,
    chat_id: i64,
    message_id: i64,
    attachment_id: i64,
) -> Result<Held, ApiError> {
    let message = sqlx::query(
        "SELECT m.sender_id, c.kind
         FROM messages m JOIN chats c ON c.id = m.chat_id
         WHERE m.id = $1 AND m.chat_id = $2",
    )
    .bind(message_id)
    .bind(chat_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some(message) = message else {
        return Err(ApiError::not_found(
            codes::MESSAGE_NOT_FOUND,
            "no such message in this chat",
        ));
    };
    let attachment = sqlx::query(
        "SELECT kind, mime, size_bytes, duration_ms, storage_key
         FROM attachments
         WHERE id = $1 AND message_id = $2",
    )
    .bind(attachment_id)
    .bind(message_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some(attachment) = attachment else {
        return Err(ApiError::not_found(
            codes::ATTACHMENT_NOT_FOUND,
            "no such attachment on this message",
        ));
    };
    Ok(Held {
        sender_id: message.get("sender_id"),
        chat_kind: message.get("kind"),
        kind: attachment.get("kind"),
        mime: attachment.get("mime"),
        size_bytes: attachment.get("size_bytes"),
        duration_ms: attachment.get("duration_ms"),
        // A location has no file; it never reaches a read, because its kind
        // is refused first.
        storage_key: attachment
            .get::<Option<String>, _>("storage_key")
            .unwrap_or_default(),
    })
}

/// THE RULE (protocol.md, "Transcripts on request"), in one place:
///
/// - the asker's OWN message — any chat they are in, direct chats included;
/// - another member's — only in the family chat (its threads are the same
///   chat), only while the family's `ai_transcripts` is on, and only when
///   that SENDER has agreed to the assistant themselves: the history a
///   mention carries is filtered by each sender's consent for exactly this
///   reason, and a voice is somebody's words;
/// - never another member's in a direct chat, at any setting;
/// - never the assistant's: its account has no consent to give (the
///   history filter's own exception says as much), so the third key above
///   refuses it in the family chat, and in an `ai` chat it is the other
///   party and the second refuses it. It sends no sound in any case.
async fn allowed(
    state: &AppState,
    asker: i64,
    held: &Held,
    switch_on: bool,
) -> Result<bool, ApiError> {
    if held.sender_id == asker {
        return Ok(true);
    }
    if held.chat_kind != "family" || !switch_on {
        return Ok(false);
    }
    Ok(crate::handlers_ai::has_assistant_consent(state, held.sender_id).await?)
}

fn not_transcribable(message: impl Into<String>) -> ApiError {
    ApiError::bad_request(codes::NOT_TRANSCRIBABLE, message)
}

/// The kept answer for this attachment, if one was ever made from its
/// stored bytes.
async fn stored_answer(
    state: &AppState,
    attachment_id: i64,
) -> Result<Option<Transcript>, ApiError> {
    let row = sqlx::query("SELECT text, language FROM transcripts WHERE attachment_id = $1")
        .bind(attachment_id)
        .fetch_optional(&state.pool)
        .await?;
    Ok(row.map(|row| Transcript {
        text: row.get("text"),
        language: row.get("language"),
    }))
}

/// `200 {"transcript": {"text": "…", "language": "ru"}}` — `language` only
/// when the provider named one.
fn answer(transcript: &Transcript) -> Response {
    let mut body = json!({"text": transcript.text});
    if let Some(language) = &transcript.language {
        body["language"] = json!(language);
    }
    (StatusCode::OK, Json(json!({"transcript": body}))).into_response()
}

/// The log line every request that got this far ends with: ids and ONE
/// word. Never the text, never the language.
fn log_outcome(ask: &Ask, outcome: &'static str) {
    info!(
        message_id = ask.message_id,
        attachment_id = ask.attachment_id,
        outcome,
        "transcript"
    );
}

/// An [`Outcome`] as the HTTP answer.
fn respond(outcome: Outcome) -> Result<Response, ApiError> {
    match outcome {
        Outcome::Answered(transcript) => Ok(answer(&transcript)),
        Outcome::Refused => Err(ApiError::bad_request(
            codes::TRANSCRIPT_REFUSED,
            "the assistant's provider refused to transcribe this",
        )),
        Outcome::Failed => Err(ApiError::Internal(anyhow::anyhow!(
            "the transcription could not be made"
        ))),
    }
}

/// The no-body form: the server's own stored bytes, one call per
/// attachment, finished and kept whatever happens to the request.
async fn stored_bytes(
    state: &AppState,
    ask: Ask,
    held: Held,
    filename: &'static str,
) -> Result<Response, ApiError> {
    let attachment_id = ask.attachment_id;
    let flight = {
        let mut flights = state.transcriptions.flights.lock().await;
        if let Some(flight) = flights.get(&attachment_id) {
            // Somebody is already asking: wait on their call. This request
            // costs nothing and records nothing.
            let flight = flight.clone();
            drop(flights);
            let outcome = flight.await;
            if matches!(outcome, Outcome::Answered(_)) {
                log_outcome(&ask, "shared");
            }
            return respond(outcome);
        } else if let Some(kept) = stored_answer(state, attachment_id).await? {
            // Read UNDER the lock: a call that finished a moment ago wrote
            // its row before it removed its entry, so it is here.
            drop(flights);
            log_outcome(&ask, "shared");
            return Ok(answer(&kept));
        } else {
            let flight = start_call(state.clone(), ask, held, filename);
            flights.insert(attachment_id, flight.clone());
            flight
        }
    };
    respond(flight.await)
}

/// Start the call for one attachment's stored bytes on its OWN task, so that
/// it runs to the end — answer written, usage recorded — even if every
/// request waiting on it goes away. Unlike the board's backdrop, whose
/// picture nobody else could use, this answer is exactly what the next
/// asker would pay for again.
fn start_call(state: AppState, ask: Ask, held: Held, filename: &'static str) -> Flight {
    let task = tokio::spawn(async move {
        let outcome = call_with_stored_bytes(&state, &ask, &held, filename).await;
        // AFTER the answer is written: see `Transcriptions::flights`.
        state
            .transcriptions
            .flights
            .lock()
            .await
            .remove(&ask.attachment_id);
        outcome
    });
    async move { task.await.unwrap_or(Outcome::Failed) }
        .boxed()
        .shared()
}

async fn call_with_stored_bytes(
    state: &AppState,
    ask: &Ask,
    held: &Held,
    filename: &'static str,
) -> Outcome {
    let Ok(_slot) = state.transcriptions.slots.clone().acquire_owned().await else {
        return Outcome::Failed;
    };
    let mime = STORED_AUDIO
        .iter()
        .find(|(mime, _)| *mime == held.mime)
        .map(|(mime, _)| *mime)
        .unwrap_or(SUPPLIED_MIME);
    let path = state.storage.blob_path(&held.storage_key);
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(error) => {
            warn!(attachment_id = ask.attachment_id, %error, "could not read a recording to transcribe");
            log_outcome(ask, "failed");
            return Outcome::Failed;
        }
    };
    let outcome = call(
        state,
        ask,
        Recording {
            bytes,
            mime,
            filename,
        },
    )
    .await;
    if let Outcome::Answered(transcript) = &outcome {
        // Kept for the next asker. `ON CONFLICT DO NOTHING` because a row
        // can only already be here if a second call was somehow made; the
        // first answer stands. A failure to keep it is not the asker's
        // problem: they get their answer, and the next asker pays again.
        if let Err(error) = sqlx::query(
            "INSERT INTO transcripts (attachment_id, text, language) VALUES ($1, $2, $3)
             ON CONFLICT (attachment_id) DO NOTHING",
        )
        .bind(ask.attachment_id)
        .bind(&transcript.text)
        .bind(&transcript.language)
        .execute(&state.pool)
        .await
        {
            warn!(attachment_id = ask.attachment_id, %error, "could not keep a transcript");
        }
        log_outcome(ask, "stored");
    }
    outcome
}

/// The multipart form: sound the asking device took out of the file itself.
/// Read only once a slot is free, bounded as it arrives, checked to be
/// MPEG-4, sent, and returned — never kept.
async fn supplied_sound(
    state: &AppState,
    ask: Ask,
    request: Request,
    max_bytes: usize,
) -> Result<Response, ApiError> {
    let Ok(_slot) = state.transcriptions.slots.clone().acquire_owned().await else {
        return Err(ApiError::Internal(anyhow::anyhow!(
            "the transcription slots are closed"
        )));
    };
    let bytes = read_supplied_part(state, request, max_bytes).await?;
    let outcome = call(
        state,
        &ask,
        Recording {
            bytes,
            mime: SUPPLIED_MIME,
            filename: SUPPLIED_FILENAME,
        },
    )
    .await;
    if matches!(outcome, Outcome::Answered(_)) {
        log_outcome(&ask, "supplied");
    }
    respond(outcome)
}

/// The one part named `audio`, at most `max_bytes`, and an MPEG-4 file —
/// or the reason it is not.
async fn read_supplied_part(
    state: &AppState,
    request: Request,
    max_bytes: usize,
) -> Result<Vec<u8>, ApiError> {
    let mut multipart = Multipart::from_request(request, state)
        .await
        .map_err(|rejection| ApiError::validation(format!("not a multipart body: {rejection}")))?;
    let mut found: Option<Vec<u8>> = None;
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(error) => return Err(multipart_error(error, max_bytes)),
        };
        let mut field = field;
        let wanted = field.name() == Some(SUPPLIED_PART) && found.is_none();
        let mut bytes: Vec<u8> = Vec::new();
        loop {
            match field.chunk().await {
                Ok(Some(chunk)) => {
                    if !wanted {
                        continue;
                    }
                    if bytes.len() + chunk.len() > max_bytes {
                        return Err(not_transcribable(format!(
                            "the sound is over the {max_bytes} bytes one transcript may send"
                        )));
                    }
                    bytes.extend_from_slice(&chunk);
                }
                Ok(None) => break,
                Err(error) => return Err(multipart_error(error, max_bytes)),
            }
        }
        if wanted {
            found = Some(bytes);
        }
    }
    let Some(bytes) = found else {
        return Err(not_transcribable(
            "send the sound as one multipart part named \"audio\"",
        ));
    };
    if bytes.is_empty() {
        return Err(not_transcribable("the \"audio\" part is empty"));
    }
    // The same check an upload gets: the bytes decide, not the label. An
    // MPEG-4 file is all a device may supply.
    if !crate::handlers_attachment::matches_magic(SUPPLIED_MIME, &bytes) {
        return Err(not_transcribable(
            "the \"audio\" part must be AAC in an MPEG-4 container (.m4a)",
        ));
    }
    Ok(bytes)
}

/// A body the route's limit cut off is a recording too large to send —
/// `not_transcribable`, the answer a client can act on — and any other
/// multipart failure is a body that could not be read.
fn multipart_error(error: axum::extract::multipart::MultipartError, max_bytes: usize) -> ApiError {
    if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
        not_transcribable(format!(
            "the sound is over the {max_bytes} bytes one transcript may send"
        ))
    } else {
        ApiError::validation(format!("the multipart body could not be read: {error}"))
    }
}

/// The provider call itself, and what it cost. Shared by both forms; what
/// differs is only what happens to the answer afterwards.
async fn call(state: &AppState, ask: &Ask, recording: Recording) -> Outcome {
    match ai::transcribe(&state.http, &ask.route, recording, ask.language.as_deref()).await {
        Ok(transcript) => {
            record_usage(state, ask).await;
            Outcome::Answered(transcript)
        }
        Err(error) => {
            let refused = ai::is_refusal(&error);
            // The chain is the URL and the allow-listed fields of the
            // provider's error — never the sound and never any text.
            warn!(
                attachment_id = ask.attachment_id,
                refused,
                error = %format!("{error:#}"),
                "the transcription deployment could not answer"
            );
            log_outcome(ask, if refused { "refused" } else { "failed" });
            if refused {
                Outcome::Refused
            } else {
                Outcome::Failed
            }
        }
    }
}

/// One `transcript` and the recording's length against the member who
/// asked (protocol.md, "Family statistics"). Best effort, like every usage
/// row: an answer the member already has must not fail on a counter.
async fn record_usage(state: &AppState, ask: &Ask) {
    let Some(family_id) = ask.family_id else {
        return;
    };
    if let Err(error) = sqlx::query(
        "INSERT INTO ai_usage (user_id, family_id, message_id, transcripts, audio_ms)
         VALUES ($1, $2, $3, 1, $4)",
    )
    .bind(ask.asker)
    .bind(family_id)
    .bind(ask.message_id)
    .bind(i64::from(ask.duration_ms.unwrap_or(0).max(0)))
    .execute(&state.pool)
    .await
    {
        warn!(%error, "could not record transcription usage");
    }
}
