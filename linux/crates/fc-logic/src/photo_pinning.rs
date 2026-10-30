//! A picture onto the wall: uploaded as a photo, its preview after it, then a PHOTO NOTE naming it
//! (docs/protocol.md, "Board": a photo note is a picture this caller uploaded and nothing has
//! claimed, named as `attachment_id`) — the web client's `pin_photo` (`PhotoPinning.cs`).
//!
//! ONE AT A TIME, AND SAID SO. A second photo quietly dropped is a photo its sender thinks is on the
//! wall.
//!
//! THE PREVIEW IS BEST EFFORT, BUT NOT BEST EFFORT ONCE: the sticker draws it, so a failed one is
//! tried twice more beside the pin — when the failure was the network's, since the same bytes
//! refused once would be refused again. Those two tries outlive the pin: they are SPAWNED (the
//! C#'s `_ = RetryPreviewAsync(…)`) through the [`Spawn`] the shell gives — the main context's
//! `spawn_local`.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use fc_core::protocol::{ApiClient, ApiError, NoteRequest, UploadFacts};
use fc_core::rt::Clock;
use fc_core::store::{BoardStore, SeqRoute, StoreResult};
use fc_core::text::string_catalog::StringCatalog;
use fc_text::board::{Font, Kind, Size, clamp_corner, fraction_of, is_compact};

use crate::calls::Spawn;
use crate::media_outbox::StagedMedia;
use crate::note_draft::note_sheet_text;

/// How pinning a photo ended (`PinOutcome`).
#[derive(Debug, Clone, PartialEq)]
pub struct PinOutcome {
    pub pinned: bool,
    /// A photo is already on its way: one at a time, and said so.
    pub busy: bool,
    pub error: Option<ApiError>,
}

impl PinOutcome {
    /// `PinOutcome.Done`.
    pub fn done() -> Self {
        PinOutcome {
            pinned: true,
            busy: false,
            error: None,
        }
    }

    /// `PinOutcome.StillPinning`.
    pub fn still_pinning() -> Self {
        PinOutcome {
            pinned: false,
            busy: true,
            error: None,
        }
    }

    /// `PinOutcome.Refused(error)`.
    pub fn refused(error: ApiError) -> Self {
        PinOutcome {
            pinned: false,
            busy: false,
            error: Some(error),
        }
    }
}

/// The pauses before the preview is tried again.
pub const PREVIEW_RETRIES: [Duration; 2] = [Duration::from_secs(2), Duration::from_secs(8)];

/// A picture onto the wall (`PhotoPinning`).
pub struct PhotoPinning {
    api: Rc<ApiClient>,
    board: Rc<BoardStore>,
    clock: Rc<dyn Clock>,
    spawn: Spawn,
    pinning: Cell<bool>,
}

impl std::fmt::Debug for PhotoPinning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PhotoPinning")
            .field("pinning", &self.pinning.get())
            .finish_non_exhaustive()
    }
}

/// The C#'s `finally`: the flag goes down however the pin ends — answered, refused, or dropped
/// part-way (dropping is cancelling).
struct Pinning<'a>(&'a Cell<bool>);

impl Drop for Pinning<'_> {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

impl PhotoPinning {
    /// `new PhotoPinning(api, board, wait)`: `clock` is the C#'s `wait` (the pauses before a preview
    /// is tried again), `spawn` how those tries start beside the pin.
    pub fn new(
        api: Rc<ApiClient>,
        board: Rc<BoardStore>,
        clock: Rc<dyn Clock>,
        spawn: Spawn,
    ) -> Self {
        PhotoPinning {
            api,
            board,
            clock,
            spawn,
            pinning: Cell::new(false),
        }
    }

    /// Whether a photo is on its way to the wall now.
    pub fn pinning(&self) -> bool {
        self.pinning.get()
    }

