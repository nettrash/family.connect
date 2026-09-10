//! One chat: its messages, and the box to add to them.

use web_sys::{Element, HtmlTextAreaElement};
use yew::prelude::*;

use crate::model::Message;

#[derive(Properties, PartialEq)]
pub struct ConversationProps {
    pub messages: Vec<Message>,
    pub my_user_id: i64,
    /// userId → the name to draw. A sender who is not in the roster reads
    /// as "Someone" rather than as a number.
    pub names: std::collections::HashMap<i64, String>,
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
                    html! {
                        <article class={classes!("bubble", mine.then_some("is-mine"))}>
                            if !mine {
                                <span class="sender">{ name }</span>
                            }
                            <p class="body">{ &message.body }</p>
                            <span class="meta">
                                { message.clock() }
                                if message.is_edited() {
                                    <span class="edited">{ " · edited" }</span>
                                }
                            </span>
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
    use wasm_bindgen_test::*;

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
