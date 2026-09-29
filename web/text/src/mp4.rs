//! Writing an MP4 — the container every client plays (docs/protocol.md,
//! "Preparing media before upload") — for what the web client encodes
//! itself: a voice note, a picked sound file brought to the profile, and a
//! picked video transcoded to it.
//!
//! A browser can ENCODE (WebCodecs hands back AAC frames and H.264 access
//! units) but it cannot write a file: there is no muxer in the platform. So
//! this is one, in Rust, and deliberately a small one. It writes exactly the
//! shape the profile asks for and nothing else: `ftyp`, then `moov`, then
//! `mdat` — the index BEFORE the media, so a player reading the upload with a
//! `Range` request can start on the first bytes instead of fetching the tail
//! first ("faststart"). One sample description per track, no fragments, no
//! metadata a reader has to understand.
//!
//! It never holds the media itself. [`layout`] takes what every sample IS
//! (its size, its duration, whether it is a key frame) and returns the bytes
//! that go in FRONT of the media — `ftyp`, the whole `moov`, the `mdat`
//! header — and the order the samples' own bytes must follow it in. The
//! caller appends them: in a browser that is a `Blob` built from the encoder's
//! own buffers, so a hundred megabytes of video never has to be copied into
//! the WASM heap to be written out. [`write`] does the appending in memory,
//! for what is small (a voice note) and for the tests.

// --- what a track is ---------------------------------------------------------------------------

/// One sample — an AAC frame, or an H.264 access unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sample {
    /// Its bytes, as the encoder produced them.
    pub size: u32,
    /// How long it lasts, in the track's timescale.
    pub duration: u32,
    /// Presentation time minus decode time, in the track's timescale. 0
    /// unless an encoder reordered frames (B-frames); negative is allowed
    /// and written as a version-1 `ctts`.
    pub composition_offset: i32,
    /// A key frame — decodable on its own. Every audio frame is one.
    pub sync: bool,
}

/// The colour a video's pixels are in, as `colr`/`nclx` names it (ISO/IEC
/// 23091-2 code points: 1 is BT.709, 13 is sRGB's transfer, 16 and 18 are the
/// HDR transfers PQ and HLG).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Colour {
    pub primaries: u16,
    pub transfer: u16,
    pub matrix: u16,
    pub full_range: bool,
}

/// What a track carries, and the decoder configuration a player needs for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Media {
    /// H.264. `avcc` is the AVCDecoderConfigurationRecord — what WebCodecs'
    /// `VideoEncoder` hands back as `decoderConfig.description` in the "avc"
    /// format.
    Video {
        width: u16,
        height: u16,
        avcc: Vec<u8>,
        colour: Option<Colour>,
    },
    /// AAC. `asc` is the AudioSpecificConfig ([`audio_specific_config`]),
    /// which the `esds` carries and a decoder cannot start without.
    Audio {
        sample_rate: u32,
        channels: u16,
        asc: Vec<u8>,
        /// Bit/s, as the `esds` states them — 0 when not known, which is what
        /// the format says "not stated" with.
        avg_bitrate: u32,
        max_bitrate: u32,
    },
}

/// A track to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    /// Ticks per second — the sample rate for audio, and for video whatever
    /// counts its frame times exactly.
    pub timescale: u32,
    pub media: Media,
    pub samples: Vec<Sample>,
    /// Media time the presentation STARTS at, in the track's timescale: an
    /// edit list that skips this much of the beginning (an AAC encoder's
    /// priming frames, carried over from a source). 0 writes no edit list.
    pub skip: u32,
}

impl Track {
    fn is_video(&self) -> bool {
        matches!(self.media, Media::Video { .. })
    }

    /// The media's own length, in its timescale.
    fn media_duration(&self) -> u64 {
        self.samples
            .iter()
            .map(|sample| u64::from(sample.duration))
            .sum()
    }

    /// The length a player presents — the media less what the edit skips.
    fn presented_duration(&self) -> u64 {
        self.media_duration().saturating_sub(u64::from(self.skip))
    }
}

/// Which family of file this is — it decides the `ftyp` brand, and nothing
/// else. A sound file is an M4A, which is what Apple's players expect an
/// `audio/mp4` upload to call itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Brand {
    M4a,
    Mp4,
}

/// The movie's own timescale: milliseconds, which every track's length is
/// converted to for `mvhd`, `tkhd` and the edit lists.
pub const MOVIE_TIMESCALE: u32 = 1000;

/// How much of each track goes in one chunk before the file turns to the
/// other track, as a fraction of a second: half a second keeps audio and
/// video close together in the file — a player streaming it never has to
/// read far ahead to find the other half of the same moment — while keeping
/// `stco` short.
const CHUNKS_PER_SECOND: u64 = 2;

// --- AAC ---------------------------------------------------------------------------------------

/// AAC-LC's Audio Object Type — the one the profile asks for.
pub const AAC_LC: u8 = 2;

/// Samples in one AAC-LC frame, always.
pub const AAC_FRAME_SAMPLES: u32 = 1024;

