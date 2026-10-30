//! Everything between "somebody pressed send" and "the server has it" — and the case that matters
//! most, which is the one in between (docs/protocol.md, "Sending on an unreliable network"). A
//! port of `win/tests/FamilyConnect.Core.Tests/Protocol/SendPipelineTests.cs`.
//!
//! The C# harness's `now: () => Now` and `wait: (_, _) => Task.CompletedTask` are one
//! [`StillClock`]: the time a row records is exactly `Now`, and the ack deadline is not slept — a
//! silent socket answers "unanswered" at once, which is the same decision ten seconds later and a
//! suite that finishes. Its `Fake` socket is [`FakeFrameSender`].

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use futures::FutureExt;
use serde_json::Value;

use fc_core::protocol::send_rules::FlushTrigger;
use fc_core::protocol::{
    ApiError, ApiResult, ChatDto, ChatRowDto, MessageDto, MessageResponse, QueueOptions,
    ReconnectBackoff, SendPipeline, ServerFrame, error_codes,
};
use fc_core::rt::{Cancel, Clock};
use fc_core::store::{ChatStore, Database, OutboxRow, OutboxStore, StoreResult};
use fc_core::testing::{FakeFrameSender, StillClock, drive};
use time::OffsetDateTime;
use time::macros::datetime;

const NOW: OffsetDateTime = datetime!(2026-09-12 10:00 UTC);
const KEY: &str = "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01";

fn id_of(frame: &str) -> String {
    let parsed: Value = serde_json::from_str(frame).expect("a JSON frame");
    parsed["client_msg_id"]
        .as_str()
        .expect("a client_msg_id")
        .to_string()
}

fn delivered(client_msg_id: &str, id: i64) -> MessageDto {
    MessageDto::new(
        id,
        42,
        7,
        Some(client_msg_id),
        "Dinner at 7?",
        "2026-09-12T10:00:00Z",
    )
}

fn acked(frame: &str) -> Option<ServerFrame> {
    let id = id_of(frame);
    Some(ServerFrame::Ack {
        value: delivered(&id, 1338),
        client_msg_id: id,
    })
}

fn refusal(code: &str, detail: &str, frame: &str) -> Option<ServerFrame> {
    Some(ServerFrame::Error {
        code: code.to_string(),
        detail: detail.to_string(),
        client_msg_id: Some(id_of(frame)),
        call_id: None,
    })
}

fn one<T>(mut items: Vec<T>) -> T {
    assert_eq!(items.len(), 1, "exactly one");
    items.remove(0)
}

type Rest = Rc<RefCell<Option<Box<dyn Fn(&OutboxRow) -> ApiResult<MessageResponse>>>>>;

struct Harness {
    pipeline: SendPipeline,
    socket: FakeFrameSender,
    outbox: Rc<OutboxStore>,
    chats: Rc<ChatStore>,
    clock: StillClock,
    refusals: Rc<RefCell<Vec<(OutboxRow, ApiError)>>>,
    posted: Rc<RefCell<Vec<OutboxRow>>>,
    rest: Rest,
}

impl Harness {
    /// What the REST fallback answers from here on; until said, there is no server.
    fn rest(&self, answer: impl Fn(&OutboxRow) -> ApiResult<MessageResponse> + 'static) {
        *self.rest.borrow_mut() = Some(Box::new(answer));
    }

    fn flush(&self, trigger: FlushTrigger) -> StoreResult<usize> {
        drive_still(self.pipeline.flush(trigger))
    }

    fn enqueue(&self, body: &str, options: &QueueOptions) -> StoreResult<OutboxRow> {
        self.pipeline.enqueue(42, body, options)
    }
}

/// A [`StillClock`]'s sleeps end at once, so a future on it never needs the clock moved; the fake
/// clock [`drive`] wants is only there to say so if one ever stalls.
fn drive_still<F: std::future::Future>(future: F) -> F::Output {
    drive(&fc_core::testing::FakeClock::at(NOW), future)
}

fn success(row: &OutboxRow, id: i64) -> ApiResult<MessageResponse> {
    ApiResult::success(
        MessageResponse {
            message: delivered(&row.client_msg_id, id),
        },
        201,
    )
}

