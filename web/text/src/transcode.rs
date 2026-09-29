//! The arithmetic of the web client's transcode, beside the decision itself
//! ([`crate::media_plan`]): the numbers a browser's encoders are configured
//! with, and the few choices the protocol leaves to how a platform gets
//! there — which frames to drop, which way to turn a picture, which H.264
//! level to ask for, whether a video's sound is copied or re-encoded
//! (docs/protocol.md, "Preparing media before upload").
//!
//! None of it changes what the planner decided. It is here, rather than in
//! the browser code that uses it, so that it is tested natively like
//! everything else in this crate.

use crate::media_plan::{VideoTarget, FRAME_RATE_TOLERANCE};
use crate::mp4::AAC_LC;

// --- H.264 --------------------------------------------------------------------------------------

/// A key frame at least this often, in seconds of the output: a player
/// seeking a `Range` into the file never has more than this to decode
/// before it can show a picture.
pub const KEYFRAME_SECONDS: f64 = 2.0;

/// The H.264 profiles the protocol names: High, and Main "where an encoder
/// offers nothing else".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum H264Profile {
    High,
    Main,
}

/// H.264's levels from 3.0 up (ITU-T H.264, Table A-1): `level_idc`, the
/// largest frame in macroblocks, and macroblocks a second.
const LEVELS: [(u8, u32, u32); 9] = [
    (30, 1_620, 40_500),
    (31, 3_600, 108_000),
    (32, 5_120, 216_000),
    (40, 8_192, 245_760),
    (41, 8_192, 245_760),
    (42, 8_704, 522_240),
    (50, 22_080, 589_824),
    (51, 36_864, 983_040),
    (52, 36_864, 2_073_600),
];

/// The WebCodecs codec string for an H.264 encoder of `profile` at
/// `width × height` and `frame_rate`: `avc1.PPCCLL`, at the lowest level
/// from 3.0 whose limits hold the frame — its macroblocks, the longest side
/// the level allows (√(8 × MaxFS)), and macroblocks a second. 720p30 is 3.1;
/// an ultra-wide 1706×720 needs 3.2. A frame too big for 5.2 still names
/// 5.2, and the browser's `isConfigSupported` has the last word.
pub fn h264_codec(profile: H264Profile, width: u32, height: u32, frame_rate: f64) -> String {
    let across = width.div_ceil(16);
    let down = height.div_ceil(16);
    let frame = across * down;
    let per_second = (f64::from(frame) * frame_rate).ceil();
    let level = LEVELS
        .iter()
        .find(|(_, max_frame, max_rate)| {
            let longest = f64::from(8 * max_frame).sqrt();
            frame <= *max_frame
                && f64::from(across.max(down)) <= longest
                && per_second <= f64::from(*max_rate)
        })
        .map_or(52, |(level, _, _)| *level);
    // constraint_set1 on Main says "decodable by a Main decoder", as every
    // Main encoder writes it.
    let (profile_idc, constraints) = match profile {
        H264Profile::High => (0x64, 0x00),
        H264Profile::Main => (0x4D, 0x40),
    };
    format!("avc1.{profile_idc:02x}{constraints:02x}{level:02x}")
}

// --- frames -------------------------------------------------------------------------------------

/// Rule 2's "at most 30", by dropping frames, never by blending or
/// re-timing them: a kept frame is shown exactly when the source showed it.
///
/// A bucket of credit that fills at `rate` a second: a frame is kept when
/// there is a whole one to spend. That keeps every other frame of a 60 fps
/// clip, and — unlike cutting time into fixed 1/30 s slots — every other
/// frame of a 59.94 one too, with no hiccup each time the two clocks drift
/// past each other. The bucket holds a frame and a quarter at most: enough
/// to carry a 31 fps clip's remainder from frame to frame (it loses about
/// one frame in nine, not half of them), too little for the 0.1 % a 59.94
/// clip leaves over to add up to an extra frame, and never enough for a gap
/// in the source to be "made up" by a burst after it. A 50 fps clip comes
/// out at 25, every other frame — smooth, where 30 would have to alternate
/// short and long frames.
#[derive(Debug, Clone)]
pub struct FrameGate {
    rate: f64,
    credit: f64,
    last_seen: Option<f64>,
}

