//! `RoundRecorderTests`: the video-message recorder's three states as a machine
//! (docs/audio-video-messages-2026-10-04.md, S3.2, S3.4, S3.6, S4, S6): every press, tick and
//! interruption, and what the window is told to do — the camera off in every refusal and in REVIEW,
//! the microphone only from Record, nothing sent but by Send, and closing over a clip asked about.
//! A `[Theory]` is one `#[test]` looping over its rows.

mod common;

use std::rc::Rc;

use fc_core::protocol::RoundVideoLimits;
use fc_logic::round_look::PlaybackEvent;
use fc_logic::round_recorder::{
    RecorderAction, RecorderEffect, RecorderInterruption, RecorderQuestion, RecorderStage,
    RecorderStatus, RoundRecorder,
};
use fc_logic::round_video_rules::{CameraTrouble, RecorderRefusal, RoundSendPlan};

use RecorderAction::*;

const LIMITS: RoundVideoLimits = RoundVideoLimits {
    max_ms: 60_000,
    max_bytes: 12_582_912,
};

fn round() -> Option<RoundSendPlan> {
    Some(RoundSendPlan::new(true, true, None))
}

fn recorder() -> RoundRecorder {
    RoundRecorder::new(LIMITS, Rc::new(common::english()))
}

fn actions(effects: &[RecorderEffect]) -> Vec<RecorderAction> {
    effects
        .iter()
        .filter(|effect| effect.action != Announce)
        .map(|effect| effect.action)
        .collect()
}

fn said(effects: &[RecorderEffect]) -> Vec<String> {
    effects
        .iter()
        .filter(|effect| effect.action == Announce)
        .map(|effect| effect.text.clone().unwrap_or_default())
        .collect()
}

/// A recorder opened at `at` with the camera on and its first frame in: PREVIEW, ready to record.
fn previewing_at(at: i64) -> RoundRecorder {
    let mut recorder = recorder();
    recorder.open(at, None, false);
    recorder.camera_on(at);
    recorder.first_frame();
    recorder
}

fn previewing() -> RoundRecorder {
    previewing_at(0)
}

/// Recording from `at` — pressed then, and the file begun then.
fn recording_at(at: i64) -> RoundRecorder {
    let mut recorder = previewing();
    recorder.slot(at);
    recorder.take_started(at);
    recorder
}

fn recording() -> RoundRecorder {
    recording_at(1_000)
}

/// In REVIEW with a clip of `length`, made and ready.
fn reviewing_for(length: i64) -> RoundRecorder {
    let mut recorder = recording_at(1_000);
    recorder.slot(1_000 + length);
    recorder.clip_finished(1_000 + length + 100, round(), Some(length));
    recorder
}

fn reviewing() -> RoundRecorder {
    reviewing_for(5_000)
}

// ---- opening (S3.2) ---------------------------------------------------------------------------

/// Opened: the camera is asked for, the neutral line while the desktop asks, "Starting camera…"
/// otherwise.
#[test]
fn opening_asks_for_the_camera() {
    let mut recorder = recorder();
    assert!(!recorder.is_open());
    assert_eq!(vec![OpenCamera], actions(&recorder.open(0, None, true)));
    assert_eq!(RecorderStage::Opening, recorder.stage());
    assert!(recorder.asking());
    assert_eq!(
        "Video messages need the camera and the microphone.",
        recorder.status(0, false, false).line
    );

    let mut again = self::recorder();
    again.open(0, None, false);
    assert_eq!("Starting camera…", again.status(0, false, false).line);
}

/// Something already refused: straight to it, and NOTHING is opened — no prompt for the other
/// device.
#[test]
fn an_earlier_refusal_opens_straight_to_it_with_the_camera_off() {
    let mut recorder = recorder();
    assert!(
        recorder
            .open(0, Some(RecorderRefusal::Camera), true)
            .is_empty()
    );
    assert_eq!(RecorderStage::Refused, recorder.stage());
    assert_eq!(
        "Family needs permission to use your camera. Turn it on in Settings.",
        recorder.status(0, false, false).line
    );
    // Record is not live; a voice message is offered instead.
    assert!(recorder.slot(5_000).is_empty());
    assert_eq!(
        vec![StartVoice, CloseCamera, Closed],
        actions(&recorder.voice_instead(false))
    );
}

