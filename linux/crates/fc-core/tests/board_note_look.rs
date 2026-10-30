//! The names a note's look travels under, and the fallbacks. EVERY name falls back rather than
//! failing — and the fallback is for DRAWING only, which is the half a port keeps getting wrong. A
//! port of `win/tests/FamilyConnect.Core.Tests/Board/NoteLookTests.cs`, against `fc_text::board`
//! where the rule lives there.

use std::collections::HashSet;

use fc_core::board::note_look::font_family;
use fc_text::board::{Answer, COLORS, Font, Kind, Size, color_hex, going_line};
use fc_text::i18n::{Lang, use_lang};

/// `ColorHex(string?)`: `color_hex` takes a `&str`, and a colour the wire omitted reaches it as
/// empty — the way the web client passes `note.color.as_deref().unwrap_or_default()`.
fn hex_of(color: Option<&str>) -> &'static str {
    color_hex(color.unwrap_or_default())
}

#[test]
fn every_name_round_trips_through_the_wire_spelling() {
    for size in Size::ALL {
        assert_eq!(Size::from_name(Some(size.name())), size);
    }
    for font in Font::ALL {
        assert_eq!(Font::from_name(Some(font.name())), font);
    }
    for answer in Answer::ALL {
        assert_eq!(Answer::from_name(Some(answer.name())), Some(answer));
    }
    for kind in [Kind::Text, Kind::Photo, Kind::Event, Kind::Tasks] {
        assert_eq!(Kind::from_name(Some(kind.name())), kind);
    }
}

#[test]
fn an_unknown_name_draws_rather_than_failing() {
    assert_eq!(Size::from_name(Some("huge")), Size::Medium);
    assert_eq!(Size::from_name(None), Size::Medium);
    assert_eq!(Size::from_name(Some("Large")), Size::Medium);
    assert_eq!(Kind::from_name(Some("video")), Kind::Text);
    assert_eq!(Font::from_name(Some("comic")), Font::Plain);
    assert_eq!(color_hex("chartreuse"), "#fff2b3");
    assert_eq!(hex_of(None), "#fff2b3");
    // An RSVP is the exception: better NO button lit than a claim that somebody said something
    // else.
    assert_eq!(Answer::from_name(Some("perhaps")), None);
    assert_eq!(Answer::from_name(None), None);
}

#[test]
fn an_edit_sends_the_size_and_face_only_when_the_author_changed_them() {
    assert_eq!(Size::Medium.patch_name(Some("medium")), None);
    assert_eq!(Size::Large.patch_name(Some("medium")), Some("large"));
    assert_eq!(Size::Small.patch_name(Some("large")), Some("small"));
    // A FOURTH size from a newer server draws as medium and must not be written back as medium
    // because somebody fixed a typo in the text.
    assert_eq!(Size::Medium.patch_name(Some("huge")), None);
    assert_eq!(Size::Large.patch_name(Some("huge")), Some("large"));
    assert_eq!(Size::Medium.patch_name(None), None);
    assert_eq!(Font::Plain.patch_name(Some("plain")), None);
    assert_eq!(Font::Mono.patch_name(Some("plain")), Some("mono"));
    assert_eq!(Font::Plain.patch_name(Some("handwriting")), None);
}

#[test]
fn the_six_colours_are_the_protocols_six_in_the_pickers_order() {
    assert_eq!(
        COLORS,
        ["yellow", "pink", "blue", "green", "orange", "purple"]
    );
    // The apps' own pastels, and all six distinct.
    let hexes: HashSet<&str> = COLORS.map(color_hex).into_iter().collect();
    assert_eq!(hexes.len(), COLORS.len());
    assert_eq!(color_hex("blue"), "#c2e0fc");
}

/// `EveryHandResolvesToAFaceThatShipsWithWindows`, renamed for the platform it now holds: the
/// hand is an INTENT, and `font_family` is the Linux answer to it (the C# asserts Segoe UI).
#[test]
fn every_hand_resolves_to_a_face_that_ships_with_ubuntu() {
    // Nothing bundled, nothing synthesised — and each hand distinct, or the picker would be a lie.
    let families = Font::ALL.map(font_family);
    let distinct: HashSet<&str> = families.into_iter().collect();
    assert_eq!(distinct.len(), families.len());
    assert!(families.iter().all(|family| !family.trim().is_empty()));
    assert!(font_family(Font::Plain).contains("Ubuntu Sans"));
    // And every list ends in the generic family fontconfig always resolves, so a face that is
    // missing falls through to one that is not.
    for (font, generic) in [
        (Font::Plain, "sans-serif"),
        (Font::Serif, "serif"),
        (Font::Mono, "monospace"),
        (Font::Casual, "cursive"),
    ] {
        let last = font_family(font).rsplit(',').next().map(str::trim);
        assert_eq!(last, Some(generic), "{font:?}");
    }
}

#[test]
fn the_sticker_says_how_many_are_coming_and_nothing_while_nobody_is() {
    // `EnglishCatalog.Instance`: the reader's language is English.
    use_lang(Lang::En);
    assert_eq!(going_line(0, 0), None);
    assert_eq!(going_line(2, 0).as_deref(), Some("2 going"));
    assert_eq!(going_line(0, 1).as_deref(), Some("1 maybe"));
    assert_eq!(going_line(2, 1).as_deref(), Some("2 going, 1 maybe"));
    // "Can't" is a word on a button, not a count.
    assert_eq!(Answer::No.title(), "Can't");
    assert_eq!(Answer::Going.title(), "Going");
}
