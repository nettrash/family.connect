//! The REST half of the protocol, as a browser speaks it.
//!
//! `{base}` for a browser is ITS OWN ORIGIN (docs/protocol.md, "A browser is
//! a client too"): the page was served by the same nginx that proxies
//! `/api/v1`, so there is no server URL to enter and no CORS anywhere. Every
//! path here is therefore relative — `/api/v1/...` — and the browser
//! resolves it against wherever it is running, which is the one thing a
//! browser knows that an app has to be told.
//!
//! Only the endpoints this client actually uses live here. The rest of the
//! protocol is not stubbed: an unused wrapper is a claim that something
//! works, and nothing here has been tested against a server it does not
//! call.

use futures::future::{select, Either};
use gloo_net::http::{Request, RequestBuilder, Response};
use gloo_timers::future::TimeoutFuture;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use web_sys::AbortController;

use crate::model::{Chat, ChatListItem, Me, Mention, Message, Poll, Reaction, Roster};
use crate::store::Outgoing;

/// Everything under one prefix, so a change of base is one line.
const API: &str = "/api/v1";

/// What went wrong, in the shape a view can act on.
///
/// `Unauthorized` is called out because the protocol gives it a meaning no
/// other status has: "the session is gone — wipe local state and return to
/// login" (docs/protocol.md, "Authentication"). Every other failure is a
/// message to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiError {
    /// 401: the session is gone.
    Unauthorized,
    /// The server answered, with a protocol error code and message.
    Server { code: String, message: String },
    /// 429: the server, or the nginx in front of it, asked for a slower
    /// pace — often in an HTML body, so the STATUS is what says so
    /// (docs/protocol.md, "Error shape"). Never a refusal. `retry_after_secs`
    /// is the header's delta-seconds, when it sent one.
    Throttled { retry_after_secs: Option<u32> },
    /// It did not answer, or the answer was not what this client can read.
    Network(String),
}

impl ApiError {
    /// What to put in front of a person. The protocol's `message` is
    /// English and meant for developers, so the caller decides whether to
    /// use it; this is the fallback when there is nothing better.
    pub fn detail(&self) -> String {
        match self {
            ApiError::Unauthorized => "Your session has expired.".to_string(),
            ApiError::Server { message, .. } => message.clone(),
            ApiError::Throttled { .. } => "The server is busy. Try again in a moment.".to_string(),
            ApiError::Network(detail) => detail.clone(),
        }
    }

    pub fn code(&self) -> Option<&str> {
        match self {
            ApiError::Server { code, .. } => Some(code),
            _ => None,
        }
    }
}

/// The protocol's error body: `{"error": {"code": "...", "message": "..."}}`.
#[derive(Debug, Deserialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Debug, Deserialize)]
struct ErrorBody {
    code: String,
    message: String,
}

#[derive(Debug, Serialize)]
struct LoginRequest<'a> {
    username: &'a str,
    password: &'a str,
}

#[derive(Debug, Deserialize)]
pub struct AuthResponse {
    pub token: String,
}

#[derive(Debug, Deserialize)]
struct ChatsResponse {
    chats: Vec<ChatListItem>,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    chat: Chat,
}

#[derive(Debug, Deserialize)]
struct MessagesResponse {
    messages: Vec<Message>,
}

#[derive(Debug, Deserialize)]
struct MessageResponse {
    message: Message,
}

/// One message's whole reaction state — the answer to a reaction request
/// and one row of the reaction catch-up.
#[derive(Debug, Clone, Deserialize)]
pub struct ReactionState {
    pub message_id: i64,
    pub reaction_seq: i64,
    pub reactions: Vec<Reaction>,
}

#[derive(Debug, Deserialize)]
struct ReactionsResponse {
    message_reactions: Vec<ReactionState>,
}

/// One poll's whole state — the answer to a vote and one row of the poll
/// catch-up.
#[derive(Debug, Clone, Deserialize)]
pub struct PollState {
    pub message_id: i64,
    pub poll: Poll,
}

#[derive(Debug, Deserialize)]
struct PollsResponse {
    polls: Vec<PollState>,
}

#[derive(Debug, Serialize)]
struct PollRequest<'a> {
    options: &'a [String],
}

/// A send. Every optional part is LEFT OUT when there is none, rather than
/// sent as null or as an empty list — the protocol's absent-is-not-empty
/// convention, and the server's own rules (a `mentions` outside the family
/// chat is `validation`).
#[derive(Debug, Serialize)]
struct SendRequest<'a> {
    client_msg_id: &'a str,
    body: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reply_to_message_id: Option<i64>,
    #[serde(skip_serializing_if = "<[Mention]>::is_empty")]
    mentions: &'a [Mention],
    #[serde(skip_serializing_if = "Option::is_none")]
    poll: Option<PollRequest<'a>>,
}

