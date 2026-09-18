//! The layout, tested in a real browser against the real stylesheet.
//!
//! The rule under test is the one a chat cannot break: the bar, the chat
//! list and the composer stay where they are, and ONLY the two panes scroll,
//! each on its own. It is a property of CSS, so it can only be checked where
//! CSS is actually laid out — which is why these run under
//! `wasm-pack test --headless --chrome` and not in a unit test.
//!
//! The stylesheet is the shipped one (`include_str!`), not a copy, so a
//! change to `styles.css` that lets the page scroll again fails here.

use std::cell::RefCell;
use std::rc::Rc;

use gloo_timers::future::TimeoutFuture;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use web_sys::{Document, Element, HtmlElement, HtmlInputElement, HtmlTextAreaElement};
use yew::Callback;

use crate::actions::Action;
use crate::live::Opening;
use crate::model::{Chat, ChatListItem, Member, Mention, Message};
use crate::views::conversation::{Conversation, ConversationProps};

fn document() -> Document {
    web_sys::window()
        .expect("a window")
        .document()
        .expect("a document")
}

/// The shipped stylesheet, once per page.
fn install_stylesheet() {
    let document = document();
    if document.get_element_by_id("fc-test-styles").is_some() {
        return;
    }
    let style = document.create_element("style").expect("a style element");
    style.set_id("fc-test-styles");
    style.set_text_content(Some(include_str!("../styles.css")));
    document
        .head()
        .expect("a head")
        .append_child(&style)
        .expect("installing the stylesheet");
}

/// A root pinned over the whole viewport, so whatever the test runner has
/// on its own page cannot offset or shrink what is measured.
fn fixed_root(style: &str) -> HtmlElement {
    let document = document();
    let root: HtmlElement = document
        .create_element("div")
        .expect("a div")
        .dyn_into_html();
    root.set_attribute("style", style)
        .expect("styling the root");
    document
        .body()
        .expect("a body")
        .append_child(&root)
        .expect("mounting the root");
    root
}

trait IntoHtml {
    fn dyn_into_html(self) -> HtmlElement;
}

impl IntoHtml for Element {
    fn dyn_into_html(self) -> HtmlElement {
        use wasm_bindgen::JsCast;
        self.dyn_into::<HtmlElement>().expect("an HTML element")
    }
}

fn query(root: &Element, selector: &str) -> Element {
    root.query_selector(selector)
        .expect("a valid selector")
        .unwrap_or_else(|| panic!("{selector} is on the page"))
}

fn viewport_height() -> f64 {
    web_sys::window()
        .expect("a window")
        .inner_height()
        .expect("a height")
        .as_f64()
        .expect("a number")
}

/// The signed-in shell's markup, with far more chats and messages than fit.
/// The CLASS NAMES are what the stylesheet keys on, and they are the ones
/// main.rs and the views render.
fn shell(chats: usize, messages: usize) -> String {
    let rows: String = (0..chats)
        .map(|i| {
            format!(
                r#"<button class="chat-row"><span class="chat-title">Chat {i}</span>
                   <span class="chat-preview">The last thing somebody said {i}</span></button>"#
            )
        })
        .collect();
    let bubbles: String = (0..messages)
        .map(|i| {
            let mine = if i % 3 == 0 { " is-mine" } else { "" };
            format!(
                r#"<div class="row"><article class="bubble{mine}"><span class="sender">Anna</span>
                   <p class="body">Message number {i}, long enough to take a line.</p>
                   <span class="meta">17:0{d}</span></article></div>"#,
                d = i % 10
            )
        })
        .collect();
    format!(
        r#"<div class="app">
             <header class="bar"><span class="brand">Family Connect</span>
               <button class="signout">Sign out</button></header>
             <div class="split">
               <nav class="chat-list">{rows}</nav>
               <section class="conversation">
                 <header class="conversation-bar"><h2 class="conversation-title">Chat</h2></header>
                 <div class="messages">{bubbles}</div>
                 <div class="composer-wrap">
                   <div class="composer"><textarea rows="2"></textarea><button>Send</button></div>
                 </div>
               </section>
             </div>
           </div>"#
    )
}

