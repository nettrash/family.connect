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

/// `typing` was the one inbound frame with no membership check: any
/// authenticated account — including one in no family at all — could name
/// any chat id in the database and make that family's chat show them
/// typing. The relay is now gated, and the refusal is SILENT on purpose:
/// an `error` frame here would spam a client whose membership lapsed
/// mid-connection with one error per keystroke, and answering only for
/// chats that exist would turn the indicator into a way to enumerate ids.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_typing_frame_for_a_foreign_chat_is_dropped_in_silence() {
    let ts = spawn_server().await;
    let (_owner, _owner_id, member, _member_id, _code) = family_of_two(&ts).await;
    let chat_id = ts.family_chat_id(&member).await;

    // Somebody who belongs to no family at all — the weakest possible
    // caller who still holds a valid session.
    let (stranger, _) = ts.register("stranger", "Stranger").await;
    let mut stranger_ws = connect_ws(&ts, &stranger).await;
    let mut member_ws = connect_ws(&ts, &member).await;

    send_frame(
        &mut stranger_ws,
        json!({"type": "typing", "chat_id": chat_id}),
    )
    .await;

    // Nothing reaches the family...
    assert_no_frame_of_type(&mut member_ws, "typing", Duration::from_millis(800)).await;
    // ...and nothing comes back either, so nothing is confirmed about the id.
    assert_no_frame_of_type(&mut stranger_ws, "error", Duration::from_millis(400)).await;

    // The gate is membership, not the frame: a real member still relays.
    send_frame(
        &mut member_ws,
        json!({"type": "typing", "chat_id": chat_id}),
    )
    .await;
    let mut owner_ws = connect_ws(&ts, &_owner).await;
    // The member's throttle has already burned this window, so type from a
    // fresh connection to prove the path is alive.
    let mut second = connect_ws(&ts, &member).await;
    send_frame(&mut second, json!({"type": "typing", "chat_id": chat_id})).await;
    let typing = next_frame_of_type(&mut owner_ws, "typing").await;
    assert_eq!(typing["chat_id"].as_i64(), Some(chat_id));
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
    assert_eq!(call.note.body, "anyone home?");
    assert_eq!(
        call.note.title, "The Smiths — Olive",
        "family chat pushes are titled '<Family> — <Sender>'"
    );
    assert_eq!(
        call.note.badge, 1,
        "one unread message for the offline member"
    );
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

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_reaction_fans_out_full_state_to_every_member_connection_and_never_pushes() {
    let ts = spawn_server().await;
    let (owner, _, member, member_id, _) = family_of_two(&ts).await;
    let chat_id = ts.family_chat_id(&owner).await;
    let response = ts
        .post_message(&owner, chat_id, &Uuid::new_v4().to_string(), "react to me")
        .await;
    assert_eq!(response.status(), 201);
    let body: Value = response.json().await.expect("message response is JSON");
    let message_id = body["message"]["id"].as_i64().expect("message id");
    let reaction_path = format!("/chats/{chat_id}/messages/{message_id}/reaction");

    // Give the owner a push token so a wrongly-pushing implementation would
    // be caught by the quiet check at the bottom.
    let response = ts
        .post(
            &owner,
            "/devices",
            json!({"platform": "ios", "push_token": "tok-owner"}),
        )
        .await;
    assert_eq!(response.status(), 201);

    let mut owner_ws = connect_ws(&ts, &owner).await;
    let mut owner_ws2 = connect_ws(&ts, &owner).await;
    // The actor's *other* device: the acting request is answered over HTTP,
    // but this connection must still hear the frame.
    let mut member_ws = connect_ws(&ts, &member).await;

    let response = ts
        .put(&member, &reaction_path, json!({"emoji": "❤️"}))
        .await;
    assert_eq!(response.status(), 200);

    for ws in [&mut owner_ws, &mut owner_ws2, &mut member_ws] {
        let frame = next_frame_of_type(ws, "reaction").await;
        assert_eq!(frame["chat_id"], chat_id);
        assert_eq!(frame["message_id"], message_id);
        assert_eq!(
            frame["reactions"],
            json!([{"user_id": member_id, "emoji": "❤️"}])
        );
        assert!(frame["reaction_seq"].as_i64().expect("seq") > 0);
    }

    // A no-op re-PUT changes nothing and fans nothing out.
    let response = ts
        .put(&member, &reaction_path, json!({"emoji": "❤️"}))
        .await;
    assert_eq!(response.status(), 200);
    assert_no_frame_of_type(&mut owner_ws, "reaction", Duration::from_millis(1200)).await;

    // Reactions never push: take the owner (who has a registered push
    // token) fully offline and react again. The member's live socket
    // proves delivery happened; the push log must stay empty (push
    // dispatch is spawned, hence the grace sleep).
    drop(owner_ws);
    drop(owner_ws2);
    let response = ts
        .put(&member, &reaction_path, json!({"emoji": "👍"}))
        .await;
    assert_eq!(response.status(), 200);
    let frame = next_frame_of_type(&mut member_ws, "reaction").await;
    assert_eq!(
        frame["reactions"],
        json!([{"user_id": member_id, "emoji": "👍"}])
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        ts.push.calls().is_empty(),
        "reactions must never reach the push seam"
    );
}

