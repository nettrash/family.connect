//! The cache's schema and how it moves forward — a port of
//! `win/tests/FamilyConnect.Core.Tests/Store/DatabaseTests.cs`.
//!
//! The test that matters here is `a_migrated_schema_is_the_same_as_a_fresh_one`. Android's board
//! shipped a column with no migration once, and every upgraded install would have crashed on the
//! next launch (issue #70): the only thing that catches it is comparing a database that walked
//! every step against one created from scratch. This suite does that per table and per column, so
//! a step added without a column — or a column added without a step — fails here.

mod common;

use common::{TempDir, older_cache};
use fc_core::protocol::{ChatDto, ChatRowDto};
use fc_core::store::migrations;
use fc_core::store::{
    BoardStore, ChatStore, Database, OutboxRow, OutboxStore, PackStore, PlayedRoundStore,
    PlayedVoiceStore, StoreError, StoreResult, TranscriptStore,
};

/// Every statement of the first `count` steps, in order — what an older build ran.
fn steps(count: usize) -> impl Iterator<Item = &'static str> {
    migrations::ALL[..count]
        .iter()
        .flat_map(|step| step.iter().copied())
}

#[test]
fn a_fresh_cache_carries_this_builds_schema_number() -> StoreResult<()> {
    let directory = TempDir::new("fc-cache");
    let database = Database::open(directory.file("fresh.db"))?;
    assert_eq!(database.user_version()?, Database::schema_version());
    assert_eq!(Database::schema_version(), 11);
    assert_eq!(
        database.tables()?,
        [
            "blocked",
            "chats",
            "family_members",
            "gone",
            "members",
            "messages",
            "meta",
            "notes",
            "outbox",
            "pack_gone",
            "pack_items",
            "pack_recents",
            "played_rounds",
            "played_voice",
            "transcripts",
        ]
    );
    Ok(())
}

#[test]
fn opening_twice_migrates_nothing_and_keeps_what_is_there() -> StoreResult<()> {
    let directory = TempDir::new("fc-cache");
    let path = directory.file("twice.db");
    {
        let first = Database::open(&path)?;
        OutboxStore::new(&first).queue(&OutboxRow::new("k", 42, "Dinner at 7?"))?;
    }
    let again = Database::open(&path)?;
    assert_eq!(again.user_version()?, Database::schema_version());
    // The outbox is the one table holding something the server has never seen: it has to survive
    // the app closing, or an interrupted send is lost rather than finished.
    let rows = OutboxStore::new(&again).all()?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].body, "Dinner at 7?");
    Ok(())
}

/// THE ONE THAT CATCHES A FORGOTTEN MIGRATION. A cache that walked every step must have the same
/// tables and the same columns as one created from scratch.
#[test]
fn a_migrated_schema_is_the_same_as_a_fresh_one() -> StoreResult<()> {
    let directory = TempDir::new("fc-cache");
    // Stepped: start at version 0 with no tables, then let migrate() walk.
    let stepped_path = directory.file("stepped.db");
    older_cache(&stepped_path, ["PRAGMA user_version = 0"]);
    let stepped = Database::open(&stepped_path)?;
    let fresh = Database::open(directory.file("compare.db"))?;

    assert_eq!(stepped.user_version()?, fresh.user_version()?);
    assert_eq!(stepped.tables()?, fresh.tables()?);
    for table in fresh.tables()? {
        assert_eq!(stepped.columns(&table)?, fresh.columns(&table)?, "{table}");
    }
    Ok(())
}

