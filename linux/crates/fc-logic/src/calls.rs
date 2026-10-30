//! One call's signalling and state (docs/protocol.md, "Voice calls") — a port of
//! `win/src/FamilyConnect.App.Logic/Calls.cs` (web `calls.rs`, ios `CallManager`): placing,
//! ringing, answering, the candidates, and the one way a call ends — with the media behind
//! [`CallMedia`], which on Linux is GStreamer's `webrtcbin` (the shell's `calls::GstCallMedia`,
//! shell-design §14) and in a test a fake.
//!
//! **A FRAME IS APPLIED ONLY TO THE CALL THIS DEVICE HOLDS**, and every other is ignored in
//! silence — the rule that lets one person be signed in on several devices without the server
//! tracking which is doing what.
//!
//! **REMOTE CANDIDATES BEFORE THE REMOTE DESCRIPTION ARE HELD** (the server's own 64): a replayed
//! offer is followed by candidates gathered while a device woke, and a live relay promises no
//! order. And **LOCAL CANDIDATES WAIT FOR THIS DEVICE'S OFFER OR ANSWER**: the server knows no call
//! before its offer, and relays a callee's candidates only once it has answered — `webrtcbin`
//! gathers on its own threads, and nothing promises which lands first here.
//!
//! **EVERY AWAIT IN SETTING A CALL UP IS FOLLOWED BY "STILL ON IT?"**: a call cancelled while the
//! microphone was being asked for must not go on to be placed, which would leave a live microphone
//! behind a panel that has gone.
//!
//! Driven from one thread — the GLib main context — as frames, media events and clicks all arrive
//! there.
//!
//! # How the shell uses it
//!
//! One [`CallEngine`] per signed-in server, over that server's socket, `ApiClient::ice_servers`
//! and the window's ONE media object (kept across calls and across servers, as Windows keeps its
//! page):
//!
//! ```text
//! let engine = CallEngine::new(socket, ice_servers, media, clock, spawn);
//! router.on_call().subscribe(move |frame| engine.hear(frame));   // call frames + call errors
//! engine.on_changed().subscribe(|_| redraw the Call window from engine.call());
//! spawn_local(engine.place(chat_id, peer_user_id, video));       // Call / Video Call / Call back
//! spawn_local(engine.answer());                                  // Accept, app.answer-call
//! engine.decline() / engine.end() / engine.toggle_mute() / engine.toggle_camera();
//! engine.leave().await; engine.detach();                         // quitting, or another server
//! ```
//!
//! `spawn` is how the engine starts the work the C# started with `_ = SomethingAsync()` — the
//! guards, the linger, the retried `call_end`/`call_answer`, the candidates going out:
//! `Rc::new(|future| { glib::MainContext::default().spawn_local(future); })` in the app, a
//! `LocalPool`'s spawner in a test. Time is the [`Clock`] seam. A guard is cancelled with an
//! [`rt::Cancel`](Cancel); a C# `event` is an [`Event`].
//!
//! # What Linux adds to the C#
//!
//! `webrtcbin` is not a browser page, and the engine meets it half way so the shell's media can
//! stay thin and the rules stay tested here:
//!
//! - **An m-line INDEX for every remote candidate.** `webrtcbin`'s `add-ice-candidate` takes the
//!   index alone, while a peer may send only `sdp_mid` (the wire carries either or both). The
//!   engine resolves a missing index from the mid against the remote description it set
//!   ([`sdp::mline_of`]); a candidate naming neither, or a mid that description lacks, is not
//!   passed on — a browser would refuse it too (`addIceCandidate` → `OperationError`), and
//!   Windows ignored that refusal.
//! - **Both halves on every local candidate.** `on-ice-candidate` gives the index alone; the
//!   engine adds the `sdp_mid` of that m-line from its own offer or answer ([`sdp::mid_of`]), so
//!   every peer — Android's `IceCandidate(sdpMid ?: "", …)` included — gets what a browser sends.
//! - **`webrtcbin`'s ICE servers** are a `stun-server` URI and `add-turn-server` URIs, not
//!   `RTCIceServer` objects: [`webrtcbin::ice_config`] makes them, with the TURN username and
//!   credential PERCENT-ENCODED into `turn(s)://user:pass@host:port` — coturn's REST username is
//!   `<expiry>:<user_id>` and its credential base64 (`+`, `/`, `=`). Proven through coturn 4.6.1
//!   with such a username and credential (`tools/call-interop`: ALLOCATE, CREATE_PERMISSION and
//!   CHANNEL_BIND succeed for it). Read from Ubuntu 26.04's GStreamer 1.28.2: `libgstwebrtcnice`
//!   imports `gst_uri_from_string_escaped`, `g_uri_unescape_segment` and `g_uri_unescape_string`
//!   and warns that it "will assume that the first ':' delineates user:pass. You should escape the
//!   user and pass", and `gst-inspect-1.0 webrtcbin` says of `turn-server` that the `:` of
//!   `timestamp:username` and the base64 password "should be escaped to be parsed properly".
//! - **`receive_video_only`** (a video call without a camera of our own) is a `recvonly` video
//!   transceiver, and [`sdp::payload_type`] tells the media which codecs and payload types an offer
//!   carries (an H.264-only offer; Firefox's Opus 109 / VP8 120 against libwebrtc's 111 / 96).
//!
//! # Where Linux differs from the C# on purpose (measured on .NET 10 with `CallEngineTests`' fakes)
//!
//! 1. **A setup that is no longer wanted gives the media back only if no other call holds it.**
//!    The C# closes the media after any await that finds its call gone — and the media is ONE
//!    object: a placement cancelled while the microphone (or the camera portal) was being asked
//!    for, with an incoming call ringing by the time the answer came, closed THAT call's media —
//!    `call.html`'s `close` stops the ring, so the call rang silent (measured: the log after the
//!    grant is `[close]` with call "two" `Incoming`). Here the media is left to the call that holds
//!    it, whose own `open` or end releases what the stale grant opened (`open` releases whatever a
//!    previous `open` left — the contract below).
//! 2. **A candidate for a call that is over is not held.** The C# holds a `call_ice` for any call
//!    it holds, ended or not, and placing the next call clears the ended one without clearing the
//!    held candidates — so a late candidate for the last call went into the NEXT call's peer
//!    connection when its answer came (measured: `[candidate:stale]` added to `call-2`). Here a
//!    candidate is held only for a call still on, as the Mac's `startCall` (`resetToIdle`) has it.
//! 3. **A call is remembered once.** `call_end` and then the `error` that refines it end the same
//!    call twice, and the C# remembered it twice, so its sixteen "just ended" slots held fifteen
//!    calls.
//! 4. **The ended state is in place before the media is torn down.** The C# tears down, then sets
//!    `Ended`; a media that reported its own `close` as `failed` would end the call again from
//!    inside the teardown, and again — here that report finds the call already over.
//!
//! Kept as the C# has it, and reported: `decline`/`hang_up` on a call already over says
//! `call_end` again (measured; the server drops a `call_end` for a call it no longer holds, in
//! silence), and [`CallEngine::leave`] leaves the call's state as it was (the engine is about to go).
//!
//! # The webrtcbin `CallMedia`
//!
//! The contract the shell's `calls::GstCallMedia` (shell-design §14, package P6) implements. ONE
//! object per window, kept across calls; ONE `gst::Pipeline` per call, built by `open` and
//! `connect` and dropped by `close`.
//!
//! **Threads.** Every method is called on the GLib main context. `webrtcbin` signals and promises
//! arrive on GStreamer's threads: the handler only posts to the main context — a
//! `futures_channel` channel drained by a `spawn_local` task, or `glib::idle_add_local_once` —
//! **never `MainContext::invoke`**, which runs AT ONCE when the caller is already on the main
//! thread and would re-enter the engine from inside one of its own calls. The events are emitted
//! on the main context only. `close`, `stop_ringing`, `ring`, `set_muted` and `set_camera` never
//! emit anything synchronously, and `failed` is never emitted for a teardown the engine asked
//! for. A candidate emitted from inside `create_offer`/`create_answer` is fine (the engine holds
//! it until its offer or answer has gone). Each async method resolves through a
//! `futures_channel::oneshot` fed by `gst::Promise::with_change_func`; after `close` every pending
//! one resolves to its failure value (`MediaGrant::None`, `false`, `None`) rather than hanging.
//!
//! **Logs.** The call id and states only — never an SDP, a candidate (it is an address), a TURN
//! credential or a name.
//!
//! **The pipeline** (the phase-0 spike's, `tools/rtc-spike`, plus §14.1's echo canceller; as
//! `calls::GstCallMedia` builds it and `tools/call-interop` proves it against Linux, Chromium 140
//! and Firefox 141 peers):
//!
//! ```text
//! webrtcbin name=call bundle-policy=max-bundle
//! pulsesrc                                     (no device: PipeWire's default source, §14.2)
//!   ! audioconvert ! audioresample ! audio/x-raw,rate=48000
//!   ! webrtcdsp echo-cancel=true noise-suppression=true gain-control=true
//!   ! volume name=mute ! audioconvert ! opusenc ! rtpopuspay pt=111
//!   ! application/x-rtp,media=audio,encoding-name=OPUS,payload=111,clock-rate=48000,
//!     encoding-params=(string)2 ! call.
//! camera (P3's media::camera: pipewiresrc on the portal's fd and node)
//!   ! videoconvert ! videoscale ! video/x-raw,width=640,height=480,pixel-aspect-ratio=1/1,
//!     framerate=30/1,format=I420                    (letterboxed: VP8 carries no aspect ratio)
//!   ! tee name=cam
//!   cam. ! queue ! valve name=camera drop=false
//!        ! vp8enc deadline=1 keyframe-max-dist=30 ! rtpvp8pay pt=96
//!        ! application/x-rtp,media=video,encoding-name=VP8,payload=96,clock-rate=90000 ! call.
//!        (answering an offer whose video has no VP8: x264enc tune=zerolatency
//!        speed-preset=ultrafast ! video/x-h264,profile=constrained-baseline
//!        ! rtph264pay config-interval=-1 ! …,encoding-name=H264,clock-rate=90000,
//!        packetization-mode=(string)1 instead)
//!   cam. ! queue ! videoflip method=horizontal-flip ! gtk4paintablesink   (the self-view)
//! call. (pad-added, each far-side stream) ! decodebin3
//!   audio ! audioconvert ! audioresample ! webrtcechoprobe ! audioconvert ! pulsesink
//!   video ! videoconvert ! gtk4paintablesink                              (the far side)
//! ```
//!
//! The probe sits after the decoder, right before the speaker, so the canceller hears what the
//! room hears. `on-negotiation-needed` is ignored: the engine decides when to offer.
//!
//! **The send caps are COMPLETE** — clock rate, Opus's two channels, H.264's packetization mode.
//! `webrtcbin` writes the offer from its sink pads' caps before any data has negotiated them, so
//! without them the offer said `a=rtpmap:111 OPUS/48000` with no `/2` (measured): Chromium
//! rejected the audio m-line and Firefox's answer was refused at once. **The payload numbers are
//! 111 / 96 in an offer and the OFFER'S OWN in an answer**: answering Firefox's Opus 109 / VP8 120
//! with tails linked as 111 / 96, `webrtcbin` answered both m-lines `recvonly` and Firefox received
//! nothing — a silent one-way call (measured). So the send tails are linked, and the pipeline
//! plays, only once those numbers are known: in `create_offer`, or in `set_remote` of an offer.
//! **A far-side stream is linked on the main context**: the `pad-added` handler only blocks the
//! pad and posts it, since `gtk4paintablesink` must be made on the main thread and an element
//! added from a streaming thread races the teardown.
//!
//! | `CallMedia` | `webrtcbin` / GStreamer |
//! | --- | --- |
//! | `open(video)` | First release whatever a previous `open` left (the engine relies on it). The microphone branch, brought to `PAUSED` to prove the source opens: failing → `MediaGrant::None` (the call ends `microphone_denied`). With `video`, the camera through P3's portal path: refused, absent or busy → `Microphone` (a camera is never a reason to miss a call); both → `MicrophoneAndCamera`. Nothing is linked to `webrtcbin` yet. |
//! | `connect(servers, receive_video_only)` | Add `webrtcbin bundle-policy=max-bundle`; from [`webrtcbin::ice_config`]`(&servers)`: `stun-server` ← `stun_server`, and `add-turn-server` (a `gboolean`; `false` is logged, not fatal) for each of `turn_servers`. Remember `receive_video_only`. Connect `on-ice-candidate`, `notify::ice-connection-state`, `notify::connection-state`, `pad-added`/`pad-removed`; pipeline → `PAUSED` (the send tails are linked, and it plays, when the payload numbers are known — above). `is_connected` → false. `false` when an element is missing or a link or state change fails. |
//! | `create_offer()` | Link what `open` opened to `sink_%u` request pads as Opus 111 / VP8 96 with the complete caps above; pipeline → `PLAYING`. With `receive_video_only`: `add-transceiver(Recvonly, application/x-rtp,media=video,encoding-name=VP8,payload=96,clock-rate=90000)` first (every peer takes VP8 — the spike). `create-offer(None, promise)` → the reply's `offer` → `set-local-description(offer, promise)`, awaited → `offer.sdp().as_text()`. `None` on any failure. |
//! | `set_remote(is_offer, sdp)` | `gst_sdp::SDPMessage::parse_buffer` (a parse failure → `false`). For an OFFER, first decide the video branch — VP8 when [`sdp::payload_type`]`(sdp, "video", "VP8")` is `Some`, else H.264 (constrained baseline with packetization mode 1 preferred among the offer's H.264 lines) — link the send tails with the OFFER'S payload numbers and play; and give every video m-line this side sends no camera to (`receive_video_only`, and a voice call whose offer carries a video m-line, the web client's old shape) a `recvonly` transceiver in that ONE codec: left alone, `webrtcbin` answers such an m-line `recvonly` with every codec the offer listed (VP8, AV1 and VP9 for Chromium, measured), and the far side may send one there is no decoder for; a voice call's video m-line is then answered `inactive` (measured against Chromium). Then `WebRTCSessionDescription::new(Offer \| Answer, message)` → `set-remote-description(description, promise)`; `false` when the promise does not reply or its reply carries an `error`. |
//! | `create_answer()` | `create-answer(None, promise)` → `answer` → `set-local-description`, awaited → SDP text, or `None`. |
//! | `add_candidate(c)` | `c.sdp_mline_index` is always `Some` (the engine sees to it). An empty `c.candidate` is the far side's end-of-candidates: skip it, as the spike did. Else `add-ice-candidate(index as u32, &c.candidate)` (or `add-ice-candidate-full` with a promise, to await it). |
//! | `is_connected()` | True once `ice-connection-state` reached `connected` or `completed` on this call; false after `connect` and `close`. |
//! | `on_candidate_gathered` | `on-ice-candidate(mline: u32, candidate: String)` → `IceCandidate { candidate, sdp_mid: None, sdp_mline_index: Some(mline) }` (the engine adds the mid). Never an empty one. |
//! | `on_connected` | `ice-connection-state` → `connected`/`completed` (more than once is harmless). |
//! | `on_failed` | `ice-connection-state` → `failed`, or `connection-state` → `failed`. NOT `disconnected`, which may recover — the Mac, Android and the browsers all leave it alone, and so does this engine (it has no grace timer: shell-design §14.1's "`Disconnected` lasting past fc-logic's grace" does not exist in `Calls.cs` or the Mac). **Measured**: `webrtcbin`'s libnice agent runs RFC 7675 consent freshness (`consent-freshness=true`, `keepalive-conncheck=true` on every call), so a peer that vanished — frozen, killed, a browser's container killed — took ICE from `completed` to `failed` in 24–30 s. The media's own rule (no RTP on the far side's audio for 30 s once connected → `failed`; a muted libwebrtc track still sends Opus silence, DTX is off unless the SDP says `usedtx=1`) is armed only for an agent WITHOUT consent freshness, where nothing else would end the call — the server's own `failed` needs NEITHER party connected for 60 s. |
//! | `on_media_changed` | A far-side stream linked to its sink (`pad-added`) or gone (`pad-removed`). |
//! | `set_muted(muted)` | `volume name=mute` `mute` — after the canceller, before the encoder: silence keeps flowing, nothing is renegotiated. |
//! | `set_camera(on)` | `valve name=camera` `drop = !on` on the encoder branch: no frames, no renegotiation — "the far side simply sees the stream stop" (protocol "Video"). The self-view is hidden by the window (`CallState::camera`). |
//! | `ring(tone)` / `stop_ringing()` | §14.4: the tones in a pipeline of their own; `ring` replaces a tone already sounding; `stop_ringing` is idempotent and silent when nothing rings. |
//! | `close()` | Stop ringing; pipeline → `NULL`; the camera's portal session and fd released — "the microphone light goes out"; pending futures resolved as failures; `is_connected` → false; no event after it. Idempotent. |
//!
//! Not in any client and not in the protocol's frames: an ICE restart (a renegotiation would need
//! a frame the protocol does not carry — every client drops a second `call_offer` for a call it
//! holds as the duplicate it is). A network change that breaks the media ends the call `failed`.
//! Proven in Docker by `tools/call-interop` (shell-design §14.1): `webrtcdsp` in the loop (with
//! test sources), an H.264-only offer (a Linux peer offering it, and Chrome's H.264 lines in a
//! captured offer), TURN through coturn with REST credentials, a web client's VOICE call with a
//! `recvonly` video m-line (answered `inactive`), Firefox's payload numbers, mute and camera-off,
//! teardown from either side, and a vanished peer. Still to prove in phase 5, on real devices:
//! the camera from the portal, Apple's and Android's libwebrtc (their H.264-first offers, and our
//! `a=bundle-only` video m-line), H.264 from a real browser, a real microphone and speaker through
//! PipeWire with the echo canceller cancelling, `gtk4paintablesink` with GL on Wayland, STUN across
//! a real NAT, TURN over TCP/TLS.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::future::Future;
use std::rc::{Rc, Weak};
use std::time::Duration;

