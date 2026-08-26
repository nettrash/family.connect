//! Integration: `POST /me/delete` (docs/protocol.md, "Deleting an account").
//!
//! One asymmetry is under test in every case here: **the person is erased,
//! the words stay.** The username, the password, the picture, the birthday,
//! the sessions, the devices and the direct chats go; the messages, the
//! board notes and the reactions stay, still attributed to a row that now
//! reads "Deleted account" and can never be signed into again.
//!
//! The live-socket helpers are copied from `ws_flow.rs` rather than shared:
//! `common/` is the REST harness, and a second copy of forty lines is a
//! cheaper price than a helper module every test binary has to compile.

mod common;

use std::time::Duration;

use common::{TestServer, assert_error, spawn_server, spawn_server_with_config};
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

/// Wait for the server to close a socket, and answer with the close code it
/// sent. Some client stacks surface a close as plain end-of-stream, which is
/// the `None` arm — an end without a code is still an end.
async fn close_code(ws: &mut WsClient) -> Option<u16> {
    let deadline = tokio::time::Instant::now() + FRAME_WAIT;
    loop {
        match tokio::time::timeout_at(deadline, ws.next())
            .await
            .expect("the socket must close")
        {
            Some(Ok(Message::Close(frame))) => {
                return frame.map(|frame| u16::from(frame.code));
            }
            Some(Ok(_)) => continue,
            None => return None,
            Some(Err(err)) => panic!("expected a clean close, got: {err}"),
        }
    }
}

/// Four bytes of JPEG magic and then filler, so the content sniffer accepts
/// it. `fill` is what makes two of these differ in CONTENT rather than in
/// length — the dedup key is a hash of the bytes.
fn jpeg_bytes(len: usize, fill: u8) -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE0];
    bytes.resize(len.max(4), fill);
    bytes
}

/// Owner plus one member; returns `(owner, owner_id, member, member_id,
/// family_chat_id, invite_code)`.
async fn family_of_two(ts: &TestServer) -> (String, i64, String, i64, i64, String) {
    let (owner, owner_id) = ts.register("owner", "Olive").await;
    let (member, member_id) = ts.register("junior", "Junior").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&member, &invite_code, "joined").await;
    let chat_id = ts.family_chat_id(&owner).await;
    (owner, owner_id, member, member_id, chat_id, invite_code)
}

async fn delete_account(ts: &TestServer, token: &str, password: &str) -> reqwest::Response {
    ts.post(token, "/me/delete", json!({"password": password}))
        .await
}

/// Delete an account and insist it worked — the precondition of nearly
/// every assertion in this file.
async fn delete_ok(ts: &TestServer, token: &str) {
    let response = delete_account(ts, token, "password123").await;
    assert_eq!(response.status(), 204, "deleting an account");
}

async fn send(ts: &TestServer, token: &str, chat_id: i64, body: &str) -> i64 {
    let response = ts
        .post(
            token,
            &format!("/chats/{chat_id}/messages"),
            json!({"client_msg_id": Uuid::new_v4().to_string(), "body": body}),
        )
        .await;
    assert_eq!(response.status(), 201, "posting a message");
    let value: Value = response.json().await.expect("message response is JSON");
    value["message"]["id"].as_i64().expect("message id")
}

/// Upload bytes and answer with the attachment id.
async fn upload(ts: &TestServer, token: &str, bytes: Vec<u8>) -> i64 {
    let response = ts
        .put_bytes_method(
            "POST",
            token,
            "/attachments?kind=photo",
            "image/jpeg",
            bytes,
        )
        .await;
    assert_eq!(response.status(), 201, "uploading an attachment");
    let value: Value = response.json().await.expect("attachment response is JSON");
    value["attachment"]["id"].as_i64().expect("attachment id")
}

/// Send a message that claims an upload, which is what turns an attachment
/// row from "pending" into part of the shared record.
async fn send_attachment(ts: &TestServer, token: &str, chat_id: i64, attachment_id: i64) {
    let response = ts
        .post(
            token,
            &format!("/chats/{chat_id}/messages"),
            json!({
                "client_msg_id": Uuid::new_v4().to_string(),
                "body": "",
                "attachment_id": attachment_id,
            }),
        )
        .await;
    assert_eq!(response.status(), 201, "sending an attachment");
}

/// Cast a vote, insisting it landed.
async fn vote(ts: &TestServer, token: &str, chat_id: i64, message_id: i64, option_id: i64) {
    let response = ts
        .put(
            token,
            &format!("/chats/{chat_id}/messages/{message_id}/vote"),
            json!({"option_id": option_id}),
        )
        .await;
    assert_eq!(response.status(), 200, "voting");
}

/// The file an attachment row names.
async fn storage_key(ts: &TestServer, attachment_id: i64) -> String {
    sqlx::query_scalar("SELECT storage_key FROM attachments WHERE id = $1")
        .bind(attachment_id)
        .fetch_one(&ts.state.pool)
        .await
        .expect("the attachment row names a file")
}

async fn my_family(ts: &TestServer, token: &str) -> Value {
    let response = ts.get(token, "/families/mine").await;
    assert_eq!(response.status(), 200, "reading the family");
    response.json().await.expect("family response is JSON")
}

async fn chats_of(ts: &TestServer, token: &str) -> Vec<Value> {
    let response = ts.get(token, "/chats").await;
    assert_eq!(response.status(), 200, "listing chats");
    let body: Value = response.json().await.expect("chats response is JSON");
    body["chats"]
        .as_array()
        .map(|entries| entries.iter().map(|entry| entry["chat"].clone()).collect())
        .unwrap_or_default()
}

async fn messages_of(ts: &TestServer, token: &str, chat_id: i64) -> Vec<Value> {
    let response = ts.get(token, &format!("/chats/{chat_id}/messages")).await;
    assert_eq!(response.status(), 200, "reading a chat's messages");
    let body: Value = response.json().await.expect("messages response is JSON");
    body["messages"]
        .as_array()
        .cloned()
        .expect("messages is an array")
}

/// Register a device and answer nothing — the row is what matters.
async fn register_device(ts: &TestServer, token: &str, push_token: &str) {
    let response = ts
        .post(
            token,
            "/devices",
            json!({"platform": "ios", "push_token": push_token}),
        )
        .await;
    assert_eq!(response.status(), 201, "registering a device");
}

