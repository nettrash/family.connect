//! The REST half of the protocol: JSON over HTTP under `{base}/api/v1` (docs/protocol.md,
//! "Transport") — a port of `win/src/FamilyConnect.Core/Protocol/ApiClient.cs`.
//!
//! The transport rules are the interesting part, and they are all here rather than at each call
//! site: the bearer token on every request, the error shape decoded into [`ApiError`] with its
//! status and `Retry-After`, and ONE retry on a GET that failed transiently — a 429, a 5xx, or a
//! request that never arrived at all. A read is safe to repeat, and the alternative is a chat that
//! empties itself because a proxy hiccuped or a pooled connection went stale while the laptop
//! slept. A write is never retried here; the send outbox owns that, because only it knows whether
//! the message has a dedup key ([`super::send_rules`]).
//!
//! Every request runs under a deadline of its OWN, kept here on the [`Clock`] and not by the
//! transport, so that the slow endpoints — a backdrop, a transcript — can be given longer than an
//! ordinary call (the C# kept them per request over an `HttpClient` with no timeout of its own).
//! The deadline is also handed to the transport as [`HttpRequest::timeout`]; whichever ends the
//! request first, the answer is the same transport failure.
//!
//! Cancelling a call is dropping its future.
//!
//! Apple counterpart: `APIClient`. Android: `ApiClient` + the per-area Api interfaces.

use std::borrow::Cow;
use std::cell::Cell;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::rc::Rc;
use std::time::Duration;

use futures_util::future::{Either, select};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use time::{Date, Month, OffsetDateTime, PrimitiveDateTime, Time, UtcOffset, Weekday};

use super::api_error::{ApiError, error_codes};
use super::api_result::{ApiResult, Nothing, TokenStore};
use super::dtos::{ErrorEnvelope, wire};
use super::frames::client_frames::SendOptions;
use super::requests::{
    AssistantConsentRequest, AssistantConsentResponse, AssistantLookupConsentResponse,
    AssistantReportResponse, AttachmentResponse, AuthResponse, BoardChangesResponse, BoardResponse,
    ChatResponse, ChatsResponse, FamilyOnlyResponse, FamilyPatch, FamilyResponse,
    GroupMemberAnswer, IceServersResponse, InvitationResponse, InvitationsResponse,
    InviteCodeResponse, JoinAnswer, JoinRequestsResponse, LeftAnswer, MeResponse, MemberResponse,
    MessagePollDto, MessageReactionsDto, MessageResponse, MessagesResponse, NotePatch, NoteRequest,
    NoteResponse, PackChangesResponse, PackItemResponse, PackResponse, PollRequest, PollsResponse,
    ReactionsResponse, ReportResponse, ReportsResponse, SendRequest, StatsResponse,
    TranscriptResponse, UserResponse,
};
use super::send_rules::RETRY_AFTER_CAP;
use super::server_url::ServerUrl;
use super::upload_queries::{self, escape_data_string};
use crate::rt::{Clock, Http, HttpRequest, HttpResponse, Method, TransportError};

/// How long an ordinary request may take: the 100 s `HttpClient` always gave every call, applied
/// per request so that one request can be given a longer one.
pub const ORDINARY_TIMEOUT: Duration = Duration::from_secs(100);

/// `POST …/board/notes/{id}/backdrop`'s OWN deadline: the server may make up to three provider
/// calls in a row, so a client gives it "a timeout of its own, no shorter than 90 s … never its
/// ordinary request timeout" — and a request whose connection closes first draws nothing
/// (docs/protocol.md, "Board", amended 2026-09-30). The Apple client's 120 s.
pub const BACKDROP_TIMEOUT: Duration = Duration::from_secs(120);

/// `POST …/attachments/{id}/transcript`'s OWN deadline: one provider call bounded by the server's
/// 180 s, and "a timeout of its OWN, no shorter than 90 s, never its ordinary request timeout"
/// (docs/protocol.md, "Transcripts on request"). It covers the WHOLE request, so it must also
/// cover a supplied sound of up to 25 MiB going up before the provider's own wait begins; the
/// reference proxy waits 300 s on this route, and this waits a little past it, as iOS and Android
/// do (310 s) — the server, or the proxy answering for it, ends the wait. Shorter, a supplied
/// request that is merely slow fails on every try: the server drops that call with the connection
/// and keeps nothing. A request for the stored bytes that runs out is still finished and KEPT on
/// the server, so asking again a little later answers at once.
pub const TRANSCRIPT_TIMEOUT: Duration = Duration::from_secs(310);

/// The header a client that knows about several families sends on EVERY request
/// (docs/protocol.md, "Several families").
pub const FAMILIES_HEADER: &str = "X-FC-Families";

/// How long a read waits before its one retry when the server named no wait of its own.
const RETRY_PAUSE: Duration = Duration::from_millis(250);

/// What a request carries.
#[derive(Debug, Clone)]
enum Payload {
    Empty,
    Json(String),
    Raw {
        content_type: String,
        bytes: Vec<u8>,
    },
}

/// The facts an upload carries in its query, besides its kind (docs/protocol.md, "Photos, videos,
/// audio, files and locations") — `Upload`'s optional parameters in `win/`. All absent by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UploadFacts<'a> {
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub duration_ms: Option<i32>,
    /// A file's name; an empty one is no name.
    pub name: Option<&'a str>,
    /// A voice note's shape, sent only on `audio` and only when it is exactly the wire.
    pub waveform: Option<&'a str>,
    /// The family the upload is FOR; `None` is the family shown, else the home family.
    pub family_id: Option<i64>,
}

/// The REST client. One per signed-in server; single-threaded, like everything here — share it as
/// an `Rc`.
pub struct ApiClient {
    http: Rc<dyn Http>,
    clock: Rc<dyn Clock>,
    tokens: Rc<dyn TokenStore>,
    base_url: ServerUrl,
    rest: String,
    request_deadline: Duration,
    backdrop_deadline: Duration,
    transcript_deadline: Duration,
    /// The family shown, or 0 for none.
    scope: Cell<i64>,
}

impl std::fmt::Debug for ApiClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // No token, and no URL with a path: the family's logging rule.
        f.debug_struct("ApiClient")
            .field("base_url", &self.base_url.to_string())
            .field("family_scope", &self.family_scope())
            .finish_non_exhaustive()
    }
}

impl ApiClient {
    /// `new ApiClient(http, baseUrl, tokens)`, with the clock its deadlines and waits run on.
    pub fn new(
        http: Rc<dyn Http>,
        base_url: ServerUrl,
        tokens: Rc<dyn TokenStore>,
        clock: Rc<dyn Clock>,
    ) -> Self {
        let rest = base_url.rest();
        ApiClient {
            http,
            clock,
            tokens,
            base_url,
            rest,
            request_deadline: ORDINARY_TIMEOUT,
            backdrop_deadline: BACKDROP_TIMEOUT,
            transcript_deadline: TRANSCRIPT_TIMEOUT,
            scope: Cell::new(0),
        }
    }

    /// The deadline this instance gives an ordinary request ([`ORDINARY_TIMEOUT`]; tests shorten
    /// it).
    pub fn with_request_deadline(mut self, deadline: Duration) -> Self {
        self.request_deadline = deadline;
        self
    }

    /// The deadline this instance gives a backdrop ([`BACKDROP_TIMEOUT`]; tests shorten it).
    pub fn with_backdrop_deadline(mut self, deadline: Duration) -> Self {
        self.backdrop_deadline = deadline;
        self
    }

    /// The deadline this instance gives a transcript ([`TRANSCRIPT_TIMEOUT`]; tests shorten it).
    pub fn with_transcript_deadline(mut self, deadline: Duration) -> Self {
        self.transcript_deadline = deadline;
        self
    }

    pub fn request_deadline(&self) -> Duration {
        self.request_deadline
    }

    pub fn backdrop_deadline(&self) -> Duration {
        self.backdrop_deadline
    }

    pub fn transcript_deadline(&self) -> Duration {
        self.transcript_deadline
    }

    /// The server this client talks to.
    pub fn base_url(&self) -> &ServerUrl {
        &self.base_url
    }

    /// The socket URL for the same server (docs/protocol.md, "WebSocket protocol").
    pub fn socket_url(&self) -> String {
        self.base_url.socket()
    }

    /// The family this app is SHOWING (docs/protocol.md, "Several families"): what every family
    /// endpoint below names when it is not given one — `/families/{id}/…` — and what an upload
    /// and a new group are for. `None` names the home family (`mine`), which is all an account in
    /// one family, or a server from before several families, has.
    ///
    /// Read once per request — so a request already on the wire when the family switches finishes
    /// for the family it was asked of. An id that is not positive reads as none, as in `win/`.
    pub fn family_scope(&self) -> Option<i64> {
        let held = self.scope.get();
        (held > 0).then_some(held)
    }

