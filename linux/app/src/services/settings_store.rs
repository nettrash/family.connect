//! `SettingsStore` — this device's settings in a `glib::KeyFile` at
//! `$XDG_CONFIG_HOME/family-connect/settings.conf`, one group `[family-connect]` (shell-design §0,
//! §2.3, §13; md's `KeyFileSettingsStore`, md shell-design §9.1).
//!
//! Why GKeyFile: it is in GLib already, it escapes what a value needs (a tab, a newline), and
//! `save_to_file` is `g_file_set_contents` — a temporary file renamed over the old one, so a crash
//! never leaves half a settings file.
//!
//! **The frozen keys** are in [`keys`], each with its type and default; a package that needs a new
//! device setting declares its key in its own module, documented the same way, and reads it with
//! the typed getters ([`SettingsStore::bool`], [`string`](SettingsStore::string),
//! [`int`](SettingsStore::int)). A value is stored as text and read by the getter's type, so a key
//! read with the wrong getter is simply absent.
//!
//! **Semantics.** Every `set_*`/`remove` writes the file through (settings change at human speed);
//! [`SettingsStore::changed`] fires once per key whose stored text really changed, after the file
//! was written, with no borrow held (a handler may read or write the store).
//!
//! **Failure modes** (md's): a file that does not parse is renamed `settings.conf.corrupt` and the
//! store starts empty; a file that cannot be read (permissions, a folder in its place) is left
//! alone and the store runs in memory; a folder that cannot be written keeps the store in memory
//! with one warning in the log.
//!
//! **The session's two memories** — the family last shown per account, and the join request this
//! device waits on — live here too (carry-forward, "For the shell (wave 3) — the session layer":
//! "both memories belong in the shell's settings file"): [`SettingsCurrentFamily`] and
//! [`SettingsAwaitingJoin`] implement fc-logic's `CurrentFamily` and `AwaitingJoin` over
//! [`keys::CURRENT_FAMILY`] and [`keys::AWAITING_JOIN`]. They are this device's, kept across
//! sign-outs, and keyed by the server so a second server never reads the first one's.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use fc_core::rt::Event;
use fc_logic::app_session::CurrentFamily;
use fc_logic::join_memory::AwaitingJoin;

/// The one group of `settings.conf`.
const GROUP: &str = "family-connect";
/// Appended to the name of a settings file that did not parse.
const CORRUPT_SUFFIX: &str = ".corrupt";

/// The frozen keys (W.3). Each line: the key, its type, its default, and the package that draws it.
pub mod keys {
    /// String — the server the reader chose, as `ServerUrl`'s base (`https://chat.example.com/`).
    /// Absent until one is chosen (or the build's default is used). S0.
    pub const SERVER: &str = "server";
    /// Int — the main window's last width, logical pixels. Absent: 1000. S0.
    pub const WINDOW_WIDTH: &str = "window-width";
    /// Int — the main window's last height, logical pixels. Absent: 680. S0.
    pub const WINDOW_HEIGHT: &str = "window-height";
    /// Bool — the main window was maximised. Default false. S0.
    pub const WINDOW_MAXIMIZED: &str = "window-maximized";
    /// Bool — "Tell me when a message arrives". Default true. Row: P4; read by the notifications.
    pub const NOTIFY: &str = "notify";
    /// Bool — "Keep running in the top bar". Default true. Row: P4; read by the close rule (S0).
    pub const KEEP_RUNNING: &str = "keep-running";
    /// Bool — "Open with Ctrl+Alt+Shift+F". Default true. Row: P4; the portal session: P5.
    pub const HOT_KEY: &str = "hot-key";
    /// Bool — "Link Previews". Default true. Row: P4; read by `LinkPreviews` (S0's composition).
    pub const LINK_PREVIEWS: &str = "link-previews";
    /// Bool — "Map Previews". Default true. Row: P4; read by P2's map tiles.
    pub const MAP_PREVIEWS: &str = "map-previews";
    /// String — the voice-message speed, `fc_logic::voice_look::voice_speed::store`'s text; read
    /// back with `voice_speed::parse` (absent or unreadable: 1×). Row: P4; applied by P3.
    pub const VOICE_SPEED: &str = "voice-speed";
    /// String — the camera chosen for video messages (P3's device id); absent or empty:
    /// "Automatic". Row: P4; used by P3.
    pub const VIDEO_CAMERA: &str = "video-camera";
    /// String — the family last shown per account, one line per account:
    /// `<server base>\t<user id>\t<family id>`. [`super::SettingsCurrentFamily`]. S0.
    pub const CURRENT_FAMILY: &str = "current-family";
    /// String — the join request this device waits on: `<server base>\t<user id>`.
    /// [`super::SettingsAwaitingJoin`]. S0.
    pub const AWAITING_JOIN: &str = "awaiting-join";

