//! `ToastArgumentsTests`' two cases that need `Notifications.cs`: how a toast's id is written, and
//! when a note is worth one — plus `For`'s round trip through the parse, in its Linux form (an
//! action and a `GVariant` target; the parse cases are `notification_target_toast_arguments.rs`).

use fc_core::protocol::NoteDto;
use fc_logic::notification_target::{
    NotificationTarget, OPEN_BOARD, OPEN_CHAT, TargetValue, ToastTarget, parse,
};
use fc_logic::notifications::{NotificationRules, Toast};
use fc_logic::toast_arguments::{ToastArguments, for_toast, is_news};
use fc_text::board::{Marks, is_unread};

/// The C#'s `Assert.Equal("1234567", big[0].Value)` under fi-FI, whose digit grouping would have
/// written a space into a culture-formatted id: here the id is an `x`, a number with no text at
/// all, and the tag is Rust's own integer formatting, which has no culture.
#[test]
fn the_id_is_written_the_same_way_in_every_language() {
    let big = for_toast(&Toast::new("chat-1234567", "t", "b", Some(1234567)));
    assert_eq!(Some(TargetValue::Int64(1234567)), big.target);
    assert_eq!("chat-1234567", NotificationRules::chat_tag(1234567));
}

/// A NOTE IS NEWS BY THE BADGE'S RULE, and a tombstone never is.
#[test]
fn a_note_is_news_exactly_when_the_badge_would_count_it() {
    let marks = Marks {
        note_id: 5,
        content_seq: 10,
    };
    let note = NoteDto {
        kind: Some("text".to_string()),
        text: Some("Milk".to_string()),
        board_seq: 12,
        content_seq: Some(12),
        ..NoteDto::new(7, 11)
    };

    assert_eq!(is_unread(7, Some(12), marks), is_news(&note, marks));
    assert!(is_news(&note, marks));
    assert!(!is_news(
        &NoteDto {
            deleted: true,
            ..note.clone()
        },
        marks
    ));
    assert!(!is_news(
        &note,
        Marks {
            note_id: 7,
            content_seq: 12
        }
    ));
}

/// `For` is the parse's inverse: what a chat's toast, a family's wall's and a wall's with no family
/// write comes back as the target it was raised for — the C#'s
/// `Parse(For(toast).ToDictionary())` of `AChatNotificationOpensItsChatAndTheWallsOpensTheWall`.
#[test]
fn what_a_toast_writes_the_click_reads_back() {
    let chat = Toast::new("chat-42", "The Smiths", "New message", Some(42));
    let family_wall = Toast::new("board", "Ivanovs", "New note", None).in_family(Some(5));
    let wall = Toast::new("board", "The Smiths", "New note", None);

    for (toast, target) in [
        (&chat, ToastTarget::chat(42)),
        (&family_wall, ToastTarget::board(Some(5))),
        (&wall, ToastTarget::board(None)),
    ] {
        let written = for_toast(toast);
        assert_eq!(
            Some(NotificationTarget::Open(target)),
            parse(written.action, written.target.as_ref())
        );
        assert_eq!(
            Some(NotificationTarget::Open(target)),
            parse(&written.detailed_action(), written.target.as_ref())
        );
    }
    assert_eq!(
        ToastArguments {
            action: OPEN_CHAT,
            target: Some(TargetValue::Int64(42))
        },
        for_toast(&chat)
    );
    assert_eq!("app.open-board", for_toast(&wall).detailed_action());
    assert_eq!(OPEN_BOARD, for_toast(&wall).action);
    assert_eq!(None, for_toast(&wall).target);
}
