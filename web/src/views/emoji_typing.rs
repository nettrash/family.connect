//! Emoji as you type (issue #101; docs/emoji-as-you-type-2026-10-18.md):
//! `:)` becomes 😊 and `:fire:` 🔥 the moment the last character goes in,
//! and a Backspace straight after puts back what was typed. The rule is
//! `fc_text::emoji_typing`, the one every client is held to; this is where
//! the web's prose fields meet it.
//!
//! It rides on #97's capitals' listeners (`capitals::use_typing_rules`), on
//! every prose field — the composer (chat, thread and edit), a board note,
//! caption or title, a list's line, a poll's question and options, a
//! report's note — and on every device, phones included. Only a
//! `beforeinput` of `insertText` with exactly one character, not composing,
//! is looked at: never a paste, a drop, a suggestion, an autocorrection or a
//! keyboard's `insertCompositionText`.
//!
//! The replacement goes in the way the capital does: what it replaces is
//! selected and the emoji typed over it with `insertText`, so the field's
//! Undo knows it and the `input` the component listens to keeps Yew's value
//! — the draft, the mentions, Send, a cap on the length — in step.
//!
//! `fc_text` counts BYTES; a browser's selection counts UTF-16 units. They
//! are converted here, at the edge.

use std::cell::RefCell;

use fc_text::emoji_typing::{self, Swap};
use web_sys::{Element, KeyboardEvent};
use yew::prelude::*;

use crate::views::capitals::{self, byte_offset, utf16_offset};

/// Emoji as you type on the element `field` refers to, and on every prose
/// field inside it — without #97's capitals, which only the multi-line
/// fields get (they have `capitals::use_capitals`, which brings emoji too).
#[hook]
pub fn use_emoji_typing(field: NodeRef) {
    capitals::use_typing_rules(field, false);
}

/// The swap a field may still grow or take back, and the field it is in:
/// one at a time, forgotten as soon as anything else happens.
#[derive(Default)]
pub(crate) struct Live(RefCell<Option<(Element, Swap)>>);

impl Live {
    pub(crate) fn forget(&self) {
        self.0.borrow_mut().take();
    }

    /// The swap, if it was made in `field`; forgotten either way.
    pub(crate) fn take(&self, field: &Element) -> Option<Swap> {
        self.0
            .borrow_mut()
            .take()
            .filter(|(made_in, _)| made_in == field)
            .map(|(_, swap)| swap)
    }

    pub(crate) fn keep(&self, field: &Element, swap: Swap) {
        *self.0.borrow_mut() = Some((field.clone(), swap));
    }
}

/// What typing one character makes of a field.
#[derive(Debug, PartialEq)]
pub(crate) struct Edit {
    /// Where, in UTF-16 units, the replaced text starts: from here to the
    /// selection's end is typed over with `swap.put`.
    pub(crate) from: u32,
    pub(crate) swap: Swap,
}

/// Typing `typed` into a field holding `value`, the selection from `start`
/// to `end` (UTF-16 units, equal for a caret): the emoji it makes, or `None`
/// when the character goes in as it is. A selection is typed over: the rule
/// reads what is before its start.
///
/// A field with a `maxlength` (`max`, in UTF-16 units) never gets a swap
/// that would take it past it: the browser would cut the emoji short — a
/// flag's tag characters, a family's joiners — so the character goes in as
/// typed instead, as on Windows.
pub(crate) fn typed(
    value: &str,
    start: u32,
    end: u32,
    typed: char,
    live: Option<&Swap>,
    max: Option<u32>,
) -> Option<Edit> {
    let (start, end) = (start.min(end), start.max(end));
    let at = byte_offset(value, start);
    let before = &value[..at];
    let swap = emoji_typing::replace(before, typed, live)?;
    let from = utf16_offset(before, swap.at);
    let length = units(value).saturating_sub(end - from) + units(&swap.put);
    if max.is_some_and(|max| length > max) {
        return None;
    }
    Some(Edit { from, swap })
}

