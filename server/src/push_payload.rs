//! Push notification composition — pure functions from event context to the
//! exact wire payloads of protocol.md "Push notifications".
//!
//! Kept free of I/O on purpose: the golden unit tests below pin every byte
//! of the APNs/FCM JSON against the protocol document without a network or
//! a database. `events.rs` supplies the context (names, ids, the per-user
//! badge) and the transports in `push.rs` serialize these values verbatim.

use serde_json::{Value, json};

use crate::models::Message;
use crate::push_text;

/// What a push SAYS (docs/protocol.md, "The words of a push"; issue #82): somebody's own words,
/// or a sentence of the server's that goes out in the language of the DEVICE it is sent to.
///
/// Rendered at the transport, device by device, because one event can go to a phone in Russian and
/// another in German; everything up to there is the same notification for every device of a
/// recipient. Compared against a `&str` it is its ENGLISH, which is what the golden tests pin and
/// what a device with no language is sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Text {
    /// Somebody's own words or a name — a message, a caption, a note, a file's name, a display
    /// name, "<Family> — <Sender>" — the same in every language. Nothing is machine-translated.
    Verbatim(String),
    /// One of the server's own sentences: its catalogue key, which is also its English, and the
    /// arguments that fill its `%@` / `%lld`, in order.
    Say {
        key: &'static str,
        args: Vec<String>,
    },
}

impl Text {
    pub fn say(key: &'static str, args: &[&str]) -> Self {
        Text::Say {
            key,
            args: args.iter().map(|arg| arg.to_string()).collect(),
        }
    }

    /// The words for a device that registered `language` — English for none, or for one the table
    /// does not speak.
    pub fn render(&self, language: Option<&str>) -> String {
        match self {
            Text::Verbatim(words) => words.clone(),
            Text::Say { key, args } => {
                let template = push_text::words(language, key).unwrap_or(key);
                push_text::fill(template, args)
            }
        }
    }
}

impl From<String> for Text {
    fn from(words: String) -> Self {
        Text::Verbatim(words)
    }
}

impl PartialEq<&str> for Text {
    fn eq(&self, english: &&str) -> bool {
        self.render(None) == *english
    }
}

impl PartialEq<str> for Text {
    fn eq(&self, english: &str) -> bool {
        self.render(None) == english
    }
}

/// Which protocol event a notification announces, carrying the ids the
/// client deep-links with: `chat_id`/`message_id` open the chat, `family_id`
/// opens the join-requests screen (`join_request`) or the chat list
/// (`joined`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushEvent {
    Message {
        chat_id: i64,
        message_id: i64,
    },
    /// A note added to the family board. Creation only — moves and edits
    /// never push (protocol.md, "Board").
    BoardNote {
        family_id: i64,
        note_id: i64,
    },
    JoinRequest {
        family_id: i64,
    },
    Joined {
        family_id: i64,
    },
    /// A member reported another to the family owner (protocol.md,
    /// "Reporting a member"). Never sent when the report names the owner
    /// themselves — see `events::push_report_created`.
    Report {
        family_id: i64,
    },
}

impl PushEvent {
    /// The `kind` discriminator clients switch on.
    pub fn kind(&self) -> &'static str {
        match self {
            PushEvent::Message { .. } => "message",
            PushEvent::BoardNote { .. } => "board_note",
            PushEvent::JoinRequest { .. } => "join_request",
            PushEvent::Joined { .. } => "joined",
            PushEvent::Report { .. } => "report",
        }
    }

    /// APNs `thread-id` / FCM notification `tag`: messages group per chat
    /// (`chat-<id>`, per protocol.md); join events are one-offs and carry no
    /// grouping key — the document defines none for them.
    fn thread_key(&self) -> Option<String> {
        match self {
            PushEvent::Message { chat_id, .. } => Some(format!("chat-{chat_id}")),
            // All board notes collapse into one entry: several notes pinned
            // at once is one thing to look at, not five alerts.
            PushEvent::BoardNote { family_id, .. } => Some(format!("board-{family_id}")),
            PushEvent::JoinRequest { .. } | PushEvent::Joined { .. } | PushEvent::Report { .. } => {
                None
            }
        }
    }
}

/// A composed, transport-agnostic notification. One instance is built per
/// *recipient user* (not per device): `badge` is that user's total unread
/// across chats at send time, so it differs between recipients of the same
/// message.
#[derive(Debug, Clone, PartialEq)]
pub struct Notification {
    pub title: Text,
    pub body: Text,
    /// Recipient's total unread across chats. APNs-only on the wire — the
    /// protocol's FCM shape carries no badge (Android renders its own).
    pub badge: i64,
    /// Recipient's unread IN THE CHAT this push is about — FCM's
    /// `android.notification.notification_count`, and never the `badge`
    /// total. `None` for everything that is not a message: `board_note`,
    /// `join_request` and `joined` carry no count at all.
    ///
    /// Per chat because Android has no icon-badge API: the only number an
    /// app can offer a launcher is `Notification.number`, which a launcher
    /// SUMS across the app's live notifications — and the app posts one per
    /// chat (`tag: "chat-<id>"`), so a total on each of three chats would
    /// render as three times the total (protocol.md, "Push notifications").
    pub chat_unread: Option<i64>,
    pub event: PushEvent,
}

impl Notification {
    /// The notification as a device on `platform` is told it. A Mac is pushed WHO, never WHAT
    /// (docs/protocol.md, "A Mac is pushed who, never what"; issue #84): the body a server with
    /// `include_message_body = false` would send, whatever the setting — the rule the desktop
    /// clients apply to the notifications they raise themselves, so a Mac says the same thing
    /// whether its app was running or quit. A phone is told the notification as composed.
    pub fn for_platform(&self, platform: &str) -> Notification {
        let mut note = self.clone();
        if platform == "macos" {
            match note.event {
                PushEvent::Message { .. } => note.body = Text::say("New message", &[]),
                PushEvent::BoardNote { .. } => note.body = Text::say("New note", &[]),
                // A report, a join request and a join carry no family text to withhold.
                PushEvent::JoinRequest { .. }
                | PushEvent::Joined { .. }
                | PushEvent::Report { .. } => {}
            }
        }
        note
    }
}

