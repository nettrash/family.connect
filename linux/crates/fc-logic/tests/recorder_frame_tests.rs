//! `RecorderFrameTests`: THE RECORDER'S PARTS NEVER OVERLAP (decision 41, 2026-10-06). On the
//! owner's iPhone the composer showed through the video recorder and its controls sat on the
//! composer's buttons, the status floated over messages and the circle competed with the chat. The
//! bounds are proved here, from what the shell's recorder layer would measure: the status capsule,
//! the circle with its ring, the reply banner, the solid bar and the controls on it — the captions
//! under the round buttons are inside the controls' measured height — never intersect, at
//! phone-narrow widths (320, 375, 430), in a short "landscape" pane, in a desktop pane, and at 100 %
//! to 225 % text.

use fc_logic::recorder_frame::{self, PaneBox, RecorderFrame, RecorderMeasures};
use fc_logic::round_look::recorder_look;
use fc_logic::round_recorder::RecorderStage;
use fc_logic::round_video_rules::{self, RecorderFit, RecorderLayout, fit};

/// What the layer measures at a text scale: 11-pixel captions, the 14-pixel status line, the round
/// buttons.
#[derive(Debug, Clone, Copy)]
struct Parts {
    scale: f64,
    below_lines: i32,
    replying: bool,
    round_buttons: i32,
}

impl Parts {
    fn new(scale: f64, below_lines: i32, replying: bool, round_buttons: i32) -> Self {
        Parts {
            scale,
            below_lines,
            replying,
            round_buttons,
        }
    }

    fn caption(&self) -> f64 {
        (15.0 * self.scale).ceil()
    }

    /// A captioned button's column: 56 at least, or as wide as its caption — "Camera" is about 36
    /// at 11 pixels.
    fn captioned_column(&self) -> f64 {
        56f64.max((36.0 * self.scale).ceil())
    }

    fn status_height(&self) -> f64 {
        12.0 + (20.0 * self.scale).ceil()
            + f64::from(self.below_lines) * (2.0 + (16.0 * self.scale).ceil())
    }

    fn status_width(&self) -> f64 {
        344f64.min(24.0 + (150.0 * self.scale).ceil())
    }

    fn banner_height(&self) -> f64 {
        if self.replying {
            40f64.max(12.0 + (20.0 * self.scale).ceil())
        } else {
            0.0
        }
    }

    fn banner_width(&self) -> f64 {
        if self.replying { 420.0 } else { 0.0 }
    }

    fn slot_column_height(&self) -> f64 {
        recorder_look::SLOT_TARGET + recorder_look::CAPTION_GAP + self.caption()
    }

    fn side_column_height(&self) -> f64 {
        recorder_look::SIDE + recorder_look::CAPTION_GAP + self.caption()
    }

    /// One row: the 6-pixel lead, the round buttons in their captioned columns, the slot, 8 between
    /// each.
    fn single_row_width(&self) -> f64 {
        let buttons = f64::from(self.round_buttons);
        6.0 + buttons * self.captioned_column() + recorder_look::SLOT_TARGET + buttons * 8.0
    }

    fn wrapped_width(&self) -> f64 {
        let buttons = f64::from(self.round_buttons);
        6.0 + (buttons * self.captioned_column() + (buttons - 1.0) * 8.0)
            .max(recorder_look::SLOT_TARGET)
    }

    fn wrapped_height(&self) -> f64 {
        self.side_column_height() + 8.0 + self.slot_column_height()
    }

    /// The column's rows as the layer measures them — WITHOUT the reply banner, which the frame
    /// places: the leading row, the middle row, the slot, 8 between each.
    fn column_height(&self) -> f64 {
        2.0 * self.side_column_height() + 8.0 + 8.0 + self.slot_column_height()
    }

    fn column_width(&self) -> f64 {
        recorder_frame::COLUMN_CONTROLS_WIDTH.max(2.0 * self.captioned_column() + 8.0)
    }
}

const SCALES: [f64; 4] = [1.0, 1.5, 2.0, 2.25];

