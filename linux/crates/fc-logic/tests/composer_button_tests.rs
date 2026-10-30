//! `ComposerButtonTests`: THE SEND SLOT, THE VIDEO BUTTON AND THE ROUND VIDEO'S ARITHMETIC, held to
//! `fc_text::record` case for case (docs/audio-video-messages-2026-10-04.md, S1.3, S1.4, S5; issue
//! #79). Every case in `fixtures/record-vectors.json` (a byte-for-byte copy of
//! `win/tests/FamilyConnect.Core.Tests/Fixtures/record-vectors.json`) was printed by the Rust the
//! web client runs; the Linux port CALLS that Rust (`composer_button` re-exports it), so these cases
//! hold its re-exports and its own wording — the translated label, notice and door label — to the
//! file. Every function in it is read but the voice recording's reducer, `hold_step`, which the
//! window drives itself (S8.6); a function added to the original and not read here fails
//! `the_file_is_the_one_the_original_printed`.
//!
//! And then what is the desktop's own and so not in the vectors: how each row is drawn and named
//! (`face`, an icon where Windows had a glyph), which press opens the microphone's menu (`respond`,
//! GTK's secondary click where Windows had `RightTapped`), the 600 ms guard and the press reaching
//! the slot (`SlotGuard`), and Ctrl+Shift+R held down (`ShortcutPresses`).

mod common;

use std::collections::BTreeSet;

use fc_core::protocol::{AttachmentDto, MessageDto};
use fc_logic::composer_button::{
    self, AttachmentFlags, Dimmed, Door, DoorInputs, Recording, ShortcutPresses, Slot, SlotDevice,
    SlotFace, SlotGuard, SlotIcon, SlotInputs, SlotPress, SlotResponse, WidthClass,
};
use fc_logic::voice_notes;
use serde_json::Value;

fn cases() -> Vec<Value> {
    common::fixture_json("record-vectors.json")
        .as_array()
        .expect("an array of cases")
        .clone()
}

fn of(cases: &[Value], function: &str) -> Vec<Value> {
    cases
        .iter()
        .filter(|row| row["function"] == function)
        .cloned()
        .collect()
}

fn optional_string(row: &Value, name: &str) -> Option<String> {
    row[name].as_str().map(str::to_string)
}

fn flag(input: &Value, name: &str) -> bool {
    input[name]
        .as_bool()
        .unwrap_or_else(|| panic!("{name} is a bool"))
}

fn recording_of(spelled: &str) -> Recording {
    match spelled {
        "none" => Recording::None,
        "hands_free" => Recording::HandsFree,
        "hands_free_beside_draft" => Recording::HandsFreeBesideDraft,
        other => panic!("a recording spelled {other}"),
    }
}

fn slot_of(input: &Value) -> SlotInputs {
    SlotInputs {
        recorder_open: flag(input, "recorder_open"),
        recording: recording_of(input["recording"].as_str().unwrap()),
        editing: flag(input, "editing"),
        draft_blank: flag(input, "draft_blank"),
        staged: flag(input, "staged"),
        assistant_chat: flag(input, "assistant_chat"),
        can_record: flag(input, "can_record"),
        call: flag(input, "call"),
        busy: flag(input, "busy"),
        not_sent: flag(input, "not_sent"),
    }
}

fn door_of(input: &Value) -> DoorInputs {
    DoorInputs {
        slot: slot_of(&input["slot"]),
        family_or_direct_chat: flag(input, "family_or_direct_chat"),
        server_offers_round: flag(input, "server_offers_round"),
        has_camera: flag(input, "has_camera"),
        encoder_probe_passes: flag(input, "encoder_probe_passes"),
        records_round_video: flag(input, "records_round_video"),
    }
}

/// The slot as the vectors spell it: its kind in snake case.
fn spelled_slot(slot: Slot) -> &'static str {
    match slot {
        Slot::Recorder => "recorder",
        Slot::SendVoice => "send_voice",
        Slot::StopRecording => "stop_recording",
        Slot::Save { .. } => "save",
        Slot::Send => "send",
        Slot::SendDisabled => "send_disabled",
        Slot::Dimmed(_) => "dimmed",
        Slot::Microphone => "microphone",
    }
}

fn spelled_reason(reason: Option<Dimmed>) -> Option<String> {
    reason.map(|reason| {
        match reason {
            Dimmed::Call => "call",
            Dimmed::Busy => "busy",
            Dimmed::NotSent => "not_sent",
        }
        .to_string()
    })
}

fn spelled_door(door: Door) -> &'static str {
    match door {
        Door::Hidden => "hidden",
        Door::Dimmed(_) => "dimmed",
        Door::Shown => "shown",
    }
}

#[test]
fn the_file_is_the_one_the_original_printed() {
    let cases = cases();
    // 218 when the hold was withdrawn (2026-10-06); fewer would be a truncated copy.
    assert!(cases.len() >= 218, "only {} cases", cases.len());
    let read = [
        "constants",
        "composer_slot",
        "video_door",
        "round_cap_ms",
        "round_warning_ms",
        "round_diameter",
        "is_round",
    ];
    // The voice recording's reducer (which keeps its first name) is driven by the window itself
    // (S8.6), and nothing else is left out. The hold threshold went with the hold (2026-10-06): a
    // file that still names it is stale.
    let not_here = ["hold_step"];
    let named: BTreeSet<String> = cases
        .iter()
        .map(|row| row["function"].as_str().unwrap().to_string())
        .collect();
    let expected: BTreeSet<String> = read
        .iter()
        .chain(not_here.iter())
        .map(|function| function.to_string())
        .collect();
    assert_eq!(expected, named);
    for function in read {
        assert!(!of(&cases, function).is_empty(), "{function}");
    }
}

