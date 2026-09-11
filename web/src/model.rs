//! The wire objects this client reads, exactly as docs/protocol.md defines
//! them.
//!
//! Every field this client does not use is simply absent from these structs
//! — serde ignores what it was not asked for, which is what the protocol's
//! first compatibility rule requires of every client: "MUST ignore unknown
//! JSON fields". Adding a field here is how this client starts caring about
//! one, and until then a newer server changes nothing.
//!
//! Optionality is the protocol's, not convenience: a field that is `Option`
//! here is one the protocol says may be absent, and a field that is not is
//! one it says is always present.

use fc_text::i18n::t;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub display_name: String,
    /// Present, and true, only on an account that has been deleted — whose
    /// `display_name` is then the English placeholder a client should
    /// replace with its own words.
    #[serde(default)]
    pub deleted: bool,
    /// How many times this person has set a profile picture: 0 is none, and
    /// initials are drawn. A cache key, never reused for another picture.
    #[serde(default)]
    pub avatar_version: i64,
    /// A day and a month, never a year — present only when one is set.
    #[serde(default)]
    pub birthday: Option<Birthday>,
}

/// A birthday: a day and a month, and deliberately no year — nobody should
/// have to publish their age to be wished a happy birthday (docs/protocol.md,
/// "Birthdays"). 29 February is a perfectly good one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
pub struct Birthday {
    pub month: u32,
    pub day: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
pub struct Family {
    pub id: i64,
    pub name: String,
    /// `open` | `approval` | `closed` — what the invite code does.
    #[serde(default = "open")]
    pub join_policy: String,
    /// Present for the OWNER only.
    #[serde(default)]
    pub invite_code: Option<String>,
    /// The family's own cap, when the owner has set one. Absent is NOT the
    /// server's ceiling: it is "we never set one" (docs/protocol.md, Family).
    #[serde(default)]
    pub max_members: Option<i64>,
    /// The one language the family speaks, when the owner has chosen one.
    /// Absent is unset — and unset is not English.
    #[serde(default)]
    pub language: Option<String>,
    /// Whether an `@ai` mention may be sent the chat's recent words.
    /// ALWAYS present, and true by default — absent only from a server
    /// that predates it, where it was always so.
    #[serde(default = "yes")]
    pub ai_history: bool,
    /// Whether a photograph may be shown to the assistant at all. False
    /// unless the owner turned it on (docs/protocol.md, "Pictures").
    #[serde(default)]
    pub ai_vision: bool,
    /// Whether a mention may also be shown the chat's recent photos.
    #[serde(default)]
    pub ai_history_photos: bool,
    /// Whether the assistant says its one unprompted good morning.
    #[serde(default)]
    pub ai_greeting: bool,
    /// Whether a mention may be shown the members' profile pictures.
    #[serde(default)]
    pub ai_faces: bool,
}

fn yes() -> bool {
    true
}

fn open() -> String {
    "open".to_string()
}

/// The caller's own join request, while one is waiting on an owner.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PendingJoin {
    pub family_id: i64,
    pub family_name: String,
    #[serde(default)]
    pub created_at: Option<String>,
}

