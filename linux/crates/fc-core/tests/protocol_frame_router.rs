//! What a live frame does to the cache (docs/protocol.md, "Server → client", "Semantics") — a port
//! of `win/tests/FamilyConnect.Core.Tests/Protocol/FrameRouterTests.cs`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use fc_core::protocol::{
    AiErrorReason, ApiError, AttachmentDto, BirthdayDto, ChatDto, ChatRowDto, FrameRouter,
    IceCandidate, MemberDto, MessageDto, NoteDto, PackItemDto, PollDto, PollOptionDto, ReactionDto,
    ServerFrame, UserDto,
};
use fc_core::store::{
    BoardStore, ChatStore, Database, OutboxRow, OutboxStore, PackStore, SeqRoute, StoreResult,
};

const ME: i64 = 7;

struct Fixture {
    database: Rc<Database>,
    chats: Rc<ChatStore>,
    board: Rc<BoardStore>,
    router: FrameRouter,
}

fn fixture() -> StoreResult<Fixture> {
    let database = Database::open_in_memory()?;
    let chats = Rc::new(ChatStore::new(&database).with_me(|| ME));
    let board = Rc::new(BoardStore::new(&database));
    let router = FrameRouter::new(chats.clone(), board.clone(), None);
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
    Ok(Fixture {
        database,
        chats,
        board,
        router,
    })
}

/// `Message(id, chat = 42, body = "Dinner at 7?", sender = 9, editSeq = null)`.
fn message(id: i64, chat: i64, body: &str, sender: i64, edit_seq: Option<i64>) -> MessageDto {
    MessageDto {
        edit_seq,
        edited_at: edit_seq.map(|_| "2026-09-12T10:05:00Z".to_string()),
        ..MessageDto::new(id, chat, sender, None, body, "2026-09-12T10:00:00Z")
    }
}

fn dinner(id: i64) -> MessageDto {
    message(id, 42, "Dinner at 7?", 9, None)
}

fn listed(chats: &ChatStore, chat_id: i64) -> StoreResult<ChatRowDto> {
    Ok(chats.chat(chat_id)?.expect("the chat is held"))
}

fn held(chats: &ChatStore, message_id: i64) -> StoreResult<MessageDto> {
    Ok(chats.message(message_id)?.expect("the message is held"))
}

fn one<T>(mut items: Vec<T>) -> T {
    assert_eq!(items.len(), 1, "exactly one");
    items.remove(0)
}

/// Records what an event said, in order.
fn recorder<T: Clone + 'static>() -> (Rc<RefCell<Vec<T>>>, impl Fn(&T) + 'static) {
    let heard = Rc::new(RefCell::new(Vec::new()));
    let into = heard.clone();
    (heard, move |value: &T| {
        into.borrow_mut().push(value.clone())
    })
}

/// A message of the reader's OWN — the fan-out reaches their other devices as an ordinary
/// `message` frame — is stored and announced and counts against nobody.
#[test]
fn your_own_message_from_your_other_device_counts_against_nobody() -> StoreResult<()> {
    let f = fixture()?;
    f.router.hear(&ServerFrame::Message(message(
        1338,
        42,
        "Dinner at 7?",
        ME,
        None,
    )))?;

    assert_eq!(held(&f.chats, 1338)?.body, "Dinner at 7?");
    assert_eq!(listed(&f.chats, 42)?.unread_count, 0);
    Ok(())
}

/// A PAGE MAY NOT RAISE THE COUNT: the list read that preceded it already counted every message on
/// it, so counting them again doubles the badge.
#[test]
fn a_page_of_messages_does_not_count_what_the_list_already_counted() -> StoreResult<()> {
    let f = fixture()?;
    f.chats.replace(
        &[ChatRowDto {
            unread_count: 2,
            ..ChatRowDto::new(ChatDto::new(42, "family", "The Smiths"))
        }],
        None,
    )?;

    f.chats
        .apply_page(&[dinner(1338), dinner(1339)], SeqRoute::CatchUpPage, true)?;

    assert_eq!(listed(&f.chats, 42)?.unread_count, 2);
    Ok(())
}

