//! `RoundVideoRulesTests`: recording a video message, the rules that need no camera
//! (docs/audio-video-messages-2026-10-04.md, Phase 3; S1.2, S3.3, S3.5, S3.6; "The recording
//! profile for a round video"): the switch, the square, the circle's size, the camera and its mode,
//! the encodes and their check, what a refusal or a busy camera says, and what a finished take is
//! sent as.
//!
//! Adapted to Linux, each saying so where it is: the switch is OFF until the real-desktop camera
//! lines pass (shell-design §10.5); no format leaves the preview blank (WinUI's #9756 is not
//! GTK's); the camera's trouble is read from a GStreamer error, not an HRESULT; a refusal names a
//! settings page, not an `ms-settings:` URI; and there is ONE audio encoding (no Media Foundation
//! 96 000). A [`Theory`] is one `#[test]` looping over its rows. The last tests are Linux's own:
//! `plan_take`, which holds what Windows' window did with a finished take, and `SquareCrop::videocrop`.

mod common;

use fc_core::protocol::RoundVideoLimits;
use fc_logic::media_encoding::{AacEncoding, H264Profile, VideoEncoding};
use fc_logic::not_sent::RecordingEnd;
use fc_logic::round_recorder::RecorderInterruption;
use fc_logic::round_video_rules::{
    self as rules, CameraChoice, CameraPanel, CameraTrouble, CaptureMode, DoorGuard, PictureCheck,
    RecorderFit, RecorderLayout, RecorderRefusal, RoundSendPlan, SettingsPage, SquareCrop,
    TakeReading,
};
use fc_logic::voice_notes::CapabilityAccess;
use fc_text::record::{self, DoorInputs, Recording, SlotInputs};

const LIMITS: RoundVideoLimits = RoundVideoLimits {
    max_ms: 60_000,
    max_bytes: 12_582_912,
};

fn limits(max_ms: i64, max_bytes: i64) -> RoundVideoLimits {
    RoundVideoLimits { max_ms, max_bytes }
}

// ---- the switch (Blocked 1, Decision 40) ------------------------------------------------------

/// The C#'s `RecordingIsSwitchedOn`, adapted: Windows switched it on (2026-10-21) ahead of its
/// trials; Linux keeps it OFF until the real-desktop checklist's camera lines pass (shell-design
/// §10.5, §C). Switching it on is a deliberate act, and this test is what says so — with it off no
/// way in is drawn, whatever the server and the machine offer.
#[test]
fn recording_is_switched_off_until_the_camera_lines_pass() {
    const { assert!(!rules::RECORDING_ENABLED) };
    assert!(!rules::available(
        rules::RECORDING_ENABLED,
        Some(&LIMITS),
        true
    ));
}

/// S1.2's round available: this build records, the server named the limits, and there is a camera
/// — all three.
#[test]
fn a_video_message_is_available_only_with_all_three() {
    assert!(rules::available(true, Some(&LIMITS), true));
    assert!(!rules::available(false, Some(&LIMITS), true));
    assert!(!rules::available(true, None, true));
    assert!(!rules::available(true, Some(&LIMITS), false));
}

/// With the switch on the video button is shown in an empty field; with it off it stays hidden
/// (Decision 40). Adapted: the switch is passed in (`true`, then [`rules::RECORDING_ENABLED`] —
/// off on Linux), since the constant itself is off.
#[test]
fn the_video_button_follows_the_switch() {
    let empty = SlotInputs {
        recorder_open: false,
        recording: Recording::None,
        editing: false,
        draft_blank: true,
        staged: false,
        assistant_chat: false,
        can_record: true,
        call: false,
        busy: false,
        not_sent: false,
    };
    let on = DoorInputs {
        slot: empty,
        family_or_direct_chat: true,
        server_offers_round: true,
        has_camera: true,
        encoder_probe_passes: true,
        records_round_video: true,
    };
    assert_eq!(record::Door::Shown, record::video_door(&on));
    assert_eq!(
        record::Door::Hidden,
        record::video_door(&DoorInputs {
            records_round_video: false,
            ..on
        })
    );
    assert_eq!(
        record::Door::Hidden,
        record::video_door(&DoorInputs {
            records_round_video: rules::RECORDING_ENABLED,
            ..on
        })
    );
}

