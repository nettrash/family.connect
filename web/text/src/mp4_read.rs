//! Reading an MP4 or QuickTime file's index — what a picked video or sound
//! file holds, where each frame of it is, and how a decoder is to be set up
//! for it (docs/protocol.md, "Preparing media before upload").
//!
//! A browser can DECODE a frame (WebCodecs) but it cannot find one: there is
//! no demuxer in the platform, only a `<video>` element that plays in real
//! time and says nothing about the stream it is playing. So this reads the
//! `moov` box — the index — and turns it into what a decoder needs: every
//! sample's offset, size, decode and presentation time and whether it is a
//! key frame, the codec's own configuration record, the rotation a phone
//! wrote into the track, and the edit that says where the presentation
//! starts.
//!
//! It never touches the media. The file lives in a `Blob` that may be a
//! gigabyte; the caller finds the top-level boxes with [`box_header`], hands
//! [`parse_moov`] the `moov` alone, and then reads each sample's bytes itself
//! by the offsets this returns. Every length in the file is checked against
//! what is actually there — a malformed index is an error, never a panic or
//! a read past the end.

// --- boxes -------------------------------------------------------------------------------------

/// A box header: the four-character type and how long the box is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoxHeader {
    pub kind: [u8; 4],
    /// 8, or 16 for a box whose size needs 64 bits.
    pub header_len: u64,
    /// The whole box, header included — None for the last box of a file
    /// whose size field says "to the end".
    pub size: Option<u64>,
}

/// The header at the start of `bytes` (at least 8 of them; 16 when the size
/// is 64-bit), or None if they are not one.
pub fn box_header(bytes: &[u8]) -> Option<BoxHeader> {
    if bytes.len() < 8 {
        return None;
    }
    let size = u32::from_be_bytes(bytes[0..4].try_into().ok()?);
    let kind: [u8; 4] = bytes[4..8].try_into().ok()?;
    match size {
        0 => Some(BoxHeader {
            kind,
            header_len: 8,
            size: None,
        }),
        1 => {
            let large = u64::from_be_bytes(bytes.get(8..16)?.try_into().ok()?);
            (large >= 16).then_some(BoxHeader {
                kind,
                header_len: 16,
                size: Some(large),
            })
        }
        2..=7 => None,
        size => Some(BoxHeader {
            kind,
            header_len: 8,
            size: Some(u64::from(size)),
        }),
    }
}

/// The children of a container's body, in order, each as `(type, body)`.
/// Stops at the first child that does not fit, rather than reading past it.
fn children(body: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut found = Vec::new();
    let mut at = 0usize;
    while at + 8 <= body.len() {
        let Some(header) = box_header(&body[at..]) else {
            break;
        };
        let size = match header.size {
            Some(size) => size,
            None => (body.len() - at) as u64,
        };
        let Some(end) = at.checked_add(size as usize) else {
            break;
        };
        if size as usize as u64 != size || end > body.len() {
            break;
        }
        found.push((header.kind, &body[at + header.header_len as usize..end]));
        at = end;
    }
    found
}

fn child<'a>(body: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    children(body)
        .into_iter()
        .find(|(found, _)| found == kind)
        .map(|(_, body)| body)
}

/// A cursor over big-endian fields that answers None instead of reading past
/// the end.
struct Fields<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Fields<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Fields { bytes, at: 0 }
    }

    fn take(&mut self, count: usize) -> Option<&'a [u8]> {
        let end = self.at.checked_add(count)?;
        let slice = self.bytes.get(self.at..end)?;
        self.at = end;
        Some(slice)
    }

    fn skip(&mut self, count: usize) -> Option<()> {
        self.take(count).map(|_| ())
    }

    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|bytes| bytes[0])
    }

    fn u16(&mut self) -> Option<u16> {
        self.take(2)
            .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    fn u32(&mut self) -> Option<u32> {
        self.take(4)
            .map(|bytes| u32::from_be_bytes(bytes.try_into().unwrap()))
    }

    fn i32(&mut self) -> Option<i32> {
        self.u32().map(|value| value as i32)
    }

    fn u64(&mut self) -> Option<u64> {
        self.take(8)
            .map(|bytes| u64::from_be_bytes(bytes.try_into().unwrap()))
    }

    fn rest(&self) -> &'a [u8] {
        &self.bytes[self.at.min(self.bytes.len())..]
    }
}

// --- what a file holds -------------------------------------------------------------------------

/// Why an index could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadError {
    /// The bytes are not a `moov`, or one of its tables contradicts another
    /// or runs past its box.
    Malformed,
    /// A fragmented file: its samples are indexed in `moof` boxes spread
    /// through the media, not in the `moov`. No camera writes one; a
    /// browser's own recorder does.
    Fragmented,
    /// More samples than any real clip has — a table that would take more
    /// memory to expand than a page should spend on a guess.
    TooLarge,
}

/// The most samples one track may have: ten hours at 240 fps.
const MAX_SAMPLES: u64 = 10 * 60 * 60 * 240;

/// One sample: where its bytes are, and when it is decoded and shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sample {
    pub offset: u64,
    pub size: u32,
    /// Decode time, in the track's timescale.
    pub dts: i64,
    /// Presentation time — decode time plus the composition offset.
    pub pts: i64,
    pub duration: u32,
    pub sync: bool,
}

/// A `colr` box's `nclx` code points (ISO/IEC 23091-2). Transfer 16 is PQ
/// and 18 is HLG — HDR, which the profile tone-maps rather than passes on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Colour {
    pub primaries: u16,
    pub transfer: u16,
    pub matrix: u16,
    pub full_range: bool,
}

impl Colour {
    pub fn is_hdr(&self) -> bool {
        matches!(self.transfer, 16 | 18)
    }
}

/// A track's first sample description: its format and what its decoder
/// needs.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SampleEntry {
    /// The four-character format: `avc1`, `hvc1`, `mp4a`, `alac`, …
    pub format: [u8; 4],
    /// A video's coded size, as the entry states it.
    pub width: u16,
    pub height: u16,
    /// A sound's channels and rate, as the entry states them (QuickTime's
    /// version 1 and 2 layouts included).
    pub channels: u32,
    pub sample_rate: u32,
    /// The codec's configuration record: `avcC`, `hvcC`, `av1C` or `vpcC` for
    /// video; the AudioSpecificConfig from the `esds` for AAC.
    pub config: Vec<u8>,
    /// The `esds`'s object type — 0x40 is MPEG-4 audio (AAC), 0x6B and 0x69
    /// are MP3 — or 0 when there is none.
    pub object_type: u8,
    /// A rate the entry states (`btrt`, or the `esds`'s average), bit/s;
    /// None when it states none.
    pub stated_bitrate: Option<u64>,
    pub colour: Option<Colour>,
}

