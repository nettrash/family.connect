//! The cache an OLDER build wrote — which kept a sticker a grey photo tile and a circle a square,
//! for good, until schema step 8's repair read it again (docs/audio-video-messages-2026-10-04.md,
//! S5.8; iOS `RoundCacheRepairTests`, Android `RoundFlagMigrationTest`): the resync's repair PASS,
//! the four pass tests of `win/tests/FamilyConnect.Core.Tests/Protocol/FlagRepairTests.cs`. (Its
//! wire test is in `protocol_flag_repair_wire.rs`, its store tests in `store_flag_repair.rs`.)

use std::rc::Rc;

use rusqlite::named_params;

use fc_core::protocol::resync::{FLAG_REPAIR_BATCH, Report};
use fc_core::protocol::{
    ApiClient, ApiError, ChatDto, ChatRowDto, FlagRepairOutcome, MemoryTokenStore, Resync,
    ServerUrl, repair_outcome,
};
use fc_core::store::{BoardStore, ChatStore, Database, StoreResult};
use fc_core::testing::{FakeClock, FakeServer, drive};
use time::macros::datetime;

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

const ME: &str =
    r#"{"user": {"id": 7, "username": "anna", "display_name": "Anna"}, "blocked_user_ids": []}"#;

/// What every test holds: the cache, read by 7, with the family chat listed.
fn fixture() -> StoreResult<(Rc<Database>, Rc<ChatStore>)> {
    let database = Database::open_in_memory()?;
    let chats = Rc::new(ChatStore::new(&database).with_me(|| 7));
    chats.replace(
        &[ChatRowDto::new(ChatDto::new(42, "family", "The Smiths"))],
        None,
    )?;
    Ok((database, chats))
}

/// A row exactly as a build from before #58/#79 wrote it: the set decoded into a record that had
/// no `sticker` and no `round`, encoded back with only the fields it knew, IN SEQUENCE — and schema
/// step 8's column at its default, because nothing says which build wrote it.
fn held_by_an_older_build(database: &Database, id: i64, attachments: &str) -> StoreResult<()> {
    database.connection().execute(
        "INSERT INTO messages (message_id, chat_id, sender_id, client_msg_id, body, created_at, attachments_json, sequenced)
VALUES ($id, 42, 9, NULL, '', 0, $attachments, 1)",
        named_params! { "$id": id, "$attachments": attachments },
    )?;
    Ok(())
}

/// One answer for a per-message read: a status and a body (or none), or `None` — no answer at all,
/// a transport failure.
type Single = Box<dyn Fn(i64) -> Option<(u16, Option<String>)>>;

