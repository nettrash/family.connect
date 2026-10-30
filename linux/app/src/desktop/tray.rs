//! The top-bar icon (shell-design §16): a StatusNotifierItem at `/StatusNotifierItem` and its
//! `com.canonical.dbusmenu` menu at `/MenuBar` ([`super::dbusmenu`]), exported over GDBus on the
//! app's own connection and registered with `org.kde.StatusNotifierWatcher` — Ubuntu's
//! AppIndicator extension draws it. No `ksni`: it would bring zbus and an executor beside GTK's,
//! and it has no Ayatana label for the count (§16.2).
//!
//! | | Mac (#80) | Linux |
//! | --- | --- | --- |
//! | Count | text beside the icon, "99+" | `XAyatanaLabel` = `tray_rules::count_text`, guide "99+" |
//! | Tooltip | "(3) Family Connect" | `ToolTip` = ("", [], "Family Connect", "(3) Family Connect") |
//! | VoiceOver | "Family Connect, 3 unread" | `IconAccessibleDesc` (libayatana-appindicator's), the same sentence |
//! | Click | opens the window | a double click (`Activate`) or middle click (`SecondaryActivate`) activates the app; a single click opens the menu — the extension's own behaviour (§1.7, D3) |
//! | Menu | Open Family Connect · Settings… · — · Quit Family Connect | the same three |
//! | Ringing | — | `Status` = `NeedsAttention` |
//!
//! **Whether an icon exists** is [`IconHost`](super::host_registry::IconHost)'s fact: a watcher
//! owns its name, our item is registered with it, and it has a host. It decides the close rule
//! (`tray_rules::close_hides`: with no icon a close QUITS — nettrash's decision), a `--background`
//! start and Settings' "Keep running" row. Before the bus has answered at all (the first
//! milliseconds of a launch) the icon counts as possible, so a login start does not flash its
//! window; the first answer that says otherwise is raised as a change, and S0's window shows itself
//! (and its two-second backstop, §3.7, holds either way).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use fc_core::rt::Event;
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::tray_rules;

use super::dbus::{self, Exported};
use super::dbusmenu::{self, MENU_PATH};
use super::dbusmenu_layout::{self, Chosen, MenuLayout};
use super::host_registry::{
    IconHost, RegisterItem, WATCHER_BUS_NAME, WATCHER_INTERFACE, WATCHER_PATH,
};
use crate::app::APP_ID;
use crate::seams::desktop::TrayState;
use crate::services::catalogue::Catalogue;
use crate::services::paths;
use crate::shell::{BRAND, Shell};

/// Where the item lives (the path a watcher assumes for a bus name).
pub const ITEM_PATH: &str = "/StatusNotifierItem";
/// The item's interface.
pub const ITEM_INTERFACE: &str = "org.kde.StatusNotifierItem";
/// The icon: the app's symbolic icon, which the package ships (P7).
pub const ICON_NAME: &str = "me.nettrash.FamilyConnect-symbolic";
/// The item's category.
pub const CATEGORY: &str = "Communications";

