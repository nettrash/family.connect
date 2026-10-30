//! Playing a VIDEO MESSAGE IN PLACE (docs/audio-video-messages-2026-10-04.md, S5.2–S5.4, S6) —
//! `RoundPlayback.cs`: a click plays it inside its own circle, at the same size, with sound — what
//! every other client does. Everything about it that needs no player is here: the circle's states
//! and what a click does in each ([`RoundPlayback`]), when its dot goes, how far round its one ring
//! is, which of its pieces show, and when it is out of view ([`round_inline`]).
//!
//! The player itself is the shell's ([`RoundPlayer`], [`RoundPlayers`]): on Windows a Media
//! Foundation player in frame-server mode copying every frame into the ellipse that draws the
//! poster, because WinUI would not clip a video to a circle; on Linux `gst_play` with
//! `gtk4paintablesink` under a `gtk::Picture` that GTK clips round itself (shell-design §10.1,
//! §10.4, measured in §1.5). So Windows' `RoundFrames` (the frame copy's throttle, surface size and
//! opacity) and `RoundCrashGuard` (the mark that sent a build that died playing in place to the
//! viewer) are NOT ported: there is no frame copy to throttle and no native frame server to crash
//! in (carry-forward, "Not ported").

use std::path::Path;
use std::rc::Rc;

use fc_core::rt::Event;

use crate::round_look::{PlaybackEvent, Playing, playback_pauses};

/// Where one circle is (iOS `RoundVideoPlayback.Phase`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RoundPhase {
    /// The poster with the play disc: before the first click, after the end, after a load given
    /// up, after a stop.
    #[default]
    Rest,
    /// Clicked and waiting — for the bytes, for the player, or for frames in a stall: the loading
    /// ring.
    Loading,
    /// Frames are moving, with sound.
    Playing,
    /// Paused part way — by a click, or by something else starting, a call, a lock, a hidden
    /// window.
    Paused,
    /// "Couldn't load the video. Tap to try again."
    Failed,
}

/// What a click on a circle did — the view says the one that needs words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RoundTap {
    /// It began to load (from rest, after a failure, or instead of another circle).
    Started,
    /// A paused circle plays on.
    Resumed,
    /// A playing circle paused.
    Paused,
    /// A second click while it loads gives up (S5.3): back to the poster.
    GaveUp,
    /// A recording runs: nothing of the app's plays under it (S1.7) — "You can play this after
    /// recording."
    Refused,
}

/// What the view does to the one player for a step of the machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RoundAct {
    /// Nothing.
    None,
    /// Let go of any player, then fetch this circle's video and open a new one (the load is
    /// [`RoundPlayback::load`]).
    Load,
    /// Play the player there is — after pausing whatever else plays.
    Play,
    /// Pause the player, keeping it and its last frame.
    Pause,
    /// Let go of the player: the circle is back to its poster.
    Stop,
}

/// One step of a circle's machine: what the click did (`tap`, when it was a click), what the view
/// does to the player (`act`), and another circle this step took the player from (`replaced`) — it
/// goes back to its poster and is redrawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RoundStep {
    pub tap: RoundTap,
    pub act: RoundAct,
    pub replaced: Option<i64>,
}

impl RoundStep {
    /// `new RoundStep(tap, act)`, replacing nothing.
    pub fn new(tap: RoundTap, act: RoundAct) -> Self {
        RoundStep {
            tap,
            act,
            replaced: None,
        }
    }
}

