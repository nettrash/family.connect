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
    /// Returns the byte count. The caller's route also carries a
    /// DefaultBodyLimit, but that one rejects with a bare 413 carrying no
    /// protocol error body — the count here is what produces a refusal a
    /// client can explain to its user.
    pub async fn write_stream<S, E>(
        &self,
        path: &Path,
        mut stream: S,
        max_bytes: usize,
    ) -> Result<u64, ApiError>
    where
        S: futures_util::Stream<Item = Result<bytes::Bytes, E>> + Unpin,
        E: std::fmt::Display,
    {
        use futures_util::StreamExt;

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
        Ok(written)
    }

    /// Remove an attachment's bytes and preview. Missing files are not an
    /// error: the row is the record, the file is only its content.
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
