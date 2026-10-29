//! The video message recorder's LAYER (shell-design §10.5, §10.6; `RoundRecorder` — its module
//! comment maps every `RecorderAction` to what the shell does, and that is what [`Layer::act`]
//! does): an `adw::Bin`-like box over the whole of the main window's content, in the window's
//! `gtk::Overlay` — not a dialog. PREVIEW (the mirrored live camera, "Not recording", Record dim
//! until the first frame, "Choose camera" with more than one, "Record a voice message instead", a
//! minute untouched closes it), RECORDING (the ring filling red, "10 seconds left" at 50 s, stopped
//! at 59.5 s into REVIEW, Delete asks from ten seconds), REVIEW (the clip as it will be sent, Space
//! plays and pauses, Delete, Retake, Send).
//!
//! While it is up the window's header buttons are insensitive and routes wait
//! (`MainWindow::set_recorder_open`). The circle is the same round clip as a received one: a
//! `gtk::Picture` in a box with `fc-round` and `overflow: hidden`, the ring drawn by its parent.
//!
//! Layout: the status capsule, the circle, the solid bar of controls — stacked, so nothing can
//! overlap; the circle's diameter is `RecorderFrames::frame`'s for the measured parts (§10.5's
//! "Positions"), recomputed as the window changes.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::{Rc, Weak};
use std::time::Duration;

use adw::prelude::*;
use fc_core::protocol::RoundVideoLimits;
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::media_outbox::StagedMedia;
use fc_logic::recorder_frame::{self, RecorderMeasures};
use fc_logic::round_look::{PlaybackEvent, recorder_look};
use fc_logic::round_recorder::{
    CameraEvent, CameraOpenFailure, RecorderAction, RecorderEffect, RecorderInterruption,
    RecorderStage, RoundRecorder, VideoMessageRecorder, VideoTake,
};
use fc_logic::round_video_rules::{self, PictureCheck, RecorderLayout};
use fc_logic::voice_notes;

use super::player::GstPlayer;
use super::video_recorder::GstVideoRecorder;
use crate::desktop::keep_awake::{self, Awake, AwakeHold};
use crate::seams::composer::ComposerHost;
use crate::seams::media::{MediaSource, Player, PlayerState};
use crate::seams::routes::WindowPart;
use crate::services::settings_store::keys;
use crate::shell::Shell;
use crate::windows::main_window::MainWindow;

/// The layer's redraw tick (the ring, the clock).
const FRAME: Duration = Duration::from_millis(100);
/// The machine's own tick (PREVIEW's idle close, the limit, the warning).
const SECOND: Duration = Duration::from_secs(1);
/// A server that named no limits gets the protocol's defaults.
const DEFAULT_LIMITS: RoundVideoLimits = RoundVideoLimits {
    max_ms: 60_000,
    max_bytes: 10 * 1024 * 1024,
};

thread_local! {
    static LAYER: RefCell<Option<Rc<Layer>>> = const { RefCell::new(None) };
}

/// Monotonic milliseconds (the machine's one clock).
fn now() -> i64 {
    glib::monotonic_time() / 1000
}

/// The recorder's camera, once the layer exists.
pub fn recorder() -> Option<Rc<GstVideoRecorder>> {
    LAYER.with(|slot| {
        slot.borrow()
            .as_ref()
            .map(|layer| Rc::clone(&layer.recorder))
    })
}

/// The layer, for the self-test.
pub fn layer() -> Option<Rc<Layer>> {
    LAYER.with(|slot| slot.borrow().clone())
}

/// Whether the recorder is up (the slot's `recorder_open`).
pub fn is_open() -> bool {
    layer().is_some_and(|layer| layer.machine.borrow().is_open())
}

/// Open the recorder over the composer's window (`ComposerRecording::record_video`).
pub fn open(window: Option<gtk::Window>, host: Rc<dyn ComposerHost>) {
    let _ = window;
    if let Some(layer) = layer() {
        layer.open(host);
    }
}

/// `Shell::close_guard` (§10.6, §3.8): a quit may go on unless a take would be lost — RECORDING
/// or REVIEW. Then the take stops into REVIEW and "Delete video message?" is asked
/// (`RecorderInterruption::WindowClosing`); Delete lets the quit go ahead
/// (`RecorderAction::CloseWindow` quits again), Keep stays.
pub fn may_close() -> bool {
    let Some(layer) = layer() else {
        return true;
    };
    let stage = layer.machine.borrow().stage();
    if !matches!(stage, RecorderStage::Recording | RecorderStage::Review) {
        return true;
    }
    crate::fc_info!("recorder: a quit waits for the question ({stage:?})");
    let effects = layer
        .machine
        .borrow_mut()
        .interrupt(RecorderInterruption::WindowClosing, now());
    layer.run(effects);
    false
}

