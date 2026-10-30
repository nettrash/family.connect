//! `PackPickingTests` (the second class of `PackModelTests.cs`): what is decided about a picture
//! somebody chose for the pack, before anything is uploaded (docs/protocol.md, "512 × 512 is the
//! CLIENT's rule") — and what the window says about the pack.

mod common;

use fc_core::protocol::{ApiError, AttachmentDto, PackItemDto, error_codes};
use fc_core::store::PackLimits;
use fc_logic::pack::pack_picking::{self, Step};
use fc_logic::pack::{PackAdded, pack_text};

use common::english;

const LIMITS: PackLimits = PackLimits {
    max_items: 200,
    max_item_bytes: 1000,
};

/// `PackPickingTests.WebP`: a `VP8X` header with the animation bit as asked, zeros after it, and no
/// RIFF length.
fn webp(width: u32, height: u32, animated: bool, length: usize) -> Vec<u8> {
    let mut file = vec![0u8; length.max(30)];
    file[0..4].copy_from_slice(b"RIFF");
    file[8..12].copy_from_slice(b"WEBP");
    file[12..16].copy_from_slice(b"VP8X");
    file[16..20].copy_from_slice(&10u32.to_le_bytes());
    file[20] = if animated { 0x02 } else { 0 };
    file[24..27].copy_from_slice(&(width - 1).to_le_bytes()[..3]);
    file[27..30].copy_from_slice(&(height - 1).to_le_bytes()[..3]);
    file
}

