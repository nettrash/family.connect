//! The global shortcut (shell-design §19, D6) through the XDG GlobalShortcuts portal — Wayland has
//! no app-registered global keys; the portal is the only door and GNOME (48+) owns the decision.
//!
//! **The sequence**, all over GDBus on `org.freedesktop.portal.Desktop`:
//!
//! 1. `org.freedesktop.host.portal.Registry.Register("me.nettrash.FamilyConnect", {})` — portal
//!    1.21 refuses GlobalShortcuts to a host app without an app id (`host_registry`).
//! 2. `CreateSession({handle_token, session_handle_token})` → `Response` (`portal_request`).
//! 3. `ListShortcuts(session)` — a binding GNOME kept from an earlier run is used as it is, with no
//!    dialog; otherwise `BindShortcuts(session, [("toggle-window", {description, preferred_trigger:
//!    "CTRL+ALT+SHIFT+f"})], parent_window, {handle_token})`. GNOME may show its own dialog and the
//!    person may pick another chord: the trigger that comes back (`trigger_description`) is what
//!    Settings shows (`PortalState::Bound`).
//! 4. `Activated(session, "toggle-window", timestamp, {activation_token})` →
//!    `gtk::Window::set_startup_id(token)` on the main window, then `app.toggle-window` — so the
//!    window really comes forward on Wayland. `ShortcutsChanged` updates the state.
//!
//! Switched off: `Session.Close`. The portal's name is watched (with auto-start: an installed
//! portal not yet running is not "absent"): when it goes, the state is `NoPortal`; when it comes
//! back, what the switch asks is applied again. Everywhere else — no portal, another desktop:
//! `family-connect --toggle` (S0) and Settings' sentence say how (`global_hot_key_rules`).
//!
//! Requests are applied one at a time, in order: a switch flipped twice while a dialog is open
//! ends where the switch ends.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use fc_core::rt::Event;
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::global_hot_key_rules::{self, PREFERRED_TRIGGER, PortalState, SHORTCUT_ID};

use super::dbus::{self, NameWatch};
use super::host_registry;
use super::portal_request::{self, PORTAL_PATH, Response};
use crate::seams::actions;
use crate::seams::desktop::HotKeyState;
use crate::services::catalogue::Catalogue;
use crate::shell::Shell;

/// The portal interface.
pub const SHORTCUTS_INTERFACE: &str = "org.freedesktop.portal.GlobalShortcuts";

/// `a(sa{sv})` read as (id, `trigger_description`) pairs.
pub fn read_shortcuts(shortcuts: &glib::Variant) -> Vec<(String, Option<String>)> {
    if shortcuts.type_().as_str() != "a(sa{sv})" {
        return Vec::new();
    }
    shortcuts
        .iter()
        .map(|shortcut| {
            let id = shortcut
                .child_value(0)
                .str()
                .unwrap_or_default()
                .to_string();
            let options = glib::VariantDict::new(Some(&shortcut.child_value(1)));
            (id, dbus::dict_string(&options, "trigger_description"))
        })
        .collect()
}

/// The state a `shortcuts` list says (`PortalState::from_shortcuts`).
pub fn state_from(shortcuts: &glib::Variant) -> PortalState {
    let read = read_shortcuts(shortcuts);
    let pairs: Vec<(&str, Option<&str>)> = read
        .iter()
        .map(|(id, trigger)| (id.as_str(), trigger.as_deref()))
        .collect();
    PortalState::from_shortcuts(&pairs)
}

/// The one shortcut the app asks for, `a(sa{sv})`.
pub fn wanted_shortcuts(say: &dyn StringCatalog) -> glib::Variant {
    let options = dbus::vardict(&[
        (
            "description",
            global_hot_key_rules::description(say).to_variant(),
        ),
        ("preferred_trigger", PREFERRED_TRIGGER.to_variant()),
    ]);
    glib::Variant::array_from_iter_with_type(
        glib::VariantTy::new("(sa{sv})").expect("a valid type"),
        [glib::Variant::tuple_from_iter([
            SHORTCUT_ID.to_variant(),
            options,
        ])],
    )
}

/// An object path variant, when `path` is one.
fn object_path(path: &str) -> Option<glib::Variant> {
    glib::variant::ObjectPath::try_from(path)
        .ok()
        .map(|path| path.to_variant())
}

