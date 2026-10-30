//! `PictureMemoryTests`: how big a tile's photo is decoded, and how many decoded pictures a view
//! keeps.

use fc_logic::picture_memory::{RecentPictures, TILE_BOX, tile_width};

fn width(width: Option<i32>, height: Option<i32>, raster_scale: f64) -> Option<i32> {
    tile_width(width, height, raster_scale, TILE_BOX)
}

/// A camera's photo is decoded so that BOTH sides come out at least the largest tile box,
/// whichever side the decoder calls its width — so an EXIF turn the attachment's numbers leave out
/// can never make a tile blurrier than before.
#[test]
fn a_big_photo_is_decoded_near_the_tile_whichever_way_up_it_is() {
    let decoded = width(Some(4000), Some(3000), 1.5);
    assert_eq!(decoded, Some(427));
    // Asked as the width: 427 × 320 one way up, and 427 × 569 the other — at least 320 either way.
    for (picture_width, picture_height) in [(4000, 3000), (3000, 4000)] {
        let decoded_height =
            f64::from(decoded.unwrap()) * f64::from(picture_height) / f64::from(picture_width);
        assert!(f64::from(decoded.unwrap()) >= TILE_BOX);
        assert!(decoded_height >= TILE_BOX - 1.0);
    }
    assert_eq!(width(Some(3000), Some(4000), 1.5), decoded);
}

/// A picture already about the tile's size, or one whose size nobody said, is decoded whole, as
/// it always was.
#[test]
fn a_small_or_unknown_photo_is_decoded_whole() {
    assert_eq!(width(Some(400), Some(300), 1.0), None);
    // A 2:1 picture is asked for at 640 effective pixels wide, 1280 device pixels at 200 %: more
    // than its 600-pixel short side, so there is nothing to save.
    assert_eq!(width(Some(1200), Some(600), 2.0), None);
    assert!(width(Some(2400), Some(1200), 1.0).is_some());
    assert_eq!(width(None, Some(3000), 1.0), None);
    assert_eq!(width(Some(4000), Some(0), 1.0), None);
    // A screen that reports no scale is taken at 100 %.
    assert_eq!(
        width(Some(4000), Some(3000), 1.0),
        width(Some(4000), Some(3000), 0.0)
    );
}

/// A panorama keeps its shape: the long side is asked for in proportion, so the short side still
/// fills the box.
#[test]
fn a_panorama_is_decoded_wide_enough_to_fill_the_box() {
    let decoded = width(Some(12000), Some(2000), 1.0);
    assert_eq!(decoded, Some(1920));
    assert!(f64::from(decoded.unwrap()) * 2000.0 / 12000.0 >= TILE_BOX);
}

#[test]
fn the_pictures_kept_are_the_most_recently_drawn() {
    let mut kept = RecentPictures::new(2);
    kept.insert("a", "A");
    kept.insert("b", "B");
    assert_eq!(kept.try_get("a"), Some("A"));
    kept.insert("c", "C");

    // "b" was drawn longest ago — "a" was just drawn again — so "b" made the room.
    assert!(!kept.contains_key("b"));
    assert!(kept.contains_key("a"));
    assert!(kept.contains_key("c"));
    assert_eq!(kept.count(), 2);
    assert_eq!(kept.try_get("b"), None);
}

/// Asking whether a picture is kept is not drawing it: it does not save it from being the next to
/// go.
#[test]
fn asking_is_not_drawing() {
    let mut kept = RecentPictures::new(2);
    kept.insert("a", "A");
    kept.insert("b", "B");
    assert!(kept.contains_key("a"));
    kept.insert("c", "C");
    assert!(!kept.contains_key("a"));
}

#[test]
fn a_picture_kept_again_replaces_the_old_one_and_clear_forgets_all() {
    let mut kept = RecentPictures::new(3);
    kept.insert("a", "old");
    kept.insert("a", "new");
    assert_eq!(kept.count(), 1);
    assert_eq!(kept.try_get("a"), Some("new"));
    kept.clear();
    assert_eq!(kept.count(), 0);
    assert!(!kept.contains_key("a"));
}

/// Not in the C# suite — the one place `Math.Max` and `f64::max` part: a NaN scale. .NET's `Max`
/// keeps the NaN and the comparison fails, so the picture is decoded whole; the port does the
/// same (measured on .NET 10: `TileWidth(4000, 3000, double.NaN)` is null).
#[test]
fn a_scale_that_is_not_a_number_decodes_whole_as_dotnet_does() {
    assert_eq!(width(Some(4000), Some(3000), f64::NAN), None);
}