    /// Set by the session whenever the current family changes.
    pub fn set_family_scope(&self, family: Option<i64>) {
        self.scope.set(family.unwrap_or(0));
    }

    /// `/families/{id}` for the family named, else the one shown, else `/families/mine`.
    fn families(&self, family: Option<i64>) -> String {
        match family.or(self.family_scope()) {
            Some(id) => format!("/families/{id}"),
            None => "/families/mine".to_string(),
        }
    }

    /// The forms without an id — `/families/leave`, `/families/members/…`,
    /// `/families/join-requests` — mean the HOME family, so the one being shown is named whenever
    /// there is one: `/families/{id}`, else plain `/families`.
    fn family_root(&self, family: Option<i64>) -> String {
        match family.or(self.family_scope()) {
            Some(id) => format!("/families/{id}"),
            None => "/families".to_string(),
        }
    }

    // ---- auth ------------------------------------------------------------------------------

    pub async fn register(
        &self,
        username: &str,
        display_name: &str,
        password: &str,
    ) -> ApiResult<AuthResponse> {
        let body = object(&[
            ("username", Some(json!(username))),
            ("display_name", Some(json!(display_name))),
            ("password", Some(json!(password))),
        ]);
        self.send_json(Method::Post, "/auth/register", Payload::Json(body))
            .await
    }

    pub async fn log_in(&self, username: &str, password: &str) -> ApiResult<AuthResponse> {
        let body = object(&[
            ("username", Some(json!(username))),
            ("password", Some(json!(password))),
        ]);
        self.send_json(Method::Post, "/auth/login", Payload::Json(body))
            .await
    }

    pub async fn log_out(&self) -> ApiResult<Nothing> {
        self.send_nothing(Method::Post, "/auth/logout", Payload::Empty)
            .await
    }

    pub async fn me(&self) -> ApiResult<MeResponse> {
        self.send_json(Method::Get, "/me", Payload::Empty).await
    }

    /// This member's own permission for their words to go to the model, and nobody else's
    /// (docs/protocol.md, "Consenting to the assistant"). Answers with the stamp the server now
    /// holds: a date when granted, `None` when withdrawn.
    ///
    /// Idempotent both ways — granting twice keeps the FIRST date, because when somebody agreed is
    /// a fact and not a counter. A server with no assistant answers `404` rather than admitting
    /// there is nothing to consent to.
    pub async fn set_assistant_consent(
        &self,
        granted: bool,
    ) -> ApiResult<AssistantConsentResponse> {
        let body = wire::encode(&AssistantConsentRequest { granted });
        self.send_json(Method::Post, "/me/assistant-consent", Payload::Json(body))
            .await
    }

    /// This member's own permission for the assistant to send a query or place name it writes
    /// from their words to the lookup providers (docs/protocol.md, "Consenting to the assistant",
    /// amended 2026-10-03). The same shape as [`set_assistant_consent`](Self::set_assistant_consent)
    /// and idempotent the same way. It may only be GRANTED on top of the assistant consent —
    /// `assistant_consent_required` otherwise — and withdrawing the assistant consent clears it on
    /// the server too. A server with no lookup source answers `404`.
    pub async fn set_assistant_lookup_consent(
        &self,
        granted: bool,
    ) -> ApiResult<AssistantLookupConsentResponse> {
        let body = wire::encode(&AssistantConsentRequest { granted });
        self.send_json(
            Method::Post,
            "/me/assistant-lookup-consent",
            Payload::Json(body),
        )
        .await
    }

    /// The family's own document: the one named, else the one shown, else the home family. A
    /// resync names EACH family the account is in.
    pub async fn family(&self, family: Option<i64>) -> ApiResult<FamilyResponse> {
        self.send_json(Method::Get, &self.families(family), Payload::Empty)
            .await
    }

    // ---- chats and messages ------------------------------------------------------------------

    pub async fn chats(&self) -> ApiResult<ChatsResponse> {
        self.send_json(Method::Get, "/chats", Payload::Empty).await
    }

    /// One page of a chat, newest first — the paging the chat view walks backwards.
    pub async fn messages(
        &self,
        chat_id: i64,
        before_id: Option<i64>,
        limit: Option<i32>,
    ) -> ApiResult<MessagesResponse> {
        let mut query = Vec::new();
        if let Some(before) = before_id {
            query.push(format!("before_id={before}"));
        }
        if let Some(many) = limit {
            query.push(format!("limit={many}"));
        }
        let path = if query.is_empty() {
            format!("/chats/{chat_id}/messages")
        } else {
            format!("/chats/{chat_id}/messages?{}", query.join("&"))
        };
        self.send_json(Method::Get, &path, Payload::Empty).await
    }

    /// Send over REST — what the outbox falls back to when a frame went unanswered. The
    /// `client_msg_id` is the DEDUP KEY: the same id sent twice is one message, which is what
    /// makes a repeat safe (docs/protocol.md, "Sending on an unreliable network"). The options are
    /// the `send` frame's own, so the two paths cannot drift apart.
    pub async fn send_message(
        &self,
        chat_id: i64,
        client_msg_id: &str,
        body: &str,
        options: &SendOptions<'_>,
    ) -> ApiResult<MessageResponse> {
        let request = SendRequest {
            client_msg_id: client_msg_id.to_string(),
            body: body.to_string(),
            reply_to_message_id: options.reply_to_message_id,
            attachment_ids: (!options.attachment_ids.is_empty())
                .then(|| options.attachment_ids.to_vec()),
            poll: (!options.poll_options.is_empty()).then(|| PollRequest {
                options: options.poll_options.to_vec(),
            }),
            mentions: (!options.mentions.is_empty()).then(|| options.mentions.to_vec()),
            // Present only when true: absent is an ordinary message, to every server there is.
            sticker: options.sticker.then_some(true),
            // The same for a video message: absent unless it is one.
            round: options.round.then_some(true),
            // And for an animation.
            animation: options.animation.then_some(true),
        };
        self.send_json(
            Method::Post,
            &format!("/chats/{chat_id}/messages"),
            Payload::Json(wire::encode(&request)),
        )
        .await
    }

    /// The reconnect catch-up: strictly newer, OLDEST FIRST — the opposite direction to a history
    /// page, and looped until a short one (docs/protocol.md, "Best-effort delivery"). `win/`'s
    /// default `limit` was 50.
    pub async fn messages_after(
        &self,
        chat_id: i64,
        after_id: i64,
        limit: i32,
    ) -> ApiResult<MessagesResponse> {
        self.send_json(
            Method::Get,
            &format!("/chats/{chat_id}/messages?after_id={after_id}&limit={limit}"),
            Payload::Empty,
        )
        .await
    }

    /// The STUN and TURN servers for a call — fetched at the start of every call, because a stale
    /// credential is a call that silently cannot relay (docs/protocol.md, "Where the servers come
    /// from"). `calls_disabled` when the server has none.
    pub async fn ice_servers(&self) -> ApiResult<IceServersResponse> {
        self.send_json(Method::Get, "/calls/ice", Payload::Empty)
            .await
    }

    /// One chain: the root first, then every reply, oldest first — named by the root OR any reply
    /// in it, so a client holding only a reply learns its root. `after_id` pages it, looped until
    /// a short page. A plain read and never a cursor (docs/protocol.md, "Threads").
    pub async fn thread(
        &self,
        chat_id: i64,
        message_id: i64,
        after_id: Option<i64>,
        limit: i32,
    ) -> ApiResult<MessagesResponse> {
        let after = after_id.map_or(String::new(), |after| format!("&after_id={after}"));
        self.send_json(
            Method::Get,
            &format!("/chats/{chat_id}/messages/{message_id}/thread?limit={limit}{after}"),
            Payload::Empty,
        )
        .await
    }

    /// The reaction catch-up, by its own sequence. `after_id` is `WHERE id > cursor` and can never
    /// see a change to an OLDER row, which is why reactions have a sequence of their own.
    pub async fn reactions_after(
        &self,
        chat_id: i64,
        after_seq: i64,
        limit: i32,
    ) -> ApiResult<ReactionsResponse> {
        self.send_json(
            Method::Get,
            &format!("/chats/{chat_id}/reactions?after_seq={after_seq}&limit={limit}"),
            Payload::Empty,
        )
        .await
    }

    /// The poll catch-up, the same shape one sequence over.
    pub async fn polls_after(
        &self,
        chat_id: i64,
        after_seq: i64,
        limit: i32,
    ) -> ApiResult<PollsResponse> {
        self.send_json(
            Method::Get,
            &format!("/chats/{chat_id}/polls?after_seq={after_seq}&limit={limit}"),
            Payload::Empty,
        )
        .await
    }