    /// Upload, preview, pin. `Err` is the cache refusing to record the pinned note (the C#'s
    /// exception); a refusal on the way is `Ok(PinOutcome { error: Some(…) })`.
    pub async fn pin(
        &self,
        photo: &StagedMedia,
        at: (f64, f64),
        color: &str,
    ) -> StoreResult<PinOutcome> {
        if self.pinning.replace(true) {
            return Ok(PinOutcome::still_pinning());
        }
        let _pinning = Pinning(&self.pinning);
        let facts = UploadFacts {
            width: photo.width,
            height: photo.height,
            ..UploadFacts::default()
        };
        let uploaded = self
            .api
            .upload("photo", &photo.mime, photo.bytes.clone(), &facts)
            .await;
        let attachment_id = match uploaded.into_result() {
            Ok(uploaded) => uploaded.attachment.id,
            Err(error) => return Ok(PinOutcome::refused(error)),
        };
        if let Some(preview) = photo.preview.as_ref().filter(|preview| !preview.is_empty()) {
            let sent = self
                .api
                .upload_preview(attachment_id, preview.clone())
                .await;
            if sent.error().is_some_and(ApiError::transient) {
                self.retry_preview(attachment_id, preview.clone());
            }
        }
        let note = NoteRequest {
            size: Some(Size::Medium.name().to_string()),
            font: Some(Font::Plain.name().to_string()),
            kind: Some(Kind::Photo.name().to_string()),
            attachment_id: Some(attachment_id),
            ..NoteRequest::new("", color, at.0, at.1)
        };
        match self.api.create_note(&note).await.into_result() {
            Err(error) => Ok(PinOutcome::refused(error)),
            Ok(created) => {
                // As EVIDENCE, exactly as a note written from the sheet: one note's news, no
                // cursor moved.
                self.board.apply(&created.note, SeqRoute::Evidence)?;
                Ok(PinOutcome::done())
            }
        }
    }

    /// Where a picked photo lands: near the upper middle, jittered so two do not stack exactly.
    /// `random` answers a number in `[0, 1)`.
    pub fn scattered(mut random: impl FnMut() -> f64) -> (f64, f64) {
        let x = 0.35 + ((random() - 0.5) * 0.1);
        let y = 0.30 + ((random() - 0.5) * 0.1);
        (x, y)
    }

    /// Where a DROPPED photo lands: a medium card centred under the pointer, held inside the wall.
    pub fn dropped_at(pointer: (f64, f64), wall: (f64, f64)) -> (f64, f64) {
        let card = Size::Medium.frame(is_compact(wall.0));
        let corner = clamp_corner(
            pointer.0 - (card.0 / 2.0),
            pointer.1 - (card.1 / 2.0),
            card,
            wall,
        );
        fraction_of(corner, wall)
    }

    pub fn still_pinning_text(say: &dyn StringCatalog) -> String {
        say.get("A photo is still being pinned. Add the next one when it is on the board.")
            .to_string()
    }

    pub fn one_at_a_time_text(say: &dyn StringCatalog) -> String {
        say.get("The board pins one photo at a time — the first is on its way.")
            .to_string()
    }

    pub fn photos_only_text(say: &dyn StringCatalog) -> String {
        say.get("The board pins photos only.").to_string()
    }

    /// A refused step of pinning: that it did not go, and why.
    pub fn failure_text(error: &ApiError, say: &dyn StringCatalog) -> String {
        format!(
            "{} {}",
            say.get("Couldn't pin that photo."),
            note_sheet_text::failure(error, say)
        )
    }

    /// The preview, twice more, after [`PREVIEW_RETRIES`]' pauses — stopping at the first that
    /// lands.
    fn retry_preview(&self, attachment_id: i64, preview: Vec<u8>) {
        let api = Rc::clone(&self.api);
        let clock = Rc::clone(&self.clock);
        (self.spawn)(Box::pin(async move {
            for pause in PREVIEW_RETRIES {
                clock.sleep(pause).await;
                if api
                    .upload_preview(attachment_id, preview.clone())
                    .await
                    .ok()
                {
                    return;
                }
            }
        }));
    }
}
