//! One chat: its messages, and the box to add to them.

use std::collections::HashMap;

use web_sys::{Element, HtmlTextAreaElement};
use yew::prelude::*;

use crate::model::Message;

#[derive(Properties, PartialEq)]
pub struct ConversationProps {
    pub messages: Vec<Message>,
    pub my_user_id: i64,
    /// userId → the name to draw. A sender who is not in the roster reads
    /// as "Someone" rather than as a number.
    pub names: HashMap<i64, String>,
    /// Who is typing here, already resolved to names.
    pub typing: Vec<String>,
    pub sending_disabled: bool,
    pub on_send: Callback<String>,
    /// Somebody is typing here. Emitted on every keystroke; the THROTTLE is
    /// the caller's, because it is the caller that holds the clock and the
    /// socket (docs/protocol.md, `typing`).
    pub on_typing: Callback<()>,
    /// There is older history to fetch; the caller pages it in.
    pub can_load_more: bool,
    pub on_load_more: Callback<()>,
    /// This device's messages that will not be sent unless somebody asks,
    /// by `client_msg_id`, with why. A pending bubble (id 0) that is NOT in
    /// here is still being sent — which is all an unknown outcome is.
    pub failed: HashMap<String, String>,
    /// Try a failed message again, by `client_msg_id`.
    pub on_retry: Callback<String>,
    /// Give up on a failed message, by `client_msg_id`.
    pub on_discard: Callback<String>,
}

/// How close to the bottom still counts as "reading the newest", in
/// pixels. A reader a line or two up is still following along; one who has
/// scrolled a screen away is reading history and must not be yanked down.
const PINNED_SLACK_PX: i32 = 48;

#[function_component(Conversation)]
pub fn conversation(props: &ConversationProps) -> Html {
    let draft = use_state(String::new);
    let list = use_node_ref();
    // Whether the reader is at the newest message. Starts true, so opening
    // a chat lands on its newest message; the scroll handler keeps it
    // honest after that.
    let pinned = use_mut_ref(|| true);

    // FOLLOW THE CONVERSATION — after the DOM has the new message, and only
    // for a reader who was already at the bottom. Two rules the phone
    // clients follow too: a message landing must not scroll somebody who is
    // up the thread reading history, and it must not leave somebody who is
    // following along one message short of the newest.
    //
    // An effect rather than a call next to the state change, because a
    // call there runs BEFORE the re-render: it measured the old list and
    // stopped one message short every time.
    {
        let list = list.clone();
        let pinned = pinned.clone();
        let newest = props
            .messages
            .last()
            .map(|message| (message.id, message.client_msg_id.clone()));
        use_effect_with((props.messages.len(), newest), move |_| {
            if *pinned.borrow() {
                if let Some(element) = list.cast::<Element>() {
                    element.set_scroll_top(element.scroll_height());
                }
            }
        });
    }

    let on_scroll = {
        let list = list.clone();
        let pinned = pinned.clone();
        Callback::from(move |_: Event| {
            if let Some(element) = list.cast::<Element>() {
                *pinned.borrow_mut() = element.scroll_top() + element.client_height()
                    >= element.scroll_height() - PINNED_SLACK_PX;
            }
        })
    };

    let on_input = {
        let draft = draft.clone();
        let on_typing = props.on_typing.clone();
        Callback::from(move |event: InputEvent| {
            let input: HtmlTextAreaElement = event.target_unchecked_into();
            let value = input.value();
            // Only when there is something to be typing. Clearing the box
            // is not typing, and a frame for it would tell the family
            // somebody is writing when they have just given up.
            if !value.trim().is_empty() {
                on_typing.emit(());
            }
            draft.set(value);
        })
    };

    let send = {
        let draft = draft.clone();
        let on_send = props.on_send.clone();
        let pinned = pinned.clone();
        Callback::from(move |_: ()| {
            let body = draft.trim().to_string();
            if body.is_empty() {
                return;
            }
            draft.set(String::new());
            // Your own message is always shown, wherever you were: you
            // just wrote it, and a send that disappeared below the fold
            // reads as a send that did not happen.
            *pinned.borrow_mut() = true;
            on_send.emit(body);
        })
    };

    // Enter sends, Shift+Enter makes a line — the arrangement every chat in
    // this product uses, and the one a person's hands already know.
    let on_key = {
        let send = send.clone();
        Callback::from(move |event: KeyboardEvent| {
            if event.key() == "Enter" && !event.shift_key() {
                event.prevent_default();
                send.emit(());
            }
        })
    };

    let load_more = {
        let on_load_more = props.on_load_more.clone();
        Callback::from(move |_| on_load_more.emit(()))
    };

    html! {
        <section class="conversation">
            <div class="messages" ref={list} onscroll={on_scroll}>
                if props.can_load_more {
                    <button class="load-more" onclick={load_more}>{ "Earlier messages" }</button>
                }
                { for props.messages.iter().map(|message| {
                    let mine = message.sender_id == props.my_user_id;
                    let name = props
                        .names
                        .get(&message.sender_id)
                        .cloned()
                        .unwrap_or_else(|| "Someone".to_string());
                    let pending = message.id == 0;
                    let client_msg_id = message.client_msg_id.clone().unwrap_or_default();
                    let failed = props.failed.get(&client_msg_id).cloned();
                    let retry = {
                        let on_retry = props.on_retry.clone();
                        let client_msg_id = client_msg_id.clone();
                        Callback::from(move |_: MouseEvent| on_retry.emit(client_msg_id.clone()))
                    };
                    let discard = {
                        let on_discard = props.on_discard.clone();
                        Callback::from(move |_: MouseEvent| on_discard.emit(client_msg_id.clone()))
                    };
                    html! {
                        <article class={classes!(
                            "bubble",
                            mine.then_some("is-mine"),
                            pending.then_some("is-pending"),
                            failed.is_some().then_some("is-failed"),
                        )}>
                            if !mine {
                                <span class="sender">{ name }</span>
                            }
                            <p class="body">{ &message.body }</p>
                            // A bubble that is not delivered SAYS so. The
                            // worst failure this product has is a message
                            // the sender believes they sent.
                            if let Some(reason) = failed {
                                <span class="meta send-failed" role="alert">
                                    { reason }
                                    <button class="link" onclick={retry}>{ "Retry" }</button>
                                    <button class="link" onclick={discard}>{ "Discard" }</button>
                                </span>
                            } else if pending {
                                <span class="meta sending">{ "Sending…" }</span>
                            } else {
                                <span class="meta">
                                    { message.clock() }
                                    if message.is_edited() {
                                        <span class="edited">{ " · edited" }</span>
                                    }
                                </span>
                            }
                        </article>
                    }
                }) }
            </div>
            if !props.typing.is_empty() {
                <p class="typing" aria-live="polite">{ typing_line(&props.typing) }</p>
            }
            <div class="composer">
                <textarea
                    aria-label="Message"
                    rows="2"
                    value={(*draft).clone()}
                    oninput={on_input}
                    onkeydown={on_key}
                />
                <button
                    onclick={let send = send.clone(); Callback::from(move |_: MouseEvent| send.emit(()))}
                    disabled={props.sending_disabled || draft.trim().is_empty()}
                >
                    { "Send" }
                </button>
            </div>
        </section>
    }
}