/// `PackPickingTests.Png`: the signature, an `IHDR`, and after it an animation control chunk or the
/// image data.
fn png(width: u32, height: u32, length: usize, animated: bool) -> Vec<u8> {
    let mut file = vec![0u8; length.max(64)];
    file[0..8].copy_from_slice(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
    file[8..12].copy_from_slice(&13u32.to_be_bytes());
    file[12..16].copy_from_slice(b"IHDR");
    file[16..20].copy_from_slice(&width.to_be_bytes());
    file[20..24].copy_from_slice(&height.to_be_bytes());
    // IHDR's crc (4), then the next chunk: an animation control chunk, or the image data.
    file[33..37].copy_from_slice(&8u32.to_be_bytes());
    file[37..41].copy_from_slice(if animated { b"acTL" } else { b"IDAT" });
    file
}

/// A GIF of that many pictures, the way the format writes them.
fn gif(frames: usize) -> Vec<u8> {
    let mut file = b"GIF89a".to_vec();
    file.extend_from_slice(&[2, 0, 2, 0, 0, 0, 0]);
    for _ in 0..frames {
        file.extend_from_slice(&[0x21, 0xF9, 4, 0, 10, 0, 0, 0]);
        file.extend_from_slice(&[0x2C, 0, 0, 0, 0, 2, 0, 2, 0, 0, 2, 2, 0x4C, 0x01, 0]);
    }
    file.push(0x3B);
    file
}

fn refused_code(plan: &pack_picking::Plan) -> &str {
    plan.refused
        .as_ref()
        .map_or("", |error| error.code.as_str())
}

/// A FINISHED STICKER IS TAKEN AS GIVEN, whatever its pixel size, so long as it is within the byte
/// ceiling: re-encoding it buys nothing and, for an animated one, is not possible.
#[test]
fn a_sticker_within_the_ceiling_goes_up_as_it_is() {
    for (width, height, animated) in [
        (512, 512, false),
        (96, 96, false),
        (2000, 2000, false),
        (2000, 2000, true),
    ] {
        let plan = pack_picking::plan_for(&webp(width, height, animated, 1000), LIMITS, 0);

        assert_eq!(Step::Take, plan.what, "{width} × {height} {animated}");
        assert_eq!(None, plan.refused);
    }
}

/// 512 × 512 is THIS CLIENT's rule, when it MAKES one: a still picture over the ceiling and larger
/// than the box is fitted into it — whole, proportions kept.
#[test]
fn a_large_still_picture_over_the_ceiling_is_fitted_into_the_box() {
    let plan = pack_picking::plan_for(&webp(2048, 1024, false, 1001), LIMITS, 0);

    assert_eq!(Step::Make, plan.what);
    assert_eq!(Some((512, 256)), plan.target);
}

/// NEVER RE-ENCODE AN ANIMATED ONE: over the ceiling it is simply too big.
#[test]
fn an_animated_sticker_over_the_ceiling_is_refused_and_never_shrunk() {
    let plan = pack_picking::plan_for(&webp(2048, 1024, true, 1001), LIMITS, 0);

    assert_eq!(Step::Refuse, plan.what);
    assert_eq!(error_codes::PACK_ITEM_TOO_LARGE, refused_code(&plan));
}

/// One that already fits the box cannot be made smaller by redrawing the same pixels.
#[test]
fn a_still_picture_already_in_the_box_and_over_the_ceiling_is_refused() {
    let plan = pack_picking::plan_for(&webp(512, 512, false, 1001), LIMITS, 0);

    assert_eq!(Step::Refuse, plan.what);
    assert_eq!(error_codes::PACK_ITEM_TOO_LARGE, refused_code(&plan));
}

/// The most basic thing wrong first, in the order the server checks: the type, the size, and only
/// then the room.
#[test]
fn the_refusal_is_the_most_basic_thing_wrong() {
    let jpeg = [0xFF, 0xD8, 0xFF, 0xE0, 0, 0, 0, 0, 0, 0, 0, 0];
    let code = |bytes: &[u8], held: usize| {
        refused_code(&pack_picking::plan_for(bytes, LIMITS, held)).to_string()
    };

    // What the picture IS comes first: a moving one that is not a WebP is refused as that even into
    // a full pack.
    assert_eq!(pack_picking::ANIMATED_CODE, code(&gif(2), 200));
    assert_eq!(error_codes::PACK_FULL, code(&jpeg, 200));
    assert_eq!(
        error_codes::PACK_ITEM_TOO_LARGE,
        code(&webp(512, 512, false, 1001), 200)
    );
    assert_eq!(
        error_codes::PACK_FULL,
        code(&webp(512, 512, false, 1000), 200)
    );
    assert_eq!(
        error_codes::PACK_FULL,
        code(&webp(2048, 2048, false, 1001), 200)
    );
    assert_eq!(
        None,
        pack_picking::plan_for(&webp(512, 512, false, 1000), LIMITS, 199).refused
    );
}

// ---- what may be picked ------------------------------------------------------------------

/// ANY STILL PICTURE THE PLATFORM CAN DECODE MAY BE PICKED. One that is not a WebP or a PNG — a
/// JPEG, a HEIC, a still GIF — is MADE into a sticker: fitted whole into 512 × 512 and written as
/// PNG, whatever its byte size. Never refused for its type, and never taken as given.
#[test]
fn a_still_picture_of_another_kind_is_made_into_a_sticker() {
    let heads: [[u8; 12]; 3] = [
        [
            0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10, b'J', b'F', b'I', b'F', 0, 1,
        ],
        [
            0, 0, 0, 0x18, b'f', b't', b'y', b'p', b'h', b'e', b'i', b'c',
        ],
        [b'B', b'M', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    ];
    for head in heads {
        let small = pack_picking::plan_for(&head, LIMITS, 0);
        let large_bytes: Vec<u8> = head.iter().copied().chain([0u8; 5000]).collect();
        let large = pack_picking::plan_for(&large_bytes, LIMITS, 0);

        assert_eq!(Step::Make, small.what);
        assert_eq!(Step::Make, large.what);
        assert_eq!(None, small.refused);
        // The decoder says how big it is; the fit is this client's
        // (the_box_keeps_the_whole_picture_and_its_shape).
        assert_eq!(None, small.target);
    }
}

#[test]
fn a_still_gif_is_made_into_a_sticker_too() {
    assert_eq!(Step::Make, pack_picking::plan_for(&gif(1), LIMITS, 0).what);
}

/// AN ANIMATED PICTURE THAT IS NOT ALREADY AN ACCEPTABLE WEBP IS REFUSED, in words — never made
/// into a sticker, which would silently keep one frame of it.
#[test]
fn an_animated_gif_is_refused_and_never_flattened() {
    let plan = pack_picking::plan_for(&gif(3), LIMITS, 0);

    assert_eq!(Step::Refuse, plan.what);
    let refused = plan.refused.expect("refused");
    assert_eq!(pack_picking::ANIMATED_CODE, refused.code);
    assert_eq!(
        "Animated stickers must be WebP.",
        pack_text::sentence(&refused, &english())
    );
    // Not a server's code, and not a transient failure: trying again would refuse again.
    assert!(!refused.transient());
}

/// An animated PNG this client would have to REDRAW — one over the byte ceiling — would come out as
/// its first frame: refused with the same sentence, whatever its pixel size. One within the ceiling
/// is a finished sticker and goes up byte for byte, its animation untouched.
#[test]
fn an_animated_png_that_would_be_flattened_is_refused() {
    let big = pack_picking::plan_for(&png(2048, 2048, 1001, true), LIMITS, 0);
    let in_the_box = pack_picking::plan_for(&png(512, 512, 1001, true), LIMITS, 0);
    let within = pack_picking::plan_for(&png(2048, 2048, 1000, true), LIMITS, 0);

    assert_eq!(pack_picking::ANIMATED_CODE, refused_code(&big));
    assert_eq!(pack_picking::ANIMATED_CODE, refused_code(&in_the_box));
    assert_eq!(Step::Take, within.what);
    // A STILL PNG of the same size is what the box is for.
    let still = pack_picking::plan_for(&png(2048, 1024, 1001, false), LIMITS, 0);
    assert_eq!(Step::Make, still.what);
    assert_eq!(Some((512, 256)), still.target);
}

/// A PNG within the ceiling is taken as given, like a WebP — whatever its pixel size.
#[test]
fn a_png_within_the_ceiling_goes_up_as_it_is() {
    for (width, height) in [(96, 96), (512, 512), (2000, 1500)] {
        assert_eq!(
            Step::Take,
            pack_picking::plan_for(&png(width, height, 1000, false), LIMITS, 0).what,
            "{width} × {height}"
        );
    }
}

/// A picture made into a sticker is asked again, as the bytes it now is — and may still be too big.
#[test]
fn bytes_about_to_go_up_are_asked_once_more() {
    let code = |bytes: &[u8], held: usize| {
        pack_picking::refusal(bytes, LIMITS, held).map(|error| error.code)
    };

    assert_eq!(None, code(&webp(512, 512, false, 1000), 0));
    assert_eq!(
        Some(error_codes::PACK_ITEM_TOO_LARGE.to_string()),
        code(&webp(512, 512, false, 1001), 0)
    );
    assert_eq!(
        Some(error_codes::INVALID_ATTACHMENT.to_string()),
        code(b"GIF89a......", 0)
    );
    assert_eq!(
        Some(error_codes::PACK_FULL.to_string()),
        code(&webp(512, 512, false, 10), 200)
    );
}

#[test]
fn the_box_keeps_the_whole_picture_and_its_shape() {
    for (width, height, fitted_width, fitted_height) in [
        (2048, 1024, 512, 256),
        (1024, 2048, 256, 512),
        (4000, 3, 512, 1),
        (300, 200, 300, 200), // never scaled UP
        (513, 513, 512, 512),
    ] {
        assert_eq!(
            (fitted_width, fitted_height),
            pack_picking::fit(width, height),
            "{width} × {height}"
        );
    }
}

#[test]
fn a_refusal_is_said_in_the_packs_own_words() {
    let say = english();
    let said =
        |code: &str, status: u16| pack_text::sentence(&ApiError::new(code, "…", status), &say);

    assert_eq!(
        "That picture is too big to be a sticker.",
        said(error_codes::PACK_ITEM_TOO_LARGE, 413)
    );
    assert_eq!(
        "A sticker has to be a WebP or PNG picture.",
        said(error_codes::INVALID_ATTACHMENT, 400)
    );
    assert_eq!(
        "The family's stickers are full. Remove one to make room.",
        said(error_codes::PACK_FULL, 409)
    );
    assert_eq!(
        "Only whoever added a sticker, or the family owner, can remove it.",
        said(error_codes::NOT_PACK_ITEM_AUTHOR, 403)
    );
    // A transient failure is never said as a refusal.
    assert_eq!(
        "The sticker didn't reach the server. Check your connection and try again.",
        pack_text::sentence(&ApiError::transport("down"), &say)
    );
    assert_eq!(
        "Something went wrong. Try again.",
        said(error_codes::PACK_ITEM_NOT_FOUND, 404)
    );
    assert_eq!(
        "The family already has that sticker.",
        pack_text::added(PackAdded::AlreadyThere, &say)
    );
    assert_eq!(
        "Added to family stickers",
        pack_text::added(PackAdded::Added, &say)
    );
}

#[test]
fn a_sticker_is_named_for_a_screen_reader_and_the_pack_is_counted() {
    let say = english();
    let picture = AttachmentDto {
        mime: Some("image/webp".to_string()),
        ..AttachmentDto::new(71, "photo")
    };
    let item = |label: Option<&str>| PackItemDto {
        added_by: 7,
        attachment: Some(picture.clone()),
        label: label.map(str::to_string),
        ..PackItemDto::new(5)
    };

    assert_eq!(
        "Sticker: party cat",
        pack_text::name(&item(Some("party cat")), &say)
    );
    assert_eq!("Sticker", pack_text::name(&item(None), &say));
    assert_eq!(
        "12 of 200 stickers",
        pack_text::fullness(
            12,
            PackLimits {
                max_items: 200,
                max_item_bytes: 524_288
            },
            &say
        )
    );
}
