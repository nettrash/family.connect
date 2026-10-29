//! The board's shared arithmetic, held to the numbers the other three clients use: a wall that
//! measured its own way would not be the same wall (docs/protocol.md, "Board"). A port of
//! `win/tests/FamilyConnect.Core.Tests/Board/BoardRulesTests.cs`, against `fc_text::board` where
//! the rule lives there.
//!
//! Three of its tests are not ported: `fc-text` already makes every one of their assertions in a
//! test of its own.
//!
//! - `TheWallIsTallerThanTheWindowAndNeverShorter` —
//!   `fc_text::board::tests::the_wall_is_taller_than_the_window_and_never_shorter` (the same
//!   assertions; `wall_height(500.0) == 800.0` pins the 1.6, and `board_oracle`'s
//!   `the_constants_are_the_originals` holds `WALL_SCREENS` to the fixture).
//! - `ANoteKeepsItsTiltForEverybodyAndAcrossLaunches` —
//!   `fc_text::board::tests::the_tilt_is_the_macs_and_never_changes` (0, 6 and 7 by value, every
//!   id in -20..40 inside -3…3 and periodic in 7, which is -7's -3).
//! - `AStickerDrawsTheFirstLinesOfAListAndSaysHowManyAreLeft` —
//!   `fc_text::board::tests::a_sticker_draws_the_first_lines_of_a_list_and_says_how_many_are_left`
//!   (identical).

mod common;

use common::{assert_equal_to_places, net_round};
use fc_core::board::{board_badge, board_picture};
use fc_text::board::{
    COMPACT_BELOW, MIN_TEXT_SCALE, Marks, Size, dragged, fitted_scale, fraction_of, is_compact,
    is_unread, later, lines_that_fit, marks_after_showing, origin,
};

#[test]
fn a_narrow_window_takes_the_phones_square_stickers() {
    assert_eq!(COMPACT_BELOW, 640.0);
    assert!(is_compact(639.0));
    assert!(!is_compact(640.0));
    // The desktop's landscape cards, and the same three steps squared on a narrow wall.
    assert_eq!(Size::Medium.frame(false), (150.0, 110.0));
    assert_eq!(Size::Small.frame(false), (120.0, 88.0));
    assert_eq!(Size::Large.frame(false), (280.0, 200.0));
    assert_eq!(Size::Medium.frame(true), (132.0, 132.0));
    // Type climbs with the step.
    assert!(Size::Small.type_px() < Size::Medium.type_px());
    assert!(Size::Medium.type_px() < Size::Large.type_px());
}

#[test]
fn a_stored_fraction_is_the_top_left_corner_held_inside_the_wall() {
    let card = (150.0, 110.0);
    let board = (900.0, 600.0);
    assert_eq!(origin((0.5, 0.25), card, board), (450.0, 150.0));
    // A stored 0.98 hugs the edge rather than hanging off it.
    assert_eq!(origin((0.98, 0.98), card, board), (750.0, 490.0));
    assert_eq!(origin((-1.0, -1.0), card, board), (0.0, 0.0));
    // A board smaller than the card pins it to the top-left.
    assert_eq!(origin((0.5, 0.5), card, (100.0, 80.0)), (0.0, 0.0));
    // In hand: the origin moved, and held inside again.
    assert_eq!(
        dragged((0.5, 0.25), (20.0, -20.0), card, board),
        (470.0, 130.0)
    );
    // Read back from where it is DRAWN, so what was dropped is what gets stored.
    let (x, y) = fraction_of((450.0, 150.0), board);
    assert_equal_to_places(0.5, x, 6);
    assert_equal_to_places(0.25, y, 6);
    assert_eq!(fraction_of((2000.0, 2000.0), board), (1.0, 1.0));
    assert_eq!(fraction_of((-5.0, -5.0), board), (0.0, 0.0));
}

