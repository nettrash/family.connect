//! ONE CACHE, ONE THREAD — a port of
//! `win/tests/FamilyConnect.Core.Tests/Store/CacheConcurrencyTests.cs`.
//!
//! In the C# app the socket writes frames into the cache on its own thread, the resync applies
//! pages on the thread pool, and the window reads on the UI thread — all through the one connection
//! the cache owns — so every operation holds a lock for its whole length, and these tests hold the
//! cache on one thread and insist an operation started on another WAITS.
//!
//! Here there is no other thread to start it on, and that is the guarantee, given by the types
//! (`linux/docs/port-plan.md`, "Async"): the cache is `!Sync` and every store holds it through an
//! `Rc`, so none of them can be sent to or shared with another thread (checked at compile time in
//! `an_operation_waits_for_the_one_in_flight`), and every store operation is a synchronous `fn`
//! that never yields, so on the one thread an operation is finished — transaction and all — before
//! the next can begin. What the C# tests check besides survives unchanged: that EVERY named
//! operation takes the per-operation hold (counted by `Database::holds`), that the list names every
//! store operation there is, and that a writer and a reader interleaved the way the app's socket
//! and window are — here two tasks on one `LocalPool`, yielding between operations — leave the
//! cache whole.

mod common;

use std::cell::Cell;
use std::future::Future;
use std::rc::Rc;
use std::task::Poll;

use common::TempDir;
use fc_core::protocol::{
    ApiError, AttachmentDto, ChatDto, ChatRowDto, MemberDto, MessageDto, NoteDto, PackItemDto,
    PollDto, PollOptionDto, ReactionDto, ReconnectBackoff, UserDto, error_codes,
};
use fc_core::store::{
    BoardStore, ChatStore, Database, KeptTranscript, OutboxRow, OutboxStore, PackLimits, PackStore,
    SeqRoute, StoreResult, TranscriptStore,
};
use fc_text::board::Marks;
use futures::executor::LocalPool;
use time::OffsetDateTime;

const SENT: &str = "2026-09-13T10:00:00Z";

/// The cache and its stores, as the C# fixture builds them — a FILE, as there.
struct Cache {
    cache: Rc<Database>,
    chats: ChatStore,
    board: BoardStore,
    outbox: OutboxStore,
    pack: PackStore,
    transcripts: TranscriptStore,
    _directory: TempDir,
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
        created_at: Some(SENT.to_string()),
        pack_seq: seq,
        ..PackItemDto::new(id)
    }
}

fn text_note(id: i64, text: &str, seq: i64) -> NoteDto {
    NoteDto {
        kind: Some("text".to_string()),
        text: Some(text.to_string()),
        board_seq: seq,
        content_seq: Some(seq),
        ..NoteDto::new(id, 7)
    }
}

fn family() -> ChatRowDto {
    ChatRowDto::new(ChatDto::new(42, "family", "The Smiths"))
}

fn anna() -> MemberDto {
    MemberDto {
        role: Some("owner".to_string()),
        ..MemberDto::new(7, "anna", "Anna")
    }
}

fn cache() -> StoreResult<Cache> {
    let directory = TempDir::new("fc-concurrency");
    let cache = Database::open(directory.file("cache.db"))?;
    let chats = ChatStore::new(&cache).with_me(|| 7);
    let board = BoardStore::new(&cache);
    let outbox = OutboxStore::new(&cache);
    let pack = PackStore::new(&cache);
    let transcripts = TranscriptStore::new(&cache);
    transcripts.keep(
        &KeptTranscript::new(34, "See you at six"),
        OffsetDateTime::UNIX_EPOCH,
    )?;
    pack.replace(&[pack_item(1, 5)], 5)?;
    chats.replace(&[family()], None)?;
    chats.replace_members(&[anna(), MemberDto::new(11, "bob", "Bob")], &[])?;
    chats.apply(
        &MessageDto::new(1, 42, 11, Some("c-1"), "hello", SENT),
        SeqRoute::CatchUpPage,
        true,
    )?;
    board.replace(&[text_note(1, "Milk", 5)], 5)?;
    outbox.queue(&OutboxRow {
        pending_files: Some(vec!["photo.jpg".to_string()]),
        staged_files: Some(vec!["photo.jpg".to_string()]),
        queued_at: Some(OffsetDateTime::UNIX_EPOCH),
        ..OutboxRow::new("c-9", 42, "queued")
    })?;
    Ok(Cache {
        cache,
        chats,
        board,
        outbox,
        pack,
        transcripts,
        _directory: directory,
    })
}