#[test]
fn a_message_is_stored_and_announced() -> StoreResult<()> {
    let f = fixture()?;
    let (arrived, on_arrived) = recorder::<MessageDto>();
    let (touched, on_touched) = recorder::<i64>();
    f.router.on_arrived().subscribe(on_arrived);
    f.router.on_chat_changed().subscribe(on_touched);

    f.router.hear(&ServerFrame::Message(dinner(1338)))?;

    assert_eq!(one(arrived.borrow().clone()).id, 1338);
    assert_eq!(held(&f.chats, 1338)?.body, "Dinner at 7?");
    assert_eq!(*touched.borrow(), [42]);
    assert_eq!(listed(&f.chats, 42)?.unread_count, 1);
    Ok(())
}

/// AN EDIT IS NOT A MESSAGE: it raises no arrival, and it moves the chat's edit cursor, which
/// nothing else may. It also carries the WHOLE message — the assistant's picture answer arrives as
/// an attachment added by exactly this frame.
#[test]
fn an_edit_announces_itself_differently_and_moves_the_edit_cursor() -> StoreResult<()> {
    let f = fixture()?;
    f.chats.apply(&dinner(1338), SeqRoute::CatchUpPage, true)?;
    let arrivals = Rc::new(Cell::new(0));
    let counted = arrivals.clone();
    f.router
        .on_arrived()
        .subscribe(move |_| counted.set(counted.get() + 1));
    let (edited, on_edited) = recorder::<MessageDto>();
    f.router.on_edited().subscribe(on_edited);

    f.router.hear(&ServerFrame::MessageEdited(MessageDto {
        attachments: Some(vec![AttachmentDto {
            width: Some(1600),
            height: Some(1200),
            ..AttachmentDto::new(34, "photo")
        }]),
        ..message(1338, 42, "Dinner at 8?", 9, Some(91))
    }))?;

    assert_eq!(arrivals.get(), 0);
    assert_eq!(one(edited.borrow().clone()).body, "Dinner at 8?");
    assert_eq!(held(&f.chats, 1338)?.body, "Dinner at 8?");
    // The whole message, not the body alone.
    assert_eq!(one(held(&f.chats, 1338)?.media().to_vec()).id, 34);
    assert_eq!(listed(&f.chats, 42)?.max_edit_seq, Some(91));
    Ok(())
}

/// A live reply raises its held root's count; an edit frame does not — not even one for a reply
/// this device never held, because an edit is not an arrival (docs/protocol.md, "Threads").
#[test]
fn a_live_reply_raises_its_root_and_an_edit_frame_does_not() -> StoreResult<()> {
    let f = fixture()?;
    f.chats.apply(
        &MessageDto {
            reply_count: Some(1),
            ..dinner(1338)
        },
        SeqRoute::Evidence,
        true,
    )?;

    f.router.hear(&ServerFrame::Message(MessageDto {
        thread_root_id: Some(1338),
        ..dinner(1339)
    }))?;
    assert_eq!(held(&f.chats, 1338)?.reply_count, Some(2));

    f.router.hear(&ServerFrame::MessageEdited(MessageDto {
        thread_root_id: Some(1338),
        ..message(1340, 42, "at 8", 9, Some(92))
    }))?;
    assert_eq!(held(&f.chats, 1338)?.reply_count, Some(2));
    assert_eq!(held(&f.chats, 1340)?.body, "at 8");
    // Held, and not where the next catch-up starts: this device never received 1340 itself, and an
    // edit says nothing about the messages before it.
    assert_eq!(f.chats.catch_up_cursor(42)?, Some(1339));
    Ok(())
}