/// Wait for the push seam to have been called at least `n` times. Dispatch
/// is spawned fire-and-forget, so a bare read races it.
async fn wait_for_push_calls(ts: &TestServer, n: usize) -> Vec<common::PushCall> {
    let deadline = tokio::time::Instant::now() + FRAME_WAIT;
    loop {
        let calls = ts.push.calls();
        if calls.len() >= n {
            return calls;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the push seam was called {} times, expected {n}",
            calls.len()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The end of an account, from the outside: the call succeeds, every door
/// the account had is shut, and the name it held is available again.
///
/// The sentinel is checked too. The scrub renames the row to
/// `deleted-<id>`, and somebody who reads a message's `sender_id` can work
/// that name out — so it has to be a name that cannot be signed into
/// either, whatever password is tried.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_deleted_account_can_never_be_signed_into_again_and_frees_its_username() {
    let ts = spawn_server().await;
    let (_owner, _owner_id, member, member_id, _chat_id, _code) = family_of_two(&ts).await;

    // A second session, so "every session" is more than one.
    let other_session = ts.login("junior", "password123").await;

    let response = delete_account(&ts, &member, "password123").await;
    assert_eq!(response.status(), 204);
    assert!(
        response
            .bytes()
            .await
            .expect("a body, even an empty one")
            .is_empty(),
        "204 means no content"
    );

    // Both sessions are gone, not just the one that asked.
    assert_error(ts.get(&member, "/me").await, 401, "unauthorized").await;
    assert_error(ts.get(&other_session, "/me").await, 401, "unauthorized").await;

    // The password can never verify again — not under the old name...
    assert_error(
        ts.login_raw("junior", "password123").await,
        401,
        "invalid_credentials",
    )
    .await;
    // ...and not under the sentinel the scrub wrote, which anybody holding
    // one of their old messages can guess.
    let sentinel = format!("deleted-{member_id}");
    assert_error(
        ts.login_raw(&sentinel, "password123").await,
        401,
        "invalid_credentials",
    )
    .await;
    // The hash column holds `'!'`, so even naming the sentinel itself as the
    // password is refused rather than compared.
    assert_error(
        ts.login_raw(&sentinel, "!").await,
        401,
        "invalid_credentials",
    )
    .await;

    // And the name is free for somebody else — a NEW account, not the old
    // one handed back.
    let (fresh, fresh_id) = ts.register("junior", "A New Junior").await;
    assert_ne!(fresh_id, member_id, "a new account, not the old one back");
    let me: Value = ts
        .get(&fresh, "/me")
        .await
        .json()
        .await
        .expect("me response is JSON");
    assert_eq!(me["user"]["display_name"], "A New Junior");
    assert!(
        me["family"].is_null(),
        "the freed username brings none of the old account's membership"
    );
}

/// The password proof, and the two ways of failing it.
///
/// A live session is not proof: the case this protects against is an
/// unattended unlocked phone, where a session is exactly what the attacker
/// already has. And a refusal has to leave the account working — a
/// half-applied deletion is the one outcome nobody could recover from.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_deletion_without_the_right_password_changes_nothing() {
    let ts = spawn_server().await;
    let (_owner, _owner_id, member, _member_id, chat_id, _code) = family_of_two(&ts).await;
    let message_id = send(&ts, &member, chat_id, "still here").await;

    assert_error(
        delete_account(&ts, &member, "not-my-password").await,
        401,
        "invalid_credentials",
    )
    .await;
    // An empty password is a malformed request rather than a wrong one, and
    // the code says which: `validation`, not `invalid_credentials`.
    assert_error(delete_account(&ts, &member, "").await, 400, "validation").await;
    // The key missing entirely is the same answer, from the same branch.
    assert_error(
        ts.post(&member, "/me/delete", json!({})).await,
        400,
        "validation",
    )
    .await;

    // Refused means untouched: session, membership and words all intact.
    let response = ts.get(&member, "/me").await;
    assert_eq!(response.status(), 200, "the account still works");
    let me: Value = response.json().await.expect("me response is JSON");
    assert_eq!(me["role"], "member");
    assert!(
        messages_of(&ts, &member, chat_id)
            .await
            .iter()
            .any(|m| m["id"].as_i64() == Some(message_id))
    );

    // With the right password it goes through, so the refusals above were
    // about the password and not about the account.
    delete_ok(&ts, &member).await;
}

/// THE CORE PROMISE. Everything the departing member said is still in the
/// family chat, still attributed to the same row, and the roster can still
/// put a name to it — from `former_members`, which is the only place a
/// tombstone appears.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_words_stay_in_the_family_chat_while_the_person_moves_to_former_members() {
    let ts = spawn_server().await;
    let (owner, owner_id, member, member_id, chat_id, _code) = family_of_two(&ts).await;

    send(&ts, &owner, chat_id, "Dinner at 7?").await;
    let theirs = send(&ts, &member, chat_id, "I'll bring the bread").await;
    // The OWNER quotes the member — the exact hole the design refuses to
    // punch: "replies answer messages that are no longer there".
    let response = ts
        .post(
            &owner,
            &format!("/chats/{chat_id}/messages"),
            json!({
                "client_msg_id": Uuid::new_v4().to_string(),
                "body": "perfect",
                "reply_to_message_id": theirs,
            }),
        )
        .await;
    assert_eq!(response.status(), 201);

    // Everything that identifies the person, so the scrub has something to
    // destroy.
    let response = ts
        .put(&member, "/me/birthday", json!({"month": 3, "day": 14}))
        .await;
    assert_eq!(response.status(), 200);
    let response = ts
        .put_bytes(&member, "/me/avatar", "image/jpeg", jpeg_bytes(256, 0x00))
        .await;
    assert_eq!(response.status(), 200);

    delete_ok(&ts, &member).await;

    // The words: same id, same body, same sender.
    let page = messages_of(&ts, &owner, chat_id).await;
    let kept = page
        .iter()
        .find(|m| m["id"].as_i64() == Some(theirs))
        .expect("the departed member's message is still in the family chat");
    assert_eq!(kept["sender_id"], member_id);
    assert_eq!(kept["body"], "I'll bring the bread");
    let reply = page
        .iter()
        .find(|m| m["body"] == "perfect")
        .expect("the owner's reply to it");
    let quoted = &reply["reply_to"];
    assert_eq!(
        quoted["message_id"].as_i64(),
        Some(theirs),
        "a reply must still answer the message it answered"
    );
    assert_eq!(quoted["sender_id"], member_id);
    assert_eq!(
        quoted["excerpt"], "I'll bring the bread",
        "and still quote what it quoted"
    );

    // The person: not a member, and in the second array instead.
    let family = my_family(&ts, &owner).await;
    let members = family["members"].as_array().expect("members is an array");
    assert_eq!(members.len(), 1, "only the owner is a member now");
    assert_eq!(members[0]["id"], owner_id);
    assert_eq!(
        members[0]["role"], "owner",
        "and is exactly what they were: nobody else's deletion moves a role"
    );
    assert!(
        members.iter().all(|m| m["id"].as_i64() != Some(member_id)),
        "a tombstone is never in `members`"
    );

    let former = family["former_members"]
        .as_array()
        .expect("former_members appears once somebody has been deleted");
    assert_eq!(former.len(), 1);
    let tombstone = &former[0];
    assert_eq!(tombstone["id"], member_id);
    assert_eq!(tombstone["deleted"], json!(true));
    assert_eq!(tombstone["display_name"], "Deleted account");
    assert_eq!(tombstone["username"], format!("deleted-{member_id}"));
    assert_eq!(
        tombstone["avatar_version"],
        json!(0),
        "0 is the wire saying `no picture`"
    );
    assert!(tombstone["role"].is_null(), "a tombstone holds no role");
    assert!(tombstone["birthday"].is_null(), "and no birthday");

    // The picture is really gone, and the endpoint only ever says "no".
    assert_error(
        ts.get(&owner, &format!("/users/{member_id}/avatar")).await,
        404,
        "user_not_found",
    )
    .await;
}

