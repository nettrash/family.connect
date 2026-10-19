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
//!
//! The same listeners carry #101's emoji as you type (`emoji_typing`), on
//! every prose field and every device, phones included: the character the
//! emoji rule sees is the one the capitals decided. A prose field is one that
//! says `autocapitalize="sentences"` — #97's own mark — and the listeners may
//! sit on the field itself or on something holding several (a poll's
//! question and options, a list's lines).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use fc_text::composer::{self, TakeBack};
use fc_text::emoji_typing::Swap;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{
    Element, EventTarget, HtmlInputElement, HtmlTextAreaElement, InputEvent, KeyboardEvent,
};
use yew::prelude::*;

use crate::views::emoji_typing::{self, Live};

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

/// A prose field: a `<textarea>`, or a one-line `<input type="text">`, that
/// says `autocapitalize="sentences"` — the mark #97 put on every field people
/// write sentences in, and on no other.
#[derive(Clone)]
pub(crate) enum TextField {
    Area(HtmlTextAreaElement),
    Line(HtmlInputElement),
}

impl TextField {
    /// The prose field an event came from, if it came from one.
    pub(crate) fn prose(target: Option<EventTarget>) -> Option<TextField> {
        let element = target?.dyn_into::<Element>().ok()?;
        if element.get_attribute("autocapitalize").as_deref() != Some("sentences") {
            return None;
        }
        match element.dyn_into::<HtmlTextAreaElement>() {
            Ok(area) => Some(TextField::Area(area)),
            Err(element) => element
                .dyn_into::<HtmlInputElement>()
                .ok()
                .filter(|line| line.type_() == "text")
                .map(TextField::Line),
        }
    }

    pub(crate) fn element(&self) -> &Element {
        match self {
            TextField::Area(area) => area,
            TextField::Line(line) => line,
        }
    }

    pub(crate) fn value(&self) -> String {
        match self {
            TextField::Area(area) => area.value(),
            TextField::Line(line) => line.value(),
        }
    }

    /// The selection, in UTF-16 units; start and end are the caret when
    /// nothing is selected.
    pub(crate) fn selection(&self) -> Option<(u32, u32)> {
        let (start, end) = match self {
            TextField::Area(area) => (area.selection_start(), area.selection_end()),
            TextField::Line(line) => (line.selection_start(), line.selection_end()),
        };
        match (start, end) {
            (Ok(Some(start)), Ok(Some(end))) => Some((start, end)),
            _ => None,
        }
    }

    /// The field's `maxlength`, in UTF-16 units, if it has one.
    pub(crate) fn max_length(&self) -> Option<u32> {
        let max = match self {
            TextField::Area(area) => area.max_length(),
            TextField::Line(line) => line.max_length(),
        };
        u32::try_from(max).ok()
    }

    pub(crate) fn select(&self, start: u32, end: u32) -> bool {
        match self {
            TextField::Area(area) => area.set_selection_range(start, end),
            TextField::Line(line) => line.set_selection_range(start, end),
        }
        .is_ok()
    }

    /// `text` over `start..end`, the caret right behind it (`setRangeText`
    /// with its "end" mode, which web-sys gives the input only behind a
    /// feature this crate does not otherwise need).
    fn set_range_text(&self, text: &str, start: u32, end: u32) -> bool {
        let replaced = match self {
            TextField::Area(area) => area.set_range_text_with_start_and_end(text, start, end),
            TextField::Line(line) => line.set_range_text_with_start_and_end(text, start, end),
        };
        let caret = start + text.encode_utf16().count() as u32;
        replaced.is_ok() && self.select(caret, caret)
    }

    /// `text` at the caret, over the selection, the way #95's new line goes
    /// in: `insertText` first, which the field's Undo knows and which fires
    /// the `input` the component listens to; where a browser refuses it, the
    /// range is replaced and that `input` sent by hand.
    pub(crate) fn insert(&self, text: &str) {
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
        let Some((start, end)) = self.selection() else {
            return;
        };
        if !self.set_range_text(text, start, end) {
            return;
        }
        let init = web_sys::EventInit::new();
        init.set_bubbles(true);
        if let Ok(input) = web_sys::Event::new_with_event_init_dict("input", &init) {
            let _ = self.element().dispatch_event(&input);
        }
    }
}

/// `text` at the caret of `area`, over the selection (`TextField::insert`).
pub fn insert_at_caret(area: &HtmlTextAreaElement, text: &str) {
    TextField::Area(area.clone()).insert(text);
}

