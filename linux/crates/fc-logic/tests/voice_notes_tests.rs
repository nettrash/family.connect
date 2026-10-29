//! `VoiceNotesTests`: a voice note — five minutes at most, nothing when it is too short, and the
//! lines it is drawn with.

mod common;

use std::time::Duration;

use fc_logic::media_outbox::StagedMedia;
use fc_logic::voice_notes::{self, CapabilityAccess, QuietWhileRecording, RecordingStart};

fn seconds(value: f64) -> Duration {
    Duration::from_secs_f64(value)
}

/// The S1.1 constants as the oracle printed them (`record-vectors.json`, the "constants" case).
fn constants() -> serde_json::Value {
    let vectors = common::fixture_json("record-vectors.json");
    let rows = vectors.as_array().expect("an array of cases");
    let found: Vec<&serde_json::Value> = rows
        .iter()
        .filter(|row| row["function"] == "constants")
        .collect();
    assert_eq!(1, found.len());
    found[0]["expected"].clone()
}

#[test]
fn a_recording_is_done_at_five_minutes() {
    assert!(!voice_notes::is_done(seconds(299.9)));
    assert!(voice_notes::is_done(Duration::from_secs(5 * 60)));
    assert!(voice_notes::is_done(Duration::from_secs(6 * 60)));
}

/// 1024 bytes or less is nothing; past it, an MP4 audio attachment with no name, its duration its
/// identity.
#[test]
fn a_recording_is_staged_unless_it_is_nothing() {
    assert_eq!(
        None,
        voice_notes::staged(vec![0; 1024], Duration::from_secs(3))
    );
    assert_eq!(
        None,
        voice_notes::staged(Vec::new(), Duration::from_secs(3))
    );

    // Half a millisecond rounds away from zero, as the Apple apps' does — not to the even neighbour.
    let staged = voice_notes::staged(vec![0; 1025], Duration::from_micros(12_344_500)).unwrap();
    assert_eq!(
        ("audio", "audio/mp4", 1025, Some(12_345)),
        (
            staged.kind.as_str(),
            staged.mime.as_str(),
            staged.bytes.len(),
            staged.duration_ms
        )
    );
    assert_eq!(None, staged.name);
    assert_eq!(None, staged.preview);

    // A recording that ran over by a tick is still five minutes long.
    assert_eq!(
        Some(300_000),
        voice_notes::staged(vec![0; 4096], seconds(300.4))
            .unwrap()
            .duration_ms
    );
}

/// A second is the floor people see (S1.1): under it nothing is staged, however many bytes it
/// wrote.
#[test]
fn a_recording_under_a_second_is_nothing() {
    assert_eq!(
        None,
        voice_notes::staged(vec![0; 64 * 1024], Duration::from_millis(999))
    );
    assert_eq!(
        None,
        voice_notes::staged(vec![0; 64 * 1024], Duration::ZERO)
    );
    assert_eq!(
        Some(1000),
        voice_notes::staged(vec![0; 64 * 1024], Duration::from_secs(1))
            .unwrap()
            .duration_ms
    );
}

/// A recording is the audio with no name; a sound file picked from disk is audio WITH its name, and
/// stays a file.
#[test]
fn only_a_recording_made_here_is_a_recording() {
    assert!(voice_notes::is_recorded(
        &voice_notes::staged(vec![0; 2048], Duration::from_secs(3)).unwrap()
    ));
    assert!(!voice_notes::is_recorded(&StagedMedia {
        duration_ms: Some(3000),
        name: Some("song.m4a".to_string()),
        ..StagedMedia::new("audio", "audio/mp4", vec![0; 2048])
    }));
    assert!(!voice_notes::is_recorded(&StagedMedia::new(
        "photo",
        "image/jpeg",
        vec![0; 2048]
    )));
    assert!(!voice_notes::is_recorded(&StagedMedia::new(
        "file",
        "application/pdf",
        vec![0; 2048]
    )));
}

/// Deleting ten seconds or more asks first (S1.1, S2.8): a recording cannot be made again.
#[test]
fn deleting_ten_seconds_or_more_asks_first() {
    assert!(!voice_notes::asks_before_deleting(0));
    assert!(!voice_notes::asks_before_deleting(9_999));
    assert!(voice_notes::asks_before_deleting(10_000));
    assert!(voice_notes::asks_before_deleting(300_000));
}

