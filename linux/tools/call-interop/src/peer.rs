//! A Linux peer: the real `CallEngine` over the app's `GstCallMedia`, signalling through the
//! relay, driven by a scenario (place or answer, mute, camera off, hang up), sampling the media
//! every second and printing a report with its own checks.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use fc_core::protocol::{ApiResult, FrameSender, IceServerDto, IceServersResponse, ServerFrame};
use fc_core::rt::{Clock, Event, LocalBoxFuture};
use fc_logic::calls::{CallEngine, CallStage, IceServerFetch, Spawn};
use futures_util::StreamExt;
use gst::prelude::*;
use serde_json::{Value, json};
use time::OffsetDateTime;

use crate::calls::{
    AgentFacts, CallDevices, CallStats, GstCallMedia, NoCamera, SilenceRule, VideoCodec,
};
use crate::link;
use crate::recording::RecordingMedia;
use crate::sdp_summary::video_codec;

/// A window of seconds after the call came up.
pub type Window = (f64, f64);

#[derive(Debug, Clone, Default)]
pub struct Options {
    pub label: String,
    pub relay: String,
    pub user: i64,
    pub peer: i64,
    pub chat: i64,
    /// `Some(video)` when this side places the call.
    pub place: Option<bool>,
    pub place_after: f64,
    pub answer_after: f64,
    /// Hang up after this long talking; `None` waits for the far side.
    pub talk: Option<f64>,
    pub mute: Option<Window>,
    pub camera_off: Option<Window>,
    pub peer_mute: Option<Window>,
    pub peer_camera_off: Option<Window>,
    pub timeout: f64,
    pub stun: Option<String>,
    pub turn: Option<String>,
    pub turn_user: Option<String>,
    pub turn_pass: Option<String>,
    pub no_camera: bool,
    pub gtk: bool,
    /// `SilenceRule::Always` instead of the default fallback.
    pub silence_always: bool,
    /// Stop this side's outgoing audio (a blocked pad after the mute) this long after the call
    /// came up — ICE stays alive, so only the silence rule can notice.
    pub stall_audio_at: Option<f64>,
    /// Offer H.264 alone (`GstCallMedia::with_offered_video`).
    pub offer_h264: bool,
    /// Decline the call instead of answering it.
    pub decline: bool,
    /// Give up this long after placing, while it rings.
    pub cancel_after: Option<f64>,
    /// The tone this side must have sounded ("Incoming", "Ringback").
    pub expect_ring: Option<String>,
    /// Both pictures must have had frames (the far side's and the self-view).
    pub expect_paintables: bool,
    /// The direction of this side's own SDP's video m-line.
    pub expect_local_video: Option<String>,
    /// The call must NOT come up (a cancel, a decline, peers that cannot reach each other).
    pub expect_no_connect: bool,
    pub expect_audio: Option<u64>,
    pub expect_video: Option<u64>,
    pub expect_no_video: bool,
    pub expect_codec: Option<String>,
    pub expect_end: Option<String>,
}

fn window(text: &str) -> Result<Window, String> {
    let (from, to) = text
        .split_once('-')
        .ok_or_else(|| format!("a window is FROM-TO, not {text}"))?;
    Ok((
        from.parse().map_err(|_| format!("bad window {text}"))?,
        to.parse().map_err(|_| format!("bad window {text}"))?,
    ))
}