/// A trimmed `GET /me`: who this is, whether they are in a family, and the
/// few server switches this client acts on.
///
/// `family` is null for an account that has not joined one — a real state
/// this client has to draw rather than a failure.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
pub struct Me {
    pub user: User,
    #[serde(default)]
    pub family: Option<Family>,
    /// The caller's own block list — the WHOLE of it, replaced rather than
    /// merged on every read (docs/protocol.md, "Blocking a member").
    #[serde(default)]
    pub blocked_user_ids: Vec<i64>,
    /// The operator's published contact, shown as sent and never made a
    /// link (docs/protocol.md, "Reporting a member").
    #[serde(default)]
    pub support_contact: Option<String>,
    /// `owner` | `member`, or absent without a family.
    #[serde(default)]
    pub role: Option<String>,
    /// The caller's live join request, if one is waiting.
    #[serde(default)]
    pub pending_join_request: Option<PendingJoin>,
    /// The operator's ceiling on any family's size — the most an owner's own
    /// cap may be, and the cap for a family that set none.
    #[serde(default)]
    pub max_family_members: Option<i64>,
    /// Whether this server takes NEW families. Absent is true: every server
    /// from before the switch took them (docs/protocol.md, "Starting a
    /// family").
    #[serde(default = "yes")]
    pub family_registration_enabled: bool,
    /// Days an account may go without a family before the server removes
    /// it; 0 when the sweep is off, and when a server predates it.
    #[serde(default)]
    pub familyless_account_ttl_days: i64,
    /// Whether this server signals calls at all, and video calls beside
    /// them. ALWAYS present; absent is a server from before calls, and a
    /// button this client does not offer (docs/protocol.md, "Voice calls").
    #[serde(default)]
    pub calls_enabled: bool,
    #[serde(default)]
    pub video_calls_enabled: bool,
    /// Whether this server posts the assistant's daily greeting at all —
    /// the operator's half of the switch whose family half is
    /// `Family::ai_greeting`. Absent is off: a server from before it posts
    /// none.
    #[serde(default)]
    pub greetings_enabled: bool,
}

impl Me {
    pub fn is_owner(&self) -> bool {
        self.role.as_deref() == Some("owner")
    }
}

/// One person in `GET /families/mine`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
pub struct Member {
    pub id: i64,
    pub display_name: String,
    #[serde(default)]
    pub username: String,
    /// `owner` | `member`; absent on a former member.
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default)]
    pub avatar_version: i64,
    #[serde(default)]
    pub birthday: Option<Birthday>,
}

impl Member {
    pub fn is_owner(&self) -> bool {
        self.role.as_deref() == Some("owner")
    }
}

/// The family's assistant, which speaks in the chat under an account of
/// its own and so needs a name like anybody else. ABSENT when the server
/// has none — and that absence is the whole capability check: no
/// `assistant`, no `@ai` offered (docs/protocol.md, "The assistant").
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Assistant {
    pub user_id: i64,
    pub display_name: String,
    /// The token that asks for it in the family chat — `@ai`.
    #[serde(default)]
    pub mention: Option<String>,
    /// The token that asks it for a picture — `/draw`.
    #[serde(default)]
    pub draw: Option<String>,
    /// Whether this SERVER can look at a picture.
    #[serde(default)]
    pub vision: bool,
    /// Whether this SERVER can make one.
    #[serde(default)]
    pub images: bool,
}

/// `GET /families/mine`, trimmed to what this client draws.
///
/// `former_members` is there for exactly this: the messages somebody left
/// behind when their account was deleted still need a name on them. It is
/// omitted when there are none.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
pub struct Roster {
    pub members: Vec<Member>,
    #[serde(default)]
    pub former_members: Vec<Member>,
    #[serde(default)]
    pub assistant: Option<Assistant>,
    #[serde(default)]
    pub blocked_user_ids: Vec<i64>,
    /// The board's high-water mark — which lives only here, and says whether
    /// there is anything past this client's board cursor to catch up on.
    #[serde(default)]
    pub max_board_seq: i64,
    /// The family itself — the owner's invite code and switches with it.
    #[serde(default)]
    pub family: Option<Family>,
    /// For the OWNER: who would inherit the family if they left right now.
    /// A prediction, read fresh before the leave dialog names anybody.
    #[serde(default)]
    pub next_owner_user_id: Option<i64>,
}

/// Somebody asking to join, for the owner (`GET /families/join-requests`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct JoinRequest {
    pub id: i64,
    pub user: User,
    #[serde(default)]
    pub created_at: Option<String>,
}

