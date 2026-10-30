//! `StartupSettingTests`: the "Start when I log in" row, in every state the desktop can answer
//! with — Windows' eight tests over the same six states, and the Linux half: how those states are
//! read off the XDG autostart files (shell-design §17), the entry the app writes, and the row's
//! sentences.
//!
//! The interesting half is the three states the app cannot change. A toggle that looks live and
//! does nothing is what this is written to prevent.

mod common;

use fc_logic::startup_setting::{
    self, AutostartEntry, AutostartFound, Note, StartupState, entry_text, exec_argument,
};

const OFFERED: [StartupState; 5] = [
    StartupState::Disabled,
    StartupState::Enabled,
    StartupState::DisabledByUser,
    StartupState::DisabledByPolicy,
    StartupState::EnabledByPolicy,
];

#[test]
fn the_row_is_drawn_wherever_there_is_a_startup_task() {
    assert!(!startup_setting::is_offered(StartupState::Unavailable));
    for state in OFFERED {
        assert!(startup_setting::is_offered(state), "{state:?}");
    }
}

#[test]
fn the_toggle_sits_where_windows_says_it_does() {
    assert!(startup_setting::is_on(StartupState::Enabled));
    assert!(startup_setting::is_on(StartupState::EnabledByPolicy));
    assert!(!startup_setting::is_on(StartupState::Disabled));
    assert!(!startup_setting::is_on(StartupState::DisabledByUser));
    assert!(!startup_setting::is_on(StartupState::DisabledByPolicy));
}

/// Only two of the five are ours to change.
#[test]
fn only_the_apps_own_two_states_are_changeable() {
    assert!(startup_setting::is_changeable(StartupState::Disabled));
    assert!(startup_setting::is_changeable(StartupState::Enabled));
    assert!(!startup_setting::is_changeable(
        StartupState::DisabledByUser
    ));
    assert!(!startup_setting::is_changeable(
        StartupState::DisabledByPolicy
    ));
    assert!(!startup_setting::is_changeable(
        StartupState::EnabledByPolicy
    ));
    assert!(!startup_setting::is_changeable(StartupState::Unavailable));
}

/// Windows' `Note.BlockedByTaskManager` is `Note::BlockedByDesktop`: the desktop's startup
/// settings are Linux's Task Manager.
#[test]
fn each_state_has_its_own_sentence() {
    assert_eq!(
        startup_setting::note_for(StartupState::Disabled),
        Note::WhatItDoes
    );
    assert_eq!(
        startup_setting::note_for(StartupState::Enabled),
        Note::WhatItDoes
    );
    assert_eq!(
        startup_setting::note_for(StartupState::DisabledByUser),
        Note::BlockedByDesktop
    );
    assert_eq!(
        startup_setting::note_for(StartupState::DisabledByPolicy),
        Note::DecidedByPolicy
    );
    assert_eq!(
        startup_setting::note_for(StartupState::EnabledByPolicy),
        Note::DecidedByPolicy
    );
}

/// The one that matters at the moment somebody flips it: a refused request must put the row back,
/// not leave it looking on.
#[test]
fn a_refused_request_is_not_a_success() {
    assert!(startup_setting::request_succeeded(StartupState::Enabled));
    assert!(startup_setting::request_succeeded(
        StartupState::EnabledByPolicy
    ));
    assert!(!startup_setting::request_succeeded(
        StartupState::DisabledByUser
    ));
    assert!(!startup_setting::request_succeeded(StartupState::Disabled));
    assert!(!startup_setting::request_succeeded(
        StartupState::DisabledByPolicy
    ));
}

#[test]
fn a_launch_windows_made_at_sign_in_draws_no_window() {
    assert!(startup_setting::starts_hidden(true, true));
}

#[test]
fn a_launch_a_person_asked_for_always_draws_its_window() {
    assert!(!startup_setting::starts_hidden(false, true));
    assert!(!startup_setting::starts_hidden(false, false));
}

#[test]
fn with_no_icon_to_come_back_from_even_a_sign_in_launch_shows_itself() {
    // Otherwise the app is running with no window and nothing to click.
    assert!(!startup_setting::starts_hidden(true, false));
}

// ---- Linux: the states read off the autostart files (§17) ------------------------------------

const OURS: AutostartEntry = AutostartEntry {
    ours: true,
    hidden: false,
    gnome_disabled: false,
};

fn found(
    writable: bool,
    user: Option<AutostartEntry>,
    system: Option<AutostartEntry>,
) -> AutostartFound {
    AutostartFound {
        writable,
        user,
        system,
    }
}

/// §17's table, row by row.
#[test]
fn the_files_found_are_the_six_states() {
    // No file: off, may be switched on.
    assert_eq!(
        StartupState::from_found(&found(true, None, None)),
        StartupState::Disabled
    );
    // Our file, enabled: on.
    assert_eq!(
        StartupState::from_found(&found(true, Some(OURS), None)),
        StartupState::Enabled
    );
    // Our file switched off by another tool, either way.
    let hidden = AutostartEntry {
        hidden: true,
        ..OURS
    };
    let gnome_off = AutostartEntry {
        gnome_disabled: true,
        ..OURS
    };
    assert_eq!(
        StartupState::from_found(&found(true, Some(hidden), None)),
        StartupState::DisabledByUser
    );
    assert_eq!(
        StartupState::from_found(&found(true, Some(gnome_off), None)),
        StartupState::DisabledByUser
    );
    // An administrator's file in /etc/xdg/autostart, either way — and it decides even over the
    // person's own file and an unwritable home.
    let theirs = AutostartEntry::default();
    assert_eq!(
        StartupState::from_found(&found(true, None, Some(theirs))),
        StartupState::EnabledByPolicy
    );
    assert_eq!(
        StartupState::from_found(&found(
            true,
            None,
            Some(AutostartEntry {
                hidden: true,
                ..theirs
            })
        )),
        StartupState::DisabledByPolicy
    );
    assert_eq!(
        StartupState::from_found(&found(false, Some(hidden), Some(theirs))),
        StartupState::EnabledByPolicy
    );
    // A home that cannot be written: the row is not drawn.
    assert_eq!(
        StartupState::from_found(&found(false, None, None)),
        StartupState::Unavailable
    );
    assert_eq!(
        StartupState::from_found(&found(false, Some(OURS), None)),
        StartupState::Unavailable
    );
}

