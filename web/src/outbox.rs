//! Getting a message to the server, on whatever network there is.
//!
//! A browser sends over REST — one row at a time, oldest first — and only
//! listens on the socket (docs/protocol.md, "A browser is a client too").
//! The rules are the protocol's "Sending on an unreliable network": a send
//! is delivered, refused or UNKNOWN, and unknown is by far the commonest on
//! a bad network. It is retried with backoff and never shown as a failure
//! until the attempts are spent; only a refusal fails a message at once.
//!
//! The first version of this client queued every message for the socket and
//! waited. When the socket did not come up, nothing was ever sent — and
//! nothing said so.

use std::future::Future;

use futures::channel::mpsc::UnboundedReceiver;
use futures::{FutureExt, StreamExt};

use crate::api::ApiError;
use crate::live::Live;
use crate::model::Message;
use crate::store::Outgoing;

/// The codes that mean the server READ the send and refused it: the
/// message will never be accepted as it stands. Every other answer — a
/// timeout, a 502 from a proxy, a 429, an `internal` — leaves the outcome
/// unknown. The protocol's list, and the phone clients'
/// (ios ChatSyncCoordinator.terminalSendCodes).
pub const TERMINAL_CODES: [&str; 10] = [
    "validation",
    "message_empty",
    "message_too_long",
    "not_chat_member",
    "chat_not_found",
    "blocked",
    "invalid_poll",
    "invalid_attachment",
    "attachment_not_found",
    "attachment_already_used",
];

pub fn is_terminal(error: &ApiError) -> bool {
    error
        .code()
        .is_some_and(|code| TERMINAL_CODES.contains(&code))
}

/// What a refused bubble says. The protocol's `message` is English for
/// developers; the codes a person can actually run into get a sentence.
pub fn refusal(error: &ApiError) -> String {
    match error.code() {
        Some("not_chat_member") | Some("chat_not_found") => {
            "Not sent: you are no longer in this chat.".to_string()
        }
        Some("blocked") => "Not sent: you have blocked this person.".to_string(),
        Some("message_too_long") => "Not sent: the message is too long.".to_string(),
        _ => format!("Not sent: {}", error.detail()),
    }
}

/// The first backoff ceiling, and the most it grows to — the phone
/// clients' reconnect shape, which the protocol tells the outbox to reuse.
pub const BACKOFF_BASE_MS: f64 = 1_000.0;
pub const BACKOFF_CAP_MS: f64 = 30_000.0;

/// The longest the `failures`-th wait in a row may be, counting from 1:
/// 1 s, 2 s, 4 s … and never more than 30 s.
pub fn backoff_ceiling_ms(failures: u32) -> f64 {
    let doublings = failures.saturating_sub(1).min(16);
    (BACKOFF_BASE_MS * f64::from(1u32 << doublings)).min(BACKOFF_CAP_MS)
}

/// FULL jitter: anywhere from nothing up to the ceiling. A family's devices
/// lose the same server at the same moment, and without it they would all
/// come back at the same moment too. `random` is in [0, 1).
pub fn jittered_ms(ceiling: f64, random: f64) -> u32 {
    (ceiling * random.clamp(0.0, 1.0)) as u32
}

/// Why the sender should look at the outbox again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wake {
    /// Something new was queued.
    Queued,
    /// The network may be back — the socket connected, the browser came
    /// online, the tab came back, a person pressed Retry. Cuts a backoff
    /// short, which `Queued` does not.
    Network,
}

