//! `VoiceLookTests`: a voice message as the approved design of 2026-10-05 draws it — the waveform
//! the sender measured (or a neutral one), the played bars, the time, the dot, the speed chip and
//! what a screen reader hears — and the waveform a recording made here is given, measured from its
//! own decoded sound.

mod common;

use fc_core::protocol::AttachmentDto;
use fc_logic::voice_look::{self, ChipFit, LiveLevels, voice_speed, voice_waveform};
use fc_text::record::MIN_TARGET_WINDOWS_EPX;
use fc_text::waveform;

const WIRE: &str = "0123456789abcdef0123456789abcdef0123456789abcdef";

/// `new AttachmentDto(40, "audio", "audio/mp4", 9000, DurationMs: 42_400, Waveform: Wire)`.
fn note() -> AttachmentDto {
    AttachmentDto {
        mime: Some("audio/mp4".to_string()),
        size: Some(9000),
        duration_ms: Some(42_400),
        waveform: Some(WIRE.to_string()),
        ..AttachmentDto::new(40, "audio")
    }
}

/// xUnit's `Assert.Equal(expected, actual, precision)`: both rounded to `places` decimals.
fn assert_near(expected: f64, actual: f64, places: i32) {
    let scale = 10f64.powi(places);
    assert_eq!(
        (expected * scale).round_ties_even(),
        (actual * scale).round_ties_even(),
        "{expected} vs {actual}"
    );
}

#[test]
fn the_bubble_holds_forty_two_bars_of_the_senders_waveform() {
    assert_eq!(210.0, voice_look::WAVE_WIDTH);
    assert_eq!(42, voice_look::bar_count(voice_look::WAVE_WIDTH));
    assert_eq!(48, voice_look::bar_count(48.0 * 5.0 - 2.0));
    assert_eq!(0, voice_look::bar_count(2.0));
    assert_eq!(0, voice_look::bar_count(f64::NAN));
    let bars = voice_look::bars(note().waveform.as_deref(), 46);
    assert_eq!(46, bars.len());
    assert_eq!(waveform::bars(&waveform::parse(WIRE).unwrap(), 46), bars);
}

/// A picked sound file, an older client's note, a malformed field: the neutral shape, never an
/// invented one. A `[Theory]`: one test over its rows.
#[test]
fn no_waveform_is_the_neutral_placeholder() {
    for waveform in [
        None,
        Some(""),
        Some("0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF"),
        Some("0123"),
    ] {
        let bars = voice_look::bars(waveform, 46);
        assert_eq!(46, bars.len());
        assert!(
            bars.iter()
                .all(|&level| level == waveform::PLACEHOLDER_LEVEL),
            "{waveform:?}"
        );
    }
}

#[test]
fn a_bar_is_two_seventeenths_at_silence_and_whole_at_full_scale() {
    assert_near(28.0 * 2.0 / 17.0, voice_look::bar_height(0, 28.0), 9);
    assert_near(28.0, voice_look::bar_height(15, 28.0), 9);
    assert_eq!(0.0, voice_look::bar_height(15, -3.0));
}

/// The played bars follow the shared rule on whole milliseconds; an unknown length lights none.
#[test]
fn played_bars_are_the_shared_rule() {
    assert_eq!(0, voice_look::played_bars(0.0, 42.4, 46));
    assert_eq!(23, voice_look::played_bars(21.2, 42.4, 46));
    assert_eq!(46, voice_look::played_bars(42.4, 42.4, 46));
    assert_eq!(46, voice_look::played_bars(99.0, 42.4, 46));
    assert_eq!(0, voice_look::played_bars(-3.0, 42.4, 46));
    assert_eq!(0, voice_look::played_bars(5.0, 0.0, 46));
    assert_eq!(0, voice_look::played_bars(f64::NAN, 42.4, 46));
    assert_eq!(
        waveform::played_bars(14_199, 14_200, 44),
        voice_look::played_bars(14.199, 14.2, 44)
    );
}

