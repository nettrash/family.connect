//! The D-Bus plumbing every desktop module shares (shell-design §0 "D-Bus", §1.6, §16–§19): gio's
//! GDBus on the application's OWN session-bus connection (`gio::Application::dbus_connection`) —
//! one connection, one thread, one main context, no zbus and no second connection.
//!
//! **Never a synchronous call.** A `call_sync` on the main thread blocks the main context, and a
//! service that answers through it (the self-test's fakes, GNOME Shell asking the app back) would
//! dead-lock until the timeout. Every call here is `call_future` or fire-and-forget.
//!
//! **Signals** go through `subscribe_to_signal` (`signal_subscribe` is deprecated in gio 0.22, an
//! error under `-D warnings`, §1.2). Objects are exported with gio's `register_object` builder and
//! introspection XML compiled in ([`interface`]); [`Exported`] and [`NameWatch`] undo themselves
//! when dropped.

use std::time::Duration;

use gio::prelude::*;
use glib::translate::IntoGlib;

/// The bus daemon's own name (and interface).
pub const BUS_NAME: &str = "org.freedesktop.DBus";
/// `org.freedesktop.DBus.Properties`.
pub const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

/// How long an ordinary method call may take before it is an error. GDBus's default is 25 s; a
/// desktop service that has not answered in 10 is not one to wait for. (A portal's DIALOG is not
/// bounded by this: its answer is a `Response` signal, `portal_request`.)
pub const CALL_TIMEOUT: Duration = Duration::from_secs(10);

/// The session bus GApplication registered on — `None` without one (Docker's `FC_NO_DBUS` only),
/// and then nothing of the desktop exists.
pub fn session(app: &impl IsA<gio::Application>) -> Option<gio::DBusConnection> {
    app.dbus_connection()
}

/// This connection's unique name (`:1.42`).
pub fn unique_name(connection: &gio::DBusConnection) -> Option<String> {
    connection.unique_name().map(|name| name.to_string())
}

/// The part of a portal request or session path that names the caller: the unique name without
/// its `:` and with every `.` as `_` (`:1.42` → `1_42`, the portal documentation's "SENDER").
pub fn sender_component(unique_name: &str) -> String {
    unique_name
        .strip_prefix(':')
        .unwrap_or(unique_name)
        .replace('.', "_")
}

/// An `a{sv}` from pairs, in their order.
pub fn vardict(entries: &[(&str, glib::Variant)]) -> glib::Variant {
    let dict = glib::VariantDict::new(None);
    for (key, value) in entries {
        dict.insert_value(key, value);
    }
    dict.end()
}

/// A string out of an `a{sv}`, whether it was written as `s` or `o` (portals differ: a
/// GlobalShortcuts `session_handle` is an `s`, other portals' are `o`).
pub fn dict_string(dict: &glib::VariantDict, key: &str) -> Option<String> {
    let value = dict.lookup_value(key, None)?;
    value.str().map(str::to_string)
}

/// One interface out of introspection XML compiled into the app. The XML is a constant and every
/// one of them is parsed by a unit test, so a failure here is a bug in this crate, not input.
pub fn interface(xml: &str, name: &str) -> gio::DBusInterfaceInfo {
    gio::DBusNodeInfo::for_xml(xml)
        .ok()
        .and_then(|node| node.lookup_interface(name))
        .unwrap_or_else(|| panic!("the compiled-in introspection of {name} is broken"))
}

/// A method call bounded by [`CALL_TIMEOUT`].
pub async fn call(
    connection: &gio::DBusConnection,
    destination: &str,
    path: &str,
    interface: &str,
    method: &str,
    parameters: Option<&glib::Variant>,
    reply: Option<&glib::VariantTy>,
) -> Result<glib::Variant, glib::Error> {
    connection
        .call_future(
            Some(destination),
            path,
            interface,
            method,
            parameters,
            reply,
            gio::DBusCallFlags::NONE,
            timeout_ms(CALL_TIMEOUT),
        )
        .await
}

