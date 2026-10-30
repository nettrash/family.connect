//! An animation's MP4 given its EXACT timing after the encoder has written it (`AnimationMp4.cs`;
//! docs/protocol.md, "Animations", "The file": "Each video frame lasts its ticks exactly — a
//! timescale that is a multiple of 30").
//!
//! WHY AFTERWARDS. An encoder counts time in its own units — Media Foundation in hundreds of
//! nanoseconds, GStreamer in nanoseconds — and a thirtieth of a second is a whole number of
//! neither: each sample is handed over at its tick rounded to the nearest unit, so its stated
//! length can be a unit off, and the timescale the muxer chooses is its own. The sample table is
//! ours to write, though: the video track's `mdhd` timescale becomes one that is a multiple of 30
//! (kept when it already is, 30 000 otherwise), its `stts` says each sample's ticks in it, and
//! every duration that follows from them — the track's, the movie's, the edit list's — is said
//! again. Composition offsets (`ctts`, which only B-frames need) and an edit's media time are
//! carried over into the new timescale.
//!
//! BEFORE THE INDEX MOVES. This runs on the file as the muxer wrote it, `moov` LAST: a `moov` that
//! grows or shrinks there moves no media data, so no chunk offset changes, and
//! [`faststart`](crate::faststart) then moves the finished index to the front as it does every
//! video's.
//!
//! ANYTHING IT CANNOT VOUCH FOR, IT LEAVES ALONE — `None`, and the file goes with the encoder's
//! own timing, which plays the same to within a unit a frame: a `moov` before media data, more
//! than one track, a sample count that is not the plan's (an encoder that dropped or doubled a
//! frame), a box it cannot walk.
//!
//! Where the C# reads past the end of a box or lets a `ulong`/`long` product wrap, this answers
//! `None` instead (an empty `tkhd` threw `IndexOutOfRangeException` out of the C#; a duration
//! or media time too large to rescale wrapped silently into a wrong one). Neither is a file an
//! encoder writes.

/// The timescale a video track is given when the encoder's own is not a multiple of 30.
pub const TIMESCALE: u32 = 30_000;

const CONTAINERS: [&[u8; 4]; 6] = [b"moov", b"trak", b"mdia", b"minf", b"stbl", b"edts"];

/// One box of the index: a leaf with its payload, or a container with its children.
#[derive(Debug, Clone)]
struct Node {
    kind: [u8; 4],
    payload: Option<Vec<u8>>,
    children: Option<Vec<Node>>,
}

impl Node {
    fn child(&self, kind: &[u8; 4]) -> Option<&Node> {
        self.children.as_ref()?.iter().find(|c| &c.kind == kind)
    }

    fn child_mut(&mut self, kind: &[u8; 4]) -> Option<&mut Node> {
        self.children.as_mut()?.iter_mut().find(|c| &c.kind == kind)
    }

    fn count(&self, kind: &[u8; 4]) -> usize {
        self.children.as_ref().map_or(0, |children| {
            children.iter().filter(|c| &c.kind == kind).count()
        })
    }

    fn payload(&self) -> Option<&[u8]> {
        self.payload.as_deref()
    }

    /// The payload of the leaf at `path` below this node.
    fn payload_at(&mut self, path: &[&[u8; 4]]) -> Option<&mut Vec<u8>> {
        let mut node = self;
        for kind in path {
            node = node.child_mut(kind)?;
        }
        node.payload.as_mut()
    }
}

