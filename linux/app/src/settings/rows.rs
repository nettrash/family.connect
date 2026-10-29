//! What each row of the Settings window shows (shell-design §13) — decided here, with no GTK, from
//! fc-logic's rules (`tray_rules`, `startup_setting`, `global_hot_key_rules`, `voice_look::
//! voice_speed`, `app_version_text`) and the catalogue, so every branch is a unit test. The window
//! (`windows::settings_window`) only copies a [`SwitchLook`] onto an `adw::SwitchRow`.

use fc_core::text::string_catalog::StringCatalog;
use fc_logic::app_version_text;
use fc_logic::global_hot_key_rules::{self, PortalState};
use fc_logic::startup_setting::{self, StartupState};
use fc_logic::voice_look::voice_speed;

use crate::seams::media::CameraDevice;

/// How one switch row looks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchLook {
    /// Drawn at all.
    pub visible: bool,
    pub title: String,
    pub active: bool,
    /// May be flipped here.
    pub sensitive: bool,
    /// The sentence under the title (empty: none).
    pub subtitle: String,
}

/// "Keep running in the top bar" (§13, §16.4; carry-forward decision 3). With an icon the switch
/// is the setting. With NO icon a close quits whatever the setting says (`tray_rules::
/// close_hides`), so the switch reads off, cannot be flipped, and says why — the setting itself is
/// kept for the day an icon exists again.
pub fn keep_running(
    keeps_running: bool,
    tray_available: bool,
    say: &dyn StringCatalog,
) -> SwitchLook {
    SwitchLook {
        visible: true,
        title: say.get("Keep running in the top bar").to_string(),
        active: fc_logic::tray_rules::close_hides(keeps_running, tray_available, false),
        sensitive: tray_available,
        subtitle: if tray_available {
            String::new()
        } else {
            say.get("Your desktop has no top-bar icons for apps, so closing the window quits Family Connect.")
                .to_string()
        },
    }
}

/// "Start when I log in" (§17): `startup_setting`'s rules, word for word — not drawn when the
/// autostart folder cannot be written, off and insensitive when the desktop or an administrator
/// decided, and the sentence that says where the real switch is.
pub fn start_at_login(state: StartupState, say: &dyn StringCatalog) -> SwitchLook {
    SwitchLook {
        visible: startup_setting::is_offered(state),
        title: startup_setting::title(say),
        active: startup_setting::is_on(state),
        sensitive: startup_setting::is_changeable(state),
        subtitle: startup_setting::note_text(startup_setting::note_for(state), say),
    }
}

/// "Open with Ctrl+Alt+Shift+F" (§19): the trigger GNOME actually bound in the title, and the
/// footnote for the portal's answer. With no portal there is nothing to switch: the row is off and
/// insensitive and says to bind `family-connect --toggle` instead. The row only WRITES the
/// `hot-key` setting; S0 applies it (`HotKeyState::apply`).
pub fn hot_key(state: &PortalState, enabled: bool, say: &dyn StringCatalog) -> SwitchLook {
    let changeable = global_hot_key_rules::is_changeable(state);
    SwitchLook {
        visible: true,
        title: global_hot_key_rules::title(state, say),
        active: enabled && changeable,
        sensitive: changeable,
        subtitle: global_hot_key_rules::footnote(state, enabled, say),
    }
}

/// The Notifications group's two sentences (§13, §15.5, D15): the Mac's "who, never what" line,
/// and where GNOME's own switch is — said always, since no API tells a host app whether GNOME will
/// show its notifications.
pub fn notification_notes(say: &dyn StringCatalog) -> [String; 2] {
    [
        say.get("When a message arrives in a chat you are not reading, a notification says who wrote — never what they wrote.")
            .to_string(),
        say.get("If no notifications appear, check Settings ▸ Notifications ▸ Family Connect.")
            .to_string(),
    ]
}

/// The version line (§13): "Family Connect for Linux 1.3 (437)" — `AppVersionText`, the build
/// from `FAMILY_CONNECT_BUILD` at compile time (CI stamps it; a local build carries none and the
/// brackets go).
pub fn version_line(say: &dyn StringCatalog) -> String {
    let version = version_value(
        env!("CARGO_PKG_VERSION_MAJOR"),
        env!("CARGO_PKG_VERSION_MINOR"),
        option_env!("FAMILY_CONNECT_BUILD"),
    );
    say.format("Family Connect for Linux %@", &[&version])
}

