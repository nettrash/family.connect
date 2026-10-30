//! `AnimationRenderTests`, the part about what a composited canvas becomes on its way into the
//! encoder (issue #96; docs/protocol.md, "Animations", "The file"): on white, at the plan's size, as
//! BT.709 NV12 — the frames in the plan's order. The file's other four tests, the MP4's sample table
//! made exact afterwards, are `animation_mp4_animation_render.rs`.

use std::cell::RefCell;
use std::rc::Rc;

use fc_core::rt::Cancel;
use fc_logic::animated_pictures::{
    AnimationFormat, AnimationFrame, AnimationLayout, FrameBlend, FrameDisposal,
};
use fc_logic::animation_frames::{AnimationFrames, FrameError, PixelsOf, animation_render};
use fc_logic::animation_timing::{self, AnimationSample};
use futures::executor::LocalPool;

/// `(index, _) => Task.FromResult(answer(index))`.
fn pixels_of(answer: impl Fn(usize) -> Option<Vec<u8>> + 'static) -> PixelsOf {
    Rc::new(move |index| {
        let pixels = answer(index);
        Box::pin(async move { pixels })
    })
}

#[test]
fn transparency_is_drawn_on_white() {
    let white = animation_render::on_white(&[10, 20, 30, 0, 10, 20, 30, 255, 0, 0, 0, 128]);
    assert_eq!(
        white,
        vec![255, 255, 255, 255, 10, 20, 30, 255, 127, 127, 127, 255]
    );
}

#[test]
fn area_averaging_is_the_mean_of_what_each_pixel_covers() {
    // 4 × 2 → 2 × 1: each target pixel is the mean of a 2 × 2 block.
    let four = [
        0, 0, 0, 255, 100, 100, 100, 255, 10, 10, 10, 255, 30, 30, 30, 255, //
        200, 200, 200, 255, 100, 100, 100, 255, 50, 50, 50, 255, 70, 70, 70, 255,
    ];
    assert_eq!(
        animation_render::scale(&four, 4, 2, 2, 1),
        vec![100, 100, 100, 255, 40, 40, 40, 255]
    );
    // 3 → 2 across: the middle pixel is split half and half.
    let three = [0, 0, 0, 255, 90, 90, 90, 255, 180, 180, 180, 255];
    assert_eq!(
        animation_render::scale(&three, 3, 1, 2, 1),
        vec![30, 30, 30, 255, 150, 150, 150, 255]
    );
    // The same size is a copy.
    let same = animation_render::scale(&three, 3, 1, 3, 1);
    assert_eq!(same, three);
    assert_ne!(same.as_ptr(), three.as_ptr());
}

/// BT.709, limited range: white and black at the ends of 16–235, grey at the middle of chroma, the
/// primaries.
#[test]
fn colours_become_bt709_limited_range() {
    assert_eq!(animation_render::y(255, 255, 255), 235);
    assert_eq!(animation_render::y(0, 0, 0), 16);
    assert_eq!(animation_render::cb_cr(255, 255, 255, 1), (128, 128));
    assert_eq!(
        animation_render::cb_cr(4 * 77, 4 * 77, 4 * 77, 4),
        (128, 128)
    );
    // Red: Y 16 + 219 × 0.2126 = 62.6, Cb 128 − 224 × 0.1146 = 102.3, Cr 240.
    assert_eq!(animation_render::y(255, 0, 0), 63);
    assert_eq!(animation_render::cb_cr(255, 0, 0, 1), (102, 240));
    // Green: Y 16 + 219 × 0.7152 = 172.6; blue: 16 + 219 × 0.0722 = 31.8.
    assert_eq!(animation_render::y(0, 255, 0), 173);
    assert_eq!(animation_render::y(0, 0, 255), 32);
    assert_eq!(animation_render::cb_cr(0, 0, 255, 1), (240, 118));
}

#[test]
fn nv12_is_a_whole_luma_plane_then_interleaved_half_size_chroma() {
    // 2 × 2 — white, white, red, red — and 4 × 2 of white.
    let pixels = [
        255, 255, 255, 255, 255, 255, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255,
    ];
    let nv12 = animation_render::nv12(&pixels, 2, 2);
    assert_eq!(nv12.len(), 6);
    assert_eq!(nv12[..4], [235, 235, 63, 63]);
    // The chroma of the four together: half white, half red.
    let (cb, cr) = animation_render::cb_cr(4 * 255, 2 * 255, 2 * 255, 4);
    assert_eq!(nv12[4..], [cb, cr]);
    let white = animation_render::nv12(&[255; 4 * 4 * 2], 4, 2);
    assert_eq!(
        white,
        vec![235, 235, 235, 235, 235, 235, 235, 235, 128, 128, 128, 128]
    );
}

fn solid(count: usize, b: u8, g: u8, r: u8) -> Vec<u8> {
    (0..count).flat_map(|_| [b, g, r, 255]).collect()
}