use futures_util::future::select;
use time::OffsetDateTime;

use fc_core::protocol::send_pipeline::uuid_v4;
use fc_core::protocol::{
    ApiResult, FrameSender, IceCandidate, IceServerDto, IceServersResponse, ServerFrame,
    client_frames,
};
use fc_core::rt::{Cancel, Clock, Event, LocalBoxFuture, Subscription};

/// Where a call has got to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CallStage {
    /// Placed; the server has not said it reached them yet.
    Dialling,
    /// Ringing on their side.
    Ringing,
    /// Somebody is calling, and this device is one of those ringing.
    Incoming,
    /// Answered, and the media is coming up.
    Connecting,
    /// The media is up.
    Talking,
    /// Over. The panel says why for a moment, then goes.
    Ended,
}

/// A call as the screen needs it: what a view compares and redraws from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallState {
    pub call_id: String,
    pub chat_id: i64,
    pub peer_user_id: i64,
    pub outgoing: bool,
    /// Decided when it was placed and fixed for its life: cameras toggle, the kind does not.
    pub video: bool,
    pub stage: CallStage,
    pub muted: bool,
    /// This device's camera, on — never true on a voice call.
    pub camera: bool,
    /// Answered — here, or for the caller by the far side: what makes ending it a `hangup`.
    pub taken: bool,
    /// When the media came up, by this device's clock: the on-screen duration counts from here,
    /// so setting up is not billed to the conversation.
    pub answered_at: Option<OffsetDateTime>,
    /// Why it ended, while the panel says so: a protocol reason, or this client's own
    /// (`microphone_denied`).
    pub ended_reason: Option<String>,
}

