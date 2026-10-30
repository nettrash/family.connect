//! Stickers and video messages through the store, and the cache an OLDER build wrote, which kept a
//! sticker a grey photo tile and a circle a square, for good, until schema step 8's repair read it
//! again (docs/audio-video-messages-2026-10-04.md, S5.8) — the STORE half of
//! `win/tests/FamilyConnect.Core.Tests/Protocol/FlagRepairTests.cs`.
//!
//! The wire half (the decode on every route) is `protocol_flag_repair_wire.rs`. The tests that run
//! a whole `Resync` pass against a scripted server — `APassReadsAgainWhatAnOlderBuildCachedAndPutsTheFlagsBack`,
//! `ARefusalSettlesOneMessageAndA5xxIsAskedAgain`, `ANetworkOrSessionFailureEndsThePassAndSettlesNothing`
//! and `WhatOneFailedReadMeans` — need the API client and the resync, and are ported with them.

use std::rc::Rc;

use fc_core::protocol::resync::FLAG_REPAIR_BATCH;
use fc_core::protocol::{ChatDto, ChatRowDto, ChatsResponse, MessageDto, MessagesResponse, wire};
use fc_core::store::{ChatStore, Database, SeqRoute, StoreResult};
use rusqlite::named_params;

// What server/src/models.rs serializes: `body` ALWAYS present ("" for a sticker and a circle), a
// real uuid for client_msg_id, BOTH `attachment` and `attachments`, `has_preview` always, and each
// flag only when true — never `"round": false` on a sticker, never `"sticker": false` on a circle
// (server/tests/round_flow.rs, pack_flow.rs).
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

const OLD_STICKER: &str = r#"[{"id":90,"kind":"photo","mime":"image/webp","size":40960,"width":512,"height":512,"has_preview":false}]"#;

const OLD_ROUND: &str = r#"[{"id":91,"kind":"video","mime":"video/mp4","size":812345,"width":480,"height":480,"duration_ms":23400,"has_preview":true}]"#;

/// The C# fixture: a cache read by 7, holding the family chat.
fn fixture() -> StoreResult<(Rc<Database>, ChatStore)> {
    let database = Database::open_in_memory()?;
    let chats = ChatStore::new(&database).with_me(|| 7);
    chats.replace(
        &[ChatRowDto::new(ChatDto::new(42, "family", "The Smiths"))],
        None,
    )?;
    Ok((database, chats))
}

/// A row exactly as a build from before #58/#79 wrote it: the set decoded into a record that had no
/// `sticker` and no `round`, encoded back with only the fields it knew, IN SEQUENCE — and schema
/// step 8's column at its default, because nothing says which build wrote it.
fn held_by_an_older_build(
    database: &Database,
    id: i64,
    attachments: Option<&str>,
    body: &str,
) -> StoreResult<()> {
    database.connection().execute(
        "INSERT INTO messages (message_id, chat_id, sender_id, client_msg_id, body, created_at, attachments_json, sequenced)
VALUES ($id, $chat, 9, NULL, $body, 0, $attachments, 1)",
        named_params! { "$id": id, "$chat": 42, "$body": body, "$attachments": attachments },
    )?;
    Ok(())
}

fn candidates(chats: &ChatStore) -> StoreResult<Vec<i64>> {
    Ok(chats
        .flag_repair_candidates(FLAG_REPAIR_BATCH)?
        .iter()
        .map(|message| message.id)
        .collect())
}

/// A FRESH CACHE IS RIGHT ON EVERY ROUTE — the store's half: a history page and the chat list's
/// trimmed preview give both flags back as they were given them, and a set this build wrote is
/// nothing to repair. (The decode of each route is the wire half's.)
#[test]
fn every_route_from_the_server_keeps_both_flags_through_the_store() -> StoreResult<()> {
    let (_database, chats) = fixture()?;
    let page: MessagesResponse = wire::decode(&format!(
        r#"{{"messages": [{STICKER_JSON}, {ROUND_JSON}]}}"#
    ))
    .expect("a page");

    chats.apply_page(
        &page.messages.expect("the page's messages"),
        SeqRoute::Evidence,
        true,
    )?;
    let sticker = chats.message(1340)?.expect("the sticker");
    assert_eq!(
        sticker.sticker_picture().map(|picture| picture.id),
        Some(90)
    );
    let round = chats.message(1341)?.expect("the circle");
    assert_eq!(round.round_video().map(|video| video.id), Some(91));
    // Written by this build, so nothing to repair.
    assert!(chats.flag_repair_candidates(FLAG_REPAIR_BATCH)?.is_empty());

    // The chat list's preview: no dimensions, the flags kept (handlers_chat.rs's own comment says
    // why).
    let other = Database::open_in_memory()?;
    let listed = ChatStore::new(&other).with_me(|| 7);
    let list: ChatsResponse = wire::decode(
        r#"
        {"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths"}, "unread_count": 0,
          "last_message": {"id": 1341, "chat_id": 42, "sender_id": 9, "client_msg_id": "4f9e21c0-ceea-4e17-a91c-0d9f8e7b2a01",
                           "body": "", "created_at": "2026-10-05T10:01:00Z",
                           "attachment": {"id": 91, "kind": "video", "mime": "video/mp4", "size": 812345, "has_preview": true, "round": true},
                           "attachments": [{"id": 91, "kind": "video", "mime": "video/mp4", "size": 812345, "has_preview": true, "round": true}]}}]}
        "#,
    )
    .expect("a chat list");
    listed.replace(&list.chats.expect("the rows"), None)?;
    let preview = listed.message(1341)?.expect("the preview");
    assert_eq!(preview.round_video().map(|video| video.id), Some(91));
    Ok(())
}