/// The item's introspection: the KDE specification, the Ayatana additions (label, guide,
/// accessible descriptions, `XAyatanaSecondaryActivate`) and `ProvideXdgActivationToken`.
pub const ITEM_XML: &str = r#"<node>
  <interface name="org.kde.StatusNotifierItem">
    <property name="Category" type="s" access="read"/>
    <property name="Id" type="s" access="read"/>
    <property name="Title" type="s" access="read"/>
    <property name="Status" type="s" access="read"/>
    <property name="WindowId" type="i" access="read"/>
    <property name="IconThemePath" type="s" access="read"/>
    <property name="IconName" type="s" access="read"/>
    <property name="IconPixmap" type="a(iiay)" access="read"/>
    <property name="OverlayIconName" type="s" access="read"/>
    <property name="OverlayIconPixmap" type="a(iiay)" access="read"/>
    <property name="AttentionIconName" type="s" access="read"/>
    <property name="AttentionIconPixmap" type="a(iiay)" access="read"/>
    <property name="AttentionMovieName" type="s" access="read"/>
    <property name="ToolTip" type="(sa(iiay)ss)" access="read"/>
    <property name="ItemIsMenu" type="b" access="read"/>
    <property name="Menu" type="o" access="read"/>
    <property name="XAyatanaLabel" type="s" access="read"/>
    <property name="XAyatanaLabelGuide" type="s" access="read"/>
    <property name="XAyatanaOrderingIndex" type="u" access="read"/>
    <property name="IconAccessibleDesc" type="s" access="read"/>
    <property name="AttentionAccessibleDesc" type="s" access="read"/>
    <method name="ContextMenu">
      <arg name="x" type="i" direction="in"/>
      <arg name="y" type="i" direction="in"/>
    </method>
    <method name="Activate">
      <arg name="x" type="i" direction="in"/>
      <arg name="y" type="i" direction="in"/>
    </method>
    <method name="SecondaryActivate">
      <arg name="x" type="i" direction="in"/>
      <arg name="y" type="i" direction="in"/>
    </method>
    <method name="XAyatanaSecondaryActivate">
      <arg name="timestamp" type="u" direction="in"/>
    </method>
    <method name="Scroll">
      <arg name="delta" type="i" direction="in"/>
      <arg name="orientation" type="s" direction="in"/>
    </method>
    <method name="ProvideXdgActivationToken">
      <arg name="token" type="s" direction="in"/>
    </method>
    <signal name="NewTitle"/>
    <signal name="NewIcon"/>
    <signal name="NewAttentionIcon"/>
    <signal name="NewOverlayIcon"/>
    <signal name="NewToolTip"/>
    <signal name="NewStatus">
      <arg name="status" type="s"/>
    </signal>
    <signal name="NewIconThemePath">
      <arg name="icon_theme_path" type="s"/>
    </signal>
    <signal name="XAyatanaNewLabel">
      <arg name="label" type="s"/>
      <arg name="guide" type="s"/>
    </signal>
  </interface>
</node>"#;

/// What the item shows — pure, so every property is tested without a bus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemLook {
    pub unread: i64,
    /// `NotificationRules::window_title`: "(3) Family Connect".
    pub window_title: String,
    /// The Mac's VoiceOver sentence ("Family Connect, 3 unread").
    pub accessible: String,
    pub ringing: bool,
    /// Taken away on the way out (§3.8).
    pub withdrawn: bool,
}

impl Default for ItemLook {
    fn default() -> Self {
        ItemLook {
            unread: 0,
            window_title: BRAND.to_string(),
            accessible: BRAND.to_string(),
            ringing: false,
            withdrawn: false,
        }
    }
}

impl ItemLook {
    /// `XAyatanaLabel`: the count, empty at zero.
    pub fn label(&self) -> String {
        tray_rules::count_text(self.unread).unwrap_or_default()
    }

    /// `Status`.
    pub fn status(&self) -> &'static str {
        if self.withdrawn {
            "Passive"
        } else if self.ringing {
            "NeedsAttention"
        } else {
            "Active"
        }
    }

    /// `ToolTip`, `(sa(iiay)ss)`: no icon of its own, the brand, and the title with the count
    /// while there is one.
    pub fn tooltip(&self) -> glib::Variant {
        let description = if self.unread > 0 {
            self.window_title.as_str()
        } else {
            ""
        };
        glib::Variant::tuple_from_iter([
            "".to_variant(),
            no_pixmaps(),
            BRAND.to_variant(),
            description.to_variant(),
        ])
    }

    /// One property of the item.
    pub fn property(&self, name: &str, icon_theme_path: &str) -> Option<glib::Variant> {
        Some(match name {
            "Category" => CATEGORY.to_variant(),
            "Id" => APP_ID.to_variant(),
            "Title" => BRAND.to_variant(),
            "Status" => self.status().to_variant(),
            "WindowId" => 0i32.to_variant(),
            "IconThemePath" => icon_theme_path.to_variant(),
            "IconName" => ICON_NAME.to_variant(),
            "IconPixmap" | "OverlayIconPixmap" | "AttentionIconPixmap" => no_pixmaps(),
            "OverlayIconName" | "AttentionIconName" | "AttentionMovieName" => "".to_variant(),
            "ToolTip" => self.tooltip(),
            "ItemIsMenu" => false.to_variant(),
            "Menu" => glib::variant::ObjectPath::try_from(MENU_PATH)
                .expect("a valid object path")
                .to_variant(),
            "XAyatanaLabel" => self.label().to_variant(),
            "XAyatanaLabelGuide" => tray_rules::COUNT_GUIDE.to_variant(),
            "XAyatanaOrderingIndex" => 0u32.to_variant(),
            "IconAccessibleDesc" | "AttentionAccessibleDesc" => self.accessible.to_variant(),
            _ => return None,
        })
    }
}

