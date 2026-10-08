//! The video-message recorder — Phase 3 of the plan for #79
//! (docs/audio-video-messages-2026-10-04.md, S3, S4, S6, S8.7, S8.8). What it
//! records with is crate::round_video's.
//!
//! It covers the whole window: a `position: fixed; inset: 0` layer, a dialog
//! for screen readers (`role="dialog"`, `aria-modal`, "Video message"), with
//! everything else on the page made `inert` while it is open — a dark scrim,
//! opaque where transparency is reduced (S3.3). Over the conversation pane:
//! a status line in its own capsule, the circle, the reply the video will
//! carry, and a control row exactly where the composer's row is, with the
//! slot in the Send button's place.
//!
//! **Revised 2026-10-06 (decision 41).** On the owner's iPhone the composer
//! row showed through the recorder's controls and the chat competed with the
//! circle. Now the composer is not drawn while the recorder is open
//! (composer.rs, `is-covered`: hidden and inert), the controls stand on their
//! own solid bar, the conversation is blurred under a darker scrim, and the
//! circle is never larger than the room between the status and the controls
//! (styles.css, `.recorder-stage`) — at any window size, text size or banner.
//!
//! **PREVIEW** — the camera on and mirrored, the microphone OFF; Close, the
//! camera choice, "Record a voice message instead", and Record, dimmed until
//! the first frame. **RECORDING** — the microphone opens at Record (asked
//! inside the click, which is what lets its audio run); a red ring fills over
//! the length limit and turns orange ten seconds before it; Delete and Stop.
//! **REVIEW** — the camera off; the clip as it will be sent, not mirrored, a
//! tap or Space playing it; Delete, Retake and Send (S3.4).
//!
//! Return is the slot, Esc closes, stops or asks, and in REVIEW Space plays
//! and pauses wherever the focus is — caught on the way down, before the
//! focused control: Space never sends, deletes or retakes. Tab walks the
//! recorder and never leaves it. Closing gives the focus back to what opened
//! it.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use fc_text::i18n::{t, t1};
use fc_text::media;
use fc_text::record::{self, ACTIVATION_GUARD_MS, DELETE_ASKS_FROM_MS, PREVIEW_IDLE_CLOSE_MS};
use gloo_timers::callback::Interval;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use web_sys::{Element, HtmlElement, HtmlVideoElement, MediaStream, Url};
use yew::prelude::*;

use crate::awake::ScreenAwake;
use crate::encode::LivePicture;
use crate::recorder::{now_ms, Listening};
use crate::round_video::{self, Closed, Facing, Held, Refusal, RoundLimits, Take};
use crate::staged::Prepared;
use crate::views::attach::phone_like;
use crate::views::composer::Replying;
use crate::views::dialog::Confirm;

/// The height a reply banner takes in the recorder (S3.3's arithmetic).
const BANNER: f64 = 40.0;

/// How the recorder lies over a pane `width × height`: its circle's diameter,
/// and whether the controls stand in a column at the trailing edge — a pane
/// shorter than 480, a phone on its side (S3.3). `banner` is the reply
/// banner's height, 0 without one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geometry {
    pub diameter: f64,
    pub column: bool,
}

pub fn geometry(width: f64, height: f64, banner: f64) -> Geometry {
    if height < 480.0 {
        Geometry {
            diameter: 320f64.min(height - 96.0).min(width - 200.0).max(160.0),
            column: true,
        }
    } else {
        Geometry {
            diameter: 320f64
                .min(width - 48.0)
                .min(height - 240.0 - banner)
                .max(160.0),
            column: false,
        }
    }
}

/// Where the recorder is.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Stage {
    /// Reading what the browser has already refused, before asking.
    Opening,
    /// The browser's own prompt is up.
    Asking,
    Refused(Refusal),
    /// Another app has the camera (S3.6).
    Busy,
    /// There was no camera, or no microphone, to open (S3.6): what video
    /// messages need is said and the camera slashed — and a voice message
    /// offered only where the device is known to have a microphone
    /// (`voice`): the missing device may be the microphone itself.
    Missing {
        voice: bool,
    },
    /// Anything else kept the camera from opening.
    Failed,
    Preview,
    Recording {
        started: f64,
    },
    /// Stopped, and the clip being made.
    Finishing,
    Review,
}

/// What "Delete video message?" was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ask {
    /// Delete in REVIEW, Esc, a recording deleted at ten seconds or more:
    /// Delete closes, or goes back to the preview.
    Delete { back_to_preview: bool },
    /// Retake at ten seconds or more.
    Retake,
}

/// What ended a recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ending {
    Person,
    Limit,
    /// A call, a hidden tab, a device gone (S4).
    Interrupted,
}

/// The clip in review.
#[derive(Debug, Clone, PartialEq)]
struct Review {
    prepared: Prepared,
    url: String,
    duration_ms: i64,
    /// How long it was recorded for, by the clock the recording was timed
    /// by — what "Delete asks from ten seconds" goes by (S1.1).
    recorded_ms: f64,
    too_big: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct View {
    stage: Stage,
    /// The camera has shown its first frame.
    first_frame: bool,
    /// An encoder is ready for Record.
    ready: bool,
    /// Two seconds of near black (S3.6).
    black: bool,
    elapsed_ms: f64,
    warned: bool,
    /// A line under the status: "That video was too short.", …
    notice: Option<String>,
    asking: Option<Ask>,
    review: Option<Review>,
    playing: bool,
    played: f64,
    cameras: Vec<(String, String)>,
    choosing: bool,
    /// Said by the recorder's own live region, numbered so the same words
    /// said twice are said twice.
    said: (u32, String),
    /// Moves each time a new stream is shown.
    stream: u32,
    first_time: bool,
}

impl Default for View {
    fn default() -> Self {
        View {
            stage: Stage::Opening,
            first_frame: false,
            ready: false,
            black: false,
            elapsed_ms: 0.0,
            warned: false,
            notice: None,
            asking: None,
            review: None,
            playing: false,
            played: 0.0,
            cameras: Vec::new(),
            choosing: false,
            said: (0, String::new()),
            stream: 0,
            first_time: false,
        }
    }
}

#[derive(Properties, PartialEq)]
pub struct RecorderProps {
    pub limits: RoundLimits,
    /// A call in any phase but ended (S4).
    pub on_call: bool,
    /// The reply the video will carry — the composer's primed reply (S1.5).
    pub replying: Option<Replying>,
    pub on_drop_reply: Callback<()>,
    /// A voice message that was not sent waits: "Record a voice message
    /// instead" is dimmed and says why (S3.4).
    pub not_sent: bool,
    /// "Record a voice message instead" — in the click.
    pub on_voice: Callback<()>,
    /// Send: the clip, and whether it goes as a video message (`round`) or,
    /// too big for one, as a regular video (S3.6).
    pub on_send: Callback<(Prepared, bool)>,
    /// Closed, with what to say once it has.
    pub on_close: Callback<Option<String>>,
    /// The conversation pane it lies over.
    pub pane: NodeRef,
}

/// What lives outside the render: the streams, the encoder made ahead, the
/// take, and what keeps the screen on.
struct Rig {
    view: RefCell<View>,
    set: RefCell<Option<UseStateHandle<View>>>,
    epoch: Cell<u32>,
    stream: RefCell<Option<MediaStream>>,
    warm: RefCell<Option<LivePicture>>,
    take: RefCell<Option<Take>>,
    held: RefCell<Option<Held>>,
    awake: RefCell<Option<ScreenAwake>>,
    preview: NodeRef,
    player: NodeRef,
    facing: Cell<Facing>,
    camera: RefCell<Option<String>>,
    /// The last time a control was used, by `now_ms` (S1.1: the preview
    /// closes after 60 s with none).
    used: Cell<f64>,
    /// Since when the picture has been near black.
    dark_since: Cell<Option<f64>>,
    /// Asked for after the recording stops: Delete at ten seconds or more.
    ask_after: Cell<Option<Ask>>,
    /// When the slot last stopped a recording (S1.1: "Record → Stop → Send
    /// in the recorder" is guarded — the slot's own activation, not Esc's).
    /// The guard after Record is the recording's own clock (`stop`).
    slot_changed: Cell<f64>,
    closed: Cell<bool>,
    on_close: RefCell<Callback<Option<String>>>,
    on_send: RefCell<Callback<(Prepared, bool)>>,
    limits: Cell<RoundLimits>,
    /// The tracks' listeners: a camera or a microphone that went away.
    watching: RefCell<Vec<(web_sys::MediaStreamTrack, Closure<dyn FnMut()>)>>,
}

impl Rig {
    fn update(&self, change: impl FnOnce(&mut View)) {
        let mut view = self.view.borrow_mut();
        change(&mut view);
        if let Some(set) = self.set.borrow().as_ref() {
            set.set(view.clone());
        }
    }

