//! `ThreadModelTests`: a chain on its own surface — rooted, read whole, kept out of the cache, and
//! counted once per copy.

mod common;

use fc_core::protocol::{ChatDto, ChatRowDto, MessageDto, PollDto, PollOptionDto, ReactionDto};
use fc_core::store::{OutboxRow, SeqRoute, StoreResult};
use fc_logic::conversation::Bubble;
use fc_logic::threads::ThreadModel;

use common::chat::{CHAT, ChatRig};
use common::server::Server;

/// The C# `Message(id, root, replies, sender, body, editSeq)`.
#[derive(Clone, Copy)]
struct M {
    id: i64,
    root: Option<i64>,
    replies: Option<i32>,
    sender: i64,
    body: &'static str,
    edit_seq: Option<i64>,
}

fn m(id: i64) -> M {
    M {
        id,
        root: None,
        replies: None,
        sender: 9,
        body: "x",
        edit_seq: None,
    }
}

fn reply(id: i64, root: i64) -> M {
    M {
        root: Some(root),
        ..m(id)
    }
}

fn root(id: i64, replies: i32) -> M {
    M {
        replies: Some(replies),
        ..m(id)
    }
}

impl M {
    fn dto(self) -> MessageDto {
        MessageDto {
            thread_root_id: self.root,
            reply_count: self.replies,
            edit_seq: self.edit_seq,
            ..MessageDto::new(
                self.id,
                CHAT,
                self.sender,
                None,
                self.body,
                "2026-09-12T10:00:00Z",
            )
        }
    }

