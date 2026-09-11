//! A call in a tab: the peer connection, the ringing, and the signalling
//! that carries it (docs/protocol.md, "Voice calls").
//!
//! The media is peer to peer and the server never touches it. What the
//! server passes are four small frames — `call_offer`, `call_answer`,
//! `call_ice`, `call_end` — and those are the ONE thing a browser sends on
//! the socket rather than over REST: calls have no REST surface, and a call
//! whose offer did not go up is a call that visibly did not happen (the
//! protocol says so under "A browser is a client too").
//!
//! Three rules here are the protocol's rather than this client's:
//! - a frame is applied only to the call this tab HOLDS, and every other one
//!   is ignored in silence — that is what makes one person's several devices
//!   work without the server tracking them;
//! - remote candidates that arrive before the remote description is set are
//!   BUFFERED, because a replayed offer is followed by the candidates
//!   gathered while a phone was waking and nothing promises the order;
//! - the socket stays open for the life of a call. A browser could not
//!   suspend it anyway, which is why a tab can be on a call at all.
//!
//! And one is the browser's: a tab that closes takes the call with it, so it
//! says `call_end` on the way out through `Wire` — the socket itself, not the
//! frame channel, because nothing polls a channel after the page is gone.

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::{spawn_local, JsFuture};
use web_sys::{
    MediaStream, MediaStreamTrack, RtcConfiguration, RtcIceCandidate, RtcIceCandidateInit,
    RtcPeerConnection, RtcPeerConnectionIceEvent, RtcPeerConnectionState, RtcSdpType,
    RtcSessionDescriptionInit, RtcTrackEvent,
};

use crate::live::Live;
use crate::model::{IceCandidate, IceServer};
use crate::socket::ClientFrame;

/// The apps' guards, mirrored (ios CallManager): the server ends an
/// unanswered call at 45 s and tells both sides, so each of these is a
/// backstop for a `call_end` that never arrived — later than the server's
/// clock, so the server's reason is the one normally shown. Neither ring
/// guard sends a frame: the server has ended the call by then.
pub const OUTGOING_RING_GUARD_MS: u32 = 90_000;
pub const INCOMING_RING_GUARD_MS: u32 = 60_000;

/// A call that was answered but never came up: this one DOES say so, as
/// `failed`, because nothing on the server ends it before 60 s of silence.
pub const ANSWER_GUARD_MS: u32 = 30_000;

/// How long the panel stays after a call ends, saying why.
pub const ENDED_LINGER_MS: u32 = 2_000;

/// How many just-ended calls are remembered, so a replayed offer for one
/// cannot ring again (ios `recentlyEnded`).
const REMEMBERED: usize = 16;

/// Where a call has got to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Placed; the server has not said it reached them yet.
    Dialling,
    /// Ringing on their side.
    Ringing,
    /// Somebody is calling: this tab is the one ringing.
    Incoming,
    /// Answered, and the media is coming up.
    Connecting,
    /// The media is up.
    Talking,
    /// Over. The panel says why for a moment, then goes.
    Ended,
}

/// A call as the screen needs it. The peer connection and the streams live
/// in `Calls`, which is a handle to the browser's own objects rather than
/// state; this is what a view can compare and redraw from.
#[derive(Debug, Clone, PartialEq)]
pub struct CallState {
    pub call_id: String,
    pub chat_id: i64,
    /// The person at the other end — the direct chat's other member.
    pub peer_user_id: i64,
    /// This tab placed it.
    pub outgoing: bool,
    /// Decided when it was placed and fixed for its life: cameras toggle,
    /// the kind does not (docs/protocol.md, "Video").
    pub video: bool,
    pub stage: Stage,
    /// Our microphone, off.
    pub muted: bool,
    /// Our camera, on — never true on a voice call.
    pub camera: bool,
    /// Answered — by this tab or, for the caller, by the far side. What
    /// decides whether ending it is a `hangup`.
    pub taken: bool,
    /// When the media came up, by this tab's clock: what the duration on
    /// screen counts from, so setting up is not billed to the conversation
    /// (the record's own duration is the server's). The apps count from
    /// exactly here too.
    pub answered_at: Option<f64>,
    /// Bumped whenever a stream appears or goes, so a view knows to attach
    /// it to its element again.
    pub media: u64,
    /// Why it ended, while the panel still says so: a reason the protocol
    /// names, or one of this client's own for something that never left the
    /// tab (`microphone_denied`).
    pub ended_reason: Option<String>,
}

impl CallState {
    fn new(call_id: String, chat_id: i64, peer_user_id: i64, outgoing: bool, video: bool) -> Self {
        CallState {
            call_id,
            chat_id,
            peer_user_id,
            outgoing,
            video,
            stage: if outgoing {
                Stage::Dialling
            } else {
                Stage::Incoming
            },
            muted: false,
            camera: video,
            taken: false,
            answered_at: None,
            media: 0,
            ended_reason: None,
        }
    }

    /// Whether the call was answered — which is when ending it is a
    /// `hangup` rather than a `cancel` (docs/protocol.md, "The sequence").
    pub fn answered(&self) -> bool {
        self.taken
    }
}

/// The socket itself, for the frames that cannot wait for a task to be
/// polled: the `call_end` a closing tab owes the other side. Shared with
/// the socket loop, which puts the live connection in and takes it out
/// again when it drops.
#[derive(Clone, Default)]
pub struct Wire(Rc<RefCell<Option<web_sys::WebSocket>>>);

impl Wire {
    pub fn new() -> Self {
        Wire::default()
    }

    /// The connection that is up now, or none while it is down.
    pub fn hold(&self, socket: Option<web_sys::WebSocket>) {
        *self.0.borrow_mut() = socket;
    }

    /// Say it now, on this connection. False when there is none, or when
    /// the browser refused it — a call frame is momentary and is never
    /// saved up: what is at stake is a call that plainly did not happen,
    /// and the person who tried it is looking at the screen.
    pub fn say(&self, frame: &ClientFrame) -> bool {
        let held = self.0.borrow();
        let Some(socket) = held.as_ref() else {
            return false;
        };
        if socket.ready_state() != web_sys::WebSocket::OPEN {
            return false;
        }
        match serde_json::to_string(frame) {
            Ok(text) => socket.send_with_str(&text).is_ok(),
            Err(_) => false,
        }
    }
}

/// A refused call frame, as a reason to end on — the apps' own words, and
/// the same short ones the record's wording is (ios CallRecordText).
pub fn refusal_reason(code: &str) -> &'static str {
    match code {
        "call_busy" | "peer_busy" => "busy",
        "peer_unreachable" => "unreachable",
        "video_calls_disabled" => "video_calls_disabled",
        // Not collapsed into one word, as the apps collapse them: a
        // person's own block is theirs to know about and to undo, and a
        // server that carries no calls is not "unavailable" either.
        "blocked" => "blocked",
        "calls_disabled" => "calls_disabled",
        _ => "failed",
    }
}

