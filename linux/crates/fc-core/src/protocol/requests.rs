//! The bodies this client sends and the envelopes it reads back (docs/protocol.md, "REST
//! endpoints") — a port of `win/src/FamilyConnect.Core/Protocol/Requests.cs`.
//!
//! Every optional field is an `Option` and omitted when `None` ([`wire::encode`]), because on
//! this wire ABSENT and EMPTY are different answers: a PATCH that leaves a field out changes
//! nothing, and one that sends it empty clears it. A field with a default would send the default.
//!
//! ---- what comes back ----
//!
//! EVERY LIST IN AN ANSWER IS AN `Option`, and every reader of one takes `unwrap_or_default()`.
//! Not because the server ever omits an array — it does not — but because a 200 is not a promise
//! that the body is THIS body. A rewriting proxy, a captive portal or a path the server has since
//! moved all answer readable JSON that simply has no `messages` key, and a page whose array came
//! back null cost a whole resync a NullReferenceException in `win/` before this was written down.
//! An answer that is not the documented one carries nothing to apply, which is exactly what an
//! empty page means.

use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize, Serializer};

use super::dtos::{
    AttachmentDto, ChatDto, MemberDto, MentionDto, MessageDto, NoteDto, PackItemDto, PollDto,
    ReactionDto, UserDto, wire,
};

/// `POST /chats/{id}/messages`: the REST twin of the `send` frame.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SendRequest {
    #[serde(default, deserialize_with = "wire::text")]
    pub client_msg_id: String,
    #[serde(default, deserialize_with = "wire::text")]
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to_message_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment_ids: Option<Vec<i64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll: Option<PollRequest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mentions: Option<Vec<MentionDto>>,
    /// `Some(true)` sends the message's one attachment as a STICKER (docs/protocol.md, "Sending
    /// one"). Absent otherwise — never `false`, which is why it is an `Option`: an ordinary
    /// message must go to a server that predates stickers exactly as it always went.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sticker: Option<bool>,
    /// `Some(true)` sends the message's one video as a VIDEO MESSAGE, drawn round
    /// (docs/protocol.md, "Video messages"). Absent otherwise — never `false` — and never sent to
    /// a server whose `GET /families/mine` does not name `max_round_video_ms`, which would ignore
    /// it and deliver a square video.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub round: Option<bool>,
    /// `Some(true)` sends the message's one video as an ANIMATION — an animated picture made into
    /// a silent looping video, drawn as a picture that plays by itself (docs/protocol.md,
    /// "Animations"). Absent otherwise — never `false`. No discovery key guards it: a server that
    /// predates it ignores it and delivers the ordinary video the file still is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub animation: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PollRequest {
    #[serde(default)]
    pub options: Vec<String>,
}

/// A new note. `kind` decides which of the rest are allowed: a photo REQUIRES
/// [`attachment_id`](Self::attachment_id), an event requires [`starts_at`](Self::starts_at) and
/// may carry a backdrop, a list carries [`items`](Self::items), and a text note refuses all of
/// them.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct NoteRequest {
    #[serde(default, deserialize_with = "wire::text")]
    pub text: String,
    #[serde(default, deserialize_with = "wire::text")]
    pub color: String,
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub starts_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ends_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mentions: Option<Vec<MentionDto>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Vec<TaskLineRequest>>,
}

impl NoteRequest {
    /// `new NoteRequest(text, color, x, y)`, the rest absent.
    pub fn new(text: &str, color: &str, x: f64, y: f64) -> Self {
        NoteRequest {
            text: text.to_string(),
            color: color.to_string(),
            x,
            y,
            ..NoteRequest::default()
        }
    }
}

/// An edit. EVERY field is optional and only what is sent changes — except the ones that
/// REPLACE: `mentions` re-decides a note's names (sending text without them clears them) and
/// `items` replaces a list's lines, where an entry that keeps its id keeps its tick.
///
/// Encoded as the wire takes it (what `ApiClient.NoteBody` did in `win/`): absent means "leave
/// alone" and every `None` is left out — so an event's end taken OFF is written in as the
/// `"ends_at": null` the protocol says clears it, by [`clears_end`](Self::clears_end).
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct NotePatch {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub size: Option<String>,
    #[serde(default)]
    pub font: Option<String>,
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
    #[serde(default)]
    pub starts_at: Option<String>,
    #[serde(default)]
    pub ends_at: Option<String>,
    #[serde(default)]
    pub place: Option<String>,
    #[serde(default)]
    pub mentions: Option<Vec<MentionDto>>,
    #[serde(default)]
    pub items: Option<Vec<TaskLineRequest>>,
    /// Send `"ends_at": null` — take an event's end off, rather than leave it alone. Never on the
    /// wire under its own name.
    #[serde(skip)]
    pub clears_end: bool,
}