/// 59.5 s and 50 s at the server's 60 000 — the shared module's arithmetic, from the discovery key.
#[test]
fn the_cap_and_the_warning_come_from_the_servers_limit() {
    assert_eq!(59_500, rules::cap_ms(&LIMITS));
    assert_eq!(50_000, rules::warning_ms(&LIMITS));
    assert_eq!(29_500, rules::cap_ms(&limits(30_000, 1)));
    assert_eq!(20_000, rules::warning_ms(&limits(30_000, 1)));
}

/// The profile's numbers (the plan's "recording profile").
#[test]
fn the_profile_is_four_eighty_at_five_hundred_kilobits() {
    assert_eq!(480u32, rules::EDGE);
    assert_eq!(500_000u32, rules::VIDEO_BITRATE);
    assert_eq!(30.0, rules::MAX_FRAME_RATE);
    assert_eq!(60_000, rules::PREVIEW_IDLE_MS);
    assert_eq!(2_000, rules::BLACK_CHECK_MS);
    assert_eq!(4.0, rules::RING_WIDTH);
}

// ---- the square -------------------------------------------------------------------------------

/// The largest square in the middle — a crop, never a squash — its corner and side even.
#[test]
fn the_square_is_cut_from_the_middle() {
    for (width, height, x, y, side) in [
        (640, 480, 80, 0, 480),
        (1280, 720, 280, 0, 720),
        (1920, 1080, 420, 0, 1080),
        (480, 640, 0, 80, 480),
        (480, 480, 0, 0, 480),
        (641, 481, 80, 0, 480),
        (645, 480, 82, 0, 480),
        (642, 480, 80, 0, 480),
        (480, 642, 0, 80, 480),
        (1, 1, 0, 0, 1),
    ] {
        assert_eq!(
            Some(SquareCrop { x, y, side }),
            rules::centre_square(width, height),
            "{width}x{height}"
        );
    }
}

#[test]
fn a_frame_with_no_pixels_has_no_square() {
    for (width, height) in [(0, 480), (640, 0)] {
        assert_eq!(None, rules::centre_square(width, height));
    }
}

// ---- the circle (S3.3) ------------------------------------------------------------------------

/// D = min(320, width − 48, height − 240 − the banner), at least 160, controls along the bottom.
#[test]
fn the_circle_fits_the_pane() {
    for (width, height, banner, diameter) in [
        (1000.0, 900.0, 0.0, 320.0),
        (300.0, 900.0, 0.0, 252.0),
        (1000.0, 520.0, 0.0, 280.0),
        (1000.0, 520.0, 40.0, 240.0),
        (150.0, 900.0, 0.0, 160.0),
        (1000.0, 480.0, 100.0, 160.0),
    ] {
        assert_eq!(
            RecorderFit {
                diameter,
                layout: RecorderLayout::Row
            },
            rules::fit(width, height, banner)
        );
    }
}

/// Shorter than 480: the controls in a column at the trailing edge, D = min(320, height − 96,
/// width − 200), at least 160.
#[test]
fn a_short_pane_stands_the_controls_in_a_column() {
    for (width, height, banner, diameter) in [
        (1000.0, 479.0, 0.0, 320.0),
        (1000.0, 400.0, 0.0, 304.0),
        (450.0, 400.0, 0.0, 250.0),
        (300.0, 300.0, 0.0, 160.0),
        (1000.0, 400.0, 300.0, 304.0),
    ] {
        assert_eq!(
            RecorderFit {
                diameter,
                layout: RecorderLayout::Column
            },
            rules::fit(width, height, banner)
        );
    }
}

#[test]
fn a_pane_not_yet_laid_out_gets_the_smallest_circle() {
    let smallest = RecorderFit {
        diameter: 160.0,
        layout: RecorderLayout::Row,
    };
    assert_eq!(smallest, rules::fit(0.0, 0.0, 0.0));
    assert_eq!(smallest, rules::fit(f64::NAN, 900.0, 0.0));
}

/// The ring fills clockwise from 12 o'clock.
#[test]
fn the_ring_fills_clockwise_from_twelve() {
    assert_point((100.0, 0.0), rules::ring_point(0.0, 100.0, 100.0));
    assert_point((200.0, 100.0), rules::ring_point(0.25, 100.0, 100.0));
    assert_point((100.0, 200.0), rules::ring_point(0.5, 100.0, 100.0));
    assert_point((0.0, 100.0), rules::ring_point(0.75, 100.0, 100.0));
    assert_point((100.0, 0.0), rules::ring_point(2.0, 100.0, 100.0));
    assert_point((100.0, 0.0), rules::ring_point(f64::NAN, 100.0, 100.0));
}

