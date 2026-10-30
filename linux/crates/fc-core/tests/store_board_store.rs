//! The wall as this device holds it: the two protocol rules that decide what a cache may keep
//! (docs/protocol.md, "Board") — a port of
//! `win/tests/FamilyConnect.Core.Tests/Store/BoardStoreTests.cs`.
//!
//! `AnUnknownNameIsKeptRatherThanNormalised` asks the board's drawing rules too; those are
//! `fc_text::board`'s here (`Size::from_name`, `color_hex`, `Kind::from_name`,
//! `Size::patch_name`), as `crate::board` documents.

use std::rc::Rc;

use fc_core::protocol::{AttachmentDto, NoteDto, RsvpDto};
use fc_core::store::{BoardStore, Database, SeqRoute, StoreResult};
use fc_text::board::{Kind, Marks, Size, color_hex};

use SeqRoute::{CatchUpPage, Evidence, LiveFrame};

fn open() -> StoreResult<Rc<Database>> {
    Database::open_in_memory()
}

/// `Note(id, seq)`: a yellow medium plain text note saying "Milk", by 7.
fn note(id: i64, seq: i64) -> NoteDto {
    NoteDto {
        kind: Some("text".to_string()),
        text: Some("Milk".to_string()),
        color: Some("yellow".to_string()),
        size: Some("medium".to_string()),
        font: Some("plain".to_string()),
        x: 0.25,
        y: 0.5,
        created_at: Some("2026-09-12T08:00:00Z".to_string()),
        updated_at: Some("2026-09-12T08:00:00Z".to_string()),
        board_seq: seq,
        ..NoteDto::new(id, 7)
    }
}

fn saying(id: i64, seq: i64, text: &str) -> NoteDto {
    NoteDto {
        text: Some(text.to_string()),
        ..note(id, seq)
    }
}

fn content(id: i64, seq: i64, content_seq: i64) -> NoteDto {
    NoteDto {
        content_seq: Some(content_seq),
        ..note(id, seq)
    }
}

fn tombstone(id: i64, seq: i64) -> NoteDto {
    NoteDto {
        deleted: true,
        ..note(id, seq)
    }
}

fn ids(notes: &[NoteDto]) -> Vec<i64> {
    notes.iter().map(|note| note.id).collect()
}

fn held(store: &BoardStore, note_id: i64) -> StoreResult<NoteDto> {
    Ok(store.note(note_id)?.expect("a held note"))
}

#[test]
fn a_note_goes_in_and_comes_back_the_same() -> StoreResult<()> {
    let database = open()?;
    let store = BoardStore::new(&database);
    store.apply(&content(12, 88, 84), CatchUpPage)?;
    let held = store.note(12)?;
    assert!(held.is_some());
    let held = held.expect("the note");
    assert_eq!(held.text.as_deref(), Some("Milk"));
    assert_eq!(held.x, 0.25);
    assert_eq!(held.board_seq, 88);
    assert_eq!(held.content_seq, Some(84));
    assert_eq!(held.created_at.as_deref(), Some("2026-09-12T08:00:00.000Z"));
    assert_eq!(store.cursor()?, 88);
    Ok(())
}

/// A FULL READ REPLACES what is held. A note somebody took down while this device was away would
/// otherwise stay on the wall for ever.
#[test]
fn a_whole_board_read_is_the_wall() -> StoreResult<()> {
    let database = open()?;
    let store = BoardStore::new(&database);
    store.apply(&note(1, 10), CatchUpPage)?;
    store.apply(&note(2, 11), CatchUpPage)?;
    store.replace(&[note(2, 11), note(3, 12)], 12)?;
    assert_eq!(ids(&store.notes()?), [3, 2]);
    assert_eq!(store.note(1)?, None);
    assert_eq!(store.cursor()?, 12);
    Ok(())
}

/// The changes feed carries TOMBSTONES, and a tombstone is a delete.
#[test]
fn a_tombstone_takes_the_note_down() -> StoreResult<()> {
    let database = open()?;
    let store = BoardStore::new(&database);
    store.apply(&note(12, 88), CatchUpPage)?;
    assert!(store.apply(&tombstone(12, 90), CatchUpPage)?);
    assert_eq!(store.note(12)?, None);
    assert!(store.notes()?.is_empty());
    // And the cursor still moves: the delete is a change like any other.
    assert_eq!(store.cursor()?, 90);
    // Idempotent — a tombstone this device has already applied changes nothing.
    assert!(!store.apply(&tombstone(12, 91), CatchUpPage)?);
    Ok(())
}

