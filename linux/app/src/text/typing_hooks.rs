//! The typing aids of a prose field (shell-design §7): capital letters at a sentence's start
//! (multi-line fields) and emoji as you type (every prose field), each taken back by the Backspace
//! straight after. The rules and the state are fc-logic's `CapitalField`; this module only feeds it
//! GTK's events and applies its answers (md's proven hook pattern, md shell-design §3.3):
//!
//! - a capture-phase `gtk::EventControllerKey` classifies each key (`classify_keysym`) and asks
//!   `backspace` before a Backspace acts;
//! - the buffer's `begin-user-action` snapshots the text and the selection, `end-user-action`
//!   judges the change (`before_change`);
//! - the replacement goes in on a HIGH-priority idle as its OWN user action, so the typed character
//!   lands first and Ctrl+Z straight after gives it back;
//! - `preedit-changed` (an input method composing), `paste-clipboard` and focus leaving are told,
//!   so a composition, a paste or a text the program set is never judged.
//!
//! Offsets cross into fc-logic in UTF-16 units through [`super::offsets`]. Nothing typed is logged.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use fc_logic::capital_field::{CapitalField, CapitalKey, CapitalPlan};
use glib::translate::IntoGlib;
use gtk::prelude::*;

use super::offsets;

/// The aids attached to one field. Keep the `Rc` [`attach_text_view`] / [`attach_editable`]
/// answers for as long as the field lives (the signal handlers hold it too).
pub struct TypingHooks {
    field: RefCell<CapitalField>,
    /// `(text, selection start, selection length)` in UTF-16 units, taken when a user action began.
    snapshot: RefCell<Option<(String, usize, usize)>>,
    depth: Cell<u32>,
}

impl std::fmt::Debug for TypingHooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TypingHooks").finish_non_exhaustive()
    }
}

impl TypingHooks {
    fn new(capitals: bool) -> Rc<TypingHooks> {
        Rc::new(TypingHooks {
            field: RefCell::new(CapitalField::new(capitals)),
            snapshot: RefCell::new(None),
            depth: Cell::new(0),
        })
    }
}

/// What a key is to `CapitalField`, from a GTK key event.
pub fn classify(keyval: gtk::gdk::Key, state: gtk::gdk::ModifierType) -> CapitalKey {
    CapitalField::classify_keysym(
        keyval.into_glib(),
        state.contains(gtk::gdk::ModifierType::CONTROL_MASK),
        state.contains(gtk::gdk::ModifierType::ALT_MASK),
        state.contains(gtk::gdk::ModifierType::SUPER_MASK),
    )
}

/// The whole text of a buffer (no hidden text: the composer has none).
pub fn buffer_text(buffer: &gtk::TextBuffer) -> String {
    let (start, end) = buffer.bounds();
    buffer.text(&start, &end, false).to_string()
}

/// The buffer's selection (or bare caret) in UTF-16 units of `text`.
fn buffer_selection(buffer: &gtk::TextBuffer, text: &str) -> (usize, usize) {
    let insert = buffer.iter_at_mark(&buffer.get_insert());
    let bound = buffer.iter_at_mark(&buffer.selection_bound());
    offsets::selection_to_utf16(
        text,
        usize::try_from(insert.offset()).unwrap_or(0),
        usize::try_from(bound.offset()).unwrap_or(0),
    )
}

/// Replaces the UTF-16 range `[at, at + length)` of the buffer with `replacement` and puts the
/// caret at the UTF-16 offset `caret` of the result — one user action. False when a range end is
/// no character boundary (nothing changed).
fn replace_in_buffer(
    buffer: &gtk::TextBuffer,
    at: usize,
    length: usize,
    replacement: &str,
    caret: usize,
) -> bool {
    let text = buffer_text(buffer);
    let Some((start, end)) = offsets::range_to_chars(&text, at, length) else {
        return false;
    };
    let (Ok(start), Ok(end)) = (i32::try_from(start), i32::try_from(end)) else {
        return false;
    };
    buffer.begin_user_action();
    let mut from = buffer.iter_at_offset(start);
    let mut to = buffer.iter_at_offset(end);
    buffer.delete(&mut from, &mut to);
    let mut at_iter = buffer.iter_at_offset(start);
    buffer.insert(&mut at_iter, replacement);
    let now = buffer_text(buffer);
    if let Some(chars) = offsets::utf16_to_chars(&now, caret).and_then(|c| i32::try_from(c).ok()) {
        buffer.place_cursor(&buffer.iter_at_offset(chars));
    }
    buffer.end_user_action();
    true
}