/// The sampling frequencies an AudioSpecificConfig can name by index.
const AAC_RATES: [u32; 13] = [
    96_000, 88_200, 64_000, 48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000, 11_025, 8_000,
    7_350,
];

/// The AudioSpecificConfig (ISO/IEC 14496-3, 1.6.2.1) for plain AAC of
/// `object_type` at `sample_rate` with `channels` — what a decoder needs
/// before the first frame, used when an encoder hands back none of its own.
/// A rate with no index is written out in full, as the format allows.
pub fn audio_specific_config(object_type: u8, sample_rate: u32, channels: u16) -> Vec<u8> {
    let mut bits = BitWriter::default();
    bits.put(u32::from(object_type & 0x1F), 5);
    match AAC_RATES.iter().position(|&rate| rate == sample_rate) {
        Some(index) => bits.put(index as u32, 4),
        None => {
            bits.put(15, 4);
            bits.put(sample_rate & 0x00FF_FFFF, 24);
        }
    }
    bits.put(u32::from(channels.min(15)), 4);
    // GASpecificConfig: 1024-sample frames, no core coder, no extension.
    bits.put(0, 3);
    bits.finish()
}

#[derive(Default)]
struct BitWriter {
    bytes: Vec<u8>,
    used: u32,
}

impl BitWriter {
    fn put(&mut self, value: u32, count: u32) {
        for shift in (0..count).rev() {
            if self.used % 8 == 0 {
                self.bytes.push(0);
            }
            let bit = ((value >> shift) & 1) as u8;
            let last = self.bytes.len() - 1;
            self.bytes[last] |= bit << (7 - self.used % 8);
            self.used += 1;
        }
    }

    fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

// --- timing ------------------------------------------------------------------------------------

/// Durations and composition offsets for frames that came out of an encoder
/// in DECODE order, each with its presentation time (`pts`, in ticks) —
/// which is how WebCodecs reports them. Decode times are the presentation
/// times sorted, so an encoder that never reorders gives offsets of 0 and a
/// plain `stts`; one that does gives the offsets that put each frame back
/// where it is shown. `last` is how long the final frame lasts.
///
/// Returns `(duration, composition_offset)` per frame, in the order given.
pub fn decode_timeline(pts: &[i64], last: u32) -> Vec<(u32, i32)> {
    let mut sorted = pts.to_vec();
    sorted.sort_unstable();
    (0..pts.len())
        .map(|index| {
            let decode = sorted[index];
            let duration = match sorted.get(index + 1) {
                Some(next) => u32::try_from(next - decode).unwrap_or(u32::MAX),
                None => last,
            };
            let offset = i32::try_from(pts[index] - decode).unwrap_or(0);
            (duration, offset)
        })
        .collect()
}

// --- layout ------------------------------------------------------------------------------------

/// Everything in front of the media, and where each sample's bytes go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// `ftyp`, `moov` and the `mdat` header, in that order.
    pub header: Vec<u8>,
    /// `(track, sample)` for each sample, in the order its bytes follow the
    /// header — interleaved by time, chunk by chunk.
    pub order: Vec<(usize, usize)>,
    /// The whole file's length, header included.
    pub total_len: u64,
}

/// One run of consecutive samples of one track, stored together.
struct Chunk {
    track: usize,
    first: usize,
    count: usize,
    /// When it starts, in its track's timescale — for ordering chunks by
    /// time across tracks.
    start: u64,
}

fn chunks(tracks: &[Track]) -> Vec<Chunk> {
    let mut all = Vec::new();
    for (index, track) in tracks.iter().enumerate() {
        let span = (u64::from(track.timescale) / CHUNKS_PER_SECOND).max(1);
        let mut time = 0u64;
        let mut current: Option<Chunk> = None;
        for (sample_index, sample) in track.samples.iter().enumerate() {
            let slot = time / span;
            match current.as_mut() {
                Some(chunk) if chunk.start / span == slot => chunk.count += 1,
                _ => {
                    all.extend(current.take());
                    current = Some(Chunk {
                        track: index,
                        first: sample_index,
                        count: 1,
                        start: time,
                    });
                }
            }
            time += u64::from(sample.duration);
        }
        all.extend(current);
    }
    // By time — compared exactly, across two timescales, by cross
    // multiplication — and the lower-numbered track first on a tie.
    all.sort_by(|a, b| {
        let left = u128::from(a.start) * u128::from(tracks[b.track].timescale);
        let right = u128::from(b.start) * u128::from(tracks[a.track].timescale);
        left.cmp(&right).then(a.track.cmp(&b.track))
    });
    all
}