/// The sender, for as long as `session` is the session.
///
/// `send` makes one attempt; `nap` waits out the backoff after the
/// `attempts`-th unknown outcome. Both are parameters so the rules can be
/// tested without a server or a clock. Returns when the session ends, when
/// the wake channel closes (which is how a sign-out stops it at once), or
/// on a 401 — after calling `expired`.
pub async fn drain<S, SF, N, NF>(
    live: Live,
    session: u64,
    mut wake: UnboundedReceiver<Wake>,
    send: S,
    nap: N,
    expired: impl Fn(),
) where
    S: Fn(Outgoing) -> SF,
    SF: Future<Output = Result<Message, ApiError>>,
    N: Fn(u32) -> NF,
    NF: Future<Output = ()>,
{
    while live.is_live(session) {
        let Some(row) = live.read(|state| state.store.next_to_send()) else {
            // Nothing to send: idle until something is.
            if wake.next().await.is_none() {
                return;
            }
            continue;
        };
        match send(row.clone()).await {
            Ok(message) => {
                live.update(session, |state| {
                    let open = state.open_chat;
                    state.store.settle(&row.client_msg_id, message, open);
                });
            }
            Err(ApiError::Unauthorized) => {
                expired();
                return;
            }
            Err(error) if is_terminal(&error) => {
                live.update(session, |state| {
                    state.store.refuse(&row.client_msg_id, refusal(&error))
                });
            }
            Err(_) => {
                let still_queued = live
                    .update(session, |state| {
                        state.store.note_unknown(&row.client_msg_id)
                    })
                    .flatten();
                let Some(attempts) = still_queued else {
                    continue;
                };
                // Wait before trying again. A network that comes back cuts
                // the wait short; a new message does NOT, or typing ahead
                // on a dead connection would spend this row's attempts in
                // seconds. Biased so a nap that is already over wins.
                let mut rest = Box::pin(nap(attempts).fuse());
                loop {
                    futures::select_biased! {
                        () = rest => break,
                        woken = wake.next() => match woken {
                            None => return,
                            Some(Wake::Network) => break,
                            Some(Wake::Queued) => {}
                        },
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;
    use std::rc::Rc;

    use futures::channel::mpsc;
    use wasm_bindgen_test::*;

    use crate::live::AppState;
    use crate::model::{Chat, ChatListItem};
    use crate::store::{Store, MAX_SEND_ATTEMPTS};

    /// What the fake server says to one attempt.
    enum Answer {
        Deliver(i64),
        Fail(ApiError),
        /// Delivers, but only after the person has signed out.
        DeliverAfterSignOut(i64),
    }

    fn network() -> ApiError {
        ApiError::Network("Failed to fetch".into())
    }

    fn server(code: &str) -> ApiError {
        ApiError::Server {
            code: code.into(),
            message: "…".into(),
        }
    }

    fn signed_in() -> Live {
        Live::new(
            AppState {
                token: Some("t".into()),
                open_chat: Some(42),
                store: Store {
                    my_user_id: 7,
                    chats: vec![ChatListItem {
                        chat: Chat {
                            id: 42,
                            kind: "family".into(),
                            title: Some("The Smiths".into()),
                            peer_user_id: None,
                        },
                        last_message: None,
                        unread_count: 0,
                        last_read_message_id: 0,
                    }],
                    ..Store::default()
                },
                ..AppState::default()
            },
            Rc::new(|| {}),
        )
    }

    struct Run {
        /// The `client_msg_id` of every attempt, in order.
        sent: Vec<String>,
        /// The `attempts` every nap was asked for.
        naps: Vec<u32>,
        expired: u32,
    }

    /// Run the sender over whatever is queued, against a scripted server,
    /// until it goes idle. The wake channel is closed up front, so idle is
    /// the end; naps are instant.
    async fn run(live: &Live, script: Vec<Answer>) -> Run {
        let script = Rc::new(RefCell::new(VecDeque::from(script)));
        let sent = Rc::new(RefCell::new(Vec::new()));
        let naps = Rc::new(RefCell::new(Vec::new()));
        let expired = Rc::new(Cell::new(0));
        let (wake, wake_in) = mpsc::unbounded::<Wake>();
        drop(wake);

        let send = {
            let script = script.clone();
            let sent = sent.clone();
            let live = live.clone();
            move |row: Outgoing| {
                sent.borrow_mut().push(row.client_msg_id.clone());
                let answer = script
                    .borrow_mut()
                    .pop_front()
                    .expect("the sender tried more often than the script allows");
                let delivered = |id: i64| Message {
                    id,
                    chat_id: row.chat_id,
                    sender_id: 7,
                    client_msg_id: Some(row.client_msg_id.clone()),
                    body: row.body.clone(),
                    created_at: "2026-09-10T10:00:00Z".into(),
                    edited_at: None,
                };
                let result = match answer {
                    Answer::Deliver(id) => Ok(delivered(id)),
                    Answer::Fail(error) => Err(error),
                    Answer::DeliverAfterSignOut(id) => {
                        live.end_session();
                        Ok(delivered(id))
                    }
                };
                async move { result }
            }
        };
        let nap = {
            let naps = naps.clone();
            move |attempts: u32| {
                naps.borrow_mut().push(attempts);
                futures::future::ready(())
            }
        };
        let on_expired = {
            let expired = expired.clone();
            move || expired.set(expired.get() + 1)
        };

        drain(live.clone(), live.session(), wake_in, send, nap, on_expired).await;

        assert!(script.borrow().is_empty(), "every scripted answer was used");
        let sent = sent.borrow().clone();
        let naps = naps.borrow().clone();
        Run {
            sent,
            naps,
            expired: expired.get(),
        }
    }

    fn queue(live: &Live, client_msg_id: &str) {
        live.now(|state| {
            state
                .store
                .queue_send(42, client_msg_id.into(), format!("body of {client_msg_id}"))
        });
    }

    fn thread_ids(live: &Live) -> Vec<i64> {
        live.read(|state| {
            state.store.threads[&42]
                .messages
                .iter()
                .map(|message| message.id)
                .collect()
        })
    }

    /// A new message must NOT cut a backoff short — typing ahead on a dead
    /// connection would spend the stuck row's attempts in seconds — but a
    /// network that comes back must, and then everything goes, in order.
    #[wasm_bindgen_test]
    async fn only_a_returning_network_cuts_a_backoff_short() {
        let live = signed_in();
        queue(&live, "a");
        let (wake, wake_in) = mpsc::unbounded::<Wake>();
        let sent = Rc::new(RefCell::new(Vec::<String>::new()));
        let send = {
            let sent = sent.clone();
            move |row: Outgoing| {
                sent.borrow_mut().push(row.client_msg_id.clone());
                let tries = sent.borrow().len() as i64;
                let result = if tries == 1 {
                    Err(network())
                } else {
                    Ok(Message {
                        id: 100 + tries,
                        chat_id: row.chat_id,
                        sender_id: 7,
                        client_msg_id: Some(row.client_msg_id),
                        body: row.body,
                        created_at: "2026-09-10T10:00:00Z".into(),
                        edited_at: None,
                    })
                };
                async move { result }
            }
        };
        // A backoff that never ends on its own: only a wake can end it.
        let nap = |_: u32| futures::future::pending::<()>();
        let done = Rc::new(Cell::new(false));
        {
            let live = live.clone();
            let done = done.clone();
            wasm_bindgen_futures::spawn_local(async move {
                drain(live.clone(), live.session(), wake_in, send, nap, || {}).await;
                done.set(true);
            });
        }
        let settle = || gloo_timers::future::TimeoutFuture::new(20);

        settle().await;
        assert_eq!(*sent.borrow(), vec!["a"], "tried once, and now waiting");

        queue(&live, "b");
        wake.unbounded_send(Wake::Queued)
            .expect("the sender is listening");
        settle().await;
        assert_eq!(*sent.borrow(), vec!["a"], "a new message waits its turn");

        wake.unbounded_send(Wake::Network)
            .expect("the sender is listening");
        settle().await;
        assert_eq!(
            *sent.borrow(),
            vec!["a", "a", "b"],
            "the network coming back ends the wait"
        );
        assert!(live.read(|state| state.store.outbox.is_empty()));

        // And closing the channel — a sign-out — stops an idle sender.
        drop(wake);
        settle().await;
        assert!(done.get());
    }

    /// Unknown, unknown, delivered: the SAME `client_msg_id` each time —
    /// which is what makes a retry a retry and not a duplicate — with a
    /// backoff between, and the bubble becoming the real message at the end.
    #[wasm_bindgen_test]
    async fn an_unknown_outcome_is_tried_again_until_it_is_delivered() {
        let live = signed_in();
        queue(&live, "a");

        let run = run(
            &live,
            vec![
                Answer::Fail(network()),
                Answer::Fail(server("internal")),
                Answer::Deliver(101),
            ],
        )
        .await;

        assert_eq!(run.sent, vec!["a", "a", "a"]);
        assert_eq!(run.naps, vec![1, 2], "a growing backoff between tries");
        assert!(live.read(|state| state.store.outbox.is_empty()));
        assert_eq!(thread_ids(&live), vec![101], "one bubble, now the real one");
    }

    /// Unknown is NOT failed — until the attempts are spent, and then it is
    /// failed VISIBLY, with nothing further tried behind the person's back.
    #[wasm_bindgen_test]
    async fn the_attempts_are_bounded_and_running_out_is_shown() {
        let live = signed_in();
        queue(&live, "a");

        let script = (0..MAX_SEND_ATTEMPTS)
            .map(|_| Answer::Fail(network()))
            .collect();
        let run = run(&live, script).await;

        assert_eq!(run.sent.len() as u32, MAX_SEND_ATTEMPTS);
        assert_eq!(run.naps, (1..MAX_SEND_ATTEMPTS).collect::<Vec<_>>());
        let failed = live.read(|state| state.store.failed_sends(42));
        assert!(failed.contains_key("a"), "shown as not sent: {failed:?}");
        assert_eq!(
            thread_ids(&live),
            vec![0],
            "the bubble stays, to be retried"
        );
    }

    /// A refusal fails the message AT ONCE — no backoff, no second try —
    /// and does not hold back the next one.
    #[wasm_bindgen_test]
    async fn a_refusal_fails_at_once_and_the_next_message_still_goes() {
        let live = signed_in();
        queue(&live, "a");
        queue(&live, "b");

        let run = run(
            &live,
            vec![
                Answer::Fail(server("not_chat_member")),
                Answer::Deliver(102),
            ],
        )
        .await;

        assert_eq!(run.sent, vec!["a", "b"]);
        assert!(run.naps.is_empty(), "a refusal is not retried");
        let failed = live.read(|state| state.store.failed_sends(42));
        assert_eq!(
            failed.get("a").map(String::as_str),
            Some("Not sent: you are no longer in this chat.")
        );
        assert_eq!(thread_ids(&live), vec![102, 0]);
    }

    /// Oldest first, and the one that is having trouble is finished — or
    /// given up on — before the next is started.
    #[wasm_bindgen_test]
    async fn messages_go_in_the_order_they_were_written() {
        let live = signed_in();
        queue(&live, "a");
        queue(&live, "b");
        queue(&live, "c");

        let run = run(
            &live,
            vec![
                Answer::Deliver(101),
                Answer::Fail(network()),
                Answer::Deliver(102),
                Answer::Deliver(103),
            ],
        )
        .await;

        assert_eq!(run.sent, vec!["a", "b", "b", "c"]);
        assert_eq!(thread_ids(&live), vec![101, 102, 103]);
    }

    /// A 401 is the session gone: the sender says so once and stops, and
    /// does not burn the queue on a token that will never work again.
    #[wasm_bindgen_test]
    async fn an_expired_session_stops_the_sender() {
        let live = signed_in();
        queue(&live, "a");
        queue(&live, "b");

        let run = run(&live, vec![Answer::Fail(ApiError::Unauthorized)]).await;

        assert_eq!(run.sent, vec!["a"]);
        assert_eq!(run.expired, 1);
    }

    /// An answer that lands after a sign-out is dropped, and the sender for
    /// that session stops rather than going on to the next row.
    #[wasm_bindgen_test]
    async fn a_sign_out_mid_send_drops_the_answer_and_stops() {
        let live = signed_in();
        queue(&live, "a");
        queue(&live, "b");

        let run = run(&live, vec![Answer::DeliverAfterSignOut(101)]).await;

        assert_eq!(run.sent, vec!["a"], "nothing sent for a session that ended");
        assert!(
            live.read(|state| state.store.threads.is_empty()),
            "the old session's answer was not written into the new one"
        );
    }

    #[wasm_bindgen_test]
    fn the_terminal_codes_are_the_protocols_and_nothing_else_is() {
        for code in TERMINAL_CODES {
            assert!(is_terminal(&server(code)), "{code} is a refusal");
        }
        for code in [
            "internal",
            "rate_limited",
            "attachment_expired",
            "not_found",
        ] {
            assert!(!is_terminal(&server(code)), "{code} leaves it unknown");
        }
        assert!(!is_terminal(&network()));
        assert!(!is_terminal(&ApiError::Unauthorized));
    }

    #[wasm_bindgen_test]
    fn the_backoff_doubles_from_a_second_up_to_half_a_minute() {
        let ceilings: Vec<f64> = (1..=8).map(backoff_ceiling_ms).collect();
        assert_eq!(
            ceilings,
            vec![1_000.0, 2_000.0, 4_000.0, 8_000.0, 16_000.0, 30_000.0, 30_000.0, 30_000.0]
        );
        // Days of flapping must not overflow the shift.
        assert_eq!(backoff_ceiling_ms(u32::MAX), 30_000.0);
        assert_eq!(backoff_ceiling_ms(0), 1_000.0);
    }

    #[wasm_bindgen_test]
    fn full_jitter_is_anywhere_from_nothing_to_the_ceiling() {
        assert_eq!(jittered_ms(8_000.0, 0.0), 0);
        assert_eq!(jittered_ms(8_000.0, 0.5), 4_000);
        assert!(jittered_ms(8_000.0, 0.999_999) <= 8_000);
        // A random source out of range cannot make the wait longer.
        assert_eq!(jittered_ms(8_000.0, 7.0), 8_000);
    }
}