#[derive(Debug, Serialize)]
struct EditRequest<'a> {
    body: &'a str,
}

#[derive(Debug, Serialize)]
struct ReactRequest<'a> {
    emoji: &'a str,
}

#[derive(Debug, Serialize)]
struct VoteRequest {
    option_id: i64,
}

#[derive(Debug, Serialize)]
struct ReadRequest {
    last_read_message_id: i64,
}

#[derive(Debug, Serialize)]
struct DirectRequest {
    user_id: i64,
}

#[derive(Debug, Serialize)]
struct ReportRequest<'a> {
    reported_user_id: i64,
    reason: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    message_id: Option<i64>,
}

/// Turn a response into either its JSON body or the protocol's error.
///
/// A 401 is separated from every other failure HERE rather than at each call
/// site, because there is exactly one right response to it and thirty places
/// that would otherwise have to remember what it is.
async fn read<T: DeserializeOwned>(response: Response) -> Result<T, ApiError> {
    check(&response).await?;
    response
        .json::<T>()
        .await
        .map_err(|error| ApiError::Network(error.to_string()))
}

/// The failure half of `read`, for answers with no body worth reading.
async fn check(response: &Response) -> Result<(), ApiError> {
    if response.status() == 401 {
        return Err(ApiError::Unauthorized);
    }
    if response.ok() {
        return Ok(());
    }
    let status = response.status();
    if status == 429 {
        return Err(ApiError::Throttled {
            retry_after_secs: response
                .headers()
                .get("Retry-After")
                .and_then(|value| retry_after_secs(&value)),
        });
    }
    match response.json::<ErrorEnvelope>().await {
        Ok(envelope) => Err(ApiError::Server {
            code: envelope.error.code,
            message: envelope.error.message,
        }),
        // A body that is not the protocol's shape is still a failure, and
        // saying which status it was beats saying nothing: nginx answers
        // its own rate limit and a dead upstream with HTML.
        Err(_) => Err(ApiError::Network(format!("The server answered {status}."))),
    }
}

/// `Retry-After` in its delta-seconds form — a whole number of seconds, the
/// one this server's proxy sends. The HTTP-date form, or anything else, is
/// no answer.
pub fn retry_after_secs(header: &str) -> Option<u32> {
    header.trim().parse::<u32>().ok()
}

fn network(error: gloo_net::Error) -> ApiError {
    ApiError::Network(error.to_string())
}

fn bearer(builder: RequestBuilder, token: &str) -> RequestBuilder {
    builder.header("Authorization", &format!("Bearer {token}"))
}

async fn get<T: DeserializeOwned>(token: &str, path: &str) -> Result<T, ApiError> {
    let request = bearer(Request::get(&format!("{API}{path}")), token);
    read(request.send().await.map_err(network)?).await
}

async fn with_body<T: DeserializeOwned, B: Serialize>(
    builder: RequestBuilder,
    token: &str,
    body: &B,
) -> Result<T, ApiError> {
    let request = bearer(builder, token).json(body).map_err(network)?;
    read(request.send().await.map_err(network)?).await
}

async fn empty<B: Serialize>(
    builder: RequestBuilder,
    token: &str,
    body: Option<&B>,
) -> Result<(), ApiError> {
    let builder = bearer(builder, token);
    let response = match body {
        Some(body) => builder.json(body).map_err(network)?.send().await,
        None => builder.send().await,
    }
    .map_err(network)?;
    check(&response).await
}

fn path(path: &str) -> String {
    format!("{API}{path}")
}

/// `POST /auth/login` — the one call made without a token.
pub async fn login(username: &str, password: &str) -> Result<AuthResponse, ApiError> {
    let request = Request::post(&path("/auth/login"))
        .json(&LoginRequest { username, password })
        .map_err(network)?;
    read(request.send().await.map_err(network)?).await
}

/// `POST /auth/logout` — revokes this session and closes its sockets.
///
/// Its failure is deliberately not reported: the client is signing out
/// either way, and a person who has already been told "signed out" cannot
/// act on "but the server did not hear". The token is dropped locally
/// regardless, which is what makes the tab safe on a shared machine.
pub async fn logout(token: &str) {
    let _ = bearer(Request::post(&path("/auth/logout")), token)
        .send()
        .await;
}

/// `GET /me` — who this is, and what the server allows.
pub async fn me(token: &str) -> Result<Me, ApiError> {
    get(token, "/me").await
}

