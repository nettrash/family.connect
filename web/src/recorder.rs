//! A voice note, recorded in the browser (docs/protocol.md, "A browser is a
//! client too"), to the profile every client records one to ("Preparing
//! media before upload": M4A, AAC-LC, mono, 64 000 bit/s) wherever this
//! browser can make that — and never the WebM most recorders default to,
//! which the server refuses. Three ways, best first:
//!
//! 1. Raw samples off the microphone, encoded by the browser's own AAC
//!    encoder and written as M4A here (crate::encode). This is the only way
//!    that is the profile to the letter: one channel, the rate asked for, a
//!    plain file with its index first and its length in it.
//! 2. The browser's recorder writing MP4 itself, asked for 64 000 bit/s.
//!    What it writes is its own business — a fragmented file, and as many
//!    channels as the microphone has — so it is second.
//! 3. Raw samples made into a WAV (fc_text::wav), mono at 16 kHz: for a
//!    browser that encodes no AAC at all.
//!
//! Which one is asked of the BROWSER (`isConfigSupported`, `isTypeSupported`)
//! when the recording starts, never guessed from its name. Only the first
//! is the voice-note row. The protocol names no fallback for a client that
//! cannot encode AAC; the other two are what a note went as before there
//! was a row, kept because a browser that cannot make the row must still
//! be able to send a voice note.
//!
//! The raw samples are taken OFF the page's own thread (an `AudioWorklet`),
//! because that thread is busy exactly when somebody is talking: a timeline
//! rendering, a sync landing, a video being transcoded in the same tab. A
//! tap on the page's thread (`ScriptProcessorNode`, which this used) loses
//! whatever arrives while the page is busy, and the note has holes in it;
//! it is kept only for a browser with no worklet.
//!
//! Five minutes at most; the conversation stops it there. What comes out is
//! sent from the recorder, or staged when the member stops it to listen or
//! add words (docs/protocol.md, "A browser is a client too") — which of the
//! two is the Send slot's rule (fc_text::record, and views::voice here).
//!
//! A recording keeps the screen on for as long as it lives (crate::awake),
//! says when its microphone stops being one ([`Recording::on_lost`]), and is
//! counted while it lives ([`in_progress`]) — what a tab closing or a sign-out
//! would lose (the plan for #79, docs/audio-video-messages-2026-10-04.md,
//! S1.7, S2.8, S4).
//!
//! It is timed on a MONOTONIC clock ([`now_ms`]), not the wall clock a
//! machine setting its time would move, and where the samples pass through
//! this page — the first and third ways — it says how loud they are
//! ([`Recording::meter`]): the level meter and the silence warning of the
//! plan's S2.9. The browser's own recorder hands over nothing until it
//! stops, so it has neither.

use fc_text::i18n::t;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use fc_text::media_plan::VOICE_NOTE_BITRATE;
use fc_text::{media, record, wav};
use futures::channel::oneshot;
use futures::future::select;
use gloo_timers::future::TimeoutFuture;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    AudioContext, AudioContextOptions, AudioContextState, AudioProcessingEvent, AudioWorkletNode,
    AudioWorkletNodeOptions, Blob, BlobEvent, BlobPropertyBag, ChannelCountMode, MediaRecorder,
    MediaRecorderOptions, MediaStream, MediaStreamConstraints, MessageEvent, ScriptProcessorNode,
    Url,
};

use crate::awake::ScreenAwake;
use crate::{encode, webcodecs};

thread_local! {
    /// How many recordings are alive in this tab: a microphone open, or a
    /// note it heard still being finished.
    static RUNNING: Cell<u32> = const { Cell::new(0) };
}

/// Whether a voice note is being recorded — or finished — anywhere in this
/// tab: something a tab closing or a sign-out would lose (the plan's S2.8).
pub fn in_progress() -> bool {
    RUNNING.get() > 0
}

#[cfg(test)]
thread_local! {
    /// How far a test has moved [`now_ms`] on (`testing::ClockAhead`).
    static AHEAD: Cell<f64> = const { Cell::new(0.0) };
}

/// Now, in milliseconds, on the clock a voice message is timed by: the
/// page's monotonic one (`performance.now()`), which nothing but time moves
/// — the timer the person sees, the five-minute limit, the activation guard
/// (the plan's S2.9, S1.1). The wall clock only where a page has no
/// `performance`, which none this runs in lacks.
pub fn now_ms() -> f64 {
    let now = web_sys::window()
        .and_then(|window| window.performance())
        .map(|performance| performance.now())
        .unwrap_or_else(js_sys::Date::now);
    #[cfg(test)]
    let now = now + AHEAD.get();
    now
}

/// Whether this page can record sound at all — the slot's "can record"
/// (S1.2): `navigator.mediaDevices` exists, which it does only in a secure
/// context. Looked up rather than bound: an http:// page has no such
/// property, and a binding would hand back `undefined` as if it were one.
pub fn can_record() -> bool {
    web_sys::window()
        .map(|window| window.navigator())
        .and_then(|navigator| {
            js_sys::Reflect::get(&navigator, &JsValue::from_str("mediaDevices")).ok()
        })
        .is_some_and(|devices| !devices.is_undefined() && !devices.is_null())
}

/// How loud the microphone has been, as a tap hears it: the loudest sample
/// since the meter last looked, and whether any sample since the recording
/// began rose above digital silence — a magnitude of 0.001, −60 dBFS
/// (fc_text::record::SILENCE_SAMPLE_MAGNITUDE): a muted microphone, not a
/// quiet room (the plan's S1.1, S2.9).
#[derive(Default)]
struct Level {
    peak: Cell<f32>,
    heard: Cell<bool>,
}

impl Level {
    fn hear(&self, block: &[f32]) {
        let loudest = block
            .iter()
            .fold(0f32, |peak, sample| peak.max(sample.abs()));
        if loudest > self.peak.get() {
            self.peak.set(loudest);
        }
        // Compared as the samples are: in single precision, where 0.001 is
        // the silence level itself and not a hair above it.
        if loudest > record::SILENCE_SAMPLE_MAGNITUDE as f32 {
            self.heard.set(true);
        }
    }
}

/// A recording's level, for the meter that draws it (S2.9).
#[derive(Clone)]
pub struct Meter(Rc<Level>);

impl Meter {
    /// The loudest sample since the last look, from 0 to 1 — and the next
    /// look starts again from silence.
    pub fn take_peak(&self) -> f32 {
        self.0.peak.replace(0.0)
    }

    /// Whether anything louder than digital silence has been heard since
    /// the recording began.
    pub fn heard(&self) -> bool {
        self.0.heard.get()
    }
}

/// The same meter, not an equal one.
impl PartialEq for Meter {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl std::fmt::Debug for Meter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Meter(heard: {})", self.heard())
    }
}

/// How many of the meter's five bars a peak lights: one at each of −50,
/// −40, −30, −20 and −10 dBFS it reaches — the PEAK level, the measure the
/// silence check reads, so the bars light alike on every client (S2.9).
pub fn lit_bars(peak: f32) -> usize {
    if peak.is_nan() || peak <= 0.0 {
        return 0;
    }
    let dbfs = 20.0 * f64::from(peak).log10();
    [-50.0, -40.0, -30.0, -20.0, -10.0]
        .iter()
        .filter(|&&threshold| dbfs >= threshold)
        .count()
}

#[cfg(test)]
thread_local! {
    /// The stream of the recording started last — the one handle a test has
    /// on the microphone of a recording a view holds out of its reach.
    static LAST_STREAM: RefCell<Option<MediaStream>> = const { RefCell::new(None) };
}

/// The microphone of the recording started last (see `LAST_STREAM`).
#[cfg(test)]
pub fn last_stream() -> Option<MediaStream> {
    LAST_STREAM.with(|last| Clone::clone(&*last.borrow()))
}

#[cfg(test)]
pub mod testing {
    /// [`super::now_ms`] moved on by however much a test asks, for as long
    /// as this lives — four and a half minutes of a recording, in a test
    /// that takes a second. Everything a voice message times is timed by
    /// that clock, so everything moves together: the recording's length, the
    /// row's timer, the activation guard.
    pub struct ClockAhead(f64);

    impl ClockAhead {
        pub fn by(ms: f64) -> ClockAhead {
            super::AHEAD.set(super::AHEAD.get() + ms);
            ClockAhead(ms)
        }

        /// Further on still, undone with the rest when this goes.
        pub fn more(&mut self, ms: f64) {
            super::AHEAD.set(super::AHEAD.get() + ms);
            self.0 += ms;
        }
    }

    impl Drop for ClockAhead {
        fn drop(&mut self) {
            super::AHEAD.set(super::AHEAD.get() - self.0);
        }
    }

