//! The chats and what was said in them: the guards that decide whether an answer may overwrite
//! what this device holds (docs/protocol.md, "Editing", "Reactions", "Threads") — a port of
//! `win/tests/FamilyConnect.Core.Tests/Store/ChatStoreTests.cs`.
//!
//! The C# helpers `Row(...)` and `Message(...)` take named optional arguments; here they are
//! [`row`] and [`message`] with struct-update syntax for the rest, and [`edit`] for the two fields
//! an edit sets together.

use std::collections::BTreeMap;
use std::rc::Rc;

use fc_core::protocol::{
    AttachmentDto, BirthdayDto, ChatDto, ChatRowDto, MemberDto, MentionDto, MessageDto, PollDto,
    PollOptionDto, QuoteParentDto, ReactionDto, ReplyToDto, UserDto, wire,
};
use fc_core::store::{ChatStore, Database, SeqRoute, StoreResult};

use SeqRoute::{CatchUpPage, Evidence, LiveFrame};

fn open() -> StoreResult<Rc<Database>> {
    Database::open_in_memory()
}

fn family() -> ChatDto {
    ChatDto::new(42, "family", "The Smiths")
}

/// `Row()`: the family chat, nothing unread, nothing read.
fn row() -> ChatRowDto {
    ChatRowDto::new(family())
}

/// `Row(chat)`.
fn row_of(chat: ChatDto) -> ChatRowDto {
    ChatRowDto::new(chat)
}

/// `Message(id)`: in the family chat, from 7, at 10:00, never edited.
fn message(id: i64) -> MessageDto {
    MessageDto::new(id, 42, 7, None, "Dinner at 7?", "2026-09-12T10:00:00Z")
}

/// `Message(id, body: body, editSeq: seq)`: an edit carries its stamp with its seq.
fn edit(id: i64, body: &str, edit_seq: i64) -> MessageDto {
    MessageDto {
        body: body.to_string(),
        edit_seq: Some(edit_seq),
        edited_at: Some("2026-09-12T10:05:00Z".to_string()),
        ..message(id)
    }
}

/// `Message(id, threadRoot: root)`.
fn reply(id: i64, root: i64) -> MessageDto {
    MessageDto {
        thread_root_id: Some(root),
        ..message(id)
    }
}

fn option(id: i64, text: &str, votes: &[i64]) -> PollOptionDto {
    PollOptionDto {
        id,
        text: text.to_string(),
        votes: votes.to_vec(),
    }
}

fn ids(messages: &[MessageDto]) -> Vec<i64> {
    messages.iter().map(|message| message.id).collect()
}

fn chat_ids(rows: &[ChatRowDto]) -> Vec<i64> {
    rows.iter().map(|row| row.chat.id).collect()
}

fn held(store: &ChatStore, message_id: i64) -> StoreResult<MessageDto> {
    Ok(store.message(message_id)?.expect("a held message"))
}

fn chat(store: &ChatStore, chat_id: i64) -> StoreResult<ChatRowDto> {
    Ok(store.chat(chat_id)?.expect("a held chat"))
}

fn member(id: i64, username: &str, name: &str, role: &str) -> MemberDto {
    MemberDto {
        role: Some(role.to_string()),
        ..MemberDto::new(id, username, name)
    }
}

/// The unanswered count reads every poll a chat holds, oldest first, and nothing else — not another
/// chat's.
#[test]
fn a_chats_polls_are_every_poll_it_holds_oldest_first() -> StoreResult<()> {
    let database = open()?;
    let store = ChatStore::new(&database);
    store.replace(&[row(), row_of(ChatDto::new(43, "direct", "Bob"))], None)?;
    let poll = PollDto {
        poll_seq: 1,
        closed: false,
        options: vec![option(1, "Pizza", &[]), option(2, "Pasta", &[])],
    };
    store.apply_page(
        &[
            MessageDto {
                poll: Some(poll.clone()),
                ..message(9)
            },
            MessageDto {
                poll: Some(PollDto {
                    closed: true,
                    ..poll.clone()
                }),
                ..message(3)
            },
            message(5),
            MessageDto {
                chat_id: 43,
                poll: Some(poll.clone()),
                ..message(7)
            },
        ],
        CatchUpPage,
        true,
    )?;

    assert_eq!(ids(&store.polls(42)?), [3, 9]);
    assert!(store.polls(42)?[0].poll.as_ref().expect("a poll").closed);
    assert!(store.polls(44)?.is_empty());
    Ok(())
}

/// A new reply raises its held root's count ONCE — on a live frame, the after_id catch-up or the
/// answer to this device's own send — and a page of history, an edit or a preview never does: the
/// root's recomputed copy already includes those (docs/protocol.md, "Threads").
#[test]
fn a_new_reply_raises_its_roots_count_once_and_only_on_a_route_that_counts() -> StoreResult<()> {
    let database = open()?;
    let store = ChatStore::new(&database);
    store.replace(&[row()], None)?;
    store.apply(
        &MessageDto {
            reply_count: Some(1),
            ..message(10)
        },
        Evidence,
        true,
    )?;

    store.apply(&reply(11, 10), LiveFrame, true)?;
    assert_eq!(held(&store, 10)?.reply_count, Some(2));
    store.apply(&reply(11, 10), LiveFrame, true)?;
    assert_eq!(held(&store, 10)?.reply_count, Some(2));

    store.apply_page(&[reply(12, 10)], CatchUpPage, true)?;
    assert_eq!(held(&store, 10)?.reply_count, Some(3));

    store.apply_page(&[reply(13, 10)], Evidence, true)?;
    store.replace(
        &[ChatRowDto {
            last_message: Some(reply(14, 10)),
            ..row()
        }],
        None,
    )?;
    assert_eq!(held(&store, 10)?.reply_count, Some(3));
    Ok(())
}

/// A root nobody had answered starts at one; a root this device does not hold, or in another chat,
/// is not raised.
#[test]
fn a_reply_counts_only_against_a_held_root_in_its_own_chat() -> StoreResult<()> {
    let database = open()?;
    let store = ChatStore::new(&database);
    store.replace(&[row(), row_of(ChatDto::new(43, "direct", "Bob"))], None)?;
    store.apply(&message(10), Evidence, true)?;
    store.apply(
        &MessageDto {
            chat_id: 43,
            ..message(20)
        },
        Evidence,
        true,
    )?;

    store.apply(&reply(11, 10), LiveFrame, true)?;
    store.apply(&reply(12, 20), LiveFrame, true)?;
    store.apply(&reply(13, 99), LiveFrame, true)?;

    assert_eq!(held(&store, 10)?.reply_count, Some(1));
    assert_eq!(held(&store, 20)?.reply_count, None);
    assert_eq!(store.message(99)?, None);
    Ok(())
}

/// A thread read refreshes what the cache HOLDS — the true count, an edit — and leaves the rest out
/// of it.
#[test]
fn a_refresh_touches_only_held_rows() -> StoreResult<()> {
    let database = open()?;
    let store = ChatStore::new(&database);
    store.replace(&[row()], None)?;
    store.apply_page(
        &[
            MessageDto {
                reply_count: Some(1),
                ..message(10)
            },
            message(50),
        ],
        Evidence,
        true,
    )?;

    let changed = store.refresh(&[
        MessageDto {
            reply_count: Some(3),
            ..edit(10, "Dinner at 8?", 2)
        },
        reply(11, 10),
        reply(300, 10),
    ])?;

    assert_eq!(changed, 1);
    let root = held(&store, 10)?;
    assert_eq!(
        (root.reply_count, root.body.as_str()),
        (Some(3), "Dinner at 8?")
    );
    assert_eq!(store.message(11)?, None);
    assert_eq!(store.message(300)?, None);
    assert_eq!(ids(&store.messages(42, None, 50)?), [50, 10]);
    Ok(())
}