/// The other half of the shared record: a note on the family board and a
/// reaction on somebody else's message. Both outlive the account, and both
/// keep naming the row that made them.
///
/// These are the two foreign keys that would have CASCADED if the row had
/// been deleted rather than scrubbed — the board and the family's reactions
/// would have gone with one member's account.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn board_notes_and_reactions_outlive_the_account_that_left_them() {
    let ts = spawn_server().await;
    let (owner, _owner_id, member, member_id, chat_id, _code) = family_of_two(&ts).await;

    let ours = send(&ts, &owner, chat_id, "Dinner at 7?").await;
    let response = ts
        .put(
            &member,
            &format!("/chats/{chat_id}/messages/{ours}/reaction"),
            json!({"emoji": "❤️"}),
        )
        .await;
    assert_eq!(response.status(), 200);
    let response = ts
        .post(
            &member,
            "/families/mine/board/notes",
            json!({"text": "Milk", "color": "yellow", "x": 0.4, "y": 0.1}),
        )
        .await;
    assert_eq!(response.status(), 201);
    let note: Value = response.json().await.expect("note response is JSON");
    let note_id = note["note"]["id"].as_i64().expect("note id");

    delete_ok(&ts, &member).await;

    let reacted = messages_of(&ts, &owner, chat_id)
        .await
        .into_iter()
        .find(|m| m["id"].as_i64() == Some(ours))
        .expect("the owner's message");
    assert_eq!(
        reacted["reactions"],
        json!([{"user_id": member_id, "emoji": "❤️"}]),
        "the reaction stays, still attributed"
    );

    let board: Value = ts
        .get(&owner, "/families/mine/board")
        .await
        .json()
        .await
        .expect("board response is JSON");
    let notes = board["notes"].as_array().expect("notes is an array");
    assert_eq!(notes.len(), 1, "the board is untouched");
    assert_eq!(notes[0]["id"].as_i64(), Some(note_id));
    assert_eq!(notes[0]["text"], "Milk");
    assert_eq!(
        notes[0]["author_id"], member_id,
        "and still says who pinned it"
    );
}

/// A one-to-one chat has no meaning with one side removed, so it goes —
/// and it goes for the PEER too, messages and all. That is the honest
/// reading of a private conversation ending, and the only history a
/// departing member can take with them without taking somebody else's.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_direct_chat_is_gone_for_the_peer_too() {
    let ts = spawn_server().await;
    let (owner, _owner_id, member, member_id, chat_id, _code) = family_of_two(&ts).await;

    let response = ts
        .post(&owner, "/chats/direct", json!({"user_id": member_id}))
        .await;
    assert_eq!(response.status(), 200);
    let direct: Value = response.json().await.expect("chat response is JSON");
    let direct_id = direct["chat"]["id"].as_i64().expect("chat id");
    send(&ts, &owner, direct_id, "just between us").await;
    send(&ts, &member, direct_id, "and between us too").await;
    let public = send(&ts, &member, chat_id, "this one is for everyone").await;

    delete_ok(&ts, &member).await;

    // Gone from the peer's list...
    let kinds: Vec<String> = chats_of(&ts, &owner)
        .await
        .iter()
        .map(|chat| chat["kind"].as_str().expect("kind").to_string())
        .collect();
    assert_eq!(
        kinds,
        vec!["family".to_string()],
        "the direct chat is gone, both halves"
    );
    // ...and gone from the database, not merely hidden: the peer's own
    // messages in it went with it.
    assert_error(
        ts.get(&owner, &format!("/chats/{direct_id}/messages"))
            .await,
        404,
        "chat_not_found",
    )
    .await;
    let left_behind: i64 = sqlx::query_scalar("SELECT count(*) FROM messages WHERE chat_id = $1")
        .bind(direct_id)
        .fetch_one(&ts.state.pool)
        .await
        .expect("counting the deleted chat's messages");
    assert_eq!(left_behind, 0, "including the peer's own");

    // The family chat is untouched — the shared record is not private
    // history.
    assert!(
        messages_of(&ts, &owner, chat_id)
            .await
            .iter()
            .any(|m| m["id"].as_i64() == Some(public))
    );
    // And a tombstone is nobody to start a new chat with.
    assert_error(
        ts.post(&owner, "/chats/direct", json!({"user_id": member_id}))
            .await,
        409,
        "not_same_family",
    )
    .await;
}

/// An owner is never refused, and the family never dies with them while
/// somebody is still in it: ownership passes to the LONGEST-STANDING
/// remaining member.
///
/// The fixture is built so that "longest-standing" and "lowest id" give
/// DIFFERENT answers — the later-registered account joins first — because
/// an implementation ordering by `user_id` passes the easy version of this
/// test and is wrong the moment somebody rejoins.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn ownership_passes_to_the_earliest_joiner_even_when_they_are_not_the_lowest_id() {
    let ts = spawn_server().await;
    let (owner, owner_id) = ts.register("owner", "Olive").await;
    // Registered FIRST, so the lower id...
    let (hazel, hazel_id) = ts.register("hazel", "Hazel").await;
    // ...and registered second, so the higher one.
    let (ivan, ivan_id) = ts.register("ivan", "Ivan").await;
    assert!(hazel_id < ivan_id, "ids follow registration order");

    let (family_id, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    // Ivan joins FIRST. He is the longest-standing member and the higher id
    // at the same time, which is the whole point of the fixture.
    ts.join(&ivan, &invite_code, "joined").await;
    ts.join(&hazel, &invite_code, "joined").await;

    // Said once, in the schema's own terms, so a failure below reads as
    // "the successor was chosen wrongly" and not "the fixture drifted".
    let joiners: Vec<i64> = sqlx::query_scalar(
        "SELECT cm.user_id
           FROM chat_members cm
           JOIN chats c ON c.id = cm.chat_id
          WHERE c.family_id = $1 AND c.kind = 'family' AND cm.user_id <> $2
          ORDER BY cm.joined_at ASC, cm.user_id ASC",
    )
    .bind(family_id)
    .bind(owner_id)
    .fetch_all(&ts.state.pool)
    .await
    .expect("reading the family chat's join order");
    assert_eq!(
        joiners,
        vec![ivan_id, hazel_id],
        "by joined_at the successor is Ivan; by user id it would be Hazel"
    );

    // Something for the new owner to inherit that only an owner may see.
    let response = ts
        .patch(&owner, "/families/mine", json!({"join_policy": "approval"}))
        .await;
    assert_eq!(response.status(), 200);
    let (waiting, waiting_id) = ts.register("waiting", "Waiting").await;
    ts.join(&waiting, &invite_code, "pending").await;
    // Which Ivan cannot see yet.
    assert_error(
        ts.get(&ivan, "/families/join-requests").await,
        403,
        "not_family_owner",
    )
    .await;

    delete_ok(&ts, &owner).await;

    // Ivan owns the family now, and finds out the ordinary way.
    let me: Value = ts
        .get(&ivan, "/me")
        .await
        .json()
        .await
        .expect("me response is JSON");
    assert_eq!(me["role"], "owner");
    assert_eq!(
        me["family"]["id"].as_i64(),
        Some(family_id),
        "the same family, not a new one"
    );
    assert_eq!(
        me["family"]["invite_code"], invite_code,
        "the invite code survives the owner"
    );
    // Hazel is exactly where she was.
    let hazel_me: Value = ts
        .get(&hazel, "/me")
        .await
        .json()
        .await
        .expect("me response is JSON");
    assert_eq!(hazel_me["role"], "member");

    // The owner-only endpoint answers Ivan now, with the request that was
    // pending all along — ownership moved, not just the label.
    let requests: Value = ts
        .get(&ivan, "/families/join-requests")
        .await
        .json()
        .await
        .expect("join requests response is JSON");
    let pending = requests["requests"].as_array().expect("requests is array");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0]["user"]["id"].as_i64(), Some(waiting_id));
    // And still refuses everybody else.
    assert_error(
        ts.get(&hazel, "/families/join-requests").await,
        403,
        "not_family_owner",
    )
    .await;

    // The roster: two live members, one former.
    let family = my_family(&ts, &ivan).await;
    let roles: Vec<(i64, String)> = family["members"]
        .as_array()
        .expect("members is an array")
        .iter()
        .map(|m| {
            (
                m["id"].as_i64().expect("id"),
                m["role"]
                    .as_str()
                    .expect("a live member always has a role")
                    .to_string(),
            )
        })
        .collect();
    assert_eq!(roles.len(), 2);
    assert!(roles.contains(&(ivan_id, "owner".to_string())));
    assert!(roles.contains(&(hazel_id, "member".to_string())));
    assert_eq!(family["former_members"][0]["id"].as_i64(), Some(owner_id));
}

