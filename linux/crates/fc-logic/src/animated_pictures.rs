//! The three animated pictures the protocol turns into a video, taken apart by this client
//! (`AnimatedPictures.cs`; issue #96; docs/protocol.md, "Animations"): which picked files are
//! animations ([`animation_prep`]), a GIF decoded here ([`GifPicture`]), an APNG and an animated
//! WebP cut into STANDALONE stills that the platform's still-picture decoder reads
//! ([`ApngPicture`], [`WebPAnimation`]), and the one description of an animation every one of them
//! becomes ([`AnimationLayout`]) — which `animation_frames` composites.
//!
//! On Windows the reasons were Windows Imaging's: its GIF decoder hands out raw sub-rectangles, its
//! PNG decoder has never read APNG, and its WebP extension may or may not hand out an animation's
//! frames. On Linux gdk-pixbuf (glycin) decodes animated GIF and WebP whole, but not APNG — and
//! doing the cutting here keeps the picture a function of its bytes on every client, tested
//! wherever `cargo test` runs. The platform is asked only to decode ONE standalone still at a time
//! (`animation_preparing::StillDecoder`).
//!
//! EVERY READ IS BOUNDED. These are bytes somebody picked from a disk — or, for a sticker, that
//! another member uploaded. A length that runs past the end ends the walk or refuses the picture,
//! as the C# does. Where the C# multiplies two sides in an `int` and lets the product wrap (a GIF
//! image of 65 535 × 65 535 makes a negative array length and throws), the product is checked here
//! and a picture too large for an `i32` count of bytes — the C# array's limit — is refused: Rust
//! would otherwise try to allocate it, and a failed allocation ends the process.
//!
//! WHICH FILES ARE ANIMATED is `fc_text::pack::is_animated`, the tests "Sticker pack" uses
//! (`fc_core::text::sticker_file` says why it is called rather than `StickerFile.cs` ported). It
//! differs from Windows' `StickerFile` in four edge cases (`linux/docs/carry-forward.md`, "Found in
//! the other clients", 9), two of which reach this: a WebP whose `VP8X` is not its first chunk is
//! a still here, and a GIF cut short after its looping extension is an animation here.

use std::rc::Rc;

use fc_text::{animation_source, media, pack};

use crate::animation_timing;

/// The three animated pictures the protocol turns into a video (docs/protocol.md, "Animations",
/// "What becomes one").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnimationFormat {
    Gif,
    WebP,
    Apng,
}

/// How a frame's pixels meet what is already on the canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrameBlend {
    /// They replace it, transparency and all (APNG `APNG_BLEND_OP_SOURCE`, a WebP frame marked
    /// "do not blend").
    Source,
    /// They are drawn over it (APNG `OVER`, WebP's alpha-blending, and every GIF frame: a
    /// transparent index draws nothing).
    Over,
}

/// What happens to a frame's rectangle once its time is up, before the next frame is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrameDisposal {
    /// It stays (GIF 0 and 1, and the values the format leaves undefined).
    None,
    /// It is cleared to transparent — what every browser does with GIF's "restore to background"
    /// too.
    Background,
    /// It goes back to what was there before the frame was drawn.
    Previous,
}

/// One frame of an animated picture: where it is drawn, how long it is stated to stand, and how it
/// is composited.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AnimationFrame {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub stated_ms: u32,
    pub blend: FrameBlend,
    pub disposal: FrameDisposal,
}

impl AnimationFrame {
    /// `new AnimationFrame(x, y, width, height, statedMs, blend, disposal)`.
    pub fn new(
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        stated_ms: u32,
        blend: FrameBlend,
        disposal: FrameDisposal,
    ) -> Self {
        AnimationFrame {
            x,
            y,
            width,
            height,
            stated_ms,
            blend,
            disposal,
        }
    }

    /// How many bytes this frame's straight-BGRA rectangle is — `None` when it is not a size an
    /// `i32` can count (the C#'s `Width × Height × 4`, which wrapped).
    pub fn pixel_bytes(&self) -> Option<usize> {
        bgra_len(self.width, self.height)
    }
}

/// `width × height × 4` as a length, or `None` past what an `i32` counts (the C#'s array limit) or
/// for a negative side.
pub(crate) fn bgra_len(width: i32, height: i32) -> Option<usize> {
    if width < 0 || height < 0 {
        return None;
    }
    let length = width.checked_mul(height)?.checked_mul(4)?;
    usize::try_from(length).ok()
}

