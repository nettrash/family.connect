//! Where the app reads its bundled data and keeps its state (shell-design §2.3, as amended by
//! carry-forward "Where the cache lives" and "For the shell (wave 3) and the editor ports").
//!
//! **Bundled data** (`data_dir`): `$FC_DATA_DIR` when set; else beside the executable —
//! `<prefix>/share/family-connect` for `<prefix>/bin/family-connect` (the `.deb`, the `.rpm` and the
//! portable archive alike, docs/packaging.md §1), or the checkout's `linux/data` for a binary cargo
//! built in it (`linux/target/<profile>/family-connect`); else `/usr/share/family-connect`; else —
//! debug builds only — the source tree's `linux/data`, so `scripts/dev.sh run` works with its target
//! directory elsewhere. `linux/data/` mirrors `/usr/share/family-connect/`: `i18n/` (written by
//! `linux/i18n/generate.py`) now, `rich/` when P2 adds it.
//!
//! **Per-user folders** come from GLib (`glib::user_*_dir`, which honour the XDG variables) and are
//! created by whoever writes first, never here:
//!
//! | What | Where |
//! | --- | --- |
//! | settings | `$XDG_CONFIG_HOME/family-connect/settings.conf` |
//! | the cache and its outbox (SQLite) | `$XDG_STATE_HOME/family-connect/cache.db` — unsent messages are user data, never `~/.cache` |
//! | staged sends (`FolderMediaStore`) | `$XDG_STATE_HOME/family-connect/media/` |
//! | parked recordings (`ParkedRecordings`) | `$XDG_STATE_HOME/family-connect/parked/` |
//! | files handed to the app (`family-connect FILE…`) | `$XDG_DATA_HOME/family-connect/inbox/` |
//! | downloaded attachments, avatars, rich drawings, map tiles | `$XDG_CACHE_HOME/family-connect/{blobs,avatars,rich,tiles}/` — rebuildable |
//! | transcode and recorder temporaries | `$XDG_CACHE_HOME/family-connect/tmp/`, swept at launch |
//! | the autostart entry | `$XDG_CONFIG_HOME/autostart/me.nettrash.FamilyConnect.desktop` |
//! | the token | the Secret Service — never a file |

use std::path::{Path, PathBuf};

/// The app's folder under each XDG base directory and under `share/`.
pub const APP_FOLDER: &str = "family-connect";
/// The settings file under `$XDG_CONFIG_HOME/family-connect`.
const SETTINGS_FILE_NAME: &str = "settings.conf";
/// The SQLite cache under `$XDG_STATE_HOME/family-connect`.
const CACHE_FILE_NAME: &str = "cache.db";
/// The installed data folder.
const INSTALLED_DATA_DIR: &str = "/usr/share/family-connect";
/// The variable that overrides the data folder (tests, a hand-run build).
const DATA_DIR_VARIABLE: &str = "FC_DATA_DIR";

/// The install folder and the per-user folders, resolved once in `startup`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    data_dir: PathBuf,
    config_dir: PathBuf,
    autostart_dir: PathBuf,
    state_dir: PathBuf,
    user_data_dir: PathBuf,
    cache_dir: PathBuf,
}

impl Paths {
    /// The folders of this process: `$FC_DATA_DIR`, the executable's place, and the XDG variables
    /// as GLib reads them.
    pub fn new() -> Paths {
        let exe = std::env::current_exe().ok();
        let overridden = std::env::var_os(DATA_DIR_VARIABLE).map(PathBuf::from);
        Paths::with_roots(
            data_dir(overridden, exe.as_deref()),
            &glib::user_config_dir(),
            &glib::user_state_dir(),
            &glib::user_data_dir(),
            &glib::user_cache_dir(),
        )
    }

    /// The same layout under given roots (tests; the self-test's redirected profile is reached
    /// through the XDG variables instead).
    pub fn with_roots(
        data_dir: PathBuf,
        config_home: &Path,
        state_home: &Path,
        data_home: &Path,
        cache_home: &Path,
    ) -> Paths {
        Paths {
            data_dir,
            config_dir: config_home.join(APP_FOLDER),
            autostart_dir: config_home.join("autostart"),
            state_dir: state_home.join(APP_FOLDER),
            user_data_dir: data_home.join(APP_FOLDER),
            cache_dir: cache_home.join(APP_FOLDER),
        }
    }

