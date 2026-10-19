//! An animated picture sent in a chat as a silent looping video (issue #96,
//! docs/protocol.md "Animations").
//!
//! An animated GIF, WebP or PNG picked in a chat composer is not uploaded as
//! the picture it is: the sender turns it into the profile's MP4 — H.264, no
//! sound — and sends it with `animation: true`, which every client draws as
//! a picture that plays by itself and loops for ever. This is the TIMING half
//! of that, the part four encoders have to agree on: which source frame is
//! shown when, how long the clip is, its size and its bitrate. Decoding the
//! frames and encoding the video are each platform's own.
//!
//! THE GRID. A GIF's frames last whatever their author typed — 20 ms, 70 ms,
//! two seconds — and the profile allows at most 30 frames a second. Dropping
//! the frames that come too soon after the last one kept would lose the
//! wrong ones: a 20 ms flash followed by a frame held for a second would keep
//! the flash for the whole second. So the animation is SAMPLED: at every
//! thirtieth of a second, the frame that is showing at that instant; and
//! samples in a row that show the same frame are one sample, longer. A
//! held frame stays held, nothing ever runs faster than 30 a second, and a
//! slow animation is never given frames it did not have (rule B).
//!
//! THE SHORTEST FRAME. A frame stated at 10 ms or less is shown for 100 ms —
//! what every browser does, for GIF, WebP and PNG alike, because so many GIFs
//! say "0" and mean "the default". Without it a "0 ms" GIF would be one
//! frame long here and a running animation everywhere else.

use crate::media_plan::{profile_video_bitrate, target_size, MAX_FRAME_RATE};

/// Samples a second: the grid the animation is read on, and the timescale
/// every duration in a plan is counted in.
pub const TICKS_PER_SECOND: u32 = 30;

/// A frame stated at this or less is shown for [`DEFAULT_FRAME_MS`].
pub const SHORTEST_FRAME_MS: u32 = 10;

/// How long such a frame is shown.
pub const DEFAULT_FRAME_MS: u32 = 100;

/// What shows through a transparent pixel: a video has no transparency, and
/// a GIF's empty corners are drawn on white wherever it is drawn on a page.
pub const BACKGROUND_RGB: [u8; 3] = [0xFF, 0xFF, 0xFF];

/// A GIF frame's delay, in hundredths of a second as its Graphic Control
/// Extension states it, in milliseconds as stated — before
/// [`frame_duration_ms`] applies the shortest-frame rule.
pub fn gif_delay_ms(centiseconds: u16) -> u32 {
    u32::from(centiseconds) * 10
}

/// An APNG frame's delay from its `fcTL`: `num ÷ den` seconds, a `den` of 0
/// meaning hundredths (the PNG specification's words), in whole milliseconds
/// rounded half up — before [`frame_duration_ms`].
pub fn apng_delay_ms(num: u16, den: u16) -> u32 {
    let den = if den == 0 { 100 } else { u32::from(den) };
    (2 * u32::from(num) * 1000 + den) / (2 * den)
}

/// How long a frame stated at `stated_ms` is shown: as stated, unless it is
/// [`SHORTEST_FRAME_MS`] or less. (A WebP's `ANMF` states milliseconds
/// directly.)
pub fn frame_duration_ms(stated_ms: u32) -> u32 {
    if stated_ms <= SHORTEST_FRAME_MS {
        DEFAULT_FRAME_MS
    } else {
        stated_ms
    }
}

/// One frame of the video: which source frame (0-based, as the source
/// orders them, each one COMPOSITED as a viewer would show it) and for how
/// many ticks of [`TICKS_PER_SECOND`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sample {
    pub frame: usize,
    pub ticks: u32,
}

/// What an animation becomes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnimationPlan {
    /// Even, the source's orientation, never larger than the source: the
    /// video profile's size rule.
    pub width: u32,
    pub height: u32,
    /// In order; together they last `total_ticks`.
    pub samples: Vec<Sample>,
    pub total_ticks: u32,
    /// `total_ticks` in milliseconds, rounded half up — the upload's
    /// `duration_ms`.
    pub duration_ms: u64,
    /// The profile's bitrate for the target size at 30 frames a second.
    pub video_bitrate: u64,
}