impl CallState {
    /// `new CallState(callId, chatId, peerUserId, outgoing, video, stage)`, the rest at their
    /// defaults.
    pub fn new(
        call_id: &str,
        chat_id: i64,
        peer_user_id: i64,
        outgoing: bool,
        video: bool,
        stage: CallStage,
    ) -> Self {
        CallState {
            call_id: call_id.to_string(),
            chat_id,
            peer_user_id,
            outgoing,
            video,
            stage,
            muted: false,
            camera: false,
            taken: false,
            answered_at: None,
            ended_reason: None,
        }
    }
}

/// What the media engine was given: nothing, the microphone, or the microphone and the camera.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaGrant {
    None,
    Microphone,
    MicrophoneAndCamera,
}

/// The two tones: somebody calling this device, and the ring the caller hears while it rings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RingTone {
    Incoming,
    Ringback,
}

/// What a call's media runs on — a peer connection, the microphone and camera, and the tones
/// (`ICallMedia`). On Linux `webrtcbin` (the contract is in this module's documentation, "The
/// webrtcbin `CallMedia`"); in a test, a fake. Everything here is one call's worth, and
/// [`close`](Self::close) gives it all back.
///
/// The futures are `'static` — an implementation clones what it needs into them — so the engine
/// can start a step and let it finish on its own.
pub trait CallMedia {
    /// The microphone, and the camera on a video call — asked again for the microphone alone when
    /// the camera is refused or missing, because a camera is never a reason to miss a call
    /// (docs/protocol.md, "Video"). Releases whatever a previous `open` left first.
    fn open(&self, video: bool) -> LocalBoxFuture<'static, MediaGrant>;

    /// A peer connection with these servers and what was opened; `receive_video_only` negotiates
    /// the far side's picture without a camera of our own.
    fn connect(
        &self,
        servers: Vec<IceServerDto>,
        receive_video_only: bool,
    ) -> LocalBoxFuture<'static, bool>;

    /// An offer, set as the local description: its SDP, or `None`.
    fn create_offer(&self) -> LocalBoxFuture<'static, Option<String>>;

    fn set_remote(&self, is_offer: bool, sdp: String) -> LocalBoxFuture<'static, bool>;

    /// An answer to the remote offer, set as the local description: its SDP, or `None`.
    fn create_answer(&self) -> LocalBoxFuture<'static, Option<String>>;

    /// A far-side candidate. Its `sdp_mline_index` is always `Some` — the engine resolves it from
    /// `sdp_mid` against the remote description when the far side sent the mid alone.
    fn add_candidate(&self, candidate: IceCandidate) -> LocalBoxFuture<'static, ()>;

    fn is_connected(&self) -> bool;

    fn set_muted(&self, muted: bool);

    fn set_camera(&self, on: bool);

    fn ring(&self, tone: RingTone);

    fn stop_ringing(&self);

    /// Everything given back: capture stopped — the microphone light goes out — and the
    /// connection closed.
    fn close(&self);

    /// A candidate this device found (`CandidateGathered`).
    fn on_candidate_gathered(&self) -> &Event<IceCandidate>;

    /// The media came up (`Connected`).
    fn on_connected(&self) -> &Event<()>;

    /// The media never came up, or died (`Failed`).
    fn on_failed(&self) -> &Event<()>;

    /// A stream appeared or went: a view attaches it again (`MediaChanged`).
    fn on_media_changed(&self) -> &Event<()>;
}

/// A call's words — web `calls.rs` and `views/call.rs`, ios `CallRecordText` (`CallText`).
pub mod call_text {
    use time::OffsetDateTime;

    use fc_core::text::string_catalog::StringCatalog;

    use super::{CallStage, CallState};

