//! `RoundPlaybackTests`: a video message played IN PLACE (docs/audio-video-messages-2026-10-04.md,
//! S5.3, S5.4, S6) — the circle's machine (rest, loading, playing, paused, ended, failed), what a
//! click does in each, when its dot goes, the one ring, what shows, and when it is out of view.
//!
//! Not ported — their subjects are not on Linux, where GTK clips the playing video to the circle
//! itself (shell-design §1.5, §10.4) and there is no frame server (carry-forward, "Not ported"):
//! the six `RoundFrames` tests (`FramesAreCopiedAtTheClipsRateAtMostThirty`,
//! `ThrottlingKeepsTheRateAndOneCopyAtATime`, `TheFrameSurfaceFitsTheCircle`,
//! `PlayingBlindGivesUpOnTheFrames`, `AClipsRateFromItsRatio`, `ACopiedFrameIsOpaqueAndKeepsItsColour`)
//! and the two `RoundCrashGuard` tests (`OnlyTheBuildThatDiedPlayingInPlaceOpensTheViewer`,
//! `TheMarkNamesTheBuild`).

mod common;

use fc_core::protocol::AttachmentDto;
use fc_logic::round_look::{self, PlaybackEvent};
use fc_logic::round_playback::{
    RoundAct, RoundPhase, RoundPlayback, RoundStep, RoundTap, round_inline,
};

const A: i64 = 91;
const B: i64 = 92;

/// `new AttachmentDto(A, "video", "video/mp4", 1649700, 480, 480, 23400, HasPreview: true,
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
        ..AttachmentDto::new(A, "video")
    }
}