/// A track's edit list, reduced to what a player does with the common one: a
/// presentation that starts `media_start` into the media — past an
/// encoder's priming, or a reordering decoder's delay — after `lead` of
/// nothing, and lasts `length` (None: to the end of the media).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Edit {
    /// In the track's timescale.
    pub media_start: i64,
    /// In the MOVIE's timescale.
    pub lead: u64,
    pub length: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    pub id: u32,
    /// `vide`, `soun`, or anything else (timecode, metadata), which is
    /// ignored.
    pub handler: [u8; 4],
    pub enabled: bool,
    pub timescale: u32,
    /// The media's length, in its own timescale (`mdhd`).
    pub duration: u64,
    /// The presentation size `tkhd` states, before its matrix.
    pub width: u32,
    pub height: u32,
    /// Clockwise quarter-turns the matrix asks for — 0, 90, 180 or 270 — or
    /// None for a matrix that is not a plain rotation (a mirror, a skew).
    pub rotation: Option<u16>,
    pub entry: Option<SampleEntry>,
    pub samples: Vec<Sample>,
    /// None without an edit list, or with one this reader does not reduce
    /// to a single [`Edit`] (see [`Track::edit_supported`]).
    pub edit: Option<Edit>,
    /// Whether the edit list had a shape beyond one empty lead and one
    /// media segment — several spliced segments, or a rate other than 1.
    pub complex_edit: bool,
}

impl Track {
    /// The bytes of every sample, and so the stream's rate in bit/s over the
    /// media's length — what the index itself says the track is, to the
    /// byte. None when there is no length to divide by.
    pub fn data_rate(&self) -> Option<u64> {
        let ticks: u64 = self
            .samples
            .iter()
            .map(|sample| u64::from(sample.duration))
            .sum();
        let bytes: u64 = self
            .samples
            .iter()
            .map(|sample| u64::from(sample.size))
            .sum();
        if ticks == 0 || self.timescale == 0 {
            return None;
        }
        let rate = u128::from(bytes) * 8 * u128::from(self.timescale) / u128::from(ticks);
        u64::try_from(rate).ok().filter(|&rate| rate > 0)
    }

    /// Frames per second: the samples over the time they cover. A variable
    /// frame rate reads as its average — the same thing a platform's
    /// "nominal" rate is.
    pub fn frame_rate(&self) -> Option<f64> {
        let ticks: u64 = self
            .samples
            .iter()
            .map(|sample| u64::from(sample.duration))
            .sum();
        if ticks == 0 || self.timescale == 0 || self.samples.is_empty() {
            return None;
        }
        Some(self.samples.len() as f64 * f64::from(self.timescale) / ticks as f64)
    }

    pub fn edit_supported(&self) -> bool {
        !self.complex_edit
    }

    fn is(&self, handler: &[u8; 4]) -> bool {
        &self.handler == handler
    }
}

/// A file's index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Movie {
    pub timescale: u32,
    /// In the movie's timescale (`mvhd`).
    pub duration: u64,
    pub tracks: Vec<Track>,
}

impl Movie {
    /// The first enabled video track with a sample description — a phone's
    /// camera track. (Cinematic and depth tracks come after it.)
    pub fn video(&self) -> Option<&Track> {
        self.tracks
            .iter()
            .find(|track| track.is(b"vide") && track.enabled && track.entry.is_some())
            .or_else(|| {
                self.tracks
                    .iter()
                    .find(|track| track.is(b"vide") && track.entry.is_some())
            })
    }

    /// The first sound track with samples in it.
    pub fn audio(&self) -> Option<&Track> {
        self.tracks
            .iter()
            .find(|track| track.is(b"soun") && track.enabled && !track.samples.is_empty())
            .or_else(|| {
                self.tracks
                    .iter()
                    .find(|track| track.is(b"soun") && !track.samples.is_empty())
            })
    }

    /// How long `track` PLAYS, in milliseconds: the length its edit list
    /// presents, which leaves out an AAC encoder's priming and padding, or
    /// the media's own length without one.
    pub fn presented_ms(&self, track: &Track) -> Option<u64> {
        if let Some(Edit {
            length: Some(length),
            ..
        }) = track.edit
        {
            if self.timescale > 0 {
                return Some(length * 1000 / u64::from(self.timescale)).filter(|&ms| ms > 0);
            }
        }
        (track.timescale > 0 && track.duration > 0)
            .then(|| track.duration * 1000 / u64::from(track.timescale))
    }

    /// How long `track` shows NOTHING before it starts, in milliseconds:
    /// the empty edit in front of its media, which is how a file says one
    /// track starts later than the other. 0 without one. Whoever writes
    /// the track out again has to write this too (it is
    /// [`crate::mp4::Track`]'s `lead`), or the picture and its sound come
    /// out this far apart. None when it is not a length a file can state.
    pub fn lead_ms(&self, track: &Track) -> Option<u32> {
        let lead = track.edit.map_or(0, |edit| edit.lead);
        if lead == 0 {
            return Some(0);
        }
        if self.timescale == 0 {
            return None;
        }
        let timescale = u128::from(self.timescale);
        u32::try_from((u128::from(lead) * 1000 + timescale / 2) / timescale).ok()
    }

    /// The whole file's length in milliseconds, or None.
    pub fn duration_ms(&self) -> Option<u64> {
        if self.timescale > 0 && self.duration > 0 {
            return Some(self.duration * 1000 / u64::from(self.timescale));
        }
        self.tracks
            .iter()
            .filter(|track| track.timescale > 0)
            .map(|track| track.duration * 1000 / u64::from(track.timescale))
            .max()
            .filter(|&ms| ms > 0)
    }
}

// --- parsing -----------------------------------------------------------------------------------

/// Read a whole `moov` box — header included — into a [`Movie`].
pub fn parse_moov(moov: &[u8]) -> Result<Movie, ReadError> {
    let header = box_header(moov).ok_or(ReadError::Malformed)?;
    if &header.kind != b"moov" {
        return Err(ReadError::Malformed);
    }
    let end = header
        .size
        .map(|size| size as usize)
        .unwrap_or(moov.len())
        .min(moov.len());
    let body = moov
        .get(header.header_len as usize..end)
        .ok_or(ReadError::Malformed)?;
    let boxes = children(body);
    if boxes.iter().any(|(kind, _)| kind == b"mvex") {
        return Err(ReadError::Fragmented);
    }
    let mvhd = boxes
        .iter()
        .find(|(kind, _)| kind == b"mvhd")
        .map(|(_, body)| *body)
        .ok_or(ReadError::Malformed)?;
    let (timescale, duration) = mvhd_fields(mvhd).ok_or(ReadError::Malformed)?;
    let mut tracks = Vec::new();
    for (kind, body) in &boxes {
        if kind == b"trak" {
            if let Some(track) = trak(body)? {
                tracks.push(track);
            }
        }
    }
    Ok(Movie {
        timescale,
        duration,
        tracks,
    })
}