impl NotePatch {
    /// Whether this patch would change nothing at all (`patch == new NotePatch()` in `win/`).
    pub fn is_empty(&self) -> bool {
        *self == NotePatch::default()
    }
}

impl Serialize for NotePatch {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        put(&mut map, "text", &self.text)?;
        put(&mut map, "color", &self.color)?;
        put(&mut map, "size", &self.size)?;
        put(&mut map, "font", &self.font)?;
        put(&mut map, "x", &self.x)?;
        put(&mut map, "y", &self.y)?;
        put(&mut map, "starts_at", &self.starts_at)?;
        if self.clears_end {
            // The one null on this body: the protocol's "take the end off".
            map.serialize_entry("ends_at", &None::<String>)?;
        } else {
            put(&mut map, "ends_at", &self.ends_at)?;
        }
        put(&mut map, "place", &self.place)?;
        put(&mut map, "mentions", &self.mentions)?;
        put(&mut map, "items", &self.items)?;
        map.end()
    }
}

/// One entry of a hand-written body, written only when it is there.
fn put<M: SerializeMap, T: Serialize>(
    map: &mut M,
    key: &'static str,
    value: &Option<T>,
) -> Result<(), M::Error> {
    match value {
        Some(value) => map.serialize_entry(key, value),
        None => Ok(()),
    }
}

/// One line of a list as the author wrote it: the id where the note already holds one — which is
/// what keeps its tick — and the words. An `id` the note does not hold is `validation`, and ids
/// are the server's to invent.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TaskLineRequest {
    #[serde(default, deserialize_with = "wire::text")]
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
}

impl TaskLineRequest {
    pub fn new(text: &str, id: Option<i64>) -> Self {
        TaskLineRequest {
            text: text.to_string(),
            id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AuthResponse {
    #[serde(default, deserialize_with = "wire::text")]
    pub token: String,
    pub user: UserDto,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MessageResponse {
    pub message: MessageDto,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct NoteResponse {
    pub note: NoteDto,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AttachmentResponse {
    pub attachment: AttachmentDto,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct BoardResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<Vec<NoteDto>>,
    #[serde(default)]
    pub max_board_seq: i64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct BoardChangesResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<Vec<NoteDto>>,
}

/// `GET /families/mine/pack`: the WHOLE pack as it now stands, tombstones excluded, in the order
/// the items were added. `max_pack_seq` is read before the items, so it is a promise: every
/// change at or below it is in the items that came with it.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PackResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Vec<PackItemDto>>,
    #[serde(default)]
    pub max_pack_seq: i64,
}

/// A page of the pack's catch-up, oldest sequence first, tombstones INCLUDED.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PackChangesResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Vec<PackItemDto>>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PackItemResponse {
    pub item: PackItemDto,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MessagesResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub messages: Option<Vec<MessageDto>>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ChatsResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chats: Option<Vec<ChatRowDto>>,
}

/// A page of the reaction catch-up, oldest sequence first.
///
/// The cursor advances with EVERY page, even for messages this client does not hold: states for
/// unknown messages are dropped, and history paging re-delivers them embedded on the messages
/// themselves.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ReactionsResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_reactions: Option<Vec<MessageReactionsDto>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MessageReactionsDto {
    #[serde(default)]
    pub message_id: i64,
    #[serde(default)]
    pub reaction_seq: i64,
    #[serde(default)]
    pub reactions: Vec<ReactionDto>,
}

/// One STUN or TURN server for a call's peer connection, with the time-limited credentials the
/// operator minted for this caller when there are any (docs/protocol.md, "Where the servers come
/// from").
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct IceServerDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub urls: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
}

/// `GET /calls/ice`: fetched at the start of every call, and never cached across calls.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct IceServersResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ice_servers: Option<Vec<IceServerDto>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_secs: Option<i64>,
}

/// A page of the poll catch-up, oldest sequence first.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PollsResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub polls: Option<Vec<MessagePollDto>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MessagePollDto {
    #[serde(default)]
    pub message_id: i64,
    pub poll: PollDto,
}

/// One row of the chat list: the chat, its preview, and this caller's own cursors.
///
/// The three `max_*_seq` cursors are absent until something of that kind has ever happened in
/// the chat, and they are high-water marks that never go back down; `last_read_message_id` is
/// always present, and `0` means "never reported reading anything here", which is a real answer.
/// `mentioned` rides only when an unread message names the caller — absent, never false.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ChatRowDto {
    pub chat: ChatDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message: Option<MessageDto>,
    #[serde(default)]
    pub unread_count: i32,
    #[serde(default)]
    pub last_read_message_id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_reaction_seq: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_edit_seq: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_poll_seq: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mentioned: Option<bool>,
}

impl ChatRowDto {
    /// `new ChatRowDto(chat)`, the rest absent.
    pub fn new(chat: ChatDto) -> Self {
        ChatRowDto {
            chat,
            ..ChatRowDto::default()
        }
    }
}

