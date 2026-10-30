//! The family's sticker pack as this device holds it — the board's machinery one table over
//! (docs/protocol.md, "Sticker pack"): a full read replaces, a removal is remembered, every write
//! is guarded by `pack_seq`, and the cursor moves in three ways and no others. A port of
//! `win/tests/FamilyConnect.Core.Tests/Store/PackStoreTests.cs`.

use std::rc::Rc;

use fc_core::protocol::{AttachmentDto, PackItemDto, wire};
use fc_core::store::{Database, PackLimits, PackStore, SeqRoute, StoreResult};
use time::macros::datetime;
use time::{Duration, OffsetDateTime};

use SeqRoute::{CatchUpPage, Evidence, LiveFrame};

fn open() -> StoreResult<Rc<Database>> {
    Database::open_in_memory()
}

fn picture(id: i64, size: i64) -> AttachmentDto {
    AttachmentDto {
        mime: Some("image/webp".to_string()),
        size: Some(size),
        width: Some(512),
        height: Some(512),
        ..AttachmentDto::new(id, "photo")
    }
}

/// `Item(id, seq)`: added by 7, a 4096-byte WebP, at 10:00, no label.
fn item(id: i64, seq: i64) -> PackItemDto {
    PackItemDto {
        added_by: 7,
        attachment: Some(picture(70 + id, 4096)),
        created_at: Some("2026-09-13T10:00:00Z".to_string()),
        pack_seq: seq,
        ..PackItemDto::new(id)
    }
}

fn labelled(id: i64, seq: i64, label: &str) -> PackItemDto {
    PackItemDto {
        label: Some(label.to_string()),
        ..item(id, seq)
    }
}

fn tombstone(id: i64, seq: i64) -> PackItemDto {
    PackItemDto {
        deleted: true,
        pack_seq: seq,
        ..PackItemDto::new(id)
    }
}

fn ids(items: &[PackItemDto]) -> Vec<i64> {
    items.iter().map(|item| item.id).collect()
}

#[test]
fn a_full_read_is_the_pack_in_the_order_it_was_added() -> StoreResult<()> {
    let database = open()?;
    let store = PackStore::new(&database);
    // The wire's order is id ascending, and the store answers it whatever order it was told in.
    assert!(store.replace(
        &[item(9, 14), labelled(5, 12, "party cat"), item(7, 13)],
        14
    )?);

    assert_eq!(ids(&store.items()?), [5, 7, 9]);
    assert_eq!(store.count()?, 3);
    assert_eq!(store.cursor()?, 14);
    let first = store.items()?.remove(0);
    assert_eq!(first.label.as_deref(), Some("party cat"));
    assert_eq!(first.added_by, 7);
    assert_eq!(first.pack_seq, 12);
    let attachment = first.attachment.expect("the picture");
    assert_eq!(attachment.mime.as_deref(), Some("image/webp"));
    assert_eq!(attachment.size, Some(4096));
    // No label is no label — never an empty one.
    assert_eq!(store.item(7)?.expect("an item").label, None);
    Ok(())
}

/// A FULL READ REPLACES: an item somebody removed while this device was away is not on the answer,
/// and must leave the panel.
#[test]
fn a_full_read_drops_what_it_does_not_carry() -> StoreResult<()> {
    let database = open()?;
    let store = PackStore::new(&database);
    store.replace(&[item(5, 12), item(6, 13)], 13)?;

    store.replace(&[item(6, 13)], 15)?;

    assert_eq!(ids(&store.items()?), [6]);
    assert_eq!(store.cursor()?, 15);
    Ok(())
}

/// …EXCEPT an item held above the read's own mark: it arrived after the read was taken, is not on
/// that answer, and stays.
#[test]
fn a_full_read_keeps_an_item_that_arrived_after_it_was_taken() -> StoreResult<()> {
    let database = open()?;
    let store = PackStore::new(&database);
    store.replace(&[item(5, 12)], 12)?;
    store.apply(&item(8, 20), LiveFrame)?;

    // A read taken at 14 cannot know about seq 20.
    store.replace(&[item(5, 12), item(6, 14)], 14)?;

    assert_eq!(ids(&store.items()?), [5, 6, 8]);
    Ok(())
}

