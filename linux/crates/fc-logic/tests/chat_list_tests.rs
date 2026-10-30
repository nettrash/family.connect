//! `ChatListModelTests`: the chat list — the order, the counts, and the one line under each name.
//!
//! A call's line is `fc_text::call_record`'s, read in the language `fc_text::i18n::use_lang` set —
//! English, a test thread's default.

mod common;

use std::rc::Rc;

use fc_core::protocol::{AttachmentDto, CallRecordDto, ChatDto, ChatRowDto, MemberDto, MessageDto};
use fc_core::store::{ChatStore, Database, SeqRoute, StoreResult, times};
use fc_logic::chat_list::{ChatListModel, ChatRow, RowTimeKind};
use fc_logic::groups::ChatFace;
use time::macros::datetime;
use time::{Duration, OffsetDateTime, UtcOffset};

const ME: i64 = 7;

const NOW: OffsetDateTime = datetime!(2026-09-12 12:00 UTC);

struct Rig {
    chats: Rc<ChatStore>,
    list: ChatListModel,
}

fn rig() -> StoreResult<Rig> {
    let cache = Database::open_in_memory()?;
    let chats = Rc::new(ChatStore::new(&cache));
    Ok(Rig {
        list: ChatListModel::new(chats.clone(), || ME, Rc::new(common::english())),
        chats,
    })
}

/// `Message(id, chat, body, sender, at, media, call)`, its instant through the product's OWN writer
/// — a fixture that formatted the wire's instants in the machine's language would hand the product
/// `16.00.00` on a Finnish computer and fail for the one reason that is not a bug.
struct Message {
    id: i64,
    chat: i64,
    body: &'static str,
    sender: i64,
    at: OffsetDateTime,
    media: Option<Vec<AttachmentDto>>,
    call: Option<CallRecordDto>,
}

impl Message {
    fn new(id: i64) -> Message {
        Message {
            id,
            chat: 42,
            body: "Dinner at 7?",
            sender: 9,
            at: NOW - Duration::minutes(5),
            media: None,
            call: None,
        }
    }

    fn dto(self) -> MessageDto {
        MessageDto {
            attachments: self.media,
            call: self.call,
            ..MessageDto::new(
                self.id,
                self.chat,
                self.sender,
                None,
                self.body,
                &times::rfc3339(Some(times::milliseconds(self.at))).expect("an instant"),
            )
        }
    }
}

impl Rig {
    fn row(&self, chat_id: i64) -> StoreResult<ChatRow> {
        Ok(self
            .list
            .rows(NOW)?
            .into_iter()
            .find(|row| row.chat.id == chat_id)
            .expect("on the list"))
    }

    fn apply(&self, message: Message) -> StoreResult<()> {
        self.chats
            .apply(&message.dto(), SeqRoute::CatchUpPage, true)?;
        Ok(())
    }

    fn ids(&self) -> StoreResult<Vec<i64>> {
        Ok(self.list.rows(NOW)?.iter().map(|row| row.chat.id).collect())
    }
}

#[test]
fn the_rows_are_in_the_order_something_last_happened_and_the_counts_are_the_servers_own()
-> StoreResult<()> {
    let rig = rig()?;
    rig.chats.replace(
        &[
            ChatRowDto {
                last_message: Some(
                    Message {
                        at: NOW - Duration::hours(3),
                        ..Message::new(1338)
                    }
                    .dto(),
                ),
                unread_count: 3,
                mentioned: Some(true),
                ..ChatRowDto::new(ChatDto::new(42, "family", "The Smiths"))
            },
            ChatRowDto {
                last_message: Some(
                    Message {
                        chat: 43,
                        at: NOW - Duration::minutes(2),
                        ..Message::new(1400)
                    }
                    .dto(),
                ),
                unread_count: 0,
                ..ChatRowDto::new(ChatDto {
                    peer_user_id: Some(11),
                    ..ChatDto::new(43, "direct", "Bob")
                })
            },
        ],
        None,
    )?;
    rig.apply(Message {
        at: NOW - Duration::hours(3),
        ..Message::new(1338)
    })?;
    rig.apply(Message {
        chat: 43,
        at: NOW - Duration::minutes(2),
        ..Message::new(1400)
    })?;

    let rows = rig.list.rows(NOW)?;

    // THE FAMILY CHAT IS FIRST even though the direct chat is the busier one: it is the room
    // everybody is in, and Apple and the web both pin it there.
    assert_eq!(
        vec![42, 43],
        rows.iter().map(|row| row.chat.id).collect::<Vec<_>>()
    );
    let family = &rows[0];
    assert_eq!("The Smiths", family.title);
    assert!(family.is_family());
    // Counted by the server, not by this device.
    assert_eq!(3, family.unread);
    assert!(family.mentioned);
    assert_eq!(RowTimeKind::Clock, family.when);
    Ok(())
}

