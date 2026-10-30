//! The send queue: the one table holding something the server has never seen (docs/protocol.md,
//! "Sending on an unreliable network") — a port of
//! `win/tests/FamilyConnect.Core.Tests/Store/OutboxStoreTests.cs`.

use std::rc::Rc;

use fc_core::protocol::send_rules::{MAX_ATTEMPTS, Outcome};
use fc_core::protocol::{ApiError, MentionDto, ReconnectBackoff, error_codes};
use fc_core::store::{Database, OutboxRow, OutboxStore, StoreResult};
use time::macros::datetime;
use time::{Duration, OffsetDateTime};

const NOW: OffsetDateTime = datetime!(2026-09-12 10:00:00 UTC);

fn open() -> StoreResult<Rc<Database>> {
    Database::open_in_memory()
}

/// The ceiling, not a sample, so the delays are exact.
fn ceilings() -> ReconnectBackoff {
    ReconnectBackoff::with_random(|ceiling| ceiling)
}

fn queued(client_msg_id: &str, chat_id: i64, body: &str, at: OffsetDateTime) -> OutboxRow {
    OutboxRow {
        queued_at: Some(at),
        ..OutboxRow::new(client_msg_id, chat_id, body)
    }
}

fn single(store: &OutboxStore) -> StoreResult<OutboxRow> {
    let mut rows = store.all()?;
    assert_eq!(rows.len(), 1, "one queued row");
    Ok(rows.remove(0))
}

fn ids(rows: &[OutboxRow]) -> Vec<&str> {
    rows.iter().map(|row| row.client_msg_id.as_str()).collect()
}

#[test]
fn a_send_is_written_down_before_any_of_it_moves() -> StoreResult<()> {
    let database = open()?;
    let store = OutboxStore::new(&database);
    store.queue(&OutboxRow {
        reply_to_message_id: Some(1337),
        mentions: Some(vec![MentionDto::new(9, "Anna")]),
        ..queued(
            "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01",
            42,
            "Dinner at 7?",
            NOW,
        )
    })?;
    let row = single(&store)?;
    assert_eq!(row.chat_id, 42);
    assert_eq!(row.body, "Dinner at 7?");
    assert_eq!(row.reply_to_message_id, Some(1337));
    let mentions = row.mentions.as_deref().expect("mentions");
    assert_eq!(mentions.len(), 1);
    assert_eq!(mentions[0].name, "Anna");
    assert_eq!(row.queued_at, Some(NOW));
    assert_eq!(row.attempts, 0);
    // Due now: nothing has failed yet, and a queue that waited for a first delay would be a message
    // that sits still under a full signal bar.
    assert_eq!(row.next_attempt_at, None);
    assert!(!row.failed());
    assert_eq!(store.due(NOW)?.len(), 1);
    Ok(())
}

#[test]
fn a_delivered_row_goes_and_the_second_answer_is_harmless() -> StoreResult<()> {
    let database = open()?;
    let store = OutboxStore::new(&database);
    store.queue(&queued("k", 42, "hi", NOW))?;
    assert!(store.delivered("k")?);
    assert!(store.all()?.is_empty());
    // An ack that crossed a REST answer for the same id is ordinary, not an error.
    assert!(!store.delivered("k")?);
    Ok(())
}

#[test]
fn a_transient_failure_keeps_the_row_and_sets_when_to_try_again() -> StoreResult<()> {
    let database = open()?;
    let store = OutboxStore::new(&database);
    store.queue(&queued("k", 42, "hi", NOW))?;
    let what = store.failed(
        "k",
        &ApiError::transport("connection reset"),
        NOW,
        &mut ceilings(),
        true,
    )?;
    assert_eq!(what, Outcome::Retry);
    let row = single(&store)?;
    assert_eq!(row.attempts, 1);
    assert_eq!(row.next_attempt_at, Some(NOW + Duration::seconds(1)));
    assert!(!row.failed());
    // Not due until then — and due the moment it is.
    assert!(store.due(NOW)?.is_empty());
    assert_eq!(store.due(NOW + Duration::seconds(1))?.len(), 1);
    Ok(())
}

#[test]
fn a_terminal_refusal_is_shown_failed_and_nothing_else_is_coming() -> StoreResult<()> {
    let database = open()?;
    let store = OutboxStore::new(&database);
    store.queue(&queued("k", 42, "hi", NOW))?;
    let what = store.failed(
        "k",
        &ApiError::new(error_codes::MESSAGE_TOO_LONG, "too long", 400),
        NOW,
        &mut ceilings(),
        true,
    )?;
    assert_eq!(what, Outcome::Failed);
    let row = single(&store)?;
    assert!(row.failed());
    assert_eq!(
        row.failed_code.as_deref(),
        Some(error_codes::MESSAGE_TOO_LONG)
    );
    // A failed row is not retried behind the user's back.
    assert!(store.due(NOW + Duration::hours(1))?.is_empty());
    Ok(())
}

