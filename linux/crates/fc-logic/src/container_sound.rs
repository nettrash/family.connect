//! The sound tracks an MP4 or a QuickTime file carries, read from the container's own track list;
//! which of them PLAY; and what Rule E decides from that (`ContainerSound.cs`; docs/protocol.md,
//! "Preparing media before upload", **Rule E — a transcode keeps the sound**; issue #98).
//!
//! WHY THE CONTAINER AND NOT THE DECODER. A decoder that cannot read a track does not list it: a
//! pipeline asked for "the audio" of a clip whose sound is in a sample entry it does not know asks
//! for none, none comes out, and the read-back agrees with itself over a SILENT upload. The `moov`
//! is not blind like that: every `trak` whose `mdia`/`hdlr` handler type is `soun` is a sound
//! track, whatever is inside it.
//!
//! WHAT COUNTS IS WHAT PLAYS. An enabled track (`tkhd` flag `0x1`) counts; of the enabled tracks
//! that share a non-zero `alternate_group` only the first does, since a player picks one of them —
//! an iPhone's spatial recording carries its stereo AAC and its spatial track as such a pair, and
//! that is one voice, not two. A file whose sound tracks are all disabled counts one — the first: a
//! player that honours the flag plays nothing, but a transcode that drops them has still lost what a
//! player that ignores it would have played.
//!
//! MORE THAN ONE THAT PLAYS IS MIXED — a screen recording's app sound and microphone, a camera's
//! two microphones. The tracks a mix is made of are found in the DECODER's list of audio streams by
//! [`mix_map`], and that mapping is the one place this has to trust an order it cannot see; see
//! there.
//!
//! ANYTHING IT CANNOT WALK IS `None` — not ISO BMFF, a box that runs past its parent, a `trak` with
//! no `tkhd` or no handler, a `tkhd` of a version it does not know, a `moov` too big to be an index
//! — and `None` means "nobody knows", which leaves the transcode to decide as it did before Rule E.
//! Boxes must tile their parent, 64-bit sizes and size 0 ("to the end") are honoured, and the
//! `moov` may sit anywhere at the top level. Only the box headers before it are read, and then the
//! `moov` alone — never the media data. A sample entry it cannot read leaves that track's channels
//! and rate unknown; it does not make the track unknown.
//!
//! THIS IS WINDOWS' OWN READER, NOT `fc_text`'s. `fc_text::mp4_read::playing_sound_tracks` counts
//! by the same rule, but it reads a malformed index LENIENTLY — a `soun` track with no `tkhd` is a
//! disabled voice, a short or unknown-version `tkhd` is in no group, a child that does not fit ends
//! the walk — where this answers `None`; and it gives neither the tracks' channels and rates the mix
//! is mapped and sized by, nor a walk of the file to find the `moov`. The two agree on every
//! well-formed file; on a damaged one this one leaves the decision to the transcode (`None` →
//! [`SoundBefore::Transcode`]), which is the side the C# suite pins.

use std::collections::HashSet;
use std::io::{self, Read, Seek, SeekFrom};

/// What Rule E says of a video about to be transcoded, from its sound tracks (issue #98).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SoundBefore {
    /// The one-track transcode, as before Rule E: one sound track that plays and the decoder shows,
    /// none, or a count nobody could read.
    Transcode,
    /// More than one sound track plays: they are MIXED into the one AAC-LC track the profile
    /// allows (an `audiomixer` in the transcode's pipeline).
    Mix,
    /// A sound track plays that the decoder does not show: nothing here can decode it, so the
    /// original goes.
    Unheard,
}

/// One `soun` track as the container lists it, in `moov` order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SoundTrack {
    /// The `tkhd` `track_ID` — for the diagnostics log; nothing else reads it.
    pub track_id: u32,
    /// `tkhd` flag `0x1`.
    pub enabled: bool,
    /// The `tkhd` `alternate_group`; 0 is "no group".
    pub alternate_group: u16,
    /// The first sample entry's channel count, or `None` when it cannot be read.
    pub channels: Option<u32>,
    /// The first sample entry's sample rate in hertz, or `None` when it cannot be read.
    pub sample_rate: Option<u32>,
}