/// STEP 2 READS THE MESSAGES AGAIN. A version-1 cache counted each chat's list preview in its
/// catch-up cursor, and the holes that left cannot be found from what is held — so its messages
/// go, and are paged back in. The outbox is the one table holding what the server has never seen,
/// and it stays; so do the chats.
#[test]
fn a_version_one_cache_loses_its_messages_and_keeps_its_outbox() -> StoreResult<()> {
    let directory = TempDir::new("fc-cache");
    let path = directory.file("one.db");
    older_cache(
        &path,
        steps(1).chain([
            "INSERT INTO chats (chat_id, kind, title) VALUES (42, 'family', 'The Smiths')",
            "INSERT INTO messages (message_id, chat_id, sender_id, body, created_at) VALUES (20, 42, 9, 'hi', 0)",
            "INSERT INTO outbox (client_msg_id, chat_id, body, queued_at) VALUES ('k', 42, 'Dinner at 7?', 0)",
            "PRAGMA user_version = 1",
        ]),
    );

    let migrated = Database::open(&path)?;
    assert_eq!(migrated.user_version()?, Database::schema_version());
    let chats = ChatStore::new(&migrated);
    assert!(chats.messages(42, None, 50)?.is_empty());
    assert_eq!(chats.catch_up_cursor(42)?, None);
    assert!(chats.chat(42)?.is_some());
    let queued = OutboxStore::new(&migrated).all()?;
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].body, "Dinner at 7?");
    assert!(
        migrated
            .columns("messages")?
            .iter()
            .any(|column| column.starts_with("sequenced"))
    );
    Ok(())
}

/// STEP 4 ADDS THE STICKER PACK AND READS NOTHING AGAIN. A version-3 cache keeps its messages, its
/// board and its outbox — and every send already queued in it is what it always was: not a
/// sticker.
#[test]
fn a_version_three_cache_gains_the_pack_and_loses_nothing() -> StoreResult<()> {
    let directory = TempDir::new("fc-cache");
    let path = directory.file("three.db");
    older_cache(
        &path,
        steps(3).chain([
            "INSERT INTO chats (chat_id, kind, title) VALUES (42, 'family', 'The Smiths')",
            "INSERT INTO messages (message_id, chat_id, sender_id, body, created_at, sequenced) VALUES (20, 42, 9, 'hi', 0, 1)",
            "INSERT INTO notes (note_id, author_id, text, board_seq) VALUES (1, 7, 'Milk', 5)",
            "INSERT INTO outbox (client_msg_id, chat_id, body, queued_at) VALUES ('k', 42, 'Dinner at 7?', 0)",
            "PRAGMA user_version = 3",
        ]),
    );

    let migrated = Database::open(&path)?;
    assert_eq!(migrated.user_version()?, Database::schema_version());
    let messages = ChatStore::new(&migrated).messages(42, None, 50)?;
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].body, "hi");
    let notes = BoardStore::new(&migrated).notes()?;
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].text.as_deref(), Some("Milk"));
    let queued = OutboxStore::new(&migrated).all()?;
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].body, "Dinner at 7?");
    assert!(!queued[0].sticker);
    let pack = PackStore::new(&migrated);
    assert!(pack.items()?.is_empty());
    assert_eq!(pack.cursor()?, 0);
    // No ceilings are known yet, so no sticker button is offered until the server names them.
    assert_eq!(pack.limits()?, None);
    Ok(())
}

/// STEP 5 ADDS THE KEPT TRANSCRIPTS AND READS NOTHING AGAIN. A version-4 cache keeps its messages,
/// its pack and its outbox, and holds no text for any recording until somebody asks.
#[test]
fn a_version_four_cache_gains_transcripts_and_loses_nothing() -> StoreResult<()> {
    let directory = TempDir::new("fc-cache");
    let path = directory.file("four.db");
    older_cache(
        &path,
        steps(4).chain([
            "INSERT INTO chats (chat_id, kind, title) VALUES (42, 'family', 'The Smiths')",
            "INSERT INTO messages (message_id, chat_id, sender_id, body, created_at, sequenced) VALUES (20, 42, 9, 'hi', 0, 1)",
            r#"INSERT INTO pack_items (item_id, attachment_json) VALUES (1, '{"id": 71, "kind": "photo"}')"#,
            "INSERT INTO outbox (client_msg_id, chat_id, body, queued_at, sticker) VALUES ('k', 42, 'Dinner at 7?', 0, 1)",
            "PRAGMA user_version = 4",
        ]),
    );

    let migrated = Database::open(&path)?;
    assert_eq!(migrated.user_version()?, Database::schema_version());
    let messages = ChatStore::new(&migrated).messages(42, None, 50)?;
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].body, "hi");
    assert_eq!(PackStore::new(&migrated).items()?.len(), 1);
    let queued = OutboxStore::new(&migrated).all()?;
    assert_eq!(queued.len(), 1);
    assert!(queued[0].sticker);
    assert_eq!(TranscriptStore::new(&migrated).find(34)?, None);
    Ok(())
}

