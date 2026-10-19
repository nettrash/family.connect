//! What an animated picture STATES about itself (issue #96, docs/protocol.md
//! "Animations"): the size of its canvas and how long each of its frames is
//! stated to last, read from its own bytes — a GIF's Graphic Control
//! Extensions, a WebP's `ANMF` chunks, an APNG's `fcTL` chunks. That list is
//! what [`crate::animation::plan`] is given; the frames themselves are
//! decoded (and composited) by the platform.
//!
//! And the drawing test: which message is drawn as an animation.
//!
//! Every reader here walks the format's own structure and nothing more: no
//! pixel is decoded. Each answers None for bytes that are not the format, or
//! that state no frame at all — the caller then sends the picture as it
//! always went (rule C). A file cut short is read as far as it goes, as a
//! browser would show it; whether the platform's decoder agrees on the count
//! is the caller's to check.

use crate::animation::{apng_delay_ms, gif_delay_ms};
use crate::pack;

/// The three formats that can animate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Gif,
    Webp,
    Png,
}

impl Format {
    /// The type a decoder is told the bytes are.
    pub fn mime(self) -> &'static str {
        match self {
            Format::Gif => "image/gif",
            Format::Webp => pack::WEBP,
            Format::Png => pack::PNG,
        }
    }

    /// Which of the three these bytes are, by their magic numbers alone.
    pub fn of(head: &[u8]) -> Option<Format> {
        if pack::is_gif(head) {
            return Some(Format::Gif);
        }
        match pack::type_of(head) {
            Some(pack::WEBP) => Some(Format::Webp),
            Some(pack::PNG) => Some(Format::Png),
            _ => None,
        }
    }
}

/// What a picture states: its canvas, and each frame's stated length in
/// milliseconds in the source's order — before the shortest-frame rule,
/// which [`crate::animation::plan`] applies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Statement {
    pub format: Format,
    pub width: u32,
    pub height: u32,
    pub stated_ms: Vec<u32>,
}

/// What `bytes` state, whichever of the three formats they are.
pub fn read(bytes: &[u8]) -> Option<Statement> {
    match Format::of(bytes)? {
        Format::Gif => gif(bytes),
        Format::Webp => webp(bytes),
        Format::Png => apng(bytes),
    }
}

fn u16_le(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*bytes.get(at)?, *bytes.get(at + 1)?]))
}

fn u24_le(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes([
        *bytes.get(at)?,
        *bytes.get(at + 1)?,
        *bytes.get(at + 2)?,
        0,
    ]))
}

fn u32_le(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn u16_be(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*bytes.get(at)?, *bytes.get(at + 1)?]))
}