/// SEQS COMMIT OUT OF ORDER, so a slower answer must never overwrite a newer one it crossed on the
/// wire.
#[test]
fn an_older_answer_is_dropped() -> StoreResult<()> {
    let database = open()?;
    let store = BoardStore::new(&database);
    store.apply(&saying(12, 90, "the newer one"), CatchUpPage)?;
    assert!(!store.apply(&saying(12, 88, "the older one"), CatchUpPage)?);
    assert_eq!(held(&store, 12)?.text.as_deref(), Some("the newer one"));
    // The same seq is not newer either: a re-read of what is held changes nothing.
    assert!(!store.apply(&saying(12, 90, "again"), CatchUpPage)?);
    assert_eq!(held(&store, 12)?.text.as_deref(), Some("the newer one"));
    Ok(())
}

/// A TOMBSTONE IS THE LAST WORD. A delete is the last thing that happens to a note, but seqs commit
/// out of order and a history page carries the pre-delete copy — so a note that has been taken down
/// must not come back when an older answer crosses the tombstone on the wire.
#[test]
fn a_note_taken_down_stays_down_however_late_an_older_copy_arrives() -> StoreResult<()> {
    let database = open()?;
    let store = BoardStore::new(&database);
    store.apply_page(&[note(12, 11), note(13, 12)], CatchUpPage)?;

    assert!(store.apply(&tombstone(12, 13), CatchUpPage)?);
    assert_eq!(ids(&store.notes()?), [13]);

    // The copy that was already travelling when the delete happened — a catch-up page, or a frame
    // that crossed it.
    assert!(!store.apply(&saying(12, 99, "back from the dead"), CatchUpPage)?);
    assert_eq!(ids(&store.notes()?), [13]);

    // And a full read that still lists it puts nothing back either.
    store.replace(&[note(12, 11), note(13, 12)], 12)?;
    assert_eq!(ids(&store.notes()?), [13]);
    Ok(())
}

/// A FULL READ REPLACES THE WALL — except what is NEWER than the read's own mark. A frame that
/// landed while the read was in flight is not on that answer, and wiping it would take a note off
/// the wall seconds after somebody pinned it.
#[test]
fn a_full_read_keeps_what_arrived_after_the_read_was_taken() -> StoreResult<()> {
    let database = open()?;
    let store = BoardStore::new(&database);
    // A frame, ahead of the read that is about to land.
    store.apply(&saying(20, 30, "just pinned"), LiveFrame)?;

    store.replace(&[note(12, 11), note(13, 12)], 12)?;

    assert_eq!(ids(&store.notes()?), [20, 13, 12]);
    assert_eq!(held(&store, 20)?.text.as_deref(), Some("just pinned"));
    // The cursor is the read's mark: the server read it BEFORE the notes, so it is never past a
    // change they missed.
    assert_eq!(store.cursor()?, 12);
    Ok(())
}

/// THE CURSOR IS NOT MOVED BY EVERY WRITE. A live frame and a page of the feed move it; the answer
/// to this client's own write is evidence about one note and says nothing about what else has
/// happened, so a cursor moved by it steps the feed past somebody else's change.
#[test]
fn only_a_frame_or_a_page_moves_the_boards_cursor() -> StoreResult<()> {
    let database = open()?;
    let store = BoardStore::new(&database);

    store.apply(&note(12, 11), Evidence)?;
    assert_eq!(held(&store, 12)?.text.as_deref(), Some("Milk"));
    assert_eq!(store.cursor()?, 0);

    // A FRAME BEFORE THIS DEVICE HAS EVER READ THE BOARD moves nothing either: the cursor being 0
    // is what tells the resync to read the whole wall, and a frame that jumped that queue would
    // leave it above changes nobody had read.
    store.apply(&note(13, 12), LiveFrame)?;
    assert_eq!(held(&store, 13)?.text.as_deref(), Some("Milk"));
    assert_eq!(store.cursor()?, 0);

    // The whole wall, read once…
    store.replace(&[note(12, 11), note(13, 12)], 12)?;
    assert_eq!(store.cursor()?, 12);

    // …and from here a frame does move it.
    store.apply(&note(14, 20), LiveFrame)?;
    assert_eq!(store.cursor()?, 20);

    store.apply_page(&[note(15, 21)], CatchUpPage)?;
    assert_eq!(store.cursor()?, 21);

    // And a page applied as evidence — the answer to our own write, which never comes as a page in
    // practice — moves nothing either.
    store.apply_page(&[note(16, 22)], Evidence)?;
    assert_eq!(store.cursor()?, 21);
    Ok(())
}