/// Why the call on screen ended, in the apps' words (ios
/// CallRecordText.status). A call this person ended themselves says the
/// plain thing: they know what they did.
pub fn ended_line(reason: &str, call: &CallState) -> String {
    match reason {
        "decline" if call.outgoing => "Declined".to_string(),
        "timeout" if call.outgoing => "No answer".to_string(),
        "timeout" => fc_text::call_record::label(
            fc_text::call_record::outcome::MISSED,
            None,
            call.video,
            false,
        ),
        "answered_elsewhere" => "Answered on another device".to_string(),
        "busy" => "Busy".to_string(),
        "unreachable" | "unavailable" => "Unavailable".to_string(),
        "blocked" => "You've blocked them.".to_string(),
        "calls_disabled" => "Calls are off on this server.".to_string(),
        "video_calls_disabled" => "Video calls are off on this server.".to_string(),
        "microphone_denied" => NO_MICROPHONE.to_string(),
        "failed" => "Call failed".to_string(),
        _ => "Call ended".to_string(),
    }
}

/// What a tab on its way out owes the other side. A callee's tab going is
/// a refusal, not a cancel: a `cancel` from the callee would be written as
/// a missed call, which says the ring was never answered by anyone.
pub fn unload_reason(call: &CallState) -> &'static str {
    match (call.answered(), call.outgoing) {
        (true, _) => "hangup",
        (false, true) => "cancel",
        (false, false) => "decline",
    }
}

/// No microphone, no call. The browser's own permission, and its own
/// prompt: this is only ever shown, never worked around.
pub const NO_MICROPHONE: &str = "Microphone access is needed for calls.";

#[derive(Default)]
struct Inner {
    pc: Option<RtcPeerConnection>,
    /// What this tab sends: the microphone, and the camera on a video call.
    local: Option<MediaStream>,
    /// What it receives. Built here rather than taken from the track event,
    /// so one element can be attached once and keep playing as tracks
    /// arrive.
    remote: Option<MediaStream>,
    /// Remote candidates held until the remote description is set.
    early: Vec<IceCandidate>,
    remote_set: bool,
    /// The offer this tab is ringing with, until it is answered.
    offer_sdp: Option<String>,
    on_ice: Option<Closure<dyn FnMut(RtcPeerConnectionIceEvent)>>,
    on_track: Option<Closure<dyn FnMut(RtcTrackEvent)>>,
    on_state: Option<Closure<dyn FnMut()>>,
    ring: Option<Ring>,
    guard: Option<gloo_timers::callback::Timeout>,
    /// The panel's own timer: how long an ended call stays on screen
    /// saying why.
    linger: Option<gloo_timers::callback::Timeout>,
    /// The calls that have just ended, newest last. A replayed offer for
    /// one of them is a call that is already over (ios `recentlyEnded`).
    recently_ended: std::collections::VecDeque<String>,
}

/// The peer connection, the media and the ringing — one call's worth.
#[derive(Clone)]
pub struct Calls {
    live: Live,
    wire: Wire,
    inner: Rc<RefCell<Inner>>,
}

impl PartialEq for Calls {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }
}

impl Calls {
    pub fn new(live: Live, wire: Wire) -> Self {
        Calls {
            live,
            wire,
            inner: Rc::new(RefCell::new(Inner::default())),
        }
    }

    pub fn wire(&self) -> Wire {
        self.wire.clone()
    }

    /// The streams a view attaches to its elements.
    pub fn remote_stream(&self) -> Option<MediaStream> {
        self.inner.borrow().remote.clone()
    }

    pub fn local_stream(&self) -> Option<MediaStream> {
        self.inner.borrow().local.clone()
    }

    fn call(&self) -> Option<CallState> {
        self.live.read(|state| state.call.clone())
    }

    /// Whether `call_id` is still the call this tab is ON — not merely the
    /// one it is still saying goodbye to. Every await in setting a call up
    /// is followed by this: a call cancelled while the browser was asking
    /// for the microphone must not go on to be placed, which would leave a
    /// live microphone behind a panel that has gone.
    fn still(&self, call_id: &str) -> bool {
        self.call()
            .is_some_and(|call| call.call_id == call_id && call.stage != Stage::Ended)
    }

    /// Change the held call, and only while it is still the same call: an
    /// answer, a candidate or an end that names another one is not ours to
    /// act on (docs/protocol.md, "Identity: the client names the call").
    fn with<R>(&self, call_id: &str, change: impl FnOnce(&mut CallState) -> R) -> Option<R> {
        self.live.now(|state| {
            state
                .call
                .as_mut()
                .filter(|call| call.call_id == call_id)
                .map(change)
        })
    }
}

/// The ringing tone: a pair of beeps every few seconds, made here rather
/// than fetched, so a call rings without an asset to load. A browser may
/// refuse to sound anything before the tab has been clicked in — the ring
/// on screen is what carries it then.
struct Ring {
    context: web_sys::AudioContext,
    /// Held, not read: dropping it stops the ringing.
    _beat: gloo_timers::callback::Interval,
}

impl Drop for Ring {
    fn drop(&mut self) {
        let _ = self.context.close();
    }
}

impl Ring {
    /// Somebody is calling: a pair of beeps every three seconds. A browser
    /// has no system ringtone to borrow, and a tab that rings silently is a
    /// call nobody in another window will notice.
    fn incoming() -> Option<Ring> {
        Ring::tone(660.0, 0.08, &[(0.0, 0.4), (0.6, 0.4)], 3_000)
    }

    /// What the CALLER hears while it rings: the tone a European exchange
    /// sends — 425 Hz, a second on and four off — which is the cadence the
    /// apps synthesise too (ios RingbackTone, default `cept`).
    fn ringback() -> Option<Ring> {
        Ring::tone(425.0, 0.05, &[(0.0, 1.0)], 5_000)
    }

    fn tone(hertz: f32, volume: f32, pattern: &'static [(f64, f64)], every: u32) -> Option<Ring> {
        let context = web_sys::AudioContext::new().ok()?;
        // A tab that has not been clicked in cannot sound anything yet;
        // asking is all this can do, and the ring on screen carries the
        // rest (docs/protocol.md, "A browser is a client too").
        let _ = context.resume();
        let beat = {
            let context = context.clone();
            move || {
                for (at, length) in pattern {
                    beep(&context, hertz, volume, *at, *length);
                }
            }
        };
        beat();
        let beat = gloo_timers::callback::Interval::new(every, beat);
        Some(Ring {
            context,
            _beat: beat,
        })
    }
}