/// Everything below the family chat goes newest conversation first, by the ID of its newest
/// message: ids are monotonic, where a stamp can be moved by a server's clock and is deliberately
/// not moved by an edit.
#[test]
fn below_the_family_chat_the_newest_conversation_is_first() -> StoreResult<()> {
    let rig = rig()?;
    rig.chats.replace(
        &[
            ChatRowDto::new(ChatDto::new(42, "family", "The Smiths")),
            ChatRowDto::new(ChatDto {
                peer_user_id: Some(11),
                ..ChatDto::new(43, "direct", "Bob")
            }),
            ChatRowDto::new(ChatDto {
                peer_user_id: Some(14),
                ..ChatDto::new(44, "direct", "Gran")
            }),
            ChatRowDto::new(ChatDto::new(45, "ai", "Assistant")),
        ],
        None,
    )?;
    rig.apply(Message {
        chat: 43,
        at: NOW - Duration::days(1),
        ..Message::new(100)
    })?;
    rig.apply(Message {
        chat: 44,
        at: NOW - Duration::minutes(1),
        ..Message::new(300)
    })?;

    // 44 has the newest message, 43 an older one, and the assistant's chat has none at all.
    assert_eq!(vec![42, 44, 43, 45], rig.ids()?);
    Ok(())
}

#[test]
fn a_chat_with_nothing_in_it_says_so_rather_than_showing_an_empty_line() -> StoreResult<()> {
    let rig = rig()?;
    rig.chats.replace(
        &[ChatRowDto::new(ChatDto::new(42, "family", "The Smiths"))],
        None,
    )?;

    let row = rig.row(42)?;

    assert_eq!("No messages yet", row.preview);
    assert_eq!(RowTimeKind::None, row.when);
    assert_eq!(None, row.at);
    Ok(())
}

#[test]
fn the_preview_is_the_first_line_and_no_more() -> StoreResult<()> {
    let rig = rig()?;
    rig.chats.replace(
        &[ChatRowDto::new(ChatDto::new(42, "family", "The Smiths"))],
        None,
    )?;
    rig.apply(Message {
        body: "first line\nsecond line\nthird",
        ..Message::new(1338)
    })?;

    // A row is one line high; a body with newlines in it would push every row below it down.
    assert_eq!("first line", rig.row(42)?.preview);
    Ok(())
}

#[test]
fn a_captionless_attachment_reads_as_what_it_is() -> StoreResult<()> {
    let rig = rig()?;
    rig.chats.replace(
        &[ChatRowDto::new(ChatDto::new(42, "family", "The Smiths"))],
        None,
    )?;
    let newest = |media: Vec<AttachmentDto>| {
        rig.apply(Message {
            body: "",
            media: Some(media),
            ..Message::new(1338)
        })
    };

    newest(vec![AttachmentDto {
        width: Some(1600),
        height: Some(1200),
        ..AttachmentDto::new(34, "photo")
    }])?;
    assert_eq!("Photo", rig.row(42)?.preview);

    newest(vec![
        AttachmentDto::new(34, "photo"),
        AttachmentDto::new(35, "photo"),
    ])?;
    assert_eq!("2 Photos", rig.row(42)?.preview);

    newest(vec![AttachmentDto::new(36, "video")])?;
    assert_eq!("Video", rig.row(42)?.preview);

    // The row for a message that IS the recording says so, where the attachment's own name would
    // only say "Audio".
    newest(vec![AttachmentDto::new(37, "audio")])?;
    assert_eq!("Voice message", rig.row(42)?.preview);

    newest(vec![AttachmentDto {
        latitude: Some(55.7558),
        longitude: Some(37.6173),
        ..AttachmentDto::new(38, "location")
    }])?;
    assert_eq!("Location", rig.row(42)?.preview);

    // A file IS its name.
    newest(vec![AttachmentDto {
        name: Some("receipts.pdf".to_string()),
        ..AttachmentDto::new(39, "file")
    }])?;
    assert_eq!("receipts.pdf", rig.row(42)?.preview);

    // And a kind this build never heard of still says something happened.
    newest(vec![AttachmentDto::new(40, "hologram")])?;
    assert_eq!("Photo", rig.row(42)?.preview);
    Ok(())
}

