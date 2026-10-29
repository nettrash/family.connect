//! The app's own files (shell-design §2.3, §2.4): fc-logic's [`BlobStore`] on disk
//! ([`FileBlobStore`], Windows' `FileBlobStore`), and the folder chores of start-up and of moving
//! to another server. The staged sends are fc-logic's own `FolderMediaStore` (it IS the file-backed
//! `MediaStore`), under `Paths::staging_dir`.
//!
//! **A write lands whole or not at all**: bytes go to a uniquely named part file in the same folder
//! and are renamed into place, so a crash or two downloads of one picture at once never leave a
//! truncated file that reads as a picture. Folders the app creates for user data are `0700`.

use std::cell::Cell;
use std::io;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};

use fc_logic::attachment_cache::BlobStore;

/// Downloaded bytes, one file per key (`AttachmentCache`'s `123`, `123.preview`;
/// `AvatarCache`'s `avatar-7-3`).
///
/// A key that is not a plain file name (anything but ASCII letters, digits, `.`, `-` and `_`, or
/// one starting with `.`) is never written and never found: a key is built by fc-logic from ids, so
/// such a key is a bug, and a bug must not reach outside the folder.
#[derive(Debug)]
pub struct FileBlobStore {
    folder: PathBuf,
    /// Part files written by this process, for unique names.
    written: Cell<u64>,
}

impl FileBlobStore {
    pub fn new(folder: impl Into<PathBuf>) -> FileBlobStore {
        FileBlobStore {
            folder: folder.into(),
            written: Cell::new(0),
        }
    }

    #[cfg(test)]
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    fn path(&self, key: &str) -> Option<PathBuf> {
        is_plain_name(key).then(|| self.folder.join(key))
    }

    fn try_write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        private_dir(&self.folder)?;
        let serial = self.written.get();
        self.written.set(serial + 1);
        let part = self.folder.join(format!(
            ".{}.{}-{serial}.part",
            path.file_name().and_then(|n| n.to_str()).unwrap_or("blob"),
            std::process::id()
        ));
        let written = std::fs::write(&part, bytes).and_then(|()| std::fs::rename(&part, path));
        if written.is_err() {
            let _ = std::fs::remove_file(&part);
        }
        written
    }
}

impl BlobStore for FileBlobStore {
    fn read(&self, key: &str) -> Option<Vec<u8>> {
        std::fs::read(self.path(key)?).ok()
    }

    fn write(&self, key: &str, bytes: &[u8]) {
        let Some(path) = self.path(key) else {
            crate::fc_warn!("blobs: a key that is not a file name was refused");
            return;
        };
        if let Err(error) = self.try_write(&path, bytes) {
            // Logged and forgotten: the next look downloads again (fc-logic's contract).
            crate::fc_warn!("blobs: a write failed ({})", error.kind());
        }
    }
}

/// Whether `name` is a plain file name of the app's own making.
fn is_plain_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

/// Creates `folder` (and its parents) `0700` when missing; tightens an existing one to `0700`.
pub fn private_dir(folder: &Path) -> io::Result<()> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(folder)?;
    std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o700))
}

/// Empties `folder` (keeps the folder itself). Missing is empty. Errors are logged by kind.
pub fn wipe_dir(folder: &Path) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let removed = if path.is_dir() && !path.is_symlink() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        if let Err(error) = removed {
            crate::fc_warn!(
                "files: a stale entry could not be removed ({})",
                error.kind()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempFolder(PathBuf);

    impl TempFolder {
        fn new(tag: &str) -> TempFolder {
            let path = std::env::temp_dir().join(format!("fc-files-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            TempFolder(path)
        }
    }

    impl Drop for TempFolder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn bytes_land_whole_under_their_key_in_a_private_folder() {
        let temp = TempFolder::new("blobs");
        let store = FileBlobStore::new(temp.0.join("blobs"));
        assert_eq!(store.read("42"), None);
        store.write("42", b"first");
        store.write("42", b"second");
        store.write("42.preview", b"small");
        store.write("avatar-7-3", b"face");
        assert_eq!(store.read("42").as_deref(), Some(b"second".as_slice()));
        assert_eq!(
            store.read("42.preview").as_deref(),
            Some(b"small".as_slice())
        );
        assert_eq!(
            store.read("avatar-7-3").as_deref(),
            Some(b"face".as_slice())
        );
        let mode = std::fs::metadata(store.folder())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700);
        // No part file is left behind.
        let names: Vec<String> = std::fs::read_dir(store.folder())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(names.iter().all(|n| !n.ends_with(".part")), "{names:?}");
    }

    #[test]
    fn a_key_that_is_not_a_plain_name_never_reaches_the_disk() {
        let temp = TempFolder::new("keys");
        let store = FileBlobStore::new(temp.0.join("blobs"));
        for key in ["", "../escape", "a/b", ".hidden", "имя", "a b"] {
            store.write(key, b"x");
            assert_eq!(store.read(key), None, "{key:?}");
        }
        assert!(!temp.0.join("escape").exists());
    }

    #[test]
    fn a_folder_that_cannot_be_written_is_survived() {
        let temp = TempFolder::new("blocked");
        std::fs::create_dir_all(&temp.0).unwrap();
        std::fs::write(temp.0.join("blobs"), "a file in the folder's place").unwrap();
        let store = FileBlobStore::new(temp.0.join("blobs"));
        store.write("1", b"x");
        assert_eq!(store.read("1"), None);
    }

    #[test]
    fn wiping_empties_a_folder_and_keeps_it() {
        let temp = TempFolder::new("wipe");
        std::fs::create_dir_all(temp.0.join("sub")).unwrap();
        std::fs::write(temp.0.join("a"), "1").unwrap();
        std::fs::write(temp.0.join("sub/b"), "2").unwrap();
        wipe_dir(&temp.0);
        assert!(temp.0.is_dir());
        assert_eq!(std::fs::read_dir(&temp.0).unwrap().count(), 0);
        wipe_dir(&temp.0.join("missing"));
    }
}
