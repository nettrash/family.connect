//! The assistant's chat, as far as it can be exercised without calling
//! Azure (docs/protocol.md, "The assistant").
//!
//! No reply is generated here — that needs the real provider — but
//! everything AROUND the reply is testable and is exactly where the first
//! bug lived: `ensure_ai_chat` ran a query with a `$1` and never bound it,
//! so the chat was never created and the only sign was a WARN line in the
//! server log. The endpoint answered 200 with a chat list that silently
//! lacked the assistant.

mod common;

use common::{TestServer, spawn_server, spawn_server_with_config};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::http::header::AUTHORIZATION;

/// A server with the assistant configured. The endpoint is never called in
/// these tests; what matters is that `is_usable()` is true, which is what
/// gates creating the chat at all.
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

/// Just the `chat` objects: a list entry wraps one alongside its
/// `last_message` and `unread_count`.
async fn chats(ts: &TestServer, token: &str) -> Vec<Value> {
    let body: Value = ts.get(token, "/chats").await.json().await.expect("JSON");
    body["chats"]
        .as_array()
        .map(|entries| entries.iter().map(|entry| entry["chat"].clone()).collect())
        .unwrap_or_default()
}

/// The regression: the chat has to actually appear.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_configured_server_gives_each_member_an_assistant_chat() {
    let ts = server_with_assistant().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    ts.create_family(&owner, "The Smiths").await;

    let list = chats(&ts, &owner).await;
    let ai = list
        .iter()
        .find(|chat| chat["kind"] == "ai")
        .expect("the assistant chat must be in the list");
    assert_eq!(ai["title"], "Assistant");
    assert!(
        ai["peer_user_id"].is_null(),
        "the assistant is not a peer — it has no account in the family"
    );
}

/// Created once, not once per request: `GET /chats` ensures it on every
/// call, and a second call must not add a second chat.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_assistant_chat_is_created_once() {
    let ts = server_with_assistant().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    ts.create_family(&owner, "The Smiths").await;

    let first = chats(&ts, &owner).await;
    let second = chats(&ts, &owner).await;
    let count = |list: &[Value]| list.iter().filter(|c| c["kind"] == "ai").count();
    assert_eq!(count(&first), 1);
    assert_eq!(count(&second), 1);
    assert_eq!(
        first
            .iter()
            .find(|c| c["kind"] == "ai")
            .map(|c| c["id"].clone()),
        second
            .iter()
            .find(|c| c["kind"] == "ai")
            .map(|c| c["id"].clone()),
        "the same chat, not a new one each time"
    );
}

/// Private is the whole point: one member's assistant chat is not in
/// another member's list.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn one_members_assistant_chat_is_invisible_to_another() {
    let ts = server_with_assistant().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (member, _) = ts.register("junior", "Junior").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&member, &invite_code, "joined").await;

    let owner_chats = chats(&ts, &owner).await;
    let owner_ai: Vec<i64> = owner_chats
        .iter()
        .filter(|c| c["kind"] == "ai")
        .filter_map(|c| c["id"].as_i64())
        .collect();
    let member_chats = chats(&ts, &member).await;
    let member_ai: Vec<i64> = member_chats
        .iter()
        .filter(|c| c["kind"] == "ai")
        .filter_map(|c| c["id"].as_i64())
        .collect();

    assert_eq!(owner_ai.len(), 1);
    assert_eq!(member_ai.len(), 1);
    assert_ne!(
        owner_ai[0], member_ai[0],
        "each member has their OWN assistant chat"
    );

    // And the other member's is not merely absent from the list — it is
    // unreadable.
    let peeking = ts
        .get(&member, &format!("/chats/{}/messages", owner_ai[0]))
        .await;
    assert_eq!(peeking.status(), 403);
}

/// With no `[ai]` section there is no assistant, and nothing in the list
/// hints that there could be.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_unconfigured_server_creates_no_assistant_chat() {
    let ts = spawn_server().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    ts.create_family(&owner, "The Smiths").await;

    let list = chats(&ts, &owner).await;
    assert!(
        list.iter().all(|chat| chat["kind"] != "ai"),
        "the assistant must not exist unless it is configured"
    );
}

