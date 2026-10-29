//! The two call tones (shell-design §14.4), synthesised exactly as Windows' `call.html` and the
//! web client (`web/src/calls.rs`, `Ring`) make them with Web Audio, and played in a pipeline of
//! their own so they come and go without touching a call's.
//!
//! - **Incoming** — somebody is calling this device: two 0.4-s beeps of 660 Hz at 0.08 (at 0 and
//!   0.6 s), every 3 s.
//! - **Ringback** — what the caller hears while it rings: 425 Hz at 0.05, 1 s on, every 5 s (the
//!   European exchange's cadence; ios `RingbackTone`'s default `cept`).
//!
//! Each beep is Web Audio's envelope, ported literally: `setValueAtTime(0, at)`,
//! `linearRampToValueAtTime(volume, at + 0.02)`, `linearRampToValueAtTime(0, at + length)` — a
//! 20-ms rise, then a straight fall to nothing at the beep's end. A sine starting at phase 0 at
//! each beep, as a fresh `OscillatorNode` does.

use std::sync::Arc;

use fc_logic::calls::RingTone;
use gst::prelude::*;

use super::pipeline;

/// The rate the tones are made at.
pub const RATE: u32 = 48_000;

/// One beep: when it starts in its cycle and how long it lasts, in seconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Beep {
    pub at: f64,
    pub length: f64,
}

/// A tone's shape: the pitch, the loudness (Web Audio gain), the beeps, and how often it repeats.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shape {
    pub hertz: f64,
    pub volume: f64,
    pub beeps: &'static [Beep],
    pub every: f64,
}

/// The rise of every beep.
pub const RISE: f64 = 0.02;

const INCOMING_BEEPS: &[Beep] = &[
    Beep {
        at: 0.0,
        length: 0.4,
    },
    Beep {
        at: 0.6,
        length: 0.4,
    },
];

const RINGBACK_BEEPS: &[Beep] = &[Beep {
    at: 0.0,
    length: 1.0,
}];

/// The shape of `tone` — `Ring::incoming` and `Ring::ringback` of the web client.
pub fn shape(tone: RingTone) -> Shape {
    match tone {
        RingTone::Incoming => Shape {
            hertz: 660.0,
            volume: 0.08,
            beeps: INCOMING_BEEPS,
            every: 3.0,
        },
        RingTone::Ringback => Shape {
            hertz: 425.0,
            volume: 0.05,
            beeps: RINGBACK_BEEPS,
            every: 5.0,
        },
    }
}

/// The gain `t` seconds into a beep of `length` at `volume`: 0 → `volume` over [`RISE`], then a
/// straight fall to 0 at `length`; 0 outside the beep.
pub fn envelope(t: f64, length: f64, volume: f64) -> f64 {
    if !(0.0..length).contains(&t) {
        0.0
    } else if t < RISE {
        volume * t / RISE
    } else {
        volume * (length - t) / (length - RISE)
    }
}

/// One whole cycle of `tone` at `rate`, mono signed 16-bit — what the ringer loops.
pub fn cycle(tone: RingTone, rate: u32) -> Vec<i16> {
    let shape = shape(tone);
    let rate = f64::from(rate);
    let samples = (shape.every * rate).round() as usize;
    let mut out = vec![0i16; samples];
    for beep in shape.beeps {
        let start = (beep.at * rate).round() as usize;
        let end = ((beep.at + beep.length) * rate).round() as usize;
        for (offset, sample) in out
            .iter_mut()
            .enumerate()
            .take(end.min(samples))
            .skip(start)
        {
            let t = (offset - start) as f64 / rate;
            let level = envelope(t, beep.length, shape.volume)
                * (std::f64::consts::TAU * shape.hertz * t).sin();
            *sample = (level * f64::from(i16::MAX)).round() as i16;
        }
    }
    out
}

/// The cycle as little-endian bytes, the buffer the ringer pushes.
fn cycle_bytes(tone: RingTone) -> Arc<[u8]> {
    cycle(tone, RATE)
        .into_iter()
        .flat_map(i16::to_le_bytes)
        .collect::<Vec<u8>>()
        .into()
}