/// One beep, `at` seconds from now, `length` long — a sine with a soft edge,
/// which is a ring rather than a click.
fn beep(context: &web_sys::AudioContext, hertz: f32, volume: f32, at: f64, length: f64) {
    let Ok(oscillator) = context.create_oscillator() else {
        return;
    };
    let Ok(gain) = context.create_gain() else {
        return;
    };
    oscillator.set_type(web_sys::OscillatorType::Sine);
    oscillator.frequency().set_value(hertz);
    let now = context.current_time() + at;
    let level = gain.gain();
    level.set_value_at_time(0.0, now).ok();
    level.linear_ramp_to_value_at_time(volume, now + 0.02).ok();
    level.linear_ramp_to_value_at_time(0.0, now + length).ok();
    let _ = oscillator.connect_with_audio_node(&gain);
    let _ = gain.connect_with_audio_node(&context.destination());
    let _ = oscillator.start_with_when(now);
    let _ = oscillator.stop_with_when(now + length + 0.05);
}

/// `{urls, username, credential}` objects for the peer connection, from
/// what `GET /calls/ice` answered.
fn configuration(servers: &[IceServer]) -> RtcConfiguration {
    let list = js_sys::Array::new();
    for server in servers {
        let object = js_sys::Object::new();
        let urls = js_sys::Array::new();
        for url in &server.urls {
            urls.push(&JsValue::from_str(url));
        }
        set(&object, "urls", &urls);
        if let Some(username) = &server.username {
            set(&object, "username", &JsValue::from_str(username));
        }
        if let Some(credential) = &server.credential {
            set(&object, "credential", &JsValue::from_str(credential));
        }
        list.push(&object);
    }
    let config = js_sys::Object::new();
    set(&config, "iceServers", &list);
    config.unchecked_into()
}

fn set(object: &js_sys::Object, key: &str, value: &JsValue) {
    let _ = js_sys::Reflect::set(object, &JsValue::from_str(key), value);
}

fn description(kind: RtcSdpType, sdp: &str) -> RtcSessionDescriptionInit {
    let description = RtcSessionDescriptionInit::new(kind);
    description.set_sdp(sdp);
    description
}

/// The browser's candidate, in the wire's spelling.
fn as_wire(candidate: &RtcIceCandidate) -> IceCandidate {
    IceCandidate {
        candidate: candidate.candidate(),
        sdp_mid: candidate.sdp_mid(),
        sdp_mline_index: candidate.sdp_m_line_index(),
    }
}

/// The wire's candidate, in the browser's.
fn as_browser(candidate: &IceCandidate) -> Option<RtcIceCandidate> {
    let init = RtcIceCandidateInit::new(&candidate.candidate);
    if let Some(mid) = &candidate.sdp_mid {
        init.set_sdp_mid(Some(mid));
    }
    if let Some(index) = candidate.sdp_mline_index {
        init.set_sdp_m_line_index(Some(index));
    }
    RtcIceCandidate::new(&init).ok()
}

/// What this tab will send. A video call asks for the camera too — and,
/// when that is refused or there is none, asks again for the microphone
/// alone: a camera is never a reason to miss a call, and the far side's
/// picture still shows (docs/protocol.md, "Video"). `Media::camera` says
/// which of the two happened.
struct Media {
    stream: MediaStream,
    camera: bool,
}

async fn media_for(video: bool) -> Option<Media> {
    if let Some(stream) = ask(true, video).await {
        return Some(Media {
            stream,
            camera: video,
        });
    }
    if !video {
        return None;
    }
    ask(true, false).await.map(|stream| Media {
        stream,
        camera: false,
    })
}

async fn ask(audio: bool, video: bool) -> Option<MediaStream> {
    let devices = web_sys::window()?.navigator().media_devices().ok()?;
    let constraints = js_sys::Object::new();
    set(&constraints, "audio", &JsValue::from_bool(audio));
    set(&constraints, "video", &JsValue::from_bool(video));
    let promise = devices
        .get_user_media_with_constraints(&constraints.unchecked_into())
        .ok()?;
    JsFuture::from(promise).await.ok()?.dyn_into().ok()
}

/// A video call with no camera of our own still negotiates the video the
/// FAR side sends: a receive-only line, added where a track would be.
fn receive_video(pc: &RtcPeerConnection) {
    let init = web_sys::RtcRtpTransceiverInit::new();
    init.set_direction(web_sys::RtcRtpTransceiverDirection::Recvonly);
    let _ = pc.add_transceiver_with_str_and_init("video", &init);
}

fn tracks(stream: &MediaStream) -> Vec<MediaStreamTrack> {
    stream
        .get_tracks()
        .iter()
        .filter_map(|track| track.dyn_into::<MediaStreamTrack>().ok())
        .collect()
}

impl Calls {
    /// Place a call in `chat_id`, to the direct chat's other member. The
    /// order is the protocol's: the microphone first (a call nobody can be
    /// heard on is not worth ringing), then `GET /calls/ice`, then the
    /// offer as soon as the local description exists — candidates trickle
    /// after it.
    pub fn place(&self, token: String, chat_id: i64, peer_user_id: i64, video: bool) {
        if self.call().is_some_and(|call| call.stage == Stage::Ended) {
            self.live.now(|state| state.call = None);
        }
        if self.call().is_some() {
            return;
        }
        let call_id = uuid();
        self.live.now(|state| {
            state.call = Some(CallState::new(
                call_id.clone(),
                chat_id,
                peer_user_id,
                true,
                video,
            ));
            state.failure = None;
        });
        let this = self.clone();
        spawn_local(async move {
            let Some(media) = media_for(video).await else {
                this.finish(&call_id, "microphone_denied", None);
                return;
            };
            let local = media.stream;
            let servers = match crate::api::ice_servers(&token).await {
                Ok(servers) => servers,
                Err(error) => {
                    let code = error.code().unwrap_or("").to_string();
                    this.finish(&call_id, refusal_reason(&code), None);
                    return;
                }
            };
            // Still on it? It may have been cancelled while the browser
            // was asking for the microphone, or while the servers were
            // being fetched.
            if !this.still(&call_id) {
                stop(&local);
                return;
            }
            let Some(pc) = this.connect(&call_id, &local, &servers) else {
                stop(&local);
                this.finish(&call_id, "failed", None);
                return;
            };
            if video && !media.camera {
                receive_video(&pc);
                this.with(&call_id, |call| call.camera = false);
            }
            let offer = match JsFuture::from(pc.create_offer()).await {
                Ok(offer) => offer.unchecked_into::<RtcSessionDescriptionInit>(),
                Err(_) => {
                    // Placed but never described: the server has not heard
                    // of it at all, so there is nothing to tell it.
                    this.finish(&call_id, "failed", None);
                    return;
                }
            };
            let sdp = js_sys::Reflect::get(&offer, &JsValue::from_str("sdp"))
                .ok()
                .and_then(|sdp| sdp.as_string())
                .unwrap_or_default();
            if JsFuture::from(pc.set_local_description(&offer))
                .await
                .is_err()
            {
                this.finish(&call_id, "failed", None);
                return;
            }
            if !this.still(&call_id) {
                this.teardown();
                return;
            }
            let sent = this.wire.say(&ClientFrame::CallOffer {
                call_id: call_id.clone(),
                chat_id,
                sdp,
                video: video.then_some(true),
            });
            if !sent {
                this.finish(&call_id, "failed", None);
                return;
            }
            this.ring_guard(&call_id, true);
        });
    }

