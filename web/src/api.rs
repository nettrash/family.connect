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
use wasm_bindgen::JsCast;
use web_sys::{AbortController, Blob, RequestCache};

use crate::model::{
    Attachment, Chat, ChatListItem, Me, Mention, Message, Note, Poll, Reaction, Roster,
};
use crate::staged::OutgoingItem;
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

#[derive(Debug, Deserialize)]
struct AttachmentResponse {
    attachment: Attachment,
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
    /// The attachments, in the sender's order — the array spelling, which
    /// carries one as well as ten (the legacy `attachment_id` is never
    /// sent).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    attachment_ids: Vec<i64>,
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

/// `GET /families/mine/board`: the whole wall, and the high-water mark read
/// before it (docs/protocol.md, "Board").
#[derive(Debug, Clone, Deserialize)]
pub struct BoardRead {
    pub notes: Vec<Note>,
    pub max_board_seq: i64,
}

#[derive(Debug, Deserialize)]
struct NotesResponse {
    notes: Vec<Note>,
}

#[derive(Debug, Deserialize)]
struct NoteResponse {
    note: Note,
}

/// A note to pin (docs/protocol.md, "Board"). Size and face always go — a
/// new note has a chosen one of each — and every part only some kinds have
/// is LEFT OUT when there is none: the server refuses a `starts_at` on
/// anything but an event and an `attachment_id` on a text note.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct NewNote {
    pub text: String,
    pub color: String,
    pub size: String,
    pub font: String,
    pub x: f64,
    pub y: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachment_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub starts_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ends_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub place: Option<String>,
}

/// A change to a note. Only what changed is sent, and WHICH fields are sent
/// is what the server checks permission against: a move is `x` and `y` and
/// nothing else, or a member's drag of somebody else's note comes back
/// `not_note_author`.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct NotePatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub starts_at: Option<String>,
    /// A DOUBLE option: absent leaves the end alone, `Some(None)` — sent as
    /// null — clears it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ends_at: Option<Option<String>>,
    /// Sent empty to clear it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub place: Option<String>,
}

impl NotePatch {
    /// Nothing to send — so nothing is sent.
    pub fn is_empty(&self) -> bool {
        *self == NotePatch::default()
    }

    /// Where a note was dropped, and nothing else.
    pub fn moved_to(x: f64, y: f64) -> Self {
        NotePatch {
            x: Some(x),
            y: Some(y),
            ..NotePatch::default()
        }
    }
}

#[derive(Debug, Serialize)]
struct RsvpRequest<'a> {
    answer: &'a str,
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
    let controller = controller()?;
    let request = bearer(
        Request::post(&path(&format!("/chats/{}/messages", row.chat_id))),
        token,
    )
    .abort_signal(Some(&controller.signal()))
    .json(&send_request(row))
    .map_err(network)?;
    let attempt = async {
        let response: MessageResponse = read(request.send().await.map_err(network)?).await?;
        Ok(response.message)
    };
    within(controller, SEND_DEADLINE_MS, attempt).await
}

fn send_request(row: &Outgoing) -> SendRequest<'_> {
    SendRequest {
        client_msg_id: &row.client_msg_id,
        body: &row.body,
        reply_to_message_id: row.reply_to_message_id,
        mentions: &row.mentions,
        poll: row.poll.as_deref().map(|options| PollRequest { options }),
        attachment_ids: row
            .items
            .iter()
            .filter_map(|item| item.attachment_id)
            .collect(),
    }
}

/// How long one upload may take before it is given up as UNKNOWN: ten
/// minutes, the apps' budget. A hundred megabytes over a slow uplink is
/// minutes, and the ten seconds a message gets would cancel every video.
pub const UPLOAD_DEADLINE_MS: u32 = 600_000;

/// A preview is a few tens of kilobytes: a minute is generous, and a hung
/// one must not hold the whole outbox for ten.
pub const PREVIEW_DEADLINE_MS: u32 = 60_000;

/// Send `request`, aborting it past `deadline_ms` so a late answer cannot
/// arrive for an attempt that has already been counted as unknown.
async fn within<T>(
    controller: AbortController,
    deadline_ms: u32,
    attempt: impl std::future::Future<Output = Result<T, ApiError>>,
) -> Result<T, ApiError> {
    match select(Box::pin(attempt), TimeoutFuture::new(deadline_ms)).await {
        Either::Left((answer, _)) => answer,
        Either::Right(_) => {
            controller.abort();
            Err(ApiError::Network(
                "The server did not answer in time.".into(),
            ))
        }
    }
}