/// STEP 6 ADDS VIDEO MESSAGES AND READS NOTHING AGAIN. A version-5 cache keeps its messages, its
/// kept text and its outbox — every send already queued in it is what it always was: not a circle,
/// and a sticker still a sticker — and no circle has been played on it yet.
#[test]
fn a_version_five_cache_gains_video_messages_and_loses_nothing() -> StoreResult<()> {
    let directory = TempDir::new("fc-cache");
    let path = directory.file("five.db");
    older_cache(
        &path,
        steps(5).chain([
            "INSERT INTO chats (chat_id, kind, title) VALUES (42, 'family', 'The Smiths')",
            "INSERT INTO messages (message_id, chat_id, sender_id, body, created_at, sequenced) VALUES (20, 42, 9, 'hi', 0, 1)",
            "INSERT INTO transcripts (attachment_id, text) VALUES (34, 'Back at six')",
            "INSERT INTO outbox (client_msg_id, chat_id, body, queued_at) VALUES ('k', 42, 'Dinner at 7?', 0)",
            "INSERT INTO outbox (client_msg_id, chat_id, body, queued_at, sticker) VALUES ('s', 42, '', 1, 1)",
            "PRAGMA user_version = 5",
        ]),
    );

    let migrated = Database::open(&path)?;
    assert_eq!(migrated.user_version()?, Database::schema_version());
    let messages = ChatStore::new(&migrated).messages(42, None, 50)?;
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].body, "hi");
    assert_eq!(
        TranscriptStore::new(&migrated)
            .find(34)?
            .expect("the kept text")
            .text,
        "Back at six"
    );
    let queued = OutboxStore::new(&migrated).all()?;
    assert_eq!(queued.len(), 2);
    assert!(queued.iter().all(|row| !row.round));
    assert!(!queued[0].sticker);
    assert!(queued[1].sticker);
    let played = PlayedRoundStore::new(&migrated);
    assert_eq!(played.count()?, 0);
    assert!(!played.played(91)?);
    Ok(())
}

/// Version 6 → 7 adds which voice messages this device has played: nothing held is touched — the
/// circles already played stay played — and no voice note is played yet, so every one that arrived
/// before shows its dot.
#[test]
fn a_version_six_cache_gains_played_voice_messages_and_loses_nothing() -> StoreResult<()> {
    let directory = TempDir::new("fc-cache");
    let path = directory.file("six.db");
    older_cache(
        &path,
        steps(6).chain([
            "INSERT INTO chats (chat_id, kind, title) VALUES (42, 'family', 'The Smiths')",
            "INSERT INTO messages (message_id, chat_id, sender_id, body, created_at, sequenced) VALUES (20, 42, 9, 'hi', 0, 1)",
            "INSERT INTO played_rounds (attachment_id) VALUES (91)",
            "PRAGMA user_version = 6",
        ]),
    );

    let migrated = Database::open(&path)?;
    assert_eq!(migrated.user_version()?, Database::schema_version());
    let messages = ChatStore::new(&migrated).messages(42, None, 50)?;
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].body, "hi");
    assert!(PlayedRoundStore::new(&migrated).played(91)?);
    let voice = PlayedVoiceStore::new(&migrated);
    assert_eq!(voice.count()?, 0);
    assert!(!voice.played(91)?);
    Ok(())
}