/// One open report in the owner's inbox (docs/protocol.md, "Reporting a
/// member"). `message_excerpt` is the WHOLE body, frozen when it was
/// reported, and never cut.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Report {
    pub id: i64,
    pub reporter: User,
    pub reported: User,
    pub reason: String,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub message_id: Option<i64>,
    #[serde(default)]
    pub message_excerpt: Option<String>,
    /// What the reported message carried, trimmed as a chat-list preview
    /// is — for a caption-less photo, the only thing on the row that says
    /// what was reported.
    #[serde(default)]
    pub message_attachments: Vec<ReportedAttachment>,
}

/// One attachment of a reported message: its kind and name, no bytes.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ReportedAttachment {
    pub kind: String,
    #[serde(default)]
    pub name: Option<String>,
}

/// One ICE candidate, as a call frame carries it (docs/protocol.md, "Voice
/// calls"). The names are the wire's; the browser's own spellings
/// (`sdpMid`, `sdpMLineIndex`) are translated at the peer connection.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
pub struct IceCandidate {
    pub candidate: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub sdp_mid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub sdp_mline_index: Option<u16>,
}

/// One STUN or TURN server from `GET /calls/ice`, with the credentials the
/// operator minted for this caller when there are any.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
pub struct IceServer {
    #[serde(default)]
    pub urls: Vec<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub credential: Option<String>,
}

/// What was sent, by kind (`GET /families/mine/stats`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
pub struct AttachmentCounts {
    #[serde(default)]
    pub count: i64,
    #[serde(default)]
    pub photo: i64,
    #[serde(default)]
    pub video: i64,
    #[serde(default)]
    pub audio: i64,
    #[serde(default)]
    pub file: i64,
    #[serde(default)]
    pub location: i64,
    #[serde(default)]
    pub bytes: i64,
    /// Each distinct file once — a family total only.
    #[serde(default)]
    pub stored_bytes: Option<i64>,
}

/// What the assistant was asked, and what it cost.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
pub struct AiCounts {
    #[serde(default)]
    pub questions: i64,
    #[serde(default)]
    pub prompt_tokens: i64,
    #[serde(default)]
    pub completion_tokens: i64,
    #[serde(default)]
    pub images: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
pub struct StatsTotals {
    #[serde(default)]
    pub members: i64,
    #[serde(default)]
    pub messages: i64,
    #[serde(default)]
    pub board_notes: i64,
    #[serde(default)]
    pub attachments: AttachmentCounts,
    #[serde(default)]
    pub ai: AiCounts,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct MemberStats {
    pub user_id: i64,
    pub display_name: String,
    #[serde(default)]
    pub messages: i64,
    #[serde(default)]
    pub attachments: AttachmentCounts,
    #[serde(default)]
    pub ai: AiCounts,
}

/// The family's numbers. The rows do NOT add up to the totals when the
/// reader has blocked somebody, and the gap is the block: never sum them.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Stats {
    #[serde(default)]
    pub generated_at: Option<String>,
    pub totals: StatsTotals,
    #[serde(default)]
    pub members: Vec<MemberStats>,
}

/// One answer to an event.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
pub struct Rsvp {
    pub user_id: i64,
    /// `going` | `maybe` | `no` — kept as sent, so an answer from a newer
    /// server is not mistaken for one of these (fc_text::board::Answer).
    pub answer: String,
}

/// A sticker on the family board (docs/protocol.md, "Objects" and "Board").
///
/// A TOMBSTONE carries only `id`, `deleted` and `board_seq`, which is why
/// everything else is optional here; a live note always has its author,
/// text, colour and position, and one that arrives without them is a server
/// fault the board refuses rather than drawing a blank. `size`, `font` and
/// `kind` are absent only from a server that predates them.
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct Note {
    pub id: i64,
    pub board_seq: i64,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default)]
    pub author_id: Option<i64>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub size: Option<String>,
    #[serde(default)]
    pub font: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    /// The picture: a photo note's content, and an event's backdrop.
    #[serde(default)]
    pub attachment: Option<Attachment>,
    /// An event's own three.
    #[serde(default)]
    pub starts_at: Option<String>,
    #[serde(default)]
    pub ends_at: Option<String>,
    #[serde(default)]
    pub place: Option<String>,
    /// Present, possibly empty, on every event; absent on every other kind.
    #[serde(default)]
    pub rsvps: Option<Vec<Rsvp>>,
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
    #[serde(default)]
    pub created_at: Option<String>,
    /// The `board_seq` of the last change to what the note SAYS — what the
    /// badge counts. Absent on a tombstone and from an older server.
    #[serde(default)]
    pub content_seq: Option<i64>,
}

