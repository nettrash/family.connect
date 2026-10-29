//! `AnimationTimingTests`: AN ANIMATION'S TIMING, held to `fc_text::animation` case for case (issue
//! #96; docs/protocol.md, "Animations", "The file"). `fixtures/animation-plan-vectors.json` is the
//! file `win/tests/FamilyConnect.Core.Tests/Fixtures` holds, byte for byte — printed by the Rust the
//! web client runs (`win/tools/board-oracle`, `cargo run -- animation-plan`); the Apple and Android
//! ports hold the same bytes.

mod common;

use fc_logic::animation_timing::{self, AnimationSample};
use serde_json::Value;

fn vectors() -> Vec<Value> {
    common::fixture_json("animation-plan-vectors.json")
        .as_array()
        .expect("an array of vectors")
        .clone()
}

fn of(function: &str) -> Vec<Value> {
    vectors()
        .into_iter()
        .filter(|vector| vector["function"] == function)
        .collect()
}

fn name(vector: &Value) -> String {
    vector["name"].as_str().unwrap_or_default().to_string()
}

fn u32_of(value: &Value) -> u32 {
    u32::try_from(value.as_u64().expect("a number")).expect("a u32")
}

fn u16_of(value: &Value) -> u16 {
    u16::try_from(value.as_u64().expect("a number")).expect("a u16")
}

#[test]
fn the_vectors_are_all_here() {
    // Every function the oracle prints is one this port answers: a vector of a kind nobody reads
    // would pass by being ignored.
    for vector in vectors() {
        let function = vector["function"].as_str().unwrap_or_default();
        assert!(
            [
                "constants",
                "gif_delay_ms",
                "apng_delay_ms",
                "frame_duration_ms",
                "plan"
            ]
            .contains(&function),
            "{function}"
        );
    }
    assert_eq!(of("plan").len(), 18);
    assert_eq!(of("apng_delay_ms").len(), 12);
    assert_eq!(of("gif_delay_ms").len(), 7);
    assert_eq!(of("frame_duration_ms").len(), 8);
}

#[test]
fn the_constants_are_the_references() {
    let constants = of("constants");
    assert_eq!(constants.len(), 1);
    let expected = &constants[0]["expected"];
    assert_eq!(
        i64::from(animation_timing::TICKS_PER_SECOND),
        expected["ticks_per_second"].as_i64().unwrap()
    );
    assert_eq!(
        animation_timing::SHORTEST_FRAME_MS,
        u32_of(&expected["shortest_frame_ms"])
    );
    assert_eq!(
        animation_timing::DEFAULT_FRAME_MS,
        u32_of(&expected["default_frame_ms"])
    );
    let background: Vec<u8> = expected["background_rgb"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_u64().unwrap() as u8)
        .collect();
    assert_eq!(background, animation_timing::BACKGROUND_RGB);
}

#[test]
fn every_delay_is_read_as_the_reference_reads_it() {
    for vector in of("gif_delay_ms") {
        let input = &vector["input"];
        assert_eq!(
            u32_of(&vector["expected"]["ms"]),
            animation_timing::gif_delay_ms(u16_of(&input["centiseconds"])),
            "{}",
            name(&vector)
        );
    }
    for vector in of("apng_delay_ms") {
        let input = &vector["input"];
        assert_eq!(
            u32_of(&vector["expected"]["ms"]),
            animation_timing::apng_delay_ms(u16_of(&input["num"]), u16_of(&input["den"])),
            "{}",
            name(&vector)
        );
    }
    for vector in of("frame_duration_ms") {
        assert_eq!(
            u32_of(&vector["expected"]["ms"]),
            animation_timing::frame_duration_ms(u32_of(&vector["input"]["stated_ms"])),
            "{}",
            name(&vector)
        );
    }
}

#[test]
fn every_plan_is_the_references() {
    for vector in of("plan") {
        let name = name(&vector);
        let input = &vector["input"];
        let stated: Vec<u32> = input["stated_ms"]
            .as_array()
            .unwrap()
            .iter()
            .map(u32_of)
            .collect();
        let plan = animation_timing::plan(
            input["width"].as_i64().unwrap(),
            input["height"].as_i64().unwrap(),
            &stated,
        );
        let expected = &vector["expected"];
        if expected.is_null() {
            assert!(plan.is_none(), "{name}");
            continue;
        }
        let plan = plan.unwrap_or_else(|| panic!("{name}: no plan"));
        assert_eq!(
            expected["width"].as_i64().unwrap(),
            i64::from(plan.width),
            "{name}"
        );
        assert_eq!(
            expected["height"].as_i64().unwrap(),
            i64::from(plan.height),
            "{name}"
        );
        assert_eq!(
            expected["total_ticks"].as_i64().unwrap(),
            i64::from(plan.total_ticks),
            "{name}"
        );
        assert_eq!(
            expected["duration_ms"].as_i64().unwrap(),
            plan.duration_ms,
            "{name}"
        );
        assert_eq!(
            expected["video_bitrate"].as_i64().unwrap(),
            plan.video_bitrate,
            "{name}"
        );
        let samples: Vec<AnimationSample> = expected["samples"]
            .as_array()
            .unwrap()
            .iter()
            .map(|pair| {
                AnimationSample::new(
                    pair[0].as_i64().unwrap() as i32,
                    pair[1].as_i64().unwrap() as i32,
                )
            })
            .collect();
        assert_eq!(samples, plan.samples, "{name}");
    }
}

