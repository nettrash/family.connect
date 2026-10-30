//! `ConversationModelTests`: one open chat — the window, paging back, the read marker, typing and
//! sending.

mod common;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use fc_core::protocol::{ApiResult, MessageDto, MessageResponse, QueueOptions, SendPipeline};
use fc_core::rt::LocalBoxFuture;
use fc_core::store::{OutboxRow, SeqRoute, StoreResult};
use fc_core::testing::{FakeFrameSender, StillClock};
use fc_logic::conversation::ConversationModel;
use fc_logic::quotes::QuoteLevel;

use common::chat::{CHAT, ChatRig, ME, START, is_uuid, set_up, socket};
use common::server::Server;

/// `Wire_(id, sender, body)`: a message as the server writes one.
fn wire(id: i64) -> String {
    format!(
        r#"
        {{"id": {id}, "chat_id": 42, "sender_id": 9, "client_msg_id": null,
         "body": "Dinner at 7?", "created_at": "2026-09-12T10:00:00Z"}}
        "#
    )
}

fn message(id: i64, sender: i64, body: &str) -> MessageDto {
    MessageDto::new(id, CHAT, sender, None, body, "2026-09-12T10:00:00Z")
}

fn plain(id: i64) -> MessageDto {
    message(id, 9, "Dinner at 7?")
}

fn page_of(count: i64, from: i64) -> String {
    let rows: Vec<String> = (0..count).map(|at| wire(from + at)).collect();
    format!(r#"{{"messages": [{}]}}"#, rows.join(","))
}

struct Built {
    chat: ConversationModel,
    server: Server,
    socket: FakeFrameSender,
}

/// `Build(server)`: a conversation over a socket that is up, posting through a fake that answers
/// every row as delivered, its typing throttle on the rig's clock.
fn build(rig: &ChatRig, server: &Server) -> StoreResult<Built> {
    let socket = socket(true);
    let log: Rc<RefCell<Vec<String>>> = Rc::default();
    let sending = SendPipeline::new(
        Rc::new(socket.clone()),
        rig.outbox.clone(),
        rig.chats.clone(),
        Rc::new(
            move |row: OutboxRow| -> LocalBoxFuture<'static, ApiResult<MessageResponse>> {
                log.borrow_mut().push(row.body.clone());
                let id = 2000 + i64::try_from(log.borrow().len()).unwrap();
                let message = MessageDto::new(
                    id,
                    CHAT,
                    ME,
                    Some(&row.client_msg_id),
                    &row.body,
                    "2026-09-12T12:00:00Z",
                );
                Box::pin(async move { ApiResult::success(MessageResponse { message }, 201) })
            },
        ),
        Rc::new(StillClock::at(START)),
    );
    let chat = ConversationModel::new(
        CHAT,
        rig.chats.clone(),
        rig.api(server),
        Rc::new(sending),
        Rc::new(socket.clone()),
        Rc::new(rig.clock.clone()),
    )?;
    Ok(Built {
        chat,
        server: server.clone(),
        socket,
    })
}

fn ids(chat: &ConversationModel) -> StoreResult<Vec<i64>> {
    Ok(chat.bubbles()?.iter().map(|b| b.message.id).collect())
}

#[test]
fn opening_a_chat_with_nothing_held_reads_its_newest_page() -> StoreResult<()> {
    let rig = ChatRig::new()?;
    let server = Server::new();
    server.on("/chats/42/messages", &page_of(50, 1));
    let built = build(&rig, &server)?;

    assert_eq!(None, rig.drive(built.chat.open())?);

    let ids = ids(&built.chat)?;
    assert_eq!(50, ids.len());
    // Oldest first: a conversation reads the other way round from the index.
    assert_eq!(1, ids[0]);
    assert_eq!(50, ids[49]);
    assert!(
        built
            .server
            .asked()
            .contains(&"/api/v1/chats/42/messages?limit=50".to_string())
    );
    // A full page says there may be more behind it.
    assert!(built.chat.may_have_older());
    Ok(())
}