/// THE BUG AS REPORTED: history scrolled with the whole page. The shell must
/// be exactly the window, nothing may escape it, and the list and the
/// messages must each be a scroller of their own.
#[wasm_bindgen_test]
async fn the_page_never_scrolls_only_the_two_panes_do() {
    install_stylesheet();
    let root = fixed_root("position:fixed;inset:0;");
    root.set_inner_html(&shell(60, 200));
    TimeoutFuture::new(20).await;

    let app = query(&root, ".app");
    let messages = query(&root, ".messages");
    let list = query(&root, ".chat-list");
    let bar = query(&root, ".bar");
    let composer = query(&root, ".composer");
    let viewport = viewport_height();

    assert!(
        (f64::from(app.client_height()) - viewport).abs() <= 1.0,
        "the shell is exactly the window: {} vs {viewport}",
        app.client_height()
    );
    assert!(
        app.scroll_height() <= app.client_height() + 1,
        "nothing escapes the shell to scroll the page: {} > {}",
        app.scroll_height(),
        app.client_height()
    );
    assert!(
        messages.scroll_height() > messages.client_height(),
        "the messages pane is a scroller of its own"
    );
    assert!(
        list.scroll_height() > list.client_height(),
        "and so is the chat list"
    );
    assert!(
        bar.get_bounding_client_rect().top() >= -0.5,
        "the bar is on screen"
    );
    assert!(
        composer.get_bounding_client_rect().bottom() <= viewport + 0.5,
        "the composer is on screen: {} > {viewport}",
        composer.get_bounding_client_rect().bottom()
    );

    // Scrolling either pane moves that pane and nothing else.
    let bar_top = bar.get_bounding_client_rect().top();
    let composer_bottom = composer.get_bounding_client_rect().bottom();
    messages.set_scroll_top(messages.scroll_height());
    list.set_scroll_top(list.scroll_height());
    TimeoutFuture::new(20).await;
    assert!(
        messages.scroll_top() > 0,
        "the messages pane really scrolled"
    );
    assert!(list.scroll_top() > 0, "the list really scrolled");
    assert_eq!(
        bar.get_bounding_client_rect().top(),
        bar_top,
        "the bar did not move"
    );
    assert_eq!(
        composer.get_bounding_client_rect().bottom(),
        composer_bottom,
        "the composer did not move"
    );

    root.remove();
}

fn message(id: i64) -> Message {
    Message {
        id,
        chat_id: 42,
        sender_id: if id % 2 == 0 { 7 } else { 9 },
        body: format!("Message number {id}, long enough to take a line of its own."),
        created_at: "2026-08-19T17:03:12Z".into(),
        ..Message::default()
    }
}

fn props(count: i64) -> ConversationProps {
    props_with(
        (1..=count).map(message).collect(),
        Some(Opening {
            chat_id: 42,
            unread_count: 0,
            last_read_message_id: 0,
        }),
        Callback::noop(),
    )
}

/// What the view asked of the app, in order.
fn recorder() -> (Rc<RefCell<Vec<Action>>>, Callback<Action>) {
    let log = Rc::new(RefCell::new(Vec::new()));
    let sink = log.clone();
    (
        log,
        Callback::from(move |action: Action| sink.borrow_mut().push(action)),
    )
}

fn at_newest_said(log: &Rc<RefCell<Vec<Action>>>) -> Vec<bool> {
    log.borrow()
        .iter()
        .filter_map(|action| match action {
            Action::AtNewest(at) => Some(*at),
            _ => None,
        })
        .collect()
}

fn scroll_to(messages: &Element, top: i32) {
    messages.set_scroll_top(top);
    let event = web_sys::Event::new("scroll").expect("a scroll event");
    messages
        .dispatch_event(&event)
        .expect("dispatching the scroll");
}

fn pane() -> HtmlElement {
    install_stylesheet();
    fixed_root(
        "position:fixed;top:0;left:0;width:600px;height:320px;\
         display:grid;grid-template-rows:minmax(0,1fr);",
    )
}

/// Type into a field the way a person does: the value, then the event the
/// view listens for.
fn type_into(field: &Element, value: &str) {
    if let Some(input) = field.dyn_ref::<HtmlInputElement>() {
        input.set_value(value);
    } else if let Some(area) = field.dyn_ref::<HtmlTextAreaElement>() {
        area.set_value(value);
    }
    let init = web_sys::EventInit::new();
    init.set_bubbles(true);
    let event = web_sys::Event::new_with_event_init_dict("input", &init).expect("an input event");
    field.dispatch_event(&event).expect("dispatching the input");
}

fn click_labelled(root: &Element, selector: &str, label: &str) {
    let found = root.query_selector_all(selector).expect("a valid selector");
    for index in 0..found.length() {
        let element = found.item(index).expect("an element");
        if element.text_content().unwrap_or_default().trim() == label {
            element
                .dyn_into::<HtmlElement>()
                .expect("an HTML element")
                .click();
            return;
        }
    }
    panic!("no {selector} reading {label:?}");
}

fn props_with(
    messages: Vec<Message>,
    opening: Option<Opening>,
    on_action: Callback<Action>,
) -> ConversationProps {
    ConversationProps {
        calls_enabled: false,
        video_calls_enabled: false,
        on_call: false,
        item: ChatListItem {
            chat: Chat {
                id: 42,
                kind: "family".into(),
                title: Some("The Smiths".into()),
                peer_user_id: None,
            },
            last_message: None,
            unread_count: 0,
            last_read_message_id: 0,
            max_reaction_seq: None,
            max_edit_seq: None,
            max_poll_seq: None,
            mentioned: false,
        },
        messages,
        my_user_id: 7,
        names: Default::default(),
        members: Vec::new(),
        assistant: None,
        blocked: Default::default(),
        revealed: Default::default(),
        revealed_quotes: Default::default(),
        opening,
        staged: Vec::new(),
        family: None,
        failed: Default::default(),
        ai_failed: Default::default(),
        peer_read: 0,
        typing: Vec::new(),
        can_load_more: false,
        unanswered_polls: 0,
        draft: String::new(),
        support_contact: None,
        on_action,
        now_ms: 0.0,
    }
}