    fn view(&self) -> View {
        self.view.borrow().clone()
    }

    fn stage(&self) -> Stage {
        self.view.borrow().stage
    }

    fn bump(&self) -> u32 {
        self.epoch.set(self.epoch.get() + 1);
        self.epoch.get()
    }

    fn current(&self, epoch: u32) -> bool {
        !self.closed.get() && self.epoch.get() == epoch
    }

    fn touch(&self) {
        self.used.set(now_ms());
    }

    /// Whether the slot is inside its 600 ms guard (S1.1): the second click
    /// of a double click on Stop can neither send the clip nor start a new
    /// recording after "That video was too short.".
    fn guarded(&self) -> bool {
        let since = now_ms() - self.slot_changed.get();
        (0.0..ACTIVATION_GUARD_MS as f64).contains(&since)
    }

    /// Whether the recording running now is under a second old — what an
    /// interruption deletes instead of keeping (S4).
    fn short(&self) -> bool {
        matches!(
            self.stage(),
            Stage::Recording { started } if now_ms() - started < record::SHORTEST_RECORDING_MS as f64
        )
    }

    /// The take let go of — nothing kept, its microphone closed.
    fn drop_take(&self) {
        self.take.borrow_mut().take();
        self.held.borrow_mut().take();
        self.awake.borrow_mut().take();
        self.unwatch_microphone();
    }

    fn say(&self, words: impl Into<String>) {
        let words = words.into();
        self.update(|view| view.said = (view.said.0 + 1, words));
    }

    fn unwatch(&self) {
        for (track, listener) in self.watching.borrow_mut().drain(..) {
            for event in ["ended", "mute"] {
                let _ = track
                    .remove_event_listener_with_callback(event, listener.as_ref().unchecked_ref());
            }
        }
    }

