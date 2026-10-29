//! What is staged to go with one chat's next message (`ComposerStaging.cs`) — held in MEMORY until
//! the send, as the web client holds it.
//!
//! STAGED IS NOT QUEUED. Nothing reaches the media store before the person presses Send: the store
//! is swept after every flush down to what the OUTBOX names, and a chip that lived on disk before
//! its row existed would be swept out from under the composer.
//!
//! PREPARED ONE AT A TIME, IN ORDER, AND STOPPED AT THE CAP rather than preparing the rest only to
//! throw them away (the web client's `ingest`, and MacConversationView's before it).
//!
//! A BATCH CAN BE CALLED OFF ([`ComposerStaging::cancel_preparing`]). Since a video is transcoded
//! first (issue #74) preparing can take minutes, and somebody who dropped the wrong clip — or closed
//! the window — must not have to sit it out behind "Wait until the current attachment is done."
//! Here a call-off DROPS the preparer's future at once (dropping is cancelling: the shell's
//! transcode sets its pipeline to `Null` and deletes its temporary), so the composer is free the
//! moment the batch is next polled, not after the preparer noticed.
//!
//! The strip holds each item as an `Rc<StagedMedia>`, because an item is taken out by REFERENCE
//! ([`ComposerStaging::remove`]): the C#'s `ReferenceEquals` is `Rc::ptr_eq` here.

use std::cell::{Cell, RefCell};
use std::future::{Future, poll_fn};
use std::pin::pin;
use std::rc::Rc;
use std::task::Poll;

use fc_core::rt::Cancel;
use fc_core::text::media_text;
use fc_core::text::string_catalog::StringCatalog;
use fc_text::media::MAX_PER_MESSAGE;

use crate::media_outbox::StagedMedia;
use crate::voice_notes;

/// Why a picked file could not be staged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrepFailure {
    /// Over the protocol's ceiling for one attachment.
    TooLarge,
    /// Gone, locked, or a photo nothing on this device can decode.
    Unreadable,
}

/// One picked file, prepared — or the reason it was not. Neither (a preparer that answered no
/// media and no reason) is read as [`PrepFailure::Unreadable`], as the C# read it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PrepOutcome {
    pub media: Option<StagedMedia>,
    pub failure: Option<PrepFailure>,
}

impl PrepOutcome {
    pub fn staged(media: StagedMedia) -> Self {
        PrepOutcome {
            media: Some(media),
            failure: None,
        }
    }

    pub fn refused(failure: PrepFailure) -> Self {
        PrepOutcome {
            media: None,
            failure: Some(failure),
        }
    }
}

/// The staging strip of one chat's composer. Its state is in cells: the shell holds it in an `Rc`,
/// and a batch being prepared and the window's own calls reach it through `&self`.
#[derive(Debug, Default)]
pub struct ComposerStaging {
    items: RefCell<Vec<Rc<StagedMedia>>>,
    preparing: Cell<bool>,
    /// The batch being prepared, so it can be called off — with its number, so a batch ending
    /// clears only its own; `None` between batches.
    batch: RefCell<Option<(u64, Cancel)>>,
    batches: Cell<u64>,
}

impl ComposerStaging {
    pub fn new() -> Self {
        ComposerStaging::default()
    }

    /// What is staged, in order.
    pub fn items(&self) -> Vec<Rc<StagedMedia>> {
        self.items.borrow().clone()
    }

    /// How many are staged.
    pub fn len(&self) -> usize {
        self.items.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.borrow().is_empty()
    }

    /// Whether a batch is being prepared — the composer's other busy.
    pub fn preparing(&self) -> bool {
        self.preparing.get()
    }

    /// Whether one more fits in this message.
    pub fn can_stage(&self) -> bool {
        self.items.borrow().len() < MAX_PER_MESSAGE
    }

    /// Stage one, if it fits. Answers whether it did.
    pub fn add(&self, media: impl Into<Rc<StagedMedia>>) -> bool {
        if !self.can_stage() {
            return false;
        }
        self.items.borrow_mut().push(media.into());
        true
    }

