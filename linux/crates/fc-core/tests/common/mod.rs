//! What the text tests share: the fixtures (copied byte for byte from
//! `win/tests/FamilyConnect.Core.Tests/Fixtures`, and `win/`'s embedded catalogues under
//! `fixtures/i18n`), JSON reading, and the catalogue the C# suite gets from its assembly.
//!
//! Each test binary uses a different part of this, so the rest is unused there.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;

use fc_core::text::catalogues::Catalogues;
use fc_core::text::plurals::PluralTables;

/// A fixture's path.
pub fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

/// A fixture's text.
pub fn fixture_text(name: &str) -> String {
    std::fs::read_to_string(fixture(name)).unwrap_or_else(|error| panic!("{name}: {error}"))
}

/// A fixture read as JSON.
pub fn fixture_json(name: &str) -> Value {
    serde_json::from_str(&fixture_text(name)).unwrap_or_else(|error| panic!("{name}: {error}"))
}

/// The catalogues `win/` embeds (`Text/i18n/*.json`), standing in for whatever the Linux client
/// ships: the C# suite reads the ones in its assembly.
pub fn catalogues() -> Catalogues {
    Catalogues::from_dir(&fixture("i18n")).expect("the fixture catalogues load")
}

/// `win/`'s `plurals.json`.
pub fn plurals() -> Rc<PluralTables> {
    catalogues().plurals()
}

// ---- JSON, the way the C# suite reads it -----------------------------------------------------

pub fn string(row: &Value, name: &str) -> String {
    row[name]
        .as_str()
        .unwrap_or_else(|| panic!("{name} is not a string in {row}"))
        .to_string()
}

pub fn optional_string(row: &Value, name: &str) -> Option<String> {
    match &row[name] {
        Value::Null => None,
        value => Some(
            value
                .as_str()
                .unwrap_or_else(|| panic!("{name} in {row}"))
                .to_string(),
        ),
    }
}

pub fn int(row: &Value, name: &str) -> i64 {
    row[name]
        .as_i64()
        .unwrap_or_else(|| panic!("{name} is not an integer in {row}"))
}

pub fn optional_int(row: &Value, name: &str) -> Option<i64> {
    match &row[name] {
        Value::Null => None,
        value => Some(value.as_i64().unwrap_or_else(|| panic!("{name} in {row}"))),
    }
}

pub fn uint(row: &Value, name: &str) -> u64 {
    row[name]
        .as_u64()
        .unwrap_or_else(|| panic!("{name} is not an unsigned integer in {row}"))
}

pub fn optional_uint(row: &Value, name: &str) -> Option<u64> {
    match &row[name] {
        Value::Null => None,
        value => Some(value.as_u64().unwrap_or_else(|| panic!("{name} in {row}"))),
    }
}

pub fn float(row: &Value, name: &str) -> f64 {
    row[name]
        .as_f64()
        .unwrap_or_else(|| panic!("{name} is not a number in {row}"))
}

pub fn optional_float(row: &Value, name: &str) -> Option<f64> {
    match &row[name] {
        Value::Null => None,
        value => Some(value.as_f64().unwrap_or_else(|| panic!("{name} in {row}"))),
    }
}

pub fn boolean(row: &Value, name: &str) -> bool {
    row[name]
        .as_bool()
        .unwrap_or_else(|| panic!("{name} is not a boolean in {row}"))
}

pub fn array<'a>(row: &'a Value, name: &str) -> &'a Vec<Value> {
    row[name]
        .as_array()
        .unwrap_or_else(|| panic!("{name} is not an array in {row}"))
}

pub fn strings(row: &Value, name: &str) -> Vec<String> {
    array(row, name)
        .iter()
        .map(|item| item.as_str().expect("a string").to_string())
        .collect()
}

pub fn ints(row: &Value, name: &str) -> Vec<i64> {
    array(row, name)
        .iter()
        .map(|item| item.as_i64().expect("an integer"))
        .collect()
}

/// A JSON string for a failure message.
pub fn quoted(text: &str) -> String {
    Value::String(text.to_string()).to_string()
}

// ---- the sentences this port draws -----------------------------------------------------------