/// The successor is re-checked with a lock held on their OWN row, and a
/// pick that fails the re-check is passed over for the next-longest-
/// standing member.
///
/// The bug this pins: the ordering query is a plain SELECT, and a plain
/// SELECT says nothing about a `users` row an uncommitted transaction is
/// in the middle of changing — a member half way through
/// `POST /families/leave` is still `family_id = F` to it. So the owner's
/// deletion handed the family to somebody on their way out of it, and
/// `require_owner` then refused every remaining member: no invite code, no
/// approvals, no removals, no language, no password reset, and no way to
/// delete the family either. Nothing in this server ever repairs
/// `families.owner_user_id`.
///
/// The departure below is written exactly as `remove_membership` writes it
/// and held open exactly as `leave_family` now holds it, which is what
/// makes the interleaving deterministic instead of a coin toss.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_successor_leaving_is_passed_over_for_the_next_longest_standing_member() {
    let ts = spawn_server().await;
    let (owner, owner_id) = ts.register("owner", "Olive").await;
    let (ivan, ivan_id) = ts.register("ivan", "Ivan").await;
    let (hazel, hazel_id) = ts.register("hazel", "Hazel").await;
    let (family_id, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    // Ivan joined first, so the ordering picks him — and he is the one
    // leaving.
    ts.join(&ivan, &invite_code, "joined").await;
    ts.join(&hazel, &invite_code, "joined").await;

    let mut leaving = ts.state.pool.begin().await.expect("begin the leave");
    sqlx::query("UPDATE users SET family_id = NULL WHERE id = $1 AND family_id = $2")
        .bind(ivan_id)
        .bind(family_id)
        .execute(&mut *leaving)
        .await
        .expect("Ivan's half-finished departure");

    let deleting = delete_account(&ts, &owner, "password123");
    tokio::pin!(deleting);
    // It must WAIT. Reaching a decision here means the successor was
    // chosen off a snapshot that cannot see the write above.
    assert!(
        tokio::time::timeout(Duration::from_millis(500), &mut deleting)
            .await
            .is_err(),
        "the handover must block on the successor's own row"
    );

    leaving.commit().await.expect("Ivan's departure lands");
    let response = deleting.await;
    assert_eq!(
        response.status(),
        204,
        "an account is never refused deletion"
    );

    // Hazel, not Ivan: the same answer the ordering would have given had
    // the leave landed a moment earlier.
    let owner_now: i64 = sqlx::query_scalar("SELECT owner_user_id FROM families WHERE id = $1")
        .bind(family_id)
        .fetch_one(&ts.state.pool)
        .await
        .expect("the family still exists");
    assert_eq!(
        owner_now, hazel_id,
        "the family went to the member who stayed"
    );
    assert_ne!(owner_now, ivan_id);
    assert_ne!(owner_now, owner_id);
    // And it is ownership, not just a column: the owner-only endpoints
    // answer her and nobody else.
    assert_eq!(
        ts.get(&hazel, "/families/join-requests").await.status(),
        200
    );
    assert_error(
        ts.get(&ivan, "/families/join-requests").await,
        403,
        "not_family_owner",
    )
    .await;
}

/// The other half of the same rule: `leave_family` holds the family row's
/// lock ACROSS the membership write.
///
/// It used to roll the lock back and let `remove_membership` open a
/// transaction of its own, which left a window with no lock on the family
/// at all — and `delete_account`, which takes that row to choose a
/// successor, could walk straight into it. Both writes are membership
/// changes in one family, so they have to queue on one row.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_leave_holds_the_family_rows_lock_across_the_membership_write() {
    // Three connections at once — the parked write, the blocked request and
    // the probe — where the harness's default is two.
    let ts = spawn_server_with_config(|cfg| cfg.database.max_connections = 8).await;
    let (owner, _owner_id) = ts.register("owner", "Olive").await;
    let (ivan, ivan_id) = ts.register("ivan", "Ivan").await;
    let (family_id, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&ivan, &invite_code, "joined").await;

    // Park the leave inside its own write: this is the row
    // `remove_membership` updates first.
    let mut holder = ts.state.pool.begin().await.expect("begin");
    let _: i64 = sqlx::query_scalar("SELECT id FROM users WHERE id = $1 FOR UPDATE")
        .bind(ivan_id)
        .fetch_one(&mut *holder)
        .await
        .expect("locking Ivan's row");

    let leaving = ts.post(&ivan, "/families/leave", json!({}));
    tokio::pin!(leaving);
    assert!(
        tokio::time::timeout(Duration::from_millis(500), &mut leaving)
            .await
            .is_err(),
        "the leave should be waiting on the row it is about to write"
    );

    // While it waits there, the family row is taken. NOWAIT so this is an
    // answer rather than a second thing to wait for.
    let mut probe = ts.state.pool.begin().await.expect("begin");
    let taken =
        sqlx::query_scalar::<_, i64>("SELECT id FROM families WHERE id = $1 FOR UPDATE NOWAIT")
            .bind(family_id)
            .fetch_optional(&mut *probe)
            .await;
    assert!(
        taken.is_err(),
        "the membership write must happen under the family row's lock"
    );
    probe.rollback().await.ok();

    holder.rollback().await.expect("release Ivan's row");
    let response = leaving.await;
    assert_eq!(response.status(), 204);
    let family_of_ivan: Option<i64> =
        sqlx::query_scalar("SELECT family_id FROM users WHERE id = $1")
            .bind(ivan_id)
            .fetch_one(&ts.state.pool)
            .await
            .expect("Ivan's row");
    assert_eq!(family_of_ivan, None, "he really did leave");
}

/// The mirror rule: with nobody left, the family goes too — its chat, its
/// board, its attachments and its invite code.
///
/// The FILE is the part that needs saying twice. Nothing in the database
/// names those bytes once the family row goes, so nothing else would ever
/// have removed them: the keys have to be collected inside the transaction
/// that deletes the family and swept after it commits, which is what
/// `delete_family_in_tx` returns them for.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_sole_owner_takes_the_family_and_its_invite_code_with_them() {
    let ts = spawn_server().await;
    let (owner, _owner_id) = ts.register("owner", "Olive").await;
    let (family_id, invite_code) = ts.create_family(&owner, "The Smiths").await;
    let chat_id = ts.family_chat_id(&owner).await;
    send(&ts, &owner, chat_id, "talking to myself").await;
    let response = ts
        .post(
            &owner,
            "/families/mine/board/notes",
            json!({"text": "Milk", "color": "yellow", "x": 0.4, "y": 0.1}),
        )
        .await;
    assert_eq!(response.status(), 201);
    let attachment_id = upload(&ts, &owner, jpeg_bytes(512, 0x00)).await;
    send_attachment(&ts, &owner, chat_id, attachment_id).await;
    let path = ts
        .state
        .storage
        .blob_path(&storage_key(&ts, attachment_id).await);
    assert!(path.exists());

    delete_ok(&ts, &owner).await;

    // The bytes are off the disk with everything else.
    assert!(
        !path.exists(),
        "the family's attachment outlived the family"
    );

    let families: i64 = sqlx::query_scalar("SELECT count(*) FROM families WHERE id = $1")
        .bind(family_id)
        .fetch_one(&ts.state.pool)
        .await
        .expect("counting families");
    assert_eq!(families, 0, "the family went with its last member");

    // The invite code cannot let anybody into a family that is not there —
    // and the refusal does not distinguish "wrong code" from "gone", which
    // is the same answer any stale code gets.
    let (stranger, _) = ts.register("stranger", "Stranger").await;
    assert_error(
        ts.post(
            &stranger,
            "/families/join",
            json!({"invite_code": invite_code}),
        )
        .await,
        404,
        "invalid_invite_code",
    )
    .await;

    // Nobody is left to name them to, so the tombstone remembers no family
    // — the FK blanked it when the family went.
    let deleted_family_id: Option<i64> =
        sqlx::query_scalar("SELECT deleted_family_id FROM users WHERE deleted_at IS NOT NULL")
            .fetch_one(&ts.state.pool)
            .await
            .expect("the tombstone row");
    assert_eq!(deleted_family_id, None);
}

