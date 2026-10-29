//! `AnimationDrawingTests.AQuoteOfAnAnimationSaysGif`: a reply quoting a caption-less animation
//! says "GIF" where the server's excerpt is empty.

mod common;

use fc_core::protocol::{AttachmentDto, MemberDto, MessageDto, ReplyToDto};
use fc_core::store::{SeqRoute, StoreResult};
use fc_logic::quotes::{self, QuoteLine};

use common::chat::{CHAT, ChatRig, ME};

const SENT: &str = "2026-10-19T10:00:00Z";

/// `new AttachmentDto(93, "video", "video/mp4", 182_330, 480, 270, 2100, HasPreview: true,
/// Animation: true)`.
fn gif() -> AttachmentDto {
    AttachmentDto {
        mime: Some("video/mp4".to_string()),
        size: Some(182_330),
        width: Some(480),
        height: Some(270),
        duration_ms: Some(2100),
        has_preview: true,
        animation: true,
        ..AttachmentDto::new(93, "video")
    }
}

#[test]
fn a_quote_of_an_animation_says_gif() -> StoreResult<()> {
    let rig = ChatRig::new()?;
    rig.chats.replace_members(
        &[
            MemberDto::new(ME, "anna", "Anna"),
            MemberDto::new(11, "bob", "Bob"),
        ],
        &[],
    )?;
    let say = common::english();
    let message = |id: i64, media: AttachmentDto| MessageDto {
        attachments: Some(vec![media]),
        ..MessageDto::new(id, CHAT, 11, None, "", SENT)
    };
    rig.chats
        .apply(&message(40, gif()), SeqRoute::CatchUpPage, true)?;
    rig.chats.apply(
        &message(
            39,
            AttachmentDto {
                id: 90,
                animation: false,
                ..gif()
            },
        ),
        SeqRoute::CatchUpPage,
        true,
    )?;
    let reply = |id: i64, excerpt: &str| -> StoreResult<QuoteLine> {
        let replying = MessageDto {
            reply_to: Some(ReplyToDto {
                message_id: id,
                sender_id: 11,
                excerpt: excerpt.to_string(),
                parent: None,
            }),
            ..MessageDto::new(50, CHAT, ME, None, "lovely", SENT)
        };
        Ok(quotes::of(&replying, &rig.chats, &say, |_| false)?
            .unwrap()
            .reply)
    };

    assert_eq!(QuoteLine::new(40, "Bob", "GIF", false), reply(40, "")?);
    assert_eq!(QuoteLine::new(39, "Bob", "", false), reply(39, "")?);
    assert_eq!(QuoteLine::new(40, "Bob", "ha", false), reply(40, "ha")?);
    Ok(())
}
