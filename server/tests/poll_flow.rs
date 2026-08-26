//! Integration: polls (docs/protocol.md, "Polls").
//!
//! The specification these assert against is the protocol document, not the
//! implementation: a poll is an ORDINARY MESSAGE that happens to be votable,
//! its question IS the message body, its options are fixed at creation, the
//! vote is an idempotent state-set with one choice per member, closing is
//! the author's and one-way, and every change takes the next value of a
//! fourth server-wide sequence so the `after_seq` feed can replay changes to
//! rows `after_id` could never see again.
//!
//! The live-socket helpers at the top are copied from `ws_flow.rs` rather
//! than shared: `common/` is the REST harness, and a second copy of forty
//! lines is cheaper than a test-only abstraction two files deep.

mod common;

use std::time::Duration;

use common::{PushCall, TestServer, assert_error, spawn_server, spawn_server_with_config};
use family_connect::push_payload::PushEvent;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::http::header::AUTHORIZATION;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use uuid::Uuid;

type WsClient = WebSocketStream<MaybeTlsStream<TcpStream>>;

const FRAME_WAIT: Duration = Duration::from_secs(5);

/// Open an authenticated WebSocket and wait for a pong, which proves the
/// server-side connection task is registered — later fan-outs cannot race
/// past a connection that has already answered a frame.
async fn connect_ws(ts: &TestServer, token: &str) -> WsClient {
    let mut request = ts
        .ws_url
        .as_str()
        .into_client_request()
        .expect("building the ws request");
    request.headers_mut().insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}")).expect("header value"),
    );
    let (mut ws, _response) = tokio_tungstenite::connect_async(request)
        .await
        .expect("websocket upgrade succeeds");
    send_frame(&mut ws, json!({"type": "ping"})).await;
    let pong = next_frame_of_type(&mut ws, "pong").await;
    assert_eq!(pong, json!({"type": "pong"}));
    ws
}

async fn send_frame(ws: &mut WsClient, frame: Value) {
    ws.send(Message::text(frame.to_string()))
        .await
        .expect("sending a frame");
}

/// Read frames until one of the wanted type arrives, skipping everything
/// else (protocol pings, unrelated relays). Panics after `FRAME_WAIT`.
async fn next_frame_of_type(ws: &mut WsClient, wanted: &str) -> Value {
    let deadline = tokio::time::Instant::now() + FRAME_WAIT;
    loop {
        let message = tokio::time::timeout_at(deadline, ws.next())
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for a {wanted:?} frame"))
            .expect("socket closed while waiting for a frame")
            .expect("socket errored while waiting for a frame");
        if let Message::Text(text) = message {
            let value: Value = serde_json::from_str(text.as_str()).expect("frames are JSON");
            if value["type"] == wanted {
                return value;
            }
        }
    }
}

/// Assert that no frame of the given type arrives within `window`.
async fn assert_no_frame_of_type(ws: &mut WsClient, unwanted: &str, window: Duration) {
    let deadline = tokio::time::Instant::now() + window;
    loop {
        match tokio::time::timeout_at(deadline, ws.next()).await {
            Err(_elapsed) => return, // window passed quietly
            Ok(Some(Ok(Message::Text(text)))) => {
                let value: Value = serde_json::from_str(text.as_str()).expect("frames are JSON");
                assert_ne!(
                    value["type"], unwanted,
                    "unexpected {unwanted:?} frame arrived: {value}"
                );
            }
            Ok(Some(Ok(_))) => {} // protocol ping/pong noise
            Ok(Some(Err(err))) => panic!("socket errored: {err}"),
            Ok(None) => panic!("socket closed during the quiet window"),
        }
    }
}

/// Family of two; returns `(owner, owner_id, member, member_id, chat_id)`.
async fn family_of_two(ts: &TestServer) -> (String, i64, String, i64, i64) {
    let (owner, owner_id) = ts.register("owner", "Olive").await;
    let (member, member_id) = ts.register("junior", "Junior").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&member, &invite_code, "joined").await;
    let chat_id = ts.family_chat_id(&owner).await;
    (owner, owner_id, member, member_id, chat_id)
}

/// A server with the assistant configured, so the assistant's chat exists
/// and can be offered a poll. The endpoint is never called.
async fn server_with_assistant() -> TestServer {
    spawn_server_with_config(|cfg| {
        cfg.ai.enabled = true;
        cfg.ai.endpoint = "https://example.invalid".to_string();
        cfg.ai.deployment = "test-deployment".to_string();
        cfg.ai.api_key = "test-key".to_string();
        cfg.ai.title = "Assistant".to_string();
    })
    .await
}

async fn post_poll(
    ts: &TestServer,
    token: &str,
    chat_id: i64,
    body: &str,
    options: Value,
) -> reqwest::Response {
    ts.post(
        token,
        &format!("/chats/{chat_id}/messages"),
        json!({
            "client_msg_id": Uuid::new_v4().to_string(),
            "body": body,
            "poll": {"options": options},
        }),
    )
    .await
}

/// A pizza-or-pasta poll posted by `token`; returns
/// `(message_id, creation_seq, option_ids)`.
async fn a_poll(ts: &TestServer, token: &str, chat_id: i64) -> (i64, i64, Vec<i64>) {
    let response = post_poll(
        ts,
        token,
        chat_id,
        "Pizza or pasta?",
        json!(["Pizza", "Pasta"]),
    )
    .await;
    assert_eq!(response.status(), 201);
    let body: Value = response.json().await.expect("message response is JSON");
    let message_id = body["message"]["id"].as_i64().expect("message id");
    let (seq, _, options) = read_poll(&body["message"]["poll"]);
    (
        message_id,
        seq,
        options.into_iter().map(|(id, _, _)| id).collect(),
    )
}

/// One option as the assertions read it: `(id, text, votes)`.
type OptionState = (i64, String, Vec<i64>);

/// A whole poll as the assertions read it: `(poll_seq, closed, options)`.
type ReadPoll = (i64, bool, Vec<OptionState>);