/// The global shortcut (see the module's comment).
pub struct HotKey {
    inner: Rc<Inner>,
}

struct Inner {
    app: adw::Application,
    connection: Option<gio::DBusConnection>,
    portal: String,
    say: Rc<Catalogue>,
    state: RefCell<PortalState>,
    changed: Event<()>,
    /// What the switch asks (`None`: not asked yet).
    wanted: Cell<Option<bool>>,
    /// A request arrived that the worker has not applied yet.
    dirty: Cell<bool>,
    running: Cell<bool>,
    session: RefCell<Option<Session>>,
    watch: RefCell<Option<NameWatch>>,
    /// The portal's GlobalShortcuts `version` (0: not read, or no portal): `ConfigureShortcuts`,
    /// Settings' "Change…", is version 2's.
    version: Cell<u32>,
}

/// An open portal session and what listens to it.
struct Session {
    handle: String,
    _signals: Vec<gio::SignalSubscription>,
}

impl HotKey {
    /// The shortcut over the desktop's portal.
    pub fn new(
        app: &adw::Application,
        connection: Option<gio::DBusConnection>,
        say: Rc<Catalogue>,
    ) -> Rc<HotKey> {
        HotKey::on_portal(app, connection, say, portal_request::PORTAL_BUS_NAME)
    }

    fn on_portal(
        app: &adw::Application,
        connection: Option<gio::DBusConnection>,
        say: Rc<Catalogue>,
        portal: &str,
    ) -> Rc<HotKey> {
        let inner = Rc::new(Inner {
            app: app.clone(),
            connection: connection.clone(),
            portal: portal.to_string(),
            say,
            state: RefCell::new(PortalState::NoPortal),
            changed: Event::new(),
            wanted: Cell::new(None),
            dirty: Cell::new(false),
            running: Cell::new(false),
            session: RefCell::new(None),
            watch: RefCell::new(None),
            version: Cell::new(0),
        });
        if let Some(connection) = connection {
            let appeared = Rc::downgrade(&inner);
            let vanished = Rc::downgrade(&inner);
            let watch = NameWatch::new(
                &connection,
                portal,
                true,
                move |_| {
                    if let Some(inner) = appeared.upgrade() {
                        crate::fc_info!("hot key: a portal is on the bus");
                        Inner::request(&inner, inner.wanted.get().unwrap_or(false));
                    }
                },
                move || {
                    if let Some(inner) = vanished.upgrade() {
                        crate::fc_info!("hot key: no portal on the bus");
                        inner.session.borrow_mut().take();
                        inner.set_state(PortalState::NoPortal);
                    }
                },
            );
            *inner.watch.borrow_mut() = Some(watch);
        }
        Rc::new(HotKey { inner })
    }
}

impl Inner {
    fn set_state(&self, state: PortalState) {
        let kind = match &state {
            PortalState::Bound(_) => "bound",
            PortalState::NotBound => "not bound",
            PortalState::NoPortal => "no portal",
        };
        if self.state.replace(state.clone()) != state {
            crate::fc_info!("hot key: {kind}");
            self.changed.emit(&());
        }
    }

    /// Asks for `enabled`; the worker applies requests one at a time, the newest last.
    fn request(this: &Rc<Inner>, enabled: bool) {
        this.wanted.set(Some(enabled));
        this.dirty.set(true);
        if this.running.replace(true) {
            return;
        }
        let inner = Rc::clone(this);
        glib::spawn_future_local(async move {
            while inner.dirty.replace(false) {
                let enabled = inner.wanted.get().unwrap_or(false);
                inner.apply_now(enabled).await;
            }
            inner.running.set(false);
        });
    }

