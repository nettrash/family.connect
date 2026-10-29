//! The app in the top bar (shell-design §3.3, §3.7, §16 and Appendix B item 1) — a port of the
//! Mac's `MenuBarRules` (`ios/FamilyConnect/Core/MenuBarRules.swift`, issue #80,
//! docs/mac-menu-bar-2026-10-07.md): every DECISION about the StatusNotifierItem and the window it
//! stands in for, with none of GTK or D-Bus. `desktop::Tray` is the wiring around it.
//!
//! THE ICON MAY NOT EXIST. The Mac's menu bar is always there; a Linux top bar shows app icons only
//! while something owns `org.kde.StatusNotifierWatcher` (Ubuntu's AppIndicator extension; not
//! vanilla GNOME, not with the extension switched off — §16.4). So every rule here that the Mac
//! keys on "the menu bar icon is kept" asks two things on Linux: the person's "Keep running in the
//! top bar" switch, and whether an icon can exist at all. With no icon a close quits and no launch
//! starts hidden: there is nothing to come back from.
//!
//! Not ported from the Mac: `policy` (the Dock icon only while a window is open) — GNOME has no
//! activation policy, and an app with no mapped window has no dock or Alt+Tab entry of its own;
//! `loginSwitchOn`/`loginSwitchEnabled` — Linux reads launch-at-login with Windows' rules
//! (`startup_setting`, §17), where a switch turned off elsewhere reads OFF and insensitive, not on
//! as the Mac's `needsApproval` does; `hotKeyAction` — `global_hot_key_rules::action` is the same
//! rule (`window_in_front` is the Mac's `appActive && mainWindowKey`).

use std::time::Duration;

use fc_core::text::string_catalog::StringCatalog;

/// The text beside the top-bar icon (the item's `XAyatanaLabel`): the unread count, nothing at
/// zero, and never wider than "99+" (also the label's `XAyatanaLabelGuide`, so the bar reserves
/// that width). `None` is written as the empty label.
pub fn count_text(unread: i64) -> Option<String> {
    if unread <= 0 {
        None
    } else if unread > 99 {
        Some(COUNT_GUIDE.to_string())
    } else {
        Some(unread.to_string())
    }
}

/// The widest [`count_text`] — `XAyatanaLabelGuide`.
pub const COUNT_GUIDE: &str = "99+";

/// The main window's close button and Ctrl+W hide it rather than close it while the person keeps
/// the app running in the top bar AND an icon there exists to bring it back — so the socket, the
/// call engine and the ringing stay alive (a closed Linux client cannot ring: there is no push).
/// Never on the way out (Quit, logout's `query-end`). The other windows — a conversation, the
/// board, a call, Settings — always close; the shell asks this of the main window only (the
/// Mac's `isMainWindow` and `anotherMainOpen` — GTK has exactly one main window).
pub fn close_hides(keeps_running: bool, tray_available: bool, quitting: bool) -> bool {
    keeps_running && tray_available && !quitting
}

/// A launch the desktop made at login (`--background`) starts in the top bar, its window hidden —
/// and only when there IS an icon to come back from (Windows' `StartupSetting.StartsHidden`, the
/// Mac's `startsHidden`). The shell's backstop still presents the window if no icon has appeared
/// [`WINDOW_ARRIVAL_GRACE`] after a hidden start (the Mac's `LaunchWindowBackstop`, §3.7).
pub fn starts_hidden(launched_at_login: bool, tray_available: bool) -> bool {
    launched_at_login && tray_available
}

/// What bringing the main window forward does (#93: a notification click must never open a
/// second window when one is there already).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MainWindowStep {
    /// Bring forward the main window that exists — shown, minimised or hidden in the top bar.
    BringForward,
    /// Open a new main window: there is none.
    Open,
    /// Wait for the one on its way: a new window was asked for a moment ago, or the app is still
    /// starting and its window has not appeared yet. Asked again shortly; the start-up backstop
    /// still guarantees a window either way.
    Wait,
}

/// How long a new window may take to appear before it counts as never coming, how long a start
/// may take to build its own — and how long a hidden start waits for its icon before showing the
/// window after all.
pub const WINDOW_ARRIVAL_GRACE: Duration = Duration::from_secs(2);

/// The decision. `has_main_window` is any main window that is not closed — visible, minimised, or
/// hidden in the top bar; `open_requested_ago` is how long since this app last asked for a new one
/// (`None`: never); `since_launch` is how long the app has been running. (GTK builds the main
/// window once at start-up, so on Linux this is `BringForward` but for a click that arrives while
/// the app is still starting.)
pub fn main_window_step(
    has_main_window: bool,
    open_requested_ago: Option<Duration>,
    since_launch: Duration,
) -> MainWindowStep {
    if has_main_window {
        return MainWindowStep::BringForward;
    }
    if open_requested_ago.is_some_and(|ago| ago < WINDOW_ARRIVAL_GRACE) {
        return MainWindowStep::Wait;
    }
    if since_launch < WINDOW_ARRIVAL_GRACE {
        return MainWindowStep::Wait;
    }
    MainWindowStep::Open
}

/// The icon's menu (`com.canonical.dbusmenu`, §16.2): the Mac's three items, a separator before
/// Quit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrayItem {
    /// Present the main window.
    Open,
    /// The Settings window.
    Settings,
    /// Quit (§3.8).
    Quit,
}

impl TrayItem {
    /// In the menu's order.
    pub const ALL: [TrayItem; 3] = [TrayItem::Open, TrayItem::Settings, TrayItem::Quit];

    /// The item's words — the Mac's `MacMenuBar` sentences.
    pub fn label(self, say: &dyn StringCatalog) -> String {
        match self {
            TrayItem::Open => say.get("Open Family Connect"),
            TrayItem::Settings => say.get("Settings…"),
            TrayItem::Quit => say.get("Quit Family Connect"),
        }
        .to_string()
    }

    /// Whether a separator stands above it.
    pub fn separated(self) -> bool {
        self == TrayItem::Quit
    }
}
