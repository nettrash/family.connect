//! `AnimationDrawingTests.TheViewerCallsItAGif`: in the viewer an animation is a GIF — one flagged
//! video, alone — and never a circle.

mod common;

use fc_core::protocol::AttachmentDto;
use fc_logic::media_album::MediaAlbum;

/// `new AttachmentDto(93, "video", "video/mp4", 182_330, 480, 270, 2100, HasPreview: true,
/// Animation: true)`.
fn gif() -> AttachmentDto {
    AttachmentDto {
        mime: Some("video/mp4".to_string()),
        size: Some(182_330),
        width: Some(480),
        height: Some(270),
        duration_ms: Some(2100),
        has_preview: true,
        animation: true,
        ..AttachmentDto::new(93, "video")
    }
}

#[test]
fn the_viewer_calls_it_a_gif() {
    let say = common::english();
    let album = MediaAlbum::new(vec![gif()], 0, false).unwrap();
    assert!(album.is_animation());
    assert!(album.is_video());
    assert!(!album.is_round());
    assert_eq!(album.title(&say), "GIF");
    let unflagged = AttachmentDto {
        animation: false,
        ..gif()
    };
    assert!(
        !MediaAlbum::new(vec![unflagged], 0, false)
            .unwrap()
            .is_animation()
    );
    let second = AttachmentDto { id: 94, ..gif() };
    assert!(
        !MediaAlbum::new(vec![gif(), second], 0, false)
            .unwrap()
            .is_animation()
    );
    let photo = AttachmentDto {
        kind: "photo".to_string(),
        ..gif()
    };
    assert!(
        !MediaAlbum::new(vec![photo], 0, false)
            .unwrap()
            .is_animation()
    );
}
