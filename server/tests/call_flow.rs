//! Integration: the voice-call signalling path (docs/protocol.md, "Voice
//! calls"). Real WebSocket clients via tokio-tungstenite, the same fixture
//! style as ws_flow.rs; the push seam is the recording double, so an
//! incoming-call push is asserted by inspecting what it captured.

mod common;

use std::time::Duration;

use common::{TestServer, spawn_server, spawn_server_with_config};
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

/// Connect WITHOUT the ping/pong handshake, so the frames the server replays
/// the moment a connection registers (a ringing offer, a recent call_end)
/// are still in the stream to be read rather than drained while waiting for
/// a pong.
async fn connect_ws_raw(ts: &TestServer, token: &str) -> WsClient {
    let mut request = ts
        .ws_url
        .as_str()
        .into_client_request()
        .expect("building the ws request");
    request.headers_mut().insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}")).expect("header value"),
    );
    let (ws, _response) = tokio_tungstenite::connect_async(request)
        .await
        .expect("websocket upgrade succeeds");
    ws
}

async fn send_frame(ws: &mut WsClient, frame: Value) {
    ws.send(Message::text(frame.to_string()))
        .await
        .expect("sending a frame");
}

async fn next_frame_of_type(ws: &mut WsClient, wanted: &str) -> Value {
    next_frame_within(ws, wanted, FRAME_WAIT).await
}

async fn next_frame_within(ws: &mut WsClient, wanted: &str, window: Duration) -> Value {
    let deadline = tokio::time::Instant::now() + window;
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

async fn assert_no_frame_of_type(ws: &mut WsClient, unwanted: &str, window: Duration) {
    let deadline = tokio::time::Instant::now() + window;
    loop {
        match tokio::time::timeout_at(deadline, ws.next()).await {
            Err(_elapsed) => return,
            Ok(Some(Ok(Message::Text(text)))) => {
                let value: Value = serde_json::from_str(text.as_str()).expect("frames are JSON");
                assert_ne!(
                    value["type"], unwanted,
                    "unexpected {unwanted:?} frame: {value}"
                );
            }
            Ok(Some(Ok(_))) => {}
            Ok(Some(Err(err))) => panic!("socket errored: {err}"),
            Ok(None) => panic!("socket closed during the quiet window"),
        }
    }
}

/// Family of two, both registered; returns their tokens and ids.
async fn family_of_two(ts: &TestServer) -> (String, i64, String, i64) {
    let (owner, owner_id) = ts.register("owner", "Olive").await;
    let (member, member_id) = ts.register("junior", "Junior").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&member, &invite_code, "joined").await;
    (owner, owner_id, member, member_id)
}

/// Get-or-create the direct chat with `peer_id`; returns its id.
async fn direct_chat(ts: &TestServer, token: &str, peer_id: i64) -> i64 {
    let response = ts
        .post(token, "/chats/direct", json!({"user_id": peer_id}))
        .await;
    assert_eq!(response.status(), 200, "creating a direct chat");
    let body: Value = response.json().await.expect("direct chat JSON");
    body["chat"]["id"].as_i64().expect("chat id")
}

async fn register_device(ts: &TestServer, token: &str, platform: &str, push_token: &str) {
    let response = ts
        .post(
            token,
            "/devices",
            json!({"platform": platform, "push_token": push_token}),
        )
        .await;
    assert_eq!(response.status(), 201, "registering a {platform} device");
}

