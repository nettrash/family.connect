//! `SeveralFamiliesTests.ANotificationIsTitledWithItsOwnFamily`, whose subject is
//! `NotificationRules` (and `ToastArguments.For`): a family chat's notification is titled with THAT
//! chat's family's name, and a note with its wall's — not the shown family's — and the wall's
//! notification says whose. (The C#'s last line, a family id of `-2` reading as the wall shown, is a
//! parse case: `notification_target_toast_arguments.rs`.)

mod common;

use std::rc::Rc;

use fc_core::protocol::{ChatDto, ChatRowDto, FamilyDto, MemberDto, MessageDto, NoteDto};
use fc_core::store::{ChatStore, Database, StoreResult};
use fc_logic::notification_target::{NotificationTarget, ToastTarget, parse};
use fc_logic::notifications::NotificationRules;
use fc_logic::toast_arguments::for_toast;

#[test]
fn a_notification_is_titled_with_its_own_family() -> StoreResult<()> {
    let cache = Database::open_in_memory()?;
    let chats = Rc::new(
        ChatStore::new(&cache)
            .with_me(|| 7)
            .with_current(|| Some(3)),
    );
    chats.replace(
        &[
            ChatRowDto::new(ChatDto {
                family_ids: Some(vec![3]),
                ..ChatDto::new(42, "family", "The Smiths")
            }),
            ChatRowDto::new(ChatDto {
                family_ids: Some(vec![5]),
                ..ChatDto::new(43, "family", "Ivanovs")
            }),
        ],
        None,
    )?;
    chats.replace_members_of(Some(5), &[MemberDto::new(11, "carl", "Carl")], &[])?;
    let rules = NotificationRules::new(chats, Rc::new(common::english()));
    rules.set_wanted(true);
    rules.set_family_name(Some("The Smiths"));
    rules.set_families(&[
        FamilyDto::new(3, "The Smiths"),
        FamilyDto::new(5, "Ivanovs"),
    ]);

    let toast = rules
        .for_message(&MessageDto::new(
            1,
            43,
            11,
            Some("c-1"),
            "hi",
            "2026-10-22T10:00:00Z",
        ))?
        .expect("said");
    assert_eq!("Ivanovs — Carl", toast.title);
    let note = rules
        .for_note(
            &NoteDto {
                kind: Some("text".to_string()),
                text: Some("Bread".to_string()),
                board_seq: 9,
                family_id: Some(5),
                ..NoteDto::new(2, 11)
            },
            true,
        )?
        .expect("said");
    assert_eq!("Ivanovs — Carl", note.title);
    assert_eq!(Some(5), note.family_id);
    // The wall's notification says whose, and a click reads it back.
    let arguments = for_toast(&note);
    assert_eq!(
        Some(NotificationTarget::Open(ToastTarget::board(Some(5)))),
        parse(arguments.action, arguments.target.as_ref())
    );
    Ok(())
}