/// `file` with its one video track timed by `ticks` — one entry per sample, in 1/30 s — or `None`
/// when that cannot be done safely.
pub fn exact_ticks(file: &[u8], ticks: &[i32]) -> Option<Vec<u8>> {
    if ticks.is_empty() || ticks.iter().any(|&tick| tick <= 0) {
        return None;
    }
    let top = walk(file, 0, file.len())?;
    let mut moovs = top.iter().filter(|b| &b.kind == b"moov");
    let moov_box = *moovs.next()?;
    if moovs.next().is_some() {
        return None;
    }
    // Every byte of media data before the index: then a moov of another size moves nothing a
    // chunk offset names.
    if top
        .iter()
        .any(|b| &b.kind == b"mdat" && b.start > moov_box.start)
    {
        return None;
    }
    let mut moov = parse(
        b"moov",
        &file[moov_box.start + moov_box.header..moov_box.start + moov_box.size],
    )?;
    let mvhd = moov.child(b"mvhd")?.payload()?.to_vec();
    if moov.count(b"trak") != 1 {
        return None;
    }
    let trak = moov.child(b"trak")?;
    let mdia = trak.child(b"mdia");
    let stbl = mdia
        .and_then(|m| m.child(b"minf"))
        .and_then(|m| m.child(b"stbl"));
    let tkhd = trak.child(b"tkhd")?.payload()?.to_vec();
    let mdia = mdia?;
    let mdhd = mdia.child(b"mdhd")?.payload()?.to_vec();
    let hdlr = mdia.child(b"hdlr")?.payload()?;
    if hdlr.len() < 12 || &hdlr[8..12] != b"vide" {
        return None;
    }
    let stbl = stbl?;
    stbl.child(b"stts")?;
    let stsz = stbl.child(b"stsz")?.payload()?;
    if stsz.len() < 12 {
        return None;
    }
    if u64::from(read_u32(stsz, 8)) != ticks.len() as u64 {
        return None;
    }
    let ctts = stbl
        .child(b"ctts")
        .and_then(Node::payload)
        .map(<[u8]>::to_vec);
    let elst = trak
        .child(b"edts")
        .and_then(|e| e.child(b"elst"))
        .and_then(Node::payload)
        .map(<[u8]>::to_vec);

    let movie_scale = field(&mvhd, timescale_at(&mvhd))?;
    let old_scale = field(&mdhd, timescale_at(&mdhd))?;
    if movie_scale == 0 || old_scale == 0 {
        return None;
    }
    let new_scale = if old_scale % 30 == 0 {
        old_scale
    } else {
        TIMESCALE
    };
    let per = u64::from(new_scale / 30);
    // Each tick is a positive i32, so the sum of any slice a file can hold fits.
    let total: u64 = ticks.iter().map(|&tick| tick as u64).sum();
    let media_duration = total.checked_mul(per)?;
    let movie_duration = rescale(total, u64::from(movie_scale), 30)?;
    // The sample table: each sample's ticks, run-length coded.
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for &tick in ticks {
        let delta = u32::try_from(tick as u64 * per).ok()?;
        match runs.last_mut() {
            Some((count, last)) if *last == delta => *count += 1,
            _ => runs.push((1, delta)),
        }
    }
    let mut table = vec![0u8; 8 + 8 * runs.len()];
    write_u32(&mut table, 4, u32::try_from(runs.len()).ok()?);
    for (run, (count, delta)) in runs.iter().enumerate() {
        write_u32(&mut table, 8 + 8 * run, *count);
        write_u32(&mut table, 12 + 8 * run, *delta);
    }

    let mut mdhd = mdhd;
    let mut tkhd = tkhd;
    let mut mvhd = mvhd;
    let mdhd_scale_at = timescale_at(&mdhd);
    let mvhd_scale_at = timescale_at(&mvhd);
    let tkhd_duration_at = tkhd_duration_at(&tkhd);
    if !set_time(&mut mdhd, mdhd_scale_at, u64::from(new_scale))
        || !set_time(&mut mdhd, mdhd_scale_at + 4, media_duration)
        || !set_time(&mut tkhd, tkhd_duration_at, movie_duration)
        || !set_time(&mut mvhd, mvhd_scale_at + 4, movie_duration)
    {
        return None;
    }
    let ctts = match ctts {
        Some(mut ctts) if new_scale != old_scale => {
            if !rescale_offsets(&mut ctts, old_scale, new_scale) {
                return None;
            }
            Some(ctts)
        }
        _ => None,
    };
    let elst = match elst {
        Some(mut elst) => {
            if !edit(&mut elst, old_scale, new_scale, movie_duration) {
                return None;
            }
            Some(elst)
        }
        None => None,
    };

    // Every leaf below was found above, so each path is there.
    const STBL: [&[u8; 4]; 4] = [b"trak", b"mdia", b"minf", b"stbl"];
    *moov.payload_at(&[b"mvhd"])? = mvhd;
    *moov.payload_at(&[b"trak", b"tkhd"])? = tkhd;
    *moov.payload_at(&[b"trak", b"mdia", b"mdhd"])? = mdhd;
    let stbl = {
        let mut node = &mut moov;
        for kind in STBL {
            node = node.child_mut(kind)?;
        }
        node
    };
    stbl.child_mut(b"stts")?.payload = Some(table);
    if let Some(ctts) = ctts {
        *stbl.child_mut(b"ctts")?.payload.as_mut()? = ctts;
    }
    if let Some(elst) = elst {
        *moov.payload_at(&[b"trak", b"edts", b"elst"])? = elst;
    }

    let written = serialize(&moov)?;
    let mut result = Vec::with_capacity(file.len() - moov_box.size + written.len());
    result.extend_from_slice(&file[..moov_box.start]);
    result.extend_from_slice(&written);
    result.extend_from_slice(&file[moov_box.start + moov_box.size..]);
    Some(result)
}