/// A `read` FRAME IS TWO DIFFERENT FACTS. One naming YOURSELF is your own marker from your other
/// device; one naming somebody else is roster data, and clearing your unread count from it would
/// be reading somebody else's reading as your own.
#[test]
fn a_read_frame_naming_you_is_your_marker_and_one_naming_anybody_else_is_not() -> StoreResult<()> {
    let f = fixture()?;
    // As frames, because a live frame is the only route that may raise the count.
    f.router.hear(&ServerFrame::Message(dinner(1338)))?;
    f.router.hear(&ServerFrame::Message(dinner(1339)))?;
    assert_eq!(listed(&f.chats, 42)?.unread_count, 2);

    // Somebody else read it: nothing of yours moves.
    f.router.hear(&ServerFrame::Read {
        chat_id: 42,
        user_id: 9,
        last_read_message_id: 1339,
    })?;
    assert_eq!(listed(&f.chats, 42)?.unread_count, 2);
    assert_eq!(listed(&f.chats, 42)?.last_read_message_id, 0);

    // You read it, on your laptop: the marker moves and the count follows.
    f.router.hear(&ServerFrame::Read {
        chat_id: 42,
        user_id: ME,
        last_read_message_id: 1339,
    })?;
    assert_eq!(listed(&f.chats, 42)?.unread_count, 0);
    assert_eq!(listed(&f.chats, 42)?.last_read_message_id, 1339);

    // And it is MONOTONIC: a frame still in flight while you read on cannot walk it back.
    f.router.hear(&ServerFrame::Read {
        chat_id: 42,
        user_id: ME,
        last_read_message_id: 1000,
    })?;
    assert_eq!(listed(&f.chats, 42)?.last_read_message_id, 1339);
    Ok(())
}

/// A peer's read marker is drawn in a DIRECT chat only: in the family chat a per-member seen state
/// over N members is a row of faces nobody asked for, and a per-member ABSENCE is exactly what a
/// blocker's suppressed inward frames would put on screen.
#[test]
fn a_peers_read_marker_is_seen_in_a_direct_chat_and_nowhere_else() -> StoreResult<()> {
    let f = fixture()?;
    let (seen, on_seen) = recorder::<(i64, i64, i64)>();
    f.router.on_peer_read().subscribe(on_seen);

    f.router.hear(&ServerFrame::Read {
        chat_id: 42,
        user_id: 9,
        last_read_message_id: 1339,
    })?;
    f.router.hear(&ServerFrame::Read {
        chat_id: 43,
        user_id: 11,
        last_read_message_id: 500,
    })?;

    let chats: Vec<i64> = seen.borrow().iter().map(|(chat, _, _)| *chat).collect();
    assert_eq!(chats, [43]);
    Ok(())
}

/// A live frame is a complete statement about that chat's sequence, so the cursor follows it EVEN
/// FOR A MESSAGE THIS DEVICE DOES NOT HOLD — the state is dropped, the sequence still happened,
/// and a cursor left behind asks for a page that answers the same nothing.
#[test]
fn a_reaction_frame_moves_the_cursor_even_when_its_message_is_unknown() -> StoreResult<()> {
    let f = fixture()?;
    f.router.hear(&ServerFrame::Reactions {
        chat_id: 42,
        message_id: 9999,
        reaction_seq: 124,
        value: vec![ReactionDto::new(9, "❤️")],
    })?;
    assert_eq!(f.chats.message(9999)?, None);
    assert_eq!(listed(&f.chats, 42)?.max_reaction_seq, Some(124));

    f.chats.apply(&dinner(1338), SeqRoute::CatchUpPage, true)?;
    f.router.hear(&ServerFrame::Reactions {
        chat_id: 42,
        message_id: 1338,
        reaction_seq: 125,
        value: vec![ReactionDto::new(9, "👍")],
    })?;
    assert_eq!(
        one(held(&f.chats, 1338)?.reactions.expect("reactions")).emoji,
        "👍"
    );
    assert_eq!(listed(&f.chats, 42)?.max_reaction_seq, Some(125));
    Ok(())
}

#[test]
fn a_poll_frame_is_complete_state_and_moves_its_own_cursor() -> StoreResult<()> {
    let f = fixture()?;
    f.router.hear(&ServerFrame::Message(message(
        1340,
        42,
        "Pizza or pasta?",
        9,
        None,
    )))?;
    assert_eq!(listed(&f.chats, 42)?.unread_count, 1);

    f.router.hear(&ServerFrame::Poll {
        chat_id: 42,
        message_id: 1340,
        value: PollDto {
            poll_seq: 88,
            closed: false,
            options: vec![
                PollOptionDto {
                    id: 5,
                    text: "Pizza".into(),
                    votes: vec![7],
                },
                PollOptionDto {
                    id: 6,
                    text: "Pasta".into(),
                    votes: vec![],
                },
            ],
        },
    })?;
    assert_eq!(
        held(&f.chats, 1340)?.poll.expect("a poll").options[0].votes,
        [7]
    );
    assert_eq!(listed(&f.chats, 42)?.max_poll_seq, Some(88));
    // Never an unread count and never a notification: a vote is not a message, so the count the
    // message itself raised is all there is.
    assert_eq!(listed(&f.chats, 42)?.unread_count, 1);

    // And a vote on a poll this device has never seen moves the cursor all the same.
    f.router.hear(&ServerFrame::Poll {
        chat_id: 42,
        message_id: 9999,
        value: PollDto {
            poll_seq: 89,
            closed: true,
            options: vec![],
        },
    })?;
    assert_eq!(f.chats.message(9999)?, None);
    assert_eq!(listed(&f.chats, 42)?.max_poll_seq, Some(89));
    Ok(())
}