/// The question a quit put is up (the self-test reads it).
#[cfg(feature = "selftest")]
pub fn asking() -> Option<adw::AlertDialog> {
    layer().and_then(|layer| layer.question.borrow().clone())
}

/// Something other than the person (§10.8).
pub fn interrupt(why: RecorderInterruption) {
    if let Some(layer) = layer() {
        let effects = layer.machine.borrow_mut().interrupt(why, now());
        layer.run(effects);
    }
}

/// Something that pauses what plays, while REVIEW may be playing.
pub fn playback_interrupted(event: PlaybackEvent) {
    if let Some(layer) = layer() {
        let effects = layer.machine.borrow().playback_interrupted(event);
        layer.run(effects);
    }
}

/// The main window's part: the layer, hidden, over the content.
pub struct RecorderPart;

impl WindowPart for RecorderPart {
    fn attach(&self, window: &Rc<MainWindow>) {
        let layer = Layer::new(window);
        window.overlay().add_overlay(&layer.root);
        LAYER.with(|slot| *slot.borrow_mut() = Some(layer));
        // Quitting over a take: the camera and the microphone are let go of (§3.8). (W.3's close
        // has no question to wait on — a REQUEST to S0.)
        Shell::get()
            .on_quitting()
            .subscribe(|_| interrupt(RecorderInterruption::SignedOut));
    }
}

/// A finished clip in REVIEW.
struct Clip {
    take: VideoTake,
    staged: StagedMedia,
}

/// The layer (see the module's comment).
pub struct Layer {
    root: gtk::Box,
    status: gtk::Label,
    below: gtk::Label,
    voice_instead: gtk::Button,
    picture: gtk::Picture,
    clip_box: gtk::Overlay,
    ring: gtk::DrawingArea,
    frame_box: gtk::Overlay,
    slot: gtk::Button,
    slot_caption: gtk::Label,
    left: gtk::Button,
    left_caption: gtk::Label,
    right: gtk::Button,
    right_caption: gtk::Label,
    bar: gtk::Box,
    window: Weak<MainWindow>,
    pub(crate) recorder: Rc<GstVideoRecorder>,
    machine: RefCell<RoundRecorder>,
    host: RefCell<Option<Rc<dyn ComposerHost>>>,
    clip: RefCell<Option<Clip>>,
    review: RefCell<Option<Rc<GstPlayer>>>,
    question: RefCell<Option<adw::AlertDialog>>,
    /// The screen kept on while a take records (§10.9, P5's hold).
    awake: RefCell<Option<AwakeHold>>,
    queue: RefCell<VecDeque<RecorderEffect>>,
    acting: Cell<bool>,
    timers: RefCell<Vec<glib::SourceId>>,
    check: RefCell<PictureCheck>,
    diameter: Cell<i32>,
    first_time: Cell<bool>,
}

impl std::fmt::Debug for Layer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Layer")
            .field("stage", &self.machine.borrow().stage())
            .finish_non_exhaustive()
    }
}

