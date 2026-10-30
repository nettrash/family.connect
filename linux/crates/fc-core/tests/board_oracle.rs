//! THE DIFFERENTIAL ORACLE: every vector in `fixtures/board-vectors.json` was produced by
//! `fc_text::board` itself — the Rust the web client runs — and this suite holds the Linux client
//! to it, number for number and string for string. A port of
//! `win/tests/FamilyConnect.Core.Tests/Board/OracleTests.cs`; the fixture is the Windows one, byte
//! for byte.
//!
//! The Linux client CALLS `fc_text::board`, so here the oracle mostly checks the original against
//! the file it once wrote — which is the point: `fc-text` has no test of its own that reads this
//! fixture, so a change to a board rule that the Windows fixture was not regenerated for fails
//! here, rather than leaving the two desktop clients drawing different walls. Regenerate with
//! `win/tools/board-oracle` (see `win/README.md`), and copy the file here unchanged.
//!
//! Four implementations of one rule need an oracle, not four readings: the portfolio has been
//! bitten by a byte-versus-character split that panicked on Cyrillic, and by four ports that agreed
//! with each other and were all wrong about `pow(10, n)`.

mod common;

use std::path::Path;
use std::sync::LazyLock;

use common::assert_equal_to_places;
use fc_core::board::board_picture;
use fc_text::board::{
    Answer, COMPACT_BELOW, COUNTER_FROM, FIT_STEPS, Kind, MAX_PLACE_CHARS, MAX_TASK_ITEM_CHARS,
    MAX_TASK_ITEMS, MAX_TEXT_CHARS, MIN_TEXT_SCALE, Marks, Size, WALL_SCREENS, WALL_TASK_LINES,
    cap_at_caret, capped, color_hex, dragged, fraction_of, going_line, is_unread, lines_that_fit,
    origin, remaining, shows_counter, tilt_degrees, wall_height, wall_task_lines,
};
use fc_text::i18n::{Lang, use_lang};
use serde_json::Value;

static VECTORS: LazyLock<Value> = LazyLock::new(|| {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/board-vectors.json");
    let text = std::fs::read_to_string(&path).expect("the board vectors");
    serde_json::from_str(&text).expect("the board vectors are JSON")
});

/// A field the fixture must have — as `GetProperty`, which throws on a missing one.
fn field<'a>(vector: &'a Value, name: &str) -> &'a Value {
    vector
        .get(name)
        .unwrap_or_else(|| panic!("no {name:?} in {vector}"))
}

fn section(name: &str) -> &'static Value {
    field(&VECTORS, name)
}

fn vectors(name: &str) -> &'static [Value] {
    section(name)
        .as_array()
        .unwrap_or_else(|| panic!("{name} is not a list"))
}

fn int(vector: &Value, name: &str) -> i64 {
    field(vector, name)
        .as_i64()
        .unwrap_or_else(|| panic!("{name} is not an integer"))
}

fn count(vector: &Value, name: &str) -> usize {
    usize::try_from(int(vector, name)).expect("a count is not negative")
}

fn double(value: &Value) -> f64 {
    value
        .as_f64()
        .unwrap_or_else(|| panic!("{value} is not a number"))
}

fn number(vector: &Value, name: &str) -> f64 {
    double(field(vector, name))
}

fn text<'a>(vector: &'a Value, name: &str) -> &'a str {
    field(vector, name)
        .as_str()
        .unwrap_or_else(|| panic!("{name} is not a string"))
}

fn flag(vector: &Value, name: &str) -> bool {
    field(vector, name)
        .as_bool()
        .unwrap_or_else(|| panic!("{name} is not a boolean"))
}

/// A string or a null — `GetString` on a JSON null is null in the C#.
fn optional_text<'a>(vector: &'a Value, name: &str) -> Option<&'a str> {
    match field(vector, name) {
        Value::Null => None,
        Value::String(text) => Some(text),
        other => panic!("{name} is {other}, not a string or null"),
    }
}

fn pair(vector: &Value, name: &str) -> (f64, f64) {
    let value = field(vector, name);
    (double(&value[0]), double(&value[1]))
}

/// The port's constant is on the left throughout, as in the C#; the ORACLE is still the
/// authority — what is asserted is that the two agree.
#[test]
fn the_constants_are_the_originals() {
    let constants = section("constants");
    assert_eq!(MAX_TEXT_CHARS, count(constants, "max_text"));
    assert_eq!(MAX_PLACE_CHARS, count(constants, "max_place"));
    assert_eq!(MAX_TASK_ITEM_CHARS, count(constants, "max_task_item"));
    assert_eq!(MAX_TASK_ITEMS, count(constants, "max_task_items"));
    assert_eq!(COUNTER_FROM, count(constants, "counter_from"));
    assert_eq!(WALL_SCREENS, number(constants, "wall_screens"));
    assert_eq!(WALL_TASK_LINES, count(constants, "wall_task_lines"));
    assert_eq!(COMPACT_BELOW, number(constants, "compact_below"));
    assert_eq!(MIN_TEXT_SCALE, number(constants, "min_text_scale"));
    assert_eq!(
        FIT_STEPS,
        u32::try_from(int(constants, "fit_steps")).expect("a step count")
    );
}

