//! `MediaPlanOracleTests`: what a picked video or sound file becomes before upload. Every case in
//! `fixtures/media-plan-vectors.json` was printed from `fc_text::media_plan`, and the same bytes
//! sit in the iOS test bundle and on Android's JVM classpath. On Linux the plan IS
//! `fc_text::media_plan`, so a case here holds the `fc_text` this build links to the file the
//! other three ports are held to. The file is PARSED, never compared as text; a case naming a
//! function this suite does not know fails.

mod common;

use serde_json::Value;

use common::{boolean, float, optional_float, optional_string, optional_uint, string, uint};
use fc_text::media_plan::{
    self, AudioPlan, AudioSource, OnFailure, Upload, VideoPlan, VideoSource, VideoTarget,
};

fn cases() -> Vec<Value> {
    match common::fixture_json("media-plan-vectors.json") {
        Value::Array(cases) => cases,
        other => panic!("not a list of cases: {other}"),
    }
}

fn u32_of(row: &Value, name: &str) -> u32 {
    u32::try_from(uint(row, name)).expect("a u32")
}

fn video(input: &Value) -> VideoSource {
    VideoSource {
        width: u32_of(input, "width"),
        height: u32_of(input, "height"),
        frame_rate: optional_float(input, "frame_rate"),
        container: string(input, "container"),
        video_codec: string(input, "video_codec"),
        audio_codec: optional_string(input, "audio_codec"),
        audio_channels: optional_uint(input, "audio_channels").map(|channels| channels as u32),
        video_bitrate: optional_uint(input, "video_bitrate"),
        audio_bitrate: optional_uint(input, "audio_bitrate"),
        size_bytes: uint(input, "size_bytes"),
        duration_ms: optional_uint(input, "duration_ms"),
    }
}

fn audio(input: &Value) -> AudioSource {
    AudioSource {
        container: string(input, "container"),
        codec: string(input, "codec"),
        channels: optional_uint(input, "channels").map(|channels| channels as u32),
        bitrate: optional_uint(input, "bitrate"),
        size_bytes: uint(input, "size_bytes"),
        duration_ms: optional_uint(input, "duration_ms"),
    }
}

/// A frame rate is compared bit for bit: the file writes the shortest decimal that round-trips.
fn same_rate(expected: f64, actual: f64, name: &str) {
    assert!(
        expected.to_bits() == actual.to_bits(),
        "{name}: frame rate {actual:?}, the original says {expected:?}"
    );
}

#[test]
fn the_file_is_the_one_the_original_printed() {
    let cases = cases();
    // 464 when this was written; fewer would mean a truncated copy, and every function has its
    // cases.
    assert!(cases.len() >= 464, "only {} cases", cases.len());
    let mut functions = vec![
        "target_size",
        "target_frame_rate",
        "profile_video_bitrate",
        "target_video_bitrate",
        "target_audio_bitrate",
        "estimated_bitrate",
        "plan_video",
        "plan_audio",
        "sendable",
        "on_failure",
        "keep_smaller",
    ];
    functions.sort_unstable();
    let mut named: Vec<String> = cases.iter().map(|row| string(row, "function")).collect();
    named.sort_unstable();
    named.dedup();
    assert_eq!(named, functions);
}

#[test]
fn the_constants_are_the_originals() {
    // The protocol's table, by value — a changed number here is a changed protocol, not a refactor.
    assert_eq!(media_plan::MAX_SHORT_SIDE, 720);
    assert_eq!(media_plan::MAX_FRAME_RATE, 30.0);
    assert_eq!(media_plan::FRAME_RATE_TOLERANCE, 30.5);
    assert_eq!(media_plan::MIN_VIDEO_BITRATE, 250_000);
    assert_eq!(media_plan::MAX_VIDEO_BITRATE, 2_000_000);
    assert_eq!(media_plan::STEREO_AUDIO_BITRATE, 128_000);
    assert_eq!(media_plan::MONO_AUDIO_BITRATE, 64_000);
    assert_eq!(media_plan::VOICE_NOTE_BITRATE, 64_000);
    assert_eq!(media_plan::MAX_KEPT_LOSSY_AUDIO_BITRATE, 192_000);
}

