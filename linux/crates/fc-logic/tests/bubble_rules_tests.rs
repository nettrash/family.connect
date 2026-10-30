//! `BubbleRulesTests`: the web client's timeline rules for a bubble — the assistant, an awaited
//! answer, the tick, the Safety items.

use fc_core::protocol::{AttachmentDto, CallRecordDto, MessageDto, PollDto};
use fc_logic::bubble_rules::{
    awaited, is_assistant, is_other_member, may_report, may_report_assistant, seen, shows_tick,
};

const ME: i64 = 7;
const ANNA: i64 = 11;
const ASSISTANT: i64 = 99;
const AT: &str = "2026-09-16T10:00:00Z";

fn from(sender: i64) -> MessageDto {
    from_with(sender, "", 5)
}

fn from_with(sender: i64, body: &str, id: i64) -> MessageDto {
    MessageDto::new(id, 42, sender, None, body, AT)
}

#[test]
fn the_assistant_is_its_account_in_the_family_and_anybody_but_you_in_its_own_chat() {
    assert!(is_assistant(&from(ASSISTANT), ME, false, Some(ASSISTANT)));
    assert!(!is_assistant(&from(ANNA), ME, false, Some(ASSISTANT)));
    assert!(!is_assistant(&from(ASSISTANT), ME, false, None));
    assert!(is_assistant(&from(ANNA), ME, true, None));
    assert!(!is_assistant(&from(ME), ME, true, Some(ASSISTANT)));

    assert!(is_other_member(&from(ANNA), ME, false, Some(ASSISTANT)));
    assert!(!is_other_member(&from(ME), ME, false, Some(ASSISTANT)));
    assert!(!is_other_member(
        &from(ASSISTANT),
        ME,
        false,
        Some(ASSISTANT)
    ));
    assert!(!is_other_member(&from(ANNA), ME, true, Some(ASSISTANT)));
}

/// A numbered row carrying nothing, not mine, from somebody who can be the assistant — and
/// nothing else.
#[test]
fn an_answer_is_awaited_while_it_carries_nothing() {
    assert!(awaited(&from(ASSISTANT), "", ME, false, Some(ASSISTANT)));
    assert!(awaited(&from(ANNA), "", ME, true, None));

    assert!(!awaited(
        &from(ASSISTANT),
        "Sure — ",
        ME,
        false,
        Some(ASSISTANT)
    ));
    let photo = MessageDto {
        attachments: Some(vec![AttachmentDto::new(3, "photo")]),
        ..from(ASSISTANT)
    };
    assert!(!awaited(&photo, "", ME, false, Some(ASSISTANT)));
    let poll = MessageDto {
        poll: Some(PollDto {
            poll_seq: 1,
            closed: false,
            options: Vec::new(),
        }),
        ..from(ASSISTANT)
    };
    assert!(!awaited(&poll, "", ME, false, Some(ASSISTANT)));
    let call = MessageDto {
        call: Some(CallRecordDto {
            outcome: "missed".to_string(),
            ..CallRecordDto::default()
        }),
        ..from(ASSISTANT)
    };
    assert!(!awaited(&call, "", ME, false, Some(ASSISTANT)));
    assert!(!awaited(&from(ME), "", ME, true, Some(ASSISTANT)));
    assert!(!awaited(
        &from_with(ASSISTANT, "", 0),
        "",
        ME,
        false,
        Some(ASSISTANT)
    ));
    assert!(!awaited(&from(ANNA), "", ME, false, Some(ASSISTANT)));
}

#[test]
fn the_tick_is_a_direct_chat_fact_about_my_numbered_messages() {
    assert!(shows_tick(&from_with(ME, "hi", 5), ME, false));
    assert!(!shows_tick(&from_with(ME, "hi", 5), ME, true));
    assert!(!shows_tick(&from_with(ANNA, "hi", 5), ME, false));
    assert!(!shows_tick(&from_with(ME, "hi", 0), ME, false));

    assert!(seen(&from_with(ME, "hi", 5), ME, false, 5));
    assert!(!seen(&from_with(ME, "hi", 6), ME, false, 5));
    assert!(!seen(&from_with(ME, "hi", 5), ME, true, 9));
    assert!(!seen(&from_with(ANNA, "hi", 5), ME, false, 9));
    assert!(!seen(&from_with(ME, "hi", 0), ME, false, 9));
}

