//! Integration: the WebSocket path — upgrade auth, ack/message fan-out,
//! read/typing relays, membership frames, the offline push hook, and
//! logout-driven closes. Uses tokio-tungstenite as a real external client.

mod common;

use std::time::Duration;

use common::{TestServer, spawn_server};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::http::header::AUTHORIZATION;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
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

/// Family of two, both registered — the standard WS fixture.
async fn family_of_two(ts: &TestServer) -> (String, i64, String, i64, String) {
    let (owner, owner_id) = ts.register("owner", "Olive").await;
    let (member, member_id) = ts.register("junior", "Junior").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&member, &invite_code, "joined").await;
    (owner, owner_id, member, member_id, invite_code)
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_ws_send_acks_the_sender_and_delivers_message_to_the_other_member() {
    let ts = spawn_server().await;
    let (owner, owner_id, member, _, _) = family_of_two(&ts).await;
    let chat_id = ts.family_chat_id(&owner).await;

    let mut owner_ws = connect_ws(&ts, &owner).await;
    let mut member_ws = connect_ws(&ts, &member).await;

    let client_msg_id = Uuid::new_v4().to_string();
    send_frame(
        &mut owner_ws,
        json!({"type": "send", "chat_id": chat_id, "client_msg_id": client_msg_id, "body": "Dinner at 7?"}),
    )
    .await;

    let ack = next_frame_of_type(&mut owner_ws, "ack").await;
    assert_eq!(ack["client_msg_id"], client_msg_id.as_str());
    assert_eq!(ack["message"]["body"], "Dinner at 7?");
    assert_eq!(ack["message"]["sender_id"], owner_id);
    let message_id = ack["message"]["id"].as_i64().expect("message id");

    let delivered = next_frame_of_type(&mut member_ws, "message").await;
    assert_eq!(delivered["message"]["id"], message_id);
    assert_eq!(delivered["message"]["body"], "Dinner at 7?");

    // A retried send re-acks with the original message and does not
    // re-fan-out to the other member.
    send_frame(
        &mut owner_ws,
        json!({"type": "send", "chat_id": chat_id, "client_msg_id": client_msg_id, "body": "Dinner at 7?"}),
    )
    .await;
    let re_ack = next_frame_of_type(&mut owner_ws, "ack").await;
    assert_eq!(re_ack["message"]["id"], message_id);
    assert_no_frame_of_type(&mut member_ws, "message", Duration::from_millis(1200)).await;
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_rest_post_reaches_every_connection_including_the_senders_own() {
    let ts = spawn_server().await;
    let (owner, _, member, _, _) = family_of_two(&ts).await;
    let chat_id = ts.family_chat_id(&owner).await;

    let mut owner_ws = connect_ws(&ts, &owner).await;
    let mut member_ws = connect_ws(&ts, &member).await;

    let response = ts
        .post_message(&owner, chat_id, &Uuid::new_v4().to_string(), "over REST")
        .await;
    assert_eq!(response.status(), 201);

    // REST is acked by the HTTP response; the socket side is `message` for
    // everyone, the sender's own connection included.
    let to_owner = next_frame_of_type(&mut owner_ws, "message").await;
    assert_eq!(to_owner["message"]["body"], "over REST");
    let to_member = next_frame_of_type(&mut member_ws, "message").await;
    assert_eq!(to_member["message"]["body"], "over REST");
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn read_and_typing_frames_relay_to_other_members_with_typing_throttled() {
    let ts = spawn_server().await;
    let (owner, owner_id, member, _, _) = family_of_two(&ts).await;
    let chat_id = ts.family_chat_id(&owner).await;

    let mut owner_ws = connect_ws(&ts, &owner).await;
    let mut member_ws = connect_ws(&ts, &member).await;

    send_frame(
        &mut owner_ws,
        json!({"type": "read", "chat_id": chat_id, "last_read_message_id": 0}),
    )
    .await;
    let read = next_frame_of_type(&mut member_ws, "read").await;
    assert_eq!(read["chat_id"], chat_id);
    assert_eq!(read["user_id"], owner_id);

    send_frame(&mut owner_ws, json!({"type": "typing", "chat_id": chat_id})).await;
    let typing = next_frame_of_type(&mut member_ws, "typing").await;
    assert_eq!(typing["user_id"], owner_id);

    // A second typing inside the 3 s throttle window is swallowed.
    send_frame(&mut owner_ws, json!({"type": "typing", "chat_id": chat_id})).await;
    assert_no_frame_of_type(&mut member_ws, "typing", Duration::from_millis(1200)).await;
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_send_into_a_foreign_chat_answers_with_an_error_frame() {
    let ts = spawn_server().await;
    let (owner, _, _, _, _) = family_of_two(&ts).await;
    let mut owner_ws = connect_ws(&ts, &owner).await;

    let client_msg_id = Uuid::new_v4().to_string();
    send_frame(
        &mut owner_ws,
        json!({"type": "send", "chat_id": 99999999, "client_msg_id": client_msg_id, "body": "hello"}),
    )
    .await;
    let error = next_frame_of_type(&mut owner_ws, "error").await;
    assert_eq!(error["code"], "chat_not_found");
    assert_eq!(
        error["client_msg_id"],
        client_msg_id.as_str(),
        "errors answering a send must echo the client_msg_id"
    );
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn membership_changes_fan_out_as_member_joined_and_member_left() {
    let ts = spawn_server().await;
    let (owner, _, member, member_id, invite_code) = family_of_two(&ts).await;
    let mut owner_ws = connect_ws(&ts, &owner).await;

    // A third user joins the open family — the owner hears about it live.
    let (third, third_id) = ts.register("third", "Third").await;
    ts.join(&third, &invite_code, "joined").await;
    let joined = next_frame_of_type(&mut owner_ws, "member_joined").await;
    assert_eq!(joined["user"]["id"], third_id);
    assert_eq!(joined["user"]["username"], "third");

    // A member leaves — the owner hears that too.
    let response = ts.post(&member, "/families/leave", json!({})).await;
    assert_eq!(response.status(), 204);
    let left = next_frame_of_type(&mut owner_ws, "member_left").await;
    assert_eq!(left["user_id"], member_id);
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn offline_members_with_push_tokens_reach_the_push_seam() {
    let ts = spawn_server().await;
    let (owner, _, member, _, invite_code) = family_of_two(&ts).await;
    let (third, third_id) = ts.register("third", "Third").await;
    ts.join(&third, &invite_code, "joined").await;
    let chat_id = ts.family_chat_id(&owner).await;

    // The online member and the offline third both have push tokens; only
    // the offline one must be pushed.
    let response = ts
        .post(
            &member,
            "/devices",
            json!({"platform": "android", "push_token": "tok-member"}),
        )
        .await;
    assert_eq!(response.status(), 201);
    let response = ts
        .post(
            &third,
            "/devices",
            json!({"platform": "ios", "push_token": "tok-third"}),
        )
        .await;
    assert_eq!(response.status(), 201);

    let mut owner_ws = connect_ws(&ts, &owner).await;
    let mut member_ws = connect_ws(&ts, &member).await;

    send_frame(
        &mut owner_ws,
        json!({"type": "send", "chat_id": chat_id,
               "client_msg_id": Uuid::new_v4().to_string(), "body": "anyone home?"}),
    )
    .await;
    next_frame_of_type(&mut owner_ws, "ack").await;
    next_frame_of_type(&mut member_ws, "message").await;

    // Push dispatch is spawned fire-and-forget; poll briefly.
    let deadline = tokio::time::Instant::now() + FRAME_WAIT;
    let calls = loop {
        let calls = ts.push.calls();
        if !calls.is_empty() {
            break calls;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the push seam was never called for the offline member"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(calls.len(), 1, "exactly one notify call for one message");
    let call = &calls[0];
    assert_eq!(call.message.body, "anyone home?");
    assert_eq!(call.devices.len(), 1, "only the offline member's device");
    assert_eq!(call.devices[0].user_id, third_id);
    assert_eq!(call.devices[0].push_token, "tok-third");
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn an_upgrade_with_a_bad_token_is_rejected_with_401() {
    let ts = spawn_server().await;
    let mut request = ts
        .ws_url
        .as_str()
        .into_client_request()
        .expect("building the ws request");
    request.headers_mut().insert(
        AUTHORIZATION,
        HeaderValue::from_static("Bearer not-a-real-token"),
    );
    match tokio_tungstenite::connect_async(request).await {
        Err(WsError::Http(response)) => assert_eq!(response.status(), 401),
        Ok(_) => panic!("upgrade with a bad token must not succeed"),
        Err(other) => panic!("expected an HTTP 401 rejection, got: {other}"),
    }
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn logging_out_closes_the_sessions_sockets_with_4401() {
    let ts = spawn_server().await;
    let (owner, _, _, _, _) = family_of_two(&ts).await;
    let mut owner_ws = connect_ws(&ts, &owner).await;

    let response = ts.post(&owner, "/auth/logout", json!({})).await;
    assert_eq!(response.status(), 204);

    // The server closes the socket; the client sees a Close frame (with the
    // session-gone code) and then the end of the stream.
    let deadline = tokio::time::Instant::now() + FRAME_WAIT;
    loop {
        match tokio::time::timeout_at(deadline, owner_ws.next())
            .await
            .expect("the socket must close after logout")
        {
            Some(Ok(Message::Close(frame))) => {
                let frame = frame.expect("close frame carries a code");
                assert_eq!(u16::from(frame.code), 4401);
                break;
            }
            Some(Ok(_)) => continue,
            // Some client stacks surface the close as end-of-stream.
            None => break,
            Some(Err(err)) => panic!("expected a clean close, got: {err}"),
        }
    }
}
