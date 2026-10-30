//! What a client does on every (re)connect, in the protocol's own order — and the two rules in it
//! that only look small (docs/protocol.md, "Best-effort delivery"). A port of
//! `win/tests/FamilyConnect.Core.Tests/Protocol/ResyncTests.cs`.
//!
//! The C# `Server` handler is [`FakeServer`]: it answers by route, first match first, and keeps
//! every path it was asked for. The C# routes that DO something as they answer — apply a live
//! frame, reopen the socket — are closures here too, over the same database (a second `ChatStore`
//! on one file is the same cache) and a slot that holds the pass's own `Resync`.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use fc_core::protocol::resync::Report;
use fc_core::protocol::{
    ApiClient, ApiError, ApiResult, AttachmentDto, ChatDto, ChatRowDto, FrameRouter,
    MemoryTokenStore, MessageDto, MessageResponse, PackItemDto, QueueOptions, Resync,
    RoundVideoLimits, SendPipeline, ServerFrame, ServerUrl, error_codes, wire,
};
use fc_core::store::{
    BoardStore, ChatStore, Database, OutboxRow, OutboxStore, PackLimits, PackStore, SeqRoute,
    StoreResult,
};
use fc_core::testing::{FakeClock, FakeFrameSender, FakeServer, drive};
use time::macros::datetime;

const ME: &str = r#"
{"user": {"id": 7, "username": "anna", "display_name": "Anna"},
 "family": {"id": 3, "name": "The Smiths"}, "role": "owner",
 "blocked_user_ids": []}
"#;

fn family_wall(mark: &str) -> String {
    format!(
        r#"
{{"family": {{"id": 3, "name": "The Smiths"}},
 "members": [{{"id": 7, "username": "anna", "display_name": "Anna", "role": "owner"}}]
 {mark}}}
"#
    )
}

/// A family whose wall has been written on — `max_board_seq` present.
fn family() -> String {
    family_wall(r#", "max_board_seq": 11"#)
}

/// A family whose wall is empty and UNTOUCHED: the mark is absent.
fn family_with_no_wall() -> String {
    family_wall("")
}

fn message_json(id: i64, chat: i64, body: &str) -> String {
    format!(
        r#"{{"id": {id}, "chat_id": {chat}, "sender_id": 7, "client_msg_id": null,
 "body": "{body}", "created_at": "2026-09-12T10:00:00Z"}}"#
    )
}

fn message(id: i64) -> String {
    message_json(id, 42, "hi")
}

fn decoded(json: &str) -> MessageDto {
    wire::decode(json).expect("a message")
}

fn page(messages: impl IntoIterator<Item = String>) -> String {
    format!(
        r#"{{"messages": [{}]}}"#,
        messages.into_iter().collect::<Vec<_>>().join(",")
    )
}

const ONE_CHAT: &str =
    r#"{"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths"}}]}"#;
const TWO_CHATS: &str = r#"
{"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths"}},
           {"chat": {"id": 43, "kind": "direct", "title": "Bob"}}]}
"#;
const EMPTY_WALL: &str = r#"{"notes": [], "max_board_seq": 0}"#;

fn family_rows() -> Vec<ChatRowDto> {
    vec![ChatRowDto::new(ChatDto::new(42, "family", "The Smiths"))]
}

fn two_rows() -> Vec<ChatRowDto> {
    vec![
        ChatRowDto::new(ChatDto::new(42, "family", "The Smiths")),
        ChatRowDto::new(ChatDto::new(43, "direct", "Bob")),
    ]
}

/// What one test builds: a client over the server, the stores, and the pass.
struct Pass {
    server: FakeServer,
    clock: FakeClock,
    chats: Rc<ChatStore>,
    board: Rc<BoardStore>,
    pack: Option<Rc<PackStore>>,
    resync: Rc<Resync>,
}

impl Pass {
    fn run(&self) -> StoreResult<Report> {
        drive(&self.clock, self.resync.run())
    }

    fn asked(&self) -> Vec<String> {
        self.server.asked()
    }

    fn asked_for(&self, path: &str) -> bool {
        self.asked().iter().any(|asked| asked == path)
    }

    fn pack(&self) -> &PackStore {
        self.pack.as_deref().expect("a pass with a pack")
    }
}

fn client(server: &FakeServer, clock: &FakeClock) -> Rc<ApiClient> {
    Rc::new(ApiClient::new(
        Rc::new(server.clone()),
        ServerUrl::normalise(Some("chat.example.com")).expect("a server"),
        Rc::new(MemoryTokenStore::new(Some("t0ken"))),
        Rc::new(clock.clone()),
    ))
}

/// `Build(server)`: a pass with no outbox and no pack, over its own stores on `database`.
fn build(database: &Rc<Database>, server: &FakeServer) -> Pass {
    build_with(database, server, false, None)
}

/// `BuildWithPack(server)`.
fn build_with_pack(database: &Rc<Database>, server: &FakeServer) -> Pass {
    build_with(database, server, true, None)
}

fn build_with(
    database: &Rc<Database>,
    server: &FakeServer,
    with_pack: bool,
    sending: Option<Rc<SendPipeline>>,
) -> Pass {
    let clock = FakeClock::at(datetime!(2026-10-28 12:00 UTC));
    let chats = Rc::new(ChatStore::new(database));
    let board = Rc::new(BoardStore::new(database));
    let pack = with_pack.then(|| Rc::new(PackStore::new(database)));
    let mut resync = Resync::new(client(server, &clock), chats.clone(), board.clone());
    if let Some(pack) = &pack {
        resync = resync.with_pack(pack.clone());
    }
    if let Some(sending) = sending {
        resync = resync.with_sending(sending);
    }
    Pass {
        server: server.clone(),
        clock,
        chats,
        board,
        pack,
        resync: Rc::new(resync),
    }
}

/// Where a route finds the pass it is part of, once there is one.
type Running = Rc<RefCell<Weak<Resync>>>;

fn snapshot(running: &Running) {
    if let Some(resync) = running.borrow().upgrade() {
        resync.snapshot().expect("the cursors");
    }
}

/// A plain `GET` route answering `json` for the paths `matches` accepts.
fn ok(json: String) -> Option<(u16, Option<String>)> {
    Some((200, Some(json)))
}