/// Attachments the member uploaded come off the disk — subject to the one
/// rule attachment deletion always obeys: a FILE goes only once no row
/// names those bytes any more (0011, "one copy per family").
///
/// Both attachment rows below are removed by the same cascade, and the two
/// files then differ in one thing only: whether anybody else's row still
/// points at them. An implementation that removed each collected key
/// outright instead of checking would delete a photo out of the owner's
/// message in the family chat — a message nobody deleted.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_accounts_files_go_unless_another_row_still_names_the_same_bytes() {
    let ts = spawn_server().await;
    let (owner, _owner_id, member, member_id, chat_id, _code) = family_of_two(&ts).await;

    // The owner sends a photo to the FAMILY chat. That message stays, so
    // its row keeps naming the file whatever happens to anybody else.
    let shared_bytes = jpeg_bytes(512, 0x00);
    let owners_copy = upload(&ts, &owner, shared_bytes.clone()).await;
    send_attachment(&ts, &owner, chat_id, owners_copy).await;

    // The member forwards the SAME bytes into a direct chat. 0011 makes
    // that a second row pointing at the first file rather than a second
    // file.
    let response = ts
        .post(&owner, "/chats/direct", json!({"user_id": member_id}))
        .await;
    assert_eq!(response.status(), 200);
    let direct: Value = response.json().await.expect("chat response is JSON");
    let direct_id = direct["chat"]["id"].as_i64().expect("chat id");
    let members_copy = upload(&ts, &member, shared_bytes).await;
    send_attachment(&ts, &member, direct_id, members_copy).await;
    // ...and a picture only they ever held, into the same chat.
    let members_own = upload(&ts, &member, jpeg_bytes(512, 0x5A)).await;
    send_attachment(&ts, &member, direct_id, members_own).await;

    let shared_key = storage_key(&ts, owners_copy).await;
    assert_eq!(
        storage_key(&ts, members_copy).await,
        shared_key,
        "identical bytes in one family are one file (0011); without that \
         this test proves nothing"
    );
    let lone_key = storage_key(&ts, members_own).await;
    assert_ne!(lone_key, shared_key);
    let shared_path = ts.state.storage.blob_path(&shared_key);
    let lone_path = ts.state.storage.blob_path(&lone_key);
    assert!(shared_path.exists());
    assert!(lone_path.exists());

    delete_ok(&ts, &member).await;

    // Both rows went with the direct chat...
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM attachments WHERE uploader_id = $1")
        .bind(member_id)
        .fetch_one(&ts.state.pool)
        .await
        .expect("counting the member's attachment rows");
    assert_eq!(rows, 0, "the member's attachment rows are gone");
    // ...but only one file did.
    assert!(
        !lone_path.exists(),
        "a file nothing names any more is off the disk"
    );
    assert!(
        shared_path.exists(),
        "a file the owner's family-chat message still names must survive — \
         deleting it would punch a hole in somebody else's message"
    );
    // The proof from the other side: the owner's photo is still readable.
    let response = ts.get(&owner, &format!("/attachments/{owners_copy}")).await;
    assert_eq!(
        response.status(),
        200,
        "the owner's photo still downloads after the other copy's account went"
    );
}

/// The family is told three things at once, in the order a client can
/// apply them, and the departing account's own devices are signed out.
///
/// `member_left` first (a client that predates the tombstone frame still
/// fixes its roster), then `member_deleted` carrying the whole tombstone
/// `Member` (which is exactly what a client has to overwrite its stored
/// name with), then `family_owner` naming the successor.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn deleting_closes_the_accounts_sockets_and_tells_the_family() {
    let ts = spawn_server().await;
    let (owner, owner_id, member, member_id, _chat_id, invite_code) = family_of_two(&ts).await;
    let (third, _third_id) = ts.register("third", "Third").await;
    ts.join(&third, &invite_code, "joined").await;

    let mut owner_ws = connect_ws(&ts, &owner).await;
    // Two connections for the same account: BOTH have to close.
    let mut owner_ws2 = connect_ws(&ts, &owner).await;
    let mut member_ws = connect_ws(&ts, &member).await;
    let mut third_ws = connect_ws(&ts, &third).await;

    // The OWNER deletes, so all three frames are in play at once.
    delete_ok(&ts, &owner).await;

    for ws in [&mut member_ws, &mut third_ws] {
        let left = next_frame_of_type(ws, "member_left").await;
        assert_eq!(left["user_id"], owner_id);

        let deleted = next_frame_of_type(ws, "member_deleted").await;
        assert_eq!(deleted["family_id"], left["family_id"]);
        let tombstone = &deleted["member"];
        assert_eq!(tombstone["id"], owner_id);
        assert_eq!(tombstone["deleted"], json!(true));
        assert_eq!(tombstone["display_name"], "Deleted account");
        assert_eq!(tombstone["username"], format!("deleted-{owner_id}"));
        assert_eq!(tombstone["avatar_version"], json!(0));
        assert!(
            tombstone["role"].is_null(),
            "the frame carries a tombstone, and a tombstone holds no role"
        );

        // The successor is named to EVERYBODY, not only to themselves: a
        // client draws the owner's crown on somebody else's row.
        let new_owner = next_frame_of_type(ws, "family_owner").await;
        assert_eq!(new_owner["family_id"], left["family_id"]);
        assert_eq!(
            new_owner["user_id"], member_id,
            "the earliest joiner takes the family"
        );
    }

    // Every socket of the deleted account closes with the session-gone
    // code: the sessions are gone, so there is nothing to reconnect with.
    for ws in [&mut owner_ws, &mut owner_ws2] {
        // `None` is a stack that surfaced the close as end-of-stream; an
        // end without a code is still an end.
        if let Some(code) = close_code(ws).await {
            assert_eq!(code, 4401);
        }
    }

    // And none of it is mail.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        ts.push.calls().is_empty(),
        "account deletion must never reach the push seam"
    );
}

