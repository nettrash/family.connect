//! [`GstCallMedia`]: fc-logic's [`CallMedia`] on GStreamer's `webrtcbin` — the contract in
//! `fc_logic::calls`' documentation, "The webrtcbin `CallMedia`", implemented as written except
//! where [the module's documentation](super) lists a deviation.
//!
//! **Threads.** Every method runs on the GLib main context. `webrtcbin`'s signals, its promises
//! and the decoders' pads arrive on GStreamer's threads, where a handler only counts, blocks a new
//! pad, and posts a [`Posted`] into a `futures_channel` channel that a `spawn_local` task drains
//! on the main context — never `MainContext::invoke`, which runs AT ONCE on the main thread and
//! would re-enter the engine from inside one of its own calls. Every structural change (an
//! element added, a pad linked, a sink created — `gtk4paintablesink` must be made on the main
//! thread) happens there too, so nothing races a teardown. Each pipeline has a serial; a message
//! from a pipeline that is no longer the call's is dropped, which is how nothing is emitted after
//! [`close`](CallMedia::close).

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use fc_core::protocol::{IceCandidate, IceServerDto};
use fc_core::rt::{Cancel, Event, LocalBoxFuture};
use fc_logic::calls::{CallMedia, MediaGrant, RingTone};
use futures_channel::{mpsc, oneshot};
use futures_util::StreamExt;
use futures_util::future::{Either, select};
use gst::prelude::*;
use gst_webrtc::{
    WebRTCICEConnectionState, WebRTCPeerConnectionState, WebRTCRTPTransceiver,
    WebRTCRTPTransceiverDirection, WebRTCSDPType, WebRTCSessionDescription,
};
use gtk::gdk;

use super::ice::{self, AgentFacts, SilenceRule, SilenceWatch};
use super::pipeline::{self, SendPlan, VideoCodec};
use super::tones::Ringer;

const LOG: &str = "family-connect";

/// Pipeline serials, process-wide: `webrtcdsp` finds its echo probe by NAME among every probe in
/// the process, so two pipelines must never share one.
static NEXT_SERIAL: AtomicU64 = AtomicU64::new(1);

fn next_serial() -> u64 {
    NEXT_SERIAL.fetch_add(1, Ordering::Relaxed)
}

/// Where a call's sound and pictures come from and go: `gst-launch` descriptions, so a test can
/// put `audiotestsrc` and `fakesink` where the desktop has PipeWire and GTK.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallDevices {
    /// A live raw-audio source. `pulsesrc` with no device: PipeWire's default source, which
    /// PipeWire moves when the default changes (§14.2).
    pub microphone: String,
    /// The far side's sound. `pulsesink`, PipeWire's default sink.
    pub speaker: String,
    /// The tones' output. `pulsesink`.
    pub ringer: String,
    /// The far side's picture and the self-view. `gtk4paintablesink`, whose `paintable` the Call
    /// window draws ([`GstCallMedia::remote_video`], [`GstCallMedia::self_view`]).
    pub video_sink: String,
    /// A live raw-video source to use INSTEAD of the [`CallCamera`] — the test switch
    /// (`FC_TEST_VIDEO_SOURCE`); `None` on the desktop.
    pub camera: Option<String>,
}

impl CallDevices {
    /// The desktop's devices.
    pub fn system() -> Self {
        CallDevices {
            microphone: "pulsesrc".into(),
            speaker: "pulsesink".into(),
            ringer: "pulsesink".into(),
            video_sink: "gtk4paintablesink".into(),
            camera: None,
        }
    }

    /// [`system`](Self::system), with the self-test's switches (shell-design §27) where set:
    /// `FC_TEST_AUDIO_SOURCE` (the microphone), `FC_TEST_AUDIO_SINK` (the speaker and the
    /// ringer), `FC_TEST_VIDEO_SINK`, `FC_TEST_VIDEO_SOURCE` (the camera). Each is a `gst-launch`
    /// description; a source must be live (`is-live=true`).
    pub fn from_env() -> Self {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// [`from_env`](Self::from_env) over any lookup — a test's.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let set = |name: &str| lookup(name).filter(|value| !value.trim().is_empty());
        let mut devices = CallDevices::system();
        if let Some(source) = set("FC_TEST_AUDIO_SOURCE") {
            devices.microphone = source;
        }
        if let Some(sink) = set("FC_TEST_AUDIO_SINK") {
            devices.speaker = sink.clone();
            devices.ringer = sink;
        }
        if let Some(sink) = set("FC_TEST_VIDEO_SINK") {
            devices.video_sink = sink;
        }
        devices.camera = set("FC_TEST_VIDEO_SOURCE");
        devices
    }
}

/// A camera opened for a call: a live raw-video source element (a bin with one `src` pad will
/// do), and whatever keeps it open — the portal session, the PipeWire fd — released when this is
/// dropped, which [`close`](CallMedia::close) does after the pipeline is `NULL`.
pub struct CameraFeed {
    pub source: gst::Element,
    pub keep: Box<dyn Any>,
}

/// The camera for a video call: P3's `media::camera` (the Camera portal, `pipewiresrc` on its fd —
/// shell-design §10.5) behind this seam. `None` when the camera is refused, absent or busy — the
/// call then goes on with the microphone alone (a camera is never a reason to miss a call).
pub trait CallCamera {
    fn open(&self) -> LocalBoxFuture<'static, Option<CameraFeed>>;
}

/// No camera at all: every video call is answered and placed camera-off (receive-only video).
/// The seam's default until P3's camera is plugged in.
#[allow(
    dead_code,
    reason = "tools/call-interop's; the app's camera is `camera::ShellCamera`"
)]
pub struct NoCamera;

impl CallCamera for NoCamera {
    fn open(&self) -> LocalBoxFuture<'static, Option<CameraFeed>> {
        Box::pin(async { None })
    }
}

