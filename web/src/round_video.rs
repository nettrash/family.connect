//! A video message, recorded in the browser — Phase 3 of the plan for #79
//! (docs/audio-video-messages-2026-10-04.md, S3, S8.7, S8.8; docs/protocol.md,
//! "Video messages"). The recorder people see is views::round_recorder; this
//! is what it records with.
//!
//! - **The camera** is asked for together with the microphone, in ONE
//!   `getUserMedia` — one prompt, not two — and the microphone's track is
//!   stopped the moment it arrives: nothing records in the preview, and no
//!   indicator contradicts "Not recording" (S3.2, S3.4). The microphone is
//!   asked for again inside the Record click, which by then the browser
//!   answers without a prompt.
//! - **The picture** is taken frame by frame as the camera shows it
//!   (`requestVideoFrameCallback`, bound by hand like crate::webcodecs), cut
//!   to its centre square at 480 × 480 the true way round, and encoded as it
//!   comes by the browser's own H.264 encoder (encode::LivePicture).
//! - **The sound** is the microphone's raw samples, tapped as a voice note's
//!   are (recorder::RawSound), and encoded as the voice note's AAC row when
//!   the recording stops.
//! - **The file** is the MP4 writer's — its index first — with the later of
//!   the two tracks starting where it did (`mp4::Track::lead`).
//!
//! **Never `MediaRecorder` for a video.** It writes WebM in Chrome, which the
//! server refuses, and whatever container it likes elsewhere.
//!
//! **Where it is offered** is asked of the browser (S8.7): `VideoEncoder` for
//! H.264 at 480 × 480 in real time and `AudioEncoder` for AAC must both pass
//! `isConfigSupported`, and `requestVideoFrameCallback` must exist. And the
//! browser must not be WebKit — the ONE place this client goes by what a
//! browser is rather than what it says it can do (crate::webcodecs's rule),
//! documented as such in the plan (Decision 29, Blocked 2) and removed when
//! the Safari trial passes: Safari answers yes to all three, and nobody has
//! yet seen what its live encoder makes of a camera.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use fc_text::media_plan::VOICE_NOTE_BITRATE;
use fc_text::wav;
use js_sys::{Array, Float32Array, Function, Object, Reflect};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Blob, HtmlVideoElement, MediaStream, MediaStreamConstraints, MediaStreamTrack};

use crate::encode::{self, LivePicture, Offered};
use crate::recorder::{now_ms, Listening, RawSound};
use crate::webcodecs::{self, Latency};

/// How long and how big a video message may be on this server: the two keys
/// `GET /families/mine` sends on a server that has video messages, and never
/// on one that has not (docs/protocol.md, "Video messages").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoundLimits {
    pub max_ms: u64,
    pub max_bytes: u64,
}

impl RoundLimits {
    /// Both keys, or a server without video messages. A value that is not a
    /// positive number is not a limit anybody can record to.
    pub fn of(max_ms: Option<i64>, max_bytes: Option<i64>) -> Option<RoundLimits> {
        let (max_ms, max_bytes) = (max_ms?, max_bytes?);
        (max_ms > 0 && max_bytes > 0).then(|| RoundLimits {
            max_ms: max_ms as u64,
            max_bytes: max_bytes as u64,
        })
    }

    /// Where a recording stops: the limit less half a second (S1.1).
    pub fn cap_ms(self) -> u64 {
        fc_text::record::round_cap_ms(self.max_ms)
    }

    /// Where "10 seconds left" is shown and said, and the ring turns orange.
    pub fn warning_ms(self) -> u64 {
        fc_text::record::round_warning_ms(self.max_ms)
    }
}

// --- where it is offered ------------------------------------------------------------------------

/// What this browser and this device can do about video messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Probe {
    /// The page can capture at all: `navigator.mediaDevices` is there. Not on
    /// an insecure (`http://`) page, where neither a voice nor a video
    /// message can be recorded (S1.2 **can record**).
    pub captures: bool,
    /// The device has a camera: `enumerateDevices` lists one ([`has_camera`]).
    pub camera: bool,
    /// This browser can record one the way the profile says (S8.7).
    pub records: bool,
}

