//! Push notification composition — pure functions from event context to the
//! exact wire payloads of protocol.md "Push notifications".
//!
//! Kept free of I/O on purpose: the golden unit tests below pin every byte
//! of the APNs/FCM JSON against the protocol document without a network or
//! a database. `events.rs` supplies the context (names, ids, the per-user
//! badge) and the transports in `push.rs` serialize these values verbatim.

use serde_json::{Value, json};

use crate::models::Message;

/// Which protocol event a notification announces, carrying the ids the
/// client deep-links with: `chat_id`/`message_id` open the chat, `family_id`
/// opens the join-requests screen (`join_request`) or the chat list
/// (`joined`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushEvent {
    Message { chat_id: i64, message_id: i64 },
    JoinRequest { family_id: i64 },
    Joined { family_id: i64 },
}

impl PushEvent {
    /// The `kind` discriminator clients switch on.
    pub fn kind(&self) -> &'static str {
        match self {
            PushEvent::Message { .. } => "message",
            PushEvent::JoinRequest { .. } => "join_request",
            PushEvent::Joined { .. } => "joined",
        }
    }

    /// APNs `thread-id` / FCM notification `tag`: messages group per chat
    /// (`chat-<id>`, per protocol.md); join events are one-offs and carry no
    /// grouping key — the document defines none for them.
    fn thread_key(&self) -> Option<String> {
        match self {
            PushEvent::Message { chat_id, .. } => Some(format!("chat-{chat_id}")),
            PushEvent::JoinRequest { .. } | PushEvent::Joined { .. } => None,
        }
    }
}

/// A composed, transport-agnostic notification. One instance is built per
/// *recipient user* (not per device): `badge` is that user's total unread
/// across chats at send time, so it differs between recipients of the same
/// message.
#[derive(Debug, Clone, PartialEq)]
pub struct Notification {
    pub title: String,
    pub body: String,
    /// Recipient's total unread across chats. APNs-only on the wire — the
    /// protocol's FCM shape carries no badge (Android renders its own).
    pub badge: i64,
    pub event: PushEvent,
}

/// Compose the notification for a new message. Title rules per protocol.md:
/// direct chat → the sender's display name; family chat →
/// `"<Family> — <Sender>"`. Body: the message text; the KIND of attachment
/// when a message carries one with no caption (which is how photos are
/// normally sent — an empty body would push a blank line); or
/// `"New message"` when `[push] include_message_body = false`.
/// What to say about a message that has no words: a file's name (which is
/// its whole identity) or what kind of thing arrived.
fn attachment_summary(message: &Message) -> Option<String> {
    let attachment = message.attachment.as_ref()?;
    Some(match attachment.kind.as_str() {
        "photo" => "Photo".to_string(),
        "video" => "Video".to_string(),
        _ => attachment
            .name
            .clone()
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "File".to_string()),
    })
}

pub fn message_notification(
    include_message_body: bool,
    chat_kind: &str,
    family_name: &str,
    sender_name: &str,
    message: &Message,
    badge: i64,
) -> Notification {
    let title = if chat_kind == "family" {
        format!("{family_name} — {sender_name}")
    } else {
        sender_name.to_string()
    };
    let body = if include_message_body {
        if message.body.is_empty() {
            attachment_summary(message).unwrap_or_else(|| "New message".to_string())
        } else {
            message.body.clone()
        }
    } else {
        "New message".to_string()
    };
    Notification {
        title,
        body,
        badge,
        event: PushEvent::Message {
            chat_id: message.chat_id,
            message_id: message.id,
        },
    }
}

/// Compose the notification the family owner gets when a join request is
/// created: title = the family name, body = `"<Display Name> asked to
/// join"`.
pub fn join_request_notification(
    family_name: &str,
    requester_display_name: &str,
    family_id: i64,
    badge: i64,
) -> Notification {
    Notification {
        title: family_name.to_string(),
        body: format!("{requester_display_name} asked to join"),
        badge,
        event: PushEvent::JoinRequest { family_id },
    }
}

/// Compose the notification the requester gets when their join request is
/// approved.
pub fn joined_notification(family_name: &str, family_id: i64, badge: i64) -> Notification {
    Notification {
        title: family_name.to_string(),
        body: format!("You're in — welcome to {family_name}"),
        badge,
        event: PushEvent::Joined { family_id },
    }
}

