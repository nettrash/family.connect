//! The `calls` group (P6, shell-design §27): TWO instances of the app calling each other — this
//! one is Alice and directs the run; the second, Bob, is started from here
//! (`family-connect --selftest OUT/bob --only calls`, its own profile under `OUT/bob/home`, the
//! media's test switches `FC_TEST_AUDIO_SOURCE`/`_SINK`/`FC_TEST_VIDEO_SOURCE` in its environment)
//! and does what it is told.
//!
//! **Two fake servers, one relay, here.** Each person gets a server of their own, both in THIS
//! process (Alice's is the run's `Harness::fake_server`, Bob's a second `FakeServer::start`), with
//! the same people seeded in each. The fake's sockets now know their person (`send_frame_to`), but
//! its `GET /chats` answers per FAMILY, and the direct chat the two share must name the OTHER
//! person to each of them (`peer_user_id`, the title) under the same id — which two worlds give for
//! free. Both say they carry calls (`FakeState { calls, video_calls }`, read by S0's `/me`). The
//! relay below reads what each client sent (`frames_received`) and does with every call frame what the real server does
//! (docs/protocol.md, "Voice calls" — `tools/call-interop/src/relay.rs`'s rules): `call_ringing`
//! back to the caller and the offer, with `from_user_id`, to the callee; the answer, the
//! candidates and `call_end` to the other side. Bob's server living here is also what lets his
//! QUIT be heard: the `call_end` his app says on the way out lands in a server that outlives him.
//!
//! **Bob is a puppet.** His instance signs in to his server and then, every 50 ms, posts what he
//! sees (`Probe`: the engine's call, the Call window's widgets, the media's counts, what his
//! desktop stand-ins were told) to `POST /api/v1/selftest/state` on it, and asks
//! `GET /api/v1/selftest/next` for what to do — a call placed, an action of a notification
//! (`app.answer-call`, `app.decline-call`), a disc pressed, the window closed, Quit. Every check is
//! made here, from both sides' probes. Bob's own report is never written: he leaves by Quit.
//!
//! **What it proves**: a voice call placed with `calls::place` rings back on Alice's side and
//! rings on Bob's — the window, Decline/Accept with Accept focused, the notice through the Notifier
//! seam (the stand-in records `ring`), the top-bar icon and the dock; a stale Answer does nothing;
//! Answer from the notification's action reaches Talking on both with RTP flowing and the notice
//! withdrawn; the duration counts; the machine is kept awake; Escape never hangs up; Mute shows
//! and is heard as silence while RTP still flows, Unmute is heard again; a hang-up on either side
//! ends both and the window goes by itself; a Decline from the notification reaches the caller as
//! "Declined"; a video call shows the far side's picture and the self-view on both sides, from the
//! camera the `video-camera` setting names; Camera off hides the self-view and stops the far
//! side's stream, Camera on resumes it; closing the window keeps the call, Return to Call brings
//! it back; Escape declines a ring; Quit with a call up ends the process within the limit and the
//! far side hears the hang-up. Screenshots of each window in each state, narrow and dark, are
//! `OUT/screenshots/calls-*.png`.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use fc_core::protocol::{ChatDto, ChatRowDto, ServerUrl, TokenStore};
use fc_core::rt::{Event, Http, HttpRequest, LocalBoxFuture, Method};
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::calls::CallStage;
use fc_logic::live_connection::Link;
use fc_logic::notifications::Toast;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::Harness;
use super::fake_server::{FakeResponse, FakeServer};
use crate::calls::call_view::Role;
use crate::calls::{self, CallDevices, hub};
use crate::seams::actions;
use crate::seams::desktop::{BadgeState, Notifier, TrayState};
use crate::seams::media::{Camera, CameraDevice, CameraRefusal};
use crate::services::settings_store::keys;
use crate::shell::Shell;
use crate::windows::main_window::GatePage;

/// Set in Bob's environment: his server's address. Its presence makes an instance the puppet.
const PUPPET: &str = "FC_SELFTEST_CALLS_PUPPET";
/// The direct chat the two share, the same id in both servers.
const DIRECT_CHAT: i64 = 9001;
/// The media's test sources (the interop harness's: ticks survive the echo canceller's noise
/// suppression, a tone would not).
const AUDIO_SOURCE: &str = "audiotestsrc is-live=true wave=ticks";
const AUDIO_SINK: &str = "fakesink sync=true";
/// Alice's camera sends MJPEG, as many a `v4l2src` does: the call's camera path decodes it.
const ALICE_CAMERA: &str = "videotestsrc is-live=true pattern=ball \
    ! video/x-raw,width=640,height=480,framerate=30/1 ! jpegenc";
const BOB_CAMERA: &str = "videotestsrc is-live=true pattern=smpte";
/// What the `video-camera` setting names on Alice's side.
const CAMERA_ID: &str = "selftest-camera";

const WAIT: Duration = Duration::from_secs(15);
const CALL_UP: Duration = Duration::from_secs(25);
const LINGER: Duration = Duration::from_secs(6);
/// `Shell::quit`'s two seconds for the hang-up, and the rest of the way out.
const QUIT_LIMIT: Duration = Duration::from_secs(6);

pub async fn run(h: &Harness) {
    match std::env::var(PUPPET) {
        Ok(server) if !server.is_empty() => puppet(h, &server).await,
        _ => director(h).await,
    }
}

// ---- what each side sees ------------------------------------------------------------------

/// A side's media counts (`GstCallMedia::stats`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Counts {
    audio_rtp_in: u64,
    video_rtp_in: u64,
    audio_rtp_out: u64,
    video_rtp_out: u64,
    video_decoded: u64,
    audio_peak: u32,
}

/// What one side sees at one moment.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Probe {
    ready: bool,
    stage: Option<String>,
    call_id: Option<String>,
    video: bool,
    muted: bool,
    camera: bool,
    ended_reason: Option<String>,
    window_visible: bool,
    title: String,
    name: String,
    status: String,
    discs: Vec<String>,
    focused: Option<String>,
    picture: bool,
    self_view: bool,
    identity: bool,
    note: bool,
    dark: bool,
    tone: Option<String>,
    counts: Counts,
    rings: Vec<(String, String, String)>,
    stops: Vec<String>,
    tray_ringing: bool,
    urgent: bool,
    awake: bool,
    return_visible: bool,
}

impl Probe {
    fn at(&self, stage: CallStage) -> bool {
        self.stage.as_deref() == Some(format!("{stage:?}").as_str())
    }

    fn idle(&self) -> bool {
        self.stage.is_none() && !self.window_visible
    }
}

