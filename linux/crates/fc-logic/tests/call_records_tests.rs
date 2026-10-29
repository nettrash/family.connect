//! `CallRecordsTests`: a call record in a bubble — which is a missed call, and when and how it calls
//! back.

use fc_core::protocol::CallRecordDto;
use fc_logic::call_records::{self, CallBack};

/// `new CallRecordDto(outcome, Video: video)`.
fn record(outcome: &str, video: bool) -> CallRecordDto {
    CallRecordDto {
        outcome: outcome.to_string(),
        duration_secs: None,
        video,
    }
}

#[test]
fn only_a_call_the_reader_missed_is_marked() {
    assert!(call_records::is_missed(&record("missed", false), false));
    assert!(!call_records::is_missed(&record("missed", false), true));
    assert!(!call_records::is_missed(&record("declined", false), false));
    assert!(!call_records::is_missed(&record("completed", false), false));
    assert!(!call_records::is_missed(&record("failed", false), false));
}

/// In a direct chat, on a server with calls, and not on a thread's panel — each of the three alone
/// refuses it.
#[test]
fn call_back_is_offered_only_where_a_call_can_be_placed() {
    let voice = record("missed", false);
    assert!(call_records::call_back(&voice, true, true, true, false).offered);
    assert!(!call_records::call_back(&voice, false, true, true, false).offered);
    assert!(!call_records::call_back(&voice, true, false, true, false).offered);
    assert!(!call_records::call_back(&voice, true, true, true, true).offered);
}

/// A video record calls back with video while the server carries it, and with voice once it does
/// not; a voice record never with video.
#[test]
fn a_video_record_calls_back_with_video_only_where_video_is_carried() {
    let both = |offered, video| CallBack { offered, video };
    assert_eq!(
        call_records::call_back(&record("completed", true), true, true, true, false),
        both(true, true)
    );
    assert_eq!(
        call_records::call_back(&record("completed", true), true, true, false, false),
        both(true, false)
    );
    assert_eq!(
        call_records::call_back(&record("completed", false), true, true, true, false),
        both(true, false)
    );
}

/// The call buttons are off while something records (docs/audio-video-messages-2026-10-04.md,
/// S1.7, decision 14) — the toolbar's and every record's "Call back" alike — and while a call is
/// on, as they always were.
#[test]
fn no_call_is_placed_over_a_recording_or_beside_a_call() {
    assert!(call_records::can_place_call(false, false));
    assert!(!call_records::can_place_call(false, true));
    assert!(!call_records::can_place_call(true, false));
    assert!(!call_records::can_place_call(true, true));
}
