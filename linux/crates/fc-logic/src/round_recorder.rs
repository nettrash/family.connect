//! The video-message recorder as a machine (docs/audio-video-messages-2026-10-04.md, S3.2, S3.4,
//! S3.6, S4, S6) — `RoundRecorder.cs`: every press, tick and interruption goes in with the time on
//! one monotonic clock (milliseconds), and what the window must do comes out as [`RecorderEffect`]s
//! — so the rules of the three states are tested here, and the shell only draws and runs the camera
//! ([`VideoMessageRecorder`], below, which the shell implements over the camera portal, PipeWire and
//! GStreamer).
//!
//! **THE CAMERA IS OFF IN EVERY REFUSAL AND IN REVIEW, AND THE MICROPHONE OPENS ONLY AT RECORD**
//! (S3.2, S3.4): a closed recorder holds nothing, and PREVIEW never records.
//!
//! **NOTHING IS SENT BUT BY SEND** (S1.7): the length limit stops into REVIEW and says so; an
//! interruption stops into REVIEW, or closes PREVIEW; a real close over a clip asks first.
//!
//! **THE SLOT GUARDS ITS OWN PRESSES** (S1.1): for 600 ms after Record or Stop changed it, a click on
//! the slot is ignored, so a double click on Record cannot stop a take and a double click on Stop
//! cannot send one.
//!
//! The machine is a plain value the shell owns (`&mut self`; an `Rc<RefCell<RoundRecorder>>` where
//! several handlers reach it) — it awaits nothing, so it needs no cells of its own.
//!
//! ## What the shell does with each effect (Windows' `RoundRecorderLayer.DoAsync`)
//!
//! Run the effects IN ORDER, each to its end before the next (a step may await the camera), then
//! redraw from the machine's state:
//! - [`RecorderAction::OpenCamera`] — close whatever camera is still open first (never two at once);
//!   [`VideoMessageRecorder::cameras`], `round_video_rules::pick_camera` with the remembered id — none
//!   → [`RoundRecorder::camera_troubled`]`(Missing)`; [`VideoMessageRecorder::open`] — `Err(Refused)`
//!   → [`RoundRecorder::refuse`] (and keep `round_video_rules::refusal(…).1` for the settings
//!   button), `Err(Troubled)` → [`RoundRecorder::camera_troubled`]; `Ok` but the machine moved on
//!   meanwhile (no longer [`RecorderStage::Opening`]) → close it again; else subscribe to its
//!   [`CameraEvent`]s (below) and call [`RoundRecorder::camera_on`]. Mark the first-time line seen
//!   on this device once it was shown.
//! - [`RecorderAction::CloseCamera`] — [`VideoMessageRecorder::close`].
//! - [`RecorderAction::StartRecording`] — pause every app sound (S1.7), inhibit idle (shell-design
//!   §10.9), hold the layout; with a screen reader running announce "Recording video" first and wait
//!   `voice_notes`' one-second lead-in; then [`VideoMessageRecorder::start`] — `false` (or the
//!   machine no longer [`RecorderStage::Recording`]) → uninhibit and
//!   [`RoundRecorder::record_failed`]; `true` → [`RoundRecorder::take_started`]`(now)`.
//! - [`RecorderAction::StopAndKeep`] — [`VideoMessageRecorder::stop`], uninhibit; then — beside the
//!   steps, not in their line, so a question put up meanwhile does not wait — make the clip:
//!   `None` → [`RoundRecorder::clip_finished`]`(now, None, None)`; a take →
//!   [`VideoMessageRecorder::read_back`] and `round_video_rules::plan_take`; the square (the plan's
//!   `from_square`) is staged with `round_video_rules::staged` and its [`VideoMessageRecorder::poster`]
//!   (or as a regular 480 × 480 video when not `round`); otherwise the take goes through the
//!   ordinary picked-video path (`MediaPreparing`); then [`RoundRecorder::clip_finished`] with the
//!   plan and the staged length. A take thrown away meanwhile lands nowhere.
//! - [`RecorderAction::StopAndDiscard`] — [`VideoMessageRecorder::discard`], uninhibit.
//! - [`RecorderAction::DiscardClip`] — delete the made clip and its files.
//! - [`RecorderAction::PauseClip`] — pause REVIEW's playback. [`RecorderAction::Ask`] /
//!   [`RecorderAction::Dismiss`] — put up / take down "Delete video message?" \[Delete\] \[Keep\].
//! - [`RecorderAction::Send`] — send the staged clip with `round` as its plan says, and the
//!   composer's primed reply. [`RecorderAction::StartVoice`]: start a hands-free voice recording
//!   once the layer is gone. [`RecorderAction::CloseWindow`]: let the pending close go ahead.
//!   [`RecorderAction::Closed`] — take the layer away, focus back to what opened it.
//! - [`RecorderAction::Announce`] — `gtk::Accessible::announce(text, Polite)` — but not a second
//!   "Recording video" already said before the microphone opened.
//!
//! [`CameraEvent`]s, on the main context, only while that camera is still the open one:
//! `FirstFrame` → [`RoundRecorder::first_frame`]; `Brightness(luma)` →
//! `round_video_rules::PictureCheck::frame(now, luma)` (a new check per open), and when it answers
//! `true` → [`RoundRecorder::covered`]; `Blocked` → [`RoundRecorder::covered`]; `Troubled(t)` →
//! [`RoundRecorder::camera_troubled`]. A one-second timer calls [`RoundRecorder::tick`] while the
//! recorder is open.

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use fc_core::protocol::RoundVideoLimits;
use fc_core::rt::{Event, LocalBoxFuture};
use fc_core::text::string_catalog::StringCatalog;
use fc_text::record;

use crate::media_encoding::VideoEncoding;
use crate::round_look::{PlaybackEvent, Playing, playback_pauses};
use crate::round_video_rules::{
    self, CameraChoice, CameraTrouble, RecorderRefusal, RoundSendPlan, TakeReading,
};
use crate::voice_notes::{self, CapabilityAccess};

