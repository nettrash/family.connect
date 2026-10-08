//! A voice note's WAVEFORM (issue #79; docs/protocol.md, "A voice note's
//! waveform"): 48 levels of 0..=15 that the SENDER computes from the peaks it
//! metered while recording, sends on the upload as 48 lowercase hex digits,
//! and every reader draws as bars before a byte of the recording is
//! downloaded.
//!
//! Written once here, because five codebases have to produce the same 48
//! characters for the same recording and draw the same bars for the same 48
//! characters. The web client runs this module as it is; the Apple, Android
//! and Windows ports are held to it by the vectors `win/tools/board-oracle`
//! prints from it (`cargo run -- waveform`), committed as
//! `waveform-vectors.json` beside each port's tests — "an oracle, not four
//! readings".
//!
//! The arithmetic is chosen to be PORTABLE BIT FOR BIT. A level is one IEEE
//! addition, an exact division by 4 (a power of two) and a floor — no
//! logarithm, no `pow`, no rounding mode a platform could disagree about —
//! and every slice boundary is integer arithmetic. A platform whose meter
//! reports linear amplitude (Android's `getMaxAmplitude()`, the web's sample
//! magnitude) takes its own logarithm to reach dBFS first; that is the
//! METER's conversion, outside this rule, and a last-ulp difference there can
//! move a level only at an exact 4 dB boundary.
//!
//! Every function is total: no input panics, and a value a reader cannot
//! parse draws as [`PLACEHOLDER`] rather than as an error.

/// How many levels the wire carries, one hex digit each.
pub const LEVELS: usize = 48;

/// The loudest level: full scale, 0 dBFS, written `f`.
pub const MAX_LEVEL: u8 = 15;

/// Level 0's centre: the same −60 dBFS `fc_text::record` calls digital
/// silence.
pub const FLOOR_DBFS: f64 = -60.0;

/// One level is this many dB: (0 − (−60)) ÷ 15.
pub const DB_PER_LEVEL: f64 = 4.0;

/// The level of every bar of [`PLACEHOLDER`].
pub const PLACEHOLDER_LEVEL: u8 = 4;

/// What a reader draws for audio WITHOUT a waveform — a picked sound file, an
/// old message, an old client's — and for one it cannot parse: a flat row,
/// which claims no shape, rather than an invented one that would.
pub const PLACEHOLDER: [u8; LEVELS] = [PLACEHOLDER_LEVEL; LEVELS];

/// One metered peak, in dBFS, as a level of 0..=15 (protocol.md, step 1).
///
/// `x = (min(max(p, −60), 0) + 60) ÷ 4`, rounded half UP: `⌊x⌋ + 1` when
/// `x − ⌊x⌋ ≥ 0.5`. So 0 is anything below −58 dBFS and 15 anything from
/// −2 dBFS up. A peak that is not a number is silence; `+∞` is 15, `−∞` 0.
///
/// The half-up test is written on the fraction rather than as
/// `⌊x + 0.5⌋`, which rounds 0.49999999999999994 UP in IEEE arithmetic —
/// a value no dBFS input here can produce, but a port that wrote it the
/// other way would be a different rule.
pub fn level(dbfs: f64) -> u8 {
    if dbfs.is_nan() {
        return 0;
    }
    let clamped = dbfs.clamp(FLOOR_DBFS, 0.0);
    let x = (clamped - FLOOR_DBFS) / DB_PER_LEVEL;
    let whole = x.floor();
    let rounded = if x - whole >= 0.5 { whole + 1.0 } else { whole };
    // 0.0..=15.0 by construction; the min is belt and braces.
    (rounded as u8).min(MAX_LEVEL)
}

/// The slice rule (protocol.md, step 2), for `n` items into `count` slices:
/// slice `i` covers items `s = ⌊i × n ÷ count⌋` up to, not including,
/// `max(s + 1, ⌊(i + 1) × n ÷ count⌋)`. Fewer items than slices is allowed
/// — an item then covers several slices. Callers guarantee `n ≥ 1`.
fn slice(i: usize, n: usize, count: usize) -> std::ops::Range<usize> {
    let at = |k: usize| ((k as u128 * n as u128) / count as u128) as usize;
    let start = at(i);
    let end = at(i + 1).max(start + 1);
    start..end
}

/// Reduce `levels` to `count` levels, each the HIGHEST of the levels its
/// slice covers. No levels at all is `count` zeros (silence); `count` 0 is
/// empty. Levels above 15 are read as 15.
///
/// This is both step 2 of the sender's computation (over the per-peak
/// levels) and what a reader does to draw 48 levels as however many bars fit
/// its bubble ([`bars`]).
pub fn reduce(levels: &[u8], count: usize) -> Vec<u8> {
    if levels.is_empty() {
        return vec![0; count];
    }
    (0..count)
        .map(|i| {
            levels[slice(i, levels.len(), count)]
                .iter()
                .copied()
                .max()
                .unwrap_or(0)
                .min(MAX_LEVEL)
        })
        .collect()
}

