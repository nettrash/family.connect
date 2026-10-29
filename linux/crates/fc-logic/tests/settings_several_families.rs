//! `SeveralFamiliesTests`' one test whose subject is `Settings.cs`: leaving one of several says
//! which family and what it costs; with one family the dialog is today's, word for word; and
//! leaving as the last member still says it deletes the family. (The rest of that file is the
//! session's, the doors', the rules', the chat list's, the live connection's, the notifications'
//! and the console's: `app_session_several_families.rs`, `doorway_several_families.rs`,
//! `families_several_families.rs`, `chat_list_several_families.rs`,
//! `live_connection_several_families.rs`, `notifications_several_families.rs`,
//! `family_console_several_families.rs`, and the family's own, `family_several_families.rs` and
//! `media_outbox_several_families.rs`.)

mod common;

use fc_logic::settings::{LeaveContext, LeaveKind, settings_text};

#[test]
fn the_leave_dialog_names_the_family_when_there_are_several() {
    let say = common::english();
    let several = LeaveContext::new(LeaveKind::Member).in_family("Ivanovs");
    assert_eq!(
        settings_text::leave_title(&several, &say),
        "Leave “Ivanovs”?"
    );
    assert_eq!(
        settings_text::leave_message(&several, &say),
        "You'll lose access to this family's chats. Direct chats stay with anyone you share another family with."
    );
    let heir = LeaveContext {
        kind: LeaveKind::Successor,
        successor_name: Some("Bob".to_string()),
        family_name: Some("Ivanovs".to_string()),
    };
    assert_eq!(
        settings_text::leave_message(&several, &say),
        settings_text::leave_message(&heir, &say)
    );
    let one = LeaveContext::new(LeaveKind::Member);
    assert_eq!(settings_text::leave_title(&one, &say), "Leave the family?");
    assert_eq!(
        settings_text::leave_message(&one, &say),
        "You'll lose access to the family chat and your direct chats. Your history returns if you rejoin."
    );
    let last = LeaveContext::new(LeaveKind::LastMember).in_family("Ivanovs");
    assert_eq!(
        settings_text::leave_title(&last, &say),
        "Delete the family?"
    );
    assert_eq!(
        settings_text::leave_message(&last, &say),
        "You're the only member left. Leaving deletes the family and everything in it."
    );
}
