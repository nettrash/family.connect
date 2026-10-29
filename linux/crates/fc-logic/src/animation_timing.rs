//! The TIMING half of an animated picture sent as a silent looping video (`AnimationTiming.cs`;
//! issue #96; docs/protocol.md, "Animations", "The file") — what becomes of an animation: the
//! video's size, its frames in order, how long it is and its bitrate. Decoding the frames and
//! encoding the video are each platform's own.
//!
//! `fc_text::animation` is the reference, and everything it has is called: the constants, the
//! three delay rules, the profile's size and bitrate (`fc_text::media_plan`). [`plan`] alone is
//! written out again, as the C# writes it: the reference walks every TICK, which for a WebP that
//! states a frame of four and a half hours (`ANMF` durations go to 16 777 215 ms) is half a million
//! turns of a loop per frame — and a picture of a few thousand such frames would hold the one
//! thread this client runs on for seconds. This walks every FRAME (a frame's ticks are those from
//! the first one at or after its start to the first one at or after the next frame's start), which
//! is the same answer: the vectors `fc_text` prints (`animation-plan-vectors.json`) and a
//! tick-by-tick walk in the tests hold the two to each other.
//!
//! THE GRID. The animation is SAMPLED thirty times a second: at each tick the frame that is
//! showing; ticks in a row that show the same frame are one video frame, that many ticks long. A
//! held frame stays held, nothing runs faster than 30 a second, and a slow animation is never given
//! frames it did not have. A flash no tick lands on is not in the video.
//!
//! THE SHORTEST FRAME. A frame stated at 10 ms or less is shown for 100 ms — the browsers' rule,
//! for the many GIFs that say "0" and mean "the default".

use fc_text::animation as reference;
use fc_text::media_plan;

/// Samples a second: the grid, and the timescale every duration in a plan is counted in.
pub const TICKS_PER_SECOND: i32 = reference::TICKS_PER_SECOND as i32;

/// A frame stated at this or less is shown for [`DEFAULT_FRAME_MS`].
pub const SHORTEST_FRAME_MS: u32 = reference::SHORTEST_FRAME_MS;

/// How long such a frame is shown.
pub const DEFAULT_FRAME_MS: u32 = reference::DEFAULT_FRAME_MS;

/// What shows through a transparent pixel: a video has none.
pub const BACKGROUND_RGB: [u8; 3] = reference::BACKGROUND_RGB;

/// One frame of an animation's video: which source frame (0-based, composited) and for how many
/// ticks of 1/30 s.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AnimationSample {
    pub frame: i32,
    pub ticks: i32,
}

impl AnimationSample {
    pub fn new(frame: i32, ticks: i32) -> Self {
        AnimationSample { frame, ticks }
    }
}

/// What an animated picture becomes (docs/protocol.md, "Animations", "The file").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnimationPlan {
    /// Even, the source's orientation, never larger than the source — the video profile's size
    /// rule.
    pub width: i32,
    pub height: i32,
    /// In order; together they last [`total_ticks`](Self::total_ticks).
    pub samples: Vec<AnimationSample>,
    pub total_ticks: i32,
    /// `total_ticks` in milliseconds, rounded half up — the upload's `duration_ms`.
    pub duration_ms: i64,
    /// The profile's bitrate for the target size at 30 frames a second.
    pub video_bitrate: i64,
}

impl AnimationPlan {
    /// The tick each sample starts on, in order — the video's presentation times on the
    /// 30-a-second grid.
    pub fn start_ticks(&self) -> Vec<i64> {
        let mut at = 0i64;
        self.samples
            .iter()
            .map(|sample| {
                let start = at;
                at += i64::from(sample.ticks);
                start
            })
            .collect()
    }

    /// Each sample's ticks, in order — what [`animation_mp4::exact_ticks`](crate::animation_mp4::exact_ticks)
    /// makes the encoded file say.
    pub fn ticks(&self) -> Vec<i32> {
        self.samples.iter().map(|sample| sample.ticks).collect()
    }
}

/// A GIF frame's delay — hundredths of a second, as its Graphic Control Extension states it — in
/// milliseconds.
pub fn gif_delay_ms(centiseconds: u16) -> u32 {
    reference::gif_delay_ms(centiseconds)
}

