//! `StickerLookTests.AStickerBehavesLikeAnyMessage`, its `ConversationModel.MayEdit` half: a sticker
//! has no words, so there is nothing to edit — and "Edit" is never offered on one, which is its own
//! rule and not a consequence of the empty body. (The `BubbleRules` half — it may be reported, its
//! sender blocked, never mistaken for an unwritten answer — is `bubble_rules_sticker_look.rs`.)

use fc_core::protocol::{AttachmentDto, MessageDto};
use fc_logic::conversation::{Bubble, ConversationModel};

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
    let mine = Bubble::new(
        MessageDto {
            sender_id: 7,
            ..message("", vec![sent()])
        },
        false,
        false,
        true,
    );

    assert!(!ConversationModel::may_edit(&mine));
    // "Edit" IS NEVER OFFERED ON A STICKER, and that is its own rule — not a consequence of the
    // empty body. A sticker that somehow carried words would be refused by the server's edit just
    // the same.
    let worded = Bubble {
        message: MessageDto {
            body: "look".to_string(),
            ..mine.message.clone()
        },
        ..mine.clone()
    };
    assert!(worded.message.sticker_picture().is_some());
    assert!(!ConversationModel::may_edit(&worded));
    // The same words on a PHOTO — the flag absent — are the author's to edit, WebP or not.
    let photo = Bubble {
        message: MessageDto {
            attachments: Some(vec![AttachmentDto {
                sticker: false,
                ..sent()
            }]),
            ..worded.message.clone()
        },
        ..worded.clone()
    };
    assert!(photo.message.sticker_picture().is_none());
    assert!(ConversationModel::may_edit(&photo));
}