/// Whether "Record Video Message" is in the paperclip's menu and the
/// microphone's (S1.5, S1.6): a family or a direct chat, on a server that
/// sends the video-message keys, on a page that can capture at all, once the
/// probe has answered — and then ALWAYS in a browser whose probe fails,
/// where it is shown to explain "This browser can't record video messages.
/// Voice messages work.": no camera listing gates that sentence (WebKit
/// lists none before a grant). Where the probe passes, only on a device
/// with a camera. On an insecure page voice messages do not work either, so
/// nothing is offered there to say they do.
pub fn offers_video_entry(
    server_has_round: bool,
    family_or_direct: bool,
    probe: Option<Probe>,
) -> bool {
    server_has_round
        && family_or_direct
        && probe.is_some_and(|probe| probe.captures && (!probe.records || probe.camera))
}

/// THE DECISION (S8.7): H.264 at 480 × 480 in real time and AAC both
/// encodable, frame callbacks there to take the camera's frames as they
/// come — and not WebKit until the Safari trial passes.
pub fn records(webkit: bool, h264: bool, aac: bool, frame_callbacks: bool) -> bool {
    !webkit && h264 && aac && frame_callbacks
}

/// Whether this browser is WebKit — Safari, and every browser on an iPhone
/// or an iPad, which must all be Safari underneath. Their `navigator.vendor`
/// is Apple's; Chrome's and Edge's is Google's, Firefox's is empty.
pub fn is_webkit() -> bool {
    web_sys::window()
        .and_then(|window| Reflect::get(&window.navigator(), &JsValue::from_str("vendor")).ok())
        .and_then(|vendor| vendor.as_string())
        .is_some_and(|vendor| vendor.starts_with("Apple"))
}

/// Whether a `<video>` can say when each frame is shown.
pub fn has_frame_callbacks() -> bool {
    Reflect::get(&js_sys::global(), &JsValue::from_str("HTMLVideoElement"))
        .ok()
        .filter(|class| class.is_function())
        .and_then(|class| Reflect::get(&class, &JsValue::from_str("prototype")).ok())
        .and_then(|prototype| {
            Reflect::get(&prototype, &JsValue::from_str("requestVideoFrameCallback")).ok()
        })
        .is_some_and(|method| method.is_function())
}

/// The encoder configuration a video message is recorded with — the one the
/// probe asks about, since it asks for this very object.
pub async fn picture_config() -> Option<Object> {
    webcodecs::h264_config(
        encode::ROUND_EDGE,
        encode::ROUND_EDGE,
        encode::ROUND_FRAME_RATE,
        encode::ROUND_BITRATE,
        Latency::Realtime,
    )
    .await
}

/// The rate the sound is asked about at: the one a voice note's context is
/// made at.
const SOUND_RATE: u32 = 48_000;

/// How many devices of `kind` (`videoinput`, `audioinput`) `enumerateDevices`
/// lists — counted by kind, whatever their ids and labels; None where there
/// is no listing (an insecure page, or one that failed).
///
/// Before a grant Chromium and Firefox list at most ONE device of each kind
/// there is, with no id and no label (w3c mediacapture-main, "creating a
/// list of device info objects": "truncate cameraList to its first item"),
/// and none of a kind the device does not have. WebKit lists no camera at
/// all until a capture is granted (bugs.webkit.org 259465) — but WebKit
/// fails the probe (S8.7) and gets the explaining item whatever it lists
/// ([`offers_video_entry`]).
async fn listed(kind: &str) -> Option<usize> {
    let listing = media_devices()?.enumerate_devices().ok()?;
    let listed = JsFuture::from(listing).await.ok()?;
    Some(
        Array::from(&listed)
            .iter()
            .filter(|device| {
                Reflect::get(device, &JsValue::from_str("kind"))
                    .ok()
                    .and_then(|kind| kind.as_string())
                    .is_some_and(|listed| listed == kind)
            })
            .count(),
    )
}

/// Whether this device has a camera: `enumerateDevices` lists one
/// ([`listed`]). No camera listed — or no listing — is no camera (S1.2).
pub async fn has_camera() -> bool {
    listed("videoinput")
        .await
        .is_some_and(|cameras| cameras > 0)
}

/// Whether this device is KNOWN to have a microphone: `enumerateDevices`
/// lists one ([`listed`]) — what a camera that was not found offers a voice
/// message by (S3.6).
pub async fn has_microphone() -> bool {
    listed("audioinput")
        .await
        .is_some_and(|microphones| microphones > 0)
}

/// Whether this page can capture at all: `navigator.mediaDevices` is only
/// there in a secure context.
pub fn captures() -> bool {
    media_devices().is_some()
}

thread_local! {
    /// The answer, once asked: a browser does not change what it can do
    /// while a page is open.
    static PROBED: RefCell<Option<Probe>> = const { RefCell::new(None) };
}

