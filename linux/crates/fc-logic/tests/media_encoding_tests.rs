//! `MediaEncodingTests`: what goes into an encode for a plan — the numbers only. The platform
//! calls that take them are the shell's (`media_encoding::preparing::MediaPreparing`, the voice
//! recorder); everything they are GIVEN is pinned here.
//!
//! THE LINUX FORM (carry-forward: AAC as asked, no Media Foundation rate ladder; `x264enc` High
//! then `openh264enc` Main). The C# cases that pinned Media Foundation's ladder — its encoder's four
//! documented rates, the second attempt at the nearest of them, the pass-through of a thin AAC track
//! no documented rate was left for, a mix's "upright" doubles — are kept with their inputs and
//! assert the Linux rule instead: ONE audio attempt at the planner's exact rate, a video asked for
//! as High then Main with that same audio, nothing passed through, a mix asked exactly as a
//! transcode. The readers' inputs are the discoverer's: an `image-orientation` tag for the turn, the
//! container's own average frame rate where the stream states none, colorimetry in ISO/IEC
//! 23091-2 code points. Each adapted case says so.

use std::time::Duration;

use fc_logic::container_sound::MixSound;
use fc_logic::media_encoding::{
    self, AacEncoding, Chosen, H264Profile, VideoEncoding, aac_came_out, aac_channels,
    aac_sample_rate, audio_attempts, audio_rate_came_out, choose, displayed, encoded, first_known,
    frame_rate, frame_rate_came_out, frame_rate_ratio, hides_audio, is_hdr, m4a_name, matches,
    mix_attempts, picked_audio_type, rotation, square_pixels, transcode_ceiling,
    voice_note_attempts,
};
use fc_text::media;
use fc_text::media_plan::{
    self, AudioPlan, AudioSource, VideoPlan, VideoSource, VideoTarget, plan_audio, plan_video,
};

fn aac(sample_rate: u32, channels: u32, bitrate: u32) -> AacEncoding {
    AacEncoding {
        sample_rate,
        channels,
        bitrate,
    }
}

/// `new VideoEncoding(width, height, rotation, bitrate, numerator, denominator, profile, audio,
/// toSdr)` with the C#'s defaults for the rest.
fn encoding(
    width: u32,
    height: u32,
    rotation: u32,
    bitrate: u32,
    numerator: u32,
    denominator: u32,
    profile: H264Profile,
    audio: Option<AacEncoding>,
    to_sdr: bool,
) -> VideoEncoding {
    VideoEncoding {
        width,
        height,
        rotation,
        bitrate,
        frame_rate_numerator: numerator,
        frame_rate_denominator: denominator,
        profile,
        audio,
        to_sdr,
        keep_audio: false,
        keyframe_spacing: None,
    }
}

/// `new VideoTarget(width, height, frameRate, videoBitrate, audioBitrate)`.
fn target(
    width: u32,
    height: u32,
    frame_rate: f64,
    video_bitrate: u64,
    audio_bitrate: Option<u64>,
) -> VideoTarget {
    VideoTarget {
        width,
        height,
        frame_rate,
        video_bitrate,
        audio_bitrate,
    }
}

/// `MediaEncoding.VideoAttempts(target, rotation, num, den, channels, sampleRate, …, hdr)` in its
/// Linux shape: the source's audio bit rate and codec no longer change the attempts.
fn video_attempts(
    target: &VideoTarget,
    rotation: u32,
    numerator: u32,
    denominator: u32,
    channels: Option<i64>,
    sample_rate: Option<i64>,
    hdr: bool,
) -> Vec<VideoEncoding> {
    media_encoding::video_attempts(
        target,
        rotation,
        numerator,
        denominator,
        channels,
        sample_rate,
        hdr,
    )
}

// --- reading ------------------------------------------------------------------------------------

#[test]
fn a_number_is_known_when_it_is_more_than_zero() {
    assert_eq!(Some(5), first_known(&[Some(0), None, Some(5)]));
    assert_eq!(Some(3), first_known(&[Some(-1), Some(3), Some(7)]));
    assert_eq!(None, first_known(&[None, Some(0)]));
    assert_eq!(None, first_known(&[]));
}

/// The first reader's turn when it states one — even 0 — and the second's otherwise; nonsense is no
/// turn. (On Linux the first reader is the discoverer's `image-orientation` tag.)
#[test]
fn the_turn_is_media_foundations_then_the_shells() {
    assert_eq!(90, rotation(Some(90), Some(0)));
    assert_eq!(0, rotation(Some(0), Some(90)));
    assert_eq!(270, rotation(None, Some(270)));
    assert_eq!(180, rotation(Some(45), Some(180)));
    assert_eq!(0, rotation(None, None));
    assert_eq!(0, rotation(Some(-90), Some(360)));
}

