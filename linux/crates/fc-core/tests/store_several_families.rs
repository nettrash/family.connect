//! The cache, one account in several families (docs/protocol.md, "Several families"; schema step
//! 11): which family each held thing belongs to, a roster, a wall and a pack per family with
//! cursors of their own, leaving ONE family dropping only its rows, and a cache an older build
//! filled made the home family's. A port of
//! `win/tests/FamilyConnect.Core.Tests/Store/SeveralFamiliesStoreTests.cs`.

mod common;

use std::cell::Cell;
use std::rc::Rc;

use common::{TempDir, older_cache};
use fc_core::protocol::{
    AttachmentDto, ChatDto, ChatRowDto, MemberDto, MessageDto, NoteDto, PackItemDto, UserDto,
};
use fc_core::store::migrations;
use fc_core::store::{
    BoardStore, ChatStore, Database, OutboxRow, OutboxStore, PackLimits, PackStore, SeqRoute,
    StoreResult,
};
use time::{Duration, OffsetDateTime};

const SENT: &str = "2026-10-22T10:00:00Z";

fn open() -> StoreResult<Rc<Database>> {
    Database::open_in_memory()
}

/// `Item(id, seq, family)`: added by 7, a 4096-byte PNG.
fn item(id: i64, seq: i64, family: Option<i64>) -> PackItemDto {
    PackItemDto {
        added_by: 7,
        attachment: Some(AttachmentDto {
            mime: Some("image/png".to_string()),
            size: Some(4096),
            width: Some(512),
            height: Some(512),
            ..AttachmentDto::new(70 + id, "photo")
        }),
        created_at: Some(SENT.to_string()),
        pack_seq: seq,
        family_id: family,
        ..PackItemDto::new(id)
    }
}

/// `new NoteDto(id, author, "text", text, BoardSeq: seq, ContentSeq: content)`.
fn note(id: i64, author: i64, text: &str, seq: i64, content: Option<i64>) -> NoteDto {
    NoteDto {
        kind: Some("text".to_string()),
        text: Some(text.to_string()),
        board_seq: seq,
        content_seq: content,
        ..NoteDto::new(id, author)
    }
}

fn person(id: i64, username: &str, name: &str, role: Option<&str>) -> MemberDto {
    MemberDto {
        role: role.map(str::to_string),
        ..MemberDto::new(id, username, name)
    }
}

fn listed_in(id: i64, kind: &str, title: &str, peer: Option<i64>, families: &[i64]) -> ChatRowDto {
    ChatRowDto::new(ChatDto {
        peer_user_id: peer,
        family_ids: Some(families.to_vec()),
        ..ChatDto::new(id, kind, title)
    })
}

fn one<T>(mut items: Vec<T>) -> T {
    assert_eq!(items.len(), 1, "exactly one");
    items.remove(0)
}

fn find(members: &[MemberDto], id: i64) -> MemberDto {
    let matching: Vec<&MemberDto> = members.iter().filter(|member| member.id == id).collect();
    assert_eq!(matching.len(), 1, "exactly one member {id}");
    matching[0].clone()
}

fn ids<T>(items: &[T], id: impl Fn(&T) -> i64) -> Vec<i64> {
    items.iter().map(id).collect()
}

// ---- the migration --------------------------------------------------------------------------------

