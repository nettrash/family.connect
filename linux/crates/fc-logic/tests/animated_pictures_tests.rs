//! `AnimatedPicturesTests`: the three animated pictures taken apart (issue #96; docs/protocol.md,
//! "Animations"): which picked files are animations, a GIF decoded and composited by this client,
//! an APNG and an animated WebP cut into standalone stills for the platform's decoder — and the
//! compositor every one of them goes through (`animation_frames::FrameCanvas`).
//!
//! The files here are written by this test's own writers (an LZW stream of literals and clears, PNG
//! chunks with their own CRC, RIFF chunks), so the production readers are never what says they read
//! correctly — and one GIF was written by Pillow, with real LZW, sub-rectangles, local colour tables
//! and transparency, and is checked against Pillow's own frames. The C# deflated the PNG data with
//! .NET's `ZLibStream`; here it is a stored-block zlib stream (`common::codecs`) — the readers copy
//! the data and never inflate it, and the test compares what it inflates.

mod common;

use fc_core::text::sticker_file;
use fc_logic::animated_pictures::{
    AnimationFormat, AnimationFrame, ApngPicture, FrameBlend, FrameDisposal, GifPicture,
    WebPAnimation, animation_prep,
};
use fc_logic::animation_frames::{FrameCanvas, WrongSize, animation_render};
use fc_text::media;

use common::codecs;

// ---- writers ----------------------------------------------------------------------------------

#[derive(Clone)]
struct GifImage {
    left: u16,
    top: u16,
    width: u16,
    height: u16,
    indexes: Vec<u8>,
    disposal: u8,
    transparent: Option<u8>,
    delay_cs: u16,
    interlaced: bool,
    stop_after: Option<usize>,
}

impl GifImage {
    fn new(left: u16, top: u16, width: u16, height: u16, indexes: Vec<u8>) -> Self {
        GifImage {
            left,
            top,
            width,
            height,
            indexes,
            disposal: 1,
            transparent: None,
            delay_cs: 10,
            interlaced: false,
            stop_after: None,
        }
    }
}

const COLOURS: [(u8, u8, u8); 8] = [
    (0, 0, 0),
    (255, 0, 0),
    (0, 255, 0),
    (0, 0, 255),
    (255, 255, 255),
    (255, 255, 0),
    (0, 255, 255),
    (255, 0, 255),
];

/// A GIF of literals: a clear code every `clear − 2` indexes, so the code size never grows — the
/// "uncompressed GIF" every decoder must read, and one whose every index this test knows.
fn gif(width: u16, height: u16, images: &[GifImage]) -> Vec<u8> {
    let mut output = b"GIF89a".to_vec();
    output.extend_from_slice(&width.to_le_bytes());
    output.extend_from_slice(&height.to_le_bytes());
    // A global table of eight colours: 2^(2+1).
    output.extend_from_slice(&[0x80 | 0x02, 0, 0]);
    for (r, g, b) in COLOURS {
        output.extend_from_slice(&[r, g, b]);
    }
    for image in images {
        output.extend_from_slice(&[
            0x21,
            0xF9,
            0x04,
            (image.disposal << 2) | u8::from(image.transparent.is_some()),
        ]);
        output.extend_from_slice(&image.delay_cs.to_le_bytes());
        output.push(image.transparent.unwrap_or(0));
        output.push(0);
        output.push(0x2C);
        for value in [image.left, image.top, image.width, image.height] {
            output.extend_from_slice(&value.to_le_bytes());
        }
        output.push(if image.interlaced { 0x40 } else { 0 });
        const MINIMUM: u32 = 3;
        output.push(MINIMUM as u8);
        let order: Vec<usize> = if image.interlaced {
            interlace_order(usize::from(image.height))
        } else {
            (0..usize::from(image.height)).collect()
        };
        let width = usize::from(image.width);
        let indexes: Vec<u8> = order
            .iter()
            .flat_map(|&row| image.indexes.iter().skip(row * width).take(width).copied())
            .take(image.stop_after.unwrap_or(usize::MAX))
            .collect();
        const CLEAR: u32 = 1 << MINIMUM;
        let mut codes = vec![CLEAR];
        for (at, &index) in indexes.iter().enumerate() {
            if at > 0 && (at as u32).is_multiple_of(CLEAR - 2) {
                codes.push(CLEAR);
            }
            codes.push(u32::from(index));
        }
        if image.stop_after.is_none() {
            codes.push(CLEAR + 1);
        }
        let mut packed = Vec::new();
        let (mut bits, mut held) = (0u32, 0u32);
        for code in codes {
            bits |= code << held;
            held += MINIMUM + 1;
            while held >= 8 {
                packed.push(bits as u8);
                bits >>= 8;
                held -= 8;
            }
        }
        if held > 0 {
            packed.push(bits as u8);
        }
        for block in packed.chunks(255) {
            output.push(block.len() as u8);
            output.extend_from_slice(block);
        }
        output.push(0);
    }
    output.push(0x3B);
    output
}

/// The order an interlaced image's rows are stored in, worked out here independently of the reader.
fn interlace_order(height: usize) -> Vec<usize> {
    let rows = 0..height;
    rows.clone()
        .filter(|row| row.is_multiple_of(8))
        .chain(rows.clone().filter(|row| row % 8 == 4))
        .chain(rows.clone().filter(|row| row % 4 == 2))
        .chain(rows.filter(|row| row % 2 == 1))
        .collect()
}

