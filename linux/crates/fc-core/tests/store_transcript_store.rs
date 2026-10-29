//! The text of recordings this member asked for, kept on this device (docs/protocol.md,
//! "Transcripts on request") — a port of
//! `win/tests/FamilyConnect.Core.Tests/Store/TranscriptStoreTests.cs`.

use fc_core::store::{Database, KeptTranscript, StoreResult, TranscriptStore};
use time::OffsetDateTime;

const EPOCH: OffsetDateTime = OffsetDateTime::UNIX_EPOCH;

fn in_language(attachment_id: i64, text: &str, language: &str) -> KeptTranscript {
    KeptTranscript {
        language: Some(language.to_string()),
        ..KeptTranscript::new(attachment_id, text)
    }
}

fn supplied(attachment_id: i64, text: &str) -> KeptTranscript {
    KeptTranscript {
        supplied: true,
        ..KeptTranscript::new(attachment_id, text)
    }
}

#[test]
fn nothing_is_held_until_somebody_asked() -> StoreResult<()> {
    let cache = Database::open_in_memory()?;
    let store = TranscriptStore::new(&cache);
    assert_eq!(store.find(34)?, None);
    Ok(())
}

#[test]
fn an_answer_is_kept_by_its_attachment_with_its_language() -> StoreResult<()> {
    let cache = Database::open_in_memory()?;
    let store = TranscriptStore::new(&cache);
    store.keep(&in_language(34, "Мы будем в шесть", "ru"), EPOCH)?;
    store.keep(&KeptTranscript::new(35, "Back at six"), EPOCH)?;

    assert_eq!(
        store.find(34)?,
        Some(in_language(34, "Мы будем в шесть", "ru"))
    );
    assert_eq!(
        store.find(35)?,
        Some(KeptTranscript::new(35, "Back at six"))
    );
    assert_eq!(store.find(36)?, None);
    Ok(())
}

/// SILENCE IS AN ANSWER: kept as the empty text it is, never as "not asked".
#[test]
fn silence_is_kept_as_an_answer() -> StoreResult<()> {
    let cache = Database::open_in_memory()?;
    let store = TranscriptStore::new(&cache);
    store.keep(&KeptTranscript::new(34, ""), EPOCH)?;
    let kept = store.find(34)?;
    assert!(kept.is_some());
    assert_eq!(kept.expect("silence").text, "");
    Ok(())
}

/// The stored-bytes answer is the one every member gets, and the server returns it instead of
/// reading supplied sound once it exists — so a supplied answer never replaces it here either; the
/// other way round, it does.
#[test]
fn a_stored_answer_is_never_replaced_by_a_supplied_one() -> StoreResult<()> {
    let cache = Database::open_in_memory()?;
    let store = TranscriptStore::new(&cache);
    store.keep(&KeptTranscript::new(34, "from the server's copy"), EPOCH)?;
    store.keep(&supplied(34, "from this device's sound"), EPOCH)?;
    assert_eq!(
        store.find(34)?,
        Some(KeptTranscript::new(34, "from the server's copy"))
    );

    store.keep(&supplied(35, "from this device's sound"), EPOCH)?;
    assert!(store.find(35)?.expect("the supplied answer").supplied);
    store.keep(&in_language(35, "from the server's copy", "en"), EPOCH)?;
    assert_eq!(
        store.find(35)?,
        Some(in_language(35, "from the server's copy", "en"))
    );
    Ok(())
}

/// The text of somebody's voice belongs to the account that asked: a sign-out takes it.
#[test]
fn a_sign_out_takes_the_text_with_it() -> StoreResult<()> {
    let cache = Database::open_in_memory()?;
    let store = TranscriptStore::new(&cache);
    store.keep(&KeptTranscript::new(34, "See you at six"), EPOCH)?;
    cache.wipe_all()?;
    assert_eq!(store.find(34)?, None);
    Ok(())
}