/// Version 7 → 8 says which cached sets know the sticker and video-message flags. Nothing held is
/// lost and nothing is read at upgrade time — but every row already there is UNKNOWN, because
/// nothing says which build wrote it: a sticker and a circle a build before #58/#79 cached come
/// back as candidates for the pass to read again, and the rows that could not have lost a flag are
/// settled without a request.
#[test]
fn a_version_seven_cache_marks_every_set_unknown_and_loses_nothing() -> StoreResult<()> {
    let directory = TempDir::new("fc-cache");
    let path = directory.file("seven.db");
    older_cache(
        &path,
        steps(7).chain([
            "INSERT INTO chats (chat_id, kind, title) VALUES (42, 'family', 'The Smiths')",
            "INSERT INTO messages (message_id, chat_id, sender_id, body, created_at, sequenced) VALUES (20, 42, 9, 'hi', 0, 1)",
            r#"INSERT INTO messages (message_id, chat_id, sender_id, body, created_at, sequenced, attachments_json)
VALUES (21, 42, 9, '', 0, 1, '[{"id":90,"kind":"photo","mime":"image/webp","size":40960,"width":512,"height":512,"has_preview":false}]')"#,
            r#"INSERT INTO messages (message_id, chat_id, sender_id, body, created_at, sequenced, attachments_json)
VALUES (22, 42, 9, '', 0, 1, '[{"id":91,"kind":"video","mime":"video/mp4","size":1,"width":480,"height":480,"has_preview":true}]')"#,
            "INSERT INTO played_voice (attachment_id) VALUES (34)",
            "INSERT INTO outbox (client_msg_id, chat_id, body, queued_at, round) VALUES ('r', 42, '', 1, 1)",
            "PRAGMA user_version = 7",
        ]),
    );

    let migrated = Database::open(&path)?;
    assert_eq!(migrated.user_version()?, Database::schema_version());
    assert!(
        migrated
            .columns("messages")?
            .iter()
            .any(|column| column.starts_with("attachments_know_flags"))
    );
    let chats = ChatStore::new(&migrated);
    assert_eq!(chats.messages(42, None, 50)?.len(), 3);
    assert_eq!(chats.catch_up_cursor(42)?, Some(22));
    assert!(PlayedVoiceStore::new(&migrated).played(34)?);
    let queued = OutboxStore::new(&migrated).all()?;
    assert_eq!(queued.len(), 1);
    assert!(queued[0].round);
    let candidates: Vec<i64> = chats
        .flag_repair_candidates(25)?
        .iter()
        .map(|message| message.id)
        .collect();
    assert_eq!(candidates, [22, 21]);
    Ok(())
}

/// Version 8 → 9 adds the outbox's animation bit (issue #96): every send already queued is what it
/// always was — not an animation, and a circle still a circle — and nothing held is touched.
#[test]
fn a_version_eight_cache_gains_animations_and_loses_nothing() -> StoreResult<()> {
    let directory = TempDir::new("fc-cache");
    let path = directory.file("eight.db");
    older_cache(
        &path,
        steps(8).chain([
            "INSERT INTO chats (chat_id, kind, title) VALUES (42, 'family', 'The Smiths')",
            "INSERT INTO messages (message_id, chat_id, sender_id, body, created_at, sequenced, attachments_know_flags) VALUES (20, 42, 9, 'hi', 0, 1, 1)",
            "INSERT INTO outbox (client_msg_id, chat_id, body, queued_at, round) VALUES ('r', 42, '', 1, 1)",
            "INSERT INTO outbox (client_msg_id, chat_id, body, queued_at) VALUES ('k', 42, 'Dinner at 7?', 2)",
            "PRAGMA user_version = 8",
        ]),
    );

    let migrated = Database::open(&path)?;
    assert_eq!(migrated.user_version()?, Database::schema_version());
    assert!(
        migrated
            .columns("outbox")?
            .iter()
            .any(|column| column.starts_with("animation"))
    );
    let messages = ChatStore::new(&migrated).messages(42, None, 50)?;
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].body, "hi");
    let queued = OutboxStore::new(&migrated).all()?;
    assert_eq!(queued.len(), 2);
    assert!(queued.iter().all(|row| !row.animation));
    assert!(queued[0].round);
    assert!(!queued[1].round);
    Ok(())
}