/// A small deterministic generator in place of the C#'s `new Random(96)` — whose sequence is
/// .NET's own — with the same `Next(min, max)` shape: the shapes drawn differ, the rule held does
/// not.
struct Random(u64);

impl Random {
    fn next(&mut self, low: u32, high: u32) -> u32 {
        // SplitMix64.
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        low + (z % u64::from(high - low)) as u32
    }
}

/// The reference walks every TICK; this port walks every FRAME. The two are held to each other
/// here over shapes the vectors do not hold: flashes that land on a tick and between ticks, long
/// holds, the shortest-frame rule.
#[test]
fn walking_the_frames_is_walking_the_ticks() {
    let mut random = Random(96);
    for _ in 0..500 {
        let count = random.next(1, 40);
        let stated: Vec<u32> = (0..count)
            .map(|_| match random.next(0, 4) {
                0 => random.next(0, 12),
                1 => random.next(12, 40),
                2 => random.next(40, 200),
                _ => random.next(200, 3000),
            })
            .collect();
        let plan = animation_timing::plan(640, 480, &stated).unwrap();
        assert_eq!(tick_by_tick(&stated), plan.samples);
        assert_eq!(
            plan.total_ticks,
            plan.samples.iter().map(|sample| sample.ticks).sum::<i32>()
        );
        assert!(plan.samples.len() <= stated.len());
    }
}

/// `fc_text::animation::plan`'s own loop, written out again.
fn tick_by_tick(stated: &[u32]) -> Vec<AnimationSample> {
    let mut starts = Vec::new();
    let mut total = 0u64;
    for &ms in stated {
        starts.push(total);
        total += u64::from(animation_timing::frame_duration_ms(ms));
    }
    let ticks = (total * 30).div_ceil(1000).max(1);
    let mut samples: Vec<AnimationSample> = Vec::new();
    let mut frame = 0usize;
    for tick in 0..ticks {
        while frame + 1 < starts.len() && starts[frame + 1] * 30 <= tick * 1000 {
            frame += 1;
        }
        match samples.last_mut() {
            Some(last) if last.frame == frame as i32 => last.ticks += 1,
            _ => samples.push(AnimationSample::new(frame as i32, 1)),
        }
    }
    samples
}

/// And the reference itself, called: the same plan, sample for sample, whenever it can be had.
#[test]
fn the_frame_walk_is_fc_texts_plan() {
    let mut random = Random(112);
    for _ in 0..200 {
        let count = random.next(1, 30);
        let stated: Vec<u32> = (0..count).map(|_| random.next(0, 2000)).collect();
        let ours = animation_timing::plan(500, 300, &stated).unwrap();
        let theirs = fc_text::animation::plan(500, 300, &stated).unwrap();
        let samples: Vec<AnimationSample> = theirs
            .samples
            .iter()
            .map(|sample| AnimationSample::new(sample.frame as i32, sample.ticks as i32))
            .collect();
        assert_eq!(ours.samples, samples);
        assert_eq!(ours.total_ticks as u32, theirs.total_ticks);
        assert_eq!(ours.duration_ms as u64, theirs.duration_ms);
        assert_eq!(ours.video_bitrate as u64, theirs.video_bitrate);
        assert_eq!(
            (ours.width as u32, ours.height as u32),
            (theirs.width, theirs.height)
        );
    }
}

/// A WebP may state a frame of four and a half hours: the plan is still a handful of samples, made
/// at once.
#[test]
fn an_hours_long_hold_is_one_sample() {
    let plan = animation_timing::plan(320, 240, &[100, 16_777_215, 100]).unwrap();
    assert_eq!(
        plan.samples,
        vec![
            AnimationSample::new(0, 3),
            AnimationSample::new(1, 503_317),
            AnimationSample::new(2, 3)
        ]
    );
}

/// Where each sample starts on the grid, and that tick in Media Foundation's hundreds of
/// nanoseconds, rounded to the nearest: a thirtieth of a second is 333 333⅓ of them.
#[test]
fn each_sample_starts_on_its_tick() {
    let plan = animation_timing::plan(200, 200, &[20, 1000, 500]).unwrap();
    assert_eq!(plan.start_ticks(), vec![0, 1, 31]);
    assert_eq!(animation_timing::hundreds(0), 0);
    assert_eq!(animation_timing::hundreds(1), 333_333);
    assert_eq!(animation_timing::hundreds(2), 666_667);
    assert_eq!(animation_timing::hundreds(30), 10_000_000);
    assert_eq!(animation_timing::hundreds(31), 10_333_333);
}

/// The port's own: the same ticks in GStreamer's nanoseconds, rounded to the nearest, so a
/// sample's duration is what reaches the next one and thirty of them are exactly a second.
#[test]
fn each_tick_in_nanoseconds_is_rounded_to_the_nearest() {
    assert_eq!(animation_timing::nanoseconds(0), 0);
    assert_eq!(animation_timing::nanoseconds(1), 33_333_333);
    assert_eq!(animation_timing::nanoseconds(2), 66_666_667);
    assert_eq!(animation_timing::nanoseconds(30), 1_000_000_000);
    assert_eq!(animation_timing::nanoseconds(31), 1_033_333_333);
    let plan = animation_timing::plan(200, 200, &[20, 1000, 500]).unwrap();
    assert_eq!(plan.ticks(), vec![1, 30, 15]);
}
