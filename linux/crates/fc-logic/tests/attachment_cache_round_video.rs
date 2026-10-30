//! `RoundVideoTests.AKeptPosterIsDrawnWithoutAskingTheServer`: the reader's own circle is drawn from
//! the bytes this device already holds — the poster it made is kept under the attachment's preview
//! key, so the delivered circle is not downloaded back to draw itself (S5.6).

mod common;

use std::rc::Rc;

use fc_core::protocol::AttachmentDto;
use fc_core::testing::{FakeClock, drive};
use fc_logic::attachment_cache::AttachmentCache;
use time::macros::datetime;

use common::media::Blobs;
use common::server::{Server, api};

fn circle() -> AttachmentDto {
    AttachmentDto {
        mime: Some("video/mp4".to_string()),
        size: Some(1_649_700),
        width: Some(480),
        height: Some(480),
        duration_ms: Some(23400),
        has_preview: true,
        round: true,
        ..AttachmentDto::new(91, "video")
    }
}

#[test]
fn a_kept_poster_is_drawn_without_asking_the_server() {
    let clock = FakeClock::at(datetime!(2026-10-05 10:00 UTC));
    let server = Server::new();
    let attachments = AttachmentCache::new(api(&server, &clock), Rc::new(Blobs::new()));
    let unannounced = AttachmentDto {
        has_preview: false,
        ..circle()
    };

    attachments.remember_preview(&unannounced, b"jpeg");
    attachments.remember(&unannounced, b"ftypmp");

    let poster = drive(&clock, attachments.bytes(&circle(), true));
    assert_eq!(Ok(b"jpeg".to_vec()), poster);
    let video = drive(&clock, attachments.bytes(&circle(), false));
    assert_eq!(Ok(b"ftypmp".to_vec()), video);
    assert!(server.asked().is_empty());
    assert!(attachments.holds(&circle(), true));
}