#[test]
fn only_another_members_numbered_message_may_be_reported() {
    assert!(may_report(&from(ANNA), ME, false, Some(ASSISTANT)));
    assert!(!may_report(
        &from_with(ANNA, "", 0),
        ME,
        false,
        Some(ASSISTANT)
    ));
    assert!(!may_report(&from(ME), ME, false, Some(ASSISTANT)));
    assert!(!may_report(&from(ASSISTANT), ME, false, Some(ASSISTANT)));
}

/// The assistant's own path, which the member one refuses: a reply may be reported, and the two
/// rules never both answer true for the same bubble — the member endpoint would answer
/// `not_same_family` for the assistant, and the assistant endpoint answers `message_not_found` for
/// a member (docs/protocol.md, "Reporting the assistant").
#[test]
fn an_assistant_reply_may_be_reported_down_its_own_path() {
    assert!(may_report_assistant(
        &from(ASSISTANT),
        ME,
        false,
        Some(ASSISTANT)
    ));
    // Its own chat, where anybody who is not the reader is the assistant.
    assert!(may_report_assistant(&from(ASSISTANT), ME, true, None));
    // Not until the server has numbered it.
    assert!(!may_report_assistant(
        &from_with(ASSISTANT, "", 0),
        ME,
        false,
        Some(ASSISTANT)
    ));
    // A member's message is not this path's business, and neither is your own.
    assert!(!may_report_assistant(
        &from(ANNA),
        ME,
        false,
        Some(ASSISTANT)
    ));
    assert!(!may_report_assistant(&from(ME), ME, true, Some(ASSISTANT)));
    // Exclusive, both ways, for every bubble a chat can hold.
    for (message, chat) in [
        (from(ASSISTANT), false),
        (from(ANNA), false),
        (from(ME), false),
        (from(ASSISTANT), true),
        (from(ANNA), true),
        (from(ME), true),
    ] {
        assert!(
            !(may_report(&message, ME, chat, Some(ASSISTANT))
                && may_report_assistant(&message, ME, chat, Some(ASSISTANT))),
            "one bubble is never both a member report and an assistant report"
        );
    }
}

/// Not in this C# file — `MediaMark` is pinned by `CachedFlagsTests` through the store and the
/// resync (a later wave); its three branches directly: a sticker, a circle, and every other set of
/// attachments by id, kind and whether it has a preview.
#[test]
fn a_bubbles_media_mark_names_its_branch_and_its_ids() {
    use fc_logic::bubble_rules::media_mark;
    let sticker = MessageDto {
        attachments: Some(vec![AttachmentDto {
            sticker: true,
            ..AttachmentDto::new(90, "photo")
        }]),
        ..from(ANNA)
    };
    assert_eq!(media_mark(&sticker), "s90");
    let circle = MessageDto {
        attachments: Some(vec![AttachmentDto {
            round: true,
            ..AttachmentDto::new(91, "video")
        }]),
        ..from(ANNA)
    };
    assert_eq!(media_mark(&circle), "r91");
    let album = MessageDto {
        attachments: Some(vec![
            AttachmentDto {
                has_preview: true,
                ..AttachmentDto::new(3, "photo")
            },
            AttachmentDto::new(4, "file"),
        ]),
        ..from(ANNA)
    };
    assert_eq!(media_mark(&album), "m3photo+.4file-");
    // The flag on a photo with words is still a sticker; on one of two, the tiles.
    let two = MessageDto {
        attachments: Some(vec![
            AttachmentDto {
                sticker: true,
                ..AttachmentDto::new(5, "photo")
            },
            AttachmentDto::new(6, "photo"),
        ]),
        ..from(ANNA)
    };
    assert_eq!(media_mark(&two), "m5photo-.6photo-");
    assert_eq!(media_mark(&from(ANNA)), "");
}
