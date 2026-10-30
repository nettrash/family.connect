//! The uploads a queued message owes: the pump that moves the bytes, one file at a time, before the
//! message may be posted at all (`MediaOutbox.cs`).
//!
//! A MEDIA SEND IS REPRESENTED BEFORE THE FIRST BYTE MOVES. The row and the files it owes are
//! written down first — that is the outbox's job — and this is what then pushes them, so an
//! interrupted send is a bubble that can be finished rather than nothing at all (docs/protocol.md,
//! "Sending on an unreliable network").
//!
//! EACH LANDING IS REMEMBERED AS IT HAPPENS. An id that landed is kept and reused within the
//! server's grace, so a retry pushes only the remainder — and a crash halfway through a four-photo
//! message costs the remaining three, not all four.
//!
//! THE ROW IS NEVER POSTED WHILE IT OWES ANYTHING. That gate is in the send pipeline; what matters
//! here is the other half of it — a row whose bytes this device can no longer find is FAILED rather
//! than retried, because an id cannot recover a picture and a backoff has nothing to wait for.
//!
//! **How the app wires it.** One pump per signed-in server, over the outbox the pipeline flushes;
//! [`MediaOutbox::pump`] is the closure `SendPipeline::with_uploads` takes, so every flush pushes
//! what is owed before it posts anything. Cancelling a push is dropping its future (the C#'s token,
//! which it checked between files): the file on its way stays owed, and so does every preview that
//! had not landed.

use std::cell::RefCell;
use std::collections::{HashSet, VecDeque};
use std::rc::Rc;

use fc_core::protocol::send_pipeline::Uploads;
use fc_core::protocol::{ApiClient, ApiError, AttachmentDto, UploadFacts, error_codes};
use fc_core::rt::{Event, LocalBoxFuture};
use fc_core::store::{OutboxRow, OutboxStore, StoreResult};

/// What is staged under one handle: the bytes, and what the server has to be told.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StagedMedia {
    pub kind: String,
    pub mime: String,
    pub bytes: Vec<u8>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub duration_ms: Option<i32>,
    pub name: Option<String>,
    /// The photo's small copy or the video's poster, as JPEG — made on this device, never by the
    /// server.
    pub preview: Option<Vec<u8>>,
    /// A place's numbers — `kind=location` only, which has no bytes and is these instead.
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub accuracy_m: Option<f64>,
    /// A voice note's shape, as the wire spells it (docs/protocol.md, "A voice note's waveform") —
    /// measured from the recording itself and sent with the upload; `None` for everything else and
    /// for a note whose sound could not be read.
    pub waveform: Option<String>,
    /// An ANIMATION (docs/protocol.md, "Animations"): an animated picture this device made into the
    /// protocol's silent looping MP4. Not on the upload — that is an ordinary `kind=video` — but on
    /// the message, which says `animation: true` when this is the one thing it carries
    /// ([`animation_send::is`]). Held in memory only: the outbox row carries the flag from the
    /// moment the message is queued.
    pub animation: bool,
}

impl StagedMedia {
    /// `new StagedMedia(kind, mime, bytes)`, the rest absent.
    pub fn new(kind: &str, mime: &str, bytes: Vec<u8>) -> Self {
        StagedMedia {
            kind: kind.to_string(),
            mime: mime.to_string(),
            bytes,
            ..StagedMedia::default()
        }
    }
}

/// Whether a send of staged media is an ANIMATION — and so says `animation: true` (`AnimationSend`).
pub mod animation_send {
    use super::StagedMedia;

    /// The server's own condition (docs/protocol.md, "What the server checks"): exactly ONE
    /// attachment, and it is an animation this device made. An animation staged beside other
    /// things goes as the ordinary video it also is — it still plays, in its viewer — because a
    /// message is one of the shapes, never two.
    pub fn is(items: &[StagedMedia]) -> bool {
        matches!(items, [only] if only.kind == "video" && only.animation)
    }
}

/// Where a send's bytes wait while it is being sent (`IMediaStore`). The seam exists because WHERE
/// that is, is the shell's question — under `$XDG_STATE_HOME`, somewhere the system will not
/// reclaim — and the logic builds and tests anywhere.
pub trait MediaStore {
    /// What is staged under this handle, or `None` when it is gone.
    fn read(&self, handle: &str) -> Option<StagedMedia>;

    /// Forget everything except these handles. Called with every handle every queued row still
    /// names, so the staging area cannot outlive the sends it is for.
    fn sweep(&self, keep: &HashSet<String>);
}

/// A preview that has not landed yet: the attachment, the JPEG, and how many tries it has left.
type OwedPreview = (i64, Vec<u8>, i32);

