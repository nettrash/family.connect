//! One chat: its messages, and the box to add to them.

use wasm_bindgen::JsCast;
use web_sys::HtmlTextAreaElement;
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

#[function_component(Conversation)]
pub fn conversation(props: &ConversationProps) -> Html {
    let draft = use_state(String::new);

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
        Callback::from(move |_: ()| {
            let body = draft.trim().to_string();
            if body.is_empty() {
                return;
            }
            draft.set(String::new());
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
            <div class="messages">
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

/// Keep the scroll at the newest message.
///
/// Called after the list changes. It is a no-op when the element is not
/// there yet, which is the first render.
pub fn scroll_to_newest() {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let Some(element) = document.query_selector(".messages").ok().flatten() else {
        return;
    };
    if let Some(element) = element.dyn_ref::<web_sys::Element>() {
        element.set_scroll_top(element.scroll_height());
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