/// The video track's timescale and its samples' durations as a file states them — what a test,
/// or a diagnostics line, reads back. `None` when there is no single video track to read.
pub fn timing(file: &[u8]) -> Option<(u32, Vec<u32>)> {
    let top = walk(file, 0, file.len())?;
    let moov_box = top.iter().find(|b| &b.kind == b"moov")?;
    let moov = parse(
        b"moov",
        &file[moov_box.start + moov_box.header..moov_box.start + moov_box.size],
    )?;
    let mdia = moov.child(b"trak")?.child(b"mdia")?;
    let mdhd = mdia.child(b"mdhd")?.payload()?;
    let stts = mdia
        .child(b"minf")?
        .child(b"stbl")?
        .child(b"stts")?
        .payload()?;
    if stts.len() < 8 {
        return None;
    }
    let scale = field(mdhd, timescale_at(mdhd))?;
    let mut durations = Vec::new();
    let count = read_u32(stts, 4) as usize;
    let mut entry = 0;
    while entry < count && 16 + 8 * entry <= stts.len() {
        let samples = read_u32(stts, 8 + 8 * entry);
        let delta = read_u32(stts, 12 + 8 * entry);
        let mut sample = 0u32;
        while sample < samples && durations.len() < 1_000_000 {
            durations.push(delta);
            sample += 1;
        }
        entry += 1;
    }
    Some((scale, durations))
}

/// Where a full box's timescale sits: after version, flags and two dates of 4 bytes (version 0)
/// or 8 (version 1).
fn timescale_at(full: &[u8]) -> usize {
    if full.first() == Some(&1) { 20 } else { 12 }
}

/// Where tkhd's duration sits: after the dates, the track id and a reserved word.
fn tkhd_duration_at(tkhd: &[u8]) -> usize {
    if tkhd.first() == Some(&1) { 28 } else { 20 }
}

/// A 4-byte field of a full box, or `None` when it does not fit. (The C#'s 8-byte reading is
/// never asked for.)
fn field(full: &[u8], at: usize) -> Option<u32> {
    (at + 4 <= full.len()).then(|| read_u32(full, at))
}

/// A time field of a full box: 4 bytes in version 0, 8 in version 1 — except a timescale, which
/// is 4 in both (the field right after the dates). `false` when it does not fit.
fn set_time(full: &mut [u8], at: usize, value: u64) -> bool {
    let wide = full.first() == Some(&1) && at != timescale_at(full);
    if at + if wide { 8 } else { 4 } > full.len() {
        return false;
    }
    if wide {
        full[at..at + 8].copy_from_slice(&value.to_be_bytes());
        return true;
    }
    let Ok(value) = u32::try_from(value) else {
        return false;
    };
    write_u32(full, at, value);
    true
}

/// `value` in `from` units, said in `to` units, rounded half up; `None` past `u64`.
fn rescale(value: u64, to: u64, from: u64) -> Option<u64> {
    Some(value.checked_mul(to)?.checked_add(from / 2)? / from)
}

/// A signed time from `from` units into `to`, its size rounded half up; `None` past `i64`.
fn rescale_signed(value: i64, from: u32, to: u32) -> Option<i64> {
    let scaled = value
        .checked_abs()?
        .checked_mul(i64::from(to))?
        .checked_add(i64::from(from / 2))?
        / i64::from(from);
    Some(if value < 0 { -scaled } else { scaled })
}