/// The time counts up while it plays (or is paused part way) and is the whole length at rest.
#[test]
fn the_time_is_where_it_is_while_playing_and_the_length_at_rest() {
    assert_eq!("0:42", voice_look::time(false, 12.0, 42.4));
    assert_eq!("0:12", voice_look::time(true, 12.2, 42.4));
    assert_eq!("0:42", voice_look::time(true, 99.0, 42.4));
    assert_eq!("0:00", voice_look::time(true, -1.0, 42.4));
}

/// Not in the C# suite: the time keeps .NET's NaN rules (measured on .NET 10 with the C#'s own
/// `Math.Clamp(position, 0, Math.Max(0, total))`): an unknown total bounds nothing, a position that
/// is not a number reads "0:00", and a negative total clamps to the start.
#[test]
fn the_time_keeps_dotnets_rules_for_what_is_not_a_number() {
    assert_eq!("0:12", voice_look::time(true, 12.0, f64::NAN));
    assert_eq!("0:00", voice_look::time(true, f64::NAN, 42.0));
    assert_eq!("0:00", voice_look::time(true, 50.0, -3.0));
}

/// A `[Theory]`: one test over its rows.
#[test]
fn the_dot_and_its_status_are_only_on_someone_elses() {
    let say = common::english();
    for (mine, played, dot, status) in [
        (false, false, true, Some("Not played")),
        (false, true, false, Some("Played")),
        (true, false, false, None),
        (true, true, false, None),
    ] {
        assert_eq!(dot, voice_look::shows_dot(mine, played), "{mine} {played}");
        assert_eq!(
            status.map(str::to_string),
            voice_look::status(mine, played, &say),
            "{mine} {played}"
        );
    }
}

#[test]
fn a_screen_reader_hears_what_it_is_and_how_long() {
    let say = common::english();
    assert_eq!("Voice message, 0:42", voice_look::name(&note(), &say));
    assert_eq!(
        "Voice message, 0:00",
        voice_look::name(
            &AttachmentDto {
                duration_ms: None,
                ..note()
            },
            &say
        )
    );
    assert!(voice_look::shows_speed(true));
    assert!(!voice_look::shows_speed(false));
}

/// 1× → 1.5× → 2× → 1×, stored invariantly on the device, and anything unreadable is 1×.
#[test]
fn the_speed_goes_round_and_is_remembered_invariantly() {
    let say = common::english();
    assert_eq!(1.5, voice_speed::next(1.0));
    assert_eq!(2.0, voice_speed::next(1.5));
    assert_eq!(1.0, voice_speed::next(2.0));
    assert_eq!(1.0, voice_speed::next(3.0));
    assert_eq!("1×", voice_speed::label(1.0, &say));
    assert_eq!("1.5×", voice_speed::label(1.5, &say));
    assert_eq!("2×", voice_speed::label(2.0, &say));
    assert_eq!("1×", voice_speed::label(0.75, &say));
    assert_eq!("Playback speed, 1.5×", voice_speed::name(1.5, &say));
    assert_eq!("1.5", voice_speed::store(1.5));
    assert_eq!("2", voice_speed::store(2.0));
    assert_eq!(1.5, voice_speed::parse(Some("1.5")));
    assert_eq!(2.0, voice_speed::parse(Some(" 2\n")));
    assert_eq!(1.0, voice_speed::parse(Some("1,5")));
    assert_eq!(1.0, voice_speed::parse(Some("3")));
    assert_eq!(1.0, voice_speed::parse(None));
    assert_eq!(1.0, voice_speed::parse(Some("fast")));
}