/// An animated picture's canvas and frames, as its own bytes state them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnimationLayout {
    pub format: AnimationFormat,
    pub width: i32,
    pub height: i32,
    pub frames: Vec<AnimationFrame>,
}

impl AnimationLayout {
    pub fn new(
        format: AnimationFormat,
        width: i32,
        height: i32,
        frames: Vec<AnimationFrame>,
    ) -> Self {
        AnimationLayout {
            format,
            width,
            height,
            frames,
        }
    }

    /// Each frame's stated length, in order — what
    /// [`animation_timing::plan`](crate::animation_timing::plan) is given.
    pub fn stated_ms(&self) -> Vec<u32> {
        self.frames.iter().map(|frame| frame.stated_ms).collect()
    }

    /// The same animation as a codec that hands out WHOLE, already composited pictures would give
    /// it: every frame the whole canvas, replacing the last.
    pub fn whole(&self) -> AnimationLayout {
        AnimationLayout {
            frames: self
                .frames
                .iter()
                .map(|frame| AnimationFrame {
                    x: 0,
                    y: 0,
                    width: self.width,
                    height: self.height,
                    blend: FrameBlend::Source,
                    disposal: FrameDisposal::None,
                    ..*frame
                })
                .collect(),
            ..self.clone()
        }
    }
}

/// Which picked pictures are ANIMATIONS, and how one is drawn once it has arrived
/// (`AnimationPrep`; docs/protocol.md, "Animations").
///
/// "Animated" is the picture's own statement, read from its bytes with the tests "Sticker pack"
/// already uses: a GIF with a second image, a WebP whose `VP8X` sets the animation flag, a PNG with
/// an `acTL` before its `IDAT`. A still GIF, WebP or BMP is none of these, and goes exactly as
/// before.
pub mod animation_prep {
    use super::*;

    /// Above this an animation's tile shows its poster and plays only when clicked — the bound on
    /// what a tile downloads (`fc_text::animation_source::TILE_PLAYS_UP_TO_BYTES`).
    pub const TILE_BYTES: i64 = animation_source::TILE_PLAYS_UP_TO_BYTES as i64;

    /// Whether these first bytes could begin an animated picture, so the whole file is worth
    /// reading to find out.
    pub fn may_be(head: &[u8]) -> bool {
        animation_source::Format::of(head).is_some()
    }

    /// Which animation these bytes are, or `None` for anything that is not one — a still picture
    /// included.
    pub fn animated(bytes: &[u8]) -> Option<AnimationFormat> {
        if pack::is_gif(bytes) {
            return pack::is_animated(bytes).then_some(AnimationFormat::Gif);
        }
        match pack::type_of(bytes) {
            Some(pack::WEBP) if pack::is_animated(bytes) => Some(AnimationFormat::WebP),
            Some(pack::PNG) if pack::is_animated(bytes) => Some(AnimationFormat::Apng),
            _ => None,
        }
    }

    /// Whether a picked file goes as an animation: where the chat takes them (`allowed` — not the
    /// assistant's, whose model is shown a photograph; not the board, nor a profile picture),
    /// within the attachment ceiling, and animated by its own statement. Anything else takes the
    /// route it always took (`fc_text::media::route`).
    pub fn route(allowed: bool, size: i64, bytes: &[u8]) -> Option<AnimationFormat> {
        if allowed && size > 0 && (size as u64) <= media::SIZE_LIMIT {
            animated(bytes)
        } else {
            None
        }
    }

    /// The canvas and frames of an animated picture, or `None` when its bytes cannot be walked.
    pub fn layout(bytes: &[u8]) -> Option<AnimationLayout> {
        match animated(bytes)? {
            AnimationFormat::Gif => GifPicture::read(bytes).map(|picture| picture.layout),
            AnimationFormat::WebP => WebPAnimation::read(bytes).map(|picture| picture.layout),
            AnimationFormat::Apng => ApngPicture::read(bytes).map(|picture| picture.layout),
        }
    }

