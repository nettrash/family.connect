//! Response DTOs and row-mapping helpers.
//!
//! DTO field names and shapes mirror protocol.md's "Objects" section exactly
//! — the WS frame unit tests in ws.rs pin them against literal JSON from the
//! document. Row mapping is hand-written `from_row(&PgRow)` (no
//! `query_as!`/offline metadata): queries here are dynamic enough that the
//! macro machinery would cost more than it saves, and `.get("col")` panics
//! loudly on a schema/query mismatch, which is a programmer error we want to
//! hear about in tests, not paper over.

use serde::{Deserialize, Serialize};
use sqlx::Row;
use sqlx::postgres::PgRow;
use time::OffsetDateTime;
use uuid::Uuid;

/// `User` object: the public identity of an account.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub display_name: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// Bumped on every profile-picture upload, `0` when there is none —
    /// the clients' cache key for `GET /users/{id}/avatar`.
    pub avatar_version: i64,
}

impl User {
    /// Map a row exposing `id, username, display_name, created_at,
    /// avatar_version`.
    pub fn from_row(row: &PgRow) -> Self {
        Self {
            id: row.get("id"),
            username: row.get("username"),
            display_name: row.get("display_name"),
            created_at: row.get("created_at"),
            avatar_version: row.get("avatar_version"),
        }
    }
}

/// Abbreviated user used inside WS membership frames — protocol.md shows
/// `member_joined.user` without `created_at`, and the frame tests pin the
/// exact shape, hence a separate type instead of reusing [`User`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UserBrief {
    pub id: i64,
    pub username: String,
    pub display_name: String,
    pub avatar_version: i64,
}

/// `Member` object: a user in the context of their family, with role.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Member {
    pub id: i64,
    pub username: String,
    pub display_name: String,
    /// `"owner"` or `"member"`.
    pub role: String,
    pub avatar_version: i64,
}

/// `Family` object. `invite_code` is present when (and only when) the caller
/// is the owner — hence `Option` + skip, never `null`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Family {
    pub id: i64,
    pub name: String,
    /// `"open"` or `"approval"`.
    pub join_policy: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invite_code: Option<String>,
}

/// `JoinRequest` object as listed for the owner.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JoinRequest {
    pub id: i64,
    pub user: User,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// `Chat` object. `title` is denormalized server-side (family name or peer
/// display name) so clients never join locally; `peer_user_id` is `null`
/// for the family chat, per the protocol example.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Chat {
    pub id: i64,
    /// `"family"` or `"direct"`.
    pub kind: String,
    pub title: String,
    pub peer_user_id: Option<i64>,
}

/// One user's reaction on a message (protocol.md `Reaction`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Reaction {
    pub user_id: i64,
    pub emoji: String,
}

/// `Message` object — identical over REST and WS.
///
/// `reactions`/`reaction_seq` are present iff the message has ever been
/// reacted to (`reaction_seq > 0` in the database); after the last reaction
/// is removed they stay present with an empty list, so clients can tell
/// "cleared" from "not provided". Absent-not-null keeps the pre-reaction
/// frame shapes byte-identical for old clients and old tests alike.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Message {
    pub id: i64,
    pub chat_id: i64,
    pub sender_id: i64,
    pub client_msg_id: Uuid,
    pub body: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reactions: Option<Vec<Reaction>>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reaction_seq: Option<i64>,
    /// Present when (and only when) this message is a reply. Recomputed on
    /// every read from the quoted row, never stored — see `ReplyTo`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reply_to: Option<ReplyTo>,
    /// Both present when (and only when) the body has been edited. Absent —
    /// not null, and not a zero seq — on a message still in its original
    /// form, which is how a client tells "never edited" from "edited".
    #[serde(
        skip_serializing_if = "Option::is_none",
        default,
        with = "crate::models::opt_rfc3339"
    )]
    pub edited_at: Option<OffsetDateTime>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub edit_seq: Option<i64>,
}

