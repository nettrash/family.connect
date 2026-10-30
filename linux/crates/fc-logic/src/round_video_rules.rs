//! Recording a VIDEO MESSAGE (docs/audio-video-messages-2026-10-04.md — "the plan" below — Phase 3,
//! S3, S4, S6, S8.6; docs/protocol.md, "Video messages") — `RoundVideoRules.cs`: everything about
//! it that needs no camera — the switch, the square, the circle's size, which camera and which of
//! its modes, the encodes, what a refusal or a busy camera says, what is sent — beside the
//! recorder's three states as a machine the window drives (`round_recorder::RoundRecorder`). The
//! camera itself is the shell's (`round_recorder::VideoMessageRecorder`: the camera portal,
//! PipeWire and GStreamer, shell-design §10.5).
//!
//! **RECORDED TO THE PROFILE, NEVER RE-PLANNED** ("The recording profile for a round video"): 480 ×
//! 480, upright, NOT mirrored, H.264 at 500 000 bit/s and at most 30 fps, AAC-LC mono at 64 000 —
//! `moov` first, a 480 × 480 poster. On Linux the square is recorded DIRECTLY (shell-design §10.5:
//! `videocrop` of the centre square, `videoscale`, `videorate`, `x264enc`, `mp4mux faststart=true`)
//! where Windows recorded the camera's own file and made the square afterwards; so [`encodes`] is
//! the order in which the shell tries to BUILD the recording pipeline, and [`came_out`] judges the
//! file it wrote. A take that did not come out as the square goes as the regular video the planner
//! makes of it ([`plan`]'s "Couldn't make it round.").
//!
//! What Linux does not take from Windows:
//! - The switch ([`RECORDING_ENABLED`]) was OFF until 2026-10-29 and is on ahead of the real-desktop
//!   checklist's camera line (§C 25), as Windows' was ahead of its trials — a build with it off
//!   shows no way of recording a circle (Decision 40).
//! - No `PreviewClip`: WinUI's `MediaPlayerElement` would not clip video to a circle, and Windows'
//!   trial T2 was to find which of three tricks did. GTK clips natively (`set_overflow(Hidden)` with
//!   `border-radius`, measured in shell-design §1.5), so there is nothing to choose.
//! - No blank-preview formats: WinUI's frame-source preview stayed blank for RGB24, UYVY and I420
//!   (microsoft-ui-xaml #9756). The Linux pipeline converts every raw format (`videoconvert`) and
//!   decodes MJPEG, and I420 is its own format, so no mode is refused for its format here; a camera
//!   whose caps will not link is reported by the shell as [`CameraTrouble::Unsupported`].
//! - No Media Foundation HRESULTs: [`trouble`] reads a GStreamer error's domain and code instead.
//! - No `ms-settings:` pages: a refusal names a [`SettingsPage`] the shell opens in the desktop's
//!   own settings (GNOME Settings' Privacy pages).
//! - One audio attempt (the `media_encoding` stub: `avenc_aac` takes 64 000 as asked), so there is
//!   no "then the encoder's 96 000" way of asking.

use std::cmp::Ordering;
use std::f64::consts::PI;

use fc_core::protocol::RoundVideoLimits;
use fc_core::text::string_catalog::StringCatalog;
use fc_text::media::time_label;
use fc_text::record;

use crate::media_encoding::{self, H264Profile, VideoEncoding};
use crate::media_outbox::StagedMedia;
use crate::not_sent::RecordingEnd;
use crate::round_recorder::RecorderInterruption;
use crate::voice_notes::CapabilityAccess;

/// Where the recorder's controls stand (S3.3): along the bottom, or — in a pane shorter than 480 —
/// in a column at the trailing edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecorderLayout {
    Row,
    Column,
}

/// The recorder's circle and where its controls go, for one pane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RecorderFit {
    pub diameter: f64,
    pub layout: RecorderLayout,
}

/// One mode a camera offers: its size, its frame rate, and the pixel format it delivers it in —
/// on Linux the format as GStreamer names it in the camera's caps: `video/x-raw`'s `format`
/// (`"YUY2"`, `"NV12"`, `"I420"`, …), `"MJPG"` for `image/jpeg`, `"H264"` for `video/x-h264`.
#[derive(Debug, Clone, PartialEq)]
pub struct CaptureMode {
    pub width: u32,
    pub height: u32,
    pub frame_rate: f64,
    pub subtype: String,
}