/// A recording refused says why, in the order the Send slot's rows put the reasons (S1.3: editing,
/// a call, an attachment on its way, a voice message not sent) — and only then the strip's cap.
#[test]
fn a_recording_is_refused_for_the_slots_reasons_in_the_slots_order() {
    let say = common::english();
    assert_eq!(
        None,
        voice_notes::refusal(false, false, false, false, false, &say)
    );
    assert_eq!(
        Some("Finish editing before attaching something.".to_string()),
        voice_notes::refusal(true, true, true, true, true, &say)
    );
    assert_eq!(
        Some("You can record a message after the call.".to_string()),
        voice_notes::refusal(false, true, true, true, true, &say)
    );
    assert_eq!(
        Some("Wait until the current attachment is done.".to_string()),
        voice_notes::refusal(false, false, true, true, true, &say)
    );
    assert_eq!(
        Some("Send or delete the voice message that wasn't sent first.".to_string()),
        voice_notes::refusal(false, false, false, true, true, &say)
    );
    assert_eq!(
        Some("You can attach up to 10 items.".to_string()),
        voice_notes::refusal(false, false, false, false, true, &say)
    );
}

/// Settings is offered only where a switch there can help (S2.2) — the person's, the system's own,
/// or when the platform could not say — never for a fault in the build, and a prompt closed without
/// an answer says nothing at all.
///
/// ADAPTED: the C#'s last line pins Windows' page, `ms-settings:privacy-microphone`; on Linux the
/// place is GNOME Settings' "microphone" panel (`MICROPHONE_SETTINGS_PANEL`).
#[test]
fn a_microphone_refusal_offers_settings_only_where_a_switch_there_can_help() {
    let say = common::english();
    let permission =
        Some("Family needs permission to use your microphone. Turn it on in Settings.".to_string());
    assert_eq!(
        (permission.clone(), true),
        voice_notes::microphone_refusal(CapabilityAccess::DeniedByUser, &say)
    );
    assert_eq!(
        (permission.clone(), true),
        voice_notes::microphone_refusal(CapabilityAccess::DeniedBySystem, &say)
    );
    assert_eq!(
        (permission, true),
        voice_notes::microphone_refusal(CapabilityAccess::Unknown, &say)
    );
    assert_eq!(
        (None, false),
        voice_notes::microphone_refusal(CapabilityAccess::UserPromptRequired, &say)
    );
    assert_eq!(
        (Some("Couldn't start recording.".to_string()), false),
        voice_notes::microphone_refusal(CapabilityAccess::NotDeclaredByApp, &say)
    );
    assert_eq!(
        (Some("Couldn't start recording.".to_string()), false),
        voice_notes::microphone_refusal(CapabilityAccess::Allowed, &say)
    );
    assert_eq!("microphone", voice_notes::MICROPHONE_SETTINGS_PANEL);
}

/// The floors are the shared rules' (`fc_text::record`, S1.1), read from the oracle's own vectors
/// rather than copied: a second, ten seconds to ask, five minutes.
#[test]
fn the_floors_are_the_shared_rules() {
    let constants = constants();
    assert_eq!(
        constants["shortest_recording_ms"].as_u64().unwrap(),
        voice_notes::SHORTEST.as_millis() as u64
    );
    assert_eq!(
        constants["delete_asks_from_ms"].as_u64().unwrap(),
        voice_notes::DELETE_ASKS_FROM.as_millis() as u64
    );
    assert_eq!(
        constants["voice_cap_ms"].as_u64().unwrap(),
        voice_notes::LONGEST.as_millis() as u64
    );
}

/// The recording row's clock is "0:42" in whole seconds gone by, as a stopwatch counts (S2.4, S2.9)
/// — 0:12 until the 13th second has run, and 5:00 only at five minutes, where the row stops.
///
/// The C#'s last case, a NEGATIVE `TimeSpan` reading "0:00", cannot be written: a
/// `std::time::Duration` is never negative (the caller clamps).
#[test]
fn the_clock_counts_whole_seconds_gone_by() {
    assert_eq!("0:00", voice_notes::clock(Duration::ZERO));
    assert_eq!("0:00", voice_notes::clock(Duration::from_millis(999)));
    assert_eq!("0:12", voice_notes::clock(seconds(12.4)));
    assert_eq!("0:12", voice_notes::clock(seconds(12.9)));
    assert_eq!("0:13", voice_notes::clock(Duration::from_secs(13)));
    assert_eq!("4:59", voice_notes::clock(seconds(299.9)));
    assert_eq!("5:00", voice_notes::clock(Duration::from_secs(5 * 60)));
}

/// At 4:30 the clock turns orange and "30 seconds left" is shown and announced (S2.5) — the shared
/// rule's own number, read from its vectors.
#[test]
fn thirty_seconds_left_is_said_at_four_thirty() {
    assert!(!voice_notes::warns(seconds(269.9)));
    assert!(voice_notes::warns(Duration::from_secs(270)));
    assert!(voice_notes::warns(voice_notes::LONGEST));
    assert_eq!(
        constants()["voice_warning_ms"].as_u64().unwrap(),
        voice_notes::WARN_FROM.as_millis() as u64
    );
}

