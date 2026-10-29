//! Stickers and video messages from the server's own bytes — the WIRE half of
//! `win/tests/FamilyConnect.Core.Tests/Protocol/FlagRepairTests.cs`. The store keeping both flags
//! is in `store_flag_repair.rs`; the repair pass over what an older build cached in
//! `protocol_flag_repair_pass.rs`.

use fc_core::protocol::{ChatsResponse, MessageDto, MessagesResponse, ServerFrame, wire};

// What server/src/models.rs serializes: `body` ALWAYS present ("" for a sticker and a circle), a
// real uuid for client_msg_id, BOTH `attachment` and `attachments`, `has_preview` always, and
// each flag only when true — never `"round": false` on a sticker, never `"sticker": false` on a
// circle (server/tests/round_flow.rs, pack_flow.rs).
const STICKER_JSON: &str = r#"
{"id": 1340, "chat_id": 42, "sender_id": 9, "client_msg_id": "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01", "body": "",
 "created_at": "2026-10-05T10:00:00Z",
 "attachment": {"id": 90, "kind": "photo", "mime": "image/webp", "size": 40960, "width": 512, "height": 512,
                "has_preview": false, "sticker": true},
 "attachments": [{"id": 90, "kind": "photo", "mime": "image/webp", "size": 40960, "width": 512, "height": 512,
                  "has_preview": false, "sticker": true}]}
"#;

const ROUND_JSON: &str = r#"
{"id": 1341, "chat_id": 42, "sender_id": 9, "client_msg_id": "4f9e21c0-ceea-4e17-a91c-0d9f8e7b2a01", "body": "",
 "created_at": "2026-10-05T10:01:00Z",
 "attachment": {"id": 91, "kind": "video", "mime": "video/mp4", "size": 812345, "width": 480, "height": 480,
                "duration_ms": 23400, "has_preview": true, "round": true},
 "attachments": [{"id": 91, "kind": "video", "mime": "video/mp4", "size": 812345, "width": 480, "height": 480,
                  "duration_ms": 23400, "has_preview": true, "round": true}]}
"#;

/// A FRESH CACHE IS RIGHT ON EVERY ROUTE: the history page, the live frame, the ack, the edits
/// feed and the chat list's trimmed preview all decode the same struct through the same
/// [`wire`]. (The C# test goes on to put them through the store; that half is the store's.)
#[test]
fn every_route_from_the_server_keeps_both_flags_through_the_store() {
    let sticker: MessageDto = wire::decode(STICKER_JSON).expect("the server's sticker");
    let round: MessageDto = wire::decode(ROUND_JSON).expect("the server's circle");
    assert_eq!(
        sticker.sticker_picture().map(|picture| picture.id),
        Some(90)
    );
    assert_eq!(round.round_video().map(|video| video.id), Some(91));

    let frame = ServerFrame::parse(&format!(
        r#"{{"type": "message", "message": {STICKER_JSON}}}"#
    ));
    let Some(ServerFrame::Message(message)) = frame else {
        panic!("expected a message frame, got {frame:?}");
    };
    assert_eq!(
        message.sticker_picture().map(|picture| picture.id),
        Some(90)
    );
    let ack = ServerFrame::parse(&format!(
        r#"{{"type": "ack", "client_msg_id": "4f9e21c0-ceea-4e17-a91c-0d9f8e7b2a01", "message": {ROUND_JSON}}}"#
    ));
    let Some(ServerFrame::Ack { value, .. }) = ack else {
        panic!("expected an ack, got {ack:?}");
    };
    assert_eq!(value.round_video().map(|video| video.id), Some(91));
    let page: MessagesResponse = wire::decode(&format!(
        r#"{{"messages": [{STICKER_JSON}, {ROUND_JSON}]}}"#
    ))
    .expect("a page");
    let messages = page.messages.expect("the page's messages");
    assert_eq!(
        messages[0].sticker_picture().map(|picture| picture.id),
        Some(90)
    );
    assert_eq!(messages[1].round_video().map(|video| video.id), Some(91));

    // The chat list's preview: no dimensions, the flags kept (handlers_chat.rs's own comment says
    // why).
    let list: ChatsResponse = wire::decode(
        r#"
        {"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths"}, "unread_count": 0,
          "last_message": {"id": 1341, "chat_id": 42, "sender_id": 9, "client_msg_id": "4f9e21c0-ceea-4e17-a91c-0d9f8e7b2a01",
                           "body": "", "created_at": "2026-10-05T10:01:00Z",
                           "attachment": {"id": 91, "kind": "video", "mime": "video/mp4", "size": 812345, "has_preview": true, "round": true},
                           "attachments": [{"id": 91, "kind": "video", "mime": "video/mp4", "size": 812345, "has_preview": true, "round": true}]}}]}
        "#,
    )
    .expect("the chat list");
    let chats = list.chats.expect("the list's rows");
    let preview = chats[0].last_message.as_ref().expect("the preview");
    assert_eq!(preview.round_video().map(|video| video.id), Some(91));
}