/// Every sentence the Linux port asks a catalogue for, scanned out of its own source — the C#
/// suite's `Get("…")` scan, over the convention [`fc_core::text::string_catalog`] states: a
/// sentence is asked of a catalogue named `say` (`say.get("…")`, `say.format("…"`,
/// `say.plural("…"`), so a map's `get("…")` is not a sentence.
pub fn sentences() -> Vec<String> {
    let crates = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut found = std::collections::BTreeSet::new();
    let mut directories = vec![crates];
    while let Some(directory) = directories.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            if path.is_dir() {
                // A crate's sources, never its tests or a build's output.
                if name != "tests" && name != "target" {
                    directories.push(path);
                }
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                // Comments name sentences without drawing them.
                let source: String = std::fs::read_to_string(&path)
                    .expect("a source file")
                    .lines()
                    .filter(|line| !line.trim_start().starts_with("//"))
                    .map(|line| format!("{line}\n"))
                    .collect();
                for call in [".get(", ".format(", ".plural("] {
                    let mut from = 0;
                    while let Some(at) = source[from..].find(call).map(|at| at + from) {
                        from = at + call.len();
                        // `say`, then perhaps a line break before the call, as rustfmt writes a
                        // chain.
                        let receiver = source[..at].trim_end();
                        let named_say = receiver.ends_with("say")
                            && !receiver[..receiver.len() - 3]
                                .ends_with(|c: char| c.is_alphanumeric() || c == '_');
                        if named_say
                            && let Some(literal) = string_literal(source[from..].trim_start())
                        {
                            found.insert(literal);
                        }
                    }
                }
            }
        }
    }
    found.into_iter().collect()
}

/// The ordinary string literal `text` starts with, unescaped.
fn string_literal(text: &str) -> Option<String> {
    let mut rest = text.strip_prefix('"')?;
    let mut out = String::new();
    loop {
        let mut characters = rest.chars();
        let character = characters.next()?;
        rest = characters.as_str();
        match character {
            '"' => return Some(out),
            '\\' => {
                let mut characters = rest.chars();
                let escaped = characters.next()?;
                rest = characters.as_str();
                match escaped {
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    'r' => out.push('\r'),
                    '0' => out.push('\0'),
                    'u' => {
                        let (digits, after) = rest.strip_prefix('{')?.split_once('}')?;
                        out.push(char::from_u32(u32::from_str_radix(digits, 16).ok()?)?);
                        rest = after;
                    }
                    // A line continuation: the newline and the next line's indent are not text.
                    '\n' => rest = rest.trim_start(),
                    other => out.push(other),
                }
            }
            other => out.push(other),
        }
    }
}

// ---- .NET's rounding, for the board's numbers ----------------------------------------------

/// .NET's `Math.Round(value, digits)` — `MidpointRounding.ToEven`, banker's rounding — exactly as
/// the runtime does it: `value * 10^digits` rounded half to even and divided back, and a value of
/// 1e16 or more (or NaN) returned as it is. Rust's `round` is half AWAY from zero, which is not it.
pub fn net_round(value: f64, digits: usize) -> f64 {
    // The runtime's own table: literals, not `powi`, so the scale is exact.
    const POWERS: [f64; 16] = [
        1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15,
    ];
    if value.abs() < 1e16 {
        let power = POWERS[digits];
        (value * power).round_ties_even() / power
    } else {
        value
    }
}

/// xUnit's `Assert.Equal(double expected, double actual, int precision)`, which the C# tests use
/// for every inexact number: both sides rounded to `precision` places by [`net_round`] and then
/// compared exactly (`double.Equals`, so NaN agrees with NaN). It is not a tolerance — two values
/// either side of a rounding midpoint disagree here as they do there.
#[track_caller]
pub fn assert_equal_to_places(expected: f64, actual: f64, precision: usize) {
    let (rounded_expected, rounded_actual) =
        (net_round(expected, precision), net_round(actual, precision));
    assert!(
        rounded_expected == rounded_actual
            || (rounded_expected.is_nan() && rounded_actual.is_nan()),
        "expected {expected}, got {actual}: {rounded_expected} and {rounded_actual} at {precision} places"
    );
}

// ---- Temporary cache files, for the store's tests -------------------------------------------

/// A directory of its own under the system's temp directory (`std::env::temp_dir`, which honours
/// `TMPDIR`), removed with everything in it when dropped — the C# tests' `Path.GetTempPath()` plus
/// a GUID, and their `Dispose`. A file the OS still holds is not a test failure, so a removal that
/// fails is ignored, as theirs is.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub fn new(prefix: &str) -> TempDir {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        let path = std::env::temp_dir().join(format!(
            "{prefix}-{}-{}-{nanos}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).expect("a temp directory");
        TempDir { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// A file in it, not yet created.
    pub fn file(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// A cache file as an OLDER build left it: `statements` run in order on a plain connection — the
/// C# tests' bare `SqliteConnection` — which is closed again before the cache opens the file.
pub fn older_cache<'a>(path: &Path, statements: impl IntoIterator<Item = &'a str>) {
    let connection = rusqlite::Connection::open(path).expect("an older cache file");
    for statement in statements {
        connection
            .execute_batch(statement)
            .unwrap_or_else(|error| panic!("{statement}: {error}"));
    }
}