/// Pull a `poll` object out of any of the four shapes that carry one — a
/// `Message`, a vote/close response, an entry in the catch-up feed, or the
/// `poll` frame.
fn read_poll(poll: &Value) -> ReadPoll {
    assert!(
        poll.get("question").is_none(),
        "the question is the message body and is deliberately not a Poll field: {poll}"
    );
    let options = poll["options"]
        .as_array()
        .expect("options is an array")
        .iter()
        .map(|option| {
            (
                option["id"].as_i64().expect("option id"),
                option["text"].as_str().expect("option text").to_string(),
                option["votes"]
                    .as_array()
                    .expect("votes is always serialized — empty means nobody")
                    .iter()
                    .map(|v| v.as_i64().expect("user id"))
                    .collect(),
            )
        })
        .collect();
    (
        poll["poll_seq"]
            .as_i64()
            .expect("poll_seq is ALWAYS present, unlike reaction_seq"),
        poll["closed"].as_bool().expect("closed is ALWAYS present"),
        options,
    )
}

/// The votes on one option, sorted: the protocol fixes the CONTENTS of the
/// list, not the order two members' votes land in.
fn voters(options: &[OptionState], index: usize) -> Vec<i64> {
    let mut votes = options[index].2.clone();
    votes.sort_unstable();
    votes
}

async fn vote(
    ts: &TestServer,
    token: &str,
    chat_id: i64,
    message_id: i64,
    option_id: i64,
) -> reqwest::Response {
    ts.put(
        token,
        &format!("/chats/{chat_id}/messages/{message_id}/vote"),
        json!({"option_id": option_id}),
    )
    .await
}

/// The `200 {message_id, poll}` all three mutating endpoints answer with.
async fn state_of(response: reqwest::Response) -> ReadPoll {
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.expect("poll response is JSON");
    assert!(body["message_id"].is_i64(), "message_id present: {body}");
    read_poll(&body["poll"])
}

/// The chat-list entry for one chat.
async fn chat_entry(ts: &TestServer, token: &str, chat_id: i64) -> Value {
    let chats: Value = ts
        .get(token, "/chats")
        .await
        .json()
        .await
        .expect("chat list is JSON");
    chats["chats"]
        .as_array()
        .expect("chats is an array")
        .iter()
        .find(|entry| entry["chat"]["id"].as_i64() == Some(chat_id))
        .unwrap_or_else(|| panic!("chat {chat_id} is listed"))
        .clone()
}

/// One message off a history page.
async fn message_on_page(ts: &TestServer, token: &str, chat_id: i64, message_id: i64) -> Value {
    let page: Value = ts
        .get(token, &format!("/chats/{chat_id}/messages"))
        .await
        .json()
        .await
        .expect("messages page is JSON");
    page["messages"]
        .as_array()
        .expect("messages is an array")
        .iter()
        .find(|m| m["id"].as_i64() == Some(message_id))
        .unwrap_or_else(|| panic!("message {message_id} is on the page"))
        .clone()
}

/// The whole catch-up feed from a cursor, as `(message_id, poll)` pairs.
async fn poll_feed(
    ts: &TestServer,
    token: &str,
    chat_id: i64,
    query: &str,
) -> Vec<(i64, ReadPoll)> {
    let response = ts
        .get(token, &format!("/chats/{chat_id}/polls{query}"))
        .await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.expect("poll feed is JSON");
    body["polls"]
        .as_array()
        .expect("polls is an array")
        .iter()
        .map(|entry| {
            (
                entry["message_id"].as_i64().expect("message_id"),
                read_poll(&entry["poll"]),
            )
        })
        .collect()
}

fn jpeg_bytes(len: usize) -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE0];
    bytes.resize(len.max(4), 0x00);
    bytes
}

/// Age a message by rewriting its timestamp — the sweep reads `created_at`,
/// so this is the whole of what "old" means to it (retention_flow.rs).
async fn age(ts: &TestServer, message_id: i64, days: i64) {
    sqlx::query("UPDATE messages SET created_at = now() - make_interval(days => $2) WHERE id = $1")
        .bind(message_id)
        .bind(days as i32)
        .execute(&ts.state.pool)
        .await
        .expect("aging the message");
}

async fn count(ts: &TestServer, sql: &str, message_id: i64) -> i64 {
    sqlx::query_scalar(sql)
        .bind(message_id)
        .fetch_one(&ts.state.pool)
        .await
        .expect("counting rows")
}