impl CaptureMode {
    /// `new CaptureMode(width, height, frameRate)` — the C#'s default subtype, `"NV12"`.
    pub fn new(width: u32, height: u32, frame_rate: f64) -> Self {
        CaptureMode {
            width,
            height,
            frame_rate,
            subtype: "NV12".to_string(),
        }
    }

    /// The same mode in another pixel format.
    pub fn with_subtype(mut self, subtype: &str) -> Self {
        self.subtype = subtype.to_string();
        self
    }
}

/// The square cut from the middle of a frame — even, so a 4:2:0 picture is never split mid-sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SquareCrop {
    pub x: u32,
    pub y: u32,
    pub side: u32,
}

impl SquareCrop {
    /// The square as `videocrop`'s four properties for a `width × height` frame — `(left, right,
    /// top, bottom)`, the pixels taken off each edge. Saturating, so a crop that does not fit the
    /// frame (never one [`centre_square`] made for it) takes nothing it does not have.
    pub fn videocrop(&self, width: u32, height: u32) -> (u32, u32, u32, u32) {
        let right = width.saturating_sub(self.x).saturating_sub(self.side);
        let bottom = height.saturating_sub(self.y).saturating_sub(self.side);
        (self.x, right, self.y, bottom)
    }
}

/// Which way a camera faces. Windows read its enclosure location; on Linux the shell reads the
/// PipeWire node's `api.libcamera.location` (`"front"` → `Front`, `"back"` → `Back`; `"external"`,
/// absent — every V4L2 webcam — or anything else → `Unknown`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CameraPanel {
    Unknown,
    Front,
    Back,
}

/// One camera as the shell lists it: its id, the name it is shown by in "Choose camera", and which
/// way it faces. On Linux the id is the PipeWire node's `node.name` — stable across launches, so
/// the remembered choice survives a restart (the node's `object.serial`, which `pipewiresrc
/// target-object` wants, is per session and looked up from it at open) — and the name its
/// `node.description` (the device monitor's display name).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CameraChoice {
    pub id: String,
    pub name: String,
    pub panel: CameraPanel,
}

impl CameraChoice {
    pub fn new(id: &str, name: &str, panel: CameraPanel) -> Self {
        CameraChoice {
            id: id.to_string(),
            name: name.to_string(),
            panel,
        }
    }
}

/// What was already refused before the recorder could start (S3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecorderRefusal {
    Camera,
    Microphone,
}

/// Why the camera cannot be used although nothing was refused (S3.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CameraTrouble {
    /// Another app holds it — a call in another app, a camera app — or took it away mid-take.
    InUse,
    /// It went away, or there is none: a USB webcam pulled out.
    Missing,
    /// A camera the pipeline cannot use: no mode whose caps link (on Windows: one whose
    /// frame-source preview stays blank) — never a recording without a picture.
    Unsupported,
    /// Anything else the platform raised.
    Failed,
}

/// Where a refusal can be mended: the desktop's own settings page for the device (S3.2) — Windows'
/// `ms-settings:privacy-webcam` / `privacy-microphone`. The shell opens it where the desktop has
/// one ([`SettingsPage::gnome_panel`]) and offers nothing elsewhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SettingsPage {
    Camera,
    Microphone,
}

impl SettingsPage {
    /// The GNOME Settings panel, as `gnome-control-center <panel>` or the `org.gnome.Settings`
    /// application's `launch-panel` action takes it: Privacy & Security → Camera / Microphone
    /// (unverified — real desktop).
    pub fn gnome_panel(self) -> &'static str {
        match self {
            SettingsPage::Camera => "camera",
            SettingsPage::Microphone => "microphone",
        }
    }
}

/// What a finished clip is sent as (S3.6): a video message, or — after the person has been told why
/// — a regular video. `round`: sent with `round: true`. `from_square`: the square clip is what goes;
/// false when it did not come out as the square, and the take goes through the ordinary video path
/// instead (on Linux the recorded file itself, brought to the profile by the planner). `notice`:
/// what REVIEW says about it, or `None` when it is a video message as planned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoundSendPlan {
    pub round: bool,
    pub from_square: bool,
    pub notice: Option<String>,
}

impl RoundSendPlan {
    pub fn new(round: bool, from_square: bool, notice: Option<&str>) -> Self {
        RoundSendPlan {
            round,
            from_square,
            notice: notice.map(str::to_string),
        }
    }
}

