//! A voice note recorded here (`VoiceNotes.cs`; docs/protocol.md, "Audio" and "A browser is a
//! client too"): MP4 with AAC in it — never WebM, which the server refuses — five minutes at most,
//! sent from the Send slot, or staged when the member stops it to listen or add words, so a
//! recording made by accident can still be thrown away (docs/audio-video-messages-2026-10-04.md,
//! S2.5).
//!
//! A SECOND IS THE FLOOR PEOPLE SEE (S1.1 and decision 15): nothing shorter is ever staged, sent or
//! kept. The 1024 bytes stay as what they always were — a recording with nothing in it — beside it.
//! The floors are `fc_text::record`'s S1.1 constants, read from there rather than copied.
//!
//! Beside the rules: [`QuietWhileRecording`] (no app sound while recording, the viewer's video
//! included), [`RecordingStart`] (the moments between the press and a running recorder), and the
//! platform seam the shell implements with GStreamer — [`VoiceRecorder`] and [`VoiceRecording`] —
//! with the one piece of the window's glue that awaits it, [`start_recording`], and the balanced
//! screen-on hold [`KeepAwake`] over the shell's [`Inhibitor`] (shell-design §10.2, §10.8, §10.9).
//!
//! THE MICROPHONE ON LINUX: a host app is never asked for it — PipeWire hands the default source to
//! `pulsesrc`, and GNOME shows the microphone in use — so the shell reports
//! [`CapabilityAccess::Allowed`] and a microphone that will not open is
//! [`RecordingFailure::CouldNotStart`] ("Couldn't start recording."). The refusal rules stay whole
//! for a sandboxed build, whose portal answer maps onto [`CapabilityAccess`]; the Settings page
//! Windows opens (`ms-settings:privacy-microphone`) is GNOME Settings'
//! [`MICROPHONE_SETTINGS_PANEL`] here.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use fc_core::rt::{Clock, Event, LocalBoxFuture};
use fc_core::text::string_catalog::StringCatalog;
use fc_text::record;

use crate::composer_staging;
use crate::media_encoding::{self, AacEncoding};
use crate::media_outbox::StagedMedia;
use crate::voice_look::VoiceMeter;

/// What the platform says about this app's use of a capture device — Windows'
/// `AppCapability.CheckAccess` answer, read on the shell's side and named here so the decisions it
/// feeds can be tested anywhere. On Linux the shell maps the camera portal's answer onto it; a host
/// app's microphone is never asked for (PipeWire gives the default source), so it is `Allowed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CapabilityAccess {
    Allowed,
    /// Never asked: the platform asks the person the next time the device is opened.
    UserPromptRequired,
    /// The person turned this app off in the desktop's settings.
    DeniedByUser,
    /// The system itself: the device switched off for every app, or a policy.
    DeniedBySystem,
    /// The app does not ask for it — a build fault, which no switch in settings can mend.
    NotDeclaredByApp,
    /// The platform could not be asked.
    Unknown,
}

/// The longest a voice note may run; the recording stops itself there (S1.1's voice cap).
pub const LONGEST: Duration = Duration::from_millis(record::VOICE_CAP_MS);

/// From here the clock turns orange and "30 seconds left" is shown beside it and announced (S2.5):
/// 4:30.
pub const WARN_FROM: Duration = Duration::from_millis(record::VOICE_WARNING_MS);

/// With a screen reader running, how long the microphone waits after "Recording" is announced (S6:
/// a fixed second where the platform cannot say when its speech ended), so the app's own voice does
/// not open every note.
pub const SCREEN_READER_LEAD_IN: Duration = Duration::from_secs(1);

/// The shortest recording that is anything at all (S1.1's "shortest recording"): under it, nothing
/// is kept.
pub const SHORTEST: Duration = Duration::from_millis(record::SHORTEST_RECORDING_MS);

/// From this long, deleting a recording asks first (S1.1's "delete asks"): it cannot be made again.
pub const DELETE_ASKS_FROM: Duration = Duration::from_millis(record::DELETE_ASKS_FROM_MS);