/// The reserved account cannot be taken, in any casing — it is what the
/// assistant sends under.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_assistant_username_is_reserved() {
    let ts = spawn_server().await;
    for name in ["assistant", "Assistant", "ASSISTANT"] {
        let response = ts
            .post_unauth(
                "/auth/register",
                serde_json::json!({
                    "username": name,
                    "display_name": "Impostor",
                    "password": "hunter2hunter2",
                }),
            )
            .await;
        assert_eq!(response.status(), 400, "{name} must be refused");
    }
}

/// Asking over the SOCKET must reach the assistant, not only over REST.
///
/// This is the bug that shipped: the trigger lived in `post_message`, and a
/// client with a live socket sends over it — so asking in the app produced
/// no answer at all while curl worked perfectly. The trigger now lives in
/// `create_message`, which both transports go through.
///
/// The reply itself cannot land here (the endpoint is unreachable), but the
/// PLACEHOLDER row is created before the provider is called, so its
/// appearance proves the assistant was invoked.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn asking_over_the_socket_reaches_the_assistant() {
    let ts = server_with_assistant().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    ts.create_family(&owner, "The Smiths").await;

    let ai_chat = chats(&ts, &owner)
        .await
        .into_iter()
        .find(|chat| chat["kind"] == "ai")
        .and_then(|chat| chat["id"].as_i64())
        .expect("the assistant chat exists");

    // Over the socket, exactly as a connected client does.
    let mut ws = connect_ws(&ts, &owner).await;
    ws.send(Message::text(
        json!({
            "type": "send",
            "chat_id": ai_chat,
            "client_msg_id": uuid::Uuid::new_v4().to_string(),
            "body": "are you there?",
        })
        .to_string(),
    ))
    .await
    .expect("sending over the socket");

    // The ack for our own message comes back first.
    let _ack = next_frame_of_type(&mut ws, "ack").await;

    // Then the assistant's empty placeholder, fanned out as an ordinary
    // message. It is created BEFORE the provider is called, so this arrives
    // even though the endpoint is unreachable.
    let placeholder = next_frame_of_type(&mut ws, "message").await;
    assert_eq!(placeholder["message"]["chat_id"].as_i64(), Some(ai_chat));
    assert_eq!(
        placeholder["message"]["body"], "",
        "the placeholder starts empty and is filled by the stream"
    );
}

/// `GET /families/mine` is where a client learns the assistant's user id —
/// without it a mention's reply is a nameless bubble in the family chat,
/// because the account is deliberately in no roster.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_configured_server_names_the_assistant_on_the_family() {
    let ts = server_with_assistant().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    ts.create_family(&owner, "The Smiths").await;

    let body: Value = ts
        .get(&owner, "/families/mine")
        .await
        .json()
        .await
        .expect("JSON");
    let assistant = &body["assistant"];
    assert!(
        assistant["user_id"].as_i64().is_some_and(|id| id > 0),
        "the assistant's user id must be given: {body}"
    );
    assert_eq!(assistant["display_name"], "Assistant");
    assert_eq!(assistant["mention"], "@ai");

    // And it is NOT a member: every screen that lists people would need a
    // special case for something that cannot be removed or messaged.
    let members = body["members"].as_array().expect("members");
    assert!(
        members
            .iter()
            .all(|member| member["id"] != assistant["user_id"]),
        "the assistant must not be in the roster: {body}"
    );
}

/// The absence of the field is the capability check: a client that offered
/// `@ai` here would offer an affordance that does nothing.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_unconfigured_server_names_no_assistant() {
    let ts = spawn_server().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    ts.create_family(&owner, "The Smiths").await;

    let body: Value = ts
        .get(&owner, "/families/mine")
        .await
        .json()
        .await
        .expect("JSON");
    assert!(
        body.get("assistant").is_none(),
        "no assistant configured, so nothing to name: {body}"
    );
}

/// The feature: `@ai` in the family chat produces an answer for EVERYONE,
/// quoting the message that asked. The provider is unreachable here, so what
/// is asserted is the placeholder — which is created before the call and is
/// the proof the assistant was invoked at all.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn mentioning_the_assistant_in_the_family_chat_answers_the_whole_family() {
    let ts = server_with_assistant().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (_, code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    let (member, member_id) = ts.register("junior", "Junior").await;
    ts.join(&member, &code, "joined").await;

    let chat = ts.family_chat_id(&owner).await;
    let asked = say(&ts, &member, chat, "@ai what is the capital of Serbia?").await;
    let asked_id = asked["id"].as_i64().expect("the question has an id");

    // The OWNER, who did not ask, must see it: the answer is public.
    let reply = wait_for_assistant_message(&ts, &owner, chat, asked_id).await;
    assert_eq!(
        reply["body"], "",
        "the placeholder starts empty and is filled by the stream"
    );
    assert_ne!(
        reply["sender_id"].as_i64(),
        Some(member_id),
        "the assistant answers under its own account, not the asker's"
    );
    assert_eq!(
        reply["reply_to"]["message_id"].as_i64(),
        Some(asked_id),
        "the answer quotes the question, or it belongs to nobody: {reply}"
    );
}