/// Version 10 → 11 adds which family each thing belongs to and a roster per family — and READS
/// NOTHING AGAIN: the chats, their messages, the wall, the pack and the outbox stay, their families
/// unknown (NULL) until the home family is named.
#[test]
fn a_version_ten_cache_gains_families_and_loses_nothing() -> StoreResult<()> {
    let directory = TempDir::new("fc-families");
    let path = directory.file("ten.db");
    older_cache(
        &path,
        migrations::ALL[..10]
            .iter()
            .flat_map(|step| step.iter().copied())
            .chain([
                "INSERT INTO chats (chat_id, kind, title) VALUES (42, 'family', 'The Smiths')",
                "INSERT INTO messages (message_id, chat_id, sender_id, body, created_at, sequenced) VALUES (20, 42, 9, 'hi', 0, 1)",
                "INSERT INTO notes (note_id, author_id, text, board_seq) VALUES (1, 9, 'Milk', 4)",
                r#"INSERT INTO pack_items (item_id, added_by, pack_seq, attachment_json) VALUES (5, 9, 2, '{"id":75,"kind":"photo"}')"#,
                "INSERT INTO members (user_id, username, display_name, role) VALUES (9, 'bob', 'Bob', 'owner')",
                "INSERT INTO meta (key, value) VALUES ('board.cursor.board_seq', '4')",
                "INSERT INTO outbox (client_msg_id, chat_id, body, queued_at) VALUES ('k', 42, 'Dinner at 7?', 0)",
                "PRAGMA user_version = 10",
            ]),
    );

    let migrated = Database::open(&path)?;
    assert_eq!(migrated.user_version()?, Database::schema_version());
    let chats = ChatStore::new(&migrated);
    assert_eq!(one(chats.messages(42, None, 50)?).body, "hi");
    assert_eq!(
        chats.chat(42)?.expect("the family chat").chat.family_ids,
        None
    );
    assert_eq!(one(OutboxStore::new(&migrated).all()?).body, "Dinner at 7?");
    let board = BoardStore::new(&migrated);
    assert_eq!(one(board.notes()?).family_id, None);
    assert_eq!(board.cursor()?, 4);
    assert_eq!(one(PackStore::new(&migrated).items()?).family_id, None);
    assert!(
        migrated
            .tables()?
            .iter()
            .any(|table| table == "family_members")
    );
    for (table, column) in [
        ("chats", "family_ids_json"),
        ("notes", "family_id"),
        ("pack_items", "family_id"),
    ] {
        assert!(
            migrated
                .columns(table)?
                .iter()
                .any(|declared| declared.starts_with(column)),
            "{table}.{column}"
        );
    }

    // Named, the home family adopts it all: the wall, the pack, the roster and the cursors are
    // family 3's.
    migrated.adopt_legacy(3)?;
    let home = BoardStore::new(&migrated).with_current(|| Some(3));
    assert_eq!(one(home.notes()?).family_id, Some(3));
    assert_eq!(home.cursor()?, 4);
    assert_eq!(board.cursor()?, 0);
    assert_eq!(
        one(PackStore::new(&migrated).with_current(|| Some(3)).items()?).family_id,
        Some(3)
    );
    let roster = ChatStore::new(&migrated)
        .with_me(|| 7)
        .with_current(|| Some(3))
        .members()?;
    assert_eq!(roster.len(), 1);
    assert!(roster[0].owner());
    assert_eq!(migrated.held_families()?, [3]);
    // And twice is once.
    migrated.adopt_legacy(3)?;
    assert_eq!(home.cursor()?, 4);
    assert_eq!(home.notes()?.len(), 1);
    Ok(())
}

// ---- chats ----------------------------------------------------------------------------------------

/// A chat keeps the families it is listed in, from the list and from a `chat_updated` alike.
#[test]
fn a_chat_keeps_the_families_it_is_listed_in() -> StoreResult<()> {
    let database = open()?;
    let chats = ChatStore::new(&database);
    chats.replace(&[listed_in(44, "direct", "Bob", Some(9), &[3, 5])], None)?;
    let families = |chats: &ChatStore| -> StoreResult<Option<Vec<i64>>> {
        Ok(chats.chat(44)?.expect("Bob's chat").chat.family_ids)
    };
    assert_eq!(families(&chats)?, Some(vec![3, 5]));
    chats.upsert(&ChatDto {
        peer_user_id: Some(9),
        family_ids: Some(vec![5]),
        ..ChatDto::new(44, "direct", "Bob")
    })?;
    assert_eq!(families(&chats)?, Some(vec![5]));
    assert_eq!(one(chats.chats()?).chat.family_ids, Some(vec![5]));
    Ok(())
}

// ---- rosters --------------------------------------------------------------------------------------