/// The ONE circle that plays in place, and every rule of moving it between [`RoundPhase`]s — the
/// iPhone's `RoundVideoPlayback` and the web's round tile, as a machine the window drives: a click
/// plays, a click pauses, a second click while it loads gives up, a failure waits for a click to
/// try again, the end returns it to its poster (and says whether its dot goes), one thing plays at
/// a time ([`make_way`](Self::make_way)), a call, a lock or a hidden window pauses it
/// ([`interrupt`](Self::interrupt)), and its row leaving the screen or the chat closing stops it
/// ([`stop`](Self::stop)).
///
/// **ONE AT A TIME, BY CONSTRUCTION.** There is one machine per conversation view and it names one
/// circle ([`id`](Self::id)); clicking another takes the player from the first, which goes back to
/// its poster ([`RoundStep::replaced`]).
///
/// **STALE NEWS IS IGNORED.** Every load has its own number ([`load`](Self::load)); whatever the
/// player reports carries the number of the load it belongs to, and news of a load that was given
/// up, replaced or stopped changes nothing — a download that lands after a second click never
/// starts playing.
///
/// How the shell wires it (Windows' `ChatsView`): a click → [`tap`](Self::tap) (with "a voice
/// recording runs or is being started"); [`RoundAct::Load`] → let go of any [`RoundPlayer`],
/// pause whatever else plays, fetch the MP4 through the attachment cache, and — if the load is still
/// this machine's and still [`RoundPhase::Loading`] — [`RoundPlayers::open`] it, failing the load
/// ([`failed`](Self::failed)) when the bytes or the player cannot be had; the player's news:
/// `Opened(total)` → [`opened`](Self::opened), and when that answers true play it (or, if a
/// recording began meanwhile, [`make_way`](Self::make_way)); `State(Playing)` →
/// [`playing`](Self::playing) (and make way if a recording began); `State(Paused)` →
/// [`paused_by_player`](Self::paused_by_player); `State(Buffering)` → [`waiting`](Self::waiting);
/// `FirstFrame` → [`first_frame`](Self::first_frame); `Ended` → [`ended`](Self::ended) (mark the
/// attachment played in `PlayedRoundStore` when it says played through); `Failed` →
/// [`failed`](Self::failed). While [`RoundPhase::Playing`], every [`round_inline::TICK_MS`] →
/// [`progress`](Self::progress) with the player's position and duration.
#[derive(Debug, Clone, Default)]
pub struct RoundPlayback {
    id: Option<i64>,
    phase: RoundPhase,
    load: u32,
    has_frames: bool,
    seen_playing: bool,
    position: f64,
    total: f64,
}

impl RoundPlayback {
    /// A machine naming no circle, at rest.
    pub fn new() -> Self {
        RoundPlayback::default()
    }

    /// The circle — the attachment's id — this machine is about, or `None` when none is.
    pub fn id(&self) -> Option<i64> {
        self.id
    }

    /// Where it is.
    pub fn phase(&self) -> RoundPhase {
        self.phase
    }

    /// The number of the current load; news carrying another is stale.
    pub fn load(&self) -> u32 {
        self.load
    }

    /// Frames have been drawn into the circle for this load, so the picture is worth showing over
    /// the poster.
    pub fn has_frames(&self) -> bool {
        self.has_frames
    }

    /// This load has been seen playing — what the dot and the ring wait for.
    pub fn seen_playing(&self) -> bool {
        self.seen_playing
    }

    /// Where it is, in seconds.
    pub fn position(&self) -> f64 {
        self.position
    }

    /// How long it is, in seconds; 0 while unknown.
    pub fn total(&self) -> f64 {
        self.total
    }

    /// Loading, playing or paused: a player is (or is about to be) held for it.
    pub fn active(&self) -> bool {
        matches!(
            self.phase(),
            RoundPhase::Loading | RoundPhase::Playing | RoundPhase::Paused
        )
    }

    /// Where the circle `id` is: its own phase when it is this machine's, at rest otherwise.
    pub fn phase_of(&self, id: i64) -> RoundPhase {
        if self.id == Some(id) {
            self.phase()
        } else {
            RoundPhase::Rest
        }
    }