type Operation<'a> = Box<dyn Fn() -> StoreResult<()> + 'a>;

/// Every public operation of the stores and the cache, by name, with the C#'s inputs.
fn operations(c: &Cache) -> Vec<(&'static str, Operation<'_>)> {
    fn op<'a, T>(run: impl Fn() -> StoreResult<T> + 'a) -> Operation<'a> {
        Box::new(move || run().map(drop))
    }
    let group = || ChatDto {
        member_ids: Some(vec![7, 11]),
        admin_user_id: Some(7),
        ..ChatDto::new(77, "group", "Cousins")
    };
    let poll = || PollDto {
        poll_seq: 5,
        closed: false,
        options: vec![PollOptionDto {
            id: 1,
            text: "Pizza".to_string(),
            votes: vec![],
        }],
    };
    let limits = PackLimits {
        max_items: 200,
        max_item_bytes: 524_288,
    };
    vec![
        ("chats.replace", op(|| c.chats.replace(&[family()], None))),
        ("chats.chats", op(|| c.chats.chats())),
        ("chats.chat", op(|| c.chats.chat(42))),
        ("chats.kind", op(|| c.chats.kind(42))),
        ("chats.is_listed", op(|| c.chats.is_listed(42))),
        ("chats.list_mark", op(|| Ok(c.chats.list_mark()))),
        ("chats.upsert", op(move || c.chats.upsert(&group()))),
        ("chats.remove", op(|| c.chats.remove(78))),
        ("chats.unread", op(|| c.chats.unread())),
        ("chats.mark_read", op(|| c.chats.mark_read(42, 1))),
        (
            "chats.advance",
            op(|| c.chats.advance(42, Some(3), None, None)),
        ),
        (
            "chats.apply",
            op(|| {
                c.chats.apply(
                    &MessageDto::new(2, 42, 11, Some("c-2"), "again", SENT),
                    SeqRoute::CatchUpPage,
                    true,
                )
            }),
        ),
        (
            "chats.apply_page",
            op(|| {
                c.chats.apply_page(
                    &[MessageDto::new(3, 42, 11, Some("c-3"), "and again", SENT)],
                    SeqRoute::CatchUpPage,
                    true,
                )
            }),
        ),
        (
            "chats.apply_reactions",
            op(|| {
                c.chats.apply_reactions(
                    42,
                    1,
                    4,
                    &[ReactionDto::new(11, "+1")],
                    SeqRoute::LiveFrame,
                )
            }),
        ),
        (
            "chats.apply_poll",
            op(move || c.chats.apply_poll(42, 1, &poll(), SeqRoute::LiveFrame)),
        ),
        ("chats.message", op(|| c.chats.message(1))),
        ("chats.messages", op(|| c.chats.messages(42, None, 50))),
        ("chats.thread", op(|| c.chats.thread(1))),
        ("chats.refresh", op(|| c.chats.refresh(&[]))),
        ("chats.polls", op(|| c.chats.polls(42))),
        ("chats.newest", op(|| c.chats.newest(42))),
        ("chats.catch_up_cursor", op(|| c.chats.catch_up_cursor(42))),
        ("chats.catch_up_cursors", op(|| c.chats.catch_up_cursors())),
        (
            "chats.flag_repair_candidates",
            op(|| c.chats.flag_repair_candidates(25)),
        ),
        ("chats.settle_flags", op(|| c.chats.settle_flags(1))),
        (
            "chats.repair_media",
            op(|| {
                c.chats
                    .repair_media(&MessageDto::new(1, 42, 11, Some("c-1"), "", SENT))
            }),
        ),
        (
            "chats.replace_members",
            op(|| c.chats.replace_members(&[anna()], &[])),
        ),
        (
            "chats.replace_members_of",
            op(|| c.chats.replace_members_of(Some(3), &[anna()], &[])),
        ),
        ("chats.members", op(|| c.chats.members())),
        ("chats.members_of", op(|| c.chats.members_of(3))),
        ("chats.roster_families", op(|| c.chats.roster_families())),
        ("chats.member", op(|| c.chats.member(11))),
        (
            "chats.joined",
            op(|| {
                c.chats
                    .joined(&UserDto::new(12, "carl", "Carl"), None, None)
            }),
        ),
        ("chats.left", op(|| c.chats.left(11, None))),
        (
            "chats.deleted",
            op(|| {
                c.chats.deleted(&MemberDto {
                    deleted: true,
                    ..MemberDto::new(11, "bob", "Bob")
                })
            }),
        ),
        ("chats.set_owner", op(|| c.chats.set_owner(11, None))),
        (
            "chats.replace_blocked",
            op(|| c.chats.replace_blocked(&[11])),
        ),
        ("chats.set_blocked", op(|| c.chats.set_blocked(11, true))),
        ("chats.blocked", op(|| c.chats.blocked())),
        ("chats.is_blocked", op(|| c.chats.is_blocked(11))),
        ("board.notes", op(|| c.board.notes())),
        ("board.note", op(|| c.board.note(1))),
        (
            "board.replace",
            op(|| c.board.replace(&[text_note(2, "Eggs", 6)], 6)),
        ),
        (
            "board.apply",
            op(|| {
                c.board
                    .apply(&text_note(3, "Bread", 7), SeqRoute::CatchUpPage)
            }),
        ),
        (
            "board.apply_page",
            op(|| {
                c.board
                    .apply_page(&[text_note(4, "Tea", 8)], SeqRoute::CatchUpPage)
            }),
        ),
        ("board.cursor", op(|| c.board.cursor())),
        ("board.marks", op(|| c.board.marks())),
        (
            "board.mark",
            op(|| {
                c.board.mark(Marks {
                    note_id: 1,
                    content_seq: 5,
                })
            }),
        ),
        ("board.mark_shown", op(|| c.board.mark_shown())),
        ("board.unread", op(|| c.board.unread())),
        ("pack.items", op(|| c.pack.items())),
        ("pack.item", op(|| c.pack.item(1))),
        ("pack.count", op(|| c.pack.count())),
        ("pack.replace", op(|| c.pack.replace(&[pack_item(2, 6)], 6))),
        (
            "pack.apply",
            op(|| c.pack.apply(&pack_item(3, 7), SeqRoute::CatchUpPage)),
        ),
        (
            "pack.apply_page",
            op(|| c.pack.apply_page(&[pack_item(4, 8)], SeqRoute::CatchUpPage)),
        ),
        ("pack.removed", op(|| c.pack.removed(1))),
        ("pack.cursor", op(|| c.pack.cursor())),
        (
            "pack.reconnected",
            op(|| {
                c.pack.reconnected();
                Ok(())
            }),
        ),
        ("pack.connection", op(|| Ok(c.pack.connection()))),
        ("pack.caught_up", op(|| Ok(c.pack.caught_up(0)))),
        ("pack.is_caught_up", op(|| Ok(c.pack.is_caught_up()))),
        ("pack.limits", op(|| c.pack.limits())),
        (
            "pack.set_limits",
            op(move || c.pack.set_limits(Some(limits))),
        ),
        (
            "pack.used",
            op(|| c.pack.used(1, OffsetDateTime::UNIX_EPOCH)),
        ),
        ("pack.recents", op(|| c.pack.recents(16))),
        ("transcripts.find", op(|| c.transcripts.find(34))),
        (
            "transcripts.keep",
            op(|| {
                c.transcripts
                    .keep(&KeptTranscript::new(35, ""), OffsetDateTime::UNIX_EPOCH)
            }),
        ),
        (
            "outbox.queue",
            op(|| {
                c.outbox.queue(&OutboxRow {
                    queued_at: Some(OffsetDateTime::UNIX_EPOCH),
                    ..OutboxRow::new("c-10", 42, "more")
                })
            }),
        ),
        ("outbox.all", op(|| c.outbox.all())),
        ("outbox.for_chat", op(|| c.outbox.for_chat(42))),
        ("outbox.due", op(|| c.outbox.due(OffsetDateTime::now_utc()))),
        ("outbox.find", op(|| c.outbox.find("c-9"))),
        ("outbox.delivered", op(|| c.outbox.delivered("c-9"))),
        (
            "outbox.failed",
            op(|| {
                c.outbox.failed(
                    "c-9",
                    &ApiError::transport("down"),
                    OffsetDateTime::now_utc(),
                    &mut ReconnectBackoff::default(),
                    true,
                )
            }),
        ),
        (
            "outbox.uploaded",
            op(|| c.outbox.uploaded("c-9", 34, "photo.jpg")),
        ),
        (
            "outbox.refuse",
            op(|| c.outbox.refuse("c-9", error_codes::BLOCKED)),
        ),
        ("outbox.reupload", op(|| c.outbox.reupload("c-9"))),
        ("outbox.retry", op(|| c.outbox.retry("c-9"))),
        ("outbox.discard", op(|| c.outbox.discard("c-9"))),
        ("cache.tables", op(|| c.cache.tables())),
        ("cache.columns", op(|| c.cache.columns("messages"))),
        ("cache.wipe_all", op(|| c.cache.wipe_all())),
        ("cache.user_version", op(|| c.cache.user_version())),
        ("cache.migrate", op(|| c.cache.migrate())),
        ("cache.drop_family", op(|| c.cache.drop_family(3))),
        ("cache.adopt_legacy", op(|| c.cache.adopt_legacy(3))),
        ("cache.held_families", op(|| c.cache.held_families())),
        (
            "board.in_family(3).notes",
            op(|| c.board.in_family(Some(3)).notes()),
        ),
        (
            "pack.in_family(3).items",
            op(|| c.pack.in_family(Some(3)).items()),
        ),
    ]
}

