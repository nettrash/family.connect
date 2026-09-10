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

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Family {
    pub id: i64,
    pub name: String,
}

/// A trimmed `GET /me`: who this is, and whether they are in a family.
///
/// `family` is null for an account that has not joined one — a real state
/// this client has to draw rather than a failure.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Me {
    pub user: User,
    #[serde(default)]
    pub family: Option<Family>,
}

/// One person in `GET /families/mine` — trimmed to what draws a name.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Member {
    pub id: i64,
    pub display_name: String,
}

/// The family's assistant, which speaks in the chat under an account of
/// its own and so needs a name like anybody else.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Assistant {
    pub user_id: i64,
    pub display_name: String,
}

/// `GET /families/mine`, trimmed to the names in it.
///
/// `former_members` is there for exactly this: the messages somebody left
/// behind when their account was deleted still need a name on them. It is
/// omitted when there are none.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Roster {
    pub members: Vec<Member>,
    #[serde(default)]
    pub former_members: Vec<Member>,
    #[serde(default)]
    pub assistant: Option<Assistant>,
}

impl Roster {
    /// Everybody this family's messages can be from, by user id.
    pub fn names(self) -> Vec<(i64, String)> {
        let assistant = self
            .assistant
            .map(|assistant| (assistant.user_id, assistant.display_name));
        self.former_members
            .into_iter()
            .chain(self.members)
            .map(|member| (member.id, member.display_name))
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

/// One row of `GET /chats`.
///
/// `last_read_message_id` is ALWAYS present (the protocol says so
/// explicitly, and `0` is a real answer meaning "never reported"), so it is
/// not an Option. `unread_count` likewise.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ChatListItem {
    pub chat: Chat,
    #[serde(default)]
    pub last_message: Option<Message>,
    pub unread_count: i64,
    pub last_read_message_id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Message {
    pub id: i64,
    pub chat_id: i64,
    pub sender_id: i64,
    #[serde(default)]
    pub client_msg_id: Option<String>,
    pub body: String,
    pub created_at: String,
    /// Present only on an edited message, which is what makes it the mark
    /// rather than a second timestamp to reconcile.
    #[serde(default)]
    pub edited_at: Option<String>,
}

impl Chat {
    /// What to call this chat in a list.
    ///
    /// The family chat carries the family's name as its title; a direct
    /// chat carries none, and the caller supplies the peer's name from the
    /// roster — which is why this takes it rather than inventing one.
    pub fn display_title(&self, peer_name: Option<&str>) -> String {
        if let Some(title) = self.title.as_ref().filter(|title| !title.is_empty()) {
            return title.clone();
        }
        peer_name.unwrap_or("Chat").to_string()
    }
}

impl Message {
    /// The clock a bubble shows: `HH:MM` out of the protocol's RFC 3339.
    ///
    /// Cut from the string rather than parsed into a date type: the wire
    /// carries UTC, this client shows UTC, and pulling in a date library to
    /// take five characters would be the wrong trade. When this client
    /// learns to show local time it will need one, and that is the moment
    /// to add it — not before.
    pub fn clock(&self) -> String {
        self.created_at
            .split('T')
            .nth(1)
            .map(|time| time.chars().take(5).collect())
            .unwrap_or_default()
    }

    pub fn is_edited(&self) -> bool {
        self.edited_at.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    #[wasm_bindgen_test]
    fn a_message_reads_its_clock_out_of_the_wire_timestamp() {
        let message = Message {
            id: 1,
            chat_id: 42,
            sender_id: 7,
            client_msg_id: None,
            body: "Dinner at 7?".into(),
            created_at: "2026-08-19T17:03:12Z".into(),
            edited_at: None,
        };
        assert_eq!(message.clock(), "17:03");
        assert!(!message.is_edited());
    }

    /// A timestamp this client cannot read must not panic and must not draw
    /// something wrong — an empty clock is the honest answer.
    #[wasm_bindgen_test]
    fn a_timestamp_without_a_time_reads_as_nothing() {
        let message = Message {
            id: 1,
            chat_id: 42,
            sender_id: 7,
            client_msg_id: None,
            body: String::new(),
            created_at: "2026-08-19".into(),
            edited_at: Some("2026-08-19T18:00:00Z".into()),
        };
        assert_eq!(message.clock(), "");
        assert!(message.is_edited());
    }

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
                          "draw": true, "vision": false, "images": true}
        }"#;
        let roster: Roster = serde_json::from_str(json).expect("a roster this client can read");
        let names: std::collections::HashMap<i64, String> = roster.names().into_iter().collect();
        assert_eq!(names.get(&9).map(String::as_str), Some("Anna"));
        assert_eq!(names.get(&4).map(String::as_str), Some("Gran"));
        assert_eq!(names.get(&2).map(String::as_str), Some("Assistant"));
        assert_eq!(names.len(), 4);

        // No former members and no assistant is an ordinary family too.
        let bare: Roster = serde_json::from_str(r#"{"members": []}"#).expect("reads");
        assert!(bare.names().is_empty());
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
}