/// Wait for the push seam's first call — dispatch is spawned
/// fire-and-forget, so it is not recorded the instant the POST returns.
async fn wait_for_push(ts: &TestServer) -> PushCall {
    let deadline = tokio::time::Instant::now() + FRAME_WAIT;
    loop {
        if let Some(call) = ts.push.calls().into_iter().next() {
            return call;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the push seam was never called"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// A poll is an ordinary message that happens to be votable: its QUESTION is
/// the body, so history, the chat-list preview and the push alert all read
/// it with no new case between them, and a client that has never heard of
/// polls draws it as a plain message and loses only the buttons.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_poll_is_an_ordinary_message_whose_question_is_its_body() {
    let ts = spawn_server().await;
    let (owner, owner_id, member, _, chat_id) = family_of_two(&ts).await;

    // The member is offline with a push token: a poll pushes exactly once,
    // as a message, with its question for a body.
    let response = ts
        .post(
            &member,
            "/devices",
            json!({"platform": "ios", "push_token": "tok-member"}),
        )
        .await;
    assert_eq!(response.status(), 201);

    let response = post_poll(
        &ts,
        &owner,
        chat_id,
        "  Pizza or pasta?  ",
        json!(["Pizza", "  Pasta  "]),
    )
    .await;
    assert_eq!(response.status(), 201);
    let created: Value = response.json().await.expect("message response is JSON");
    let message = &created["message"];
    let message_id = message["id"].as_i64().expect("message id");

    // Everything an ordinary message is.
    assert_eq!(
        message["body"], "Pizza or pasta?",
        "the question is the message body, trimmed like any other"
    );
    assert_eq!(message["sender_id"].as_i64(), Some(owner_id));
    assert!(message["created_at"].is_string());
    assert!(
        message.get("attachment").is_none(),
        "a poll carries no attachment: {message}"
    );

    // Plus the options, which are all the Poll object holds.
    let (seq, closed, options) = read_poll(&message["poll"]);
    assert!(seq > 0, "a poll has a seq from the moment it exists");
    assert!(!closed, "a new poll is open");
    assert_eq!(
        options
            .iter()
            .map(|(_, text, _)| text.as_str())
            .collect::<Vec<_>>(),
        vec!["Pizza", "Pasta"],
        "options are trimmed and keep creation order"
    );
    for index in 0..options.len() {
        assert_eq!(
            voters(&options, index),
            Vec::<i64>::new(),
            "nobody has voted yet, and the empty list is still on the wire"
        );
    }

    // The other member reads the poll back on a history page, whole.
    let fetched = message_on_page(&ts, &member, chat_id, message_id).await;
    assert_eq!(fetched["body"], "Pizza or pasta?");
    assert_eq!(read_poll(&fetched["poll"]), (seq, false, options));

    // It counts as unread and previews as its question — the chat list
    // needs no case for polls at all, and deliberately carries no options.
    let entry = chat_entry(&ts, &member, chat_id).await;
    assert_eq!(entry["unread_count"].as_i64(), Some(1));
    assert_eq!(entry["last_message"]["body"], "Pizza or pasta?");
    assert!(
        entry["last_message"].get("poll").is_none(),
        "a preview reads the question and stops there: {entry}"
    );

    // A reply quotes the question too, for the same reason: there is no
    // second thing to excerpt.
    let response = ts
        .post(
            &member,
            &format!("/chats/{chat_id}/messages"),
            json!({
                "client_msg_id": Uuid::new_v4().to_string(),
                "body": "Pizza",
                "reply_to_message_id": message_id,
            }),
        )
        .await;
    assert_eq!(response.status(), 201);
    let reply: Value = response.json().await.expect("message response is JSON");
    assert_eq!(
        reply["message"]["reply_to"]["excerpt"], "Pizza or pasta?",
        "the quote reads the question, with no case for polls at all"
    );
    assert!(
        reply["message"].get("poll").is_none(),
        "a reply to a poll is not itself a poll: {reply}"
    );

    // And it pushes once, as one message — there is no "Olive started a
    // poll" alert, because the question is more useful than the fact.
    let call = wait_for_push(&ts).await;
    assert_eq!(call.note.body, "Pizza or pasta?");
    assert_eq!(call.note.title, "The Smiths — Olive");
    assert_eq!(
        call.note.event,
        PushEvent::Message {
            chat_id,
            message_id
        }
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        ts.push.calls().len(),
        1,
        "a poll pushes exactly once, as a message"
    );
}

/// `max_poll_seq` is the client's "is my poll cursor stale?" test, so it is
/// present exactly when the chat holds a poll and absent otherwise — never
/// a zero a client would have to know to ignore.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_chat_list_reports_max_poll_seq_only_for_a_chat_that_holds_a_poll() {
    let ts = spawn_server().await;
    let (owner, _, member, member_id, chat_id) = family_of_two(&ts).await;

    // A direct chat, which will never hold a poll, listed alongside.
    let direct: Value = ts
        .post(&owner, "/chats/direct", json!({"user_id": member_id}))
        .await
        .json()
        .await
        .expect("direct chat is JSON");
    let direct_id = direct["chat"]["id"].as_i64().expect("direct chat id");
    let response = ts
        .post_message(&owner, direct_id, &Uuid::new_v4().to_string(), "psst")
        .await;
    assert_eq!(response.status(), 201);

    for (label, id) in [("family", chat_id), ("direct", direct_id)] {
        let entry = chat_entry(&ts, &owner, id).await;
        assert!(
            entry.get("max_poll_seq").is_none(),
            "the {label} chat holds no poll yet: {entry}"
        );
    }

    let (message_id, creation_seq, options) = a_poll(&ts, &owner, chat_id).await;
    let entry = chat_entry(&ts, &owner, chat_id).await;
    assert_eq!(
        entry["max_poll_seq"].as_i64(),
        Some(creation_seq),
        "creating a poll is itself a change the feed must replay"
    );
    let entry = chat_entry(&ts, &owner, direct_id).await;
    assert!(
        entry.get("max_poll_seq").is_none(),
        "a poll in the family chat says nothing about the direct one: {entry}"
    );

    // Every later change moves the chat's cursor, which is what tells a
    // reconnecting client to read the feed at all.
    let (vote_seq, _, _) =
        state_of(vote(&ts, &member, chat_id, message_id, options[0]).await).await;
    assert!(vote_seq > creation_seq);
    let entry = chat_entry(&ts, &member, chat_id).await;
    assert_eq!(entry["max_poll_seq"].as_i64(), Some(vote_seq));
}

/// One choice, and you may change it: `PUT` names the option the caller now
/// holds and `DELETE` retracts it, both idempotent state-sets rather than
/// toggles. Votes are attributed, so every member draws the same tally.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn voting_is_an_idempotent_state_set_with_one_choice_per_member() {
    let ts = spawn_server().await;
    let (owner, owner_id, member, member_id, chat_id) = family_of_two(&ts).await;
    let (message_id, creation_seq, options) = a_poll(&ts, &owner, chat_id).await;
    let (pizza, pasta) = (options[0], options[1]);
    let vote_path = format!("/chats/{chat_id}/messages/{message_id}/vote");

    // A vote takes a new seq and names the voter.
    let (seq1, _, state) = state_of(vote(&ts, &member, chat_id, message_id, pizza).await).await;
    assert!(seq1 > creation_seq, "a vote takes the next seq");
    assert_eq!(voters(&state, 0), vec![member_id]);
    assert_eq!(voters(&state, 1), Vec::<i64>::new());

    // Every member sees the same tally, whoever asks: the author reads it
    // off an ordinary history page, attributed.
    let fetched = message_on_page(&ts, &owner, chat_id, message_id).await;
    let (page_seq, _, page_state) = read_poll(&fetched["poll"]);
    assert_eq!(page_seq, seq1);
    assert_eq!(voters(&page_state, 0), vec![member_id]);

    // Re-PUT of the option already held is a no-op: no seq burned.
    let (seq_noop, _, state) = state_of(vote(&ts, &member, chat_id, message_id, pizza).await).await;
    assert_eq!(seq_noop, seq1, "a no-op re-PUT must not burn a seq");
    assert_eq!(voters(&state, 0), vec![member_id]);

    // Changing the choice MOVES the vote — there is no multiple choice.
    let (seq2, _, state) = state_of(vote(&ts, &member, chat_id, message_id, pasta).await).await;
    assert!(seq2 > seq1);
    assert_eq!(
        voters(&state, 0),
        Vec::<i64>::new(),
        "the old choice is released"
    );
    assert_eq!(voters(&state, 1), vec![member_id]);

    // A second voter is attributed alongside the first.
    let (seq3, _, state) = state_of(vote(&ts, &owner, chat_id, message_id, pasta).await).await;
    assert!(seq3 > seq2);
    let mut both = vec![member_id, owner_id];
    both.sort_unstable();
    assert_eq!(voters(&state, 1), both);

    // Retraction, and retracting nothing.
    let (seq4, _, state) = state_of(ts.delete(&member, &vote_path).await).await;
    assert!(seq4 > seq3);
    assert_eq!(voters(&state, 1), vec![owner_id]);
    let (seq_noop, _, state) = state_of(ts.delete(&member, &vote_path).await).await;
    assert_eq!(
        seq_noop, seq4,
        "retracting nothing returns the current state and burns no seq"
    );
    assert_eq!(voters(&state, 1), vec![owner_id]);

    // An option that belongs to a DIFFERENT poll is not an option here —
    // without the poll_id half of the check, a member could vote on a poll
    // in a chat they cannot see and the tally would name them there.
    let (_, _, other_options) = a_poll(&ts, &owner, chat_id).await;
    assert_error(
        vote(&ts, &member, chat_id, message_id, other_options[0]).await,
        400,
        "invalid_poll",
    )
    .await;
    // As is an option id that names nothing at all.
    assert_error(
        vote(&ts, &member, chat_id, message_id, 99_999_999).await,
        400,
        "invalid_poll",
    )
    .await;

    // And so is one belonging to another FAMILY's poll, which is the case
    // the check is really there for: without the poll_id half, a member
    // could vote on a poll in a chat they cannot see and be named in its
    // tally there.
    let (outsider, _) = ts.register("outsider", "Out Sider").await;
    ts.create_family(&outsider, "The Joneses").await;
    let their_chat = ts.family_chat_id(&outsider).await;
    let (their_poll, _, their_options) = a_poll(&ts, &outsider, their_chat).await;
    assert_error(
        vote(&ts, &member, chat_id, message_id, their_options[0]).await,
        400,
        "invalid_poll",
    )
    .await;
    // Nor is their poll visible from this chat's feed, or votable through
    // it — a poll is only ever reached through the chat that holds it.
    let ours: Vec<i64> = poll_feed(&ts, &member, chat_id, "")
        .await
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert!(
        !ours.contains(&their_poll),
        "another family's poll surfaced in this chat's feed: {ours:?}"
    );
    assert_error(
        vote(&ts, &member, chat_id, their_poll, their_options[0]).await,
        404,
        "message_not_found",
    )
    .await;

    // Their tally is untouched by any of it.
    let theirs = poll_feed(&ts, &outsider, their_chat, "").await;
    assert_eq!(theirs.len(), 1);
    let (_, (_, _, their_state)) = &theirs[0];
    assert_eq!(voters(their_state, 0), Vec::<i64>::new());
}

