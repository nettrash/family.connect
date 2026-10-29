//! The XDG portals' Request/Response pattern, which every portal call of the app goes through
//! (shell-design §19's sequence; xdg-desktop-portal's `org.freedesktop.portal.Request`).
//!
//! A portal method that may show a dialog does not answer with its result. It answers with the
//! path of a REQUEST object, and the result comes later as that object's `Response(u, a{sv})`
//! signal — after a dialog that may take the person minutes. The portal builds the path from the
//! caller's unique name and a `handle_token` the caller chose:
//! `/org/freedesktop/portal/desktop/request/<SENDER>/<token>` ([`request_path`]), so the app
//! subscribes to the signal on that predicted path BEFORE it calls — a fast portal can answer
//! before the method's own reply has been read. A portal older than 0.9 answered with another path
//! than the predicted one; then the subscription moves there.
//!
//! **Cancellation is dropping the future** (port-plan "Async"): a request dropped before its
//! answer sends `Request.Close`, which takes the portal's dialog away.
//!
//! **The parent window** ([`parent_window`]) is what lets GNOME put its dialog over the app's
//! window: `wayland:<handle>` from `gdk4_wayland::WaylandToplevel::export_handle`, the handle
//! dropped when the request is done; `""` when the window is not mapped, and on X11 — its `x11:<XID>`
//! would need the `gdk4-x11` crate, which shell-design Appendix B adds only if X11 sessions get
//! parent windows (GNOME 50 has no X11 session).

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gio::prelude::*;
use gtk::prelude::*;

use super::dbus;

/// The portals' bus name, and the path every portal interface lives on.
pub const PORTAL_BUS_NAME: &str = "org.freedesktop.portal.Desktop";
pub const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
/// The request objects' interface.
pub const REQUEST_INTERFACE: &str = "org.freedesktop.portal.Request";
/// The session objects' interface (GlobalShortcuts, ScreenCast …).
pub const SESSION_INTERFACE: &str = "org.freedesktop.portal.Session";

/// How long a window's handle may take to be exported before the dialog goes without a parent.
const EXPORT_LIMIT: Duration = Duration::from_secs(1);

thread_local! {
    static SERIAL: Cell<u64> = const { Cell::new(0) };
}

/// A fresh `handle_token` / `session_handle_token`: letters, digits and `_` only (an object path
/// element), unique in this process, and not guessable from outside it.
pub fn new_token(kind: &str) -> String {
    let serial = SERIAL.with(|serial| {
        let next = serial.get() + 1;
        serial.set(next);
        next
    });
    token_from(kind, serial, glib::random_int())
}

/// [`new_token`]'s shape, from its parts.
pub fn token_from(kind: &str, serial: u64, random: u32) -> String {
    let kind: String = kind
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    format!("fc_{kind}{serial}_{random:08x}")
}

/// Whether `token` may be a path element.
#[cfg(test)]
pub fn is_valid_token(token: &str) -> bool {
    !token.is_empty() && token.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The request object the portal will create for `token` from the connection named `unique_name`.
pub fn request_path(unique_name: &str, token: &str) -> String {
    format!(
        "{PORTAL_PATH}/request/{}/{token}",
        dbus::sender_component(unique_name)
    )
}

/// The session object the portal will create for `token` (`session_handle_token`).
pub fn session_path(unique_name: &str, token: &str) -> String {
    format!(
        "{PORTAL_PATH}/session/{}/{token}",
        dbus::sender_component(unique_name)
    )
}

/// What a portal answered.
#[derive(Debug, Clone, PartialEq)]
pub enum Response {
    /// `0`: done; its results (an `a{sv}`).
    Success(glib::Variant),
    /// `1`: the person dismissed the dialog.
    Cancelled,
    /// `2` (or anything else): the interaction ended some other way.
    Ended,
}

impl Response {
    /// The `Response` signal's two values.
    pub fn from_code(code: u32, results: glib::Variant) -> Response {
        match code {
            0 => Response::Success(results),
            1 => Response::Cancelled,
            _ => Response::Ended,
        }
    }

    /// The results of a success, readable by key.
    pub fn results(&self) -> Option<glib::VariantDict> {
        match self {
            Response::Success(results) if results.type_().as_str() == "a{sv}" => {
                Some(glib::VariantDict::new(Some(results)))
            }
            Response::Success(_) => Some(glib::VariantDict::new(None)),
            _ => None,
        }
    }

    /// The name a log uses.
    pub fn name(&self) -> &'static str {
        match self {
            Response::Success(_) => "success",
            Response::Cancelled => "cancelled",
            Response::Ended => "ended",
        }
    }
}

/// Why a request has no [`Response`].
#[derive(Debug)]
pub enum RequestError {
    /// No session bus, or a connection without a unique name.
    NoBus,
    /// The method call itself failed.
    Call(glib::Error),
    /// The answer can no longer come: the portal left the bus before it answered.
    Lost,
}

impl RequestError {
    /// Nothing answers there: no portal, one without this interface or method, or one that left
    /// the bus before it answered.
    pub fn is_absent(&self) -> bool {
        match self {
            RequestError::NoBus => true,
            RequestError::Call(error) => dbus::is_absent(error),
            RequestError::Lost => true,
        }
    }

