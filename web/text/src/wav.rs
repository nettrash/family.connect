//! A voice note as WAV, for the browsers whose recorder cannot write MP4.
//!
//! Most browsers' `MediaRecorder` produces WebM, which `kind=audio` does not
//! accept and a family's phones could not play (docs/protocol.md, "A
//! browser is a client too"). Where MP4 is not on offer the web client
//! records raw samples instead, and this turns them into the one container
//! every platform plays: 16-bit PCM, one channel, at [`VOICE_RATE`].

/// The rate a voice note is kept at: wideband speech, and a quarter of the
/// bytes of a microphone's usual 48 kHz — five minutes is under 10 MB.
pub const VOICE_RATE: u32 = 16_000;

/// `samples` taken at `from` Hz, at `to` Hz instead. Going down, each output
/// sample is the average of the input it covers — a box filter, which is
/// crude and enough to keep a voice from aliasing; going up, the nearest
/// earlier sample.
pub fn resample(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || from == 0 || to == 0 || samples.is_empty() {
        return samples.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let count = ((samples.len() as f64) / ratio).floor() as usize;
    (0..count)
        .map(|index| {
            let start = (index as f64 * ratio).floor() as usize;
            let end =
                (((index + 1) as f64 * ratio).floor() as usize).clamp(start + 1, samples.len());
            let window = &samples[start.min(samples.len() - 1)..end];
            window.iter().sum::<f32>() / window.len() as f32
        })
        .collect()
}

/// A WAV file of `samples` — floats in -1..=1, clipped outside it — as
/// 16-bit little-endian PCM, one channel, at `rate` Hz.
pub fn encode(samples: &[f32], rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // the fmt chunk's size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // one channel
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes()); // bytes per second
    out.extend_from_slice(&2u16.to_le_bytes()); // bytes per frame
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        let clipped = if sample.is_finite() {
            sample.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        let value = (clipped * i16::MAX as f32).round() as i16;
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

/// How long `samples` at `rate` Hz last, in milliseconds.
pub fn duration_ms(samples: usize, rate: u32) -> i64 {
    if rate == 0 {
        return 0;
    }
    (samples as f64 * 1000.0 / rate as f64).round() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::matches_magic;

    /// The header the server checks, and a player reads: RIFF, WAVE, PCM,
    /// one channel, sixteen bits, and the sizes adding up.
    #[test]
    fn a_wav_is_what_the_server_and_every_player_expect() {
        let wav = encode(&[0.0, 1.0, -1.0, 0.5], 16_000);
        assert!(matches_magic("audio/wav", &wav[..12]));
        assert_eq!(wav.len(), 44 + 8);
        assert_eq!(&wav[12..16], b"fmt ");
        assert_eq!(u16::from_le_bytes([wav[20], wav[21]]), 1, "PCM");
        assert_eq!(u16::from_le_bytes([wav[22], wav[23]]), 1, "one channel");
        assert_eq!(
            u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]),
            16_000
        );
        assert_eq!(
            u32::from_le_bytes([wav[28], wav[29], wav[30], wav[31]]),
            32_000
        );
        assert_eq!(u16::from_le_bytes([wav[34], wav[35]]), 16);
        assert_eq!(u32::from_le_bytes([wav[4], wav[5], wav[6], wav[7]]), 36 + 8);
        assert_eq!(u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]), 8);
        let samples: Vec<i16> = wav[44..]
            .chunks(2)
            .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        assert_eq!(samples, vec![0, i16::MAX, -i16::MAX, 16_384]);
    }

    #[test]
    fn out_of_range_samples_are_clipped_not_wrapped() {
        let wav = encode(&[2.0, -3.0, f32::NAN], 8_000);
        let samples: Vec<i16> = wav[44..]
            .chunks(2)
            .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        assert_eq!(samples, vec![i16::MAX, -i16::MAX, 0]);
    }

    #[test]
    fn a_microphones_rate_comes_down_to_a_voices() {
        let second: Vec<f32> = (0..48_000)
            .map(|n| if n % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        let down = resample(&second, 48_000, VOICE_RATE);
        assert_eq!(down.len(), 16_000);
        // A tone far above what 16 kHz can hold averages away rather than
        // folding back as a false one.
        assert!(down.iter().all(|sample| sample.abs() <= 1.0 / 3.0 + 1e-6));
        let flat = resample(&[0.25; 441], 44_100, VOICE_RATE);
        assert_eq!(flat.len(), 160);
        assert!(flat.iter().all(|sample| (sample - 0.25).abs() < 1e-6));
        assert_eq!(resample(&[0.1, 0.2], 16_000, 16_000), vec![0.1, 0.2]);
        assert!(resample(&[], 48_000, 16_000).is_empty());
    }

    #[test]
    fn a_duration_is_counted_from_the_samples() {
        assert_eq!(duration_ms(16_000, 16_000), 1_000);
        assert_eq!(duration_ms(24_000, 16_000), 1_500);
        assert_eq!(duration_ms(1, 0), 0);
    }
}
