//! `RoundVideoTests`, the cases whose subject is `RoundLook`: how a circle is drawn and named
//! (S5.2, S6), when its dot goes, the viewer's ring and circle, and what pauses it (S4's last
//! column, `PlaybackPauses`). The quote is `quotes_round_video.rs`, the kept poster
//! `attachment_cache_round_video.rs`, Edit and the send `conversation_round_video.rs`, the viewer's
//! title `media_album_round_video.rs`, the chat-list row `chat_list_media_rows.rs`.

mod common;

use fc_core::protocol::AttachmentDto;
use fc_logic::not_sent::RecordingEnd;
use fc_logic::round_look::{self, PlaybackEvent, Playing, playback_pauses};
use fc_logic::sticker_look::StickerLook;

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

/// xUnit's `Assert.Equal(expected, actual, precision)`: both rounded to `places` decimals.
fn assert_near(expected: f64, actual: f64, places: i32, what: &str) {
    let scale = 10f64.powi(places);
    assert_eq!(
        (expected * scale).round_ties_even(),
        (actual * scale).round_ties_even(),
        "{what}: {expected} vs {actual}"
    );
}

// ---- how it is drawn (S5.2, S6) -------------------------------------------------------------

/// 240 on the desktop, which is never a compact width — larger than a sticker, smaller than a
/// video tile.
#[test]
fn a_circle_is_two_hundred_forty_across_larger_than_a_sticker() {
    assert_eq!(240.0, round_look::DIAMETER);
    const { assert!(round_look::DIAMETER > StickerLook::BOX) };
    // The approved design's 48 play disc (the mockup's .round .pd), as Android draws it.
    assert_eq!(48.0, round_look::PLAY_DISC);
    assert_eq!(8.0, round_look::DOT);
    assert_eq!(3.0, round_look::RING);
}

/// The capsule says how long it is, and a screen reader hears the same with what it is (S6).
#[test]
fn a_circle_says_how_long_it_is_and_whether_it_was_played() {
    let say = common::english();
    assert_eq!(Some("0:23".to_string()), round_look::capsule(&circle()));
    assert_eq!(
        Some("1:00".to_string()),
        round_look::capsule(&AttachmentDto {
            duration_ms: Some(60_000),
            ..circle()
        })
    );
    assert_eq!(
        None,
        round_look::capsule(&AttachmentDto {
            duration_ms: None,
            ..circle()
        })
    );
    assert_eq!(
        None,
        round_look::capsule(&AttachmentDto {
            duration_ms: Some(0),
            ..circle()
        })
    );

    assert_eq!("Video message, 0:23", round_look::name(&circle(), &say));
    assert_eq!(
        "Video message",
        round_look::name(
            &AttachmentDto {
                duration_ms: None,
                ..circle()
            },
            &say
        )
    );

    assert_eq!(
        Some("Not played".to_string()),
        round_look::value(false, false, &say)
    );
    assert_eq!(
        Some("Played".to_string()),
        round_look::value(false, true, &say)
    );
}

/// THE DOT IS ONLY ON SOMEONE ELSE'S CIRCLE (S5.2, S6) — as on iOS, Android and the web: the
/// reader's own circles never carry it, nor a screen reader's "Not played" or "Played", whether or
/// not this device has played them. A `[Theory]`: one test over its rows.
#[test]
fn the_dot_is_only_on_someone_elses_unplayed_circle() {
    let say = common::english();
    for (mine, played, dot) in [
        (false, false, true),
        (false, true, false),
        (true, false, false),
        (true, true, false),
    ] {
        assert_eq!(dot, round_look::shows_dot(mine, played), "{mine} {played}");
        let expected = if mine {
            None
        } else if dot {
            Some("Not played".to_string())
        } else {
            Some("Played".to_string())
        };
        assert_eq!(
            expected,
            round_look::value(mine, played, &say),
            "{mine} {played}"
        );
    }
}

/// THE DOT GOES AT THE END (S5.3: "at the end it returns to the poster and loses its dot"), as on
/// every other client — not when it starts, not on a seek to the end of a clip never played here,
/// never for a clip of unknown length. A `[Theory]`: one test over its rows.
#[test]
fn it_counts_as_played_only_once_played_through() {
    for (seen_playing, position, total, through) in [
        (true, 0.0, 23.4, false),   // it has only just started
        (true, 12.0, 23.4, false),  // halfway
        (true, 23.1, 23.4, false),  // just short of the end
        (true, 23.15, 23.4, true),  // within the clock's tick of it
        (true, 23.4, 23.4, true),   // at the end
        (false, 23.4, 23.4, false), // seeked to the end, never played
        (true, 0.0, 0.0, false),    // a length not known
        (true, 5.0, f64::NAN, false),
        (true, f64::NAN, 23.4, false),
    ] {
        assert_eq!(
            through,
            round_look::played_through(seen_playing, position, total),
            "{seen_playing} {position} {total}"
        );
    }
}

