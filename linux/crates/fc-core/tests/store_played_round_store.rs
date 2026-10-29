//! The recordings this device has played — what the dot beside an unplayed circle or voice note is
//! drawn from (docs/audio-video-messages-2026-10-04.md, S5.2). A port of
//! `win/tests/FamilyConnect.Core.Tests/Store/PlayedRoundStoreTests.cs`, both of its classes:
//! `PlayedRoundStoreTests` and `PlayedVoiceStoreTests`.

use std::rc::Rc;

use fc_core::store::{Database, PlayedRoundStore, PlayedStore, PlayedVoiceStore, StoreResult};

fn open() -> StoreResult<Rc<Database>> {
    Database::open_in_memory()
}

const KEPT: i64 = PlayedStore::KEPT as i64;

// ---- PlayedRoundStoreTests: the video messages ------------------------------------------------

#[test]
fn nothing_is_played_until_it_is() -> StoreResult<()> {
    let cache = open()?;
    let store = PlayedRoundStore::new(&cache);
    assert!(!store.played(91)?);
    assert_eq!(store.count()?, 0);
    Ok(())
}

/// Marked once, by attachment — and a second mark is not news, so nothing redraws for it.
#[test]
fn a_play_is_remembered_by_its_attachment_and_only_once() -> StoreResult<()> {
    let cache = open()?;
    let store = PlayedRoundStore::new(&cache);
    assert!(store.mark_played(91)?);
    assert!(!store.mark_played(91)?);
    assert!(store.played(91)?);
    assert!(!store.played(92)?);
    assert_eq!(store.count()?, 1);
    Ok(())
}

/// THE NEWEST 5 000 AND NO MORE: ids only grow, so the lowest go first — even when an old circle is
/// the one played last, it is the oldest that has to make room.
#[test]
fn only_the_newest_five_thousand_are_kept() -> StoreResult<()> {
    let cache = open()?;
    let store = PlayedRoundStore::new(&cache);
    assert_eq!(PlayedStore::KEPT, 5000);
    for id in 1..=KEPT {
        store.mark_played(id + 100)?;
    }
    assert_eq!(store.count()?, PlayedStore::KEPT);
    assert!(store.played(101)?);

    store.mark_played(100_000)?;
    assert_eq!(store.count()?, PlayedStore::KEPT);
    assert!(!store.played(101)?);
    assert!(store.played(102)?);
    assert!(store.played(100_000)?);

    // An OLDER one played now is below everything kept: it does not push a newer one out.
    store.mark_played(5)?;
    assert_eq!(store.count()?, PlayedStore::KEPT);
    assert!(!store.played(5)?);
    assert!(store.played(102)?);
    Ok(())
}

/// The device's own knowledge, and this account's: a sign-out takes it with the rest of the cache.
#[test]
fn a_sign_out_forgets_what_was_played() -> StoreResult<()> {
    let cache = open()?;
    let store = PlayedRoundStore::new(&cache);
    store.mark_played(91)?;
    cache.wipe_all()?;
    assert!(!store.played(91)?);
    assert_eq!(store.count()?, 0);
    Ok(())
}

// ---- PlayedVoiceStoreTests: the voice messages, kept apart from the circles ------------------------

/// A play of one kind is never a play of the other, and neither kind's 5 000 pushes the other's
/// out.
#[test]
fn a_voice_note_is_remembered_apart_from_the_circles() -> StoreResult<()> {
    let cache = open()?;
    let voice = PlayedVoiceStore::new(&cache);
    let rounds = PlayedRoundStore::new(&cache);
    assert!(!voice.played(91)?);
    assert!(voice.mark_played(91)?);
    assert!(!voice.mark_played(91)?);
    assert!(voice.played(91)?);
    assert!(!rounds.played(91)?);
    for id in 1..=KEPT {
        rounds.mark_played(id + 1000)?;
    }
    assert!(voice.played(91)?);
    assert_eq!(voice.count()?, 1);
    Ok(())
}

#[test]
fn a_sign_out_forgets_the_voice_notes_too() -> StoreResult<()> {
    let cache = open()?;
    let voice = PlayedVoiceStore::new(&cache);
    voice.mark_played(91)?;
    cache.wipe_all()?;
    assert!(!voice.played(91)?);
    Ok(())
}