impl Layer {
    fn new(window: &Rc<MainWindow>) -> Rc<Layer> {
        let shell = Shell::get();
        let say = shell.say.as_ref();
        let limits = DEFAULT_LIMITS;
        let status = gtk::Label::builder()
            .css_classes(["fc-media-status", "numeric"])
            .halign(gtk::Align::Center)
            .build();
        let below = gtk::Label::builder()
            .css_classes(["caption", "dim-label"])
            .wrap(true)
            .justify(gtk::Justification::Center)
            .halign(gtk::Align::Center)
            .visible(false)
            .build();
        let voice_instead = gtk::Button::builder()
            .label(say.get("Record a voice message instead"))
            .css_classes(["flat", "caption"])
            .halign(gtk::Align::Center)
            .build();
        let side = round_video_rules::LARGEST_CIRCLE as i32;
        let picture = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Cover)
            .can_shrink(true)
            .css_classes(["fc-media-poster"])
            .build();
        let clip_box = gtk::Overlay::builder()
            .width_request(side)
            .height_request(side)
            .overflow(gtk::Overflow::Hidden)
            .css_classes(["fc-round"])
            .build();
        clip_box.set_child(Some(&picture));
        let tall = adw::Clamp::builder()
            .orientation(gtk::Orientation::Vertical)
            .maximum_size(side)
            .tightening_threshold(side)
            .child(&clip_box)
            .build();
        let wide = adw::Clamp::builder()
            .orientation(gtk::Orientation::Horizontal)
            .maximum_size(side)
            .tightening_threshold(side)
            .child(&tall)
            .build();
        let reach = recorder_frame::RING_CLEARANCE.round() as i32;
        wide.set_margin_start(reach);
        wide.set_margin_end(reach);
        wide.set_margin_top(reach);
        wide.set_margin_bottom(reach);
        let ring = gtk::DrawingArea::builder().can_target(false).build();
        let frame_box = gtk::Overlay::builder()
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .vexpand(true)
            .build();
        frame_box.set_child(Some(&wide));
        frame_box.add_overlay(&ring);
        let round = |classes: &[&str], size: f64| {
            gtk::Button::builder()
                .width_request(size as i32)
                .height_request(size as i32)
                .halign(gtk::Align::Center)
                .css_classes(classes.to_vec())
                .build()
        };
        let slot = round(&["fc-media-slot", "fc-media-record"], recorder_look::SLOT);
        let left = round(&["circular"], recorder_look::SIDE);
        let right = round(&["circular"], recorder_look::SIDE);
        let caption = || {
            gtk::Label::builder()
                .css_classes(["caption"])
                .halign(gtk::Align::Center)
                .build()
        };
        let (slot_caption, left_caption, right_caption) = (caption(), caption(), caption());
        let column = |button: &gtk::Button, label: &gtk::Label| {
            let column = gtk::Box::new(
                gtk::Orientation::Vertical,
                recorder_look::CAPTION_GAP as i32,
            );
            column.set_valign(gtk::Align::Center);
            column.append(button);
            column.append(label);
            column
        };
        let bar = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(32)
            .halign(gtk::Align::Fill)
            .css_classes(["fc-media-bar"])
            .build();
        let controls = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(32)
            .halign(gtk::Align::Center)
            .hexpand(true)
            .build();
        controls.append(&column(&left, &left_caption));
        controls.append(&column(&slot, &slot_caption));
        controls.append(&column(&right, &right_caption));
        bar.append(&controls);
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .hexpand(true)
            .vexpand(true)
            .css_classes(["fc-media-layer"])
            .visible(false)
            .build();
        let top = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(4)
            .margin_top(16)
            .build();
        top.append(&status);
        top.append(&below);
        top.append(&voice_instead);
        root.append(&top);
        root.append(&frame_box);
        root.append(&bar);
        let layer = Rc::new(Layer {
            root,
            status,
            below,
            voice_instead,
            picture,
            clip_box,
            ring,
            frame_box,
            slot,
            slot_caption,
            left,
            left_caption,
            right,
            right_caption,
            bar,
            window: Rc::downgrade(window),
            recorder: GstVideoRecorder::new(),
            machine: RefCell::new(RoundRecorder::new(limits, shell.say.clone())),
            host: RefCell::new(None),
            clip: RefCell::new(None),
            review: RefCell::new(None),
            question: RefCell::new(None),
            awake: RefCell::new(None),
            queue: RefCell::new(VecDeque::new()),
            acting: Cell::new(false),
            timers: RefCell::new(Vec::new()),
            check: RefCell::new(PictureCheck::new()),
            diameter: Cell::new(side),
            first_time: Cell::new(true),
        });
        layer.wire();
        layer
    }

    fn wire(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        let on = |button: &gtk::Button, press: fn(&Rc<Layer>)| {
            let weak = weak.clone();
            button.connect_clicked(move |_| {
                if let Some(layer) = weak.upgrade() {
                    press(&layer);
                }
            });
        };
        on(&self.slot, |layer| {
            let effects = layer.machine.borrow_mut().slot(now());
            layer.run(effects);
        });
        on(&self.left, |layer| {
            let stage = layer.machine.borrow().stage();
            let effects = if matches!(stage, RecorderStage::Recording | RecorderStage::Review) {
                layer.machine.borrow_mut().delete(now())
            } else {
                layer.machine.borrow_mut().close_button()
            };
            layer.run(effects);
        });
        on(&self.right, |layer| {
            let stage = layer.machine.borrow().stage();
            if stage == RecorderStage::Review {
                let effects = layer.machine.borrow_mut().retake(now());
                layer.run(effects);
            } else {
                layer.choose_camera();
            }
        });
        on(&self.voice_instead, |layer| {
            let waits = layer
                .host
                .borrow()
                .as_ref()
                .is_some_and(|host| layer_not_sent(host.chat_id()));
            let effects = layer.machine.borrow_mut().voice_instead(waits);
            layer.run(effects);
        });
        {
            let weak = weak.clone();
            self.ring.set_draw_func(move |_, cr, width, height| {
                if let Some(layer) = weak.upgrade() {
                    layer.draw_ring(cr, f64::from(width), f64::from(height));
                }
            });
        }
        let click = gtk::GestureClick::new();
        {
            let weak = weak.clone();
            click.connect_released(move |_, _, _, _| {
                if let Some(layer) = weak.upgrade() {
                    layer.toggle_review();
                }
            });
        }
        self.clip_box.add_controller(click);
        // Escape and Space are the recorder's (in the bubble phase — never an accelerator).
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed({
            let weak = weak.clone();
            move |_, key, _, _| {
                let Some(layer) = weak.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                match key {
                    gtk::gdk::Key::Escape => {
                        let effects = layer.machine.borrow_mut().escape(now());
                        layer.run(effects);
                        glib::Propagation::Stop
                    }
                    gtk::gdk::Key::space if layer.machine.borrow().catches_space() => {
                        layer.toggle_review();
                        glib::Propagation::Stop
                    }
                    _ => glib::Propagation::Proceed,
                }
            }
        });
        self.root.add_controller(keys);
        self.recorder.events().subscribe(move |event| {
            if let Some(layer) = weak.upgrade() {
                layer.camera_event(*event);
            }
        });
    }

    /// Opened from the video button, the paperclip or the menu (S3.1).
    fn open(self: &Rc<Self>, host: Rc<dyn ComposerHost>) {
        if self.machine.borrow().is_open() {
            return;
        }
        let shell = Shell::get();
        let limits = shell
            .connection()
            .and_then(|connection| connection.session.state().round_video)
            .unwrap_or(DEFAULT_LIMITS);
        *self.machine.borrow_mut() = RoundRecorder::new(limits, shell.say.clone());
        *self.host.borrow_mut() = Some(host);
        let camera = self.recorder.camera_access();
        let microphone = self.recorder.microphone_access();
        let effects = self.machine.borrow_mut().open(
            now(),
            round_video_rules::refused_before(camera, microphone),
            round_video_rules::will_ask(camera, microphone),
        );
        self.root.set_visible(true);
        if let Some(window) = self.window.upgrade() {
            window.set_recorder_open(true);
        }
        self.start_timers();
        self.slot.grab_focus();
        self.run(effects);
        self.draw();
    }

    fn start_timers(self: &Rc<Self>) {
        self.stop_timers();
        let weak = Rc::downgrade(self);
        let frame = glib::timeout_add_local(FRAME, move || match weak.upgrade() {
            Some(layer) => {
                layer.fit();
                layer.draw();
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        });
        let weak = Rc::downgrade(self);
        let second = glib::timeout_add_local(SECOND, move || match weak.upgrade() {
            Some(layer) => {
                let effects = layer.machine.borrow_mut().tick(now());
                layer.run(effects);
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        });
        self.timers.borrow_mut().extend([frame, second]);
    }

    fn stop_timers(&self) {
        for timer in self.timers.borrow_mut().drain(..) {
            timer.remove();
        }
    }

    /// Runs effects IN ORDER, each to its end before the next (the module comment of
    /// `round_recorder`).
    fn run(self: &Rc<Self>, effects: Vec<RecorderEffect>) {
        if effects.is_empty() {
            self.draw();
            return;
        }
        self.queue.borrow_mut().extend(effects);
        if self.acting.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            loop {
                let Some(layer) = weak.upgrade() else {
                    return;
                };
                let next = layer.queue.borrow_mut().pop_front();
                let Some(effect) = next else {
                    layer.acting.set(false);
                    layer.draw();
                    return;
                };
                layer.act(effect).await;
                layer.draw();
            }
        });
    }

    async fn act(self: &Rc<Self>, effect: RecorderEffect) {
        let say = Shell::get().say.clone();
        match effect.action {
            RecorderAction::OpenCamera => {
                self.recorder.close().await;
                let cameras = self.recorder.cameras().await;
                let remembered = Shell::get().settings.string(keys::VIDEO_CAMERA);
                let Some(camera) =
                    round_video_rules::pick_camera(&cameras, remembered.as_deref()).cloned()
                else {
                    let effects = self
                        .machine
                        .borrow_mut()
                        .camera_troubled(round_video_rules::CameraTrouble::Missing, now());
                    self.queue.borrow_mut().extend(effects);
                    return;
                };
                *self.check.borrow_mut() = PictureCheck::new();
                let effects = match self.recorder.open(&camera.id).await {
                    Err(CameraOpenFailure::Refused(refused)) => {
                        self.machine.borrow_mut().refuse(refused)
                    }
                    Err(CameraOpenFailure::Troubled(trouble)) => {
                        self.machine.borrow_mut().camera_troubled(trouble, now())
                    }
                    Ok(()) if self.machine.borrow().stage() != RecorderStage::Opening => {
                        self.recorder.close().await;
                        Vec::new()
                    }
                    Ok(()) => {
                        self.picture.set_paintable(self.recorder.preview().as_ref());
                        self.machine.borrow_mut().camera_on(now())
                    }
                };
                self.queue.borrow_mut().extend(effects);
            }
            RecorderAction::CloseCamera => {
                self.recorder.close().await;
                if self.clip.borrow().is_none() {
                    self.picture.set_paintable(None::<&gtk::gdk::Paintable>);
                }
            }
            RecorderAction::StartRecording => {
                super::now_playing().set_recording(true);
                let window = self
                    .window
                    .upgrade()
                    .map(|window| window.window().clone().upcast::<gtk::Window>());
                *self.awake.borrow_mut() = Some(keep_awake::hold(
                    &Shell::get().app,
                    window.as_ref(),
                    Awake::VideoMessage,
                    say.as_ref(),
                ));
                if super::session_watch::screen_reader() {
                    self.announce(say.get("Recording video"));
                    glib::timeout_future(voice_notes::SCREEN_READER_LEAD_IN).await;
                }
                let started = self.recorder.start().await;
                if !started || self.machine.borrow().stage() != RecorderStage::Recording {
                    self.release_recording();
                    let effects = self.machine.borrow_mut().record_failed();
                    self.queue.borrow_mut().extend(effects);
                } else {
                    self.machine.borrow_mut().take_started(now());
                }
            }
            RecorderAction::StopAndKeep => {
                let take = self.recorder.stop().await;
                self.release_recording();
                // Beside the steps, not in their line: a question put up meanwhile does not wait.
                let weak = Rc::downgrade(self);
                glib::spawn_future_local(async move {
                    if let Some(layer) = weak.upgrade() {
                        layer.make_clip(take).await;
                    }
                });
            }
            RecorderAction::StopAndDiscard => {
                self.recorder.discard().await;
                self.release_recording();
            }
            RecorderAction::DiscardClip => {
                self.stop_review();
                if let Some(clip) = self.clip.borrow_mut().take() {
                    self.recorder.delete(&clip.take);
                }
            }
            RecorderAction::PauseClip => {
                if let Some(player) = self.review.borrow().as_ref() {
                    player.pause();
                }
            }
            RecorderAction::Ask => self.ask(),
            RecorderAction::Dismiss => {
                if let Some(dialog) = self.question.borrow_mut().take() {
                    dialog.force_close();
                }
            }
            RecorderAction::Send => {
                self.stop_review();
                let clip = self.clip.borrow_mut().take();
                if let (Some(clip), Some(host)) = (clip, self.host.borrow().clone()) {
                    // A VIDEO MESSAGE: the outbox row's `round: true`.
                    host.send_video_message(clip.staged);
                    self.recorder.delete(&clip.take);
                }
            }
            RecorderAction::StartVoice => {
                let host = self.host.borrow().clone();
                glib::idle_add_local_once(move || {
                    if let Some(host) = host
                        && let Some(slots) = super::recording::installed()
                        && let Some(composer) = slots
                            .composers()
                            .into_iter()
                            .find(|composer| composer.chat_id() == host.chat_id())
                    {
                        crate::seams::composer::ComposerRecording::start_voice(
                            composer.as_ref(),
                            false,
                        );
                    }
                });
            }
            RecorderAction::CloseWindow => {
                Shell::get().quit();
            }
            RecorderAction::Closed => self.closed(),
            RecorderAction::Announce => {
                if let Some(text) = effect.text.as_deref() {
                    self.announce(text);
                }
            }
        }
    }

    fn release_recording(&self) {
        super::now_playing().set_recording(false);
        drop(self.awake.borrow_mut().take());
    }

    fn announce(&self, text: &str) {
        self.root
            .announce(text, gtk::AccessibleAnnouncementPriority::Medium);
    }

    fn camera_event(self: &Rc<Self>, event: CameraEvent) {
        let effects = match event {
            CameraEvent::FirstFrame => self.machine.borrow_mut().first_frame(),
            CameraEvent::Brightness(luma) => {
                if self.check.borrow_mut().frame(now(), luma) {
                    self.machine.borrow_mut().covered();
                }
                Vec::new()
            }
            CameraEvent::Blocked => {
                self.machine.borrow_mut().covered();
                Vec::new()
            }
            CameraEvent::Troubled(trouble) => {
                self.machine.borrow_mut().camera_troubled(trouble, now())
            }
        };
        self.run(effects);
    }

    /// The take's clip: read back, planned, staged (`round_video_rules::plan_take`).
    async fn make_clip(self: &Rc<Self>, take: Option<VideoTake>) {
        let Some(take) = take else {
            let effects = self.machine.borrow_mut().clip_finished(now(), None, None);
            self.run(effects);
            return;
        };
        let shell = Shell::get();
        let limits = shell
            .connection()
            .and_then(|connection| connection.session.state().round_video)
            .unwrap_or(DEFAULT_LIMITS);
        let reading = self.recorder.read_back(&take).await;
        let (plan, duration) = round_video_rules::plan_take(
            &take.asked,
            reading.as_ref(),
            take.elapsed_ms,
            &limits,
            shell.say.as_ref(),
        );
        let staged = if plan.from_square {
            let poster = self.recorder.poster(&take).await;
            std::fs::read(&take.path).ok().map(|bytes| {
                round_video_rules::staged(
                    bytes,
                    i32::try_from(duration).unwrap_or(i32::MAX),
                    poster,
                )
            })
        } else {
            let cancel = fc_core::rt::Cancel::new();
            match super::preparing::prepare_picked(&take.path, "video/mp4", None, &cancel).await {
                Ok(super::preparing::Picked::Staged(media)) => Some(*media),
                _ => None,
            }
        };
        if !self.machine.borrow().holds_clip() {
            // Thrown away meanwhile: it lands nowhere.
            self.recorder.delete(&take);
            return;
        }
        let Some(staged) = staged else {
            self.recorder.delete(&take);
            let effects = self.machine.borrow_mut().clip_finished(now(), None, None);
            self.run(effects);
            return;
        };
        let player = GstPlayer::new(true);
        player.load(MediaSource::File(take.path.clone()));
        player.pause();
        self.picture.set_paintable(player.paintable().as_ref());
        *self.review.borrow_mut() = Some(player);
        *self.clip.borrow_mut() = Some(Clip { take, staged });
        let effects = self
            .machine
            .borrow_mut()
            .clip_finished(now(), Some(plan), Some(duration));
        self.run(effects);
    }

    fn toggle_review(&self) {
        if self.machine.borrow().stage() != RecorderStage::Review {
            return;
        }
        if let Some(player) = self.review.borrow().as_ref() {
            if player.state() == PlayerState::Playing {
                player.pause();
            } else {
                player.play();
            }
        }
    }

    fn stop_review(&self) {
        if let Some(player) = self.review.borrow_mut().take() {
            player.stop();
        }
    }

    /// "Delete video message?" [Delete] [Keep].
    fn ask(self: &Rc<Self>) {
        let say = Shell::get().say.clone();
        let dialog = adw::AlertDialog::new(Some(say.get("Delete video message?")), None);
        dialog.add_response("keep", say.get("Keep"));
        dialog.add_response("delete", say.get("Delete"));
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("keep"));
        dialog.set_close_response("keep");
        let weak = Rc::downgrade(self);
        dialog.connect_response(None, move |dialog, response| {
            let Some(layer) = weak.upgrade() else {
                return;
            };
            let ours = layer
                .question
                .borrow()
                .as_ref()
                .is_some_and(|asked| asked == dialog);
            if !ours {
                return;
            }
            layer.question.borrow_mut().take();
            let effects = layer
                .machine
                .borrow_mut()
                .answer(response == "delete", now());
            layer.run(effects);
        });
        let parent = self.window.upgrade().map(|window| window.window().clone());
        dialog.present(parent.as_ref());
        *self.question.borrow_mut() = Some(dialog);
    }

    /// "Choose camera" (S3.5): the next camera, remembered for this device.
    fn choose_camera(self: &Rc<Self>) {
        let cameras = Shell::get().camera.get().devices();
        if !round_video_rules::offers_camera_choice(cameras.len()) {
            return;
        }
        let shell = Shell::get();
        let current = shell.settings.string(keys::VIDEO_CAMERA);
        let at = cameras
            .iter()
            .position(|camera| Some(&camera.id) == current.as_ref())
            .map_or(0, |at| (at + 1) % cameras.len());
        shell
            .settings
            .set_string(keys::VIDEO_CAMERA, &cameras[at].id);
        let effects = self.machine.borrow_mut().switch_camera(now());
        self.run(effects);
    }

    fn closed(&self) {
        self.stop_review();
        self.stop_timers();
        self.picture.set_paintable(None::<&gtk::gdk::Paintable>);
        self.root.set_visible(false);
        self.host.borrow_mut().take();
        if let Some(window) = self.window.upgrade() {
            window.set_recorder_open(false);
        }
        self.first_time.set(false);
    }

    /// The circle's diameter for the window as it is now (`round_video_rules::fit` and
    /// `RecorderFrames::frame` over the measured parts).
    fn fit(&self) {
        let (width, height) = (f64::from(self.root.width()), f64::from(self.root.height()));
        if width <= 0.0 || height <= 0.0 {
            return;
        }
        let status = self.status.preferred_size().1;
        let controls = self.bar.preferred_size().1;
        let fit = round_video_rules::fit(width, height, 0.0);
        let measures = RecorderMeasures::new(
            width,
            height,
            f64::from(status.width()),
            f64::from(status.height()),
            0.0,
            0.0,
            f64::from(controls.width()),
            f64::from(controls.height()),
        );
        let frame = recorder_frame::frame(fit, &measures);
        let diameter = frame.diameter.max(recorder_frame::SHORTEST_CIRCLE).round() as i32;
        if self.diameter.replace(diameter) != diameter {
            self.clip_box.set_size_request(diameter, diameter);
            if let Some(clamp) = self
                .clip_box
                .parent()
                .and_then(|tall| tall.downcast::<adw::Clamp>().ok())
            {
                clamp.set_maximum_size(diameter);
                clamp.set_tightening_threshold(diameter);
                if let Some(wide) = clamp
                    .parent()
                    .and_then(|wide| wide.downcast::<adw::Clamp>().ok())
                {
                    wide.set_maximum_size(diameter);
                    wide.set_tightening_threshold(diameter);
                }
            }
        }
        self.bar.set_orientation(match fit.layout {
            RecorderLayout::Row => gtk::Orientation::Horizontal,
            RecorderLayout::Column => gtk::Orientation::Horizontal,
        });
    }

    fn draw(&self) {
        let shell = Shell::get();
        let say = shell.say.as_ref();
        let machine = self.machine.borrow();
        let stage = machine.stage();
        let at = now();
        let status = machine.status(
            at,
            self.first_time.get(),
            round_video_rules::MICROPHONE_ON_IN_PREVIEW,
        );
        self.status.set_label(&status.line);
        if status.red {
            self.status.add_css_class("fc-media-red");
        } else {
            self.status.remove_css_class("fc-media-red");
        }
        self.below
            .set_label(status.below.as_deref().unwrap_or_default());
        self.below.set_visible(status.below.is_some());
        self.voice_instead.set_visible(matches!(
            stage,
            RecorderStage::Opening
                | RecorderStage::Preview
                | RecorderStage::Refused
                | RecorderStage::Unavailable
        ));
        // The slot: Record, Stop, Send.
        for class in ["fc-media-record", "fc-media-stop", "fc-media-send"] {
            self.slot.remove_css_class(class);
        }
        let (class, icon) = match recorder_look::shape(stage) {
            fc_logic::round_look::SlotShape::RecordDisc => {
                ("fc-media-record", "media-record-symbolic")
            }
            fc_logic::round_look::SlotShape::StopSquare => {
                ("fc-media-stop", "media-playback-stop-symbolic")
            }
            fc_logic::round_look::SlotShape::SendArrow => (
                "fc-media-send",
                fc_logic::composer_button::SlotIcon::Send.icon_name(),
            ),
        };
        self.slot.add_css_class(class);
        self.slot.set_icon_name(icon);
        let caption = recorder_look::caption(stage, say);
        self.slot_caption.set_label(&caption);
        self.slot
            .update_property(&[gtk::accessible::Property::Label(&caption)]);
        let slot_live = match stage {
            RecorderStage::Preview => machine.has_picture(),
            RecorderStage::Recording => true,
            RecorderStage::Review => machine.clip_ready(),
            _ => false,
        };
        self.slot.set_sensitive(slot_live);
        let (left_icon, left_word) = match stage {
            RecorderStage::Recording | RecorderStage::Review => {
                ("user-trash-symbolic", say.get("Delete"))
            }
            _ => ("window-close-symbolic", say.get("Close")),
        };
        self.left.set_icon_name(left_icon);
        self.left_caption.set_label(left_word);
        self.left
            .update_property(&[gtk::accessible::Property::Label(left_word)]);
        let cameras = shell.camera.get().devices().len();
        let (right_icon, right_word, right_shown) = match stage {
            RecorderStage::Review => ("view-refresh-symbolic", say.get("Retake"), true),
            RecorderStage::Preview | RecorderStage::Unavailable => (
                "camera-switch-symbolic",
                say.get("Choose camera"),
                round_video_rules::offers_camera_choice(cameras),
            ),
            _ => ("camera-switch-symbolic", say.get("Choose camera"), false),
        };
        self.right.set_icon_name(right_icon);
        self.right_caption.set_label(right_word);
        self.right
            .update_property(&[gtk::accessible::Property::Label(right_word)]);
        self.right.set_visible(right_shown);
        self.right_caption.set_visible(right_shown);
        drop(machine);
        self.ring.queue_draw();
        let _ = self.frame_box.is_visible();
    }

    fn draw_ring(&self, cr: &gtk::cairo::Context, width: f64, height: f64) {
        let machine = self.machine.borrow();
        let stage = machine.stage();
        if !recorder_look::shows_track(stage) {
            return;
        }
        let radius =
            f64::from(self.diameter.get()) / 2.0 + round_video_rules::RING_WIDTH / 2.0 + 1.0;
        let (cx, cy) = (width / 2.0, height / 2.0);
        cr.set_line_width(recorder_look::TRACK);
        cr.set_source_rgba(0.5, 0.5, 0.5, 0.5);
        cr.arc(cx, cy, radius, 0.0, 2.0 * std::f64::consts::PI);
        let _ = cr.stroke();
        let (fraction, colour) = match stage {
            RecorderStage::Recording => {
                let warn = machine.warns(now());
                (
                    machine.ring_fraction(now()),
                    if warn {
                        (0.78, 0.27, 0.0)
                    } else {
                        (0.88, 0.11, 0.14)
                    },
                )
            }
            RecorderStage::Review => {
                let (position, total) = self
                    .review
                    .borrow()
                    .as_ref()
                    .map_or((0.0, 0.0), |player| (player.position(), player.duration()));
                let accent = crate::theme::palette().accent;
                (
                    if total > 0.0 {
                        (position / total).clamp(0.0, 1.0)
                    } else {
                        0.0
                    },
                    (
                        f64::from(accent.0) / 255.0,
                        f64::from(accent.1) / 255.0,
                        f64::from(accent.2) / 255.0,
                    ),
                )
            }
            _ => (0.0, (0.0, 0.0, 0.0)),
        };
        if fraction <= 0.0 {
            return;
        }
        cr.set_line_width(round_video_rules::RING_WIDTH);
        cr.set_source_rgb(colour.0, colour.1, colour.2);
        let start = -std::f64::consts::FRAC_PI_2;
        cr.arc(
            cx,
            cy,
            radius,
            start,
            start + fraction * 2.0 * std::f64::consts::PI,
        );
        let _ = cr.stroke();
    }
}