/// Where the video-message recorder is (docs/audio-video-messages-2026-10-04.md, S3.4) — the
/// plan's three states and the ways around them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecorderStage {
    /// The camera is being opened — and, the first time, the desktop is asking for it.
    Opening,
    /// The camera or the microphone was refused (S3.2): the camera is off and the sentence says
    /// where to mend it.
    Refused,
    /// The camera cannot be used — another app has it, it went, or its preview would stay blank
    /// (S3.6).
    Unavailable,
    /// The live front camera, mirrored; the microphone not opened; Record.
    Preview,
    /// Recording: the camera and the microphone on, the ring filling, Stop.
    Recording,
    /// The clip as it will be sent, the camera off; Send, Delete, Retake.
    Review,
    /// Gone: the camera off and nothing held.
    Closed,
}

/// A question the recorder has put up, and what its answer is about (S3.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RecorderQuestion {
    #[default]
    None,
    /// Delete during a take of ten seconds or more: it stopped first; Keep goes to REVIEW.
    DeleteWhileRecording,
    /// Delete in REVIEW, ten seconds or more: Delete closes the recorder.
    DeleteReview,
    /// Retake in REVIEW, ten seconds or more: Delete turns the camera back on.
    Retake,
    /// Esc in REVIEW, which always asks: Delete closes the recorder.
    Escape,
    /// The window really closing over a clip (S4): Delete lets it close; Keep cancels the close.
    CloseWindow,
}

/// Something other than the person that reaches the recorder (S4's three video columns;
/// shell-design §10.8 says where each comes from on Linux).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecorderInterruption {
    /// A call rings, starts or is placed.
    Call,
    /// The window minimised (`notify::suspended`), or hidden to the top bar.
    WindowHidden,
    /// The session locked, the screen saver started, or the computer is going to sleep.
    SessionLocked,
    /// The window only lost focus and is still visible.
    FocusLost,
    /// The window really closing: Quit, or the close button with Keep running off.
    WindowClosing,
    /// The session ended — signed out, the family left, the server changed.
    SignedOut,
}

/// What the window does for one step of the recorder (the module's comment says how).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecorderAction {
    /// Open the camera (PREVIEW again after a Retake, or after a clip that could not be read).
    OpenCamera,
    /// Let go of the camera and the microphone — the light goes out.
    CloseCamera,
    /// Open the microphone and start writing the take.
    StartRecording,
    /// Stop the take and keep it: finish the file and make the clip.
    StopAndKeep,
    /// Stop the take and throw it away.
    StopAndDiscard,
    /// Throw away the finished clip.
    DiscardClip,
    /// Pause the clip REVIEW is playing back — a call, a lock, a hidden window, the output gone
    /// (S4's last column).
    PauseClip,
    /// Put up "Delete video message?" \[Delete\] \[Keep\].
    Ask,
    /// Take the question away unanswered — another one replaces it.
    Dismiss,
    /// Send the clip — round or not, as its plan says — with the composer's primed reply.
    Send,
    /// Start a hands-free voice recording in the composer.
    StartVoice,
    /// Close the window now — the person deleted the clip that was holding it.
    CloseWindow,
    /// Take the recorder away and give focus back to whatever opened it.
    Closed,
    /// Say [`RecorderEffect::text`] to a screen reader — politely, and only state changes (S6).
    Announce,
}

/// One thing for the window to do, with what it says where it says something.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecorderEffect {
    pub action: RecorderAction,
    pub text: Option<String>,
}

impl RecorderEffect {
    pub fn new(action: RecorderAction) -> Self {
        RecorderEffect { action, text: None }
    }
}

/// What the status line says (S3.4): the line itself, and the one under it — a first-time hint, a
/// warning or a notice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecorderStatus {
    pub line: String,
    pub below: Option<String>,
    pub red: bool,
    pub warning: bool,
}

impl RecorderStatus {
    pub fn new(line: &str, below: Option<&str>, red: bool, warning: bool) -> Self {
        RecorderStatus {
            line: line.to_string(),
            below: below.map(str::to_string),
            red,
            warning,
        }
    }
}

/// The video-message recorder's state machine (`RoundRecorder(limits, say)`).
pub struct RoundRecorder {
    limits: RoundVideoLimits,
    say: Rc<dyn StringCatalog>,
    recording_at: i64,
    take_started: bool,
    stopped_by_interruption: bool,
    last_used: i64,
    guard_until: i64,
    warned: bool,
    stage: RecorderStage,
    question: RecorderQuestion,
    has_picture: bool,
    looks_covered: bool,
    asking: bool,
    clip_ready: bool,
    clip_ms: i64,
    refused: Option<RecorderRefusal>,
    trouble: Option<CameraTrouble>,
    plan: Option<RoundSendPlan>,
    notice: Option<String>,
}

impl RoundRecorder {
    pub fn new(limits: RoundVideoLimits, say: Rc<dyn StringCatalog>) -> Self {
        RoundRecorder {
            limits,
            say,
            recording_at: 0,
            take_started: false,
            stopped_by_interruption: false,
            last_used: 0,
            guard_until: i64::MIN,
            warned: false,
            stage: RecorderStage::Closed,
            question: RecorderQuestion::None,
            has_picture: false,
            looks_covered: false,
            asking: false,
            clip_ready: false,
            clip_ms: 0,
            refused: None,
            trouble: None,
            plan: None,
            notice: None,
        }
    }

    pub fn stage(&self) -> RecorderStage {
        self.stage
    }

    pub fn question(&self) -> RecorderQuestion {
        self.question
    }

    /// The camera has delivered its first frame: Record is live (S3.4).
    pub fn has_picture(&self) -> bool {
        self.has_picture
    }

    /// The first two seconds stayed near-black (S3.6).
    pub fn looks_covered(&self) -> bool {
        self.looks_covered
    }

    /// The desktop is asking for the camera and microphone right now — losing focus to that is not
    /// leaving (S4).
    pub fn asking(&self) -> bool {
        self.asking
    }

    /// The finished clip is ready to play and send; false while it is still being made.
    pub fn clip_ready(&self) -> bool {
        self.clip_ready
    }

    /// How long the clip in REVIEW is.
    pub fn clip_ms(&self) -> i64 {
        self.clip_ms
    }

    /// What was refused ([`RecorderStage::Refused`]).
    pub fn refused(&self) -> Option<RecorderRefusal> {
        self.refused
    }

    /// Why the camera cannot be used ([`RecorderStage::Unavailable`]).
    pub fn trouble(&self) -> Option<CameraTrouble> {
        self.trouble
    }

