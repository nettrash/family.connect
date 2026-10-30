//! Where the video-message recorder's parts stand in the conversation pane
//! (docs/audio-video-messages-2026-10-04.md, S3.3, decision 41 of 2026-10-06) — `RecorderFrame.cs`.
//! On the owner's iPhone the composer showed through the recorder and its controls sat on top of
//! the composer's own buttons; the fix, the same on every platform: the composer is not drawn and
//! takes no hits while the recorder is open, the controls stand on their own solid bar, the status
//! sits in its own capsule above the circle, and the circle is sized so that it never overlaps the
//! status, the controls or any caption, at any size. The shell's recorder layer (shell-design
//! §10.5, §10.6) measures its parts and places them where [`frame`] says; the tests prove nothing
//! intersects. Every number is in GTK logical pixels, the pane's origin at its top-left (in a
//! right-to-left locale the shell mirrors the boxes across the pane).

use crate::round_look::recorder_look;
use crate::round_recorder::RecorderStage;
use crate::round_video_rules::{RING_WIDTH, RecorderFit, RecorderLayout};

/// A rectangle in the conversation pane's own logical pixels, its origin at the pane's top-left.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaneBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl PaneBox {
    const EPSILON: f64 = 1e-9;

    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        PaneBox {
            x,
            y,
            width,
            height,
        }
    }

    pub fn right(&self) -> f64 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.height
    }

    /// Whether the two share any area — touching edges do not.
    pub fn intersects(&self, other: &PaneBox) -> bool {
        self.width > 0.0
            && self.height > 0.0
            && other.width > 0.0
            && other.height > 0.0
            && self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }

    /// Whether `inner` lies wholly inside this one.
    pub fn contains(&self, inner: &PaneBox) -> bool {
        inner.x >= self.x - Self::EPSILON
            && inner.y >= self.y - Self::EPSILON
            && inner.right() <= self.right() + Self::EPSILON
            && inner.bottom() <= self.bottom() + Self::EPSILON
    }
}

/// What the recorder measured of its parts, in logical pixels at the reader's text size: the pane,
/// the status capsule, the reply banner (zero without one), the controls as they are arranged now
/// (zero before they are measured), and the composer's Send button — whose centre the slot stands
/// under, so the pointer does not move sideways (S3.3). `send_centre_x` is Send's centre from the
/// pane's left edge, NaN when it is not laid out ([`RecorderMeasures::new`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RecorderMeasures {
    pub pane_width: f64,
    pub pane_height: f64,
    pub status_width: f64,
    pub status_height: f64,
    pub banner_width: f64,
    pub banner_height: f64,
    pub controls_width: f64,
    pub controls_height: f64,
    pub send_centre_x: f64,
}

impl RecorderMeasures {
    /// The measures, Send not laid out (`send_centre_x` NaN).
    pub fn new(
        pane_width: f64,
        pane_height: f64,
        status_width: f64,
        status_height: f64,
        banner_width: f64,
        banner_height: f64,
        controls_width: f64,
        controls_height: f64,
    ) -> Self {
        RecorderMeasures {
            pane_width,
            pane_height,
            status_width,
            status_height,
            banner_width,
            banner_height,
            controls_width,
            controls_height,
            send_centre_x: f64::NAN,
        }
    }

    /// The same, with Send's centre laid out at `send_centre_x`.
    pub fn with_send_centre_x(self, send_centre_x: f64) -> Self {
        RecorderMeasures {
            send_centre_x,
            ..self
        }
    }
}

/// The recorder laid out in one pane: its circle's diameter and the boxes the status capsule, the
/// circle (with its ring), the reply banner, the solid bar and the controls on it occupy. In a
/// column the banner rides in the controls where they have the height for it, and `banner` is
/// `None`; where they do not, it stands under the circle beside the bar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RecorderFrame {
    pub layout: RecorderLayout,
    pub diameter: f64,
    pub bar: PaneBox,
    pub controls: PaneBox,
    pub status: PaneBox,
    pub circle: PaneBox,
    pub banner: Option<PaneBox>,
}

/// The bar's padding round its controls, top and bottom (and in a column, each side).
pub const BAR_PADDING: f64 = 12.0;

/// The space kept clear at the pane's sides, and between the bar and what stands over it.
pub const GUTTER: f64 = 16.0;

/// The space between the status capsule, the circle and the banner.
pub const GAP: f64 = 12.0;

/// A column bar is never narrower than this inside its padding — the controls' own width before
/// 2026-10-06.
pub const COLUMN_CONTROLS_WIDTH: f64 = 184.0;

/// The space between the lines of the controls (the grid's row spacing) — in a column, the banner's
/// line too.
pub const CONTROLS_LINE_GAP: f64 = 8.0;