    thread_local! {
        static STOPS_HELD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    /// A stopped recording is not finished into a note for as long as this
    /// lives — the seconds a long note takes to encode, made as long as a
    /// test needs, so what the composer does meanwhile can be asked of it.
    pub struct StopsHeld;

    impl StopsHeld {
        pub fn new() -> StopsHeld {
            STOPS_HELD.set(true);
            StopsHeld
        }
    }

    impl Drop for StopsHeld {
        fn drop(&mut self) {
            STOPS_HELD.set(false);
        }
    }

    pub(super) async fn while_stops_held() {
        while STOPS_HELD.get() {
            gloo_timers::future::TimeoutFuture::new(10).await;
        }
    }
}

/// Why a microphone stopped being this recording's, as the browser tells it
/// on the track (the Media Capture spec's `ended` and `mute`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lost {
    /// The device went away or the permission was taken back: it will not
    /// come back. The plan's S4 "the recorder fails": "The recording stopped
    /// unexpectedly."
    Ended,
    /// The system took it for something else for a while — a phone call, an
    /// assistant, another app using it exclusively: S4's "Siri, an alarm or
    /// another app takes the microphone".
    Muted,
}

/// A recording on its way from the pane that held it to what finishes it —
/// stopped by something other than the person, it is kept rather than lost
/// (the plan's S2.8). One owner at a time: whoever takes it out has it, and
/// one nobody takes is dropped with the last handle, the microphone with it.
#[derive(Clone)]
pub struct Handover(Rc<RefCell<Option<Recording>>>);

impl Handover {
    pub fn of(recording: Recording) -> Handover {
        Handover(Rc::new(RefCell::new(Some(recording))))
    }

    pub fn take(&self) -> Option<Recording> {
        self.0.borrow_mut().take()
    }
}

/// The same handover, not an equal one: a recording is not a value.
impl PartialEq for Handover {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl std::fmt::Debug for Handover {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let held = if self.0.borrow().is_some() {
            "a recording"
        } else {
            "nothing"
        };
        write!(f, "Handover({held})")
    }
}

/// Why recording did not start, with what to say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    MicrophoneDenied,
    CouldNotStart,
}

impl Failure {
    pub fn message(self) -> &'static str {
        match self {
            Failure::MicrophoneDenied => { t("Family needs permission to use your microphone. Allow it in your browser's settings for this site.") }
            Failure::CouldNotStart => t("Couldn't start recording."),
        }
    }
}

/// The MP4 types this browser's recorder may write, most wanted first. The
/// plain one is a second choice only: Chrome without an AAC encoder takes
/// it and writes OPUS into the MP4, which passes the server's container
/// check and plays on no phone — so what it actually chose is read back
/// after it starts, and Opus goes to the WAV path instead.
const MP4_TYPES: [&str; 2] = ["audio/mp4;codecs=mp4a.40.2", "audio/mp4"];

/// What the microphone has said so far: a block of samples per callback,
/// kept in the BROWSER's memory, not this module's. Five minutes at 48 kHz
/// is 58 MB of floats, and memory this module grows by is never given back
/// for as long as the tab lives; the browser's is collected with the
/// recording.
type Samples = Rc<js_sys::Array>;

/// The rates the profile records a voice note at.
const VOICE_RATES: [u32; 2] = [44_100, 48_000];

/// What a finished recording is.
pub struct Recorded {
    pub blob: Blob,
    /// What the upload declares — `audio/mp4` or `audio/wav`.
    pub mime: &'static str,
    pub duration_ms: i64,
}

enum Engine {
    /// The browser's recorder, writing MP4.
    Mp4 {
        recorder: MediaRecorder,
        chunks: Rc<RefCell<Vec<Blob>>>,
        waiting: oneshot::Receiver<()>,
        _on_data: Closure<dyn FnMut(BlobEvent)>,
        _on_stop: Closure<dyn FnMut()>,
    },
    /// Raw samples, off the microphone: for the AAC encoder where this
    /// browser has one (`aac`), and for a WAV where it has not.
    Pcm {
        context: AudioContext,
        tap: Tap,
        samples: Samples,
        aac: bool,
    },
}

/// Where the microphone's samples come out of the audio graph.
enum Tap {
    /// A worklet, on the audio thread: it gathers the samples there and
    /// posts them here a block at a time. A message waits for a busy page;
    /// nothing is lost to one.
    Worklet {
        node: AudioWorkletNode,
        /// Fires when the worklet has handed over its last, part-filled
        /// block.
        drained: oneshot::Receiver<()>,
        /// How loud the blocks have been.
        level: Rc<Level>,
        _on_block: Closure<dyn FnMut(MessageEvent)>,
    },
    /// A `ScriptProcessorNode`, on the page's own thread — for a browser
    /// with no worklet. What arrives while the page is busy is lost.
    Script {
        processor: ScriptProcessorNode,
        level: Rc<Level>,
        _on_audio: Closure<dyn FnMut(AudioProcessingEvent)>,
    },
}

/// How long a worklet is given to hand over its last block. It answers in
/// one turn of the audio thread — 3 ms — unless the context has stopped.
const DRAIN_MS: u32 = 500;

impl Tap {
    fn level(&self) -> &Rc<Level> {
        match self {
            Tap::Worklet { level, .. } | Tap::Script { level, .. } => level,
        }
    }

    /// Stop listening without keeping what is still on its way. The
    /// handler goes FIRST: an event arriving after its closure was dropped
    /// throws, every time, for as long as the thing keeps running.
    fn detach(&self) {
        match self {
            Tap::Worklet { node, .. } => {
                if let Ok(port) = node.port() {
                    port.set_onmessage(None);
                    let _ = port.post_message(&JsValue::from_str(TAP_STOP));
                }
                let _ = node.disconnect();
            }
            Tap::Script { processor, .. } => {
                processor.set_onaudioprocess(None);
                let _ = processor.disconnect();
            }
        }
    }

    /// Stop listening, once everything heard so far has arrived: the
    /// worklet is asked for the block it was still filling, and waited on
    /// for it — the last words of a note are in it.
    async fn finish(self) {
        if let Tap::Worklet { node, drained, .. } = self {
            if let Ok(port) = node.port() {
                if port.post_message(&JsValue::from_str(TAP_STOP)).is_ok() {
                    let _ = select(drained, TimeoutFuture::new(DRAIN_MS)).await;
                }
                port.set_onmessage(None);
            }
            let _ = node.disconnect();
        } else {
            self.detach();
        }
    }
}

impl Engine {
    /// Stop without keeping anything.
    fn abandon(self) {
        match self {
            Engine::Mp4 { recorder, .. } => {
                recorder.set_ondataavailable(None);
                recorder.set_onstop(None);
                let _ = recorder.stop();
            }
            Engine::Pcm { context, tap, .. } => {
                tap.detach();
                let _ = context.close();
            }
        }
    }
}

/// A recording in progress. However it ends — stopped, cancelled, or
/// simply dropped because whatever held it went away — the microphone is
/// let go of: a live microphone nothing can reach is a microphone left on.
/// And the screen with it: it stays on for as long as the recording lives.
pub struct Recording {
    stream: MediaStream,
    engine: Option<Engine>,
    /// When it started, by [`now_ms`].
    started: f64,
    /// The listeners [`Recording::on_lost`] put on the microphone's tracks.
    watching: Vec<Watch>,
    _awake: ScreenAwake,
}

/// One listener on one track.
struct Watch {
    track: web_sys::MediaStreamTrack,
    event: &'static str,
    listener: Closure<dyn FnMut()>,
}

impl Drop for Recording {
    fn drop(&mut self) {
        // The listeners FIRST: nothing this recording does on its way out
        // is news to anybody.
        self.unwatch();
        if let Some(engine) = self.engine.take() {
            engine.abandon();
        }
        stop_tracks(&self.stream);
        RUNNING.set(RUNNING.get().saturating_sub(1));
    }
}

/// What a recording reads the microphone through, made IN THE CLICK that
/// asks for one — before anything is awaited.
///
/// A browser holds an audio context it did not see a click make, and what
/// comes after the permission prompt is no longer the click: made there,
/// the context could be held, and the recording then waited on it with the
/// microphone already live before giving up on it. Made here it is running
/// by the time the person has answered the prompt. None where the browser
/// has no audio context at all; the recorder is then its own.
pub struct Listening(Option<AudioContext>);

impl Listening {
    pub fn in_the_click() -> Listening {
        let context = voice_context();
        if let Some(context) = &context {
            // Safari starts a context on `resume()` in a click, not on its
            // being made in one.
            let _ = context.resume();
        }
        Listening(context)
    }
}

impl Drop for Listening {
    /// Not used after all — permission refused, no microphone — and not
    /// left open: a browser allows a page only a handful of contexts.
    fn drop(&mut self) {
        if let Some(context) = self.0.take() {
            let _ = context.close();
        }
    }
}