/// S1.1's numbers by value, read from the original rather than copied: a changed one is a changed
/// rule.
#[test]
fn the_constants_are_the_originals() {
    let cases = cases();
    let constants = of(&cases, "constants");
    assert_eq!(1, constants.len());
    let constants = &constants[0]["expected"];
    let number = |name: &str| constants[name].as_u64().unwrap();
    assert_eq!(
        composer_button::ACTIVATION_GUARD_MS,
        number("activation_guard_ms")
    );
    assert_eq!(
        composer_button::SHORTEST_RECORDING_MS,
        number("shortest_recording_ms")
    );
    assert_eq!(composer_button::VOICE_CAP_MS, number("voice_cap_ms"));
    assert_eq!(
        composer_button::VOICE_WARNING_MS,
        number("voice_warning_ms")
    );
    assert_eq!(
        composer_button::DELETE_ASKS_FROM_MS,
        number("delete_asks_from_ms")
    );
    assert_eq!(
        composer_button::SLOT_CROSSFADE_MS,
        number("slot_crossfade_ms")
    );
    assert_eq!(
        u64::from(composer_button::MIN_TARGET_WINDOWS_EPX),
        number("min_target_windows_epx")
    );
    assert_eq!(
        composer_button::DEFAULT_MAX_ROUND_VIDEO_MS,
        number("default_max_round_video_ms")
    );
    assert_eq!(
        composer_button::ROUND_CAP_MARGIN_MS,
        number("round_cap_margin_ms")
    );
    assert_eq!(
        composer_button::ROUND_WARNING_LEAD_MS,
        number("round_warning_lead_ms")
    );
    assert_eq!(
        u64::from(composer_button::ROUND_DIAMETER_COMPACT),
        number("round_diameter_compact")
    );
    assert_eq!(
        u64::from(composer_button::ROUND_DIAMETER_REGULAR),
        number("round_diameter_regular")
    );
    assert_eq!(
        Some(composer_button::VIDEO_DOOR_LABEL),
        constants["video_door_label"].as_str()
    );
    assert_eq!(
        Some(composer_button::VIDEO_DOOR_TOOLTIP),
        constants["video_door_tooltip"].as_str()
    );
    // And the voice note's own numbers, which the recorder and the review chip read, are the same
    // ones.
    assert_eq!(
        composer_button::VOICE_CAP_MS,
        voice_notes::LONGEST.as_millis() as u64
    );
    assert_eq!(
        composer_button::VOICE_WARNING_MS,
        voice_notes::WARN_FROM.as_millis() as u64
    );
    assert_eq!(
        composer_button::SHORTEST_RECORDING_MS,
        voice_notes::SHORTEST.as_millis() as u64
    );
    assert_eq!(
        composer_button::DELETE_ASKS_FROM_MS,
        voice_notes::DELETE_ASKS_FROM.as_millis() as u64
    );
    // The visual is 40 in a target that meets the 44 (S8.6).
    const { assert!(composer_button::SLOT_VISUAL_PX < composer_button::MIN_TARGET_WINDOWS_EPX) };
}

/// Every slot case: the row, the kind, Save's switch, a dimmed microphone's reason, the label and
/// what activation says. One test for all of them, so the assertion names the case that disagreed.
#[test]
fn every_slot_is_the_row_the_original_chooses() {
    let say = common::english();
    let mut checked = 0;
    for row in of(&cases(), "composer_slot") {
        let name = row["name"].as_str().unwrap().to_string();
        let expected = &row["expected"];
        let slot = composer_button::composer_slot(&slot_of(&row["input"]));
        assert_eq!(
            expected["row"].as_u64().unwrap(),
            u64::from(slot.row()),
            "{name}: row"
        );
        assert_eq!(
            expected["slot"].as_str().unwrap(),
            spelled_slot(slot),
            "{name}: kind"
        );
        assert_eq!(
            expected["enabled"].as_bool(),
            composer_button::enabled(slot),
            "{name}: enabled"
        );
        assert_eq!(
            optional_string(expected, "reason"),
            spelled_reason(composer_button::reason(slot)),
            "{name}: reason"
        );
        assert_eq!(
            optional_string(expected, "label"),
            composer_button::label(slot, &say),
            "{name}: label"
        );
        assert_eq!(
            optional_string(expected, "notice"),
            composer_button::notice(composer_button::reason(slot), &say),
            "{name}: notice"
        );
        assert_eq!(
            slot.is_microphone(),
            (7..=10).contains(&slot.row()),
            "{name}: microphone"
        );
        checked += 1;
    }
    // 58 until 2026-10-06, when the two held-microphone cases went with the hold and one hands-free
    // case took their place.
    assert_eq!(57, checked);
}

#[test]
fn every_video_button_is_the_one_the_original_chooses() {
    let say = common::english();
    let mut checked = 0;
    for row in of(&cases(), "video_door") {
        let name = row["name"].as_str().unwrap().to_string();
        let expected = &row["expected"];
        let door = composer_button::video_door(&door_of(&row["input"]));
        assert_eq!(
            expected["door"].as_str().unwrap(),
            spelled_door(door),
            "{name}: kind"
        );
        assert_eq!(
            optional_string(expected, "reason"),
            spelled_reason(composer_button::door_reason(door)),
            "{name}: reason"
        );
        assert_eq!(
            optional_string(expected, "label"),
            composer_button::door_label(door, &say),
            "{name}: label"
        );
        assert_eq!(
            optional_string(expected, "notice"),
            composer_button::notice(composer_button::door_reason(door), &say),
            "{name}: notice"
        );
        checked += 1;
    }
    // 35 until 2026-10-06, when the two Undo-window cases went with the hold.
    assert_eq!(33, checked);
}