/// RFC3339 for an OPTIONAL timestamp. `time::serde::rfc3339::option` exists
/// but pairs badly with skip_serializing_if, so this thin wrapper keeps the
/// absent case absent rather than null.
pub mod opt_rfc3339 {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use time::OffsetDateTime;
    use time::format_description::well_known::Rfc3339;

    pub fn serialize<S: Serializer>(
        value: &Option<OffsetDateTime>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            Some(stamp) => stamp
                .format(&Rfc3339)
                .map_err(serde::ser::Error::custom)?
                .serialize(serializer),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<OffsetDateTime>, D::Error> {
        let raw = Option::<String>::deserialize(deserializer)?;
        raw.map(|s| OffsetDateTime::parse(&s, &Rfc3339))
            .transpose()
            .map_err(serde::de::Error::custom)
    }
}

/// The quoted message, as much of it as a client needs to draw the quote
/// without having the original in its cache (protocol.md, "Replies").
///
/// Deliberately a snapshot computed AT READ TIME rather than columns stored
/// beside the reply: a quoted message can be edited later, and a snippet
/// frozen at send time would go on showing text its author has since
/// changed. It is never nested — a reply to a reply carries its parent's
/// excerpt, not its grandparent's.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReplyTo {
    pub message_id: i64,
    pub sender_id: i64,
    pub excerpt: String,
}

impl ReplyTo {
    /// Longest excerpt sent, in Unicode scalar values. Enough to recognise
    /// the quoted message; a client that wants the rest has the id.
    pub const MAX_EXCERPT_CHARS: usize = 120;

    pub fn new(message_id: i64, sender_id: i64, body: &str) -> Self {
        Self {
            message_id,
            sender_id,
            excerpt: Self::excerpt(body),
        }
    }

    /// Cut to [`MAX_EXCERPT_CHARS`] scalar values — `chars()`, never bytes,
    /// so the cut can never land inside a character and produce invalid
    /// UTF-8 (or, on the clients, tofu).
    pub fn excerpt(body: &str) -> String {
        body.chars().take(Self::MAX_EXCERPT_CHARS).collect()
    }
}

impl Message {
    /// Map a row exposing `id, chat_id, sender_id, client_msg_id, body,
    /// created_at`. Reaction fields start `None`; the paths that expose
    /// them (`get_messages`) enrich afterwards.
    ///
    /// The reply snippet is read with `try_get` from the joined columns
    /// `reply_to_message_id` / `reply_sender_id` / `reply_body`, so the
    /// SAME mapper serves both the narrow SELECTs (chat-list previews,
    /// which carry no quote) and the joined one in `get_messages`. All
    /// three must be present and non-NULL for a snippet to appear.
    pub fn from_row(row: &PgRow) -> Self {
        let reply_to = match (
            row.try_get::<Option<i64>, _>("reply_to_message_id"),
            row.try_get::<Option<i64>, _>("reply_sender_id"),
            row.try_get::<Option<String>, _>("reply_body"),
        ) {
            (Ok(Some(message_id)), Ok(Some(sender_id)), Ok(Some(body))) => {
                Some(ReplyTo::new(message_id, sender_id, &body))
            }
            _ => None,
        };
        // Both sequences: zero in the column means "never happened" and is
        // ABSENT on the wire. Read here rather than by each caller — a read
        // path that forgot the reaction_seq loop silently dropped every
        // reaction from its response, which is exactly how this moved.
        let reaction_seq = match row.try_get::<i64, _>("reaction_seq") {
            Ok(seq) if seq > 0 => Some(seq),
            _ => None,
        };
        let edit_seq = match row.try_get::<i64, _>("edit_seq") {
            Ok(seq) if seq > 0 => Some(seq),
            _ => None,
        };
        let edited_at = row
            .try_get::<Option<OffsetDateTime>, _>("edited_at")
            .unwrap_or_default();
        Self {
            id: row.get("id"),
            chat_id: row.get("chat_id"),
            sender_id: row.get("sender_id"),
            client_msg_id: row.get("client_msg_id"),
            body: row.get("body"),
            created_at: row.get("created_at"),
            reactions: None,
            reaction_seq,
            reply_to,
            edited_at,
            edit_seq,
        }
    }
}