    async fn apply_now(self: &Rc<Self>, enabled: bool) {
        let Some(connection) = self.connection.clone() else {
            self.set_state(PortalState::NoPortal);
            return;
        };
        if !enabled {
            if let Some(session) = self.session.borrow_mut().take() {
                portal_request::close_session(&connection, &self.portal, &session.handle);
                crate::fc_info!("hot key: session closed");
            }
            let state = if self.has_portal(&connection).await {
                PortalState::NotBound
            } else {
                PortalState::NoPortal
            };
            self.set_state(state);
            return;
        }
        if !self.has_portal(&connection).await {
            self.set_state(PortalState::NoPortal);
            return;
        }
        host_registry::register_app(&connection, &self.portal).await;
        let existing = self
            .session
            .borrow()
            .as_ref()
            .map(|session| session.handle.clone());
        let handle = match existing {
            Some(handle) => handle,
            None => match self.create_session(&connection).await {
                Ok(handle) => handle,
                Err(state) => {
                    self.set_state(state);
                    return;
                }
            },
        };
        let Some(session) = object_path(&handle) else {
            self.set_state(PortalState::NotBound);
            return;
        };
        // A binding GNOME kept is used as it is — no dialog.
        let listed = portal_request::request(
            &connection,
            &self.portal,
            SHORTCUTS_INTERFACE,
            "ListShortcuts",
            |token| {
                glib::Variant::tuple_from_iter([
                    session.clone(),
                    dbus::vardict(&[("handle_token", token.to_variant())]),
                ])
            },
        )
        .await;
        if let Err(error) = &listed
            && error.is_absent()
        {
            self.set_state(PortalState::NoPortal);
            return;
        }
        if let Ok(response) = &listed
            && let Some(results) = response.results()
            && let Some(shortcuts) = results.lookup_value("shortcuts", None)
            && let bound @ PortalState::Bound(_) = state_from(&shortcuts)
        {
            self.set_state(bound);
            return;
        }
        let window = Shell::try_get()
            .and_then(|shell| shell.main_window())
            .map(|main| main.window().clone().upcast::<gtk::Window>());
        let parent = portal_request::parent_window(window.as_ref()).await;
        let wanted = wanted_shortcuts(&*self.say);
        let bound = portal_request::request(
            &connection,
            &self.portal,
            SHORTCUTS_INTERFACE,
            "BindShortcuts",
            |token| {
                glib::Variant::tuple_from_iter([
                    session.clone(),
                    wanted,
                    parent.identifier().to_variant(),
                    dbus::vardict(&[("handle_token", token.to_variant())]),
                ])
            },
        )
        .await;
        drop(parent);
        let state = match bound {
            Ok(Response::Success(results)) => {
                let results = glib::VariantDict::new(Some(&results));
                results
                    .lookup_value("shortcuts", None)
                    .map_or(PortalState::NotBound, |shortcuts| state_from(&shortcuts))
            }
            Ok(other) => {
                crate::fc_info!("hot key: the desktop's dialog ended ({})", other.name());
                PortalState::NotBound
            }
            Err(error) if error.is_absent() => PortalState::NoPortal,
            Err(error) => {
                crate::fc_warn!("hot key: BindShortcuts failed ({})", error.kind());
                PortalState::NotBound
            }
        };
        // The switch went off while the dialog was open: the worker's next turn closes it.
        self.set_state(state);
    }

    /// Whether the portal has GlobalShortcuts (its `version` property answers — kept: version 2
    /// has `ConfigureShortcuts`).
    async fn has_portal(&self, connection: &gio::DBusConnection) -> bool {
        let version = dbus::get_property(
            connection,
            &self.portal,
            PORTAL_PATH,
            SHORTCUTS_INTERFACE,
            "version",
        )
        .await;
        self.version.set(
            version
                .as_ref()
                .ok()
                .and_then(|v| v.get::<u32>())
                .unwrap_or(0),
        );
        version.is_ok()
    }

    /// Settings' "Change…": `ConfigureShortcuts(session, parent_window, {})` — the desktop's own
    /// dialog for the shortcut; it answers nothing, and a new trigger comes back as
    /// `ShortcutsChanged`. Only with a session open and a version-2 portal.
    fn configure(this: &Rc<Inner>, parent: Option<gtk::Window>) {
        let (Some(connection), Some(handle)) = (
            this.connection.clone(),
            this.session
                .borrow()
                .as_ref()
                .map(|session| session.handle.clone()),
        ) else {
            return;
        };
        if this.version.get() < 2 {
            return;
        }
        let Some(session) = object_path(&handle) else {
            return;
        };
        let portal = this.portal.clone();
        glib::spawn_future_local(async move {
            let parent = portal_request::parent_window(parent.as_ref()).await;
            let arguments = glib::Variant::tuple_from_iter([
                session,
                parent.identifier().to_variant(),
                dbus::vardict(&[]),
            ]);
            match dbus::call(
                &connection,
                &portal,
                PORTAL_PATH,
                SHORTCUTS_INTERFACE,
                "ConfigureShortcuts",
                Some(&arguments),
                None,
            )
            .await
            {
                Ok(_) => crate::fc_info!("hot key: the desktop's shortcut dialog asked for"),
                Err(error) => crate::fc_info!(
                    "hot key: ConfigureShortcuts failed ({})",
                    dbus::error_kind(&error)
                ),
            }
        });
    }