#[test]
fn the_pass_goes_in_the_documented_order() -> StoreResult<()> {
    let database = Database::open_in_memory()?;
    let server = FakeServer::new();
    server
        .always("/me", ME)
        .always("/families/mine/board/changes", r#"{"notes": []}"#)
        .always("/families/mine/board", EMPTY_WALL)
        .always("/families/mine", &family())
        .always("/chats/42/messages", r#"{"messages": []}"#)
        .always("/chats", ONE_CHAT);
    let pass = build(&database, &server);

    let report = pass.run()?;
    assert!(report.complete());
    assert!(report.signed);
    assert!(report.has_family);
    assert_eq!(report.chats, 1);
    // A chat holding nothing has no hole to fill, and so no message catch-up: it is read from its
    // newest page when it is opened.
    assert_eq!(
        pass.asked(),
        [
            "/api/v1/me",
            "/api/v1/families/mine",
            "/api/v1/chats",
            "/api/v1/families/mine/board",
        ]
    );
    Ok(())
}

#[test]
fn an_account_with_no_family_asks_for_neither_the_roster_nor_the_wall() -> StoreResult<()> {
    let database = Database::open_in_memory()?;
    let server = FakeServer::new();
    server
        .always(
            "/me",
            r#"{"user": {"id": 7, "username": "anna", "display_name": "Anna"}, "blocked_user_ids": []}"#,
        )
        .always("/chats", r#"{"chats": []}"#);
    let pass = build(&database, &server);

    let report = pass.run()?;
    assert!(report.complete());
    assert!(!report.has_family);
    // `GET /families/mine` would answer `not_in_family`, and the wall belongs to a family.
    assert_eq!(pass.asked(), ["/api/v1/me", "/api/v1/chats"]);
    Ok(())
}

/// THE CURSOR BELONGS TO THE LOOP: read once, then advanced by the largest id each page actually
/// returned. Re-reading the store between pages lets a live message arriving mid-loop jump the
/// cursor, and every message between the last page and it is skipped — permanently, because
/// `after_id` can never look back.
#[test]
fn the_message_cursor_follows_the_pages_and_not_the_store() -> StoreResult<()> {
    let database = Database::open_in_memory()?;
    let pages = Rc::new(Cell::new(0));
    let server = FakeServer::new();
    let (counted, live) = (pages.clone(), database.clone());
    server
        .always("/me", ME)
        .always("/families/mine/board", EMPTY_WALL)
        .always("/families/mine", &family())
        .always("/chats", ONE_CHAT)
        .on(move |path| {
            if !path.contains("/chats/42/messages") {
                return None;
            }
            counted.set(counted.get() + 1);
            if counted.get() == 1 {
                // A full page of fifty, ids 2..51 — and, as it is answered, a LIVE message with id
                // 900 lands from the socket that started this resync. The store's max(id) is 900
                // from here on; the loop's cursor must still be 51.
                ChatStore::new(&live)
                    .apply(&decoded(&message(900)), SeqRoute::LiveFrame, true)
                    .expect("the live message");
                return ok(page((2..=51).map(message)));
            }
            // The second page must be asked for from 51 — the largest id page one RETURNED — and
            // not from 900, which is what the store now says.
            if path.contains("after_id=51") {
                ok(page([message(52)]))
            } else {
                ok(r#"{"messages": []}"#.to_string())
            }
        });
    let pass = build(&database, &server);
    pass.chats.replace(&family_rows(), None)?;
    // The one message this device already held, so that there is a cursor to follow.
    pass.chats
        .apply(&decoded(&message(1)), SeqRoute::CatchUpPage, true)?;

    let report = pass.run()?;
    assert!(report.complete());
    assert_eq!(report.messages, 51);
    assert!(pass.asked_for("/api/v1/chats/42/messages?after_id=1&limit=50"));
    assert!(pass.asked_for("/api/v1/chats/42/messages?after_id=51&limit=50"));
    // Nothing was skipped: 52 is held, which is the message the cursor would have jumped over.
    assert!(pass.chats.message(52)?.is_some());
    assert!(pass.chats.message(900)?.is_some());
    Ok(())
}

#[test]
fn the_message_loop_stops_on_a_short_page() -> StoreResult<()> {
    let database = Database::open_in_memory()?;
    let server = FakeServer::new();
    server
        .always("/me", ME)
        .always("/families/mine/board", EMPTY_WALL)
        .always("/families/mine", &family())
        .always("/chats", ONE_CHAT)
        .always("/chats/42/messages", &page([message(2), message(3)]));
    let pass = build(&database, &server);
    pass.chats.replace(&family_rows(), None)?;
    pass.chats
        .apply(&decoded(&message(1)), SeqRoute::CatchUpPage, true)?;

    let report = pass.run()?;
    assert_eq!(report.messages, 2);
    // Two of fifty is short, so one request only.
    assert_eq!(
        pass.asked()
            .iter()
            .filter(|path| path.contains("messages"))
            .count(),
        1
    );
    Ok(())
}

/// THE LIST'S PREVIEW IS NOT WHERE THE CATCH-UP STARTS. Step 2 stores each chat's newest message,
/// trimmed, beside the ones pages delivered; counted in `max(id)` it made `after_id` the server's
/// newest id, the page came back empty, and the Windows client lost a day of a family chat for good
/// (docs/protocol.md, "Best-effort delivery", step 3).
#[test]
fn the_lists_preview_is_not_where_the_catch_up_starts() -> StoreResult<()> {
    let database = Database::open_in_memory()?;
    let server = FakeServer::new();
    let ten = page((11..=20).map(message));
    server
        .always("/me", ME)
        .always("/families/mine/board", EMPTY_WALL)
        .always("/families/mine", &family())
        .always(
            "/chats",
            &format!(
                r#"{{"chats": [{{"chat": {{"id": 42, "kind": "family", "title": "The Smiths"}},
                            "last_message": {}}}]}}"#,
                message(20)
            ),
        )
        .on(move |path| {
            if !path.contains("/chats/42/messages") {
                None
            } else if path.contains("after_id=10&") {
                ok(ten.clone())
            } else {
                ok(r#"{"messages": []}"#.to_string())
            }
        });
    let pass = build(&database, &server);
    pass.chats.replace(&family_rows(), None)?;
    let held: Vec<MessageDto> = (1..=10).map(|id| decoded(&message(id))).collect();
    pass.chats.apply_page(&held, SeqRoute::CatchUpPage, true)?;

    let report = pass.run()?;
    assert!(report.complete());
    assert!(pass.asked_for("/api/v1/chats/42/messages?after_id=10&limit=50"));
    assert_eq!(report.messages, 10);
    for id in 11..=20 {
        assert!(pass.chats.message(id)?.is_some(), "message {id}");
    }

    // Once a page has delivered it, the preview IS held in sequence, and the next pass starts from
    // it.
    pass.server.clear_asked();
    assert!(pass.run()?.complete());
    assert!(pass.asked_for("/api/v1/chats/42/messages?after_id=20&limit=50"));
    Ok(())
}

/// A chat that holds nothing but its preview has no hole to fill, so it has no message catch-up:
/// opening it reads the newest page down, and a request here would page a family's whole history
/// from `after_id=0` on a new device.
#[test]
fn a_chat_holding_only_its_preview_is_left_to_history_paging() -> StoreResult<()> {
    let database = Database::open_in_memory()?;
    let server = FakeServer::new();
    server
        .always("/me", ME)
        .always("/families/mine/board", EMPTY_WALL)
        .always("/families/mine", &family())
        .always(
            "/chats",
            &format!(
                r#"{{"chats": [{{"chat": {{"id": 42, "kind": "family", "title": "The Smiths"}},
                            "last_message": {}}}]}}"#,
                message(20)
            ),
        );
    let pass = build(&database, &server);

    assert!(pass.run()?.complete());
    assert!(!pass.asked().iter().any(|path| path.contains("/messages")));
    // The row still draws itself from it.
    assert_eq!(pass.chats.newest(42)?.map(|newest| newest.id), Some(20));
    assert_eq!(pass.chats.catch_up_cursor(42)?, None);
    Ok(())
}