/// Nothing is fetched when the cache already holds this chat: the resync's own catch-up is what
/// brings it up to date, and a read here would race it for no gain.
#[test]
fn opening_a_chat_that_is_already_held_asks_for_nothing() -> StoreResult<()> {
    let rig = ChatRig::new()?;
    rig.chats
        .apply_page(&[plain(1), plain(2)], SeqRoute::CatchUpPage, true)?;
    let built = build(&rig, &Server::new())?;

    assert_eq!(None, rig.drive(built.chat.open())?);

    assert!(built.server.asked().is_empty());
    assert_eq!(2, built.chat.bubbles()?.len());
    Ok(())
}

/// THE LIST'S PREVIEW IS NOT HOLDING THE CHAT: it is one trimmed row with nothing under it. The
/// catch-up does not page a chat that holds nothing in sequence, so opening it is what reads its
/// newest page — or the window is one bubble, and the next catch-up has nowhere to start from.
#[test]
fn opening_a_chat_that_holds_only_its_preview_reads_its_newest_page() -> StoreResult<()> {
    let rig = ChatRig::new()?;
    rig.chats.replace(
        &[fc_core::protocol::ChatRowDto {
            last_message: Some(plain(50)),
            ..fc_core::protocol::ChatRowDto::new(fc_core::protocol::ChatDto::new(
                CHAT,
                "family",
                "The Smiths",
            ))
        }],
        None,
    )?;
    let server = Server::new();
    server.on("/chats/42/messages", &page_of(50, 1));
    let built = build(&rig, &server)?;

    assert_eq!(None, rig.drive(built.chat.open())?);

    assert!(
        built
            .server
            .asked()
            .contains(&"/api/v1/chats/42/messages?limit=50".to_string())
    );
    assert_eq!(50, built.chat.bubbles()?.len());
    assert_eq!(Some(50), rig.chats.catch_up_cursor(CHAT)?);
    Ok(())
}

/// A short page IS the beginning of the chat. Saying otherwise leaves a button that asks the server
/// for nothing for ever.
#[test]
fn a_short_page_is_the_beginning_of_the_chat() -> StoreResult<()> {
    let rig = ChatRig::new()?;
    let server = Server::new();
    server.on("/chats/42/messages", &page_of(3, 1));
    let built = build(&rig, &server)?;

    rig.drive(built.chat.open())?;

    assert_eq!(3, built.chat.bubbles()?.len());
    assert!(!built.chat.may_have_older());
    Ok(())
}

/// Paging back widens the WINDOW first: what the cache already holds below the oldest drawn
/// message is paged in without a request at all, which is what makes reopening a chat cheap.
#[test]
fn paging_back_reads_the_cache_before_it_reads_the_server() -> StoreResult<()> {
    let rig = ChatRig::new()?;
    let held: Vec<MessageDto> = (1..=60).map(plain).collect();
    rig.chats.apply_page(&held, SeqRoute::CatchUpPage, true)?;
    let built = build(&rig, &Server::new())?;
    rig.drive(built.chat.open())?;
    assert_eq!(50, built.chat.bubbles()?.len());

    assert_eq!(None, rig.drive(built.chat.older())?);

    assert_eq!(60, built.chat.bubbles()?.len());
    assert!(built.server.asked().is_empty());
    Ok(())
}

/// A page of history is not news: a reply on it is already in its root's recomputed count
/// (docs/protocol.md, "Threads").
#[test]
fn a_page_of_history_raises_no_roots_count() -> StoreResult<()> {
    let rig = ChatRig::new()?;
    rig.chats.apply(
        &MessageDto {
            reply_count: Some(1),
            ..plain(300)
        },
        SeqRoute::Evidence,
        true,
    )?;
    let server = Server::new();
    server.on(
        "/chats/42/messages",
        r#"{"messages": [{"id": 299, "chat_id": 42, "sender_id": 9, "client_msg_id": null, "body": "yes", "created_at": "2026-09-12T10:00:00Z", "thread_root_id": 300}]}"#,
    );
    let built = build(&rig, &server)?;
    rig.drive(built.chat.open())?;

    assert_eq!(None, rig.drive(built.chat.older())?);

    assert!(rig.chats.message(299)?.is_some());
    assert_eq!(Some(1), rig.chats.message(300)?.unwrap().reply_count);
    Ok(())
}