const NAMES: &[&str] = &[
    "chats.replace",
    "chats.chats",
    "chats.chat",
    "chats.kind",
    "chats.is_listed",
    "chats.list_mark",
    "chats.upsert",
    "chats.remove",
    "chats.unread",
    "chats.mark_read",
    "chats.advance",
    "chats.apply",
    "chats.apply_page",
    "chats.apply_reactions",
    "chats.apply_poll",
    "chats.message",
    "chats.messages",
    "chats.thread",
    "chats.refresh",
    "chats.polls",
    "chats.newest",
    "chats.catch_up_cursor",
    "chats.catch_up_cursors",
    "chats.flag_repair_candidates",
    "chats.settle_flags",
    "chats.repair_media",
    "chats.replace_members",
    "chats.replace_members_of",
    "chats.members",
    "chats.members_of",
    "chats.roster_families",
    "chats.member",
    "chats.joined",
    "chats.left",
    "chats.deleted",
    "chats.set_owner",
    "chats.replace_blocked",
    "chats.set_blocked",
    "chats.blocked",
    "chats.is_blocked",
    "board.notes",
    "board.note",
    "board.replace",
    "board.apply",
    "board.apply_page",
    "board.cursor",
    "board.marks",
    "board.mark",
    "board.mark_shown",
    "board.unread",
    "pack.items",
    "pack.item",
    "pack.count",
    "pack.replace",
    "pack.apply",
    "pack.apply_page",
    "pack.removed",
    "pack.cursor",
    "pack.reconnected",
    "pack.connection",
    "pack.caught_up",
    "pack.is_caught_up",
    "pack.limits",
    "pack.set_limits",
    "pack.used",
    "pack.recents",
    "transcripts.find",
    "transcripts.keep",
    "outbox.queue",
    "outbox.all",
    "outbox.for_chat",
    "outbox.due",
    "outbox.find",
    "outbox.delivered",
    "outbox.failed",
    "outbox.uploaded",
    "outbox.refuse",
    "outbox.reupload",
    "outbox.retry",
    "outbox.discard",
    "cache.tables",
    "cache.columns",
    "cache.wipe_all",
    "cache.user_version",
    "cache.migrate",
    "cache.drop_family",
    "cache.adopt_legacy",
    "cache.held_families",
    "board.in_family(3).notes",
    "pack.in_family(3).items",
];