/// Compose the notification for a new message. Title rules per protocol.md:
/// direct chat → the sender's display name; family chat →
/// `"<Family> — <Sender>"`. Body: the message text; the KIND of attachment
/// when a message carries one with no caption (which is how photos are
/// normally sent — an empty body would push a blank line); or
/// `"New message"` when `[push] include_message_body = false`.
/// What to say about a message that has no words: for ONE attachment, the
/// file's name (which is its whole identity) or what kind of thing arrived;
/// for several of ONE kind, a count — `"3 Photos"`, `"2 Videos"`,
/// `"2 Audio"`, `"4 Files"` — where names give way to the count, because
/// four filenames are not a lock-screen line; for a mixed set,
/// `"N attachments"` (protocol.md, "Push notifications"). A location's
/// label only ever appears alone, since a location is always a message's
/// only attachment.
fn attachment_summary(message: &Message) -> Option<Text> {
    // The list, with the legacy single field as its one-element fallback —
    // the server always fills both together, so the fallback only matters
    // to a `Message` built by hand.
    let attachments: &[crate::models::Attachment] = match message.attachments.as_deref() {
        Some(list) if !list.is_empty() => list,
        _ => std::slice::from_ref(message.attachment.as_ref()?),
    };
    if let [attachment] = attachments {
        return Some(match attachment.kind.as_str() {
            // A sticker is a photo on the wire and its own word on a lock
            // screen: "Photo" for a dancing cat promises a picture of
            // somebody (protocol.md, "Sticker pack").
            "photo" if attachment.sticker => Text::say("Sticker", &[]),
            "photo" => Text::say("Photo", &[]),
            // A video message is a video on the wire and its own word on a
            // lock screen, the sticker's case again: "Video" promises a clip
            // somebody chose, "Video message" says somebody is talking to
            // you (protocol.md, "Video messages").
            "video" if attachment.round => Text::say("Video message", &[]),
            "video" => Text::say("Video", &[]),
            // A voice note has no name worth showing, so the kind is the
            // summary; a track picked off a disk may carry one, and that
            // wins.
            "audio" => attachment
                .name
                .clone()
                .filter(|name| !name.is_empty())
                .map(Text::Verbatim)
                .unwrap_or_else(|| Text::say("Audio", &[])),
            // A location's label if it was given one, and the word
            // otherwise. Never the coordinates: an alert on a lock screen
            // is the one place a family member's position should not be
            // readable without unlocking the phone.
            "location" => attachment
                .name
                .clone()
                .filter(|name| !name.is_empty())
                .map(Text::Verbatim)
                .unwrap_or_else(|| Text::say("Location", &[])),
            _ => attachment
                .name
                .clone()
                .filter(|name| !name.is_empty())
                .map(Text::Verbatim)
                .unwrap_or_else(|| Text::say("File", &[])),
        });
    }
    let count = attachments.len().to_string();
    let count = count.as_str();
    let first_kind = attachments[0].kind.as_str();
    Some(
        if attachments
            .iter()
            .all(|attachment| attachment.kind == first_kind)
        {
            match first_kind {
                "photo" => Text::say("%lld Photos", &[count]),
                "video" => Text::say("%lld Videos", &[count]),
                "audio" => Text::say("%lld Audio", &[count]),
                "file" => Text::say("%lld Files", &[count]),
                // A location is always alone, and a kind added later has no
                // plural word here yet — the generic count is safe for both.
                _ => Text::say("%lld attachments", &[count]),
            }
        } else {
            Text::say("%lld attachments", &[count])
        },
    )
}

pub fn message_notification(
    include_message_body: bool,
    chat_kind: &str,
    family_name: &str,
    sender_name: &str,
    message: &Message,
    badge: i64,
    chat_unread: i64,
) -> Notification {
    let title = Text::Verbatim(if chat_kind == "family" {
        format!("{family_name} — {sender_name}")
    } else {
        sender_name.to_string()
    });
    let body = if include_message_body {
        if message.body.is_empty() {
            attachment_summary(message).unwrap_or_else(|| Text::say("New message", &[]))
        } else {
            Text::Verbatim(message.body.clone())
        }
    } else {
        Text::say("New message", &[])
    };
    Notification {
        title,
        body,
        badge,
        // A message push is the one kind that carries it, and it always
        // does — the number is computed beside the badge, from the same
        // rows (see `build_message_unread_query`).
        chat_unread: Some(chat_unread),
        event: PushEvent::Message {
            chat_id: message.chat_id,
            message_id: message.id,
        },
    }
}

/// The message push for a member the message NAMES: the same push, a
/// different title — `"<Family> — <Sender> mentioned you"` — and never a
/// second one (protocol.md, "Mentioning a member"). Family chat only, which
/// is the only chat a mention is accepted in, so the direct-chat title rule
/// has no mention form.
pub fn mention_notification(
    include_message_body: bool,
    family_name: &str,
    sender_name: &str,
    message: &Message,
    badge: i64,
    chat_unread: i64,
) -> Notification {
    let mut note = message_notification(
        include_message_body,
        "family",
        family_name,
        sender_name,
        message,
        badge,
        chat_unread,
    );
    note.title = Text::say("%@ — %@ mentioned you", &[family_name, sender_name]);
    note
}

