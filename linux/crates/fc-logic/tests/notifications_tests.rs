//! `NotificationRulesTests`: when this client speaks up, and what it says when it does — and that
//! what it says is never the message.

mod common;

use std::rc::Rc;

use fc_core::protocol::{
    ChatDto, ChatRowDto, MemberDto, MentionDto, MessageDto, NoteDto, ReplyToDto,
};
use fc_core::store::{ChatStore, Database, SeqRoute, StoreResult};
use fc_logic::notifications::NotificationRules;

const ME: i64 = 7;
const ASSISTANT: i64 = 1;

struct Rig {
    chats: Rc<ChatStore>,
    rules: NotificationRules,
}

impl Rig {
    fn new() -> StoreResult<Rig> {
        let cache = Database::open_in_memory()?;
        let chats = Rc::new(ChatStore::new(&cache).with_me(|| ME));
        chats.replace(
            &[
                ChatRowDto::new(ChatDto::new(42, "family", "The Smiths")),
                ChatRowDto::new(ChatDto {
                    peer_user_id: Some(11),
                    ..ChatDto::new(43, "direct", "Bob")
                }),
            ],
            None,
        )?;
        chats.replace_members(
            &[
                MemberDto {
                    role: Some("owner".to_string()),
                    ..MemberDto::new(7, "anna", "Anna")
                },
                MemberDto {
                    role: Some("member".to_string()),
                    ..MemberDto::new(11, "bob", "Bob")
                },
            ],
            &[],
        )?;
        let rules = NotificationRules::new(chats.clone(), Rc::new(common::english()));
        rules.set_wanted(true);
        rules.set_in_front(false);
        rules.set_family_name(Some("The Smiths"));
        rules.set_assistant_user_id(Some(ASSISTANT));
        Ok(Rig { chats, rules })
    }
}

/// The C#'s `Message(id, chat, sender, body, mentions, replyTo)`.
struct Message {
    id: i64,
    chat: i64,
    sender: i64,
    body: &'static str,
    mentions: Option<Vec<MentionDto>>,
    reply_to: Option<ReplyToDto>,
}

impl Message {
    fn new(id: i64) -> Self {
        Message {
            id,
            chat: 42,
            sender: 11,
            body: "Dinner at 7?",
            mentions: None,
            reply_to: None,
        }
    }

    fn dto(self) -> MessageDto {
        MessageDto {
            mentions: self.mentions,
            reply_to: self.reply_to,
            ..MessageDto::new(
                self.id,
                self.chat,
                self.sender,
                None,
                self.body,
                "2026-09-12T10:00:00Z",
            )
        }
    }
}

fn message(id: i64) -> MessageDto {
    Message::new(id).dto()
}

fn reply(message_id: i64, sender_id: i64, excerpt: &str) -> ReplyToDto {
    ReplyToDto {
        message_id,
        sender_id,
        excerpt: excerpt.to_string(),
        ..ReplyToDto::default()
    }
}

#[test]
fn the_family_chat_names_the_family_first_and_the_body_is_never_the_message() -> StoreResult<()> {
    let rig = Rig::new()?;
    let toast = rig.rules.for_message(&message(1338))?.expect("said");

    // A notification centre shows a list, and "Bob" alone does not say where he said it.
    assert_eq!("The Smiths — Bob", toast.title);
    assert_eq!("New message", toast.body);
    assert!(!toast.body.contains("Dinner"));
    assert_eq!("chat-42", toast.tag);
    assert_eq!(Some(42), toast.chat_id);
    Ok(())
}

#[test]
fn a_direct_chat_is_the_sender_alone() -> StoreResult<()> {
    let rig = Rig::new()?;
    let toast = rig
        .rules
        .for_message(
            &Message {
                chat: 43,
                ..Message::new(1338)
            }
            .dto(),
        )?
        .expect("said");

    assert_eq!("Bob", toast.title);
    Ok(())
}

#[test]
fn a_message_that_names_the_reader_says_so_in_the_title() -> StoreResult<()> {
    let rig = Rig::new()?;
    let toast = rig
        .rules
        .for_message(
            &Message {
                mentions: Some(vec![MentionDto::new(ME, "Anna")]),
                ..Message::new(1338)
            }
            .dto(),
        )?
        .expect("said");

    // The same notification, a different title — never a second one.
    assert_eq!("The Smiths — Bob mentioned you", toast.title);
    Ok(())
}

#[test]
fn nothing_is_said_while_the_reader_is_looking_or_has_not_asked() -> StoreResult<()> {
    let rig = Rig::new()?;
    rig.rules.set_in_front(true);
    assert_eq!(None, rig.rules.for_message(&message(1338))?);

    rig.rules.set_in_front(false);
    rig.rules.set_wanted(false);
    assert_eq!(None, rig.rules.for_message(&message(1338))?);

    rig.rules.set_wanted(true);
    assert!(rig.rules.for_message(&message(1338))?.is_some());
    Ok(())
}

#[test]
fn your_own_message_from_your_other_device_says_nothing() -> StoreResult<()> {
    let rig = Rig::new()?;
    assert_eq!(
        None,
        rig.rules.for_message(
            &Message {
                sender: ME,
                ..Message::new(1338)
            }
            .dto()
        )?
    );
    Ok(())
}