/// `org.freedesktop.DBus.Properties.Get`, unwrapped from its `(v)`.
pub async fn get_property(
    connection: &gio::DBusConnection,
    destination: &str,
    path: &str,
    interface: &str,
    name: &str,
) -> Result<glib::Variant, glib::Error> {
    let reply = call(
        connection,
        destination,
        path,
        PROPERTIES,
        "Get",
        Some(&(interface, name).to_variant()),
        Some(glib::VariantTy::new("(v)").expect("a valid type")),
    )
    .await?;
    Ok(reply.child_value(0).as_variant().unwrap_or(reply))
}

/// A call whose answer nobody waits for (a `Request.Close` from a dropped request, a
/// `Session.Close`). A failure is logged by kind.
pub fn call_and_forget(
    connection: &gio::DBusConnection,
    destination: &str,
    path: &str,
    interface: &str,
    method: &str,
    parameters: Option<&glib::Variant>,
) {
    let what = format!("{interface}.{method}");
    connection.call(
        Some(destination),
        path,
        interface,
        method,
        parameters,
        None,
        gio::DBusCallFlags::NO_AUTO_START,
        timeout_ms(CALL_TIMEOUT),
        None::<&gio::Cancellable>,
        move |result| {
            if let Err(error) = result {
                crate::fc_debug!("dbus: {what} unanswered ({})", error_kind(&error));
            }
        },
    );
}

/// Emit a signal from one of the app's objects, to everybody (`destination: None`) or to one
/// connection. A failure (only a closed connection can cause one) is logged by kind.
pub fn emit(
    connection: &gio::DBusConnection,
    destination: Option<&str>,
    path: &str,
    interface: &str,
    signal: &str,
    parameters: &glib::Variant,
) {
    if let Err(error) =
        connection.emit_signal(destination, path, interface, signal, Some(parameters))
    {
        crate::fc_debug!(
            "dbus: {interface}.{signal} not emitted ({})",
            error_kind(&error)
        );
    }
}

/// `org.freedesktop.DBus.Properties.PropertiesChanged` for `interface` on `path`.
pub fn emit_properties_changed(
    connection: &gio::DBusConnection,
    path: &str,
    interface: &str,
    changed: &[(&str, glib::Variant)],
) {
    let invalidated: Vec<String> = Vec::new();
    let parameters = glib::Variant::tuple_from_iter([
        interface.to_variant(),
        vardict(changed),
        invalidated.to_variant(),
    ]);
    emit(
        connection,
        None,
        path,
        PROPERTIES,
        "PropertiesChanged",
        &parameters,
    );
}

/// Whether an error says nothing answers there at all — no owner for the name, no such object,
/// interface, method or property — rather than that something answered "no".
pub fn is_absent(error: &glib::Error) -> bool {
    matches!(
        error.kind::<gio::DBusError>(),
        Some(
            gio::DBusError::ServiceUnknown
                | gio::DBusError::NameHasNoOwner
                | gio::DBusError::UnknownMethod
                | gio::DBusError::UnknownInterface
                | gio::DBusError::UnknownObject
                | gio::DBusError::UnknownProperty
                | gio::DBusError::SpawnServiceNotFound
                | gio::DBusError::SpawnServiceInvalid
                | gio::DBusError::SpawnExecFailed
                | gio::DBusError::SpawnChildExited
        )
    )
}

/// An error as a log may say it (§26): the remote D-Bus error's name, or GLib's domain and code —
/// never its message, which can carry a path or a name.
pub fn error_kind(error: &glib::Error) -> String {
    match gio::DBusError::remote_error(error) {
        Some(name) => name.to_string(),
        None => format!("{} {}", error.domain().as_str(), error_code(error)),
    }
}

/// The numeric code of a GLib error (gio's `IOErrorEnum`, `DBusError`, or the raw code).
fn error_code(error: &glib::Error) -> i32 {
    if let Some(kind) = error.kind::<gio::DBusError>() {
        return kind.into_glib();
    }
    if let Some(kind) = error.kind::<gio::IOErrorEnum>() {
        return kind.into_glib();
    }
    -1
}

/// Milliseconds for GDBus's `timeout_msec`.
pub fn timeout_ms(duration: Duration) -> i32 {
    i32::try_from(duration.as_millis()).unwrap_or(i32::MAX)
}

/// Objects the app exported on a connection; unregistered when dropped.
pub struct Exported {
    connection: gio::DBusConnection,
    ids: Vec<gio::RegistrationId>,
}