fn controller() -> Result<AbortController, ApiError> {
    AbortController::new()
        .map_err(|_| ApiError::Network("This browser cannot time a request out.".into()))
}

/// The query string an upload's metadata rides in (docs/protocol.md,
/// "Attachments"): only what the item has.
pub fn upload_query(item: &OutgoingItem) -> String {
    let mut query = vec![format!("kind={}", item.kind)];
    let mut number = |key: &str, value: Option<i64>| {
        if let Some(value) = value {
            query.push(format!("{key}={value}"));
        }
    };
    number("width", item.width);
    number("height", item.height);
    number("duration_ms", item.duration_ms);
    if let (Some(latitude), Some(longitude)) = (item.latitude, item.longitude) {
        // Seven places: a centimetre, and more than any fix is good for.
        query.push(format!("latitude={latitude:.7}"));
        query.push(format!("longitude={longitude:.7}"));
        if let Some(accuracy) = item.accuracy_m.filter(|accuracy| accuracy.is_finite()) {
            query.push(format!("accuracy_m={}", accuracy.round().max(0.0) as i64));
        }
    }
    if let Some(name) = item.name.as_deref().filter(|name| !name.is_empty()) {
        query.push(format!(
            "name={}",
            String::from(js_sys::encode_uri_component(name))
        ));
    }
    query.join("&")
}

/// `POST /attachments` — the bytes, raw, with their type; the metadata in
/// the query. A location sends no body at all.
pub async fn upload_attachment(
    token: &str,
    item: &OutgoingItem,
    file: Option<&Blob>,
) -> Result<Attachment, ApiError> {
    let controller = controller()?;
    let url = path(&format!("/attachments?{}", upload_query(item)));
    let mut request = bearer(Request::post(&url), token).abort_signal(Some(&controller.signal()));
    if !item.mime.is_empty() && !item.is_location() {
        request = request.header("Content-Type", &item.mime);
    }
    let request = match file {
        Some(file) => request.body(wasm_bindgen::JsValue::from(file.clone())),
        None => request.build(),
    }
    .map_err(network)?;
    let attempt = async {
        let response: AttachmentResponse = read(request.send().await.map_err(network)?).await?;
        Ok(response.attachment)
    };
    within(controller, UPLOAD_DEADLINE_MS, attempt).await
}

/// `PUT /attachments/{id}/preview` — the downscaled photo or the poster.
/// Best effort: a message may go without it (docs/protocol.md).
pub async fn upload_preview(token: &str, attachment_id: i64, jpeg: &Blob) -> Result<(), ApiError> {
    let controller = controller()?;
    let request = bearer(
        Request::put(&path(&format!("/attachments/{attachment_id}/preview"))),
        token,
    )
    .header("Content-Type", "image/jpeg")
    .abort_signal(Some(&controller.signal()))
    .body(wasm_bindgen::JsValue::from(jpeg.clone()))
    .map_err(network)?;
    let attempt = async {
        let response = request.send().await.map_err(network)?;
        check(&response).await
    };
    within(controller, PREVIEW_DEADLINE_MS, attempt).await
}