    /// `CreateSession`, and the signals of the session it made. Answers the session's handle, or
    /// the state to show when there is none.
    async fn create_session(
        self: &Rc<Self>,
        connection: &gio::DBusConnection,
    ) -> Result<String, PortalState> {
        let session_token = portal_request::new_token("s");
        let predicted = dbus::unique_name(connection)
            .map(|unique| portal_request::session_path(&unique, &session_token))
            .unwrap_or_default();
        let created = portal_request::request(
            connection,
            &self.portal,
            SHORTCUTS_INTERFACE,
            "CreateSession",
            |token| {
                glib::Variant::tuple_from_iter([dbus::vardict(&[
                    ("handle_token", token.to_variant()),
                    ("session_handle_token", session_token.to_variant()),
                ])])
            },
        )
        .await;
        let handle = match created {
            Ok(Response::Success(results)) => {
                let results = glib::VariantDict::new(Some(&results));
                dbus::dict_string(&results, "session_handle").unwrap_or(predicted)
            }
            Ok(other) => {
                crate::fc_info!("hot key: no session ({})", other.name());
                return Err(PortalState::NotBound);
            }
            Err(error) if error.is_absent() => return Err(PortalState::NoPortal),
            Err(error) => {
                crate::fc_warn!("hot key: CreateSession failed ({})", error.kind());
                return Err(PortalState::NotBound);
            }
        };
        crate::fc_info!("hot key: session created");
        let signals = self.listen(connection, &handle);
        *self.session.borrow_mut() = Some(Session {
            handle: handle.clone(),
            _signals: signals,
        });
        Ok(handle)
    }

    /// `Activated`, `ShortcutsChanged` and `Session.Closed` for the session `handle`.
    fn listen(
        self: &Rc<Self>,
        connection: &gio::DBusConnection,
        handle: &str,
    ) -> Vec<gio::SignalSubscription> {
        let activated = {
            let weak = Rc::downgrade(self);
            let handle = handle.to_string();
            connection.subscribe_to_signal(
                None,
                Some(SHORTCUTS_INTERFACE),
                Some("Activated"),
                Some(PORTAL_PATH),
                None,
                gio::DBusSignalFlags::NONE,
                move |signal| {
                    let parameters = signal.parameters;
                    if parameters.type_().as_str() != "(osta{sv})"
                        || parameters.child_value(0).str() != Some(handle.as_str())
                        || parameters.child_value(1).str() != Some(SHORTCUT_ID)
                    {
                        return;
                    }
                    let options = glib::VariantDict::new(Some(&parameters.child_value(3)));
                    let token = dbus::dict_string(&options, "activation_token");
                    if let Some(inner) = weak.upgrade() {
                        inner.activated(token);
                    }
                },
            )
        };
        let changed = {
            let weak = Rc::downgrade(self);
            let handle = handle.to_string();
            connection.subscribe_to_signal(
                None,
                Some(SHORTCUTS_INTERFACE),
                Some("ShortcutsChanged"),
                Some(PORTAL_PATH),
                None,
                gio::DBusSignalFlags::NONE,
                move |signal| {
                    let parameters = signal.parameters;
                    if parameters.type_().as_str() != "(oa(sa{sv}))"
                        || parameters.child_value(0).str() != Some(handle.as_str())
                    {
                        return;
                    }
                    if let Some(inner) = weak.upgrade() {
                        inner.set_state(state_from(&parameters.child_value(1)));
                    }
                },
            )
        };
        let closed = {
            let weak = Rc::downgrade(self);
            let handle_closed = handle.to_string();
            portal_request::on_session_closed(connection, handle, move || {
                if let Some(inner) = weak.upgrade() {
                    let ours = inner
                        .session
                        .borrow()
                        .as_ref()
                        .is_some_and(|session| session.handle == handle_closed);
                    if ours {
                        crate::fc_info!("hot key: the portal closed the session");
                        inner.session.borrow_mut().take();
                        inner.set_state(PortalState::NotBound);
                    }
                }
            })
        };
        vec![activated, changed, closed]
    }