    /// What the clip will be sent as, once it is known.
    pub fn plan(&self) -> Option<&RoundSendPlan> {
        self.plan.as_ref()
    }

    /// A sentence shown under the status line until the next change: "That video was too short.",
    /// "Recording stopped at one minute."
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    /// Whether something recorded would be lost by closing: a take running, or a clip in REVIEW (S4).
    pub fn holds_clip(&self) -> bool {
        matches!(self.stage, RecorderStage::Recording | RecorderStage::Review)
    }

    pub fn is_open(&self) -> bool {
        self.stage != RecorderStage::Closed
    }

    /// Space is the recorder's, caught before the focused control (S3.4): in REVIEW it plays and
    /// pauses wherever focus is — while the clip is still being made too, when it does nothing —
    /// and never sends, deletes or retakes. Not while the question is up: that is the question's.
    pub fn catches_space(&self) -> bool {
        self.stage == RecorderStage::Review && self.question == RecorderQuestion::None
    }

    /// The take's length so far, or the clip's. A take counts from [`take_started`](Self::take_started)
    /// — when the file really began, after a screen reader's lead-in and the camera's own start —
    /// never from the press (S6: the timer starts with it).
    pub fn elapsed_ms(&self, now: i64) -> i64 {
        match self.stage {
            RecorderStage::Recording if self.take_started => {
                now.saturating_sub(self.recording_at).max(0)
            }
            RecorderStage::Review => self.clip_ms,
            _ => 0,
        }
    }

    // ---- opening ------------------------------------------------------------------------------

    /// Opened from the video button, the paperclip or the menu (S3.1). Something already refused
    /// opens straight to it and asks the desktop nothing (S3.2); otherwise the camera is opened,
    /// with the neutral line while the desktop asks.
    pub fn open(
        &mut self,
        now: i64,
        refused_already: Option<RecorderRefusal>,
        desktop_will_ask: bool,
    ) -> Vec<RecorderEffect> {
        self.reset(now);
        if let Some(refused) = refused_already {
            self.stage = RecorderStage::Refused;
            self.refused = Some(refused);
            return Vec::new();
        }
        self.stage = RecorderStage::Opening;
        self.asking = desktop_will_ask;
        vec![RecorderEffect::new(RecorderAction::OpenCamera)]
    }

    /// The camera is on and its preview attached; the picture follows ([`first_frame`](Self::first_frame)).
    pub fn camera_on(&mut self, now: i64) -> Vec<RecorderEffect> {
        if self.stage != RecorderStage::Opening {
            // Opened after the recorder closed or moved on: let go of it again.
            return if matches!(self.stage, RecorderStage::Closed | RecorderStage::Review) {
                vec![RecorderEffect::new(RecorderAction::CloseCamera)]
            } else {
                Vec::new()
            };
        }
        self.stage = RecorderStage::Preview;
        self.asking = false;
        self.last_used = now;
        Vec::new()
    }

    /// The first frame arrived: Record comes alive and "Camera ready" is said.
    pub fn first_frame(&mut self) -> Vec<RecorderEffect> {
        if self.stage != RecorderStage::Preview || self.has_picture {
            return Vec::new();
        }
        self.has_picture = true;
        vec![announce(self.say.get("Camera ready"))]
    }

    /// The picture stayed near-black for two seconds (S3.6): said in the status line; Record stays
    /// usable.
    pub fn covered(&mut self) {
        if self.stage == RecorderStage::Preview {
            self.looks_covered = true;
        }
    }

    /// The desktop refused the camera or the microphone while opening (S3.2): the camera is off.
    pub fn refuse(&mut self, refused: RecorderRefusal) -> Vec<RecorderEffect> {
        if !matches!(self.stage, RecorderStage::Opening | RecorderStage::Preview) {
            return Vec::new();
        }
        self.stage = RecorderStage::Refused;
        self.refused = Some(refused);
        self.asking = false;
        vec![RecorderEffect::new(RecorderAction::CloseCamera)]
    }

    /// The camera cannot be used, or stopped being usable (S3.6): PREVIEW says why in place of the
    /// picture, Record dimmed and a voice message offered; a take in progress stops into REVIEW with
    /// what was recorded and the same sentence.
    pub fn camera_troubled(&mut self, trouble: CameraTrouble, now: i64) -> Vec<RecorderEffect> {
        match self.stage {
            RecorderStage::Opening | RecorderStage::Preview => {
                self.stage = RecorderStage::Unavailable;
                self.trouble = Some(trouble);
                self.asking = false;
                self.has_picture = false;
                vec![RecorderEffect::new(RecorderAction::CloseCamera)]
            }
            RecorderStage::Recording => {
                // Taken by another app: the camera's own sentence (S3.6); gone or failed: what S4
                // says of a recorder that fails.
                let notice = if trouble == CameraTrouble::InUse {
                    round_video_rules::trouble_sentence(trouble, self.say.as_ref())
                } else {
                    self.say
                        .get("The recording stopped unexpectedly.")
                        .to_string()
                };
                self.stop_into(now, Some(notice), true)
            }
            _ => Vec::new(),
        }
    }

    // ---- the slot and the controls (S3.4) -----------------------------------------------------

    /// The slot — Record, Stop or Send — clicked or Return pressed on it. Ignored for 600 ms after
    /// its own Record or Stop (S1.1), and Record until the first frame.
    pub fn slot(&mut self, now: i64) -> Vec<RecorderEffect> {
        if now < self.guard_until {
            return Vec::new();
        }
        self.last_used = now;
        match self.stage {
            RecorderStage::Preview if self.has_picture => {
                self.stage = RecorderStage::Recording;
                self.recording_at = now;
                self.take_started = false;
                self.warned = false;
                self.notice = None;
                self.looks_covered = false;
                self.guard_until = now.saturating_add(guard_ms());
                vec![
                    RecorderEffect::new(RecorderAction::StartRecording),
                    announce(self.say.get("Recording video")),
                ]
            }
            RecorderStage::Recording => {
                self.guard_until = now.saturating_add(guard_ms());
                self.stop(now, None)
            }
            RecorderStage::Review if self.clip_ready && self.plan.is_some() => {
                let sent = announce(self.say.get("Video message sent"));
                self.close(vec![RecorderEffect::new(RecorderAction::Send), sent], false)
            }
            _ => Vec::new(),
        }
    }