fn probe(shell: &Shell, desk: &Desk) -> Probe {
    let connection = shell.connection();
    let call = connection
        .as_ref()
        .and_then(|connection| connection.calls.call());
    let ready = connection.as_ref().is_some_and(|connection| {
        connection.session.state().can_chat() && connection.live.link() == Link::Up
    });
    let calls = hub::get();
    let window = calls.as_ref().and_then(|calls| calls.window());
    let media = calls.as_ref().map(|calls| calls.media());
    let counts = media.as_ref().map_or(Counts::default(), |media| {
        let stats = media.stats();
        Counts {
            audio_rtp_in: stats.audio_rtp_in,
            video_rtp_in: stats.video_rtp_in,
            audio_rtp_out: stats.audio_rtp_out,
            video_rtp_out: stats.video_rtp_out,
            video_decoded: stats.video_decoded,
            audio_peak: stats.audio_peak,
        }
    });
    Probe {
        ready,
        stage: call.as_ref().map(|call| format!("{:?}", call.stage)),
        call_id: call.as_ref().map(|call| call.call_id.clone()),
        video: call.as_ref().is_some_and(|call| call.video),
        muted: call.as_ref().is_some_and(|call| call.muted),
        camera: call.as_ref().is_some_and(|call| call.camera),
        ended_reason: call.as_ref().and_then(|call| call.ended_reason.clone()),
        window_visible: window.as_ref().is_some_and(|window| window.is_visible()),
        title: window
            .as_ref()
            .and_then(|window| window.window().title())
            .map(|title| title.to_string())
            .unwrap_or_default(),
        name: window
            .as_ref()
            .map(|window| window.name())
            .unwrap_or_default(),
        status: window
            .as_ref()
            .map(|window| window.status())
            .unwrap_or_default(),
        discs: window
            .as_ref()
            .map(|window| window.shown_discs())
            .unwrap_or_default(),
        focused: window.as_ref().and_then(|window| window.focused_disc()),
        picture: window.as_ref().is_some_and(|window| window.picture_shown()),
        self_view: window
            .as_ref()
            .is_some_and(|window| window.self_view_shown()),
        identity: window
            .as_ref()
            .is_some_and(|window| window.identity_shown()),
        note: window.as_ref().is_some_and(|window| window.note_shown()),
        dark: window.as_ref().is_some_and(|window| window.is_dark()),
        tone: media
            .as_ref()
            .and_then(|media| media.ringing())
            .map(|tone| format!("{tone:?}")),
        counts,
        rings: desk.rings.borrow().clone(),
        stops: desk.stops.borrow().clone(),
        tray_ringing: desk.tray_ringing.get(),
        urgent: desk.urgent.get(),
        awake: calls.as_ref().is_some_and(|calls| calls.awake()),
        // Its own flag: the main window may be hidden in the top bar.
        return_visible: calls
            .as_ref()
            .and_then(|calls| calls.return_button())
            .is_some_and(|button| button.get_visible()),
    }
}

/// "0:07", "12:30", "1:02:03".
fn is_duration(text: &str) -> bool {
    let parts: Vec<&str> = text.split(':').collect();
    (2..=3).contains(&parts.len())
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        && parts[1..].iter().all(|part| part.len() == 2)
}

// ---- the desktop's stand-ins, recording ---------------------------------------------------------

/// P5's notifier, top-bar icon and badge as the self-test sees them: what the calls asked of them.
/// Its `ring` and `stop_ringing` set and calm the icon and the dock as P5's `DesktopNotifier` does
/// (one call at a time here), so the checks see what the desktop would.
#[derive(Default)]
struct Desk {
    rings: RefCell<Vec<(String, String, String)>>,
    stops: RefCell<Vec<String>>,
    tray_ringing: Cell<bool>,
    urgent: Cell<bool>,
    available: Event<bool>,
}

impl Notifier for Desk {
    fn available(&self) -> bool {
        false
    }

    fn show(&self, _toast: &Toast) {}

    fn withdraw(&self, _tag: &str) {}

    fn ring(&self, call_id: &str, title: &str, body: &str) {
        self.rings
            .borrow_mut()
            .push((call_id.to_string(), title.to_string(), body.to_string()));
        self.set_ringing(true);
        self.set_urgent(true);
    }

    fn stop_ringing(&self, call_id: &str) {
        self.stops.borrow_mut().push(call_id.to_string());
        self.set_ringing(false);
        self.set_urgent(false);
    }
}

impl TrayState for Desk {
    fn available(&self) -> bool {
        false
    }

    fn on_available_changed(&self) -> &Event<bool> {
        &self.available
    }

    fn set_unread(&self, _unread: i64, _title: &str) {}

    fn set_ringing(&self, ringing: bool) {
        self.tray_ringing.set(ringing);
    }
}

impl BadgeState for Desk {
    fn set_count(&self, _unread: i64) {}

    fn set_urgent(&self, urgent: bool) {
        self.urgent.set(urgent);
    }
}

/// The desk installed in the three slots, and what was there before (put back at the end).
struct DeskInstalled {
    desk: Rc<Desk>,
    notifier: Rc<dyn Notifier>,
    tray: Rc<dyn TrayState>,
    badge: Rc<dyn BadgeState>,
}

impl DeskInstalled {
    fn install(shell: &Shell) -> DeskInstalled {
        let desk = Rc::new(Desk::default());
        let installed = DeskInstalled {
            desk: Rc::clone(&desk),
            notifier: shell.notifier.get(),
            tray: shell.tray.get(),
            badge: shell.badge.get(),
        };
        shell.notifier.install(desk.clone());
        shell.tray.install(desk.clone());
        shell.badge.install(desk);
        installed
    }

    fn restore(self, shell: &Shell) {
        shell.notifier.install(self.notifier);
        shell.tray.install(self.tray);
        shell.badge.install(self.badge);
    }
}

/// P3's camera for Alice: a test pattern sent as MJPEG, remembering which device it was asked for.
#[derive(Default)]
struct FakeCamera {
    asked: RefCell<Vec<Option<String>>>,
    devices: Event<()>,
}

impl Camera for FakeCamera {
    fn present(&self) -> bool {
        true
    }

    fn devices(&self) -> Vec<CameraDevice> {
        vec![CameraDevice {
            id: CAMERA_ID.into(),
            name: "Test pattern".into(),
        }]
    }

    fn on_devices_changed(&self) -> &Event<()> {
        &self.devices
    }

    fn open(
        &self,
        device: Option<String>,
    ) -> LocalBoxFuture<'static, Result<gst::Element, CameraRefusal>> {
        self.asked.borrow_mut().push(device);
        let source = gst::parse::bin_from_description(ALICE_CAMERA, true)
            .map(|bin| bin.upcast::<gst::Element>())
            .map_err(|_| CameraRefusal::Failed);
        Box::pin(std::future::ready(source))
    }
}

// ---- the two servers and the relay ------------------------------------------------------------

/// Who is who in one server.
#[derive(Debug, Clone, Copy)]
struct World {
    alice: i64,
    bob: i64,
}

/// The people (seeded once per server), the direct chat as `me` lists it, and `/me` saying the
/// server carries calls and video calls.
fn seed(fake: &FakeServer, me: &str) -> Option<World> {
    let find = |name: &str| {
        fake.state()
            .users
            .iter()
            .find(|user| user.username == name)
            .map(|user| user.id)
    };
    if find("alice").is_none() {
        fake.seed_default();
    }
    let (alice, bob) = (find("alice")?, find("bob")?);
    let family = fake
        .state()
        .user(alice)
        .and_then(|user| user.families.first().map(|(id, _)| *id))?;
    let bob_in = fake
        .state()
        .user(bob)
        .is_some_and(|user| user.families.iter().any(|(id, _)| *id == family));
    if !bob_in {
        fake.join(bob, family, "member");
    }
    let (peer, title) = if me == "alice" {
        (bob, "Bob")
    } else {
        (alice, "Alice")
    };
    let chat = ChatDto {
        peer_user_id: Some(peer),
        family_ids: Some(vec![family]),
        ..ChatDto::new(DIRECT_CHAT, "direct", title)
    };
    let mut rows = fake.state().chats.get(&family).cloned().unwrap_or_default();
    rows.retain(|row| row.chat.id != DIRECT_CHAT);
    rows.push(ChatRowDto::new(chat));
    fake.set_chats(family, rows);
    {
        let mut state = fake.state();
        state.calls = true;
        state.video_calls = true;
    }
    Some(World { alice, bob })
}