#[test]
fn the_budget_is_spent_after_six_tries_and_then_it_says_so() -> StoreResult<()> {
    let database = open()?;
    let store = OutboxStore::new(&database);
    store.queue(&queued("k", 42, "hi", NOW))?;
    let offline = ApiError::transport("offline");
    for _ in 1..MAX_ATTEMPTS {
        assert_eq!(
            store.failed("k", &offline, NOW, &mut ceilings(), true)?,
            Outcome::Retry
        );
    }
    assert_eq!(
        store.failed("k", &offline, NOW, &mut ceilings(), true)?,
        Outcome::Failed
    );
    let row = single(&store)?;
    assert_eq!(row.attempts, MAX_ATTEMPTS);
    assert!(row.failed());
    Ok(())
}

#[test]
fn pressing_retry_starts_the_budget_again_and_does_not_wait() -> StoreResult<()> {
    let database = open()?;
    let store = OutboxStore::new(&database);
    store.queue(&queued("k", 42, "hi", NOW))?;
    store.failed(
        "k",
        &ApiError::new(error_codes::VALIDATION, "no", 400),
        NOW,
        &mut ceilings(),
        true,
    )?;
    assert!(single(&store)?.failed());

    store.retry("k")?;
    let row = single(&store)?;
    assert!(!row.failed());
    assert_eq!(row.attempts, 0);
    assert_eq!(row.next_attempt_at, None);
    assert_eq!(store.due(NOW)?.len(), 1);
    Ok(())
}

/// A row with uploads still owed must never be posted: a message claiming no attachments is a text
/// message, and the server would take it happily — a delivered bubble with the pictures gone.
#[test]
fn a_row_that_owes_uploads_says_so_and_keeps_the_ids_that_landed() -> StoreResult<()> {
    let database = open()?;
    let store = OutboxStore::new(&database);
    store.queue(&OutboxRow {
        pending_files: Some(vec!["/tmp/a.jpg".to_string(), "/tmp/b.jpg".to_string()]),
        ..queued("k", 42, "", NOW)
    })?;
    assert!(single(&store)?.owes_uploads());

    store.uploaded("k", 34, "/tmp/a.jpg")?;
    let half = single(&store)?;
    assert_eq!(half.attachment_ids, Some(vec![34]));
    assert_eq!(half.pending_files, Some(vec!["/tmp/b.jpg".to_string()]));
    assert!(half.owes_uploads());

    store.uploaded("k", 35, "/tmp/b.jpg")?;
    let whole = single(&store)?;
    // The ids that DID land are kept and reused, so a retry pushes only the remainder.
    assert_eq!(whole.attachment_ids, Some(vec![34, 35]));
    assert!(!whole.owes_uploads());
    assert_eq!(whole.pending_files, Some(vec![]));
    Ok(())
}

#[test]
fn a_failure_for_a_row_that_is_already_gone_is_not_a_failure_to_show() -> StoreResult<()> {
    let database = open()?;
    let store = OutboxStore::new(&database);
    // Delivered by the socket while a REST attempt for the same id was in flight.
    let what = store.failed(
        "gone",
        &ApiError::transport("late"),
        NOW,
        &mut ceilings(),
        true,
    )?;
    assert_eq!(what, Outcome::Retry);
    assert!(store.all()?.is_empty());
    Ok(())
}

#[test]
fn one_chats_queue_is_its_own() -> StoreResult<()> {
    let database = open()?;
    let store = OutboxStore::new(&database);
    store.queue(&queued("a", 42, "one", NOW))?;
    store.queue(&queued("b", 43, "two", NOW + Duration::seconds(1)))?;
    store.queue(&queued("c", 42, "three", NOW + Duration::seconds(2)))?;
    assert_eq!(ids(&store.for_chat(42)?), ["a", "c"]);
    assert_eq!(ids(&store.for_chat(43)?), ["b"]);
    // Oldest first, which is the order they were written in.
    assert_eq!(ids(&store.all()?), ["a", "b", "c"]);
    Ok(())
}

#[test]
fn an_expired_upload_is_queued_again_rather_than_given_up_on() -> StoreResult<()> {
    let database = open()?;
    let store = OutboxStore::new(&database);
    store.queue(&OutboxRow {
        attachment_ids: Some(vec![34]),
        ..queued("k", 42, "", NOW)
    })?;
    let expired = ApiError::new(error_codes::ATTACHMENT_EXPIRED, "swept", 404);
    assert_eq!(
        store.failed("k", &expired, NOW, &mut ceilings(), true)?,
        Outcome::ReuploadAndRetry
    );
    let row = single(&store)?;
    assert!(!row.failed());
    assert_eq!(store.due(NOW)?.len(), 1);
    // Without the bytes there is no way back, and it says so instead of retrying for ever.
    assert_eq!(
        store.failed("k", &expired, NOW, &mut ceilings(), false)?,
        Outcome::Failed
    );
    assert!(single(&store)?.failed());
    Ok(())
}

