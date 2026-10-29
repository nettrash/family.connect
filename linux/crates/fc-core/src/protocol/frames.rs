//! The realtime frames (docs/protocol.md, "WebSocket protocol"): JSON text messages tagged by
//! `"type"` — a port of `win/src/FamilyConnect.Core/Protocol/Frames.cs`.
//!
//! The parser answers `None` for a frame it does not know, and that is the rule rather than
//! laziness: a newer server may add a type, and a client that failed would drop the connection
//! over something it was free to ignore.
//!
//! Web counterpart: `web/src/socket.rs`. Apple: `SocketFrames`.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::api_error::error_codes;
use super::dtos::{
    ChatDto, MemberDto, MessageDto, NoteDto, PackItemDto, PollDto, ReactionDto, UserDto, wire,
};
use super::requests::InvitationDto;

/// One frame from the server, as this client uses it.
#[derive(Debug, Clone, PartialEq)]
pub enum ServerFrame {
    /// A message that is NEW: it bumps unread counts and may raise a notification.
    Message(MessageDto),

    /// The answer to this device's own `send`. Carries the whole message, so the pending row is
    /// replaced rather than patched.
    Ack {
        client_msg_id: String,
        value: MessageDto,
    },

    /// An edit. A SEPARATE type from [`Message`](Self::Message) on purpose: an edit must not
    /// bump an unread count or raise a notification, and it carries the WHOLE message — the
    /// assistant's picture answer arrives as an attachment added by exactly this frame, and a
    /// body-only merge draws nothing.
    MessageEdited(MessageDto),

    Read {
        chat_id: i64,
        user_id: i64,
        last_read_message_id: i64,
    },

    Typing {
        chat_id: i64,
        user_id: i64,
    },

    /// Complete state, never a delta — `reactions: []` means cleared.
    Reactions {
        chat_id: i64,
        message_id: i64,
        reaction_seq: i64,
        value: Vec<ReactionDto>,
    },

    Poll {
        chat_id: i64,
        message_id: i64,
        value: PollDto,
    },

    BoardNote(NoteDto),

    /// One item of the family's sticker pack in whatever state it now has — added, or a
    /// tombstone. It notifies nobody, counts as unread nowhere, and is not filtered by blocks.
    PackItem(PackItemDto),

    /// Somebody arrived in the family the frame names. `password_reset` rides on the family
    /// OWNER's copy alone — whether they may reset the newcomer's password, exactly as on their
    /// roster — and is absent (`None`) from everybody else's and from a server before several
    /// families (docs/protocol.md, "Several families"). `None` says NOTHING: the store keeps
    /// whatever flag it already holds for that member.
    MemberJoined {
        family_id: Option<i64>,
        user: UserDto,
        password_reset: Option<bool>,
    },

    MemberLeft {
        family_id: Option<i64>,
        user_id: i64,
    },

    MemberDeleted {
        family_id: Option<i64>,
        member: MemberDto,
    },

    FamilyOwner {
        family_id: Option<i64>,
        user_id: i64,
    },

    MemberBlocked {
        user_id: i64,
        blocked: bool,
    },

    /// A GROUP as it now stands — created, renamed, or its members or admin changed — sent to
    /// every member, one just added included, which is how a group appears on their devices at
    /// once (docs/protocol.md, "Groups").
    ChatUpdated(ChatDto),

    /// A GROUP this reader is no longer in: they left or were removed, or it was deleted. Drop
    /// it, its messages and anything being drafted there (docs/protocol.md, "Groups").
    ChatRemoved {
        chat_id: i64,
    },

    /// An invitation into a family, new or changed — to the invited person and to that family's
    /// owner (docs/protocol.md, "Several families").
    Invitation(InvitationDto),

    /// An invitation that lapsed: accepted, declined, withdrawn, or gone with its family or
    /// either account.
    InvitationRemoved {
        invitation_id: i64,
    },

    /// The assistant, mid-reply.
    AiDelta {
        chat_id: i64,
        message_id: i64,
        text: String,
    },