/// The other half of the same rule, and the more important one: an ordinary
/// family message must not reach the provider.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_ordinary_family_message_does_not_reach_the_assistant() {
    let ts = server_with_assistant().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    ts.create_family(&owner, "The Smiths").await;
    let chat = ts.family_chat_id(&owner).await;

    // Deliberately full of near-misses: an email address, a name that
    // starts with the token, and the bare word.
    for body in [
        "dinner at 7?",
        "mail me at anna@ai.example",
        "@aiden is coming too",
        "ai is everywhere these days",
    ] {
        say(&ts, &owner, chat, body).await;
    }

    // Long enough that a spawned reply would have created its row.
    tokio::time::sleep(Duration::from_millis(600)).await;
    let messages = messages_in(&ts, &owner, chat).await;
    assert_eq!(
        messages.len(),
        4,
        "nothing but the four messages sent: {messages:#?}"
    );
}

/// A direct chat is two people who each already have a private assistant;
/// a third party turning up in a one-to-one is not something either asked
/// for (docs/protocol.md, "Mentioning the assistant in the family chat").
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_mention_in_a_direct_chat_does_nothing() {
    let ts = server_with_assistant().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (_, code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    let (member, member_id) = ts.register("junior", "Junior").await;
    ts.join(&member, &code, "joined").await;

    let response = ts
        .post(&owner, "/chats/direct", json!({"user_id": member_id}))
        .await;
    let body: Value = response.json().await.expect("JSON");
    let direct = body["chat"]["id"].as_i64().expect("a direct chat");

    say(&ts, &owner, direct, "@ai are you there?").await;
    tokio::time::sleep(Duration::from_millis(600)).await;

    let messages = messages_in(&ts, &owner, direct).await;
    assert_eq!(messages.len(), 1, "only what was typed: {messages:#?}");
}

/// Leaving a family deletes the member's row from `chat_members` for every
/// chat of that family — the assistant chat included. Without restoring it
/// on the way back in, a member who left and rejoined is locked out of a
/// private thread nobody else can even see.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn rejoining_a_family_restores_the_assistant_chat() {
    let ts = server_with_assistant().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (_, code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    let (member, _) = ts.register("junior", "Junior").await;
    ts.join(&member, &code, "joined").await;

    let before = chats(&ts, &member)
        .await
        .into_iter()
        .find(|chat| chat["kind"] == "ai")
        .and_then(|chat| chat["id"].as_i64())
        .expect("the assistant chat exists");
    say(&ts, &member, before, "remember this").await;

    assert_eq!(
        ts.post(&member, "/families/leave", json!({}))
            .await
            .status(),
        204
    );
    ts.join(&member, &code, "joined").await;

    let after = chats(&ts, &member)
        .await
        .into_iter()
        .find(|chat| chat["kind"] == "ai")
        .and_then(|chat| chat["id"].as_i64())
        .expect("the assistant chat comes back rather than vanishing");
    assert_eq!(after, before, "the same chat, not a fresh one");

    // And it is READABLE again, which is the part membership decides.
    // The count is deliberately not asserted: asking the assistant anything
    // also creates its (empty) answer row.
    let messages = messages_in(&ts, &member, after).await;
    assert!(
        messages
            .iter()
            .any(|message| message["body"] == "remember this"),
        "the history is still there: {messages:#?}"
    );
}

/// The reserved account is seeded with a password hash nobody can present.
/// It must answer like any other unknown login — a 500 for one username and
/// a 401 for every other is an existence oracle.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn logging_in_as_the_assistant_is_refused_like_any_other() {
    let ts = server_with_assistant().await;
    let response = ts.login_raw("assistant", "anything at all").await;
    assert_eq!(
        response.status(),
        401,
        "the reserved account must look exactly like a wrong password"
    );
    let body: Value = response.json().await.expect("JSON");
    assert_eq!(body["error"]["code"], "invalid_credentials");
}

