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

use fc_text::i18n::{t, t1};
use futures::future::{select, Either};
use gloo_net::http::{Request, RequestBuilder, Response};
use gloo_timers::future::TimeoutFuture;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use wasm_bindgen::JsCast;
use web_sys::{AbortController, Blob, RequestCache};

use crate::model::{
    Attachment, Birthday, Chat, ChatListItem, Family, JoinRequest, Me, Member, Mention, Message,
    Note, PackItem, Poll, Reaction, Report, Roster, Stats, User,
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
    /// It ANSWERED, but not in the protocol's shape: a status with no body a
    /// client can read (nginx's own rate limit, a 502 while the server
    /// restarts). A variant of its own rather than a sentence, because
    /// "the server had a problem" is a different thing to say than "can't
    /// reach the server" — and because a sentence to match on stops
    /// matching the moment it is translated.
    Answered { status: u16 },
    /// It did not answer, or the answer was not what this client can read.
    Network(String),
}

impl ApiError {
    /// What to put in front of a person. The protocol's `message` is
    /// English and meant for developers, so the caller decides whether to
    /// use it; this is the fallback when there is nothing better.
    pub fn detail(&self) -> String {
        match self {
            ApiError::Unauthorized => t("Your session has expired.").to_string(),
            ApiError::Server { message, .. } => message.clone(),
            ApiError::Throttled { .. } => {
                t("The server is busy. Try again in a moment.").to_string()
            }
            ApiError::Answered { status } => t1("The server answered %lld.", &status.to_string()),
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

#[derive(Debug, Serialize)]
struct RegisterRequest<'a> {
    username: &'a str,
    display_name: &'a str,
    password: &'a str,
}

#[derive(Debug, Deserialize)]
struct FamilyResponse {
    family: Family,
}

#[derive(Debug, Deserialize)]
struct UserResponse {
    user: User,
}

#[derive(Debug, Deserialize)]
struct MemberResponse {
    member: Member,
}

/// `POST /families/join`'s answer: `joined` or `pending`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Joined {
    pub status: String,
}

/// `POST /families/leave`'s answer when the caller was the owner and the
/// family passed to somebody.
#[derive(Debug, Deserialize)]
struct LeftResponse {
    #[serde(default)]
    new_owner_user_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct InviteCodeResponse {
    invite_code: String,
}

#[derive(Debug, Deserialize)]
struct RequestsResponse {
    requests: Vec<JoinRequest>,
}

#[derive(Debug, Deserialize)]
struct ReportsResponse {
    reports: Vec<Report>,
}

/// A change to the family's settings — the owner's. Only what is sent
/// changes, and `max_members` and `language` are the protocol's two DOUBLE
/// options: `Some(None)` — sent as null — clears them, absent leaves them.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct FamilyPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub join_policy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_members: Option<Option<i64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_history: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_vision: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_history_photos: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_greeting: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_faces: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_transcripts: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai_lookups: Option<bool>,
    /// The places for the greeting's weather: the WHOLE list, replacing
    /// the stored one — `Some(vec![])` clears it, `None` leaves it alone.
    /// Never a null: the server refuses one (docs/protocol.md, "Today's
    /// weather, for places the owner chose").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub greeting_places: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct AuthResponse {
    pub token: String,
}

#[derive(Debug, Deserialize)]
struct IceResponse {
    #[serde(default)]
    ice_servers: Vec<crate::model::IceServer>,
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
    /// What makes the message a sticker (docs/protocol.md, "Sending one").
    /// Left out when it is not one — absent is an ordinary message, and a
    /// server from before stickers never sees a field it would ignore.
    #[serde(skip_serializing_if = "is_false")]
    sticker: bool,
    /// What makes the message a video message (docs/protocol.md, "Video
    /// messages"): left out when it is not one, never sent as `false`.
    #[serde(skip_serializing_if = "is_false")]
    round: bool,
}

fn is_false(flag: &bool) -> bool {
    !*flag
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

/// `GET /families/mine/pack`: the whole sticker pack, tombstones excluded,
/// in the order it was added to — and the high-water mark read before it
/// (docs/protocol.md, "Sticker pack"). 0 for a pack never written to.
#[derive(Debug, Clone, Deserialize)]
pub struct PackRead {
    pub items: Vec<PackItem>,
    #[serde(default)]
    pub max_pack_seq: i64,
}

#[derive(Debug, Deserialize)]
struct PackItemsResponse {
    items: Vec<PackItem>,
}

#[derive(Debug, Deserialize)]
struct PackItemResponse {
    item: PackItem,
}

/// A claim of an upload for the pack. The label is LEFT OUT when there is
/// none — an empty one is no label, and absent says so without a word.
#[derive(Debug, Serialize)]
struct PackClaim<'a> {
    attachment_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<&'a str>,
}

/// What a claim did. `added` is false when the pack ALREADY held it — the
/// same upload claimed again, or other bytes identical to a live item's —
/// which is `200` and not an error: the item that comes back is the one
/// that was there, under the attachment id the pack already had, and may
/// not be the id the claim named.
#[derive(Debug, Clone, PartialEq)]
pub struct PackClaimed {
    pub item: PackItem,
    pub added: bool,
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
    /// The members the text names (docs/protocol.md, "Board").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mentions: Option<Vec<crate::model::Mention>>,
    /// A task list's lines. Sent only on a `tasks` note, where the text is
    /// the title (docs/protocol.md, "Board").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items: Option<Vec<TaskLine>>,
}

/// One line the author is writing. `id` says "the line you already have",
/// which is what carries its TICK through a rewrite; absent, it is new.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TaskLine {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    pub text: String,
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
    /// REPLACES the note's names, and is sent with every text edit — a
    /// note's names are re-decided on every edit, and a text patch without
    /// them clears them (docs/protocol.md, "Board").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mentions: Option<Vec<crate::model::Mention>>,
    /// REPLACES a task list's lines: a line that carries its id keeps its
    /// TICK, one without an id is new, and a line left out is gone
    /// (docs/protocol.md, "Board").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items: Option<Vec<TaskLine>>,
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

/// `POST /reports/assistant` — not under `/families`: it needs no family,
/// and no family owner may read it (docs/protocol.md, "Reporting the
/// assistant").
#[derive(Debug, Serialize)]
struct AssistantReportRequest<'a> {
    message_id: i64,
    reason: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<&'a str>,
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
        return Err(unauthorized(response).await);
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
        // A proxy's own 413 is HTML, and it is still an answer — the body
        // was too big for what stands in front of the server — never "can't
        // reach the server".
        Err(_) if status == 413 => Err(ApiError::Server {
            code: TOO_LARGE.to_string(),
            message: String::new(),
        }),
        // A body that is not the protocol's shape is still a failure, and
        // saying which status it was beats saying nothing: nginx answers
        // its own rate limit and a dead upstream with HTML.
        Err(_) => Err(ApiError::Answered { status }),
    }
}

