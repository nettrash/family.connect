//! `Notifier`, `TrayState`, `BadgeState`, `HotKeyState`, `AutostartState` (FROZEN, W.3) — P5's
//! desktop integration (§15–§19), and what S0's composition and P4's Settings rows ask of it.
//!
//! **Who decides what.** S0's composition (`shell::Attention`) decides WHEN — fc-logic's
//! `NotificationRules` on live frames only, the count from the cache, the window's title — and
//! calls these. P5 decides HOW: GNotification, the StatusNotifierItem and dbusmenu, the
//! LauncherEntry signal, the GlobalShortcuts portal, the autostart file. Clicks and shortcuts come
//! back to the app as its ACTIONS (`seams::actions`): a notification's default action is
//! `app.open-chat(x)` / `app.open-board(x)`, its buttons `app.answer-call(s)` /
//! `app.decline-call(s)`; the tray's double click and "Open Family Connect" activate the
//! application (`gio::Application::activate`), "Settings…" `app.show-settings`, "Quit Family
//! Connect" `app.quit`; the portal's `Activated` is `app.toggle-window` (after
//! `gtk::Window::set_startup_id` with its activation token).

use fc_core::rt::Event;
use fc_logic::global_hot_key_rules::PortalState;
use fc_logic::notifications::Toast;
use fc_logic::startup_setting::StartupState;

/// Notifications (§15) — "who, never what".
pub trait Notifier {
    /// Whether this app can raise notifications at all. GLib always can try; `false` only when P5
    /// knows better. `NotificationRules::set_wanted` is given `available() && notify setting`.
    fn available(&self) -> bool {
        true
    }

    /// Raise `toast`: `title` and `body` as given (the body is never message text), priority high,
    /// category `im.received` (GLib's GNOME backend, `org.gtk.Notifications`, carries no category
    /// at all — measured by P5; only the freedesktop backend sends the hint), the app's icon,
    /// `send_notification(Some(&toast.tag), …)` so a second one about the same chat replaces the
    /// first. The default action and target are
    /// `fc_logic::toast_arguments::for_toast(toast)`'s — and a wall whose family is not known
    /// (`target: None`) is written as `app.open-board(0)`: `app.open-board` always takes an `x`,
    /// and 0 reads back as "the wall shown".
    fn show(&self, toast: &Toast);

    /// Withdraw the notification raised under `tag` (`NotificationRules::chat_tag(id)` when that
    /// chat is opened — Windows' rule).
    fn withdraw(&self, tag: &str);

    /// A call rings (§14.4, §14.6): urgent, category `call.incoming`, buttons Decline
    /// (`app.decline-call(s)`) and Answer (`app.answer-call(s)`) — the reliable door on Wayland,
    /// where a call window may not take focus. `title`/`body` are P6's words (`CallText`).
    fn ring(&self, call_id: &str, title: &str, body: &str) {
        let _ = (call_id, title, body);
    }

    /// The call stopped ringing (answered here or elsewhere, declined, ended).
    fn stop_ringing(&self, call_id: &str) {
        let _ = call_id;
    }
}

/// The top-bar icon (§16).
pub trait TrayState {
    /// Whether an icon can exist NOW: something owns `org.kde.StatusNotifierWatcher` (Ubuntu's
    /// AppIndicator extension). It decides the close rule (`tray_rules::close_hides`), a
    /// `--background` start (`tray_rules::starts_hidden`) and Settings' "Keep running" row.
    fn available(&self) -> bool;

    /// Raised with the new answer when [`available`](Self::available) changes (the extension
    /// toggled, GNOME Shell restarted).
    fn on_available_changed(&self) -> &Event<bool>;

    /// The count beside the icon (`tray_rules::count_text(unread)`, `XAyatanaLabel`) and the
    /// tooltip's title (`NotificationRules::window_title`, "(3) Family Connect").
    fn set_unread(&self, unread: i64, title: &str);

    /// A call rings: `Status` = `NeedsAttention` (else `Active`).
    fn set_ringing(&self, ringing: bool) {
        let _ = ringing;
    }

    /// Take the icon away — on the way out (§3.8).
    fn withdraw(&self) {}
}

/// The dock's badge (§18): `com.canonical.Unity.LauncherEntry.Update`.
pub trait BadgeState {
    /// `count` and `count-visible` (`count > 0`).
    fn set_count(&self, unread: i64);

    /// `urgent` while a call rings.
    fn set_urgent(&self, urgent: bool) {
        let _ = urgent;
    }
}

/// The global shortcut (§19).
pub trait HotKeyState {
    /// What the portal bound — Settings shows `Bound`'s trigger as the desktop wrote it.
    fn state(&self) -> PortalState;

    /// Raised when the state changes (`BindShortcuts` answered, `ShortcutsChanged`).
    fn on_state_changed(&self) -> &Event<()>;

    /// The "Open with Ctrl+Alt+Shift+F" switch: bind the shortcut (on) or close the session (off).
    /// S0 calls it once at start-up with the `hot-key` setting; P4's row calls it on a change.
    fn apply(&self, enabled: bool);

    /// Settings' "Change…" (§13, §19): the desktop's own dialog for the bound shortcut — the
    /// portal's `ConfigureShortcuts` (GlobalShortcuts version 2) — over `parent`; the new trigger
    /// comes back as `ShortcutsChanged` ([`on_state_changed`](Self::on_state_changed)). Offered
    /// only where [`can_configure`](Self::can_configure) says. (Added at integration.)
    fn configure(&self, parent: Option<&gtk::Window>) {
        let _ = parent;
    }

    /// Whether "Change…" is offered: a shortcut is bound and the portal has `ConfigureShortcuts`.
    /// (Added at integration.)
    fn can_configure(&self) -> bool {
        false
    }
}

/// Launch at login (§17): the XDG autostart entry.
pub trait AutostartState {
    /// The entry's state now (`StartupState::from_found`).
    fn state(&self) -> StartupState;

    /// Write (`true`) or remove (`false`) the app's own entry; answers the state after
    /// (`startup_setting::request_succeeded` judges it).
    fn set_enabled(&self, enabled: bool) -> StartupState;
}

/// S0's stand-ins: notifications go nowhere, there is no icon (so a close QUITS — the decision
/// "no top-bar icon → close quits"), no badge, no portal, no autostart.
#[derive(Debug, Default)]
pub struct NoDesktop {
    available: Event<bool>,
    hot_key: Event<()>,
}

impl Notifier for NoDesktop {
    fn available(&self) -> bool {
        false
    }

    fn show(&self, toast: &Toast) {
        crate::fc_debug!("notify: {} not shown (no notifier installed)", toast.tag);
    }

    fn withdraw(&self, _tag: &str) {}
}

impl TrayState for NoDesktop {
    fn available(&self) -> bool {
        false
    }

    fn on_available_changed(&self) -> &Event<bool> {
        &self.available
    }

    fn set_unread(&self, _unread: i64, _title: &str) {}
}

impl BadgeState for NoDesktop {
    fn set_count(&self, _unread: i64) {}
}

impl HotKeyState for NoDesktop {
    fn state(&self) -> PortalState {
        PortalState::NoPortal
    }

    fn on_state_changed(&self) -> &Event<()> {
        &self.hot_key
    }

    fn apply(&self, _enabled: bool) {}
}

impl AutostartState for NoDesktop {
    fn state(&self) -> StartupState {
        StartupState::Unavailable
    }

    fn set_enabled(&self, _enabled: bool) -> StartupState {
        StartupState::Unavailable
    }
}