/// Attaches the aids to a multi-line prose field (`capitals`: the composer, the thread reply, a
/// poll's question, a report's note) or a field with emoji only.
pub fn attach_text_view(view: &gtk::TextView, capitals: bool) -> Rc<TypingHooks> {
    let hooks = TypingHooks::new(capitals);
    let buffer = view.buffer();

    // Keys, before the field acts on them.
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let (hooks, buffer) = (Rc::clone(&hooks), buffer.clone());
        keys.connect_key_pressed(move |_, keyval, _, state| {
            let kind = classify(keyval, state);
            if kind == CapitalKey::Backspace {
                let text = buffer_text(&buffer);
                let (start, length) = buffer_selection(&buffer, &text);
                let restore = hooks.field.borrow_mut().backspace(&text, start, length);
                if let Some(restore) = restore {
                    hooks.field.borrow_mut().set_applying(true);
                    let done = replace_in_buffer(
                        &buffer,
                        restore.at,
                        restore.length,
                        &restore.replacement,
                        restore.caret,
                    );
                    hooks.field.borrow_mut().set_applying(false);
                    if done {
                        return glib::Propagation::Stop;
                    }
                }
            }
            hooks.field.borrow_mut().key(kind);
            glib::Propagation::Proceed
        });
    }
    view.add_controller(keys);

    // The change, judged at the user action's end.
    {
        let hooks = Rc::clone(&hooks);
        buffer.connect_begin_user_action(move |buffer| {
            let depth = hooks.depth.get();
            hooks.depth.set(depth + 1);
            if depth == 0 && !hooks.field.borrow().applying() {
                let text = buffer_text(buffer);
                let (start, length) = buffer_selection(buffer, &text);
                *hooks.snapshot.borrow_mut() = Some((text, start, length));
            }
        });
    }
    {
        let hooks = Rc::clone(&hooks);
        buffer.connect_end_user_action(move |buffer| {
            let depth = hooks.depth.get().saturating_sub(1);
            hooks.depth.set(depth);
            if depth != 0 {
                return;
            }
            let Some((old, start, length)) = hooks.snapshot.borrow_mut().take() else {
                return;
            };
            let new = buffer_text(buffer);
            if new == old {
                return;
            }
            let plan = hooks
                .field
                .borrow_mut()
                .before_change(&old, start, length, &new);
            if let Some(plan) = plan {
                apply_later(&hooks, buffer, plan);
            }
        });
    }
    {
        let hooks = Rc::clone(&hooks);
        buffer.connect_mark_set(move |buffer, _, mark| {
            if hooks.depth.get() != 0 || hooks.field.borrow().applying() {
                return;
            }
            if mark != &buffer.get_insert() && mark != &buffer.selection_bound() {
                return;
            }
            let text = buffer_text(buffer);
            let (start, length) = buffer_selection(buffer, &text);
            hooks.field.borrow_mut().selection_moved(start, length);
        });
    }
    {
        let hooks = Rc::clone(&hooks);
        view.connect_preedit_changed(move |_, preedit| {
            if preedit.is_empty() {
                hooks.field.borrow_mut().composition_ended();
            } else {
                hooks.field.borrow_mut().composition_started();
            }
        });
    }
    {
        let hooks = Rc::clone(&hooks);
        view.connect_paste_clipboard(move |_| hooks.field.borrow_mut().pasting());
    }
    let focus = gtk::EventControllerFocus::new();
    {
        let hooks = Rc::clone(&hooks);
        focus.connect_leave(move |_| hooks.field.borrow_mut().focus_lost());
    }
    view.add_controller(focus);
    hooks
}

/// The plan, applied on the next high-priority turn — after the typed character landed, ahead of
/// the next input — as its own user action, if it still fits the field then.
fn apply_later(hooks: &Rc<TypingHooks>, buffer: &gtk::TextBuffer, plan: CapitalPlan) {
    let (hooks, buffer) = (Rc::clone(hooks), buffer.clone());
    glib::idle_add_local_full(glib::Priority::HIGH, move || {
        let text = buffer_text(&buffer);
        let (start, length) = buffer_selection(&buffer, &text);
        let fits = hooks
            .field
            .borrow()
            .still_applies(&plan, &text, start, length, 0);
        if fits {
            hooks.field.borrow_mut().set_applying(true);
            let done =
                replace_in_buffer(&buffer, plan.at, plan.length, &plan.replacement, plan.caret);
            hooks.field.borrow_mut().set_applying(false);
            if done {
                hooks.field.borrow_mut().applied(&plan);
            }
        }
        glib::ControlFlow::Break
    });
}