/// THE CURSORS ARE TAKEN WHEN THE CONNECTION OPENS. The pass reaches the messages several round
/// trips later, and a live message landing in between must not become where it starts: everything
/// missed while the socket was down is below it. A second opening before the pass keeps the
/// EARLIER cursors, and a pass that could not finish hands its own on.
#[test]
fn the_cursors_are_the_ones_taken_when_the_connection_opened() -> StoreResult<()> {
    let database = Database::open_in_memory()?;
    let failing = Rc::new(Cell::new(true));
    let server = FakeServer::new();
    let still_failing = failing.clone();
    server
        .always("/me", ME)
        .always("/families/mine/board", EMPTY_WALL)
        .always("/families/mine", &family())
        .always("/chats", TWO_CHATS)
        .always("/chats/43/messages", r#"{"messages": []}"#)
        .on(move |path| {
            if !path.contains("/chats/42/messages") {
                None
            } else if still_failing.get() {
                Some((500, None))
            } else {
                ok(r#"{"messages": []}"#.to_string())
            }
        });
    let pass = build(&database, &server);
    pass.chats.replace(&two_rows(), None)?;
    let held: Vec<MessageDto> = (1..=10).map(|id| decoded(&message(id))).collect();
    pass.chats.apply_page(&held, SeqRoute::CatchUpPage, true)?;

    // A connection opens at 10; frames on it move the store to 15 — and give chat 43, which held
    // nothing, its first message — and it opens AGAIN before the pass runs. The earlier of the two
    // is kept, per chat...
    pass.resync.snapshot()?;
    let live: Vec<MessageDto> = (11..=15).map(|id| decoded(&message(id))).collect();
    pass.chats.apply_page(&live, SeqRoute::LiveFrame, true)?;
    pass.chats.apply(
        &decoded(&message_json(50, 43, "hi")),
        SeqRoute::LiveFrame,
        true,
    )?;
    pass.resync.snapshot()?;
    // ...and a live message lands before the pass reaches the messages.
    pass.chats
        .apply(&decoded(&message(900)), SeqRoute::LiveFrame, true)?;

    assert!(!pass.run()?.complete());
    assert!(pass.asked_for("/api/v1/chats/42/messages?after_id=10&limit=50"));

    // The pass that could not finish handed its cursors on: the next one does not start from 900
    // either.
    pass.server.clear_asked();
    failing.set(false);
    assert!(pass.run()?.complete());
    assert!(pass.asked_for("/api/v1/chats/42/messages?after_id=10&limit=50"));
    // And the chat only the second opening had a cursor for is caught up from it, not forgotten by
    // the merge.
    assert!(pass.asked_for("/api/v1/chats/43/messages?after_id=50&limit=50"));

    // A pass that finished hands nothing on: the one after it takes its own, from what is held now.
    pass.server.clear_asked();
    assert!(pass.run()?.complete());
    assert!(pass.asked_for("/api/v1/chats/42/messages?after_id=900&limit=50"));
    Ok(())
}

/// The reaction catch-up is asked for only when the chat says there is something to catch up to,
/// and its cursor advances per PAGE — even for messages this device does not hold, whose states are
/// dropped.
#[test]
fn the_reaction_catch_up_runs_only_when_the_chat_says_so_and_advances_per_page() -> StoreResult<()>
{
    let database = Database::open_in_memory()?;
    let server = FakeServer::new();
    server
        .always("/me", ME)
        .always("/families/mine/board", EMPTY_WALL)
        .always("/families/mine", &family())
        .always(
            "/chats",
            r#"{"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths"},
                            "max_reaction_seq": 124}]}"#,
        )
        .always("/chats/42/messages", r#"{"messages": []}"#)
        .always(
            "/chats/42/reactions",
            r#"{"message_reactions": [
                {"message_id": 1338, "reaction_seq": 123, "reactions": [{"user_id": 9, "emoji": "❤️"}]},
                {"message_id": 9999, "reaction_seq": 124, "reactions": [{"user_id": 9, "emoji": "👍"}]}]}"#,
        );
    let pass = build(&database, &server);
    pass.chats.replace(&family_rows(), None)?;
    pass.chats
        .apply(&decoded(&message(1338)), SeqRoute::CatchUpPage, true)?;

    let report = pass.run()?;
    assert_eq!(report.reactions, 2);
    assert!(pass.asked_for("/api/v1/chats/42/reactions?after_seq=0&limit=50"));
    // The state for the message this device holds was applied…
    let reactions = pass
        .chats
        .message(1338)?
        .and_then(|held| held.reactions)
        .expect("reactions");
    assert_eq!(reactions.len(), 1);
    assert_eq!(reactions[0].emoji, "❤️");
    // …the one for a message it has never seen was dropped, and the cursor still moved past both,
    // which is what stops the next pass asking for them again.
    assert_eq!(pass.chats.message(9999)?, None);
    assert_eq!(
        pass.chats.chat(42)?.and_then(|row| row.max_reaction_seq),
        Some(124)
    );

    // And a second pass asks for nothing: the chat's maximum is no longer above the cursor.
    pass.server.clear_asked();
    pass.run()?;
    assert!(!pass.asked().iter().any(|path| path.contains("reactions")));
    Ok(())
}

/// The edit catch-up is the ONLY step that learns of a change to a message this device already
/// holds: step 3's `after_id` is `WHERE id > cursor` and can never look at an older row, so
/// without this feed a device that slept through an edit shows the old words until something else
/// happens to that message.
#[test]
fn the_edit_catch_up_is_the_only_way_an_older_message_is_known_to_have_changed() -> StoreResult<()>
{
    let database = Database::open_in_memory()?;
    let server = FakeServer::new();
    server
        .always("/me", ME)
        .always("/families/mine/board", EMPTY_WALL)
        .always("/families/mine", &family())
        .always(
            "/chats",
            r#"{"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths"},
                            "max_edit_seq": 5}]}"#,
        )
        // Nothing NEWER than what is held — the message that changed is three pages back.
        .always("/chats/42/messages", r#"{"messages": []}"#)
        .always(
            "/chats/42/edits",
            r#"{"messages": [{"id": 10, "chat_id": 42, "sender_id": 7, "body": "Bins on Tuesday",
                               "created_at": "2026-09-12T10:00:00Z",
                               "edited_at": "2026-09-12T11:00:00Z", "edit_seq": 5}]}"#,
        );
    let pass = build(&database, &server);
    pass.chats.replace(&family_rows(), None)?;
    pass.chats.apply(
        &decoded(&message_json(10, 42, "Bins")),
        SeqRoute::CatchUpPage,
        true,
    )?;

    let report = pass.run()?;
    assert_eq!(report.edits, 1);
    assert!(pass.asked_for("/api/v1/chats/42/edits?after_seq=0&limit=50"));
    // The whole message was applied, through the path a page of history goes through.
    assert_eq!(
        pass.chats.message(10)?.map(|held| held.body).as_deref(),
        Some("Bins on Tuesday")
    );
    assert_eq!(
        pass.chats.chat(42)?.and_then(|row| row.max_edit_seq),
        Some(5)
    );

    // And the next pass asks for nothing: the chat's maximum is no longer above the cursor.
    pass.server.clear_asked();
    pass.run()?;
    assert!(!pass.asked().iter().any(|path| path.contains("edits")));
    Ok(())
}