/// Every device row goes with the sessions (0021's CASCADE), so no push
/// token outlives the account it belonged to and nothing is ever sent to a
/// phone whose account no longer exists.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_accounts_devices_go_with_its_sessions_so_nothing_is_pushed_to_them() {
    let ts = spawn_server().await;
    let (owner, _owner_id, member, member_id, chat_id, _code) = family_of_two(&ts).await;
    register_device(&ts, &member, "tok-junior").await;
    // A second device on a second session, so this is about the ACCOUNT and
    // not about the session that happens to ask.
    let second_session = ts.login("junior", "password123").await;
    register_device(&ts, &second_session, "tok-junior-tablet").await;

    // Nobody is connected, so a message wakes the member — that is the
    // baseline this test then takes away.
    send(&ts, &owner, chat_id, "anyone home?").await;
    let calls = wait_for_push_calls(&ts, 1).await;
    assert_eq!(calls.len(), 1);
    let woken: Vec<&str> = calls[0]
        .devices
        .iter()
        .map(|device| device.push_token.as_str())
        .collect();
    assert_eq!(woken.len(), 2, "both of the member's devices");
    assert!(woken.contains(&"tok-junior"));
    assert!(woken.contains(&"tok-junior-tablet"));

    // A THIRD row in the legacy shape: `session_id IS NULL`. Migration 0021
    // made the session FK CASCADE and every registration since 0020 binds a
    // session, but 0021 deliberately LEFT the pre-0020 orphans alone — so on
    // any server upgraded rather than freshly installed, a device row that no
    // session cascade can reach still exists. protocol.md says "every session
    // and every device row, and with them every push token", without an
    // exception for how the row got there.
    sqlx::query(
        "INSERT INTO devices (user_id, platform, push_token, session_id)
         VALUES ($1, 'ios', 'tok-junior-legacy', NULL)",
    )
    .bind(member_id)
    .execute(&ts.state.pool)
    .await
    .expect("inserting a pre-0020 device row");

    delete_ok(&ts, &member).await;

    let devices: i64 = sqlx::query_scalar("SELECT count(*) FROM devices WHERE user_id = $1")
        .bind(member_id)
        .fetch_one(&ts.state.pool)
        .await
        .expect("counting the deleted account's devices");
    assert_eq!(
        devices, 0,
        "the sessions took the device rows — and the push tokens — with them, \
         including one no session cascade could reach"
    );

    // The same message that woke them before now wakes nobody.
    send(&ts, &owner, chat_id, "still there?").await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(
        ts.push.calls().len(),
        1,
        "no push may be attempted for an account that no longer exists"
    );
}

/// A direct-chat peer can be OUTSIDE the family — the direct pair is unique
/// globally and a chat follows a pair across families — and they are told
/// too, because their chat is about to vanish from under them.
///
/// Bob and Alice open a chat in Alice's family; Bob then leaves and joins
/// another family entirely. Alice is in no family of Bob's by the time he
/// deletes, and the roster fan-out cannot reach her — only the chat can.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_direct_chat_peer_outside_the_family_is_told_as_well() {
    let ts = spawn_server().await;
    let (alice, alice_id) = ts.register("alice", "Alice").await;
    let (bob, bob_id) = ts.register("bob", "Bob").await;
    let (carol, _carol_id) = ts.register("carol", "Carol").await;

    let (alders_id, alders_code) = ts.create_family(&alice, "The Alders").await;
    ts.set_open_policy(&alice).await;
    ts.join(&bob, &alders_code, "joined").await;

    let response = ts
        .post(&alice, "/chats/direct", json!({"user_id": bob_id}))
        .await;
    assert_eq!(response.status(), 200);
    let direct: Value = response.json().await.expect("chat response is JSON");
    let direct_id = direct["chat"]["id"].as_i64().expect("chat id");
    send(&ts, &alice, direct_id, "see you Sunday").await;

    // Bob moves families. The chat follows the pair: Alice keeps it in her
    // list even though the two of them share no family any more.
    let response = ts.post(&bob, "/families/leave", json!({})).await;
    assert_eq!(response.status(), 204);
    let (birches_id, birches_code) = ts.create_family(&carol, "The Birches").await;
    ts.set_open_policy(&carol).await;
    ts.join(&bob, &birches_code, "joined").await;
    assert_ne!(alders_id, birches_id);
    assert!(
        chats_of(&ts, &alice)
            .await
            .iter()
            .any(|chat| chat["id"].as_i64() == Some(direct_id)),
        "the direct chat outlives the shared family"
    );

    let mut alice_ws = connect_ws(&ts, &alice).await;
    let mut carol_ws = connect_ws(&ts, &carol).await;

    delete_ok(&ts, &bob).await;

    // Carol hears it as a member of Bob's family...
    let left = next_frame_of_type(&mut carol_ws, "member_left").await;
    assert_eq!(left["user_id"], bob_id);
    assert_eq!(left["family_id"].as_i64(), Some(birches_id));

    // ...and Alice hears it only because of the chat. The frame names Bob's
    // family, which Alice is not in: it is the one id the frame has to
    // carry, and what she needs from it is the `member`, not the family.
    let deleted = next_frame_of_type(&mut alice_ws, "member_deleted").await;
    assert_eq!(deleted["member"]["id"].as_i64(), Some(bob_id));
    assert_eq!(deleted["member"]["deleted"], json!(true));
    assert_eq!(deleted["member"]["display_name"], "Deleted account");

    // And the chat she was told about is gone from her list.
    assert!(
        chats_of(&ts, &alice)
            .await
            .iter()
            .all(|chat| chat["id"].as_i64() != Some(direct_id)),
        "the direct chat went with the account"
    );
    // Alice's own family is untouched by any of it.
    let family = my_family(&ts, &alice).await;
    assert_eq!(family["family"]["id"].as_i64(), Some(alders_id));
    assert_eq!(
        family["members"].as_array().expect("members").len(),
        1,
        "Alice is alone in her family, and still in it"
    );
    assert!(
        family["former_members"].is_null(),
        "Bob was deleted from the OTHER family; Alice's roster does not name him"
    );
    assert_eq!(alice_id, family["members"][0]["id"].as_i64().expect("id"));
}

/// The same rule with NO family at all behind it: an account that left its
/// family still has direct chats, and deleting it must still tell those
/// peers — with no `family_id`, because there is no family to name.
///
/// Without this the deletion is SILENT: the peer's chat is destroyed under
/// them and their live client goes on drawing a chat that no longer exists
/// until its next `GET /chats`.
///
/// The fixture has to be built this way round. `remove_membership` deletes
/// a leaver's `chat_members` rows for EVERY chat carrying that `family_id`,
/// the direct one included, so it is the peer who must stay in the family
/// that made the chat: A and B share a direct chat in A's family, B leaves,
/// and A is still a member of the chat that names them both.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_account_with_no_family_still_tells_its_direct_chat_peers() {
    let ts = spawn_server().await;
    let (alice, alice_id) = ts.register("alice", "Alice").await;
    let (bob, bob_id) = ts.register("bob", "Bob").await;
    let (alders_id, alders_code) = ts.create_family(&alice, "The Alders").await;
    ts.set_open_policy(&alice).await;
    ts.join(&bob, &alders_code, "joined").await;

    // Made while they still shared a family, which is the only time
    // `POST /chats/direct` will make one.
    let response = ts
        .post(&alice, "/chats/direct", json!({"user_id": bob_id}))
        .await;
    assert_eq!(response.status(), 200);
    let direct: Value = response.json().await.expect("chat response is JSON");
    let direct_id = direct["chat"]["id"].as_i64().expect("chat id");
    send(&ts, &alice, direct_id, "see you Sunday").await;

    // Bob leaves and joins nothing. He now belongs to no family at all —
    // the case the roster fan-out has no way to reach.
    let response = ts.post(&bob, "/families/leave", json!({})).await;
    assert_eq!(response.status(), 204);
    let me: Value = ts
        .get(&bob, "/me")
        .await
        .json()
        .await
        .expect("me response is JSON");
    assert!(
        me["family"].is_null(),
        "the fixture needs Bob in NO family, not in another one"
    );
    // The chat follows the pair, so Alice still holds it.
    assert!(
        chats_of(&ts, &alice)
            .await
            .iter()
            .any(|chat| chat["id"].as_i64() == Some(direct_id)),
        "the direct chat outlives the membership that created it"
    );

    let mut alice_ws = connect_ws(&ts, &alice).await;

    delete_ok(&ts, &bob).await;

    // Alice is told, and the tombstone is the whole point of the frame.
    let deleted = next_frame_of_type(&mut alice_ws, "member_deleted").await;
    assert_eq!(deleted["member"]["id"].as_i64(), Some(bob_id));
    assert_eq!(deleted["member"]["deleted"], json!(true));
    assert_eq!(deleted["member"]["display_name"], "Deleted account");
    assert_eq!(deleted["member"]["username"], format!("deleted-{bob_id}"));
    // ABSENT, not null: the house wire rule, and here it is also the only
    // honest answer — there is no family this account belonged to.
    assert!(
        deleted
            .as_object()
            .expect("a frame is a JSON object")
            .get("family_id")
            .is_none(),
        "an account with no family sends no family_id at all: {deleted}"
    );

    // And the chat she was told about is gone from her list.
    assert!(
        chats_of(&ts, &alice)
            .await
            .iter()
            .all(|chat| chat["id"].as_i64() != Some(direct_id)),
        "the direct chat went with the account"
    );
    // Alice's own family is untouched, and never names Bob: he was in no
    // family when he went, so there is no former membership to remember.
    let family = my_family(&ts, &alice).await;
    assert_eq!(family["family"]["id"].as_i64(), Some(alders_id));
    assert_eq!(
        family["members"].as_array().expect("members").len(),
        1,
        "Alice is alone in her family, and still in it"
    );
    assert_eq!(family["members"][0]["id"].as_i64(), Some(alice_id));
    assert!(
        family["former_members"].is_null(),
        "an account that belonged to no family leaves no former member"
    );
}

