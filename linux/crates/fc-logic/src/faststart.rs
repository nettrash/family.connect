//! An MP4's `moov` box moved in front of its media data (`Faststart.cs`) — what ffmpeg calls
//! `+faststart` and qt-faststart does on its own. The protocol's profile asks for it: "`moov` before
//! `mdat` — so a player can start on the first bytes of a `Range` read" (docs/protocol.md,
//! "Preparing media before upload").
//!
//! WHY IT IS DONE HERE. Windows' Media Foundation MP4 sink writes the media data as it arrives and
//! the index when it finishes; on Linux every encode goes through `mp4mux faststart=true`
//! (carry-forward: else `moov` is last), but a file that arrives from anywhere else — a picked
//! clip kept as it is, an animation's re-timed MP4 (`animation_mp4`) — is checked and, when it
//! needs it, rearranged here. Rearranging the finished bytes is the same answer on every machine,
//! and it can be tested anywhere.
//!
//! MOVING THE INDEX MOVES EVERYTHING IT POINTS AT. The chunk offsets in every track's `stco`
//! (32-bit) or `co64` (64-bit) are ABSOLUTE file positions, and every byte between the first
//! `mdat` and the old `moov` slides forward by the size of the `moov`. So exactly those offsets
//! grow by it and no others — an `mdat` that was already after the `moov` stays where it was. A
//! 32-bit offset that would no longer fit is not converted to 64 bits: this gives up instead,
//! which for a file under the 100 MB ceiling cannot happen.
//!
//! ANYTHING IT DOES NOT UNDERSTAND, IT LEAVES ALONE. A top-level box it cannot vouch for —
//! fragments (`moof`, `sidx`), a `meta` with its own absolute offsets — could point into what it
//! moves, so the answer is `None` rather than a file that plays until somebody seeks.

use std::borrow::Cow;

/// Top-level boxes that hold no absolute file position, and so may slide without being rewritten.
const MOVABLE: [&[u8; 4]; 9] = [
    b"ftyp", b"moov", b"mdat", b"free", b"skip", b"wide", b"uuid", b"udta", b"pdin",
];

/// The boxes a chunk-offset table can sit inside: moov → trak → mdia → minf → stbl.
const CONTAINERS: [&[u8; 4]; 4] = [b"trak", b"mdia", b"minf", b"stbl"];

#[derive(Debug, Clone, Copy)]
struct Box {
    start: usize,
    size: usize,
    header: usize,
    kind: [u8; 4],
    /// A size of 0: "until the end of the file".
    to_end: bool,
}

/// Whether the `moov` comes before the first `mdat`. `None` when the top level cannot be walked,
/// or has no `moov` or no `mdat` to compare.
pub fn is_moov_first(file: &[u8]) -> Option<bool> {
    let boxes = walk(file, 0, file.len())?;
    let moov = boxes.iter().position(|b| &b.kind == b"moov")?;
    let mdat = boxes.iter().position(|b| &b.kind == b"mdat")?;
    Some(moov < mdat)
}

/// The file with its `moov` in front of its media data: the SAME bytes (`Cow::Borrowed`) when it
/// already is, a rearranged copy of the same length when it was not, and `None` when that cannot
/// be done safely.
pub fn moov_first(file: &[u8]) -> Option<Cow<'_, [u8]>> {
    let boxes = walk(file, 0, file.len())?;
    let mut moovs = boxes.iter().filter(|b| &b.kind == b"moov");
    let moov = *moovs.next()?;
    if moovs.next().is_some() {
        return None;
    }
    let mdat = *boxes.iter().find(|b| &b.kind == b"mdat")?;
    if moov.start < mdat.start {
        return Some(Cow::Borrowed(file));
    }
    if boxes.iter().any(|b| !MOVABLE.contains(&&b.kind)) {
        return None;
    }
    let mut index = file[moov.start..moov.start + moov.size].to_vec();
    if moov.to_end {
        // "Until the end of the file" stops being true the moment it is not at the end.
        let size = u32::try_from(moov.size).ok()?;
        index[..4].copy_from_slice(&size.to_be_bytes());
    }
    let length = index.len();
    if !shift(
        &mut index,
        moov.header,
        length,
        mdat.start as u64,
        moov.start as u64,
        moov.size as u64,
    ) {
        return None;
    }
    let mut arranged = Vec::with_capacity(file.len());
    arranged.extend_from_slice(&file[..mdat.start]);
    arranged.extend_from_slice(&index);
    arranged.extend_from_slice(&file[mdat.start..moov.start]);
    arranged.extend_from_slice(&file[moov.start + moov.size..]);
    Some(Cow::Owned(arranged))
}

/// Every chunk offset under `start..end` of `index` that points at or after `from` and before
/// `until`, moved on by `by`. `false` when a box inside is malformed or a 32-bit offset would
/// overflow.
fn shift(index: &mut [u8], start: usize, end: usize, from: u64, until: u64, by: u64) -> bool {
    let Some(children) = walk(index, start, end) else {
        return false;
    };
    for child in children {
        let body = child.start + child.header;
        let body_end = child.start + child.size;
        if CONTAINERS.contains(&&child.kind) {
            if !shift(index, body, body_end, from, until, by) {
                return false;
            }
            continue;
        }
        let width = match &child.kind {
            b"stco" => 4,
            b"co64" => 8,
            _ => continue,
        };
        // A full box: version and flags, the count, then the offsets.
        if body_end - body < 8 {
            return false;
        }
        let count = read_u32(index, body + 4) as usize;
        if count > (body_end - body - 8) / width {
            return false;
        }
        for entry in 0..count {
            let at = body + 8 + entry * width;
            if width == 4 {
                let offset = u64::from(read_u32(index, at));
                if offset >= from && offset < until {
                    let Ok(moved) = u32::try_from(offset + by) else {
                        return false;
                    };
                    index[at..at + 4].copy_from_slice(&moved.to_be_bytes());
                }
            } else {
                let offset = read_u64(index, at);
                if offset >= from && offset < until {
                    // `offset < until`, a position inside the file, so this cannot overflow.
                    index[at..at + 8].copy_from_slice(&(offset + by).to_be_bytes());
                }
            }
        }
    }
    true
}

/// The boxes laid end to end in `start..end`, or `None` when they do not tile it exactly: a size
/// shorter than its own header, one running past the end, or a stub too short to be a header.
fn walk(data: &[u8], start: usize, end: usize) -> Option<Vec<Box>> {
    let mut boxes = Vec::new();
    let mut at = start;
    while at < end {
        if end - at < 8 {
            return None;
        }
        let mut size = u64::from(read_u32(data, at));
        let kind = [data[at + 4], data[at + 5], data[at + 6], data[at + 7]];
        let mut header = 8;
        let mut to_end = false;
        if size == 1 {
            if end - at < 16 {
                return None;
            }
            size = read_u64(data, at + 8);
            header = 16;
        } else if size == 0 {
            size = (end - at) as u64;
            to_end = true;
        }
        if size < header as u64 || size > (end - at) as u64 {
            return None;
        }
        // At most `end - at`, so it fits.
        let size = size as usize;
        boxes.push(Box {
            start: at,
            size,
            header,
            kind,
            to_end,
        });
        at += size;
    }
    Some(boxes)
}

fn read_u32(data: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
}

fn read_u64(data: &[u8], at: usize) -> u64 {
    let mut bytes = [0; 8];
    bytes.copy_from_slice(&data[at..at + 8]);
    u64::from_be_bytes(bytes)
}