/// A Backspace with the caret at `caret` (UTF-16 units, nothing selected):
/// where, in UTF-16 units, the replacement starts when the caret is right
/// behind it and it is still there — select from there to the caret and
/// type `swap.typed` over it — or `None`, and the Backspace is an ordinary
/// one. So it is when putting the typed text back would take the field past
/// its `maxlength` (`max`, UTF-16 units).
pub(crate) fn taken_back(value: &str, caret: u32, swap: &Swap, max: Option<u32>) -> Option<u32> {
    let (undone, _) = swap.backspace(value, byte_offset(value, caret))?;
    if max.is_some_and(|max| units(&undone) > max) {
        return None;
    }
    Some(utf16_offset(value, swap.at))
}

/// `text`'s length in UTF-16 units, as `maxlength` counts it.
fn units(text: &str) -> u32 {
    text.encode_utf16().count() as u32
}

/// Whether a key forgets the swap: one that moves the caret or acts on the
/// text without typing a character — an arrow, Home, Delete, Enter, Tab,
/// Escape — or any Ctrl or Cmd chord. A typed character and a Backspace
/// come to the swap through `beforeinput` instead, and a modifier on its own
/// does nothing. So does a phone's keyboard's "Unidentified" (Android sends
/// it for every key), a "Process" (an IME) and a dead key.
pub(crate) fn drops_on_key(event: &KeyboardEvent) -> bool {
    key_drops(
        &event.key(),
        event.ctrl_key() || event.meta_key(),
        event.is_composing(),
    )
}