/// An empty `a(iiay)`.
fn no_pixmaps() -> glib::Variant {
    Vec::<(i32, i32, Vec<u8>)>::new().to_variant()
}

/// The Mac's VoiceOver label for the icon.
pub fn accessible_label(say: &dyn StringCatalog, unread: i64) -> String {
    if unread > 0 {
        say.format("Family Connect, %lld unread", &[&unread])
    } else {
        BRAND.to_string()
    }
}

/// The top-bar icon (see the module's comment).
pub struct Tray {
    inner: Rc<Inner>,
}

struct Inner {
    app: adw::Application,
    connection: Option<gio::DBusConnection>,
    say: Rc<Catalogue>,
    look: RefCell<ItemLook>,
    icon_theme_path: String,
    available: Cell<bool>,
    available_changed: Event<bool>,
    /// A token the host gave (`ProvideXdgActivationToken`) for the next activation.
    activation_token: RefCell<Option<String>>,
    exported: RefCell<Option<Exported>>,
    host: RefCell<Option<IconHost>>,
}

impl Tray {
    /// Exports the item and its menu on `connection` and starts following the watcher. With no
    /// connection there is no icon, ever.
    pub fn new(
        app: &adw::Application,
        connection: Option<gio::DBusConnection>,
        say: Rc<Catalogue>,
    ) -> Rc<Tray> {
        let icon_theme_path = std::env::current_exe()
            .ok()
            .and_then(|exe| paths::icons_beside(&exe))
            .and_then(|icons| icons.to_str().map(str::to_string))
            .unwrap_or_default();
        let inner = Rc::new(Inner {
            app: app.clone(),
            connection: connection.clone(),
            say,
            look: RefCell::new(ItemLook::default()),
            icon_theme_path,
            available: Cell::new(false),
            available_changed: Event::new(),
            activation_token: RefCell::new(None),
            exported: RefCell::new(None),
            host: RefCell::new(None),
        });
        let tray = Rc::new(Tray { inner });
        if let Some(connection) = connection {
            match Inner::export(&tray.inner, &connection) {
                Ok(exported) => {
                    *tray.inner.exported.borrow_mut() = Some(exported);
                    // Unknown until the bus answers: possible (see the module's comment).
                    tray.inner.available.set(true);
                    Inner::follow_watcher(&tray.inner, &connection);
                }
                Err(error) => crate::fc_warn!(
                    "tray: the item could not be exported ({})",
                    dbus::error_kind(&error)
                ),
            }
        } else {
            crate::fc_info!("tray: no session bus, so no icon");
        }
        tray
    }
}

impl Inner {
    fn export(this: &Rc<Inner>, connection: &gio::DBusConnection) -> Result<Exported, glib::Error> {
        let info = dbus::interface(ITEM_XML, ITEM_INTERFACE);
        let methods = Rc::downgrade(this);
        let properties = Rc::downgrade(this);
        let item = connection
            .register_object(ITEM_PATH, &info)
            .method_call(move |_, _, _, _, method, parameters, invocation| {
                match methods.upgrade() {
                    Some(inner) => inner.answer(method, &parameters, invocation),
                    None => invocation.return_value(None),
                }
            })
            .property(move |_, _, _, _, name| {
                properties
                    .upgrade()
                    .and_then(|inner| inner.look.borrow().property(name, &inner.icon_theme_path))
                    .unwrap_or_else(|| "".to_variant())
            })
            .build()?;
        let menu = Rc::new(MenuLayout::new(&*this.say));
        let chosen = {
            let weak = Rc::downgrade(this);
            Rc::new(move |item| {
                if let Some(inner) = weak.upgrade() {
                    match dbusmenu_layout::chosen(item) {
                        Chosen::Activate => inner.open(),
                        Chosen::Action(name) => inner.app.activate_action(name, None),
                    }
                }
            })
        };
        let mut exported = dbusmenu::export(connection, menu, chosen)?;
        exported.push(item);
        Ok(exported)
    }