#[test]
fn a_quarter_turn_swaps_the_displayed_sides() {
    assert_eq!((1080, 1920), displayed(1920, 1080, 90));
    assert_eq!((1080, 1920), displayed(1920, 1080, 270));
    assert_eq!((1920, 1080), displayed(1920, 1080, 180));
    assert_eq!((1920, 1080), displayed(1920, 1080, 0));
}

/// ADAPTED: the stream's own ratio first; the second witness is the CONTAINER'S average rate
/// (`fc_text::mp4_read`'s `Track::frame_rate`), where Windows read the shell's frames per thousand
/// seconds (29 970 for 29.97).
#[test]
fn a_frame_rate_is_the_streams_ratio_or_the_shells_thousandths() {
    assert_eq!(
        Some(30_000.0 / 1001.0),
        frame_rate(30_000, 1001, Some(29.97))
    );
    assert_eq!(Some(29.97), frame_rate(0, 0, Some(29.97)));
    assert_eq!(Some(25.0), frame_rate(60, 0, Some(25.0)));
    assert_eq!(None, frame_rate(0, 1, None));
    assert_eq!(None, frame_rate(30, 0, None));
    // A container rate nobody could time is no rate.
    assert_eq!(None, frame_rate(0, 0, Some(0.0)));
    assert_eq!(None, frame_rate(0, 0, Some(f64::NAN)));
}

/// ADAPTED: ISO/IEC 23091-2 code points (what GStreamer's colorimetry converts to), where Windows
/// read Media Foundation's: PQ is 16 and HLG 18 here (15 and 16 there), BT.709 is 1 (5 and 2 there);
/// BT.2020's primaries are 9 in both.
#[test]
fn hdr_is_pq_or_hlg_or_the_wide_gamut() {
    assert!(is_hdr(Some(16), None));
    assert!(is_hdr(Some(18), Some(1)));
    assert!(is_hdr(None, Some(9)));
    assert!(!is_hdr(Some(1), Some(1)));
    assert!(!is_hdr(None, None));
    // Media Foundation's PQ number is BT.2020's 12-bit SDR transfer in these code points.
    assert!(!is_hdr(Some(15), None));
}

/// AIFF and FLAC reach the audio rules although the server takes neither; a document never does.
#[test]
fn a_sound_file_the_router_sent_as_a_file_is_still_judged_as_sound() {
    assert_eq!(
        Some("audio/x-aiff".to_string()),
        picked_audio_type("audio/x-aiff", "take.aiff")
    );
    assert_eq!(
        Some("audio/flac".to_string()),
        picked_audio_type("application/octet-stream", "Song.FLAC")
    );
    assert_eq!(
        Some("audio/x-flac".to_string()),
        picked_audio_type("audio/x-flac", "song.flac")
    );
    assert_eq!(None, picked_audio_type("application/pdf", "minutes.pdf"));
    assert_eq!(None, picked_audio_type("video/x-matroska", "clip.mkv"));
}

#[test]
fn a_re_encoded_track_is_named_for_what_it_now_is() {
    assert_eq!(Some("song.m4a".to_string()), m4a_name(Some("song.wav")));
    assert_eq!(
        Some("Take 3.m4a".to_string()),
        m4a_name(Some("Take 3.FLAC"))
    );
    assert_eq!(Some("memo.m4a".to_string()), m4a_name(Some("memo")));
    assert_eq!(None, m4a_name(None));
    // At the limit the stem is cut, never the extension.
    let named = m4a_name(Some(&"a".repeat(255))).unwrap();
    // The server counts characters; these are all ASCII, so bytes are characters.
    assert_eq!(255, named.chars().count());
    assert!(named.ends_with(".m4a"));
}

// --- the profile --------------------------------------------------------------------------------

/// A portrait phone clip is a landscape frame with a turn; it is ENCODED landscape and turned by
/// metadata.
#[test]
fn the_encoded_sides_undo_the_turn() {
    assert_eq!((1280, 720), encoded(720, 1280, 90));
    assert_eq!((1280, 720), encoded(720, 1280, 270));
    assert_eq!((720, 1280), encoded(720, 1280, 180));
    assert_eq!((720, 1280), encoded(720, 1280, 0));
}