/// What the shell read back from a finished take — the discoverer's numbers
/// (`round_recorder::VideoMessageRecorder::read_back`), judged by [`came_out`] and [`plan_take`].
#[derive(Debug, Clone, PartialEq)]
pub struct TakeReading {
    /// The file's size in bytes.
    pub bytes: i64,
    /// The video stream's codec: `"h264"` for `video/x-h264`, otherwise the caps' media type.
    pub video_codec: String,
    /// The STORED sides and the turn the container carries (`image-orientation` tag → 0, 90, 180,
    /// 270).
    pub width: u32,
    pub height: u32,
    pub rotation: u32,
    /// The sound stream's codec: `"aac"` for `audio/mpeg, mpegversion=4`, otherwise the caps' media
    /// type; `None` when the file has no sound stream.
    pub audio_codec: Option<String>,
    /// The video's frame rate (the caps' `framerate`), `None` when it cannot be read.
    pub frame_rate: Option<f64>,
    /// The duration, whole milliseconds, `None` when it cannot be read.
    pub duration_ms: Option<i64>,
    /// `moov` before `mdat` (`faststart::is_moov_first` on the file's bytes).
    pub moov_first: bool,
}

/// THE SWITCH. ON on Linux since 2026-10-29, ahead of the real-desktop checklist's camera line (§C
/// 25: the portal's prompt, the mirrored preview, a 480 × 480 file that plays on the other clients),
/// as Windows switched it on 2026-10-21 ahead of its own trials: the pipeline was measured with a
/// test source only. With it off the video button, the paperclip's and the microphone menu's
/// "Record Video Message" are never drawn, and nothing here touches a camera.
pub const RECORDING_ENABLED: bool = true;

/// Whether the desktop shows the microphone in use while the recorder only previews. It does not on
/// Linux: the audio branch (`pulsesrc`) starts at Record (shell-design §10.5). When it does, PREVIEW
/// says "Camera and microphone on · Not recording" (S3.4).
pub const MICROPHONE_ON_IN_PREVIEW: bool = false;

/// The square's side, in pixels: the centre of the 640 × 480 mode front cameras offer, H.264 level
/// 3.0.
pub const EDGE: u32 = 480;

/// The video bitrate: 2 000 000 × (230 400 ÷ 921 600), the profile's step 3 at the 30 fps target.
pub const VIDEO_BITRATE: u32 = 500_000;

/// The highest frame rate written; a camera delivering fewer is kept as it is.
pub const MAX_FRAME_RATE: f64 = 30.0;

/// The profile's longest gap between keyframes, in seconds — `fc_text::transcode::KEYFRAME_SECONDS`,
/// the web's.
pub const KEYFRAME_SECONDS: f64 = fc_text::transcode::KEYFRAME_SECONDS;

/// The recorder's circle, in logical pixels (S3.3).
pub const LARGEST_CIRCLE: f64 = 320.0;
pub const SMALLEST_CIRCLE: f64 = 160.0;

/// The ring just OUTSIDE the circle, so it never covers a face (S3.3).
pub const RING_WIDTH: f64 = 4.0;

/// PREVIEW turns the camera off after this long with no control used (S1.1) —
/// `fc_text::record::PREVIEW_IDLE_CLOSE_MS`.
pub const PREVIEW_IDLE_MS: i64 = record::PREVIEW_IDLE_CLOSE_MS as i64;

/// How long a picture may stay near-black before PREVIEW asks whether the camera is covered (S3.6).
pub const BLACK_CHECK_MS: i64 = 2_000;

/// A frame whose mean luma (0 to 1) is at or under this is near-black — a privacy shutter, not a
/// dark room.
pub const BLACK_LUMA: f64 = 0.06;

// ---- whether there is a way in at all (S1.2, S1.4) --------------------------------------------

/// S1.2's **round available**: this build records (`recording_enabled`, [`RECORDING_ENABLED`]), the
/// server named the limits, and the machine has a camera.
pub fn available(
    recording_enabled: bool,
    limits: Option<&RoundVideoLimits>,
    has_camera: bool,
) -> bool {
    recording_enabled && limits.is_some() && has_camera
}