/// The APNs request body, exactly as protocol.md specifies it: the `aps`
/// dictionary plus top-level `kind` and the event's ids.
pub fn apns_payload(note: &Notification) -> Value {
    let mut aps = json!({
        "alert": {"title": note.title, "body": note.body},
        "sound": "default",
        "badge": note.badge,
    });
    if let Some(thread) = note.event.thread_key() {
        aps["thread-id"] = json!(thread);
    }
    let mut payload = json!({"aps": aps, "kind": note.event.kind()});
    match note.event {
        PushEvent::Message {
            chat_id,
            message_id,
        } => {
            payload["chat_id"] = json!(chat_id);
            payload["message_id"] = json!(message_id);
        }
        PushEvent::JoinRequest { family_id } | PushEvent::Joined { family_id } => {
            payload["family_id"] = json!(family_id);
        }
    }
    payload
}

/// The FCM HTTP v1 request body per protocol.md: `notification` + `data` so
/// the system tray renders when the app process is dead. FCM `data` values
/// MUST be strings — ids are stringified here, never sent as numbers.
pub fn fcm_message(note: &Notification, push_token: &str) -> Value {
    let mut data = json!({"kind": note.event.kind()});
    match note.event {
        PushEvent::Message {
            chat_id,
            message_id,
        } => {
            data["chat_id"] = json!(chat_id.to_string());
            data["message_id"] = json!(message_id.to_string());
        }
        PushEvent::JoinRequest { family_id } | PushEvent::Joined { family_id } => {
            data["family_id"] = json!(family_id.to_string());
        }
    }
    let mut android_notification = json!({"channel_id": "messages"});
    if let Some(tag) = note.event.thread_key() {
        android_notification["tag"] = json!(tag);
    }
    json!({"message": {
        "token": push_token,
        "notification": {"title": note.title, "body": note.body},
        "data": data,
        "android": {"priority": "HIGH", "notification": android_notification},
    }})
}

