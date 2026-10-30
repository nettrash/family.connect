//! What a sticker's BYTES say about themselves (`StickerFile.cs`; docs/protocol.md, "What a sticker
//! is made of"): which of the two accepted types they are, how big the picture is, and whether it
//! moves.
//!
//! `fc_text::pack` and `fc_text::animation_source` read most of it, and are called: the type
//! (`pack::type_of`, C# `Mime`), whether a type may be a sticker (`pack::is_sticker_type`), a GIF
//! (`pack::is_gif`), whether it moves (`pack::is_animated`, which for a GIF is the C#'s
//! `IsAnimatedGif`), and an animated WebP's frame durations (`animation_source::webp`). The one
//! thing neither reads is a STILL picture's pixel size, which is here.
//!
//! READ FROM THE BYTES, NEVER FROM THE NAME, and NOTHING HERE DECODES A PIXEL: the sizes are in the
//! container's own headers. Every read is bounded — these are bytes somebody picked from a disk or
//! another member uploaded, and a length field that lies must end the walk, not the process.

use fc_text::pack;

/// The picture's own pixel size — a PNG's `IHDR`, or a WebP's `VP8X` canvas, `VP8L` or `VP8 `
/// frame header, whichever comes first — or `None` when the header does not say.
pub fn size(bytes: &[u8]) -> Option<(u32, u32)> {
    match pack::type_of(bytes)? {
        pack::WEBP => webp_size(bytes),
        _ => png_size(bytes),
    }
}

fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    // Signature (8), then IHDR: length (4), "IHDR" (4), width (4), height (4), big-endian.
    if bytes.len() < 24 || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    plausible(width, height)
}

fn webp_size(bytes: &[u8]) -> Option<(u32, u32)> {
    for (name, payload) in riff_chunks(bytes) {
        match name {
            b"VP8X" => {
                // flags (1), reserved (3), canvas width-1 (3), canvas height-1 (3), little-endian.
                if payload.len() < 10 {
                    return None;
                }
                let width = u32::from_le_bytes([payload[4], payload[5], payload[6], 0]) + 1;
                let height = u32::from_le_bytes([payload[7], payload[8], payload[9], 0]) + 1;
                return plausible(width, height);
            }
            b"VP8L" => {
                // 0x2F, then width-1 and height-1 packed into 14 bits each.
                if payload.len() < 5 || payload[0] != 0x2F {
                    return None;
                }
                let bits = u32::from_le_bytes([payload[1], payload[2], payload[3], payload[4]]);
                return plausible((bits & 0x3FFF) + 1, ((bits >> 14) & 0x3FFF) + 1);
            }
            b"VP8 " => {
                // A 3-byte frame tag, the start code 9D 01 2A, then 14 bits of each dimension.
                if payload.len() < 10 || payload[3..6] != [0x9D, 0x01, 0x2A] {
                    return None;
                }
                let width = u16::from_le_bytes([payload[6], payload[7]]) & 0x3FFF;
                let height = u16::from_le_bytes([payload[8], payload[9]]) & 0x3FFF;
                return plausible(u32::from(width), u32::from(height));
            }
            _ => {}
        }
    }
    None
}

/// A size a picture can have; a zero, or one past what an `i32` carries (the C#'s `int`), is a
/// header that lies.
fn plausible(width: u32, height: u32) -> Option<(u32, u32)> {
    let fits = |side: u32| side > 0 && i32::try_from(side).is_ok();
    (fits(width) && fits(height)).then_some((width, height))
}

/// The chunks of a RIFF/WEBP file after its twelve-byte header, walked without trusting a length:
/// a chunk that claims more than the file holds is read as far as the file goes — the header fields
/// wanted are at its front — and ends the walk.
fn riff_chunks(bytes: &[u8]) -> impl Iterator<Item = (&[u8], &[u8])> {
    let mut at = Some(12usize);
    std::iter::from_fn(move || {
        let start = at?;
        let header = bytes.get(start..start.checked_add(8)?)?;
        let size = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
        let data = start + 8;
        let held = size.min(bytes.len() - data);
        // Chunks are padded to an even length.
        at = data
            .checked_add(size)
            .and_then(|end| end.checked_add(size & 1))
            .filter(|&next| next <= bytes.len());
        Some((&header[..4], &bytes[data..data + held]))
    })
}
