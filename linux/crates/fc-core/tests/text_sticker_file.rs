//! `StickerFileTests`: what a sticker's bytes say about themselves (docs/protocol.md, "What a
//! sticker is made of"): the two accepted types by the server's own magic numbers, the pixel size,
//! and whether it moves — all from the container's headers, with no codec in sight.
//!
//! The C# `StickerFile` members and what answers them here: `Mime` → `fc_text::pack::type_of`,
//! `IsStickerType` → `pack::is_sticker_type`, `IsGif` → `pack::is_gif`, `IsAnimated` (WebP, PNG)
//! → `pack::is_animated`, `IsAnimatedGif` → `pack::is_gif && pack::is_animated`, `FrameDurations`
//! → `fc_text::animation_source::webp`'s stated frame lengths, `Size` →
//! `fc_core::text::sticker_file::size`.

use fc_core::text::sticker_file::size;
use fc_text::pack;

fn mime(bytes: &[u8]) -> Option<&'static str> {
    pack::type_of(bytes)
}

fn is_animated_gif(bytes: &[u8]) -> bool {
    pack::is_gif(bytes) && pack::is_animated(bytes)
}

fn frame_durations(bytes: &[u8]) -> Vec<u32> {
    fc_text::animation_source::webp(bytes)
        .map(|statement| statement.stated_ms)
        .unwrap_or_default()
}

// ---- bytes built the way the formats say ----------------------------------------------------

fn riff(chunks: &[Vec<u8>]) -> Vec<u8> {
    let body: Vec<u8> = chunks.concat();
    let mut file = b"RIFF".to_vec();
    file.extend_from_slice(&(4 + body.len() as u32).to_le_bytes());
    file.extend_from_slice(b"WEBP");
    file.extend_from_slice(&body);
    file
}

fn chunk(name: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    // Padded to an even length, as RIFF pads.
    let mut chunk = name.to_vec();
    chunk.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    chunk.extend_from_slice(payload);
    if payload.len() % 2 == 1 {
        chunk.push(0);
    }
    chunk
}

fn write24(at: &mut [u8], value: u32) {
    at[..3].copy_from_slice(&value.to_le_bytes()[..3]);
}

fn vp8x(width: u32, height: u32, animated: bool) -> Vec<u8> {
    let mut payload = [0u8; 10];
    payload[0] = 0x10 | if animated { 0x02 } else { 0x00 };
    write24(&mut payload[4..], width - 1);
    write24(&mut payload[7..], height - 1);
    chunk(b"VP8X", &payload)
}

fn anmf(duration_ms: u32, frame_bytes: usize) -> Vec<u8> {
    let mut payload = vec![0u8; 16 + frame_bytes];
    write24(&mut payload[12..], duration_ms);
    chunk(b"ANMF", &payload)
}

fn lossy_frame(width: u16, height: u16) -> Vec<u8> {
    let mut payload = [0u8; 10];
    payload[3..6].copy_from_slice(&[0x9D, 0x01, 0x2A]);
    payload[6..8].copy_from_slice(&width.to_le_bytes());
    payload[8..10].copy_from_slice(&height.to_le_bytes());
    chunk(b"VP8 ", &payload)
}

fn lossless_frame(width: u32, height: u32) -> Vec<u8> {
    let mut payload = vec![0x2F];
    payload.extend_from_slice(&((width - 1) | ((height - 1) << 14)).to_le_bytes());
    chunk(b"VP8L", &payload)
}

fn png(width: u32, height: u32, chunks_after_header: &[&[u8; 4]]) -> Vec<u8> {
    let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut header = [0u8; 13];
    header[..4].copy_from_slice(&width.to_be_bytes());
    header[4..8].copy_from_slice(&height.to_be_bytes());
    bytes.extend(png_chunk(b"IHDR", &header));
    for name in chunks_after_header {
        bytes.extend(png_chunk(name, &[0u8; 8]));
    }
    bytes.extend(png_chunk(b"IEND", &[]));
    bytes
}

fn png_chunk(name: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut chunk = (data.len() as u32).to_be_bytes().to_vec();
    chunk.extend_from_slice(name);
    chunk.extend_from_slice(data);
    chunk.extend_from_slice(&[0u8; 4]);
    chunk
}

// ---- the type -------------------------------------------------------------------------------

/// The server's own check: `RIFF` at 0 and `WEBP` at 8, and the four bytes between are the file's
/// length and are NOT checked.
#[test]
fn a_web_p_is_known_by_riff_at_zero_and_webp_at_eight() {
    assert_eq!(mime(&riff(&[lossy_frame(512, 512)])), Some(pack::WEBP));
    let any_length = [b"RIFF".as_slice(), &[0xFF; 4], b"WEBP"].concat();
    assert_eq!(mime(&any_length), Some(pack::WEBP));
    // A RIFF that is something else — a WAV is one — is not a sticker.
    let wave = [b"RIFF".as_slice(), &[0; 4], b"WAVE"].concat();
    assert_eq!(mime(&wave), None);
}