/// xUnit's `Assert.Equal(expected, actual, precision)`: both rounded to `places` decimals.
fn assert_near(expected: f64, actual: f64, places: i32, what: &str) {
    let scale = 10f64.powi(places);
    assert_eq!(
        (expected * scale).round_ties_even(),
        (actual * scale).round_ties_even(),
        "{what}: {expected} vs {actual}"
    );
}

fn lay(width: f64, height: f64, parts: Parts) -> (RecorderFrame, f64) {
    lay_inset(width, height, parts, 40.0)
}

fn lay_inset(width: f64, height: f64, parts: Parts, send_inset: f64) -> (RecorderFrame, f64) {
    let fit = fit(width, height, parts.banner_height());
    let send = width - send_inset;
    let (controls_width, controls_height) = if fit.layout == RecorderLayout::Column {
        (parts.column_width(), parts.column_height())
    } else if recorder_frame::row_wraps(width, parts.single_row_width(), send) {
        (parts.wrapped_width(), parts.wrapped_height())
    } else {
        (parts.single_row_width(), parts.slot_column_height())
    };
    let frame = recorder_frame::frame(
        fit,
        &RecorderMeasures::new(
            width,
            height,
            parts.status_width(),
            parts.status_height(),
            parts.banner_width(),
            parts.banner_height(),
            controls_width,
            controls_height,
        )
        .with_send_centre_x(send),
    );
    (frame, controls_width)
}

fn assert_apart(frame: &RecorderFrame, width: f64, height: f64, controls_width: f64, what: &str) {
    let pane = PaneBox::new(0.0, 0.0, width, height);
    let mut parts = vec![
        ("status", frame.status),
        ("circle", frame.circle),
        ("bar", frame.bar),
    ];
    if let Some(banner) = frame.banner {
        parts.push(("banner", banner));
    }
    for (i, (name, part)) in parts.iter().enumerate() {
        assert!(
            pane.contains(part),
            "{what}: {name} {part:?} leaves the pane"
        );
        for (other_name, other) in &parts[i + 1..] {
            assert!(
                !part.intersects(other),
                "{what}: {name} {part:?} overlaps {other_name} {other:?}"
            );
        }
    }
    // The controls — every round button and every caption under it — stand on the bar, whole, and
    // nothing else does.
    assert!(
        frame.bar.contains(&frame.controls),
        "{what}: controls {:?} off the bar {:?}",
        frame.controls,
        frame.bar
    );
    assert!(
        frame.controls.width >= controls_width - 1e-9,
        "{what}: controls cut to {} of {controls_width}",
        frame.controls.width
    );
    for (name, other) in parts.iter().filter(|part| part.0 != "bar") {
        assert!(
            !other.intersects(&frame.controls),
            "{what}: {name} {other:?} overlaps the controls {:?}",
            frame.controls
        );
    }
    // The circle box is the circle and its ring, exactly.
    assert_near(
        frame.diameter + 2.0 * recorder_frame::RING_CLEARANCE,
        frame.circle.width,
        9,
        what,
    );
    assert_near(frame.circle.width, frame.circle.height, 9, what);
}

/// Phone-narrow panes, portrait: 320, 375 and 430 wide, every text size, a reply or none, two
/// status lines. A `[Theory]`: one test over its rows.
#[test]
fn nothing_overlaps_in_a_row() {
    for (width, height) in [
        (320.0, 568.0),
        (375.0, 667.0),
        (430.0, 932.0),
        (320.0, 480.0),
        (375.0, 520.0),
        (700.0, 900.0),
        (1000.0, 1000.0),
    ] {
        for scale in SCALES {
            for replying in [false, true] {
                for below in [0, 1, 3] {
                    for buttons in [1, 3] {
                        let parts = Parts::new(scale, below, replying, buttons);
                        let (frame, controls_width) = lay(width, height, parts);
                        assert_eq!(RecorderLayout::Row, frame.layout);
                        assert_apart(
                            &frame,
                            width,
                            height,
                            controls_width,
                            &format!("{width}x{height} at {scale}, {parts:?}"),
                        );
                    }
                }
            }
        }
    }
}

