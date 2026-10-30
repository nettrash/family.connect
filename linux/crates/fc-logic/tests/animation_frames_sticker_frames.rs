//! `StickerFramesTests`: an animated sticker's frames as this client draws them in a chat
//! (`animation_frames::sticker_frames`) — composited from the file's own frames, kept transparent,
//! premultiplied, at the size drawn.

use std::rc::Rc;

use fc_core::rt::Cancel;
use fc_logic::animated_pictures::{
    AnimationFormat, AnimationFrame, AnimationLayout, FrameBlend, FrameDisposal,
};
use fc_logic::animation_frames::{PixelsOf, sticker_frames};
use fc_logic::transcode_attempts::OperationCanceled;
use futures::executor::LocalPool;

// Straight BGRA, one pixel each.
const RED: [u8; 4] = [0, 0, 255, 255];
const BLUE: [u8; 4] = [255, 0, 0, 255];
const CLEAR: [u8; 4] = [0, 0, 0, 0];

fn pixels(each: &[[u8; 4]]) -> Vec<u8> {
    each.concat()
}

/// `(index, _) => Task.FromResult(frames[index])`.
fn from(frames: Vec<Option<Vec<u8>>>) -> PixelsOf {
    Rc::new(move |index| {
        let pixels = frames[index].clone();
        Box::pin(async move { pixels })
    })
}

fn compose(
    layout: &AnimationLayout,
    frames: Vec<Option<Vec<u8>>>,
    width: i32,
    height: i32,
) -> Option<Vec<Vec<u8>>> {
    LocalPool::new()
        .run_until(sticker_frames::compose(
            layout,
            &from(frames),
            width,
            height,
            &Cancel::new(),
        ))
        .unwrap()
}

/// Opaque stays itself, transparent stays transparent, and half-covered is premultiplied.
#[test]
fn pixels_are_premultiplied_and_transparency_is_kept() {
    let half_red = [0, 0, 255, 128];
    let drawn = sticker_frames::premultiplied(&pixels(&[RED, CLEAR, half_red]), 3, 1, 3, 1);
    assert_eq!(drawn, pixels(&[RED, CLEAR, [0, 0, 128, 128]]));
}

/// A transparent pixel's colour does not bleed into the edge when the sticker is scaled down: the
/// green of an invisible pixel next to red would come out a muddy dark edge if straight pixels
/// were averaged.
#[test]
fn scaling_down_blends_no_invisible_colour_into_the_edge() {
    let invisible_green = [0, 255, 0, 0];
    let drawn = sticker_frames::premultiplied(&pixels(&[RED, invisible_green]), 2, 1, 1, 1);
    assert_eq!(drawn[1], 0); // no green
    assert_eq!(drawn[2], 128); // half the red
    assert_eq!(drawn[3], 128); // half covered
}

/// Premultiplied BGRA never holds a colour above its alpha, whatever the rounding did.
#[test]
fn no_colour_is_left_above_its_alpha() {
    let faint = [255, 255, 255, 1];
    let drawn = sticker_frames::premultiplied(&pixels(&[faint, faint, faint]), 3, 1, 2, 1);
    for pixel in drawn.chunks(4) {
        assert!(pixel[0] <= pixel[3] && pixel[1] <= pixel[3] && pixel[2] <= pixel[3]);
    }
}

/// Every frame is the CANVAS at that moment, not the file's rectangle: a second frame that only
/// paints the right pixel leaves the first frame's left pixel standing.
#[test]
fn each_frame_is_the_whole_canvas_as_a_viewer_shows_it() {
    let layout = AnimationLayout::new(
        AnimationFormat::Apng,
        2,
        1,
        vec![
            AnimationFrame::new(0, 0, 2, 1, 100, FrameBlend::Source, FrameDisposal::None),
            AnimationFrame::new(1, 0, 1, 1, 100, FrameBlend::Over, FrameDisposal::None),
        ],
    );
    let drawn = compose(
        &layout,
        vec![Some(pixels(&[RED, CLEAR])), Some(BLUE.to_vec())],
        2,
        1,
    )
    .unwrap();
    assert_eq!(drawn.len(), 2);
    assert_eq!(drawn[0], pixels(&[RED, CLEAR]));
    assert_eq!(drawn[1], pixels(&[RED, BLUE]));
}

/// "Restore to background" clears the frame's rectangle to transparent before the next is drawn.
#[test]
fn a_disposed_frame_leaves_transparency_not_white() {
    let layout = AnimationLayout::new(
        AnimationFormat::WebP,
        2,
        1,
        vec![
            AnimationFrame::new(
                0,
                0,
                1,
                1,
                100,
                FrameBlend::Source,
                FrameDisposal::Background,
            ),
            AnimationFrame::new(1, 0, 1, 1, 100, FrameBlend::Source, FrameDisposal::None),
        ],
    );
    let drawn = compose(&layout, vec![Some(RED.to_vec()), Some(BLUE.to_vec())], 2, 1).unwrap();
    assert_eq!(drawn[1], pixels(&[CLEAR, BLUE]));
}

/// A frame that will not decode is not drawn around: the whole animation is answered `None` and
/// drawn by the old way.
#[test]
fn a_frame_that_will_not_decode_gives_no_frames() {
    let layout = AnimationLayout::new(
        AnimationFormat::Apng,
        1,
        1,
        vec![
            AnimationFrame::new(0, 0, 1, 1, 100, FrameBlend::Source, FrameDisposal::None),
            AnimationFrame::new(0, 0, 1, 1, 100, FrameBlend::Source, FrameDisposal::None),
        ],
    );
    assert_eq!(compose(&layout, vec![Some(RED.to_vec()), None], 1, 1), None);
    assert_eq!(
        compose(
            &layout,
            vec![Some(RED.to_vec()), Some(pixels(&[RED, RED]))],
            1,
            1
        ),
        None
    );
}

/// Frames come out at the size the sticker is drawn, not the file's.
#[test]
fn frames_come_out_at_the_drawn_size() {
    let layout = AnimationLayout::new(
        AnimationFormat::Apng,
        4,
        4,
        vec![
            AnimationFrame::new(0, 0, 4, 4, 100, FrameBlend::Source, FrameDisposal::None),
            AnimationFrame::new(0, 0, 4, 4, 100, FrameBlend::Source, FrameDisposal::None),
        ],
    );
    let full = pixels(&[RED; 16]);
    let drawn = compose(&layout, vec![Some(full.clone()), Some(full)], 2, 2).unwrap();
    assert!(drawn.iter().all(|frame| frame.len() == 2 * 2 * 4));
    assert_eq!(drawn[0], pixels(&[RED, RED, RED, RED]));
}

/// The port's own: a size that is not positive — where the C# threw — gives no frames, and a cancel
/// is a cancel, not a failure.
#[test]
fn no_size_gives_no_frames_and_a_cancel_is_a_cancel() {
    let layout = AnimationLayout::new(
        AnimationFormat::Apng,
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
    assert_eq!(compose(&layout, vec![Some(RED.to_vec())], 0, 1), None);
    let cancel = Cancel::new();
    cancel.cancel();
    assert_eq!(
        LocalPool::new().run_until(sticker_frames::compose(
            &layout,
            &from(vec![Some(RED.to_vec())]),
            1,
            1,
            &cancel
        )),
        Err(OperationCanceled)
    );
}