/// ONLY WHAT COULD HAVE LOST A FLAG IS ASKED ABOUT. A message with words, an album, a voice note, a
/// file, a message with no attachment and a set that already carries its flag draw the same either
/// way, and are settled with no request; a caption-less photo or video could be a sticker or a
/// circle, and is asked once.
#[test]
fn only_a_bodyless_single_photo_or_video_with_no_flag_is_a_candidate() -> StoreResult<()> {
    let (database, chats) = fixture()?;
    held_by_an_older_build(&database, 1, Some(OLD_STICKER), "look at this")?;
    held_by_an_older_build(
        &database,
        2,
        Some(
            r#"[{"id":1,"kind":"photo","mime":"image/jpeg","size":1,"has_preview":true},{"id":2,"kind":"photo","mime":"image/jpeg","size":1,"has_preview":true}]"#,
        ),
        "",
    )?;
    held_by_an_older_build(
        &database,
        3,
        Some(r#"[{"id":3,"kind":"audio","mime":"audio/mp4","size":1,"has_preview":false}]"#),
        "",
    )?;
    held_by_an_older_build(
        &database,
        4,
        Some(
            r#"[{"id":4,"kind":"file","mime":"application/pdf","size":1,"has_preview":false,"name":"a.pdf"}]"#,
        ),
        "",
    )?;
    held_by_an_older_build(&database, 5, None, "")?;
    held_by_an_older_build(
        &database,
        6,
        Some(
            r#"[{"id":6,"kind":"photo","mime":"image/webp","size":1,"has_preview":false,"sticker":true}]"#,
        ),
        "",
    )?;
    held_by_an_older_build(&database, 7, Some(OLD_STICKER), "")?;
    held_by_an_older_build(&database, 8, Some(OLD_ROUND), "")?;

    assert_eq!(candidates(&chats)?, [8, 7]);
    // Settled for good: asking again finds the same two and nothing else.
    assert_eq!(candidates(&chats)?, [8, 7]);
    Ok(())
}

/// At most a batch per pass, newest first — however many non-candidates lie between them.
#[test]
fn candidates_are_a_batch_newest_first() -> StoreResult<()> {
    let (database, chats) = fixture()?;
    for id in 1..=60 {
        held_by_an_older_build(
            &database,
            id,
            Some(if id % 2 == 0 { OLD_STICKER } else { OLD_ROUND }),
            if id % 3 == 0 { "words" } else { "" },
        )?;
    }
    let expected: Vec<i64> = (1..=60)
        .rev()
        .filter(|id| id % 3 != 0)
        .take(FLAG_REPAIR_BATCH)
        .collect();
    assert_eq!(candidates(&chats)?, expected);
    Ok(())
}

/// Every set this build writes knows the flags, and a copy that brings a set settles a row an older
/// build wrote — so a history page that happens to cover it repairs it without the pass ever
/// asking.
#[test]
fn a_set_this_build_writes_is_never_a_candidate() -> StoreResult<()> {
    let (database, chats) = fixture()?;
    held_by_an_older_build(&database, 1340, Some(OLD_STICKER), "")?;
    let sticker: MessageDto = wire::decode(STICKER_JSON).expect("the server's sticker");
    chats.apply(&sticker, SeqRoute::Evidence, true)?;
    assert_eq!(
        chats
            .message(1340)?
            .expect("the sticker")
            .sticker_picture()
            .map(|picture| picture.id),
        Some(90)
    );
    assert!(chats.flag_repair_candidates(FLAG_REPAIR_BATCH)?.is_empty());

    // A copy with NO set (an edit feed's answer never lacks one, but absent is not empty) leaves the
    // row unknown.
    held_by_an_older_build(&database, 1341, Some(OLD_ROUND), "")?;
    chats.apply(
        &MessageDto::new(1341, 42, 9, None, "", "2026-10-05T10:01:00Z"),
        SeqRoute::Evidence,
        false,
    )?;
    assert_eq!(candidates(&chats)?, [1341]);
    Ok(())
}