impl Options {
    pub fn parse(args: &[String]) -> Result<Options, String> {
        let mut options = Options {
            relay: "ws://relay:9000".into(),
            user: 1,
            peer: 2,
            chat: 7,
            place_after: 1.0,
            answer_after: 1.0,
            timeout: 60.0,
            ..Options::default()
        };
        let mut args = args.iter();
        while let Some(flag) = args.next() {
            let mut value = || {
                args.next()
                    .cloned()
                    .ok_or_else(|| format!("{flag} needs a value"))
            };
            let number = |text: String| -> Result<f64, String> {
                text.parse().map_err(|_| format!("{flag}: not a number"))
            };
            match flag.as_str() {
                "--label" => options.label = value()?,
                "--relay" => options.relay = value()?,
                "--user" => options.user = number(value()?)? as i64,
                "--peer" => options.peer = number(value()?)? as i64,
                "--chat" => options.chat = number(value()?)? as i64,
                "--place" => options.place = Some(value()? == "video"),
                "--place-after" => options.place_after = number(value()?)?,
                "--answer-after" => options.answer_after = number(value()?)?,
                "--talk" => options.talk = Some(number(value()?)?),
                "--mute" => options.mute = Some(window(&value()?)?),
                "--camera-off" => options.camera_off = Some(window(&value()?)?),
                "--peer-mute" => options.peer_mute = Some(window(&value()?)?),
                "--peer-camera-off" => options.peer_camera_off = Some(window(&value()?)?),
                "--timeout" => options.timeout = number(value()?)?,
                "--stun" => options.stun = Some(value()?),
                "--turn" => options.turn = Some(value()?),
                "--turn-user" => options.turn_user = Some(value()?),
                "--turn-pass" => options.turn_pass = Some(value()?),
                "--no-camera" => options.no_camera = true,
                "--gtk" => options.gtk = true,
                "--silence-rule" => options.silence_always = value()? == "always",
                "--stall-audio-at" => options.stall_audio_at = Some(number(value()?)?),
                "--offer-video" => options.offer_h264 = value()?.eq_ignore_ascii_case("h264"),
                "--decline" => options.decline = true,
                "--cancel-after" => options.cancel_after = Some(number(value()?)?),
                "--expect-ring" => options.expect_ring = Some(value()?),
                "--expect-paintables" => options.expect_paintables = true,
                "--expect-local-video" => options.expect_local_video = Some(value()?),
                "--expect-no-connect" => options.expect_no_connect = true,
                "--expect-audio" => options.expect_audio = Some(number(value()?)? as u64),
                "--expect-video" => options.expect_video = Some(number(value()?)? as u64),
                "--expect-no-video" => options.expect_no_video = true,
                "--expect-codec" => options.expect_codec = Some(value()?.to_ascii_uppercase()),
                "--expect-end" => options.expect_end = Some(value()?),
                other => return Err(format!("unknown flag {other}")),
            }
        }
        Ok(options)
    }

    /// What `GET /calls/ice` would have answered.
    pub fn ice_servers(&self) -> Vec<IceServerDto> {
        let mut servers = Vec::new();
        if let Some(stun) = &self.stun {
            servers.push(IceServerDto {
                urls: Some(vec![stun.clone()]),
                username: None,
                credential: None,
            });
        }
        if let Some(turn) = &self.turn {
            servers.push(IceServerDto {
                urls: Some(vec![turn.clone()]),
                username: self.turn_user.clone(),
                credential: self.turn_pass.clone(),
            });
        }
        servers
    }
}

/// The socket half the engine writes to.
struct Wire {
    out: std::sync::mpsc::Sender<String>,
    frames: Event<ServerFrame>,
    said: RefCell<Vec<(f64, String)>>,
    start: Instant,
}

impl FrameSender for Wire {
    fn is_connected(&self) -> bool {
        true
    }

    fn try_send(&self, frame: String) -> LocalBoxFuture<'_, bool> {
        let kind = serde_json::from_str::<Value>(&frame)
            .ok()
            .and_then(|value| value["type"].as_str().map(str::to_string))
            .unwrap_or_default();
        self.said
            .borrow_mut()
            .push((self.start.elapsed().as_secs_f64(), kind));
        let sent = self.out.send(frame).is_ok();
        Box::pin(async move { sent })
    }

    fn on_frame(&self) -> &Event<ServerFrame> {
        &self.frames
    }
}

struct GlibClock;

impl Clock for GlibClock {
    fn now(&self) -> OffsetDateTime {
        OffsetDateTime::now_utc()
    }