fn u32_be(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// A GIF: the logical screen's size, and for every image in it the delay
/// its Graphic Control Extension states — in hundredths of a second, read as
/// [`gif_delay_ms`] reads them — or 0 for an image with none in front of it
/// (which the shortest-frame rule then shows for 100 ms, as browsers do).
///
/// A Graphic Control Extension applies to the ONE image that follows it; a
/// second one before that image replaces the first.
pub fn gif(bytes: &[u8]) -> Option<Statement> {
    /// Past a run of sub-blocks; None when the bytes run out first.
    fn past_sub_blocks(bytes: &[u8], mut at: usize) -> Option<usize> {
        loop {
            let length = usize::from(*bytes.get(at)?);
            at += 1;
            if length == 0 {
                return Some(at);
            }
            at += length;
        }
    }
    fn table(packed: u8) -> usize {
        if packed & 0x80 != 0 {
            3 << ((packed & 0x07) + 1)
        } else {
            0
        }
    }
    if !pack::is_gif(bytes) {
        return None;
    }
    let width = u32::from(u16_le(bytes, 6)?);
    let height = u32::from(u16_le(bytes, 8)?);
    let packed = *bytes.get(10)?;
    let mut at = 13 + table(packed);
    let mut stated_ms = Vec::new();
    let mut delay: Option<u16> = None;
    loop {
        match bytes.get(at) {
            Some(0x2C) => {
                stated_ms.push(gif_delay_ms(delay.take().unwrap_or(0)));
                let Some(&packed) = bytes.get(at + 9) else {
                    break;
                };
                match past_sub_blocks(bytes, at + 10 + table(packed) + 1) {
                    Some(next) => at = next,
                    None => break,
                }
            }
            Some(0x21) => {
                // A Graphic Control Extension: label 0xF9, a block of four —
                // packed fields, the delay (little-endian), the transparent
                // index — and its terminator.
                if bytes.get(at + 1) == Some(&0xF9) && bytes.get(at + 2) == Some(&4) {
                    delay = u16_le(bytes, at + 4);
                }
                match past_sub_blocks(bytes, at + 2) {
                    Some(next) => at = next,
                    None => break,
                }
            }
            // The trailer, the end of what was read, or something a GIF does
            // not hold: what was read is what there is.
            _ => break,
        }
    }
    (!stated_ms.is_empty()).then_some(Statement {
        format: Format::Gif,
        width,
        height,
        stated_ms,
    })
}

/// An animated WebP: the canvas its `VP8X` chunk states (each side stored
/// less one, in 24 bits), and every `ANMF` frame's duration, which is
/// milliseconds as stated. A WebP with no `VP8X`, or no `ANMF`, states no
/// animation: None.
pub fn webp(bytes: &[u8]) -> Option<Statement> {
    if pack::type_of(bytes) != Some(pack::WEBP) {
        return None;
    }
    let mut at = 12usize;
    let mut canvas = None;
    let mut stated_ms = Vec::new();
    while let Some(fourcc) = bytes.get(at..at + 4) {
        let Some(size) = u32_le(bytes, at + 4) else {
            break;
        };
        let data = at + 8;
        match fourcc {
            b"VP8X" => {
                canvas = Some((u24_le(bytes, data + 4)? + 1, u24_le(bytes, data + 7)? + 1));
            }
            b"ANMF" => match u24_le(bytes, data + 12) {
                Some(duration) => stated_ms.push(duration),
                None => break,
            },
            _ => {}
        }
        // The chunk, padded to an even length.
        let Some(next) = usize::try_from(size)
            .ok()
            .and_then(|size| data.checked_add(size))
            .and_then(|end| end.checked_add(end & 1))
        else {
            break;
        };
        at = next;
    }
    let (width, height) = canvas?;
    (!stated_ms.is_empty()).then_some(Statement {
        format: Format::Webp,
        width,
        height,
        stated_ms,
    })
}

/// An animated PNG: the size its `IHDR` states, and every `fcTL` frame's
/// delay, `delay_num ÷ delay_den` seconds as [`apng_delay_ms`] reads them.
/// Every `fcTL` is one frame of the animation — the default image is one
/// only when an `fcTL` comes before its `IDAT`, which is exactly when there
/// is one for it. A PNG with no `fcTL` states no animation: None.
pub fn apng(bytes: &[u8]) -> Option<Statement> {
    if pack::type_of(bytes) != Some(pack::PNG) {
        return None;
    }
    let mut at = 8usize;
    let mut size = None;
    let mut stated_ms = Vec::new();
    while let Some(kind) = bytes.get(at + 4..at + 8) {
        let Some(length) = u32_be(bytes, at) else {
            break;
        };
        let data = at + 8;
        match kind {
            b"IHDR" => size = Some((u32_be(bytes, data)?, u32_be(bytes, data + 4)?)),
            // Sequence number, width, height, x, y — four bytes each — then
            // the delay's numerator and denominator.
            b"fcTL" => match (u16_be(bytes, data + 20), u16_be(bytes, data + 22)) {
                (Some(num), Some(den)) => stated_ms.push(apng_delay_ms(num, den)),
                _ => break,
            },
            b"IEND" => break,
            _ => {}
        }
        // Length, type, the data, and its CRC.
        let Some(next) = usize::try_from(length)
            .ok()
            .and_then(|length| data.checked_add(length))
            .and_then(|end| end.checked_add(4))
        else {
            break;
        };
        at = next;
    }
    let (width, height) = size?;
    (!stated_ms.is_empty()).then_some(Statement {
        format: Format::Png,
        width,
        height,
        stated_ms,
    })
}

/// One attachment, as far as the drawing test needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttachmentFlags<'a> {
    /// The attachment's `kind`, as the wire spells it (`"video"`).
    pub kind: &'a str,
    /// Its `animation` — absent on the wire is false.
    pub animation: bool,
}