/// A recording this small is nothing — ios `AudioRecorder`'s 1024 bytes or less.
pub const NOTHING_AT_OR_BELOW_BYTES: usize = 1024;

/// What a voice note is uploaded as: the container the server checks, and every family phone plays.
pub const MIME: &str = "audio/mp4";

/// Where the person can give this app the microphone back (S2.2) — on Windows the page
/// `ms-settings:privacy-microphone`; on Linux GNOME Settings' panel of this name, which the shell
/// opens by activating `org.gnome.Settings`'s `launch-panel` action with `("microphone", [])` over
/// the session bus — and offers nothing where that service is absent (unverified — real desktop).
pub const MICROPHONE_SETTINGS_PANEL: &str = "microphone";

/// Whether a recording has run as long as a voice note may.
pub fn is_done(elapsed: Duration) -> bool {
    elapsed >= LONGEST
}

/// Whether a recording has reached its last thirty seconds: words as well as colour (WCAG 1.4.1).
pub fn warns(elapsed: Duration) -> bool {
    elapsed >= WARN_FROM
}

/// How long the microphone waits after "Recording" is said — a second when a screen reader runs,
/// else none.
pub fn lead_in(screen_reader: bool) -> Duration {
    if screen_reader {
        SCREEN_READER_LEAD_IN
    } else {
        Duration::ZERO
    }
}

/// Whether deleting a recording this long asks "Delete this recording?" first.
pub fn asks_before_deleting(duration_ms: i32) -> bool {
    i64::from(duration_ms) >= DELETE_ASKS_FROM.as_millis() as i64
}

/// Whether a staged item is a voice note recorded HERE rather than a sound file picked from disk:
/// both are `kind=audio`, and a recording is the one with no name — its duration is its identity
/// ([`staged`]).
pub fn is_recorded(item: &StagedMedia) -> bool {
    item.kind == "audio" && item.name.is_none()
}

/// The recording as a staged attachment — its duration its identity, and no name — or `None` when
/// it is too short to be anything: under a second, or no more than 1024 bytes. The composer says
/// which.
pub fn staged(bytes: Vec<u8>, elapsed: Duration) -> Option<StagedMedia> {
    if bytes.len() <= NOTHING_AT_OR_BELOW_BYTES || elapsed < SHORTEST {
        return None;
    }
    Some(StagedMedia {
        duration_ms: Some(duration_ms(elapsed)),
        ..StagedMedia::new("audio", MIME, bytes)
    })
}

/// A recording's length as the attachment carries it: whole milliseconds, half rounded away from
/// zero (Rust's `round`, the C#'s `MidpointRounding.AwayFromZero`), at most five minutes. Counted
/// from the nanoseconds, so 12 344.5 ms is exactly that before it is rounded.
pub fn duration_ms(elapsed: Duration) -> i32 {
    let milliseconds = elapsed.as_nanos() as f64 / 1_000_000.0;
    milliseconds.clamp(0.0, LONGEST.as_millis() as f64).round() as i32
}

/// Why a recording cannot start right now, or `None` when it can — in the order the Send slot's own
/// rows put them (S1.3: an edit, a call, an attachment on its way, a voice message that was not
/// sent), and then the strip's cap.
///
/// `busy`: files are being prepared, or a send's files are being written down. `not_sent`: this
/// chat holds a voice message that was not sent (S2.8). `full`: the strip holds as many items as one
/// message can.
pub fn refusal(
    editing: bool,
    call: bool,
    busy: bool,
    not_sent: bool,
    full: bool,
    say: &dyn StringCatalog,
) -> Option<String> {
    if editing {
        Some(
            say.get("Finish editing before attaching something.")
                .to_string(),
        )
    } else if call {
        Some(
            say.get("You can record a message after the call.")
                .to_string(),
        )
    } else if busy {
        Some(
            say.get("Wait until the current attachment is done.")
                .to_string(),
        )
    } else if not_sent {
        Some(
            say.get("Send or delete the voice message that wasn't sent first.")
                .to_string(),
        )
    } else if full {
        Some(composer_staging::cap_sentence(say))
    } else {
        None
    }
}