/// Decision 40: a build that does not record round video shows no video button — wherever the slot
/// is, every slot case in the file — even on a server that offers it and a device with a camera.
/// (Windows' name for it: no build before Phase 3d.)
#[test]
fn no_windows_build_before_phase3d_shows_a_video_button() {
    for row in of(&cases(), "composer_slot") {
        let door = composer_button::video_door(&DoorInputs {
            slot: slot_of(&row["input"]),
            family_or_direct_chat: true,
            server_offers_round: true,
            has_camera: true,
            encoder_probe_passes: true,
            records_round_video: false,
        });
        assert_eq!(Door::Hidden, door);
    }
}

#[test]
fn the_round_arithmetic_is_the_originals() {
    let cases = cases();
    for row in of(&cases, "round_cap_ms") {
        assert_eq!(
            row["expected"]["cap_ms"].as_u64().unwrap(),
            composer_button::round_cap_ms(row["input"]["max_round_video_ms"].as_u64().unwrap()),
            "{}",
            row["name"]
        );
    }
    for row in of(&cases, "round_warning_ms") {
        assert_eq!(
            row["expected"]["warning_ms"].as_u64().unwrap(),
            composer_button::round_warning_ms(row["input"]["max_round_video_ms"].as_u64().unwrap()),
            "{}",
            row["name"]
        );
    }
    for row in of(&cases, "round_diameter") {
        let width = match row["input"]["width_class"].as_str().unwrap() {
            "compact" => WidthClass::Compact,
            "regular" => WidthClass::Regular,
            other => panic!("a width class spelled {other}"),
        };
        assert_eq!(
            row["expected"]["diameter"].as_u64().unwrap(),
            u64::from(composer_button::round_diameter(width))
        );
    }
    assert_eq!(
        22,
        of(&cases, "round_cap_ms").len() + of(&cases, "round_warning_ms").len()
    );
}

#[test]
fn a_round_video_is_exactly_one_flagged_video_with_no_body() {
    let mut checked = 0;
    for row in of(&cases(), "is_round") {
        let input = &row["input"];
        let name = row["name"].as_str().unwrap().to_string();
        let items: Vec<(String, bool)> = input["attachments"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| {
                (
                    item["kind"].as_str().unwrap().to_string(),
                    item["round"].as_bool().unwrap(),
                )
            })
            .collect();
        let attachments: Vec<AttachmentFlags<'_>> = items
            .iter()
            .map(|(kind, round)| AttachmentFlags {
                kind,
                round: *round,
            })
            .collect();
        let body = input["body"].as_str().unwrap();
        let expected = row["expected"]["round"].as_bool().unwrap();
        assert_eq!(
            expected,
            composer_button::is_round(body, &attachments),
            "{name}"
        );
        // And the test the conversation actually DRAWS by — the message's own — says the same,
        // body and all.
        let media: Vec<AttachmentDto> = items
            .iter()
            .enumerate()
            .map(|(at, (kind, round))| AttachmentDto {
                mime: Some("video/mp4".to_string()),
                size: Some(1),
                round: *round,
                ..AttachmentDto::new(90 + at as i64, kind)
            })
            .collect();
        let message = MessageDto {
            attachments: Some(media),
            ..MessageDto::new(1, 42, 9, None, body, "2026-10-05T10:00:00Z")
        };
        assert_eq!(
            expected,
            message.round_video().is_some(),
            "MessageDto::round_video: {name}"
        );
        checked += 1;
    }
    assert_eq!(12, checked);
}

// ---- what the desktop draws for each row (S8.6) --------------------------------------------------

fn face_of(slot: Slot, send_held: bool) -> SlotFace {
    composer_button::face(slot, send_held, &common::english())
}

fn drawn(
    icon: SlotIcon,
    name: &str,
    tooltip: &str,
    help: &str,
    enabled: bool,
    looks_dimmed: bool,
) -> SlotFace {
    SlotFace {
        icon,
        name: name.to_string(),
        tooltip: tooltip.to_string(),
        help_text: help.to_string(),
        enabled,
        looks_dimmed,
    }
}

/// Send, the microphone, Stop, Save — each named, each with a tooltip. (Windows pinned the Segoe
/// glyphs E724, E720, E71A and E73E; here the icon each names.)
#[test]
fn each_row_has_its_glyph_name_and_tooltip() {
    let record = "Record a voice message (Ctrl+Shift+R)";
    assert_eq!(
        drawn(
            SlotIcon::Send,
            "Send voice message",
            "Send voice message",
            "",
            true,
            false
        ),
        face_of(Slot::SendVoice, false)
    );
    assert_eq!(
        drawn(
            SlotIcon::Stop,
            "Stop recording",
            "Stop recording",
            "",
            true,
            false
        ),
        face_of(Slot::StopRecording, false)
    );
    assert_eq!(
        drawn(SlotIcon::Save, "Save", "Save", "", true, false),
        face_of(Slot::Save { enabled: true }, false)
    );
    assert_eq!(
        drawn(SlotIcon::Send, "Send", "Send", "", true, false),
        face_of(Slot::Send, false)
    );
    assert_eq!(
        drawn(SlotIcon::Send, "Send", "Send", "", false, false),
        face_of(Slot::SendDisabled, false)
    );
    assert_eq!(
        drawn(
            SlotIcon::Microphone,
            "Record voice message",
            record,
            "",
            true,
            false
        ),
        face_of(Slot::Microphone, false)
    );
    // The recorder draws its own controls: the slot behind it is a Send that does nothing.
    assert!(!face_of(Slot::Recorder, false).enabled);
    // Each icon is a symbolic icon the desktop's theme draws.
    for icon in [
        SlotIcon::Send,
        SlotIcon::Microphone,
        SlotIcon::Stop,
        SlotIcon::Save,
    ] {
        assert!(icon.icon_name().ends_with("-symbolic"), "{icon:?}");
    }
}