/// What the recorder hears of what ends a voice recording (S4): a call, a hidden or minimised
/// window and a lock as themselves; leaving the chat — which the recorder covering the window
/// allows only from outside it — as a hidden window, stopping a take into REVIEW and keeping it;
/// the app going or the session ending as a sign-out, which deletes (a real close over a clip has
/// asked before it gets here). `None` for the person's own endings, which are not the recorder's
/// business.
pub fn interruption_of(why: RecordingEnd) -> Option<RecorderInterruption> {
    match why {
        RecordingEnd::Call => Some(RecorderInterruption::Call),
        RecordingEnd::WindowHidden | RecordingEnd::LeftChat => {
            Some(RecorderInterruption::WindowHidden)
        }
        RecordingEnd::SessionLocked => Some(RecorderInterruption::SessionLocked),
        RecordingEnd::WindowClosed | RecordingEnd::SignedOut => {
            Some(RecorderInterruption::SignedOut)
        }
        _ => None,
    }
}

/// Where a take stops on its own: `max_round_video_ms` − 500, 59.5 s at 60 000
/// (`fc_text::record::round_cap_ms`).
pub fn cap_ms(limits: &RoundVideoLimits) -> i64 {
    to_ms(record::round_cap_ms(from_ms(limits.max_ms)))
}

/// Where "10 seconds left" is shown and said: `max_round_video_ms` − 10 000, 50 s at 60 000
/// (`fc_text::record::round_warning_ms`).
pub fn warning_ms(limits: &RoundVideoLimits) -> i64 {
    to_ms(record::round_warning_ms(from_ms(limits.max_ms)))
}

// ---- the picture ------------------------------------------------------------------------------

/// The largest square in the middle of a frame (the profile: a centre crop, never a squash), its
/// corner and side even. `None` for a frame with no pixels.
pub fn centre_square(width: u32, height: u32) -> Option<SquareCrop> {
    let shortest = width.min(height);
    if shortest == 0 {
        return None;
    }
    let side = if shortest >= 2 {
        shortest & !1
    } else {
        shortest
    };
    Some(SquareCrop {
        x: ((width - side) / 2) & !1,
        y: ((height - side) / 2) & !1,
        side,
    })
}

/// The recorder's circle for a pane (S3.3): D = min(320, width − 48, height − 240 − the reply
/// banner), at least 160, with the controls along the bottom; in a pane shorter than 480 the
/// controls stand in a column at the trailing edge and D = min(320, height − 96, width − 200), at
/// least 160.
pub fn fit(pane_width: f64, pane_height: f64, banner_height: f64) -> RecorderFit {
    if !pane_width.is_finite()
        || !pane_height.is_finite()
        || pane_width <= 0.0
        || pane_height <= 0.0
    {
        return RecorderFit {
            diameter: SMALLEST_CIRCLE,
            layout: RecorderLayout::Row,
        };
    }
    let banner = if banner_height.is_finite() {
        banner_height.max(0.0)
    } else {
        0.0
    };
    if pane_height < 480.0 {
        let column = LARGEST_CIRCLE.min((pane_height - 96.0).min(pane_width - 200.0));
        return RecorderFit {
            diameter: SMALLEST_CIRCLE.max(column).floor(),
            layout: RecorderLayout::Column,
        };
    }
    let row = LARGEST_CIRCLE.min((pane_width - 48.0).min(pane_height - 240.0 - banner));
    RecorderFit {
        diameter: SMALLEST_CIRCLE.max(row).floor(),
        layout: RecorderLayout::Row,
    }
}

/// A point on the ring, clockwise from 12 o'clock (S3.4): where the arc of `fraction` ends, on a
/// circle of `radius` about (`centre`, `centre`).
pub fn ring_point(fraction: f64, radius: f64, centre: f64) -> (f64, f64) {
    let part = if fraction.is_finite() { fraction } else { 0.0 };
    let turn = 2.0 * PI * part.clamp(0.0, 1.0);
    (centre + radius * turn.sin(), centre - radius * turn.cos())
}

/// Whether a mode's preview draws at all. On Windows every format but the three WinUI's frame
/// source left blank (#9756); on Linux every format — the pipeline converts or decodes what the
/// camera delivers, and a camera whose caps will not link is [`CameraTrouble::Unsupported`],
/// reported by the shell. An empty format is a mode nobody named, and does not draw.
pub fn preview_draws(subtype: &str) -> bool {
    !subtype.is_empty()
}

