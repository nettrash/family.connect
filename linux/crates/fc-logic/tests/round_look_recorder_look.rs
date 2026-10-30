//! `RecorderLookTests`: the video-message recorder's one big slot (the approved design): Record →
//! Stop → Send, captioned, 64 across.

mod common;

use fc_logic::round_look::SlotShape;
use fc_logic::round_look::recorder_look;
use fc_logic::round_recorder::RecorderStage;
use fc_text::record::MIN_TARGET_WINDOWS_EPX;

/// A `[Theory]`: one test over its six rows.
#[test]
fn the_slot_goes_record_stop_send() {
    let say = common::english();
    let rows = [
        (
            RecorderStage::Opening,
            SlotShape::RecordDisc,
            "Record",
            false,
        ),
        (
            RecorderStage::Refused,
            SlotShape::RecordDisc,
            "Record",
            false,
        ),
        (
            RecorderStage::Unavailable,
            SlotShape::RecordDisc,
            "Record",
            false,
        ),
        (
            RecorderStage::Preview,
            SlotShape::RecordDisc,
            "Record",
            true,
        ),
        (
            RecorderStage::Recording,
            SlotShape::StopSquare,
            "Stop",
            true,
        ),
        (RecorderStage::Review, SlotShape::SendArrow, "Send", true),
    ];
    for (stage, shape, caption, track) in rows {
        assert_eq!(shape, recorder_look::shape(stage), "{stage:?}");
        assert_eq!(caption, recorder_look::caption(stage, &say), "{stage:?}");
        assert_eq!(track, recorder_look::shows_track(stage), "{stage:?}");
    }
}

#[test]
fn the_slot_is_the_big_button_and_the_others_are_smaller() {
    const { assert!(recorder_look::SLOT >= 64.0) };
    const { assert!(recorder_look::SIDE >= MIN_TARGET_WINDOWS_EPX as f64) };
    const { assert!(recorder_look::SIDE < recorder_look::SLOT) };
    const { assert!(recorder_look::STOP_SQUARE < recorder_look::SLOT / 2.0) };
}

/// The slot's target is the disc and its halo. (Until 2026-10-06 its column was also lifted so the
/// slot sat centred on Send's row; the controls now stand on their own bar — decision 41 — with the
/// slot under Send's centre across, and `recorder_frame_tests.rs` holds where it stands.)
#[test]
fn the_slots_target_is_its_disc_and_halo() {
    assert_eq!(
        recorder_look::SLOT + 2.0 * recorder_look::HALO,
        recorder_look::SLOT_TARGET
    );
}