#[test]
fn paging_past_what_is_held_asks_for_older_than_the_oldest() -> StoreResult<()> {
    let rig = ChatRig::new()?;
    let held: Vec<MessageDto> = (100..150).map(plain).collect();
    rig.chats.apply_page(&held, SeqRoute::CatchUpPage, true)?;
    let server = Server::new();
    server.on("/chats/42/messages", &page_of(2, 98));
    let built = build(&rig, &server)?;
    rig.drive(built.chat.open())?;

    assert_eq!(None, rig.drive(built.chat.older())?);

    assert!(
        built
            .server
            .asked()
            .contains(&"/api/v1/chats/42/messages?before_id=100&limit=50".to_string())
    );
    assert_eq!(52, built.chat.bubbles()?.len());
    // Two of fifty: there is nothing older.
    assert!(!built.chat.may_have_older());
    Ok(())
}

/// A CLICK ON A QUOTE FINDS THE MESSAGE IT NAMES, from the cache and without a request: the window
/// widens over rows this device already holds until the message is drawn (docs/protocol.md,
/// "Replies" — the id is how a client that wants the whole message finds it). A message this
/// install never held is not found, and nothing is asked for it.
#[test]
fn a_jump_widens_the_window_over_what_is_held_and_asks_for_nothing() -> StoreResult<()> {
    let rig = ChatRig::new()?;
    let held: Vec<MessageDto> = (1..=130).map(plain).collect();
    rig.chats.apply_page(&held, SeqRoute::CatchUpPage, true)?;
    let built = build(&rig, &Server::new())?;

    // A page is drawn: the newest 50, so 81 is not among them.
    assert!(built.chat.draws(130)?);
    assert!(!built.chat.draws(80)?);

    assert!(built.chat.draw_to(80)?);
    assert!(built.chat.draws(80)?);
    // Nothing older than the cache, and nothing from another chat.
    assert!(!built.chat.draw_to(500)?);
    assert!(built.server.asked().is_empty());
    Ok(())
}

/// A quote can name a message retention has swept, or one older than anything this install held:
/// the window widens over what there is, stops, and says it cannot draw it — rather than looping
/// over the same short read.
#[test]
fn a_jump_to_a_message_the_cache_does_not_hold_stops() -> StoreResult<()> {
    let rig = ChatRig::new()?;
    rig.chats
        .apply_page(&[plain(1), plain(2)], SeqRoute::CatchUpPage, true)?;
    let built = build(&rig, &Server::new())?;

    assert!(!built.chat.draw_to(99)?);
    assert!(built.chat.draw_to(1)?);
    assert!(built.server.asked().is_empty());
    Ok(())
}

/// A hidden quote reveals one level at a time, and the reveal belongs to the conversation, so it
/// outlives the redraw (docs/protocol.md, "Blocking a member").
#[test]
fn a_quotes_reveal_is_per_level_and_outlives_the_redraw() -> StoreResult<()> {
    let rig = ChatRig::new()?;
    let built = build(&rig, &Server::new())?;

    assert!(!built.chat.quote_revealed(11, QuoteLevel::Reply));
    built.chat.reveal_quote(11, QuoteLevel::Reply);
    assert!(built.chat.quote_revealed(11, QuoteLevel::Reply));
    // One level says nothing about the other, nor about another message's quote.
    assert!(!built.chat.quote_revealed(11, QuoteLevel::Parent));
    assert!(!built.chat.quote_revealed(12, QuoteLevel::Reply));
    Ok(())
}

