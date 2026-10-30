//! `AvatarTests`: profile pictures — what may be sent, and the version that decides what is drawn.
//! Plus one that is not the C#'s: the downscale's rounding, pinned against .NET 10.

mod common;

use std::rc::Rc;

use fc_core::testing::{FakeClock, drive};
use fc_logic::avatars::{AvatarCache, avatar_rules};
use time::macros::datetime;

use common::media::Blobs;
use common::server::{AnyPath, api_over};

struct Rig {
    cache: AvatarCache,
    handler: AnyPath,
    blobs: Blobs,
    clock: FakeClock,
}

fn build(handler: AnyPath) -> Rig {
    let clock = FakeClock::at(datetime!(2026-09-12 12:00 UTC));
    let blobs = Blobs::new();
    Rig {
        cache: AvatarCache::new(
            api_over(Rc::new(handler.clone()), &clock),
            Rc::new(blobs.clone()),
        ),
        handler,
        blobs,
        clock,
    }
}

fn faces() -> AnyPath {
    AnyPath::new(200, "JPEG")
}

#[test]
fn a_picture_is_shrunk_to_five_hundred_and_twelve_and_never_blown_up() {
    assert!(avatar_rules::needs_downscale(4032, 3024));
    assert_eq!((512, 384), avatar_rules::fit(4032, 3024));
    assert_eq!((384, 512), avatar_rules::fit(3024, 4032));
    assert_eq!((512, 512), avatar_rules::fit(1024, 1024));

    // Already small enough: sent as it is, rather than blown up to look worse.
    assert!(!avatar_rules::needs_downscale(200, 120));
    assert_eq!((200, 120), avatar_rules::fit(200, 120));

    // A picture the platform could not measure is not a picture to send.
    assert_eq!((0, 0), avatar_rules::fit(0, 0));
}

#[test]
fn only_the_two_types_the_server_takes_are_offered() {
    assert!(avatar_rules::is_allowed("image/jpeg"));
    assert!(avatar_rules::is_allowed("IMAGE/PNG"));
    assert!(!avatar_rules::is_allowed("image/heic"));
    assert!(!avatar_rules::is_allowed("image/gif"));
    assert_eq!("image/jpeg", avatar_rules::SEND_AS);
}

/// THE VERSION IS PART OF THE KEY. No frame carries a picture — only the number — so a cache keyed
/// on the user alone shows a face the family replaced weeks ago.
#[test]
fn a_picture_is_kept_per_version_so_a_change_is_drawn() {
    let rig = build(faces());

    let bytes = drive(&rig.clock, rig.cache.bytes(11, 3)).expect("bytes");
    assert_eq!(Some(b"JPEG".to_vec()), bytes);
    assert_eq!(1, rig.handler.asked().len());

    // The same version again asks nobody.
    drive(&rig.clock, rig.cache.bytes(11, 3)).expect("bytes");
    assert_eq!(1, rig.handler.asked().len());

    // A new version is a new picture, and it is fetched.
    drive(&rig.clock, rig.cache.bytes(11, 4)).expect("bytes");
    assert_eq!(2, rig.handler.asked().len());
    assert_eq!(2, rig.blobs.len());
    assert_eq!("avatar-11-4", AvatarCache::key_for(11, 4));
}

#[test]
fn version_zero_is_no_picture_and_asks_nobody() {
    let rig = build(faces());

    let bytes = drive(&rig.clock, rig.cache.bytes(11, 0));

    assert_eq!(Ok(None), bytes);
    assert!(rig.handler.asked().is_empty());
}

/// "NO PICTURE" IS AN ANSWER AND IT IS CACHED: a 404 is stable for that version, and asking again
/// on every redraw would be a request per row per frame.
#[test]
fn a_missing_picture_is_remembered_rather_than_asked_for_again() {
    let rig = build(AnyPath::new(
        404,
        r#"{"error": {"code": "user_not_found", "message": "no"}}"#,
    ));

    // Not a failure to show: the caller draws initials.
    assert_eq!(Ok(None), drive(&rig.clock, rig.cache.bytes(11, 3)));

    assert_eq!(Ok(None), drive(&rig.clock, rig.cache.bytes(11, 3)));
    assert_eq!(1, rig.handler.asked().len());
}

#[test]
fn a_transient_failure_is_not_remembered_as_no_picture() {
    let rig = build(AnyPath::new(502, "<html>502</html>"));

    let error = drive(&rig.clock, rig.cache.bytes(11, 3)).expect_err("a failure");

    assert!(error.transient());
    assert!(rig.blobs.is_empty());

    let _ = drive(&rig.clock, rig.cache.bytes(11, 3));
    assert_eq!(2, rig.handler.asked().len());
}

/// Not a C# test: `Fit`'s rounding is `Math.Round`'s, half to EVEN — the sizes .NET 10 printed for
/// these pictures (the oracle in the session's scratchpad).
#[test]
fn the_downscale_rounds_as_dotnet_does() {
    for (width, height, fitted) in [
        (1025, 1023, (512, 511)),
        (1536, 1024, (512, 341)),
        (3, 1025, (1, 512)),
        (1024, 3, (512, 2)),
        (2048, 1, (512, 1)),
        (1000, 999, (512, 511)),
        (4032, 3024, (512, 384)),
        (513, 257, (512, 256)),
        (1023, 1, (512, 1)),
        // Ties: 2.5 → 2 and 510.5 → 510, where rounding away from zero would say 3 and 511.
        (1024, 5, (512, 2)),
        (2048, 10, (512, 2)),
        (1024, 1021, (512, 510)),
        (5, 1024, (2, 512)),
    ] {
        assert_eq!(
            fitted,
            avatar_rules::fit(width, height),
            "{width} × {height}"
        );
    }
}