/// What a call's media has carried so far — buffers, counted where they pass: RTP from the far
/// side (`webrtcbin`'s source pads), decoded far-side audio and pictures, and RTP this device
/// encoded toward `webrtcbin` (after the mute and after the camera valve).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CallStats {
    pub audio_rtp_in: u64,
    pub video_rtp_in: u64,
    pub audio_decoded: u64,
    pub video_decoded: u64,
    pub audio_rtp_out: u64,
    pub video_rtp_out: u64,
    /// The loudest decoded far-side sample since the last [`GstCallMedia::stats`], 0–32767.
    pub audio_peak: u32,
}

#[derive(Debug, Default)]
struct Counters {
    audio_rtp_in: AtomicU64,
    video_rtp_in: AtomicU64,
    audio_decoded: AtomicU64,
    video_decoded: AtomicU64,
    audio_rtp_out: AtomicU64,
    video_rtp_out: AtomicU64,
    audio_peak: AtomicU32,
}

impl Counters {
    fn snapshot(&self) -> CallStats {
        CallStats {
            audio_rtp_in: self.audio_rtp_in.load(Ordering::Relaxed),
            video_rtp_in: self.video_rtp_in.load(Ordering::Relaxed),
            audio_decoded: self.audio_decoded.load(Ordering::Relaxed),
            video_decoded: self.video_decoded.load(Ordering::Relaxed),
            audio_rtp_out: self.audio_rtp_out.load(Ordering::Relaxed),
            video_rtp_out: self.video_rtp_out.load(Ordering::Relaxed),
            audio_peak: self.audio_peak.swap(0, Ordering::Relaxed),
        }
    }

    fn rtp_in(&self, kind: Kind) -> &AtomicU64 {
        match kind {
            Kind::Audio => &self.audio_rtp_in,
            Kind::Video => &self.video_rtp_in,
        }
    }

    fn rtp_out(&self, kind: Kind) -> &AtomicU64 {
        match kind {
            Kind::Audio => &self.audio_rtp_out,
            Kind::Video => &self.video_rtp_out,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Audio,
    Video,
}

/// What a GStreamer thread hands the main context, stamped with its pipeline's serial.
enum Posted {
    Candidate {
        serial: u64,
        mline: u32,
        candidate: String,
    },
    Ice {
        serial: u64,
        state: WebRTCICEConnectionState,
    },
    Peer {
        serial: u64,
        state: WebRTCPeerConnectionState,
    },
    /// A far-side stream on `webrtcbin`, held by a blocking probe until it is linked.
    Stream {
        serial: u64,
        pad: gst::Pad,
        block: Option<gst::PadProbeId>,
    },
    /// A decoded far-side stream, held the same way.
    Decoded {
        serial: u64,
        pad: gst::Pad,
        block: Option<gst::PadProbeId>,
    },
    StreamGone {
        serial: u64,
    },
}

struct CameraParts {
    bin: gst::Bin,
    valve: gst::Element,
    self_view: Option<gdk::Paintable>,
    /// Dropped after the pipeline is `NULL`: the portal session and the fd.
    _keep: Box<dyn Any>,
}

/// One call's pipeline and what is known about it.
struct Call {
    serial: u64,
    /// Raised by `close`: every pending future resolves to its failure value.
    closing: Cancel,
    counters: Arc<Counters>,
    pipeline: gst::Pipeline,
    _bus: Option<gst::bus::BusWatchGuard>,
    /// The microphone bin, up to `volume name=mute`.
    microphone: gst::Element,
    mute: Option<gst::Element>,
    echo: gst::Element,
    camera: Option<CameraParts>,
    webrtc: Option<gst::Element>,
    receive_video_only: bool,
    linked: bool,
    connected: bool,
    failed: bool,
    camera_on: bool,
    remote_video: Option<gdk::Paintable>,
}

struct Inner {
    devices: CallDevices,
    camera: Rc<dyn CallCamera>,
    silence: Cell<SilenceRule>,
    offered: Cell<VideoCodec>,
    ringer: Ringer,
    post: mpsc::UnboundedSender<Posted>,
    call: RefCell<Option<Call>>,
    last: Cell<CallStats>,
    gathered: Event<IceCandidate>,
    connected: Event<()>,
    failed: Event<()>,
    changed: Event<()>,
}

/// fc-logic's [`CallMedia`] on `webrtcbin` — ONE per window, kept across calls (and across
/// servers), each call's pipeline built by `open` and `connect` and dropped by `close`. Make it
/// on the GLib main context, with GStreamer initialised (it initialises it if not).
pub struct GstCallMedia {
    inner: Rc<Inner>,
}

impl GstCallMedia {
    /// The window's media over `devices` ([`CallDevices::from_env`] in the app — the system's
    /// unless a self-test switch is set) and `camera` ([`NoCamera`] until P3's portal camera).
    pub fn new(devices: CallDevices, camera: Rc<dyn CallCamera>) -> Self {
        if let Err(error) = gst::init() {
            glib::g_warning!(LOG, "call: GStreamer did not initialise: {error}");
        }
        let (post, mut posted) = mpsc::unbounded();
        let inner = Rc::new(Inner {
            ringer: Ringer::new(devices.ringer.clone()),
            devices,
            camera,
            silence: Cell::new(SilenceRule::Fallback),
            offered: Cell::new(VideoCodec::Vp8),
            post,
            call: RefCell::new(None),
            last: Cell::new(CallStats::default()),
            gathered: Event::new(),
            connected: Event::new(),
            failed: Event::new(),
            changed: Event::new(),
        });
        let weak = Rc::downgrade(&inner);
        glib::MainContext::ref_thread_default().spawn_local(async move {
            while let Some(message) = posted.next().await {
                let Some(inner) = weak.upgrade() else {
                    break;
                };
                inner.posted(message);
            }
        });
        GstCallMedia { inner }
    }

    /// When the dead-peer fallback runs ([`ice`](super::ice)'s documentation):
    /// [`SilenceRule::Fallback`] unless told otherwise.
    #[allow(dead_code, reason = "tools/call-interop's (the app runs the default)")]
    pub fn with_silence_rule(self, rule: SilenceRule) -> Self {
        self.inner.silence.set(rule);
        self
    }

    /// The video codec this device's offers carry: VP8 (what every peer takes) unless told
    /// otherwise — H.264 is for the self-test's and the harness's H.264-only offer.
    #[allow(dead_code, reason = "tools/call-interop's (the app runs the default)")]
    pub fn with_offered_video(self, codec: VideoCodec) -> Self {
        self.inner.offered.set(codec);
        self
    }

    /// The far side's picture, once its stream is linked to a sink that has one (the window
    /// attaches it again on every `on_media_changed`); `None` before, without video, and after
    /// `close`.
    pub fn remote_video(&self) -> Option<gdk::Paintable> {
        self.inner.call.borrow().as_ref()?.remote_video.clone()
    }

    /// This device's camera, mirrored — from an `open(true)` that got the camera until `close`.
    /// The window hides it while `CallState::camera` is off.
    pub fn self_view(&self) -> Option<gdk::Paintable> {
        self.inner
            .call
            .borrow()
            .as_ref()?
            .camera
            .as_ref()?
            .self_view
            .clone()
    }

    /// The current call's counts, or the last call's after `close` (diagnostics, the self-test,
    /// the interop harness). Reading resets [`CallStats::audio_peak`].
    #[allow(dead_code, reason = "the self-test's and tools/call-interop's")]
    pub fn stats(&self) -> CallStats {
        match self.inner.call.borrow().as_ref() {
            Some(call) => call.counters.snapshot(),
            None => self.inner.last.get(),
        }
    }

    /// The tone sounding now, if any.
    #[allow(dead_code, reason = "the self-test's and tools/call-interop's")]
    pub fn ringing(&self) -> Option<RingTone> {
        self.inner.ringer.tone()
    }

    /// The current call's pipeline — for the self-test's and the harness's eyes only (its state,
    /// a weak reference that proves it was freed).
    #[allow(dead_code, reason = "tools/call-interop's")]
    pub fn pipeline(&self) -> Option<gst::Pipeline> {
        Some(self.inner.call.borrow().as_ref()?.pipeline.clone())
    }

    /// How the libnice agent under the current call's `webrtcbin` was made (`None` before ICE
    /// has a transport).
    #[allow(dead_code, reason = "tools/call-interop's")]
    pub fn agent_facts(&self) -> Option<AgentFacts> {
        let webrtc = self.inner.call.borrow().as_ref()?.webrtc.clone()?;
        ice::agent_facts(&webrtc)
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.release();
    }
}

impl CallMedia for GstCallMedia {
    fn open(&self, video: bool) -> LocalBoxFuture<'static, MediaGrant> {
        let inner = self.inner.clone();
        Box::pin(async move { inner.open(video).await })
    }

    fn connect(
        &self,
        servers: Vec<IceServerDto>,
        receive_video_only: bool,
    ) -> LocalBoxFuture<'static, bool> {
        let connected = self.inner.connect(&servers, receive_video_only);
        Box::pin(async move { connected })
    }