/// Attaches emoji-as-you-type to a single-line field (a poll's option): `gtk::Editable`'s
/// `insert-text` judges a typed character, the replacement goes in on a high-priority idle.
pub fn attach_editable(
    editable: &impl IsA<gtk::Editable>,
    widget: &impl IsA<gtk::Widget>,
) -> Rc<TypingHooks> {
    let hooks = TypingHooks::new(false);
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let (hooks, editable) = (
            Rc::clone(&hooks),
            editable.clone().upcast::<gtk::Editable>(),
        );
        keys.connect_key_pressed(move |_, keyval, _, state| {
            let kind = classify(keyval, state);
            if kind == CapitalKey::Backspace {
                let text = editable.text().to_string();
                let (a, b) = editable.selection_bounds().unwrap_or_else(|| {
                    let at = editable.position();
                    (at, at)
                });
                let (start, length) = offsets::selection_to_utf16(
                    &text,
                    usize::try_from(a).unwrap_or(0),
                    usize::try_from(b).unwrap_or(0),
                );
                let restore = hooks.field.borrow_mut().backspace(&text, start, length);
                if let Some(restore) = restore
                    && let Some((from, to)) =
                        offsets::range_to_chars(&text, restore.at, restore.length)
                {
                    hooks.field.borrow_mut().set_applying(true);
                    replace_in_editable(&editable, from, to, &restore.replacement, restore.caret);
                    hooks.field.borrow_mut().set_applying(false);
                    return glib::Propagation::Stop;
                }
            }
            hooks.field.borrow_mut().key(kind);
            glib::Propagation::Proceed
        });
    }
    widget.add_controller(keys);
    {
        let hooks = Rc::clone(&hooks);
        editable.connect_insert_text(move |editable, inserted, position| {
            let editable = editable.clone().upcast::<gtk::Editable>();
            if hooks.field.borrow().applying() {
                return;
            }
            let old = editable.text().to_string();
            let at = usize::try_from(*position).unwrap_or(0);
            let byte = offsets::chars_to_byte(&old, at);
            let mut new = String::with_capacity(old.len() + inserted.len());
            new.push_str(&old[..byte]);
            new.push_str(inserted);
            new.push_str(&old[byte..]);
            let start = offsets::chars_to_utf16(&old, at);
            let plan = hooks.field.borrow_mut().before_change(&old, start, 0, &new);
            if let Some(plan) = plan {
                let hooks = Rc::clone(&hooks);
                glib::idle_add_local_full(glib::Priority::HIGH, move || {
                    let text = editable.text().to_string();
                    let caret = offsets::chars_to_utf16(
                        &text,
                        usize::try_from(editable.position()).unwrap_or(0),
                    );
                    let selected = editable.selection_bounds().is_some_and(|(a, b)| a != b);
                    let fits = !selected
                        && hooks
                            .field
                            .borrow()
                            .still_applies(&plan, &text, caret, 0, 0);
                    if fits
                        && let Some((from, to)) =
                            offsets::range_to_chars(&text, plan.at, plan.length)
                    {
                        hooks.field.borrow_mut().set_applying(true);
                        replace_in_editable(&editable, from, to, &plan.replacement, plan.caret);
                        hooks.field.borrow_mut().set_applying(false);
                        hooks.field.borrow_mut().applied(&plan);
                    }
                    glib::ControlFlow::Break
                });
            }
        });
    }
    let focus = gtk::EventControllerFocus::new();
    {
        let hooks = Rc::clone(&hooks);
        focus.connect_leave(move |_| hooks.field.borrow_mut().focus_lost());
    }
    widget.add_controller(focus);
    hooks
}

fn replace_in_editable(
    editable: &gtk::Editable,
    from: usize,
    to: usize,
    replacement: &str,
    caret: usize,
) {
    let (Ok(from), Ok(to)) = (i32::try_from(from), i32::try_from(to)) else {
        return;
    };
    editable.delete_text(from, to);
    let mut at = from;
    editable.insert_text(replacement, &mut at);
    let now = editable.text().to_string();
    if let Some(chars) = offsets::utf16_to_chars(&now, caret).and_then(|c| i32::try_from(c).ok()) {
        editable.set_position(chars);
    }
}