#[test]
fn the_cap_cuts_where_the_original_cuts() {
    for vector in vectors("capped") {
        let value = text(vector, "text");
        let max = count(vector, "max");
        assert_eq!(count(vector, "scalars"), value.chars().count());
        assert_eq!(text(vector, "kept"), capped(value, max));
    }
}

#[test]
fn the_caret_moves_where_the_original_moves_it() {
    for vector in vectors("cap_at_caret") {
        let value = text(vector, "value");
        let caret = count(vector, "caret");
        let max = count(vector, "max");
        let (kept, moved) = cap_at_caret(value, caret, max);
        assert_eq!(text(vector, "kept"), kept);
        assert_eq!(count(vector, "moved"), moved);
    }
}

#[test]
fn the_counter_counts_what_the_original_counts() {
    for vector in vectors("remaining") {
        let value = text(vector, "text");
        assert_eq!(count(vector, "remaining"), remaining(value));
        assert_eq!(flag(vector, "counter"), shows_counter(value));
    }
}

#[test]
fn a_picture_is_fitted_exactly_as_the_original_fits_it() {
    for vector in vectors("fitted_picture") {
        let space = pair(vector, "space");
        let picture = pair(vector, "picture");
        let fitted = pair(vector, "fitted");
        // `(int)picture[0].GetDouble()`: the dimensions reach the port as the whole pixels an
        // attachment carries.
        let (width, height) = board_picture::fitted(
            space.0,
            space.1,
            Some(picture.0 as i32),
            Some(picture.1 as i32),
        );
        assert_equal_to_places(fitted.0, width, 9);
        assert_equal_to_places(fitted.1, height, 9);
    }
}

#[test]
fn the_wall_and_the_cards_measure_as_the_original_does() {
    for vector in vectors("wall_height") {
        assert_equal_to_places(
            number(vector, "height"),
            wall_height(number(vector, "visible")),
            9,
        );
    }
    for vector in vectors("cards") {
        let size = Size::from_name(Some(text(vector, "size")));
        let compact = flag(vector, "compact");
        assert_eq!(pair(vector, "card"), size.frame(compact));
        assert_equal_to_places(number(vector, "type_px"), size.type_px(), 9);
    }
    for vector in vectors("lines_that_fit") {
        assert_eq!(
            u32::try_from(int(vector, "lines")).expect("a line count"),
            lines_that_fit(number(vector, "height"), number(vector, "line_height"))
        );
    }
}

#[test]
fn a_note_sits_where_the_original_puts_it() {
    for vector in vectors("geometry") {
        let fraction = pair(vector, "fraction");
        let card = pair(vector, "card");
        let board = pair(vector, "board");
        let offset = pair(vector, "offset");
        let at = pair(vector, "origin");
        let moved = pair(vector, "dragged");
        let back = pair(vector, "fraction_of_origin");
        assert_eq!(at, origin(fraction, card, board));
        assert_eq!(moved, dragged(fraction, offset, card, board));
        let (x, y) = fraction_of(at, board);
        assert_equal_to_places(back.0, x, 9);
        assert_equal_to_places(back.1, y, 9);
    }
    for vector in vectors("tilt") {
        assert_eq!(int(vector, "degrees"), tilt_degrees(int(vector, "id")));
    }
    for vector in vectors("task_lines") {
        let (shown, left) = wall_task_lines(count(vector, "total"));
        assert_eq!(count(vector, "shown"), shown);
        assert_eq!(count(vector, "left"), left);
    }
}

#[test]
fn every_name_falls_back_as_the_original_falls_back() {
    for vector in vectors("colors") {
        // `ColorHex(name)` with a null name is `color_hex("")`, as the web client passes it.
        let name = optional_text(vector, "name").unwrap_or_default();
        assert_eq!(text(vector, "hex"), color_hex(name));
    }
    for vector in vectors("fallbacks") {
        let given = optional_text(vector, "given");
        assert_eq!(text(vector, "size"), Size::from_name(given).name());
    }
    for vector in vectors("kinds") {
        let given = optional_text(vector, "given");
        assert_eq!(text(vector, "kind"), Kind::from_name(given).name());
    }
    for vector in vectors("answers") {
        let given = optional_text(vector, "given");
        let answer = Answer::from_name(given);
        match optional_text(vector, "answer") {
            // The one field where an unknown value lights NOTHING rather than falling back.
            None => assert_eq!(answer, None),
            Some(expected) => assert_eq!(Some(expected), answer.map(Answer::name)),
        }
    }
}

#[test]
fn the_sticker_says_what_the_original_says() {
    // `EnglishCatalog.Instance`: the reader's language is English.
    use_lang(Lang::En);
    for vector in vectors("going_line") {
        let line = going_line(count(vector, "going"), count(vector, "maybe"));
        assert_eq!(optional_text(vector, "line"), line.as_deref());
    }
}

#[test]
fn the_badge_judges_as_the_original_judges() {
    let marks = Marks {
        note_id: 10,
        content_seq: 100,
    };
    for vector in vectors("unread") {
        let seq = match field(vector, "content_seq") {
            Value::Null => None,
            _ => Some(int(vector, "content_seq")),
        };
        assert_eq!(
            flag(vector, "unread"),
            is_unread(int(vector, "note_id"), seq, marks)
        );
    }
}