#[test]
fn a_frame_rate_ratio_is_the_sources_own_when_it_is_the_sources_rate() {
    assert_eq!(
        (30_000, 1001),
        frame_rate_ratio(30_000.0 / 1001.0, 30_000, 1001)
    );
    assert_eq!(
        (24_000, 1001),
        frame_rate_ratio(24_000.0 / 1001.0, 24_000, 1001)
    );
    assert_eq!((25, 1), frame_rate_ratio(25.0, 25, 1));
    // Capped from 60: the cap, not the source.
    assert_eq!((30, 1), frame_rate_ratio(30.0, 60, 1));
    assert_eq!((30, 1), frame_rate_ratio(30.0, 60_000, 1001));
    // Read from the second witness alone, or a ratio that does not match: the nearest thousandth not
    // above it.
    assert_eq!((2997, 100), frame_rate_ratio(29.97, 0, 0));
    assert_eq!((2997, 125), frame_rate_ratio(23.976, 0, 0));
    assert_eq!((2997, 100), frame_rate_ratio(30_000.0 / 1001.0, 0, 0));
    assert_eq!((49, 2), frame_rate_ratio(24.5, 0, 0));
    assert_eq!((61, 2), frame_rate_ratio(30.5, 0, 0));
    assert_eq!((30, 1), frame_rate_ratio(f64::NAN, 0, 0));
}

/// Rule B: whatever the ratio, it is never a hair above the target.
#[test]
fn a_frame_rate_ratio_is_never_above_its_target() {
    // The C#'s `for (target = 0.0004; target <= 30.5; target += 0.0137)`, the same additions.
    let mut target = 0.0004;
    while target <= 30.5 {
        let (numerator, denominator) = frame_rate_ratio(target, 0, 0);
        assert!(
            f64::from(numerator) / f64::from(denominator) <= target,
            "{numerator}/{denominator} for {target:?}"
        );
        assert!(numerator > 0 && denominator > 0);
        target += 0.0137;
    }
}

/// 44.1 and 48 kHz and nothing else — the two rates the voice note's row names.
#[test]
fn the_sample_rate_is_one_the_encoder_takes() {
    assert_eq!(48_000, aac_sample_rate(Some(48_000)));
    assert_eq!(48_000, aac_sample_rate(Some(96_000)));
    assert_eq!(48_000, aac_sample_rate(Some(44_101)));
    assert_eq!(44_100, aac_sample_rate(Some(44_100)));
    assert_eq!(44_100, aac_sample_rate(Some(22_050)));
    assert_eq!(44_100, aac_sample_rate(None));
}

#[test]
fn mono_stays_mono_and_everything_else_is_stereo() {
    assert_eq!(1, aac_channels(Some(1)));
    assert_eq!(2, aac_channels(Some(2)));
    assert_eq!(2, aac_channels(Some(6)));
    assert_eq!(2, aac_channels(None));
}

/// ADAPTED: Media Foundation's encoder took 96, 128, 160 and 192 kbit/s and nothing else, and a
/// refused target fell back to the nearest of them that kept rule B. `avenc_aac` takes every target,
/// so there is no nearest rate: each of the C#'s targets is asked for once, at itself — which keeps
/// rule B by construction, since the planner's target is already capped by the source.
#[test]
fn the_nearest_encoder_rate_keeps_rule_b() {
    for (target, channels) in [
        (128_000, 2),
        (112_000, 2),
        (64_000, 1),
        (96_000, 2),
        (48_000, 1),
    ] {
        let attempts = audio_attempts(target, Some(channels), Some(44_100));
        assert_eq!(
            vec![aac(44_100, channels as u32, target as u32)],
            attempts,
            "{target}"
        );
    }
}

/// ADAPTED: the exact rate is the only one asked — Windows' second attempt at the encoder's lowest
/// documented rate for a mono WAV does not exist here.
#[test]
fn a_picked_track_asks_for_the_exact_rate_first() {
    // A stereo WAV.
    assert_eq!(
        vec![aac(48_000, 2, 128_000)],
        audio_attempts(128_000, Some(2), Some(48_000))
    );
    // A mono WAV: 64 000, and nothing after it.
    assert_eq!(
        vec![aac(44_100, 1, 64_000)],
        audio_attempts(64_000, Some(1), Some(44_100))
    );
    // A thin Ogg at 32 000.
    assert_eq!(
        vec![aac(48_000, 1, 32_000)],
        audio_attempts(32_000, Some(1), Some(48_000))
    );
}

/// ADAPTED: the protocol's voice note — AAC-LC, mono, 44.1 kHz, 64 000 — and nothing after it.
#[test]
fn a_voice_note_asks_for_the_protocols_numbers_first() {
    assert_eq!(vec![aac(44_100, 1, 64_000)], voice_note_attempts());
    assert_eq!(
        media_plan::VOICE_NOTE_BITRATE,
        u64::from(voice_note_attempts()[0].bitrate)
    );
}

