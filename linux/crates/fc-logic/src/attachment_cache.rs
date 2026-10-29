//! One attachment's bytes, downloaded once and kept — with the one rule about previews that three
//! of this app's clients have each had to learn (`AttachmentCache.cs`).
//!
//! A PREVIEW IS ONLY ASKED FOR WHEN THE ATTACHMENT SAYS IT HAS ONE. The server generates no
//! preview for a picture the ASSISTANT drew, and none at all for a file, a piece of audio or a
//! location — so `has_preview` is a fact, not a hint. Asking anyway answers 404, and a client that
//! treats that as "no picture" draws an empty frame for ever: it is exactly how the Android board's
//! backdrops failed (issue #71's follow-up), and the fix belongs HERE rather than at every call
//! site, where one of them will always forget.
//!
//! A FAILED DOWNLOAD IS NOT CACHED. Nothing is written unless bytes arrived, so the next look asks
//! again — a transient failure must not become a permanently blank picture.

use std::rc::Rc;

use fc_core::protocol::{ApiClient, ApiError, AttachmentDto};

/// Where downloaded bytes are kept (`IBlobStore`). A seam because WHERE is the shell's question —
/// the app's own cache folder, which the system may clear — and because a test has no business
/// writing files. Shared by [`AttachmentCache`] and [`crate::avatars::AvatarCache`].
///
/// Neither call fails: a store that cannot read answers `None` (the C#'s `FileBlobStore` reads an
/// `IOException` as "not held"), and a write that cannot land is the store's to log and forget —
/// the next look then asks the server again, which is what a cache that was cleared does anyway.
/// (The C#'s write let a disk-full exception out to the view.)
pub trait BlobStore {
    /// The bytes under this key, or `None` when they are not held.
    fn read(&self, key: &str) -> Option<Vec<u8>>;

    /// Keep these bytes under this key. A write lands whole or not at all.
    fn write(&self, key: &str, bytes: &[u8]);
}

/// One attachment's bytes, downloaded once and kept.
pub struct AttachmentCache {
    api: Rc<ApiClient>,
    blobs: Rc<dyn BlobStore>,
}

impl std::fmt::Debug for AttachmentCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AttachmentCache").finish_non_exhaustive()
    }
}

impl AttachmentCache {
    pub fn new(api: Rc<ApiClient>, blobs: Rc<dyn BlobStore>) -> Self {
        AttachmentCache { api, blobs }
    }

    /// The key bytes are kept under: the id, and whether they are the small copy.
    pub fn key_for(attachment_id: i64, preview: bool) -> String {
        if preview {
            format!("{attachment_id}.preview")
        } else {
            attachment_id.to_string()
        }
    }

    /// The bytes for this attachment, from the cache when they are held and from the server
    /// otherwise.
    ///
    /// `preview`: whether the small copy will do. It is asked for only when the attachment HAS
    /// one; on one that does not, this reads the original instead of asking for something that is
    /// not there.
    pub async fn bytes(
        &self,
        attachment: &AttachmentDto,
        preview: bool,
    ) -> Result<Vec<u8>, ApiError> {
        // THE ONE RULE: a preview exists only where the attachment says so.
        let small = preview && attachment.has_preview;
        let key = AttachmentCache::key_for(attachment.id, small);
        if let Some(held) = self.blobs.read(&key) {
            return Ok(held);
        }
        // Not cached on a failure: the next look asks again rather than drawing a blank for ever.
        let bytes = self
            .api
            .download(attachment.id, small)
            .await
            .into_result()?;
        self.blobs.write(&key, &bytes);
        Ok(bytes)
    }

    /// Keep bytes this device ALREADY HAS under an attachment's id — a sticker it just uploaded,
    /// which it would otherwise download straight back to draw. The original, never a preview. An
    /// id never names different bytes, so this cannot go stale.
    pub fn remember(&self, attachment: &AttachmentDto, bytes: &[u8]) {
        self.blobs
            .write(&AttachmentCache::key_for(attachment.id, false), bytes);
    }

    /// Keep a PREVIEW this device made and just sent — a video message's square poster — under the
    /// attachment's id, so the circle it becomes is drawn at once rather than downloaded back
    /// (S5.6). Kept under the preview's own key, which a later read asks for once the message says
    /// `has_preview`; an id never names different bytes.
    pub fn remember_preview(&self, attachment: &AttachmentDto, jpeg: &[u8]) {
        self.blobs
            .write(&AttachmentCache::key_for(attachment.id, true), jpeg);
    }

    /// Whether these bytes are already here — what a view asks before it decides to show a
    /// spinner.
    pub fn holds(&self, attachment: &AttachmentDto, preview: bool) -> bool {
        self.blobs
            .read(&AttachmentCache::key_for(
                attachment.id,
                preview && attachment.has_preview,
            ))
            .is_some()
    }
}