/// Compiles only while `T` is NOT `Sync`: were it `Sync`, both impls would apply and
/// `<T as NotSync<_>>::CHECK` could not be inferred (the `static_assertions` crate's trick, written
/// out because the port takes no dependency for it).
trait NotSync<A> {
    const CHECK: () = ();
}
impl<T: ?Sized> NotSync<()> for T {}
impl<T: ?Sized + Sync> NotSync<u8> for T {}

/// The same for `Send`.
trait NotSend<A> {
    const CHECK: () = ();
}
impl<T: ?Sized> NotSend<()> for T {}
impl<T: ?Sized + Send> NotSend<u8> for T {}

/// EVERY OPERATION WAITS FOR THE ONE IN FLIGHT — the C# Theory, every name in turn on a fresh
/// cache. Here "waits" is the types' promise and is checked as one: no second thread can hold the
/// cache or a store at all, so on the one thread that can, an operation is always finished before
/// the next begins. What is checked at run time is what the C# test's failure means: an operation
/// that forgot to take the cache. Each one runs while the test holds the cache, as there, and must
/// hold it too — re-entrantly, which is the C#'s monitor's behaviour as well — and leave it as it
/// found it.
#[test]
fn an_operation_waits_for_the_one_in_flight() -> StoreResult<()> {
    // Nothing that reaches the cache can be shared with, or sent to, another thread.
    let () = <Database as NotSync<_>>::CHECK;
    let () = <Rc<Database> as NotSend<_>>::CHECK;
    let () = <ChatStore as NotSync<_>>::CHECK;
    let () = <ChatStore as NotSend<_>>::CHECK;
    let () = <BoardStore as NotSync<_>>::CHECK;
    let () = <BoardStore as NotSend<_>>::CHECK;
    let () = <OutboxStore as NotSync<_>>::CHECK;
    let () = <OutboxStore as NotSend<_>>::CHECK;
    let () = <PackStore as NotSync<_>>::CHECK;
    let () = <PackStore as NotSend<_>>::CHECK;
    let () = <TranscriptStore as NotSync<_>>::CHECK;
    let () = <TranscriptStore as NotSend<_>>::CHECK;

    for &name in NAMES {
        let fixture = cache()?;
        let operations = operations(&fixture);
        let (_, operation) = operations
            .iter()
            .find(|(named, _)| *named == name)
            .unwrap_or_else(|| panic!("{name} is not an operation"));
        let before = fixture.cache.holds();
        {
            let _held = fixture.cache.hold();
            operation().unwrap_or_else(|error| panic!("{name}: {error}"));
            assert!(
                fixture.cache.holds() > before + 1,
                "{name} used the cache without holding it"
            );
            assert!(fixture.cache.is_held());
        }
        assert!(!fixture.cache.is_held(), "{name} left the cache held");
    }
    Ok(())
}