    /// A refused call frame, as a reason to end on.
    pub fn refusal_reason(code: &str) -> &'static str {
        match code {
            "call_busy" | "peer_busy" => "busy",
            "peer_unreachable" => "unreachable",
            "video_calls_disabled" => "video_calls_disabled",
            // Not collapsed into one word: a person's own block is theirs to undo, and a server
            // with no calls is a fact about the server.
            "blocked" => "blocked",
            "calls_disabled" => "calls_disabled",
            _ => "failed",
        }
    }

    /// How ending a call is said for its stage (ios `performHangUp`): answered is a `hangup`, the
    /// caller giving up while it rings is a `cancel`, and the callee saying no is a `decline`.
    pub fn end_reason(call: &CallState) -> &'static str {
        match (call.stage, call.outgoing) {
            (CallStage::Incoming, _) => "decline",
            (CallStage::Dialling | CallStage::Ringing, true) => "cancel",
            (CallStage::Dialling | CallStage::Ringing, false) => "decline",
            _ => "hangup",
        }
    }

    /// What an app on its way out owes the other side: a callee going is a refusal, never a missed
    /// call.
    pub fn unload_reason(call: &CallState) -> &'static str {
        if call.taken {
            "hangup"
        } else if call.outgoing {
            "cancel"
        } else {
            "decline"
        }
    }

    /// Why the call on screen ended, in the apps' words. A call this person ended themselves says
    /// the plain thing.
    ///
    /// The callee's `timeout` is the call record's own line ("Missed voice call"), drawn by
    /// `fc_text::call_record` in the reader's language (`fc_text::i18n::use_lang`, which the shell
    /// sets with the catalogue's), as `win/` drew it with `CallRecordText`.
    pub fn ended_line(reason: &str, call: &CallState, say: &dyn StringCatalog) -> String {
        match reason {
            "decline" if call.outgoing => say.get("Declined").to_string(),
            "timeout" if call.outgoing => say.get("No answer").to_string(),
            "timeout" => fc_text::call_record::label("missed", None, call.video, false),
            "answered_elsewhere" => say.get("Answered on another device").to_string(),
            "busy" => say.get("Busy").to_string(),
            "unreachable" | "unavailable" => say.get("Unavailable").to_string(),
            "blocked" => say.get("You've blocked them.").to_string(),
            "calls_disabled" => say.get("Calls are off on this server.").to_string(),
            "video_calls_disabled" => say.get("Video calls are off on this server.").to_string(),
            "microphone_denied" => say
                .get("Microphone access is needed for calls.")
                .to_string(),
            "failed" => say.get("Call failed").to_string(),
            _ => say.get("Call ended").to_string(),
        }
    }

    /// Where the call is, in words — and once it is up, how long it has been.
    pub fn status_line(call: &CallState, now: OffsetDateTime, say: &dyn StringCatalog) -> String {
        match call.stage {
            CallStage::Dialling => say.get("Calling…").to_string(),
            CallStage::Ringing => say.get("Ringing…").to_string(),
            CallStage::Incoming if call.video => say.get("Incoming video call").to_string(),
            CallStage::Incoming => say.get("Incoming call").to_string(),
            CallStage::Connecting => say.get("Connecting…").to_string(),
            // Whole seconds, cut toward zero, as the C#'s `(long)TotalSeconds`; a clock that went
            // back draws 0:00.
            CallStage::Talking => fc_text::call_record::duration(
                call.answered_at
                    .map_or(0, |answered| (now - answered).whole_seconds()),
            ),
            CallStage::Ended => {
                ended_line(call.ended_reason.as_deref().unwrap_or("hangup"), call, say)
            }
        }
    }
}

/// The few lines of an SDP the engine and the media read — never more than that: the SDP is the
/// media's business, and these answer only where a candidate goes and what an offer carries.
pub mod sdp {
    /// Each m-section: its media kind and its lines, the `m=` line first.
    fn sections(sdp: &str) -> Vec<(&str, Vec<&str>)> {
        let mut sections: Vec<(&str, Vec<&str>)> = Vec::new();
        for line in sdp.lines().map(|line| line.trim_end_matches('\r')) {
            if let Some(media) = line.strip_prefix("m=") {
                let kind = media.split(' ').next().unwrap_or_default();
                sections.push((kind, vec![line]));
            } else if let Some((_, lines)) = sections.last_mut() {
                lines.push(line);
            }
        }
        sections
    }

    fn mid(lines: &[&str]) -> Option<String> {
        lines
            .iter()
            .find_map(|line| line.strip_prefix("a=mid:"))
            .map(|mid| mid.trim().to_string())
    }

    /// The index of the m-line whose `a=mid:` is `mid` — what `webrtcbin` needs of a candidate
    /// that came with its mid alone.
    pub fn mline_of(sdp: &str, mid: &str) -> Option<i32> {
        sections(sdp)
            .iter()
            .position(|(_, lines)| self::mid(lines).as_deref() == Some(mid))
            .and_then(|index| i32::try_from(index).ok())
    }

    /// The `a=mid:` of the m-line at `mline_index` — what a peer may want of a candidate that
    /// came with its index alone.
    pub fn mid_of(sdp: &str, mline_index: i32) -> Option<String> {
        let index = usize::try_from(mline_index).ok()?;
        sections(sdp).get(index).and_then(|(_, lines)| mid(lines))
    }

    /// The payload type the first `media` m-line ("audio", "video") gives `encoding` ("OPUS",
    /// "VP8", "H264"; compared without case) in its `a=rtpmap`, or `None` when it does not carry
    /// it — how the media tells an H.264-only offer, and the offer's own payload numbers.
    pub fn payload_type(sdp: &str, media: &str, encoding: &str) -> Option<u32> {
        let sections = sections(sdp);
        let (_, lines) = sections.iter().find(|(kind, _)| *kind == media)?;
        lines.iter().find_map(|line| {
            let (payload, format) = line.strip_prefix("a=rtpmap:")?.split_once(' ')?;
            let name = format.split('/').next()?;
            if name.eq_ignore_ascii_case(encoding) {
                payload.trim().parse().ok()
            } else {
                None
            }
        })
    }
}

/// `GET /calls/ice`'s answer in `webrtcbin`'s terms: one `stun-server` URI and the
/// `add-turn-server` URIs.
pub mod webrtcbin {
    use std::fmt::Write;

    use fc_core::protocol::IceServerDto;

    /// The ICE servers of one call, as `webrtcbin` takes them.
    #[derive(Debug, Clone, PartialEq, Eq, Default)]
    pub struct IceConfig {
        /// `stun-server`: the FIRST `stun:` URL, as `stun://host:port` — `webrtcbin` takes one.
        pub stun_server: Option<String>,
        /// `add-turn-server`, one each, in the server's order:
        /// `turn(s)://<user>:<credential>@host:port[?transport=…]`, the user and the credential
        /// percent-encoded.
        pub turn_servers: Vec<String>,
    }

    /// What `RTCPeerConnection({iceServers})` would have been given. A `stuns:` URL (`webrtcbin`
    /// has no STUN over TLS), a URL of any other scheme, one with no host, and a TURN URL whose
    /// server gave no username or credential (a relay that cannot authenticate relays nothing;
    /// a browser refuses the whole configuration) are left out. A missing port is the scheme's
    /// default: 3478, or 5349 for `turns` (RFC 7064, RFC 7065).
    pub fn ice_config(servers: &[IceServerDto]) -> IceConfig {
        let mut config = IceConfig::default();
        for server in servers {
            for url in server.urls.iter().flatten() {
                let Some((scheme, rest)) = url.split_once(':') else {
                    continue;
                };
                let scheme = scheme.to_ascii_lowercase();
                let (address, query) = match rest.split_once('?') {
                    Some((address, query)) => (address, Some(query)),
                    None => (rest, None),
                };
                match scheme.as_str() {
                    "stun" => {
                        if config.stun_server.is_none()
                            && let Some(host) = host_and_port(address, 3478)
                        {
                            config.stun_server = Some(format!("stun://{host}"));
                        }
                    }
                    "turn" | "turns" => {
                        let (Some(username), Some(credential)) =
                            (server.username.as_deref(), server.credential.as_deref())
                        else {
                            continue;
                        };
                        let default = if scheme == "turns" { 5349 } else { 3478 };
                        let Some(host) = host_and_port(address, default) else {
                            continue;
                        };
                        let query = query.map_or(String::new(), |query| format!("?{query}"));
                        config.turn_servers.push(format!(
                            "{scheme}://{}:{}@{host}{query}",
                            escape(username),
                            escape(credential)
                        ));
                    }
                    _ => {}
                }
            }
        }
        config
    }