/// THE CATCH-UP CURSOR IS WHAT WAS DELIVERED IN SEQUENCE — a page or a frame — and nothing else
/// (docs/protocol.md, "Best-effort delivery", step 3). A list preview, the REST answer to a send
/// and a thread read are each one message with nothing known under it; counted, the next
/// `after_id` skips everything between.
#[test]
fn only_what_was_delivered_in_sequence_moves_the_catch_up_cursor() -> StoreResult<()> {
    let database = open()?;
    let store = ChatStore::new(&database);
    let bob = ChatDto::new(43, "direct", "Bob");
    store.replace(&[row(), row_of(bob.clone())], None)?;
    assert_eq!(store.catch_up_cursor(42)?, None);

    store.apply_page(&[message(1), message(10)], CatchUpPage, true)?;
    assert_eq!(store.catch_up_cursor(42)?, Some(10));

    // The list read's preview is held, and draws the row — and is not the cursor.
    store.replace(
        &[
            ChatRowDto {
                last_message: Some(message(20)),
                ..row()
            },
            ChatRowDto {
                last_message: Some(MessageDto {
                    chat_id: 43,
                    ..message(30)
                }),
                ..row_of(bob)
            },
        ],
        None,
    )?;
    assert_eq!(store.newest(42)?.map(|newest| newest.id), Some(20));
    assert_eq!(store.catch_up_cursor(42)?, Some(10));
    // A chat holding nothing but its preview has no cursor at all.
    assert_eq!(store.catch_up_cursor(43)?, None);

    // The answer to a send, and a thread read that refreshes the preview itself: neither.
    store.apply(&message(21), CatchUpPage, false)?;
    assert_eq!(store.refresh(&[edit(20, "Dinner at 8?", 3)])?, 1);
    assert_eq!(held(&store, 20)?.body, "Dinner at 8?");
    assert_eq!(store.catch_up_cursor(42)?, Some(10));

    // A page that delivers them puts them in sequence, and a later preview cannot take that back.
    store.apply_page(
        &[edit(20, "Dinner at 8?", 3), message(21)],
        CatchUpPage,
        true,
    )?;
    assert_eq!(store.catch_up_cursor(42)?, Some(21));
    store.replace(
        &[ChatRowDto {
            last_message: Some(edit(21, "Dinner at 9?", 4)),
            ..row()
        }],
        None,
    )?;
    assert_eq!(held(&store, 21)?.body, "Dinner at 9?");
    assert_eq!(store.catch_up_cursor(42)?, Some(21));

    assert_eq!(store.catch_up_cursors()?, BTreeMap::from([(42, 21)]));
    Ok(())
}

/// A QUOTE SURVIVES THE CACHE. `reply_to` is why a client can draw a reply without holding the
/// message it answers (docs/protocol.md, "Replies") — so dropping it on the way into the cache
/// leaves every reply drawn as "Someone" with no words, because everything this window draws it
/// reads back from here.
#[test]
fn a_quote_is_kept_whole_both_levels_of_it() -> StoreResult<()> {
    let database = open()?;
    let store = ChatStore::new(&database);
    store.replace(&[row()], None)?;
    let quote = ReplyToDto {
        message_id: 10,
        sender_id: 7,
        excerpt: "Dinner at 7?".to_string(),
        parent: Some(QuoteParentDto {
            message_id: 9,
            sender_id: 11,
            excerpt: "What time?".to_string(),
        }),
    };
    store.apply_page(
        &[MessageDto {
            reply_to: Some(quote.clone()),
            ..message(11)
        }],
        CatchUpPage,
        true,
    )?;

    assert_eq!(held(&store, 11)?.reply_to, Some(quote.clone()));
    let page = store.messages(42, None, 10)?;
    let only: Vec<&MessageDto> = page.iter().filter(|held| held.id == 11).collect();
    assert_eq!(only.len(), 1);
    assert_eq!(only[0].reply_to, Some(quote));
    Ok(())
}

/// The server RECOMPUTES a quote on every read, so a newer answer's copy wins — and an answer that
/// carries no quote at all (a chat-list preview) leaves the held one alone, which is the same rule
/// the reactions and the attachments follow.
#[test]
fn a_newer_quote_wins_and_a_preview_without_one_changes_nothing() -> StoreResult<()> {
    let database = open()?;
    let store = ChatStore::new(&database);
    store.replace(&[row()], None)?;
    let quoting = |excerpt: &str| MessageDto {
        reply_to: Some(ReplyToDto {
            message_id: 10,
            sender_id: 7,
            excerpt: excerpt.to_string(),
            parent: None,
        }),
        ..message(11)
    };
    store.apply_page(&[quoting("Dinner at 7?")], CatchUpPage, true)?;
    // The quoted message was edited: the excerpt comes back different.
    store.apply_page(&[quoting("Dinner at 8?")], CatchUpPage, true)?;
    let excerpt = |store: &ChatStore| -> StoreResult<String> {
        Ok(held(store, 11)?.reply_to.expect("a quote").excerpt)
    };
    assert_eq!(excerpt(&store)?, "Dinner at 8?");

    store.replace(
        &[ChatRowDto {
            last_message: Some(MessageDto {
                body: "yes".to_string(),
                ..message(11)
            }),
            ..row()
        }],
        None,
    )?;
    assert_eq!(excerpt(&store)?, "Dinner at 8?");
    Ok(())
}

#[test]
fn the_list_is_what_the_server_answered() -> StoreResult<()> {
    let database = open()?;
    let store = ChatStore::new(&database);
    let anna = ChatDto {
        peer_user_id: Some(9),
        ..ChatDto::new(43, "direct", "Anna")
    };
    store.replace(&[row(), row_of(anna.clone())], None)?;
    let mut listed = chat_ids(&store.chats()?);
    listed.sort_unstable();
    assert_eq!(listed, [42, 43]);

    // A direct chat that is no longer listed is one the reader has BLOCKED. It leaves the LIST and
    // nothing else: the messages stay, the marker stays, and an unblock brings the whole
    // conversation back rather than an empty chat.
    store.apply(
        &MessageDto {
            chat_id: 43,
            body: "hello".to_string(),
            ..message(1)
        },
        CatchUpPage,
        true,
    )?;
    store.mark_read(43, 1)?;
    store.replace(&[row()], None)?;
    assert_eq!(store.chats()?.len(), 1);
    assert!(!store.is_listed(43)?);
    assert_eq!(held(&store, 1)?.body, "hello");
    assert_eq!(chat(&store, 43)?.last_read_message_id, 1);
    // And it counts towards nothing while it is hidden.
    store.apply(
        &MessageDto {
            chat_id: 43,
            body: "and another".to_string(),
            ..message(2)
        },
        CatchUpPage,
        true,
    )?;
    assert_eq!(store.unread()?, 0);

    // Unblocked: the list carries it again, whole.
    store.replace(&[row(), row_of(anna)], None)?;
    assert!(store.is_listed(43)?);
    assert_eq!(store.chats()?.len(), 2);
    assert_eq!(
        chat(&store, 43)?.last_message.expect("a preview").body,
        "and another"
    );
    Ok(())
}

#[test]
fn a_message_goes_in_and_comes_back_whole() -> StoreResult<()> {
    let database = open()?;
    let store = ChatStore::new(&database);
    store.replace(&[row()], None)?;
    store.apply(
        &MessageDto {
            attachments: Some(vec![AttachmentDto {
                mime: Some("image/jpeg".to_string()),
                width: Some(600),
                height: Some(1200),
                ..AttachmentDto::new(34, "photo")
            }]),
            attachment: Some(AttachmentDto {
                mime: Some("image/jpeg".to_string()),
                ..AttachmentDto::new(34, "photo")
            }),
            mentions: Some(vec![MentionDto::new(9, "Anna")]),
            reactions: Some(vec![ReactionDto::new(9, "❤️")]),
            reaction_seq: Some(123),
            ..message(1338)
        },
        CatchUpPage,
        true,
    )?;
    let held = held(&store, 1338)?;
    assert_eq!(held.body, "Dinner at 7?");
    assert_eq!(held.created_at, "2026-09-12T10:00:00.000Z");
    assert_eq!(held.media().len(), 1);
    assert_eq!(held.media()[0].id, 34);
    let mentions = held.mentions.as_deref().expect("mentions");
    assert_eq!(mentions.len(), 1);
    assert_eq!(mentions[0].name, "Anna");
    assert_eq!(held.reaction_seq, Some(123));
    // A message with no poll and no call has neither field, not empty ones.
    assert_eq!(held.poll, None);
    assert_eq!(held.call, None);
    Ok(())
}

