//! The timeline's widget (shell-design §5.1): a vertical `gtk::Box` of row widgets in a
//! `gtk::ScrolledWindow` — BOUNDED AND NOT LAZY (a `gtk::ListView` recycles rows, tearing an inline
//! video down on every scroll, and its height estimates move the anchor when a giant row lands —
//! the Apple clients' scroll history) — centred in a column at most `MAX_COLUMN` wide with
//! `PANE_INSET` clear either side.
//!
//! **Redraws keep what has not changed**: each row comes with a `conversation_rows::RowMark`;
//! `conversation_rows::plan` says which drawn widgets may stay, and `conversation_rows::merge`
//! turns the box into the rows wanted with the fewest insertions and removals — a kept widget is
//! never moved (a sticker or a circle moved is a teardown and a start).
//!
//! **Anchoring by hand**: pinned to the bottom while the reader is at the newest (the adjustment's
//! `upper` grows → value = upper − page); a page of history prepended keeps the first visible row
//! where it was (the distance from the bottom is kept); the reader scrolled up stays put when
//! something arrives below. "Jump to newest" floats over the corner while the reader is not at the
//! bottom.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use fc_logic::composer_row::conversation_column;
use fc_logic::conversation_rows::{self, RowMark};

/// How long a row a quote led to stays tinted (the Mac's 1.6 s fade).
const JUMP_TINT: std::time::Duration = std::time::Duration::from_millis(1600);

/// How close to the bottom (px) still counts as "at the newest".
const BOTTOM_SLACK: f64 = 24.0;
/// How close to the top (px) asks for an older page.
const TOP_SLACK: f64 = 160.0;

/// Where the next layout keeps the view.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Keep {
    /// At the bottom.
    Bottom,
    /// This far from the bottom (a page prepended above).
    FromBottom(f64),
    /// Where it is.
    Value,
}

/// The timeline (see the module's comment).
pub struct Timeline {
    overlay: gtk::Overlay,
    scroller: gtk::ScrolledWindow,
    column: gtk::Box,
    shown: RefCell<Vec<gtk::Widget>>,
    marks: RefCell<Vec<RowMark>>,
    pinned: Cell<bool>,
    keep: Cell<Keep>,
    jump: gtk::Button,
    /// At the top while an older page is on its way (the Mac's small spinner over the history).
    older: adw::Spinner,
    /// What stands in the middle while there is nothing to draw (a thread loading, or failed).
    placeholder: RefCell<Option<gtk::Widget>>,
    /// Called when the reader nears the top (page back) or reaches the bottom (read).
    on_top: RefCell<Option<Box<dyn Fn()>>>,
    on_bottom: RefCell<Option<Box<dyn Fn()>>>,
}

impl std::fmt::Debug for Timeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Timeline")
            .field("rows", &self.shown.borrow().len())
            .finish_non_exhaustive()
    }
}