/// A machine with circle A playing: clicked, opened, playing, its first frame drawn.
fn playing_a() -> RoundPlayback {
    let mut round = RoundPlayback::new();
    round.tap(A, false);
    let load = round.load();
    assert!(round.opened(load, 23.4));
    round.playing(load);
    round.first_frame(load);
    round
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

// ---- a click ----------------------------------------------------------------------------------

/// A click at rest loads it — and nothing plays by itself: there is no circle until one is clicked
/// (no autoplay).
#[test]
fn a_tap_at_rest_loads_it() {
    let mut round = RoundPlayback::new();
    assert_eq!(None, round.id());
    assert_eq!(RoundPhase::Rest, round.phase_of(A));

    let step = round.tap(A, false);

    assert_eq!(RoundStep::new(RoundTap::Started, RoundAct::Load), step);
    assert_eq!(Some(A), round.id());
    assert_eq!(RoundPhase::Loading, round.phase());
    assert!(!round.has_frames());
    assert!(!round.seen_playing());
}

/// Opened, it is played only while still waited for; the player saying "playing" is what makes it
/// play.
#[test]
fn loading_then_playing() {
    let mut round = RoundPlayback::new();
    round.tap(A, false);
    let load = round.load();

    assert!(round.opened(load, 23.4));
    assert_eq!(RoundPhase::Loading, round.phase());
    assert_eq!(23.4, round.total());
    round.playing(load);
    assert_eq!(RoundPhase::Playing, round.phase());
    assert!(round.seen_playing());
    assert!(!round.has_frames());
    round.first_frame(load);
    assert!(round.has_frames());
}

/// A second click while it plays pauses it, keeping its frame; a third plays on.
#[test]
fn a_second_tap_pauses_and_a_third_plays_on() {
    let mut round = playing_a();

    assert_eq!(
        RoundStep::new(RoundTap::Paused, RoundAct::Pause),
        round.tap(A, false)
    );
    assert_eq!(RoundPhase::Paused, round.phase());
    assert!(round.has_frames());

    assert_eq!(
        RoundStep::new(RoundTap::Resumed, RoundAct::Play),
        round.tap(A, false)
    );
    assert_eq!(RoundPhase::Playing, round.phase());
}

/// "A second tap while it loads gives up" (S5.3): back to the poster, and the download landing
/// later plays nothing.
#[test]
fn a_second_tap_while_it_loads_gives_up() {
    let mut round = RoundPlayback::new();
    round.tap(A, false);
    let load = round.load();

    assert_eq!(
        RoundStep::new(RoundTap::GaveUp, RoundAct::Stop),
        round.tap(A, false)
    );
    assert_eq!(RoundPhase::Rest, round.phase());

    // The bytes land after all: stale, and nothing starts.
    assert!(!round.opened(load, 23.4));
    round.playing(load);
    round.first_frame(load);
    assert_eq!(RoundPhase::Rest, round.phase());
    assert!(!round.has_frames());
    assert_eq!(RoundAct::None, round.failed(load));
    assert_eq!(RoundPhase::Rest, round.phase());
}

/// A stall is loading too, and a click in it gives up as well.
#[test]
fn a_stall_shows_the_loading_ring_and_a_tap_gives_up() {
    let mut round = playing_a();
    round.waiting(round.load());
    assert_eq!(RoundPhase::Loading, round.phase());
    assert!(round_inline::shows_spinner(round.phase()));
    assert!(round_inline::shows_frames(
        round.phase(),
        round.has_frames()
    ));

    assert_eq!(RoundTap::GaveUp, round.tap(A, false).tap);
    assert_eq!(RoundPhase::Rest, round.phase());
}

/// Paused before the player opened, it stays paused once it opens.
#[test]
fn a_pause_before_the_opening_is_kept() {
    let mut round = RoundPlayback::new();
    round.tap(A, false);
    let load = round.load();
    round.first_frame(load);
    assert_eq!(RoundAct::Pause, round.make_way());

    assert!(!round.opened(load, 23.4));
    assert_eq!(RoundPhase::Paused, round.phase());
}

/// Nothing STARTS while a recording runs (S1.7); a pause is always allowed.
#[test]
fn nothing_starts_under_a_recording_but_a_pause_is_allowed() {
    let mut round = RoundPlayback::new();
    assert_eq!(
        RoundStep::new(RoundTap::Refused, RoundAct::None),
        round.tap(A, true)
    );
    assert_eq!(None, round.id());

    let mut playing = playing_a();
    assert_eq!(RoundTap::Paused, playing.tap(A, true).tap);
    assert_eq!(
        RoundStep::new(RoundTap::Refused, RoundAct::None),
        playing.tap(A, true)
    );
    assert_eq!(RoundPhase::Paused, playing.phase());
    assert_eq!(RoundTap::Refused, playing.tap(B, true).tap);
    assert_eq!(Some(A), playing.id());
}

/// One circle at a time: clicking another takes the player from the first, which goes back to its
/// poster.
#[test]
fn tapping_another_circle_replaces_the_first() {
    let mut round = playing_a();
    let first = round.load();

    let step = round.tap(B, false);

    assert_eq!(
        RoundStep {
            tap: RoundTap::Started,
            act: RoundAct::Load,
            replaced: Some(A),
        },
        step
    );
    assert_eq!(Some(B), round.id());
    assert_eq!(RoundPhase::Rest, round.phase_of(A));
    assert_eq!(RoundPhase::Loading, round.phase_of(B));
    // The first one's late news reaches nothing.
    round.playing(first);
    assert_eq!(RoundPhase::Loading, round.phase());
    assert_eq!((RoundAct::None, false), round.ended(first));
}

// ---- the end, failures ------------------------------------------------------------------------

/// At the end it returns to the poster and, played through here, loses its dot (S5.3).
#[test]
fn the_end_returns_to_the_poster_and_the_dot_goes() {
    let mut round = playing_a();
    round.progress(round.load(), 23.4, 23.4);

    assert_eq!((RoundAct::Stop, true), round.ended(round.load()));
    assert_eq!(RoundPhase::Rest, round.phase());
    assert!(!round.has_frames());
    assert_eq!(0.0, round.position());
    assert_eq!(
        0.0,
        round_inline::ring(
            round.phase(),
            round.seen_playing(),
            round.position(),
            round.total(),
            false
        )
    );
    assert!(round_inline::shows_disc(round.phase()));

    // And a click plays it again from the start.
    assert_eq!(RoundTap::Started, round.tap(A, false).tap);
}

/// An end the player reports before it was ever seen playing is no play-through: the dot stays.
#[test]
fn an_end_never_seen_playing_keeps_the_dot() {
    let mut round = RoundPlayback::new();
    round.tap(A, false);
    assert_eq!((RoundAct::Stop, false), round.ended(round.load()));
}

/// A failure waits with its sentence, and a click tries again.
#[test]
fn a_failure_waits_for_a_tap_to_try_again() {
    let mut round = RoundPlayback::new();
    round.tap(A, false);
    let load = round.load();

    assert_eq!(RoundAct::Stop, round.failed(load));
    assert_eq!(RoundPhase::Failed, round.phase());
    assert!(round_inline::shows_failure(round.phase()));
    assert!(round_inline::shows_disc(round.phase()));

    let again = round.tap(A, false);
    assert_eq!(RoundStep::new(RoundTap::Started, RoundAct::Load), again);
    assert_ne!(load, round.load());
    assert!(!round_inline::shows_failure(round.phase()));
}

// ---- one thing at a time, interruptions, stopping ---------------------------------------------

/// Something else starts — a voice note, the viewer, a recording: a playing circle pauses; one only
/// loading gives up. (`Yield` is `make_way` here: `yield` is a Rust keyword.)
#[test]
fn it_makes_way_for_whatever_else_starts() {
    let mut playing = playing_a();
    assert_eq!(RoundAct::Pause, playing.make_way());
    assert_eq!(RoundPhase::Paused, playing.phase());
    assert_eq!(RoundAct::None, playing.make_way());

    let mut loading = RoundPlayback::new();
    loading.tap(A, false);
    assert_eq!(RoundAct::Stop, loading.make_way());
    assert_eq!(RoundPhase::Rest, loading.phase());

    assert_eq!(RoundAct::None, RoundPlayback::new().make_way());
}

/// A call, a lock, a new output device and a hidden window pause it (S4); losing focus does not. A
/// `[Theory]`: one test over its rows.
#[test]
fn what_pauses_it_in_place() {
    for (happened, act) in [
        (PlaybackEvent::Call, RoundAct::Pause),
        (PlaybackEvent::SessionLocked, RoundAct::Pause),
        (PlaybackEvent::OutputChanged, RoundAct::Pause),
        (PlaybackEvent::WindowHidden, RoundAct::Pause),
        (PlaybackEvent::FocusLost, RoundAct::None),
    ] {
        let mut round = playing_a();
        assert_eq!(act, round.interrupt(happened), "{happened:?}");
        let phase = if act == RoundAct::Pause {
            RoundPhase::Paused
        } else {
            RoundPhase::Playing
        };
        assert_eq!(phase, round.phase(), "{happened:?}");
    }
}

/// Its row scrolled away or its chat closed: it stops, back to the poster, and says which circle
/// that was.
#[test]
fn stopping_returns_it_to_the_poster() {
    let mut round = playing_a();
    let load = round.load();

    assert_eq!(Some(A), round.stop());
    assert_eq!(None, round.id());
    assert_eq!(RoundPhase::Rest, round.phase_of(A));
    round.playing(load);
    assert_eq!(RoundPhase::Rest, round.phase());
    assert_eq!(None, round.stop());
}

/// The system pausing the player is a pause, not a stop.
#[test]
fn the_player_pausing_by_itself_is_a_pause() {
    let mut round = playing_a();
    round.paused_by_player(round.load());
    assert_eq!(RoundPhase::Paused, round.phase());
    assert!(round_inline::shows_disc(round.phase()));
}

// ---- how it is drawn --------------------------------------------------------------------------

/// The ring follows the time while it plays, steps once a second under Reduce Motion, and is never
/// drawn at rest. A `[Theory]`: one test over its rows.
#[test]
fn the_one_ring() {
    use RoundPhase::*;
    for (phase, seen, position, total, reduced, ring) in [
        (Playing, true, 5.5, 22.0, false, 0.25),
        (Playing, true, 5.5, 22.0, true, 5.0 / 22.0),
        (Paused, true, 11.0, 22.0, false, 0.5),
        (Loading, true, 11.0, 22.0, false, 0.5),
        (Loading, false, 0.0, 22.0, false, 0.0),
        (Rest, true, 11.0, 22.0, false, 0.0),
        (Failed, true, 11.0, 22.0, false, 0.0),
        (Playing, true, 30.0, 22.0, false, 1.0),
        (Playing, true, 3.0, 0.0, false, 0.0),
        (Playing, true, f64::NAN, 22.0, false, 0.0),
    ] {
        assert_near(
            ring,
            round_inline::ring(phase, seen, position, total, reduced),
            6,
            &format!("{phase:?} {seen} {position} {total} {reduced}"),
        );
    }
}

/// What shows in each phase: the disc, the loading ring, the frames over the poster, the expand
/// control. A `[Theory]`: one test over its rows.
#[test]
fn what_shows() {
    use RoundPhase::*;
    for (phase, has_frames, disc, spinner, frames) in [
        (Rest, false, true, false, false),
        (Loading, false, false, true, false),
        (Loading, true, false, true, true),
        (Playing, false, false, false, false),
        (Playing, true, false, false, true),
        (Paused, true, true, false, true),
        (Failed, false, true, false, false),
    ] {
        assert_eq!(disc, round_inline::shows_disc(phase), "{phase:?}");
        assert_eq!(spinner, round_inline::shows_spinner(phase), "{phase:?}");
        assert_eq!(
            frames,
            round_inline::shows_frames(phase, has_frames),
            "{phase:?}"
        );
        assert_eq!(
            frames,
            round_inline::shows_expand(phase, has_frames),
            "{phase:?}"
        );
    }
}

/// The capsule counts up once it has started, and says the length otherwise; a screen reader hears
/// what a press does.
#[test]
fn the_capsule_and_the_action() {
    let say = common::english();
    let video = circle();
    let at = |phase, seen, position| round_inline::capsule(&video, phase, seen, position);
    assert_eq!(Some("0:23".to_string()), at(RoundPhase::Rest, false, 0.0));
    assert_eq!(
        Some("0:23".to_string()),
        at(RoundPhase::Loading, false, 0.0)
    );
    assert_eq!(Some("0:07".to_string()), at(RoundPhase::Playing, true, 7.2));
    assert_eq!(Some("0:07".to_string()), at(RoundPhase::Paused, true, 7.2));

    assert_eq!("Pause", round_inline::action(RoundPhase::Playing, &say));
    assert_eq!("Pause", round_inline::action(RoundPhase::Loading, &say));
    assert_eq!("Play", round_inline::action(RoundPhase::Paused, &say));
    assert_eq!("Play", round_inline::action(RoundPhase::Rest, &say));
    assert_eq!("Play", round_inline::action(RoundPhase::Failed, &say));
}

/// In place is this build's choice; the viewer stays Open Full Screen's.
#[test]
fn this_build_plays_circles_in_place() {
    const { assert!(round_inline::PLAYS_IN_PLACE) };
}

/// The ring runs outside the edge, past the gap — never over the picture.
#[test]
fn the_ring_is_outside_the_edge() {
    assert_eq!(
        round_inline::RING_GAP + round_look::RING,
        round_inline::RING_REACH
    );
}

/// Wholly out of its scroller's view, it is out of view; any part seen, or no scroller at all, it
/// is in. A `[Theory]`: one test over its rows.
#[test]
fn in_view() {
    for (x, y, w, h, seen) in [
        (0.0, 0.0, 400.0, 600.0, true),
        (-100.0, -200.0, 400.0, 300.0, true),
        (0.0, -600.0, 400.0, 600.0, false),
        (0.0, -360.0, 400.0, 600.0, true),
        (0.0, 240.0, 400.0, 600.0, false),
        (0.0, 239.0, 400.0, 600.0, true),
        (500.0, 0.0, 400.0, 600.0, false),
        (0.0, 0.0, 0.0, 0.0, false),
        (0.0, 0.0, -1.0, -1.0, false),
    ] {
        assert_eq!(
            seen,
            round_inline::in_view(x, y, w, h, 240.0, 240.0),
            "{x} {y} {w} {h}"
        );
    }
}

/// An empty rectangle (infinite corner, negative infinite size — WinUI's, and what a shell may
/// hand over for a widget not mapped) is out of view; an unbounded one is in.
#[test]
fn empty_and_unbounded_viewports() {
    assert!(!round_inline::in_view(
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
        240.0,
        240.0
    ));
    assert!(round_inline::in_view(
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::INFINITY,
        240.0,
        240.0
    ));
    assert!(!round_inline::in_view(
        f64::NAN,
        0.0,
        400.0,
        600.0,
        240.0,
        240.0
    ));
}