    /// Hear when the camera's — or the microphone's — tracks stop being this
    /// recorder's.
    fn watch(self: &Rc<Self>, stream: &MediaStream, events: &[&'static str], lost: fn(&Rc<Rig>)) {
        let weak = Rc::downgrade(self);
        for track in stream.get_tracks().iter() {
            let track: web_sys::MediaStreamTrack = track.unchecked_into();
            let weak = weak.clone();
            let listener = Closure::<dyn FnMut()>::new(move || {
                let weak = weak.clone();
                spawn_local(async move {
                    if let Some(rig) = weak.upgrade() {
                        lost(&rig);
                    }
                });
            });
            for event in events {
                let _ = track
                    .add_event_listener_with_callback(event, listener.as_ref().unchecked_ref());
            }
            self.watching.borrow_mut().push((track, listener));
        }
    }

    /// The camera off: its light goes out.
    fn camera_off(&self) {
        self.unwatch();
        if let Some(stream) = self.stream.borrow_mut().take() {
            round_video::stop(&stream);
        }
        if let Some(video) = self.preview.cast::<HtmlVideoElement>() {
            video.set_src_object(None);
        }
    }

    /// Everything let go of.
    fn shut(&self) {
        self.bump();
        self.camera_off();
        self.take.borrow_mut().take();
        self.warm.borrow_mut().take();
        self.held.borrow_mut().take();
        self.awake.borrow_mut().take();
        if let Some(review) = self.view.borrow().review.as_ref() {
            let _ = Url::revoke_object_url(&review.url);
        }
    }

    fn close(&self, said: Option<String>) {
        if self.closed.replace(true) {
            return;
        }
        self.shut();
        let on_close = self.on_close.borrow().clone();
        on_close.emit(said);
    }

    /// Open the camera — asking for the microphone with it the first time
    /// (S3.2): while the browser would still prompt for it, or, where it will
    /// not say, until this device has asked once. Never when it is allowed
    /// already, so no microphone indicator contradicts "Not recording"
    /// (S3.4). `first`: the recorder has just opened, and the refusals the
    /// browser already holds are read before anything is asked.
    fn open(self: &Rc<Self>, first: bool) {
        let epoch = self.bump();
        self.touch();
        self.update(|view| {
            *view = View {
                stage: Stage::Opening,
                said: view.said.clone(),
                stream: view.stream,
                cameras: view.cameras.clone(),
                notice: view.notice.clone(),
                ..View::default()
            };
        });
        let this = self.clone();
        spawn_local(async move {
            let mut ask_microphone = false;
            if first {
                let (camera, microphone) = round_video::statuses().await;
                if let Some(refusal) =
                    round_video::refused(camera.as_deref(), microphone.as_deref())
                {
                    if this.current(epoch) {
                        this.update(|view| view.stage = Stage::Refused(refusal));
                    }
                    return;
                }
                if !this.current(epoch) {
                    return;
                }
                ask_microphone = round_video::asks_microphone(
                    microphone.as_deref(),
                    round_video::asked_both_before(),
                );
                this.update(|view| view.stage = Stage::Asking);
            }
            let camera = this.camera.borrow().clone();
            let opened = round_video::open_camera(camera, this.facing.get(), ask_microphone).await;
            if ask_microphone && opened.is_ok() {
                round_video::remember_asked_both();
            }
            if !this.current(epoch) {
                if let Ok(stream) = opened {
                    round_video::stop(&stream);
                }
                return;
            }
            match opened {
                Ok(stream) => this.preview(stream, epoch),
                Err(Closed::Refused(refusal)) => {
                    this.update(|view| view.stage = Stage::Refused(refusal))
                }
                Err(Closed::Busy) => this.update(|view| view.stage = Stage::Busy),
                Err(Closed::Missing) => {
                    let voice = round_video::has_microphone().await;
                    if this.current(epoch) {
                        this.update(|view| view.stage = Stage::Missing { voice });
                    }
                }
                Err(Closed::Failed) => this.update(|view| view.stage = Stage::Failed),
            }
        });
    }

    /// The camera is on: PREVIEW, and an encoder made ready for Record now —
    /// a browser stops the camera's frames for most of a second while it
    /// brings one up, which is better spent before the first frame than
    /// after Record.
    fn preview(self: &Rc<Self>, stream: MediaStream, epoch: u32) {
        self.camera_off();
        self.watch(&stream, &["ended"], Rig::camera_lost);
        *self.stream.borrow_mut() = Some(stream);
        self.touch();
        self.dark_since.set(None);
        let first_time = !round_video::previewed_before();
        self.update(|view| {
            view.stage = Stage::Preview;
            view.first_frame = false;
            view.black = false;
            view.stream += 1;
            view.first_time = first_time;
        });
        self.warm_up(epoch);
        let this = self.clone();
        spawn_local(async move {
            let cameras = round_video::cameras().await;
            if this.current(epoch) {
                this.update(|view| view.cameras = cameras);
            }
        });
    }

    fn warm_up(self: &Rc<Self>, epoch: u32) {
        if self.warm.borrow().is_some() {
            self.update(|view| view.ready = true);
            return;
        }
        let this = self.clone();
        spawn_local(async move {
            let picture = match round_video::picture_config().await {
                Some(config) => LivePicture::start(&config),
                None => None,
            };
            if !this.current(epoch) {
                return;
            }
            match picture {
                Some(picture) => {
                    *this.warm.borrow_mut() = Some(picture);
                    this.update(|view| view.ready = true);
                }
                None => this
                    .update(|view| view.notice = Some(t("Couldn't start recording.").to_string())),
            }
        });
    }

    /// The camera went away, or another app took it (S3.6, S4).
    fn camera_lost(self: &Rc<Self>) {
        match self.stage() {
            // Under a second there is nothing worth keeping (S4): deleted,
            // and the camera sentence over a camera that is off — never a
            // preview of a stream that has ended.
            Stage::Recording { .. } if self.short() => {
                self.bump();
                self.drop_take();
                self.camera_off();
                self.update(|view| {
                    view.stage = Stage::Busy;
                    view.ready = false;
                    view.notice = None;
                });
            }
            Stage::Preview => {
                self.camera_off();
                self.update(|view| view.stage = Stage::Busy);
            }
            Stage::Recording { .. } => {
                self.update(|view| {
                    view.notice = Some(t("The camera is being used by another app.").to_string())
                });
                self.stop(Ending::Interrupted);
            }
            _ => {}
        }
    }

    fn microphone_lost(self: &Rc<Self>) {
        if matches!(self.stage(), Stage::Recording { .. }) {
            self.stop(Ending::Interrupted);
        }
    }

    /// Record — IN the click (S8.8): the audio context is made here, the
    /// microphone asked for, and the take begins with the next frame.
    fn record(self: &Rc<Self>) {
        let view = self.view();
        if view.stage != Stage::Preview || !view.first_frame || !view.ready {
            return;
        }
        let Some(video) = self.preview.cast::<HtmlVideoElement>() else {
            return;
        };
        let Some(picture) = self.warm.borrow_mut().take() else {
            return;
        };
        let listening = Listening::in_the_click();
        // Nothing of the app's plays into a recording (S1.7).
        crate::views::attach::pause_all_playing(Some(video.unchecked_ref()));
        let Some(take) = Take::begin(&video, picture) else {
            self.update(|view| {
                view.ready = false;
                view.notice = Some(t("Couldn't start recording.").to_string());
            });
            return;
        };
        let epoch = self.bump();
        *self.take.borrow_mut() = Some(take);
        *self.held.borrow_mut() = Some(Held::new());
        *self.awake.borrow_mut() = Some(ScreenAwake::hold());
        let started = now_ms();
        self.update(|view| {
            view.stage = Stage::Recording { started };
            view.elapsed_ms = 0.0;
            view.warned = false;
            view.notice = None;
            view.choosing = false;
            view.ready = false;
        });
        self.say(t("Recording video"));
        let this = self.clone();
        spawn_local(async move {
            let opened = round_video::open_microphone().await;
            if !this.current(epoch) {
                if let Ok(microphone) = opened {
                    round_video::stop(&microphone);
                }
                return;
            }
            let microphone = match opened {
                Ok(microphone) => microphone,
                Err(closed) => {
                    this.take.borrow_mut().take();
                    this.held.borrow_mut().take();
                    this.awake.borrow_mut().take();
                    match closed {
                        Closed::Refused(_) => {
                            this.camera_off();
                            this.update(|view| view.stage = Stage::Refused(Refusal::Microphone));
                        }
                        _ => {
                            this.update(|view| {
                                view.stage = Stage::Preview;
                                view.notice = Some(t("Couldn't start recording.").to_string());
                            });
                            this.warm_up(this.epoch.get());
                        }
                    }
                    return;
                }
            };
            let sound = round_video::tap(&microphone, listening).await;
            if !this.current(epoch) {
                round_video::stop(&microphone);
                return;
            }
            match (sound, this.take.borrow().as_ref()) {
                (Some(sound), Some(take)) => {
                    take.hear(sound, Clone::clone(&microphone));
                }
                _ => {
                    round_video::stop(&microphone);
                    return;
                }
            }
            this.watch(&microphone, &["ended", "mute"], Rig::microphone_lost);
        });
    }

    /// Stop: under a second, nothing (back to the preview, "That video was
    /// too short."); otherwise the camera and the microphone off and the
    /// clip made, into REVIEW (S3.4).
    fn stop(self: &Rc<Self>, ending: Ending) {
        let Stage::Recording { started } = self.stage() else {
            return;
        };
        let recorded = now_ms() - started;
        if ending == Ending::Person && recorded < ACTIVATION_GUARD_MS as f64 {
            return;
        }
        let epoch = self.bump();
        if recorded < record::SHORTEST_RECORDING_MS as f64 {
            self.drop_take();
            let words = t("That video was too short.").to_string();
            self.update(|view| {
                view.stage = Stage::Preview;
                view.notice = Some(words.clone());
            });
            self.say(words);
            self.warm_up(epoch);
            return;
        }
        let Some(take) = self.take.borrow_mut().take() else {
            return;
        };
        self.camera_off();
        self.update(|view| view.stage = Stage::Finishing);
        let this = self.clone();
        let limits = self.limits.get();
        spawn_local(async move {
            let clip = take.finish().await;
            let prepared = match clip {
                Some(clip) => Some(crate::prep::round_video(clip.blob, clip.duration_ms).await),
                None => None,
            };
            if !this.current(epoch) {
                return;
            }
            this.awake.borrow_mut().take();
            let Some(prepared) = prepared else {
                this.held.borrow_mut().take();
                let words = t("The recording stopped unexpectedly.").to_string();
                this.update(|view| view.notice = Some(words.clone()));
                this.say(words);
                this.open(false);
                return;
            };
            let url = prepared
                .file
                .as_ref()
                .and_then(|blob| Url::create_object_url_with_blob(blob).ok())
                .unwrap_or_default();
            let duration_ms = prepared.duration_ms.unwrap_or(0);
            let too_big = prepared.size.max(0) as u64 > limits.max_bytes;
            let recorded_ms = recorded.max(duration_ms as f64);
            let asked = this.ask_after.take();
            this.update(|view| {
                view.stage = Stage::Review;
                view.review = Some(Review {
                    prepared,
                    url,
                    duration_ms,
                    recorded_ms,
                    too_big,
                });
                view.playing = false;
                view.played = 0.0;
                view.asking = asked;
                if too_big {
                    view.notice = Some(
                        t("Too big for a video message. It will be sent as a regular video.")
                            .to_string(),
                    );
                }
            });
            if ending == Ending::Limit {
                let words = t("Recording stopped at one minute.").to_string();
                this.update(|view| view.notice = Some(words.clone()));
                this.say(words);
            }
        });
    }

    fn unwatch_microphone(&self) {
        // The take's microphone went with it; its listeners go too.
        self.watching.borrow_mut().retain(|(track, listener)| {
            let audio = track.kind() == "audio";
            if audio {
                for event in ["ended", "mute"] {
                    let _ = track.remove_event_listener_with_callback(
                        event,
                        listener.as_ref().unchecked_ref(),
                    );
                }
            }
            !audio
        });
    }

    /// Delete while recording: under ten seconds at once, back to the
    /// preview; from ten seconds it stops first and asks (S3.4).
    fn delete_recording(self: &Rc<Self>) {
        let Stage::Recording { started } = self.stage() else {
            return;
        };
        if now_ms() - started >= DELETE_ASKS_FROM_MS as f64 {
            self.ask_after.set(Some(Ask::Delete {
                back_to_preview: true,
            }));
            self.stop(Ending::Person);
            return;
        }
        let epoch = self.bump();
        self.drop_take();
        self.update(|view| {
            view.stage = Stage::Preview;
            view.notice = None;
        });
        self.say(t("Recording deleted"));
        self.warm_up(epoch);
    }

    fn long(&self) -> bool {
        self.view
            .borrow()
            .review
            .as_ref()
            .is_some_and(|review| review.recorded_ms >= DELETE_ASKS_FROM_MS as f64)
    }

    fn retake(self: &Rc<Self>) {
        if self.long() {
            self.update(|view| view.asking = Some(Ask::Retake));
        } else {
            self.retake_now();
        }
    }

    fn retake_now(self: &Rc<Self>) {
        self.drop_review();
        self.held.borrow_mut().take();
        self.update(|view| view.notice = None);
        self.open(false);
    }

    fn drop_review(&self) {
        let review = self.view.borrow_mut().review.take();
        if let Some(review) = review {
            let _ = Url::revoke_object_url(&review.url);
        }
        if let Some(player) = self.player.cast::<HtmlVideoElement>() {
            let _ = player.pause();
        }
    }

    fn delete_review(self: &Rc<Self>) {
        if self.long() {
            self.update(|view| {
                view.asking = Some(Ask::Delete {
                    back_to_preview: false,
                })
            });
        } else {
            self.close(None);
        }
    }

    fn answer(self: &Rc<Self>, delete: bool) {
        let asked = self.view.borrow().asking;
        self.update(|view| view.asking = None);
        if !delete {
            return;
        }
        match asked {
            Some(Ask::Delete {
                back_to_preview: true,
            }) => {
                self.drop_review();
                self.held.borrow_mut().take();
                self.update(|view| view.notice = None);
                self.say(t("Recording deleted"));
                self.open(false);
            }
            Some(Ask::Delete { .. }) => self.close(None),
            Some(Ask::Retake) => self.retake_now(),
            None => {}
        }
    }

    fn send(self: &Rc<Self>) {
        let Some(review) = self.view.borrow().review.clone() else {
            return;
        };
        if let Some(player) = self.player.cast::<HtmlVideoElement>() {
            let _ = player.pause();
        }
        let on_send = self.on_send.borrow().clone();
        on_send.emit((review.prepared, !review.too_big));
        self.close(Some(t("Video message sent").to_string()));
    }

    fn toggle_play(&self) {
        let Some(player) = self.player.cast::<HtmlVideoElement>() else {
            return;
        };
        if player.paused() {
            if player.ended() {
                player.set_current_time(0.0);
            }
            let _ = player.play();
        } else {
            let _ = player.pause();
        }
    }

    /// What the slot does, stage by stage (S3.4).
    fn slot(self: &Rc<Self>) {
        self.touch();
        if self.guarded() {
            return;
        }
        match self.stage() {
            Stage::Preview => self.record(),
            Stage::Recording { .. } => {
                self.stop(Ending::Person);
                if !matches!(self.stage(), Stage::Recording { .. }) {
                    self.slot_changed.set(now_ms());
                }
            }
            Stage::Review => self.send(),
            _ => {}
        }
    }

    /// Esc (S3.4): Close, Stop, or "Delete video message?".
    fn escape(self: &Rc<Self>) {
        self.touch();
        match self.stage() {
            Stage::Recording { .. } => self.stop(Ending::Person),
            Stage::Finishing => {}
            Stage::Review => self.update(|view| {
                view.asking = Some(Ask::Delete {
                    back_to_preview: false,
                })
            }),
            _ => self.close(None),
        }
    }

    /// Something other than the person (S4): a call, a hidden tab. The
    /// preview closes; a recording stops into REVIEW; REVIEW is kept.
    fn interrupt(self: &Rc<Self>) {
        match self.stage() {
            // Under a second it is deleted, and the preview it would go back
            // to closes as a preview does (S4).
            Stage::Recording { .. } if self.short() => self.close(None),
            Stage::Recording { .. } => self.stop(Ending::Interrupted),
            Stage::Finishing | Stage::Review => {}
            _ => self.close(None),
        }
    }

    /// Once a tick: the timer, the limit and its warning while recording;
    /// the idle preview's 60 s; the picture that stays black.
    fn tick(self: &Rc<Self>) {
        match self.stage() {
            Stage::Recording { started } => {
                let elapsed = now_ms() - started;
                let limits = self.limits.get();
                if elapsed >= limits.cap_ms() as f64 {
                    self.stop(Ending::Limit);
                    return;
                }
                let warn = elapsed >= limits.warning_ms() as f64 && !self.view.borrow().warned;
                self.update(|view| {
                    view.elapsed_ms = elapsed;
                    view.warned |= warn;
                });
                if warn {
                    self.say(t("10 seconds left"));
                }
                if self.take.borrow().as_ref().is_some_and(Take::failed) {
                    self.stop(Ending::Interrupted);
                }
            }
            Stage::Preview => {
                if now_ms() - self.used.get() >= PREVIEW_IDLE_CLOSE_MS as f64 {
                    self.close(Some(t("Camera turned off").to_string()));
                    return;
                }
                let Some(video) = self.preview.cast::<HtmlVideoElement>() else {
                    return;
                };
                match round_video::shows_black(&video) {
                    Some(true) => {
                        let since = self.dark_since.get().unwrap_or_else(now_ms);
                        self.dark_since.set(Some(since));
                        if now_ms() - since >= 2_000.0 && !self.view.borrow().black {
                            self.update(|view| view.black = true);
                        }
                    }
                    Some(false) => {
                        self.dark_since.set(None);
                        if self.view.borrow().black {
                            self.update(|view| view.black = false);
                        }
                    }
                    None => {}
                }
            }
            _ => {}
        }
    }
}

/// Every child of the page's body but `keep`, made inert — and those it
/// made so, to give back.
fn make_inert(keep: &Element) -> Vec<Element> {
    let mut made = Vec::new();
    let Some(body) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.body())
    else {
        return made;
    };
    let children = body.children();
    for index in 0..children.length() {
        let Some(child) = children.item(index) else {
            continue;
        };
        if &child == keep || child.has_attribute("inert") {
            continue;
        }
        if child.set_attribute("inert", "").is_ok() {
            made.push(child);
        }
    }
    made
}

/// What should have the focus back when the recorder closes: the control
/// that opened it — or, where that is drawn anew by then (the video button
/// goes while the recorder owns the row), the one in its place; and for an
/// item of a menu that has closed since, the button the menu belongs to.
#[derive(Clone)]
struct Opener {
    element: Option<HtmlElement>,
    /// Where to find it again.
    selector: &'static str,
}

impl Opener {
    fn now() -> Opener {
        let active = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.active_element());
        let within = |selector: &str| {
            active
                .as_ref()
                .and_then(|active| active.closest(selector).ok().flatten())
                .is_some()
        };
        let selector = if within(".video-door") {
            ".composer .video-door"
        } else if within(".attach") {
            ".attach > button.tool"
        } else if within(".slot-wrap") {
            ".composer .slot"
        } else {
            ".composer textarea"
        };
        let in_menu = within("[role=menu]");
        Opener {
            element: active
                .filter(|_| !in_menu)
                .and_then(|active| active.dyn_into().ok()),
            selector,
        }
    }