/// A poll is created over the socket like any other message, and every
/// change to it afterwards reaches every member connection — the voter's own
/// devices included — as a `poll` frame that never wakes anybody.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_poll_fans_out_full_state_to_every_member_connection_and_never_pushes() {
    let ts = spawn_server().await;
    let (owner, _, member, member_id, _) = family_of_two(&ts).await;
    let chat_id = ts.family_chat_id(&owner).await;

    // Give the owner a push token so a wrongly-pushing implementation would
    // be caught by the quiet check at the bottom.
    let response = ts
        .post(
            &owner,
            "/devices",
            json!({"platform": "ios", "push_token": "tok-owner"}),
        )
        .await;
    assert_eq!(response.status(), 201);

    let mut owner_ws = connect_ws(&ts, &owner).await;
    let mut member_ws = connect_ws(&ts, &member).await;

    // The poll rides the ordinary `send` frame, and the ack carries it.
    send_frame(
        &mut owner_ws,
        json!({
            "type": "send",
            "chat_id": chat_id,
            "client_msg_id": Uuid::new_v4().to_string(),
            "body": "Pizza or pasta?",
            "poll": {"options": ["Pizza", "Pasta"]},
        }),
    )
    .await;
    let ack = next_frame_of_type(&mut owner_ws, "ack").await;
    let message_id = ack["message"]["id"].as_i64().expect("message id");
    let options = ack["message"]["poll"]["options"]
        .as_array()
        .expect("the ack carries the poll")
        .clone();
    assert_eq!(options.len(), 2);
    let pizza = options[0]["id"].as_i64().expect("option id");

    // ...and it arrives at the other member as an ordinary `message`.
    let delivered = next_frame_of_type(&mut member_ws, "message").await;
    assert_eq!(delivered["message"]["id"].as_i64(), Some(message_id));
    assert_eq!(delivered["message"]["poll"]["closed"], json!(false));

    // A vote reaches BOTH sockets: the voter's own request is answered over
    // HTTP, but the author's connection learns of it only from the frame.
    let response = ts
        .put(
            &member,
            &format!("/chats/{chat_id}/messages/{message_id}/vote"),
            json!({"option_id": pizza}),
        )
        .await;
    assert_eq!(response.status(), 200);
    for ws in [&mut owner_ws, &mut member_ws] {
        let frame = next_frame_of_type(ws, "poll").await;
        assert_eq!(frame["chat_id"], chat_id);
        assert_eq!(frame["message_id"], message_id);
        assert_eq!(frame["poll"]["options"][0]["votes"], json!([member_id]));
        assert_eq!(frame["poll"]["options"][1]["votes"], json!([]));
        assert!(frame["poll"]["poll_seq"].as_i64().expect("seq") > 0);
    }

    // A no-op re-PUT changes nothing and fans nothing out.
    let response = ts
        .put(
            &member,
            &format!("/chats/{chat_id}/messages/{message_id}/vote"),
            json!({"option_id": pizza}),
        )
        .await;
    assert_eq!(response.status(), 200);
    assert_no_frame_of_type(&mut owner_ws, "poll", Duration::from_millis(1200)).await;

    // Votes never push: take the owner (who has a registered push token)
    // fully offline and close the poll. The member's live socket proves
    // delivery happened; the push log must stay empty.
    drop(owner_ws);
    let response = ts
        .post(
            &owner,
            &format!("/chats/{chat_id}/messages/{message_id}/poll/close"),
            json!({}),
        )
        .await;
    assert_eq!(response.status(), 200);
    let frame = next_frame_of_type(&mut member_ws, "poll").await;
    assert_eq!(frame["poll"]["closed"], json!(true));
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        ts.push.calls().is_empty(),
        "poll changes must never reach the push seam"
    );
}