/// Levels as the wire writes them: one lowercase hex digit each, the first
/// slice first. A level above 15 is written `f`.
pub fn encode(levels: &[u8]) -> String {
    levels
        .iter()
        .map(|&level| char::from_digit(u32::from(level.min(MAX_LEVEL)), 16).unwrap_or('f'))
        .collect()
}

/// The waveform a sender uploads: every metered peak (dBFS, at a fixed
/// interval, in time order) as a [`level`], [`reduce`]d to `levels` slices
/// and [`encode`]d. The wire takes exactly [`LEVELS`]; other counts are for
/// tests and for a live meter that draws fewer.
///
/// No peaks at all is all zeros — nothing was heard — which is a valid
/// waveform; the composer never sends a recording that short anyway.
pub fn from_peaks(samples_dbfs: &[f64], levels: usize) -> String {
    let per_peak: Vec<u8> = samples_dbfs.iter().map(|&peak| level(peak)).collect();
    encode(&reduce(&per_peak, levels))
}

/// A waveform as the wire spells one — EXACTLY [`LEVELS`] lowercase hex
/// digits and nothing else — as its levels, or `None`. The server refuses
/// anything else with `validation`, so a reader should never meet a value
/// this rejects; when it does, it draws [`PLACEHOLDER`] ([`levels_or_placeholder`]).
pub fn parse(waveform: &str) -> Option<[u8; LEVELS]> {
    let bytes = waveform.as_bytes();
    if bytes.len() != LEVELS {
        return None;
    }
    let mut levels = [0u8; LEVELS];
    for (slot, &byte) in levels.iter_mut().zip(bytes) {
        *slot = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            _ => return None,
        };
    }
    Some(levels)
}

/// What a reader draws for an attachment's `waveform` field: its levels, or
/// [`PLACEHOLDER`] when it is absent or unparseable.
pub fn levels_or_placeholder(waveform: Option<&str>) -> [u8; LEVELS] {
    waveform.and_then(parse).unwrap_or(PLACEHOLDER)
}

/// The 48 levels as `count` bars — [`reduce`], named for the reader.
pub fn bars(levels: &[u8], count: usize) -> Vec<u8> {
    reduce(levels, count)
}

/// A bar's height as a fraction of the waveform's: `(2 + level) ÷ 17`, so
/// silence is still a visible stub (2/17 ≈ 12 %) and 15 is the full height.
/// Above 15 reads as 15.
pub fn bar_fraction(level: u8) -> f64 {
    f64::from(2 + level.min(MAX_LEVEL)) / 17.0
}

