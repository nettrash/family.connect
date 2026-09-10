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

use gloo_timers::future::TimeoutFuture;
use wasm_bindgen_test::*;
use web_sys::{Document, Element, HtmlElement};
use yew::Callback;

use crate::model::Message;
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
                r#"<article class="bubble{mine}"><span class="sender">Anna</span>
                   <p class="body">Message number {i}, long enough to take a line.</p>
                   <span class="meta">17:0{d}</span></article>"#,
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
                 <div class="messages">{bubbles}</div>
                 <div class="composer"><textarea rows="2"></textarea><button>Send</button></div>
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
        client_msg_id: None,
        body: format!("Message number {id}, long enough to take a line of its own."),
        created_at: "2026-08-19T17:03:12Z".into(),
        edited_at: None,
    }
}

fn props(count: i64) -> ConversationProps {
    ConversationProps {
        messages: (1..=count).map(message).collect(),
        my_user_id: 7,
        names: Default::default(),
        typing: Vec::new(),
        sending_disabled: false,
        on_send: Callback::noop(),
        on_typing: Callback::noop(),
        can_load_more: false,
        on_load_more: Callback::noop(),
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