/// Closing is the author's, and one-way. The family owner does not outrank
/// an author here, for the same reason they cannot edit anybody else's
/// message anywhere else in this protocol.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn only_the_author_can_close_a_poll_and_a_closed_poll_refuses_votes() {
    let ts = spawn_server().await;
    let (owner, _, member, member_id, chat_id) = family_of_two(&ts).await;

    // Authored by the ordinary member, so the family OWNER is the one who
    // may not close it: no rank outranks authorship.
    let (message_id, _, options) = a_poll(&ts, &member, chat_id).await;
    let close_path = format!("/chats/{chat_id}/messages/{message_id}/poll/close");
    let vote_path = format!("/chats/{chat_id}/messages/{message_id}/vote");

    let (vote_seq, _, _) =
        state_of(vote(&ts, &member, chat_id, message_id, options[0]).await).await;

    assert_error(
        ts.post(&owner, &close_path, json!({})).await,
        403,
        "not_message_author",
    )
    .await;
    // …and the refusal wrote nothing: still open, still votable.
    let (seq, closed, _) = state_of(vote(&ts, &owner, chat_id, message_id, options[1]).await).await;
    assert!(!closed);
    assert!(seq > vote_seq);

    let (closed_seq, closed, state) =
        state_of(ts.post(&member, &close_path, json!({})).await).await;
    assert!(closed, "the author closed it");
    assert!(closed_seq > seq, "closing is a change and takes a seq");
    assert_eq!(
        voters(&state, 0),
        vec![member_id],
        "a closed poll keeps its result"
    );

    // Closing a closed poll is a no-op.
    let (noop_seq, closed, again) = state_of(ts.post(&member, &close_path, json!({})).await).await;
    assert!(closed);
    assert_eq!(noop_seq, closed_seq, "a second close burns no seq");
    assert_eq!(again, state);

    // And a closed poll refuses further votes on both verbs.
    assert_error(
        vote(&ts, &owner, chat_id, message_id, options[0]).await,
        409,
        "poll_closed",
    )
    .await;
    assert_error(ts.delete(&member, &vote_path).await, 409, "poll_closed").await;

    // The result is still readable, and still closed, to everyone.
    let fetched = message_on_page(&ts, &owner, chat_id, message_id).await;
    let (seq, closed, state) = read_poll(&fetched["poll"]);
    assert_eq!(seq, closed_seq);
    assert!(closed);
    assert_eq!(voters(&state, 0), vec![member_id]);
}