fn build() -> StoreResult<Harness> {
    let database = Database::open_in_memory()?;
    let socket = FakeFrameSender::new();
    let outbox = Rc::new(OutboxStore::new(&database));
    let chats = Rc::new(ChatStore::new(&database));
    chats.replace(
        &[ChatRowDto::new(ChatDto::new(42, "family", "The Smiths"))],
        None,
    )?;
    let clock = StillClock::at(NOW);
    let refusals = Rc::new(RefCell::new(Vec::new()));
    let posted = Rc::new(RefCell::new(Vec::new()));
    let rest: Rest = Rc::new(RefCell::new(None));
    let (posts, answers) = (posted.clone(), rest.clone());
    let pipeline = SendPipeline::new(
        Rc::new(socket.clone()),
        outbox.clone(),
        chats.clone(),
        Rc::new(move |row: OutboxRow| {
            posts.borrow_mut().push(row.clone());
            let answer = match answers.borrow().as_ref() {
                Some(answer) => answer(&row),
                None => ApiResult::failure(ApiError::transport("no server here")),
            };
            Box::pin(async move { answer })
        }),
        Rc::new(clock.clone()) as Rc<dyn Clock>,
    )
    .with_backoff(ReconnectBackoff::with_random(|ceiling| ceiling))
    .with_next_id(|| KEY.to_string());
    let into = refusals.clone();
    pipeline
        .on_refused()
        .subscribe(move |refused: &(OutboxRow, ApiError)| into.borrow_mut().push(refused.clone()));
    Ok(Harness {
        pipeline,
        socket,
        outbox,
        chats,
        clock,
        refusals,
        posted,
        rest,
    })
}

fn frames(socket: &FakeFrameSender) -> Vec<Value> {
    socket
        .sent()
        .iter()
        .map(|sent| serde_json::from_str(sent).expect("a JSON frame"))
        .collect()
}

#[test]
fn a_send_is_written_down_before_anything_moves() -> StoreResult<()> {
    let harness = build()?;
    let row = harness.enqueue("Dinner at 7?", &QueueOptions::default())?;
    // The bubble is drawn from this, and the id is the dedup key the server will take.
    assert_eq!(row.client_msg_id, KEY);
    assert_eq!(row.queued_at, Some(NOW));
    assert_eq!(harness.outbox.all()?.len(), 1);
    // And nothing has been sent yet: the flush is a separate act, which is what lets it run first
    // in a resync and on every trigger.
    assert!(harness.socket.sent().is_empty());
    Ok(())
}

#[test]
fn an_acked_frame_is_delivered_and_the_row_goes() -> StoreResult<()> {
    let harness = build()?;
    harness.socket.answer_with(acked);
    harness.enqueue("Dinner at 7?", &QueueOptions::default())?;

    assert_eq!(harness.flush(FlushTrigger::Timer)?, 1);
    assert!(harness.outbox.all()?.is_empty());
    // The message went into the cache through the same path a page does.
    assert_eq!(
        harness.chats.message(1338)?.expect("delivered").body,
        "Dinner at 7?"
    );
    // Over the SOCKET: no REST call was made.
    assert_eq!(harness.socket.sent().len(), 1);
    assert!(harness.posted.borrow().is_empty());
    Ok(())
}

/// THE CASE THAT MATTERS. A frame with no answer inside the deadline says nothing about whether
/// the message landed — so the client re-sends over REST with the SAME dedup key, which is what
/// makes the repeat safe.
#[test]
fn an_unanswered_frame_falls_back_to_rest_with_the_same_dedup_key() -> StoreResult<()> {
    let harness = build()?;
    // Silence from the socket.
    harness.socket.answer_with(|_| None);
    harness.rest(|row| success(row, 1339));
    let row = harness.enqueue("Dinner at 7?", &QueueOptions::default())?;

    assert_eq!(harness.flush(FlushTrigger::Timer)?, 1);
    assert_eq!(harness.socket.sent().len(), 1);
    assert_eq!(
        one(harness.posted.borrow().clone()).client_msg_id,
        row.client_msg_id
    );
    assert!(harness.outbox.all()?.is_empty());
    assert!(harness.chats.message(1339)?.is_some());
    // Held, and not where the next catch-up starts: an answer over REST says nothing about the
    // messages before it, and a send answered while the socket was reconnecting would otherwise
    // skip everything it missed.
    assert_eq!(harness.chats.catch_up_cursor(42)?, None);
    // The deadline was the protocol's ten seconds.
    assert_eq!(
        harness.clock.slept(),
        [fc_core::protocol::send_rules::ACK_DEADLINE]
    );
    Ok(())
}

