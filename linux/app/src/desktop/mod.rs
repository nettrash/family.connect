//! P5 — the desktop (shell-design §15–§22, W.2): everything that talks to the desktop over D-Bus,
//! on the application's own session-bus connection through gio's GDBus (§0 "D-Bus": no zbus).
//!
//! | Module | What | Implements (`seams`) |
//! | --- | --- | --- |
//! | [`dbus`] | calls, signals, exported objects, name watches, `a{sv}` builders | — |
//! | [`portal_request`] | the portals' Request/Response with handle tokens, cancellation, parent windows | — |
//! | [`host_registry`] | the portals' host-app registry; the StatusNotifierWatcher and its hosts | — |
//! | [`notifier`] | GNotification: who, never what; ringing calls with Decline/Answer (§15, §14.4) | `Notifier` |
//! | [`tray`], [`dbusmenu`], [`dbusmenu_layout`] | the top-bar icon: StatusNotifierItem + `com.canonical.dbusmenu` (§16) | `TrayState` |
//! | [`badge`] | `com.canonical.Unity.LauncherEntry` (§18) | `BadgeState` |
//! | [`autostart`], [`autostart_file`] | the XDG autostart entry with `--background` (§17) | `AutostartState` |
//! | [`hotkey`] | the GlobalShortcuts portal (§19) | `HotKeyState` |
//! | [`secret`] | the token in the Secret Service through libsecret (§20) | `TokenVault` |
//! | [`keep_awake`] | `gtk::Application::inhibit`, balanced (§10.9, §14.5) — for P3 and P6 | — |
//!
//! [`install`] fills the shell's slots; S0 calls it from `app::install_packages` in `startup`,
//! after the application registered on the bus and before the main window is built.

pub mod autostart;
pub mod autostart_file;
pub mod badge;
pub mod dbus;
pub mod dbusmenu;
pub mod dbusmenu_layout;
pub mod host_registry;
pub mod hotkey;
pub mod keep_awake;
pub mod notifier;
pub mod portal_request;
pub mod secret;
pub mod tray;

use std::rc::Rc;

use crate::shell::Shell;

/// Puts P5's services into the shell's slots: the Secret Service vault, the notifier, the top-bar
/// icon, the dock's badge, the global shortcut and launch at login.
pub fn install(shell: &Rc<Shell>) {
    let connection = dbus::session(&shell.app);
    shell.vault.install(Rc::new(secret::SecretVault::new()));
    shell
        .notifier
        .install(Rc::new(notifier::DesktopNotifier::new(
            &shell.app,
            Rc::clone(&shell.say),
        )));
    shell.tray.install(tray::Tray::new(
        &shell.app,
        connection.clone(),
        Rc::clone(&shell.say),
    ));
    shell
        .badge
        .install(Rc::new(badge::Badge::new(connection.clone())));
    shell.hot_key.install(hotkey::HotKey::new(
        &shell.app,
        connection,
        Rc::clone(&shell.say),
    ));
    shell.autostart.install(Rc::new(autostart::Autostart::new(
        shell.paths.autostart_dir(),
    )));
}