fn mvhd_fields(body: &[u8]) -> Option<(u32, u64)> {
    let mut fields = Fields::new(body);
    let version = fields.u8()?;
    fields.skip(3)?;
    if version == 1 {
        fields.skip(16)?;
        let timescale = fields.u32()?;
        Some((timescale, fields.u64()?))
    } else {
        fields.skip(8)?;
        let timescale = fields.u32()?;
        Some((timescale, u64::from(fields.u32()?)))
    }
}

/// Clockwise quarter-turns from a `tkhd` matrix's a, b, c, d (16.16). The
/// matrix maps (x, y) to (a·x + c·y, b·x + d·y) in a y-down picture, so
/// (0, 1, −1, 0) turns the x axis onto the y axis: 90° clockwise — a phone
/// held upright.
fn rotation(a: i32, b: i32, c: i32, d: i32) -> Option<u16> {
    const ONE: i32 = 0x0001_0000;
    const MINUS_ONE: i32 = -0x0001_0000;
    match (a, b, c, d) {
        (ONE, 0, 0, ONE) => Some(0),
        (0, ONE, MINUS_ONE, 0) => Some(90),
        (MINUS_ONE, 0, 0, MINUS_ONE) => Some(180),
        (0, MINUS_ONE, ONE, 0) => Some(270),
        _ => None,
    }
}

fn trak(body: &[u8]) -> Result<Option<Track>, ReadError> {
    let boxes = children(body);
    let find = |kind: &[u8; 4]| {
        boxes
            .iter()
            .find(|(found, _)| found == kind)
            .map(|(_, body)| *body)
    };
    let Some(tkhd) = find(b"tkhd") else {
        return Ok(None);
    };
    let Some(mdia) = find(b"mdia") else {
        return Ok(None);
    };
    let mut fields = Fields::new(tkhd);
    let version = fields.u8().ok_or(ReadError::Malformed)?;
    let flags = fields.take(3).ok_or(ReadError::Malformed)?;
    let enabled = flags[2] & 1 == 1;
    let skip = if version == 1 { 16 } else { 8 };
    fields.skip(skip).ok_or(ReadError::Malformed)?;
    let id = fields.u32().ok_or(ReadError::Malformed)?;
    fields
        .skip(4 + if version == 1 { 8 } else { 4 } + 8 + 8)
        .ok_or(ReadError::Malformed)?;
    let mut matrix = [0i32; 9];
    for value in matrix.iter_mut() {
        *value = fields.i32().ok_or(ReadError::Malformed)?;
    }
    let width = fields.u32().ok_or(ReadError::Malformed)? >> 16;
    let height = fields.u32().ok_or(ReadError::Malformed)? >> 16;

    let mdia = children(mdia);
    let in_mdia = |kind: &[u8; 4]| {
        mdia.iter()
            .find(|(found, _)| found == kind)
            .map(|(_, body)| *body)
    };
    let mdhd = in_mdia(b"mdhd").ok_or(ReadError::Malformed)?;
    let (timescale, duration) = mvhd_fields(mdhd).ok_or(ReadError::Malformed)?;
    let handler: [u8; 4] = in_mdia(b"hdlr")
        .and_then(|hdlr| hdlr.get(8..12))
        .and_then(|kind| kind.try_into().ok())
        .unwrap_or(*b"    ");
    let stbl = in_mdia(b"minf")
        .and_then(|minf| child(minf, b"stbl"))
        .ok_or(ReadError::Malformed)?;
    let (entry, samples) = sample_table(stbl, &handler)?;
    let (edit, complex_edit) = match find(b"edts").and_then(|edts| child(edts, b"elst")) {
        Some(elst) => edit(elst).ok_or(ReadError::Malformed)?,
        None => (None, false),
    };
    Ok(Some(Track {
        id,
        handler,
        enabled,
        timescale,
        duration,
        width,
        height,
        rotation: rotation(matrix[0], matrix[1], matrix[3], matrix[4]),
        entry,
        samples,
        edit,
        complex_edit,
    }))
}

/// An edit list reduced to one [`Edit`] — an optional empty lead, then one
/// segment at normal rate — and whether it had anything more than that.
fn edit(elst: &[u8]) -> Option<(Option<Edit>, bool)> {
    let mut fields = Fields::new(elst);
    let version = fields.u8()?;
    fields.skip(3)?;
    let count = fields.u32()?;
    let mut lead = 0u64;
    let mut found: Option<Edit> = None;
    let mut complex = false;
    for _ in 0..count.min(1024) {
        let (length, media_time) = if version == 1 {
            (fields.u64()?, fields.u64()? as i64)
        } else {
            (u64::from(fields.u32()?), i64::from(fields.i32()?))
        };
        let rate = fields.u32()?;
        if media_time == -1 {
            if found.is_some() {
                complex = true;
            } else {
                lead += length;
            }
            continue;
        }
        if found.is_some() || rate != 0x0001_0000 {
            complex = true;
            continue;
        }
        found = Some(Edit {
            media_start: media_time,
            lead,
            length: (length > 0).then_some(length),
        });
    }
    Some((found, complex))
}

fn full_box_body(body: &[u8]) -> Option<(u8, &[u8])> {
    let version = *body.first()?;
    Some((version, body.get(4..)?))
}