impl Note {
    /// Whether this copy can be drawn: a tombstone cannot, and neither can a
    /// live note missing what every live note has.
    pub fn is_drawable(&self) -> bool {
        !self.deleted
            && self.author_id.is_some()
            && self.text.is_some()
            && self.color.is_some()
            && self.x.is_some_and(f64::is_finite)
            && self.y.is_some_and(f64::is_finite)
    }

    pub fn text(&self) -> &str {
        self.text.as_deref().unwrap_or_default()
    }

    pub fn kind(&self) -> fc_text::board::Kind {
        fc_text::board::Kind::from_name(self.kind.as_deref())
    }

    pub fn size(&self) -> fc_text::board::Size {
        fc_text::board::Size::from_name(self.size.as_deref())
    }

    pub fn font(&self) -> fc_text::board::Font {
        fc_text::board::Font::from_name(self.font.as_deref())
    }

    pub fn position(&self) -> (f64, f64) {
        (self.x.unwrap_or_default(), self.y.unwrap_or_default())
    }

    pub fn rsvps(&self) -> &[Rsvp] {
        self.rsvps.as_deref().unwrap_or(&[])
    }

    /// What `user_id` answered, as sent.
    pub fn answer_of(&self, user_id: i64) -> Option<&str> {
        self.rsvps()
            .iter()
            .find(|rsvp| rsvp.user_id == user_id)
            .map(|rsvp| rsvp.answer.as_str())
    }

    /// How many gave `answer`.
    pub fn count(&self, answer: fc_text::board::Answer) -> usize {
        self.rsvps()
            .iter()
            .filter(|rsvp| rsvp.answer == answer.name())
            .count()
    }
}