    fn create_offer(&self) -> LocalBoxFuture<'static, Option<String>> {
        let inner = self.inner.clone();
        Box::pin(async move { inner.create_offer().await })
    }

    fn set_remote(&self, is_offer: bool, sdp: String) -> LocalBoxFuture<'static, bool> {
        let inner = self.inner.clone();
        Box::pin(async move { inner.set_remote(is_offer, sdp).await })
    }

    fn create_answer(&self) -> LocalBoxFuture<'static, Option<String>> {
        let inner = self.inner.clone();
        Box::pin(async move { inner.create_answer().await })
    }

    fn add_candidate(&self, candidate: IceCandidate) -> LocalBoxFuture<'static, ()> {
        if let Some((index, text)) = ice::remote(&candidate)
            && let Some(webrtc) = self.inner.webrtc()
        {
            webrtc.emit_by_name::<()>("add-ice-candidate", &[&index, &text]);
        }
        Box::pin(async {})
    }

    fn is_connected(&self) -> bool {
        self.inner
            .call
            .borrow()
            .as_ref()
            .is_some_and(|call| call.connected)
    }

    fn set_muted(&self, muted: bool) {
        if let Some(mute) = self
            .inner
            .call
            .borrow()
            .as_ref()
            .and_then(|call| call.mute.as_ref())
        {
            mute.set_property("mute", muted);
        }
    }

    fn set_camera(&self, on: bool) {
        if let Some(call) = self.inner.call.borrow_mut().as_mut() {
            call.camera_on = on;
            if call.linked
                && let Some(camera) = &call.camera
            {
                camera.valve.set_property("drop", !on);
            }
        }
    }

    fn ring(&self, tone: RingTone) {
        self.inner.ringer.ring(tone);
    }

    fn stop_ringing(&self) {
        self.inner.ringer.stop();
    }

    fn close(&self) {
        self.inner.ringer.stop();
        self.inner.release();
    }

    fn on_candidate_gathered(&self) -> &Event<IceCandidate> {
        &self.inner.gathered
    }

    fn on_connected(&self) -> &Event<()> {
        &self.inner.connected
    }

    fn on_failed(&self) -> &Event<()> {
        &self.inner.failed
    }

    fn on_media_changed(&self) -> &Event<()> {
        &self.inner.changed
    }
}

impl Inner {
    // ---- the call's life ------------------------------------------------------------------

    /// Everything the current call holds given back — the pipeline `NULL`, then the camera —
    /// and every pending future resolved as a failure. Emits nothing.
    fn release(&self) {
        let call = self.call.borrow_mut().take();
        if let Some(call) = call {
            call.closing.cancel();
            self.last.set(call.counters.snapshot());
            if call.pipeline.set_state(gst::State::Null).is_err() {
                glib::g_warning!(LOG, "call {}: the pipeline refused NULL", call.serial);
            }
            glib::g_info!(LOG, "call {}: closed", call.serial);
            // The camera's portal session and fd go with `call`, after the pipeline let go.
        }
    }