    /// A click on the circle `id` (the single click, after the double-click window that is the
    /// heart): playing, it pauses; loading, it gives up; paused, it plays on; at rest or failed, it
    /// loads — taking the player from any other circle. Nothing STARTS while a recording runs
    /// (S1.7), but a pause is always allowed. `recording`: a voice recording runs, or is being
    /// started.
    pub fn tap(&mut self, id: i64, recording: bool) -> RoundStep {
        if self.id == Some(id) {
            match self.phase() {
                RoundPhase::Playing => {
                    self.phase = RoundPhase::Paused;
                    return RoundStep::new(RoundTap::Paused, RoundAct::Pause);
                }
                RoundPhase::Loading => {
                    // "A second tap while it loads gives up" (S5.3) — the bytes, if they land, land
                    // on nothing.
                    self.rest();
                    return RoundStep::new(RoundTap::GaveUp, RoundAct::Stop);
                }
                RoundPhase::Paused => {
                    if recording {
                        return RoundStep::new(RoundTap::Refused, RoundAct::None);
                    }
                    // A stall's frames are still there; until the player says it plays, it is
                    // loading.
                    self.phase = if self.has_frames {
                        RoundPhase::Playing
                    } else {
                        RoundPhase::Loading
                    };
                    return RoundStep::new(RoundTap::Resumed, RoundAct::Play);
                }
                RoundPhase::Rest | RoundPhase::Failed => {}
            }
        }
        if recording {
            return RoundStep::new(RoundTap::Refused, RoundAct::None);
        }
        let replaced = self.id.filter(|&other| other != id);
        self.id = Some(id);
        self.phase = RoundPhase::Loading;
        self.load = self.load.wrapping_add(1);
        self.has_frames = false;
        self.seen_playing = false;
        self.position = 0.0;
        self.total = 0.0;
        RoundStep {
            tap: RoundTap::Started,
            act: RoundAct::Load,
            replaced,
        }
    }

    /// The player has opened load `load`: whether to play it now — only when it is still this
    /// machine's load and still waited for (a pause meanwhile keeps it paused). `total` is its
    /// length in seconds, 0 when the player cannot say.
    pub fn opened(&mut self, load: u32, total: f64) -> bool {
        if load != self.load || self.phase() != RoundPhase::Loading {
            return false;
        }
        self.total = finite(total);
        true
    }

    /// The player says load `load` is playing.
    pub fn playing(&mut self, load: u32) {
        if load != self.load || !self.active() {
            return;
        }
        self.seen_playing = true;
        self.phase = RoundPhase::Playing;
    }

    /// The player paused load `load` by itself — the system, not a click.
    pub fn paused_by_player(&mut self, load: u32) {
        if load == self.load && self.phase() == RoundPhase::Playing {
            self.phase = RoundPhase::Paused;
        }
    }

    /// The player is waiting for bytes in the middle of load `load`: the loading ring again.
    pub fn waiting(&mut self, load: u32) {
        if load == self.load && self.phase() == RoundPhase::Playing {
            self.phase = RoundPhase::Loading;
        }
    }

    /// A frame of load `load` is in the circle: from now the picture shows over the poster.
    pub fn first_frame(&mut self, load: u32) {
        if load == self.load && self.active() {
            self.has_frames = true;
        }
    }

    /// Where load `load` is, in seconds: the ring and the clock follow it.
    pub fn progress(&mut self, load: u32, position: f64, total: f64) {
        if load != self.load || !self.active() {
            return;
        }
        if total.is_finite() && total > 0.0 {
            self.total = total;
        }
        let most = if self.total > 0.0 {
            self.total
        } else {
            f64::MAX
        };
        self.position = finite(position).clamp(0.0, most);
    }

    /// Load `load` played to its end: back to the poster and the play disc (S5.3), the player let
    /// go — and whether this was a PLAY-THROUGH, so the circle's dot goes: it was seen playing
    /// here. A clip that never started (an end reported straight after opening) does not count.
    pub fn ended(&mut self, load: u32) -> (RoundAct, bool) {
        if load != self.load || !self.active() {
            return (RoundAct::None, false);
        }
        let through = self.seen_playing;
        self.rest();
        (RoundAct::Stop, through)
    }

    /// Load `load` could not be fetched or played: "Couldn't load the video. Tap to try again."
    pub fn failed(&mut self, load: u32) -> RoundAct {
        if load != self.load || !self.active() {
            return RoundAct::None;
        }
        self.phase = RoundPhase::Failed;
        self.has_frames = false;
        self.seen_playing = false;
        self.position = 0.0;
        RoundAct::Stop
    }

