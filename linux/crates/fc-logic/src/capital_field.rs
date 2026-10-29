//! One prose field's typing aids — `CapitalField.cs`: the state a text field keeps between its
//! events, and every decision about them, so the window's part is to read the field, ask, and apply.
//! Two rules run here: capital letters at a sentence's start (issue #97; [`crate::capitals`]), in
//! the MULTI-LINE prose fields only, and emoji as you type (issue #101; [`crate::emoji_typing`]), in
//! EVERY prose field. On a field that has both, capitals decide the typed character first and the
//! emoji rule sees what they decided — so `xD` + space at a sentence's start is `XD `, and 😆.
//!
//! The order the events come in is a hardware keyboard's — the key, then the text change it makes
//! — and what this type assumes is written next to each. On Linux the shell feeds it from GTK
//! (shell-design §7): a capture-phase key controller gives [`CapitalField::key`] (classified by
//! [`CapitalField::classify_keysym`]) and [`CapitalField::backspace`]; the buffer's
//! `begin-user-action` snapshot and `end-user-action` give [`CapitalField::before_change`];
//! `preedit-changed` gives the composition; `paste-clipboard` gives [`CapitalField::pasting`]; the
//! replacement goes in on a high-priority idle as its own user action. Offsets are UTF-16 units
//! (carry-forward, "Caret units"): the shell converts GTK's character offsets at its edge.
//!
//! ONLY TYPING IS JUDGED. A change is a candidate only when a key went down just before it
//! ([`CapitalField::key`]), the field is not composing (an input method's candidate text is not
//! typing), and it is not a paste ([`CapitalField::pasting`]). Every text the window sets itself —
//! a draft put back, a message opened for editing, a name accepted from the mention list — is
//! therefore left exactly as it is, even when it happens to be one lower-case letter or the colon
//! that closes a name. An on-screen keyboard, which raises no key events, is left alone too.
//!
//! APPLIED AFTER, NEVER INSTEAD. The character the keyboard typed goes into the field as typed —
//! one undo unit — and is then replaced — a second — on the next high-priority turn, ahead of the
//! next input: by its capital, or (with what was typed before it) by an emoji. So the keystroke can
//! never be lost, and Ctrl+Z straight after gives the typed text back. md.win's smart typing learnt
//! both halves on real Windows (2026-09-27), and md.linux's GTK hook pattern is the same shape.
//!
//! ONE LIVE REPLACEMENT at a time: the capital ([`CapitalField::take_back`]) or the emoji
//! ([`CapitalField::swap`]) the rule just made, which a Backspace straight after takes back. The
//! capital is forgotten by the next key; the emoji survives the next TYPING key until its change is
//! judged, because that character may grow it (`:(` then `(` is `:((`), and is forgotten by
//! anything else — another key, a click, a selection, a paste, the focus leaving.

use crate::capitals::{self, CapitalTakeBack, split_at_units, utf16_len};
use crate::emoji_typing::{self, EmojiSwap};

/// What a key press is, to [`CapitalField`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapitalKey {
    /// Shift, Ctrl, Alt, the Windows (Super) key, Caps Lock: on their own they change nothing, and
    /// forget nothing.
    Modifier,
    /// Backspace with nothing but Shift held: the one key that may take a capital or an emoji back.
    Backspace,
    /// A Ctrl chord — Ctrl+V, Ctrl+Z, Ctrl+Backspace: a paste, a step through history or a word
    /// deleted, never a letter typed. (On Windows only without Alt: Ctrl with Alt is AltGr there,
    /// which TYPES on most European layouts — `ą` is AltGr+A in Polish. Not so on Linux:
    /// [`CapitalField::classify_keysym`].)
    Chord,
    /// A key that types nothing: an arrow, Home, End, Page Up or Down, Tab, Esc, Insert, Delete, a
    /// function key, the menu key. It forgets the take-back, and whatever changes the text after it
    /// — a name accepted from the mention list on Tab, a draft put back — was not typed.
    Moving,
    /// Anything else: a letter, a digit, a space, punctuation, Enter.
    Typing,
}