    async fn open(self: Rc<Self>, video: bool) -> MediaGrant {
        self.release();
        let serial = next_serial();
        let closing = Cancel::new();
        let Some(call) = self.build(serial, &closing) else {
            return MediaGrant::None;
        };
        *self.call.borrow_mut() = Some(call);
        if !video {
            return MediaGrant::Microphone;
        }
        // The call is in place before the camera is asked for: a `close` (or the next `open`)
        // meanwhile raises its `closing`, and the wait below ends there.
        let feed = match &self.devices.camera {
            Some(description) => launch_camera(description),
            None => {
                let asking = self.camera.open();
                match select(asking, closing.cancelled()).await {
                    Either::Left((feed, _)) => feed,
                    Either::Right(_) => None,
                }
            }
        };
        if closing.is_cancelled() || !self.is_current(serial) {
            // Closed while the camera was being asked for: what it opened drops here.
            return MediaGrant::None;
        }
        match feed {
            Some(feed) => {
                if self.add_camera(serial, feed) {
                    MediaGrant::MicrophoneAndCamera
                } else {
                    MediaGrant::Microphone
                }
            }
            None => MediaGrant::Microphone,
        }
    }

    /// The microphone branch and the echo probe, `PAUSED` to prove the source opens.
    fn build(self: &Rc<Self>, serial: u64, closing: &Cancel) -> Option<Call> {
        let pipeline = gst::Pipeline::with_name(&format!("fc-call-{serial}"));
        let probe = pipeline::echo_probe_name(serial);
        let microphone = match gst::parse::bin_from_description(
            &pipeline::microphone(&self.devices.microphone, &probe),
            true,
        ) {
            Ok(bin) => bin,
            Err(error) => {
                glib::g_warning!(
                    LOG,
                    "call {serial}: no microphone branch: {}",
                    error.message()
                );
                return None;
            }
        };
        set_phone_role(microphone.upcast_ref());
        let mute = microphone.by_name("mute");
        let echo = gst::ElementFactory::make("webrtcechoprobe")
            .name(probe.as_str())
            .build()
            .ok()?;
        if pipeline
            .add_many([microphone.upcast_ref::<gst::Element>(), &echo])
            .is_err()
        {
            return None;
        }
        if pipeline.set_state(gst::State::Paused).is_err() {
            glib::g_warning!(LOG, "call {serial}: the microphone did not open");
            let _ = pipeline.set_state(gst::State::Null);
            return None;
        }
        let bus = watch_bus(&pipeline, serial);
        glib::g_info!(LOG, "call {serial}: microphone open");
        Some(Call {
            serial,
            closing: closing.clone(),
            counters: Arc::new(Counters::default()),
            pipeline,
            _bus: bus,
            microphone: microphone.upcast(),
            mute,
            echo,
            camera: None,
            webrtc: None,
            receive_video_only: false,
            linked: false,
            connected: false,
            failed: false,
            camera_on: true,
            remote_video: None,
        })
    }

    /// The camera branch — the self-view and the shut valve — beside the microphone; `false`
    /// (and nothing left behind) when it does not start.
    fn add_camera(&self, serial: u64, feed: CameraFeed) -> bool {
        let mut guard = self.call.borrow_mut();
        let Some(call) = guard.as_mut().filter(|call| call.serial == serial) else {
            return false;
        };
        let Ok(bin) =
            gst::parse::bin_from_description(&pipeline::camera(&self.devices.video_sink), true)
        else {
            return false;
        };
        let Some(valve) = bin.by_name("camera") else {
            return false;
        };
        let source = feed.source;
        if call
            .pipeline
            .add_many([&source, bin.upcast_ref::<gst::Element>()])
            .is_err()
        {
            return false;
        }
        let started = source.link(&bin).is_ok()
            && bin.sync_state_with_parent().is_ok()
            && source.sync_state_with_parent().is_ok();
        if !started {
            glib::g_warning!(LOG, "call {serial}: the camera did not start");
            let _ = source.set_state(gst::State::Null);
            let _ = bin.set_state(gst::State::Null);
            let _ = call
                .pipeline
                .remove_many([&source, bin.upcast_ref::<gst::Element>()]);
            return false;
        }
        let self_view = paintable_in(&bin);
        call.camera = Some(CameraParts {
            bin,
            valve,
            self_view,
            _keep: feed.keep,
        });
        call.camera_on = true;
        glib::g_info!(LOG, "call {serial}: camera open");
        true
    }

    fn connect(self: &Rc<Self>, servers: &[IceServerDto], receive_video_only: bool) -> bool {
        let mut guard = self.call.borrow_mut();
        let Some(call) = guard.as_mut().filter(|call| call.webrtc.is_none()) else {
            return false;
        };
        let Ok(webrtc) = gst::ElementFactory::make("webrtcbin")
            .name("call")
            .property_from_str("bundle-policy", "max-bundle")
            .build()
        else {
            glib::g_warning!(LOG, "call {}: no webrtcbin", call.serial);
            return false;
        };
        let config = ice::configure(&webrtc, servers);
        self.wire(&webrtc, call.serial, &call.counters);
        if call.pipeline.add(&webrtc).is_err() || webrtc.sync_state_with_parent().is_err() {
            let _ = webrtc.set_state(gst::State::Null);
            return false;
        }
        glib::g_info!(
            LOG,
            "call {}: connecting (stun {}, turn {}, receive-only video {receive_video_only})",
            call.serial,
            config.stun_server.is_some(),
            config.turn_servers.len()
        );
        call.webrtc = Some(webrtc);
        call.receive_video_only = receive_video_only;
        call.connected = false;
        true
    }