/// xUnit's `Assert.Equal(expected, actual, 6)`: equal when rounded to six decimals.
fn assert_point(expected: (f64, f64), actual: (f64, f64)) {
    let six = |value: f64| (value * 1e6).round_ties_even() / 1e6;
    assert_eq!(six(expected.0), six(actual.0), "x of {actual:?}");
    assert_eq!(six(expected.1), six(actual.1), "y of {actual:?}");
}

// ---- the camera and its mode (S3.5, S3.6) -----------------------------------------------------

/// The one chosen before while it is there, else the front panel, else the first.
#[test]
fn the_front_camera_unless_another_was_chosen() {
    let back = CameraChoice::new("b", "Rear", CameraPanel::Back);
    let usb = CameraChoice::new("u", "USB", CameraPanel::Unknown);
    let front = CameraChoice::new("f", "Front", CameraPanel::Front);
    let all = [back.clone(), usb.clone(), front.clone()];
    assert_eq!(Some(&front), rules::pick_camera(&all, None));
    assert_eq!(Some(&usb), rules::pick_camera(&all, Some("u")));
    assert_eq!(Some(&front), rules::pick_camera(&all, Some("gone")));
    assert_eq!(
        Some(&back),
        rules::pick_camera(&[back.clone(), usb.clone()], None)
    );
    assert_eq!(None, rules::pick_camera(&[], Some("u")));
    assert!(!rules::offers_camera_choice(1));
    assert!(rules::offers_camera_choice(2));
}

fn mode(width: u32, height: u32, rate: f64) -> CaptureMode {
    CaptureMode::new(width, height, rate)
}

/// The smallest mode with a short side of 480 or more, at 24 fps or more, nearest 30 without going
/// over.
#[test]
fn the_mode_is_the_smallest_that_needs_no_upscale() {
    let modes = [
        mode(320, 240, 30.0),
        mode(640, 480, 15.0),
        mode(640, 480, 30.0),
        mode(640, 480, 60.0),
        mode(1280, 720, 30.0),
        mode(1920, 1080, 30.0),
    ];
    assert_eq!(Some(mode(640, 480, 30.0)), rules::pick_mode(&modes, None));
    // Only 60 at that size: still the size, the nearest rate over 30.
    assert_eq!(
        Some(mode(640, 480, 60.0)),
        rules::pick_mode(
            &[
                mode(640, 480, 60.0),
                mode(1280, 720, 60.0),
                mode(640, 480, 90.0)
            ],
            None
        )
    );
    // Nothing reaches 480: the largest there is.
    assert_eq!(
        Some(mode(352, 288, 30.0)),
        rules::pick_mode(
            &[
                mode(320, 240, 30.0),
                mode(352, 288, 30.0),
                mode(160, 120, 30.0)
            ],
            None
        )
    );
    // A slow camera is kept slow rather than refused.
    assert_eq!(
        Some(mode(640, 480, 15.0)),
        rules::pick_mode(&[mode(640, 480, 15.0), mode(320, 240, 30.0)], None)
    );
    // 24 fps or more beats a smaller size at less.
    assert_eq!(
        Some(mode(1280, 720, 30.0)),
        rules::pick_mode(&[mode(640, 480, 15.0), mode(1280, 720, 30.0)], None)
    );
}

/// The record stream in the preview's shape where it is offered, so the square cut is the square
/// seen.
#[test]
fn the_record_mode_keeps_the_previews_shape() {
    let modes = [
        mode(640, 480, 30.0),
        mode(848, 480, 30.0),
        mode(1280, 720, 30.0),
    ];
    assert_eq!(
        Some(mode(848, 480, 30.0)),
        rules::pick_mode(&modes, Some(848.0 / 480.0))
    );
    assert_eq!(
        Some(mode(640, 480, 30.0)),
        rules::pick_mode(&modes, Some(4.0 / 3.0))
    );
    assert_eq!(
        Some(mode(640, 480, 30.0)),
        rules::pick_mode(&modes, Some(1.0))
    );
}