/// Deleting an account tells the family three things at once — the roster
/// change, the tombstone to overwrite the stored name with, and the new
/// owner — and signs every device of the departing account out
/// (protocol.md, "Deleting an account").
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn deleting_an_account_fans_out_the_tombstone_and_closes_its_sockets() {
    let ts = spawn_server().await;
    let (owner, owner_id, member, member_id, _) = family_of_two(&ts).await;
    let mut owner_ws = connect_ws(&ts, &owner).await;
    let mut member_ws = connect_ws(&ts, &member).await;

    // The OWNER deletes, so ownership passes to the only other member.
    let response = ts
        .post(&owner, "/me/delete", json!({"password": "password123"}))
        .await;
    assert_eq!(response.status(), 204);

    // `member_left` first, so a client that predates the tombstone frame
    // still fixes its roster.
    let left = next_frame_of_type(&mut member_ws, "member_left").await;
    assert_eq!(left["user_id"], owner_id);

    // The tombstone carries the WHOLE member, because that is exactly what
    // a client has to overwrite.
    let deleted = next_frame_of_type(&mut member_ws, "member_deleted").await;
    assert_eq!(deleted["member"]["id"], owner_id);
    assert_eq!(deleted["member"]["deleted"], json!(true));
    assert_eq!(deleted["member"]["display_name"], "Deleted account");
    assert_eq!(deleted["member"]["username"], format!("deleted-{owner_id}"));
    assert_eq!(deleted["member"]["avatar_version"], json!(0));
    assert!(deleted["member"]["role"].is_null());

    // And the successor learns they own the family without asking.
    let new_owner = next_frame_of_type(&mut member_ws, "family_owner").await;
    assert_eq!(new_owner["user_id"], member_id);
    assert_eq!(new_owner["family_id"], left["family_id"]);

    // The deleted account's own socket is closed with the session-gone code
    // — its sessions are gone, so there is nothing left to reconnect with.
    let deadline = tokio::time::Instant::now() + FRAME_WAIT;
    loop {
        match tokio::time::timeout_at(deadline, owner_ws.next())
            .await
            .expect("the socket must close once the account is deleted")
        {
            Some(Ok(Message::Close(frame))) => {
                let frame = frame.expect("close frame carries a code");
                assert_eq!(u16::from(frame.code), 4401);
                break;
            }
            Some(Ok(_)) => continue,
            None => break,
            Some(Err(err)) => panic!("expected a clean close, got: {err}"),
        }
    }

    // None of it pushes: a deletion is not mail.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        ts.push.calls().is_empty(),
        "account deletion must never reach the push seam"
    );
}

/// **THE SILENCE PROPERTY.** A block reaches every connection of the
/// BLOCKER and nobody else — above all not the person blocked, for whom
/// nothing whatsoever changes (protocol.md, "Blocking a member").
///
/// This is the test the whole feature rests on. The one-element recipient
/// list in `events::deliver_member_blocked` is the entire mechanism, and a
/// later "helpful" widening of it would be silent in production and loud
/// only here.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_block_reaches_only_the_blockers_own_devices() {
    let ts = spawn_server().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (member, member_id) = ts.register("junior", "Junior").await;
    let (third, _) = ts.register("cousin", "Cousin").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&member, &invite_code, "joined").await;
    ts.join(&third, &invite_code, "joined").await;

    // The blocker on two devices, the blocked member, and a bystander.
    let mut owner_phone = connect_ws(&ts, &owner).await;
    let mut owner_mac = connect_ws(&ts, &owner).await;
    let mut blocked_ws = connect_ws(&ts, &member).await;
    let mut bystander_ws = connect_ws(&ts, &third).await;

    let blocked = ts
        .put(
            &owner,
            &format!("/families/members/{member_id}/block"),
            json!({}),
        )
        .await;
    assert_eq!(blocked.status(), 204);

    // BOTH of the blocker's devices learn it, so a block set on the phone
    // is in force on the Mac without a resync.
    for ws in [&mut owner_phone, &mut owner_mac] {
        let frame = next_frame_of_type(ws, "member_blocked").await;
        assert_eq!(frame["user_id"], member_id);
        assert_eq!(frame["blocked"], true);
        assert!(
            frame.get("family_id").is_none(),
            "a block is a pair, not a membership: {frame}"
        );
    }

    // NOBODY ELSE. Not the person blocked — that is the whole feature —
    // and not a bystander either.
    assert_no_frame_of_type(
        &mut blocked_ws,
        "member_blocked",
        Duration::from_millis(400),
    )
    .await;
    assert_no_frame_of_type(
        &mut bystander_ws,
        "member_blocked",
        Duration::from_millis(400),
    )
    .await;

    // An unblock is the SAME frame with `false`, and is just as private.
    let cleared = ts
        .delete(&owner, &format!("/families/members/{member_id}/block"))
        .await;
    assert_eq!(cleared.status(), 204);
    let frame = next_frame_of_type(&mut owner_phone, "member_blocked").await;
    assert_eq!(frame["blocked"], false, "state, not an event: {frame}");
    assert_no_frame_of_type(
        &mut blocked_ws,
        "member_blocked",
        Duration::from_millis(400),
    )
    .await;
}