impl Timeline {
    pub fn new(jump_label: &str) -> Rc<Timeline> {
        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.set_margin_top(8);
        column.set_margin_bottom(8);
        let clamp = adw::Clamp::builder()
            .maximum_size(conversation_column::MAX_COLUMN as i32)
            .tightening_threshold(conversation_column::MAX_COLUMN as i32)
            .child(&column)
            .margin_start(conversation_column::PANE_INSET as i32)
            .margin_end(conversation_column::PANE_INSET as i32)
            .build();
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&clamp)
            .build();
        let jump = gtk::Button::builder()
            .icon_name("go-bottom-symbolic")
            .css_classes(["osd", "fc-chat-jump"])
            .halign(gtk::Align::End)
            .valign(gtk::Align::End)
            .margin_end(28)
            .margin_bottom(12)
            .tooltip_text(jump_label)
            .visible(false)
            .build();
        jump.update_property(&[gtk::accessible::Property::Label(jump_label)]);
        let older = adw::Spinner::builder()
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Start)
            .margin_top(10)
            .width_request(20)
            .height_request(20)
            .can_target(false)
            .visible(false)
            .build();
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&scroller));
        overlay.add_overlay(&jump);
        overlay.add_overlay(&older);
        let timeline = Rc::new(Timeline {
            overlay,
            scroller,
            column,
            shown: RefCell::new(Vec::new()),
            marks: RefCell::new(Vec::new()),
            pinned: Cell::new(true),
            keep: Cell::new(Keep::Bottom),
            jump,
            older,
            placeholder: RefCell::new(None),
            on_top: RefCell::new(None),
            on_bottom: RefCell::new(None),
        });
        timeline.follow();
        timeline
    }

    fn follow(self: &Rc<Self>) {
        let adjustment = self.scroller.vadjustment();
        {
            let weak = Rc::downgrade(self);
            adjustment.connect_value_changed(move |adjustment| {
                let Some(timeline) = weak.upgrade() else {
                    return;
                };
                let at_bottom = at_bottom(adjustment);
                timeline.pinned.set(at_bottom);
                timeline.jump.set_visible(!at_bottom);
                if at_bottom {
                    timeline.keep.set(Keep::Bottom);
                    if let Some(on_bottom) = timeline.on_bottom.borrow().as_ref() {
                        on_bottom();
                    }
                } else if timeline.keep.get() == Keep::Bottom {
                    timeline.keep.set(Keep::Value);
                }
                if adjustment.value() < TOP_SLACK
                    && adjustment.upper() > adjustment.page_size()
                    && let Some(on_top) = timeline.on_top.borrow().as_ref()
                {
                    on_top();
                }
            });
        }
        {
            let weak = Rc::downgrade(self);
            adjustment.connect_upper_notify(move |adjustment| {
                let Some(timeline) = weak.upgrade() else {
                    return;
                };
                match timeline.keep.get() {
                    Keep::Bottom => {
                        timeline.move_view(adjustment.upper() - adjustment.page_size());
                    }
                    Keep::FromBottom(distance) => {
                        timeline.move_view((adjustment.upper() - distance).max(0.0));
                        timeline.keep.set(Keep::Value);
                    }
                    Keep::Value => {}
                }
            });
        }
        {
            let weak = Rc::downgrade(self);
            adjustment.connect_page_size_notify(move |adjustment| {
                if let Some(timeline) = weak.upgrade()
                    && timeline.keep.get() == Keep::Bottom
                {
                    timeline.move_view(adjustment.upper() - adjustment.page_size());
                }
            });
        }
        {
            let weak = Rc::downgrade(self);
            self.jump.connect_clicked(move |_| {
                if let Some(timeline) = weak.upgrade() {
                    timeline.scroll_to_bottom();
                }
            });
        }
        // Up and Down move between bubbles once one has focus (§24).
        let keys = gtk::EventControllerKey::new();
        {
            let column = self.column.clone();
            keys.connect_key_pressed(move |_, key, _, state| {
                if state.intersects(
                    gtk::gdk::ModifierType::CONTROL_MASK
                        | gtk::gdk::ModifierType::ALT_MASK
                        | gtk::gdk::ModifierType::SHIFT_MASK,
                ) {
                    return glib::Propagation::Proceed;
                }
                let forward = match key {
                    gtk::gdk::Key::Down => true,
                    gtk::gdk::Key::Up => false,
                    _ => return glib::Propagation::Proceed,
                };
                let Some(focused) = column.focus_child() else {
                    return glib::Propagation::Proceed;
                };
                let mut next = if forward {
                    focused.next_sibling()
                } else {
                    focused.prev_sibling()
                };
                while let Some(candidate) = next {
                    if candidate.is_focusable() && candidate.is_visible() {
                        candidate.grab_focus();
                        return glib::Propagation::Stop;
                    }
                    next = if forward {
                        candidate.next_sibling()
                    } else {
                        candidate.prev_sibling()
                    };
                }
                glib::Propagation::Stop
            });
        }
        self.column.add_controller(keys);
    }

    /// Moves the view to `value` from an `upper` or `page-size` notification. GtkViewport sends
    /// those from inside its own allocation, after it has placed its child, and the allocation
    /// that `value-changed` asks for is dropped there — measured: the adjustment said the newest
    /// while the rows stayed drawn at the old offset until some unrelated layout (a new message
    /// hidden below the composer; after the dark scheme's redraw, the first rows). So ask again
    /// once that allocation is over.
    fn move_view(&self, value: f64) {
        self.scroller.vadjustment().set_value(value);
        if let Some(viewport) = self.scroller.child() {
            glib::idle_add_local_once(move || viewport.queue_allocate());
        }
    }

    /// The widget to place.
    pub fn widget(&self) -> &gtk::Overlay {
        &self.overlay
    }

    pub fn set_on_top(&self, handler: impl Fn() + 'static) {
        *self.on_top.borrow_mut() = Some(Box::new(handler));
    }

    pub fn set_on_bottom(&self, handler: impl Fn() + 'static) {
        *self.on_bottom.borrow_mut() = Some(Box::new(handler));
    }

    /// Whether the reader is at the newest.
    pub fn at_newest(&self) -> bool {
        self.pinned.get()
    }

    /// Draws `wanted`; `build(i)` makes row i's widget when it cannot be kept. `prepended`: the
    /// change adds history above (the view keeps its place from the bottom).
    pub fn update(&self, wanted: Vec<RowMark>, build: impl Fn(usize) -> gtk::Widget) {
        let drawn = self.marks.borrow().clone();
        let plan = conversation_rows::plan(&drawn, &wanted, None);
        let old: Vec<gtk::Widget> = self.shown.borrow().clone();
        let widgets: Vec<gtk::Widget> = plan
            .iter()
            .enumerate()
            .map(|(index, kept)| match kept.and_then(|at| old.get(at)) {
                Some(widget) => widget.clone(),
                None => build(index),
            })
            .collect();
        let prepended = match (drawn.first(), wanted.first()) {
            (Some(was), Some(now)) => was.key != now.key && wanted.iter().any(|m| m.key == was.key),
            _ => false,
        };
        if prepended && !self.pinned.get() {
            let adjustment = self.scroller.vadjustment();
            self.keep
                .set(Keep::FromBottom(adjustment.upper() - adjustment.value()));
        }
        let column = self.column.clone();
        let mut shown = self.shown.borrow_mut();
        let merged = conversation_rows::merge(
            &mut shown,
            &widgets,
            |index| {
                if let Some(child) = nth_child(&column, index) {
                    column.remove(&child);
                }
            },
            |index, widget| {
                let after = if index == 0 {
                    None
                } else {
                    nth_child(&column, index - 1)
                };
                column.insert_child_after(widget, after.as_ref());
            },
        );
        if merged.is_none() {
            // The kept widgets were out of order: draw them all again, as it used to.
            while let Some(child) = column.first_child() {
                column.remove(&child);
            }
            for widget in &widgets {
                column.append(widget);
            }
            *shown = widgets;
        }
        drop(shown);
        *self.marks.borrow_mut() = wanted;
    }

    /// Forget every drawn row: the next update builds them all (the roster, the block list or the
    /// theme changed — what is in no row's signature).
    pub fn clear(&self) {
        if self.pinned.get() {
            self.keep.set(Keep::Bottom);
        }
        while let Some(child) = self.column.first_child() {
            self.column.remove(&child);
        }
        self.shown.borrow_mut().clear();
        self.marks.borrow_mut().clear();
    }

    /// Down to the newest, and follow it from here.
    pub fn scroll_to_bottom(&self) {
        self.keep.set(Keep::Bottom);
        self.pinned.set(true);
        let adjustment = self.scroller.vadjustment();
        adjustment.set_value(adjustment.upper() - adjustment.page_size());
        // Once more after the layout the change may still cause.
        let scroller = self.scroller.clone();
        glib::idle_add_local_once(move || {
            let adjustment = scroller.vadjustment();
            adjustment.set_value(adjustment.upper() - adjustment.page_size());
        });
    }

    /// The spinner at the top while an older page loads.
    pub fn set_loading_older(&self, loading: bool) {
        self.older.set_visible(loading);
    }

    /// `widget` in the middle of the view, over the (empty) rows — `None` takes it away.
    pub fn set_placeholder(&self, widget: Option<&gtk::Widget>) {
        if let Some(old) = self.placeholder.borrow_mut().take() {
            self.overlay.remove_overlay(&old);
        }
        if let Some(widget) = widget {
            self.overlay.add_overlay(widget);
            *self.placeholder.borrow_mut() = Some(widget.clone());
        }
    }

    /// The row drawn for `key`, if it is drawn.
    pub fn row(&self, key: &str) -> Option<gtk::Widget> {
        let index = self
            .marks
            .borrow()
            .iter()
            .position(|mark| mark.key == key)?;
        self.shown.borrow().get(index).cloned()
    }

    /// Brings the row with `key` to the top of the view (an opening anchor, a quote's message);
    /// `focus`: the row takes the keyboard focus too, and is tinted for a moment so the eye finds
    /// it (a quote followed — the Mac's fading highlight; never an opening, which must not take
    /// the focus from the chat list). False when no such row is drawn.
    pub fn scroll_to(&self, key: &str, focus: bool) -> bool {
        let Some(index) = self.marks.borrow().iter().position(|mark| mark.key == key) else {
            return false;
        };
        let Some(widget) = self.shown.borrow().get(index).cloned() else {
            return false;
        };
        self.keep.set(Keep::Value);
        let scroller = self.scroller.clone();
        glib::idle_add_local_once(move || {
            if let Some(point) =
                widget.compute_point(&scroller, &gtk::graphene::Point::new(0.0, 0.0))
            {
                let adjustment = scroller.vadjustment();
                let target = adjustment.value() + f64::from(point.y()) - 8.0;
                adjustment.set_value(target.max(0.0));
            }
            if focus && widget.root().is_some() {
                widget.grab_focus();
                widget.add_css_class("fc-chat-jumped");
                let tinted = widget.downgrade();
                glib::timeout_add_local_once(JUMP_TINT, move || {
                    if let Some(widget) = tinted.upgrade() {
                        widget.remove_css_class("fc-chat-jumped");
                    }
                });
            }
        });
        true
    }

    /// The adjustment (value, upper, page) and where the newest row is DRAWN in the scroller (its
    /// top and bottom, px) — the self-test compares the two.
    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "the chat self-test reads it")
    )]
    pub fn placement(&self) -> (f64, f64, f64, Option<(f32, f32)>) {
        let adjustment = self.scroller.vadjustment();
        let newest = self
            .shown
            .borrow()
            .last()
            .and_then(|row| row.compute_bounds(&self.scroller))
            .map(|bounds| (bounds.y(), bounds.y() + bounds.height()));
        (
            adjustment.value(),
            adjustment.upper(),
            adjustment.page_size(),
            newest,
        )
    }

    /// The row widgets drawn, in order (the self-test reads them).
    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "the chat self-test reads it")
    )]
    pub fn rows(&self) -> Vec<gtk::Widget> {
        self.shown.borrow().clone()
    }

    /// The keys drawn, in order.
    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "the chat self-test reads it")
    )]
    pub fn keys(&self) -> Vec<String> {
        self.marks.borrow().iter().map(|m| m.key.clone()).collect()
    }
}

fn at_bottom(adjustment: &gtk::Adjustment) -> bool {
    adjustment.value() + adjustment.page_size() >= adjustment.upper() - BOTTOM_SLACK
}

fn nth_child(parent: &gtk::Box, index: usize) -> Option<gtk::Widget> {
    let mut child = parent.first_child();
    for _ in 0..index {
        child = child?.next_sibling();
    }
    child
}