/// THE BLOCK REACHES ONE STEP FURTHER THAN THE SENDER: the assistant's answer to a blocked member's
/// question would light up a notification for a thread its reader cannot read.
#[test]
fn a_blocked_members_message_and_the_assistants_answer_to_it_both_say_nothing() -> StoreResult<()> {
    let rig = Rig::new()?;
    rig.chats.set_blocked(11, true)?;

    assert_eq!(
        None,
        rig.rules.for_message(
            &Message {
                sender: 11,
                ..Message::new(1338)
            }
            .dto()
        )?
    );
    assert_eq!(
        None,
        rig.rules.for_message(
            &Message {
                sender: ASSISTANT,
                reply_to: Some(reply(1338, 11, "what is for dinner")),
                ..Message::new(1339)
            }
            .dto()
        )?
    );

    // The assistant answering anybody else still speaks.
    assert!(
        rig.rules
            .for_message(
                &Message {
                    sender: ASSISTANT,
                    reply_to: Some(reply(1337, ME, "what is for dinner")),
                    ..Message::new(1340)
                }
                .dto()
            )?
            .is_some()
    );
    Ok(())
}

#[test]
fn a_message_in_a_chat_this_device_does_not_hold_says_nothing() -> StoreResult<()> {
    let rig = Rig::new()?;
    // A notification that cannot be opened is worse than none: the list read that names this chat
    // has not landed yet.
    assert_eq!(
        None,
        rig.rules.for_message(
            &Message {
                chat: 99,
                ..Message::new(1338)
            }
            .dto()
        )?
    );
    Ok(())
}

#[test]
fn a_sender_the_roster_cannot_name_is_somebody() -> StoreResult<()> {
    let rig = Rig::new()?;
    let toast = rig
        .rules
        .for_message(
            &Message {
                sender: 14,
                ..Message::new(1338)
            }
            .dto(),
        )?
        .expect("said");

    assert_eq!("The Smiths — Someone", toast.title);
    Ok(())
}

/// A NOTE IS NEWS ONLY WHEN IT SAYS SOMETHING NEW. The badge and the notification answer the same
/// question, so the board's own verdict decides both.
#[test]
fn a_note_speaks_only_when_the_board_says_it_is_news() -> StoreResult<()> {
    let rig = Rig::new()?;
    let note = NoteDto {
        kind: Some("text".to_string()),
        text: Some("Bins on Tuesday".to_string()),
        board_seq: 11,
        content_seq: Some(11),
        ..NoteDto::new(12, 11)
    };

    let toast = rig.rules.for_note(&note, true)?.expect("said");
    assert_eq!("The Smiths — Bob", toast.title);
    assert_eq!("New note", toast.body);
    assert_eq!("board", toast.tag);
    assert!(toast.is_board());

    // Dragged across the wall: no badge, and nothing said.
    assert_eq!(None, rig.rules.for_note(&note, false)?);
    // Mine, and a blocked member's.
    assert_eq!(
        None,
        rig.rules.for_note(
            &NoteDto {
                author_id: ME,
                ..note.clone()
            },
            true
        )?
    );
    rig.chats.set_blocked(11, true)?;
    assert_eq!(None, rig.rules.for_note(&note, true)?);
    Ok(())
}

#[test]
fn the_window_title_carries_the_count() -> StoreResult<()> {
    let rig = Rig::new()?;
    assert_eq!("Family Connect", rig.rules.window_title("Family Connect")?);

    rig.chats.apply(&message(1338), SeqRoute::LiveFrame, true)?;
    rig.chats.apply(&message(1339), SeqRoute::LiveFrame, true)?;
    assert_eq!(
        "(2) Family Connect",
        rig.rules.window_title("Family Connect")?
    );
    Ok(())
}

/// A GROUP is titled by its own name in place of the family's — "<Group> — <Sender>", and "…
/// mentioned you" for a member it names (docs/protocol.md, "Push notifications", 2026-10-22).
#[test]
fn a_group_names_the_group_first() -> StoreResult<()> {
    let rig = Rig::new()?;
    rig.chats.upsert(&ChatDto {
        member_ids: Some(vec![ME, 11]),
        admin_user_id: Some(11),
        ..ChatDto::new(77, "group", "Cousins")
    })?;

    assert_eq!(
        "Cousins — Bob",
        rig.rules
            .for_message(
                &Message {
                    chat: 77,
                    ..Message::new(1500)
                }
                .dto()
            )?
            .expect("said")
            .title
    );
    assert_eq!(
        "Cousins — Bob mentioned you",
        rig.rules
            .for_message(
                &Message {
                    chat: 77,
                    body: "@Anna 7?",
                    mentions: Some(vec![MentionDto::new(ME, "Anna")]),
                    ..Message::new(1501)
                }
                .dto()
            )?
            .expect("said")
            .title
    );
    assert_eq!(
        "chat-77",
        rig.rules
            .for_message(
                &Message {
                    chat: 77,
                    ..Message::new(1502)
                }
                .dto()
            )?
            .expect("said")
            .tag
    );
    Ok(())
}