/// Every case, whatever function it names: the assertion names the case that disagreed.
#[test]
fn every_case_is_decided_as_the_original_decides_it() {
    let cases = cases();
    let mut checked = 0;
    for row in &cases {
        let name = string(row, "name");
        let input = &row["input"];
        let expected = &row["expected"];
        match string(row, "function").as_str() {
            "target_size" => assert_eq!(
                (u32_of(expected, "width"), u32_of(expected, "height")),
                media_plan::target_size(u32_of(input, "width"), u32_of(input, "height")),
                "{name}"
            ),
            "target_frame_rate" => same_rate(
                float(expected, "frame_rate"),
                media_plan::target_frame_rate(optional_float(input, "frame_rate")),
                &name,
            ),
            "profile_video_bitrate" => assert_eq!(
                uint(expected, "bitrate"),
                media_plan::profile_video_bitrate(
                    u32_of(input, "width"),
                    u32_of(input, "height"),
                    float(input, "frame_rate")
                ),
                "{name}"
            ),
            "target_video_bitrate" => assert_eq!(
                uint(expected, "bitrate"),
                media_plan::target_video_bitrate(
                    u32_of(input, "width"),
                    u32_of(input, "height"),
                    float(input, "frame_rate"),
                    optional_uint(input, "source_bitrate")
                ),
                "{name}"
            ),
            "target_audio_bitrate" => assert_eq!(
                uint(expected, "bitrate"),
                media_plan::target_audio_bitrate(
                    optional_uint(input, "channels").map(|channels| channels as u32),
                    optional_uint(input, "source_bitrate")
                ),
                "{name}"
            ),
            "estimated_bitrate" => assert_eq!(
                optional_uint(expected, "bitrate"),
                media_plan::estimated_bitrate(
                    uint(input, "size_bytes"),
                    optional_uint(input, "duration_ms"),
                    uint(input, "audio_bitrate")
                ),
                "{name}"
            ),
            "plan_video" => check_video(&name, &video(input), expected),
            "plan_audio" => check_audio(&name, &audio(input), expected),
            "sendable" => assert_eq!(
                boolean(expected, "sendable"),
                media_plan::sendable(
                    &string(input, "kind"),
                    &string(input, "container"),
                    boolean(input, "honest"),
                    uint(input, "size_bytes"),
                    uint(input, "ceiling_bytes")
                ),
                "{name}"
            ),
            "on_failure" => {
                let send = media_plan::on_failure(boolean(input, "source_sendable"));
                let said = if send == OnFailure::Original {
                    "original"
                } else {
                    "todays_path"
                };
                assert_eq!(string(expected, "send"), said, "{name}");
            }
            "keep_smaller" => {
                let upload = media_plan::keep_smaller(
                    uint(input, "source_bytes"),
                    boolean(input, "source_sendable"),
                    uint(input, "result_bytes"),
                );
                let said = if upload == Upload::Source {
                    "source"
                } else {
                    "result"
                };
                assert_eq!(string(expected, "upload"), said, "{name}");
            }
            other => panic!("{name}: a function this port does not know — {other}"),
        }
        checked += 1;
    }
    assert_eq!(checked, cases.len());
}

fn named(plan: &VideoPlan) -> &'static str {
    match plan {
        VideoPlan::Keep => "keep",
        VideoPlan::Transcode(_) => "transcode",
        VideoPlan::Fallback => "fallback",
    }
}