/// The microphone refused: nothing else is offered — voice needs it too.
#[test]
fn a_microphone_refusal_offers_no_voice_message() {
    let mut recorder = recorder();
    recorder.open(0, Some(RecorderRefusal::Microphone), false);
    assert_eq!(
        "Family needs permission to use your microphone. Turn it on in Settings.",
        recorder.status(0, false, false).line
    );
    assert!(recorder.voice_instead(false).is_empty());
    assert!(recorder.is_open());
}

/// Refused while the desktop asked: the camera goes off at once.
#[test]
fn a_refusal_while_opening_turns_the_camera_off() {
    let mut recorder = recorder();
    recorder.open(0, None, true);
    assert_eq!(
        vec![CloseCamera],
        actions(&recorder.refuse(RecorderRefusal::Camera))
    );
    assert_eq!(RecorderStage::Refused, recorder.stage());
    assert!(!recorder.asking());
}

/// A camera that opened after the recorder closed is let go of at once.
#[test]
fn a_camera_that_opens_too_late_is_let_go() {
    let mut recorder = recorder();
    recorder.open(0, None, false);
    recorder.close_button();
    assert_eq!(vec![CloseCamera], actions(&recorder.camera_on(100)));
}

// ---- PREVIEW (S3.4) ---------------------------------------------------------------------------

/// "Starting camera…" until the first frame, Record dimmed; then "Not recording", the first-time
/// line under it, "Camera ready".
#[test]
fn record_waits_for_the_first_frame() {
    let mut recorder = recorder();
    recorder.open(0, None, false);
    recorder.camera_on(10);
    assert_eq!(RecorderStage::Preview, recorder.stage());
    assert_eq!("Starting camera…", recorder.status(10, true, false).line);
    assert!(recorder.slot(1_000).is_empty());
    assert_eq!(RecorderStage::Preview, recorder.stage());
    assert_eq!(vec!["Camera ready"], said(&recorder.first_frame()));
    assert!(recorder.first_frame().is_empty());
    assert_eq!(
        RecorderStatus::new(
            "Not recording",
            Some("Only you can see this until you start recording."),
            false,
            false
        ),
        recorder.status(20, true, false)
    );
    assert_eq!(
        RecorderStatus::new("Not recording", None, false, false),
        recorder.status(20, false, false)
    );
}

/// Where the microphone is held in PREVIEW, PREVIEW does not claim otherwise.
#[test]
fn a_held_microphone_is_said() {
    assert_eq!(
        "Camera and microphone on · Not recording",
        previewing().status(0, false, true).line
    );
}

/// A picture that stayed black adds the question; Record stays usable — a dark room is not an
/// error.
#[test]
fn a_covered_camera_is_asked_about_and_record_stays_live() {
    let mut recorder = previewing();
    recorder.covered();
    assert_eq!(
        Some("We can't see anything. Is the camera turned off or covered?".to_string()),
        recorder.status(0, true, false).below
    );
    assert!(actions(&recorder.slot(3_000)).contains(&StartRecording));
}

/// Record: the microphone opens now, "Recording video" is said, the red clock runs.
#[test]
fn record_opens_the_microphone_only_now() {
    let mut recorder = previewing();
    let effects = recorder.slot(1_000);
    assert_eq!(vec![StartRecording], actions(&effects));
    assert_eq!(vec!["Recording video"], said(&effects));
    assert_eq!(RecorderStage::Recording, recorder.stage());
    assert!(recorder.holds_clip());
    recorder.take_started(1_000);
    assert_eq!(
        RecorderStatus::new("0:12", None, true, false),
        recorder.status(13_000, false, false)
    );
}

/// The clock starts with the take, not the press (S6: with a screen reader the microphone opens a
/// second after Record, once "Recording video" is said, "and the timer starts with it"): until the
/// file begins it shows 0:00 and the ring is empty, and a Stop 1.3 s after the press but 0.3 s into
/// the take is too short — nothing under 1.0 s is sent (S1.1).
#[test]
fn the_clock_starts_with_the_take_not_the_press() {
    let mut recorder = previewing();
    recorder.slot(1_000);
    assert_eq!("0:00", recorder.status(1_900, false, false).line);
    assert_eq!(0.0, recorder.ring_fraction(1_900));
    assert_eq!(0, recorder.elapsed_ms(1_900));
    recorder.take_started(2_000);
    // A second report of the same start changes nothing.
    recorder.take_started(2_200);
    assert_eq!(300, recorder.elapsed_ms(2_300));
    let effects = recorder.slot(2_300);
    assert_eq!(vec![StopAndDiscard], actions(&effects));
    assert_eq!(vec!["That video was too short."], said(&effects));
    assert_eq!(RecorderStage::Preview, recorder.stage());

    // And the limit counts from the take too: 59.5 s of FILE, whatever the lead-in took.
    let mut long = previewing();
    long.slot(1_000);
    long.take_started(2_000);
    assert!(actions(&long.tick(60_499)).is_empty());
    assert!(actions(&long.tick(61_500)).contains(&StopAndKeep));
    assert_eq!(59_500, long.clip_ms());
}