fn fill(count: usize, index: u8) -> Vec<u8> {
    vec![index; count]
}

type Rgba = (u8, u8, u8, u8);

/// A pixel of a straight-BGRA canvas, as (r, g, b, a).
fn at(canvas: &[u8], width: usize, x: usize, y: usize) -> Rgba {
    let at = (y * width + x) * 4;
    (canvas[at + 2], canvas[at + 1], canvas[at], canvas[at + 3])
}

const RED: Rgba = (255, 0, 0, 255);
const GREEN: Rgba = (0, 255, 0, 255);
const BLUE: Rgba = (0, 0, 255, 255);
const EMPTY: Rgba = (0, 0, 0, 0);

/// Every frame of a GIF composited, as the canvas stands after each.
fn composited(bytes: &[u8]) -> Vec<Vec<u8>> {
    let picture = GifPicture::read(bytes).unwrap();
    let layout = &picture.layout;
    let mut canvas = FrameCanvas::new(layout.width, layout.height).unwrap();
    layout
        .frames
        .iter()
        .enumerate()
        .map(|(index, frame)| {
            canvas.draw(frame, &picture.pixels(index).unwrap()).unwrap();
            canvas.pixels().to_vec()
        })
        .collect()
}

fn two_frames() -> Vec<u8> {
    gif(
        2,
        2,
        &[
            GifImage::new(0, 0, 2, 2, fill(4, 1)),
            GifImage::new(0, 0, 2, 2, fill(4, 2)),
        ],
    )
}

fn one_frame() -> Vec<u8> {
    gif(2, 2, &[GifImage::new(0, 0, 2, 2, fill(4, 1))])
}

// ---- which files are animations -----------------------------------------------------------------

#[test]
fn an_animation_is_what_its_own_bytes_say() {
    let two = two_frames();
    let one = one_frame();
    assert_eq!(animation_prep::animated(&two), Some(AnimationFormat::Gif));
    assert_eq!(animation_prep::animated(&one), None);
    assert_eq!(
        animation_prep::animated(&apng()),
        Some(AnimationFormat::Apng)
    );
    assert_eq!(
        animation_prep::animated(&webp()),
        Some(AnimationFormat::WebP)
    );
    assert_eq!(animation_prep::animated(&still_png()), None);
    assert_eq!(animation_prep::animated(&still_webp()), None);
    assert_eq!(animation_prep::animated(b"BM\0\0\0\0"), None);
    assert!(animation_prep::may_be(&two));
    assert!(animation_prep::may_be(&apng()));
    assert!(animation_prep::may_be(&webp()));
    assert!(!animation_prep::may_be(b"BM\0\0\0\0\0\0\0\0\0\0"));
    assert!(!animation_prep::may_be(&[0xFF, 0xD8, 0xFF, 0xE0]));
}

/// ANIMATED → AN ANIMATION, STILL → EXACTLY AS BEFORE: a still GIF or WebP is a file and a still
/// PNG a photo (the router it always went through), and an animation is one only where a chat takes
/// them and within the ceiling.
#[test]
fn an_animated_picture_is_routed_as_an_animation_and_a_still_one_as_before() {
    let animated = two_frames();
    let still = one_frame();
    let size = |bytes: &[u8]| bytes.len() as i64;
    assert_eq!(
        animation_prep::route(true, size(&animated), &animated),
        Some(AnimationFormat::Gif)
    );
    assert_eq!(
        animation_prep::route(true, size(&apng()), &apng()),
        Some(AnimationFormat::Apng)
    );
    assert_eq!(
        animation_prep::route(true, size(&webp()), &webp()),
        Some(AnimationFormat::WebP)
    );
    // The assistant's chat, the board, a profile picture: not offered.
    assert_eq!(
        animation_prep::route(false, size(&animated), &animated),
        None
    );
    // Over the ceiling, the route it always had: refused, as a file.
    assert_eq!(
        animation_prep::route(true, media::SIZE_LIMIT as i64 + 1, &animated),
        None
    );
    assert_eq!(animation_prep::route(true, size(&still), &still), None);
    assert_eq!(
        media::route("image/gif", "still.gif", &still),
        media::Route::File
    );
    assert_eq!(
        animation_prep::route(true, size(&still_webp()), &still_webp()),
        None
    );
    assert_eq!(
        media::route("image/webp", "still.webp", &still_webp()),
        media::Route::File
    );
    assert_eq!(
        animation_prep::route(true, size(&still_png()), &still_png()),
        None
    );
    assert_eq!(
        media::route("image/png", "still.png", &still_png()),
        media::Route::Photo
    );
    // A layout for each animation, none for a still.
    assert_eq!(
        animation_prep::layout(&animated).unwrap().format,
        AnimationFormat::Gif
    );
    assert_eq!(
        animation_prep::layout(&apng()).unwrap().format,
        AnimationFormat::Apng
    );
    assert_eq!(
        animation_prep::layout(&webp()).unwrap().format,
        AnimationFormat::WebP
    );
    assert_eq!(animation_prep::layout(&still), None);
}

// ---- a GIF ------------------------------------------------------------------------------------

