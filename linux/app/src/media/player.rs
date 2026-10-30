//! The player (shell-design §10.1): ONE wrapper over `gstreamer-play` for everything the app plays —
//! the viewer's video (P2), voice notes, circles in place, a take in review.
//!
//! - **Its news on the main context.** `gst_play::Play` posts its state, position, duration, end
//!   and error as application messages on its own bus; that bus is watched with
//!   `Bus::add_watch_local` on the GLib main context, so no closure crosses a thread and nothing
//!   needs `Send` (§2.4) — the one exception is playbin's `source-setup`, below.
//! - **Video** goes to `gtk4paintablesink`, whose `paintable` a `gtk::Picture` draws.
//! - **Authorised streaming** (§1.4): on playbin's `source-setup` a `souphttpsrc` gets
//!   `extra-headers` = `Authorization: Bearer …` (sent on every request, Range reads included) and
//!   the app's user agent. The token lives in an `Arc<Mutex<…>>` the signal (a GStreamer thread)
//!   reads — never logged; the app never sets `GST_DEBUG`.
//! - **Rate** keeps the pitch: playbin's `audio-filter` is a `scaletempo`.
//! - **One thing at a time** is the caller's: it claims [`NowPlaying`](crate::seams::media::NowPlaying)
//!   before `play` (the voice bubble, the circle, the viewer).
//!
//! [`GstRoundPlayers`] puts fc-logic's `RoundPlayer` (a circle's one load, §10.4) on the same
//! player.

use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::{Rc, Weak};
use std::sync::{Arc, Mutex};

use fc_core::rt::Event;
use fc_logic::round_playback::{self, RoundPlayer, RoundPlayerNews, RoundPlayers};
use gst::prelude::*;

use super::gst_util;
use super::pipelines;
use crate::seams::media::{MediaSource, Player, PlayerState, Players};

/// What the player heard, as it heard it — the richer news [`GstRoundPlayers`] reads.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Heard {
    State(PlayerState),
    Duration(f64),
    /// The video's size is known: frames are coming.
    Picture,
    Ended,
    Failed,
}

/// One playback (see the module's comment).
pub struct GstPlayer {
    play: gst_play::Play,
    paintable: Option<gtk::gdk::Paintable>,
    state: Cell<PlayerState>,
    duration: Cell<f64>,
    position: Cell<f64>,
    loaded: Cell<bool>,
    on_state: Event<PlayerState>,
    on_position: Event<f64>,
    heard: Event<Heard>,
    bearer: Arc<Mutex<Option<String>>>,
    watch: RefCell<Option<gst::bus::BusWatchGuard>>,
}

impl std::fmt::Debug for GstPlayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GstPlayer")
            .field("state", &self.state.get())
            .finish_non_exhaustive()
    }
}

impl GstPlayer {
    /// A new player; `video` makes its `gtk4paintablesink`. Call on the main thread.
    pub fn new(video: bool) -> Rc<GstPlayer> {
        let play = gst_play::Play::new(None::<gst_play::PlayVideoRenderer>);
        let mut config = play.config();
        config.set_position_update_interval(100);
        config.set_user_agent(&format!(
            "FamilyConnect-Linux/{}",
            env!("CARGO_PKG_VERSION")
        ));
        if play.set_config(config).is_err() {
            crate::fc_warn!("player: its configuration was refused");
        }
        let pipeline = play.pipeline();
        let mut paintable = None;
        if video {
            match gst::ElementFactory::make("gtk4paintablesink").build() {
                Ok(sink) => {
                    paintable = Some(sink.property::<gtk::gdk::Paintable>("paintable"));
                    pipeline.set_property("video-sink", &sink);
                }
                Err(_) => crate::fc_warn!("player: gtk4paintablesink is missing"),
            }
        } else if let Ok(sink) = gst::ElementFactory::make("fakesink").build() {
            pipeline.set_property("video-sink", &sink);
        }
        if super::test_sources()
            && let Ok(sink) = gst::ElementFactory::make("fakesink")
                .property("sync", true)
                .build()
        {
            // The self-test's container has no sound server: the clock still runs.
            pipeline.set_property("audio-sink", &sink);
        }
        if let Ok(tempo) = gst::ElementFactory::make("scaletempo").build() {
            pipeline.set_property("audio-filter", &tempo);
        }
        let bearer: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        {
            let bearer = Arc::clone(&bearer);
            pipeline.connect("source-setup", false, move |args| {
                let source = args
                    .get(1)
                    .and_then(|value| value.get::<gst::Element>().ok());
                if let Some(source) = source
                    && source.has_property("extra-headers")
                {
                    let token = bearer.lock().ok().and_then(|token| token.clone());
                    if let Some(token) = token
                        && let Ok(headers) =
                            pipelines::bearer_headers(&token).parse::<gst::Structure>()
                    {
                        source.set_property("extra-headers", &headers);
                    }
                }
                None
            });
        }
        let player = Rc::new(GstPlayer {
            play,
            paintable,
            state: Cell::new(PlayerState::Idle),
            duration: Cell::new(0.0),
            position: Cell::new(0.0),
            loaded: Cell::new(false),
            on_state: Event::new(),
            on_position: Event::new(),
            heard: Event::new(),
            bearer,
            watch: RefCell::new(None),
        });
        let weak = Rc::downgrade(&player);
        match player
            .play
            .message_bus()
            .add_watch_local(move |_, message| match weak.upgrade() {
                Some(player) => {
                    player.hear(message);
                    glib::ControlFlow::Continue
                }
                None => glib::ControlFlow::Break,
            }) {
            Ok(guard) => *player.watch.borrow_mut() = Some(guard),
            Err(_) => crate::fc_warn!("player: its bus could not be watched"),
        }
        player
    }