/// The `pub fn`s of one `impl` block in a store's source, with the receiver each takes: the
/// stand-in for the C#'s reflection over a type's public instance members.
fn public_methods(source: &str, type_name: &str) -> Vec<(String, String)> {
    let header = format!("impl {type_name} {{");
    let start = source
        .lines()
        .position(|line| line == header)
        .unwrap_or_else(|| panic!("no `{header}`"));
    let mut methods = Vec::new();
    let lines: Vec<&str> = source.lines().skip(start + 1).collect();
    for (at, line) in lines.iter().enumerate() {
        if *line == "}" {
            break;
        }
        let Some(rest) = line.strip_prefix("    pub fn ") else {
            continue;
        };
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        // The receiver: what follows the `(`, on this line or the next.
        let after = rest.split_once('(').map_or("", |(_, after)| after).trim();
        let receiver = if after.is_empty() {
            lines.get(at + 1).map_or("", |next| next.trim())
        } else {
            after
        };
        methods.push((name, receiver.to_string()));
    }
    methods
}

/// And the list above is ALL of them: a store operation added later without being named here would
/// be an operation nobody checked holds the cache.
#[test]
fn every_store_operation_is_named() -> StoreResult<()> {
    let fixture = cache()?;
    let mut keys: Vec<&str> = operations(&fixture).iter().map(|(name, _)| *name).collect();
    let mut names = NAMES.to_vec();
    keys.sort_unstable();
    names.sort_unstable();
    assert_eq!(names, keys);
    let named: std::collections::HashSet<&str> = keys
        .iter()
        .map(|key| key.split('(').next().unwrap_or(key))
        .collect();
    for (prefix, source, type_name) in [
        (
            "chats",
            include_str!("../src/store/chat_store.rs"),
            "ChatStore",
        ),
        (
            "board",
            include_str!("../src/store/board_store.rs"),
            "BoardStore",
        ),
        (
            "outbox",
            include_str!("../src/store/outbox_store.rs"),
            "OutboxStore",
        ),
        (
            "pack",
            include_str!("../src/store/pack_store.rs"),
            "PackStore",
        ),
        (
            "transcripts",
            include_str!("../src/store/transcript_store.rs"),
            "TranscriptStore",
        ),
    ] {
        let methods = public_methods(source, type_name);
        assert!(!methods.is_empty(), "{type_name} has no methods to check");
        for (method, receiver) in methods {
            // Instance members only, as the C# asks reflection for: `new` and the associated
            // functions take no `self`, and the builders (the C#'s constructor arguments) take it
            // by value. The reader, the family shown and the home family are functions the store
            // was given, not the cache; and `in_family` makes another store, whose operations are
            // these same ones.
            if !receiver.starts_with("&self")
                || matches!(method.as_str(), "reader" | "family" | "home" | "in_family")
            {
                continue;
            }
            let key = format!("{prefix}.{method}");
            assert!(named.contains(key.as_str()), "{key} is not named");
        }
    }
    Ok(())
}

