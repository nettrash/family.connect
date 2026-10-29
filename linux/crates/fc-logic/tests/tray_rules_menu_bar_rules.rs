//! The Mac's `MenuBarRulesTests` (`ios/FamilyConnectTests/MenuBarRulesTests.swift`, #80) for the
//! top-bar icon (shell-design §3.3, §3.7, §16): what a close does, whether a launch starts hidden,
//! the shortcut, the icon's count and the window step — case for case, with the one thing the Mac
//! never has to ask: whether an icon can exist at all.
//!
//! Not ported, with their rules (see `tray_rules`' module comment): `policyFollowsTheWindows`,
//! `minimisedKeepsTheDockIcon`, `hiddenAppKeepsItsPolicy`, `noMenuBarIsAlwaysRegular` — the Dock
//! activation policy, which GNOME does not have; `loginSwitch` — Linux reads launch-at-login with
//! Windows' rules (`startup_setting_tests.rs`). `hotKey`'s cases are below, against
//! `global_hot_key_rules::action`, the one rule both platforms share.

mod common;

use std::time::Duration;

use fc_logic::global_hot_key_rules::{HotKeyAction, action};
use fc_logic::tray_rules::{
    COUNT_GUIDE, MainWindowStep, TrayItem, WINDOW_ARRIVAL_GRACE, close_hides, count_text,
    main_window_step, starts_hidden,
};

/// "only the main window hides on close, only with the icon kept, never on the way out". The
/// Mac's `keepsMenuBar` is both of Linux's facts: the switch and an icon to come back to. Its
/// `isMainWindow: false` and `anotherMainOpen` cases are the shell's — it asks this of the one
/// main window only, and every other window closes.
#[test]
fn close_hides_only_with_the_icon_kept_and_never_on_the_way_out() {
    assert!(close_hides(true, true, false));
    assert!(!close_hides(false, true, false));
    assert!(!close_hides(true, true, true));
    // No icon to come back from — vanilla GNOME, the extension switched off: a close quits,
    // whatever the switch says (§16.4).
    assert!(!close_hides(true, false, false));
    assert!(!close_hides(false, false, false));
    assert!(!close_hides(true, false, true));
}

/// "a launch at login starts hidden only when there is an icon to come back from".
#[test]
fn a_launch_at_login_starts_hidden_only_with_an_icon_to_come_back_from() {
    assert!(starts_hidden(true, true));
    assert!(!starts_hidden(true, false));
    assert!(!starts_hidden(false, true));
    assert!(!starts_hidden(false, false));
}

/// "the shortcut shows the window from anywhere, and hides it when it is the one in front": the
/// Mac's `appActive && mainWindowKey` is `window_in_front`; `keepsMenuBar` is `can_hide_to_tray`.
#[test]
fn the_shortcut_shows_the_window_from_anywhere_and_hides_it_in_front() {
    let hot_key = |app_active: bool, main_window_key: bool, keeps: bool| {
        action(app_active && main_window_key, keeps)
    };
    assert_eq!(hot_key(false, false, true), HotKeyAction::Show);
    // The app in front, but on Settings or a conversation window: show the main one.
    assert_eq!(hot_key(true, false, true), HotKeyAction::Show);
    assert_eq!(hot_key(true, true, true), HotKeyAction::Hide);
    // Nowhere to hide it to.
    assert_eq!(hot_key(true, true, false), HotKeyAction::Show);
}

/// "the count beside the icon: nothing at zero, never wider than 99+".
#[test]
fn the_count_beside_the_icon_is_nothing_at_zero_and_never_wider_than_99_plus() {
    assert_eq!(count_text(0), None);
    assert_eq!(count_text(-3), None);
    assert_eq!(count_text(1).as_deref(), Some("1"));
    assert_eq!(count_text(99).as_deref(), Some("99"));
    assert_eq!(count_text(100).as_deref(), Some("99+"));
    assert_eq!(count_text(i64::MAX).as_deref(), Some("99+"));
    // The label guide reserves the widest the count gets.
    assert_eq!(COUNT_GUIDE, "99+");
}

/// "a notification brings forward the main window that exists — never a second one (#93)".
#[test]
fn a_notification_brings_forward_the_main_window_that_exists() {
    let seconds = Duration::from_secs_f64;
    // Any main window that is not closed, hidden into the top bar included.
    assert_eq!(
        main_window_step(true, None, seconds(600.0)),
        MainWindowStep::BringForward
    );
    assert_eq!(
        main_window_step(true, Some(seconds(0.1)), seconds(0.2)),
        MainWindowStep::BringForward
    );
    // None at all, long after launch: open one.
    assert_eq!(
        main_window_step(false, None, seconds(600.0)),
        MainWindowStep::Open
    );
    // One asked for a moment ago and not here yet — a second click waits for it.
    assert_eq!(
        main_window_step(false, Some(seconds(0.3)), seconds(600.0)),
        MainWindowStep::Wait
    );
    // Asked for and never came: ask again.
    assert_eq!(
        main_window_step(false, Some(seconds(5.0)), seconds(600.0)),
        MainWindowStep::Open
    );
    // Launched by the click itself: the window is on its way.
    assert_eq!(
        main_window_step(false, None, seconds(0.5)),
        MainWindowStep::Wait
    );
    assert_eq!(
        main_window_step(false, None, seconds(2.5)),
        MainWindowStep::Open
    );
    assert_eq!(WINDOW_ARRIVAL_GRACE, Duration::from_secs(2));
}

/// The icon's menu: the Mac's three items in its order, the separator before Quit, the words the
/// catalogue's.
#[test]
fn the_menu_is_the_macs_three_items() {
    let say = common::english();
    let labels: Vec<String> = TrayItem::ALL.iter().map(|item| item.label(&say)).collect();
    assert_eq!(
        labels,
        ["Open Family Connect", "Settings…", "Quit Family Connect"]
    );
    let separated: Vec<bool> = TrayItem::ALL.iter().map(|item| item.separated()).collect();
    assert_eq!(separated, [false, false, true]);
    // Translated where Windows' tables have them.
    let german = common::catalogues().for_language("de");
    assert_ne!(TrayItem::Quit.label(&german), "Quit Family Connect");
}