/// What this browser can do, asked once per page.
pub async fn probe() -> Probe {
    if let Some(known) = PROBED.with(|probed| *probed.borrow()) {
        return known;
    }
    let captures = captures();
    let camera = captures && has_camera().await;
    let webkit = is_webkit();
    let frames = has_frame_callbacks();
    // Asked only where the answer could still matter.
    let h264 = !webkit && frames && picture_config().await.is_some();
    let aac = h264 && webcodecs::aac_supported(SOUND_RATE, 1, VOICE_NOTE_BITRATE).await;
    let answer = Probe {
        captures,
        camera,
        records: records(webkit, h264, aac, frames),
    };
    PROBED.with(|probed| *probed.borrow_mut() = Some(answer));
    answer
}

// --- the camera and the microphone --------------------------------------------------------------

/// Which device a person refused, as far as the browser says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    Camera,
    Microphone,
    /// One combined request, and a browser that will not say which (S3.2).
    Both,
}

impl Refusal {
    /// What the recorder says (S3.2).
    pub fn sentence(self) -> &'static str {
        use fc_text::i18n::t;
        match self {
            Refusal::Camera => t(
                "Family needs permission to use your camera. Allow it in your browser's settings for this site.",
            ),
            Refusal::Microphone => t(
                "Family needs permission to use your microphone. Allow it in your browser's settings for this site.",
            ),
            Refusal::Both => t(
                "Family needs permission to use your camera and microphone. Allow them in your browser's settings for this site.",
            ),
        }
    }
}

/// Why the camera or the microphone did not open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Closed {
    Refused(Refusal),
    /// Another app has it (S3.6).
    Busy,
    /// There is no camera — or no microphone — to open (S3.6): a device
    /// gone since the probe, or one the listing was wrong about.
    Missing,
    /// Anything else.
    Failed,
}

fn media_devices() -> Option<web_sys::MediaDevices> {
    let navigator = web_sys::window()?.navigator();
    let devices = Reflect::get(&navigator, &JsValue::from_str("mediaDevices")).ok()?;
    (!devices.is_undefined() && !devices.is_null()).then(|| devices.unchecked_into())
}

/// What the browser says of the permission `name` ("camera", "microphone")
/// — "granted", "denied" or "prompt" — where it says anything: Firefox and
/// older browsers throw on "camera", which is "it will not say".
pub async fn permission(name: &str) -> Option<String> {
    let navigator = web_sys::window()?.navigator();
    let permissions = Reflect::get(&navigator, &JsValue::from_str("permissions")).ok()?;
    let query: Function = Reflect::get(&permissions, &JsValue::from_str("query"))
        .ok()?
        .dyn_into()
        .ok()?;
    let asked = webcodecs::object(&[("name", JsValue::from_str(name))]);
    let promise: js_sys::Promise = query.call1(&permissions, &asked).ok()?.dyn_into().ok()?;
    let status = JsFuture::from(promise).await.ok()?;
    Reflect::get(&status, &JsValue::from_str("state"))
        .ok()?
        .as_string()
}

/// What the browser says of the camera and the microphone — `granted`,
/// `prompt` or `denied`; None where it will not say.
pub async fn statuses() -> (Option<String>, Option<String>) {
    (permission("camera").await, permission("microphone").await)
}

/// A refusal the browser has ALREADY recorded — read before anything is
/// asked, so that a person who turned the microphone off is not prompted for
/// a camera that would be no use to them (S3.2).
pub fn refused(camera: Option<&str>, microphone: Option<&str>) -> Option<Refusal> {
    if camera == Some("denied") {
        return Some(Refusal::Camera);
    }
    if microphone == Some("denied") {
        return Some(Refusal::Microphone);
    }
    None
}

/// Whether opening the camera asks for the microphone in the same request
/// (S3.2: one prompt for both, the first time). Not once it is allowed —
/// the microphone stays off in PREVIEW (S3.4) and opens at Record; while
/// the browser would prompt for it, yes, or Record would raise a second
/// prompt; where the browser will not say, only until this device has
/// asked once.
pub fn asks_microphone(microphone: Option<&str>, asked_before: bool) -> bool {
    match microphone {
        Some("granted") => false,
        Some(_) => true,
        None => !asked_before,
    }
}

/// That this device asked for the camera and the microphone together, and
/// was given them (S3.2) — what [`asks_microphone`] goes by where the
/// browser will not say.
const ASKED_KEY: &str = "fc.round.asked";

pub fn asked_both_before() -> bool {
    local().is_some_and(|storage| storage.get_item(ASKED_KEY).ok().flatten().is_some())
}