/// A CIRCLE IS STILL A CIRCLE OUT OF THE CACHE. Everything this window draws it reads back from
/// here, so a flag dropped on the way in would draw every video message square from the second look
/// on (S5.8).
#[test]
fn a_video_message_comes_back_round() -> StoreResult<()> {
    let database = open()?;
    let store = ChatStore::new(&database);
    store.replace(&[row()], None)?;
    let video = AttachmentDto {
        mime: Some("video/mp4".to_string()),
        size: Some(1_649_700),
        width: Some(480),
        height: Some(480),
        duration_ms: Some(23_400),
        has_preview: true,
        round: true,
        ..AttachmentDto::new(91, "video")
    };
    store.apply(
        &MessageDto {
            body: String::new(),
            attachments: Some(vec![video.clone()]),
            attachment: Some(video.clone()),
            ..message(1341)
        },
        CatchUpPage,
        true,
    )?;
    store.apply(
        &MessageDto {
            body: String::new(),
            attachments: Some(vec![AttachmentDto {
                id: 92,
                round: false,
                ..video
            }]),
            ..message(1342)
        },
        CatchUpPage,
        true,
    )?;

    let circle = held(&store, 1341)?;
    let round = circle.round_video().expect("a circle");
    assert_eq!(round.id, 91);
    assert_eq!(round.duration_ms, Some(23_400));
    assert_eq!(held(&store, 1342)?.round_video(), None);
    Ok(())
}

/// APPLYING AN EDIT IS GUARDED. A history page fetched BEFORE an edit but delivered after it would
/// otherwise restore the old text, and two devices in a family disagree about what was said.
#[test]
fn an_older_copy_never_restores_the_old_text() -> StoreResult<()> {
    let database = open()?;
    let store = ChatStore::new(&database);
    store.replace(&[row()], None)?;
    store.apply(&message(1338), CatchUpPage, true)?;
    assert!(store.apply(&edit(1338, "Dinner at 8?", 88), CatchUpPage, true)?);
    assert_eq!(held(&store, 1338)?.body, "Dinner at 8?");

    // The page that was already in flight, carrying the pre-edit body and no seq.
    assert!(!store.apply(&message(1338), CatchUpPage, true)?);
    assert_eq!(held(&store, 1338)?.body, "Dinner at 8?");
    // And an edit at the SAME seq still applies: the guard is "at least", because the same edit
    // may arrive by two paths and the whole message comes with it.
    assert!(store.apply(&edit(1338, "Dinner at 8?", 88), CatchUpPage, true)?);
    Ok(())
}

/// The one edit that changes more than the body: the assistant's reply gains its picture as an
/// attachment through exactly this path, so the WHOLE message is applied.
#[test]
fn an_edit_can_add_an_attachment_because_the_whole_message_is_applied() -> StoreResult<()> {
    let database = open()?;
    let store = ChatStore::new(&database);
    store.replace(&[row()], None)?;
    store.apply(
        &MessageDto {
            sender_id: 1,
            body: "Here it is".to_string(),
            ..message(1339)
        },
        CatchUpPage,
        true,
    )?;
    assert!(held(&store, 1339)?.media().is_empty());

    store.apply(
        &MessageDto {
            sender_id: 1,
            attachments: Some(vec![AttachmentDto {
                mime: Some("image/png".to_string()),
                ..AttachmentDto::new(900, "photo")
            }]),
            ..edit(1339, "Here it is", 90)
        },
        CatchUpPage,
        true,
    )?;
    let media = held(&store, 1339)?.media().to_vec();
    assert_eq!(media.len(), 1);
    assert_eq!(media[0].id, 900);
    Ok(())
}

/// A PREVIEW carries less than a page does — no reactions, no poll, no quote — and absent means
/// "not included here", never "cleared".
#[test]
fn a_preview_does_not_strip_what_a_fuller_copy_had() -> StoreResult<()> {
    let database = open()?;
    let store = ChatStore::new(&database);
    store.replace(&[row()], None)?;
    store.apply(
        &MessageDto {
            reactions: Some(vec![ReactionDto::new(9, "❤️")]),
            reaction_seq: Some(5),
            poll: Some(PollDto {
                poll_seq: 7,
                closed: false,
                options: vec![option(1, "Pizza", &[9])],
            }),
            ..message(1340)
        },
        CatchUpPage,
        true,
    )?;
    // The list read arrives with the trimmed preview of the same message.
    store.replace(
        &[ChatRowDto {
            last_message: Some(message(1340)),
            unread_count: 1,
            ..row()
        }],
        None,
    )?;
    let kept = held(&store, 1340)?;
    let reactions = kept.reactions.as_deref().expect("reactions");
    assert_eq!(reactions.len(), 1);
    assert_eq!(reactions[0].emoji, "❤️");
    assert_eq!(kept.reaction_seq, Some(5));
    assert!(kept.poll.is_some());

    // AND THE CASE THAT ACTUALLY WRITES: a preview carrying a NEWER edit seq — an edit happened,
    // and the list read brought the trimmed copy before a page did. The body is taken, and the
    // reactions and the poll it does not carry are NOT cleared by it.
    store.replace(
        &[ChatRowDto {
            last_message: Some(edit(1340, "Dinner at 8?", 91)),
            unread_count: 1,
            ..row()
        }],
        None,
    )?;
    let edited = held(&store, 1340)?;
    assert_eq!(edited.body, "Dinner at 8?");
    assert_eq!(edited.edit_seq, Some(91));
    let reactions = edited.reactions.as_deref().expect("reactions");
    assert_eq!(reactions.len(), 1);
    assert_eq!(reactions[0].emoji, "❤️");
    assert_eq!(edited.reaction_seq, Some(5));
    assert!(edited.poll.is_some());
    Ok(())
}

#[test]
fn reactions_are_complete_state_under_their_own_seq() -> StoreResult<()> {
    let database = open()?;
    let store = ChatStore::new(&database);
    store.replace(&[row()], None)?;
    store.apply(&message(1341), CatchUpPage, true)?;
    assert!(store.apply_reactions(42, 1341, 10, &[ReactionDto::new(9, "❤️")], LiveFrame)?);
    let reactions = held(&store, 1341)?.reactions.expect("reactions");
    assert_eq!(reactions.len(), 1);
    assert_eq!(reactions[0].emoji, "❤️");
    // Cleared is `[]` and the field STAYS: a client tells "cleared" from "no data".
    assert!(store.apply_reactions(42, 1341, 11, &[], LiveFrame)?);
    assert_eq!(held(&store, 1341)?.reactions, Some(vec![]));
    // And an older frame that crossed a newer one is dropped.
    assert!(!store.apply_reactions(42, 1341, 9, &[ReactionDto::new(9, "👍")], LiveFrame)?);
    assert_eq!(held(&store, 1341)?.reactions, Some(vec![]));
    // The chat's cursor followed the newest.
    assert_eq!(chat(&store, 42)?.max_reaction_seq, Some(11));
    Ok(())
}