fn key_drops(key: &str, chord: bool, composing: bool) -> bool {
    if composing
        || matches!(
            key,
            "Shift" | "Control" | "Alt" | "Meta" | "CapsLock" | "AltGraph" | "Fn"
        )
    {
        return false;
    }
    if chord {
        return true;
    }
    key.chars().count() > 1 && !matches!(key, "Backspace" | "Unidentified" | "Process" | "Dead")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::views::capitals::tests::as_computer;
    use std::rc::Rc;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;
    use web_sys::{HtmlInputElement, HtmlTextAreaElement};

    #[wasm_bindgen_test]
    fn the_edit_is_in_utf16_units() {
        // A Cyrillic word: two bytes a letter, one unit.
        let edit = typed("Привет :", 8, 8, ')', None, None).unwrap();
        assert_eq!(edit.from, 7);
        assert_eq!(edit.swap.put, "😊");
        // An emoji before the caret: four bytes, two units.
        let edit = typed("😀 :fire", 8, 8, ':', None, None).unwrap();
        assert_eq!(edit.from, 3);
        assert_eq!(edit.swap.put, "🔥");
        // Typed over a selection: what is before its start is read.
        let edit = typed("ok :xyz", 4, 7, ')', None, None).unwrap();
        assert_eq!(edit.from, 3);
        assert_eq!(typed("ok :xyz", 7, 7, ')', None, None), None);
        // The take-back, behind an emoji and Cyrillic.
        let swap = typed("😀 Ж :", 6, 6, ')', None, None).unwrap().swap;
        let (text, _) = swap.apply("😀 Ж :", "");
        assert_eq!(text, "😀 Ж 😊");
        assert_eq!(taken_back(&text, 7, &swap, None), Some(5));
        assert_eq!(taken_back(&text, 5, &swap, None), None);
        // A maxlength the swap would pass: no swap. `:wales:` is 7 units,
        // its flag 14.
        assert_eq!(typed(":wales", 6, 6, ':', None, Some(7)), None);
        assert!(typed(":wales", 6, 6, ':', None, Some(14)).is_some());
        assert!(typed(":", 1, 1, ')', None, Some(2)).is_some());
        // The selection typed over counts as gone.
        assert!(typed("(:walesxxxxxxxxx", 7, 16, ':', None, Some(15)).is_some());
        assert_eq!(typed("(:walesxxxxxxxxx", 7, 16, ':', None, Some(14)), None);
        // A take-back the maxlength has no room for is an ordinary Backspace.
        let swap = typed(":fire", 5, 5, ':', None, None).unwrap().swap;
        assert_eq!(taken_back("🔥", 2, &swap, Some(5)), None);
        assert_eq!(taken_back("🔥", 2, &swap, Some(6)), Some(0));
    }

    #[wasm_bindgen_test]
    fn which_keys_forget_the_swap() {
        for key in [
            "ArrowLeft",
            "ArrowRight",
            "Home",
            "End",
            "Delete",
            "Enter",
            "Tab",
        ] {
            assert!(key_drops(key, false, false), "{key}");
        }
        for key in [
            "(",
            ":",
            "a",
            "Backspace",
            "Shift",
            "Unidentified",
            "Process",
            "Dead",
        ] {
            assert!(!key_drops(key, false, false), "{key}");
        }
        assert!(key_drops("a", true, false), "a chord");
        assert!(!key_drops("Meta", true, false), "the modifier alone");
        assert!(!key_drops("ArrowLeft", false, true), "composing");
    }

    #[derive(Properties, PartialEq)]
    struct FieldsProps {
        heard: Rc<RefCell<Vec<String>>>,
    }

    /// A multi-line prose field with the capitals' hook and two one-line
    /// ones under a container's emoji hook — plus a field that is not prose
    /// — each telling what its `oninput` heard, the way a component's state
    /// would.
    #[function_component(Fields)]
    fn fields(props: &FieldsProps) -> Html {
        let area = use_node_ref();
        let lines = use_node_ref();
        capitals::use_capitals(area.clone());
        use_emoji_typing(lines.clone());
        let heard = props.heard.clone();
        let oninput = Callback::from(move |event: InputEvent| {
            let target = event.target().unwrap();
            let value = match target.dyn_ref::<HtmlTextAreaElement>() {
                Some(area) => area.value(),
                None => target.unchecked_into::<HtmlInputElement>().value(),
            };
            heard.borrow_mut().push(value);
        });
        html! {
            <div>
                <textarea ref={area} autocapitalize="sentences" oninput={oninput.clone()} />
                <div ref={lines}>
                    <input class="one" type="text" autocapitalize="sentences" oninput={oninput.clone()} />
                    <input class="two" autocapitalize="sentences" oninput={oninput.clone()} />
                    <input class="plain" type="text" oninput={oninput} />
                </div>
            </div>
        }
    }

    struct Mounted {
        root: Element,
        handle: yew::AppHandle<Fields>,
        heard: Rc<RefCell<Vec<String>>>,
    }

    impl Mounted {
        async fn new() -> Mounted {
            let document = web_sys::window().unwrap().document().unwrap();
            let root = document.create_element("div").unwrap();
            document.body().unwrap().append_child(&root).unwrap();
            let heard = Rc::new(RefCell::new(Vec::new()));
            let handle = yew::Renderer::<Fields>::with_root_and_props(
                root.clone(),
                FieldsProps {
                    heard: heard.clone(),
                },
            )
            .render();
            settle().await;
            Mounted {
                root,
                handle,
                heard,
            }
        }

        fn field(&self, selector: &str) -> web_sys::HtmlElement {
            self.root
                .query_selector(selector)
                .unwrap()
                .unwrap()
                .unchecked_into()
        }

        fn last_heard(&self) -> Option<String> {
            self.heard.borrow().last().cloned()
        }

        fn gone(self) {
            self.handle.destroy();
            self.root.remove();
        }
    }

    async fn settle() {
        gloo_timers::future::TimeoutFuture::new(20).await;
    }

    /// The field as one value and caret, a textarea or an input alike.
    pub(crate) fn state(field: &web_sys::HtmlElement) -> (String, u32, u32) {
        if let Some(area) = field.dyn_ref::<HtmlTextAreaElement>() {
            (
                area.value(),
                area.selection_start().unwrap().unwrap(),
                area.selection_end().unwrap().unwrap(),
            )
        } else {
            let line: &HtmlInputElement = field.unchecked_ref();
            (
                line.value(),
                line.selection_start().unwrap().unwrap(),
                line.selection_end().unwrap().unwrap(),
            )
        }
    }

    pub(crate) fn set(field: &web_sys::HtmlElement, value: &str) {
        if let Some(area) = field.dyn_ref::<HtmlTextAreaElement>() {
            area.set_value(value);
        } else {
            field.unchecked_ref::<HtmlInputElement>().set_value(value);
        }
    }

    pub(crate) fn select(field: &web_sys::HtmlElement, start: u32, end: u32) {
        if let Some(area) = field.dyn_ref::<HtmlTextAreaElement>() {
            area.set_selection_range(start, end).unwrap();
        } else {
            field
                .unchecked_ref::<HtmlInputElement>()
                .set_selection_range(start, end)
                .unwrap();
        }
    }

    /// A `beforeinput`, as a keyboard sends it. A synthetic one puts nothing
    /// in by itself, so when the hook lets it be, the browser's own part —
    /// the character at the caret, and its `input` — is done here by hand.
    pub(crate) fn before_input(
        field: &web_sys::HtmlElement,
        input_type: &str,
        data: Option<&str>,
    ) -> web_sys::InputEvent {
        let init = web_sys::InputEventInit::new();
        init.set_input_type(input_type);
        init.set_data(data);
        init.set_bubbles(true);
        init.set_cancelable(true);
        let event = web_sys::InputEvent::new_with_event_init_dict("beforeinput", &init).unwrap();
        field.dispatch_event(&event).unwrap();
        event
    }

    /// Types `keys` one at a time, as a keyboard would: `⌫` a Backspace,
    /// everything else a character, at the caret. Where the hook leaves a
    /// key alone, the browser's default is played by hand.
    pub(crate) fn type_keys(field: &web_sys::HtmlElement, keys: &str) {
        for key in keys.chars() {
            let (value, start, end) = state(field);
            if key == '⌫' {
                let event = before_input(field, "deleteContentBackward", None);
                if !event.default_prevented() {
                    let from = if start == end {
                        let at = byte_offset(&value, start);
                        let previous = value[..at].chars().next_back().map_or(0, char::len_utf16);
                        start - previous as u32
                    } else {
                        start
                    };
                    browser_puts(field, &value, from, end, "");
                }
            } else {
                let text = key.to_string();
                let event = before_input(field, "insertText", Some(&text));
                if !event.default_prevented() {
                    browser_puts(field, &value, start, end, &text);
                }
            }
        }
    }

    /// What a browser does with a key nobody stopped: `text` over the
    /// range, the caret after it, and an `input`.
    pub(crate) fn browser_puts(
        field: &web_sys::HtmlElement,
        value: &str,
        from: u32,
        to: u32,
        text: &str,
    ) {
        let units: Vec<u16> = value.encode_utf16().collect();
        let mut next = String::from_utf16(&units[..from as usize]).unwrap();
        next.push_str(text);
        let caret = next.encode_utf16().count() as u32;
        next.push_str(&String::from_utf16(&units[to as usize..]).unwrap());
        set(field, &next);
        select(field, caret, caret);
        let init = web_sys::EventInit::new();
        init.set_bubbles(true);
        field
            .dispatch_event(&web_sys::Event::new_with_event_init_dict("input", &init).unwrap())
            .unwrap();
    }

    /// #101 on a real textarea and a real input: the emoji goes in, the
    /// caret right behind it, the component's `oninput` hears the new
    /// value, and a Backspace straight after puts back what was typed.
    #[wasm_bindgen_test]
    async fn faces_and_shortcodes_become_emoji_and_backspace_takes_them_back() {
        as_computer(Some(false));
        let mounted = Mounted::new().await;
        for selector in ["textarea", "input.one", "input.two"] {
            let field = mounted.field(selector);
            field.focus().unwrap();
            for (keys, shown) in [
                ("hi :)", "hi 😊"),
                (":fire:", "🔥"),
                ("so :fire: today", "so 🔥 today"),
                ("Привет :)", "Привет 😊"),
                ("😀 :)", "😀 😊"),
                ("👍🏽 x :fire:", "👍🏽 x 🔥"),
                (":((", "😩"),
                (">((", "😡"),
                (":o ", "😨 "),
                ("x<3", "x<3"),
                ("`:)", "`:)"),
                (":)⌫", ":)"),
                (":fire:⌫", ":fire:"),
                (":((⌫", ":(("),
                ("Привет :)⌫", "Привет :)"),
                ("😀 :fire:⌫", "😀 :fire:"),
                (":)⌫⌫", ":"),
            ] {
                set(&field, "");
                select(&field, 0, 0);
                type_keys(&field, keys);
                settle().await;
                let (value, start, end) = state(&field);
                assert_eq!(value, shown, "{selector}: {keys:?}");
                assert_eq!(
                    (start, end),
                    (units(shown), units(shown)),
                    "{selector}: {keys:?}'s caret"
                );
                assert_eq!(
                    mounted.last_heard().as_deref(),
                    Some(shown),
                    "{selector}: {keys:?} heard"
                );
            }
        }
        as_computer(None);
        mounted.gone();
    }

    /// In the middle of a line: the text after the caret stays where it
    /// is, the caret lands right behind the emoji, and its take-back
    /// leaves the caret right behind what was typed.
    #[wasm_bindgen_test]
    async fn the_text_after_the_caret_stays() {
        as_computer(Some(false));
        let mounted = Mounted::new().await;
        for selector in ["textarea", "input.one"] {
            let field = mounted.field(selector);
            field.focus().unwrap();
            set(&field, "Ж 😀 : конец");
            select(&field, 6, 6);
            type_keys(&field, ")");
            settle().await;
            assert_eq!(state(&field), ("Ж 😀 😊 конец".into(), 7, 7), "{selector}");
            type_keys(&field, "⌫");
            settle().await;
            assert_eq!(state(&field), ("Ж 😀 :) конец".into(), 7, 7), "{selector}");
            assert_eq!(mounted.last_heard().as_deref(), Some("Ж 😀 :) конец"));
            // A selection typed over: the rule reads what is before it.
            set(&field, "ok :xyz bye");
            select(&field, 4, 7);
            type_keys(&field, ")");
            settle().await;
            assert_eq!(state(&field), ("ok 😊 bye".into(), 5, 5), "{selector}");
        }
        as_computer(None);
        mounted.gone();
    }

    /// Anything else in between — a caret move, a click, a paste, leaving
    /// the field — and the next key neither grows the swap nor takes it
    /// back. A swap made in one field is not another's.
    #[wasm_bindgen_test]
    async fn anything_else_forgets_the_swap() {
        as_computer(Some(false));
        let mounted = Mounted::new().await;
        let field = mounted.field("input.one");
        let other = mounted.field("input.two");
        field.focus().unwrap();
        let key = |name: &str| {
            let init = web_sys::KeyboardEventInit::new();
            init.set_key(name);
            init.set_bubbles(true);
            init.set_cancelable(true);
            let event = web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init)
                .unwrap();
            field.dispatch_event(&event).unwrap();
        };
        let plain = |name: &str| {
            let init = web_sys::EventInit::new();
            init.set_bubbles(true);
            field
                .dispatch_event(&web_sys::Event::new_with_event_init_dict(name, &init).unwrap())
                .unwrap();
        };
        for (between, name) in [
            (0, "ArrowLeft"),
            (1, "mousedown"),
            (1, "pointerdown"),
            (1, "paste"),
            (1, "focusout"),
        ] {
            set(&field, "");
            select(&field, 0, 0);
            type_keys(&field, ":(");
            if between == 0 {
                key(name);
            } else {
                plain(name);
            }
            type_keys(&field, "(");
            assert_eq!(state(&field).0, "😞(", "{name}: no growing");
            set(&field, "");
            select(&field, 0, 0);
            type_keys(&field, ":)");
            if between == 0 {
                key(name);
            } else {
                plain(name);
            }
            type_keys(&field, "⌫");
            assert_eq!(state(&field).0, "", "{name}: an ordinary Backspace");
        }
        // Typing on does not forget it: its keydown is not "something else".
        set(&field, "");
        select(&field, 0, 0);
        type_keys(&field, ":(");
        key("(");
        type_keys(&field, "(");
        assert_eq!(state(&field).0, "😩");
        // A swap in one field, a Backspace in another: ordinary there, and
        // the swap is gone from the first.
        set(&other, "x");
        select(&other, 1, 1);
        set(&field, "");
        select(&field, 0, 0);
        type_keys(&field, ":)");
        type_keys(&other, "⌫");
        assert_eq!(state(&other).0, "");
        type_keys(&field, "⌫");
        assert_eq!(state(&field).0, "", "the swap went");
        as_computer(None);
        mounted.gone();
    }

    /// A field's `maxlength`: a swap that would pass it is not made — the
    /// browser would cut the emoji short — and the character goes in as
    /// typed; one within it is made as ever. `:wales:` is 7 units, its flag
    /// 14.
    #[wasm_bindgen_test]
    async fn a_maxlength_is_never_passed_by_an_emoji() {
        as_computer(Some(false));
        let mounted = Mounted::new().await;
        let field = mounted.field("input.one");
        field.set_attribute("maxlength", "7").unwrap();
        field.focus().unwrap();
        set(&field, "");
        select(&field, 0, 0);
        type_keys(&field, ":wales:");
        settle().await;
        assert_eq!(state(&field), (":wales:".into(), 7, 7));
        assert_eq!(mounted.last_heard().as_deref(), Some(":wales:"));
        // And no swap was kept: the Backspace after it is an ordinary one.
        type_keys(&field, "⌫");
        assert_eq!(state(&field).0, ":wales");
        // Within the limit, as ever.
        set(&field, "");
        select(&field, 0, 0);
        type_keys(&field, "ok :)");
        settle().await;
        assert_eq!(state(&field), ("ok 😊".into(), 5, 5));
        // A take-back with no room: a value set from outside past the
        // limit (a field's maxlength binds typing, not the page), and the
        // Backspace is the browser's.
        field.set_attribute("maxlength", "4").unwrap();
        set(&field, ":fire");
        select(&field, 5, 5);
        type_keys(&field, ":");
        assert_eq!(state(&field).0, "🔥");
        let back = before_input(&field, "deleteContentBackward", None);
        assert!(!back.default_prevented(), "no room to put :fire: back");
        assert_eq!(state(&field).0, "🔥");
        field.remove_attribute("maxlength").unwrap();
        as_computer(None);
        mounted.gone();
    }

    /// Only one typed character counts: a paste, a keyboard's composition,
    /// a word put in at once, and a field that is not prose are left alone.
    #[wasm_bindgen_test]
    async fn a_paste_a_composition_and_a_plain_field_are_untouched() {
        as_computer(Some(false));
        let mounted = Mounted::new().await;
        let field = mounted.field("textarea");
        field.focus().unwrap();
        set(&field, "hi :");
        select(&field, 4, 4);
        for (input_type, data) in [
            ("insertFromPaste", ")"),
            ("insertCompositionText", ")"),
            ("insertReplacementText", ")"),
            ("insertText", "))"),
            ("insertText", ") "),
        ] {
            let event = before_input(&field, input_type, Some(data));
            assert!(!event.default_prevented(), "{input_type} {data:?}");
            assert_eq!(
                state(&field),
                ("hi :".into(), 4, 4),
                "{input_type} {data:?}"
            );
        }
        // A composing `insertText` too.
        let init = web_sys::InputEventInit::new();
        init.set_input_type("insertText");
        init.set_data(Some(")"));
        init.set_is_composing(true);
        init.set_bubbles(true);
        init.set_cancelable(true);
        let event = web_sys::InputEvent::new_with_event_init_dict("beforeinput", &init).unwrap();
        field.dispatch_event(&event).unwrap();
        assert!(!event.default_prevented(), "composing");
        // A field without `autocapitalize="sentences"` is not prose.
        let plain = mounted.field("input.plain");
        plain.focus().unwrap();
        type_keys(&plain, "hi :) :fire:");
        assert_eq!(state(&plain).0, "hi :) :fire:");
        as_computer(None);
        mounted.gone();
    }

    /// On a computer the capitals run first, in the multi-line field: the
    /// character the emoji rule sees is the one they decided — `x` at a
    /// sentence's start is `X`, so `xD ` is the `XD ` face — and in a
    /// one-line field there are no capitals, only emoji.
    #[wasm_bindgen_test]
    async fn the_capitals_decide_first() {
        as_computer(Some(true));
        let mounted = Mounted::new().await;
        let area = mounted.field("textarea");
        area.focus().unwrap();
        set(&area, "");
        select(&area, 0, 0);
        type_keys(&area, "hi :)");
        settle().await;
        assert_eq!(state(&area).0, "Hi 😊");
        set(&area, "");
        select(&area, 0, 0);
        type_keys(&area, "xD ");
        settle().await;
        assert_eq!(state(&area).0, "😆 ");
        assert_eq!(mounted.last_heard().as_deref(), Some("😆 "));
        type_keys(&area, "⌫");
        assert_eq!(state(&area).0, "XD ");
        let line = mounted.field("input.one");
        line.focus().unwrap();
        set(&line, "");
        select(&line, 0, 0);
        type_keys(&line, "hi :)");
        assert_eq!(state(&line).0, "hi 😊");
        as_computer(None);
        mounted.gone();
    }
}
