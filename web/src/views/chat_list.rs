//! The list of chats down the side: the family chat always and first,
//! direct chats and the assistant's once they exist, newest conversation
//! first, each with its unread count and — while an unread message names
//! the reader — an "@".

use std::collections::{HashMap, HashSet};

use fc_text::i18n::{t, tn};
use yew::prelude::*;

use crate::model::{ChatListItem, Message};
use crate::time;
use crate::views::avatar::Avatar;

#[derive(Properties, PartialEq)]
pub struct ChatListProps {
    /// Already in the order drawn (`Store::sorted_chats`).
    pub chats: Vec<ChatListItem>,
    /// userId → name, for the direct chats: those carry no title, and are
    /// called by the person at the other end.
    pub names: HashMap<i64, String>,
    pub blocked: HashSet<i64>,
    /// userId → `avatar_version`, for a direct chat's picture: the person
    /// at the other end.
    #[prop_or_default]
    pub avatars: HashMap<i64, i64>,
    pub my_user_id: i64,
    pub selected: Option<i64>,
    pub on_select: Callback<i64>,
    /// Wall-clock now, for the short times on the rows.
    pub now_ms: f64,
}

/// The one line under a chat's name.
///
/// A blocked member's message is the hidden row here too, with no name and
/// no reveal — a list is not where a person peeks (docs/protocol.md, `GET
/// /chats`). A call record reads as a call and never as its English
/// placeholder body; a caption-less attachment reads as what it is, because
/// a preview with nothing in it is a row that looks like nothing happened.
pub fn preview(message: &Message, my_user_id: i64, blocked: &HashSet<i64>) -> String {
    if message.sender_id != my_user_id && blocked.contains(&message.sender_id) {
        return t("Hidden — blocked member").to_string();
    }
    if let Some(call) = &message.call {
        return crate::views::bubble::call_record_line(call, message.sender_id == my_user_id);
    }
    if !message.body.is_empty() {
        return message.body.lines().next().unwrap_or_default().to_string();
    }
    let attachments = message.attachments();
    match attachments
        .first()
        .map(|attachment| attachment.kind.as_str())
    {
        Some("photo") if attachments.len() > 1 => tn("%lld Photos", attachments.len() as i64),
        Some("photo") => t("Photo").to_string(),
        Some("video") => t("Video").to_string(),
        Some("audio") => t("Voice message").to_string(),
        Some("location") => t("Location").to_string(),
        Some("file") => attachments[0]
            .name
            .clone()
            .unwrap_or_else(|| t("File").to_string()),
        _ => String::new(),
    }
}

#[function_component(ChatList)]
pub fn chat_list(props: &ChatListProps) -> Html {
    html! {
        <nav class="chat-list" aria-label={t("Chats")}>
            { for props.chats.iter().map(|item| {
                let chat_id = item.chat.id;
                let selected = props.selected == Some(chat_id);
                let on_select = props.on_select.clone();
                let onclick = Callback::from(move |_| on_select.emit(chat_id));
                let peer = item
                    .chat
                    .peer_user_id
                    .and_then(|peer| props.names.get(&peer))
                    .map(String::as_str);
                let (line, when) = match &item.last_message {
                    Some(message) => (
                        preview(message, props.my_user_id, &props.blocked),
                        time::row_time(&message.created_at, props.now_ms),
                    ),
                    None => (t("No messages yet").to_string(), String::new()),
                };
                html! {
                    <button
                        class={classes!(
                            "chat-row",
                            selected.then_some("is-selected"),
                            item.chat.is_family().then_some("is-family"),
                        )}
                        aria-current={selected.then_some("true")}
                        {onclick}
                    >
                        // The family is a house; a direct chat is the peer's
                        // face; the assistant's chat has no peer id, and is
                        // the initials of its name (ios MacChatView).
                        <Avatar
                            title={item.chat.display_title(peer)}
                            family={item.chat.is_family()}
                            user_id={item.chat.peer_user_id.filter(|_| item.chat.is_direct())}
                            version={item.chat.peer_user_id.and_then(|peer| props.avatars.get(&peer).copied()).unwrap_or(0)}
                            size={34}
                        />
                        <span class="chat-text">
                        <span class="chat-head">
                            <span class="chat-title">{ item.chat.display_title(peer) }</span>
                            if !when.is_empty() {
                                <span class="chat-time">{ when }</span>
                            }
                        </span>
                        <span class="chat-foot">
                            <span class="chat-preview">{ line }</span>
                            if item.mentioned {
                                <span class="mention-mark" aria-label={t("You were mentioned")}>{ "@" }</span>
                            }
                            if item.unread_count > 0 {
                                <span class="badge" aria-label={tn("%lld unread", item.unread_count)}>
                                    { item.unread_count }
                                </span>
                            }
                        </span>
                        </span>
                    </button>
                }
            }) }
        </nav>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Attachment, Call};
    use wasm_bindgen_test::*;

    fn message(sender: i64, body: &str) -> Message {
        Message {
            id: 1,
            chat_id: 42,
            sender_id: sender,
            body: body.into(),
            created_at: "2026-09-10T10:00:00Z".into(),
            ..Message::default()
        }
    }

    #[wasm_bindgen_test]
    fn a_blocked_senders_preview_is_the_hidden_row_with_no_words() {
        let blocked = HashSet::from([9]);
        assert_eq!(
            preview(&message(9, "anything"), 7, &blocked),
            "Hidden — blocked member"
        );
        assert_eq!(preview(&message(7, "mine"), 7, &blocked), "mine");
    }

    #[wasm_bindgen_test]
    fn an_empty_body_reads_as_what_it_carries() {
        let blocked = HashSet::new();
        let mut photos = message(9, "");
        photos.attachments = Some(vec![
            Attachment {
                id: 1,
                kind: "photo".into(),
                ..Default::default()
            },
            Attachment {
                id: 2,
                kind: "photo".into(),
                ..Default::default()
            },
        ]);
        assert_eq!(preview(&photos, 7, &blocked), "2 Photos");
        let mut file = message(9, "");
        file.attachments = Some(vec![Attachment {
            id: 3,
            kind: "file".into(),
            name: Some("receipts.pdf".into()),
            ..Default::default()
        }]);
        assert_eq!(preview(&file, 7, &blocked), "receipts.pdf");
        assert_eq!(
            preview(&message(9, "first line\nsecond"), 7, &blocked),
            "first line"
        );
    }

    /// A call record's body is an English placeholder the list never shows.
    #[wasm_bindgen_test]
    fn a_call_record_reads_as_a_call_not_its_placeholder() {
        let mut record = message(9, "Missed voice call");
        record.call = Some(Call {
            outcome: "completed".into(),
            duration_secs: Some(75),
            video: false,
        });
        let line = preview(&record, 7, &HashSet::new());
        assert!(line.contains("1:15"), "{line}");
        assert!(!line.contains("Missed"));
    }
}