/// A connection that opens WHILE a pass is failing and the pass's own starting cursors are MERGED,
/// per chat, the earlier kept — neither replaces the other. Chat 42's pass started from 10 although
/// frame 900 was already held above a gap: the reconnection's 900 alone would skip that gap. Chat
/// 43 held nothing when the pass began; a frame gave it 50 before the socket dropped, and the
/// reconnection took that: the pass's cursors alone would skip the chat, and everything sent to it
/// while the socket was down would sit for good below the frames that came after.
#[test]
fn a_connection_that_opens_during_a_failed_pass_is_not_forgotten() -> StoreResult<()> {
    let database = Database::open_in_memory()?;
    let failing = Rc::new(Cell::new(true));
    let running: Running = Rc::new(RefCell::new(Weak::new()));
    let server = FakeServer::new();
    let (still_failing, reopening, live) = (failing.clone(), running.clone(), database.clone());
    server
        .always("/me", ME)
        .always("/families/mine/board", EMPTY_WALL)
        .always("/families/mine", &family())
        .always("/chats", TWO_CHATS)
        .always("/chats/43/messages", r#"{"messages": []}"#)
        .on(move |path| {
            if !path.contains("/chats/42/messages") {
                return None;
            }
            if !still_failing.get() {
                return ok(r#"{"messages": []}"#.to_string());
            }
            // Mid-pass: a frame for chat 43, the socket drops and opens again — and then this read
            // fails.
            ChatStore::new(&live)
                .apply(
                    &decoded(&message_json(50, 43, "hi")),
                    SeqRoute::LiveFrame,
                    true,
                )
                .expect("the live message");
            snapshot(&reopening);
            Some((500, None))
        });
    let pass = build(&database, &server);
    *running.borrow_mut() = Rc::downgrade(&pass.resync);
    pass.chats.replace(&two_rows(), None)?;
    pass.chats
        .apply(&decoded(&message(10)), SeqRoute::CatchUpPage, true)?;
    // A connection opens at 10, and a live message lands on it before the pass.
    pass.resync.snapshot()?;
    pass.chats
        .apply(&decoded(&message(900)), SeqRoute::LiveFrame, true)?;

    assert!(!pass.run()?.complete());
    // A frame on the new connection, before the next pass.
    pass.chats.apply(
        &decoded(&message_json(60, 43, "hi")),
        SeqRoute::LiveFrame,
        true,
    )?;

    pass.server.clear_asked();
    failing.set(false);
    assert!(pass.run()?.complete());
    assert!(pass.asked_for("/api/v1/chats/42/messages?after_id=10&limit=50"));
    assert!(pass.asked_for("/api/v1/chats/43/messages?after_id=50&limit=50"));
    Ok(())
}

/// Cursors a connection handed over are held to what the cache still holds: a cache wiped since — a
/// sign-out, another account, another server — is never paged from a stranger's cursor, past what
/// its own messages reach.
#[test]
fn handed_over_cursors_are_held_to_what_the_cache_still_holds() -> StoreResult<()> {
    let database = Database::open_in_memory()?;
    let server = FakeServer::new();
    server
        .always("/me", ME)
        .always("/families/mine/board", EMPTY_WALL)
        .always("/families/mine", &family())
        .always("/chats", TWO_CHATS)
        .always("/chats/42/messages", r#"{"messages": []}"#)
        .always("/chats/43/messages", r#"{"messages": []}"#);
    let pass = build(&database, &server);
    let rows = two_rows();
    pass.chats.replace(&rows, None)?;
    pass.chats.apply_page(
        &[
            decoded(&message(1200)),
            decoded(&message_json(70, 43, "hi")),
        ],
        SeqRoute::CatchUpPage,
        true,
    )?;
    pass.resync.snapshot()?;

    // The cache is wiped; the next copy of chat 42 reaches only 1000, and of chat 43 it holds
    // nothing.
    database.wipe_all()?;
    pass.chats.replace(&rows, None)?;
    pass.chats
        .apply(&decoded(&message(1000)), SeqRoute::CatchUpPage, true)?;

    assert!(pass.run()?.complete());
    assert!(pass.asked_for("/api/v1/chats/42/messages?after_id=1000&limit=50"));
    assert!(
        !pass
            .asked()
            .iter()
            .any(|path| path.contains("/chats/43/messages"))
    );
    Ok(())
}

/// A copy from the edits feed is held and drawn, and it is not where the next catch-up starts: an
/// edited message may be one this device never held, and it says nothing about the messages before
/// it (docs/protocol.md, "Best-effort delivery", step 3).
#[test]
fn an_edited_copy_is_not_where_the_next_catch_up_starts() -> StoreResult<()> {
    let database = Database::open_in_memory()?;
    let server = FakeServer::new();
    server
        .always("/me", ME)
        .always("/families/mine/board", EMPTY_WALL)
        .always("/families/mine", &family())
        .always(
            "/chats",
            r#"{"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths"},
                            "max_edit_seq": 5}]}"#,
        )
        // A server whose pages have not caught up with its edits feed: whatever the reason, the
        // copy the feed carries must not move the cursor past what the pages have not delivered.
        .always("/chats/42/messages", r#"{"messages": []}"#)
        .always(
            "/chats/42/edits",
            r#"{"messages": [{"id": 30, "chat_id": 42, "sender_id": 9, "body": "Bins on Tuesday",
                               "created_at": "2026-09-12T10:00:00Z",
                               "edited_at": "2026-09-12T11:00:00Z", "edit_seq": 5}]}"#,
        );
    let pass = build(&database, &server);
    pass.chats.replace(&family_rows(), None)?;
    pass.chats
        .apply(&decoded(&message(10)), SeqRoute::CatchUpPage, true)?;

    assert!(pass.run()?.complete());
    assert_eq!(
        pass.chats.message(30)?.map(|held| held.body).as_deref(),
        Some("Bins on Tuesday")
    );
    assert_eq!(pass.chats.catch_up_cursor(42)?, Some(10));
    Ok(())
}

/// The messages catch-up delivers only what is newer than everything held, so a reply on it raises
/// its root; the edits catch-up delivers copies the root's recomputed count already includes, and
/// raises nothing (docs/protocol.md, "Threads").
#[test]
fn only_the_messages_catch_up_raises_a_roots_count() -> StoreResult<()> {
    let database = Database::open_in_memory()?;
    let server = FakeServer::new();
    server
        .always("/me", ME)
        .always("/families/mine/board", EMPTY_WALL)
        .always("/families/mine", &family())
        .always(
            "/chats",
            r#"{"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths"},
                            "max_edit_seq": 5}]}"#,
        )
        .always(
            "/chats/42/messages",
            r#"{"messages": [{"id": 12, "chat_id": 42, "sender_id": 9, "body": "at 8",
                               "created_at": "2026-09-12T10:02:00Z", "thread_root_id": 9}]}"#,
        )
        .always(
            "/chats/42/edits",
            r#"{"messages": [{"id": 11, "chat_id": 42, "sender_id": 9, "body": "at 7!",
                               "created_at": "2026-09-12T10:01:00Z", "thread_root_id": 9,
                               "edited_at": "2026-09-12T11:00:00Z", "edit_seq": 5}]}"#,
        );
    let pass = build(&database, &server);
    pass.chats.replace(&family_rows(), None)?;
    pass.chats.apply(
        &MessageDto {
            reply_count: Some(1),
            ..decoded(&message_json(9, 42, "Dinner?"))
        },
        SeqRoute::Evidence,
        true,
    )?;

    pass.run()?;

    assert_eq!(
        pass.chats.message(11)?.map(|held| held.body).as_deref(),
        Some("at 7!")
    );
    assert_eq!(
        pass.chats.message(9)?.and_then(|held| held.reply_count),
        Some(2)
    );
    Ok(())
}