/// **THE TRANSPOSED-FILTER TEST.** The read/typing suppression is INWARD
/// ONLY: a frame FROM somebody is not relayed TO anybody who blocked them.
/// Written the other way round — the same one-line filter with its two
/// arguments swapped — the BLOCKER's own reads and typing would stop
/// reaching the person they blocked, and one member going quiet to exactly
/// one other is a signal you can test for.
///
/// So this asserts the POSITIVE direction as loudly as the negative one:
/// after A blocks B, A's frames still reach B byte-identically.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn read_and_typing_are_suppressed_inward_only() {
    let ts = spawn_server().await;
    let (blocker, _) = ts.register("owner", "Olive").await;
    let (blocked, blocked_id) = ts.register("junior", "Junior").await;
    // A THIRD member, whose socket is what distinguishes "drop the people
    // who blocked this sender" from "drop everybody". In a two-person
    // family the surviving recipient list is empty either way.
    let (bystander, _) = ts.register("cousin", "Cousin").await;
    let (_, invite_code) = ts.create_family(&blocker, "The Smiths").await;
    ts.set_open_policy(&blocker).await;
    ts.join(&blocked, &invite_code, "joined").await;
    ts.join(&bystander, &invite_code, "joined").await;
    let chat_id = ts.family_chat_id(&blocker).await;

    ts.put(
        &blocker,
        &format!("/families/members/{blocked_id}/block"),
        json!({}),
    )
    .await;

    let mut blocker_ws = connect_ws(&ts, &blocker).await;
    let mut blocked_ws = connect_ws(&ts, &blocked).await;
    let mut bystander_ws = connect_ws(&ts, &bystander).await;

    // Seed a message so there is something to read.
    let seeded: Value = ts
        .post(
            &blocker,
            &format!("/chats/{chat_id}/messages"),
            json!({"client_msg_id": Uuid::new_v4().to_string(), "body": "hello"}),
        )
        .await
        .json()
        .await
        .expect("message");
    let seeded_id = seeded["message"]["id"].as_i64().expect("id");
    let _ = next_frame_of_type(&mut blocked_ws, "message").await;

    // INWARD: the blocked member reads and types; neither reaches the
    // blocker.
    ts.post(
        &blocked,
        &format!("/chats/{chat_id}/read"),
        json!({"last_read_message_id": seeded_id}),
    )
    .await;
    blocked_ws
        .send(Message::Text(
            json!({"type": "typing", "chat_id": chat_id})
                .to_string()
                .into(),
        ))
        .await
        .expect("send typing");
    assert_no_frame_of_type(&mut blocker_ws, "read", Duration::from_millis(400)).await;
    assert_no_frame_of_type(&mut blocker_ws, "typing", Duration::from_millis(400)).await;
    // ...but the BYSTANDER, who blocked nobody, receives both. This is what
    // separates a filter that drops the blockers from one that drops the
    // whole recipient list.
    let seen = next_frame_of_type(&mut bystander_ws, "read").await;
    assert_eq!(
        seen["user_id"], blocked_id,
        "relayed to everyone else: {seen}"
    );
    let typing_seen = next_frame_of_type(&mut bystander_ws, "typing").await;
    assert_eq!(typing_seen["user_id"], blocked_id);

    // OUTWARD: the blocker reads and types, and BOTH still reach the person
    // they blocked, unchanged. This is the half a transposed filter breaks.
    ts.post(
        &blocker,
        &format!("/chats/{chat_id}/read"),
        json!({"last_read_message_id": seeded_id}),
    )
    .await;
    let read = next_frame_of_type(&mut blocked_ws, "read").await;
    assert_eq!(read["chat_id"], chat_id);
    assert_eq!(read["last_read_message_id"], seeded_id);

    blocker_ws
        .send(Message::Text(
            json!({"type": "typing", "chat_id": chat_id})
                .to_string()
                .into(),
        ))
        .await
        .expect("send typing");
    let typing = next_frame_of_type(&mut blocked_ws, "typing").await;
    assert_eq!(
        typing["chat_id"], chat_id,
        "the blocker's own typing must still reach the blocked member"
    );
}