/// `GET /families/mine` — the roster, the assistant, the block list.
pub async fn family(token: &str) -> Result<Roster, ApiError> {
    get(token, "/families/mine").await
}

/// `GET /chats` — the family chat always, direct chats once they exist.
pub async fn chats(token: &str) -> Result<Vec<ChatListItem>, ApiError> {
    let response: ChatsResponse = get(token, "/chats").await?;
    Ok(response.chats)
}

/// `POST /chats/direct` — get-or-create the one-to-one chat with a member.
pub async fn direct_chat(token: &str, user_id: i64) -> Result<Chat, ApiError> {
    let response: ChatResponse = with_body(
        Request::post(&path("/chats/direct")),
        token,
        &DirectRequest { user_id },
    )
    .await?;
    Ok(response.chat)
}

/// `GET /chats/{id}/messages` — the newest page, or the page before
/// `before_id` (docs/protocol.md: strictly older, newest-first).
pub async fn messages(
    token: &str,
    chat_id: i64,
    before_id: Option<i64>,
    limit: u32,
) -> Result<Vec<Message>, ApiError> {
    let mut url = format!("/chats/{chat_id}/messages?limit={limit}");
    if let Some(before_id) = before_id {
        url.push_str(&format!("&before_id={before_id}"));
    }
    let response: MessagesResponse = get(token, &url).await?;
    Ok(response.messages)
}

/// `GET /chats/{id}/messages?after_id=` — the reconnect catch-up, oldest
/// first, looped by the caller until a short page.
pub async fn messages_after(
    token: &str,
    chat_id: i64,
    after_id: i64,
    limit: u32,
) -> Result<Vec<Message>, ApiError> {
    let url = format!("/chats/{chat_id}/messages?after_id={after_id}&limit={limit}");
    let response: MessagesResponse = get(token, &url).await?;
    Ok(response.messages)
}

/// `GET /chats/{id}/messages/{mid}/thread` — the chain a message belongs to,
/// root first. A plain read: no cursor moves.
pub async fn thread(
    token: &str,
    chat_id: i64,
    message_id: i64,
    after_id: Option<i64>,
    limit: u32,
) -> Result<Vec<Message>, ApiError> {
    let mut url = format!("/chats/{chat_id}/messages/{message_id}/thread?limit={limit}");
    if let Some(after_id) = after_id {
        url.push_str(&format!("&after_id={after_id}"));
    }
    let response: MessagesResponse = get(token, &url).await?;
    Ok(response.messages)
}

/// `GET /chats/{id}/edits?after_seq=` — the edit catch-up.
pub async fn edits(
    token: &str,
    chat_id: i64,
    after_seq: i64,
    limit: u32,
) -> Result<Vec<Message>, ApiError> {
    let url = format!("/chats/{chat_id}/edits?after_seq={after_seq}&limit={limit}");
    let response: MessagesResponse = get(token, &url).await?;
    Ok(response.messages)
}

/// `GET /chats/{id}/reactions?after_seq=` — the reaction catch-up.
pub async fn reactions(
    token: &str,
    chat_id: i64,
    after_seq: i64,
    limit: u32,
) -> Result<Vec<ReactionState>, ApiError> {
    let url = format!("/chats/{chat_id}/reactions?after_seq={after_seq}&limit={limit}");
    let response: ReactionsResponse = get(token, &url).await?;
    Ok(response.message_reactions)
}

/// `GET /chats/{id}/polls?after_seq=` — the poll catch-up.
pub async fn polls(
    token: &str,
    chat_id: i64,
    after_seq: i64,
    limit: u32,
) -> Result<Vec<PollState>, ApiError> {
    let url = format!("/chats/{chat_id}/polls?after_seq={after_seq}&limit={limit}");
    let response: PollsResponse = get(token, &url).await?;
    Ok(response.polls)
}

/// `GET /chats/{id}/polls/open` — what is still to decide. Not a cursor.
pub async fn open_polls(token: &str, chat_id: i64) -> Result<Vec<Message>, ApiError> {
    let response: MessagesResponse = get(token, &format!("/chats/{chat_id}/polls/open")).await?;
    Ok(response.messages)
}

/// How long one send may take before its outcome counts as UNKNOWN: the
/// protocol's 10 s ("Sending on an unreliable network"). A browser's fetch
/// has no deadline of its own and will wait on a dead connection for as
/// long as the operating system does, which is far longer than a person.
pub const SEND_DEADLINE_MS: u32 = 10_000;