/// Two full reads can land in either order; the older one landing second changes nothing.
#[test]
fn an_older_full_read_landing_second_is_ignored() -> StoreResult<()> {
    let database = open()?;
    let store = PackStore::new(&database);
    assert!(store.replace(&[item(5, 12), item(6, 14)], 14)?);

    assert!(!store.replace(&[item(5, 12)], 12)?);

    assert_eq!(ids(&store.items()?), [5, 6]);
    assert_eq!(store.cursor()?, 14);
    Ok(())
}

#[test]
fn a_tombstone_removes_and_is_remembered() -> StoreResult<()> {
    let database = open()?;
    let store = PackStore::new(&database);
    store.replace(&[item(5, 12), item(6, 13)], 13)?;

    assert!(store.apply(&tombstone(5, 14), CatchUpPage)?);
    assert_eq!(ids(&store.items()?), [6]);

    // THE GONE SET: ids are never reused, so an older copy arriving late — a frame that crossed the
    // removal, a slower page — never brings the item back.
    assert!(!store.apply(&item(5, 12), LiveFrame)?);
    assert!(!store.apply(&item(5, 99), CatchUpPage)?);
    store.replace(&[item(5, 12), item(6, 13)], 20)?;
    assert_eq!(ids(&store.items()?), [6]);
    Ok(())
}

/// A tombstone for an item this device never held is remembered, and is not news.
#[test]
fn a_tombstone_for_an_item_never_held_changes_nothing_drawn() -> StoreResult<()> {
    let database = open()?;
    let store = PackStore::new(&database);

    assert!(!store.apply(&tombstone(5, 14), CatchUpPage)?);

    assert!(!store.apply(&item(5, 12), CatchUpPage)?);
    assert!(store.items()?.is_empty());
    Ok(())
}

/// A full read that LEFT AN ITEM OUT has seen it removed, exactly as a tombstone would have said —
/// so a late frame carrying the old copy cannot resurrect it.
#[test]
fn a_full_read_that_left_an_item_out_remembers_it_as_gone() -> StoreResult<()> {
    let database = open()?;
    let store = PackStore::new(&database);
    store.replace(&[item(5, 12), item(6, 13)], 13)?;

    store.replace(&[item(6, 13)], 15)?;

    assert!(!store.apply(&item(5, 12), LiveFrame)?);
    assert_eq!(ids(&store.items()?), [6]);
    Ok(())
}

/// An item is written only when the incoming seq is GREATER than the one held.
#[test]
fn an_item_is_written_only_when_its_seq_is_newer() -> StoreResult<()> {
    let database = open()?;
    let store = PackStore::new(&database);
    store.apply(&labelled(5, 12, "party cat"), CatchUpPage)?;

    assert!(!store.apply(&labelled(5, 12, "same seq"), CatchUpPage)?);
    assert!(!store.apply(&labelled(5, 11, "older"), CatchUpPage)?);

    assert_eq!(
        store.item(5)?.expect("the item").label.as_deref(),
        Some("party cat")
    );
    Ok(())
}

/// A live item is its picture; one without it is nothing this client can draw or send.
#[test]
fn a_live_item_with_no_picture_is_not_stored() -> StoreResult<()> {
    let database = open()?;
    let store = PackStore::new(&database);

    assert!(!store.apply(
        &PackItemDto {
            added_by: 7,
            pack_seq: 12,
            ..PackItemDto::new(5)
        },
        CatchUpPage
    )?);

    assert!(store.items()?.is_empty());
    Ok(())
}

// ---- the cursor: three ways and no others --------------------------------------------------------

#[test]
fn a_catch_up_page_moves_the_cursor_to_its_highest_seq() -> StoreResult<()> {
    let database = open()?;
    let store = PackStore::new(&database);
    store.replace(&[item(5, 12)], 12)?;

    // Tombstones are in the feed and move the seq like anything else. Three things changed what is
    // drawn: two stickers arrived and one left.
    assert_eq!(
        store.apply_page(&[item(6, 13), tombstone(5, 17), item(7, 15)], CatchUpPage)?,
        3
    );

    assert_eq!(store.cursor()?, 17);
    assert_eq!(ids(&store.items()?), [6, 7]);
    Ok(())
}

