//! `CapitalField::classify_keysym`: Linux's counterpart of `CapitalField::classify` — what a key is
//! from its X keysym (a `gdk::Key`'s number) and the modifiers GTK reports. The cases mirror
//! `CapitalFieldTests.AltGrTypes` and the keys the C# field tests press, in Linux's terms, and then
//! pin where Linux parts from Windows: AltGr is its own key here (`ISO_Level3_Shift`), so every
//! Ctrl combination is a chord; Super is a chord (shell-design §7); the lock keys are modifiers and
//! the keys that type nothing are moving.

use fc_logic::capital_field::{CapitalField, CapitalKey, keysym};

/// No modifier held.
fn bare(value: u32) -> CapitalKey {
    CapitalField::classify_keysym(value, false, false, false)
}

const V: u32 = 0x0076;
const A: u32 = 0x0061;
const Z: u32 = 0x007a;
const SPACE: u32 = 0x0020;
/// `ą`, which AltGr+A types on a Polish layout: the keysym itself, with neither Ctrl nor Alt held.
const AOGONEK: u32 = 0x01b1;
/// `dead_acute`.
const DEAD_ACUTE: u32 = 0xfe51;
const KP_0: u32 = 0xffb0;
const KP_LEFT: u32 = 0xff96;
const F12: u32 = 0xffc9;
const XF86_AUDIO_RAISE_VOLUME: u32 = 0x1008_ff13;

/// `AltGrTypes`' classifications, key for key: Ctrl+V and Ctrl+Backspace are chords, a bare
/// Backspace takes back, the right Control key is a modifier even with Ctrl held, Left and Tab
/// move, Enter and Space type.
#[test]
fn the_windows_cases_in_keysyms() {
    assert_eq!(
        CapitalField::classify_keysym(V, true, false, false),
        CapitalKey::Chord
    );
    assert_eq!(
        CapitalField::classify_keysym(keysym::BACKSPACE, true, false, false),
        CapitalKey::Chord
    );
    assert_eq!(bare(keysym::BACKSPACE), CapitalKey::Backspace);
    assert_eq!(
        CapitalField::classify_keysym(keysym::CONTROL_L + 1, true, false, false),
        CapitalKey::Modifier
    );
    assert_eq!(bare(keysym::LEFT), CapitalKey::Moving);
    assert_eq!(bare(keysym::TAB), CapitalKey::Moving);
    assert_eq!(bare(keysym::RETURN), CapitalKey::Typing);
    assert_eq!(bare(SPACE), CapitalKey::Typing);
    assert_eq!(bare(A), CapitalKey::Typing);
}

/// AltGr TYPES on Linux too — but as its own level-3 key, not as Ctrl+Alt: pressed alone it is a
/// modifier, and what it types arrives as the character's own keysym with neither Ctrl nor Alt.
/// The `ą` it types starts a sentence like any letter.
#[test]
fn alt_gr_is_its_own_key_and_types() {
    assert_eq!(bare(keysym::ISO_LEVEL3_SHIFT), CapitalKey::Modifier);
    assert_eq!(bare(keysym::ISO_LEVEL5_SHIFT), CapitalKey::Modifier);
    assert_eq!(bare(keysym::MODE_SWITCH), CapitalKey::Modifier);
    assert_eq!(bare(AOGONEK), CapitalKey::Typing);

    let mut field = CapitalField::new(true);
    field.key(bare(keysym::ISO_LEVEL3_SHIFT));
    field.key(bare(AOGONEK));
    let plan = field.before_change("", 0, 0, "ą").expect("a capital");
    assert_eq!(plan.replacement, "Ą");
}

/// So a real Ctrl+Alt is a chord: it types nothing in a GTK field, and must not make the change
/// after it count as typed.
#[test]
fn control_with_alt_is_a_chord() {
    assert_eq!(
        CapitalField::classify_keysym(V, true, true, false),
        CapitalKey::Chord
    );
    assert_eq!(
        CapitalField::classify_keysym(A, true, true, false),
        CapitalKey::Chord
    );

    let mut field = CapitalField::new(true);
    field.key(CapitalField::classify_keysym(A, true, true, false));
    assert_eq!(field.before_change("", 0, 0, "a"), None);
}