/// Who a server frame goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum To {
    Sender,
    Other,
}

/// The server frames one client call frame from `from` (the caller's id in the CALLEE's world)
/// turns into — `tools/call-interop/src/relay.rs`'s `route`, the server's rules.
fn route(from: i64, frame: &str) -> Vec<(To, String)> {
    let Ok(Value::Object(frame)) = serde_json::from_str::<Value>(frame) else {
        return Vec::new();
    };
    let Some(call_id) = frame.get("call_id").and_then(Value::as_str) else {
        return Vec::new();
    };
    let field = |name: &str| frame.get(name).cloned().unwrap_or(Value::Null);
    match frame.get("type").and_then(Value::as_str) {
        Some("call_offer") => vec![
            (
                To::Sender,
                json!({"type": "call_ringing", "call_id": call_id}).to_string(),
            ),
            (
                To::Other,
                json!({
                    "type": "call_offer",
                    "call_id": call_id,
                    "chat_id": field("chat_id"),
                    "from_user_id": from,
                    "sdp": field("sdp"),
                    "video": frame.get("video").and_then(Value::as_bool).unwrap_or(false),
                })
                .to_string(),
            ),
        ],
        Some("call_answer") => vec![(
            To::Other,
            json!({"type": "call_answer", "call_id": call_id, "sdp": field("sdp")}).to_string(),
        )],
        Some("call_ice") => vec![(
            To::Other,
            json!({"type": "call_ice", "call_id": call_id, "candidate": field("candidate")})
                .to_string(),
        )],
        Some("call_end") => vec![(
            To::Other,
            json!({"type": "call_end", "call_id": call_id, "reason": field("reason")}).to_string(),
        )],
        _ => Vec::new(),
    }
}

/// The server's part of every call between the two servers' clients, running until stopped.
#[derive(Default)]
struct Relay {
    stop: Cell<bool>,
    /// "alice call_offer <id>", "bob call_end hangup <id>", … — what went across, in order.
    carried: RefCell<Vec<String>>,
}

impl Relay {
    fn start(alice: Rc<FakeServer>, bob: Rc<FakeServer>, worlds: (World, World)) -> Rc<Relay> {
        let relay = Rc::new(Relay::default());
        let running = Rc::clone(&relay);
        let (world_a, world_b) = worlds;
        glib::spawn_future_local(async move {
            let mut seen = (alice.frames_received().len(), bob.frames_received().len());
            while !running.stop.get() {
                seen.0 = running.pump("alice", &alice, &bob, world_b.alice, seen.0);
                seen.1 = running.pump("bob", &bob, &alice, world_a.bob, seen.1);
                glib::timeout_future(Duration::from_millis(15)).await;
            }
        });
        relay
    }

    /// What `from`'s client sent since `seen`, carried; answers the new mark.
    fn pump(
        &self,
        who: &str,
        from: &FakeServer,
        to: &FakeServer,
        sender: i64,
        seen: usize,
    ) -> usize {
        let frames = from.frames_received();
        for frame in frames.iter().skip(seen) {
            let routed = route(sender, frame);
            if let Some(kind) = routed.first().and_then(|_| {
                serde_json::from_str::<Value>(frame).ok().map(|value| {
                    let kind = value["type"].as_str().unwrap_or_default().to_string();
                    let call = value["call_id"].as_str().unwrap_or_default();
                    match value["reason"].as_str() {
                        Some(reason) => format!("{who} {kind} {reason} {call}"),
                        None => format!("{who} {kind} {call}"),
                    }
                })
            }) {
                self.carried.borrow_mut().push(kind);
            }
            for (destination, out) in routed {
                match destination {
                    To::Sender => from.send_frame(&out),
                    To::Other => to.send_frame(&out),
                }
            }
        }
        frames.len()
    }

    fn carried(&self, line: &str) -> bool {
        self.carried.borrow().iter().any(|carried| carried == line)
    }
}

/// Bob's commands and his probes, through his server.
#[derive(Default)]
struct PuppetLine {
    commands: RefCell<VecDeque<Value>>,
    probes: RefCell<VecDeque<(Instant, Probe)>>,
}

impl PuppetLine {
    const KEPT: usize = 6000;

    fn serve(bob_server: &FakeServer) -> Rc<PuppetLine> {
        let line = Rc::new(PuppetLine::default());
        let next = Rc::clone(&line);
        bob_server.route(
            "GET",
            "/api/v1/selftest/next",
            Rc::new(move |_, _| match next.commands.borrow_mut().pop_front() {
                Some(command) => FakeResponse::json(200, &command),
                None => FakeResponse::empty(204),
            }),
        );
        let state = Rc::clone(&line);
        bob_server.route(
            "POST",
            "/api/v1/selftest/state",
            Rc::new(move |_, request| {
                if let Ok(probe) = serde_json::from_slice::<Probe>(&request.body) {
                    let mut probes = state.probes.borrow_mut();
                    probes.push_back((Instant::now(), probe));
                    while probes.len() > PuppetLine::KEPT {
                        probes.pop_front();
                    }
                }
                FakeResponse::empty(204)
            }),
        );
        line
    }

    fn tell(&self, command: Value) {
        self.commands.borrow_mut().push_back(command);
    }

    fn latest(&self) -> Probe {
        self.probes
            .borrow()
            .back()
            .map(|(_, probe)| probe.clone())
            .unwrap_or_default()
    }

    /// The probes posted between `from` and `to`.
    fn between(&self, from: Instant, to: Instant) -> Vec<Probe> {
        self.probes
            .borrow()
            .iter()
            .filter(|(at, _)| *at >= from && *at <= to)
            .map(|(_, probe)| probe.clone())
            .collect()
    }
}

/// The loudest far-side sample and how much RTP of `field` came, over `probes` (oldest first).
fn window_facts(probes: &[Probe], field: impl Fn(&Counts) -> u64) -> (u32, u64) {
    let peak = probes
        .iter()
        .map(|probe| probe.counts.audio_peak)
        .max()
        .unwrap_or(0);
    let delta = match (probes.first(), probes.last()) {
        (Some(first), Some(last)) => field(&last.counts).saturating_sub(field(&first.counts)),
        _ => 0,
    };
    (peak, delta)
}

// ---- Bob ------------------------------------------------------------------------------------

/// Bob's instance: signs in to `server`, then posts what he sees and does what he is told.
async fn puppet(h: &Harness, server: &str) {
    let shell = h.shell();
    let desk = DeskInstalled::install(&shell);
    let Some(url) = ServerUrl::normalise(Some(server)) else {
        h.check("puppet_server", false, "no server address");
        return;
    };
    shell.open_server(url);
    h.wait_until(WAIT, || {
        shell.connection().is_some() && !shell.gate_facts().booting
    })
    .await;
    if let Some(connection) = shell.connection()
        && connection
            .session
            .state()
            .me
            .as_ref()
            .map(|me| me.username.as_str())
            != Some("bob")
    {
        let _ = connection.session.sign_in("bob", "password").await;
    }
    let ready = h
        .wait_until(WAIT, || {
            shell.connection().is_some_and(|connection| {
                connection.session.state().can_chat() && connection.live.link() == Link::Up
            })
        })
        .await;
    h.check("puppet_ready", ready, "bob signed in, socket up");
    // His main window out of the way, as if in the top bar: a call comes in to a hidden window
    // (§C 7), and Alice's screenshots and clicks find only her own.
    if let Some(main) = shell.main_window() {
        main.hide_to_tray();
    }
    loop {
        let Some(connection) = shell.connection() else {
            h.sleep(Duration::from_millis(100)).await;
            continue;
        };
        let Some(token) = connection.tokens.token() else {
            h.sleep(Duration::from_millis(100)).await;
            continue;
        };
        let bearer = format!("Bearer {token}");
        let seen = probe(&shell, &desk.desk);
        let body = serde_json::to_vec(&seen).unwrap_or_default();
        let post = HttpRequest::new(Method::Post, format!("{server}/api/v1/selftest/state"))
            .header("Authorization", bearer.clone())
            .body("application/json", body);
        let _ = shell.http.send(post).await;
        let next = HttpRequest::new(Method::Get, format!("{server}/api/v1/selftest/next"))
            .header("Authorization", bearer);
        if let Ok(answer) = shell.http.send(next).await
            && answer.status == 200
            && let Ok(command) = serde_json::from_slice::<Value>(&answer.body)
        {
            obey(&shell, &command);
        }
        h.sleep(Duration::from_millis(50)).await;
    }
}