    /// Something else began to play — a voice note, the viewer, a recording — so this one makes
    /// way (one thing at a time, S5.3): playing, or stalled with frames up, it pauses where it is;
    /// still loading with nothing drawn, it gives up. (`Yield` in the C#; `yield` is a Rust
    /// keyword.)
    pub fn make_way(&mut self) -> RoundAct {
        match self.phase() {
            RoundPhase::Loading if !self.has_frames => {
                self.rest();
                RoundAct::Stop
            }
            RoundPhase::Playing | RoundPhase::Loading => {
                self.phase = RoundPhase::Paused;
                RoundAct::Pause
            }
            _ => RoundAct::None,
        }
    }

    /// Something happened to the app (S4's last column, [`playback_pauses`]): a call, a lock, a
    /// change of output device or a hidden window makes it [`make_way`](Self::make_way); anything
    /// else leaves it.
    pub fn interrupt(&mut self, happened: PlaybackEvent) -> RoundAct {
        if playback_pauses::pauses(happened, Playing::RoundVideo) {
            self.make_way()
        } else {
            RoundAct::None
        }
    }

    /// Its row left the screen, its chat closed, the window let go of the view: back to the poster,
    /// the player let go. Returns the circle that was stopped, or `None` when there was nothing to
    /// stop.
    pub fn stop(&mut self) -> Option<i64> {
        let id = self.id?;
        self.rest();
        self.id = None;
        Some(id)
    }

    fn rest(&mut self) {
        self.phase = RoundPhase::Rest;
        // A load given up must not be revived by its own late news.
        self.load = self.load.wrapping_add(1);
        self.has_frames = false;
        self.seen_playing = false;
        self.position = 0.0;
    }
}

fn finite(value: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        0.0
    }
}

/// How a circle that plays in place is drawn (S5.2, S5.3, S6) — what shows in each
/// [`RoundPhase`] (`RoundInline`).
pub mod round_inline {
    use fc_core::protocol::AttachmentDto;
    use fc_core::text::string_catalog::StringCatalog;
    use fc_text::media::time_label;

    use super::RoundPhase;
    use crate::round_look;

    /// THE FLAG. True: a click plays the circle in place, and only Open Full Screen opens the
    /// viewer. False: the interim of 2026-10-04, a click opens the viewer (S5.3's old "Windows in
    /// this version"). On Linux GTK clips the playing video to the circle itself (shell-design
    /// §1.5, §10.4), so it is on, and nothing ever turns it off circle by circle — Windows' fallback
    /// for a machine whose frame server failed has no counterpart here.
    pub const PLAYS_IN_PLACE: bool = true;

    /// How often the circle's clock redraws the ring and the time while it plays, in milliseconds.
    pub const TICK_MS: u64 = 100;

    /// The expand control's visual: a 28 dark disc carrying the full-screen icon (S5.4).
    pub const EXPAND_GLYPH: f64 = 28.0;

    /// The expand control's target (S5.4, S1.1).
    pub const EXPAND_TARGET: f64 = 44.0;

    /// The gap between the circle's edge and its ring, which runs OUTSIDE the picture, never over a
    /// face.
    pub const RING_GAP: f64 = 3.0;

    /// How far the ring reaches past the circle on every side.
    pub const RING_REACH: f64 = RING_GAP + round_look::RING;

    /// How far round the ONE accent ring is, 0 to 1 (S5.3): as far as it has played while it
    /// plays, is paused or stalls — stepping once a second, not sweeping, where the desktop asks
    /// for less motion (S6, Reduce Motion: GTK's `gtk-enable-animations` off) — and nothing at
    /// rest, while loading before it ever played, or after a failure. Seconds; `total` 0 or less is
    /// unknown.
    pub fn ring(
        phase: RoundPhase,
        seen_playing: bool,
        position: f64,
        total: f64,
        reduced_motion: bool,
    ) -> f64 {
        if matches!(phase, RoundPhase::Rest | RoundPhase::Failed)
            || !seen_playing
            || !total.is_finite()
            || total <= 0.0
            || !position.is_finite()
        {
            return 0.0;
        }
        let at = if reduced_motion {
            position.floor()
        } else {
            position
        };
        (at / total).clamp(0.0, 1.0)
    }

