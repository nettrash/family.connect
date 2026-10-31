//! The `desktop` group (P5, shell-design §27): everything P5 says to the desktop, observed on the
//! self-test's PRIVATE session bus by fakes of the desktop's own services, which this group runs
//! on a SECOND connection of its own (so every call is real D-Bus traffic through the bus daemon,
//! and a fake leaving the bus is a real `NameOwnerChanged`):
//!
//! | Fake | Owns | Stands in for |
//! | --- | --- | --- |
//! | notifications | `org.gtk.Notifications` and `org.freedesktop.Notifications` | GNOME Shell (GLib's GNOME backend), or any notification server (GLib's freedesktop backend) — GLib picks its backend once, at the first notification, so both are served |
//! | dock | (a signal subscription) | Ubuntu Dock reading `com.canonical.Unity.LauncherEntry.Update` |
//! | watcher | `org.kde.StatusNotifierWatcher` | Ubuntu's AppIndicator extension |
//! | portal | `org.freedesktop.portal.Desktop` (`host.portal.Registry`, `GlobalShortcuts`, sessions) | xdg-desktop-portal with GNOME's backend |
//!
//! And for real: the token saved to and read back from gnome-keyring (the run's unlocked keyring,
//! `scripts/dev.sh`), directly and through S0's sign-in; `Unavailable` from a Secret Service bus
//! name nobody owns; the autostart entry written and removed under `OUT/home`.
//!
//! The fakes never answer the app synchronously from the main thread's point of view: P5 makes no
//! sync call, and the group starts the portal only after the notification backend is chosen and
//! stops it before it asks GTK to inhibit (GTK's inhibit is a sync call).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use fc_core::protocol::{ChatDto, ChatRowDto, ServerUrl, TokenStore};
use fc_logic::global_hot_key_rules::PortalState;
use fc_logic::notifications::{NotificationRules, Toast};
use fc_logic::startup_setting::{self, StartupState};
use fc_logic::tray_rules;

use super::Harness;
use crate::desktop::dbus::{self, Exported};
use crate::desktop::{
    autostart_file, badge, dbusmenu, host_registry, hotkey, keep_awake, portal_request, secret,
    tray,
};
use crate::seams::routes::{CallRoutes, ChatRoutes, FamilyRoutes};
use crate::seams::tokens::{TokenVault, VaultRead, VaultedTokens, origin_of};
use crate::services::settings_store::keys;
use crate::windows::main_window::GatePage;

const WAIT: Duration = Duration::from_secs(10);
const LONG_WAIT: Duration = Duration::from_secs(20);
/// Set in the child process that meets a desktop with no Secret Service.
const NO_KEYRING_ENV: &str = "FC_SELFTEST_DESKTOP_NO_KEYRING";
/// A session bus with no service folder: nothing can be activated, so no Secret Service exists.
const NO_SERVICE_BUS: &str = r#"<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:tmpdir=/tmp</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
"#;
/// A message text that must never reach the desktop.
const SECRET_TEXT: &str = "Secret dinner plans at seven";

/// A `bind_answer` for a dialog nobody ever closes.
const NEVER_ANSWER: u32 = 99;

const GTK_NOTIFICATIONS: &str = "org.gtk.Notifications";
const FDO_NOTIFICATIONS: &str = "org.freedesktop.Notifications";

thread_local! {
    /// The fakes [`serve_notifications`] put on the bus before the first group, and whether both
    /// notification names were owned; [`run`] takes them.
    static EARLY: RefCell<Option<(Rc<Fakes>, bool)>> = const { RefCell::new(None) };
}

/// Puts the fake notification servers on the bus before the first group, as GNOME Shell's is from
/// the start of a session: GLib picks its notification backend once, at the first notification or
/// withdrawal ANY group causes, and this group's checks read the GNOME backend's.
pub async fn serve_notifications() {
    if std::env::var_os(NO_KEYRING_ENV).is_some() {
        return;
    }
    let Some(fakes) = Fakes::start().await else {
        return;
    };
    let up = fakes.start_notifications().await;
    EARLY.with(|early| *early.borrow_mut() = Some((fakes, up)));
}
const FDO_PATH: &str = "/org/freedesktop/Notifications";
const SNI_INTERFACE: &str = "org.kde.StatusNotifierItem";

const GTK_NOTIFICATIONS_XML: &str = r#"<node>
  <interface name="org.gtk.Notifications">
    <method name="AddNotification">
      <arg name="app_id" type="s" direction="in"/>
      <arg name="id" type="s" direction="in"/>
      <arg name="notification" type="a{sv}" direction="in"/>
    </method>
    <method name="RemoveNotification">
      <arg name="app_id" type="s" direction="in"/>
      <arg name="id" type="s" direction="in"/>
    </method>
  </interface>
</node>"#;

const FDO_NOTIFICATIONS_XML: &str = r#"<node>
  <interface name="org.freedesktop.Notifications">
    <method name="Notify">
      <arg name="app_name" type="s" direction="in"/>
      <arg name="replaces_id" type="u" direction="in"/>
      <arg name="app_icon" type="s" direction="in"/>
      <arg name="summary" type="s" direction="in"/>
      <arg name="body" type="s" direction="in"/>
      <arg name="actions" type="as" direction="in"/>
      <arg name="hints" type="a{sv}" direction="in"/>
      <arg name="expire_timeout" type="i" direction="in"/>
      <arg name="id" type="u" direction="out"/>
    </method>
    <method name="CloseNotification">
      <arg name="id" type="u" direction="in"/>
    </method>
    <method name="GetCapabilities">
      <arg name="capabilities" type="as" direction="out"/>
    </method>
    <method name="GetServerInformation">
      <arg name="name" type="s" direction="out"/>
      <arg name="vendor" type="s" direction="out"/>
      <arg name="version" type="s" direction="out"/>
      <arg name="spec_version" type="s" direction="out"/>
    </method>
    <signal name="NotificationClosed">
      <arg name="id" type="u"/>
      <arg name="reason" type="u"/>
    </signal>
    <signal name="ActionInvoked">
      <arg name="id" type="u"/>
      <arg name="action_key" type="s"/>
    </signal>
  </interface>
</node>"#;

const WATCHER_XML: &str = r#"<node>
  <interface name="org.kde.StatusNotifierWatcher">
    <method name="RegisterStatusNotifierItem">
      <arg name="service" type="s" direction="in"/>
    </method>
    <method name="RegisterStatusNotifierHost">
      <arg name="service" type="s" direction="in"/>
    </method>
    <property name="RegisteredStatusNotifierItems" type="as" access="read"/>
    <property name="IsStatusNotifierHostRegistered" type="b" access="read"/>
    <property name="ProtocolVersion" type="i" access="read"/>
    <signal name="StatusNotifierItemRegistered">
      <arg name="service" type="s"/>
    </signal>
    <signal name="StatusNotifierItemUnregistered">
      <arg name="service" type="s"/>
    </signal>
    <signal name="StatusNotifierHostRegistered"/>
    <signal name="StatusNotifierHostUnregistered"/>
  </interface>
</node>"#;

const PORTAL_XML: &str = r#"<node>
  <interface name="org.freedesktop.host.portal.Registry">
    <method name="Register">
      <arg name="app_id" type="s" direction="in"/>
      <arg name="options" type="a{sv}" direction="in"/>
    </method>
  </interface>
  <interface name="org.freedesktop.portal.GlobalShortcuts">
    <method name="CreateSession">
      <arg name="options" type="a{sv}" direction="in"/>
      <arg name="handle" type="o" direction="out"/>
    </method>
    <method name="BindShortcuts">
      <arg name="session_handle" type="o" direction="in"/>
      <arg name="shortcuts" type="a(sa{sv})" direction="in"/>
      <arg name="parent_window" type="s" direction="in"/>
      <arg name="options" type="a{sv}" direction="in"/>
      <arg name="request_handle" type="o" direction="out"/>
    </method>
    <method name="ListShortcuts">
      <arg name="session_handle" type="o" direction="in"/>
      <arg name="options" type="a{sv}" direction="in"/>
      <arg name="request_handle" type="o" direction="out"/>
    </method>
    <method name="ConfigureShortcuts">
      <arg name="session_handle" type="o" direction="in"/>
      <arg name="parent_window" type="s" direction="in"/>
      <arg name="options" type="a{sv}" direction="in"/>
    </method>
    <signal name="Activated">
      <arg name="session_handle" type="o"/>
      <arg name="shortcut_id" type="s"/>
      <arg name="timestamp" type="t"/>
      <arg name="options" type="a{sv}"/>
    </signal>
    <signal name="ShortcutsChanged">
      <arg name="session_handle" type="o"/>
      <arg name="shortcuts" type="a(sa{sv})"/>
    </signal>
    <property name="version" type="u" access="read"/>
  </interface>
</node>"#;

const SESSION_XML: &str = r#"<node>
  <interface name="org.freedesktop.portal.Session">
    <method name="Close"/>
    <signal name="Closed">
      <arg name="details" type="a{sv}"/>
    </signal>
    <property name="version" type="u" access="read"/>
  </interface>
</node>"#;

/// A notification as a fake server saw it, whichever backend GLib chose.
#[derive(Clone, Debug, Default)]
struct Seen {
    /// `gtk` or `fdo`.
    backend: &'static str,
    /// gtk: the GNotification id (the tag); fdo: the server's number.
    key: String,
    sender: String,
    title: String,
    body: String,
    /// gtk: GLib's priority nick; fdo: `urgency <n>`.
    priority: String,
    category: String,
    default_action: Option<String>,
    default_target: Option<glib::Variant>,
    buttons: Vec<Button>,
    /// Everything that was sent, printed — searched for message text.
    raw: String,
}

#[derive(Clone, Debug)]
struct Button {
    label: String,
    /// gtk: the action; fdo: the detailed action (`app.answer-call('c-1')`).
    action: String,
    target: Option<glib::Variant>,
}

/// One `LauncherEntry.Update` the dock saw.
#[derive(Clone, Debug, PartialEq)]
struct LauncherSeen {
    sender: String,
    uri: String,
    count: i64,
    visible: bool,
    urgent: bool,
}