    /// Answer the call this tab is ringing with. The microphone is asked
    /// for here, from the click that answered — which is also the gesture
    /// that lets the remote audio play.
    pub fn answer(&self) {
        let Some(call) = self.call().filter(|call| call.stage == Stage::Incoming) else {
            return;
        };
        let Some(sdp) = self.inner.borrow().offer_sdp.clone() else {
            return;
        };
        let call_id = call.call_id.clone();
        let this = self.clone();
        spawn_local(async move {
            let Some(media) = media_for(call.video).await else {
                // A refusal on the wire — the protocol has no reason for
                // "my browser would not let me" — and the reason on screen
                // is the one that can be acted on. Leaving the caller
                // ringing at somebody who cannot speak is worse (ios sends
                // `decline` here too).
                this.finish(&call_id, "microphone_denied", Some("decline"));
                return;
            };
            let local = media.stream;
            let token = this
                .live
                .read(|state| state.token.clone())
                .unwrap_or_default();
            // A refusal the server MEANS — calls turned off — is the end of
            // it; a server that merely could not be reached leaves the
            // candidates this browser finds on its own, which are enough
            // on one network and honest about the rest.
            let servers = match crate::api::ice_servers(&token).await {
                Ok(servers) => servers,
                Err(error) => match error.code() {
                    Some(code) => {
                        stop(&local);
                        this.finish(&call_id, refusal_reason(code), Some("decline"));
                        return;
                    }
                    None => Vec::new(),
                },
            };
            if !this.still(&call_id) {
                stop(&local);
                return;
            }
            let Some(pc) = this.connect(&call_id, &local, &servers) else {
                stop(&local);
                this.hang_up("failed");
                return;
            };
            if call.video && !media.camera {
                receive_video(&pc);
                this.with(&call_id, |call| call.camera = false);
            }
            if JsFuture::from(pc.set_remote_description(&description(RtcSdpType::Offer, &sdp)))
                .await
                .is_err()
            {
                this.hang_up("failed");
                return;
            }
            this.remote_is_set(&call_id).await;
            let answer = match JsFuture::from(pc.create_answer()).await {
                Ok(answer) => answer.unchecked_into::<RtcSessionDescriptionInit>(),
                Err(_) => {
                    this.hang_up("failed");
                    return;
                }
            };
            let sdp = js_sys::Reflect::get(&answer, &JsValue::from_str("sdp"))
                .ok()
                .and_then(|sdp| sdp.as_string())
                .unwrap_or_default();
            if JsFuture::from(pc.set_local_description(&answer))
                .await
                .is_err()
            {
                this.hang_up("failed");
                return;
            }
            // The answer is what takes the call, so it is worth a few
            // tries: losing it to a socket a moment from opening would
            // leave the caller ringing at somebody who has said yes.
            this.say_retrying(ClientFrame::CallAnswer {
                call_id: call_id.clone(),
                sdp,
            });
            this.stop_ringing();
            this.with(&call_id, |call| {
                call.stage = Stage::Connecting;
                call.taken = true;
            });
            this.answer_guard(&call_id);
            // The media may already be up: the state change that says so
            // can land while the candidates held back are going in.
            this.catch_up(&pc, &call_id);
        });
    }

    /// Refuse it: the callee saying no, which ends it on every one of their
    /// devices (docs/protocol.md: there is no `call_decline` frame).
    pub fn decline(&self) {
        self.hang_up("decline");
    }

    /// End it the way this stage means (ios `performHangUp`): a call that
    /// was answered is a `hangup`, the caller giving up while it rings is a
    /// `cancel`, and the callee doing it is a `decline`.
    pub fn end(&self) {
        let Some(call) = self.call() else { return };
        let reason = match (call.stage, call.outgoing) {
            (Stage::Ended, _) => return,
            (Stage::Incoming, _) => "decline",
            (Stage::Dialling | Stage::Ringing, true) => "cancel",
            (Stage::Dialling | Stage::Ringing, false) => "decline",
            _ => "hangup",
        };
        self.hang_up(reason);
    }

    /// Say `call_end` with this reason, and put the call away.
    pub fn hang_up(&self, reason: &str) {
        let Some(call) = self.call() else { return };
        self.finish(&call.call_id, reason, Some(reason));
    }

    /// The tab is going: one synchronous frame, from the unload handler,
    /// where nothing polled afterwards would ever run.
    pub fn hang_up_on_unload(&self) {
        let Some(call) = self.call().filter(|call| call.stage != Stage::Ended) else {
            return;
        };
        let reason = unload_reason(&call);
        self.wire.say(&ClientFrame::CallEnd {
            call_id: call.call_id,
            reason: reason.to_string(),
        });
    }

    /// The one way a call ends here: the media given back, the reason left
    /// on screen for a moment, and the call remembered just long enough
    /// that a frame still in flight for it cannot start it again.
    ///
    /// `shown` is for the person and may be one of this client's own words
    /// (`microphone_denied`, `busy`); `say` is for the SERVER, which takes
    /// only the protocol's four — `hangup`, `decline`, `cancel`, `failed` —
    /// and answers `invalid_call` to anything else. None when the server is
    /// the one who said so, or when it never heard of the call: a ring
    /// guard fires long after the server's own 45 s.
    fn finish(&self, call_id: &str, shown: &str, say: Option<&str>) {
        if self.call().map(|call| call.call_id).as_deref() != Some(call_id) {
            return;
        }
        if let Some(reason) = say {
            debug_assert!(
                matches!(reason, "hangup" | "decline" | "cancel" | "failed"),
                "the wire takes only the protocol's four reasons"
            );
            self.say_retrying(ClientFrame::CallEnd {
                call_id: call_id.to_string(),
                reason: reason.to_string(),
            });
        }
        self.teardown();
        {
            let mut inner = self.inner.borrow_mut();
            inner.recently_ended.push_back(call_id.to_string());
            while inner.recently_ended.len() > REMEMBERED {
                inner.recently_ended.pop_front();
            }
        }
        let shown = shown.to_string();
        self.with(call_id, |call| {
            call.stage = Stage::Ended;
            call.ended_reason = Some(shown);
        });
        self.linger(call_id);
    }

    /// The panel goes after `ENDED_LINGER_MS`, unless another call has
    /// started in the meantime.
    fn linger(&self, call_id: &str) {
        let live = self.live.clone();
        let call_id = call_id.to_string();
        let timer = gloo_timers::callback::Timeout::new(ENDED_LINGER_MS, move || {
            live.now(|state| {
                let over = state
                    .call
                    .as_ref()
                    .is_some_and(|call| call.call_id == call_id && call.stage == Stage::Ended);
                if over {
                    state.call = None;
                }
            });
        });
        self.inner.borrow_mut().linger = Some(timer);
    }