/// Step 1 of the resync: who I am, whether I have a family, and what this SERVER can do.
///
/// The capability flags are here rather than discovered as errors — a shut door is shown shut
/// rather than met as a 403 after somebody has typed a name. `blocked_user_ids` is the one read
/// where absence is not allowed to mean "leave what you hold alone": it is complete state, and
/// `[]` is the answer when nobody is blocked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeResponse {
    pub user: UserDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<FamilyDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_join_request: Option<PendingJoinDto>,
    #[serde(default)]
    pub calls_enabled: bool,
    #[serde(default)]
    pub video_calls_enabled: bool,
    #[serde(default)]
    pub max_family_members: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_user_ids: Option<Vec<i64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub support_contact: Option<String>,
    /// Whether this server takes NEW families. Absent is true: every server from before the
    /// switch took them.
    #[serde(default = "wire::yes")]
    pub family_registration_enabled: bool,
    #[serde(default)]
    pub familyless_account_ttl_days: i32,
    #[serde(default)]
    pub greetings_enabled: bool,
    /// When this caller agreed that their words may go to the model, or `None` until they have —
    /// and `None` on a server with no assistant, which a client never has to tell apart because
    /// such a server offers no `ai` chat (docs/protocol.md, "Consenting to the assistant").
    ///
    /// Read at step 1 of the resync, so the composer knows before it is drawn whether the next
    /// thing to show is the consent screen rather than a send.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_consent_at: Option<String>,
    /// When this caller agreed that the assistant may send a query or place name it writes from
    /// their words to the lookup providers, or `None` until they have — and `None` on a server
    /// with no lookup source (docs/protocol.md, "Consenting to the assistant", amended
    /// 2026-10-03). Absent on an older server, which reads as `None`: no lookups.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_lookup_consent_at: Option<String>,
    /// Every family the caller is in, the HOME family first and then in the order they were
    /// joined, each with its own `role` (docs/protocol.md, "Several families", 2026-10-22).
    /// ALWAYS present on a server that knows several families, so `None` is an older server —
    /// read through [`all_families`](Self::all_families), which falls back to
    /// [`family`](Self::family).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub families: Option<Vec<FamilyDto>>,
    /// Every join request still waiting, oldest first; `None` from an older server
    /// ([`all_pending`](Self::all_pending)).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_join_requests: Option<Vec<PendingJoinDto>>,
    /// The caller's own pending invitations into other families, oldest first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invitations: Option<Vec<InvitationDto>>,
    /// The operator's `[families] member_families`: whether a member may start another family in
    /// the app. False is what an older server's silence means — it has no such door.
    #[serde(default)]
    pub member_families_enabled: bool,
    /// How many families one account may be in; 0 from an older server, which allows one
    /// ([`family_cap`](Self::family_cap)).
    #[serde(default)]
    pub max_families_per_account: i32,
    /// Whether this server charges for a NEW family (docs/protocol.md, "Paid families",
    /// 2026-10-30): `[subscriptions] enabled`. ALWAYS present on a server that knows paid families;
    /// absent is `false`, every server from before — and with it false nothing below applies.
    #[serde(default)]
    pub subscriptions_enabled: bool,
    /// The caller's own App Store subscription — their active one, or else the one that ended last
    /// — or `None`: none ever, or a server with subscriptions off. Only `active` decides anything
    /// here: a client that cannot sell offers "Create a family" only while it is true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscription: Option<SubscriptionDto>,
    /// What a paid family gets, each limit present only while it is on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paid_family_limits: Option<PaidFamilyLimitsDto>,
    /// How many days an unpaid family is kept before it is deleted (14 by default); absent with
    /// subscriptions off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unpaid_days: Option<i32>,
}

impl Default for MeResponse {
    fn default() -> Self {
        MeResponse {
            user: UserDto::default(),
            family: None,
            role: None,
            pending_join_request: None,
            calls_enabled: false,
            video_calls_enabled: false,
            max_family_members: 0,
            blocked_user_ids: None,
            support_contact: None,
            family_registration_enabled: true,
            familyless_account_ttl_days: 0,
            greetings_enabled: false,
            assistant_consent_at: None,
            assistant_lookup_consent_at: None,
            families: None,
            pending_join_requests: None,
            invitations: None,
            member_families_enabled: false,
            max_families_per_account: 0,
            subscriptions_enabled: false,
            subscription: None,
            paid_family_limits: None,
            unpaid_days: None,
        }
    }
}

impl MeResponse {
    /// `new MeResponse(user)`, the rest at their defaults.
    pub fn new(user: UserDto) -> Self {
        MeResponse {
            user,
            ..MeResponse::default()
        }
    }

    /// Whether the caller owns the HOME family — what `role` is about.
    pub fn is_owner(&self) -> bool {
        self.role.as_deref() == Some("owner")
    }