/// A live tally must not go on counting somebody who no longer exists, and
/// a finished one is a record like any other message.
///
/// The retraction is a change to the poll like any other: it takes the next
/// sequence value and fans the new state out, or a client that was offline
/// would show the departed member's vote forever.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_open_poll_drops_the_vote_and_fans_out_while_a_closed_one_keeps_it() {
    let ts = spawn_server().await;
    let (owner, owner_id, member, member_id, chat_id, _code) = family_of_two(&ts).await;

    let mut polls = Vec::new();
    for question in ["Pizza or pasta?", "Beach or mountains?"] {
        let response = ts
            .post(
                &owner,
                &format!("/chats/{chat_id}/messages"),
                json!({
                    "client_msg_id": Uuid::new_v4().to_string(),
                    "body": question,
                    "poll": {"options": ["One", "Two"]},
                }),
            )
            .await;
        assert_eq!(response.status(), 201);
        let body: Value = response.json().await.expect("message response is JSON");
        let poll = &body["message"]["poll"];
        polls.push((
            body["message"]["id"].as_i64().expect("message id"),
            poll["options"][0]["id"].as_i64().expect("option id"),
            poll["options"][1]["id"].as_i64().expect("option id"),
        ));
    }
    let (open_poll, open_first, open_second) = polls[0];
    let (closed_poll, closed_first, _) = polls[1];

    vote(&ts, &member, chat_id, open_poll, open_first).await;
    // Somebody else votes too, so an implementation that cleared the whole
    // tally instead of one row would be caught.
    vote(&ts, &owner, chat_id, open_poll, open_second).await;
    vote(&ts, &member, chat_id, closed_poll, closed_first).await;

    let response = ts
        .post(
            &owner,
            &format!("/chats/{chat_id}/messages/{closed_poll}/poll/close"),
            json!({}),
        )
        .await;
    assert_eq!(response.status(), 200);
    let closed_state: Value = response.json().await.expect("poll response is JSON");
    let closed_seq = closed_state["poll"]["poll_seq"].as_i64().expect("poll_seq");

    let mut owner_ws = connect_ws(&ts, &owner).await;
    delete_ok(&ts, &member).await;

    // The open poll's new state reaches the family live.
    let frame = next_frame_of_type(&mut owner_ws, "poll").await;
    assert_eq!(
        frame["message_id"].as_i64(),
        Some(open_poll),
        "a closed poll changes nothing, so only the open one fans out"
    );
    assert_eq!(frame["chat_id"].as_i64(), Some(chat_id));
    assert_eq!(
        frame["poll"]["options"][0]["votes"],
        json!([]),
        "the tally stopped counting them"
    );
    assert_eq!(
        frame["poll"]["options"][1]["votes"],
        json!([owner_id]),
        "and counts everybody else exactly as before"
    );
    assert!(
        frame["poll"]["poll_seq"].as_i64().expect("poll_seq") > closed_seq,
        "a retraction takes the next sequence value, or no catch-up carries it"
    );

    // The same two answers from the catch-up feed, which is what an offline
    // client reads.
    let feed: Value = ts
        .get(&owner, &format!("/chats/{chat_id}/polls?after_seq=0"))
        .await
        .json()
        .await
        .expect("polls response is JSON");
    let entries = feed["polls"].as_array().expect("polls is an array");
    let open = entries
        .iter()
        .find(|entry| entry["message_id"].as_i64() == Some(open_poll))
        .expect("the open poll");
    assert!(
        open["poll"]["options"][0]["votes"]
            .as_array()
            .expect("votes")
            .is_empty()
    );
    let closed = entries
        .iter()
        .find(|entry| entry["message_id"].as_i64() == Some(closed_poll))
        .expect("the closed poll");
    assert_eq!(closed["poll"]["closed"], json!(true));
    assert_eq!(
        closed["poll"]["options"][0]["votes"],
        json!([member_id]),
        "a finished poll keeps its record, the same way the messages do"
    );
    assert_eq!(
        closed["poll"]["poll_seq"].as_i64(),
        Some(closed_seq),
        "and burns no sequence value"
    );
}