pub fn remember_asked_both() {
    if let Some(storage) = local() {
        let _ = storage.set_item(ASKED_KEY, "1");
    }
}

/// Which way a phone's camera faces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Facing {
    /// The front camera — the first choice (S3.5).
    #[default]
    User,
    Environment,
}

impl Facing {
    fn word(self) -> &'static str {
        match self {
            Facing::User => "user",
            Facing::Environment => "environment",
        }
    }

    pub fn other(self) -> Facing {
        match self {
            Facing::User => Facing::Environment,
            Facing::Environment => Facing::User,
        }
    }
}

/// The plan's constraints (Web → Phase 3): `facingMode`, 640 × 480 and
/// 30 fps as ideals, which no camera can fail — or one camera by its id — and one
/// channel of sound, ideally.
fn constraints(camera: Option<&str>, facing: Facing, sound: bool) -> MediaStreamConstraints {
    let ideal = |value: JsValue| webcodecs::object(&[("ideal", value)]);
    let mut video = vec![
        ("width", ideal(JsValue::from(640)).into()),
        ("height", ideal(JsValue::from(480)).into()),
        // The profile's rate (at most 30 fps) as an ideal, which no camera
        // can fail; the live encoder thins whatever comes faster.
        ("frameRate", ideal(JsValue::from(30)).into()),
    ];
    match camera {
        Some(id) => video.push((
            "deviceId",
            webcodecs::object(&[("exact", JsValue::from_str(id))]).into(),
        )),
        None => video.push(("facingMode", JsValue::from_str(facing.word()))),
    }
    let asked = MediaStreamConstraints::new();
    asked.set_video(&webcodecs::object(&video));
    if sound {
        asked.set_audio(&webcodecs::object(&[(
            "channelCount",
            ideal(JsValue::from(1)).into(),
        )]));
    }
    asked
}

fn error_name(error: &JsValue) -> String {
    Reflect::get(error, &JsValue::from_str("name"))
        .ok()
        .and_then(|name| name.as_string())
        .unwrap_or_default()
}

async fn get_user_media(asked: &MediaStreamConstraints) -> Result<MediaStream, String> {
    let devices = media_devices().ok_or_else(String::new)?;
    let promise = devices
        .get_user_media_with_constraints(asked)
        .map_err(|error| error_name(&error))?;
    JsFuture::from(promise)
        .await
        .map(JsCast::unchecked_into)
        .map_err(|error| error_name(&error))
}

/// Which device a refusal of the combined request was about, as far as the
/// browser will now say.
async fn which_refused() -> Refusal {
    match (
        permission("camera").await.as_deref(),
        permission("microphone").await.as_deref(),
    ) {
        (Some("denied"), Some("denied")) | (None, None) => Refusal::Both,
        (Some("denied"), _) => Refusal::Camera,
        (_, Some("denied")) => Refusal::Microphone,
        _ => Refusal::Both,
    }
}

fn closed(name: &str) -> Option<Closed> {
    match name {
        "NotAllowedError" | "SecurityError" | "PermissionDeniedError" => None,
        "NotReadableError" | "TrackStartError" | "AbortError" => Some(Closed::Busy),
        // Asked by `facingMode` (an ideal), these say no such device exists;
        // a camera chosen by its id has already been given up for any.
        "NotFoundError" | "DevicesNotFoundError" | "OverconstrainedError" => Some(Closed::Missing),
        _ => Some(Closed::Failed),
    }
}

/// The camera — `camera` by its id where one was chosen, the front one
/// otherwise — and, the first time (`ask_microphone`), the microphone in the
/// same request: ONE prompt for both. The microphone's track is stopped and
/// taken out before this returns. A chosen camera that is gone is given up
/// for the first one that answers.
pub async fn open_camera(
    camera: Option<String>,
    facing: Facing,
    ask_microphone: bool,
) -> Result<MediaStream, Closed> {
    let mut asked = get_user_media(&constraints(camera.as_deref(), facing, ask_microphone)).await;
    if camera.is_some()
        && matches!(&asked, Err(name) if name == "OverconstrainedError" || name == "NotFoundError")
    {
        asked = get_user_media(&constraints(None, facing, ask_microphone)).await;
    }
    match asked {
        Ok(stream) => {
            for track in stream.get_audio_tracks().iter() {
                let track: MediaStreamTrack = track.unchecked_into();
                track.stop();
                stream.remove_track(&track);
            }
            Ok(stream)
        }
        Err(name) => match closed(&name) {
            Some(closed) => Err(closed),
            None if ask_microphone => Err(Closed::Refused(which_refused().await)),
            None => Err(Closed::Refused(Refusal::Camera)),
        },
    }
}