    /// The error as a log may say it.
    pub fn kind(&self) -> String {
        match self {
            RequestError::NoBus => "no bus".to_string(),
            RequestError::Call(error) => dbus::error_kind(error),
            RequestError::Lost => "lost".to_string(),
        }
    }
}

/// Sends `Request.Close` for a request still open when the caller stops waiting.
struct CloseOnDrop {
    connection: gio::DBusConnection,
    portal: String,
    handle: String,
    armed: bool,
}

impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        if self.armed {
            crate::fc_debug!("portal: a request was dropped; closing it");
            dbus::call_and_forget(
                &self.connection,
                &self.portal,
                &self.handle,
                REQUEST_INTERFACE,
                "Close",
                None,
            );
        }
    }
}

/// Call `interface.method` on the portal `portal` (normally [`PORTAL_BUS_NAME`]) and wait for its
/// `Response`. `parameters` builds the call's arguments from the `handle_token` it must carry in
/// its options. Dropping the future closes the request.
pub async fn request(
    connection: &gio::DBusConnection,
    portal: &str,
    interface: &str,
    method: &str,
    parameters: impl FnOnce(&str) -> glib::Variant,
) -> Result<Response, RequestError> {
    let unique = dbus::unique_name(connection).ok_or(RequestError::NoBus)?;
    let token = new_token("r");
    let predicted = request_path(&unique, &token);
    // `None`: the portal left the bus before it answered.
    let (answer, answered) = futures_channel::oneshot::channel::<Option<(u32, glib::Variant)>>();
    let answer = Rc::new(RefCell::new(Some(answer)));
    let subscribe = |path: &str| {
        let answer = Rc::clone(&answer);
        connection.subscribe_to_signal(
            None,
            Some(REQUEST_INTERFACE),
            Some("Response"),
            Some(path),
            None,
            gio::DBusSignalFlags::NONE,
            move |signal| {
                let parameters = signal.parameters;
                if parameters.type_().as_str() != "(ua{sv})" {
                    return;
                }
                let code = parameters.child_value(0).get::<u32>().unwrap_or(2);
                let results = parameters.child_value(1);
                if let Some(answer) = answer.borrow_mut().take() {
                    let _ = answer.send(Some((code, results)));
                }
            },
        )
    };
    let mut subscription = subscribe(&predicted);
    // A portal that crashes or is replaced mid-dialog never answers: its leaving ends the wait.
    let left = {
        let answer = Rc::clone(&answer);
        connection.subscribe_to_signal(
            Some(dbus::BUS_NAME),
            Some(dbus::BUS_NAME),
            Some("NameOwnerChanged"),
            Some("/org/freedesktop/DBus"),
            Some(portal),
            gio::DBusSignalFlags::NONE,
            move |signal| {
                if signal.parameters.child_value(2).str() == Some("")
                    && let Some(answer) = answer.borrow_mut().take()
                {
                    let _ = answer.send(None);
                }
            },
        )
    };
    let mut guard = CloseOnDrop {
        connection: connection.clone(),
        portal: portal.to_string(),
        handle: predicted.clone(),
        armed: true,
    };
    let arguments = parameters(&token);
    let reply = dbus::call(
        connection,
        portal,
        PORTAL_PATH,
        interface,
        method,
        Some(&arguments),
        Some(glib::VariantTy::new("(o)").expect("a valid type")),
    )
    .await;
    let reply = match reply {
        Ok(reply) => reply,
        Err(error) => {
            // No request object was made: nothing to close.
            guard.armed = false;
            return Err(RequestError::Call(error));
        }
    };
    if let Some(handle) = reply.child_value(0).str()
        && handle != predicted
        && !handle.is_empty()
    {
        crate::fc_debug!("portal: {method} answered on an unpredicted path");
        subscription = subscribe(handle);
        guard.handle = handle.to_string();
    }
    let answered = answered.await;
    guard.armed = false;
    drop(subscription);
    drop(left);
    match answered {
        Ok(Some((code, results))) => {
            let response = Response::from_code(code, results);
            crate::fc_debug!("portal: {method} {}", response.name());
            Ok(response)
        }
        Ok(None) => {
            crate::fc_debug!("portal: left the bus during {method}");
            Err(RequestError::Lost)
        }
        Err(_) => Err(RequestError::Lost),
    }
}

/// Close a portal session (`Session.Close`), without waiting.
pub fn close_session(connection: &gio::DBusConnection, portal: &str, session_handle: &str) {
    dbus::call_and_forget(
        connection,
        portal,
        session_handle,
        SESSION_INTERFACE,
        "Close",
        None,
    );
}

/// `closed()` when the portal ends the session itself (`Session.Closed`).
pub fn on_session_closed(
    connection: &gio::DBusConnection,
    session_handle: &str,
    closed: impl Fn() + 'static,
) -> gio::SignalSubscription {
    connection.subscribe_to_signal(
        None,
        Some(SESSION_INTERFACE),
        Some("Closed"),
        Some(session_handle),
        None,
        gio::DBusSignalFlags::NONE,
        move |_| closed(),
    )
}