/// Entry of the `GET /chats` list: chat + preview + authoritative unread.
/// `max_reaction_seq` is omitted while nothing in the chat was ever reacted
/// to; `last_message` previews never carry reactions.
#[derive(Debug, Clone, Serialize)]
pub struct ChatListEntry {
    pub chat: Chat,
    pub last_message: Option<Message>,
    pub unread_count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_reaction_seq: Option<i64>,
    /// Omitted while nothing in the chat has ever been edited.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_edit_seq: Option<i64>,
}

/// `GET /me` — the caller's own pending join request, if any.
#[derive(Debug, Clone, Serialize)]
pub struct PendingJoinRequest {
    pub family_id: i64,
    pub family_name: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn family_omits_invite_code_entirely_for_non_owners() {
        let family = Family {
            id: 3,
            name: "The Smiths".to_string(),
            join_policy: "approval".to_string(),
            created_at: datetime!(2026-08-19 17:03:12 UTC),
            invite_code: None,
        };
        let json = serde_json::to_value(&family).expect("serialize");
        assert!(
            json.get("invite_code").is_none(),
            "invite_code must be absent, not null: {json}"
        );
    }

    #[test]
    fn chat_serializes_a_null_peer_for_the_family_chat() {
        let chat = Chat {
            id: 42,
            kind: "family".to_string(),
            title: "The Smiths".to_string(),
            peer_user_id: None,
        };
        let json = serde_json::to_value(&chat).expect("serialize");
        assert_eq!(json["peer_user_id"], serde_json::Value::Null);
    }

    #[test]
    fn timestamps_serialize_as_rfc3339_utc_with_a_z_suffix() {
        let user = User {
            id: 7,
            username: "anna".to_string(),
            display_name: "Anna".to_string(),
            created_at: datetime!(2026-08-19 17:03:12 UTC),
            avatar_version: 0,
        };
        let json = serde_json::to_value(&user).expect("serialize");
        assert_eq!(json["created_at"], "2026-08-19T17:03:12Z");
    }

    #[test]
    fn a_short_body_is_quoted_whole() {
        let reply = ReplyTo::new(41, 9, "See you at six");
        assert_eq!(reply.excerpt, "See you at six");
        assert_eq!(reply.message_id, 41);
        assert_eq!(reply.sender_id, 9);
    }

    #[test]
    fn a_long_body_is_cut_to_the_documented_length() {
        let body = "x".repeat(500);
        let excerpt = ReplyTo::excerpt(&body);
        assert_eq!(excerpt.chars().count(), ReplyTo::MAX_EXCERPT_CHARS);
    }

    /// The cut counts SCALAR VALUES, not bytes: cutting a multi-byte
    /// character in half would produce invalid UTF-8 — which serde would
    /// refuse to serialize, taking the whole message page down with it.
    #[test]
    fn the_cut_never_lands_inside_a_character() {
        // 'é' is 2 bytes, '中' 3, '😀' 4 — a byte-based cut at 120 would
        // split one of them.
        let body: String = "é中😀".repeat(100);
        let excerpt = ReplyTo::excerpt(&body);
        assert_eq!(excerpt.chars().count(), ReplyTo::MAX_EXCERPT_CHARS);
        // Round-trips as valid UTF-8 through JSON.
        let json = serde_json::to_string(&ReplyTo::new(1, 2, &body)).expect("serializes");
        assert!(json.contains("\"message_id\":1"));
    }

    #[test]
    fn an_empty_body_quotes_as_empty() {
        assert_eq!(ReplyTo::excerpt(""), "");
    }
}
