//! What a picked video or sound file IS, read from its own bytes, in the
//! terms [`crate::media_plan`] decides on (docs/protocol.md, "Preparing media
//! before upload").
//!
//! The planner is the same arithmetic on every platform; what differs is how
//! each one READS a file. AVFoundation and Media3 have readers that answer
//! "what codec, what size, what rate"; a browser has none — its `<video>`
//! element plays a file without saying what is in it. So this is the web's
//! reader, in Rust: the MP4/QuickTime index through [`crate::mp4_read`], and
//! the headers of the four sound formats that are not MP4 (WAV, AIFF, FLAC,
//! MP3, Ogg) by hand.
//!
//! One choice here is the web's own, and it is about what "the rate the
//! container STATES" means. An MP4's index lists every sample's size and
//! duration, so the index itself states each track's rate to the byte —
//! which is also what AVFoundation reports as a track's data rate. That is
//! what is read, for video and for sound in an MP4; a stated `btrt` or
//! `esds` average is only what an encoder was asked for. For the formats
//! with no index, the rate is the one the header states (WAV's byte rate, an
//! MP3's frame header or its Xing/VBRI totals, Vorbis's nominal rate), and
//! where there is none the planner estimates it from the size and duration,
//! as the protocol says.

use crate::media_plan::{AudioSource, VideoSource};
use crate::mp4_read::{self, Movie};

// --- video --------------------------------------------------------------------------------------

/// A picked video as the planner sees it, from its index — sent as
/// `container` (`"video/mp4"` or `"video/quicktime"`), `size_bytes` long.
///
/// None when there is nothing to plan with: no video track, or a track
/// whose matrix is not a plain turn (a mirror, a skew), whose DISPLAYED size
/// cannot be told — which the caller treats as "cannot transcode this
/// source at all" (rule C).
pub fn video_source(movie: &Movie, container: &str, size_bytes: u64) -> Option<VideoSource> {
    let video = movie.video()?;
    let entry = video.entry.as_ref()?;
    let rotation = video.rotation?;
    // `tkhd`'s size is the presentation size BEFORE the matrix (with any
    // pixel aspect already applied); the entry's is the coded size.
    let (width, height) = if video.width > 0 && video.height > 0 {
        (video.width, video.height)
    } else {
        (u32::from(entry.width), u32::from(entry.height))
    };
    let (width, height) = if rotation % 180 == 90 {
        (height, width)
    } else {
        (width, height)
    };
    let audio = movie.audio();
    let audio_entry = audio.and_then(|track| track.entry.as_ref());
    Some(VideoSource {
        width,
        height,
        frame_rate: video.frame_rate(),
        container: container.to_string(),
        video_codec: mp4_read::video_codec_name(entry).to_string(),
        // A sound track is "present" even when its format cannot be named.
        audio_codec: audio.map(|_| {
            audio_entry
                .map(mp4_read::audio_codec_name)
                .unwrap_or("unknown")
                .to_string()
        }),
        audio_channels: audio_entry.and_then(channels_of),
        video_bitrate: video.data_rate(),
        audio_bitrate: audio.and_then(|track| track.data_rate()),
        size_bytes,
        duration_ms: movie.duration_ms(),
    })
}

/// A sound entry's channel count: as the entry states it, or as its
/// AudioSpecificConfig does when the entry says 0.
fn channels_of(entry: &mp4_read::SampleEntry) -> Option<u32> {
    if entry.channels > 0 {
        return Some(entry.channels);
    }
    mp4_read::audio_config(&entry.config)
        .map(|config| u32::from(config.channels))
        .filter(|&channels| channels > 0)
}

// --- sound --------------------------------------------------------------------------------------

/// A picked sound file as the planner sees it, and the rate its samples run
/// at — which the re-encode keeps, where it can (see
/// [`crate::transcode::audio_output_rate`]).
#[derive(Debug, Clone, PartialEq)]
pub struct AudioProbe {
    pub source: AudioSource,
    pub sample_rate: Option<u32>,
}

impl AudioProbe {
    fn unknown(container: &str, size_bytes: u64) -> Self {
        AudioProbe {
            source: AudioSource {
                container: container.to_string(),
                codec: "unknown".into(),
                channels: None,
                bitrate: None,
                size_bytes,
                duration_ms: None,
            },
            sample_rate: None,
        }
    }
}