    fn sleep(&self, duration: Duration) -> LocalBoxFuture<'static, ()> {
        glib::timeout_future(duration)
    }
}

#[derive(Debug, Clone)]
struct Sample {
    t: f64,
    talk: Option<f64>,
    stage: String,
    ring: String,
    ice: String,
    pc: String,
    stats: CallStats,
}

struct Run {
    options: Options,
    start: Instant,
    main: glib::MainLoop,
    engine: RefCell<Weak<CallEngine>>,
    media: Rc<GstCallMedia>,
    recording: Rc<RecordingMedia>,
    wire: Rc<Wire>,
    stages: RefCell<Vec<(f64, String)>>,
    last_stage: Cell<Option<CallStage>>,
    talking_at: Cell<Option<f64>>,
    ended: RefCell<Option<(f64, String)>>,
    answering: Cell<bool>,
    samples: RefCell<Vec<Sample>>,
    pipeline: RefCell<Option<glib::WeakRef<gst::Pipeline>>>,
    agent: Cell<Option<AgentFacts>>,
    /// The two pictures 3 s into the call: whether there was one, and its intrinsic size (0 × 0
    /// until a frame reached it).
    paintables: Cell<Option<(Option<(i32, i32)>, Option<(i32, i32)>)>>,
    finished: Cell<bool>,
    exit: Cell<i32>,
}

fn stage_name(stage: CallStage) -> &'static str {
    match stage {
        CallStage::Dialling => "Dialling",
        CallStage::Ringing => "Ringing",
        CallStage::Incoming => "Incoming",
        CallStage::Connecting => "Connecting",
        CallStage::Talking => "Talking",
        CallStage::Ended => "Ended",
    }
}

impl Run {
    fn now(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }

    fn engine(&self) -> Option<Rc<CallEngine>> {
        self.engine.borrow().upgrade()
    }

    /// After `seconds`, on the main context, outside whatever is running now.
    fn later(self: &Rc<Self>, seconds: f64, then: impl FnOnce(&Rc<Run>) + 'static) {
        let run = self.clone();
        glib::timeout_add_local_once(Duration::from_secs_f64(seconds.max(0.0)), move || {
            then(&run);
        });
    }

    fn saw(self: &Rc<Self>) {
        let Some(call) = self.engine().and_then(|engine| engine.call()) else {
            return;
        };
        if self.last_stage.get() == Some(call.stage) {
            return;
        }
        self.last_stage.set(Some(call.stage));
        let now = self.now();
        println!(
            "[{}] {now:6.2} stage {}{}",
            self.options.label,
            stage_name(call.stage),
            call.ended_reason
                .as_deref()
                .map(|reason| format!(" ({reason})"))
                .unwrap_or_default()
        );
        self.stages
            .borrow_mut()
            .push((now, stage_name(call.stage).to_string()));
        match call.stage {
            CallStage::Incoming if !self.answering.replace(true) => {
                self.later(self.options.answer_after, |run| {
                    if let Some(engine) = run.engine() {
                        if run.options.decline {
                            println!("[{}] {:6.2} declining", run.options.label, run.now());
                            engine.decline();
                        } else {
                            glib::MainContext::default().spawn_local(engine.answer());
                        }
                    }
                });
            }
            CallStage::Talking if self.talking_at.get().is_none() => {
                self.talking_at.set(Some(now));
                self.schedule_talk();
            }
            CallStage::Ended if self.ended.borrow().is_none() => {
                *self.ended.borrow_mut() = Some((now, call.ended_reason.unwrap_or_default()));
                self.later(3.0, |run| run.finish(false));
            }
            _ => {}
        }
    }