impl Roster {
    /// Everybody this family's messages can be from, by user id.
    pub fn names(&self) -> Vec<(i64, String)> {
        let assistant = self
            .assistant
            .as_ref()
            .map(|assistant| (assistant.user_id, assistant.display_name.clone()));
        self.former_members
            .iter()
            .chain(self.members.iter())
            .map(|member| (member.id, member.display_name.clone()))
            .chain(assistant)
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Chat {
    pub id: i64,
    /// `family` | `direct` | `ai`.
    pub kind: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub peer_user_id: Option<i64>,
}

impl Chat {
    pub const FAMILY: &'static str = "family";
    pub const DIRECT: &'static str = "direct";
    pub const AI: &'static str = "ai";

    pub fn is_family(&self) -> bool {
        self.kind == Self::FAMILY
    }

    pub fn is_direct(&self) -> bool {
        self.kind == Self::DIRECT
    }

    pub fn is_ai(&self) -> bool {
        self.kind == Self::AI
    }

    /// What to call this chat in a list.
    ///
    /// The family chat carries the family's name as its title; a direct
    /// chat carries none, and the caller supplies the peer's name from the
    /// roster — which is why this takes it rather than inventing one.
    pub fn display_title(&self, peer_name: Option<&str>) -> String {
        if let Some(title) = self.title.as_ref().filter(|title| !title.is_empty()) {
            return title.clone();
        }
        peer_name.unwrap_or_else(|| t("Chat")).to_string()
    }
}

/// One row of `GET /chats`.
///
/// `last_read_message_id` is ALWAYS present (the protocol says so
/// explicitly, and `0` is a real answer meaning "never reported"), so it is
/// not an Option. `unread_count` likewise. The three `max_*_seq` are
/// high-water marks, omitted until the first reaction, edit or poll.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ChatListItem {
    pub chat: Chat,
    #[serde(default)]
    pub last_message: Option<Message>,
    pub unread_count: i64,
    pub last_read_message_id: i64,
    #[serde(default)]
    pub max_reaction_seq: Option<i64>,
    #[serde(default)]
    pub max_edit_seq: Option<i64>,
    #[serde(default)]
    pub max_poll_seq: Option<i64>,
    /// An unread message here names the reader. Absent, never false.
    #[serde(default)]
    pub mentioned: bool,
}

/// The quote a reply carries, RECOMPUTED by the server on every read —
/// with exactly one more level under it when the quoted message was itself
/// a reply (docs/protocol.md, "Replies").
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
pub struct ReplyTo {
    pub message_id: i64,
    pub sender_id: i64,
    pub excerpt: String,
    #[serde(default)]
    pub parent: Option<ReplyParent>,
}

/// The second level of a quote, which by construction has no third.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
pub struct ReplyParent {
    pub message_id: i64,
    pub sender_id: i64,
    pub excerpt: String,
}

/// A member named in a body — `name` exactly AS TYPED after the `@`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
pub struct Mention {
    pub user_id: i64,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
pub struct Reaction {
    pub user_id: i64,
    pub emoji: String,
}

/// An attachment's metadata. The bytes are fetched separately, and only
/// what a kind actually has is present: dimensions on photos and videos, a
/// duration on video and audio, a name on files, coordinates on locations.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
pub struct Attachment {
    pub id: i64,
    /// `photo` | `video` | `audio` | `file` | `location`.
    pub kind: String,
    #[serde(default)]
    pub mime: Option<String>,
    #[serde(default)]
    pub size: Option<i64>,
    #[serde(default)]
    pub width: Option<i64>,
    #[serde(default)]
    pub height: Option<i64>,
    #[serde(default)]
    pub duration_ms: Option<i64>,
    #[serde(default)]
    pub has_preview: bool,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub latitude: Option<f64>,
    #[serde(default)]
    pub longitude: Option<f64>,
    #[serde(default)]
    pub accuracy_m: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
pub struct PollOption {
    pub id: i64,
    pub text: String,
    /// Everybody who chose this option — full state, never a delta.
    pub votes: Vec<i64>,
}

/// A poll's options and votes. The QUESTION is the message body.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
pub struct Poll {
    pub poll_seq: i64,
    pub closed: bool,
    pub options: Vec<PollOption>,
}

/// The record of a call, whose body is an English placeholder this client
/// never shows (docs/protocol.md, "Voice calls").
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
pub struct Call {
    /// `completed` | `missed` | `declined` | `failed`.
    pub outcome: String,
    #[serde(default)]
    pub duration_secs: Option<i64>,
    #[serde(default)]
    pub video: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, Default)]
pub struct Message {
    pub id: i64,
    pub chat_id: i64,
    pub sender_id: i64,
    #[serde(default)]
    pub client_msg_id: Option<String>,
    pub body: String,
    pub created_at: String,
    /// Present only on an edited message, with `edit_seq` beside it — the
    /// value that guards applying an edit (docs/protocol.md, "Editing").
    #[serde(default)]
    pub edited_at: Option<String>,
    #[serde(default)]
    pub edit_seq: Option<i64>,
    /// Present once the message has ever been reacted to; `[]` after the
    /// last reaction goes, which is "cleared", not "no data".
    #[serde(default)]
    pub reactions: Option<Vec<Reaction>>,
    #[serde(default)]
    pub reaction_seq: Option<i64>,
    #[serde(default)]
    pub reply_to: Option<ReplyTo>,
    #[serde(default)]
    pub thread_root_id: Option<i64>,
    /// On a ROOT with at least one reply: how many name it as their root.
    #[serde(default)]
    pub reply_count: Option<i64>,
    #[serde(default)]
    pub mentions: Option<Vec<Mention>>,
    #[serde(default)]
    pub attachments: Option<Vec<Attachment>>,
    #[serde(default)]
    pub poll: Option<Poll>,
    #[serde(default)]
    pub call: Option<Call>,
}

impl Message {
    pub fn is_edited(&self) -> bool {
        self.edited_at.is_some()
    }

