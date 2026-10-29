//! The two registries on the host the desktop modules depend on.
//!
//! 1. **The portals' host-app registry** (`org.freedesktop.host.portal.Registry`, xdg-desktop-portal
//!    1.19.4+, shell-design §1.7 and §19 step 1): a host — unsandboxed — app has no app id the
//!    portal can derive, and portal 1.21 REFUSES GlobalShortcuts to such an app (#1817). GTK did not
//!    register in the container (§1.6), so [`register_app`] does, once per connection; an "already
//!    registered" answer and a portal too old to have the registry are both fine.
//!
//! 2. **The StatusNotifierWatcher and its hosts** (§16.2, §16.4): an icon in the top bar exists
//!    only while something owns `org.kde.StatusNotifierWatcher` (Ubuntu's AppIndicator extension)
//!    AND a host is registered with it — the watcher's `IsStatusNotifierHostRegistered`, followed
//!    through `StatusNotifierHostRegistered` / `…Unregistered`. [`IconHost`] watches both, asks its
//!    owner (the tray) to register the item every time the watcher gets a new owner — GNOME Shell
//!    restarted, the extension switched off and on — and reports whether an icon can be shown NOW.
//!    That answer is what `TrayState::available` is: a fact, not a guess (§16.4: with no host a
//!    close QUITS — nettrash's decision).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use fc_core::rt::LocalBoxFuture;
use gio::prelude::*;

use super::dbus::{self, NameWatch};
use crate::app::APP_ID;

/// The portals' host-app registry interface (on the portal's bus name and path).
pub const PORTAL_REGISTRY_INTERFACE: &str = "org.freedesktop.host.portal.Registry";

/// The StatusNotifierWatcher's well-known name, path and interface.
pub const WATCHER_BUS_NAME: &str = "org.kde.StatusNotifierWatcher";
pub const WATCHER_PATH: &str = "/StatusNotifierWatcher";
pub const WATCHER_INTERFACE: &str = "org.kde.StatusNotifierWatcher";

thread_local! {
    /// The connection (unique name) the app id was registered on.
    static REGISTERED_ON: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// What the registry answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppRegistration {
    /// Registered now, or earlier on this connection.
    Registered,
    /// The portal is older than the registry (before 1.19.4): it derives what it can.
    NoRegistry,
    /// Refused — typically "already registered" (by GTK, or a racing call). Not an obstacle.
    Refused,
    /// No portal answered at all.
    NoPortal,
}

/// `Register("me.nettrash.FamilyConnect", {})` on `portal`, once per connection.
pub async fn register_app(connection: &gio::DBusConnection, portal: &str) -> AppRegistration {
    let unique = dbus::unique_name(connection);
    if unique.is_some() && REGISTERED_ON.with(|done| *done.borrow() == unique) {
        return AppRegistration::Registered;
    }
    let options = dbus::vardict(&[]);
    let parameters = glib::Variant::tuple_from_iter([APP_ID.to_variant(), options]);
    let answer = dbus::call(
        connection,
        portal,
        super::portal_request::PORTAL_PATH,
        PORTAL_REGISTRY_INTERFACE,
        "Register",
        Some(&parameters),
        None,
    )
    .await;
    let registration = match answer {
        Ok(_) => AppRegistration::Registered,
        Err(error) => match error.kind::<gio::DBusError>() {
            Some(
                gio::DBusError::UnknownMethod
                | gio::DBusError::UnknownInterface
                | gio::DBusError::UnknownObject,
            ) => AppRegistration::NoRegistry,
            _ if dbus::is_absent(&error) => AppRegistration::NoPortal,
            _ => {
                crate::fc_debug!("portal: registry refused ({})", dbus::error_kind(&error));
                AppRegistration::Refused
            }
        },
    };
    if matches!(
        registration,
        AppRegistration::Registered | AppRegistration::Refused
    ) {
        REGISTERED_ON.with(|done| *done.borrow_mut() = unique);
    }
    crate::fc_info!("portal: app id registration {registration:?}");
    registration
}

/// Whether an icon can be shown: a watcher, our item registered with it, and a host — when the
/// watcher does not say (an old watcher without the property) a host is assumed.
pub fn icon_possible(watcher: bool, registered: bool, host: Option<bool>) -> bool {
    watcher && registered && host.unwrap_or(true)
}

/// Registers the item with a watcher's owner; answers whether it was accepted.
pub type RegisterItem = Rc<dyn Fn() -> LocalBoxFuture<'static, bool>>;

/// Watches the StatusNotifierWatcher and its hosts (see the module's comment).
pub struct IconHost {
    inner: Rc<Inner>,
}

struct Inner {
    connection: gio::DBusConnection,
    register: RegisterItem,
    changed: Box<dyn Fn(bool)>,
    /// The watcher's owner now.
    owner: RefCell<Option<String>>,
    registered: Cell<bool>,
    host: Cell<Option<bool>>,
    /// The answer last reported (`None`: nothing reported yet).
    reported: Cell<Option<bool>>,
    /// Bumped on every owner change, so a late answer about an old owner is dropped.
    generation: Cell<u64>,
    subscriptions: RefCell<Vec<gio::SignalSubscription>>,
    watch: RefCell<Option<NameWatch>>,
}