/// `new NoteDto(id, author, "text", text, BoardSeq: seq)`.
fn note(id: i64, author: i64, text: &str, seq: i64) -> NoteDto {
    NoteDto {
        kind: Some("text".to_string()),
        text: Some(text.to_string()),
        board_seq: seq,
        ..NoteDto::new(id, author)
    }
}

#[test]
fn a_note_frame_is_applied_under_its_own_sequence() -> StoreResult<()> {
    let f = fixture()?;
    let (changed, on_changed) = recorder::<NoteDto>();
    f.router.on_board_changed().subscribe(on_changed);

    f.router
        .hear(&ServerFrame::BoardNote(note(12, 7, "Milk", 11)))?;
    assert_eq!(one(f.board.notes()?).text.as_deref(), Some("Milk"));
    assert_eq!(one(changed.borrow().clone()).id, 12);

    // An out-of-order frame cannot undo a newer move.
    f.router
        .hear(&ServerFrame::BoardNote(note(12, 7, "Bread", 10)))?;
    assert_eq!(one(f.board.notes()?).text.as_deref(), Some("Milk"));

    // And a tombstone takes it down.
    f.router.hear(&ServerFrame::BoardNote(NoteDto {
        board_seq: 12,
        deleted: true,
        ..NoteDto::new(12, 7)
    }))?;
    assert!(f.board.notes()?.is_empty());
    Ok(())
}

fn member(id: i64, username: &str, name: &str) -> MemberDto {
    MemberDto::new(id, username, name)
}

fn held_member(chats: &ChatStore, user_id: i64) -> StoreResult<MemberDto> {
    Ok(chats.member(user_id)?.expect("the member is held"))
}

#[test]
fn the_roster_follows_its_four_frames() -> StoreResult<()> {
    let f = fixture()?;
    let changes = Rc::new(Cell::new(0));
    let counted = changes.clone();
    f.router
        .on_roster_changed()
        .subscribe(move |_| counted.set(counted.get() + 1));
    f.chats.replace_members(
        &[
            MemberDto {
                role: Some("owner".into()),
                ..member(7, "anna", "Anna")
            },
            MemberDto {
                role: Some("member".into()),
                birthday: Some(BirthdayDto { month: 3, day: 14 }),
                ..member(11, "bob", "Bob")
            },
        ],
        &[],
    )?;

    // A join, carrying a User — so a member, never an owner by arriving.
    f.router.hear(&ServerFrame::MemberJoined {
        family_id: Some(3),
        user: UserDto::new(12, "kid", "Kid"),
        password_reset: None,
    })?;
    assert_eq!(held_member(&f.chats, 12)?.role.as_deref(), Some("member"));

    // A frame that says nothing about a birthday leaves the one this device knows alone.
    f.router.hear(&ServerFrame::MemberJoined {
        family_id: Some(3),
        user: UserDto::new(11, "bob", "Bobby"),
        password_reset: None,
    })?;
    assert_eq!(held_member(&f.chats, 11)?.display_name, "Bobby");
    assert_eq!(
        held_member(&f.chats, 11)?.birthday,
        Some(BirthdayDto { month: 3, day: 14 })
    );

    // A leave keeps the row: their messages keep their author.
    f.router.hear(&ServerFrame::MemberLeft {
        family_id: Some(3),
        user_id: 12,
    })?;
    assert!(held_member(&f.chats, 12)?.has_left);
    assert_eq!(held_member(&f.chats, 12)?.role, None);

    // And a REJOIN takes the flag back off — as a member, because arriving is not owning: the
    // family passes on by its own frame and never by somebody walking back in.
    f.router.hear(&ServerFrame::MemberJoined {
        family_id: Some(3),
        user: UserDto::new(12, "kid", "Kid"),
        password_reset: None,
    })?;
    assert!(!held_member(&f.chats, 12)?.has_left);
    assert_eq!(held_member(&f.chats, 12)?.role.as_deref(), Some("member"));
    assert!(!held_member(&f.chats, 12)?.owner());
    // The owner is still the owner: one join did not move it.
    assert!(held_member(&f.chats, 7)?.owner());

    // A deleted account is the one write whose job is to WIPE.
    f.router.hear(&ServerFrame::MemberDeleted {
        family_id: Some(3),
        member: MemberDto {
            deleted: true,
            ..member(11, "", "Deleted account")
        },
    })?;
    let gone = held_member(&f.chats, 11)?;
    assert!(gone.deleted);
    assert!(gone.has_left);
    assert_eq!(gone.avatar_version, 0);
    assert_eq!(gone.birthday, None);

    // And the family passes on: one owner, so whoever held it stops holding it.
    f.chats
        .joined(&UserDto::new(14, "gran", "Gran"), None, None)?;
    f.router.hear(&ServerFrame::FamilyOwner {
        family_id: Some(3),
        user_id: 14,
    })?;
    assert!(held_member(&f.chats, 14)?.owner());
    assert!(!held_member(&f.chats, 7)?.owner());
    // Six frames, six announcements: three joins, a leave, a deletion and a new owner.
    assert_eq!(changes.get(), 6);
    Ok(())
}