fn is_at_newest(messages: &Element) -> bool {
    messages.scroll_top() + messages.client_height() >= messages.scroll_height() - 2
}

/// Opening a chat lands on its newest message, and a message that arrives
/// while the reader is following along is scrolled INTO view — after it has
/// rendered, which is exactly what the old call-before-render missed.
#[wasm_bindgen_test]
async fn a_new_message_is_scrolled_into_view_for_a_reader_at_the_bottom() {
    install_stylesheet();
    // A grid cell like `.split`'s, so the pane gets a definite height.
    let root = fixed_root(
        "position:fixed;top:0;left:0;width:600px;height:320px;\
         display:grid;grid-template-rows:minmax(0,1fr);",
    );
    let mut handle =
        yew::Renderer::<Conversation>::with_root_and_props(root.clone().into(), props(80)).render();
    TimeoutFuture::new(50).await;
    let messages = query(&root, ".messages");
    assert!(
        messages.scroll_height() > messages.client_height(),
        "the fixture overflows"
    );
    assert!(
        is_at_newest(&messages),
        "a chat opens on its newest message"
    );

    handle.update(props(81));
    TimeoutFuture::new(50).await;
    assert!(
        is_at_newest(&messages),
        "the new message is in view, not one below the fold"
    );

    handle.destroy();
    root.remove();
}

/// Somebody reading HISTORY is not yanked back down by a message landing —
/// the rule every client in this product follows.
#[wasm_bindgen_test]
async fn a_reader_up_the_thread_stays_where_they_are() {
    install_stylesheet();
    let root = fixed_root(
        "position:fixed;top:0;left:0;width:600px;height:320px;\
         display:grid;grid-template-rows:minmax(0,1fr);",
    );
    let mut handle =
        yew::Renderer::<Conversation>::with_root_and_props(root.clone().into(), props(80)).render();
    TimeoutFuture::new(50).await;
    let messages = query(&root, ".messages");

    // Scroll to the top, and let the scroll event reach the component.
    messages.set_scroll_top(0);
    let event = web_sys::Event::new("scroll").expect("a scroll event");
    messages
        .dispatch_event(&event)
        .expect("dispatching the scroll");
    TimeoutFuture::new(20).await;

    handle.update(props(81));
    TimeoutFuture::new(50).await;
    assert_eq!(
        messages.scroll_top(),
        0,
        "a reader up the thread is left where they were"
    );

    handle.destroy();
    root.remove();
}

/// A chat opens where its unread state says — decided only once its
/// messages are in, from the state as it stood BEFORE anything read it —
/// and the view says whether its reader is at the newest only THEN. With
/// the divider two screens up, that is "no": the chat stays unread until
/// the reader gets to the bottom.
#[wasm_bindgen_test]
async fn a_chat_opens_at_its_divider_and_is_read_only_at_the_newest() {
    let root = pane();
    let (log, on_action) = recorder();
    let loading = props_with((1..=80).map(message).collect(), None, on_action.clone());
    let mut handle =
        yew::Renderer::<Conversation>::with_root_and_props(root.clone().into(), loading).render();
    TimeoutFuture::new(50).await;
    let messages = query(&root, ".messages");
    // Somebody scrolling about while it loads — up, and back to the bottom —
    // is not somebody who has arrived anywhere yet.
    scroll_to(&messages, 0);
    TimeoutFuture::new(20).await;
    scroll_to(&messages, messages.scroll_height());
    TimeoutFuture::new(20).await;
    assert!(
        at_newest_said(&log).is_empty(),
        "nothing is said while the chat is still loading"
    );
    assert!(root.query_selector("#unread-divider").unwrap().is_none());

    // Loaded: five unread from Anna (the odd ids) above the marker at 70.
    handle.update(props_with(
        (1..=80).map(message).collect(),
        Some(Opening {
            chat_id: 42,
            unread_count: 5,
            last_read_message_id: 70,
        }),
        on_action.clone(),
    ));
    TimeoutFuture::new(50).await;
    let divider = query(&root, "#unread-divider");
    assert_eq!(
        divider.text_content().unwrap_or_default().trim(),
        "5 new messages"
    );
    assert_eq!(
        divider
            .next_element_sibling()
            .and_then(|row| row.query_selector("article").ok().flatten())
            .map(|bubble| bubble.id()),
        Some("m-71".to_string()),
        "the divider sits over the first unread message"
    );
    assert_eq!(at_newest_said(&log), vec![false], "opened at the divider");

    scroll_to(&messages, messages.scroll_height());
    TimeoutFuture::new(20).await;
    assert_eq!(
        at_newest_said(&log),
        vec![false, true],
        "reaching the newest is reading it"
    );

    handle.destroy();
    root.remove();
}