    /// Set the caller's reaction. A STATE-SET, not a toggle: the client decides whether a tap
    /// means set or remove, and the answer is the message's whole list.
    pub async fn react(
        &self,
        chat_id: i64,
        message_id: i64,
        emoji: &str,
    ) -> ApiResult<MessageReactionsDto> {
        self.send_json(
            Method::Put,
            &format!("/chats/{chat_id}/messages/{message_id}/reaction"),
            Payload::Json(object(&[("emoji", Some(json!(emoji)))])),
        )
        .await
    }

    /// Take the caller's reaction off. Idempotent: removing nothing answers the list unchanged.
    pub async fn unreact(&self, chat_id: i64, message_id: i64) -> ApiResult<MessageReactionsDto> {
        self.send_json(
            Method::Delete,
            &format!("/chats/{chat_id}/messages/{message_id}/reaction"),
            Payload::Empty,
        )
        .await
    }

    /// Hold one option of a poll. A state-set, not a toggle: re-PUTting the option already held
    /// is a no-op on the server, so what a tap means is the caller's to decide (docs/protocol.md,
    /// "Polls").
    pub async fn vote(
        &self,
        chat_id: i64,
        message_id: i64,
        option_id: i64,
    ) -> ApiResult<MessagePollDto> {
        self.send_json(
            Method::Put,
            &format!("/chats/{chat_id}/messages/{message_id}/vote"),
            Payload::Json(object(&[("option_id", Some(json!(option_id)))])),
        )
        .await
    }

    /// Retract the caller's vote. Idempotent: retracting nothing answers the poll unchanged.
    pub async fn unvote(&self, chat_id: i64, message_id: i64) -> ApiResult<MessagePollDto> {
        self.send_json(
            Method::Delete,
            &format!("/chats/{chat_id}/messages/{message_id}/vote"),
            Payload::Empty,
        )
        .await
    }

    /// Close a poll: the author's, and one-way. Closing a closed poll is a no-op.
    pub async fn close_poll(&self, chat_id: i64, message_id: i64) -> ApiResult<MessagePollDto> {
        self.send_json(
            Method::Post,
            &format!("/chats/{chat_id}/messages/{message_id}/poll/close"),
            Payload::Empty,
        )
        .await
    }

    /// A chat's OPEN polls as they stand, each as the whole message that carries it, oldest first.
    /// A plain read and not a cursor: it moves nothing, and the feed of changes stays
    /// [`polls_after`](Self::polls_after) (docs/protocol.md, "Finding the open ones").
    pub async fn open_polls(&self, chat_id: i64) -> ApiResult<MessagesResponse> {
        self.send_json(
            Method::Get,
            &format!("/chats/{chat_id}/polls/open"),
            Payload::Empty,
        )
        .await
    }

    /// Replace the body of the caller's own message. The send rules apply (trimmed, non-empty, at
    /// most 4000 characters), and re-sending the body it already has changes nothing.
    pub async fn edit_message(
        &self,
        chat_id: i64,
        message_id: i64,
        body: &str,
    ) -> ApiResult<MessageResponse> {
        self.send_json(
            Method::Patch,
            &format!("/chats/{chat_id}/messages/{message_id}"),
            Payload::Json(object(&[("body", Some(json!(body)))])),
        )
        .await
    }

    /// The edit catch-up, which answers whole messages rather than a bespoke patch — so a client
    /// applies them through exactly the same path as a page of history.
    pub async fn edits_after(
        &self,
        chat_id: i64,
        after_seq: i64,
        limit: i32,
    ) -> ApiResult<MessagesResponse> {
        self.send_json(
            Method::Get,
            &format!("/chats/{chat_id}/edits?after_seq={after_seq}&limit={limit}"),
            Payload::Empty,
        )
        .await
    }

    /// The text of a voice note or an audio file, for the CALLER only (docs/protocol.md,
    /// "Transcripts on request"). NO REQUEST BODY: that is the form that sends the server's
    /// STORED copy, whose answer the server keeps and hands to whoever the rule lets ask next. The
    /// supplied-sound form is [`transcript_of_sound`](Self::transcript_of_sound).
    ///
    /// SLOW: it runs under the transcript deadline, never the ordinary one. A write, so never
    /// retried here — a transient failure is the member's to retry, and the server will have kept
    /// the answer by then.
    pub async fn transcript(
        &self,
        chat_id: i64,
        message_id: i64,
        attachment_id: i64,
    ) -> ApiResult<TranscriptResponse> {
        let path = format!(
            "/chats/{chat_id}/messages/{message_id}/attachments/{attachment_id}/transcript"
        );
        let answer = self
            .exchange(
                Method::Post,
                &path,
                Payload::Empty,
                self.transcript_deadline,
            )
            .await;
        read_json(answer)
    }

    /// The text of a recording from sound THIS DEVICE made — a video's sound track, an Ogg file's,
    /// an audio file's over the ceiling — sent as `multipart/form-data` with one part named
    /// `audio`: an M4A of AAC, part type `audio/mp4`, at most `transcribe_max_bytes` (the caller
    /// holds it to that). The server never keeps this answer: it is returned to the caller and
    /// nowhere else.
    ///
    /// Under the transcript deadline, and never retried: unlike the stored form, a supplied
    /// request is dropped when its connection closes, so asking again is the member's to decide.
    pub async fn transcript_of_sound(
        &self,
        chat_id: i64,
        message_id: i64,
        attachment_id: i64,
        sound: Vec<u8>,
    ) -> ApiResult<TranscriptResponse> {
        let path = format!(
            "/chats/{chat_id}/messages/{message_id}/attachments/{attachment_id}/transcript"
        );
        let (content_type, bytes) = form_data("audio", "audio.m4a", "audio/mp4", &sound);
        let answer = self
            .exchange(
                Method::Post,
                &path,
                Payload::Raw {
                    content_type,
                    bytes,
                },
                self.transcript_deadline,
            )
            .await;
        read_json(answer)
    }

    pub async fn mark_read(&self, chat_id: i64, last_read_message_id: i64) -> ApiResult<Nothing> {
        self.send_nothing(
            Method::Post,
            &format!("/chats/{chat_id}/read"),
            Payload::Json(object(&[(
                "last_read_message_id",
                Some(json!(last_read_message_id)),
            )])),
        )
        .await
    }

    // ---- the board ---------------------------------------------------------------------------

    /// One family's whole wall: the one named, else the one shown, else the home family.
    pub async fn board(&self, family: Option<i64>) -> ApiResult<BoardResponse> {
        self.send_json(
            Method::Get,
            &format!("{}/board", self.families(family)),
            Payload::Empty,
        )
        .await
    }

    /// The catch-up, looped until a short page (docs/protocol.md, "Board") — each family has its
    /// own cursor, so a resync names the family. `win/`'s default `limit` was 50.
    pub async fn board_changes(
        &self,
        family: Option<i64>,
        after_seq: i64,
        limit: i32,
    ) -> ApiResult<BoardChangesResponse> {
        self.send_json(
            Method::Get,
            &format!(
                "{}/board/changes?after_seq={after_seq}&limit={limit}",
                self.families(family)
            ),
            Payload::Empty,
        )
        .await
    }

    pub async fn create_note(&self, note: &NoteRequest) -> ApiResult<NoteResponse> {
        self.send_json(
            Method::Post,
            &format!("{}/board/notes", self.families(None)),
            Payload::Json(wire::encode(note)),
        )
        .await
    }

    /// An edit: absent means "leave alone", and an event's end taken OFF is the `null` the
    /// protocol says clears it ([`NotePatch`] writes itself that way).
    pub async fn patch_note(&self, note_id: i64, patch: &NotePatch) -> ApiResult<NoteResponse> {
        self.send_json(
            Method::Patch,
            &format!("{}/board/notes/{note_id}", self.families(None)),
            Payload::Json(wire::encode(patch)),
        )
        .await
    }

    /// An idempotent state-set, not a toggle; ANY member may send it.
    pub async fn answer(&self, note_id: i64, answer: &str) -> ApiResult<NoteResponse> {
        self.send_json(
            Method::Put,
            &format!("{}/board/notes/{note_id}/rsvp", self.families(None)),
            Payload::Json(object(&[("answer", Some(json!(answer)))])),
        )
        .await
    }

    pub async fn retract_answer(&self, note_id: i64) -> ApiResult<NoteResponse> {
        self.send_json(
            Method::Delete,
            &format!("{}/board/notes/{note_id}/rsvp", self.families(None)),
            Payload::Empty,
        )
        .await
    }

    /// The same shape one field over: a state, and the server records who.
    pub async fn tick_task(
        &self,
        note_id: i64,
        item_id: i64,
        done: bool,
    ) -> ApiResult<NoteResponse> {
        self.send_json(
            Method::Put,
            &format!(
                "{}/board/notes/{note_id}/tasks/{item_id}",
                self.families(None)
            ),
            Payload::Json(object(&[("done", Some(json!(done)))])),
        )
        .await
    }