/// The mode to capture in: one whose preview draws ([`preview_draws`]), of `aspect` where one is
/// asked for and offered; a short side of 480 or more where there is one — the smallest such, so
/// nothing is upscaled and nothing larger than needed is moved — and otherwise the largest; 24 fps
/// or more where offered; and the rate nearest 30 without going over, then the nearest over it.
/// `None` when nothing draws.
pub fn pick_mode(modes: &[CaptureMode], aspect: Option<f64>) -> Option<CaptureMode> {
    let mut drawing: Vec<&CaptureMode> = modes
        .iter()
        .filter(|mode| mode.width > 0 && mode.height > 0 && preview_draws(&mode.subtype))
        .collect();
    if drawing.is_empty() {
        return None;
    }
    if let Some(wanted) = aspect {
        let same: Vec<&CaptureMode> = drawing
            .iter()
            .copied()
            .filter(|mode| (f64::from(mode.width) / f64::from(mode.height) - wanted).abs() < 0.01)
            .collect();
        if !same.is_empty() {
            drawing = same;
        }
    }
    let enough: Vec<&CaptureMode> = drawing
        .iter()
        .copied()
        .filter(|mode| mode.width.min(mode.height) >= EDGE)
        .collect();
    let mut pool = if enough.is_empty() {
        drawing
    } else {
        enough.clone()
    };
    let smooth: Vec<&CaptureMode> = pool
        .iter()
        .copied()
        .filter(|mode| mode.frame_rate >= 24.0)
        .collect();
    if !smooth.is_empty() {
        pool = smooth;
    }
    let ascending = !enough.is_empty();
    // LINQ's OrderBy/ThenBy is a STABLE sort, and so is `sort_by`: equal modes keep the camera's
    // order.
    pool.sort_by(|a, b| {
        let area = |mode: &CaptureMode| u64::from(mode.width) * u64::from(mode.height);
        let by_size = if ascending {
            area(a).cmp(&area(b))
        } else {
            area(b).cmp(&area(a))
        };
        let over = |mode: &CaptureMode| u8::from(mode.frame_rate > MAX_FRAME_RATE + 0.01);
        by_size.then(over(a).cmp(&over(b))).then_with(|| {
            dotnet_compare(
                (MAX_FRAME_RATE - a.frame_rate).abs(),
                (MAX_FRAME_RATE - b.frame_rate).abs(),
            )
        })
    });
    pool.first().map(|mode| (*mode).clone())
}

/// The camera to open (S3.5): the one chosen before on this device while it is still there, else
/// the first on the front panel, else the first the shell lists. `None` with no camera.
pub fn pick_camera<'a>(
    cameras: &'a [CameraChoice],
    remembered: Option<&str>,
) -> Option<&'a CameraChoice> {
    cameras
        .iter()
        .find(|camera| remembered.is_some_and(|id| camera.id == id))
        .or_else(|| {
            cameras
                .iter()
                .find(|camera| camera.panel == CameraPanel::Front)
        })
        .or_else(|| cameras.first())
}

/// "Choose camera" is offered only with more than one to choose from (S3.4, S3.5).
pub fn offers_camera_choice(cameras: usize) -> bool {
    cameras > 1
}

// ---- the encode -------------------------------------------------------------------------------

/// The square's encodes, in the order to ask for them — the profile's H.264 High at 480 × 480,
/// 500 000 bit/s and the camera's own rate up to 30 fps, upright, SDR, with AAC-LC mono at 64 000
/// (the `media_encoding` stub's ONE voice-note encoding: no Media Foundation 96 000 on Linux); then
/// Main with it, "Main where an encoder offers nothing else". A take with no sound track — a
/// microphone that gave nothing — is asked for without one: an encode does not invent silence.
///
/// Each asks for a keyframe at least every [`KEYFRAME_SECONDS`] — at the rate encoded, in whole
/// frames (`x264enc key-int-max`, `openh264enc gop-size`). Windows' encoder could refuse its output
/// type for the spacing, so the last way is asked once more without it — kept here, where it is
/// the encoder's own default spacing, though no GStreamer encoder refuses the property.
///
/// On Linux these are the ways to BUILD the recording pipeline: the shell takes the first whose
/// encoder exists and links (`x264enc` for High, `openh264enc` for Main), before Record.
pub fn encodes(rate_numerator: u32, rate_denominator: u32, has_audio: bool) -> Vec<VideoEncoding> {
    let rate = if rate_numerator > 0 && rate_denominator > 0 {
        f64::from(rate_numerator) / f64::from(rate_denominator)
    } else {
        MAX_FRAME_RATE
    };
    let (numerator, denominator) = media_encoding::frame_rate_ratio(
        rate.min(MAX_FRAME_RATE),
        rate_numerator,
        rate_denominator,
    );
    let spacing = (KEYFRAME_SECONDS * f64::from(numerator) / f64::from(denominator))
        .floor()
        .max(1.0) as u32;
    let first = VideoEncoding {
        width: EDGE,
        height: EDGE,
        rotation: 0,
        bitrate: VIDEO_BITRATE,
        frame_rate_numerator: numerator,
        frame_rate_denominator: denominator,
        profile: H264Profile::High,
        audio: None,
        to_sdr: false,
        keep_audio: false,
        keyframe_spacing: Some(spacing),
    };
    let mut attempts = if has_audio {
        let audio = media_encoding::voice_note_attempts();
        let mut asked: Vec<VideoEncoding> = audio
            .iter()
            .map(|aac| VideoEncoding {
                audio: Some(*aac),
                ..first
            })
            .collect();
        asked.push(VideoEncoding {
            profile: H264Profile::Main,
            audio: audio.last().copied(),
            ..first
        });
        asked
    } else {
        vec![
            first,
            VideoEncoding {
                profile: H264Profile::Main,
                ..first
            },
        ]
    };
    if let Some(last) = attempts.last().copied() {
        attempts.push(VideoEncoding {
            keyframe_spacing: None,
            ..last
        });
    }
    attempts
}