    /// Whether an arrived animation's tile plays by itself: the system has not asked for less
    /// motion, and the video is within [`TILE_BYTES`] — the one video a tile downloads to draw
    /// itself. A size nobody stated is not presumed small.
    ///
    /// `fc_text::animation_source::plays_in_tile` answers, after the one rule of the C#'s it lacks:
    /// a size of 0 is not "within" anything (the web client would play it; no server sends one).
    pub fn plays_in_tile(animations_wanted: bool, size: Option<i64>) -> bool {
        size.is_some_and(|size| size > 0)
            && animation_source::plays_in_tile(size, !animations_wanted)
    }
}

// ---- a GIF ----------------------------------------------------------------------------------

/// One image of a GIF: its frame, where its colour table and its data are, and how its indexes
/// read.
#[derive(Debug, Clone)]
struct GifImage {
    frame: AnimationFrame,
    palette_at: usize,
    palette_entries: usize,
    transparent: Option<u8>,
    interlaced: bool,
    data_at: usize,
}

/// A GIF decoded by this client: its blocks walked, each image's LZW data unpacked into its own
/// rectangle.
///
/// WHY NOT THE PLATFORM'S DECODER. Windows Imaging hands out each image as the RAW sub-rectangle
/// the file stores, with disposal and transparency left to the caller — so compositing was ours
/// either way, and decoding as well makes the whole picture a function of its bytes.
///
/// EVERY READ IS BOUNDED. A length that runs past the end ends the walk; an image whose data stops
/// early leaves the rest of its rectangle undrawn, as a browser does; a frame with no colour table
/// at all cannot be drawn, and the picture is not one this can read.
#[derive(Debug, Clone)]
pub struct GifPicture {
    bytes: Rc<[u8]>,
    images: Vec<GifImage>,
    /// The canvas and the frames.
    pub layout: AnimationLayout,
}

impl GifPicture {
    /// The picture's blocks, or `None` when they cannot be walked or hold no image.
    pub fn read(bytes: impl Into<Rc<[u8]>>) -> Option<GifPicture> {
        let bytes: Rc<[u8]> = bytes.into();
        let length = bytes.len();
        if !pack::is_gif(&bytes) || length < 13 {
            return None;
        }
        let width = u16_le(&bytes, 6);
        let height = u16_le(&bytes, 8);
        let packed = bytes[10];
        let mut global: Option<usize> = None;
        let mut global_entries = 0usize;
        let mut at = 13usize;
        if packed & 0x80 != 0 {
            global_entries = 1 << ((packed & 0x07) + 1);
            global = Some(at);
            at += 3 * global_entries;
        }
        let mut images: Vec<GifImage> = Vec::new();
        // What the next image's Graphic Control Extension said, if one came.
        let mut delay: u16 = 0;
        let mut disposal = FrameDisposal::None;
        let mut transparent: Option<u8> = None;
        while at < length {
            match bytes[at] {
                0x21 => {
                    if at + 2 > length {
                        break;
                    }
                    if bytes[at + 1] == 0xF9 && at + 8 <= length && bytes[at + 2] >= 4 {
                        let control = bytes[at + 3];
                        disposal = match (control >> 2) & 0x07 {
                            2 => FrameDisposal::Background,
                            3 => FrameDisposal::Previous,
                            _ => FrameDisposal::None,
                        };
                        delay = u16_le(&bytes, at + 4);
                        transparent = (control & 0x01 != 0).then_some(bytes[at + 6]);
                    }
                    at = sub_blocks(&bytes, at + 2);
                }
                0x2C => {
                    if at + 10 > length {
                        break;
                    }
                    let left = u16_le(&bytes, at + 1);
                    let top = u16_le(&bytes, at + 3);
                    let frame_width = u16_le(&bytes, at + 5);
                    let frame_height = u16_le(&bytes, at + 7);
                    let image_packed = bytes[at + 9];
                    at += 10;
                    let mut palette = global;
                    let mut palette_entries = global_entries;
                    if image_packed & 0x80 != 0 {
                        palette_entries = 1 << ((image_packed & 0x07) + 1);
                        palette = Some(at);
                        at += 3 * palette_entries;
                    }
                    if at >= length {
                        break;
                    }
                    let data_at = at;
                    at = sub_blocks(&bytes, at + 1);
                    // Nothing to say what its indexes are: not a picture this can draw.
                    let palette_at = palette.filter(|&p| p + 3 * palette_entries <= length)?;
                    images.push(GifImage {
                        frame: AnimationFrame::new(
                            i32::from(left),
                            i32::from(top),
                            i32::from(frame_width),
                            i32::from(frame_height),
                            animation_timing::gif_delay_ms(delay),
                            FrameBlend::Over,
                            disposal,
                        ),
                        palette_at,
                        palette_entries,
                        transparent,
                        interlaced: image_packed & 0x40 != 0,
                        data_at,
                    });
                    // A control extension is for the ONE image after it.
                    delay = 0;
                    disposal = FrameDisposal::None;
                    transparent = None;
                }
                // The trailer (0x3B), or bytes that are not a GIF's: either way the end.
                _ => break,
            }
        }
        if images.is_empty() {
            return None;
        }
        // A logical screen of nothing is a GIF whose images say how big it is.
        let canvas_width = if width > 0 {
            i32::from(width)
        } else {
            images.iter().map(|i| i.frame.x + i.frame.width).max()?
        };
        let canvas_height = if height > 0 {
            i32::from(height)
        } else {
            images.iter().map(|i| i.frame.y + i.frame.height).max()?
        };
        if canvas_width <= 0 || canvas_height <= 0 {
            return None;
        }
        let layout = AnimationLayout::new(
            AnimationFormat::Gif,
            canvas_width,
            canvas_height,
            images.iter().map(|image| image.frame).collect(),
        );
        Some(GifPicture {
            bytes,
            images,
            layout,
        })
    }

