//! The list of chats down the side: the family chat always and first,
//! groups, direct chats and the assistant's once they exist, newest
//! conversation first, each with its unread count and — while an unread
//! message names the reader — an "@". Above them, "New Group"
//! (docs/protocol.md, "Groups": a button in the list's header on the web,
//! the Mac and Windows).

use std::collections::{HashMap, HashSet};

use fc_text::i18n::{t, tn};
use yew::prelude::*;

use crate::actions::Action;
use crate::model::{ChatListItem, Member, Message};
use crate::time;
use crate::views::avatar::Avatar;
use crate::views::group::NewGroupDialog;

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
    /// The family's roster, whom a new group may hold — None where there is
    /// no family to make a group in, and then no "New Group" at all.
    #[prop_or_default]
    pub members: Option<Vec<Member>>,
    #[prop_or_default]
    pub on_action: Callback<Action>,
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
        // A sticker says so, where a photo's row says so of a photo
        // (docs/protocol.md, "How it is drawn") — the `last_message`
        // preview carries the flag for exactly this.
        Some("photo") if message.sticker().is_some() => t("Sticker").to_string(),
        Some("photo") if attachments.len() > 1 => tn("%lld Photos", attachments.len() as i64),
        Some("photo") => t("Photo").to_string(),
        // A video message says so — checked before "Video", from the flag
        // `last_message` carries for exactly this (the plan for #79, S5.7).
        Some("video") if message.round_video().is_some() => t("Video message").to_string(),
        // An animation says "GIF" where a video's row says "Video"
        // (docs/protocol.md, "Animations"), from the flag `last_message`
        // carries; with a caption, the caption, as any message's.
        Some("video") if message.animation().is_some() => t("GIF").to_string(),
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
    let making = use_state(|| false);
    let new_group = props.members.clone().map(|members| {
        let open = {
            let making = making.clone();
            Callback::from(move |_: MouseEvent| making.set(true))
        };
        let close = {
            let making = making.clone();
            Callback::from(move |_: ()| making.set(false))
        };
        html! {
            <>
                <div class="chat-list-head">
                    <button type="button" class="link new-group" aria-haspopup="dialog" onclick={open}>
                        <span aria-hidden="true">{ "+ " }</span>{ t("New Group") }
                    </button>
                </div>
                if *making {
                    <NewGroupDialog
                        {members}
                        my_user_id={props.my_user_id}
                        on_action={props.on_action.clone()}
                        on_close={close}
                    />
                }
            </>
        }
    });
    html! {
        <nav class="chat-list" aria-label={t("Chats")}>
            { new_group.unwrap_or_default() }
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
                        // The family is a house; a group is its people; a
                        // direct chat is the peer's face; the assistant's
                        // chat has no peer id, and is the initials of its
                        // name (ios MacChatView).
                        <Avatar
                            title={item.chat.display_title(peer)}
                            family={item.chat.is_family()}
                            group={item.chat.is_group()}
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

    /// A sticker's row says "Sticker"; the same photo without the flag
    /// says "Photo"; and a blocked member's is the hidden row like any
    /// other message of theirs.
    #[wasm_bindgen_test]
    fn a_stickers_row_says_sticker() {
        let picture = |sticker: bool| Attachment {
            id: 90,
            kind: "photo".into(),
            mime: Some("image/webp".into()),
            sticker,
            ..Default::default()
        };
        let mut sent = message(9, "");
        sent.attachments = Some(vec![picture(true)]);
        assert_eq!(preview(&sent, 7, &HashSet::new()), "Sticker");
        assert_eq!(
            preview(&sent, 7, &HashSet::from([9])),
            "Hidden — blocked member"
        );
        let mut photo = message(9, "");
        photo.attachments = Some(vec![picture(false)]);
        assert_eq!(preview(&photo, 7, &HashSet::new()), "Photo");
    }

    /// A video message's row says "Video message", checked before "Video";
    /// the same video without the flag says "Video"; a voice note keeps the
    /// web's word; a blocked member's is the hidden row (S5.7).
    #[wasm_bindgen_test]
    fn a_video_messages_row_says_video_message() {
        let video = |round: bool| Attachment {
            id: 91,
            kind: "video".into(),
            mime: Some("video/mp4".into()),
            width: Some(480),
            height: Some(480),
            round,
            ..Default::default()
        };
        let mut sent = message(9, "");
        sent.attachments = Some(vec![video(true)]);
        assert_eq!(preview(&sent, 7, &HashSet::new()), "Video message");
        assert_eq!(
            preview(&sent, 7, &HashSet::from([9])),
            "Hidden — blocked member"
        );
        let mut plain = message(9, "");
        plain.attachments = Some(vec![video(false)]);
        assert_eq!(preview(&plain, 7, &HashSet::new()), "Video");
        let mut pair = message(9, "");
        pair.attachments = Some(vec![video(true), video(true)]);
        assert_eq!(preview(&pair, 7, &HashSet::new()), "Video", "never two");
        let mut voice = message(9, "");
        voice.attachments = Some(vec![Attachment {
            id: 92,
            kind: "audio".into(),
            ..Default::default()
        }]);
        assert_eq!(preview(&voice, 7, &HashSet::new()), "Voice message");
    }

    /// An animation's row says "GIF" where a video's says "Video"
    /// (docs/protocol.md, "Animations"); with a caption, the caption; two
    /// flagged videos are not one.
    #[wasm_bindgen_test]
    fn an_animations_row_says_gif() {
        let video = |animation: bool| Attachment {
            id: 93,
            kind: "video".into(),
            mime: Some("video/mp4".into()),
            animation,
            ..Default::default()
        };
        let mut sent = message(9, "");
        sent.attachments = Some(vec![video(true)]);
        assert_eq!(preview(&sent, 7, &HashSet::new()), "GIF");
        let mut captioned = message(9, "look at him go");
        captioned.attachments = Some(vec![video(true)]);
        assert_eq!(preview(&captioned, 7, &HashSet::new()), "look at him go");
        let mut plain = message(9, "");
        plain.attachments = Some(vec![video(false)]);
        assert_eq!(preview(&plain, 7, &HashSet::new()), "Video");
        let mut pair = message(9, "");
        pair.attachments = Some(vec![video(true), video(true)]);
        assert_eq!(preview(&pair, 7, &HashSet::new()), "Video");
    }

    fn row(chat: crate::model::Chat) -> ChatListItem {
        ChatListItem {
            chat,
            ..ChatListItem::default()
        }
    }

    /// A group is listed under its name below the family chat, its circle
    /// its people and named "Group"; "New Group" heads the list wherever
    /// there is a family to make one in, and opens the dialog
    /// (docs/protocol.md, "Groups").
    #[wasm_bindgen_test]
    async fn a_group_is_listed_and_new_group_heads_the_list() {
        use crate::model::Chat;
        use wasm_bindgen::JsCast;
        let chats = vec![
            row(Chat {
                id: 42,
                kind: "family".into(),
                title: Some("The Smiths".into()),
                ..Chat::default()
            }),
            row(Chat {
                id: 77,
                kind: "group".into(),
                title: Some("Cousins".into()),
                member_ids: Some(vec![7, 9]),
                admin_user_id: Some(9),
                ..Chat::default()
            }),
        ];
        let props = |members: Option<Vec<crate::model::Member>>| ChatListProps {
            chats: chats.clone(),
            names: HashMap::new(),
            blocked: HashSet::new(),
            avatars: HashMap::new(),
            my_user_id: 7,
            selected: None,
            on_select: Callback::noop(),
            now_ms: 0.0,
            members,
            on_action: Callback::noop(),
        };
        let document = web_sys::window().unwrap().document().unwrap();
        let root = document.create_element("div").unwrap();
        document.body().unwrap().append_child(&root).unwrap();
        let handle =
            yew::Renderer::<ChatList>::with_root_and_props(root.clone(), props(Some(Vec::new())))
                .render();
        gloo_timers::future::TimeoutFuture::new(20).await;
        let titles = root.query_selector_all(".chat-title").unwrap();
        let titles: Vec<String> = (0..titles.length())
            .map(|at| titles.item(at).unwrap().text_content().unwrap_or_default())
            .collect();
        assert_eq!(titles, vec!["The Smiths", "Cousins"]);
        let circle = root
            .query_selector(".chat-row:nth-of-type(2) .avatar")
            .unwrap()
            .expect("the group's circle");
        assert_eq!(circle.get_attribute("aria-label").as_deref(), Some("Group"));
        assert_eq!(circle.get_attribute("role").as_deref(), Some("img"));
        assert!(
            circle.query_selector("svg").unwrap().is_some(),
            "its people"
        );
        let family = root
            .query_selector(".chat-row.is-family .avatar")
            .unwrap()
            .unwrap();
        assert_eq!(family.get_attribute("aria-hidden").as_deref(), Some("true"));

        let new_group: web_sys::HtmlElement = root
            .query_selector(".chat-list-head .new-group")
            .unwrap()
            .expect("New Group")
            .dyn_into()
            .unwrap();
        assert!(new_group
            .text_content()
            .unwrap_or_default()
            .contains("New Group"));
        new_group.click();
        gloo_timers::future::TimeoutFuture::new(20).await;
        let title = root
            .query_selector(".dialog h2")
            .unwrap()
            .expect("the dialog");
        assert_eq!(title.text_content().as_deref(), Some("New Group"));
        handle.destroy();

        // No family, no group to make.
        let handle =
            yew::Renderer::<ChatList>::with_root_and_props(root.clone(), props(None)).render();
        gloo_timers::future::TimeoutFuture::new(20).await;
        assert!(root.query_selector(".new-group").unwrap().is_none());
        handle.destroy();
        root.remove();
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