/// Backspace's take-back, as an edit: replace `length` UTF-16 units at `at` with `replacement`,
/// and put the caret at `caret`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restore {
    pub at: usize,
    pub length: usize,
    pub replacement: String,
    pub caret: usize,
}

/// What the field does to a typed character, between the change that typed it and the turn that
/// rewrites it: the field should then hold `expected` with the caret at `typed_end`, and the plan
/// replaces `length` units at `at` with `replacement` and puts the caret at `caret`. Exactly one of
/// `take_back` (a capital, #97) and `swap` (an emoji, #101) is set: what the field keeps after.
/// Every offset is in UTF-16 units.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapitalPlan {
    /// The field's text once the typed character is in.
    pub expected: String,
    /// Where the caret is then: right behind the typed character.
    pub typed_end: usize,
    /// Where the replacement starts.
    pub at: usize,
    /// How many units it replaces: the typed character, and for an emoji what was typed before it.
    pub length: usize,
    /// The capital, or the emoji (and its space).
    pub replacement: String,
    /// Where the caret goes after: right behind the replacement.
    pub caret: usize,
    /// The capital's take-back, for a capital.
    pub take_back: Option<CapitalTakeBack>,
    /// The emoji's swap, for an emoji.
    pub swap: Option<EmojiSwap>,
}

/// One prose field's typing aids (see the module's notes).
#[derive(Debug, Clone)]
pub struct CapitalField {
    has_capitals: bool,
    take_back: Option<CapitalTakeBack>,
    swap: Option<EmojiSwap>,
    composing: bool,
    applying: bool,
    keyed: bool,
    pasting: bool,
}

impl Default for CapitalField {
    /// A multi-line prose field: capitals and emoji.
    fn default() -> Self {
        CapitalField::new(true)
    }
}

impl CapitalField {
    /// A field with emoji as you type, and — when `capitals` — capital letters at a sentence's
    /// start: true for a multi-line prose field (#97); a single-line one gets emoji only (#101).
    pub fn new(capitals: bool) -> Self {
        CapitalField {
            has_capitals: capitals,
            take_back: None,
            swap: None,
            composing: false,
            applying: false,
            keyed: false,
            pasting: false,
        }
    }

    /// Whether this field capitalises a sentence's first letter (#97: multi-line prose fields only).
    pub fn has_capitals(&self) -> bool {
        self.has_capitals
    }

    /// The capital the field may still take back with a Backspace — kept from the moment it went
    /// in until anything else happens.
    pub fn take_back(&self) -> Option<&CapitalTakeBack> {
        self.take_back.as_ref()
    }

    /// The emoji the field may still take back with a Backspace, or grow with the next typed
    /// character.
    pub fn swap(&self) -> Option<&EmojiSwap> {
        self.swap.as_ref()
    }

    /// While an input method composes (a non-empty preedit): nothing is judged.
    pub fn composing(&self) -> bool {
        self.composing
    }

    /// Set by the window around its own edits, whose change events must not be judged or forget
    /// the take-back.
    pub fn applying(&self) -> bool {
        self.applying
    }

    pub fn set_applying(&mut self, applying: bool) {
        self.applying = applying;
    }