impl Recording {
    /// Ask for the microphone and start. The browser asks the person the
    /// first time; a refusal is said out loud, not left as a dead button.
    pub async fn start(mut listening: Listening) -> Result<Recording, Failure> {
        let navigator = web_sys::window().ok_or(Failure::CouldNotStart)?.navigator();
        // Absent outside a secure context — an http:// server on a LAN.
        let devices = navigator
            .media_devices()
            .map_err(|_| Failure::CouldNotStart)?;
        let constraints = MediaStreamConstraints::new();
        // One channel, where the microphone can: a voice note is mono, and
        // the browser's own recorder writes as many channels as it is
        // handed. Asked for as `ideal`, which no microphone can fail.
        let mono = js_sys::Object::new();
        let ideal = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&ideal, &JsValue::from_str("ideal"), &JsValue::from(1));
        let _ = js_sys::Reflect::set(&mono, &JsValue::from_str("channelCount"), &ideal);
        constraints.set_audio(&mono);
        let promise = devices
            .get_user_media_with_constraints(&constraints)
            .map_err(|_| Failure::CouldNotStart)?;
        let stream: MediaStream = match JsFuture::from(promise).await {
            Ok(stream) => stream.unchecked_into(),
            Err(error) => {
                let name = js_sys::Reflect::get(&error, &JsValue::from_str("name"))
                    .ok()
                    .and_then(|name| name.as_string())
                    .unwrap_or_default();
                return Err(if name == "NotAllowedError" || name == "SecurityError" {
                    Failure::MicrophoneDenied
                } else {
                    Failure::CouldNotStart
                });
            }
        };
        let engine = engine(&stream, listening.0.take()).await;
        match engine {
            Some(engine) => Ok(Recording::begun(stream, engine)),
            None => {
                stop_tracks(&stream);
                Err(Failure::CouldNotStart)
            }
        }
    }

    /// A recording that has started: counted, and keeping the screen on.
    fn begun(stream: MediaStream, engine: Engine) -> Recording {
        RUNNING.set(RUNNING.get() + 1);
        #[cfg(test)]
        LAST_STREAM.with(|last| *last.borrow_mut() = Some(Clone::clone(&stream)));
        Recording {
            stream,
            engine: Some(engine),
            started: now_ms(),
            watching: Vec::new(),
            _awake: ScreenAwake::hold(),
        }
    }

    /// When it started, by [`now_ms`] — where the timer the person sees
    /// counts from.
    pub fn started_ms(&self) -> f64 {
        self.started
    }

    /// How loud it is, for the level meter and the silence warning (S2.9) —
    /// None for the browser's own recorder, which hands nothing over until
    /// it stops and so has no level to give: it draws the dot only.
    pub fn meter(&self) -> Option<Meter> {
        match &self.engine {
            Some(Engine::Pcm { tap, .. }) => Some(Meter(tap.level().clone())),
            _ => None,
        }
    }

    /// Say to `heard` when the microphone stops being this recording's — the
    /// device gone, the permission taken back, the system taking it for a
    /// call — for as long as the recording runs. Called again, it replaces
    /// the last listener. Said from inside the track's own event: whoever
    /// hears it and hands the recording on must not end it there and then.
    pub fn on_lost(&mut self, heard: impl Fn(Lost) + 'static) {
        self.unwatch();
        let heard = Rc::new(heard);
        for track in self.stream.get_audio_tracks().iter() {
            let Ok(track) = track.dyn_into::<web_sys::MediaStreamTrack>() else {
                continue;
            };
            for (event, lost) in [("ended", Lost::Ended), ("mute", Lost::Muted)] {
                let heard = heard.clone();
                let listener = Closure::<dyn FnMut()>::new(move || heard(lost));
                let _ = track
                    .add_event_listener_with_callback(event, listener.as_ref().unchecked_ref());
                self.watching.push(Watch {
                    // Rust's clone — the same track. `track.clone()` is the
                    // DOM's, which makes a new track of its own.
                    track: Clone::clone(&track),
                    event,
                    listener,
                });
            }
        }
    }

    fn unwatch(&mut self) {
        for watch in self.watching.drain(..) {
            let _ = watch.track.remove_event_listener_with_callback(
                watch.event,
                watch.listener.as_ref().unchecked_ref(),
            );
        }
    }

    /// How long it has been going, in milliseconds.
    pub fn elapsed_ms(&self) -> f64 {
        now_ms() - self.started
    }

    /// Stop and hand over what was recorded — None when it is too short to
    /// be anything (ios AudioRecorder: 1024 bytes or less).
    pub async fn stop(mut self) -> Option<Recorded> {
        // Stopped is stopped: a microphone that goes away while the note is
        // being finished is not news.
        self.unwatch();
        let duration_ms = self.elapsed_ms().round() as i64;
        #[cfg(test)]
        testing::while_stops_held().await;
        let recorded = match self.engine.take()? {
            Engine::Mp4 {
                recorder,
                chunks,
                waiting,
                _on_data,
                _on_stop,
            } => {
                let _ = recorder.stop();
                // The last chunk comes before `stop`; a recorder that never
                // says stop is not waited on for ever.
                let _ = select(waiting, TimeoutFuture::new(5_000)).await;
                recorder.set_ondataavailable(None);
                recorder.set_onstop(None);
                let parts = js_sys::Array::new();
                for chunk in chunks.borrow().iter() {
                    parts.push(chunk);
                }
                let options = BlobPropertyBag::new();
                options.set_type("audio/mp4");
                let blob = Blob::new_with_blob_sequence_and_options(&parts, &options).ok()?;
                Recorded {
                    blob,
                    mime: "audio/mp4",
                    duration_ms,
                }
            }
            Engine::Pcm {
                context,
                tap,
                samples,
                aac,
            } => {
                tap.finish().await;
                let rate = context.sample_rate() as u32;
                let _ = context.close();
                // Let go of the microphone BEFORE encoding: its light should
                // go out when Stop is pressed, not a second later.
                stop_tracks(&self.stream);
                finished(&samples, rate, aac).await?
            }
        };
        // The microphone goes with `self`, below.
        (recorded.blob.size() as u64 > media::VOICE_MIN_BYTES).then_some(recorded)
    }

    /// Give it up: nothing is kept, and the microphone is let go of.
    pub fn cancel(self) {
        drop(self);
    }
}

/// The microphone's raw samples, tapped the way a voice note's are — off
/// the page's thread where the browser has a worklet — for a VIDEO MESSAGE's
/// sound, which crate::round_video encodes beside its picture. Whatever
/// becomes of it, the tap is taken off and the context closed: dropped
/// unfinished, it keeps nothing.
pub struct RawSound {
    context: Option<AudioContext>,
    tap: Option<Tap>,
    samples: Samples,
    /// When the tap began to hear, by [`now_ms`].
    began: f64,
}

impl RawSound {
    /// `stream`'s sound, through the context made in the click that asked
    /// for it (`listening`) — one at a rate the profile names wherever the
    /// browser makes one. None where the context will not run or nothing can
    /// tap it.
    pub async fn start(stream: &MediaStream, mut listening: Listening) -> Option<RawSound> {
        let context = listening.0.take().or_else(voice_context)?;
        if !running(&context).await {
            let _ = context.close();
            return None;
        }
        let samples: Samples = Rc::new(js_sys::Array::new());
        let level = Rc::new(Level::default());
        let tap = match worklet_tap(stream, &context, &samples, &level).await {
            Some(tap) => Some(tap),
            None => script_tap(stream, &context, &samples, &level),
        };
        let Some(tap) = tap else {
            let _ = context.close();
            return None;
        };
        Some(RawSound {
            context: Some(context),
            tap: Some(tap),
            samples,
            began: now_ms(),
        })
    }

    /// Samples a second.
    pub fn rate(&self) -> u32 {
        self.context
            .as_ref()
            .map_or(0, |context| context.sample_rate() as u32)
    }

    /// When the tap began to hear, by [`now_ms`].
    pub fn began_ms(&self) -> f64 {
        self.began
    }

    /// Stop hearing, once what was heard has all arrived: the blocks of mono
    /// samples, in order, and their rate.
    pub async fn finish(mut self) -> (js_sys::Array, u32) {
        let rate = self.rate();
        if let Some(tap) = self.tap.take() {
            tap.finish().await;
        }
        if let Some(context) = self.context.take() {
            let _ = context.close();
        }
        (Clone::clone(&*self.samples), rate)
    }
}

impl Drop for RawSound {
    fn drop(&mut self) {
        if let Some(tap) = self.tap.take() {
            tap.detach();
        }
        if let Some(context) = self.context.take() {
            let _ = context.close();
        }
    }
}

fn stop_tracks(stream: &MediaStream) {
    for track in stream.get_tracks().iter() {
        if let Ok(track) = track.dyn_into::<web_sys::MediaStreamTrack>() {
            track.stop();
        }
    }
}

