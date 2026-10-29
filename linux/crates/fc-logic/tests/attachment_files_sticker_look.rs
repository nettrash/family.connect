//! `StickerLookTests`, the two cases about `AttachmentFiles`: a sticker is drawn from its original
//! whatever `has_preview` says, and a saved sticker keeps its own type.

use fc_core::protocol::AttachmentDto;
use fc_logic::attachment_files::{self, TileSource};

/// `new AttachmentDto(90, "photo", "image/webp", 4096, 512, 512, Sticker: true)`.
fn sent() -> AttachmentDto {
    AttachmentDto {
        mime: Some("image/webp".to_string()),
        size: Some(4096),
        width: Some(512),
        height: Some(512),
        sticker: true,
        ..AttachmentDto::new(90, "photo")
    }
}

/// A STICKER IS ALWAYS ITS ORIGINAL, whatever `has_preview` says — a preview is a JPEG, and the
/// flag can be inherited through dedup from somebody who once sent the same PNG as a photo.
#[test]
fn a_sticker_is_drawn_from_its_original_even_where_it_says_it_has_a_preview() {
    assert_eq!(
        attachment_files::source_for(&AttachmentDto {
            has_preview: true,
            ..sent()
        }),
        TileSource::Original
    );
    assert_eq!(attachment_files::source_for(&sent()), TileSource::Original);
    // And a photo is still a photo.
    assert_eq!(
        attachment_files::source_for(&AttachmentDto {
            sticker: false,
            has_preview: true,
            ..sent()
        }),
        TileSource::Preview
    );
}

#[test]
fn a_saved_sticker_keeps_its_own_type() {
    assert_eq!(attachment_files::file_name(&sent()), "photo-90.webp");
    assert_eq!(
        attachment_files::file_name(&AttachmentDto {
            mime: Some("image/png".to_string()),
            ..sent()
        }),
        "photo-90.png"
    );
}