    /// Ask the assistant for an event's backdrop. NO REQUEST BODY: the prompt is the note's title
    /// and nothing else (docs/protocol.md, "Board").
    ///
    /// SLOW: it runs under the backdrop deadline, never the ordinary one.
    pub async fn draw_backdrop(&self, note_id: i64) -> ApiResult<NoteResponse> {
        let path = format!("{}/board/notes/{note_id}/backdrop", self.families(None));
        let answer = self
            .exchange(Method::Post, &path, Payload::Empty, self.backdrop_deadline)
            .await;
        read_json(answer)
    }

    pub async fn delete_note(&self, note_id: i64) -> ApiResult<Nothing> {
        self.send_nothing(
            Method::Delete,
            &format!("{}/board/notes/{note_id}", self.families(None)),
            Payload::Empty,
        )
        .await
    }

    // ---- the sticker pack --------------------------------------------------------------------

    /// The WHOLE pack as it now stands, in the order its items were added — a full read, which
    /// REPLACES what a client holds (docs/protocol.md, "Sticker pack"). Not paged: a pack is at
    /// most `max_pack_items`. The family named, else the one shown, else the home family.
    pub async fn pack(&self, family: Option<i64>) -> ApiResult<PackResponse> {
        self.send_json(
            Method::Get,
            &format!("{}/pack", self.families(family)),
            Payload::Empty,
        )
        .await
    }

    /// The pack's catch-up, tombstones included, looped until a short page. `win/`'s default
    /// `limit` was 50.
    pub async fn pack_changes(
        &self,
        family: Option<i64>,
        after_seq: i64,
        limit: i32,
    ) -> ApiResult<PackChangesResponse> {
        self.send_json(
            Method::Get,
            &format!(
                "{}/pack/changes?after_seq={after_seq}&limit={limit}",
                self.families(family)
            ),
            Payload::Empty,
        )
        .await
    }

    /// Claim an upload as a pack item — the third way an attachment is claimed. ANY member may.
    ///
    /// `201` and `200` both answer an item, and both are success: a `200` is the pack ALREADY
    /// holding it — a retry of the same id, or bytes it has under another id, in which case the
    /// item's attachment id is NOT the one sent. Adding the same sticker twice is not an error and
    /// not two stickers. WHICH of the two it was is [`ApiResult::status`], and nothing else says
    /// it: the actor's own `pack_item` frame can land before this answer does, so "did this
    /// device already hold the item" is not the question.
    pub async fn add_to_pack(
        &self,
        attachment_id: i64,
        label: Option<&str>,
    ) -> ApiResult<PackItemResponse> {
        // An empty label is no label, and no label is no key. `str::trim` and .NET's
        // `string.IsNullOrWhiteSpace`/`Trim` agree on what white space is.
        let label = label.map(str::trim).filter(|label| !label.is_empty());
        self.send_json(
            Method::Post,
            &format!("{}/pack", self.families(None)),
            Payload::Json(object(&[
                ("attachment_id", Some(json!(attachment_id))),
                ("label", label.map(|label| json!(label))),
            ])),
        )
        .await
    }

    /// Take an item out: whoever added it, or the family's owner. Idempotent — removing one
    /// already removed is `204` too — and nothing ever sent with that sticker is touched.
    pub async fn remove_from_pack(&self, item_id: i64) -> ApiResult<Nothing> {
        self.send_nothing(
            Method::Delete,
            &format!("{}/pack/{item_id}", self.families(None)),
            Payload::Empty,
        )
        .await
    }

    // ---- this account ------------------------------------------------------------------------

    /// Change your own password, which requires proving you know the current one — A LIVE SESSION
    /// IS NOT PROOF, because an unattended unlocked machine is exactly what this defends against.
    ///
    /// The refusal is `invalid_credentials`, and it means "that is not your password" and NOT
    /// "your session has expired": it must never sign anybody out ([`ApiError::session_gone`]).
    pub async fn change_password(
        &self,
        current_password: &str,
        new_password: &str,
    ) -> ApiResult<Nothing> {
        self.send_nothing(
            Method::Post,
            "/me/password",
            Payload::Json(object(&[
                ("current_password", Some(json!(current_password))),
                ("new_password", Some(json!(new_password))),
            ])),
        )
        .await
    }

    /// Delete this account, permanently. The password is required for the same reason the change
    /// above requires it.
    pub async fn delete_account(&self, password: &str) -> ApiResult<Nothing> {
        self.send_nothing(
            Method::Post,
            "/me/delete",
            Payload::Json(object(&[("password", Some(json!(password)))])),
        )
        .await
    }

    /// The owner resetting a member's password WITHOUT knowing the current one — the whole point
    /// is that the member has forgotten it. Every session of theirs is revoked.
    pub async fn reset_member_password(
        &self,
        user_id: i64,
        new_password: &str,
    ) -> ApiResult<Nothing> {
        self.send_nothing(
            Method::Post,
            &format!("{}/members/{user_id}/password", self.family_root(None)),
            Payload::Json(object(&[("new_password", Some(json!(new_password)))])),
        )
        .await
    }

    /// A new profile picture: RAW BYTES, which with the two avatar reads is the only binary
    /// surface in the protocol. The answer carries the incremented `avatar_version`.
    ///
    /// Clients downscale and re-encode BEFORE this: a square JPEG whose longest edge is 512 px is
    /// what the phones and the Mac send, and the server stores what it is given after validating
    /// the type and the size — it never transcodes.
    pub async fn put_avatar(&self, bytes: Vec<u8>, mime: &str) -> ApiResult<UserResponse> {
        self.send_json(
            Method::Put,
            "/me/avatar",
            Payload::Raw {
                content_type: mime.to_string(),
                bytes,
            },
        )
        .await
    }

    /// Drop it, and set `avatar_version` back to 0. Idempotent.
    pub async fn delete_avatar(&self) -> ApiResult<Nothing> {
        self.send_nothing(Method::Delete, "/me/avatar", Payload::Empty)
            .await
    }

    /// Somebody's picture. Visible to themselves and to members of the same family; anybody else
    /// — and a user with no picture at all — is `404 user_not_found`, the same answer for both,
    /// which is the non-enumeration rule this protocol follows everywhere.
    ///
    /// Asked ONCE, as in `win/`: the bytes reads do not take the JSON reads' retry.
    pub async fn avatar(&self, user_id: i64) -> ApiResult<Vec<u8>> {
        self.bytes(&format!("/users/{user_id}/avatar")).await
    }

    // ---- the family's own console ------------------------------------------------------------

    /// Start a family. The caller becomes its owner and its chat is made with it.
    pub async fn create_family(&self, name: &str) -> ApiResult<FamilyOnlyResponse> {
        self.send_json(
            Method::Post,
            "/families",
            Payload::Json(object(&[("name", Some(json!(name)))])),
        )
        .await
    }

    /// Ask to join one. Under policy `open` the answer is `joined` and membership is immediate;
    /// under `approval` it is `pending` and an owner has to answer.
    ///
    /// A CLOSED family answers `invalid_invite_code`, byte-identical to a code that never existed:
    /// a shut door tells a stranger nothing. A FULL one answers `family_full`, which does admit
    /// the code is real — the alternative is telling an invited member their code is invalid on
    /// the day the family filled up.
    pub async fn join_family(&self, invite_code: &str) -> ApiResult<JoinAnswer> {
        self.send_json(
            Method::Post,
            "/families/join",
            Payload::Json(object(&[("invite_code", Some(json!(invite_code)))])),
        )
        .await
    }

    /// Change the family (owner only). Only the fields present change — and the two that can be
    /// CLEARED send an explicit null ([`FamilyPatch`] writes itself that way).
    pub async fn patch_family(&self, patch: &FamilyPatch) -> ApiResult<FamilyOnlyResponse> {
        self.send_json(
            Method::Patch,
            &self.families(None),
            Payload::Json(wire::encode(patch)),
        )
        .await
    }

    /// A new invite code (owner). The old one stops working; pending requests survive.
    pub async fn rotate_invite_code(&self) -> ApiResult<InviteCodeResponse> {
        self.send_json(
            Method::Post,
            &format!("{}/invite-code/rotate", self.family_root(None)),
            Payload::Empty,
        )
        .await
    }

    pub async fn join_requests(&self) -> ApiResult<JoinRequestsResponse> {
        self.send_json(
            Method::Get,
            &format!("{}/join-requests", self.family_root(None)),
            Payload::Empty,
        )
        .await
    }

    /// Let them in. The cap is re-checked HERE, because the roster can fill between a request and
    /// the decision: `family_full` leaves the request PENDING — a full family is a temporary
    /// condition and not a decision, and the owner may approve it again once a seat frees.
    pub async fn approve_join_request(&self, request_id: i64) -> ApiResult<MemberResponse> {
        self.send_json(
            Method::Post,
            &format!(
                "{}/join-requests/{request_id}/approve",
                self.family_root(None)
            ),
            Payload::Empty,
        )
        .await
    }