/// TWO FULL READS CAN LAND IN EITHER ORDER, and an older one landing second must change nothing:
/// it would drop every note written between the two and set the cursor back.
#[test]
fn a_full_read_older_than_one_already_applied_is_ignored() -> StoreResult<()> {
    let database = open()?;
    let store = BoardStore::new(&database);
    store.replace(&[note(12, 11), note(13, 12)], 12)?;

    // The slower read, taken before the faster one and arriving after it.
    store.replace(&[note(12, 11)], 10)?;

    assert_eq!(ids(&store.notes()?), [13, 12]);
    assert_eq!(store.cursor()?, 12);
    Ok(())
}

#[test]
fn a_page_of_changes_is_applied_in_order_and_moves_the_cursor_once() -> StoreResult<()> {
    let database = open()?;
    let store = BoardStore::new(&database);
    let changed = store.apply_page(
        &[
            note(1, 10),
            note(2, 11),
            saying(1, 12, "rewritten"),
            tombstone(3, 13),
        ],
        CatchUpPage,
    )?;
    // Two notes written, one rewritten, and a tombstone for a note this device never had.
    assert_eq!(changed, 3);
    assert_eq!(held(&store, 1)?.text.as_deref(), Some("rewritten"));
    assert_eq!(store.cursor()?, 13);
    Ok(())
}

/// `[]` and ABSENT are different answers all the way down: an event nobody has answered sends `[]`,
/// a list with nothing on it sends `[]`, and every other kind sends nothing at all.
#[test]
fn an_empty_list_is_not_a_missing_one() -> StoreResult<()> {
    let database = open()?;
    let store = BoardStore::new(&database);
    store.apply(
        &NoteDto {
            kind: Some("tasks".to_string()),
            items: Some(vec![]),
            ..note(1, 10)
        },
        CatchUpPage,
    )?;
    store.apply(
        &NoteDto {
            kind: Some("event".to_string()),
            rsvps: Some(vec![]),
            starts_at: Some("2026-12-24T17:00:00Z".to_string()),
            ..note(2, 11)
        },
        CatchUpPage,
    )?;
    store.apply(&note(3, 12), CatchUpPage)?;
    assert!(held(&store, 1)?.items.expect("a list").is_empty());
    assert!(held(&store, 1)?.items.is_some());
    assert!(held(&store, 2)?.rsvps.is_some());
    assert!(held(&store, 2)?.rsvps.expect("answers").is_empty());
    // A text note is not a list and not an event: no items, no answers.
    assert_eq!(held(&store, 3)?.items, None);
    assert_eq!(held(&store, 3)?.rsvps, None);
    assert!(held(&store, 3)?.task_list().is_empty());
    Ok(())
}

#[test]
fn an_event_keeps_its_when_where_who_and_its_backdrop() -> StoreResult<()> {
    let database = open()?;
    let store = BoardStore::new(&database);
    store.apply(
        &NoteDto {
            kind: Some("event".to_string()),
            starts_at: Some("2026-12-24T17:00:00Z".to_string()),
            ends_at: Some("2026-12-24T19:30:00Z".to_string()),
            place: Some("Gran's".to_string()),
            rsvps: Some(vec![
                RsvpDto {
                    user_id: 7,
                    answer: "going".to_string(),
                },
                RsvpDto {
                    user_id: 9,
                    answer: "maybe".to_string(),
                },
            ]),
            attachment: Some(AttachmentDto {
                mime: Some("image/png".to_string()),
                width: Some(1024),
                height: Some(1024),
                ..AttachmentDto::new(900, "photo")
            }),
            ..saying(13, 20, "Gran's birthday")
        },
        CatchUpPage,
    )?;
    let held = held(&store, 13)?;
    assert_eq!(held.starts_at.as_deref(), Some("2026-12-24T17:00:00.000Z"));
    assert_eq!(held.ends_at.as_deref(), Some("2026-12-24T19:30:00.000Z"));
    assert_eq!(held.place.as_deref(), Some("Gran's"));
    assert_eq!(held.count("going"), 1);
    assert_eq!(held.my_answer(9), Some("maybe"));
    // The BACKDROP, with the flag that decides which bytes a card asks for.
    let backdrop = held.attachment.expect("a backdrop");
    assert_eq!(backdrop.id, 900);
    assert!(!backdrop.has_preview);
    assert_eq!(backdrop.aspect_ratio(), Some(1.0));
    Ok(())
}