    /// `host:port` from what follows the scheme — a bracketed IPv6 literal kept as it is, the
    /// default port added when there is none.
    fn host_and_port(address: &str, default: u16) -> Option<String> {
        let address = address.trim_start_matches("//");
        let (host, port) = if let Some(bracketed) = address.strip_prefix('[') {
            let (inside, after) = bracketed.split_once(']')?;
            (format!("[{inside}]"), after.strip_prefix(':'))
        } else {
            match address.rsplit_once(':') {
                Some((host, port)) => (host.to_string(), Some(port)),
                None => (address.to_string(), None),
            }
        };
        if host.is_empty() || host == "[]" || host.contains(['@', '/']) {
            return None;
        }
        match port {
            None => Some(format!("{host}:{default}")),
            Some(port) if !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit()) => {
                Some(format!("{host}:{port}"))
            }
            Some(_) => None,
        }
    }

    /// Percent-encoding for a URI's user or password: every byte but RFC 3986's unreserved
    /// characters (`A–Z a–z 0–9 - . _ ~`) as `%XX`. `libgstwebrtcnice` unescapes each half after
    /// splitting at the first `:`, so `1700000000:42` must travel as `1700000000%3A42`.
    pub fn escape(text: &str) -> String {
        let mut escaped = String::with_capacity(text.len());
        for byte in text.bytes() {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
                escaped.push(char::from(byte));
            } else {
                let _ = write!(escaped, "%{byte:02X}");
            }
        }
        escaped
    }
}

/// `GET /calls/ice`, fetched at the start of every call (the C#'s `iceServers` delegate): the
/// app's `ApiClient::ice_servers`, a script in a test.
pub type IceServerFetch = Rc<dyn Fn() -> LocalBoxFuture<'static, ApiResult<IceServersResponse>>>;

/// How the engine starts work that finishes on its own (`_ = SomethingAsync()` in the C#):
/// `glib::MainContext::spawn_local` in the app, a `LocalPool`'s spawner in a test.
pub type Spawn = Rc<dyn Fn(LocalBoxFuture<'static, ()>)>;

/// One call's signalling and state (docs/protocol.md, "Voice calls"; web `calls.rs`, ios
/// `CallManager`): see the module's documentation.
pub struct CallEngine {
    inner: Rc<Inner>,
    attached: RefCell<Option<[Subscription; 4]>>,
}

struct Inner {
    socket: Rc<dyn FrameSender>,
    ice_servers: IceServerFetch,
    media: Rc<dyn CallMedia>,
    clock: Rc<dyn Clock>,
    spawn: Spawn,
    new_call_id: RefCell<Rc<dyn Fn() -> String>>,
    state: RefCell<State>,
    changed: Event<()>,
}

#[derive(Default)]
struct State {
    call: Option<CallState>,
    /// Remote candidates held for the remote description.
    early: Vec<IceCandidate>,
    /// Local candidates held for this device's offer or answer.
    unsent: Vec<IceCandidate>,
    recently_ended: VecDeque<String>,
    offer_sdp: Option<String>,
    remote_set: bool,
    described: bool,
    /// The remote description, for a candidate's m-line index (Linux).
    remote_sdp: Option<String>,
    /// This device's offer or answer, for a candidate's mid (Linux).
    local_sdp: Option<String>,
    guard: Option<Cancel>,
    linger: Option<Cancel>,
}

impl CallEngine {
    /// Backstops for a `call_end` that never came — later than the server's 45 s, so its reason
    /// is normally the one shown. Neither says a word.
    pub const OUTGOING_RING_GUARD: Duration = Duration::from_secs(90);

    pub const INCOMING_RING_GUARD: Duration = Duration::from_secs(60);

    /// An answered call that never came up — this one DOES say `failed`, as nothing on the server
    /// ends it before a minute of silence.
    pub const ANSWER_GUARD: Duration = Duration::from_secs(30);

    /// How long a call that ended stays on screen saying why.
    pub const ENDED_LINGER: Duration = Duration::from_secs(2);

    /// A frame worth a few tries — ten, half a second apart (ios): the socket may be a moment from
    /// opening.
    pub const SAY_EVERY: Duration = Duration::from_millis(500);

    pub const SAY_ATTEMPTS: usize = 10;

    /// How many just-ended calls are remembered, so a replayed offer for one cannot ring again.
    pub const REMEMBERED: usize = 16;

    /// How many candidates are held waiting — the server's own buffer.
    pub const HELD_CANDIDATES: usize = 64;

    /// An engine over this server's socket and `GET /calls/ice`, and the window's media — whose
    /// events it hears from now until [`detach`](Self::detach). Call ids are UUIDs v4.
    pub fn new(
        socket: Rc<dyn FrameSender>,
        ice_servers: IceServerFetch,
        media: Rc<dyn CallMedia>,
        clock: Rc<dyn Clock>,
        spawn: Spawn,
    ) -> Self {
        let inner = Rc::new(Inner {
            socket,
            ice_servers,
            media,
            clock,
            spawn,
            new_call_id: RefCell::new(Rc::new(uuid_v4)),
            state: RefCell::new(State::default()),
            changed: Event::new(),
        });
        let attached = Inner::attach(&inner);
        CallEngine {
            inner,
            attached: RefCell::new(Some(attached)),
        }
    }

    /// The same engine, minting call ids with `ids` (the C#'s `newCallId`) — a test's
    /// `call-1`, `call-2`, ….
    pub fn with_call_ids(self, ids: impl Fn() -> String + 'static) -> Self {
        *self.inner.new_call_id.borrow_mut() = Rc::new(ids);
        self
    }

    /// Done with the media: the window has moved to another server, and the media it keeps is
    /// another engine's now. Nothing the media says afterwards reaches this one. Dropping the
    /// engine does the same.
    pub fn detach(&self) {
        if let Some([gathered, connected, failed, changed]) = self.attached.borrow_mut().take() {
            let media = &self.inner.media;
            media.on_candidate_gathered().unsubscribe(gathered);
            media.on_connected().unsubscribe(connected);
            media.on_failed().unsubscribe(failed);
            media.on_media_changed().unsubscribe(changed);
        }
    }

    /// The call on screen, or `None`.
    pub fn call(&self) -> Option<CallState> {
        self.inner.call()
    }

    /// On a call that is not over: a second one can be neither placed nor rung.
    pub fn busy(&self) -> bool {
        self.inner
            .call()
            .is_some_and(|call| call.stage != CallStage::Ended)
    }

    /// Every change of [`call`](Self::call), and every stream that appeared or went (`Changed`).
    pub fn on_changed(&self) -> &Event<()> {
        &self.inner.changed
    }

    /// One call frame, or a refusal naming a call — what `FrameRouter::on_call` passes on.
    pub fn hear(&self, frame: &ServerFrame) {
        self.inner.hear(frame);
    }

    /// Place a call in a direct chat, to its other member — in the protocol's order: the
    /// microphone first (a call nobody can be heard on is not worth ringing), then the servers,
    /// then the offer the moment it is described. Spawn it; it ends when the offer has gone or the
    /// placing has ended.
    pub fn place(
        &self,
        chat_id: i64,
        peer_user_id: i64,
        video: bool,
    ) -> impl Future<Output = ()> + 'static {
        let inner = self.inner.clone();
        async move { inner.place(chat_id, peer_user_id, video).await }
    }

    /// Answer the call this device is ringing with: the microphone, the servers, the offer in,
    /// the answer out. Spawn it.
    pub fn answer(&self) -> impl Future<Output = ()> + 'static {
        let inner = self.inner.clone();
        async move { inner.answer().await }
    }

    /// Refuse it — there is no `call_decline` frame: a `call_end` that ends it on every one of
    /// their devices.
    pub fn decline(&self) {
        self.inner.hang_up("decline");
    }

    /// End it the way its stage means. A call already over is left alone.
    pub fn end(&self) {
        if let Some(call) = self
            .inner
            .call()
            .filter(|call| call.stage != CallStage::Ended)
        {
            self.inner.hang_up(call_text::end_reason(&call));
        }
    }

    /// Say `call_end` with this reason, and put the call away.
    pub fn hang_up(&self, reason: &str) {
        self.inner.hang_up(reason);
    }

    /// The app is going: the reason its stage means, said once — nothing runs after this to try
    /// again. Await it before the socket goes.
    pub fn leave(&self) -> impl Future<Output = ()> + 'static {
        let inner = self.inner.clone();
        async move { inner.leave().await }
    }

    pub fn toggle_mute(&self) {
        let Some(call) = self.inner.call() else {
            return;
        };
        let muted = !call.muted;
        self.inner.set(CallState { muted, ..call });
        self.inner.media.set_muted(muted);
    }

    /// The camera, off and on: a track disabled, nothing renegotiated — the kind never changes.
    pub fn toggle_camera(&self) {
        let Some(call) = self.inner.call().filter(|call| call.video) else {
            return;
        };
        let on = !call.camera;
        self.inner.set(CallState { camera: on, ..call });
        self.inner.media.set_camera(on);
    }
}