fn mp4(stream: &MediaStream, mime: &str) -> Option<Engine> {
    let options = MediaRecorderOptions::new();
    options.set_mime_type(mime);
    // The voice-note row's rate; a recorder left to itself picks 128 000.
    options.set_audio_bits_per_second(VOICE_NOTE_BITRATE as u32);
    let recorder =
        MediaRecorder::new_with_media_stream_and_media_recorder_options(stream, &options).ok()?;
    let chunks = Rc::new(RefCell::new(Vec::new()));
    let (sender, waiting) = oneshot::channel();
    let stopped = Rc::new(RefCell::new(Some(sender)));
    let on_data = {
        let chunks = chunks.clone();
        Closure::<dyn FnMut(BlobEvent)>::new(move |event: BlobEvent| {
            if let Some(data) = event.data() {
                chunks.borrow_mut().push(data);
            }
        })
    };
    let on_stop = {
        let stopped = stopped.clone();
        Closure::<dyn FnMut()>::new(move || {
            if let Some(sender) = stopped.borrow_mut().take() {
                let _ = sender.send(());
            }
        })
    };
    recorder.set_ondataavailable(Some(on_data.as_ref().unchecked_ref()));
    recorder.set_onstop(Some(on_stop.as_ref().unchecked_ref()));
    if recorder.start().is_err() || recorder.mime_type().to_ascii_lowercase().contains("opus") {
        recorder.set_ondataavailable(None);
        recorder.set_onstop(None);
        let _ = recorder.stop();
        return None;
    }
    Some(Engine::Mp4 {
        recorder,
        chunks,
        waiting,
        _on_data: on_data,
        _on_stop: on_stop,
    })
}

/// A context to read the microphone through, at a rate the profile names.
///
/// Left to itself a context runs at the rate of the OUTPUT device, and that
/// is not always 44.1 or 48 kHz: Bluetooth earphones with their microphone
/// in use run at 16 or 24 kHz, and everybody wearing them — which is a
/// good share of the people recording a voice note — would have their note
/// go the recorder's way for it. Asked for a rate, the browser resamples
/// between the device and the context. A browser that will not make one at
/// a chosen rate makes one at its own.
fn voice_context() -> Option<AudioContext> {
    let options = AudioContextOptions::new();
    options.set_sample_rate(VOICE_RATES[1] as f32);
    AudioContext::new_with_context_options(&options)
        .or_else(|_| AudioContext::new())
        .ok()
}

/// The way this browser records a voice note — see the top of the file.
/// `context` is the one made in the click ([`Listening`]), when there is one.
async fn engine(stream: &MediaStream, context: Option<AudioContext>) -> Option<Engine> {
    // The encoder is asked about the rate the microphone's samples will
    // actually arrive at, which only the context knows: the one it was
    // asked for ([`voice_context`]), or — in a browser that would not make
    // one at a chosen rate — its device's, which is left to the recorder
    // unless it happens to be one of the two the profile names.
    if let Some(context) = context.or_else(voice_context) {
        let rate = context.sample_rate() as u32;
        if VOICE_RATES.contains(&rate)
            && webcodecs::aac_supported(rate, 1, VOICE_NOTE_BITRATE).await
            && running(&context).await
        {
            if let Some(engine) = pcm(stream, context, true).await {
                return Some(engine);
            }
        } else {
            let _ = context.close();
        }
    }
    let recorder = MP4_TYPES
        .into_iter()
        .filter(|mime| MediaRecorder::is_type_supported(mime))
        .find_map(|mime| mp4(stream, mime));
    match recorder {
        Some(engine) => Some(engine),
        None => pcm(stream, AudioContext::new().ok()?, false).await,
    }
}

/// How long a context that is not running yet is given to start. A slow
/// machine takes a good part of this to bring its audio device up, and a
/// context given up on too soon is a note recorded the second-best way for
/// no reason. One that IS running — as one made in the click nearly always
/// is, by the time the permission prompt has been answered — is not waited
/// on at all.
const CONTEXT_START_MS: u32 = 1_000;

thread_local! {
    /// Whether this browser held the last context it was asked to run.
    /// The microphone is already live while a context is waited on, and
    /// what is said in that time is recorded by nobody — so a browser that
    /// has held one is not waited on again, note after note: its next
    /// context is recorded from only if it is running when it is looked at.
    static HELD: Cell<bool> = const { Cell::new(false) };
}

/// Whether `context` is actually running. A browser may hold a context it
/// did not see a click make, and a held context hands over no samples: the
/// note would record, in silence, to nothing. `resume()` settles when the
/// context runs, and does not settle while it is held.
async fn running(context: &AudioContext) -> bool {
    if context.state() == AudioContextState::Running {
        HELD.set(false);
        return true;
    }
    let Ok(resumed) = context.resume() else {
        return false;
    };
    if HELD.get() {
        return false;
    }
    let runs = matches!(
        select(
            JsFuture::from(resumed),
            TimeoutFuture::new(CONTEXT_START_MS)
        )
        .await,
        futures::future::Either::Left((Ok(_), _))
    );
    HELD.set(!runs);
    runs
}

async fn pcm(stream: &MediaStream, context: AudioContext, aac: bool) -> Option<Engine> {
    let samples: Samples = Rc::new(js_sys::Array::new());
    let level = Rc::new(Level::default());
    let tap = match worklet_tap(stream, &context, &samples, &level).await {
        Some(tap) => Some(tap),
        None => script_tap(stream, &context, &samples, &level),
    };
    let Some(tap) = tap else {
        let _ = context.close();
        return None;
    };
    Some(Engine::Pcm {
        context,
        tap,
        samples,
        aac,
    })
}

/// Samples to a block: what the page is handed at a time, 85 ms at 48 kHz.
const TAP_BLOCK: u32 = 4096;

/// What the page says to the worklet when the recording stops.
const TAP_STOP: &str = "stop";

/// The worklet's name in the audio graph.
const TAP_NAME: &str = "fc-voice-tap";

/// The worklet itself — the one piece of this client that is not Rust,
/// because an `AudioWorkletProcessor` is a class defined inside the audio
/// thread's own scope, which nothing compiled for the page can reach. It
/// does as little as a thing can: it copies the first (mono) channel of
/// what it is given into a block, posts each block as it fills — handing
/// the block itself over, not a copy — and, told to stop, posts the part
/// it was still filling and then `null`, which says "that was all of it".
fn tap_source() -> String {
    format!(
        "registerProcessor('{TAP_NAME}', class extends AudioWorkletProcessor {{
  constructor() {{
    super();
    this.block = new Float32Array({TAP_BLOCK});
    this.filled = 0;
    this.done = false;
    this.port.onmessage = (event) => {{
      if (event.data !== '{TAP_STOP}' || this.done) return;
      this.done = true;
      if (this.filled > 0) this.port.postMessage(this.block.slice(0, this.filled));
      this.port.postMessage(null);
    }};
  }}
  process(inputs) {{
    if (this.done) return false;
    const heard = inputs[0] && inputs[0][0];
    if (!heard) return true;
    let at = 0;
    while (at < heard.length) {{
      const take = Math.min(heard.length - at, this.block.length - this.filled);
      this.block.set(heard.subarray(at, at + take), this.filled);
      this.filled += take;
      at += take;
      if (this.filled === this.block.length) {{
        this.port.postMessage(this.block, [this.block.buffer]);
        this.block = new Float32Array({TAP_BLOCK});
        this.filled = 0;
      }}
    }}
    return true;
  }}
}});"
    )
}

/// How long the worklet's module is given to load. It is a few hundred
/// bytes handed over as a `blob:` URL — no request leaves the page — but
/// the first load also starts the audio thread's own scope, which a slow
/// machine takes its time over. Anything longer is a browser that will
/// not run it.
const TAP_LOAD_MS: u32 = 3_000;