/// A PHOTO IS DRAWN WHOLE (issue #71): fitted in both dimensions, never cropped — the same
/// numbers `fc_text::board::fitted_picture`, Apple's and Android's pin.
#[test]
fn a_picture_is_fitted_in_both_dimensions_and_never_cropped() {
    // The 600x1200 portrait the issue was reported with, on the medium card.
    let (width, height) = board_picture::fitted(150.0, 110.0, Some(600), Some(1200));
    assert_equal_to_places(55.0, width, 2);
    assert_equal_to_places(110.0, height, 2);
    assert_equal_to_places(0.5, width / height, 3);
    // A wide one comes back SHORT, by the same rule.
    let (wide, short) = board_picture::fitted(150.0, 110.0, Some(1600), Some(900));
    assert_equal_to_places(150.0, wide, 2);
    assert_equal_to_places(84.375, short, 3);
    // A picture the shape of its space fills it exactly, and neither side ever grows past it.
    assert_eq!(
        board_picture::fitted(150.0, 110.0, Some(300), Some(220)),
        (150.0, 110.0)
    );
    let (small, smaller) = board_picture::fitted(150.0, 110.0, Some(15), Some(11));
    assert!(small <= 150.0 && smaller <= 110.0);
    // Dimensions the server never recorded take the whole space: a margin at worst.
    assert_eq!(
        board_picture::fitted(150.0, 110.0, None, None),
        (150.0, 110.0)
    );
    assert_eq!(
        board_picture::fitted(150.0, 110.0, Some(600), None),
        (150.0, 110.0)
    );
    assert_eq!(
        board_picture::fitted(150.0, 110.0, Some(0), Some(1200)),
        (150.0, 110.0)
    );
    assert_eq!(
        board_picture::fitted(150.0, 110.0, Some(-4), Some(8)),
        (150.0, 110.0)
    );
    // And a panorama still leaves something to click.
    let (_, hairline) = board_picture::fitted(150.0, 110.0, Some(20_000), Some(10));
    assert!(hairline >= 1.0);
    // FITTED, never filled: the one word #71 was, named so a test can hold it. The C# holds
    // XAML's "Uniform"; this port's word is GTK's (and CSS's) "contain".
    assert_eq!(board_picture::CONTENT_FIT, "contain");
}

#[test]
fn the_text_fits_the_note_down_to_the_floor_and_is_cut_below_it() {
    assert_eq!(MIN_TEXT_SCALE, 0.6);
    // Fits at full size: nothing to search for.
    assert_eq!(fitted_scale(|_| true, 0.6, 7), Some(1.0));
    // Never fits: it has to be cut instead.
    assert_eq!(fitted_scale(|_| false, 0.6, 7), None);
    // Fits only below 0.8: the search lands just under it and never above.
    let scale = fitted_scale(|candidate| candidate <= 0.8, 0.6, 7);
    let scale = scale.expect("a scale that fits");
    assert!(scale <= 0.8);
    assert!(scale > 0.79);
    // Lines are at least one, whatever the arithmetic says.
    assert_eq!(lines_that_fit(80.0, 20.0), 4);
    assert_eq!(lines_that_fit(10.0, 20.0), 1);
    assert_eq!(lines_that_fit(80.0, 0.0), 1);
    assert_eq!(lines_that_fit(f64::NAN, 20.0), 1);
}

#[test]
fn the_badge_counts_what_a_note_says_and_never_a_move() {
    let marks = Marks {
        note_id: 10,
        content_seq: 100,
    };
    // Judged by its content seq when it has one…
    assert!(is_unread(3, Some(101), marks));
    assert!(!is_unread(99, Some(100), marks));
    // …and by its id when it does not (an older server).
    assert!(is_unread(11, None, marks));
    assert!(!is_unread(10, None, marks));
    // A zero seq is "no data", not "seq 0".
    assert!(is_unread(11, Some(0), marks));
    // Showing the board marks everything on it, monotonically in both fields: a board that has
    // just lost its newest note must not bring a cleared badge back.
    let shown = marks_after_showing([(12, Some(130)), (4, None)], marks);
    assert_eq!(
        shown,
        Marks {
            note_id: 12,
            content_seq: 130
        }
    );
    assert_eq!(marks_after_showing([(2, Some(5))], shown), shown);
    // Two windows of one account: neither walks the other's marks back.
    assert_eq!(
        later(shown, marks),
        Marks {
            note_id: 12,
            content_seq: 130
        }
    );
    assert_eq!(
        board_badge::count([(11, None), (3, Some(101)), (2, Some(7))], marks),
        2
    );
}

/// Not a port: the helper the ported assertions use in place of xUnit's
/// `Assert.Equal(expected, actual, precision)` rounds both sides half to EVEN, as .NET does — held
/// here so it cannot quietly turn into Rust's half-away rounding or into a tolerance.
#[test]
fn xunit_precision_rounds_both_sides_half_to_even_as_dotnet_does() {
    assert_eq!(net_round(0.125, 2), 0.12);
    assert_eq!(net_round(0.375, 2), 0.38);
    assert_eq!(net_round(2.5, 0), 2.0);
    assert_eq!(net_round(3.5, 0), 4.0);
    assert_eq!(net_round(-2.5, 0), -2.0);
    // At 1e16 and beyond the runtime leaves the value alone.
    assert_eq!(net_round(1e16 + 2.0, 2), 1e16 + 2.0);
    assert!(net_round(f64::NAN, 3).is_nan());
    // Not a tolerance: 0.0002 apart, either side of a midpoint, is a disagreement.
    assert_ne!(net_round(0.1249, 2), net_round(0.1251, 2));
    assert_equal_to_places(55.0, 54.99999999999999, 2);
    assert_equal_to_places(f64::NAN, f64::NAN, 9);
}