    /// `webrtcbin`'s signals, each posting to the main context.
    fn wire(&self, webrtc: &gst::Element, serial: u64, counters: &Arc<Counters>) {
        let post = self.post.clone();
        webrtc.connect("on-ice-candidate", false, move |values| {
            let mline = values.get(1).and_then(|value| value.get::<u32>().ok());
            let candidate = values.get(2).and_then(|value| value.get::<String>().ok());
            if let (Some(mline), Some(candidate)) = (mline, candidate) {
                let _ = post.unbounded_send(Posted::Candidate {
                    serial,
                    mline,
                    candidate,
                });
            }
            None
        });
        let post = self.post.clone();
        webrtc.connect_notify(Some("ice-connection-state"), move |webrtc, _| {
            let state = webrtc.property::<WebRTCICEConnectionState>("ice-connection-state");
            let _ = post.unbounded_send(Posted::Ice { serial, state });
        });
        let post = self.post.clone();
        webrtc.connect_notify(Some("connection-state"), move |webrtc, _| {
            let state = webrtc.property::<WebRTCPeerConnectionState>("connection-state");
            let _ = post.unbounded_send(Posted::Peer { serial, state });
        });
        let post = self.post.clone();
        let counters = counters.clone();
        webrtc.connect_pad_added(move |_, pad| {
            if pad.direction() != gst::PadDirection::Src {
                return;
            }
            let counted = counters.clone();
            pad.add_probe(
                gst::PadProbeType::BUFFER | gst::PadProbeType::BUFFER_LIST,
                move |pad, info| {
                    if let Some(kind) = rtp_kind(pad) {
                        counted
                            .rtp_in(kind)
                            .fetch_add(buffers(info), Ordering::Relaxed);
                    }
                    gst::PadProbeReturn::Ok
                },
            );
            let block = pad.add_probe(gst::PadProbeType::BLOCK_DOWNSTREAM, |_, _| {
                gst::PadProbeReturn::Ok
            });
            let _ = post.unbounded_send(Posted::Stream {
                serial,
                pad: pad.clone(),
                block,
            });
        });
        let post = self.post.clone();
        webrtc.connect_pad_removed(move |_, pad| {
            if pad.direction() == gst::PadDirection::Src {
                let _ = post.unbounded_send(Posted::StreamGone { serial });
            }
        });
    }

    // ---- negotiation ----------------------------------------------------------------------

    /// The current call's `webrtcbin` and its closing flag.
    fn ready(&self) -> Option<(gst::Element, Cancel)> {
        let guard = self.call.borrow();
        let call = guard.as_ref()?;
        Some((call.webrtc.clone()?, call.closing.clone()))
    }

    fn webrtc(&self) -> Option<gst::Element> {
        self.call.borrow().as_ref()?.webrtc.clone()
    }

    async fn create_offer(self: Rc<Self>) -> Option<String> {
        let (webrtc, closing) = self.ready()?;
        if !self.link(SendPlan::offering(self.offered.get()), None) {
            return None;
        }
        let reply = promised(&closing, |promise| {
            webrtc.emit_by_name::<()>("create-offer", &[&None::<gst::Structure>, promise]);
        })
        .await
        .ok()??;
        let offer = reply.get::<WebRTCSessionDescription>("offer").ok()?;
        self.describe(&webrtc, &closing, offer).await
    }

    async fn create_answer(self: Rc<Self>) -> Option<String> {
        let (webrtc, closing) = self.ready()?;
        let reply = promised(&closing, |promise| {
            webrtc.emit_by_name::<()>("create-answer", &[&None::<gst::Structure>, promise]);
        })
        .await
        .ok()??;
        let answer = reply.get::<WebRTCSessionDescription>("answer").ok()?;
        self.describe(&webrtc, &closing, answer).await
    }

    /// `set-local-description`, awaited: the description's SDP, or `None`.
    async fn describe(
        &self,
        webrtc: &gst::Element,
        closing: &Cancel,
        description: WebRTCSessionDescription,
    ) -> Option<String> {
        let reply = promised(closing, |promise| {
            webrtc.emit_by_name::<()>("set-local-description", &[&description, promise]);
        })
        .await
        .ok()?;
        if refused(&reply) {
            return None;
        }
        description.sdp().as_text().ok()
    }

    async fn set_remote(self: Rc<Self>, is_offer: bool, sdp: String) -> bool {
        let Ok(message) = gst_sdp::SDPMessage::parse_buffer(sdp.as_bytes()) else {
            return false;
        };
        let Some((webrtc, closing)) = self.ready() else {
            return false;
        };
        if is_offer && !self.link(SendPlan::answering(&sdp), Some(&sdp)) {
            return false;
        }
        let kind = if is_offer {
            WebRTCSDPType::Offer
        } else {
            WebRTCSDPType::Answer
        };
        let description = WebRTCSessionDescription::new(kind, message);
        match promised(&closing, |promise| {
            webrtc.emit_by_name::<()>("set-remote-description", &[&description, promise]);
        })
        .await
        {
            Ok(reply) => !refused(&reply),
            Err(()) => false,
        }
    }

    /// The send branches linked to `webrtcbin` under `plan` — once per call — and the pipeline
    /// `PLAYING`. Answering (`offer` given), an offer's video m-line this device has no camera
    /// for gets a `recvonly` transceiver in the offer's codec, so the far side's picture of a
    /// camera-off video call still shows, and a voice call's stray video m-line is answered
    /// without a stream of ours.
    fn link(&self, plan: SendPlan, offer: Option<&str>) -> bool {
        let mut guard = self.call.borrow_mut();
        let Some(call) = guard.as_mut() else {
            return false;
        };
        if call.linked {
            return true;
        }
        let Some(webrtc) = call.webrtc.clone() else {
            return false;
        };
        let serial = call.serial;
        let Some(audio) = tail(&pipeline::audio_encoder(plan.audio_pt)) else {
            return false;
        };
        if call.pipeline.add(&audio).is_err()
            || call.microphone.link(&audio).is_err()
            || !link_to(&audio, &webrtc)
        {
            glib::g_warning!(LOG, "call {serial}: the microphone did not link");
            return false;
        }
        count_out(&audio, &call.counters, Kind::Audio);
        let mut tails = vec![audio];
        if let Some(camera) = &call.camera {
            let Some(video) = tail(&pipeline::video_encoder(plan.video, plan.video_pt)) else {
                return false;
            };
            if call.pipeline.add(&video).is_err()
                || camera.bin.link(&video).is_err()
                || !link_to(&video, &webrtc)
            {
                glib::g_warning!(LOG, "call {serial}: the camera did not link");
                return false;
            }
            count_out(&video, &call.counters, Kind::Video);
            camera.valve.set_property("drop", !call.camera_on);
            tails.push(video);
        } else if call.receive_video_only
            || offer.is_some_and(|offer| pipeline::has_media(offer, "video"))
        {
            let caps = pipeline::receive_only_caps(plan.video, plan.video_pt);
            let Ok(caps) = caps.parse::<gst::Caps>() else {
                return false;
            };
            webrtc.emit_by_name::<WebRTCRTPTransceiver>(
                "add-transceiver",
                &[&WebRTCRTPTransceiverDirection::Recvonly, &caps],
            );
        }
        for tail in &tails {
            if tail.sync_state_with_parent().is_err() {
                return false;
            }
        }
        if call.pipeline.set_state(gst::State::Playing).is_err() {
            glib::g_warning!(LOG, "call {serial}: the pipeline did not play");
            return false;
        }
        call.linked = true;
        let video = if call.camera.is_some() {
            format!(", {}/{}", plan.video.encoding_name(), plan.video_pt)
        } else if call.receive_video_only
            || offer.is_some_and(|offer| pipeline::has_media(offer, "video"))
        {
            format!(
                ", receiving {}/{}",
                plan.video.encoding_name(),
                plan.video_pt
            )
        } else {
            String::new()
        };
        glib::g_info!(LOG, "call {serial}: sending opus/{}{video}", plan.audio_pt);
        true
    }