    /// Close (PREVIEW, and every state that has no picture to keep): the camera goes off.
    pub fn close_button(&mut self) -> Vec<RecorderEffect> {
        if matches!(
            self.stage,
            RecorderStage::Opening
                | RecorderStage::Refused
                | RecorderStage::Unavailable
                | RecorderStage::Preview
        ) {
            self.close(Vec::new(), true)
        } else {
            Vec::new()
        }
    }

    /// Delete: during a take under ten seconds it is gone at once and PREVIEW comes back; from ten
    /// seconds the take stops first and the question is put. In REVIEW the same, closing the
    /// recorder.
    pub fn delete(&mut self, now: i64) -> Vec<RecorderEffect> {
        self.last_used = now;
        match self.stage {
            RecorderStage::Recording if self.elapsed_ms(now) < delete_asks_from_ms() => {
                self.stage = RecorderStage::Preview;
                self.notice = None;
                vec![
                    RecorderEffect::new(RecorderAction::StopAndDiscard),
                    announce(self.say.get("Recording deleted")),
                ]
            }
            RecorderStage::Recording => {
                // Stopped while the question is up: nothing more is recorded. The camera stays on
                // for a Delete that goes back to PREVIEW; Keep turns it off.
                self.clip_ms = self.elapsed_ms(now);
                self.stage = RecorderStage::Review;
                self.clip_ready = false;
                self.stopped_by_interruption = false;
                let mut effects = vec![RecorderEffect::new(RecorderAction::StopAndKeep)];
                effects.extend(self.ask(RecorderQuestion::DeleteWhileRecording));
                effects
            }
            RecorderStage::Review if self.clip_ms < delete_asks_from_ms() => self.close(
                vec![RecorderEffect::new(RecorderAction::DiscardClip)],
                false,
            ),
            RecorderStage::Review => self.ask(RecorderQuestion::DeleteReview),
            _ => Vec::new(),
        }
    }

    /// Retake (REVIEW): from ten seconds it asks first; then the clip goes and the camera comes back
    /// on.
    pub fn retake(&mut self, now: i64) -> Vec<RecorderEffect> {
        if self.stage != RecorderStage::Review {
            return Vec::new();
        }
        self.last_used = now;
        if self.clip_ms >= delete_asks_from_ms() {
            self.ask(RecorderQuestion::Retake)
        } else {
            self.back_to_preview(now)
        }
    }

    /// Esc (and a screen reader's way of leaving): PREVIEW and the states with no picture close;
    /// RECORDING stops; REVIEW always asks "Delete video message?".
    pub fn escape(&mut self, now: i64) -> Vec<RecorderEffect> {
        if self.question != RecorderQuestion::None {
            return Vec::new();
        }
        match self.stage {
            RecorderStage::Recording => self.stop(now, None),
            RecorderStage::Review => self.ask(RecorderQuestion::Escape),
            _ => self.close_button(),
        }
    }

    /// "Record a voice message instead" (S3.4): the camera closes and a hands-free voice recording
    /// starts — unless a voice message that was not sent waits, which it says instead (row 9).
    pub fn voice_instead(&mut self, not_sent_waits: bool) -> Vec<RecorderEffect> {
        if !matches!(
            self.stage,
            RecorderStage::Opening
                | RecorderStage::Preview
                | RecorderStage::Refused
                | RecorderStage::Unavailable
        ) || self.refused == Some(RecorderRefusal::Microphone)
        {
            return Vec::new();
        }
        if not_sent_waits {
            let notice = self
                .say
                .get("Send or delete the voice message that wasn't sent first.")
                .to_string();
            self.notice = Some(notice.clone());
            return vec![announce(&notice)];
        }
        self.close(vec![RecorderEffect::new(RecorderAction::StartVoice)], true)
    }

    /// Another camera chosen from "Choose camera" (S3.5) — in PREVIEW, or in place of one that
    /// cannot be used: this one goes off and that one opens.
    pub fn switch_camera(&mut self, now: i64) -> Vec<RecorderEffect> {
        if !matches!(
            self.stage,
            RecorderStage::Preview | RecorderStage::Unavailable
        ) {
            return Vec::new();
        }
        self.last_used = now;
        self.stage = RecorderStage::Opening;
        self.has_picture = false;
        self.looks_covered = false;
        self.trouble = None;
        self.notice = None;
        vec![
            RecorderEffect::new(RecorderAction::CloseCamera),
            RecorderEffect::new(RecorderAction::OpenCamera),
        ]
    }

    /// The take really began — the microphone open and the file being written, after the screen
    /// reader's lead-in (S6): the clock, the ring, the warning, the limit and the 1.0 s floor all
    /// count from here, so what is measured is what the file holds. Before it a Stop finds nothing
    /// recorded — "too short" — and the clock shows 0:00.
    pub fn take_started(&mut self, now: i64) {
        if self.stage == RecorderStage::Recording && !self.take_started {
            self.take_started = true;
            self.recording_at = now;
        }
    }

    /// The take would not start — the microphone refused to open, the disk full: back to PREVIEW,
    /// saying so.
    pub fn record_failed(&mut self) -> Vec<RecorderEffect> {
        if self.stage != RecorderStage::Recording {
            return Vec::new();
        }
        self.stage = RecorderStage::Preview;
        let notice = self.say.get("Couldn't start recording.").to_string();
        self.notice = Some(notice.clone());
        vec![announce(&notice)]
    }

    /// Any other control used — the camera menu, the reply's ✕ — which keeps PREVIEW from timing
    /// out.
    pub fn used(&mut self, now: i64) {
        self.last_used = now;
    }

    /// The answer to the question that is up: Delete (`true`) or Keep (`false`).
    pub fn answer(&mut self, delete: bool, now: i64) -> Vec<RecorderEffect> {
        let question = self.question;
        self.question = RecorderQuestion::None;
        self.last_used = now;
        match question {
            RecorderQuestion::DeleteWhileRecording if delete => {
                self.clip_ready = false;
                self.clip_ms = 0;
                self.plan = None;
                self.stage = RecorderStage::Preview;
                vec![
                    RecorderEffect::new(RecorderAction::DiscardClip),
                    announce(self.say.get("Recording deleted")),
                ]
            }
            // Keep: REVIEW, and the light goes out.
            RecorderQuestion::DeleteWhileRecording => {
                vec![RecorderEffect::new(RecorderAction::CloseCamera)]
            }
            RecorderQuestion::DeleteReview | RecorderQuestion::Escape if delete => self.close(
                vec![RecorderEffect::new(RecorderAction::DiscardClip)],
                false,
            ),
            RecorderQuestion::Retake if delete => self.back_to_preview(now),
            RecorderQuestion::CloseWindow if delete => self.close(
                vec![
                    RecorderEffect::new(RecorderAction::DiscardClip),
                    RecorderEffect::new(RecorderAction::CloseWindow),
                ],
                false,
            ),
            _ => Vec::new(),
        }
    }

