//! `StickerLookTests.AStickerBehavesLikeAnyMessage`, its `BubbleRules` half: a sticker is a message
//! like any other — it may be reported and its sender blocked, and it is never mistaken for an
//! assistant answer not yet written. (The other half, "Edit is never offered on a sticker", is
//! `ConversationModel.MayEdit` — `Conversation.cs`, a later wave.)

use fc_core::protocol::{AttachmentDto, MessageDto};
use fc_logic::bubble_rules::{awaited, is_other_member, may_report};

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

/// `Message(body, media)`: from member 9 in chat 42.
fn message(body: &str, media: Vec<AttachmentDto>) -> MessageDto {
    MessageDto {
        attachments: Some(media),
        ..MessageDto::new(1, 42, 9, None, body, "2026-09-13T10:00:00Z")
    }
}

#[test]
fn a_sticker_behaves_like_any_message() {
    let theirs = message("", vec![sent()]);
    assert!(theirs.sticker_picture().is_some());
    assert!(may_report(&theirs, 7, false, None));
    assert!(is_other_member(&theirs, 7, false, None));
    // Never mistaken for an assistant answer that has not been written yet.
    assert!(!awaited(&theirs, "", 7, true, Some(9)));
}