/// Whether the square came out as asked (Windows' trial T3 checks the same): H.264, exactly 480 ×
/// 480, no turn, its sound track there exactly when one was asked for and AAC, and no faster than
/// asked.
pub fn came_out(
    asked: &VideoEncoding,
    video_codec: &str,
    width: u32,
    height: u32,
    rotation: u32,
    audio_codec: Option<&str>,
    frame_rate: Option<f64>,
) -> bool {
    video_codec == "h264"
        && media_encoding::matches(asked, width, height, rotation)
        && match asked.audio {
            None => audio_codec.is_none(),
            Some(_) => audio_codec == Some("aac"),
        }
        && media_encoding::frame_rate_came_out(asked, frame_rate)
}

/// What a finished take is sent as, from what the shell read back of it — the window's half of
/// Windows' `MakeClipAsync`, made testable here: the square goes when it came out as asked
/// ([`came_out`]) with its index in front (the profile's `moov` first — Windows' `RoundAsync`
/// refused one that could not be moved), otherwise the take goes as a regular video ([`plan`]). The
/// length is the file's own where it could be read, else the recorder's clock (`elapsed_ms`).
/// Answers the plan and the length to stage and to hand `RoundRecorder::clip_finished`.
pub fn plan_take(
    asked: &VideoEncoding,
    reading: Option<&TakeReading>,
    elapsed_ms: i64,
    limits: &RoundVideoLimits,
    say: &dyn StringCatalog,
) -> (RoundSendPlan, i64) {
    let square = reading.filter(|read| {
        read.moov_first
            && came_out(
                asked,
                &read.video_codec,
                read.width,
                read.height,
                read.rotation,
                read.audio_codec.as_deref(),
                read.frame_rate,
            )
    });
    let duration = reading
        .and_then(|read| read.duration_ms)
        .unwrap_or(elapsed_ms.max(0));
    (
        plan(square.map(|read| read.bytes), Some(duration), limits, say),
        duration,
    )
}

// ---- what is refused, and what the camera says (S3.2, S3.6) -----------------------------------

/// What was refused already, read BEFORE anything is opened (S3.2): either one refused opens the
/// recorder straight to it and asks the desktop nothing. The microphone is named first: with it
/// off, the camera refusal's way out — a voice message instead — would lead to the same refusal.
pub fn refused_before(
    camera: CapabilityAccess,
    microphone: CapabilityAccess,
) -> Option<RecorderRefusal> {
    if denied(microphone) {
        Some(RecorderRefusal::Microphone)
    } else if denied(camera) {
        Some(RecorderRefusal::Camera)
    } else {
        None
    }
}

/// What a refused open was refused for, read once the desktop has answered: the one now denied, the
/// microphone first; the camera when it says neither — the camera is the one this recorder is for.
pub fn refused_after(camera: CapabilityAccess, microphone: CapabilityAccess) -> RecorderRefusal {
    refused_before(camera, microphone).unwrap_or(RecorderRefusal::Camera)
}

/// Whether the desktop will ask the person when the camera and microphone are opened — and so the
/// neutral "needs" line shows (S3.2).
pub fn will_ask(camera: CapabilityAccess, microphone: CapabilityAccess) -> bool {
    camera == CapabilityAccess::UserPromptRequired
        || microphone == CapabilityAccess::UserPromptRequired
}

fn denied(access: CapabilityAccess) -> bool {
    matches!(
        access,
        CapabilityAccess::DeniedByUser | CapabilityAccess::DeniedBySystem
    )
}