fn uuid() -> String {
    Uuid::new_v4().to_string()
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_full_call_rings_answers_relays_and_records_as_completed() {
    let ts = spawn_server().await;
    let (owner, owner_id, member, member_id) = family_of_two(&ts).await;
    let chat_id = direct_chat(&ts, &owner, member_id).await;

    let mut caller = connect_ws(&ts, &owner).await;
    let mut callee_a = connect_ws(&ts, &member).await;
    let mut callee_b = connect_ws(&ts, &member).await;

    let call_id = uuid();
    send_frame(
        &mut caller,
        json!({"type": "call_offer", "call_id": call_id, "chat_id": chat_id, "sdp": "v=0-offer"}),
    )
    .await;

    // The caller hears it ringing; both callee devices get the offer.
    let ringing = next_frame_of_type(&mut caller, "call_ringing").await;
    assert_eq!(ringing["call_id"], call_id.as_str());
    for callee in [&mut callee_a, &mut callee_b] {
        let offer = next_frame_of_type(callee, "call_offer").await;
        assert_eq!(offer["call_id"], call_id.as_str());
        assert_eq!(offer["chat_id"], chat_id);
        assert_eq!(offer["from_user_id"], owner_id);
        assert_eq!(offer["sdp"], "v=0-offer");
    }

    // Device A answers: the caller gets the answer, device B stops ringing.
    send_frame(
        &mut callee_a,
        json!({"type": "call_answer", "call_id": call_id, "sdp": "v=0-answer"}),
    )
    .await;
    let answer = next_frame_of_type(&mut caller, "call_answer").await;
    assert_eq!(answer["sdp"], "v=0-answer");
    let elsewhere = next_frame_of_type(&mut callee_b, "call_end").await;
    assert_eq!(elsewhere["reason"], "answered_elsewhere");

    // Candidates relay both ways.
    send_frame(
        &mut caller,
        json!({"type": "call_ice", "call_id": call_id,
               "candidate": {"candidate": "candidate:c", "sdp_mid": "0", "sdp_mline_index": 0}}),
    )
    .await;
    let ice = next_frame_of_type(&mut callee_a, "call_ice").await;
    assert_eq!(ice["candidate"]["candidate"], "candidate:c");
    send_frame(
        &mut callee_a,
        json!({"type": "call_ice", "call_id": call_id,
               "candidate": {"candidate": "candidate:d"}}),
    )
    .await;
    let ice = next_frame_of_type(&mut caller, "call_ice").await;
    assert_eq!(ice["candidate"]["candidate"], "candidate:d");
    assert!(
        ice["candidate"].get("sdp_mid").is_none(),
        "a candidate given no locator carries none"
    );

    // The caller hangs up: the answered device gets call_end, and a message
    // record with a completed outcome and a duration reaches everyone.
    send_frame(
        &mut caller,
        json!({"type": "call_end", "call_id": call_id, "reason": "hangup"}),
    )
    .await;
    let end = next_frame_of_type(&mut callee_a, "call_end").await;
    assert_eq!(end["reason"], "hangup");

    let record = next_frame_of_type(&mut callee_a, "message").await;
    assert_eq!(record["message"]["sender_id"], owner_id);
    assert_eq!(record["message"]["call"]["outcome"], "completed");
    assert!(
        record["message"]["call"]["duration_secs"].is_i64(),
        "a completed call carries a duration: {record}"
    );
    let message_id = record["message"]["id"].as_i64().expect("record id");

    // The chat list preview carries the call object, and the record is
    // unread for the callee.
    let chats: Value = ts
        .get(&member, "/chats")
        .await
        .json()
        .await
        .expect("chats JSON");
    let direct = chats["chats"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["chat"]["id"] == chat_id)
        .expect("the direct chat is listed");
    assert_eq!(direct["last_message"]["call"]["outcome"], "completed");
    assert_eq!(direct["unread_count"], 1);

    // The record cannot be edited.
    let patched = ts
        .patch(
            &member,
            &format!("/chats/{chat_id}/messages/{message_id}"),
            json!({"body": "not a call"}),
        )
        .await;
    common::assert_error(patched, 400, "validation").await;

    // No push was raised — both parties were connected throughout.
    assert!(ts.push.call_pushes().is_empty());
    assert!(ts.push.calls().is_empty());
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_decline_records_as_declined_and_never_pushes() {
    let ts = spawn_server().await;
    let (owner, _owner_id, member, member_id) = family_of_two(&ts).await;
    let chat_id = direct_chat(&ts, &owner, member_id).await;

    let mut caller = connect_ws(&ts, &owner).await;
    let mut callee = connect_ws(&ts, &member).await;

    let call_id = uuid();
    send_frame(
        &mut caller,
        json!({"type": "call_offer", "call_id": call_id, "chat_id": chat_id, "sdp": "v=0"}),
    )
    .await;
    next_frame_of_type(&mut caller, "call_ringing").await;
    next_frame_of_type(&mut callee, "call_offer").await;

    send_frame(
        &mut callee,
        json!({"type": "call_end", "call_id": call_id, "reason": "decline"}),
    )
    .await;
    let end = next_frame_of_type(&mut caller, "call_end").await;
    assert_eq!(end["reason"], "decline");
    let record = next_frame_of_type(&mut caller, "message").await;
    assert_eq!(record["message"]["call"]["outcome"], "declined");
    assert!(record["message"]["call"].get("duration_secs").is_none());
    assert!(ts.push.calls().is_empty(), "a declined call does not push");
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_cancel_to_an_offline_member_records_missed_and_pushes() {
    let ts = spawn_server().await;
    let (owner, _owner_id, member, member_id) = family_of_two(&ts).await;
    let chat_id = direct_chat(&ts, &owner, member_id).await;
    // The member is signed in on a device but has no socket.
    register_device(&ts, &member, "android", "android-token-cancel").await;

    let mut caller = connect_ws(&ts, &owner).await;
    let call_id = uuid();
    send_frame(
        &mut caller,
        json!({"type": "call_offer", "call_id": call_id, "chat_id": chat_id, "sdp": "v=0"}),
    )
    .await;
    next_frame_of_type(&mut caller, "call_ringing").await;

    // The offer rang the offline device — a call push was recorded.
    let deadline = tokio::time::Instant::now() + FRAME_WAIT;
    while ts.push.call_pushes().is_empty() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "no call push was raised"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let call_push = &ts.push.call_pushes()[0];
    assert_eq!(call_push.call.chat_id, chat_id);
    assert_eq!(call_push.call.caller_name, "Olive");

    // The caller gives up while it rings.
    send_frame(
        &mut caller,
        json!({"type": "call_end", "call_id": call_id, "reason": "cancel"}),
    )
    .await;
    let record = next_frame_of_type(&mut caller, "message").await;
    assert_eq!(record["message"]["call"]["outcome"], "missed");

    // A missed call pushes as the message it is, with its placeholder body.
    let deadline = tokio::time::Instant::now() + FRAME_WAIT;
    while ts.push.calls().is_empty() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the missed record never pushed"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(ts.push.calls()[0].note.body, "Missed voice call");
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_call_nobody_answers_rings_out_after_the_timeout() {
    let ts = spawn_server_with_config(|cfg| cfg.calls.ring_timeout_secs = 5).await;
    let (owner, _owner_id, member, member_id) = family_of_two(&ts).await;
    let chat_id = direct_chat(&ts, &owner, member_id).await;

    let mut caller = connect_ws(&ts, &owner).await;
    let mut callee = connect_ws(&ts, &member).await;
    let call_id = uuid();
    send_frame(
        &mut caller,
        json!({"type": "call_offer", "call_id": call_id, "chat_id": chat_id, "sdp": "v=0"}),
    )
    .await;
    next_frame_of_type(&mut caller, "call_ringing").await;
    next_frame_of_type(&mut callee, "call_offer").await;

    // Nobody answers. The sweeper ends it as a timeout within ~6 s.
    let end = next_frame_within(&mut caller, "call_end", Duration::from_secs(10)).await;
    assert_eq!(end["reason"], "timeout");
    let record = next_frame_of_type(&mut caller, "message").await;
    assert_eq!(record["message"]["call"]["outcome"], "missed");
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_person_on_a_call_is_busy_on_both_sides() {
    let ts = spawn_server().await;
    let (owner, owner_id) = ts.register("owner", "Olive").await;
    let (member, member_id) = ts.register("junior", "Junior").await;
    let (aunt, aunt_id) = ts.register("aunt", "Aunt").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&member, &invite_code, "joined").await;
    ts.join(&aunt, &invite_code, "joined").await;
    let _ = owner_id;

    let om = direct_chat(&ts, &owner, member_id).await;
    let am = direct_chat(&ts, &aunt, member_id).await;

    let mut caller = connect_ws(&ts, &owner).await;
    let mut callee = connect_ws(&ts, &member).await;
    let mut aunt_ws = connect_ws(&ts, &aunt).await;

    let first = uuid();
    send_frame(
        &mut caller,
        json!({"type": "call_offer", "call_id": first, "chat_id": om, "sdp": "v=0"}),
    )
    .await;
    next_frame_of_type(&mut caller, "call_ringing").await;
    next_frame_of_type(&mut callee, "call_offer").await;

    // The caller placing a second call is busy.
    send_frame(
        &mut caller,
        json!({"type": "call_offer", "call_id": uuid(), "chat_id": om, "sdp": "v=0"}),
    )
    .await;
    let err = next_frame_of_type(&mut caller, "error").await;
    assert_eq!(err["code"], "call_busy");

    // A third person calling the ringing callee gets peer_busy.
    send_frame(
        &mut aunt_ws,
        json!({"type": "call_offer", "call_id": uuid(), "chat_id": am, "sdp": "v=0"}),
    )
    .await;
    let err = next_frame_of_type(&mut aunt_ws, "error").await;
    assert_eq!(err["code"], "peer_busy");
    let _ = aunt_id;
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_call_in_the_family_chat_is_refused() {
    let ts = spawn_server().await;
    let (owner, _owner_id, _member, _member_id) = family_of_two(&ts).await;
    let family_chat = ts.family_chat_id(&owner).await;

    let mut caller = connect_ws(&ts, &owner).await;
    let call_id = uuid();
    send_frame(
        &mut caller,
        json!({"type": "call_offer", "call_id": call_id, "chat_id": family_chat, "sdp": "v=0"}),
    )
    .await;
    let err = next_frame_of_type(&mut caller, "error").await;
    assert_eq!(err["code"], "invalid_call");
    assert_eq!(err["call_id"], call_id.as_str());
    assert!(err.get("client_msg_id").is_none());
}

/// An offer past the SDP ceiling is refused on the call channel — and the
/// refusal leaves nothing behind: the caller is not busy, and the very next
/// offer rings.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn an_oversized_offer_is_refused_and_leaves_the_caller_free() {
    let ts = spawn_server().await;
    let (owner, _owner_id, member, member_id) = family_of_two(&ts).await;
    let chat_id = direct_chat(&ts, &owner, member_id).await;
    let mut caller = connect_ws(&ts, &owner).await;
    let mut callee = connect_ws(&ts, &member).await;

    let call_id = uuid();
    let huge = "v".repeat(64 * 1024 + 1);
    send_frame(
        &mut caller,
        json!({"type": "call_offer", "call_id": call_id, "chat_id": chat_id, "sdp": huge}),
    )
    .await;
    let err = next_frame_of_type(&mut caller, "error").await;
    assert_eq!(err["code"], "invalid_call");
    assert_eq!(err["call_id"], call_id.as_str());
    assert_no_frame_of_type(&mut callee, "call_offer", Duration::from_millis(300)).await;

    // Nothing was begun, so nothing is busy: a real offer rings at once.
    let call_id = uuid();
    send_frame(
        &mut caller,
        json!({"type": "call_offer", "call_id": call_id, "chat_id": chat_id, "sdp": "v=0"}),
    )
    .await;
    next_frame_of_type(&mut caller, "call_ringing").await;
    let offer = next_frame_of_type(&mut callee, "call_offer").await;
    assert_eq!(offer["call_id"], call_id.as_str());
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_late_callee_connection_is_replayed_the_offer_and_the_candidates() {
    let ts = spawn_server().await;
    let (owner, _owner_id, member, member_id) = family_of_two(&ts).await;
    let chat_id = direct_chat(&ts, &owner, member_id).await;
    register_device(&ts, &member, "android", "android-token-late").await;

    let mut caller = connect_ws(&ts, &owner).await;
    let call_id = uuid();
    send_frame(
        &mut caller,
        json!({"type": "call_offer", "call_id": call_id, "chat_id": chat_id, "sdp": "v=0-late"}),
    )
    .await;
    next_frame_of_type(&mut caller, "call_ringing").await;
    // The caller trickles a candidate while the callee is still asleep.
    send_frame(
        &mut caller,
        json!({"type": "call_ice", "call_id": call_id,
               "candidate": {"candidate": "candidate:buffered", "sdp_mid": "0"}}),
    )
    .await;
    // Give the buffer a moment to record it.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Now the callee connects: it is replayed the offer, then the candidate.
    let mut callee = connect_ws_raw(&ts, &member).await;
    let offer = next_frame_of_type(&mut callee, "call_offer").await;
    assert_eq!(offer["call_id"], call_id.as_str());
    assert_eq!(offer["sdp"], "v=0-late");
    let ice = next_frame_of_type(&mut callee, "call_ice").await;
    assert_eq!(ice["candidate"]["candidate"], "candidate:buffered");
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_recently_ended_call_is_replayed_to_a_callee_connecting_after() {
    let ts = spawn_server().await;
    let (owner, _owner_id, member, member_id) = family_of_two(&ts).await;
    let chat_id = direct_chat(&ts, &owner, member_id).await;
    register_device(&ts, &member, "android", "android-token-recent").await;

    let mut caller = connect_ws(&ts, &owner).await;
    let call_id = uuid();
    send_frame(
        &mut caller,
        json!({"type": "call_offer", "call_id": call_id, "chat_id": chat_id, "sdp": "v=0"}),
    )
    .await;
    next_frame_of_type(&mut caller, "call_ringing").await;
    send_frame(
        &mut caller,
        json!({"type": "call_end", "call_id": call_id, "reason": "cancel"}),
    )
    .await;
    next_frame_of_type(&mut caller, "message").await;

    // The callee, woken and connecting after the call already ended, is told
    // it ended so its phone stops ringing.
    let mut callee = connect_ws_raw(&ts, &member).await;
    let end = next_frame_of_type(&mut callee, "call_end").await;
    assert_eq!(end["call_id"], call_id.as_str());
    assert_eq!(end["reason"], "cancel");
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn the_caller_vanishing_while_ringing_cancels_the_call() {
    let ts = spawn_server().await;
    let (owner, _owner_id, member, member_id) = family_of_two(&ts).await;
    let chat_id = direct_chat(&ts, &owner, member_id).await;

    let mut caller = connect_ws(&ts, &owner).await;
    let mut callee = connect_ws(&ts, &member).await;
    let call_id = uuid();
    send_frame(
        &mut caller,
        json!({"type": "call_offer", "call_id": call_id, "chat_id": chat_id, "sdp": "v=0"}),
    )
    .await;
    next_frame_of_type(&mut caller, "call_ringing").await;
    next_frame_of_type(&mut callee, "call_offer").await;

    // The caller's app dies while it rings.
    drop(caller);
    let end = next_frame_of_type(&mut callee, "call_end").await;
    assert_eq!(end["reason"], "cancel");
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn an_offer_to_a_wholly_unreachable_member_is_peer_unreachable() {
    let ts = spawn_server().await;
    let (owner, _owner_id, _member, member_id) = family_of_two(&ts).await;
    let chat_id = direct_chat(&ts, &owner, member_id).await;

    let mut caller = connect_ws(&ts, &owner).await;
    let call_id = uuid();
    send_frame(
        &mut caller,
        json!({"type": "call_offer", "call_id": call_id, "chat_id": chat_id, "sdp": "v=0"}),
    )
    .await;
    let err = next_frame_of_type(&mut caller, "error").await;
    assert_eq!(err["code"], "peer_unreachable");
    assert_eq!(err["call_id"], call_id.as_str());
    // A missed record was still written — the callee was nowhere to be
    // reached, which is a missed call.
    let record = next_frame_of_type(&mut caller, "message").await;
    assert_eq!(record["message"]["call"]["outcome"], "missed");
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_member_with_a_call_token_is_reachable() {
    let ts = spawn_server().await;
    let (owner, _owner_id, member, member_id) = family_of_two(&ts).await;
    let chat_id = direct_chat(&ts, &owner, member_id).await;
    register_device(&ts, &member, "android", "android-token-reachable").await;

    let mut caller = connect_ws(&ts, &owner).await;
    let call_id = uuid();
    send_frame(
        &mut caller,
        json!({"type": "call_offer", "call_id": call_id, "chat_id": chat_id, "sdp": "v=0"}),
    )
    .await;
    // It rings rather than failing, and the offline device gets a push.
    next_frame_of_type(&mut caller, "call_ringing").await;
    assert_no_frame_of_type(&mut caller, "error", Duration::from_millis(500)).await;
    let deadline = tokio::time::Instant::now() + FRAME_WAIT;
    while ts.push.call_pushes().is_empty() {
        assert!(tokio::time::Instant::now() < deadline, "no call push");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_voip_token_is_set_cleared_and_left_alone_by_absence() {
    let ts = spawn_server().await;
    let (token, user_id) = ts.register("callee", "Callee").await;

    async fn voip(ts: &TestServer, user_id: i64) -> Option<String> {
        sqlx::query_scalar("SELECT voip_token FROM devices WHERE user_id = $1")
            .bind(user_id)
            .fetch_one(&ts.state.pool)
            .await
            .expect("device row")
    }

    // Set both tokens.
    ts.post(
        &token,
        "/devices",
        json!({"platform": "ios", "push_token": "apns-1", "voip_token": "voip-1"}),
    )
    .await;
    assert_eq!(voip(&ts, user_id).await.as_deref(), Some("voip-1"));

    // A launch carrying only the alert token leaves the VoIP token alone.
    ts.post(
        &token,
        "/devices",
        json!({"platform": "ios", "push_token": "apns-1"}),
    )
    .await;
    assert_eq!(
        voip(&ts, user_id).await.as_deref(),
        Some("voip-1"),
        "an absent voip_token must not wipe the stored one"
    );

    // A null clears it.
    ts.post(
        &token,
        "/devices",
        json!({"platform": "ios", "push_token": "apns-1", "voip_token": Value::Null}),
    )
    .await;
    assert_eq!(voip(&ts, user_id).await, None);
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn me_reports_calls_enabled_and_ice_serves_stun() {
    let ts = spawn_server().await;
    let (token, _id) = ts.register("solo", "Solo").await;

    let me: Value = ts.get(&token, "/me").await.json().await.expect("me JSON");
    assert_eq!(me["calls_enabled"], true);

    let ice: Value = ts
        .get(&token, "/calls/ice")
        .await
        .json()
        .await
        .expect("ice JSON");
    assert_eq!(ice["ttl_secs"], 86400);
    let servers = ice["ice_servers"].as_array().expect("ice_servers");
    assert!(servers.iter().any(|s| {
        s["urls"][0]
            .as_str()
            .is_some_and(|u| u.starts_with("stun:"))
    }));
    // No TURN configured by default → no credentials anywhere.
    assert!(servers.iter().all(|s| s.get("credential").is_none()));
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn ice_mints_time_limited_turn_credentials_from_a_secret() {
    let ts = spawn_server_with_config(|cfg| {
        cfg.calls.turn_urls = vec!["turn:turn.example.com:3478".to_string()];
        cfg.calls.turn_secret = "s3cr3t".to_string();
    })
    .await;
    let (token, user_id) = ts.register("solo", "Solo").await;

    let ice: Value = ts
        .get(&token, "/calls/ice")
        .await
        .json()
        .await
        .expect("ice JSON");
    let turn = ice["ice_servers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| {
            s["urls"][0]
                .as_str()
                .is_some_and(|u| u.starts_with("turn:"))
        })
        .expect("a turn server");
    let username = turn["username"].as_str().expect("a minted username");
    // username is "<expiry>:<user_id>".
    assert!(
        username.ends_with(&format!(":{user_id}")),
        "username was {username}"
    );
    assert!(turn["credential"].as_str().is_some_and(|c| !c.is_empty()));
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn calls_disabled_refuses_ice_and_offers() {
    let ts = spawn_server_with_config(|cfg| cfg.calls.enabled = false).await;
    let (owner, _owner_id, _member, member_id) = family_of_two(&ts).await;
    let chat_id = direct_chat(&ts, &owner, member_id).await;

    let me: Value = ts.get(&owner, "/me").await.json().await.expect("me JSON");
    assert_eq!(me["calls_enabled"], false);

    common::assert_error(ts.get(&owner, "/calls/ice").await, 403, "calls_disabled").await;

    let mut caller = connect_ws(&ts, &owner).await;
    let call_id = uuid();
    send_frame(
        &mut caller,
        json!({"type": "call_offer", "call_id": call_id, "chat_id": chat_id, "sdp": "v=0"}),
    )
    .await;
    let err = next_frame_of_type(&mut caller, "error").await;
    assert_eq!(err["code"], "calls_disabled");
    assert_eq!(err["call_id"], call_id.as_str());
}