/// What to say when the microphone was refused, and whether to offer the desktop's settings
/// ([`MICROPHONE_SETTINGS_PANEL`]; S2.2: the iOS sentence with [Open Settings]). The page is offered
/// only where a switch on it can help — the person's own, or the system's — and when the platform
/// could not say, because the refusal was a refusal. A prompt closed without an answer says nothing:
/// nothing was taken, and asking again is one click. (A host app on Linux never sees anything but
/// `Allowed`, which says "Couldn't start recording." — see the module's notes.)
pub fn microphone_refusal(
    access: CapabilityAccess,
    say: &dyn StringCatalog,
) -> (Option<String>, bool) {
    match access {
        CapabilityAccess::DeniedByUser
        | CapabilityAccess::DeniedBySystem
        | CapabilityAccess::Unknown => (
            Some(
                say.get("Family needs permission to use your microphone. Turn it on in Settings.")
                    .to_string(),
            ),
            true,
        ),
        CapabilityAccess::UserPromptRequired => (None, false),
        CapabilityAccess::Allowed | CapabilityAccess::NotDeclaredByApp => (
            Some(say.get("Couldn't start recording.").to_string()),
            false,
        ),
    }
}

/// The recording row's clock, "0:42" (S2.4, S2.9): whole seconds gone by, as a stopwatch counts
/// them — so it reads 5:00 only when five minutes have really run, and the row stops there.
pub fn clock(elapsed: Duration) -> String {
    fc_text::media::time_label(elapsed.as_secs_f64().floor())
}

/// Whether a sentence the composer shows is also SAID — its polite live region raised
/// (`gtk::Accessible::announce`). Never while a recording runs: the app's own speech stays out of
/// the note (S6), so only the recording's own few announcements are made then. The sentence is on
/// the screen either way, and a dimmed control carries it as its accessible description.
pub fn notice_said(recording: bool) -> bool {
    !recording
}

/// "0:12 / 0:42": a staged or not-sent note while it plays (S2.7) — where it is, and how long it is.
pub fn playing_label(elapsed_seconds: f64, total_seconds: f64) -> String {
    format!(
        "{} / {}",
        fc_text::media::time_label(elapsed_seconds.min(total_seconds)),
        fc_text::media::time_label(total_seconds)
    )
}

/// How long a recording is, in seconds, from the attachment — so the scrubber is right before a
/// byte arrives — and never zero, which would give the scrubber no length at all (ios
/// `AudioPlayerView.total`).
pub fn total_seconds(duration_ms: Option<i32>) -> f64 {
    (f64::from(duration_ms.unwrap_or(0)) / 1000.0).max(0.1)
}

/// Whether Play starts again from the beginning: at the end, or within a fifth of a second of it —
/// without which the button would do nothing on a recording that has played through.
pub fn replays_from_start(elapsed_seconds: f64, total_seconds: f64) -> bool {
    elapsed_seconds >= total_seconds - 0.2
}

/// The viewer's video while something records (S1.7: no app sound plays while recording). Its
/// controls go and it says why, and its player is muted — so nothing a key or the system's media
/// buttons start can be heard in the note — and as the recording ends the player is left as muted,
/// or not, as it was before.
#[derive(Debug, Default)]
pub struct QuietWhileRecording {
    muted_before: Option<bool>,
}

impl QuietWhileRecording {
    pub fn new() -> Self {
        QuietWhileRecording::default()
    }

    /// Whether the player is to be muted now: always while a recording runs — remembering, the
    /// first time, whether it was muted already — and once it has ended, exactly as it was before
    /// it began. `muted_now`: whether the player is muted as this is asked.
    pub fn muted(&mut self, recording: bool, muted_now: bool) -> bool {
        if recording {
            self.muted_before.get_or_insert(muted_now);
            return true;
        }
        self.muted_before.take().unwrap_or(muted_now)
    }
}