/// A refusal's sentence, and the settings page that can mend it (S3.2) — offered unless the app
/// itself is at fault (`NotDeclaredByApp`: on Linux an AppArmor profile that denies the device),
/// which no switch in Settings mends. The sentence is Windows' and fits GNOME, whose settings app
/// is called Settings.
pub fn refusal(
    refused: RecorderRefusal,
    access: CapabilityAccess,
    say: &dyn StringCatalog,
) -> (String, Option<SettingsPage>) {
    let offered =
        |page: SettingsPage| (access != CapabilityAccess::NotDeclaredByApp).then_some(page);
    match refused {
        RecorderRefusal::Camera => (
            say.get("Family needs permission to use your camera. Turn it on in Settings.")
                .to_string(),
            offered(SettingsPage::Camera),
        ),
        RecorderRefusal::Microphone => (
            say.get("Family needs permission to use your microphone. Turn it on in Settings.")
                .to_string(),
            offered(SettingsPage::Microphone),
        ),
    }
}

/// GLib's error domains a GStreamer pipeline posts on its bus, as `g_quark_to_string` spells them.
pub const RESOURCE_ERROR_DOMAIN: &str = "gst-resource-error-quark";
pub const STREAM_ERROR_DOMAIN: &str = "gst-stream-error-quark";
pub const CORE_ERROR_DOMAIN: &str = "gst-core-error-quark";

/// What a GStreamer error says about the camera (S3.6) — Windows read Media Foundation's HRESULT;
/// Linux reads the bus `Error` message's GLib domain and code (`gst::ResourceError`, …):
/// - `RESOURCE_ERROR_BUSY` (4): another app holds the device (`v4l2src`'s "Device is busy") →
///   [`CameraTrouble::InUse`];
/// - `RESOURCE_ERROR_NOT_FOUND` (3): the device is not there → [`CameraTrouble::Missing`] (the
///   device monitor's `DeviceRemoved` for the open camera is `Missing` too — the shell says so
///   directly);
/// - `CORE_ERROR_NEGOTIATION` (7), `CORE_ERROR_CAPS` (10), `STREAM_ERROR_NOT_IMPLEMENTED` (3),
///   `STREAM_ERROR_WRONG_TYPE` (5), `STREAM_ERROR_CODEC_NOT_FOUND` (6), `STREAM_ERROR_FORMAT` (11):
///   the camera's caps will not link → [`CameraTrouble::Unsupported`];
/// - anything else → [`CameraTrouble::Failed`].
///
/// UNCONFIRMED on a real desktop (shell-design §C): what `pipewiresrc` posts for a camera another
/// app holds through PipeWire.
pub fn trouble(domain: &str, code: i32) -> CameraTrouble {
    match (domain, code) {
        (RESOURCE_ERROR_DOMAIN, 4) => CameraTrouble::InUse,
        (RESOURCE_ERROR_DOMAIN, 3) => CameraTrouble::Missing,
        (CORE_ERROR_DOMAIN, 7 | 10) | (STREAM_ERROR_DOMAIN, 3 | 5 | 6 | 11) => {
            CameraTrouble::Unsupported
        }
        _ => CameraTrouble::Failed,
    }
}

/// What PREVIEW says in place of the picture for a camera that cannot be used (S3.6).
pub fn trouble_sentence(trouble: CameraTrouble, say: &dyn StringCatalog) -> String {
    match trouble {
        CameraTrouble::InUse => say
            .get("The camera is being used by another app.")
            .to_string(),
        CameraTrouble::Unsupported => say
            .get("Video messages can't be recorded with this camera.")
            .to_string(),
        CameraTrouble::Missing | CameraTrouble::Failed => {
            say.get("Something went wrong. Try again.").to_string()
        }
    }
}

/// A frame is near-black (S3.6): a privacy shutter or a camera switched off, never merely a dark
/// room.
pub fn looks_black(mean_luma: f64) -> bool {
    mean_luma.is_finite() && mean_luma <= BLACK_LUMA
}

// ---- what is sent (S3.6, "On the wire") -------------------------------------------------------

