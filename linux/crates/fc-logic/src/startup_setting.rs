//! What the "Start when I log in" row shows, and whether it may be touched (`StartupSetting.cs`,
//! in Linux terms: shell-design §17 and Appendix B item 2).
//!
//! On Windows the state is `Windows.ApplicationModel.StartupTaskState`. On Linux the app writes the
//! XDG autostart entry itself (`$XDG_CONFIG_HOME/autostart/me.nettrash.FamilyConnect.desktop`, not
//! the Background portal — §17's decision), so the SAME six states are read off the files the
//! shell finds ([`StartupState::from_found`]), and Windows' rules apply to them unchanged.
//!
//! Three of the five offered states are ones the app will not change, and they are the reason this
//! is a rule rather than a bool. A toggle that looks live and does nothing is the failure mode: on
//! Windows `RequestEnableAsync` on a task the person disabled in Task Manager answers
//! `DisabledByUser` and changes nothing; on Linux a file the person switched off in another tool
//! (`Hidden=true`, `X-GNOME-Autostart-enabled=false`) is THEIR choice, and an administrator's file
//! in `/etc/xdg/autostart` is the organisation's — so the row says where the real switch is
//! instead of springing back on its own.
//!
//! The shell does the file system (reads the two places, writes the entry atomically, removes it);
//! everything decided about what it found is here, and tested.

use fc_core::text::string_catalog::StringCatalog;

/// The autostart entry's file name, in `$XDG_CONFIG_HOME/autostart` and (an administrator's) in
/// `$XDG_CONFIG_DIRS/autostart` — `/etc/xdg/autostart`.
pub const ENTRY_FILE_NAME: &str = "me.nettrash.FamilyConnect.desktop";

/// The argument an autostart launch carries, so the app knows nobody asked to look at it.
pub const BACKGROUND_ARG: &str = "--background";

/// The key the app writes into its own entry, so a file of the same name written by another tool
/// is told apart from ours.
pub const OWN_MARK: &str = "X-FamilyConnect-Autostart";

/// Whether the app may start when its person logs in — Windows' `StartupTaskState`, read on Linux
/// from the autostart files (§17's table).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StartupState {
    /// No entry. The app may write one.
    Disabled,
    /// An entry that starts the app, and the app may remove it.
    Enabled,
    /// The PERSON turned it off outside the app — the entry says `Hidden=true` or
    /// `X-GNOME-Autostart-enabled=false` (Windows: Task Manager). Not the app's to undo, so the
    /// row says where the switch is.
    DisabledByUser,
    /// An administrator's entry of that name in `/etc/xdg/autostart` that does not start it.
    DisabledByPolicy,
    /// An administrator's entry of that name in `/etc/xdg/autostart` that starts it.
    EnabledByPolicy,
    /// `$XDG_CONFIG_HOME/autostart` cannot be written (Windows: no startup task — an unpackaged
    /// run). The row is not drawn.
    Unavailable,
}

/// What an autostart entry says, as far as these rules read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AutostartEntry {
    /// It carries [`OWN_MARK`]`=1`: the app wrote it.
    pub ours: bool,
    /// `Hidden=true` — the XDG specification's "this entry does not exist".
    pub hidden: bool,
    /// `X-GNOME-Autostart-enabled=false` — GNOME's own off switch.
    pub gnome_disabled: bool,
}

impl AutostartEntry {
    /// Read the `[Desktop Entry]` group of a desktop file (the XDG Desktop Entry key-file format):
    /// keys are matched exactly, values `true`/`false` and `1` as the specification and GLib's key
    /// file read booleans; other groups, comments and anything unreadable are ignored — a file
    /// somebody edited by hand is still a file to read.
    pub fn read(text: &str) -> AutostartEntry {
        let mut entry = AutostartEntry::default();
        let mut in_entry_group = false;
        for line in text.lines() {
            let line = line.trim_start();
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            if line.starts_with('[') {
                in_entry_group = line.trim_end() == "[Desktop Entry]";
                continue;
            }
            if !in_entry_group {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let (key, value) = (key.trim_end(), value.trim());
            match key {
                "Hidden" => entry.hidden = value == "true",
                "X-GNOME-Autostart-enabled" => entry.gnome_disabled = value == "false",
                OWN_MARK => entry.ours = value == "1" || value == "true",
                _ => {}
            }
        }
        entry
    }

    /// Whether this entry starts the app at login.
    pub fn starts(&self) -> bool {
        !self.hidden && !self.gnome_disabled
    }
}

/// What the shell found on disk, before anything is judged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AutostartFound {
    /// `$XDG_CONFIG_HOME/autostart` exists and is writable, or can be created.
    pub writable: bool,
    /// The file of [`ENTRY_FILE_NAME`] in `$XDG_CONFIG_HOME/autostart`, read.
    pub user: Option<AutostartEntry>,
    /// A file of that name in a system autostart directory (`/etc/xdg/autostart`), read.
    pub system: Option<AutostartEntry>,
}