/// A sound file in an MP4 (`audio/mp4`: M4A — AAC, ALAC, …), from its index.
pub fn audio_from_movie(movie: &Movie, container: &str, size_bytes: u64) -> AudioProbe {
    let Some(track) = movie.audio() else {
        return AudioProbe::unknown(container, size_bytes);
    };
    let Some(entry) = track.entry.as_ref() else {
        return AudioProbe::unknown(container, size_bytes);
    };
    let config_rate = mp4_read::audio_config(&entry.config).map(|config| config.sample_rate);
    let sample_rate = match &entry.format {
        // An AAC entry's own rate field is 16.16 and caps at 65 535; the
        // AudioSpecificConfig says what the stream really is.
        b"mp4a" => config_rate.or(Some(entry.sample_rate)),
        _ => Some(entry.sample_rate),
    }
    .filter(|&rate| rate > 0);
    let duration_ms = movie.presented_ms(track).or_else(|| movie.duration_ms());
    AudioProbe {
        source: AudioSource {
            container: container.to_string(),
            codec: mp4_read::audio_codec_name(entry).to_string(),
            channels: channels_of(entry),
            bitrate: track.data_rate(),
            size_bytes,
            duration_ms,
        },
        sample_rate,
    }
}

/// The length of an ID3v2 tag at the start of `head` — which a sound file's
/// own header comes AFTER, and which can be megabytes of cover art — or 0.
pub fn id3_length(head: &[u8]) -> u64 {
    if head.len() < 10 || &head[0..3] != b"ID3" {
        return 0;
    }
    let size = head[6..10]
        .iter()
        .fold(0u64, |size, byte| (size << 7) | u64::from(byte & 0x7F));
    let footer = if head[5] & 0x10 != 0 { 10 } else { 0 };
    10 + size + footer
}

/// A sound file that is not an MP4, from the first bytes of its own header —
/// for MP3 and FLAC, the bytes AFTER any ID3 tag ([`id3_length`]).
/// `container` is the type it would be sent as: `audio/wav`, `audio/mpeg`,
/// `audio/ogg`, or the platform's name for one the server does not take
/// (`audio/aiff`, `audio/flac`).
pub fn audio_from_header(container: &str, header: &[u8], size_bytes: u64) -> AudioProbe {
    let read = match container {
        "audio/wav" => wav(header),
        "audio/aiff" => aiff(header),
        "audio/flac" => flac(header),
        "audio/mpeg" => mp3(header),
        "audio/ogg" => ogg(header),
        _ => None,
    };
    let Some(read) = read else {
        return AudioProbe::unknown(container, size_bytes);
    };
    AudioProbe {
        source: AudioSource {
            container: container.to_string(),
            codec: read.codec.to_string(),
            channels: read.channels.filter(|&channels| channels > 0),
            bitrate: read.bitrate.filter(|&rate| rate > 0),
            size_bytes,
            duration_ms: read.duration_ms.filter(|&ms| ms > 0),
        },
        sample_rate: read.sample_rate.filter(|&rate| rate > 0),
    }
}

/// What one header said.
struct Header {
    codec: &'static str,
    channels: Option<u32>,
    sample_rate: Option<u32>,
    bitrate: Option<u64>,
    duration_ms: Option<u64>,
}

fn le16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn le32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn be16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn be32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// RIFF/WAVE: the `fmt ` chunk's format, channels, rate and byte rate, and
/// the `data` chunk's length when the header reaches it.
fn wav(bytes: &[u8]) -> Option<Header> {
    if bytes.get(0..4)? != b"RIFF" || bytes.get(8..12)? != b"WAVE" {
        return None;
    }
    let mut at = 12usize;
    let mut header: Option<Header> = None;
    let mut byte_rate = 0u32;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = le32(bytes, at + 4)? as usize;
        let body = at + 8;
        if id == b"fmt " {
            let tag = le16(bytes, body)?;
            let channels = le16(bytes, body + 2)?;
            let rate = le32(bytes, body + 4)?;
            byte_rate = le32(bytes, body + 8)?;
            // WAVE_FORMAT_EXTENSIBLE names its real format in the first two
            // bytes of a sub-format GUID.
            let format = if tag == 0xFFFE {
                le16(bytes, body + 24).unwrap_or(0)
            } else {
                tag
            };
            let codec = match format {
                1 | 3 => "pcm", // integer and float PCM
                0x55 => "mp3",
                _ => "unknown", // ADPCM, µ-law, GSM, …
            };
            header = Some(Header {
                codec,
                channels: Some(u32::from(channels)),
                sample_rate: Some(rate),
                bitrate: Some(u64::from(byte_rate) * 8),
                duration_ms: None,
            });
        } else if id == b"data" {
            if let Some(found) = header.as_mut() {
                if byte_rate > 0 {
                    found.duration_ms = Some(size as u64 * 1000 / u64::from(byte_rate));
                }
            }
            break;
        }
        // Chunks are padded to an even length.
        at = body.checked_add(size)?.checked_add(size & 1)?;
    }
    header
}