/// The C#'s `ABlankPreviewFormatIsNeverChosen`, adapted: WinUI's frame-source preview stayed blank
/// for RGB24, UYVY and I420 (#9756), so Windows never chose them. The Linux pipeline converts every
/// raw format and decodes MJPEG — I420 is its own format — so on Linux every named format draws,
/// and only a mode with no pixels, or none at all, is no mode.
#[test]
fn a_blank_preview_format_is_never_chosen() {
    assert!(rules::preview_draws("RGB24"));
    assert!(rules::preview_draws("uyvy"));
    assert!(rules::preview_draws("I420"));
    assert!(rules::preview_draws("NV12"));
    assert!(rules::preview_draws("YUY2"));
    assert!(rules::preview_draws("MJPG"));
    assert!(!rules::preview_draws(""));
    // Windows chose the MJPG mode for the RGB24 one beside it; here the size decides, as for any
    // two modes — the smaller that reaches 480.
    assert_eq!(
        Some(mode(640, 480, 30.0).with_subtype("RGB24")),
        rules::pick_mode(
            &[
                mode(640, 480, 30.0).with_subtype("RGB24"),
                mode(1280, 720, 30.0).with_subtype("MJPG")
            ],
            None
        )
    );
    assert_eq!(
        Some(mode(640, 480, 30.0).with_subtype("UYVY")),
        rules::pick_mode(
            &[
                mode(640, 480, 30.0).with_subtype("UYVY"),
                mode(1280, 720, 30.0).with_subtype("I420")
            ],
            None
        )
    );
    assert_eq!(None, rules::pick_mode(&[], None));
    assert_eq!(None, rules::pick_mode(&[mode(0, 480, 30.0)], None));
    assert_eq!(
        None,
        rules::pick_mode(&[mode(640, 480, 30.0).with_subtype("")], None)
    );
}

// ---- the encode (the profile) -----------------------------------------------------------------

/// H.264 High at 480 × 480, 500 000 bit/s, the camera's rate up to 30, upright and SDR, with
/// AAC-LC mono 64 000; then Main with that. Adapted: Windows asked for the encoder's 96 000 after
/// 64 000 (four encodes); GStreamer's `avenc_aac` takes 64 000 as asked, so Linux has three.
#[test]
fn the_encodes_are_the_profiles_in_order() {
    let encodes = rules::encodes(30, 1, true);
    assert_eq!(3, encodes.len());
    for encode in &encodes {
        assert_eq!(
            (480, 480, 0),
            (encode.width, encode.height, encode.rotation)
        );
        assert_eq!(500_000, encode.bitrate);
        assert_eq!(
            (30, 1),
            (encode.frame_rate_numerator, encode.frame_rate_denominator)
        );
        assert!(!encode.to_sdr);
        assert!(!encode.keep_audio);
    }
    assert_eq!(
        vec![H264Profile::High, H264Profile::Main, H264Profile::Main],
        encodes
            .iter()
            .map(|encode| encode.profile)
            .collect::<Vec<_>>()
    );
    let mono = AacEncoding {
        sample_rate: 44_100,
        channels: 1,
        bitrate: 64_000,
    };
    assert_eq!(
        vec![Some(mono); 3],
        encodes
            .iter()
            .map(|encode| encode.audio)
            .collect::<Vec<_>>()
    );
    // A keyframe at least every 2 s — then, once, the last way again with the encoder's own spacing.
    assert_eq!(
        vec![Some(60), Some(60), None],
        encodes
            .iter()
            .map(|encode| encode.keyframe_spacing)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        VideoEncoding {
            keyframe_spacing: None,
            ..encodes[1]
        },
        encodes[2]
    );
}

/// "Keyframes at most every 2 s", in whole frames at the rate encoded — never 0, never more than
/// 2 s.
#[test]
fn keyframes_come_at_least_every_two_seconds() {
    for (numerator, denominator, frames) in [
        (30, 1, 60),
        (60, 1, 60),
        (15, 1, 30),
        (30000, 1001, 59),
        (0, 0, 60),
    ] {
        assert_eq!(
            Some(frames),
            rules::encodes(numerator, denominator, true)[0].keyframe_spacing,
            "{numerator}/{denominator}"
        );
        assert_eq!(
            Some(frames),
            rules::encodes(numerator, denominator, false)[0].keyframe_spacing,
            "{numerator}/{denominator}"
        );
    }
    // The planner's videos are left to the encoder, as before.
    let planned = VideoEncoding {
        width: 640,
        height: 360,
        rotation: 0,
        bitrate: 1_000_000,
        frame_rate_numerator: 30,
        frame_rate_denominator: 1,
        profile: H264Profile::High,
        audio: None,
        to_sdr: false,
        keep_audio: false,
        keyframe_spacing: None,
    };
    assert_eq!(None, planned.keyframe_spacing);
}