/// A portal call's `parent_window` argument, and the exported handle behind it (dropped with it).
pub struct ParentWindow {
    identifier: String,
    exported: Option<(gdk_wayland::WaylandToplevel, String)>,
}

impl ParentWindow {
    /// No parent: the dialog stands on its own.
    pub fn none() -> ParentWindow {
        ParentWindow {
            identifier: String::new(),
            exported: None,
        }
    }

    /// `wayland:<handle>`, or `""`.
    pub fn identifier(&self) -> &str {
        &self.identifier
    }
}

impl Drop for ParentWindow {
    fn drop(&mut self) {
        if let Some((toplevel, handle)) = self.exported.take() {
            toplevel.drop_exported_handle(&handle);
        }
    }
}

impl std::fmt::Debug for ParentWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParentWindow")
            .field("kind", &self.identifier.split(':').next().unwrap_or(""))
            .finish()
    }
}

/// The identifier of a Wayland handle (the portal documentation's "Window Identifiers").
pub fn wayland_identifier(handle: &str) -> String {
    format!("wayland:{handle}")
}

/// The parent window for a portal dialog over `window`: its exported Wayland handle when it is
/// mapped on Wayland, otherwise none (see the module's comment).
pub async fn parent_window(window: Option<&gtk::Window>) -> ParentWindow {
    let Some(window) = window.filter(|window| window.is_mapped()) else {
        return ParentWindow::none();
    };
    let Some(surface) = window.surface() else {
        return ParentWindow::none();
    };
    let Ok(toplevel) = surface.downcast::<gdk_wayland::WaylandToplevel>() else {
        return ParentWindow::none();
    };
    let (sender, exported) = futures_channel::oneshot::channel::<Option<String>>();
    let sender = Cell::new(Some(sender));
    let started = toplevel.export_handle(move |_, handle| {
        if let Some(sender) = sender.take() {
            let _ = sender.send(handle.ok().map(str::to_string));
        }
    });
    if !started {
        return ParentWindow::none();
    }
    match glib::future_with_timeout(EXPORT_LIMIT, exported).await {
        Ok(Ok(Some(handle))) => ParentWindow {
            identifier: wayland_identifier(&handle),
            exported: Some((toplevel, handle)),
        },
        _ => ParentWindow::none(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_path_elements_and_never_repeat() {
        let a = new_token("r");
        let b = new_token("r");
        assert_ne!(a, b);
        assert!(is_valid_token(&a), "{a}");
        assert!(is_valid_token(&new_token("s")));
        assert_eq!(token_from("r", 7, 0xbeef), "fc_r7_0000beef");
        // A kind that is not a path element is cleaned, never passed through.
        assert_eq!(token_from("a/b.c", 1, 1), "fc_abc1_00000001");
        assert!(!is_valid_token(""));
        assert!(!is_valid_token("a-b"));
        assert!(!is_valid_token("a/b"));
    }

    #[test]
    fn request_and_session_paths_are_the_portals_prediction() {
        assert_eq!(
            request_path(":1.42", "fc_r1_00000001"),
            "/org/freedesktop/portal/desktop/request/1_42/fc_r1_00000001"
        );
        assert_eq!(
            session_path(":1.42", "fc_s2_00000002"),
            "/org/freedesktop/portal/desktop/session/1_42/fc_s2_00000002"
        );
        assert!(glib::variant::ObjectPath::try_from(request_path(":1.9", &new_token("r"))).is_ok());
    }

    #[test]
    fn response_codes_map_to_the_three_outcomes() {
        let results = dbus::vardict(&[("shortcuts", 1u32.to_variant())]);
        let success = Response::from_code(0, results.clone());
        assert_eq!(success, Response::Success(results));
        assert!(success.results().unwrap().contains("shortcuts"));
        assert_eq!(Response::from_code(1, ().to_variant()), Response::Cancelled);
        assert_eq!(Response::from_code(2, ().to_variant()), Response::Ended);
        assert_eq!(Response::from_code(9, ().to_variant()), Response::Ended);
        assert!(Response::Cancelled.results().is_none());
        // A success whose results are not a dictionary reads as an empty one.
        assert!(
            !Response::from_code(0, 5u32.to_variant())
                .results()
                .unwrap()
                .contains("x")
        );
    }

    #[test]
    fn absent_errors_are_told_apart_from_refusals() {
        assert!(RequestError::NoBus.is_absent());
        assert!(
            RequestError::Call(glib::Error::new(gio::DBusError::UnknownInterface, "x")).is_absent()
        );
        assert!(
            !RequestError::Call(glib::Error::new(gio::DBusError::AccessDenied, "x")).is_absent()
        );
        // A portal that left the bus mid-dialog is gone, not a refusal.
        assert!(RequestError::Lost.is_absent());
        assert_eq!(wayland_identifier("abc"), "wayland:abc");
        assert_eq!(ParentWindow::none().identifier(), "");
    }
}