/// A recording being started (S1.7 and S4): from the press until the recorder runs — or the start
/// gives up — there is no recorder yet, and that can be long: a screen reader's one-second lead-in,
/// a microphone slow to open.
///
/// NO APP SOUND FROM THE START ON (S1.7): a play pressed while the microphone is being opened would
/// run on into the note, so playback is refused while a start is under way as it is while a
/// recording runs ([`quiet`](Self::quiet)).
///
/// AN INTERRUPTION DURING THE START IS REMEMBERED (S4): a lock, sleep, a call, the window hidden or
/// closed, leaving the chat or a sign-out finds no recorder to stop — so the start notes it, and
/// lets go of the microphone it is granted rather than recording behind a lock screen.
///
/// Its state is in cells: the start awaits the recorder while the window's interruption handlers
/// note what happened, both through `&self`.
#[derive(Debug, Default)]
pub struct RecordingStart {
    starting: Cell<bool>,
    interrupted: Cell<bool>,
}

impl RecordingStart {
    pub fn new() -> Self {
        RecordingStart::default()
    }

    /// Whether a start is under way — so a second press starts nothing more.
    pub fn starting(&self) -> bool {
        self.starting.get()
    }

    /// Whether something interrupted the start under way.
    pub fn was_interrupted(&self) -> bool {
        self.interrupted.get()
    }

    /// A start begins: nothing heard before it counts against it, since only a start under way
    /// notes anything.
    pub fn begin(&self) {
        self.starting.set(true);
    }

    /// Something other than the person ends what is recorded: noted against the start under way,
    /// if there is one.
    pub fn interrupted(&self) {
        if self.starting.get() {
            self.interrupted.set(true);
        }
    }

    /// The start is over — the recorder running, or given up.
    pub fn end(&self) {
        self.starting.set(false);
        self.interrupted.set(false);
    }

    /// Whether app sound is refused now: while a recording runs or one is being started.
    pub fn quiet(&self, recording: bool) -> bool {
        recording || self.starting.get()
    }
}

// --- the platform: the microphone, through GStreamer ---------------------------------------------

/// Why a recording did not start, so the composer can say it rather than leave a dead button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecordingFailure {
    /// The platform refused the microphone to this app — then [`VoiceRecorder::microphone_access`]
    /// and [`microphone_refusal`] say what to tell the person. A host app on Linux never sees it.
    MicrophoneDenied,
    /// Anything else: no source, the pipeline would not start, no AAC encoder.
    CouldNotStart,
}

/// What a stopped recording handed over: how long it ran — ALWAYS there, because it decides whether
/// a recording that could not be read back was worth saying so (`not_sent::ended`) — and its bytes,
/// `None` when nothing could be read back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recorded {
    pub bytes: Option<Vec<u8>>,
    pub elapsed: Duration,
}

/// The microphone, as the shell opens it (`VoiceRecorder.cs`; shell-design §10.2). Everything runs
/// on the GLib main context; GStreamer's own threads only post to the bus, which the shell watches
/// there.
pub trait VoiceRecorder {
    /// Open the default microphone and start recording a voice note, answering the running
    /// recording or why there is none.
    ///
    /// THE PIPELINE: `pulsesrc` (the default source; PipeWire's Pulse server) `! audioconvert !
    /// audioresample ! audio/x-raw,rate=<sample_rate>,channels=<channels> ! tee name=t`, `t. ! queue
    /// ! avenc_aac bitrate=<bitrate> ! aacparse ! mp4mux faststart=true ! filesink` (a temporary
    /// under `$XDG_CACHE_HOME/family-connect/tmp/`, mode 0600), and `t. ! queue ! level
    /// interval=50000000 post-messages=true ! fakesink` for the live peaks — ONE microphone open,
    /// never two. The numbers come from `encodings`, which is [`media_encoding::voice_note_attempts`]:
    /// on Linux one encoding, AAC-LC mono 64 000 at 44.1 kHz. For each encoding in order the shell
    /// tries `avenc_aac`, then `voaacenc bitrate=<bitrate>`; a start that fails with every one is
    /// [`RecordingFailure::CouldNotStart`].
    ///
    /// `speak_first`, when given, is awaited ONCE after the microphone has opened (the pipeline in
    /// PAUSED, the source's device acquired) and BEFORE anything is recorded or the clock starts:
    /// with a screen reader running "Recording" is announced and a second passes (S6), so the
    /// app's own voice is not the first thing in the note. Then the pipeline goes to PLAYING and the
    /// recording's monotonic clock starts.
    ///
    /// Errors: a bus `Error` before PLAYING, a missing element or a refused state change →
    /// `CouldNotStart` (logged as the element's factory name and the error's domain and code only —
    /// never its message, which can name the device). Dropping the future before it answers lets
    /// go of whatever was opened (the pipeline to `Null`, the temporary deleted).
    fn start(
        &self,
        encodings: Vec<AacEncoding>,
        speak_first: Option<LocalBoxFuture<'static, ()>>,
    ) -> LocalBoxFuture<'static, Result<Rc<dyn VoiceRecording>, RecordingFailure>>;