/// Shared bytes a `gst::Buffer` can wrap without a copy per cycle.
struct Bytes(Arc<[u8]>);

impl AsRef<[u8]> for Bytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// The ringing: one tone at a time in a pipeline of its own. `ring` replaces a tone already
/// sounding; `stop` is idempotent. Neither emits anything — a ringer that cannot open its output
/// (no sound server) logs it and stays silent: the Call window and the notification still say a
/// call is ringing.
pub struct Ringer {
    sink: String,
    playing: std::cell::RefCell<Option<(RingTone, gst::Pipeline)>>,
}

impl Ringer {
    /// A ringer into `sink` (a `gst-launch` description: `pulsesink` on the desktop).
    pub fn new(sink: impl Into<String>) -> Self {
        Ringer {
            sink: sink.into(),
            playing: std::cell::RefCell::new(None),
        }
    }

    /// The tone now sounding, if any.
    pub fn tone(&self) -> Option<RingTone> {
        self.playing.borrow().as_ref().map(|(tone, _)| *tone)
    }

    /// Sound `tone`, in place of whatever sounds now.
    pub fn ring(&self, tone: RingTone) {
        self.stop();
        match self.start(tone) {
            Ok(pipeline) => *self.playing.borrow_mut() = Some((tone, pipeline)),
            Err(why) => glib::g_warning!("family-connect", "the call tone did not start: {why}"),
        }
    }

    /// Silence. Idempotent, and silent when nothing rings.
    pub fn stop(&self) {
        let playing = self.playing.borrow_mut().take();
        if let Some((_, pipeline)) = playing {
            let _ = pipeline.set_state(gst::State::Null);
        }
    }

    fn start(&self, tone: RingTone) -> Result<gst::Pipeline, String> {
        let description = pipeline::ringer(&self.sink, RATE);
        let bin = gst::parse::bin_from_description(&description, false)
            .map_err(|error| format!("parse: {}", error.message()))?;
        super::media::set_phone_role(bin.upcast_ref());
        let pipeline = gst::Pipeline::with_name("fc-call-tone");
        pipeline
            .add(&bin)
            .map_err(|_| "the tone's bin was refused".to_string())?;
        let source = bin
            .by_name("tone")
            .and_then(|element| element.downcast::<gst_app::AppSrc>().ok())
            .ok_or_else(|| "no appsrc".to_string())?;
        let bytes = cycle_bytes(tone);
        let length =
            gst::ClockTime::from_nseconds((shape(tone).every * 1_000_000_000.0).round() as u64);
        let mut next = gst::ClockTime::ZERO;
        source.set_callbacks(
            gst_app::AppSrcCallbacks::builder()
                .need_data(move |source, _| {
                    let mut buffer = gst::Buffer::from_slice(Bytes(bytes.clone()));
                    if let Some(buffer) = buffer.get_mut() {
                        buffer.set_pts(next);
                        buffer.set_duration(length);
                    }
                    next += length;
                    let _ = source.push_buffer(buffer);
                })
                .build(),
        );
        if pipeline.set_state(gst::State::Playing).is_err() {
            let _ = pipeline.set_state(gst::State::Null);
            return Err("the output did not open".into());
        }
        Ok(pipeline)
    }
}

impl Drop for Ringer {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peak(samples: &[i16]) -> i16 {
        samples
            .iter()
            .map(|s| s.saturating_abs())
            .max()
            .unwrap_or(0)
    }

    /// Sign changes from negative to non-negative: one per period of a sine.
    fn rising_crossings(samples: &[i16]) -> usize {
        samples
            .windows(2)
            .filter(|pair| pair[0] < 0 && pair[1] >= 0)
            .count()
    }

