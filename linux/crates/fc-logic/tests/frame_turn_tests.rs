//! `FrameTurnTests`: a frame of a transcode beside the same frame of its source — the check for a
//! result whose NUMBERS are all as asked and whose picture is on its side. The frames here are
//! drawn by this test: a scene with a bright sky over dark ground and one lit window off-centre,
//! which looks like itself in exactly one of its four turns.

use fc_logic::frame_turn::{self, FrameVerdict, SIDE};

/// Sky above, ground below, a window up and to the left — brightness, row by row.
fn scene() -> Vec<f64> {
    let mut grid = vec![0.0; SIDE * SIDE];
    for row in 0..SIDE {
        for column in 0..SIDE {
            let sky = if row < SIDE / 2 {
                190 - (row as i32 * 4)
            } else {
                60 + column as i32
            };
            let window = if (2..5).contains(&row) && (3..6).contains(&column) {
                60
            } else {
                0
            };
            grid[(row * SIDE) + column] = f64::from(sky + window);
        }
    }
    grid
}

/// What an encoder does to a frame: every cell a little off, by a pattern that is no picture.
fn encoded(grid: &[f64]) -> Vec<f64> {
    grid.iter()
        .enumerate()
        .map(|(at, cell)| cell + f64::from(((at as i32 * 7919) % 11) - 5))
        .collect()
}

fn pixels(grid: &[f64]) -> Vec<u8> {
    let mut bgra = vec![0u8; 4 * grid.len()];
    for (cell, value) in grid.iter().enumerate() {
        // `Math.Round` is banker's rounding.
        let value = value.round_ties_even().clamp(0.0, 255.0) as u8;
        bgra[4 * cell..4 * cell + 4].copy_from_slice(&[value, value, value, 255]);
    }
    bgra
}

/// xunit's `Assert.Equal(expected, actual, precision)`: equal once both are rounded (to even) to
/// that many decimal places.
fn assert_equal_places(expected: f64, actual: f64, places: i32) {
    let scale = 10f64.powi(places);
    assert_eq!(
        (expected * scale).round_ties_even() / scale,
        (actual * scale).round_ties_even() / scale,
        "{expected} vs {actual}"
    );
}

#[test]
fn a_grid_is_each_cells_brightness() {
    let grey = frame_turn::grid(&pixels(&scene())).expect("a grid");
    assert_eq!(grey.len(), SIDE * SIDE);
    assert_equal_places(scene()[0], grey[0], 3);
    assert_equal_places(scene()[SIDE * SIDE - 1], grey[SIDE * SIDE - 1], 3);

    // Green weighs most and blue least, as an eye has it.
    let mut colours = vec![0u8; 4 * SIDE * SIDE];
    (colours[0], colours[5], colours[10]) = (255, 255, 255);
    let weighed = frame_turn::grid(&colours).expect("a grid");
    assert!(weighed[1] > weighed[2] && weighed[2] > weighed[0] && weighed[0] > 0.0);

    assert!(frame_turn::grid(&[0u8; 4 * SIDE]).is_none());
    assert!(frame_turn::grid(&[]).is_none());
}

#[test]
fn four_quarter_turns_are_no_turn_and_one_moves_the_top_left_to_the_top_right() {
    let scene = scene();
    assert_eq!(frame_turn::turn(&scene, 0), scene);
    assert_eq!(frame_turn::turn(&scene, 4), scene);
    assert_eq!(frame_turn::turn(&frame_turn::turn(&scene, 1), 3), scene);
    assert_eq!(
        frame_turn::turn(&scene, 2),
        frame_turn::turn(&frame_turn::turn(&scene, 1), 1)
    );
    assert_eq!(frame_turn::turn(&scene, 3), frame_turn::turn(&scene, -1));

    let mut corner = vec![0.0; SIDE * SIDE];
    corner[0] = 1.0;
    assert_eq!(frame_turn::turn(&corner, 1)[SIDE - 1], 1.0);
    assert_eq!(frame_turn::turn(&corner, 2)[(SIDE * SIDE) - 1], 1.0);
    assert_eq!(frame_turn::turn(&corner, 3)[(SIDE - 1) * SIDE], 1.0);
}

#[test]
fn a_re_encode_of_the_same_frame_is_the_same_picture() {
    assert_eq!(frame_turn::judge(&scene(), &scene()), FrameVerdict::Same);
    assert_eq!(
        frame_turn::judge(&scene(), &encoded(&scene())),
        FrameVerdict::Same
    );
}

/// An HDR source beside its tone-mapped result: darker all over and flatter, and still the same
/// picture.
#[test]
fn a_darker_flatter_copy_is_the_same_picture() {
    let tone_mapped: Vec<f64> = scene().iter().map(|cell| 20.0 + (0.6 * cell)).collect();
    assert_eq!(
        frame_turn::judge(&scene(), &encoded(&tone_mapped)),
        FrameVerdict::Same
    );
}