fn rescale_offsets(ctts: &mut [u8], from: u32, to: u32) -> bool {
    if ctts.len() < 8 {
        return false;
    }
    let signed = ctts[0] == 1;
    let count = read_u32(ctts, 4) as usize;
    if count > (ctts.len() - 8) / 8 {
        return false;
    }
    for entry in 0..count {
        let at = 12 + 8 * entry;
        let raw = read_u32(ctts, at);
        let offset = if signed {
            i64::from(raw as i32)
        } else {
            i64::from(raw)
        };
        let Some(scaled) = rescale_signed(offset, from, to) else {
            return false;
        };
        if signed {
            let Ok(scaled) = i32::try_from(scaled) else {
                return false;
            };
            write_u32(ctts, at, scaled as u32);
        } else {
            let Ok(scaled) = u32::try_from(scaled) else {
                return false;
            };
            write_u32(ctts, at, scaled);
        }
    }
    true
}

/// The edit list: its media times into the new timescale, and a single edit's length the clip's.
fn edit(elst: &mut [u8], from: u32, to: u32, movie_duration: u64) -> bool {
    if elst.len() < 8 {
        return false;
    }
    let wide = elst[0] == 1;
    let size = if wide { 20 } else { 12 };
    let count = read_u32(elst, 4) as usize;
    if count > (elst.len() - 8) / size {
        return false;
    }
    for entry in 0..count {
        let at = 8 + size * entry;
        if count == 1 {
            if wide {
                elst[at..at + 8].copy_from_slice(&movie_duration.to_be_bytes());
            } else if let Ok(duration) = u32::try_from(movie_duration) {
                write_u32(elst, at, duration);
            } else {
                return false;
            }
        }
        let media_at = at + if wide { 8 } else { 4 };
        let media = if wide {
            read_u64(elst, media_at) as i64
        } else {
            i64::from(read_u32(elst, media_at) as i32)
        };
        if media < 0 || from == to {
            // −1 is an empty edit: no media time to carry.
            continue;
        }
        let Some(scaled) = rescale_signed(media, from, to) else {
            return false;
        };
        if wide {
            elst[media_at..media_at + 8].copy_from_slice(&scaled.to_be_bytes());
        } else if let Ok(scaled) = i32::try_from(scaled) {
            write_u32(elst, media_at, scaled as u32);
        } else {
            return false;
        }
    }
    true
}

#[derive(Debug, Clone, Copy)]
struct Box {
    start: usize,
    size: usize,
    header: usize,
    kind: [u8; 4],
}

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
        if size == 1 {
            if end - at < 16 {
                return None;
            }
            size = read_u64(data, at + 8);
            header = 16;
        } else if size == 0 {
            size = (end - at) as u64;
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
        });
        at += size;
    }
    Some(boxes)
}

fn parse(kind: &[u8; 4], payload: &[u8]) -> Option<Node> {
    if !CONTAINERS.contains(&kind) {
        return Some(Node {
            kind: *kind,
            payload: Some(payload.to_vec()),
            children: None,
        });
    }
    let boxes = walk(payload, 0, payload.len())?;
    let mut children = Vec::with_capacity(boxes.len());
    for b in boxes {
        children.push(parse(
            &b.kind,
            &payload[b.start + b.header..b.start + b.size],
        )?);
    }
    Some(Node {
        kind: *kind,
        payload: None,
        children: Some(children),
    })
}

/// The box and everything in it, each with an ordinary 32-bit size; `None` for a box of 4 GiB or
/// more, which no index is.
fn serialize(node: &Node) -> Option<Vec<u8>> {
    let mut output = Vec::new();
    write(&mut output, node)?;
    Some(output)
}

fn write(output: &mut Vec<u8>, node: &Node) -> Option<()> {
    let body = match &node.children {
        Some(children) => {
            let mut inner = Vec::new();
            for child in children {
                write(&mut inner, child)?;
            }
            inner
        }
        None => node.payload.clone().unwrap_or_default(),
    };
    let size = u32::try_from(body.len() + 8).ok()?;
    output.extend_from_slice(&size.to_be_bytes());
    output.extend_from_slice(&node.kind);
    output.extend_from_slice(&body);
    Some(())
}

fn read_u32(data: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
}

fn read_u64(data: &[u8], at: usize) -> u64 {
    let mut bytes = [0; 8];
    bytes.copy_from_slice(&data[at..at + 8]);
    u64::from_be_bytes(bytes)
}

fn write_u32(data: &mut [u8], at: usize, value: u32) {
    data[at..at + 4].copy_from_slice(&value.to_be_bytes());
}