#[test]
fn a_gifs_frames_and_delays_are_read_from_its_blocks() {
    let bytes = gif(
        4,
        3,
        &[
            GifImage {
                disposal: 0,
                delay_cs: 7,
                ..GifImage::new(0, 0, 4, 3, fill(12, 1))
            },
            GifImage {
                disposal: 2,
                transparent: Some(0),
                delay_cs: 0,
                ..GifImage::new(1, 1, 2, 1, fill(2, 2))
            },
            GifImage {
                disposal: 3,
                delay_cs: 100,
                ..GifImage::new(0, 0, 1, 1, fill(1, 3))
            },
            GifImage {
                disposal: 6,
                delay_cs: 2,
                ..GifImage::new(0, 0, 1, 1, fill(1, 3))
            },
        ],
    );
    let layout = GifPicture::read(bytes.as_slice()).unwrap().layout;
    assert_eq!((layout.width, layout.height), (4, 3));
    assert_eq!(layout.stated_ms(), vec![70, 0, 1000, 20]);
    assert_eq!(
        layout
            .frames
            .iter()
            .map(|frame| frame.disposal)
            .collect::<Vec<_>>(),
        vec![
            FrameDisposal::None,
            FrameDisposal::Background,
            FrameDisposal::Previous,
            FrameDisposal::None
        ]
    );
    assert!(
        layout
            .frames
            .iter()
            .all(|frame| frame.blend == FrameBlend::Over)
    );
    assert_eq!(
        layout.frames[1],
        AnimationFrame::new(1, 1, 2, 1, 0, FrameBlend::Over, FrameDisposal::Background)
    );
}

#[test]
fn a_transparent_index_draws_nothing_and_the_rest_over() {
    // Frame 1 is green with its two left pixels transparent (index 0, named transparent).
    let bytes = gif(
        3,
        2,
        &[
            GifImage::new(0, 0, 3, 2, fill(6, 1)),
            GifImage {
                transparent: Some(0),
                ..GifImage::new(0, 0, 3, 2, vec![0, 2, 2, 0, 2, 2])
            },
        ],
    );
    let shown = composited(&bytes);
    assert_eq!(at(&shown[1], 3, 0, 0), RED);
    assert_eq!(at(&shown[1], 3, 1, 0), GREEN);
    assert_eq!(at(&shown[1], 3, 0, 1), RED);
    assert_eq!(at(&shown[1], 3, 2, 1), GREEN);
}

#[test]
fn disposal_none_leaves_a_frame_where_it_was() {
    let bytes = gif(
        3,
        1,
        &[
            GifImage::new(0, 0, 3, 1, fill(3, 1)),
            GifImage::new(1, 0, 1, 1, vec![2]),
            GifImage::new(2, 0, 1, 1, vec![3]),
        ],
    );
    let shown = composited(&bytes);
    assert_eq!(at(&shown[2], 3, 0, 0), RED);
    assert_eq!(at(&shown[2], 3, 1, 0), GREEN);
    assert_eq!(at(&shown[2], 3, 2, 0), BLUE);
}

/// "Restore to background" clears to EMPTY — what browsers draw, whatever background colour the
/// file names.
#[test]
fn disposal_background_clears_the_frames_rectangle_to_empty() {
    let bytes = gif(
        3,
        1,
        &[
            GifImage::new(0, 0, 3, 1, fill(3, 1)),
            GifImage {
                disposal: 2,
                ..GifImage::new(1, 0, 2, 1, fill(2, 2))
            },
            GifImage::new(0, 0, 1, 1, vec![3]),
        ],
    );
    let shown = composited(&bytes);
    assert_eq!(at(&shown[1], 3, 1, 0), GREEN);
    // Frame 1's rectangle is gone once frame 2 is drawn; frame 0 outside it stays under frame 2.
    assert_eq!(at(&shown[2], 3, 0, 0), BLUE);
    assert_eq!(at(&shown[2], 3, 1, 0), EMPTY);
    assert_eq!(at(&shown[2], 3, 2, 0), EMPTY);
    // And on white, which is what the video shows.
    let white = animation_render::on_white(&shown[2]);
    assert_eq!(at(&white, 3, 1, 0), (255, 255, 255, 255));
}

#[test]
fn disposal_previous_puts_back_what_the_frame_covered() {
    let bytes = gif(
        3,
        1,
        &[
            GifImage::new(0, 0, 3, 1, fill(3, 1)),
            GifImage {
                disposal: 3,
                ..GifImage::new(0, 0, 2, 1, fill(2, 2))
            },
            GifImage::new(2, 0, 1, 1, vec![3]),
        ],
    );
    let shown = composited(&bytes);
    assert_eq!(at(&shown[1], 3, 0, 0), GREEN);
    assert_eq!(at(&shown[2], 3, 0, 0), RED);
    assert_eq!(at(&shown[2], 3, 1, 0), RED);
    assert_eq!(at(&shown[2], 3, 2, 0), BLUE);
}

#[test]
fn an_interlaced_image_is_put_back_in_row_order() {
    let rows: Vec<u8> = (0..10).map(|row| (row % 8) as u8).collect();
    let bytes = gif(
        1,
        10,
        &[
            GifImage {
                interlaced: true,
                ..GifImage::new(0, 0, 1, 10, rows)
            },
            GifImage::new(0, 0, 1, 1, vec![0]),
        ],
    );
    let pixels = GifPicture::read(bytes.as_slice())
        .unwrap()
        .pixels(0)
        .unwrap();
    for row in 0..10 {
        let (r, g, b) = COLOURS[row % 8];
        assert_eq!(at(&pixels, 1, 0, row), (r, g, b, 255));
    }
    assert_eq!(
        (0..10)
            .map(|written| GifPicture::interlaced_row(written, 10))
            .collect::<Vec<_>>(),
        vec![0, 8, 4, 2, 6, 1, 3, 5, 7, 9]
    );
}