/// An unknown kind, colour, size or face is kept AS SENT: a fourth size from a newer server draws
/// as medium and must not be written back as medium by an edit.
#[test]
fn an_unknown_name_is_kept_rather_than_normalised() -> StoreResult<()> {
    let database = open()?;
    let store = BoardStore::new(&database);
    store.apply(
        &NoteDto {
            size: Some("huge".to_string()),
            color: Some("chartreuse".to_string()),
            font: Some("comic".to_string()),
            kind: Some("hologram".to_string()),
            ..note(1, 10)
        },
        CatchUpPage,
    )?;
    let held = held(&store, 1)?;
    assert_eq!(held.size.as_deref(), Some("huge"));
    assert_eq!(held.color.as_deref(), Some("chartreuse"));
    assert_eq!(held.font.as_deref(), Some("comic"));
    assert_eq!(held.kind.as_deref(), Some("hologram"));
    // And the drawing rules still resolve them to the fallbacks.
    assert_eq!(Size::from_name(held.size.as_deref()), Size::Medium);
    assert_eq!(color_hex(held.color.as_deref().unwrap_or("")), "#fff2b3");
    assert_eq!(Kind::from_name(held.kind.as_deref()), Kind::Text);
    // An untouched picker sends nothing, so the name survives an edit.
    assert_eq!(Size::Medium.patch_name(held.size.as_deref()), None);
    Ok(())
}

/// The badge counts what a note SAYS, and the marks are what this DEVICE has shown — moved only
/// when the board has been on screen, and never backwards.
#[test]
fn the_badge_counts_what_is_new_to_read_and_the_marks_never_go_back() -> StoreResult<()> {
    let database = open()?;
    let store = BoardStore::new(&database);
    store.apply(&content(1, 10, 10), CatchUpPage)?;
    store.apply(&content(2, 11, 11), CatchUpPage)?;
    assert_eq!(store.unread()?, 2);
    store.mark_shown()?;
    assert_eq!(store.unread()?, 0);
    assert_eq!(
        store.marks()?,
        Marks {
            note_id: 2,
            content_seq: 11
        }
    );

    // A move is not news: a new board_seq with the same content_seq raises nothing.
    store.apply(
        &NoteDto {
            x: 0.9,
            ..content(1, 12, 10)
        },
        CatchUpPage,
    )?;
    assert_eq!(store.unread()?, 0);
    // A rewrite is.
    store.apply(
        &NoteDto {
            text: Some("Milk and eggs".to_string()),
            ..content(1, 13, 13)
        },
        CatchUpPage,
    )?;
    assert_eq!(store.unread()?, 1);

    // And the newest note going away must not bring a cleared badge back.
    store.mark_shown()?;
    store.apply(&tombstone(1, 14), CatchUpPage)?;
    assert_eq!(store.unread()?, 0);
    assert_eq!(store.marks()?.content_seq, 13);

    // TWO WINDOWS OF ONE ACCOUNT both write these, and neither may walk the other's back: marks
    // that arrive BEHIND what is held change nothing. `mark_shown` cannot show this on its own — it
    // hands in the maximum it just computed — so the stale set is written directly, which is what a
    // second window's older snapshot looks like.
    store.mark(Marks {
        note_id: 1,
        content_seq: 5,
    })?;
    // Both fields stand: the note id is still 2 (a note that has since been deleted, which is
    // exactly why the mark may not go back) and the content seq is still 13.
    assert_eq!(
        store.marks()?,
        Marks {
            note_id: 2,
            content_seq: 13
        }
    );
    assert_eq!(store.unread()?, 0);
    Ok(())
}

#[test]
fn a_note_from_a_server_with_no_content_seq_is_judged_by_its_id() -> StoreResult<()> {
    let database = open()?;
    let store = BoardStore::new(&database);
    store.apply(&note(5, 10), CatchUpPage)?;
    // 0 is this cache's spelling of "the server never said", which reads back as absent.
    assert_eq!(held(&store, 5)?.content_seq, None);
    assert_eq!(store.unread()?, 1);
    store.mark(Marks {
        note_id: 5,
        content_seq: 0,
    })?;
    assert_eq!(store.unread()?, 0);
    Ok(())
}