/// Not the server's: this client's own name for a bare 413 from a proxy.
pub const TOO_LARGE: &str = "payload_too_large";

/// A 401 is a session that is gone — EXCEPT `invalid_credentials`, which
/// shares the status and not the meaning: a password typed wrong at sign-in,
/// or as the proof `POST /me/password` and `POST /me/delete` ask for
/// (docs/protocol.md, "Error shape"). Signing somebody out for a typo would
/// be the wrong answer to it, and "your session has expired" the wrong
/// sentence.
async fn unauthorized(response: &Response) -> ApiError {
    match response.json::<ErrorEnvelope>().await {
        Ok(envelope) if envelope.error.code == "invalid_credentials" => ApiError::Server {
            code: envelope.error.code,
            message: envelope.error.message,
        },
        _ => ApiError::Unauthorized,
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

/// `POST /auth/register` — a new account, signed in. Usernames are unique
/// ignoring case; the account belongs to no family until it makes or joins
/// one.
pub async fn register(
    username: &str,
    display_name: &str,
    password: &str,
) -> Result<AuthResponse, ApiError> {
    let request = Request::post(&path("/auth/register"))
        .json(&RegisterRequest {
            username,
            display_name,
            password,
        })
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

/// `POST /families` — a new family, with the caller as its owner.
pub async fn create_family(token: &str, name: &str) -> Result<Family, ApiError> {
    let response: FamilyResponse = with_body(
        Request::post(&path("/families")),
        token,
        &serde_json::json!({ "name": name }),
    )
    .await?;
    Ok(response.family)
}

/// `POST /families/join` — in at once, or a request waiting on the owner.
pub async fn join_family(token: &str, invite_code: &str) -> Result<Joined, ApiError> {
    with_body(
        Request::post(&path("/families/join")),
        token,
        &serde_json::json!({ "invite_code": invite_code }),
    )
    .await
}

/// `POST /families/leave` — answers who the family passed to, when the
/// caller was its owner and somebody remains.
pub async fn leave_family(token: &str) -> Result<Option<i64>, ApiError> {
    let response = bearer(Request::post(&path("/families/leave")), token)
        .send()
        .await
        .map_err(network)?;
    check(&response).await?;
    if response.status() == 204 {
        return Ok(None);
    }
    let left: LeftResponse = response
        .json()
        .await
        .map_err(|error| ApiError::Network(error.to_string()))?;
    Ok(left.new_owner_user_id)
}

/// `PATCH /families/mine` — the owner's settings; only what is sent changes.
pub async fn patch_family(token: &str, patch: &FamilyPatch) -> Result<Family, ApiError> {
    let response: FamilyResponse =
        with_body(Request::patch(&path("/families/mine")), token, patch).await?;
    Ok(response.family)
}

/// `POST /families/invite-code/rotate` — the old code stops working at once.
pub async fn rotate_invite_code(token: &str) -> Result<String, ApiError> {
    let response: InviteCodeResponse = read(
        bearer(Request::post(&path("/families/invite-code/rotate")), token)
            .send()
            .await
            .map_err(network)?,
    )
    .await?;
    Ok(response.invite_code)
}

/// `GET /families/join-requests` — the owner's, pending only.
pub async fn join_requests(token: &str) -> Result<Vec<JoinRequest>, ApiError> {
    let response: RequestsResponse = get(token, "/families/join-requests").await?;
    Ok(response.requests)
}

/// `POST /families/join-requests/{id}/approve` or `…/reject`.
pub async fn decide_join_request(token: &str, id: i64, approve: bool) -> Result<(), ApiError> {
    let verb = if approve { "approve" } else { "reject" };
    let url = path(&format!("/families/join-requests/{id}/{verb}"));
    empty::<()>(Request::post(&url), token, None).await
}

/// `GET /families/reports` — the owner's open reports, oldest first.
pub async fn reports(token: &str) -> Result<Vec<Report>, ApiError> {
    let response: ReportsResponse = get(token, "/families/reports").await?;
    Ok(response.reports)
}

/// `POST /families/reports/{id}/resolve` — off the owner's list.
pub async fn resolve_report(token: &str, id: i64) -> Result<(), ApiError> {
    let url = path(&format!("/families/reports/{id}/resolve"));
    empty::<()>(Request::post(&url), token, None).await
}

/// `DELETE /families/members/{id}` — the owner removes somebody.
pub async fn remove_member(token: &str, user_id: i64) -> Result<(), ApiError> {
    let url = path(&format!("/families/members/{user_id}"));
    empty::<()>(Request::delete(&url), token, None).await
}

/// `POST /families/members/{id}/password` — the owner sets a new password
/// for somebody who has forgotten theirs; every device of theirs signs out.
pub async fn reset_member_password(
    token: &str,
    user_id: i64,
    new_password: &str,
) -> Result<(), ApiError> {
    let url = path(&format!("/families/members/{user_id}/password"));
    empty(
        Request::post(&url),
        token,
        Some(&serde_json::json!({ "new_password": new_password })),
    )
    .await
}

/// `PUT` / `DELETE /families/members/{id}/birthday` — the owner filling one
/// in for somebody, or clearing it.
pub async fn set_member_birthday(
    token: &str,
    user_id: i64,
    birthday: Option<Birthday>,
) -> Result<Option<Member>, ApiError> {
    let url = path(&format!("/families/members/{user_id}/birthday"));
    match birthday {
        Some(birthday) => {
            let response: MemberResponse = with_body(Request::put(&url), token, &birthday).await?;
            Ok(Some(response.member))
        }
        None => {
            empty::<()>(Request::delete(&url), token, None).await?;
            Ok(None)
        }
    }
}

/// `GET /families/mine/stats` — the family's numbers, for every member.
pub async fn stats(token: &str) -> Result<Stats, ApiError> {
    get(token, "/families/mine/stats").await
}

/// `PUT` / `DELETE /me/birthday` — your own.
pub async fn set_my_birthday(
    token: &str,
    birthday: Option<Birthday>,
) -> Result<Option<User>, ApiError> {
    let url = path("/me/birthday");
    match birthday {
        Some(birthday) => {
            let response: UserResponse = with_body(Request::put(&url), token, &birthday).await?;
            Ok(Some(response.user))
        }
        None => {
            empty::<()>(Request::delete(&url), token, None).await?;
            Ok(None)
        }
    }
}

/// `POST /me/password` — proving the current one; every OTHER session goes.
pub async fn change_password(token: &str, current: &str, new: &str) -> Result<(), ApiError> {
    empty(
        Request::post(&path("/me/password")),
        token,
        Some(&serde_json::json!({ "current_password": current, "new_password": new })),
    )
    .await
}

/// `POST /me/delete` — the account, permanently, on its password.
pub async fn delete_account(token: &str, password: &str) -> Result<(), ApiError> {
    empty(
        Request::post(&path("/me/delete")),
        token,
        Some(&serde_json::json!({ "password": password })),
    )
    .await
}

/// `GET /calls/ice` — the STUN and TURN servers for one call, with the
/// credentials the operator minted for this caller. Fetched at the start of
/// every call and never kept: a stale credential is a call that silently
/// cannot relay (docs/protocol.md, "Where the servers come from").
pub async fn ice_servers(token: &str) -> Result<Vec<crate::model::IceServer>, ApiError> {
    let response: IceResponse = get(token, "/calls/ice").await?;
    Ok(response.ice_servers)
}

/// `PUT /me/avatar` — the picture, as raw JPEG bytes.
pub async fn upload_avatar(token: &str, jpeg: &Blob) -> Result<User, ApiError> {
    let request = bearer(Request::put(&path("/me/avatar")), token)
        .header("Content-Type", "image/jpeg")
        .body(wasm_bindgen::JsValue::from(jpeg.clone()))
        .map_err(network)?;
    let response: UserResponse = read(request.send().await.map_err(network)?).await?;
    Ok(response.user)
}

/// `DELETE /me/avatar` — back to initials.
pub async fn delete_avatar(token: &str) -> Result<(), ApiError> {
    empty::<()>(Request::delete(&path("/me/avatar")), token, None).await
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
        sticker: row.sticker,
        round: row.round,
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
                t("The server did not answer in time.").to_string(),
            ))
        }
    }
}

