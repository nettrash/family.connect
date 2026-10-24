//! The list of chats down the side: the family chat always and first,
//! groups, direct chats and the assistant's once they exist, newest
//! conversation first, each with its unread count and — while an unread
//! message names the reader — an "@". Above them, "New Group"
//! (docs/protocol.md, "Groups": a button in the list's header on the web,
//! the Mac and Windows).
//!
//! An account in several families sees one at a time: the list is the
//! family on screen's, and its header carries the switcher — every family
//! with its unread count, a join request still waiting, and the ways to
//! another family. An invitation is a banner at the top until it is
//! answered (docs/protocol.md, "Several families"). An account in ONE
//! family with nothing pending sees none of this.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use fc_text::i18n::{t, tn};
use yew::prelude::*;

use crate::actions::Action;
use crate::live::Panel;
use crate::model::{ChatListItem, Invitation, Member, Message, PendingJoin};
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
    /// Every family the account is in, for the switcher.
    #[prop_or_default]
    pub families: Families,
}

/// One family's row in the switcher.
#[derive(Debug, Clone, PartialEq)]
pub struct FamilyRow {
    pub id: i64,
    pub name: String,
    /// The unread of the chats listed in it — a direct chat counts in each
    /// family that lists it.
    pub unread: i64,
}

/// What the switcher and the invitation banner draw.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Families {
    pub rows: Vec<FamilyRow>,
    /// The family on screen.
    pub current: Option<i64>,
    pub invitations: Vec<Invitation>,
    /// Join requests still waiting on an owner.
    pub pending: Vec<PendingJoin>,
    /// "New Family…" is offered (fc_text::families::offers_new_family).
    pub new_family: bool,
    /// "Join Another Family…" is offered.
    pub join_another: bool,
}

impl Families {
    /// Whether the switcher is drawn at all.
    pub fn shown(&self) -> bool {
        fc_text::families::shows_switcher(
            self.rows.len(),
            self.invitations.len(),
            self.pending.len(),
        )
    }
}

#[derive(Properties, PartialEq)]
struct SwitcherProps {
    families: Families,
    on_action: Callback<Action>,
}

/// The family on screen's name, and the button that lists the others: a
/// popover with every family and its unread count, the requests waiting,
/// the invitations, and the ways to another family.
#[function_component(FamilySwitcher)]
fn family_switcher(props: &SwitcherProps) -> Html {
    let open = use_state(|| false);
    let families = &props.families;
    let current = families
        .rows
        .iter()
        .find(|row| Some(row.id) == families.current);
    let toggle = {
        let open = open.clone();
        Callback::from(move |_: MouseEvent| open.set(!*open))
    };
    let close = {
        let open = open.clone();
        Callback::from(move |_: MouseEvent| open.set(false))
    };
    let keys = {
        let open = open.clone();
        Callback::from(move |event: KeyboardEvent| {
            if event.key() == "Escape" {
                event.prevent_default();
                open.set(false);
            }
        })
    };
    // Every choice in it closes it, and then does what it says.
    let choose = |make: Rc<dyn Fn() -> Action>| {
        let open = open.clone();
        let on_action = props.on_action.clone();
        Callback::from(move |_: MouseEvent| {
            open.set(false);
            on_action.emit(make());
        })
    };
    html! {
        <div class="family-switch" onkeydown={keys}>
            <h2 class="family-name">{ current.map(|row| row.name.clone()).unwrap_or_default() }</h2>
            <button
                type="button"
                class="link switch-family"
                aria-label={t("Switch Family")}
                title={t("Switch Family")}
                aria-haspopup="dialog"
                aria-expanded={if *open { "true" } else { "false" }}
                onclick={toggle}
            >{ "⌄" }</button>
            if *open {
                <div class="menu-backdrop" onclick={close} aria-hidden="true"></div>
                <div class="menu family-menu" role="dialog" aria-label={t("Switch Family")}>
                    <ul class="family-rows">
                        { for families.rows.iter().map(|row| {
                            let here = Some(row.id) == families.current;
                            html! {
                                <li key={row.id.to_string()}>
                                    <button
                                        type="button"
                                        class={classes!("family-row", here.then_some("is-selected"))}
                                        aria-current={here.then_some("true")}
                                        aria-label={fc_text::families::row_label(&row.name, row.unread)}
                                        onclick={{ let id = row.id; choose(Rc::new(move || Action::SwitchFamily(id))) }}
                                    >
                                        <span class="family-row-name">{ row.name.clone() }</span>
                                        if row.unread > 0 {
                                            <span class="badge" aria-hidden="true">{ row.unread }</span>
                                        }
                                    </button>
                                </li>
                            }
                        }) }
                        { for families.pending.iter().map(|pending| html! {
                            <li key={format!("pending-{}", pending.family_id)} class="family-pending">
                                <span class="family-row-name">{ pending.family_name.clone() }</span>
                                <span class="muted">{ t("Waiting for approval") }</span>
                            </li>
                        }) }
                        { for families.invitations.iter().map(|invitation| html! {
                            <li key={format!("invitation-{}", invitation.id)} class="family-invitation">
                                <span>{ fc_text::families::invited(&invitation.invited_by.display_name, &invitation.family_name) }</span>
                                <span class="row-actions">
                                    <button type="button" class="link" onclick={{ let id = invitation.id; choose(Rc::new(move || Action::AcceptInvitation { invitation_id: id })) }}>{ t("Join") }</button>
                                    <button type="button" class="link" onclick={{ let id = invitation.id; choose(Rc::new(move || Action::DeclineInvitation { invitation_id: id })) }}>{ t("Decline") }</button>
                                </span>
                            </li>
                        }) }
                    </ul>
                    if families.new_family || families.join_another {
                        <div class="family-doors">
                            if families.new_family {
                                <button type="button" class="link" onclick={choose(Rc::new(|| Action::OpenPanel(Panel::NewFamily)))}>{ t("New Family…") }</button>
                            }
                            if families.join_another {
                                <button type="button" class="link" onclick={choose(Rc::new(|| Action::OpenPanel(Panel::JoinFamily)))}>{ t("Join Another Family…") }</button>
                            }
                        </div>
                    }
                </div>
            }
        </div>
    }
}