    /// The families, whatever the server's age: [`families`](Self::families) when it is there,
    /// and otherwise the one [`family`](Self::family) an older server describes, carrying the
    /// [`role`](Self::role) that came beside it.
    pub fn all_families(&self) -> Vec<FamilyDto> {
        if let Some(families) = &self.families {
            return families.clone();
        }
        match &self.family {
            Some(home) => {
                let mut home = home.clone();
                if home.role.is_none() {
                    home.role = self.role.clone();
                }
                vec![home]
            }
            None => Vec::new(),
        }
    }

    /// The waiting join requests, whatever the server's age.
    pub fn all_pending(&self) -> &[PendingJoinDto] {
        match (&self.pending_join_requests, &self.pending_join_request) {
            (Some(all), _) => all,
            (None, Some(one)) => std::slice::from_ref(one),
            (None, None) => &[],
        }
    }

    /// The cap on families per account, never below one.
    pub fn family_cap(&self) -> i32 {
        self.max_families_per_account.max(1)
    }

    /// Whether the caller holds an ACTIVE subscription (docs/protocol.md, "Paid families"): not
    /// revoked, and its expiry — or Apple's grace end — still ahead, as the server judged it.
    pub fn subscription_active(&self) -> bool {
        self.subscription.as_ref().is_some_and(|held| held.active)
    }
}

/// An account's App Store subscription as the server holds it (docs/protocol.md, "Paid families"):
/// `expires_at` is when it stops covering anything — the expiry, or Apple's grace end when later —
/// and `renews` is present only when the server knows. Sold and sent by Apple's apps alone; this
/// client only reads `active`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SubscriptionDto {
    #[serde(default, deserialize_with = "wire::text")]
    pub product_id: String,
    #[serde(default, deserialize_with = "wire::text")]
    pub environment: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub renews: Option<bool>,
}

/// `GET /me`'s `paid_family_limits`: what a paid family gets. Each key is present only while that
/// limit is on (`0` in the operator's file turns one off).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PaidFamilyLimitsDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub members: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_bytes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript_ms: Option<i64>,
}

/// An owner's invitation into their family (docs/protocol.md, "Several families"): pending until
/// the invited person accepts or declines it. [`user`](Self::user) is the invited person.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct InvitationDto {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub family_id: i64,
    #[serde(default, deserialize_with = "wire::text")]
    pub family_name: String,
    pub invited_by: UserDto,
    pub user: UserDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct InvitationResponse {
    pub invitation: InvitationDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct InvitationsResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invitations: Option<Vec<InvitationDto>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PendingJoinDto {
    #[serde(default)]
    pub family_id: i64,
    #[serde(default, deserialize_with = "wire::text")]
    pub family_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

/// A family (docs/protocol.md, "Objects").
///
/// `ai_history` reads an ABSENT key as TRUE, where `win/` reads false: the protocol gives it "a
/// real default (true)", and the Mac (`FamilyDTO.init(from:)`) and the web client both fall back
/// to true for a server that predates it. [`Default`] (and [`FamilyDto::new`]) agree with the
/// decoder; every other switch is false.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FamilyDto {
    #[serde(default)]
    pub id: i64,
    #[serde(default, deserialize_with = "wire::text")]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub join_policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invite_code: Option<String>,
    /// The owner's own cap, ABSENT when they never set one — which is not the same as "equal to
    /// the operator's ceiling", because the ceiling moves. A client holds both numbers and the
    /// door takes the lower.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_members: Option<i32>,
    #[serde(default = "wire::yes")]
    pub ai_history: bool,
    #[serde(default)]
    pub ai_vision: bool,
    #[serde(default)]
    pub ai_history_photos: bool,
    #[serde(default)]
    pub ai_greeting: bool,
    #[serde(default)]
    pub ai_faces: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// The owner's switch for OTHER members' recordings (docs/protocol.md, "Transcripts on
    /// request"): with it on, a member may ask for the text of somebody else's voice note or
    /// audio in the family chat. Off by default, tied to no other switch, and never needed for a
    /// member's OWN recordings.
    #[serde(default)]
    pub ai_transcripts: bool,
    /// The owner's switch for looking things up (docs/protocol.md, "Looking things up"): with it
    /// on — and only where the server has a source and the asking member has given the lookup
    /// consent — the assistant may send a query or place name it wrote to the providers
    /// `assistant.lookups` names. Off by default, tied to no other switch; an older server omits
    /// it, which reads as false.
    #[serde(default)]
    pub ai_lookups: bool,
    /// The places the daily greeting gives today's weather for (docs/protocol.md, "Today's
    /// weather, for places the owner chose"), in the owner's order and spelt as the server KEPT
    /// them: at most three, every member reads them, only the owner sets them. `[]` by default;
    /// ABSENT on a server that predates them, which reads as none. A `null` inside the list is
    /// dropped as it is read (see [`wire`]).
    #[serde(
        default,
        deserialize_with = "wire::names",
        skip_serializing_if = "Option::is_none"
    )]
    pub greeting_places: Option<Vec<String>>,
    /// The caller's own role in this family, `"owner"` or `"member"` — on each of `GET /me`'s
    /// `families` (docs/protocol.md, "Several families"); absent on every other read of a family.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// The most members this family may have whatever its owner sets — the operator's
    /// `max_family_members` or, for a paid family, the lower paid limit (docs/protocol.md, "Paid
    /// families", 2026-10-30). The owner's cap picker draws its range from it; absent on a server
    /// that predates it, where `GET /me`'s `max_family_members` is the range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member_ceiling: Option<i32>,
    /// `"free"` or `"paid"`, present only while the server's subscriptions are on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    /// When the server will DELETE this family because its owner's subscription has ended —
    /// present only while a paid family is unpaid. Every member's app says so in a banner; its
    /// absence is the family being covered (again).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delete_at: Option<String>,
}