/// Never faster than 30; a camera delivering fewer is kept as it is — its own ratio, 30000/1001
/// included.
#[test]
fn the_frame_rate_is_the_cameras_up_to_thirty() {
    for (numerator, denominator, want_numerator, want_denominator) in [
        (60, 1, 30, 1),
        (15, 1, 15, 1),
        (30000, 1001, 30000, 1001),
        (0, 0, 30, 1),
    ] {
        let first = rules::encodes(numerator, denominator, true)[0];
        assert_eq!(
            (want_numerator, want_denominator),
            (first.frame_rate_numerator, first.frame_rate_denominator)
        );
    }
}

/// A take with no sound is asked for without any: an encode does not invent silence.
#[test]
fn a_take_with_no_sound_is_encoded_without() {
    let encodes = rules::encodes(30, 1, false);
    assert_eq!(
        vec![H264Profile::High, H264Profile::Main, H264Profile::Main],
        encodes
            .iter()
            .map(|encode| encode.profile)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        vec![Some(60), Some(60), None],
        encodes
            .iter()
            .map(|encode| encode.keyframe_spacing)
            .collect::<Vec<_>>()
    );
    assert!(encodes.iter().all(|encode| encode.audio.is_none()));
}

/// The square read back: H.264, exactly 480 × 480, upright, AAC where asked, none where not, no
/// faster.
#[test]
fn the_square_is_checked_not_trusted() {
    let asked = rules::encodes(30, 1, true)[0];
    let thirty = Some(30.0);
    assert!(rules::came_out(
        &asked,
        "h264",
        480,
        480,
        0,
        Some("aac"),
        thirty
    ));
    assert!(!rules::came_out(
        &asked,
        "hevc",
        480,
        480,
        0,
        Some("aac"),
        thirty
    ));
    assert!(!rules::came_out(
        &asked,
        "h264",
        640,
        480,
        0,
        Some("aac"),
        thirty
    ));
    assert!(!rules::came_out(
        &asked,
        "h264",
        480,
        480,
        90,
        Some("aac"),
        thirty
    ));
    assert!(!rules::came_out(&asked, "h264", 480, 480, 0, None, thirty));
    assert!(!rules::came_out(
        &asked,
        "h264",
        480,
        480,
        0,
        Some("mp3"),
        thirty
    ));
    assert!(!rules::came_out(
        &asked,
        "h264",
        480,
        480,
        0,
        Some("aac"),
        Some(60.0)
    ));
    let silent = rules::encodes(30, 1, false)[0];
    assert!(rules::came_out(&silent, "h264", 480, 480, 0, None, thirty));
    assert!(!rules::came_out(
        &silent,
        "h264",
        480,
        480,
        0,
        Some("aac"),
        thirty
    ));
}

// ---- refusals and trouble (S3.2, S3.6) --------------------------------------------------------

/// Read before anything opens: either refused opens straight to it — the microphone named first —
/// and asks nothing.
#[test]
fn what_was_refused_is_shown_before_anything_is_asked() {
    use CapabilityAccess::*;
    assert_eq!(None, rules::refused_before(Allowed, Allowed));
    assert_eq!(
        None,
        rules::refused_before(UserPromptRequired, UserPromptRequired)
    );
    assert_eq!(
        Some(RecorderRefusal::Camera),
        rules::refused_before(DeniedByUser, UserPromptRequired)
    );
    assert_eq!(
        Some(RecorderRefusal::Camera),
        rules::refused_before(DeniedBySystem, Allowed)
    );
    assert_eq!(
        Some(RecorderRefusal::Microphone),
        rules::refused_before(Allowed, DeniedByUser)
    );
    assert_eq!(
        Some(RecorderRefusal::Microphone),
        rules::refused_before(DeniedByUser, DeniedBySystem)
    );
    assert!(rules::will_ask(UserPromptRequired, Allowed));
    assert!(rules::will_ask(Allowed, UserPromptRequired));
    assert!(!rules::will_ask(Allowed, Allowed));
    // Refused while opening with the desktop saying neither: the camera, which is what this
    // recorder is for.
    assert_eq!(
        RecorderRefusal::Camera,
        rules::refused_after(Unknown, Unknown)
    );
    assert_eq!(
        RecorderRefusal::Microphone,
        rules::refused_after(Allowed, DeniedByUser)
    );
}