/// DIMMED IS NOT DISABLED (S1.3): a dimmed microphone looks disabled but stays enabled — focusable,
/// clickable — and gives its reason as its accessible description (S6). Only row 6, Save with
/// nothing to save, and Send while a send is being written down are disabled.
#[test]
fn a_dimmed_microphone_stays_live_and_says_why() {
    for (reason, sentence) in [
        (Dimmed::Call, "You can record a message after the call."),
        (Dimmed::Busy, "Wait until the current attachment is done."),
        (
            Dimmed::NotSent,
            "Send or delete the voice message that wasn't sent first.",
        ),
    ] {
        let face = face_of(Slot::Dimmed(reason), false);
        assert!(face.enabled);
        assert!(face.looks_dimmed);
        assert_eq!(SlotIcon::Microphone, face.icon);
        assert_eq!("Record voice message", face.name);
        assert_eq!(sentence, face.help_text);
    }
    assert!(!face_of(Slot::Save { enabled: false }, false).enabled);
    assert!(!face_of(Slot::Send, true).enabled);
    // Waiting on a send dims nothing else: a running recording's Send and Stop are always live.
    assert!(face_of(Slot::SendVoice, true).enabled);
    assert!(face_of(Slot::StopRecording, true).enabled);
    for slot in [
        Slot::SendVoice,
        Slot::StopRecording,
        Slot::Save { enabled: true },
        Slot::Send,
        Slot::SendDisabled,
        Slot::Microphone,
    ] {
        assert!(!face_of(slot, false).looks_dimmed, "{slot:?}");
    }
}

#[test]
fn the_slot_speaks_the_readers_language() {
    let russian = common::catalogues().for_language("ru");
    assert_ne!(
        "Record voice message",
        composer_button::face(Slot::Microphone, false, &russian).name
    );
    assert_ne!(
        "Send voice message",
        composer_button::face(Slot::SendVoice, false, &russian).name
    );
    assert_ne!(
        "Stop recording",
        composer_button::face(Slot::StopRecording, false, &russian).name
    );
    assert_ne!(
        "Send or delete the voice message that wasn't sent first.",
        composer_button::face(Slot::Dimmed(Dimmed::NotSent), false, &russian).help_text
    );
    // The shortcut is the same keys in every language.
    assert!(
        composer_button::face(Slot::Microphone, false, &russian)
            .tooltip
            .ends_with("(Ctrl+Shift+R)")
    );
}

// ---- which press does what (S8.6) -----------------------------------------------------------------

/// EVERY INPUT CLICKS: a press of any length with a mouse, a finger or a pen, the keyboard, a
/// screen reader.
#[test]
fn every_input_clicks() {
    for device in [
        SlotDevice::Mouse,
        SlotDevice::Touch,
        SlotDevice::Pen,
        SlotDevice::None,
    ] {
        assert_eq!(
            SlotResponse::Activate,
            composer_button::respond(SlotPress::Click, device, false, true)
        );
        assert_eq!(
            SlotResponse::Activate,
            composer_button::respond(SlotPress::Click, device, false, false)
        );
    }
    // A barrel button "down" on anything but a pen is no barrel.
    assert_eq!(
        SlotResponse::Activate,
        composer_button::respond(SlotPress::Click, SlotDevice::Mouse, true, true)
    );
}

/// The microphone's menu: a mouse's secondary click, a pen tap with the barrel button down,
/// Shift+F10 or the Menu key.
#[test]
fn the_menu_opens_for_the_mouse_the_pens_barrel_and_the_keyboard() {
    assert_eq!(
        SlotResponse::Menu,
        composer_button::respond(SlotPress::SecondaryClick, SlotDevice::Mouse, false, true)
    );
    assert_eq!(
        SlotResponse::Menu,
        composer_button::respond(SlotPress::SecondaryClick, SlotDevice::Pen, true, true)
    );
    // The barrel tap may arrive as the button's click: it is the pen's secondary click, never a
    // recording.
    assert_eq!(
        SlotResponse::Menu,
        composer_button::respond(SlotPress::Click, SlotDevice::Pen, true, true)
    );
    assert_eq!(
        SlotResponse::Menu,
        composer_button::respond(SlotPress::ContextKey, SlotDevice::None, false, true)
    );
}

/// NEVER A HOLD: a finger's or a pen's secondary click without the barrel is ignored, and so is a
/// context request at a pointer — the secondary click has already answered the mouse.
#[test]
fn a_hold_never_opens_the_menu() {
    assert_eq!(
        SlotResponse::Ignore,
        composer_button::respond(SlotPress::SecondaryClick, SlotDevice::Touch, false, true)
    );
    assert_eq!(
        SlotResponse::Ignore,
        composer_button::respond(SlotPress::SecondaryClick, SlotDevice::Pen, false, true)
    );
    assert_eq!(
        SlotResponse::Ignore,
        composer_button::respond(SlotPress::SecondaryClick, SlotDevice::Touch, true, true)
    );
    for device in [
        SlotDevice::Mouse,
        SlotDevice::Touch,
        SlotDevice::Pen,
        SlotDevice::None,
    ] {
        assert_eq!(
            SlotResponse::Ignore,
            composer_button::respond(SlotPress::ContextAtPointer, device, true, true)
        );
    }
}