    /// The assistant stopped early — and, when the server knows, WHY (docs/protocol.md, "The
    /// assistant": `ai_error`'s optional `reason`). `None` is every other failure, exactly as it
    /// was before the field existed.
    AiError {
        chat_id: i64,
        message_id: i64,
        reason: Option<AiErrorReason>,
    },

    CallOffer {
        call_id: String,
        chat_id: i64,
        from_user_id: i64,
        sdp: String,
        video: bool,
    },

    CallRinging {
        call_id: String,
    },

    CallAnswer {
        call_id: String,
        sdp: String,
    },

    CallIce {
        call_id: String,
        candidate: IceCandidate,
    },

    CallEnd {
        call_id: String,
        reason: String,
    },

    Pong,

    /// A refusal. `client_msg_id` is present when it answers a `send` and `call_id` when it
    /// answers a call frame — never both.
    ///
    /// The wire calls the sentence `message`; here it is `detail`, because
    /// [`Message`](Self::Message) is already a frame. It is English for a log either way — every
    /// client shows its own wording for a code.
    Error {
        code: String,
        detail: String,
        client_msg_id: Option<String>,
        call_id: Option<String>,
    },
}

impl ServerFrame {
    /// The frame, or `None` when this client has no use for it.
    ///
    /// A field of the WRONG JSON TYPE reads as absent — the frame's own fallback for a missing
    /// field applies, and a frame that cannot stand without that field is no frame. `win/`'s
    /// `GetValue<string>()`/`GetValue<bool>()` threw instead, and its socket loop read the
    /// exception as a broken connection: one malformed frame dropped the socket.
    pub fn parse(json: &str) -> Option<ServerFrame> {
        let node: Value = serde_json::from_str(json).ok()?;
        let frame = node.as_object()?;
        let kind = text(frame, "type")?;
        match kind {
            "message" => decode::<MessageDto>(frame.get("message")).map(ServerFrame::Message),
            "ack" => {
                let acked = decode::<MessageDto>(frame.get("message"))?;
                let id = text(frame, "client_msg_id")?;
                Some(ServerFrame::Ack {
                    client_msg_id: id.to_string(),
                    value: acked,
                })
            }
            "message_edited" => {
                decode::<MessageDto>(frame.get("message")).map(ServerFrame::MessageEdited)
            }
            "read" => Some(ServerFrame::Read {
                chat_id: number(frame.get("chat_id")),
                user_id: number(frame.get("user_id")),
                last_read_message_id: number(frame.get("last_read_message_id")),
            }),
            "typing" => Some(ServerFrame::Typing {
                chat_id: number(frame.get("chat_id")),
                user_id: number(frame.get("user_id")),
            }),
            "reaction" => Some(ServerFrame::Reactions {
                chat_id: number(frame.get("chat_id")),
                message_id: number(frame.get("message_id")),
                reaction_seq: number(frame.get("reaction_seq")),
                value: decode::<Vec<ReactionDto>>(frame.get("reactions")).unwrap_or_default(),
            }),
            "poll" => decode::<PollDto>(frame.get("poll")).map(|poll| ServerFrame::Poll {
                chat_id: number(frame.get("chat_id")),
                message_id: number(frame.get("message_id")),
                value: poll,
            }),
            "board_note" => decode::<NoteDto>(frame.get("note")).map(ServerFrame::BoardNote),
            "pack_item" => decode::<PackItemDto>(frame.get("item")).map(ServerFrame::PackItem),
            "member_joined" => {
                decode::<UserDto>(frame.get("user")).map(|user| ServerFrame::MemberJoined {
                    family_id: optional(frame.get("family_id")),
                    user,
                    password_reset: flag(frame.get("password_reset")),
                })
            }
            "member_left" => Some(ServerFrame::MemberLeft {
                family_id: optional(frame.get("family_id")),
                user_id: number(frame.get("user_id")),
            }),
            "member_deleted" => {
                decode::<MemberDto>(frame.get("member")).map(|member| ServerFrame::MemberDeleted {
                    family_id: optional(frame.get("family_id")),
                    member,
                })
            }
            "family_owner" => Some(ServerFrame::FamilyOwner {
                family_id: optional(frame.get("family_id")),
                user_id: number(frame.get("user_id")),
            }),
            "chat_updated" => decode::<ChatDto>(frame.get("chat"))
                .filter(|chat| chat.id > 0)
                .map(ServerFrame::ChatUpdated),
            "chat_removed" => match number(frame.get("chat_id")) {
                removed if removed > 0 => Some(ServerFrame::ChatRemoved { chat_id: removed }),
                _ => None,
            },
            "invitation" => decode::<InvitationDto>(frame.get("invitation"))
                .filter(|invitation| invitation.id > 0)
                .map(ServerFrame::Invitation),
            "invitation_removed" => match number(frame.get("invitation_id")) {
                lapsed if lapsed > 0 => Some(ServerFrame::InvitationRemoved {
                    invitation_id: lapsed,
                }),
                _ => None,
            },
            "member_blocked" => Some(ServerFrame::MemberBlocked {
                user_id: number(frame.get("user_id")),
                blocked: flag(frame.get("blocked")).unwrap_or(false),
            }),
            "ai_delta" => Some(ServerFrame::AiDelta {
                chat_id: number(frame.get("chat_id")),
                message_id: number(frame.get("message_id")),
                text: text(frame, "text").unwrap_or_default().to_string(),
            }),
            "ai_error" => Some(ServerFrame::AiError {
                chat_id: number(frame.get("chat_id")),
                message_id: number(frame.get("message_id")),
                reason: ai_reason(frame.get("reason")),
            }),
            "call_offer" => text(frame, "call_id").map(|offered| ServerFrame::CallOffer {
                call_id: offered.to_string(),
                chat_id: number(frame.get("chat_id")),
                from_user_id: number(frame.get("from_user_id")),
                sdp: text(frame, "sdp").unwrap_or_default().to_string(),
                video: flag(frame.get("video")).unwrap_or(false),
            }),
            "call_ringing" => text(frame, "call_id").map(|ringing| ServerFrame::CallRinging {
                call_id: ringing.to_string(),
            }),
            "call_answer" => text(frame, "call_id").map(|answered| ServerFrame::CallAnswer {
                call_id: answered.to_string(),
                sdp: text(frame, "sdp").unwrap_or_default().to_string(),
            }),
            "call_ice" => {
                let iced = text(frame, "call_id")?;
                let candidate = decode::<IceCandidate>(frame.get("candidate"))?;
                Some(ServerFrame::CallIce {
                    call_id: iced.to_string(),
                    candidate,
                })
            }
            "call_end" => text(frame, "call_id").map(|ended| ServerFrame::CallEnd {
                call_id: ended.to_string(),
                reason: text(frame, "reason").unwrap_or("failed").to_string(),
            }),
            "pong" => Some(ServerFrame::Pong),
            "error" => Some(ServerFrame::Error {
                code: text(frame, "code")
                    .unwrap_or(error_codes::INTERNAL)
                    .to_string(),
                detail: text(frame, "message").unwrap_or_default().to_string(),
                client_msg_id: text(frame, "client_msg_id").map(str::to_string),
                call_id: text(frame, "call_id").map(str::to_string),
            }),
            _ => None,
        }
    }
}