#[test]
fn attachments_ride_on_a_caption_rather_than_replacing_it() -> StoreResult<()> {
    let rig = rig()?;
    rig.chats.replace(
        &[ChatRowDto::new(ChatDto::new(42, "family", "The Smiths"))],
        None,
    )?;
    rig.apply(Message {
        body: "look at this",
        media: Some(vec![AttachmentDto::new(34, "photo")]),
        ..Message::new(1338)
    })?;

    assert_eq!("look at this", rig.row(42)?.preview);
    Ok(())
}

/// A CALL READS AS A CALL. The server writes an English placeholder into a record's body for clients
/// that predate calls, and a client that knows the object never shows it.
#[test]
fn a_call_record_reads_as_a_call_and_never_as_its_placeholder_body() -> StoreResult<()> {
    let rig = rig()?;
    rig.chats.replace(
        &[ChatRowDto::new(ChatDto {
            peer_user_id: Some(11),
            ..ChatDto::new(43, "direct", "Bob")
        })],
        None,
    )?;
    // A record is immutable and each call is its own message, so each one gets its own id: an upsert
    // never rewrites a message's sender, and nor does a real server.
    let record = |id: i64, outcome: &str, seconds: Option<i32>, sender: i64| {
        rig.apply(Message {
            chat: 43,
            body: "Voice call",
            sender,
            call: Some(CallRecordDto {
                outcome: outcome.to_string(),
                duration_secs: seconds,
                ..CallRecordDto::default()
            }),
            ..Message::new(id)
        })
    };

    record(1500, "completed", Some(222), 11)?;
    assert_eq!("Voice call · 3:42", rig.row(43)?.preview);

    // Whose call it was changes the sentence: mine that nobody answered…
    record(1501, "missed", None, ME)?;
    assert_eq!("No answer", rig.row(43)?.preview);

    // …and theirs that I never answered.
    record(1502, "missed", None, 11)?;
    assert_eq!("Missed voice call", rig.row(43)?.preview);
    Ok(())
}

/// A BLOCKED MEMBER'S MESSAGE IS THE HIDDEN ROW HERE TOO, with no name and no reveal: a list is not
/// where a person peeks. In the family chat it still counts, because the count is the other half of
/// the read marker.
#[test]
fn a_blocked_senders_preview_is_the_hidden_row_and_the_count_stands() -> StoreResult<()> {
    let rig = rig()?;
    rig.chats.replace(
        &[ChatRowDto {
            unread_count: 2,
            ..ChatRowDto::new(ChatDto::new(42, "family", "The Smiths"))
        }],
        None,
    )?;
    rig.apply(Message {
        body: "something unpleasant",
        sender: 11,
        ..Message::new(1338)
    })?;
    rig.chats.set_blocked(11, true)?;

    let row = rig.row(42)?;

    assert!(row.hidden);
    assert_eq!("Hidden — blocked member", row.preview);
    assert_eq!(2, row.unread);

    // My own words are never hidden from me, whoever else I have blocked.
    rig.apply(Message {
        body: "mine",
        sender: ME,
        ..Message::new(1339)
    })?;
    assert!(!rig.row(42)?.hidden);
    assert_eq!("mine", rig.row(42)?.preview);

    // Not even if this device somehow held its OWN id on the block list. The server refuses a
    // self-block (`cannot_block_self`), so this is insurance against a cache rather than a reachable
    // state — and insurance that is never checked is a comment.
    rig.chats.set_blocked(ME, true)?;
    assert!(!rig.row(42)?.hidden);
    assert_eq!("mine", rig.row(42)?.preview);
    Ok(())
}

#[test]
fn a_direct_chat_with_no_title_falls_back_to_the_roster_and_then_to_a_word() -> StoreResult<()> {
    let rig = rig()?;
    let untitled = ChatRowDto::new(ChatDto {
        peer_user_id: Some(11),
        ..ChatDto::new(43, "direct", "")
    });
    rig.chats.replace(std::slice::from_ref(&untitled), None)?;
    assert_eq!("Chat", rig.row(43)?.title);

    rig.chats.replace_members(
        &[MemberDto {
            role: Some("member".to_string()),
            ..MemberDto::new(11, "bob", "Bob")
        }],
        &[],
    )?;
    assert_eq!("Bob", rig.row(43)?.title);

    // And the wire's own title wins when it has one: the server recomputes it per read.
    rig.chats.replace(
        &[ChatRowDto::new(ChatDto {
            peer_user_id: Some(11),
            ..ChatDto::new(43, "direct", "Bobby")
        })],
        None,
    )?;
    assert_eq!("Bobby", rig.row(43)?.title);
    Ok(())
}