/// A file of our name that another tool wrote (GNOME Tweaks' "Startup Applications" copies the
/// app's own desktop file) starts the app all the same: on, and the person may take it away from
/// here; switched off there, it is their choice made elsewhere.
#[test]
fn a_file_another_tool_wrote_is_read_for_what_it_does() {
    let tweaks = AutostartEntry::read(
        "[Desktop Entry]\nType=Application\nName=Family Connect\nExec=family-connect %U\n",
    );
    assert!(!tweaks.ours);
    assert_eq!(
        StartupState::from_found(&found(true, Some(tweaks), None)),
        StartupState::Enabled
    );
    let switched_off = AutostartEntry::read(
        "[Desktop Entry]\nType=Application\nExec=family-connect\nX-GNOME-Autostart-enabled=false\n",
    );
    assert_eq!(
        StartupState::from_found(&found(true, Some(switched_off), None)),
        StartupState::DisabledByUser
    );
}

/// The entry is read as GLib's key file reads it: the `[Desktop Entry]` group only, exact keys,
/// comments and blank lines skipped.
#[test]
fn an_entry_is_read_from_its_desktop_entry_group() {
    let ours = AutostartEntry::read(&entry_text("/usr/bin/family-connect"));
    assert_eq!(ours, OURS);
    assert!(ours.starts());

    let hidden = AutostartEntry::read(
        "# switched off\n[Desktop Entry]\nHidden = true\nX-FamilyConnect-Autostart=1\n",
    );
    assert!(hidden.hidden && hidden.ours && !hidden.starts());

    // Another group's keys say nothing about the entry.
    let elsewhere = AutostartEntry::read(
        "[Desktop Entry]\nType=Application\n\n[Desktop Action quit]\nHidden=true\nX-GNOME-Autostart-enabled=false\n",
    );
    assert!(elsewhere.starts());
    // A localised or differently cased key is another key.
    assert!(AutostartEntry::read("[Desktop Entry]\nhidden=true\nHidden[de]=true\n").starts());
    // Garbage is an entry that starts nothing special and is not ours.
    assert_eq!(
        AutostartEntry::read("\u{0}\u{1}not a key file"),
        AutostartEntry::default()
    );
}

/// §17's file, byte for byte, with the running binary's path.
#[test]
fn the_entry_is_the_one_the_design_spells() {
    assert_eq!(
        entry_text("/usr/bin/family-connect"),
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Family Connect\n\
         Exec=/usr/bin/family-connect --background\n\
         Icon=me.nettrash.FamilyConnect\n\
         X-GNOME-Autostart-enabled=true\n\
         NoDisplay=true\n\
         X-FamilyConnect-Autostart=1\n"
    );
    assert_eq!(
        startup_setting::ENTRY_FILE_NAME,
        "me.nettrash.FamilyConnect.desktop"
    );
    assert_eq!(startup_setting::BACKGROUND_ARG, "--background");
}

/// The tarball unpacked anywhere: a path with a space, a quote, a dollar or a backslash is quoted
/// by the Desktop Entry specification's rules, then escaped for the key file.
#[test]
fn a_path_that_needs_quoting_is_quoted() {
    assert_eq!(
        exec_argument("/usr/bin/family-connect"),
        "/usr/bin/family-connect"
    );
    assert_eq!(
        exec_argument("/home/ann/My Apps/family-connect"),
        "\"/home/ann/My Apps/family-connect\""
    );
    assert_eq!(exec_argument("/opt/a\"b/fc"), "\"/opt/a\\\\\"b/fc\"");
    assert_eq!(exec_argument("/opt/$HOME/fc"), "\"/opt/\\\\$HOME/fc\"");
    // A backslash: escaped inside the quotes, then that escape escaped for the key file — four.
    assert_eq!(exec_argument("/opt/a\\b/fc"), "\"/opt/a\\\\\\\\b/fc\"");
    // A per cent is a field code's start, doubled with or without quotes.
    assert_eq!(exec_argument("/opt/100%/fc"), "/opt/100%%/fc");
    assert_eq!(exec_argument(""), "\"\"");
    assert!(
        entry_text("/home/ann/My Apps/family-connect")
            .contains("Exec=\"/home/ann/My Apps/family-connect\" --background\n")
    );
}

/// The row's sentences, one per note, in English as the catalogue's keys.
#[test]
fn the_rows_words_are_the_linux_ones() {
    let say = common::english();
    assert_eq!(startup_setting::title(&say), "Start when I log in");
    assert_eq!(
        startup_setting::note_text(Note::WhatItDoes, &say),
        "Family Connect opens in the top bar when you log in, so a message or a call reaches you without opening it first."
    );
    assert_eq!(
        startup_setting::note_text(Note::BlockedByDesktop, &say),
        "Turned off in your desktop's startup settings."
    );
    assert_eq!(
        startup_setting::note_text(Note::DecidedByPolicy, &say),
        "Your organisation decides whether Family Connect starts when you log in."
    );
}