/// The live line grows in from the right, the newest bar last, and the oldest falls off the left.
#[test]
fn the_live_waveform_scrolls_in_from_the_right() {
    let mut live = LiveLevels::new(4);
    assert_eq!(vec![0, 0, 0], live.bars(3));
    live.push(-60.0);
    live.push(0.0);
    assert_eq!(vec![0, 0, 15], live.bars(3));
    assert_eq!(vec![0, 0, 0, 0, 15], live.bars(5));
    live.push(-30.0);
    live.push(-58.0);
    live.push(f64::NEG_INFINITY);
    assert_eq!(5, live.heard());
    assert_eq!(vec![15, 8, 1, 0], live.bars(4));
    assert_eq!(vec![1, 0], live.bars(2));
    live.clear();
    assert_eq!(0, live.heard());
    assert_eq!(vec![0, 0], live.bars(2));
}

// ---- the waveform a recording made here is given --------------------------------------------

fn wav(samples: &[i16]) -> Vec<u8> {
    wav_at(samples, 1000, 1)
}

fn wav_at(samples: &[i16], rate: i32, channels: i32) -> Vec<u8> {
    let data: Vec<u8> = samples
        .iter()
        .flat_map(|sample| sample.to_le_bytes())
        .collect();
    riff(1, 16, rate, channels, &data, false)
}

/// The C# helper's `BinaryWriter`: little-endian, `Write(int)` four bytes, `Write(short)` two.
fn riff(
    format: i16,
    bits: i16,
    rate: i32,
    channels: i32,
    data: &[u8],
    list_first: bool,
) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&0i32.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16i32.to_le_bytes());
    out.extend_from_slice(&format.to_le_bytes());
    out.extend_from_slice(&(channels as i16).to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * channels * i32::from(bits) / 8).to_le_bytes());
    out.extend_from_slice(&((channels * i32::from(bits) / 8) as i16).to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    if list_first {
        // An odd-sized chunk before the data, padded to even as RIFF requires.
        out.extend_from_slice(b"LIST");
        out.extend_from_slice(&3i32.to_le_bytes());
        out.extend_from_slice(b"abc");
        out.push(0);
    }
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as i32).to_le_bytes());
    out.extend_from_slice(data);
    out
}

/// One peak per tenth of a second, the loudest sample of either channel, in dBFS — then the shared
/// rule. At 1 000 samples a second a block is 100 samples: full scale, half scale (−6.02 dB: x =
/// 13.49, level 13), and silence.
#[test]
fn a_recordings_peaks_are_its_loudest_sample_per_tenth_of_a_second() {
    let mut samples = vec![0i16; 300];
    samples[10] = i16::MIN;
    samples[150] = 16384;
    samples[151] = -100;
    let peaks = voice_waveform::peaks(&wav(&samples)).unwrap();
    assert_eq!(3, peaks.len());
    assert_near(0.0, peaks[0], 9);
    assert_near(20.0 * 0.5f64.log10(), peaks[1], 9);
    assert_eq!(f64::NEG_INFINITY, peaks[2]);
    assert_eq!(
        Some(waveform::from_peaks(&peaks, waveform::LEVELS)),
        voice_waveform::from_wav(&wav(&samples))
    );
    assert_eq!(
        Some(format!(
            "{}{}{}",
            "f".repeat(16),
            "d".repeat(16),
            "0".repeat(16)
        )),
        voice_waveform::from_wav(&wav(&samples))
    );
}

/// A block is exactly a tenth of a second: the first sample of the next one is never counted in
/// this one.
#[test]
fn a_block_ends_exactly_at_a_tenth_of_a_second() {
    let mut samples = vec![0i16; 200];
    samples[100] = 16384;
    let peaks = voice_waveform::peaks(&wav(&samples)).unwrap();
    assert_eq!(2, peaks.len());
    assert_eq!(f64::NEG_INFINITY, peaks[0]);
    assert_near(20.0 * 0.5f64.log10(), peaks[1], 9);
    // At 16 kHz, the rate the decode asks for, a block is 1 600 frames.
    let mut sixteen = vec![0i16; 3200];
    sixteen[1600] = 32767;
    assert_eq!(
        f64::NEG_INFINITY,
        voice_waveform::peaks(&wav_at(&sixteen, 16_000, 1)).unwrap()[0]
    );
}