/// Version 9 → 10 adds a GROUP's members and admin to its chat row (docs/protocol.md, "Groups"):
/// nothing held is read again — the chats, their messages and the outbox stay — and every chat
/// already cached carries neither until a list read or a `chat_updated` writes them, which is what
/// every kind but a group always says.
#[test]
fn a_version_nine_cache_gains_group_members_and_loses_nothing() -> StoreResult<()> {
    let directory = TempDir::new("fc-cache");
    let path = directory.file("nine.db");
    older_cache(
        &path,
        steps(9).chain([
            "INSERT INTO chats (chat_id, kind, title) VALUES (42, 'family', 'The Smiths')",
            // A group an older build met before it knew groups: a chat with a name and nothing
            // else.
            "INSERT INTO chats (chat_id, kind, title) VALUES (77, 'group', 'Cousins')",
            "INSERT INTO messages (message_id, chat_id, sender_id, body, created_at, sequenced) VALUES (20, 77, 9, 'hi', 0, 1)",
            "INSERT INTO outbox (client_msg_id, chat_id, body, queued_at) VALUES ('k', 42, 'Dinner at 7?', 0)",
            "PRAGMA user_version = 9",
        ]),
    );

    let migrated = Database::open(&path)?;
    assert_eq!(migrated.user_version()?, Database::schema_version());
    let chats = ChatStore::new(&migrated);
    let messages = chats.messages(77, None, 50)?;
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].body, "hi");
    let group = chats.chat(77)?.expect("the group").chat;
    assert_eq!(group.title, "Cousins");
    assert_eq!(group.member_ids, None);
    assert_eq!(group.admin_user_id, None);
    let queued = OutboxStore::new(&migrated).all()?;
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].body, "Dinner at 7?");

    // The next list read writes them.
    chats.replace(
        &[
            ChatRowDto::new(ChatDto::new(42, "family", "The Smiths")),
            ChatRowDto::new(ChatDto {
                member_ids: Some(vec![7, 9]),
                admin_user_id: Some(9),
                ..ChatDto::new(77, "group", "Cousins")
            }),
        ],
        None,
    )?;
    let group = chats.chat(77)?.expect("the group").chat;
    assert_eq!(group.member_ids, Some(vec![7, 9]));
    assert_eq!(group.admin_user_id, Some(9));
    Ok(())
}

/// A file from a NEWER build is refused rather than used. The alternative is this build writing
/// rows a newer schema will misread — and nothing here may delete a family's outbox to make itself
/// comfortable.
#[test]
fn a_cache_from_a_newer_build_is_refused() {
    let directory = TempDir::new("fc-cache");
    let path = directory.file("newer.db");
    let stamp = format!("PRAGMA user_version = {}", Database::schema_version() + 7);
    older_cache(&path, [stamp.as_str()]);
    match Database::open(&path) {
        Err(refused @ StoreError::NewerSchema { .. }) => {
            assert!(refused.to_string().contains("newer version"), "{refused}");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn foreign_keys_are_on_because_the_schema_leans_on_them() -> StoreResult<()> {
    // Off by default in SQLite, per connection — a cascade that silently did nothing would leave a
    // chat's messages behind when the chat went.
    let database = Database::open_in_memory()?;
    let on: i64 = database
        .connection()
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
    assert_eq!(on, 1);
    Ok(())
}

#[test]
fn every_migration_is_append_only_and_numbered() {
    // The index is the version it upgrades FROM, so the count IS the schema number. A step that
    // has shipped is never edited: an installed file has already run it.
    assert_eq!(
        usize::try_from(Database::schema_version()).ok(),
        Some(migrations::ALL.len())
    );
    assert!(migrations::ALL.iter().all(|step| !step.is_empty()));
}