    /// The bundled data (`/usr/share/family-connect` once installed).
    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "the shell self-test reports it")
    )]
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// The string tables `services::catalogue` loads: `<data>/i18n/<lang>.json` + `plurals.json`.
    pub fn i18n_dir(&self) -> PathBuf {
        self.data_dir.join("i18n")
    }

    /// The rich renderer's page and engines (`<data>/rich/`, P2).
    pub fn rich_dir(&self) -> PathBuf {
        self.data_dir.join("rich")
    }

    /// `$XDG_CONFIG_HOME/family-connect/settings.conf`.
    pub fn settings_file(&self) -> PathBuf {
        self.config_dir.join(SETTINGS_FILE_NAME)
    }

    /// `$XDG_CONFIG_HOME/autostart` — where P5's autostart entry goes (§17).
    pub fn autostart_dir(&self) -> &Path {
        &self.autostart_dir
    }

    /// `$XDG_STATE_HOME/family-connect`.
    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    /// `$XDG_STATE_HOME/family-connect/cache.db` — the cache, the outbox in it.
    pub fn cache_db(&self) -> PathBuf {
        self.state_dir.join(CACHE_FILE_NAME)
    }

    /// `$XDG_STATE_HOME/family-connect/media` — `FolderMediaStore`'s staged sends.
    pub fn staging_dir(&self) -> PathBuf {
        self.state_dir.join("media")
    }

    /// `$XDG_STATE_HOME/family-connect/parked` — `ParkedRecordings` (P3).
    pub fn parked_dir(&self) -> PathBuf {
        self.state_dir.join("parked")
    }

    /// `$XDG_DATA_HOME/family-connect/inbox` — files handed over on the command line (§8.6, P2).
    pub fn inbox_dir(&self) -> PathBuf {
        self.user_data_dir.join("inbox")
    }

    /// `$XDG_CACHE_HOME/family-connect`.
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// `$XDG_CACHE_HOME/family-connect/blobs` — downloaded attachments (`AttachmentCache`).
    pub fn blobs_dir(&self) -> PathBuf {
        self.cache_dir.join("blobs")
    }

    /// `$XDG_CACHE_HOME/family-connect/avatars` — profile pictures (`AvatarCache`).
    pub fn avatars_dir(&self) -> PathBuf {
        self.cache_dir.join("avatars")
    }

    /// `$XDG_CACHE_HOME/family-connect/rich` — formulas and diagrams drawn (P2).
    pub fn rich_cache_dir(&self) -> PathBuf {
        self.cache_dir.join("rich")
    }

    /// `$XDG_CACHE_HOME/family-connect/tiles` — map tiles (P2).
    pub fn tiles_dir(&self) -> PathBuf {
        self.cache_dir.join("tiles")
    }

    /// `$XDG_CACHE_HOME/family-connect/tmp` — transcode and recorder temporaries, swept at launch.
    pub fn tmp_dir(&self) -> PathBuf {
        self.cache_dir.join("tmp")
    }

    /// The folders that hold one server's downloads: wiped when the app moves to another server
    /// (attachment ids are a server's own — Windows' `AppServices.UseAsync`).
    pub fn server_bound_dirs(&self) -> [PathBuf; 2] {
        [self.blobs_dir(), self.avatars_dir()]
    }
}

impl Default for Paths {
    fn default() -> Self {
        Paths::new()
    }
}

/// Whether `dir` is a data folder: it holds the string tables.
fn holds_data(dir: &Path) -> bool {
    dir.join("i18n").join("plurals.json").is_file()
}

/// `$FC_DATA_DIR` when set and not empty; else the data beside the executable `exe`
/// ([`beside_executable`]); else the installed folder when it exists; else, in a debug build, the
/// source tree's `linux/data`; else the installed folder anyway, so a broken install reports the
/// missing files by their real path.
fn data_dir(overridden: Option<PathBuf>, exe: Option<&Path>) -> PathBuf {
    if let Some(dir) = overridden.filter(|dir| !dir.as_os_str().is_empty()) {
        return dir;
    }
    if let Some(dir) = exe.and_then(beside_executable) {
        return dir;
    }
    let installed = Path::new(INSTALLED_DATA_DIR);
    if installed.is_dir() {
        return installed.to_path_buf();
    }
    // `#[cfg]`, not `cfg!`: a release binary must not carry the build machine's path.
    #[cfg(debug_assertions)]
    {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("data")
    }
    #[cfg(not(debug_assertions))]
    {
        installed.to_path_buf()
    }
}

/// The data an executable at `exe` carries with it: `<prefix>/share/family-connect` for
/// `<prefix>/bin/family-connect`, else `<linux>/data` for `<linux>/target/<profile>/family-connect`;
/// each only when it holds the tables.
fn beside_executable(exe: &Path) -> Option<PathBuf> {
    let prefix = exe.parent()?.parent()?;
    let shared = prefix.join("share").join(APP_FOLDER);
    if holds_data(&shared) {
        return Some(shared);
    }
    let checkout = prefix.parent()?.join("data");
    holds_data(&checkout).then_some(checkout)
}