    /// The edit guard's value: absent counts as 0 (docs/protocol.md).
    pub fn edit_seq(&self) -> i64 {
        self.edit_seq.unwrap_or(0)
    }

    pub fn attachments(&self) -> &[Attachment] {
        self.attachments.as_deref().unwrap_or(&[])
    }

    pub fn reactions(&self) -> &[Reaction] {
        self.reactions.as_deref().unwrap_or(&[])
    }

    pub fn mentions(&self) -> &[Mention] {
        self.mentions.as_deref().unwrap_or(&[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    /// The protocol's first compatibility rule, as a test: a newer server
    /// sends fields this client has never heard of, and it reads the
    /// message anyway.
    #[wasm_bindgen_test]
    fn unknown_fields_are_ignored() {
        let json = r#"{
            "id": 1338, "chat_id": 42, "sender_id": 7, "client_msg_id": null,
            "body": "@Anna are you in?", "created_at": "2026-08-19T17:03:12Z",
            "mentions": [{"user_id": 9, "name": "Anna"}],
            "thread_root_id": 1337, "reply_count": 2,
            "something_invented_next_year": {"deeply": ["nested"]}
        }"#;
        let message: Message = serde_json::from_str(json).expect("a message this client can read");
        assert_eq!(message.id, 1338);
        assert_eq!(message.body, "@Anna are you in?");
        assert_eq!(message.mentions()[0].name, "Anna");
        assert_eq!(message.thread_root_id, Some(1337));
    }

    /// Everything a message can carry, in the protocol's own shapes.
    #[wasm_bindgen_test]
    fn a_full_message_reads_every_part() {
        let json = r#"{
            "id": 1340, "chat_id": 42, "sender_id": 9, "client_msg_id": "8f14e45f",
            "body": "Pizza or pasta?", "created_at": "2026-08-19T17:03:12Z",
            "edited_at": "2026-08-19T17:04:00Z", "edit_seq": 88,
            "reactions": [{"user_id": 7, "emoji": "❤️"}], "reaction_seq": 123,
            "reply_to": {"message_id": 41, "sender_id": 9, "excerpt": "See you at six",
                         "parent": {"message_id": 38, "sender_id": 4, "excerpt": "What time?"}},
            "attachments": [{"id": 34, "kind": "photo", "mime": "image/jpeg", "size": 182734,
                             "width": 1600, "height": 1200, "has_preview": true}],
            "attachment": {"id": 34, "kind": "photo"},
            "poll": {"poll_seq": 89, "closed": false,
                     "options": [{"id": 5, "text": "Pizza", "votes": [7, 9]},
                                 {"id": 6, "text": "Pasta", "votes": []}]},
            "call": {"outcome": "completed", "duration_secs": 222, "video": true}
        }"#;
        let message: Message = serde_json::from_str(json).expect("reads");
        assert_eq!(message.edit_seq(), 88);
        assert_eq!(message.reactions()[0].emoji, "❤️");
        let quote = message.reply_to.as_ref().expect("a quote");
        assert_eq!(quote.parent.as_ref().map(|p| p.message_id), Some(38));
        assert_eq!(message.attachments()[0].width, Some(1600));
        let poll = message.poll.as_ref().expect("a poll");
        assert_eq!(poll.options[0].votes, vec![7, 9]);
        let call = message.call.as_ref().expect("a call record");
        assert!(call.video);
        assert_eq!(call.duration_secs, Some(222));
    }

    /// A message in its first form has none of it, and says so by absence.
    #[wasm_bindgen_test]
    fn a_plain_message_has_none_of_it() {
        let json = r#"{"id": 1, "chat_id": 42, "sender_id": 7, "body": "hi",
                       "created_at": "2026-08-19T17:03:12Z"}"#;
        let message: Message = serde_json::from_str(json).expect("reads");
        assert!(!message.is_edited());
        assert_eq!(message.edit_seq(), 0, "absent counts as 0 for the guard");
        assert!(
            message.reactions.is_none(),
            "no data, which is not the same as cleared"
        );
        assert!(message.attachments().is_empty());
        assert!(message.poll.is_none() && message.call.is_none() && message.reply_to.is_none());
    }