/// A short pane — a phone or a small window turned "landscape" — stands the bar at the trailing
/// edge. A `[Theory]`: one test over its rows.
#[test]
fn nothing_overlaps_in_a_column() {
    for (width, height) in [
        (568.0, 320.0),
        (667.0, 375.0),
        (932.0, 430.0),
        (1000.0, 479.0),
        (480.0, 400.0),
        (420.0, 360.0),
    ] {
        // Every text size, the largest in the shortest pane too: until 2026-10-06 the column kept
        // the banner in the controls whatever it cost, and at 568 × 320 and 200 % with a reply the
        // slot ended 24 below the pane, out of reach.
        for scale in SCALES {
            for replying in [false, true] {
                for below in [0, 1, 2] {
                    let parts = Parts::new(scale, below, replying, 3);
                    let (frame, controls_width) = lay(width, height, parts);
                    let what = format!("{width}x{height} at {scale}, {parts:?}");
                    assert_eq!(RecorderLayout::Column, frame.layout);
                    assert_apart(&frame, width, height, controls_width, &what);
                    // The banner rides at the column's top where the bar has the height for it, and
                    // stands under the circle where it has not; without a reply there is none.
                    let rides = replying
                        && recorder_frame::banner_rides_in_column(
                            height,
                            parts.column_height(),
                            parts.banner_height(),
                        );
                    assert_eq!(
                        frame.banner.is_none(),
                        !replying || rides,
                        "{what}: banner {:?}",
                        frame.banner
                    );
                    let expected = parts.column_height()
                        + if rides {
                            8.0 + parts.banner_height()
                        } else {
                            0.0
                        };
                    assert_near(expected, frame.controls.height, 9, &what);
                    // The controls — the slot last — whole inside the bar's padding, so the slot is
                    // always in reach.
                    assert!(
                        frame.controls.y >= recorder_frame::BAR_PADDING - 1e-9
                            && frame.controls.bottom()
                                <= height - recorder_frame::BAR_PADDING + 1e-9,
                        "{what}: controls {:?}",
                        frame.controls
                    );
                    // The status, the circle and (when it is there) the banner are centred top to
                    // bottom beside the bar, a gutter clear of both edges.
                    assert!(
                        frame.status.y >= recorder_frame::GUTTER - 1e-9,
                        "{what}: status at {}",
                        frame.status.y
                    );
                    let last = frame.banner.map_or(frame.circle.bottom(), |b| b.bottom());
                    assert_near(frame.status.y, height - last, 6, &what);
                    assert!(
                        frame
                            .banner
                            .is_none_or(|under| under.right()
                                <= frame.bar.x - recorder_frame::GUTTER + 1e-9),
                        "{what}: banner reaches the bar"
                    );
                }
            }
        }
    }
}

/// Where there is room the circle is the plan's (`fit`'s) size; where there is not it shrinks,
/// never overlaps.
#[test]
fn the_circle_keeps_its_size_where_there_is_room_and_shrinks_where_there_is_not() {
    let roomy = lay(1000.0, 1000.0, Parts::new(1.0, 0, false, 3)).0;
    assert_eq!(round_video_rules::LARGEST_CIRCLE, roomy.diameter);
    // A 320 × 480 pane at 200 % with a reply and a wrapped row: `fit` asks for more than there is
    // room for — the old layout drew it anyway, over the status — and the frame draws less, still a
    // circle.
    let parts = Parts::new(2.0, 2, true, 3);
    let tight = lay(320.0, 480.0, parts).0;
    let asked = fit(320.0, 480.0, parts.banner_height()).diameter;
    assert!(tight.diameter < asked, "{} of {asked}", tight.diameter);
    assert!(tight.diameter > 0.0);
}