    /// The richer news (state, duration, the first picture, end, failure).
    pub fn heard(&self) -> &Event<Heard> {
        &self.heard
    }

    fn set_state(&self, state: PlayerState) {
        if self.state.replace(state) != state {
            self.on_state.emit(&state);
            self.heard.emit(&Heard::State(state));
        }
    }

    fn hear(&self, message: &gst::Message) {
        let Ok(news) = gst_play::PlayMessage::parse(message) else {
            return;
        };
        match news {
            gst_play::PlayMessage::StateChanged(changed) => {
                if !self.loaded.get() {
                    return;
                }
                let state = match changed.state() {
                    gst_play::PlayState::Buffering => PlayerState::Loading,
                    gst_play::PlayState::Paused => PlayerState::Paused,
                    gst_play::PlayState::Playing => PlayerState::Playing,
                    // An end stays an end; a failure stays a failure.
                    _ if matches!(self.state.get(), PlayerState::Ended | PlayerState::Failed) => {
                        return;
                    }
                    _ => PlayerState::Idle,
                };
                // Playing to the end then pausing at it is still the end.
                if self.state.get() == PlayerState::Ended && state == PlayerState::Paused {
                    return;
                }
                self.set_state(state);
            }
            gst_play::PlayMessage::PositionUpdated(updated) => {
                let position = gst_util::seconds(updated.position());
                self.position.set(position);
                if self.state.get() == PlayerState::Playing {
                    self.on_position.emit(&position);
                }
            }
            gst_play::PlayMessage::DurationChanged(changed) => {
                let duration = gst_util::seconds(changed.duration());
                self.duration.set(duration);
                self.heard.emit(&Heard::Duration(duration));
            }
            gst_play::PlayMessage::VideoDimensionsChanged(dimensions) => {
                if dimensions.width() > 0 && dimensions.height() > 0 {
                    self.heard.emit(&Heard::Picture);
                }
            }
            gst_play::PlayMessage::EndOfStream(_) => {
                // `GstPlay` leaves its pipeline PLAYING at the end, and the sound server's stream
                // with it: running, starved, the output kept open until the next load — so the
                // next voice note recorded with the output busy (on a VMware guest's sound card
                // that lost most of the take, measured). Paused, the stream goes idle at once;
                // `play` and `seek` still start again from here.
                self.play.pause();
                self.position.set(self.duration.get());
                self.set_state(PlayerState::Ended);
                self.heard.emit(&Heard::Ended);
            }
            gst_play::PlayMessage::Error(error) => {
                crate::fc_warn!("player: {}", gst_util::error_line(error.error()));
                self.set_state(PlayerState::Failed);
                self.heard.emit(&Heard::Failed);
            }
            _ => {}
        }
    }
}

impl Drop for GstPlayer {
    fn drop(&mut self) {
        self.watch.borrow_mut().take();
        self.play.stop();
    }
}

impl Player for GstPlayer {
    fn load(&self, source: MediaSource) {
        self.play.stop();
        let uri = match source {
            MediaSource::File(path) => {
                if let Ok(mut token) = self.bearer.lock() {
                    *token = None;
                }
                pipelines::file_uri(&path)
            }
            MediaSource::Stream { url, bearer } => {
                if let Ok(mut token) = self.bearer.lock() {
                    *token = bearer;
                }
                url
            }
        };
        self.duration.set(0.0);
        self.position.set(0.0);
        self.loaded.set(true);
        self.play.set_uri(Some(&uri));
        self.set_state(PlayerState::Loading);
    }

    fn play(&self) {
        if !self.loaded.get() {
            return;
        }
        if self.state.get() == PlayerState::Ended {
            self.play.seek(gst::ClockTime::ZERO);
            self.position.set(0.0);
        }
        self.play.play();
    }

    fn pause(&self) {
        if self.loaded.get() {
            self.play.pause();
        }
    }

    fn stop(&self) {
        self.loaded.set(false);
        self.play.stop();
        self.position.set(0.0);
        self.set_state(PlayerState::Idle);
    }

    fn seek(&self, seconds: f64) {
        if !self.loaded.get() {
            return;
        }
        self.position.set(seconds.max(0.0));
        if self.state.get() == PlayerState::Ended {
            self.set_state(PlayerState::Paused);
        }
        self.play.seek(gst_util::clock_time(seconds));
    }

