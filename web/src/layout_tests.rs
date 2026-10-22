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
pub(crate) fn install_stylesheet() {
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
pub(crate) fn fixed_root(style: &str) -> HtmlElement {
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

pub(crate) trait IntoHtml {
    fn dyn_into_html(self) -> HtmlElement;
}

impl IntoHtml for Element {
    fn dyn_into_html(self) -> HtmlElement {
        use wasm_bindgen::JsCast;
        self.dyn_into::<HtmlElement>().expect("an HTML element")
    }
}

pub(crate) fn query(root: &Element, selector: &str) -> Element {
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

pub(crate) fn message(id: i64) -> Message {
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
pub(crate) fn recorder() -> (Rc<RefCell<Vec<Action>>>, Callback<Action>) {
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

pub(crate) fn pane() -> HtmlElement {
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

/// An element's text without what is `aria-hidden` — a menu line's label
/// without its decorative icon (the paperclip's menu, #78).
pub(crate) fn visible_text(element: &Element) -> String {
    let children = element.child_nodes();
    let mut text = String::new();
    for index in 0..children.length() {
        let Some(node) = children.item(index) else {
            continue;
        };
        match node.dyn_ref::<Element>() {
            Some(child) if child.get_attribute("aria-hidden").as_deref() == Some("true") => {}
            Some(child) => text.push_str(&visible_text(child)),
            None => text.push_str(&node.text_content().unwrap_or_default()),
        }
    }
    text
}

pub(crate) fn click_labelled(root: &Element, selector: &str, label: &str) {
    let found = root.query_selector_all(selector).expect("a valid selector");
    for index in 0..found.length() {
        let element = found.item(index).expect("an element");
        // Either reading: a label with a decorative, aria-hidden icon
        // (the paperclip's menu) or one whose every character counts.
        if element
            .dyn_ref::<Element>()
            .is_some_and(|element| visible_text(element).trim() == label)
            || element.text_content().unwrap_or_default().trim() == label
        {
            element
                .dyn_into::<HtmlElement>()
                .expect("an HTML element")
                .click();
            return;
        }
    }
    panic!("no {selector} reading {label:?}");
}

pub(crate) fn props_with(
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
                ..Chat::default()
            },
            last_message: None,
            unread_count: 0,
            last_read_message_id: 0,
            max_reaction_seq: None,
            max_edit_seq: None,
            max_poll_seq: None,
            mentioned: false,
        },
        messages: messages.into(),
        my_user_id: 7,
        names: Default::default(),
        members: Vec::new(),
        assistant: None,
        blocked: Default::default(),
        revealed: Default::default(),
        revealed_quotes: Default::default(),
        opening,
        staged: Vec::new(),
        not_sent: Vec::new(),
        session: 0,
        family: None,
        failed: Default::default(),
        ai_failed: Default::default(),
        peer_read: 0,
        typing: Vec::new(),
        can_load_more: false,
        unanswered_polls: 0,
        draft: String::new(),
        support_contact: None,
        stickers: None,
        agreed_to_assistant: true,
        transcripts: Default::default(),
        on_action,
        now_ms: 0.0,
        round: None,
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

fn drawn_rows(messages: &Element) -> u32 {
    messages
        .query_selector_all("article")
        .expect("a valid selector")
        .length()
}

/// A LONG chat is drawn a window at a time (timeline::Window): its newest
/// rows when it opens, and more of them as the reader scrolls up to them —
/// with what they were reading left exactly where it was, and no "Earlier
/// messages" until the rows held above are all drawn.
#[wasm_bindgen_test]
async fn a_long_chat_draws_its_newest_rows_and_more_as_the_reader_scrolls_up() {
    use crate::timeline::{WINDOW, WINDOW_STEP};
    let root = pane();
    let mut long = props(500);
    long.can_load_more = true;
    let handle =
        yew::Renderer::<Conversation>::with_root_and_props(root.clone().into(), long).render();
    TimeoutFuture::new(50).await;
    let messages = query(&root, ".messages");
    assert_eq!(drawn_rows(&messages), WINDOW as u32, "the newest window");
    assert!(is_at_newest(&messages), "opened at the newest");
    assert!(document().get_element_by_id("m-500").is_some());
    assert!(document().get_element_by_id("m-400").is_none());
    assert!(
        root.query_selector(".load-more").unwrap().is_none(),
        "rows are held above the window: those first"
    );

    let first = document().get_element_by_id("m-401").expect("the top row");
    // The scroll event is handled as it is dispatched; the rows it asks for
    // are drawn after — so this is where the row was before they were.
    scroll_to(&messages, 0);
    let was = first.get_bounding_client_rect().top();
    TimeoutFuture::new(50).await;
    assert_eq!(
        drawn_rows(&messages),
        (WINDOW + WINDOW_STEP) as u32,
        "scrolling up drew more"
    );
    assert!(document().get_element_by_id("m-351").is_some());
    let now = document().get_element_by_id("m-401").expect("still drawn");
    assert!(
        (now.get_bounding_client_rect().top() - was).abs() < 2.0,
        "the row being read stayed put: {was} then {}",
        now.get_bounding_client_rect().top()
    );
    assert!(
        messages.scroll_top() > 0,
        "the reader is where they were, not at the new top"
    );

    handle.destroy();
    root.remove();
}

/// A chat opening at a divider far above the newest — well outside the
/// newest window — still opens AT it: the window is taken up to the divider
/// before the opening scrolls there.
#[wasm_bindgen_test]
async fn a_long_chat_opens_at_a_divider_above_its_newest_window() {
    let root = pane();
    let (log, on_action) = recorder();
    let loading = props_with((1..=500).map(message).collect(), None, on_action.clone());
    let mut handle =
        yew::Renderer::<Conversation>::with_root_and_props(root.clone().into(), loading).render();
    TimeoutFuture::new(50).await;
    // 125 unread from Anna (the odd ids), counted back from the newest with
    // no marker: the first of them is 251, 250 rows up.
    handle.update(props_with(
        (1..=500).map(message).collect(),
        Some(Opening {
            chat_id: 42,
            unread_count: 125,
            last_read_message_id: 0,
        }),
        on_action,
    ));
    TimeoutFuture::new(50).await;
    let divider = query(&root, "#unread-divider");
    assert_eq!(
        divider.text_content().unwrap_or_default().trim(),
        "125 new messages"
    );
    assert_eq!(
        divider
            .next_element_sibling()
            .and_then(|row| row.query_selector("article").ok().flatten())
            .map(|bubble| bubble.id()),
        Some("m-251".to_string())
    );
    let messages = query(&root, ".messages");
    let list_top = messages.get_bounding_client_rect().top();
    let at = divider.get_bounding_client_rect().top();
    assert!(
        (at - list_top).abs() < 40.0,
        "opened with the divider at the top: {at} against {list_top}"
    );
    assert_eq!(at_newest_said(&log), vec![false], "opened at the divider");

    handle.destroy();
    root.remove();
}

/// A quote whose message is held but above the window is still a way to
/// it: the window is taken up to the quoted message, which is shown and
/// tinted.
#[wasm_bindgen_test]
async fn a_quote_reaches_a_message_above_the_window() {
    let root = pane();
    let mut held: Vec<Message> = (1..=500).map(message).collect();
    held[499].reply_to = Some(crate::model::ReplyTo {
        message_id: 50,
        sender_id: 9,
        excerpt: "Message number 50".into(),
        parent: None,
    });
    let handle = yew::Renderer::<Conversation>::with_root_and_props(
        root.clone().into(),
        props_with(
            held,
            Some(Opening {
                chat_id: 42,
                unread_count: 0,
                last_read_message_id: 500,
            }),
            Callback::noop(),
        ),
    )
    .render();
    TimeoutFuture::new(50).await;
    assert!(
        document().get_element_by_id("m-50").is_none(),
        "not drawn yet"
    );
    let newest = document().get_element_by_id("m-500").expect("the reply");
    query(&newest, ".quote").dyn_into_html().click();
    TimeoutFuture::new(80).await;
    let target = document()
        .get_element_by_id("m-50")
        .expect("drawn for the jump");
    let messages = query(&root, ".messages");
    let list = messages.get_bounding_client_rect();
    let shown = target.get_bounding_client_rect();
    assert!(
        shown.top() >= list.top() - 1.0 && shown.top() < list.bottom(),
        "message 50 is on screen: {} in {}..{}",
        shown.top(),
        list.top(),
        list.bottom()
    );
    assert!(
        target
            .parent_element()
            .is_some_and(|row| row.class_list().contains("is-highlighted")),
        "and tinted"
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
                processor: Some("Microsoft — Azure OpenAI".into()),
                transcribe: false,
                transcribe_max_bytes: None,
                lookups: Vec::new(),
                greeting_weather: false,
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
                    visible_text(&found.item(index).unwrap().dyn_into::<Element>().unwrap())
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
    // The slot is an icon now (#79, S8.7); "Send" is the word it keeps as
    // visually hidden text, which is what is read — and clicked — here.
    let slot = query(&root, ".composer .slot");
    assert!(slot.query_selector("svg").unwrap().is_some(), "an icon");
    assert_eq!(
        query(&slot, ".visually-hidden").text_content().as_deref(),
        Some("Send")
    );
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

/// The conversation hands its staged strip to the composer, which draws it
/// in the one order of #106 (doc 2.4): inside the composer, under the
/// notices and the reply, above the row — not above the whole composer.
#[wasm_bindgen_test]
async fn the_staged_strip_is_drawn_inside_the_composer_above_the_row() {
    let root = pane();
    let (_, on_action) = recorder();
    let mut props = props_with(vec![my_message(100, "hi")], None, on_action);
    props.staged = vec![crate::staged::Prepared::location(1.0, 2.0, None)];
    let handle =
        yew::Renderer::<Conversation>::with_root_and_props(root.clone().into(), props).render();
    TimeoutFuture::new(50).await;
    let strip = query(&root, ".composer-wrap > .staging");
    let mut after = strip.next_element_sibling();
    while let Some(element) = after
        .as_ref()
        .filter(|element| !element.matches(".composer").unwrap_or(false))
    {
        after = element.next_element_sibling();
    }
    assert!(after.is_some(), "the row comes after the strip");
    assert!(
        rect_of(&root, ".staging").bottom() <= rect_of(&root, ".composer").top() + 0.5,
        "above the row"
    );
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
        messages: vec![photo_root].into(),
        my_user_id: 7,
        chat_kind: "family".into(),
        names: Default::default(),
        members: Vec::new(),
        chat_members: Vec::new(),
        assistant: Some(Assistant {
            user_id: 2,
            display_name: "Assistant".into(),
            mention: Some("@ai".into()),
            draw: Some("/draw".into()),
            vision: true,
            transcribe: false,
            transcribe_max_bytes: None,
            lookups: Vec::new(),
            greeting_weather: false,
            images: false,
            processor: Some("Microsoft — Azure OpenAI".into()),
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
        stickers: None,
        agreed_to_assistant: true,
        transcripts: Default::default(),
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
    // An empty box's slot is the microphone (#79, S1.3), named for it.
    let microphone = query(&root, ".composer .slot")
        .get_attribute("aria-label")
        .unwrap_or_default();
    type_into(&query(&root, "textarea"), "Привет");
    TimeoutFuture::new(20).await;
    // With words it is Send, and keeps the word as visually hidden text.
    let send = query(&root, ".composer .slot")
        .text_content()
        .unwrap_or_default();
    // Back to English BEFORE the assertions: every other test here reads
    // the page in it, and the reader's language outlives one test.
    use_lang(Lang::En);
    handle.destroy();
    root.remove();
    assert_eq!(label, "Сообщение");
    assert_eq!(microphone, "Записать голосовое сообщение");
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
    // The Send slot: a fixed icon button now (#79, S1.3), the same height
    // whichever it shows — the microphone of an empty box, here.
    let send = query(&root, ".composer .slot");
    let empty = area.get_bounding_client_rect().height();
    let button = send.get_bounding_client_rect().height();
    assert!(
        (empty - button).abs() <= 2.0,
        "an empty box is the height of the Send slot: {empty} vs {button}"
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

/// A file pasted onto the page, the way ⌘V outside the box delivers one.
fn paste(file: &web_sys::File) {
    js_sys::Function::new_with_args(
        "file",
        "const carried = new DataTransfer(); carried.items.add(file); \
         document.body.dispatchEvent(new ClipboardEvent('paste', \
           { clipboardData: carried, bubbles: true, cancelable: true }));",
    )
    .call1(&wasm_bindgen::JsValue::NULL, file)
    .expect("pasting");
}

/// The media notice's words, once they satisfy `wanted` — waited for, up to
/// `ms`.
async fn notice_saying(root: &Element, ms: u32, wanted: impl Fn(&str) -> bool) -> Option<String> {
    for _ in 0..ms / 20 {
        let said = root
            .query_selector(".media-notice")
            .expect("a selector")
            .and_then(|notice| notice.text_content());
        if let Some(said) = said.filter(|said| wanted(said)) {
            return Some(said);
        }
        TimeoutFuture::new(20).await;
    }
    None
}

fn staged_so_far(log: &Rc<RefCell<Vec<Action>>>) -> usize {
    log.borrow()
        .iter()
        .filter(|action| matches!(action, Action::Stage { .. }))
        .count()
}

/// A video outside the profile is transcoded as it is staged — seconds for
/// a short clip, minutes for a long one — and the composer is busy for as
/// long. So the notice says how far it has got, and offers a way out:
/// Cancel gives the composer back at once and stages NOTHING, neither the
/// transcode nor the original in its place. And a pane that goes away
/// takes its transcode with it.
#[wasm_bindgen_test]
async fn a_long_preparation_says_how_far_it_is_and_can_be_called_off() {
    let root = pane();
    let (log, on_action) = recorder();
    let asked = props_with(vec![message(1)], None, on_action);
    let handle =
        yew::Renderer::<Conversation>::with_root_and_props(root.clone().into(), asked).render();
    TimeoutFuture::new(50).await;
    let clip = crate::encode::testing::film(1280, 720, 60, 3, 8_000_000, Some(256_000)).await;
    let options = web_sys::FilePropertyBag::new();
    options.set_type("video/mp4");
    let picked = web_sys::File::new_with_blob_sequence_and_options(
        &js_sys::Array::of1(&clip),
        "IMG_0042.mp4",
        &options,
    )
    .expect("a file");

    paste(&picked);
    let said = notice_saying(&root, 10_000, |said| said.contains('%'))
        .await
        .expect("the notice says how far the transcode has got");
    assert!(said.starts_with("Preparing…"), "{said}");
    assert_eq!(staged_so_far(&log), 0);
    // While it is busy, the way out is Cancel — not a notice to dismiss.
    assert!(root
        .query_selector(".media-notice button[aria-label='Dismiss']")
        .unwrap()
        .is_none());
    click_labelled(&root, ".media-notice button", "Cancel");
    TimeoutFuture::new(50).await;
    assert!(
        root.query_selector(".media-notice").unwrap().is_none(),
        "the composer is given back at once"
    );
    // Long enough for the whole transcode, had it gone on.
    TimeoutFuture::new(2_500).await;
    assert_eq!(staged_so_far(&log), 0, "nothing of it was staged");
    assert!(root.query_selector(".media-notice").unwrap().is_none());

    // The composer is free again: the same file, left alone, is staged —
    // as the profile's MP4, not as the original.
    paste(&picked);
    for _ in 0..500 {
        if staged_so_far(&log) > 0 {
            break;
        }
        TimeoutFuture::new(20).await;
    }
    let staged = log
        .borrow()
        .iter()
        .find_map(|action| match action {
            Action::Stage { item, .. } => Some(item.clone()),
            _ => None,
        })
        .expect("staged");
    assert_eq!(staged.kind, "video");
    assert!((staged.size as f64) < clip.size() / 2.0);
    TimeoutFuture::new(50).await;
    assert!(
        root.query_selector(".media-notice").unwrap().is_none(),
        "and the notice goes when it is done"
    );

    // A pane that goes away mid-transcode stages nothing after it.
    paste(&picked);
    notice_saying(&root, 10_000, |said| said.contains('%'))
        .await
        .expect("under way again");
    handle.destroy();
    TimeoutFuture::new(2_500).await;
    assert_eq!(staged_so_far(&log), 1, "only the one that was let finish");
    root.remove();
}

/// The sticker button is beside the paperclip in EVERY chat a message can
/// be sent in, wherever there is a pack to send from — the family chat, a
/// one-to-one chat and the assistant's own — and nowhere there is none: a
/// server that predates packs, an account in no family.
#[wasm_bindgen_test]
async fn the_sticker_button_is_there_only_where_a_pack_is() {
    install_stylesheet();
    let mount = |kind: &'static str, stickers: Option<Vec<crate::model::PackItem>>| async move {
        let root = pane();
        let (_, on_action) = recorder();
        let mut props = props_with(vec![message(1)], None, on_action);
        props.item.chat.kind = kind.into();
        props.stickers = stickers;
        let handle =
            yew::Renderer::<Conversation>::with_root_and_props(root.clone().into(), props).render();
        TimeoutFuture::new(50).await;
        let there = root
            .query_selector(".composer .pack-menu .tool")
            .expect("a valid selector")
            .is_some();
        handle.destroy();
        root.remove();
        there
    };
    assert!(
        mount("family", Some(Vec::new())).await,
        "an empty pack still has a panel"
    );
    assert!(
        mount("direct", Some(Vec::new())).await,
        "any chat between people"
    );
    assert!(!mount("family", None).await, "a server from before packs");
    assert!(
        mount("ai", Some(Vec::new())).await,
        "the assistant's chat too — through its consent question"
    );
    assert!(!mount("ai", None).await, "a server from before packs");
}

/// An assistant as `GET /families/mine` names one — or does not.
fn assistant_answered_by(processor: Option<&str>) -> crate::model::Assistant {
    crate::model::Assistant {
        user_id: 2,
        display_name: "Assistant".into(),
        mention: Some("@ai".into()),
        transcribe: false,
        transcribe_max_bytes: None,
        lookups: Vec::new(),
        greeting_weather: false,
        draw: Some("/draw".into()),
        vision: true,
        images: false,
        processor: processor.map(str::to_string),
    }
}

/// A media cache that already holds the pictures of [`pack_of`], so the
/// panel's stickers are there to be clicked.
fn loader_holding(count: i64) -> crate::media::MediaLoader {
    let loader = crate::media::MediaLoader::new(crate::live::Live::new(
        crate::live::AppState {
            token: Some("t".into()),
            ..Default::default()
        },
        Rc::new(|| {}),
    ));
    for id in 1..=count {
        let parts = js_sys::Array::of1(&wasm_bindgen::JsValue::from_str("party cat"));
        loader.seed(
            700 + id,
            crate::media::Variant::Sticker,
            web_sys::Blob::new_with_str_sequence(&parts).expect("a blob"),
        );
    }
    loader
}

/// WHERE THE SERVER CAN LOOK THINGS UP, THE CHAT'S CONSENT SCREEN OFFERS
/// LOOKUPS BESIDE THE ASSISTANT, never folded into it (docs/protocol.md,
/// "Consenting to the assistant", amended 2026-10-03): the providers are
/// named in one more line before anything is agreed; "Agree With Lookups"
/// asks for both, "Agree Without Lookups" for the assistant alone, and
/// "Not Now" for neither. On a server with no source the screen is the one
/// it always was.
#[wasm_bindgen_test]
async fn the_consent_screen_offers_lookups_beside_the_assistant_and_never_assumes_them() {
    let mount = |lookups: Vec<String>, on_action: Callback<Action>| async move {
        let root = pane_of(600.0);
        let mut props = props_with(vec![message(1)], None, on_action);
        props.item.chat.kind = "ai".into();
        let mut assistant = assistant_answered_by(Some("Microsoft — Azure OpenAI"));
        assistant.lookups = lookups;
        props.assistant = Some(assistant);
        props.agreed_to_assistant = false;
        props.stickers = Some(pack_of(3));
        let children = yew::html! { <Conversation ..props /> };
        let handle = yew::Renderer::<WithMedia>::with_root_and_props(
            root.clone().into(),
            WithMediaProps {
                loader: loader_holding(3),
                children,
            },
        )
        .render();
        TimeoutFuture::new(50).await;
        (root, handle)
    };
    let sources = || {
        ["Brave Search", "Open-Meteo", "Wikipedia"]
            .iter()
            .map(|name| name.to_string())
            .collect::<Vec<_>>()
    };
    let buttons = |root: &Element| -> Vec<String> {
        let found = root
            .query_selector_all(".dialog-actions button")
            .expect("a valid selector");
        (0..found.length())
            .map(|index| {
                found
                    .item(index)
                    .and_then(|button| button.text_content())
                    .unwrap_or_default()
                    .trim()
                    .to_string()
            })
            .collect()
    };

    for (pressed, expected) in [
        ("Agree With Lookups", Some(Action::AgreeWithLookups)),
        (
            "Agree Without Lookups",
            Some(Action::SetAssistantConsent { granted: true }),
        ),
        ("Not Now", None),
    ] {
        let (log, on_action) = recorder();
        let (root, handle) = mount(sources(), on_action).await;
        pick_a_sticker(&root, ".composer").await;
        let said = root.text_content().unwrap_or_default();
        assert!(said.contains("Before the assistant answers"), "{said}");
        assert!(
            said.contains("If your family's owner turns on lookups")
                && said.contains("to Brave Search, Open-Meteo and Wikipedia, and its answer then lists its sources."),
            "the providers are named before anything is agreed: {said}"
        );
        assert_eq!(
            buttons(&root),
            ["Not Now", "Agree Without Lookups", "Agree With Lookups"]
        );
        click_labelled(&root, ".dialog-actions button", pressed);
        TimeoutFuture::new(50).await;
        let consents: Vec<String> = log
            .borrow()
            .iter()
            .filter(|action| {
                matches!(
                    action,
                    Action::AgreeWithLookups
                        | Action::SetAssistantConsent { .. }
                        | Action::SetAssistantLookupConsent { .. }
                )
            })
            .map(|action| format!("{action:?}"))
            .collect();
        assert_eq!(
            consents,
            expected
                .iter()
                .map(|action| format!("{action:?}"))
                .collect::<Vec<_>>(),
            "{pressed}"
        );
        assert_eq!(
            stickers_sent(&log),
            0,
            "agreeing still sends nothing by itself"
        );
        handle.destroy();
        root.remove();
    }

    // No source on this server: the screen it always was.
    let (_log, on_action) = recorder();
    let (root, handle) = mount(Vec::new(), on_action).await;
    pick_a_sticker(&root, ".composer").await;
    assert!(!root.text_content().unwrap_or_default().contains("lookups"));
    assert_eq!(buttons(&root), ["Not Now", "I Agree"]);
    handle.destroy();
    root.remove();
}

/// Open the sticker panel under `composer` and click its first sticker.
async fn pick_a_sticker(root: &Element, composer: &str) {
    query(root, &format!("{composer} .pack-menu .tool"))
        .dyn_into_html()
        .click();
    TimeoutFuture::new(50).await;
    query(root, ".pack-panel .pack-cell:not([disabled])")
        .dyn_into_html()
        .click();
    TimeoutFuture::new(50).await;
}

fn stickers_sent(log: &Rc<RefCell<Vec<Action>>>) -> usize {
    log.borrow()
        .iter()
        .filter(|action| matches!(action, Action::SendSticker { .. }))
        .count()
}

/// IN THE ASSISTANT'S CHAT A STICKER GOES THROUGH THE CONSENT QUESTION,
/// never round it. One click there is a picture shown to the model, so
/// until this member has agreed the click raises the screen that asks —
/// the same one Send raises for words — and sends NOTHING; "Not Now"
/// leaves it unsent; "I Agree" records the answer and still sends nothing
/// by itself. Once agreed, one click sends, as anywhere. On a server that
/// names nobody it says so and sends nothing, agreed or not.
#[wasm_bindgen_test]
async fn a_sticker_in_the_assistants_chat_goes_through_the_consent_question() {
    let mount = |agreed: bool, processor: Option<&'static str>, on_action: Callback<Action>| async move {
        let root = pane_of(600.0);
        let mut props = props_with(vec![message(1)], None, on_action);
        props.item.chat.kind = "ai".into();
        props.assistant = Some(assistant_answered_by(processor));
        props.agreed_to_assistant = agreed;
        props.stickers = Some(pack_of(3));
        let children = yew::html! { <Conversation ..props /> };
        let handle = yew::Renderer::<WithMedia>::with_root_and_props(
            root.clone().into(),
            WithMediaProps {
                loader: loader_holding(3),
                children,
            },
        )
        .render();
        TimeoutFuture::new(50).await;
        (root, handle)
    };
    let asking = |root: &Element| {
        root.text_content()
            .unwrap_or_default()
            .contains("Before the assistant answers")
    };

    // Not yet agreed: asked, not sent.
    let (log, on_action) = recorder();
    let (root, handle) = mount(false, Some("Microsoft — Azure OpenAI"), on_action).await;
    pick_a_sticker(&root, ".composer").await;
    assert!(asking(&root), "the consent screen is up");
    assert_eq!(stickers_sent(&log), 0, "and nothing went round it");
    click_labelled(&root, ".dialog-actions button", "Not Now");
    TimeoutFuture::new(50).await;
    assert!(!asking(&root));
    assert_eq!(stickers_sent(&log), 0, "declined: still unsent");
    pick_a_sticker(&root, ".composer").await;
    click_labelled(&root, ".dialog-actions button", "I Agree");
    TimeoutFuture::new(50).await;
    assert!(
        log.borrow()
            .iter()
            .any(|action| matches!(action, Action::SetAssistantConsent { granted: true })),
        "{:?}",
        log.borrow()
    );
    assert_eq!(
        stickers_sent(&log),
        0,
        "agreeing records the answer; the sticker is picked again to send"
    );
    handle.destroy();
    root.remove();

    // Agreed: one click sends, as in any chat.
    let (log, on_action) = recorder();
    let (root, handle) = mount(true, Some("Microsoft — Azure OpenAI"), on_action).await;
    pick_a_sticker(&root, ".composer").await;
    assert!(!asking(&root));
    assert!(
        log.borrow().iter().any(|action| matches!(
            action,
            Action::SendSticker {
                chat_id: 42,
                item_id: 1,
                reply_to_message_id: None
            }
        )),
        "{:?}",
        log.borrow()
    );
    handle.destroy();
    root.remove();

    // Nobody named: nothing is sent, and the reason is said.
    for agreed in [false, true] {
        let (log, on_action) = recorder();
        let (root, handle) = mount(agreed, None, on_action).await;
        pick_a_sticker(&root, ".composer").await;
        assert_eq!(stickers_sent(&log), 0);
        assert!(!asking(&root), "there is no honest way to ask");
        assert!(
            notice_saying(&root, 200, |said| said
                .contains("This server hasn't said which service answers"))
            .await
            .is_some(),
            "{}",
            root.text_content().unwrap_or_default()
        );
        handle.destroy();
        root.remove();
    }
}

/// A STICKER IS DRAWN WITH NO BUBBLE: no balloon behind it and no border
/// round it, mine or anybody's — its transparency shows the chat — in a box
/// that is the same for every sticker, with the picture fitted into it
/// WHOLE. A property of the stylesheet, so it is checked against the
/// shipped one; the class names are the ones views/bubble.rs and
/// views/stickers.rs render.
#[wasm_bindgen_test]
async fn a_sticker_has_no_bubble_and_is_fitted_whole() {
    install_stylesheet();
    let root = fixed_root("position:fixed;top:0;left:0;width:600px;height:400px;");
    // A 1×1 transparent GIF stands in for the picture: what is measured is
    // the box, not the bytes.
    let pixel = "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";
    root.set_inner_html(&format!(
        r#"<div class="messages">
             <div class="row"><article class="bubble is-mine is-media-only is-chat-sticker" id="mine">
               <button class="chat-sticker" style="width:160px;height:160px"><img src="{pixel}" alt="Sticker"></button>
               <div class="bubble-foot"><span class="meta">17:03</span></div></article></div>
             <div class="row"><article class="bubble is-chat-sticker" id="theirs">
               <div class="quote"><span class="quote-name">Anna</span><span class="quote-text">Dinner?</span></div>
               <button class="chat-sticker" style="width:160px;height:160px"><img src="{pixel}" alt="Sticker"></button>
             </article></div>
             <div class="row"><article class="bubble is-mine" id="words"><p class="body">hi</p></article></div>
           </div>"#
    ));
    TimeoutFuture::new(30).await;
    let window = web_sys::window().expect("a window");
    let style = |element: &Element, property: &str| {
        window
            .get_computed_style(element)
            .expect("computes")
            .expect("a style")
            .get_property_value(property)
            .expect("a value")
    };
    let clear = "rgba(0, 0, 0, 0)";
    for id in ["#mine", "#theirs"] {
        let bubble = query(&root, id);
        assert_eq!(
            style(&bubble, "background-color"),
            clear,
            "{id}: no balloon"
        );
        assert_eq!(style(&bubble, "border-top-color"), clear, "{id}: no border");
        assert_eq!(style(&bubble, "padding-top"), "0px");
        let sticker = query(&bubble, ".chat-sticker");
        assert_eq!(
            style(&sticker, "background-color"),
            clear,
            "{id}: the chat shows through"
        );
        let frame = sticker.get_bounding_client_rect();
        assert_eq!(
            (frame.width(), frame.height()),
            (160.0, 160.0),
            "{id}: the one box"
        );
        let picture = query(&sticker, "img");
        assert_eq!(
            style(&picture, "object-fit"),
            "contain",
            "{id}: whole, never cropped"
        );
        let drawn = picture.get_bounding_client_rect();
        assert_eq!((drawn.width(), drawn.height()), (160.0, 160.0));
    }
    // An ordinary message still has its balloon: the rule is the sticker's.
    assert_ne!(style(&query(&root, "#words"), "background-color"), clear);
    // A sticker that answers something keeps its quote readable.
    assert_ne!(
        style(&query(&root, "#theirs .quote"), "background-color"),
        clear
    );
    // Mine sits on my side of the chat, like any message of mine.
    let mine = query(&root, "#mine").get_bounding_client_rect();
    let theirs = query(&root, "#theirs").get_bounding_client_rect();
    assert!(
        mine.left() > theirs.left(),
        "{} vs {}",
        mine.left(),
        theirs.left()
    );
    root.remove();
}

/// A sticker shown larger stays INSIDE its dialog, whatever its own pixel
/// size: the picture fitted whole into a square stage, and the words and
/// the buttons under it still readable. Found end to end — a percentage
/// height in an auto-sized box let a big sticker open over the line that
/// says whether the pack holds it.
#[wasm_bindgen_test]
async fn a_sticker_shown_larger_stays_inside_its_dialog() {
    install_stylesheet();
    let root = fixed_root("position:fixed;top:0;left:0;width:900px;height:700px;");
    // Two thousand pixels square, as a finished sticker may be.
    let big = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='2000' height='2000'%3E%3Crect width='2000' height='2000' fill='red'/%3E%3C/svg%3E";
    root.set_inner_html(&format!(
        r#"<div class="dialog-backdrop"><div class="dialog chat-sticker-view">
             <h2>Sticker</h2>
             <div class="chat-sticker-stage"><img src="{big}" alt="Sticker"></div>
             <p class="footnote">Already in the family's stickers.</p>
             <div class="dialog-actions"><button class="secondary">Close</button></div>
           </div></div>"#
    ));
    TimeoutFuture::new(80).await;
    let dialog = query(&root, ".dialog").get_bounding_client_rect();
    let stage = query(&root, ".chat-sticker-stage").get_bounding_client_rect();
    let picture = query(&root, ".chat-sticker-stage img").get_bounding_client_rect();
    let words = query(&root, ".footnote").get_bounding_client_rect();
    let close = query(&root, ".dialog-actions").get_bounding_client_rect();
    assert!(
        (stage.width() - stage.height()).abs() < 1.0 && stage.width() <= 512.0,
        "a square no bigger than a made sticker: {}×{}",
        stage.width(),
        stage.height()
    );
    assert!(
        picture.width() <= stage.width() + 0.5 && picture.height() <= stage.height() + 0.5,
        "the picture is inside the stage: {}×{}",
        picture.width(),
        picture.height()
    );
    assert!(picture.bottom() <= words.top() + 0.5, "not over the words");
    assert!(words.bottom() <= close.top() + 0.5);
    assert!(
        close.bottom() <= dialog.bottom() && dialog.bottom() <= viewport_height(),
        "and the way out is on the screen"
    );
    root.remove();
}

/// A pack of `count` stickers, as a panel is handed one.
fn pack_of(count: i64) -> Vec<crate::model::PackItem> {
    (1..=count)
        .map(|id| crate::model::PackItem {
            id,
            pack_seq: id,
            added_by: Some(9),
            attachment: Some(crate::model::Attachment {
                id: 700 + id,
                kind: "photo".into(),
                mime: Some("image/webp".into()),
                size: Some(11),
                ..Default::default()
            }),
            ..Default::default()
        })
        .collect()
}

/// A pane of a given width, as a phone's whole window is.
fn pane_of(width: f64) -> HtmlElement {
    install_stylesheet();
    fixed_root(&format!(
        "position:fixed;top:0;left:0;width:{width}px;height:480px;\
         display:grid;grid-template-rows:minmax(0,1fr);"
    ))
}

/// Where the open sticker panel is, against the pane it is in: asserts it
/// is inside it on both sides, above the composer, no wider than it was
/// designed to be — and that its last column of stickers is inside IT.
fn assert_panel_is_on_screen(root: &Element, width: f64, pane: &str) {
    let inside = query(root, pane).get_bounding_client_rect();
    let panel = query(root, ".pack-panel").get_bounding_client_rect();
    let composer = query(root, ".composer-wrap").get_bounding_client_rect();
    assert!(
        panel.left() >= inside.left() && panel.right() <= inside.right(),
        "at {width}px the panel is {}..{} in a pane that is {}..{}",
        panel.left(),
        panel.right(),
        inside.left(),
        inside.right()
    );
    assert!(panel.width() <= 348.5, "{width}px: {} wide", panel.width());
    assert!(
        panel.width() >= (composer.width() - 32.0).min(348.0) - 0.5,
        "{width}px: as wide as the composer has room for, not {}",
        panel.width()
    );
    assert!(
        panel.bottom() <= composer.top() + 0.5 && panel.top() >= inside.top(),
        "{width}px: above the composer, under the top of the pane"
    );
    let cells = root
        .query_selector_all(".pack-panel .pack-cell")
        .expect("a valid selector");
    assert!(cells.length() > 0, "the pack is drawn");
    for index in 0..cells.length() {
        let cell = cells
            .item(index)
            .expect("a cell")
            .dyn_into::<Element>()
            .expect("an element")
            .get_bounding_client_rect();
        assert!(
            cell.left() >= panel.left() && cell.right() <= panel.right(),
            "{width}px: sticker {index} is {}..{} in a panel that is {}..{}",
            cell.left(),
            cell.right(),
            panel.left(),
            panel.right()
        );
    }
}

/// THE STICKER PANEL STAYS ON THE SCREEN, at a phone's width as at a
/// desk's. It once hung from the left edge of its own button — the second
/// tool, some sixty pixels in — at a width worked out from the WINDOW, and
/// so ran 27 pixels past the right edge of a 360-pixel phone, where the
/// conversation clips it: the last column of stickers and the scrollbar
/// were not there to be touched. A property of the stylesheet against the
/// markup the view really renders, so both are the shipped ones.
#[wasm_bindgen_test]
async fn the_sticker_panel_stays_on_the_screen_at_a_phones_width() {
    for width in [320.0, 360.0, 390.0, 600.0] {
        let root = pane_of(width);
        let (_, on_action) = recorder();
        let mut props = props_with(vec![message(1)], None, on_action);
        props.stickers = Some(pack_of(12));
        let handle =
            yew::Renderer::<Conversation>::with_root_and_props(root.clone().into(), props).render();
        TimeoutFuture::new(50).await;
        query(&root, ".composer .pack-menu .tool")
            .dyn_into_html()
            .click();
        TimeoutFuture::new(50).await;
        assert_panel_is_on_screen(&root, width, ".conversation");
        handle.destroy();
        root.remove();
    }
}

#[derive(yew::Properties, PartialEq)]
struct WithMediaProps {
    loader: crate::media::MediaLoader,
    children: yew::Html,
}

/// What the app gives every view: the tab's media cache.
#[yew::function_component(WithMedia)]
fn with_media(props: &WithMediaProps) -> yew::Html {
    yew::html! {
        <yew::ContextProvider<crate::media::MediaLoader> context={props.loader.clone()}>
            { props.children.clone() }
        </yew::ContextProvider<crate::media::MediaLoader>>
    }
}

/// A THREAD HAS THE STICKER BUTTON TOO (ios ThreadView), wherever the chat
/// has one — a server with packs — and what one click sends answers the
/// ROOT, as every send from a thread does. Its panel stays inside the
/// thread's pane, which is narrower than any phone. In a thread of the
/// ASSISTANT'S chat the button is there as well, and the click goes
/// through the consent question exactly as in the chat itself.
#[wasm_bindgen_test]
async fn a_thread_sends_a_sticker_to_its_root() {
    use crate::media::{MediaLoader, Variant};
    use crate::views::thread_panel::ThreadPanel;
    let loader = MediaLoader::new(crate::live::Live::new(
        crate::live::AppState {
            token: Some("t".into()),
            ..Default::default()
        },
        Rc::new(|| {}),
    ));
    let parts = js_sys::Array::of1(&wasm_bindgen::JsValue::from_str("party cat"));
    loader.seed(
        702,
        Variant::Sticker,
        web_sys::Blob::new_with_str_sequence(&parts).expect("a blob"),
    );
    let mount_agreed = |is_ai_chat: bool,
                        stickers: Option<Vec<crate::model::PackItem>>,
                        agreed_to_assistant: bool,
                        on_action: Callback<Action>| {
        let loader = loader.clone();
        async move {
            let root = pane_of(300.0);
            let children = yew::html! {
                <ThreadPanel
                    chat_id={42}
                    root_id={1}
                    messages={crate::store::Messages::from(vec![message(1), message(2)])}
                    my_user_id={7}
                    chat_kind={if is_ai_chat { "ai" } else { "family" }}
                    names={std::collections::HashMap::<i64, String>::new()}
                    members={Vec::<Member>::new()}
                    chat_members={Vec::<Member>::new()}
                    assistant={is_ai_chat.then(|| assistant_answered_by(Some("Microsoft — Azure OpenAI")))}
                    blocked={std::collections::HashSet::<i64>::new()}
                    revealed={std::collections::HashSet::<i64>::new()}
                    revealed_quotes={std::collections::HashSet::<(i64, u8)>::new()}
                    failed={std::collections::HashMap::<String, String>::new()}
                    ai_failed={std::collections::HashMap::<i64, crate::model::AiFailure>::new()}
                    {stickers}
                    {agreed_to_assistant}
                    {on_action}
                />
            };
            let handle = yew::Renderer::<WithMedia>::with_root_and_props(
                root.clone().into(),
                WithMediaProps { loader, children },
            )
            .render();
            TimeoutFuture::new(50).await;
            (root, handle)
        }
    };

    let mount = |is_ai_chat: bool,
                 stickers: Option<Vec<crate::model::PackItem>>,
                 on_action: Callback<Action>| {
        mount_agreed(is_ai_chat, stickers, false, on_action)
    };

    let (log, on_action) = recorder();
    let (root, handle) = mount(false, Some(pack_of(12)), on_action).await;
    query(&root, ".thread-panel .composer .pack-menu .tool")
        .dyn_into_html()
        .click();
    TimeoutFuture::new(50).await;
    assert_panel_is_on_screen(&root, 300.0, ".thread-panel");
    query(&root, ".pack-panel .pack-cell:not([disabled])")
        .dyn_into_html()
        .click();
    TimeoutFuture::new(50).await;
    assert!(
        matches!(
            log.borrow()[..],
            [Action::SendSticker {
                chat_id: 42,
                item_id: 2,
                reply_to_message_id: Some(1)
            }]
        ),
        "{:?}",
        log.borrow()
    );
    handle.destroy();
    root.remove();

    let (root, handle) = mount(false, None, Callback::noop()).await;
    assert!(
        root.query_selector(".pack-menu")
            .expect("a valid selector")
            .is_none(),
        "a server from before packs"
    );
    handle.destroy();
    root.remove();

    // A thread of the assistant's chat: the button is there, and until
    // this member has agreed the click asks instead of sending.
    let asking = |root: &Element| {
        root.text_content()
            .unwrap_or_default()
            .contains("Before the assistant answers")
    };
    let (log, on_action) = recorder();
    let (root, handle) = mount(true, Some(pack_of(12)), on_action).await;
    pick_a_sticker(&root, ".thread-panel .composer").await;
    assert!(asking(&root), "the consent screen, from the thread");
    assert_eq!(stickers_sent(&log), 0, "and nothing went round it");
    click_labelled(&root, ".dialog-actions button", "I Agree");
    TimeoutFuture::new(50).await;
    assert!(
        matches!(
            log.borrow()[..],
            [Action::SetAssistantConsent { granted: true }]
        ),
        "{:?}",
        log.borrow()
    );
    handle.destroy();
    root.remove();

    let (log, on_action) = recorder();
    let (root, handle) = mount_agreed(true, Some(pack_of(12)), true, on_action).await;
    pick_a_sticker(&root, ".thread-panel .composer").await;
    assert!(!asking(&root));
    assert!(
        matches!(
            log.borrow()[..],
            [Action::SendSticker {
                chat_id: 42,
                item_id: 2,
                reply_to_message_id: Some(1)
            }]
        ),
        "{:?}",
        log.borrow()
    );
    handle.destroy();
    root.remove();
}

/// A sticker whose bytes are still on their way says so with the spinner
/// — also in the OUTBOX, where a reload took the bytes and the upload is
/// fetching them again: that bubble was once an empty box with nothing to
/// say it was coming. A send that FAILED is waiting for nothing, and spins
/// nothing.
#[wasm_bindgen_test]
async fn a_sticker_still_on_its_way_spins_and_a_failed_one_does_not() {
    install_stylesheet();
    let root = fixed_root("position:fixed;top:0;left:0;width:600px;height:400px;");
    root.set_inner_html(
        r#"<div class="messages">
             <div class="row"><article class="bubble is-mine is-pending is-chat-sticker" id="queued">
               <button class="chat-sticker is-loading" style="width:160px;height:160px"></button></article></div>
             <div class="row"><article class="bubble is-mine is-failed is-chat-sticker" id="failed">
               <button class="chat-sticker is-loading" style="width:160px;height:160px"></button></article></div>
           </div>"#,
    );
    TimeoutFuture::new(30).await;
    let window = web_sys::window().expect("a window");
    let spinner = |id: &str| {
        window
            .get_computed_style_with_pseudo_elt(&query(&root, id), "::after")
            .expect("computes")
            .expect("a style")
            .get_property_value("content")
            .expect("a value")
    };
    assert_eq!(spinner("#queued .chat-sticker"), "\"\"", "on its way");
    assert_eq!(spinner("#failed .chat-sticker"), "none", "not coming");
    root.remove();
}

// --- #106: the row, the column, one surface at a time ----------------------
// docs/composer-and-wide-layout-2026-10-20.md

/// An assistant that names who answers: `@ai` may be written.
fn an_assistant() -> crate::model::Assistant {
    assistant_answered_by(Some("Microsoft — Azure OpenAI"))
}

fn rect_of(root: &Element, selector: &str) -> web_sys::DomRect {
    query(root, selector).get_bounding_client_rect()
}

/// The hit area a button's `::after` gives it, as wide and as tall.
fn hit_area(element: &Element) -> (f64, f64) {
    let style = web_sys::window()
        .expect("a window")
        .get_computed_style_with_pseudo_elt(element, "::after")
        .expect("a style")
        .expect("a style");
    let px = |property: &str| {
        style
            .get_property_value(property)
            .unwrap_or_default()
            .trim_end_matches("px")
            .parse::<f64>()
            .unwrap_or(0.0)
    };
    (px("width"), px("height"))
}

/// THE ROW ON A PHONE (#106, 2.1, and section 3's first fix): with every
/// button the family chat can have — the paperclip, the sticker button and
/// the video button inside the field, the slot — the slot is wholly on the
/// screen (it was clipped ~31 px off a 393-px one), the words have room
/// (at least 150 px at 390), and the sticker button stays INSIDE the field
/// while typing, so nothing moves under the thumb. `@ai` has no button of
/// its own any more, and every button is a 44-px target.
#[wasm_bindgen_test]
async fn the_row_fits_a_phone_with_every_button() {
    for width in [320.0, 360.0, 390.0] {
        let root = pane_of(width);
        let (_, on_action) = recorder();
        let mut props = props_with(vec![message(1)], None, on_action);
        props.stickers = Some(pack_of(3));
        props.assistant = Some(an_assistant());
        props.round = crate::round_video::RoundLimits::of(Some(60_000), Some(20_000_000));
        let handle =
            yew::Renderer::<Conversation>::with_root_and_props(root.clone().into(), props).render();
        // The video button comes once the browser has said what it can
        // record; where it cannot, the row is only roomier.
        let door = crate::round_record_tests::until(5_000, || {
            root.query_selector(".composer .field .video-door")
                .ok()
                .flatten()
                .is_some()
        })
        .await;
        let pane = root.get_bounding_client_rect();
        let slot = rect_of(&root, ".composer .slot");
        assert!(
            slot.left() >= pane.left() - 0.5 && slot.right() <= pane.right() + 0.5,
            "{width}px: the slot is {}..{} in a pane {}..{}",
            slot.left(),
            slot.right(),
            pane.left(),
            pane.right()
        );
        assert!(slot.width() >= 39.5, "{width}px: the slot is whole");
        let field = rect_of(&root, ".composer textarea");
        if width >= 390.0 {
            assert!(
                field.width() >= 150.0,
                "390px: the words have {} px (door {door})",
                field.width()
            );
        }
        let area = query(&root, ".composer .field > textarea");
        assert_eq!(
            area.get_attribute("placeholder").as_deref(),
            Some("Message")
        );
        assert!(
            root.query_selector(".composer .field .pack-menu .tool")
                .unwrap()
                .is_some(),
            "{width}px: the sticker button is inside the field"
        );
        assert!(
            root.query_selector(".composer > button").unwrap().is_none(),
            "{width}px: no `@ai` button in the row"
        );
        let in_field = root.query_selector_all(".composer .field button").unwrap();
        assert_eq!(
            in_field.length(),
            if door { 2 } else { 1 },
            "{width}px: in the field, the sticker button and the video button only"
        );
        let buttons = root
            .query_selector_all(".composer .tool, .composer .slot, .composer .video-door")
            .unwrap();
        for index in 0..buttons.length() {
            let button: Element = buttons.item(index).unwrap().dyn_into().unwrap();
            let (wide, tall) = hit_area(&button);
            assert!(
                wide >= 43.5 && tall >= 43.5,
                "{width}px: {} is a {wide}×{tall} target",
                button.get_attribute("aria-label").unwrap_or_default()
            );
        }

        // Typing: the sticker button stays where it was, and the words only
        // gain the video button's room.
        let sticker = rect_of(&root, ".composer .field .pack-menu .tool");
        type_into(&area, "Sunday works for us, we'll bring the cake");
        TimeoutFuture::new(30).await;
        let typed = rect_of(&root, ".composer .field .pack-menu .tool");
        assert!(
            (typed.right() - sticker.right()).abs() <= 48.5 && typed.width() > 0.0,
            "{width}px: the sticker button stays in the field"
        );
        assert!(rect_of(&root, ".composer textarea").width() >= field.width() - 0.5);
        let slot = rect_of(&root, ".composer .slot");
        assert!(
            slot.right() <= pane.right() + 0.5,
            "{width}px: Send is whole"
        );
        handle.destroy();
        root.remove();
    }
}

/// A message by somebody else, or by me, long enough to fill any bubble.
fn long_message(id: i64, sender_id: i64) -> Message {
    Message {
        sender_id,
        body: "So much to say about Sunday ".repeat(40),
        ..message(id)
    }
}

/// THE COLUMN (#106, 2.2): on a wide window the messages are one column,
/// min(pane − 32, 1024) wide and centred, and a bubble is at most
/// min(column − 48, 560), where it was 78 % of whatever the pane was. The
/// composer is NOT in it on the web (2026-10-21): it spans the pane with
/// the 16 px margin it had before #106.
#[wasm_bindgen_test]
async fn the_column_and_its_bubbles_on_a_wide_window() {
    for width in [1920.0, 1440.0, 600.0] {
        let root = pane_of(width);
        let (_, on_action) = recorder();
        let props = props_with(
            vec![long_message(1, 9), long_message(2, 7)],
            None,
            on_action,
        );
        let handle =
            yew::Renderer::<Conversation>::with_root_and_props(root.clone().into(), props).render();
        TimeoutFuture::new(60).await;
        let pane = root.get_bounding_client_rect();
        let column = (width - 32.0).min(1024.0);
        let side = (width - column) / 2.0;

        // The composer: the whole pane, 16 px in from either edge.
        let clip = rect_of(&root, ".composer .attach > .tool");
        let slot = rect_of(&root, ".composer .slot");
        assert!(
            (clip.left() - pane.left() - 16.0).abs() <= 1.0,
            "{width}px: the paperclip at {} — the composer starts 16 px in",
            clip.left() - pane.left()
        );
        assert!(
            (pane.right() - slot.right() - 16.0).abs() <= 1.0,
            "{width}px: the slot ends {} from the edge, not 16",
            pane.right() - slot.right()
        );

        // The messages: the same column, less the list's scrollbar.
        let rows = root.query_selector_all(".messages .row").unwrap();
        assert_eq!(rows.length(), 2);
        for index in 0..rows.length() {
            let row: Element = rows.item(index).unwrap().dyn_into().unwrap();
            let drawn = row.get_bounding_client_rect();
            assert!(
                (drawn.left() - pane.left() - side).abs() <= 1.0,
                "{width}px: row {index} starts at {}",
                drawn.left() - pane.left()
            );
            assert!(
                drawn.width() <= column + 0.5 && drawn.width() >= column - 20.0,
                "{width}px: row {index} is {} wide in a {column} column",
                drawn.width()
            );
            let bubble = query(&row, ".bubble").get_bounding_client_rect();
            let most = (drawn.width() - 48.0).min(560.0);
            assert!(
                (bubble.width() - most).abs() <= 1.0,
                "{width}px: bubble {index} is {} wide, not min(column − 48, 560) = {most}",
                bubble.width()
            );
            // Mine at the column's trailing edge, theirs at its leading one.
            if index == 1 {
                assert!(
                    (bubble.right() - drawn.right()).abs() <= 1.0,
                    "{width}px: mine"
                );
            } else {
                assert!(
                    (bubble.left() - drawn.left()).abs() <= 1.0,
                    "{width}px: theirs"
                );
            }
        }
        handle.destroy();
        root.remove();
    }
}

/// THE JUMP BUTTON COVERS NOTHING (#106): it stood over the end of the last
/// bubble — mine end there. Now it has a strip of its own under the list.
#[wasm_bindgen_test]
async fn the_jump_button_never_covers_a_bubble() {
    let root = pane();
    let (_, on_action) = recorder();
    let handle =
        yew::Renderer::<Conversation>::with_root_and_props(root.clone().into(), props(40)).render();
    TimeoutFuture::new(60).await;
    let messages = query(&root, ".messages");
    assert!(root.query_selector(".jump-newest").unwrap().is_none());
    scroll_to(&messages, messages.scroll_top() - 120);
    TimeoutFuture::new(40).await;
    let jump = rect_of(&root, ".jump-newest");
    let list = messages.get_bounding_client_rect();
    assert!(
        jump.top() >= list.bottom() - 0.5,
        "under the list ({} vs {}), not over its last bubble",
        jump.top(),
        list.bottom()
    );
    drop(on_action);
    handle.destroy();
    root.remove();
}

/// The way back to the list, in the conversation's own header: it closes
/// the chat — nothing is read while the list is in front.
#[wasm_bindgen_test]
async fn the_way_back_to_the_list_closes_the_chat() {
    let root = pane();
    let (log, on_action) = recorder();
    let handle = yew::Renderer::<Conversation>::with_root_and_props(
        root.clone().into(),
        props_with(vec![message(1)], None, on_action),
    )
    .render();
    TimeoutFuture::new(50).await;
    let back = query(&root, ".conversation-bar .back-to-chats");
    assert_eq!(visible_text(&back).trim(), "Chats");
    back.dyn_into_html().click();
    TimeoutFuture::new(20).await;
    assert!(log.borrow().contains(&Action::CloseChat));
    handle.destroy();
    root.remove();
}

/// "ASK THE ASSISTANT" IN THE PAPERCLIP'S MENU (#106): the first line of its
/// last group in the family chat — where the row's `@ai` button was — and
/// it puts `@ai ` into the draft. Not in a direct chat, nor where the server
/// names nobody.
#[wasm_bindgen_test]
async fn the_paperclip_asks_the_assistant_in_the_family_chat() {
    let mount = |kind: &'static str, processor: Option<&'static str>| async move {
        let root = pane();
        let (_, on_action) = recorder();
        let mut props = props_with(vec![message(1)], None, on_action);
        props.item.chat.kind = kind.into();
        props.assistant = Some(assistant_answered_by(processor));
        let handle =
            yew::Renderer::<Conversation>::with_root_and_props(root.clone().into(), props).render();
        TimeoutFuture::new(50).await;
        query(&root, ".composer .attach > .tool")
            .dyn_into_html()
            .click();
        TimeoutFuture::new(30).await;
        (root, handle)
    };
    let (root, handle) = mount("family", Some("Microsoft — Azure OpenAI")).await;
    let children = root
        .query_selector_all(".attach-menu > hr, .attach-menu > [role=menuitem]")
        .unwrap();
    let mut last_group = Vec::new();
    for index in 0..children.length() {
        let child: Element = children.item(index).unwrap().dyn_into().unwrap();
        if child.tag_name() == "HR" {
            last_group.clear();
        } else if child.get_attribute("role").as_deref() == Some("menuitem") {
            last_group.push(visible_text(&child).trim().to_string());
        }
    }
    assert_eq!(last_group, ["Ask the Assistant", "Location", "Poll"]);
    click_labelled(&root, ".attach-menu [role=menuitem]", "Ask the Assistant");
    TimeoutFuture::new(30).await;
    let area: HtmlTextAreaElement = query(&root, "textarea").dyn_into().unwrap();
    assert_eq!(area.value(), "@ai ");
    assert!(
        root.query_selector(".attach-menu").unwrap().is_none(),
        "closed"
    );
    handle.destroy();
    root.remove();

    for (kind, processor) in [
        ("direct", Some("Microsoft — Azure OpenAI")),
        ("family", None),
    ] {
        let (root, handle) = mount(kind, processor).await;
        let lines = root
            .query_selector_all(".attach-menu [role=menuitem]")
            .unwrap();
        let said: Vec<String> = (0..lines.length())
            .map(|index| {
                visible_text(&lines.item(index).unwrap().dyn_into::<Element>().unwrap())
                    .trim()
                    .to_string()
            })
            .collect();
        assert!(
            !said.iter().any(|line| line == "Ask the Assistant"),
            "{kind}, {processor:?}: {said:?}"
        );
        handle.destroy();
        root.remove();
    }
}

/// ESC IN A THREAD NEVER LOSES A DRAFT (#106, 2.3): with words in the box
/// the first Esc leaves the box, onto the panel, and the panel stays; a
/// second closes it. With the box empty, one closes it, as it always did.
#[wasm_bindgen_test]
async fn esc_in_a_thread_keeps_its_draft() {
    use crate::views::thread_panel::{ThreadPanel, ThreadPanelProps};
    let escape = |at: &Element| {
        let init = web_sys::KeyboardEventInit::new();
        init.set_key("Escape");
        init.set_bubbles(true);
        init.set_cancelable(true);
        at.dispatch_event(
            &web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap(),
        )
        .unwrap();
    };
    let closed = |log: &Rc<RefCell<Vec<Action>>>| {
        log.borrow()
            .iter()
            .filter(|action| **action == Action::CloseThread)
            .count()
    };
    let root = pane();
    let (log, on_action) = recorder();
    let props = ThreadPanelProps {
        chat_id: 42,
        root_id: 1,
        messages: vec![message(1)].into(),
        my_user_id: 7,
        chat_kind: "family".into(),
        names: Default::default(),
        members: Vec::new(),
        chat_members: Vec::new(),
        assistant: None,
        blocked: Default::default(),
        revealed: Default::default(),
        revealed_quotes: Default::default(),
        failed: Default::default(),
        ai_failed: Default::default(),
        family: None,
        stickers: None,
        agreed_to_assistant: true,
        transcripts: Default::default(),
        on_action,
    };
    let handle =
        yew::Renderer::<ThreadPanel>::with_root_and_props(root.clone().into(), props).render();
    TimeoutFuture::new(50).await;
    let area = query(&root, ".thread-panel textarea");
    area.clone().dyn_into_html().focus().unwrap();
    type_into(&area, "on my way");
    TimeoutFuture::new(20).await;
    escape(&area);
    TimeoutFuture::new(20).await;
    assert_eq!(closed(&log), 0, "a draft keeps the panel open");
    let panel = query(&root, ".thread-panel");
    assert_eq!(
        document().active_element().as_ref(),
        Some(&panel),
        "the box is left, onto the panel"
    );
    assert_eq!(
        area.dyn_ref::<HtmlTextAreaElement>().unwrap().value(),
        "on my way",
        "its words kept"
    );
    escape(&panel);
    TimeoutFuture::new(20).await;
    assert_eq!(closed(&log), 1, "a second Esc closes it");

    type_into(&area, "");
    TimeoutFuture::new(20).await;
    escape(&area);
    TimeoutFuture::new(20).await;
    assert_eq!(closed(&log), 2, "an empty box: one Esc");
    handle.destroy();
    root.remove();
}

/// The shipped stylesheet in a frame `width` wide — a WINDOW that wide, so
/// its media queries are the ones a phone or a tablet meets — with `body`.
async fn framed_window(width: u32, body: &str) -> (web_sys::HtmlIFrameElement, Document) {
    let frame: web_sys::HtmlIFrameElement = document()
        .create_element("iframe")
        .unwrap()
        .dyn_into()
        .unwrap();
    frame
        .set_attribute(
            "style",
            &format!("position:fixed;top:0;left:0;width:{width}px;height:640px;border:0"),
        )
        .unwrap();
    let html = format!(
        "<!doctype html><html><head><style>{}</style></head><body>{body}\
         <i id=\"drawn\"></i></body></html>",
        include_str!("../styles.css")
    );
    frame.set_attribute("srcdoc", &html).unwrap();
    document().body().unwrap().append_child(&frame).unwrap();
    assert!(
        crate::round_record_tests::until(3_000, || frame
            .content_document()
            .and_then(|inner| inner.get_element_by_id("drawn"))
            .is_some())
        .await,
        "the frame drew"
    );
    TimeoutFuture::new(30).await;
    let inner = frame.content_document().unwrap();
    (frame, inner)
}

/// The signed-in shell as main.rs draws it, with `split`'s classes and what
/// stands beside the list.
fn split_shell(classes: &str, pane: &str, panel: &str) -> String {
    let rows: String = (0..30)
        .map(|i| {
            format!(r#"<button class="chat-row"><span class="chat-title">Chat {i}</span></button>"#)
        })
        .collect();
    format!(
        r#"<div class="app"><header class="bar"><span class="brand">Family Connect</span></header>
           <div class="split {classes}"><nav class="chat-list">{rows}</nav>{pane}{panel}</div></div>"#
    )
}

/// A conversation's markup, as views/conversation.rs draws it.
const CHAT_PANE: &str = r#"<section class="conversation">
    <header class="conversation-bar"><button class="back-to-chats">‹ Chats</button>
      <h2 class="conversation-title">The Smiths</h2></header>
    <div class="messages"><div class="row"><article class="bubble"><p class="body">Hello</p></article></div></div>
    <div class="composer-wrap"><div class="composer">
      <div class="attach"><button class="tool">clip</button></div>
      <div class="field"><textarea rows="1"></textarea></div>
      <div class="slot-wrap"><button class="slot">Send</button></div>
    </div></div></section>"#;

const THREAD_PANEL: &str = r#"<aside class="thread-panel"><header class="thread-bar"><h2>2 replies</h2></header>
    <div class="thread-messages"></div></aside>"#;

fn shown(document: &Document, selector: &str) -> bool {
    document
        .query_selector(selector)
        .unwrap()
        .is_some_and(|element| element.get_bounding_client_rect().width() > 0.0)
}

fn frame_rect(document: &Document, selector: &str) -> web_sys::DomRect {
    document
        .query_selector(selector)
        .unwrap()
        .unwrap_or_else(|| panic!("{selector} is in the frame"))
        .get_bounding_client_rect()
}

/// ONE SURFACE AT A TIME ON A PHONE (#106, section 3): the list, or the
/// conversation it opened — the whole window, with the way back in its
/// header — never the list as a band over a chat that showed three
/// bubbles. With nothing open the list is the whole window. Beside the
/// list, where there is room, the way back is not drawn.
#[wasm_bindgen_test]
async fn a_phone_shows_the_list_or_the_chat_never_both() {
    let (frame, inner) = framed_window(390, &split_shell("is-reading", CHAT_PANE, "")).await;
    assert!(!shown(&inner, ".chat-list"), "reading: no list");
    let chat = frame_rect(&inner, ".conversation");
    assert!(
        (chat.width() - 390.0).abs() <= 1.0,
        "the whole width: {}",
        chat.width()
    );
    assert!(shown(&inner, ".back-to-chats"), "and the way back");
    let back = frame_rect(&inner, ".back-to-chats");
    assert!(back.height() >= 43.5, "a 44-px target: {}", back.height());
    assert!(frame_rect(&inner, ".composer").bottom() <= 640.5);
    frame.remove();

    let empty =
        r#"<section class="conversation empty"><p>Pick a chat from the sidebar.</p></section>"#;
    let (frame, inner) = framed_window(390, &split_shell("is-listing", empty, "")).await;
    assert!(!shown(&inner, ".conversation"), "listing: no empty pane");
    let list = frame_rect(&inner, ".chat-list");
    let split = frame_rect(&inner, ".split");
    assert!(
        (list.height() - split.height()).abs() <= 1.0 && (list.width() - 390.0).abs() <= 1.0,
        "the list is the whole window: {}×{}",
        list.width(),
        list.height()
    );
    frame.remove();

    let (frame, inner) = framed_window(1024, &split_shell("is-reading", CHAT_PANE, "")).await;
    assert!(shown(&inner, ".chat-list"), "two panes");
    assert!(
        !shown(&inner, ".back-to-chats"),
        "no way back where the list is beside"
    );
    assert!(
        frame_rect(&inner, ".chat-list").right()
            <= frame_rect(&inner, ".conversation").left() + 0.5
    );
    frame.remove();
}

/// AN UPRIGHT IPAD (834 px, #106, section 3): a thread or the open polls
/// open OVER the conversation, from its trailing edge — the conversation
/// keeps its width and its whole composer under it, where beside it the
/// chat was squeezed to ~154 px. Wider, the panel stands beside the chat;
/// on a phone it takes the chat's place.
#[wasm_bindgen_test]
async fn an_upright_ipad_opens_a_panel_over_the_chat() {
    let (frame, inner) = framed_window(
        834,
        &split_shell("with-panel is-reading", CHAT_PANE, THREAD_PANEL),
    )
    .await;
    let list = frame_rect(&inner, ".chat-list");
    let chat = frame_rect(&inner, ".conversation");
    let panel = frame_rect(&inner, ".thread-panel");
    let composer = frame_rect(&inner, ".composer-wrap");
    assert!(
        (chat.width() - (834.0 - list.width())).abs() <= 1.0 && chat.width() >= 500.0,
        "the chat keeps its width: {}",
        chat.width()
    );
    assert!(
        (composer.width() - chat.width()).abs() <= 1.0,
        "and its whole composer"
    );
    assert!(
        (panel.right() - 834.0).abs() <= 1.0 && panel.left() >= chat.left() - 0.5,
        "the panel is over the chat: {}..{}",
        panel.left(),
        panel.right()
    );
    assert!(
        (280.0..=380.5).contains(&panel.width()),
        "as wide as beside: {}",
        panel.width()
    );
    let on_top = inner.element_from_point(
        (panel.left() + panel.width() / 2.0) as f32,
        (panel.top() + 10.0) as f32,
    );
    let aside = inner.query_selector(".thread-panel").unwrap().unwrap();
    assert!(
        on_top.is_some_and(|hit| aside.contains(Some(&hit))),
        "and on top of it"
    );
    frame.remove();

    let (frame, inner) = framed_window(
        1280,
        &split_shell("with-panel is-reading", CHAT_PANE, THREAD_PANEL),
    )
    .await;
    assert!(
        frame_rect(&inner, ".conversation").right()
            <= frame_rect(&inner, ".thread-panel").left() + 0.5,
        "a desk: beside"
    );
    frame.remove();

    let (frame, inner) = framed_window(
        390,
        &split_shell("with-panel is-reading", CHAT_PANE, THREAD_PANEL),
    )
    .await;
    assert!(!shown(&inner, ".conversation") && !shown(&inner, ".chat-list"));
    assert!(
        (frame_rect(&inner, ".thread-panel").width() - 390.0).abs() <= 1.0,
        "a phone: alone"
    );
    frame.remove();
}

/// A GROUP'S CONVERSATION (docs/protocol.md, "Groups"): its title opens
/// Group Info and says how many are in it; it has open polls as the family
/// chat does and never a call, even on a server that carries calls; and at
/// a phone's width the bar and the dialog stay on the screen.
#[wasm_bindgen_test]
async fn a_groups_conversation_has_its_info_and_no_calls() {
    let root = pane_of(390.0);
    let (_, on_action) = recorder();
    let mut props = props_with(vec![message(1), message(2)], None, on_action);
    props.calls_enabled = true;
    props.item.chat = Chat {
        id: 42,
        kind: "group".into(),
        title: Some("The cousins who meet on Sundays at grandmother's house".into()),
        member_ids: Some(vec![7, 9]),
        admin_user_id: Some(7),
        ..Chat::default()
    };
    props.members = vec![
        Member {
            id: 7,
            display_name: "Me".into(),
            ..Member::default()
        },
        Member {
            id: 9,
            display_name: "Anna".into(),
            ..Member::default()
        },
        Member {
            id: 11,
            display_name: "Bob".into(),
            ..Member::default()
        },
    ];
    let handle =
        yew::Renderer::<Conversation>::with_root_and_props(root.clone().into(), props).render();
    TimeoutFuture::new(50).await;
    assert!(
        root.query_selector(".call-buttons").unwrap().is_none(),
        "no calls"
    );
    assert!(
        root.query_selector(".open-polls").unwrap().is_some(),
        "polls"
    );
    let title = query(&root, ".conversation-title .title-button");
    assert!(title
        .text_content()
        .unwrap_or_default()
        .contains("Members: 2"));
    // Sender names, as in the family chat: Anna's message says who.
    assert!(
        root.query_selector(".sender").unwrap().is_some(),
        "a group names its senders"
    );
    // The bar stays inside the phone: the long name gives way.
    let bar = query(&root, ".conversation-bar").get_bounding_client_rect();
    let polls = query(&root, ".open-polls").get_bounding_client_rect();
    assert!(
        bar.right() <= 390.5 && polls.right() <= bar.right() + 0.5,
        "{} {}",
        bar.right(),
        polls.right()
    );

    title.dyn_into_html().click();
    TimeoutFuture::new(50).await;
    let dialog = query(&root, ".dialog.group-dialog");
    assert!(visible_text(&dialog).contains("Group Info"));
    let members = dialog
        .query_selector_all(".group-members .member-row")
        .unwrap();
    assert_eq!(members.length(), 2, "the group's members, not the family's");
    handle.destroy();
    root.remove();

    // The dialog's own markup at a phone's width and a desk's: inside the
    // window, nothing wider than it.
    for width in [390, 1024] {
        let markup = r#"<div class="dialog-backdrop"><div class="dialog group-dialog" role="dialog">
               <h2>Group Info</h2><p class="group-name"><strong>The cousins who meet on Sundays at grandmother's house</strong></p>
               <h3>Members: 2</h3><ul class="members group-members">
               <li class="member-row"><span class="avatar" style="width:28px;height:28px"></span><span class="identity-text"><span><strong>Alexandrina Wilhelmina Featherstonehaugh</strong><span class="capsule">Admin</span></span></span></li>
               <li class="member-row"><span class="avatar" style="width:28px;height:28px"></span><span class="identity-text"><span><strong>Anna</strong></span></span><button class="link group-remove">Remove from Group</button></li>
               </ul><div class="group-tools"><button class="link">Rename Group</button><button class="link">Add Members</button><button class="link danger">Leave Group</button><button class="link danger">Delete Group</button></div>
               <div class="dialog-actions"><button class="primary">Done</button></div></div></div>"#;
        let (frame, inner) = framed_window(width, markup).await;
        let dialog = inner
            .query_selector(".dialog")
            .unwrap()
            .unwrap()
            .get_bounding_client_rect();
        assert!(
            dialog.left() >= 0.0 && dialog.right() <= f64::from(width),
            "{width}: {} {}",
            dialog.left(),
            dialog.right()
        );
        let page = inner.document_element().unwrap();
        assert!(
            page.scroll_width() <= page.client_width(),
            "{width}: no sideways scroll"
        );
        let remove = inner
            .query_selector(".group-remove")
            .unwrap()
            .unwrap()
            .get_bounding_client_rect();
        assert!(
            remove.right() <= dialog.right(),
            "{width}: Remove is inside the dialog"
        );
        frame.remove();
    }
}