/// The viewer's one ring: none before it plays, as far as it has played while it plays or is
/// paused part way — and none once it has FINISHED, when the mockup hides it and the circle is back
/// to its poster, never a full ring left round it. A `[Theory]`: one test over its rows.
#[test]
fn the_viewer_ring_goes_when_the_clip_ends() {
    for (seen_playing, playing, position, total, ring) in [
        (false, false, 0.0, 23.4, 0.0),
        (false, false, 11.7, 23.4, 0.0),
        (true, true, 11.7, 23.4, 0.5),
        (true, false, 11.7, 23.4, 0.5),
        (true, true, 23.4, 23.4, 1.0),
        (true, false, 23.4, 23.4, 0.0),
        (true, false, 23.3, 23.4, 0.0),
        (true, true, 5.0, 0.0, 0.0),
        (true, true, f64::NAN, 23.4, 0.0),
    ] {
        assert_near(
            ring,
            round_look::viewer_ring(seen_playing, playing, position, total),
            6,
            &format!("{seen_playing} {playing} {position} {total}"),
        );
    }
}

/// The viewer's circle: as large as the room allows, never larger than the recording's 480 pixels
/// and never smaller than the conversation's own circle. A `[Theory]`: one test over its rows.
#[test]
fn the_viewers_circle_fits_the_room_within_bounds() {
    for (width, height, chrome, diameter) in [
        (1600.0, 1000.0, 120.0, 480.0),
        (600.0, 1000.0, 120.0, 480.0),
        (400.0, 1000.0, 120.0, 352.0),
        (1600.0, 500.0, 120.0, 332.0),
        (300.0, 300.0, 120.0, 200.0),
        (1600.0, 500.0, -50.0, 452.0),
    ] {
        assert_eq!(
            diameter,
            round_look::viewer_diameter(width, height, chrome),
            "{width} {height} {chrome}"
        );
    }
}

#[test]
fn a_viewer_not_yet_measured_gets_the_largest_circle() {
    assert_eq!(
        round_look::VIEWER_LARGEST,
        round_look::viewer_diameter(0.0, 0.0, 100.0)
    );
    assert_eq!(
        round_look::VIEWER_LARGEST,
        round_look::viewer_diameter(f64::NAN, 900.0, 100.0)
    );
}

/// Not in the C#: a chrome that is not a number counts as none — `Math.Max(0, NaN)` let it through
/// and the C# drew a circle of NaN.
#[test]
fn a_chrome_that_is_not_a_number_counts_as_none() {
    assert_eq!(
        round_look::viewer_diameter(1600.0, 500.0, 0.0),
        round_look::viewer_diameter(1600.0, 500.0, f64::NAN)
    );
    assert_eq!(
        round_look::VIEWER_SMALLEST,
        round_look::viewer_diameter(1600.0, 500.0, f64::INFINITY)
    );
}

// ---- what pauses it (S4's last column) ------------------------------------------------------

/// A `[Theory]`: one test over its rows.
#[test]
fn what_pauses_what_plays() {
    use PlaybackEvent::*;
    use Playing::*;
    for (happened, playing, pauses) in [
        (Call, VoiceNote, true),
        (Call, RoundVideo, true),
        (SessionLocked, VoiceNote, true),
        (SessionLocked, RoundVideo, true),
        (OutputChanged, VoiceNote, true),
        (OutputChanged, RoundVideo, true),
        // Minimised or hidden: a circle is looked at and pauses; a voice note is heard and plays
        // on.
        (WindowHidden, RoundVideo, true),
        (WindowHidden, VoiceNote, false),
        // Only losing focus, still visible: everything plays on.
        (FocusLost, RoundVideo, false),
        (FocusLost, VoiceNote, false),
        // An ordinary video in the viewer is not in S4's column: this version leaves it as it
        // was.
        (Call, Video, false),
        (SessionLocked, Video, false),
        (OutputChanged, Video, false),
        (WindowHidden, Video, false),
    ] {
        assert_eq!(
            pauses,
            playback_pauses::pauses(happened, playing),
            "{happened:?} {playing:?}"
        );
    }
}

/// A call, a lock and a hidden window reach playback through what stops a recording; the rest stop
/// it outright.
#[test]
fn the_windows_interruptions_reach_playback() {
    assert_eq!(
        Some(PlaybackEvent::Call),
        playback_pauses::of(RecordingEnd::Call)
    );
    assert_eq!(
        Some(PlaybackEvent::SessionLocked),
        playback_pauses::of(RecordingEnd::SessionLocked)
    );
    assert_eq!(
        Some(PlaybackEvent::WindowHidden),
        playback_pauses::of(RecordingEnd::WindowHidden)
    );
    assert_eq!(None, playback_pauses::of(RecordingEnd::LeftChat));
    assert_eq!(None, playback_pauses::of(RecordingEnd::WindowClosed));
    assert_eq!(None, playback_pauses::of(RecordingEnd::SignedOut));
    assert_eq!(None, playback_pauses::of(RecordingEnd::Stopped));
    assert_eq!(None, playback_pauses::of(RecordingEnd::Sent));
}