/// The microphone, alone — at Record (S3.4).
pub async fn open_microphone() -> Result<MediaStream, Closed> {
    let asked = MediaStreamConstraints::new();
    asked.set_audio(&webcodecs::object(&[(
        "channelCount",
        webcodecs::object(&[("ideal", JsValue::from(1))]).into(),
    )]));
    get_user_media(&asked)
        .await
        .map_err(|name| closed(&name).unwrap_or(Closed::Refused(Refusal::Microphone)))
}

/// Every track of `stream` stopped: its light goes out.
pub fn stop(stream: &MediaStream) {
    for track in stream.get_tracks().iter() {
        track.unchecked_into::<MediaStreamTrack>().stop();
    }
}

/// The cameras this device has, as `(id, name)` — named once permission has
/// been given.
pub async fn cameras() -> Vec<(String, String)> {
    let Some(listing) = media_devices().and_then(|devices| devices.enumerate_devices().ok()) else {
        return Vec::new();
    };
    let Ok(listed) = JsFuture::from(listing).await else {
        return Vec::new();
    };
    Array::from(&listed)
        .iter()
        .filter_map(|device| {
            let field = |key: &str| {
                Reflect::get(&device, &JsValue::from_str(key))
                    .ok()
                    .and_then(|value| value.as_string())
                    .unwrap_or_default()
            };
            (field("kind") == "videoinput").then(|| (field("deviceId"), field("label")))
        })
        .collect()
}

/// The camera the person chose last, remembered on this device (S3.5).
const CAMERA_KEY: &str = "fc.round.camera";

/// Whether this device has shown a preview before: the first-time line says
/// "Only you can see this until you start recording." once (S3.4, S7.5).
const PREVIEWED_KEY: &str = "fc.round.previewed";

fn local() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

pub fn chosen_camera() -> Option<String> {
    local()?
        .get_item(CAMERA_KEY)
        .ok()?
        .filter(|id| !id.is_empty())
}

pub fn choose_camera(id: &str) {
    if let Some(storage) = local() {
        let _ = storage.set_item(CAMERA_KEY, id);
    }
}

/// Whether the preview has been shown on this device before — and, from
/// now, that it has.
pub fn previewed_before() -> bool {
    let Some(storage) = local() else {
        return true;
    };
    let before = storage.get_item(PREVIEWED_KEY).ok().flatten().is_some();
    if !before {
        let _ = storage.set_item(PREVIEWED_KEY, "1");
    }
    before
}

// --- "we can't see anything" --------------------------------------------------------------------

/// Whether a picture — RGBA pixels — is near black: an average brightness
/// under 16 of 255. A dark room is dimmer than daylight and still well above
/// that; a privacy shutter, or Android 12's camera switch, is not (S3.6).
pub fn near_black(pixels: &[u8]) -> bool {
    let count = pixels.len() / 4;
    if count == 0 {
        return false;
    }
    let total: u64 = pixels
        .chunks_exact(4)
        .map(|pixel| {
            (u64::from(pixel[0]) * 299 + u64::from(pixel[1]) * 587 + u64::from(pixel[2]) * 114)
                / 1000
        })
        .sum();
    total / (count as u64) < 16
}

/// What `video` is showing, as near black or not — None before it shows
/// anything.
pub fn shows_black(video: &HtmlVideoElement) -> Option<bool> {
    if video.video_width() == 0 || video.video_height() == 0 {
        return None;
    }
    let document = web_sys::window()?.document()?;
    let canvas: web_sys::HtmlCanvasElement =
        document.create_element("canvas").ok()?.dyn_into().ok()?;
    canvas.set_width(16);
    canvas.set_height(16);
    let context: web_sys::CanvasRenderingContext2d =
        canvas.get_context("2d").ok()??.dyn_into().ok()?;
    context
        .draw_image_with_html_video_element_and_dw_and_dh(video, 0.0, 0.0, 16.0, 16.0)
        .ok()?;
    let data = context.get_image_data(0.0, 0.0, 16.0, 16.0).ok()?;
    Some(near_black(&data.data()))
}

// --- what a recording holds ---------------------------------------------------------------------

thread_local! {
    /// Recordings running or waiting in review in this tab.
    static HELD: Cell<u32> = const { Cell::new(0) };
}

/// Whether a video message is being recorded, or waits in review, anywhere
/// in this tab — what a tab closing would lose without asking (S4).
pub fn in_progress() -> bool {
    HELD.get() > 0
}

