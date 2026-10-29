//! Calls (shell-design §14, package P6): the media — [`GstCallMedia`], fc-logic's `CallMedia` on
//! GStreamer's `webrtcbin`, and the ring tones — and the shell around it: the Call window, the
//! ring's notice, the call routes, the main window's Return to Call, keeping awake in a call, and
//! the call buttons' function. The call's signalling and state are fc-logic's
//! [`CallEngine`](fc_logic::calls::CallEngine); its contract for this media is the
//! `fc_logic::calls` documentation, "The webrtcbin `CallMedia`".
//!
//! # The shell's half
//!
//! [`install`] (one line in `app::install_packages`) puts ONE [`GstCallMedia`] into
//! `Shell::call_media` — [`CallDevices::from_env`] (PipeWire, unless a self-test switch is set) and
//! a [`CallCamera`] over P3's `Shell::camera` (`camera::ShellCamera`: the `video-camera` setting
//! picks the device, a refusal is a camera-off call) — kept across calls and across servers; every
//! connection's `CallEngine` runs on it. It installs P6's `CallRoutes` (`routes`), the main
//! window's Return to Call, the window's CSS, and the part that follows the engine (`hub`, which
//! also keeps the machine awake from the answer to the end through P5's `desktop::keep_awake`):
//!
//! | Module | What |
//! | --- | --- |
//! | `call_view` | what the window shows, when it comes forward, when a call rings on the desktop and keeps the machine awake — pure, unit-tested |
//! | `call_window` | the Call window (§14.3): the far side's picture or the avatar, the self-view, the status line, the discs |
//! | `call_notice` | the ring told to the desktop (§14.4): P5's `Notifier::ring` / `stop_ringing` (which also mark the icon and the dock) |
//! | `camera` | P3's camera as the media's `CallCamera` |
//! | `routes` | `app.answer-call` / `app.decline-call` (that call only) / `app.return-to-call`, and Return to Call |
//! | `hub` | the one part following the current connection's engine |
//!
//! **The call buttons** are P1's (the conversation header's Call and Video Call, a record's "Call
//! back"); they call [`place`], are shown by [`offered`] and made sensitive by [`can_place`],
//! re-asked on every [`on_changed`].
//!
//! **`tools/call-interop` compiles the media's four files** (`ice`, `media`, `pipeline`, `tones`)
//! as they are, through a module of its own pointed at this folder — never this file, whose shell
//! half needs the app.
//!
//! # The media: how the shell uses it
//!
//! ONE media per window, kept across calls and across servers; one engine per signed-in server
//! over it (`fc_logic::calls`' "How the shell uses it"):
//!
//! ```text
//! let media = Rc::new(GstCallMedia::new(CallDevices::from_env(), camera));   // camera: P3's
//! let engine = CallEngine::new(socket, ice_servers, media.clone(), clock, spawn);
//! engine.on_changed().subscribe(…);          // the Call window redraws from engine.call() and,
//!                                            // for pictures, from the two paintables below
//! ```
//!
//! - [`CallDevices::from_env`] is PipeWire through `pulsesrc`/`pulsesink` and `gtk4paintablesink`
//!   unless a self-test switch is set (`FC_TEST_AUDIO_SOURCE`, `FC_TEST_AUDIO_SINK`,
//!   `FC_TEST_VIDEO_SOURCE`, `FC_TEST_VIDEO_SINK` — shell-design §27).
//! - `camera` is an `Rc<dyn` [`CallCamera`]`>`: P3's `media::camera` (the Camera portal, then
//!   `pipewiresrc` on its fd) wrapped to hand over a [`CameraFeed`] — the source element and
//!   whatever keeps the portal session open (`camera::ShellCamera`). `NoCamera` is the harness's: every
//!   video call is then camera-off, receiving the far side's picture.
//! - **Pictures for the Call window**: [`GstCallMedia::remote_video`] (the far side, a
//!   `gdk::Paintable` for a `gtk::Picture` with `ContentFit::Contain`) and
//!   [`GstCallMedia::self_view`] (this camera, already mirrored). Read them on every
//!   `engine.on_changed()` — the engine passes the media's `on_media_changed` on — and on
//!   `None` show the avatar. Both are `None` after the call ends.
//! - [`GstCallMedia::stats`], [`GstCallMedia::ringing`], [`GstCallMedia::pipeline`] and
//!   [`GstCallMedia::agent_facts`] are for the self-test and the interop harness
//!   (`tools/call-interop`).
//!
//! # Deviations from the contract, each for a reason measured or stated here
//!
//! 1. **`connect` leaves the pipeline `PAUSED`; it plays when the send branches are linked** — in
//!    `create_offer` (Opus 111 / VP8 96) or in `set_remote` of an offer (the offer's own payload
//!    numbers, and its codec: VP8, else H.264). An answer must use the offer's numbers: answering
//!    Firefox's Opus 109 / VP8 120 offer with tails linked as 111 / 96, `webrtcbin` answered BOTH
//!    m-lines `recvonly` and Firefox received nothing — a silent one-way call (measured). And a
//!    live source playing into an unlinked tail stops with `not-linked`. The engine sees no
//!    difference: nothing happens on `webrtcbin` between `connect` and those calls.
//! 2. **A far-side stream is linked on the main context**, not in `pad-added`: the handler
//!    blocks the new pad and posts it, so `gtk4paintablesink` is made on the main thread (it
//!    insists, and blocking a streaming thread on the main one deadlocks a teardown) and no
//!    element is added to a pipeline that is going to `NULL`.
//! 3. **The camera picture is 640 × 480 with SQUARE pixels** (`pixel-aspect-ratio=1/1`): a 16:9
//!    camera is letterboxed rather than squashed, since VP8 carries no aspect ratio.
//! 4. **An offer's video m-line this device has no camera for gets a `recvonly` transceiver in
//!    the ONE codec this side chose** — the contract's camera-off video call, and also a VOICE
//!    call whose offer carries a video m-line (the web client's old shape: answered `inactive`).
//!    Left alone, `webrtcbin` answers such an m-line `recvonly` with EVERY codec the offer listed
//!    (measured: VP8, AV1 and VP9 for Chromium), and the far side may then send one this pipeline
//!    has no decoder for.
//! 5. **The dead-peer fallback is conditional** ([`ice`]'s documentation): `webrtcbin`'s libnice
//!    agent runs RFC 7675 consent freshness (measured), so a vanished peer reaches ICE `failed` in
//!    about 30 s and the contract's "no far-side audio for 30 s" rule is armed only for an agent
//!    without it ([`ice::SilenceRule`]).
//! 6. **Two knobs the contract does not have**, both defaulting to it:
//!    [`GstCallMedia::with_silence_rule`] (`Always` proves the fallback) and
//!    [`GstCallMedia::with_offered_video`] (H.264 offers — no Playwright browser here has H.264,
//!    so the H.264 answer is proven against a Linux peer offering it).