/// `GET /attachments/{id}` or its preview, as bytes in this tab's memory.
///
/// `no-store`: the server marks media `private, immutable`, which is right
/// for an app's own cache and wrong for a browser's — shared with every
/// account that signs in on the machine, and outliving the tab
/// (docs/protocol.md, "A browser is a client too").
pub async fn attachment_bytes(token: &str, id: i64, preview: bool) -> Result<Blob, ApiError> {
    let url = if preview {
        path(&format!("/attachments/{id}/preview"))
    } else {
        path(&format!("/attachments/{id}"))
    };
    let response = bearer(Request::get(&url), token)
        .cache(RequestCache::NoStore)
        .send()
        .await
        .map_err(network)?;
    check(&response).await?;
    // A FILE is served as an attachment that must never render (docs/
    // protocol.md, "Files"), and that header does not travel with the
    // bytes into a blob: URL of this page's own origin. So its bytes are
    // re-typed as the least interesting type there is, and an uploaded
    // .html or .svg can only ever be saved, never run here.
    let is_file = response
        .headers()
        .get("Content-Disposition")
        .is_some_and(|value| value.to_ascii_lowercase().starts_with("attachment"));
    let raw: web_sys::Response = response.into();
    let unreadable = || ApiError::Network("The answer could not be read.".into());
    let promise = raw.blob().map_err(|_| unreadable())?;
    let blob = wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map_err(|_| unreadable())?
        .dyn_into::<Blob>()
        .map_err(|_| unreadable())?;
    if !is_file {
        return Ok(blob);
    }
    let options = web_sys::BlobPropertyBag::new();
    options.set_type("application/octet-stream");
    Blob::new_with_blob_sequence_and_options(&js_sys::Array::of1(&blob), &options)
        .map_err(|_| unreadable())
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

/// `GET /families/mine/board` — the whole wall as it now stands.
pub async fn board(token: &str) -> Result<BoardRead, ApiError> {
    get(token, "/families/mine/board").await
}

/// `GET /families/mine/board/changes?after_seq=` — the board catch-up,
/// tombstones included, looped by the caller until a short page.
pub async fn board_changes(token: &str, after_seq: i64, limit: u32) -> Result<Vec<Note>, ApiError> {
    let url = format!("/families/mine/board/changes?after_seq={after_seq}&limit={limit}");
    let response: NotesResponse = get(token, &url).await?;
    Ok(response.notes)
}

/// `POST /families/mine/board/notes` — the caller becomes the author.
pub async fn create_note(token: &str, note: &NewNote) -> Result<Note, ApiError> {
    let response: NoteResponse = with_body(
        Request::post(&path("/families/mine/board/notes")),
        token,
        note,
    )
    .await?;
    Ok(response.note)
}

/// `PATCH /families/mine/board/notes/{id}` — a move (anyone) or a rewrite
/// (the author), by which fields are present.
pub async fn patch_note(token: &str, note_id: i64, patch: &NotePatch) -> Result<Note, ApiError> {
    let url = path(&format!("/families/mine/board/notes/{note_id}"));
    let response: NoteResponse = with_body(Request::patch(&url), token, patch).await?;
    Ok(response.note)
}

/// `PUT` / `DELETE …/notes/{id}/rsvp` — say whether you are coming, or take
/// it back. Anyone in the family may; answering is not authorship.
pub async fn answer_event(
    token: &str,
    note_id: i64,
    answer: Option<&str>,
) -> Result<Note, ApiError> {
    let url = path(&format!("/families/mine/board/notes/{note_id}/rsvp"));
    let response: NoteResponse = match answer {
        Some(answer) => with_body(Request::put(&url), token, &RsvpRequest { answer }).await?,
        None => {
            read(
                bearer(Request::delete(&url), token)
                    .send()
                    .await
                    .map_err(network)?,
            )
            .await?
        }
    };
    Ok(response.note)
}

/// `DELETE /families/mine/board/notes/{id}` — the author's; idempotent.
pub async fn delete_note(token: &str, note_id: i64) -> Result<(), ApiError> {
    let url = path(&format!("/families/mine/board/notes/{note_id}"));
    empty::<()>(Request::delete(&url), token, None).await
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
            items: Vec::new(),
            attempts: 0,
            failed: None,
        }
    }

    fn encode(row: &Outgoing) -> serde_json::Value {
        serde_json::to_value(send_request(row)).expect("encodes")
    }

    fn item(kind: &str, provisional_id: i64) -> OutgoingItem {
        OutgoingItem {
            provisional_id,
            kind: kind.into(),
            mime: String::new(),
            size: 0,
            width: None,
            height: None,
            duration_ms: None,
            name: None,
            latitude: None,
            longitude: None,
            accuracy_m: None,
            has_preview: false,
            attachment_id: None,
        }
    }

    /// The array spelling, in the order chosen, carrying only what landed
    /// — and nothing at all for a message without attachments.
    #[wasm_bindgen_test]
    fn a_send_names_its_attachments_in_order() {
        let mut photos = row();
        photos.body = String::new();
        let mut first = item("photo", -1);
        first.attachment_id = Some(34);
        let mut second = item("photo", -2);
        second.attachment_id = Some(35);
        photos.items = vec![first, second];
        assert_eq!(
            encode(&photos),
            serde_json::json!({"client_msg_id": "8f14e45f", "body": "", "attachment_ids": [34, 35]})
        );
    }

    /// The upload's query: the kind, what the item has, the name escaped —
    /// and a location's three numbers, to seven places and whole metres.
    #[wasm_bindgen_test]
    fn an_upload_says_what_it_is_in_the_query() {
        let mut video = item("video", -1);
        video.width = Some(1920);
        video.height = Some(1080);
        video.duration_ms = Some(8400);
        assert_eq!(
            upload_query(&video),
            "kind=video&width=1920&height=1080&duration_ms=8400"
        );
        let mut file = item("file", -2);
        file.name = Some("Q3 report & notes.pdf".into());
        assert_eq!(
            upload_query(&file),
            "kind=file&name=Q3%20report%20%26%20notes.pdf"
        );
        let mut place = item("location", -3);
        place.latitude = Some(55.7558);
        place.longitude = Some(37.6173);
        place.accuracy_m = Some(12.4);
        assert_eq!(
            upload_query(&place),
            "kind=location&latitude=55.7558000&longitude=37.6173000&accuracy_m=12"
        );
        place.accuracy_m = None;
        assert_eq!(
            upload_query(&place),
            "kind=location&latitude=55.7558000&longitude=37.6173000",
            "no accuracy is no accuracy, never zero"
        );
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

    /// Each kind carries exactly its own parts: a text note no kind and no
    /// picture, a photo its picture and an empty caption, an event its
    /// times and place — never a null where the protocol means absent.
    #[wasm_bindgen_test]
    fn a_new_note_carries_only_what_its_kind_has() {
        let text = NewNote {
            text: "Milk".into(),
            color: "yellow".into(),
            size: "medium".into(),
            font: "plain".into(),
            x: 0.4,
            y: 0.3,
            ..NewNote::default()
        };
        assert_eq!(
            serde_json::to_value(&text).expect("encodes"),
            serde_json::json!({"text": "Milk", "color": "yellow", "size": "medium",
                               "font": "plain", "x": 0.4, "y": 0.3})
        );
        let photo = NewNote {
            text: String::new(),
            kind: Some("photo".into()),
            attachment_id: Some(34),
            ..text.clone()
        };
        let value = serde_json::to_value(&photo).expect("encodes");
        assert_eq!(value["kind"], "photo");
        assert_eq!(value["attachment_id"], 34);
        assert_eq!(value["text"], "");
        assert!(value.get("starts_at").is_none());
        let event = NewNote {
            kind: Some("event".into()),
            starts_at: Some("2026-09-12T11:00:00Z".into()),
            place: Some("The park".into()),
            ..text
        };
        let value = serde_json::to_value(&event).expect("encodes");
        assert_eq!(value["starts_at"], "2026-09-12T11:00:00Z");
        assert!(
            value.get("ends_at").is_none(),
            "no end is left out, not null"
        );
        assert!(value.get("attachment_id").is_none());
    }

    /// A move is x and y and NOTHING else; an end cleared is null, an end
    /// left alone is absent; and an edit that changed nothing is empty.
    #[wasm_bindgen_test]
    fn a_patch_sends_only_what_changed() {
        assert_eq!(
            serde_json::to_value(NotePatch::moved_to(0.25, 0.5)).expect("encodes"),
            serde_json::json!({"x": 0.25, "y": 0.5})
        );
        let cleared = NotePatch {
            ends_at: Some(None),
            ..NotePatch::default()
        };
        assert_eq!(
            serde_json::to_value(&cleared).expect("encodes"),
            serde_json::json!({"ends_at": null})
        );
        let moved_end = NotePatch {
            ends_at: Some(Some("2026-09-12T13:00:00Z".into())),
            place: Some(String::new()),
            ..NotePatch::default()
        };
        assert_eq!(
            serde_json::to_value(&moved_end).expect("encodes"),
            serde_json::json!({"ends_at": "2026-09-12T13:00:00Z", "place": ""})
        );
        assert!(NotePatch::default().is_empty());
        assert!(!cleared.is_empty());
    }

    #[wasm_bindgen_test]
    fn the_board_reads_its_whole_wall_and_its_changes() {
        let read: BoardRead = serde_json::from_str(
            r#"{"notes": [{"id": 12, "author_id": 7, "text": "Milk", "color": "yellow",
                           "x": 0.4, "y": 0.1, "board_seq": 88}], "max_board_seq": 91}"#,
        )
        .expect("reads");
        assert_eq!(read.max_board_seq, 91);
        assert_eq!(read.notes[0].id, 12);
        let changes: NotesResponse =
            serde_json::from_str(r#"{"notes": [{"id": 12, "deleted": true, "board_seq": 92}]}"#)
                .expect("reads");
        assert!(changes.notes[0].deleted);
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