#[test]
fn a_write_that_did_not_go_is_the_same_as_no_answer() -> StoreResult<()> {
    let harness = build()?;
    harness.socket.set_writes_go_through(false);
    harness.rest(|row| success(row, 1338));
    harness.enqueue("Dinner at 7?", &QueueOptions::default())?;

    assert_eq!(harness.flush(FlushTrigger::Timer)?, 1);
    // A socket whose connection is dead absorbs a write, so this is as much of an answer as the
    // deadline expiring: REST decides it.
    assert_eq!(harness.posted.borrow().len(), 1);
    Ok(())
}

#[test]
fn with_no_socket_at_all_it_goes_straight_to_rest() -> StoreResult<()> {
    let harness = build()?;
    harness.socket.set_connected(false);
    harness.rest(|row| success(row, 1338));
    harness.enqueue("Dinner at 7?", &QueueOptions::default())?;

    assert_eq!(harness.flush(FlushTrigger::WindowActivated)?, 1);
    assert!(harness.socket.sent().is_empty());
    assert_eq!(harness.posted.borrow().len(), 1);
    // The trigger is recorded, not acted on.
    assert_eq!(
        harness.pipeline.last_trigger(),
        Some(FlushTrigger::WindowActivated)
    );
    Ok(())
}

/// An `error` frame carrying a TERMINAL code is a refusal: the message will never be accepted as
/// it stands, so it is shown failed and REST is not tried.
#[test]
fn a_terminal_refusal_over_the_socket_is_final() -> StoreResult<()> {
    let harness = build()?;
    harness
        .socket
        .answer_with(|frame| refusal(error_codes::MESSAGE_TOO_LONG, "too long", frame));
    harness.enqueue(&"a".repeat(5000), &QueueOptions::default())?;

    assert_eq!(harness.flush(FlushTrigger::Timer)?, 0);
    assert!(harness.posted.borrow().is_empty());
    let row = one(harness.outbox.all()?);
    assert!(row.failed());
    assert_eq!(
        row.failed_code.as_deref(),
        Some(error_codes::MESSAGE_TOO_LONG)
    );
    // The user is told exactly once, and only about this.
    let (refused, error) = one(harness.refusals.borrow().clone());
    assert_eq!(refused.client_msg_id, row.client_msg_id);
    assert_eq!(error.code, error_codes::MESSAGE_TOO_LONG);
    Ok(())
}

/// An `error` frame with a TRANSIENT code is treated exactly as no answer at all — so REST still
/// gets its turn.
#[test]
fn a_transient_error_frame_is_not_a_refusal() -> StoreResult<()> {
    let harness = build()?;
    harness
        .socket
        .answer_with(|frame| refusal(error_codes::INTERNAL, "busy", frame));
    harness.rest(|row| success(row, 1338));
    harness.enqueue("Dinner at 7?", &QueueOptions::default())?;

    assert_eq!(harness.flush(FlushTrigger::Timer)?, 1);
    assert_eq!(harness.posted.borrow().len(), 1);
    assert!(harness.refusals.borrow().is_empty());
    Ok(())
}

#[test]
fn an_unknown_outcome_stays_queued_and_is_not_shown_as_failed() -> StoreResult<()> {
    let harness = build()?;
    harness.socket.answer_with(|_| None);
    harness.rest(|_| ApiResult::failure(ApiError::new("", "gateway", 502)));
    harness.enqueue("Dinner at 7?", &QueueOptions::default())?;

    assert_eq!(harness.flush(FlushTrigger::Timer)?, 0);
    let row = one(harness.outbox.all()?);
    assert!(!row.failed());
    assert_eq!(row.attempts, 1);
    assert_eq!(row.next_attempt_at, Some(NOW + time::Duration::seconds(1)));
    // NOTHING is shown to the user: unknown is not a refusal.
    assert!(harness.refusals.borrow().is_empty());
    Ok(())
}