    // ---- what GStreamer's threads posted ----------------------------------------------------

    fn is_current(&self, serial: u64) -> bool {
        self.call
            .borrow()
            .as_ref()
            .is_some_and(|call| call.serial == serial)
    }

    fn posted(self: &Rc<Self>, message: Posted) {
        match message {
            Posted::Candidate {
                serial,
                mline,
                candidate,
            } => {
                if self.is_current(serial)
                    && let Some(candidate) = ice::gathered(mline, candidate)
                {
                    self.gathered.emit(&candidate);
                }
            }
            Posted::Ice { serial, state } => self.ice_state(serial, state),
            Posted::Peer { serial, state } => {
                if self.is_current(serial) {
                    glib::g_debug!(LOG, "call {serial}: peer connection {state:?}");
                    if state == WebRTCPeerConnectionState::Failed {
                        self.report_failed(serial);
                    }
                }
            }
            Posted::Stream { serial, pad, block } => self.stream(serial, &pad, block),
            Posted::Decoded { serial, pad, block } => self.decoded(serial, &pad, block),
            Posted::StreamGone { serial } => {
                if self.is_current(serial) {
                    self.changed.emit(&());
                }
            }
        }
    }

    fn ice_state(self: &Rc<Self>, serial: u64, state: WebRTCICEConnectionState) {
        let first_up = {
            let mut guard = self.call.borrow_mut();
            let Some(call) = guard.as_mut().filter(|call| call.serial == serial) else {
                return;
            };
            match state {
                WebRTCICEConnectionState::Connected | WebRTCICEConnectionState::Completed => {
                    let first = !call.connected;
                    call.connected = true;
                    Some(first)
                }
                _ => None,
            }
        };
        glib::g_info!(LOG, "call {serial}: ice {state:?}");
        match first_up {
            Some(first) => {
                if first {
                    self.watch_silence(serial);
                }
                self.connected.emit(&());
            }
            None if state == WebRTCICEConnectionState::Failed => self.report_failed(serial),
            None => {}
        }
    }

    /// `failed`, once per call, for the current call only.
    fn report_failed(&self, serial: u64) {
        let first = {
            let mut guard = self.call.borrow_mut();
            let Some(call) = guard.as_mut().filter(|call| call.serial == serial) else {
                return;
            };
            !std::mem::replace(&mut call.failed, true)
        };
        if first {
            glib::g_info!(LOG, "call {serial}: failed");
            self.failed.emit(&());
        }
    }