/// Up in the history is not reading; back at the newest is.
#[wasm_bindgen_test]
async fn the_view_says_when_its_reader_leaves_and_returns_to_the_newest() {
    let root = pane();
    let (log, on_action) = recorder();
    let opened = props_with(
        (1..=80).map(message).collect(),
        Some(Opening {
            chat_id: 42,
            unread_count: 0,
            last_read_message_id: 80,
        }),
        on_action,
    );
    let handle =
        yew::Renderer::<Conversation>::with_root_and_props(root.clone().into(), opened).render();
    TimeoutFuture::new(50).await;
    assert_eq!(at_newest_said(&log), vec![true], "opened at the newest");

    let messages = query(&root, ".messages");
    scroll_to(&messages, 0);
    TimeoutFuture::new(20).await;
    scroll_to(&messages, messages.scroll_height());
    TimeoutFuture::new(20).await;
    assert_eq!(at_newest_said(&log), vec![true, false, true]);

    handle.destroy();
    root.remove();
}

/// "Earlier messages" puts fifty rows ABOVE the reader, and what they were
/// reading stays exactly where it was on the screen.
#[wasm_bindgen_test]
async fn earlier_messages_arrive_above_without_moving_the_reader() {
    let root = pane();
    let range = |from: i64, to: i64| (from..=to).map(message).collect::<Vec<_>>();
    let mut handle = yew::Renderer::<Conversation>::with_root_and_props(
        root.clone().into(),
        props_with(
            range(51, 130),
            Some(Opening {
                chat_id: 42,
                unread_count: 0,
                last_read_message_id: 130,
            }),
            Callback::noop(),
        ),
    )
    .render();
    TimeoutFuture::new(50).await;
    let messages = query(&root, ".messages");
    scroll_to(&messages, messages.scroll_height() / 2);
    TimeoutFuture::new(20).await;

    let list_top = messages.get_bounding_client_rect().top();
    let bubbles = messages.query_selector_all("article").unwrap();
    let reading = (0..bubbles.length())
        .map(|index| bubbles.item(index).unwrap().dyn_into::<Element>().unwrap())
        .find(|bubble| bubble.get_bounding_client_rect().top() >= list_top)
        .expect("a bubble in view");
    let id = reading.id();
    // Its menu open, too: what a bubble holds belongs to its message, not
    // to wherever that message happened to be drawn.
    query(&reading, ".more").dyn_into_html().click();
    TimeoutFuture::new(20).await;
    let was = reading.get_bounding_client_rect().top();

    handle.update(props_with(
        range(1, 130),
        Some(Opening {
            chat_id: 42,
            unread_count: 0,
            last_read_message_id: 130,
        }),
        Callback::noop(),
    ));
    TimeoutFuture::new(50).await;
    let now = document()
        .get_element_by_id(&id)
        .expect("still on the page");
    assert!(
        (now.get_bounding_client_rect().top() - was).abs() < 2.0,
        "{id} moved from {was} to {} when earlier messages arrived",
        now.get_bounding_client_rect().top()
    );
    let menus = messages.query_selector_all(".menu").unwrap();
    assert_eq!(menus.length(), 1);
    assert!(
        now.contains(menus.item(0).as_ref()),
        "the open menu is still on {id}"
    );

    handle.destroy();
    root.remove();
}

fn my_message(id: i64, body: &str) -> Message {
    Message {
        id,
        chat_id: 42,
        sender_id: 7,
        client_msg_id: Some(format!("c{id}")),
        body: body.into(),
        created_at: "2026-08-19T17:03:12Z".into(),
        ..Message::default()
    }
}

/// Edit mode ends when the server has the edit — not when Save is pressed
/// — so an edit that is refused or lost keeps its words in the box.
#[wasm_bindgen_test]
async fn a_failed_edit_keeps_its_words() {
    let root = pane();
    let (log, on_action) = recorder();
    let handle = yew::Renderer::<Conversation>::with_root_and_props(
        root.clone().into(),
        props_with(
            vec![my_message(100, "Dinner at 7?")],
            Some(Opening {
                chat_id: 42,
                unread_count: 0,
                last_read_message_id: 100,
            }),
            on_action,
        ),
    )
    .render();
    TimeoutFuture::new(50).await;
    query(&root, ".more").dyn_into_html().click();
    TimeoutFuture::new(20).await;
    click_labelled(&root, ".menu [role=menuitem]", "Edit");
    TimeoutFuture::new(20).await;
    let area = query(&root, "textarea");
    type_into(&area, "Dinner at 8?");
    TimeoutFuture::new(20).await;
    click_labelled(&root, ".composer button", "Save");
    TimeoutFuture::new(20).await;

    let done = log
        .borrow()
        .iter()
        .find_map(|action| match action {
            Action::SaveEdit {
                message_id: 100,
                body,
                done,
                ..
            } if body == "Dinner at 8?" => Some(done.clone()),
            _ => None,
        })
        .expect("the edit was asked for");
    let area_text = || {
        query(&root, "textarea")
            .dyn_into::<HtmlTextAreaElement>()
            .unwrap()
            .value()
    };
    assert!(root
        .text_content()
        .unwrap_or_default()
        .contains("Editing message"));
    assert_eq!(area_text(), "Dinner at 8?", "the words wait for the answer");

    done.emit(());
    TimeoutFuture::new(20).await;
    assert!(!root
        .text_content()
        .unwrap_or_default()
        .contains("Editing message"));
    assert_eq!(area_text(), "", "the draft set aside is back");

    handle.destroy();
    root.remove();
}

