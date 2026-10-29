//! `WaveformOracleTests`: a voice note's waveform held to `fc_text::waveform`, case for case:
//! every case in `fixtures/waveform-vectors.json` was printed by the Rust, and the same bytes sit
//! in the iOS test bundle and on Android's JVM classpath. On Linux the waveform IS
//! `fc_text::waveform`, so this holds the `fc_text` this build links to the file the other ports
//! are held to. A case naming a function this suite does not know FAILS.
//!
//! JSON holds no NaN or infinity, so those peaks are the strings "NaN", "Infinity" and
//! "-Infinity". A level is compared as a number and a fraction bit for bit — read from the file's
//! own digits with `str::parse`, which rounds correctly: `serde_json` without its
//! `float_roundtrip` feature can land one ulp away from the double the file names (it does for
//! three of these fractions).
//! (`ATieRoundsUpWhereHalfToEvenWouldNot` is `fc_text::waveform`'s own
//! `a_level_is_4_db_rounded_half_up` and `one_ulp_below_a_tie_rounds_down`.)

mod common;

use std::collections::HashSet;

use serde_json::Value;

use common::{int, string, uint};
use fc_text::waveform;

fn cases() -> Vec<Value> {
    match common::fixture_json("waveform-vectors.json") {
        Value::Array(cases) => cases,
        other => panic!("not a list of cases: {other}"),
    }
}

/// Each case with its own line of the file (the file is one case per line).
fn cases_with_lines() -> Vec<(Value, String)> {
    common::fixture_text("waveform-vectors.json")
        .lines()
        .filter(|line| line.trim_start().starts_with('{'))
        .map(|line| {
            let line = line.trim().trim_end_matches(',').to_string();
            (serde_json::from_str(&line).expect("a case"), line)
        })
        .collect()
}

/// The number written after `"key":` on a line, parsed exactly.
fn exact(line: &str, key: &str) -> f64 {
    let label = format!("\"{key}\":");
    let at = line.find(&label).expect("the key") + label.len();
    let digits: String = line[at..]
        .chars()
        .take_while(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'))
        .collect();
    digits.parse().expect("a number")
}

fn peak(value: &Value) -> f64 {
    match value {
        Value::String(word) => match word.as_str() {
            "NaN" => f64::NAN,
            "Infinity" => f64::INFINITY,
            "-Infinity" => f64::NEG_INFINITY,
            other => panic!("a peak spelled {other}"),
        },
        number => number.as_f64().expect("a peak"),
    }
}

fn levels_of(value: &Value) -> Vec<u8> {
    value
        .as_array()
        .expect("levels")
        .iter()
        .map(|level| level.as_u64().expect("a level").min(255) as u8)
        .collect()
}

fn same_levels(actual: Option<&[u8]>, expected: &Value) -> bool {
    match (actual, expected) {
        (None, Value::Null) => true,
        (Some(actual), expected) if !expected.is_null() => actual == levels_of(expected),
        _ => false,
    }
}

#[test]
fn the_file_is_the_one_the_original_printed() {
    let cases = cases();
    // 401 when this was written; fewer would mean a truncated copy.
    assert!(cases.len() >= 401, "{} cases", cases.len());
    let functions: HashSet<String> = cases.iter().map(|row| string(row, "function")).collect();
    for name in [
        "constants",
        "level",
        "from_peaks",
        "parse",
        "levels_or_placeholder",
        "bars",
        "bar_fraction",
        "played_bars",
    ] {
        assert!(functions.contains(name), "{name}");
    }
}

#[test]
fn every_case_agrees() {
    let cases = cases_with_lines();
    assert_eq!(cases.len(), self::cases().len());
    let mut failures = Vec::new();
    for (row, line) in cases {
        let name = string(&row, "name");
        let input = &row["input"];
        let expected = &row["expected"];
        let failure = match string(&row, "function").as_str() {
            "constants" => (int(expected, "levels") as usize != waveform::LEVELS
                || int(expected, "max_level") != i64::from(waveform::MAX_LEVEL)
                || expected["floor_dbfs"].as_f64() != Some(waveform::FLOOR_DBFS)
                || expected["db_per_level"].as_f64() != Some(waveform::DB_PER_LEVEL)
                || int(expected, "placeholder_level") != i64::from(waveform::PLACEHOLDER_LEVEL)
                || string(expected, "placeholder") != waveform::encode(&waveform::PLACEHOLDER))
            .then(|| "the constants differ".to_string()),
            "level" => {
                let level = waveform::level(peak(&input["dbfs"]));
                (i64::from(level) != int(expected, "level")).then(|| format!("level {level}"))
            }
            "from_peaks" => {
                let samples: Vec<f64> = input["samples_dbfs"]
                    .as_array()
                    .expect("peaks")
                    .iter()
                    .map(peak)
                    .collect();
                let wire = waveform::from_peaks(&samples, uint(input, "levels") as usize);
                (wire != string(expected, "waveform")).then(|| format!("waveform {wire}"))
            }
            "parse" => {
                let parsed = waveform::parse(&string(input, "waveform"));
                (!same_levels(
                    parsed.as_ref().map(|levels| levels.as_slice()),
                    &expected["levels"],
                ))
                .then(|| "parse differs".to_string())
            }
            "levels_or_placeholder" => {
                let shown = waveform::levels_or_placeholder(input["waveform"].as_str());
                (!same_levels(Some(&shown), &expected["levels"]))
                    .then(|| "levels differ".to_string())
            }
            "bars" => {
                let bars =
                    waveform::bars(&levels_of(&input["levels"]), uint(input, "count") as usize);
                (!same_levels(Some(&bars), &expected["bars"])).then(|| format!("bars {bars:?}"))
            }
            "bar_fraction" => {
                let fraction = waveform::bar_fraction(uint(input, "level").min(255) as u8);
                (fraction.to_bits() != exact(&line, "fraction").to_bits())
                    .then(|| format!("fraction {fraction:?}"))
            }
            "played_bars" => {
                let played = waveform::played_bars(
                    uint(input, "position_ms"),
                    uint(input, "duration_ms"),
                    uint(input, "bars") as usize,
                );
                (played as u64 != uint(expected, "played")).then(|| format!("played {played}"))
            }
            other => Some(format!("a function this suite does not know: {other}")),
        };
        if let Some(failure) = failure {
            failures.push(format!("{name}: {failure}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{}",
        failures
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
