//! Profile pictures: what one must be before it is sent, what may be sent at all, and the cache
//! that keeps them per VERSION (`Avatars.cs`).

use std::rc::Rc;

use fc_core::protocol::{ApiClient, ApiError};

use crate::attachment_cache::BlobStore;

/// What a profile picture must be before it is sent, and what may be sent at all (`AvatarRules`).
///
/// The server stores what it is given after validating the type and the size and NEVER
/// transcodes, so the downscale is the client's job: a square JPEG whose longest edge is 512 px is
/// what the phones and the Mac send, and a client that sent a 12-megapixel original would hand
/// every other client a download nobody needs (docs/protocol.md, "Profile pictures").
pub mod avatar_rules {
    /// The longest edge, after the downscale every client does before sending.
    pub const MAX_EDGE: i32 = 512;

    /// The two types the server takes. Nothing else is offered.
    pub const MIMES: [&str; 2] = ["image/jpeg", "image/png"];

    /// What this client sends when it has re-encoded a picture itself.
    pub const SEND_AS: &str = "image/jpeg";

    /// Whether a picture of this size has to be shrunk before it is sent.
    pub fn needs_downscale(width: i32, height: i32) -> bool {
        width > MAX_EDGE || height > MAX_EDGE
    }

    /// The size to draw it at: the longest edge down to [`MAX_EDGE`], never up — a picture already
    /// smaller is sent as it is rather than blown up to look worse. Rounded half to EVEN, as the
    /// C#'s `Math.Round` rounds (measured on .NET 10: 1536 × 1024 → 512 × 341).
    pub fn fit(width: i32, height: i32) -> (i32, i32) {
        if width <= 0 || height <= 0 {
            return (0, 0);
        }
        if !needs_downscale(width, height) {
            return (width, height);
        }
        let scale = f64::from(MAX_EDGE) / f64::from(width.max(height));
        let side = |edge: i32| ((f64::from(edge) * scale).round_ties_even() as i32).max(1);
        (side(width), side(height))
    }

    /// Whether this is a type the server will take. The C# compares `OrdinalIgnoreCase`, which for
    /// these two types is exactly an ASCII comparison: no character outside ASCII folds onto one of
    /// their letters (measured on .NET 10 over the whole BMP) — so a Turkish dotless ı is not an
    /// `i`, there or here, and no reader's alphabet is consulted.
    pub fn is_allowed(mime: &str) -> bool {
        MIMES
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(mime))
    }
}

/// Profile pictures, downloaded once per VERSION and kept (`AvatarCache`).
///
/// THE VERSION IS PART OF THE KEY. A picture cached under the user's id alone never changes when
/// somebody changes theirs: `avatar_version` is the only signal there is — no frame carries the
/// picture, only the number — so a cache that ignores it shows a face the family replaced weeks
/// ago. It is the same mistake, in a different place, as caching a board note's backdrop by the
/// NOTE instead of by the attachment (issue #70).
///
/// "NO PICTURE" IS AN ANSWER AND IT IS CACHED. A user with none — and anybody outside the family —
/// is a 404, which is stable for that version: asking again on every redraw would be a request per
/// row per frame. A TRANSIENT failure caches nothing.
pub struct AvatarCache {
    api: Rc<ApiClient>,
    blobs: Rc<dyn BlobStore>,
}

impl std::fmt::Debug for AvatarCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AvatarCache").finish_non_exhaustive()
    }
}

impl AvatarCache {
    pub fn new(api: Rc<ApiClient>, blobs: Rc<dyn BlobStore>) -> Self {
        AvatarCache { api, blobs }
    }

    pub fn key_for(user_id: i64, version: i32) -> String {
        format!("avatar-{user_id}-{version}")
    }

    /// The bytes of somebody's picture, or `Ok(None)` when there is none to draw — which is a real
    /// answer and not a failure: the caller draws initials. `Err` only for a transient failure,
    /// which the next look asks again about.
    pub async fn bytes(&self, user_id: i64, version: i32) -> Result<Option<Vec<u8>>, ApiError> {
        if version <= 0 {
            // 0 means no picture at all, and it says so without a request.
            return Ok(None);
        }
        let key = AvatarCache::key_for(user_id, version);
        if let Some(held) = self.blobs.read(&key) {
            // What a missing picture is written as: present, and empty.
            return Ok((!held.is_empty()).then_some(held));
        }
        match self.api.avatar(user_id).await.into_result() {
            Ok(bytes) => {
                self.blobs.write(&key, &bytes);
                Ok(Some(bytes))
            }
            // Nothing is written: the next look asks again rather than drawing initials for ever.
            Err(error) if error.transient() => Err(error),
            Err(_) => {
                // A refusal — no picture, or not this caller's business to see. Stable for this
                // version.
                self.blobs.write(&key, &[]);
                Ok(None)
            }
        }
    }
}