/// How many of `bars` bars are drawn as PLAYED (the accent colour) at
/// `position_ms` into a recording of `duration_ms`:
/// `⌊position_ms × bars ÷ duration_ms⌋`, at most `bars`. A recording of no
/// known length has none played. Integer arithmetic, so a bar lights at the
/// same millisecond everywhere.
pub fn played_bars(position_ms: u64, duration_ms: u64, bars: usize) -> usize {
    if duration_ms == 0 {
        return 0;
    }
    let played = (u128::from(position_ms) * bars as u128) / u128::from(duration_ms);
    played.min(bars as u128) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_constants() {
        assert_eq!(LEVELS, 48);
        assert_eq!(MAX_LEVEL, 15);
        assert_eq!(FLOOR_DBFS, crate::record::SILENCE_PEAK_DBFS);
        assert_eq!(DB_PER_LEVEL * f64::from(MAX_LEVEL), -FLOOR_DBFS);
        assert_eq!(encode(&PLACEHOLDER), "4".repeat(48));
    }

    #[test]
    fn a_level_is_4_db_rounded_half_up() {
        assert_eq!(level(-60.0), 0);
        assert_eq!(level(-90.0), 0);
        assert_eq!(level(-160.0), 0);
        assert_eq!(level(-58.0 - 1.0 / 65_536.0), 0);
        assert_eq!(level(-58.0), 1, "a tie rounds up");
        assert_eq!(level(-56.0), 1);
        assert_eq!(level(-54.0), 2);
        assert_eq!(level(-30.0), 8, "x = 7.5");
        assert_eq!(level(-2.0 - 1.0 / 65_536.0), 14);
        assert_eq!(level(-2.0), 15);
        assert_eq!(level(0.0), 15);
        assert_eq!(level(-0.0), 15);
        assert_eq!(level(3.5), 15, "above full scale is full scale");
        assert_eq!(level(f64::NAN), 0);
        assert_eq!(level(f64::INFINITY), 15);
        assert_eq!(level(f64::NEG_INFINITY), 0);
        for k in 0..=15u8 {
            assert_eq!(level(FLOOR_DBFS + DB_PER_LEVEL * f64::from(k)), k);
        }
    }

    /// The tie's two sides, one ulp apart: the next double below −58 is
    /// level 0, −58 itself is 1.
    #[test]
    fn one_ulp_below_a_tie_rounds_down() {
        let below = f64::from_bits((-58.0_f64).to_bits() + 1);
        assert!(below < -58.0);
        assert_eq!(level(below), 0);
        assert_eq!(level(-58.0), 1);
        // Near −2 the ADDITION decides: −2 − 2⁻⁵¹ + 60 is not representable
        // and rounds to exactly 58, a tie, so it is 15 — on every platform,
        // because IEEE addition is correctly rounded. The vectors carry it
        // so that a port computing anything but exactly this double-precision
        // addition is found out, whichever way its answer goes.
        let below = f64::from_bits((-2.0_f64).to_bits() + 1);
        assert_eq!(level(below), 15);
    }

    #[test]
    fn slices_cover_every_peak_once_when_there_are_enough() {
        for n in [48usize, 49, 95, 96, 100, 1000, 30_000] {
            let mut next = 0;
            for i in 0..LEVELS {
                let range = slice(i, n, LEVELS);
                assert_eq!(range.start, next, "n = {n}, slice {i}");
                assert!(range.end > range.start);
                next = range.end;
            }
            assert_eq!(next, n);
        }
    }

    #[test]
    fn fewer_peaks_than_slices_stretch() {
        assert_eq!(from_peaks(&[], LEVELS), "0".repeat(48));
        assert_eq!(from_peaks(&[0.0], LEVELS), "f".repeat(48));
        assert_eq!(
            from_peaks(&[-60.0, 0.0], LEVELS),
            format!("{}{}", "0".repeat(24), "f".repeat(24))
        );
        let three = from_peaks(&[-60.0, -30.0, 0.0], LEVELS);
        assert_eq!(
            three,
            format!("{}{}{}", "0".repeat(16), "8".repeat(16), "f".repeat(16))
        );
        assert_eq!(from_peaks(&[-30.0], 0), "");
    }

    #[test]
    fn a_slice_takes_its_loudest_peak() {
        let mut peaks = vec![-60.0; 96];
        peaks[1] = 0.0; // slice 0 is peaks 0 and 1
        peaks[94] = -30.0; // slice 47 is peaks 94 and 95
        let wire = from_peaks(&peaks, LEVELS);
        assert_eq!(wire.len(), 48);
        assert_eq!(&wire[..1], "f");
        assert_eq!(&wire[1..47], "0".repeat(46));
        assert_eq!(&wire[47..], "8");
    }

    #[test]
    fn parse_takes_exactly_the_wire() {
        let wire = "0123456789abcdef0123456789abcdef0123456789abcdef";
        let levels = parse(wire).expect("valid");
        assert_eq!(levels[0], 0);
        assert_eq!(levels[15], 15);
        assert_eq!(encode(&levels), wire, "parse and encode are inverses");
        for refused in [
            String::new(),
            "0".repeat(47),
            "0".repeat(49),
            wire.to_uppercase(),
            format!("{}g", &wire[..47]),
            format!("{} ", &wire[..47]),
            format!("{}é", &wire[..46]),
            format!("{}٣", &wire[..47]),
        ] {
            assert_eq!(parse(&refused), None, "{refused:?}");
            assert_eq!(levels_or_placeholder(Some(&refused)), PLACEHOLDER);
        }
        assert_eq!(levels_or_placeholder(None), PLACEHOLDER);
        assert_eq!(levels_or_placeholder(Some(wire)), levels);
    }

    #[test]
    fn bars_reduce_and_stretch() {
        let levels = parse("0123456789abcdef0123456789abcdef0123456789abcdef").unwrap();
        assert_eq!(bars(&levels, 48), levels.to_vec());
        assert_eq!(bars(&levels, 1), vec![15]);
        assert_eq!(bars(&levels, 3), vec![15, 15, 15]);
        assert_eq!(bars(&levels, 0), Vec::<u8>::new());
        let wide = bars(&levels, 96);
        assert_eq!(wide.len(), 96);
        assert_eq!(&wide[..4], &[0, 0, 1, 1]);
        assert_eq!(reduce(&[200], 2), vec![15, 15], "above 15 reads as 15");
        assert_eq!(encode(&[16, 255]), "ff");
    }

    #[test]
    fn bar_heights_and_played_bars() {
        assert_eq!(bar_fraction(0), 2.0 / 17.0);
        assert_eq!(bar_fraction(15), 1.0);
        assert_eq!(bar_fraction(99), 1.0);
        assert_eq!(played_bars(0, 14_200, 44), 0);
        assert_eq!(played_bars(7_100, 14_200, 44), 22);
        assert_eq!(played_bars(14_199, 14_200, 44), 43);
        assert_eq!(played_bars(14_200, 14_200, 44), 44);
        assert_eq!(played_bars(99_999, 14_200, 44), 44);
        assert_eq!(played_bars(5_000, 0, 44), 0);
        assert_eq!(played_bars(u64::MAX, u64::MAX - 1, 48), 48);
    }
}
