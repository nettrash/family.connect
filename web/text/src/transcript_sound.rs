//! The sound a device sends for a recording's text, when the server's
//! stored copy will not do (docs/protocol.md, "Transcripts on request";
//! issue #62, phase 3).
//!
//! The server sends a voice note's STORED bytes to its provider as they
//! are, but only for the types the provider reads and only up to
//! `transcribe_max_bytes`. A video's sound cannot be cut out on the server,
//! an Ogg file is refused by the provider, and an audio file over the
//! ceiling is too big — so for those, the asking device takes the sound out
//! of the file it holds and sends it with the request, as AAC in an M4A.
//! An answer made from that sound is the asker's alone: the server cannot
//! check that the sound is really the recording's, so it keeps nothing.
//!
//! This module is the DECISION, as arithmetic, beside the browser code that
//! carries it out (`web/src/encode.rs`, built on the #74 media code — the
//! MP4 reader and writer here, the browser's own codecs there):
//!
//! - AAC already, in an MP4 or QuickTime index, and small enough: its frames
//!   are COPIED into an M4A untouched ([`Extraction::Copy`]). No decoder, no
//!   encoder, no second generation of loss — and nothing of the picture is
//!   read, only the sound's own samples.
//! - Anything else, or AAC too big to copy: decoded and re-encoded to
//!   [`TEXT_BITRATE`] mono AAC-LC ([`Extraction::Encode`]) — speech needs no
//!   more, and 25 MiB then holds about 54 minutes.
//! - Longer than even that would hold: [`Extraction::TooLong`], told
//!   without the work. No sound track at all, or a browser that cannot
//!   encode AAC where it would have to: [`Extraction::Unreadable`].
//!
//! Neither of the last two is decided before "Show text" is pressed: what a
//! file holds is only known once the file is read, and no client downloads
//! every video in a chat to decide whether to draw a button. So the action
//! is drawn, and the answer to pressing it is one terminal line — the
//! catalogue's own sentence for each — never a guess beforehand.

use crate::media_plan::MONO_AUDIO_BITRATE;
use crate::mp4::{self, Brand, Media};
use crate::mp4_read::{self, Movie};
use crate::transcode::audio_output_rate;

/// The rate sound is re-encoded at for its text: the profile's own mono
/// rate (`media_plan::MONO_AUDIO_BITRATE`), which keeps speech whole.
pub const TEXT_BITRATE: u64 = MONO_AUDIO_BITRATE;

/// One channel: the provider hears words, not a stereo image, and one
/// channel is half the bytes.
pub const TEXT_CHANNELS: u32 = 1;

/// The sound track, as this device's reader found it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Found {
    /// The exact length of the M4A its frames make copied as they are —
    /// only for AAC read from an MP4 or QuickTime index ([`copied_track`]).
    pub copy_bytes: Option<u64>,
    /// How long it plays, where the file says.
    pub duration_ms: Option<u64>,
    /// The rate its samples run at, where the file says.
    pub sample_rate: Option<u32>,
    /// Its channels, where the file says.
    pub channels: Option<u32>,
}

/// What becomes of the sound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Extraction {
    /// AAC frames copied into an M4A, byte for byte.
    Copy,
    /// Decoded and re-encoded: AAC-LC, [`TEXT_CHANNELS`], [`TEXT_BITRATE`],
    /// at this rate — one an AAC encoder takes
    /// ([`crate::transcode::audio_output_rate`]).
    Encode { sample_rate: u32 },
    /// More than the server takes even re-encoded. Asking again will not
    /// help.
    TooLong,
    /// Nothing this device can send: no sound track, or one it would have
    /// to re-encode with no AAC encoder to do it.
    Unreadable,
}

/// The bytes `duration_ms` of sound comes to at [`TEXT_BITRATE`] — the
/// frames alone; the index on top is the writer's, and checked after.
pub fn encoded_bytes(duration_ms: u64) -> u64 {
    duration_ms.saturating_mul(TEXT_BITRATE) / 8_000
}

