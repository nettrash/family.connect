//! The tab's own notifications, and the count in its title
//! (docs/protocol.md, "A browser is a client too").
//!
//! A browser takes no push, so this is what it has instead. Two rules are
//! the protocol's: what it says names WHO and never what — the operator's
//! `include_message_body` is not on the wire, and a browser that cannot
//! know it has been told no does not guess — and what is notified is what
//! would be pushed, the block first among the gating.
//!
//! One rule is the browser's: a page may not ask for permission unprompted.
//! So the asking is a switch in Settings, and this module only ever acts on
//! a permission somebody has already given.

use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

/// Where the switch is kept: this browser, past the tab, beside the board's
/// seen-marks — a preference is not worth a round trip, and it is nobody
/// else's business which of somebody's devices notifies them.
const WANTED_KEY: &str = "fc.notify";

/// Whether the reader asked for notifications ON THIS BROWSER.
pub fn wanted() -> bool {
    lasting()
        .and_then(|storage| storage.get_item(WANTED_KEY).ok().flatten())
        .as_deref()
        == Some("yes")
}

pub fn set_wanted(wanted: bool) {
    if let Some(storage) = lasting() {
        let _ = if wanted {
            storage.set_item(WANTED_KEY, "yes")
        } else {
            storage.remove_item(WANTED_KEY)
        };
    }
}

fn lasting() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

/// What this browser has been told about notifications: "granted",
/// "denied", "default" — or "unsupported" where there is no Notification at
/// all (an old browser, or a page not served over https).
pub fn permission() -> String {
    let Some(notification) = class() else {
        return "unsupported".to_string();
    };
    js_sys::Reflect::get(&notification, &JsValue::from_str("permission"))
        .ok()
        .and_then(|value| value.as_string())
        .unwrap_or_else(|| "default".to_string())
}

pub fn supported() -> bool {
    class().is_some()
}

/// Ask, from the click that asked for it — the only moment a browser will
/// allow the question. Answers whether it may notify now.
pub async fn ask() -> bool {
    let Some(notification) = class() else {
        return false;
    };
    let Ok(request) = js_sys::Reflect::get(&notification, &JsValue::from_str("requestPermission"))
    else {
        return false;
    };
    let Ok(request) = request.dyn_into::<js_sys::Function>() else {
        return false;
    };
    let Ok(answer) = request.call0(&notification) else {
        return false;
    };
    // Old browsers answer a callback rather than a promise; those simply
    // report whatever the permission is once the call returns.
    if let Ok(promise) = answer.dyn_into::<js_sys::Promise>() {
        let _ = JsFuture::from(promise).await;
    }
    permission() == "granted"
}

/// Show one, if the reader asked for it and the browser allows it. `tag`
/// replaces a notification already on screen rather than stacking another:
/// one line per chat, not one per message.
///
/// Clicking it brings the tab to the front — which is the whole point of
/// it — and `chosen` is told which chat it was for.
pub fn tell(title: &str, body: &str, tag: &str, chosen: impl Fn() + 'static) {
    if !wanted() || permission() != "granted" {
        return;
    }
    let options = web_sys::NotificationOptions::new();
    options.set_body(body);
    options.set_tag(tag);
    let Ok(shown) = web_sys::Notification::new_with_options(title, &options) else {
        return;
    };
    let clicked = Closure::<dyn FnMut()>::new(move || {
        if let Some(window) = web_sys::window() {
            let _ = window.focus();
        }
        chosen();
    });
    shown.set_onclick(Some(clicked.as_ref().unchecked_ref()));
    // The closure outlives this call by design: it belongs to a
    // notification the reader may click minutes later.
    clicked.forget();
}

fn class() -> Option<JsValue> {
    let window = web_sys::window()?;
    let notification = js_sys::Reflect::get(&window, &JsValue::from_str("Notification")).ok()?;
    (!notification.is_undefined() && !notification.is_null()).then_some(notification)
}