    #[test]
    fn the_shapes_are_the_web_clients() {
        let incoming = shape(RingTone::Incoming);
        assert_eq!(incoming.hertz, 660.0);
        assert_eq!(incoming.volume, 0.08);
        assert_eq!(incoming.every, 3.0);
        assert_eq!(
            incoming.beeps,
            &[
                Beep {
                    at: 0.0,
                    length: 0.4
                },
                Beep {
                    at: 0.6,
                    length: 0.4
                }
            ]
        );
        let ringback = shape(RingTone::Ringback);
        assert_eq!(ringback.hertz, 425.0);
        assert_eq!(ringback.volume, 0.05);
        assert_eq!(ringback.every, 5.0);
        assert_eq!(
            ringback.beeps,
            &[Beep {
                at: 0.0,
                length: 1.0
            }]
        );
    }

    #[test]
    fn the_envelope_rises_in_20_ms_and_falls_straight_to_the_end() {
        assert_eq!(envelope(0.0, 0.4, 0.08), 0.0);
        assert!((envelope(0.01, 0.4, 0.08) - 0.04).abs() < 1e-12);
        assert!((envelope(0.02, 0.4, 0.08) - 0.08).abs() < 1e-12);
        // Halfway down the fall: (0.02 + 0.4) / 2.
        assert!((envelope(0.21, 0.4, 0.08) - 0.04).abs() < 1e-12);
        assert_eq!(envelope(0.4, 0.4, 0.08), 0.0);
        assert_eq!(envelope(-0.001, 0.4, 0.08), 0.0);
        assert_eq!(envelope(0.5, 0.4, 0.08), 0.0);
    }

    #[test]
    fn an_incoming_cycle_is_three_seconds_with_two_beeps_and_silence_between() {
        let samples = cycle(RingTone::Incoming, 48_000);
        assert_eq!(samples.len(), 144_000);
        let at = |seconds: f64| (seconds * 48_000.0) as usize;
        assert!(peak(&samples[at(0.0)..at(0.4)]) > 0);
        assert_eq!(peak(&samples[at(0.4)..at(0.6)]), 0);
        assert!(peak(&samples[at(0.6)..at(1.0)]) > 0);
        assert_eq!(peak(&samples[at(1.0)..]), 0);
        // The loudest a beep gets is its volume, at the end of the rise.
        let loudest = f64::from(peak(&samples));
        let expected = 0.08 * f64::from(i16::MAX);
        assert!(
            loudest <= expected.ceil() && loudest > expected * 0.98,
            "{loudest}"
        );
        // 660 Hz: 264 periods in each 0.4-s beep, give or take the edge.
        let crossings = rising_crossings(&samples[at(0.0)..at(0.4)]);
        assert!((262..=265).contains(&crossings), "{crossings}");
    }

    #[test]
    fn a_ringback_cycle_is_one_second_of_425_hz_in_five() {
        let samples = cycle(RingTone::Ringback, 48_000);
        assert_eq!(samples.len(), 240_000);
        assert!(peak(&samples[..48_000]) > 0);
        assert_eq!(peak(&samples[48_000..]), 0);
        let crossings = rising_crossings(&samples[..48_000]);
        assert!((423..=426).contains(&crossings), "{crossings}");
        let loudest = f64::from(peak(&samples));
        assert!(loudest <= (0.05 * f64::from(i16::MAX)).ceil(), "{loudest}");
    }

    #[test]
    fn every_beep_starts_and_ends_at_silence() {
        for tone in [RingTone::Incoming, RingTone::Ringback] {
            let samples = cycle(tone, 48_000);
            for beep in shape(tone).beeps {
                let start = (beep.at * 48_000.0) as usize;
                let end = ((beep.at + beep.length) * 48_000.0) as usize;
                assert_eq!(samples[start], 0, "{tone:?} starts at silence");
                assert!(samples[end - 1].abs() < 40, "{tone:?} ends near silence");
                // Within the 20-ms rise the level only grows: the first millisecond is quiet.
                assert!(
                    peak(&samples[start..start + 48]) < 200,
                    "{tone:?} rises softly"
                );
            }
        }
    }

    #[test]
    fn the_cycle_follows_the_rate() {
        assert_eq!(cycle(RingTone::Incoming, 16_000).len(), 48_000);
        assert_eq!(cycle(RingTone::Ringback, 8_000).len(), 40_000);
        assert_eq!(cycle_bytes(RingTone::Ringback).len(), 2 * 240_000);
    }
}