/// Whether a recording of `duration_ms` (its metadata's) is certainly too
/// long for `max_bytes`, even re-encoded — told before a byte of it is
/// downloaded. A length nobody stated is not "too long".
pub fn known_too_long(duration_ms: Option<i64>, max_bytes: u64) -> bool {
    duration_ms
        .and_then(|ms| u64::try_from(ms).ok())
        .filter(|&ms| ms > 0)
        .is_some_and(|ms| encoded_bytes(ms) > max_bytes)
}

/// Whether what was made goes: something, and within the server's ceiling.
pub fn fits(bytes: u64, max_bytes: u64) -> bool {
    bytes > 0 && bytes <= max_bytes
}

/// The rate a re-encode runs at, for a source at `sample_rate`.
pub fn output_rate(found: &Found) -> u32 {
    audio_output_rate(found.sample_rate)
}

/// What to do with the sound — `found` None when the file holds none —
/// within `max_bytes`, where `encodes` says whether this device can encode
/// AAC-LC at [`output_rate`], one channel, [`TEXT_BITRATE`].
///
/// Copying wins wherever it fits: it is exact, it is fast, and it is the
/// sound as it was. A copy that does not fit is re-encoded, unless even
/// that is certainly too long — which is said without the work.
pub fn extraction(found: Option<&Found>, max_bytes: u64, encodes: bool) -> Extraction {
    let Some(found) = found else {
        return Extraction::Unreadable;
    };
    if found.copy_bytes.is_some_and(|bytes| fits(bytes, max_bytes)) {
        return Extraction::Copy;
    }
    if found
        .duration_ms
        .is_some_and(|ms| encoded_bytes(ms) > max_bytes)
    {
        return Extraction::TooLong;
    }
    if !encodes {
        return Extraction::Unreadable;
    }
    Extraction::Encode {
        sample_rate: output_rate(found),
    }
}

/// What a movie's index says of its sound: `found`, and the track its AAC
/// frames are copied as, where they can be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InMovie {
    pub found: Found,
    pub copy: Option<mp4::Track>,
}

/// The sound of `movie` — None when it has no sound track at all, which
/// is "nothing to hear", not a failure to read.
pub fn in_movie(movie: &Movie) -> Option<InMovie> {
    let track = movie.audio()?;
    let entry = track.entry.as_ref();
    let copy = copied_track(movie);
    let config = entry.and_then(|entry| mp4_read::audio_config(&entry.config));
    let sample_rate = copy
        .as_ref()
        .and_then(|copy| match copy.media {
            Media::Audio { sample_rate, .. } => Some(sample_rate),
            Media::Video { .. } => None,
        })
        .or_else(|| entry.map(|entry| entry.sample_rate))
        .filter(|&rate| rate > 0);
    let channels = config
        .and_then(|config| config.channel_count())
        .or_else(|| entry.map(|entry| entry.channels))
        .filter(|&channels| channels > 0);
    Some(InMovie {
        found: Found {
            copy_bytes: copy.as_ref().map(m4a_bytes),
            duration_ms: movie.presented_ms(track).or_else(|| movie.duration_ms()),
            sample_rate,
            channels,
        },
        copy,
    })
}