/// A STICKER TAPPED OFFLINE IS STILL A STICKER TOMORROW. The flag is written down with the row, so a
/// send that waited out a night — and an app closed and opened in between — goes as what it was,
/// not as a photo in a bubble (docs/protocol.md, "Sending one").
#[test]
fn a_queued_sticker_stays_a_sticker_across_a_relaunch() -> StoreResult<()> {
    let database = open()?;
    let store = || OutboxStore::new(&database);
    store().queue(&OutboxRow {
        pending_files: Some(vec!["0123456789abcdef0123456789abcdef".to_string()]),
        sticker: true,
        ..queued("8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01", 42, "", NOW)
    })?;
    store().queue(&queued(
        "plain",
        42,
        "Dinner at 7?",
        NOW + Duration::seconds(1),
    ))?;

    let rows = store().all()?;
    assert!(rows[0].sticker);
    assert!(rows[0].owes_uploads());
    assert_eq!(rows[0].body, "");
    // And every other row is what it always was.
    assert!(!rows[1].sticker);

    // The flag survives what the row goes through on its way out.
    store().uploaded(
        &rows[0].client_msg_id,
        90,
        "0123456789abcdef0123456789abcdef",
    )?;
    store().failed(
        &rows[0].client_msg_id,
        &ApiError::transport("down"),
        NOW,
        &mut ceilings(),
        true,
    )?;
    store().retry(&rows[0].client_msg_id)?;
    let landed = store().find(&rows[0].client_msg_id)?.expect("the sticker");
    assert!(landed.sticker);
    assert_eq!(landed.attachment_ids, Some(vec![90]));
    assert!(!landed.owes_uploads());
    Ok(())
}

/// A CIRCLE RECORDED OFFLINE IS STILL A CIRCLE TOMORROW — the sticker's rule, one column over
/// (docs/protocol.md, "Video messages"): the flag is written down with the row and survives what
/// the row goes through on its way out.
#[test]
fn a_queued_video_message_stays_round_across_a_relaunch() -> StoreResult<()> {
    let database = open()?;
    let store = || OutboxStore::new(&database);
    store().queue(&OutboxRow {
        pending_files: Some(vec!["0123456789abcdef0123456789abcdef".to_string()]),
        round: true,
        ..queued("8f14e45f-ceea-4e17-a91c-0d9f8e7b2a02", 42, "", NOW)
    })?;
    store().queue(&queued(
        "plain",
        42,
        "Dinner at 7?",
        NOW + Duration::seconds(1),
    ))?;
    store().queue(&OutboxRow {
        sticker: true,
        ..queued("sticker", 42, "", NOW + Duration::seconds(2))
    })?;

    let rows = store().all()?;
    assert!(rows[0].round);
    assert!(!rows[0].sticker);
    assert!(!rows[1].round);
    // The two flags are two columns: a sticker is not a circle.
    assert!(rows[2].sticker);
    assert!(!rows[2].round);

    store().uploaded(
        &rows[0].client_msg_id,
        91,
        "0123456789abcdef0123456789abcdef",
    )?;
    store().failed(
        &rows[0].client_msg_id,
        &ApiError::transport("down"),
        NOW,
        &mut ceilings(),
        true,
    )?;
    store().retry(&rows[0].client_msg_id)?;
    let landed = store().find(&rows[0].client_msg_id)?.expect("the circle");
    assert!(landed.round);
    assert_eq!(landed.attachment_ids, Some(vec![91]));
    Ok(())
}

/// AN ANIMATION QUEUED OFFLINE IS STILL ONE TOMORROW (docs/protocol.md, "Animations") — the circle's
/// rule, one column over: the flag is written down with the row and survives what the row goes
/// through on its way out.
#[test]
fn a_queued_animation_stays_one_across_a_relaunch() -> StoreResult<()> {
    let database = open()?;
    let store = || OutboxStore::new(&database);
    store().queue(&OutboxRow {
        pending_files: Some(vec!["0123456789abcdef0123456789abcdef".to_string()]),
        animation: true,
        ..queued("8f14e45f-ceea-4e17-a91c-0d9f8e7b2a03", 42, "ha", NOW)
    })?;
    store().queue(&OutboxRow {
        round: true,
        ..queued("round", 42, "", NOW + Duration::seconds(1))
    })?;

    let rows = store().all()?;
    assert!(rows[0].animation);
    assert!(!rows[0].round);
    assert!(!rows[0].sticker);
    assert!(!rows[1].animation);
    assert!(rows[1].round);

    store().uploaded(
        &rows[0].client_msg_id,
        93,
        "0123456789abcdef0123456789abcdef",
    )?;
    store().failed(
        &rows[0].client_msg_id,
        &ApiError::transport("down"),
        NOW,
        &mut ceilings(),
        true,
    )?;
    store().retry(&rows[0].client_msg_id)?;
    let landed = store()
        .find(&rows[0].client_msg_id)?
        .expect("the animation");
    assert!(landed.animation);
    assert_eq!(landed.attachment_ids, Some(vec![93]));
    Ok(())
}