#[test]
fn a_tail_shorter_than_a_block_is_a_peak_of_its_own_and_stereo_takes_either_channel() {
    // Two channels, 150 frames: a block of 100 and a tail of 50; the right channel is the loud one.
    let mut samples = vec![0i16; 300];
    samples[2 * 120 + 1] = 32767;
    let peaks = voice_waveform::peaks(&wav_at(&samples, 1000, 2)).unwrap();
    assert_eq!(2, peaks.len());
    assert_eq!(f64::NEG_INFINITY, peaks[0]);
    assert_near(20.0 * (32767.0f64 / 32768.0).log10(), peaks[1], 9);
}

#[test]
fn float_samples_and_chunks_before_the_data_are_read() {
    let mut data = vec![0u8; 200 * 4];
    data[4 * 5..4 * 5 + 4].copy_from_slice(&(-0.25f32).to_le_bytes());
    data[4 * 150..4 * 150 + 4].copy_from_slice(&2.0f32.to_le_bytes());
    let peaks = voice_waveform::peaks(&riff(3, 32, 1000, 1, &data, true)).unwrap();
    assert_eq!(2, peaks.len());
    assert_near(20.0 * 0.25f64.log10(), peaks[0], 9);
    // Above full scale is full scale.
    assert_near(0.0, peaks[1], 9);
}

/// Anything this cannot read is no waveform — the note goes without one and readers draw the
/// placeholder.
#[test]
fn what_cannot_be_read_is_no_waveform() {
    assert_eq!(None, voice_waveform::from_wav(&[]));
    assert_eq!(None, voice_waveform::from_wav(b"RIFF\0\0\0\0WAVE"));
    assert_eq!(None, voice_waveform::from_wav(&wav(&[])));
    assert_eq!(
        None,
        voice_waveform::from_wav(&riff(1, 8, 1000, 1, &[0; 100], false))
    );
    assert_eq!(
        None,
        voice_waveform::from_wav(&riff(2, 16, 1000, 1, &[0; 100], false))
    );
    let mut no_riff = wav(&[0; 100]);
    no_riff[0] = b'X';
    assert_eq!(None, voice_waveform::from_wav(&no_riff));
    assert_eq!(f64::NEG_INFINITY, voice_waveform::dbfs(0.0));
    assert_eq!(f64::NEG_INFINITY, voice_waveform::dbfs(f64::NAN));
    assert_eq!(0.0, voice_waveform::dbfs(3.0));
}

// ---- the not-sent chip at its width ---------------------------------------------------------

/// Room to spare: the mockup's 22 bars, "Not sent" on the line.
#[test]
fn a_wide_not_sent_chip_draws_the_mockups_bars_on_one_line() {
    let fit = voice_look::fit_not_sent(342.0, 100.0, 55.0, 10.0, 22);
    assert_eq!(
        ChipFit {
            label_above: false,
            bars: 22
        },
        fit
    );
}

/// German's "Nicht gesendet" and "Senden": fewer bars, the WHOLE shape reduced to them — never 22
/// bars cut off at the column's edge, which showed only part of the shape.
#[test]
fn a_longer_language_gets_fewer_bars_not_a_cut_off_shape() {
    let fit = voice_look::fit_not_sent(342.0, 150.0, 95.0, 10.0, 22);
    assert!(!fit.label_above);
    assert!(fit.bars < 22);
    assert!(fit.bars >= voice_look::MIN_CHIP_BARS);
    let room = 342.0 - 150.0 - 95.0 - 5.0 * 10.0;
    let bars = fit.bars as f64;
    assert!(bars * voice_look::BAR_WIDTH + (bars - 1.0) * voice_look::BAR_GAP <= room);
    assert_eq!(voice_look::bar_count(room), fit.bars);
}