/// Days are compared in the READER's zone: a message sent at 23:30 last night is "Yesterday" to the
/// person reading it, whatever the clock says in Greenwich. (The C# read the machine's zone; here
/// it is given — and a zone far from UTC is checked too.)
#[test]
fn the_row_time_is_a_decision_and_not_a_format() {
    for zone in [
        UtcOffset::UTC,
        UtcOffset::from_hms(9, 30, 0).expect("an offset"),
    ] {
        let now = NOW.to_offset(zone);
        let when = |at: OffsetDateTime| ChatListModel::when(at, now, &zone);
        assert_eq!(RowTimeKind::Clock, when(now - Duration::hours(2)));
        assert_eq!(RowTimeKind::Yesterday, when(now - Duration::days(1)));
        assert_eq!(RowTimeKind::Weekday, when(now - Duration::days(3)));
        assert_eq!(RowTimeKind::Date, when(now - Duration::days(20)));
        // A stamp from the future is not "in six days": it is a date.
        assert_eq!(RowTimeKind::Date, when(now + Duration::days(1)));
    }
    // 23:30 last night in the reader's zone is yesterday, though it is today in Greenwich.
    let tokyo = UtcOffset::from_hms(9, 0, 0).expect("an offset");
    assert_eq!(
        RowTimeKind::Yesterday,
        ChatListModel::when(
            datetime!(2026-09-12 23:30 +09:00),
            datetime!(2026-09-13 08:00 +09:00),
            &tokyo
        )
    );
    assert_eq!(
        RowTimeKind::Clock,
        ChatListModel::when(
            datetime!(2026-09-12 23:30 +09:00),
            datetime!(2026-09-13 08:00 +09:00),
            &UtcOffset::UTC
        )
    );
}

#[test]
fn a_hidden_chat_is_not_on_the_list_at_all() -> StoreResult<()> {
    let rig = rig()?;
    rig.chats.replace(
        &[
            ChatRowDto::new(ChatDto::new(42, "family", "The Smiths")),
            ChatRowDto::new(ChatDto {
                peer_user_id: Some(11),
                ..ChatDto::new(43, "direct", "Bob")
            }),
        ],
        None,
    )?;
    rig.chats.set_blocked(11, true)?;

    assert_eq!(vec![42], rig.ids()?);
    Ok(())
}

/// A GROUP IS A ROW LIKE ANY CHAT, below the family chat and ordered by its last message
/// (docs/protocol.md, "Groups"; docs/groups-2026-10-21.md, §6): its name is its title, and its circle
/// is the people glyph rather than initials.
#[test]
fn a_group_is_listed_below_the_family_by_its_last_message() -> StoreResult<()> {
    let rig = rig()?;
    let at = |id: i64, chat: i64, ago: Duration| Message {
        chat,
        at: NOW - ago,
        ..Message::new(id)
    };
    rig.chats.replace(
        &[
            ChatRowDto {
                last_message: Some(at(1300, 42, Duration::hours(5)).dto()),
                ..ChatRowDto::new(ChatDto::new(42, "family", "The Smiths"))
            },
            ChatRowDto {
                last_message: Some(at(1400, 43, Duration::hours(1)).dto()),
                ..ChatRowDto::new(ChatDto {
                    peer_user_id: Some(11),
                    ..ChatDto::new(43, "direct", "Bob")
                })
            },
            ChatRowDto {
                last_message: Some(at(1500, 77, Duration::minutes(10)).dto()),
                unread_count: 2,
                ..ChatRowDto::new(ChatDto {
                    member_ids: Some(vec![ME, 9, 11]),
                    admin_user_id: Some(9),
                    ..ChatDto::new(77, "group", "Cousins")
                })
            },
            ChatRowDto::new(ChatDto {
                member_ids: Some(vec![ME, 12]),
                admin_user_id: Some(ME),
                ..ChatDto::new(78, "group", "Book club")
            }),
        ],
        None,
    )?;
    rig.apply(at(1300, 42, Duration::hours(5)))?;
    rig.apply(at(1400, 43, Duration::hours(1)))?;
    rig.apply(at(1500, 77, Duration::minutes(10)))?;

    // The busiest chat is a group, and the family chat is still first.
    assert_eq!(vec![42, 77, 43, 78], rig.ids()?);
    let group = rig.row(77)?;
    assert!(group.is_group());
    assert!(!(group.is_family() || group.is_direct() || group.is_assistant()));
    assert_eq!("Cousins", group.title);
    assert_eq!(2, group.unread);
    assert_eq!(ChatFace::People, group.face());
    assert_eq!(ChatFace::House, rig.row(42)?.face());
    assert_eq!(ChatFace::Initials, rig.row(43)?.face());
    // A group with nothing in it yet says so, as any chat does.
    assert_eq!("No messages yet", rig.row(78)?.preview);
    Ok(())
}