/// A poll is a family deciding something together. Between two people it is
/// a question whose answer is the next message, and the assistant is not a
/// family — anywhere but the family chat is `invalid_poll`.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_poll_is_refused_anywhere_but_the_family_chat() {
    let ts = server_with_assistant().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (member, member_id) = ts.register("junior", "Junior").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&member, &invite_code, "joined").await;

    let direct: Value = ts
        .post(&owner, "/chats/direct", json!({"user_id": member_id}))
        .await
        .json()
        .await
        .expect("direct chat is JSON");
    let direct_id = direct["chat"]["id"].as_i64().expect("direct chat id");

    // `GET /chats` is what creates the assistant's chat, on demand.
    let chats: Value = ts
        .get(&owner, "/chats")
        .await
        .json()
        .await
        .expect("chat list is JSON");
    let entries = chats["chats"].as_array().expect("chats is an array");
    let ai_id = entries
        .iter()
        .find(|entry| entry["chat"]["kind"] == "ai")
        .expect("a configured server gives each member an assistant chat")["chat"]["id"]
        .as_i64()
        .expect("chat id");
    let family_id = entries
        .iter()
        .find(|entry| entry["chat"]["kind"] == "family")
        .expect("the family chat is always listed")["chat"]["id"]
        .as_i64()
        .expect("chat id");

    for (label, id) in [("direct", direct_id), ("the assistant's", ai_id)] {
        assert_error(
            post_poll(
                &ts,
                &owner,
                id,
                "Pizza or pasta?",
                json!(["Pizza", "Pasta"]),
            )
            .await,
            400,
            "invalid_poll",
        )
        .await;
        // The refusal wrote nothing: the chat is still empty.
        let page: Value = ts
            .get(&owner, &format!("/chats/{id}/messages"))
            .await
            .json()
            .await
            .expect("messages page is JSON");
        assert!(
            page["messages"].as_array().expect("array").is_empty(),
            "a refused poll left a message behind in the {label} chat"
        );
    }

    // The same request in the family chat is accepted, so it is the chat
    // that was refused and nothing else about the body.
    let response = post_poll(
        &ts,
        &owner,
        family_id,
        "Pizza or pasta?",
        json!(["Pizza", "Pasta"]),
    )
    .await;
    assert_eq!(response.status(), 201);
}

/// The price of the question being the body: a poll may not be empty the
/// way a photo may, and it cannot carry an attachment at all.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_poll_needs_a_question_and_cannot_carry_an_attachment() {
    let ts = spawn_server().await;
    let (owner, _, _, _, chat_id) = family_of_two(&ts).await;

    // A message carrying an attachment may have an empty body; a poll gets
    // no such relaxation, and the error is about the BODY, not the poll.
    assert_error(
        post_poll(&ts, &owner, chat_id, "   ", json!(["Pizza", "Pasta"])).await,
        400,
        "message_empty",
    )
    .await;

    // An attachment and a poll are mutually exclusive — checked before the
    // attachment is even looked up, so an id that names nothing still gets
    // `invalid_poll` rather than `attachment_not_found`.
    assert_error(
        ts.post(
            &owner,
            &format!("/chats/{chat_id}/messages"),
            json!({
                "client_msg_id": Uuid::new_v4().to_string(),
                "body": "Pizza or pasta?",
                "attachment_id": 987_654,
                "poll": {"options": ["Pizza", "Pasta"]},
            }),
        )
        .await,
        400,
        "invalid_poll",
    )
    .await;

    // And with a real, claimable upload, which is the case a client would
    // actually hit.
    let uploaded: Value = ts
        .put_bytes_method(
            "POST",
            &owner,
            "/attachments?kind=photo",
            "image/jpeg",
            jpeg_bytes(512),
        )
        .await
        .json()
        .await
        .expect("attachment response is JSON");
    let attachment_id = uploaded["attachment"]["id"]
        .as_i64()
        .expect("attachment id");
    assert_error(
        ts.post(
            &owner,
            &format!("/chats/{chat_id}/messages"),
            json!({
                "client_msg_id": Uuid::new_v4().to_string(),
                "body": "Pizza or pasta?",
                "attachment_id": attachment_id,
                "poll": {"options": ["Pizza", "Pasta"]},
            }),
        )
        .await,
        400,
        "invalid_poll",
    )
    .await;

    // The refusal consumed nothing: the upload is still claimable on its
    // own, as an ordinary photo message.
    let response = ts
        .post(
            &owner,
            &format!("/chats/{chat_id}/messages"),
            json!({
                "client_msg_id": Uuid::new_v4().to_string(),
                "body": "",
                "attachment_id": attachment_id,
            }),
        )
        .await;
    assert_eq!(response.status(), 201);
}