    /// The default of every frozen boolean key.
    pub fn default_bool(key: &str) -> bool {
        matches!(
            key,
            NOTIFY | KEEP_RUNNING | HOT_KEY | LINK_PREVIEWS | MAP_PREVIEWS
        )
    }
}

/// This device's settings, persisted in `settings.conf`. One per process ([`Shell::settings`]).
///
/// [`Shell::settings`]: crate::shell::Shell::settings
pub struct SettingsStore {
    path: Option<PathBuf>,
    file: RefCell<glib::KeyFile>,
    /// False when the file exists but could not be read: never overwrite what we could not see.
    persist: bool,
    /// The one warning for an unwritable folder has been given.
    warned: Cell<bool>,
    changed: Event<str>,
}

impl std::fmt::Debug for SettingsStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingsStore")
            .field("persist", &self.persist)
            .finish_non_exhaustive()
    }
}

impl SettingsStore {
    /// Loads `path` (missing → empty; corrupt → renamed aside, empty; unreadable → empty, in
    /// memory only).
    pub fn open(path: PathBuf) -> SettingsStore {
        let (file, persist) = match load(&path) {
            Load::File(file) => (file, true),
            Load::Missing => (glib::KeyFile::new(), true),
            Load::Corrupt(message) => {
                let aside = corrupt_path(&path);
                match std::fs::rename(&path, &aside) {
                    Ok(()) => crate::fc_warn!(
                        "settings could not be read ({message}); kept aside as settings.conf{CORRUPT_SUFFIX}, starting with none"
                    ),
                    Err(err) => crate::fc_warn!(
                        "settings could not be read ({message}) nor moved aside ({}); starting with none",
                        err.kind()
                    ),
                }
                (glib::KeyFile::new(), true)
            }
            Load::Unreadable(message) => {
                crate::fc_warn!("settings could not be read ({message}); they stay in memory");
                (glib::KeyFile::new(), false)
            }
        };
        SettingsStore {
            path: Some(path),
            file: RefCell::new(file),
            persist,
            warned: Cell::new(false),
            changed: Event::new(),
        }
    }

    /// A store that never touches a disk (tests).
    #[cfg(test)]
    pub fn in_memory() -> SettingsStore {
        SettingsStore {
            path: None,
            file: RefCell::new(glib::KeyFile::new()),
            persist: false,
            warned: Cell::new(false),
            changed: Event::new(),
        }
    }

    /// The stored text of `key`, unparsed.
    fn raw(&self, key: &str) -> Option<String> {
        self.file
            .borrow()
            .value(GROUP, key)
            .ok()
            .map(|value| value.to_string())
    }

    /// A string value (unescaped).
    pub fn string(&self, key: &str) -> Option<String> {
        self.file
            .borrow()
            .string(GROUP, key)
            .ok()
            .map(|value| value.to_string())
    }

    pub fn set_string(&self, key: &str, value: &str) {
        self.change(key, |file| file.set_string(GROUP, key, value));
    }

    /// A boolean value, or `default` when absent or not a boolean.
    pub fn bool(&self, key: &str, default: bool) -> bool {
        self.file.borrow().boolean(GROUP, key).unwrap_or(default)
    }

    pub fn set_bool(&self, key: &str, value: bool) {
        self.change(key, |file| file.set_boolean(GROUP, key, value));
    }

    /// An integer value, or `None` when absent or not an integer.
    pub fn int(&self, key: &str) -> Option<i64> {
        self.file.borrow().int64(GROUP, key).ok()
    }

    pub fn set_int(&self, key: &str, value: i64) {
        self.change(key, |file| file.set_int64(GROUP, key, value));
    }

    /// Forget `key`.
    pub fn remove(&self, key: &str) {
        self.change(key, |file| {
            let _ = file.remove_key(GROUP, key);
        });
    }

    /// Raised with the key after its stored text changed and the file was written.
    pub fn changed(&self) -> &Event<str> {
        &self.changed
    }

    /// A frozen boolean key at its default ([`keys::default_bool`]).
    pub fn flag(&self, key: &str) -> bool {
        self.bool(key, keys::default_bool(key))
    }