/// A ROSTER PER FAMILY. The same person is one row — their name and picture are the account's — and
/// a member of each family with that family's role; the roster read is the family SHOWN, and a name
/// resolves for anybody this device has met.
#[test]
fn each_family_has_its_own_roster_over_one_person() -> StoreResult<()> {
    let database = open()?;
    let shown = Rc::new(Cell::new(Some(3)));
    let showing = Rc::clone(&shown);
    let chats = ChatStore::new(&database)
        .with_me(|| 7)
        .with_current(move || showing.get());
    chats.replace_members_of(
        Some(3),
        &[
            person(7, "anna", "Anna", Some("member")),
            person(9, "bob", "Bob", Some("owner")),
        ],
        &[],
    )?;
    chats.replace_members_of(
        Some(5),
        &[
            person(7, "anna", "Anna", Some("owner")),
            MemberDto {
                password_reset: Some(true),
                ..person(11, "carl", "Carl", Some("member"))
            },
        ],
        &[MemberDto {
            has_left: true,
            ..MemberDto::new(12, "dana", "Dana")
        }],
    )?;

    assert_eq!(ids(&chats.members()?, |member| member.id), [7, 9]);
    assert!(!find(&chats.members()?, 7).owner());
    // Somebody from the other family still has a name.
    assert_eq!(chats.member(11)?.expect("Carl").display_name, "Carl");
    assert_eq!(chats.roster_families()?, [3, 5]);

    shown.set(Some(5));
    let mut present = ids(&chats.members()?, |member| member.id);
    present.sort_by_key(|&id| (id != 12, id));
    assert_eq!(present, [12, 7, 11]);
    assert!(find(&chats.members()?, 7).owner());
    assert_eq!(find(&chats.members()?, 11).password_reset, Some(true));
    assert!(find(&chats.members()?, 12).is_former());
    assert_eq!(find(&chats.members_of(3)?, 9).password_reset, None);

    // A family's own read replaces that family's rows and nobody else's.
    chats.replace_members_of(Some(5), &[person(7, "anna", "Anna", Some("owner"))], &[])?;
    assert_eq!(ids(&chats.members()?, |member| member.id), [7]);
    assert_eq!(ids(&chats.members_of(3)?, |member| member.id), [7, 9]);
    Ok(())
}

/// The narrow writers go to the family named — and, naming none, the family shown.
#[test]
fn joins_leaves_and_owners_are_a_familys() -> StoreResult<()> {
    let database = open()?;
    let chats = ChatStore::new(&database)
        .with_me(|| 7)
        .with_current(|| Some(3));
    chats.replace_members_of(
        Some(3),
        &[
            person(7, "anna", "Anna", Some("owner")),
            person(9, "bob", "Bob", Some("member")),
        ],
        &[],
    )?;
    chats.replace_members_of(
        Some(5),
        &[
            person(9, "bob", "Bob", Some("owner")),
            person(7, "anna", "Anna", Some("member")),
        ],
        &[],
    )?;

    chats.joined(&UserDto::new(12, "dana", "Dana"), Some(5), None)?;
    chats.left(9, Some(5))?;
    chats.set_owner(7, Some(5))?;
    assert!(!chats.members()?.iter().any(|member| member.id == 12));
    assert!(!find(&chats.members()?, 9).is_former());
    assert!(find(&chats.members_of(5)?, 9).has_left);
    assert!(find(&chats.members_of(5)?, 7).owner());
    assert!(find(&chats.members()?, 7).owner());

    // Naming none: the family shown.
    chats.left(9, None)?;
    assert!(find(&chats.members()?, 9).has_left);
    // An account deleted is out of every family.
    chats.deleted(&MemberDto {
        deleted: true,
        ..MemberDto::new(12, "deleted-12", "Deleted account")
    })?;
    assert!(find(&chats.members_of(5)?, 12).deleted);
    assert!(find(&chats.members_of(5)?, 12).has_left);
    Ok(())
}

/// A join keeps the role of somebody already here — on a family's rows and on a cache's one roster
/// from before several families alike — and makes a newcomer or a returning former member a member;
/// the reset flag keeps its narrow rule.
#[test]
fn a_join_never_downgrades_somebody_already_here() -> StoreResult<()> {
    let database = open()?;
    let chats = ChatStore::new(&database)
        .with_me(|| 7)
        .with_current(|| Some(5));
    chats.replace_members_of(
        Some(5),
        &[
            person(7, "anna", "Anna", Some("owner")),
            MemberDto {
                password_reset: Some(false),
                ..person(11, "carl", "Carl", Some("member"))
            },
        ],
        &[],
    )?;
    chats.joined(&UserDto::new(7, "anna", "Anna"), Some(5), None)?;
    chats.joined(&UserDto::new(11, "carl", "Carl"), Some(5), None)?;
    assert_eq!(
        find(&chats.members_of(5)?, 7).role.as_deref(),
        Some("owner")
    );
    assert_eq!(
        find(&chats.members_of(5)?, 11).role.as_deref(),
        Some("member")
    );
    assert_eq!(find(&chats.members_of(5)?, 11).password_reset, Some(false));
    chats.left(7, Some(5))?;
    chats.joined(&UserDto::new(7, "anna", "Anna"), Some(5), None)?;
    assert_eq!(
        find(&chats.members_of(5)?, 7).role.as_deref(),
        Some("member")
    );

    // One family, no family named: the person's own row.
    let legacy = open()?;
    let only = ChatStore::new(&legacy).with_me(|| 7);
    only.replace_members(&[person(7, "anna", "Anna", Some("owner"))], &[])?;
    only.joined(&UserDto::new(7, "anna", "Anna"), None, None)?;
    assert!(only.member(7)?.expect("Anna").owner());
    only.left(7, None)?;
    only.joined(&UserDto::new(7, "anna", "Anna"), None, None)?;
    let anna = only.member(7)?.expect("Anna");
    assert_eq!(anna.role.as_deref(), Some("member"));
    assert!(!anna.has_left);
    Ok(())
}