/// What the fakes have seen and how they answer.
#[derive(Default)]
struct World {
    notes: Vec<Seen>,
    removed: Vec<String>,
    next_fdo_id: u32,
    launcher: Vec<LauncherSeen>,
    /// The watcher's registrations: (sender, service).
    items: Vec<(String, String)>,
    item_signals: Vec<String>,
    host: bool,
    registered_apps: Vec<String>,
    portal_calls: Vec<String>,
    sessions: HashMap<String, Exported>,
    bound: HashMap<String, Vec<(String, String)>>,
    /// The `Response` code `BindShortcuts` answers (0: bound; 1: dismissed; [`NEVER_ANSWER`]).
    bind_answer: u32,
    trigger: String,
    /// The last `BindShortcuts`: (parent_window, the shortcuts printed).
    last_bind: Option<(String, String)>,
    /// `Activated` signals the portal emitted for an app session.
    last_session: Option<String>,
}

/// The fakes and their connection.
struct Fakes {
    connection: gio::DBusConnection,
    world: Rc<RefCell<World>>,
    exported: RefCell<HashMap<&'static str, Vec<Exported>>>,
    subscriptions: RefCell<Vec<gio::SignalSubscription>>,
}

impl Fakes {
    async fn start() -> Option<Rc<Fakes>> {
        let address =
            gio::dbus_address_get_for_bus_sync(gio::BusType::Session, None::<&gio::Cancellable>)
                .ok()?;
        let connection = gio::DBusConnection::for_address_future(
            &address,
            gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
                | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
            None,
        )
        .await
        .ok()?;
        let world = World {
            next_fdo_id: 1,
            host: true,
            trigger: "Ctrl+Alt+Shift+F".to_string(),
            ..World::default()
        };
        Some(Rc::new(Fakes {
            connection,
            world: Rc::new(RefCell::new(world)),
            exported: RefCell::new(HashMap::new()),
            subscriptions: RefCell::new(Vec::new()),
        }))
    }

    async fn request_name(&self, name: &str) -> bool {
        let reply = dbus::call(
            &self.connection,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "RequestName",
            Some(&(name, 4u32).to_variant()),
            Some(glib::VariantTy::new("(u)").unwrap()),
        )
        .await;
        // 1: the primary owner now; 4: already it.
        matches!(
            reply.ok().and_then(|r| r.child_value(0).get::<u32>()),
            Some(1 | 4)
        )
    }

    async fn release_name(&self, name: &str) {
        let _ = dbus::call(
            &self.connection,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "ReleaseName",
            Some(&(name,).to_variant()),
            None,
        )
        .await;
    }

    /// Exports `interface` of `xml` at `path` under `key`; `answer` gets (world, connection,
    /// sender, method, parameters, invocation).
    fn export(
        &self,
        key: &'static str,
        path: &str,
        xml: &str,
        interface: &str,
        answer: impl Fn(
            &Rc<RefCell<World>>,
            &gio::DBusConnection,
            &str,
            &str,
            &glib::Variant,
            gio::DBusMethodInvocation,
        ) + 'static,
        property: impl Fn(&World, &str) -> glib::Variant + 'static,
    ) -> bool {
        let info = dbus::interface(xml, interface);
        let world = Rc::clone(&self.world);
        let props = Rc::clone(&self.world);
        let registered = self
            .connection
            .register_object(path, &info)
            .method_call(
                move |connection, sender, _, _, method, parameters, invocation| {
                    answer(
                        &world,
                        &connection,
                        sender.unwrap_or_default(),
                        method,
                        &parameters,
                        invocation,
                    );
                },
            )
            .property(move |_, _, _, _, name| property(&props.borrow(), name))
            .build();
        match registered {
            Ok(id) => {
                let mut exported = Exported::new(&self.connection);
                exported.push(id);
                self.exported
                    .borrow_mut()
                    .entry(key)
                    .or_default()
                    .push(exported);
                true
            }
            Err(error) => {
                eprintln!(
                    "selftest: cannot export {path}: {}",
                    dbus::error_kind(&error)
                );
                false
            }
        }
    }

    fn unexport(&self, key: &'static str) {
        self.exported.borrow_mut().remove(key);
    }

    fn world(&self) -> std::cell::Ref<'_, World> {
        self.world.borrow()
    }

    // ---- the notification servers ---------------------------------------------------------------

    async fn start_notifications(&self) -> bool {
        let gtk = self.export(
            "notifications",
            "/org/gtk/Notifications",
            GTK_NOTIFICATIONS_XML,
            GTK_NOTIFICATIONS,
            answer_gtk_notifications,
            |_, _| ().to_variant(),
        );
        let fdo = self.export(
            "notifications",
            FDO_PATH,
            FDO_NOTIFICATIONS_XML,
            FDO_NOTIFICATIONS,
            answer_fdo_notifications,
            |_, _| ().to_variant(),
        );
        gtk && fdo
            && self.request_name(GTK_NOTIFICATIONS).await
            && self.request_name(FDO_NOTIFICATIONS).await
    }

    /// The notification that `matches`, last first.
    fn note(&self, matches: impl Fn(&Seen) -> bool) -> Option<Seen> {
        self.world()
            .notes
            .iter()
            .rev()
            .find(|n| matches(n))
            .cloned()
    }

    /// Clicks `seen` the way its backend's server would: GNOME Shell's
    /// `org.freedesktop.Application.ActivateAction`, or the freedesktop `ActionInvoked` signal.
    /// `button`: which button, or the body.
    async fn click(&self, seen: &Seen, button: Option<usize>, app_path: &str) -> bool {
        match seen.backend {
            "gtk" => {
                let (action, target) = match button {
                    None => (seen.default_action.clone(), seen.default_target.clone()),
                    Some(index) => match seen.buttons.get(index) {
                        Some(button) => (Some(button.action.clone()), button.target.clone()),
                        None => return false,
                    },
                };
                let Some(name) = action.as_deref().and_then(|a| a.strip_prefix("app.")) else {
                    return false;
                };
                let parameter = glib::Variant::array_from_iter_with_type(
                    glib::VariantTy::VARIANT,
                    target.iter().map(glib::Variant::from_variant),
                );
                let platform =
                    dbus::vardict(&[("activation-token", "selftest-token".to_variant())]);
                let call = glib::Variant::tuple_from_iter([name.to_variant(), parameter, platform]);
                dbus::call(
                    &self.connection,
                    &seen.sender,
                    app_path,
                    "org.freedesktop.Application",
                    "ActivateAction",
                    Some(&call),
                    None,
                )
                .await
                .is_ok()
            }
            _ => {
                let key = match button {
                    None => "default".to_string(),
                    Some(index) => match seen.buttons.get(index) {
                        Some(button) => button.action.clone(),
                        None => return false,
                    },
                };
                let Ok(id) = seen.key.parse::<u32>() else {
                    return false;
                };
                self.connection
                    .emit_signal(
                        Some(&seen.sender),
                        FDO_PATH,
                        FDO_NOTIFICATIONS,
                        "ActionInvoked",
                        Some(&(id, key.as_str()).to_variant()),
                    )
                    .is_ok()
            }
        }
    }

    // ---- the dock ------------------------------------------------------------------------------

    fn watch_launcher(&self) {
        let world = Rc::clone(&self.world);
        let subscription = self.connection.subscribe_to_signal(
            None,
            Some(badge::LAUNCHER_INTERFACE),
            Some("Update"),
            None,
            None,
            gio::DBusSignalFlags::NONE,
            move |signal| {
                let parameters = signal.parameters;
                if parameters.type_().as_str() != "(sa{sv})" {
                    return;
                }
                let properties = glib::VariantDict::new(Some(&parameters.child_value(1)));
                let read_bool = |key: &str| {
                    properties
                        .lookup_value(key, None)
                        .and_then(|v| v.get::<bool>())
                        .unwrap_or(false)
                };
                world.borrow_mut().launcher.push(LauncherSeen {
                    sender: signal.sender_name.to_string(),
                    uri: parameters
                        .child_value(0)
                        .str()
                        .unwrap_or_default()
                        .to_string(),
                    count: properties
                        .lookup_value("count", None)
                        .and_then(|v| v.get::<i64>())
                        .unwrap_or(-1),
                    visible: read_bool("count-visible"),
                    urgent: read_bool("urgent"),
                });
            },
        );
        self.subscriptions.borrow_mut().push(subscription);
    }

    fn last_launcher(&self) -> Option<LauncherSeen> {
        self.world().launcher.last().cloned()
    }

    // ---- the StatusNotifierWatcher ----------------------------------------------------------------

    async fn start_watcher(&self) -> bool {
        self.export(
            "watcher",
            host_registry::WATCHER_PATH,
            WATCHER_XML,
            host_registry::WATCHER_INTERFACE,
            answer_watcher,
            |world, name| match name {
                "RegisteredStatusNotifierItems" => world
                    .items
                    .iter()
                    .map(|(sender, _)| format!("{sender}{}", tray::ITEM_PATH))
                    .collect::<Vec<_>>()
                    .to_variant(),
                "IsStatusNotifierHostRegistered" => world.host.to_variant(),
                _ => 0i32.to_variant(),
            },
        ) && self.request_name(host_registry::WATCHER_BUS_NAME).await
    }

    async fn stop_watcher(&self) {
        self.release_name(host_registry::WATCHER_BUS_NAME).await;
        self.unexport("watcher");
    }

    fn watch_item(&self, app: &str) {
        let world = Rc::clone(&self.world);
        let subscription = self.connection.subscribe_to_signal(
            Some(app),
            Some(SNI_INTERFACE),
            None,
            Some(tray::ITEM_PATH),
            None,
            gio::DBusSignalFlags::NONE,
            move |signal| {
                world.borrow_mut().item_signals.push(format!(
                    "{} {}",
                    signal.signal_name,
                    signal.parameters.print(false)
                ));
            },
        );
        self.subscriptions.borrow_mut().push(subscription);
    }

    async fn item_properties(&self, app: &str) -> Option<glib::VariantDict> {
        let reply = dbus::call(
            &self.connection,
            app,
            tray::ITEM_PATH,
            dbus::PROPERTIES,
            "GetAll",
            Some(&(SNI_INTERFACE,).to_variant()),
            None,
        )
        .await
        .ok()?;
        Some(glib::VariantDict::new(Some(&reply.child_value(0))))
    }

    async fn item_call(
        &self,
        app: &str,
        path: &str,
        interface: &str,
        method: &str,
        parameters: glib::Variant,
    ) -> Result<glib::Variant, glib::Error> {
        dbus::call(
            &self.connection,
            app,
            path,
            interface,
            method,
            Some(&parameters),
            None,
        )
        .await
    }

    // ---- the portal --------------------------------------------------------------------------------

    async fn start_portal(&self) -> bool {
        let registry = self.export(
            "portal",
            portal_request::PORTAL_PATH,
            PORTAL_XML,
            host_registry::PORTAL_REGISTRY_INTERFACE,
            answer_portal,
            |_, _| ().to_variant(),
        );
        let shortcuts = self.export(
            "portal",
            portal_request::PORTAL_PATH,
            PORTAL_XML,
            hotkey::SHORTCUTS_INTERFACE,
            answer_portal,
            // Version 2: `ConfigureShortcuts` (Settings' "Change…").
            |_, _| 2u32.to_variant(),
        );
        registry && shortcuts && self.request_name(portal_request::PORTAL_BUS_NAME).await
    }

    async fn stop_portal(&self) {
        self.release_name(portal_request::PORTAL_BUS_NAME).await;
        self.unexport("portal");
        self.world.borrow_mut().sessions.clear();
    }

    async fn stop(&self) {
        self.stop_portal().await;
        self.stop_watcher().await;
        self.release_name(GTK_NOTIFICATIONS).await;
        self.release_name(FDO_NOTIFICATIONS).await;
        self.unexport("notifications");
        self.subscriptions.borrow_mut().clear();
    }
}