/// What the decoder reads of one audio stream — on Linux the discoverer's
/// `GstDiscovererAudioInfo` (`channels`, `sample_rate`, `bitrate`), each `None` (or 0) when it does
/// not state it — in the order it lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct DecodedTrack {
    pub channels: Option<u32>,
    pub sample_rate: Option<u32>,
    pub bitrate: Option<u32>,
}

/// What a mix is encoded from: the channel count, sample rate and total bit rate of the tracks
/// mixed. `channels`: 2 when any track is stereo or more, 1 when every one is mono, `None` when
/// none says and none is stereo. `sample_rate`: the highest any track has, or `None` when none
/// says. `bitrate`: the tracks' bit rates added up — rule B's cap for the one track they become —
/// or `None` when any is unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MixSound {
    pub channels: Option<i64>,
    pub sample_rate: Option<i64>,
    pub bitrate: Option<i64>,
}

/// The biggest `moov` read into memory. An hour of 4K is a few megabytes of index; anything past
/// this is not a file this walk vouches for, and it says so with `None` rather than allocating it.
pub const MAX_MOOV_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
struct Atom {
    start: usize,
    size: usize,
    header: usize,
    kind: [u8; 4],
}

// --- Rule E -------------------------------------------------------------------------------------

/// What Rule E says before a transcode: `playing` is what [`playing`] counted, `decoder_shows_audio`
/// whether the decoder's view of the streams has an audio stream. A count nobody could read is
/// [`SoundBefore::Transcode`] — behaviour as before Rule E, with `media_encoding::hides_audio` still
/// the guard it was. Sound the decoder does not show at all is [`SoundBefore::Unheard`] however many
/// tracks there are: there is nothing to mix it from.
pub fn before_transcode(playing: Option<usize>, decoder_shows_audio: bool) -> SoundBefore {
    match playing {
        Some(count) if count > 0 && !decoder_shows_audio => SoundBefore::Unheard,
        Some(count) if count > 1 => SoundBefore::Mix,
        _ => SoundBefore::Transcode,
    }
}

/// Whether a transcode LOST the sound: its source had a sound track that plays and the result has
/// none. That is a failed transcode (Rule C) however much smaller it is — Rule D never keeps a
/// silent result over a source with sound. A count that could not be read on either side says
/// nothing, and the read-back decides as before.
pub fn lost_sound(source: Option<usize>, result: Option<usize>) -> bool {
    source.is_some_and(|count| count > 0) && result == Some(0)
}

/// Whether a MIX came out as the one sound track the profile allows: exactly one in the result's
/// own track list. A count that cannot be read is not one — a mix is this file's own render, and one
/// it cannot check is not sent.
pub fn mix_came_out(result: Option<usize>) -> bool {
    result == Some(1)
}

/// Which of the decoder's audio streams (in its order) are the container's PLAYING tracks — or
/// `None` when that cannot be told, and the source goes as it is (Rule C).
///
/// BY POSITION, AND ONLY WHEN NOTHING IS MISSING. A decoder's stream carries no `track_ID` the
/// discoverer reports. `qtdemux` adds a pad for each track in the order the `moov` lists them, so
/// when the discoverer lists exactly as many audio streams as the container has `soun` tracks, the
/// n-th of one is the n-th of the other. When it lists FEWER — a sample entry it cannot read, the
/// very case Rule E exists for — which one is missing cannot be known, and a mix of the wrong ones
/// would keep fewer voices than the source has, which the protocol counts as a failed transcode.
///
/// AND CHECKED WHERE IT CAN BE. Where both sides state a channel count or a sample rate and they
/// differ, the order is not what it was assumed to be, and the answer is `None` too.
pub fn mix_map(container: &[SoundTrack], embedded: &[DecodedTrack]) -> Option<Vec<usize>> {
    if container.is_empty() || embedded.len() != container.len() {
        return None;
    }
    let disagree = container.iter().zip(embedded).any(|(track, decoded)| {
        differ(track.channels, decoded.channels) || differ(track.sample_rate, decoded.sample_rate)
    });
    if disagree {
        return None;
    }
    Some(playing_indexes(container))
}