impl Default for FamilyDto {
    fn default() -> Self {
        FamilyDto {
            id: 0,
            name: String::new(),
            join_policy: None,
            created_at: None,
            invite_code: None,
            max_members: None,
            ai_history: true,
            ai_vision: false,
            ai_history_photos: false,
            ai_greeting: false,
            ai_faces: false,
            language: None,
            ai_transcripts: false,
            ai_lookups: false,
            greeting_places: None,
            role: None,
            member_ceiling: None,
            plan: None,
            delete_at: None,
        }
    }
}

impl FamilyDto {
    /// `new FamilyDto(id, name)`, the rest at their defaults (`ai_history` the protocol's true).
    pub fn new(id: i64, name: &str) -> Self {
        FamilyDto {
            id,
            name: name.to_string(),
            ..FamilyDto::default()
        }
    }

    /// Whether the caller owns this family, as `GET /me`'s `families` says.
    pub fn is_owner(&self) -> bool {
        self.role.as_deref() == Some("owner")
    }
}

/// The family as the family screen needs it, with the assistant's capabilities.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FamilyResponse {
    pub family: FamilyDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub members: Option<Vec<MemberDto>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub former_members: Option<Vec<MemberDto>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_user_ids: Option<Vec<i64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant: Option<AssistantDto>,
    /// The wall's high-water mark, and the ONLY place it is published. ABSENT while the board is
    /// empty and untouched, which is what it is for: a client reads it to know whether a board
    /// catch-up is worth a request at all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_board_seq: Option<i64>,
    /// The sticker pack's high-water mark, the board's mark one table over: ABSENT while the pack
    /// is empty and untouched, and never lower than it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_pack_seq: Option<i64>,
    /// The pack's two ceilings. ALWAYS present on a server that has packs — so their ABSENCE is
    /// how a client knows this server predates them, and offers no sticker button and no pack
    /// management rather than discovering a 404 when somebody taps one (docs/protocol.md, "What
    /// old clients and old servers do").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_pack_items: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_pack_item_bytes: Option<i64>,
    /// Who would inherit the family if the owner left RIGHT NOW — the owner's answer only, and a
    /// PREDICTION with no frame of its own. Any join or leave changes it, so it is re-read
    /// immediately before the leave dialog and never named from a cached value; absent on a fresh
    /// read means the owner is the last member and leaving DELETES the family.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_owner_user_id: Option<i64>,
    /// The two limits of a VIDEO MESSAGE (docs/protocol.md, "Video messages", 2026-10-05): its
    /// length, fixed at 60 000, and the byte ceiling in force. ALWAYS present on a server that has
    /// video messages, so their ABSENCE is how a client knows to offer no way of recording one and
    /// never to send `round` ([`RoundVideoLimits::of`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_round_video_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_round_video_bytes: Option<i64>,
}

impl FamilyResponse {
    /// `new FamilyResponse(family)`, the rest absent.
    pub fn new(family: FamilyDto) -> Self {
        FamilyResponse {
            family,
            ..FamilyResponse::default()
        }
    }

    /// Whether this server can draw at all — the whole of the capability check for the board's
    /// backdrop and the assistant's `/draw` (docs/protocol.md, "Pictures").
    pub fn can_draw_pictures(&self) -> bool {
        self.assistant
            .as_ref()
            .is_some_and(|assistant| assistant.images)
    }

    /// ABSENT means no assistant: a client with none must not offer `@ai` in the composer,
    /// because typing it would produce nothing.
    pub fn has_assistant(&self) -> bool {
        self.assistant.is_some()
    }
}

