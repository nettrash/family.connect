//! `RoundVideoTests`, the conversation's half: no Edit on a circle, and the send that makes one
//! (with the media pump and the pipeline it goes through). The quote is `quotes_round_video.rs`, the
//! kept poster `attachment_cache_round_video.rs`; the rest of that file is the chat list's, the
//! viewer's and playback's (later packages).

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use fc_core::protocol::send_rules::FlushTrigger;
use fc_core::protocol::{AttachmentDto, MemberDto, MessageDto, SendPipeline};
use fc_core::store::StoreResult;
use fc_core::testing::StillClock;
use fc_logic::conversation::{Bubble, ConversationModel};
use fc_logic::media_outbox::{MediaOutbox, StagedMedia};

use common::chat::{CHAT, ChatRig, ME, START, socket};
use common::media::Staging;
use common::server::Server;

const SENT: &str = "2026-10-05T10:00:00Z";

/// `new AttachmentDto(91, "video", "video/mp4", 1649700, 480, 480, 23400, HasPreview: true, Round:
/// true)`.
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

fn message(id: i64, sender: i64, body: &str, media: Vec<AttachmentDto>) -> MessageDto {
    MessageDto {
        attachments: Some(media),
        ..MessageDto::new(id, CHAT, sender, None, body, SENT)
    }
}

/// The class constructor: Anna (the reader) and Bob.
fn rig() -> StoreResult<ChatRig> {
    let rig = ChatRig::new()?;
    rig.chats.replace_members(
        &[
            MemberDto::new(ME, "anna", "Anna"),
            MemberDto::new(11, "bob", "Bob"),
        ],
        &[],
    )?;
    Ok(rig)
}

/// NO EDIT on a circle, as on a sticker: the server refuses it and there is no balloon to hold
/// words.
#[test]
fn a_video_message_is_never_offered_edit() {
    let of = |message: MessageDto| {
        let mine = message.sender_id == ME;
        Bubble::new(message, false, false, mine)
    };

    assert!(!ConversationModel::may_edit(&of(message(
        1,
        ME,
        "",
        vec![circle()]
    ))));
    // Even with words some server stored beside it.
    assert!(!ConversationModel::may_edit(&of(message(
        1,
        ME,
        "look",
        vec![circle()]
    ))));
    // An ordinary video with a caption still may be.
    assert!(ConversationModel::may_edit(&of(message(
        1,
        ME,
        "look",
        vec![AttachmentDto {
            round: false,
            ..circle()
        }]
    ))));
}

/// THE STICKER'S PATTERN: the row is written down first, the square video goes up as a video with
/// its poster, and only then is the message posted — with `round: true`, no words, and the reply it
/// answers.
#[test]
fn send_round_uploads_the_square_video_and_its_poster_then_posts_the_flag() -> StoreResult<()> {
    const HANDLE: &str = "0123456789abcdef0123456789abcdef";
    let rig = rig()?;
    let server = Server::new();
    server
        .on_status(
            "/attachments",
            201,
            Some(
                r#"{"attachment": {"id": 91, "kind": "video", "mime": "video/mp4", "size": 6, "width": 480, "height": 480, "duration_ms": 23400}}"#,
            ),
        )
        .on_status("/attachments/91/preview", 204, None)
        .on_status(
            "/chats/42/messages",
            201,
            Some(
                r#"{"message": {"id": 1341, "chat_id": 42, "sender_id": 7, "client_msg_id": "c-1", "body": "", "created_at": "2026-10-05T10:00:00Z", "attachments": [{"id": 91, "kind": "video", "mime": "video/mp4", "size": 6, "width": 480, "height": 480, "duration_ms": 23400, "has_preview": true, "round": true}]}}"#,
            ),
        );
    let api = rig.api(&server);
    let staging = Staging {
        keeps_everything: true,
        ..Staging::new()
    };
    staging.put(
        HANDLE,
        StagedMedia {
            width: Some(480),
            height: Some(480),
            duration_ms: Some(23400),
            preview: Some(b"jpeg".to_vec()),
            ..StagedMedia::new("video", "video/mp4", b"ftypmp".to_vec())
        },
    );
    let media = Rc::new(MediaOutbox::new(
        rig.outbox.clone(),
        api.clone(),
        Rc::new(staging),
    ));
    let landed: Rc<RefCell<Vec<(bool, i64)>>> = Rc::default();
    let heard = landed.clone();
    media.landed.subscribe(move |(owed, attachment, _)| {
        heard.borrow_mut().push((owed.round, attachment.id))
    });
    let no_socket = socket(false);
    let sending = Rc::new(
        SendPipeline::from_api(
            Rc::new(no_socket.clone()),
            rig.outbox.clone(),
            rig.chats.clone(),
            api.clone(),
            Rc::new(StillClock::at(START)),
        )
        .with_next_id(|| "c-1".to_string())
        .with_uploads(media.pump()),
    );
    let chat = ConversationModel::new(
        CHAT,
        rig.chats.clone(),
        api,
        sending.clone(),
        Rc::new(no_socket),
        Rc::new(rig.clock.clone()),
    )?
    .with_outbox(rig.outbox.clone());

    let row = chat.send_round(HANDLE, Some(41))?;

    assert!(row.round);
    assert!(!row.sticker);
    assert_eq!("", row.body);
    let pending = chat.pending()?;
    assert_eq!(1, pending.len());
    assert!(pending[0].owes_uploads());
    assert!(server.asked().is_empty());

    assert_eq!(
        1,
        rig.drive(sending.flush(FlushTrigger::ConnectivityRestored))?
    );

    let asked = server.asked();
    assert_eq!(
        "/api/v1/attachments?kind=video&width=480&height=480&duration_ms=23400",
        asked[0]
    );
    assert_eq!("/api/v1/attachments/91/preview", asked[1]);
    assert_eq!("/api/v1/chats/42/messages", asked[2]);
    assert_eq!(
        Some(
            r#"{"client_msg_id":"c-1","body":"","reply_to_message_id":41,"attachment_ids":[91],"round":true}"#
        ),
        server.bodies().last().map(String::as_str)
    );
    assert_eq!(vec![(true, 91)], *landed.borrow());
    assert!(rig.outbox.all()?.is_empty());
    assert_eq!(
        91,
        rig.chats.message(1341)?.unwrap().round_video().unwrap().id
    );
    Ok(())
}