    pub async fn reject_join_request(&self, request_id: i64) -> ApiResult<Nothing> {
        self.send_nothing(
            Method::Post,
            &format!(
                "{}/join-requests/{request_id}/reject",
                self.family_root(None)
            ),
            Payload::Empty,
        )
        .await
    }

    /// Leave — the family named (that family only, with the owner's hand-off as ever), else the
    /// one shown, else the home family. AN OWNER WHO LEAVES HANDS THE FAMILY ON and is never
    /// refused; the answer names the successor, or carries nobody when the family went with them.
    ///
    /// Only an owner's leave has a body. A member's — the common one — and the last member's are a
    /// `204` with none (docs/protocol.md, `POST /families/leave`), and that means a [`LeftAnswer`]
    /// naming nobody: not "the answer could not be read".
    pub async fn leave_family(&self, family: Option<i64>) -> ApiResult<LeftAnswer> {
        let path = format!("{}/leave", self.family_root(family));
        self.send_or(Method::Post, &path, Payload::Empty, LeftAnswer::default())
            .await
    }

    // ---- invitations (docs/protocol.md, "Several families") ----------------------------------

    /// Invite somebody who shares ANOTHER family with this owner into this one. `201` with a new
    /// invitation, or `200` with the one already pending — both success. Errors:
    /// `not_family_owner`, `not_same_family`, `user_already_in_family`, `validation` (50 pending at
    /// most).
    pub async fn invite(&self, family_id: i64, user_id: i64) -> ApiResult<InvitationResponse> {
        self.send_json(
            Method::Post,
            &format!("{}/invitations", self.families(Some(family_id))),
            Payload::Json(object(&[("user_id", Some(json!(user_id)))])),
        )
        .await
    }

    /// This family's pending invitations, oldest first (owner).
    pub async fn invitations(&self, family_id: i64) -> ApiResult<InvitationsResponse> {
        self.send_json(
            Method::Get,
            &format!("{}/invitations", self.families(Some(family_id))),
            Payload::Empty,
        )
        .await
    }

    /// Withdraw one (owner); idempotent.
    pub async fn withdraw_invitation(
        &self,
        family_id: i64,
        invitation_id: i64,
    ) -> ApiResult<Nothing> {
        self.send_nothing(
            Method::Delete,
            &format!(
                "{}/invitations/{invitation_id}",
                self.families(Some(family_id))
            ),
            Payload::Empty,
        )
        .await
    }

    /// Accept an invitation into a family: membership is immediate, whatever its join policy.
    /// `family_full` and `too_many_families` leave it PENDING; `invitation_not_pending` means it
    /// is gone.
    pub async fn accept_invitation(&self, invitation_id: i64) -> ApiResult<FamilyOnlyResponse> {
        self.send_json(
            Method::Post,
            &format!("/invitations/{invitation_id}/accept"),
            Payload::Empty,
        )
        .await
    }

    /// Decline one; idempotent for the invited person, and the owner is not told by push.
    pub async fn decline_invitation(&self, invitation_id: i64) -> ApiResult<Nothing> {
        self.send_nothing(
            Method::Post,
            &format!("/invitations/{invitation_id}/decline"),
            Payload::Empty,
        )
        .await
    }

    /// The one-to-one chat with a member: get-or-create, and idempotent — "Message" pressed twice
    /// is one chat (docs/protocol.md, `POST /chats/direct`).
    pub async fn direct_chat(&self, user_id: i64) -> ApiResult<ChatResponse> {
        self.send_json(
            Method::Post,
            "/chats/direct",
            Payload::Json(object(&[("user_id", Some(json!(user_id)))])),
        )
        .await
    }

    /// A new GROUP (docs/protocol.md, `POST /chats/groups`): the caller is added and becomes its
    /// admin, and `member_ids` names the others. `validation` for a name that is empty or over 64
    /// characters once trimmed, or nobody else named; `not_same_family`; `too_many_groups` past
    /// the family's 50.
    ///
    /// The group is made in the family being shown — `family_id`, absent (the home family) while
    /// none is.
    pub async fn create_group(&self, title: &str, member_ids: &[i64]) -> ApiResult<ChatResponse> {
        self.send_json(
            Method::Post,
            "/chats/groups",
            Payload::Json(object(&[
                ("title", Some(json!(title))),
                ("member_ids", Some(json!(member_ids))),
                ("family_id", self.family_scope().map(|id| json!(id))),
            ])),
        )
        .await
    }

    /// A group renamed — its admin only, `not_group_admin` for anybody else (`PATCH /chats/{id}`).
    pub async fn rename_group(&self, chat_id: i64, title: &str) -> ApiResult<ChatResponse> {
        self.send_json(
            Method::Patch,
            &format!("/chats/{chat_id}"),
            Payload::Json(object(&[("title", Some(json!(title)))])),
        )
        .await
    }

    /// Members added to a group — its admin only; an id already in it is ignored, and each one
    /// added starts with the group's newest message read (`POST /chats/{id}/members`).
    pub async fn add_group_members(
        &self,
        chat_id: i64,
        user_ids: &[i64],
    ) -> ApiResult<ChatResponse> {
        self.send_json(
            Method::Post,
            &format!("/chats/{chat_id}/members"),
            Payload::Json(object(&[("user_ids", Some(json!(user_ids)))])),
        )
        .await
    }

    /// A member out of a group: the admin removing somebody else (`200 {chat}`), or — with the
    /// caller's OWN id — any member LEAVING, which is a `204` with no body
    /// (`DELETE /chats/{id}/members/{user_id}`). An admin who leaves hands the group on, and the
    /// last one out deletes it; both are the server's to do.
    pub async fn remove_group_member(
        &self,
        chat_id: i64,
        user_id: i64,
    ) -> ApiResult<GroupMemberAnswer> {
        self.send_or(
            Method::Delete,
            &format!("/chats/{chat_id}/members/{user_id}"),
            Payload::Empty,
            GroupMemberAnswer::default(),
        )
        .await
    }

    /// A group deleted with everything in it, for every member — its admin only
    /// (`DELETE /chats/{id}`). Never the family chat, a direct chat or an `ai` chat: those answer
    /// `validation`.
    pub async fn delete_group(&self, chat_id: i64) -> ApiResult<Nothing> {
        self.send_nothing(Method::Delete, &format!("/chats/{chat_id}"), Payload::Empty)
            .await
    }

    pub async fn remove_member(&self, user_id: i64) -> ApiResult<Nothing> {
        self.send_nothing(
            Method::Delete,
            &format!("{}/members/{user_id}", self.family_root(None)),
            Payload::Empty,
        )
        .await
    }

    /// Your own birthday: a day and a month, and no year at all.
    pub async fn set_my_birthday(&self, month: i32, day: i32) -> ApiResult<UserResponse> {
        self.send_json(
            Method::Put,
            "/me/birthday",
            Payload::Json(object(&[
                ("month", Some(json!(month))),
                ("day", Some(json!(day))),
            ])),
        )
        .await
    }

    pub async fn clear_my_birthday(&self) -> ApiResult<Nothing> {
        self.send_nothing(Method::Delete, "/me/birthday", Payload::Empty)
            .await
    }

    /// The owner filling one in for somebody else — a parent for a child, typically, which is
    /// what makes the family calendar usable at all. The owner MAY name themselves here.
    pub async fn set_member_birthday(
        &self,
        user_id: i64,
        month: i32,
        day: i32,
    ) -> ApiResult<MemberResponse> {
        self.send_json(
            Method::Put,
            &format!("{}/members/{user_id}/birthday", self.family_root(None)),
            Payload::Json(object(&[
                ("month", Some(json!(month))),
                ("day", Some(json!(day))),
            ])),
        )
        .await
    }

    pub async fn clear_member_birthday(&self, user_id: i64) -> ApiResult<Nothing> {
        self.send_nothing(
            Method::Delete,
            &format!("{}/members/{user_id}/birthday", self.family_root(None)),
            Payload::Empty,
        )
        .await
    }

    /// Stop seeing a member. ANY member may block any other, the owner included — and the blocked
    /// member is never told (docs/protocol.md, "Blocking a member").
    pub async fn block(&self, user_id: i64) -> ApiResult<Nothing> {
        // The form WITHOUT a family, on purpose: a block is a pair and holds in every family the
        // two share, and that is what this form means since 2026-10-22 (docs/protocol.md,
        // "Several families").
        self.send_nothing(
            Method::Put,
            &format!("/families/members/{user_id}/block"),
            Payload::Empty,
        )
        .await
    }

    /// Unblock. Scoped to the CALLER'S OWN list and not to the roster: any id on it may be
    /// cleared, including somebody who has since left or deleted their account.
    pub async fn unblock(&self, user_id: i64) -> ApiResult<Nothing> {
        self.send_nothing(
            Method::Delete,
            &format!("/families/members/{user_id}/block"),
            Payload::Empty,
        )
        .await
    }