/// Counted in [`in_progress`] for as long as this lives.
pub struct Held(());

impl Held {
    pub fn new() -> Held {
        HELD.set(HELD.get() + 1);
        Held(())
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        HELD.set(HELD.get().saturating_sub(1));
    }
}

/// Whether the chat may be left now (S4, "Leaving the chat"). Over a video
/// message recorded or waiting in review — reachable only while a call has
/// the recorder step aside and give the page back — it asks "Delete video
/// message?" first; true to go on (and the clip with the chat), false to
/// stay. Nothing held: true, nothing asked.
pub fn may_leave() -> bool {
    if !in_progress() {
        return true;
    }
    #[cfg(test)]
    if let Some(answer) = testing::LEAVING.with(|leaving| {
        leaving.borrow_mut().as_mut().map(|asked| {
            asked.0 += 1;
            asked.1
        })
    }) {
        return answer;
    }
    web_sys::window()
        .and_then(|window| {
            window
                .confirm_with_message(fc_text::i18n::t("Delete video message?"))
                .ok()
        })
        .unwrap_or(true)
}

thread_local! {
    /// Recorders open in this tab.
    static OPEN: Cell<u32> = const { Cell::new(0) };
    /// What a notification tapped while one was open asked for — done once
    /// it closes.
    static WAITING: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
}

/// A recorder open, for as long as this lives: the conversation under it
/// cannot change — a notification tapped meanwhile opens its chat once the
/// recorder closes, never under a clip in review (S4).
pub struct Open(());

impl Open {
    pub fn now() -> Open {
        OPEN.set(OPEN.get() + 1);
        Open(())
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        OPEN.set(OPEN.get().saturating_sub(1));
        if OPEN.get() == 0 {
            if let Some(waiting) = WAITING.with(|waiting| waiting.borrow_mut().take()) {
                waiting();
            }
        }
    }
}

/// Do `then` now — or, while a recorder is open, once it has closed (the
/// latest such wish wins).
pub fn when_closed(then: impl FnOnce() + 'static) {
    if OPEN.get() == 0 {
        then();
    } else {
        WAITING.with(|waiting| *waiting.borrow_mut() = Some(Box::new(then)));
    }
}

#[wasm_bindgen]
extern "C" {
    /// A `<video>`, for the two calls web-sys has no binding for without
    /// its unstable flag.
    #[wasm_bindgen(extends = HtmlVideoElement)]
    type FramedVideo;
    #[wasm_bindgen(method, catch, js_name = requestVideoFrameCallback)]
    fn request_frame(this: &FramedVideo, callback: &Function) -> Result<u32, JsValue>;
    #[wasm_bindgen(method, catch, js_name = cancelVideoFrameCallback)]
    fn cancel_frame(this: &FramedVideo, handle: u32) -> Result<(), JsValue>;
}

/// The performance clock as the browser reads it, without the tests' shift
/// (`recorder::testing::ClockAhead`) — the clock a frame callback's times
/// are on.
fn raw_now() -> f64 {
    web_sys::window()
        .and_then(|window| window.performance())
        .map_or(0.0, |performance| performance.now())
}

/// When a frame was captured, from what its callback was told: the camera's
/// own `captureTime` where the browser gives one, else when it is to be
/// shown, else when the callback ran. Whichever the FIRST frame had is kept
/// for every frame after it, so that the clock never changes mid-clip.
fn frame_time(now: f64, metadata: &JsValue, clock: &Cell<u8>) -> f64 {
    let field = |key: &str| {
        Reflect::get(metadata, &JsValue::from_str(key))
            .ok()
            .and_then(|value| value.as_f64())
            .filter(|value| value.is_finite() && *value > 0.0)
    };
    if clock.get() == 0 {
        clock.set(if field("captureTime").is_some() {
            1
        } else if field("expectedDisplayTime").is_some() {
            2
        } else {
            3
        });
    }
    match clock.get() {
        1 => field("captureTime"),
        2 => field("expectedDisplayTime"),
        _ => None,
    }
    .unwrap_or(now)
}

struct Inner {
    video: HtmlVideoElement,
    picture: RefCell<Option<LivePicture>>,
    callback: RefCell<Option<Closure<dyn FnMut(f64, JsValue)>>>,
    handle: Cell<Option<u32>>,
    clock: Cell<u8>,
    /// When the first frame was captured, on [`now_ms`]'s clock.
    first: Cell<Option<f64>>,
    sound: RefCell<Option<RawSound>>,
    microphone: RefCell<Option<MediaStream>>,
    failed: Cell<bool>,
    stopped: Cell<bool>,
}