    fn json(self) -> String {
        let mut json = format!(
            r#"{{"id": {}, "chat_id": {CHAT}, "sender_id": {}, "client_msg_id": null, "body": "{}", "created_at": "2026-09-12T10:00:00Z""#,
            self.id, self.sender, self.body
        );
        if let Some(root) = self.root {
            json.push_str(&format!(r#", "thread_root_id": {root}"#));
        }
        if let Some(count) = self.replies {
            json.push_str(&format!(r#", "reply_count": {count}"#));
        }
        if let Some(seq) = self.edit_seq {
            json.push_str(&format!(
                r#", "edit_seq": {seq}, "edited_at": "2026-09-12T11:00:00Z""#
            ));
        }
        json.push('}');
        json
    }
}

/// `Page(messages…)`: a `200` with these rows.
fn page(messages: &[M]) -> String {
    let rows: Vec<String> = messages.iter().map(|message| message.json()).collect();
    format!(r#"{{"messages": [{}]}}"#, rows.join(", "))
}

fn apply(rig: &ChatRig, messages: &[M]) -> StoreResult<()> {
    let messages: Vec<MessageDto> = messages.iter().map(|message| message.dto()).collect();
    rig.chats.apply_page(&messages, SeqRoute::Evidence, true)?;
    Ok(())
}

fn build(rig: &ChatRig, server: &Server, named: i64) -> StoreResult<ThreadModel> {
    Ok(
        ThreadModel::new(CHAT, named, rig.chats.clone(), rig.api(server))?
            .with_outbox(rig.outbox.clone()),
    )
}

fn ids(bubbles: &[Bubble]) -> Vec<i64> {
    bubbles.iter().map(|bubble| bubble.message.id).collect()
}

fn reacted(message: M, seq: i64, emoji: &str) -> MessageDto {
    MessageDto {
        reaction_seq: Some(seq),
        reactions: Some(vec![ReactionDto::new(9, emoji)]),
        ..message.dto()
    }
}

/// Opened from a held reply, the chain is rooted before anything is asked, and draws what is held.
#[test]
fn a_chain_opens_at_once_on_what_is_held() -> StoreResult<()> {
    let rig = ChatRig::new()?;
    apply(&rig, &[root(10, 1), reply(11, 10), m(12)])?;

    let thread = build(&rig, &Server::new(), 11)?;

    assert_eq!(10, thread.root_id());
    assert_eq!(vec![10, 11], ids(&thread.bubbles()?));
    assert_eq!(1, thread.replies(&thread.bubbles()?));
    assert_eq!(12, build(&rig, &Server::new(), 12)?.root_id());
    Ok(())
}

/// Named by a reply the device does not hold, the first page answers with the root first — and
/// re-roots.
#[test]
fn the_first_page_roots_the_chain() -> StoreResult<()> {
    let rig = ChatRig::new()?;
    let server = Server::new();
    server.then(
        "/chats/42/messages/11/thread",
        &[(200, Some(&page(&[root(10, 1), reply(11, 10)])))],
    );
    let thread = build(&rig, &server, 11)?;
    assert_eq!(11, thread.root_id());

    assert_eq!(None, rig.drive(thread.load())?);

    assert!(thread.loaded());
    assert_eq!(10, thread.root_id());
    assert_eq!(vec![10, 11], ids(&thread.bubbles()?));
    assert_eq!(
        vec!["/api/v1/chats/42/messages/11/thread?limit=50"],
        server.asked()
    );
    Ok(())
}

/// A full page asks again after the newest row it held, and a short one ends the read.
#[test]
fn a_chain_is_read_page_by_page_until_a_short_one() -> StoreResult<()> {
    let rig = ChatRig::new()?;
    let first: Vec<M> = std::iter::once(root(10, 50))
        .chain((11..60).map(|id| reply(id, 10)))
        .collect();
    let server = Server::new();
    server.then(
        "/chats/42/messages/10/thread",
        &[
            (200, Some(&page(&first))),
            (200, Some(&page(&[reply(60, 10)]))),
        ],
    );
    let thread = build(&rig, &server, 10)?;

    rig.drive(thread.load())?;

    assert_eq!(
        vec![
            "/api/v1/chats/42/messages/10/thread?limit=50",
            "/api/v1/chats/42/messages/10/thread?limit=50&after_id=59"
        ],
        server.asked()
    );
    assert_eq!(51, thread.bubbles()?.len());
    assert_eq!(10, thread.root_id());
    Ok(())
}

/// The read stays OUT of the cache: a row it does not hold stays out, so no page starts from the
/// wrong place — while a row it holds takes the true count and the edit.
#[test]
fn the_read_refreshes_held_rows_and_adds_nothing_to_the_cache() -> StoreResult<()> {
    let rig = ChatRig::new()?;
    apply(&rig, &[root(10, 1), m(50)])?;
    let server = Server::new();
    server.then(
        "/chats/42/messages/10/thread",
        &[(
            200,
            Some(&page(&[
                M {
                    body: "edited",
                    edit_seq: Some(2),
                    ..root(10, 3)
                },
                reply(11, 10),
                reply(300, 10),
            ])),
        )],
    );
    let thread = build(&rig, &server, 10)?;

    rig.drive(thread.load())?;

    let held = rig.chats.message(10)?.unwrap();
    assert_eq!((Some(3), "edited"), (held.reply_count, held.body.as_str()));
    assert_eq!(None, rig.chats.message(11)?);
    assert_eq!(None, rig.chats.message(300)?);
    assert_eq!(
        vec![50, 10],
        rig.chats
            .messages(CHAT, None, 50)?
            .iter()
            .map(|message| message.id)
            .collect::<Vec<_>>()
    );
    assert_eq!(vec![10, 11, 300], ids(&thread.bubbles()?));
    Ok(())
}

/// A reply that reaches the cache after the read joins the surface and raises the surface's root —
/// once.
#[test]
fn a_later_reply_raises_the_surfaces_root_once() -> StoreResult<()> {
    let rig = ChatRig::new()?;
    let server = Server::new();
    server.then(
        "/chats/42/messages/10/thread",
        &[(200, Some(&page(&[root(10, 1), reply(11, 10)])))],
    );
    let thread = build(&rig, &server, 10)?;
    rig.drive(thread.load())?;

    rig.chats
        .apply(&reply(12, 10).dto(), SeqRoute::LiveFrame, true)?;

    assert_eq!(vec![10, 11, 12], ids(&thread.bubbles()?));
    assert_eq!(Some(2), thread.bubbles()?[0].message.reply_count);
    assert!(!thread.heard(&reply(13, 99).dto()));
    assert!(thread.heard(&reply(12, 10).dto()));
    assert_eq!(Some(2), thread.bubbles()?[0].message.reply_count);
    Ok(())
}

/// A root the cache holds is counted by the cache; the surface draws that copy, and does not count
/// it again.
#[test]
fn a_held_root_is_not_counted_twice() -> StoreResult<()> {
    let rig = ChatRig::new()?;
    apply(&rig, &[root(10, 1), reply(11, 10)])?;
    let server = Server::new();
    server.then(
        "/chats/42/messages/10/thread",
        &[(200, Some(&page(&[root(10, 1), reply(11, 10)])))],
    );
    let thread = build(&rig, &server, 10)?;
    rig.drive(thread.load())?;

    rig.chats
        .apply(&reply(12, 10).dto(), SeqRoute::LiveFrame, true)?;

    assert_eq!(Some(2), rig.chats.message(10)?.unwrap().reply_count);
    assert_eq!(Some(2), thread.bubbles()?[0].message.reply_count);
    Ok(())
}

/// An older copy never undoes an edit, but brings the true count.
#[test]
fn an_older_copy_keeps_the_edit_and_takes_the_count() -> StoreResult<()> {
    let rig = ChatRig::new()?;
    let server = Server::new();
    server.then(
        "/chats/42/messages/10/thread",
        &[(
            200,
            Some(&page(&[M {
                body: "new",
                edit_seq: Some(4),
                ..root(10, 1)
            }])),
        )],
    );
    let thread = build(&rig, &server, 10)?;
    rig.drive(thread.load())?;

    assert!(
        thread.heard(
            &M {
                body: "old",
                edit_seq: Some(3),
                ..root(10, 5)
            }
            .dto()
        )
    );

    let bubbles = thread.bubbles()?;
    assert_eq!(1, bubbles.len());
    let root = &bubbles[0].message;
    assert_eq!(
        ("new", Some(4), Some(5)),
        (root.body.as_str(), root.edit_seq, root.reply_count)
    );
    Ok(())
}

/// A read that fails says why and keeps what was drawn.
#[test]
fn a_failed_read_keeps_what_is_held() -> StoreResult<()> {
    let rig = ChatRig::new()?;
    apply(&rig, &[root(10, 1), reply(11, 10)])?;
    let server = Server::new();
    server.then(
        "/chats/42/messages/10/thread",
        &[
            // A refusal, not a 502: a read is safe to repeat, and the client would try a 502 again
            // by itself.
            (
                403,
                Some(r#"{"error": {"code": "not_in_family", "message": "no"}}"#),
            ),
            (200, Some(&page(&[root(10, 1), reply(11, 10)]))),
        ],
    );
    let thread = build(&rig, &server, 10)?;

    assert!(rig.drive(thread.load())?.is_some());

    assert!(!thread.loaded());
    assert!(thread.failure().is_some());
    assert_eq!(vec![10, 11], ids(&thread.bubbles()?));

    // The next read that answers clears it.
    assert_eq!(None, rig.drive(thread.load())?);
    assert_eq!(None, thread.failure());
    assert!(thread.loaded());
    Ok(())
}

/// This reader's replies on their way belong to the chain when they answer the root or any row of
/// it.
#[test]
fn pending_replies_to_the_chain_are_its_own() -> StoreResult<()> {
    let rig = ChatRig::new()?;
    let server = Server::new();
    server.then(
        "/chats/42/messages/10/thread",
        &[(200, Some(&page(&[root(10, 1), reply(11, 10)])))],
    );
    let thread = build(&rig, &server, 10)?;
    rig.drive(thread.load())?;
    let answering = |key: &str, body: &str, to: Option<i64>| OutboxRow {
        reply_to_message_id: to,
        ..OutboxRow::new(key, CHAT, body)
    };
    rig.outbox.queue(&answering("a", "8?", Some(11)))?;
    rig.outbox.queue(&answering("b", "sure", Some(10)))?;
    rig.outbox.queue(&answering("c", "elsewhere", Some(99)))?;
    rig.outbox.queue(&answering("d", "no reply", None))?;

    let mut pending: Vec<String> = thread
        .pending()?
        .into_iter()
        .map(|row| row.client_msg_id)
        .collect();
    pending.sort();
    assert_eq!(vec!["a", "b"], pending);
    Ok(())
}

/// A blocked member's row is the hidden row here too, and revealing it is this surface's.
#[test]
fn a_blocked_members_row_is_hidden_on_the_surface() -> StoreResult<()> {
    let rig = ChatRig::new()?;
    apply(
        &rig,
        &[
            root(10, 1),
            M {
                sender: 5,
                ..reply(11, 10)
            },
        ],
    )?;
    rig.chats.replace_blocked(&[5])?;
    let thread = build(&rig, &Server::new(), 10)?;

    assert!(thread.bubbles()?[1].hidden);
    assert!(!thread.bubbles()?[1].revealed);
    thread.reveal(11);
    assert!(thread.bubbles()?[1].revealed);
    thread.hide(11);
    assert!(!thread.bubbles()?[1].revealed);
    Ok(())
}

/// The chain is this chat's: a row rooted at the same id in another chat is neither held here nor
/// heard.
#[test]
fn a_chain_is_its_own_chats() -> StoreResult<()> {
    let rig = ChatRig::new()?;
    rig.chats.replace(
        &[
            ChatRowDto::new(ChatDto::new(CHAT, "family", "The Smiths")),
            ChatRowDto::new(ChatDto::new(43, "direct", "Bob")),
        ],
        None,
    )?;
    rig.chats.apply_page(
        &[
            root(10, 1).dto(),
            reply(11, 10).dto(),
            MessageDto {
                chat_id: 43,
                ..reply(20, 10).dto()
            },
        ],
        SeqRoute::Evidence,
        true,
    )?;
    let thread = build(&rig, &Server::new(), 10)?;

    let bubbles = thread.bubbles()?;

    assert_eq!(vec![10, 11], ids(&bubbles));
    assert_eq!(Some(1), bubbles[0].message.reply_count);
    assert!(!thread.heard(&MessageDto {
        chat_id: 43,
        ..reply(21, 10).dto()
    }));
    Ok(())
}

/// A row the cache holds is drawn from the fresher copy: a later edit and a later poll state reach
/// the surface.
#[test]
fn a_held_row_draws_the_caches_fresher_state() -> StoreResult<()> {
    let rig = ChatRig::new()?;
    let poll = PollDto {
        poll_seq: 1,
        closed: false,
        options: vec![
            PollOptionDto {
                id: 1,
                text: "Pizza".to_string(),
                votes: Vec::new(),
            },
            PollOptionDto {
                id: 2,
                text: "Pasta".to_string(),
                votes: Vec::new(),
            },
        ],
    };
    rig.chats.apply_page(
        &[
            MessageDto {
                poll: Some(poll.clone()),
                ..root(10, 1).dto()
            },
            M {
                body: "a",
                edit_seq: Some(1),
                ..reply(11, 10)
            }
            .dto(),
        ],
        SeqRoute::Evidence,
        true,
    )?;
    let server = Server::new();
    server.then(
        "/chats/42/messages/10/thread",
        &[(
            200,
            Some(&page(&[
                root(10, 1),
                M {
                    body: "a",
                    edit_seq: Some(1),
                    ..reply(11, 10)
                },
            ])),
        )],
    );
    let thread = build(&rig, &server, 10)?;
    rig.drive(thread.load())?;

    rig.chats.apply(
        &M {
            body: "b",
            edit_seq: Some(2),
            ..reply(11, 10)
        }
        .dto(),
        SeqRoute::Evidence,
        true,
    )?;
    rig.chats.apply_poll(
        CHAT,
        10,
        &PollDto {
            poll_seq: 2,
            closed: true,
            ..poll
        },
        SeqRoute::LiveFrame,
    )?;

    let bubbles = thread.bubbles()?;
    assert_eq!("b", bubbles[1].message.body);
    assert!(bubbles[0].message.poll.as_ref().unwrap().closed);
    Ok(())
}

/// Reactions come from the copy with the newer reaction sequence, whichever copy that is.
#[test]
fn reactions_come_from_the_newer_sequence() -> StoreResult<()> {
    let rig = ChatRig::new()?;
    let thread = build(&rig, &Server::new(), 10)?;

    assert!(thread.heard(&reacted(m(10), 5, "👍")));
    assert!(thread.heard(&reacted(m(10), 3, "❤️")));
    let bubbles = thread.bubbles()?;
    assert_eq!(1, bubbles.len());
    let reactions = bubbles[0].message.reactions.as_ref().unwrap();
    assert_eq!(1, reactions.len());
    assert_eq!("👍", reactions[0].emoji);

    assert!(thread.heard(&reacted(m(10), 6, "😂")));
    let bubbles = thread.bubbles()?;
    let reactions = bubbles[0].message.reactions.as_ref().unwrap();
    assert_eq!(1, reactions.len());
    assert_eq!("😂", reactions[0].emoji);
    // The root itself is no reply to itself.
    assert_eq!(None, bubbles[0].message.reply_count);
    Ok(())
}

/// A reaction on a row the cache does not hold is decided against the surface's copy, and its
/// answer lands there — unless the surface already holds a newer reaction sequence.
#[test]
fn a_reaction_on_a_row_the_cache_does_not_hold_lands_on_the_surface() -> StoreResult<()> {
    let rig = ChatRig::new()?;
    let server = Server::new();
    server
        .then(
            "/chats/42/messages/10/thread",
            &[(
                200,
                Some(&page(&[
                    root(10, 3),
                    reply(11, 10),
                    reply(12, 10),
                    reply(13, 10),
                ])),
            )],
        )
        .on(
            "/chats/42/messages/11/reaction",
            r#"{"message_id": 11, "reaction_seq": 3, "reactions": [{"user_id": 7, "emoji": "👍"}]}"#,
        )
        .on(
            "/chats/42/messages/12/reaction",
            r#"{"message_id": 12, "reaction_seq": 3, "reactions": [{"user_id": 7, "emoji": "👍"}]}"#,
        )
        .on(
            "/chats/42/messages/13/reaction",
            r#"{"message_id": 13, "reaction_seq": 5, "reactions": [{"user_id": 7, "emoji": "👍"}]}"#,
        );
    let thread = build(&rig, &server, 10)?;
    rig.drive(thread.load())?;
    thread.heard(&reacted(reply(12, 10), 5, "❤️"));
    thread.heard(&reacted(reply(13, 10), 5, "❤️"));

    assert_eq!(None, rig.drive(thread.react(11, "👍"))?);
    assert_eq!(None, rig.drive(thread.react(12, "👍"))?);
    assert_eq!(None, rig.drive(thread.react(13, "👍"))?);
    assert_eq!(None, rig.drive(thread.react(99, "👍"))?);

    let bubbles = thread.bubbles()?;
    let only = |at: usize| {
        let message = &bubbles[at].message;
        let reactions = message.reactions.as_ref().unwrap();
        assert_eq!(1, reactions.len());
        (reactions[0].emoji.clone(), message.reaction_seq.unwrap())
    };
    assert_eq!(("👍".to_string(), 3), only(1));
    assert_eq!(("❤️".to_string(), 5), only(2));
    assert_eq!(("👍".to_string(), 5), only(3));
    assert_eq!(None, rig.chats.message(11)?);
    assert_eq!(4, server.asked().len());
    Ok(())
}

/// The root arriving is no reply to itself: it raises nothing, and the first reply after it raises
/// one.
#[test]
fn the_root_arriving_raises_nothing() -> StoreResult<()> {
    let rig = ChatRig::new()?;
    let thread = build(&rig, &Server::new(), 10)?;

    assert!(thread.heard(&m(10).dto()));
    let bubbles = thread.bubbles()?;
    assert_eq!(1, bubbles.len());
    assert_eq!(None, bubbles[0].message.reply_count);

    assert!(thread.heard(&reply(11, 10).dto()));
    assert_eq!(Some(1), thread.bubbles()?[0].message.reply_count);
    Ok(())
}

/// Before anything is read or held, a reply to the root is still the chain's.
#[test]
fn a_reply_to_the_root_is_the_chains_before_anything_is_read() -> StoreResult<()> {
    let rig = ChatRig::new()?;
    let thread = build(&rig, &Server::new(), 10)?;
    rig.outbox.queue(&OutboxRow {
        reply_to_message_id: Some(10),
        ..OutboxRow::new("a", CHAT, "first!")
    })?;

    let pending = thread.pending()?;
    assert_eq!(1, pending.len());
    assert_eq!("a", pending[0].client_msg_id);
    Ok(())
}

/// A full page that cannot move the cursor ends the read, rather than asking the same thing for
/// ever.
#[test]
fn a_full_page_that_cannot_move_the_cursor_ends_the_read() -> StoreResult<()> {
    let rig = ChatRig::new()?;
    let full: Vec<M> = (10..60)
        .map(|id| if id == 10 { m(10) } else { reply(id, 10) })
        .collect();
    let server = Server::new();
    server.then("/chats/42/messages/10/thread", &[(200, Some(&page(&full)))]);
    let thread = build(&rig, &server, 10)?;

    assert_eq!(None, rig.drive(thread.load())?);

    assert_eq!(2, server.asked().len());
    assert!(thread.loaded());
    Ok(())
}