/// The sound track of `movie` as an M4A's one track, its frames to be
/// copied as they are — each of them, in order, from the source track's
/// own samples ([`Movie::audio`]). None unless it is AAC (HE-AAC included:
/// it is copied, not decoded) with a configuration a decoder can start
/// from.
///
/// Where the source's edit skips an encoder's priming, the copy skips it
/// too; its `lead` — where the sound starts against a picture — means
/// nothing without the picture, and is dropped.
pub fn copied_track(movie: &Movie) -> Option<mp4::Track> {
    let track = movie.audio()?;
    let entry = track.entry.as_ref()?;
    if mp4_read::audio_codec_name(entry) != "aac" || track.timescale == 0 {
        return None;
    }
    let config = mp4_read::audio_config(&entry.config)?;
    let channels = config
        .channel_count()
        .or((entry.channels > 0).then_some(entry.channels))?;
    let sample_rate = Some(config.sample_rate)
        .filter(|&rate| rate > 0)
        .or(Some(entry.sample_rate).filter(|&rate| rate > 0))?;
    let skip = if track.edit_supported() {
        track.edit.map_or(0, |edit| edit.media_start.max(0))
    } else {
        0
    };
    Some(mp4::Track {
        timescale: track.timescale,
        media: Media::Audio {
            sample_rate,
            channels: u16::try_from(channels).ok()?,
            asc: entry.config.clone(),
            avg_bitrate: track
                .data_rate()
                .and_then(|rate| u32::try_from(rate).ok())
                .unwrap_or(0),
            max_bitrate: 0,
        },
        samples: track
            .samples
            .iter()
            .map(|sample| mp4::Sample {
                size: sample.size,
                duration: sample.duration,
                composition_offset: 0,
                sync: true,
            })
            .collect(),
        skip: u32::try_from(skip).ok()?,
        lead: 0,
    })
}

/// The whole M4A `track` makes, in bytes — known before a frame is read,
/// because the index's length depends only on the counts in it.
pub fn m4a_bytes(track: &mp4::Track) -> u64 {
    mp4::layout(std::slice::from_ref(track), Brand::M4a).total_len
}

#[cfg(test)]
mod tests {
    use super::*;

    const MOV: &[u8] = include_bytes!("../fixtures/quicktime-h264-360p.mov");
    const MP4: &[u8] = include_bytes!("../fixtures/within-h264-720p30.mp4");
    const M4A: &[u8] = include_bytes!("../fixtures/aac-128k.m4a");
    const ALAC: &[u8] = include_bytes!("../fixtures/lossless-alac.m4a");

    const MAX: u64 = 26_214_400;

    /// A file's index, as `web/src/encode.rs` finds it: the top-level boxes
    /// walked, the `moov` read whole.
    fn movie(file: &[u8]) -> Movie {
        let mut at = 0usize;
        while let Some(header) = mp4_read::box_header(&file[at..]) {
            let size = header.size.unwrap_or((file.len() - at) as u64) as usize;
            if &header.kind == b"moov" {
                return mp4_read::parse_moov(&file[at..at + size]).unwrap();
            }
            at += size;
        }
        panic!("no moov");
    }

    fn found(copy_bytes: Option<u64>, duration_ms: Option<u64>) -> Found {
        Found {
            copy_bytes,
            duration_ms,
            sample_rate: Some(44_100),
            channels: Some(2),
        }
    }

    /// THE DECISION: copy where it fits, re-encode where it does not, and
    /// say "too long" or "unreadable" without the work where that is known.
    #[test]
    fn copy_where_it_fits_and_re_encode_where_it_does_not() {
        // AAC within the ceiling: copied, whether or not this browser could
        // encode — a browser with no AAC encoder still sends a phone's
        // video's sound.
        for encodes in [true, false] {
            assert_eq!(
                extraction(Some(&found(Some(1_000), Some(60_000))), MAX, encodes),
                Extraction::Copy
            );
        }
        // The ceiling is inclusive.
        assert_eq!(
            extraction(Some(&found(Some(MAX), None)), MAX, false),
            Extraction::Copy
        );
        // AAC too big to copy, or not AAC at all: re-encoded, at a rate the
        // encoder takes for the source's family.
        for copy in [Some(MAX + 1), None] {
            assert_eq!(
                extraction(Some(&found(copy, Some(60_000))), MAX, true),
                Extraction::Encode {
                    sample_rate: 44_100
                },
                "{copy:?}"
            );
        }
        let ogg = Found {
            copy_bytes: None,
            duration_ms: Some(4_000),
            sample_rate: Some(16_000),
            channels: Some(1),
        };
        assert_eq!(
            extraction(Some(&ogg), MAX, true),
            Extraction::Encode {
                sample_rate: 48_000
            }
        );
        // …unless this browser cannot encode it.
        assert_eq!(extraction(Some(&ogg), MAX, false), Extraction::Unreadable);
        assert_eq!(
            extraction(Some(&found(Some(MAX + 1), Some(60_000))), MAX, false),
            Extraction::Unreadable
        );
        // No sound track: nothing to hear, encoder or not.
        for encodes in [true, false] {
            assert_eq!(extraction(None, MAX, encodes), Extraction::Unreadable);
        }
    }