    /// Report a member, or one of their messages. An OPEN report by this caller against this
    /// member is returned as it stands and creates nothing — a double tap is not two rows in the
    /// owner's list, while reporting a second message IS a second report.
    pub async fn report(
        &self,
        reported_user_id: i64,
        reason: &str,
        message_id: Option<i64>,
    ) -> ApiResult<ReportResponse> {
        self.send_json(
            Method::Post,
            &format!("{}/reports", self.family_root(None)),
            Payload::Json(object(&[
                ("reported_user_id", Some(json!(reported_user_id))),
                ("reason", Some(json!(reason))),
                ("message_id", message_id.map(|id| json!(id))),
            ])),
        )
        .await
    }

    /// What the ASSISTANT got wrong. A separate endpoint from [`report`](Self::report), and not
    /// under `/families` at all: it needs no family, and no owner may read it (docs/protocol.md,
    /// "Reporting the assistant"). A second report of the same reply answers 200 with the stored
    /// row and creates nothing. A note that is only white space is no note.
    pub async fn report_assistant(
        &self,
        message_id: i64,
        reason: &str,
        note: Option<&str>,
    ) -> ApiResult<AssistantReportResponse> {
        let note = note.filter(|note| !note.trim().is_empty());
        self.send_json(
            Method::Post,
            "/reports/assistant",
            Payload::Json(object(&[
                ("message_id", Some(json!(message_id))),
                ("reason", Some(json!(reason))),
                ("note", note.map(|note| json!(note))),
            ])),
        )
        .await
    }

    /// The owner's moderation list: open only, oldest first.
    pub async fn reports(&self) -> ApiResult<ReportsResponse> {
        self.send_json(
            Method::Get,
            &format!("{}/reports", self.family_root(None)),
            Payload::Empty,
        )
        .await
    }

    /// Dealt with — what that MEANS is the owner's business.
    pub async fn resolve_report(&self, report_id: i64) -> ApiResult<Nothing> {
        self.send_nothing(
            Method::Post,
            &format!("{}/reports/{report_id}/resolve", self.family_root(None)),
            Payload::Empty,
        )
        .await
    }

    /// What the family has sent. NOT owner-only: it is a shared curiosity, and the same numbers go
    /// to everyone — except that a member the caller has blocked is left out of the rows.
    pub async fn stats(&self) -> ApiResult<StatsResponse> {
        self.send_json(
            Method::Get,
            &format!("{}/stats", self.families(None)),
            Payload::Empty,
        )
        .await
    }

    // ---- attachments -------------------------------------------------------------------------

    /// The bytes, raw, with their type; the metadata in the query (docs/protocol.md, "Photos,
    /// videos, audio, files and locations"). Unclaimed until a message or a note names it, and
    /// swept after the grace if nothing ever does.
    pub async fn upload(
        &self,
        kind: &str,
        mime: &str,
        bytes: Vec<u8>,
        facts: &UploadFacts<'_>,
    ) -> ApiResult<AttachmentResponse> {
        let mut query = vec![format!("kind={}", escape_data_string(kind))];
        if let Some(width) = facts.width {
            query.push(format!("width={width}"));
        }
        if let Some(height) = facts.height {
            query.push(format!("height={height}"));
        }
        if let Some(ms) = facts.duration_ms {
            query.push(format!("duration_ms={ms}"));
        }
        if let Some(name) = facts.name.filter(|name| !name.is_empty()) {
            query.push(format!("name={}", escape_data_string(name)));
        }
        // A voice note's shape (docs/protocol.md, "A voice note's waveform"): only on audio, and
        // only exactly the wire — the server refuses it anywhere else, and refuses a malformed
        // one, with `validation`. A server from before waveforms ignores the parameter (its query
        // struct does not deny unknown fields), so it is sent to every server, and an answer
        // without it is no failure. Exactly the wire is lowercase hex, which needs no escaping.
        if let Some(waveform) = facts.waveform
            && kind == "audio"
            && fc_text::waveform::parse(waveform).is_some()
        {
            query.push(format!("waveform={waveform}"));
        }
        // The family the upload is FOR (docs/protocol.md, "Several families"): a message may use
        // it only in a chat that family lists, a note or sticker only on its board and in its
        // pack. Absent is the home family.
        if let Some(family) = self.upload_family(facts.family_id) {
            query.push(family);
        }
        self.send_json(
            Method::Post,
            &format!("/attachments?{}", query.join("&")),
            Payload::Raw {
                content_type: mime.to_string(),
                bytes,
            },
        )
        .await
    }

    /// A place: no bytes and no body — its three numbers in the query are the whole upload
    /// (docs/protocol.md, "Locations"). Claimed and swept like any other attachment.
    pub async fn upload_location(
        &self,
        latitude: f64,
        longitude: f64,
        accuracy_m: Option<f64>,
        name: Option<&str>,
        family_id: Option<i64>,
    ) -> ApiResult<AttachmentResponse> {
        let mut query = upload_queries::location(latitude, longitude, accuracy_m, name);
        if let Some(family) = self.upload_family(family_id) {
            query.push('&');
            query.push_str(&family);
        }
        self.send_json(
            Method::Post,
            &format!("/attachments?{query}"),
            Payload::Empty,
        )
        .await
    }

    /// The upload's `family_id` parameter: the family named, else the one shown, else none.
    fn upload_family(&self, family_id: Option<i64>) -> Option<String> {
        family_id
            .or(self.family_scope())
            .map(|id| format!("family_id={id}"))
    }

    /// The downscaled photo or the video's poster, as JPEG. Uploader only, idempotent, and not
    /// closed by the message that claims the attachment: a preview that failed may be sent again
    /// later (docs/protocol.md, "A preview may be uploaded again, later").
    pub async fn upload_preview(&self, attachment_id: i64, jpeg: Vec<u8>) -> ApiResult<Nothing> {
        self.send_nothing(
            Method::Put,
            &format!("/attachments/{attachment_id}/preview"),
            Payload::Raw {
                content_type: "image/jpeg".to_string(),
                bytes: jpeg,
            },
        )
        .await
    }

    /// One attachment's bytes, or its preview. Asked ONCE, as in `win/`.
    pub async fn download(&self, attachment_id: i64, preview: bool) -> ApiResult<Vec<u8>> {
        let path = if preview {
            format!("/attachments/{attachment_id}/preview")
        } else {
            format!("/attachments/{attachment_id}")
        };
        self.bytes(&path).await
    }

    // ---- the transport -----------------------------------------------------------------------

    fn request(
        &self,
        method: Method,
        path: &str,
        payload: Payload,
        deadline: Duration,
    ) -> HttpRequest {
        // EVERY request: without it the server answers as it did before 2026-10-22, about the
        // home family alone.
        let mut request =
            HttpRequest::new(method, format!("{}{path}", self.rest)).header(FAMILIES_HEADER, "1");
        if let Some(token) = self.tokens.token().filter(|token| !token.is_empty()) {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        request.timeout = Some(deadline);
        match payload {
            Payload::Empty => request,
            Payload::Json(json) => request.body("application/json; charset=utf-8", json.into()),
            Payload::Raw {
                content_type,
                bytes,
            } => request.body(&content_type, bytes),
        }
    }

    /// One request under a deadline of its own. A deadline that passes is a transport failure,
    /// exactly what `HttpClient.Timeout` produced before the deadlines moved into the client. The
    /// transport answers with the whole body, so the deadline covers the body too.
    async fn send_within(
        &self,
        request: HttpRequest,
        deadline: Duration,
    ) -> Result<HttpResponse, TransportError> {
        match select(self.http.send(request), self.clock.sleep(deadline)).await {
            Either::Left((answer, _)) => answer,
            Either::Right(((), _)) => Err(TransportError::Timeout),
        }
    }

    /// The exchange every JSON call makes: a 2xx response, or the failure. A read that failed
    /// transiently is tried once more; nothing else is.
    async fn exchange(
        &self,
        method: Method,
        path: &str,
        payload: Payload,
        deadline: Duration,
    ) -> Result<HttpResponse, ApiError> {
        // A read is safe to repeat and a write is not: only the outbox knows whether a send
        // carries a dedup key, so nothing here retries a POST, PATCH, PUT or DELETE.
        let may_retry = method == Method::Get;
        let mut payload = Some(payload);
        let mut attempt = 0;
        loop {
            let body = if may_retry {
                payload.clone()
            } else {
                payload.take()
            };
            let request = self.request(method, path, body.unwrap_or(Payload::Empty), deadline);
            let response = match self.send_within(request, deadline).await {
                Ok(response) => response,
                Err(unreached) => {
                    // A read that never arrived is retried once, like a read that hit a 502: a
                    // pooled connection dropped while idle fails exactly this way, and the first
                    // call after a laptop wakes up is the common case.
                    if may_retry && attempt == 0 {
                        self.clock.sleep(RETRY_PAUSE).await;
                        attempt += 1;
                        continue;
                    }
                    return Err(ApiError::transport(&unreached.to_string()));
                }
            };
            if (200..=299).contains(&response.status) {
                return Ok(response);
            }
            let error = self.failure(&response);
            if may_retry && attempt == 0 && error.transient() {
                // Once, and only on a read. Honour the server's own wait where it gave one.
                let wait = error
                    .retry_after
                    .unwrap_or(time::Duration::milliseconds(250))
                    .min(RETRY_AFTER_CAP);
                if wait > time::Duration::ZERO {
                    self.clock
                        .sleep(Duration::try_from(wait).unwrap_or(RETRY_PAUSE))
                        .await;
                }
                attempt += 1;
                continue;
            }
            return Err(error);
        }
    }

    /// A call whose answer is a JSON document.
    async fn send_json<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        payload: Payload,
    ) -> ApiResult<T> {
        let answer = self
            .exchange(method, path, payload, self.request_deadline)
            .await;
        read_json(answer)
    }