/// The byte offset in `text` of a browser's UTF-16 offset (`selectionStart`
/// counts UTF-16 units; `fc_text` counts bytes). One inside a surrogate pair
/// lands after the character.
pub(crate) fn byte_offset(text: &str, utf16: u32) -> usize {
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
pub(crate) fn utf16_offset(text: &str, byte: usize) -> u32 {
    text[..byte].encode_utf16().count() as u32
}

/// The memory of the fields one hook listens to: the capital a Backspace
/// may still take back, the emoji swap likewise, and whether the text going
/// in now is this module's own.
struct Memory {
    /// Whether the capitals run here: the multi-line fields only (#97).
    capitals: bool,
    take_back: RefCell<Option<TakeBack>>,
    /// The emoji swap just made, and the field it was made in (#101).
    emoji: Live,
    /// Set while this module inserts: its own `insertText` fires a
    /// `beforeinput` too, and the lower-case letter a take-back puts in
    /// would otherwise be capitalised straight back.
    inserting: Cell<bool>,
}

impl Memory {
    fn new(capitals: bool) -> Memory {
        Memory {
            capitals,
            take_back: RefCell::default(),
            emoji: Live::default(),
            inserting: Cell::new(false),
        }
    }

    fn forget(&self) {
        self.take_back.borrow_mut().take();
        self.emoji.forget();
    }

    fn insert(&self, field: &TextField, text: &str) {
        self.inserting.set(true);
        field.insert(text);
        self.inserting.set(false);
    }

    /// Whatever is about to go in: a typed character, or a Backspace.
    /// Anything else forgets both the capital and the emoji swap.
    fn before_input(&self, event: &InputEvent) {
        if self.inserting.get() {
            return;
        }
        // Whatever goes in, the capital is no longer the last thing done.
        self.take_back.borrow_mut().take();
        let Some(field) = TextField::prose(event.target()) else {
            self.emoji.forget();
            return;
        };
        // Nor is the emoji swap — unless this is the key that grows it or
        // the Backspace that takes it back, which get it here.
        let live = self.emoji.take(field.element());
        // A keyboard still composing (an IME, or a phone's keyboard
        // building a word with `insertCompositionText`) is left alone.
        if event.is_composing() {
            return;
        }
        match event.input_type().as_str() {
            "insertText" => self.typed(event, &field, live.as_ref()),
            "deleteContentBackward" => {
                if let Some(swap) = live {
                    self.backspace(event, &field, &swap);
                }
            }
            _ => {}
        }
    }

    /// One character typed: its capital, where the capitals run and the
    /// rule says (#97), and then whatever emoji it makes (#101).
    ///
    /// A selection is typed over, as the browser would: the rules read the
    /// text before the selection's start, which is the Mac's reading too.
    fn typed(&self, event: &InputEvent, field: &TextField, live: Option<&Swap>) {
        let Some(data) = event.data() else {
            return;
        };
        let mut chars = data.chars();
        let (Some(typed), None) = (chars.next(), chars.next()) else {
            return;
        };
        let Some((start, end)) = field.selection() else {
            return;
        };
        let value = field.value();
        let at = byte_offset(&value, start);
        let capital = if self.capitals
            && matches!(field, TextField::Area(_))
            && on_a_computer()
            && composer::capitalizes(&value[..at], typed)
        {
            composer::capital_of(typed)
        } else {
            None
        };
        let decided = capital.unwrap_or(typed);
        if let Some(edit) =
            emoji_typing::typed(&value, start, end, decided, live, field.max_length())
        {
            event.prevent_default();
            // What the emoji replaces, the selection with it, selected and
            // typed over: one step the field's Undo knows.
            if field.select(edit.from, end) {
                self.insert(field, &edit.swap.put);
                self.emoji.keep(field.element(), edit.swap);
            } else {
                self.insert(field, &decided.to_string());
            }
            return;
        }
        let Some(capital) = capital else {
            return;
        };
        event.prevent_default();
        self.insert(field, &capital.to_string());
        *self.take_back.borrow_mut() = TakeBack::new(at, typed);
    }

    /// A Backspace right behind the emoji swap, nothing selected: what was
    /// typed goes back in its place.
    fn backspace(&self, event: &InputEvent, field: &TextField, swap: &Swap) {
        let Some((start, end)) = field.selection() else {
            return;
        };
        if start != end {
            return;
        }
        let value = field.value();
        let Some(from) = emoji_typing::taken_back(&value, start, swap, field.max_length()) else {
            return;
        };
        event.prevent_default();
        if field.select(from, start) {
            self.insert(field, &swap.typed);
        }
    }

    /// A key: a plain Backspace right behind the capital puts the lower-case
    /// letter back; any other key forgets it. A modifier on its own is not
    /// a key that does anything to the text, so it does neither. The emoji
    /// swap is forgotten by a key that moves the caret or acts on the text
    /// without typing (`emoji_typing::drops_on_key`); a typed character or a
    /// Backspace reaches it in `beforeinput`.
    fn key_down(&self, event: &KeyboardEvent) {
        if emoji_typing::drops_on_key(event) {
            self.emoji.forget();
        }
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
        let Some(field) = TextField::prose(event.target()) else {
            return;
        };
        let Some((start, end)) = field.selection() else {
            return;
        };
        if start != end {
            return;
        }
        let value = field.value();
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
        if field.select(from, start) {
            self.insert(&field, &take_back.typed.to_string());
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

/// The built-in capitals, and emoji as you type, on the multi-line prose
/// field `field` refers to — or on every prose field inside it.
#[hook]
pub fn use_capitals(field: NodeRef) {
    use_typing_rules(field, true);
}

/// The typing rules on the element `field` refers to, and every prose field
/// inside it: emoji as you type always (#101), the capitals (#97) when
/// `capitals` says — and then only in a `<textarea>`, on a computer.
///
/// Listeners by hand: Yew 0.21 has no `onbeforeinput`. Whichever element
/// `field` holds after a render is the one listened to, so a field drawn
/// later (a sheet's) or drawn anew is covered.
#[hook]
pub(crate) fn use_typing_rules(field: NodeRef, capitals: bool) {
    let memory = use_memo((), move |_| Rc::new(Memory::new(capitals)));
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
    // A click or a tap moves the caret, a paste and leaving the field are
    // other acts: each forgets the capital and the emoji swap. (`focusout`
    // rather than `blur`: it bubbles, to a hook on a field's container.)
    for name in ["mousedown", "pointerdown", "paste", "focusout"] {
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