const MICROPHONE: Slot = Slot::Microphone;
const SEND_TEXT: Slot = Slot::Send;
const SEND_VOICE: Slot = Slot::SendVoice;
const STOP_BESIDE_DRAFT: Slot = Slot::StopRecording;

/// A LONG PRESS ON THE MICROPHONE IS NOT A GESTURE (2026-10-06, every platform): while a finger or
/// a pen without its barrel button is down, nothing records and nothing opens — a secondary click
/// or a context request from the press opens no menu, and the window sends nothing on the press
/// going down — and when it lifts inside the button it is the button's ordinary click, however long
/// it was held, so a slow press is never a dead button (`fc_text::record`'s rule).
#[test]
fn a_long_press_on_the_microphone_starts_nothing_until_it_lifts_and_opens_no_menu() {
    for device in [SlotDevice::Touch, SlotDevice::Pen] {
        let mut guard = SlotGuard::new();
        let press = guard.pointer_down(1_000, device, false);
        // Held for five seconds: a secondary click from it opens nothing.
        assert_eq!(SlotResponse::Ignore, guard.secondary_click(device, true));
        assert_eq!(
            SlotResponse::Ignore,
            composer_button::respond(SlotPress::ContextAtPointer, device, false, true)
        );
        // And no callout while it is down: the slot's tooltip is taken away.
        assert!(!composer_button::tooltip_during(device));
        // Lifted inside: the click — a hands-free recording, exactly as a quick tap.
        assert_eq!(SlotResponse::Activate, guard.click(6_000, MICROPHONE, ""));
        guard.press_ended(press);
    }
    // A mouse's tooltip is a hover's, and a key or a screen reader has no press to hold: theirs
    // stay.
    assert!(composer_button::tooltip_during(SlotDevice::Mouse));
    assert!(composer_button::tooltip_during(SlotDevice::None));
}

/// The menu is the MICROPHONE's (S1.6): on Send, Save or a running recording a secondary click does
/// nothing — and a barrel tap never sends.
#[test]
fn only_the_microphone_has_a_menu() {
    assert_eq!(
        SlotResponse::Ignore,
        composer_button::respond(SlotPress::SecondaryClick, SlotDevice::Mouse, false, false)
    );
    assert_eq!(
        SlotResponse::Ignore,
        composer_button::respond(SlotPress::ContextKey, SlotDevice::None, false, false)
    );
    assert_eq!(
        SlotResponse::Ignore,
        composer_button::respond(SlotPress::Click, SlotDevice::Pen, true, false)
    );
}

// ---- the activation guard (S1.1) and the press reaching the slot (S8.6) ---------------------------

#[test]
fn the_guard_runs_six_hundred_milliseconds_after_the_slots_own_activation() {
    let mut guard = SlotGuard::new();
    assert_eq!(SlotResponse::Activate, guard.click(0, MICROPHONE, ""));
    guard.arm(10_000, "");
    assert_eq!(SlotResponse::Ignore, guard.click(10_000, MICROPHONE, ""));
    assert_eq!(SlotResponse::Ignore, guard.click(10_599, MICROPHONE, ""));
    assert_eq!(SlotResponse::Activate, guard.click(10_600, MICROPHONE, ""));
    assert!(guard.running(10_599));
    assert!(!guard.running(10_600));
}

/// A press that went down while the guard ran is ignored WHOLE, however late it lifts — a slow
/// double click on Send cannot start a recording — while one that went down before the guard was
/// armed, or after it ended, is a press like any other.
#[test]
fn a_press_that_began_while_guarded_is_ignored_whole() {
    // Down under the guard, lifted long after it: ignored — a pointer's press and a key's alike.
    let mut pointer = SlotGuard::new();
    pointer.arm(10_000, "");
    pointer.pointer_down(10_100, SlotDevice::Mouse, false);
    assert_eq!(SlotResponse::Ignore, pointer.click(11_500, MICROPHONE, ""));
    let mut key = SlotGuard::new();
    key.arm(10_000, "");
    key.key_down(10_000);
    assert_eq!(SlotResponse::Ignore, key.click(12_000, MICROPHONE, ""));
    // Down once it has run out: a press like any other.
    let mut after = SlotGuard::new();
    after.arm(10_000, "");
    after.pointer_down(10_600, SlotDevice::Touch, false);
    assert_eq!(SlotResponse::Activate, after.click(10_700, MICROPHONE, ""));
    // Down before it was armed: only the clock decides.
    let mut before = SlotGuard::new();
    before.pointer_down(9_990, SlotDevice::Mouse, false);
    before.arm(10_000, "");
    assert_eq!(SlotResponse::Activate, before.click(10_700, MICROPHONE, ""));
    // A screen reader's activation began with no press at all: only the clock decides.
    assert_eq!(
        SlotResponse::Activate,
        pointer.click(11_600, MICROPHONE, "")
    );
}

/// ARMED AGAIN, IT FORGETS NOTHING (S1.1): a click on the microphone arms the guard and the
/// recording starting arms it again — and the second press of a double click, gone down between the
/// two and held past the later one, is still ignored whole when it lifts on the Send arrow. It
/// cannot send the recording it double-clicked into.
#[test]
fn arming_again_keeps_a_press_that_began_under_the_first_guard_ignored() {
    for lift in [10_200 + 900, 10_200 + 1_300] {
        let mut guard = SlotGuard::new();
        guard.arm(10_000, "");
        guard.pointer_down(10_200, SlotDevice::Mouse, false);
        guard.arm(10_400, "");
        assert_eq!(SlotResponse::Ignore, guard.click(lift, SEND_VOICE, ""));
    }
    // Armed again while it runs, it runs on to 600 ms after the later activation — never shorter.
    let mut longer = SlotGuard::new();
    longer.arm(10_000, "");
    longer.arm(10_400, "");
    assert!(longer.running(10_999));
    assert!(!longer.running(11_000));
    let mut shorter = SlotGuard::new();
    shorter.arm(20_000, "");
    shorter.arm(19_500, "");
    assert!(shorter.running(20_599));
    assert!(!shorter.running(20_600));
    // A press that goes down after every guard has run out is a press like any other.
    longer.pointer_down(11_000, SlotDevice::Mouse, false);
    assert_eq!(SlotResponse::Activate, longer.click(11_100, SEND_VOICE, ""));
}

