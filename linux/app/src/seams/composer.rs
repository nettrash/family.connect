//! `ComposerSlots` (FROZEN, W.3) — P3's recording row and the slot's recording states inside P1's
//! composer (§6.1, §10.2, §10.5, §10.6).
//!
//! P1 owns the composer: the field, the paperclip, the Send slot and its face (`composer_button`),
//! the staged strip, the keys. P3 owns everything that records. They meet here:
//!
//! 1. P1 builds a composer and calls `shell.composer_slots.get().attach(host)` once, handing P3 a
//!    [`ComposerHost`] (P1's) and getting back a [`ComposerRecording`] (P3's).
//! 2. P1 fills `fc_text::record::SlotInputs` from it — `recording` ← [`ComposerRecording::recording`],
//!    `can_record` ← [`ComposerRecording::can_record`], `not_sent` ← [`ComposerRecording::not_sent`],
//!    `recorder_open` ← [`ComposerRecording::recorder_open`] — and asks `composer_slot` for the row.
//! 3. While `recording()` is not `Recording::None`, P1 shows [`ComposerRecording::row`] in the
//!    field's place (the field stays alive underneath: its words become the caption). Above the
//!    composer it shows [`ComposerRecording::not_sent_rows`] when that is `Some`.
//! 4. The slot's presses go to P3 by row: Microphone → `start_voice(false)`; the paperclip's
//!    "Record Voice Message" and Ctrl+Shift+R with words typed or items staged →
//!    `start_voice(true)`; SendVoice → `send()`; StopRecording and Escape → `stop()`; the video
//!    button and "Record Video Message" → `record_video()`.
//! 5. P3 tells P1 through the host: a finished note to stage ([`ComposerHost::stage`]) or to send
//!    now ([`ComposerHost::send`]), a video message ([`ComposerHost::send_video_message`], sent
//!    round), a refusal to say on the composer's line ([`ComposerHost::notice`]), the reply a
//!    parked recording takes ([`ComposerHost::reply_to`], [`ComposerHost::release_reply`]), and
//!    every change of state ([`ComposerHost::changed`]) so the slot is redrawn.
//! 6. P1 calls [`ComposerRecording::interrupt`] with the `RecordingEnd` of anything that ends a
//!    recording from its side (another chat, the page unmapped, a sign-out); P3's own session
//!    watch handles the lock, sleep, a minimised window and a call. While the video recorder is
//!    up P3 also calls `MainWindow::set_recorder_open(true)` (header buttons insensitive, routes
//!    held — §10.6).

use std::rc::Rc;

use fc_logic::composer_button::Recording;
use fc_logic::media_outbox::StagedMedia;
use fc_logic::not_sent::RecordingEnd;

/// P1's side of one composer, handed to P3.
pub trait ComposerHost {
    /// The chat this composer writes to.
    fn chat_id(&self) -> i64;

    /// The field's words as they stand — a parked note's caption (`NotSent`), and whether the
    /// draft is blank (row 2 or row 3).
    fn draft(&self) -> String;

    /// How many items are staged.
    fn staged(&self) -> usize;

    /// The window this composer is in (the recorder layer covers its content; a dialog's parent).
    fn window(&self) -> Option<gtk::Window>;

    /// Row 3's Stop: put the finished voice note into the staged strip beside the words. Nothing is
    /// sent until Send.
    fn stage(&self, note: StagedMedia);

    /// Row 2's Send arrow: send this voice note now, with the reply primed in the composer,
    /// through the outbox (`ComposerStaging`, `MediaOutbox`).
    fn send(&self, media: StagedMedia);

    /// A video message's Send (§10.5): `media` is the recorder's square clip with its poster, sent
    /// now as a VIDEO MESSAGE — the outbox row's `round: true` (docs/protocol.md, "Video
    /// messages") — with the reply primed. (Added at integration; the default sends it as
    /// [`send`](Self::send) would, which a composer that draws no circles may keep.)
    fn send_video_message(&self, media: StagedMedia) {
        self.send(media);
    }