    #[wasm_bindgen_test]
    fn a_chat_row_reads_its_cursors_and_mention_mark() {
        let json = r#"{"chat": {"id": 42, "kind": "family", "title": "The Smiths"},
                       "last_message": null, "unread_count": 3, "last_read_message_id": 1337,
                       "max_reaction_seq": 123, "mentioned": true}"#;
        let item: ChatListItem = serde_json::from_str(json).expect("reads");
        assert_eq!(item.max_reaction_seq, Some(123));
        assert_eq!(item.max_edit_seq, None, "never edited");
        assert!(item.mentioned);
        assert!(item.chat.is_family());

        let quiet = r#"{"chat": {"id": 43, "kind": "direct", "peer_user_id": 9},
                        "unread_count": 0, "last_read_message_id": 0}"#;
        let item: ChatListItem = serde_json::from_str(quiet).expect("reads");
        assert!(!item.mentioned, "absent is false");
    }

    /// The roster names everybody a message can be from: the members, the
    /// former members whose messages are still there, and the assistant.
    #[wasm_bindgen_test]
    fn the_roster_names_members_former_members_and_the_assistant() {
        let json = r#"{
            "family": {"id": 3, "name": "The Smiths", "join_policy": "open"},
            "members": [
                {"id": 7, "username": "me", "display_name": "Me", "role": "owner", "avatar_version": 0},
                {"id": 9, "username": "anna", "display_name": "Anna", "role": "member",
                 "avatar_version": 3, "birthday": {"month": 3, "day": 14}}
            ],
            "former_members": [
                {"id": 4, "username": "deleted-4", "display_name": "Gran", "avatar_version": 0,
                 "deleted": true}
            ],
            "max_board_seq": 88,
            "assistant": {"user_id": 2, "display_name": "Assistant", "mention": "@ai",
                          "draw": "/draw", "vision": false, "images": true},
            "blocked_user_ids": [4]
        }"#;
        let roster: Roster = serde_json::from_str(json).expect("a roster this client can read");
        let names: std::collections::HashMap<i64, String> = roster.names().into_iter().collect();
        assert_eq!(names.get(&9).map(String::as_str), Some("Anna"));
        assert_eq!(names.get(&4).map(String::as_str), Some("Gran"));
        assert_eq!(names.get(&2).map(String::as_str), Some("Assistant"));
        assert_eq!(names.len(), 4);
        let assistant = roster.assistant.as_ref().expect("an assistant");
        assert_eq!(assistant.mention.as_deref(), Some("@ai"));
        assert!(assistant.images && !assistant.vision);
        assert_eq!(roster.blocked_user_ids, vec![4]);
        assert_eq!(roster.members[0].role.as_deref(), Some("owner"));
        assert!(roster.former_members[0].deleted);

        // No former members and no assistant is an ordinary family too.
        let bare: Roster = serde_json::from_str(r#"{"members": []}"#).expect("reads");
        assert!(bare.names().is_empty());
    }