/// The status capsule's padding at each side; its line of text gets the rest of the capsule's
/// width.
pub const STATUS_PADDING_X: f64 = 14.0;

/// The red dot before the clock and the space after it (8 + 8).
pub const STATUS_DOT_ROOM: f64 = 16.0;

/// Where the room runs out — a tiny pane, the largest text and a notice of several lines — the
/// circle keeps at least this much and the status capsule gives up its last lines instead (it is
/// clipped to [`RecorderFrame::status`]): the camera is what the recorder is for.
pub const SHORTEST_CIRCLE: f64 = 96.0;

/// What the ring and its clearance add round the circle, each side (the circle's outer box).
pub const RING_CLEARANCE: f64 = RING_WIDTH + 2.0;

/// Whether the controls, laid out in a single row, are wider than the bar has room for — a narrow
/// pane, large text or long captions — so the row wraps: the slot under Send on its own line, the
/// other controls on the line above it. `send_centre_x` NaN when Send is not laid out.
pub fn row_wraps(pane_width: f64, single_row_width: f64, send_centre_x: f64) -> bool {
    single_row_width > row_room(pane_width, send_centre_x).width
}

/// The recorder in a pane (S3.3, decision 41): `fit`'s layout and its diameter as the most the
/// circle may be, shrunk — below `round_video_rules::SMALLEST_CIRCLE` if it must, to nothing at
/// worst — until the circle overlaps neither the status capsule nor the bar. In a row, the bar runs
/// along the bottom, the controls on it with the slot under Send, and the circle, the status above
/// it and the banner below it stand just over it; in a column the bar stands at the trailing edge,
/// the reply banner in its controls where they have the height for it (else under the circle), and
/// the status and circle are centred in the room beside it.
pub fn frame(fit: RecorderFit, m: &RecorderMeasures) -> RecorderFrame {
    let width = finite(m.pane_width);
    let height = finite(m.pane_height);
    let status_height = finite(m.status_height);
    let controls_height = finite(m.controls_height);
    let preferred = finite(fit.diameter).max(0.0);
    if fit.layout == RecorderLayout::Column {
        // `controls_height` is the column measured WITHOUT the reply banner: the banner rides at its
        // top where the bar has the height for both, and otherwise stands under the circle — a
        // short pane at large text would push the slot off the bottom of the pane, out of reach,
        // before it gave up the banner's line.
        let bar_width = column_bar_width(width, m.controls_width);
        let bar = PaneBox::new(width - bar_width, 0.0, bar_width, height);
        let banner_height = finite(m.banner_height);
        let in_controls =
            banner_height > 0.0 && banner_rides_in_column(height, controls_height, banner_height);
        let drawn_controls = if in_controls {
            controls_height + CONTROLS_LINE_GAP + banner_height
        } else {
            controls_height
        };
        let controls = PaneBox::new(
            bar.x + BAR_PADDING,
            BAR_PADDING.max((height - drawn_controls) / 2.0),
            (bar_width - 2.0 * BAR_PADDING).max(0.0),
            drawn_controls,
        );
        let room_width = bar.x.max(0.0);
        let under = if banner_height > 0.0 && !in_controls {
            GAP + banner_height
        } else {
            0.0
        };
        let (diameter, drawn_status) = share(
            preferred,
            room_width - 2.0 * GUTTER - 2.0 * RING_CLEARANCE,
            height - 2.0 * GUTTER - under,
            status_height,
        );
        let outer = diameter + 2.0 * RING_CLEARANCE;
        let stack = drawn_status + GAP + outer + under;
        let top = GUTTER.max((height - stack) / 2.0);
        let status = centred(room_width, m.status_width, top, drawn_status);
        let circle = PaneBox::new(
            (room_width - outer) / 2.0,
            status.bottom() + GAP,
            outer,
            outer,
        );
        let banner = (under > 0.0).then(|| {
            centred(
                room_width,
                m.banner_width,
                circle.bottom() + GAP,
                banner_height,
            )
        });
        RecorderFrame {
            layout: RecorderLayout::Column,
            diameter,
            bar,
            controls,
            status,
            circle,
            banner,
        }
    } else {
        let bar_height = height.min(controls_height + 2.0 * BAR_PADDING);
        let bar = PaneBox::new(0.0, height - bar_height, width, bar_height);
        let room = row_room(width, m.send_centre_x);
        let controls = PaneBox::new(room.x, bar.y + BAR_PADDING, room.width, controls_height);
        let replying = finite(m.banner_height) > 0.0;
        let banner_height = if replying {
            finite(m.banner_height)
        } else {
            0.0
        };
        // From the bar up: the banner, the circle, the status — so a status that grows a line
        // mid-take grows upward and never moves the circle under the person's eyes.
        let circle_bottom = bar.y - GUTTER - if replying { banner_height + GAP } else { 0.0 };
        let (diameter, drawn_status) = share(
            preferred,
            width - 2.0 * GUTTER - 2.0 * RING_CLEARANCE,
            circle_bottom - GUTTER,
            status_height,
        );
        let outer = diameter + 2.0 * RING_CLEARANCE;
        let circle = PaneBox::new((width - outer) / 2.0, circle_bottom - outer, outer, outer);
        let status = centred(
            width,
            m.status_width,
            circle.y - GAP - drawn_status,
            drawn_status,
        );
        let banner =
            replying.then(|| centred(width, m.banner_width, circle_bottom + GAP, banner_height));
        RecorderFrame {
            layout: RecorderLayout::Row,
            diameter,
            bar,
            controls,
            status,
            circle,
            banner,
        }
    }
}