impl Drop for CallEngine {
    fn drop(&mut self) {
        self.detach();
    }
}

impl Inner {
    /// The media's four events, heard through a weak reference: the media outlives engines (one
    /// per server), and must not keep a detached one alive.
    fn attach(inner: &Rc<Inner>) -> [Subscription; 4] {
        let media = &inner.media;
        let weak = Rc::downgrade(inner);
        let gathered = media.on_candidate_gathered().subscribe({
            let weak = weak.clone();
            move |candidate| {
                if let Some(inner) = weak.upgrade() {
                    inner.gathered(candidate);
                }
            }
        });
        let connected = media.on_connected().subscribe({
            let weak = weak.clone();
            move |_| {
                if let Some(inner) = weak.upgrade()
                    && let Some(call) = inner.call()
                {
                    inner.up(&call.call_id);
                }
            }
        });
        // A call that dies is reported by the client, from exactly here: the server deliberately
        // does not end one over a dropped socket (docs/protocol.md, "The sequence").
        let failed = media.on_failed().subscribe({
            let weak = weak.clone();
            move |_| {
                if let Some(inner) = weak.upgrade()
                    && let Some(call) = inner.call().filter(|call| call.stage != CallStage::Ended)
                {
                    inner.finish(&call.call_id, "failed", Some("failed"));
                }
            }
        });
        let changed = media.on_media_changed().subscribe(move |_| {
            if let Some(inner) = weak.upgrade() {
                inner.changed.emit(&());
            }
        });
        [gathered, connected, failed, changed]
    }

    fn call(&self) -> Option<CallState> {
        self.state.borrow().call.clone()
    }

    /// Not in the C#: a mute or the camera turned off while the microphone was being asked for
    /// reached a media with no pipeline to apply it to (`GstCallMedia` builds one in `open`), so the
    /// person saw "muted" and the far side heard them. Said again once the media is connected —
    /// only what differs from a fresh pipeline (a live microphone, the camera on), and the camera
    /// only when there is one.
    fn repeat_toggles(&self, call_id: &str, cameraless: bool) {
        let Some(call) = self.call().filter(|call| call.call_id == call_id) else {
            return;
        };
        if call.muted {
            self.media.set_muted(true);
        }
        if call.video && !cameraless && !call.camera {
            self.media.set_camera(false);
        }
    }

    fn set(&self, call: CallState) {
        self.state.borrow_mut().call = Some(call);
        self.changed.emit(&());
    }

    fn change(&self, call_id: &str, change: impl FnOnce(CallState) -> CallState) {
        if let Some(call) = self.call().filter(|call| call.call_id == call_id) {
            self.set(change(call));
        }
    }

    /// Still ON this call — not merely still saying goodbye to it.
    fn still(&self, call_id: &str) -> bool {
        self.state
            .borrow()
            .call
            .as_ref()
            .is_some_and(|call| call.stage != CallStage::Ended && call.call_id == call_id)
    }

    fn spawn(&self, future: impl Future<Output = ()> + 'static) {
        (self.spawn)(Box::pin(future));
    }

    /// Write a frame once, whether or not it goes (`_ = socket.TrySend(frame)`).
    fn send(&self, frames: Vec<String>) {
        if frames.is_empty() {
            return;
        }
        let socket = self.socket.clone();
        self.spawn(async move {
            // In order, each tried whatever the one before answered.
            for frame in frames {
                let _ = socket.try_send(frame).await;
            }
        });
    }

    fn hear(self: &Rc<Self>, frame: &ServerFrame) {
        match frame {
            ServerFrame::CallOffer {
                call_id,
                chat_id,
                from_user_id,
                sdp,
                video,
            } => self.offered(call_id, *chat_id, *from_user_id, sdp, *video),
            ServerFrame::CallRinging { call_id } => self.ringing(call_id),
            ServerFrame::CallAnswer { call_id, sdp } => self.answered(call_id, sdp),
            ServerFrame::CallIce { call_id, candidate } => self.candidate(call_id, candidate),
            // The server's reason, shown; nothing said back.
            ServerFrame::CallEnd { call_id, reason } => self.finish(call_id, reason, None),
            // It may land AFTER the call_end the server sent first, and then it says better what
            // happened.
            ServerFrame::Error {
                code,
                call_id: Some(call_id),
                ..
            } => self.finish(call_id, call_text::refusal_reason(code), None),
            _ => {}
        }
    }

    async fn place(self: Rc<Self>, chat_id: i64, peer_user_id: i64, video: bool) {
        if self
            .call()
            .is_some_and(|call| call.stage == CallStage::Ended)
        {
            self.clear();
        }
        if self.call().is_some() {
            return;
        }
        let new_call_id = self.new_call_id.borrow().clone();
        let call_id = new_call_id();
        self.set(CallState {
            camera: video,
            ..CallState::new(
                &call_id,
                chat_id,
                peer_user_id,
                true,
                video,
                CallStage::Dialling,
            )
        });
        let grant = self.media.open(video).await;
        if !self.still(&call_id) {
            self.give_back(&call_id);
            return;
        }
        if grant == MediaGrant::None {
            self.finish(&call_id, "microphone_denied", None);
            return;
        }
        let servers = (self.ice_servers)().await;
        if !self.still(&call_id) {
            self.give_back(&call_id);
            return;
        }
        let servers = match servers.outcome {
            Ok(answered) => answered.ice_servers.unwrap_or_default(),
            Err(refusal) => {
                // The server never heard of this call: there is nothing to tell it.
                self.finish(&call_id, call_text::refusal_reason(&refusal.code), None);
                return;
            }
        };
        let cameraless = video && grant != MediaGrant::MicrophoneAndCamera;
        let connected = self.media.connect(servers, cameraless).await;
        if !self.still(&call_id) {
            self.give_back(&call_id);
            return;
        }
        if !connected {
            self.finish(&call_id, "failed", None);
            return;
        }
        if cameraless {
            self.change(&call_id, |call| CallState {
                camera: false,
                ..call
            });
        }
        self.repeat_toggles(&call_id, cameraless);
        let sdp = self.media.create_offer().await;
        if !self.still(&call_id) {
            self.give_back(&call_id);
            return;
        }
        let offered = match sdp {
            Some(sdp) => {
                self.state.borrow_mut().local_sdp = Some(sdp.clone());
                self.socket
                    .try_send(client_frames::call_offer(&call_id, chat_id, &sdp, video))
                    .await
            }
            None => false,
        };
        if !offered {
            // Placed but never offered — a call that visibly did not happen, to the person
            // looking at it.
            self.finish(&call_id, "failed", None);
            return;
        }
        self.described(&call_id);
        self.ring_guard(&call_id, true);
    }