/// What the mix of `mixed` is encoded from (Rule E): stereo when any of them is, at the highest
/// sample rate any of them has, capped by their bit rates added up. The decoder's numbers first,
/// the container's where those leave one out.
pub fn mix_format(mixed: impl IntoIterator<Item = (DecodedTrack, SoundTrack)>) -> MixSound {
    let tracks: Vec<(DecodedTrack, SoundTrack)> = mixed.into_iter().collect();
    let channels: Vec<Option<u32>> = tracks
        .iter()
        .map(|(decoded, container)| known(decoded.channels).or(known(container.channels)))
        .collect();
    let rates: Vec<Option<u32>> = tracks
        .iter()
        .map(|(decoded, container)| known(decoded.sample_rate).or(known(container.sample_rate)))
        .collect();
    let bitrates: Vec<Option<u32>> = tracks
        .iter()
        .map(|(decoded, _)| known(decoded.bitrate))
        .collect();
    let channels = if channels.iter().any(|count| count.is_some_and(|c| c >= 2)) {
        Some(2)
    } else if !channels.is_empty() && channels.iter().all(|count| *count == Some(1)) {
        Some(1)
    } else {
        None
    };
    MixSound {
        channels,
        // LINQ's Max over nullables: the nulls are left out, and none at all is null.
        sample_rate: rates.iter().flatten().max().map(|&rate| i64::from(rate)),
        bitrate: if !bitrates.is_empty() && bitrates.iter().all(Option::is_some) {
            Some(bitrates.iter().flatten().map(|&rate| i64::from(rate)).sum())
        } else {
            None
        },
    }
}

// --- counting -----------------------------------------------------------------------------------

/// The positions in `tracks` of the ones that PLAY: enabled, the first of each non-zero alternate
/// group, and the first track when none is enabled.
pub fn playing_indexes(tracks: &[SoundTrack]) -> Vec<usize> {
    let mut playing = Vec::new();
    let mut groups = HashSet::new();
    for (at, track) in tracks.iter().enumerate() {
        // Group 0 is "no group": every such track plays. A non-zero group plays one of its members.
        if track.enabled && (track.alternate_group == 0 || groups.insert(track.alternate_group)) {
            playing.push(at);
        }
    }
    if !tracks.is_empty() && playing.is_empty() {
        vec![0]
    } else {
        playing
    }
}

/// The sound tracks that play in `file`, or `None` when it cannot be walked.
pub fn playing(file: &[u8]) -> Option<usize> {
    read(file).map(|tracks| playing_indexes(&tracks).len())
}

/// The sound tracks that play in a seekable `file` ([`read_from`]); `Ok(None)` when it cannot be
/// walked or cannot seek, `Err` when a read fails.
pub fn playing_from<R: Read + Seek>(file: &mut R) -> io::Result<Option<usize>> {
    Ok(read_from(file)?.map(|tracks| playing_indexes(&tracks).len()))
}

/// Every `soun` track in `file`, in `moov` order, or `None` when it cannot be walked.
pub fn read(file: &[u8]) -> Option<Vec<SoundTrack>> {
    // A slice cannot fail to read or to seek.
    read_from(&mut io::Cursor::new(file)).ok().flatten()
}