/// Lay out a file of `tracks`, `moov` first.
///
/// Every offset in the index depends on how long the index is, and how long
/// it is depends only on the counts in it (not on the offsets' values) once
/// `stco` or `co64` is chosen — so the `moov` is built once with the offsets
/// it will actually have, computed from a size known in advance.
pub fn layout(tracks: &[Track], brand: Brand) -> Layout {
    let chunks = chunks(tracks);
    let payload: u64 = tracks
        .iter()
        .flat_map(|track| track.samples.iter())
        .map(|sample| u64::from(sample.size))
        .sum();
    let ftyp = ftyp(brand);
    // A `moov` with every chunk offset 0 is exactly as long as the real one.
    let wide = payload + (1 << 24) > u64::from(u32::MAX);
    let placeholder = moov(tracks, &chunks, &vec![0; chunks.len()], wide);
    let mdat_header_len: u64 = if payload + 8 > u64::from(u32::MAX) {
        16
    } else {
        8
    };
    let media_start = ftyp.len() as u64 + placeholder.len() as u64 + mdat_header_len;
    let mut offsets = Vec::with_capacity(chunks.len());
    let mut order = Vec::new();
    let mut at = media_start;
    for chunk in &chunks {
        offsets.push(at);
        for sample in chunk.first..chunk.first + chunk.count {
            order.push((chunk.track, sample));
            at += u64::from(tracks[chunk.track].samples[sample].size);
        }
    }
    let moov = moov(tracks, &chunks, &offsets, wide);
    debug_assert_eq!(moov.len(), placeholder.len());
    let mut header = ftyp;
    header.extend_from_slice(&moov);
    if mdat_header_len == 16 {
        header.extend_from_slice(&1u32.to_be_bytes());
        header.extend_from_slice(b"mdat");
        header.extend_from_slice(&(payload + 16).to_be_bytes());
    } else {
        header.extend_from_slice(&((payload + 8) as u32).to_be_bytes());
        header.extend_from_slice(b"mdat");
    }
    Layout {
        header,
        order,
        total_len: at,
    }
}

/// The whole file, in memory: [`layout`]'s header and then each sample's
/// bytes from `payloads[track][sample]`, in its order.
pub fn write(tracks: &[Track], payloads: &[Vec<Vec<u8>>], brand: Brand) -> Vec<u8> {
    let layout = layout(tracks, brand);
    let mut out = layout.header;
    out.reserve((layout.total_len as usize).saturating_sub(out.len()));
    for (track, sample) in layout.order {
        out.extend_from_slice(&payloads[track][sample]);
    }
    out
}

// --- the boxes ---------------------------------------------------------------------------------

/// A box: its size, its four-character type, and `body` — the size filled
/// in once the body is written.
fn boxed(out: &mut Vec<u8>, kind: &[u8; 4], body: impl FnOnce(&mut Vec<u8>)) {
    let start = out.len();
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(kind);
    body(out);
    let size = (out.len() - start) as u32;
    out[start..start + 4].copy_from_slice(&size.to_be_bytes());
}

/// A full box: a box whose body starts with a version and 24 bits of flags.
fn full(out: &mut Vec<u8>, kind: &[u8; 4], version: u8, flags: u32, body: impl FnOnce(&mut Vec<u8>)) {
    boxed(out, kind, |out| {
        out.push(version);
        out.extend_from_slice(&flags.to_be_bytes()[1..]);
        body(out);
    });
}