    async fn answer(self: Rc<Self>) {
        let (call, offer) = {
            let state = self.state.borrow();
            match (&state.call, &state.offer_sdp) {
                (Some(call), Some(offer)) if call.stage == CallStage::Incoming => {
                    (call.clone(), offer.clone())
                }
                _ => return,
            }
        };
        let call_id = call.call_id.clone();
        let grant = self.media.open(call.video).await;
        if !self.still(&call_id) {
            self.give_back(&call_id);
            return;
        }
        if grant == MediaGrant::None {
            // A refusal on the wire, and the reason on screen is the one that can be acted on:
            // leaving the caller ringing at somebody who cannot speak is worse.
            self.finish(&call_id, "microphone_denied", Some("decline"));
            return;
        }
        let servers = (self.ice_servers)().await;
        if !self.still(&call_id) {
            self.give_back(&call_id);
            return;
        }
        let list = match servers.outcome {
            Ok(answered) => answered.ice_servers.unwrap_or_default(),
            // A refusal the server MEANS is the end of it. One it merely could not give leaves
            // the candidates this device finds on its own, which are enough on one network.
            Err(refusal) if !refusal.transient() => {
                self.finish(
                    &call_id,
                    call_text::refusal_reason(&refusal.code),
                    Some("decline"),
                );
                return;
            }
            Err(_) => Vec::new(),
        };
        let cameraless = call.video && grant != MediaGrant::MicrophoneAndCamera;
        let connected = self.media.connect(list, cameraless).await;
        if !self.still(&call_id) {
            self.give_back(&call_id);
            return;
        }
        if !connected {
            self.hang_up("failed");
            return;
        }
        if cameraless {
            self.change(&call_id, |held| CallState {
                camera: false,
                ..held
            });
        }
        self.repeat_toggles(&call_id, cameraless);
        let offer_in = self.media.set_remote(true, offer.clone()).await;
        if !self.still(&call_id) {
            return;
        }
        if !offer_in {
            self.hang_up("failed");
            return;
        }
        self.remote_is_set(offer).await;
        let sdp = self.media.create_answer().await;
        if !self.still(&call_id) {
            return;
        }
        let Some(sdp) = sdp else {
            self.hang_up("failed");
            return;
        };
        self.state.borrow_mut().local_sdp = Some(sdp.clone());
        // The answer is what takes the call, so it is worth a few tries.
        self.send_answer(&call_id, sdp);
        self.media.stop_ringing();
        self.change(&call_id, |held| CallState {
            stage: CallStage::Connecting,
            taken: true,
            ..held
        });
        self.arm_answer_guard(&call_id);
        self.catch_up(&call_id);
    }

    fn hang_up(self: &Rc<Self>, reason: &str) {
        if let Some(call) = self.call() {
            self.finish(&call.call_id, reason, Some(reason));
        }
    }

    async fn leave(self: Rc<Self>) {
        let Some(call) = self.call().filter(|call| call.stage != CallStage::Ended) else {
            return;
        };
        let _ = self
            .socket
            .try_send(client_frames::call_end(
                &call.call_id,
                call_text::unload_reason(&call),
            ))
            .await;
        self.teardown();
    }

    // ---- frames ----------------------------------------------------------------------------

    /// Somebody is calling. Every connection the callee has gets this, so a second copy of one
    /// held is the duplicate it is; and an offer for a call that just ended here crossed its own
    /// `call_end`.
    fn offered(
        self: &Rc<Self>,
        call_id: &str,
        chat_id: i64,
        from_user_id: i64,
        sdp: &str,
        video: bool,
    ) {
        if self
            .state
            .borrow()
            .recently_ended
            .iter()
            .any(|ended| ended == call_id)
        {
            return;
        }
        // A call still saying why it ended is not a call: the next one takes its place at once.
        if self
            .call()
            .is_some_and(|call| call.stage == CallStage::Ended)
        {
            self.clear();
        }
        // Already ringing with it, or on a call: their other devices may still take this one, and
        // ending it here would end it for all of them.
        if self.call().is_some() {
            return;
        }
        self.teardown();
        self.state.borrow_mut().offer_sdp = Some(sdp.to_string());
        self.media.ring(RingTone::Incoming);
        self.set(CallState {
            camera: video,
            ..CallState::new(
                call_id,
                chat_id,
                from_user_id,
                false,
                video,
                CallStage::Incoming,
            )
        });
        self.ring_guard(call_id, false);
    }

    /// The server reached them: it rings on their side now, which is when the caller hears the
    /// tone.
    fn ringing(&self, call_id: &str) {
        if let Some(call) = self
            .call()
            .filter(|call| call.stage == CallStage::Dialling && call.call_id == call_id)
        {
            self.set(CallState {
                stage: CallStage::Ringing,
                ..call
            });
            self.media.ring(RingTone::Ringback);
        }
    }

    /// They took it, on one of their devices.
    fn answered(self: &Rc<Self>, call_id: &str, sdp: &str) {
        let applies = self.call().is_some_and(|call| {
            call.outgoing
                && !call.taken
                && call.stage != CallStage::Ended
                && call.call_id == call_id
        });
        if !applies {
            return;
        }
        let setting = self.media.set_remote(false, sdp.to_string());
        let (inner, call_id, sdp) = (self.clone(), call_id.to_string(), sdp.to_string());
        self.spawn(async move {
            let set = setting.await;
            if !inner.still(&call_id) {
                return;
            }
            if !set {
                inner.finish(&call_id, "failed", Some("failed"));
                return;
            }
            inner.remote_is_set(sdp).await;
            inner.media.stop_ringing();
            inner.change(&call_id, |held| CallState {
                stage: CallStage::Connecting,
                taken: true,
                ..held
            });
            // The caller owes itself the callee's guard: nothing ends an answered call that never
            // comes up for a minute.
            inner.arm_answer_guard(&call_id);
            inner.catch_up(&call_id);
        });
    }

    /// A relayed candidate: in now if the remote description is, and held until it is if not —
    /// for a call still on (Linux; the C# held one for a call already over, see the module's
    /// documentation).
    fn candidate(&self, call_id: &str, candidate: &IceCandidate) {
        if !self.still(call_id) {
            return;
        }
        if self.state.borrow().remote_set {
            if let Some(candidate) = self.for_media(candidate.clone()) {
                let adding = self.media.add_candidate(candidate);
                self.spawn(adding);
            }
            return;
        }
        let mut state = self.state.borrow_mut();
        if state.early.len() < CallEngine::HELD_CANDIDATES {
            state.early.push(candidate.clone());
        }
    }

    // ---- the call's own machinery ----------------------------------------------------------

    /// The one way a call ends here: the media given back, the reason left on screen for a moment,
    /// and the call remembered just long enough that a frame still in flight for it cannot start
    /// it again. `say` is for the SERVER — only the protocol's four reasons — and `None` when the
    /// server said so first or never heard of the call.
    fn finish(self: &Rc<Self>, call_id: &str, shown: &str, say: Option<&str>) {
        let Some(call) = self.call().filter(|call| call.call_id == call_id) else {
            return;
        };
        if let Some(reason) = say {
            self.say_retrying(client_frames::call_end(call_id, reason));
        }
        // Over BEFORE the media is given back: a media that reports its own close as a failure
        // then finds nothing to end.
        self.state.borrow_mut().call = Some(CallState {
            stage: CallStage::Ended,
            ended_reason: Some(shown.to_string()),
            ..call
        });
        self.teardown();
        {
            let mut state = self.state.borrow_mut();
            // Once, though `call_end` and the `error` that refines it end it twice.
            if !state.recently_ended.iter().any(|ended| ended == call_id) {
                state.recently_ended.push_back(call_id.to_string());
            }
            while state.recently_ended.len() > CallEngine::REMEMBERED {
                state.recently_ended.pop_front();
            }
        }
        self.changed.emit(&());
        self.linger(call_id);
    }

    fn teardown(&self) {
        let guard = {
            let mut state = self.state.borrow_mut();
            state.early.clear();
            state.unsent.clear();
            state.offer_sdp = None;
            state.remote_set = false;
            state.described = false;
            state.remote_sdp = None;
            state.local_sdp = None;
            state.guard.take()
        };
        if let Some(guard) = guard {
            guard.cancel();
        }
        self.media.stop_ringing();
        self.media.close();
    }

