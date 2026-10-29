//! The app's log (shell-design §26): GLib logging under the domain `family-connect` — journald on
//! the desktop (`journalctl --user -t family-connect`), stderr in a terminal. Info and debug lines
//! are GLib's to hide unless `G_MESSAGES_DEBUG=family-connect` (or `all`) is set; warnings always
//! show.
//!
//! **What a line may carry** (port-plan "Logs", §26): ids, counts, codes, a state's name and an
//! error's KIND. Never a message's text, a name, a token, a server URL with a path, a file name a
//! person chose, or a sound's bytes. Every macro here formats like `format!`; the caller keeps to
//! the rule — there is no filter after the fact.
//!
//! ```text
//! crate::fc_info!("notify: chat {chat_id} raised");
//! crate::fc_warn!("cache: {}", log::store_error(&error));
//! crate::fc_debug!("socket: link {link:?}");
//! ```
//!
//! No `diagnostics.log` file is kept (§26, D20). The app never sets `GST_DEBUG` (souphttpsrc
//! prints the `Authorization` header at LOG level).

use fc_core::store::StoreError;

/// The log domain every line of the app is written under.
pub const DOMAIN: &str = "family-connect";

/// Writes one line at `level` under [`DOMAIN`]. The macros call this.
pub fn write(level: glib::LogLevel, line: &str) {
    glib::g_log!(DOMAIN, level, "{}", line);
}

/// An informational line (hidden unless `G_MESSAGES_DEBUG` names the domain).
#[macro_export]
macro_rules! fc_info {
    ($($arg:tt)*) => {
        $crate::services::log::write(glib::LogLevel::Info, &format!($($arg)*))
    };
}

/// A warning: something failed that the app went on without.
#[macro_export]
macro_rules! fc_warn {
    ($($arg:tt)*) => {
        $crate::services::log::write(glib::LogLevel::Warning, &format!($($arg)*))
    };
}

/// A debug line (hidden unless `G_MESSAGES_DEBUG` names the domain).
#[macro_export]
macro_rules! fc_debug {
    ($($arg:tt)*) => {
        $crate::services::log::write(glib::LogLevel::Debug, &format!($($arg)*))
    };
}

/// A cache failure as a log may say it: the kind and SQLite's code, never a row.
pub fn store_error(error: &StoreError) -> String {
    match error {
        StoreError::Sqlite(rusqlite_error) => match rusqlite_error.sqlite_error_code() {
            Some(code) => format!("sqlite {code:?}"),
            None => "sqlite (no code)".to_string(),
        },
        StoreError::NewerSchema { found, known } => {
            format!("newer schema {found} (this build knows {known})")
        }
        StoreError::UnknownSchema { found } => format!("unknown schema {found}"),
    }
}