/// "Never judged … a chord with Ctrl or Super" (shell-design §7).
#[test]
fn super_is_a_chord() {
    assert_eq!(
        CapitalField::classify_keysym(Z, false, false, true),
        CapitalKey::Chord
    );
    assert_eq!(bare(keysym::SUPER_L), CapitalKey::Modifier);
    assert_eq!(bare(keysym::SUPER_L + 1), CapitalKey::Modifier);
    assert_eq!(bare(keysym::HYPER_R), CapitalKey::Modifier);
}

/// Shift+Backspace takes back (the keysym is still BackSpace); Alt+Backspace does not.
#[test]
fn only_a_bare_or_shifted_backspace_takes_back() {
    assert_eq!(bare(keysym::BACKSPACE), CapitalKey::Backspace);
    assert_eq!(
        CapitalField::classify_keysym(keysym::BACKSPACE, false, true, false),
        CapitalKey::Chord
    );
    assert_eq!(
        CapitalField::classify_keysym(keysym::BACKSPACE, false, false, true),
        CapitalKey::Chord
    );
}

/// Every modifier and lock key changes nothing and forgets nothing.
#[test]
fn modifiers_and_locks() {
    for value in [
        keysym::SHIFT_L,
        keysym::SHIFT_L + 1,
        keysym::CONTROL_L,
        keysym::CAPS_LOCK,
        keysym::CAPS_LOCK + 1,
        keysym::ALT_L,
        keysym::ALT_L + 1,
        keysym::ALT_L - 2,
        keysym::NUM_LOCK,
        keysym::SCROLL_LOCK,
        keysym::ISO_LOCK,
        keysym::ISO_LAST_GROUP_LOCK,
    ] {
        assert_eq!(bare(value), CapitalKey::Modifier, "{value:#x}");
    }
}

/// Keys that type nothing: the cursor block, the keypad's with Num Lock off, the function keys,
/// Shift+Tab, Escape, Insert, Delete, the menu key — and those Windows reads as typing for want of
/// listing them: Pause, Print, Help, the multimedia keys.
#[test]
fn keys_that_type_nothing_move() {
    for value in [
        keysym::HOME,
        keysym::LEFT,
        keysym::UP,
        keysym::RIGHT,
        keysym::DOWN,
        keysym::PAGE_UP,
        keysym::PAGE_DOWN,
        keysym::END,
        keysym::BEGIN,
        keysym::ISO_LEFT_TAB,
        keysym::KP_TAB,
        keysym::ESCAPE,
        keysym::INSERT,
        keysym::DELETE,
        keysym::MENU,
        keysym::F1,
        F12,
        keysym::F35,
        keysym::KP_F1,
        keysym::KP_F4,
        keysym::KP_HOME,
        KP_LEFT,
        keysym::KP_DELETE,
        keysym::PAUSE,
        keysym::PRINT,
        keysym::SYS_REQ,
        keysym::UNDO,
        keysym::HELP,
        keysym::BREAK,
        XF86_AUDIO_RAISE_VOLUME,
    ] {
        assert_eq!(bare(value), CapitalKey::Moving, "{value:#x}");
    }
}

/// Typing: letters, digits, the keypad's digits and Enter, Return, a line feed — and a dead key,
/// which types nothing by itself and is the "typing key that typed nothing" case, as on Windows.
#[test]
fn what_types() {
    for value in [
        A,
        AOGONEK,
        SPACE,
        KP_0,
        keysym::KP_ENTER,
        keysym::RETURN,
        keysym::LINEFEED,
        DEAD_ACUTE,
        keysym::DEAD_GRAVE,
    ] {
        assert_eq!(bare(value), CapitalKey::Typing, "{value:#x}");
    }
    // Alt alone on a letter is what Windows calls it: typing (it types nothing in a GTK field, and
    // the field then treats the next typing key as "something else happened").
    assert_eq!(
        CapitalField::classify_keysym(A, false, true, false),
        CapitalKey::Typing
    );
}