/// `POST /chats/{id}/messages` — how a browser sends every message.
///
/// A retry carries the SAME `client_msg_id` — and the same quote, mentions
/// and poll, because they ride on the outbox row — which is what makes it
/// idempotent rather than a duplicate: the server answers a repeat with the
/// message it already has (docs/protocol.md). Past the deadline the request
/// is ABORTED, not merely abandoned, so a late answer cannot arrive for a
/// send that has already been counted as unknown.
pub async fn send_message(token: &str, row: &Outgoing) -> Result<Message, ApiError> {
    let controller = AbortController::new()
        .map_err(|_| ApiError::Network("This browser cannot time a request out.".into()))?;
    let request = bearer(
        Request::post(&path(&format!("/chats/{}/messages", row.chat_id))),
        token,
    )
    .abort_signal(Some(&controller.signal()))
    .json(&SendRequest {
        client_msg_id: &row.client_msg_id,
        body: &row.body,
        reply_to_message_id: row.reply_to_message_id,
        mentions: &row.mentions,
        poll: row.poll.as_deref().map(|options| PollRequest { options }),
    })
    .map_err(network)?;
    let attempt = async {
        let response: MessageResponse = read(request.send().await.map_err(network)?).await?;
        Ok(response.message)
    };
    let deadline = TimeoutFuture::new(SEND_DEADLINE_MS);
    match select(Box::pin(attempt), deadline).await {
        Either::Left((answer, _)) => answer,
        Either::Right(_) => {
            controller.abort();
            Err(ApiError::Network(
                "The server did not answer in time.".into(),
            ))
        }
    }
}

/// `PATCH /chats/{id}/messages/{mid}` — author only; the body alone changes.
pub async fn edit_message(
    token: &str,
    chat_id: i64,
    message_id: i64,
    body: &str,
) -> Result<Message, ApiError> {
    let url = path(&format!("/chats/{chat_id}/messages/{message_id}"));
    let response: MessageResponse =
        with_body(Request::patch(&url), token, &EditRequest { body }).await?;
    Ok(response.message)
}

/// `PUT …/reaction` — set or replace the caller's one reaction.
pub async fn react(
    token: &str,
    chat_id: i64,
    message_id: i64,
    emoji: &str,
) -> Result<ReactionState, ApiError> {
    let url = path(&format!("/chats/{chat_id}/messages/{message_id}/reaction"));
    with_body(Request::put(&url), token, &ReactRequest { emoji }).await
}

/// `DELETE …/reaction` — take the caller's reaction back.
pub async fn unreact(
    token: &str,
    chat_id: i64,
    message_id: i64,
) -> Result<ReactionState, ApiError> {
    let url = path(&format!("/chats/{chat_id}/messages/{message_id}/reaction"));
    read(
        bearer(Request::delete(&url), token)
            .send()
            .await
            .map_err(network)?,
    )
    .await
}

/// `PUT …/vote` — choose an option.
pub async fn vote(
    token: &str,
    chat_id: i64,
    message_id: i64,
    option_id: i64,
) -> Result<PollState, ApiError> {
    let url = path(&format!("/chats/{chat_id}/messages/{message_id}/vote"));
    with_body(Request::put(&url), token, &VoteRequest { option_id }).await
}

/// `DELETE …/vote` — retract the caller's choice.
pub async fn unvote(token: &str, chat_id: i64, message_id: i64) -> Result<PollState, ApiError> {
    let url = path(&format!("/chats/{chat_id}/messages/{message_id}/vote"));
    read(
        bearer(Request::delete(&url), token)
            .send()
            .await
            .map_err(network)?,
    )
    .await
}

/// `POST …/poll/close` — the author's, and one-way.
pub async fn close_poll(token: &str, chat_id: i64, message_id: i64) -> Result<PollState, ApiError> {
    let url = path(&format!(
        "/chats/{chat_id}/messages/{message_id}/poll/close"
    ));
    read(
        bearer(Request::post(&url), token)
            .send()
            .await
            .map_err(network)?,
    )
    .await
}

/// `POST /chats/{id}/read` — monotonic on the server, so a stale report is
/// harmless rather than a marker that walks backwards.
pub async fn post_read(
    token: &str,
    chat_id: i64,
    last_read_message_id: i64,
) -> Result<(), ApiError> {
    let url = path(&format!("/chats/{chat_id}/read"));
    empty(
        Request::post(&url),
        token,
        Some(&ReadRequest {
            last_read_message_id,
        }),
    )
    .await
}

/// `POST /families/reports` — one person, or one message of theirs.
pub async fn report(
    token: &str,
    reported_user_id: i64,
    reason: &str,
    message_id: Option<i64>,
) -> Result<(), ApiError> {
    let body = ReportRequest {
        reported_user_id,
        reason,
        message_id,
    };
    empty(
        Request::post(&path("/families/reports")),
        token,
        Some(&body),
    )
    .await
}

