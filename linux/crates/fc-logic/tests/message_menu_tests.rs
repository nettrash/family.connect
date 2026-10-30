//! `MessageMenuTests`: a message's context menu (the approved design of 2026-10-05) — a voice or
//! video message offers what does something for a recording — reactions, Reply, Show text, Playback
//! speed, Save…, Open Full Screen — and never Copy, Edit or anything about text it does not have;
//! every other message's menu is exactly what it was.

use fc_core::protocol::{ApiError, AttachmentDto, MessageDto, error_codes};
use fc_logic::message_menu::{self, MessageAction, RecordingKind, TextItem};
use fc_logic::transcripts::{TranscriptLook, TranscriptPhase, transcript_rules};

use MessageAction::*;

/// `new AttachmentDto(40, "audio", "audio/mp4", 9000, DurationMs: 4200)`.
fn voice() -> AttachmentDto {
    AttachmentDto {
        mime: Some("audio/mp4".to_string()),
        size: Some(9000),
        duration_ms: Some(4200),
        ..AttachmentDto::new(40, "audio")
    }
}

/// `new AttachmentDto(91, "video", "video/mp4", 1649700, 480, 480, 23400, HasPreview: true, Round:
/// true)`.
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

/// `new AttachmentDto(50, "photo", "image/jpeg", 1000, 600, 800)`.
fn photo() -> AttachmentDto {
    AttachmentDto {
        mime: Some("image/jpeg".to_string()),
        size: Some(1000),
        width: Some(600),
        height: Some(800),
        ..AttachmentDto::new(50, "photo")
    }
}

/// `new MessageDto(1, 42, 7, null, body, "2026-10-05T10:00:00Z", ThreadRootId: root, Attachments:
/// media)`.
fn message(body: &str, root: Option<i64>, media: Vec<AttachmentDto>) -> MessageDto {
    MessageDto {
        thread_root_id: root,
        attachments: Some(media),
        ..MessageDto::new(1, 42, 7, None, body, "2026-10-05T10:00:00Z")
    }
}

#[test]
fn a_voice_message_has_nothing_to_copy_or_edit() {
    let note = message("", None, vec![voice()]);
    assert_eq!(Some(RecordingKind::Voice), message_menu::recording(&note));
    assert_eq!(
        vec![React, Reply, ShowText, PlaybackSpeed, Save],
        message_menu::actions(&note, true, false, true)
    );
    assert_eq!(
        vec![React, Reply, PlaybackSpeed, Save],
        message_menu::actions(&note, false, false, false)
    );
    // In a chain: the thread straight after Reply — the order iOS, the Mac, Android and the web
    // draw.
    assert_eq!(
        vec![React, Reply, ViewThread, ShowText, PlaybackSpeed, Save],
        message_menu::actions(&message("", Some(3), vec![voice()]), false, false, true)
    );
}

#[test]
fn a_video_message_opens_full_screen_and_has_no_speed() {
    let round = message("", Some(3), vec![circle()]);
    assert_eq!(Some(RecordingKind::Round), message_menu::recording(&round));
    assert_eq!(
        vec![React, Reply, ViewThread, ShowText, Save, OpenFullScreen],
        message_menu::actions(&round, false, false, true)
    );
    assert!(!message_menu::actions(&round, false, true, true).contains(&ViewThread));
}

/// Words make it a message with words: a sound with a caption, two sounds, a sound beside a photo —
/// as ever.
#[test]
fn any_other_message_keeps_its_menu() {
    assert_eq!(
        None,
        message_menu::recording(&message("Listen to this", None, vec![voice()]))
    );
    let second = AttachmentDto { id: 41, ..voice() };
    assert_eq!(
        None,
        message_menu::recording(&message("", None, vec![voice(), second]))
    );
    assert_eq!(
        None,
        message_menu::recording(&message("", None, vec![voice(), photo()]))
    );
    assert_eq!(None, message_menu::recording(&message("hi", None, vec![])));
    let flat = AttachmentDto {
        round: false,
        ..circle()
    };
    assert_eq!(
        None,
        message_menu::recording(&message("", None, vec![flat]))
    );
    assert_eq!(
        vec![Reply, React, Copy, Edit],
        message_menu::actions(
            &message("Listen to this", None, vec![voice()]),
            true,
            false,
            true
        )
    );
    assert_eq!(
        vec![Reply, React, Copy, ViewThread, Edit],
        message_menu::actions(&message("hi", Some(3), vec![]), true, false, false)
    );
    assert_eq!(
        vec![Reply, React, Copy],
        message_menu::actions(&message("hi", Some(3), vec![]), true, true, false)
    );
    assert_eq!(
        vec![Reply, React],
        message_menu::actions(&message("", None, vec![photo()]), false, false, false)
    );
}

// ---- the text item, as the words under the recording offer it ----------------------------------

/// Folded away: "Show text"; open: "Hide text" — the same two the row under the player shows.
#[test]
fn the_text_item_is_show_or_hide_as_the_row_is() {
    assert_eq!(
        TextItem::Show,
        message_menu::text_item_for(&TranscriptLook::closed())
    );
    assert_eq!(
        TextItem::Hide,
        message_menu::text_item_for(&TranscriptLook::open("hello"))
    );
    assert_eq!(
        TextItem::Hide,
        message_menu::text_item_for(&TranscriptLook::open(""))
    );
}

/// "Getting the text…" is already asking: a second ask from the menu would only race the first.
#[test]
fn no_text_item_while_it_is_being_asked_for() {
    assert_eq!(
        TextItem::None,
        message_menu::text_item_for(&TranscriptLook::asking())
    );
}

/// A failure the row offers "Try Again" for may be asked again from the menu too; one it
/// deliberately offers nothing for — a refusal, a final error — gets no menu item that would re-ask
/// behind the row's back.
#[test]
fn a_failed_ask_is_offered_again_only_where_the_row_offers_try_again() {
    let transient = ApiError::transport("offline");
    let last = ApiError::new(error_codes::TRANSCRIPT_REFUSED, "no", 422);
    assert!(transcript_rules::may_retry(&transient));
    assert!(!transcript_rules::may_retry(&last));
    assert_eq!(
        TextItem::Show,
        message_menu::text_item_for(&TranscriptLook::failed(transient))
    );
    assert_eq!(
        TextItem::None,
        message_menu::text_item_for(&TranscriptLook::failed(last))
    );
    // No error recorded is the row's "no answer": a transport failure, which it offers Try Again
    // for.
    assert_eq!(
        TextItem::Show,
        message_menu::text_item_for(&TranscriptLook::new(TranscriptPhase::Failed, None, None))
    );
}