#[test]
fn a_row_that_is_not_due_is_left_alone() -> StoreResult<()> {
    let harness = build()?;
    harness.socket.answer_with(|_| None);
    harness.rest(|_| ApiResult::failure(ApiError::transport("offline")));
    harness.enqueue("Dinner at 7?", &QueueOptions::default())?;
    harness.flush(FlushTrigger::Timer)?;
    let sent_once = harness.socket.sent().len();

    // The clock has not moved, so the row's delay has not passed.
    assert_eq!(harness.flush(FlushTrigger::SocketConnected)?, 0);
    assert_eq!(harness.socket.sent().len(), sent_once);
    Ok(())
}

/// A row that still owes uploads must NEVER be posted: a message claiming no attachments is a text
/// message, and the server would take it happily — a delivered bubble with the pictures gone.
#[test]
fn a_row_that_owes_uploads_is_not_sent_at_all() -> StoreResult<()> {
    let harness = build()?;
    harness.socket.answer_with(acked);
    harness.enqueue(
        "",
        &QueueOptions {
            pending_files: vec!["/tmp/a.jpg".to_string()],
            ..QueueOptions::default()
        },
    )?;

    assert_eq!(harness.flush(FlushTrigger::Timer)?, 0);
    assert!(harness.socket.sent().is_empty());
    assert!(harness.posted.borrow().is_empty());
    // Still queued, and not failed: it is waiting for its bytes, not refused.
    assert!(!one(harness.outbox.all()?).failed());
    Ok(())
}

/// An ack that arrived after the deadline — or after a relaunch, with nobody waiting — is still a
/// delivery.
#[test]
fn a_late_ack_still_delivers_the_row() -> StoreResult<()> {
    let harness = build()?;
    let row = harness.enqueue("Dinner at 7?", &QueueOptions::default())?;
    harness.socket.raise(&ServerFrame::Ack {
        client_msg_id: row.client_msg_id.clone(),
        value: delivered(&row.client_msg_id, 1338),
    });
    assert!(harness.outbox.all()?.is_empty());
    assert!(harness.chats.message(1338)?.is_some());
    Ok(())
}

/// And this device's own send arriving as an ORDINARY message — the ack lost on the way back — is
/// a delivery too, not a second bubble.
#[test]
fn own_message_arriving_without_its_ack_is_still_a_delivery() -> StoreResult<()> {
    let harness = build()?;
    let row = harness.enqueue("Dinner at 7?", &QueueOptions::default())?;
    harness
        .socket
        .raise(&ServerFrame::Message(delivered(&row.client_msg_id, 1338)));
    assert!(harness.outbox.all()?.is_empty());
    assert!(harness.chats.message(1338)?.is_some());
    Ok(())
}

#[test]
fn the_same_send_never_runs_twice_at_once() -> StoreResult<()> {
    let harness = build()?;
    let gate = Cancel::new();
    let posts = Rc::new(Cell::new(0));
    harness.socket.set_connected(false);
    let (opened, counted) = (gate.clone(), posts.clone());
    let pipeline = SendPipeline::new(
        Rc::new(harness.socket.clone()),
        harness.outbox.clone(),
        harness.chats.clone(),
        Rc::new(move |row: OutboxRow| {
            counted.set(counted.get() + 1);
            let opened = opened.clone();
            Box::pin(async move {
                opened.cancelled().await;
                success(&row, 1338)
            })
        }),
        Rc::new(StillClock::at(NOW)),
    )
    .with_next_id(|| "one".to_string());
    pipeline.enqueue(42, "Dinner at 7?", &QueueOptions::default())?;

    let mut first = Box::pin(pipeline.flush(FlushTrigger::Timer));
    // The first flush is at the gate, posting.
    assert!((&mut first).now_or_never().is_none());
    // The second finds it still working and does nothing: two passes over one row would each
    // upload the remainder and post it twice.
    //
    // BOUNDED on purpose. Without the guard the second flush joins the first behind the gate,
    // and `drive` fails it as stuck — a test that fails rather than one that hangs for ever.
    assert_eq!(
        drive_still(pipeline.flush(FlushTrigger::ConnectivityRestored))?,
        0
    );
    gate.cancel();
    assert_eq!(drive_still(first)?, 1);
    assert_eq!(posts.get(), 1);
    Ok(())
}