/// Options are fixed at creation, so this is the only place they are ever
/// checked: 2–10 of them, each trimmed, non-empty, at most 100 characters,
/// and no two the same ignoring case.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn poll_options_are_bounded_non_empty_and_distinct_ignoring_case() {
    let ts = spawn_server().await;
    let (owner, _, _, _, chat_id) = family_of_two(&ts).await;

    let too_many: Vec<String> = (0..11).map(|i| format!("Option {i}")).collect();
    let too_long = "x".repeat(101);
    for (why, options) in [
        ("one option is not a choice", json!(["Only one"])),
        ("eleven is one too many", json!(too_many)),
        ("an empty option", json!(["Pizza", "   "])),
        ("101 characters", json!(["Pizza", too_long])),
        ("the same option twice", json!(["Pizza", "Pizza"])),
        (
            "the same option in a different case",
            json!(["Pizza", "pizza"]),
        ),
        (
            "the same option once trimmed",
            json!(["Pizza", "  PIZZA  ", "Pasta"]),
        ),
    ] {
        let response = post_poll(&ts, &owner, chat_id, "Which?", options).await;
        assert_eq!(response.status(), 400, "should have been refused: {why}");
        assert_error(response, 400, "invalid_poll").await;
    }

    // Characters, not bytes, exactly as the message body is counted: a
    // family that writes in Cyrillic gets the same hundred characters.
    assert_error(
        post_poll(
            &ts,
            &owner,
            chat_id,
            "Which?",
            json!(["Пицца", "я".repeat(101)]),
        )
        .await,
        400,
        "invalid_poll",
    )
    .await;
    let response = post_poll(
        &ts,
        &owner,
        chat_id,
        "Which?",
        json!(["Пицца", "я".repeat(100)]),
    )
    .await;
    assert_eq!(
        response.status(),
        201,
        "a hundred Cyrillic characters is a hundred characters, not two hundred bytes"
    );

    // Exactly at both limits is fine: ten options of a hundred characters.
    let at_the_limit: Vec<String> = (0..10).map(|i| format!("{i}{}", "x".repeat(99))).collect();
    let response = post_poll(&ts, &owner, chat_id, "Which?", json!(at_the_limit)).await;
    assert_eq!(response.status(), 201);
    let body: Value = response.json().await.expect("message response is JSON");
    let (_, _, options) = read_poll(&body["message"]["poll"]);
    assert_eq!(options.len(), 10);
    assert_eq!(options[0].1.chars().count(), 100);

    // Not one poll was created by any of the refusals above.
    let feed = poll_feed(&ts, &owner, chat_id, "").await;
    assert_eq!(feed.len(), 2, "a refused poll must leave nothing behind");
}

/// `after_id` is `WHERE id > cursor` and can never see a change to an older
/// row. The poll feed replays by `poll_seq` instead, oldest change first,
/// full current state per poll, looped until a short page.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_catch_up_feed_replays_state_after_a_seq_ascending_and_pages() {
    let ts = spawn_server().await;
    let (owner, _, member, member_id, chat_id) = family_of_two(&ts).await;
    let (first, first_seq, first_options) = a_poll(&ts, &owner, chat_id).await;
    let (second, second_seq, _) = a_poll(&ts, &owner, chat_id).await;
    let (third, third_seq, _) = a_poll(&ts, &owner, chat_id).await;
    assert!(first_seq < second_seq && second_seq < third_seq);

    // From a zero cursor: every poll the chat holds, ascending.
    let feed = poll_feed(&ts, &member, chat_id, "").await;
    assert_eq!(
        feed.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        vec![first, second, third],
        "ordered by poll_seq ascending"
    );

    // …and it pages: a short page is how a client knows it is done.
    let page = poll_feed(&ts, &member, chat_id, "?after_seq=0&limit=2").await;
    assert_eq!(
        page.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        vec![first, second]
    );
    let (_, (cursor, _, _)) = &page[1];
    let cursor = *cursor;
    assert_eq!(cursor, second_seq);
    let page = poll_feed(
        &ts,
        &member,
        chat_id,
        &format!("?after_seq={cursor}&limit=2"),
    )
    .await;
    assert_eq!(
        page.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        vec![third]
    );

    // Voting on the OLDEST poll re-stamps it to the newest seq, which is
    // exactly the change `after_id` could never show.
    let (vote_seq, _, _) =
        state_of(vote(&ts, &member, chat_id, first, first_options[1]).await).await;
    assert!(vote_seq > third_seq);
    let page = poll_feed(&ts, &member, chat_id, &format!("?after_seq={third_seq}")).await;
    assert_eq!(page.len(), 1, "only what changed after the cursor");
    let (id, (seq, closed, state)) = page.into_iter().next().expect("one entry");
    assert_eq!(id, first);
    assert_eq!(seq, vote_seq);
    assert!(!closed);
    assert_eq!(
        voters(&state, 1),
        vec![member_id],
        "the feed carries full current state, never a delta"
    );
    assert_eq!(state.len(), 2, "options come with it, in creation order");

    // Nothing past the newest cursor: the short page that ends the loop.
    assert!(
        poll_feed(&ts, &member, chat_id, &format!("?after_seq={vote_seq}"))
            .await
            .is_empty()
    );

    // Guard rails.
    assert_error(
        ts.get(&member, &format!("/chats/{chat_id}/polls?after_seq=abc"))
            .await,
        400,
        "invalid_pagination",
    )
    .await;
    let (stranger, _) = ts.register("stranger", "Sam").await;
    assert_error(
        ts.get(&stranger, &format!("/chats/{chat_id}/polls")).await,
        403,
        "not_chat_member",
    )
    .await;
    assert_error(
        ts.get(&member, "/chats/99999999/polls").await,
        404,
        "chat_not_found",
    )
    .await;
}

/// Retrying with the same `client_msg_id` returns the existing message —
/// never a duplicate — and a poll is part of that message, not a second
/// write that could land twice.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn re_sending_a_poll_creates_no_second_poll() {
    let ts = spawn_server().await;
    let (owner, _, member, member_id, chat_id) = family_of_two(&ts).await;

    let client_msg_id = Uuid::new_v4().to_string();
    let payload = json!({
        "client_msg_id": client_msg_id,
        "body": "Pizza or pasta?",
        "poll": {"options": ["Pizza", "Pasta"]},
    });
    let response = ts
        .post(
            &owner,
            &format!("/chats/{chat_id}/messages"),
            payload.clone(),
        )
        .await;
    assert_eq!(response.status(), 201);
    let first: Value = response.json().await.expect("message response is JSON");
    let message_id = first["message"]["id"].as_i64().expect("message id");
    let (_, _, options) = read_poll(&first["message"]["poll"]);

    // A vote in between, so the re-ack has to carry state rather than a
    // remembered copy of what was created.
    let (vote_seq, _, _) =
        state_of(vote(&ts, &member, chat_id, message_id, options[0].0).await).await;

    let response = ts
        .post(&owner, &format!("/chats/{chat_id}/messages"), payload)
        .await;
    assert_eq!(
        response.status(),
        200,
        "a retry is a dedup hit, not a create"
    );
    let retry: Value = response.json().await.expect("message response is JSON");
    assert_eq!(
        retry["message"]["id"].as_i64(),
        Some(message_id),
        "the first write wins"
    );
    let (seq, closed, state) = read_poll(&retry["message"]["poll"]);
    assert_eq!(seq, vote_seq, "the re-ack carries the poll as it now is");
    assert!(!closed);
    assert_eq!(voters(&state, 0), vec![member_id]);
    assert_eq!(
        state.iter().map(|(id, _, _)| *id).collect::<Vec<_>>(),
        options.iter().map(|(id, _, _)| *id).collect::<Vec<_>>(),
        "the same options, not a second set"
    );

    // One message, one poll, one set of options.
    assert_eq!(poll_feed(&ts, &owner, chat_id, "").await.len(), 1);
    let page: Value = ts
        .get(&owner, &format!("/chats/{chat_id}/messages"))
        .await
        .json()
        .await
        .expect("messages page is JSON");
    assert_eq!(page["messages"].as_array().expect("array").len(), 1);
    assert_eq!(
        count(
            &ts,
            "SELECT count(*) FROM poll_options WHERE poll_id = $1",
            message_id
        )
        .await,
        2
    );
}