    /// What the platform says about this app and the microphone, asked after a
    /// [`RecordingFailure::MicrophoneDenied`] to know whether the settings are worth offering. A
    /// host app answers [`CapabilityAccess::Allowed`].
    fn microphone_access(&self) -> CapabilityAccess;
}

/// A voice note in progress. HOWEVER IT ENDS — stopped, abandoned, dropped, the window going — THE
/// MICROPHONE IS LET GO OF: dropping the last handle without [`stop`](Self::stop) is
/// [`abandon`](Self::abandon).
///
/// It is also the row's live meter: `voice_look::VoiceMeter` (its `take_peak_dbfs`) is a
/// supertrait, answered from the `level` branch of the same pipeline — the loudest the microphone
/// has been since the last ask, in dBFS (the `level` element's `peak`, the largest over its
/// channels), `None` when nothing was heard since or the meter branch is not running. Drawn and
/// forgotten: the note's own waveform is measured from the finished file (`fc_text::waveform`),
/// never from this.
pub trait VoiceRecording: VoiceMeter {
    /// How long it has been recording, by a MONOTONIC clock that started when the pipeline reached
    /// PLAYING (S2.9: a wall clock moved while somebody talks would move the length the note is
    /// kept, sent and asked about with). Frozen once stopped or abandoned.
    fn elapsed(&self) -> Duration;

    /// Raised AT MOST ONCE, on the main context, when the recording stops on its own: a bus `Error`
    /// from the source or the encoder (the microphone pulled out or taken, the disk full). The
    /// window ends the recording with `RecordingEnd::RecorderFailed` and keeps what can be read
    /// back (S4). Never raised after `stop` or `abandon` began.
    fn failed(&self) -> &Event<()>;

    /// Stop and hand over what was recorded: EOS sent into the pipeline, the bus's `Eos` awaited
    /// (so `mp4mux` has written `moov` before `mdat`), the pipeline to `Null`, the file read whole
    /// and deleted. `bytes` is `None` when nothing could be read back (no file, an empty one, a bus
    /// error instead of EOS); `elapsed` is always the clock at the moment of the call. The result
    /// is read back with the discoverer and a mismatch with what was asked
    /// (`media_encoding::audio_rate_came_out`, AAC in the channels asked) is only LOGGED — a
    /// recording cannot be sent as an original instead. A second call answers `bytes: None`.
    fn stop(&self) -> LocalBoxFuture<'static, Recorded>;

    /// Let go now and keep nothing: the pipeline to `Null`, the temporary deleted. Idempotent.
    fn abandon(&self);
}

/// How [`start_recording`] ended.
pub enum Started {
    /// The recorder runs. `spoken`: "Recording" was already announced during the lead-in, so the
    /// window does not say it again.
    Recording {
        recording: Rc<dyn VoiceRecording>,
        spoken: bool,
    },
    /// No recorder: why (the window says "Couldn't start recording.", or asks
    /// [`VoiceRecorder::microphone_access`] for a denial).
    Failed(RecordingFailure),
    /// Granted after something interrupted the start, or after the window no longer wanted it (the
    /// chat left, a call or an edit begun, the window hidden): the microphone was let go of (S4).
    LetGo,
}