/// An 80-bit IEEE extended float — AIFF's sample rate.
fn extended(bytes: &[u8]) -> Option<f64> {
    let exponent = i32::from(be16(bytes, 0)? & 0x7FFF);
    let mantissa = u64::from_be_bytes(bytes.get(2..10)?.try_into().ok()?);
    if exponent == 0 && mantissa == 0 {
        return Some(0.0);
    }
    Some(mantissa as f64 * 2f64.powi(exponent - 16383 - 63))
}

/// AIFF and AIFF-C: `COMM` — channels, frames, bits, rate, and for AIFF-C
/// the compression, which is still PCM for the byte-order and float kinds.
fn aiff(bytes: &[u8]) -> Option<Header> {
    if bytes.get(0..4)? != b"FORM" {
        return None;
    }
    let compressed = match bytes.get(8..12)? {
        b"AIFF" => false,
        b"AIFC" => true,
        _ => return None,
    };
    let mut at = 12usize;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = be32(bytes, at + 4)? as usize;
        let body = at + 8;
        if id == b"COMM" {
            let channels = be16(bytes, body)?;
            let frames = be32(bytes, body + 2)?;
            let bits = be16(bytes, body + 6)?;
            let rate = extended(bytes.get(body + 8..body + 18)?)?;
            let codec = if !compressed {
                "pcm"
            } else {
                match bytes.get(body + 18..body + 22)? {
                    b"NONE" | b"sowt" | b"twos" | b"raw " | b"fl32" | b"FL32" | b"fl64"
                    | b"FL64" | b"in24" | b"in32" => "pcm",
                    _ => "unknown",
                }
            };
            let rate = (rate.is_finite() && rate > 0.0).then(|| rate.round() as u32);
            return Some(Header {
                codec,
                channels: Some(u32::from(channels)),
                sample_rate: rate,
                bitrate: rate.map(|rate| u64::from(rate) * u64::from(channels) * u64::from(bits)),
                duration_ms: rate
                    .filter(|&rate| rate > 0)
                    .map(|rate| u64::from(frames) * 1000 / u64::from(rate)),
            });
        }
        at = body.checked_add(size)?.checked_add(size & 1)?;
    }
    None
}

/// FLAC's STREAMINFO (at `offset` into `bytes`): 20 bits of rate, 3 of
/// channels less one, 5 of bits less one, 36 of total samples.
fn streaminfo(bytes: &[u8], offset: usize) -> Option<Header> {
    let info = bytes.get(offset + 10..offset + 18)?;
    let packed = u64::from_be_bytes(info.try_into().ok()?);
    let rate = (packed >> 44) as u32;
    let channels = ((packed >> 41) & 0x7) as u32 + 1;
    let total = packed & 0xF_FFFF_FFFF;
    Some(Header {
        codec: "flac",
        channels: Some(channels),
        sample_rate: Some(rate),
        bitrate: None,
        duration_ms: (rate > 0 && total > 0).then(|| total * 1000 / u64::from(rate)),
    })
}

fn flac(bytes: &[u8]) -> Option<Header> {
    // `fLaC`, then the first metadata block, which is always STREAMINFO.
    if bytes.get(0..4)? != b"fLaC" || bytes.get(4)? & 0x7F != 0 {
        return None;
    }
    streaminfo(bytes, 8)
}