    /// What a key is, from its Win32 virtual-key code and the modifiers held (Windows' own codes,
    /// `VK_SHIFT` 0x10 …, read as numbers so this can be decided — and tested — without Windows).
    /// Linux's counterpart is [`CapitalField::classify_keysym`].
    pub fn classify(virtual_key: i32, control: bool, alt: bool) -> CapitalKey {
        match virtual_key {
            // Shift, Ctrl, Alt; Caps Lock; the left and right Windows keys; the left and right
            // Shift, Ctrl and Alt.
            0x10 | 0x11 | 0x12 | 0x14 | 0x5B | 0x5C | 0xA0..=0xA5 => CapitalKey::Modifier,
            _ if control && !alt => CapitalKey::Chord,
            0x08 if !control && !alt => CapitalKey::Backspace,
            // Tab, Esc; Page Up … Down (the arrows, Home and End among them); Insert, Delete; the
            // menu key; F1 … F24.
            0x09 | 0x1B | 0x21..=0x28 | 0x2D | 0x2E | 0x5D | 0x70..=0x87 => CapitalKey::Moving,
            _ => CapitalKey::Typing,
        }
    }

    /// What a key is on Linux, from its X keysym (a `gdk::Key`'s number) and the modifiers held
    /// (`gdk::ModifierType`'s CONTROL, ALT and SUPER masks) — the shell's counterpart of
    /// [`CapitalField::classify`], to the same five answers.
    ///
    /// WHERE IT PARTS FROM WINDOWS, AND WHY:
    ///
    /// - CTRL IS A CHORD WITH OR WITHOUT ALT. Windows treats Ctrl+Alt as AltGr because Windows
    ///   itself emulates AltGr that way, and AltGr types (`ą` is AltGr+A in Polish). On Linux AltGr
    ///   is its own key — the keysym `ISO_Level3_Shift` (0xfe03), a level-3 modifier — and the
    ///   character it types arrives with neither Ctrl nor Alt in the state: AltGr+A is simply the
    ///   keysym `aogonek` and is TYPING. A real Ctrl+Alt+key types nothing in a GTK field, so it is
    ///   a chord, like every Ctrl combination. (A shell that cannot strip consumed modifiers is
    ///   still safe: a chord only ever makes the rule do less.)
    /// - SUPER IS A CHORD too: "never judged … a chord with Ctrl or Super" (shell-design §7).
    /// - BACKSPACE WITH ALT IS A CHORD, not typing: it deletes nothing GTK would call a character,
    ///   and must not take a capital back. (Windows reads Alt+Backspace as typing.)
    /// - The lock keys (Caps, Num, Scroll, Shift Lock) and the level/group shifts are MODIFIERS,
    ///   and the keys that type nothing — Pause, Print, Help, Undo, the multimedia keys — are
    ///   MOVING. Windows lists neither its Num/Scroll Lock nor Pause/Print and so reads them as
    ///   typing; here they change nothing typed, as their names say.
    /// - A dead key (`dead_acute` …) is TYPING, as on Windows (`VK_OEM_7`): it types nothing by
    ///   itself, which is exactly the "a typing key that typed nothing" case [`CapitalField::key`]
    ///   handles. Enter (`Return`, `KP_Enter`) is typing too.
    pub fn classify_keysym(keysym: u32, control: bool, alt: bool, super_key: bool) -> CapitalKey {
        if is_modifier_keysym(keysym) {
            return CapitalKey::Modifier;
        }
        if control || super_key {
            return CapitalKey::Chord;
        }
        if keysym == keysym::BACKSPACE {
            return if alt {
                CapitalKey::Chord
            } else {
                CapitalKey::Backspace
            };
        }
        if is_moving_keysym(keysym) {
            return CapitalKey::Moving;
        }
        CapitalKey::Typing
    }

    pub fn composition_started(&mut self) {
        self.composing = true;
        self.keyed = false;
        self.forget();
    }

    pub fn composition_ended(&mut self) {
        self.composing = false;
    }

    /// A key went down (before the field acts on it). Anything but a modifier forgets the capital —
    /// the Backspace that uses it goes through [`CapitalField::backspace`] first — and anything but
    /// typing forgets the emoji. Typing makes the change that follows a candidate; and a typing key
    /// that comes while the one before it is still waiting for its change (it typed nothing: Enter
    /// in a single-line field, a dead key, a character the field refused) is "something else
    /// happened", so the emoji is forgotten.
    pub fn key(&mut self, key: CapitalKey) {
        if key == CapitalKey::Modifier {
            return;
        }
        self.take_back = None;
        if key != CapitalKey::Typing || self.keyed {
            self.swap = None;
        }
        self.keyed = matches!(key, CapitalKey::Typing | CapitalKey::Backspace) && !self.composing;
    }