/// A PRESS IS USED ONCE, AND ENDS. A press that went down while the guard ran and ended without a
/// click — dragged off the button — leaves nothing behind for the next activation that comes with
/// no press of its own: an assistive technology's activation.
#[test]
fn a_press_that_ended_without_a_click_leaves_nothing_behind() {
    let mut guard = SlotGuard::new();
    guard.arm(10_000, "");
    let dragged = guard.pointer_down(10_100, SlotDevice::Mouse, false);
    guard.press_ended(dragged);
    assert_eq!(SlotResponse::Activate, guard.click(20_000, MICROPHONE, ""));
    // A key that went down under the guard and came up with no click is gone the same way.
    guard.arm(30_000, "");
    let key = guard.key_down(30_100);
    guard.press_ended(key);
    assert_eq!(SlotResponse::Activate, guard.click(31_000, MICROPHONE, ""));
}

/// A press's end can arrive after the NEXT press went down: an old press ending forgets only
/// itself, and the press under way — gone down while the guard ran — is still ignored whole when it
/// lifts after the guard has run out.
#[test]
fn an_old_press_ending_leaves_the_new_one_alone() {
    let mut guard = SlotGuard::new();
    guard.arm(10_000, "");
    let first = guard.pointer_down(10_100, SlotDevice::Mouse, false);
    guard.pointer_down(10_200, SlotDevice::Mouse, false);
    guard.press_ended(first);
    assert_eq!(SlotResponse::Ignore, guard.click(10_900, SEND_VOICE, ""));
    // A key's press the same way.
    let mut keys = SlotGuard::new();
    keys.arm(20_000, "");
    let down = keys.key_down(20_050);
    keys.key_down(20_100);
    keys.press_ended(down);
    assert_eq!(SlotResponse::Ignore, keys.click(20_800, SEND_VOICE, ""));
}

/// A pen's barrel tap that reaches the slot only as a secondary click opens the menu — and the next
/// activation with no press, a screen reader's, is an activation: never the menu's twin, never
/// swallowed as the same barrel tap, on Send too.
#[test]
fn a_barrel_tap_leaves_nothing_for_the_next_invoke() {
    let mut guard = SlotGuard::new();
    let tap = guard.pointer_down(1_000, SlotDevice::Pen, true);
    guard.press_ended(tap);
    assert_eq!(
        SlotResponse::Menu,
        guard.secondary_click(SlotDevice::Pen, true)
    );
    assert_eq!(SlotResponse::Activate, guard.click(5_000, MICROPHONE, ""));
    assert_eq!(
        SlotResponse::Activate,
        guard.click(5_100, SEND_TEXT, "hello")
    );
}

/// The barrel tap that arrives twice — as the button's click and as a secondary click — opens ONE
/// menu; the next barrel tap opens its own; and a barrel button "down" on anything but a pen is no
/// barrel.
#[test]
fn a_barrel_tap_opens_one_menu() {
    let mut guard = SlotGuard::new();
    let tap = guard.pointer_down(1_000, SlotDevice::Pen, true);
    assert_eq!(SlotResponse::Menu, guard.click(1_050, MICROPHONE, ""));
    assert_eq!(
        SlotResponse::Ignore,
        guard.secondary_click(SlotDevice::Pen, true)
    );
    guard.press_ended(tap);
    guard.pointer_down(2_000, SlotDevice::Pen, true);
    assert_eq!(
        SlotResponse::Menu,
        guard.secondary_click(SlotDevice::Pen, true)
    );
    assert_eq!(SlotResponse::Ignore, guard.click(2_050, MICROPHONE, ""));
    // A barrel tap on Send is nothing at all — never a send.
    guard.pointer_down(3_000, SlotDevice::Pen, true);
    assert_eq!(SlotResponse::Ignore, guard.click(3_050, SEND_TEXT, "hello"));
    // A mouse's secondary click is always the menu; a finger's never is.
    guard.pointer_down(4_000, SlotDevice::Mouse, true);
    assert_eq!(
        SlotResponse::Menu,
        guard.secondary_click(SlotDevice::Mouse, true)
    );
    assert_eq!(
        SlotResponse::Menu,
        guard.secondary_click(SlotDevice::Mouse, true)
    );
    guard.pointer_down(5_000, SlotDevice::Touch, false);
    assert_eq!(
        SlotResponse::Ignore,
        guard.secondary_click(SlotDevice::Touch, true)
    );
    assert_eq!(SlotResponse::Activate, guard.click(5_050, MICROPHONE, ""));
}

/// ENTER IN THE FIELD IS THE SLOT'S ACTIVATION (S1.3): after the slot's Stop in row 3 — which arms
/// the guard and hands focus to the field, the words still there — a second Enter sends nothing
/// until the guard has run out, exactly as a second click on the slot would not (`fc_text::record`:
/// "a double tap on the Stop square must not send what it staged").
#[test]
fn a_second_enter_after_the_slots_stop_sends_nothing() {
    let mut guard = SlotGuard::new();
    guard.arm(10_000, "the words beside the note");
    assert!(guard.ignores_field_key(10_150, false, true, "the words beside the note"));
    assert!(guard.ignores_field_key(10_599, false, true, "the words beside the note"));
    assert!(!guard.ignores_field_key(10_600, false, true, "the words beside the note"));
    // The click on what is now Send is guarded the same way.
    assert_eq!(
        SlotResponse::Ignore,
        guard.click(10_300, SEND_TEXT, "the words beside the note")
    );
    assert_eq!(
        SlotResponse::Activate,
        guard.click(10_600, SEND_TEXT, "the words beside the note")
    );
    // Nothing guards a composer nobody's slot changed.
    assert!(!SlotGuard::new().ignores_field_key(0, false, true, "hello"));
}