fn no_answer(invocation: gio::DBusMethodInvocation) {
    invocation.return_dbus_error("org.freedesktop.DBus.Error.UnknownMethod", "not faked");
}

fn answer_gtk_notifications(
    world: &Rc<RefCell<World>>,
    _: &gio::DBusConnection,
    sender: &str,
    method: &str,
    parameters: &glib::Variant,
    invocation: gio::DBusMethodInvocation,
) {
    match method {
        "AddNotification" => {
            let key = parameters
                .child_value(1)
                .str()
                .unwrap_or_default()
                .to_string();
            let notification = parameters.child_value(2);
            let dict = glib::VariantDict::new(Some(&notification));
            let text = |name: &str| dbus::dict_string(&dict, name).unwrap_or_default();
            let buttons = dict
                .lookup_value("buttons", None)
                .map(|buttons| {
                    buttons
                        .iter()
                        .map(|button| {
                            let button = glib::VariantDict::new(Some(&button));
                            Button {
                                label: dbus::dict_string(&button, "label").unwrap_or_default(),
                                action: dbus::dict_string(&button, "action").unwrap_or_default(),
                                target: button.lookup_value("target", None),
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            let seen = Seen {
                backend: "gtk",
                key: key.clone(),
                sender: sender.to_string(),
                title: text("title"),
                body: text("body"),
                priority: text("priority"),
                category: text("category"),
                default_action: dbus::dict_string(&dict, "default-action"),
                default_target: dict.lookup_value("default-action-target", None),
                buttons,
                raw: notification.print(true).to_string(),
            };
            let mut world = world.borrow_mut();
            world
                .notes
                .retain(|n| !(n.backend == "gtk" && n.key == key));
            world.notes.push(seen);
            invocation.return_value(None);
        }
        "RemoveNotification" => {
            let key = parameters
                .child_value(1)
                .str()
                .unwrap_or_default()
                .to_string();
            let mut world = world.borrow_mut();
            world
                .notes
                .retain(|n| !(n.backend == "gtk" && n.key == key));
            world.removed.push(key);
            invocation.return_value(None);
        }
        _ => no_answer(invocation),
    }
}

fn answer_fdo_notifications(
    world: &Rc<RefCell<World>>,
    connection: &gio::DBusConnection,
    sender: &str,
    method: &str,
    parameters: &glib::Variant,
    invocation: gio::DBusMethodInvocation,
) {
    match method {
        "Notify" => {
            let replaces = parameters.child_value(1).get::<u32>().unwrap_or(0);
            let actions = parameters
                .child_value(5)
                .get::<Vec<String>>()
                .unwrap_or_default();
            let hints = glib::VariantDict::new(Some(&parameters.child_value(6)));
            let mut default_action = None;
            let mut buttons = Vec::new();
            for pair in actions.chunks(2) {
                let (key, label) = (pair[0].clone(), pair.get(1).cloned().unwrap_or_default());
                if key == "default" {
                    default_action = Some(key);
                } else {
                    buttons.push(Button {
                        label,
                        action: key,
                        target: None,
                    });
                }
            }
            let mut world = world.borrow_mut();
            let id = if replaces != 0
                && world
                    .notes
                    .iter()
                    .any(|n| n.backend == "fdo" && n.key == replaces.to_string())
            {
                replaces
            } else {
                world.next_fdo_id += 1;
                world.next_fdo_id
            };
            let seen = Seen {
                backend: "fdo",
                key: id.to_string(),
                sender: sender.to_string(),
                title: parameters
                    .child_value(3)
                    .str()
                    .unwrap_or_default()
                    .to_string(),
                body: parameters
                    .child_value(4)
                    .str()
                    .unwrap_or_default()
                    .to_string(),
                priority: format!(
                    "urgency {}",
                    hints
                        .lookup_value("urgency", None)
                        .and_then(|v| v.get::<u8>())
                        .unwrap_or(1)
                ),
                category: dbus::dict_string(&hints, "category").unwrap_or_default(),
                default_action,
                default_target: None,
                buttons,
                raw: parameters.print(true).to_string(),
            };
            world
                .notes
                .retain(|n| !(n.backend == "fdo" && n.key == seen.key));
            world.notes.push(seen);
            invocation.return_value(Some(&(id,).to_variant()));
        }
        "CloseNotification" => {
            let id = parameters.child_value(0).get::<u32>().unwrap_or(0);
            {
                let mut world = world.borrow_mut();
                world
                    .notes
                    .retain(|n| !(n.backend == "fdo" && n.key == id.to_string()));
                world.removed.push(id.to_string());
            }
            invocation.return_value(None);
            let _ = connection.emit_signal(
                Some(sender),
                FDO_PATH,
                FDO_NOTIFICATIONS,
                "NotificationClosed",
                Some(&(id, 3u32).to_variant()),
            );
        }
        "GetCapabilities" => {
            invocation.return_value(Some(&(vec!["actions", "body"],).to_variant()));
        }
        "GetServerInformation" => {
            invocation.return_value(Some(
                &("selftest", "family-connect", "1", "1.2").to_variant(),
            ));
        }
        _ => no_answer(invocation),
    }
}

fn answer_watcher(
    world: &Rc<RefCell<World>>,
    connection: &gio::DBusConnection,
    sender: &str,
    method: &str,
    parameters: &glib::Variant,
    invocation: gio::DBusMethodInvocation,
) {
    match method {
        "RegisterStatusNotifierItem" => {
            let service = parameters
                .child_value(0)
                .str()
                .unwrap_or_default()
                .to_string();
            world
                .borrow_mut()
                .items
                .push((sender.to_string(), service.clone()));
            invocation.return_value(None);
            let _ = connection.emit_signal(
                None,
                host_registry::WATCHER_PATH,
                host_registry::WATCHER_INTERFACE,
                "StatusNotifierItemRegistered",
                Some(&(service,).to_variant()),
            );
        }
        "RegisterStatusNotifierHost" => invocation.return_value(None),
        _ => no_answer(invocation),
    }
}

/// `Response(code, results)` on `request` to `sender`, after the method's own reply.
fn respond_later(
    connection: &gio::DBusConnection,
    sender: &str,
    request: String,
    code: u32,
    results: glib::Variant,
) {
    let connection = connection.clone();
    let sender = sender.to_string();
    glib::timeout_add_local_once(Duration::from_millis(20), move || {
        let _ = connection.emit_signal(
            Some(&sender),
            &request,
            portal_request::REQUEST_INTERFACE,
            "Response",
            Some(&glib::Variant::tuple_from_iter([
                code.to_variant(),
                results,
            ])),
        );
    });
}

/// `a(sa{sv})` of (id, trigger) pairs.
fn shortcut_list(bound: &[(String, String)]) -> glib::Variant {
    glib::Variant::array_from_iter_with_type(
        glib::VariantTy::new("(sa{sv})").unwrap(),
        bound.iter().map(|(id, trigger)| {
            glib::Variant::tuple_from_iter([
                id.to_variant(),
                dbus::vardict(&[("trigger_description", trigger.to_variant())]),
            ])
        }),
    )
}

fn request_of(sender: &str, options: &glib::Variant) -> String {
    let options = glib::VariantDict::new(Some(options));
    let token = dbus::dict_string(&options, "handle_token").unwrap_or_default();
    portal_request::request_path(sender, &token)
}

fn object_path(path: &str) -> glib::Variant {
    glib::variant::ObjectPath::try_from(path)
        .map(|p| p.to_variant())
        .unwrap_or_else(|_| {
            glib::variant::ObjectPath::try_from("/")
                .unwrap()
                .to_variant()
        })
}

fn answer_portal(
    world: &Rc<RefCell<World>>,
    connection: &gio::DBusConnection,
    sender: &str,
    method: &str,
    parameters: &glib::Variant,
    invocation: gio::DBusMethodInvocation,
) {
    match method {
        "Register" => {
            let app = parameters
                .child_value(0)
                .str()
                .unwrap_or_default()
                .to_string();
            let again = {
                let mut world = world.borrow_mut();
                let again = world.registered_apps.contains(&app);
                world.registered_apps.push(app);
                again
            };
            if again {
                invocation
                    .return_dbus_error("org.freedesktop.portal.Error.Failed", "already registered");
            } else {
                invocation.return_value(None);
            }
        }
        "CreateSession" => {
            let options = parameters.child_value(0);
            let request = request_of(sender, &options);
            let dict = glib::VariantDict::new(Some(&options));
            let token = dbus::dict_string(&dict, "session_handle_token").unwrap_or_default();
            let session = portal_request::session_path(sender, &token);
            // The session object, closed by `Session.Close`.
            let info = dbus::interface(SESSION_XML, portal_request::SESSION_INTERFACE);
            let closing = Rc::clone(world);
            let path = session.clone();
            let registered = connection
                .register_object(&session, &info)
                .method_call(move |_, _, _, _, method, _, invocation| {
                    if method == "Close" {
                        closing
                            .borrow_mut()
                            .portal_calls
                            .push("Session.Close".to_string());
                        invocation.return_value(None);
                        let closing = Rc::clone(&closing);
                        let path = path.clone();
                        glib::idle_add_local_once(move || {
                            closing.borrow_mut().sessions.remove(&path);
                            closing.borrow_mut().bound.remove(&path);
                        });
                    } else {
                        no_answer(invocation);
                    }
                })
                .property(|_, _, _, _, _| 1u32.to_variant())
                .build();
            if let Ok(id) = registered {
                let mut exported = Exported::new(connection);
                exported.push(id);
                world
                    .borrow_mut()
                    .sessions
                    .insert(session.clone(), exported);
            }
            {
                let mut world = world.borrow_mut();
                world.portal_calls.push("CreateSession".to_string());
                world.last_session = Some(session.clone());
            }
            invocation.return_value(Some(&glib::Variant::tuple_from_iter([object_path(
                &request,
            )])));
            respond_later(
                connection,
                sender,
                request,
                0,
                dbus::vardict(&[("session_handle", session.to_variant())]),
            );
        }
        "ListShortcuts" => {
            let session = parameters
                .child_value(0)
                .str()
                .unwrap_or_default()
                .to_string();
            let request = request_of(sender, &parameters.child_value(1));
            let bound = {
                let mut world = world.borrow_mut();
                world.portal_calls.push("ListShortcuts".to_string());
                world.bound.get(&session).cloned().unwrap_or_default()
            };
            invocation.return_value(Some(&glib::Variant::tuple_from_iter([object_path(
                &request,
            )])));
            respond_later(
                connection,
                sender,
                request,
                0,
                dbus::vardict(&[("shortcuts", shortcut_list(&bound))]),
            );
        }
        "BindShortcuts" => {
            let session = parameters
                .child_value(0)
                .str()
                .unwrap_or_default()
                .to_string();
            let shortcuts = parameters.child_value(1);
            let parent = parameters
                .child_value(2)
                .str()
                .unwrap_or_default()
                .to_string();
            let request = request_of(sender, &parameters.child_value(3));
            let (code, results) = {
                let mut world = world.borrow_mut();
                world.portal_calls.push("BindShortcuts".to_string());
                world.last_bind = Some((parent, shortcuts.print(true).to_string()));
                if world.bind_answer == 0 {
                    let trigger = world.trigger.clone();
                    let bound: Vec<(String, String)> = shortcuts
                        .iter()
                        .map(|s| {
                            (
                                s.child_value(0).str().unwrap_or_default().to_string(),
                                trigger.clone(),
                            )
                        })
                        .collect();
                    world.bound.insert(session, bound.clone());
                    (0, dbus::vardict(&[("shortcuts", shortcut_list(&bound))]))
                } else {
                    (world.bind_answer, dbus::vardict(&[]))
                }
            };
            invocation.return_value(Some(&glib::Variant::tuple_from_iter([object_path(
                &request,
            )])));
            // NEVER_ANSWER: the dialog stays open until the portal leaves.
            if code != NEVER_ANSWER {
                respond_later(connection, sender, request, code, results);
            }
        }
        // GNOME's own dialog for the shortcut: the person picks Ctrl+Alt+K there, and the
        // session's shortcuts change (no Request: the method answers nothing).
        "ConfigureShortcuts" => {
            let session = parameters
                .child_value(0)
                .str()
                .unwrap_or_default()
                .to_string();
            let bound = vec![("toggle-window".to_string(), "Ctrl+Alt+K".to_string())];
            {
                let mut world = world.borrow_mut();
                world.portal_calls.push("ConfigureShortcuts".to_string());
                world.bound.insert(session.clone(), bound.clone());
            }
            invocation.return_value(None);
            let changed =
                glib::Variant::tuple_from_iter([object_path(&session), shortcut_list(&bound)]);
            let _ = connection.emit_signal(
                Some(sender),
                portal_request::PORTAL_PATH,
                hotkey::SHORTCUTS_INTERFACE,
                "ShortcutsChanged",
                Some(&changed),
            );
        }
        _ => no_answer(invocation),
    }
}

// ---- recorders standing in for the other packages' routes --------------------------------------

/// Records `open_chat` and hands everything on to the routes installed before (P1's).
struct ChatRecorder {
    before: Rc<dyn ChatRoutes>,
    seen: RefCell<Vec<i64>>,
}

impl ChatRoutes for ChatRecorder {
    fn open_chat(&self, chat_id: i64) {
        self.seen.borrow_mut().push(chat_id);
        self.before.open_chat(chat_id);
    }
    fn record_voice(&self) {
        self.before.record_voice();
    }
    fn record_video(&self) {
        self.before.record_video();
    }
    fn paste_attachment(&self) {
        self.before.paste_attachment();
    }
}

/// Records call routes and does nothing else (no Call window for a call that does not exist).
#[derive(Default)]
struct CallRecorder(RefCell<Vec<String>>);

impl CallRoutes for CallRecorder {
    fn answer(&self, call_id: &str) {
        self.0.borrow_mut().push(format!("answer {call_id}"));
    }
    fn decline(&self, call_id: &str) {
        self.0.borrow_mut().push(format!("decline {call_id}"));
    }
    fn return_to_call(&self) {
        self.0.borrow_mut().push("return".to_string());
    }
}

/// Records `show_settings` and does nothing else.
#[derive(Default)]
struct FamilyRecorder(RefCell<Vec<&'static str>>);

impl FamilyRoutes for FamilyRecorder {
    fn show_settings(&self) {
        self.0.borrow_mut().push("settings");
    }
    fn open_board(&self, family: Option<i64>) {
        let _ = family;
        self.0.borrow_mut().push("board");
    }
}

/// Counts activations of an `app.` action, alongside S0's own handler.
struct ActionCounter {
    action: gio::SimpleAction,
    handler: Option<glib::SignalHandlerId>,
    count: Rc<Cell<u32>>,
}

impl ActionCounter {
    fn new(app: &adw::Application, name: &str) -> Option<ActionCounter> {
        let action = app
            .lookup_action(name)?
            .downcast::<gio::SimpleAction>()
            .ok()?;
        let count = Rc::new(Cell::new(0));
        let counting = Rc::clone(&count);
        let handler = action.connect_activate(move |_, _| counting.set(counting.get() + 1));
        Some(ActionCounter {
            action,
            handler: Some(handler),
            count,
        })
    }

    fn count(&self) -> u32 {
        self.count.get()
    }
}

impl Drop for ActionCounter {
    fn drop(&mut self) {
        if let Some(handler) = self.handler.take() {
            self.action.disconnect(handler);
        }
    }
}

// ---- the group -----------------------------------------------------------------------------------

pub async fn run(h: &Harness) {
    if std::env::var_os(NO_KEYRING_ENV).is_some() {
        no_keyring_here(h).await;
        return;
    }
    let shell = h.shell();
    let Some(main) = shell.main_window() else {
        h.check("main_window", false, "no main window was built");
        return;
    };
    let window: gtk::Window = main.window().clone().upcast();
    let Some(app_connection) = shell.app.dbus_connection() else {
        h.check(
            "session_bus",
            false,
            "the app has no session bus (scripts/dev.sh runs one)",
        );
        return;
    };
    let app = dbus::unique_name(&app_connection).unwrap_or_default();
    let app_path = shell
        .app
        .dbus_object_path()
        .map(|p| p.to_string())
        .unwrap_or_default();
    h.check(
        "installed",
        shell.notifier.get().available() && shell.vault.get().persistent(),
        "P5's notifier and Secret Service vault are in the shell's slots",
    );

    // ---- the Secret Service ----------------------------------------------------------------------
    keyring(h).await;

    // ---- launch at login -------------------------------------------------------------------------
    autostart(h).await;

    // ---- the fakes -------------------------------------------------------------------------------
    let (fakes, notifications_up) = match EARLY.with(|early| early.borrow_mut().take()) {
        Some(early) => early,
        None => {
            let Some(fakes) = Fakes::start().await else {
                h.check("fakes", false, "no second connection to the session bus");
                return;
            };
            let up = fakes.start_notifications().await;
            (fakes, up)
        }
    };
    fakes.watch_launcher();
    fakes.watch_item(&app);
    h.check(
        "fake_notification_servers",
        notifications_up,
        "org.gtk.Notifications and org.freedesktop.Notifications owned by the fakes",
    );

    // ---- signed in, through S0: the token reaches the keyring ------------------------------------
    let chats = ensure_chats(h).await;
    h.check("signed_in", chats, format!("page {:?}", main.page()));
    if let Some(connection) = shell.connection() {
        let origin = origin_of(&connection.server);
        let held = connection.tokens.token();
        let kept = h
            .with_timeout(WAIT, async {
                // The write is spawned by `VaultedTokens::set_token`: read until it has landed.
                for _ in 0..40 {
                    let read = secret::SecretVault::new().load(&origin).await;
                    if held.is_some() && read == VaultRead::Found(held.clone().unwrap_or_default())
                    {
                        return true;
                    }
                    glib::timeout_future(Duration::from_millis(100)).await;
                }
                false
            })
            .await
            .unwrap_or(false);
        h.check(
            "sign_in_token_in_keyring",
            kept && connection.tokens.remembered(),
            format!(
                "held {}, remembered {}",
                held.is_some(),
                connection.tokens.remembered()
            ),
        );
    }

    // ---- the dock's badge ----------------------------------------------------------------------
    shell.badge.get().set_count(3);
    let three = h
        .wait_until(WAIT, || {
            fakes
                .last_launcher()
                .is_some_and(|l| l.count == 3 && l.visible && !l.urgent)
        })
        .await;
    let seen = fakes.last_launcher();
    h.check(
        "badge_count_signal",
        three
            && seen.as_ref().is_some_and(|l| {
                l.uri == "application://me.nettrash.FamilyConnect.desktop" && l.sender == app
            }),
        format!("{seen:?}"),
    );
    shell.badge.get().set_count(0);
    let cleared = h
        .wait_until(WAIT, || {
            fakes
                .last_launcher()
                .is_some_and(|l| l.count == 0 && !l.visible)
        })
        .await;
    h.check(
        "badge_cleared",
        cleared,
        format!("{:?}", fakes.last_launcher()),
    );
    let queried = fakes
        .item_call(
            &app,
            badge::LAUNCHER_PATH,
            badge::LAUNCHER_INTERFACE,
            "Query",
            ().to_variant(),
        )
        .await
        .ok()
        .map(|reply| reply.print(true).to_string());
    h.check(
        "badge_answers_query",
        queried
            .as_deref()
            .is_some_and(|q| q.contains("'count': <int64 0>")),
        format!("{queried:?}"),
    );

    // ---- the top-bar icon --------------------------------------------------------------------------
    tray_with_watcher(h, &fakes, &app, &main, &window).await;

    // ---- notifications ---------------------------------------------------------------------------
    notifications(h, &fakes, &app_path, &main, &window).await;

    // ---- the icon's watcher goes and comes back --------------------------------------------------
    watcher_vanishes(h, &fakes, &main).await;

    // ---- the global shortcut ---------------------------------------------------------------------
    hot_key(h, &fakes, &main, &window).await;

    // ---- keeping awake (GTK's inhibit is a sync call: never while a fake portal runs) ----------
    fakes.stop().await;
    let hold = keep_awake::hold(
        &shell.app,
        Some(&window),
        keep_awake::Awake::Call { video: false },
        &*shell.say,
    );
    let held = hold.held();
    drop(hold);
    h.check(
        "keep_awake_balanced",
        true,
        format!("inhibitor answered: {held} (the container has no session manager, §1.6)"),
    );

    // ---- as found ------------------------------------------------------------------------------------
    let hot_key_wanted = shell.settings.flag(keys::HOT_KEY);
    shell.hot_key.get().apply(hot_key_wanted);
    let gone = h.wait_until(WAIT, || !shell.tray.get().available()).await;
    h.check(
        "no_watcher_no_icon_close_quits",
        gone && !shell.can_hide_to_tray()
            && !tray_rules::close_hides(shell.keeps_running(), shell.tray.get().available(), false),
        "the fakes left the bus: no icon, so a close quits",
    );
    main.present();
    h.screenshot("end").await;
}

/// The token in gnome-keyring: saved, read back, cleared; and a Secret Service nobody owns.
async fn keyring(h: &Harness) {
    let vault = secret::SecretVault::new();
    let origin = "http://127.0.0.1:1";
    let saved = h
        .with_timeout(WAIT, vault.save(origin, Some("selftest-token-1".into())))
        .await
        .unwrap_or(false);
    let read = h.with_timeout(WAIT, vault.load(origin)).await;
    h.check(
        "keyring_round_trip",
        saved && read == Some(VaultRead::Found("selftest-token-1".into())) && vault.persistent(),
        format!("saved {saved}, read {}", read_kind(read.as_ref())),
    );
    let other = h.with_timeout(WAIT, vault.load("http://127.0.0.1:2")).await;
    h.check(
        "keyring_keyed_by_origin",
        other == Some(VaultRead::Empty),
        read_kind(other.as_ref()),
    );
    let cleared = h
        .with_timeout(WAIT, vault.save(origin, None))
        .await
        .unwrap_or(false);
    let after = h.with_timeout(WAIT, vault.load(origin)).await;
    h.check(
        "keyring_cleared",
        cleared && after == Some(VaultRead::Empty),
        format!("cleared {cleared}, read {}", read_kind(after.as_ref())),
    );

    no_keyring_in_a_child(h).await;
}

/// The checks the child process runs on a bus with no Secret Service (see
/// [`no_keyring_in_a_child`]).
async fn no_keyring_here(h: &Harness) {
    let shell = h.shell();
    let origin = "http://127.0.0.1:1";
    let vault: Rc<dyn TokenVault> = Rc::new(secret::SecretVault::new());
    let missing = h.with_timeout(WAIT, vault.load(origin)).await;
    let kept = h
        .with_timeout(WAIT, vault.save(origin, Some("x".into())))
        .await;
    h.check(
        "no_keyring_is_unavailable",
        missing == Some(VaultRead::Unavailable) && kept == Some(false) && !vault.persistent(),
        format!(
            "read {}, saved {kept:?}, persistent {}",
            read_kind(missing.as_ref()),
            vault.persistent()
        ),
    );
    // Through S0's layer over the INSTALLED vault: the sign-in lives in memory, not remembered —
    // what Settings' Server group reads.
    let server = ServerUrl::normalise(Some(origin)).expect("an address");
    let tokens = VaultedTokens::new(&server, shell.vault.get(), shell.spawn.clone());
    let loaded = h.with_timeout(WAIT, tokens.load()).await;
    tokens.set_token(Some("in-memory".into()));
    h.sleep(Duration::from_millis(300)).await;
    h.check(
        "no_keyring_not_remembered",
        loaded == Some(VaultRead::Unavailable)
            && !tokens.remembered()
            && !shell.vault.get().persistent()
            && tokens.token().as_deref() == Some("in-memory"),
        format!("remembered {}", tokens.remembered()),
    );
}

/// The token's other half: a desktop with NO Secret Service. libsecret 0.21 ignores the bus name
/// it is given, so the only way to meet one is another process: this binary's own self-test, on
/// a private bus whose configuration names no service folder (nothing is activatable, so
/// `org.freedesktop.secrets` is unknown), with [`NO_KEYRING_ENV`] telling its `desktop` group to
/// run [`no_keyring_here`] alone. Its report's checks are recorded here as `child_<name>`.
async fn no_keyring_in_a_child(h: &Harness) {
    let out = h.out().join("no-keyring");
    let _ = std::fs::remove_dir_all(&out);
    let conf = h.out().join("no-keyring-bus.conf");
    let prepared =
        std::fs::create_dir_all(&out).and_then(|()| std::fs::write(&conf, NO_SERVICE_BUS));
    let exe = std::env::current_exe();
    let (Ok(()), Ok(exe)) = (prepared, exe) else {
        h.check(
            "no_keyring_child",
            false,
            "could not prepare the child's folder",
        );
        return;
    };
    let launcher = gio::SubprocessLauncher::new(
        gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_MERGE,
    );
    for (variable, folder) in [
        ("XDG_CONFIG_HOME", ".config"),
        ("XDG_STATE_HOME", ".local/state"),
        ("XDG_DATA_HOME", ".local/share"),
        ("XDG_CACHE_HOME", ".cache"),
    ] {
        launcher.setenv(variable, out.join("home").join(folder), true);
    }
    launcher.setenv(NO_KEYRING_ENV, "1", true);
    let argv: Vec<std::ffi::OsString> = vec![
        "dbus-run-session".into(),
        "--config-file".into(),
        conf.into_os_string(),
        "--".into(),
        exe.into_os_string(),
        "--selftest".into(),
        out.clone().into_os_string(),
        "--only".into(),
        "desktop".into(),
        "--no-input".into(),
    ];
    let argv: Vec<&std::ffi::OsStr> = argv.iter().map(|a| a.as_os_str()).collect();
    let finished = match launcher.spawn(&argv) {
        Ok(process) => h
            .with_timeout(
                Duration::from_secs(120),
                process.communicate_utf8_future(None),
            )
            .await
            .is_some(),
        Err(error) => {
            h.check("no_keyring_child", false, dbus::error_kind(&error));
            return;
        }
    };
    let report = std::fs::read_to_string(out.join(super::REPORT_FILE_NAME)).unwrap_or_default();
    let checks: Vec<(String, bool, String)> = serde_json::from_str::<serde_json::Value>(&report)
        .ok()
        .and_then(|report| report.get("checks").and_then(|c| c.as_array()).cloned())
        .unwrap_or_default()
        .iter()
        .map(|check| {
            (
                check["name"].as_str().unwrap_or_default().to_string(),
                check["passed"].as_bool().unwrap_or(false),
                check["detail"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    let ours: Vec<&(String, bool, String)> = checks
        .iter()
        .filter(|(name, _, _)| name.starts_with("desktop.no_keyring"))
        .collect();
    h.check(
        "no_keyring_child",
        finished && ours.len() == 2,
        format!("finished {finished}, {} checks in its report", checks.len()),
    );
    for (name, passed, detail) in ours {
        let name = name.trim_start_matches("desktop.");
        h.check(&format!("child_{name}"), *passed, detail.clone());
    }
}

fn read_kind(read: Option<&VaultRead>) -> String {
    match read {
        Some(VaultRead::Found(_)) => "found".into(),
        Some(VaultRead::Empty) => "empty".into(),
        Some(VaultRead::Unavailable) => "unavailable".into(),
        None => "no answer".into(),
    }
}

/// The autostart entry written with `--background` and removed; the states another tool's edits
/// give.
async fn autostart(h: &Harness) {
    let shell = h.shell();
    let switch = shell.autostart.get();
    let dir = shell.paths.autostart_dir().to_path_buf();
    let entry = autostart_file::entry_path(&dir);
    let _ = std::fs::remove_file(&entry);
    let before = switch.state();
    h.check(
        "autostart_offered",
        before == StartupState::Disabled && startup_setting::is_offered(before),
        format!("{before:?}"),
    );
    let after = switch.set_enabled(true);
    let text = std::fs::read_to_string(&entry).unwrap_or_default();
    let exe = std::env::current_exe()
        .ok()
        .and_then(|exe| std::fs::canonicalize(exe).ok())
        .and_then(|exe| exe.to_str().map(str::to_string))
        .unwrap_or_default();
    h.check(
        "autostart_written",
        startup_setting::request_succeeded(after)
            && text.contains(&format!(
                "Exec={} --background\n",
                startup_setting::exec_argument(&exe)
            ))
            && text.contains("X-FamilyConnect-Autostart=1\n")
            && entry.starts_with(h.home()),
        format!("{after:?}, {}", entry.display()),
    );
    // Switched off in another tool: the person's choice, which the app does not undo.
    let _ = std::fs::write(
        &entry,
        text.replace(
            "X-GNOME-Autostart-enabled=true",
            "X-GNOME-Autostart-enabled=false",
        ),
    );
    let blocked = switch.state();
    let untouched = switch.set_enabled(true);
    h.check(
        "autostart_disabled_by_user",
        blocked == StartupState::DisabledByUser
            && untouched == StartupState::DisabledByUser
            && !startup_setting::is_changeable(blocked),
        format!("{blocked:?} then {untouched:?}"),
    );
    let _ = std::fs::write(&entry, &text);
    let removed = switch.set_enabled(false);
    h.check(
        "autostart_removed",
        removed == StartupState::Disabled && !entry.exists(),
        format!("{removed:?}"),
    );
}

/// The chats page, signing in to the run's fake server when the group runs alone.
async fn ensure_chats(h: &Harness) -> bool {
    let shell = h.shell();
    let Some(main) = shell.main_window() else {
        return false;
    };
    if main.page() == GatePage::Chats {
        return true;
    }
    let fake = h.fake_server();
    fake.seed_default();
    let Some(server) = ServerUrl::normalise(Some(&fake.url())) else {
        return false;
    };
    shell.open_server(server);
    h.wait_until(LONG_WAIT, || {
        matches!(main.page(), GatePage::SignIn | GatePage::Chats)
    })
    .await;
    if main.page() != GatePage::Chats {
        let Some(connection) = shell.connection() else {
            return false;
        };
        let _ = connection.session.sign_in("alice", "password").await;
    }
    h.wait_until(LONG_WAIT, || main.page() == GatePage::Chats)
        .await
}

/// The icon with a watcher: registered, its properties, the count, the menu and its clicks.
async fn tray_with_watcher(
    h: &Harness,
    fakes: &Rc<Fakes>,
    app: &str,
    main: &Rc<crate::windows::main_window::MainWindow>,
    window: &gtk::Window,
) {
    let shell = h.shell();
    h.check(
        "tray_absent_without_watcher",
        !shell.tray.get().available() && !shell.can_hide_to_tray(),
        "nothing owns org.kde.StatusNotifierWatcher yet: no icon, a close quits",
    );
    let changes = Rc::new(RefCell::new(Vec::new()));
    let recording = Rc::clone(&changes);
    let subscription = shell
        .tray
        .get()
        .on_available_changed()
        .subscribe(move |available| recording.borrow_mut().push(*available));
    let up = fakes.start_watcher().await;
    let registered = h
        .wait_until(WAIT, || {
            fakes
                .world()
                .items
                .iter()
                .any(|(sender, service)| sender == app && service == app)
                && shell.tray.get().available()
        })
        .await;
    h.check(
        "tray_registered_with_watcher",
        up && registered && changes.borrow().last() == Some(&true),
        format!(
            "items {:?}, available {}, changes {:?}",
            fakes.world().items,
            shell.tray.get().available(),
            changes.borrow()
        ),
    );
    h.check(
        "tray_close_hides_with_an_icon",
        shell.can_hide_to_tray(),
        "keep running is on and an icon exists",
    );
    // A real close now hides: the app runs on with the window in the top bar.
    window.close();
    let hidden = h.wait_until(WAIT, || !window.is_visible()).await;
    h.check(
        "tray_close_request_hides",
        hidden && !shell.quitting(),
        format!("visible {}", window.is_visible()),
    );

    let properties = fakes.item_properties(app).await;
    let text = |name: &str| {
        properties
            .as_ref()
            .and_then(|p| dbus::dict_string(p, name))
            .unwrap_or_default()
    };
    h.check(
        "tray_item_properties",
        text("Id") == "me.nettrash.FamilyConnect"
            && text("Category") == "Communications"
            && text("Status") == "Active"
            && text("IconName") == tray::ICON_NAME
            && text("Menu") == dbusmenu::MENU_PATH
            && text("XAyatanaLabelGuide") == "99+"
            && properties
                .as_ref()
                .and_then(|p| p.lookup_value("ItemIsMenu", None))
                .and_then(|v| v.get::<bool>())
                == Some(false),
        format!(
            "Id {:?} Status {:?} IconName {:?} Menu {:?} Label {:?}",
            text("Id"),
            text("Status"),
            text("IconName"),
            text("Menu"),
            text("XAyatanaLabel")
        ),
    );

    shell.tray.get().set_unread(5, "(5) Family Connect");
    let labelled = h
        .wait_until(WAIT, || {
            fakes
                .world()
                .item_signals
                .iter()
                .any(|s| s == "XAyatanaNewLabel ('5', '99+')")
        })
        .await;
    let label = fakes
        .item_properties(app)
        .await
        .and_then(|p| dbus::dict_string(&p, "XAyatanaLabel"));
    let tooltip = fakes
        .item_properties(app)
        .await
        .and_then(|p| p.lookup_value("ToolTip", None))
        .map(|t| t.print(false).to_string());
    h.check(
        "tray_count_beside_icon",
        labelled
            && label.as_deref() == Some("5")
            && tooltip
                .as_deref()
                .is_some_and(|t| t.contains("(5) Family Connect")),
        format!("label {label:?}, tooltip {tooltip:?}"),
    );
    shell.tray.get().set_unread(150, "(99+) Family Connect");
    let capped = h
        .wait_until(WAIT, || {
            fakes
                .world()
                .item_signals
                .iter()
                .any(|s| s == "XAyatanaNewLabel ('99+', '99+')")
        })
        .await;
    h.check("tray_count_capped", capped, "150 unread reads 99+");
    shell.refresh_attention();

    // The menu, as the extension reads it.
    let layout = fakes
        .item_call(
            app,
            dbusmenu::MENU_PATH,
            dbusmenu::MENU_INTERFACE,
            "GetLayout",
            (0i32, -1i32, Vec::<String>::new()).to_variant(),
        )
        .await
        .map(|reply| reply.print(false).to_string());
    let printed = layout.clone().unwrap_or_default();
    let say = &shell.say;
    use fc_core::text::string_catalog::StringCatalog;
    let labels = [
        say.get("Open Family Connect").to_string(),
        say.get("Settings…").to_string(),
        say.get("Quit Family Connect").to_string(),
    ];
    let in_order = {
        let mut at = 0;
        labels
            .iter()
            .all(|label| match printed[at..].find(label.as_str()) {
                Some(found) => {
                    at += found + label.len();
                    true
                }
                None => false,
            })
    };
    h.check(
        "tray_menu_layout",
        in_order && printed.contains("'separator'"),
        format!("{} chars of layout", printed.len()),
    );

    // "Open Family Connect" brings the hidden window back.
    let clicked = fakes
        .item_call(
            app,
            dbusmenu::MENU_PATH,
            dbusmenu::MENU_INTERFACE,
            "Event",
            glib::Variant::tuple_from_iter([
                1i32.to_variant(),
                "clicked".to_variant(),
                glib::Variant::from_variant(&0i32.to_variant()),
                0u32.to_variant(),
            ]),
        )
        .await
        .is_ok();
    let shown = h.wait_until(WAIT, || window.is_visible()).await;
    h.check(
        "tray_menu_open_shows_window",
        clicked && shown,
        format!("visible {}", window.is_visible()),
    );

    // "Settings…" is app.show-settings.
    let before = shell.family_routes.get();
    let family = Rc::new(FamilyRecorder::default());
    shell.family_routes.install(family.clone());
    let _ = fakes
        .item_call(
            app,
            dbusmenu::MENU_PATH,
            dbusmenu::MENU_INTERFACE,
            "Event",
            glib::Variant::tuple_from_iter([
                2i32.to_variant(),
                "clicked".to_variant(),
                glib::Variant::from_variant(&0i32.to_variant()),
                0u32.to_variant(),
            ]),
        )
        .await;
    let settings = h
        .wait_until(WAIT, || family.0.borrow().as_slice() == ["settings"])
        .await;
    shell.family_routes.install(before);
    h.check(
        "tray_menu_settings",
        settings,
        format!("{:?}", family.0.borrow()),
    );

    // A double click is `Activate`.
    main.hide_to_tray();
    let activated = fakes
        .item_call(
            app,
            tray::ITEM_PATH,
            SNI_INTERFACE,
            "Activate",
            (0i32, 0i32).to_variant(),
        )
        .await
        .is_ok();
    let back = h.wait_until(WAIT, || window.is_visible()).await;
    h.check(
        "tray_activate_shows_window",
        activated && back,
        format!("visible {}", window.is_visible()),
    );
    h.screenshot("tray").await;
    shell
        .tray
        .get()
        .on_available_changed()
        .unsubscribe(subscription);
}

/// Notifications: who, never what; replaced by tag; a click opens the chat; withdrawn; a ringing
/// call's buttons; a live message end to end.
async fn notifications(
    h: &Harness,
    fakes: &Rc<Fakes>,
    app_path: &str,
    main: &Rc<crate::windows::main_window::MainWindow>,
    window: &gtk::Window,
) {
    let shell = h.shell();
    let notifier = shell.notifier.get();
    let tag = NotificationRules::chat_tag(42);
    notifier.show(&Toast::new(
        &tag,
        "The Testers — Bob",
        "New message",
        Some(42),
    ));
    let first = h
        .wait_until(WAIT, || {
            fakes.note(|n| n.title == "The Testers — Bob").is_some()
        })
        .await;
    let seen = fakes.note(|n| n.title == "The Testers — Bob");
    let backend = seen.as_ref().map_or("none", |n| n.backend);
    h.check(
        "notification_shown",
        first
            && seen.as_ref().is_some_and(|n| {
                n.body == "New message"
                    && n.default_action.is_some()
                    && n.sender
                        == dbus::unique_name(&shell.app.dbus_connection().unwrap())
                            .unwrap_or_default()
            }),
        format!("backend {backend}"),
    );
    if let Some(n) = seen.as_ref().filter(|n| n.backend == "gtk") {
        h.check(
            "notification_action_and_target",
            n.key == tag
                && n.default_action.as_deref() == Some("app.open-chat")
                && n.default_target.as_ref().and_then(|t| t.get::<i64>()) == Some(42)
                && n.priority == "high"
                && n.raw.contains("me.nettrash.FamilyConnect"),
            // GLib's GNOME backend does not carry the category (measured: `raw` has none).
            format!("what GNOME Shell receives: {}", n.raw),
        );
    } else if let Some(n) = seen.as_ref() {
        h.check(
            "notification_action_and_target",
            n.default_action.as_deref() == Some("default"),
            format!("freedesktop backend: {} {}", n.priority, n.category),
        );
    }

    // A second about the same chat replaces the first.
    let count = fakes.world().notes.len();
    notifier.show(&Toast::new(
        &tag,
        "The Testers — Bob mentioned you",
        "New message",
        Some(42),
    ));
    let replaced = h
        .wait_until(WAIT, || {
            fakes
                .note(|n| n.title == "The Testers — Bob mentioned you")
                .is_some()
        })
        .await;
    h.check(
        "notification_replaced_by_tag",
        replaced
            && fakes.world().notes.len() == count
            && fakes.note(|n| n.title == "The Testers — Bob").is_none(),
        format!("{} live", fakes.world().notes.len()),
    );

    // A click opens that chat, through `app.open-chat` (held until the chats are shown).
    let before = shell.chat_routes.get();
    let routes = Rc::new(ChatRecorder {
        before: Rc::clone(&before),
        seen: RefCell::new(Vec::new()),
    });
    shell.chat_routes.install(routes.clone());
    main.hide_to_tray();
    let mentioned = fakes.note(|n| n.title == "The Testers — Bob mentioned you");
    let clicked = match &mentioned {
        Some(n) => fakes.click(n, None, app_path).await,
        None => false,
    };
    let opened = h
        .wait_until(WAIT, || routes.seen.borrow().as_slice() == [42])
        .await;
    h.check(
        "notification_click_opens_chat",
        clicked && opened && window.is_visible(),
        format!(
            "clicked {clicked}, routes {:?}, visible {}",
            routes.seen.borrow(),
            window.is_visible()
        ),
    );
    shell.chat_routes.install(before);

    // A wall of no known family: app.open-board(0).
    notifier.show(&Toast::new("board", "The Testers — Bob", "New note", None));
    let board = h
        .wait_until(WAIT, || fakes.note(|n| n.body == "New note").is_some())
        .await;
    let note = fakes.note(|n| n.body == "New note");
    h.check(
        "notification_board_target",
        board
            && note.as_ref().is_some_and(|n| {
                n.backend != "gtk"
                    || (n.default_action.as_deref() == Some("app.open-board")
                        && n.default_target.as_ref().and_then(|t| t.get::<i64>()) == Some(0))
            }),
        format!(
            "{:?}",
            note.as_ref()
                .map(|n| (&n.default_action, &n.default_target))
        ),
    );

    // Withdrawn by tag.
    notifier.withdraw(&tag);
    notifier.withdraw("board");
    let withdrawn = h
        .wait_until(WAIT, || {
            fakes
                .note(|n| n.title.starts_with("The Testers — Bob"))
                .is_none()
        })
        .await;
    h.check(
        "notification_withdrawn",
        withdrawn,
        format!("removed {:?}", fakes.world().removed),
    );

    // A ringing call: urgent, Decline then Answer for that call; the icon and the dock know.
    notifier.ring("c-1", "Bob", "Incoming call");
    let ringing = h
        .wait_until(WAIT, || {
            fakes
                .note(|n| n.title == "Bob" && n.body == "Incoming call")
                .is_some()
        })
        .await;
    let call = fakes.note(|n| n.title == "Bob" && n.body == "Incoming call");
    let say = &shell.say;
    use fc_core::text::string_catalog::StringCatalog;
    let (decline, answer) = (
        say.get("Decline").to_string(),
        say.get("Answer").to_string(),
    );
    let buttons_right = call.as_ref().is_some_and(|n| {
        n.buttons.len() == 2
            && n.buttons[0].label == decline
            && n.buttons[1].label == answer
            && match n.backend {
                "gtk" => {
                    n.buttons[0].action == "app.decline-call"
                        && n.buttons[1].action == "app.answer-call"
                        && n.buttons.iter().all(|b| {
                            b.target
                                .as_ref()
                                .and_then(|t| t.str().map(str::to_string))
                                .as_deref()
                                == Some("c-1")
                        })
                        && n.priority == "urgent"
                        && n.default_action.as_deref() == Some("app.return-to-call")
                }
                _ => {
                    n.buttons[0].action == "app.decline-call('c-1')"
                        && n.buttons[1].action == "app.answer-call('c-1')"
                        && n.priority == "urgency 2"
                }
            }
    });
    h.check(
        "ring_notification",
        ringing && buttons_right,
        format!(
            "{:?}",
            call.as_ref()
                .map(|n| (&n.buttons, &n.priority, &n.category))
        ),
    );
    let urgent = h
        .wait_until(WAIT, || fakes.last_launcher().is_some_and(|l| l.urgent))
        .await;
    let attention = h
        .wait_until(WAIT, || {
            fakes
                .world()
                .item_signals
                .iter()
                .any(|s| s == "NewStatus ('NeedsAttention',)")
        })
        .await;
    h.check(
        "ring_marks_icon_and_dock",
        urgent && attention,
        format!("dock urgent {urgent}, icon NeedsAttention {attention}"),
    );
    let before = shell.call_routes.get();
    let calls = Rc::new(CallRecorder::default());
    shell.call_routes.install(calls.clone());
    if let Some(n) = &call {
        fakes.click(n, Some(1), app_path).await;
        h.wait_until(WAIT, || !calls.0.borrow().is_empty()).await;
        fakes.click(n, Some(0), app_path).await;
        h.wait_until(WAIT, || calls.0.borrow().len() >= 2).await;
    }
    h.check(
        "ring_buttons_reach_the_call",
        calls.0.borrow().as_slice() == ["answer c-1", "decline c-1"],
        format!("{:?}", calls.0.borrow()),
    );
    shell.call_routes.install(before);
    notifier.stop_ringing("c-1");
    let stopped = h
        .wait_until(WAIT, || {
            fakes
                .note(|n| n.title == "Bob" && n.body == "Incoming call")
                .is_none()
                && fakes.last_launcher().is_some_and(|l| !l.urgent)
                && fakes
                    .world()
                    .item_signals
                    .iter()
                    .rev()
                    .find(|s| s.starts_with("NewStatus"))
                    .map(String::as_str)
                    == Some("NewStatus ('Active',)")
        })
        .await;
    h.check(
        "stop_ringing_withdraws",
        stopped,
        "notification gone, dock and icon calm",
    );

    // A live message, end to end: S0 decides, P5 shows — who, never what.
    live_message(h, fakes, main, window).await;
}

/// A message frame from the fake server while the window is hidden: the notification names who and
/// where and says "New message" — the text appears nowhere; the count reaches the icon and the dock.
async fn live_message(
    h: &Harness,
    fakes: &Rc<Fakes>,
    main: &Rc<crate::windows::main_window::MainWindow>,
    window: &gtk::Window,
) {
    let shell = h.shell();
    let Some(connection) = shell.connection() else {
        h.check("live_message", false, "no connection");
        return;
    };
    let fake = h.fake_server();
    let family = connection.session.state().family.as_ref().map(|f| f.id);
    let me = connection.session.state().me.as_ref().map(|u| u.id);
    let sender = fake
        .state()
        .users
        .iter()
        .find(|u| Some(u.id) != me && u.username == "bob")
        .map(|u| u.id);
    let (Some(family), Some(sender)) = (family, sender) else {
        h.check("live_message", false, "no family or no second member");
        return;
    };
    let chat_id = 4242;
    let mut chat = ChatDto::new(chat_id, "family", "");
    chat.family_ids = Some(vec![family]);
    fake.set_chats(family, vec![ChatRowDto::new(chat)]);
    connection.live.refresh();
    let listed = h
        .wait_until(LONG_WAIT, || {
            matches!(connection.chats.chat(chat_id), Ok(Some(_)))
        })
        .await;
    main.hide_to_tray();
    let frame = serde_json::json!({
        "type": "message",
        "message": {
            "id": 990_001, "chat_id": chat_id, "sender_id": sender,
            "body": SECRET_TEXT, "created_at": "2026-10-29T10:00:00Z"
        }
    });
    fake.send_frame(&frame.to_string());
    let raised = h
        .wait_until(LONG_WAIT, || {
            fakes
                .note(|n| n.body == "New message" && n.title.contains("Bob"))
                .is_some()
        })
        .await;
    let seen = fakes.note(|n| n.body == "New message" && n.title.contains("Bob"));
    let leaked = fakes
        .world()
        .notes
        .iter()
        .any(|n| n.raw.contains(SECRET_TEXT) || n.title.contains(SECRET_TEXT));
    h.check(
        "live_message_who_never_what",
        listed
            && raised
            && !leaked
            && seen
                .as_ref()
                .is_some_and(|n| n.title == "The Testers — Bob"),
        format!(
            "listed {listed}, raised {raised}, leaked {leaked}, title {:?}",
            seen.as_ref().map(|n| n.title.as_str())
        ),
    );
    let counted = h
        .wait_until(WAIT, || {
            fakes.last_launcher().is_some_and(|l| l.count >= 1)
                && fakes
                    .world()
                    .item_signals
                    .iter()
                    .any(|s| s == "XAyatanaNewLabel ('1', '99+')")
        })
        .await;
    h.check(
        "live_message_counts_on_icon_and_dock",
        counted,
        format!("dock {:?}", fakes.last_launcher()),
    );
    if let Some(n) = &seen {
        shell.notifier.get().withdraw(&n.key);
    }
    shell
        .notifier
        .get()
        .withdraw(&NotificationRules::chat_tag(chat_id));
    main.present();
    let _ = window;
}

/// The watcher leaves the bus: no icon, a close quits, the hidden window comes back; it returns:
/// the item registers again by itself.
async fn watcher_vanishes(
    h: &Harness,
    fakes: &Rc<Fakes>,
    main: &Rc<crate::windows::main_window::MainWindow>,
) {
    let shell = h.shell();
    main.hide_to_tray();
    let registrations = fakes.world().items.len();
    fakes.stop_watcher().await;
    let gone = h.wait_until(WAIT, || !shell.tray.get().available()).await;
    let shown = h.wait_until(WAIT, || main.window().is_visible()).await;
    h.check(
        "watcher_gone_no_icon",
        gone && !shell.can_hide_to_tray() && shown,
        format!(
            "available {}, window shown again {shown}",
            shell.tray.get().available()
        ),
    );
    let back = fakes.start_watcher().await;
    let again = h
        .wait_until(WAIT, || {
            fakes.world().items.len() > registrations && shell.tray.get().available()
        })
        .await;
    h.check(
        "watcher_back_item_registers_again",
        back && again,
        format!("{} registrations", fakes.world().items.len()),
    );
    // A watcher without a host: no icon either.
    fakes.world.borrow_mut().host = false;
    let _ = fakes.connection.emit_signal(
        None,
        host_registry::WATCHER_PATH,
        host_registry::WATCHER_INTERFACE,
        "StatusNotifierHostUnregistered",
        None,
    );
    let hostless = h.wait_until(WAIT, || !shell.tray.get().available()).await;
    fakes.world.borrow_mut().host = true;
    let _ = fakes.connection.emit_signal(
        None,
        host_registry::WATCHER_PATH,
        host_registry::WATCHER_INTERFACE,
        "StatusNotifierHostRegistered",
        None,
    );
    let hosted = h.wait_until(WAIT, || shell.tray.get().available()).await;
    h.check(
        "watcher_host_decides",
        hostless && hosted,
        format!("hostless {hostless}, hosted again {hosted}"),
    );
    fakes.stop_watcher().await;
}

/// The global shortcut against the fake portal: registered, bound to what the desktop said,
/// activated, changed, closed, dismissed, gone.
async fn hot_key(
    h: &Harness,
    fakes: &Rc<Fakes>,
    main: &Rc<crate::windows::main_window::MainWindow>,
    window: &gtk::Window,
) {
    let shell = h.shell();
    let hot_key = shell.hot_key.get();
    h.check(
        "hot_key_no_portal",
        hot_key.state() == PortalState::NoPortal,
        format!("{:?}", hot_key.state()),
    );
    let changes = Rc::new(Cell::new(0u32));
    let counting = Rc::clone(&changes);
    let subscription = hot_key
        .on_state_changed()
        .subscribe(move |_| counting.set(counting.get() + 1));
    main.present();
    let up = fakes.start_portal().await;
    hot_key.apply(true);
    let bound = h
        .wait_until(WAIT, || {
            hot_key.state() == PortalState::Bound("Ctrl+Alt+Shift+F".into())
        })
        .await;
    let calls = fakes.world().portal_calls.clone();
    let registered = fakes.world().registered_apps.first().cloned();
    let bind = fakes.world().last_bind.clone();
    h.check(
        "hot_key_bound",
        up && bound
            && changes.get() >= 1
            && registered.as_deref() == Some("me.nettrash.FamilyConnect")
            && matches!(
                (
                    calls.iter().position(|c| c == "CreateSession"),
                    calls.iter().position(|c| c == "BindShortcuts"),
                ),
                (Some(created), Some(bound)) if created < bound
            ),
        format!(
            "state {:?}, calls {calls:?}, registered {registered:?}",
            hot_key.state()
        ),
    );
    h.check(
        "hot_key_asks_preferred_trigger",
        bind.as_ref().is_some_and(|(parent, shortcuts)| {
            shortcuts.contains("'toggle-window'")
                && shortcuts.contains("'CTRL+ALT+SHIFT+f'")
                && shortcuts.contains("Show or hide Family Connect")
                && (parent.is_empty() || parent.starts_with("wayland:"))
        }),
        format!("{bind:?}"),
    );

    // Activated → app.toggle-window, with the portal's activation token.
    let toggles = ActionCounter::new(&shell.app, crate::seams::actions::TOGGLE_WINDOW.name);
    let session = fakes.world().last_session.clone().unwrap_or_default();
    let app = dbus::unique_name(&shell.app.dbus_connection().unwrap()).unwrap_or_default();
    let activated = glib::Variant::tuple_from_iter([
        object_path(&session),
        "toggle-window".to_variant(),
        1234u64.to_variant(),
        dbus::vardict(&[("activation_token", "selftest-token".to_variant())]),
    ]);
    let _ = fakes.connection.emit_signal(
        Some(&app),
        portal_request::PORTAL_PATH,
        hotkey::SHORTCUTS_INTERFACE,
        "Activated",
        Some(&activated),
    );
    let toggled = h
        .wait_until(WAIT, || toggles.as_ref().is_some_and(|t| t.count() == 1))
        .await;
    h.check(
        "hot_key_activated_toggles",
        toggled,
        format!("{:?} toggles", toggles.as_ref().map(ActionCounter::count)),
    );
    drop(toggles);
    main.present();

    // GNOME changed the binding.
    let changed = glib::Variant::tuple_from_iter([
        object_path(&session),
        shortcut_list(&[("toggle-window".into(), "Shift+Ctrl+Alt+F".into())]),
    ]);
    let _ = fakes.connection.emit_signal(
        Some(&app),
        portal_request::PORTAL_PATH,
        hotkey::SHORTCUTS_INTERFACE,
        "ShortcutsChanged",
        Some(&changed),
    );
    let followed = h
        .wait_until(WAIT, || {
            hot_key.state() == PortalState::Bound("Shift+Ctrl+Alt+F".into())
        })
        .await;
    h.check(
        "hot_key_follows_changes",
        followed,
        format!("{:?}", hot_key.state()),
    );

    // Settings' "Change…" (§13, §19): offered on a version-2 portal with a binding; asking is
    // `ConfigureShortcuts` on the session, and the trigger GNOME's dialog picked comes back. (The
    // Settings row's button is the `settings` group's check, over a stand-in.)
    let offered = hot_key.can_configure();
    hot_key.configure(Some(window));
    let configured = h
        .wait_until(WAIT, || {
            fakes
                .world()
                .portal_calls
                .iter()
                .any(|c| c == "ConfigureShortcuts")
                && hot_key.state() == PortalState::Bound("Ctrl+Alt+K".into())
        })
        .await;
    h.check(
        "hot_key_change_asks_the_desktop",
        offered && configured,
        format!(
            "offered {offered} (version 2, bound); {:?}",
            hot_key.state()
        ),
    );

    // Off: the session is closed.
    hot_key.apply(false);
    let closed = h
        .wait_until(WAIT, || {
            fakes
                .world()
                .portal_calls
                .iter()
                .any(|c| c == "Session.Close")
                && hot_key.state() == PortalState::NotBound
        })
        .await;
    h.check(
        "hot_key_off_closes_session",
        closed,
        format!("{:?}", hot_key.state()),
    );

    // Dismissed in GNOME's dialog: not bound.
    fakes.world.borrow_mut().bind_answer = 1;
    let binds = fakes
        .world()
        .portal_calls
        .iter()
        .filter(|c| *c == "BindShortcuts")
        .count();
    hot_key.apply(true);
    let dismissed = h
        .wait_until(WAIT, || {
            fakes
                .world()
                .portal_calls
                .iter()
                .filter(|c| *c == "BindShortcuts")
                .count()
                > binds
        })
        .await;
    h.sleep(Duration::from_millis(300)).await;
    h.check(
        "hot_key_dismissed_not_bound",
        dismissed && hot_key.state() == PortalState::NotBound,
        format!("{:?}", hot_key.state()),
    );

    // The portal leaves while its dialog is open: the request ends, the state is NoPortal, and a
    // portal that comes back binds — nothing waits for an answer that cannot come.
    hot_key.apply(false);
    h.wait_until(WAIT, || hot_key.state() == PortalState::NotBound)
        .await;
    fakes.world.borrow_mut().bind_answer = NEVER_ANSWER;
    let binds = fakes
        .world()
        .portal_calls
        .iter()
        .filter(|c| *c == "BindShortcuts")
        .count();
    hot_key.apply(true);
    let open = h
        .wait_until(WAIT, || {
            fakes
                .world()
                .portal_calls
                .iter()
                .filter(|c| *c == "BindShortcuts")
                .count()
                > binds
        })
        .await;
    fakes.stop_portal().await;
    let gone = h
        .wait_until(WAIT, || hot_key.state() == PortalState::NoPortal)
        .await;
    h.check(
        "hot_key_portal_gone",
        open && gone,
        format!("dialog open {open}, {:?}", hot_key.state()),
    );
    fakes.world.borrow_mut().bind_answer = 0;
    let back = fakes.start_portal().await;
    let rebound = h
        .wait_until(WAIT, || {
            hot_key.state() == PortalState::Bound("Ctrl+Alt+Shift+F".into())
        })
        .await;
    h.check(
        "hot_key_portal_back_binds_again",
        back && rebound,
        format!("{:?}", hot_key.state()),
    );
    fakes.stop_portal().await;
    h.wait_until(WAIT, || hot_key.state() == PortalState::NoPortal)
        .await;
    hot_key.on_state_changed().unsubscribe(subscription);
    let _ = window;
}