/// Each refusal's sentence, and the settings page for it — not where the app itself is at fault.
/// Adapted: the page is a [`SettingsPage`] (GNOME Settings' Privacy panels), not an `ms-settings:`
/// URI.
#[test]
fn a_refusal_says_where_to_mend_it() {
    let say = common::english();
    assert_eq!(
        (
            "Family needs permission to use your camera. Turn it on in Settings.".to_string(),
            Some(SettingsPage::Camera)
        ),
        rules::refusal(
            RecorderRefusal::Camera,
            CapabilityAccess::DeniedByUser,
            &say
        )
    );
    assert_eq!(
        (
            "Family needs permission to use your microphone. Turn it on in Settings.".to_string(),
            Some(SettingsPage::Microphone)
        ),
        rules::refusal(
            RecorderRefusal::Microphone,
            CapabilityAccess::DeniedBySystem,
            &say
        )
    );
    assert_eq!(
        None,
        rules::refusal(
            RecorderRefusal::Camera,
            CapabilityAccess::NotDeclaredByApp,
            &say
        )
        .1
    );
    assert_eq!("camera", SettingsPage::Camera.gnome_panel());
    assert_eq!("microphone", SettingsPage::Microphone.gnome_panel());
}

/// The C#'s `TheCamerasTroubleIsReadFromItsCode`, adapted: Windows read Media Foundation's
/// HRESULTs (0xC00D3704 … → InUse, 0xC00DABE0 … → Missing, 0x80004005 → Failed); Linux reads the
/// GStreamer bus error's GLib domain and code — a busy device, a missing one, caps that will not
/// link, and anything else.
#[test]
fn the_cameras_trouble_is_read_from_its_code() {
    for (domain, code, trouble) in [
        (rules::RESOURCE_ERROR_DOMAIN, 4, CameraTrouble::InUse),
        (rules::RESOURCE_ERROR_DOMAIN, 3, CameraTrouble::Missing),
        (rules::CORE_ERROR_DOMAIN, 7, CameraTrouble::Unsupported),
        (rules::CORE_ERROR_DOMAIN, 10, CameraTrouble::Unsupported),
        (rules::STREAM_ERROR_DOMAIN, 3, CameraTrouble::Unsupported),
        (rules::STREAM_ERROR_DOMAIN, 5, CameraTrouble::Unsupported),
        (rules::STREAM_ERROR_DOMAIN, 6, CameraTrouble::Unsupported),
        (rules::STREAM_ERROR_DOMAIN, 11, CameraTrouble::Unsupported),
        (rules::RESOURCE_ERROR_DOMAIN, 1, CameraTrouble::Failed),
        (rules::RESOURCE_ERROR_DOMAIN, 5, CameraTrouble::Failed),
        (rules::STREAM_ERROR_DOMAIN, 1, CameraTrouble::Failed),
        (rules::CORE_ERROR_DOMAIN, 4, CameraTrouble::Failed),
        ("g-io-error-quark", 4, CameraTrouble::Failed),
    ] {
        assert_eq!(trouble, rules::trouble(domain, code), "{domain} {code}");
    }
}

#[test]
fn each_trouble_has_its_sentence() {
    let say = common::english();
    assert_eq!(
        "The camera is being used by another app.",
        rules::trouble_sentence(CameraTrouble::InUse, &say)
    );
    assert_eq!(
        "Video messages can't be recorded with this camera.",
        rules::trouble_sentence(CameraTrouble::Unsupported, &say)
    );
    assert_eq!(
        "Something went wrong. Try again.",
        rules::trouble_sentence(CameraTrouble::Missing, &say)
    );
    assert_eq!(
        "Something went wrong. Try again.",
        rules::trouble_sentence(CameraTrouble::Failed, &say)
    );
}

/// Near-black is a shutter, not a dark room.
#[test]
fn near_black_is_almost_no_light() {
    assert!(rules::looks_black(0.0));
    assert!(rules::looks_black(0.06));
    assert!(!rules::looks_black(0.061));
    assert!(!rules::looks_black(0.2));
    assert!(!rules::looks_black(f64::NAN));
}

/// Two seconds of near-black frames, decided once; one lit frame settles it the other way.
#[test]
fn a_picture_that_stays_black_for_two_seconds_is_said_once() {
    let mut check = PictureCheck::new();
    assert!(!check.frame(1_000, 0.01));
    assert!(!check.frame(2_000, 0.0));
    assert_eq!(None, check.black());
    assert!(check.frame(3_000, 0.02));
    assert_eq!(Some(true), check.black());
    assert!(!check.frame(4_000, 0.0));

    let mut lit = PictureCheck::new();
    assert!(!lit.frame(0, 0.01));
    assert!(!lit.frame(500, 0.3));
    assert_eq!(Some(false), lit.black());
    assert!(!lit.frame(5_000, 0.0));
    assert_eq!(Some(false), lit.black());
}