/// THE READ MARKER GOES FORWARD ONLY, AND ONLY WHEN THERE IS SOMETHING TO REPORT: the server keeps
/// the maximum ever reported, so an older id buys a request and nothing else, and a report per
/// redraw costs the family's server a request per redraw.
#[test]
fn reading_reports_the_newest_once_and_not_again() -> StoreResult<()> {
    let rig = ChatRig::new()?;
    rig.chats.apply(&plain(1338), SeqRoute::LiveFrame, true)?;
    let server = Server::new();
    server.on_status("/chats/42/read", 204, None);
    let built = build(&rig, &server)?;
    assert_eq!(1, rig.chats.chat(CHAT)?.unwrap().unread_count);

    assert!(rig.drive(built.chat.read(None))?);

    // The badge goes out at once, whatever the network is doing…
    let row = rig.chats.chat(CHAT)?.unwrap();
    assert_eq!(0, row.unread_count);
    assert_eq!(1338, row.last_read_message_id);
    assert_eq!(1, server.asked().len());

    // …and looking again reports nothing, because nothing has been said since.
    assert!(!rig.drive(built.chat.read(None))?);
    assert_eq!(1, server.asked().len());

    // A new message, and there is something to report again.
    rig.chats.apply(&plain(1339), SeqRoute::LiveFrame, true)?;
    assert!(rig.drive(built.chat.read(None))?);
    assert_eq!(2, server.asked().len());
    Ok(())
}

/// A WINDOW THAT IS NOT AT THE BOTTOM SAYS SO. A reader who opened a chat at an old position and
/// never came down has not read the rest of it, and a client that reported the newest anyway would
/// clear a badge nobody looked at.
#[test]
fn a_window_scrolled_up_the_history_reports_only_what_it_has_shown() -> StoreResult<()> {
    let rig = ChatRig::new()?;
    rig.chats.apply_page(
        &[plain(1338), plain(1339), plain(1340)],
        SeqRoute::LiveFrame,
        true,
    )?;
    let server = Server::new();
    server.on_status("/chats/42/read", 204, None);
    let built = build(&rig, &server)?;

    assert!(rig.drive(built.chat.read(Some(1339)))?);

    let row = rig.chats.chat(CHAT)?.unwrap();
    assert_eq!(1339, row.last_read_message_id);
    // One message is still unread, and the badge still says so.
    assert_eq!(1, row.unread_count);

    // Coming down to the bottom reports the rest.
    assert!(rig.drive(built.chat.read(None))?);
    let row = rig.chats.chat(CHAT)?.unwrap();
    assert_eq!(1340, row.last_read_message_id);
    assert_eq!(0, row.unread_count);
    Ok(())
}

#[test]
fn a_window_that_names_an_id_this_device_has_never_seen_is_clamped() -> StoreResult<()> {
    let rig = ChatRig::new()?;
    rig.chats.apply(&plain(1338), SeqRoute::LiveFrame, true)?;
    let server = Server::new();
    server.on_status("/chats/42/read", 204, None);
    let built = build(&rig, &server)?;

    assert!(rig.drive(built.chat.read(Some(999_999)))?);

    // What is HELD, not what was named: a marker past the newest message would report a message
    // this device has never seen as read.
    assert_eq!(1338, rig.chats.chat(CHAT)?.unwrap().last_read_message_id);
    Ok(())
}

#[test]
fn a_read_that_the_network_refuses_still_takes_the_badge_out() -> StoreResult<()> {
    let rig = ChatRig::new()?;
    rig.chats.apply(&plain(1338), SeqRoute::LiveFrame, true)?;
    let server = Server::new();
    server.on_status("/chats/42/read", 502, Some("<html>502</html>"));
    let built = build(&rig, &server)?;

    assert!(rig.drive(built.chat.read(None))?);

    // Locally read; the server keeps the maximum ever reported, so the next report fixes it.
    let row = rig.chats.chat(CHAT)?.unwrap();
    assert_eq!(0, row.unread_count);
    assert_eq!(1338, row.last_read_message_id);
    Ok(())
}