// ---- a sticker is a message (docs/protocol.md, "Sending one") --------------------------------

/// The flag rides the frame — and nothing else about the send is new: the same row, the same dedup
/// key, the same ack.
#[test]
fn a_sticker_goes_over_the_socket_with_its_flag() -> StoreResult<()> {
    let harness = build()?;
    harness.socket.answer_with(acked);
    let row = harness.enqueue(
        "",
        &QueueOptions {
            attachment_ids: vec![90],
            sticker: true,
            ..QueueOptions::default()
        },
    )?;
    assert!(row.sticker);

    assert_eq!(harness.flush(FlushTrigger::Timer)?, 1);

    let frame = one(frames(&harness.socket));
    assert_eq!(frame["sticker"], Value::Bool(true));
    assert_eq!(frame["attachment_ids"][0].as_i64(), Some(90));
    assert_eq!(frame["body"].as_str(), Some(""));
    assert!(harness.outbox.all()?.is_empty());
    Ok(())
}

/// An unanswered frame falls back to REST with the SAME row — flag included — and a sticker that
/// still owes its upload is never posted at all: it would land as an empty message.
#[test]
fn a_sticker_falls_back_to_rest_still_a_sticker() -> StoreResult<()> {
    let harness = build()?;
    harness.rest(|row| success(row, 1338));
    harness.socket.set_connected(false);
    let owing = harness.enqueue(
        "",
        &QueueOptions {
            pending_files: vec!["0123456789abcdef0123456789abcdef".to_string()],
            sticker: true,
            ..QueueOptions::default()
        },
    )?;

    assert_eq!(harness.flush(FlushTrigger::Timer)?, 0);
    assert!(harness.posted.borrow().is_empty());

    harness
        .outbox
        .uploaded(&owing.client_msg_id, 90, "0123456789abcdef0123456789abcdef")?;
    assert_eq!(harness.flush(FlushTrigger::Timer)?, 1);

    let posted = one(harness.posted.borrow().clone());
    assert!(posted.sticker);
    assert_eq!(posted.attachment_ids, Some(vec![90]));
    assert!(harness.outbox.all()?.is_empty());
    Ok(())
}

/// A refusal about the picture is terminal and shown: `invalid_attachment` is what the server
/// answers for a sticker that is not one, and no retry would change it.
#[test]
fn a_refused_sticker_is_shown_failed_like_any_refused_message() -> StoreResult<()> {
    let harness = build()?;
    harness
        .socket
        .answer_with(|frame| refusal(error_codes::INVALID_ATTACHMENT, "not a sticker", frame));
    harness.enqueue(
        "",
        &QueueOptions {
            attachment_ids: vec![90],
            sticker: true,
            ..QueueOptions::default()
        },
    )?;

    assert_eq!(harness.flush(FlushTrigger::Timer)?, 0);

    let (row, error) = one(harness.refusals.borrow().clone());
    assert!(row.sticker);
    assert_eq!(error.code, error_codes::INVALID_ATTACHMENT);
    assert!(one(harness.outbox.all()?).failed());
    assert!(harness.posted.borrow().is_empty());
    Ok(())
}

// ---- a video message is a message (docs/protocol.md, "Video messages") -----------------------

