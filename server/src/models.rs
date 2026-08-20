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
}

impl User {
    /// Map a row exposing `id, username, display_name, created_at`.
    pub fn from_row(row: &PgRow) -> Self {
        Self {
            id: row.get("id"),
            username: row.get("username"),
            display_name: row.get("display_name"),
            created_at: row.get("created_at"),
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
}

/// `Member` object: a user in the context of their family, with role.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Member {
    pub id: i64,
    pub username: String,
    pub display_name: String,
    /// `"owner"` or `"member"`.
    pub role: String,
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
}

impl Message {
    /// Map a row exposing `id, chat_id, sender_id, client_msg_id, body,
    /// created_at`. Reaction fields start `None`; the paths that expose
    /// them (`get_messages`) enrich afterwards.
    pub fn from_row(row: &PgRow) -> Self {
        Self {
            id: row.get("id"),
            chat_id: row.get("chat_id"),
            sender_id: row.get("sender_id"),
            client_msg_id: row.get("client_msg_id"),
            body: row.get("body"),
            created_at: row.get("created_at"),
            reactions: None,
            reaction_seq: None,
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
        };
        let json = serde_json::to_value(&user).expect("serialize");
        assert_eq!(json["created_at"], "2026-08-19T17:03:12Z");
    }
}