impl IconHost {
    /// Starts watching. `register` puts the item on the watcher (called for every new owner);
    /// `changed(possible)` is called with every new answer, the first one included.
    pub fn new(
        connection: &gio::DBusConnection,
        register: RegisterItem,
        changed: impl Fn(bool) + 'static,
    ) -> IconHost {
        let inner = Rc::new(Inner {
            connection: connection.clone(),
            register,
            changed: Box::new(changed),
            owner: RefCell::new(None),
            registered: Cell::new(false),
            host: Cell::new(None),
            reported: Cell::new(None),
            generation: Cell::new(0),
            subscriptions: RefCell::new(Vec::new()),
            watch: RefCell::new(None),
        });
        let appeared = Rc::downgrade(&inner);
        let vanished = Rc::downgrade(&inner);
        let watch = NameWatch::new(
            connection,
            WATCHER_BUS_NAME,
            false,
            move |owner| {
                if let Some(inner) = appeared.upgrade() {
                    Inner::appeared(&inner, owner);
                }
            },
            move || {
                if let Some(inner) = vanished.upgrade() {
                    inner.vanished();
                }
            },
        );
        *inner.watch.borrow_mut() = Some(watch);
        IconHost { inner }
    }
}

impl Drop for IconHost {
    fn drop(&mut self) {
        self.inner.watch.borrow_mut().take();
        self.inner.subscriptions.borrow_mut().clear();
    }
}

impl Inner {
    fn appeared(this: &Rc<Inner>, owner: &str) {
        let generation = this.generation.get() + 1;
        this.generation.set(generation);
        *this.owner.borrow_mut() = Some(owner.to_string());
        this.registered.set(false);
        this.host.set(None);
        crate::fc_info!("tray: a StatusNotifierWatcher appeared");
        let host_changed = {
            let weak = Rc::downgrade(this);
            move |_: gio::DBusSignalRef| {
                if let Some(inner) = weak.upgrade() {
                    Inner::read_host(&inner, generation);
                }
            }
        };
        let subscriptions = [
            "StatusNotifierHostRegistered",
            "StatusNotifierHostUnregistered",
        ]
        .into_iter()
        .map(|signal| {
            this.connection.subscribe_to_signal(
                Some(owner),
                Some(WATCHER_INTERFACE),
                Some(signal),
                Some(WATCHER_PATH),
                None,
                gio::DBusSignalFlags::NONE,
                host_changed.clone(),
            )
        })
        .collect();
        *this.subscriptions.borrow_mut() = subscriptions;
        let weak = Rc::downgrade(this);
        let registering = (this.register)();
        glib::spawn_future_local(async move {
            let accepted = registering.await;
            let Some(inner) = weak.upgrade() else {
                return;
            };
            if inner.generation.get() != generation {
                return;
            }
            inner.registered.set(accepted);
            Inner::read_host(&inner, generation);
        });
    }

    fn vanished(&self) {
        self.generation.set(self.generation.get() + 1);
        if self.owner.borrow_mut().take().is_some() {
            crate::fc_info!("tray: the StatusNotifierWatcher went away");
        }
        self.registered.set(false);
        self.host.set(None);
        self.subscriptions.borrow_mut().clear();
        self.report();
    }

    /// Reads `IsStatusNotifierHostRegistered`, then reports.
    fn read_host(this: &Rc<Inner>, generation: u64) {
        let weak = Rc::downgrade(this);
        let connection = this.connection.clone();
        glib::spawn_future_local(async move {
            let read = dbus::get_property(
                &connection,
                WATCHER_BUS_NAME,
                WATCHER_PATH,
                WATCHER_INTERFACE,
                "IsStatusNotifierHostRegistered",
            )
            .await;
            let Some(inner) = weak.upgrade() else {
                return;
            };
            if inner.generation.get() != generation {
                return;
            }
            inner
                .host
                .set(read.ok().and_then(|value| value.get::<bool>()));
            inner.report();
        });
    }

    fn report(&self) {
        let possible = icon_possible(
            self.owner.borrow().is_some(),
            self.registered.get(),
            self.host.get(),
        );
        if self.reported.replace(Some(possible)) != Some(possible) {
            crate::fc_info!("tray: an icon can be shown: {possible}");
            (self.changed)(possible);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_icon_needs_a_watcher_our_registration_and_a_host() {
        assert!(icon_possible(true, true, Some(true)));
        // An old watcher that does not say: a host is assumed.
        assert!(icon_possible(true, true, None));
        assert!(!icon_possible(true, true, Some(false)));
        assert!(!icon_possible(true, false, Some(true)));
        assert!(!icon_possible(false, true, Some(true)));
        assert!(!icon_possible(false, false, None));
    }
}