impl FrameGate {
    pub fn new(rate: f64) -> Self {
        FrameGate {
            rate,
            credit: 0.0,
            last_seen: None,
        }
    }

    /// Whether the frame shown at `seconds` is kept. Frames must come in
    /// presentation order, as a decoder hands them out; the first is always
    /// kept.
    pub fn keep(&mut self, seconds: f64) -> bool {
        let Some(last) = self.last_seen.replace(seconds) else {
            return true;
        };
        let elapsed = (seconds - last).max(0.0);
        self.credit = (self.credit + elapsed * self.rate).min(1.25);
        // A millionth of a frame of slack: 2/60 s × 30 is not exactly 1 in
        // binary, and must count as a whole frame.
        if self.credit >= 1.0 - 1e-6 {
            self.credit -= 1.0;
            true
        } else {
            false
        }
    }
}

/// The gate for a source at `source_rate` going to `target_rate` — None
/// when the planner kept the source's own rate, which is every frame. A
/// source at 29.97 or 30.5 loses nothing; one at 60, or whose rate the
/// reader could not tell, is held to the target.
pub fn frame_gate(source_rate: Option<f64>, target_rate: f64) -> Option<FrameGate> {
    match crate::media_plan::known_frame_rate(source_rate) {
        Some(rate) if rate <= FRAME_RATE_TOLERANCE => None,
        _ => Some(FrameGate::new(target_rate)),
    }
}

/// Where a decoded frame is drawn on the output canvas so that it comes out
/// the right way up: `transform` for `setTransform(a, b, c, d, e, f)`, and
/// the size to draw the frame at in that space.
///
/// A phone's portrait clip is a landscape frame with a 90° turn in its
/// matrix. The output carries no matrix — a player that ignores one would
/// show it on its side — so the turn is drawn into the pixels: the canvas is
/// the DISPLAYED size, and the frame is drawn at its own (unturned) shape,
/// turned.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub transform: [f64; 6],
    pub width: f64,
    pub height: f64,
}

/// The placement for a clockwise `rotation` (0, 90, 180, 270) onto a canvas
/// of the target's displayed `width × height`.
pub fn placement(rotation: u16, width: u32, height: u32) -> Placement {
    let (w, h) = (f64::from(width), f64::from(height));
    match rotation % 360 {
        // (x, y) → (w − y, x): the frame's top edge becomes the right.
        90 => Placement {
            transform: [0.0, 1.0, -1.0, 0.0, w, 0.0],
            width: h,
            height: w,
        },
        180 => Placement {
            transform: [-1.0, 0.0, 0.0, -1.0, w, h],
            width: w,
            height: h,
        },
        // (x, y) → (y, h − x): the top edge becomes the left.
        270 => Placement {
            transform: [0.0, -1.0, 1.0, 0.0, 0.0, h],
            width: h,
            height: w,
        },
        _ => Placement {
            transform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            width: w,
            height: h,
        },
    }
}

/// Whether transcoding a source OVER the ceiling could produce something
/// under it: the target's rates over the clip's length, with room for an
/// encoder that comes in at half the rate it was asked for. A source within
/// the ceiling always could; so could one whose length is unknown. Only a
/// transcode whose result would certainly be refused anyway is skipped —
/// rule C then does exactly what it would have done after it.
pub fn could_fit(
    source_bytes: u64,
    ceiling_bytes: u64,
    target: &VideoTarget,
    duration_ms: Option<u64>,
) -> bool {
    if source_bytes <= ceiling_bytes {
        return true;
    }
    let Some(ms) = duration_ms.filter(|&ms| ms > 0) else {
        return true;
    };
    let bits_per_second = target.video_bitrate + target.audio_bitrate.unwrap_or(0);
    let expected = bits_per_second.saturating_mul(ms) / 8_000;
    expected / 2 <= ceiling_bytes
}