pub mod ice;
mod media;
pub mod pipeline;
pub mod tones;

mod call_notice;
pub mod call_view;
pub mod call_window;
mod camera;
pub mod hub;
mod routes;

pub use media::{CallCamera, CallDevices, CameraFeed, GstCallMedia};

use std::rc::Rc;

use fc_core::rt::Event;

use crate::shell::Shell;

/// `app::install_packages`' line for P6: the media, the routes, the window part, the CSS, the hub.
pub fn install(shell: &Rc<Shell>) {
    let media = Rc::new(GstCallMedia::new(
        CallDevices::from_env(),
        Rc::new(camera::ShellCamera),
    ));
    shell.call_media.install(media.clone());
    shell.call_routes.install(Rc::new(routes::WindowCallRoutes));
    shell.add_window_part(Rc::new(routes::ReturnToCall));
    crate::theme::register(call_window::css);
    hub::install(shell, media);
}

/// **P1's call buttons**: Call / Video Call in a direct chat's header, and a call record's "Call
/// back" (`fc_logic::call_records::call_back` says which kind) — place a call to `peer_user_id` in
/// `chat_id`. The Call window comes up by itself as the call starts. Does nothing while a call is
/// under way (the buttons are insensitive then: [`can_place`]) or with no server.
pub fn place(shell: &Rc<Shell>, chat_id: i64, peer_user_id: i64, video: bool) {
    let Some(connection) = shell.connection() else {
        return;
    };
    if connection.calls.busy() {
        crate::fc_info!("call: not placed — a call is under way");
        hub::present();
        return;
    }
    crate::fc_info!("call: placing one in chat {chat_id} (video {video})");
    (shell.spawn)(Box::pin(connection.calls.place(
        chat_id,
        peer_user_id,
        video,
    )));
}

/// Whether the buttons are SHOWN for a direct chat with `peer_user_id` (the Mac's `canCall`): the
/// server carries calls (`calls_enabled`; a Video Call also `video_calls_enabled`), and the reader
/// has not blocked them (the server would refuse with `blocked`). P1 shows them in direct chats
/// only (`fc_logic::call_records::call_back`'s `direct_chat`).
pub fn offered(shell: &Shell, peer_user_id: i64, video: bool) -> bool {
    let Some(connection) = shell.connection() else {
        return false;
    };
    let state = connection.session.state();
    let blocked = connection.chats.is_blocked(peer_user_id).unwrap_or(false);
    state.calls_enabled && (!video || state.video_calls_enabled) && !blocked
}

/// Whether the buttons are SENSITIVE now: no call under way (a call still saying why it ended
/// does not count) and nothing recording — `recording` is P3's answer
/// (`fc_logic::call_records::can_place_call`: a call over a recording has no right answer).
pub fn can_place(shell: &Shell, recording: bool) -> bool {
    let busy = shell
        .connection()
        .is_some_and(|connection| connection.calls.busy());
    fc_logic::call_records::can_place_call(busy, recording)
}

/// Raised whenever a call begins, moves on or ends, and when the connection changes — when
/// [`can_place`] may answer differently.
pub fn on_changed() -> Rc<Event<()>> {
    hub::get().map_or_else(|| Rc::new(Event::new()), |calls| calls.changed())
}

/// The self-test's media: the same window media over `devices` (its test sources) and the shell's
/// camera, installed in `Shell::call_media` and followed by the hub. Call it before the server is
/// opened: a connection's engine keeps the media it was made with.
#[cfg(feature = "selftest")]
pub fn use_devices(shell: &Rc<Shell>, devices: CallDevices) -> Rc<GstCallMedia> {
    let media = Rc::new(GstCallMedia::new(devices, Rc::new(camera::ShellCamera)));
    shell.call_media.install(media.clone());
    if let Some(calls) = hub::get() {
        calls.use_media(media.clone());
    }
    media
}