/// MPEG audio: the first frame header after any ID3 tag, confirmed by the
/// frame that follows it where the bytes reach — and the Xing, Info or VBRI
/// header a variable-rate file states its totals in.
fn mp3(bytes: &[u8]) -> Option<Header> {
    const RATES: [[u32; 3]; 4] = [
        [11_025, 12_000, 8_000], // MPEG 2.5
        [0, 0, 0],
        [22_050, 24_000, 16_000], // MPEG 2
        [44_100, 48_000, 32_000], // MPEG 1
    ];
    const V1_L1: [u32; 15] = [0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448];
    const V1_L2: [u32; 15] = [0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384];
    const V1_L3: [u32; 15] = [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320];
    const V2_L1: [u32; 15] = [0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256];
    const V2_L23: [u32; 15] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];

    struct Frame {
        version: usize,
        layer: u8,
        kbps: u32,
        rate: u32,
        mono: bool,
        length: usize,
        samples: u32,
    }
    let frame_at = |at: usize| -> Option<Frame> {
        let head = bytes.get(at..at + 4)?;
        if head[0] != 0xFF || head[1] & 0xE0 != 0xE0 {
            return None;
        }
        let version = usize::from((head[1] >> 3) & 3);
        let layer = match (head[1] >> 1) & 3 {
            1 => 3,
            2 => 2,
            3 => 1,
            _ => return None,
        };
        let bitrate_index = usize::from(head[2] >> 4);
        let rate_index = usize::from((head[2] >> 2) & 3);
        if version == 1 || bitrate_index == 0 || bitrate_index == 15 || rate_index == 3 {
            return None;
        }
        let table = match (version == 3, layer) {
            (true, 1) => &V1_L1,
            (true, 2) => &V1_L2,
            (true, _) => &V1_L3,
            (false, 1) => &V2_L1,
            (false, _) => &V2_L23,
        };
        let kbps = table[bitrate_index];
        let rate = RATES[version][rate_index];
        let padding = usize::from((head[2] >> 1) & 1);
        let samples = match (layer, version == 3) {
            (1, _) => 384,
            (2, _) | (3, true) => 1152,
            _ => 576,
        };
        let length = if layer == 1 {
            (12 * kbps as usize * 1000 / rate as usize + padding) * 4
        } else {
            samples as usize / 8 * kbps as usize * 1000 / rate as usize + padding
        };
        Some(Frame {
            version,
            layer,
            kbps,
            rate,
            mono: head[3] >> 6 == 3,
            length,
            samples,
        })
    };
    let start = id3_length(bytes) as usize;
    let mut at = start;
    let frame = loop {
        if at + 4 > bytes.len() {
            return None;
        }
        if let Some(frame) = frame_at(at) {
            let next = at + frame.length;
            // A second frame where the first says it ends — or the end of
            // what was read — is what tells a header from a stray 0xFF.
            if next + 4 > bytes.len() || frame_at(next).is_some() {
                break frame;
            }
        }
        at += 1;
    };
    let channels = if frame.mono { 1 } else { 2 };
    // Where a Layer III frame keeps its Xing/Info tag: after the side
    // information, whose length depends on the version and the channels.
    let side = match (frame.version == 3, frame.mono) {
        (true, false) => 32,
        (true, true) | (false, false) => 17,
        (false, true) => 9,
    };
    let tag = at + 4 + side;
    let (mut frames, mut total_bytes, mut variable) = (None, None, false);
    match bytes.get(tag..tag + 4) {
        Some(b"Xing") | Some(b"Info") => {
            variable = bytes.get(tag..tag + 4) == Some(b"Xing");
            let flags = be32(bytes, tag + 4).unwrap_or(0);
            let mut field = tag + 8;
            if flags & 1 != 0 {
                frames = be32(bytes, field);
                field += 4;
            }
            if flags & 2 != 0 {
                total_bytes = be32(bytes, field);
            }
        }
        _ => {
            if bytes.get(at + 36..at + 40) == Some(b"VBRI") {
                variable = true;
                total_bytes = be32(bytes, at + 46);
                frames = be32(bytes, at + 50);
            }
        }
    }
    let frames = frames.filter(|&count| count > 0);
    let seconds_ms = frames.map(|count| {
        u64::from(count) * u64::from(frame.samples) * 1000 / u64::from(frame.rate)
    });
    let bitrate = if variable {
        match (frames, total_bytes) {
            (Some(count), Some(bytes)) if bytes > 0 => Some(
                u64::from(bytes) * 8 * u64::from(frame.rate)
                    / (u64::from(count) * u64::from(frame.samples)),
            ),
            _ => None,
        }
    } else {
        Some(u64::from(frame.kbps) * 1000)
    };
    Some(Header {
        codec: if frame.layer == 3 { "mp3" } else { "unknown" },
        channels: Some(channels),
        sample_rate: Some(frame.rate),
        bitrate,
        duration_ms: seconds_ms,
    })
}