/// Every `soun` track in a seekable `file`, in `moov` order — from its start, wherever it stands —
/// or `Ok(None)` when it cannot be walked: it cannot seek (the C#'s `CanSeek`: a seek to its end
/// fails), no `moov`, a box that does not fit, or a `moov` over [`MAX_MOOV_BYTES`]. Reads only the
/// top-level box headers up to the first `moov`, and that `moov`. A read that fails is `Err` — the
/// caller's to log (its kind only: the error's words may hold a path).
///
/// The shell calls it over the opened file (`std::fs::File`): the source before a transcode, and
/// each result.
pub fn read_from<R: Read + Seek>(file: &mut R) -> io::Result<Option<Vec<SoundTrack>>> {
    let Ok(end) = file.seek(SeekFrom::End(0)) else {
        return Ok(None);
    };
    let mut header = [0u8; 16];
    let mut at: u64 = 0;
    while at < end {
        if end - at < 8 {
            return Ok(None);
        }
        file.seek(SeekFrom::Start(at))?;
        file.read_exact(&mut header[..8])?;
        let mut size = u64::from(be32(&header[0..4]));
        let mut length: u64 = 8;
        if size == 1 {
            if end - at < 16 {
                return Ok(None);
            }
            file.read_exact(&mut header[8..16])?;
            // A 64-bit size past i64's range is clamped, as the C#'s long is.
            size = be64(&header[8..16]).min(i64::MAX as u64);
            length = 16;
        } else if size == 0 {
            size = end - at;
        }
        if size < length || size > end - at {
            return Ok(None);
        }
        if &header[4..8] == b"moov" {
            if size - length > MAX_MOOV_BYTES {
                return Ok(None);
            }
            let mut moov = vec![0u8; (size - length) as usize];
            file.read_exact(&mut moov)?;
            return Ok(read_moov(&moov));
        }
        at += size;
    }
    Ok(None)
}

/// The `soun` tracks among a `moov`'s `trak`s, given its body; `None` when one cannot be read.
fn read_moov(moov: &[u8]) -> Option<Vec<SoundTrack>> {
    let children = walk(moov, 0, moov.len())?;
    let mut sound = Vec::new();
    for child in children.iter().filter(|child| &child.kind == b"trak") {
        // `None` is a track that cannot be read; `Some(None)` one that is not a sound track.
        if let Some(track) = read_track(body(moov, child))? {
            sound.push(track);
        }
    }
    Some(sound)
}

/// One `trak`, given its body: `None` when its `tkhd` or its `mdia`/`hdlr` is missing or short;
/// otherwise `Some`, holding the track when it is a sound track and `None` when it is anything
/// else.
fn read_track(trak: &[u8]) -> Option<Option<SoundTrack>> {
    let children = walk(trak, 0, trak.len())?;
    let tkhd = first(&children, b"tkhd")?;
    let mdia = first(&children, b"mdia")?;
    let header = body(trak, &tkhd);
    // A full box: version (1) and flags (3), then creation, modification, track_ID, reserved and
    // duration — 32-bit times and duration in version 0, 64-bit in version 1 — then reserved (8)
    // and layer (2) before alternate_group.
    let (id_at, group_at) = match header.first() {
        Some(0) => (4 + 4 + 4, 4 + 4 + 4 + 4 + 4 + 4 + 8 + 2),
        Some(1) => (4 + 8 + 8, 4 + 8 + 8 + 4 + 4 + 8 + 8 + 2),
        _ => return None,
    };
    if header.len() < group_at + 2 {
        return None;
    }

    let media = body(trak, &mdia);
    let boxes = walk(media, 0, media.len())?;
    let hdlr = first(&boxes, b"hdlr")?;
    // A full box: version and flags (4), pre_defined — QuickTime's component type, "mhlr" — (4),
    // then handler_type.
    let handler = body(media, &hdlr);
    if handler.len() < 12 {
        return None;
    }
    if &handler[8..12] != b"soun" {
        return Some(None);
    }
    let (channels, sample_rate) = match first(&boxes, b"minf") {
        Some(minf) => sample_entry(body(media, &minf)),
        None => (None, None),
    };
    Some(Some(SoundTrack {
        track_id: be32(&header[id_at..id_at + 4]),
        enabled: header[3] & 0x1 != 0,
        alternate_group: be16(&header[group_at..group_at + 2]),
        channels,
        sample_rate,
    }))
}