/// A poll's question names members the way a message does.
#[wasm_bindgen_test]
async fn a_poll_question_carries_its_mentions() {
    let root = pane();
    let (log, on_action) = recorder();
    let mut asked = props_with(
        vec![my_message(100, "hi")],
        Some(Opening {
            chat_id: 42,
            unread_count: 0,
            last_read_message_id: 100,
        }),
        on_action,
    );
    asked.members = vec![Member {
        id: 9,
        display_name: "Anna".into(),
        username: "anna".into(),
        role: Some("member".into()),
        deleted: false,
        ..Default::default()
    }];
    let handle =
        yew::Renderer::<Conversation>::with_root_and_props(root.clone().into(), asked).render();
    TimeoutFuture::new(50).await;
    // Poll is in the paperclip's menu, as it is on the Mac.
    query(&root, "[aria-label='Attach']")
        .dyn_into_html()
        .click();
    TimeoutFuture::new(20).await;
    click_labelled(&root, ".attach-menu [role=menuitem]", "Poll");
    TimeoutFuture::new(20).await;
    type_into(
        &query(&root, ".dialog .field input"),
        "@Anna pizza or pasta?",
    );
    TimeoutFuture::new(20).await;
    type_into(&query(&root, "[aria-label='Option 1']"), "Pizza");
    TimeoutFuture::new(20).await;
    type_into(&query(&root, "[aria-label='Option 2']"), "Pasta");
    TimeoutFuture::new(20).await;
    query(&root, ".dialog .primary").dyn_into_html().click();
    TimeoutFuture::new(20).await;

    let sent = log
        .borrow()
        .iter()
        .find_map(|action| match action {
            Action::Send { draft, .. } => Some(draft.clone()),
            _ => None,
        })
        .expect("the poll was sent");
    assert_eq!(sent.body, "@Anna pizza or pasta?");
    assert_eq!(
        sent.poll,
        Some(vec!["Pizza".to_string(), "Pasta".to_string()])
    );
    assert_eq!(
        sent.mentions,
        vec![Mention {
            user_id: 9,
            name: "Anna".into()
        }]
    );

    handle.destroy();
    root.remove();
}

/// What a bubble holds — an open menu, here — belongs to its message. A
/// message landing below must not hand it to the message next door, which is
/// what rows matched up by position do.
#[wasm_bindgen_test]
async fn a_new_message_leaves_an_open_menu_where_it_was() {
    let root = pane();
    let opened = Some(Opening {
        chat_id: 42,
        unread_count: 0,
        last_read_message_id: 20,
    });
    let mut handle = yew::Renderer::<Conversation>::with_root_and_props(
        root.clone().into(),
        props_with((1..=20).map(message).collect(), opened, Callback::noop()),
    )
    .render();
    TimeoutFuture::new(50).await;
    let bubble = document().get_element_by_id("m-18").expect("message 18");
    query(&bubble, ".more").dyn_into_html().click();
    TimeoutFuture::new(20).await;
    assert!(bubble.query_selector(".menu").unwrap().is_some());

    handle.update(props_with(
        (1..=21).map(message).collect(),
        opened,
        Callback::noop(),
    ));
    TimeoutFuture::new(50).await;
    let menus = root.query_selector_all(".menu").unwrap();
    assert_eq!(menus.length(), 1);
    let holder = document().get_element_by_id("m-18").expect("message 18");
    assert!(
        holder.contains(menus.item(0).as_ref()),
        "the menu stayed on message 18"
    );

    handle.destroy();
    root.remove();
}