    fn schedule_talk(self: &Rc<Self>) {
        let toggle_mute = |run: &Rc<Run>| {
            if let Some(engine) = run.engine() {
                engine.toggle_mute();
            }
        };
        let toggle_camera = |run: &Rc<Run>| {
            if let Some(engine) = run.engine() {
                engine.toggle_camera();
            }
        };
        if let Some((from, to)) = self.options.mute {
            self.later(from, toggle_mute);
            self.later(to, toggle_mute);
        }
        if let Some((from, to)) = self.options.camera_off {
            self.later(from, toggle_camera);
            self.later(to, toggle_camera);
        }
        if let Some(talk) = self.options.talk {
            self.later(talk, |run| {
                if let Some(engine) = run.engine() {
                    println!("[{}] {:6.2} hanging up", run.options.label, run.now());
                    engine.end();
                }
            });
        }
        if let Some(at) = self.options.stall_audio_at {
            self.later(at, |run| {
                let stalled = run
                    .media
                    .pipeline()
                    .and_then(|pipeline| pipeline.by_name("mute"))
                    .and_then(|mute| mute.static_pad("src"))
                    .and_then(|pad| {
                        pad.add_probe(gst::PadProbeType::BLOCK_DOWNSTREAM, |_, _| {
                            gst::PadProbeReturn::Ok
                        })
                    });
                println!(
                    "[{}] {:6.2} outgoing audio stalled: {}",
                    run.options.label,
                    run.now(),
                    stalled.is_some()
                );
            });
        }
        self.later(2.0, |run| run.agent.set(run.media.agent_facts()));
        self.later(3.0, |run| {
            use gtk::prelude::PaintableExt;
            let size = |paintable: gtk::gdk::Paintable| {
                (paintable.intrinsic_width(), paintable.intrinsic_height())
            };
            run.paintables.set(Some((
                run.media.remote_video().map(size),
                run.media.self_view().map(size),
            )));
        });
    }

    fn sample(&self) {
        let (ice, pc) = self
            .media
            .pipeline()
            .and_then(|pipeline| pipeline.by_name("call"))
            .map(|webrtc| {
                (
                    format!(
                        "{:?}",
                        webrtc.property::<gst_webrtc::WebRTCICEConnectionState>(
                            "ice-connection-state"
                        )
                    ),
                    format!(
                        "{:?}",
                        webrtc
                            .property::<gst_webrtc::WebRTCPeerConnectionState>("connection-state")
                    ),
                )
            })
            .unwrap_or_else(|| ("-".into(), "-".into()));
        if self.pipeline.borrow().is_none() {
            *self.pipeline.borrow_mut() =
                self.media.pipeline().map(|pipeline| pipeline.downgrade());
        }
        let t = self.now();
        let stage = self
            .last_stage
            .get()
            .map(stage_name)
            .unwrap_or("-")
            .to_string();
        let talk = self.talking_at.get().map(|at| t - at);
        let stats = self.media.stats();
        let ring = self
            .media
            .ringing()
            .map_or("-".to_string(), |tone| format!("{tone:?}"));
        self.samples.borrow_mut().push(Sample {
            t,
            talk,
            stage,
            ring,
            ice,
            pc,
            stats,
        });
    }

    // ---- the report ----------------------------------------------------------------------

    /// The samples whose time-since-talking lies in [from, to].
    fn window(&self, from: f64, to: f64) -> Vec<Sample> {
        self.samples
            .borrow()
            .iter()
            .filter(|sample| sample.talk.is_some_and(|talk| talk >= from && talk <= to))
            .cloned()
            .collect()
    }

    fn delta(samples: &[Sample], field: fn(&CallStats) -> u64) -> Option<u64> {
        let first = samples.first()?;
        let last = samples.last()?;
        Some(field(&last.stats).saturating_sub(field(&first.stats)))
    }

    fn peak(samples: &[Sample]) -> u32 {
        samples
            .iter()
            .map(|sample| sample.stats.audio_peak)
            .max()
            .unwrap_or(0)
    }

