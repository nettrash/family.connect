//! The voice recording row (shell-design §10.2 — the Mac's `MacVoiceRecordingRow`; S2.4, S2.5,
//! S2.9): it takes the field's place in P1's composer while a voice note records — **Delete, the
//! red dot, the clock, the meter, Stop** — at the field's height, so nothing above it moves.
//!
//! - The red dot pulses between 100 % and 40 % once a second; still under Reduce Motion.
//! - The clock reads whole seconds (`voice_notes::clock`), tabular digits; from 4:30 it turns
//!   orange and "30 seconds left" takes the meter's place — words as well as colour (WCAG 1.4.1).
//! - The meter is the live waveform: the newest peaks scroll in from the trailing edge, a bar a
//!   tick, in the recording red (`LiveLevels`); five static bars under Reduce Motion; none in a row
//!   too narrow for eight.
//! - Stop is shown only when the slot is not Stop already (row 2: the slot is the Send arrow).
//!   Delete and Stop become icons below 300 px, with the same names.
//!
//! Every word is the catalogue's; the row is drawn by P3 and placed by P1 (`ComposerRecording::row`
//! — the same widget every time).

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use fc_core::rt::Event;
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::voice_look;
use fc_logic::voice_notes;
use gtk::prelude::*;

use crate::media::waves::{Ink, Waves};

/// The meter's height in the row.
const METER_HEIGHT: f64 = 24.0;
/// The meter's widest (the row gives it what is left).
const METER_WIDTH: f64 = 160.0;
/// Below this width Delete and Stop are icons.
const NARROW: i32 = 300;
/// The pulse: half a second each way.
const PULSE: Duration = Duration::from_millis(500);
/// The five still bars of Reduce Motion.
const STILL_BARS: [u8; 5] = [6, 10, 14, 10, 6];

/// What the person pressed in the row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowPress {
    Delete,
    Stop,
}

/// The row (see the module's comment).
pub struct RecordingRow {
    root: gtk::Box,
    delete: gtk::Button,
    dot: gtk::Box,
    clock: gtk::Label,
    meter: Waves,
    warning: gtk::Label,
    /// "We can't hear anything. Is the microphone muted?" — in the meter's place (S2.9).
    silence: gtk::Label,
    stop: gtk::Button,
    pressed: Event<RowPress>,
    pulse: RefCell<Option<glib::SourceId>>,
    still: Cell<bool>,
    narrow: Cell<bool>,
}

impl std::fmt::Debug for RecordingRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordingRow").finish_non_exhaustive()
    }
}