fn sample_table(
    stbl: &[u8],
    handler: &[u8; 4],
) -> Result<(Option<SampleEntry>, Vec<Sample>), ReadError> {
    let boxes = children(stbl);
    let find = |kind: &[u8; 4]| {
        boxes
            .iter()
            .find(|(found, _)| found == kind)
            .map(|(_, body)| *body)
    };
    let entry = find(b"stsd").and_then(|stsd| sample_entry(stsd, handler));

    // Sizes.
    let sizes: Vec<u32> = if let Some(stsz) = find(b"stsz") {
        let (_, body) = full_box_body(stsz).ok_or(ReadError::Malformed)?;
        let mut fields = Fields::new(body);
        let constant = fields.u32().ok_or(ReadError::Malformed)?;
        let count = u64::from(fields.u32().ok_or(ReadError::Malformed)?);
        if count > MAX_SAMPLES {
            return Err(ReadError::TooLarge);
        }
        if constant != 0 {
            vec![constant; count as usize]
        } else {
            (0..count)
                .map(|_| fields.u32().ok_or(ReadError::Malformed))
                .collect::<Result<_, _>>()?
        }
    } else if let Some(stz2) = find(b"stz2") {
        let (_, body) = full_box_body(stz2).ok_or(ReadError::Malformed)?;
        let mut fields = Fields::new(body);
        fields.skip(3).ok_or(ReadError::Malformed)?;
        let field_size = fields.u8().ok_or(ReadError::Malformed)?;
        let count = u64::from(fields.u32().ok_or(ReadError::Malformed)?);
        if count > MAX_SAMPLES {
            return Err(ReadError::TooLarge);
        }
        let mut sizes = Vec::with_capacity(count as usize);
        match field_size {
            16 => {
                for _ in 0..count {
                    sizes.push(u32::from(fields.u16().ok_or(ReadError::Malformed)?));
                }
            }
            8 => {
                for _ in 0..count {
                    sizes.push(u32::from(fields.u8().ok_or(ReadError::Malformed)?));
                }
            }
            4 => {
                let packed = fields
                    .take(count.div_ceil(2) as usize)
                    .ok_or(ReadError::Malformed)?;
                for index in 0..count as usize {
                    let byte = packed[index / 2];
                    sizes.push(u32::from(if index % 2 == 0 {
                        byte >> 4
                    } else {
                        byte & 0x0F
                    }));
                }
            }
            _ => return Err(ReadError::Malformed),
        }
        sizes
    } else {
        Vec::new()
    };
    let count = sizes.len();
    if count == 0 {
        return Ok((entry, Vec::new()));
    }

    // Decode times.
    let stts = find(b"stts").ok_or(ReadError::Malformed)?;
    let (_, body) = full_box_body(stts).ok_or(ReadError::Malformed)?;
    let mut fields = Fields::new(body);
    let runs = fields.u32().ok_or(ReadError::Malformed)?;
    let mut durations = Vec::with_capacity(count);
    for _ in 0..runs {
        let run = fields.u32().ok_or(ReadError::Malformed)? as usize;
        let delta = fields.u32().ok_or(ReadError::Malformed)?;
        let room = count - durations.len();
        durations.extend(std::iter::repeat_n(delta, run.min(room)));
        if durations.len() == count {
            break;
        }
    }
    // A table short of the sample count repeats its last delta, as players
    // do; one with none at all is not a table.
    let last = *durations.last().ok_or(ReadError::Malformed)?;
    durations.resize(count, last);

    // Composition offsets. Version 0 is unsigned by the letter of the
    // standard and signed in QuickTime's; read as signed, both come out
    // right for every offset a real file has.
    let mut offsets = vec![0i32; count];
    if let Some(ctts) = find(b"ctts") {
        let (_, body) = full_box_body(ctts).ok_or(ReadError::Malformed)?;
        let mut fields = Fields::new(body);
        let runs = fields.u32().ok_or(ReadError::Malformed)?;
        let mut at = 0usize;
        for _ in 0..runs {
            let run = fields.u32().ok_or(ReadError::Malformed)? as usize;
            let offset = fields.i32().ok_or(ReadError::Malformed)?;
            let end = at.saturating_add(run).min(count);
            offsets[at..end].fill(offset);
            at = end;
            if at == count {
                break;
            }
        }
    }

    // Key frames: every sample, when there is no table.
    let mut sync = vec![true; count];
    if let Some(stss) = find(b"stss") {
        let (_, body) = full_box_body(stss).ok_or(ReadError::Malformed)?;
        let mut fields = Fields::new(body);
        let entries = fields.u32().ok_or(ReadError::Malformed)?;
        sync.fill(false);
        for _ in 0..entries {
            let number = fields.u32().ok_or(ReadError::Malformed)? as usize;
            if number >= 1 && number <= count {
                sync[number - 1] = true;
            }
        }
    }

    // Chunks, and where each sample is.
    let chunk_offsets: Vec<u64> = if let Some(stco) = find(b"stco") {
        let (_, body) = full_box_body(stco).ok_or(ReadError::Malformed)?;
        let mut fields = Fields::new(body);
        let entries = fields.u32().ok_or(ReadError::Malformed)?;
        (0..entries)
            .map(|_| fields.u32().map(u64::from).ok_or(ReadError::Malformed))
            .collect::<Result<_, _>>()?
    } else if let Some(co64) = find(b"co64") {
        let (_, body) = full_box_body(co64).ok_or(ReadError::Malformed)?;
        let mut fields = Fields::new(body);
        let entries = fields.u32().ok_or(ReadError::Malformed)?;
        (0..entries)
            .map(|_| fields.u64().ok_or(ReadError::Malformed))
            .collect::<Result<_, _>>()?
    } else {
        return Err(ReadError::Malformed);
    };
    let stsc = find(b"stsc").ok_or(ReadError::Malformed)?;
    let (_, body) = full_box_body(stsc).ok_or(ReadError::Malformed)?;
    let mut fields = Fields::new(body);
    let entries = fields.u32().ok_or(ReadError::Malformed)?;
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for _ in 0..entries {
        let first = fields.u32().ok_or(ReadError::Malformed)?;
        let per_chunk = fields.u32().ok_or(ReadError::Malformed)?;
        fields.u32().ok_or(ReadError::Malformed)?;
        if first == 0 || runs.last().is_some_and(|(last, _)| first <= *last) {
            return Err(ReadError::Malformed);
        }
        runs.push((first, per_chunk));
    }
    let mut samples = Vec::with_capacity(count);
    let mut index = 0usize;
    let mut dts: i64 = 0;
    'chunks: for (chunk_index, &chunk_offset) in chunk_offsets.iter().enumerate() {
        let number = chunk_index as u32 + 1;
        let per_chunk = runs
            .iter()
            .rev()
            .find(|(first, _)| *first <= number)
            .map(|(_, per_chunk)| *per_chunk)
            .ok_or(ReadError::Malformed)?;
        let mut offset = chunk_offset;
        for _ in 0..per_chunk {
            if index == count {
                break 'chunks;
            }
            samples.push(Sample {
                offset,
                size: sizes[index],
                dts,
                pts: dts + i64::from(offsets[index]),
                duration: durations[index],
                sync: sync[index],
            });
            offset += u64::from(sizes[index]);
            dts += i64::from(durations[index]);
            index += 1;
        }
    }
    if samples.len() != count {
        // Fewer chunks than the sizes need: the index is not self-consistent.
        return Err(ReadError::Malformed);
    }
    Ok((entry, samples))
}