// ---- the video button's guard (S1.1) ----------------------------------------------------------

/// It ignores clicks for 600 ms after it APPEARS — not after every redraw while it is there.
#[test]
fn the_video_button_ignores_clicks_just_after_it_appears() {
    let mut guard = DoorGuard::new();
    assert!(!guard.accepts(10_000));
    guard.showing(true, 10_000);
    assert!(!guard.accepts(10_599));
    guard.showing(true, 10_300);
    assert!(guard.accepts(10_600));
    guard.showing(false, 11_000);
    assert!(!guard.accepts(11_000));
    guard.showing(true, 12_000);
    assert!(!guard.accepts(12_500));
    assert!(guard.accepts(12_600));
}

// ---- what is sent (S3.6) ----------------------------------------------------------------------

/// The square within the limits goes as a video message, and says nothing more.
#[test]
fn a_square_within_the_limits_goes_round() {
    let say = common::english();
    let round = RoundSendPlan::new(true, true, None);
    assert_eq!(
        round,
        rules::plan(Some(4_200_000), Some(59_500), &LIMITS, &say)
    );
    assert_eq!(
        round,
        rules::plan(Some(12_582_912), Some(60_000), &LIMITS, &say)
    );
    assert_eq!(round, rules::plan(Some(1), Some(1), &LIMITS, &say));
}

/// Over the byte ceiling an operator set: the square itself, as a regular video, after saying so.
#[test]
fn a_square_too_big_goes_as_a_regular_video() {
    let say = common::english();
    let too_big = RoundSendPlan::new(
        false,
        true,
        Some("Too big for a video message. It will be sent as a regular video."),
    );
    assert_eq!(
        too_big,
        rules::plan(Some(12_582_913), Some(59_500), &LIMITS, &say)
    );
    assert_eq!(
        too_big,
        rules::plan(
            Some(2_000_000),
            Some(10_000),
            &limits(60_000, 1_000_000),
            &say
        )
    );
}

/// No square, or a length the server would refuse round: the take itself, as a regular video.
#[test]
fn no_square_goes_as_the_take_itself() {
    let say = common::english();
    for (bytes, duration) in [
        (None, Some(30_000)),
        (Some(0), Some(30_000)),
        (Some(4_000_000), None),
        (Some(4_000_000), Some(0)),
        (Some(4_000_000), Some(60_001)),
    ] {
        assert_eq!(
            RoundSendPlan::new(
                false,
                false,
                Some("Couldn't make it round. It will be sent as a regular video.")
            ),
            rules::plan(bytes, duration, &LIMITS, &say),
            "{bytes:?} {duration:?}"
        );
    }
}

/// Staged as the wire wants it: kind=video, video/mp4, 480 × 480, its length and its poster.
#[test]
fn the_square_is_staged_as_the_wire_wants_it() {
    let bytes = vec![0, 0, 0, 24, b'f', b't', b'y', b'p'];
    let jpeg = vec![0xFF, 0xD8, 0xFF];
    let staged = rules::staged(bytes.clone(), 23_400, Some(jpeg.clone()));
    assert_eq!(
        ("video", "video/mp4", Some(480), Some(480), Some(23_400)),
        (
            staged.kind.as_str(),
            staged.mime.as_str(),
            staged.width,
            staged.height,
            staged.duration_ms
        )
    );
    assert_eq!(bytes, staged.bytes);
    assert_eq!(Some(jpeg), staged.preview);
}

/// REVIEW's line, "Video message · 0:23".
#[test]
fn review_says_what_it_is_and_how_long() {
    let say = common::english();
    assert_eq!("Video message · 0:23", rules::review_line(23_400, &say));
    assert_eq!("Video message · 0:59", rules::review_line(59_499, &say));
    assert_eq!("Video message · 0:00", rules::review_line(-5, &say));
}

// ---- what the recorder hears (S4) -------------------------------------------------------------

