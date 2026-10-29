//! The objects on the wire (docs/protocol.md, "Objects"), as the structs this client decodes into
//! — a port of `win/src/FamilyConnect.Core/Protocol/Dtos.cs`.
//!
//! Two habits the whole protocol depends on, and they are the reason every optional field here is
//! an `Option` rather than defaulted: **an absent key is not an empty one** — a message with no
//! reactions omits `reactions`, while one whose last reaction was removed sends `"reactions": []`,
//! and clients distinguish "cleared" from "no data" — and **an unknown value falls back rather
//! than failing**, which is why kinds, sizes, colours and fonts are carried as strings and resolved
//! by the board's rules.
//!
//! How a document is read, field by field, is [`wire`]'s rule, kept to what `win/`'s
//! `Wire.Options` does: an unknown field is skipped; a field the protocol may omit is an `Option`;
//! a number or flag `win/` declares without a default reads a missing key as 0 or false, exactly as
//! System.Text.Json fills it; a string `win/` declares without a default reads a missing key or a
//! `null` as `""` (C# read both as `null`). An OBJECT `win/` declares without a default — a
//! response's `message`, a row's `chat` — is required here: C#'s decoder filled a missing one with
//! `null` and the reader then dereferenced it, so a document without it fails to decode instead,
//! which `ApiClient` reports as an answer it could not read.

use serde::{Deserialize, Serialize};

/// The one serializer (`win/`'s `Wire`): snake_case is spelled out by the field names, because
/// the wire is a document and not a convention.
pub mod wire {
    use serde::de::DeserializeOwned;
    use serde::{Deserialize, Deserializer, Serialize};

    /// The value, or `None` for anything that is not one: malformed JSON, the wrong shape, a
    /// required field missing. An unknown field is a NEWER SERVER, not an error — the
    /// compatibility rule is that a client ignores what it does not know (docs/protocol.md,
    /// "Compatibility rules") — and serde skips one unless told otherwise.
    pub fn decode<T: DeserializeOwned>(json: &str) -> Option<T> {
        serde_json::from_str(json).ok()
    }

    /// The JSON for a value: every `None` left out (System.Text.Json's `WhenWritingNull`), every
    /// other field written. serde_json fails only on a map with non-string keys or a `Serialize`
    /// that errors, and nothing in this crate has either, so the empty string is never produced.
    pub fn encode<T: Serialize + ?Sized>(value: &T) -> String {
        serde_json::to_string(value).unwrap_or_default()
    }

    /// A string `win/` declares without a default: a missing key or a `null` reads as `""` — C#'s
    /// decoder read both as `null`, and every reader treated that as empty or as nothing.
    pub(crate) fn text<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
        Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
    }

    /// A list of names that may carry a `null` inside (`greeting_places`, `assistant.lookups`):
    /// C# decoded `["Moscow", null]` into a `string[]` holding a null, and every reader of one
    /// (`GreetingWeather.Saved`, `Lookups.Providers`) dropped it — so it is dropped here, where the
    /// document is read, and a stray `null` never fails the whole family's read.
    pub(crate) fn names<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Vec<String>>, D::Error> {
        Ok(Option::<Vec<Option<String>>>::deserialize(deserializer)?
            .map(|names| names.into_iter().flatten().collect()))
    }

    pub(crate) fn yes() -> bool {
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct UserDto {
    #[serde(default)]
    pub id: i64,
    #[serde(default, deserialize_with = "wire::text")]
    pub username: String,
    #[serde(default, deserialize_with = "wire::text")]
    pub display_name: String,
    #[serde(default)]
    pub avatar_version: i32,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub birthday: Option<BirthdayDto>,
}

impl UserDto {
    /// `new UserDto(id, username, displayName)`, the rest at their defaults.
    pub fn new(id: i64, username: &str, display_name: &str) -> Self {
        UserDto {
            id,
            username: username.to_string(),
            display_name: display_name.to_string(),
            ..UserDto::default()
        }
    }
}

/// A member of the family, with the two flags that outlive them.
///
/// A member who has LEFT and an account that was DELETED both keep their rows, because their
/// messages and their notes keep their authors: a name has to resolve for an old message, and
/// "Deleted account" is a name this client supplies rather than the server.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MemberDto {
    #[serde(default)]
    pub id: i64,
    #[serde(default, deserialize_with = "wire::text")]
    pub username: String,
    #[serde(default, deserialize_with = "wire::text")]
    pub display_name: String,
    #[serde(default)]
    pub avatar_version: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default)]
    pub has_left: bool,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub birthday: Option<BirthdayDto>,
    /// Whether the OWNER reading their own family may reset this member's password — which needs
    /// them to own every family the member is in (docs/protocol.md, "Several families", D7).
    /// Present in the owner's own read only; `None` from every other read and from a server that
    /// predates several families (see `FamilyRules.MayResetPassword`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password_reset: Option<bool>,
}