/// A server with no family to read and one chat, whose catch-up has nothing new — and the
/// per-message reads, answered by `single` for the id `before_id - 1` names.
fn pass_server(single: Single) -> FakeServer {
    let server = FakeServer::new();
    server.on(move |path| {
        if path == "/api/v1/me" {
            return Some((200, Some(ME.to_string())));
        }
        if path == "/api/v1/chats" {
            return Some((
                200,
                Some(
                    r#"{"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths"}}]}"#
                        .to_string(),
                ),
            ));
        }
        if path.contains("after_id=") {
            return Some((200, Some(r#"{"messages": []}"#.to_string())));
        }
        let marker = "/messages?before_id=";
        let at = path.find(marker)?;
        if !path.ends_with("&limit=1") {
            return None;
        }
        let digits = &path[at + marker.len()..];
        let before: i64 = digits[..digits.find('&')?].parse().ok()?;
        single(before - 1)
    });
    server
}

struct Repair {
    server: FakeServer,
    clock: FakeClock,
    resync: Resync,
}

impl Repair {
    fn run(&self) -> StoreResult<Report> {
        drive(&self.clock, self.resync.run())
    }

    fn asked_before(&self) -> Vec<String> {
        self.server
            .asked()
            .into_iter()
            .filter(|path| path.contains("before_id"))
            .collect()
    }
}

fn build(database: &Rc<Database>, chats: &Rc<ChatStore>, single: Single) -> Repair {
    let server = pass_server(single);
    let clock = FakeClock::at(datetime!(2026-10-28 12:00 UTC));
    let api = Rc::new(ApiClient::new(
        Rc::new(server.clone()),
        ServerUrl::normalise(Some("chat.example.com")).expect("a server"),
        Rc::new(MemoryTokenStore::new(Some("t0ken"))),
        Rc::new(clock.clone()),
    ));
    let resync = Resync::new(api, chats.clone(), Rc::new(BoardStore::new(database)));
    Repair {
        server,
        clock,
        resync,
    }
}

fn page(messages: &[&str]) -> Option<(u16, Option<String>)> {
    Some((
        200,
        Some(format!(r#"{{"messages": [{}]}}"#, messages.join(","))),
    ))
}

fn candidates(chats: &ChatStore) -> StoreResult<Vec<i64>> {
    Ok(chats
        .flag_repair_candidates(FLAG_REPAIR_BATCH)?
        .iter()
        .map(|message| message.id)
        .collect())
}

/// THE BUG, AND ITS REPAIR. A sticker and a circle an older build cached are a photo and a square
/// video to this one — the record now has the flags, the bytes on disk never did — and the chat is
/// held IN SEQUENCE, so opening it reads nothing and the catch-up only ever asks for what is newer:
/// nothing anywhere would ever read them again. The pass reads each once, by
/// `before_id = id + 1, limit = 1`, and puts the server's set back — and with it the sticker and
/// the circle.
#[test]
fn a_pass_reads_again_what_an_older_build_cached_and_puts_the_flags_back() -> StoreResult<()> {
    let (database, chats) = fixture()?;
    held_by_an_older_build(&database, 1340, OLD_STICKER)?;
    held_by_an_older_build(&database, 1341, OLD_ROUND)?;
    assert!(
        chats
            .message(1340)?
            .expect("held")
            .sticker_picture()
            .is_none()
    );
    assert!(chats.message(1341)?.expect("held").round_video().is_none());
    assert_eq!(chats.catch_up_cursor(42)?, Some(1341));

    let repair = build(
        &database,
        &chats,
        Box::new(|id| match id {
            1340 => page(&[STICKER_JSON]),
            1341 => page(&[ROUND_JSON]),
            _ => None,
        }),
    );
    let report = repair.run()?;

    assert!(report.complete());
    assert_eq!(report.flags_repaired, 2);
    assert_eq!(
        chats
            .message(1340)?
            .expect("held")
            .sticker_picture()
            .map(|picture| picture.id),
        Some(90)
    );
    assert_eq!(
        chats
            .message(1341)?
            .expect("held")
            .round_video()
            .map(|video| video.id),
        Some(91)
    );
    // Newest first, one message each.
    assert_eq!(
        repair.asked_before(),
        [
            "/api/v1/chats/42/messages?before_id=1342&limit=1",
            "/api/v1/chats/42/messages?before_id=1341&limit=1"
        ]
    );
    // The repair moved no cursor and touched nothing else about the rows.
    assert_eq!(chats.catch_up_cursor(42)?, Some(1341));
    assert_eq!(chats.message(1340)?.expect("held").body, "");

    // And once is enough: the next pass asks about neither.
    repair.server.clear_asked();
    assert_eq!(repair.run()?.flags_repaired, 0);
    assert!(repair.asked_before().is_empty());
    Ok(())
}

/// ONE FAILED READ IS ABOUT ONE MESSAGE (iOS's and Android's rules): a refusal and an unreadable
/// answer settle it, a message the server no longer has settles it, a 5xx leaves it for the next
/// pass — and the pass goes on to the others.
#[test]
fn a_refusal_settles_one_message_and_a_5xx_is_asked_again() -> StoreResult<()> {
    let (database, chats) = fixture()?;
    held_by_an_older_build(&database, 10, OLD_ROUND)?; // 403: a chat this device still caches and may no longer read
    held_by_an_older_build(&database, 11, OLD_ROUND)?; // 404: lost
    held_by_an_older_build(&database, 12, OLD_ROUND)?; // 502: no answer yet
    held_by_an_older_build(&database, 13, OLD_ROUND)?; // the server no longer has it: an empty page
    held_by_an_older_build(&database, 14, OLD_ROUND)?; // 200 with a body this build cannot read
    held_by_an_older_build(&database, 1341, OLD_ROUND)?; // fine
    let repair = build(
        &database,
        &chats,
        Box::new(|id| match id {
            10 => Some((
                403,
                Some(r#"{"error": {"code": "forbidden", "message": "no"}}"#.to_string()),
            )),
            11 => Some((
                404,
                Some(r#"{"error": {"code": "not_found", "message": "no"}}"#.to_string()),
            )),
            12 => Some((502, Some("<html>502</html>".to_string()))),
            13 => page(&[]),
            14 => Some((200, Some("not json".to_string()))),
            1341 => page(&[ROUND_JSON]),
            _ => None,
        }),
    );

    let report = repair.run()?;
    assert!(report.complete());
    assert_eq!(report.flags_repaired, 1);
    assert_eq!(
        chats
            .message(1341)?
            .expect("held")
            .round_video()
            .map(|video| video.id),
        Some(91)
    );
    assert_eq!(candidates(&chats)?, [12]);
    // Every one of them was asked, the 5xx one twice (the client's one retry of a read).
    assert_eq!(repair.asked_before().len(), 7);
    Ok(())
}

/// A failure that says nothing about the one message — no network, a lost session, a server asking
/// us to slow down — ENDS the pass: every further read would fail the same way. Nothing is settled,
/// and the pass itself is not failed. (The C# `[Theory]`'s three statuses — 401, 429 and none at
/// all, a transport failure — each on a cache of its own.)
#[test]
fn a_network_or_session_failure_ends_the_pass_and_settles_nothing() -> StoreResult<()> {
    for status in [401_u16, 429, 0] {
        let (database, chats) = fixture()?;
        held_by_an_older_build(&database, 1340, OLD_STICKER)?;
        held_by_an_older_build(&database, 1341, OLD_ROUND)?;
        let repair = build(
            &database,
            &chats,
            Box::new(move |_| match status {
                // The fake has no answer at all: a transport failure.
                0 => None,
                401 => Some((
                    401,
                    Some(r#"{"error": {"code": "unauthorized", "message": "no"}}"#.to_string()),
                )),
                _ => Some((status, None)),
            }),
        );

        let report = repair.run()?;
        assert!(report.complete(), "status {status}");
        assert_eq!(report.flags_repaired, 0, "status {status}");
        // The newest was asked (twice, where the client retries a read once); the older one never
        // was.
        let asked = repair.asked_before();
        assert!(!asked.is_empty(), "status {status}");
        assert!(
            asked
                .iter()
                .all(|path| path == "/api/v1/chats/42/messages?before_id=1342&limit=1"),
            "status {status}: {asked:?}"
        );
        assert_eq!(candidates(&chats)?, [1341, 1340], "status {status}");
    }
    Ok(())
}

/// What one failed read means (the C# `[Theory]`, row by row).
#[test]
fn what_one_failed_read_means() {
    for (code, status, expected) in [
        ("__transport", 0, FlagRepairOutcome::EndPass),
        ("unauthorized", 401, FlagRepairOutcome::EndPass),
        ("too_many_requests", 429, FlagRepairOutcome::EndPass),
        ("", 429, FlagRepairOutcome::EndPass),
        ("internal", 500, FlagRepairOutcome::AskAgainLater),
        ("", 502, FlagRepairOutcome::AskAgainLater),
        ("", 408, FlagRepairOutcome::AskAgainLater),
        ("forbidden", 403, FlagRepairOutcome::Settled),
        ("not_found", 404, FlagRepairOutcome::Settled),
        ("conflict", 409, FlagRepairOutcome::Settled),
        ("validation", 200, FlagRepairOutcome::Settled),
    ] {
        assert_eq!(
            repair_outcome(&ApiError::new(code, "", status)),
            expected,
            "{code:?} {status}"
        );
    }
}