/// A FRAME MOVES THE CURSOR ONLY ONCE THIS CONNECTION HAS CAUGHT UP. Before that the cursor is
/// where the catch-up starts from, and a frame that jumped it would leave every change made while
/// the socket was down unread for good.
#[test]
fn a_frame_does_not_move_the_cursor_until_the_connection_has_caught_up() -> StoreResult<()> {
    let database = open()?;
    let store = PackStore::new(&database);
    store.replace(&[item(5, 12)], 12)?;
    store.reconnected();

    // The item is applied — it is drawn at once — and the cursor stays where the feed needs it.
    assert!(store.apply(&item(9, 20), LiveFrame)?);
    assert_eq!(store.cursor()?, 12);
    assert!(!store.is_caught_up());

    // The catch-up then reads 13..19 from 12, which is the whole point.
    store.apply_page(&[item(6, 13), item(9, 20)], CatchUpPage)?;
    store.caught_up(store.connection());
    assert_eq!(store.cursor()?, 20);

    store.apply(&item(10, 21), LiveFrame)?;
    assert_eq!(store.cursor()?, 21);
    Ok(())
}

/// "Caught up" is said OF A CONNECTION. A pass that began on one and finishes after another has
/// opened is holding answers older than the socket now listening, and its word is dropped: a frame
/// on the new connection still may not move the cursor.
#[test]
fn catching_up_an_earlier_connection_does_not_catch_up_this_one() -> StoreResult<()> {
    let database = open()?;
    let store = PackStore::new(&database);
    store.replace(&[item(5, 12)], 12)?;
    store.reconnected();
    let began = store.connection();
    store.reconnected();

    assert!(!store.caught_up(began));
    assert!(!store.is_caught_up());
    store.apply(&item(9, 20), LiveFrame)?;
    assert_eq!(store.cursor()?, 12);

    // The pass the newer connection started is the one that may say it.
    assert!(store.caught_up(store.connection()));
    store.apply(&item(10, 21), LiveFrame)?;
    assert_eq!(store.cursor()?, 21);
    Ok(())
}

/// A relaunch is a new connection: nothing has been caught up on yet.
#[test]
fn a_fresh_store_has_not_caught_up() -> StoreResult<()> {
    let database = open()?;
    let first = PackStore::new(&database);
    first.replace(&[item(5, 12)], 12)?;
    first.caught_up(first.connection());

    let relaunched = PackStore::new(&database);
    relaunched.apply(&item(9, 20), LiveFrame)?;

    assert!(!relaunched.is_caught_up());
    assert_eq!(relaunched.cursor()?, 12);
    Ok(())
}

/// The item in the answer to this device's own POST — and what this device makes of its own
/// DELETE — is applied under the per-item guard and MOVES NO CURSOR.
#[test]
fn evidence_moves_no_cursor() -> StoreResult<()> {
    let database = open()?;
    let store = PackStore::new(&database);
    store.replace(&[item(5, 12)], 12)?;
    store.caught_up(store.connection());

    assert!(store.apply(&item(9, 30), Evidence)?);
    assert!(store.removed(5)?);
    assert_eq!(store.apply_page(&[item(10, 31)], Evidence)?, 1);

    assert_eq!(store.cursor()?, 12);
    assert_eq!(ids(&store.items()?), [9, 10]);
    Ok(())
}

/// This device's own DELETE, answered: the item leaves at once, its "recently used" goes with it,
/// and it is remembered as gone — so the frame for the ADD, arriving late, cannot put it back. Said
/// of an item this device never held, it changes nothing drawn and is still remembered.
#[test]
fn this_devices_own_removal_is_remembered_and_moves_nothing() -> StoreResult<()> {
    let database = open()?;
    let store = PackStore::new(&database);
    store.replace(&[item(5, 12), item(6, 13)], 13)?;
    store.used(5, OffsetDateTime::UNIX_EPOCH)?;

    assert!(store.removed(5)?);
    assert!(!store.removed(5)?);
    assert!(!store.removed(44)?);

    assert_eq!(ids(&store.items()?), [6]);
    assert!(store.recents(16)?.is_empty());
    assert_eq!(store.cursor()?, 13);
    assert!(!store.apply(&item(5, 12), LiveFrame)?);
    assert!(!store.apply(&item(44, 40), LiveFrame)?);
    Ok(())
}