/// The reasoning doc's first fixture: a 4K60 HDR portrait phone clip comes out 720×1280 displayed
/// — ENCODED 1280×720 with the turn carried — at 30 fps, 2 Mbit/s, H.264 High, AAC-LC 128k, asked for
/// SDR.
#[test]
fn a4_k60_hdr_portrait_clip_is_asked_for720p30_sdr() {
    let phone = VideoSource {
        width: 2160,
        height: 3840,
        frame_rate: Some(60_000.0 / 1001.0),
        container: "video/quicktime".into(),
        video_codec: "hevc".into(),
        audio_codec: Some("aac".into()),
        audio_channels: Some(2),
        video_bitrate: Some(40_000_000),
        audio_bitrate: Some(256_000),
        size_bytes: 150_000_000,
        duration_ms: Some(30_000),
    };
    let VideoPlan::Transcode(planned) = plan_video(&phone) else {
        panic!("a 4K60 HEVC clip is transcoded");
    };
    let attempts = video_attempts(&planned, 90, 60_000, 1001, Some(2), Some(48_000), true);
    assert_eq!(
        vec![
            encoding(
                1280,
                720,
                90,
                2_000_000,
                30,
                1,
                H264Profile::High,
                Some(aac(48_000, 2, 128_000)),
                true
            ),
            encoding(
                1280,
                720,
                90,
                2_000_000,
                30,
                1,
                H264Profile::Main,
                Some(aac(48_000, 2, 128_000)),
                true
            ),
        ],
        attempts
    );
}

/// A kept frame rate keeps its exact ratio; the bitrate is the planner's, to the bit.
#[test]
fn an_ntsc_clip_keeps_its_ratio_and_its_capped_bitrate() {
    let clip = VideoSource {
        width: 1920,
        height: 1080,
        frame_rate: Some(30_000.0 / 1001.0),
        container: "video/mp4".into(),
        video_codec: "h264".into(),
        audio_codec: Some("aac".into()),
        audio_channels: Some(2),
        video_bitrate: Some(1_234_567),
        audio_bitrate: Some(128_000),
        size_bytes: 20_000_000,
        duration_ms: Some(19_000),
    };
    let VideoPlan::Transcode(planned) = plan_video(&clip) else {
        panic!("a 1080p clip is transcoded");
    };
    let first = video_attempts(&planned, 0, 30_000, 1001, Some(2), Some(44_100), false)[0];
    assert_eq!((1280, 720, 0), (first.width, first.height, first.rotation));
    assert_eq!(
        (30_000, 1001),
        (first.frame_rate_numerator, first.frame_rate_denominator)
    );
    assert_eq!(1_234_567, first.bitrate);
    assert_eq!(Some(aac(44_100, 2, 128_000)), first.audio);
    assert!(!first.to_sdr);
}

fn profiles_and_rates(attempts: &[VideoEncoding]) -> Vec<(H264Profile, Option<u32>)> {
    attempts
        .iter()
        .map(|attempt| (attempt.profile, attempt.audio.map(|audio| audio.bitrate)))
        .collect()
}

/// ADAPTED: a mono clip asks for 64 000 under High and under Main — no encoder's 96 000 between
/// them — and a thin one for its own rate.
#[test]
fn a_mono_clip_falls_back_to_the_encoders_rate_only_where_rule_b_allows() {
    let mono = target(1280, 720, 30.0, 2_000_000, Some(64_000));
    assert_eq!(
        vec![
            (H264Profile::High, Some(64_000)),
            (H264Profile::Main, Some(64_000))
        ],
        profiles_and_rates(&video_attempts(
            &mono,
            0,
            30,
            1,
            Some(1),
            Some(48_000),
            false
        ))
    );

    let thin = target(1280, 720, 30.0, 2_000_000, Some(48_000));
    assert_eq!(
        vec![
            (H264Profile::High, Some(48_000)),
            (H264Profile::Main, Some(48_000))
        ],
        profiles_and_rates(&video_attempts(
            &thin,
            0,
            30,
            1,
            Some(1),
            Some(48_000),
            false
        ))
    );
}

/// ADAPTED: a 4K clip with 64 000 mono AAC — a messenger's re-share, a screen recording. On Windows
/// the encoder's lowest documented rate was above it, so the track was passed through as itself;
/// `avenc_aac` takes 64 000, so it is ENCODED at its own rate under High and under Main, and never
/// passed through.
#[test]
fn a_thin_aac_track_is_passed_through_where_the_encoder_has_no_rate_for_it() {
    let thin = target(1280, 720, 30.0, 2_000_000, Some(64_000));
    let attempts = video_attempts(&thin, 0, 30, 1, Some(1), Some(22_050), false);
    assert_eq!(
        vec![
            (H264Profile::High, aac(44_100, 1, 64_000), false),
            (H264Profile::Main, aac(44_100, 1, 64_000), false),
        ],
        attempts
            .iter()
            .map(|attempt| (attempt.profile, attempt.audio.unwrap(), attempt.keep_audio))
            .collect::<Vec<_>>()
    );

    // Stereo at 112 000: its own rate, encoded, under both.
    let stereo = target(1280, 720, 30.0, 2_000_000, Some(112_000));
    assert_eq!(
        vec![
            (H264Profile::High, Some(112_000), false),
            (H264Profile::Main, Some(112_000), false),
        ],
        video_attempts(&stereo, 0, 30, 1, Some(2), Some(48_000), false)
            .iter()
            .map(|attempt| (
                attempt.profile,
                attempt.audio.map(|audio| audio.bitrate),
                attempt.keep_audio
            ))
            .collect::<Vec<_>>()
    );
}