fn controller() -> Result<AbortController, ApiError> {
    AbortController::new()
        .map_err(|_| ApiError::Network(t("This browser cannot time a request out.").to_string()))
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
    let unreadable = || ApiError::Network(t("The answer could not be read.").to_string());
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

/// `GET /users/{id}/avatar` — a profile picture, as bytes in this tab's
/// memory, under the same `no-store` rule as every other picture: the
/// server's `immutable` is right for an app's cache and wrong for a browser
/// every account on the machine shares (docs/protocol.md, "A browser is a
/// client too").
pub async fn avatar_bytes(token: &str, user_id: i64) -> Result<Blob, ApiError> {
    let response = bearer(
        Request::get(&path(&format!("/users/{user_id}/avatar"))),
        token,
    )
    .cache(RequestCache::NoStore)
    .send()
    .await
    .map_err(network)?;
    check(&response).await?;
    let raw: web_sys::Response = response.into();
    let unreadable = || ApiError::Network(t("The answer could not be read.").to_string());
    let promise = raw.blob().map_err(|_| unreadable())?;
    wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map_err(|_| unreadable())?
        .dyn_into::<Blob>()
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

/// `PUT …/notes/{id}/tasks/{item_id}` — tick or untick one line. Anyone in
/// the family may; ticking is not authorship, and it is a STATE rather than
/// a toggle so two phones cannot undo each other (docs/protocol.md,
/// "Board").
pub async fn tick_task(
    token: &str,
    note_id: i64,
    item_id: i64,
    done: bool,
) -> Result<Note, ApiError> {
    let url = path(&format!(
        "/families/mine/board/notes/{note_id}/tasks/{item_id}"
    ));
    let response: NoteResponse = with_body(Request::put(&url), token, &TaskDone { done }).await?;
    Ok(response.note)
}

#[derive(Debug, Serialize)]
struct TaskDone {
    done: bool,
}

/// `POST …/notes/{id}/backdrop` — ask the assistant for a picture to sit
/// behind an event. The AUTHOR's, and drawn from the note's own title: there
/// is nothing to send (docs/protocol.md, "Board").
pub async fn draw_backdrop(token: &str, note_id: i64) -> Result<Note, ApiError> {
    let url = path(&format!("/families/mine/board/notes/{note_id}/backdrop"));
    let response: NoteResponse =
        with_body(Request::post(&url), token, &serde_json::json!({})).await?;
    Ok(response.note)
}

/// How long asking for a recording's text may take before this client
/// stops waiting: its OWN deadline, never an ordinary request's — the
/// provider listens to the whole recording first, and the protocol's floor
/// for it is 90 s. Five minutes is the reference proxy's own read timeout
/// on this route, so waiting longer could only wait on a closed connection.
/// Giving up loses nothing: the server finishes a stored-bytes request and
/// keeps its answer, so asking again later is answered at once
/// (docs/protocol.md, "Transcripts on request").
pub const TRANSCRIPT_DEADLINE_MS: u32 = 300_000;

/// `POST /chats/{id}/messages/{id}/attachments/{id}/transcript` — the text
/// of a voice note or an audio file, from the server's STORED bytes.
///
/// The body is `{}`: anything that is not `multipart/form-data` asks for
/// the stored copy. The answer is this asker's alone.
pub async fn transcript(
    token: &str,
    chat_id: i64,
    message_id: i64,
    attachment_id: i64,
) -> Result<fc_text::transcript::Transcript, ApiError> {
    transcript_within(
        token,
        chat_id,
        message_id,
        attachment_id,
        None,
        TRANSCRIPT_DEADLINE_MS,
    )
    .await
}

/// The name the supplied sound's part goes under, as the protocol names it.
pub const SUPPLIED_PART: &str = "audio";

/// The type the supplied sound's part declares: AAC in an MPEG-4 container,
/// the one shape the server takes.
pub const SUPPLIED_TYPE: &str = "audio/mp4";

/// The same request with SOUND THIS DEVICE SUPPLIED — the sound track of a
/// video, or of an audio file the server's stored copy will not do for —
/// as `multipart/form-data` with one part, [`SUPPLIED_PART`], an M4A
/// (fc_text::transcript_sound). The server keeps nothing of the answer, so
/// asking again asks the provider again; this device keeps it instead.
pub async fn transcript_of_sound(
    token: &str,
    chat_id: i64,
    message_id: i64,
    attachment_id: i64,
    sound: &Blob,
) -> Result<fc_text::transcript::Transcript, ApiError> {
    transcript_within(
        token,
        chat_id,
        message_id,
        attachment_id,
        Some(sound),
        TRANSCRIPT_DEADLINE_MS,
    )
    .await
}

/// `sound` as the multipart form the server reads: one part named `audio`,
/// declared `audio/mp4`, under a name ending `.m4a`. The browser writes the
/// boundary and the part's headers itself, and sets the request's
/// `Content-Type` to match — which is why the request names none.
fn supplied_form(sound: &Blob) -> Result<web_sys::FormData, ApiError> {
    // Never shown as it is: a request with no code is "try again".
    let unmade = |error: wasm_bindgen::JsValue| ApiError::Network(format!("{error:?}"));
    let sound = if sound.type_() == SUPPLIED_TYPE {
        sound.clone()
    } else {
        let options = web_sys::BlobPropertyBag::new();
        options.set_type(SUPPLIED_TYPE);
        Blob::new_with_blob_sequence_and_options(&js_sys::Array::of1(sound), &options)
            .map_err(unmade)?
    };
    let form = web_sys::FormData::new().map_err(unmade)?;
    form.append_with_blob_and_filename(SUPPLIED_PART, &sound, "sound.m4a")
        .map_err(unmade)?;
    Ok(form)
}

async fn transcript_within(
    token: &str,
    chat_id: i64,
    message_id: i64,
    attachment_id: i64,
    sound: Option<&Blob>,
    deadline_ms: u32,
) -> Result<fc_text::transcript::Transcript, ApiError> {
    let controller = controller()?;
    let url = path(&format!(
        "/chats/{chat_id}/messages/{message_id}/attachments/{attachment_id}/transcript"
    ));
    let request = bearer(Request::post(&url), token).abort_signal(Some(&controller.signal()));
    let request = match sound {
        None => request.json(&serde_json::json!({})),
        Some(sound) => request.body(wasm_bindgen::JsValue::from(supplied_form(sound)?)),
    }
    .map_err(network)?;
    let attempt = async {
        let response: TranscriptResponse = read(request.send().await.map_err(network)?).await?;
        Ok(fc_text::transcript::Transcript {
            text: response.transcript.text,
            language: response
                .transcript
                .language
                .filter(|language| !language.trim().is_empty()),
        })
    };
    within(controller, deadline_ms, attempt).await
}

#[derive(Debug, Deserialize)]
struct TranscriptResponse {
    transcript: TranscriptBody,
}

/// `text` is always present — `""` is silence, an answer.
#[derive(Debug, Deserialize)]
struct TranscriptBody {
    text: String,
    #[serde(default)]
    language: Option<String>,
}

/// `DELETE /families/mine/board/notes/{id}` — the author's; idempotent.
pub async fn delete_note(token: &str, note_id: i64) -> Result<(), ApiError> {
    let url = path(&format!("/families/mine/board/notes/{note_id}"));
    empty::<()>(Request::delete(&url), token, None).await
}

/// `GET /families/mine/pack` — the whole sticker pack as it now stands.
pub async fn pack(token: &str) -> Result<PackRead, ApiError> {
    get(token, "/families/mine/pack").await
}

/// `GET /families/mine/pack/changes?after_seq=` — the pack catch-up,
/// tombstones included, looped by the caller until a short page.
pub async fn pack_changes(
    token: &str,
    after_seq: i64,
    limit: u32,
) -> Result<Vec<PackItem>, ApiError> {
    let url = format!("/families/mine/pack/changes?after_seq={after_seq}&limit={limit}");
    let response: PackItemsResponse = get(token, &url).await?;
    Ok(response.items)
}

/// `POST /families/mine/pack` — claim an upload of the caller's own as a
/// sticker. Any member may. `201` is a new item; `200` is the item the pack
/// already held, which takes no seq and sends no frame.
pub async fn add_pack_item(
    token: &str,
    attachment_id: i64,
    label: Option<&str>,
) -> Result<PackClaimed, ApiError> {
    let request = bearer(Request::post(&path("/families/mine/pack")), token)
        .json(&PackClaim {
            attachment_id,
            label,
        })
        .map_err(network)?;
    let response = request.send().await.map_err(network)?;
    let added = response.status() == 201;
    let answer: PackItemResponse = read(response).await?;
    Ok(PackClaimed {
        item: answer.item,
        added,
    })
}

/// `DELETE /families/mine/pack/{id}` — whoever added it, or the family
/// owner; idempotent.
pub async fn remove_pack_item(token: &str, item_id: i64) -> Result<(), ApiError> {
    let url = path(&format!("/families/mine/pack/{item_id}"));
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

/// `POST /reports/assistant`: what the ASSISTANT got wrong.
///
/// A separate endpoint from [`report`], and deliberately so — the assistant
/// belongs to no family, so the member-report endpoint answers
/// `not_same_family`, and this row is read by the people who run the server
/// rather than by the family owner (docs/protocol.md, "Reporting the
/// assistant"). Reporting the same reply twice answers 200 with the stored
/// row and creates nothing, so this needs no idempotency of its own.
pub async fn report_assistant(
    token: &str,
    message_id: i64,
    reason: &str,
    note: Option<&str>,
) -> Result<(), ApiError> {
    let note = note.map(str::trim).filter(|note| !note.is_empty());
    let body = AssistantReportRequest {
        message_id,
        reason,
        note,
    };
    empty(
        Request::post(&path("/reports/assistant")),
        token,
        Some(&body),
    )
    .await
}

/// `POST /me/assistant-consent` — this member's own permission for their
/// words to go to the model, and nobody else's (docs/protocol.md,
/// "Consenting to the assistant").
///
/// Answers with the stamp the server now holds: a date when granted, none
/// when withdrawn. Idempotent both ways — granting twice keeps the FIRST
/// date, because when somebody agreed is a fact and not a counter. A
/// server with no assistant answers 404 rather than admitting there is
/// nothing to consent to.
pub async fn set_assistant_consent(token: &str, granted: bool) -> Result<Option<String>, ApiError> {
    let body = AssistantConsentRequest { granted };
    let answer: AssistantConsentResponse =
        with_body(Request::post(&path("/me/assistant-consent")), token, &body).await?;
    Ok(answer.assistant_consent_at)
}

#[derive(Debug, Serialize)]
struct AssistantConsentRequest {
    granted: bool,
}

#[derive(Debug, Deserialize)]
struct AssistantConsentResponse {
    #[serde(default)]
    assistant_consent_at: Option<String>,
}

/// `POST /me/assistant-lookup-consent` — this member's own agreement that
/// the assistant may send a query or a place name it writes from their
/// words to the providers `assistant.lookups` names (docs/protocol.md,
/// "Consenting to the assistant", amended 2026-10-03).
///
/// The same shape as the first: the stamp the server now holds comes back,
/// a date when granted and none when withdrawn, and granting twice keeps
/// the first date. It may only be GRANTED on top of the assistant consent —
/// `assistant_consent_required` (403) otherwise — and a server with no
/// lookup source answers 404.
pub async fn set_assistant_lookup_consent(
    token: &str,
    granted: bool,
) -> Result<Option<String>, ApiError> {
    let body = AssistantConsentRequest { granted };
    let answer: AssistantLookupConsentResponse = with_body(
        Request::post(&path("/me/assistant-lookup-consent")),
        token,
        &body,
    )
    .await?;
    Ok(answer.assistant_lookup_consent_at)
}

#[derive(Debug, Deserialize)]
struct AssistantLookupConsentResponse {
    #[serde(default)]
    assistant_lookup_consent_at: Option<String>,
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

    fn answered(status: u16, body: &str) -> Response {
        let init = web_sys::ResponseInit::new();
        init.set_status(status);
        Response::from(
            web_sys::Response::new_with_opt_str_and_init(Some(body), &init).expect("a response"),
        )
    }

    /// Both are 401; only one is a session that has ended. A wrong password
    /// signs nobody out, and is not told as an expired session.
    #[wasm_bindgen_test]
    async fn a_wrong_password_is_not_an_ended_session() {
        let wrong = answered(
            401,
            r#"{"error":{"code":"invalid_credentials","message":"invalid username or password"}}"#,
        );
        assert_eq!(
            check(&wrong).await,
            Err(ApiError::Server {
                code: "invalid_credentials".into(),
                message: "invalid username or password".into(),
            })
        );
        let gone = answered(
            401,
            r#"{"error":{"code":"unauthorized","message":"session expired"}}"#,
        );
        assert_eq!(check(&gone).await, Err(ApiError::Unauthorized));
        // A proxy's own 413 is an answer — too big — not an unreachable
        // server; and a protocol 413 keeps its own code.
        assert_eq!(
            check(&answered(413, "<html>413 Request Entity Too Large</html>")).await,
            Err(ApiError::Server {
                code: TOO_LARGE.into(),
                message: String::new(),
            })
        );
        assert_eq!(
            check(&answered(
                413,
                r#"{"error":{"code":"avatar_too_large","message":"too big"}}"#
            ))
            .await
            .unwrap_err()
            .code(),
            Some("avatar_too_large")
        );
        // A proxy's own 401, in HTML: the session, as before.
        assert_eq!(
            check(&answered(401, "<html>401</html>")).await,
            Err(ApiError::Unauthorized)
        );
    }

    fn row() -> Outgoing {
        Outgoing {
            chat_id: 42,
            client_msg_id: "8f14e45f".into(),
            body: "Dinner at 7?".into(),
            reply_to_message_id: None,
            mentions: Vec::new(),
            poll: None,
            items: Vec::new(),
            sticker: false,
            round: false,
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
            source_attachment_id: None,
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

    /// A sticker is one attachment and one flag, with an empty body — and
    /// the flag is left out of every other send, never sent as `false`.
    #[wasm_bindgen_test]
    fn a_sticker_send_carries_the_flag_and_nothing_else_does() {
        let mut sticker = row();
        sticker.body = String::new();
        sticker.sticker = true;
        sticker.reply_to_message_id = Some(1337);
        let mut picture = item("photo", -1);
        picture.attachment_id = Some(90);
        sticker.items = vec![picture];
        assert_eq!(
            encode(&sticker),
            serde_json::json!({"client_msg_id": "8f14e45f", "body": "", "attachment_ids": [90],
                               "sticker": true, "reply_to_message_id": 1337})
        );
        assert!(encode(&row()).get("sticker").is_none());
    }

    /// A video message is one video and one flag, with an empty body —
    /// a reply as well as not — and the flag is left out of every other
    /// send, never sent as `false` (docs/protocol.md, "Video messages").
    #[wasm_bindgen_test]
    fn a_video_message_send_carries_the_flag_and_nothing_else_does() {
        let mut round = row();
        round.body = String::new();
        round.round = true;
        round.reply_to_message_id = Some(41);
        let mut video = item("video", -1);
        video.attachment_id = Some(91);
        round.items = vec![video];
        assert_eq!(
            encode(&round),
            serde_json::json!({"client_msg_id": "8f14e45f", "body": "", "attachment_ids": [91],
                               "round": true, "reply_to_message_id": 41})
        );
        assert!(encode(&row()).get("round").is_none());
        round.round = false;
        assert!(encode(&round).get("round").is_none());
    }

    /// The pack's three answers, in the protocol's shapes: the whole pack
    /// with its mark, a page of changes with a tombstone in it, and a claim
    /// — with a label only when there is one.
    #[wasm_bindgen_test]
    fn the_pack_reads_and_claims_in_the_protocols_shapes() {
        let read: PackRead = serde_json::from_str(
            r#"{"items": [{"id": 5, "added_by": 7, "pack_seq": 12, "created_at": "2026-09-30T10:00:00Z",
                           "attachment": {"id": 71, "kind": "photo", "mime": "image/webp"}}],
                "max_pack_seq": 14}"#,
        )
        .expect("reads");
        assert_eq!(read.max_pack_seq, 14);
        assert!(read.items[0].is_usable());
        let changes: PackItemsResponse =
            serde_json::from_str(r#"{"items": [{"id": 5, "deleted": true, "pack_seq": 15}]}"#)
                .expect("reads");
        assert!(changes.items[0].deleted);
        assert_eq!(
            serde_json::to_value(PackClaim {
                attachment_id: 71,
                label: Some("party cat"),
            })
            .expect("encodes"),
            serde_json::json!({"attachment_id": 71, "label": "party cat"})
        );
        assert_eq!(
            serde_json::to_value(PackClaim {
                attachment_id: 71,
                label: None,
            })
            .expect("encodes"),
            serde_json::json!({"attachment_id": 71})
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

    /// A family patch sends only what changed, and the two double options
    /// clear with a null.
    #[wasm_bindgen_test]
    fn a_family_patch_sends_only_what_changed() {
        assert_eq!(
            serde_json::to_value(FamilyPatch {
                join_policy: Some("approval".into()),
                ..FamilyPatch::default()
            })
            .expect("encodes"),
            serde_json::json!({"join_policy": "approval"})
        );
        assert_eq!(
            serde_json::to_value(FamilyPatch {
                max_members: Some(None),
                language: Some(None),
                ..FamilyPatch::default()
            })
            .expect("encodes"),
            serde_json::json!({"max_members": null, "language": null})
        );
        assert_eq!(
            serde_json::to_value(FamilyPatch {
                max_members: Some(Some(8)),
                ai_vision: Some(true),
                ..FamilyPatch::default()
            })
            .expect("encodes"),
            serde_json::json!({"max_members": 8, "ai_vision": true})
        );
        assert_eq!(
            serde_json::to_value(FamilyPatch::default()).expect("encodes"),
            serde_json::json!({})
        );
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

    /// THE TRANSCRIPT CALL: the stored-bytes shape (`POST`, a JSON `{}`
    /// that is not multipart), the answer read as given — silence is `""`
    /// and an answer — and every refusal kept as the code it is.
    #[wasm_bindgen_test]
    async fn a_transcript_is_asked_for_the_stored_bytes() {
        use crate::fake_server::{Answer, FakeServer};
        const ROUTE: &str = "/chats/42/messages/1338/attachments/34/transcript";
        let answer = std::rc::Rc::new(std::cell::RefCell::new(Answer::Json(
            200,
            serde_json::json!({"transcript": {"text": "Dinner at seven", "language": "en"}}),
        )));
        let server = {
            let answer = answer.clone();
            FakeServer::answering(move |asked| {
                if asked.path == ROUTE {
                    std::mem::replace(&mut *answer.borrow_mut(), Answer::Nothing)
                } else {
                    Answer::refusal(404, "not_found")
                }
            })
        };
        let said = transcript("t", 42, 1338, 34).await.expect("an answer");
        assert_eq!(said.text, "Dinner at seven");
        assert_eq!(said.language.as_deref(), Some("en"));
        let asked = server.asked(&[ROUTE]);
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].method, "POST");
        assert!(
            asked[0].content_type.starts_with("application/json"),
            "not multipart: {}",
            asked[0].content_type
        );
        assert_eq!(asked[0].json(), serde_json::json!({}));

        // Silence, with no language named.
        *answer.borrow_mut() = Answer::Json(200, serde_json::json!({"transcript": {"text": ""}}));
        let silence = transcript("t", 42, 1338, 34).await.expect("an answer");
        assert!(silence.is_silence());
        assert_eq!(silence.language, None);

        for (status, code) in [
            (400, "transcript_refused"),
            (400, "not_transcribable"),
            (403, "transcript_not_allowed"),
            (403, "transcripts_unavailable"),
            (403, "assistant_consent_required"),
            (500, "internal"),
        ] {
            *answer.borrow_mut() = Answer::refusal(status, code);
            assert_eq!(
                transcript("t", 42, 1338, 34).await.unwrap_err().code(),
                Some(code)
            );
        }
        // No connection at all: no code, which is worth another try.
        *answer.borrow_mut() = Answer::Nothing;
        assert!(matches!(
            transcript("t", 42, 1338, 34).await,
            Err(ApiError::Network(_))
        ));
    }

    /// THE SUPPLIED-SOUND SHAPE: `multipart/form-data` with exactly one
    /// part, named `audio`, declared `audio/mp4` under an `.m4a` name, the
    /// sound byte for byte in it — and the boundary the browser chose named
    /// in the request's own `Content-Type`. A blob that came without a type
    /// goes declared all the same; the answer and every refusal are read as
    /// for the stored-bytes shape.
    #[wasm_bindgen_test]
    async fn a_transcript_of_supplied_sound_is_one_audio_part() {
        use crate::fake_server::{Answer, FakeServer};
        const ROUTE: &str = "/chats/42/messages/1338/attachments/36/transcript";
        let answer = std::rc::Rc::new(std::cell::RefCell::new(Answer::Json(
            200,
            serde_json::json!({"transcript": {"text": "Look at the snow"}}),
        )));
        let server = {
            let answer = answer.clone();
            FakeServer::answering(move |asked| {
                if asked.path == ROUTE {
                    std::mem::replace(&mut *answer.borrow_mut(), Answer::Nothing)
                } else {
                    Answer::refusal(404, "not_found")
                }
            })
        };
        let sound: Vec<u8> = b"\0\0\0\x18ftypM4A \0\0\0\0M4A mp42"
            .iter()
            .copied()
            .chain((0..=255u8).cycle().take(3_000))
            .collect();
        let find = |haystack: &[u8], needle: &[u8]| {
            haystack
                .windows(needle.len())
                .filter(|window| *window == needle)
                .count()
        };
        for declared in ["audio/mp4", ""] {
            let options = web_sys::BlobPropertyBag::new();
            options.set_type(declared);
            let blob = Blob::new_with_u8_array_sequence_and_options(
                &js_sys::Array::of1(&js_sys::Uint8Array::from(sound.as_slice())),
                &options,
            )
            .unwrap();
            *answer.borrow_mut() = Answer::Json(
                200,
                serde_json::json!({"transcript": {"text": "Look at the snow"}}),
            );
            let said = transcript_of_sound("t", 42, 1338, 36, &blob)
                .await
                .expect("an answer");
            assert_eq!(said.text, "Look at the snow");
            assert_eq!(said.language, None);
            let asked = server.asked(&[ROUTE]).pop().unwrap();
            assert_eq!(asked.method, "POST");
            let boundary = asked
                .content_type
                .strip_prefix("multipart/form-data; boundary=")
                .unwrap_or_else(|| panic!("multipart: {}", asked.content_type))
                .to_string();
            let body = &asked.body;
            let opening = format!("--{boundary}\r\n");
            let closing = format!("--{boundary}--");
            assert_eq!(find(body, opening.as_bytes()), 1, "exactly one part");
            assert_eq!(find(body, closing.as_bytes()), 1);
            assert_eq!(
                find(
                    body,
                    b"Content-Disposition: form-data; name=\"audio\"; filename=\"sound.m4a\"\r\n"
                ),
                1
            );
            assert_eq!(
                find(body, b"Content-Type: audio/mp4\r\n\r\n"),
                1,
                "{declared:?}"
            );
            assert_eq!(find(body, &sound), 1, "the sound, byte for byte");
        }
        assert_eq!(server.asked(&[ROUTE]).len(), 2);

        for (status, code) in [
            (400, "not_transcribable"),
            (400, "validation"),
            (403, "assistant_consent_required"),
            (500, "internal"),
        ] {
            *answer.borrow_mut() = Answer::refusal(status, code);
            let blob = Blob::new_with_u8_array_sequence(&js_sys::Array::of1(
                &js_sys::Uint8Array::from(sound.as_slice()),
            ))
            .unwrap();
            assert_eq!(
                transcript_of_sound("t", 42, 1338, 36, &blob)
                    .await
                    .unwrap_err()
                    .code(),
                Some(code)
            );
        }
    }

    /// Its own deadline, of at least the protocol's 90 s and never the
    /// ten seconds a message gets — and past it the request is given up
    /// as a failure to try again, not left hanging.
    #[wasm_bindgen_test]
    async fn a_transcript_has_a_deadline_of_its_own() {
        use crate::fake_server::{Answer, FakeServer};
        const ROUTE: &str = "/chats/42/messages/1338/attachments/35/transcript";
        const { assert!(TRANSCRIPT_DEADLINE_MS >= 90_000) };
        assert_ne!(TRANSCRIPT_DEADLINE_MS, SEND_DEADLINE_MS);
        let _server = FakeServer::answering(|asked| {
            if asked.path == ROUTE {
                Answer::Hang
            } else {
                Answer::refusal(404, "not_found")
            }
        });
        let gave_up = transcript_within("t", 42, 1338, 35, None, 50).await;
        assert_eq!(
            gave_up,
            Err(ApiError::Network(
                "The server did not answer in time.".to_string()
            ))
        );
        assert_eq!(
            fc_text::transcript::after_refusal(gave_up.unwrap_err().code()),
            fc_text::transcript::Next::Fail(fc_text::transcript::Failure::TryAgain)
        );
    }

    /// THE LOOKUP CONSENT, against a stand-in server: one POST to its own
    /// route with exactly `{"granted": …}`, the server's stamp handed back
    /// (a date granted, none withdrawn), and each refusal the protocol names
    /// kept as its code — the first consent's route is never touched.
    #[wasm_bindgen_test]
    async fn the_lookup_consent_is_its_own_request() {
        use crate::fake_server::{Answer, FakeServer};
        const ROUTE: &str = "/me/assistant-lookup-consent";
        let answer = std::rc::Rc::new(std::cell::RefCell::new(Answer::Nothing));
        let server = {
            let answer = answer.clone();
            FakeServer::answering(move |asked| {
                if asked.path == ROUTE {
                    std::mem::replace(&mut *answer.borrow_mut(), Answer::Nothing)
                } else {
                    Answer::refusal(404, "not_found")
                }
            })
        };
        *answer.borrow_mut() = Answer::Json(
            200,
            serde_json::json!({"assistant_lookup_consent_at": "2026-10-03T09:30:00Z"}),
        );
        assert_eq!(
            set_assistant_lookup_consent("t", true).await,
            Ok(Some("2026-10-03T09:30:00Z".to_string()))
        );
        *answer.borrow_mut() = Answer::Json(
            200,
            serde_json::json!({"assistant_lookup_consent_at": null}),
        );
        assert_eq!(set_assistant_lookup_consent("t", false).await, Ok(None));
        let asked = server.asked(&[ROUTE, "/me/assistant-consent"]);
        assert_eq!(asked.len(), 2);
        assert!(asked.iter().all(|asked| asked.path == ROUTE));
        assert!(asked.iter().all(|asked| asked.method == "POST"));
        assert_eq!(asked[0].json(), serde_json::json!({"granted": true}));
        assert_eq!(asked[1].json(), serde_json::json!({"granted": false}));

        for (status, code) in [
            (403, "assistant_consent_required"),
            (404, "not_found"),
            (400, "validation"),
        ] {
            *answer.borrow_mut() = Answer::refusal(status, code);
            assert_eq!(
                set_assistant_lookup_consent("t", true)
                    .await
                    .unwrap_err()
                    .code(),
                Some(code)
            );
        }
    }

    /// The owner's switch goes as its one key, and only when it changed.
    #[wasm_bindgen_test]
    fn the_lookups_switch_is_one_key_of_the_patch() {
        let patch = FamilyPatch {
            ai_lookups: Some(true),
            ..FamilyPatch::default()
        };
        assert_eq!(
            serde_json::to_value(&patch).unwrap(),
            serde_json::json!({"ai_lookups": true})
        );
        assert_eq!(
            serde_json::to_value(FamilyPatch::default()).unwrap(),
            serde_json::json!({}),
            "absent, never false, when the owner did not touch it"
        );
    }

    /// The greeting's places go as the WHOLE list under their one key:
    /// `[]` to clear — never a null, which the server refuses — and absent
    /// when the owner did not touch them.
    #[wasm_bindgen_test]
    fn the_greeting_places_go_as_the_whole_list_and_never_as_null() {
        let patch = FamilyPatch {
            greeting_places: Some(vec!["Moscow".into(), "Belgrade".into()]),
            ..FamilyPatch::default()
        };
        assert_eq!(
            serde_json::to_string(&patch).unwrap(),
            r#"{"greeting_places":["Moscow","Belgrade"]}"#
        );
        let cleared = FamilyPatch {
            greeting_places: Some(Vec::new()),
            ..FamilyPatch::default()
        };
        assert_eq!(
            serde_json::to_string(&cleared).unwrap(),
            r#"{"greeting_places":[]}"#
        );
        let other = FamilyPatch {
            ai_greeting: Some(true),
            ..FamilyPatch::default()
        };
        assert_eq!(
            serde_json::to_string(&other).unwrap(),
            r#"{"ai_greeting":true}"#,
            "absent, never null, when the places were not touched"
        );
    }
}