/// One of the director's commands, done the way a person (or their desktop) would.
fn obey(shell: &Rc<Shell>, command: &Value) {
    let text = |name: &str| command[name].as_str().unwrap_or_default().to_string();
    let window = hub::get().and_then(|calls| calls.window());
    match command["do"].as_str().unwrap_or_default() {
        "place" => calls::place(
            shell,
            command["chat"].as_i64().unwrap_or_default(),
            command["peer"].as_i64().unwrap_or_default(),
            command["video"].as_bool().unwrap_or(false),
        ),
        "answer" => shell
            .app
            .activate_action(actions::ANSWER_CALL.name, Some(&text("call").to_variant())),
        "decline" => shell
            .app
            .activate_action(actions::DECLINE_CALL.name, Some(&text("call").to_variant())),
        "press" => {
            let role = match text("disc").as_str() {
                "accept" => Role::Accept,
                "decline" => Role::Decline,
                "mute" => Role::Mute,
                "camera" => Role::Camera,
                _ => Role::HangUp,
            };
            if let Some(window) = window {
                window.disc(role).emit_clicked();
            }
        }
        "close" => {
            if let Some(window) = window {
                window.window().close();
            }
        }
        "return" => shell
            .app
            .activate_action(actions::RETURN_TO_CALL.name, None),
        "quit" => shell.quit(),
        _ => {}
    }
}

// ---- Alice, directing ------------------------------------------------------------------------

/// What Alice's half of the run holds.
struct Director<'a> {
    h: &'a Harness,
    shell: Rc<Shell>,
    desk: Rc<Desk>,
    line: Rc<PuppetLine>,
    bob_pid: Option<String>,
}

impl Director<'_> {
    fn alice(&self) -> Probe {
        probe(&self.shell, &self.desk)
    }

    fn bob(&self) -> Probe {
        self.line.latest()
    }

    async fn wait_alice(&self, limit: Duration, condition: impl Fn(&Probe) -> bool) -> bool {
        self.h
            .wait_until(limit, || condition(&probe(&self.shell, &self.desk)))
            .await
    }

    async fn wait_bob(&self, limit: Duration, condition: impl Fn(&Probe) -> bool) -> bool {
        self.h
            .wait_until(limit, || condition(&self.line.latest()))
            .await
    }

    async fn wait_both(&self, limit: Duration, condition: impl Fn(&Probe) -> bool) -> bool {
        self.h
            .wait_until(limit, || {
                condition(&probe(&self.shell, &self.desk)) && condition(&self.line.latest())
            })
            .await
    }

    fn window(&self) -> Option<gtk::Window> {
        hub::get()
            .and_then(|calls| calls.window())
            .map(|window| window.window().clone().upcast())
    }

    /// Alice presses a disc: a real click when the run has input, the same handler otherwise.
    async fn press(&self, role: Role) -> &'static str {
        let Some(window) = hub::get().and_then(|calls| calls.window()) else {
            return "no window";
        };
        let disc = window.disc(role);
        let toplevel: gtk::Window = window.window().clone().upcast();
        let title = toplevel.title().map(|t| t.to_string()).unwrap_or_default();
        if self.h.input() && self.click(&toplevel, &title, &disc).await {
            "clicked"
        } else {
            disc.emit_clicked();
            "emitted"
        }
    }

    /// A real click on `widget` of Alice's window `title`: raised first — the two instances'
    /// windows open at the same place, with no window manager to keep them apart — and the pointer
    /// taken away after, so no tooltip is left standing over the next look.
    async fn click(&self, toplevel: &gtk::Window, title: &str, widget: &gtk::Button) -> bool {
        let Some(xid) = self.xid(title, false).await else {
            return false;
        };
        self.h.xdotool(&["windowraise", &xid]).await;
        self.h.sleep(Duration::from_millis(200)).await;
        let (_, geometry) = self
            .h
            .xdotool(&["getwindowgeometry", "--shell", &xid])
            .await;
        let field = |name: &str| {
            geometry
                .lines()
                .find_map(|line| line.strip_prefix(name))
                .and_then(|value| value.trim().parse::<f64>().ok())
        };
        let (Some(wx), Some(wy)) = (field("X="), field("Y=")) else {
            return false;
        };
        let middle =
            gtk::graphene::Point::new(widget.width() as f32 / 2.0, widget.height() as f32 / 2.0);
        let Some(point) = widget.compute_point(toplevel, &middle) else {
            return false;
        };
        let (sx, sy) = toplevel.surface_transform();
        let scale = toplevel.surface().map_or(1.0, |surface| surface.scale());
        let x = (wx + (sx + f64::from(point.x())) * scale).round() as i32;
        let y = (wy + (sy + f64::from(point.y())) * scale).round() as i32;
        // The pointer first, then — once GTK has seen it arrive — the button.
        self.h
            .xdotool(&["mousemove", "--sync", &x.to_string(), &y.to_string()])
            .await;
        self.h.sleep(Duration::from_millis(150)).await;
        let (clicked, _) = self.h.xdotool(&["click", "1"]).await;
        self.h.sleep(Duration::from_millis(300)).await;
        self.h.xdotool(&["mousemove", "--sync", "0", "799"]).await;
        clicked
    }

    /// Alice's Call window resized on the X server (no window manager does it here).
    async fn resize_call_window(&self, width: i32, height: i32) {
        let Some(title) = self
            .window()
            .and_then(|window| window.title())
            .map(|title| title.to_string())
        else {
            return;
        };
        if let Some(xid) = self.xid(&title, false).await {
            self.h
                .xdotool(&[
                    "windowsize",
                    "--sync",
                    &xid,
                    &width.to_string(),
                    &height.to_string(),
                ])
                .await;
            self.h.sleep(Duration::from_millis(400)).await;
        }
    }

    /// Alice's Call window raised and given the keyboard (for Escape).
    async fn focus_call_window(&self) {
        let Some(title) = self
            .window()
            .and_then(|window| window.title())
            .map(|title| title.to_string())
        else {
            return;
        };
        if let Some(xid) = self.xid(&title, false).await {
            self.h.xdotool(&["windowraise", &xid]).await;
            self.h.xdotool(&["windowfocus", "--sync", &xid]).await;
            self.h.sleep(Duration::from_millis(200)).await;
        }
    }

    /// The X window of `title` in Alice's process (or Bob's): of the visible ones so named, the
    /// largest and then the newest — a tooltip carries its window's title, and is small. (`--all`:
    /// xdotool's conditions are OR-ed by default, which matched every window of the process.)
    async fn xid(&self, title: &str, bob: bool) -> Option<String> {
        let pid = if bob {
            self.bob_pid.clone().unwrap_or_default()
        } else {
            std::process::id().to_string()
        };
        let pattern = format!("^{}$", literal(title));
        let (_, ids) = self
            .h
            .xdotool(&[
                "search",
                "--all",
                "--onlyvisible",
                "--pid",
                &pid,
                "--name",
                &pattern,
            ])
            .await;
        let mut best: Option<(i64, String)> = None;
        let mut seen = Vec::new();
        for id in ids.lines().map(str::trim).filter(|id| !id.is_empty()) {
            let (_, geometry) = self.h.xdotool(&["getwindowgeometry", "--shell", id]).await;
            let field = |name: &str| {
                geometry
                    .lines()
                    .find_map(|line| line.strip_prefix(name))
                    .and_then(|value| value.trim().parse::<i64>().ok())
                    .unwrap_or(0)
            };
            let area = field("WIDTH=") * field("HEIGHT=");
            seen.push(format!(
                "{id} {}x{}+{}+{}",
                field("WIDTH="),
                field("HEIGHT="),
                field("X="),
                field("Y=")
            ));
            if best.as_ref().is_none_or(|(most, _)| area >= *most) {
                best = Some((area, id.to_string()));
            }
        }
        println!("selftest: windows titled {title:?} (pid {pid}): {seen:?}");
        best.map(|(_, id)| id)
    }

    /// `OUT/screenshots/calls-<name>.png`: the window titled `title` of `pid` (Alice's when
    /// `None`), raised and taken alone; the whole screen when it cannot be found.
    async fn shot(&self, name: &str, title: &str, bob: bool) {
        self.h.sleep(Duration::from_millis(450)).await;
        let Some(xid) = self.xid(title, bob).await else {
            self.h.screenshot(name).await;
            return;
        };
        self.h.xdotool(&["windowraise", &xid]).await;
        self.h.sleep(Duration::from_millis(300)).await;
        let folder = self.h.out().join("screenshots");
        let _ = std::fs::create_dir_all(&folder);
        let path = folder.join(format!("calls-{name}.png"));
        let (ok, output) = self
            .h
            .run(&["import", "-window", &xid, &path.to_string_lossy()])
            .await;
        if ok {
            println!("SHOT {}", path.display());
        } else {
            eprintln!("selftest: screenshot {name} failed: {output}");
        }
    }

    /// Both sides back to no call, the windows gone.
    async fn settle(&self, name: &str) -> bool {
        let settled = self.wait_both(LINGER, Probe::idle).await;
        let (alice, bob) = (self.alice(), self.bob());
        self.h.check(
            name,
            settled
                && !alice.return_visible
                && !bob.return_visible
                && !alice.awake
                && !bob.awake
                && alice.tone.is_none()
                && bob.tone.is_none(),
            format!(
                "alice {:?} window {} awake {} tone {:?}; bob {:?} window {} awake {} tone {:?}",
                alice.stage,
                alice.window_visible,
                alice.awake,
                alice.tone,
                bob.stage,
                bob.window_visible,
                bob.awake,
                bob.tone
            ),
        )
    }
}