/// A Stop, or an interruption, before the take began at all: nothing was recorded, so nothing is
/// kept.
#[test]
fn nothing_is_kept_before_the_take_begins() {
    let mut stopped = previewing();
    stopped.slot(1_000);
    assert_eq!(vec![StopAndDiscard], actions(&stopped.slot(5_000)));
    assert_eq!(RecorderStage::Preview, stopped.stage());

    let mut called = previewing();
    called.slot(1_000);
    assert_eq!(
        vec![StopAndDiscard, CloseCamera, Closed],
        actions(&called.interrupt(RecorderInterruption::Call, 5_000))
    );

    // A take that would not start, then a new one: the new one waits for its own start.
    let mut failed = previewing();
    failed.slot(1_000);
    failed.take_started(1_000);
    failed.record_failed();
    failed.slot(3_000);
    assert_eq!(0, failed.elapsed_ms(8_000));
}

/// The clip READ BACK under 1.0 s is never sent (S1.1), whatever the clock said: after the
/// person's Stop, PREVIEW again with "That video was too short."; after an interruption, deleted
/// and closed (S4); with a closing window's question up, the close goes ahead — there is nothing to
/// keep.
#[test]
fn a_clip_that_reads_back_under_a_second_is_never_sent() {
    let mut recorder = recording_at(1_000);
    recorder.slot(2_100);
    let effects = recorder.clip_finished(3_000, round(), Some(950));
    assert_eq!(vec![DiscardClip, OpenCamera], actions(&effects));
    assert_eq!(vec!["That video was too short."], said(&effects));
    assert_eq!(RecorderStage::Opening, recorder.stage());
    assert!(!recorder.clip_ready());
    assert_eq!(None, recorder.plan());
    assert!(recorder.slot(10_000).is_empty());
    assert_eq!(Some("That video was too short."), recorder.notice());

    // Exactly a second is enough.
    let mut second = recording_at(1_000);
    second.slot(2_100);
    second.clip_finished(3_000, round(), Some(1_000));
    assert!(second.clip_ready());

    let mut called = recording_at(1_000);
    called.interrupt(RecorderInterruption::Call, 2_100);
    assert_eq!(
        vec![DiscardClip, Closed],
        actions(&called.clip_finished(3_000, round(), Some(900)))
    );
    assert!(!called.is_open());

    let mut closing = recording_at(1_000);
    closing.interrupt(RecorderInterruption::WindowClosing, 2_100);
    assert_eq!(RecorderQuestion::CloseWindow, closing.question());
    assert_eq!(
        vec![Dismiss, DiscardClip, CloseWindow, Closed],
        actions(&closing.clip_finished(3_000, round(), Some(900)))
    );
    assert!(!closing.is_open());

    // A person's Stop after an earlier interrupted take is the person's again.
    let mut later = recording_at(1_000);
    later.interrupt(RecorderInterruption::Call, 5_000);
    later.clip_finished(5_100, round(), Some(4_000));
    later.retake(6_000);
    later.camera_on(6_100);
    later.first_frame();
    later.slot(7_000);
    // The new take waits for its own start, whatever the last one did.
    assert_eq!(0, later.elapsed_ms(7_500));
    later.take_started(7_000);
    later.slot(8_100);
    assert_eq!(
        vec![DiscardClip, OpenCamera],
        actions(&later.clip_finished(8_200, round(), Some(900)))
    );
}

/// A minute with nothing used turns the camera off — and says so — and loses nothing.
#[test]
fn preview_closes_after_a_minute_untouched() {
    let mut recorder = previewing();
    assert!(recorder.tick(59_999).is_empty());
    recorder.used(30_000);
    assert!(recorder.tick(89_999).is_empty());
    let effects = recorder.tick(90_000);
    assert_eq!(vec![CloseCamera, Closed], actions(&effects));
    assert_eq!(vec!["Camera turned off"], said(&effects));
    assert!(!recorder.is_open());
}