/// What a server that has video messages says about them on `GET /families/mine`: how long one
/// may be and how many bytes (docs/protocol.md, "Video messages"). A client records to
/// [`max_ms`](Self::max_ms) − 500 and sends the flag only on a clip within
/// [`max_bytes`](Self::max_bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoundVideoLimits {
    pub max_ms: i64,
    pub max_bytes: i64,
}

impl RoundVideoLimits {
    /// Both keys, or nothing: one missing — or a number no server sends — is a server this
    /// client cannot record for, and a video entry that leads to a refusal is worse than none.
    pub fn of(family: &FamilyResponse) -> Option<RoundVideoLimits> {
        match (family.max_round_video_ms, family.max_round_video_bytes) {
            (Some(max_ms), Some(max_bytes)) if max_ms > 0 && max_bytes > 0 => {
                Some(RoundVideoLimits { max_ms, max_bytes })
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AssistantDto {
    #[serde(default)]
    pub user_id: i64,
    #[serde(default, deserialize_with = "wire::text")]
    pub display_name: String,
    #[serde(default, deserialize_with = "wire::text")]
    pub mention: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draw: Option<String>,
    #[serde(default)]
    pub vision: bool,
    #[serde(default)]
    pub images: bool,
    /// WHO ANSWERS, in the operator's own words — shown VERBATIM on the consent screen, because a
    /// person cannot weigh "some third party" (docs/protocol.md, "Consenting to the assistant").
    /// Absent on a server that predates the field, and a client that cannot name the recipient
    /// offers no assistant there at all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub processor: Option<String>,
    /// Whether this server can turn a recording into text — the whole of the capability check
    /// for "Show text" (docs/protocol.md, "Transcripts on request"). Absent on an older server,
    /// which reads as false.
    #[serde(default)]
    pub transcribe: bool,
    /// The most bytes of sound one transcript request may send; present only while
    /// [`transcribe`](Self::transcribe) is true. 25 MiB by default and never more.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcribe_max_bytes: Option<i64>,
    /// The providers the assistant may look things up in, by NAME, in the server's order (web
    /// search, then `"Open-Meteo"`, then `"Wikipedia"`) — named on the consent screen and beside
    /// the owner's `ai_lookups` switch. ABSENT, never `[]`, on a server with no source, and on one
    /// that predates it (docs/protocol.md, "Looking things up"); read through the lookup rules,
    /// which treat an empty or blank list as absent too. A `null` inside is dropped as it is read.
    #[serde(
        default,
        deserialize_with = "wire::names",
        skip_serializing_if = "Option::is_none"
    )]
    pub lookups: Option<Vec<String>>,
    /// Whether the daily greeting can carry today's forecast for the family's `greeting_places`:
    /// true exactly when this server posts greetings and has its weather source on
    /// (docs/protocol.md, "Today's weather, for places the owner chose"). ALWAYS present on a
    /// server that knows it; absent on an older one, which reads as false — no places field.
    /// Bound to no family switch, `ai_lookups` included.
    #[serde(default)]
    pub greeting_weather: bool,
}

/// `POST /me/assistant-consent` — this member's own answer to the assistant question, and nobody
/// else's: there is no shape of this request that names another member (docs/protocol.md,
/// "Consenting to the assistant").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AssistantConsentRequest {
    #[serde(default)]
    pub granted: bool,
}

/// What the server now holds: a stamp when granted, `None` when withdrawn.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AssistantConsentResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_consent_at: Option<String>,
}

/// `POST /me/assistant-lookup-consent`'s answer: the lookup stamp the server now holds, `None`
/// when withdrawn (docs/protocol.md, "Consenting to the assistant", amended 2026-10-03).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AssistantLookupConsentResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_lookup_consent_at: Option<String>,
}

// ---- the family's own console --------------------------------------------

/// A change to the family, by its owner. EVERY field is optional and which fields are PRESENT
/// decides what changes — sending none of them is a valid no-op.
///
/// **TWO OF THESE ARE THE ONLY PLACES ON THIS WIRE WHERE `null` MEANS SOMETHING A MISSING KEY
/// DOES NOT.** `"language": null` CLEARS the family's language and `"max_members": null` CLEARS
/// the cap, while leaving either key out leaves it alone. The booleans are not such places: each
/// has a real default, absent leaves it alone, and there is nothing for a null to mean — which is
/// why they are `Option<bool>` and the two clearable fields need
/// [`clears_cap`](Self::clears_cap) and [`clears_language`](Self::clears_language) to say "send
/// the null".
///
/// Encoded as the wire takes it — what `ApiClient.PatchFamily` built by hand in `win/`, in the
/// same key order.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct FamilyPatch {
    #[serde(default)]
    pub join_policy: Option<String>,
    #[serde(default)]
    pub max_members: Option<i32>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub ai_history: Option<bool>,
    #[serde(default)]
    pub ai_vision: Option<bool>,
    #[serde(default)]
    pub ai_history_photos: Option<bool>,
    #[serde(default)]
    pub ai_greeting: Option<bool>,
    #[serde(default)]
    pub ai_faces: Option<bool>,
    #[serde(default)]
    pub ai_transcripts: Option<bool>,
    #[serde(default)]
    pub ai_lookups: Option<bool>,
    /// The greeting's places, REPLACING the stored list: `[]` clears it and `None` (the default)
    /// leaves it alone. Not a third place where `null` means something — the server refuses
    /// `"greeting_places": null`, so `None` here is never sent at all (docs/protocol.md, "Today's
    /// weather, for places the owner chose").
    #[serde(default)]
    pub greeting_places: Option<Vec<String>>,
    /// Send `"max_members": null` — clear the cap, rather than leave it alone.
    #[serde(skip)]
    pub clears_cap: bool,
    /// Send `"language": null` — clear it, rather than leave it alone.
    #[serde(skip)]
    pub clears_language: bool,
}

