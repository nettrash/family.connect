//! `MediaAlbumTests`: the full-size viewer — an album that pages and stops at its ends, and a
//! photo's zoom from 1× to 6×.

mod common;

use fc_core::protocol::AttachmentDto;
use fc_logic::media_album::{EmptyAlbum, MediaAlbum, ViewerKey};

fn named(id: i64, kind: &str, name: &str) -> AttachmentDto {
    AttachmentDto {
        name: Some(name.to_string()),
        ..AttachmentDto::new(id, kind)
    }
}

fn three() -> Vec<AttachmentDto> {
    vec![
        named(1, "photo", "beach.jpg"),
        AttachmentDto::new(2, "video"),
        AttachmentDto::new(3, "photo"),
    ]
}

/// `Assert.Equal(expected, actual, 9)`: equal to nine decimal places.
fn close(expected: f64, actual: f64) {
    assert!(
        (expected - actual).abs() < 5e-10,
        "{expected} is not {actual}"
    );
}

#[test]
fn an_album_is_a_messages_photos_and_videos_in_order() {
    let carried = vec![
        AttachmentDto::new(1, "photo"),
        named(2, "file", "a.pdf"),
        AttachmentDto::new(3, "video"),
        AttachmentDto::new(4, "audio"),
        AttachmentDto::new(5, "location"),
        AttachmentDto::new(6, "photo"),
    ];
    assert_eq!(
        MediaAlbum::of(&carried)
            .iter()
            .map(|attachment| attachment.id)
            .collect::<Vec<_>>(),
        vec![1, 3, 6]
    );
    assert_eq!(MediaAlbum::new(vec![], 0, false), Err(EmptyAlbum));
    assert_eq!(MediaAlbum::new(three(), 7, false).unwrap().index(), 2);
    assert_eq!(MediaAlbum::new(three(), -3, false).unwrap().index(), 0);
}

/// ← and → page and stop at the ends; Esc closes; a key with nowhere to go does nothing.
#[test]
fn paging_stops_at_the_ends() {
    let mut album = MediaAlbum::new(three(), 0, false).unwrap();
    assert!(!album.has_previous());
    assert_eq!(album.key("Left"), ViewerKey::None);
    assert!(!album.step(-1));
    assert_eq!(album.key("Right"), ViewerKey::Next);
    assert!(album.step(1));
    assert!(album.is_video());
    assert!(album.step(1));
    assert!(!album.has_next());
    assert_eq!(album.key("Right"), ViewerKey::None);
    assert!(!album.step(1));
    assert_eq!(album.index(), 2);
    assert_eq!(album.key("Left"), ViewerKey::Previous);
    assert_eq!(album.key("Escape"), ViewerKey::Close);
    assert_eq!(album.key("Up"), ViewerKey::None);
}

/// The web's own vectors: never smaller than the window, 1.25× a step, and no further than 6×.
#[test]
fn zoom_goes_from_one_to_six_and_no_further() {
    assert_eq!(MediaAlbum::zoom_step(1.0, -1.0), 1.0);
    close(1.25, MediaAlbum::zoom_step(1.0, 1.0));
    assert_eq!(MediaAlbum::zoom_step(5.5, 1.0), MediaAlbum::MAX_ZOOM);
    assert_eq!(
        MediaAlbum::zoom_step(MediaAlbum::MAX_ZOOM, 1.0),
        MediaAlbum::MAX_ZOOM
    );
    close(1.6, MediaAlbum::zoom_step(2.0, -1.0));
}

/// A double click toggles 1× and 2×; a pinch lands where it lands, within 1–6; a page turn starts
/// at 1× again.
#[test]
fn zoom_toggles_clamps_and_resets_on_a_page_turn() {
    let mut album = MediaAlbum::new(three(), 0, false).unwrap();
    assert!(!album.can_zoom_out());
    album.toggle_zoom();
    assert_eq!(album.zoom(), 2.0);
    album.toggle_zoom();
    assert_eq!(album.zoom(), 1.0);
    album.toggle_zoom();
    album.zoom_in();
    close(2.5, album.zoom());
    album.toggle_zoom();
    assert_eq!(album.zoom(), 1.0);
    album.zoomed_to(9.0);
    assert_eq!(album.zoom(), MediaAlbum::MAX_ZOOM);
    assert!(!album.can_zoom_in());
    album.zoomed_to(0.4);
    assert_eq!(album.zoom(), 1.0);
    album.zoom_out();
    assert_eq!(album.zoom(), 1.0);

    album.zoomed_to(3.0);
    album.step(1);
    assert_eq!(album.zoom(), 1.0);
    // A video has no zoom to offer — not even one a stray pinch left behind.
    assert!(!album.can_zoom_in());
    assert!(!album.can_zoom_out());
    album.zoomed_to(3.0);
    assert!(!album.can_zoom_in());
    assert!(!album.can_zoom_out());
}

#[test]
fn the_words_over_the_picture() {
    let say = common::english();
    let mut album = MediaAlbum::new(three(), 0, false).unwrap();
    assert_eq!(album.title(&say), "beach.jpg");
    assert_eq!(album.position(&say).as_deref(), Some("1 of 3"));
    album.step(1);
    assert_eq!(album.position(&say).as_deref(), Some("2 of 3"));
    assert_eq!(
        album.title(&say),
        fc_text::media::display_name("video", None)
    );
    assert_eq!(
        MediaAlbum::new(vec![AttachmentDto::new(9, "photo")], 0, false)
            .unwrap()
            .position(&say),
        None
    );

    let mut zoomed = MediaAlbum::new(three(), 0, false).unwrap();
    assert_eq!(zoomed.zoom_text(), "100%");
    zoomed.zoom_in();
    assert_eq!(zoomed.zoom_text(), "125%");
}

/// The port's own: the percentage rounds half to even, as .NET's `Math.Round` does.
#[test]
fn the_zoom_text_rounds_half_to_even() {
    let mut album = MediaAlbum::new(three(), 0, false).unwrap();
    album.zoomed_to(1.125);
    assert_eq!(album.zoom_text(), "112%");
    album.zoomed_to(1.135);
    assert_eq!(album.zoom_text(), "114%");
    album.zoomed_to(6.0);
    assert_eq!(album.zoom_text(), "600%");
}
