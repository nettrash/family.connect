//! `StickerLookTests.ShownLargerItIsCalledASticker`: in the viewer a sticker is called one, and the
//! same attachment without the flag is the photo it always was.

mod common;

use fc_core::protocol::AttachmentDto;
use fc_logic::media_album::MediaAlbum;

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

#[test]
fn shown_larger_it_is_called_a_sticker() {
    let say = common::english();
    assert_eq!(
        MediaAlbum::new(vec![sent()], 0, false).unwrap().title(&say),
        "Sticker"
    );
    let photo = AttachmentDto {
        sticker: false,
        ..sent()
    };
    assert_eq!(
        MediaAlbum::new(vec![photo], 0, false).unwrap().title(&say),
        "Photo"
    );
}