// ---- staleness: the same guard for a live item and a tombstone -----------------------------------

/// AN ITEM — LIVE OR A TOMBSTONE — IS APPLIED ONLY WHEN ITS SEQ IS ABOVE THE ONE HELD. A tombstone
/// that is not newer than the copy held is an older state of the item, and changes nothing; the
/// real one, which always takes a newer seq than the add, removes.
#[test]
fn a_tombstone_is_applied_only_when_its_seq_is_above_the_one_held() -> StoreResult<()> {
    let database = open()?;
    let store = PackStore::new(&database);
    store.replace(&[item(5, 12)], 12)?;

    assert!(!store.apply(&tombstone(5, 12), LiveFrame)?);
    assert!(!store.apply(&tombstone(5, 11), LiveFrame)?);
    assert_eq!(store.apply_page(&[tombstone(5, 9)], Evidence)?, 0);
    assert_eq!(ids(&store.items()?), [5]);
    // Not applied is not remembered either: the item is still one a newer copy may replace.
    assert!(store.apply(&labelled(5, 13, "newer"), CatchUpPage)?);

    assert!(store.apply(&tombstone(5, 14), LiveFrame)?);
    assert!(store.items()?.is_empty());
    Ok(())
}

/// A FULL READ IS IGNORED WHEN ITS MARK IS BELOW THE CURSOR ALREADY APPLIED — whichever of the
/// three ways moved the cursor there. Here a catch-up page and then a frame on a caught-up
/// connection did; a full read taken before either knows less than this device does, and applying
/// it would drop what arrived since and set the cursor back behind it.
#[test]
fn a_full_read_below_the_cursor_already_applied_is_ignored() -> StoreResult<()> {
    let database = open()?;
    let store = PackStore::new(&database);
    assert!(store.replace(&[item(5, 12)], 12)?);
    store.apply_page(&[item(6, 15)], CatchUpPage)?;
    assert_eq!(store.cursor()?, 15);

    assert!(!store.replace(&[item(5, 12)], 14)?);
    assert_eq!(ids(&store.items()?), [5, 6]);
    assert_eq!(store.cursor()?, 15);

    store.caught_up(store.connection());
    store.apply(&item(7, 18), LiveFrame)?;
    assert!(!store.replace(&[item(5, 12), item(6, 15)], 17)?);
    assert_eq!(ids(&store.items()?), [5, 6, 7]);
    assert_eq!(store.cursor()?, 18);

    // Level with the cursor is not below it, and one above is simply newer.
    assert!(store.replace(&[item(5, 12), item(6, 15), item(7, 18)], 18)?);
    assert!(store.replace(&[item(6, 15), item(7, 18)], 19)?);
    assert_eq!(ids(&store.items()?), [6, 7]);
    assert_eq!(store.cursor()?, 19);
    Ok(())
}

/// A frame on a connection that has NOT caught up moves no cursor, so it cannot make a full read
/// look stale: the read this pass is about to take must still be applied.
#[test]
fn a_frame_before_catch_up_does_not_make_the_full_read_stale() -> StoreResult<()> {
    let database = open()?;
    let store = PackStore::new(&database);
    store.reconnected();
    let began = store.connection();
    assert!(store.apply(&item(9, 20), LiveFrame)?);
    assert_eq!(store.cursor()?, 0);

    // Taken before the frame's change committed: its mark is below the item held, which stays.
    assert!(store.replace(&[item(5, 12)], 15)?);

    assert_eq!(ids(&store.items()?), [5, 9]);
    assert_eq!(store.cursor()?, 15);
    assert!(store.caught_up(began));
    Ok(())
}