    /// The item at `index` out of the strip; nothing when there is none there.
    pub fn remove_at(&self, index: usize) {
        let mut items = self.items.borrow_mut();
        if index < items.len() {
            items.remove(index);
        }
    }

    /// This very item out of the strip — by REFERENCE, not by place or by value: a question asked
    /// about it ("Delete this recording?") may be answered after other items came or went, and two
    /// notes can hold equal bytes. Answers whether it was still there.
    pub fn remove(&self, item: &Rc<StagedMedia>) -> bool {
        let mut items = self.items.borrow_mut();
        match items.iter().position(|held| Rc::ptr_eq(held, item)) {
            Some(at) => {
                items.remove(at);
                true
            }
            None => false,
        }
    }

    /// Everything staged, handed to a send, and the strip left empty.
    pub fn take_all(&self) -> Vec<Rc<StagedMedia>> {
        std::mem::take(&mut *self.items.borrow_mut())
    }

    /// Put a send's items back in front of anything staged since — the send could not be written
    /// down.
    pub fn restore(&self, taken: Vec<Rc<StagedMedia>>) {
        let mut items = self.items.borrow_mut();
        let later = std::mem::replace(&mut *items, taken);
        items.extend(later);
    }

    /// Whether a voice note recorded here is in review in this strip ([`voice_notes::is_recorded`]).
    pub fn holds_recordings(&self) -> bool {
        self.items
            .borrow()
            .iter()
            .any(|item| voice_notes::is_recorded(item))
    }

    /// The voice notes recorded here, taken out of the strip in their order — to wait as "not sent"
    /// when the person leaves the chat (S2.8). Everything else staged stays: a photo can be picked
    /// again, and a recording cannot be made again.
    pub fn take_recordings(&self) -> Vec<Rc<StagedMedia>> {
        let mut items = self.items.borrow_mut();
        let (taken, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut *items)
            .into_iter()
            .partition(|item| voice_notes::is_recorded(item));
        *items = kept;
        taken
    }

    /// Why nothing may be attached right now, or `None`. Which busy it is, because the two have
    /// different ways out (MacConversationView.composerBusyNotice).
    pub fn busy_reason(&self, editing: bool, say: &dyn StringCatalog) -> Option<String> {
        if editing {
            Some(
                say.get("Finish editing before attaching something.")
                    .to_string(),
            )
        } else if self.preparing.get() {
            Some(
                say.get("Wait until the current attachment is done.")
                    .to_string(),
            )
        } else {
            None
        }
    }

    /// Call off the batch being prepared: the file in hand is dropped (its preparer told to stop
    /// through its [`Cancel`] too), the rest are never started, and nothing is said — the person
    /// asked for this. What the batch had already staged stays; each has its own chip to remove it
    /// by. Nothing happens when no batch is running.
    pub fn cancel_preparing(&self) {
        if let Some((_, cancel)) = self.batch.borrow().as_ref() {
            cancel.cancel();
        }
    }