/// ADAPTED: `MediaEncoding.KeepsAudio` is not ported — with the exact rate always there, nothing is
/// ever passed through. Every one of the C#'s cases asks for no pass-through.
#[test]
fn a_track_is_passed_through_only_when_the_profile_already_takes_it() {
    for (channels, sample_rate, row) in [
        (Some(1), Some(44_100), 64_000),
        (Some(2), Some(48_000), 128_000),
        (Some(6), Some(48_000), 128_000),
        (None, Some(48_000), 128_000),
        (Some(2), None, 128_000),
    ] {
        let at_row = target(1280, 720, 30.0, 2_000_000, Some(row));
        assert!(
            video_attempts(&at_row, 0, 30, 1, channels, sample_rate, false)
                .iter()
                .all(|attempt| !attempt.keep_audio)
        );
    }
    // A track above its row is never among the attempts as itself.
    let loud = target(1280, 720, 30.0, 2_000_000, Some(128_000));
    assert!(
        !video_attempts(&loud, 0, 30, 1, Some(2), Some(48_000), false)
            .iter()
            .any(|attempt| attempt.keep_audio)
    );
}

/// No audio track in, none out: a transcode does not invent silence.
#[test]
fn a_silent_clip_asks_for_no_audio() {
    let silent = target(640, 360, 25.0, 417_000, None);
    let attempts = video_attempts(&silent, 0, 25, 1, None, None, false);
    assert_eq!(2, attempts.len());
    assert!(attempts.iter().all(|attempt| attempt.audio.is_none()));
    assert_eq!(
        (640, 360, 25, 1, 417_000),
        (
            attempts[0].width,
            attempts[0].height,
            attempts[0].frame_rate_numerator,
            attempts[0].frame_rate_denominator,
            attempts[0].bitrate
        )
    );
}

/// The read-back: the sides exactly and the SAME turn. A frame turned into the pixels, a turn lost,
/// or a turn the other way are all failures — each is a video that plays sideways, squashed or
/// upside down.
#[test]
fn a_result_matches_only_when_it_is_what_was_asked_for() {
    let asked = encoding(
        1280,
        720,
        90,
        2_000_000,
        30,
        1,
        H264Profile::High,
        None,
        false,
    );
    assert!(matches(&asked, 1280, 720, 90));
    assert!(!matches(&asked, 720, 1280, 0), "turned into the pixels");
    assert!(!matches(&asked, 1280, 720, 0), "the turn lost");
    assert!(!matches(&asked, 1280, 720, 270), "turned the other way");
    assert!(!matches(&asked, 1282, 720, 90), "not the size asked for");
}

/// A 1080p60 source asked for at 30 and written at 60 has a bitrate worked out for half its frames,
/// and is outside the profile's "at most 30": a failed transcode. A rate BELOW the one asked for is
/// fine — no transcoder invents frames — and so is NTSC's hair under 30.
#[test]
fn a_result_faster_than_asked_is_not_what_was_asked_for() {
    let thirty = encoding(
        1280,
        720,
        0,
        2_000_000,
        30,
        1,
        H264Profile::High,
        None,
        false,
    );
    assert!(!frame_rate_came_out(&thirty, Some(60.0)));
    assert!(!frame_rate_came_out(&thirty, Some(60_000.0 / 1001.0)));
    assert!(!frame_rate_came_out(&thirty, Some(30.6)));
    assert!(frame_rate_came_out(&thirty, Some(30.0)));
    assert!(frame_rate_came_out(&thirty, Some(30.5)));
    assert!(frame_rate_came_out(&thirty, Some(30_000.0 / 1001.0)));
    assert!(
        frame_rate_came_out(&thirty, Some(12.0)),
        "a slow source stays slow"
    );
    assert!(
        frame_rate_came_out(&thirty, None),
        "a rate nobody can read is not judged"
    );

    let film = VideoEncoding {
        frame_rate_numerator: 24_000,
        frame_rate_denominator: 1001,
        ..thirty
    };
    assert!(frame_rate_came_out(&film, Some(24.0)));
    assert!(!frame_rate_came_out(&film, Some(25.0)));
}