    /// A note in each of its shapes — and a tombstone, which carries nothing
    /// but its id, its seq and that it is gone.
    #[wasm_bindgen_test]
    fn a_note_reads_every_shape_the_protocol_gives_it() {
        let event: Note = serde_json::from_str(
            r#"{"id": 12, "author_id": 7, "kind": "event", "text": "Picnic", "color": "blue",
                "size": "large", "font": "casual", "x": 0.42, "y": 0.13,
                "created_at": "2026-09-10T10:00:00Z", "updated_at": "2026-09-10T10:00:00Z",
                "board_seq": 88, "content_seq": 84, "starts_at": "2026-09-12T11:00:00Z",
                "place": "The park", "rsvps": [{"user_id": 9, "answer": "going"},
                                               {"user_id": 11, "answer": "perhaps"}],
                "invented": true}"#,
        )
        .expect("an event this client can read");
        assert!(event.is_drawable());
        assert_eq!(event.kind(), fc_text::board::Kind::Event);
        assert_eq!(event.size(), fc_text::board::Size::Large);
        assert_eq!(event.font(), fc_text::board::Font::Casual);
        assert_eq!(event.content_seq, Some(84));
        assert_eq!(event.answer_of(9), Some("going"));
        assert_eq!(
            event.answer_of(7),
            None,
            "the author's answer is not assumed"
        );
        assert_eq!(event.count(fc_text::board::Answer::Going), 1);
        assert_eq!(
            event.count(fc_text::board::Answer::Maybe),
            0,
            "an answer from a newer server is not a maybe"
        );

        let photo: Note = serde_json::from_str(
            r#"{"id": 13, "author_id": 9, "kind": "photo", "text": "", "color": "yellow",
                "x": 0.1, "y": 0.2, "board_seq": 90,
                "attachment": {"id": 34, "kind": "photo", "has_preview": true}}"#,
        )
        .expect("reads");
        assert!(photo.is_drawable(), "an empty caption is still a note");
        assert_eq!(photo.attachment.as_ref().map(|a| a.id), Some(34));
        assert_eq!(
            photo.size(),
            fc_text::board::Size::Medium,
            "an older server's note"
        );
        assert!(photo.rsvps().is_empty());

        let gone: Note = serde_json::from_str(r#"{"id": 12, "deleted": true, "board_seq": 91}"#)
            .expect("a tombstone reads");
        assert!(gone.deleted && !gone.is_drawable());

        let broken: Note =
            serde_json::from_str(r#"{"id": 14, "board_seq": 92, "text": "x"}"#).expect("reads");
        assert!(
            !broken.is_drawable(),
            "a live note with no author or place is refused"
        );
    }

    #[wasm_bindgen_test]
    fn the_roster_carries_the_boards_high_water_mark() {
        let roster: Roster =
            serde_json::from_str(r#"{"members": [], "max_board_seq": 88}"#).expect("reads");
        assert_eq!(roster.max_board_seq, 88);
        let older: Roster = serde_json::from_str(r#"{"members": []}"#).expect("reads");
        assert_eq!(older.max_board_seq, 0);
    }

    #[wasm_bindgen_test]
    fn a_chat_titles_itself_and_falls_back_to_the_peer() {
        let family = Chat {
            id: 42,
            kind: "family".into(),
            title: Some("The Smiths".into()),
            peer_user_id: None,
        };
        assert_eq!(family.display_title(None), "The Smiths");
        let direct = Chat {
            id: 43,
            kind: "direct".into(),
            title: None,
            peer_user_id: Some(9),
        };
        assert_eq!(direct.display_title(Some("Anna")), "Anna");
        // A direct chat whose peer is not in the roster yet still draws as
        // something rather than as an empty row.
        assert_eq!(direct.display_title(None), "Chat");
        // An EMPTY title is not a title: the server sends one for a family
        // chat, and a family named "" would otherwise draw a blank row.
        let empty = Chat {
            id: 44,
            kind: "direct".into(),
            title: Some(String::new()),
            peer_user_id: Some(9),
        };
        assert_eq!(empty.display_title(Some("Anna")), "Anna");
    }

    #[wasm_bindgen_test]
    fn me_reads_the_block_list_and_the_support_contact() {
        let json = r#"{"user": {"id": 7, "username": "me", "display_name": "Me"},
                       "family": {"id": 3, "name": "The Smiths"},
                       "blocked_user_ids": [9, 11], "support_contact": "ops@example.com"}"#;
        let me: Me = serde_json::from_str(json).expect("reads");
        assert_eq!(me.blocked_user_ids, vec![9, 11]);
        assert_eq!(me.support_contact.as_deref(), Some("ops@example.com"));
    }
}