    /// The shortcut was pressed: the activation token first, then `app.toggle-window`.
    fn activated(&self, token: Option<String>) {
        crate::fc_info!("hot key: activated");
        if let Some(token) = token.filter(|token| !token.is_empty())
            && let Some(main) = Shell::try_get().and_then(|shell| shell.main_window())
        {
            main.window().set_startup_id(&token);
        }
        self.app.activate_action(actions::TOGGLE_WINDOW.name, None);
    }
}

impl HotKeyState for HotKey {
    fn state(&self) -> PortalState {
        self.inner.state.borrow().clone()
    }

    fn on_state_changed(&self) -> &Event<()> {
        &self.inner.changed
    }

    fn apply(&self, enabled: bool) {
        Inner::request(&self.inner, enabled);
    }

    fn configure(&self, parent: Option<&gtk::Window>) {
        Inner::configure(&self.inner, parent.cloned());
    }

    fn can_configure(&self) -> bool {
        matches!(*self.inner.state.borrow(), PortalState::Bound(_))
            && self.inner.version.get() >= 2
            && self.inner.session.borrow().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fc_core::text::string_catalog::EnglishCatalog;

    fn shortcuts(rows: &[(&str, Option<&str>)]) -> glib::Variant {
        let rows: Vec<glib::Variant> = rows
            .iter()
            .map(|(id, trigger)| {
                let options = match trigger {
                    Some(trigger) => {
                        dbus::vardict(&[("trigger_description", trigger.to_variant())])
                    }
                    None => dbus::vardict(&[]),
                };
                glib::Variant::tuple_from_iter([id.to_variant(), options])
            })
            .collect();
        glib::Variant::array_from_iter_with_type(glib::VariantTy::new("(sa{sv})").unwrap(), rows)
    }

    #[test]
    fn the_bound_trigger_is_what_the_desktop_wrote() {
        assert_eq!(
            state_from(&shortcuts(&[("toggle-window", Some("Ctrl+Alt+Shift+F"))])),
            PortalState::Bound("Ctrl+Alt+Shift+F".into())
        );
        assert_eq!(
            state_from(&shortcuts(&[
                ("other", Some("Ctrl+K")),
                ("toggle-window", None)
            ])),
            PortalState::NotBound
        );
        assert_eq!(state_from(&shortcuts(&[])), PortalState::NotBound);
        // Anything that is not a shortcut list binds nothing.
        assert_eq!(state_from(&5u32.to_variant()), PortalState::NotBound);
        assert_eq!(
            read_shortcuts(&shortcuts(&[("toggle-window", Some(" Shift+Ctrl+Alt+F "))])),
            vec![(
                "toggle-window".to_string(),
                Some(" Shift+Ctrl+Alt+F ".to_string())
            )]
        );
    }

    #[test]
    fn the_app_asks_for_one_shortcut_with_the_preferred_trigger() {
        let wanted = wanted_shortcuts(&EnglishCatalog::new());
        assert_eq!(wanted.type_().as_str(), "a(sa{sv})");
        assert_eq!(wanted.n_children(), 1);
        let shortcut = wanted.child_value(0);
        assert_eq!(shortcut.child_value(0).str(), Some("toggle-window"));
        let options = glib::VariantDict::new(Some(&shortcut.child_value(1)));
        assert_eq!(
            dbus::dict_string(&options, "preferred_trigger").as_deref(),
            Some("CTRL+ALT+SHIFT+f")
        );
        assert_eq!(
            dbus::dict_string(&options, "description").as_deref(),
            Some("Show or hide Family Connect")
        );
        assert!(object_path("/org/freedesktop/portal/desktop/session/1_2/fc_s1_0").is_some());
        assert!(object_path("not a path").is_none());
    }
}