/// An encoder that takes 64 000 and writes its own 96 000 raised a bitrate without refusing
/// anything (rule B). A tenth either way is an encoder's arithmetic, not a different rate.
#[test]
fn an_audio_rate_the_encoder_chose_for_itself_is_not_the_rate_asked_for() {
    assert!(!audio_rate_came_out(64_000, Some(96_000)));
    assert!(!audio_rate_came_out(112_000, Some(128_000)));
    assert!(audio_rate_came_out(64_000, Some(64_000)));
    assert!(audio_rate_came_out(128_000, Some(131_072)));
    assert!(audio_rate_came_out(128_000, Some(140_800)));
    assert!(!audio_rate_came_out(128_000, Some(140_801)));
    assert!(
        audio_rate_came_out(128_000, Some(96_000)),
        "under is never a raise"
    );
    assert!(audio_rate_came_out(64_000, Some(0)), "not stated");
    assert!(audio_rate_came_out(64_000, None));
}

#[test]
fn a_recording_or_a_re_encode_is_aac_in_the_channels_asked_for() {
    let note = voice_note_attempts()[0];
    assert!(aac_came_out(&note, "aac", Some(1)));
    assert!(
        aac_came_out(&note, "aac", Some(0)),
        "a count not stated is not judged"
    );
    assert!(aac_came_out(&note, "aac", None));
    assert!(
        !aac_came_out(&note, "aac", Some(2)),
        "stereo where mono was asked for"
    );
    assert!(!aac_came_out(&note, "mp3", Some(1)));
    assert!(!aac_came_out(&note, "unknown", Some(1)));
}

/// A mix (Rule E, issue #98) is asked for as the one track it becomes: stereo when any voice is, at
/// the highest rate any has, at the profile's row capped by the voices' rates together — never
/// passed through — while the picture is asked for exactly as the planner's own attempts ask for it.
#[test]
fn a_mix_is_asked_for_as_the_one_track_it_becomes() {
    // The decoder showed the mono microphone, so the plan's audio is mono's 64 000; the app's sound
    // is stereo.
    let planned_target = target(1280, 720, 30.0, 2_000_000, Some(64_000));
    let mix = MixSound {
        channels: Some(2),
        sample_rate: Some(48_000),
        bitrate: Some(192_000),
    };
    let attempts = mix_attempts(&planned_target, 0, 30, 1, &mix, false);
    assert_eq!(
        vec![
            (H264Profile::High, aac(48_000, 2, 128_000), false),
            (H264Profile::Main, aac(48_000, 2, 128_000), false),
        ],
        attempts
            .iter()
            .map(|attempt| (attempt.profile, attempt.audio.unwrap(), attempt.keep_audio))
            .collect::<Vec<_>>()
    );
    let planned = video_attempts(&planned_target, 0, 30, 1, Some(1), Some(48_000), false)[0];
    for attempt in &attempts {
        assert_eq!(
            VideoEncoding {
                audio: attempt.audio,
                profile: attempt.profile,
                ..planned
            },
            *attempt
        );
    }
}

/// ADAPTED: two mono voices mix to mono at 64 000 under High and under Main — no encoder's 96 000.
#[test]
fn two_mono_voices_mix_to_mono_and_never_raise_their_rate() {
    let planned_target = target(1280, 720, 30.0, 2_000_000, Some(64_000));
    let mono = |bitrate| MixSound {
        channels: Some(1),
        sample_rate: Some(44_100),
        bitrate,
    };
    assert_eq!(
        vec![
            (H264Profile::High, aac(44_100, 1, 64_000)),
            (H264Profile::Main, aac(44_100, 1, 64_000)),
        ],
        mix_attempts(&planned_target, 0, 30, 1, &mono(Some(128_000)), false)
            .iter()
            .map(|attempt| (attempt.profile, attempt.audio.unwrap()))
            .collect::<Vec<_>>()
    );
    // Two thin voices together under the profile's row: the row is capped by them.
    assert_eq!(
        vec![Some(40_000), Some(40_000)],
        mix_attempts(&planned_target, 0, 30, 1, &mono(Some(40_000)), false)
            .iter()
            .map(|attempt| attempt.audio.map(|audio| audio.bitrate))
            .collect::<Vec<_>>()
    );
    // Nothing stated: stereo's row, at 44.1 kHz.
    let unstated = MixSound {
        channels: None,
        sample_rate: None,
        bitrate: None,
    };
    assert_eq!(
        Some(aac(44_100, 2, 128_000)),
        mix_attempts(&planned_target, 0, 30, 1, &unstated, false)[0].audio
    );
}

