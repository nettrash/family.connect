//! The dock's badge (shell-design §18): `com.canonical.Unity.LauncherEntry.Update(s, a{sv})`,
//! emitted on the app's own connection from `/me/nettrash/FamilyConnect/launcher` with
//! `application://me.nettrash.FamilyConnect.desktop` and `count`, `count-visible` (count > 0) and
//! `urgent` (a call rings). Ubuntu Dock subscribes to the signal from any sender, keys the entry by
//! sender and clears it when the app leaves the bus (measured from its source, §1.7) — so there is
//! nothing to clean up at quit. Vanilla GNOME's dash draws no badge (D19); the window title and the
//! top-bar label still count.
//!
//! The entry also answers `Query()` with what it last said, so a dock that restarts while the app
//! runs (libunity's method) can ask instead of waiting for the next change.

use std::cell::Cell;

use gio::prelude::*;

use super::dbus::{self, Exported};
use crate::app::APP_ID;
use crate::seams::desktop::BadgeState;

/// The interface the dock listens to.
pub const LAUNCHER_INTERFACE: &str = "com.canonical.Unity.LauncherEntry";
/// The app's own object path for it.
pub const LAUNCHER_PATH: &str = "/me/nettrash/FamilyConnect/launcher";

const LAUNCHER_XML: &str = r#"<node>
  <interface name="com.canonical.Unity.LauncherEntry">
    <method name="Query">
      <arg type="s" name="app_uri" direction="out"/>
      <arg type="a{sv}" name="properties" direction="out"/>
    </method>
    <signal name="Update">
      <arg type="s" name="app_uri"/>
      <arg type="a{sv}" name="properties"/>
    </signal>
  </interface>
</node>"#;

/// `application://me.nettrash.FamilyConnect.desktop` — the desktop entry the dock shows.
pub fn app_uri() -> String {
    format!("application://{APP_ID}.desktop")
}

/// What the entry says: the count, whether it shows, and whether the icon asks for attention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Entry {
    pub count: i64,
    pub urgent: bool,
}

impl Entry {
    /// `Update`'s parameters, `(sa{sv})`.
    pub fn parameters(&self) -> glib::Variant {
        glib::Variant::tuple_from_iter([app_uri().to_variant(), self.properties()])
    }

    /// The entry's `a{sv}`.
    pub fn properties(&self) -> glib::Variant {
        dbus::vardict(&[
            ("count", self.count.to_variant()),
            ("count-visible", (self.count > 0).to_variant()),
            ("urgent", self.urgent.to_variant()),
        ])
    }
}

/// The badge (see the module's comment).
pub struct Badge {
    connection: Option<gio::DBusConnection>,
    entry: std::rc::Rc<Cell<Entry>>,
    /// What was last emitted (`None`: nothing yet).
    sent: Cell<Option<Entry>>,
    _exported: Option<Exported>,
}

impl Badge {
    pub fn new(connection: Option<gio::DBusConnection>) -> Badge {
        let entry = std::rc::Rc::new(Cell::new(Entry::default()));
        let exported = connection.as_ref().and_then(|connection| {
            let info = dbus::interface(LAUNCHER_XML, LAUNCHER_INTERFACE);
            let answered = std::rc::Rc::clone(&entry);
            let registered = connection
                .register_object(LAUNCHER_PATH, &info)
                .method_call(move |_, _, _, _, method, _, invocation| match method {
                    "Query" => invocation.return_value(Some(&answered.get().parameters())),
                    _ => invocation.return_dbus_error(
                        "org.freedesktop.DBus.Error.UnknownMethod",
                        "no such method",
                    ),
                })
                .build();
            match registered {
                Ok(id) => {
                    let mut exported = Exported::new(connection);
                    exported.push(id);
                    Some(exported)
                }
                Err(error) => {
                    crate::fc_warn!(
                        "badge: the launcher entry could not be exported ({})",
                        dbus::error_kind(&error)
                    );
                    None
                }
            }
        });
        Badge {
            connection,
            entry,
            sent: Cell::new(None),
            _exported: exported,
        }
    }

    /// What the dock was last told.
    #[cfg(test)]
    pub fn entry(&self) -> Entry {
        self.entry.get()
    }

    fn update(&self, edit: impl FnOnce(&mut Entry)) {
        let mut entry = self.entry.get();
        edit(&mut entry);
        self.entry.set(entry);
        if self.sent.get() == Some(entry) {
            return;
        }
        let Some(connection) = self.connection.as_ref() else {
            return;
        };
        self.sent.set(Some(entry));
        crate::fc_debug!("badge: count {}, urgent {}", entry.count, entry.urgent);
        dbus::emit(
            connection,
            None,
            LAUNCHER_PATH,
            LAUNCHER_INTERFACE,
            "Update",
            &entry.parameters(),
        );
    }
}

impl BadgeState for Badge {
    fn set_count(&self, unread: i64) {
        self.update(|entry| entry.count = unread.max(0));
    }

    fn set_urgent(&self, urgent: bool) {
        self.update(|entry| entry.urgent = urgent);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_entry_is_what_ubuntu_dock_reads() {
        assert_eq!(app_uri(), "application://me.nettrash.FamilyConnect.desktop");
        let entry = Entry {
            count: 3,
            urgent: false,
        };
        let parameters = entry.parameters();
        assert_eq!(parameters.type_().as_str(), "(sa{sv})");
        let properties = glib::VariantDict::new(Some(&parameters.child_value(1)));
        assert_eq!(
            properties
                .lookup_value("count", None)
                .and_then(|v| v.get::<i64>()),
            Some(3)
        );
        assert_eq!(
            properties
                .lookup_value("count-visible", None)
                .and_then(|v| v.get::<bool>()),
            Some(true)
        );
        let quiet = Entry::default().properties();
        let quiet = glib::VariantDict::new(Some(&quiet));
        assert_eq!(
            quiet
                .lookup_value("count-visible", None)
                .and_then(|v| v.get::<bool>()),
            Some(false)
        );
        assert!(
            dbus::interface(LAUNCHER_XML, LAUNCHER_INTERFACE)
                .lookup_method("Query")
                .is_some()
        );
    }

    #[test]
    fn without_a_bus_the_badge_remembers_and_says_nothing() {
        let badge = Badge::new(None);
        badge.set_count(-4);
        assert_eq!(badge.entry().count, 0);
        badge.set_count(7);
        badge.set_urgent(true);
        assert_eq!(
            badge.entry(),
            Entry {
                count: 7,
                urgent: true
            }
        );
    }
}