/// `member_blocked` carries FULL CURRENT STATE rather than an event, so an unblock is the same
/// frame with `false` — and it is the one frame about another member that reaches one person's own
/// devices and stops there.
#[test]
fn blocking_is_a_state_set_and_the_chat_follows_it() -> StoreResult<()> {
    let f = fixture()?;
    f.chats.apply(
        &message(500, 43, "Are we still on?", 11, None),
        SeqRoute::CatchUpPage,
        true,
    )?;
    let (states, on_state) = recorder::<(i64, bool)>();
    f.router.on_block_changed().subscribe(on_state);

    f.router.hear(&ServerFrame::MemberBlocked {
        user_id: 11,
        blocked: true,
    })?;
    assert!(f.chats.is_blocked(11)?);
    assert!(!f.chats.is_listed(43)?);

    f.router.hear(&ServerFrame::MemberBlocked {
        user_id: 11,
        blocked: false,
    })?;
    assert!(!f.chats.is_blocked(11)?);
    assert!(f.chats.is_listed(43)?);
    // Nothing about the chat was ever deleted.
    assert_eq!(held(&f.chats, 500)?.body, "Are we still on?");
    let blocked: Vec<bool> = states
        .borrow()
        .iter()
        .map(|(_, blocked)| *blocked)
        .collect();
    assert_eq!(blocked, [true, false]);
    Ok(())
}

/// The assistant's deltas are NOT stored: the finished answer lands as `message_edited`, and a
/// half-written body written into the cache is what a relaunch mid-answer would draw for ever.
#[test]
fn the_assistants_deltas_are_passed_on_and_not_stored() -> StoreResult<()> {
    let f = fixture()?;
    f.chats
        .apply(&message(1341, 42, "", 9, None), SeqRoute::CatchUpPage, true)?;
    let text = Rc::new(RefCell::new(String::new()));
    let written = text.clone();
    f.router
        .on_ai_delta()
        .subscribe(move |(_, _, piece)| written.borrow_mut().push_str(piece));
    let (stopped, on_stopped) = recorder::<(i64, i64, Option<AiErrorReason>)>();
    f.router.on_ai_stopped().subscribe(on_stopped);

    f.router.hear(&ServerFrame::AiDelta {
        chat_id: 42,
        message_id: 1341,
        text: "Once ".into(),
    })?;
    f.router.hear(&ServerFrame::AiDelta {
        chat_id: 42,
        message_id: 1341,
        text: "upon".into(),
    })?;
    f.router.hear(&ServerFrame::AiError {
        chat_id: 42,
        message_id: 1341,
        reason: None,
    })?;
    // The reason is passed on with the failure, for the row to remember which sentence it says.
    f.router.hear(&ServerFrame::AiError {
        chat_id: 42,
        message_id: 1341,
        reason: Some(AiErrorReason::Refused),
    })?;

    assert_eq!(*text.borrow(), "Once upon");
    let reasons: Vec<Option<AiErrorReason>> = stopped
        .borrow()
        .iter()
        .map(|(_, _, reason)| *reason)
        .collect();
    assert_eq!(reasons, [None, Some(AiErrorReason::Refused)]);
    assert_eq!(held(&f.chats, 1341)?.body, "");
    Ok(())
}