/// Not while the desktop asks: a person reading a permission prompt is not idle.
#[test]
fn the_prompt_does_not_time_out() {
    let mut recorder = recorder();
    recorder.open(0, None, true);
    assert!(recorder.tick(120_000).is_empty());
    assert!(recorder.is_open());
}

/// Close and Esc close PREVIEW, the camera off.
#[test]
fn close_and_escape_close_preview() {
    assert_eq!(
        vec![CloseCamera, Closed],
        actions(&previewing().close_button())
    );
    assert_eq!(vec![CloseCamera, Closed], actions(&previewing().escape(5)));
    assert!(recording().close_button().is_empty());
}

/// "Record a voice message instead": the camera closes and a voice recording starts — unless one
/// not sent waits.
#[test]
fn voice_instead_closes_the_camera_unless_a_note_waits() {
    let mut recorder = previewing();
    let waits = recorder.voice_instead(true);
    assert!(actions(&waits).is_empty());
    assert_eq!(
        vec!["Send or delete the voice message that wasn't sent first."],
        said(&waits)
    );
    assert!(recorder.is_open());
    assert_eq!(
        vec![StartVoice, CloseCamera, Closed],
        actions(&recorder.voice_instead(false))
    );
    assert!(recording().voice_instead(false).is_empty());
}

/// Another camera chosen: this one off, that one on, and the picture awaited again.
#[test]
fn choosing_a_camera_reopens_it() {
    let mut recorder = previewing();
    assert_eq!(
        vec![CloseCamera, OpenCamera],
        actions(&recorder.switch_camera(10))
    );
    assert_eq!(RecorderStage::Opening, recorder.stage());
    assert!(!recorder.has_picture());
    assert!(recording().switch_camera(10).is_empty());
}

// ---- RECORDING (S3.4) -------------------------------------------------------------------------

/// The slot ignores clicks for 600 ms after its own Record: a double click cannot stop the take.
#[test]
fn a_double_click_on_record_does_not_stop() {
    let mut recorder = recording_at(1_000);
    assert!(recorder.slot(1_599).is_empty());
    assert_eq!(RecorderStage::Recording, recorder.stage());
    assert!(actions(&recorder.slot(5_000)).contains(&StopAndKeep));
}

/// Stop under a second: back to PREVIEW, "That video was too short.", nothing kept.
#[test]
fn a_stop_under_a_second_is_too_short() {
    let mut recorder = recording_at(1_000);
    recorder.slot(1_000);
    // Esc is Stop too, and is not the slot's guard.
    let effects = recorder.escape(1_999);
    assert_eq!(vec![StopAndDiscard], actions(&effects));
    assert_eq!(vec!["That video was too short."], said(&effects));
    assert_eq!(RecorderStage::Preview, recorder.stage());
    assert_eq!(
        Some("That video was too short.".to_string()),
        recorder.status(2_000, false, false).below
    );
}

/// Stop: the take is kept, the camera goes off, and REVIEW waits for the clip.
#[test]
fn stop_goes_to_review_with_the_camera_off() {
    let mut recorder = recording_at(1_000);
    assert_eq!(
        vec![StopAndKeep, CloseCamera],
        actions(&recorder.slot(24_400))
    );
    assert_eq!(RecorderStage::Review, recorder.stage());
    assert!(!recorder.clip_ready());
    assert_eq!(23_400, recorder.clip_ms());
    assert_eq!(
        "Video message · 0:23",
        recorder.status(30_000, false, false).line
    );
    // Send waits for the clip.
    assert!(recorder.slot(30_000).is_empty());
    assert_eq!(RecorderStage::Review, recorder.stage());
}