    /// A paste is about to change the text: whatever it brings is not typed, and nothing is taken
    /// back after it.
    pub fn pasting(&mut self) {
        self.pasting = true;
        self.forget();
    }

    /// Backspace, before the field acts on it, with its text and selection as they stand (UTF-16
    /// units): the edit that puts back what was typed — the lower-case letter, or every character
    /// the emoji replaced — or `None`, and the Backspace is an ordinary one. Either way the
    /// take-back is used up.
    pub fn backspace(
        &mut self,
        text: &str,
        selection_start: usize,
        selection_length: usize,
    ) -> Option<Restore> {
        let take_back = self.take_back.take();
        let swap = self.swap.take();
        // A typing key since that changed nothing: the replacement is no longer the last thing
        // that happened.
        let stale = self.keyed;
        self.forget();
        if self.composing || stale {
            return None;
        }
        if let Some(swap) = swap {
            if selection_length != 0 {
                return None;
            }
            let (_, caret) = swap.backspace(text, selection_start)?;
            return Some(Restore {
                at: swap.at,
                length: utf16_len(&swap.put),
                replacement: swap.typed,
                caret,
            });
        }
        let take_back = take_back?;
        let (_, caret) = take_back.backspace(text, selection_start, selection_length)?;
        Some(Restore {
            at: take_back.at,
            length: utf16_len(&take_back.capital),
            replacement: take_back.typed,
            caret,
        })
    }

    /// The text is about to change (the field still holds the OLD text and selection; `new_text`
    /// is what the change makes). The plan for what was typed — its capital, or an emoji — or
    /// `None`. A change the window is making itself is ignored; any other forgets the take-back and
    /// uses up the key and the paste that came before it.
    pub fn before_change(
        &mut self,
        old_text: &str,
        selection_start: usize,
        selection_length: usize,
        new_text: &str,
    ) -> Option<CapitalPlan> {
        if self.applying {
            return None;
        }
        let judged = self.keyed && !self.pasting && !self.composing;
        // Only a bare caret can be right behind the emoji; typing over a selection grows nothing.
        let live = if selection_length == 0 {
            self.swap.take()
        } else {
            None
        };
        self.keyed = false;
        self.pasting = false;
        self.forget();
        if !judged {
            return None;
        }
        let (start, typed) =
            capitals::typed_character(old_text, selection_start, selection_length, new_text)?;
        let (before, _) = split_at_units(new_text, start)?;
        let typed_end = start + utf16_len(&typed);
        // 1. Capitals decide the typed character (#97) …
        let capital = if self.has_capitals && capitals::capitalizes(before, &typed) {
            CapitalTakeBack::after(start, &typed)
        } else {
            None
        };
        // 2. … and the emoji rule sees what they decided (#101). An emoji replaces everything from
        // where it starts to the end of the character as the field holds it.
        let decided = capital
            .as_ref()
            .map_or(typed.as_str(), |c| c.capital.as_str());
        if let Some(swap) = emoji_typing::replace(before, decided, live.as_ref()) {
            return Some(CapitalPlan {
                expected: new_text.to_string(),
                typed_end,
                at: swap.at,
                length: typed_end.saturating_sub(swap.at),
                replacement: swap.put.clone(),
                caret: swap.at + utf16_len(&swap.put),
                take_back: None,
                swap: Some(swap),
            });
        }
        let capital = capital?;
        Some(CapitalPlan {
            expected: new_text.to_string(),
            typed_end,
            at: start,
            length: utf16_len(&typed),
            replacement: capital.capital.clone(),
            caret: start + utf16_len(&capital.capital),
            take_back: Some(capital),
            swap: None,
        })
    }

