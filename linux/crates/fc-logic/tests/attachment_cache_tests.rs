//! `AttachmentCacheTests`: one attachment's bytes, and the preview rule three clients have each had
//! to learn.

mod common;

use std::rc::Rc;

use fc_core::protocol::AttachmentDto;
use fc_core::testing::{FakeClock, drive};
use fc_logic::attachment_cache::AttachmentCache;
use time::macros::datetime;

use common::media::Blobs;
use common::server::{AnyPath, api_over};

struct Rig {
    cache: AttachmentCache,
    handler: AnyPath,
    blobs: Blobs,
    clock: FakeClock,
}

/// `Build(handler)`: the cache over a handler that answers any attachment path.
fn build(handler: AnyPath) -> Rig {
    let clock = FakeClock::at(datetime!(2026-09-12 12:00 UTC));
    let blobs = Blobs::new();
    Rig {
        cache: AttachmentCache::new(
            api_over(Rc::new(handler.clone()), &clock),
            Rc::new(blobs.clone()),
        ),
        handler,
        blobs,
        clock,
    }
}

fn pictures() -> AnyPath {
    AnyPath::new(200, "JPEG")
}

fn photo(has_preview: bool) -> AttachmentDto {
    AttachmentDto {
        mime: Some("image/jpeg".to_string()),
        width: Some(1600),
        height: Some(1200),
        has_preview,
        ..AttachmentDto::new(34, "photo")
    }
}

#[test]
fn bytes_are_downloaded_once_and_kept() {
    let rig = build(pictures());
    let photo = photo(true);

    let bytes = drive(&rig.clock, rig.cache.bytes(&photo, false)).expect("bytes");

    assert_eq!(b"JPEG".to_vec(), bytes);
    assert_eq!(vec!["/api/v1/attachments/34"], rig.handler.asked());

    // The second look asks nobody.
    assert!(rig.cache.holds(&photo, false));
    assert!(drive(&rig.clock, rig.cache.bytes(&photo, false)).is_ok());
    assert_eq!(1, rig.handler.asked().len());
    assert_eq!(1, rig.blobs.len());
}

#[test]
fn the_preview_and_the_original_are_different_things() {
    let rig = build(pictures());
    let photo = photo(true);

    drive(&rig.clock, rig.cache.bytes(&photo, true)).expect("the preview");
    drive(&rig.clock, rig.cache.bytes(&photo, false)).expect("the original");

    assert_eq!(
        vec!["/api/v1/attachments/34/preview", "/api/v1/attachments/34"],
        rig.handler.asked()
    );
    assert_eq!(2, rig.blobs.len());
}

/// A PREVIEW IS ONLY ASKED FOR WHEN THE ATTACHMENT SAYS IT HAS ONE. The server generates none for a
/// picture the assistant drew; asking anyway answers 404, and a client that reads that as "no
/// picture" draws an empty frame for ever — which is exactly how the Android board's backdrops
/// failed.
#[test]
fn a_picture_with_no_preview_is_fetched_whole_rather_than_asked_for_twice() {
    let rig = build(pictures());
    let drawn = photo(false);

    let bytes = drive(&rig.clock, rig.cache.bytes(&drawn, true));

    assert!(bytes.is_ok());
    // The ORIGINAL, and nothing else: no 404 on a preview that was never generated.
    assert_eq!(vec!["/api/v1/attachments/34"], rig.handler.asked());
    assert!(rig.cache.holds(&drawn, true));
}

/// A FAILED DOWNLOAD IS NOT CACHED: a transient failure must not become a permanently blank
/// picture.
#[test]
fn a_failed_download_is_not_remembered_as_an_answer() {
    let rig = build(AnyPath::new(502, "<html>502</html>"));
    let photo = photo(true);

    let error = drive(&rig.clock, rig.cache.bytes(&photo, false)).expect_err("no bytes");

    assert!(error.transient());
    assert!(rig.blobs.is_empty());
    assert!(!rig.cache.holds(&photo, false));

    // And it asks again rather than drawing nothing for ever.
    let _ = drive(&rig.clock, rig.cache.bytes(&photo, false));
    assert_eq!(2, rig.handler.asked().len());
}

#[test]
fn the_keys_say_which_copy_they_are() {
    assert_eq!("34", AttachmentCache::key_for(34, false));
    assert_eq!("34.preview", AttachmentCache::key_for(34, true));
}
