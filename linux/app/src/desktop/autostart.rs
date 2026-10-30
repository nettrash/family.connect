//! Launch at login (shell-design §17, D5): the app writes the XDG autostart entry itself — not the
//! Background portal, which for a host app would write the same file after asking for a registered
//! app id and possibly a dialog, and whose "Background Apps" list tracks sandboxed apps only.
//!
//! `Exec` is the RUNNING binary's absolute path with `--background` (`startup_setting::entry_text`),
//! so the tarball's copy starts itself. The five states are fc-logic's (`StartupState::from_found`
//! over [`autostart_file::find`](super::autostart_file::find)); a request the rules do not allow —
//! an entry the person switched off elsewhere, an administrator's — changes nothing, and the
//! answer is the state read back after (`startup_setting::request_succeeded` judges it).

use std::path::{Path, PathBuf};

use fc_logic::startup_setting::{self, StartupState};
use gio::prelude::*;

use super::autostart_file;
use crate::seams::desktop::AutostartState;

/// The "Start when I log in" switch over the autostart entry.
pub struct Autostart {
    user_dir: PathBuf,
    system_dirs: Vec<PathBuf>,
}

impl Autostart {
    /// `user_dir` is `$XDG_CONFIG_HOME/autostart` (`Paths::autostart_dir`); the administrator's
    /// folders are `$XDG_CONFIG_DIRS`' (`/etc/xdg/autostart`).
    pub fn new(user_dir: &Path) -> Autostart {
        Autostart {
            user_dir: user_dir.to_path_buf(),
            system_dirs: autostart_file::system_autostart_dirs(&glib::system_config_dirs()),
        }
    }

    /// The running binary as an `Exec` may name it.
    fn executable() -> Option<String> {
        let exe = std::env::current_exe().ok()?;
        let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
        exe.to_str().map(str::to_string)
    }
}

/// Whether the process may write in `dir` (`access(2)` through GIO).
fn writable(dir: &Path) -> bool {
    gio::File::for_path(dir)
        .query_info(
            gio::FILE_ATTRIBUTE_ACCESS_CAN_WRITE.as_str(),
            gio::FileQueryInfoFlags::NONE,
            None::<&gio::Cancellable>,
        )
        .map(|info| info.boolean(gio::FILE_ATTRIBUTE_ACCESS_CAN_WRITE.as_str()))
        .unwrap_or(false)
}

impl AutostartState for Autostart {
    fn state(&self) -> StartupState {
        StartupState::from_found(&autostart_file::find(
            &self.user_dir,
            &self.system_dirs,
            writable,
        ))
    }

    fn set_enabled(&self, enabled: bool) -> StartupState {
        let before = self.state();
        if !startup_setting::is_changeable(before) || startup_setting::is_on(before) == enabled {
            return before;
        }
        let done = if enabled {
            match Autostart::executable() {
                Some(exe) => {
                    autostart_file::write_entry(&self.user_dir, &startup_setting::entry_text(&exe))
                }
                None => Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "no executable path",
                )),
            }
        } else {
            autostart_file::remove_entry(&self.user_dir)
        };
        match done {
            Ok(()) => crate::fc_info!(
                "autostart: entry {}",
                if enabled { "written" } else { "removed" }
            ),
            Err(error) => crate::fc_warn!(
                "autostart: the entry could not be changed ({})",
                error.kind()
            ),
        }
        self.state()
    }
}