/// An error that answers a send belongs to the pipeline, which is waiting for it; one that answers
/// a call belongs to the call layer; anything else is nobody's, and is spoken.
#[test]
fn an_error_goes_to_whoever_is_waiting_for_it() -> StoreResult<()> {
    let f = fixture()?;
    let (refusals, on_refused) = recorder::<ApiError>();
    let (calls, on_call) = recorder::<ServerFrame>();
    f.router.on_refused().subscribe(on_refused);
    f.router.on_call().subscribe(on_call);

    f.router.hear(&ServerFrame::Error {
        code: "not_chat_member".into(),
        detail: "no".into(),
        client_msg_id: Some("8f14e45f".into()),
        call_id: None,
    })?;
    f.router.hear(&ServerFrame::Error {
        code: "peer_busy".into(),
        detail: "busy".into(),
        client_msg_id: None,
        call_id: Some("6a1f0c3e".into()),
    })?;
    f.router.hear(&ServerFrame::Error {
        code: "internal".into(),
        detail: "oh dear".into(),
        client_msg_id: None,
        call_id: None,
    })?;
    f.router.hear(&ServerFrame::Pong)?;

    let codes: Vec<String> = refusals
        .borrow()
        .iter()
        .map(|error| error.code.clone())
        .collect();
    assert_eq!(codes, ["internal"]);
    assert_eq!(calls.borrow().len(), 1);
    Ok(())
}

#[test]
fn a_call_frame_is_passed_on_whole_because_signalling_is_not_a_cache() -> StoreResult<()> {
    let f = fixture()?;
    let (heard, on_call) = recorder::<ServerFrame>();
    f.router.on_call().subscribe(on_call);

    f.router.hear(&ServerFrame::CallOffer {
        call_id: "6a1f0c3e".into(),
        chat_id: 43,
        from_user_id: 11,
        sdp: "v=0".into(),
        video: false,
    })?;
    f.router.hear(&ServerFrame::CallRinging {
        call_id: "6a1f0c3e".into(),
    })?;
    f.router.hear(&ServerFrame::CallAnswer {
        call_id: "6a1f0c3e".into(),
        sdp: "v=0".into(),
    })?;
    f.router.hear(&ServerFrame::CallIce {
        call_id: "6a1f0c3e".into(),
        candidate: IceCandidate::new("candidate:1", None, None),
    })?;
    f.router.hear(&ServerFrame::CallEnd {
        call_id: "6a1f0c3e".into(),
        reason: "hangup".into(),
    })?;

    assert_eq!(heard.borrow().len(), 5);
    Ok(())
}

// ---- the sticker pack (docs/protocol.md, "Sticker pack") -------------------------------------

/// `PackItem(id, seq, by = 9)`: a 512×512 WebP of 4096 bytes.
fn pack_item(id: i64, seq: i64, by: i64) -> PackItemDto {
    PackItemDto {
        added_by: by,
        attachment: Some(AttachmentDto {
            mime: Some("image/webp".to_string()),
            size: Some(4096),
            width: Some(512),
            height: Some(512),
            ..AttachmentDto::new(70 + id, "photo")
        }),
        created_at: Some("2026-09-13T10:00:00Z".to_string()),
        pack_seq: seq,
        ..PackItemDto::new(id)
    }
}

fn routed_with_pack(f: &Fixture) -> (Rc<PackStore>, FrameRouter) {
    let pack = Rc::new(PackStore::new(&f.database));
    let routed = FrameRouter::new(f.chats.clone(), f.board.clone(), Some(pack.clone()));
    (pack, routed)
}