    /// Give it the focus back — once the page is drawn without the recorder.
    fn focus(self) {
        spawn_local(async move {
            gloo_timers::future::TimeoutFuture::new(0).await;
            let found = self
                .element
                .filter(|element| element.is_connected())
                .or_else(|| {
                    web_sys::window()?
                        .document()?
                        .query_selector(self.selector)
                        .ok()
                        .flatten()?
                        .dyn_into()
                        .ok()
                });
            if let Some(found) = found {
                let _ = found.focus();
            }
        });
    }
}

const FOCUSABLE: &str = "button:not([disabled]), [tabindex]:not([tabindex='-1']), video[controls]";

fn focusables(layer: &Element) -> Vec<HtmlElement> {
    let Ok(found) = layer.query_selector_all(FOCUSABLE) else {
        return Vec::new();
    };
    (0..found.length())
        .filter_map(|index| found.item(index))
        .filter_map(|node| node.dyn_into::<HtmlElement>().ok())
        .filter(|element| element.offset_parent().is_some() || element.class_name() == "recorder")
        .collect()
}

/// The rectangle a recorder measures itself against: the pane, and the
/// composer row in it, in window coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct Frame {
    left: f64,
    top: f64,
    width: f64,
    height: f64,
    row_height: f64,
    row_bottom: f64,
}