/// ADAPTED: a mix is the transcode's own pipeline with an `audiomixer`, so a turned source is asked
/// for ONCE each way of asking, with its turn carried as metadata exactly as a transcode's is —
/// Windows asked a composition both ways because it could not know which a composition does.
#[test]
fn a_turned_source_is_also_asked_for_upright() {
    let portrait = target(720, 1280, 30.0, 2_000_000, Some(128_000));
    let stereo = MixSound {
        channels: Some(2),
        sample_rate: Some(48_000),
        bitrate: Some(256_000),
    };
    let attempts = mix_attempts(&portrait, 90, 30, 1, &stereo, true);
    assert_eq!(
        vec![
            (H264Profile::High, 1280, 720, 90),
            (H264Profile::Main, 1280, 720, 90),
        ],
        attempts
            .iter()
            .map(|attempt| (
                attempt.profile,
                attempt.width,
                attempt.height,
                attempt.rotation
            ))
            .collect::<Vec<_>>()
    );
    assert!(attempts.iter().all(|attempt| attempt.to_sdr));
    for attempt in &attempts {
        assert_eq!(
            displayed(720, 1280, 0),
            displayed(
                i64::from(attempt.width),
                i64::from(attempt.height),
                attempt.rotation
            )
        );
    }
    assert_eq!(2, mix_attempts(&portrait, 180, 30, 1, &stereo, false).len());
}

/// A video whose audio the decoder does not show: no track is read, none is asked for, none comes
/// out — and the read-back agrees with itself over a silent upload. A second reader's numbers are
/// the only witness (on Linux, the container's own sample entry).
#[test]
fn sound_the_shell_reads_and_media_foundation_does_not_is_hidden_sound() {
    assert!(hides_audio(false, Some(2), None, None));
    assert!(hides_audio(false, None, Some(48_000), None));
    assert!(hides_audio(false, Some(0), Some(0), Some(96_000)));
    assert!(!hides_audio(false, None, None, None), "a silent clip");
    assert!(
        !hides_audio(false, Some(0), Some(0), Some(0)),
        "a silent clip whose second reader says nought"
    );
    assert!(!hides_audio(true, Some(2), Some(48_000), Some(128_000)));
}

/// HDV's 1440×1080 is shown 16:9 through 4:3 pixels; scaling its stored sides would squash it.
#[test]
fn only_square_or_unstated_pixels_are_scaled() {
    assert!(square_pixels(1, 1));
    assert!(square_pixels(8, 8));
    assert!(square_pixels(0, 0));
    assert!(square_pixels(0, 1));
    assert!(square_pixels(1, 0));
    assert!(!square_pixels(4, 3));
    assert!(!square_pixels(10, 11));
}

// --- what goes ----------------------------------------------------------------------------------

const CEILING: u64 = media::SIZE_LIMIT;

/// Rule D, with the numbers of the reasoning doc's clips.
#[test]
fn a_smaller_result_goes_and_a_bigger_one_is_thrown_away() {
    // A 60 MB clip brought to 8 MB.
    assert_eq!(
        Chosen::Result,
        choose(60_000_000, true, 8_000_000, true, true, CEILING)
    );
    // A thin clip that grew: the original, which could go.
    assert_eq!(
        Chosen::Original,
        choose(3_000_000, true, 3_000_001, true, true, CEILING)
    );
    // Exactly the source's size is not "bigger".
    assert_eq!(
        Chosen::Result,
        choose(3_000_000, true, 3_000_000, true, true, CEILING)
    );
    // A QuickTime container lying about its bytes cannot go as a video: the result is the only
    // thing that can.
    assert_eq!(
        Chosen::Result,
        choose(3_000_000, false, 4_000_000, true, true, CEILING)
    );
}

/// A 400 MB 4K clip is what this is for; one still over the ceiling afterwards is refused as it
/// always was.
#[test]
fn a_result_over_the_ceiling_is_refused_only_when_the_source_could_not_go_either() {
    assert_eq!(
        Chosen::Result,
        choose(400_000_000, false, 40_000_000, true, true, CEILING)
    );
    assert_eq!(
        Chosen::Result,
        choose(400_000_000, false, CEILING, true, true, CEILING)
    );
    assert_eq!(
        Chosen::TooLarge,
        choose(400_000_000, false, CEILING + 1, true, true, CEILING)
    );
    // A sendable source under a result over the ceiling is a bigger result: the source goes.
    assert_eq!(
        Chosen::Original,
        choose(90_000_000, true, CEILING + 1, false, true, CEILING)
    );
}

/// The index in front is asked of a VIDEO. A 40 MB WAV re-encoded to a 4 MB M4A whose index would
/// not move once went as the 40 MB WAV — and an Ogg as the Ogg, which does not play on iOS or
/// macOS.
#[test]
fn an_index_that_could_not_be_moved_costs_a_video_its_transcode_and_a_sound_file_nothing() {
    assert_eq!(
        Chosen::Original,
        choose(60_000_000, true, 8_000_000, false, true, CEILING)
    );
    // The source could not have gone at all: index at the end beats nothing.
    assert_eq!(
        Chosen::Result,
        choose(400_000_000, false, 40_000_000, false, true, CEILING)
    );
    assert_eq!(
        Chosen::Result,
        choose(40_000_000, true, 4_000_000, false, false, CEILING)
    );
    assert_eq!(
        Chosen::Result,
        choose(40_000_000, true, 4_000_000, true, false, CEILING)
    );
}