impl RecordingRow {
    pub fn new(say: &dyn StringCatalog) -> Rc<RecordingRow> {
        let delete = gtk::Button::builder()
            .label(say.get("Delete"))
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .tooltip_text(say.get("Delete recording"))
            .build();
        delete.update_property(&[gtk::accessible::Property::Label(
            say.get("Delete recording"),
        )]);
        let dot = gtk::Box::builder()
            .width_request(10)
            .height_request(10)
            .valign(gtk::Align::Center)
            .css_classes(["fc-media-recording-dot"])
            .build();
        let clock = gtk::Label::builder()
            .label(voice_notes::clock(Duration::ZERO))
            .css_classes(["numeric", "fc-media-time"])
            .valign(gtk::Align::Center)
            .build();
        let meter = Waves::new(METER_WIDTH, METER_HEIGHT, Ink::Recording);
        meter.widget().set_hexpand(true);
        meter.widget().set_halign(gtk::Align::End);
        let warning = gtk::Label::builder()
            .label(say.get("30 seconds left"))
            .css_classes(["fc-media-warning"])
            .hexpand(true)
            .xalign(0.0)
            .visible(false)
            .build();
        let silence = gtk::Label::builder()
            .label(say.get("We can't hear anything. Is the microphone muted?"))
            .css_classes(["caption", "fc-media-warning"])
            .hexpand(true)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .tooltip_text(say.get("We can't hear anything. Is the microphone muted?"))
            .visible(false)
            .build();
        let stop = gtk::Button::builder()
            .label(say.get("Stop"))
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .tooltip_text(say.get("Stop recording"))
            .build();
        stop.update_property(&[gtk::accessible::Property::Label(say.get("Stop recording"))]);
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(10)
            .hexpand(true)
            .css_classes(["fc-media-recording-row"])
            .build();
        root.append(&delete);
        root.append(&dot);
        root.append(&clock);
        root.append(meter.widget());
        root.append(&warning);
        root.append(&silence);
        root.append(&stop);
        root.update_property(&[gtk::accessible::Property::Label(say.get("Recording"))]);
        let row = Rc::new(RecordingRow {
            root,
            delete,
            dot,
            clock,
            meter,
            warning,
            silence,
            stop,
            pressed: Event::new(),
            pulse: RefCell::new(None),
            still: Cell::new(false),
            narrow: Cell::new(false),
        });
        let weak = Rc::downgrade(&row);
        row.delete.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(row) = weak.upgrade() {
                    row.pressed.emit(&RowPress::Delete);
                }
            }
        });
        row.stop.connect_clicked(move |_| {
            if let Some(row) = weak.upgrade() {
                row.pressed.emit(&RowPress::Stop);
            }
        });
        row
    }

    /// The widget P1 shows in the field's place — the same one every time.
    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    /// What the person pressed.
    pub fn pressed(&self) -> &Event<RowPress> {
        &self.pressed
    }

    /// A recording begins: a fresh row. `beside_draft`: the slot is Stop already (row 3), so the
    /// row's own Stop is not drawn. `still`: Reduce Motion (no pulse, five static bars).
    pub fn begin(&self, beside_draft: bool, still: bool, say: &dyn StringCatalog) {
        self.still.set(still);
        self.stop.set_visible(!beside_draft);
        self.warning.set_visible(false);
        self.silence.set_visible(false);
        self.clock.remove_css_class("fc-media-warning");
        self.clock.set_label(&voice_notes::clock(Duration::ZERO));
        self.meter.set_levels(if still {
            STILL_BARS.to_vec()
        } else {
            Vec::new()
        });
        self.meter.widget().set_visible(true);
        self.fit(say);
        self.start_pulse();
    }

    /// The recording ended: the pulse stops.
    pub fn end(&self) {
        if let Some(pulse) = self.pulse.borrow_mut().take() {
            pulse.remove();
        }
        self.dot.set_opacity(1.0);
    }

    /// One tick (an eighth of a second): the clock, the meter or the warning.
    /// `silent`: nothing above the silence line yet, three seconds in — the warning takes the
    /// meter's place until a sound arrives (the Mac's `showsSilenceWarning`).
    pub fn tick(
        &self,
        elapsed: Duration,
        levels: &voice_look::LiveLevels,
        silent: bool,
        say: &dyn StringCatalog,
    ) {
        self.clock.set_label(&voice_notes::clock(elapsed));
        self.fit(say);
        if voice_notes::warns(elapsed) {
            self.clock.add_css_class("fc-media-warning");
            self.warning.set_visible(true);
            self.silence.set_visible(false);
            self.meter.widget().set_visible(false);
            return;
        }
        self.silence.set_visible(silent);
        if silent {
            self.meter.widget().set_visible(false);
            return;
        }
        if self.still.get() {
            return;
        }
        let room = self.meter.capacity();
        let shown = levels.heard() > 0 && room >= 8;
        self.meter.widget().set_visible(shown);
        if shown {
            self.meter.set_levels(levels.bars(room));
        }
    }

    /// How many bars the meter holds (for `LiveLevels::new`).
    pub fn capacity(&self) -> usize {
        self.meter.capacity()
    }

    /// Too narrow for words (S2.4): Delete and Stop become icons, with the same names.
    fn fit(&self, say: &dyn StringCatalog) {
        let width = self.root.width();
        let narrow = width > 0 && width < NARROW;
        if self.narrow.replace(narrow) == narrow {
            return;
        }
        if narrow {
            self.delete.set_icon_name("user-trash-symbolic");
            self.stop.set_icon_name("media-playback-stop-symbolic");
        } else {
            self.delete.set_label(say.get("Delete"));
            self.stop.set_label(say.get("Stop"));
        }
    }

    fn start_pulse(&self) {
        if let Some(pulse) = self.pulse.borrow_mut().take() {
            pulse.remove();
        }
        self.dot.set_opacity(1.0);
        if self.still.get() {
            return;
        }
        let dot = self.dot.downgrade();
        let bright = Cell::new(true);
        let source = glib::timeout_add_local(PULSE, move || {
            let Some(dot) = dot.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let now = !bright.get();
            bright.set(now);
            dot.set_opacity(if now { 1.0 } else { 0.4 });
            glib::ControlFlow::Continue
        });
        *self.pulse.borrow_mut() = Some(source);
    }
}

impl Drop for RecordingRow {
    fn drop(&mut self) {
        if let Some(pulse) = self.pulse.borrow_mut().take() {
            pulse.remove();
        }
    }
}