/// The frames in the plan's order — and EVERY source frame drawn, sampled or not: a 20 ms flash no
/// tick lands on is not a video frame, but what it left on the canvas is under the next one.
#[test]
fn every_frame_is_drawn_and_only_the_sampled_ones_are_given() {
    // 2 × 2: frame 0 red for 510 ms; frame 1 a green left half for 20 ms, never sampled; frame 2 a
    // blue top-right pixel.
    let layout = AnimationLayout::new(
        AnimationFormat::Gif,
        2,
        2,
        vec![
            AnimationFrame::new(0, 0, 2, 2, 510, FrameBlend::Over, FrameDisposal::None),
            AnimationFrame::new(0, 0, 1, 2, 20, FrameBlend::Over, FrameDisposal::None),
            AnimationFrame::new(1, 0, 1, 1, 500, FrameBlend::Over, FrameDisposal::None),
        ],
    );
    let plan = animation_timing::plan(2, 2, &layout.stated_ms()).unwrap();
    assert_eq!(
        plan.samples,
        vec![AnimationSample::new(0, 16), AnimationSample::new(2, 15)]
    );
    let asked: Rc<RefCell<Vec<usize>>> = Rc::default();
    let sources = [
        solid(4, 0, 0, 255),
        solid(2, 0, 255, 0),
        solid(1, 255, 0, 0),
    ];
    let noted = asked.clone();
    let mut frames = AnimationFrames::new(
        layout,
        plan,
        pixels_of(move |index| {
            noted.borrow_mut().push(index);
            Some(sources[index].clone())
        }),
    )
    .unwrap();
    let cancel = Cancel::new();
    let mut pool = LocalPool::new();
    let first = pool.run_until(frames.next(&cancel)).unwrap();
    let second = pool.run_until(frames.next(&cancel)).unwrap();
    assert_eq!(pool.run_until(frames.next(&cancel)), Ok(None));
    assert_eq!(frames.given(), 2);
    assert_eq!(*asked.borrow(), vec![0, 1, 2]);
    assert_eq!(first, Some(solid(4, 0, 0, 255)));
    // Green left (from the unsampled flash), blue top right, red bottom right.
    assert_eq!(
        second,
        Some(vec![
            0, 255, 0, 255, 255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255
        ])
    );
}

#[test]
fn a_frame_that_will_not_decode_ends_the_animation() {
    let layout = AnimationLayout::new(
        AnimationFormat::WebP,
        2,
        2,
        vec![
            AnimationFrame::new(0, 0, 2, 2, 100, FrameBlend::Over, FrameDisposal::None),
            AnimationFrame::new(0, 0, 2, 2, 100, FrameBlend::Over, FrameDisposal::None),
        ],
    );
    let plan = animation_timing::plan(2, 2, &layout.stated_ms()).unwrap();
    let cancel = Cancel::new();
    let mut pool = LocalPool::new();
    let mut undecodable =
        AnimationFrames::new(layout.clone(), plan.clone(), pixels_of(|_| None)).unwrap();
    assert!(matches!(
        pool.run_until(undecodable.next(&cancel)),
        Err(FrameError::Undecodable { frame: 0, .. })
    ));
    // A decoder that answers at another size is not drawn either.
    let mut wrong = AnimationFrames::new(layout, plan, pixels_of(|_| Some(vec![0; 4]))).unwrap();
    assert!(matches!(
        pool.run_until(wrong.next(&cancel)),
        Err(FrameError::Undecodable { .. })
    ));
}

#[test]
fn it_is_scaled_to_the_plans_size() {
    // 333 × 201 becomes 332 × 200 (even), and the frame is what the plan says.
    let layout = AnimationLayout::new(
        AnimationFormat::Apng,
        333,
        201,
        vec![AnimationFrame::new(
            0,
            0,
            333,
            201,
            100,
            FrameBlend::Source,
            FrameDisposal::None,
        )],
    );
    let plan = animation_timing::plan(333, 201, &layout.stated_ms()).unwrap();
    let mut frames = AnimationFrames::new(
        layout,
        plan,
        pixels_of(|_| Some((0..333 * 201).flat_map(|_| [10, 20, 30, 255]).collect())),
    )
    .unwrap();
    let frame = LocalPool::new()
        .run_until(frames.next(&Cancel::new()))
        .unwrap()
        .unwrap();
    assert_eq!(frame.len(), 332 * 200 * 4);
    assert!(frame.chunks(4).all(|pixel| pixel == [10, 20, 30, 255]));
}

/// The port's own: a cancel raised before the next source frame is a cancel — the C#'s
/// `ThrowIfCancellationRequested` — and nothing more is asked of the decoder.
#[test]
fn a_cancel_stops_before_the_next_frame_is_asked_for() {
    let layout = AnimationLayout::new(
        AnimationFormat::Gif,
        1,
        1,
        vec![AnimationFrame::new(
            0,
            0,
            1,
            1,
            100,
            FrameBlend::Source,
            FrameDisposal::None,
        )],
    );
    let plan = animation_timing::plan(2, 2, &layout.stated_ms()).unwrap();
    let asked = Rc::new(RefCell::new(0));
    let counted = asked.clone();
    let mut frames = AnimationFrames::new(
        layout,
        plan,
        pixels_of(move |_| {
            *counted.borrow_mut() += 1;
            Some(vec![0, 0, 0, 255])
        }),
    )
    .unwrap();
    let cancel = Cancel::new();
    cancel.cancel();
    assert_eq!(
        LocalPool::new().run_until(frames.next(&cancel)),
        Err(FrameError::Canceled)
    );
    assert_eq!(*asked.borrow(), 0);
    assert_eq!(frames.given(), 0);
}