/// Rule C's promise, over every combination: a source that could go is NEVER refused, whatever its
/// transcode turned out to be; nothing over the ceiling is ever the thing sent; and the original is
/// only ever named when it can go.
#[test]
fn no_send_that_worked_before_is_lost_to_its_transcode() {
    let sizes = [
        1,
        1_000,
        3_000_000,
        CEILING - 1,
        CEILING,
        CEILING + 1,
        4 * CEILING,
    ];
    for source in sizes {
        for result in sizes {
            for sendable in [true, false] {
                // Sendable is within the ceiling by definition (`media_plan::sendable`).
                if sendable && source > CEILING {
                    continue;
                }
                for moov_first in [true, false] {
                    for video in [true, false] {
                        let chosen = choose(source, sendable, result, moov_first, video, CEILING);
                        let said = format!(
                            "{source} → {result}, sendable {sendable}, moov first {moov_first}, video {video}: {chosen:?}"
                        );
                        assert!(!sendable || chosen != Chosen::TooLarge, "{said}");
                        assert!(chosen != Chosen::Result || result <= CEILING, "{said}");
                        assert!(chosen != Chosen::Original || sendable, "{said}");
                        assert!(
                            chosen != Chosen::Result || !sendable || result <= source,
                            "{said}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn a_transcode_has_a_ceiling() {
    assert_eq!(Duration::from_secs(5 * 60), transcode_ceiling(Some(60_000)));
    assert_eq!(Duration::from_secs(10 * 60), transcode_ceiling(None));
    assert_eq!(Duration::from_secs(10 * 60), transcode_ceiling(Some(0)));
}

/// The reasoning doc's fourth fixture: a WAV becomes AAC 128k; a 128 kbit/s MP3 is untouched.
#[test]
fn a_wav_is_re_encoded_and_a128k_mp3_is_not() {
    let wav = AudioSource {
        container: "audio/wav".into(),
        codec: "pcm".into(),
        channels: Some(2),
        bitrate: Some(1_411_200),
        size_bytes: 31_752_000,
        duration_ms: Some(180_000),
    };
    let plan = plan_audio(&wav);
    assert_eq!(AudioPlan::Transcode { bitrate: 128_000 }, plan);
    let AudioPlan::Transcode { bitrate } = plan else {
        unreachable!()
    };
    assert_eq!(
        vec![aac(44_100, 2, 128_000)],
        audio_attempts(bitrate as i64, wav.channels.map(i64::from), Some(44_100))
    );
    assert_eq!(
        AudioPlan::Keep,
        plan_audio(&AudioSource {
            container: "audio/mpeg".into(),
            codec: "mp3".into(),
            channels: Some(2),
            bitrate: Some(128_000),
            size_bytes: 2_880_000,
            duration_ms: Some(180_000),
        })
    );
}

/// Not a C# test: the ceiling is the animation path's own rule, so both say the same — and a length
/// too large to triple saturates instead of wrapping.
#[test]
fn the_ceiling_is_the_animation_paths_and_saturates() {
    for duration in [None, Some(0), Some(-5), Some(2100), Some(60_000)] {
        assert_eq!(
            fc_logic::animation_preparing::transcode_ceiling(duration),
            transcode_ceiling(duration)
        );
    }
    assert!(transcode_ceiling(Some(i64::MAX)) >= Duration::from_secs(120));
}

/// Not a C# test: the log names an attempt by its numbers alone.
#[test]
fn an_attempt_reads_as_its_numbers() {
    let attempt = encoding(
        1280,
        720,
        90,
        2_000_000,
        30,
        1,
        H264Profile::High,
        Some(aac(48_000, 2, 128_000)),
        true,
    );
    assert_eq!(
        "H.264 High 1280x720 turned 90, 2000000 bit/s, 30/1 fps, AAC 48000 Hz 2 ch 128000 bit/s, to SDR",
        attempt.to_string()
    );
    assert_eq!(
        "H.264 Main 480x480 turned 0, 500000 bit/s, 30/1 fps, no audio, a keyframe every 60 at most",
        VideoEncoding {
            profile: H264Profile::Main,
            width: 480,
            height: 480,
            rotation: 0,
            bitrate: 500_000,
            audio: None,
            to_sdr: false,
            keyframe_spacing: Some(60),
            ..attempt
        }
        .to_string()
    );
}