    /// The play disc: on the poster at rest, paused or failed; faded out while it loads or plays
    /// (S5.3).
    pub fn shows_disc(phase: RoundPhase) -> bool {
        matches!(
            phase,
            RoundPhase::Rest | RoundPhase::Paused | RoundPhase::Failed
        )
    }

    /// The loading ring over the poster: from the click until it plays, and in a stall (S5.3).
    pub fn shows_spinner(phase: RoundPhase) -> bool {
        phase == RoundPhase::Loading
    }

    /// The frames, not the poster, fill the circle: once one has been drawn, until it ends, fails
    /// or stops.
    pub fn shows_frames(phase: RoundPhase, has_frames: bool) -> bool {
        has_frames
            && matches!(
                phase,
                RoundPhase::Playing | RoundPhase::Paused | RoundPhase::Loading
            )
    }

    /// The expand control at the top trailing edge — Open Full Screen — while it plays or is paused
    /// part way (S5.4).
    pub fn shows_expand(phase: RoundPhase, has_frames: bool) -> bool {
        shows_frames(phase, has_frames)
    }

    /// "Couldn't load the video. Tap to try again." under the circle.
    pub fn shows_failure(phase: RoundPhase) -> bool {
        phase == RoundPhase::Failed
    }

    /// The capsule: how far it has played once it has started, its whole length otherwise
    /// ("0:23"). `position` in seconds.
    pub fn capsule(
        video: &AttachmentDto,
        phase: RoundPhase,
        seen_playing: bool,
        position: f64,
    ) -> Option<String> {
        if seen_playing
            && matches!(
                phase,
                RoundPhase::Playing | RoundPhase::Paused | RoundPhase::Loading
            )
        {
            let at = if position.is_finite() { position } else { 0.0 };
            Some(time_label(at.max(0.0)))
        } else {
            round_look::capsule(video)
        }
    }

    /// What a press does next, for a screen reader (S6: Play/Pause is the default action) and the
    /// tooltip: "Pause" while it plays or loads — a press there pauses or gives up — "Play"
    /// otherwise.
    pub fn action(phase: RoundPhase, say: &dyn StringCatalog) -> String {
        if matches!(phase, RoundPhase::Playing | RoundPhase::Loading) {
            say.get("Pause").to_string()
        } else {
            say.get("Play").to_string()
        }
    }

    /// Whether any of a circle is on the screen: its visible rectangle — the part of its parent
    /// scrollers it is seen through, in its own coordinates (`x`, `y`, `view_width`,
    /// `view_height`) — overlaps its own `width` × `height`. An empty rectangle (zero, negative or
    /// NaN size, an infinite corner) is off the screen; an unbounded one (no scroller round it:
    /// infinite size) is on it. A circle wholly out of view stops (S5.3), as the web's
    /// IntersectionObserver does. On GTK the shell computes the rectangle from
    /// `gtk::Widget::compute_bounds` against the scrolled window's viewport.
    pub fn in_view(
        x: f64,
        y: f64,
        view_width: f64,
        view_height: f64,
        width: f64,
        height: f64,
    ) -> bool {
        overlaps(x, view_width, width) && overlaps(y, view_height, height)
    }

    fn overlaps(start: f64, length: f64, size: f64) -> bool {
        if start.is_nan() || length.is_nan() || length <= 0.0 || size <= 0.0 {
            return false;
        }
        if length == f64::INFINITY {
            return true;
        }
        if !start.is_finite() {
            return false;
        }
        start < size && start + length > 0.0
    }
}

/// The state a [`RoundPlayer`] reports it is in (Windows' `MediaPlaybackState`, as the view reads
/// it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlayerState {
    /// Frames and sound are moving.
    Playing,
    /// Paused — by [`RoundPlayer::pause`] or by the system.
    Paused,
    /// Waiting for data in the middle of the clip (a stall).
    Buffering,
}