/// Compose the notification for a new board note.
///
/// Title `"<Family> — <Author>"`, body the note's text — governed by the
/// SAME `include_message_body` switch as a message, because a note is family
/// content in exactly the way a message is.
///
/// A member the note NAMES gets `"<Family> — <Author> mentioned you"`
/// instead, exactly as a mentioned member does in the family chat
/// (protocol.md, "Board"). It is the same notification with a different
/// title, never a second one: naming somebody does not push twice.
/// One board-note alert's ingredients. A struct rather than eight
/// arguments — the codebase carries no `allow` attributes, and "who wrote
/// it, on whose board, and does it name this reader" reads better named
/// than positional.
pub struct BoardNoteAlert<'a> {
    pub include_body: bool,
    pub family_name: &'a str,
    pub author_name: &'a str,
    pub family_id: i64,
    pub note_id: i64,
    pub text: &'a str,
    pub badge: i64,
    /// Whether THIS reader is one of the members the note names.
    pub mentioned: bool,
}

pub fn board_note_notification(alert: BoardNoteAlert<'_>) -> Notification {
    let BoardNoteAlert {
        include_body,
        family_name,
        author_name,
        family_id,
        note_id,
        text,
        badge,
        mentioned,
    } = alert;
    Notification {
        title: if mentioned {
            Text::say("%@ — %@ mentioned you", &[family_name, author_name])
        } else {
            Text::Verbatim(format!("{family_name} — {author_name}"))
        },
        body: if include_body {
            Text::Verbatim(text.to_string())
        } else {
            Text::say("New note", &[])
        },
        badge,
        // A note is not a message: no chat, and so no per-chat count.
        chat_unread: None,
        event: PushEvent::BoardNote { family_id, note_id },
    }
}

/// Compose the notification the family owner gets when a join request is
/// created: title = the family name, body = `"<Display Name> asked to
/// join"`.
/// The owner's alert that somebody filed a report.
///
/// It carries NO reported text, regardless of `include_message_body`: the
/// excerpt is the very content somebody asked to have looked at, and a lock
/// screen is where it must not be readable. The body is the fixed English
/// "New report", which the switch does not vary because there is nothing
/// there to withhold (protocol.md, "Push notifications") — in the device's language, like every
/// sentence of the server's own.
pub fn report_notification(family_name: &str, family_id: i64, badge: i64) -> Notification {
    Notification {
        title: Text::Verbatim(family_name.to_string()),
        body: Text::say("New report", &[]),
        badge,
        chat_unread: None,
        event: PushEvent::Report { family_id },
    }
}

pub fn join_request_notification(
    family_name: &str,
    requester_display_name: &str,
    family_id: i64,
    badge: i64,
) -> Notification {
    Notification {
        title: Text::Verbatim(family_name.to_string()),
        body: Text::say("%@ asked to join", &[requester_display_name]),
        badge,
        chat_unread: None,
        event: PushEvent::JoinRequest { family_id },
    }
}

/// Compose the notification the requester gets when their join request is
/// approved.
pub fn joined_notification(family_name: &str, family_id: i64, badge: i64) -> Notification {
    Notification {
        title: Text::Verbatim(family_name.to_string()),
        body: Text::say("You're in — welcome to %@", &[family_name]),
        badge,
        chat_unread: None,
        event: PushEvent::Joined { family_id },
    }
}

/// The APNs request body, exactly as protocol.md specifies it: the `aps`
/// dictionary plus top-level `kind` and the event's ids — its words in `language`, the one the
/// device registered (docs/protocol.md, "The words of a push").
pub fn apns_payload(note: &Notification, language: Option<&str>) -> Value {
    let mut aps = json!({
        "alert": {"title": note.title.render(language), "body": note.body.render(language)},
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
        PushEvent::BoardNote { family_id, note_id } => {
            payload["family_id"] = json!(family_id);
            payload["note_id"] = json!(note_id);
        }
        PushEvent::JoinRequest { family_id }
        | PushEvent::Joined { family_id }
        | PushEvent::Report { family_id } => {
            payload["family_id"] = json!(family_id);
        }
    }
    payload
}

/// The FCM HTTP v1 request body per protocol.md: `notification` + `data` so
/// the system tray renders when the app process is dead. FCM `data` values
/// MUST be strings — ids are stringified here, never sent as numbers.
/// `android.notification.notification_count` is the exception that is not
/// one: it lives outside `data`, so it goes on the wire as a NUMBER.
pub fn fcm_message(note: &Notification, push_token: &str, language: Option<&str>) -> Value {
    let mut data = json!({"kind": note.event.kind()});
    match note.event {
        PushEvent::Message {
            chat_id,
            message_id,
        } => {
            data["chat_id"] = json!(chat_id.to_string());
            data["message_id"] = json!(message_id.to_string());
        }
        PushEvent::BoardNote { family_id, note_id } => {
            data["family_id"] = json!(family_id.to_string());
            data["note_id"] = json!(note_id.to_string());
        }
        PushEvent::JoinRequest { family_id }
        | PushEvent::Joined { family_id }
        | PushEvent::Report { family_id } => {
            data["family_id"] = json!(family_id.to_string());
        }
    }
    let mut android_notification = json!({"channel_id": "messages"});
    if let Some(tag) = note.event.thread_key() {
        android_notification["tag"] = json!(tag);
    }
    // A NUMBER, deliberately: the everything-is-a-string rule above governs
    // `data` alone, and `notification_count` is a field of the notification
    // object FCM itself types as an integer. Present on message pushes only.
    if let Some(chat_unread) = note.chat_unread {
        android_notification["notification_count"] = json!(chat_unread);
    }
    json!({"message": {
        "token": push_token,
        "notification": {"title": note.title.render(language), "body": note.body.render(language)},
        "data": data,
        "android": {"priority": "HIGH", "notification": android_notification},
    }})
}

/// A voice call to ring a phone with (docs/protocol.md, "Incoming calls").
///
/// This is NOT a `Notification`: a call does not draw a banner, it makes a
/// device ring, so it carries no title/body/badge and takes its own path
/// through the transports (APNs `voip` push type, FCM data-only). The device
/// reports the call to CallKit / a full-screen intent from these fields and
/// then connects its socket, over which the offer is replayed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallPush {
    pub call_id: String,
    pub chat_id: i64,
    pub from_user_id: i64,
    /// The caller's display name, shown while the phone rings. On the wire
    /// even under `include_message_body = false`: a sender's name titles
    /// every push under that setting too, and a phone that rings without
    /// saying who is calling is worse than one that does not.
    pub caller_name: String,
    /// A VIDEO call: what makes the woken device ring with a camera UI,
    /// `hasVideo` on CallKit included. On the wire when and only when true
    /// (protocol.md, "Incoming calls" and "Video").
    pub video: bool,
    /// The ring timeout, as the push's own expiry: a call push FCM or APNs
    /// could not deliver while it rang must not wake a phone into a call
    /// that is already over.
    pub ring_timeout_secs: u64,
}