#[test]
fn a_png_is_known_by_its_eight_bytes_and_nothing_else_is_a_sticker() {
    assert_eq!(mime(&png(512, 512, &[])), Some(pack::PNG));
    // A JPEG named .png is a JPEG: read from the bytes, never from the name.
    assert_eq!(
        mime(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0, 0, 0, 0, 0, 0, 0]),
        None
    );
    assert_eq!(mime(b"GIF89a"), None);
    assert_eq!(mime(&[]), None);
}

#[test]
fn only_web_p_and_png_are_sticker_types() {
    for (given, sticker) in [
        ("image/webp", true),
        ("image/png", true),
        ("IMAGE/WEBP; q=1", true),
        ("image/jpeg", false),
        ("image/gif", false),
        // The C#'s `null` is the empty type here.
        ("", false),
    ] {
        assert_eq!(pack::is_sticker_type(given), sticker, "{given:?}");
    }
}

// ---- the size -------------------------------------------------------------------------------

#[test]
fn the_pixel_size_is_read_from_each_of_the_four_headers() {
    assert_eq!(size(&png(512, 384, &[])), Some((512, 384)));
    assert_eq!(size(&riff(&[lossy_frame(512, 384)])), Some((512, 384)));
    assert_eq!(size(&riff(&[lossless_frame(512, 384)])), Some((512, 384)));
    // The extended header's CANVAS, which is the picture — not whatever the first frame covers.
    assert_eq!(
        size(&riff(&[vp8x(512, 384, true), anmf(40, 3), anmf(40, 3)])),
        Some((512, 384))
    );
    assert_eq!(size(&riff(&[lossless_frame(16383, 1)])), Some((16383, 1)));
}

#[test]
fn a_size_the_header_does_not_say_is_not_guessed() {
    assert_eq!(size(&[]), None);
    assert_eq!(size(&riff(&[])), None);
    assert_eq!(size(&riff(&[chunk(b"VP8 ", &[0u8; 10])])), None);
    assert_eq!(size(&riff(&[chunk(b"VP8L", &[0x00, 1, 2, 3, 4])])), None);
    // A PNG cut off before its header finished.
    assert_eq!(size(&png(512, 512, &[])[..20]), None);
    assert_eq!(size(&png(0, 512, &[])), None);
}

// ---- whether it moves ------------------------------------------------------------------------

#[test]
fn an_animated_web_p_says_so_in_its_extended_header() {
    assert!(pack::is_animated(&riff(&[
        vp8x(512, 512, true),
        anmf(40, 3),
        anmf(40, 3)
    ])));
    assert!(!pack::is_animated(&riff(&[
        vp8x(512, 512, false),
        lossy_frame(512, 512)
    ])));
    // A simple file is one frame by construction.
    assert!(!pack::is_animated(&riff(&[lossy_frame(512, 512)])));
    assert!(!pack::is_animated(&riff(&[lossless_frame(512, 512)])));
}

/// APNG: an animation control chunk BEFORE the image data. A platform decoder will draw its first
/// frame, and this client must still never re-encode it.
#[test]
fn an_animated_png_is_one_with_an_animation_control_chunk_before_its_data() {
    assert!(pack::is_animated(&png(512, 512, &[b"acTL", b"IDAT"])));
    assert!(!pack::is_animated(&png(512, 512, &[b"IDAT"])));
    // After the data it is not the chunk the format means.
    assert!(!pack::is_animated(&png(512, 512, &[b"IDAT", b"acTL"])));
    assert!(!pack::is_animated(&[]));
}

#[test]
fn frame_durations_are_read_in_file_order_and_unclamped() {
    let file = riff(&[
        vp8x(512, 512, true),
        anmf(40, 3),
        anmf(0, 3),
        anmf(1000, 5),
        anmf(70_000, 3),
    ]);

    assert_eq!(frame_durations(&file), [40, 0, 1000, 70_000]);
    assert!(frame_durations(&riff(&[lossy_frame(512, 512)])).is_empty());
    assert!(frame_durations(&png(512, 512, &[b"acTL", b"IDAT"])).is_empty());
}

// ---- bytes that lie ---------------------------------------------------------------------------