/// The microphone tapped on the audio thread, or None where this browser
/// has no worklet or will not load one — a page whose policy forbids
/// `blob:` scripts, say.
async fn worklet_tap(
    stream: &MediaStream,
    context: &AudioContext,
    samples: &Samples,
    level: &Rc<Level>,
) -> Option<Tap> {
    let source = js_sys::Array::of1(&JsValue::from_str(&tap_source()));
    let options = BlobPropertyBag::new();
    options.set_type("text/javascript");
    let module = Blob::new_with_str_sequence_and_options(&source, &options).ok()?;
    let url = Url::create_object_url_with_blob(&module).ok()?;
    let loading = context
        .audio_worklet()
        .and_then(|worklet| worklet.add_module(&url));
    let loaded = match loading {
        Ok(loading) => matches!(
            select(JsFuture::from(loading), TimeoutFuture::new(TAP_LOAD_MS)).await,
            futures::future::Either::Left((Ok(_), _))
        ),
        Err(_) => false,
    };
    let _ = Url::revoke_object_url(&url);
    if !loaded {
        return None;
    }
    // One input channel: a stereo microphone is mixed down by the browser,
    // and a voice note is mono whatever it was recorded with.
    let options = AudioWorkletNodeOptions::new();
    options.set_number_of_inputs(1);
    options.set_number_of_outputs(1);
    options.set_channel_count(1);
    options.set_channel_count_mode(ChannelCountMode::Explicit);
    let node = AudioWorkletNode::new_with_options(context, TAP_NAME, &options).ok()?;
    let (sender, drained) = oneshot::channel();
    let sender = RefCell::new(Some(sender));
    let on_block = {
        let samples = samples.clone();
        let level = level.clone();
        Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
            let data = event.data();
            if data.is_null() {
                if let Some(sender) = sender.borrow_mut().take() {
                    let _ = sender.send(());
                }
            } else if data.is_instance_of::<js_sys::Float32Array>() {
                samples.push(&data);
                level.hear(&data.unchecked_ref::<js_sys::Float32Array>().to_vec());
            }
        })
    };
    let port = node.port().ok()?;
    port.set_onmessage(Some(on_block.as_ref().unchecked_ref()));
    let wired = context
        .create_media_stream_source(stream)
        .and_then(|source| source.connect_with_audio_node(&node))
        // Connected through to the speakers, as the other tap is, for the
        // browser that only runs what something is listening to; its
        // output is silence, since nothing is ever written to it.
        .and_then(|_| node.connect_with_audio_node(&context.destination()));
    if wired.is_err() {
        port.set_onmessage(None);
        let _ = node.disconnect();
        return None;
    }
    Some(Tap::Worklet {
        node,
        drained,
        level: level.clone(),
        _on_block: on_block,
    })
}

fn script_tap(
    stream: &MediaStream,
    context: &AudioContext,
    samples: &Samples,
    level: &Rc<Level>,
) -> Option<Tap> {
    let source = context.create_media_stream_source(stream).ok()?;
    // One input channel: a stereo microphone is mixed down by the browser,
    // and a voice note is mono whatever it was recorded with.
    let processor = context
        .create_script_processor_with_buffer_size_and_number_of_input_channels_and_number_of_output_channels(TAP_BLOCK, 1, 1)
        .ok()?;
    let on_audio = {
        let samples = samples.clone();
        let level = level.clone();
        Closure::<dyn FnMut(AudioProcessingEvent)>::new(move |event: AudioProcessingEvent| {
            if let Ok(buffer) = event.input_buffer() {
                if let Ok(channel) = buffer.get_channel_data(0) {
                    samples.push(&js_sys::Float32Array::from(channel.as_slice()));
                    level.hear(&channel);
                }
            }
        })
    };
    processor.set_onaudioprocess(Some(on_audio.as_ref().unchecked_ref()));
    source.connect_with_audio_node(&processor).ok()?;
    // Connected through to the speakers or some browsers never run it; its
    // output is silence, since nothing is ever written to it.
    processor
        .connect_with_audio_node(&context.destination())
        .ok()?;
    Some(Tap::Script {
        processor,
        level: level.clone(),
        _on_audio: on_audio,
    })
}

/// Raw mono `samples` at `rate` Hz as a voice note: M4A where `aac` says
/// this browser encodes it, and a WAV where it does not — or where the
/// encoder, asked, turned out not to after all. A recording is never lost
/// to an encoder: the samples are still here, and the WAV needs nothing
/// the browser could refuse.
async fn finished(samples: &js_sys::Array, rate: u32, aac: bool) -> Option<Recorded> {
    let count: u32 = samples
        .iter()
        .map(|block| block.unchecked_into::<js_sys::Float32Array>().length())
        .sum();
    if aac {
        if let Some(recorded) = as_m4a(samples, count, rate).await {
            return Some(recorded);
        }
        log::warn!("This browser's AAC encoder failed; the voice note goes as WAV");
    }
    let mut all = Vec::with_capacity(count as usize);
    for block in samples.iter() {
        all.extend(block.unchecked_into::<js_sys::Float32Array>().to_vec());
    }
    let voice = wav::resample(&all, rate, wav::VOICE_RATE);
    let bytes = wav::encode(&voice, wav::VOICE_RATE);
    let array = js_sys::Uint8Array::from(bytes.as_slice());
    let parts = js_sys::Array::of1(&array);
    let options = BlobPropertyBag::new();
    options.set_type("audio/wav");
    let blob = Blob::new_with_u8_array_sequence_and_options(&parts, &options).ok()?;
    Some(Recorded {
        blob,
        mime: "audio/wav",
        duration_ms: wav::duration_ms(voice.len(), wav::VOICE_RATE),
    })
}

