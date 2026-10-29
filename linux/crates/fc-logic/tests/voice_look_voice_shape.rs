//! The waveform a recording made here is sent with, through the shell's decoder — Windows'
//! `VoiceShape` (`MeasureAsync`, `ShapedAsync`), which lives in the app and has no C# test: the
//! note is decoded (`VoiceDecoder`, faked here), its peak per tenth of a second taken, and the
//! shared rule makes the 48 digits; anything that goes wrong, or a decode longer than the
//! 15-second ceiling, is a note WITHOUT a waveform — never a note that does not go. Time is the
//! fake clock's.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use fc_core::rt::{Clock, LocalBoxFuture};
use fc_core::testing::{FakeClock, drive};
use fc_logic::media_outbox::StagedMedia;
use fc_logic::voice_look::{Pcm, VoiceDecoder, voice_waveform};
use time::macros::datetime;

/// A decoder that answers what it was given after `takes` on the clock, and counts its calls.
struct Decoder {
    clock: FakeClock,
    takes: Duration,
    answer: Option<Pcm>,
    asked: Cell<usize>,
    bytes: RefCell<Vec<Vec<u8>>>,
}

impl Decoder {
    fn new(clock: &FakeClock, takes: Duration, answer: Option<Pcm>) -> Self {
        Decoder {
            clock: clock.clone(),
            takes,
            answer,
            asked: Cell::new(0),
            bytes: RefCell::default(),
        }
    }
}

impl VoiceDecoder for Decoder {
    fn decode(&self, m4a: Rc<[u8]>) -> LocalBoxFuture<'static, Option<Pcm>> {
        self.asked.set(self.asked.get() + 1);
        self.bytes.borrow_mut().push(m4a.to_vec());
        let sleep = self.clock.sleep(self.takes);
        let answer = self.answer.clone();
        Box::pin(async move {
            sleep.await;
            answer
        })
    }
}

fn clock() -> FakeClock {
    FakeClock::at(datetime!(2026-10-05 10:00 UTC))
}

/// Three tenths of a second at 1 kHz: full scale, half scale and silence — "f…d…0…".
fn speech() -> Pcm {
    let mut samples = vec![0i16; 300];
    samples[10] = i16::MIN;
    samples[150] = 16384;
    Pcm {
        rate: 1000,
        channels: 1,
        samples,
    }
}

fn recorded() -> StagedMedia {
    StagedMedia {
        duration_ms: Some(3000),
        ..StagedMedia::new("audio", "audio/mp4", vec![1, 2, 3, 4])
    }
}

/// Decoded PCM is measured exactly as the same samples in a WAV are.
#[test]
fn decoded_samples_give_the_digits_their_wav_gives() {
    assert_eq!(
        Some(format!(
            "{}{}{}",
            "f".repeat(16),
            "d".repeat(16),
            "0".repeat(16)
        )),
        voice_waveform::from_pcm(&speech())
    );
    let stereo = Pcm {
        rate: 1000,
        channels: 2,
        samples: {
            let mut samples = vec![0i16; 300];
            samples[2 * 120 + 1] = 32767;
            samples
        },
    };
    let peaks = voice_waveform::pcm_peaks(&stereo).unwrap();
    assert_eq!(2, peaks.len());
    assert_eq!(f64::NEG_INFINITY, peaks[0]);
    assert!((peaks[1] - 20.0 * (32767.0f64 / 32768.0).log10()).abs() < 1e-9);
    assert_eq!(None, voice_waveform::from_pcm(&Pcm::default()));
    assert_eq!(
        None,
        voice_waveform::from_pcm(&Pcm {
            rate: 16_000,
            channels: 1,
            samples: Vec::new(),
        })
    );
    assert_eq!(
        None,
        voice_waveform::from_pcm(&Pcm {
            rate: 0,
            channels: 1,
            samples: vec![1, 2, 3],
        })
    );
}

/// A recording with no shape yet is decoded and goes with its 48 digits.
#[test]
fn a_recording_is_shaped_from_its_own_sound() {
    let clock = clock();
    let decoder = Decoder::new(&clock, Duration::from_secs(2), Some(speech()));
    let shaped = drive(&clock, voice_waveform::shaped(recorded(), &decoder, &clock));
    assert_eq!(1, decoder.asked.get());
    assert_eq!(vec![vec![1, 2, 3, 4]], *decoder.bytes.borrow());
    assert_eq!(
        Some(format!(
            "{}{}{}",
            "f".repeat(16),
            "d".repeat(16),
            "0".repeat(16)
        )),
        shaped.waveform
    );
    assert_eq!(
        StagedMedia {
            waveform: None,
            ..shaped
        },
        recorded()
    );
}

/// Only a sound without a shape is measured: a note that has one, a photo, a video go as they are,
/// and nothing is decoded.
#[test]
fn only_a_sound_without_a_shape_is_measured() {
    let clock = clock();
    let decoder = Decoder::new(&clock, Duration::ZERO, Some(speech()));
    let has = StagedMedia {
        waveform: Some("0".repeat(48)),
        ..recorded()
    };
    assert_eq!(
        has,
        drive(
            &clock,
            voice_waveform::shaped(has.clone(), &decoder, &clock)
        )
    );
    let photo = StagedMedia::new("photo", "image/jpeg", vec![0xFF, 0xD8]);
    assert_eq!(
        photo,
        drive(
            &clock,
            voice_waveform::shaped(photo.clone(), &decoder, &clock)
        )
    );
    assert_eq!(0, decoder.asked.get());
}

/// A decode that fails, a note of no bytes, or one that decodes to nothing: the note goes without
/// a waveform, and every reader draws the placeholder.
#[test]
fn what_cannot_be_measured_goes_without_a_shape() {
    let clock = clock();
    let failing = Decoder::new(&clock, Duration::from_secs(1), None);
    assert_eq!(
        recorded(),
        drive(&clock, voice_waveform::shaped(recorded(), &failing, &clock))
    );
    let empty = StagedMedia::new("audio", "audio/mp4", Vec::new());
    let decoder = Decoder::new(&clock, Duration::ZERO, Some(speech()));
    assert_eq!(
        empty,
        drive(
            &clock,
            voice_waveform::shaped(empty.clone(), &decoder, &clock)
        )
    );
    assert_eq!(0, decoder.asked.get(), "no bytes, nothing to decode");
    let silent = Decoder::new(&clock, Duration::ZERO, Some(Pcm::default()));
    assert_eq!(
        recorded(),
        drive(&clock, voice_waveform::shaped(recorded(), &silent, &clock))
    );
}

/// The ceiling: a decode still running after fifteen seconds is let go — dropped, which cancels
/// it — and the note goes without a shape; one done just inside it counts.
#[test]
fn a_decode_past_the_ceiling_is_let_go() {
    let clock = clock();
    let slow = Decoder::new(&clock, Duration::from_secs(16), Some(speech()));
    assert_eq!(
        None,
        drive(
            &clock,
            voice_waveform::measure(Rc::from(&[1u8, 2, 3][..]), &slow, &clock)
        )
    );
    assert_eq!(
        0,
        clock.sleepers(),
        "the abandoned decode left nothing waiting"
    );
    assert_eq!(Duration::from_secs(15), voice_waveform::CEILING);
    let clock = self::clock();
    let in_time = Decoder::new(&clock, Duration::from_secs(14), Some(speech()));
    assert!(
        drive(
            &clock,
            voice_waveform::measure(Rc::from(&[1u8, 2, 3][..]), &in_time, &clock)
        )
        .is_some()
    );
}