/// `<prefix>/share/icons` for `<prefix>/bin/family-connect`, when it holds a hicolor folder — the
/// portable archive's icon (docs/packaging.md §12).
pub fn icons_beside(exe: &Path) -> Option<PathBuf> {
    let icons = exe.parent()?.parent()?.join("share").join("icons");
    icons.join("hicolor").is_dir().then_some(icons)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder under the temp directory, removed on drop.
    struct TempFolder(PathBuf);

    impl TempFolder {
        fn new(tag: &str) -> TempFolder {
            let path = std::env::temp_dir().join(format!("fc-paths-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            TempFolder(path)
        }

        /// `i18n/plurals.json` under `dir`.
        fn with_data(&self, dir: &str) -> PathBuf {
            let dir = self.0.join(dir);
            std::fs::create_dir_all(dir.join("i18n")).unwrap();
            std::fs::write(dir.join("i18n/plurals.json"), "{}").unwrap();
            dir
        }
    }

    impl Drop for TempFolder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn fc_data_dir_wins_and_an_empty_one_does_not() {
        let temp = TempFolder::new("override");
        let exe = temp.with_data("opt/share/family-connect");
        let exe = exe.join("../../bin/family-connect");
        assert_eq!(
            data_dir(Some(PathBuf::from("/srv/fc")), Some(&exe)),
            PathBuf::from("/srv/fc")
        );
        assert_eq!(data_dir(Some(PathBuf::new()), None), data_dir(None, None));
    }

    #[test]
    fn a_prefix_install_finds_its_share_folder() {
        let temp = TempFolder::new("prefix");
        let shared = temp.with_data("opt/share/family-connect");
        assert_eq!(
            beside_executable(&temp.0.join("opt/bin/family-connect")),
            Some(shared)
        );
    }

    #[test]
    fn a_build_in_the_checkout_finds_linux_data() {
        let temp = TempFolder::new("checkout");
        let data = temp.with_data("linux/data");
        assert_eq!(
            beside_executable(&temp.0.join("linux/target/release/family-connect")),
            Some(data.clone())
        );
        assert_eq!(
            data_dir(
                None,
                Some(&temp.0.join("linux/target/debug/family-connect"))
            ),
            data
        );
    }

    #[test]
    fn a_folder_without_the_tables_is_not_data() {
        let temp = TempFolder::new("partial");
        std::fs::create_dir_all(temp.0.join("p/share/family-connect/i18n")).unwrap();
        assert_eq!(
            beside_executable(&temp.0.join("p/bin/family-connect")),
            None
        );
        assert_eq!(beside_executable(Path::new("/family-connect")), None);
        assert_eq!(beside_executable(Path::new("family-connect")), None);
    }

    #[test]
    fn a_debug_build_without_an_install_finds_the_source_tree() {
        if Path::new(INSTALLED_DATA_DIR).is_dir() {
            return; // an installed copy is preferred; nothing to check here
        }
        let dir = data_dir(None, None);
        assert!(holds_data(&dir), "{}", dir.display());
    }

    #[test]
    fn every_folder_sits_where_the_design_puts_it() {
        let paths = Paths::with_roots(
            PathBuf::from("/usr/share/family-connect"),
            Path::new("/h/.config"),
            Path::new("/h/.local/state"),
            Path::new("/h/.local/share"),
            Path::new("/h/.cache"),
        );
        let p = PathBuf::from;
        assert_eq!(paths.i18n_dir(), p("/usr/share/family-connect/i18n"));
        assert_eq!(paths.rich_dir(), p("/usr/share/family-connect/rich"));
        assert_eq!(
            paths.settings_file(),
            p("/h/.config/family-connect/settings.conf")
        );
        assert_eq!(paths.autostart_dir(), Path::new("/h/.config/autostart"));
        assert_eq!(
            paths.cache_db(),
            p("/h/.local/state/family-connect/cache.db")
        );
        assert_eq!(
            paths.staging_dir(),
            p("/h/.local/state/family-connect/media")
        );
        assert_eq!(
            paths.parked_dir(),
            p("/h/.local/state/family-connect/parked")
        );
        assert_eq!(paths.inbox_dir(), p("/h/.local/share/family-connect/inbox"));
        assert_eq!(paths.blobs_dir(), p("/h/.cache/family-connect/blobs"));
        assert_eq!(paths.avatars_dir(), p("/h/.cache/family-connect/avatars"));
        assert_eq!(paths.rich_cache_dir(), p("/h/.cache/family-connect/rich"));
        assert_eq!(paths.tiles_dir(), p("/h/.cache/family-connect/tiles"));
        assert_eq!(paths.tmp_dir(), p("/h/.cache/family-connect/tmp"));
        // The outbox is never under the cache folder: tools purge caches.
        assert!(!paths.cache_db().starts_with(paths.cache_dir()));
        assert!(!paths.staging_dir().starts_with(paths.cache_dir()));
    }

    #[test]
    fn icons_are_looked_for_beside_the_executable_only_when_a_hicolor_folder_is_there() {
        let temp = TempFolder::new("icons");
        let exe = temp.0.join("bin").join("family-connect");
        assert_eq!(icons_beside(&exe), None);
        std::fs::create_dir_all(temp.0.join("share/icons/hicolor")).unwrap();
        assert_eq!(icons_beside(&exe), Some(temp.0.join("share").join("icons")));
    }
}
