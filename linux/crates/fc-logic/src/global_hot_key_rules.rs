//! The shortcut that brings Family Connect forward from any app (issue #80,
//! docs/mac-menu-bar-2026-10-07.md; `GlobalHotKeyRules.cs`) — the Mac's ⌃⌥⌘F, here, as on
//! Windows, Ctrl+Alt+Shift+F.
//!
//! WITHOUT WIN32 (shell-design §19, Appendix B item 3). Windows registers the chord itself and
//! learns it is TAKEN (`ERROR_HOTKEY_ALREADY_REGISTERED`); on Linux an app cannot grab a key — the
//! XDG GlobalShortcuts portal binds it, GNOME shows its own dialog when the preferred trigger is
//! taken and the person picks, and the trigger actually bound comes back in `BindShortcuts`'
//! results or `ShortcutsChanged` (`trigger_description`). So "taken" is replaced by the portal's
//! three answers ([`PortalState`]), and Settings shows the trigger GNOME bound.
//!
//! Three modifiers, as on the Mac, and never Ctrl+Alt alone: on Windows Ctrl+Alt IS AltGr, and on a
//! Hungarian or Czech keyboard AltGr+F TYPES "[", which a system-wide shortcut would take from
//! every app. Linux keeps AltGr as its own key (ISO_Level3_Shift), but X11 setups that map AltGr to
//! Ctrl+Alt exist, and one chord on every desktop client is worth more than a shorter one.

use fc_core::text::string_catalog::StringCatalog;

/// What the global shortcut does when pressed (#80).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HotKeyAction {
    /// Bring the window forward — from the top bar, from behind other windows, from minimised.
    Show,
    /// Put it back in the top bar.
    Hide,
}

/// The shortcut's id in the portal session (`BindShortcuts`' `a(sa{sv})`, `Activated`).
pub const SHORTCUT_ID: &str = "toggle-window";

/// The trigger asked for, in the XDG shortcuts specification's spelling (portal
/// `preferred_trigger`): unbound in GNOME 50 with Ubuntu's overrides (measured, §1.7).
pub const PREFERRED_TRIGGER: &str = "CTRL+ALT+SHIFT+f";

/// The shortcut as people write it in English; translated, because German says
/// "Strg+Alt+Umschalt+F" (the catalogue key Windows uses).
pub const LABEL: &str = "Ctrl+Alt+Shift+F";

/// From anywhere it brings the window forward; pressed while the window is the one in front, it
/// puts it back in the top bar — only when there is an icon there to come back from and closing is
/// allowed to keep the app running (otherwise there is nowhere to hide it, and it simply stays in
/// front).
pub fn action(window_in_front: bool, can_hide_to_tray: bool) -> HotKeyAction {
    if window_in_front && can_hide_to_tray {
        HotKeyAction::Hide
    } else {
        HotKeyAction::Show
    }
}

/// What the portal answered about the shortcut.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PortalState {
    /// Bound, to this trigger as the desktop describes it (`trigger_description`, in its own
    /// words and language: "Ctrl+Alt+Shift+F", "Shift+Ctrl+Alt+F"…).
    Bound(String),
    /// The portal answered and bound nothing — the person dismissed GNOME's dialog, or the session
    /// was closed — so the shortcut does nothing now.
    NotBound,
    /// No GlobalShortcuts portal on this desktop (another desktop, X11, an old portal):
    /// `family-connect --toggle` is the way, bound in the desktop's own keyboard settings.
    NoPortal,
}

impl PortalState {
    /// `BindShortcuts`' results (or `ShortcutsChanged`), as the shell reads them out of the
    /// `a(sa{sv})`: each shortcut's id and its `trigger_description`. Ours, with a description
    /// that says something, is bound; anything else is not. Trimmed: a description is shown.
    pub fn from_shortcuts(shortcuts: &[(&str, Option<&str>)]) -> PortalState {
        shortcuts
            .iter()
            .find(|(id, _)| *id == SHORTCUT_ID)
            .and_then(|(_, trigger)| trigger.map(str::trim).filter(|text| !text.is_empty()))
            .map_or(PortalState::NotBound, |trigger| {
                PortalState::Bound(trigger.to_string())
            })
    }
}

/// Whether the row's switch may be flipped: with no portal there is nothing for it to ask.
pub fn is_changeable(state: &PortalState) -> bool {
    *state != PortalState::NoPortal
}

/// The description the portal shows beside the shortcut in GNOME's dialog and its keyboard
/// settings.
pub fn description(say: &dyn StringCatalog) -> String {
    say.get("Show or hide Family Connect").to_string()
}

/// The row's title: "Open with" the trigger GNOME bound — or, until it has bound one, the one
/// asked for, in the reader's language.
pub fn title(state: &PortalState, say: &dyn StringCatalog) -> String {
    match state {
        PortalState::Bound(trigger) => say.format("Open with %@", &[trigger]),
        _ => say.format("Open with %@", &[&say.get("Ctrl+Alt+Shift+F")]),
    }
}

/// The sentence under the row: what the shortcut does — or, switched on and not bound, that it
/// does nothing here; with no portal, how to get the same from the desktop's own settings.
pub fn footnote(state: &PortalState, switched_on: bool, say: &dyn StringCatalog) -> String {
    match state {
        PortalState::NoPortal => say
            .get("Set a shortcut in your desktop's keyboard settings to run family-connect --toggle.")
            .to_string(),
        PortalState::NotBound if switched_on => {
            say.get("Your desktop didn't set this shortcut.").to_string()
        }
        _ => say
            .get("Brings Family Connect forward from any app. Press it again to put it back in the top bar.")
            .to_string(),
    }
}