impl Inner {
    fn ask(self: &Rc<Self>) {
        if self.stopped.get() {
            return;
        }
        let video: &FramedVideo = self.video.unchecked_ref();
        if let Some(callback) = self.callback.borrow().as_ref() {
            match video.request_frame(callback.as_ref().unchecked_ref()) {
                Ok(handle) => self.handle.set(Some(handle)),
                Err(_) => self.failed.set(true),
            }
        }
    }

    fn frame(self: &Rc<Self>, now: f64, metadata: JsValue) {
        self.handle.set(None);
        if self.stopped.get() {
            return;
        }
        let at = frame_time(now, &metadata, &self.clock);
        let offered = match self.picture.borrow_mut().as_mut() {
            Some(picture) => {
                let offered = picture.frame(&self.video, at);
                if offered == Offered::Kept && self.first.get().is_none() {
                    // Onto the clock the sound is timed by.
                    let first = picture.first_ms().unwrap_or(at);
                    self.first.set(Some(first + (now_ms() - raw_now())));
                }
                offered
            }
            None => return,
        };
        if offered == Offered::Failed {
            self.failed.set(true);
            return;
        }
        self.ask();
    }

    fn halt(&self) {
        self.stopped.set(true);
        if let Some(handle) = self.handle.take() {
            let video: &FramedVideo = self.video.unchecked_ref();
            let _ = video.cancel_frame(handle);
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.halt();
        if let Some(microphone) = self.microphone.borrow_mut().take() {
            stop(&microphone);
        }
    }
}

/// One take: the picture from the moment it began, the sound from the
/// moment the microphone opened. Dropped — Delete, Close, the pane gone — it
/// keeps nothing and lets go of the microphone.
pub struct Take(Rc<Inner>);

/// A finished take.
pub struct Clip {
    pub blob: Blob,
    pub duration_ms: i64,
}

impl Take {
    /// Begin with the next frame `video` shows, encoded by `picture` — an
    /// encoder made and configured BEFORE Record, in the preview: a browser
    /// stops showing the camera's frames for most of a second while it
    /// brings a hardware encoder up, and a take that waited for that would
    /// begin that much after Record.
    pub fn begin(video: &HtmlVideoElement, picture: LivePicture) -> Option<Take> {
        let inner = Rc::new(Inner {
            video: Clone::clone(video),
            picture: RefCell::new(Some(picture)),
            callback: RefCell::new(None),
            handle: Cell::new(None),
            clock: Cell::new(0),
            first: Cell::new(None),
            sound: RefCell::new(None),
            microphone: RefCell::new(None),
            failed: Cell::new(false),
            stopped: Cell::new(false),
        });
        let weak: Weak<Inner> = Rc::downgrade(&inner);
        *inner.callback.borrow_mut() = Some(Closure::new(move |now: f64, metadata: JsValue| {
            if let Some(inner) = weak.upgrade() {
                inner.frame(now, metadata);
            }
        }));
        inner.ask();
        if inner.failed.get() {
            return None;
        }
        Some(Take(inner))
    }

    /// The microphone, opened at Record, and what taps it.
    pub fn hear(&self, sound: RawSound, microphone: MediaStream) {
        *self.0.sound.borrow_mut() = Some(sound);
        *self.0.microphone.borrow_mut() = Some(microphone);
    }

    /// Frames encoded and dropped so far.
    #[cfg(test)]
    pub fn counts(&self) -> (usize, usize) {
        self.0
            .picture
            .borrow()
            .as_ref()
            .map_or((0, 0), LivePicture::counts)
    }

    /// The encoder failed, or the frames stopped coming: the take cannot go
    /// on.
    pub fn failed(&self) -> bool {
        self.0.failed.get()
    }