fn measure(pane: &NodeRef) -> Frame {
    let window = web_sys::window();
    let (window_width, window_height) = window
        .as_ref()
        .map(|window| {
            (
                window
                    .inner_width()
                    .ok()
                    .and_then(|width| width.as_f64())
                    .unwrap_or(0.0),
                window
                    .inner_height()
                    .ok()
                    .and_then(|height| height.as_f64())
                    .unwrap_or(0.0),
            )
        })
        .unwrap_or_default();
    let Some(pane) = pane.cast::<Element>() else {
        return Frame {
            left: 0.0,
            top: 0.0,
            width: window_width,
            height: window_height,
            row_height: 64.0,
            row_bottom: 0.0,
        };
    };
    let rect = pane.get_bounding_client_rect();
    let row = pane
        .query_selector(".composer")
        .ok()
        .flatten()
        .map(|row| row.get_bounding_client_rect());
    Frame {
        left: rect.left(),
        top: rect.top(),
        width: rect.width(),
        height: rect.height(),
        row_height: row.as_ref().map_or(64.0, |row| row.height().max(44.0)),
        row_bottom: row
            .as_ref()
            .map_or(0.0, |row| (rect.bottom() - row.bottom()).max(0.0)),
    }
}

fn reduced_motion() -> bool {
    web_sys::window()
        .and_then(|window| {
            window
                .match_media("(prefers-reduced-motion: reduce)")
                .ok()
                .flatten()
        })
        .is_some_and(|query| query.matches())
}

/// The recorder's glyphs, drawn inline.
fn glyph(path: &'static str) -> Html {
    html! {
        <svg class="recorder-icon" viewBox="0 0 24 24" aria-hidden="true" focusable="false">
            <path fill="currentColor" d={path} />
        </svg>
    }
}

const CLOSE: &str = "M18.3 5.71 12 12.01 5.7 5.7 4.29 7.11 10.59 13.4l-6.3 6.3 1.41 1.41 6.3-6.29 6.29 6.29 1.42-1.41-6.3-6.3 6.3-6.29z";
const TRASH: &str = "M6 19a2 2 0 0 0 2 2h8a2 2 0 0 0 2-2V7H6v12zM19 4h-3.5l-1-1h-5l-1 1H5v2h14V4z";
const RETAKE: &str = "M12 5V1L7 6l5 5V7a6 6 0 1 1-6 6H4a8 8 0 1 0 8-8z";
const SEND: &str = "M12 4 5 11l1.41 1.41L11 7.83V20h2V7.83l4.59 4.58L19 11z";
const MICROPHONE: &str = "M12 14a3 3 0 0 0 3-3V5a3 3 0 0 0-6 0v6a3 3 0 0 0 3 3zm5-3a5 5 0 0 1-10 0H5a7 7 0 0 0 6 6.92V21h2v-3.08A7 7 0 0 0 19 11z";
const SWITCH: &str = "M20 5h-3.17L15 3H9L7.17 5H4a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2V7a2 2 0 0 0-2-2zm-5 11.5V14H9v2.5L5.5 13 9 9.5V12h6V9.5l3.5 3.5-3.5 3.5z";
const NO_CAMERA: &str = "M21 6.5l-4 4V7a1 1 0 0 0-1-1H9.82L21 17.18V6.5zM3.27 2 2 3.27 4.73 6H4a1 1 0 0 0-1 1v10a1 1 0 0 0 1 1h12c.21 0 .39-.08.54-.18L19.73 21 21 19.73 3.27 2z";