/// The voice-note row of the profile: M4A, AAC-LC, mono, at the rate the
/// microphone arrived at, 64 000 bit/s.
async fn as_m4a(samples: &js_sys::Array, count: u32, rate: u32) -> Option<Recorded> {
    let mut blocks = samples.iter();
    let aac = encode::aac_from_pcm(
        move || {
            let block: js_sys::Float32Array = blocks.next()?.unchecked_into();
            let frames = block.length();
            Some((block, frames))
        },
        rate,
        1,
        VOICE_NOTE_BITRATE,
    )
    .await?;
    Some(Recorded {
        blob: encode::m4a(&aac)?,
        mime: "audio/mp4",
        duration_ms: wav::duration_ms(count as usize, rate),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encode::testing::{heard, noise, read, whole};
    use crate::webcodecs::testing::{refusing, without, Lack, Stand};
    use fc_text::mp4_read;
    use wasm_bindgen_test::*;

    /// What a microphone hands over: blocks of 4096 samples.
    fn spoken(seconds: u32, rate: u32) -> js_sys::Array {
        let samples = noise(seconds * rate, rate, 9);
        let blocks = js_sys::Array::new();
        for block in samples.chunks(4096) {
            blocks.push(&js_sys::Float32Array::from(block));
        }
        blocks
    }

    /// The voice-note row, to the letter: M4A with its index first, AAC-LC,
    /// one channel, 64 000 bit/s — and a fraction of the WAV it replaces.
    #[wasm_bindgen_test]
    async fn a_voice_note_is_mono_aac_lc_at_64k_in_an_m4a() {
        let recorded = finished(&spoken(5, 48_000), 48_000, true).await.unwrap();
        assert_eq!(recorded.mime, "audio/mp4");
        assert_eq!(recorded.blob.type_(), "audio/mp4");
        assert_eq!(recorded.duration_ms, 5_000);
        let bytes = whole(&recorded.blob).await;
        assert!(
            media::matches_magic("audio/mp4", &bytes[..12]),
            "the server takes it"
        );
        assert_eq!(&bytes[4..12], b"ftypM4A ");
        let (kinds, movie) = read(&bytes);
        assert_eq!(kinds, vec![*b"ftyp", *b"moov", *b"mdat"]);
        assert_eq!(movie.tracks.len(), 1);
        let track = movie.audio().unwrap();
        let config = mp4_read::audio_config(&track.entry.as_ref().unwrap().config).unwrap();
        assert_eq!(
            (config.object_type, config.channels, config.sample_rate),
            (2, 1, 48_000)
        );
        let rate = track.data_rate().unwrap();
        assert!((48_000..=72_000).contains(&rate), "{rate} bit/s");
        // The WAV this browser used to fall back to would be 160 000 bytes
        // at 16 kHz — and 480 000 at the microphone's own rate.
        assert!(bytes.len() < 48_000, "{} bytes", bytes.len());
        let decoded = heard(&recorded.blob, 48_000).await;
        assert!(
            (4.95..=5.15).contains(&decoded.duration()),
            "{} s",
            decoded.duration()
        );
        // 44.1 kHz, the other rate a microphone arrives at.
        let cd = finished(&spoken(2, 44_100), 44_100, true).await.unwrap();
        let (_, movie) = read(&whole(&cd.blob).await);
        let entry = movie.audio().unwrap().entry.clone().unwrap();
        assert_eq!(
            mp4_read::audio_config(&entry.config).unwrap().sample_rate,
            44_100
        );
        assert_eq!(cd.duration_ms, 2_000);
    }

    /// With no AAC encoder the note is still a WAV — but speech-sized:
    /// mono, 16 bits, 16 kHz, whatever rate the microphone ran at. And an
    /// encoder that was promised and then failed costs nobody a recording.
    #[wasm_bindgen_test]
    async fn without_an_aac_encoder_a_voice_note_is_a_16k_mono_wav() {
        let check = |bytes: &[u8]| {
            assert!(media::matches_magic("audio/wav", &bytes[..12]));
            let channels = u16::from_le_bytes(bytes[22..24].try_into().unwrap());
            let rate = u32::from_le_bytes(bytes[24..28].try_into().unwrap());
            let bits = u16::from_le_bytes(bytes[34..36].try_into().unwrap());
            assert_eq!((channels, rate, bits), (1, 16_000, 16));
            assert_eq!(bytes.len(), 44 + 2 * 16_000 * 2, "two seconds");
        };
        let recorded = finished(&spoken(2, 48_000), 48_000, false).await.unwrap();
        assert_eq!((recorded.mime, recorded.duration_ms), ("audio/wav", 2_000));
        check(&whole(&recorded.blob).await);
        for stand in [without as Lack, refusing] {
            let _stand = stand("AudioEncoder");
            // Promised at the start, gone by the end.
            let recorded = finished(&spoken(2, 48_000), 48_000, true).await.unwrap();
            assert_eq!(recorded.mime, "audio/wav");
            check(&whole(&recorded.blob).await);
        }
    }

    // --- a real capture -------------------------------------------------------------------------
    //
    // The tests below record. `webdriver.json`, beside Cargo.toml, starts
    // the test browser with a microphone that is not one (Chrome's fake
    // capture device: a beep a second, mono, 48 kHz), with its permission
    // prompt already answered, and with contexts allowed to run on a page
    // nobody has clicked in — which a test page is.

    async fn sleep(ms: u32) {
        TimeoutFuture::new(ms).await;
    }

    /// A page too busy to look up, for `ms`: nothing else runs on its
    /// thread — no callback, no timer, no message.
    fn busy(ms: f64) {
        let until = js_sys::Date::now() + ms;
        while js_sys::Date::now() < until {}
    }

    /// The microphone is read at a rate the profile names whatever rate the
    /// machine's own audio runs at — 24 kHz, with Bluetooth earphones on,
    /// which is how this was found: these tests, and the note of anybody
    /// wearing them, went the recorder's way the moment a pair connected.
    #[wasm_bindgen_test]
    fn the_microphone_is_read_at_a_rate_the_profile_names() {
        let context = voice_context().unwrap();
        assert_eq!(context.sample_rate(), 48_000.0);
        let _ = context.close();
        let listening = Listening::in_the_click();
        assert_eq!(listening.0.as_ref().unwrap().sample_rate(), 48_000.0);
    }

    /// And if the test browser was started without them, that is said here,
    /// by name — not by six tests failing for a reason none of them states.
    #[wasm_bindgen_test]
    async fn the_test_browser_has_a_microphone_and_lets_a_context_run() {
        let context = AudioContext::new().unwrap();
        assert!(
            running(&context).await,
            "webdriver.json's --autoplay-policy=no-user-gesture-required is not in force"
        );
        let _ = context.close();
        let recording = Recording::start(Listening::in_the_click()).await;
        assert!(
            recording.is_ok(),
            "webdriver.json's --use-fake-device-for-media-stream and \
             --use-fake-ui-for-media-stream are not in force: {:?}",
            recording.err()
        );
        recording.unwrap().cancel();
    }

    /// The whole path, from the microphone: asked for, recorded through the
    /// tap, encoded — and what comes out is the voice-note row, as long as
    /// the recording was, with the microphone's sound in it.
    #[wasm_bindgen_test]
    async fn a_note_recorded_from_the_microphone_is_the_profiles_row() {
        let recording = Recording::start(Listening::in_the_click())
            .await
            .expect("recording starts");
        assert!(matches!(
            recording.engine,
            Some(Engine::Pcm {
                aac: true,
                tap: Tap::Worklet { .. },
                ..
            })
        ));
        sleep(1_500).await;
        let elapsed = recording.elapsed_ms();
        let recorded = recording.stop().await.expect("a note");
        assert_eq!(recorded.mime, "audio/mp4");
        // As long as it was recorded for: nothing dropped on the way, and
        // the last part-filled block handed over too.
        let drift = (recorded.duration_ms as f64 - elapsed).abs();
        assert!(
            drift < 200.0,
            "{} ms of a {elapsed} ms recording",
            recorded.duration_ms
        );
        let bytes = whole(&recorded.blob).await;
        let (kinds, movie) = read(&bytes);
        assert_eq!(kinds, vec![*b"ftyp", *b"moov", *b"mdat"]);
        let track = movie.audio().unwrap();
        let config = mp4_read::audio_config(&track.entry.as_ref().unwrap().config).unwrap();
        assert_eq!(
            (config.object_type, config.channels),
            (2, 1),
            "AAC-LC, mono"
        );
        assert!(VOICE_RATES.contains(&config.sample_rate));
        // The microphone's sound, not silence.
        let decoded = heard(&recorded.blob, config.sample_rate).await;
        let samples = decoded.get_channel_data(0).unwrap();
        let loudest = samples.iter().fold(0f32, |peak, s| peak.max(s.abs()));
        assert!(loudest > 0.05, "the loudest sample is {loudest}");
        // And staged as the audio it is.
        let staged = crate::prep::recording(recorded.blob, recorded.mime, recorded.duration_ms)
            .await
            .unwrap();
        assert_eq!(
            (staged.kind.as_str(), staged.mime.as_str()),
            ("audio", "audio/mp4")
        );
        assert_eq!(staged.duration_ms, Some(recorded.duration_ms));
    }

    /// A steady tone standing in for a voice, so that a hole in what was
    /// recorded can be SEEN: the context it is played in, and its stream.
    fn tone() -> (AudioContext, MediaStream) {
        let context = AudioContext::new().unwrap();
        let stream = js_sys::Function::new_with_args(
            "context",
            "const tone = context.createOscillator(); tone.frequency.value = 440; \
             const out = context.createMediaStreamDestination(); \
             tone.connect(out); tone.start(); return out.stream;",
        )
        .call1(&JsValue::NULL, &context)
        .unwrap()
        .unchecked_into();
        (context, stream)
    }

    /// Where a recording of a 440 Hz tone stops being one: each sample of
    /// a sine follows from the two before it, so a sample that does not is
    /// where something was dropped, repeated or put in.
    fn breaks(samples: &[f32], rate: f32) -> Vec<usize> {
        let turn = 2.0 * (std::f32::consts::TAU * 440.0 / rate).cos();
        let mut found = Vec::new();
        let mut at = 2;
        while at < samples.len() {
            if (samples[at] - (turn * samples[at - 1] - samples[at - 2])).abs() > 0.01 {
                found.push(at);
                at += 2;
            }
            at += 1;
        }
        found
    }

    /// Everything a tap heard of `stream` over a second and a half, with
    /// the page busy — deaf to every callback — for 600 ms in the middle.
    async fn heard_while_busy(
        tap: Tap,
        context: AudioContext,
        samples: Samples,
    ) -> (Vec<f32>, f32) {
        sleep(500).await;
        busy(600.0);
        sleep(400).await;
        tap.finish().await;
        let rate = context.sample_rate();
        let _ = context.close();
        let mut all = Vec::new();
        for block in samples.iter() {
            all.extend(block.unchecked_into::<js_sys::Float32Array>().to_vec());
        }
        (all, rate)
    }

    /// A page that is busy while somebody talks — a timeline rendering, a
    /// video being transcoded in the same tab — loses none of the note.
    /// The tap this replaced, on the page's own thread, lost what arrived
    /// while the page was not looking: the same tone came out of it with
    /// five holes across those 600 ms.
    #[wasm_bindgen_test]
    async fn a_busy_page_loses_none_of_a_note() {
        let (playing, stream) = tone();
        let started = js_sys::Date::now();
        let Some(Engine::Pcm {
            context,
            tap,
            samples,
            aac: true,
        }) = engine(&stream, None).await
        else {
            panic!("this browser records through its AAC encoder");
        };
        assert!(matches!(tap, Tap::Worklet { .. }), "off the page's thread");
        let (all, rate) = heard_while_busy(tap, context, samples).await;
        let seconds = (js_sys::Date::now() - started) / 1000.0;
        stop_tracks(&stream);
        let _ = playing.close();
        // All of it arrived — the 600 ms the page was busy for included.
        let recorded = f64::from(all.len() as u32) / f64::from(rate);
        assert!(
            recorded > seconds - 0.2 && recorded < seconds + 0.05,
            "{recorded} s recorded of {seconds} s"
        );
        // And in one piece. The first tenth of a second is two contexts
        // starting up against each other, which is not the tap's doing.
        let settled = (rate / 10.0) as usize;
        let holes = breaks(&all[settled..], rate);
        assert!(holes.is_empty(), "the tone breaks at {holes:?}");
        let power =
            all[settled..].iter().map(|s| s * s).sum::<f32>() / (all.len() - settled) as f32;
        assert!(power > 0.1, "it is the tone, not silence: {power}");
    }

    /// Something on `AudioContext.prototype` replaced for the length of a
    /// test, and put back after. (The class itself cannot be stood in for:
    /// web-sys looks `AudioContext` up once, when the module loads.)
    struct Patched(Vec<(&'static str, JsValue)>);

    impl Patched {
        fn prototype() -> JsValue {
            js_sys::Function::new_no_args("return AudioContext.prototype;")
                .call0(&JsValue::NULL)
                .unwrap()
        }

        /// `name` answering whatever `getter` returns — shadowing the real
        /// accessor, which lives one prototype up and comes back when this
        /// one is deleted.
        fn getter(mut self, name: &'static str, getter: &str) -> Patched {
            js_sys::Function::new_with_args(
                "prototype, name",
                &format!(
                    "Object.defineProperty(prototype, name, \
                     {{ get() {{ {getter} }}, configurable: true }});"
                ),
            )
            .call2(&JsValue::NULL, &Self::prototype(), &JsValue::from_str(name))
            .unwrap();
            self.0.push((name, JsValue::UNDEFINED));
            self
        }

        /// The method `name` replaced by one doing `body`.
        fn method(mut self, name: &'static str, body: &str) -> Patched {
            let prototype = Self::prototype();
            let key = JsValue::from_str(name);
            let was = js_sys::Reflect::get(&prototype, &key).unwrap();
            let stand_in = js_sys::Function::new_no_args(body);
            js_sys::Reflect::set(&prototype, &key, &stand_in).unwrap();
            self.0.push((name, was));
            self
        }

        /// A browser HOLDING every context: suspended, and `resume()` never
        /// settling — what a page nobody has clicked in gets.
        fn held() -> Patched {
            Patched(Vec::new())
                .getter("state", "return 'suspended';")
                .method("resume", "return new Promise(() => {});")
        }

        /// A browser with no `AudioWorklet`.
        fn without_a_worklet() -> Patched {
            Patched(Vec::new()).getter("audioWorklet", "return undefined;")
        }
    }

    impl Drop for Patched {
        fn drop(&mut self) {
            let prototype = Self::prototype();
            for (name, was) in self.0.drain(..).rev() {
                let key = JsValue::from_str(name);
                if was.is_undefined() {
                    let _ = js_sys::Reflect::delete_property(
                        prototype.unchecked_ref::<js_sys::Object>(),
                        &key,
                    );
                } else {
                    let _ = js_sys::Reflect::set(&prototype, &key, &was);
                }
            }
        }
    }

    /// Which way a note is recorded is what the browser ANSWERS, in order:
    /// its AAC encoder — fed by a worklet, or by the page's own thread
    /// where it has no worklet — then its recorder's MP4 at 64 000, then
    /// WAV.
    #[wasm_bindgen_test]
    async fn the_way_a_note_is_recorded_is_what_the_browser_answers() {
        let (playing, stream) = tone();
        let first = engine(&stream, None).await.expect("an engine");
        assert!(
            matches!(
                first,
                Engine::Pcm {
                    aac: true,
                    tap: Tap::Worklet { .. },
                    ..
                }
            ),
            "this browser encodes AAC, and has a worklet to feed it"
        );
        first.abandon();
        {
            let _no_worklet = Patched::without_a_worklet();
            let Some(Engine::Pcm {
                context,
                tap,
                samples,
                aac: true,
            }) = engine(&stream, None).await
            else {
                panic!("the encoder is still the way, with no worklet");
            };
            assert!(matches!(tap, Tap::Script { .. }));
            // And it hears: the older tap is a worse one, not a dead one.
            sleep(400).await;
            tap.finish().await;
            let _ = context.close();
            assert!(samples.length() >= 2, "{} blocks", samples.length());
        }
        {
            let _no_encoder = refusing("AudioEncoder");
            let second = engine(&stream, None).await.expect("an engine");
            match &second {
                Engine::Mp4 { recorder, .. } => {
                    assert!(recorder.mime_type().starts_with("audio/mp4"));
                    assert_eq!(recorder.audio_bits_per_second(), 64_000);
                }
                Engine::Pcm { .. } => panic!("this browser's recorder writes MP4"),
            }
            second.abandon();
            // And a browser whose recorder writes no MP4 either.
            let no_mp4 = js_sys::Function::new_no_args(
                "const R = function() { throw new Error('no'); }; \
                 R.isTypeSupported = () => false; return R;",
            )
            .call0(&JsValue::NULL)
            .unwrap();
            let _no_recorder = Stand::in_for("MediaRecorder", &no_mp4);
            let third = engine(&stream, None).await.expect("an engine");
            assert!(matches!(third, Engine::Pcm { aac: false, .. }));
            third.abandon();
        }
        stop_tracks(&stream);
        let _ = playing.close();
    }

    /// A context the browser is holding hands over no samples, so the note
    /// is the recorder's, not a silent M4A — and a browser that has held
    /// one is not waited on again: the microphone is already live while a
    /// context is waited on, and whatever is said in that time is recorded
    /// by nobody.
    #[wasm_bindgen_test]
    async fn a_context_the_browser_holds_is_not_recorded_from_nor_waited_on_twice() {
        let (playing, stream) = tone();
        {
            let _held = Patched::held();
            let asked = js_sys::Date::now();
            let chosen = engine(&stream, None).await.expect("an engine");
            let first = js_sys::Date::now() - asked;
            assert!(matches!(chosen, Engine::Mp4 { .. }), "held: the recorder");
            chosen.abandon();
            assert!(
                first >= f64::from(CONTEXT_START_MS) - 50.0,
                "the first is given its time to start: {first} ms"
            );
            // The next note: no second wait.
            let asked = js_sys::Date::now();
            let chosen = engine(&stream, None).await.expect("an engine");
            let second = js_sys::Date::now() - asked;
            assert!(matches!(chosen, Engine::Mp4 { .. }));
            chosen.abandon();
            assert!(
                second < first - f64::from(CONTEXT_START_MS) / 2.0,
                "{second} ms the second time, {first} ms the first"
            );
        }
        // A context that is seen to run is believed again, and the encoder
        // is the way again.
        assert!(HELD.get());
        let context = AudioContext::new().unwrap();
        for _ in 0..100 {
            if context.state() == AudioContextState::Running {
                break;
            }
            sleep(20).await;
        }
        assert!(running(&context).await);
        assert!(!HELD.get());
        let _ = context.close();
        let chosen = engine(&stream, None).await.expect("an engine");
        assert!(matches!(chosen, Engine::Pcm { aac: true, .. }));
        chosen.abandon();
        stop_tracks(&stream);
        let _ = playing.close();
    }

    /// The second way, recorded for real: with no AAC encoder the browser's
    /// own recorder writes the note, and what it writes is an MP4 the
    /// server's check takes and the composer stages as audio.
    #[wasm_bindgen_test]
    async fn the_recorders_own_mp4_is_staged_as_audio() {
        let _no_encoder = refusing("AudioEncoder");
        let recording = Recording::start(Listening::in_the_click())
            .await
            .expect("recording starts");
        assert!(matches!(recording.engine, Some(Engine::Mp4 { .. })));
        sleep(1_200).await;
        let recorded = recording.stop().await.expect("a note");
        assert_eq!(recorded.mime, "audio/mp4");
        assert!((1_200..1_700).contains(&recorded.duration_ms));
        let bytes = whole(&recorded.blob).await;
        assert!(media::matches_magic("audio/mp4", &bytes[..12]));
        let staged = crate::prep::recording(recorded.blob, recorded.mime, recorded.duration_ms)
            .await
            .unwrap();
        assert_eq!(staged.kind, "audio");
    }

    /// A recording is COUNTED for as long as it lives — what a closing tab or
    /// a sign-out asks about — and keeps the SCREEN ON (the plan for #79,
    /// S1.7, S2.8), letting both go however it ends: cancelled, or stopped
    /// and finished.
    #[wasm_bindgen_test]
    async fn a_recording_is_counted_and_keeps_the_screen_on_until_it_ends() {
        let screen = crate::awake::testing::FakeWakeLock::install();
        assert!(!in_progress(), "nothing records before");
        let recording = Recording::start(Listening::in_the_click())
            .await
            .expect("recording starts");
        assert!(in_progress(), "counted while it lives");
        sleep(50).await;
        assert_eq!(
            (screen.asked(), screen.released()),
            (1, 0),
            "the screen kept on"
        );
        recording.cancel();
        sleep(50).await;
        assert!(!in_progress(), "not once it is cancelled");
        assert_eq!(screen.released(), 1, "the screen let go of with it");

        let recording = Recording::start(Listening::in_the_click())
            .await
            .expect("recording starts");
        sleep(300).await;
        let _ = recording.stop().await;
        sleep(50).await;
        assert!(!in_progress(), "nor once it is finished");
        assert_eq!((screen.asked(), screen.released()), (2, 2));
    }

    /// A microphone that stops being the recording's — the device gone, the
    /// system taking it for a call — is HEARD (the plan for #79, S4). A
    /// listener set again replaces the last, and once the recording has
    /// ended nothing is listening: a listener left behind would call into a
    /// closure that has gone, which throws in the page.
    #[wasm_bindgen_test]
    async fn a_lost_microphone_is_heard_while_the_recording_lives() {
        let mut recording = Recording::start(Listening::in_the_click())
            .await
            .expect("recording starts");
        let track: web_sys::MediaStreamTrack = recording
            .stream
            .get_audio_tracks()
            .get(0)
            .dyn_into()
            .expect("an audio track");
        let fire = |name: &str| {
            track
                .dispatch_event(&web_sys::Event::new(name).unwrap())
                .unwrap();
        };
        let replaced = Rc::new(RefCell::new(Vec::new()));
        let heard = Rc::new(RefCell::new(Vec::new()));
        {
            let replaced = replaced.clone();
            recording.on_lost(move |lost| replaced.borrow_mut().push(lost));
        }
        {
            let heard = heard.clone();
            recording.on_lost(move |lost| heard.borrow_mut().push(lost));
        }
        fire("mute");
        fire("ended");
        assert!(
            replaced.borrow().is_empty(),
            "set again, it replaced the first"
        );
        assert_eq!(*heard.borrow(), vec![Lost::Muted, Lost::Ended]);

        let thrown = Rc::new(Cell::new(0));
        let on_error = {
            let thrown = thrown.clone();
            Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| {
                thrown.set(thrown.get() + 1)
            })
        };
        let window = web_sys::window().unwrap();
        window
            .add_event_listener_with_callback("error", on_error.as_ref().unchecked_ref())
            .unwrap();
        recording.cancel();
        fire("ended");
        fire("mute");
        window
            .remove_event_listener_with_callback("error", on_error.as_ref().unchecked_ref())
            .unwrap();
        assert_eq!(heard.borrow().len(), 2, "nothing heard once it has ended");
        assert_eq!(thrown.get(), 0, "and nothing left listening");
    }

    /// Stopped is stopped: a microphone that goes away while the note is
    /// still being finished is news to nobody — and the note is finished
    /// all the same.
    #[wasm_bindgen_test]
    async fn a_microphone_lost_while_the_note_is_finished_is_not_news() {
        let mut recording = Recording::start(Listening::in_the_click())
            .await
            .expect("recording starts");
        let track: web_sys::MediaStreamTrack = recording
            .stream
            .get_audio_tracks()
            .get(0)
            .dyn_into()
            .expect("an audio track");
        let heard = Rc::new(RefCell::new(Vec::new()));
        {
            let heard = heard.clone();
            recording.on_lost(move |lost| heard.borrow_mut().push(lost));
        }
        // Long enough to be a note, not "too short".
        sleep(1_200).await;
        let (done, finished) = oneshot::channel();
        wasm_bindgen_futures::spawn_local(async move {
            let _ = done.send(recording.stop().await.is_some());
        });
        // One turn, so the stop has begun — it is waiting on the worklet's
        // last block — when the microphone goes.
        let _ = JsFuture::from(js_sys::Promise::resolve(&JsValue::NULL)).await;
        track
            .dispatch_event(&web_sys::Event::new("ended").unwrap())
            .unwrap();
        assert!(finished.await.unwrap(), "the note is finished");
        assert!(heard.borrow().is_empty(), "and the loss was news to nobody");
    }

    /// THE METER'S FIVE BARS (the plan for #79, S2.9): one at each of −50,
    /// −40, −30, −20 and −10 dBFS of the PEAK, the measure the silence check
    /// reads — so they light alike on every client.
    #[wasm_bindgen_test]
    fn the_meter_lights_a_bar_at_every_ten_decibels_from_minus_fifty() {
        let at = |dbfs: f64| 10f64.powf(dbfs / 20.0) as f32;
        assert_eq!(lit_bars(0.0), 0);
        assert_eq!(lit_bars(f32::NAN), 0);
        assert_eq!(lit_bars(-0.5), 0, "a peak is a magnitude");
        assert_eq!(lit_bars(at(-60.0)), 0, "digital silence lights nothing");
        assert_eq!(lit_bars(at(-50.5)), 0);
        assert_eq!(lit_bars(at(-49.9)), 1);
        assert_eq!(lit_bars(at(-39.9)), 2);
        assert_eq!(lit_bars(at(-29.9)), 3);
        assert_eq!(lit_bars(at(-19.9)), 4);
        assert_eq!(lit_bars(at(-10.5)), 4);
        assert_eq!(lit_bars(at(-9.9)), 5);
        assert_eq!(lit_bars(1.0), 5);
    }

    /// HEARD is anything above digital silence — a sample magnitude over
    /// 0.001, −60 dBFS (S1.1): a muted microphone, not a quiet room. The
    /// meter takes the loudest sample since it last looked, and starts again.
    #[wasm_bindgen_test]
    fn heard_is_anything_above_digital_silence() {
        let level = Rc::new(Level::default());
        let meter = Meter(level.clone());
        level.hear(&[0.0, -0.001, 0.0005, 0.001]);
        assert!(!meter.heard(), "at the silence level is silence");
        assert_eq!(meter.take_peak(), 0.001);
        assert_eq!(meter.take_peak(), 0.0, "each look starts again");
        level.hear(&[0.0, -0.0011]);
        assert!(meter.heard());
        level.hear(&[0.0]);
        assert!(meter.heard(), "once heard, heard");
        level.hear(&[0.25, -0.5, 0.1]);
        level.hear(&[0.2]);
        assert_eq!(meter.take_peak(), 0.5, "the loudest since the last look");
    }

    /// The recorder's clock is the page's monotonic one, which a test can
    /// move on — and a recording's length is measured on it. (A tone stands
    /// in for the microphone here and below: the test browser's fake one is
    /// a single device whose own settings outlive a recording ended a moment
    /// after its beep, and the tests that listen for its beep come after.)
    #[wasm_bindgen_test]
    async fn a_recording_is_timed_on_the_monotonic_clock() {
        let performance = web_sys::window().unwrap().performance().unwrap();
        assert!(
            (now_ms() - performance.now()).abs() < 50.0,
            "performance.now()"
        );
        let (playing, stream) = tone();
        let chosen = engine(&stream, None).await.expect("an engine");
        let recording = Recording::begun(Clone::clone(&stream), chosen);
        assert!((recording.started_ms() - now_ms()).abs() < 1_000.0);
        let before = recording.elapsed_ms();
        {
            let _later = testing::ClockAhead::by(270_000.0);
            assert!(recording.elapsed_ms() >= before + 270_000.0);
        }
        assert!(recording.elapsed_ms() < before + 1_000.0, "and back");
        recording.cancel();
        let _ = playing.close();
        assert!(can_record(), "a secure page can record");
    }

    /// The meter is the tap's: there with the encoder's worklet, hearing what
    /// it is given — and absent from the browser's own recorder, which hands
    /// nothing over until it stops (S2.9).
    #[wasm_bindgen_test]
    async fn the_meter_is_the_taps_and_the_browsers_recorder_has_none() {
        let (playing, stream) = tone();
        let chosen = engine(&stream, None).await.expect("an engine");
        assert!(matches!(
            chosen,
            Engine::Pcm {
                tap: Tap::Worklet { .. },
                ..
            }
        ));
        let recording = Recording::begun(Clone::clone(&stream), chosen);
        let meter = recording.meter().expect("the worklet's meter");
        assert!(recording.meter() == Some(meter.clone()), "the same one");
        for _ in 0..80 {
            if meter.heard() {
                break;
            }
            sleep(25).await;
        }
        assert!(meter.heard(), "the tone is heard");
        assert!(meter.take_peak() > 0.5, "at the tone's own level");
        recording.cancel();
        let _ = playing.close();

        let _no_encoder = refusing("AudioEncoder");
        let (playing, stream) = tone();
        let chosen = engine(&stream, None).await.expect("an engine");
        assert!(matches!(chosen, Engine::Mp4 { .. }));
        let recording = Recording::begun(Clone::clone(&stream), chosen);
        assert!(recording.meter().is_none());
        recording.cancel();
        let _ = playing.close();
    }

    /// A recording given up — or simply dropped with whatever held it —
    /// lets the microphone go.
    #[wasm_bindgen_test]
    async fn a_cancelled_recording_lets_go_of_the_microphone() {
        let recording = Recording::start(Listening::in_the_click())
            .await
            .expect("recording starts");
        // Rust's clone — a second handle on the SAME stream. `stream.clone()`
        // is the DOM's, which makes a new stream with tracks of its own.
        let stream = Clone::clone(&recording.stream);
        let live = |stream: &MediaStream| {
            stream.get_tracks().iter().any(|track| {
                js_sys::Reflect::get(&track, &JsValue::from_str("readyState"))
                    .ok()
                    .and_then(|state| state.as_string())
                    .as_deref()
                    == Some("live")
            })
        };
        assert!(live(&stream));
        recording.cancel();
        assert!(!live(&stream));
    }
}