/// With a screen reader running the microphone opens a second after "Recording" is announced (S6:
/// a fixed second where the platform cannot say when its speech ended), so the app's own voice does
/// not open the note; otherwise at once.
#[test]
fn the_microphone_waits_for_the_screen_reader_to_speak() {
    assert_eq!(Duration::from_secs(1), voice_notes::lead_in(true));
    assert_eq!(Duration::ZERO, voice_notes::lead_in(false));
}

/// A staged or not-sent note while it plays: "0:12 / 0:42" (S2.7) — never past its own length.
#[test]
fn a_playing_note_says_where_it_is_and_how_long_it_is() {
    assert_eq!("0:12 / 0:42", voice_notes::playing_label(12.2, 42.0));
    assert_eq!("0:00 / 0:42", voice_notes::playing_label(0.0, 42.0));
    assert_eq!("0:42 / 0:42", voice_notes::playing_label(43.5, 42.0));
    assert_eq!("1:05 / 5:00", voice_notes::playing_label(65.0, 300.0));
}

/// The scrubber's length comes from the attachment, and is never zero.
#[test]
fn a_recording_is_as_long_as_its_attachment_says() {
    assert_eq!(65.0, voice_notes::total_seconds(Some(65_000)));
    assert_eq!(1.5, voice_notes::total_seconds(Some(1_500)));
    assert_eq!(0.1, voice_notes::total_seconds(None));
    assert_eq!(0.1, voice_notes::total_seconds(Some(0)));
    assert_eq!(0.1, voice_notes::total_seconds(Some(40)));
}

#[test]
fn play_starts_again_only_at_the_end() {
    assert!(!voice_notes::replays_from_start(0.0, 65.0));
    assert!(!voice_notes::replays_from_start(64.7, 65.0));
    assert!(voice_notes::replays_from_start(64.8, 65.0));
    assert!(voice_notes::replays_from_start(65.0, 65.0));
    assert!(voice_notes::replays_from_start(66.0, 65.0));
}

/// THE APP'S OWN SPEECH STAYS OUT OF THE NOTE (S6): while a recording runs, a sentence on the
/// composer's notice line — a play button saying "You can play this after recording." — is shown
/// and never said; otherwise it is said as it appears.
#[test]
fn nothing_on_the_notice_line_is_said_while_a_recording_runs() {
    assert!(!voice_notes::notice_said(true));
    assert!(voice_notes::notice_said(false));
}

/// NO APP SOUND WHILE RECORDING (S1.7), the viewer's video included: muted for as long as a
/// recording runs, and left as it was once it ends — muted if the person had muted it, not if they
/// had not — however often it is asked meanwhile.
#[test]
fn the_viewers_video_is_muted_while_recording_and_given_back_as_it_was() {
    let mut quiet = QuietWhileRecording::new();
    assert!(quiet.muted(true, false));
    // A video opened in the viewer mid-recording: muted already, by this — which is not what it was
    // before.
    assert!(quiet.muted(true, true));
    assert!(!quiet.muted(false, true));
    // The person had muted it before the recording: muted it stays.
    assert!(quiet.muted(true, true));
    assert!(quiet.muted(false, true));
    // Ended twice, or with nothing recorded meanwhile: nothing is changed.
    assert!(!quiet.muted(false, false));
    assert!(quiet.muted(false, true));
}

/// NO APP SOUND WHILE RECORDING (S1.7) STARTS WITH THE START: the microphone being opened and a
/// screen reader's one-second lead-in both come before the recorder exists, and a play pressed then
/// would run on into the note. Playback is refused from the moment a start begins, for as long as it
/// is under way or the recording runs — and not before or after.
#[test]
fn nothing_plays_while_a_recording_is_starting() {
    let start = RecordingStart::new();
    assert!(!start.starting());
    assert!(!start.quiet(false));
    assert!(start.quiet(true));
    start.begin();
    assert!(start.starting());
    assert!(start.quiet(false));
    start.end();
    assert!(!start.starting());
    assert!(!start.quiet(false));
    assert!(start.quiet(true));
}

/// AN INTERRUPTION DURING THE START IS NOT LOST (S4): a lock, the computer sleeping, a call, the
/// window hidden or closed, leaving the chat, a sign-out — any of them arriving while the
/// microphone opens, or during the lead-in, finds no recorder to stop, so the start remembers it and
/// lets go of the microphone once it is granted. One heard with no start under way, or during an
/// earlier start, says nothing about the next.
#[test]
fn an_interruption_during_the_start_lets_go_of_the_microphone() {
    let start = RecordingStart::new();
    start.interrupted();
    start.begin();
    assert!(!start.was_interrupted());
    start.interrupted();
    assert!(start.was_interrupted());
    assert!(start.starting());
    start.end();
    assert!(!start.was_interrupted());
    start.begin();
    assert!(!start.was_interrupted());
    start.end();
    start.interrupted();
    start.begin();
    assert!(!start.was_interrupted());
}
