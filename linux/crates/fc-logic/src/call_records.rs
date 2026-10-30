//! A call record in a bubble (`CallRecords.cs`; ios `CallRecordView`, web `views/bubble.rs`;
//! docs/protocol.md, "The record"): how it is marked, and whether it calls back. What it SAYS is
//! `fc_text::call_record`.

use fc_core::protocol::CallRecordDto;

/// A call the READER missed — somebody else's, never answered — is drawn in red. The caller's own
/// unanswered call is not.
pub fn is_missed(call: &CallRecordDto, mine: bool) -> bool {
    call.outcome == "missed" && !mine
}

/// Whether a record offers "Call back" (`offered`), and whether the call it places has video.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CallBack {
    pub offered: bool,
    pub video: bool,
}

/// Whether a record offers "Call back", and what kind of call it places. Calling back is what a
/// record is FOR, half the time: offered in a direct chat — a call lives in one — while the server
/// carries calls, and never on a thread's panel. A video record calls back with video only where
/// the server still carries video; otherwise it is a voice call.
pub fn call_back(
    call: &CallRecordDto,
    direct_chat: bool,
    calls_enabled: bool,
    video_calls_enabled: bool,
    in_thread: bool,
) -> CallBack {
    CallBack {
        offered: direct_chat && calls_enabled && !in_thread,
        video: call.video && video_calls_enabled,
    }
}

/// Whether a call can be placed from the window now — the toolbar's Call and Video Call and every
/// record's "Call back" alike: never while a call is on, and never while something records
/// (docs/audio-video-messages-2026-10-04.md, S1.7 and decision 14: a call over a recording has no
/// right answer).
pub fn can_place_call(call_busy: bool, recording: bool) -> bool {
    !call_busy && !recording
}