/// A `pack_item` frame is applied under the seq guard and announced — and that is ALL it does: it
/// never counts as unread, never raises a message, and touches no chat.
#[test]
fn a_pack_frame_changes_the_pack_and_nothing_else() -> StoreResult<()> {
    let f = fixture()?;
    let (pack, routed) = routed_with_pack(&f);
    let (changes, on_changed) = recorder::<PackItemDto>();
    routed.on_pack_changed().subscribe(on_changed);
    let arrived = Rc::new(Cell::new(0));
    let touched = Rc::new(Cell::new(0));
    let (a, t) = (arrived.clone(), touched.clone());
    routed.on_arrived().subscribe(move |_| a.set(a.get() + 1));
    routed
        .on_chat_changed()
        .subscribe(move |_| t.set(t.get() + 1));

    routed.hear(&ServerFrame::PackItem(pack_item(5, 12, 9)))?;
    routed.hear(&ServerFrame::PackItem(pack_item(6, 13, 9)))?;
    routed.hear(&ServerFrame::PackItem(PackItemDto {
        deleted: true,
        pack_seq: 14,
        ..PackItemDto::new(5)
    }))?;

    let ids: Vec<i64> = pack.items()?.iter().map(|item| item.id).collect();
    assert_eq!(ids, [6]);
    assert_eq!(changes.borrow().len(), 3);
    assert!(changes.borrow()[2].deleted);
    assert_eq!(arrived.get(), 0);
    assert_eq!(touched.get(), 0);
    assert_eq!(f.chats.unread()?, 0);
    Ok(())
}

/// NOT FILTERED BY BLOCKS: an item is a picture the family keeps, not something a person said, and
/// the frame reaches a blocker exactly as it reaches everyone.
#[test]
fn a_pack_frame_from_a_blocked_member_is_applied_like_any_other() -> StoreResult<()> {
    let f = fixture()?;
    let (pack, routed) = routed_with_pack(&f);
    f.chats.set_blocked(9, true)?;

    routed.hear(&ServerFrame::PackItem(pack_item(5, 12, 9)))?;

    assert_eq!(one(pack.items()?).added_by, 9);
    Ok(())
}

/// A frame moves the pack's cursor only once this connection has caught up.
#[test]
fn a_pack_frame_moves_the_cursor_only_once_caught_up() -> StoreResult<()> {
    let f = fixture()?;
    let (pack, routed) = routed_with_pack(&f);
    pack.replace(&[pack_item(5, 12, 9)], 12)?;
    pack.reconnected();

    routed.hear(&ServerFrame::PackItem(pack_item(9, 20, 9)))?;
    assert_eq!(pack.cursor()?, 12);

    pack.caught_up(pack.connection());
    routed.hear(&ServerFrame::PackItem(pack_item(10, 21, 9)))?;
    assert_eq!(pack.cursor()?, 21);
    Ok(())
}

/// A router built without a pack — nothing here has one to keep — drops the frame quietly.
#[test]
fn a_pack_frame_with_no_pack_to_keep_it_in_is_dropped() -> StoreResult<()> {
    let f = fixture()?;
    let heard = Rc::new(Cell::new(0));
    let counted = heard.clone();
    f.router
        .on_pack_changed()
        .subscribe(move |_| counted.set(counted.get() + 1));

    f.router.hear(&ServerFrame::PackItem(pack_item(5, 12, 9)))?;

    assert_eq!(heard.get(), 0);
    Ok(())
}

fn group(id: i64, title: &str, members: &[i64], admin: i64) -> ChatDto {
    ChatDto {
        member_ids: Some(members.to_vec()),
        admin_user_id: Some(admin),
        ..ChatDto::new(id, "group", title)
    }
}

