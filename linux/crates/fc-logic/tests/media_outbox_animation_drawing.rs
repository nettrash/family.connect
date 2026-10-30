//! `AnimationDrawingTests.ASendIsAnAnimationOnlyWhenTheAnimationIsAlone`: the send says
//! `animation: true` only for the server's own shape — ONE attachment, and it an animation this
//! device made; beside anything else it goes as the ordinary video it also is (`AnimationSend`) —
//! and its chip in the composer says what it will be (`ComposerStaging.Label`: "GIF", and "Video"
//! without the flag).

mod common;

use fc_logic::composer_staging;
use fc_logic::media_outbox::{StagedMedia, animation_send};

#[test]
fn a_send_is_an_animation_only_when_the_animation_is_alone() {
    let made = StagedMedia {
        width: Some(480),
        height: Some(270),
        duration_ms: Some(2100),
        animation: true,
        ..StagedMedia::new("video", "video/mp4", vec![0, 0, 0, 0])
    };
    let photo = StagedMedia::new("photo", "image/jpeg", vec![0xFF, 0xD8, 0xFF]);
    assert!(animation_send::is(std::slice::from_ref(&made)));
    assert!(!animation_send::is(&[made.clone(), photo.clone()]));
    assert!(!animation_send::is(&[StagedMedia {
        animation: false,
        ..made.clone()
    }]));
    assert!(!animation_send::is(&[StagedMedia {
        animation: true,
        ..photo
    }]));
    assert!(!animation_send::is(&[]));
    // Its chip in the composer says what it will be.
    let say = common::english();
    assert_eq!("GIF", composer_staging::label(&made, &say, "."));
    assert_eq!(
        "Video",
        composer_staging::label(
            &StagedMedia {
                animation: false,
                ..made
            },
            &say,
            "."
        )
    );
}