/// A CHANGE MADE BY TYPING IS NEVER GUARDED (S1.1): after a send emptied the box, "ok" typed at once
/// and sent at once goes — by Enter or by a click, even before the field has said the words
/// changed. A microphone nobody's words have touched since still waits.
#[test]
fn words_typed_since_the_guard_was_armed_are_never_guarded() {
    let mut guard = SlotGuard::new();
    guard.arm(10_000, "");
    assert!(!guard.ignores_field_key(10_300, false, true, "ok"));
    assert_eq!(SlotResponse::Activate, guard.click(10_300, SEND_TEXT, "ok"));
    // Nothing typed: the microphone the send left behind waits.
    assert_eq!(SlotResponse::Ignore, guard.click(10_300, MICROPHONE, ""));
    // A Save after the words changed is a Save.
    guard.arm(20_000, "");
    assert_eq!(
        SlotResponse::Activate,
        guard.click(20_100, Slot::Save { enabled: true }, "fixed")
    );
    // A Stop is not a send: words changed meanwhile change nothing for it.
    guard.arm(30_000, "before");
    assert_eq!(
        SlotResponse::Ignore,
        guard.click(30_100, STOP_BESIDE_DRAFT, "after")
    );
}

/// STAGING IS NEVER GUARDED (S1.1): after a send emptied the box, a picture pasted or dropped at
/// once and sent at once goes — by Enter or by a click — though the words did not change. A held
/// key's repeats still never act, and the slot's own activation arming the guard again — a Stop in
/// row 3, whose note the slot stages itself — forgets what the person staged before it.
#[test]
fn something_staged_since_the_guard_was_armed_is_never_guarded() {
    let mut guard = SlotGuard::new();
    guard.arm(10_000, "");
    guard.changed(false);
    assert!(!guard.ignores_field_key(10_300, false, true, ""));
    assert_eq!(SlotResponse::Activate, guard.click(10_300, SEND_TEXT, ""));
    assert!(guard.ignores_field_key(10_300, true, true, ""));
    // A press that went down under the guard, then the picture landed: its Send is the person's,
    // and goes.
    let mut pressed = SlotGuard::new();
    pressed.arm(20_000, "");
    pressed.pointer_down(20_100, SlotDevice::Mouse, false);
    pressed.changed(false);
    assert_eq!(SlotResponse::Activate, pressed.click(20_300, SEND_TEXT, ""));
    // Armed again by the slot's own activation: what was staged before it is forgotten, and Enter
    // waits.
    guard.arm(30_000, "the words beside the note");
    assert!(guard.ignores_field_key(30_100, false, true, "the words beside the note"));
    assert_eq!(
        SlotResponse::Ignore,
        guard.click(30_100, SEND_TEXT, "the words beside the note")
    );
    // Nothing staged, nothing typed: a send's guard holds, as it always did.
    let mut untouched = SlotGuard::new();
    untouched.arm(40_000, "");
    assert!(untouched.ignores_field_key(40_300, false, true, ""));
}

/// THE PERSON'S OWN CHANGE LIFTS THE GUARD (S1.1; `fc_text::record`'s `OtherAction`): "ok" sent,
/// then "x" typed and deleted at once, leaves a microphone that records; words typed and taken back
/// to the very words the guard was armed over leave a Send that sends; and the field's own clear
/// after a send, which leaves what the guard was armed over, lifts nothing. While a recording runs
/// nothing lifts the guard on its Send, and a press on the microphone that went down under the guard
/// stays ignored whole.
#[test]
fn the_persons_own_change_lifts_the_guard_for_the_microphone_too() {
    let mut typed = SlotGuard::new();
    typed.arm(10_000, "");
    typed.text_changed("", false);
    assert_eq!(SlotResponse::Ignore, typed.click(10_100, MICROPHONE, ""));
    typed.text_changed("x", false);
    typed.text_changed("", false);
    assert!(!typed.running(10_200));
    assert_eq!(SlotResponse::Activate, typed.click(10_200, MICROPHONE, ""));

    // Back to the words the Stop in row 3 left beside its note: still the person's change.
    let mut back = SlotGuard::new();
    back.arm(20_000, "dinner?");
    back.text_changed("dinner", false);
    back.text_changed("dinner?", false);
    assert_eq!(
        SlotResponse::Activate,
        back.click(20_200, SEND_TEXT, "dinner?")
    );
    assert!(!back.ignores_field_key(20_200, false, true, "dinner?"));

    // Recording: the box is behind the row, and the guard on its Send stays.
    let mut recording = SlotGuard::new();
    recording.arm(30_000, "");
    recording.text_changed("x", true);
    recording.changed(true);
    assert!(recording.running(30_100));
    assert_eq!(
        SlotResponse::Ignore,
        recording.click(30_100, SEND_VOICE, "")
    );

    // A press on the microphone that went down under the guard is ignored whole, whatever changes
    // before it lifts.
    let mut pressed = SlotGuard::new();
    pressed.arm(40_000, "");
    pressed.pointer_down(40_100, SlotDevice::Mouse, false);
    pressed.text_changed("x", false);
    pressed.text_changed("", false);
    assert_eq!(SlotResponse::Ignore, pressed.click(40_200, MICROPHONE, ""));
}