    /// A frame worth a few tries: the socket may be a moment from opening —
    /// a call placed just as the network came back — and a call is a poor
    /// thing to lose to that (ios retries a send ten times, 500 ms apart).
    fn say_retrying(&self, frame: ClientFrame) {
        if self.wire.say(&frame) {
            return;
        }
        let wire = self.wire.clone();
        spawn_local(async move {
            for _ in 0..10 {
                gloo_timers::future::TimeoutFuture::new(500).await;
                if wire.say(&frame) {
                    return;
                }
            }
        });
    }

    pub fn toggle_mute(&self) {
        let muted = self.live.now(|state| {
            state.call.as_mut().map(|call| {
                call.muted = !call.muted;
                call.muted
            })
        });
        let Some(muted) = muted else { return };
        if let Some(local) = self.local_stream() {
            for track in tracks(&local) {
                if track.kind() == "audio" {
                    track.set_enabled(!muted);
                }
            }
        }
    }

    /// The camera, off and on. The call's KIND never changes: a track is
    /// disabled, nothing is renegotiated, and the far side simply sees the
    /// picture stop (docs/protocol.md, "Video").
    pub fn toggle_camera(&self) {
        let camera = self.live.now(|state| {
            state.call.as_mut().filter(|call| call.video).map(|call| {
                call.camera = !call.camera;
                call.camera
            })
        });
        let Some(camera) = camera else { return };
        if let Some(local) = self.local_stream() {
            for track in tracks(&local) {
                if track.kind() == "video" {
                    track.set_enabled(camera);
                }
            }
        }
    }
}

impl Calls {
    /// One peer connection, wired to this call: candidates out as they are
    /// gathered, tracks in as they arrive, and the connection's own state
    /// watched — because a call that dies is reported by the client from
    /// exactly here, as `failed`.
    fn connect(
        &self,
        call_id: &str,
        local: &MediaStream,
        servers: &[IceServer],
    ) -> Option<RtcPeerConnection> {
        let pc = RtcPeerConnection::new_with_configuration(&configuration(servers)).ok()?;
        let remote = MediaStream::new().ok()?;
        for track in tracks(local) {
            let _ = pc.add_track_0(&track, local);
        }

        let on_ice = {
            let this = self.clone();
            let call_id = call_id.to_string();
            Closure::<dyn FnMut(RtcPeerConnectionIceEvent)>::new(
                move |event: RtcPeerConnectionIceEvent| {
                    // The last candidate is a null one: gathering is done,
                    // and there is nothing to relay.
                    if let Some(candidate) = event.candidate() {
                        this.wire.say(&ClientFrame::CallIce {
                            call_id: call_id.clone(),
                            candidate: as_wire(&candidate),
                        });
                    }
                },
            )
        };
        pc.set_onicecandidate(Some(on_ice.as_ref().unchecked_ref()));

        let on_track = {
            let this = self.clone();
            let call_id = call_id.to_string();
            // `Clone::clone`, spelled out: `remote.clone()` would reach the
            // DOM's OWN `MediaStream.clone()`, which returns a NEW stream
            // holding COPIES of the tracks — the handler would then fill a
            // stream nothing is playing.
            let remote = Clone::clone(&remote);
            Closure::<dyn FnMut(RtcTrackEvent)>::new(move |event: RtcTrackEvent| {
                remote.add_track(&event.track());
                // The view attaches the stream again, and starts it.
                this.with(&call_id, |call| call.media += 1);
            })
        };
        pc.set_ontrack(Some(on_track.as_ref().unchecked_ref()));

        let on_state = {
            let this = self.clone();
            let call_id = call_id.to_string();
            let watched = pc.clone();
            Closure::<dyn FnMut()>::new(move || match watched.connection_state() {
                // Up. A call answered is a call talking.
                RtcPeerConnectionState::Connected => this.up(&call_id),
                // The media never came up, or it died: the client reports
                // it, because the server deliberately does not end a call
                // over a dropped socket (docs/protocol.md, "The sequence").
                RtcPeerConnectionState::Failed
                    if this.call().map(|call| call.call_id).as_deref()
                        == Some(call_id.as_str()) =>
                {
                    this.finish(&call_id, "failed", Some("failed"));
                }
                _ => {}
            })
        };
        pc.set_onconnectionstatechange(Some(on_state.as_ref().unchecked_ref()));

        let mut inner = self.inner.borrow_mut();
        inner.pc = Some(pc.clone());
        // Again `Clone::clone`: a DOM-cloned stream's tracks are copies,
        // so muting or stopping them would leave the real microphone on.
        inner.local = Some(Clone::clone(local));
        inner.remote = Some(remote);
        inner.remote_set = false;
        // What arrived while this tab was ringing STAYS: the caller
        // trickled candidates at a device that had no peer connection yet,
        // and those are exactly the ones a callee cannot do without
        // (docs/protocol.md, "The sequence").
        inner.on_ice = Some(on_ice);
        inner.on_track = Some(on_track);
        inner.on_state = Some(on_state);
        drop(inner);
        self.with(call_id, |call| call.media += 1);
        Some(pc)
    }

    /// The remote description is in: everything held back goes in now, in
    /// the order it arrived.
    async fn remote_is_set(&self, call_id: &str) {
        let (pc, early) = {
            let mut inner = self.inner.borrow_mut();
            inner.remote_set = true;
            (inner.pc.clone(), std::mem::take(&mut inner.early))
        };
        let Some(pc) = pc else { return };
        for candidate in early {
            self.add_candidate(&pc, &candidate).await;
        }
        let _ = call_id;
    }

    async fn add_candidate(&self, pc: &RtcPeerConnection, candidate: &IceCandidate) {
        if let Some(browser) = as_browser(candidate) {
            let _ = JsFuture::from(pc.add_ice_candidate_with_opt_rtc_ice_candidate(Some(&browser)))
                .await;
        }
    }

    /// This tab's own backstop for a call nobody answers — 90 s placing,
    /// 60 s ringing, both past the server's 45 s, and neither says a word:
    /// by then the server has ended the call and told both sides.
    fn ring_guard(&self, call_id: &str, outgoing: bool) {
        let this = self.clone();
        let call_id = call_id.to_string();
        let after = if outgoing {
            OUTGOING_RING_GUARD_MS
        } else {
            INCOMING_RING_GUARD_MS
        };
        let timer = gloo_timers::callback::Timeout::new(after, move || {
            let waiting = this
                .call()
                .is_some_and(|call| call.call_id == call_id && !call.answered());
            if waiting {
                this.finish(&call_id, "timeout", None);
            }
        });
        self.inner.borrow_mut().guard = Some(timer);
    }