    /// The recording's state changed: redraw the slot, show or hide the row.
    fn changed(&self);

    /// The reply the composer is primed with (`None` while an edit is open — an edit has no
    /// reply): a recording an interruption parks keeps it (`not_sent::for_recording`). (Added at
    /// integration.)
    fn reply_to(&self) -> Option<i64> {
        None
    }

    /// The parked recording took the reply with it: the composer lets go of it (its banner goes).
    /// (Added at integration.)
    fn release_reply(&self) {}

    /// Say `sentence` on the composer's notice line — a refusal ("You can record a message after
    /// the call."), "That recording was too short." — and to a screen reader unless a recording
    /// runs (the microphone would take the speech in). (Added at integration; the default says
    /// nothing.)
    fn notice(&self, sentence: &str) {
        let _ = sentence;
    }
}

/// P3's side of one composer, handed back to P1.
pub trait ComposerRecording {
    /// The row that takes the field's place while a voice note records (Delete, the red dot, the
    /// clock, the meter, Stop) — the same widget every time.
    fn row(&self) -> gtk::Widget;

    /// `SlotInputs::recording`.
    fn recording(&self) -> Recording;

    /// `SlotInputs::can_record`: a microphone can be opened at all.
    fn can_record(&self) -> bool;

    /// `SlotInputs::not_sent`: the chat holds a voice message that was not sent.
    fn not_sent(&self) -> bool;

    /// The "Voice message not sent" rows to stack above the composer, or `None` when there are
    /// none.
    fn not_sent_rows(&self) -> Option<gtk::Widget>;

    /// `SlotInputs::recorder_open`: the video message recorder owns the row.
    fn recorder_open(&self) -> bool;

    /// Whether the video button is offered in the empty field
    /// (`RoundVideoRules::recording_enabled`, a camera present, the server's limits).
    fn video_offered(&self) -> bool;

    /// Start a hands-free voice recording; `beside_draft` when words are typed or items staged
    /// (row 3: the note will be staged, not sent). A press while one runs is a stop.
    fn start_voice(&self, beside_draft: bool);

    /// Stop the recording: into the staged strip (row 3) or review.
    fn stop(&self);

    /// Stop and send (row 2's arrow).
    fn send(&self);

    /// End whatever records for this reason, keeping it as "not sent" where the plan says so.
    fn interrupt(&self, why: RecordingEnd);

    /// Open the video message recorder over the composer's window.
    fn record_video(&self);
}

/// P3's factory, installed in `Shell::composer_slots`.
pub trait ComposerSlots {
    /// The recording half of one composer. Called once per composer P1 builds.
    fn attach(&self, host: Rc<dyn ComposerHost>) -> Rc<dyn ComposerRecording>;
}

/// S0's stand-in: nothing records — `can_record` is false, so the slot never offers a microphone
/// (row 6, Send disabled while blank).
#[derive(Debug, Default)]
pub struct NoRecording;

impl ComposerSlots for NoRecording {
    fn attach(&self, _host: Rc<dyn ComposerHost>) -> Rc<dyn ComposerRecording> {
        Rc::new(NoRecording)
    }
}

impl ComposerRecording for NoRecording {
    fn row(&self) -> gtk::Widget {
        gtk::Box::new(gtk::Orientation::Horizontal, 0).into()
    }

    fn recording(&self) -> Recording {
        Recording::None
    }

    fn can_record(&self) -> bool {
        false
    }

    fn not_sent(&self) -> bool {
        false
    }

    fn not_sent_rows(&self) -> Option<gtk::Widget> {
        None
    }

    fn recorder_open(&self) -> bool {
        false
    }

    fn video_offered(&self) -> bool {
        false
    }

    fn start_voice(&self, _beside_draft: bool) {}

    fn stop(&self) {}

    fn send(&self) {}

    fn interrupt(&self, _why: RecordingEnd) {}

    fn record_video(&self) {}
}