fn sample_entry(stsd: &[u8], handler: &[u8; 4]) -> Option<SampleEntry> {
    let (_, body) = full_box_body(stsd)?;
    let mut fields = Fields::new(body);
    let count = fields.u32()?;
    if count == 0 {
        return None;
    }
    let (format, entry) = children(fields.rest()).into_iter().next()?;
    let mut found = SampleEntry {
        format,
        ..SampleEntry::default()
    };
    let mut fields = Fields::new(entry);
    fields.skip(8)?; // reserved, data reference index
    let extensions = if handler == b"vide" {
        fields.skip(16)?;
        found.width = fields.u16()?;
        found.height = fields.u16()?;
        fields.skip(4 + 4 + 4 + 2 + 32 + 2 + 2)?;
        fields.rest()
    } else if handler == b"soun" {
        let version = fields.u16()?;
        fields.skip(6)?; // revision, vendor
        let channels = fields.u16()?;
        fields.skip(6)?; // sample size, compression id, packet size
        let rate = fields.u32()? >> 16;
        found.channels = u32::from(channels);
        found.sample_rate = rate;
        match version {
            1 => {
                fields.skip(16)?;
            }
            2 => {
                // The version-2 layout replaces both with wider fields.
                fields.skip(4)?; // size of the struct
                let rate = f64::from_bits(fields.u64()?);
                let channels = fields.u32()?;
                fields.skip(20)?;
                if rate.is_finite() && rate > 0.0 && rate < 1e7 {
                    found.sample_rate = rate.round() as u32;
                }
                found.channels = channels;
            }
            _ => {}
        }
        fields.rest()
    } else {
        return Some(found);
    };
    for (kind, body) in children(extensions) {
        match &kind {
            b"avcC" | b"hvcC" | b"av1C" => found.config = body.to_vec(),
            // vpcC is a full box: the record follows its version and flags.
            b"vpcC" => found.config = body.to_vec(),
            b"esds" => esds(body, &mut found),
            // QuickTime nests the esds in a `wave` box.
            b"wave" => {
                if let Some(esds_body) = child(body, b"esds") {
                    esds(esds_body, &mut found);
                }
            }
            // ALAC's own configuration (the entry's channel field says 2
            // whatever the stream holds): numChannels, and the rate.
            b"alac" if handler == b"soun" => {
                if let (Some(&channels), Some(rate)) = (body.get(13), body.get(24..28)) {
                    if channels > 0 {
                        found.channels = u32::from(channels);
                    }
                    let rate = u32::from_be_bytes(rate.try_into().unwrap_or([0; 4]));
                    if rate > 0 {
                        found.sample_rate = rate;
                    }
                }
            }
            // Opus's: its output channel count.
            b"dOps" => {
                if let Some(&channels) = body.get(1) {
                    found.channels = u32::from(channels);
                }
            }
            b"btrt" => {
                let mut fields = Fields::new(body);
                let _ = fields.skip(8); // buffer size, maximum rate
                found.stated_bitrate = fields.u32().map(u64::from).filter(|&rate| rate > 0);
            }
            b"colr" if body.get(0..4) == Some(b"nclx") || body.get(0..4) == Some(b"nclc") => {
                let mut fields = Fields::new(&body[4..]);
                if let (Some(primaries), Some(transfer), Some(matrix)) =
                    (fields.u16(), fields.u16(), fields.u16())
                {
                    found.colour = Some(Colour {
                        primaries,
                        transfer,
                        matrix,
                        full_range: fields.u8().is_some_and(|byte| byte & 0x80 != 0),
                    });
                }
            }
            _ => {}
        }
    }
    Some(found)
}

/// An MPEG-4 descriptor's tag and body, from the start of `bytes`, and the
/// bytes after it.
fn descriptor(bytes: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let tag = *bytes.first()?;
    let mut length = 0usize;
    let mut at = 1;
    for _ in 0..4 {
        let byte = *bytes.get(at)?;
        at += 1;
        length = (length << 7) | usize::from(byte & 0x7F);
        if byte & 0x80 == 0 {
            break;
        }
    }
    let end = at.checked_add(length)?;
    Some((tag, bytes.get(at..end)?, bytes.get(end..)?))
}

fn esds(body: &[u8], entry: &mut SampleEntry) {
    let Some((_, body)) = full_box_body(body) else {
        return;
    };
    let Some((0x03, es, _)) = descriptor(body) else {
        return;
    };
    let mut fields = Fields::new(es);
    let (Some(_), Some(flags)) = (fields.u16(), fields.u8()) else {
        return;
    };
    if flags & 0x80 != 0 && fields.skip(2).is_none() {
        return;
    }
    if flags & 0x40 != 0 {
        let Some(length) = fields.u8() else {
            return;
        };
        if fields.skip(usize::from(length)).is_none() {
            return;
        }
    }
    if flags & 0x20 != 0 && fields.skip(2).is_none() {
        return;
    }
    let mut rest = fields.rest();
    while let Some((tag, inner, after)) = descriptor(rest) {
        if tag == 0x04 {
            let mut config = Fields::new(inner);
            let (Some(object_type), Some(_), Some(_), Some(_), Some(average)) = (
                config.u8(),
                config.u8(),
                config.take(3),
                config.u32(),
                config.u32(),
            ) else {
                return;
            };
            entry.object_type = object_type;
            if average > 0 {
                entry.stated_bitrate = Some(u64::from(average));
            }
            if let Some((0x05, specific, _)) = descriptor(config.rest()) {
                entry.config = specific.to_vec();
            }
            return;
        }
        rest = after;
    }
}

// --- what a decoder is told ---------------------------------------------------------------------

/// What an AudioSpecificConfig says: the Audio Object Type (2 is AAC-LC, 5
/// HE-AAC, 29 HE-AAC v2), the sampling rate and the channel configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioConfig {
    pub object_type: u8,
    pub sample_rate: u32,
    /// The channel CONFIGURATION — an index, not a count (see
    /// [`AudioConfig::channel_count`]).
    pub channels: u8,
}

impl AudioConfig {
    /// How many channels the stream decodes to (ISO/IEC 14496-3, Table
    /// 1.19): the configuration itself up to 6, and 8 for 7 (7.1). HE-AAC
    /// v2 codes ONE channel and its decoder hands out two — that is what
    /// parametric stereo is. None for configuration 0, which leaves the
    /// layout to the stream itself, and for the reserved ones.
    pub fn channel_count(&self) -> Option<u32> {
        match (self.object_type, self.channels) {
            (29, 1) => Some(2),
            (_, count @ 1..=6) => Some(u32::from(count)),
            (_, 7) => Some(8),
            _ => None,
        }
    }
}

const AAC_RATES: [u32; 13] = [
    96_000, 88_200, 64_000, 48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000, 11_025, 8_000,
    7_350,
];