    // ---- the take's end -----------------------------------------------------------------------

    /// The take's file is finished and its clip made — or not (`plan` says what is sent), and how
    /// long it is. A take nothing could be read back from goes back to PREVIEW with "The recording
    /// stopped unexpectedly." (S4). A clip that reads back shorter than 1.0 s is never sent (S1.1):
    /// the person's Stop goes back to PREVIEW with "That video was too short."; one an interruption
    /// stopped is deleted and the recorder closes (S4: under 1.0 s, deleted); one a closing window
    /// asked about lets the close go ahead.
    pub fn clip_finished(
        &mut self,
        now: i64,
        plan: Option<RoundSendPlan>,
        duration_ms: Option<i64>,
    ) -> Vec<RecorderEffect> {
        if self.stage != RecorderStage::Review || self.clip_ready {
            return Vec::new();
        }
        let too_short = duration_ms.is_some_and(|length| length < shortest_recording_ms());
        let Some(plan) = plan else {
            self.question = RecorderQuestion::None;
            let mut effects = vec![RecorderEffect::new(RecorderAction::Dismiss)];
            effects.extend(self.back_to_preview(now));
            let notice = self
                .say
                .get("The recording stopped unexpectedly.")
                .to_string();
            self.notice = Some(notice.clone());
            effects.push(announce(&notice));
            return effects;
        };
        if too_short {
            let asked = self.question;
            let mut effects = if asked != RecorderQuestion::None {
                vec![RecorderEffect::new(RecorderAction::Dismiss)]
            } else {
                Vec::new()
            };
            self.question = RecorderQuestion::None;
            if asked == RecorderQuestion::CloseWindow {
                effects.push(RecorderEffect::new(RecorderAction::DiscardClip));
                effects.push(RecorderEffect::new(RecorderAction::CloseWindow));
                return self.close(effects, false);
            }
            if self.stopped_by_interruption {
                effects.push(RecorderEffect::new(RecorderAction::DiscardClip));
                return self.close(effects, false);
            }
            effects.extend(self.back_to_preview(now));
            let notice = self.say.get("That video was too short.").to_string();
            self.notice = Some(notice.clone());
            effects.push(announce(&notice));
            return effects;
        }
        self.clip_ready = true;
        if let Some(length) = duration_ms.filter(|length| *length > 0) {
            self.clip_ms = length;
        }
        let notice = plan.notice.clone();
        self.plan = Some(plan);
        if let Some(notice) = notice {
            self.notice = Some(notice.clone());
            return vec![announce(&notice)];
        }
        vec![announce(&round_video_rules::review_line(
            self.clip_ms,
            self.say.as_ref(),
        ))]
    }

    /// The clock: PREVIEW closes after a minute untouched ("Camera turned off"); a take says "10
    /// seconds left" at the warning and stops itself at the limit, into REVIEW, with "Recording
    /// stopped at one minute." — never sent by it.
    pub fn tick(&mut self, now: i64) -> Vec<RecorderEffect> {
        match self.stage {
            RecorderStage::Preview | RecorderStage::Opening
                if !self.asking
                    && now.saturating_sub(self.last_used) >= round_video_rules::PREVIEW_IDLE_MS =>
            {
                let said = announce(self.say.get("Camera turned off"));
                self.close(vec![said], true)
            }
            RecorderStage::Recording
                if self.elapsed_ms(now) >= round_video_rules::cap_ms(&self.limits) =>
            {
                let notice = self.say.get("Recording stopped at one minute.").to_string();
                self.stop(now, Some(notice))
            }
            RecorderStage::Recording
                if !self.warned
                    && self.elapsed_ms(now) >= round_video_rules::warning_ms(&self.limits) =>
            {
                self.warned = true;
                vec![announce(self.say.get("10 seconds left"))]
            }
            _ => Vec::new(),
        }
    }

    /// Whether the ring has turned orange: from the warning on (S3.4) — with the words beside it,
    /// never colour alone.
    pub fn warns(&self, now: i64) -> bool {
        self.stage == RecorderStage::Recording
            && self.elapsed_ms(now) >= round_video_rules::warning_ms(&self.limits)
    }

    /// How far round the red ring has filled, over the length limit (S3.4).
    pub fn ring_fraction(&self, now: i64) -> f64 {
        if self.stage != RecorderStage::Recording {
            return 0.0;
        }
        let cap = round_video_rules::cap_ms(&self.limits);
        if cap <= 0 {
            1.0
        } else {
            (self.elapsed_ms(now) as f64 / cap as f64).clamp(0.0, 1.0)
        }
    }

    // ---- interruptions (S4) -------------------------------------------------------------------