/// `text` as an extended regular expression matching itself (xdotool's `--name`).
fn literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if "\\^$.|?*+()[]{}".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Starts Bob's instance against `server`.
fn spawn_bob(h: &Harness, server: &str) -> Result<gio::Subprocess, String> {
    let exe = std::env::current_exe().map_err(|error| format!("no executable: {error}"))?;
    let out = h.out().join("bob");
    let home = out.join("home");
    std::fs::create_dir_all(&home).map_err(|error| format!("{}: {error}", home.display()))?;
    let launcher = gio::SubprocessLauncher::new(gio::SubprocessFlags::NONE);
    for (variable, folder) in [
        ("XDG_CONFIG_HOME", ".config"),
        ("XDG_STATE_HOME", ".local/state"),
        ("XDG_DATA_HOME", ".local/share"),
        ("XDG_CACHE_HOME", ".cache"),
    ] {
        launcher.setenv(variable, home.join(folder), true);
    }
    launcher.setenv("FC_TEST_AUDIO_SOURCE", AUDIO_SOURCE, true);
    launcher.setenv("FC_TEST_AUDIO_SINK", AUDIO_SINK, true);
    launcher.setenv("FC_TEST_VIDEO_SOURCE", BOB_CAMERA, true);
    launcher.unsetenv("FC_TEST_VIDEO_SINK");
    launcher.setenv(PUPPET, server, true);
    let argv: [&std::ffi::OsStr; 5] = [
        exe.as_os_str(),
        "--selftest".as_ref(),
        out.as_os_str(),
        "--only".as_ref(),
        "calls".as_ref(),
    ];
    launcher
        .spawn(&argv)
        .map_err(|error| format!("bob did not start: {error}"))
}

async fn director(h: &Harness) {
    let shell = h.shell();
    let Some(main) = shell.main_window() else {
        h.check("main_window", false, "no main window was built");
        return;
    };
    let Some(part) = hub::get() else {
        h.check("installed", false, "calls::install did not run");
        return;
    };
    let return_button = part.return_button();
    let in_header = return_button.as_ref().is_some_and(|button| {
        button
            .ancestor(adw::HeaderBar::static_type())
            .is_some_and(|header| header == main.content_header().upcast::<gtk::Widget>())
    });
    h.check(
        "installed",
        in_header
            && return_button
                .as_ref()
                .is_some_and(|button| !button.is_visible()),
        "the hub, Return to Call in the content header (hidden with no call)",
    );

    // Alice's desktop stand-ins, her camera and her media on the test sources.
    let installed = DeskInstalled::install(&shell);
    let camera = Rc::new(FakeCamera::default());
    let camera_before = shell.camera.get();
    shell.camera.install(camera.clone());
    shell.settings.set_string(keys::VIDEO_CAMERA, CAMERA_ID);
    calls::use_devices(
        &shell,
        CallDevices {
            microphone: AUDIO_SOURCE.into(),
            speaker: AUDIO_SINK.into(),
            ringer: AUDIO_SINK.into(),
            video_sink: "gtk4paintablesink".into(),
            camera: None,
        },
    );

    // The two servers.
    let alice_server = h.fake_server();
    let bob_server = match FakeServer::start() {
        Ok(server) => server,
        Err(error) => {
            h.check("bob_server", false, error);
            return;
        }
    };
    let (Some(world_a), Some(world_b)) = (seed(&alice_server, "alice"), seed(&bob_server, "bob"))
    else {
        h.check("seeded", false, "the people could not be seeded");
        return;
    };
    let relay = Relay::start(
        Rc::clone(&alice_server),
        Rc::clone(&bob_server),
        (world_a, world_b),
    );
    let line = PuppetLine::serve(&bob_server);

    // Alice signs in (again: a connection made before her media changed keeps the old one).
    let Some(url) = ServerUrl::normalise(Some(&alice_server.url())) else {
        h.check("alice_server", false, "no address");
        return;
    };
    shell.open_server(url);
    h.wait_until(WAIT, || {
        shell.connection().is_some() && !shell.gate_facts().booting
    })
    .await;
    if let Some(connection) = shell.connection()
        && connection
            .session
            .state()
            .me
            .as_ref()
            .map(|me| me.username.as_str())
            != Some("alice")
    {
        let _ = connection.session.sign_in("alice", "password").await;
    }
    let alice_ready = h
        .wait_until(WAIT, || {
            main.page() == GatePage::Chats
                && shell
                    .connection()
                    .is_some_and(|connection| connection.live.link() == Link::Up)
        })
        .await;
    h.check(
        "alice_signed_in",
        alice_ready,
        format!("page {:?}", main.page()),
    );
    h.check(
        "buttons_offered",
        calls::offered(&shell, world_a.bob, false)
            && calls::offered(&shell, world_a.bob, true)
            && calls::can_place(&shell, false)
            && !calls::can_place(&shell, true),
        "Call and Video Call shown with calls on; insensitive while recording",
    );

    // Bob.
    let bob = match spawn_bob(h, &bob_server.url()) {
        Ok(process) => process,
        Err(error) => {
            h.check("bob_started", false, error);
            return;
        }
    };
    let director = Director {
        h,
        shell: Rc::clone(&shell),
        desk: Rc::clone(&installed.desk),
        line: Rc::clone(&line),
        bob_pid: bob.identifier().map(|pid| pid.to_string()),
    };
    let bob_ready = director
        .wait_bob(Duration::from_secs(90), |bob| bob.ready)
        .await;
    h.check(
        "bob_started",
        bob_ready,
        format!("pid {:?}", director.bob_pid),
    );
    if bob_ready && alice_ready {
        voice_call(&director, world_a).await;
        declined(&director, world_a).await;
        video_call(&director, world_b, &camera).await;
        escape_declines(&director, world_b).await;
        quit_in_a_call(&director, world_a, &bob, &relay).await;
    }

    // Whatever happened, Bob goes.
    if director
        .h
        .with_timeout(Duration::from_millis(200), bob.wait_future())
        .await
        .is_none()
    {
        bob.force_exit();
        let _ = director
            .h
            .with_timeout(Duration::from_secs(5), bob.wait_future())
            .await;
    }
    relay.stop.set(true);
    let unhandled: Vec<String> = alice_server
        .unhandled()
        .into_iter()
        .chain(bob_server.unhandled())
        .collect();
    h.check(
        "fake_servers_knew_every_request",
        unhandled.is_empty(),
        format!("{unhandled:?}"),
    );
    shell.camera.install(camera_before);
    installed.restore(&shell);
}

