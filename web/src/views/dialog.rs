//! A modal dialog, and the confirmation built on it.
//!
//! Focus goes INTO the dialog as it opens — its first field, or the dialog
//! itself when it has none, so a confirmation's destructive button is never
//! where a stray Return lands — and back to whatever had it as it closes.
//! Escape cancels — except while what it asked for is on its way: a
//! request already sent is not taken back by closing its dialog, and a
//! dialog closed over one would leave its answer nowhere to land (a reset
//! password nobody was told of). Tab goes round the dialog rather than out
//! of it into the page behind, which a person using the keyboard could not
//! see.

use std::cell::Cell;

use wasm_bindgen::JsCast;
use web_sys::HtmlElement;
use yew::prelude::*;

use crate::api::ApiError;

thread_local! {
    static NEXT_ID: Cell<u64> = const { Cell::new(0) };
}

const FOCUSABLE: &str =
    "a[href], button:not([disabled]), input:not([disabled]):not([type=hidden]), \
     select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex='-1'])";

const FIELDS: &str = "input:not([disabled]):not([type=hidden]):not([type=file]), \
     select:not([disabled]), textarea:not([disabled])";

#[derive(Properties, PartialEq)]
pub struct ModalProps {
    pub title: AttrValue,
    #[prop_or_default]
    pub class: Classes,
    pub on_cancel: Callback<()>,
    /// Something it asked for is on its way: Escape does nothing.
    #[prop_or_default]
    pub busy: bool,
    /// The id of what says what the dialog is about, read with its title.
    #[prop_or_default]
    pub describedby: Option<AttrValue>,
    #[prop_or_default]
    pub children: Html,
}

/// An id no other dialog on the page has.
pub fn fresh_id(prefix: &str) -> String {
    NEXT_ID.with(|next| {
        next.set(next.get() + 1);
        format!("{prefix}-{}", next.get())
    })
}

#[function_component(Modal)]
pub fn modal(props: &ModalProps) -> Html {
    let dialog = use_node_ref();
    let title_id = use_memo((), |_| fresh_id("dialog-title"));
    {
        let dialog = dialog.clone();
        use_effect_with((), move |_| {
            let previous = web_sys::window()
                .and_then(|window| window.document())
                .and_then(|document| document.active_element())
                .and_then(|element| element.dyn_into::<HtmlElement>().ok());
            if let Some(dialog) = dialog.cast::<HtmlElement>() {
                let field = dialog
                    .query_selector(FIELDS)
                    .ok()
                    .flatten()
                    .and_then(|element| element.dyn_into::<HtmlElement>().ok());
                let _ = field.unwrap_or(dialog).focus();
            }
            move || {
                if let Some(previous) = previous.filter(|element| element.is_connected()) {
                    let _ = previous.focus();
                }
            }
        });
    }
    let on_key = {
        let dialog = dialog.clone();
        let on_cancel = props.on_cancel.clone();
        let busy = props.busy;
        Callback::from(move |event: KeyboardEvent| match event.key().as_str() {
            "Escape" => {
                event.prevent_default();
                event.stop_propagation();
                if !busy {
                    on_cancel.emit(());
                }
            }
            "Tab" => {
                let Some(dialog) = dialog.cast::<HtmlElement>() else {
                    return;
                };
                let Ok(list) = dialog.query_selector_all(FOCUSABLE) else {
                    return;
                };
                let items: Vec<HtmlElement> = (0..list.length())
                    .filter_map(|index| list.item(index))
                    .filter_map(|node| node.dyn_into::<HtmlElement>().ok())
                    .collect();
                let (Some(first), Some(last)) = (items.first(), items.last()) else {
                    event.prevent_default();
                    return;
                };
                let active = web_sys::window()
                    .and_then(|window| window.document())
                    .and_then(|document| document.active_element());
                let on = |element: &HtmlElement| {
                    active
                        .as_ref()
                        .is_some_and(|active| active == element.unchecked_ref::<web_sys::Element>())
                };
                let on_dialog = active
                    .as_ref()
                    .is_some_and(|active| active == dialog.unchecked_ref::<web_sys::Element>());
                if event.shift_key() && (on(first) || on_dialog) {
                    event.prevent_default();
                    let _ = last.focus();
                } else if !event.shift_key() && on(last) {
                    event.prevent_default();
                    let _ = first.focus();
                }
            }
            _ => {}
        })
    };
    html! {
        <div class="dialog-backdrop" onkeydown={on_key}>
            <div
                class={classes!("dialog", props.class.clone())}
                role="dialog"
                aria-modal="true"
                aria-labelledby={(*title_id).clone()}
                aria-describedby={props.describedby.clone()}
                tabindex="-1"
                ref={dialog}
            >
                <h2 id={(*title_id).clone()}>{ props.title.clone() }</h2>
                { props.children.clone() }
            </div>
        </div>
    }
}