/// What a finished take is sent as: a video message when the square came out, it is 1 ms to
/// `max_round_video_ms` long and within `max_round_video_bytes`; the take as a regular video —
/// "Couldn't make it round." — when the square did not come out or the length cannot be sent round;
/// the square itself as a regular video — "Too big for a video message." — over the byte ceiling
/// (Rule C: an optimisation never turns a send that would have worked into one that does not).
pub fn plan(
    square_bytes: Option<i64>,
    duration_ms: Option<i64>,
    limits: &RoundVideoLimits,
    say: &dyn StringCatalog,
) -> RoundSendPlan {
    let square = square_bytes.filter(|bytes| *bytes > 0);
    let length = duration_ms.filter(|length| (1..=limits.max_ms).contains(length));
    match (square, length) {
        (Some(bytes), Some(_)) if bytes > limits.max_bytes => RoundSendPlan::new(
            false,
            true,
            Some(say.get("Too big for a video message. It will be sent as a regular video.")),
        ),
        (Some(_), Some(_)) => RoundSendPlan::new(true, true, None),
        _ => RoundSendPlan::new(
            false,
            false,
            Some(say.get("Couldn't make it round. It will be sent as a regular video.")),
        ),
    }
}

/// The square as the outbox stages it: `kind=video`, `video/mp4`, 480 × 480, its length, and its
/// 480 × 480 poster — which the upload sends with `PUT /attachments/{id}/preview`.
pub fn staged(square: Vec<u8>, duration_ms: i32, poster: Option<Vec<u8>>) -> StagedMedia {
    StagedMedia {
        width: Some(EDGE as i32),
        height: Some(EDGE as i32),
        duration_ms: Some(duration_ms),
        preview: poster,
        ..StagedMedia::new("video", "video/mp4", square)
    }
}

/// "Video message · 0:23" — REVIEW's status line (S3.4).
pub fn review_line(duration_ms: i64, say: &dyn StringCatalog) -> String {
    let length = time_label(duration_ms.max(0) as f64 / 1000.0);
    say.format("Video message · %@", &[&length])
}

// ---- the video button's guard and the covered camera ------------------------------------------

/// The video button's own guard (S1.1): it ignores activation for 600 ms after it appears, because
/// it appears beside the slot the moment a text Send empties the field, and a second click that
/// drifts left must not turn the camera on.
#[derive(Debug, Clone, Default)]
pub struct DoorGuard {
    shown_at: Option<i64>,
    shown: bool,
}

impl DoorGuard {
    pub fn new() -> Self {
        DoorGuard::default()
    }

    /// The button is drawn (`true`) or taken away (`false`) at `now`; only the moment it APPEARS
    /// arms the guard.
    pub fn showing(&mut self, showing: bool, now: i64) {
        if showing && !self.shown {
            self.shown_at = Some(now);
        }
        self.shown = showing;
    }

    /// Whether a click at `now` may open the recorder.
    pub fn accepts(&self, now: i64) -> bool {
        self.shown
            && self
                .shown_at
                .is_some_and(|at| now.saturating_sub(at) >= record::ACTIVATION_GUARD_MS as i64)
    }
}

/// Whether the first two seconds of PREVIEW stayed near-black (S3.6) — decided once, from the
/// frames that came.
#[derive(Debug, Clone, Default)]
pub struct PictureCheck {
    first: Option<i64>,
    saw_light: bool,
    black: Option<bool>,
}

impl PictureCheck {
    pub fn new() -> Self {
        PictureCheck::default()
    }

    /// The verdict, once made: `Some(true)` for a picture that stayed black.
    pub fn black(&self) -> Option<bool> {
        self.black
    }

    /// One frame's mean luma at `now`. Answers `true` exactly once, when two seconds of frames were
    /// all near-black; a single lit frame settles it the other way.
    pub fn frame(&mut self, now: i64, mean_luma: f64) -> bool {
        if self.black.is_some() {
            return false;
        }
        let first = *self.first.get_or_insert(now);
        self.saw_light |= !looks_black(mean_luma);
        if self.saw_light {
            self.black = Some(false);
            return false;
        }
        if now.saturating_sub(first) < BLACK_CHECK_MS {
            return false;
        }
        self.black = Some(true);
        true
    }
}

// ---- inside -----------------------------------------------------------------------------------

/// .NET's `Comparer<double>.Default`, which LINQ's `ThenBy` uses: NaN before every number and equal
/// to itself.
fn dotnet_compare(a: f64, b: f64) -> Ordering {
    match (a.is_nan(), b.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
    }
}

/// A server's milliseconds as `fc_text::record` takes them: a negative one is 0.
fn from_ms(ms: i64) -> u64 {
    ms.max(0) as u64
}

fn to_ms(ms: u64) -> i64 {
    i64::try_from(ms).unwrap_or(i64::MAX)
}
