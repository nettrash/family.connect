//! `AttachMenuTests`: the paperclip's menu (docs/attachment-menu-2026-10-07.md, issue #78) — the
//! same lines in the same three groups on every client, top to bottom, no Camera, and no separator
//! around a group that is empty.

mod common;

use fc_core::protocol::AssistantDto;
use fc_logic::attach_menu::{AttachItem, PHOTO_OR_VIDEO_TYPES, assistant_available, groups};

use common::consent::{PROCESSOR, assistant};

use AttachItem::*;

fn menu(actual: Vec<Vec<AttachItem>>, expected: &[&[AttachItem]]) {
    assert_eq!(expected.len(), actual.len());
    for (group, lines) in expected.iter().enumerate() {
        assert_eq!(lines.to_vec(), actual[group]);
    }
}

#[test]
fn a_family_chat_has_every_line_in_three_groups() {
    menu(
        groups(false, false, true, true, false),
        &[
            &[PhotoOrVideo, File, Paste],
            &[RecordVoice, RecordVideo],
            &[Location, Poll],
        ],
    );
}

#[test]
fn a_direct_chat_has_no_poll_and_no_video_message_where_none_can_be_recorded() {
    menu(
        groups(false, false, false, false, true),
        &[&[PhotoOrVideo, File, Paste], &[RecordVoice], &[Location]],
    );
}

#[test]
fn the_assistants_photo_stands_in_photo_or_videos_place_and_nothing_is_recorded() {
    // Never both lines, and no recording group — so no empty group for a separator to sit around.
    menu(
        groups(true, true, false, true, true),
        &[&[AssistantPhoto, File, Paste], &[Location]],
    );
}

#[test]
fn an_assistant_that_cannot_see_offers_no_photo_line_at_all() {
    menu(
        groups(true, false, false, false, false),
        &[&[File, Paste], &[Location]],
    );
}

#[test]
fn no_group_is_ever_empty_and_no_line_appears_twice() {
    for assistant in [false, true] {
        for pictures in [false, true] {
            for family in [false, true] {
                for round in [false, true] {
                    for asks in [false, true] {
                        let groups = groups(assistant, pictures, family, round, asks);
                        assert!(groups.iter().all(|group| !group.is_empty()));
                        let lines: Vec<AttachItem> = groups.iter().flatten().copied().collect();
                        let mut distinct = Vec::new();
                        for line in &lines {
                            if !distinct.contains(line) {
                                distinct.push(*line);
                            }
                        }
                        assert_eq!(lines.len(), distinct.len());
                        // Never both photo lines.
                        let photo_lines = lines
                            .iter()
                            .filter(|line| matches!(line, PhotoOrVideo | AssistantPhoto))
                            .count();
                        assert!(photo_lines <= 1);
                        assert!(lines.contains(&File));
                        assert!(lines.contains(&Paste));
                        assert!(lines.contains(&Location));
                        // "Ask the Assistant" only in the family chat of a server whose assistant
                        // can be asked — and first in its group.
                        assert_eq!(family && asks, lines.contains(&AskAssistant));
                        if family && asks {
                            assert_eq!(AskAssistant, groups[groups.len() - 1][0]);
                        }
                    }
                }
            }
        }
    }
}

/// #106: the @ai button left the composer's row, and "Ask the Assistant" opens the family chat's
/// last group — never a direct chat's, never the assistant's own (where every message already asks
/// it).
#[test]
fn ask_the_assistant_opens_the_family_chats_last_group() {
    menu(
        groups(false, false, true, false, true),
        &[
            &[PhotoOrVideo, File, Paste],
            &[RecordVoice],
            &[AskAssistant, Location, Poll],
        ],
    );
    menu(
        groups(false, false, false, false, true),
        &[&[PhotoOrVideo, File, Paste], &[RecordVoice], &[Location]],
    );
    menu(
        groups(true, false, false, false, true),
        &[&[File, Paste], &[Location]],
    );
}

/// An assistant can be asked only where the server names who answers (docs/protocol.md,
/// "Consenting to the assistant").
#[test]
fn the_assistant_is_available_only_where_its_processor_is_named() {
    let named = |processor: Option<&str>| AssistantDto {
        processor: processor.map(str::to_string),
        ..assistant(99, "Assistant")
    };
    assert!(assistant_available(Some(&named(Some(PROCESSOR)))));
    assert!(!assistant_available(Some(&named(None))));
    assert!(!assistant_available(Some(&named(Some("  ")))));
    assert!(!assistant_available(None));
}

#[test]
fn the_photo_or_video_picker_offers_only_pictures_and_videos_the_client_knows() {
    for kind in PHOTO_OR_VIDEO_TYPES {
        assert!(kind.starts_with('.'));
        let mime = fc_text::media::mime_for(&format!("x{kind}"));
        assert!(
            mime.starts_with("image/") || mime.starts_with("video/"),
            "{kind} is {mime}"
        );
    }
    let mut distinct = PHOTO_OR_VIDEO_TYPES.to_vec();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(PHOTO_OR_VIDEO_TYPES.len(), distinct.len());
    // Every video the server takes as a video, and every photo the client re-draws.
    for kind in [
        ".mp4", ".m4v", ".mov", ".jpg", ".jpeg", ".png", ".heic", ".heif", ".tif", ".tiff", ".avif",
    ] {
        assert!(PHOTO_OR_VIDEO_TYPES.contains(&kind), "{kind}");
    }
    assert!(!PHOTO_OR_VIDEO_TYPES.contains(&".svg"));
}