    /// Something other than the person (S4): PREVIEW closes — but not for focus lost to a permission
    /// prompt it raised; a take stops into REVIEW — but keeps recording while the window is merely
    /// unfocused; REVIEW is kept, and a real close over it asks first. A sign-out deletes
    /// everything.
    pub fn interrupt(&mut self, why: RecorderInterruption, now: i64) -> Vec<RecorderEffect> {
        if self.stage == RecorderStage::Closed {
            return Vec::new();
        }
        if why == RecorderInterruption::SignedOut {
            let mut gone = if self.question != RecorderQuestion::None {
                vec![RecorderEffect::new(RecorderAction::Dismiss)]
            } else {
                Vec::new()
            };
            self.question = RecorderQuestion::None;
            gone.push(RecorderEffect::new(
                if self.stage == RecorderStage::Recording {
                    RecorderAction::StopAndDiscard
                } else {
                    RecorderAction::DiscardClip
                },
            ));
            let camera_on = self.stage != RecorderStage::Review;
            return self.close(gone, camera_on);
        }
        match self.stage {
            RecorderStage::Opening if why == RecorderInterruption::FocusLost && self.asking => {
                Vec::new()
            }
            RecorderStage::Opening
            | RecorderStage::Preview
            | RecorderStage::Refused
            | RecorderStage::Unavailable => self.close(Vec::new(), true),
            RecorderStage::Recording if why == RecorderInterruption::FocusLost => Vec::new(),
            RecorderStage::Recording if why == RecorderInterruption::WindowClosing => {
                let mut effects = self.stop_into(now, None, true);
                effects.extend(self.ask(RecorderQuestion::CloseWindow));
                effects
            }
            RecorderStage::Recording => self.stop_into(now, None, true),
            RecorderStage::Review
                if self.question == RecorderQuestion::DeleteWhileRecording
                    && why != RecorderInterruption::FocusLost =>
            {
                // A take stopped to ask about deleting it kept the camera on for a Delete that goes
                // back to PREVIEW. Taken out of the question's hands, it is kept — and the light
                // goes out, before a call reaches for the camera.
                self.question = RecorderQuestion::None;
                let mut kept = vec![
                    RecorderEffect::new(RecorderAction::Dismiss),
                    RecorderEffect::new(RecorderAction::CloseCamera),
                ];
                if why == RecorderInterruption::WindowClosing {
                    kept.extend(self.ask(RecorderQuestion::CloseWindow));
                }
                kept
            }
            RecorderStage::Review if why == RecorderInterruption::WindowClosing => {
                if self.question == RecorderQuestion::CloseWindow {
                    return Vec::new();
                }
                let mut effects = if self.question != RecorderQuestion::None {
                    vec![RecorderEffect::new(RecorderAction::Dismiss)]
                } else {
                    Vec::new()
                };
                effects.extend(self.ask(RecorderQuestion::CloseWindow));
                effects
            }
            _ => Vec::new(),
        }
    }

    /// Something that pauses what plays (S4's last column, `round_look::playback_pauses`) while
    /// REVIEW may be playing its clip back: a call, a lock, a hidden window or the output device
    /// going pause it as they pause any circle; losing focus lets it play on.
    pub fn playback_interrupted(&self, happened: PlaybackEvent) -> Vec<RecorderEffect> {
        if self.stage == RecorderStage::Review
            && self.clip_ready
            && playback_pauses::pauses(happened, Playing::RoundVideo)
        {
            vec![RecorderEffect::new(RecorderAction::PauseClip)]
        } else {
            Vec::new()
        }
    }

    // ---- what the window draws ----------------------------------------------------------------

    /// The status line (S3.4): "Starting camera…" or, while the desktop asks, "Video messages need
    /// the camera and the microphone."; the refusal or the camera's trouble; "Not recording" —
    /// "Camera and microphone on · Not recording" where the microphone is held
    /// (`round_video_rules::MICROPHONE_ON_IN_PREVIEW`) — with the first-time line or the
    /// covered-camera question under it; the red clock with "10 seconds left"; "Video message ·
    /// 0:23".
    pub fn status(
        &self,
        now: i64,
        first_time_on_device: bool,
        microphone_on_in_preview: bool,
    ) -> RecorderStatus {
        let say = self.say.as_ref();
        let notice = self.notice.as_deref();
        match self.stage {
            RecorderStage::Opening => {
                let line = if self.asking {
                    say.get("Video messages need the camera and the microphone.")
                } else {
                    say.get("Starting camera…")
                };
                RecorderStatus::new(line, notice, false, false)
            }
            RecorderStage::Refused => {
                let (line, _) = round_video_rules::refusal(
                    self.refused.unwrap_or(RecorderRefusal::Camera),
                    CapabilityAccess::Unknown,
                    say,
                );
                RecorderStatus::new(&line, notice, false, false)
            }
            RecorderStage::Unavailable => RecorderStatus::new(
                &round_video_rules::trouble_sentence(
                    self.trouble.unwrap_or(CameraTrouble::Failed),
                    say,
                ),
                notice,
                false,
                false,
            ),
            RecorderStage::Preview if !self.has_picture => {
                RecorderStatus::new(say.get("Starting camera…"), notice, false, false)
            }
            RecorderStage::Preview => {
                let line = if microphone_on_in_preview {
                    say.get("Camera and microphone on · Not recording")
                } else {
                    say.get("Not recording")
                };
                let below = notice.or_else(|| {
                    if self.looks_covered {
                        Some(say.get("We can't see anything. Is the camera turned off or covered?"))
                    } else if first_time_on_device {
                        Some(say.get("Only you can see this until you start recording."))
                    } else {
                        None
                    }
                });
                RecorderStatus::new(line, below, false, false)
            }
            RecorderStage::Recording => {
                let warns = self.warns(now);
                let elapsed = Duration::from_millis(self.elapsed_ms(now).max(0) as u64);
                let below = warns.then(|| say.get("10 seconds left"));
                RecorderStatus::new(&voice_notes::clock(elapsed), below, true, warns)
            }
            RecorderStage::Review => RecorderStatus::new(
                &round_video_rules::review_line(self.clip_ms, say),
                notice,
                false,
                false,
            ),
            RecorderStage::Closed => RecorderStatus::new("", None, false, false),
        }
    }

    // ---- inside -------------------------------------------------------------------------------

    fn reset(&mut self, now: i64) {
        self.stage = RecorderStage::Closed;
        self.question = RecorderQuestion::None;
        self.has_picture = false;
        self.looks_covered = false;
        self.asking = false;
        self.clip_ready = false;
        self.clip_ms = 0;
        self.refused = None;
        self.trouble = None;
        self.plan = None;
        self.notice = None;
        self.warned = false;
        self.take_started = false;
        self.stopped_by_interruption = false;
        self.last_used = now;
        self.guard_until = i64::MIN;
    }

    /// The person's Stop (or the limit's): under a second goes back to PREVIEW, "That video was too
    /// short."
    fn stop(&mut self, now: i64, notice: Option<String>) -> Vec<RecorderEffect> {
        if self.elapsed_ms(now) < shortest_recording_ms() {
            self.stage = RecorderStage::Preview;
            let notice = self.say.get("That video was too short.").to_string();
            self.notice = Some(notice.clone());
            return vec![
                RecorderEffect::new(RecorderAction::StopAndDiscard),
                announce(&notice),
            ];
        }
        self.stop_into(now, notice, false)
    }