    /// A call that was answered and never came up. Nothing on the server
    /// ends it before a minute of silence, so this one DOES say so.
    fn answer_guard(&self, call_id: &str) {
        let this = self.clone();
        let call_id = call_id.to_string();
        let timer = gloo_timers::callback::Timeout::new(ANSWER_GUARD_MS, move || {
            let silent = this
                .call()
                .is_some_and(|call| call.call_id == call_id && call.stage == Stage::Connecting);
            if silent {
                this.finish(&call_id, "failed", Some("failed"));
            }
        });
        self.inner.borrow_mut().guard = Some(timer);
    }

    /// The media is up: the call is a conversation, and the clock starts
    /// here rather than at the answer — setting up is not talking.
    fn up(&self, call_id: &str) {
        self.with(call_id, |call| {
            if call.stage == Stage::Connecting {
                call.stage = Stage::Talking;
                call.answered_at = Some(crate::sync::wall_ms());
            }
        });
    }

    /// The state change that says the media is up may land while the
    /// candidates held back are still going in — before there is a
    /// `Connecting` stage for it to promote. So it is read once more, from
    /// the connection itself, after the stage is set.
    fn catch_up(&self, pc: &RtcPeerConnection, call_id: &str) {
        if pc.connection_state() == RtcPeerConnectionState::Connected {
            self.up(call_id);
        }
    }

    fn stop_ringing(&self) {
        self.inner.borrow_mut().ring = None;
    }

    /// Whether a call has just ended here — an offer for it is a frame
    /// that crossed its own `call_end`.
    fn just_ended(&self, call_id: &str) -> bool {
        self.inner
            .borrow()
            .recently_ended
            .iter()
            .any(|held| held == call_id)
    }

    /// Everything the browser gave us, given back: the microphone light
    /// goes out, the connection closes, the handlers are dropped.
    pub fn teardown(&self) {
        let mut inner = self.inner.borrow_mut();
        inner.ring = None;
        inner.guard = None;
        if let Some(local) = inner.local.take() {
            stop(&local);
        }
        inner.remote = None;
        if let Some(pc) = inner.pc.take() {
            pc.set_onicecandidate(None);
            pc.set_ontrack(None);
            pc.set_onconnectionstatechange(None);
            pc.close();
        }
        inner.on_ice = None;
        inner.on_track = None;
        inner.on_state = None;
        inner.early.clear();
        inner.offer_sdp = None;
        inner.remote_set = false;
    }
}

/// Every track stopped: what turns the microphone and camera lights off.
fn stop(stream: &MediaStream) {
    for track in tracks(stream) {
        track.stop();
    }
}

fn uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// The frames, applied. Each is for the call this tab HOLDS and no other:
/// the protocol's one rule for a person with several devices, and the
/// reason none of this needs the server to know which device is doing what.
impl Calls {
    /// Somebody is calling. Every connection the callee has gets this, so a
    /// second copy of one already held is the duplicate it is (docs/protocol.md,
    /// "Late arrivals").
    pub fn offered(
        &self,
        call_id: String,
        chat_id: i64,
        from_user_id: i64,
        sdp: String,
        video: bool,
    ) {
        // A call that ended here a moment ago: this offer crossed its own
        // `call_end` on the way, and ringing again for it would ring for
        // nothing (ios keeps the same short memory).
        if self.just_ended(&call_id) {
            return;
        }
        // A call still on screen saying why it ended is not a call: the
        // next one takes its place at once.
        if self.call().is_some_and(|call| call.stage == Stage::Ended) {
            self.live.now(|state| state.call = None);
        }
        // Already ringing with it, or already on a call this tab cannot
        // leave for it: their other devices may still take it, and ending
        // it here would end it for all of them.
        if self.call().is_some() {
            return;
        }
        self.teardown();
        {
            let mut inner = self.inner.borrow_mut();
            inner.offer_sdp = Some(sdp);
            inner.ring = Ring::incoming();
        }
        self.live.now(|state| {
            state.call = Some(CallState::new(
                call_id.clone(),
                chat_id,
                from_user_id,
                false,
                video,
            ));
        });
        self.ring_guard(&call_id, false);
    }

    /// The server reached them: it is ringing on their side now — which is
    /// when the caller hears the tone for it.
    pub fn ringing(&self, call_id: &str) {
        let began = self.with(call_id, |call| {
            let first = call.stage == Stage::Dialling;
            if first {
                call.stage = Stage::Ringing;
            }
            first
        });
        if began == Some(true) {
            self.inner.borrow_mut().ring = Ring::ringback();
        }
    }

    /// They took it — on one of their devices, and this is the answer.
    pub fn answered(&self, call_id: String, sdp: String) {
        let ours = self
            .call()
            .is_some_and(|call| call.call_id == call_id && call.outgoing && !call.answered());
        if !ours {
            return;
        }
        let Some(pc) = self.inner.borrow().pc.clone() else {
            return;
        };
        let this = self.clone();
        spawn_local(async move {
            if JsFuture::from(pc.set_remote_description(&description(RtcSdpType::Answer, &sdp)))
                .await
                .is_err()
            {
                this.finish(&call_id, "failed", Some("failed"));
                return;
            }
            this.remote_is_set(&call_id).await;
            this.stop_ringing();
            this.with(&call_id, |call| {
                call.stage = Stage::Connecting;
                call.taken = true;
            });
            // The caller owes itself the same guard the callee has: nothing
            // on the server ends an answered call that never comes up
            // before a minute of silence.
            this.answer_guard(&call_id);
            this.catch_up(&pc, &call_id);
        });
    }

    /// A relayed candidate. Before the remote description is set it is
    /// HELD: the replay after a woken device's offer arrives in that order,
    /// but a live relay promises nothing.
    pub fn candidate(&self, call_id: &str, candidate: IceCandidate) {
        if self.call().map(|call| call.call_id).as_deref() != Some(call_id) {
            return;
        }
        let (pc, ready) = {
            let inner = self.inner.borrow();
            (inner.pc.clone(), inner.remote_set)
        };
        match (pc, ready) {
            (Some(pc), true) => {
                let this = self.clone();
                spawn_local(async move { this.add_candidate(&pc, &candidate).await });
            }
            _ => {
                let mut inner = self.inner.borrow_mut();
                // The same 64 the server buffers while a call rings: a
                // trickle nobody drains is not worth growing for.
                if inner.early.len() < 64 {
                    inner.early.push(candidate);
                }
            }
        }
    }

    /// It is over, and the reason is the server's: the panel says it.
    pub fn ended(&self, call_id: &str, reason: &str) {
        self.finish(call_id, reason, None);
    }

    /// A refused call frame — the door said no. It may arrive AFTER the
    /// `call_end` the server sent first (it ends a call it cannot deliver
    /// before it answers the error), and then it REFINES what the panel
    /// says: `finish` on the call already ended leaves the better reason,
    /// which is the whole reason this is not dropped.
    pub fn refused(&self, call_id: &str, code: &str) {
        self.finish(call_id, refusal_reason(code), None);
    }
}

