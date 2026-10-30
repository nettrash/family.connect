//! The chat-list rows that waited for `ChatListModel` in three other C# files:
//! `AnimationDrawingTests.AChatListRowSaysGifWhereAVideosSaysVideo`,
//! `RoundVideoTests.AChatListRowSaysVideoMessageWhereAVideosSaysVideo` and
//! `StickerLookTests.AChatListRowSaysStickerWhereAPhotosSaysPhoto` — what a row says of a message
//! that is a GIF, a circle or a sticker, and of the same attachment without its flag.

mod common;

use std::rc::Rc;

use fc_core::protocol::{AttachmentDto, ChatDto, ChatRowDto, MemberDto, MessageDto};
use fc_core::store::{ChatStore, Database, StoreResult};
use fc_logic::chat_list::ChatListModel;

const ME: i64 = 7;
const CHAT: i64 = 42;

fn list() -> StoreResult<ChatListModel> {
    let cache = Database::open_in_memory()?;
    let chats = Rc::new(ChatStore::new(&cache).with_me(|| ME));
    chats.replace(
        &[ChatRowDto::new(ChatDto::new(CHAT, "family", "The Smiths"))],
        None,
    )?;
    chats.replace_members(
        &[
            MemberDto::new(ME, "anna", "Anna"),
            MemberDto::new(11, "bob", "Bob"),
        ],
        &[],
    )?;
    Ok(ChatListModel::new(chats, || ME, Rc::new(common::english())))
}

fn message(id: i64, sender: i64, body: &str, sent: &str, media: Vec<AttachmentDto>) -> MessageDto {
    MessageDto {
        attachments: Some(media),
        ..MessageDto::new(id, CHAT, sender, None, body, sent)
    }
}

fn video(id: i64, size: i64, width: i32, height: i32, duration_ms: i32) -> AttachmentDto {
    AttachmentDto {
        mime: Some("video/mp4".to_string()),
        size: Some(size),
        width: Some(width),
        height: Some(height),
        duration_ms: Some(duration_ms),
        has_preview: true,
        ..AttachmentDto::new(id, "video")
    }
}

/// "GIF" on the chat list where a video says "Video" — and the caption first, as any video's row
/// has it.
#[test]
fn a_chat_list_row_says_gif_where_a_videos_says_video() -> StoreResult<()> {
    const SENT: &str = "2026-10-05T10:00:00Z";
    let gif = AttachmentDto {
        animation: true,
        ..video(93, 182_330, 480, 270, 2100)
    };
    let list = list()?;
    let row = |body: &str, media: Vec<AttachmentDto>, hidden: bool| {
        list.preview(&message(1, 11, body, SENT, media), hidden)
    };
    assert_eq!("GIF", row("", vec![gif.clone()], false));
    assert_eq!("ha", row("ha", vec![gif.clone()], false));
    assert_eq!(
        "Video",
        row(
            "",
            vec![AttachmentDto {
                animation: false,
                ..gif.clone()
            }],
            false
        )
    );
    // Two flagged videos are not an animation, whatever they carry.
    assert_eq!(
        "Video",
        row(
            "",
            vec![
                gif.clone(),
                AttachmentDto {
                    id: 94,
                    ..gif.clone()
                }
            ],
            false
        )
    );
    assert_eq!("Hidden — blocked member", row("", vec![gif], true));
    Ok(())
}

#[test]
fn a_chat_list_row_says_video_message_where_a_videos_says_video() -> StoreResult<()> {
    const SENT: &str = "2026-10-05T10:00:00Z";
    let circle = AttachmentDto {
        round: true,
        ..video(91, 1_649_700, 480, 480, 23_400)
    };
    let list = list()?;
    let row = |media: Vec<AttachmentDto>, hidden: bool| {
        list.preview(&message(1, 11, "", SENT, media), hidden)
    };
    assert_eq!("Video message", row(vec![circle.clone()], false));
    assert_eq!(
        "Video",
        row(
            vec![AttachmentDto {
                round: false,
                ..circle.clone()
            }],
            false
        )
    );
    // Two videos are not a circle, whatever they carry.
    assert_eq!(
        "Video",
        row(
            vec![
                circle.clone(),
                AttachmentDto {
                    id: 92,
                    ..circle.clone()
                }
            ],
            false
        )
    );
    assert_eq!("Hidden — blocked member", row(vec![circle], true));
    // A voice note keeps this client's own word for it.
    assert_eq!(
        "Voice message",
        row(vec![AttachmentDto::new(93, "audio")], false)
    );
    Ok(())
}

/// On a chat-list row it is the word, where a photo's row says so of a photo.
#[test]
fn a_chat_list_row_says_sticker_where_a_photos_says_photo() -> StoreResult<()> {
    let cache = Database::open_in_memory()?;
    let list = ChatListModel::new(
        Rc::new(ChatStore::new(&cache)),
        || 7,
        Rc::new(common::english()),
    );
    let sent = AttachmentDto {
        mime: Some("image/webp".to_string()),
        size: Some(4096),
        width: Some(512),
        height: Some(512),
        sticker: true,
        ..AttachmentDto::new(90, "photo")
    };
    let row = |media: AttachmentDto, hidden: bool| {
        list.preview(
            &MessageDto {
                attachments: Some(vec![media]),
                ..MessageDto::new(1, 42, 9, None, "", "2026-09-13T10:00:00Z")
            },
            hidden,
        )
    };

    assert_eq!("Sticker", row(sent.clone(), false));
    // OLD MESSAGES: the same attachment without the flag is the photo it always was.
    assert_eq!(
        "Photo",
        row(
            AttachmentDto {
                sticker: false,
                ..sent.clone()
            },
            false
        )
    );
    // A blocked member's sticker is the hidden row, like anything else they sent.
    assert_eq!("Hidden — blocked member", row(sent, true));
    Ok(())
}