/// Whether, in a column, the reply banner rides at the top of the controls: the column measured
/// without it, the line gap and the banner fit inside the bar's padding in a pane `pane_height`
/// tall.
pub fn banner_rides_in_column(pane_height: f64, controls_height: f64, banner_height: f64) -> bool {
    finite(controls_height) + CONTROLS_LINE_GAP + finite(banner_height) + 2.0 * BAR_PADDING
        <= finite(pane_height)
}

/// The span the status capsule (and the circle) stand across, known before the frame so the
/// capsule is measured at the width it is drawn at: the whole pane in a row; in a column, the room
/// beside the bar, whose width the controls set.
pub fn status_span(layout: RecorderLayout, pane_width: f64, controls_width: f64) -> f64 {
    let width = finite(pane_width);
    if layout == RecorderLayout::Column {
        (width - column_bar_width(width, controls_width)).max(0.0)
    } else {
        width
    }
}

/// How wide the status capsule's line of text may be in a span (the pane in a row, the room beside
/// the bar in a column): the span less its gutters, the capsule's padding and, while recording, the
/// red dot — so a long sentence wraps inside the capsule, which is measured with it, instead of
/// running out of it.
pub fn status_line_width(span_width: f64, dot: bool) -> f64 {
    let dot_room = if dot { STATUS_DOT_ROOM } else { 0.0 };
    (finite(span_width) - 2.0 * GUTTER - 2.0 * STATUS_PADDING_X - dot_room).max(0.0)
}

/// What of the status line lays the recorder out again when it changes (with the stage, the notice
/// and the controls): the line itself — "Starting camera…" becoming "Not recording", a review's
/// length landing — but not the clock that ticks while recording, whose digits keep one width.
pub fn line_that_lays_out(stage: RecorderStage, line: &str) -> Option<&str> {
    if stage == RecorderStage::Recording {
        None
    } else {
        Some(line)
    }
}

fn column_bar_width(width: f64, controls_width: f64) -> f64 {
    width.min(COLUMN_CONTROLS_WIDTH.max(finite(controls_width)) + 2.0 * BAR_PADDING)
}

/// The circle's diameter and the status capsule's drawn height in `room` — the height the two and
/// the gap between them share: the preferred circle where it fits beside the whole status; else the
/// largest that does, down to [`SHORTEST_CIRCLE`]; and below that the circle keeps the shortest (or
/// what the width allows) and the status is cut to what is left.
fn share(preferred: f64, width_limit: f64, room: f64, status: f64) -> (f64, f64) {
    let limit = preferred.min(width_limit).max(0.0);
    let beside = room - status - GAP - 2.0 * RING_CLEARANCE;
    if beside >= limit.min(SHORTEST_CIRCLE) {
        return (limit.min(beside).floor(), status);
    }
    let diameter = limit
        .min(SHORTEST_CIRCLE)
        .min(room - GAP - 2.0 * RING_CLEARANCE)
        .max(0.0)
        .floor();
    (
        diameter,
        status
            .min(room - GAP - 2.0 * RING_CLEARANCE - diameter)
            .max(0.0),
    )
}

/// The span a row's controls get: the gutter on the left; on the right, so the slot's centre is
/// Send's.
fn row_room(pane_width: f64, send_centre_x: f64) -> PaneBox {
    let width = finite(pane_width);
    let right = if send_centre_x.is_finite() {
        (width - send_centre_x - recorder_look::SLOT_TARGET / 2.0)
            .clamp(BAR_PADDING, BAR_PADDING.max(width / 2.0))
    } else {
        GUTTER
    };
    PaneBox::new(GUTTER, 0.0, (width - GUTTER - right).max(0.0), 0.0)
}

/// A box centred across a span, never wider than the span less its gutters.
fn centred(span_width: f64, width: f64, y: f64, height: f64) -> PaneBox {
    let drawn = finite(width).min(span_width - 2.0 * GUTTER).max(0.0);
    PaneBox::new((span_width - drawn) / 2.0, y, drawn, height)
}

fn finite(value: f64) -> f64 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}
