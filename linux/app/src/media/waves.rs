//! The bars of a voice note (shell-design §10.3, `VoiceLook`): one `gtk::DrawingArea` drawing
//! rounded bars of [`voice_look::BAR_WIDTH`] with [`voice_look::BAR_GAP`] between, each
//! `(2 + level) / 17` of the height, the first `lit` in the lit colour — the voice bubble's
//! waveform (the sender's, or the placeholder), the review and not-sent chips' mini waveform, and
//! the recording row's live line (red, right-aligned). Vectors: crisp at any scale (§25).

use std::cell::RefCell;
use std::rc::Rc;

use fc_logic::voice_look;
use gtk::prelude::*;

/// How the bars are coloured.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Ink {
    /// In someone else's balloon: lit in the accent, the rest the text colour dimmed.
    Accent,
    /// In the reader's own balloon: white, the rest white dimmed.
    OnAccent,
    /// The recording row's live line: the recording red, every bar.
    Recording,
}

struct State {
    levels: Vec<u8>,
    lit: usize,
    ink: Ink,
}

/// The bars (see the module's comment).
#[derive(Clone)]
pub struct Waves {
    area: gtk::DrawingArea,
    state: Rc<RefCell<State>>,
}

impl std::fmt::Debug for Waves {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Waves").finish_non_exhaustive()
    }
}

/// The recording red (libadwaita's destructive colour, both schemes close to it).
const RED: (f64, f64, f64) = (0.88, 0.11, 0.14);

impl Waves {
    /// Bars in a `width × height` area (logical pixels; the width also fixes how many fit).
    pub fn new(width: f64, height: f64, ink: Ink) -> Waves {
        let area = gtk::DrawingArea::builder()
            .content_width(width.round().max(1.0) as i32)
            .content_height(height.round().max(1.0) as i32)
            .valign(gtk::Align::Center)
            .build();
        let state = Rc::new(RefCell::new(State {
            levels: Vec::new(),
            lit: 0,
            ink,
        }));
        {
            let state = Rc::clone(&state);
            area.set_draw_func(move |area, cr, width, height| {
                let state = state.borrow();
                draw(area, cr, f64::from(width), f64::from(height), &state);
            });
        }
        Waves { area, state }
    }

    /// The widget to place.
    pub fn widget(&self) -> &gtk::DrawingArea {
        &self.area
    }

    /// How many bars fit the area's own width.
    pub fn capacity(&self) -> usize {
        voice_look::bar_count(f64::from(self.area.content_width()))
    }

    /// The bars to draw, oldest first.
    pub fn set_levels(&self, levels: Vec<u8>) {
        self.state.borrow_mut().levels = levels;
        self.area.queue_draw();
    }

    /// How many of them are lit (played).
    pub fn set_lit(&self, lit: usize) {
        let changed = {
            let mut state = self.state.borrow_mut();
            let changed = state.lit != lit;
            state.lit = lit;
            changed
        };
        if changed {
            self.area.queue_draw();
        }
    }
}

fn draw(area: &gtk::DrawingArea, cr: &gtk::cairo::Context, width: f64, height: f64, state: &State) {
    let text = area.color();
    let accent = crate::theme::palette().accent;
    let rgb = |c: crate::theme::palette::Rgb| {
        (
            f64::from(c.0) / 255.0,
            f64::from(c.1) / 255.0,
            f64::from(c.2) / 255.0,
        )
    };
    let (lit, rest, rest_alpha) = match state.ink {
        Ink::Accent => (
            rgb(accent),
            (
                f64::from(text.red()),
                f64::from(text.green()),
                f64::from(text.blue()),
            ),
            0.35,
        ),
        Ink::OnAccent => ((1.0, 1.0, 1.0), (1.0, 1.0, 1.0), 0.45),
        Ink::Recording => (RED, RED, 1.0),
    };
    let count = state.levels.len().min(voice_look::bar_count(width));
    let step = voice_look::BAR_WIDTH + voice_look::BAR_GAP;
    let radius = voice_look::BAR_WIDTH / 2.0;
    for (index, level) in state.levels.iter().take(count).enumerate() {
        let bar = voice_look::bar_height(*level, height).max(voice_look::BAR_WIDTH);
        let x = index as f64 * step;
        let y = (height - bar) / 2.0;
        if index < state.lit {
            cr.set_source_rgba(lit.0, lit.1, lit.2, 1.0);
        } else {
            cr.set_source_rgba(rest.0, rest.1, rest.2, rest_alpha);
        }
        rounded_bar(cr, x, y, voice_look::BAR_WIDTH, bar, radius);
        let _ = cr.fill();
    }
}

fn rounded_bar(cr: &gtk::cairo::Context, x: f64, y: f64, width: f64, height: f64, radius: f64) {
    use std::f64::consts::PI;
    let radius = radius.min(width / 2.0).min(height / 2.0);
    cr.new_sub_path();
    cr.arc(x + width - radius, y + radius, radius, -PI / 2.0, 0.0);
    cr.arc(
        x + width - radius,
        y + height - radius,
        radius,
        0.0,
        PI / 2.0,
    );
    cr.arc(x + radius, y + height - radius, radius, PI / 2.0, PI);
    cr.arc(x + radius, y + radius, radius, PI, 1.5 * PI);
    cr.close_path();
}