/// "Show the Assistant a Photo…" is a door that exists only where it leads
/// somewhere: the assistant's own chat, on a server that can see, in a
/// family that allows it — and is ABSENT otherwise, never offered inert.
#[wasm_bindgen_test]
async fn the_assistant_photo_door_is_there_only_when_all_three_allow_it() {
    use crate::model::{Assistant, Family};
    let menu_items = |vision: bool, ai_vision: bool, kind: &str| {
        let kind = kind.to_string();
        async move {
            let root = pane();
            let mut asked = props_with(
                vec![my_message(100, "hi")],
                Some(Opening {
                    chat_id: 42,
                    unread_count: 0,
                    last_read_message_id: 100,
                }),
                Callback::noop(),
            );
            asked.item.chat.kind = kind;
            asked.assistant = Some(Assistant {
                user_id: 2,
                display_name: "Assistant".into(),
                mention: Some("@ai".into()),
                draw: Some("/draw".into()),
                vision,
                images: false,
            });
            asked.family = Some(Family {
                id: 3,
                name: "The Smiths".into(),
                ai_history: true,
                ai_vision,
                ai_history_photos: false,
                ..Default::default()
            });
            let handle =
                yew::Renderer::<Conversation>::with_root_and_props(root.clone().into(), asked)
                    .render();
            TimeoutFuture::new(50).await;
            query(&root, "[aria-label='Attach']")
                .dyn_into_html()
                .click();
            TimeoutFuture::new(20).await;
            let found = root
                .query_selector_all(".attach-menu [role=menuitem]")
                .unwrap();
            let labels: Vec<String> = (0..found.length())
                .map(|index| {
                    found
                        .item(index)
                        .unwrap()
                        .text_content()
                        .unwrap_or_default()
                })
                .collect();
            handle.destroy();
            root.remove();
            labels
        }
    };
    let door = "Show the Assistant a Photo…".to_string();
    assert!(menu_items(true, true, "ai").await.contains(&door));
    assert!(
        !menu_items(false, true, "ai").await.contains(&door),
        "a server that cannot see"
    );
    assert!(
        !menu_items(true, false, "ai").await.contains(&door),
        "a family that has not allowed it"
    );
    let family = menu_items(true, true, "family").await;
    assert!(!family.contains(&door), "only in the assistant's own chat");
    assert!(
        family.contains(&"Poll".to_string()),
        "and Poll only in the family chat"
    );
    assert!(!menu_items(true, true, "ai")
        .await
        .contains(&"Poll".to_string()));
}

/// Send takes what is staged with it, the box's words as its caption.
#[wasm_bindgen_test]
async fn a_send_carries_what_is_staged() {
    let root = pane();
    let (log, on_action) = recorder();
    let mut asked = props_with(
        vec![my_message(100, "hi")],
        Some(Opening {
            chat_id: 42,
            unread_count: 0,
            last_read_message_id: 100,
        }),
        on_action,
    );
    asked.staged = vec![crate::staged::Prepared::location(1.0, 2.0, None)];
    let handle =
        yew::Renderer::<Conversation>::with_root_and_props(root.clone().into(), asked).render();
    TimeoutFuture::new(50).await;
    type_into(&query(&root, "textarea"), "Look");
    TimeoutFuture::new(20).await;
    click_labelled(&root, ".composer button", "Send");
    TimeoutFuture::new(20).await;
    let sent = log
        .borrow()
        .iter()
        .find_map(|action| match action {
            Action::Send { draft, .. } => Some(draft.clone()),
            _ => None,
        })
        .expect("sent");
    assert_eq!(sent.body, "Look");
    assert_eq!(sent.attachments.len(), 1);
    handle.destroy();
    root.remove();
}

/// A thread's box says it too: every send there replies to the root, so an
/// `@ai` there is pointed at the root's photos.
#[wasm_bindgen_test]
async fn a_threads_box_says_what_goes_to_the_assistant() {
    use crate::model::{Assistant, Attachment, Family};
    use crate::views::thread_panel::{ThreadPanel, ThreadPanelProps};
    let root = pane();
    let mut photo_root = message(10);
    photo_root.attachments = Some(vec![Attachment {
        id: 34,
        kind: "photo".into(),
        mime: Some("image/jpeg".into()),
        has_preview: true,
        ..Attachment::default()
    }]);
    let props = ThreadPanelProps {
        chat_id: 42,
        root_id: 10,
        messages: vec![photo_root],
        my_user_id: 7,
        is_family_chat: true,
        is_ai_chat: false,
        names: Default::default(),
        members: Vec::new(),
        assistant: Some(Assistant {
            user_id: 2,
            display_name: "Assistant".into(),
            mention: Some("@ai".into()),
            draw: Some("/draw".into()),
            vision: true,
            images: false,
        }),
        blocked: Default::default(),
        revealed: Default::default(),
        revealed_quotes: Default::default(),
        failed: Default::default(),
        ai_failed: Default::default(),
        family: Some(Family {
            id: 3,
            name: "The Smiths".into(),
            ai_history: true,
            ai_vision: true,
            ai_history_photos: false,
            ..Default::default()
        }),
        on_action: Callback::noop(),
    };
    let handle =
        yew::Renderer::<ThreadPanel>::with_root_and_props(root.clone().into(), props).render();
    TimeoutFuture::new(50).await;
    type_into(&query(&root, "textarea"), "@ai what is this?");
    TimeoutFuture::new(20).await;
    let said = query(&root, ".picture-notice")
        .text_content()
        .unwrap_or_default();
    assert!(
        said.contains("The photo you're replying to goes to the model"),
        "{said}"
    );
    handle.destroy();
    root.remove();
}

