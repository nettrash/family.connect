//! `RoundVideoTests.AQuoteOfAVideoMessageSaysSo`: a reply quoting a circle says "Video message"
//! where the server's excerpt is empty (S5.7), when this device holds the quoted message; a held
//! one with words, and one it does not hold, stay as the server cut them.

mod common;

use fc_core::protocol::{AttachmentDto, MemberDto, MessageDto, QuoteParentDto, ReplyToDto};
use fc_core::store::{SeqRoute, StoreResult};
use fc_logic::quotes::{self, QuoteLine};

use common::chat::{CHAT, ChatRig, ME};

const SENT: &str = "2026-10-05T10:00:00Z";

fn circle() -> AttachmentDto {
    AttachmentDto {
        mime: Some("video/mp4".to_string()),
        size: Some(1_649_700),
        width: Some(480),
        height: Some(480),
        duration_ms: Some(23400),
        has_preview: true,
        round: true,
        ..AttachmentDto::new(91, "video")
    }
}

fn message(id: i64, sender: i64, media: Vec<AttachmentDto>) -> MessageDto {
    MessageDto {
        attachments: Some(media),
        ..MessageDto::new(id, CHAT, sender, None, "", SENT)
    }
}

#[test]
fn a_quote_of_a_video_message_says_so() -> StoreResult<()> {
    let rig = ChatRig::new()?;
    rig.chats.replace_members(
        &[
            MemberDto::new(ME, "anna", "Anna"),
            MemberDto::new(11, "bob", "Bob"),
        ],
        &[],
    )?;
    let say = common::english();
    rig.chats.apply(
        &message(40, 11, vec![circle()]),
        SeqRoute::CatchUpPage,
        true,
    )?;
    rig.chats.apply(
        &message(
            39,
            11,
            vec![AttachmentDto {
                id: 90,
                round: false,
                ..circle()
            }],
        ),
        SeqRoute::CatchUpPage,
        true,
    )?;
    let reply_to = |id: i64, excerpt: &str| MessageDto {
        reply_to: Some(ReplyToDto {
            message_id: id,
            sender_id: 11,
            excerpt: excerpt.to_string(),
            parent: None,
        }),
        ..MessageDto::new(50, CHAT, ME, None, "lovely", SENT)
    };
    let reply = |message: MessageDto| -> StoreResult<QuoteLine> {
        Ok(quotes::of(&message, &rig.chats, &say, |_| false)?
            .unwrap()
            .reply)
    };

    assert_eq!(
        QuoteLine::new(40, "Bob", "Video message", false),
        reply(reply_to(40, ""))?
    );
    assert_eq!(
        QuoteLine::new(39, "Bob", "", false),
        reply(reply_to(39, ""))?
    );
    assert_eq!(
        QuoteLine::new(38, "Bob", "", false),
        reply(reply_to(38, ""))?
    );
    assert_eq!(
        QuoteLine::new(40, "Bob", "the server's", false),
        reply(reply_to(40, "the server's"))?
    );
    // And at the second level the same.
    let deeper = MessageDto {
        reply_to: Some(ReplyToDto {
            message_id: 50,
            sender_id: ME,
            excerpt: "lovely".to_string(),
            parent: Some(QuoteParentDto {
                message_id: 40,
                sender_id: 11,
                excerpt: String::new(),
            }),
        }),
        ..MessageDto::new(51, CHAT, 11, None, "yes", SENT)
    };
    assert_eq!(
        Some(QuoteLine::new(40, "Bob", "Video message", false)),
        quotes::of(&deeper, &rig.chats, &say, |_| false)?
            .unwrap()
            .parent
    );
    Ok(())
}