/// Alice calls Bob (voice); Bob answers from his notification's action; mute; Bob hangs up.
async fn voice_call(d: &Director<'_>, world: World) {
    let h = d.h;
    let say = &d.shell.say;
    calls::place(&d.shell, DIRECT_CHAT, world.bob, false);
    let ringing = d
        .wait_alice(WAIT, |alice| alice.at(CallStage::Ringing))
        .await;
    let alice = d.alice();
    h.check(
        "voice_rings_back",
        ringing
            && alice.window_visible
            && alice.title == "Bob"
            && alice.status == say.get("Ringing…")
            && alice.discs == [say.get("Mute"), say.get("Hang Up")]
            && alice.tone.as_deref() == Some("Ringback")
            && alice.return_visible
            && alice.identity
            && !alice.dark,
        format!("{alice:?}"),
    );
    d.shot("alice-ringing", "Bob", false).await;

    let incoming = d.wait_bob(WAIT, |bob| bob.at(CallStage::Incoming)).await;
    let bob = d.bob();
    let call_id = bob.call_id.clone().unwrap_or_default();
    h.check(
        "voice_rings_on_bob",
        incoming
            && bob.window_visible
            && bob.title == "Alice"
            && bob.status == say.get("Incoming call")
            && bob.discs == [say.get("Decline"), say.get("Accept")]
            && bob.focused.as_deref() == Some(say.get("Accept"))
            && bob.tone.as_deref() == Some("Incoming"),
        format!("{bob:?}"),
    );
    h.check(
        "ring_notice_raised",
        bob.rings.last()
            == Some(&(
                call_id.clone(),
                say.get("Incoming call").to_string(),
                say.format("%@ is calling", &[&"Alice"]),
            ))
            && bob.tray_ringing
            && bob.urgent
            && bob.return_visible,
        format!(
            "rings {:?}, tray {}, urgent {}",
            bob.rings, bob.tray_ringing, bob.urgent
        ),
    );
    d.shot("bob-incoming", "Alice", true).await;

    d.line
        .tell(json!({"do": "answer", "call": "a-stale-notification"}));
    h.sleep(Duration::from_millis(700)).await;
    let bob = d.bob();
    h.check(
        "stale_answer_does_nothing",
        bob.at(CallStage::Incoming) && d.alice().at(CallStage::Ringing),
        format!("bob {:?}", bob.stage),
    );

    d.line
        .tell(json!({"do": "answer", "call": call_id.as_str()}));
    let talking = d
        .wait_both(CALL_UP, |side| side.at(CallStage::Talking))
        .await;
    let (alice, bob) = (d.alice(), d.bob());
    h.check(
        "answered_from_the_notification",
        talking,
        format!("alice {:?}, bob {:?}", alice.stage, bob.stage),
    );
    h.check(
        "ring_notice_withdrawn",
        bob.stops.contains(&call_id)
            && !bob.tray_ringing
            && !bob.urgent
            && bob.tone.is_none()
            && alice.tone.is_none(),
        format!(
            "stops {:?}, tray {}, urgent {}, tones {:?}/{:?}",
            bob.stops, bob.tray_ringing, bob.urgent, alice.tone, bob.tone
        ),
    );
    if !talking {
        return;
    }
    let before = (d.alice().counts, Instant::now());
    h.sleep(Duration::from_millis(1500)).await;
    let after = d.alice().counts;
    let heard = d.line.between(before.1, Instant::now());
    let (_, bob_in) = window_facts(&heard, |counts| counts.audio_rtp_in);
    h.check(
        "rtp_flows_both_ways",
        after.audio_rtp_in > before.0.audio_rtp_in
            && after.audio_rtp_out > before.0.audio_rtp_out
            && bob_in > 0,
        format!(
            "alice in +{} out +{}, bob in +{bob_in}",
            after.audio_rtp_in - before.0.audio_rtp_in,
            after.audio_rtp_out - before.0.audio_rtp_out
        ),
    );
    let (alice, bob) = (d.alice(), d.bob());
    h.check(
        "duration_counts",
        is_duration(&alice.status) && is_duration(&bob.status),
        format!("{:?} / {:?}", alice.status, bob.status),
    );
    h.check(
        "kept_awake_in_a_call",
        alice.awake && bob.awake,
        format!("alice {}, bob {}", alice.awake, bob.awake),
    );
    h.check(
        "talking_discs",
        alice.discs == [say.get("Mute"), say.get("Hang Up")] && alice.identity && !alice.picture,
        format!("{:?}", alice.discs),
    );
    d.shot("alice-talking", "Bob", false).await;
    d.shot("bob-talking", "Alice", true).await;

    if h.input() {
        d.focus_call_window().await;
        h.key("Escape").await;
        h.sleep(Duration::from_millis(500)).await;
        h.check(
            "escape_never_hangs_up",
            d.alice().at(CallStage::Talking),
            "Escape in a call does nothing",
        );
    }

    // Mute: shown, and heard as silence while the RTP goes on.
    h.sleep(Duration::from_millis(1500)).await;
    let loud_from = Instant::now();
    h.sleep(Duration::from_secs(2)).await;
    let (loud, _) = window_facts(&d.line.between(loud_from, Instant::now()), |c| {
        c.audio_rtp_in
    });
    let how = d.press(Role::Mute).await;
    let muted = d
        .wait_alice(Duration::from_secs(3), |alice| alice.muted)
        .await;
    let alice = d.alice();
    h.check(
        "mute_shows",
        muted && alice.discs.first().map(String::as_str) == Some(say.get("Unmute")),
        format!("{how}; {:?}", alice.discs),
    );
    d.shot("alice-muted", "Bob", false).await;
    h.sleep(Duration::from_millis(1500)).await;
    let quiet_from = Instant::now();
    h.sleep(Duration::from_secs(2)).await;
    let (quiet, rtp) = window_facts(&d.line.between(quiet_from, Instant::now()), |c| {
        c.audio_rtp_in
    });
    h.check(
        "mute_heard_as_silence",
        rtp > 0 && quiet * 10 < loud.max(1),
        format!("bob's peak {loud} before, {quiet} muted; rtp +{rtp} while muted"),
    );
    d.press(Role::Mute).await;
    h.sleep(Duration::from_millis(1500)).await;
    let again_from = Instant::now();
    h.sleep(Duration::from_secs(2)).await;
    let (again, _) = window_facts(&d.line.between(again_from, Instant::now()), |c| {
        c.audio_rtp_in
    });
    h.check(
        "unmute_heard_again",
        !d.alice().muted && again * 10 > loud,
        format!("bob's peak {again} after unmute, {loud} before"),
    );

    // Dark and narrow.
    let styles = adw::StyleManager::default();
    styles.set_color_scheme(adw::ColorScheme::ForceDark);
    d.shot("alice-talking-dark", "Bob", false).await;
    d.resize_call_window(480, 400).await;
    d.shot("alice-talking-narrow", "Bob", false).await;
    d.resize_call_window(780, 560).await;
    styles.set_color_scheme(adw::ColorScheme::Default);

    // Bob hangs up.
    d.line.tell(json!({"do": "press", "disc": "hangup"}));
    let ended = d.wait_both(WAIT, |side| side.at(CallStage::Ended)).await;
    let (alice, bob) = (d.alice(), d.bob());
    h.check(
        "hang_up_on_one_side_ends_both",
        ended
            && alice.status == say.get("Call ended")
            && bob.status == say.get("Call ended")
            && alice.discs.is_empty(),
        format!(
            "alice {:?} {:?}, bob {:?} {:?}",
            alice.stage, alice.status, bob.stage, bob.status
        ),
    );
    d.shot("alice-ended", "Bob", false).await;
    d.settle("window_goes_by_itself").await;
}