impl Serialize for FamilyPatch {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        put(&mut map, "join_policy", &self.join_policy)?;
        if self.clears_cap {
            map.serialize_entry("max_members", &None::<i32>)?;
        } else {
            put(&mut map, "max_members", &self.max_members)?;
        }
        if self.clears_language {
            map.serialize_entry("language", &None::<String>)?;
        } else {
            put(&mut map, "language", &self.language)?;
        }
        put(&mut map, "ai_history", &self.ai_history)?;
        put(&mut map, "ai_vision", &self.ai_vision)?;
        put(&mut map, "ai_history_photos", &self.ai_history_photos)?;
        put(&mut map, "ai_greeting", &self.ai_greeting)?;
        put(&mut map, "ai_faces", &self.ai_faces)?;
        put(&mut map, "ai_transcripts", &self.ai_transcripts)?;
        put(&mut map, "ai_lookups", &self.ai_lookups)?;
        // A list or nothing: `null` is `validation` for this key, and `[]` is how it is cleared.
        put(&mut map, "greeting_places", &self.greeting_places)?;
        map.end()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FamilyOnlyResponse {
    pub family: FamilyDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MemberResponse {
    pub member: MemberDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ChatResponse {
    pub chat: ChatDto,
}

/// What `DELETE /chats/{id}/members/{user_id}` answers (docs/protocol.md, "Groups"): the group
/// as it now stands when its admin removed somebody else, and — [`chat`](Self::chat) `None` —
/// nothing at all when the caller LEFT, which is a `204` and not "the answer could not be read".
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct GroupMemberAnswer {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat: Option<ChatDto>,
}

impl GroupMemberAnswer {
    /// The caller left: the group is no longer theirs to hold.
    pub fn left(&self) -> bool {
        self.chat.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct UserResponse {
    pub user: UserDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct InviteCodeResponse {
    #[serde(default, deserialize_with = "wire::text")]
    pub invite_code: String,
}

/// `POST /families/join`'s answer.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct JoinAnswer {
    /// `joined` or `pending`.
    #[serde(default, deserialize_with = "wire::text")]
    pub status: String,
    /// The family the code belongs to — joined, or waited on (docs/protocol.md, `POST
    /// /families/join`, 2026-10-22). Absent from a server that predates several families.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family_id: Option<i64>,
}

impl JoinAnswer {
    /// Membership was immediate: the family's policy is `open`.
    pub fn joined(&self) -> bool {
        self.status == "joined"
    }

    /// A request was created and an owner has to answer it.
    pub fn pending(&self) -> bool {
        self.status == "pending"
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct JoinRequestDto {
    #[serde(default)]
    pub id: i64,
    pub user: UserDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct JoinRequestsResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requests: Option<Vec<JoinRequestDto>>,
}

/// Who left, when an OWNER did. Absent means nobody remained and the family went with them —
/// which is a different dialog and a different confirmation.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LeftAnswer {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_owner_user_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ReportDto {
    #[serde(default)]
    pub id: i64,
    pub reporter: UserDto,
    pub reported: UserDto,
    #[serde(default, deserialize_with = "wire::text")]
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    /// Dropped once retention has swept that message; the excerpt stays.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<i64>,
    /// The WHOLE body, frozen when the report was raised and never cut — a moderator judging a
    /// message has to see all of it. The one quotation in this protocol that is STORED.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_excerpt: Option<String>,
    /// Trimmed as a chat-list preview is — `{"kind", "name"}`, NO id — so each reads `id` 0, as
    /// C#'s decoder filled it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_attachments: Option<Vec<AttachmentDto>>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ReportResponse {
    pub report: ReportDto,
}

/// What a member said the ASSISTANT got wrong (docs/protocol.md, "Reporting the assistant"). The
/// OPERATOR's row: it is returned to the reporter's own app so it can show the report was taken,
/// and appears in no other read — the family owner's inbox included.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AssistantReportDto {
    #[serde(default)]
    pub id: i64,
    /// The reply reported, while it lasts. Retention drops it; the excerpt outlives it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<i64>,
    /// The WHOLE reply, frozen when the report was raised.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_excerpt: Option<String>,
    /// `ai` for the reporter's private thread, `family` for an `@ai` answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// The reporter's own words, when they wrote any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AssistantReportResponse {
    pub report: AssistantReportDto,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ReportsResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reports: Option<Vec<ReportDto>>,
}

/// What the family has actually sent — the same numbers for everybody, except that a member the
/// CALLER has blocked is left out of the rows.
///
/// **THE ROWS DO NOT ADD UP TO THE TOTALS, AND THE GAP IS THE BLOCK.** The totals are the
/// family's numbers and are not projected per caller; the rows are what this caller may see of
/// them. A client must never derive a total by summing the rows, nor a member's share by dividing
/// into that sum (docs/protocol.md, "Family statistics").
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct StatsResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_at: Option<String>,
    pub totals: StatsTotalsDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub members: Option<Vec<StatsMemberDto>>,
    /// What a PAID family has used of its limits (docs/protocol.md, "Family statistics", amended
    /// 2026-10-30): absent for a free family and on a server with subscriptions off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowance: Option<AllowanceDto>,
}

/// A paid family's allowance: each limit present only while the operator has it on, and the
/// moment the MONTHLY ones start again (the 1st of the next month, UTC). `storage_bytes` counts
/// everything the family keeps on the disk, so it can differ from the statistics' `stored_bytes`
/// and neither is wrong.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AllowanceDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub renews_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_bytes: Option<AllowanceUseDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_tokens: Option<AllowanceUseDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<AllowanceUseDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript_ms: Option<AllowanceUseDto>,
}

/// One limit of an [`AllowanceDto`]: what is used of it, and the limit itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AllowanceUseDto {
    #[serde(default)]
    pub used: i64,
    #[serde(default)]
    pub limit: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct StatsTotalsDto {
    #[serde(default)]
    pub members: i32,
    #[serde(default)]
    pub messages: i64,
    #[serde(default)]
    pub board_notes: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachments: Option<StatsMediaDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai: Option<StatsAiDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct StatsMemberDto {
    #[serde(default)]
    pub user_id: i64,
    #[serde(default, deserialize_with = "wire::text")]
    pub display_name: String,
    #[serde(default)]
    pub messages: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachments: Option<StatsMediaDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai: Option<StatsAiDto>,
}

/// `bytes` adds up what was SENT; `stored_bytes` counts each distinct file once, because
/// identical bytes are stored once per family — the difference is what dedup saved, and it is a
/// family total only.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct StatsMediaDto {
    #[serde(default)]
    pub count: i32,
    #[serde(default)]
    pub photo: i32,
    #[serde(default)]
    pub video: i32,
    #[serde(default)]
    pub audio: i32,
    #[serde(default)]
    pub file: i32,
    #[serde(default)]
    pub location: i32,
    #[serde(default)]
    pub bytes: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stored_bytes: Option<i64>,
}

/// `images` counts the pictures the assistant GENERATED and is its own number rather than a share
/// of `questions`: an image model reports no tokens, so a family reading only the token counts
/// would see the expensive half of the assistant as free.
///
/// `transcripts` and `transcript_duration_ms` are the recordings turned into text and their
/// length: billed by audio length, not tokens, and charged to the member who ASKED. A kept answer
/// handed out again counts nothing.
///
/// `searches` counts the PAID web searches the assistant made that came back with an answer,
/// charged to the member who asked (docs/protocol.md, "Family statistics", amended 2026-10-03).
/// Weather and Wikipedia are free and not counted; an older server omits it, which reads as 0.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct StatsAiDto {
    #[serde(default)]
    pub questions: i32,
    #[serde(default)]
    pub prompt_tokens: i64,
    #[serde(default)]
    pub completion_tokens: i64,
    #[serde(default)]
    pub images: i32,
    #[serde(default)]
    pub transcripts: i32,
    #[serde(default)]
    pub transcript_duration_ms: i64,
    #[serde(default)]
    pub searches: i32,
}

/// `POST /chats/{id}/messages/{mid}/attachments/{aid}/transcript`'s answer (docs/protocol.md,
/// "Transcripts on request"). `text` is always present and `""` is SILENCE — an answer, drawn as
/// "No speech", never an error; `language` only when the provider named one, spelled as the
/// provider spells it.
///
/// `text` is an `Option` because a malformed answer can lack it — a missing key and a
/// `"text": null` alike read as `None` — not because the protocol allows it. The one reader
/// (`TranscriptModel`) refuses such an answer as a failure; it is never read as `""`, which would
/// keep a malformed answer on this device for ever and draw it as "No speech".
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TranscriptDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TranscriptResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript: Option<TranscriptDto>,
}