#[test]
fn a_poll_is_complete_state_too_and_the_cursor_follows_it() -> StoreResult<()> {
    let database = open()?;
    let store = ChatStore::new(&database);
    store.replace(&[row()], None)?;
    store.apply(
        &MessageDto {
            body: "Pizza or pasta?".to_string(),
            ..message(1342)
        },
        CatchUpPage,
        true,
    )?;
    assert!(store.apply_poll(
        42,
        1342,
        &PollDto {
            poll_seq: 88,
            closed: false,
            options: vec![option(5, "Pizza", &[7]), option(6, "Pasta", &[])],
        },
        LiveFrame,
    )?);
    assert_eq!(
        held(&store, 1342)?.poll.expect("a poll").options[0].votes,
        [7]
    );
    assert!(!store.apply_poll(
        42,
        1342,
        &PollDto {
            poll_seq: 87,
            closed: true,
            options: vec![],
        },
        LiveFrame,
    )?);
    assert!(!held(&store, 1342)?.poll.expect("a poll").closed);
    assert_eq!(chat(&store, 42)?.max_poll_seq, Some(88));
    Ok(())
}

/// A STATE FOR A MESSAGE THIS DEVICE DOES NOT HOLD IS DROPPED, AND THE CURSOR STILL MOVES. History
/// paging re-delivers such a state embedded on the message itself, while a cursor left behind asks
/// for a page that answers the same nothing — for ever, because the chat's mark never comes back
/// down.
#[test]
fn a_state_for_an_unknown_message_is_dropped_and_the_cursor_moves_anyway() -> StoreResult<()> {
    let database = open()?;
    let store = ChatStore::new(&database);
    store.replace(&[row()], None)?;

    assert!(!store.apply_reactions(42, 9999, 124, &[ReactionDto::new(9, "❤️")], LiveFrame)?);
    assert_eq!(store.message(9999)?, None);
    assert_eq!(chat(&store, 42)?.max_reaction_seq, Some(124));

    assert!(!store.apply_poll(
        42,
        9998,
        &PollDto {
            poll_seq: 88,
            closed: false,
            options: vec![],
        },
        LiveFrame,
    )?);
    assert_eq!(chat(&store, 42)?.max_poll_seq, Some(88));

    // Evidence about a message this device has never seen is evidence about nothing.
    assert!(!store.apply_reactions(42, 9997, 300, &[ReactionDto::new(9, "👍")], Evidence)?);
    assert_eq!(chat(&store, 42)?.max_reaction_seq, Some(124));
    Ok(())
}

/// ONLY A LIVE FRAME AND A CATCH-UP PAGE MAY MOVE A CHAT CURSOR. A state that arrives as EVIDENCE —
/// embedded on a fetched message, or in the answer to this client's own reaction — is applied to the
/// message and moves nothing: advancing the cursor from evidence would step the chat past states for
/// messages this device has never seen, and `after_seq` can never look back.
#[test]
fn evidence_applies_to_the_message_and_moves_no_cursor() -> StoreResult<()> {
    let database = open()?;
    let store = ChatStore::new(&database);
    store.replace(&[row()], None)?;
    store.apply(&message(1), CatchUpPage, true)?;
    store.apply(
        &MessageDto {
            body: "Pizza or pasta?".to_string(),
            ..message(2)
        },
        CatchUpPage,
        true,
    )?;

    // The answer to our own reaction: applied, cursor untouched.
    assert!(store.apply_reactions(42, 1, 300, &[ReactionDto::new(7, "❤️")], Evidence)?);
    let reactions = held(&store, 1)?.reactions.expect("reactions");
    assert_eq!(reactions.len(), 1);
    assert_eq!(reactions[0].emoji, "❤️");
    assert_eq!(chat(&store, 42)?.max_reaction_seq, None);

    // The answer to our own vote: the same.
    assert!(store.apply_poll(
        42,
        2,
        &PollDto {
            poll_seq: 400,
            closed: false,
            options: vec![option(5, "Pizza", &[7])],
        },
        Evidence,
    )?);
    assert_eq!(held(&store, 2)?.poll.expect("a poll").options[0].votes, [7]);
    assert_eq!(chat(&store, 42)?.max_poll_seq, None);

    // A live frame and a catch-up page both move it.
    store.apply_reactions(42, 1, 301, &[], LiveFrame)?;
    assert_eq!(chat(&store, 42)?.max_reaction_seq, Some(301));
    store.apply_poll(
        42,
        2,
        &PollDto {
            poll_seq: 401,
            closed: true,
            options: vec![],
        },
        CatchUpPage,
    )?;
    assert_eq!(chat(&store, 42)?.max_poll_seq, Some(401));
    Ok(())
}

/// THE THREE CURSORS ARE THIS DEVICE'S, NOT THE SERVER'S. A `GET /chats` row carries `max_*_seq`
/// too, with the same names and a different meaning — the server's maximum — and a list read must
/// not write them anywhere near these columns: step 3's catch-up runs only "when the chat's
/// max_reaction_seq from step 2 exceeds the locally stored reaction cursor", so a device that stored
/// the server's mark as its own would never ask for a page again. They move under
/// `ChatStore::advance` alone, and there as high-water marks: a 0 that arrives after a 124 is not a
/// reset.
#[test]
fn the_catch_up_cursors_are_local_and_a_list_read_does_not_write_them() -> StoreResult<()> {
    let database = open()?;
    let store = ChatStore::new(&database);
    store.replace(
        &[ChatRowDto {
            max_reaction_seq: Some(124),
            max_edit_seq: Some(88),
            max_poll_seq: Some(89),
            ..row()
        }],
        None,
    )?;
    let fresh = chat(&store, 42)?;
    // The server says 124; this device has applied nothing, and says so.
    assert_eq!(fresh.max_reaction_seq, None);
    assert_eq!(fresh.max_edit_seq, None);
    assert_eq!(fresh.max_poll_seq, None);

    // What this device applies is what its cursors read back as…
    store.advance(42, Some(124), Some(88), Some(89))?;
    assert_eq!(chat(&store, 42)?.max_reaction_seq, Some(124));
    assert_eq!(chat(&store, 42)?.max_edit_seq, Some(88));
    assert_eq!(chat(&store, 42)?.max_poll_seq, Some(89));

    // …and another list read neither lowers them nor raises them.
    store.replace(&[row()], None)?;
    assert_eq!(chat(&store, 42)?.max_reaction_seq, Some(124));
    store.replace(
        &[ChatRowDto {
            max_reaction_seq: Some(900),
            ..row()
        }],
        None,
    )?;
    assert_eq!(chat(&store, 42)?.max_reaction_seq, Some(124));

    // A high-water mark: an older page cannot walk one back.
    store.advance(42, Some(5), None, None)?;
    assert_eq!(chat(&store, 42)?.max_reaction_seq, Some(124));

    // And a chat this device has just learned of starts level with nothing.
    store.replace(
        &[ChatRowDto {
            max_reaction_seq: Some(300),
            ..row_of(ChatDto {
                peer_user_id: Some(11),
                ..ChatDto::new(44, "direct", "Bob")
            })
        }],
        None,
    )?;
    assert_eq!(chat(&store, 44)?.max_reaction_seq, None);
    Ok(())
}

/// The read marker is MONOTONIC: a list read still in flight while the reader is reading must never
/// walk it backwards.
#[test]
fn the_read_marker_only_ever_moves_forward() -> StoreResult<()> {
    let database = open()?;
    let store = ChatStore::new(&database);
    let read_at_1000 = || ChatRowDto {
        last_read_message_id: 1000,
        ..row()
    };
    store.replace(&[read_at_1000()], None)?;
    store.mark_read(42, 1337)?;
    assert_eq!(chat(&store, 42)?.last_read_message_id, 1337);
    // The stale answer.
    store.replace(&[read_at_1000()], None)?;
    assert_eq!(chat(&store, 42)?.last_read_message_id, 1337);
    store.mark_read(42, 900)?;
    assert_eq!(chat(&store, 42)?.last_read_message_id, 1337);
    Ok(())
}