impl MemberDto {
    /// `new MemberDto(id, username, displayName)`, the rest at their defaults.
    pub fn new(id: i64, username: &str, display_name: &str) -> Self {
        MemberDto {
            id,
            username: username.to_string(),
            display_name: display_name.to_string(),
            ..MemberDto::default()
        }
    }

    /// Whether this member owns the family. The wire says `"role": "owner"|"member"` — a STRING,
    /// and there is no `owner` boolean to decode: a port that declared one got `false` for
    /// everybody, including the owner reading their own roster, and every owner-only door in the
    /// app would have been shut. The same idiom as [`MeResponse::is_owner`], deliberately.
    ///
    /// [`MeResponse::is_owner`]: super::requests::MeResponse::is_owner
    pub fn owner(&self) -> bool {
        self.role.as_deref() == Some("owner")
    }

    /// A member who is GONE: they left, or their account was deleted. Both keep their row, and
    /// this is the one question a name-resolving caller asks about it.
    pub fn is_former(&self) -> bool {
        self.has_left || self.deleted
    }
}

/// A day and a month, with NO YEAR — "nobody should have to publish their age to be wished a
/// happy birthday" (docs/protocol.md). 29 February is a birthday here, because there is no year
/// for it to fail to exist in.
///
/// ONE OBJECT, NOT TWO SIBLING FIELDS, and that shape is load-bearing for a reason worth writing
/// down: `win/` first declared it a string, which made `GET /families/mine` UNREADABLE for any
/// family where somebody had set one — the decode failed, the answer came back as a terminal
/// "2xx I cannot read", and the whole resync stopped. A field whose type is wrong does not
/// degrade; it takes the document with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct BirthdayDto {
    #[serde(default)]
    pub month: i32,
    #[serde(default)]
    pub day: i32,
}

/// A chat (docs/protocol.md, "Objects"): `family`, `direct`, `ai` or — since 2026-10-22 —
/// `group`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ChatDto {
    #[serde(default)]
    pub id: i64,
    #[serde(default, deserialize_with = "wire::text")]
    pub kind: String,
    #[serde(default, deserialize_with = "wire::text")]
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_user_id: Option<i64>,
    /// A GROUP's members, the reader included, in the order they joined (docs/protocol.md,
    /// "Groups"). Present only for a group; `None` for every other kind, and for a group a server
    /// before groups described.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member_ids: Option<Vec<i64>>,
    /// The one member of a GROUP who may rename it, add and remove members, and delete it.
    /// Present only for a group.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admin_user_id: Option<i64>,
    /// Which families the chat is LISTED in (docs/protocol.md, "Several families"): exactly one
    /// for a family chat, a group and an `ai` chat; for a direct chat every family the two people
    /// share, ascending. `None` from a server that predates several families, where it means the
    /// one family the account has — so a `None` is listed in the HOME family alone
    /// (`FamilyRules.FamiliesOf`). An empty list is listed nowhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family_ids: Option<Vec<i64>>,
}