    fn follow_watcher(this: &Rc<Inner>, connection: &gio::DBusConnection) {
        let register: RegisterItem = {
            let connection = connection.clone();
            Rc::new(move || {
                let connection = connection.clone();
                Box::pin(async move {
                    let Some(unique) = dbus::unique_name(&connection) else {
                        return false;
                    };
                    let answer = dbus::call(
                        &connection,
                        WATCHER_BUS_NAME,
                        WATCHER_PATH,
                        WATCHER_INTERFACE,
                        "RegisterStatusNotifierItem",
                        Some(&(unique,).to_variant()),
                        None,
                    )
                    .await;
                    match answer {
                        Ok(_) => {
                            crate::fc_info!("tray: registered with the watcher");
                            true
                        }
                        Err(error) => {
                            crate::fc_warn!(
                                "tray: the watcher refused the item ({})",
                                dbus::error_kind(&error)
                            );
                            false
                        }
                    }
                })
            })
        };
        let weak = Rc::downgrade(this);
        let host = IconHost::new(connection, register, move |possible| {
            if let Some(inner) = weak.upgrade()
                && inner.available.replace(possible) != possible
            {
                inner.available_changed.emit(&possible);
            }
        });
        *this.host.borrow_mut() = Some(host);
    }

    fn answer(
        &self,
        method: &str,
        parameters: &glib::Variant,
        invocation: gio::DBusMethodInvocation,
    ) {
        match method {
            "Activate" | "SecondaryActivate" | "XAyatanaSecondaryActivate" => {
                invocation.return_value(None);
                crate::fc_info!("tray: {method}");
                self.open();
            }
            "ContextMenu" | "Scroll" => invocation.return_value(None),
            "ProvideXdgActivationToken" => {
                let token = parameters.child_value(0).str().map(str::to_string);
                *self.activation_token.borrow_mut() = token.filter(|t| !t.is_empty());
                invocation.return_value(None);
            }
            _ => invocation
                .return_dbus_error("org.freedesktop.DBus.Error.UnknownMethod", "no such method"),
        }
    }

    /// The window forward: the host's activation token for Wayland focus when it gave one, then
    /// `activate` (S0's: present the main window).
    fn open(&self) {
        let token = self.activation_token.borrow_mut().take();
        let app = self.app.clone();
        glib::idle_add_local_once(move || {
            if let Some(token) = token
                && let Some(main) = Shell::try_get().and_then(|shell| shell.main_window())
            {
                main.window().set_startup_id(&token);
            }
            app.activate();
        });
    }

    /// Changes the look and tells the host what changed.
    fn change(&self, edit: impl FnOnce(&mut ItemLook)) {
        let (before, after) = {
            let mut look = self.look.borrow_mut();
            let before = look.clone();
            edit(&mut look);
            (before, look.clone())
        };
        let Some(connection) = self.connection.as_ref() else {
            return;
        };
        if self.exported.borrow().is_none() {
            return;
        }
        let mut changed: Vec<(&str, glib::Variant)> = Vec::new();
        if before.label() != after.label() {
            let label = after.label();
            dbus::emit(
                connection,
                None,
                ITEM_PATH,
                ITEM_INTERFACE,
                "XAyatanaNewLabel",
                &(label.as_str(), tray_rules::COUNT_GUIDE).to_variant(),
            );
            changed.push(("XAyatanaLabel", label.to_variant()));
        }
        if before.tooltip() != after.tooltip() {
            dbus::emit(
                connection,
                None,
                ITEM_PATH,
                ITEM_INTERFACE,
                "NewToolTip",
                &().to_variant(),
            );
            changed.push(("ToolTip", after.tooltip()));
        }
        if before.accessible != after.accessible {
            changed.push(("IconAccessibleDesc", after.accessible.to_variant()));
        }
        if before.status() != after.status() {
            dbus::emit(
                connection,
                None,
                ITEM_PATH,
                ITEM_INTERFACE,
                "NewStatus",
                &(after.status(),).to_variant(),
            );
            changed.push(("Status", after.status().to_variant()));
        }
        if !changed.is_empty() {
            dbus::emit_properties_changed(connection, ITEM_PATH, ITEM_INTERFACE, &changed);
        }
    }
}