/// The whole page in another language: the reader's, from the browser, and
/// applied where it can only be seen in a real render — an attribute Yew
/// writes and a button's own words.
#[wasm_bindgen_test]
async fn a_russian_reader_reads_the_page_in_russian() {
    use fc_text::i18n::{use_lang, Lang};

    install_stylesheet();
    let root = fixed_root(
        "position:fixed;top:0;left:0;width:600px;height:320px;\
         display:grid;grid-template-rows:minmax(0,1fr);",
    );
    use_lang(Lang::Ru);
    let handle =
        yew::Renderer::<Conversation>::with_root_and_props(root.clone().into(), props(3)).render();
    TimeoutFuture::new(50).await;
    let label = query(&root, "textarea")
        .get_attribute("aria-label")
        .unwrap_or_default();
    let send = query(&root, ".composer button:not(.tool):not(.link)")
        .text_content()
        .unwrap_or_default();
    // Back to English BEFORE the assertions: every other test here reads
    // the page in it, and the reader's language outlives one test.
    use_lang(Lang::En);
    handle.destroy();
    root.remove();
    assert_eq!(label, "Сообщение");
    assert_eq!(send.trim(), "Отправить");
}

/// The composer is ONE ROW: the box for the words is as tall as the buttons
/// beside it, not twice their height — and it grows with what is typed,
/// to five lines and no further.
#[wasm_bindgen_test]
async fn the_message_box_is_as_tall_as_the_buttons_beside_it() {
    install_stylesheet();
    let root = fixed_root(
        "position:fixed;top:0;left:0;width:600px;height:320px;\
         display:grid;grid-template-rows:minmax(0,1fr);",
    );
    let handle =
        yew::Renderer::<Conversation>::with_root_and_props(root.clone().into(), props(3)).render();
    TimeoutFuture::new(50).await;
    let area = query(&root, ".composer textarea");
    let send = query(&root, ".composer button:not(.tool):not(.link)");
    let empty = area.get_bounding_client_rect().height();
    let button = send.get_bounding_client_rect().height();
    assert!(
        (empty - button).abs() <= 2.0,
        "an empty box is the height of the Send button: {empty} vs {button}"
    );

    // One line stays one line; five lines is five lines tall.
    type_into(&area, "one line, typed");
    TimeoutFuture::new(30).await;
    let one_line = area.get_bounding_client_rect().height();
    assert!(
        (one_line - empty).abs() <= 1.0,
        "a line of words does not make it grow: {one_line} vs {empty}"
    );

    type_into(&area, "one\ntwo\nthree\nfour\nfive");
    TimeoutFuture::new(30).await;
    let five = area.get_bounding_client_rect().height();
    assert!(
        five - one_line >= 60.0,
        "five lines grew the box by four of them: {five} vs {one_line}"
    );

    // And no further: the sixth line scrolls inside a box the same height.
    type_into(&area, "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight");
    TimeoutFuture::new(30).await;
    let many = area.get_bounding_client_rect().height();
    let scroller = area
        .dyn_ref::<HtmlTextAreaElement>()
        .expect("a textarea")
        .scroll_height();
    assert!(
        (many - five).abs() <= 1.0,
        "the ceiling holds at five lines: {many} vs {five}"
    );
    assert!(
        f64::from(scroller) > many + 1.0,
        "and what is past it scrolls inside: {scroller} vs {many}"
    );

    // Emptied by a send, it comes back to one row.
    type_into(&area, "");
    TimeoutFuture::new(30).await;
    let back = area.get_bounding_client_rect().height();
    assert!(
        (back - empty).abs() <= 1.0,
        "an emptied box is one row again: {back} vs {empty}"
    );
    handle.destroy();
    root.remove();
}