fn u16be(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn u32be(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}

/// The identity matrix, in 16.16 and 2.30 fixed point. A picture is written
/// the right way up — a rotation is drawn INTO the pixels, never left as a
/// matrix a player may or may not honour.
fn identity_matrix(out: &mut Vec<u8>) {
    for value in [0x0001_0000u32, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000] {
        u32be(out, value);
    }
}

fn ftyp(brand: Brand) -> Vec<u8> {
    let (major, compatible): (&[u8; 4], &[&[u8; 4]]) = match brand {
        Brand::M4a => (b"M4A ", &[b"M4A ", b"isom", b"iso2", b"mp41"]),
        Brand::Mp4 => (b"isom", &[b"isom", b"iso2", b"avc1", b"mp41"]),
    };
    let mut out = Vec::new();
    boxed(&mut out, b"ftyp", |out| {
        out.extend_from_slice(major);
        u32be(out, 0x200);
        for brand in compatible {
            out.extend_from_slice(*brand);
        }
    });
    out
}

/// A track's length in the movie's milliseconds, rounded to the nearest.
fn in_movie_time(ticks: u64, timescale: u32) -> u64 {
    if timescale == 0 {
        return 0;
    }
    (ticks * u64::from(MOVIE_TIMESCALE) + u64::from(timescale) / 2) / u64::from(timescale)
}

fn moov(tracks: &[Track], chunks: &[Chunk], offsets: &[u64], wide: bool) -> Vec<u8> {
    let mut out = Vec::new();
    let movie_duration = tracks
        .iter()
        .map(|track| in_movie_time(track.presented_duration(), track.timescale))
        .max()
        .unwrap_or(0);
    boxed(&mut out, b"moov", |out| {
        full(out, b"mvhd", 0, 0, |out| {
            u32be(out, 0); // creation time: none — a date here is a date leaked
            u32be(out, 0); // modification time
            u32be(out, MOVIE_TIMESCALE);
            u32be(out, movie_duration.min(u64::from(u32::MAX)) as u32);
            u32be(out, 0x0001_0000); // rate 1.0
            u16be(out, 0x0100); // volume 1.0
            out.extend_from_slice(&[0; 10]);
            identity_matrix(out);
            out.extend_from_slice(&[0; 24]);
            u32be(out, tracks.len() as u32 + 1); // the next track id
        });
        for (index, track) in tracks.iter().enumerate() {
            let mine: Vec<(&Chunk, u64)> = chunks
                .iter()
                .zip(offsets.iter().copied())
                .filter(|(chunk, _)| chunk.track == index)
                .collect();
            trak(out, index as u32 + 1, track, &mine, wide);
        }
    });
    out
}

fn trak(out: &mut Vec<u8>, id: u32, track: &Track, chunks: &[(&Chunk, u64)], wide: bool) {
    let presented = in_movie_time(track.presented_duration(), track.timescale);
    boxed(out, b"trak", |out| {
        // Enabled, and in the movie.
        full(out, b"tkhd", 0, 0x3, |out| {
            u32be(out, 0);
            u32be(out, 0);
            u32be(out, id);
            u32be(out, 0);
            u32be(out, presented.min(u64::from(u32::MAX)) as u32);
            out.extend_from_slice(&[0; 8]);
            u16be(out, 0); // layer
            u16be(out, 0); // alternate group
            u16be(out, if track.is_video() { 0 } else { 0x0100 });
            u16be(out, 0);
            identity_matrix(out);
            let (width, height) = match track.media {
                Media::Video { width, height, .. } => (width, height),
                Media::Audio { .. } => (0, 0),
            };
            u32be(out, u32::from(width) << 16);
            u32be(out, u32::from(height) << 16);
        });
        if track.skip > 0 {
            boxed(out, b"edts", |out| {
                full(out, b"elst", 0, 0, |out| {
                    u32be(out, 1);
                    u32be(out, presented.min(u64::from(u32::MAX)) as u32);
                    u32be(out, track.skip);
                    u16be(out, 1); // rate 1.0
                    u16be(out, 0);
                });
            });
        }
        boxed(out, b"mdia", |out| {
            full(out, b"mdhd", 0, 0, |out| {
                u32be(out, 0);
                u32be(out, 0);
                u32be(out, track.timescale);
                u32be(out, track.media_duration().min(u64::from(u32::MAX)) as u32);
                u16be(out, 0x55C4); // "und", packed
                u16be(out, 0);
            });
            let (handler, name): (&[u8; 4], &[u8]) = if track.is_video() {
                (b"vide", b"VideoHandler\0")
            } else {
                (b"soun", b"SoundHandler\0")
            };
            full(out, b"hdlr", 0, 0, |out| {
                u32be(out, 0);
                out.extend_from_slice(handler);
                out.extend_from_slice(&[0; 12]);
                out.extend_from_slice(name);
            });
            boxed(out, b"minf", |out| {
                if track.is_video() {
                    full(out, b"vmhd", 0, 1, |out| out.extend_from_slice(&[0; 8]));
                } else {
                    full(out, b"smhd", 0, 0, |out| out.extend_from_slice(&[0; 4]));
                }
                boxed(out, b"dinf", |out| {
                    full(out, b"dref", 0, 0, |out| {
                        u32be(out, 1);
                        // Flag 1: the media is in this same file.
                        full(out, b"url ", 0, 1, |_| {});
                    });
                });
                stbl(out, track, chunks, wide);
            });
        });
    });
}

fn stbl(out: &mut Vec<u8>, track: &Track, chunks: &[(&Chunk, u64)], wide: bool) {
    boxed(out, b"stbl", |out| {
        full(out, b"stsd", 0, 0, |out| {
            u32be(out, 1);
            sample_entry(out, &track.media);
        });
        // stts: runs of equal durations.
        let mut runs: Vec<(u32, u32)> = Vec::new();
        for sample in &track.samples {
            match runs.last_mut() {
                Some((count, duration)) if *duration == sample.duration => *count += 1,
                _ => runs.push((1, sample.duration)),
            }
        }
        full(out, b"stts", 0, 0, |out| {
            u32be(out, runs.len() as u32);
            for (count, duration) in &runs {
                u32be(out, *count);
                u32be(out, *duration);
            }
        });
        // ctts: only when some frame is shown other than when it is decoded.
        if track
            .samples
            .iter()
            .any(|sample| sample.composition_offset != 0)
        {
            let mut runs: Vec<(u32, i32)> = Vec::new();
            for sample in &track.samples {
                match runs.last_mut() {
                    Some((count, offset)) if *offset == sample.composition_offset => *count += 1,
                    _ => runs.push((1, sample.composition_offset)),
                }
            }
            let version = u8::from(runs.iter().any(|(_, offset)| *offset < 0));
            full(out, b"ctts", version, 0, |out| {
                u32be(out, runs.len() as u32);
                for (count, offset) in &runs {
                    u32be(out, *count);
                    out.extend_from_slice(&offset.to_be_bytes());
                }
            });
        }
        // stss: omitted when every sample is a key frame, which is what its
        // absence means.
        if track.samples.iter().any(|sample| !sample.sync) {
            let keys: Vec<u32> = track
                .samples
                .iter()
                .enumerate()
                .filter(|(_, sample)| sample.sync)
                .map(|(index, _)| index as u32 + 1)
                .collect();
            full(out, b"stss", 0, 0, |out| {
                u32be(out, keys.len() as u32);
                for key in keys {
                    u32be(out, key);
                }
            });
        }
        // stsc: runs of chunks holding the same number of samples.
        let mut runs: Vec<(u32, u32)> = Vec::new();
        for (index, (chunk, _)) in chunks.iter().enumerate() {
            let count = chunk.count as u32;
            if runs.last().map(|(_, last)| *last) != Some(count) {
                runs.push((index as u32 + 1, count));
            }
        }
        full(out, b"stsc", 0, 0, |out| {
            u32be(out, runs.len() as u32);
            for (first, count) in &runs {
                u32be(out, *first);
                u32be(out, *count);
                u32be(out, 1); // the one sample description
            }
        });
        full(out, b"stsz", 0, 0, |out| {
            u32be(out, 0); // sizes vary: every one is listed
            u32be(out, track.samples.len() as u32);
            for sample in &track.samples {
                u32be(out, sample.size);
            }
        });
        if wide {
            full(out, b"co64", 0, 0, |out| {
                u32be(out, chunks.len() as u32);
                for (_, offset) in chunks {
                    out.extend_from_slice(&offset.to_be_bytes());
                }
            });
        } else {
            full(out, b"stco", 0, 0, |out| {
                u32be(out, chunks.len() as u32);
                for (_, offset) in chunks {
                    u32be(out, *offset as u32);
                }
            });
        }
    });
}

fn sample_entry(out: &mut Vec<u8>, media: &Media) {
    match media {
        Media::Video {
            width,
            height,
            avcc,
            colour,
        } => boxed(out, b"avc1", |out| {
            out.extend_from_slice(&[0; 6]);
            u16be(out, 1); // data reference index
            out.extend_from_slice(&[0; 16]);
            u16be(out, *width);
            u16be(out, *height);
            u32be(out, 0x0048_0000); // 72 dpi, both ways
            u32be(out, 0x0048_0000);
            u32be(out, 0);
            u16be(out, 1); // one frame per sample
            out.extend_from_slice(&[0; 32]); // no compressor name
            u16be(out, 0x0018); // colour, no alpha
            out.extend_from_slice(&[0xFF, 0xFF]); // pre_defined = -1
            boxed(out, b"avcC", |out| out.extend_from_slice(avcc));
            if let Some(colour) = colour {
                boxed(out, b"colr", |out| {
                    out.extend_from_slice(b"nclx");
                    u16be(out, colour.primaries);
                    u16be(out, colour.transfer);
                    u16be(out, colour.matrix);
                    out.push(if colour.full_range { 0x80 } else { 0 });
                });
            }
        }),
        Media::Audio {
            sample_rate,
            channels,
            asc,
            avg_bitrate,
            max_bitrate,
        } => boxed(out, b"mp4a", |out| {
            out.extend_from_slice(&[0; 6]);
            u16be(out, 1);
            out.extend_from_slice(&[0; 8]);
            u16be(out, *channels);
            u16be(out, 16); // sample size
            u16be(out, 0);
            u16be(out, 0);
            // 16.16; every rate the profile uses fits the integer half.
            u32be(out, (*sample_rate).min(0xFFFF) << 16);
            esds(out, asc, *avg_bitrate, *max_bitrate);
        }),
    }
}

/// An MPEG-4 descriptor: its tag, its length in the variable-length form
/// (seven bits a byte, high bit "more"), and its body.
fn descriptor(out: &mut Vec<u8>, tag: u8, body: &[u8]) {
    out.push(tag);
    let mut length = body.len() as u32;
    let mut bytes = vec![(length & 0x7F) as u8];
    length >>= 7;
    while length > 0 {
        bytes.push((length & 0x7F) as u8 | 0x80);
        length >>= 7;
    }
    bytes.reverse();
    out.extend_from_slice(&bytes);
    out.extend_from_slice(body);
}

fn esds(out: &mut Vec<u8>, asc: &[u8], avg_bitrate: u32, max_bitrate: u32) {
    full(out, b"esds", 0, 0, |out| {
        let mut decoder_config = vec![
            0x40, // MPEG-4 Audio
            0x15, // an audio stream (5 << 2), not upstream, reserved bit set
            0,
            0,
            0, // buffer size: not stated
        ];
        decoder_config.extend_from_slice(&max_bitrate.max(avg_bitrate).to_be_bytes());
        decoder_config.extend_from_slice(&avg_bitrate.to_be_bytes());
        descriptor(&mut decoder_config, 0x05, asc);
        let mut es = vec![0, 0, 0]; // ES_ID 0, no dependence, URL or OCR
        descriptor(&mut es, 0x04, &decoder_config);
        descriptor(&mut es, 0x06, &[0x02]); // SLConfig: predefined for MP4
        descriptor(out, 0x03, &es);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A box found in `bytes`: its type, where it starts, its whole size.
    fn top_level(bytes: &[u8]) -> Vec<([u8; 4], usize, usize)> {
        let mut boxes = Vec::new();
        let mut at = 0;
        while at + 8 <= bytes.len() {
            let mut size = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
            let kind: [u8; 4] = bytes[at + 4..at + 8].try_into().unwrap();
            if size == 1 {
                size = u64::from_be_bytes(bytes[at + 8..at + 16].try_into().unwrap()) as usize;
            }
            boxes.push((kind, at, size));
            at += size;
        }
        boxes
    }

    /// The body of the first box of `kind` anywhere under `bytes`, found by
    /// walking the containers this writer produces.
    fn find<'a>(bytes: &'a [u8], path: &[&[u8; 4]]) -> Option<&'a [u8]> {
        let (first, rest) = path.split_first()?;
        let mut at = 0;
        while at + 8 <= bytes.len() {
            let size = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
            if size < 8 || at + size > bytes.len() {
                return None;
            }
            if &bytes[at + 4..at + 8] == *first {
                let body = &bytes[at + 8..at + size];
                if rest.is_empty() {
                    return Some(body);
                }
                // Sample descriptions have a fixed header before their
                // children; stsd has a count.
                let skip = match *first {
                    b"stsd" => 8,
                    b"mp4a" => 28,
                    b"avc1" => 78,
                    _ => 0,
                };
                return find(&body[skip..], rest);
            }
            at += size;
        }
        None
    }

    fn be32(bytes: &[u8], at: usize) -> u32 {
        u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap())
    }

    fn voice(frames: usize) -> (Track, Vec<Vec<u8>>) {
        let asc = audio_specific_config(AAC_LC, 48_000, 1);
        let payloads: Vec<Vec<u8>> = (0..frames)
            .map(|index| vec![index as u8; 100 + index % 7])
            .collect();
        let track = Track {
            timescale: 48_000,
            media: Media::Audio {
                sample_rate: 48_000,
                channels: 1,
                asc,
                avg_bitrate: 64_000,
                max_bitrate: 64_000,
            },
            samples: payloads
                .iter()
                .map(|payload| Sample {
                    size: payload.len() as u32,
                    duration: AAC_FRAME_SAMPLES,
                    composition_offset: 0,
                    sync: true,
                })
                .collect(),
            skip: 0,
        };
        (track, payloads)
    }

    #[test]
    fn the_audio_specific_config_names_lc_its_rate_and_its_channels() {
        // 00010 0011 0001 000 — LC, 48 kHz (index 3), mono.
        assert_eq!(audio_specific_config(AAC_LC, 48_000, 1), vec![0x11, 0x88]);
        // 00010 0100 0010 000 — LC, 44.1 kHz (index 4), stereo.
        assert_eq!(audio_specific_config(AAC_LC, 44_100, 2), vec![0x12, 0x10]);
        // A rate with no index is written out: escape 15, then 24 bits.
        let odd = audio_specific_config(AAC_LC, 50_000, 1);
        assert_eq!(odd.len(), 5);
        assert_eq!(odd[0] >> 3, AAC_LC);
    }

    #[test]
    fn ftyp_comes_first_then_moov_then_mdat() {
        let (track, payloads) = voice(10);
        let file = write(&[track], &[payloads], Brand::M4a);
        let boxes = top_level(&file);
        let kinds: Vec<&[u8; 4]> = boxes.iter().map(|(kind, _, _)| kind).collect();
        assert_eq!(kinds, vec![b"ftyp", b"moov", b"mdat"]);
        assert_eq!(&file[8..12], b"M4A ", "an M4A says so");
        let (_, start, size) = boxes[2];
        assert_eq!(start + size, file.len(), "mdat runs to the end");
        assert!(crate::media::matches_magic("audio/mp4", &file[..12]));
        let video = write(&[video_track(3).0], &[video_track(3).1], Brand::Mp4);
        assert_eq!(&video[8..12], b"isom");
        assert!(crate::media::matches_magic("video/mp4", &video[..12]));
    }

    #[test]
    fn every_chunk_offset_points_at_its_samples_bytes() {
        let (track, payloads) = voice(200);
        let file = write(&[track.clone()], &[payloads.clone()], Brand::M4a);
        let stbl = find(&file, &[b"moov", b"trak", b"mdia", b"minf", b"stbl"]).unwrap();
        let stco = find(stbl, &[b"stco"]).unwrap();
        let stsc = find(stbl, &[b"stsc"]).unwrap();
        let stsz = find(stbl, &[b"stsz"]).unwrap();
        // stsz lists every size.
        assert_eq!(be32(stsz, 4), 0);
        assert_eq!(be32(stsz, 8), 200);
        for (index, payload) in payloads.iter().enumerate() {
            assert_eq!(be32(stsz, 12 + 4 * index) as usize, payload.len());
        }
        // Walk stsc and stco the way a player does, and find each sample's
        // own bytes where the index says they are.
        let chunk_count = be32(stco, 4) as usize;
        let runs: Vec<(usize, usize)> = (0..be32(stsc, 4) as usize)
            .map(|run| {
                (
                    be32(stsc, 8 + 12 * run) as usize,
                    be32(stsc, 12 + 12 * run) as usize,
                )
            })
            .collect();
        let mut sample = 0;
        for chunk in 1..=chunk_count {
            let per_chunk = runs
                .iter()
                .rev()
                .find(|(first, _)| *first <= chunk)
                .unwrap()
                .1;
            let mut at = be32(stco, 4 + 4 * chunk) as usize;
            for _ in 0..per_chunk {
                assert_eq!(&file[at..at + payloads[sample].len()], &payloads[sample][..]);
                at += payloads[sample].len();
                sample += 1;
            }
        }
        assert_eq!(sample, 200, "every sample is in some chunk");
        // 200 frames of 1024 at 48 kHz is 4.27 s: half-second chunks.
        assert_eq!(chunk_count, 9);
    }

    #[test]
    fn stts_counts_the_frames_and_the_durations_add_up() {
        let (track, payloads) = voice(47);
        let file = write(&[track], &[payloads], Brand::M4a);
        let stts = find(
            &file,
            &[b"moov", b"trak", b"mdia", b"minf", b"stbl", b"stts"],
        )
        .unwrap();
        assert_eq!(be32(stts, 4), 1, "one run: every frame is 1024 long");
        assert_eq!(be32(stts, 8), 47);
        assert_eq!(be32(stts, 12), AAC_FRAME_SAMPLES);
        let mdhd = find(&file, &[b"moov", b"trak", b"mdia", b"mdhd"]).unwrap();
        assert_eq!(be32(mdhd, 12), 48_000);
        assert_eq!(be32(mdhd, 16), 47 * 1024);
        let mvhd = find(&file, &[b"moov", b"mvhd"]).unwrap();
        assert_eq!(be32(mvhd, 12), MOVIE_TIMESCALE);
        // 48 128 samples at 48 kHz is 1002.67 ms.
        assert_eq!(be32(mvhd, 16), 1003);
        // No key-frame table for audio: every frame is one.
        let stbl = find(&file, &[b"moov", b"trak", b"mdia", b"minf", b"stbl"]).unwrap();
        assert!(find(stbl, &[b"stss"]).is_none());
        assert!(find(stbl, &[b"ctts"]).is_none());
    }

    #[test]
    fn the_esds_carries_the_audio_specific_config_and_the_bitrate() {
        let (track, payloads) = voice(5);
        let file = write(&[track], &[payloads], Brand::M4a);
        let stbl = find(&file, &[b"moov", b"trak", b"mdia", b"minf", b"stbl"]).unwrap();
        let stsd = find(stbl, &[b"stsd"]).unwrap();
        let entry = &stsd[8..];
        assert_eq!(&entry[4..8], b"mp4a");
        let mp4a = &entry[8..];
        assert_eq!(u16::from_be_bytes([mp4a[16], mp4a[17]]), 1, "mono");
        assert_eq!(u16::from_be_bytes([mp4a[24], mp4a[25]]), 48_000u32 as u16);
        let esds = find(stbl, &[b"stsd", b"mp4a", b"esds"]).unwrap();
        // version/flags, then ES_Descriptor (3) ⊃ DecoderConfig (4) ⊃
        // DecoderSpecificInfo (5) = the ASC, and SLConfig (6).
        assert_eq!(&esds[..4], &[0, 0, 0, 0]);
        assert_eq!(esds[4], 0x03);
        let es_len = esds[5] as usize;
        assert_eq!(es_len, esds.len() - 6, "the ES descriptor fills the box");
        assert_eq!(esds[9], 0x04);
        assert_eq!(esds[11], 0x40, "MPEG-4 audio");
        assert_eq!(esds[12], 0x15, "an audio stream");
        assert_eq!(be32(esds, 16), 64_000, "max bitrate");
        assert_eq!(be32(esds, 20), 64_000, "average bitrate");
        assert_eq!(esds[24], 0x05);
        assert_eq!(esds[25], 2);
        assert_eq!(&esds[26..28], &[0x11, 0x88], "LC, 48 kHz, mono");
        assert_eq!(&esds[28..31], &[0x06, 0x01, 0x02]);
    }

    fn video_track(frames: usize) -> (Track, Vec<Vec<u8>>) {
        let payloads: Vec<Vec<u8>> = (0..frames)
            .map(|index| vec![0xA0 | (index % 16) as u8; 1000 + index])
            .collect();
        let track = Track {
            timescale: 30_000,
            media: Media::Video {
                width: 1280,
                height: 720,
                avcc: vec![1, 0x64, 0, 0x1F, 0xFF, 0xE0, 0],
                colour: Some(Colour {
                    primaries: 1,
                    transfer: 1,
                    matrix: 1,
                    full_range: false,
                }),
            },
            samples: payloads
                .iter()
                .enumerate()
                .map(|(index, payload)| Sample {
                    size: payload.len() as u32,
                    duration: 1000,
                    composition_offset: 0,
                    sync: index % 60 == 0,
                })
                .collect(),
            skip: 0,
        };
        (track, payloads)
    }

    #[test]
    fn a_video_and_its_sound_are_interleaved_by_time() {
        let (video, video_payloads) = video_track(90); // 3 s at 30 fps
        let (mut audio, audio_payloads) = voice(141); // 3.008 s
        audio.skip = 1024;
        let tracks = [video, audio];
        let payloads = [video_payloads, audio_payloads];
        let layout = layout(&tracks, Brand::Mp4);
        // Never more than a chunk of one before the other.
        let mut switches = 0;
        for pair in layout.order.windows(2) {
            if pair[0].0 != pair[1].0 {
                switches += 1;
            }
        }
        assert!(switches >= 10, "{switches}");
        // Each track's own samples stay in order.
        for track in 0..2 {
            let mine: Vec<usize> = layout
                .order
                .iter()
                .filter(|(t, _)| *t == track)
                .map(|(_, s)| *s)
                .collect();
            assert_eq!(mine, (0..tracks[track].samples.len()).collect::<Vec<_>>());
        }
        let file = write(&tracks, &payloads, Brand::Mp4);
        assert_eq!(file.len() as u64, layout.total_len);
        let boxes = top_level(&file);
        assert_eq!(boxes[1].0, *b"moov");
        assert_eq!(boxes[2].0, *b"mdat");
        // Key frames every 60: frames 1, 61 in stss.
        let video_stbl = find(&file, &[b"moov", b"trak", b"mdia", b"minf", b"stbl"]).unwrap();
        let stss = find(video_stbl, &[b"stss"]).unwrap();
        assert_eq!(be32(stss, 4), 2);
        assert_eq!((be32(stss, 8), be32(stss, 12)), (1, 61));
        let stsd = find(video_stbl, &[b"stsd"]).unwrap();
        assert_eq!(&stsd[12..16], b"avc1");
        assert_eq!(
            find(video_stbl, &[b"stsd", b"avc1", b"avcC"]).unwrap(),
            &[1, 0x64, 0, 0x1F, 0xFF, 0xE0, 0]
        );
        assert_eq!(
            find(video_stbl, &[b"stsd", b"avc1", b"colr"]).unwrap(),
            &[b'n', b'c', b'l', b'x', 0, 1, 0, 1, 0, 1, 0]
        );
    }

    #[test]
    fn an_audio_skip_is_an_edit_list() {
        let (mut audio, payloads) = voice(100);
        audio.skip = 2112;
        let file = write(&[audio], &[payloads], Brand::M4a);
        let elst = find(&file, &[b"moov", b"trak", b"edts", b"elst"]).unwrap();
        assert_eq!(be32(elst, 4), 1);
        // 100 × 1024 − 2112 = 100 288 samples = 2089.33 ms.
        assert_eq!(be32(elst, 8), 2089);
        assert_eq!(be32(elst, 12), 2112);
        let (plain, payloads) = voice(100);
        let file = write(&[plain], &[payloads], Brand::M4a);
        assert!(find(&file, &[b"moov", b"trak", b"edts"]).is_none());
    }

    #[test]
    fn reordered_frames_get_composition_offsets() {
        // I P B B, shown as I B B P: pts in decode order 0, 3, 1, 2.
        let timeline = decode_timeline(&[0, 3000, 1000, 2000], 1000);
        assert_eq!(
            timeline,
            vec![(1000, 0), (1000, 2000), (1000, -1000), (1000, -1000)]
        );
        // In order, nothing to offset.
        assert_eq!(
            decode_timeline(&[0, 1001, 2002], 1001),
            vec![(1001, 0), (1001, 0), (1001, 0)]
        );
        let (mut video, payloads) = video_track(4);
        for (sample, (duration, offset)) in video.samples.iter_mut().zip(timeline) {
            sample.duration = duration;
            sample.composition_offset = offset;
        }
        let file = write(&[video], &[payloads], Brand::Mp4);
        let stbl = find(&file, &[b"moov", b"trak", b"mdia", b"minf", b"stbl"]).unwrap();
        let ctts_start = stbl.windows(4).position(|w| w == b"ctts").unwrap();
        assert_eq!(stbl[ctts_start + 4], 1, "negative offsets need version 1");
    }

    #[test]
    fn an_empty_track_list_is_still_a_file() {
        let file = write(&[], &[], Brand::M4a);
        let kinds: Vec<[u8; 4]> = top_level(&file).iter().map(|(k, _, _)| *k).collect();
        assert_eq!(kinds, vec![*b"ftyp", *b"moov", *b"mdat"]);
    }
}