    /// The dead-peer fallback ([`ice`]'s documentation): armed by the [`SilenceRule`] for this
    /// call's agent; once armed, the far side's audio RTP is looked at every second, and
    /// [`ice::SILENCE_LIMIT`] without a packet is `failed`.
    fn watch_silence(self: &Rc<Self>, serial: u64) {
        let Some((counters, webrtc)) = self
            .call
            .borrow()
            .as_ref()
            .and_then(|call| Some((call.counters.clone(), call.webrtc.clone()?)))
        else {
            return;
        };
        let facts = ice::agent_facts(&webrtc);
        let rule = self.silence.get();
        if !rule.arms(facts) {
            glib::g_debug!(LOG, "call {serial}: consent freshness on; no silence watch");
            return;
        }
        glib::g_info!(
            LOG,
            "call {serial}: silence watch armed ({rule:?}, {facts:?})"
        );
        let weak = Rc::downgrade(self);
        let started = Instant::now();
        let mut watch = SilenceWatch::new(ice::SILENCE_LIMIT);
        glib::timeout_add_local(Duration::from_secs(1), move || {
            let Some(inner) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if !inner.is_current(serial) {
                return glib::ControlFlow::Break;
            }
            let heard = counters.audio_rtp_in.load(Ordering::Relaxed);
            if watch.observe(started.elapsed(), heard) {
                glib::g_warning!(
                    LOG,
                    "call {serial}: no far-side audio for {} s",
                    ice::SILENCE_LIMIT.as_secs()
                );
                inner.report_failed(serial);
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
    }

    /// A far-side stream: into a `decodebin3`, then let go of.
    fn stream(&self, serial: u64, pad: &gst::Pad, block: Option<gst::PadProbeId>) {
        let pipeline = match self.call.borrow().as_ref() {
            Some(call) if call.serial == serial => call.pipeline.clone(),
            // An old pipeline's: its pads flush with it.
            _ => return,
        };
        let linked = gst::ElementFactory::make("decodebin3")
            .build()
            .ok()
            .is_some_and(|decoder| {
                let post = self.post.clone();
                decoder.connect_pad_added(move |_, pad| {
                    let block = pad.add_probe(gst::PadProbeType::BLOCK_DOWNSTREAM, |_, _| {
                        gst::PadProbeReturn::Ok
                    });
                    let _ = post.unbounded_send(Posted::Decoded {
                        serial,
                        pad: pad.clone(),
                        block,
                    });
                });
                pipeline.add(&decoder).is_ok()
                    && decoder.sync_state_with_parent().is_ok()
                    && decoder
                        .static_pad("sink")
                        .is_some_and(|sink| pad.link(&sink).is_ok())
            });
        if !linked {
            glib::g_warning!(LOG, "call {serial}: a far-side stream did not link");
        }
        if let Some(block) = block {
            pad.remove_probe(block);
        }
    }

    /// A decoded far-side stream: sound through the echo probe to the speaker, the picture to
    /// its sink — then the window told.
    fn decoded(&self, serial: u64, pad: &gst::Pad, block: Option<gst::PadProbeId>) {
        let linked = {
            let mut guard = self.call.borrow_mut();
            let Some(call) = guard.as_mut().filter(|call| call.serial == serial) else {
                return;
            };
            match decoded_kind(pad) {
                Some(Kind::Audio) => {
                    count_decoded(pad, &call.counters, Kind::Audio);
                    self.link_speaker(call, pad)
                }
                Some(Kind::Video) => {
                    count_decoded(pad, &call.counters, Kind::Video);
                    self.link_picture(call, pad)
                }
                None => false,
            }
        };
        if !linked {
            glib::g_warning!(LOG, "call {serial}: a decoded stream did not link");
        }
        if let Some(block) = block {
            pad.remove_probe(block);
        }
        if linked {
            self.changed.emit(&());
        }
    }

    fn link_speaker(&self, call: &Call, pad: &gst::Pad) -> bool {
        let (Some(head), Some(tail)) = (
            self::tail(pipeline::SPEAKER_HEAD),
            self::tail(&pipeline::speaker_tail(&self.devices.speaker)),
        ) else {
            return false;
        };
        set_phone_role(&tail);
        call.pipeline.add_many([&head, &tail]).is_ok()
            && head.link(&call.echo).is_ok()
            && call.echo.link(&tail).is_ok()
            && tail.sync_state_with_parent().is_ok()
            && head.sync_state_with_parent().is_ok()
            && head
                .static_pad("sink")
                .is_some_and(|sink| pad.link(&sink).is_ok())
    }

    fn link_picture(&self, call: &mut Call, pad: &gst::Pad) -> bool {
        let Some(picture) = tail(&pipeline::picture(&self.devices.video_sink)) else {
            return false;
        };
        let linked = call.pipeline.add(&picture).is_ok()
            && picture.sync_state_with_parent().is_ok()
            && picture
                .static_pad("sink")
                .is_some_and(|sink| pad.link(&sink).is_ok());
        if linked && let Some(bin) = picture.downcast_ref::<gst::Bin>() {
            call.remote_video = paintable_in(bin);
        }
        linked
    }
}

/// What a `gst::Promise` answered — `Ok` with its reply (which may carry no structure) — or
/// `Err` when it was interrupted, expired, freed unanswered, or the call closed first.
async fn promised(
    closing: &Cancel,
    emit: impl FnOnce(&gst::Promise),
) -> Result<Option<gst::Structure>, ()> {
    let (answer, answered) = oneshot::channel();
    let promise = gst::Promise::with_change_func(move |reply| {
        let _ = answer.send(match reply {
            Ok(reply) => Ok(reply.map(ToOwned::to_owned)),
            Err(_) => Err(()),
        });
    });
    emit(&promise);
    drop(promise);
    match select(answered, closing.cancelled()).await {
        Either::Left((Ok(reply), _)) => reply,
        _ => Err(()),
    }
}

/// A `webrtcbin` reply that carries an `error`.
fn refused(reply: &Option<gst::Structure>) -> bool {
    reply.as_ref().is_some_and(|reply| reply.has_field("error"))
}

/// A bin from a description, its unlinked pads ghosted.
fn tail(description: &str) -> Option<gst::Element> {
    gst::parse::bin_from_description(description, true)
        .map(|bin| bin.upcast())
        .ok()
}

/// A tail's `src` into a new `webrtcbin` sink pad — which makes the transceiver.
fn link_to(tail: &gst::Element, webrtc: &gst::Element) -> bool {
    match (tail.static_pad("src"), webrtc.request_pad_simple("sink_%u")) {
        (Some(src), Some(sink)) => src.link(&sink).is_ok(),
        _ => false,
    }
}

/// A camera from a description — the test switch.
fn launch_camera(description: &str) -> Option<CameraFeed> {
    let source = gst::parse::bin_from_description(description, true).ok()?;
    Some(CameraFeed {
        source: source.upcast(),
        keep: Box::new(()),
    })
}

/// `media.role=phone` on every PulseAudio element in `element` (itself, or inside it when it is a
/// bin), so PipeWire's policy can treat the call — and its ring — as a phone call.
pub(super) fn set_phone_role(element: &gst::Element) {
    let mut elements = vec![element.clone()];
    if let Some(bin) = element.downcast_ref::<gst::Bin>() {
        elements.extend(bin.iterate_recurse().into_iter().flatten());
    }
    for element in elements {
        if element.has_property("stream-properties") {
            let properties = gst::Structure::builder("properties")
                .field("media.role", "phone")
                .build();
            element.set_property("stream-properties", properties);
        }
    }
}

/// The `paintable` of the first element in `bin` that has one (`gtk4paintablesink`). Main thread
/// only — the sink makes its paintable there.
fn paintable_in(bin: &gst::Bin) -> Option<gdk::Paintable> {
    bin.iterate_recurse()
        .into_iter()
        .flatten()
        .find(|element| element.has_property_with_type("paintable", gdk::Paintable::static_type()))
        .map(|element| element.property::<gdk::Paintable>("paintable"))
}

/// The bus, watched on the main context: errors logged (the element and GStreamer's sentence —
/// never an SDP), and latency recomputed when a sink joins a live pipeline.
fn watch_bus(pipeline: &gst::Pipeline, serial: u64) -> Option<gst::bus::BusWatchGuard> {
    let weak = pipeline.downgrade();
    pipeline
        .bus()?
        .add_watch_local(move |_, message| {
            match message.view() {
                gst::MessageView::Error(error) => glib::g_warning!(
                    LOG,
                    "call {serial}: {} failed: {}",
                    element_kind(message),
                    error.error().message()
                ),
                gst::MessageView::Warning(warning) => glib::g_debug!(
                    LOG,
                    "call {serial}: {} warned: {}",
                    element_kind(message),
                    warning.error().message()
                ),
                gst::MessageView::Latency(_) => {
                    if let Some(pipeline) = weak.upgrade() {
                        let _ = pipeline.recalculate_latency();
                    }
                }
                _ => {}
            }
            glib::ControlFlow::Continue
        })
        .ok()
}

/// The factory of a message's source element, for a log line.
fn element_kind(message: &gst::Message) -> String {
    message
        .src()
        .and_then(|source| source.downcast_ref::<gst::Element>())
        .and_then(|element| element.factory())
        .map(|factory| factory.name().to_string())
        .unwrap_or_else(|| "an element".into())
}

/// How many buffers a probe saw.
fn buffers(info: &gst::PadProbeInfo) -> u64 {
    info.buffer_list()
        .map_or(1, |list| u64::try_from(list.len()).unwrap_or(u64::MAX))
}

/// Audio or video, from an RTP pad's caps.
fn rtp_kind(pad: &gst::Pad) -> Option<Kind> {
    let caps = pad.current_caps()?;
    match caps.structure(0)?.get::<&str>("media").ok()? {
        "audio" => Some(Kind::Audio),
        "video" => Some(Kind::Video),
        _ => None,
    }
}

/// Audio or video, from a `decodebin3` pad (named `audio_%u` / `video_%u`), else its caps.
fn decoded_kind(pad: &gst::Pad) -> Option<Kind> {
    let name = pad.name();
    if name.starts_with("audio") {
        return Some(Kind::Audio);
    }
    if name.starts_with("video") {
        return Some(Kind::Video);
    }
    let caps = pad.current_caps().unwrap_or_else(|| pad.query_caps(None));
    let structure = caps.structure(0)?;
    if structure.name().starts_with("audio/") {
        Some(Kind::Audio)
    } else if structure.name().starts_with("video/") {
        Some(Kind::Video)
    } else {
        None
    }
}

/// RTP this device sends, counted on a tail's `src`.
fn count_out(tail: &gst::Element, counters: &Arc<Counters>, kind: Kind) {
    let Some(pad) = tail.static_pad("src") else {
        return;
    };
    let counters = counters.clone();
    pad.add_probe(
        gst::PadProbeType::BUFFER | gst::PadProbeType::BUFFER_LIST,
        move |_, info| {
            counters
                .rtp_out(kind)
                .fetch_add(buffers(info), Ordering::Relaxed);
            gst::PadProbeReturn::Ok
        },
    );
}

/// Decoded far-side buffers, counted — and the sound's loudest sample kept.
fn count_decoded(pad: &gst::Pad, counters: &Arc<Counters>, kind: Kind) {
    let counters = counters.clone();
    pad.add_probe(gst::PadProbeType::BUFFER, move |pad, info| {
        match kind {
            Kind::Audio => {
                counters.audio_decoded.fetch_add(1, Ordering::Relaxed);
                if let Some(peak) = info.buffer().and_then(|buffer| peak(pad, buffer)) {
                    counters.audio_peak.fetch_max(peak, Ordering::Relaxed);
                }
            }
            Kind::Video => {
                counters.video_decoded.fetch_add(1, Ordering::Relaxed);
            }
        }
        gst::PadProbeReturn::Ok
    });
}

/// The loudest sample of a raw audio buffer, 0–32767 (S16LE or F32LE; `None` for others).
fn peak(pad: &gst::Pad, buffer: &gst::BufferRef) -> Option<u32> {
    let caps = pad.current_caps()?;
    let format = caps.structure(0)?.get::<&str>("format").ok()?;
    let map = buffer.map_readable().ok()?;
    match format {
        "S16LE" => map
            .chunks_exact(2)
            .map(|sample| u32::from(i16::from_le_bytes([sample[0], sample[1]]).unsigned_abs()))
            .max(),
        "F32LE" => map
            .chunks_exact(4)
            .map(|sample| {
                let value = f32::from_le_bytes([sample[0], sample[1], sample[2], sample[3]]);
                (value.abs().min(1.0) * 32767.0) as u32
            })
            .max(),
        _ => None,
    }
}

impl std::fmt::Debug for GstCallMedia {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GstCallMedia")
            .field("devices", &self.inner.devices)
            .field(
                "call",
                &self.inner.call.borrow().as_ref().map(|call| call.serial),
            )
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_devices_are_pipewire_and_gtk() {
        assert_eq!(
            CallDevices::system(),
            CallDevices {
                microphone: "pulsesrc".into(),
                speaker: "pulsesink".into(),
                ringer: "pulsesink".into(),
                video_sink: "gtk4paintablesink".into(),
                camera: None,
            }
        );
    }

    #[test]
    fn the_self_test_switches_replace_the_devices_they_name() {
        let devices = CallDevices::from_lookup(|name| match name {
            "FC_TEST_AUDIO_SOURCE" => Some("audiotestsrc is-live=true".into()),
            "FC_TEST_AUDIO_SINK" => Some("fakesink sync=true".into()),
            "FC_TEST_VIDEO_SOURCE" => Some("videotestsrc is-live=true".into()),
            _ => None,
        });
        assert_eq!(devices.microphone, "audiotestsrc is-live=true");
        assert_eq!(devices.speaker, "fakesink sync=true");
        assert_eq!(devices.ringer, "fakesink sync=true");
        assert_eq!(devices.video_sink, "gtk4paintablesink");
        assert_eq!(devices.camera.as_deref(), Some("videotestsrc is-live=true"));
    }

    #[test]
    fn an_empty_switch_is_no_switch() {
        let devices = CallDevices::from_lookup(|_| Some("  ".into()));
        assert_eq!(devices, CallDevices::system());
    }
}