#[test]
fn the_recorder_hears_what_ends_a_voice_recording_its_own_way() {
    assert_eq!(
        Some(RecorderInterruption::Call),
        rules::interruption_of(RecordingEnd::Call)
    );
    assert_eq!(
        Some(RecorderInterruption::WindowHidden),
        rules::interruption_of(RecordingEnd::WindowHidden)
    );
    assert_eq!(
        Some(RecorderInterruption::WindowHidden),
        rules::interruption_of(RecordingEnd::LeftChat)
    );
    assert_eq!(
        Some(RecorderInterruption::SessionLocked),
        rules::interruption_of(RecordingEnd::SessionLocked)
    );
    assert_eq!(
        Some(RecorderInterruption::SignedOut),
        rules::interruption_of(RecordingEnd::WindowClosed)
    );
    assert_eq!(
        Some(RecorderInterruption::SignedOut),
        rules::interruption_of(RecordingEnd::SignedOut)
    );
    for own in [
        RecordingEnd::Stopped,
        RecordingEnd::Sent,
        RecordingEnd::Capped,
        RecordingEnd::Deleted,
        RecordingEnd::RecorderFailed,
    ] {
        assert_eq!(None, rules::interruption_of(own), "{own:?}");
    }
}

// ---- Linux's own: the finished take, and the crop ---------------------------------------------

fn square_read(bytes: i64, duration_ms: Option<i64>) -> TakeReading {
    TakeReading {
        bytes,
        video_codec: "h264".to_string(),
        width: 480,
        height: 480,
        rotation: 0,
        audio_codec: Some("aac".to_string()),
        frame_rate: Some(30.0),
        duration_ms,
        moov_first: true,
    }
}

/// What Windows' window did with a finished take (`MakeClipAsync`), held here: the square read back
/// as asked goes round, with the file's own length; one too big goes as the square, a regular
/// video; one that came out wrong — or with its index last, or not read at all — goes as the take
/// itself, through the planner, with the recorder's clock where the file says nothing.
#[test]
fn a_finished_take_goes_as_what_its_read_back_says() {
    let say = common::english();
    let asked = rules::encodes(30, 1, true)[0];
    assert_eq!(
        (RoundSendPlan::new(true, true, None), 23_450),
        rules::plan_take(
            &asked,
            Some(&square_read(1_649_700, Some(23_450))),
            23_400,
            &LIMITS,
            &say
        )
    );
    assert_eq!(
        (
            RoundSendPlan::new(
                false,
                true,
                Some("Too big for a video message. It will be sent as a regular video.")
            ),
            23_450
        ),
        rules::plan_take(
            &asked,
            Some(&square_read(12_582_913, Some(23_450))),
            23_400,
            &LIMITS,
            &say
        )
    );
    let regular = RoundSendPlan::new(
        false,
        false,
        Some("Couldn't make it round. It will be sent as a regular video."),
    );
    let sideways = TakeReading {
        rotation: 90,
        ..square_read(1_649_700, Some(23_450))
    };
    assert_eq!(
        (regular.clone(), 23_450),
        rules::plan_take(&asked, Some(&sideways), 23_400, &LIMITS, &say)
    );
    let index_last = TakeReading {
        moov_first: false,
        ..square_read(1_649_700, Some(23_450))
    };
    assert_eq!(
        (regular.clone(), 23_450),
        rules::plan_take(&asked, Some(&index_last), 23_400, &LIMITS, &say)
    );
    assert_eq!(
        (regular.clone(), 23_400),
        rules::plan_take(&asked, None, 23_400, &LIMITS, &say)
    );
    // A length the file does not state: the recorder's clock.
    assert_eq!(
        (RoundSendPlan::new(true, true, None), 23_400),
        rules::plan_take(
            &asked,
            Some(&square_read(1_649_700, None)),
            23_400,
            &LIMITS,
            &say
        )
    );
}

/// `videocrop`'s four edges for the centre square — what the shell's recording branch is built with.
#[test]
fn the_square_is_what_videocrop_takes_off_each_edge() {
    let crop = rules::centre_square(1280, 720).expect("a square");
    assert_eq!((280, 280, 0, 0), crop.videocrop(1280, 720));
    let odd = rules::centre_square(645, 480).expect("a square");
    assert_eq!((82, 83, 0, 0), odd.videocrop(645, 480));
    let tall = rules::centre_square(480, 642).expect("a square");
    assert_eq!((0, 0, 80, 82), tall.videocrop(480, 642));
    // A crop that does not fit the frame takes nothing it does not have.
    assert_eq!(
        (10, 0, 10, 0),
        SquareCrop {
            x: 10,
            y: 10,
            side: 100
        }
        .videocrop(50, 50)
    );
}
