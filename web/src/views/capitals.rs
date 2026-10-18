//! Capital letters at the start of a sentence, on a computer (issue #97;
//! docs/capitals-and-rich-markdown-2026-10-17.md, "1. Capital letters").
//!
//! A phone's keyboard capitalises by itself once the field asks
//! (`autocapitalize="sentences"`, on every prose field). A computer's never
//! does, so on a device with a fine pointer and hover the MULTI-LINE prose
//! fields — the composer (chat, thread and edit), a board note, a report's
//! note — do it themselves, by `fc_text::composer::capitalizes`, the rule
//! the Mac and Windows hold too. Never a single-line field: that is the set
//! every desktop client agrees on.
//!
//! The letter is replaced in `beforeinput`, before the browser puts it in,
//! and the capital goes in with `insertText` — so the field's own Undo knows
//! it, and the `input` the component already listens to keeps Yew's value in
//! step, exactly as #95's new line does. A Backspace pressed straight after
//! puts the lower-case letter back instead of deleting it; anything else
//! forgets that it could.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use fc_text::composer::{self, TakeBack};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{Element, HtmlTextAreaElement, InputEvent, KeyboardEvent};
use yew::prelude::*;

/// A computer: a pointer that can hover and aim finely. A phone or a tablet
/// answers no and is left to its keyboard — which does capitalise, and
/// would otherwise see every letter it capitalised capitalised again.
fn on_a_computer() -> bool {
    #[cfg(test)]
    if let Some(forced) = tests::FORCED.with(Cell::get) {
        return forced;
    }
    web_sys::window()
        .and_then(|window| {
            window
                .match_media("(hover: hover) and (pointer: fine)")
                .ok()
        })
        .flatten()
        .is_some_and(|list| list.matches())
}

/// `text` at the caret, over the selection, the way #95's new line goes
/// in: `insertText` first, which the field's Undo knows and which fires the
/// `input` the component listens to; where a browser refuses it, the range
/// is replaced and that `input` sent by hand.
pub fn insert_at_caret(area: &HtmlTextAreaElement, text: &str) {
    let inserted = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.dyn_into::<web_sys::HtmlDocument>().ok())
        .is_some_and(|document| {
            document
                .exec_command_with_show_ui_and_value("insertText", false, text)
                .unwrap_or(false)
        });
    if inserted {
        return;
    }
    let (Ok(Some(start)), Ok(Some(end))) = (area.selection_start(), area.selection_end()) else {
        return;
    };
    if area
        .set_range_text_with_start_and_end_and_mode(text, start, end, "end")
        .is_err()
    {
        return;
    }
    let init = web_sys::EventInit::new();
    init.set_bubbles(true);
    if let Ok(input) = web_sys::Event::new_with_event_init_dict("input", &init) {
        let _ = area.dispatch_event(&input);
    }
}

/// The byte offset in `text` of a browser's UTF-16 offset (`selectionStart`
/// counts UTF-16 units; `fc_text` counts bytes). One inside a surrogate pair
/// lands after the character.
fn byte_offset(text: &str, utf16: u32) -> usize {
    let mut units = 0u32;
    for (at, c) in text.char_indices() {
        if units >= utf16 {
            return at;
        }
        units += c.len_utf16() as u32;
    }
    text.len()
}

/// The UTF-16 offset of a byte offset in `text`.
fn utf16_offset(text: &str, byte: usize) -> u32 {
    text[..byte].encode_utf16().count() as u32
}

/// One field's memory: the capital a Backspace may still take back, and
/// whether the text going in now is this module's own.
#[derive(Default)]
struct Memory {
    take_back: RefCell<Option<TakeBack>>,
    /// Set while this module inserts: its own `insertText` fires a
    /// `beforeinput` too, and the lower-case letter a take-back puts in
    /// would otherwise be capitalised straight back.
    inserting: Cell<bool>,
}

impl Memory {
    fn forget(&self) {
        self.take_back.borrow_mut().take();
    }

    fn insert(&self, area: &HtmlTextAreaElement, text: &str) {
        self.inserting.set(true);
        insert_at_caret(area, text);
        self.inserting.set(false);
    }

    /// A character about to go in: its capital instead, when the rule says.
    ///
    /// A selection is typed over, as the browser would: the rule reads the
    /// text before the selection's start, which is the Mac's reading too.
    fn before_input(&self, event: &InputEvent) {
        if self.inserting.get() {
            return;
        }
        // Whatever goes in, the capital is no longer the last thing done.
        self.forget();
        if event.is_composing() || event.input_type() != "insertText" || !on_a_computer() {
            return;
        }
        let Some(data) = event.data() else {
            return;
        };
        let mut chars = data.chars();
        let (Some(typed), None) = (chars.next(), chars.next()) else {
            return;
        };
        let Some(area) = event
            .target()
            .and_then(|target| target.dyn_into::<HtmlTextAreaElement>().ok())
        else {
            return;
        };
        let Ok(Some(start)) = area.selection_start() else {
            return;
        };
        let value = area.value();
        let at = byte_offset(&value, start);
        if !composer::capitalizes(&value[..at], typed) {
            return;
        }
        let Some(capital) = composer::capital_of(typed) else {
            return;
        };
        event.prevent_default();
        self.insert(&area, &capital.to_string());
        *self.take_back.borrow_mut() = TakeBack::new(at, typed);
    }