/// The uploads a queued message owes.
pub struct MediaOutbox {
    outbox: Rc<OutboxStore>,
    api: Rc<ApiClient>,
    media: Rc<dyn MediaStore>,
    family_for: Option<Box<dyn Fn(i64) -> Option<i64>>>,
    owed_previews: RefCell<Vec<OwedPreview>>,
    /// A row whose media cannot be sent, and the reason to show (`Refused`).
    pub refused: Event<(OutboxRow, ApiError)>,
    /// One file landed: the row it was for, what the server made of it, and the bytes that went
    /// (`Landed`). It is how a sent STICKER is drawn from the bytes this device already holds — the
    /// message it becomes names a new attachment id for the very same bytes, and fetching them back
    /// would be a download, and a blank box while it ran, for a picture that never left this
    /// machine.
    pub landed: Event<(OutboxRow, AttachmentDto, StagedMedia)>,
}

impl std::fmt::Debug for MediaOutbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaOutbox")
            .field("owed_previews", &self.owed_previews.borrow().len())
            .finish_non_exhaustive()
    }
}

impl MediaOutbox {
    /// How many times a preview is sent before this device stops trying.
    pub const PREVIEW_ATTEMPTS: i32 = 3;

    pub fn new(outbox: Rc<OutboxStore>, api: Rc<ApiClient>, media: Rc<dyn MediaStore>) -> Self {
        MediaOutbox {
            outbox,
            api,
            media,
            family_for: None,
            owed_previews: RefCell::new(Vec::new()),
            refused: Event::new(),
            landed: Event::new(),
        }
    }

    /// The family each chat is in (the C#'s `familyFor`): an upload goes for the family the CHAT
    /// is in, not whichever is shown by the time it goes (docs/protocol.md, "Several families").
    pub fn with_family_for(mut self, family_for: impl Fn(i64) -> Option<i64> + 'static) -> Self {
        self.family_for = Some(Box::new(family_for));
        self
    }

    /// Previews that did not land yet and will be sent again.
    pub fn owed_previews(&self) -> usize {
        self.owed_previews.borrow().len()
    }