// ---- the wall and the pack --------------------------------------------------------------------------

/// ONE WALL PER FAMILY, each with its own seqs, cursor, full-read mark and badge: a full read of one
/// family's wall drops nothing of another's, and seeing one wall marks nothing on the other.
#[test]
fn each_family_has_its_own_wall_and_cursor() -> StoreResult<()> {
    let database = open()?;
    let shown = Rc::new(Cell::new(Some(3)));
    let showing = Rc::clone(&shown);
    let board = BoardStore::new(&database).with_current(move || showing.get());
    let other = board.in_family(Some(5));
    assert_eq!(board.family(), Some(3));
    assert_eq!(other.family(), Some(5));

    board.replace(&[note(1, 9, "Milk", 4, Some(4))], 4)?;
    other.replace(&[note(2, 11, "Bread", 9, Some(9))], 9)?;
    // A full read of family 3 at a LOWER seq is not older than family 5's wall: each wall has its
    // own numbers.
    board.replace(
        &[
            note(1, 9, "Milk", 4, Some(4)),
            note(3, 9, "Tea", 5, Some(5)),
        ],
        5,
    )?;

    assert_eq!(ids(&board.notes()?, |note| note.id), [3, 1]);
    assert!(board.notes()?.iter().all(|note| note.family_id == Some(3)));
    assert_eq!(one(other.notes()?).id, 2);
    assert_eq!(board.cursor()?, 5);
    assert_eq!(other.cursor()?, 9);
    assert_eq!(board.unread()?, 2);
    board.mark_shown()?;
    assert_eq!(board.unread()?, 0);
    assert_eq!(other.unread()?, 1);

    // A frame names its family; the store held to that family writes it there.
    other.apply(
        &NoteDto {
            family_id: Some(5),
            ..note(4, 11, "Eggs", 10, Some(10))
        },
        SeqRoute::LiveFrame,
    )?;
    assert_eq!(other.cursor()?, 10);
    assert_eq!(board.cursor()?, 5);
    shown.set(Some(5));
    assert_eq!(ids(&board.notes()?, |note| note.id), [4, 2]);
    Ok(())
}

/// One pack per family, with its own ceilings and catch-up — and ONE connection: a socket reopening
/// un-catches every family, and catching one up does not catch the others.
#[test]
fn each_family_has_its_own_pack_on_one_connection() -> StoreResult<()> {
    let database = open()?;
    let pack = PackStore::new(&database).with_current(|| Some(3));
    let other = pack.in_family(Some(5));
    pack.replace(&[item(1, 2, None)], 2)?;
    other.replace(&[item(2, 7, None)], 7)?;
    pack.set_limits(Some(PackLimits {
        max_items: 200,
        max_item_bytes: 524_288,
    }))?;
    assert_eq!(ids(&pack.items()?, |item| item.id), [1]);
    assert_eq!(one(pack.items()?).family_id, Some(3));
    assert_eq!(other.count()?, 1);
    assert_eq!(pack.cursor()?, 2);
    assert_eq!(other.cursor()?, 7);
    assert!(pack.limits()?.is_some());
    assert_eq!(other.limits()?, None);

    pack.reconnected();
    let began = other.connection();
    assert!(pack.caught_up(began));
    assert!(pack.is_caught_up());
    assert!(!other.is_caught_up());
    // A frame on a family not yet caught up does not move its cursor.
    other.apply(&item(3, 8, Some(5)), SeqRoute::LiveFrame)?;
    assert_eq!(other.cursor()?, 7);
    assert!(other.caught_up(began));
    other.apply(&item(4, 9, Some(5)), SeqRoute::LiveFrame)?;
    assert_eq!(other.cursor()?, 9);
    pack.used(1, OffsetDateTime::UNIX_EPOCH)?;
    other.used(4, OffsetDateTime::UNIX_EPOCH + Duration::seconds(1))?;
    assert_eq!(pack.recents(16)?, [1]);
    assert_eq!(other.recents(16)?, [4]);
    pack.reconnected();
    assert!(!pack.is_caught_up());
    assert!(!other.is_caught_up());
    Ok(())
}