/// "10 seconds left" at 50 s, said once, with the ring orange; at 59.5 s the take stops itself —
/// into REVIEW, never sent.
#[test]
fn the_limit_warns_then_stops_into_review() {
    let mut recorder = recording_at(0);
    assert!(recorder.tick(49_999).is_empty());
    assert!(!recorder.warns(49_999));
    assert_eq!(vec!["10 seconds left"], said(&recorder.tick(50_000)));
    assert!(recorder.tick(50_250).is_empty());
    assert!(recorder.warns(50_250));
    assert_eq!(
        RecorderStatus::new("0:50", Some("10 seconds left"), true, true),
        recorder.status(50_250, false, false)
    );
    let six = |value: f64| (value * 1e6).round_ties_even() / 1e6;
    assert_eq!(
        six(50_000.0 / 59_500.0),
        six(recorder.ring_fraction(50_000))
    );
    let stopped = recorder.tick(59_500);
    assert_eq!(vec![StopAndKeep, CloseCamera], actions(&stopped));
    assert_eq!(vec!["Recording stopped at one minute."], said(&stopped));
    assert!(!actions(&stopped).contains(&Send));
    assert_eq!(RecorderStage::Review, recorder.stage());
    assert_eq!(0.0, recorder.ring_fraction(60_000));
}

/// Delete under ten seconds: gone at once, back to PREVIEW with the camera still on.
#[test]
fn deleting_a_short_take_is_immediate() {
    let mut recorder = recording_at(1_000);
    let effects = recorder.delete(10_999);
    assert_eq!(vec![StopAndDiscard], actions(&effects));
    assert_eq!(vec!["Recording deleted"], said(&effects));
    assert_eq!(RecorderStage::Preview, recorder.stage());
}

/// Delete from ten seconds: it stops first and asks; Delete goes back to PREVIEW, Keep to REVIEW
/// with the light out.
#[test]
fn deleting_a_long_take_stops_and_asks() {
    let mut recorder = recording_at(1_000);
    assert_eq!(vec![StopAndKeep, Ask], actions(&recorder.delete(11_000)));
    assert_eq!(RecorderQuestion::DeleteWhileRecording, recorder.question());
    assert_eq!(RecorderStage::Review, recorder.stage());
    assert_eq!(vec![DiscardClip], actions(&recorder.answer(true, 12_000)));
    assert_eq!(RecorderStage::Preview, recorder.stage());
    assert!(recorder.has_picture());

    let mut kept = recording_at(1_000);
    kept.delete(11_000);
    assert_eq!(vec![CloseCamera], actions(&kept.answer(false, 12_000)));
    assert_eq!(RecorderStage::Review, kept.stage());
    assert_eq!(RecorderQuestion::None, kept.question());
}

/// The take would not start: back to PREVIEW, saying so.
#[test]
fn a_take_that_would_not_start_says_so() {
    let mut recorder = recording_at(1_000);
    assert_eq!(
        vec!["Couldn't start recording."],
        said(&recorder.record_failed())
    );
    assert_eq!(RecorderStage::Preview, recorder.stage());
    assert!(previewing().record_failed().is_empty());
}

// ---- the camera's trouble (S3.6) --------------------------------------------------------------

/// In PREVIEW, the sentence in place of the picture and the camera off; mid-take, REVIEW with what
/// was recorded.
#[test]
fn a_camera_taken_away_stops_into_review_or_says_why() {
    let mut preview = previewing();
    assert_eq!(
        vec![CloseCamera],
        actions(&preview.camera_troubled(CameraTrouble::InUse, 100))
    );
    assert_eq!(RecorderStage::Unavailable, preview.stage());
    assert_eq!(
        "The camera is being used by another app.",
        preview.status(100, false, false).line
    );
    assert!(preview.slot(5_000).is_empty());

    let mut unsupported = recorder();
    unsupported.open(0, None, false);
    unsupported.camera_troubled(CameraTrouble::Unsupported, 10);
    assert_eq!(
        "Video messages can't be recorded with this camera.",
        unsupported.status(10, false, false).line
    );

    let mut recording = recording_at(1_000);
    let effects = recording.camera_troubled(CameraTrouble::InUse, 8_000);
    assert_eq!(vec![StopAndKeep, CloseCamera], actions(&effects));
    assert_eq!(
        vec!["The camera is being used by another app."],
        said(&effects)
    );
    assert_eq!(RecorderStage::Review, recording.stage());
    assert_eq!(
        Some("The camera is being used by another app.".to_string()),
        recording.status(8_000, false, false).below
    );

    // Gone or failed mid-take: what was recorded, and what S4 says of a recorder that fails.
    let mut pulled = recording_at(1_000);
    assert_eq!(
        vec!["The recording stopped unexpectedly."],
        said(&pulled.camera_troubled(CameraTrouble::Missing, 8_000))
    );
    assert_eq!(RecorderStage::Review, pulled.stage());
}