    /// Image `index`'s own rectangle as straight BGRA, `width × height × 4` bytes: its transparent
    /// index, an index past its colour table and every pixel its data never reached all come out
    /// with an alpha of 0, so drawing it OVER the canvas leaves those pixels as they were.
    ///
    /// `None` for an index past the last image, and for a rectangle too large to count in an `i32`
    /// (where the C# threw).
    pub fn pixels(&self, index: usize) -> Option<Vec<u8>> {
        let image = self.images.get(index)?;
        let frame = image.frame;
        let mut pixels = vec![0u8; frame.pixel_bytes()?];
        let count = pixels.len() / 4;
        if count == 0 {
            return Some(pixels);
        }
        let width = frame.width as usize;
        let mut indexes = vec![0u8; count];
        let decoded = GifPicture::lzw(&self.bytes, image.data_at, &mut indexes);
        for (at, &colour) in indexes.iter().enumerate().take(decoded) {
            let row = if image.interlaced {
                GifPicture::interlaced_row(at / width, frame.height as usize)
            } else {
                at / width
            };
            let target = (row * width + at % width) * 4;
            if Some(colour) == image.transparent || usize::from(colour) >= image.palette_entries {
                continue;
            }
            let rgb = image.palette_at + 3 * usize::from(colour);
            pixels[target] = self.bytes[rgb + 2];
            pixels[target + 1] = self.bytes[rgb + 1];
            pixels[target + 2] = self.bytes[rgb];
            pixels[target + 3] = 0xFF;
        }
        Some(pixels)
    }

    /// The row the `written`th row of an interlaced image belongs on: every 8th from 0, every 8th
    /// from 4, every 4th from 2, every 2nd from 1.
    pub fn interlaced_row(written: usize, height: usize) -> usize {
        let mut written = written;
        let first = height.div_ceil(8);
        if written < first {
            return written * 8;
        }
        written -= first;
        let second = (height + 3) / 8;
        if written < second {
            return 4 + written * 8;
        }
        written -= second;
        let third = (height + 1) / 4;
        if written < third {
            return 2 + written * 4;
        }
        written -= third;
        1 + written * 2
    }