/// The drawing test (docs/protocol.md, "How it is drawn"), one test on every
/// client, the sticker's and the video message's: exactly ONE attachment, of
/// kind `video`, carrying `animation: true`. A caption is allowed — unlike a
/// video message, an animation may carry one. Anything else is the ordinary
/// message it otherwise is.
pub fn is_animation(attachments: &[AttachmentFlags]) -> bool {
    matches!(attachments, [only] if only.kind == "video" && only.animation)
}

/// The largest animation a TILE downloads to play by itself: 8 MiB. Above
/// it the tile shows its poster and plays only when tapped — the bound on
/// the one exception to "a tile never downloads a video" (docs/protocol.md,
/// "How it is drawn").
pub const TILE_PLAYS_UP_TO_BYTES: u64 = 8 * 1024 * 1024;

/// Whether an animation's tile plays by itself — downloading its video to do
/// it — or shows its poster, still, with the label: it plays unless the
/// system asks for less motion, or its `size` is above
/// [`TILE_PLAYS_UP_TO_BYTES`] (or not known, which no server sends). Either
/// way a tap plays it in its viewer.
pub fn plays_in_tile(size: Option<i64>, reduced_motion: bool) -> bool {
    !reduced_motion
        && size
            .and_then(|size| u64::try_from(size).ok())
            .is_some_and(|size| size <= TILE_PLAYS_UP_TO_BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A GIF of `width × height` whose images are preceded by Graphic
    /// Control Extensions stating `delays` (None: no extension at all).
    fn gif_of(width: u16, height: u16, delays: &[Option<u16>], global_table: bool) -> Vec<u8> {
        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&width.to_le_bytes());
        gif.extend_from_slice(&height.to_le_bytes());
        if global_table {
            // Four colours: 3 << (1 + 1) = 12 bytes.
            gif.extend_from_slice(&[0x81, 0, 0]);
            gif.extend_from_slice(&[0; 12]);
        } else {
            gif.extend_from_slice(&[0x00, 0, 0]);
        }
        // A loop extension, which says nothing about any frame's delay.
        gif.extend_from_slice(&[0x21, 0xFF, 11]);
        gif.extend_from_slice(b"NETSCAPE2.0");
        gif.extend_from_slice(&[3, 1, 0, 0, 0]);
        for delay in delays {
            if let Some(delay) = delay {
                let [lo, hi] = delay.to_le_bytes();
                gif.extend_from_slice(&[0x21, 0xF9, 4, 0x09, lo, hi, 0, 0]);
            }
            // A comment between the extension and its image changes nothing.
            gif.extend_from_slice(&[0x21, 0xFE, 2, b'h', b'i', 0]);
            // A descriptor with a two-colour local table.
            gif.extend_from_slice(&[0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0x80]);
            gif.extend_from_slice(&[0; 6]);
            gif.extend_from_slice(&[2, 2, 0x44, 0x01, 0]);
        }
        gif.push(0x3B);
        gif
    }

    #[test]
    fn a_gif_states_each_images_delay_in_hundredths() {
        let bytes = gif_of(320, 240, &[Some(10), Some(0), Some(7), Some(250)], true);
        let read = read(&bytes).unwrap();
        assert_eq!(read.format, Format::Gif);
        assert_eq!((read.width, read.height), (320, 240));
        assert_eq!(read.stated_ms, vec![100, 0, 70, 2500]);
    }

    #[test]
    fn a_gif_image_with_no_control_extension_is_stated_at_nought() {
        let bytes = gif_of(16, 9, &[None, Some(5), None], false);
        assert_eq!(gif(&bytes).unwrap().stated_ms, vec![0, 50, 0]);
    }

    #[test]
    fn a_control_extension_applies_to_the_one_image_after_it() {
        let mut bytes = gif_of(4, 4, &[Some(3)], false);
        bytes.pop();
        // A second extension replacing nothing yet, then an image.
        bytes.extend_from_slice(&[0x21, 0xF9, 4, 0, 20, 0, 0, 0]);
        bytes.extend_from_slice(&[0x21, 0xF9, 4, 0, 40, 0, 0, 0]);
        bytes.extend_from_slice(&[0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0]);
        bytes.extend_from_slice(&[2, 2, 0x44, 0x01, 0]);
        bytes.push(0x3B);
        assert_eq!(gif(&bytes).unwrap().stated_ms, vec![30, 400]);
    }

    #[test]
    fn a_gif_cut_short_is_read_as_far_as_it_goes() {
        let whole = gif_of(8, 8, &[Some(10), Some(20), Some(30)], true);
        // Cut inside the third image's data.
        let cut = &whole[..whole.len() - 3];
        assert_eq!(gif(cut).unwrap().stated_ms, vec![100, 200, 300]);
        // Cut before any image.
        assert_eq!(gif(&whole[..30]), None);
        assert_eq!(gif(b"GIF89a"), None);
        assert_eq!(gif(b"not a gif at all"), None);
    }

    /// A RIFF chunk: its four characters, its length, its data and the pad.
    fn chunk(fourcc: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = fourcc.to_vec();
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(data);
        if data.len() % 2 == 1 {
            out.push(0);
        }
        out
    }

    fn u24(value: u32) -> [u8; 3] {
        let [a, b, c, _] = value.to_le_bytes();
        [a, b, c]
    }

    fn webp_of(width: u32, height: u32, durations: &[u32]) -> Vec<u8> {
        let mut vp8x = vec![0x12, 0, 0, 0];
        vp8x.extend_from_slice(&u24(width - 1));
        vp8x.extend_from_slice(&u24(height - 1));
        let mut body = b"WEBP".to_vec();
        body.extend(chunk(b"VP8X", &vp8x));
        body.extend(chunk(b"ANIM", &[255, 255, 255, 255, 0, 0]));
        for &duration in durations {
            let mut anmf = Vec::new();
            anmf.extend_from_slice(&u24(0));
            anmf.extend_from_slice(&u24(0));
            anmf.extend_from_slice(&u24(width - 1));
            anmf.extend_from_slice(&u24(height - 1));
            anmf.extend_from_slice(&u24(duration));
            anmf.push(0);
            // An odd-length frame payload, so the padding is exercised.
            anmf.extend(chunk(b"VP8L", &[0x2F, 0, 0, 0, 0]));
            body.extend(chunk(b"ANMF", &anmf));
        }
        let mut riff = b"RIFF".to_vec();
        riff.extend_from_slice(&(body.len() as u32).to_le_bytes());
        riff.extend(body);
        riff
    }

    #[test]
    fn a_webp_states_its_canvas_and_each_frames_milliseconds() {
        let bytes = webp_of(500, 281, &[40, 0, 1000, 16_777_215]);
        let read = read(&bytes).unwrap();
        assert_eq!(read.format, Format::Webp);
        assert_eq!((read.width, read.height), (500, 281));
        assert_eq!(read.stated_ms, vec![40, 0, 1000, 16_777_215]);
    }

    #[test]
    fn a_webp_with_no_frames_or_no_canvas_states_no_animation() {
        assert_eq!(webp(&webp_of(10, 10, &[])), None);
        let mut simple = b"RIFF\x0c\x00\x00\x00WEBP".to_vec();
        simple.extend(chunk(b"VP8L", &[0x2F, 0, 0, 0]));
        assert_eq!(webp(&simple), None);
        assert_eq!(webp(b"RIFF"), None);
    }

    /// A PNG chunk: length, type, data and a CRC this reader never checks.
    fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = (data.len() as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        out.extend_from_slice(&[0; 4]);
        out
    }

    fn fctl(sequence: u32, num: u16, den: u16) -> Vec<u8> {
        let mut data = sequence.to_be_bytes().to_vec();
        data.extend_from_slice(&[0; 16]);
        data.extend_from_slice(&num.to_be_bytes());
        data.extend_from_slice(&den.to_be_bytes());
        data.extend_from_slice(&[0, 0]);
        png_chunk(b"fcTL", &data)
    }

    fn apng_of(
        width: u32,
        height: u32,
        delays: &[(u16, u16)],
        default_is_a_frame: bool,
    ) -> Vec<u8> {
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        let mut ihdr = width.to_be_bytes().to_vec();
        ihdr.extend_from_slice(&height.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        png.extend(png_chunk(b"IHDR", &ihdr));
        let mut actl = (delays.len() as u32).to_be_bytes().to_vec();
        actl.extend_from_slice(&0u32.to_be_bytes());
        png.extend(png_chunk(b"acTL", &actl));
        let mut sequence = 0;
        let mut rest = delays.iter();
        if default_is_a_frame {
            let (num, den) = rest.next().unwrap();
            png.extend(fctl(sequence, *num, *den));
            sequence += 1;
        }
        png.extend(png_chunk(b"IDAT", &[0x78, 0x01]));
        for (num, den) in rest {
            png.extend(fctl(sequence, *num, *den));
            png.extend(png_chunk(b"fdAT", &(sequence + 1).to_be_bytes()));
            sequence += 2;
        }
        png.extend(png_chunk(b"IEND", &[]));
        png
    }

    #[test]
    fn an_apng_states_each_frames_fraction_of_a_second() {
        let bytes = apng_of(100, 50, &[(1, 10), (0, 0), (5, 0), (2, 3)], true);
        let read = read(&bytes).unwrap();
        assert_eq!(read.format, Format::Png);
        assert_eq!((read.width, read.height), (100, 50));
        assert_eq!(read.stated_ms, vec![100, 0, 50, 667]);
    }

    #[test]
    fn a_default_image_outside_the_animation_is_not_a_frame() {
        // The IDAT has no fcTL in front of it: the frames are the two after.
        let bytes = apng_of(10, 10, &[(1, 5), (1, 4)], false);
        assert_eq!(apng(&bytes).unwrap().stated_ms, vec![200, 250]);
    }

    #[test]
    fn a_still_png_states_no_animation() {
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        png.extend(png_chunk(b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0]));
        png.extend(png_chunk(b"IDAT", &[0x78, 0x01]));
        png.extend(png_chunk(b"IEND", &[]));
        assert_eq!(apng(&png), None);
        assert_eq!(read(b"\xFF\xD8\xFF\xE0 a jpeg"), None);
    }

    #[test]
    fn what_is_read_is_what_the_plan_is_given() {
        // A 10 fps GIF with a "0" in it: three ticks a frame, the "0" too.
        let bytes = gif_of(320, 240, &[Some(10), Some(0), Some(10)], true);
        let read = read(&bytes).unwrap();
        let plan = crate::animation::plan(read.width, read.height, &read.stated_ms).unwrap();
        assert_eq!(plan.total_ticks, 9);
        assert_eq!(
            plan.samples.iter().map(|s| s.ticks).collect::<Vec<_>>(),
            vec![3, 3, 3]
        );
    }

    #[test]
    fn the_formats_are_told_by_their_bytes() {
        assert_eq!(Format::of(b"GIF87a...."), Some(Format::Gif));
        assert_eq!(Format::of(b"RIFF\0\0\0\0WEBPVP8X"), Some(Format::Webp));
        assert_eq!(
            Format::of(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]),
            Some(Format::Png)
        );
        assert_eq!(Format::of(b"\xFF\xD8\xFF"), None);
        assert_eq!(Format::Gif.mime(), "image/gif");
        assert_eq!(Format::Webp.mime(), "image/webp");
        assert_eq!(Format::Png.mime(), "image/png");
    }

    #[test]
    fn a_tile_plays_by_itself_up_to_eight_mebibytes_and_never_for_less_motion() {
        assert_eq!(TILE_PLAYS_UP_TO_BYTES, 8_388_608);
        assert!(plays_in_tile(Some(182_330), false));
        assert!(plays_in_tile(Some(8_388_608), false), "at the bound");
        assert!(!plays_in_tile(Some(8_388_609), false), "above it");
        assert!(!plays_in_tile(Some(182_330), true), "less motion");
        assert!(!plays_in_tile(None, false), "a size not known");
        assert!(!plays_in_tile(Some(-1), false));
    }

    #[test]
    fn an_animation_is_one_video_carrying_the_flag_caption_or_not() {
        let video = |animation| AttachmentFlags {
            kind: "video",
            animation,
        };
        assert!(is_animation(&[video(true)]));
        assert!(!is_animation(&[video(false)]));
        assert!(!is_animation(&[video(true), video(true)]));
        assert!(!is_animation(&[]));
        for kind in ["photo", "audio", "file", "location", "Video", ""] {
            assert!(
                !is_animation(&[AttachmentFlags {
                    kind,
                    animation: true
                }]),
                "{kind}"
            );
        }
    }
}
