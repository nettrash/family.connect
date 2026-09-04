//! Attachment bytes on disk.
//!
//! Streaming both ways, deliberately: a 100 MB video must never be held in
//! memory in one piece. The upload writes chunks to a temp file and renames
//! it into place only once the whole body has arrived, so a connection that
//! drops half-way leaves a `.part` file the sweeper removes rather than a
//! truncated attachment somebody can open.
//!
//! Files are sharded two levels deep by the id's low digits. One flat
//! directory with tens of thousands of entries is slow to list and unkind
//! to every backup tool that has to walk it.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::Context;
use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::error::ApiError;

/// What a completed write produced.
#[derive(Debug, Clone)]
pub struct Written {
    pub bytes: u64,
    /// Hex SHA-256 of the stored bytes — the dedup key.
    pub sha256: String,
}

/// Where one attachment's bytes and its preview live.
#[derive(Debug, Clone)]
pub struct Storage {
    root: PathBuf,
}

impl Storage {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Create the root eagerly at boot, so a misconfigured directory is a
    /// startup failure rather than a surprise on the family's first photo.
    pub async fn ensure_root(&self) -> anyhow::Result<()> {
        fs::create_dir_all(&self.root)
            .await
            .with_context(|| format!("creating attachments dir {}", self.root.display()))
    }

    /// Bytes still free on the filesystem the attachments live on, as the
    /// unprivileged service user sees them (`f_bavail`, not `f_bfree` —
    /// the difference is the root-only reserve, which this process cannot
    /// use and must not count).
    ///
    /// `None` when the filesystem cannot be interrogated at all, which is
    /// treated by callers as "do not refuse": a server that stopped
    /// accepting photos because a syscall failed would be a worse failure
    /// than the one this guards against.
    pub fn free_bytes(&self) -> Option<u64> {
        use std::os::unix::ffi::OsStrExt;

        let path = std::ffi::CString::new(self.root.as_os_str().as_bytes()).ok()?;
        // SAFETY: `path` is a valid NUL-terminated C string that outlives
        // the call, and `stat` is a correctly-sized, correctly-aligned
        // output buffer the callee fully initializes on success.
        let stats = unsafe {
            let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
            if libc::statvfs(path.as_ptr(), stat.as_mut_ptr()) != 0 {
                return None;
            }
            stat.assume_init()
        };
        // f_frsize is the fragment size and the unit f_bavail counts in;
        // f_bsize is the preferred I/O block size and is NOT the same
        // number on every filesystem.
        (stats.f_bavail as u64).checked_mul(stats.f_frsize as u64)
    }

    /// Would accepting `incoming` more bytes leave less than `floor` free?
    ///
    /// Separated from the syscall so the arithmetic — which is where an
    /// off-by-a-factor-of-1024 would live — can be tested without a full
    /// disk. A `floor` of 0 disables the check, and so does a filesystem
    /// that would not answer.
    pub fn would_breach_floor(free: Option<u64>, incoming: u64, floor: u64) -> bool {
        if floor == 0 {
            return false;
        }
        let Some(free) = free else { return false };
        free.saturating_sub(incoming) < floor
    }

    /// `<root>/ab/cd/<key>` — two levels of shard from the key's tail.
    fn path_for(&self, key: &str, suffix: &str) -> PathBuf {
        let tail: String = key.chars().rev().take(4).collect();
        let (a, b) = tail.split_at(2);
        self.root.join(a).join(b).join(format!("{key}{suffix}"))
    }

    pub fn blob_path(&self, key: &str) -> PathBuf {
        self.path_for(key, "")
    }

    pub fn preview_path(&self, key: &str) -> PathBuf {
        self.path_for(key, ".preview")
    }