/// A payload object, or `None` when it is missing, `null` or not the shape.
fn decode<T: for<'de> Deserialize<'de>>(node: Option<&Value>) -> Option<T> {
    T::deserialize(node?).ok()
}

/// A number, or 0 for one that is missing or not a 64-bit integer — `win/`'s `Number`, which
/// caught the same failures.
fn number(node: Option<&Value>) -> i64 {
    node.and_then(Value::as_i64).unwrap_or(0)
}

/// A number that may be absent: `None` for a missing key or a `null`, and otherwise as
/// [`number`] reads it.
fn optional(node: Option<&Value>) -> Option<i64> {
    match node {
        None | Some(Value::Null) => None,
        Some(value) => Some(number(Some(value))),
    }
}

/// An optional flag, or `None`. Anything that is not a JSON boolean is read as ABSENT rather
/// than as either answer — an absent flag keeps what the store holds, where a guessed one would
/// overwrite it.
fn flag(node: Option<&Value>) -> Option<bool> {
    node.and_then(Value::as_bool)
}

/// A string field, or `None` when it is missing, `null` or not a string.
fn text<'a>(frame: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    frame.get(key).and_then(Value::as_str)
}

/// `ai_error`'s `reason`, or `None`. A value this client does not know — and anything that is
/// not a string at all — is read as ABSENT, never guessed at: the compatibility rules, applied to
/// a value rather than a field, so a later reason cannot be given a meaning here.
fn ai_reason(node: Option<&Value>) -> Option<AiErrorReason> {
    match node.and_then(Value::as_str) {
        Some("refused") => Some(AiErrorReason::Refused),
        _ => None,
    }
}

