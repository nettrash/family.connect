//! `RoundVideoTests.TheViewerCallsItAVideoMessage`: the viewer calls a circle a video message only
//! when it was opened onto one — the attachment `MessageDto::round_video` found. A flagged photo, a
//! flagged video among several, or one opened from a message's tiles is the ordinary item it
//! otherwise is.

mod common;

use fc_core::protocol::AttachmentDto;
use fc_logic::media_album::MediaAlbum;

/// `new AttachmentDto(91, "video", "video/mp4", 1649700, 480, 480, 23400, HasPreview: true,
/// Round: true)`.
fn circle() -> AttachmentDto {
    AttachmentDto {
        mime: Some("video/mp4".to_string()),
        size: Some(1_649_700),
        width: Some(480),
        height: Some(480),
        duration_ms: Some(23_400),
        has_preview: true,
        round: true,
        ..AttachmentDto::new(91, "video")
    }
}

#[test]
fn the_viewer_calls_it_a_video_message() {
    let say = common::english();
    let round = MediaAlbum::new(vec![circle()], 0, true).unwrap();
    assert!(round.is_round());
    assert_eq!(round.title(&say), "Video message");
    let video = fc_text::media::display_name("video", None).into_owned();
    let unflagged = AttachmentDto {
        round: false,
        ..circle()
    };
    assert_eq!(
        MediaAlbum::new(vec![unflagged], 0, true)
            .unwrap()
            .title(&say),
        video
    );
    // Flagged, but not opened as a video message: its tile, in an album.
    assert!(
        !MediaAlbum::new(vec![circle()], 0, false)
            .unwrap()
            .is_round()
    );
    assert_eq!(
        MediaAlbum::new(vec![circle()], 0, false)
            .unwrap()
            .title(&say),
        video
    );
    // Two flagged videos: the S5.1 test fails, so each is a video.
    let second = AttachmentDto { id: 92, ..circle() };
    let two = MediaAlbum::new(vec![circle(), second], 1, true).unwrap();
    assert!(!two.is_round());
    assert_eq!(two.title(&say), video);
    // The flag on a photo.
    let flagged_photo = AttachmentDto {
        name: Some("beach.jpg".to_string()),
        round: true,
        ..AttachmentDto::new(93, "photo")
    };
    let photo = MediaAlbum::new(vec![flagged_photo], 0, true).unwrap();
    assert!(!photo.is_round());
    assert_eq!(photo.title(&say), "beach.jpg");
}