/// A poll dies with its message and nothing has to remember to take it —
/// the retention sweep removes the message and the poll, its options and
/// its votes go with it.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_poll_dies_with_its_message_when_the_retention_sweep_takes_it() {
    let ts = spawn_server().await;
    let (owner, _, member, _, chat_id) = family_of_two(&ts).await;
    let days = ts.state.cfg.limits.retention_days;

    let (old, _, old_options) = a_poll(&ts, &owner, chat_id).await;
    state_of(vote(&ts, &member, chat_id, old, old_options[0]).await).await;
    let (fresh, _, _) = a_poll(&ts, &owner, chat_id).await;

    assert_eq!(
        count(
            &ts,
            "SELECT count(*) FROM poll_votes WHERE poll_id = $1",
            old
        )
        .await,
        1
    );

    age(&ts, old, days + 1).await;
    let swept = family_connect::handlers_chat::sweep_expired_messages(&ts.state)
        .await
        .expect("sweep");
    assert_eq!(swept, 1);

    // Poll, options and votes all went with the message, on the cascade.
    assert_eq!(
        count(&ts, "SELECT count(*) FROM polls WHERE message_id = $1", old).await,
        0
    );
    assert_eq!(
        count(
            &ts,
            "SELECT count(*) FROM poll_options WHERE poll_id = $1",
            old
        )
        .await,
        0
    );
    assert_eq!(
        count(
            &ts,
            "SELECT count(*) FROM poll_votes WHERE poll_id = $1",
            old
        )
        .await,
        0
    );

    // It is gone from history and from the catch-up feed; the younger poll
    // is untouched, options and all.
    let page: Value = ts
        .get(&owner, &format!("/chats/{chat_id}/messages"))
        .await
        .json()
        .await
        .expect("messages page is JSON");
    let ids: Vec<i64> = page["messages"]
        .as_array()
        .expect("array")
        .iter()
        .map(|m| m["id"].as_i64().expect("id"))
        .collect();
    assert_eq!(ids, vec![fresh]);
    let feed = poll_feed(&ts, &owner, chat_id, "").await;
    assert_eq!(feed.len(), 1);
    assert_eq!(feed[0].0, fresh);
    let (_, (_, _, surviving_options)) = &feed[0];
    assert_eq!(surviving_options.len(), 2);

    // And the swept poll is simply not there any more.
    assert_error(
        vote(&ts, &member, chat_id, old, old_options[0]).await,
        404,
        "message_not_found",
    )
    .await;
}

/// Every change to a poll reaches every member of the chat as a `poll`
/// frame carrying its FULL CURRENT STATE — the voter's own connections
/// included, because one frame is serialised once for everybody — and none
/// of it ever wakes a phone: a vote is not a message.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_vote_reaches_every_member_connection_as_a_poll_frame_and_never_pushes() {
    let ts = spawn_server().await;
    let (owner, _, member, member_id, chat_id) = family_of_two(&ts).await;

    // Only the author carries a push token, so creating the poll (which IS
    // a message and does push) records nothing — leaving the push log's
    // emptiness at the end to mean exactly one thing.
    let response = ts
        .post(
            &owner,
            "/devices",
            json!({"platform": "ios", "push_token": "tok-owner"}),
        )
        .await;
    assert_eq!(response.status(), 201);
    let (message_id, creation_seq, options) = a_poll(&ts, &owner, chat_id).await;

    let mut owner_ws = connect_ws(&ts, &owner).await;
    let mut member_ws = connect_ws(&ts, &member).await;
    let mut member_ws2 = connect_ws(&ts, &member).await;

    // The voter's request is answered over HTTP; every connection in the
    // chat — the voter's own two included — learns of it from the frame.
    let response = vote(&ts, &member, chat_id, message_id, options[1]).await;
    assert_eq!(response.status(), 200);
    for ws in [&mut owner_ws, &mut member_ws, &mut member_ws2] {
        let frame = next_frame_of_type(ws, "poll").await;
        assert_eq!(frame["chat_id"].as_i64(), Some(chat_id));
        assert_eq!(frame["message_id"].as_i64(), Some(message_id));
        let (seq, closed, state) = read_poll(&frame["poll"]);
        assert!(seq > creation_seq);
        assert!(!closed);
        assert_eq!(
            state
                .iter()
                .map(|(_, text, _)| text.as_str())
                .collect::<Vec<_>>(),
            vec!["Pizza", "Pasta"],
            "full state: the options come with it, not just the delta"
        );
        assert_eq!(voters(&state, 0), Vec::<i64>::new());
        assert_eq!(voters(&state, 1), vec![member_id]);
    }

    // A no-op re-PUT fans nothing out: re-voting for the option you hold is
    // not an event anyone needs to hear about.
    let response = vote(&ts, &member, chat_id, message_id, options[1]).await;
    assert_eq!(response.status(), 200);
    assert_no_frame_of_type(&mut owner_ws, "poll", Duration::from_millis(1200)).await;

    // Now take the author — the one account with a push token — fully
    // offline and change the vote. The member's socket proves the frame was
    // delivered; the push log proves nobody's phone was woken for it.
    drop(owner_ws);
    let response = ts
        .delete(
            &member,
            &format!("/chats/{chat_id}/messages/{message_id}/vote"),
        )
        .await;
    assert_eq!(response.status(), 200);
    let frame = next_frame_of_type(&mut member_ws, "poll").await;
    let (_, _, state) = read_poll(&frame["poll"]);
    assert_eq!(voters(&state, 1), Vec::<i64>::new());

    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        ts.push.calls().is_empty(),
        "a vote, a retraction and a close never reach the push seam"
    );
}