impl StartupState {
    /// §17's table. An administrator's entry decides first (the organisation's choice, and the row
    /// says so); then a home that cannot be written offers nothing; then the person's own file:
    /// none is off, one that starts the app is on (whoever wrote it — the person put it there, and
    /// may take it away from here), one that is switched off is THEIR choice made elsewhere.
    pub fn from_found(found: &AutostartFound) -> StartupState {
        if let Some(system) = found.system {
            return if system.starts() {
                StartupState::EnabledByPolicy
            } else {
                StartupState::DisabledByPolicy
            };
        }
        if !found.writable {
            return StartupState::Unavailable;
        }
        match found.user {
            None => StartupState::Disabled,
            Some(entry) if entry.starts() => StartupState::Enabled,
            Some(_) => StartupState::DisabledByUser,
        }
    }
}

/// Which sentence belongs under the row. The caller turns these into localised text
/// ([`note_text`]); they are a closed set so no branch can go unwritten.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Note {
    /// Ordinary: what turning it on does.
    WhatItDoes,
    /// The person switched it off outside this app (Windows: in Task Manager; Linux: in the
    /// desktop's startup settings).
    BlockedByDesktop,
    /// An administrator decided, either way.
    DecidedByPolicy,
}

/// Whether the row is drawn at all.
pub fn is_offered(state: StartupState) -> bool {
    state != StartupState::Unavailable
}

/// Where the toggle sits.
pub fn is_on(state: StartupState) -> bool {
    matches!(state, StartupState::Enabled | StartupState::EnabledByPolicy)
}

/// Whether this app can change it.
pub fn is_changeable(state: StartupState) -> bool {
    matches!(state, StartupState::Disabled | StartupState::Enabled)
}

pub fn note_for(state: StartupState) -> Note {
    match state {
        StartupState::DisabledByUser => Note::BlockedByDesktop,
        StartupState::DisabledByPolicy | StartupState::EnabledByPolicy => Note::DecidedByPolicy,
        _ => Note::WhatItDoes,
    }
}

/// The row's own words (shell-design §13 and §17 — Linux sentences, `linux/i18n/linux.json`).
pub fn title(say: &dyn StringCatalog) -> String {
    say.get("Start when I log in").to_string()
}

/// The sentence under the row.
pub fn note_text(note: Note, say: &dyn StringCatalog) -> String {
    match note {
        Note::WhatItDoes => say
            .get("Family Connect opens in the top bar when you log in, so a message or a call reaches you without opening it first.")
            .to_string(),
        Note::BlockedByDesktop => say
            .get("Turned off in your desktop's startup settings.")
            .to_string(),
        Note::DecidedByPolicy => say
            .get("Your organisation decides whether Family Connect starts when you log in.")
            .to_string(),
    }
}

/// Whether a launch should draw a window. A launch the desktop made at login is not one anybody
/// asked to look at, so it stays in the top bar — but only when there IS an icon to come back
/// from: a hidden window with no icon is an app nobody can reach. The same rule as
/// [`crate::tray_rules::starts_hidden`] (the Mac's `MenuBarRules.startsHidden`).
pub fn starts_hidden(from_autostart: bool, tray_available: bool) -> bool {
    crate::tray_rules::starts_hidden(from_autostart, tray_available)
}

/// What a request to turn it ON actually achieved, given the state read back after it. False means
/// the row must go back to off — the request was refused (the file could not be written, an
/// administrator's entry wins), and pretending otherwise is the bug this exists to prevent.
pub fn request_succeeded(after: StartupState) -> bool {
    is_on(after)
}

/// The entry the app writes, exactly as §17 spells it, starting `executable` (the running binary's
/// absolute path, so the tarball's copy starts itself) with [`BACKGROUND_ARG`]. The path is quoted
/// by the Desktop Entry specification's rules when it needs to be ("/opt/My Apps/family-connect").
pub fn entry_text(executable: &str) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Family Connect\n\
         Exec={} {BACKGROUND_ARG}\n\
         Icon=me.nettrash.FamilyConnect\n\
         X-GNOME-Autostart-enabled=true\n\
         NoDisplay=true\n\
         {OWN_MARK}=1\n",
        exec_argument(executable)
    )
}

/// One argument of an `Exec` key (Desktop Entry Specification, "The Exec key"): as it is when it
/// holds no reserved character; otherwise in double quotes, with `"`, `` ` ``, `$` and `\`
/// backslash-escaped inside them. A `%` is a field code's start and is always doubled. Then the
/// key file's own escaping: a backslash is written `\\`, so a backslash in the path is four in
/// the file. A line break cannot be in a value at all and is written `\n` — which no real path
/// holds.
pub fn exec_argument(argument: &str) -> String {
    const RESERVED: &[char] = &[
        ' ', '\t', '\n', '"', '\'', '\\', '>', '<', '~', '|', '&', ';', '$', '*', '?', '#', '(',
        ')', '`',
    ];
    let quoted = argument.contains(RESERVED) || argument.is_empty();
    let mut exec = String::new();
    if quoted {
        exec.push('"');
    }
    for ch in argument.chars() {
        match ch {
            '%' => exec.push_str("%%"),
            '"' | '`' | '$' | '\\' if quoted => {
                exec.push('\\');
                exec.push(ch);
            }
            _ => exec.push(ch),
        }
    }
    if quoted {
        exec.push('"');
    }
    // The key file's escapes, over the whole value.
    exec.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
        .replace('\r', "\\r")
}