/// THE COUNT IS INCREMENTED BY A LIVE FRAME AND NEVER RECOMPUTED FROM WHAT IS HELD. Local history
/// is not the whole of it — retention has swept some of it, paging has not fetched the rest — so a
/// recount is a badge that silently falls to one the moment anything arrives: a device told "12
/// unread" by the list, then handed one live message, would draw 1.
#[test]
fn the_unread_count_is_the_other_half_of_the_marker() -> StoreResult<()> {
    let database = open()?;
    let store = ChatStore::new(&database);
    store.replace(&[row()], None)?;
    store.apply(&message(1), LiveFrame, true)?;
    store.apply(&message(2), LiveFrame, true)?;
    store.apply(&message(3), LiveFrame, true)?;
    assert_eq!(chat(&store, 42)?.unread_count, 3);
    assert_eq!(store.unread()?, 3);

    store.mark_read(42, 2)?;
    assert_eq!(chat(&store, 42)?.unread_count, 1);
    // An EDIT moves neither the count nor the ordering: it never re-notifies.
    store.apply(&edit(1, "rewritten", 5), LiveFrame, true)?;
    assert_eq!(chat(&store, 42)?.unread_count, 1);

    store.mark_read(42, 3)?;
    assert_eq!(store.unread()?, 0);
    Ok(())
}

/// The four conditions on raising the count, each of which is a badge bug on its own.
#[test]
fn only_somebody_elses_new_live_message_above_the_marker_counts() -> StoreResult<()> {
    let database = open()?;
    // Somebody who is NOT the fixture's default sender, so that "somebody else's message" and "my
    // own" are told apart by the store and not by the test's luck.
    let mine = 99;
    let store = ChatStore::new(&database).with_me(move || mine);
    store.replace(
        &[ChatRowDto {
            last_read_message_id: 10,
            ..row()
        }],
        None,
    )?;

    // A page: the list read that preceded it already counted these.
    store.apply_page(&[message(11), message(12)], CatchUpPage, true)?;
    assert_eq!(chat(&store, 42)?.unread_count, 0);

    // A live frame from somebody else, above the marker: one.
    store.apply(&message(13), LiveFrame, true)?;
    assert_eq!(chat(&store, 42)?.unread_count, 1);

    // The same message again — a lost ack re-delivered as an ordinary message — is not two.
    store.apply(&message(13), LiveFrame, true)?;
    assert_eq!(chat(&store, 42)?.unread_count, 1);

    // This reader's own send, arriving at their other device: nothing.
    store.apply(
        &MessageDto {
            sender_id: mine,
            ..message(14)
        },
        LiveFrame,
        true,
    )?;
    assert_eq!(chat(&store, 42)?.unread_count, 1);

    // Something from BELOW the marker, arriving late: read before it got here.
    store.apply(&message(5), LiveFrame, true)?;
    assert_eq!(chat(&store, 42)?.unread_count, 1);

    // An EDIT never counts, not even for a message this device has never held: the frame exists
    // precisely so that a rewrite bumps nothing and notifies nobody.
    store.apply(&edit(20, "rewritten", 7), LiveFrame, true)?;
    assert_eq!(held(&store, 20)?.body, "rewritten");
    assert_eq!(chat(&store, 42)?.unread_count, 1);
    Ok(())
}

/// A list read's count is the SERVER's and is never lowered to what this device can see — but
/// neither is it allowed to be lower than that: a message that arrived while the read was in
/// flight is not on that answer, and the two numbers are both lower bounds.
#[test]
fn a_list_read_takes_the_servers_count_and_never_less_than_what_is_visible() -> StoreResult<()> {
    let database = open()?;
    let store = ChatStore::new(&database).with_me(|| 99);
    let unread = |count: i32| ChatRowDto {
        unread_count: count,
        ..row()
    };
    store.replace(&[unread(12)], None)?;
    assert_eq!(chat(&store, 42)?.unread_count, 12);

    // A frame lands while the next list read is in flight; that read says 12 again.
    store.apply(&message(99), LiveFrame, true)?;
    assert_eq!(chat(&store, 42)?.unread_count, 13);
    store.replace(&[unread(12)], None)?;
    assert_eq!(chat(&store, 42)?.unread_count, 13);

    // And a read that reports the message as read brings it back down.
    store.replace(
        &[ChatRowDto {
            last_read_message_id: 99,
            ..unread(0)
        }],
        None,
    )?;
    assert_eq!(chat(&store, 42)?.unread_count, 0);

    // A message of the READER'S OWN, racing the same read, adds nothing: it was never unread, and
    // the server did not count it either.
    store.apply(
        &MessageDto {
            sender_id: 99,
            ..message(150)
        },
        LiveFrame,
        true,
    )?;
    store.replace(
        &[ChatRowDto {
            last_read_message_id: 99,
            last_message: Some(message(120)),
            ..unread(0)
        }],
        None,
    )?;
    assert_eq!(chat(&store, 42)?.unread_count, 0);
    Ok(())
}

/// READING PART OF A RUN IS SUBTRACTION, not a recount. A device told "12 unread" that holds the
/// last three of them and reads two is ten behind; a recount from local history would say ONE,
/// which is the badge quietly going out on a conversation nine messages deep.
///
/// The marker is an id THRESHOLD, so reading up to the newest message this device holds means
/// everything at or below it is read — that is the other branch, and it is 0. Where the two
/// disagree is a PARTIAL read, and there the subtraction can leave the count too HIGH when local
/// history is sparse (the server had more messages in the range than this device holds). Too high
/// is the safe direction: the next list read replaces it.
#[test]
fn reading_part_of_a_run_subtracts_rather_than_recounting() -> StoreResult<()> {
    let database = open()?;
    let store = ChatStore::new(&database).with_me(|| 99);
    store.replace(
        &[ChatRowDto {
            unread_count: 12,
            ..row()
        }],
        None,
    )?;
    store.apply_page(
        &[message(498), message(499), message(500)],
        CatchUpPage,
        true,
    )?;

    store.mark_read(42, 499)?;

    assert_eq!(chat(&store, 42)?.unread_count, 10);
    assert_eq!(chat(&store, 42)?.last_read_message_id, 499);

    // And reading to the bottom of what is held is the other branch: the threshold covers
    // everything this device could know about.
    store.mark_read(42, 500)?;
    assert_eq!(chat(&store, 42)?.unread_count, 0);
    Ok(())
}

/// A MENTION THIS DEVICE RAISED IS NOT CLEARED BY AN ANSWER THAT DID NOT SEE IT. `mentioned` rides
/// on a list row only while an unread message names the caller, and a read that crossed the frame
/// carrying one would otherwise take the mark off a message still unread.
#[test]
fn a_mention_raised_by_a_frame_survives_a_list_read_that_missed_it() -> StoreResult<()> {
    let database = open()?;
    let store = ChatStore::new(&database).with_me(|| 99);
    store.replace(&[row()], None)?;
    store.apply(
        &MessageDto {
            mentions: Some(vec![MentionDto::new(99, "Anna")]),
            ..message(600)
        },
        LiveFrame,
        true,
    )?;
    assert_eq!(chat(&store, 42)?.mentioned, Some(true));

    // The list read was in flight when the frame landed, so it says nothing about a mention.
    store.replace(
        &[ChatRowDto {
            unread_count: 1,
            ..row()
        }],
        None,
    )?;
    assert_eq!(chat(&store, 42)?.mentioned, Some(true));

    // Reading past it is what takes the mark off.
    store.mark_read(42, 600)?;
    assert_eq!(chat(&store, 42)?.mentioned, None);
    Ok(())
}

/// ADAPTED: the C# test reads as nobody (id 0) and names Anna (9), and its store marks the chat for
/// a mention of anybody. docs/protocol.md marks it only for a message that names THE READER (see
/// `a_mention_marks_only_the_reader_it_names_from_somebody_not_blocked`), so here Anna reads.
#[test]
fn a_mention_marks_the_chat_until_it_is_read() -> StoreResult<()> {
    let database = open()?;
    let store = ChatStore::new(&database).with_me(|| 9);
    store.replace(&[row()], None)?;
    store.apply(
        &MessageDto {
            body: "@Anna are you in?".to_string(),
            mentions: Some(vec![MentionDto::new(9, "Anna")]),
            ..message(10)
        },
        CatchUpPage,
        true,
    )?;
    // Absent, never false — which is the wire's shape and what a list row draws its mark from.
    assert_eq!(chat(&store, 42)?.mentioned, Some(true));
    store.mark_read(42, 10)?;
    assert_eq!(chat(&store, 42)?.mentioned, None);
    Ok(())
}