    /// The image data at `at` (its minimum code size, then its sub-blocks) unpacked into `output`.
    /// Answers how many indexes it held — fewer than asked for when the data stops early.
    pub fn lzw(bytes: &[u8], at: usize, output: &mut [u8]) -> usize {
        const MAX_CODES: usize = 4096;
        if at >= bytes.len() {
            return 0;
        }
        let minimum = u32::from(bytes[at]);
        // The format's own range: 2 to 8 bits a colour, and room above it for the decoder's
        // 12-bit codes.
        if !(2..=11).contains(&minimum) {
            return 0;
        }
        let mut prefix = [0i16; MAX_CODES];
        let mut suffix = [0u8; MAX_CODES];
        let mut first = [0u8; MAX_CODES];
        let mut stack = [0u8; MAX_CODES + 1];
        let clear = 1usize << minimum;
        let end = clear + 1;
        for code in 0..clear {
            prefix[code] = -1;
            suffix[code] = code as u8;
            first[code] = code as u8;
        }
        let mut size = minimum + 1;
        let mut next = clear + 2;
        let mut previous: Option<usize> = None;
        let mut written = 0usize;
        // The bit reader over the sub-blocks.
        let mut left = 0usize;
        let mut position = at + 1;
        let mut bits = 0u32;
        let mut held = 0u32;
        while written < output.len() {
            while held < size {
                if left == 0 {
                    if position >= bytes.len() || bytes[position] == 0 {
                        return written;
                    }
                    left = usize::from(bytes[position]);
                    position += 1;
                }
                if position >= bytes.len() {
                    return written;
                }
                bits |= u32::from(bytes[position]) << held;
                position += 1;
                left -= 1;
                held += 8;
            }
            let code = (bits & ((1 << size) - 1)) as usize;
            bits >>= size;
            held -= size;
            if code == clear {
                size = minimum + 1;
                next = clear + 2;
                previous = None;
                continue;
            }
            if code == end {
                return written;
            }
            let emit = match previous {
                None => {
                    if code >= clear {
                        // The first code after a clear must be a colour; anything else is data
                        // that cannot be read on.
                        return written;
                    }
                    code
                }
                Some(previous) if code < next => {
                    if next < MAX_CODES {
                        prefix[next] = previous as i16;
                        suffix[next] = first[code];
                        first[next] = first[previous];
                        next += 1;
                    }
                    code
                }
                Some(previous) if code == next && next < MAX_CODES => {
                    // The one code the decoder has not made yet: the previous string and its own
                    // first byte.
                    prefix[next] = previous as i16;
                    suffix[next] = first[previous];
                    first[next] = first[previous];
                    next += 1;
                    code
                }
                Some(_) => return written,
            };
            // The string, back to front, then out in order.
            let mut depth = 0usize;
            let mut walk = emit as i32;
            while walk >= 0 && depth < stack.len() {
                stack[depth] = suffix[walk as usize];
                depth += 1;
                walk = i32::from(prefix[walk as usize]);
            }
            while depth > 0 && written < output.len() {
                depth -= 1;
                output[written] = stack[depth];
                written += 1;
            }
            previous = Some(emit);
            if next == (1 << size) && size < 12 {
                size += 1;
            }
        }
        written
    }
}

