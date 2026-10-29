//! `SeveralFamiliesTests.QueuedUploadsAreForTheChatsFamily`, whose subject is `MediaOutbox`: a
//! queued message's uploads are for the family its chat is in, whatever is shown by the time they
//! go.

mod common;

use std::collections::HashSet;
use std::rc::Rc;

use fc_core::store::{Database, OutboxRow, OutboxStore, StoreResult};
use fc_core::testing::{FakeClock, drive};
use fc_logic::media_outbox::{MediaOutbox, MediaStore, StagedMedia};
use time::macros::datetime;

use common::server::{Server, api};

/// `OneFile`: whatever is asked for is the same small photo.
struct OneFile;

impl MediaStore for OneFile {
    fn read(&self, _handle: &str) -> Option<StagedMedia> {
        Some(StagedMedia {
            width: Some(16),
            height: Some(16),
            ..StagedMedia::new("photo", "image/jpeg", vec![1, 2, 3])
        })
    }

    fn sweep(&self, _keep: &HashSet<String>) {}
}

#[test]
fn queued_uploads_are_for_the_chats_family() -> StoreResult<()> {
    let cache = Database::open_in_memory()?;
    let outbox = Rc::new(OutboxStore::new(&cache));
    outbox.queue(&OutboxRow {
        pending_files: Some(vec!["a.jpg".to_string()]),
        staged_files: Some(vec!["a.jpg".to_string()]),
        ..OutboxRow::new("k", 43, "look")
    })?;
    let server = Server::new();
    server.on(
        "/attachments",
        r#"{"attachment": {"id": 34, "kind": "photo"}}"#,
    );
    let clock = FakeClock::at(datetime!(2026-10-22 10:00 UTC));
    let api = api(&server, &clock);
    // `{ FamilyScope = 3 }`: another family is shown by the time the upload goes.
    api.set_family_scope(Some(3));
    let media = MediaOutbox::new(outbox, api, Rc::new(OneFile))
        .with_family_for(|chat_id| (chat_id == 43).then_some(5));

    assert_eq!(1, drive(&clock, media.push())?);

    let asked = server.asked();
    assert_eq!(1, asked.len());
    assert!(asked[0].ends_with("&family_id=5"), "{}", asked[0]);
    Ok(())
}