/// Alice calls Bob; Bob declines from his notification's action.
async fn declined(d: &Director<'_>, world: World) {
    let h = d.h;
    let say = &d.shell.say;
    calls::place(&d.shell, DIRECT_CHAT, world.bob, false);
    let incoming = d.wait_bob(WAIT, |bob| bob.at(CallStage::Incoming)).await;
    let call_id = d.bob().call_id.unwrap_or_default();
    d.line
        .tell(json!({"do": "decline", "call": call_id.as_str()}));
    let declined = d
        .wait_alice(WAIT, |alice| {
            alice.at(CallStage::Ended) && alice.ended_reason.as_deref() == Some("decline")
        })
        .await;
    let alice = d.alice();
    h.check(
        "decline_reaches_the_caller",
        incoming && declined && alice.status == say.get("Declined"),
        format!("{:?} {:?}", alice.stage, alice.status),
    );
    d.shot("alice-declined", "Bob", false).await;
    let withdrawn = d
        .wait_bob(WAIT, |bob| {
            bob.stops.contains(&call_id) && !bob.tray_ringing && !bob.urgent
        })
        .await;
    h.check(
        "decline_withdraws_the_notice",
        withdrawn,
        format!("{:?}", d.bob().stops),
    );
    d.settle("declined_call_goes").await;
}