    /// Write a stream to `path`, refusing past `max_bytes`.
    ///
    /// Returns the byte count and the SHA-256 of what was written. The
    /// digest is computed as the bytes go past — they are in hand anyway,
    /// and re-reading a 100 MB file to hash it afterwards would double the
    /// I/O of every upload. It is what lets a family store one copy of a
    /// photo it sends itself three times (migration 0011).
    ///
    /// The caller's route also carries a DefaultBodyLimit, but that one
    /// rejects with a bare 413 carrying no protocol error body — the count
    /// here is what produces a refusal a client can explain to its user.
    pub async fn write_stream<S, E>(
        &self,
        path: &Path,
        mut stream: S,
        max_bytes: usize,
    ) -> Result<Written, ApiError>
    where
        S: futures_util::Stream<Item = Result<bytes::Bytes, E>> + Unpin,
        E: std::fmt::Display,
    {
        use futures_util::StreamExt;
        use sha2::{Digest, Sha256};

        let mut hasher = Sha256::new();

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|err| ApiError::Internal(anyhow::anyhow!("creating {parent:?}: {err}")))?;
        }
        let temp = path.with_extension("part");
        let mut file = fs::File::create(&temp)
            .await
            .map_err(|err| ApiError::Internal(anyhow::anyhow!("creating {temp:?}: {err}")))?;

        let mut written: u64 = 0;
        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(err) => {
                    // A dropped upload leaves nothing behind.
                    let _ = fs::remove_file(&temp).await;
                    return Err(ApiError::validation(format!("upload interrupted: {err}")));
                }
            };
            written += chunk.len() as u64;
            hasher.update(&chunk);
            if written > max_bytes as u64 {
                let _ = fs::remove_file(&temp).await;
                return Err(ApiError::payload_too_large(
                    crate::error::codes::ATTACHMENT_TOO_LARGE,
                    format!("attachment must be at most {max_bytes} bytes"),
                ));
            }
            if let Err(err) = file.write_all(&chunk).await {
                let _ = fs::remove_file(&temp).await;
                return Err(ApiError::Internal(anyhow::anyhow!(
                    "writing {temp:?}: {err}"
                )));
            }
        }
        file.flush()
            .await
            .map_err(|err| ApiError::Internal(anyhow::anyhow!("flushing {temp:?}: {err}")))?;
        drop(file);

        // Rename only now: until this point nothing can open a partial file.
        fs::rename(&temp, path)
            .await
            .map_err(|err| ApiError::Internal(anyhow::anyhow!("renaming into {path:?}: {err}")))?;
        Ok(Written {
            bytes: written,
            sha256: hex::encode(hasher.finalize()),
        })
    }

    /// Remove one file, without touching its preview. Used when an upload
    /// turns out to duplicate bytes the family already holds: the row keeps
    /// the EXISTING key, and the copy that was just written is dropped.
    pub async fn discard(&self, path: &Path) {
        if let Err(err) = fs::remove_file(path).await
            && err.kind() != ErrorKind::NotFound
        {
            tracing::warn!(?path, %err, "discarding a duplicate upload failed");
        }
    }

    /// Remove an attachment's bytes and preview. Missing files are not an
    /// error: the row is the record, the file is only its content.
    ///
    /// CALLERS MUST CHECK FIRST that no other attachment row still names
    /// this key — since 0011 a file can be shared within a family, and
    /// removing it out from under another row would break a message that
    /// looks perfectly fine in the database.
    pub async fn remove(&self, key: &str) {
        for path in [self.blob_path(key), self.preview_path(key)] {
            if let Err(err) = fs::remove_file(&path).await
                && err.kind() != ErrorKind::NotFound
            {
                tracing::warn!(?path, %err, "removing attachment file failed");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The free-space floor is arithmetic on two large numbers, which is
    /// exactly where a factor-of-1024 mistake hides — and it would hide
    /// silently, because being too generous only shows up as a full disk
    /// and being too strict only shows up as refused photos.
    #[test]
    fn the_floor_refuses_only_what_would_breach_it() {
        let gib = 1024 * 1024 * 1024;
        let floor = 2 * gib;

        // Plenty of room: a 100 MB video lands with room to spare.
        assert!(!Storage::would_breach_floor(
            Some(50 * gib),
            100 * 1024 * 1024,
            floor
        ));
        // The upload itself is what would breach it — the free figure alone
        // is still above the floor, so a check that ignored the incoming
        // size would wrongly accept this.
        assert!(Storage::would_breach_floor(Some(2 * gib + 10), 1024, floor));
        // Already under: refuse even a byte.
        assert!(Storage::would_breach_floor(Some(gib), 0, floor));
        // Exactly at the floor after the write is NOT a breach; below is.
        assert!(!Storage::would_breach_floor(Some(floor + 100), 100, floor));
        assert!(Storage::would_breach_floor(Some(floor + 100), 101, floor));
    }

    #[test]
    fn a_zero_floor_disables_the_check() {
        // The documented off switch. Note it holds even when the disk is
        // reporting no free space at all.
        assert!(!Storage::would_breach_floor(Some(0), u64::MAX, 0));
    }

    #[test]
    fn an_unreadable_filesystem_does_not_refuse_uploads() {
        // A server that stopped accepting photos because statvfs failed
        // would be a worse failure than the one this guards against.
        assert!(!Storage::would_breach_floor(None, u64::MAX, 1));
    }

    #[test]
    fn a_huge_upload_cannot_wrap_the_subtraction() {
        // saturating_sub, not `-`: an underflow here would compute a
        // gigantic "free after" and accept an upload that fills the disk.
        assert!(Storage::would_breach_floor(Some(10), u64::MAX, 1));
    }

    #[test]
    fn free_bytes_reads_a_real_filesystem() {
        let storage = Storage::new(std::env::temp_dir());
        let free = storage.free_bytes().expect("temp dir is on a filesystem");
        // Not a tautology: the bug worth catching is multiplying by the
        // wrong block size, which lands orders of magnitude out. A dev or
        // CI machine has more than a megabyte and less than an exabyte.
        assert!(free > 1024 * 1024, "suspiciously small: {free}");
        assert!(free < (1u64 << 60), "suspiciously large: {free}");
    }

    #[test]
    fn a_missing_directory_reports_nothing_rather_than_zero() {
        // Zero would mean "full" and refuse every upload. None means "do
        // not know", which callers treat as "do not refuse".
        let storage = Storage::new(PathBuf::from("/nonexistent-family-connect-test-path"));
        assert_eq!(storage.free_bytes(), None);
    }
}