/// The window's start of a voice recording, from the press to a running recorder (S1.7, S4, S6) —
/// the part of `StartRecordingAsync` that awaits the platform. `start` notes the start for its whole
/// length — so a second press starts nothing and playback is refused — and is ended however this
/// ends, the future dropped included. With a screen reader running, `say_recording` ("Recording",
/// announced) is called once the microphone has opened and the recording waits a second on `clock`
/// before it begins. `still_wanted` is asked once the recorder answers: the window's own reasons to
/// let go (the chat left, another recording, a call, an edit, the window away, the screen saver).
///
/// The window refuses a start first ([`refusal`]), quiets what plays, arms the slot's guard, and on
/// [`Started::Recording`] holds the screen on ([`KeepAwake`]), subscribes to
/// [`VoiceRecording::failed`] and shows the recording row.
pub async fn start_recording(
    recorder: &dyn VoiceRecorder,
    start: &RecordingStart,
    screen_reader: bool,
    clock: Rc<dyn Clock>,
    say_recording: Rc<dyn Fn()>,
    still_wanted: impl Fn() -> bool,
) -> Started {
    struct Ending<'a>(&'a RecordingStart);
    impl Drop for Ending<'_> {
        fn drop(&mut self) {
            self.0.end();
        }
    }

    start.begin();
    let _ending = Ending(start);
    let lead = lead_in(screen_reader);
    let spoken = Rc::new(Cell::new(false));
    let speak_first: Option<LocalBoxFuture<'static, ()>> = if lead > Duration::ZERO {
        let spoken = Rc::clone(&spoken);
        Some(Box::pin(async move {
            spoken.set(true);
            say_recording();
            clock.sleep(lead).await;
        }))
    } else {
        None
    };
    match recorder
        .start(media_encoding::voice_note_attempts(), speak_first)
        .await
    {
        Err(failure) => Started::Failed(failure),
        Ok(recording) => {
            if start.was_interrupted() || !still_wanted() {
                recording.abandon();
                Started::LetGo
            } else {
                Started::Recording {
                    recording,
                    spoken: spoken.get(),
                }
            }
        }
    }
}

/// The screen-on request the shell makes (shell-design §10.9): `gtk::Application::inhibit(
/// Some(&window), ApplicationInhibitFlags::IDLE, Some(reason))` — an idle timer that locked the
/// session halfway through a long story would stop the recording (S1.7).
pub trait Inhibitor {
    /// Ask; answers the cookie, or 0 when no inhibitor is available (measured in the container,
    /// shell-design §1.6 — the shell logs that once).
    fn inhibit(&self) -> u32;

    /// Give back the request `cookie` answered.
    fn uninhibit(&self, cookie: u32);
}

/// The screen kept on while somebody records (`KeepAwake.cs`), BALANCED: a release with nothing
/// held does nothing, and a second hold is the same hold. A request the desktop could not grant
/// (cookie 0) is not held, and the next hold asks again.
pub struct KeepAwake {
    inhibitor: Rc<dyn Inhibitor>,
    cookie: RefCell<Option<u32>>,
}

impl KeepAwake {
    pub fn new(inhibitor: Rc<dyn Inhibitor>) -> Self {
        KeepAwake {
            inhibitor,
            cookie: RefCell::new(None),
        }
    }

    /// Keep the screen on until [`release`](Self::release). Answers whether it is held.
    pub fn hold(&self) -> bool {
        if self.cookie.borrow().is_some() {
            return true;
        }
        match self.inhibitor.inhibit() {
            0 => false,
            cookie => {
                *self.cookie.borrow_mut() = Some(cookie);
                true
            }
        }
    }

    /// Let the screen sleep again; nothing when nothing is held.
    pub fn release(&self) {
        let held = self.cookie.borrow_mut().take();
        if let Some(cookie) = held {
            self.inhibitor.uninhibit(cookie);
        }
    }

    pub fn is_held(&self) -> bool {
        self.cookie.borrow().is_some()
    }
}

impl Drop for KeepAwake {
    fn drop(&mut self) {
        self.release();
    }
}