// ---- leaving one family -----------------------------------------------------------------------------

/// LEAVING ONE FAMILY DROPS ONLY ITS ROWS (docs/protocol.md, "Frames": "drops the family's chats,
/// board, pack, roster and cursors — and only that family's"): its family chat with its messages and
/// queued sends, its wall, its pack and their cursors and marks, its roster — while a direct chat
/// still listed in another family stays, naming that one only.
#[test]
fn dropping_a_family_takes_its_rows_and_no_others() -> StoreResult<()> {
    let database = open()?;
    let chats = ChatStore::new(&database)
        .with_me(|| 7)
        .with_current(|| Some(3));
    chats.replace(
        &[
            listed_in(42, "family", "The Smiths", None, &[3]),
            listed_in(43, "family", "Ivanovs", None, &[5]),
            listed_in(44, "direct", "Bob", Some(9), &[3, 5]),
            listed_in(45, "direct", "Carl", Some(11), &[5]),
        ],
        None,
    )?;
    chats.apply(
        &MessageDto::new(1, 43, 11, Some("c-1"), "hi", SENT),
        SeqRoute::CatchUpPage,
        true,
    )?;
    chats.apply(
        &MessageDto::new(2, 44, 9, Some("c-2"), "hello", SENT),
        SeqRoute::CatchUpPage,
        true,
    )?;
    OutboxStore::new(&database).queue(&OutboxRow::new("q-1", 43, "queued"))?;
    OutboxStore::new(&database).queue(&OutboxRow::new("q-2", 42, "stays"))?;
    chats.replace_members_of(
        Some(3),
        &[
            person(7, "anna", "Anna", Some("owner")),
            MemberDto::new(9, "bob", "Bob"),
        ],
        &[],
    )?;
    chats.replace_members_of(
        Some(5),
        &[
            MemberDto::new(7, "anna", "Anna"),
            person(11, "carl", "Carl", Some("owner")),
        ],
        &[],
    )?;
    let board = BoardStore::new(&database);
    board
        .in_family(Some(3))
        .replace(&[note(1, 9, "Milk", 4, None)], 4)?;
    board
        .in_family(Some(5))
        .replace(&[note(2, 11, "Bread", 9, None)], 9)?;
    let pack = PackStore::new(&database);
    pack.in_family(Some(3)).replace(&[item(1, 2, None)], 2)?;
    pack.in_family(Some(5)).replace(&[item(2, 7, None)], 7)?;
    pack.in_family(Some(5))
        .used(2, OffsetDateTime::UNIX_EPOCH)?;
    assert_eq!(database.held_families()?, [3, 5]);

    let mut gone = database.drop_family(5)?;

    gone.sort_unstable();
    assert_eq!(gone, [43, 45]);
    assert_eq!(chats.chat(43)?, None);
    assert_eq!(chats.chat(45)?, None);
    assert!(chats.messages(43, None, 50)?.is_empty());
    assert_eq!(
        chats.chat(44)?.expect("Bob's chat").chat.family_ids,
        Some(vec![3])
    );
    assert_eq!(one(chats.messages(44, None, 50)?).body, "hello");
    assert_eq!(one(OutboxStore::new(&database).all()?).client_msg_id, "q-2");
    assert!(board.in_family(Some(5)).notes()?.is_empty());
    assert_eq!(board.in_family(Some(5)).cursor()?, 0);
    assert_eq!(board.in_family(Some(3)).notes()?.len(), 1);
    assert_eq!(board.in_family(Some(3)).cursor()?, 4);
    assert!(pack.in_family(Some(5)).items()?.is_empty());
    assert_eq!(pack.in_family(Some(5)).cursor()?, 0);
    assert!(pack.in_family(Some(5)).recents(16)?.is_empty());
    assert_eq!(pack.in_family(Some(3)).items()?.len(), 1);
    assert!(chats.members_of(5)?.is_empty());
    assert_eq!(chats.members_of(3)?.len(), 2);
    // The person stays a name: their old messages still need one.
    assert_eq!(chats.member(11)?.expect("Carl").display_name, "Carl");
    assert_eq!(database.held_families()?, [3]);
    // A family's keys are its own: dropping 5 leaves a family 55 alone.
    board
        .in_family(Some(55))
        .replace(&[note(3, 9, "Tea", 2, None)], 2)?;
    database.drop_family(5)?;
    assert_eq!(board.in_family(Some(55)).cursor()?, 2);
    Ok(())
}