/// The plan for an animation of `width × height` whose frames are stated to
/// last `stated_ms` each (in the source's order). None when there is no size
/// or no frame.
pub fn plan(width: u32, height: u32, stated_ms: &[u32]) -> Option<AnimationPlan> {
    if width == 0 || height == 0 || stated_ms.is_empty() {
        return None;
    }
    let (width, height) = target_size(width, height);
    if width == 0 || height == 0 {
        return None;
    }
    // Where each frame starts, in milliseconds; and where the last one ends.
    let mut starts = Vec::with_capacity(stated_ms.len());
    let mut total: u64 = 0;
    for &stated in stated_ms {
        starts.push(total);
        total += u64::from(frame_duration_ms(stated));
    }
    let tps = u64::from(TICKS_PER_SECOND);
    // Enough ticks to cover the whole animation: ceil(total × 30 ÷ 1000).
    let total_ticks = ((total * tps).div_ceil(1000)).max(1);
    let mut samples: Vec<Sample> = Vec::new();
    let mut frame = 0usize;
    for tick in 0..total_ticks {
        // The frame showing at tick ÷ 30 s: the last one that has started.
        // Compared as tick × 1000 against start × 30, in integers.
        while frame + 1 < starts.len() && starts[frame + 1] * tps <= tick * 1000 {
            frame += 1;
        }
        match samples.last_mut() {
            Some(last) if last.frame == frame => last.ticks += 1,
            _ => samples.push(Sample { frame, ticks: 1 }),
        }
    }
    let total_ticks = u32::try_from(total_ticks).ok()?;
    Some(AnimationPlan {
        width,
        height,
        samples,
        total_ticks,
        duration_ms: (2 * u64::from(total_ticks) * 1000 + tps) / (2 * tps),
        video_bitrate: profile_video_bitrate(width, height, MAX_FRAME_RATE),
    })
}

/// Every case [`plan`] is held to here, and every port through the vectors
/// board-oracle prints from it: `(width, height, stated frame durations)`.
pub const PLAN_CASES: &[(u32, u32, &[u32])] = &[
    // A 10 fps GIF: three ticks a frame.
    (320, 240, &[100, 100, 100, 100]),
    // "0" means the default, 100 ms — as does anything at or under 10.
    (320, 240, &[0, 0, 0]),
    (320, 240, &[10, 11, 10]),
    // 50 fps: faster than the grid, so some frames are never sampled.
    (200, 200, &[20, 20, 20, 20, 20, 20, 20, 20, 20, 20]),
    // A flash, then a frame held for a second: the held frame is kept.
    (200, 200, &[20, 1000]),
    // A held frame, then a flash shorter than a tick, then another hold.
    (200, 200, &[500, 20, 500]),
    // One frame.
    (64, 64, &[100]),
    (64, 64, &[1]),
    // 70 ms frames, which do not fall on the grid.
    (480, 270, &[70, 70, 70, 70, 70, 70, 70]),
    // A portrait animation, larger than the profile: scaled, kept portrait.
    (1080, 1920, &[40, 40, 40, 40]),
    // Larger than the profile, landscape.
    (1920, 1080, &[33, 33, 34]),
    // Odd sides are made even.
    (333, 201, &[100, 200]),
    (1, 1, &[100]),
    // A long animation at a common GIF rate.
    (500, 500, &[60; 50]),
    // Mixed stated durations, including a two-second hold.
    (400, 300, &[0, 50, 2000, 30, 30, 30]),
    // Nothing to plan.
    (0, 240, &[100]),
    (320, 0, &[100]),
    (320, 240, &[]),
];

/// `(num, den)` for [`apng_delay_ms`], held to the same vectors.
pub const APNG_DELAY_CASES: &[(u16, u16)] = &[
    (1, 10),
    (0, 0),
    (5, 0),
    (1, 3),
    (2, 3),
    (1, 30),
    (100, 1000),
    (1, 1),
    (65535, 65535),
    (1, 65535),
    (65535, 1),
    (7, 0),
];