/// AN ITEM TAKEN OFF THE STRIP IS THE PERSON'S CHANGE (S1.1, S2.7): after a Stop beside words, ✕ on
/// the note and Send at once sends the words — as on Android and the web.
#[test]
fn an_item_taken_off_after_a_stop_beside_it_leaves_a_send_that_sends() {
    let mut guard = SlotGuard::new();
    guard.arm(10_000, "the words beside the note");
    assert_eq!(
        SlotResponse::Ignore,
        guard.click(10_100, SEND_TEXT, "the words beside the note")
    );
    guard.changed(false);
    assert_eq!(
        SlotResponse::Activate,
        guard.click(10_150, SEND_TEXT, "the words beside the note")
    );
    assert!(!guard.ignores_field_key(10_150, false, true, "the words beside the note"));
}

/// A KEY HELD DOWN IS ONE PRESS (S2.4, S8.6): its first went wherever focus was — the slot's Stop
/// or Send, the recording row's Stop or Delete, Esc stopping a recording — and handed focus to the
/// field; its repeats there never send, guard or no guard, and an Esc's never drop the reply the
/// note was recorded under. A fresh Esc is not the slot's and waits for nothing.
#[test]
fn a_held_keys_repeats_never_act_in_the_field() {
    let mut guard = SlotGuard::new();
    assert!(guard.ignores_field_key(50_000, true, true, "hello"));
    assert!(guard.ignores_field_key(50_000, true, false, "hello"));
    guard.arm(60_000, "hello");
    assert!(guard.ignores_field_key(61_000, true, true, "hello"));
    assert!(guard.ignores_field_key(61_000, true, true, "hello, edited"));
    assert!(!guard.ignores_field_key(60_100, false, false, "hello"));
}

/// CTRL+SHIFT+R HELD DOWN IS ONE PRESS (S1.6): a held key repeats, so the window swallows the
/// chord's repeats as they come in and an invocation that still follows one does nothing — a held
/// chord neither stops the recording it started nor starts one after its Stop. A fresh press always
/// acts, and so does one whose key-down the window never saw.
#[test]
fn a_held_shortcut_acts_once() {
    let mut shortcut = ShortcutPresses::new();
    assert!(shortcut.fresh(0));
    assert!(!shortcut.key_down(1_000, false));
    assert!(shortcut.fresh(1_000));
    assert!(shortcut.key_down(1_500, true));
    assert!(!shortcut.fresh(1_500));
    assert!(!shortcut.fresh(1_500 + ShortcutPresses::SAME_KEY_MS));
    assert!(shortcut.key_down(1_533, true));
    assert!(!shortcut.fresh(1_540));
    // Long after the last repeat seen, an invocation is another press — one whose key went down
    // outside this view.
    assert!(shortcut.fresh(1_533 + ShortcutPresses::SAME_KEY_MS + 1));
    assert!(!shortcut.key_down(3_000, false));
    assert!(shortcut.fresh(3_000));
}

// ---- the menu's two record commands (the Mac's `MacVoiceCommands`) -------------------------------

/// An empty main composer in a family chat, with a microphone: the place both commands act.
fn idle() -> SlotInputs {
    SlotInputs {
        recorder_open: false,
        recording: Recording::None,
        editing: false,
        draft_blank: true,
        staged: false,
        assistant_chat: false,
        can_record: true,
        call: false,
        busy: false,
        not_sent: false,
    }
}

#[test]
fn record_voice_is_offered_wherever_the_microphone_is_not_dimmed() {
    assert!(composer_button::records_voice(&idle()));
    // Beside words or staged items it records beside them.
    let words = SlotInputs {
        draft_blank: false,
        staged: true,
        ..idle()
    };
    assert!(composer_button::records_voice(&words));
    // During a recording it stops it — even in a call that began meanwhile.
    let recording = SlotInputs {
        recording: Recording::HandsFreeBesideDraft,
        call: true,
        ..idle()
    };
    assert!(composer_button::records_voice(&recording));
    for off in [
        SlotInputs {
            recorder_open: true,
            ..idle()
        },
        SlotInputs {
            assistant_chat: true,
            ..idle()
        },
        SlotInputs {
            can_record: false,
            ..idle()
        },
        SlotInputs {
            editing: true,
            ..idle()
        },
        SlotInputs {
            call: true,
            ..idle()
        },
        SlotInputs {
            busy: true,
            ..idle()
        },
        SlotInputs {
            not_sent: true,
            ..idle()
        },
    ] {
        assert!(!composer_button::records_voice(&off), "{off:?}");
    }
}

#[test]
fn record_video_needs_the_door_and_a_free_composer() {
    assert!(composer_button::records_video(&idle(), true));
    assert!(!composer_button::records_video(&idle(), false));
    // A voice message waiting, words, staged items: a video message travels alone.
    let waiting = SlotInputs {
        not_sent: true,
        draft_blank: false,
        staged: true,
        ..idle()
    };
    assert!(composer_button::records_video(&waiting, true));
    for off in [
        SlotInputs {
            recorder_open: true,
            ..idle()
        },
        SlotInputs {
            recording: Recording::HandsFree,
            ..idle()
        },
        SlotInputs {
            assistant_chat: true,
            ..idle()
        },
        SlotInputs {
            editing: true,
            ..idle()
        },
        SlotInputs {
            call: true,
            ..idle()
        },
        SlotInputs {
            busy: true,
            ..idle()
        },
    ] {
        assert!(!composer_button::records_video(&off, true), "{off:?}");
    }
}