/// Post a message and hand back the Message the server made of it.
async fn say(ts: &TestServer, token: &str, chat_id: i64, body: &str) -> Value {
    let response = ts
        .post_message(token, chat_id, &uuid::Uuid::new_v4().to_string(), body)
        .await;
    assert_eq!(response.status(), 201, "sending {body:?}");
    let sent: Value = response.json().await.expect("JSON");
    sent["message"].clone()
}

async fn messages_in(ts: &TestServer, token: &str, chat_id: i64) -> Vec<Value> {
    let body: Value = ts
        .get(token, &format!("/chats/{chat_id}/messages"))
        .await
        .json()
        .await
        .expect("JSON");
    body["messages"].as_array().cloned().unwrap_or_default()
}

/// Poll for the assistant's row. The reply is spawned, so it does not exist
/// the moment the send returns — and polling beats a fixed sleep, which is
/// either flaky or slow.
async fn wait_for_assistant_message(
    ts: &TestServer,
    token: &str,
    chat_id: i64,
    after_id: i64,
) -> Value {
    for _ in 0..50 {
        let messages = messages_in(ts, token, chat_id).await;
        if let Some(found) = messages
            .into_iter()
            .find(|message| message["id"].as_i64().is_some_and(|id| id > after_id))
        {
            return found;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the assistant never created its placeholder");
}

/// A location sent the way a CONNECTED client sends one — over the socket
/// — and received the way another member's client receives it.
///
/// The live `message` frame is its own path: it does not come from the
/// joined read used by history, it is built by `create_message` from the
/// INSERT's RETURNING plus `claim_attachment`'s. A location has NO BYTES,
/// so if either of those forgets the coordinate columns the frame carries
/// an attachment that renders as nothing at all — and the sender, whose own
/// device already holds them, sees nothing wrong.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_location_sent_over_the_socket_arrives_with_its_coordinates() {
    let ts = spawn_server().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (_, code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    let (member, _) = ts.register("junior", "Junior").await;
    ts.join(&member, &code, "joined").await;
    let chat = ts.family_chat_id(&owner).await;

    // The upload, exactly as a client with no bytes makes it: metadata in
    // the query string, empty body, no content type.
    let response = ts
        .put_bytes_method(
            "POST",
            &member,
            "/attachments?kind=location&latitude=55.7558&longitude=37.6173&accuracy_m=12",
            "",
            Vec::new(),
        )
        .await;
    assert_eq!(response.status(), 201);
    let uploaded: Value = response.json().await.expect("JSON");
    let attachment_id = uploaded["attachment"]["id"].as_i64().expect("id");

    // The OTHER member is listening, which is the case that matters.
    let mut watcher = connect_ws(&ts, &owner).await;
    let mut sender = connect_ws(&ts, &member).await;
    sender
        .send(Message::text(
            json!({
                "type": "send",
                "chat_id": chat,
                "client_msg_id": uuid::Uuid::new_v4().to_string(),
                "body": "",
                "attachment_id": attachment_id,
            })
            .to_string(),
        ))
        .await
        .expect("sending over the socket");

    let frame = next_frame_of_type(&mut watcher, "message").await;
    let attachment = &frame["message"]["attachment"];
    assert_eq!(attachment["kind"], "location", "frame: {frame}");
    assert_eq!(
        attachment["latitude"].as_f64(),
        Some(55.7558),
        "the LIVE frame must carry the pin, or the bubble draws nothing: {frame}"
    );
    assert_eq!(attachment["longitude"].as_f64(), Some(37.6173));
    assert_eq!(attachment["accuracy_m"].as_i64(), Some(12));

    // ...and so must the history read the other client falls back on.
    let read: Value = ts
        .get(&owner, &format!("/chats/{chat}/messages"))
        .await
        .json()
        .await
        .expect("JSON");
    let drawn = &read["messages"][0]["attachment"];
    assert_eq!(drawn["latitude"].as_f64(), Some(55.7558), "history: {read}");
}

// -- socket helpers (mirroring push_flow.rs) ---------------------------------

type WsClient =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

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
    // A pong proves the connection task is registered — a later fan-out
    // cannot race past a connection that has already answered a frame.
    ws.send(Message::text(json!({"type": "ping"}).to_string()))
        .await
        .expect("ping");
    let _pong = next_frame_of_type(&mut ws, "pong").await;
    ws
}

async fn next_frame_of_type(ws: &mut WsClient, wanted: &str) -> Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
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