    fn checks(&self) -> Vec<(String, bool, String)> {
        let mut checks = Vec::new();
        let samples = self.samples.borrow().clone();
        let most = |field: fn(&CallStats) -> u64| {
            samples
                .iter()
                .map(|sample| field(&sample.stats))
                .max()
                .unwrap_or(0)
        };
        let notes = self.recording.notes();
        let ended = self.ended.borrow().clone();
        if self.options.expect_no_connect {
            checks.push((
                "never connected".into(),
                self.talking_at.get().is_none(),
                format!("talking at {:?}", self.talking_at.get()),
            ));
        } else {
            checks.push((
                "connected".into(),
                self.talking_at.get().is_some(),
                format!("talking at {:?}", self.talking_at.get()),
            ));
        }
        if let Some(least) = self.options.expect_audio {
            let decoded = most(|stats| stats.audio_decoded);
            checks.push((
                "far audio decoded".into(),
                decoded >= least,
                format!(
                    "{decoded} buffers (rtp {})",
                    most(|stats| stats.audio_rtp_in)
                ),
            ));
        }
        if let Some(least) = self.options.expect_video {
            let decoded = most(|stats| stats.video_decoded);
            checks.push((
                "far video decoded".into(),
                decoded >= least,
                format!(
                    "{decoded} frames (rtp {})",
                    most(|stats| stats.video_rtp_in)
                ),
            ));
        }
        if self.options.expect_no_video {
            let rtp = most(|stats| stats.video_rtp_in);
            checks.push(("no far video".into(), rtp == 0, format!("{rtp} rtp")));
        }
        if let Some(codec) = &self.options.expect_codec {
            let answer = if self.options.place.is_some() {
                notes.remote_sdp.clone()
            } else {
                notes.local_sdp.clone()
            };
            let got = answer.as_deref().and_then(video_codec);
            checks.push((
                "negotiated video codec".into(),
                got.as_deref() == Some(codec.as_str()),
                format!("{got:?} in {answer:?}"),
            ));
        }
        if let Some(tone) = &self.options.expect_ring {
            let heard: Vec<String> = samples.iter().map(|sample| sample.ring.clone()).collect();
            checks.push((
                "rang".into(),
                heard.iter().any(|ring| ring == tone),
                format!("tones seen each second: {heard:?}"),
            ));
        }
        if self.options.expect_paintables {
            let pictures = self.paintables.get();
            let drawn = |size: Option<(i32, i32)>| size.is_some_and(|(w, h)| w > 0 && h > 0);
            checks.push((
                "pictures drawn (far side, self-view)".into(),
                pictures.is_some_and(|(remote, own)| drawn(remote) && drawn(own)),
                format!("{pictures:?}"),
            ));
        }
        if let Some(direction) = &self.options.expect_local_video {
            let video = notes
                .local_sdp
                .as_deref()
                .and_then(|sdp| {
                    sdp.split(" | ")
                        .find(|section| section.starts_with("video "))
                })
                .map(str::to_string);
            checks.push((
                "own video m-line".into(),
                video
                    .as_deref()
                    .and_then(|section| section.split(' ').nth(1))
                    .is_some_and(|state| state.starts_with(direction.as_str())),
                format!("{video:?}"),
            ));
        }
        if let Some(reason) = &self.options.expect_end {
            let got = ended.as_ref().map(|(_, reason)| reason.clone());
            checks.push((
                "ended with".into(),
                got.as_deref() == Some(reason.as_str()),
                format!("{got:?}"),
            ));
        }
        if ended.is_some() {
            checks.push((
                "no event after close".into(),
                notes.events_after_close == 0,
                format!("{} event(s)", notes.events_after_close),
            ));
            let pipeline = self
                .pipeline
                .borrow()
                .as_ref()
                .map(|weak| weak.upgrade().map(|pipeline| pipeline.current_state()));
            // `None`: this side never built one (a callee that only rang).
            checks.push((
                "pipeline freed".into(),
                matches!(pipeline, None | Some(None)),
                match pipeline {
                    None => "none was built".to_string(),
                    Some(None) => "freed".to_string(),
                    Some(Some(state)) => format!("still alive in {state:?}"),
                },
            ));
            checks.push((
                "no ring left".into(),
                self.media.ringing().is_none(),
                format!("{:?}", self.media.ringing()),
            ));
        }
        if let Some((from, to)) = self.options.peer_mute {
            let before = self.window(1.0, from - 0.5);
            let during = self.window(from + 1.5, to - 0.2);
            let after = self.window(to + 1.5, to + 4.0);
            let rtp = Self::delta(&during, |stats| stats.audio_rtp_in).unwrap_or(0);
            let (loud, quiet, again) =
                (Self::peak(&before), Self::peak(&during), Self::peak(&after));
            checks.push((
                "far mute: rtp keeps flowing, sound stops".into(),
                rtp > 0 && quiet * 10 < loud.max(1) && again * 10 > loud,
                format!("rtp +{rtp} while muted; peak before {loud}, muted {quiet}, after {again}"),
            ));
        }
        if let Some((from, to)) = self.options.peer_camera_off {
            let before = self.window(1.0, from - 0.2);
            let during = self.window(from + 1.5, to - 0.2);
            let after = self.window(to + 1.5, to + 4.0);
            let field = |stats: &CallStats| stats.video_rtp_in;
            let (b, d, a) = (
                Self::delta(&before, field).unwrap_or(0),
                Self::delta(&during, field).unwrap_or(0),
                Self::delta(&after, field).unwrap_or(0),
            );
            checks.push((
                "far camera off: video stops, then resumes".into(),
                b > 0 && d == 0 && a > 0,
                format!("video rtp +{b} before, +{d} while off, +{a} after"),
            ));
        }
        if let Some((from, to)) = self.options.mute {
            let during = self.window(from + 0.5, to - 0.2);
            let sent = Self::delta(&during, |stats| stats.audio_rtp_out).unwrap_or(0);
            checks.push((
                "own mute: audio rtp still sent".into(),
                sent > 0,
                format!("audio rtp out +{sent} while muted"),
            ));
        }
        if let Some((from, to)) = self.options.camera_off {
            let during = self.window(from + 0.5, to - 0.2);
            let after = self.window(to + 1.0, to + 3.0);
            let field = |stats: &CallStats| stats.video_rtp_out;
            let (d, a) = (
                Self::delta(&during, field).unwrap_or(0),
                Self::delta(&after, field).unwrap_or(0),
            );
            checks.push((
                "own camera off: no video sent, then again".into(),
                d == 0 && a > 0,
                format!("video rtp out +{d} while off, +{a} after"),
            ));
        }
        checks
    }