/// The old Row layout's arithmetic (before 2026-10-06), kept to show what the frame fixed: the
/// stack — status, circle at `fit`'s diameter, banner, 12 apart — grew up from 16 over the slot's
/// column on the composer's own row, sized without the status's height, the text size or a bar. At
/// 320 × 480 and 200 % with a reply and a two-line notice its status line was pushed off the top of
/// the pane; the frame keeps every part inside and apart.
#[test]
fn the_old_layout_overflowed_where_the_frame_does_not() {
    let (width, height) = (320.0, 480.0);
    let parts = Parts::new(2.0, 2, true, 3);
    let old_diameter = fit(width, height, parts.banner_height()).diameter;
    let old_stack_bottom = height - (parts.slot_column_height() + 16.0);
    let old_circle_top = old_stack_bottom
        - parts.banner_height()
        - 12.0
        - (old_diameter + 2.0 * recorder_frame::RING_CLEARANCE);
    let old_status_top = old_circle_top - 12.0 - parts.status_height();
    assert!(
        old_status_top < 0.0,
        "the old status line began at {old_status_top}"
    );
    let (frame, controls_width) = lay(width, height, parts);
    assert_apart(&frame, width, height, controls_width, "320x480 at 200 %");
    assert!(frame.status.y >= recorder_frame::GUTTER);
    assert!(
        frame.diameter < old_diameter,
        "{} of {old_diameter}",
        frame.diameter
    );
}

/// Where the room runs out — 320 × 480 at 225 %, a three-line notice and a reply — the camera keeps
/// a circle of at least `SHORTEST_CIRCLE` and the status capsule is cut to what is left, still apart
/// from it.
#[test]
fn where_the_room_runs_out_the_circle_keeps_its_shortest_and_the_status_is_cut() {
    let parts = Parts::new(2.25, 3, true, 3);
    let (frame, controls_width) = lay(320.0, 480.0, parts);
    assert_apart(&frame, 320.0, 480.0, controls_width, "320x480 at 225 %");
    assert_eq!(recorder_frame::SHORTEST_CIRCLE, frame.diameter);
    assert!(
        frame.status.height < parts.status_height(),
        "{}",
        frame.status.height
    );
    assert!(frame.status.height > 0.0);
    // With a one-line status and no reply the same pane draws the whole status and a larger circle.
    let roomier = lay(320.0, 480.0, Parts::new(2.25, 0, false, 3)).0;
    assert_eq!(
        Parts::new(2.25, 0, false, 3).status_height(),
        roomier.status.height
    );
    assert!(
        roomier.diameter > recorder_frame::SHORTEST_CIRCLE,
        "{}",
        roomier.diameter
    );
}

/// The slot stands under Send: the controls end where a 72-pixel slot centred on Send would.
#[test]
fn the_slot_stands_under_send() {
    let row = RecorderFit {
        diameter: 320.0,
        layout: RecorderLayout::Row,
    };
    let frame = recorder_frame::frame(
        row,
        &RecorderMeasures::new(1000.0, 900.0, 200.0, 32.0, 0.0, 0.0, 270.0, 89.0)
            .with_send_centre_x(950.0),
    );
    assert_near(
        950.0 + recorder_look::SLOT_TARGET / 2.0,
        frame.controls.right(),
        9,
        "right",
    );
    assert_near(recorder_frame::GUTTER, frame.controls.x, 9, "x");
    // Not laid out yet: the gutter on both sides.
    let unknown = recorder_frame::frame(
        row,
        &RecorderMeasures::new(1000.0, 900.0, 200.0, 32.0, 0.0, 0.0, 270.0, 89.0),
    );
    assert_near(
        1000.0 - recorder_frame::GUTTER,
        unknown.controls.right(),
        9,
        "unknown",
    );
}

/// The bar is solid along the whole bottom of the pane, as tall as the controls and their padding.
#[test]
fn the_bar_runs_the_width_of_the_pane_at_its_bottom() {
    let frame = recorder_frame::frame(
        RecorderFit {
            diameter: 320.0,
            layout: RecorderLayout::Row,
        },
        &RecorderMeasures::new(800.0, 900.0, 200.0, 32.0, 0.0, 0.0, 270.0, 89.0)
            .with_send_centre_x(760.0),
    );
    assert_eq!(
        PaneBox::new(
            0.0,
            900.0 - 89.0 - 2.0 * recorder_frame::BAR_PADDING,
            800.0,
            89.0 + 2.0 * recorder_frame::BAR_PADDING
        ),
        frame.bar
    );
    // The controls stand inside its padding, top and bottom.
    assert_near(
        frame.bar.y + recorder_frame::BAR_PADDING,
        frame.controls.y,
        9,
        "top",
    );
    assert_near(
        frame.bar.bottom() - recorder_frame::BAR_PADDING,
        frame.controls.bottom(),
        9,
        "bottom",
    );
    // And the circle and status stand just over it, the circle centred.
    assert_near(
        frame.bar.y - recorder_frame::GUTTER,
        frame.circle.bottom(),
        9,
        "circle",
    );
    assert_near(
        400.0,
        frame.circle.x + frame.circle.width / 2.0,
        9,
        "centre",
    );
    assert_near(
        frame.circle.y - recorder_frame::GAP,
        frame.status.bottom(),
        9,
        "status",
    );
}