    /// Into REVIEW with what was recorded — the camera off, the clip being made. An interruption
    /// under a second deletes instead: there is nothing worth keeping (S4).
    fn stop_into(
        &mut self,
        now: i64,
        notice: Option<String>,
        interrupted: bool,
    ) -> Vec<RecorderEffect> {
        let elapsed = self.elapsed_ms(now);
        if interrupted && elapsed < shortest_recording_ms() {
            return self.close(
                vec![RecorderEffect::new(RecorderAction::StopAndDiscard)],
                true,
            );
        }
        self.clip_ms = elapsed;
        self.stage = RecorderStage::Review;
        self.clip_ready = false;
        self.plan = None;
        self.notice = notice.clone();
        self.stopped_by_interruption = interrupted;
        let mut effects = vec![
            RecorderEffect::new(RecorderAction::StopAndKeep),
            RecorderEffect::new(RecorderAction::CloseCamera),
        ];
        if let Some(notice) = notice {
            effects.push(announce(&notice));
        }
        effects
    }

    fn back_to_preview(&mut self, now: i64) -> Vec<RecorderEffect> {
        self.stage = RecorderStage::Opening;
        self.has_picture = false;
        self.looks_covered = false;
        self.clip_ready = false;
        self.clip_ms = 0;
        self.plan = None;
        self.notice = None;
        self.last_used = now;
        vec![
            RecorderEffect::new(RecorderAction::DiscardClip),
            RecorderEffect::new(RecorderAction::OpenCamera),
        ]
    }

    fn ask(&mut self, question: RecorderQuestion) -> Vec<RecorderEffect> {
        self.question = question;
        vec![RecorderEffect::new(RecorderAction::Ask)]
    }

    fn close(&mut self, first: Vec<RecorderEffect>, camera_on: bool) -> Vec<RecorderEffect> {
        self.stage = RecorderStage::Closed;
        self.question = RecorderQuestion::None;
        let mut effects = first;
        if camera_on {
            effects.push(RecorderEffect::new(RecorderAction::CloseCamera));
        }
        effects.push(RecorderEffect::new(RecorderAction::Closed));
        effects
    }
}

/// Said to a screen reader: text already in the reader's language.
fn announce(text: &str) -> RecorderEffect {
    RecorderEffect {
        action: RecorderAction::Announce,
        text: Some(text.to_string()),
    }
}

fn guard_ms() -> i64 {
    record::ACTIVATION_GUARD_MS as i64
}

fn delete_asks_from_ms() -> i64 {
    record::DELETE_ASKS_FROM_MS as i64
}

fn shortest_recording_ms() -> i64 {
    record::SHORTEST_RECORDING_MS as i64
}

// ---- the camera: the shell's half ---------------------------------------------------------------

/// What the open camera tells the recorder, as it happens — raised on the MAIN CONTEXT (the shell
/// hops its GStreamer-thread callbacks over), and only for the camera that is open.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CameraEvent {
    /// The live picture's first frame reached the preview sink (S3.4: Record comes alive). Once
    /// per open.
    FirstFrame,
    /// One frame's mean luma, 0 to 1, for the first `round_video_rules::BLACK_CHECK_MS` + 500 ms of
    /// the picture (S3.6's covered camera): the mean of every eighth pixel's Y of the I420 frame
    /// (`Y / 255`, limited range not rescaled — the threshold is 0.06), a few times a second at
    /// most.
    Brightness(f64),
    /// The desktop blocked the picture for privacy — the camera switched off in its privacy
    /// settings while open. Linux has no such signal today (the portal revokes access instead,
    /// which arrives as `Troubled`); kept for a desktop that adds one.
    Blocked,
    /// The camera failed, was taken, or went (S3.6, S4) — `round_video_rules::trouble` of the bus
    /// error, or `Missing` when the device monitor removes the open camera. At most once per open.
    Troubled(CameraTrouble),
}

/// Why the camera did not open: refused (and which device — `round_video_rules::refused_after`),
/// or troubled (and how).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraOpenFailure {
    Refused(RecorderRefusal),
    Troubled(CameraTrouble),
}

/// A finished take: the square MP4 on disk — the caller's from here — how long it ran by a
/// monotonic clock, and the encoding its pipeline was built with (what `round_video_rules::came_out`
/// judges it against).
#[derive(Debug, Clone, PartialEq)]
pub struct VideoTake {
    pub path: PathBuf,
    pub elapsed_ms: i64,
    pub asked: VideoEncoding,
}

/// The camera of a VIDEO MESSAGE — Windows' `VideoMessageRecorder` service, the SHELL's to
/// implement (shell-design §10.5, §10.6, §10.8). Everything runs on the GLib main context; every
/// future is cancelled by DROPPING it, which must leave nothing running and nothing on disk.
///
/// **HOWEVER IT ENDS — CLOSED, REFUSED, FAILED, OR THE WINDOW GOING — THE CAMERA AND THE MICROPHONE
/// ARE LET GO OF**: every path ends in [`close`](Self::close), which stops a take, sets the
/// pipeline to `NULL`, closes the PipeWire remote's fd and drops the preview paintable. A camera
/// light nothing can reach is a camera left on.
///
/// Logging: ids, sizes, counts, GStreamer error domains and codes only — never a file path, a
/// device name, the portal's handle token or an error's message text.
pub trait VideoMessageRecorder {
    /// Whether there is a camera at all, as cheaply as can be known without asking anyone: the
    /// camera portal's `IsCameraPresent` property (`org.freedesktop.portal.Camera`), else a
    /// `gst::DeviceMonitor` filtered on `Video/Source` finding one. Feeds
    /// `round_video_rules::available` and the video button.
    fn has_camera(&self) -> bool;

    /// What the desktop says about this app and the camera, WITHOUT prompting (S3.2 reads it before
    /// anything opens): `Allowed` once `AccessCamera` succeeded in this session (or the portal is
    /// absent and the device fallback is used); `UserPromptRequired` while it has not been asked in
    /// this session; `DeniedByUser` after the person refused in this session (the portal's response
    /// 1); `DeniedBySystem` when GNOME's privacy setting turns every camera off
    /// (`org.gnome.desktop.privacy disable-camera`); `NotDeclaredByApp` when the device fallback is
    /// refused by the app's AppArmor profile (EACCES on `/dev/video*` under a profile — a build
    /// fault); `Unknown` when the portal answered something else.
    fn camera_access(&self) -> CapabilityAccess;