/// A mention is BOLD, and the SAME COLOUR as the words around it — on my
/// own balloon above all, whose background is the tint the mention used to
/// be drawn in (docs/protocol.md, "Mentioning a member").
#[wasm_bindgen_test]
async fn a_mention_is_bold_and_the_colour_of_the_words_around_it() {
    install_stylesheet();
    let root = fixed_root(
        "position:fixed;top:0;left:0;width:600px;height:400px;\
         display:grid;grid-template-rows:minmax(0,1fr);",
    );
    let named = |id: i64, sender: i64| Message {
        id,
        chat_id: 42,
        sender_id: sender,
        body: "@Anna are you in?".into(),
        created_at: "2026-08-19T17:03:12Z".into(),
        mentions: Some(vec![Mention {
            user_id: 8,
            name: "Anna".into(),
        }]),
        ..Message::default()
    };
    // 7 is the reader, so the first is mine and the second is theirs.
    let mut with_mentions = props_with(
        vec![named(1, 7), named(2, 9)],
        Some(Opening {
            chat_id: 42,
            unread_count: 0,
            last_read_message_id: 0,
        }),
        Callback::noop(),
    );
    with_mentions.members = vec![Member {
        id: 8,
        display_name: "Anna".into(),
        ..Member::default()
    }];
    let handle =
        yew::Renderer::<Conversation>::with_root_and_props(root.clone().into(), with_mentions)
            .render();
    TimeoutFuture::new(50).await;

    let window = web_sys::window().expect("a window");
    let colour = |element: &Element| -> String {
        window
            .get_computed_style(element)
            .ok()
            .flatten()
            .and_then(|style| style.get_property_value("color").ok())
            .unwrap_or_default()
    };
    let weight = |element: &Element| -> String {
        window
            .get_computed_style(element)
            .ok()
            .flatten()
            .and_then(|style| style.get_property_value("font-weight").ok())
            .unwrap_or_default()
    };

    let bubbles = root.query_selector_all(".bubble").expect("a selector");
    assert_eq!(bubbles.length(), 2, "one of mine, one of theirs");
    for index in 0..bubbles.length() {
        let bubble: Element = bubbles.get(index).expect("a bubble").dyn_into().unwrap();
        let mention = bubble
            .query_selector(".mention")
            .expect("a selector")
            .expect("the mention is drawn");
        let body = bubble
            .query_selector(".md")
            .expect("a selector")
            .expect("the body is drawn");
        assert_eq!(
            colour(&mention),
            colour(&body),
            "the mention reads in the body's own colour, bubble {index}"
        );
        let heavy: i32 = weight(&mention).parse().unwrap_or(400);
        assert!(
            heavy >= 600,
            "and it is bold: {} in bubble {index}",
            weight(&mention)
        );
    }
    handle.destroy();
    root.remove();
}

/// A PICTURE IN THE VIEWER FITS THE WINDOW, both ways.
///
/// The bug this pins was a stylesheet rule and nothing else, which is why it
/// is here rather than in the viewer's own tests: `.viewer-photo` is a grid,
/// its tracks were AUTO, an auto track grows to its content — so a 600×1200
/// photograph made the grid area 1200 tall, `max-height: 100%` resolved
/// against that, and the picture opened at natural size and was clipped by
/// the overflow. In a window with room for all of it, a reader saw the
/// middle third.
///
/// The markup is built by hand: the component needs a media loader and a
/// fetched URL to draw anything, and what is under test is the CSS.
#[wasm_bindgen_test]
async fn a_tall_picture_in_the_viewer_fits_the_window() {
    install_stylesheet();
    let document = document();
    // A viewer over a window-sized root, as the real one is (`position:
    // fixed; inset: 0`), with a bar above the stage.
    let root = fixed_root(
        "position:fixed;top:0;left:0;width:900px;height:600px;display:grid;\
         grid-template-rows:minmax(0,1fr);",
    );
    root.set_inner_html(
        "<div class=\"viewer\">\
           <div class=\"viewer-bar\"><span class=\"viewer-title\">Photo</span></div>\
           <div class=\"viewer-stage\">\
             <div class=\"viewer-photo\"><img alt=\"\" /></div>\
           </div>\
         </div>",
    );
    let image: HtmlElement = root
        .query_selector(".viewer-photo img")
        .expect("a selector")
        .expect("the picture")
        .dyn_into()
        .unwrap();
    // A 3×6 pixel PNG stretched by CSS behaves like the 600×1200 one: what
    // decides the layout is the intrinsic RATIO, not the pixel count.
    let tall = "data:image/svg+xml;base64,\
        PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSI2MDAiIGhlaWdodD0iMTIwMCI+\
        PHJlY3Qgd2lkdGg9IjYwMCIgaGVpZ2h0PSIxMjAwIiBmaWxsPSIjMDA4MGZmIi8+PC9zdmc+";
    image
        .set_attribute("src", tall)
        .expect("the picture's source");
    // Two frames: one for the load, one for the layout it changes.
    TimeoutFuture::new(120).await;

    let stage: HtmlElement = root
        .query_selector(".viewer-stage")
        .expect("a selector")
        .expect("the stage")
        .dyn_into()
        .unwrap();
    let drawn = image.get_bounding_client_rect();
    let box_ = stage.get_bounding_client_rect();
    assert!(drawn.height() > 0.0, "the picture drew at all");
    assert!(
        drawn.height() <= box_.height() + 1.0,
        "the picture is no taller than the stage: {} in {}",
        drawn.height(),
        box_.height()
    );
    assert!(
        drawn.width() <= box_.width() + 1.0,
        "nor wider: {} in {}",
        drawn.width(),
        box_.width()
    );
    // And it keeps its own shape rather than being squashed into the box.
    let ratio = drawn.width() / drawn.height();
    assert!(
        (ratio - 0.5).abs() < 0.02,
        "a 1:2 photograph stays 1:2: {ratio}"
    );
    root.remove();
}