    /// A call whose success carries nothing this client reads: any 2xx is the answer.
    async fn send_nothing(
        &self,
        method: Method,
        path: &str,
        payload: Payload,
    ) -> ApiResult<Nothing> {
        match self
            .exchange(method, path, payload, self.request_deadline)
            .await
        {
            Ok(response) => ApiResult::success(Nothing, response.status),
            Err(error) => ApiResult::failure(error),
        }
    }

    /// A call whose protocol row says a success may carry no body: a `204` MEANS `no_content`.
    /// Everywhere else a 2xx this client cannot read stays a failure.
    async fn send_or<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        payload: Payload,
        no_content: T,
    ) -> ApiResult<T> {
        match self
            .exchange(method, path, payload, self.request_deadline)
            .await
        {
            Ok(response) if response.status == 204 => ApiResult::success(no_content, 204),
            answer => read_json(answer),
        }
    }

    /// The bytes of a GET, raw — asked once, with no retry, as `Avatar` and `Download` were.
    async fn bytes(&self, path: &str) -> ApiResult<Vec<u8>> {
        let request = self.request(Method::Get, path, Payload::Empty, self.request_deadline);
        match self.send_within(request, self.request_deadline).await {
            Err(unreached) => ApiResult::failure(ApiError::transport(&unreached.to_string())),
            Ok(response) if (200..=299).contains(&response.status) => {
                let status = response.status;
                ApiResult::success(response.body, status)
            }
            Ok(response) => ApiResult::failure(self.failure(&response)),
        }
    }

    /// The failure, read as the protocol writes it — and classified without it where nginx
    /// answered its own rate limit with an HTML body.
    ///
    /// A body that is not the error shape — HTML, `{}`, a proxy's `{"message": …}`, an `"error"`
    /// that is not an object — is no envelope, and the STATUS classifies it. (`win/` threw a
    /// NullReferenceException here for a JSON body without `"error"`, which nothing caught.)
    fn failure(&self, response: &HttpResponse) -> ApiError {
        let status = response.status;
        let wait = response
            .header("Retry-After")
            .and_then(|value| retry_after(value, self.clock.now()));
        let (code, message) = match wire::decode::<ErrorEnvelope>(&text_of(&response.body)) {
            Some(envelope) => (envelope.error.code, envelope.error.message),
            None => (
                if status == 429 {
                    error_codes::TOO_MANY_REQUESTS
                } else {
                    ""
                }
                .to_string(),
                reason_phrase(status).unwrap_or_default().to_string(),
            ),
        };
        let error = ApiError::new(&code, &message, status);
        match wait {
            Some(wait) => error.with_retry_after(wait),
            None => error,
        }
    }
}

/// A 2xx read as `T`, or the failure.
fn read_json<T: DeserializeOwned>(answer: Result<HttpResponse, ApiError>) -> ApiResult<T> {
    match answer {
        Err(error) => ApiResult::failure(error),
        Ok(response) => match wire::decode::<T>(&text_of(&response.body)) {
            Some(value) => ApiResult::success(value, response.status),
            // A 2xx whose body this client cannot read is not a success it can act on — and it is
            // TERMINAL, not transient: repeating the call will produce the same body.
            None => ApiResult::failure(ApiError::new(
                error_codes::VALIDATION,
                "the answer could not be read",
                response.status,
            )),
        },
    }
}

/// A body as text, as .NET's `ReadAsStringAsync` reads one: a UTF-8 byte-order mark dropped, and
/// a byte that is not UTF-8 read as U+FFFD rather than failing the whole body (measured on
/// .NET 10). A charset other than UTF-8 is read as UTF-8 here, where .NET honoured it — the server
/// writes nothing else.
fn text_of(body: &[u8]) -> Cow<'_, str> {
    String::from_utf8_lossy(body.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(body))
}

/// A JSON object with its keys in the order given and every `None` left out — what
/// System.Text.Json made of an anonymous object under `WhenWritingNull`. (`serde_json::json!`
/// sorts its keys, and the bodies' order is what the tests and the C# read.)
fn object(entries: &[(&str, Option<Value>)]) -> String {
    let mut out = String::from("{");
    for (key, value) in entries
        .iter()
        .filter_map(|(key, value)| value.as_ref().map(|value| (key, value)))
    {
        if out.len() > 1 {
            out.push(',');
        }
        out.push_str(&wire::encode(key));
        out.push(':');
        out.push_str(&wire::encode(value));
    }
    out.push('}');
    out
}

/// One part of `multipart/form-data` (RFC 7578) and the `Content-Type` that names its boundary:
/// `MultipartFormDataContent { { part, name, fileName } }` in `win/`. The boundary is random — as
/// .NET's GUID was — and checked not to occur in the bytes.
fn form_data(name: &str, file_name: &str, content_type: &str, bytes: &[u8]) -> (String, Vec<u8>) {
    let boundary = loop {
        let candidate = format!("fc-{:016x}{:016x}", random_u64(), random_u64());
        if !bytes
            .windows(candidate.len())
            .any(|window| window == candidate.as_bytes())
        {
            break candidate;
        }
    };
    let mut body = Vec::with_capacity(bytes.len() + 256);
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"; \
             filename=\"{file_name}\"\r\nContent-Type: {content_type}\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

/// The system's randomness, without a crate: std's `RandomState` is seeded from the operating
/// system once per thread and each new one hashes with fresh keys. A boundary is not a secret.
fn random_u64() -> u64 {
    RandomState::new().build_hasher().finish()
}

/// What a response's `Retry-After` asks for, as a wait from `now`: delta-seconds, or an HTTP date
/// (which may be in the past — a NEGATIVE wait, which the reader clamps). `None` for anything
/// .NET's `RetryConditionHeaderValue` did not read.
///
/// Held to .NET 10's own parser (measured; the cases are in `tests/protocol_api_client.rs`):
/// - leading and trailing spaces and tabs are allowed;
/// - text that starts with a digit is delta-seconds or nothing: digits only, at most
///   `i32::MAX` — `+2`, `2.5`, `0x10` and `21 Oct 2015 …` are nothing;
/// - otherwise a date in one of the three forms HTTP has had: `Wed, 21 Oct 2015 07:28:00 GMT`
///   (RFC 1123, with a two-digit year allowed), `Wednesday, 21-Oct-15 07:28:00 GMT` (RFC 850)
///   and `Wed Oct 21 07:28:00 2015` (asctime). Day and month names are read without case and
///   the weekday must be the date's; the zone is `GMT` or `UTC` (in capitals), a numeric offset
///   (`+0100`, `+01:00`, `+1:00`, at most 14 hours — not after a two-digit RFC 1123 year), or
///   absent, which is UTC; asctime takes no zone. Day, hour, minute and second may be one digit
///   or two; a two-digit year is 2000–2049 or 1950–1999. Spaces and tabs between the parts are
///   free.
///
/// Where .NET's `DateTimeOffset.TryParseExact` was looser still — digits run together across
/// parts (`201507:28:00`) — this reads nothing, and the read falls back to its own wait.
pub fn retry_after(value: &str, now: OffsetDateTime) -> Option<time::Duration> {
    let text = value.trim_matches([' ', '\t']);
    if text.starts_with(|c: char| c.is_ascii_digit()) {
        if !text.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let digits = text.trim_start_matches('0');
        if digits.len() > 10 {
            return None;
        }
        let seconds: i64 = digits.parse().unwrap_or(0);
        return (seconds <= i64::from(i32::MAX)).then(|| time::Duration::seconds(seconds));
    }
    http_date(text).map(|when| when - now)
}

/// One piece of an HTTP date.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece<'a> {
    Word(&'a str),
    Number(&'a str),
    /// A sign glued to the digits after it, with an optional `:MM` glued on: `+0100`, `-5:30`.
    Offset(&'a str),
    Comma,
    Colon,
    Dash,
}

fn pieces(text: &str) -> Option<Vec<Piece<'_>>> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        match bytes[i] {
            b' ' | b'\t' => i += 1,
            b',' => {
                out.push(Piece::Comma);
                i += 1;
            }
            b':' => {
                out.push(Piece::Colon);
                i += 1;
            }
            // A dash is an offset's sign only where an offset can stand — glued to digits, after
            // the time's last number; anywhere else (`21-Oct-15`) it is a dash.
            b'-' if !(bytes.get(i + 1).is_some_and(u8::is_ascii_digit)
                && matches!(out.last(), Some(Piece::Number(_)))) =>
            {
                out.push(Piece::Dash);
                i += 1;
            }
            b'+' | b'-' => {
                i += 1;
                while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b':') {
                    i += 1;
                }
                out.push(Piece::Offset(&text[start..i]));
            }
            b if b.is_ascii_digit() => {
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                out.push(Piece::Number(&text[start..i]));
            }
            b if b.is_ascii_alphabetic() => {
                while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
                    i += 1;
                }
                out.push(Piece::Word(&text[start..i]));
            }
            _ => return None,
        }
    }
    Some(out)
}