/// The recorder.
#[function_component(RoundRecorder)]
pub fn round_recorder(props: &RecorderProps) -> Html {
    let view = use_state(View::default);
    let preview = use_node_ref();
    let player = use_node_ref();
    let layer = use_node_ref();
    let slot = use_node_ref();
    let rig: Rc<Rig> = {
        let preview = preview.clone();
        let player = player.clone();
        let limits = props.limits;
        let made = use_memo((), move |_| {
            Rc::new(Rig {
                view: RefCell::new(View::default()),
                set: RefCell::new(None),
                epoch: Cell::new(0),
                stream: RefCell::new(None),
                warm: RefCell::new(None),
                take: RefCell::new(None),
                held: RefCell::new(None),
                awake: RefCell::new(None),
                preview,
                player,
                facing: Cell::new(Facing::User),
                camera: RefCell::new(round_video::chosen_camera()),
                used: Cell::new(now_ms()),
                dark_since: Cell::new(None),
                ask_after: Cell::new(None),
                slot_changed: Cell::new(f64::NEG_INFINITY),
                closed: Cell::new(false),
                on_close: RefCell::new(Callback::noop()),
                on_send: RefCell::new(Callback::noop()),
                limits: Cell::new(limits),
                watching: RefCell::new(Vec::new()),
            })
        });
        (*made).clone()
    };
    *rig.set.borrow_mut() = Some(view.clone());
    *rig.on_close.borrow_mut() = props.on_close.clone();
    *rig.on_send.borrow_mut() = props.on_send.clone();
    rig.limits.set(props.limits);

    // Where it is drawn: a layer of its own on the page's body, so that the
    // rest of the page can be made inert around it.
    let host = use_memo((), |_| {
        let document = web_sys::window()?.document()?;
        let host = document.create_element("div").ok()?;
        host.set_class_name("recorder-host");
        document.body()?.append_child(&host).ok()?;
        Some(host)
    });
    let frame = use_state(|| measure(&props.pane));

    // Opened: the camera asked for, the rest of the page inert, the focus on
    // the slot. Closed — or gone with the pane — everything let go of, the
    // page given back and the focus with it.
    {
        let rig = rig.clone();
        let host = host.clone();
        let slot = slot.clone();
        use_effect_with((), move |_| {
            let back = Opener::now();
            let open = round_video::Open::now();
            rig.open(true);
            if let Some(slot) = slot.cast::<HtmlElement>() {
                let _ = slot.focus();
            }
            move || {
                rig.closed.set(true);
                rig.shut();
                if let Some(host) = (*host).as_ref() {
                    host.remove();
                }
                back.focus();
                // A notification tapped meanwhile opens its chat now (S4).
                drop(open);
            }
        });
    }
    // The rest of the page inert while it is open — except during a call,
    // whose bar must be answerable: the recorder steps aside under it,
    // still in REVIEW when the call ends (S4).
    {
        let host = host.clone();
        use_effect_with(props.on_call, move |on_call| {
            let made = if *on_call {
                Vec::new()
            } else {
                (*host).as_ref().map(make_inert).unwrap_or_default()
            };
            move || {
                for element in made {
                    let _ = element.remove_attribute("inert");
                }
            }
        });
    }
    // The camera's stream into the preview, each time there is a new one.
    {
        let rig = rig.clone();
        use_effect_with(view.stream, move |_| {
            if let (Some(video), Some(stream)) = (
                rig.preview.cast::<HtmlVideoElement>(),
                rig.stream.borrow().as_ref(),
            ) {
                video.set_muted(true);
                video.set_src_object(Some(stream));
                let _ = video.play();
            }
        });
    }
    // A call (S4).
    {
        let rig = rig.clone();
        use_effect_with(props.on_call, move |on_call| {
            if *on_call {
                rig.interrupt();
            }
        });
    }
    // A hidden tab; a desktop window that loses focus closes the preview —
    // but not to the permission prompt the recorder raised (S3.4, S4); and a
    // window resized, measured again unless a recording runs (S3.3).
    {
        let rig = rig.clone();
        let frame = frame.clone();
        let pane = props.pane.clone();
        use_effect_with((), move |_| {
            let hidden = {
                let rig = rig.clone();
                Closure::<dyn Fn()>::new(move || {
                    if !crate::sync::page_visible() {
                        rig.interrupt();
                    }
                })
            };
            let blurred = {
                let rig = rig.clone();
                Closure::<dyn Fn()>::new(move || {
                    if phone_like() {
                        return;
                    }
                    let rig = rig.clone();
                    // Focus moving within the page blurs nothing; a window
                    // that is no longer in front still says so a moment on.
                    spawn_local(async move {
                        gloo_timers::future::TimeoutFuture::new(100).await;
                        let away = web_sys::window()
                            .and_then(|window| window.document())
                            .is_some_and(|document| !document.has_focus().unwrap_or(true));
                        if away && matches!(rig.stage(), Stage::Preview | Stage::Busy) {
                            rig.close(None);
                        }
                    });
                })
            };
            let resized = {
                let rig = rig.clone();
                Closure::<dyn Fn()>::new(move || {
                    if !matches!(rig.stage(), Stage::Recording { .. }) {
                        frame.set(measure(&pane));
                    }
                })
            };
            let window = web_sys::window();
            let document = window.as_ref().and_then(|window| window.document());
            if let Some(document) = &document {
                let _ = document.add_event_listener_with_callback(
                    "visibilitychange",
                    hidden.as_ref().unchecked_ref(),
                );
            }
            if let Some(window) = &window {
                let _ = window
                    .add_event_listener_with_callback("blur", blurred.as_ref().unchecked_ref());
                let _ = window
                    .add_event_listener_with_callback("resize", resized.as_ref().unchecked_ref());
            }
            move || {
                if let Some(document) = document {
                    let _ = document.remove_event_listener_with_callback(
                        "visibilitychange",
                        hidden.as_ref().unchecked_ref(),
                    );
                }
                if let Some(window) = window {
                    let _ = window.remove_event_listener_with_callback(
                        "blur",
                        blurred.as_ref().unchecked_ref(),
                    );
                    let _ = window.remove_event_listener_with_callback(
                        "resize",
                        resized.as_ref().unchecked_ref(),
                    );
                }
            }
        });
    }
    // The clock: a tenth of a second while recording, a second otherwise.
    {
        let rig = rig.clone();
        let recording = matches!(view.stage, Stage::Recording { .. });
        use_effect_with(recording, move |recording| {
            let every = if *recording { 100 } else { 500 };
            let interval = Interval::new(every, move || rig.tick());
            move || drop(interval)
        });
    }
    // REVIEW: Space plays and pauses WHEREVER the focus is — caught on the
    // way down, before the focused control can take it, and its keyup too,
    // on which a button would click (S3.4).
    {
        let rig = rig.clone();
        let layer = layer.clone();
        let review = view.stage == Stage::Review && view.asking.is_none();
        use_effect_with(review, move |review| {
            let listener = review.then(|| {
                let rig = rig.clone();
                Closure::<dyn Fn(web_sys::KeyboardEvent)>::new(
                    move |event: web_sys::KeyboardEvent| {
                        if event.key() != " " {
                            return;
                        }
                        event.prevent_default();
                        event.stop_propagation();
                        if event.type_() == "keydown" && !event.repeat() {
                            rig.touch();
                            rig.toggle_play();
                        }
                    },
                )
            });
            let element = layer.cast::<Element>();
            if let (Some(element), Some(listener)) = (&element, &listener) {
                for kind in ["keydown", "keyup"] {
                    let _ = element.add_event_listener_with_callback_and_bool(
                        kind,
                        listener.as_ref().unchecked_ref(),
                        true,
                    );
                }
            }
            move || {
                if let (Some(element), Some(listener)) = (element, listener) {
                    for kind in ["keydown", "keyup"] {
                        let _ = element.remove_event_listener_with_callback_and_bool(
                            kind,
                            listener.as_ref().unchecked_ref(),
                            true,
                        );
                    }
                }
            }
        });
    }

    let Some(host) = (*host).clone() else {
        return Html::default();
    };
    let reduced = reduced_motion();
    let limits = props.limits;
    let banner = if props.replying.is_some() {
        BANNER
    } else {
        0.0
    };
    let shape = geometry(frame.width, frame.height, banner);
    let diameter = shape.diameter;

    // --- what is said and shown --------------------------------------------
    let stage = view.stage;
    let refused_camera = matches!(stage, Stage::Refused(Refusal::Camera | Refusal::Both));
    let status: Html = match stage {
        Stage::Opening | Stage::Asking | Stage::Missing { .. } => {
            html! { <span>{ t("Video messages need the camera and the microphone.") }</span> }
        }
        Stage::Refused(refusal) => html! { <span>{ refusal.sentence() }</span> },
        Stage::Busy => html! { <span>{ t("The camera is being used by another app.") }</span> },
        Stage::Failed => html! { <span>{ t("Couldn't start recording.") }</span> },
        Stage::Preview if !view.first_frame => html! { <span>{ t("Starting camera…") }</span> },
        Stage::Preview => html! { <span>{ t("Not recording") }</span> },
        Stage::Recording { .. } => html! {
            <>
                <span class="recorder-dot" aria-hidden="true"></span>
                <span class="recorder-time" aria-live="off">
                    { media::time_label((view.elapsed_ms / 1000.0).floor()) }
                </span>
                if view.warned {
                    <span class="recorder-left">{ t("10 seconds left") }</span>
                }
            </>
        },
        Stage::Finishing => html! { <span>{ t("Preparing…") }</span> },
        Stage::Review => html! {
            <span>{ t1(
                "Video message · %@",
                &media::time_label(
                    view.review.as_ref().map_or(0, |review| review.duration_ms) as f64 / 1000.0,
                ),
            ) }</span>
        },
    };
    let mut lines: Vec<String> = Vec::new();
    if stage == Stage::Preview && view.first_time {
        lines.push(t("Only you can see this until you start recording.").to_string());
    }
    if stage == Stage::Preview && view.black {
        lines.push(t("We can't see anything. Is the camera turned off or covered?").to_string());
    }
    if let Some(notice) = &view.notice {
        lines.push(notice.clone());
    }

    // --- the ring ----------------------------------------------------------
    let ring = match stage {
        Stage::Preview => Some(("is-preview", None)),
        Stage::Recording { .. } => Some((
            if view.warned {
                "is-warning"
            } else {
                "is-recording"
            },
            Some(
                crate::views::round_tile::ring_progress(
                    view.elapsed_ms / 1000.0,
                    limits.cap_ms() as f64 / 1000.0,
                    reduced,
                ) * 100.0,
            ),
        )),
        Stage::Finishing => Some(("is-preview", None)),
        Stage::Review if view.playing || view.played > 0.0 => Some((
            "is-progress",
            Some(
                crate::views::round_tile::ring_progress(
                    view.played,
                    view.review
                        .as_ref()
                        .map_or(0.0, |review| review.duration_ms as f64 / 1000.0),
                    reduced,
                ) * 100.0,
            ),
        )),
        Stage::Review => Some(("is-review", None)),
        _ => None,
    };
    let ring_size = diameter + 12.0;

    // --- the controls ------------------------------------------------------
    let act = |run: fn(&Rc<Rig>)| {
        let rig = rig.clone();
        Callback::from(move |event: MouseEvent| {
            event.stop_propagation();
            rig.touch();
            run(&rig);
        })
    };
    let ready = stage == Stage::Preview && view.first_frame && view.ready;
    let (slot_label, slot_class, slot_path, slot_dimmed) = match stage {
        Stage::Recording { .. } => (t("Stop recording"), "is-stop", None, false),
        Stage::Finishing => (t("Stop recording"), "is-stop", None, true),
        Stage::Review => (t("Send video message"), "is-send", Some(SEND), false),
        _ => (t("Record"), "is-record", None, !ready),
    };
    let voice_reason = props
        .not_sent
        .then(|| t(record::Dimmed::NotSent.notice()).to_string());
    let offers_voice = matches!(
        stage,
        Stage::Preview
            | Stage::Busy
            | Stage::Missing { voice: true }
            | Stage::Refused(Refusal::Camera)
    );
    let several = view.cameras.len() > 1;
    let voice = {
        let rig = rig.clone();
        let on_voice = props.on_voice.clone();
        let reason = voice_reason.clone();
        Callback::from(move |event: MouseEvent| {
            event.stop_propagation();
            rig.touch();
            if let Some(reason) = &reason {
                rig.update(|view| view.notice = Some(reason.clone()));
                return;
            }
            rig.close(None);
            on_voice.emit(());
        })
    };
    let switch = {
        let rig = rig.clone();
        Callback::from(move |event: MouseEvent| {
            event.stop_propagation();
            rig.touch();
            rig.facing.set(rig.facing.get().other());
            *rig.camera.borrow_mut() = None;
            rig.open(false);
        })
    };
    let choose = {
        let rig = rig.clone();
        Callback::from(move |id: String| {
            rig.touch();
            round_video::choose_camera(&id);
            *rig.camera.borrow_mut() = Some(id);
            rig.update(|view| view.choosing = false);
            rig.open(false);
        })
    };
    let on_key = {
        let rig = rig.clone();
        let layer = layer.clone();
        Callback::from(move |event: KeyboardEvent| {
            // As it is now, not as it was drawn: a key can come before the
            // page has caught up with the last change.
            let (asking, choosing) = {
                let view = rig.view.borrow();
                (view.asking.is_some(), view.choosing)
            };
            if asking {
                return;
            }
            match event.key().as_str() {
                "Escape" => {
                    event.prevent_default();
                    if choosing {
                        rig.update(|view| view.choosing = false);
                    } else {
                        rig.escape();
                    }
                }
                "Enter" => {
                    // A focused control answers its own Return; anywhere
                    // else in the recorder, Return is the slot (S6).
                    let on_control = event
                        .target()
                        .and_then(|target| target.dyn_into::<Element>().ok())
                        .is_some_and(|target| {
                            target
                                .closest("button, [role=menuitem]")
                                .ok()
                                .flatten()
                                .is_some()
                        });
                    if !on_control && !event.repeat() {
                        event.prevent_default();
                        rig.slot();
                    }
                }
                "Tab" => {
                    // Round the recorder, never out of it.
                    let Some(layer) = layer.cast::<Element>() else {
                        return;
                    };
                    let all = focusables(&layer);
                    let (Some(first), Some(last)) = (all.first(), all.last()) else {
                        return;
                    };
                    let active = web_sys::window()
                        .and_then(|window| window.document())
                        .and_then(|document| document.active_element());
                    let at_edge = |element: &HtmlElement| {
                        active
                            .as_ref()
                            .is_some_and(|active| active == element.unchecked_ref::<Element>())
                    };
                    let inside = active
                        .as_ref()
                        .is_some_and(|active| layer.contains(Some(active.unchecked_ref())));
                    if event.shift_key() && (at_edge(first) || !inside) {
                        event.prevent_default();
                        let _ = last.focus();
                    } else if !event.shift_key() && (at_edge(last) || !inside) {
                        event.prevent_default();
                        let _ = first.focus();
                    }
                }
                _ => {}
            }
        })
    };
    let on_slot = {
        let rig = rig.clone();
        Callback::from(move |event: MouseEvent| {
            event.stop_propagation();
            rig.slot();
        })
    };
    let on_circle = {
        let rig = rig.clone();
        Callback::from(move |_: MouseEvent| {
            rig.touch();
            rig.toggle_play();
        })
    };
    let on_frame = {
        let rig = rig.clone();
        Callback::from(move |_: Event| {
            if rig.stage() == Stage::Preview && !rig.view.borrow().first_frame {
                rig.update(|view| view.first_frame = true);
                rig.say(t("Camera ready"));
            }
        })
    };
    let on_played = {
        let rig = rig.clone();
        Callback::from(move |event: Event| {
            let Some(video) = event
                .target()
                .and_then(|target| target.dyn_into::<HtmlVideoElement>().ok())
            else {
                return;
            };
            let playing = !video.paused() && !video.ended();
            let at = if video.ended() {
                0.0
            } else {
                video.current_time()
            };
            // On a phone's browser the screen stays on while it plays (S1.7).
            if playing && phone_like() {
                if rig.awake.borrow().is_none() {
                    *rig.awake.borrow_mut() = Some(ScreenAwake::hold());
                }
            } else if !playing {
                rig.awake.borrow_mut().take();
            }
            rig.update(|view| {
                view.playing = playing;
                view.played = at;
            });
        })
    };

    let column = shape.column;
    let style = format!(
        "--pane-left:{}px;--pane-top:{}px;--pane-width:{}px;--pane-height:{}px;\
         --row-height:{}px;--row-bottom:{}px;--circle:{}px",
        frame.left,
        frame.top,
        frame.width,
        frame.height,
        frame.row_height,
        frame.row_bottom,
        diameter
    );
    let review = view.review.clone();
    // Every control is a round button with its word under it (the approved
    // design): the word for the eye, the button's own label for a screen
    // reader, which hears it once.
    let captioned = |button: Html, caption: &str| {
        html! {
            <div class="recorder-item">
                { button }
                <span class="recorder-caption" aria-hidden="true">{ caption.to_string() }</span>
            </div>
        }
    };
    let leading = match stage {
        Stage::Recording { .. } | Stage::Finishing => captioned(
            html! {
                <button type="button" class="recorder-control" aria-label={t("Delete recording")}
                        title={t("Delete recording")} disabled={stage == Stage::Finishing}
                        onclick={act(Rig::delete_recording)}>
                    { glyph(TRASH) }
                </button>
            },
            t("Delete"),
        ),
        Stage::Review => captioned(
            html! {
                <button type="button" class="recorder-control" aria-label={t("Delete")}
                        title={t("Delete")} onclick={act(Rig::delete_review)}>
                    { glyph(TRASH) }
                </button>
            },
            t("Delete"),
        ),
        _ => captioned(
            html! {
                <button type="button" class="recorder-control" aria-label={t("Close")}
                        title={t("Close")} onclick={act(|rig| rig.close(None))}>
                    { glyph(CLOSE) }
                </button>
            },
            t("Close"),
        ),
    };
    let voice_id = "recorder-voice-reason";
    let middle = html! {
        <>
            if stage == Stage::Review {
                { captioned(
                    html! {
                        <button type="button" class="recorder-control recorder-retake" aria-label={t("Retake")}
                                title={t("Retake")} onclick={act(Rig::retake)}>
                            { glyph(RETAKE) }
                        </button>
                    },
                    t("Retake"),
                ) }
            }
            if stage == Stage::Preview && several {
                if phone_like() {
                    { captioned(
                        html! {
                            <button type="button" class="recorder-control" aria-label={t("Switch camera")}
                                    title={t("Switch camera")} onclick={switch}>
                                { glyph(SWITCH) }
                            </button>
                        },
                        t("Switch"),
                    ) }
                } else {
                    <div class="recorder-item recorder-choose">
                        <button type="button" class="recorder-control" aria-label={t("Choose camera")}
                                title={t("Choose camera")} aria-haspopup="menu"
                                aria-expanded={view.choosing.to_string()}
                                onclick={{
                                    let rig = rig.clone();
                                    Callback::from(move |event: MouseEvent| {
                                        event.stop_propagation();
                                        rig.touch();
                                        rig.update(|view| view.choosing = !view.choosing);
                                    })
                                }}>
                            { glyph(SWITCH) }
                        </button>
                        <span class="recorder-caption" aria-hidden="true">{ t("Camera") }</span>
                        if view.choosing {
                            <div class="menu recorder-cameras" role="menu">
                                { for view.cameras.iter().enumerate().map(|(index, (id, name))| {
                                    let choose = choose.clone();
                                    let id = id.clone();
                                    let name = if name.is_empty() { format!("{} {}", t("Camera"), index + 1) } else { name.clone() };
                                    html! {
                                        <button role="menuitem" onclick={Callback::from(move |_: MouseEvent| choose.emit(id.clone()))}>
                                            { name }
                                        </button>
                                    }
                                }) }
                            </div>
                        }
                    </div>
                }
            }
            if offers_voice {
                { captioned(
                    html! {
                        <button type="button"
                                class={classes!("recorder-control", "recorder-voice", voice_reason.is_some().then_some("is-dimmed"))}
                                aria-label={t("Record a voice message instead")}
                                title={t("Record a voice message instead")}
                                aria-disabled={voice_reason.is_some().then_some("true")}
                                aria-describedby={voice_reason.is_some().then_some(voice_id)}
                                onclick={voice}>
                            { glyph(MICROPHONE) }
                        </button>
                    },
                    t("Voice message"),
                ) }
                if let Some(reason) = voice_reason.clone() {
                    <span id={voice_id} hidden=true>{ reason }</span>
                }
            }
        </>
    };
    let slot_reason_id = "recorder-slot-reason";
    let slot_reason = match stage {
        Stage::Preview if !view.first_frame || !view.ready => {
            Some(t("Starting camera…").to_string())
        }
        _ => None,
    };
    // ONE big slot (the approved design): a red disc to Record, a red
    // rounded square to Stop, the accent arrow to Send.
    let slot_caption = match stage {
        Stage::Recording { .. } | Stage::Finishing => t("Stop"),
        Stage::Review => t("Send"),
        _ => t("Record"),
    };
    let slot_html = html! {
        <div class="recorder-item recorder-slot-item">
            <button
                ref={slot.clone()}
                type="button"
                class={classes!("recorder-slot", slot_class, slot_dimmed.then_some("is-dimmed"))}
                aria-label={slot_label}
                title={slot_label}
                aria-disabled={slot_dimmed.then_some("true")}
                aria-describedby={slot_reason.is_some().then_some(slot_reason_id)}
                onclick={on_slot}
            >
                if let Some(path) = slot_path {
                    { glyph(path) }
                } else {
                    <span class="recorder-slot-mark" aria-hidden="true"></span>
                }
                if let Some(reason) = slot_reason {
                    <span id={slot_reason_id} hidden=true>{ reason }</span>
                }
            </button>
            <span class="recorder-caption" aria-hidden="true">{ slot_caption }</span>
        </div>
    };
    let circle = html! {
        <div class={classes!("recorder-circle", (stage == Stage::Review).then_some("is-review"))}>
            if matches!(stage, Stage::Preview | Stage::Recording { .. }) {
                <video ref={preview.clone()} class="recorder-preview" muted=true autoplay=true
                       playsinline=true aria-hidden="true"
                       onloadeddata={on_frame.clone()} onplaying={on_frame} />
            } else if let Some(review) = review.as_ref().filter(|_| stage == Stage::Review) {
                <video ref={player.clone()} class="recorder-review" src={review.url.clone()}
                       preload="auto" playsinline=true data-playback="true"
                       onplay={on_played.clone()} onpause={on_played.clone()}
                       onended={on_played.clone()} ontimeupdate={on_played} />
                <button type="button" class="recorder-play" onclick={on_circle}
                        aria-label={if view.playing { t("Pause") } else { t("Play") }}
                        aria-pressed={if view.playing { "true" } else { "false" }}>
                    if !view.playing {
                        <span class="round-play" aria-hidden="true">{ "▶" }</span>
                    }
                </button>
            } else if refused_camera || matches!(stage, Stage::Busy | Stage::Missing { .. }) {
                <span class="recorder-glyph" aria-hidden="true">{ glyph(NO_CAMERA) }</span>
            }
            // A thin track ring OUTSIDE the circle, and on it the arc: red
            // over the minute while it records, the accent while the clip
            // plays in review (the approved design).
            if let Some((kind, length)) = ring {
                <svg class={classes!("recorder-ring", kind)} aria-hidden="true"
                     width={ring_size.to_string()} height={ring_size.to_string()}
                     viewBox={format!("0 0 {ring_size} {ring_size}")}>
                    <circle class="recorder-track" vector-effect="non-scaling-stroke"
                            cx={(ring_size / 2.0).to_string()} cy={(ring_size / 2.0).to_string()}
                            r={(diameter / 2.0 + 3.0).to_string()} />
                    if let Some(length) = length {
                        <circle class="recorder-arc" vector-effect="non-scaling-stroke"
                                cx={(ring_size / 2.0).to_string()} cy={(ring_size / 2.0).to_string()}
                                r={(diameter / 2.0 + 3.0).to_string()} pathLength="100"
                                stroke-dasharray={format!("{length:.2} 100")}
                                transform={format!("rotate(-90 {} {})", ring_size / 2.0, ring_size / 2.0)} />
                    }
                </svg>
            }
        </div>
    };
    let banner_html = props.replying.as_ref().map(|reply| {
        let drop = {
            let on_drop_reply = props.on_drop_reply.clone();
            let rig = rig.clone();
            Callback::from(move |_: MouseEvent| {
                rig.touch();
                on_drop_reply.emit(());
            })
        };
        html! {
            <div class="recorder-banner">
                <span class="banner-text">
                    { t1("Replying to %@", &reply.name) }
                    if !reply.excerpt.is_empty() { { format!(": {}", reply.excerpt) } }
                </span>
                <button type="button" class="recorder-banner-drop" onclick={drop} aria-label={t("Cancel reply")}>{ "✕" }</button>
            </div>
        }
    });
    let dialog = html! {
        <div
            ref={layer.clone()}
            class={classes!("recorder", column.then_some("is-column"))}
            role="dialog"
            aria-modal="true"
            aria-label={t("Video message")}
            hidden={props.on_call}
            style={style}
            onkeydown={on_key}
        >
            <div class="recorder-pane">
                <div class="recorder-status">
                    <p class="recorder-line">{ status }</p>
                    { for lines.iter().map(|line| html! { <p class="recorder-note">{ line }</p> }) }
                </div>
                <div class="recorder-stage">{ circle }</div>
                <div class="recorder-dock">
                    { banner_html.unwrap_or_default() }
                    <div class="recorder-row">
                        <div class="recorder-leading">{ leading }</div>
                        <div class="recorder-middle">{ middle }</div>
                        { slot_html }
                    </div>
                </div>
            </div>
            <div class="visually-hidden" aria-live="polite" aria-atomic="true">
                if view.said.0 > 0 {
                    <span key={view.said.0}>{ view.said.1.clone() }</span>
                }
            </div>
            if view.asking.is_some() {
                <Confirm
                    title={t("Delete video message?")}
                    confirm={t("Delete")}
                    cancel={AttrValue::from(t("Keep"))}
                    on_confirm={{ let rig = rig.clone(); Callback::from(move |_: ()| rig.answer(true)) }}
                    on_cancel={{ let rig = rig.clone(); Callback::from(move |_: ()| rig.answer(false)) }}
                />
            }
        </div>
    };
    create_portal(dialog, host)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    /// S3.3's arithmetic: 320 where there is room; the pane's width less 48
    /// and its height less 240 (and the banner) where there is not; never
    /// under 160; and in a pane shorter than 480 the controls stand in a
    /// column, with D = min(320, height − 96, width − 200).
    #[wasm_bindgen_test]
    fn the_circle_fits_the_pane_and_a_short_pane_puts_the_controls_aside() {
        assert_eq!(
            geometry(1024.0, 768.0, 0.0),
            Geometry {
                diameter: 320.0,
                column: false
            }
        );
        // A phone in portrait: 375 × 667 — the width decides.
        assert_eq!(geometry(375.0, 667.0, 0.0).diameter, 320.0);
        assert_eq!(geometry(320.0, 568.0, 0.0).diameter, 272.0);
        // A reply banner takes its height from the circle.
        assert_eq!(geometry(1024.0, 520.0, 40.0).diameter, 240.0);
        assert_eq!(geometry(1024.0, 480.0, 0.0).diameter, 240.0);
        // On its side: a column, and the height decides.
        let side = geometry(667.0, 375.0, 0.0);
        assert!(side.column);
        assert_eq!(side.diameter, 279.0);
        assert_eq!(geometry(844.0, 390.0, 40.0).diameter, 294.0);
        // Never under 160.
        assert_eq!(geometry(150.0, 900.0, 0.0).diameter, 160.0);
        assert_eq!(geometry(300.0, 200.0, 0.0).diameter, 160.0);
    }
}