    fn clear(&self) {
        let linger = {
            let mut state = self.state.borrow_mut();
            state.call = None;
            state.linger.take()
        };
        if let Some(linger) = linger {
            linger.cancel();
        }
    }

    /// A setup that is no longer wanted gives back what it opened — unless another call holds the
    /// media by now (Linux; the C# closed it under that call, see the module's documentation).
    fn give_back(&self, call_id: &str) {
        let another = self
            .state
            .borrow()
            .call
            .as_ref()
            .is_some_and(|call| call.call_id != call_id && call.stage != CallStage::Ended);
        if !another {
            self.media.close();
        }
    }

    /// A candidate this device found: after its offer or answer, or held until that has gone.
    fn gathered(&self, candidate: &IceCandidate) {
        let Some(call) = self.call().filter(|call| call.stage != CallStage::Ended) else {
            return;
        };
        {
            let mut state = self.state.borrow_mut();
            if !state.described {
                if state.unsent.len() < CallEngine::HELD_CANDIDATES {
                    state.unsent.push(candidate.clone());
                }
                return;
            }
        }
        let candidate = self.for_wire(candidate.clone());
        self.send(vec![client_frames::call_ice(&call.call_id, &candidate)]);
    }

    /// This device's offer or answer has gone: what was found before it follows it now, in order.
    fn described(&self, call_id: &str) {
        let unsent = {
            let mut state = self.state.borrow_mut();
            state.described = true;
            std::mem::take(&mut state.unsent)
        };
        let frames = unsent
            .into_iter()
            .map(|candidate| client_frames::call_ice(call_id, &self.for_wire(candidate)))
            .collect();
        self.send(frames);
    }

    fn send_answer(self: &Rc<Self>, call_id: &str, sdp: String) {
        let (inner, call_id) = (self.clone(), call_id.to_string());
        self.spawn(async move {
            let frame = client_frames::call_answer(&call_id, &sdp);
            if say_retrying(&*inner.socket, &*inner.clock, frame).await && inner.still(&call_id) {
                inner.described(&call_id);
            }
        });
    }

    /// A `call_end` worth a few tries, started and left to finish.
    fn say_retrying(&self, frame: String) {
        let (socket, clock) = (self.socket.clone(), self.clock.clone());
        self.spawn(async move {
            say_retrying(&*socket, &*clock, frame).await;
        });
    }

    /// The remote description is in: everything held back goes in now, in the order it came.
    async fn remote_is_set(&self, sdp: String) {
        let held = {
            let mut state = self.state.borrow_mut();
            state.remote_set = true;
            state.remote_sdp = Some(sdp);
            std::mem::take(&mut state.early)
        };
        for candidate in held {
            if let Some(candidate) = self.for_media(candidate) {
                self.media.add_candidate(candidate).await;
            }
        }
    }

    /// A far-side candidate as the media takes it — with its m-line index, which `webrtcbin`
    /// needs, resolved from its mid when it came without one; `None` when it names neither, or a
    /// mid the remote description lacks (Linux).
    fn for_media(&self, mut candidate: IceCandidate) -> Option<IceCandidate> {
        if candidate.sdp_mline_index.is_some_and(|index| index >= 0) {
            return Some(candidate);
        }
        let index = {
            let state = self.state.borrow();
            let (mid, sdp) = candidate
                .sdp_mid
                .as_deref()
                .zip(state.remote_sdp.as_deref())?;
            sdp::mline_of(sdp, mid)?
        };
        candidate.sdp_mline_index = Some(index);
        Some(candidate)
    }

    /// One of this device's candidates as the wire carries it — both halves, the missing one read
    /// from this device's own offer or answer (Linux: `webrtcbin` gives the index alone).
    fn for_wire(&self, mut candidate: IceCandidate) -> IceCandidate {
        let state = self.state.borrow();
        if let Some(sdp) = state.local_sdp.as_deref() {
            if candidate.sdp_mid.is_none()
                && let Some(index) = candidate.sdp_mline_index
            {
                candidate.sdp_mid = sdp::mid_of(sdp, index);
            }
            if candidate.sdp_mline_index.is_none()
                && let Some(mid) = candidate.sdp_mid.as_deref()
            {
                candidate.sdp_mline_index = sdp::mline_of(sdp, mid);
            }
        }
        candidate
    }

    /// The media is up: the call is a conversation, and the clock starts here rather than at the
    /// answer.
    fn up(&self, call_id: &str) {
        if let Some(call) = self
            .call()
            .filter(|call| call.stage == CallStage::Connecting && call.call_id == call_id)
        {
            self.set(CallState {
                stage: CallStage::Talking,
                answered_at: Some(self.clock.now()),
                ..call
            });
        }
    }

    /// The media may already be up — its event can land while held candidates were going in,
    /// before there was a Connecting stage to promote.
    fn catch_up(&self, call_id: &str) {
        if self.media.is_connected() {
            self.up(call_id);
        }
    }

    fn ring_guard(self: &Rc<Self>, call_id: &str, outgoing: bool) {
        let after = if outgoing {
            CallEngine::OUTGOING_RING_GUARD
        } else {
            CallEngine::INCOMING_RING_GUARD
        };
        let call_id = call_id.to_string();
        self.arm(after, move |inner| {
            if inner
                .call()
                .is_some_and(|call| !call.taken && call.call_id == call_id)
            {
                inner.finish(&call_id, "timeout", None);
            }
        });
    }

    fn arm_answer_guard(self: &Rc<Self>, call_id: &str) {
        let call_id = call_id.to_string();
        self.arm(CallEngine::ANSWER_GUARD, move |inner| {
            if inner
                .call()
                .is_some_and(|call| call.stage == CallStage::Connecting && call.call_id == call_id)
            {
                inner.finish(&call_id, "failed", Some("failed"));
            }
        });
    }

    fn arm(self: &Rc<Self>, after: Duration, then: impl FnOnce(&Rc<Inner>) + 'static) {
        let armed = Cancel::new();
        let previous = self.state.borrow_mut().guard.replace(armed.clone());
        if let Some(previous) = previous {
            previous.cancel();
        }
        self.wait(after, armed, then);
    }

    /// The panel goes after a moment, unless another call has started meanwhile.
    fn linger(self: &Rc<Self>, call_id: &str) {
        let waiting = Cancel::new();
        let previous = self.state.borrow_mut().linger.replace(waiting.clone());
        if let Some(previous) = previous {
            previous.cancel();
        }
        let call_id = call_id.to_string();
        self.wait(CallEngine::ENDED_LINGER, waiting, move |inner| {
            let gone = {
                let mut state = inner.state.borrow_mut();
                let over = state
                    .call
                    .as_ref()
                    .is_some_and(|call| call.stage == CallStage::Ended && call.call_id == call_id);
                if over {
                    state.call = None;
                }
                over
            };
            if gone {
                inner.changed.emit(&());
            }
        });
    }

    /// `then`, after `after` — unless `cancel` is raised first, or the engine has gone. The wait
    /// is asked of the clock at once, as the C# asked for its delay.
    fn wait(
        self: &Rc<Self>,
        after: Duration,
        cancel: Cancel,
        then: impl FnOnce(&Rc<Inner>) + 'static,
    ) {
        let sleeping = self.clock.sleep(after);
        let engine: Weak<Inner> = Rc::downgrade(self);
        self.spawn(async move {
            select(sleeping, cancel.cancelled()).await;
            if cancel.is_cancelled() {
                return;
            }
            if let Some(inner) = engine.upgrade() {
                then(&inner);
            }
        });
    }
}

/// A frame worth a few tries: once, then up to [`CallEngine::SAY_ATTEMPTS`] more,
/// [`CallEngine::SAY_EVERY`] apart — whether it went.
async fn say_retrying(socket: &dyn FrameSender, clock: &dyn Clock, frame: String) -> bool {
    if socket.try_send(frame.clone()).await {
        return true;
    }
    for _ in 0..CallEngine::SAY_ATTEMPTS {
        clock.sleep(CallEngine::SAY_EVERY).await;
        if socket.try_send(frame.clone()).await {
            return true;
        }
    }
    false
}