/// The one SQL statement behind the APNs badge: the recipient's total unread
/// across all their chats — per-chat unread is "messages newer than my read
/// marker, not sent by me" (the same rule `GET /chats` uses), and a flat
/// COUNT over the three-table join is the SUM of those per-chat counts.
/// `$1` is the recipient's user id.
pub fn build_unread_badge_query() -> &'static str {
    "SELECT count(*)
     FROM chat_members m
     JOIN messages msg ON msg.chat_id = m.chat_id
     LEFT JOIN chat_reads cr ON cr.chat_id = m.chat_id AND cr.user_id = m.user_id
     WHERE m.user_id = $1
       AND msg.id > COALESCE(cr.last_read_message_id, 0)
       AND msg.sender_id <> $1"
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;
    use uuid::Uuid;

    /// The message from protocol.md's push examples: id 1338 in chat 42.
    fn protocol_message() -> Message {
        Message {
            id: 1338,
            chat_id: 42,
            sender_id: 7,
            client_msg_id: Uuid::nil(),
            body: "Dinner at 7?".to_string(),
            created_at: datetime!(2026-08-19 17:03:12 UTC),
            reactions: None,
            reply_to: None,
            edited_at: None,
            edit_seq: None,
            attachment: None,
            reaction_seq: None,
        }
    }

    #[test]
    fn the_apns_message_payload_matches_the_protocol_example_exactly() {
        let note =
            message_notification(true, "direct", "The Smiths", "Anna", &protocol_message(), 3);
        assert_eq!(
            apns_payload(&note),
            json!({
                "aps": {"alert": {"title": "Anna", "body": "Dinner at 7?"}, "sound": "default",
                        "badge": 3, "thread-id": "chat-42"},
                "chat_id": 42, "message_id": 1338, "kind": "message"
            })
        );
    }

    #[test]
    fn the_fcm_message_matches_the_protocol_example_exactly() {
        let note =
            message_notification(true, "direct", "The Smiths", "Anna", &protocol_message(), 3);
        assert_eq!(
            fcm_message(&note, "token-1"),
            json!({"message": {"token": "token-1",
                "notification": {"title": "Anna", "body": "Dinner at 7?"},
                "data": {"kind": "message", "chat_id": "42", "message_id": "1338"},
                "android": {"priority": "HIGH",
                            "notification": {"channel_id": "messages", "tag": "chat-42"}}}})
        );
    }

    #[test]
    fn a_family_chat_title_combines_family_and_sender_with_an_em_dash() {
        let note =
            message_notification(true, "family", "The Smiths", "Anna", &protocol_message(), 1);
        assert_eq!(note.title, "The Smiths — Anna");
    }

    #[test]
    fn a_direct_chat_title_is_just_the_sender_display_name() {
        let note =
            message_notification(true, "direct", "The Smiths", "Anna", &protocol_message(), 1);
        assert_eq!(note.title, "Anna");
    }

    /// A photo is normally sent with no caption at all, so this is the
    /// ordinary case rather than an edge one — and an alert with the
    /// sender's name and a blank line tells the reader nothing.
    #[test]
    fn an_uncaptioned_attachment_says_what_arrived() {
        fn with_attachment(kind: &str, name: Option<&str>) -> Message {
            Message {
                body: String::new(),
                attachment: Some(crate::models::Attachment {
                    id: 34,
                    kind: kind.to_string(),
                    mime: "image/jpeg".to_string(),
                    size: 4096,
                    width: None,
                    height: None,
                    duration_ms: None,
                    has_preview: false,
                    name: name.map(str::to_string),
                }),
                ..protocol_message()
            }
        }

        let photo = message_notification(
            true,
            "direct",
            "The Smiths",
            "Anna",
            &with_attachment("photo", None),
            1,
        );
        assert_eq!(photo.body, "Photo");
        let video = message_notification(
            true,
            "direct",
            "The Smiths",
            "Anna",
            &with_attachment("video", None),
            1,
        );
        assert_eq!(video.body, "Video");
        // A file's name IS the thing worth saying.
        let file = message_notification(
            true,
            "family",
            "The Smiths",
            "Anna",
            &with_attachment("file", Some("Rechnung.pdf")),
            1,
        );
        assert_eq!(file.body, "Rechnung.pdf");

        // A caption still wins over the summary.
        let captioned = Message {
            body: "at the lake".to_string(),
            ..with_attachment("photo", None)
        };
        let note = message_notification(true, "direct", "S", "Anna", &captioned, 1);
        assert_eq!(note.body, "at the lake");

        // And the privacy switch still hides everything.
        let hidden = message_notification(
            false,
            "direct",
            "S",
            "Anna",
            &with_attachment("photo", None),
            1,
        );
        assert_eq!(hidden.body, "New message");
    }

    #[test]
    fn include_message_body_false_replaces_the_body_with_new_message() {
        let note = message_notification(
            false,
            "direct",
            "The Smiths",
            "Anna",
            &protocol_message(),
            1,
        );
        assert_eq!(note.body, "New message");
        assert_eq!(
            apns_payload(&note)["aps"]["alert"]["body"],
            "New message",
            "the redacted body must reach the wire payload"
        );
    }

    #[test]
    fn the_join_request_payload_carries_family_id_and_kind_without_a_thread() {
        let note = join_request_notification("The Smiths", "Junior", 7, 2);
        assert_eq!(
            apns_payload(&note),
            json!({
                "aps": {"alert": {"title": "The Smiths", "body": "Junior asked to join"},
                        "sound": "default", "badge": 2},
                "family_id": 7, "kind": "join_request"
            })
        );
    }

    #[test]
    fn the_joined_payload_welcomes_the_requester_by_family_name() {
        let note = joined_notification("The Smiths", 7, 0);
        assert_eq!(
            apns_payload(&note),
            json!({
                "aps": {"alert": {"title": "The Smiths",
                                  "body": "You're in — welcome to The Smiths"},
                        "sound": "default", "badge": 0},
                "family_id": 7, "kind": "joined"
            })
        );
    }

    #[test]
    fn fcm_data_values_are_strings_for_every_event_kind() {
        let joined = fcm_message(&joined_notification("The Smiths", 7, 0), "t");
        assert_eq!(
            joined["message"]["data"],
            json!({"kind": "joined", "family_id": "7"})
        );
        let request = fcm_message(
            &join_request_notification("The Smiths", "Junior", 7, 0),
            "t",
        );
        assert_eq!(
            request["message"]["data"],
            json!({"kind": "join_request", "family_id": "7"})
        );
        assert!(
            request["message"]["android"]["notification"]
                .get("tag")
                .is_none(),
            "join events have no chat to group under"
        );
    }

    #[test]
    fn the_badge_query_sums_unread_over_the_expected_tables() {
        let query = build_unread_badge_query();
        for table in ["chat_members", "chat_reads", "messages"] {
            assert!(
                query.contains(table),
                "badge query must join {table}: {query}"
            );
        }
        assert!(
            query.contains("msg.sender_id <> $1"),
            "own messages are never unread: {query}"
        );
        assert!(
            query.contains("COALESCE(cr.last_read_message_id, 0)"),
            "a user with no read marker has everything unread: {query}"
        );
    }
}
