//! `TypingBox.cs`: a text field with an undo count, driven the way the window drives
//! `CapitalField` — the same calls in the same order: the key, the change before it lands, the
//! selection the change leaves, the high-priority turn that applies the capital or the emoji, the
//! Backspace — so the wiring the shell will make is held to its sequence here (issues #97, #101).
//!
//! Its text is a Rust `String` and its caret and selection are UTF-16 units, as `CapitalField`
//! counts them (and as the C#'s `TextBox` does); every edit is made on the text's UTF-16 units, as
//! the C# box makes it, and decoded back. Keys are Win32 virtual keys, as in the C#: the box is a
//! port of Windows' sequence (the Linux keysym classifier has tests of its own).

use fc_logic::capital_field::{CapitalField, CapitalPlan};

pub const VK_A: i32 = 0x41;
pub const VK_BACK: i32 = 0x08;
pub const VK_ENTER: i32 = 0x0D;
pub const VK_LEFT: i32 = 0x25;
pub const VK_RIGHT: i32 = 0x27;
pub const VK_V: i32 = 0x56;

/// How many UTF-16 units `text` is.
pub fn units(text: &str) -> usize {
    text.encode_utf16().count()
}

/// `text` with the units `from..to` replaced by `insert`, as a `TextBox` edits its string.
pub fn splice(text: &str, from: usize, to: usize, insert: &str) -> String {
    let old: Vec<u16> = text.encode_utf16().collect();
    let mut new: Vec<u16> = old[..from].to_vec();
    new.extend(insert.encode_utf16());
    new.extend_from_slice(&old[to..]);
    String::from_utf16(&new).expect("an edit that splits no surrogate pair")
}

pub struct TypingBox {
    pub text: String,
    pub start: usize,
    pub length: usize,
    pub undo_units: usize,
    /// The box's `MaxLength`: 0 is none, and over it the box refuses what is typed.
    pub max_length: usize,
    pub field: CapitalField,
    pending: Option<CapitalPlan>,
}

impl TypingBox {
    pub fn new(capitals: bool) -> Self {
        TypingBox {
            text: String::new(),
            start: 0,
            length: 0,
            undo_units: 0,
            max_length: 0,
            field: CapitalField::new(capitals),
            pending: None,
        }
    }

    /// `new TypingBox { Text = …, Start = …, Length = … }`.
    pub fn holding(text: &str, start: usize, length: usize) -> Self {
        let mut typing_box = TypingBox::new(true);
        typing_box.text = text.to_string();
        typing_box.start = start;
        typing_box.length = length;
        typing_box
    }

    /// A letter key typed on a hardware keyboard.
    pub fn type_text(&mut self, typed: &str) {
        self.type_key(typed, VK_A, false, false);
    }

    /// A key typed on a hardware keyboard: the key, then the change it makes, then the high-priority
    /// turn.
    pub fn type_key(&mut self, typed: &str, key: i32, control: bool, alt: bool) {
        self.field.key(CapitalField::classify(key, control, alt));
        if self.max_length > 0 && units(&self.text) - self.length + units(typed) > self.max_length {
            // The box refuses the character: no change at all.
            return;
        }
        let new_text = splice(&self.text, self.start, self.start + self.length, typed);
        let caret = self.start + units(typed);
        self.change(&new_text, caret);
        self.turn();
    }

    /// A key that types nothing in this box (Enter in a single-line one, a dead key): only the key.
    pub fn type_nothing(&mut self, key: i32) {
        self.field.key(CapitalField::classify(key, false, false));
    }

    /// The control's own edit: the change with the old text and selection, then the new text and
    /// caret.
    pub fn change(&mut self, new_text: &str, caret: usize) {
        let plan = self
            .field
            .before_change(&self.text, self.start, self.length, new_text);
        if plan.is_some() {
            self.pending = plan;
        }
        self.text = new_text.to_string();
        self.start = caret;
        self.length = 0;
        self.undo_units += 1;
        // The caret the change leaves moves the selection like any other move — not inside the
        // window's own edit.
        self.field.selection_moved(self.start, self.length);
    }

    /// The high-priority turn after a change: the replacement through the selection — one more
    /// undo unit.
    pub fn turn(&mut self) {
        let Some(plan) = self.pending.take() else {
            return;
        };
        if !self
            .field
            .still_applies(&plan, &self.text, self.start, self.length, self.max_length)
        {
            return;
        }
        self.field.set_applying(true);
        self.text = splice(
            &self.text,
            plan.at,
            plan.at + plan.length,
            &plan.replacement,
        );
        self.start = plan.caret;
        self.undo_units += 1;
        self.field.selection_moved(self.start, self.length);
        self.field.set_applying(false);
        self.field.applied(&plan);
    }

    /// Backspace: the take-back, or the control's own deletion of one character (a surrogate pair
    /// as a whole).
    pub fn backspace(&mut self) {
        if let Some(restore) = self.field.backspace(&self.text, self.start, self.length) {
            self.field.set_applying(true);
            self.text = splice(
                &self.text,
                restore.at,
                restore.at + restore.length,
                &restore.replacement,
            );
            self.start = restore.caret;
            self.field.selection_moved(self.start, self.length);
            self.field.set_applying(false);
            return;
        }
        self.field
            .key(CapitalField::classify(VK_BACK, false, false));
        if self.length > 0 {
            let new_text = splice(&self.text, self.start, self.start + self.length, "");
            self.change(&new_text, self.start);
        } else if self.start > 0 {
            let before: Vec<u16> = self.text.encode_utf16().take(self.start).collect();
            let pair = before.len() >= 2
                && (0xD800..0xDC00).contains(&before[before.len() - 2])
                && (0xDC00..0xE000).contains(&before[before.len() - 1]);
            let one = if pair { 2 } else { 1 };
            let new_text = splice(&self.text, self.start - one, self.start, "");
            self.change(&new_text, self.start - one);
        }
    }

    /// A click that puts the caret at `caret`, selecting `length`.
    pub fn click(&mut self, caret: usize, length: usize) {
        self.start = caret;
        self.length = length;
        self.field.selection_moved(self.start, self.length);
    }

    /// Ctrl+V: the chord, the paste event, then the change.
    pub fn paste(&mut self, pasted: &str) {
        self.field.key(CapitalField::classify(VK_V, true, false));
        self.field.pasting();
        let new_text = splice(&self.text, self.start, self.start + self.length, pasted);
        let caret = self.start + units(pasted);
        self.change(&new_text, caret);
        self.turn();
    }

    /// Every character of `keys` typed, with `⌫` a Backspace, `→` an arrow key, and `\r` or `\n`
    /// Enter — the vectors' alphabet, through the window's sequence.
    pub fn type_all(&mut self, keys: &str) {
        for key in keys.chars() {
            match key {
                '⌫' => self.backspace(),
                // An arrow that, at the end of the text, moves nothing: the key alone is
                // "something else happened".
                '→' => self
                    .field
                    .key(CapitalField::classify(VK_RIGHT, false, false)),
                '\r' | '\n' => self.type_key(&key.to_string(), VK_ENTER, false, false),
                _ => self.type_text(&key.to_string()),
            }
        }
    }
}
