//! `AnimationDrawingTests.ATilePlaysByItselfOnlyWhenWantedAndSmall`: a tile plays by itself only
//! while the system wants motion and the video is within 8 MiB — the one video a tile downloads.
//! Otherwise its poster, still, and a click plays it in the viewer.

use fc_logic::animated_pictures::animation_prep;

#[test]
fn a_tile_plays_by_itself_only_when_wanted_and_small() {
    assert_eq!(animation_prep::TILE_BYTES, 8_388_608);
    assert!(animation_prep::plays_in_tile(true, Some(182_330)));
    assert!(animation_prep::plays_in_tile(true, Some(8_388_608)));
    assert!(!animation_prep::plays_in_tile(true, Some(8_388_609)));
    assert!(!animation_prep::plays_in_tile(false, Some(182_330)));
    assert!(!animation_prep::plays_in_tile(true, None));
    assert!(!animation_prep::plays_in_tile(true, Some(0)));
}