    /// 64 kbit/s is 8 000 bytes a second, so 25 MiB holds 3 276.8 s — and a
    /// recording longer than that is too long however it is made, with or
    /// without an encoder; a length nobody stated is tried.
    #[test]
    fn too_long_is_said_without_the_work() {
        assert_eq!(encoded_bytes(1_000), 8_000);
        assert_eq!(encoded_bytes(0), 0);
        assert_eq!(encoded_bytes(u64::MAX), u64::MAX / 8_000);
        let longest = MAX / 8;
        assert_eq!(encoded_bytes(longest), MAX);
        for encodes in [true, false] {
            assert_eq!(
                extraction(Some(&found(Some(MAX * 2), Some(longest + 1))), MAX, encodes),
                Extraction::TooLong
            );
        }
        assert_eq!(
            extraction(Some(&found(Some(MAX * 2), Some(longest))), MAX, true),
            Extraction::Encode {
                sample_rate: 44_100
            }
        );
        // Too long to re-encode, yet small enough to copy: copied. A
        // 30-minute voice note at 24 kbit/s is 5 MB, and 30 minutes at
        // 64 kbit/s would be 14 MB.
        assert_eq!(
            extraction(Some(&found(Some(1_000), Some(longest * 4))), MAX, true),
            Extraction::Copy
        );
        assert_eq!(
            extraction(Some(&found(None, None)), 10, true),
            Extraction::Encode {
                sample_rate: 44_100
            },
            "no length: tried, and the result checked"
        );

        // From the metadata alone, before anything is downloaded.
        assert!(known_too_long(Some(longest as i64 + 1), MAX));
        assert!(!known_too_long(Some(longest as i64), MAX));
        for nobody_said in [None, Some(0), Some(-5)] {
            assert!(!known_too_long(nobody_said, 1), "{nobody_said:?}");
        }
    }

    #[test]
    fn what_was_made_fits_or_it_does_not() {
        assert!(fits(1, MAX));
        assert!(fits(MAX, MAX));
        assert!(!fits(MAX + 1, MAX));
        assert!(!fits(0, MAX), "nothing made is nothing to send");
    }

    /// The constants are the protocol's: 64 kbit/s, one channel — "≈ 50
    /// minutes in 25 MB".
    #[test]
    fn the_target_is_sixty_four_kilobits_of_one_channel() {
        assert_eq!(TEXT_BITRATE, 64_000);
        assert_eq!(TEXT_CHANNELS, 1);
        assert_eq!(output_rate(&found(None, None)), 44_100);
        assert_eq!(
            output_rate(&Found {
                sample_rate: None,
                ..Found::default()
            }),
            48_000
        );
    }