    /// The main window's remembered size, when one is kept.
    pub fn window_size(&self) -> Option<(i32, i32)> {
        let width = self.int(keys::WINDOW_WIDTH)?;
        let height = self.int(keys::WINDOW_HEIGHT)?;
        let width = i32::try_from(width).ok().filter(|w| *w > 0)?;
        let height = i32::try_from(height).ok().filter(|h| *h > 0)?;
        Some((width, height))
    }

    fn change(&self, key: &str, edit: impl FnOnce(&glib::KeyFile)) {
        let before = self.raw(key);
        edit(&self.file.borrow());
        if self.raw(key) == before {
            return;
        }
        self.save();
        // No borrow is held: a handler may read (or write) the store.
        self.changed.emit(key);
    }

    /// Writes every value; on failure the store keeps working in memory.
    fn save(&self) {
        let Some(path) = self.path.as_deref().filter(|_| self.persist) else {
            return;
        };
        let written = match path.parent() {
            Some(folder) => {
                std::fs::create_dir_all(folder).map_err(|err| format!("{}", err.kind()))
            }
            None => Ok(()),
        }
        .and_then(|()| {
            self.file
                .borrow()
                .save_to_file(path)
                .map_err(|err| err.to_string())
        });
        if let Err(message) = written
            && !self.warned.replace(true)
        {
            crate::fc_warn!(
                "settings stay in memory: settings.conf could not be written ({message})"
            );
        }
    }
}

enum Load {
    File(glib::KeyFile),
    Missing,
    Corrupt(String),
    Unreadable(String),
}

fn load(path: &Path) -> Load {
    // GLib calls anything but a regular file a *parse* error; it is not a corrupt settings file.
    if let Ok(metadata) = std::fs::metadata(path)
        && !metadata.is_file()
    {
        return Load::Unreadable(String::from("not a regular file"));
    }
    let file = glib::KeyFile::new();
    match file.load_from_file(path, glib::KeyFileFlags::KEEP_COMMENTS) {
        Ok(()) => Load::File(file),
        Err(err) if err.matches(glib::FileError::Noent) || err.matches(glib::FileError::Notdir) => {
            Load::Missing
        }
        Err(err) if err.kind::<glib::KeyFileError>().is_some() => Load::Corrupt(err.to_string()),
        Err(err) => Load::Unreadable(err.to_string()),
    }
}

fn corrupt_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(CORRUPT_SUFFIX);
    path.with_file_name(name)
}

/// fc-logic's `CurrentFamily` (`ICurrentFamily`, Windows' `CurrentFamilyFile`) over
/// [`keys::CURRENT_FAMILY`]: which family this device last showed each account of `server`. Lines
/// of other servers are kept untouched.
pub struct SettingsCurrentFamily {
    settings: Rc<SettingsStore>,
    server: String,
}

impl SettingsCurrentFamily {
    /// `server` spelled the same way every time: `ServerUrl`'s `to_string()`.
    pub fn new(settings: Rc<SettingsStore>, server: &str) -> Self {
        SettingsCurrentFamily {
            settings,
            server: server.to_string(),
        }
    }
}

/// `(server, user, family)` lines; anything else is skipped.
fn family_lines(text: &str) -> Vec<(String, i64, i64)> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let (server, user, family) = (parts.next()?, parts.next()?, parts.next()?);
            if parts.next().is_some() || server.is_empty() {
                return None;
            }
            Some((server.to_string(), digits(user)?, digits(family)?))
        })
        .collect()
}

/// A positive id written as ASCII digits only (.NET's `NumberStyles.None`).
fn digits(text: &str) -> Option<i64> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok().filter(|id: &i64| *id > 0)
}

impl CurrentFamily for SettingsCurrentFamily {
    fn family_for(&self, user_id: i64) -> Option<i64> {
        let text = self.settings.string(keys::CURRENT_FAMILY)?;
        family_lines(&text)
            .into_iter()
            .find(|(server, user, _)| *server == self.server && *user == user_id)
            .map(|(_, _, family)| family)
    }

    fn remember(&self, user_id: i64, family_id: i64) {
        let text = self
            .settings
            .string(keys::CURRENT_FAMILY)
            .unwrap_or_default();
        let mut lines: Vec<(String, i64, i64)> = family_lines(&text)
            .into_iter()
            .filter(|(server, user, _)| !(*server == self.server && *user == user_id))
            .collect();
        lines.push((self.server.clone(), user_id, family_id));
        let written: Vec<String> = lines
            .iter()
            .map(|(server, user, family)| format!("{server}\t{user}\t{family}"))
            .collect();
        self.settings
            .set_string(keys::CURRENT_FAMILY, &written.join("\n"));
    }
}