/// NOT A PORT — the protocol's rule, which the C# store does not follow (measured on .NET 10: every
/// case below marks the chat there). The "@" mark is the server's filter: a message above the
/// marker that names THIS READER, from somebody else they have not blocked (docs/protocol.md, "The
/// mark on the chat row"; the Mac and the web client ask the same) — and reading past the newest
/// such message clears it, whatever names somebody else after it, because "a chat with `mentioned`
/// always has an unread count".
#[test]
fn a_mention_marks_only_the_reader_it_names_from_somebody_not_blocked() -> StoreResult<()> {
    let database = open()?;
    let store = ChatStore::new(&database).with_me(|| 99);
    store.replace(&[row()], None)?;
    let naming = |id: i64, sender: i64, named: i64| MessageDto {
        sender_id: sender,
        mentions: Some(vec![MentionDto::new(named, "Somebody")]),
        ..message(id)
    };

    // Somebody else named.
    store.apply(&naming(1, 7, 9), LiveFrame, true)?;
    assert_eq!(chat(&store, 42)?.mentioned, None);
    store.mark_read(42, 1)?;

    // The reader named, by somebody they have blocked.
    store.set_blocked(7, true)?;
    store.apply(&naming(2, 7, 99), LiveFrame, true)?;
    assert_eq!(chat(&store, 42)?.mentioned, None);
    store.mark_read(42, 2)?;

    // The reader's own message, naming somebody.
    store.apply(&naming(3, 99, 9), LiveFrame, true)?;
    assert_eq!(chat(&store, 42)?.mentioned, None);
    store.mark_read(42, 3)?;

    // The reader named by somebody they have not blocked: marked — and read past, cleared, though a
    // later message names somebody else.
    store.set_blocked(7, false)?;
    store.apply(&naming(4, 7, 99), LiveFrame, true)?;
    assert_eq!(chat(&store, 42)?.mentioned, Some(true));
    store.apply(&naming(5, 11, 9), CatchUpPage, true)?;
    store.mark_read(42, 4)?;
    assert_eq!(chat(&store, 42)?.mentioned, None);
    Ok(())
}

#[test]
fn a_page_comes_back_newest_first_and_pages_backwards() -> StoreResult<()> {
    let database = open()?;
    let store = ChatStore::new(&database);
    store.replace(&[row()], None)?;
    let page: Vec<MessageDto> = (1..=10).map(message).collect();
    store.apply_page(&page, CatchUpPage, true)?;
    assert_eq!(ids(&store.messages(42, None, 3)?), [10, 9, 8]);
    assert_eq!(ids(&store.messages(42, Some(8), 2)?), [7, 6]);
    // The newest is the row's preview.
    assert_eq!(store.newest(42)?.map(|newest| newest.id), Some(10));
    assert_eq!(
        chat(&store, 42)?.last_message.map(|preview| preview.id),
        Some(10)
    );
    Ok(())
}

/// A CHAIN IS ROOTED AT THE TOP and read far more often than it is written, which is why the root
/// is stored rather than walked.
#[test]
fn a_chain_is_the_root_and_everything_that_names_it() -> StoreResult<()> {
    let database = open()?;
    let store = ChatStore::new(&database);
    store.replace(&[row()], None)?;
    let saying = |message: MessageDto, body: &str| MessageDto {
        body: body.to_string(),
        ..message
    };
    store.apply_page(
        &[
            saying(message(38), "Where shall we eat?"),
            saying(reply(39, 38), "The Italian place"),
            saying(message(40), "somewhere else entirely"),
            saying(reply(41, 38), "Six works"),
        ],
        CatchUpPage,
        true,
    )?;
    assert_eq!(ids(&store.thread(38)?), [38, 39, 41]);
    // A message nobody answered is a chain of one.
    assert_eq!(ids(&store.thread(40)?), [40]);
    Ok(())
}

#[test]
fn a_member_who_left_keeps_their_row_because_their_messages_keep_their_author() -> StoreResult<()> {
    let database = open()?;
    let store = ChatStore::new(&database);
    store.replace_members(
        &[member(7, "anna", "Anna", "owner")],
        &[MemberDto {
            has_left: true,
            deleted: true,
            ..MemberDto::new(11, "junior", "Junior")
        }],
    )?;
    assert_eq!(store.members()?.len(), 2);
    let gone = store.member(11)?.expect("a former member");
    assert!(gone.has_left);
    assert!(gone.deleted);
    assert!(gone.is_former());
    // The ROLE is the wire's own word, and a former member has none — which is not the same answer
    // as "a member who is not the owner".
    assert_eq!(gone.role, None);
    assert!(store.member(7)?.expect("the owner").owner());
    assert_eq!(
        store.member(7)?.expect("the owner").role.as_deref(),
        Some("owner")
    );
    // An id this device has never heard of is None, and naming it is the caller's job — "Deleted
    // account" is a client's word, not the server's.
    assert_eq!(store.member(99)?, None);
    Ok(())
}

/// THE BLOCK LIST IS COMPLETE STATE. Both reads that carry it carry all of it, there is no catch-up
/// feed to cursor and nothing to miss, and an absent list means nobody rather than "leave what you
/// hold alone". An id on it may name somebody the roster cannot.
#[test]
fn the_block_list_is_complete_state_and_need_not_resolve_to_a_name() -> StoreResult<()> {
    let database = open()?;
    let store = ChatStore::new(&database);
    store.replace_members(&[member(11, "bob", "Bob", "member")], &[])?;
    store.replace_blocked(&[11, 14, 11])?;
    assert_eq!(*store.blocked()?, [11, 14]);
    assert!(store.is_blocked(11)?);
    // 14 is nobody this device has heard of — a blocked member who left, or whose account is gone
    // — and the list carries them anyway.
    assert!(store.is_blocked(14)?);
    assert_eq!(store.member(14)?, None);

    store.replace_blocked(&[])?;
    assert!(store.blocked()?.is_empty());
    assert!(!store.is_blocked(11)?);
    Ok(())
}

/// THE ROSTER AND THE BLOCK LIST ARE KEPT IN MEMORY, AND NO WRITE MAY LEAVE THE COPY BEHIND — not
/// one through another store on the same file, and not a wipe, which no store sees. A conversation
/// asks both once per bubble, so a stale copy would be a blocked member's words drawn, or a renamed
/// member drawn by their old name, until a restart.
#[test]
fn what_is_kept_of_the_roster_follows_every_write_whoever_made_it() -> StoreResult<()> {
    let database = open()?;
    let reading = ChatStore::new(&database);
    let writing = ChatStore::new(&database);
    writing.replace_members(
        &[
            member(7, "anna", "Anna", "owner"),
            member(11, "bob", "Bob", "member"),
        ],
        &[],
    )?;
    assert_eq!(reading.member(11)?.expect("Bob").display_name, "Bob");
    assert!(!reading.is_blocked(11)?);
    assert!(reading.blocked()?.is_empty());

    writing.joined(
        &UserDto {
            avatar_version: 3,
            ..UserDto::new(11, "bob", "Robert")
        },
        None,
        None,
    )?;
    assert_eq!(reading.member(11)?.expect("Bob").display_name, "Robert");
    let members = reading.members()?;
    let bob: Vec<&MemberDto> = members.iter().filter(|member| member.id == 11).collect();
    assert_eq!(bob.len(), 1);
    assert_eq!(bob[0].avatar_version, 3);
    writing.set_owner(11, None)?;
    assert!(reading.member(11)?.expect("Bob").owner());
    writing.left(7, None)?;
    assert!(reading.member(7)?.expect("Anna").has_left);
    writing.deleted(&MemberDto::new(7, "deleted", "Deleted account"))?;
    assert!(reading.member(7)?.expect("Anna").deleted);

    writing.set_blocked(11, true)?;
    assert!(reading.is_blocked(11)?);
    assert_eq!(*reading.blocked()?, [11]);
    writing.replace_blocked(&[14])?;
    assert!(!reading.is_blocked(11)?);
    assert!(reading.is_blocked(14)?);

    database.wipe_all()?;
    assert!(reading.members()?.is_empty());
    assert_eq!(reading.member(11)?, None);
    assert!(!reading.is_blocked(14)?);
    assert!(reading.blocked()?.is_empty());
    Ok(())
}

