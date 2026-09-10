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
    }];
    let handle =
        yew::Renderer::<Conversation>::with_root_and_props(root.clone().into(), asked).render();
    TimeoutFuture::new(50).await;
    query(&root, "[aria-label='New poll']")
        .dyn_into_html()
        .click();
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