    fn set_rate(&self, rate: f64) {
        if rate.is_finite() && rate > 0.0 {
            self.play.set_rate(rate);
        }
    }

    fn set_muted(&self, muted: bool) {
        self.play.set_mute(muted);
    }

    fn position(&self) -> f64 {
        if matches!(self.state.get(), PlayerState::Playing | PlayerState::Paused) {
            let now = gst_util::seconds(self.play.position());
            if now > 0.0 {
                return now;
            }
        }
        self.position.get()
    }

    fn duration(&self) -> f64 {
        let duration = self.duration.get();
        if duration > 0.0 {
            duration
        } else {
            gst_util::seconds(self.play.duration())
        }
    }

    fn state(&self) -> PlayerState {
        self.state.get()
    }

    fn paintable(&self) -> Option<gtk::gdk::Paintable> {
        self.paintable.clone()
    }

    fn on_state(&self) -> &Event<PlayerState> {
        &self.on_state
    }

    fn on_position(&self) -> &Event<f64> {
        &self.on_position
    }
}

/// `Shell::players`.
#[derive(Debug, Default)]
pub struct GstPlayers;

impl Players for GstPlayers {
    fn create(&self, video: bool) -> Rc<dyn Player> {
        GstPlayer::new(video)
    }
}

/// fc-logic's `RoundPlayer` (one load of a circle, §10.4) over a [`GstPlayer`].
pub struct GstRoundPlayer {
    player: Rc<GstPlayer>,
    news: Event<RoundPlayerNews>,
    opened: Cell<bool>,
    pictured: Cell<bool>,
    done: Cell<bool>,
}

impl GstRoundPlayer {
    /// The paintable the circle shows once `FirstFrame` is reported.
    pub fn paintable(&self) -> Option<gtk::gdk::Paintable> {
        self.player.paintable()
    }

    fn hear(&self, heard: Heard) {
        if self.done.get() {
            return;
        }
        match heard {
            Heard::State(PlayerState::Paused | PlayerState::Playing) if !self.opened.get() => {
                self.opened.set(true);
                self.news.emit(&RoundPlayerNews::Opened {
                    total: self.player.duration(),
                });
                if self.player.state() == PlayerState::Playing {
                    self.news.emit(&RoundPlayerNews::State(
                        round_playback::PlayerState::Playing,
                    ));
                }
            }
            Heard::State(PlayerState::Playing) => self.news.emit(&RoundPlayerNews::State(
                round_playback::PlayerState::Playing,
            )),
            Heard::State(PlayerState::Paused) => self
                .news
                .emit(&RoundPlayerNews::State(round_playback::PlayerState::Paused)),
            Heard::State(PlayerState::Loading) if self.opened.get() => self.news.emit(
                &RoundPlayerNews::State(round_playback::PlayerState::Buffering),
            ),
            Heard::Picture if !self.pictured.get() => {
                self.pictured.set(true);
                self.news.emit(&RoundPlayerNews::FirstFrame);
            }
            Heard::Ended => self.news.emit(&RoundPlayerNews::Ended),
            Heard::Failed => {
                self.done.set(true);
                self.news.emit(&RoundPlayerNews::Failed);
            }
            _ => {}
        }
    }
}

impl RoundPlayer for GstRoundPlayer {
    fn play(&self) {
        self.player.play();
    }

    fn pause(&self) {
        self.player.pause();
    }

    fn rewind(&self) {
        self.player.seek(0.0);
    }

    fn position(&self) -> f64 {
        self.player.position()
    }

    fn duration(&self) -> f64 {
        self.player.duration()
    }

    fn news(&self) -> &Event<RoundPlayerNews> {
        &self.news
    }
}

/// fc-logic's `RoundPlayers`: a circle's clip, already fetched through the attachment cache, on a
/// new video player that loads at once and does not play until asked.
#[derive(Debug, Default)]
pub struct GstRoundPlayers;

impl GstRoundPlayers {
    /// The same as [`RoundPlayers::open`], keeping the concrete type (the circle wants its
    /// paintable).
    pub fn open_round(&self, file: &Path) -> Option<Rc<GstRoundPlayer>> {
        if !gst_util::has_element("gtk4paintablesink") {
            return None;
        }
        let player = GstPlayer::new(true);
        let round = Rc::new(GstRoundPlayer {
            player: Rc::clone(&player),
            news: Event::new(),
            opened: Cell::new(false),
            pictured: Cell::new(false),
            done: Cell::new(false),
        });
        let weak: Weak<GstRoundPlayer> = Rc::downgrade(&round);
        player.heard().subscribe(move |heard| {
            if let Some(round) = weak.upgrade() {
                round.hear(*heard);
            }
        });
        player.load(MediaSource::File(file.to_path_buf()));
        // Prerolled, not played: `Opened` follows, then the caller plays.
        player.pause();
        Some(round)
    }
}

impl RoundPlayers for GstRoundPlayers {
    fn open(&self, file: &Path, _mime: &str) -> Option<Rc<dyn RoundPlayer>> {
        self.open_round(file)
            .map(|round| round as Rc<dyn RoundPlayer>)
    }
}