    /// Whether `plan` still fits the field on the turn it is applied: the field holds exactly the
    /// text the typing made, with the caret right behind the typed character and nothing selected,
    /// and — when the field has a `max_length` (0 is none; UTF-16 units, as a field's limit
    /// counts) — the replacement fits under it (a few shortcodes are SHORTER than their emoji:
    /// `:wales:` is 7 units, its flag 14). Anything else — the change was refused, or more happened
    /// first — and the character stays as typed.
    pub fn still_applies(
        &self,
        plan: &CapitalPlan,
        text: &str,
        selection_start: usize,
        selection_length: usize,
        max_length: usize,
    ) -> bool {
        !self.composing
            && selection_length == 0
            && selection_start == plan.typed_end
            && text == plan.expected
            && (max_length == 0
                || utf16_len(text).saturating_sub(plan.length) + utf16_len(&plan.replacement)
                    <= max_length)
    }

    /// The replacement is in: the next Backspace, straight after, may take it back — and the next
    /// typed character may grow an emoji.
    pub fn applied(&mut self, plan: &CapitalPlan) {
        self.take_back = plan.take_back.clone();
        self.swap = plan.swap.clone();
    }

    /// The selection moved (a click, an arrow, a drag). Unless it is still the bare caret right
    /// behind the replacement, the take-back is forgotten.
    pub fn selection_moved(&mut self, selection_start: usize, selection_length: usize) {
        if self.applying {
            return;
        }
        if let Some(take_back) = &self.take_back
            && (selection_length != 0
                || selection_start != take_back.at + utf16_len(&take_back.capital))
        {
            self.take_back = None;
        }
        if let Some(swap) = &self.swap
            && (selection_length != 0 || selection_start != swap.at + utf16_len(&swap.put))
        {
            self.swap = None;
        }
    }

    /// Focus left the field: whatever was typed last is no longer "just" typed.
    pub fn focus_lost(&mut self) {
        self.keyed = false;
        self.pasting = false;
        self.forget();
    }

    fn forget(&mut self) {
        self.take_back = None;
        self.swap = None;
    }
}