    /// Prepare `files` into this strip, one at a time and in order. Answers what to say afterwards —
    /// the cap, or the last refusal — or `None` for nothing. A pane that went away meanwhile
    /// (`still_here`) stops the batch: its files must not land in somebody else's composer.
    ///
    /// `prepare` is handed each file and the batch's [`Cancel`]; its future answers the outcome, or
    /// `Err` for anything that went wrong — what the C# caught as an exception — which is an
    /// unreadable file, not a crash. A batch called off stages nothing more and says nothing,
    /// whatever its preparer was about to answer: its future is dropped the moment the call-off is
    /// seen. Dropping THIS future ends the batch the same way.
    pub async fn ingest<T, P, F, E>(
        &self,
        files: impl IntoIterator<Item = T>,
        prepare: P,
        still_here: impl Fn() -> bool,
        say: &dyn StringCatalog,
    ) -> Option<String>
    where
        P: Fn(T, Cancel) -> F,
        F: Future<Output = Result<PrepOutcome, E>>,
    {
        let number = self.batches.get().wrapping_add(1);
        self.batches.set(number);
        let cancel = Cancel::new();
        *self.batch.borrow_mut() = Some((number, cancel.clone()));
        self.preparing.set(true);
        let _ending = BatchEnding {
            staging: self,
            number,
        };
        let mut said = None;
        for file in files {
            if !still_here() || cancel.is_cancelled() {
                return None;
            }
            if !self.can_stage() {
                said = Some(cap_sentence(say));
                break;
            }
            let outcome = {
                let mut preparing = pin!(prepare(file, cancel.clone()));
                let mut called_off = pin!(cancel.cancelled());
                let answered = poll_fn(|cx| {
                    if called_off.as_mut().poll(cx).is_ready() {
                        return Poll::Ready(None);
                    }
                    preparing.as_mut().poll(cx).map(Some)
                })
                .await;
                match answered {
                    None => return None,
                    Some(Ok(outcome)) => outcome,
                    Some(Err(_)) => PrepOutcome::refused(PrepFailure::Unreadable),
                }
            };
            if !still_here() || cancel.is_cancelled() {
                return None;
            }
            match outcome.media {
                Some(media) => self.items.borrow_mut().push(Rc::new(media)),
                None => {
                    said = Some(sentence(
                        outcome.failure.unwrap_or(PrepFailure::Unreadable),
                        say,
                    ))
                }
            }
        }
        said
    }
}

/// The C#'s `finally`: a batch that ends — answered, called off or dropped — is no longer preparing,
/// unless a later batch has begun meanwhile (the C# cleared the flag and the later batch's handle
/// either way).
struct BatchEnding<'a> {
    staging: &'a ComposerStaging,
    number: u64,
}

impl Drop for BatchEnding<'_> {
    fn drop(&mut self) {
        let mut batch = self.staging.batch.borrow_mut();
        if batch
            .as_ref()
            .is_some_and(|(number, _)| *number == self.number)
        {
            *batch = None;
            self.staging.preparing.set(false);
        }
    }
}

/// "You can attach up to 10 items."
pub fn cap_sentence(say: &dyn StringCatalog) -> String {
    say.format("You can attach up to %lld items.", &[&MAX_PER_MESSAGE])
}

/// What a refusal says.
pub fn sentence(failure: PrepFailure, say: &dyn StringCatalog) -> String {
    match failure {
        PrepFailure::TooLarge => say.get("That file is over the 100 MB limit.").to_string(),
        PrepFailure::Unreadable => say.get("Couldn't read that file.").to_string(),
    }
}

/// What a staged item is called on its chip: a picture by its kind, a file by its name and size —
/// and a voice note recorded here by its LENGTH, "Voice message · 0:42" (S2.7, S10): its size says
/// nothing to the person deciding whether to send it. `decimal_separator` is the reader's own
/// (the C#'s culture), for the size.
pub fn label(item: &StagedMedia, say: &dyn StringCatalog, decimal_separator: &str) -> String {
    match item.kind.as_str() {
        "photo" => say.get("Photo").to_string(),
        "video" if item.animation => say.get("GIF").to_string(),
        "video" => say.get("Video").to_string(),
        "audio" if voice_notes::is_recorded(item) => say.format(
            "Voice message · %@",
            &[&fc_text::media::time_label(
                f64::from(item.duration_ms.unwrap_or(0)) / 1000.0,
            )],
        ),
        // A sound file picked from disk keeps its name — a voice note is the audio with none
        // (`voice_notes::is_recorded`).
        "audio" | "file" => {
            let name = match &item.name {
                Some(name) => name.clone(),
                None => say.get("File").to_string(),
            };
            let size = media_text::display_size(
                i64::try_from(item.bytes.len()).unwrap_or(i64::MAX),
                decimal_separator,
            );
            format!("{name} · {size}")
        }
        _ => match &item.name {
            Some(name) => name.clone(),
            None => say.get("File").to_string(),
        },
    }
}