fn u16_le(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

/// Past a run of sub-blocks; the end of the bytes when they run out first.
fn sub_blocks(bytes: &[u8], mut at: usize) -> usize {
    while at < bytes.len() {
        let length = usize::from(bytes[at]);
        at += 1 + length;
        if length == 0 {
            return at;
        }
    }
    bytes.len()
}

// ---- an APNG --------------------------------------------------------------------------------

const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// An animated PNG taken apart by this client: its `fcTL` frames, and each one rebuilt as a
/// STANDALONE PNG — the file's own header chunks around that frame's data — which any PNG decoder
/// reads as the still picture it is.
///
/// WHY. A frame's data is ordinary PNG image data (zlib, filtered rows) at the frame's own size, so
/// a PNG whose `IHDR` names that size and whose `IDAT` is that data IS the frame — decoding stays
/// the platform's (gdk-pixbuf does not animate APNG either), and only the cutting is ours.
///
/// A frame that reaches past the canvas, chunks out of order, or an animation with no frames is a
/// picture this does not read — and goes as the file it is (rule C). So is a canvas side past
/// 2³¹ − 1, which the PNG specification forbids and the C#'s `(int)` casts wrapped.
#[derive(Debug, Clone)]
pub struct ApngPicture {
    bytes: Rc<[u8]>,
    ihdr: [u8; 13],
    /// The whole chunks (length, type, payload, CRC) every frame shares.
    shared: Vec<std::ops::Range<usize>>,
    /// Each frame's image data, in parts.
    data: Vec<Vec<std::ops::Range<usize>>>,
    /// The canvas and the frames.
    pub layout: AnimationLayout,
}

impl ApngPicture {
    /// The picture's frames, or `None` when it is not an APNG this can take apart.
    pub fn read(bytes: impl Into<Rc<[u8]>>) -> Option<ApngPicture> {
        let bytes: Rc<[u8]> = bytes.into();
        if !bytes.starts_with(&PNG_SIGNATURE) {
            return None;
        }
        let mut ihdr: Option<[u8; 13]> = None;
        let mut shared = Vec::new();
        let mut frames: Vec<AnimationFrame> = Vec::new();
        let mut data: Vec<Vec<std::ops::Range<usize>>> = Vec::new();
        let mut seen_image = false;
        let mut at = PNG_SIGNATURE.len();
        // Whether the frame being collected takes the IDAT (an fcTL before it) or fdATs.
        let mut collecting = false;
        while at + 12 <= bytes.len() {
            let length = u32_be(&bytes, at);
            if length as usize > bytes.len() - at - 12 {
                return None;
            }
            let length = length as usize;
            let kind: [u8; 4] = [bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]];
            let payload = at + 8..at + 8 + length;
            let whole = at..at + 12 + length;
            at += length + 12;
            match &kind {
                b"IHDR" => {
                    if ihdr.is_some() || length != 13 {
                        return None;
                    }
                    let mut header = [0u8; 13];
                    header.copy_from_slice(&bytes[payload]);
                    ihdr = Some(header);
                }
                b"acTL" => {}
                b"fcTL" => {
                    let header = ihdr?;
                    if length < 26 {
                        return None;
                    }
                    let control = &bytes[payload];
                    let frame_width = u32_be(control, 4);
                    let frame_height = u32_be(control, 8);
                    let x = u32_be(control, 12);
                    let y = u32_be(control, 16);
                    let numerator = u16::from_be_bytes([control[20], control[21]]);
                    let denominator = u16::from_be_bytes([control[22], control[23]]);
                    let dispose = control[24];
                    let blend = control[25];
                    let canvas_width = u32_be(&header, 0);
                    let canvas_height = u32_be(&header, 4);
                    if frame_width == 0
                        || frame_height == 0
                        || u64::from(x) + u64::from(frame_width) > u64::from(canvas_width)
                        || u64::from(y) + u64::from(frame_height) > u64::from(canvas_height)
                        || i32::try_from(canvas_width).is_err()
                        || i32::try_from(canvas_height).is_err()
                    {
                        return None;
                    }
                    frames.push(AnimationFrame::new(
                        x as i32,
                        y as i32,
                        frame_width as i32,
                        frame_height as i32,
                        animation_timing::apng_delay_ms(numerator, denominator),
                        if blend == 1 {
                            FrameBlend::Over
                        } else {
                            FrameBlend::Source
                        },
                        match dispose {
                            1 => FrameDisposal::Background,
                            // "If the first fcTL chunk uses a dispose_op of APNG_DISPOSE_OP_PREVIOUS
                            // it should be treated as APNG_DISPOSE_OP_BACKGROUND."
                            2 if frames.is_empty() => FrameDisposal::Background,
                            2 => FrameDisposal::Previous,
                            _ => FrameDisposal::None,
                        },
                    ));
                    data.push(Vec::new());
                    collecting = !seen_image;
                }
                b"IDAT" => {
                    seen_image = true;
                    if collecting && data.len() == 1 {
                        data[0].push(payload);
                    }
                }
                b"fdAT" => {
                    if length < 4 {
                        return None;
                    }
                    // A sequence number, then the frame's image data.
                    data.last_mut()?.push(payload.start + 4..payload.end);
                }
                b"IEND" => at = bytes.len(),
                _ => {
                    // Everything the file says about its pixels before they begin — PLTE, tRNS,
                    // gAMA, sRGB, iCCP… — is every frame's too.
                    if !seen_image {
                        shared.push(whole);
                    }
                }
            }
        }
        let ihdr = ihdr?;
        if frames.is_empty() || data.iter().any(Vec::is_empty) {
            return None;
        }
        let layout = AnimationLayout::new(
            AnimationFormat::Apng,
            u32_be(&ihdr, 0) as i32,
            u32_be(&ihdr, 4) as i32,
            frames,
        );
        Some(ApngPicture {
            bytes,
            ihdr,
            shared,
            data,
            layout,
        })
    }

    /// Frame `index` as a PNG of its own: the file's header with the frame's size, the chunks
    /// before the image data, the frame's data as one `IDAT`, and the end. `None` for an index past
    /// the last frame.
    pub fn standalone(&self, index: usize) -> Option<Vec<u8>> {
        let frame = self.layout.frames.get(index)?;
        let parts = self.data.get(index)?;
        let mut header = self.ihdr;
        header[0..4].copy_from_slice(&(frame.width as u32).to_be_bytes());
        header[4..8].copy_from_slice(&(frame.height as u32).to_be_bytes());
        let mut output = Vec::new();
        output.extend_from_slice(&PNG_SIGNATURE);
        png_chunk(&mut output, b"IHDR", &header);
        for chunk in &self.shared {
            output.extend_from_slice(&self.bytes[chunk.clone()]);
        }
        let joined: Vec<u8> = parts
            .iter()
            .flat_map(|part| self.bytes[part.clone()].iter().copied())
            .collect();
        png_chunk(&mut output, b"IDAT", &joined);
        png_chunk(&mut output, b"IEND", &[]);
        Some(output)
    }
}