/// A future that is pending once and then done: the turn a task gives the other between two
/// operations, as the GLib main context interleaves the socket's handler and the window's.
fn yield_now() -> impl Future<Output = ()> {
    let mut yielded = false;
    futures::future::poll_fn(move |context| {
        if yielded {
            Poll::Ready(())
        } else {
            yielded = true;
            context.waker().wake_by_ref();
            Poll::Pending
        }
    })
}

/// The same, the way the app does it: a writer and a reader, flat out, at once — on one thread,
/// interleaved between operations, for a fixed number of pages rather than two seconds of wall
/// clock.
#[test]
fn the_socket_writes_while_the_window_reads() -> StoreResult<()> {
    const PAGES: i64 = 100;
    let fixture = cache()?;
    let written = Cell::new(0_i64);

    let writer = async {
        let mut id = 100_i64;
        for _ in 0..PAGES {
            let page: Vec<MessageDto> = (0..20)
                .map(|at| {
                    let message_id = id + at;
                    MessageDto::new(
                        message_id,
                        42,
                        11,
                        None,
                        &format!("message {message_id}"),
                        SENT,
                    )
                })
                .collect();
            id += 20;
            fixture
                .chats
                .apply_page(&page, SeqRoute::CatchUpPage, true)?;
            written.set(id - 1);
            yield_now().await;
        }
        StoreResult::Ok(())
    };
    let reader = async {
        for _ in 0..PAGES {
            fixture.chats.messages(42, None, 50)?;
            fixture.chats.chats()?;
            fixture.board.unread()?;
            fixture.outbox.due(OffsetDateTime::now_utc())?;
            yield_now().await;
        }
        StoreResult::Ok(())
    };

    let (wrote, read) = LocalPool::new().run_until(futures::future::join(writer, reader));
    wrote?;
    read?;

    assert_eq!(
        fixture.chats.newest(42)?.map(|newest| newest.id),
        Some(written.get())
    );
    Ok(())
}