/// At 150–200 % text the fixed columns alone overflow: "Not sent" goes above, and ✕ is never pushed
/// out.
#[test]
fn large_text_moves_not_sent_above_the_line() {
    let fit = voice_look::fit_not_sent(342.0, 240.0, 150.0, 10.0, 22);
    assert!(fit.label_above);
    let room = 342.0 - 240.0 - 4.0 * 10.0;
    assert_eq!(voice_look::bar_count(room), fit.bars);
    // Even with no room for a single bar, the line still holds ▶, the length, Send and ✕: the
    // waveform gives way.
    assert_eq!(
        ChipFit {
            label_above: true,
            bars: 0
        },
        voice_look::fit_not_sent(342.0, 330.0, 150.0, 10.0, 22)
    );
}

/// Across every width, the bars drawn always fit the room they are given, and never exceed the
/// design's.
#[test]
fn the_chips_bars_always_fit_their_column() {
    for width in 0..=400 {
        for label in [40.0, 90.0, 160.0] {
            let fit = voice_look::fit_not_sent(f64::from(width), 120.0, label, 10.0, 22);
            let room = f64::from(width)
                - 120.0
                - if fit.label_above {
                    4.0 * 10.0
                } else {
                    label + 5.0 * 10.0
                };
            assert!(fit.bars <= 22, "{width} {label}");
            if fit.bars > 0 {
                let bars = fit.bars as f64;
                assert!(
                    bars * voice_look::BAR_WIDTH + (bars - 1.0) * voice_look::BAR_GAP <= room,
                    "{width} {label}"
                );
            }
        }
    }
}

// ---- targets, the idle seek, a note on its way ----------------------------------------------

/// S1.1: a 44 target round a smaller visual, reached by a negative margin — the row it sits in no
/// taller. A `[Theory]`: one test over its rows.
#[test]
fn a_small_control_reaches_a_forty_four_target() {
    for (visual, reach) in [
        (19.0, -12.5),
        (28.0, -8.0),
        (32.0, -6.0),
        (44.0, 0.0),
        (64.0, 0.0),
    ] {
        assert_eq!(reach, voice_look::reach(visual), "{visual}");
        assert_eq!(
            f64::max(visual, f64::from(MIN_TARGET_WINDOWS_EPX)),
            visual - 2.0 * voice_look::reach(visual),
            "{visual}"
        );
    }
}

/// A note SEEKED while idle starts where it was put — not from 0:00, which made the seek do nothing
/// anyone could see — and one seeked to its very end starts from the top.
#[test]
fn play_starts_where_an_idle_note_was_seeked() {
    assert_eq!(12.5, voice_look::start_at(Some(12.5), 42.4));
    assert_eq!(0.0, voice_look::start_at(None, 42.4));
    assert_eq!(0.0, voice_look::start_at(Some(0.0), 42.4));
    assert_eq!(0.0, voice_look::start_at(Some(42.3), 42.4));
    assert_eq!(0.0, voice_look::start_at(Some(-1.0), 42.4));
    assert_eq!(0.0, voice_look::start_at(Some(f64::NAN), 42.4));
    assert_eq!(0.0, voice_look::start_at(Some(5.0), 0.0));
    // The bubble seeked to 12.5 of 42.4 shows that much lit and that time, as it will when it
    // plays.
    assert_eq!(12, voice_look::played_bars(12.5, 42.4, 42));
    assert_eq!("0:13", voice_look::time(true, 12.5, 42.4));
}

/// A voice note going up is drawn as the bubble it will be — but only a lone note with no words.
#[test]
fn only_a_lone_wordless_note_on_its_way_is_a_pending_voice_bubble() {
    assert!(voice_look::is_pending_voice(Some(""), &["audio"]));
    assert!(voice_look::is_pending_voice(None, &["audio"]));
    assert!(!voice_look::is_pending_voice(Some("hello"), &["audio"]));
    assert!(!voice_look::is_pending_voice(Some(""), &["audio", "photo"]));
    assert!(!voice_look::is_pending_voice(Some(""), &["photo"]));
    assert!(!voice_look::is_pending_voice::<&str>(Some(""), &[]));
}