fn u32_be(bytes: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn png_chunk(output: &mut Vec<u8>, kind: &[u8; 4], payload: &[u8]) {
    output.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    let mut typed = Vec::with_capacity(4 + payload.len());
    typed.extend_from_slice(kind);
    typed.extend_from_slice(payload);
    output.extend_from_slice(&typed);
    output.extend_from_slice(&crc32::of(&typed).to_be_bytes());
}

// ---- an animated WebP -----------------------------------------------------------------------

/// An animated WebP taken apart by this client: its `ANMF` frames, and each one rewrapped as a
/// STANDALONE still WebP that any WebP decoder reads.
///
/// A frame's own bitstream (`VP8` or `VP8L`, with its `ALPH`) is exactly a still WebP's, so wrapped
/// in a RIFF header of its own it is one — and the platform's still-picture path decodes it.
#[derive(Debug, Clone)]
pub struct WebPAnimation {
    bytes: Rc<[u8]>,
    /// Each frame's data after its sixteen-byte `ANMF` header.
    frame_data: Vec<std::ops::Range<usize>>,
    /// The canvas and the frames.
    pub layout: AnimationLayout,
}

impl WebPAnimation {
    /// The animation's frames, or `None` when the file is not an animated WebP this can take
    /// apart.
    pub fn read(bytes: impl Into<Rc<[u8]>>) -> Option<WebPAnimation> {
        let bytes: Rc<[u8]> = bytes.into();
        if pack::type_of(&bytes) != Some(pack::WEBP) {
            return None;
        }
        let (mut width, mut height) = (0i32, 0i32);
        let mut animated = false;
        let mut frames = Vec::new();
        let mut data = Vec::new();
        for (kind, payload) in riff_chunks(&bytes, 12, bytes.len()) {
            let span = &bytes[payload.clone()];
            match &kind {
                b"VP8X" => {
                    if span.len() < 10 {
                        return None;
                    }
                    animated = span[0] & 0x02 != 0;
                    width = u24(&span[4..]) + 1;
                    height = u24(&span[7..]) + 1;
                }
                b"ANMF" => {
                    if span.len() < 16 || width == 0 {
                        return None;
                    }
                    let x = u24(span) * 2;
                    let y = u24(&span[3..]) * 2;
                    let frame_width = u24(&span[6..]) + 1;
                    let frame_height = u24(&span[9..]) + 1;
                    let duration = u24(&span[12..]) as u32;
                    let flags = span[15];
                    if i64::from(x) + i64::from(frame_width) > i64::from(width)
                        || i64::from(y) + i64::from(frame_height) > i64::from(height)
                    {
                        return None;
                    }
                    frames.push(AnimationFrame::new(
                        x,
                        y,
                        frame_width,
                        frame_height,
                        duration,
                        if flags & 0x02 != 0 {
                            FrameBlend::Source
                        } else {
                            FrameBlend::Over
                        },
                        if flags & 0x01 != 0 {
                            FrameDisposal::Background
                        } else {
                            FrameDisposal::None
                        },
                    ));
                    data.push(payload.start + 16..payload.end);
                }
                _ => {}
            }
        }
        if !animated || frames.is_empty() {
            return None;
        }
        Some(WebPAnimation {
            bytes,
            frame_data: data,
            layout: AnimationLayout::new(AnimationFormat::WebP, width, height, frames),
        })
    }

    /// Frame `index` as a still WebP: its `VP8L` alone, its `VP8` alone, or — where it carries an
    /// `ALPH` — an extended file whose `VP8X` says alpha, at the frame's own size. `None` for a
    /// frame whose data holds no bitstream, and for an index past the last frame.
    pub fn standalone(&self, index: usize) -> Option<Vec<u8>> {
        let frame = self.layout.frames.get(index)?;
        let own = self.frame_data.get(index)?.clone();
        let (mut alpha, mut lossy, mut lossless) = (None, None, None);
        for (kind, payload) in riff_chunks(&self.bytes, own.start, own.end) {
            match &kind {
                b"ALPH" => {
                    alpha.get_or_insert(payload);
                }
                b"VP8 " => {
                    lossy.get_or_insert(payload);
                }
                b"VP8L" => {
                    lossless.get_or_insert(payload);
                }
                _ => {}
            }
        }
        let mut body = b"WEBP".to_vec();
        if let Some(vp8l) = lossless {
            riff_chunk(&mut body, b"VP8L", &self.bytes[vp8l]);
        } else {
            // No bitstream at all: nothing to decode.
            let vp8 = lossy?;
            if let Some(alph) = alpha {
                let mut extended = [0u8; 10];
                extended[0] = 0x10;
                write_u24(&mut extended[4..], frame.width - 1);
                write_u24(&mut extended[7..], frame.height - 1);
                riff_chunk(&mut body, b"VP8X", &extended);
                riff_chunk(&mut body, b"ALPH", &self.bytes[alph]);
            }
            riff_chunk(&mut body, b"VP8 ", &self.bytes[vp8]);
        }
        let mut file = Vec::with_capacity(8 + body.len());
        file.extend_from_slice(b"RIFF");
        file.extend_from_slice(&(body.len() as u32).to_le_bytes());
        file.extend_from_slice(&body);
        Some(file)
    }
}

fn u24(at: &[u8]) -> i32 {
    i32::from(at[0]) | (i32::from(at[1]) << 8) | (i32::from(at[2]) << 16)
}

fn write_u24(at: &mut [u8], value: i32) {
    at[0] = value as u8;
    at[1] = (value >> 8) as u8;
    at[2] = (value >> 16) as u8;
}

fn riff_chunk(output: &mut Vec<u8>, kind: &[u8; 4], payload: &[u8]) {
    output.extend_from_slice(kind);
    output.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    output.extend_from_slice(payload);
    if payload.len() & 1 != 0 {
        output.push(0);
    }
}

/// The RIFF chunks in `start..end`, each bounded by what the bytes hold: a chunk that claims more
/// is read as far as the bytes go, and ends the walk.
fn riff_chunks(bytes: &[u8], start: usize, end: usize) -> Vec<([u8; 4], std::ops::Range<usize>)> {
    let mut chunks = Vec::new();
    let mut at = start;
    while at + 8 <= end {
        let size = u32::from_le_bytes([bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]]);
        let held = (size as usize).min(end - at - 8);
        chunks.push((
            [bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]],
            at + 8..at + 8 + held,
        ));
        let next = at as u64 + 8 + u64::from(size) + u64::from(size & 1);
        if next > end as u64 {
            break;
        }
        at = next as usize;
    }
    chunks
}

/// PNG's CRC-32 (ISO 3309, the polynomial zlib uses), for the chunks a rebuilt frame is written
/// with (`Crc32`).
pub mod crc32 {
    const TABLE: [u32; 256] = {
        let mut table = [0u32; 256];
        let mut n = 0;
        while n < 256 {
            let mut c = n as u32;
            let mut k = 0;
            while k < 8 {
                c = if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
                k += 1;
            }
            table[n] = c;
            n += 1;
        }
        table
    };

    /// The CRC of `bytes`.
    pub fn of(bytes: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for &value in bytes {
            crc = TABLE[((crc ^ u32::from(value)) & 0xFF) as usize] ^ (crc >> 8);
        }
        crc ^ 0xFFFF_FFFF
    }
}