/// An APNG frame's delay from its `fcTL`: `num ÷ den` seconds, a `den` of 0 meaning hundredths,
/// in whole milliseconds rounded half up.
pub fn apng_delay_ms(numerator: u16, denominator: u16) -> u32 {
    reference::apng_delay_ms(numerator, denominator)
}

/// How long a frame stated at `stated_ms` is shown: as stated, unless 10 ms or less.
pub fn frame_duration_ms(stated_ms: u32) -> u32 {
    reference::frame_duration_ms(stated_ms)
}

/// The plan for an animation of `width × height` whose frames are stated to last `stated_ms` each,
/// in the source's order. `None` when there is no size, no frame, or a size that rounds to nothing.
///
/// The C# takes `long` sides; a side past `u32` (which no animated picture's header can state —
/// every [`AnimationLayout`](crate::animated_pictures::AnimationLayout) side is an `i32`) is
/// answered `None` here rather than given a size the reference cannot compute.
pub fn plan(width: i64, height: i64, stated_ms: &[u32]) -> Option<AnimationPlan> {
    if width <= 0 || height <= 0 || stated_ms.is_empty() {
        return None;
    }
    let (target_width, target_height) =
        media_plan::target_size(u32::try_from(width).ok()?, u32::try_from(height).ok()?);
    if target_width == 0 || target_height == 0 {
        return None;
    }
    let target_width = i32::try_from(target_width).ok()?;
    let target_height = i32::try_from(target_height).ok()?;
    // Where each frame starts, in milliseconds; and where the last one ends.
    let mut starts = Vec::with_capacity(stated_ms.len());
    let mut total: u64 = 0;
    for &stated in stated_ms {
        starts.push(total);
        total += u64::from(frame_duration_ms(stated));
    }
    let tps = u64::from(reference::TICKS_PER_SECOND);
    // The first tick at or after `ms` (tick × 1000 ≥ ms × 30).
    let ceil_ticks = |ms: u64| (ms * tps).div_ceil(1000);
    // Enough ticks to cover the whole animation: ceil(total × 30 ÷ 1000), at least one.
    let total_ticks = ceil_ticks(total).max(1);
    let total_ticks = i32::try_from(total_ticks).ok()?;
    let ticks_end = total_ticks as u64;
    let mut samples = Vec::new();
    for (frame, &start) in starts.iter().enumerate() {
        // The first tick at or after this frame's start, and the first at or after the next's.
        let first = ceil_ticks(start);
        let end = match starts.get(frame + 1) {
            Some(&next) => ceil_ticks(next).min(ticks_end),
            None => ticks_end,
        };
        if end > first {
            samples.push(AnimationSample {
                frame: i32::try_from(frame).ok()?,
                // At most `total_ticks`, which fits.
                ticks: (end - first) as i32,
            });
        }
    }
    Some(AnimationPlan {
        width: target_width,
        height: target_height,
        samples,
        total_ticks,
        duration_ms: ((2 * ticks_end * 1000 + tps) / (2 * tps)) as i64,
        video_bitrate: media_plan::profile_video_bitrate(
            target_width as u32,
            target_height as u32,
            media_plan::MAX_FRAME_RATE,
        ) as i64,
    })
}

/// A tick on the grid as Media Foundation counts time — hundreds of nanoseconds — rounded to the
/// nearest: 1/30 s is not a whole number of them, so each sample's time is placed and its length
/// is what reaches the next one.
pub fn hundreds(tick: i64) -> i64 {
    (tick * 10_000_000 + i64::from(TICKS_PER_SECOND / 2)) / i64::from(TICKS_PER_SECOND)
}

/// A tick on the grid as GStreamer counts time — nanoseconds — rounded to the nearest, as
/// [`hundreds`] does for Media Foundation: the `pts` the shell gives a sample, whose `duration` is
/// what reaches the next sample's (`nanoseconds(start + ticks) − nanoseconds(start)`). Linux's own:
/// the C# has only the hundreds.
pub fn nanoseconds(tick: i64) -> i64 {
    (tick * 1_000_000_000 + i64::from(TICKS_PER_SECOND / 2)) / i64::from(TICKS_PER_SECOND)
}