/// For the self-test.
#[cfg_attr(
    not(feature = "selftest"),
    allow(
        dead_code,
        reason = "the media self-test drives and reads the recorder through these"
    )
)]
impl Layer {
    /// The machine's stage.
    pub fn stage(&self) -> RecorderStage {
        self.machine.borrow().stage()
    }

    /// Whether the camera's first frame came (Record is live).
    pub fn has_picture(&self) -> bool {
        self.machine.borrow().has_picture()
    }

    /// Why the camera cannot be used, if it cannot.
    pub fn trouble(&self) -> Option<round_video_rules::CameraTrouble> {
        self.machine.borrow().trouble()
    }

    /// The layer's widget.
    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// Whether the clip in REVIEW is ready.
    pub fn clip_ready(&self) -> bool {
        self.machine.borrow().clip_ready()
    }

    /// Presses the slot (Record, Stop, Send).
    pub fn press_slot(self: &Rc<Self>) {
        let effects = self.machine.borrow_mut().slot(now());
        self.run(effects);
    }
}

/// Whether the chat holds a voice message that was not sent (row 9's notice).
fn layer_not_sent(chat_id: i64) -> bool {
    super::recording::installed()
        .and_then(|slots| slots.store())
        .is_some_and(|store| store.any(chat_id))
}