/// TYPING IS MOMENTARY: one frame every four seconds at most, and never while the socket is down —
/// by the time a queued one went it would be a lie.
#[test]
fn typing_is_throttled_and_never_queued() -> StoreResult<()> {
    let rig = ChatRig::new()?;
    let built = build(&rig, &Server::new())?;

    assert!(rig.drive(built.chat.typing()));
    assert_eq!(1, built.socket.sent().len());
    assert!(built.socket.sent()[0].contains("\"type\":\"typing\""));

    // Every keystroke after it, for four seconds, says nothing.
    rig.clock.advance(Duration::from_secs(3));
    assert!(!rig.drive(built.chat.typing()));
    assert_eq!(1, built.socket.sent().len());

    rig.clock.advance(Duration::from_secs(1));
    assert!(rig.drive(built.chat.typing()));
    assert_eq!(2, built.socket.sent().len());

    // And with the socket down, nothing at all: not queued, not tried.
    set_up(&built.socket, false);
    rig.clock.advance(Duration::from_secs(56));
    assert!(!rig.drive(built.chat.typing()));
    assert_eq!(2, built.socket.sent().len());
    Ok(())
}

#[test]
fn sending_writes_the_row_down_first_and_the_outbox_owns_it() -> StoreResult<()> {
    let rig = ChatRig::new()?;
    let built = build(&rig, &Server::new())?;
    set_up(&built.socket, false);

    let row = built.chat.send("Dinner at 7?", &QueueOptions::default())?;

    // Written down before anything was tried: an interrupted send is a bubble that can be finished
    // rather than nothing at all.
    let all = rig.outbox.all()?;
    assert_eq!(1, all.len());
    assert_eq!("Dinner at 7?", all[0].body);
    assert_eq!(CHAT, row.chat_id);
    // And the dedup key is a real UUID, which is what makes a repeat safe.
    assert!(is_uuid(&row.client_msg_id), "{}", row.client_msg_id);

    built.chat.send("…or 8?", &QueueOptions::default())?;
    assert_eq!(2, rig.outbox.all()?.len());
    Ok(())
}

/// A REVEAL BELONGS TO THE CONVERSATION: held here, it outlives the bubble being drawn again, which
/// is the difference between tapping "show" once and tapping it after every arriving message.
#[test]
fn a_hidden_bubble_can_be_revealed_and_stays_revealed() -> StoreResult<()> {
    let rig = ChatRig::new()?;
    rig.chats.apply_page(
        &[message(1338, 11, "something unpleasant"), plain(1339)],
        SeqRoute::CatchUpPage,
        true,
    )?;
    rig.chats.set_blocked(11, true)?;
    let built = build(&rig, &Server::new())?;

    let blocked = built.chat.bubbles()?.remove(0);
    assert!(blocked.hidden);
    assert!(!blocked.reads());
    assert!(!built.chat.bubbles()?[1].hidden);

    built.chat.reveal(1338);
    assert!(built.chat.bubbles()?[0].revealed);
    assert!(built.chat.bubbles()?[0].reads());

    // Another message arrives and the bubbles are drawn again: the reveal is still there.
    rig.chats.apply(&plain(1340), SeqRoute::LiveFrame, true)?;
    assert!(built.chat.bubbles()?[0].revealed);

    built.chat.hide(1338);
    assert!(!built.chat.bubbles()?[0].revealed);
    Ok(())
}

#[test]
fn my_own_bubbles_are_mine_and_never_hidden() -> StoreResult<()> {
    let rig = ChatRig::new()?;
    rig.chats.apply_page(
        &[message(1338, ME, "mine"), message(1339, 11, "Dinner at 7?")],
        SeqRoute::CatchUpPage,
        true,
    )?;
    rig.chats.set_blocked(11, true)?;
    let built = build(&rig, &Server::new())?;

    let bubbles = built.chat.bubbles()?;

    assert!(bubbles[0].mine);
    assert!(!bubbles[0].hidden);
    assert!(!bubbles[1].mine);
    assert!(bubbles[1].hidden);
    Ok(())
}
