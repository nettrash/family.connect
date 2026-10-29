//! The autostart entry on disk (shell-design §17) — what the files say, read and written with
//! std, no GTK: tested with temporary folders. Every DECISION about what was found is fc-logic's
//! `startup_setting` (`StartupState::from_found`, `entry_text`); this is the file system around it.
//!
//! - The person's entry: `$XDG_CONFIG_HOME/autostart/me.nettrash.FamilyConnect.desktop`.
//! - An administrator's: a file of that name in `<dir>/autostart` for each of `$XDG_CONFIG_DIRS`
//!   (`/etc/xdg`), the first one found.
//! - Written ATOMICALLY: a part file in the same folder renamed over the entry, so a session
//!   starting at that moment reads the old file or the new one, never half of one.

use std::io;
use std::path::{Path, PathBuf};

use fc_logic::startup_setting::{AutostartEntry, AutostartFound, ENTRY_FILE_NAME};

/// The entry's path in a folder.
pub fn entry_path(dir: &Path) -> PathBuf {
    dir.join(ENTRY_FILE_NAME)
}

/// Reads an entry, when the file exists and is text.
pub fn read_entry(dir: &Path) -> Option<AutostartEntry> {
    let text = std::fs::read_to_string(entry_path(dir)).ok()?;
    Some(AutostartEntry::read(&text))
}

/// What the two places say. `user_dir` is `$XDG_CONFIG_HOME/autostart`; `system_dirs` the
/// administrator's autostart folders in order; `writable` answers whether a folder may be written.
pub fn find(
    user_dir: &Path,
    system_dirs: &[PathBuf],
    writable: impl Fn(&Path) -> bool,
) -> AutostartFound {
    AutostartFound {
        writable: can_create_in(user_dir, &writable),
        user: read_entry(user_dir),
        system: system_dirs.iter().find_map(|dir| read_entry(dir)),
    }
}

/// Whether `dir` is writable, or could be created: the nearest folder that exists, going up,
/// must be writable.
pub fn can_create_in(dir: &Path, writable: &impl Fn(&Path) -> bool) -> bool {
    let mut at = Some(dir);
    while let Some(path) = at {
        if path.is_dir() {
            return writable(path);
        }
        if path.exists() {
            // A file where the folder should be.
            return false;
        }
        at = path.parent();
    }
    false
}

/// Writes `text` as the entry in `dir`, creating the folder; atomically.
pub fn write_entry(dir: &Path, text: &str) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let part = dir.join(format!(".{ENTRY_FILE_NAME}.{}.part", std::process::id()));
    let written =
        std::fs::write(&part, text).and_then(|()| std::fs::rename(&part, entry_path(dir)));
    if written.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    written
}

/// Removes the entry from `dir`; one that is not there is removed already.
pub fn remove_entry(dir: &Path) -> io::Result<()> {
    match std::fs::remove_file(entry_path(dir)) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// The administrator's autostart folders for `config_dirs` (`$XDG_CONFIG_DIRS`).
pub fn system_autostart_dirs(config_dirs: &[PathBuf]) -> Vec<PathBuf> {
    config_dirs
        .iter()
        .map(|dir| dir.join("autostart"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use fc_logic::startup_setting::{StartupState, entry_text};

    /// A folder under the temp directory, removed on drop.
    struct TempFolder(PathBuf);

    impl TempFolder {
        fn new(tag: &str) -> TempFolder {
            let path =
                std::env::temp_dir().join(format!("fc-autostart-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            TempFolder(path)
        }
    }

    impl Drop for TempFolder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn state(user: &Path, system: &[PathBuf]) -> StartupState {
        StartupState::from_found(&find(user, system, |_| true))
    }

    #[test]
    fn writing_and_removing_the_entry_moves_the_state() {
        let home = TempFolder::new("rw");
        let user = home.0.join("config").join("autostart");
        let system = vec![home.0.join("etc").join("autostart")];
        assert_eq!(state(&user, &system), StartupState::Disabled);
        write_entry(&user, &entry_text("/usr/bin/family-connect")).unwrap();
        assert_eq!(state(&user, &system), StartupState::Enabled);
        let written = std::fs::read_to_string(entry_path(&user)).unwrap();
        assert!(written.contains("Exec=/usr/bin/family-connect --background\n"));
        assert!(written.contains("X-FamilyConnect-Autostart=1\n"));
        assert!(read_entry(&user).unwrap().ours);
        // No part file is left behind.
        let names: Vec<String> = std::fs::read_dir(&user)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec![ENTRY_FILE_NAME.to_string()]);
        remove_entry(&user).unwrap();
        assert_eq!(state(&user, &system), StartupState::Disabled);
        // Removing what is not there is fine.
        remove_entry(&user).unwrap();
    }

    #[test]
    fn an_entry_switched_off_elsewhere_is_the_persons_and_an_administrators_wins() {
        let home = TempFolder::new("states");
        let user = home.0.join("autostart");
        let etc = home.0.join("etc-autostart");
        let system = vec![home.0.join("nowhere"), etc.clone()];
        write_entry(
            &user,
            "[Desktop Entry]\nExec=x --background\nX-GNOME-Autostart-enabled=false\n",
        )
        .unwrap();
        assert_eq!(state(&user, &system), StartupState::DisabledByUser);
        write_entry(&user, "[Desktop Entry]\nExec=x\nHidden=true\n").unwrap();
        assert_eq!(state(&user, &system), StartupState::DisabledByUser);
        write_entry(&etc, "[Desktop Entry]\nExec=x\n").unwrap();
        assert_eq!(state(&user, &system), StartupState::EnabledByPolicy);
        write_entry(&etc, "[Desktop Entry]\nExec=x\nHidden=true\n").unwrap();
        assert_eq!(state(&user, &system), StartupState::DisabledByPolicy);
    }

    #[test]
    fn a_home_that_cannot_be_written_offers_nothing() {
        let home = TempFolder::new("ro");
        let user = home.0.join("a").join("autostart");
        // Not writable: the nearest existing folder up is asked.
        let asked = std::cell::RefCell::new(Vec::new());
        let found = find(&user, &[], |path| {
            asked.borrow_mut().push(path.to_path_buf());
            false
        });
        assert_eq!(StartupState::from_found(&found), StartupState::Unavailable);
        assert_eq!(asked.borrow().as_slice(), std::slice::from_ref(&home.0));
        // A file where the folder should be.
        let blocked = home.0.join("file");
        std::fs::write(&blocked, "x").unwrap();
        assert!(!can_create_in(&blocked.join("autostart"), &|_| true));
        assert_eq!(
            system_autostart_dirs(&[PathBuf::from("/etc/xdg")]),
            vec![PathBuf::from("/etc/xdg/autostart")]
        );
    }
}
