//! `GlobalHotKeyRulesTests`: the global shortcut (#80) — what a press does, and the combination
//! itself — with Win32's "taken" replaced by the GlobalShortcuts portal's three answers
//! (shell-design §19).
//!
//! Not ported: `OnlyAlreadyRegisteredIsTaken` — `ERROR_HOTKEY_ALREADY_REGISTERED` (1409) is
//! Win32's; on Linux GNOME resolves a taken trigger in its own dialog, and what comes back is
//! [`PortalState`] (`the_portal_says_what_it_bound`). `TheCombinationIsNeverAltGr`'s Win32
//! modifier and virtual-key numbers became the portal's preferred trigger.

mod common;

use fc_logic::global_hot_key_rules::{
    self, HotKeyAction, LABEL, PREFERRED_TRIGGER, PortalState, SHORTCUT_ID, action,
};

#[test]
fn from_anywhere_it_shows_and_in_front_it_hides() {
    for (in_front, can_hide, expected) in [
        (false, true, HotKeyAction::Show),
        (false, false, HotKeyAction::Show),
        (true, true, HotKeyAction::Hide),
        (true, false, HotKeyAction::Show),
    ] {
        assert_eq!(
            action(in_front, can_hide),
            expected,
            "{in_front} {can_hide}"
        );
    }
}

/// Never Ctrl+Alt alone: that is AltGr on Windows (and on an X11 setup that maps it so), and
/// AltGr+F types "[" on Hungarian and Czech keyboards. Three modifiers and F, in the portal's
/// spelling and in the words people read.
#[test]
fn the_combination_is_never_alt_gr() {
    assert_eq!(PREFERRED_TRIGGER, "CTRL+ALT+SHIFT+f");
    let parts: Vec<&str> = PREFERRED_TRIGGER.split('+').collect();
    assert_eq!(parts, ["CTRL", "ALT", "SHIFT", "f"]);
    assert_eq!(LABEL, "Ctrl+Alt+Shift+F");
    assert_eq!(SHORTCUT_ID, "toggle-window");
}

/// What `BindShortcuts` (or `ShortcutsChanged`) gave back: ours with a trigger is bound, to
/// exactly the trigger GNOME describes; ours without one, or not there at all, is not.
#[test]
fn the_portal_says_what_it_bound() {
    assert_eq!(
        PortalState::from_shortcuts(&[("toggle-window", Some("Shift+Ctrl+Alt+F"))]),
        PortalState::Bound("Shift+Ctrl+Alt+F".to_string())
    );
    assert_eq!(
        PortalState::from_shortcuts(&[
            ("something-else", Some("Ctrl+Q")),
            ("toggle-window", Some(" Ctrl+Alt+Shift+F "))
        ]),
        PortalState::Bound("Ctrl+Alt+Shift+F".to_string())
    );
    assert_eq!(
        PortalState::from_shortcuts(&[("toggle-window", None)]),
        PortalState::NotBound
    );
    assert_eq!(
        PortalState::from_shortcuts(&[("toggle-window", Some("  "))]),
        PortalState::NotBound
    );
    assert_eq!(
        PortalState::from_shortcuts(&[("something-else", Some("Ctrl+Q"))]),
        PortalState::NotBound
    );
    assert_eq!(PortalState::from_shortcuts(&[]), PortalState::NotBound);
}

/// Settings shows the trigger GNOME bound, says when the shortcut does nothing, and with no
/// portal says how to get it from the desktop instead — with its switch insensitive.
#[test]
fn settings_says_what_the_portal_answered() {
    let say = common::english();
    let bound = PortalState::Bound("Shift+Ctrl+Alt+F".to_string());
    assert_eq!(
        global_hot_key_rules::title(&bound, &say),
        "Open with Shift+Ctrl+Alt+F"
    );
    assert_eq!(
        global_hot_key_rules::title(&PortalState::NotBound, &say),
        "Open with Ctrl+Alt+Shift+F"
    );
    assert_eq!(
        global_hot_key_rules::title(&PortalState::NoPortal, &say),
        "Open with Ctrl+Alt+Shift+F"
    );

    let what_it_does =
        "Brings Family Connect forward from any app. Press it again to put it back in the top bar.";
    assert_eq!(
        global_hot_key_rules::footnote(&bound, true, &say),
        what_it_does
    );
    assert_eq!(
        global_hot_key_rules::footnote(&PortalState::NotBound, false, &say),
        what_it_does
    );
    assert_eq!(
        global_hot_key_rules::footnote(&PortalState::NotBound, true, &say),
        "Your desktop didn't set this shortcut."
    );
    let no_portal =
        "Set a shortcut in your desktop's keyboard settings to run family-connect --toggle.";
    assert_eq!(
        global_hot_key_rules::footnote(&PortalState::NoPortal, true, &say),
        no_portal
    );
    assert_eq!(
        global_hot_key_rules::footnote(&PortalState::NoPortal, false, &say),
        no_portal
    );

    assert!(global_hot_key_rules::is_changeable(&bound));
    assert!(global_hot_key_rules::is_changeable(&PortalState::NotBound));
    assert!(!global_hot_key_rules::is_changeable(&PortalState::NoPortal));
    assert_eq!(
        global_hot_key_rules::description(&say),
        "Show or hide Family Connect"
    );
}

/// The label is a sentence of the catalogue: German writes "Strg+Alt+Umschalt+F" (Windows'
/// table), and the title follows.
#[test]
fn the_shortcut_is_written_in_the_readers_language() {
    let german = common::catalogues().for_language("de");
    let title = global_hot_key_rules::title(&PortalState::NotBound, &german);
    assert!(title.contains("Strg"), "{title}");
}