/// What a [`RoundPlayer`] reports, in the order it happens. Every piece of news is raised on the
/// GLib main context, and never after the player was dropped.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RoundPlayerNews {
    /// The clip is open and prerolled: its length in seconds, 0 when it does not say. Raised once.
    Opened { total: f64 },
    /// Its state changed.
    State(PlayerState),
    /// The first decoded frame is in the picture — from now the circle may show it over the poster.
    /// Raised once.
    FirstFrame,
    /// It played to its end.
    Ended,
    /// It cannot be played (a demuxer, decoder or sink error). Raised at most once; nothing follows.
    Failed,
}

/// One video message playing IN PLACE — the shell's player for ONE load of a circle (Windows'
/// `RoundFramePlayer`, without its frame server: shell-design §10.1, §10.4).
///
/// Implemented over `gst_play::Play` with a `gst_play::PlaySignalAdapter` made
/// `with_main_context(&glib::MainContext::default())`; the video sink is `gtk4paintablesink`, whose
/// `paintable` the circle's `gtk::Picture` (`ContentFit::Cover`, clipped round by its `RoundClip`
/// parent's `overflow: hidden` + `border-radius`) shows over the poster once
/// [`RoundPlayerNews::FirstFrame`] has been reported. The adapter's closures are `Send`, so they
/// forward into a `futures_channel::mpsc` (or hold a `glib::SendWeakRef`) and the news is emitted
/// from the main context — never an `Rc` captured in them.
///
/// Contract:
/// - [`news`](Self::news) emits, on the main context: `Opened { total }` once the pipeline has
///   prerolled (`duration-changed`/`media-info-updated` with the duration in seconds, 0 when
///   unknown); `State(…)` on every `state-changed` (`Playing`, `Paused`, `Buffering`; `Stopped` is
///   not reported); `FirstFrame` once, when the sink's paintable first invalidates its contents (or
///   the first `video-dimensions-changed`); `Ended` at `end-of-stream`; `Failed` on `error` (the
///   GError's domain and code may be logged, never its message, which can quote a path). Nothing is
///   emitted after the player is dropped.
/// - [`play`](Self::play) / [`pause`](Self::pause) / [`rewind`](Self::rewind) (seek to 0) return at
///   once; their effect is reported through the news.
/// - [`position`](Self::position) / [`duration`](Self::duration): seconds, read synchronously from
///   `Play::position()` / `Play::duration()`; 0 when unknown.
/// - Dropping the player sets the pipeline to `Null` and lets go of the sink; the shell gives every
///   circle that showed its paintable its poster back BEFORE dropping it.
/// - Volume: the app's normal output; the rate stays 1× (a circle has no speed chip).
pub trait RoundPlayer {
    fn play(&self);
    fn pause(&self);
    /// Back to the start, for a clip played again from its beginning.
    fn rewind(&self);
    /// Where it is, in seconds.
    fn position(&self) -> f64;
    /// How long it is, in seconds; 0 while unknown.
    fn duration(&self) -> f64;
    /// What it reports ([`RoundPlayerNews`]).
    fn news(&self) -> &Event<RoundPlayerNews>;
}

/// Makes the [`RoundPlayer`] for one load (the C# view's `new RoundFramePlayer(…)` + `Open`).
///
/// Contract: [`open`](Self::open) is handed the clip ALREADY FETCHED through the attachment cache
/// — a local file (the cache's blob for the attachment, played as a `file://` URI; never a network
/// URI: the cache is how circles replay offline and how the session's header reaches the server,
/// Windows' rule) — and its MIME type (`video/mp4` when the attachment does not say). It starts
/// loading at once and returns the player, which reports through its news; it does NOT play until
/// [`RoundPlayer::play`]. `None` when no player can be made at all (no GStreamer element for the
/// sink or the decoder) — the caller then fails the load ([`RoundPlayback::failed`]). Called on the
/// main context; never blocks it.
pub trait RoundPlayers {
    fn open(&self, file: &Path, mime: &str) -> Option<Rc<dyn RoundPlayer>>;
}