/// Centiseconds for [`gif_delay_ms`].
pub const GIF_DELAY_CASES: &[u16] = &[0, 1, 2, 7, 10, 100, 65535];

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(plan: &AnimationPlan) -> Vec<(usize, u32)> {
        plan.samples.iter().map(|s| (s.frame, s.ticks)).collect()
    }

    #[test]
    fn a_ten_frames_a_second_animation_is_three_ticks_a_frame() {
        let plan = plan(320, 240, &[100, 100, 100, 100]).unwrap();
        assert_eq!(frames(&plan), vec![(0, 3), (1, 3), (2, 3), (3, 3)]);
        assert_eq!(plan.total_ticks, 12);
        assert_eq!(plan.duration_ms, 400);
        assert_eq!((plan.width, plan.height), (320, 240));
    }

    #[test]
    fn a_frame_of_ten_milliseconds_or_less_lasts_a_tenth_of_a_second() {
        assert_eq!(frame_duration_ms(0), 100);
        assert_eq!(frame_duration_ms(10), 100);
        assert_eq!(frame_duration_ms(11), 11);
        let zero = plan(320, 240, &[0, 0, 0]).unwrap();
        assert_eq!(frames(&zero), vec![(0, 3), (1, 3), (2, 3)]);
    }

    #[test]
    fn a_held_frame_after_a_flash_is_kept_for_its_whole_length() {
        let plan = plan(200, 200, &[20, 1000]).unwrap();
        // The flash starts at 0 and is sampled at tick 0; the held frame
        // starts at 20 ms, before tick 1 (33.3 ms), and holds to 1020 ms.
        assert_eq!(frames(&plan), vec![(0, 1), (1, 30)]);
        assert_eq!(plan.total_ticks, 31);
    }

    #[test]
    fn a_flash_between_two_holds_that_no_tick_lands_on_is_not_sampled() {
        let on_a_tick = plan(200, 200, &[500, 20, 500]).unwrap();
        // 500 ms is tick 15 exactly, which the flash (500–520) owns.
        assert_eq!(frames(&on_a_tick), vec![(0, 15), (1, 1), (2, 15)]);
        let between = plan(200, 200, &[510, 20, 500]).unwrap();
        // 510–530 holds no tick (15 is 500, 16 is 533.3): never shown.
        assert_eq!(frames(&between), vec![(0, 16), (2, 15)]);
    }

    #[test]
    fn nothing_runs_faster_than_thirty_frames_a_second() {
        for &(w, h, stated) in PLAN_CASES {
            let Some(plan) = plan(w, h, stated) else {
                continue;
            };
            assert!(plan.samples.iter().all(|s| s.ticks >= 1));
            assert_eq!(
                plan.samples.iter().map(|s| s.ticks).sum::<u32>(),
                plan.total_ticks
            );
            assert!(
                plan.samples.len() <= stated.len(),
                "never more frames than the source"
            );
            assert!(
                plan.samples.windows(2).all(|p| p[0].frame < p[1].frame),
                "in order, no repeats"
            );
            assert_eq!(plan.width % 2, 0);
            assert_eq!(plan.height % 2, 0);
        }
    }

    #[test]
    fn the_size_follows_the_video_profile() {
        let portrait = plan(1080, 1920, &[40]).unwrap();
        assert_eq!((portrait.width, portrait.height), (720, 1280));
        let odd = plan(333, 201, &[100]).unwrap();
        assert_eq!((odd.width, odd.height), (332, 200));
        assert_eq!(
            plan(1, 1, &[100]),
            None,
            "a one-pixel picture rounds to nothing"
        );
    }

    #[test]
    fn there_is_nothing_to_plan_without_a_size_or_a_frame() {
        assert_eq!(plan(0, 240, &[100]), None);
        assert_eq!(plan(320, 0, &[100]), None);
        assert_eq!(plan(320, 240, &[]), None);
    }

    #[test]
    fn delays_are_read_as_each_format_states_them() {
        assert_eq!(gif_delay_ms(7), 70);
        assert_eq!(apng_delay_ms(1, 10), 100);
        assert_eq!(apng_delay_ms(5, 0), 50, "a den of 0 is hundredths");
        assert_eq!(apng_delay_ms(1, 3), 333);
        assert_eq!(apng_delay_ms(2, 3), 667, "half up");
        assert_eq!(apng_delay_ms(65535, 1), 65_535_000);
    }

    #[test]
    fn the_bitrate_is_the_profiles_at_thirty_frames_a_second() {
        let plan = plan(1280, 720, &[100]).unwrap();
        assert_eq!(plan.video_bitrate, 2_000_000);
        let small = plan_small();
        assert_eq!(small.video_bitrate, 250_000, "the floor");
    }

    fn plan_small() -> AnimationPlan {
        plan(64, 64, &[100]).unwrap()
    }
}