    /// When the far side's audio last moved, and how long until the call ended — the dead-peer
    /// measurement.
    fn silence_to_end(&self) -> Option<(f64, f64)> {
        let samples = self.samples.borrow();
        let ended = self.ended.borrow().as_ref()?.0;
        let last_heard = samples
            .iter()
            .filter(|sample| sample.t <= ended)
            .cloned()
            .collect::<Vec<_>>()
            .windows(2)
            .filter(|pair| pair[1].stats.audio_rtp_in > pair[0].stats.audio_rtp_in)
            .map(|pair| pair[1].t)
            .next_back()?;
        Some((last_heard, ended - last_heard))
    }

    fn finish(self: &Rc<Self>, timed_out: bool) {
        if self.finished.replace(true) {
            return;
        }
        self.sample();
        let checks = self.checks();
        let failed = timed_out || checks.iter().any(|(_, passed, _)| !passed);
        let notes = self.recording.notes();
        let samples: Vec<Value> = self
            .samples
            .borrow()
            .iter()
            .map(|sample| {
                json!([
                    (sample.t * 10.0).round() / 10.0,
                    sample.talk.map(|talk| (talk * 10.0).round() / 10.0),
                    sample.stage,
                    sample.ring,
                    sample.ice,
                    sample.pc,
                    sample.stats.audio_rtp_in,
                    sample.stats.video_rtp_in,
                    sample.stats.audio_decoded,
                    sample.stats.video_decoded,
                    sample.stats.audio_rtp_out,
                    sample.stats.video_rtp_out,
                    sample.stats.audio_peak,
                ])
            })
            .collect();
        let report = json!({
            "label": self.options.label,
            "timed_out": timed_out,
            "stages": *self.stages.borrow(),
            "ended": *self.ended.borrow(),
            "media_calls": notes.calls,
            "media_events": notes.events.iter()
                .filter(|(_, what)| what != "candidate").collect::<Vec<_>>(),
            "candidates_gathered": notes.events.iter().filter(|(_, what)| what == "candidate").count(),
            "candidates_added": self.recording.candidates_added(),
            "events_after_close": notes.events_after_close,
            "grant": notes.grant.map(|grant| format!("{grant:?}")),
            "local_sdp": notes.local_sdp,
            "remote_sdp": notes.remote_sdp,
            "said": *self.wire.said.borrow(),
            "agent": self.agent.get().map(|facts| json!({
                "consent_freshness": facts.consent_freshness,
                "keepalive_conncheck": facts.keepalive_conncheck,
                "ice_trickle": facts.ice_trickle,
            })),
            "silence_to_end": self.silence_to_end(),
            "paintables": self.paintables.get().map(|(remote, own)| json!({
                "remote_video": remote, "self_view": own,
            })),
            "sample_columns": ["t", "talk", "stage", "ring", "ice", "pc", "audio_rtp_in", "video_rtp_in",
                               "audio_decoded", "video_decoded", "audio_rtp_out", "video_rtp_out",
                               "audio_peak"],
            "samples": samples,
            "checks": checks.iter().map(|(name, passed, detail)| json!({
                "check": name, "passed": passed, "detail": detail,
            })).collect::<Vec<_>>(),
        });
        drop(notes);
        println!("REPORT {report}");
        for (name, passed, detail) in &checks {
            println!(
                "[{}] CHECK {}: {name} — {detail}",
                self.options.label,
                if *passed { "PASS" } else { "FAIL" }
            );
        }
        if timed_out {
            println!("[{}] CHECK FAIL: timed out", self.options.label);
        }
        println!(
            "[{}] RESULT {}",
            self.options.label,
            if failed { "FAIL" } else { "PASS" }
        );
        self.exit.set(if failed { 1 } else { 0 });
        self.main.quit();
    }
}