/// Why the assistant's answer failed, as `ai_error`'s `reason` says it. Only the values the
/// protocol defines are here; an unknown one never reaches this type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiErrorReason {
    /// The AI provider's OWN safety or content filter refused the question, the answer, or a
    /// picture's description — asking again in the same words gets the same refusal.
    Refused,
}

/// One ICE candidate. `sdp_mid` and `sdp_mline_index` are each optional: a WebRTC stack
/// supplies one, the other, or both, and the receiving stack accepts whichever it was given.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct IceCandidate {
    #[serde(default, deserialize_with = "wire::text")]
    pub candidate: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdp_mid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdp_mline_index: Option<i32>,
}

impl IceCandidate {
    pub fn new(candidate: &str, sdp_mid: Option<&str>, sdp_mline_index: Option<i32>) -> Self {
        IceCandidate {
            candidate: candidate.to_string(),
            sdp_mid: sdp_mid.map(str::to_string),
            sdp_mline_index,
        }
    }
}

/// What this client sends. Each frame is written by a struct whose fields are in the protocol's
/// order, because the frame names are a document — and because a JSON map in serde_json sorts
/// its keys, which would turn `{"type": "send", …}` into `{"body": …, …, "type": "send"}`.
///
/// The JSON is serde_json's: non-ASCII text goes as UTF-8, where `win/`'s writer escaped it as
/// `\uXXXX`. Both are the same JSON string to any reader.
pub mod client_frames {
    use serde::Serialize;

    use super::IceCandidate;
    use crate::protocol::dtos::{MentionDto, wire};

