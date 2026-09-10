//! Where the token lives, and how the tab remembers who is signed in.
//!
//! `sessionStorage`, NOT `localStorage` (docs/protocol.md, "A browser is a
//! client too"): the token is the whole credential, and one that outlives
//! the tab is one left behind on a shared machine. Closing the tab is a
//! sign-out.
//!
//! Every accessor is total. Storage can be absent or throw — a private
//! window, a browser configured to block site data — and a chat client that
//! panicked there would be a white page rather than a login form.

use web_sys::window;

const TOKEN_KEY: &str = "fc.token";

fn storage() -> Option<web_sys::Storage> {
    window()?.session_storage().ok()?
}

pub fn token() -> Option<String> {
    storage()?
        .get_item(TOKEN_KEY)
        .ok()?
        .filter(|t| !t.is_empty())
}

pub fn set_token(token: &str) {
    if let Some(storage) = storage() {
        let _ = storage.set_item(TOKEN_KEY, token);
    }
}

pub fn clear() {
    if let Some(storage) = storage() {
        let _ = storage.remove_item(TOKEN_KEY);
    }
}

/// The origin this page was served from — which IS the server
/// (docs/protocol.md, "A browser is a client too").
///
/// Empty when there is no window at all, which is only the case under a
/// test harness; every caller treats that as "relative", which is what a
/// browser does with a path anyway.
pub fn origin() -> String {
    window()
        .and_then(|window| window.location().origin().ok())
        .unwrap_or_default()
}