/// The channel count and sample rate of the first sample entry in `minf`/`stbl`/`stsd`, each `None`
/// when it cannot be read. The layout is the ISO `AudioSampleEntry`, which is QuickTime's sound
/// description versions 0 and 1 too: six reserved bytes and the data reference index, a version
/// (QuickTime's; 0 in ISO), revision and vendor, then the channel count (2) at 16, sample size,
/// compression id and packet size, and the rate as 16.16 at 24. QuickTime's version 2 moves both:
/// the rate to a 64-bit float at 32 and the count to 32 bits at 40.
fn sample_entry(minf: &[u8]) -> (Option<u32>, Option<u32>) {
    let Some(stbl) = walk(minf, 0, minf.len()).and_then(|boxes| first(&boxes, b"stbl")) else {
        return (None, None);
    };
    let table = body(minf, &stbl);
    let Some(stsd) = walk(table, 0, table.len()).and_then(|boxes| first(&boxes, b"stsd")) else {
        return (None, None);
    };
    // A full box: version and flags (4), entry_count (4), then the entries, each a box.
    let descriptions = body(table, &stsd);
    if descriptions.len() < 8 || be32(&descriptions[4..8]) == 0 {
        return (None, None);
    }
    let Some(entry) = walk(descriptions, 8, descriptions.len())
        .and_then(|entries| entries.first().copied())
        .map(|first| body(descriptions, &first))
    else {
        return (None, None);
    };
    if entry.len() < 28 {
        return (None, None);
    }
    if be16(&entry[8..10]) == 2 {
        if entry.len() < 44 {
            return (None, None);
        }
        let hertz = f64::from_bits(be64(&entry[32..40]));
        // `Math.Round` is banker's: to even.
        let rate = (hertz.is_finite() && (1.0..=f64::from(u32::MAX)).contains(&hertz))
            .then(|| hertz.round_ties_even() as u32);
        return (known(Some(be32(&entry[40..44]))), rate);
    }
    (
        known(Some(u32::from(be16(&entry[16..18])))),
        known(Some(u32::from(be16(&entry[24..26])))),
    )
}

fn body<'a>(parent: &'a [u8], child: &Atom) -> &'a [u8] {
    &parent[child.start + child.header..child.start + child.size]
}

/// The first child of a kind, when it has a size — the C#'s `FirstOrDefault(…) is { Size: > 0 }`.
fn first(boxes: &[Atom], kind: &[u8; 4]) -> Option<Atom> {
    boxes
        .iter()
        .find(|child| &child.kind == kind)
        .copied()
        .filter(|child| child.size > 0)
}

fn known(value: Option<u32>) -> Option<u32> {
    value.filter(|&value| value > 0)
}

fn differ(container: Option<u32>, decoded: Option<u32>) -> bool {
    matches!((known(container), known(decoded)), (Some(one), Some(other)) if one != other)
}

/// The boxes laid end to end in `start..end`, as `faststart` walks them, or `None` when they do not
/// tile it. The one slack is QuickTime's: an atom list may end in a 32-bit zero terminator, so a
/// stub shorter than a header is let through when it is all zeros.
fn walk(data: &[u8], start: usize, end: usize) -> Option<Vec<Atom>> {
    let mut boxes = Vec::new();
    let mut at = start;
    while at < end {
        if end - at < 8 {
            return data[at..end].iter().all(|&byte| byte == 0).then_some(boxes);
        }
        let mut size = u64::from(be32(&data[at..at + 4]));
        let mut header = 8usize;
        if size == 1 {
            if end - at < 16 {
                return None;
            }
            size = be64(&data[at + 8..at + 16]).min(i64::MAX as u64);
            header = 16;
        } else if size == 0 {
            size = (end - at) as u64;
        }
        if size < header as u64 || size > (end - at) as u64 {
            return None;
        }
        let size = size as usize;
        let mut kind = [0u8; 4];
        kind.copy_from_slice(&data[at + 4..at + 8]);
        boxes.push(Atom {
            start: at,
            size,
            header,
            kind,
        });
        at += size;
    }
    Some(boxes)
}

fn be16(bytes: &[u8]) -> u16 {
    u16::from_be_bytes([bytes[0], bytes[1]])
}

fn be32(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

fn be64(bytes: &[u8]) -> u64 {
    let mut word = [0u8; 8];
    word.copy_from_slice(&bytes[..8]);
    u64::from_be_bytes(word)
}