    /// A key: a plain Backspace right behind the capital puts the lower-case
    /// letter back; any other key forgets it. A modifier on its own is not
    /// a key that does anything to the text, so it does neither.
    fn key_down(&self, event: &KeyboardEvent) {
        let key = event.key();
        if matches!(
            key.as_str(),
            "Shift" | "Control" | "Alt" | "Meta" | "CapsLock" | "AltGraph"
        ) {
            return;
        }
        let Some(take_back) = self.take_back.borrow_mut().take() else {
            return;
        };
        if key != "Backspace"
            || event.ctrl_key()
            || event.alt_key()
            || event.meta_key()
            || event.is_composing()
        {
            return;
        }
        let Some(area) = event
            .target()
            .and_then(|target| target.dyn_into::<HtmlTextAreaElement>().ok())
        else {
            return;
        };
        let (Ok(Some(start)), Ok(Some(end))) = (area.selection_start(), area.selection_end())
        else {
            return;
        };
        if start != end {
            return;
        }
        let value = area.value();
        if take_back
            .backspace(&value, byte_offset(&value, start))
            .is_none()
        {
            return;
        }
        event.prevent_default();
        // The capital selected and the letter typed over it: one step the
        // field's Undo knows, the caret right behind the letter after it.
        let from = utf16_offset(&value, take_back.at);
        if area.set_selection_range(from, start).is_ok() {
            self.insert(&area, &take_back.typed.to_string());
        }
    }
}

/// One listener: the event's name and what hears it.
type Listener = (&'static str, Closure<dyn Fn(web_sys::Event)>);

/// The listeners on one field, held so they can be taken off again.
struct Attached {
    field: Element,
    listeners: Vec<Listener>,
}

impl Attached {
    fn detach(self) {
        for (name, listener) in &self.listeners {
            let _ = self
                .field
                .remove_event_listener_with_callback(name, listener.as_ref().unchecked_ref());
        }
    }
}

/// The built-in capitals on the textarea `field` refers to.
///
/// Listeners by hand: Yew 0.21 has no `onbeforeinput`. They go on the
/// element itself, so they run before Yew's delegated `onkeydown` — a
/// Backspace taken back is already default-prevented when the component
/// sees it. Whichever element `field` holds after a render is the one
/// listened to, so a field drawn later (a sheet's) or drawn anew is covered.
#[hook]
pub fn use_capitals(field: NodeRef) {
    let memory = use_memo((), |_| Rc::new(Memory::default()));
    let attached = use_mut_ref(|| Option::<Attached>::None);
    {
        let attached = attached.clone();
        let memory = (*memory).clone();
        use_effect(move || {
            let now = field.cast::<Element>();
            let same = attached.borrow().as_ref().map(|attached| &attached.field) == now.as_ref();
            if !same {
                if let Some(old) = attached.borrow_mut().take() {
                    old.detach();
                }
                if let Some(element) = now {
                    *attached.borrow_mut() = Some(attach(element, memory));
                }
            }
        });
    }
    {
        use_effect_with((), move |_| {
            move || {
                if let Some(old) = attached.borrow_mut().take() {
                    old.detach();
                }
            }
        });
    }
}

fn attach(field: Element, memory: Rc<Memory>) -> Attached {
    let mut listeners: Vec<Listener> = Vec::new();
    {
        let memory = memory.clone();
        listeners.push((
            "beforeinput",
            Closure::new(move |event: web_sys::Event| {
                if let Some(event) = event.dyn_ref::<InputEvent>() {
                    memory.before_input(event);
                }
            }),
        ));
    }
    {
        let memory = memory.clone();
        listeners.push((
            "keydown",
            Closure::new(move |event: web_sys::Event| {
                if let Some(event) = event.dyn_ref::<KeyboardEvent>() {
                    memory.key_down(event);
                }
            }),
        ));
    }
    // A click moves the caret, a paste and leaving the field are other
    // acts: each forgets the capital.
    for name in ["mousedown", "paste", "blur"] {
        let memory = memory.clone();
        listeners.push((name, Closure::new(move |_: web_sys::Event| memory.forget())));
    }
    for (name, listener) in &listeners {
        let _ = field.add_event_listener_with_callback(name, listener.as_ref().unchecked_ref());
    }
    Attached { field, listeners }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    thread_local! {
        /// The answer `on_a_computer` gives under test, when set: the test
        /// browser's own pointer is whatever its machine has.
        pub(crate) static FORCED: Cell<Option<bool>> = const { Cell::new(None) };
    }

    /// Take the device to be a computer, or not — or, given None, to be
    /// whatever the test browser's machine is.
    pub(crate) fn as_computer(computer: Option<bool>) {
        FORCED.with(|forced| forced.set(computer));
    }

    #[wasm_bindgen_test]
    fn offsets_cross_utf16_and_bytes() {
        let text = "Ok 😀. é";
        // "Ok " 3 units, the emoji 2, ". " 2, then é.
        assert_eq!(byte_offset(text, 3), 3);
        assert_eq!(byte_offset(text, 5), 7);
        assert_eq!(byte_offset(text, 7), 9);
        assert_eq!(byte_offset(text, 8), text.len());
        assert_eq!(byte_offset(text, 99), text.len());
        assert_eq!(utf16_offset(text, 7), 5);
        assert_eq!(utf16_offset(text, 9), 7);
    }
}