    /// A phone's video: its AAC frames are copied as they are — the same
    /// sizes, the same durations, the same configuration — and the M4A
    /// they make is exactly as long as the index said before a frame was
    /// read.
    #[test]
    fn a_videos_aac_is_copied_frame_for_frame() {
        for file in [MOV, MP4] {
            let movie = movie(file);
            let source = movie.audio().expect("a sound track");
            let copy = copied_track(&movie).expect("AAC, copied");
            assert_eq!(copy.samples.len(), source.samples.len());
            for (copied, read) in copy.samples.iter().zip(&source.samples) {
                assert_eq!((copied.size, copied.duration), (read.size, read.duration));
                assert!(copied.sync);
            }
            assert_eq!(copy.timescale, source.timescale);
            assert_eq!(copy.lead, 0, "no picture to start against");
            let Media::Audio { asc, .. } = &copy.media else {
                panic!("a sound track");
            };
            assert_eq!(asc, &source.entry.as_ref().unwrap().config);

            // Written out, it reads back as one AAC track — no picture —
            // whose frames are the source's own bytes.
            let payloads: Vec<Vec<u8>> = source
                .samples
                .iter()
                .map(|sample| {
                    let at = sample.offset as usize;
                    file[at..at + sample.size as usize].to_vec()
                })
                .collect();
            let m4a = mp4::write(
                std::slice::from_ref(&copy),
                std::slice::from_ref(&payloads),
                Brand::M4a,
            );
            assert_eq!(m4a.len() as u64, m4a_bytes(&copy));
            assert!(crate::media::matches_magic("audio/mp4", &m4a[..12]));
            let again = self::movie(&m4a);
            assert!(again.video().is_none(), "nothing of the picture");
            let track = again.audio().unwrap();
            assert_eq!(
                mp4_read::audio_codec_name(track.entry.as_ref().unwrap()),
                "aac"
            );
            for (sample, bytes) in track.samples.iter().zip(&payloads) {
                let at = sample.offset as usize;
                assert_eq!(&m4a[at..at + sample.size as usize], bytes.as_slice());
            }

            let found = in_movie(&movie).unwrap();
            assert_eq!(found.copy.as_ref(), Some(&copy));
            assert_eq!(found.found.copy_bytes, Some(m4a.len() as u64));
            assert!(found.found.duration_ms.is_some_and(|ms| ms > 0));
            assert!(found.found.sample_rate.is_some());
            assert!(found.found.channels.is_some());
        }
    }

    /// An M4A of AAC is copied too — an audio file over the ceiling whose
    /// copy would be no smaller is re-encoded instead, by the decision.
    #[test]
    fn an_m4a_is_copied_and_alac_is_not() {
        let aac = in_movie(&movie(M4A)).unwrap();
        assert!(aac.copy.is_some());
        let bytes = aac.found.copy_bytes.unwrap();
        assert_eq!(extraction(Some(&aac.found), bytes, true), Extraction::Copy);
        assert!(matches!(
            extraction(Some(&aac.found), bytes - 1, true),
            Extraction::Encode { .. }
        ));

        // ALAC is not AAC: nothing to copy, so it is decoded and
        // re-encoded — and still has a length and a rate to plan with.
        let alac = in_movie(&movie(ALAC)).unwrap();
        assert_eq!(alac.copy, None);
        assert_eq!(alac.found.copy_bytes, None);
        assert!(alac.found.duration_ms.is_some());
        assert!(alac.found.sample_rate.is_some());
        assert!(matches!(
            extraction(Some(&alac.found), MAX, true),
            Extraction::Encode { .. }
        ));
    }

    /// A movie with no sound track has nothing to hear.
    #[test]
    fn a_silent_film_has_no_sound_to_send() {
        let mut silent = movie(MOV);
        silent.tracks.retain(|track| &track.handler != b"soun");
        assert!(silent.video().is_some());
        assert_eq!(in_movie(&silent), None);
        assert_eq!(copied_track(&silent), None);
        assert_eq!(extraction(None, MAX, true), Extraction::Unreadable);
    }

    /// A configuration a decoder cannot start from is not copied: the
    /// server would take it, and the provider would hear nothing.
    #[test]
    fn aac_with_no_readable_configuration_is_not_copied() {
        let mut broken = movie(MOV);
        for track in &mut broken.tracks {
            if &track.handler == b"soun" {
                if let Some(entry) = track.entry.as_mut() {
                    entry.config.clear();
                    entry.object_type = 0x40;
                }
            }
        }
        assert_eq!(copied_track(&broken), None);
        let found = in_movie(&broken).unwrap();
        assert_eq!(found.found.copy_bytes, None);
    }
}