/// The APNs VoIP push body (protocol.md, "Incoming calls"). No `aps`
/// dictionary: a VoIP push has nothing for the system to draw, and the app
/// reports the call to CallKit from these fields the moment it arrives.
pub fn apns_voip_payload(call: &CallPush) -> Value {
    let mut payload = json!({
        "kind": "call",
        "call_id": call.call_id,
        "chat_id": call.chat_id,
        "from_user_id": call.from_user_id,
        "caller_name": call.caller_name,
    });
    // Present when and only when it is a video call — absent, not false, on
    // a voice call, so the voice payload stays byte-identical.
    if call.video {
        payload["video"] = json!(true);
    }
    payload
}

/// The FCM call message (protocol.md, "Incoming calls"): data only and no
/// `notification` block, so the app process is woken to ring rather than the
/// tray asked to draw. `data` values are strings; `ttl` is `"<secs>s"`.
pub fn fcm_call_message(call: &CallPush, push_token: &str) -> Value {
    let mut data = json!({
        "kind": "call",
        "call_id": call.call_id,
        "chat_id": call.chat_id.to_string(),
        "from_user_id": call.from_user_id.to_string(),
        "caller_name": call.caller_name,
    });
    // The STRING "true", because FCM types `data` as map<string, string> —
    // and present only on a video call, keeping the voice payload
    // byte-identical.
    if call.video {
        data["video"] = json!("true");
    }
    json!({"message": {
        "token": push_token,
        "data": data,
        "android": {"priority": "HIGH", "ttl": format!("{}s", call.ring_timeout_secs)},
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
       AND msg.sender_id <> $1
       AND NOT EXISTS (
             SELECT 1 FROM chats c
              WHERE c.id = m.chat_id AND c.kind = 'direct'
                AND EXISTS (
                      SELECT 1 FROM member_blocks mb
                       WHERE mb.blocker_user_id = $1
                         AND mb.blocked_user_id =
                             CASE WHEN c.user_a_id = $1 THEN c.user_b_id ELSE c.user_a_id END
                )
       )"
}

/// The one SQL statement behind BOTH numbers a MESSAGE push carries: the
/// recipient's total unread across chats (APNs `badge`) and their unread in
/// the chat this push is about (FCM `notification_count`). `$1` is the
/// recipient's user id, `$2` the chat.
///
/// One statement rather than two, and a FILTER rather than a second
/// predicate: the per-chat number counts a SUBSET of the very rows the
/// badge counts, so the two can never disagree about what "unread" means —
/// which is the whole risk in shipping a second count. That predicate is
/// also the one `GET /chats` measures its `unread_count` with, word for
/// word: newer than my read marker (0 when I have never reported one), and
/// not sent by me.
///
/// Both queries carry one further exclusion, at the CHAT level rather than
/// the message level: a direct chat the recipient has blocked into
/// contributes nothing to any unread count and nothing to the badge, because
/// it is gone from every path they can reach it by and a badge they could
/// never clear would itself be a quantity that moved when they blocked
/// somebody (protocol.md, "Blocking a member").
///
/// The FAMILY chat is deliberately untouched: a hidden row there still
/// counts, because the count is the other half of the read marker and
/// projecting one without the other desynchronises them.
pub fn build_message_unread_query() -> &'static str {
    "SELECT count(*) AS badge,
            count(*) FILTER (WHERE msg.chat_id = $2) AS chat_unread
     FROM chat_members m
     JOIN messages msg ON msg.chat_id = m.chat_id
     LEFT JOIN chat_reads cr ON cr.chat_id = m.chat_id AND cr.user_id = m.user_id
     WHERE m.user_id = $1
       AND msg.id > COALESCE(cr.last_read_message_id, 0)
       AND msg.sender_id <> $1
       AND NOT EXISTS (
             SELECT 1 FROM chats c
              WHERE c.id = m.chat_id AND c.kind = 'direct'
                AND EXISTS (
                      SELECT 1 FROM member_blocks mb
                       WHERE mb.blocker_user_id = $1
                         AND mb.blocked_user_id =
                             CASE WHEN c.user_a_id = $1 THEN c.user_b_id ELSE c.user_a_id END
                )
       )"
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
            thread_root_id: None,
            reply_count: None,
            edited_at: None,
            edit_seq: None,
            attachment: None,
            attachments: None,
            reaction_seq: None,
            poll: None,
            call: None,
            mentions: None,
        }
    }

    fn protocol_call() -> CallPush {
        CallPush {
            call_id: "6a1f0c3e-0000-4000-8000-000000000001".to_string(),
            chat_id: 42,
            from_user_id: 7,
            caller_name: "Anna".to_string(),
            video: false,
            ring_timeout_secs: 45,
        }
    }

    #[test]
    fn the_apns_voip_payload_has_no_aps_and_carries_the_call_fields() {
        let payload = apns_voip_payload(&protocol_call());
        assert_eq!(
            payload,
            json!({
                "kind": "call",
                "call_id": "6a1f0c3e-0000-4000-8000-000000000001",
                "chat_id": 42, "from_user_id": 7, "caller_name": "Anna"
            })
        );
        assert!(payload.get("aps").is_none(), "a VoIP push has no aps dict");
        assert!(
            payload.get("video").is_none(),
            "a voice call's payload stays byte-identical: no video key at all"
        );
    }

    /// The video flag on both transports, per protocol.md "Incoming calls":
    /// APNs carries `"video": true` (a JSON boolean — the VoIP payload is
    /// not FCM `data`), FCM carries the STRING `"true"` because `data` is
    /// map<string, string>. Present when and only when it is a video call —
    /// the voice payloads above pin the absence.
    #[test]
    fn a_video_call_push_carries_the_flag_on_both_transports() {
        let call = CallPush {
            video: true,
            ..protocol_call()
        };
        assert_eq!(
            apns_voip_payload(&call),
            json!({
                "kind": "call",
                "call_id": "6a1f0c3e-0000-4000-8000-000000000001",
                "chat_id": 42, "from_user_id": 7, "caller_name": "Anna",
                "video": true
            })
        );
        let message = fcm_call_message(&call, "token-1");
        assert_eq!(
            message,
            json!({"message": {
                "token": "token-1",
                "data": {"kind": "call", "call_id": "6a1f0c3e-0000-4000-8000-000000000001",
                         "chat_id": "42", "from_user_id": "7", "caller_name": "Anna",
                         "video": "true"},
                "android": {"priority": "HIGH", "ttl": "45s"}
            }})
        );
        assert!(
            message["message"]["data"]
                .as_object()
                .expect("data is an object")
                .values()
                .all(Value::is_string),
            "every value in FCM data is a string, the video flag included"
        );
    }

    #[test]
    fn the_fcm_call_message_is_data_only_with_a_string_ttl() {
        let message = fcm_call_message(&protocol_call(), "token-1");
        assert_eq!(
            message,
            json!({"message": {
                "token": "token-1",
                "data": {"kind": "call", "call_id": "6a1f0c3e-0000-4000-8000-000000000001",
                         "chat_id": "42", "from_user_id": "7", "caller_name": "Anna"},
                "android": {"priority": "HIGH", "ttl": "45s"}
            }})
        );
        assert!(
            message["message"].get("notification").is_none(),
            "a call push must not carry a notification block — the app rings, the tray does not"
        );
        assert!(
            message["message"]["data"].get("video").is_none(),
            "a voice call's data stays byte-identical: no video key at all"
        );
    }

    /// A mention is the message's own push with a different title and
    /// nothing else different: same body, same badge, same event, same
    /// `kind` — so routing and grouping never change.
    #[test]
    fn a_mention_changes_the_title_and_nothing_else() {
        let message = protocol_message();
        let plain = message_notification(true, "family", "The Smiths", "Anna", &message, 3, 1);
        let named = mention_notification(true, "The Smiths", "Anna", &message, 3, 1);
        assert_eq!(named.title, "The Smiths — Anna mentioned you");
        assert_eq!(named.body, plain.body);
        assert_eq!(named.badge, plain.badge);
        assert_eq!(named.chat_unread, plain.chat_unread);
        assert_eq!(named.event, plain.event);
        assert_eq!(named.event.kind(), "message");
        // Under the privacy switch the body is withheld exactly as it is
        // for everybody else — the title says who, never what.
        let hidden = mention_notification(false, "The Smiths", "Anna", &message, 3, 1);
        assert_eq!(hidden.body, "New message");
        assert_eq!(hidden.title, "The Smiths — Anna mentioned you");
    }

    #[test]
    fn the_apns_message_payload_matches_the_protocol_example_exactly() {
        let note = message_notification(
            true,
            "direct",
            "The Smiths",
            "Anna",
            &protocol_message(),
            3,
            3,
        );
        assert_eq!(
            apns_payload(&note, None),
            json!({
                "aps": {"alert": {"title": "Anna", "body": "Dinner at 7?"}, "sound": "default",
                        "badge": 3, "thread-id": "chat-42"},
                "chat_id": 42, "message_id": 1338, "kind": "message"
            })
        );
    }

    #[test]
    fn the_fcm_message_matches_the_protocol_example_exactly() {
        let note = message_notification(
            true,
            "direct",
            "The Smiths",
            "Anna",
            &protocol_message(),
            3,
            3,
        );
        assert_eq!(
            fcm_message(&note, "token-1", None),
            json!({"message": {"token": "token-1",
                "notification": {"title": "Anna", "body": "Dinner at 7?"},
                "data": {"kind": "message", "chat_id": "42", "message_id": "1338"},
                "android": {"priority": "HIGH",
                            "notification": {"channel_id": "messages", "tag": "chat-42",
                                             "notification_count": 3}}}})
        );
    }

    /// The two numbers a message push carries are DIFFERENT numbers, and
    /// the wire is where that has to show: APNs takes the total, FCM takes
    /// the count for this one chat. Composed with 9 and 2 precisely so a
    /// payload that reached for the wrong field cannot pass.
    #[test]
    fn the_fcm_notification_count_is_the_chat_unread_and_never_the_badge_total() {
        let note = message_notification(
            true,
            "direct",
            "The Smiths",
            "Anna",
            &protocol_message(),
            9,
            2,
        );
        assert_eq!(
            fcm_message(&note, "token-1", None)["message"]["android"]["notification"],
            json!({"channel_id": "messages", "tag": "chat-42", "notification_count": 2})
        );
        assert_eq!(
            apns_payload(&note, None)["aps"]["badge"],
            json!(9),
            "APNs is unchanged: the badge stays the total across chats"
        );
        assert!(
            !apns_payload(&note, None)
                .to_string()
                .contains("notification_count"),
            "notification_count is an FCM field and has no place in the APNs payload"
        );
    }

    /// A JSON number, not a string: the everything-is-a-string rule governs
    /// `data`, and `notification_count` is not in `data`. A string here is
    /// an FCM 400 (INVALID_ARGUMENT), so this is the byte that decides
    /// whether Android hears anything at all.
    #[test]
    fn the_notification_count_goes_on_the_wire_as_a_number() {
        let note = message_notification(
            true,
            "direct",
            "The Smiths",
            "Anna",
            &protocol_message(),
            3,
            3,
        );
        let count = &fcm_message(&note, "t", None)["message"]["android"]["notification"]["notification_count"];
        assert!(count.is_number(), "must be a JSON number, got {count}");
        assert_eq!(count.as_i64(), Some(3));
        assert!(
            fcm_message(&note, "t", None)["message"]["data"]
                .as_object()
                .expect("data is an object")
                .values()
                .all(Value::is_string),
            "…while every value in `data` is still a string"
        );
    }

    /// Zero is a real count and must still be sent: it is what a recipient
    /// whose other device has already read the chat is owed, and a launcher
    /// showing a stale 3 is worse than one showing nothing.
    #[test]
    fn a_zero_chat_unread_is_still_sent() {
        let note = message_notification(
            true,
            "direct",
            "The Smiths",
            "Anna",
            &protocol_message(),
            0,
            0,
        );
        assert_eq!(
            fcm_message(&note, "t", None)["message"]["android"]["notification"]["notification_count"],
            json!(0)
        );
    }

    /// It rides on message pushes ALONE — protocol.md omits it from the
    /// other three kinds, which have no chat to count in.
    #[test]
    fn only_message_pushes_carry_a_notification_count() {
        let cases = [
            fcm_message(
                &board_note_notification(BoardNoteAlert {
                    include_body: true,
                    family_name: "The Smiths",
                    author_name: "Junior",
                    family_id: 7,
                    note_id: 3,
                    text: "Milk",
                    badge: 0,
                    mentioned: false,
                }),
                "t",
                None,
            ),
            fcm_message(
                &join_request_notification("The Smiths", "Junior", 7, 0),
                "t",
                None,
            ),
            fcm_message(&joined_notification("The Smiths", 7, 0), "t", None),
        ];
        for message in cases {
            let kind = message["message"]["data"]["kind"].clone();
            assert!(
                message["message"]["android"]["notification"]
                    .get("notification_count")
                    .is_none(),
                "a {kind} push has no chat to count in: {message}"
            );
        }
    }

    #[test]
    fn a_family_chat_title_combines_family_and_sender_with_an_em_dash() {
        let note = message_notification(
            true,
            "family",
            "The Smiths",
            "Anna",
            &protocol_message(),
            1,
            1,
        );
        assert_eq!(note.title, "The Smiths — Anna");
    }

    #[test]
    fn a_direct_chat_title_is_just_the_sender_display_name() {
        let note = message_notification(
            true,
            "direct",
            "The Smiths",
            "Anna",
            &protocol_message(),
            1,
            1,
        );
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
                    latitude: None,
                    longitude: None,
                    accuracy_m: None,
                    sticker: false,
                    round: false,
                    waveform: None,
                }),
                ..protocol_message()
            }
        }

        // A shared location alerts with its label, or the word — never the
        // coordinates, which would put somebody's position on a locked
        // screen.
        let pin = message_notification(
            true,
            "direct",
            "The Smiths",
            "Anna",
            &with_attachment("location", None),
            1,
            1,
        );
        assert_eq!(pin.body, "Location");
        let labelled = message_notification(
            true,
            "direct",
            "The Smiths",
            "Anna",
            &with_attachment("location", Some("Home")),
            1,
            1,
        );
        assert_eq!(labelled.body, "Home");

        let photo = message_notification(
            true,
            "direct",
            "The Smiths",
            "Anna",
            &with_attachment("photo", None),
            1,
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
            1,
        );
        assert_eq!(file.body, "Rechnung.pdf");

        // A caption still wins over the summary.
        let captioned = Message {
            body: "at the lake".to_string(),
            ..with_attachment("photo", None)
        };
        let note = message_notification(true, "direct", "S", "Anna", &captioned, 1, 1);
        assert_eq!(note.body, "at the lake");

        // And the privacy switch still hides everything.
        let hidden = message_notification(
            false,
            "direct",
            "S",
            "Anna",
            &with_attachment("photo", None),
            1,
            1,
        );
        assert_eq!(hidden.body, "New message");
    }

    /// Several attachments push a COUNT: `"3 Photos"` / `"2 Videos"` /
    /// `"2 Audio"` / `"4 Files"` when they are all one kind — names give
    /// way to the count, because four filenames are not a lock-screen line
    /// — and `"N attachments"` for a mixed set (protocol.md). A caption
    /// still wins over all of it, and the privacy switch still hides
    /// everything.
    #[test]
    fn several_uncaptioned_attachments_push_a_count() {
        fn attachment(kind: &str, name: Option<&str>) -> crate::models::Attachment {
            crate::models::Attachment {
                id: 34,
                kind: kind.to_string(),
                mime: "application/octet-stream".to_string(),
                size: 4096,
                width: None,
                height: None,
                duration_ms: None,
                has_preview: false,
                name: name.map(str::to_string),
                latitude: None,
                longitude: None,
                accuracy_m: None,
                sticker: false,
                round: false,
                waveform: None,
            }
        }
        fn with_attachments(list: Vec<crate::models::Attachment>) -> Message {
            Message {
                body: String::new(),
                attachment: list.first().cloned(),
                attachments: Some(list),
                ..protocol_message()
            }
        }
        let summary = |message: &Message| {
            message_notification(true, "direct", "The Smiths", "Anna", message, 1, 1).body
        };

        let photos = with_attachments(vec![
            attachment("photo", None),
            attachment("photo", None),
            attachment("photo", None),
        ]);
        assert_eq!(summary(&photos), "3 Photos");
        let videos = with_attachments(vec![attachment("video", None), attachment("video", None)]);
        assert_eq!(summary(&videos), "2 Videos");
        let audio = with_attachments(vec![
            attachment("audio", Some("Track A.mp3")),
            attachment("audio", Some("Track B.mp3")),
        ]);
        assert_eq!(summary(&audio), "2 Audio", "names give way to the count");
        let files = with_attachments(vec![
            attachment("file", Some("a.pdf")),
            attachment("file", Some("b.pdf")),
            attachment("file", Some("c.pdf")),
            attachment("file", Some("d.pdf")),
        ]);
        assert_eq!(summary(&files), "4 Files", "names give way to the count");

        let mixed = with_attachments(vec![
            attachment("photo", None),
            attachment("file", Some("a.pdf")),
            attachment("audio", None),
        ]);
        assert_eq!(summary(&mixed), "3 attachments");

        // ONE attachment in the array is exactly the singular wording the
        // test above pins — the array changes nothing about what one photo
        // or one named file says.
        assert_eq!(
            summary(&with_attachments(vec![attachment("photo", None)])),
            "Photo"
        );
        assert_eq!(
            summary(&with_attachments(vec![attachment(
                "file",
                Some("Rechnung.pdf")
            )])),
            "Rechnung.pdf"
        );

        // A caption still wins.
        let captioned = Message {
            body: "at the lake".to_string(),
            ..with_attachments(vec![attachment("photo", None), attachment("photo", None)])
        };
        assert_eq!(summary(&captioned), "at the lake");

        // And the privacy switch still hides everything.
        let hidden = message_notification(
            false,
            "direct",
            "S",
            "Anna",
            &with_attachments(vec![attachment("photo", None), attachment("photo", None)]),
            1,
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
            1,
        );
        assert_eq!(note.body, "New message");
        assert_eq!(
            apns_payload(&note, None)["aps"]["alert"]["body"],
            "New message",
            "the redacted body must reach the wire payload"
        );
    }

    #[test]
    fn the_join_request_payload_carries_family_id_and_kind_without_a_thread() {
        let note = join_request_notification("The Smiths", "Junior", 7, 2);
        assert_eq!(
            apns_payload(&note, None),
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
            apns_payload(&note, None),
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
        let joined = fcm_message(&joined_notification("The Smiths", 7, 0), "t", None);
        assert_eq!(
            joined["message"]["data"],
            json!({"kind": "joined", "family_id": "7"})
        );
        let request = fcm_message(
            &join_request_notification("The Smiths", "Junior", 7, 0),
            "t",
            None,
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

    /// The per-chat count and the badge must never be able to disagree
    /// about what "unread" means — so the message-push query counts both
    /// from ONE set of rows, the per-chat number being a FILTER over them.
    /// This test pins that structure: same predicate, one statement.
    #[test]
    fn the_message_push_query_counts_the_chat_as_a_subset_of_the_badge() {
        let query = build_message_unread_query();
        for predicate in [
            "msg.id > COALESCE(cr.last_read_message_id, 0)",
            "msg.sender_id <> $1",
        ] {
            assert!(
                query.contains(predicate),
                "the per-chat count must use the badge's predicate `{predicate}`: {query}"
            );
            assert!(
                build_unread_badge_query().contains(predicate),
                "…which is also the badge's own: {predicate}"
            );
        }
        assert!(
            query.contains("count(*) FILTER (WHERE msg.chat_id = $2)"),
            "the chat's count is a filter over the badge's rows, not a second query: {query}"
        );
        // One TOP-LEVEL source, which is what "one round trip" means here.
        // This used to count every "FROM" in the string; the hidden-DM
        // exclusion added two correlated subqueries, which are still one
        // statement, so the crude count started measuring the wrong thing.
        // `FROM chat_members` is the source the badge is computed over and
        // it appears exactly once — splitting this into two queries is
        // still caught.
        assert_eq!(
            query.matches("FROM chat_members").count(),
            1,
            "both numbers come from one statement and one round trip: {query}"
        );
        assert_eq!(
            query.matches("count(*)").count(),
            2,
            "exactly the badge and its filtered subset, never a second count: {query}"
        );
    }

    /// A sticker is a photo on the wire and its own word on a lock screen
    /// (protocol.md, "Sticker pack"): the flag changes what one picture is
    /// CALLED, and nothing about how an album is counted.
    #[test]
    fn a_sticker_is_called_a_sticker() {
        fn picture(sticker: bool) -> crate::models::Attachment {
            crate::models::Attachment {
                id: 90,
                kind: "photo".to_string(),
                mime: "image/webp".to_string(),
                size: 4096,
                width: None,
                height: None,
                duration_ms: None,
                has_preview: false,
                name: None,
                latitude: None,
                longitude: None,
                accuracy_m: None,
                sticker,
                round: false,
                waveform: None,
            }
        }
        fn carrying(picture: crate::models::Attachment) -> Message {
            Message {
                body: String::new(),
                attachment: Some(picture.clone()),
                attachments: Some(vec![picture]),
                ..protocol_message()
            }
        }
        assert_eq!(
            attachment_summary(&carrying(picture(true)))
                .map(|text| text.render(None))
                .as_deref(),
            Some("Sticker")
        );
        // The same WebP sent as an ordinary photo is still a photo.
        assert_eq!(
            attachment_summary(&carrying(picture(false)))
                .map(|text| text.render(None))
                .as_deref(),
            Some("Photo")
        );
        // And the operator's switch still withholds it.
        let hidden =
            message_notification(false, "direct", "", "Anna", &carrying(picture(true)), 1, 1);
        assert_eq!(hidden.body, "New message");
    }

    /// A Mac is pushed who, never what (docs/protocol.md; issue #84): the body a server withholding
    /// message bodies would send, whatever the setting — and a phone is told as composed.
    #[test]
    fn a_mac_is_pushed_who_never_what() {
        let message = message_notification(
            true,
            "family",
            "The Smiths",
            "Anna",
            &protocol_message(),
            1,
            1,
        );
        let mac = message.for_platform("macos");
        assert_eq!(mac.body, "New message");
        assert_eq!(mac.body.render(Some("ru")), "Новое сообщение");
        assert_eq!(
            mac.title, message.title,
            "the title is the same for everyone"
        );
        assert_eq!(message.for_platform("ios"), message);
        assert_eq!(message.for_platform("android"), message);

        let note = board_note_notification(BoardNoteAlert {
            include_body: true,
            family_name: "The Smiths",
            author_name: "Anna",
            family_id: 7,
            note_id: 3,
            text: "Milk",
            badge: 0,
            mentioned: false,
        });
        assert_eq!(note.body, "Milk");
        assert_eq!(note.for_platform("macos").body, "New note");

        // Nothing to withhold: a report, a join request and a join say the same to a Mac.
        for other in [
            report_notification("The Smiths", 7, 0),
            join_request_notification("The Smiths", "Junior", 7, 0),
            joined_notification("The Smiths", 7, 0),
        ] {
            assert_eq!(other.for_platform("macos"), other);
        }
    }

    /// The server's own words go out in the device's language and somebody's own words exactly as
    /// written (docs/protocol.md, "The words of a push"; issue #82): one message, three devices.
    #[test]
    fn a_push_speaks_the_devices_language_and_never_translates_what_somebody_wrote() {
        fn video(id: i64, round: bool) -> crate::models::Attachment {
            crate::models::Attachment {
                id,
                kind: "video".to_string(),
                mime: "video/mp4".to_string(),
                size: 1_649_700,
                width: Some(480),
                height: Some(480),
                duration_ms: Some(23_400),
                has_preview: true,
                name: None,
                latitude: None,
                longitude: None,
                accuracy_m: None,
                sticker: false,
                round,
                waveform: None,
            }
        }
        let carrying = |videos: Vec<crate::models::Attachment>| Message {
            body: String::new(),
            attachment: videos.first().cloned(),
            attachments: Some(videos),
            ..protocol_message()
        };
        let one = message_notification(
            true,
            "family",
            "The Smiths",
            "Anna",
            &carrying(vec![video(91, true)]),
            1,
            1,
        );
        assert_eq!(one.body.render(Some("ru")), "Видеосообщение");
        assert_eq!(one.body.render(Some("de")), "Videonachricht");
        assert_eq!(one.body.render(None), "Video message");
        // The title is names, and names are never translated.
        assert_eq!(one.title.render(Some("ja")), "The Smiths — Anna");

        let three = message_notification(
            true,
            "direct",
            "",
            "Anna",
            &carrying(vec![video(91, false), video(92, false), video(93, false)]),
            1,
            1,
        );
        assert_eq!(three.body, "3 Videos");
        assert!(
            three.body.render(Some("ru")).contains('3'),
            "the count survives translation"
        );
        assert_ne!(three.body.render(Some("ru")), "3 Videos");

        // A caption is somebody's words: the same in every language.
        let captioned = Message {
            body: "at the lake".to_string(),
            ..carrying(vec![video(91, false)])
        };
        let note = message_notification(true, "direct", "", "Anna", &captioned, 1, 1);
        for language in [None, Some("ru"), Some("zh-Hans")] {
            assert_eq!(note.body.render(language), "at the lake");
        }

        // A sentence with arguments takes them in the translation's order.
        let named = mention_notification(true, "The Smiths", "Anna", &protocol_message(), 1, 1);
        assert_eq!(
            named.title.render(Some("de")),
            "The Smiths — Anna hat dich erwähnt"
        );
        assert_eq!(named.title, "The Smiths — Anna mentioned you");

        // And the transports say it: APNs and FCM, the same push in two languages.
        assert_eq!(
            apns_payload(&joined_notification("The Smiths", 7, 0), Some("fr"))["aps"]["alert"]["body"],
            json!("C’est fait — bienvenue dans The Smiths")
        );
        assert_eq!(
            fcm_message(&report_notification("The Smiths", 7, 0), "t", Some("ru"))["message"]["notification"]
                ["body"],
            json!("Новая жалоба")
        );
    }

    /// A video message is a video on the wire and its own word on a lock
    /// screen (protocol.md, "Video messages"): the flag changes what one
    /// video is CALLED, nothing about how several are counted, and nothing
    /// about the operator's switch.
    #[test]
    fn a_video_message_is_called_a_video_message() {
        fn video(id: i64, round: bool) -> crate::models::Attachment {
            crate::models::Attachment {
                id,
                kind: "video".to_string(),
                mime: "video/mp4".to_string(),
                size: 1_649_700,
                width: Some(480),
                height: Some(480),
                duration_ms: Some(23_400),
                has_preview: true,
                name: None,
                latitude: None,
                longitude: None,
                accuracy_m: None,
                sticker: false,
                round,
                waveform: None,
            }
        }
        fn carrying(videos: Vec<crate::models::Attachment>) -> Message {
            Message {
                body: String::new(),
                attachment: videos.first().cloned(),
                attachments: Some(videos),
                ..protocol_message()
            }
        }
        assert_eq!(
            attachment_summary(&carrying(vec![video(91, true)]))
                .map(|text| text.render(None))
                .as_deref(),
            Some("Video message")
        );
        // The same square MP4 sent as an ordinary video is still a video.
        assert_eq!(
            attachment_summary(&carrying(vec![video(91, false)]))
                .map(|text| text.render(None))
                .as_deref(),
            Some("Video")
        );
        // Two videos are counted as videos, whatever either says — the
        // server never sends that shape, and the word is for ONE.
        assert_eq!(
            attachment_summary(&carrying(vec![video(91, true), video(92, false)]))
                .map(|text| text.render(None))
                .as_deref(),
            Some("2 Videos")
        );
        // The word is what a body-less message pushes; the operator's
        // switch still withholds it.
        let shown = message_notification(
            true,
            "direct",
            "",
            "Anna",
            &carrying(vec![video(91, true)]),
            1,
            1,
        );
        assert_eq!(shown.body, "Video message");
        let hidden = message_notification(
            false,
            "direct",
            "",
            "Anna",
            &carrying(vec![video(91, true)]),
            1,
            1,
        );
        assert_eq!(hidden.body, "New message");
    }
}