impl TrayState for Tray {
    fn available(&self) -> bool {
        self.inner.available.get()
    }

    fn on_available_changed(&self) -> &Event<bool> {
        &self.inner.available_changed
    }

    fn set_unread(&self, unread: i64, title: &str) {
        let unread = unread.max(0);
        let accessible = accessible_label(&*self.inner.say, unread);
        self.inner.change(|look| {
            look.unread = unread;
            look.window_title = title.to_string();
            look.accessible = accessible;
        });
    }

    fn set_ringing(&self, ringing: bool) {
        self.inner.change(|look| look.ringing = ringing);
    }

    fn withdraw(&self) {
        if self.inner.look.borrow().withdrawn {
            return;
        }
        crate::fc_info!("tray: withdrawn");
        // Passive first, so a host hides it at once; then off the bus. No availability change is
        // raised: the app is on its way out, and a window must not come up for it.
        self.inner.change(|look| look.withdrawn = true);
        self.inner.host.borrow_mut().take();
        self.inner.exported.borrow_mut().take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fc_core::text::string_catalog::EnglishCatalog;

    #[test]
    fn the_introspection_parses() {
        let info = dbus::interface(ITEM_XML, ITEM_INTERFACE);
        for method in [
            "Activate",
            "SecondaryActivate",
            "XAyatanaSecondaryActivate",
            "ContextMenu",
        ] {
            assert!(info.lookup_method(method).is_some(), "{method}");
        }
        assert!(info.lookup_signal("XAyatanaNewLabel").is_some());
        // Every property the XML declares has an answer of the declared type.
        let look = ItemLook::default();
        let declared: Vec<(&str, &str)> = ITEM_XML
            .split("<property name=\"")
            .skip(1)
            .map(|rest| {
                let (name, rest) = rest.split_once('"').unwrap();
                let kind = rest
                    .split("type=\"")
                    .nth(1)
                    .unwrap()
                    .split('"')
                    .next()
                    .unwrap();
                (name, kind)
            })
            .collect();
        assert_eq!(declared.len(), 21);
        for (name, kind) in declared {
            assert!(info.lookup_property(name).is_some(), "{name}");
            let value = look.property(name, "").expect(name);
            assert_eq!(value.type_().as_str(), kind, "{name}");
        }
        assert!(look.property("Nope", "").is_none());
    }

    #[test]
    fn the_count_is_the_macs_beside_the_icon() {
        let mut look = ItemLook::default();
        assert_eq!(look.label(), "");
        look.unread = 3;
        assert_eq!(look.label(), "3");
        look.unread = 150;
        assert_eq!(look.label(), "99+");
        assert_eq!(
            look.property("XAyatanaLabelGuide", "").unwrap().str(),
            Some("99+")
        );
    }

    #[test]
    fn the_status_says_ringing_and_withdrawn() {
        let mut look = ItemLook::default();
        assert_eq!(look.status(), "Active");
        look.ringing = true;
        assert_eq!(look.status(), "NeedsAttention");
        look.withdrawn = true;
        assert_eq!(look.status(), "Passive");
    }

    #[test]
    fn the_tooltip_carries_the_title_with_the_count_only_while_there_is_one() {
        let mut look = ItemLook::default();
        let quiet = look.tooltip();
        assert_eq!(quiet.type_().as_str(), "(sa(iiay)ss)");
        assert_eq!(quiet.child_value(2).str(), Some("Family Connect"));
        assert_eq!(quiet.child_value(3).str(), Some(""));
        look.unread = 3;
        look.window_title = "(3) Family Connect".into();
        assert_eq!(
            look.tooltip().child_value(3).str(),
            Some("(3) Family Connect")
        );
    }

    #[test]
    fn the_accessible_label_is_the_macs_sentence() {
        let say = EnglishCatalog::new();
        assert_eq!(accessible_label(&say, 0), "Family Connect");
        assert_eq!(accessible_label(&say, 3), "Family Connect, 3 unread");
    }
}