    /// This pump as the send pipeline's uploads (`uploads: media.PushAsync`): every flush pushes
    /// what is owed before it posts anything, and a cache that refuses a write ends the flush.
    pub fn pump(self: &Rc<Self>) -> Uploads {
        let media = Rc::clone(self);
        Rc::new(move || -> LocalBoxFuture<'static, StoreResult<()>> {
            let media = Rc::clone(&media);
            Box::pin(async move { media.push().await.map(|_| ()) })
        })
    }

    /// Push what is owed. Answers how many files landed; a transient failure leaves the rest owed,
    /// and the next flush comes back to them.
    pub async fn push(&self) -> StoreResult<usize> {
        let mut landed = 0;
        for row in self.outbox.all()? {
            if row.failed() || !row.owes_uploads() {
                continue;
            }
            for handle in row.pending_files.clone().unwrap_or_default() {
                let staged = self.media.read(&handle);
                // A place without its numbers is as lost as a photo without its bytes.
                let Some(staged) = staged.filter(|staged| {
                    staged.kind != "location"
                        || (staged.latitude.is_some() && staged.longitude.is_some())
                }) else {
                    // The bytes are gone — a reclaimed temporary, or a reinstall. Nothing else is
                    // coming, and saying so beats a bubble that spins for ever.
                    self.outbox
                        .refuse(&row.client_msg_id, error_codes::MEDIA_MISSING)?;
                    self.refused.emit(&(
                        row.clone(),
                        ApiError::coded(error_codes::MEDIA_MISSING, "the file is no longer here"),
                    ));
                    break;
                };
                // For the family the CHAT is in, not whichever is shown by the time the upload goes
                // (docs/protocol.md, "Several families"): a message may use an attachment only in a
                // chat its family lists.
                let family = self.family_for.as_ref().and_then(|of| of(row.chat_id));
                let answer = match (staged.kind.as_str(), staged.latitude, staged.longitude) {
                    // A place has no bytes: its three numbers are the whole upload.
                    ("location", Some(latitude), Some(longitude)) => {
                        self.api
                            .upload_location(
                                latitude,
                                longitude,
                                staged.accuracy_m,
                                staged.name.as_deref(),
                                family,
                            )
                            .await
                    }
                    _ => {
                        let facts = UploadFacts {
                            width: staged.width,
                            height: staged.height,
                            duration_ms: staged.duration_ms,
                            name: staged.name.as_deref(),
                            waveform: staged.waveform.as_deref(),
                            family_id: family,
                        };
                        self.api
                            .upload(&staged.kind, &staged.mime, staged.bytes.clone(), &facts)
                            .await
                    }
                };
                match answer.into_result() {
                    Ok(answer) => {
                        // Remembered AT ONCE: the id is reusable within the grace, and a crash here
                        // costs the remainder rather than the lot.
                        self.outbox
                            .uploaded(&row.client_msg_id, answer.attachment.id, &handle)?;
                        landed += 1;
                        let attachment_id = answer.attachment.id;
                        let landing = (row.clone(), answer.attachment, staged);
                        self.landed.emit(&landing);
                        let (_, _, staged) = landing;
                        if let Some(preview) = staged.preview.filter(|preview| !preview.is_empty())
                        {
                            self.send_preview(
                                attachment_id,
                                preview,
                                MediaOutbox::PREVIEW_ATTEMPTS,
                            )
                            .await;
                        }
                    }
                    // Still owed. The next flush — a returning network, the window coming forward,
                    // the socket connecting — comes back to it.
                    Err(error) if error.transient() => break,
                    Err(error) => {
                        // A refusal about the FILE ITSELF: too large, not the kind it claimed, an
                        // image the server cannot read. Trying again with the same bytes would
                        // refuse again.
                        self.outbox.refuse(&row.client_msg_id, &error.code)?;
                        self.refused.emit(&(row.clone(), error));
                        break;
                    }
                }
            }
        }
        self.retry_previews().await;
        Ok(landed)
    }

    /// One preview. BEST EFFORT, BUT NOT BEST EFFORT ONCE: a message may go without its preview,
    /// but a video's poster is the one picture with no second source, and a poster that failed once
    /// would be a grey tile for every recipient for good. So a failed one is kept — the bytes are
    /// held here, because the staged file goes with its send — and sent again on later flushes, a
    /// bounded number of times. A REFUSAL is not tried again: the same bytes would be refused the
    /// same way.
    ///
    /// Owed until it is ANSWERED: a push dropped while a preview is on its way (the C#'s cancel)
    /// leaves that preview owed with the tries it had — where the C#, whose cancel is an exception
    /// out of the upload, lost it.
    async fn send_preview(&self, attachment_id: i64, jpeg: Vec<u8>, left: i32) {
        let mut sending = Requeue {
            owed: &self.owed_previews,
            due: VecDeque::from([(attachment_id, jpeg.clone(), left)]),
        };
        let answer = self.api.upload_preview(attachment_id, jpeg).await;
        let Some((attachment_id, jpeg, left)) = sending.due.pop_front() else {
            return;
        };
        if answer.error().is_none_or(|error| !error.transient()) || left <= 1 {
            return;
        }
        self.owed_previews
            .borrow_mut()
            .push((attachment_id, jpeg, left - 1));
    }

    /// Every owed preview, once more. The list is taken whole; what a push dropped part-way (the
    /// C#'s cancel) had not yet sent goes back on it untried — the C# put those back only for a
    /// cancel it saw between two previews, and lost the rest with the exception of one in flight.
    async fn retry_previews(&self) {
        let due: VecDeque<OwedPreview> =
            std::mem::take(&mut *self.owed_previews.borrow_mut()).into();
        let mut waiting = Requeue {
            owed: &self.owed_previews,
            due,
        };
        while let Some((attachment_id, jpeg, left)) = waiting.due.pop_front() {
            self.send_preview(attachment_id, jpeg, left).await;
        }
    }

    /// Forget the staged bytes no queued row names any more — a send that is over, one way or the
    /// other. Called after a flush, so the staging area cannot outlive the sends it is for.
    ///
    /// A SWEEP KEEPS WHAT IS STAGED, NOT WHAT IS OWED: a row two of whose three uploads have landed
    /// still needs those two files, because `attachment_expired` means push them again.
    pub fn sweep(&self) -> StoreResult<()> {
        let keep: HashSet<String> = self
            .outbox
            .all()?
            .into_iter()
            .flat_map(|row| row.staged_files.unwrap_or_default())
            .collect();
        self.media.sweep(&keep);
        Ok(())
    }

    /// What a row is waiting for, for a bubble that wants to say "sending 2 of 3": how many files
    /// landed, and how many are still owed.
    pub fn progress(&self, row: &OutboxRow) -> (usize, usize) {
        (
            row.attachment_ids.as_ref().map_or(0, Vec::len),
            row.pending_files.as_ref().map_or(0, Vec::len),
        )
    }
}

/// Previews not yet answered, put back on the owed list if the push that holds them is dropped.
struct Requeue<'a> {
    owed: &'a RefCell<Vec<OwedPreview>>,
    due: VecDeque<OwedPreview>,
}

impl Drop for Requeue<'_> {
    fn drop(&mut self) {
        self.owed.borrow_mut().extend(self.due.drain(..));
    }
}