// ---- REVIEW (S3.4, S3.6) ----------------------------------------------------------------------

/// Space is the recorder's in REVIEW — from the moment the take stops, while the clip is still
/// being made too — and never reaches a focused Delete or Retake (S3.4). Elsewhere it is the
/// focused control's, and under a question the question's.
#[test]
fn space_is_the_recorders_throughout_review() {
    assert!(!previewing().catches_space());
    assert!(!recording().catches_space());
    let mut making = recording_at(1_000);
    making.slot(6_000);
    assert!(!making.clip_ready());
    assert!(making.catches_space());
    assert!(reviewing().catches_space());
    let mut asking = reviewing_for(12_000);
    asking.escape(20_000);
    assert!(!asking.catches_space());
}

/// The clip REVIEW plays back is a circle playing (S4's last column): a call, a lock, a hidden
/// window and the output device going pause it; losing focus does not; and there is nothing to
/// pause before the clip is ready or outside REVIEW.
#[test]
fn review_playback_pauses_as_a_circle_does() {
    for (happened, pauses) in [
        (PlaybackEvent::Call, true),
        (PlaybackEvent::SessionLocked, true),
        (PlaybackEvent::WindowHidden, true),
        (PlaybackEvent::OutputChanged, true),
        (PlaybackEvent::FocusLost, false),
    ] {
        let recorder = reviewing();
        let expected = if pauses { vec![PauseClip] } else { Vec::new() };
        assert_eq!(
            expected,
            actions(&recorder.playback_interrupted(happened)),
            "{happened:?}"
        );
        // Paused, REVIEW is kept.
        assert_eq!(RecorderStage::Review, recorder.stage());
        assert!(recorder.clip_ready());

        let mut making = recording_at(1_000);
        making.slot(6_000);
        assert!(making.playback_interrupted(happened).is_empty());
        assert!(previewing().playback_interrupted(happened).is_empty());
        assert!(recording().playback_interrupted(happened).is_empty());
    }
}

/// The clip made: REVIEW's line said, Send live — and Send sends, says so, and closes.
#[test]
fn send_sends_once_the_clip_is_ready() {
    let mut recorder = recording_at(1_000);
    recorder.slot(24_400);
    assert_eq!(
        vec!["Video message · 0:23"],
        said(&recorder.clip_finished(25_000, round(), Some(23_450)))
    );
    assert!(recorder.clip_ready());
    assert_eq!(23_450, recorder.clip_ms());
    // The guard after Stop still holds.
    assert!(recorder.slot(24_999).is_empty());
    let sent = recorder.slot(25_000);
    assert_eq!(vec![Send, Closed], actions(&sent));
    assert_eq!(vec!["Video message sent"], said(&sent));
    assert!(!recorder.is_open());
}

/// A clip that cannot go round says so in REVIEW, and Send still sends it — as what the plan says.
#[test]
fn a_clip_that_cannot_go_round_says_so() {
    let mut recorder = recording_at(1_000);
    recorder.slot(6_000);
    let notice = "Couldn't make it round. It will be sent as a regular video.";
    let plan = RoundSendPlan::new(false, false, Some(notice));
    assert_eq!(
        vec![notice],
        said(&recorder.clip_finished(7_000, Some(plan.clone()), Some(5_000)))
    );
    assert_eq!(Some(&plan), recorder.plan());
    assert_eq!(
        Some(notice.to_string()),
        recorder.status(7_000, false, false).below
    );
    assert!(actions(&recorder.slot(8_000)).contains(&Send));
}

/// A take nothing could be read back from: PREVIEW again, "The recording stopped unexpectedly."
#[test]
fn a_take_that_could_not_be_read_goes_back_to_preview() {
    let mut recorder = recording_at(1_000);
    recorder.slot(6_000);
    let effects = recorder.clip_finished(7_000, None, None);
    assert_eq!(vec![Dismiss, DiscardClip, OpenCamera], actions(&effects));
    assert_eq!(vec!["The recording stopped unexpectedly."], said(&effects));
    assert_eq!(RecorderStage::Opening, recorder.stage());
    assert_eq!(
        Some("The recording stopped unexpectedly."),
        recorder.notice()
    );
}