/// A status that grows a line mid-take grows upward: the circle stays where it was.
#[test]
fn a_growing_status_never_moves_the_circle() {
    let fit = RecorderFit {
        diameter: 320.0,
        layout: RecorderLayout::Row,
    };
    let one = recorder_frame::frame(
        fit,
        &RecorderMeasures::new(800.0, 900.0, 200.0, 32.0, 0.0, 0.0, 270.0, 89.0)
            .with_send_centre_x(760.0),
    );
    let two = recorder_frame::frame(
        fit,
        &RecorderMeasures::new(800.0, 900.0, 200.0, 50.0, 0.0, 0.0, 270.0, 89.0)
            .with_send_centre_x(760.0),
    );
    assert_eq!(one.circle, two.circle);
    assert!(two.status.y < one.status.y);
}

/// A single row wraps where it would not fit beside Send: the slot on its own line.
#[test]
fn the_row_wraps_when_it_would_not_fit() {
    assert!(!recorder_frame::row_wraps(800.0, 270.0, 760.0));
    assert!(recorder_frame::row_wraps(320.0, 342.0, 280.0));
    // Room is the pane less the gutter on the left and, on the right, what puts the slot's centre
    // on Send.
    let room = 375.0 - recorder_frame::GUTTER - (375.0 - 300.0 - recorder_look::SLOT_TARGET / 2.0);
    assert!(!recorder_frame::row_wraps(375.0, room, 300.0));
    assert!(recorder_frame::row_wraps(375.0, room + 1.0, 300.0));
    // Send hard against the edge: the bar's padding is kept on the right all the same.
    assert!(!recorder_frame::row_wraps(
        375.0,
        375.0 - recorder_frame::GUTTER - recorder_frame::BAR_PADDING,
        370.0
    ));
    assert!(recorder_frame::row_wraps(
        375.0,
        375.0 - recorder_frame::GUTTER - recorder_frame::BAR_PADDING + 1.0,
        370.0
    ));
}

/// A short "landscape" pane: at 100 % the reply banner rides at the top of the column; at 200 % the
/// column would run off the pane with it, so it stands under the circle instead and the slot stays
/// whole on the bar.
#[test]
fn a_short_pane_gives_up_the_banners_place_in_the_column_not_the_slot() {
    let small = Parts::new(1.0, 0, true, 3);
    let (riding, _) = lay(568.0, 320.0, small);
    assert_eq!(None, riding.banner);
    assert_near(
        small.column_height() + recorder_frame::CONTROLS_LINE_GAP + small.banner_height(),
        riding.controls.height,
        9,
        "riding",
    );
    let large = Parts::new(2.0, 0, true, 3);
    let (under, controls_width) = lay(568.0, 320.0, large);
    assert_apart(&under, 568.0, 320.0, controls_width, "568x320 at 200 %");
    let banner = under.banner.expect("the banner stands under the circle");
    assert_near(
        under.circle.bottom() + recorder_frame::GAP,
        banner.y,
        9,
        "banner",
    );
    assert_near(large.column_height(), under.controls.height, 9, "controls");
    assert!(
        under.controls.bottom() <= 320.0 - recorder_frame::BAR_PADDING,
        "{:?}",
        under.controls
    );
    assert!(recorder_frame::banner_rides_in_column(320.0, 200.0, 40.0));
    assert!(recorder_frame::banner_rides_in_column(
        320.0,
        320.0 - 24.0 - 8.0 - 40.0,
        40.0
    ));
    assert!(!recorder_frame::banner_rides_in_column(
        320.0,
        320.0 - 24.0 - 8.0 - 40.0 + 1.0,
        40.0
    ));
}