/// A LIVE ITEM MISSING `added_by` OR `attachment` IS DROPPED, NOT DRAWN — from a frame, on a page
/// and in a full read alike. The cursor still follows the page: the item was read, it is simply not
/// something this client can draw, send or offer a removal for.
#[test]
fn a_live_item_missing_who_added_it_or_its_picture_is_dropped() -> StoreResult<()> {
    let database = open()?;
    let store = PackStore::new(&database);
    let nobody = PackItemDto {
        attachment: Some(picture(75, 4096)),
        pack_seq: 12,
        ..PackItemDto::new(5)
    };
    let no_picture = PackItemDto {
        added_by: 7,
        pack_seq: 13,
        ..PackItemDto::new(6)
    };

    assert!(!store.apply(&nobody, LiveFrame)?);
    assert!(!store.apply(&no_picture, LiveFrame)?);
    assert_eq!(
        store.apply_page(&[nobody.clone(), no_picture.clone()], CatchUpPage)?,
        0
    );
    assert_eq!(store.cursor()?, 13);
    assert!(store.replace(&[nobody, no_picture, item(7, 14)], 14)?);

    assert_eq!(ids(&store.items()?), [7]);
    // As the wire writes them, too: the fields absent, not zero.
    let decoded: PackItemDto = wire::decode(
        r#"{"id": 8, "attachment": {"id": 78, "kind": "photo", "mime": "image/png"}, "pack_seq": 15}"#,
    )
    .expect("an item");
    assert!(!store.apply(&decoded, LiveFrame)?);
    assert_eq!(ids(&store.items()?), [7]);
    Ok(())
}

#[test]
fn an_untouched_pack_reads_as_nothing_held_and_no_cursor() -> StoreResult<()> {
    let database = open()?;
    let store = PackStore::new(&database);

    assert!(store.replace(&[], 0)?);

    assert!(store.items()?.is_empty());
    assert_eq!(store.cursor()?, 0);
    Ok(())
}

// ---- the ceilings -------------------------------------------------------------------------------

/// The two ceilings are how the window knows the server has packs at all, so "the server said
/// nothing" is written down as that rather than left as whatever it said last time.
#[test]
fn the_ceilings_are_kept_and_their_absence_is_kept_too() -> StoreResult<()> {
    let database = open()?;
    let store = PackStore::new(&database);
    assert_eq!(store.limits()?, None);

    let known = PackLimits {
        max_items: 200,
        max_item_bytes: 524_288,
    };
    store.set_limits(Some(known))?;
    assert_eq!(store.limits()?, Some(known));
    // Across a relaunch, and offline: the answer is there before the first read of a launch.
    assert_eq!(PackStore::new(&database).limits()?, Some(known));

    store.set_limits(None)?;
    assert_eq!(store.limits()?, None);
    Ok(())
}

// ---- recents --------------------------------------------------------------------------------------

#[test]
fn recents_are_newest_first_and_only_what_the_pack_still_holds() -> StoreResult<()> {
    let database = open()?;
    let store = PackStore::new(&database);
    let at = datetime!(2026-09-13 10:00:00 UTC);
    store.replace(&[item(5, 12), item(6, 13), item(7, 14)], 14)?;

    store.used(5, at)?;
    store.used(7, at + Duration::minutes(1))?;
    store.used(5, at + Duration::minutes(2))?;
    // An id the pack does not hold is not a sticker that can be sent.
    store.used(99, at + Duration::minutes(3))?;

    assert_eq!(store.recents(16)?, [5, 7]);
    assert_eq!(store.recents(1)?, [5]);

    // A removed sticker cannot be "recently used" either.
    store.apply(&tombstone(5, 15), CatchUpPage)?;
    assert_eq!(store.recents(16)?, [7]);
    Ok(())
}

/// A sign-out wipes every table, and the pack — recents and ceilings included — goes with it.
#[test]
fn wiping_the_cache_takes_the_pack_with_it() -> StoreResult<()> {
    let database = open()?;
    let store = PackStore::new(&database);
    store.replace(&[item(5, 12)], 12)?;
    store.apply(&tombstone(5, 13), CatchUpPage)?;
    store.replace(&[item(6, 14)], 14)?;
    store.used(6, OffsetDateTime::UNIX_EPOCH)?;
    store.set_limits(Some(PackLimits {
        max_items: 200,
        max_item_bytes: 524_288,
    }))?;

    database.wipe_all()?;

    assert!(store.items()?.is_empty());
    assert!(store.recents(16)?.is_empty());
    assert_eq!(store.cursor()?, 0);
    assert_eq!(store.limits()?, None);
    // And the gone set with it: the next account's pack is another family's, with its own ids.
    assert!(store.apply(&item(5, 12), CatchUpPage)?);
    Ok(())
}