    /// Stopped: the frames, the sound and the microphone, at once; then the
    /// clip made — None when anything in it failed, or it has no sound.
    pub async fn finish(self) -> Option<Clip> {
        let inner = self.0;
        inner.halt();
        let picture = inner.picture.borrow_mut().take()?;
        let sound = inner.sound.borrow_mut().take();
        let microphone = inner.microphone.borrow_mut().take();
        let heard = match sound {
            Some(sound) => {
                let began = sound.began_ms();
                let (blocks, rate) = sound.finish().await;
                Some((blocks, rate, began))
            }
            None => None,
        };
        // The microphone's light goes out as the sound is in, before the
        // encoding.
        if let Some(microphone) = microphone {
            stop(&microphone);
        }
        let (blocks, rate, began) = heard?;
        let first = inner.first.get()?;
        let (track, frames) = picture.finish().await?;
        let duration_ms = encode::track_ms(&track);
        let (lead, skip) = alignment(first, began, rate);
        let aac = sound_of(&blocks, rate, skip).await?;
        let blob = encode::round_clip((track, frames), Some((&aac, lead)))?;
        Some(Clip { blob, duration_ms })
    }
}

/// How the sound lines up with the picture: the picture began at `first`,
/// the sound at `began` (both in ms, on one clock). A sound that began LATER
/// starts that much into the clip — `lead` ms of nothing in front of it — and
/// one that began earlier loses what it heard before the first frame:
/// `skip` samples at `rate`.
pub fn alignment(first: f64, began: f64, rate: u32) -> (u32, u64) {
    let late = began - first;
    if late >= 0.0 {
        (late.round() as u32, 0)
    } else {
        (0, (-late * f64::from(rate) / 1000.0).round() as u64)
    }
}

/// The rates the AAC row is made at.
const AAC_RATES: [u32; 2] = [44_100, 48_000];

/// `blocks` of mono samples at `rate`, less the first `skip`, as the voice
/// note's AAC row (64 000 bit/s, mono). Sound at a rate the row does not
/// name — a context the browser would not make at 48 kHz — is resampled to
/// 48 kHz first.
async fn sound_of(blocks: &Array, rate: u32, skip: u64) -> Option<encode::Aac> {
    if !AAC_RATES.contains(&rate) {
        let mut all = Vec::new();
        for block in blocks.iter() {
            all.extend(block.unchecked_into::<Float32Array>().to_vec());
        }
        let all = all.split_off((skip as usize).min(all.len()));
        let at = wav::resample(&all, rate, SOUND_RATE);
        let mut once = Some(Float32Array::from(at.as_slice()));
        return encode::aac_from_pcm(
            move || {
                let block = once.take()?;
                let frames = block.length();
                Some((block, frames))
            },
            SOUND_RATE,
            1,
            VOICE_NOTE_BITRATE,
        )
        .await;
    }
    let mut left = skip;
    let mut each = blocks.iter();
    encode::aac_from_pcm(
        move || loop {
            let block: Float32Array = each.next()?.unchecked_into();
            let length = u64::from(block.length());
            if left >= length {
                left -= length;
                continue;
            }
            let block = block.subarray(left as u32, length as u32);
            left = 0;
            let frames = block.length();
            return Some((block, frames));
        },
        rate,
        1,
        VOICE_NOTE_BITRATE,
    )
    .await
}

/// The microphone tapped for a take — through the context made in the
/// Record click.
pub async fn tap(microphone: &MediaStream, listening: Listening) -> Option<RawSound> {
    RawSound::start(microphone, listening).await
}

#[cfg(test)]
pub mod testing {
    use super::*;

    /// The probe answered `probe` for as long as this lives — a browser
    /// that records, or one that cannot, whichever this test runs in — and
    /// asked afresh after.
    pub struct Probed;

    impl Probed {
        pub fn as_if(probe: Probe) -> Probed {
            PROBED.with(|probed| *probed.borrow_mut() = Some(probe));
            Probed
        }
    }

    impl Drop for Probed {
        fn drop(&mut self) {
            PROBED.with(|probed| *probed.borrow_mut() = None);
        }
    }

    /// Ask afresh.
    pub fn forget() {
        PROBED.with(|probed| *probed.borrow_mut() = None);
    }

    thread_local! {
        /// [`may_leave`]'s question, answered by a test: how often it was
        /// asked, and the answer.
        pub static LEAVING: RefCell<Option<(u32, bool)>> = const { RefCell::new(None) };
    }

    /// "Delete video message?" answered `answer` for as long as this lives —
    /// the browser's own `confirm` never raised in a test.
    pub struct Leaving;

    impl Leaving {
        pub fn answering(answer: bool) -> Leaving {
            LEAVING.with(|leaving| *leaving.borrow_mut() = Some((0, answer)));
            Leaving
        }

        pub fn answer(&self, answer: bool) {
            LEAVING.with(|leaving| {
                if let Some(asked) = leaving.borrow_mut().as_mut() {
                    asked.1 = answer;
                }
            });
        }

        /// How often it was asked.
        pub fn asked(&self) -> u32 {
            LEAVING.with(|leaving| leaving.borrow().map_or(0, |asked| asked.0))
        }
    }

    impl Drop for Leaving {
        fn drop(&mut self) {
            LEAVING.with(|leaving| *leaving.borrow_mut() = None);
        }
    }
}