/// fc-logic's `AwaitingJoin` (`IAwaitingJoin`, Windows' `AwaitingJoinFile`) over
/// [`keys::AWAITING_JOIN`]: whose join request this device waits on, for `server` only.
pub struct SettingsAwaitingJoin {
    settings: Rc<SettingsStore>,
    server: String,
}

impl SettingsAwaitingJoin {
    pub fn new(settings: Rc<SettingsStore>, server: &str) -> Self {
        SettingsAwaitingJoin {
            settings,
            server: server.to_string(),
        }
    }
}

impl AwaitingJoin for SettingsAwaitingJoin {
    fn user_id(&self) -> Option<i64> {
        let text = self.settings.string(keys::AWAITING_JOIN)?;
        let (server, user) = text.split_once('\t')?;
        (server == self.server).then(|| digits(user)).flatten()
    }

    fn set_user_id(&self, user_id: Option<i64>) {
        match user_id {
            Some(id) => self
                .settings
                .set_string(keys::AWAITING_JOIN, &format!("{}\t{id}", self.server)),
            // Only this server's wait is forgotten; another server's stays.
            None if self.user_id().is_some() => self.settings.remove(keys::AWAITING_JOIN),
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh folder under the system temp directory, removed when dropped.
    struct TempFolder(PathBuf);

    impl TempFolder {
        fn new(name: &str) -> TempFolder {
            let dir =
                std::env::temp_dir().join(format!("fc-settings-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("temp folder");
            TempFolder(dir)
        }

        fn settings(&self) -> PathBuf {
            self.0.join("family-connect").join("settings.conf")
        }
    }

    impl Drop for TempFolder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn values_survive_a_reopen_with_their_types() {
        let temp = TempFolder::new("round-trip");
        let store = SettingsStore::open(temp.settings());
        store.set_string(keys::SERVER, "https://chat.example.com/");
        store.set_bool(keys::NOTIFY, false);
        store.set_int(keys::WINDOW_WIDTH, 1200);
        store.set_int(keys::WINDOW_HEIGHT, 800);
        store.set_string(keys::VIDEO_CAMERA, " \tleading\nand\\escapes\r ✓");
        store.set_string(keys::VOICE_SPEED, "1.5");
        store.remove(keys::VOICE_SPEED);
        assert!(temp.settings().is_file());

        let reopened = SettingsStore::open(temp.settings());
        assert_eq!(
            reopened.string(keys::SERVER).as_deref(),
            Some("https://chat.example.com/")
        );
        assert!(!reopened.flag(keys::NOTIFY));
        assert!(reopened.flag(keys::KEEP_RUNNING));
        assert_eq!(reopened.window_size(), Some((1200, 800)));
        assert_eq!(
            reopened.string(keys::VIDEO_CAMERA).as_deref(),
            Some(" \tleading\nand\\escapes\r ✓")
        );
        assert_eq!(reopened.string(keys::VOICE_SPEED), None);
        let text = std::fs::read_to_string(temp.settings()).unwrap();
        assert!(text.starts_with("[family-connect]\n"), "{text}");
        assert!(text.contains("\nnotify=false\n"), "{text}");
    }

    #[test]
    fn a_value_read_with_the_wrong_getter_is_absent() {
        let store = SettingsStore::in_memory();
        store.set_string(keys::WINDOW_WIDTH, "wide");
        store.set_string(keys::NOTIFY, "maybe");
        assert_eq!(store.int(keys::WINDOW_WIDTH), None);
        assert!(store.bool(keys::NOTIFY, true));
        assert!(!store.bool(keys::NOTIFY, false));
        assert!(store.flag(keys::HOT_KEY));
        assert!(!store.flag(keys::WINDOW_MAXIMIZED));
        store.set_int(keys::WINDOW_WIDTH, -5);
        store.set_int(keys::WINDOW_HEIGHT, 400);
        assert_eq!(store.window_size(), None);
    }

    #[test]
    fn changed_fires_once_per_real_change_after_the_file_was_written() {
        let temp = TempFolder::new("changed");
        let store = Rc::new(SettingsStore::open(temp.settings()));
        let seen: Rc<RefCell<Vec<(String, Option<String>)>>> = Rc::new(RefCell::new(Vec::new()));
        {
            let (seen, path, weak) = (Rc::clone(&seen), temp.settings(), Rc::downgrade(&store));
            store.changed().subscribe(move |key: &str| {
                let on_disk = SettingsStore::open(path.clone()).string(key);
                let live = weak.upgrade().and_then(|s| s.string(key));
                assert_eq!(on_disk, live);
                seen.borrow_mut().push((key.to_string(), on_disk));
            });
        }
        store.set_string(keys::SERVER, "a");
        store.set_string(keys::SERVER, "a");
        store.set_string(keys::SERVER, "b");
        store.set_bool(keys::NOTIFY, true);
        store.set_bool(keys::NOTIFY, true);
        store.remove(keys::HOT_KEY);
        store.remove(keys::SERVER);
        store.remove(keys::SERVER);
        let keys: Vec<String> = seen.borrow().iter().map(|(k, _)| k.clone()).collect();
        assert_eq!(
            keys,
            [keys::SERVER, keys::SERVER, keys::NOTIFY, keys::SERVER]
        );
        assert_eq!(seen.borrow()[1].1.as_deref(), Some("b"));
        assert_eq!(seen.borrow()[3].1, None);
    }

    #[test]
    fn a_corrupt_file_is_moved_aside_and_the_store_starts_empty() {
        let temp = TempFolder::new("corrupt");
        let path = temp.settings();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let garbage = b"this is not a key file\n[[[\n\xff\xfe\n";
        std::fs::write(&path, garbage).unwrap();

        let store = SettingsStore::open(path.clone());
        assert_eq!(store.string(keys::SERVER), None);
        assert!(!path.exists());
        let aside = path.with_file_name("settings.conf.corrupt");
        assert_eq!(std::fs::read(&aside).unwrap(), garbage);

        store.set_string(keys::SERVER, "x");
        assert_eq!(
            SettingsStore::open(path).string(keys::SERVER).as_deref(),
            Some("x")
        );
    }

    #[test]
    fn an_unreadable_file_is_never_overwritten_and_an_unwritable_folder_keeps_memory() {
        let temp = TempFolder::new("unreadable");
        std::fs::create_dir_all(temp.settings()).unwrap();
        let store = SettingsStore::open(temp.settings());
        store.set_string(keys::SERVER, "x");
        assert_eq!(store.string(keys::SERVER).as_deref(), Some("x"));
        assert!(temp.settings().is_dir());

        let blocked = TempFolder::new("unwritable");
        std::fs::write(blocked.0.join("family-connect"), "").unwrap();
        let store = SettingsStore::open(blocked.settings());
        store.set_bool(keys::NOTIFY, false);
        assert!(!store.flag(keys::NOTIFY));
        assert!(store.warned.get());
    }

    #[test]
    fn the_family_last_shown_is_kept_per_server_and_per_account() {
        let settings = Rc::new(SettingsStore::in_memory());
        let one = SettingsCurrentFamily::new(Rc::clone(&settings), "https://one.example/");
        let two = SettingsCurrentFamily::new(Rc::clone(&settings), "https://two.example/");
        assert_eq!(one.family_for(7), None);
        one.remember(7, 3);
        one.remember(8, 4);
        two.remember(7, 9);
        one.remember(7, 5);
        assert_eq!(one.family_for(7), Some(5));
        assert_eq!(one.family_for(8), Some(4));
        assert_eq!(two.family_for(7), Some(9));
        assert_eq!(two.family_for(8), None);
        // A damaged line is skipped, never misread.
        settings.set_string(
            keys::CURRENT_FAMILY,
            "https://one.example/\t7\t-1\nhttps://one.example/\t+8\t4\nhttps://one.example/\t9\t2\textra\n\t1\t1",
        );
        assert_eq!(one.family_for(7), None);
        assert_eq!(one.family_for(8), None);
        assert_eq!(one.family_for(9), None);
    }

    #[test]
    fn a_wait_is_remembered_for_its_server_only() {
        let settings = Rc::new(SettingsStore::in_memory());
        let one = SettingsAwaitingJoin::new(Rc::clone(&settings), "https://one.example/");
        let two = SettingsAwaitingJoin::new(Rc::clone(&settings), "https://two.example/");
        assert_eq!(one.user_id(), None);
        one.set_user_id(Some(42));
        assert_eq!(one.user_id(), Some(42));
        assert_eq!(two.user_id(), None);
        two.set_user_id(None);
        assert_eq!(one.user_id(), Some(42));
        one.set_user_id(None);
        assert_eq!(one.user_id(), None);
        assert_eq!(settings.string(keys::AWAITING_JOIN), None);
    }
}