struct BitReader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl BitReader<'_> {
    fn bits(&mut self, count: usize) -> Option<u32> {
        let mut value = 0u32;
        for _ in 0..count {
            let byte = *self.bytes.get(self.at / 8)?;
            value = (value << 1) | u32::from((byte >> (7 - self.at % 8)) & 1);
            self.at += 1;
        }
        Some(value)
    }
}

/// Read the head of an AudioSpecificConfig (ISO/IEC 14496-3, 1.6.2.1).
pub fn audio_config(asc: &[u8]) -> Option<AudioConfig> {
    let mut bits = BitReader { bytes: asc, at: 0 };
    let mut object_type = bits.bits(5)?;
    if object_type == 31 {
        object_type = 32 + bits.bits(6)?;
    }
    let index = bits.bits(4)? as usize;
    let sample_rate = if index == 15 {
        bits.bits(24)?
    } else {
        *AAC_RATES.get(index)?
    };
    let channels = bits.bits(4)? as u8;
    Some(AudioConfig {
        object_type: object_type as u8,
        sample_rate,
        channels,
    })
}

/// The WebCodecs codec string for a video sample entry (RFC 6381, and ISO/IEC
/// 14496-15 Annex E for HEVC), or None for a format this reader does not
/// know how to name.
pub fn video_codec_string(entry: &SampleEntry) -> Option<String> {
    let config = &entry.config;
    match &entry.format {
        b"avc1" | b"avc3" => {
            // configurationVersion, then profile, constraints, level.
            let (profile, constraints, level) = (config.get(1)?, config.get(2)?, config.get(3)?);
            let prefix = std::str::from_utf8(&entry.format).ok()?;
            Some(format!(
                "{prefix}.{profile:02x}{constraints:02x}{level:02x}"
            ))
        }
        b"hvc1" | b"hev1" => {
            let first = *config.get(1)?;
            let space = ["", "A", "B", "C"][usize::from(first >> 6)];
            let tier = if first & 0x20 != 0 { 'H' } else { 'L' };
            let profile = first & 0x1F;
            let compatibility = u32::from_be_bytes(config.get(2..6)?.try_into().ok()?);
            let constraints = config.get(6..12)?;
            let level = *config.get(12)?;
            let prefix = std::str::from_utf8(&entry.format).ok()?;
            let mut text = format!(
                "{prefix}.{space}{profile}.{:X}.{tier}{level}",
                compatibility.reverse_bits()
            );
            let kept = constraints
                .iter()
                .rposition(|&byte| byte != 0)
                .map_or(0, |last| last + 1);
            for byte in &constraints[..kept] {
                text.push_str(&format!(".{byte:02X}"));
            }
            Some(text)
        }
        b"av01" => {
            let (second, third) = (*config.get(1)?, *config.get(2)?);
            let profile = second >> 5;
            let level = second & 0x1F;
            let tier = if third & 0x80 != 0 { 'H' } else { 'M' };
            let depth = match (third & 0x40 != 0, third & 0x20 != 0) {
                (true, true) => 12,
                (true, false) => 10,
                _ => 8,
            };
            Some(format!("av01.{profile}.{level:02}{tier}.{depth:02}"))
        }
        b"vp09" => {
            // A full box: version and flags, then profile, level, and the
            // bit depth in the high four bits of the next byte.
            let (profile, level, depth) = (*config.get(4)?, *config.get(5)?, *config.get(6)? >> 4);
            Some(format!("vp09.{profile:02}.{level:02}.{depth:02}"))
        }
        _ => None,
    }
}

/// The configuration record WebCodecs' `VideoDecoder` takes as its
/// `description`: the record itself for H.264 and HEVC (AV1's is optional
/// and VP9 has none).
pub fn video_description(entry: &SampleEntry) -> Option<&[u8]> {
    match &entry.format {
        b"avc1" | b"avc3" | b"hvc1" | b"hev1" | b"av01" if !entry.config.is_empty() => {
            Some(&entry.config)
        }
        _ => None,
    }
}

/// The name [`crate::media_plan`] knows a video format by: `"h264"`, `"hevc"`,
/// `"av1"`, `"vp9"` or `"unknown"`.
pub fn video_codec_name(entry: &SampleEntry) -> &'static str {
    match &entry.format {
        b"avc1" | b"avc3" => "h264",
        b"hvc1" | b"hev1" | b"dvh1" | b"dvhe" => "hevc",
        b"av01" => "av1",
        b"vp09" => "vp9",
        _ => "unknown",
    }
}