const DAYS: [(&str, &str, Weekday); 7] = [
    ("mon", "monday", Weekday::Monday),
    ("tue", "tuesday", Weekday::Tuesday),
    ("wed", "wednesday", Weekday::Wednesday),
    ("thu", "thursday", Weekday::Thursday),
    ("fri", "friday", Weekday::Friday),
    ("sat", "saturday", Weekday::Saturday),
    ("sun", "sunday", Weekday::Sunday),
];

const MONTHS: [(&str, Month); 12] = [
    ("jan", Month::January),
    ("feb", Month::February),
    ("mar", Month::March),
    ("apr", Month::April),
    ("may", Month::May),
    ("jun", Month::June),
    ("jul", Month::July),
    ("aug", Month::August),
    ("sep", Month::September),
    ("oct", Month::October),
    ("nov", Month::November),
    ("dec", Month::December),
];

fn short_day(word: &str) -> Option<Weekday> {
    DAYS.iter()
        .find(|(short, _, _)| word.eq_ignore_ascii_case(short))
        .map(|(_, _, day)| *day)
}

fn long_day(word: &str) -> Option<Weekday> {
    DAYS.iter()
        .find(|(_, long, _)| word.eq_ignore_ascii_case(long))
        .map(|(_, _, day)| *day)
}

fn month(word: &str) -> Option<Month> {
    MONTHS
        .iter()
        .find(|(name, _)| word.eq_ignore_ascii_case(name))
        .map(|(_, month)| *month)
}

/// A number of one or two digits.
fn small(piece: &Piece<'_>) -> Option<u8> {
    match piece {
        Piece::Number(digits) if digits.len() <= 2 => digits.parse().ok(),
        _ => None,
    }
}

/// A two-digit year as .NET's invariant calendar reads one: 00–49 is 2000–2049, 50–99 is 1950–
/// 1999 (`TwoDigitYearMax` 2049).
fn year(piece: &Piece<'_>, long: bool) -> Option<i32> {
    match piece {
        Piece::Number(digits) if long && digits.len() == 4 => digits.parse().ok(),
        Piece::Number(digits) if !long && digits.len() == 2 => {
            let short: i32 = digits.parse().ok()?;
            Some(if short < 50 {
                2000 + short
            } else {
                1900 + short
            })
        }
        _ => None,
    }
}

/// `H:m:s`, one or two digits each, from `pieces[at..at + 5]`.
fn clock_time(pieces: &[Piece<'_>], at: usize) -> Option<Time> {
    match pieces.get(at..at + 5)? {
        [hour, Piece::Colon, minute, Piece::Colon, second] => {
            Time::from_hms(small(hour)?, small(minute)?, small(second)?).ok()
        }
        _ => None,
    }
}

/// The zone after a date, which must be the last piece: `GMT`, `UTC`, an offset (when allowed), or
/// nothing at all.
fn zone(rest: &[Piece<'_>], offsets: bool) -> Option<UtcOffset> {
    match rest {
        [] => Some(UtcOffset::UTC),
        [Piece::Word("GMT" | "UTC")] => Some(UtcOffset::UTC),
        [Piece::Offset(text)] if offsets => offset(text),
        _ => None,
    }
}

/// `+HHMM`, `+H:MM` or `+HH:MM`, at most fourteen hours either way.
fn offset(text: &str) -> Option<UtcOffset> {
    let (sign, digits) = text.split_at(1);
    let (hours, minutes) = match digits.split_once(':') {
        Some((hours, minutes)) if (1..=2).contains(&hours.len()) && minutes.len() == 2 => {
            (hours, minutes)
        }
        None if digits.len() == 4 => digits.split_at(2),
        _ => return None,
    };
    let (hours, minutes): (i8, i8) = (hours.parse().ok()?, minutes.parse().ok()?);
    if minutes >= 60 || hours > 14 || (hours == 14 && minutes > 0) {
        return None;
    }
    let sign = if sign == "-" { -1 } else { 1 };
    UtcOffset::from_hms(sign * hours, sign * minutes, 0).ok()
}

/// The moment, when the date is real and the weekday is its own.
fn moment(
    weekday: Weekday,
    year: i32,
    month: Month,
    day: u8,
    time: Time,
    zone: UtcOffset,
) -> Option<OffsetDateTime> {
    let date = Date::from_calendar_date(year, month, day).ok()?;
    if !(1..=9999).contains(&year) || date.weekday() != weekday {
        return None;
    }
    Some(PrimitiveDateTime::new(date, time).assume_offset(zone))
}

/// An HTTP date in any of the three forms [`retry_after`] reads.
fn http_date(text: &str) -> Option<OffsetDateTime> {
    let pieces = pieces(text)?;
    match pieces.as_slice() {
        // RFC 850: `Wednesday, 21-Oct-15 07:28:00 GMT`.
        [
            Piece::Word(day),
            Piece::Comma,
            date,
            Piece::Dash,
            Piece::Word(name),
            Piece::Dash,
            short,
            ..,
        ] if long_day(day).is_some() => moment(
            long_day(day)?,
            year(short, false)?,
            month(name)?,
            small(date)?,
            clock_time(&pieces, 7)?,
            zone(pieces.get(12..)?, true)?,
        ),
        // RFC 1123: `Wed, 21 Oct 2015 07:28:00 GMT`, or `… 15 …`.
        [
            Piece::Word(day),
            Piece::Comma,
            date,
            Piece::Word(name),
            Piece::Number(digits),
            ..,
        ] => {
            let long = digits.len() == 4;
            moment(
                short_day(day)?,
                year(&Piece::Number(digits), long)?,
                month(name)?,
                small(date)?,
                clock_time(&pieces, 5)?,
                zone(pieces.get(10..)?, long)?,
            )
        }
        // asctime: `Wed Oct 21 07:28:00 2015`, no zone.
        [Piece::Word(day), Piece::Word(name), date, ..] => {
            let time = clock_time(&pieces, 3)?;
            match pieces.get(8..)? {
                [full] => moment(
                    short_day(day)?,
                    year(full, true)?,
                    month(name)?,
                    small(date)?,
                    time,
                    UtcOffset::UTC,
                ),
                _ => None,
            }
        }
        _ => None,
    }
}

/// .NET's reason phrase for a status (`HttpResponseMessage.ReasonPhrase` when the transport gave
/// none), measured on .NET 10: the message of a failure with no envelope.
fn reason_phrase(status: u16) -> Option<&'static str> {
    Some(match status {
        100 => "Continue",
        101 => "Switching Protocols",
        102 => "Processing",
        103 => "Early Hints",
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        203 => "Non-Authoritative Information",
        204 => "No Content",
        205 => "Reset Content",
        206 => "Partial Content",
        207 => "Multi-Status",
        208 => "Already Reported",
        226 => "IM Used",
        300 => "Multiple Choices",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        305 => "Use Proxy",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        402 => "Payment Required",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        406 => "Not Acceptable",
        407 => "Proxy Authentication Required",
        408 => "Request Timeout",
        409 => "Conflict",
        410 => "Gone",
        411 => "Length Required",
        412 => "Precondition Failed",
        413 => "Request Entity Too Large",
        414 => "Request-Uri Too Long",
        415 => "Unsupported Media Type",
        416 => "Requested Range Not Satisfiable",
        417 => "Expectation Failed",
        421 => "Misdirected Request",
        422 => "Unprocessable Entity",
        423 => "Locked",
        424 => "Failed Dependency",
        426 => "Upgrade Required",
        428 => "Precondition Required",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        451 => "Unavailable For Legal Reasons",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        505 => "Http Version Not Supported",
        506 => "Variant Also Negotiates",
        507 => "Insufficient Storage",
        508 => "Loop Detected",
        510 => "Not Extended",
        511 => "Network Authentication Required",
        _ => return None,
    })
}