fn check_video(name: &str, source: &VideoSource, expected: &Value) {
    let plan = media_plan::plan_video(source);
    assert_eq!(string(expected, "plan"), named(&plan), "{name}: plan");
    assert_eq!(
        boolean(expected, "within_profile"),
        media_plan::within_profile(source),
        "{name}: rule A"
    );
    assert_eq!(
        optional_uint(expected, "source_video_bitrate"),
        media_plan::source_video_bitrate(source),
        "{name}: V"
    );
    let target = media_plan::video_target(source);
    let wanted = &expected["target"];
    if wanted.is_null() {
        assert_eq!(target, None, "{name}: a target where the original has none");
        return;
    }
    let actual = target.unwrap_or_else(|| panic!("{name}: no target where the original has one"));
    assert_eq!(
        u32_of(wanted, "width"),
        actual.width,
        "{name}: target width"
    );
    assert_eq!(
        u32_of(wanted, "height"),
        actual.height,
        "{name}: target height"
    );
    same_rate(float(wanted, "frame_rate"), actual.frame_rate, name);
    assert_eq!(
        uint(wanted, "video_bitrate"),
        actual.video_bitrate,
        "{name}: target video bitrate"
    );
    assert_eq!(
        optional_uint(wanted, "audio_bitrate"),
        actual.audio_bitrate,
        "{name}: target audio bitrate"
    );
    if let VideoPlan::Transcode(to) = plan {
        // A transcode is to the target, and to nothing else.
        assert_eq!(to, actual, "{name}");
    }
}

fn check_audio(name: &str, source: &AudioSource, expected: &Value) {
    let bitrate = media_plan::source_audio_bitrate(source);
    assert_eq!(
        optional_uint(expected, "source_bitrate"),
        bitrate,
        "{name}: source bitrate"
    );
    let target = media_plan::target_audio_bitrate(source.channels, bitrate);
    assert_eq!(
        uint(expected, "target_bitrate"),
        target,
        "{name}: target bitrate"
    );
    let plan = media_plan::plan_audio(source);
    let said = match plan {
        AudioPlan::Transcode { .. } => "transcode",
        AudioPlan::Keep => "keep",
    };
    assert_eq!(string(expected, "plan"), said, "{name}: plan");
    if let AudioPlan::Transcode { bitrate } = plan {
        assert_eq!(target, bitrate, "{name}");
    }
}

/// What the file cannot carry: JSON has no NaN or infinity, so the reader's other "unknown" frame
/// rates are pinned here, against the rule the original states for them (`known_frame_rate`).
#[test]
fn a_frame_rate_that_is_not_a_number_is_unknown() {
    for nonsense in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.0, -1.0] {
        assert_eq!(media_plan::known_frame_rate(Some(nonsense)), None);
        assert_eq!(media_plan::target_frame_rate(Some(nonsense)), 30.0);
    }
    let clip = VideoSource {
        width: 1280,
        height: 720,
        frame_rate: Some(f64::NAN),
        container: "video/mp4".into(),
        video_codec: "h264".into(),
        audio_codec: Some("aac".into()),
        audio_channels: Some(2),
        video_bitrate: Some(1_500_000),
        audio_bitrate: Some(128_000),
        size_bytes: 20_000_000,
        duration_ms: Some(19_000),
    };
    assert!(!media_plan::within_profile(&clip));
    assert_eq!(
        media_plan::plan_video(&clip),
        VideoPlan::Transcode(VideoTarget {
            width: 1280,
            height: 720,
            frame_rate: 30.0,
            video_bitrate: 1_500_000,
            audio_bitrate: Some(128_000),
        })
    );
}

/// Half UP, where .NET's default is half to EVEN — the one mistake a C# port is built to make.
#[test]
fn a_half_thousand_rounds_up_and_not_to_even() {
    assert_eq!(media_plan::profile_video_bitrate(404, 288, 30.0), 253_000);
    assert_eq!(media_plan::profile_video_bitrate(484, 360, 24.0), 303_000);
    assert_eq!(media_plan::profile_video_bitrate(960, 540, 25.0), 938_000);
    assert_eq!(media_plan::profile_video_bitrate(852, 480, 30.0), 888_000);
    // Left to right as the protocol writes it, this one is 837.4999… and would round down.
    assert_eq!(media_plan::profile_video_bitrate(3618, 128, 25.0), 838_000);
}