    /// The same for the microphone: `Allowed` (a host app is not asked — PipeWire gives the default
    /// source), `DeniedBySystem` when GNOME's `disable-microphone` privacy setting is on.
    fn microphone_access(&self) -> CapabilityAccess;

    /// The cameras, in the order the device monitor lists them, as [`CameraChoice`]s (id = the
    /// PipeWire node's `node.name`, name = its description, panel from `api.libcamera.location`).
    /// Empty when there are none or nothing can say. Started on the portal's PipeWire remote once
    /// access was granted; on the default PipeWire before that and in the fallback.
    fn cameras(&self) -> LocalBoxFuture<'static, Vec<CameraChoice>>;

    /// Open `camera_id` for a video message — NOTHING RECORDS (the take begins at
    /// [`start`](Self::start)), and the microphone is NOT opened:
    /// 1. `AccessCamera` through the portal (the desktop asks the first time; the person's refusal
    ///    → `Err(Refused(round_video_rules::refused_after(camera_access, microphone_access)))` — a
    ///    refusal by the PERSON is never bypassed by the fallback), then `OpenPipeWireRemote` → an
    ///    fd. Where the portal is absent or answers "not supported": the default PipeWire, then
    ///    `v4l2src` (logind's uaccess).
    /// 2. The camera's modes from its caps → `round_video_rules::pick_mode(modes, None)` (one stream
    ///    feeds preview and take). None → `Err(Troubled(Unsupported))`.
    /// 3. `round_video_rules::encodes(mode's rate, has_audio = true)` → the first way whose encoder
    ///    element exists (`x264enc` for `H264Profile::High`, `openh264enc` for `Main`); none →
    ///    `Err(Troubled(Unsupported))`.
    /// 4. The preview branch: `pipewiresrc fd=… target-object=<serial> ! videoconvert !
    ///    video/x-raw,format=I420 ! tee`, `queue leaky=downstream ! videoflip
    ///    method=horizontal-flip ! gtk4paintablesink` — MIRRORED, the preview only — set PLAYING.
    ///    Raise [`CameraEvent`]s on [`events`](Self::events) from here on. The sink's `paintable` is
    ///    the recorder's round clip (shell-design §10.4's `RoundClip`) — the shell's own business.
    ///
    /// A bus error before the first frame → `Err(Troubled(round_video_rules::trouble(domain,
    /// code)))`. Dropping the future closes everything it opened.
    fn open(&self, camera_id: &str) -> LocalBoxFuture<'static, Result<(), CameraOpenFailure>>;

    /// The open camera's events (see [`CameraEvent`]).
    fn events(&self) -> &Event<CameraEvent>;

    /// Open the microphone and start the take (S3.4: the clip begins at Record), building the
    /// RECORDING branch for this take — a new one per take, torn down after it, while the preview
    /// runs on:
    /// `tee. ! queue ! videocrop` (`round_video_rules::centre_square(mode)` →
    /// `SquareCrop::videocrop`) `! videoscale ! videorate ! video/x-raw,format=I420,width=480,
    /// height=480,framerate=<asked.frame_rate_numerator>/<asked.frame_rate_denominator>` (the
    /// camera's own rate up to 30 — never raised) `! x264enc bitrate=500 key-int-max=<asked.
    /// keyframe_spacing> speed-preset=veryfast tune=zerolatency ! video/x-h264,profile=high` (or
    /// `openh264enc bitrate=500000 gop-size=… ! video/x-h264,profile=main`) `! h264parse ! mux.`;
    /// `pulsesrc ! audioconvert ! audioresample ! audio/x-raw,rate=<asked.audio.sample_rate>,
    /// channels=1 ! avenc_aac bitrate=<asked.audio.bitrate>` (else `voaacenc`) `! aacparse ! mux.`;
    /// `mp4mux name=mux faststart=true ! filesink` at a new file under
    /// `$XDG_CACHE_HOME/family-connect/tmp/round-<uuid>.mp4`. NOT MIRRORED, upright. A monotonic
    /// clock starts when the first buffer reaches the muxer. `true` once both are flowing; `false`
    /// when it would not start (the microphone refused, the disk full) — the camera stays on and
    /// nothing is left on disk.
    fn start(&self) -> LocalBoxFuture<'static, bool>;

    /// Stop the take and hand it over: EOS into the recording branch only, wait for the muxer's EOS
    /// (the file final, `moov` first), tear the branch down; the camera stays on (a Delete that goes
    /// back to PREVIEW keeps it; `CloseCamera` turns it off). `None` when nothing could be finished
    /// (no take running, an empty file, an error) — and then nothing is left on disk.
    fn stop(&self) -> LocalBoxFuture<'static, Option<VideoTake>>;

    /// Stop the take and throw it away (the file deleted); the camera stays on.
    fn discard(&self) -> LocalBoxFuture<'static, ()>;

    /// Let go of everything: a take (thrown away unless [`stop`](Self::stop) handed it over), the
    /// pipeline (`NULL`), the PipeWire fd, the preview. Idempotent and safe from every path.
    fn close(&self) -> LocalBoxFuture<'static, ()>;

    /// Read a finished take back (`gst_pbutils::Discoverer` on the file, and
    /// `faststart::is_moov_first` on its bytes) as a [`TakeReading`]; `None` when it will not read.
    fn read_back(&self, take: &VideoTake) -> LocalBoxFuture<'static, Option<TakeReading>>;

    /// The clip's poster as the protocol says (docs/protocol.md, "Video messages"): a frame of the
    /// SQUARE at 0.5 s, else 0 s, else 2 s, as a baseline JPEG at most 600 a side — so 480 × 480 —
    /// no EXIF. `None` when no frame could be read; the clip then goes without one.
    fn poster(&self, take: &VideoTake) -> LocalBoxFuture<'static, Option<Vec<u8>>>;

    /// A take or a clip nobody will send: gone from the disk. Takes left by an earlier run (a crash
    /// mid-take) are swept when the recorder next opens.
    fn delete(&self, take: &VideoTake);
}