/// Ogg: the first page's first packet names the codec — Vorbis, Opus, or
/// FLAC in Ogg — and says its channels and rate.
fn ogg(bytes: &[u8]) -> Option<Header> {
    if bytes.get(0..4)? != b"OggS" {
        return None;
    }
    let segments = usize::from(*bytes.get(26)?);
    let packet = bytes.get(27 + segments..)?;
    if packet.starts_with(b"\x01vorbis") {
        let nominal = le32(packet, 20).map(|rate| rate as i32);
        return Some(Header {
            codec: "vorbis",
            channels: packet.get(11).map(|&count| u32::from(count)),
            sample_rate: le32(packet, 12),
            bitrate: nominal.filter(|&rate| rate > 0).map(|rate| rate as u64),
            duration_ms: None,
        });
    }
    if packet.starts_with(b"OpusHead") {
        return Some(Header {
            codec: "opus",
            channels: packet.get(9).map(|&count| u32::from(count)),
            // Opus always decodes at 48 kHz, whatever the input was.
            sample_rate: Some(48_000),
            bitrate: None,
            duration_ms: None,
        });
    }
    if packet.starts_with(b"\x7FFLAC") && packet.get(9..13) == Some(b"fLaC") {
        return streaminfo(packet, 17);
    }
    Some(Header {
        codec: "unknown",
        channels: None,
        sample_rate: None,
        bitrate: None,
        duration_ms: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media_plan::{plan_audio, plan_video, AudioPlan, VideoPlan};
    use crate::mp4_read::{box_header, parse_moov};

    /// The `moov` of a whole file, wherever it is — first, or after the
    /// media as a camera leaves it.
    fn movie(file: &[u8]) -> Movie {
        let mut at = 0usize;
        while let Some(header) = box_header(&file[at..]) {
            let size = header.size.unwrap_or((file.len() - at) as u64) as usize;
            if &header.kind == b"moov" {
                return parse_moov(&file[at..at + size]).unwrap();
            }
            at += size;
        }
        panic!("no moov");
    }

    const PORTRAIT: &[u8] = include_bytes!("../fixtures/portrait-hevc-hlg-4k60.mov");
    const WITHIN: &[u8] = include_bytes!("../fixtures/within-h264-720p30.mp4");
    const QUICKTIME: &[u8] = include_bytes!("../fixtures/quicktime-h264-360p.mov");

    /// An iPhone-shaped clip, written by AVFoundation: a 3840×2160 HEVC
    /// track turned a quarter clockwise is a PORTRAIT 2160×3840 at 60 fps,
    /// in HLG — and the plan is the profile's 720×1280 at 30.
    #[test]
    fn a_turned_4k60_hdr_clip_reads_as_portrait_and_plans_to_720_by_1280_at_30() {
        let movie = movie(PORTRAIT);
        let video = movie.video().unwrap();
        assert_eq!(video.rotation, Some(90));
        assert_eq!((video.width, video.height), (3840, 2160));
        let entry = video.entry.as_ref().unwrap();
        assert_eq!(&entry.format, b"hvc1");
        assert!(entry.colour.unwrap().is_hdr(), "{:?}", entry.colour);
        assert_eq!(entry.colour.unwrap().transfer, 18, "HLG");
        let codec = mp4_read::video_codec_string(entry).unwrap();
        assert!(codec.starts_with("hvc1.2.4."), "Main 10: {codec}");
        assert!(mp4_read::video_description(entry).is_some());
        let source = video_source(&movie, "video/quicktime", PORTRAIT.len() as u64).unwrap();
        assert_eq!((source.width, source.height), (2160, 3840));
        assert_eq!(source.frame_rate, Some(60.0));
        assert_eq!(source.video_codec, "hevc");
        assert_eq!(source.audio_codec.as_deref(), Some("aac"));
        assert_eq!(source.audio_channels, Some(2));
        assert!(source.video_bitrate.is_some() && source.audio_bitrate.is_some());
        match plan_video(&source) {
            VideoPlan::Transcode(target) => {
                assert_eq!((target.width, target.height), (720, 1280));
                assert_eq!(target.frame_rate, 30.0);
                assert!(target.audio_bitrate.is_some());
            }
            other => panic!("{other:?}"),
        }
        // Every sample lies inside the file.
        let len = PORTRAIT.len() as u64;
        for track in &movie.tracks {
            for sample in &track.samples {
                assert!(sample.offset + u64::from(sample.size) <= len);
            }
        }
        // Presentation order is not decode order in an HEVC stream from a
        // phone, and the edit list says where it starts.
        assert!(video.samples.iter().any(|s| s.pts != s.dts));
        assert!(video.edit.is_some());
        assert!(video.edit_supported());
    }

    /// Already 720p30 H.264 with AAC, in an MP4 whose index sits AFTER the
    /// media: rule A keeps it.
    #[test]
    fn a_clip_within_the_profile_is_kept_even_with_its_index_at_the_end() {
        let tops: Vec<[u8; 4]> = {
            let mut kinds = Vec::new();
            let mut at = 0usize;
            while let Some(header) = box_header(&WITHIN[at..]) {
                kinds.push(header.kind);
                at += header.size.unwrap() as usize;
            }
            kinds
        };
        assert_eq!(tops, vec![*b"ftyp", *b"mdat", *b"moov"]);
        let movie = movie(WITHIN);
        let source = video_source(&movie, "video/mp4", WITHIN.len() as u64).unwrap();
        assert_eq!((source.width, source.height), (1280, 720));
        assert_eq!(source.frame_rate, Some(30.0));
        assert_eq!(source.video_codec, "h264");
        assert_eq!(source.audio_codec.as_deref(), Some("aac"));
        let entry = movie.video().unwrap().entry.as_ref().unwrap();
        assert!(mp4_read::video_codec_string(entry)
            .unwrap()
            .starts_with("avc1.64"));
        assert_eq!(plan_video(&source), VideoPlan::Keep);
    }

    /// The same codecs in a QuickTime movie are never within the profile —
    /// and QuickTime's sound description, `esds` and all, still reads.
    #[test]
    fn a_quicktime_movie_is_transcoded_and_its_sound_still_reads() {
        let movie = movie(QUICKTIME);
        let source = video_source(&movie, "video/quicktime", QUICKTIME.len() as u64).unwrap();
        assert_eq!((source.width, source.height), (640, 360));
        assert_eq!(source.audio_codec.as_deref(), Some("aac"));
        assert_eq!(source.audio_channels, Some(1));
        let audio = movie.audio().unwrap().entry.as_ref().unwrap();
        assert_eq!(
            mp4_read::audio_config(&audio.config).map(|c| (c.object_type, c.sample_rate)),
            Some((2, 44_100))
        );
        match plan_video(&source) {
            VideoPlan::Transcode(target) => {
                assert_eq!((target.width, target.height), (640, 360), "never upscaled");
                assert_eq!(target.audio_bitrate.map(|rate| rate <= 64_000), Some(true));
            }
            other => panic!("{other:?}"),
        }
        let as_mp4 = video_source(&movie, "video/mp4", QUICKTIME.len() as u64).unwrap();
        assert_eq!(plan_video(&as_mp4), VideoPlan::Keep, "only the container stood in the way");
    }

    fn probe_mp4(file: &[u8]) -> AudioProbe {
        audio_from_movie(&movie(file), "audio/mp4", file.len() as u64)
    }

    #[test]
    fn picked_sound_files_meet_the_audio_rules() {
        // AAC at 256 kbit/s — the index says so, byte for byte — is above
        // 192 000 and re-encoded, at 128 000 because it is stereo.
        let aac = probe_mp4(include_bytes!("../fixtures/aac-256k.m4a"));
        assert_eq!(aac.source.codec, "aac");
        assert_eq!(aac.source.channels, Some(2));
        assert_eq!(aac.sample_rate, Some(44_100));
        assert!(aac.source.bitrate.unwrap() > 192_000, "{:?}", aac.source.bitrate);
        // 24 frames of 1024: the index's own length. afconvert states the
        // 0.5 s it was made from only in iTunes' gapless atom, not in an edit
        // list, and the planner needs the length only when no rate can be
        // read — never for an MP4, whose index always gives one.
        assert_eq!(aac.source.duration_ms, Some(557));
        assert_eq!(plan_audio(&aac.source), AudioPlan::Transcode { bitrate: 128_000 });
        // At 128 it is left as it is.
        let kept = probe_mp4(include_bytes!("../fixtures/aac-128k.m4a"));
        assert!(kept.source.bitrate.unwrap() <= 192_000);
        assert_eq!(plan_audio(&kept.source), AudioPlan::Keep);
        // Lossless in an M4A: re-encoded, mono at 64 000.
        let alac = probe_mp4(include_bytes!("../fixtures/lossless-alac.m4a"));
        assert_eq!(alac.source.codec, "alac");
        assert_eq!(alac.source.channels, Some(1));
        assert_eq!(plan_audio(&alac.source), AudioPlan::Transcode { bitrate: 64_000 });
        // FLAC and AIFF, which the server does not take as audio at all.
        let flac_file: &[u8] = include_bytes!("../fixtures/lossless.flac");
        let flac = audio_from_header("audio/flac", flac_file, flac_file.len() as u64);
        assert_eq!(flac.source.codec, "flac");
        assert_eq!(
            (flac.source.channels, flac.sample_rate, flac.source.duration_ms),
            (Some(1), Some(44_100), Some(500))
        );
        assert_eq!(plan_audio(&flac.source), AudioPlan::Transcode { bitrate: 64_000 });
        let aiff_file: &[u8] = include_bytes!("../fixtures/uncompressed.aiff");
        let aiff = audio_from_header("audio/aiff", aiff_file, aiff_file.len() as u64);
        assert_eq!(aiff.source.codec, "pcm");
        assert_eq!(
            (aiff.source.channels, aiff.sample_rate, aiff.source.duration_ms),
            (Some(1), Some(44_100), Some(500))
        );
        assert_eq!(aiff.source.bitrate, Some(705_600));
        assert_eq!(plan_audio(&aiff.source), AudioPlan::Transcode { bitrate: 64_000 });
    }

    #[test]
    fn a_wav_is_pcm_with_its_rate_and_length() {
        let samples = vec![0.0f32; 16_000];
        let file = crate::wav::encode(&samples, 16_000);
        let probe = audio_from_header("audio/wav", &file, file.len() as u64);
        assert_eq!(probe.source.codec, "pcm");
        assert_eq!(probe.source.channels, Some(1));
        assert_eq!(probe.sample_rate, Some(16_000));
        assert_eq!(probe.source.bitrate, Some(256_000));
        assert_eq!(probe.source.duration_ms, Some(1_000));
        assert_eq!(plan_audio(&probe.source), AudioPlan::Transcode { bitrate: 64_000 });
        // ADPCM in a WAV is no rule's business: kept.
        let mut adpcm = file.clone();
        adpcm[20] = 0x11;
        let probe = audio_from_header("audio/wav", &adpcm, adpcm.len() as u64);
        assert_eq!(probe.source.codec, "unknown");
        assert_eq!(plan_audio(&probe.source), AudioPlan::Keep);
    }

    /// A Layer III frame header of `kbps` at 44.1 kHz stereo, and the silent
    /// frame it heads, `count` times.
    fn mp3_frames(bitrate_index: u8, count: usize, tag: Option<&[u8]>) -> Vec<u8> {
        let kbps = [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320]
            [bitrate_index as usize];
        let length = 144 * kbps * 1000 / 44_100;
        let mut file = Vec::new();
        for index in 0..count {
            let mut frame = vec![0xFF, 0xFB, bitrate_index << 4, 0x00];
            frame.resize(length, 0);
            if index == 0 {
                if let Some(tag) = tag {
                    frame[36..36 + tag.len()].copy_from_slice(tag);
                }
            }
            file.extend_from_slice(&frame);
        }
        file
    }

    #[test]
    fn an_mp3_states_its_rate_in_its_frames_or_its_xing_totals() {
        // 128 kbit/s: kept.
        let file = mp3_frames(9, 5, None);
        let probe = audio_from_header("audio/mpeg", &file, file.len() as u64);
        assert_eq!(probe.source.codec, "mp3");
        assert_eq!(probe.source.bitrate, Some(128_000));
        assert_eq!(probe.source.channels, Some(2));
        assert_eq!(probe.sample_rate, Some(44_100));
        assert_eq!(plan_audio(&probe.source), AudioPlan::Keep);
        // 320: above 192 000, re-encoded.
        let loud = mp3_frames(14, 5, None);
        let probe = audio_from_header("audio/mpeg", &loud, loud.len() as u64);
        assert_eq!(probe.source.bitrate, Some(320_000));
        assert_eq!(plan_audio(&probe.source), AudioPlan::Transcode { bitrate: 128_000 });
        // Behind an ID3 tag, which is skipped, not read as audio.
        let mut tagged = vec![b'I', b'D', b'3', 4, 0, 0, 0, 0, 0, 20];
        tagged.extend_from_slice(&[0xFF; 20]);
        assert_eq!(id3_length(&tagged), 30);
        tagged.extend_from_slice(&file);
        let probe = audio_from_header("audio/mpeg", &tagged, tagged.len() as u64);
        assert_eq!(probe.source.bitrate, Some(128_000));
        // A VBR file's Xing header states its totals — 1000 frames, and
        // 1 600 000 bytes — and the rate and length come from those, not
        // from whichever frame happens to be first.
        let mut xing = b"Xing".to_vec();
        xing.extend_from_slice(&3u32.to_be_bytes());
        xing.extend_from_slice(&1000u32.to_be_bytes());
        xing.extend_from_slice(&1_600_000u32.to_be_bytes());
        let vbr = mp3_frames(9, 3, Some(&xing));
        let probe = audio_from_header("audio/mpeg", &vbr, vbr.len() as u64);
        // 1 600 000 × 8 × 44 100 ÷ (1000 × 1152) = 490 000.
        assert_eq!(probe.source.bitrate, Some(490_000));
        assert_eq!(probe.source.duration_ms, Some(26_122));
        // A Xing tag with no totals states nothing; the planner estimates.
        let mut bare = b"Xing".to_vec();
        bare.extend_from_slice(&0u32.to_be_bytes());
        let vbr = mp3_frames(9, 3, Some(&bare));
        let probe = audio_from_header("audio/mpeg", &vbr, vbr.len() as u64);
        assert_eq!(probe.source.bitrate, None);
    }

    fn ogg_page(packet: &[u8]) -> Vec<u8> {
        let mut page = b"OggS".to_vec();
        page.extend_from_slice(&[0, 2]);
        page.extend_from_slice(&[0; 20]);
        page.push(1);
        page.push(packet.len() as u8);
        page.extend_from_slice(packet);
        page
    }

    #[test]
    fn anything_in_ogg_is_re_encoded_and_its_codec_is_named() {
        let mut opus = b"OpusHead".to_vec();
        opus.extend_from_slice(&[1, 2, 0x38, 0x01, 0x80, 0xBB, 0, 0, 0, 0, 0]);
        let file = ogg_page(&opus);
        let probe = audio_from_header("audio/ogg", &file, 400_000);
        assert_eq!(probe.source.codec, "opus");
        assert_eq!(probe.source.channels, Some(2));
        assert_eq!(probe.sample_rate, Some(48_000));
        assert!(matches!(plan_audio(&probe.source), AudioPlan::Transcode { .. }));
        let mut vorbis = b"\x01vorbis".to_vec();
        vorbis.extend_from_slice(&0u32.to_le_bytes());
        vorbis.push(1);
        vorbis.extend_from_slice(&44_100u32.to_le_bytes());
        vorbis.extend_from_slice(&0i32.to_le_bytes());
        vorbis.extend_from_slice(&96_000i32.to_le_bytes());
        vorbis.extend_from_slice(&0i32.to_le_bytes());
        let file = ogg_page(&vorbis);
        let probe = audio_from_header("audio/ogg", &file, 400_000);
        assert_eq!(probe.source.codec, "vorbis");
        assert_eq!(probe.source.bitrate, Some(96_000));
        // Mono: the mono rate, which is below what it states.
        assert_eq!(plan_audio(&probe.source), AudioPlan::Transcode { bitrate: 64_000 });
        let odd = ogg_page(b"Speex   ");
        let probe = audio_from_header("audio/ogg", &odd, 1000);
        assert_eq!(probe.source.codec, "unknown");
        assert!(matches!(plan_audio(&probe.source), AudioPlan::Transcode { .. }));
    }

    #[test]
    fn a_header_that_is_not_one_is_unknown_never_a_panic() {
        for container in ["audio/wav", "audio/aiff", "audio/flac", "audio/mpeg", "audio/ogg"] {
            for garbage in [&b""[..], b"RIFF", b"FORM\0\0\0\0AIFF", b"fLaC\0", &[0xFF; 3][..]] {
                let probe = audio_from_header(container, garbage, 10);
                let _ = plan_audio(&probe.source);
            }
        }
        let probe = audio_from_header("audio/wav", b"not a wav at all", 16);
        assert_eq!(probe.source.codec, "unknown");
        assert_eq!(plan_audio(&probe.source), AudioPlan::Keep);
    }
}