#[cfg(test)]
impl Calls {
    /// How many remote candidates are waiting for the remote description.
    fn held_candidates(&self) -> usize {
        self.inner.borrow().early.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::AppState;
    use wasm_bindgen_test::wasm_bindgen_test;

    /// A call engine with nowhere to send: `Wire` holds no socket, so every
    /// frame it would say is dropped — which is what a tab with its socket
    /// down does anyway, and leaves the state machine to be read on its own.
    fn engine() -> Calls {
        let live = Live::new(
            AppState {
                token: Some("t".into()),
                ..AppState::default()
            },
            Rc::new(|| {}),
        );
        Calls::new(live, Wire::new())
    }

    fn offer(calls: &Calls, call_id: &str) {
        calls.offered(call_id.to_string(), 42, 9, "v=0".into(), false);
    }

    /// Every connection the callee has gets the offer, and a woken one gets
    /// it replayed: a second copy of the call already held is the duplicate
    /// it is, and starts nothing (docs/protocol.md, "Late arrivals").
    #[wasm_bindgen_test]
    fn a_second_copy_of_one_offer_starts_nothing() {
        let calls = engine();
        offer(&calls, "one");
        let first = calls.call().expect("ringing");
        assert_eq!(first.stage, Stage::Incoming);
        assert!(!first.outgoing && first.peer_user_id == 9 && first.chat_id == 42);
        // The replay can land just after this device answered: the
        // duplicate must not put the call back to ringing.
        calls.live.now(|state| {
            let call = state.call.as_mut().expect("held");
            call.stage = Stage::Connecting;
            call.answered_at = Some(1.0);
        });
        offer(&calls, "one");
        let held = calls.call().expect("still the same call");
        assert_eq!(held.stage, Stage::Connecting, "the answer stands");
        assert_eq!(held.media, first.media);
    }

    /// An offer for a call that ended here a moment ago crossed its own
    /// `call_end`: ringing for it would ring for nothing.
    #[wasm_bindgen_test]
    fn an_offer_that_crossed_its_own_end_does_not_ring() {
        let calls = engine();
        offer(&calls, "one");
        calls.hang_up("decline");
        calls.live.now(|state| state.call = None);
        offer(&calls, "one");
        assert!(calls.call().is_none(), "it is over, and stays over");
        // Another call is another matter.
        offer(&calls, "two");
        assert!(calls.call().is_some());
    }

    /// The reason follows the stage, as the apps' `performHangUp` does:
    /// answered is a hangup, the caller giving up is a cancel, and the
    /// callee saying no is a decline.
    #[wasm_bindgen_test]
    fn the_reason_follows_the_stage() {
        let calls = engine();
        offer(&calls, "in");
        calls.end();
        assert_eq!(
            calls.call().and_then(|call| call.ended_reason),
            Some("decline".to_string())
        );
        // Ending it again says nothing new — and must not overwrite what
        // the panel is saying with a reason nobody chose.
        calls.end();
        assert_eq!(
            calls.call().and_then(|call| call.ended_reason),
            Some("decline".to_string()),
            "a call already over is left alone"
        );

        let calls = engine();
        calls.live.now(|state| {
            state.call = Some(CallState::new("out".into(), 42, 9, true, false));
        });
        calls.end();
        assert_eq!(
            calls.call().and_then(|call| call.ended_reason),
            Some("cancel".to_string())
        );

        let calls = engine();
        calls.live.now(|state| {
            let mut call = CallState::new("up".into(), 42, 9, true, false);
            call.stage = Stage::Talking;
            call.taken = true;
            call.answered_at = Some(1.0);
            state.call = Some(call);
        });
        calls.end();
        assert_eq!(
            calls.call().and_then(|call| call.ended_reason),
            Some("hangup".to_string())
        );
        // And a call already over is not ended twice.
        calls.end();
        assert_eq!(
            calls.call().and_then(|call| call.ended_reason),
            Some("hangup".to_string())
        );
    }

    /// A candidate that arrives before the remote description is HELD, not
    /// dropped: a replayed offer is followed by the candidates gathered
    /// while a device was waking (docs/protocol.md, "The sequence").
    #[wasm_bindgen_test]
    fn candidates_before_the_remote_description_are_held() {
        let calls = engine();
        offer(&calls, "one");
        let candidate = IceCandidate {
            candidate: "candidate:1".into(),
            sdp_mid: Some("0".into()),
            sdp_mline_index: Some(0),
        };
        calls.candidate("one", candidate.clone());
        calls.candidate("one", candidate.clone());
        assert_eq!(calls.held_candidates(), 2);
        // For another call, nothing is held at all.
        calls.candidate("another", candidate);
        assert_eq!(calls.held_candidates(), 2);
    }

    /// A frame naming a call this tab does not hold is ignored in silence —
    /// the rule that lets one person be signed in twice.
    #[wasm_bindgen_test]
    fn a_frame_for_another_call_is_ignored() {
        let calls = engine();
        // A call of this tab's own, waiting to be told it is ringing.
        calls.live.now(|state| {
            state.call = Some(CallState::new("mine".into(), 42, 9, true, false));
        });
        calls.ringing("theirs");
        assert_eq!(
            calls.call().map(|call| call.stage),
            Some(Stage::Dialling),
            "another call's ringing is not this one's"
        );
        calls.ringing("mine");
        assert_eq!(calls.call().map(|call| call.stage), Some(Stage::Ringing));
        calls.answered("theirs".into(), "v=0".into());
        calls.ended("theirs", "hangup");
        let held = calls.call().expect("still ringing");
        assert_eq!(held.call_id, "mine");
        assert_eq!(held.stage, Stage::Ringing);
    }

    /// The server ends a call it cannot deliver BEFORE it answers the
    /// error, so the error lands on a call already ended — and says better
    /// what happened than the `cancel` that came first.
    #[wasm_bindgen_test]
    fn an_error_refines_the_call_it_ends() {
        let calls = engine();
        calls.live.now(|state| {
            state.call = Some(CallState::new("out".into(), 42, 9, true, false));
        });
        calls.ended("out", "cancel");
        assert_eq!(
            calls.call().and_then(|call| call.ended_reason),
            Some("cancel".to_string())
        );
        calls.refused("out", "peer_unreachable");
        let held = calls.call().expect("still on screen");
        assert_eq!(held.stage, Stage::Ended);
        assert_eq!(held.ended_reason.as_deref(), Some("unreachable"));
        assert_eq!(ended_line("unreachable", &held), "Unavailable");
    }

    /// The trap that cost an afternoon, written down as a test: web-sys
    /// generates an inherent `clone()` for `MediaStream` — the DOM's own
    /// method, which returns a NEW stream holding COPIES of the tracks —
    /// and it SHADOWS `Clone::clone`. Handing one to a track handler and
    /// the other to an element gives a call with no sound.
    #[wasm_bindgen_test]
    fn cloning_a_stream_the_rust_way_keeps_the_same_stream() {
        let stream = MediaStream::new().expect("a stream");
        let rust = Clone::clone(&stream);
        let dom = stream.clone();
        assert!(
            js_sys::Object::is(rust.as_ref(), stream.as_ref()),
            "Clone::clone is the same stream"
        );
        assert!(
            !js_sys::Object::is(dom.as_ref(), stream.as_ref()),
            "`.clone()` is the DOM's clone: another stream"
        );
    }

    /// What a refusal is called, and what each is said as.
    #[wasm_bindgen_test]
    fn a_refusal_is_named_and_said() {
        assert_eq!(refusal_reason("peer_busy"), "busy");
        assert_eq!(refusal_reason("call_busy"), "busy");
        assert_eq!(refusal_reason("peer_unreachable"), "unreachable");
        // Not one word for both, as the apps have it: a person's own block
        // is theirs to undo, and a server that carries no calls is a fact
        // about the server.
        assert_eq!(refusal_reason("blocked"), "blocked");
        assert_eq!(refusal_reason("calls_disabled"), "calls_disabled");
        assert_eq!(
            refusal_reason("video_calls_disabled"),
            "video_calls_disabled"
        );
        assert_eq!(refusal_reason("invalid_call"), "failed");

        let mut call = CallState::new("c".into(), 42, 9, true, false);
        assert_eq!(ended_line("timeout", &call), "No answer");
        assert_eq!(ended_line("decline", &call), "Declined");
        assert_eq!(ended_line("hangup", &call), "Call ended");
        assert_eq!(ended_line("failed", &call), "Call failed");
        assert_eq!(
            ended_line("answered_elsewhere", &call),
            "Answered on another device"
        );
        assert_eq!(ended_line("blocked", &call), "You've blocked them.");
        assert_eq!(
            ended_line("calls_disabled", &call),
            "Calls are off on this server."
        );
        assert_eq!(
            ended_line("video_calls_disabled", &call),
            "Video calls are off on this server."
        );
        assert_eq!(ended_line("microphone_denied", &call), NO_MICROPHONE);
        // The callee's own words differ: a call they did not take is the
        // missed call the record calls it.
        call.outgoing = false;
        assert_eq!(ended_line("timeout", &call), "Missed voice call");
        call.video = true;
        assert_eq!(ended_line("timeout", &call), "Missed video call");
        assert_eq!(ended_line("decline", &call), "Call ended");
    }

    /// A tab on its way out says the reason its stage means, and says
    /// nothing at all about a call that is already over.
    #[wasm_bindgen_test]
    fn a_closing_tab_says_what_its_stage_means() {
        let calls = engine();
        calls.live.now(|state| {
            state.call = Some(CallState::new("out".into(), 42, 9, true, false));
        });
        // Nothing to assert on the wire — there is no socket in a test —
        // but the call must be left alone: the tab is going, and the frame
        // is the last thing it does.
        calls.hang_up_on_unload();
        assert_eq!(calls.call().map(|call| call.stage), Some(Stage::Dialling));
    }

    /// THE SET-UP GUARD. A call cancelled while the browser was asking for
    /// the microphone must not go on to be placed: the panel is gone, and
    /// nothing would ever turn the microphone off again.
    #[wasm_bindgen_test]
    fn a_call_cancelled_during_set_up_is_not_still_on() {
        let calls = engine();
        calls.live.now(|state| {
            state.call = Some(CallState::new("out".into(), 42, 9, true, false));
        });
        assert!(calls.still("out"), "on it");
        assert!(!calls.still("other"), "never another call");
        calls.end();
        assert!(
            !calls.still("out"),
            "an ended call keeps its id on screen, and is not still on"
        );
    }

    /// THE CANDIDATES A CALLEE CANNOT DO WITHOUT. They arrive while the tab
    /// rings, before there is a peer connection; building one must not
    /// throw them away, or an answered call has no candidates at all.
    #[wasm_bindgen_test]
    fn building_the_connection_keeps_what_arrived_while_ringing() {
        let calls = engine();
        offer(&calls, "one");
        let candidate = IceCandidate {
            candidate: "candidate:1".into(),
            sdp_mid: Some("0".into()),
            sdp_mline_index: Some(0),
        };
        calls.candidate("one", candidate.clone());
        calls.candidate("one", candidate);
        assert_eq!(calls.held_candidates(), 2);
        let stream = MediaStream::new().expect("a stream");
        assert!(calls.connect("one", &stream, &[]).is_some());
        assert_eq!(
            calls.held_candidates(),
            2,
            "still there, for the remote description to let them in"
        );
        calls.teardown();
    }

    /// The buffer is capped where the server caps its own.
    #[wasm_bindgen_test]
    fn the_candidate_buffer_is_capped() {
        let calls = engine();
        offer(&calls, "one");
        for index in 0..80 {
            calls.candidate(
                "one",
                IceCandidate {
                    candidate: format!("candidate:{index}"),
                    ..IceCandidate::default()
                },
            );
        }
        assert_eq!(calls.held_candidates(), 64);
    }

    /// The clock counts from the media coming UP, not from the answer:
    /// setting up is not talking (the apps count from exactly here).
    #[wasm_bindgen_test]
    fn the_clock_starts_when_the_media_does() {
        let calls = engine();
        calls.live.now(|state| {
            let mut call = CallState::new("c".into(), 42, 9, false, false);
            call.stage = Stage::Connecting;
            call.taken = true;
            state.call = Some(call);
        });
        assert_eq!(calls.call().and_then(|call| call.answered_at), None);
        calls.up("c");
        let held = calls.call().expect("held");
        assert_eq!(held.stage, Stage::Talking);
        assert!(held.answered_at.is_some(), "the clock starts here");
        // And a second Connected changes nothing.
        let started = held.answered_at;
        calls.up("c");
        assert_eq!(calls.call().and_then(|call| call.answered_at), started);
    }

    /// A call still saying why it ended is not a call: the next one takes
    /// its place at once, rather than waiting out the two seconds.
    #[wasm_bindgen_test]
    fn a_new_call_takes_an_ended_one_s_place() {
        let calls = engine();
        offer(&calls, "one");
        calls.hang_up("decline");
        assert_eq!(calls.call().map(|call| call.stage), Some(Stage::Ended));
        offer(&calls, "two");
        let held = calls.call().expect("ringing again");
        assert_eq!(held.call_id, "two");
        assert_eq!(held.stage, Stage::Incoming);
    }

    /// What a closing tab owes the other side, by who it was to the call.
    #[wasm_bindgen_test]
    fn a_closing_tab_owes_the_right_word() {
        let mut call = CallState::new("c".into(), 42, 9, true, false);
        assert_eq!(unload_reason(&call), "cancel");
        call.outgoing = false;
        assert_eq!(
            unload_reason(&call),
            "decline",
            "a callee's tab going is a refusal, not a missed call"
        );
        call.taken = true;
        assert_eq!(unload_reason(&call), "hangup");
    }
}