/// The copy handed out is the caller's to read, never to change: a second caller must get the
/// table, not what the first one did to its answer. (The C# checks that `IList.Add` throws; here the
/// copy is a slice shared behind an `Rc` the store also holds, so there is no `&mut` to be had.)
#[test]
fn the_kept_roster_cannot_be_changed_by_whoever_was_handed_it() -> StoreResult<()> {
    let database = open()?;
    let store = ChatStore::new(&database);
    store.replace_members(&[member(7, "anna", "Anna", "owner")], &[])?;
    store.replace_blocked(&[11])?;
    let mut members = store.members()?;
    let mut blocked = store.blocked()?;
    assert!(Rc::get_mut(&mut members).is_none());
    assert!(Rc::get_mut(&mut blocked).is_none());
    // What the caller does to a copy of its own is its own.
    let mut mine = members.to_vec();
    mine.push(MemberDto::new(8, "x", "X"));
    let mut also_mine = blocked.to_vec();
    also_mine.push(12);
    assert_eq!(store.members()?.len(), 1);
    assert_eq!(*store.blocked()?, [11]);
    Ok(())
}

/// A chat's kind is what its row says — asked of every bubble a conversation draws — `None` for a
/// chat with no row yet, and picked up once the list read writes one; a wipe forgets it.
#[test]
fn a_chats_kind_is_its_rows_and_follows_the_list_read() -> StoreResult<()> {
    let database = open()?;
    let store = ChatStore::new(&database);
    assert_eq!(store.kind(42)?, None);
    store.replace(
        &[
            row(),
            row_of(ChatDto::new(43, "direct", "Bob")),
            row_of(ChatDto::new(44, "ai", "Assistant")),
        ],
        None,
    )?;
    assert_eq!(store.kind(42)?.as_deref(), Some("family"));
    assert_eq!(store.kind(43)?.as_deref(), Some("direct"));
    assert_eq!(store.kind(44)?.as_deref(), Some("ai"));
    assert_eq!(store.kind(43)?, Some(chat(&store, 43)?.chat.kind));
    // A chat the list no longer carries is hidden, not forgotten: its kind is still its row's.
    store.replace(&[row()], None)?;
    assert_eq!(store.kind(43)?.as_deref(), Some("direct"));
    assert_eq!(store.kind(99)?, None);

    database.wipe_all()?;
    assert_eq!(store.kind(42)?, None);
    Ok(())
}

/// The `member_blocked` frame is a STATE-SET, and it is the one place a client applies the
/// consequence itself: the frame reaches the blocker's own devices and no list read is coming. The
/// chat leaves the list and comes back WHOLE, because nothing about it is ever deleted
/// (docs/protocol.md, "Blocking a member").
#[test]
fn blocking_a_member_hides_their_chat_and_unblocking_brings_it_back_whole() -> StoreResult<()> {
    let database = open()?;
    let store = ChatStore::new(&database);
    let direct = ChatDto {
        peer_user_id: Some(11),
        ..ChatDto::new(43, "direct", "Bob")
    };
    store.replace(&[row(), row_of(direct)], None)?;
    store.apply(
        &MessageDto {
            chat_id: 43,
            body: "Are we still on?".to_string(),
            sender_id: 11,
            ..message(500)
        },
        CatchUpPage,
        true,
    )?;

    store.set_blocked(11, true)?;
    assert_eq!(chat_ids(&store.chats()?), [42]);
    assert!(!store.is_listed(43)?);
    // Hidden, not deleted: the marker, the cursors and every word are still here.
    assert!(store.chat(43)?.is_some());
    assert_eq!(held(&store, 500)?.body, "Are we still on?");

    store.set_blocked(11, false)?;
    let mut listed = chat_ids(&store.chats()?);
    listed.sort_unstable();
    assert_eq!(listed, [42, 43]);
    assert_eq!(held(&store, 500)?.body, "Are we still on?");

    // The family chat is NOT hidden by a block: a blocked member's message still arrives, still
    // counts and may still be the preview — the count is the other half of the read marker, and
    // projecting one without the other desynchronises them.
    store.set_blocked(11, true)?;
    assert!(chat_ids(&store.chats()?).contains(&42));
    Ok(())
}

/// A complete-state read does NOT decide what is on the list — the list read that follows it does,
/// and two writers for one fact would fight over it.
#[test]
fn reading_the_block_list_does_not_relist_a_chat_the_server_left_out() -> StoreResult<()> {
    let database = open()?;
    let store = ChatStore::new(&database);
    let direct = ChatDto {
        peer_user_id: Some(11),
        ..ChatDto::new(43, "direct", "Bob")
    };
    store.replace(&[row(), row_of(direct)], None)?;
    // The server stopped listing it: this reader has blocked Bob.
    store.replace(&[row()], None)?;
    assert!(!store.is_listed(43)?);

    // Reading `/me` again says the same thing about Bob and must not undo it.
    store.replace_blocked(&[11])?;
    assert!(!store.is_listed(43)?);
    // Nor does a list that no longer names him relist the chat on its own.
    store.replace_blocked(&[])?;
    assert!(!store.is_listed(43)?);
    Ok(())
}

/// A birthday is a MONTH AND A DAY, arriving as one object — and it survives the store, because the
/// family calendar is drawn from the roster and nothing else replays it.
#[test]
fn a_birthday_is_a_month_and_a_day_and_it_survives_the_roster() -> StoreResult<()> {
    let database = open()?;
    let store = ChatStore::new(&database);
    let anna: Option<MemberDto> = wire::decode(
        r#"
        {"id": 7, "username": "anna", "display_name": "Anna", "role": "owner",
         "birthday": {"month": 3, "day": 14}}
        "#,
    );
    // A field whose type is wrong does not degrade — it takes the whole answer with it.
    let anna = anna.expect("Anna, read");
    assert!(anna.owner());
    assert_eq!(anna.birthday, Some(BirthdayDto { month: 3, day: 14 }));

    store.replace_members(&[anna.clone(), MemberDto::new(11, "bob", "Bob")], &[])?;
    assert_eq!(
        store.member(7)?.expect("Anna").birthday,
        Some(BirthdayDto { month: 3, day: 14 })
    );
    // Nobody's birthday is not everybody's: absent stays absent.
    assert_eq!(store.member(11)?.expect("Bob").birthday, None);

    // 29 February is a birthday here: there is no year for it to fail to exist in.
    store.replace_members(
        &[MemberDto {
            birthday: Some(BirthdayDto { month: 2, day: 29 }),
            ..anna
        }],
        &[],
    )?;
    assert_eq!(
        store.member(7)?.expect("Anna").birthday,
        Some(BirthdayDto { month: 2, day: 29 })
    );
    Ok(())
}