/// THE case: a portrait clip whose pixels the transcoder turned as well as carrying the turn. Every
/// number read back is what was asked for; the picture is on its side — or, where two conventions
/// disagree, upside down.
#[test]
fn a_result_on_its_side_or_upside_down_is_turned() {
    for quarters in [1, 2, 3] {
        assert_eq!(
            frame_turn::judge(&scene(), &encoded(&frame_turn::turn(&scene(), quarters))),
            FrameVerdict::Turned,
            "{quarters}"
        );
    }
}

/// The other way a double turn comes out: the upright frame fitted INTO the landscape one with
/// bars either side, and then turned. It is not the source's picture in any of its four turns.
#[test]
fn a_result_squashed_between_bars_is_unlike_its_source() {
    let scene = scene();
    let turned = frame_turn::turn(&scene, 1);
    let mut barred = vec![0.0; SIDE * SIDE];
    for row in 0..SIDE {
        for column in 0..SIDE {
            // The middle five rows hold the whole turned picture, squeezed; the rest is black.
            let inside = (5..10).contains(&row);
            barred[(row * SIDE) + column] = if inside {
                turned[((row - 5) * 3 * SIDE) + column]
            } else {
                0.0
            };
        }
    }
    assert_eq!(frame_turn::judge(&scene, &barred), FrameVerdict::Unlike);
}

/// A horizon, a corridor: the same upside down. No turned reading is CLEARLY better, so it is the
/// same picture.
#[test]
fn a_picture_that_looks_the_same_turned_is_not_called_turned() {
    let mut rings = vec![0.0; SIDE * SIDE];
    for row in 0..SIDE {
        for column in 0..SIDE {
            let from_centre = (row as f64 - 7.5).abs().max((column as f64 - 7.5).abs());
            rings[(row * SIDE) + column] = 40.0 + (from_centre * 25.0);
        }
    }
    assert_eq!(
        frame_turn::judge(&rings, &encoded(&frame_turn::turn(&rings, 1))),
        FrameVerdict::Same
    );
    assert_eq!(
        frame_turn::judge(&rings, &encoded(&frame_turn::turn(&rings, 2))),
        FrameVerdict::Same
    );
}

/// A clip that opens on black: there is nothing to compare, turned or not.
#[test]
fn a_black_or_flat_frame_says_nothing() {
    let black = vec![0.0; SIDE * SIDE];
    let dim: Vec<f64> = (0..SIDE * SIDE).map(|at| 12.0 + (at % 5) as f64).collect();
    assert_eq!(frame_turn::judge(&black, &black), FrameVerdict::CannotTell);
    assert_eq!(frame_turn::judge(&dim, &scene()), FrameVerdict::CannotTell);
    assert_eq!(
        frame_turn::judge(&scene(), &black),
        FrameVerdict::CannotTell
    );
    assert_eq!(
        frame_turn::judge(&scene(), &[0.0; 4]),
        FrameVerdict::CannotTell
    );
}

#[test]
fn a_different_picture_is_unlike() {
    let stripes: Vec<f64> = (0..SIDE * SIDE)
        .map(|at| if at % 2 == 0 { 200.0 } else { 30.0 })
        .collect();
    assert_eq!(frame_turn::judge(&scene(), &stripes), FrameVerdict::Unlike);
}

/// What a clip's frames add up to: a turned one fails it at once, a same one passes it, an unlike
/// one fails it unless another moment is the same, and frames that say nothing let it go — a clip
/// of a dark room must stay sendable.
#[test]
fn the_frames_of_one_clip_add_up() {
    use FrameVerdict::{CannotTell, Same, Turned, Unlike};
    assert!(frame_turn::looks_right([Same]));
    assert!(frame_turn::looks_right([CannotTell, Same]));
    // A cut exactly at the first moment asked: the next moment settles it.
    assert!(frame_turn::looks_right([Unlike, Same]));
    assert!(frame_turn::looks_right([
        CannotTell, CannotTell, CannotTell
    ]));
    assert!(frame_turn::looks_right([]));

    assert!(!frame_turn::looks_right([Turned]));
    assert!(!frame_turn::looks_right([CannotTell, Turned]));
    assert!(!frame_turn::looks_right([Unlike, Turned, Same]));
    assert!(!frame_turn::looks_right([Unlike, CannotTell, Unlike]));
    assert!(!frame_turn::looks_right([CannotTell, Unlike]));
}

/// Not in the C#: a grid that is not 16 × 16 comes back unturned rather than panicking (the C#
/// threw `IndexOutOfRangeException` for a short one).
#[test]
fn a_grid_of_another_size_is_not_turned() {
    assert_eq!(
        frame_turn::turn(&[1.0, 2.0, 3.0, 4.0], 1),
        [1.0, 2.0, 3.0, 4.0]
    );
}