/// [`version_line`]'s value from its three parts as text.
pub fn version_value(major: &str, minor: &str, build: Option<&str>) -> String {
    let number = |text: &str| text.trim().parse::<u32>().unwrap_or(0);
    app_version_text::for_version(number(major), number(minor), build.map_or(0, number))
}

/// What Log Out asks (the Mac's question, "this device" for "this Mac"), and the outbox's warning
/// when something is still waiting to be sent (§13).
pub fn logout_message(unsent: bool, say: &dyn StringCatalog) -> String {
    let removed = say.get("Local messages are removed from this device.");
    if unsent {
        format!(
            "{removed} {}",
            say.get("What hasn't been sent yet is lost.")
        )
    } else {
        removed.to_string()
    }
}

/// The Server group's line when the sign-in lives in memory only (§20, D17).
pub fn remembered_line(remembered: bool, say: &dyn StringCatalog) -> Option<String> {
    (!remembered).then(|| {
        say.get("Your sign-in isn't remembered on this computer: no keyring is available.")
            .to_string()
    })
}

/// The voice speed's toggles, by name — the stored text of each of `voice_speed::RATES`
/// ("1", "1.5", "2") — and the one the setting picks (anything unreadable is 1×).
pub fn speed_names() -> Vec<String> {
    voice_speed::RATES
        .iter()
        .map(|rate| voice_speed::store(*rate))
        .collect()
}

/// The toggle the stored `voice-speed` text picks.
pub fn speed_selected(stored: Option<&str>) -> String {
    voice_speed::store(voice_speed::parse(stored))
}

/// The camera row's choices (§13): "Automatic" first, then every camera by its own name — and the
/// index the stored `video-camera` id picks. A stored camera that is not plugged in now shows as
/// "Automatic" (the recorder falls back the same way: `round_video_rules`' choice of camera).
pub fn camera_rows(
    devices: &[CameraDevice],
    chosen: Option<&str>,
    say: &dyn StringCatalog,
) -> (Vec<String>, u32) {
    let mut names = vec![say.get("Automatic").to_string()];
    names.extend(devices.iter().map(|device| device.name.clone()));
    let selected = chosen
        .filter(|id| !id.is_empty())
        .and_then(|id| devices.iter().position(|device| device.id == id))
        .map_or(0, |at| u32::try_from(at + 1).unwrap_or(0));
    (names, selected)
}

