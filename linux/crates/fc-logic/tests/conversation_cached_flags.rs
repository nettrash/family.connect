//! `CachedFlagsTests`: what the WINDOW reads for a sticker and a circle —
//! `ConversationModel::bubbles`, the record the shell's bubble branches on — when the cache was
//! written by a build that predates the flags: the bug the owner saw on Windows (a sticker in a grey
//! tile, a circle square, while every other device drew them right), and schema step 8's repair of
//! it.

mod common;

use std::rc::Rc;

use fc_core::protocol::{ApiResult, MessageResponse, Resync, SendPipeline};
use fc_core::rt::LocalBoxFuture;
use fc_core::store::{BoardStore, OutboxRow, StoreResult};
use fc_core::testing::StillClock;
use fc_logic::bubble_rules::media_mark;
use fc_logic::conversation::ConversationModel;

use common::chat::{CHAT, ChatRig, START, socket};
use common::server::Server;

const STICKER_JSON: &str = r#"
    {"id": 1340, "chat_id": 42, "sender_id": 9, "client_msg_id": "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01", "body": "",
     "created_at": "2026-10-05T10:00:00Z",
     "attachment": {"id": 90, "kind": "photo", "mime": "image/webp", "size": 40960, "width": 512, "height": 512, "has_preview": false, "sticker": true},
     "attachments": [{"id": 90, "kind": "photo", "mime": "image/webp", "size": 40960, "width": 512, "height": 512, "has_preview": false, "sticker": true}]}
    "#;

const ROUND_JSON: &str = r#"
    {"id": 1341, "chat_id": 42, "sender_id": 9, "client_msg_id": "4f9e21c0-ceea-4e17-a91c-0d9f8e7b2a01", "body": "",
     "created_at": "2026-10-05T10:01:00Z",
     "attachment": {"id": 91, "kind": "video", "mime": "video/mp4", "size": 812345, "width": 480, "height": 480, "duration_ms": 23400, "has_preview": true, "round": true},
     "attachments": [{"id": 91, "kind": "video", "mime": "video/mp4", "size": 812345, "width": 480, "height": 480, "duration_ms": 23400, "has_preview": true, "round": true}]}
    "#;

/// The two rows exactly as a build before #58/#79 left them: in sequence, the set without either
/// flag.
fn cached_by_an_older_build(rig: &ChatRig) {
    for (id, attachments) in [
        (
            1340,
            r#"[{"id":90,"kind":"photo","mime":"image/webp","size":40960,"width":512,"height":512,"has_preview":false}]"#,
        ),
        (
            1341,
            r#"[{"id":91,"kind":"video","mime":"video/mp4","size":812345,"width":480,"height":480,"duration_ms":23400,"has_preview":true}]"#,
        ),
    ] {
        rig.cache
            .connection()
            .execute_batch(&format!(
                "INSERT INTO messages (message_id, chat_id, sender_id, body, created_at, attachments_json, sequenced)
VALUES ({id}, 42, 9, '', 0, '{attachments}', 1)"
            ))
            .expect("the row an older build wrote");
    }
}

/// `Open(server)`: a conversation whose pipeline never posts ("no sends here") over a socket that
/// is up, its clock the rig's.
fn open(rig: &ChatRig, server: &Server) -> StoreResult<ConversationModel> {
    let socket = socket(true);
    let sending = SendPipeline::new(
        Rc::new(socket.clone()),
        rig.outbox.clone(),
        rig.chats.clone(),
        Rc::new(
            |_: OutboxRow| -> LocalBoxFuture<'static, ApiResult<MessageResponse>> {
                panic!("no sends here")
            },
        ),
        Rc::new(StillClock::at(START)),
    );
    ConversationModel::new(
        CHAT,
        rig.chats.clone(),
        rig.api(server),
        Rc::new(sending),
        Rc::new(socket),
        Rc::new(rig.clock.clone()),
    )
}

/// A FRESH CACHE DRAWS THEM RIGHT: the newest page, as the server sends it, reaches the bubbles
/// with both flags — the sticker branch and the circle branch, not the photo tile.
#[test]
fn a_fresh_cache_hands_the_window_a_sticker_and_a_circle() -> StoreResult<()> {
    let rig = ChatRig::new()?;
    let server = Server::new();
    server.on(
        "/chats/42/messages",
        &format!(r#"{{"messages": [{STICKER_JSON}, {ROUND_JSON}]}}"#),
    );
    let conversation = open(&rig, &server)?;

    assert_eq!(None, rig.drive(conversation.open())?);

    let bubbles = conversation.bubbles()?;
    assert_eq!(90, bubbles[0].message.sticker_picture().unwrap().id);
    assert_eq!(91, bubbles[1].message.round_video().unwrap().id);
    assert_eq!("s90", media_mark(&bubbles[0].message));
    assert_eq!("r91", media_mark(&bubbles[1].message));
    Ok(())
}

/// THE OWNER'S SCREEN, AND ITS REPAIR. Held in sequence, the chat is not read on open — so the
/// window is handed a photo and a plain video, and draws a tile and a square. One resync pass reads
/// each once and the window is handed a sticker and a circle; the redraw key changes with them, so
/// the conversation is drawn again.
#[test]
fn a_cache_an_older_build_wrote_is_drawn_as_photo_and_square_until_a_pass_repairs_it()
-> StoreResult<()> {
    let rig = ChatRig::new()?;
    cached_by_an_older_build(&rig);
    let server = Server::new();
    server
        .on(
            "/me",
            r#"{"user": {"id": 7, "username": "anna", "display_name": "Anna"}, "blocked_user_ids": []}"#,
        )
        .on(
            "/chats",
            r#"{"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths"}}]}"#,
        )
        .then(
            "/chats/42/messages",
            &[
                // the catch-up: after_id=1341, nothing newer
                (200, Some(r#"{"messages": []}"#)),
                // before_id=1342&limit=1
                (200, Some(&format!(r#"{{"messages": [{ROUND_JSON}]}}"#))),
                // before_id=1341&limit=1
                (200, Some(&format!(r#"{{"messages": [{STICKER_JSON}]}}"#))),
            ],
        );
    let conversation = open(&rig, &server)?;

    assert_eq!(None, rig.drive(conversation.open())?);
    assert!(server.asked().is_empty());
    let before = conversation.bubbles()?;
    assert!(before[0].message.sticker_picture().is_none());
    assert!(before[1].message.round_video().is_none());
    let marks: Vec<String> = before
        .iter()
        .map(|bubble| media_mark(&bubble.message))
        .collect();
    assert_eq!(vec!["m90photo-", "m91video+"], marks);

    let report = rig.drive(
        Resync::new(
            rig.api(&server),
            rig.chats.clone(),
            Rc::new(BoardStore::new(&rig.cache)),
        )
        .run(),
    )?;
    assert!(report.complete());
    assert_eq!(2, report.flags_repaired);
    assert_eq!(
        vec![
            "/api/v1/me",
            "/api/v1/chats",
            "/api/v1/chats/42/messages?after_id=1341&limit=50",
            "/api/v1/chats/42/messages?before_id=1342&limit=1",
            "/api/v1/chats/42/messages?before_id=1341&limit=1",
        ],
        server.asked()
    );

    let after = conversation.bubbles()?;
    assert_eq!(90, after[0].message.sticker_picture().unwrap().id);
    assert_eq!(91, after[1].message.round_video().unwrap().id);
    assert_eq!(
        vec!["s90", "r91"],
        after
            .iter()
            .map(|bubble| media_mark(&bubble.message))
            .collect::<Vec<_>>()
    );
    Ok(())
}