pub fn run(options: Options) -> i32 {
    if options.gtk {
        if let Err(error) = gtk::init() {
            eprintln!("peer: GTK did not initialise: {error}");
            return 2;
        }
    } else if let Err(error) = gst::init() {
        eprintln!("peer: GStreamer did not initialise: {error}");
        return 2;
    }
    let start = Instant::now();
    let main = glib::MainLoop::new(None, false);
    let url = format!("{}/?user={}", options.relay, options.user);
    let (out, mut frames) = link::open(&url);
    let wire = Rc::new(Wire {
        out,
        frames: Event::new(),
        said: RefCell::new(Vec::new()),
        start,
    });
    let mut devices = CallDevices::from_env();
    if options.no_camera {
        devices.camera = None;
    }
    let rule = if options.silence_always {
        SilenceRule::Always
    } else {
        SilenceRule::Fallback
    };
    let offered = if options.offer_h264 {
        VideoCodec::H264
    } else {
        VideoCodec::Vp8
    };
    let media = Rc::new(
        GstCallMedia::new(devices, Rc::new(NoCamera))
            .with_silence_rule(rule)
            .with_offered_video(offered),
    );
    let recording = Rc::new(RecordingMedia::new(media.clone(), start));
    let servers = options.ice_servers();
    let fetch: IceServerFetch = Rc::new(move || {
        let servers = servers.clone();
        Box::pin(async move {
            ApiResult::success(
                IceServersResponse {
                    ice_servers: Some(servers),
                    ttl_secs: Some(86_400),
                },
                200,
            )
        })
    });
    let spawn: Spawn = Rc::new(|future| {
        glib::MainContext::default().spawn_local(future);
    });
    let engine = Rc::new(CallEngine::new(
        wire.clone(),
        fetch,
        recording.clone(),
        Rc::new(GlibClock),
        spawn,
    ));
    let run = Rc::new(Run {
        options: options.clone(),
        start,
        main: main.clone(),
        engine: RefCell::new(Rc::downgrade(&engine)),
        media,
        recording,
        wire,
        stages: RefCell::new(Vec::new()),
        last_stage: Cell::new(None),
        talking_at: Cell::new(None),
        ended: RefCell::new(None),
        answering: Cell::new(false),
        samples: RefCell::new(Vec::new()),
        pipeline: RefCell::new(None),
        agent: Cell::new(None),
        paintables: Cell::new(None),
        finished: Cell::new(false),
        exit: Cell::new(1),
    });
    {
        let engine = engine.clone();
        glib::MainContext::default().spawn_local(async move {
            while let Some(text) = frames.next().await {
                if let Some(frame) = ServerFrame::parse(&text) {
                    engine.hear(&frame);
                }
            }
        });
    }
    {
        let run = run.clone();
        engine.on_changed().subscribe(move |_| run.saw());
    }
    {
        let run = run.clone();
        glib::timeout_add_local(Duration::from_secs(1), move || {
            if run.finished.get() {
                return glib::ControlFlow::Break;
            }
            run.sample();
            glib::ControlFlow::Continue
        });
    }
    if let Some(video) = options.place {
        let (chat, peer) = (options.chat, options.peer);
        run.later(options.place_after, move |run| {
            if let Some(engine) = run.engine() {
                println!("[{}] {:6.2} placing", run.options.label, run.now());
                glib::MainContext::default().spawn_local(engine.place(chat, peer, video));
            }
        });
        if let Some(after) = options.cancel_after {
            run.later(options.place_after + after, |run| {
                if let Some(engine) = run.engine() {
                    println!("[{}] {:6.2} giving up", run.options.label, run.now());
                    engine.end();
                }
            });
        }
    }
    run.later(options.timeout, |run| run.finish(true));
    main.run();
    let exit = run.exit.get();
    drop(engine);
    exit
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn the_options_read_every_flag() {
        let options = Options::parse(&args(
            "--label a1 --relay ws://r:1 --user 2 --peer 1 --place video --talk 12 \
             --mute 4-7 --camera-off 5-8 --peer-mute 4-7 --peer-camera-off 5-8 \
             --turn turn:t:3478 --turn-user 1700000000:42 --turn-pass x+y= --no-camera \
             --expect-audio 50 --expect-video 30 --expect-codec h264 --expect-end hangup",
        ))
        .unwrap();
        assert_eq!(options.label, "a1");
        assert_eq!(options.relay, "ws://r:1");
        assert_eq!((options.user, options.peer), (2, 1));
        assert_eq!(options.place, Some(true));
        assert_eq!(options.talk, Some(12.0));
        assert_eq!(options.mute, Some((4.0, 7.0)));
        assert_eq!(options.peer_camera_off, Some((5.0, 8.0)));
        assert!(options.no_camera);
        assert_eq!(options.expect_codec.as_deref(), Some("H264"));
        assert_eq!(
            options.ice_servers(),
            vec![IceServerDto {
                urls: Some(vec!["turn:t:3478".into()]),
                username: Some("1700000000:42".into()),
                credential: Some("x+y=".into()),
            }]
        );
    }

    #[test]
    fn a_bad_flag_is_refused() {
        assert!(Options::parse(&args("--nope")).is_err());
        assert!(Options::parse(&args("--mute 4")).is_err());
        assert!(Options::parse(&args("--talk")).is_err());
    }
}