/// The name [`crate::media_plan`] knows a sound format by: `"aac"` for any
/// MPEG-4 audio in an `mp4a` (HE-AAC included), `"mp3"`, `"alac"`, `"opus"`,
/// `"flac"`, `"pcm"` for QuickTime's uncompressed formats, or `"unknown"`.
pub fn audio_codec_name(entry: &SampleEntry) -> &'static str {
    match &entry.format {
        b"mp4a" => match entry.object_type {
            // MPEG-4 audio, and MPEG-2 AAC's three profiles.
            0x40 | 0x66..=0x68 => "aac",
            0x69 | 0x6B => "mp3",
            // QuickTime writes no object type for AAC in some files; the
            // AudioSpecificConfig then says it.
            0 if !entry.config.is_empty() => "aac",
            _ => "unknown",
        },
        b".mp3" => "mp3",
        b"alac" => "alac",
        b"Opus" => "opus",
        b"fLaC" => "flac",
        b"lpcm" | b"sowt" | b"twos" | b"in24" | b"in32" | b"fl32" | b"fl64" | b"raw " => "pcm",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mp4::{self, Brand, Media, Track as Out};

    fn top_level(file: &[u8]) -> Vec<(BoxHeader, usize)> {
        let mut found = Vec::new();
        let mut at = 0usize;
        while let Some(header) = box_header(&file[at..]) {
            let size = header.size.unwrap_or((file.len() - at) as u64) as usize;
            found.push((header, at));
            at += size;
            if at >= file.len() {
                break;
            }
        }
        found
    }

    fn moov_of(file: &[u8]) -> &[u8] {
        let (header, at) = top_level(file)
            .into_iter()
            .find(|(header, _)| &header.kind == b"moov")
            .unwrap();
        &file[at..at + header.size.unwrap() as usize]
    }

    fn written() -> (Vec<u8>, Vec<Vec<Vec<u8>>>) {
        let video: Vec<Vec<u8>> = (0..60).map(|i| vec![i as u8; 500 + i]).collect();
        let audio: Vec<Vec<u8>> = (0..94).map(|i| vec![0x80 | (i % 64) as u8; 200]).collect();
        let tracks = [
            Out {
                timescale: 600,
                media: Media::Video {
                    width: 640,
                    height: 360,
                    avcc: vec![1, 0x64, 0x00, 0x1E, 0xFF, 0xE1, 0, 0, 1, 0],
                    colour: None,
                },
                samples: video
                    .iter()
                    .enumerate()
                    .map(|(i, payload)| mp4::Sample {
                        size: payload.len() as u32,
                        duration: 20,
                        composition_offset: 0,
                        sync: i % 30 == 0,
                    })
                    .collect(),
                skip: 0,
                lead: 0,
            },
            Out {
                timescale: 48_000,
                media: Media::Audio {
                    sample_rate: 48_000,
                    channels: 2,
                    asc: mp4::audio_specific_config(mp4::AAC_LC, 48_000, 2),
                    avg_bitrate: 128_000,
                    max_bitrate: 128_000,
                },
                samples: audio
                    .iter()
                    .map(|payload| mp4::Sample {
                        size: payload.len() as u32,
                        duration: 1024,
                        composition_offset: 0,
                        sync: true,
                    })
                    .collect(),
                skip: 1024,
                lead: 0,
            },
        ];
        let payloads = vec![video, audio];
        (mp4::write(&tracks, &payloads, Brand::Mp4), payloads)
    }

    /// What the writer writes, the reader reads back, sample for sample —
    /// each at the offset where its own bytes actually are.
    #[test]
    fn the_writers_file_reads_back_sample_for_sample() {
        let (file, payloads) = written();
        let movie = parse_moov(moov_of(&file)).unwrap();
        assert_eq!(movie.tracks.len(), 2);
        let video = movie.video().unwrap();
        let entry = video.entry.as_ref().unwrap();
        assert_eq!(&entry.format, b"avc1");
        assert_eq!((entry.width, entry.height), (640, 360));
        assert_eq!((video.width, video.height), (640, 360));
        assert_eq!(video.rotation, Some(0));
        assert_eq!(video.timescale, 600);
        assert_eq!(video.samples.len(), 60);
        assert_eq!(video.frame_rate(), Some(30.0));
        assert_eq!(video_codec_string(entry).as_deref(), Some("avc1.64001e"));
        assert_eq!(video_codec_name(entry), "h264");
        for (sample, payload) in video.samples.iter().zip(&payloads[0]) {
            let at = sample.offset as usize;
            assert_eq!(&file[at..at + sample.size as usize], &payload[..]);
        }
        let keys: Vec<usize> = video
            .samples
            .iter()
            .enumerate()
            .filter(|(_, s)| s.sync)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(keys, vec![0, 30]);
        assert_eq!(video.samples[59].dts, 59 * 20);
        let audio = movie.audio().unwrap();
        let entry = audio.entry.as_ref().unwrap();
        assert_eq!(audio_codec_name(entry), "aac");
        assert_eq!((entry.channels, entry.sample_rate), (2, 48_000));
        assert_eq!(entry.stated_bitrate, Some(128_000));
        assert_eq!(
            audio_config(&entry.config),
            Some(AudioConfig {
                object_type: 2,
                sample_rate: 48_000,
                channels: 2
            })
        );
        assert_eq!(
            audio.edit,
            Some(Edit {
                media_start: 1024,
                lead: 0,
                length: Some(1984)
            })
        );
        for (sample, payload) in audio.samples.iter().zip(&payloads[1]) {
            let at = sample.offset as usize;
            assert_eq!(&file[at..at + sample.size as usize], &payload[..]);
        }
        // 94 × 200 bytes over 94 × 1024 samples at 48 kHz.
        assert_eq!(audio.data_rate(), Some(75_000));
        // The longer track: 60 frames of 1/30 s.
        assert_eq!(movie.duration_ms(), Some(2000));
    }

    #[test]
    fn a_box_header_reads_32_and_64_bit_sizes_and_to_the_end() {
        assert_eq!(
            box_header(&[0, 0, 0, 16, b'f', b't', b'y', b'p']),
            Some(BoxHeader {
                kind: *b"ftyp",
                header_len: 8,
                size: Some(16)
            })
        );
        let mut large = vec![0, 0, 0, 1, b'm', b'd', b'a', b't'];
        large.extend_from_slice(&(5_000_000_000u64).to_be_bytes());
        assert_eq!(box_header(&large).unwrap().size, Some(5_000_000_000));
        assert_eq!(box_header(&large).unwrap().header_len, 16);
        assert_eq!(
            box_header(&[0, 0, 0, 0, b'm', b'd', b'a', b't'])
                .unwrap()
                .size,
            None
        );
        assert_eq!(box_header(&[0, 0, 0, 5, b'b', b'a', b'd', b'!']), None);
        assert_eq!(box_header(&[0, 0, 0]), None);
    }

    #[test]
    fn rotations_are_read_from_the_matrix() {
        const ONE: i32 = 0x10000;
        assert_eq!(rotation(ONE, 0, 0, ONE), Some(0));
        assert_eq!(rotation(0, ONE, -ONE, 0), Some(90));
        assert_eq!(rotation(-ONE, 0, 0, -ONE), Some(180));
        assert_eq!(rotation(0, -ONE, ONE, 0), Some(270));
        assert_eq!(rotation(-ONE, 0, 0, ONE), None, "a mirror is not a turn");
    }

    #[test]
    fn hevc_av1_and_vp9_are_named_the_way_webcodecs_asks() {
        // An iPhone's HEVC Main: profile space 0, tier Main, profile 1,
        // compatibility 0x60000000 (read reversed: 6), level 93 (3.1),
        // constraint byte 0xB0.
        let mut hvcc = vec![1, 0x01, 0x60, 0, 0, 0, 0xB0, 0, 0, 0, 0, 0, 93];
        hvcc.extend_from_slice(&[0xF0, 0, 0xFC, 0xFD, 0xF8, 0xF8, 0, 0, 0x0F]);
        let entry = SampleEntry {
            format: *b"hvc1",
            config: hvcc,
            ..SampleEntry::default()
        };
        assert_eq!(
            video_codec_string(&entry).as_deref(),
            Some("hvc1.1.6.L93.B0")
        );
        // Main 10, high tier, level 150.
        let main10 = SampleEntry {
            format: *b"hev1",
            config: vec![1, 0x22, 0x20, 0, 0, 0, 0x90, 0, 0, 0, 0, 0, 150],
            ..SampleEntry::default()
        };
        assert_eq!(
            video_codec_string(&main10).as_deref(),
            Some("hev1.2.4.H150.90")
        );
        // AV1 main profile, level 8 (4.0), main tier, 10-bit.
        let av1 = SampleEntry {
            format: *b"av01",
            config: vec![0x81, 0x08, 0x4C, 0],
            ..SampleEntry::default()
        };
        assert_eq!(video_codec_string(&av1).as_deref(), Some("av01.0.08M.10"));
        let vp9 = SampleEntry {
            format: *b"vp09",
            config: vec![1, 0, 0, 0, 0, 31, 0x82, 0, 0, 0, 0, 0],
            ..SampleEntry::default()
        };
        assert_eq!(video_codec_string(&vp9).as_deref(), Some("vp09.00.31.08"));
        assert_eq!(
            video_codec_string(&SampleEntry {
                format: *b"mp4v",
                ..SampleEntry::default()
            }),
            None
        );
    }

    #[test]
    fn an_audio_specific_config_reads_its_type_rate_and_channels() {
        assert_eq!(
            audio_config(&[0x12, 0x10]),
            Some(AudioConfig {
                object_type: 2,
                sample_rate: 44_100,
                channels: 2
            })
        );
        // HE-AAC v1 (5) at 24 kHz core, mono.
        assert_eq!(audio_config(&[0x2B, 0x08]).unwrap().object_type, 5);
        assert_eq!(audio_config(&[]), None);
        // The configuration is an index: 7 is eight channels, 0 is "ask the
        // stream", and HE-AAC v2's one coded channel decodes to two.
        let config = |object_type, channels| AudioConfig {
            object_type,
            sample_rate: 48_000,
            channels,
        };
        assert_eq!(config(2, 1).channel_count(), Some(1));
        assert_eq!(config(2, 2).channel_count(), Some(2));
        assert_eq!(config(2, 6).channel_count(), Some(6));
        assert_eq!(config(2, 7).channel_count(), Some(8));
        assert_eq!(config(2, 0).channel_count(), None);
        assert_eq!(config(2, 9).channel_count(), None);
        assert_eq!(config(29, 1).channel_count(), Some(2));
        assert_eq!(config(5, 1).channel_count(), Some(1));
    }

    /// An empty edit in front of a track's media is its lead — read, kept
    /// apart from where the media starts, and not an edit list this reader
    /// gives up on. What comes after the media, or a second piece of it, is.
    #[test]
    fn an_empty_edit_is_a_lead_and_is_read_back_from_the_writers_file() {
        let (file, _) = written();
        let plain = parse_moov(moov_of(&file)).unwrap();
        assert_eq!(plain.lead_ms(plain.video().unwrap()), Some(0));
        assert_eq!(plain.lead_ms(plain.audio().unwrap()), Some(0));

        let payloads: Vec<Vec<u8>> = (0..30).map(|i| vec![i as u8; 300]).collect();
        let late = Out {
            timescale: 600,
            media: Media::Video {
                width: 640,
                height: 360,
                avcc: vec![1, 0x64, 0x00, 0x1E, 0xFF, 0xE1, 0, 0, 1, 0],
                colour: None,
            },
            samples: payloads
                .iter()
                .map(|payload| mp4::Sample {
                    size: payload.len() as u32,
                    duration: 20,
                    composition_offset: 0,
                    sync: true,
                })
                .collect(),
            skip: 0,
            lead: 300,
        };
        let file = mp4::write(&[late], &[payloads], Brand::Mp4);
        let movie = parse_moov(moov_of(&file)).unwrap();
        let video = movie.video().unwrap();
        assert_eq!(
            video.edit,
            Some(Edit {
                media_start: 0,
                lead: 300,
                length: Some(1000)
            })
        );
        assert!(video.edit_supported());
        assert_eq!(movie.lead_ms(video), Some(300));
        assert_eq!(movie.presented_ms(video), Some(1000));
        assert_eq!(movie.duration_ms(), Some(1300));

        // In another movie timescale — a phone's 90 000 — and as version 1.
        let entry = |length: u64, media_time: i64| {
            let mut bytes = length.to_be_bytes().to_vec();
            bytes.extend_from_slice(&media_time.to_be_bytes());
            bytes.extend_from_slice(&0x0001_0000u32.to_be_bytes());
            bytes
        };
        let list = |entries: &[Vec<u8>]| {
            let mut elst = vec![1, 0, 0, 0];
            elst.extend_from_slice(&(entries.len() as u32).to_be_bytes());
            for entry in entries {
                elst.extend_from_slice(entry);
            }
            elst
        };
        let (found, complex) = edit(&list(&[entry(27_045, -1), entry(90_000, 3_003)])).unwrap();
        assert!(!complex);
        let mut track = video.clone();
        track.edit = found;
        let mut phone = movie.clone();
        phone.timescale = 90_000;
        assert_eq!(phone.lead_ms(&track), Some(301), "300.5 ms, to the nearest");
        assert_eq!(found.unwrap().media_start, 3_003);
        // Nothing AFTER the media, or media in two pieces: not one edit.
        let (_, complex) = edit(&list(&[entry(90_000, 0), entry(9_000, -1)])).unwrap();
        assert!(complex);
        let (_, complex) = edit(&list(&[entry(90_000, 0), entry(90_000, 180_000)])).unwrap();
        assert!(complex);
    }

    #[test]
    fn a_fragmented_file_is_named_as_one() {
        let mut moov = Vec::new();
        let mvex = [0u8, 0, 0, 8, b'm', b'v', b'e', b'x'];
        let mut mvhd = vec![0u8, 0, 0, 108, b'm', b'v', b'h', b'd'];
        mvhd.resize(108, 0);
        let size = (8 + mvhd.len() + mvex.len()) as u32;
        moov.extend_from_slice(&size.to_be_bytes());
        moov.extend_from_slice(b"moov");
        moov.extend_from_slice(&mvhd);
        moov.extend_from_slice(&mvex);
        assert_eq!(parse_moov(&moov), Err(ReadError::Fragmented));
    }

    /// Garbage is an error, never a panic: every truncation of a real index
    /// either reads or is refused.
    #[test]
    fn every_truncation_of_an_index_is_refused_or_read_never_a_panic() {
        let (file, _) = written();
        let moov = moov_of(&file).to_vec();
        for cut in 0..moov.len() {
            let mut short = moov[..cut].to_vec();
            if short.len() >= 4 {
                // Keep the header honest about the new length, so the cut
                // lands inside the tables rather than at the outer box.
                let len = short.len() as u32;
                short[0..4].copy_from_slice(&len.to_be_bytes());
            }
            let _ = parse_moov(&short);
        }
        // And flipped bytes, one at a time.
        for at in 0..moov.len() {
            let mut bent = moov.clone();
            bent[at] ^= 0xFF;
            let _ = parse_moov(&bent);
        }
    }
}