impl Exported {
    pub fn new(connection: &gio::DBusConnection) -> Exported {
        Exported {
            connection: connection.clone(),
            ids: Vec::new(),
        }
    }

    pub fn push(&mut self, id: gio::RegistrationId) {
        self.ids.push(id);
    }
}

impl Drop for Exported {
    fn drop(&mut self) {
        for id in self.ids.drain(..) {
            let _ = self.connection.unregister_object(id);
        }
    }
}

impl std::fmt::Debug for Exported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Exported")
            .field("objects", &self.ids.len())
            .finish()
    }
}

/// A watch on a bus name (`g_bus_watch_name_on_connection`); unwatched when dropped.
pub struct NameWatch(Option<Box<dyn FnOnce()>>);

impl std::fmt::Debug for NameWatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NameWatch")
            .field("watching", &self.0.is_some())
            .finish()
    }
}

impl NameWatch {
    /// `appeared(owner)` when somebody owns `name` (at once if somebody does now), `vanished()`
    /// when nobody does (at once if nobody does now). With `auto_start` the bus is asked to start
    /// an activatable owner first — a portal that is installed but not yet running is not
    /// "absent".
    pub fn new(
        connection: &gio::DBusConnection,
        name: &str,
        auto_start: bool,
        appeared: impl Fn(&str) + 'static,
        vanished: impl Fn() + 'static,
    ) -> NameWatch {
        let flags = if auto_start {
            gio::BusNameWatcherFlags::AUTO_START
        } else {
            gio::BusNameWatcherFlags::NONE
        };
        let id = gio::bus_watch_name_on_connection(
            connection,
            name,
            flags,
            move |_, _, owner| appeared(owner),
            move |_, _| vanished(),
        );
        NameWatch(Some(Box::new(move || gio::bus_unwatch_name(id))))
    }
}

impl Drop for NameWatch {
    fn drop(&mut self) {
        if let Some(unwatch) = self.0.take() {
            unwatch();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sender_component_is_the_unique_name_without_colon_and_dots() {
        assert_eq!(sender_component(":1.42"), "1_42");
        assert_eq!(sender_component(":1.4.2"), "1_4_2");
        assert_eq!(sender_component("1.7"), "1_7");
    }

    #[test]
    fn a_vardict_keeps_every_pair_and_its_type() {
        let dict = vardict(&[
            ("handle_token", "fc_1".to_variant()),
            ("count", 3i64.to_variant()),
        ]);
        assert_eq!(dict.type_().as_str(), "a{sv}");
        let read = glib::VariantDict::new(Some(&dict));
        assert_eq!(dict_string(&read, "handle_token").as_deref(), Some("fc_1"));
        assert_eq!(
            read.lookup_value("count", None)
                .and_then(|v| v.get::<i64>()),
            Some(3)
        );
        assert_eq!(dict_string(&read, "missing"), None);
    }

    #[test]
    fn a_dict_string_reads_an_object_path_too() {
        let path =
            glib::variant::ObjectPath::try_from("/org/freedesktop/portal/desktop/session/1_4/t")
                .unwrap();
        let dict = vardict(&[("session_handle", path.to_variant())]);
        let read = glib::VariantDict::new(Some(&dict));
        assert_eq!(
            dict_string(&read, "session_handle").as_deref(),
            Some("/org/freedesktop/portal/desktop/session/1_4/t")
        );
    }

    #[test]
    fn absent_means_nothing_answered_and_kinds_carry_no_message() {
        let unknown = glib::Error::new(gio::DBusError::ServiceUnknown, "no /home/x here");
        assert!(is_absent(&unknown));
        let denied = glib::Error::new(gio::DBusError::AccessDenied, "denied");
        assert!(!is_absent(&denied));
        let kind = error_kind(&unknown);
        assert!(!kind.contains("/home/x"), "{kind}");
        // GDBus names its own errors by their D-Bus name.
        assert_eq!(kind, "org.freedesktop.DBus.Error.ServiceUnknown");
        let io = glib::Error::new(gio::IOErrorEnum::NotFound, "/home/x/secret");
        assert_eq!(
            error_kind(&io),
            format!(
                "g-io-error-quark {}",
                gio::IOErrorEnum::NotFound.into_glib()
            )
        );
        assert_eq!(timeout_ms(Duration::from_secs(10)), 10_000);
    }
}
