//! `Player`, `NowPlaying`, `Camera` (FROZEN, W.3) — P3's media services, used by P2 (the viewer's
//! video), P3 itself (voice bubbles, circles, the recorder) and P6 (the camera in a call).
//!
//! All on the main thread. GStreamer's own threads never reach these objects: an implementation
//! marshals bus messages and signals back to the main context (§2.4) before it raises an event.

use std::path::PathBuf;
use std::rc::Rc;

use fc_core::rt::{Event, LocalBoxFuture};
use fc_logic::round_look::{PlaybackEvent, Playing};

/// What a player plays (§10.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaSource {
    /// A file already in the attachment cache (`file://`): voice notes and circles, fetched through
    /// the cache on the click — how they replay offline.
    File(PathBuf),
    /// A server URL streamed with authorised Range reads: `souphttpsrc` gets `extra-headers` =
    /// `Authorization: Bearer <token>` on every request (§1.4). The token is never logged.
    Stream { url: String, bearer: Option<String> },
}

/// Where a player is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlayerState {
    /// Nothing loaded, or stopped.
    Idle,
    /// Loading or buffering.
    Loading,
    Playing,
    Paused,
    /// Played to the end (position back at the start is the caller's call).
    Ended,
    /// It cannot play this (a missing element, a refused stream, a broken file).
    Failed,
}

/// One playback (`media::Player`, §10.1): `gstreamer-play` with `gtk4paintablesink` for video,
/// `scaletempo` for rate, the session's header for streams.
pub trait Player {
    /// Load `source` (stops what was loaded). Does not play.
    fn load(&self, source: MediaSource);
    fn play(&self);
    fn pause(&self);
    /// Stop and unload.
    fn stop(&self);
    /// Seek to `seconds` from the start.
    fn seek(&self, seconds: f64);
    /// 1.0, 1.5, 2.0 — the voice speed; pitch kept.
    fn set_rate(&self, rate: f64);
    fn set_muted(&self, muted: bool);
    /// Seconds from the start.
    fn position(&self) -> f64;
    /// Seconds; 0 while unknown.
    fn duration(&self) -> f64;
    fn state(&self) -> PlayerState;
    /// The video's paintable for a `gtk::Picture` (`gtk4paintablesink`'s), `None` for sound.
    fn paintable(&self) -> Option<gtk::gdk::Paintable>;
    /// Raised on every state change.
    fn on_state(&self) -> &Event<PlayerState>;
    /// Raised about ten times a second while playing, with the position.
    fn on_position(&self) -> &Event<f64>;
}

/// Makes players (installed in `Shell::players`).
pub trait Players {
    /// A new player; `video` when it will draw pictures (the sink is made for it).
    fn create(&self, video: bool) -> Rc<dyn Player>;
}

/// The one-thing-plays-at-a-time registry (§10.1, `PlaybackPauses`): a circle starting pauses a
/// voice note and the viewer's video and the other way round; an interruption pauses what
/// `round_look::playback_pauses::pauses` says.
pub trait NowPlaying {
    /// `kind` starts playing: whatever holds the claim is paused first (its `pause` is called),
    /// then this one holds it. Answers the claim's id.
    fn claim(&self, kind: Playing, pause: Rc<dyn Fn()>) -> u64;
    /// The claim ended (paused by the person, played to the end, the widget gone). A claim already
    /// taken over does nothing.
    fn release(&self, claim: u64);
    /// Something happened to the app (a call, a lock, the window hidden, the output changed):
    /// pause the holder when `playback_pauses::pauses(event, kind)`.
    fn interrupt(&self, event: PlaybackEvent);
    /// What holds the claim now.
    fn playing(&self) -> Option<Playing>;
}

/// A camera this machine has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraDevice {
    /// Stable for the session (a PipeWire node's serial, a `/dev/video*` path); what the
    /// `video-camera` setting stores.
    pub id: String,
    /// What the person reads (the device's own name).
    pub name: String,
}

/// Why the camera could not be opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CameraRefusal {
    /// The person (or GNOME on their behalf) said no — never bypassed: "Video messages can't be
    /// recorded without the camera."
    Denied,
    /// No camera at all.
    Absent,
    /// The portal and every fallback failed for another reason.
    Failed,
}

/// The camera (§10.5): the Camera portal, then PipeWire's devices, then `v4l2src`.
pub trait Camera {
    /// Whether a camera can exist (the portal's `IsCameraPresent`, or a device found).
    fn present(&self) -> bool;
    /// The cameras known — Settings' "Camera for video messages", the recorder's "Choose camera".
    fn devices(&self) -> Vec<CameraDevice>;
    /// Raised when the list or `present` changes.
    fn on_devices_changed(&self) -> &Event<()>;
    /// Open a camera (`None`: the chosen one, else the first) as a GStreamer source element ready
    /// to link (`pipewiresrc fd=… target-object=…`, or `v4l2src`), asking for access first.
    fn open(
        &self,
        device: Option<String>,
    ) -> LocalBoxFuture<'static, Result<gst::Element, CameraRefusal>>;
}

/// S0's stand-ins: players that never play, a registry that holds nothing, no camera.
#[derive(Debug, Default)]
pub struct NoMedia {
    state: Event<PlayerState>,
    position: Event<f64>,
    devices: Event<()>,
}

impl Players for NoMedia {
    fn create(&self, _video: bool) -> Rc<dyn Player> {
        Rc::new(NoMedia::default())
    }
}

impl Player for NoMedia {
    fn load(&self, _source: MediaSource) {
        self.state.emit(&PlayerState::Failed);
    }
    fn play(&self) {}
    fn pause(&self) {}
    fn stop(&self) {}
    fn seek(&self, _seconds: f64) {}
    fn set_rate(&self, _rate: f64) {}
    fn set_muted(&self, _muted: bool) {}
    fn position(&self) -> f64 {
        0.0
    }
    fn duration(&self) -> f64 {
        0.0
    }
    fn state(&self) -> PlayerState {
        PlayerState::Idle
    }
    fn paintable(&self) -> Option<gtk::gdk::Paintable> {
        None
    }
    fn on_state(&self) -> &Event<PlayerState> {
        &self.state
    }
    fn on_position(&self) -> &Event<f64> {
        &self.position
    }
}

impl NowPlaying for NoMedia {
    fn claim(&self, _kind: Playing, _pause: Rc<dyn Fn()>) -> u64 {
        0
    }
    fn release(&self, _claim: u64) {}
    fn interrupt(&self, _event: PlaybackEvent) {}
    fn playing(&self) -> Option<Playing> {
        None
    }
}

impl Camera for NoMedia {
    fn present(&self) -> bool {
        false
    }
    fn devices(&self) -> Vec<CameraDevice> {
        Vec::new()
    }
    fn on_devices_changed(&self) -> &Event<()> {
        &self.devices
    }
    fn open(
        &self,
        _device: Option<String>,
    ) -> LocalBoxFuture<'static, Result<gst::Element, CameraRefusal>> {
        Box::pin(std::future::ready(Err(CameraRefusal::Absent)))
    }
}