    /// What a `send` may carry beyond its chat, its key and its body — all absent by default.
    #[derive(Debug, Clone, Copy, PartialEq, Default)]
    pub struct SendOptions<'a> {
        pub reply_to_message_id: Option<i64>,
        pub attachment_ids: &'a [i64],
        pub poll_options: &'a [String],
        pub mentions: &'a [MentionDto],
        pub sticker: bool,
        pub round: bool,
        pub animation: bool,
    }

    fn not(flag: &bool) -> bool {
        !*flag
    }

    #[derive(Serialize)]
    struct SendFrame<'a> {
        #[serde(rename = "type")]
        kind: &'static str,
        chat_id: i64,
        client_msg_id: &'a str,
        body: &'a str,
        // Absent on an ordinary message, like every optional field on this wire — never false.
        #[serde(skip_serializing_if = "not")]
        sticker: bool,
        // A video message (docs/protocol.md, "Video messages"): the sticker's pattern — present
        // only when true.
        #[serde(skip_serializing_if = "not")]
        round: bool,
        // An animation (docs/protocol.md, "Animations"): the same pattern — present only when
        // true.
        #[serde(skip_serializing_if = "not")]
        animation: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        reply_to_message_id: Option<i64>,
        #[serde(skip_serializing_if = "<[i64]>::is_empty")]
        attachment_ids: &'a [i64],
        #[serde(skip_serializing_if = "Option::is_none")]
        poll: Option<PollOptions<'a>>,
        #[serde(skip_serializing_if = "<[MentionDto]>::is_empty")]
        mentions: &'a [MentionDto],
    }

    #[derive(Serialize)]
    struct PollOptions<'a> {
        options: &'a [String],
    }

    /// A message. The `client_msg_id` must be a real UUID and is what the `ack` comes back with —
    /// it is this device's handle on a row it has already drawn.
    pub fn send(
        chat_id: i64,
        client_msg_id: &str,
        body: &str,
        options: &SendOptions<'_>,
    ) -> String {
        wire::encode(&SendFrame {
            kind: "send",
            chat_id,
            client_msg_id,
            body,
            sticker: options.sticker,
            round: options.round,
            animation: options.animation,
            reply_to_message_id: options.reply_to_message_id,
            attachment_ids: options.attachment_ids,
            poll: (!options.poll_options.is_empty()).then_some(PollOptions {
                options: options.poll_options,
            }),
            mentions: options.mentions,
        })
    }

    #[derive(Serialize)]
    struct ReadFrame {
        #[serde(rename = "type")]
        kind: &'static str,
        chat_id: i64,
        last_read_message_id: i64,
    }

    pub fn read(chat_id: i64, last_read_message_id: i64) -> String {
        wire::encode(&ReadFrame {
            kind: "read",
            chat_id,
            last_read_message_id,
        })
    }

    #[derive(Serialize)]
    struct TypingFrame {
        #[serde(rename = "type")]
        kind: &'static str,
        chat_id: i64,
    }

    pub fn typing(chat_id: i64) -> String {
        wire::encode(&TypingFrame {
            kind: "typing",
            chat_id,
        })
    }

    #[derive(Serialize)]
    struct PingFrame {
        #[serde(rename = "type")]
        kind: &'static str,
    }

    pub fn ping() -> String {
        wire::encode(&PingFrame { kind: "ping" })
    }

    #[derive(Serialize)]
    struct CallOfferFrame<'a> {
        #[serde(rename = "type")]
        kind: &'static str,
        call_id: &'a str,
        chat_id: i64,
        sdp: &'a str,
        // Absent on a voice call, like every optional field on this wire — never false.
        #[serde(skip_serializing_if = "not")]
        video: bool,
    }

    pub fn call_offer(call_id: &str, chat_id: i64, sdp: &str, video: bool) -> String {
        wire::encode(&CallOfferFrame {
            kind: "call_offer",
            call_id,
            chat_id,
            sdp,
            video,
        })
    }

    #[derive(Serialize)]
    struct CallAnswerFrame<'a> {
        #[serde(rename = "type")]
        kind: &'static str,
        call_id: &'a str,
        sdp: &'a str,
    }

    pub fn call_answer(call_id: &str, sdp: &str) -> String {
        wire::encode(&CallAnswerFrame {
            kind: "call_answer",
            call_id,
            sdp,
        })
    }

    #[derive(Serialize)]
    struct CallIceFrame<'a> {
        #[serde(rename = "type")]
        kind: &'static str,
        call_id: &'a str,
        // Only the halves it was given: IceCandidate leaves out a `None`.
        candidate: &'a IceCandidate,
    }

    pub fn call_ice(call_id: &str, candidate: &IceCandidate) -> String {
        wire::encode(&CallIceFrame {
            kind: "call_ice",
            call_id,
            candidate,
        })
    }

    #[derive(Serialize)]
    struct CallEndFrame<'a> {
        #[serde(rename = "type")]
        kind: &'static str,
        call_id: &'a str,
        reason: &'a str,
    }

    pub fn call_end(call_id: &str, reason: &str) -> String {
        wire::encode(&CallEndFrame {
            kind: "call_end",
            call_id,
            reason,
        })
    }
}