/// The flag rides the frame, and an ordinary send carries no `round` at all.
#[test]
fn a_video_message_goes_over_the_socket_with_its_flag() -> StoreResult<()> {
    let harness = build()?;
    harness.socket.answer_with(acked);
    let row = harness.enqueue(
        "",
        &QueueOptions {
            attachment_ids: vec![91],
            round: true,
            ..QueueOptions::default()
        },
    )?;
    assert!(row.round);
    assert!(!row.sticker);
    assert_eq!(harness.flush(FlushTrigger::Timer)?, 1);
    harness.enqueue("Dinner at 7?", &QueueOptions::default())?;
    assert_eq!(harness.flush(FlushTrigger::Timer)?, 1);

    let frames = frames(&harness.socket);
    assert_eq!(frames[0]["round"], Value::Bool(true));
    assert!(frames[0].get("sticker").is_none());
    assert_eq!(frames[0]["attachment_ids"][0].as_i64(), Some(91));
    assert!(frames[1].get("round").is_none());
    assert!(harness.outbox.all()?.is_empty());
    Ok(())
}

/// Unanswered, it falls back to REST still round — and one that owes its upload is never posted:
/// it would land as an empty message.
#[test]
fn a_video_message_falls_back_to_rest_still_round() -> StoreResult<()> {
    let harness = build()?;
    harness.rest(|row| success(row, 1338));
    harness.socket.set_connected(false);
    let owing = harness.enqueue(
        "",
        &QueueOptions {
            reply_to_message_id: Some(41),
            pending_files: vec!["0123456789abcdef0123456789abcdef".to_string()],
            round: true,
            ..QueueOptions::default()
        },
    )?;

    assert_eq!(harness.flush(FlushTrigger::Timer)?, 0);
    assert!(harness.posted.borrow().is_empty());

    harness
        .outbox
        .uploaded(&owing.client_msg_id, 91, "0123456789abcdef0123456789abcdef")?;
    assert_eq!(harness.flush(FlushTrigger::Timer)?, 1);

    let posted = one(harness.posted.borrow().clone());
    assert!(posted.round);
    assert!(!posted.sticker);
    assert_eq!(posted.reply_to_message_id, Some(41));
    assert_eq!(posted.attachment_ids, Some(vec![91]));
    Ok(())
}

/// A refusal is terminal and shown, the flag kept, so Try Again sends a circle again.
#[test]
fn a_refused_video_message_is_shown_failed_like_any_refused_message() -> StoreResult<()> {
    let harness = build()?;
    harness.socket.answer_with(|frame| {
        refusal(
            error_codes::INVALID_ATTACHMENT,
            "not a video message",
            frame,
        )
    });
    harness.enqueue(
        "",
        &QueueOptions {
            attachment_ids: vec![91],
            round: true,
            ..QueueOptions::default()
        },
    )?;

    assert_eq!(harness.flush(FlushTrigger::Timer)?, 0);

    let (row, error) = one(harness.refusals.borrow().clone());
    assert!(row.round);
    assert_eq!(error.code, error_codes::INVALID_ATTACHMENT);
    assert!(one(harness.outbox.all()?).failed());
    Ok(())
}

// ---- not a port: the dedup key ---------------------------------------------------------------

/// A REAL UUID, as `Guid.NewGuid().ToString()` writes one — version 4, the RFC 4122 variant,
/// lower-case and hyphenated — and never the same twice: the server takes the dedup key as given,
/// and a key that collided would merge two messages in a family's chat. (No C# test: `win/` took
/// .NET's own.)
#[test]
fn a_dedup_key_is_a_real_uuid() {
    let keys: Vec<String> = (0..64)
        .map(|_| fc_core::protocol::send_pipeline::uuid_v4())
        .collect();
    for key in &keys {
        let groups: Vec<&str> = key.split('-').collect();
        assert_eq!(
            groups.iter().map(|group| group.len()).collect::<Vec<_>>(),
            [8, 4, 4, 4, 12],
            "{key}"
        );
        assert!(
            key.chars()
                .all(|c| c == '-' || c.is_ascii_digit() || ('a'..='f').contains(&c)),
            "{key}"
        );
        assert!(groups[2].starts_with('4'), "{key}");
        assert!(groups[3].starts_with(['8', '9', 'a', 'b']), "{key}");
    }
    let distinct: std::collections::HashSet<&String> = keys.iter().collect();
    assert_eq!(distinct.len(), keys.len());
}