/// The id to store for a choice of the camera row: `None` for "Automatic" (the key is removed).
pub fn camera_choice(devices: &[CameraDevice], selected: u32) -> Option<String> {
    let at = usize::try_from(selected).ok()?.checked_sub(1)?;
    devices.get(at).map(|device| device.id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use fc_core::text::string_catalog::EnglishCatalog;

    fn say() -> EnglishCatalog {
        EnglishCatalog::new()
    }

    #[test]
    fn keep_running_is_the_setting_only_while_an_icon_exists() {
        let with_icon = keep_running(true, true, &say());
        assert!(with_icon.active && with_icon.sensitive && with_icon.subtitle.is_empty());
        assert_eq!(with_icon.title, "Keep running in the top bar");
        let off = keep_running(false, true, &say());
        assert!(!off.active && off.sensitive);
        // No icon: a close quits, so the row reads off, cannot be flipped, and says why.
        let no_icon = keep_running(true, false, &say());
        assert!(!no_icon.active && !no_icon.sensitive);
        assert!(no_icon.subtitle.contains("closing the window quits"));
    }

    #[test]
    fn start_at_login_follows_the_six_states() {
        let look = |state| start_at_login(state, &say());
        assert!(!look(StartupState::Unavailable).visible);
        let disabled = look(StartupState::Disabled);
        assert!(disabled.visible && !disabled.active && disabled.sensitive);
        assert!(
            disabled
                .subtitle
                .starts_with("Family Connect opens in the top bar")
        );
        let enabled = look(StartupState::Enabled);
        assert!(enabled.active && enabled.sensitive);
        let by_user = look(StartupState::DisabledByUser);
        assert!(!by_user.active && !by_user.sensitive);
        assert_eq!(
            by_user.subtitle,
            "Turned off in your desktop's startup settings."
        );
        let policy = look(StartupState::EnabledByPolicy);
        assert!(policy.active && !policy.sensitive);
        assert!(policy.subtitle.starts_with("Your organisation decides"));
        assert!(!look(StartupState::DisabledByPolicy).active);
        assert_eq!(enabled.title, "Start when I log in");
    }

    #[test]
    fn the_hot_key_row_shows_what_the_portal_bound() {
        let bound = hot_key(&PortalState::Bound("Shift+Ctrl+Alt+F".into()), true, &say());
        assert_eq!(bound.title, "Open with Shift+Ctrl+Alt+F");
        assert!(bound.active && bound.sensitive);
        assert!(bound.subtitle.starts_with("Brings Family Connect forward"));
        let refused = hot_key(&PortalState::NotBound, true, &say());
        assert_eq!(refused.title, "Open with Ctrl+Alt+Shift+F");
        assert_eq!(refused.subtitle, "Your desktop didn't set this shortcut.");
        let off = hot_key(&PortalState::NotBound, false, &say());
        assert!(!off.active && off.sensitive);
        // No portal: nothing to switch; bind `--toggle` instead.
        let none = hot_key(&PortalState::NoPortal, true, &say());
        assert!(!none.active && !none.sensitive);
        assert!(none.subtitle.contains("family-connect --toggle"));
    }

    #[test]
    fn the_version_line_drops_an_unstamped_build() {
        assert_eq!(version_value("1", "3", None), "1.3");
        assert_eq!(version_value("1", "3", Some("0")), "1.3");
        assert_eq!(version_value("1", "3", Some(" 437 ")), "1.3 (437)");
        assert_eq!(version_value("1", "3", Some("x")), "1.3");
        assert!(version_line(&say()).starts_with("Family Connect for Linux 1."));
    }

    #[test]
    fn log_out_warns_about_what_is_unsent() {
        assert_eq!(
            logout_message(false, &say()),
            "Local messages are removed from this device."
        );
        assert_eq!(
            logout_message(true, &say()),
            "Local messages are removed from this device. What hasn't been sent yet is lost."
        );
        assert_eq!(remembered_line(true, &say()), None);
        assert!(
            remembered_line(false, &say())
                .unwrap()
                .contains("no keyring")
        );
    }

    #[test]
    fn the_speed_toggles_are_the_stored_texts() {
        assert_eq!(speed_names(), vec!["1", "1.5", "2"]);
        assert_eq!(speed_selected(None), "1");
        assert_eq!(speed_selected(Some("1.5")), "1.5");
        assert_eq!(speed_selected(Some(" 2 ")), "2");
        assert_eq!(speed_selected(Some("3")), "1");
        assert_eq!(speed_selected(Some("1,5")), "1");
    }

    #[test]
    fn the_camera_row_offers_automatic_first() {
        let devices = vec![
            CameraDevice {
                id: "node-41".into(),
                name: "Integrated Camera".into(),
            },
            CameraDevice {
                id: "/dev/video2".into(),
                name: "USB Camera".into(),
            },
        ];
        let (names, selected) = camera_rows(&devices, None, &say());
        assert_eq!(names, vec!["Automatic", "Integrated Camera", "USB Camera"]);
        assert_eq!(selected, 0);
        assert_eq!(camera_rows(&devices, Some("/dev/video2"), &say()).1, 2);
        // Unplugged since: Automatic.
        assert_eq!(camera_rows(&devices, Some("gone"), &say()).1, 0);
        assert_eq!(camera_rows(&devices, Some(""), &say()).1, 0);
        assert_eq!(camera_choice(&devices, 0), None);
        assert_eq!(camera_choice(&devices, 1).as_deref(), Some("node-41"));
        assert_eq!(camera_choice(&devices, 9), None);
        assert_eq!(
            camera_rows(&[], None, &say()),
            (vec!["Automatic".into()], 0)
        );
    }

    #[test]
    fn the_notification_notes_are_who_never_what_and_where_the_switch_is() {
        let [mac, where_] = notification_notes(&say());
        assert!(mac.ends_with("never what they wrote."));
        assert!(where_.contains("Settings ▸ Notifications"));
    }
}