/// The assistant is a reserved account in no family, and a member deleting
/// theirs must not touch it — nor the usage rows that say what it cost.
///
/// The member's private assistant thread DOES go, like their direct chats,
/// and the usage row that named a message in it keeps its billing figures
/// with `message_id` blanked. A usage row is what "the family spent this"
/// is computed from; losing one because somebody left would quietly rewrite
/// the family's statistics.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_assistant_account_and_the_usage_it_recorded_are_untouched() {
    let ts = spawn_server_with_config(|cfg| {
        cfg.ai.enabled = true;
        cfg.ai.endpoint = "https://example.invalid".to_string();
        cfg.ai.deployment = "test-deployment".to_string();
        cfg.ai.api_key = "test-key".to_string();
        cfg.ai.title = "Assistant".to_string();
    })
    .await;
    let (owner, _owner_id, member, member_id, chat_id, _code) = family_of_two(&ts).await;
    let family_id: i64 = sqlx::query_scalar("SELECT id FROM families")
        .fetch_one(&ts.state.pool)
        .await
        .expect("the family id");
    let assistant_id = family_connect::handlers_ai::assistant_user_id(&ts.state)
        .await
        .expect("the query runs")
        .expect("migration 0015 inserted the assistant account");

    // The member's own thread with the assistant. Listing the chats is what
    // creates it.
    let ai_chat = chats_of(&ts, &member)
        .await
        .into_iter()
        .find(|chat| chat["kind"] == "ai")
        .and_then(|chat| chat["id"].as_i64())
        .expect("a configured server gives every member an assistant chat");
    let asked = send(&ts, &member, ai_chat, "what is for dinner?").await;
    // Two usage rows: one for the question in the private thread (whose
    // message is about to go), one for a message in the family chat (which
    // stays).
    let public = send(&ts, &member, chat_id, "asking the assistant").await;
    for message_id in [asked, public] {
        sqlx::query(
            "INSERT INTO ai_usage (user_id, family_id, message_id, prompt_tokens, completion_tokens)
             VALUES ($1, $2, $3, 100, 200)",
        )
        .bind(member_id)
        .bind(family_id)
        .bind(message_id)
        .execute(&ts.state.pool)
        .await
        .expect("recording what a reply cost");
    }

    delete_ok(&ts, &member).await;

    // The reserved account is exactly as migration 0015 left it: same name,
    // no family, and alive.
    let assistant: (String, String, Option<i64>, Option<time::OffsetDateTime>) = sqlx::query_as(
        "SELECT username, display_name, family_id, deleted_at FROM users WHERE id = $1",
    )
    .bind(assistant_id)
    .fetch_one(&ts.state.pool)
    .await
    .expect("the assistant row");
    assert_eq!(assistant.0, "assistant");
    assert_eq!(assistant.1, "Assistant");
    assert_eq!(assistant.2, None, "the assistant belongs to no family");
    assert!(
        assistant.3.is_none(),
        "a member deleting their account must not scrub the assistant"
    );
    // And it is still reachable as the assistant — the lookup is by name
    // and a NULL family, which a tombstone shares half of.
    assert_eq!(
        family_connect::handlers_ai::assistant_user_id(&ts.state)
            .await
            .expect("the query runs"),
        Some(assistant_id)
    );

    // The private thread went with the account, like a direct chat.
    let threads: i64 =
        sqlx::query_scalar("SELECT count(*) FROM chats WHERE kind = 'ai' AND user_a_id = $1")
            .bind(member_id)
            .fetch_one(&ts.state.pool)
            .await
            .expect("counting the member's assistant threads");
    assert_eq!(threads, 0, "the private assistant thread goes the same way");

    // Both usage rows survive: the users row survives, so the CASCADE that
    // would have taken them never fires.
    let usage: Vec<(Option<i64>, i32, i32)> = sqlx::query_as(
        "SELECT message_id, prompt_tokens, completion_tokens FROM ai_usage
         WHERE user_id = $1 ORDER BY id",
    )
    .bind(member_id)
    .fetch_all(&ts.state.pool)
    .await
    .expect("reading the usage rows");
    assert_eq!(usage.len(), 2, "what the assistant cost is not the person");
    assert_eq!(
        usage[0],
        (None, 100, 200),
        "the message went, so the link is blanked — the cost is not"
    );
    assert_eq!(
        usage[1],
        (Some(public), 100, 200),
        "and a row naming a surviving message keeps naming it"
    );

    // Which is what the family's statistics are built on.
    let stats: Value = ts
        .get(&owner, "/families/mine/stats")
        .await
        .json()
        .await
        .expect("stats response is JSON");
    assert_eq!(
        stats["totals"]["ai"]["questions"], 2,
        "the family's spend is the family's, and does not shrink when \
         somebody leaves"
    );
    assert_eq!(stats["totals"]["ai"]["prompt_tokens"], 200);
    assert_eq!(stats["totals"]["ai"]["completion_tokens"], 400);
}

/// A deleted account is a member of NOTHING, so every endpoint that names a
/// member refuses it — and the refusals are the ordinary ones, because
/// `family_id IS NULL` is what makes a tombstone invisible rather than a
/// check each of them had to grow.
///
/// The sweep over every member-listing surface, so a future endpoint that
/// resolves a member by id has a precedent to match.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_tombstone_is_not_a_member_anywhere() {
    let ts = spawn_server().await;
    let (owner, _owner_id, member, member_id, _chat_id, _code) = family_of_two(&ts).await;

    delete_ok(&ts, &member).await;

    assert_error(
        ts.delete(&owner, &format!("/families/members/{member_id}"))
            .await,
        404,
        "user_not_found",
    )
    .await;
    assert_error(
        ts.post(
            &owner,
            &format!("/families/members/{member_id}/password"),
            json!({"new_password": "password456"}),
        )
        .await,
        403,
        "not_same_family",
    )
    .await;
    assert_error(
        ts.put(
            &owner,
            &format!("/families/members/{member_id}/birthday"),
            json!({"month": 3, "day": 14}),
        )
        .await,
        403,
        "not_same_family",
    )
    .await;
}

/// The statistics split the same way everything else does: the family's
/// totals still count the tombstone's MESSAGES — they are real messages in
/// the chat — while neither the member count nor the per-member breakdown
/// counts them as a member (protocol.md, "Deleting an account").
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn statistics_keep_the_messages_and_drop_the_member() {
    let ts = spawn_server().await;
    let (owner, _owner_id, member, _member_id, chat_id, _code) = family_of_two(&ts).await;
    send(&ts, &owner, chat_id, "one").await;
    send(&ts, &member, chat_id, "two").await;
    send(&ts, &member, chat_id, "three").await;

    delete_ok(&ts, &member).await;

    let stats: Value = ts
        .get(&owner, "/families/mine/stats")
        .await
        .json()
        .await
        .expect("stats response is JSON");
    assert_eq!(stats["totals"]["messages"], 3, "the words are still there");
    assert_eq!(stats["totals"]["members"], 1);
    let members = stats["members"].as_array().expect("members is an array");
    assert_eq!(members.len(), 1, "a tombstone is not a member");
}

/// The same file leak from the OTHER endpoint that can delete a family: the
/// sole member LEAVING takes it too, and used to strand every file it held.
///
/// This is the `leave_family` half of the collect-keys helper both paths
/// share, and nothing else exercises it — the deletion tests only ever
/// reach it through `POST /me/delete`.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_sole_member_leaving_takes_the_familys_files_with_it() {
    let ts = spawn_server().await;
    let (owner, _owner_id) = ts.register("owner", "Olive").await;
    ts.create_family(&owner, "The Smiths").await;
    let chat_id = ts.family_chat_id(&owner).await;

    let attachment_id = upload(&ts, &owner, jpeg_bytes(512, 0x00)).await;
    send_attachment(&ts, &owner, chat_id, attachment_id).await;
    let path = ts
        .state
        .storage
        .blob_path(&storage_key(&ts, attachment_id).await);
    assert!(path.exists());

    let response = ts.post(&owner, "/families/leave", json!({})).await;
    assert_eq!(response.status(), 204);

    assert!(
        !path.exists(),
        "the family's attachment outlived the family"
    );
}

/// An account with no family deletes just as cleanly: every branch that
/// names one is skipped, and a join request it was waiting on goes with it
/// — an owner must not be left approving somebody who no longer exists.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_account_outside_a_family_deletes_and_takes_its_join_request() {
    let ts = spawn_server().await;
    let (owner, _owner_id) = ts.register("owner", "Olive").await;
    let (_family_id, invite_code) = ts.create_family(&owner, "The Smiths").await;
    // The default policy is `approval`, so this waits rather than joining.
    let (waiting, waiting_id) = ts.register("waiting", "Waiting").await;
    ts.join(&waiting, &invite_code, "pending").await;

    delete_ok(&ts, &waiting).await;

    let requests: Value = ts
        .get(&owner, "/families/join-requests")
        .await
        .json()
        .await
        .expect("join requests response is JSON");
    assert_eq!(
        requests["requests"].as_array().expect("array").len(),
        0,
        "a request from an account that no longer exists is not pending"
    );
    // No family means no former membership to remember, and no
    // `former_members` key on a roster nobody has been deleted from.
    let deleted_family_id: Option<i64> =
        sqlx::query_scalar("SELECT deleted_family_id FROM users WHERE id = $1")
            .bind(waiting_id)
            .fetch_one(&ts.state.pool)
            .await
            .expect("the tombstone row");
    assert_eq!(deleted_family_id, None);
    let family = my_family(&ts, &owner).await;
    assert!(family["former_members"].is_null());
}