/// Data that stops early leaves the rest of the rectangle undrawn, as a browser leaves it.
#[test]
fn an_image_whose_data_stops_early_leaves_the_rest_undrawn() {
    let bytes = gif(
        4,
        1,
        &[
            GifImage::new(0, 0, 4, 1, fill(4, 1)),
            // A clear and three indexes are sixteen bits: the data ends on a byte, with no padding
            // to read as a code.
            GifImage {
                stop_after: Some(3),
                ..GifImage::new(0, 0, 4, 1, fill(4, 2))
            },
        ],
    );
    let shown = composited(&bytes);
    assert_eq!(at(&shown[1], 4, 2, 0), GREEN);
    assert_eq!(at(&shown[1], 4, 3, 0), RED);
}

/// A frame that reaches past the logical screen is clipped to it, never a crash.
#[test]
fn a_frame_past_the_canvas_is_clipped() {
    let bytes = gif(
        2,
        2,
        &[
            GifImage::new(0, 0, 2, 2, fill(4, 1)),
            GifImage::new(1, 1, 3, 3, fill(9, 2)),
        ],
    );
    let shown = composited(&bytes);
    assert_eq!(at(&shown[1], 2, 1, 1), GREEN);
    assert_eq!(at(&shown[1], 2, 0, 0), RED);
}

#[test]
fn bytes_that_are_not_a_gif_are_no_picture() {
    assert!(GifPicture::read(b"GIF89a".as_slice()).is_none());
    assert!(GifPicture::read(apng()).is_none());
    // A header and nothing after it: no image.
    assert!(GifPicture::read(gif(2, 2, &[])).is_none());
    // Cut off in the middle of a block: what was read before it stands.
    let whole = two_frames();
    assert_eq!(
        GifPicture::read(&whole[..whole.len() - 20])
            .unwrap()
            .layout
            .frames
            .len(),
        1
    );
}

/// A GIF Pillow wrote — real LZW at an 8-bit code size, frames 1 and 2 as sub-rectangles with local
/// colour tables and a transparent index for every pixel that did not change — composited exactly
/// into Pillow's own three frames (SHA-256 of each composited canvas as straight BGRA, from
/// `Image.convert("RGB")` of the frames Pillow was given).
#[test]
fn a_gif_pillow_wrote_is_composited_into_pillows_own_frames() {
    let bytes = codecs::base64(PILLOW_GIF);
    let layout = GifPicture::read(bytes.as_slice()).unwrap().layout;
    assert_eq!((layout.width, layout.height), (32, 24));
    assert_eq!(layout.stated_ms(), vec![70, 20, 1000]);
    assert_eq!(
        layout.frames[1],
        AnimationFrame::new(9, 5, 6, 7, 20, FrameBlend::Over, FrameDisposal::None)
    );
    let shown = composited(&bytes);
    assert_eq!(
        shown
            .iter()
            .map(|canvas| codecs::sha256_hex(canvas))
            .collect::<Vec<_>>(),
        vec![
            "cab041f8fa5a527187a4c1406fb0c6e00881462d6b86961a318ae60c6e3f288a",
            "9795d1a04e272008c8b850793e00c572f3752185de731fb3f55a861b5b156171",
            "589a3295fdb8b1b9b480cdd24c3628e6d8538e5addb8c3373770a20d6f0e7e24",
        ]
    );
}

const PILLOW_GIF: &str = concat!(
    "R0lGODlhIAAYAIMAAL2hymkxrhcLVdChLIKtto1DBfT346J8saUOVB8/4Gy+IkQ6moSBY93RGI222dAvZyH/C05F",
    "VFNDQVBFMi4wAwEAAAAh+QQEBwAAACwAAAAAIAAYAAAI/wAdKDgQIAGBAwUSNADwgEABAQAcIABQIMCChwgMOAQw",
    "wICCBAI+CkDoIEEBBQQCDHigoEFKAwAaNBCQ4AEAAAIONkBAIIFFAwYQLGDAwAGBAQkUOMj5oMGAAg8QhnygkIAB",
    "Bw97Ijig8SCCBgaLJhjA4EDMAweGBgCAoACDtg8MCFhg1QFLBQwAGDig8AFLBA4OKKDa8QCDkQQUCCgg8+HctgUM",
    "UEWQYOiCAwQuB9g7YEDPBwEIAFjgs8ACnoAHBGCwM4CDAQuiFoDaNgDgAq+F5u3Y1MGCpw9gJxCMEwFZwwhca2yg",
    "wLNTA2QfzKW5QICAtgsWoFSAQMFVBgYsjv8F/vppgAA0CQowYLCAzpYMOq683KAgUQREBbj+WDAy1wIDHNAAV8wd",
    "4AAAZkGEQHIEIEATe84VcJMDDCTgQH2HrfUAA7aJllNZmSWm1FU7AdjAQwqcZlcDG7Z0gE0WLSQXVgUdMFFR9rE0",
    "l0FRGVDURQE84ABQJzGgXWAHdDZkYgyAptqGrAnQkIgGAIgUaSlKOEBISS3VAF0LBGCaaVsSsGF2VD2wAIY5ZXZg",
    "QV+6d1FnYK0UH4tfdbbAazVhGJmDswUAlgIFbNUAUWBZJ2ZiQgJQ1E0H0YTVaZQJYOSCPw4wEGvnNXAggowVltRA",
    "3HW3EGZoRaYAQ3zNZeSBgMVMthIB6wGQgE8hnXZiRg0EVRYDD4G5npEWRjWaUh/N1hZH3IGXU0QUUTXYYrAFdpNq",
    "twaAF2xCJhDTaAMY1ytUBEhEFa12pVQWXQMEBAAh+QQFAgAQACwJAAUABgAHAIO9ocppMa4XC1XQoSyCrbaNQwX0",
    "9+OifLGlDlQfP+BsviJEOpqEgWPd0RiNttnQL2cILwANHCjwwACCBA8cCBAAYMFBAAkcBAAAIICCAgUcElDQoIAC",
    "BQkOLBhAAOOBAAEBACH5BAVkABAALAkABQAMAA4Ag72hymkxrhcLVdChLIKtto1DBfT346J8saUOVB8/4Gy+IkQ6",
    "moSBY93RGI222dAvZwigAAUoeBBgQYEGBQMAIIDgQAMHAwQYQAgAAgMCBh4YVIAAggMBEAgsYAABQoIEBQIIMLjg",
    "AIICChYAMLCgAYEEDBIccMngQQEGAgI8YOAAwACUR1saUBBgJ4MBAwIUKDCgAQIGCBkcAJBggcwGAwAgQECAgE8D",
    "BwwwdNBSAdChCSAEwHiywAEIX7s2SNs0gVoFK3d6TVlgwQMHDhIocGAgIAA7",
);