/// Only once, and only in REVIEW.
#[test]
fn a_clip_lands_once() {
    let mut recorder = reviewing();
    assert!(recorder.clip_finished(9_000, round(), Some(1)).is_empty());
    assert_eq!(5_000, recorder.clip_ms());
    assert!(
        previewing()
            .clip_finished(9_000, round(), Some(1))
            .is_empty()
    );
}

/// Delete under ten seconds closes at once; from ten it asks, and Delete closes.
#[test]
fn delete_in_review_asks_from_ten_seconds() {
    assert_eq!(
        vec![DiscardClip, Closed],
        actions(&reviewing_for(9_999).delete(20_000))
    );
    let mut recorder = reviewing_for(10_000);
    assert_eq!(vec![Ask], actions(&recorder.delete(20_000)));
    assert_eq!(RecorderQuestion::DeleteReview, recorder.question());
    assert!(recorder.answer(false, 21_000).is_empty());
    assert_eq!(RecorderStage::Review, recorder.stage());
    recorder.delete(22_000);
    assert_eq!(
        vec![DiscardClip, Closed],
        actions(&recorder.answer(true, 23_000))
    );
    assert!(!recorder.is_open());
}

/// Retake: the camera back on — after asking, from ten seconds.
#[test]
fn retake_turns_the_camera_back_on() {
    let mut recorder = reviewing_for(5_000);
    assert_eq!(
        vec![DiscardClip, OpenCamera],
        actions(&recorder.retake(9_000))
    );
    assert_eq!(RecorderStage::Opening, recorder.stage());

    let mut asked = reviewing_for(12_000);
    assert_eq!(vec![Ask], actions(&asked.retake(20_000)));
    assert_eq!(RecorderQuestion::Retake, asked.question());
    assert_eq!(
        vec![DiscardClip, OpenCamera],
        actions(&asked.answer(true, 21_000))
    );
    assert!(previewing().retake(10).is_empty());
}

/// Esc in REVIEW always asks, however short the clip; and does nothing while a question is up.
#[test]
fn escape_in_review_always_asks() {
    let mut recorder = reviewing_for(2_000);
    assert_eq!(vec![Ask], actions(&recorder.escape(9_000)));
    assert_eq!(RecorderQuestion::Escape, recorder.question());
    assert!(recorder.escape(9_100).is_empty());
    assert_eq!(
        vec![DiscardClip, Closed],
        actions(&recorder.answer(true, 9_200))
    );
}

// ---- interruptions (S4) -----------------------------------------------------------------------

/// A call, a hidden window or a lock: PREVIEW closes; a take stops into REVIEW; REVIEW is kept.
#[test]
fn an_interruption_closes_preview_stops_a_take_and_keeps_review() {
    for why in [
        RecorderInterruption::Call,
        RecorderInterruption::WindowHidden,
        RecorderInterruption::SessionLocked,
    ] {
        let mut preview = previewing();
        assert_eq!(
            vec![CloseCamera, Closed],
            actions(&preview.interrupt(why, 100)),
            "{why:?}"
        );

        let mut recording = recording_at(1_000);
        let stopped = recording.interrupt(why, 9_000);
        assert_eq!(vec![StopAndKeep, CloseCamera], actions(&stopped));
        assert!(!actions(&stopped).contains(&Send));
        assert_eq!(RecorderStage::Review, recording.stage());
        assert_eq!(8_000, recording.clip_ms());

        let mut review = reviewing();
        assert!(review.interrupt(why, 20_000).is_empty());
        assert_eq!(RecorderStage::Review, review.stage());
    }
}

/// Under a second, an interrupted take is deleted instead: there is nothing worth keeping.
#[test]
fn an_interrupted_take_under_a_second_is_deleted() {
    let mut recorder = recording_at(1_000);
    assert_eq!(
        vec![StopAndDiscard, CloseCamera, Closed],
        actions(&recorder.interrupt(RecorderInterruption::Call, 1_500))
    );
    assert!(!recorder.is_open());
}

/// Focus lost to another window: PREVIEW closes — not to a prompt it raised — and a take records
/// on.
#[test]
fn losing_focus_closes_preview_but_not_a_prompt_or_a_take() {
    assert_eq!(
        vec![CloseCamera, Closed],
        actions(&previewing().interrupt(RecorderInterruption::FocusLost, 10))
    );

    let mut asking = recorder();
    asking.open(0, None, true);
    assert!(
        asking
            .interrupt(RecorderInterruption::FocusLost, 10)
            .is_empty()
    );
    assert!(asking.is_open());

    let mut recording = recording_at(1_000);
    assert!(
        recording
            .interrupt(RecorderInterruption::FocusLost, 5_000)
            .is_empty()
    );
    assert_eq!(RecorderStage::Recording, recording.stage());
}