/// The endpoint rows' 404s: the three mutating verbs answer for a poll IN
/// THIS CHAT and nothing else. A message that is not a poll, a poll reached
/// through another chat and an id that names nothing are one answer, so the
/// endpoint never confirms an id the caller cannot otherwise see.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_vote_and_close_endpoints_answer_only_for_a_poll_in_this_chat() {
    let ts = spawn_server().await;
    let (owner, _, member, member_id, chat_id) = family_of_two(&ts).await;
    let (message_id, _, options) = a_poll(&ts, &owner, chat_id).await;
    let option_id = options[0];

    // An ordinary message in the same chat, which is not a poll.
    let response = ts
        .post_message(&owner, chat_id, &Uuid::new_v4().to_string(), "just talking")
        .await;
    assert_eq!(response.status(), 201);
    let plain: Value = response.json().await.expect("message response is JSON");
    let plain_id = plain["message"]["id"].as_i64().expect("message id");
    assert!(
        plain["message"].get("poll").is_none(),
        "an ordinary message carries no poll key at all: {plain}"
    );

    // A real chat of the caller's through which the poll is not reachable.
    let direct: Value = ts
        .post(&owner, "/chats/direct", json!({"user_id": member_id}))
        .await
        .json()
        .await
        .expect("direct chat is JSON");
    let direct_id = direct["chat"]["id"].as_i64().expect("direct chat id");

    for (chat, message) in [
        (chat_id, plain_id),     // not a poll
        (direct_id, message_id), // a poll, through the wrong chat
        (chat_id, 99_999_999),   // nothing at all
    ] {
        assert_error(
            vote(&ts, &member, chat, message, option_id).await,
            404,
            "message_not_found",
        )
        .await;
        assert_error(
            ts.delete(&member, &format!("/chats/{chat}/messages/{message}/vote"))
                .await,
            404,
            "message_not_found",
        )
        .await;
        assert_error(
            ts.post(
                &owner,
                &format!("/chats/{chat}/messages/{message}/poll/close"),
                json!({}),
            )
            .await,
            404,
            "message_not_found",
        )
        .await;
    }

    // A chat that does not exist is `chat_not_found`, and one the caller is
    // not in is `not_chat_member` — a poll is only ever reached through a
    // chat the caller can already see.
    assert_error(
        vote(&ts, &member, 99_999_999, message_id, option_id).await,
        404,
        "chat_not_found",
    )
    .await;
    let (stranger, _) = ts.register("stranger", "Sam").await;
    assert_error(
        vote(&ts, &stranger, chat_id, message_id, option_id).await,
        403,
        "not_chat_member",
    )
    .await;
    assert_error(
        ts.delete(
            &stranger,
            &format!("/chats/{chat_id}/messages/{message_id}/vote"),
        )
        .await,
        403,
        "not_chat_member",
    )
    .await;
    assert_error(
        ts.post(
            &stranger,
            &format!("/chats/{chat_id}/messages/{message_id}/poll/close"),
            json!({}),
        )
        .await,
        403,
        "not_chat_member",
    )
    .await;

    // None of that touched the poll: still open, still nobody's vote.
    let (_, closed, state) = state_of(
        ts.delete(
            &member,
            &format!("/chats/{chat_id}/messages/{message_id}/vote"),
        )
        .await,
    )
    .await;
    assert!(!closed);
    assert_eq!(voters(&state, 0), Vec::<i64>::new());
}

/// Editing the message edits the QUESTION, through the ordinary author-only
/// edit path, and that is as far as changing a poll goes: the options were
/// fixed at creation and the votes already cast stay where they were cast.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn editing_a_poll_changes_its_question_and_leaves_the_options_alone() {
    let ts = spawn_server().await;
    let (owner, _, member, member_id, chat_id) = family_of_two(&ts).await;
    let (message_id, _, options) = a_poll(&ts, &owner, chat_id).await;
    let (vote_seq, _, before) =
        state_of(vote(&ts, &member, chat_id, message_id, options[0]).await).await;
    let path = format!("/chats/{chat_id}/messages/{message_id}");

    // Not the voter's to edit — a poll is a message, and editing one is an
    // authorship act like any other.
    assert_error(
        ts.patch(&member, &path, json!({"body": "Sushi?"})).await,
        403,
        "not_message_author",
    )
    .await;

    let response = ts
        .patch(&owner, &path, json!({"body": "Pizza or pasta tonight?"}))
        .await;
    assert_eq!(response.status(), 200);
    let edited: Value = response.json().await.expect("edit response is JSON");
    assert_eq!(edited["message"]["body"], "Pizza or pasta tonight?");
    assert!(edited["message"]["edited_at"].is_string());
    let (seq, closed, after) = read_poll(&edited["message"]["poll"]);
    assert_eq!(
        seq, vote_seq,
        "an edit is a message change, not a poll change: it burns no poll seq"
    );
    assert!(!closed);
    assert_eq!(after, before, "the options and the votes are untouched");
    assert_eq!(voters(&after, 0), vec![member_id]);

    // Every reader sees the new question against the same poll.
    let fetched = message_on_page(&ts, &member, chat_id, message_id).await;
    assert_eq!(fetched["body"], "Pizza or pasta tonight?");
    assert_eq!(read_poll(&fetched["poll"]), (vote_seq, false, before));

    // And the chat's poll cursor did not move for an edit.
    let entry = chat_entry(&ts, &member, chat_id).await;
    assert_eq!(entry["max_poll_seq"].as_i64(), Some(vote_seq));
}