#[derive(Properties, PartialEq)]
pub struct ConfirmProps {
    pub title: AttrValue,
    /// What doing it means, when the title alone does not say.
    #[prop_or_default]
    pub message: AttrValue,
    /// The button that does it — named for what it does, never "OK".
    pub confirm: AttrValue,
    #[prop_or(true)]
    pub destructive: bool,
    #[prop_or_default]
    pub busy: bool,
    #[prop_or_default]
    pub error: Option<String>,
    pub on_confirm: Callback<()>,
    pub on_cancel: Callback<()>,
}

#[function_component(Confirm)]
pub fn confirm(props: &ConfirmProps) -> Html {
    let go = {
        let on_confirm = props.on_confirm.clone();
        Callback::from(move |_: MouseEvent| on_confirm.emit(()))
    };
    let cancel = {
        let on_cancel = props.on_cancel.clone();
        Callback::from(move |_: MouseEvent| on_cancel.emit(()))
    };
    let message_id = use_memo((), |_| fresh_id("dialog-message"));
    let described = (!props.message.is_empty()).then(|| AttrValue::from((*message_id).clone()));
    html! {
        <Modal
            title={props.title.clone()}
            on_cancel={props.on_cancel.clone()}
            busy={props.busy}
            describedby={described}
        >
            if !props.message.is_empty() {
                <p class="dialog-message" id={(*message_id).clone()}>{ props.message.clone() }</p>
            }
            if let Some(error) = props.error.clone() {
                <p class="error" role="alert">{ error }</p>
            }
            <div class="dialog-actions">
                <button class="secondary" disabled={props.busy} onclick={cancel}>{ "Cancel" }</button>
                <button
                    class={if props.destructive { "danger-button" } else { "primary" }}
                    disabled={props.busy}
                    onclick={go}
                >{ props.confirm.clone() }</button>
            </div>
        </Modal>
    }
}

/// A refusal no dialog has its own words for.
pub fn generic_failure(error: &ApiError) -> String {
    match error {
        ApiError::Throttled { .. } => error.detail(),
        _ if server_trouble(error) => SERVER_TROUBLE.to_string(),
        ApiError::Network(_) => "Can't reach the server. Check your connection.".to_string(),
        _ => "That didn't work. Try again.".to_string(),
    }
}

pub const SERVER_TROUBLE: &str = "The server had a problem. Try again in a moment.";

/// The server — or the proxy in front of it — ANSWERED, and the answer was
/// a failure of its own rather than a refusal of the request: `internal`,
/// or a status with no protocol body (a 502 while it restarts). Not "can't
/// reach the server", which would send somebody to check a network that
/// is fine.
pub fn server_trouble(error: &ApiError) -> bool {
    match error {
        ApiError::Network(detail) => detail.starts_with("The server answered"),
        ApiError::Server { code, .. } => code == "internal",
        _ => false,
    }
}