/// A real close over a take stops it into REVIEW and asks; Delete closes the window, Keep leaves
/// everything.
#[test]
fn a_really_closing_window_asks_over_a_clip() {
    let mut recording = recording_at(1_000);
    assert_eq!(
        vec![StopAndKeep, CloseCamera, Ask],
        actions(&recording.interrupt(RecorderInterruption::WindowClosing, 9_000))
    );
    assert_eq!(RecorderQuestion::CloseWindow, recording.question());
    assert_eq!(
        vec![DiscardClip, CloseWindow, Closed],
        actions(&recording.answer(true, 10_000))
    );

    let mut review = reviewing();
    assert_eq!(
        vec![Ask],
        actions(&review.interrupt(RecorderInterruption::WindowClosing, 20_000))
    );
    // A second close while asked: the one question.
    assert!(
        review
            .interrupt(RecorderInterruption::WindowClosing, 20_100)
            .is_empty()
    );
    assert!(review.answer(false, 21_000).is_empty());
    assert_eq!(RecorderStage::Review, review.stage());

    let mut other_question = reviewing_for(12_000);
    other_question.delete(20_000);
    assert_eq!(
        vec![Dismiss, Ask],
        actions(&other_question.interrupt(RecorderInterruption::WindowClosing, 20_100))
    );
    assert_eq!(RecorderQuestion::CloseWindow, other_question.question());

    assert_eq!(
        vec![CloseCamera, Closed],
        actions(&previewing().interrupt(RecorderInterruption::WindowClosing, 5))
    );
}

/// A take stopped to ask about deleting it, then interrupted: kept, the question taken away, the
/// light out.
#[test]
fn an_interruption_during_the_delete_question_keeps_the_take() {
    let mut recorder = recording_at(1_000);
    recorder.delete(12_000);
    assert_eq!(
        vec![Dismiss, CloseCamera],
        actions(&recorder.interrupt(RecorderInterruption::Call, 12_500))
    );
    assert_eq!(RecorderQuestion::None, recorder.question());
    assert_eq!(RecorderStage::Review, recorder.stage());

    let mut closing = recording_at(1_000);
    closing.delete(12_000);
    assert_eq!(
        vec![Dismiss, CloseCamera, Ask],
        actions(&closing.interrupt(RecorderInterruption::WindowClosing, 12_500))
    );
    assert_eq!(RecorderQuestion::CloseWindow, closing.question());

    let mut unfocused = recording_at(1_000);
    unfocused.delete(12_000);
    assert!(
        unfocused
            .interrupt(RecorderInterruption::FocusLost, 12_500)
            .is_empty()
    );
    assert_eq!(RecorderQuestion::DeleteWhileRecording, unfocused.question());
}

/// A sign-out deletes everything recorded and not sent, whatever state it is in.
#[test]
fn a_sign_out_deletes_everything() {
    assert_eq!(
        vec![StopAndDiscard, CloseCamera, Closed],
        actions(&recording_at(1_000).interrupt(RecorderInterruption::SignedOut, 30_000))
    );
    assert_eq!(
        vec![DiscardClip, Closed],
        actions(&reviewing().interrupt(RecorderInterruption::SignedOut, 30_000))
    );
    let mut asked = reviewing_for(12_000);
    asked.escape(20_000);
    assert_eq!(
        vec![Dismiss, DiscardClip, Closed],
        actions(&asked.interrupt(RecorderInterruption::SignedOut, 30_000))
    );
    assert!(
        recorder()
            .interrupt(RecorderInterruption::SignedOut, 0)
            .is_empty()
    );
}

/// Opened again after closing: everything from before is forgotten.
#[test]
fn reopening_starts_afresh() {
    let mut recorder = reviewing();
    recorder.slot(50_000);
    assert!(!recorder.is_open());
    recorder.open(60_000, None, false);
    assert_eq!(RecorderStage::Opening, recorder.stage());
    assert!(!recorder.clip_ready());
    assert_eq!(None, recorder.plan());
    assert_eq!(0, recorder.clip_ms());
    assert!(!recorder.has_picture());
}