// --- sound --------------------------------------------------------------------------------------

/// The rate a re-encoded sound runs at: 44.1 kHz for a source in that
/// family (22.05, 44.1, 88.2 kHz), 48 kHz for everything else. The platform
/// AAC encoders a browser uses take only these two — Chrome's on macOS
/// refuses 16, 22.05, 24 and 32 kHz outright, and Windows' Media Foundation
/// encoder is the same — and a rate from its own family resamples cleanly.
pub fn audio_output_rate(source_rate: Option<u32>) -> u32 {
    match source_rate {
        Some(rate) if rate > 0 && rate % 11_025 == 0 => 44_100,
        _ => 48_000,
    }
}

/// How the sound in a video reaches the audio-in-video row (AAC-LC,
/// 128 000 stereo, 64 000 mono, never above the source's).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioRoute {
    /// Already AAC-LC at no more than its row's rate: its frames are copied
    /// as they are. Re-encoding them at the rate they already have would
    /// cost a generation of quality and save nothing.
    Copy,
    /// Decoded and re-encoded as AAC-LC at this rate, with this many
    /// channels.
    Encode { bitrate: u64, channels: u32 },
}

/// The route for a video's sound: `object_type` from its AudioSpecificConfig
/// (2 is AAC-LC; None when it is not AAC at all), its channels, its rate in
/// bit/s as the index gives it, and the planner's target for it.
///
/// None when this client cannot bring it to the row: more than two
/// channels, or a count it cannot tell. Down-mixing surround means knowing
/// which channel is which, and decoders do not agree on the order they hand
/// them out in — so such a clip takes rule C instead, and goes as it would
/// have before.
pub fn audio_route(
    object_type: Option<u8>,
    channels: Option<u32>,
    source_bitrate: Option<u64>,
    target_bitrate: u64,
) -> Option<AudioRoute> {
    let channels = channels.filter(|&count| count == 1 || count == 2)?;
    let within = source_bitrate.is_some_and(|rate| rate > 0 && rate <= target_bitrate);
    if object_type == Some(AAC_LC) && within {
        return Some(AudioRoute::Copy);
    }
    Some(AudioRoute::Encode {
        bitrate: target_bitrate,
        channels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_level_is_the_lowest_that_holds_the_frame() {
        assert_eq!(h264_codec(H264Profile::High, 1280, 720, 30.0), "avc1.64001f");
        assert_eq!(h264_codec(H264Profile::High, 720, 1280, 30.0), "avc1.64001f");
        assert_eq!(h264_codec(H264Profile::Main, 1280, 720, 30.0), "avc1.4d401f");
        assert_eq!(h264_codec(H264Profile::High, 640, 360, 30.0), "avc1.64001e");
        assert_eq!(h264_codec(H264Profile::High, 176, 144, 15.0), "avc1.64001e");
        // 107 × 45 macroblocks is past 3.1's 3 600.
        assert_eq!(h264_codec(H264Profile::High, 1706, 720, 30.0), "avc1.640020");
        // 720 × 720 at 30: 2 025 macroblocks, past 3.0's 1 620.
        assert_eq!(h264_codec(H264Profile::High, 720, 720, 30.0), "avc1.64001f");
        // A panorama: 45 × 250 macroblocks — too long a side below 5.0.
        assert_eq!(h264_codec(H264Profile::High, 720, 4000, 30.0), "avc1.640032");
        // Past everything: 5.2 all the same.
        assert_eq!(h264_codec(H264Profile::High, 720, 20000, 30.0), "avc1.640034");
    }

    fn kept(rate: f64, times: impl Iterator<Item = f64>) -> Vec<f64> {
        let mut gate = FrameGate::new(rate);
        times.filter(|&t| gate.keep(t)).collect()
    }

    #[test]
    fn sixty_frames_a_second_become_thirty_by_keeping_every_other() {
        let times = (0..120).map(|frame| f64::from(frame * 10) / 600.0);
        let kept = kept(30.0, times);
        assert_eq!(kept.len(), 60);
        for (index, time) in kept.iter().enumerate() {
            assert!((time - index as f64 / 30.0).abs() < 1e-9, "{index}: {time}");
        }
        // 59.94 keeps EVERY other frame too — two minutes of it, not one
        // hiccup where the clocks drift apart.
        let times: Vec<f64> = (0..7_200).map(|frame| f64::from(frame * 1001) / 60_000.0).collect();
        let kept = self::kept(30.0, times.iter().copied());
        assert_eq!(kept.len(), 3_600);
        for (index, time) in kept.iter().enumerate() {
            assert_eq!(*time, times[2 * index]);
        }
        // 120 and 240 keep every fourth and every eighth.
        assert_eq!(self::kept(30.0, (0..480).map(|f| f64::from(f) / 120.0)).len(), 120);
        assert_eq!(self::kept(30.0, (0..960).map(|f| f64::from(f) / 240.0)).len(), 120);
    }

    #[test]
    fn no_source_comes_out_faster_than_thirty() {
        for (ticks, timescale) in [
            (1001, 60_000), // 59.94
            (1, 120),
            (1, 50),
            (1, 31),
            (1001, 48_000), // 47.95
            (512, 15_360),  // 30 exactly: nothing dropped
            (1, 24),        // below: nothing dropped
        ] {
            let times: Vec<f64> = (0..3_000)
                .map(|frame| f64::from(frame * ticks) / f64::from(timescale))
                .collect();
            let kept = kept(30.0, times.iter().copied());
            let seconds = times.last().unwrap();
            let source_rate = f64::from(timescale) / f64::from(ticks);
            // Over the whole clip: at most 30 a second (and the first frame).
            assert!(kept.len() as f64 <= seconds * 30.0 + 1.0, "{ticks}/{timescale}: {}", kept.len());
            // And never starved: at least every n-th frame, for the
            // smallest n that brings the source to 30 — all of them, for a
            // source at 30 or below.
            let every = (source_rate / 30.0 - 1e-9).ceil().max(1.0);
            let floor = (times.len() as f64 / every).floor() - 1.0;
            assert!(kept.len() as f64 >= floor, "{ticks}/{timescale}: {} of {floor}", kept.len());
            // In any one second, never more than a single frame over.
            for window in kept.windows(32) {
                assert!(window[31] - window[0] >= 1.0, "{ticks}/{timescale}");
            }
        }
    }

    #[test]
    fn a_gap_in_the_source_is_not_made_up_by_a_burst_after_it() {
        // 60 fps, a half-second hole, then 60 fps again.
        let mut times: Vec<f64> = (0..60).map(|f| f64::from(f) / 60.0).collect();
        times.extend((90..150).map(|f| f64::from(f) / 60.0));
        let kept = kept(30.0, times.into_iter());
        for pair in kept.windows(3) {
            assert!(pair[2] - pair[0] >= 1.0 / 30.0, "{pair:?}");
        }
    }

    #[test]
    fn a_rate_the_planner_kept_drops_nothing() {
        assert!(frame_gate(Some(29.97), 29.97).is_none());
        assert!(frame_gate(Some(30.5), 30.5).is_none());
        assert!(frame_gate(Some(24.0), 24.0).is_none());
        assert!(frame_gate(Some(60.0), 30.0).is_some());
        assert!(frame_gate(Some(30.51), 30.0).is_some());
        assert!(frame_gate(None, 30.0).is_some(), "an unknown rate is held to 30");
        assert!(frame_gate(Some(f64::NAN), 30.0).is_some());
    }

    /// Where a point of the (unturned) frame lands on the canvas.
    fn land(placement: &Placement, x: f64, y: f64) -> (f64, f64) {
        let [a, b, c, d, e, f] = placement.transform;
        (a * x + c * y + e, b * x + d * y + f)
    }

    #[test]
    fn a_turned_frame_is_drawn_upright_and_fills_the_canvas() {
        // A portrait 720 × 1280 target from a landscape frame turned 90°
        // clockwise: the frame is drawn 1280 × 720, and its top-left corner
        // lands top-RIGHT, its bottom-left top-left.
        let turned = placement(90, 720, 1280);
        assert_eq!((turned.width, turned.height), (1280.0, 720.0));
        assert_eq!(land(&turned, 0.0, 0.0), (720.0, 0.0));
        assert_eq!(land(&turned, 0.0, 720.0), (0.0, 0.0));
        assert_eq!(land(&turned, 1280.0, 720.0), (0.0, 1280.0));
        let back = placement(270, 720, 1280);
        assert_eq!(land(&back, 0.0, 0.0), (0.0, 1280.0), "top-left to bottom-left");
        assert_eq!(land(&back, 1280.0, 0.0), (0.0, 0.0));
        let over = placement(180, 1280, 720);
        assert_eq!(land(&over, 0.0, 0.0), (1280.0, 720.0));
        let plain = placement(0, 1280, 720);
        assert_eq!(land(&plain, 1280.0, 720.0), (1280.0, 720.0));
        assert_eq!((plain.width, plain.height), (1280.0, 720.0));
    }

    #[test]
    fn sound_is_resampled_only_within_its_own_family() {
        assert_eq!(audio_output_rate(Some(44_100)), 44_100);
        assert_eq!(audio_output_rate(Some(22_050)), 44_100);
        assert_eq!(audio_output_rate(Some(88_200)), 44_100);
        assert_eq!(audio_output_rate(Some(48_000)), 48_000);
        assert_eq!(audio_output_rate(Some(16_000)), 48_000);
        assert_eq!(audio_output_rate(Some(96_000)), 48_000);
        assert_eq!(audio_output_rate(None), 48_000);
        assert_eq!(audio_output_rate(Some(0)), 48_000);
    }

    #[test]
    fn a_videos_sound_is_copied_only_when_it_is_already_on_the_row() {
        // Stereo AAC-LC at 128 000 or below: copied.
        assert_eq!(audio_route(Some(2), Some(2), Some(125_000), 125_000), Some(AudioRoute::Copy));
        // At 192 000 the planner's target is 128 000: re-encoded.
        assert_eq!(
            audio_route(Some(2), Some(2), Some(192_000), 128_000),
            Some(AudioRoute::Encode { bitrate: 128_000, channels: 2 })
        );
        // HE-AAC is not AAC-LC, whatever its rate.
        assert_eq!(
            audio_route(Some(5), Some(2), Some(48_000), 48_000),
            Some(AudioRoute::Encode { bitrate: 48_000, channels: 2 })
        );
        // Not AAC at all (PCM in a QuickTime movie): re-encoded.
        assert_eq!(
            audio_route(None, Some(1), Some(705_600), 64_000),
            Some(AudioRoute::Encode { bitrate: 64_000, channels: 1 })
        );
        // A rate the index could not give: re-encoded at the target.
        assert_eq!(
            audio_route(Some(2), Some(2), None, 128_000),
            Some(AudioRoute::Encode { bitrate: 128_000, channels: 2 })
        );
        // Surround, or a count nobody could tell: rule C.
        assert_eq!(audio_route(Some(2), Some(6), Some(384_000), 128_000), None);
        assert_eq!(audio_route(Some(2), None, Some(128_000), 128_000), None);
    }

    #[test]
    fn only_a_transcode_that_could_not_fit_is_skipped() {
        let target = VideoTarget {
            width: 1280,
            height: 720,
            frame_rate: 30.0,
            video_bitrate: 2_000_000,
            audio_bitrate: Some(128_000),
        };
        let ceiling = 100 * 1024 * 1024;
        // Within the ceiling: always.
        assert!(could_fit(ceiling, ceiling, &target, Some(3_600_000)));
        // Over it, five minutes: 80 MB expected.
        assert!(could_fit(ceiling + 1, ceiling, &target, Some(300_000)));
        // Over it, twenty minutes: 319 MB expected, more than twice over.
        assert!(!could_fit(ceiling + 1, ceiling, &target, Some(1_200_000)));
        // Unknown length: try.
        assert!(could_fit(ceiling * 4, ceiling, &target, None));
    }
}