// ---- an APNG ----------------------------------------------------------------------------------

fn crc(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &value in bytes {
        crc ^= u32::from(value);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn chunk(kind: &str, payload: &[u8]) -> Vec<u8> {
    let mut chunk = (payload.len() as u32).to_be_bytes().to_vec();
    chunk.extend_from_slice(kind.as_bytes());
    chunk.extend_from_slice(payload);
    let check = crc(&chunk[4..]);
    chunk.extend_from_slice(&check.to_be_bytes());
    chunk
}

const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

fn ihdr(width: u32, height: u32) -> Vec<u8> {
    let mut ihdr = [0u8; 13];
    ihdr[..4].copy_from_slice(&width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&height.to_be_bytes());
    ihdr[8] = 8;
    ihdr[9] = 6;
    chunk("IHDR", &ihdr)
}

/// RGBA rows, each behind a filter byte of 0, deflated as PNG's image data is.
fn image_data(width: usize, height: usize, seed: u8) -> Vec<u8> {
    let row = 1 + width * 4;
    let raw: Vec<u8> = (0..height * row)
        .map(|at| {
            if at.is_multiple_of(row) {
                0
            } else {
                seed.wrapping_add(at as u8)
            }
        })
        .collect();
    codecs::zlib(&raw)
}

fn fctl(
    sequence: u32,
    width: u32,
    height: u32,
    x: u32,
    y: u32,
    num: u16,
    den: u16,
    dispose: u8,
    blend: u8,
) -> Vec<u8> {
    let mut fctl = Vec::with_capacity(26);
    for value in [sequence, width, height, x, y] {
        fctl.extend_from_slice(&value.to_be_bytes());
    }
    fctl.extend_from_slice(&num.to_be_bytes());
    fctl.extend_from_slice(&den.to_be_bytes());
    fctl.push(dispose);
    fctl.push(blend);
    chunk("fcTL", &fctl)
}

fn fdat(sequence: u32, data: &[u8]) -> Vec<u8> {
    let mut fdat = sequence.to_be_bytes().to_vec();
    fdat.extend_from_slice(data);
    chunk("fdAT", &fdat)
}

fn actl(frames: u32) -> Vec<u8> {
    let mut actl = frames.to_be_bytes().to_vec();
    actl.extend_from_slice(&[0; 4]);
    chunk("acTL", &actl)
}

fn concat(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

/// A 4 × 3 APNG: frame 0 is the default image (its fcTL before the IDAT), frame 1 a 2 × 2
/// rectangle at (1, 1) in two fdATs, frame 2 a 1 × 1 at (3, 2). A gAMA between the header and the
/// image data is every frame's.
fn apng() -> Vec<u8> {
    let second = image_data(2, 2, 2);
    concat(&[
        &PNG_SIGNATURE,
        &ihdr(4, 3),
        &actl(3),
        &chunk("gAMA", &[0, 0, 0xB1, 0x8F]),
        &fctl(0, 4, 3, 0, 0, 1, 10, 2, 0),
        &chunk("IDAT", &image_data(4, 3, 1)),
        &fctl(1, 2, 2, 1, 1, 1, 3, 2, 1),
        &fdat(2, &second[..5]),
        &fdat(3, &second[5..]),
        &fctl(4, 1, 1, 3, 2, 0, 0, 1, 0),
        &fdat(5, &image_data(1, 1, 3)),
        &chunk("tEXt", b"Comment\0after"),
        &chunk("IEND", &[]),
    ])
}

fn still_png() -> Vec<u8> {
    concat(&[
        &PNG_SIGNATURE,
        &ihdr(2, 2),
        &chunk("IDAT", &image_data(2, 2, 9)),
        &chunk("IEND", &[]),
    ])
}

/// The chunks of a PNG by this test's own walk: type, payload, and whether its CRC is right.
fn chunks(png: &[u8]) -> Vec<(String, Vec<u8>, bool)> {
    assert_eq!(png[..8], PNG_SIGNATURE);
    let mut chunks = Vec::new();
    let mut at = 8;
    while at < png.len() {
        let length = u32::from_be_bytes([png[at], png[at + 1], png[at + 2], png[at + 3]]) as usize;
        let check = u32::from_be_bytes([
            png[at + 8 + length],
            png[at + 9 + length],
            png[at + 10 + length],
            png[at + 11 + length],
        ]);
        chunks.push((
            String::from_utf8_lossy(&png[at + 4..at + 8]).into_owned(),
            png[at + 8..at + 8 + length].to_vec(),
            check == crc(&png[at + 4..at + 8 + length]),
        ));
        at += 12 + length;
    }
    chunks
}

#[test]
fn an_apngs_frames_are_its_fctls() {
    let layout = ApngPicture::read(apng()).unwrap().layout;
    assert_eq!((layout.width, layout.height), (4, 3));
    assert_eq!(
        layout.frames,
        vec![
            // A "previous" disposal on the FIRST frame is a background one (the APNG
            // specification's words).
            AnimationFrame::new(
                0,
                0,
                4,
                3,
                100,
                FrameBlend::Source,
                FrameDisposal::Background
            ),
            AnimationFrame::new(1, 1, 2, 2, 333, FrameBlend::Over, FrameDisposal::Previous),
            // 0/0: a den of 0 is hundredths, so 0 ms — which the timing then shows for 100.
            AnimationFrame::new(3, 2, 1, 1, 0, FrameBlend::Source, FrameDisposal::Background),
        ]
    );
}

/// Each frame rebuilt as a PNG of its own that any decoder reads: the signature, the header at the
/// FRAME's size with a right CRC, every chunk that came before the image data (but never the
/// animation's own), the frame's data — the fdATs' joined, their sequence numbers gone — as one
/// IDAT, and the end.
#[test]
fn each_apng_frame_becomes_a_standalone_png() {
    let picture = ApngPicture::read(apng()).unwrap();
    for index in 0..3 {
        let chunks = chunks(&picture.standalone(index).unwrap());
        for (kind, _, sound) in &chunks {
            assert!(*sound, "frame {index}: {kind}'s CRC");
        }
        assert_eq!(
            chunks
                .iter()
                .map(|chunk| chunk.0.as_str())
                .collect::<Vec<_>>(),
            vec!["IHDR", "gAMA", "IDAT", "IEND"]
        );
        let frame = picture.layout.frames[index];
        let header = &chunks[0].1;
        assert_eq!(
            u32::from_be_bytes([header[0], header[1], header[2], header[3]]),
            frame.width as u32
        );
        assert_eq!(
            u32::from_be_bytes([header[4], header[5], header[6], header[7]]),
            frame.height as u32
        );
        // Bit depth, colour type and the rest are the file's own.
        assert_eq!(header[8..], [8, 6, 0, 0, 0]);
        assert_eq!(
            codecs::inflate(&image_data(
                frame.width as usize,
                frame.height as usize,
                (index + 1) as u8
            )),
            codecs::inflate(&chunks[2].1)
        );
    }
}

/// A default image with no fcTL before it is not part of the animation (the specification's
/// "hidden" image).
#[test]
fn a_hidden_default_image_is_no_frame() {
    let hidden = concat(&[
        &PNG_SIGNATURE,
        &ihdr(2, 2),
        &actl(1),
        &chunk("IDAT", &image_data(2, 2, 9)),
        &fctl(0, 2, 2, 0, 0, 1, 10, 0, 0),
        &fdat(1, &image_data(2, 2, 4)),
        &chunk("IEND", &[]),
    ]);
    let picture = ApngPicture::read(hidden).unwrap();
    assert_eq!(picture.layout.frames.len(), 1);
    assert_eq!(
        codecs::inflate(&image_data(2, 2, 4)),
        codecs::inflate(&chunks(&picture.standalone(0).unwrap())[1].1)
    );
}

#[test]
fn an_apng_this_cannot_take_apart_is_none() {
    // A frame reaching past the canvas.
    assert!(
        ApngPicture::read(concat(&[
            &PNG_SIGNATURE,
            &ihdr(2, 2),
            &actl(1),
            &fctl(0, 2, 2, 1, 0, 1, 10, 0, 0),
            &chunk("IDAT", &image_data(2, 2, 9)),
            &chunk("IEND", &[]),
        ]))
        .is_none()
    );
    // A frame with no data.
    assert!(
        ApngPicture::read(concat(&[
            &PNG_SIGNATURE,
            &ihdr(2, 2),
            &actl(2),
            &fctl(0, 2, 2, 0, 0, 1, 10, 0, 0),
            &chunk("IDAT", &image_data(2, 2, 9)),
            &fctl(1, 2, 2, 0, 0, 1, 10, 0, 0),
            &chunk("IEND", &[]),
        ]))
        .is_none()
    );
    // A length that runs past the end.
    let whole = apng();
    assert!(ApngPicture::read(&whole[..whole.len() - 40]).is_none());
    assert!(ApngPicture::read(still_png()).is_none());
}

// ---- an animated WebP -------------------------------------------------------------------------

fn riff(kind: &str, payload: &[u8]) -> Vec<u8> {
    let mut chunk = kind.as_bytes().to_vec();
    chunk.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    chunk.extend_from_slice(payload);
    if payload.len() & 1 != 0 {
        chunk.push(0);
    }
    chunk
}

fn u24(value: u32) -> Vec<u8> {
    vec![value as u8, (value >> 8) as u8, (value >> 16) as u8]
}

fn anmf(
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    duration: u32,
    flags: u8,
    data: &[&[u8]],
) -> Vec<u8> {
    riff(
        "ANMF",
        &concat(&[
            &u24(x / 2),
            &u24(y / 2),
            &u24(width - 1),
            &u24(height - 1),
            &u24(duration),
            &[flags],
            &data.concat(),
        ]),
    )
}

fn webp_file(chunks: &[&[u8]]) -> Vec<u8> {
    let body = concat(&[b"WEBP", &chunks.concat()]);
    let mut file = b"RIFF".to_vec();
    file.extend_from_slice(&(body.len() as u32).to_le_bytes());
    file.extend_from_slice(&body);
    file
}

fn lossless() -> Vec<u8> {
    riff("VP8L", &[0x2F, 1, 2, 3, 4, 5, 6])
}

fn alpha() -> Vec<u8> {
    riff("ALPH", &[0, 9, 9])
}

fn lossy() -> Vec<u8> {
    riff("VP8 ", &[7, 7, 7, 0x9D, 0x01, 0x2A, 1, 0, 1, 0])
}

/// An 8 × 6 animated WebP: a whole-canvas lossless frame, a lossy frame with alpha at (2, 4) that
/// is not blended and is disposed to background, and a lossy frame with no alpha.
fn webp() -> Vec<u8> {
    webp_file(&[
        &riff("VP8X", &concat(&[&[0x12, 0, 0, 0], &u24(7), &u24(5)])),
        &riff("ANIM", &[255, 255, 255, 255, 0, 0]),
        &anmf(0, 0, 8, 6, 80, 0x00, &[&lossless()]),
        &anmf(2, 4, 3, 2, 0, 0x03, &[&alpha(), &lossy()]),
        &anmf(4, 0, 4, 6, 16_777_215, 0x00, &[&lossy()]),
    ])
}

fn still_webp() -> Vec<u8> {
    webp_file(&[&lossless()])
}

#[test]
fn an_animated_webps_frames_are_its_anmfs() {
    let layout = WebPAnimation::read(webp()).unwrap().layout;
    assert_eq!((layout.width, layout.height), (8, 6));
    assert_eq!(
        layout.frames,
        vec![
            AnimationFrame::new(0, 0, 8, 6, 80, FrameBlend::Over, FrameDisposal::None),
            AnimationFrame::new(2, 4, 3, 2, 0, FrameBlend::Source, FrameDisposal::Background),
            AnimationFrame::new(
                4,
                0,
                4,
                6,
                16_777_215,
                FrameBlend::Over,
                FrameDisposal::None
            ),
        ]
    );
    assert!(WebPAnimation::read(still_webp()).is_none());
    assert!(WebPAnimation::read(apng()).is_none());
}

/// Each frame rewrapped as a still WebP: a lossless one as a simple file, a lossy one with alpha as
/// an extended file whose VP8X says alpha at the frame's size, a lossy one without alpha as a
/// simple file — each RIFF the size it says.
#[test]
fn each_webp_frame_becomes_a_standalone_still() {
    let animation = WebPAnimation::read(webp()).unwrap();
    let simple = animation.standalone(0).unwrap();
    assert_eq!(simple, webp_file(&[&lossless()]));
    let with_alpha = animation.standalone(1).unwrap();
    assert_eq!(
        with_alpha,
        webp_file(&[
            &riff("VP8X", &concat(&[&[0x10, 0, 0, 0], &u24(2), &u24(1)])),
            &alpha(),
            &lossy(),
        ])
    );
    assert_eq!(sticker_file::size(&with_alpha), Some((3, 2)));
    assert!(!fc_text::pack::is_animated(&with_alpha));
    assert_eq!(animation.standalone(2).unwrap(), webp_file(&[&lossy()]));
    for file in [&simple, &with_alpha] {
        assert_eq!(
            file.len() - 8,
            u32::from_le_bytes([file[4], file[5], file[6], file[7]]) as usize
        );
    }
}

#[test]
fn a_webp_frame_outside_its_canvas_is_none() {
    assert!(
        WebPAnimation::read(webp_file(&[
            &riff("VP8X", &concat(&[&[0x02, 0, 0, 0], &u24(3), &u24(3)])),
            &anmf(2, 2, 4, 4, 100, 0, &[&lossless()]),
        ]))
        .is_none()
    );
}

// ---- the compositor ---------------------------------------------------------------------------

/// OVER in straight alpha, rounded to the nearest: half-transparent red over blue, and over
/// nothing.
#[test]
fn over_blends_as_porter_duff_says() {
    let mut blue = [255, 0, 0, 255];
    FrameCanvas::over(&[0, 0, 255, 128], &mut blue);
    // r = 255·128/255 ≈ 128, b = 255·127/255 = 127, opaque.
    assert_eq!(blue, [127, 0, 128, 255]);
    let mut nothing = [0, 0, 0, 0];
    FrameCanvas::over(&[10, 20, 30, 128], &mut nothing);
    assert_eq!(nothing, [10, 20, 30, 128]);
    let mut kept = [1, 2, 3, 4];
    FrameCanvas::over(&[9, 9, 9, 0], &mut kept);
    assert_eq!(kept, [1, 2, 3, 4]);
    // Two half-covers make three quarters.
    let mut half = [0, 0, 255, 128];
    FrameCanvas::over(&[0, 255, 0, 128], &mut half);
    assert_eq!(half[3], 192);
}

/// SOURCE replaces, transparency and all: a WebP frame marked "do not blend" punches a hole.
#[test]
fn source_replaces_even_with_transparency() {
    let mut canvas = FrameCanvas::new(2, 1).unwrap();
    let whole = AnimationFrame::new(0, 0, 2, 1, 100, FrameBlend::Source, FrameDisposal::None);
    canvas
        .draw(&whole, &[0, 0, 255, 255, 0, 0, 255, 255])
        .unwrap();
    canvas
        .draw(
            &AnimationFrame::new(1, 0, 1, 1, 100, FrameBlend::Source, FrameDisposal::None),
            &[0, 255, 0, 0],
        )
        .unwrap();
    assert_eq!(canvas.pixels(), [0, 0, 255, 255, 0, 255, 0, 0]);
    assert_eq!(canvas.draw(&whole, &[0, 0, 0, 0]), Err(WrongSize));
}

/// A "previous" disposal after a "previous" one restores each to what was under IT, in turn.
#[test]
fn previous_disposals_in_a_row_each_restore_their_own() {
    let mut canvas = FrameCanvas::new(1, 1).unwrap();
    let (red, green, blue) = ([0, 0, 255, 255], [0, 255, 0, 255], [255, 0, 0, 255]);
    let frame = |blend, disposal| AnimationFrame::new(0, 0, 1, 1, 100, blend, disposal);
    canvas
        .draw(&frame(FrameBlend::Source, FrameDisposal::None), &red)
        .unwrap();
    canvas
        .draw(&frame(FrameBlend::Source, FrameDisposal::Previous), &green)
        .unwrap();
    assert_eq!(canvas.pixels(), green);
    canvas
        .draw(
            &frame(FrameBlend::Over, FrameDisposal::Previous),
            &[0, 0, 0, 0],
        )
        .unwrap();
    // Green was put back to red before this empty frame drew nothing over it.
    assert_eq!(canvas.pixels(), red);
    canvas
        .draw(&frame(FrameBlend::Over, FrameDisposal::None), &[0, 0, 0, 0])
        .unwrap();
    assert_eq!(canvas.pixels(), red);
    canvas
        .draw(&frame(FrameBlend::Over, FrameDisposal::None), &blue)
        .unwrap();
    assert_eq!(canvas.pixels(), blue);
}

/// A codec's whole, already composited frames go through the same compositor as replacements.
#[test]
fn a_whole_frame_layout_replaces_the_canvas_each_time() {
    let whole = WebPAnimation::read(webp()).unwrap().layout.whole();
    for frame in &whole.frames {
        assert_eq!((frame.x, frame.y, frame.width, frame.height), (0, 0, 8, 6));
        assert_eq!(frame.blend, FrameBlend::Source);
        assert_eq!(frame.disposal, FrameDisposal::None);
    }
    assert_eq!(whole.stated_ms(), vec![80, 0, 16_777_215]);
}

// ---- the port's own ---------------------------------------------------------------------------

/// The codecs this file takes from .NET, held to their published vectors.
#[test]
fn the_test_codecs_answer_their_published_vectors() {
    assert_eq!(codecs::base64("aGVsbG8="), b"hello");
    assert_eq!(codecs::base64("aGVsbG8hIQ=="), b"hello!!");
    assert_eq!(codecs::base64("YWJj"), b"abc");
    assert_eq!(
        codecs::sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        codecs::sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        codecs::sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
    assert_eq!(codecs::adler32(b"Wikipedia"), 0x11E6_0398);
    let data: Vec<u8> = (0..200_000u32).map(|at| at as u8).collect();
    assert_eq!(codecs::inflate(&codecs::zlib(&data)), data);
    assert!(codecs::inflate(&codecs::zlib(&[])).is_empty());
}

/// Where the C#'s `int` products wrapped — a GIF image of 65 535 × 65 535 makes a negative array
/// length — the picture is refused here, never allocated: a failed allocation would end the
/// process.
#[test]
fn a_rectangle_too_large_to_count_is_refused_not_allocated() {
    let huge = gif(
        2,
        2,
        &[
            GifImage::new(0, 0, 65_535, 65_535, vec![]),
            GifImage::new(0, 0, 2, 2, fill(4, 1)),
        ],
    );
    let picture = GifPicture::read(huge.as_slice()).unwrap();
    assert_eq!(picture.pixels(0), None);
    assert_eq!(picture.pixels(1).map(|pixels| pixels.len()), Some(16));
    assert_eq!(picture.pixels(2), None);
    assert!(FrameCanvas::new(65_535, 65_535).is_none());
    assert!(FrameCanvas::new(0, 4).is_none());
}

/// An APNG canvas past 2³¹ − 1 — which the PNG specification forbids, and whose frames the C#'s
/// `(int)` casts wrapped — is not a picture this reads.
#[test]
fn an_apng_canvas_past_what_png_allows_is_none() {
    let mut header = [0u8; 13];
    header[..4].copy_from_slice(&0x8000_0000u32.to_be_bytes());
    header[4..8].copy_from_slice(&2u32.to_be_bytes());
    header[8] = 8;
    header[9] = 6;
    assert!(
        ApngPicture::read(concat(&[
            &PNG_SIGNATURE,
            &chunk("IHDR", &header),
            &actl(1),
            &fctl(0, 2, 2, 0x7FFF_FFFF, 0, 1, 10, 0, 0),
            &chunk("IDAT", &image_data(2, 2, 9)),
            &chunk("IEND", &[]),
        ]))
        .is_none()
    );
}