/// `chat_updated` PUTS A GROUP ON THE LIST, members and admin as stated — which is how a member
/// just added sees it at once — and a later one rewrites them whole without touching what is
/// counted (docs/protocol.md, "Groups").
#[test]
fn a_chat_updated_frame_upserts_the_group() -> StoreResult<()> {
    let f = fixture()?;
    let (touched, on_touched) = recorder::<i64>();
    f.router.on_chat_changed().subscribe(on_touched);

    f.router
        .hear(&ServerFrame::ChatUpdated(group(77, "Cousins", &[9, ME], 9)))?;
    f.router.hear(&ServerFrame::Message(message(
        1500,
        77,
        "Dinner at 7?",
        9,
        None,
    )))?;
    f.router.hear(&ServerFrame::ChatUpdated(group(
        77,
        "The cousins",
        &[9, ME, 11],
        ME,
    )))?;

    let held_group = listed(&f.chats, 77)?;
    assert_eq!(held_group.chat.title, "The cousins");
    assert_eq!(held_group.chat.member_ids, Some(vec![9, ME, 11]));
    assert_eq!(held_group.chat.admin_user_id, Some(ME));
    assert_eq!(f.chats.kind(77)?.as_deref(), Some("group"));
    assert!(f.chats.is_listed(77)?);
    // The rename counted nothing and cleared nothing: the message that arrived between them is
    // still unread.
    assert_eq!(held_group.unread_count, 1);
    assert!(f.chats.chats()?.iter().any(|row| row.chat.id == 77));
    assert_eq!(*touched.borrow(), [77, 77, 77]);
    Ok(())
}

/// `chat_removed` DROPS THE GROUP, its messages and its queued sends — and is said once, to
/// whoever has it open. A frame naming the family chat or a direct chat drops nothing: those never
/// leave this way.
#[test]
fn a_chat_removed_frame_drops_the_group_and_nothing_else() -> StoreResult<()> {
    let f = fixture()?;
    let outbox = OutboxStore::new(&f.database);
    f.router
        .hear(&ServerFrame::ChatUpdated(group(77, "Cousins", &[9, ME], 9)))?;
    f.router.hear(&ServerFrame::Message(message(
        1500,
        77,
        "Dinner at 7?",
        9,
        None,
    )))?;
    f.router.hear(&ServerFrame::Message(dinner(1501)))?;
    outbox.queue(&OutboxRow::new("k77", 77, "Still there?"))?;
    outbox.queue(&OutboxRow::new("k42", 42, "Dinner at 7?"))?;
    let (removed, on_removed) = recorder::<i64>();
    f.router.on_chat_removed().subscribe(on_removed);

    f.router.hear(&ServerFrame::ChatRemoved { chat_id: 77 })?;
    f.router.hear(&ServerFrame::ChatRemoved { chat_id: 42 })?;
    f.router.hear(&ServerFrame::ChatRemoved { chat_id: 43 })?;
    f.router.hear(&ServerFrame::ChatRemoved { chat_id: 77 })?;

    assert_eq!(f.chats.chat(77)?, None);
    assert_eq!(f.chats.message(1500)?, None);
    assert_eq!(f.chats.kind(77)?, None);
    assert!(!f.chats.chats()?.iter().any(|row| row.chat.id == 77));
    assert_eq!(one(outbox.all()?).client_msg_id, "k42");
    // The family chat and the direct chat are untouched.
    assert!(f.chats.message(1501)?.is_some());
    assert!(f.chats.chat(42)?.is_some());
    assert!(f.chats.chat(43)?.is_some());
    assert_eq!(*removed.borrow(), [77]);
    Ok(())
}

/// "SEEN" IS NOT DRAWN IN A GROUP, as in the family chat: somebody else's read marker there is
/// forwarded nowhere (docs/protocol.md, "Groups").
#[test]
fn a_peers_read_marker_in_a_group_is_not_forwarded() -> StoreResult<()> {
    let f = fixture()?;
    f.router
        .hear(&ServerFrame::ChatUpdated(group(77, "Cousins", &[9, ME], 9)))?;
    let (seen, on_seen) = recorder::<(i64, i64, i64)>();
    f.router.on_peer_read().subscribe(on_seen);

    f.router.hear(&ServerFrame::Read {
        chat_id: 77,
        user_id: 9,
        last_read_message_id: 1500,
    })?;
    f.router.hear(&ServerFrame::Read {
        chat_id: 43,
        user_id: 11,
        last_read_message_id: 1500,
    })?;

    let chats: Vec<i64> = seen.borrow().iter().map(|(chat, _, _)| *chat).collect();
    assert_eq!(chats, [43]);
    Ok(())
}