/// `PUT` / `DELETE /families/members/{id}/block`.
pub async fn set_blocked(token: &str, user_id: i64, blocked: bool) -> Result<(), ApiError> {
    let url = path(&format!("/families/members/{user_id}/block"));
    if blocked {
        empty(Request::put(&url), token, Some(&serde_json::json!({}))).await
    } else {
        empty::<()>(Request::delete(&url), token, None).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    fn row() -> Outgoing {
        Outgoing {
            chat_id: 42,
            client_msg_id: "8f14e45f".into(),
            body: "Dinner at 7?".into(),
            reply_to_message_id: None,
            mentions: Vec::new(),
            poll: None,
            attempts: 0,
            failed: None,
        }
    }

    fn encode(row: &Outgoing) -> serde_json::Value {
        serde_json::to_value(SendRequest {
            client_msg_id: &row.client_msg_id,
            body: &row.body,
            reply_to_message_id: row.reply_to_message_id,
            mentions: &row.mentions,
            poll: row.poll.as_deref().map(|options| PollRequest { options }),
        })
        .expect("encodes")
    }

    /// Delta-seconds, and nothing else: an HTTP date, a negative or a word
    /// is no answer, and the caller falls back to its own backoff.
    #[wasm_bindgen_test]
    fn only_delta_seconds_are_a_retry_after() {
        assert_eq!(retry_after_secs("12"), Some(12));
        assert_eq!(retry_after_secs(" 7 "), Some(7));
        assert_eq!(retry_after_secs("0"), Some(0));
        assert_eq!(retry_after_secs("-3"), None);
        assert_eq!(retry_after_secs("1.5"), None);
        assert_eq!(retry_after_secs("Wed, 21 Oct 2026 07:28:00 GMT"), None);
        assert_eq!(retry_after_secs(""), None);
    }

    /// A plain send carries nothing it does not have: no null quote, no
    /// empty mention list — the server refuses `mentions` outside the
    /// family chat, so an empty one would be a refusal waiting to happen.
    #[wasm_bindgen_test]
    fn a_plain_send_leaves_every_optional_part_out() {
        assert_eq!(
            encode(&row()),
            serde_json::json!({"client_msg_id": "8f14e45f", "body": "Dinner at 7?"})
        );
    }

    #[wasm_bindgen_test]
    fn a_reply_with_a_mention_and_a_poll_are_the_protocols_shapes() {
        let mut reply = row();
        reply.reply_to_message_id = Some(1337);
        reply.mentions = vec![Mention {
            user_id: 9,
            name: "Anna".into(),
        }];
        assert_eq!(
            encode(&reply),
            serde_json::json!({"client_msg_id": "8f14e45f", "body": "Dinner at 7?",
                               "reply_to_message_id": 1337,
                               "mentions": [{"user_id": 9, "name": "Anna"}]})
        );

        let mut poll = row();
        poll.body = "Pizza or pasta?".into();
        poll.poll = Some(vec!["Pizza".into(), "Pasta".into()]);
        assert_eq!(
            encode(&poll),
            serde_json::json!({"client_msg_id": "8f14e45f", "body": "Pizza or pasta?",
                               "poll": {"options": ["Pizza", "Pasta"]}})
        );
    }

    #[wasm_bindgen_test]
    fn the_catch_up_rows_read_the_protocols_shapes() {
        let reactions: ReactionsResponse = serde_json::from_str(
            r#"{"message_reactions": [{"message_id": 1338, "reaction_seq": 124,
                 "reactions": [{"user_id": 9, "emoji": "❤️"}]}]}"#,
        )
        .expect("reads");
        assert_eq!(reactions.message_reactions[0].reaction_seq, 124);
        let polls: PollsResponse = serde_json::from_str(
            r#"{"polls": [{"message_id": 1340, "poll": {"poll_seq": 89, "closed": false,
                 "options": [{"id": 5, "text": "Pizza", "votes": [7, 9]}]}}]}"#,
        )
        .expect("reads");
        assert_eq!(polls.polls[0].poll.options[0].votes, vec![7, 9]);
    }

    #[wasm_bindgen_test]
    fn a_report_names_the_message_only_when_there_is_one() {
        let person = serde_json::to_value(ReportRequest {
            reported_user_id: 9,
            reason: "harassment",
            message_id: None,
        })
        .expect("encodes");
        assert_eq!(
            person,
            serde_json::json!({"reported_user_id": 9, "reason": "harassment"})
        );
    }
}