/// These are bytes somebody picked from a disk or another member uploaded: a length field that
/// lies must end the walk, not the process.
#[test]
fn a_length_that_lies_ends_the_walk_and_nothing_else() {
    // A chunk claiming four gigabytes, with the header fields still at its front.
    let mut liar = riff(&[vp8x(512, 384, true), anmf(40, 3)]);
    liar[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(size(&liar), Some((512, 384)));
    assert!(pack::is_animated(&liar));
    assert!(frame_durations(&liar).is_empty());

    // A PNG chunk whose length runs off the end.
    let mut lying_png = png(512, 512, &[b"tEXt", b"acTL", b"IDAT"]);
    lying_png[33..37].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(!pack::is_animated(&lying_png));

    // And every prefix of a good file is something this can be asked about.
    let good = riff(&[vp8x(512, 384, true), anmf(40, 3), anmf(60, 3)]);
    for length in 0..=good.len() {
        let cut = &good[..length];
        let _ = mime(cut);
        let _ = size(cut);
        let _ = pack::is_animated(cut);
        let _ = frame_durations(cut);
    }
    let still = png(512, 512, &[b"acTL", b"IDAT"]);
    for length in 0..=still.len() {
        let _ = size(&still[..length]);
        let _ = pack::is_animated(&still[..length]);
    }
}

// ---- a GIF: never a sticker as it is, and a moving one never made into one ------------------

/// A GIF the way the format says: header, screen, an optional colour table, then the blocks given.
fn gif(global_table: bool, blocks: &[Vec<u8>]) -> Vec<u8> {
    let mut file = b"GIF89a".to_vec();
    // Width 2, height 2, packed (a 4-entry table when there is one), background, aspect.
    file.extend_from_slice(&[2, 0, 2, 0, if global_table { 0x81 } else { 0x00 }, 0, 0]);
    if global_table {
        file.extend_from_slice(&[0u8; 12]);
    }
    for block in blocks {
        file.extend_from_slice(block);
    }
    file.push(0x3B);
    file
}

/// One picture: its descriptor, an optional local colour table, the code size, and its data in
/// sub-blocks.
fn gif_image(local_table: bool) -> Vec<u8> {
    let mut image = vec![
        0x2C,
        0,
        0,
        0,
        0,
        2,
        0,
        2,
        0,
        if local_table { 0x80 } else { 0x00 },
    ];
    if local_table {
        image.extend_from_slice(&[0u8; 6]);
    }
    // Data that LOOKS like an image separator, inside a sub-block: it must be stepped over, not
    // counted.
    image.extend_from_slice(&[2, 3, 0x2C, 0x2C, 0x2C, 1, 0x2C, 0]);
    image
}

/// A graphic control extension, as every frame of an animation carries — and a still GIF may too.
fn gif_control() -> Vec<u8> {
    vec![0x21, 0xF9, 4, 0, 10, 0, 0, 0]
}

/// The "loop for ever" application extension.
fn gif_loop() -> Vec<u8> {
    [&[0x21, 0xFF, 11][..], b"NETSCAPE2.0", &[3, 1, 0, 0, 0]].concat()
}

#[test]
fn a_gif_with_more_than_one_picture_is_animated() {
    assert!(is_animated_gif(&gif(
        true,
        &[
            gif_loop(),
            gif_control(),
            gif_image(false),
            gif_control(),
            gif_image(false)
        ]
    )));
    assert!(is_animated_gif(&gif(
        false,
        &[gif_image(true), gif_image(true)]
    )));
    // GIF87a has no extensions at all, and may still hold two pictures.
    let mut old = gif(true, &[gif_image(false), gif_image(false)]);
    old[..6].copy_from_slice(b"GIF87a");
    assert!(is_animated_gif(&old));
}

/// A STILL GIF IS A STILL PICTURE, whatever extensions it carries — a loop block and a control
/// block do not make one picture move — and bytes inside its data are never counted as pictures.
#[test]
fn a_gif_with_one_picture_is_still() {
    assert!(!is_animated_gif(&gif(true, &[gif_image(false)])));
    assert!(!is_animated_gif(&gif(
        true,
        &[gif_loop(), gif_control(), gif_image(true)]
    )));
    assert!(!is_animated_gif(&gif(false, &[])));
}

#[test]
fn only_a_gif_is_asked_whether_it_is_an_animated_gif() {
    assert!(pack::is_gif(b"GIF89a"));
    assert!(pack::is_gif(b"GIF87a"));
    assert!(!pack::is_gif(b"GIF90a"));
    assert!(!pack::is_gif(b"GIF"));
    // A GIF is never a sticker as it is: the two accepted types are read from the bytes, and it
    // is neither.
    assert_eq!(
        mime(&gif(true, &[gif_image(false), gif_image(false)])),
        None
    );
    assert!(!is_animated_gif(&riff(&[
        vp8x(512, 384, true),
        anmf(40, 3),
        anmf(60, 3)
    ])));
    assert!(!is_animated_gif(&png(512, 512, &[b"acTL", b"IDAT"])));
    assert!(!is_animated_gif(&[
        0xFF, 0xD8, 0xFF, 0xE0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
    ]));
}

/// Every prefix of an animated GIF, and one whose lengths lie, ends the walk and nothing else.
#[test]
fn a_gif_that_is_cut_short_or_lies_ends_the_walk() {
    let moving = gif(
        true,
        &[
            gif_loop(),
            gif_control(),
            gif_image(true),
            gif_control(),
            gif_image(false),
        ],
    );
    for length in 0..=moving.len() {
        let _ = is_animated_gif(&moving[..length]);
    }
    // A sub-block that claims 255 bytes the file does not have.
    let mut liar = gif(true, &[gif_image(false), gif_image(false)]);
    liar[13 + 12 + 10 + 1] = 0xFF;
    assert!(!is_animated_gif(&liar));
    // A screen that promises a 768-byte colour table in a 20-byte file.
    let mut table = gif(false, &[gif_image(false), gif_image(false)]);
    table[10] = 0x87;
    let _ = is_animated_gif(&table);
    assert!(!is_animated_gif(&table[..20]));
}