/// The three feeds are asked for in the protocol's own order, after the messages and before the
/// wall — and each only when its own mark is above its own cursor.
#[test]
fn the_three_feeds_follow_the_messages_in_the_documented_order() -> StoreResult<()> {
    let database = Database::open_in_memory()?;
    let server = FakeServer::new();
    server
        .always("/me", ME)
        .always("/families/mine/board", EMPTY_WALL)
        .always("/families/mine", &family())
        .always(
            "/chats",
            r#"{"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths"},
                            "max_reaction_seq": 1, "max_edit_seq": 2, "max_poll_seq": 3}]}"#,
        )
        .always("/chats/42/messages", r#"{"messages": []}"#)
        .always("/chats/42/reactions", r#"{"message_reactions": []}"#)
        .always("/chats/42/edits", r#"{"messages": []}"#)
        .always("/chats/42/polls", r#"{"polls": []}"#);
    let pass = build(&database, &server);
    pass.chats.replace(&family_rows(), None)?;
    pass.chats
        .apply(&decoded(&message(1)), SeqRoute::CatchUpPage, true)?;

    assert!(pass.run()?.complete());
    assert_eq!(
        pass.asked(),
        [
            "/api/v1/me",
            "/api/v1/families/mine",
            "/api/v1/chats",
            "/api/v1/chats/42/messages?after_id=1&limit=50",
            "/api/v1/chats/42/reactions?after_seq=0&limit=50",
            "/api/v1/chats/42/edits?after_seq=0&limit=50",
            "/api/v1/chats/42/polls?after_seq=0&limit=50",
            "/api/v1/families/mine/board",
        ]
    );
    Ok(())
}

/// A FULL PAGE THAT CANNOT MOVE THE CURSOR ENDS THE LOOP. Every feed answers strictly newer rows,
/// so this cannot happen against a server keeping its word — and "cannot happen" is exactly the
/// shape of a loop that asks the same question until the process dies.
#[test]
fn a_page_that_cannot_advance_the_cursor_stops_the_loop_rather_than_spinning_it() -> StoreResult<()>
{
    let database = Database::open_in_memory()?;
    let pages = Rc::new(Cell::new(0));
    // Fifty states, every one of them at seq 7: a full page, and one that says nothing new after
    // the first. The fake gives up after six so that a client which does not stop fails this test
    // instead of hanging it.
    let fifty = (1..=50)
        .map(|id| format!(r#"{{"message_id": {id}, "reaction_seq": 7, "reactions": []}}"#))
        .collect::<Vec<_>>()
        .join(",");
    let server = FakeServer::new();
    let counted = pages.clone();
    server
        .always("/me", ME)
        .always("/families/mine/board", EMPTY_WALL)
        .always("/families/mine", &family())
        .always(
            "/chats",
            r#"{"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths"},
                            "max_reaction_seq": 9}]}"#,
        )
        .always("/chats/42/messages", r#"{"messages": []}"#)
        .on(move |path| {
            if !path.contains("/reactions") {
                return None;
            }
            counted.set(counted.get() + 1);
            if counted.get() > 6 {
                ok(r#"{"message_reactions": []}"#.to_string())
            } else {
                ok(format!(r#"{{"message_reactions": [{fifty}]}}"#))
            }
        });
    let pass = build(&database, &server);

    let report = pass.run()?;
    assert!(report.complete());
    // Twice: once from 0, once from 7 — and the second answer moved nothing, so that is that.
    assert_eq!(pages.get(), 2);
    assert_eq!(
        pass.chats.chat(42)?.and_then(|row| row.max_reaction_seq),
        Some(7)
    );
    Ok(())
}

#[test]
fn the_wall_is_read_whole_once_and_then_as_changes() -> StoreResult<()> {
    let database = Database::open_in_memory()?;
    let server = FakeServer::new();
    server
        .always("/me", ME)
        .always(
            "/families/mine/board/changes",
            r#"{"notes": [{"id": 13, "author_id": 7, "text": "Bins", "board_seq": 12}]}"#,
        )
        .always(
            "/families/mine/board",
            r#"{"notes": [{"id": 12, "author_id": 7, "text": "Milk", "board_seq": 11}],
                 "max_board_seq": 11}"#,
        )
        .always("/families/mine", &family_wall(r#", "max_board_seq": 12"#))
        .always("/chats", r#"{"chats": []}"#);
    let pass = build(&database, &server);

    let first = pass.run()?;
    assert_eq!(first.notes, 1);
    assert_eq!(pass.board.cursor()?, 11);
    assert!(pass.asked_for("/api/v1/families/mine/board"));

    // With a cursor in hand the next pass asks for CHANGES, from where it left off.
    pass.server.clear_asked();
    let second = pass.run()?;
    assert_eq!(second.notes, 1);
    assert!(pass.asked_for("/api/v1/families/mine/board/changes?after_seq=11&limit=50"));
    assert_eq!(pass.board.cursor()?, 12);
    assert_eq!(pass.board.notes()?.len(), 2);
    Ok(())
}

/// The block list is read in FULL on every pass, from both reads that carry it, and stored as
/// complete state — there is no catch-up feed for it and so nothing to miss. An absent list means
/// nobody, which is the one read where absence is not "leave what you hold alone".
#[test]
fn the_block_list_is_read_in_full_on_every_pass() -> StoreResult<()> {
    let database = Database::open_in_memory()?;
    let blocking_me = r#"
        {"user": {"id": 7, "username": "anna", "display_name": "Anna"},
         "family": {"id": 3, "name": "The Smiths"}, "role": "owner",
         "blocked_user_ids": [11, 14]}"#;
    let server = FakeServer::new();
    server
        .always("/me", blocking_me)
        .always("/families/mine", &family_with_no_wall())
        .always("/chats", r#"{"chats": []}"#);
    let pass = build(&database, &server);
    pass.chats.replace_blocked(&[99])?;

    assert!(pass.run()?.complete());
    // Replaced whole: the id this device held and the server no longer names is gone.
    assert_eq!(&*pass.chats.blocked()?, [11, 14]);
    assert!(!pass.chats.is_blocked(99)?);

    // The family's own document carries the same list, and when it is THERE it applies — when it
    // is not, it is an older server and not an empty list, so the value `/me` just gave stands
    // rather than being cleared by a document that never mentioned it.
    let loud = FakeServer::new();
    loud.always("/me", blocking_me)
        .always(
            "/families/mine",
            &family_wall(r#", "blocked_user_ids": [14]"#),
        )
        .always("/chats", r#"{"chats": []}"#)
        .always("/families/mine/board", EMPTY_WALL);
    let again = build(&database, &loud);
    assert!(again.run()?.complete());
    assert_eq!(&*again.chats.blocked()?, [14]);
    Ok(())
}

/// `max_board_seq` is the one published mark for the wall, and it is ABSENT while the wall is empty
/// and untouched — which is what it is for: a client reads it to know whether a board request is
/// worth making. Level with it, or no mark at all, and nothing is asked for.
#[test]
fn a_wall_nobody_has_touched_is_not_asked_for_and_neither_is_one_already_level() -> StoreResult<()>
{
    let database = Database::open_in_memory()?;
    let server = FakeServer::new();
    server
        .always("/me", ME)
        .always("/families/mine", &family_with_no_wall())
        .always("/chats", r#"{"chats": []}"#);
    let pass = build(&database, &server);

    // No mark, nothing held: no request at all — and there is no route for one, so a client that
    // asked would fail this test rather than quietly cost a round trip.
    assert!(pass.run()?.complete());
    assert_eq!(
        pass.asked(),
        ["/api/v1/me", "/api/v1/families/mine", "/api/v1/chats"]
    );

    // Now the same family with a wall, read once…
    let wall = FakeServer::new();
    wall.always("/me", ME)
        .always("/families/mine", &family())
        .always("/chats", r#"{"chats": []}"#)
        .always(
            "/families/mine/board",
            r#"{"notes": [{"id": 12, "author_id": 7, "text": "Milk", "board_seq": 11}],
                 "max_board_seq": 11}"#,
        );
    let again = build(&database, &wall);
    assert_eq!(again.run()?.notes, 1);
    assert_eq!(again.board.cursor()?, 11);

    // …and a second pass asks for neither the wall nor its changes: the mark it just read says this
    // device has applied everything there has been.
    again.server.clear_asked();
    assert!(again.run()?.complete());
    assert!(!again.asked().iter().any(|path| path.contains("board")));
    Ok(())
}

/// THE FLUSH IS NOT A STEP. A client that could not even sign in must still flush what it holds:
/// putting the one operation that recovers a stuck message behind the reads most likely to fail on
/// the network that stuck it is exactly the bug both of this app's ports shipped.
#[test]
fn the_outbox_is_flushed_even_when_every_read_fails() -> StoreResult<()> {
    let database = Database::open_in_memory()?;
    let server = FakeServer::new();
    server.on(|_| Some((502, Some("<html>502</html>".to_string()))));
    let clock = FakeClock::at(datetime!(2026-10-28 12:00 UTC));
    let chats = Rc::new(ChatStore::new(&database));
    let board = Rc::new(BoardStore::new(&database));
    let outbox = Rc::new(OutboxStore::new(&database));
    let posts = Rc::new(Cell::new(0));
    let counted = posts.clone();
    // A socket that never connected.
    let socket = FakeFrameSender::new();
    socket.set_connected(false);
    let pipeline = Rc::new(SendPipeline::new(
        Rc::new(socket),
        outbox.clone(),
        chats.clone(),
        Rc::new(move |_row: OutboxRow| {
            counted.set(counted.get() + 1);
            Box::pin(async {
                ApiResult::<MessageResponse>::failure(ApiError::transport("still down"))
            })
        }),
        Rc::new(clock.clone()),
    ));
    chats.replace(&family_rows(), None)?;
    pipeline.enqueue(42, "Dinner at 7?", &QueueOptions::default())?;

    let resync = Resync::new(client(&server, &clock), chats, board).with_sending(pipeline);
    let report = drive(&clock, resync.run())?;

    // The reads got nowhere…
    assert!(!report.complete());
    assert!(!report.signed);
    // …and the flush happened anyway, first.
    assert!(report.flushed);
    assert_eq!(posts.get(), 1);
    // The row is still queued, not failed: one transient attempt is not a refusal.
    let rows = outbox.all()?;
    assert_eq!(rows.len(), 1);
    assert!(!rows[0].failed());
    Ok(())
}

/// A 200 is not a promise that the body is the documented body: a rewriting proxy or a moved path
/// answers readable JSON with no `messages` key at all. That is an empty page — nothing to apply —
/// and not a crash, which is what it was before every one of these lists was read with `?? []`.
#[test]
fn an_answer_with_no_list_at_all_is_an_empty_page_and_not_a_crash() -> StoreResult<()> {
    let database = Database::open_in_memory()?;
    let server = FakeServer::new();
    server
        .always("/me", ME)
        .always("/families/mine/board", r#"{"max_board_seq": 0}"#)
        .always("/families/mine", &family())
        .always("/chats", ONE_CHAT)
        .always("/chats/42/messages", "{}");
    let pass = build(&database, &server);

    let report = pass.run()?;
    assert!(report.complete());
    assert_eq!(report.messages, 0);
    assert_eq!(report.notes, 0);
    assert!(pass.board.notes()?.is_empty());
    Ok(())
}

#[test]
fn a_read_that_fails_stops_the_reads_and_says_where() -> StoreResult<()> {
    let database = Database::open_in_memory()?;
    let server = FakeServer::new();
    server
        .always("/me", ME)
        .always("/families/mine", &family())
        .always_with(
            "/chats",
            r#"{"error": {"code": "internal", "message": "no"}}"#,
            500,
        );
    let pass = build(&database, &server);

    let report = pass.run()?;
    assert!(!report.complete());
    assert!(report.signed);
    let stopped = report.stopped.expect("where it stopped");
    assert_eq!(stopped.code, error_codes::INTERNAL);
    // Transient, so the caller knows to come back rather than to show a failure.
    assert!(stopped.transient());
    Ok(())
}

// ---- the sticker pack (docs/protocol.md, "Sticker pack") -------------------------------------
//
// The board's catch-up, one table over: a device that holds no pack reads the whole of it, one
// that does and is behind loops the change feed, and one that is level asks for nothing.

const CEILINGS: &str = r#", "max_pack_items": 200, "max_pack_item_bytes": 524288"#;

/// A family whose pack has been written to: the mark and both ceilings.
fn family_with_pack(mark: i64) -> String {
    family_wall(&format!(r#", "max_pack_seq": {mark}{CEILINGS}"#))
}

fn pack_item_json(id: i64, seq: i64) -> String {
    format!(
        r#"{{"id": {id}, "added_by": 7,
 "attachment": {{"id": {}, "kind": "photo", "mime": "image/webp", "size": 4096, "width": 512, "height": 512}},
 "created_at": "2026-09-13T10:00:00Z", "pack_seq": {seq}}}"#,
        70 + id
    )
}

fn pack_item(id: i64, seq: i64) -> PackItemDto {
    PackItemDto {
        added_by: 7,
        attachment: Some(AttachmentDto {
            mime: Some("image/webp".to_string()),
            size: Some(4096),
            width: Some(512),
            height: Some(512),
            ..AttachmentDto::new(70 + id, "photo")
        }),
        created_at: Some("2026-09-13T10:00:00Z".to_string()),
        pack_seq: seq,
        ..PackItemDto::new(id)
    }
}

fn pack_server(family: &str) -> FakeServer {
    let server = FakeServer::new();
    server
        .always("/me", ME)
        .always("/families/mine", family)
        .always("/chats", r#"{"chats": []}"#);
    server
}

fn item_ids(pack: &PackStore) -> StoreResult<Vec<i64>> {
    Ok(pack.items()?.iter().map(|item| item.id).collect())
}

fn last(asked: &[String]) -> &str {
    asked.last().map_or("", String::as_str)
}

/// A frame off the socket, through a router of its own over the same cache.
fn hear_pack_frame(database: &Rc<Database>, pack: &Rc<PackStore>, item: PackItemDto) {
    FrameRouter::new(
        Rc::new(ChatStore::new(database)),
        Rc::new(BoardStore::new(database)),
        Some(pack.clone()),
    )
    .hear(&ServerFrame::PackItem(item))
    .expect("the frame");
}

#[test]
fn a_device_that_holds_no_pack_reads_the_whole_of_it() -> StoreResult<()> {
    let database = Database::open_in_memory()?;
    let server = pack_server(&family_with_pack(14));
    server.always(
        "/families/mine/pack",
        &format!(
            r#"{{"items": [{}, {}], "max_pack_seq": 14}}"#,
            pack_item_json(5, 12),
            pack_item_json(6, 14)
        ),
    );
    let pass = build_with_pack(&database, &server);

    let report = pass.run()?;

    assert!(report.complete());
    assert_eq!(report.pack_items, 2);
    assert_eq!(item_ids(pass.pack())?, [5, 6]);
    assert_eq!(pass.pack().cursor()?, 14);
    assert!(pass.pack().is_caught_up());
    assert_eq!(
        pass.pack().limits()?,
        Some(PackLimits {
            max_items: 200,
            max_item_bytes: 524_288
        })
    );
    // After `GET /families/mine`, and the full read only — never the change feed from zero.
    assert_eq!(
        pass.asked(),
        [
            "/api/v1/me",
            "/api/v1/families/mine",
            "/api/v1/chats",
            "/api/v1/families/mine/pack"
        ]
    );
    Ok(())
}

/// A device that HOLDS a pack and is behind loops the change feed from its own cursor — and the
/// feed carries TOMBSTONES, which is how a sticker removed while it slept leaves the panel.
#[test]
fn a_device_behind_loops_the_change_feed_and_applies_its_tombstones() -> StoreResult<()> {
    let database = Database::open_in_memory()?;
    let server = pack_server(&family_with_pack(16));
    server.always(
        "/families/mine/pack/changes",
        &format!(
            r#"{{"items": [{}, {{"id": 5, "deleted": true, "pack_seq": 16}}]}}"#,
            pack_item_json(7, 15)
        ),
    );
    let pass = build_with_pack(&database, &server);
    pass.pack()
        .replace(&[pack_item(5, 12), pack_item(6, 14)], 14)?;

    let report = pass.run()?;

    assert!(report.complete());
    assert_eq!(report.pack_items, 2);
    assert_eq!(item_ids(pass.pack())?, [6, 7]);
    assert_eq!(pass.pack().cursor()?, 16);
    assert!(pass.pack().is_caught_up());
    assert_eq!(
        last(&pass.asked()),
        "/api/v1/families/mine/pack/changes?after_seq=14&limit=50"
    );
    assert!(!pass.asked_for("/api/v1/families/mine/pack"));
    Ok(())
}

#[test]
fn the_change_feed_is_looped_until_a_short_page() -> StoreResult<()> {
    let database = Database::open_in_memory()?;
    let pages = Rc::new(RefCell::new(Vec::<String>::new()));
    let server = pack_server(&family_with_pack(80));
    let asked = pages.clone();
    server.on(move |path| {
        if !path.starts_with("/api/v1/families/mine/pack/changes") {
            return None;
        }
        asked.borrow_mut().push(path.to_string());
        // A full page of fifty (seqs 15..64), then a short one (65..80).
        let first = asked.borrow().len() == 1;
        let (from, to) = if first { (15, 64) } else { (65, 80) };
        let items = (from..=to)
            .map(|seq| pack_item_json(100 + seq, seq))
            .collect::<Vec<_>>()
            .join(",");
        ok(format!(r#"{{"items": [{items}]}}"#))
    });
    let pass = build_with_pack(&database, &server);
    pass.pack().replace(&[pack_item(5, 14)], 14)?;

    let report = pass.run()?;

    assert_eq!(report.pack_items, 66);
    assert_eq!(
        *pages.borrow(),
        [
            "/api/v1/families/mine/pack/changes?after_seq=14&limit=50",
            "/api/v1/families/mine/pack/changes?after_seq=64&limit=50",
        ]
    );
    assert_eq!(pass.pack().cursor()?, 80);
    assert_eq!(pass.pack().count()?, 67);
    Ok(())
}

#[test]
fn a_device_level_with_the_server_asks_for_nothing() -> StoreResult<()> {
    let database = Database::open_in_memory()?;
    let pass = build_with_pack(&database, &pack_server(&family_with_pack(14)));
    pass.pack().replace(&[pack_item(5, 14)], 14)?;

    let report = pass.run()?;

    assert!(report.complete());
    assert_eq!(report.pack_items, 0);
    assert_eq!(
        pass.asked(),
        ["/api/v1/me", "/api/v1/families/mine", "/api/v1/chats"]
    );
    // Level IS caught up: frames may move the cursor from here.
    assert!(pass.pack().is_caught_up());
    Ok(())
}

/// `max_pack_seq` is OMITTED while the pack has never been written to: there is nothing to read,
/// so nothing is asked for — but the ceilings are there, and the button is offered.
#[test]
fn an_untouched_pack_is_not_read_and_still_has_its_ceilings() -> StoreResult<()> {
    let database = Database::open_in_memory()?;
    let pass = build_with_pack(&database, &pack_server(&family_wall(CEILINGS)));

    let report = pass.run()?;

    assert!(report.complete());
    assert_eq!(
        pass.asked(),
        ["/api/v1/me", "/api/v1/families/mine", "/api/v1/chats"]
    );
    assert_eq!(
        pass.pack().limits()?,
        Some(PackLimits {
            max_items: 200,
            max_item_bytes: 524_288
        })
    );
    assert!(pass.pack().is_caught_up());
    Ok(())
}

/// A SERVER THAT PREDATES THE PACK names no ceilings, and that absence is the whole signal: no pack
/// request is made — it would be a 404 — and a ceiling remembered from before is forgotten, so no
/// sticker button is offered there.
#[test]
fn a_server_that_predates_packs_is_asked_for_none_and_offers_none() -> StoreResult<()> {
    let database = Database::open_in_memory()?;
    // Even with a mark on the document: without the ceilings this is not a server with packs.
    let pass = build_with_pack(
        &database,
        &pack_server(&family_wall(r#", "max_pack_seq": 14"#)),
    );
    pass.pack().set_limits(Some(PackLimits {
        max_items: 200,
        max_item_bytes: 524_288,
    }))?;

    let report = pass.run()?;

    assert!(report.complete());
    assert_eq!(pass.pack().limits()?, None);
    assert_eq!(
        pass.asked(),
        ["/api/v1/me", "/api/v1/families/mine", "/api/v1/chats"]
    );
    Ok(())
}

/// A FRAME MAY NOT JUMP THE CATCH-UP. A connection opens, a live `pack_item` lands before the pass
/// reaches the pack, and the pass must still read everything between the cursor this device held
/// and that frame — which it can only do if the frame did not move the cursor.
#[test]
fn a_frame_that_lands_before_the_pass_does_not_skip_what_was_missed() -> StoreResult<()> {
    let database = Database::open_in_memory()?;
    let server = pack_server(&family_with_pack(20));
    server.always(
        "/families/mine/pack/changes",
        &format!(
            r#"{{"items": [{}, {}, {}]}}"#,
            pack_item_json(7, 15),
            pack_item_json(8, 17),
            pack_item_json(9, 20)
        ),
    );
    let pass = build_with_pack(&database, &server);
    let pack = pass.pack.clone().expect("a pack");
    pack.replace(&[pack_item(5, 14)], 14)?;
    pack.caught_up(pack.connection());

    // The connection opens…
    pass.resync.snapshot()?;
    // …and the newest add arrives on it before the pass has asked for anything.
    hear_pack_frame(&database, &pack, pack_item(9, 20));
    assert_eq!(pack.cursor()?, 14);

    pass.run()?;

    assert_eq!(
        last(&pass.asked()),
        "/api/v1/families/mine/pack/changes?after_seq=14&limit=50"
    );
    assert_eq!(item_ids(&pack)?, [5, 7, 8, 9]);
    assert_eq!(pack.cursor()?, 20);
    Ok(())
}

/// A PASS CATCHES UP THE CONNECTION IT BEGAN ON, AND NO OTHER. The socket drops and reopens while
/// a pass is waiting on the change feed; a change is committed after the server answered that
/// request and before the new socket was listening, so it is on neither. The overtaken pass may
/// not call the NEW connection caught up — if it did, the next frame would step the cursor over
/// that change, the following pass would find itself level with the server's mark and ask for
/// nothing, and the change would be lost until the cache was wiped.
#[test]
fn a_pass_a_reconnect_overtook_does_not_catch_up_the_new_connection() -> StoreResult<()> {
    let database = Database::open_in_memory()?;
    let mark = Rc::new(Cell::new(19));
    let running: Running = Rc::new(RefCell::new(Weak::new()));
    let server = FakeServer::new();
    let (marked, reopening) = (mark.clone(), running.clone());
    server
        .always("/me", ME)
        .on(move |path| {
            (path == "/api/v1/families/mine").then(|| (200, Some(family_with_pack(marked.get()))))
        })
        .always("/chats", r#"{"chats": []}"#)
        .on(move |path| {
            if path == "/api/v1/families/mine/pack/changes?after_seq=14&limit=50" {
                // The answer is taken here, at 19 — and the socket reopens before it is applied.
                snapshot(&reopening);
                return ok(format!(
                    r#"{{"items": [{}, {}]}}"#,
                    pack_item_json(7, 15),
                    pack_item_json(6, 19)
                ));
            }
            (path == "/api/v1/families/mine/pack/changes?after_seq=19&limit=50").then(|| {
                (
                    200,
                    Some(format!(
                        r#"{{"items": [{}, {}]}}"#,
                        pack_item_json(8, 20),
                        pack_item_json(9, 21)
                    )),
                )
            })
        });
    let pass = build_with_pack(&database, &server);
    let pack = pass.pack.clone().expect("a pack");
    pack.replace(&[pack_item(5, 14)], 14)?;
    pass.resync.snapshot()?;
    *running.borrow_mut() = Rc::downgrade(&pass.resync);

    // The pass the FIRST connection started, overtaken by the second while it waited.
    let overtaken = pass.run()?;

    assert!(overtaken.complete());
    assert_eq!(pack.cursor()?, 19);
    assert!(!pack.is_caught_up());

    // Item 8 (seq 20) was added while no socket was listening. Item 9 (seq 21) arrives on the new
    // one, before the pass that connection started has reached the pack: drawn at once, and the
    // cursor stays where the feed needs it.
    mark.set(21);
    hear_pack_frame(&database, &pack, pack_item(9, 21));
    assert_eq!(pack.cursor()?, 19);

    pass.run()?;

    assert_eq!(
        last(&pass.asked()),
        "/api/v1/families/mine/pack/changes?after_seq=19&limit=50"
    );
    assert_eq!(item_ids(&pack)?, [5, 6, 7, 8, 9]);
    assert_eq!(pack.cursor()?, 21);
    assert!(pack.is_caught_up());
    Ok(())
}

/// The same overtaking where the pass would have asked for NOTHING: it read a mark level with its
/// cursor before the socket reopened. "Level" is a fact about the connection the mark was read on,
/// and says nothing about one that opened afterwards.
#[test]
fn a_level_pass_a_reconnect_overtook_does_not_catch_up_the_new_connection_either() -> StoreResult<()>
{
    let database = Database::open_in_memory()?;
    let running: Running = Rc::new(RefCell::new(Weak::new()));
    let server = FakeServer::new();
    let reopening = running.clone();
    server
        .always("/me", ME)
        .always("/families/mine", &family_with_pack(14))
        .on(move |path| {
            if path == "/api/v1/chats" {
                // After the family's document was read, and before the pass reaches the pack.
                snapshot(&reopening);
                return ok(r#"{"chats": []}"#.to_string());
            }
            None
        });
    let pass = build_with_pack(&database, &server);
    let pack = pass.pack.clone().expect("a pack");
    pack.replace(&[pack_item(5, 14)], 14)?;
    pass.resync.snapshot()?;
    *running.borrow_mut() = Rc::downgrade(&pass.resync);

    let overtaken = pass.run()?;

    assert!(overtaken.complete());
    assert!(!pack.is_caught_up());
    pack.apply(&pack_item(9, 21), SeqRoute::LiveFrame)?;
    assert_eq!(pack.cursor()?, 14);
    Ok(())
}

/// A pack read that fails stops the reads and leaves the connection NOT caught up — so a frame
/// still cannot move the cursor, and the next pass reads what this one could not.
#[test]
fn a_pack_read_that_fails_leaves_the_catch_up_owed() -> StoreResult<()> {
    let database = Database::open_in_memory()?;
    let server = pack_server(&family_with_pack(16));
    server.always_with(
        "/families/mine/pack/changes",
        r#"{"error": {"code": "internal", "message": "…"}}"#,
        500,
    );
    let pass = build_with_pack(&database, &server);
    pass.pack().replace(&[pack_item(5, 14)], 14)?;
    pass.resync.snapshot()?;

    let report = pass.run()?;

    assert!(!report.complete());
    assert!(report.stopped.as_ref().is_some_and(ApiError::transient));
    assert!(!pass.pack().is_caught_up());
    pass.pack().apply(&pack_item(9, 20), SeqRoute::LiveFrame)?;
    assert_eq!(pass.pack().cursor()?, 14);
    Ok(())
}

/// A resync built without a pack — every test above this section — asks for none.
#[test]
fn a_pass_with_no_pack_to_keep_asks_for_none() -> StoreResult<()> {
    let database = Database::open_in_memory()?;
    let pass = build(&database, &pack_server(&family_with_pack(14)));

    let report = pass.run()?;

    assert!(report.complete());
    assert_eq!(
        pass.asked(),
        ["/api/v1/me", "/api/v1/families/mine", "/api/v1/chats"]
    );
    Ok(())
}

// ---- video messages (docs/protocol.md, "Video messages", 2026-10-05) -------------------------

/// The pass carries the video message's two limits OUT of the family's own document, as it carries
/// the assistant — and says that it read that document, so a pass that stopped before it is never
/// taken for a server without them.
#[test]
fn a_pass_carries_the_video_messages_limits_and_says_it_read_the_family() -> StoreResult<()> {
    let database = Database::open_in_memory()?;
    let server = FakeServer::new();
    server
        .always("/me", ME)
        .always(
            "/families/mine",
            &family_wall(r#", "max_round_video_ms": 60000, "max_round_video_bytes": 12582912"#),
        )
        .always("/chats", r#"{"chats": []}"#);
    let pass = build(&database, &server);

    let report = pass.run()?;

    assert!(report.complete());
    assert!(report.family_read);
    assert_eq!(
        report.round_video,
        Some(RoundVideoLimits {
            max_ms: 60_000,
            max_bytes: 12_582_912
        })
    );
    Ok(())
}

/// A server that predates video messages names neither: read, and nothing to record with.
#[test]
fn an_older_server_reads_as_no_video_messages() -> StoreResult<()> {
    let database = Database::open_in_memory()?;
    let server = FakeServer::new();
    server
        .always("/me", ME)
        .always("/families/mine", &family_with_no_wall())
        .always("/chats", r#"{"chats": []}"#);
    let pass = build(&database, &server);

    let report = pass.run()?;

    assert!(report.family_read);
    assert_eq!(report.round_video, None);
    Ok(())
}

/// A pass that stopped at the family's document has not read it — which is not "no video
/// messages".
#[test]
fn a_pass_that_stopped_at_the_family_did_not_read_it() -> StoreResult<()> {
    let database = Database::open_in_memory()?;
    let server = FakeServer::new();
    server.always("/me", ME).always_with(
        "/families/mine",
        r#"{"error": {"code": "internal", "message": "no"}}"#,
        500,
    );
    let pass = build(&database, &server);

    let report = pass.run()?;

    assert!(!report.complete());
    assert!(!report.family_read);
    assert_eq!(report.round_video, None);
    Ok(())
}