#[test]
fn the_list_is_ordered_by_when_something_last_happened() -> StoreResult<()> {
    let database = open()?;
    let store = ChatStore::new(&database);
    store.replace(
        &[
            row(),
            row_of(ChatDto {
                peer_user_id: Some(9),
                ..ChatDto::new(43, "direct", "Anna")
            }),
        ],
        None,
    )?;
    let at = |id: i64, chat: i64, created: &str| MessageDto {
        chat_id: chat,
        created_at: created.to_string(),
        ..message(id)
    };
    store.apply(&at(1, 43, "2026-09-12T09:00:00Z"), CatchUpPage, true)?;
    store.apply(&at(2, 42, "2026-09-12T11:00:00Z"), CatchUpPage, true)?;
    assert_eq!(chat_ids(&store.chats()?), [42, 43]);
    store.apply(&at(3, 43, "2026-09-12T12:00:00Z"), CatchUpPage, true)?;
    assert_eq!(chat_ids(&store.chats()?), [43, 42]);

    // AN EDIT NEVER MOVES THE CHAT'S ORDERING, whatever its original timestamp: it is not a new
    // message, it never re-notifies and it never bumps a count. A list that jumped when somebody
    // fixed a typo three pages back would be a list nobody could keep their place in.
    store.apply(
        &MessageDto {
            created_at: "2026-09-12T13:00:00Z".to_string(),
            ..edit(2, "rewritten", 7)
        },
        CatchUpPage,
        true,
    )?;
    assert_eq!(chat_ids(&store.chats()?), [43, 42]);
    Ok(())
}

fn group(id: i64, title: &str, members: &[i64], admin: i64) -> ChatDto {
    ChatDto {
        member_ids: Some(members.to_vec()),
        admin_user_id: Some(admin),
        ..ChatDto::new(id, "group", title)
    }
}

/// A GROUP's members and admin are kept on its row as the server stated them, in join order, and
/// rewritten whole by every list read — and every other kind carries neither (docs/protocol.md,
/// "Groups").
#[test]
fn a_groups_members_and_admin_are_persisted_and_rewritten_whole() -> StoreResult<()> {
    let database = open()?;
    let store = ChatStore::new(&database);
    store.replace(
        &[row(), row_of(group(77, "Cousins", &[11, 7, 9], 11))],
        None,
    )?;

    let cousins = chat(&store, 77)?.chat;
    assert_eq!(cousins.member_ids, Some(vec![11, 7, 9]));
    assert_eq!(cousins.admin_user_id, Some(11));
    assert_eq!(cousins.peer_user_id, None);
    assert_eq!(chat(&store, 42)?.chat.member_ids, None);
    assert_eq!(chat(&store, 42)?.chat.admin_user_id, None);

    // The admin left and the hand-off went to the member who joined earliest after them.
    store.replace(&[row(), row_of(group(77, "Cousins", &[7, 9], 7))], None)?;
    let cousins: Vec<ChatDto> = store
        .chats()?
        .into_iter()
        .filter(|row| row.chat.id == 77)
        .map(|row| row.chat)
        .collect();
    assert_eq!(cousins.len(), 1);
    assert_eq!(cousins[0].member_ids, Some(vec![7, 9]));
    assert_eq!(cousins[0].admin_user_id, Some(7));
    Ok(())
}

/// A group a list read no longer carries is HIDDEN, as every chat is — "a client drops every chat
/// that leaves `GET /chats`, of any kind" — and comes back when a later read carries it again.
#[test]
fn a_group_that_leaves_the_list_is_hidden() -> StoreResult<()> {
    let database = open()?;
    let store = ChatStore::new(&database);
    store.replace(&[row(), row_of(group(77, "Cousins", &[7, 9], 9))], None)?;

    store.replace(&[row()], None)?;
    assert!(!store.is_listed(77)?);
    assert!(!chat_ids(&store.chats()?).contains(&77));

    store.replace(&[row(), row_of(group(77, "Cousins", &[7, 9], 9))], None)?;
    assert!(store.is_listed(77)?);
    Ok(())
}

/// `ChatStore::upsert` writes the chat and puts it on the list without touching its counts or its
/// marker; `ChatStore::remove` takes a group and its messages away, and refuses every other kind.
#[test]
fn upsert_keeps_the_counts_and_remove_takes_only_a_group() -> StoreResult<()> {
    let database = open()?;
    let store = ChatStore::new(&database);
    store.replace(
        &[
            row(),
            ChatRowDto {
                unread_count: 4,
                last_read_message_id: 30,
                ..row_of(group(77, "Cousins", &[7, 9], 9))
            },
        ],
        None,
    )?;
    store.apply_page(
        &[MessageDto {
            chat_id: 77,
            sender_id: 9,
            ..message(40)
        }],
        CatchUpPage,
        true,
    )?;

    store.upsert(&group(77, "The cousins", &[7, 9, 11], 9))?;
    let kept = chat(&store, 77)?;
    assert_eq!(kept.chat.title, "The cousins");
    assert_eq!(kept.unread_count, 4);
    assert_eq!(kept.last_read_message_id, 30);

    // A chat never held before starts with nothing unread.
    store.upsert(&group(78, "Book club", &[7, 12], 12))?;
    assert_eq!(chat(&store, 78)?.unread_count, 0);

    assert!(!store.remove(42)?);
    assert!(store.chat(42)?.is_some());
    assert!(store.remove(77)?);
    assert_eq!(store.chat(77)?, None);
    assert_eq!(store.message(40)?, None);
    assert!(!store.remove(77)?);
    Ok(())
}

/// A LIST READ THAT BEGAN BEFORE A GROUP WAS MADE does not hide it when it lands — and one that
/// began before a rename keeps the newer name while still taking its counts. The next read, begun
/// after, decides normally.
#[test]
fn a_list_read_older_than_an_upsert_does_not_undo_it() -> StoreResult<()> {
    let database = open()?;
    let store = ChatStore::new(&database);
    store.replace(&[row(), row_of(group(77, "Cousins", &[7, 9], 9))], None)?;

    let mark = store.list_mark();
    // While the request is in flight: a group is created, and another is renamed.
    store.upsert(&group(78, "Book club", &[7, 12], 7))?;
    store.upsert(&group(77, "The cousins", &[7, 9, 11], 9))?;
    store.replace(
        &[
            row(),
            ChatRowDto {
                unread_count: 3,
                ..row_of(group(77, "Cousins", &[7, 9], 9))
            },
        ],
        Some(mark),
    )?;

    assert!(store.is_listed(78)?);
    let renamed = chat(&store, 77)?;
    assert_eq!(renamed.chat.title, "The cousins");
    assert_eq!(renamed.chat.member_ids, Some(vec![7, 9, 11]));
    assert_eq!(renamed.unread_count, 3);

    // A read begun after the changes is the list again.
    store.replace(
        &[row(), row_of(group(77, "Cousins", &[7, 9], 9))],
        Some(store.list_mark()),
    )?;
    assert!(!store.is_listed(78)?);
    assert_eq!(chat(&store, 77)?.chat.title, "Cousins");
    Ok(())
}

/// A LIST READ THAT BEGAN BEFORE A GROUP WAS REMOVED does not write it back; the next read, begun
/// after, adds it if the server lists it again (the reader was added back).
#[test]
fn a_list_read_older_than_a_remove_does_not_write_it_back() -> StoreResult<()> {
    let database = open()?;
    let store = ChatStore::new(&database);
    let cousins = group(77, "Cousins", &[7, 9], 9);
    store.replace(&[row(), row_of(cousins.clone())], None)?;

    let mark = store.list_mark();
    assert!(store.remove(77)?);
    // And a removal of a group this device never held is a fact too.
    assert!(!store.remove(79)?);
    store.replace(
        &[
            row(),
            ChatRowDto {
                last_message: Some(MessageDto {
                    chat_id: 77,
                    sender_id: 9,
                    ..message(40)
                }),
                ..row_of(cousins.clone())
            },
            row_of(group(79, "Never seen", &[7, 9], 9)),
        ],
        Some(mark),
    )?;

    assert_eq!(store.chat(77)?, None);
    assert_eq!(store.message(40)?, None);
    assert_eq!(store.chat(79)?, None);
    assert!(store.chat(42)?.is_some());

    store.replace(&[row(), row_of(cousins)], Some(store.list_mark()))?;
    assert!(store.is_listed(77)?);
    Ok(())
}