/// "Anna is typing…", "Anna and Bob are typing…", "Several people are
/// typing…" — the third because a list of five names is a line nobody
/// reads, and the count is the part that matters.
pub fn typing_line(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => format!("{one} is typing…"),
        [one, two] => format!("{one} and {two} are typing…"),
        _ => "Several people are typing…".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;
    use web_sys::HtmlElement;

    fn mine(id: i64, client_msg_id: &str, body: &str) -> Message {
        Message {
            id,
            chat_id: 42,
            sender_id: 7,
            client_msg_id: Some(client_msg_id.into()),
            body: body.into(),
            created_at: if id == 0 {
                String::new()
            } else {
                "2026-09-10T10:00:00Z".into()
            },
            edited_at: None,
        }
    }

    /// Delivered, still being sent, and not sent: three bubbles that must
    /// not look alike — and the one that failed can be tried again.
    #[wasm_bindgen_test]
    async fn a_bubble_says_whether_it_was_delivered() {
        let document = web_sys::window().unwrap().document().unwrap();
        let root = document.create_element("div").unwrap();
        document.body().unwrap().append_child(&root).unwrap();

        let retried = Rc::new(RefCell::new(Vec::<String>::new()));
        let discarded = Rc::new(RefCell::new(Vec::<String>::new()));
        let props = ConversationProps {
            messages: vec![
                mine(101, "a", "delivered"),
                mine(0, "b", "on its way"),
                mine(0, "c", "refused"),
            ],
            my_user_id: 7,
            names: HashMap::new(),
            typing: Vec::new(),
            sending_disabled: false,
            on_send: Callback::noop(),
            on_typing: Callback::noop(),
            can_load_more: false,
            on_load_more: Callback::noop(),
            failed: HashMap::from([("c".to_string(), "Not sent: blocked.".to_string())]),
            on_retry: {
                let retried = retried.clone();
                Callback::from(move |id: String| retried.borrow_mut().push(id))
            },
            on_discard: {
                let discarded = discarded.clone();
                Callback::from(move |id: String| discarded.borrow_mut().push(id))
            },
        };
        let handle =
            yew::Renderer::<Conversation>::with_root_and_props(root.clone(), props).render();
        gloo_timers::future::TimeoutFuture::new(20).await;

        let bubbles = root.query_selector_all(".bubble").unwrap();
        let text = |index: u32| {
            bubbles
                .item(index)
                .unwrap()
                .text_content()
                .unwrap_or_default()
        };
        assert!(
            text(0).contains("10:00"),
            "a delivered bubble shows its time: {}",
            text(0)
        );
        assert!(!text(0).contains("Sending"));
        assert!(text(1).contains("Sending…"), "{}", text(1));
        assert!(text(2).contains("Not sent: blocked."), "{}", text(2));
        assert!(!text(2).contains("Sending"), "failed is not pending");
        assert_eq!(
            root.query_selector_all(".bubble.is-failed")
                .unwrap()
                .length(),
            1
        );

        let buttons = bubbles
            .item(2)
            .unwrap()
            .dyn_into::<Element>()
            .unwrap()
            .query_selector_all("button")
            .unwrap();
        buttons
            .item(0)
            .unwrap()
            .dyn_into::<HtmlElement>()
            .unwrap()
            .click();
        buttons
            .item(1)
            .unwrap()
            .dyn_into::<HtmlElement>()
            .unwrap()
            .click();
        assert_eq!(*retried.borrow(), vec!["c".to_string()]);
        assert_eq!(*discarded.borrow(), vec!["c".to_string()]);

        handle.destroy();
        root.remove();
    }

    #[wasm_bindgen_test]
    fn the_typing_line_names_one_or_two_and_counts_the_rest() {
        assert_eq!(typing_line(&[]), "");
        assert_eq!(typing_line(&["Anna".to_string()]), "Anna is typing…");
        assert_eq!(
            typing_line(&["Anna".to_string(), "Bob".to_string()]),
            "Anna and Bob are typing…"
        );
        assert_eq!(
            typing_line(&["Anna".to_string(), "Bob".to_string(), "Gran".to_string()]),
            "Several people are typing…"
        );
    }
}