/// The FRAME is not suppressed, only the push: a blocked member's message
/// still arrives over the socket, because the hidden row has to have
/// something to reveal and the read cursor only learns ids the server hands
/// it.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_blocked_members_message_still_arrives_over_the_socket() {
    let ts = spawn_server().await;
    let (blocker, _) = ts.register("owner", "Olive").await;
    let (blocked, blocked_id) = ts.register("junior", "Junior").await;
    let (_, invite_code) = ts.create_family(&blocker, "The Smiths").await;
    ts.set_open_policy(&blocker).await;
    ts.join(&blocked, &invite_code, "joined").await;
    let chat_id = ts.family_chat_id(&blocker).await;
    ts.put(
        &blocker,
        &format!("/families/members/{blocked_id}/block"),
        json!({}),
    )
    .await;

    let mut blocker_ws = connect_ws(&ts, &blocker).await;
    ts.post(
        &blocked,
        &format!("/chats/{chat_id}/messages"),
        json!({"client_msg_id": Uuid::new_v4().to_string(), "body": "still delivered"}),
    )
    .await;

    let frame = next_frame_of_type(&mut blocker_ws, "message").await;
    assert_eq!(
        frame["message"]["body"], "still delivered",
        "history is never filtered — a short page reads as the end of the feed, \
         and a frozen read marker is an oracle: {frame}"
    );
}

/// An owner's leave tells the family who owns it now — and the ORDER is
/// load-bearing: `family_owner` before `member_left`, so no client ever
/// momentarily holds a family whose `owner_user_id` names nobody in
/// `members`.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn an_owners_leave_tells_the_family_who_owns_it_now() {
    let ts = spawn_server().await;
    let (owner, owner_id) = ts.register("owner", "Olive").await;
    let (first, first_id) = ts.register("junior", "Junior").await;
    let (second, _) = ts.register("cousin", "Cousin").await;
    let (family_id, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&first, &invite_code, "joined").await;
    ts.join(&second, &invite_code, "joined").await;

    let mut successor_ws = connect_ws(&ts, &first).await;
    let mut other_ws = connect_ws(&ts, &second).await;

    let left = ts.post(&owner, "/families/leave", json!({})).await;
    assert_eq!(left.status(), 200);

    // Read in ARRIVAL order on the successor's socket: the first of the two
    // frames must be family_owner.
    let mut seen: Vec<String> = Vec::new();
    for _ in 0..2 {
        let deadline = tokio::time::Instant::now() + FRAME_WAIT;
        loop {
            let message = tokio::time::timeout_at(deadline, successor_ws.next())
                .await
                .expect("timed out waiting for a membership frame")
                .expect("socket closed")
                .expect("socket errored");
            if let Message::Text(text) = message {
                let value: Value = serde_json::from_str(text.as_str()).expect("JSON");
                let kind = value["type"].as_str().unwrap_or_default().to_string();
                if kind == "family_owner" {
                    assert_eq!(value["user_id"], first_id, "longest-standing inherits");
                    assert_eq!(value["family_id"], family_id);
                }
                if kind == "family_owner" || kind == "member_left" {
                    seen.push(kind);
                    break;
                }
            }
        }
    }
    assert_eq!(
        seen,
        vec!["family_owner".to_string(), "member_left".to_string()],
        "ownership must land BEFORE the departure, or a client holds a family \
         owned by somebody who is not in it"
    );

    // The other member is told the owner left too.
    let left_frame = next_frame_of_type(&mut other_ws, "member_left").await;
    assert_eq!(left_frame["user_id"], owner_id);
}