/// Bob calls Alice with video; stale actions; Alice accepts with a click; the pictures; the camera
/// off and on; the window closed and brought back; Alice hangs up.
async fn video_call(d: &Director<'_>, world_b: World, camera: &FakeCamera) {
    let h = d.h;
    let say = &d.shell.say;
    d.line
        .tell(json!({"do": "place", "chat": DIRECT_CHAT, "peer": world_b.alice, "video": true}));
    let incoming = d
        .wait_alice(WAIT, |alice| alice.at(CallStage::Incoming))
        .await;
    let alice = d.alice();
    let call_id = alice.call_id.clone().unwrap_or_default();
    h.check(
        "video_rings_on_alice",
        incoming
            && alice.window_visible
            && alice.title == "Bob"
            && alice.status == say.get("Incoming video call")
            && alice.discs == [say.get("Decline"), say.get("Accept")]
            && alice.focused.as_deref() == Some(say.get("Accept"))
            && alice.dark
            && alice.tone.as_deref() == Some("Incoming"),
        format!("{alice:?}"),
    );
    h.check(
        "video_ring_notice",
        alice.rings.last()
            == Some(&(
                call_id.clone(),
                say.get("Incoming video call").to_string(),
                say.format("%@ is calling", &[&"Bob"]),
            ))
            && alice.tray_ringing
            && alice.urgent,
        format!("{:?}", alice.rings.last()),
    );
    d.shot("alice-incoming-video", "Bob", false).await;

    let app = &d.shell.app;
    app.activate_action(actions::ANSWER_CALL.name, Some(&"stale-call".to_variant()));
    app.activate_action(actions::DECLINE_CALL.name, Some(&"stale-call".to_variant()));
    h.sleep(Duration::from_millis(600)).await;
    h.check(
        "stale_actions_do_nothing",
        d.alice().at(CallStage::Incoming) && d.bob().at(CallStage::Ringing),
        format!("alice {:?}, bob {:?}", d.alice().stage, d.bob().stage),
    );

    let how = d.press(Role::Accept).await;
    let talking = d
        .wait_both(CALL_UP, |side| side.at(CallStage::Talking))
        .await;
    h.check(
        "accept_answers",
        talking,
        format!(
            "{how}; alice {:?}, bob {:?}",
            d.alice().stage,
            d.bob().stage
        ),
    );
    if !talking {
        return;
    }
    let pictures = d
        .wait_both(WAIT, |side| {
            side.picture && side.self_view && !side.identity
        })
        .await;
    let (alice, bob) = (d.alice(), d.bob());
    h.check(
        "far_picture_and_self_view",
        pictures && alice.dark && bob.dark,
        format!(
            "alice picture {} self {} identity {}; bob picture {} self {} identity {}",
            alice.picture,
            alice.self_view,
            alice.identity,
            bob.picture,
            bob.self_view,
            bob.identity
        ),
    );
    h.check(
        "camera_from_the_setting",
        camera.asked.borrow().as_slice() == [Some(CAMERA_ID.to_string())],
        format!("{:?}", camera.asked.borrow()),
    );
    let from = Instant::now();
    let before = d.alice().counts;
    h.sleep(Duration::from_millis(1500)).await;
    let after = d.alice().counts;
    let (_, bob_video) = window_facts(&d.line.between(from, Instant::now()), |c| c.video_rtp_in);
    h.check(
        "video_flows_both_ways",
        after.video_rtp_in > before.video_rtp_in
            && after.video_rtp_out > before.video_rtp_out
            && after.video_decoded > before.video_decoded
            && bob_video > 0,
        format!(
            "alice video in +{} out +{} decoded +{}, bob in +{bob_video}",
            after.video_rtp_in - before.video_rtp_in,
            after.video_rtp_out - before.video_rtp_out,
            after.video_decoded - before.video_decoded
        ),
    );
    d.shot("alice-video", "Bob", false).await;
    d.shot("bob-video", "Alice", true).await;

    // The camera off: the self-view goes, the far side's stream stops; on: it comes back.
    let how = d.press(Role::Camera).await;
    let off = d
        .wait_alice(Duration::from_secs(3), |alice| {
            !alice.camera && !alice.self_view
        })
        .await;
    let alice = d.alice();
    h.check(
        "camera_off_shows",
        off && alice.discs.get(1).map(String::as_str) == Some(say.get("Turn camera on")),
        format!("{how}; {:?}", alice.discs),
    );
    d.shot("alice-camera-off", "Bob", false).await;
    h.sleep(Duration::from_millis(1500)).await;
    let (from, sent) = (Instant::now(), d.alice().counts);
    h.sleep(Duration::from_secs(2)).await;
    let still = d.alice().counts;
    let seen = d.line.between(from, Instant::now());
    let (_, bob_video) = window_facts(&seen, |c| c.video_rtp_in);
    let (_, bob_audio) = window_facts(&seen, |c| c.audio_rtp_in);
    h.check(
        "camera_off_stops_the_stream",
        still.video_rtp_out == sent.video_rtp_out && bob_video < 3 && bob_audio > 0,
        format!(
            "alice video out +{}, bob video in +{bob_video}, audio in +{bob_audio}",
            still.video_rtp_out - sent.video_rtp_out
        ),
    );
    d.press(Role::Camera).await;
    h.sleep(Duration::from_millis(1500)).await;
    let from = Instant::now();
    h.sleep(Duration::from_secs(2)).await;
    let (_, bob_video) = window_facts(&d.line.between(from, Instant::now()), |c| c.video_rtp_in);
    let alice = d.alice();
    h.check(
        "camera_on_resumes",
        alice.camera && alice.self_view && bob_video > 10,
        format!("bob video in +{bob_video}"),
    );

    d.resize_call_window(480, 400).await;
    d.shot("alice-video-narrow", "Bob", false).await;
    d.resize_call_window(780, 560).await;

    // Closing the window keeps the call; Return to Call brings it back.
    if let Some(window) = d.window() {
        window.close();
    }
    h.sleep(Duration::from_millis(800)).await;
    let from = (Instant::now(), d.alice().counts);
    h.sleep(Duration::from_millis(1200)).await;
    let alice = d.alice();
    h.check(
        "closing_keeps_the_call",
        !alice.window_visible
            && alice.at(CallStage::Talking)
            && d.bob().at(CallStage::Talking)
            && alice.counts.audio_rtp_in > from.1.audio_rtp_in
            && alice.return_visible,
        format!(
            "window {}, stage {:?}, return {}",
            alice.window_visible, alice.stage, alice.return_visible
        ),
    );
    d.shot("main-return-to-call", "Family Connect", false).await;
    let button = hub::get().and_then(|calls| calls.return_button());
    let main = d.shell.main_window();
    let clicked = match (button, main) {
        (Some(button), Some(main)) if h.input() => {
            let toplevel: gtk::Window = main.window().clone().upcast();
            let title = toplevel.title().map(|t| t.to_string()).unwrap_or_default();
            d.click(&toplevel, &title, &button).await
        }
        _ => false,
    };
    if !clicked {
        app.activate_action(actions::RETURN_TO_CALL.name, None);
    }
    let back = d
        .wait_alice(Duration::from_secs(3), |alice| alice.window_visible)
        .await;
    h.check(
        "return_to_call_brings_it_back",
        back && d.alice().at(CallStage::Talking),
        format!("clicked {clicked}"),
    );

    let how = d.press(Role::HangUp).await;
    let ended = d.wait_both(WAIT, |side| side.at(CallStage::Ended)).await;
    let bob = d.bob();
    h.check(
        "hang_up_from_the_window",
        ended
            && bob.status == say.get("Call ended")
            && bob.ended_reason.as_deref() == Some("hangup"),
        format!("{how}; bob {:?} {:?}", bob.stage, bob.ended_reason),
    );
    d.settle("video_call_goes").await;
}

/// Bob calls Alice; Escape in Alice's Call window declines it (the Mac's `.cancelAction`).
async fn escape_declines(d: &Director<'_>, world_b: World) {
    let h = d.h;
    if !h.input() {
        return;
    }
    let say = &d.shell.say;
    d.line
        .tell(json!({"do": "place", "chat": DIRECT_CHAT, "peer": world_b.alice, "video": false}));
    let incoming = d
        .wait_alice(WAIT, |alice| alice.at(CallStage::Incoming))
        .await;
    d.focus_call_window().await;
    h.key("Escape").await;
    let declined = d
        .wait_bob(WAIT, |bob| {
            bob.at(CallStage::Ended) && bob.ended_reason.as_deref() == Some("decline")
        })
        .await;
    h.check(
        "escape_declines_a_ring",
        incoming && declined && d.bob().status == say.get("Declined"),
        format!("bob {:?} {:?}", d.bob().stage, d.bob().status),
    );
    d.settle("escaped_call_goes").await;
}

/// A call up, Bob quits (§3.8): his process is gone within the limit — the Call window holds
/// nothing back — and Alice hears the hang-up.
async fn quit_in_a_call(d: &Director<'_>, world: World, bob: &gio::Subprocess, relay: &Relay) {
    let h = d.h;
    calls::place(&d.shell, DIRECT_CHAT, world.bob, false);
    let incoming = d.wait_bob(WAIT, |bob| bob.at(CallStage::Incoming)).await;
    let call_id = d.bob().call_id.unwrap_or_default();
    d.line.tell(json!({"do": "press", "disc": "accept"}));
    let talking = d
        .wait_both(CALL_UP, |side| side.at(CallStage::Talking))
        .await;
    if !(incoming && talking && d.bob().window_visible) {
        h.check(
            "quit_with_a_call_up",
            false,
            format!("bob {:?}, window {}", d.bob().stage, d.bob().window_visible),
        );
        return;
    }
    let asked = Instant::now();
    d.line.tell(json!({"do": "quit"}));
    let gone = h
        .with_timeout(QUIT_LIMIT, bob.wait_future())
        .await
        .is_some();
    let took = asked.elapsed();
    h.check(
        "quit_with_a_call_up",
        gone,
        format!("bob's process ended after {} ms", took.as_millis()),
    );
    let heard = d
        .wait_alice(WAIT, |alice| alice.at(CallStage::Ended) || alice.idle())
        .await;
    h.check(
        "quit_hangs_up",
        heard
            && d.alice().ended_reason.as_deref() == Some("hangup")
            && relay.carried(&format!("bob call_end hangup {call_id}")),
        format!(
            "alice {:?} ({:?}); relayed {:?} for {call_id}",
            d.alice().stage,
            d.alice().ended_reason,
            relay
                .carried
                .borrow()
                .iter()
                .filter(|line| line.starts_with("bob call_end"))
                .collect::<Vec<_>>()
        ),
    );
    d.settle_alice().await;
}

impl Director<'_> {
    /// Alice alone back to no call (Bob has gone).
    async fn settle_alice(&self) {
        let settled = self.wait_alice(LINGER, Probe::idle).await;
        let alice = self.alice();
        self.h.check(
            "alone_again",
            settled && !alice.awake && !alice.return_visible,
            format!("{:?}, window {}", alice.stage, alice.window_visible),
        );
    }
}