/// The X keysyms [`CapitalField::classify_keysym`] names (`<X11/keysymdef.h>`; `gdk::Key`'s values).
pub mod keysym {
    pub const BACKSPACE: u32 = 0xff08;
    pub const TAB: u32 = 0xff09;
    pub const LINEFEED: u32 = 0xff0a;
    pub const RETURN: u32 = 0xff0d;
    pub const PAUSE: u32 = 0xff13;
    pub const SCROLL_LOCK: u32 = 0xff14;
    pub const SYS_REQ: u32 = 0xff15;
    pub const ESCAPE: u32 = 0xff1b;
    pub const HOME: u32 = 0xff50;
    pub const LEFT: u32 = 0xff51;
    pub const UP: u32 = 0xff52;
    pub const RIGHT: u32 = 0xff53;
    pub const DOWN: u32 = 0xff54;
    pub const PAGE_UP: u32 = 0xff55;
    pub const PAGE_DOWN: u32 = 0xff56;
    pub const END: u32 = 0xff57;
    pub const BEGIN: u32 = 0xff58;
    pub const SELECT: u32 = 0xff60;
    pub const PRINT: u32 = 0xff61;
    pub const EXECUTE: u32 = 0xff62;
    pub const INSERT: u32 = 0xff63;
    pub const UNDO: u32 = 0xff65;
    pub const REDO: u32 = 0xff66;
    pub const MENU: u32 = 0xff67;
    pub const FIND: u32 = 0xff68;
    pub const CANCEL: u32 = 0xff69;
    pub const HELP: u32 = 0xff6a;
    pub const BREAK: u32 = 0xff6b;
    pub const MODE_SWITCH: u32 = 0xff7e;
    pub const NUM_LOCK: u32 = 0xff7f;
    pub const KP_TAB: u32 = 0xff89;
    pub const KP_ENTER: u32 = 0xff8d;
    pub const KP_F1: u32 = 0xff91;
    pub const KP_F4: u32 = 0xff94;
    pub const KP_HOME: u32 = 0xff95;
    pub const KP_DELETE: u32 = 0xff9f;
    pub const F1: u32 = 0xffbe;
    pub const F35: u32 = 0xffe0;
    pub const SHIFT_L: u32 = 0xffe1;
    pub const CONTROL_L: u32 = 0xffe3;
    pub const CAPS_LOCK: u32 = 0xffe5;
    pub const ALT_L: u32 = 0xffe9;
    pub const SUPER_L: u32 = 0xffeb;
    pub const HYPER_R: u32 = 0xffee;
    pub const DELETE: u32 = 0xffff;
    /// The ISO lock, latch and level-shift keys: 0xfe01 `ISO_Lock` … 0xfe0f `ISO_Last_Group_Lock`,
    /// AltGr (`ISO_Level3_Shift`, 0xfe03) and Level 5 (0xfe11 … 0xfe13) among them.
    pub const ISO_LOCK: u32 = 0xfe01;
    pub const ISO_LEVEL3_SHIFT: u32 = 0xfe03;
    pub const ISO_LAST_GROUP_LOCK: u32 = 0xfe0f;
    pub const ISO_LEVEL5_SHIFT: u32 = 0xfe11;
    pub const ISO_LEVEL5_LOCK: u32 = 0xfe13;
    /// Shift+Tab.
    pub const ISO_LEFT_TAB: u32 = 0xfe20;
    /// The first and last dead key (`dead_grave` … ): typing, as on Windows.
    pub const DEAD_GRAVE: u32 = 0xfe50;
    /// The vendor keys — volume, media, brightness, the browser keys (`XF86XK_*`).
    pub const XF86_FIRST: u32 = 0x1008_ff01;
    pub const XF86_LAST: u32 = 0x1008_ffff;
}

/// Shift, Control, Caps/Shift Lock, Meta, Alt, Super and Hyper (left and right), Num and Scroll
/// Lock, Mode_switch and the ISO level and group keys — AltGr among them.
fn is_modifier_keysym(value: u32) -> bool {
    matches!(
        value,
        keysym::SHIFT_L..=keysym::HYPER_R
            | keysym::ISO_LOCK..=keysym::ISO_LAST_GROUP_LOCK
            | keysym::ISO_LEVEL5_SHIFT..=keysym::ISO_LEVEL5_LOCK
            | keysym::MODE_SWITCH
            | keysym::NUM_LOCK
            | keysym::SCROLL_LOCK
    )
}

/// Keys that type nothing: Tab (Shift+Tab too), Escape, the cursor block (Home … Begin), Insert,
/// Delete, the menu key, the function keys (F1 … F35, KP_F1 … KP_F4), the keypad's own cursor keys
/// with Num Lock off (KP_Home … KP_Delete), KP_Tab — and what Windows forgot: Pause, Print, Sys
/// Req, Select, Execute, Undo, Redo, Find, Cancel, Help, Break and the vendor (multimedia) keys.
fn is_moving_keysym(value: u32) -> bool {
    matches!(
        value,
        keysym::TAB
            | keysym::ISO_LEFT_TAB
            | keysym::KP_TAB
            | keysym::ESCAPE
            | keysym::HOME..=keysym::BEGIN
            | keysym::SELECT..=keysym::BREAK
            | keysym::PAUSE
            | keysym::SYS_REQ
            | keysym::DELETE
            | keysym::KP_F1..=keysym::KP_F4
            | keysym::KP_HOME..=keysym::KP_DELETE
            | keysym::F1..=keysym::F35
            | keysym::XF86_FIRST..=keysym::XF86_LAST
    )
}
