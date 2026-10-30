//! Whether a transcode still LOOKS like what it came from (`FrameTurn.cs`) — the half of the
//! read-back that the container's numbers cannot give (`MediaEncoding::matches` is the other half).
//!
//! THE NUMBERS CAN ALL BE RIGHT AND THE PICTURE WRONG. A portrait phone clip is asked for in its
//! STORED sides with its turn as metadata. If the transcoder also turns the PIXELS — or the turn
//! that was read and the turn that was written mean opposite directions — the result has exactly
//! the sides and the turn that were asked for, and plays on its side, squashed. So the result is
//! looked at: one frame of the source and the same frame of the result, read by the SAME reader
//! and compared. Whatever that reader does with a turn it does to both, so the comparison holds
//! whichever convention it has.
//!
//! A FRAME IS SQUASHED TO A SQUARE GRID OF BRIGHTNESS, [`SIDE`] each way, because a square can be
//! turned and compared cell for cell; and what is compared is how the cells VARY together (a
//! correlation), not their values, so an HDR source beside its tone-mapped result — darker or
//! lighter all over — is still the same picture.
//!
//! ONLY A FRAME WITH SOMETHING IN IT CAN SAY ANYTHING. A clip that opens on black correlates with
//! nothing, turned or not; that is [`FrameVerdict::CannotTell`], and another moment of the clip is
//! asked.

/// What a frame of a transcode looks like beside the same frame of its source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameVerdict {
    /// The same picture, the same way up.
    Same,
    /// The source's picture on its side or upside down.
    Turned,
    /// A picture, but not the source's — squashed into bars, or something else altogether.
    Unlike,
    /// Nothing to go by: a black frame, a fade, a flat wall.
    CannotTell,
}

/// The grid's side: fine enough to tell a picture from its quarter turn, coarse enough to ignore
/// the encoder.
pub const SIDE: usize = 16;

/// Brightness that varies by less than this (of 255) across a frame is no picture to compare.
const FLAT: f64 = 8.0;

/// How alike two grids must be to be the same picture — a re-encode of the same frame is far
/// above it.
const ALIKE: f64 = 0.7;

/// How much more alike than the upright reading a turned one must be before the result is called
/// turned.
const CLEARLY: f64 = 0.2;

/// The grid of a frame already scaled to [`SIDE`] × [`SIDE`] BGRA pixels: each cell's brightness
/// (BT.601's weights — which ones matters little, since only how cells differ is compared).
/// `None` when it is not that many pixels.
pub fn grid(bgra: &[u8]) -> Option<Vec<f64>> {
    if bgra.len() != 4 * SIDE * SIDE {
        return None;
    }
    Some(
        bgra.as_chunks::<4>()
            .0
            .iter()
            .map(|pixel| {
                (0.114 * f64::from(pixel[0]))
                    + (0.587 * f64::from(pixel[1]))
                    + (0.299 * f64::from(pixel[2]))
            })
            .collect(),
    )
}

/// `result` beside `source`. Turned only when a turned reading is CLEARLY the better one — a
/// picture that looks the same upside down (a horizon, a corridor) is the same picture.
pub fn judge(source: &[f64], result: &[f64]) -> FrameVerdict {
    if source.len() != SIDE * SIDE
        || result.len() != SIDE * SIDE
        || is_flat(source)
        || is_flat(result)
    {
        return FrameVerdict::CannotTell;
    }
    let upright = likeness(source, result);
    let turned = [1, 2, 3]
        .into_iter()
        .map(|quarters| likeness(&turn(source, quarters), result))
        .fold(f64::NEG_INFINITY, f64::max);
    if turned >= ALIKE && turned > upright + CLEARLY {
        FrameVerdict::Turned
    } else if upright >= ALIKE {
        FrameVerdict::Same
    } else {
        FrameVerdict::Unlike
    }
}

/// What the frames of one transcode add up to, asked in order (`MediaPrep::POSTER_SEEK_SECONDS`):
/// `true` when it may go. One turned frame fails it and one that is the same passes it; when no
/// frame is the same, one unlike its source fails it too. Frames that say nothing at all — every
/// one black, or none readable — let it go: the numbers were right, and a clip of a dark room must
/// not be unsendable.
pub fn looks_right(frames: impl IntoIterator<Item = FrameVerdict>) -> bool {
    let mut unlike = false;
    for frame in frames {
        match frame {
            FrameVerdict::Turned => return false,
            FrameVerdict::Same => return true,
            FrameVerdict::Unlike => unlike = true,
            FrameVerdict::CannotTell => {}
        }
    }
    !unlike
}

/// The grid turned clockwise by `quarters` quarter turns (negative turns anticlockwise). A grid
/// that is not [`SIDE`] × [`SIDE`] comes back as it is (the C# threw for a short one).
pub fn turn(grid: &[f64], quarters: i32) -> Vec<f64> {
    let mut turned = grid.to_vec();
    if grid.len() != SIDE * SIDE {
        return turned;
    }
    for _ in 0..quarters.rem_euclid(4) {
        let mut next = vec![0.0; SIDE * SIDE];
        for row in 0..SIDE {
            for column in 0..SIDE {
                // Clockwise: the left column, read bottom to top, becomes the top row.
                next[(row * SIDE) + column] = turned[((SIDE - 1 - column) * SIDE) + row];
            }
        }
        turned = next;
    }
    turned
}

fn mean(grid: &[f64]) -> f64 {
    grid.iter().sum::<f64>() / grid.len() as f64
}

fn is_flat(grid: &[f64]) -> bool {
    let mean = mean(grid);
    let spread = grid
        .iter()
        .map(|cell| (cell - mean) * (cell - mean))
        .sum::<f64>();
    (spread / grid.len() as f64).sqrt() < FLAT
}

/// Pearson's correlation of two grids that are neither of them flat: 1 for the same picture, about
/// 0 for unrelated ones.
fn likeness(one: &[f64], other: &[f64]) -> f64 {
    let one_mean = mean(one);
    let other_mean = mean(other);
    let (mut together, mut one_spread, mut other_spread) = (0.0, 0.0, 0.0);
    for (a, b) in one.iter().zip(other) {
        let a = a - one_mean;
        let b = b - other_mean;
        together += a * b;
        one_spread += a * a;
        other_spread += b * b;
    }
    together / (one_spread * other_spread).sqrt()
}