impl ChatDto {
    /// `new ChatDto(id, kind, title)`, the rest absent.
    pub fn new(id: i64, kind: &str, title: &str) -> Self {
        ChatDto {
            id,
            kind: kind.to_string(),
            title: title.to_string(),
            ..ChatDto::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MentionDto {
    #[serde(default)]
    pub user_id: i64,
    #[serde(default, deserialize_with = "wire::text")]
    pub name: String,
}

impl MentionDto {
    pub fn new(user_id: i64, name: &str) -> Self {
        MentionDto {
            user_id,
            name: name.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ReactionDto {
    #[serde(default)]
    pub user_id: i64,
    #[serde(default, deserialize_with = "wire::text")]
    pub emoji: String,
}

impl ReactionDto {
    pub fn new(user_id: i64, emoji: &str) -> Self {
        ReactionDto {
            user_id,
            emoji: emoji.to_string(),
        }
    }
}

/// The level UNDER a quote: the quoted message's own quote, one deep and no deeper
/// (docs/protocol.md, "Replies"). It has no `parent` of its own on purpose — the protocol's depth
/// cap is structural, so a message four deep still shows exactly two levels and no page can drag
/// a whole ancestry along.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct QuoteParentDto {
    #[serde(default)]
    pub message_id: i64,
    #[serde(default)]
    pub sender_id: i64,
    #[serde(default, deserialize_with = "wire::text")]
    pub excerpt: String,
}

/// The quote over a reply, as the server RECOMPUTES it on every read: who wrote the answered
/// message, a cut of what it said, and one further level when it was itself a reply. It exists so
/// that a client can draw the quote without holding the original, and the id is how a client
/// that wants the whole message finds it (docs/protocol.md, "Replies").
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ReplyToDto {
    #[serde(default)]
    pub message_id: i64,
    #[serde(default)]
    pub sender_id: i64,
    #[serde(default, deserialize_with = "wire::text")]
    pub excerpt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<QuoteParentDto>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AttachmentDto {
    #[serde(default)]
    pub id: i64,
    #[serde(default, deserialize_with = "wire::text")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i32>,
    #[serde(default)]
    pub has_preview: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latitude: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub longitude: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accuracy_m: Option<f64>,
    /// This attachment was SENT AS A STICKER (docs/protocol.md, "Sticker pack"): the send said
    /// so, the server stamped it, and it never changes. Present on the wire only when true — so
    /// an attachment from before stickers, and every one a pack ITEM carries, reads false here.
    ///
    /// NOT a board note. "Sticker" in this codebase already names the card a note is drawn as;
    /// this is the other thing — a picture in a chat — and the only place the wire itself spells
    /// the word.
    #[serde(default)]
    pub sticker: bool,
    /// This video was SENT AS A VIDEO MESSAGE (docs/protocol.md, "Video messages"): a square
    /// H.264/AAC MP4 recorded to be drawn round. Present on the wire only when true, set by the
    /// send and never changed — so every video from before video messages, and every one a 1.2
    /// app sent, reads false here. Never beside [`sticker`](Self::sticker), never on anything but
    /// a video; in every other respect it is still `kind=video`.
    #[serde(default)]
    pub round: bool,
    /// A voice note's shape (docs/protocol.md, "A voice note's waveform"): 48 lowercase hex
    /// digits the SENDER measured, absent on a picked sound file, a video, anything else, and
    /// everything an older client or server sent. Read through the waveform rules
    /// (`fc_text::waveform`), never trusted to be well formed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waveform: Option<String>,
    /// This video is an ANIMATION (docs/protocol.md, "Animations"): an animated GIF, WebP or PNG
    /// the sender turned into a silent looping H.264 MP4. Present on the wire only when true, set
    /// by the send and never changed — every other video, and everything an older server sent,
    /// reads false here. Never beside [`sticker`](Self::sticker) or [`round`](Self::round), never
    /// on anything but a video; in every other respect it is still `kind=video`.
    #[serde(default)]
    pub animation: bool,
}

impl AttachmentDto {
    /// `new AttachmentDto(id, kind)`, the rest absent.
    pub fn new(id: i64, kind: &str) -> Self {
        AttachmentDto {
            id,
            kind: kind.to_string(),
            ..AttachmentDto::default()
        }
    }

    pub fn is_photo(&self) -> bool {
        self.kind == "photo"
    }

    pub fn is_video(&self) -> bool {
        self.kind == "video"
    }

    /// The shape to reserve before the bytes arrive, or `None` when the uploader could not say.
    /// A tile built from this is a tile that never jumps — and never crops (issue #71).
    ///
    /// Not `fc_text::media::aspect_ratio`, which answers 4:3 for "could not say": `win/` keeps
    /// the absence, and its readers choose their own fallback.
    pub fn aspect_ratio(&self) -> Option<f64> {
        match (self.width, self.height) {
            (Some(width), Some(height)) if width > 0 && height > 0 => {
                Some(f64::from(width) / f64::from(height))
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PollOptionDto {
    #[serde(default)]
    pub id: i64,
    #[serde(default, deserialize_with = "wire::text")]
    pub text: String,
    #[serde(default)]
    pub votes: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PollDto {
    #[serde(default)]
    pub poll_seq: i64,
    #[serde(default)]
    pub closed: bool,
    #[serde(default)]
    pub options: Vec<PollOptionDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CallRecordDto {
    #[serde(default, deserialize_with = "wire::text")]
    pub outcome: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_secs: Option<i32>,
    #[serde(default)]
    pub video: bool,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MessageDto {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub chat_id: i64,
    #[serde(default)]
    pub sender_id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_msg_id: Option<String>,
    #[serde(default, deserialize_with = "wire::text")]
    pub body: String,
    #[serde(default, deserialize_with = "wire::text")]
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reactions: Option<Vec<ReactionDto>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reaction_seq: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<ReplyToDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_root_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_count: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mentions: Option<Vec<MentionDto>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edited_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit_seq: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<AttachmentDto>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment: Option<AttachmentDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll: Option<PollDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call: Option<CallRecordDto>,
}

impl MessageDto {
    /// `new MessageDto(id, chatId, senderId, clientMsgId, body, createdAt)`, the rest absent.
    pub fn new(
        id: i64,
        chat_id: i64,
        sender_id: i64,
        client_msg_id: Option<&str>,
        body: &str,
        created_at: &str,
    ) -> Self {
        MessageDto {
            id,
            chat_id,
            sender_id,
            client_msg_id: client_msg_id.map(str::to_string),
            body: body.to_string(),
            created_at: created_at.to_string(),
            ..MessageDto::default()
        }
    }

    /// Everything this message carries, in the sender's order — reading `attachments` and falling
    /// back to the singular only for a server that predates plurality. A client that reads the
    /// plural ignores the singular; the two are never present without each other.
    pub fn media(&self) -> &[AttachmentDto] {
        match (&self.attachments, &self.attachment) {
            (Some(all), _) => all,
            (None, Some(one)) => std::slice::from_ref(one),
            (None, None) => &[],
        }
    }

    /// The picture this message IS, when it is a sticker (docs/protocol.md, "Sending one").
    ///
    /// THE ONE TEST, AND THE SAME ON EVERY CLIENT: exactly ONE attachment, that attachment is
    /// `kind=photo`, and it carries `sticker: true`. Nothing else is asked — not the body, not the
    /// type — so five clients cannot disagree about which message is a sticker. Anything else (a
    /// message from before stickers, a server that ignores the flag, two attachments) is `None`,
    /// and is drawn as the ordinary message it otherwise is. (`fc-text` has no counterpart: the
    /// web client asks it inline too.)
    pub fn sticker_picture(&self) -> Option<&AttachmentDto> {
        match self.media() {
            [picture] if picture.sticker && picture.kind == "photo" => Some(picture),
            _ => None,
        }
    }

    /// The video this message IS, when it is a video message — drawn as a circle with no balloon
    /// (docs/protocol.md, "Video messages"; docs/audio-video-messages-2026-10-04.md, S5.1).
    ///
    /// THE SAME TEST ON EVERY CLIENT, and here it IS that test — `fc_text::record::is_round`,
    /// held to `record-vectors.json`: exactly ONE attachment, that attachment is `kind=video`, it
    /// carries `round: true`, and the message has NO BODY. Anything else — two attachments, the
    /// flag on a photo, a body, a server that ignores the flag — is `None`, and is drawn as the
    /// ordinary message it otherwise is. The body is compared EXACTLY: the server stores an
    /// attachment message whose body trims to nothing as `""`, so no port's idea of whitespace
    /// can make two clients disagree. A body the wire left out is read as none.
    pub fn round_video(&self) -> Option<&AttachmentDto> {
        let media = self.media();
        let flags: Vec<fc_text::record::AttachmentFlags<'_>> = media
            .iter()
            .map(|attachment| fc_text::record::AttachmentFlags {
                kind: &attachment.kind,
                round: attachment.round,
            })
            .collect();
        if fc_text::record::is_round(&self.body, &flags) {
            media.first()
        } else {
            None
        }
    }

    /// The video this message IS, when it is an ANIMATION — drawn as a picture that plays by
    /// itself, muted, looping, with a small "GIF" label and no player controls (docs/protocol.md,
    /// "Animations", "How it is drawn").
    ///
    /// THE STICKER'S AND THE VIDEO MESSAGE'S ONE TEST, here `fc_text::animation_source::
    /// is_animation`: exactly ONE attachment, that attachment is `kind=video`, and it carries
    /// `animation: true`. Unlike a video message an animation MAY carry a caption, so the body is
    /// not asked. Anything else — two attachments, the flag on a photo, a server that ignores the
    /// flag — is `None`, and is drawn as the ordinary message it otherwise is.
    pub fn animation_video(&self) -> Option<&AttachmentDto> {
        let media = self.media();
        let flags: Vec<fc_text::animation_source::AttachmentFlags<'_>> = media
            .iter()
            .map(|attachment| fc_text::animation_source::AttachmentFlags {
                kind: &attachment.kind,
                animation: attachment.animation,
            })
            .collect();
        if fc_text::animation_source::is_animation(&flags) {
            media.first()
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TaskItemDto {
    #[serde(default)]
    pub id: i64,
    #[serde(default, deserialize_with = "wire::text")]
    pub text: String,
    #[serde(default)]
    pub done: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done_by: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RsvpDto {
    #[serde(default)]
    pub user_id: i64,
    #[serde(default, deserialize_with = "wire::text")]
    pub answer: String,
}

/// One note on the family's board, in whatever state it now has — a tombstone included, which
/// carries no author (`author_id` then reads 0, as C#'s decoder filled it).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct NoteDto {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub author_id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub board_seq: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_seq: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment: Option<AttachmentDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub starts_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ends_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rsvps: Option<Vec<RsvpDto>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Vec<TaskItemDto>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mentions: Option<Vec<MentionDto>>,
    #[serde(default)]
    pub deleted: bool,
    /// The family whose board the note is on (docs/protocol.md, "Several families",
    /// 2026-10-22) — how a `board_note` frame says whose it is. `None` from a server that
    /// predates several families: the one family there is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family_id: Option<i64>,
}

impl NoteDto {
    /// `new NoteDto(id, authorId)`, the rest absent.
    pub fn new(id: i64, author_id: i64) -> Self {
        NoteDto {
            id,
            author_id,
            ..NoteDto::default()
        }
    }

    /// How many people gave this answer — the number the sticker shows.
    pub fn count(&self, answer: &str) -> usize {
        self.rsvps.as_deref().map_or(0, |rsvps| {
            rsvps.iter().filter(|rsvp| rsvp.answer == answer).count()
        })
    }

    /// This reader's own answer, or `None` while they have not said.
    pub fn my_answer(&self, user_id: i64) -> Option<&str> {
        self.rsvps
            .as_deref()?
            .iter()
            .find(|rsvp| rsvp.user_id == user_id)
            .map(|rsvp| rsvp.answer.as_str())
    }

    /// The lines of a list. An EMPTY list sends `[]` and every other kind sends nothing at all,
    /// which is the difference between "a list with nothing on it yet" and "not a list".
    pub fn task_list(&self) -> &[TaskItemDto] {
        self.items.as_deref().unwrap_or(&[])
    }
}

/// One item of the family's sticker pack (docs/protocol.md, "Sticker pack") — a picture the
/// family KEEPS, which is not the same thing as a sticker somebody SENT: a sent one is a message
/// carrying its own copy, and neither names the other.
///
/// A TOMBSTONE is `{"id", "deleted": true, "pack_seq"}` and nothing else — no picture, no author,
/// no label — which is why everything but the id and the seq is optional here.
///
/// [`attachment`](Self::attachment) is an ordinary `kind=photo` attachment whose bytes are the
/// sticker, and it never carries `sticker`: that flag is a message's. Its ORIGINAL bytes are what
/// is drawn, whatever `has_preview` says — the flag can be inherited through dedup from somebody
/// who once sent the same PNG as a photograph, and a preview is a JPEG.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PackItemDto {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub added_by: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment: Option<AttachmentDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default)]
    pub pack_seq: i64,
    /// A few words for a screen reader, when whoever added it gave any. Never drawn over the
    /// picture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default)]
    pub deleted: bool,
    /// Whose pack the item is in (2026-10-22); `None` from an older server, which has one family
    /// to mean.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family_id: Option<i64>,
}

impl PackItemDto {
    /// `new PackItemDto(id)`, the rest absent.
    pub fn new(id: i64) -> Self {
        PackItemDto {
            id,
            ..PackItemDto::default()
        }
    }
}

/// The error body, as it arrives (docs/protocol.md, "Error shape"). A body without its `error`
/// object is no envelope at all — the status classifies it — where C#'s decoder handed back an
/// envelope whose `Error` was null and `ApiClient` dereferenced it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ErrorEnvelope {
    pub error: ErrorBody,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ErrorBody {
    #[serde(default, deserialize_with = "wire::text")]
    pub code: String,
    #[serde(default, deserialize_with = "wire::text")]
    pub message: String,
}