/// The status is measured across the span it is drawn across — the room beside a column's bar, not
/// the whole pane, where a notice would wrap fewer times than it is drawn and lose its last line to
/// the capsule's clip — and its line of text wraps inside the capsule.
#[test]
fn the_status_is_measured_across_the_span_it_is_drawn_in() {
    for (width, height) in [
        (568.0, 320.0),
        (420.0, 360.0),
        (1000.0, 479.0),
        (375.0, 667.0),
        (1000.0, 1000.0),
    ] {
        let parts = Parts::new(1.5, 1, false, 3);
        let (frame, controls_width) = lay(width, height, parts);
        let span = recorder_frame::status_span(frame.layout, width, controls_width);
        let expected = if frame.layout == RecorderLayout::Column {
            frame.bar.x
        } else {
            width
        };
        assert_near(expected, span, 9, "span");
        // The capsule drawn never wider than the span less its gutters, and centred in it.
        assert!(frame.status.width <= span - 2.0 * recorder_frame::GUTTER + 1e-9);
        assert_near(
            span / 2.0,
            frame.status.x + frame.status.width / 2.0,
            9,
            "centred",
        );
    }
    assert_near(
        400.0 - 32.0 - 28.0,
        recorder_frame::status_line_width(400.0, false),
        9,
        "no dot",
    );
    assert_near(
        400.0 - 32.0 - 28.0 - 16.0,
        recorder_frame::status_line_width(400.0, true),
        9,
        "dot",
    );
    assert_eq!(0.0, recorder_frame::status_line_width(40.0, true));
}

/// A new line lays the recorder out again — "Starting camera…" turning into a longer sentence would
/// otherwise be cut to the old capsule — but the clock ticking while recording does not: every
/// second would re-measure the whole layer.
#[test]
fn a_new_status_line_lays_out_and_the_clock_does_not() {
    use recorder_frame::line_that_lays_out;
    assert_eq!(
        line_that_lays_out(RecorderStage::Recording, "0:01"),
        line_that_lays_out(RecorderStage::Recording, "0:42")
    );
    assert_ne!(
        line_that_lays_out(RecorderStage::Preview, "Starting camera…"),
        line_that_lays_out(RecorderStage::Preview, "Not recording")
    );
    assert_ne!(
        line_that_lays_out(RecorderStage::Review, "Video message · 0:09"),
        line_that_lays_out(RecorderStage::Review, "Video message · 0:10")
    );
    assert_eq!(
        Some("Starting camera…"),
        line_that_lays_out(RecorderStage::Opening, "Starting camera…")
    );
}

#[test]
fn boxes_touching_at_an_edge_do_not_overlap() {
    let a = PaneBox::new(0.0, 0.0, 10.0, 10.0);
    assert!(!a.intersects(&PaneBox::new(10.0, 0.0, 10.0, 10.0)));
    assert!(!a.intersects(&PaneBox::new(0.0, 10.0, 10.0, 10.0)));
    assert!(!PaneBox::new(10.0, 0.0, 10.0, 10.0).intersects(&a));
    assert!(!PaneBox::new(0.0, 10.0, 10.0, 10.0).intersects(&a));
    assert!(a.intersects(&PaneBox::new(9.5, 9.5, 10.0, 10.0)));
    assert!(!a.intersects(&PaneBox::new(5.0, 5.0, 0.0, 10.0)));
    assert!(a.contains(&PaneBox::new(0.0, 0.0, 10.0, 10.0)));
    assert!(!a.contains(&PaneBox::new(0.0, 0.0, 10.5, 10.0)));
}

/// A pane not yet laid out draws nothing at all rather than something negative.
#[test]
fn a_pane_not_yet_laid_out_is_harmless() {
    let frame = recorder_frame::frame(
        fit(0.0, 0.0, 0.0),
        &RecorderMeasures::new(0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0),
    );
    assert_eq!(0.0, frame.diameter);
    assert!(frame.bar.width >= 0.0 && frame.bar.height >= 0.0);
    let nan = recorder_frame::frame(
        RecorderFit {
            diameter: f64::NAN,
            layout: RecorderLayout::Row,
        },
        &RecorderMeasures::new(f64::NAN, 900.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0),
    );
    assert_eq!(0.0, nan.diameter);
}