/// The invitations, at the top of the list until each is answered.
fn invitation_banner(invitations: &[Invitation], on_action: &Callback<Action>) -> Html {
    html! {
        { for invitations.iter().map(|invitation| {
            let id = invitation.id;
            let join = on_action.reform(move |_: MouseEvent| Action::AcceptInvitation { invitation_id: id });
            let decline = on_action.reform(move |_: MouseEvent| Action::DeclineInvitation { invitation_id: id });
            html! {
                <div class="banner invitation-banner" role="status" key={format!("invitation-{id}")}>
                    <span>{ fc_text::families::invited(&invitation.invited_by.display_name, &invitation.family_name) }</span>
                    <span class="row-actions">
                        <button type="button" onclick={join}>{ t("Join") }</button>
                        <button type="button" class="link" onclick={decline}>{ t("Decline") }</button>
                    </span>
                </div>
            }
        }) }
    }
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
    let switcher = props.families.shown().then(|| {
        html! {
            <FamilySwitcher families={props.families.clone()} on_action={props.on_action.clone()} />
        }
    });
    html! {
        <nav class="chat-list" aria-label={t("Chats")}>
            { switcher.unwrap_or_default() }
            { invitation_banner(&props.families.invitations, &props.on_action) }
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
            families: Families::default(),
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

    /// THE SWITCHER (docs/protocol.md, "Several families"): none at all for
    /// one family with nothing pending; with two, the family on screen's
    /// name and a "Switch Family" button listing every family with its
    /// unread count, the requests waiting, and the ways to another family —
    /// and an invitation is a banner with Join and Decline.
    #[wasm_bindgen_test]
    async fn the_switcher_lists_every_family_and_an_invitation_is_a_banner() {
        use crate::model::{Invitation, PendingJoin, User};
        use std::cell::RefCell;
        use wasm_bindgen::JsCast;
        let log = Rc::new(RefCell::new(Vec::<Action>::new()));
        let props = |families: Families| {
            let sink = log.clone();
            ChatListProps {
                chats: Vec::new(),
                names: HashMap::new(),
                blocked: HashSet::new(),
                avatars: HashMap::new(),
                my_user_id: 7,
                selected: None,
                on_select: Callback::noop(),
                now_ms: 0.0,
                members: None,
                on_action: Callback::from(move |action| sink.borrow_mut().push(action)),
                families,
            }
        };
        let one = Families {
            rows: vec![FamilyRow {
                id: 3,
                name: "Parents".into(),
                unread: 2,
            }],
            current: Some(3),
            join_another: true,
            ..Families::default()
        };
        let document = web_sys::window().unwrap().document().unwrap();
        let root = document.create_element("div").unwrap();
        document.body().unwrap().append_child(&root).unwrap();
        let handle =
            yew::Renderer::<ChatList>::with_root_and_props(root.clone(), props(one.clone()))
                .render();
        gloo_timers::future::TimeoutFuture::new(20).await;
        assert!(
            root.query_selector(".family-switch").unwrap().is_none(),
            "one family and nothing pending: nothing changes"
        );
        handle.destroy();

        let invitation = Invitation {
            id: 4,
            family_id: 9,
            family_name: "Ivanovs".into(),
            invited_by: User {
                id: 9,
                display_name: "Anna".into(),
                ..User::default()
            },
            user: User {
                id: 7,
                ..User::default()
            },
            created_at: None,
        };
        let several = Families {
            rows: vec![
                FamilyRow {
                    id: 3,
                    name: "Parents".into(),
                    unread: 2,
                },
                FamilyRow {
                    id: 4,
                    name: "Ours".into(),
                    unread: 0,
                },
            ],
            current: Some(3),
            invitations: vec![invitation],
            pending: vec![PendingJoin {
                family_id: 5,
                family_name: "Cousins".into(),
                created_at: None,
            }],
            new_family: true,
            join_another: true,
        };
        let handle =
            yew::Renderer::<ChatList>::with_root_and_props(root.clone(), props(several)).render();
        gloo_timers::future::TimeoutFuture::new(20).await;
        let name = root
            .query_selector(".family-name")
            .unwrap()
            .expect("the name");
        assert_eq!(name.text_content().as_deref(), Some("Parents"));
        let banner = root
            .query_selector(".invitation-banner")
            .unwrap()
            .expect("the invitation, at the top of the list");
        assert!(banner
            .text_content()
            .unwrap_or_default()
            .contains("Anna invited you to “Ivanovs”"));
        let button: web_sys::HtmlElement = root
            .query_selector(".switch-family")
            .unwrap()
            .expect("the switcher")
            .dyn_into()
            .unwrap();
        assert_eq!(
            button.get_attribute("aria-label").as_deref(),
            Some("Switch Family")
        );
        button.click();
        gloo_timers::future::TimeoutFuture::new(20).await;
        let rows = root.query_selector_all(".family-row").unwrap();
        let labels: Vec<String> = (0..rows.length())
            .map(|at| {
                rows.item(at)
                    .unwrap()
                    .dyn_into::<web_sys::Element>()
                    .unwrap()
                    .get_attribute("aria-label")
                    .unwrap_or_default()
            })
            .collect();
        assert_eq!(labels, vec!["Parents, 2 unread", "Ours, 0 unread"]);
        let pending = root
            .query_selector(".family-pending")
            .unwrap()
            .expect("waiting");
        assert!(pending
            .text_content()
            .unwrap_or_default()
            .contains("Waiting for approval"));
        let doors = root
            .query_selector(".family-doors")
            .unwrap()
            .expect("doors");
        let words = doors.text_content().unwrap_or_default();
        assert!(words.contains("New Family…") && words.contains("Join Another Family…"));
        rows.item(1)
            .unwrap()
            .dyn_into::<web_sys::HtmlElement>()
            .unwrap()
            .click();
        gloo_timers::future::TimeoutFuture::new(20).await;
        assert!(matches!(log.borrow()[..], [Action::SwitchFamily(4)]));
        assert!(
            root.query_selector(".family-menu").unwrap().is_none(),
            "a choice closes it"
        );
        banner
            .query_selector("button")
            .unwrap()
            .unwrap()
            .dyn_into::<web_sys::HtmlElement>()
            .unwrap()
            .click();
        gloo_timers::future::TimeoutFuture::new(20).await;
        assert!(matches!(
            log.borrow()[..],
            [_, Action::AcceptInvitation { invitation_id: 4 }]
        ));
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
