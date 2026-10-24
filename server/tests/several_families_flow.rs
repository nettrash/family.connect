//! Integration: one account in several families (docs/protocol.md, "Several
//! families"; issue #88; decisions D1–D8 in docs/families-2026-10-22.md).
//!
//! Three things are under test throughout, because each is a way the change
//! could quietly break somebody:
//!
//! - **the doors** — a member creating another family, an owner's
//!   invitation, an existing account entering a code — each under its own
//!   rule and cap;
//! - **the scoping** — a family is its own world: its roster, chats, board,
//!   pack, assistant, uploads and reports never reach another family, and a
//!   client that does not say `X-FC-Families: 1` (or `families=1` on the
//!   socket) is answered about its HOME family exactly as before;
//! - **the pair** — a direct chat belongs to two people, listed in every
//!   family they share, shrinking with a `chat_updated` and closing when they
//!   share none.
//!
//! The live-socket helpers are copied from `ws_flow.rs` rather than shared,
//! for the reason `account_flow.rs` gives.

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

use family_connect::push_payload::PushEvent;

type WsClient = WebSocketStream<MaybeTlsStream<TcpStream>>;

const FRAME_WAIT: Duration = Duration::from_secs(5);

/// The header a multi-family client sends on every request.
const MULTI: (&str, &str) = ("X-FC-Families", "1");

/// Open an authenticated WebSocket — `multi` adds `families=1` — and wait
/// for a pong, which proves the connection is registered.
async fn connect_ws(ts: &TestServer, token: &str, multi: bool) -> WsClient {
    let url = if multi {
        format!("{}?families=1", ts.ws_url)
    } else {
        ts.ws_url.clone()
    };
    let mut request = url
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
    ws.send(Message::text(json!({"type": "ping"}).to_string()))
        .await
        .expect("sending a ping");
    next_frame_of_type(&mut ws, "pong").await;
    ws
}

/// Read frames until one of the wanted type arrives, skipping the rest.
async fn next_frame_of_type(ws: &mut WsClient, wanted: &str) -> Value {
    next_frame_where(ws, |frame| frame["type"] == wanted).await
}

async fn next_frame_where(ws: &mut WsClient, wanted: impl Fn(&Value) -> bool) -> Value {
    frames_until(ws, wanted)
        .await
        .pop()
        .expect("frames_until always ends on the match")
}

/// Every frame up to and including the first that matches — what a
/// "this did NOT arrive" assertion reads: the forbidden frame is sent first,
/// a probe that must arrive is sent after it, and everything in between is
/// in this list.
async fn frames_until(ws: &mut WsClient, wanted: impl Fn(&Value) -> bool) -> Vec<Value> {
    let deadline = tokio::time::Instant::now() + FRAME_WAIT;
    let mut seen = Vec::new();
    loop {
        let message = tokio::time::timeout_at(deadline, ws.next())
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for a frame; saw {seen:?}"))
            .expect("socket closed while waiting for a frame")
            .expect("socket errored while waiting for a frame");
        if let Message::Text(text) = message {
            let value: Value = serde_json::from_str(text.as_str()).expect("frames are JSON");
            let done = wanted(&value);
            seen.push(value);
            if done {
                return seen;
            }
        }
    }
}

async fn json_of(response: reqwest::Response, what: &str) -> Value {
    assert!(
        response.status().is_success(),
        "{what}: unexpected status {}",
        response.status()
    );
    response.json().await.expect("JSON body")
}

/// `GET /me` as a multi-family client, or as one from before.
async fn me(ts: &TestServer, token: &str, multi: bool) -> Value {
    let headers: &[(&str, &str)] = if multi { &[MULTI] } else { &[] };
    json_of(ts.get_with(token, "/me", headers).await, "GET /me").await
}

/// `GET /chats`, as the `chat` objects.
async fn chats(ts: &TestServer, token: &str, multi: bool) -> Vec<Value> {
    let headers: &[(&str, &str)] = if multi { &[MULTI] } else { &[] };
    let body = json_of(ts.get_with(token, "/chats", headers).await, "GET /chats").await;
    body["chats"]
        .as_array()
        .expect("chats is an array")
        .iter()
        .map(|entry| entry["chat"].clone())
        .collect()
}

/// The family chat of one family, by its `family_ids`.
async fn family_chat_of(ts: &TestServer, token: &str, family_id: i64) -> i64 {
    chats(ts, token, true)
        .await
        .into_iter()
        .find(|chat| chat["kind"] == "family" && chat["family_ids"] == json!([family_id]))
        .unwrap_or_else(|| panic!("the family chat of {family_id} is listed"))["id"]
        .as_i64()
        .expect("chat id")
}

fn family_ids_of(me: &Value) -> Vec<i64> {
    me["families"]
        .as_array()
        .expect("families is always present")
        .iter()
        .map(|family| family["id"].as_i64().expect("family id"))
        .collect()
}

/// Open a family's door by its id — `set_open_policy` patches the HOME
/// family, which is not the one a second family's owner means.
async fn open_family(ts: &TestServer, owner: &str, family_id: i64) {
    let response = ts
        .patch(
            owner,
            &format!("/families/{family_id}"),
            json!({"join_policy": "open"}),
        )
        .await;
    assert_eq!(response.status(), 200, "opening family {family_id}");
}

/// The invite code of a family its owner holds, by id.
async fn invite_code_of(ts: &TestServer, owner: &str, family_id: i64) -> String {
    let body = json_of(
        ts.get(owner, &format!("/families/{family_id}")).await,
        "reading a family",
    )
    .await;
    body["family"]["invite_code"]
        .as_str()
        .expect("the owner sees the invite code")
        .to_string()
}

async fn send(ts: &TestServer, token: &str, chat_id: i64, body: &str) -> i64 {
    let response = ts
        .post_message(token, chat_id, &Uuid::new_v4().to_string(), body)
        .await;
    assert_eq!(response.status(), 201, "posting a message");
    let value: Value = response.json().await.expect("message response is JSON");
    value["message"]["id"].as_i64().expect("message id")
}

async fn invite(ts: &TestServer, owner: &str, family_id: i64, user_id: i64) -> reqwest::Response {
    ts.post(
        owner,
        &format!("/families/{family_id}/invitations"),
        json!({"user_id": user_id}),
    )
    .await
}

async fn invite_ok(ts: &TestServer, owner: &str, family_id: i64, user_id: i64) -> i64 {
    let response = invite(ts, owner, family_id, user_id).await;
    assert_eq!(
        response.status(),
        201,
        "inviting {user_id} into {family_id}"
    );
    let body: Value = response.json().await.expect("JSON");
    body["invitation"]["id"].as_i64().expect("invitation id")
}

/// Mark every session `user_id` holds right now as one that knows about
/// several families (0058) — what a current app's first request with
/// `X-FC-Families: 1` does, written straight to the row so no test races the
/// fire-and-forget write. A session opened LATER, like an old phone's
/// `ts.login`, stays unmarked.
async fn as_multi_app(ts: &TestServer, user_id: i64) {
    sqlx::query("UPDATE sessions SET multi_family = true WHERE user_id = $1")
        .bind(user_id)
        .execute(&ts.state.pool)
        .await
        .expect("marking the sessions");
}

/// Two families: Olive owns the Smiths with Bob in it; Bob then starts the
/// Browns. Returns `(olive, olive_id, bob, bob_id, smiths, browns)`. Both
/// accounts are signed in on a current app.
async fn bob_in_two(ts: &TestServer) -> (String, i64, String, i64, i64, i64) {
    let (olive, olive_id) = ts.register("olive", "Olive").await;
    let (bob, bob_id) = ts.register("bob", "Bob").await;
    as_multi_app(ts, olive_id).await;
    as_multi_app(ts, bob_id).await;
    let (smiths, code) = ts.create_family(&olive, "The Smiths").await;
    ts.set_open_policy(&olive).await;
    ts.join(&bob, &code, "joined").await;
    let (browns, _) = ts.create_family(&bob, "The Browns").await;
    (olive, olive_id, bob, bob_id, smiths, browns)
}

// ---------------------------------------------------------------------------
// Creating another family
// ---------------------------------------------------------------------------

/// A member starts a family of their own from inside the app, stays in the
/// first, and the first stays HOME (docs/protocol.md, "Several families";
/// D3). They may create one only while they own none.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_member_starts_a_second_family_and_stays_in_the_first() {
    let ts = spawn_server().await;
    let (_olive, _, bob, _bob_id, smiths, browns) = bob_in_two(&ts).await;
    assert_ne!(smiths, browns);

    let body = me(&ts, &bob, true).await;
    assert_eq!(family_ids_of(&body), vec![smiths, browns], "home first");
    assert_eq!(
        body["family"]["id"].as_i64(),
        Some(smiths),
        "home is the first joined"
    );
    assert_eq!(body["role"], "member");
    assert_eq!(body["families"][0]["role"], "member");
    assert_eq!(body["families"][1]["role"], "owner");
    assert!(
        body["families"][1]["invite_code"].is_string(),
        "the owner's own family carries its code"
    );
    assert_eq!(body["member_families_enabled"], true);
    assert_eq!(body["max_families_per_account"], 5);
    assert_eq!(body["pending_join_requests"], json!([]));
    assert_eq!(body["invitations"], json!([]));

    // An app from before sees exactly what it always did: its home family.
    let old = me(&ts, &bob, false).await;
    assert_eq!(old["family"]["id"].as_i64(), Some(smiths));
    assert_eq!(old["role"], "member");

    // Owning one, a second is refused — ownership is the rule, not the count.
    assert_error(
        ts.post(&bob, "/families", json!({"name": "The Greens"}))
            .await,
        409,
        "already_own_a_family",
    )
    .await;
}

/// A member of a family on a server closed to strangers — the first family
/// seeded by hand, the way such an operator would have made it — and that
/// member's token.
async fn member_on_a_closed_server(ts: &TestServer) -> String {
    let (olive, _) = ts.register("olive", "Olive").await;
    assert_error(
        ts.post(&olive, "/families", json!({"name": "The Smiths"}))
            .await,
        403,
        "family_registration_disabled",
    )
    .await;
    let smiths: i64 = sqlx::query_scalar(
        "INSERT INTO families (name, invite_code, owner_user_id, join_policy)
         SELECT 'The Smiths', 'SEEDCODE', id, 'open' FROM users WHERE username = 'olive'
         RETURNING id",
    )
    .fetch_one(&ts.state.pool)
    .await
    .expect("seeding a family");
    sqlx::query("INSERT INTO chats (family_id, kind) VALUES ($1, 'family')")
        .bind(smiths)
        .execute(&ts.state.pool)
        .await
        .expect("its chat");
    let (bob, _) = ts.register("bob", "Bob").await;
    ts.join(&bob, "SEEDCODE", "joined").await;
    bob
}

/// The two operator switches are separate: `member_families` decides the
/// in-app door, and only when it is UNSET does it follow `registration` — so
/// a server closed to strangers is not reopened by an upgrade, and its
/// operator can still let members start families by saying so
/// (docs/protocol.md, "Several families").
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn in_app_creation_is_its_own_switch() {
    // Registration closed and nothing said about members: closed to them too.
    let ts = spawn_server_with_config(|cfg| cfg.families.registration = false).await;
    let bob = member_on_a_closed_server(&ts).await;
    assert_error(
        ts.post(&bob, "/families", json!({"name": "The Browns"}))
            .await,
        403,
        "member_families_disabled",
    )
    .await;
    assert_eq!(me(&ts, &bob, true).await["member_families_enabled"], false);

    // Registration closed, member families opened on purpose: the issue's
    // own case.
    let ts = spawn_server_with_config(|cfg| {
        cfg.families.registration = false;
        cfg.families.member_families = Some(true);
    })
    .await;
    let bob = member_on_a_closed_server(&ts).await;
    assert_eq!(me(&ts, &bob, true).await["member_families_enabled"], true);
    let response = ts
        .post(&bob, "/families", json!({"name": "The Browns"}))
        .await;
    assert_eq!(
        response.status(),
        201,
        "a member may start a family on a server closed to strangers"
    );

    // Member families closed: a member is refused, a newcomer is not.
    let ts = spawn_server_with_config(|cfg| cfg.families.member_families = Some(false)).await;
    let (olive, _) = ts.register("olive", "Olive").await;
    let (_, code) = ts.create_family(&olive, "The Smiths").await;
    ts.set_open_policy(&olive).await;
    let (bob, _) = ts.register("bob", "Bob").await;
    ts.join(&bob, &code, "joined").await;
    assert_error(
        ts.post(&bob, "/families", json!({"name": "The Browns"}))
            .await,
        403,
        "member_families_disabled",
    )
    .await;
    let body = me(&ts, &bob, true).await;
    assert_eq!(body["member_families_enabled"], false);
}

/// At most `max_families_per_account` families, by any door — creating and
/// joining with a code both read it (docs/protocol.md, "Limits").
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_per_account_cap_holds_at_every_door() {
    let ts = spawn_server_with_config(|cfg| cfg.families.max_families_per_account = 2).await;
    let (olive, _) = ts.register("olive", "Olive").await;
    let (petra, _) = ts.register("petra", "Petra").await;
    let (quinn, _) = ts.register("quinn", "Quinn").await;
    let (carl, _) = ts.register("carl", "Carl").await;
    let (_, olive_code) = ts.create_family(&olive, "The Smiths").await;
    ts.set_open_policy(&olive).await;
    let (_, petra_code) = ts.create_family(&petra, "The Parks").await;
    ts.set_open_policy(&petra).await;
    let (_, quinn_code) = ts.create_family(&quinn, "The Quinns").await;
    ts.set_open_policy(&quinn).await;

    // Carl joins two families with codes: at the cap, owning none.
    let joined = json_of(
        ts.post(&carl, "/families/join", json!({"invite_code": olive_code}))
            .await,
        "joining the Smiths",
    )
    .await;
    assert_eq!(joined["status"], "joined");
    assert!(
        joined["family_id"].is_i64(),
        "the joined family is named: {joined}"
    );
    ts.join(&carl, &petra_code, "joined").await;
    assert_error(
        ts.post(&carl, "/families", json!({"name": "The Carls"}))
            .await,
        409,
        "too_many_families",
    )
    .await;
    assert_error(
        ts.post(&carl, "/families/join", json!({"invite_code": quinn_code}))
            .await,
        409,
        "too_many_families",
    )
    .await;
    // Already in THAT family is its own answer, and comes first.
    assert_error(
        ts.post(&carl, "/families/join", json!({"invite_code": olive_code}))
            .await,
        409,
        "already_in_family",
    )
    .await;
}

// ---------------------------------------------------------------------------
// Joining with a code
// ---------------------------------------------------------------------------

/// An existing account enters another family's code under that family's own
/// rules (D5), may wait on several requests at once, and `/me` names the
/// OLDEST as `pending_join_request`.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_existing_account_joins_another_family_by_code_and_approval() {
    let ts = spawn_server().await;
    let (olive, _) = ts.register("olive", "Olive").await;
    let (bob, bob_id) = ts.register("bob", "Bob").await;
    let (carol, _) = ts.register("carol", "Carol").await;
    let (dan, _) = ts.register("dan", "Dan").await;
    let (smiths, olive_code) = ts.create_family(&olive, "The Smiths").await;
    ts.set_open_policy(&olive).await;
    ts.join(&bob, &olive_code, "joined").await;
    // Approval is the default policy.
    let (carols, carol_code) = ts.create_family(&carol, "The Carols").await;
    let (dans, dan_code) = ts.create_family(&dan, "The Dans").await;

    ts.join(&bob, &carol_code, "pending").await;
    // Both answers name the family the code belongs to (docs/protocol.md,
    // `POST /families/join`).
    let waiting = json_of(
        ts.post(&bob, "/families/join", json!({"invite_code": dan_code}))
            .await,
        "joining the Dans",
    )
    .await;
    assert_eq!(waiting["status"], "pending");
    assert_eq!(waiting["family_id"].as_i64(), Some(dans));
    let body = me(&ts, &bob, true).await;
    assert_eq!(body["family"]["id"].as_i64(), Some(smiths));
    let pending: Vec<i64> = body["pending_join_requests"]
        .as_array()
        .expect("always present")
        .iter()
        .map(|request| request["family_id"].as_i64().expect("family id"))
        .collect();
    assert_eq!(pending, vec![carols, dans]);
    assert_eq!(
        body["pending_join_request"]["family_id"].as_i64(),
        Some(dans),
        "the newest request, as an app from before was always shown"
    );

    // Carol approves; Bob is in three families, Dan's request untouched.
    let requests = json_of(ts.get(&carol, "/families/join-requests").await, "requests").await;
    let request_id = requests["requests"][0]["id"].as_i64().expect("request id");
    assert_eq!(requests["requests"][0]["user"]["id"].as_i64(), Some(bob_id));
    let response = ts
        .post(
            &carol,
            &format!("/families/join-requests/{request_id}/approve"),
            json!({}),
        )
        .await;
    assert_eq!(response.status(), 200);
    let body = me(&ts, &bob, true).await;
    assert_eq!(family_ids_of(&body), vec![smiths, carols]);
    assert_eq!(
        body["pending_join_requests"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(
        body["pending_join_request"]["family_id"].as_i64(),
        Some(dans)
    );
}

// ---------------------------------------------------------------------------
// Invitations
// ---------------------------------------------------------------------------

/// The whole life of an invitation: who may be invited, the frames to the
/// two parties and nobody else, the push, decline, accept — and the four
/// ways it lapses (docs/protocol.md, "Several families"; D4).
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_invitation_is_offered_answered_and_lapses() {
    let ts = spawn_server().await;
    let (olive, olive_id, bob, bob_id, smiths, browns) = bob_in_two(&ts).await;
    let (stranger, stranger_id) = ts.register("stranger", "Stranger").await;
    let smiths_code = invite_code_of(&ts, &olive, smiths).await;
    let _ = stranger;
    // Her phone signs in on a session of its own, so the sockets opened
    // below (on her first session) do not count as the phone looking.
    let phone = ts.login("olive", "password123").await;
    let device = ts
        .post(
            &phone,
            "/devices",
            json!({"platform": "ios", "push_token": "olive-phone"}),
        )
        .await;
    assert_eq!(device.status(), 201);

    // Only people from the owner's OTHER families, and an unknown id answers
    // exactly as a stranger does.
    assert_error(
        invite(&ts, &bob, browns, stranger_id).await,
        403,
        "not_same_family",
    )
    .await;
    assert_error(
        invite(&ts, &bob, browns, 999_999).await,
        403,
        "not_same_family",
    )
    .await;
    assert_error(
        invite(&ts, &bob, browns, bob_id).await,
        409,
        "user_already_in_family",
    )
    .await;
    // Only the owner, and only of a family the caller is in.
    assert_error(
        invite(&ts, &olive, browns, bob_id).await,
        404,
        "family_not_found",
    )
    .await;
    assert_error(
        invite(&ts, &bob, smiths, olive_id).await,
        403,
        "not_family_owner",
    )
    .await;

    let mut olive_multi = connect_ws(&ts, &olive, true).await;
    let mut olive_old = connect_ws(&ts, &olive, false).await;
    let mut bob_multi = connect_ws(&ts, &bob, true).await;

    let invitation_id = invite_ok(&ts, &bob, browns, olive_id).await;
    // Twice is the same invitation, 200.
    let again = invite(&ts, &bob, browns, olive_id).await;
    assert_eq!(again.status(), 200);
    let again: Value = again.json().await.expect("JSON");
    assert_eq!(again["invitation"]["id"].as_i64(), Some(invitation_id));
    assert_eq!(again["invitation"]["family_name"], "The Browns");
    assert_eq!(
        again["invitation"]["invited_by"]["id"].as_i64(),
        Some(bob_id)
    );
    assert_eq!(again["invitation"]["user"]["id"].as_i64(), Some(olive_id));

    let frame = next_frame_of_type(&mut olive_multi, "invitation").await;
    assert_eq!(frame["invitation"]["id"].as_i64(), Some(invitation_id));
    let frame = next_frame_of_type(&mut bob_multi, "invitation").await;
    assert_eq!(frame["invitation"]["family_id"].as_i64(), Some(browns));

    // The push, to the invited person: the family's name over "<Owner>
    // invited you", with both ids.
    let deadline = tokio::time::Instant::now() + FRAME_WAIT;
    let push = loop {
        if let Some(call) = ts
            .push
            .calls()
            .into_iter()
            .find(|call| matches!(call.note.event, PushEvent::Invitation { .. }))
        {
            break call;
        }
        assert!(tokio::time::Instant::now() < deadline, "no invitation push");
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(push.note.title, "The Browns");
    assert_eq!(push.note.body, "Bob invited you");
    assert_eq!(
        push.note.event,
        PushEvent::Invitation {
            family_id: browns,
            invitation_id
        }
    );

    // Both lists hold it.
    let olive_me = me(&ts, &olive, true).await;
    assert_eq!(
        olive_me["invitations"][0]["id"].as_i64(),
        Some(invitation_id)
    );
    let list = json_of(
        ts.get(&bob, &format!("/families/{browns}/invitations"))
            .await,
        "the owner's list",
    )
    .await;
    assert_eq!(list["invitations"][0]["id"].as_i64(), Some(invitation_id));

    // Decline: gone from both, frames to both, idempotent, never accepted.
    let declined = ts
        .post(
            &olive,
            &format!("/invitations/{invitation_id}/decline"),
            json!({}),
        )
        .await;
    assert_eq!(declined.status(), 204);
    let frame = next_frame_of_type(&mut olive_multi, "invitation_removed").await;
    assert_eq!(frame["invitation_id"].as_i64(), Some(invitation_id));
    next_frame_of_type(&mut bob_multi, "invitation_removed").await;
    let declined = ts
        .post(
            &olive,
            &format!("/invitations/{invitation_id}/decline"),
            json!({}),
        )
        .await;
    assert_eq!(declined.status(), 204);
    assert_error(
        ts.post(
            &olive,
            &format!("/invitations/{invitation_id}/accept"),
            json!({}),
        )
        .await,
        409,
        "invitation_not_pending",
    )
    .await;

    // The app from before heard none of it: the next frame it gets is the
    // probe, a message in its home family's chat.
    let smiths_chat = family_chat_of(&ts, &olive, smiths).await;
    send(&ts, &bob, smiths_chat, "probe").await;
    let seen = frames_until(&mut olive_old, |frame| frame["type"] == "message").await;
    assert!(
        seen.iter().all(|frame| !frame["type"]
            .as_str()
            .unwrap_or("")
            .starts_with("invitation")),
        "an app from before is never sent an invitation frame: {seen:?}"
    );

    // Asked again, and accepted: immediate, whatever the policy (Browns is
    // `approval`), the family comes back, and everyone in it is told.
    let invitation_id = invite_ok(&ts, &bob, browns, olive_id).await;
    let accepted = ts
        .post(
            &olive,
            &format!("/invitations/{invitation_id}/accept"),
            json!({}),
        )
        .await;
    assert_eq!(accepted.status(), 200);
    let accepted: Value = accepted.json().await.expect("JSON");
    assert_eq!(accepted["family"]["id"].as_i64(), Some(browns));
    assert!(
        accepted["family"].get("invite_code").is_none(),
        "Olive is a member of the Browns, not its owner"
    );
    let joined = next_frame_where(&mut bob_multi, |frame| {
        frame["type"] == "member_joined" && frame["family_id"].as_i64() == Some(browns)
    })
    .await;
    assert_eq!(joined["user"]["id"].as_i64(), Some(olive_id));
    assert_eq!(
        family_ids_of(&me(&ts, &olive, true).await),
        vec![smiths, browns]
    );
    let list = json_of(
        ts.get(&bob, &format!("/families/{browns}/invitations"))
            .await,
        "the owner's list",
    )
    .await;
    assert_eq!(list["invitations"], json!([]));

    // Withdrawn by the owner.
    let (carl, carl_id) = ts.register("carl", "Carl").await;
    ts.join(&carl, &smiths_code, "joined").await;
    let withdrawn_id = invite_ok(&ts, &bob, browns, carl_id).await;
    let response = ts
        .delete(
            &bob,
            &format!("/families/{browns}/invitations/{withdrawn_id}"),
        )
        .await;
    assert_eq!(response.status(), 204);
    assert_eq!(me(&ts, &carl, true).await["invitations"], json!([]));
    let response = ts
        .delete(
            &bob,
            &format!("/families/{browns}/invitations/{withdrawn_id}"),
        )
        .await;
    assert_eq!(response.status(), 204, "withdrawing is idempotent");

    // Lapsed by another door: Carl comes in with the code instead.
    let lapsing_id = invite_ok(&ts, &bob, browns, carl_id).await;
    open_family(&ts, &bob, browns).await;
    let browns_code = invite_code_of(&ts, &bob, browns).await;
    let mut carl_multi = connect_ws(&ts, &carl, true).await;
    ts.join(&carl, &browns_code, "joined").await;
    let frame = next_frame_of_type(&mut carl_multi, "invitation_removed").await;
    assert_eq!(frame["invitation_id"].as_i64(), Some(lapsing_id));
    assert_eq!(me(&ts, &carl, true).await["invitations"], json!([]));

    // Lapsed by the invited person's account going.
    let (dora, dora_id) = ts.register("dora", "Dora").await;
    ts.join(&dora, &smiths_code, "joined").await;
    invite_ok(&ts, &bob, browns, dora_id).await;
    let response = ts
        .post(&dora, "/me/delete", json!({"password": "password123"}))
        .await;
    assert_eq!(response.status(), 204);
    let list = json_of(
        ts.get(&bob, &format!("/families/{browns}/invitations"))
            .await,
        "the owner's list",
    )
    .await;
    assert_eq!(
        list["invitations"],
        json!([]),
        "the deleted account's invitation lapsed"
    );
}

/// The family going takes its invitations with it, and the invited person is
/// told.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_deleted_family_takes_its_invitations_with_it() {
    let ts = spawn_server().await;
    let (olive, olive_id, bob, _bob_id, _smiths, browns) = bob_in_two(&ts).await;
    let invitation_id = invite_ok(&ts, &bob, browns, olive_id).await;
    let mut olive_multi = connect_ws(&ts, &olive, true).await;
    // Bob is the Browns' only member; leaving deletes it.
    let response = ts
        .post(&bob, &format!("/families/{browns}/leave"), json!({}))
        .await;
    assert_eq!(response.status(), 204);
    let frame = next_frame_of_type(&mut olive_multi, "invitation_removed").await;
    assert_eq!(frame["invitation_id"].as_i64(), Some(invitation_id));
    assert_eq!(me(&ts, &olive, true).await["invitations"], json!([]));
}

// ---------------------------------------------------------------------------
// The home family, for apps from before
// ---------------------------------------------------------------------------

/// Without the header and without `families=1`, a client is answered about
/// its HOME family alone — `/chats`, and every frame — and with them it gets
/// everything, each chat saying whose it is (docs/protocol.md, "Several
/// families"; D8).
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_app_from_before_sees_its_home_family_and_nothing_else() {
    let ts = spawn_server().await;
    let (olive, _, bob, _, smiths, browns) = bob_in_two(&ts).await;

    let old_list = chats(&ts, &bob, false).await;
    assert!(
        old_list
            .iter()
            .all(|chat| chat["family_ids"] == json!([smiths])),
        "only the home family's chats: {old_list:?}"
    );
    assert_eq!(old_list.len(), 1, "the Smiths' family chat");
    let new_list = chats(&ts, &bob, true).await;
    let mut families: Vec<Value> = new_list
        .iter()
        .map(|chat| chat["family_ids"].clone())
        .collect();
    families.sort_by_key(|ids| ids[0].as_i64());
    assert_eq!(families, vec![json!([smiths]), json!([browns])]);

    let smiths_chat = family_chat_of(&ts, &bob, smiths).await;
    let browns_chat = family_chat_of(&ts, &bob, browns).await;
    let mut old = connect_ws(&ts, &bob, false).await;
    let mut new = connect_ws(&ts, &bob, true).await;

    // A message and a board note in the Browns, then a probe in the Smiths.
    let browns_message = send(&ts, &bob, browns_chat, "only the Browns").await;
    let note = ts
        .post(
            &bob,
            &format!("/families/{browns}/board/notes"),
            json!({"text": "Browns' milk", "color": "yellow", "x": 0.1, "y": 0.1}),
        )
        .await;
    assert_eq!(note.status(), 201);
    let note: Value = note.json().await.expect("JSON");
    assert_eq!(
        note["note"]["family_id"].as_i64(),
        Some(browns),
        "a note says whose it is"
    );
    let probe = send(&ts, &olive, smiths_chat, "the Smiths").await;

    let seen = frames_until(&mut old, |frame| {
        frame["type"] == "message" && frame["message"]["id"].as_i64() == Some(probe)
    })
    .await;
    assert!(
        seen.iter().all(|frame| frame["type"] != "board_note"
            && frame["message"]["id"].as_i64() != Some(browns_message)),
        "nothing of the Browns reaches an app from before: {seen:?}"
    );
    // The multi-family socket got all three.
    let seen = frames_until(&mut new, |frame| {
        frame["type"] == "message" && frame["message"]["id"].as_i64() == Some(probe)
    })
    .await;
    assert!(seen.iter().any(|frame| frame["type"] == "board_note"));
    assert!(
        seen.iter()
            .any(|frame| frame["message"]["id"].as_i64() == Some(browns_message))
    );
}

// ---------------------------------------------------------------------------
// Direct chats
// ---------------------------------------------------------------------------

/// A direct chat is listed in every family the pair shares, shrinks with a
/// `chat_updated` when they stop sharing one, closes when they share none —
/// and comes back with its history when they share one again
/// (docs/protocol.md, "Several families").
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_direct_chat_follows_the_families_the_pair_shares() {
    let ts = spawn_server().await;
    let (olive, olive_id, bob, bob_id, smiths, browns) = bob_in_two(&ts).await;
    let invitation_id = invite_ok(&ts, &bob, browns, olive_id).await;
    let response = ts
        .post(
            &olive,
            &format!("/invitations/{invitation_id}/accept"),
            json!({}),
        )
        .await;
    assert_eq!(response.status(), 200);

    let opened = json_of(
        ts.post(&olive, "/chats/direct", json!({"user_id": bob_id}))
            .await,
        "opening a direct chat",
    )
    .await;
    let direct = opened["chat"]["id"].as_i64().expect("chat id");
    assert_eq!(opened["chat"]["family_ids"], json!([smiths, browns]));
    send(&ts, &olive, direct, "see you Sunday").await;

    let mut olive_ws = connect_ws(&ts, &olive, true).await;
    let mut bob_ws = connect_ws(&ts, &bob, true).await;

    // Olive leaves the Browns: still shared through the Smiths.
    let response = ts
        .post(&olive, &format!("/families/{browns}/leave"), json!({}))
        .await;
    assert_eq!(response.status(), 204);
    for ws in [&mut olive_ws, &mut bob_ws] {
        let updated = next_frame_where(ws, |frame| {
            frame["type"] == "chat_updated" && frame["chat"]["id"].as_i64() == Some(direct)
        })
        .await;
        assert_eq!(updated["chat"]["family_ids"], json!([smiths]));
    }
    // Each side reads the chat as the OTHER person.
    let olive_view = chats(&ts, &olive, true)
        .await
        .into_iter()
        .find(|chat| chat["id"].as_i64() == Some(direct))
        .expect("still listed");
    assert_eq!(olive_view["peer_user_id"].as_i64(), Some(bob_id));
    assert_eq!(olive_view["family_ids"], json!([smiths]));

    // Bob leaves the Smiths — the last family they shared. Bob still has
    // the Browns, so this is not his last family.
    let response = ts
        .post(&bob, &format!("/families/{smiths}/leave"), json!({}))
        .await;
    assert_eq!(response.status(), 204);
    // Closed: shared nowhere. The side that left gets `[]`; the side that
    // stayed still holds the chat, under a family of theirs — the Smiths,
    // which it was last listed in — so an app sorting by `family_ids` does
    // not lose it.
    let updated = next_frame_where(&mut bob_ws, |frame| {
        frame["type"] == "chat_updated" && frame["chat"]["id"].as_i64() == Some(direct)
    })
    .await;
    assert_eq!(
        updated["chat"]["family_ids"],
        json!([]),
        "the side that left"
    );
    let updated = next_frame_where(&mut olive_ws, |frame| {
        frame["type"] == "chat_updated" && frame["chat"]["id"].as_i64() == Some(direct)
    })
    .await;
    assert_eq!(
        updated["chat"]["family_ids"],
        json!([smiths]),
        "the side that stayed"
    );
    let stayed = chats(&ts, &olive, true)
        .await
        .into_iter()
        .find(|chat| chat["id"].as_i64() == Some(direct))
        .expect("still held on the side that stayed");
    assert_eq!(stayed["family_ids"], json!([smiths]));
    assert!(
        chats(&ts, &bob, true)
            .await
            .iter()
            .all(|chat| chat["id"].as_i64() != Some(direct)),
        "closed on the side that left"
    );
    assert_error(
        ts.post(&bob, "/chats/direct", json!({"user_id": olive_id}))
            .await,
        409,
        "not_same_family",
    )
    .await;

    // Bob comes back into the Smiths: the chat returns, history and all.
    let code = invite_code_of(&ts, &olive, smiths).await;
    ts.join(&bob, &code, "joined").await;
    let back = chats(&ts, &bob, true)
        .await
        .into_iter()
        .find(|chat| chat["id"].as_i64() == Some(direct))
        .expect("the chat comes back when the two share a family again");
    assert_eq!(back["family_ids"], json!([smiths]));
    let messages = json_of(
        ts.get(&bob, &format!("/chats/{direct}/messages")).await,
        "the history",
    )
    .await;
    assert_eq!(messages["messages"][0]["body"], "see you Sunday");
}

/// A direct chat the other side has left is still the stayer's to write in,
/// attachments included; and when the two share a family again — a different
/// one — the row moves onto it, so nothing read off the chat (a transcript's
/// switch, language and statistics) goes on naming a family one of them left.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_closed_direct_chat_takes_uploads_and_reopens_under_the_shared_family() {
    let ts = spawn_server().await;
    let (olive, _, bob, bob_id, smiths, browns) = bob_in_two(&ts).await;
    let opened = json_of(
        ts.post(&olive, "/chats/direct", json!({"user_id": bob_id}))
            .await,
        "opening a direct chat",
    )
    .await;
    let direct = opened["chat"]["id"].as_i64().expect("chat id");

    // Bob leaves the Smiths, the one family the two shared.
    let response = ts
        .post(&bob, &format!("/families/{smiths}/leave"), json!({}))
        .await;
    assert_eq!(response.status(), 204);

    // Olive stayed: a photo goes where a text does.
    let photo = attachment_id(upload_for(&ts, &olive, Some(smiths)).await).await;
    let response = ts
        .post(
            &olive,
            &format!("/chats/{direct}/messages"),
            json!({"client_msg_id": Uuid::new_v4().to_string(), "body": "",
                   "attachment_ids": [photo]}),
        )
        .await;
    assert_eq!(
        response.status(),
        201,
        "a photo in the chat the stayer holds"
    );

    // They meet again in the Browns — by its code, since an invitation needs
    // a family the two already share.
    open_family(&ts, &bob, browns).await;
    let code = invite_code_of(&ts, &bob, browns).await;
    ts.join(&olive, &code, "joined").await;
    let row_family: i64 = sqlx::query_scalar("SELECT family_id FROM chats WHERE id = $1")
        .bind(direct)
        .fetch_one(&ts.state.pool)
        .await
        .expect("the chat row");
    assert_eq!(row_family, browns, "the row names a family the two share");
    let back = chats(&ts, &bob, true)
        .await
        .into_iter()
        .find(|chat| chat["id"].as_i64() == Some(direct))
        .expect("reopened");
    assert_eq!(back["family_ids"], json!([browns]));
}

/// A chat held by the side that stayed moves with them when they leave the
/// family its row names, so that family's later deletion cannot take it —
/// and the last member deleting a family by leaving it tells their own other
/// devices, as any departure does.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_held_chat_outlives_the_family_it_was_opened_in() {
    let ts = spawn_server().await;
    let (olive, _, bob, _, smiths, browns) = bob_in_two(&ts).await;
    let (carl, carl_id) = ts.register("carl", "Carl").await;
    let smiths_code = invite_code_of(&ts, &olive, smiths).await;
    ts.join(&carl, &smiths_code, "joined").await;
    let opened = json_of(
        ts.post(&bob, "/chats/direct", json!({"user_id": carl_id}))
            .await,
        "opening a direct chat",
    )
    .await;
    let direct = opened["chat"]["id"].as_i64().expect("chat id");
    send(&ts, &bob, direct, "keep this").await;

    // Carl leaves: closed on his side, Bob holds it. Then Bob leaves the
    // Smiths too, and Olive — the last — deletes them by leaving.
    for (token, path) in [
        (&carl, format!("/families/{smiths}/leave")),
        (&bob, format!("/families/{smiths}/leave")),
    ] {
        let response = ts.post(token, &path, json!({})).await;
        assert_eq!(response.status(), 204);
    }
    let response = ts
        .post(&olive, &format!("/families/{smiths}/leave"), json!({}))
        .await;
    assert_eq!(response.status(), 204);
    let kept = chats(&ts, &bob, true)
        .await
        .into_iter()
        .find(|chat| chat["id"].as_i64() == Some(direct))
        .expect("the chat outlives the Smiths");
    assert_eq!(kept["family_ids"], json!([browns]));
    let messages = json_of(
        ts.get(&bob, &format!("/chats/{direct}/messages")).await,
        "the history",
    )
    .await;
    assert_eq!(messages["messages"][0]["body"], "keep this");

    // Bob joins the Dans, then leaves the Browns, where he is the only one:
    // his other device hears the Browns are gone, and the chat moves again.
    let (dan, _) = ts.register("dan", "Dan").await;
    let (dans, _) = ts.create_family(&dan, "The Dans").await;
    open_family(&ts, &dan, dans).await;
    let dans_code = invite_code_of(&ts, &dan, dans).await;
    ts.join(&bob, &dans_code, "joined").await;
    let mut other_device = connect_ws(&ts, &bob, true).await;
    let response = ts
        .post(&bob, &format!("/families/{browns}/leave"), json!({}))
        .await;
    assert_eq!(response.status(), 204);
    let left = next_frame_of_type(&mut other_device, "member_left").await;
    assert_eq!(left["family_id"].as_i64(), Some(browns));
    let kept = chats(&ts, &bob, true)
        .await
        .into_iter()
        .find(|chat| chat["id"].as_i64() == Some(direct))
        .expect("the chat outlives the Browns");
    assert_eq!(kept["family_ids"], json!([dans]));
}

/// A held chat whose row names a family its holder is not in — what two
/// departures of one account racing each other can leave behind — is put
/// right by that account's next departure, once it has committed.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_held_chat_left_adrift_is_put_right_by_the_next_departure() {
    let ts = spawn_server().await;
    let (olive, _, bob, _, smiths, _browns) = bob_in_two(&ts).await;
    let (carl, carl_id) = ts.register("carl", "Carl").await;
    let smiths_code = invite_code_of(&ts, &olive, smiths).await;
    ts.join(&carl, &smiths_code, "joined").await;
    let opened = json_of(
        ts.post(&bob, "/chats/direct", json!({"user_id": carl_id}))
            .await,
        "opening a direct chat",
    )
    .await;
    let direct = opened["chat"]["id"].as_i64().expect("chat id");
    let response = ts
        .post(&carl, &format!("/families/{smiths}/leave"), json!({}))
        .await;
    assert_eq!(response.status(), 204);

    // Adrift: the row names a family Bob is not in.
    let (dan, _) = ts.register("dan", "Dan").await;
    let (dans, _) = ts.create_family(&dan, "The Dans").await;
    sqlx::query("UPDATE chats SET family_id = $2 WHERE id = $1")
        .bind(direct)
        .bind(dans)
        .execute(&ts.state.pool)
        .await
        .expect("setting the chat adrift");

    // Bob comes and goes through some other family.
    open_family(&ts, &dan, dans).await;
    let dans_code = invite_code_of(&ts, &dan, dans).await;
    ts.join(&bob, &dans_code, "joined").await;
    let mut bob_ws = connect_ws(&ts, &bob, true).await;
    let response = ts
        .post(&bob, &format!("/families/{dans}/leave"), json!({}))
        .await;
    assert_eq!(response.status(), 204);
    let row_family: i64 = sqlx::query_scalar("SELECT family_id FROM chats WHERE id = $1")
        .bind(direct)
        .fetch_one(&ts.state.pool)
        .await
        .expect("the chat row");
    assert_eq!(row_family, smiths, "back on Bob's home family");
    let updated = next_frame_where(&mut bob_ws, |frame| {
        frame["type"] == "chat_updated" && frame["chat"]["id"].as_i64() == Some(direct)
    })
    .await;
    assert_eq!(updated["chat"]["family_ids"], json!([smiths]));
}

/// The hourly backstop: an invitation whose person and family owner no
/// longer share another family — one a departure could not see — lapses, and
/// a valid one is left alone.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_invitation_nobody_shares_a_family_for_is_swept() {
    let ts = spawn_server().await;
    let (olive, olive_id, bob, _, smiths, browns) = bob_in_two(&ts).await;
    let (carl, carl_id) = ts.register("carl", "Carl").await;
    let smiths_code = invite_code_of(&ts, &olive, smiths).await;
    ts.join(&carl, &smiths_code, "joined").await;
    let unshared = invite_ok(&ts, &bob, browns, carl_id).await;
    let valid = invite_ok(&ts, &bob, browns, olive_id).await;
    // Carl out of the Smiths behind every departure's back.
    sqlx::query("DELETE FROM family_members WHERE family_id = $1 AND user_id = $2")
        .bind(smiths)
        .bind(carl_id)
        .execute(&ts.state.pool)
        .await
        .expect("unsharing the pair");
    let mut bob_ws = connect_ws(&ts, &bob, true).await;

    let swept = family_connect::handlers_family::sweep_unshared_invitations(&ts.state)
        .await
        .expect("the sweep runs");
    assert_eq!(swept, 1);
    let removed = next_frame_of_type(&mut bob_ws, "invitation_removed").await;
    assert_eq!(removed["invitation_id"].as_i64(), Some(unshared));
    let list = json_of(
        ts.get(&bob, &format!("/families/{browns}/invitations"))
            .await,
        "the owner's list",
    )
    .await;
    let ids: Vec<i64> = list["invitations"]
        .as_array()
        .expect("invitations")
        .iter()
        .map(|invitation| invitation["id"].as_i64().expect("id"))
        .collect();
    assert_eq!(ids, vec![valid]);
    let _ = carl;
}

/// An app from before several families is not sent what happens in a closed
/// direct chat that its account holds under a family other than home —
/// `GET /chats` does not list it there, so neither do the frames.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_held_chat_outside_home_stays_off_an_app_from_before() {
    let ts = spawn_server().await;
    let (_olive, _, bob, _, smiths, browns) = bob_in_two(&ts).await;
    let (carl, carl_id) = ts.register("carl", "Carl").await;
    open_family(&ts, &bob, browns).await;
    let browns_code = invite_code_of(&ts, &bob, browns).await;
    ts.join(&carl, &browns_code, "joined").await;
    let opened = json_of(
        ts.post(&bob, "/chats/direct", json!({"user_id": carl_id}))
            .await,
        "opening a direct chat",
    )
    .await;
    let direct = opened["chat"]["id"].as_i64().expect("chat id");
    let response = ts
        .post(&carl, &format!("/families/{browns}/leave"), json!({}))
        .await;
    assert_eq!(response.status(), 204);

    let old_phone = ts.login("bob", "password123").await;
    assert!(
        chats(&ts, &old_phone, false)
            .await
            .iter()
            .all(|chat| chat["id"].as_i64() != Some(direct)),
        "not listed to the old app"
    );
    let mut old_socket = connect_ws(&ts, &old_phone, false).await;
    send(&ts, &bob, direct, "a note to myself").await;
    let home_chat = family_chat_of(&ts, &bob, smiths).await;
    let probe = send(&ts, &bob, home_chat, "probe").await;
    let seen = frames_until(&mut old_socket, |frame| {
        frame["type"] == "message" && frame["message"]["id"].as_i64() == Some(probe)
    })
    .await;
    assert!(
        seen.iter()
            .all(|frame| frame["message"]["chat_id"].as_i64() != Some(direct)),
        "nothing from the held chat reached the old app: {seen:?}"
    );
}

/// Opening a direct chat needs ONE shared family, whichever it is.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_direct_chat_needs_one_shared_family_not_the_home_one() {
    let ts = spawn_server().await;
    let (_olive, _, bob, _, _smiths, browns) = bob_in_two(&ts).await;
    let (carl, carl_id) = ts.register("carl", "Carl").await;
    open_family(&ts, &bob, browns).await;
    let code = invite_code_of(&ts, &bob, browns).await;
    ts.join(&carl, &code, "joined").await;
    let opened = json_of(
        ts.post(&bob, "/chats/direct", json!({"user_id": carl_id}))
            .await,
        "opening a direct chat through the Browns",
    )
    .await;
    assert_eq!(opened["chat"]["family_ids"], json!([browns]));
    // Bob's app from before does not list it: the Smiths are its world.
    let direct = opened["chat"]["id"].as_i64();
    assert!(
        chats(&ts, &bob, false)
            .await
            .iter()
            .all(|chat| chat["id"].as_i64() != direct)
    );
    let _ = carl;
}

// ---------------------------------------------------------------------------
// Leaving, and the familyless clock
// ---------------------------------------------------------------------------

async fn familyless_since(ts: &TestServer, user_id: i64) -> Option<time::OffsetDateTime> {
    sqlx::query_scalar("SELECT familyless_since FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&ts.state.pool)
        .await
        .expect("the account row")
}

/// Leaving one family leaves that one; the familyless clock starts only at
/// the LAST; a pending invitation spares the account from the sweep like a
/// pending join request does (docs/protocol.md, "Several families").
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn leaving_one_family_keeps_the_others_and_the_sweep_waits_for_the_last() {
    let ts = spawn_server().await;
    let (olive, _, bob, bob_id, smiths, browns) = bob_in_two(&ts).await;
    let (carl, carl_id) = ts.register("carl", "Carl").await;
    let smiths_code = invite_code_of(&ts, &olive, smiths).await;
    ts.join(&carl, &smiths_code, "joined").await;
    open_family(&ts, &bob, browns).await;
    let browns_code = invite_code_of(&ts, &bob, browns).await;
    ts.join(&carl, &browns_code, "joined").await;

    // Carl leaves the Browns by id: still in the Smiths, clock stopped.
    let response = ts
        .post(&carl, &format!("/families/{browns}/leave"), json!({}))
        .await;
    assert_eq!(response.status(), 204);
    assert_eq!(family_ids_of(&me(&ts, &carl, true).await), vec![smiths]);
    assert_eq!(familyless_since(&ts, carl_id).await, None);

    // Invited back into the Browns, then out of his last family — the
    // Smiths, the one family he shared with Bob. The invitation lapses with
    // it: Bob may invite only somebody he shares another family with.
    let invitation_id = invite_ok(&ts, &bob, browns, carl_id).await;
    let mut bob_ws = connect_ws(&ts, &bob, true).await;
    let response = ts.post(&carl, "/families/leave", json!({})).await;
    assert_eq!(response.status(), 204);
    assert!(
        familyless_since(&ts, carl_id).await.is_some(),
        "the clock starts at the last"
    );
    let removed = next_frame_of_type(&mut bob_ws, "invitation_removed").await;
    assert_eq!(removed["invitation_id"].as_i64(), Some(invitation_id));
    assert_eq!(
        me(&ts, &carl, true).await["invitations"],
        json!([]),
        "nobody shares a family with him any more"
    );
    assert_error(
        ts.post(
            &carl,
            &format!("/invitations/{invitation_id}/accept"),
            json!({}),
        )
        .await,
        409,
        "invitation_not_pending",
    )
    .await;

    // Long past the grace — and spared by a join request still waiting.
    let (dan, _) = ts.register("dan", "Dan").await;
    let (_, dans_code) = ts.create_family(&dan, "The Dans").await;
    ts.join(&carl, &dans_code, "pending").await;
    sqlx::query("UPDATE users SET familyless_since = now() - interval '30 days' WHERE id = $1")
        .bind(carl_id)
        .execute(&ts.state.pool)
        .await
        .expect("ageing the clock");
    let swept = family_connect::handlers_auth::sweep_familyless_accounts(&ts.state)
        .await
        .expect("the sweep runs");
    assert_eq!(swept, 0, "a pending join request keeps the sweep away");
    let requests = json_of(ts.get(&dan, "/families/join-requests").await, "requests").await;
    let request_id = requests["requests"][0]["id"].as_i64().expect("request id");
    let response = ts
        .post(
            &dan,
            &format!("/families/join-requests/{request_id}/reject"),
            json!({}),
        )
        .await;
    assert!(response.status().is_success(), "rejecting the request");
    let swept = family_connect::handlers_auth::sweep_familyless_accounts(&ts.state)
        .await
        .expect("the sweep runs");
    assert_eq!(swept, 1, "with nothing pending, the account goes");

    // Bob, owner of the Browns, leaves the Smiths: he keeps his own family
    // and the Smiths keep their owner.
    let response = ts.post(&bob, "/families/leave", json!({})).await;
    assert_eq!(response.status(), 204);
    let body = me(&ts, &bob, true).await;
    assert_eq!(
        family_ids_of(&body),
        vec![browns],
        "the next family became home"
    );
    assert_eq!(body["family"]["id"].as_i64(), Some(browns));
    assert_eq!(body["role"], "owner");
    assert_eq!(familyless_since(&ts, bob_id).await, None);
}

// ---------------------------------------------------------------------------
// Owners' powers
// ---------------------------------------------------------------------------

/// An owner may reset the password only of a member ALL of whose families
/// they own, and the roster says so to the owner alone (D7).
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_password_reset_needs_every_family_of_the_member() {
    let ts = spawn_server().await;
    let (olive, olive_id, bob, bob_id, smiths, _browns) = bob_in_two(&ts).await;
    let (kid, kid_id) = ts.register("kid", "Kid").await;
    let code = invite_code_of(&ts, &olive, smiths).await;
    ts.join(&kid, &code, "joined").await;

    let roster = json_of(ts.get(&olive, "/families/mine").await, "roster").await;
    let flag = |id: i64| {
        roster["members"]
            .as_array()
            .expect("members")
            .iter()
            .find(|member| member["id"].as_i64() == Some(id))
            .expect("listed")
            .get("password_reset")
            .cloned()
    };
    assert_eq!(flag(kid_id), Some(json!(true)), "a child only here");
    assert_eq!(
        flag(bob_id),
        Some(json!(false)),
        "Bob owns a family of his own"
    );
    assert_eq!(
        flag(olive_id),
        Some(json!(false)),
        "present on every row of the owner's read, false on their own"
    );
    let as_member = json_of(ts.get(&kid, "/families/mine").await, "roster").await;
    assert!(
        as_member["members"]
            .as_array()
            .expect("members")
            .iter()
            .all(|member| member.get("password_reset").is_none()),
        "absent from every read but the owner's"
    );

    assert_error(
        ts.post(
            &olive,
            &format!("/families/members/{bob_id}/password"),
            json!({"new_password": "newpassword1"}),
        )
        .await,
        403,
        "member_in_other_families",
    )
    .await;
    let response = ts
        .post(
            &olive,
            &format!("/families/{smiths}/members/{kid_id}/password"),
            json!({"new_password": "newpassword1"}),
        )
        .await;
    assert_eq!(response.status(), 204);
    let _ = bob;
}

/// A newcomer arrives on the owner's screens already saying whether the owner
/// may reset their password — on the approval's answer and on the owner's
/// copy of `member_joined` — and on nobody else's copy at all.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_newcomer_arrives_with_the_owners_password_reset_flag() {
    let ts = spawn_server().await;
    let (olive, _) = ts.register("olive", "Olive").await;
    let (kid, _) = ts.register("kid", "Kid").await;
    let (carol, carol_id) = ts.register("carol", "Carol").await;
    let (_smiths, code) = ts.create_family(&olive, "The Smiths").await;
    ts.create_family(&carol, "The Carols").await;
    // Approval is the default policy: the kid comes in by an approval too.
    ts.join(&kid, &code, "pending").await;
    let requests = json_of(ts.get(&olive, "/families/join-requests").await, "requests").await;
    let kid_request = requests["requests"][0]["id"].as_i64().expect("request id");
    let approved = json_of(
        ts.post(
            &olive,
            &format!("/families/join-requests/{kid_request}/approve"),
            json!({}),
        )
        .await,
        "approving the kid",
    )
    .await;
    assert_eq!(
        approved["member"]["password_reset"],
        json!(true),
        "only here, so the owner may"
    );

    let mut olive_ws = connect_ws(&ts, &olive, true).await;
    let mut kid_ws = connect_ws(&ts, &kid, true).await;
    ts.join(&carol, &code, "pending").await;
    let requests = json_of(ts.get(&olive, "/families/join-requests").await, "requests").await;
    let carol_request = requests["requests"]
        .as_array()
        .expect("requests")
        .iter()
        .find(|request| request["user"]["id"].as_i64() == Some(carol_id))
        .expect("Carol's request")["id"]
        .as_i64()
        .expect("request id");
    let approved = json_of(
        ts.post(
            &olive,
            &format!("/families/join-requests/{carol_request}/approve"),
            json!({}),
        )
        .await,
        "approving Carol",
    )
    .await;
    assert_eq!(
        approved["member"]["password_reset"],
        json!(false),
        "Carol owns a family of her own"
    );
    let owners = next_frame_where(&mut olive_ws, |frame| {
        frame["type"] == "member_joined" && frame["user"]["id"].as_i64() == Some(carol_id)
    })
    .await;
    assert_eq!(owners["password_reset"], json!(false));
    let members = next_frame_where(&mut kid_ws, |frame| {
        frame["type"] == "member_joined" && frame["user"]["id"].as_i64() == Some(carol_id)
    })
    .await;
    assert!(
        members.get("password_reset").is_none(),
        "absent from everybody's copy but the owner's: {members}"
    );
}

/// The invitation push is held back only by a socket that was SENT the
/// invitation: a device whose app predates several families is never sent the
/// frame, so its open socket must not stand in for the push.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_invitation_wakes_a_device_whose_socket_predates_several_families() {
    let ts = spawn_server().await;
    let (_olive, olive_id, bob, _, _smiths, browns) = bob_in_two(&ts).await;
    let phone = ts.login("olive", "password123").await;
    let device = ts
        .post(
            &phone,
            "/devices",
            json!({"platform": "ios", "push_token": "olive-old-phone"}),
        )
        .await;
    assert_eq!(device.status(), 201);
    let _old_app = connect_ws(&ts, &phone, false).await;

    invite_ok(&ts, &bob, browns, olive_id).await;
    let deadline = tokio::time::Instant::now() + FRAME_WAIT;
    while !ts
        .push
        .calls()
        .iter()
        .any(|call| matches!(call.note.event, PushEvent::Invitation { .. }))
    {
        assert!(
            tokio::time::Instant::now() < deadline,
            "no invitation push for a device with only a legacy socket"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// An app from before several families that was not looking when its home
/// family went — the account still in another — is signed out rather than
/// left holding the old family's data and cursors as the new one's. Only
/// sessions that never said `X-FC-Families: 1` are ended, only when the HOME
/// family moves, and their devices go with them.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn leaving_the_home_family_signs_out_apps_from_before() {
    let ts = spawn_server().await;
    let (_olive, _, bob, bob_id, smiths, browns) = bob_in_two(&ts).await;
    let old_phone = ts.login("bob", "password123").await;
    let device = ts
        .post(
            &old_phone,
            "/devices",
            json!({"platform": "android", "push_token": "bob-old-phone"}),
        )
        .await;
    assert_eq!(device.status(), 201);
    // A sign-in that says it knows about several families is marked from the
    // start.
    let new_phone = json_of(
        ts.client
            .post(ts.url("/auth/login"))
            .header(MULTI.0, MULTI.1)
            .json(&json!({"username": "bob", "password": "password123"}))
            .send()
            .await
            .expect("request sends"),
        "signing in on a current app",
    )
    .await["token"]
        .as_str()
        .expect("token")
        .to_string();

    // Leaving a family that is NOT home moves nothing.
    let (carl, _) = ts.register("carl", "Carl").await;
    let (carls, _) = ts.create_family(&carl, "The Carls").await;
    open_family(&ts, &carl, carls).await;
    let code = invite_code_of(&ts, &carl, carls).await;
    ts.join(&bob, &code, "joined").await;
    let response = ts
        .post(&bob, &format!("/families/{carls}/leave"), json!({}))
        .await;
    assert_eq!(response.status(), 204);
    assert_eq!(ts.get(&old_phone, "/me").await.status(), 200);

    let mut old_socket = connect_ws(&ts, &old_phone, false).await;
    let response = ts
        .post(&bob, &format!("/families/{smiths}/leave"), json!({}))
        .await;
    assert_eq!(response.status(), 204);

    // The old app's socket is closed as "your session no longer exists".
    let deadline = tokio::time::Instant::now() + FRAME_WAIT;
    let code = loop {
        let message = tokio::time::timeout_at(deadline, old_socket.next())
            .await
            .expect("the old socket closes");
        match message {
            Some(Ok(Message::Close(Some(frame)))) => break u16::from(frame.code),
            Some(Ok(_)) => continue,
            other => panic!("expected a close frame, got {other:?}"),
        }
    };
    assert_eq!(code, 4401);
    assert_eq!(ts.get(&old_phone, "/me").await.status(), 401);
    let devices: i64 =
        sqlx::query_scalar("SELECT count(*) FROM devices WHERE push_token = 'bob-old-phone'")
            .fetch_one(&ts.state.pool)
            .await
            .expect("counting devices");
    assert_eq!(
        devices, 0,
        "the old phone's push token goes with its session"
    );

    // The current apps carry on, now at home in the Browns.
    for token in [&bob, &new_phone] {
        let body = me(&ts, token, true).await;
        assert_eq!(body["family"]["id"].as_i64(), Some(browns));
    }
    let _ = bob_id;
}

/// Removing a member removes them from THAT family: its chat and groups go,
/// their other family and its chat stay.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn removing_a_member_removes_them_from_that_family_only() {
    let ts = spawn_server().await;
    let (olive, olive_id, bob, bob_id, smiths, browns) = bob_in_two(&ts).await;
    let (kid, kid_id) = ts.register("kid", "Kid").await;
    let code = invite_code_of(&ts, &olive, smiths).await;
    ts.join(&kid, &code, "joined").await;
    let group = json_of(
        ts.post(
            &olive,
            "/chats/groups",
            json!({"title": "Cousins", "member_ids": [bob_id, kid_id]}),
        )
        .await,
        "a group in the Smiths",
    )
    .await;
    assert_eq!(group["chat"]["family_ids"], json!([smiths]));
    let group_id = group["chat"]["id"].as_i64().expect("id");
    // A group in a family the creator names, and never across families.
    assert_error(
        ts.post(
            &bob,
            "/chats/groups",
            json!({"title": "Browns", "member_ids": [kid_id], "family_id": browns}),
        )
        .await,
        409,
        "not_same_family",
    )
    .await;
    assert_error(
        ts.post(
            &kid,
            "/chats/groups",
            json!({"title": "X", "member_ids": [olive_id], "family_id": browns}),
        )
        .await,
        404,
        "family_not_found",
    )
    .await;

    let response = ts
        .delete(&olive, &format!("/families/{smiths}/members/{bob_id}"))
        .await;
    assert_eq!(response.status(), 204);
    let bob_chats = chats(&ts, &bob, true).await;
    assert!(
        bob_chats
            .iter()
            .all(|chat| chat["id"].as_i64() != Some(group_id))
    );
    assert!(
        bob_chats
            .iter()
            .all(|chat| chat["family_ids"] == json!([browns]))
    );
    assert_eq!(family_ids_of(&me(&ts, &bob, true).await), vec![browns]);
    let _ = kid;
}

// ---------------------------------------------------------------------------
// Uploads, the assistant, and the 404 for somebody else's family
// ---------------------------------------------------------------------------

async fn upload_for(ts: &TestServer, token: &str, family: Option<i64>) -> reqwest::Response {
    let path = match family {
        Some(family) => format!("/attachments?kind=photo&family_id={family}"),
        None => "/attachments?kind=photo".to_string(),
    };
    let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE0];
    bytes.resize(256, 0x11);
    ts.put_bytes_method("POST", token, &path, "image/jpeg", bytes)
        .await
}

async fn attachment_id(response: reqwest::Response) -> i64 {
    assert_eq!(response.status(), 201, "uploading");
    let body: Value = response.json().await.expect("JSON");
    body["attachment"]["id"].as_i64().expect("id")
}

/// An upload belongs to the family it was made for, and may be used only in
/// a chat that family lists, or its board — otherwise it is an id that does
/// not exist (docs/protocol.md, "Several families").
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_upload_is_used_only_where_its_family_is() {
    let ts = spawn_server().await;
    let (olive, _, bob, _, smiths, browns) = bob_in_two(&ts).await;
    let smiths_chat = family_chat_of(&ts, &bob, smiths).await;
    let browns_chat = family_chat_of(&ts, &bob, browns).await;
    let send_with = |chat: i64, attachment: i64| {
        let ts = &ts;
        let bob = &bob;
        async move {
            ts.post(
                bob,
                &format!("/chats/{chat}/messages"),
                json!({"client_msg_id": Uuid::new_v4().to_string(), "body": "",
                       "attachment_ids": [attachment]}),
            )
            .await
        }
    };

    let for_browns = attachment_id(upload_for(&ts, &bob, Some(browns)).await).await;
    assert_error(
        send_with(smiths_chat, for_browns).await,
        404,
        "attachment_not_found",
    )
    .await;
    assert_eq!(send_with(browns_chat, for_browns).await.status(), 201);

    // No family named: the home family's — not the Browns' board.
    let for_home = attachment_id(upload_for(&ts, &bob, None).await).await;
    assert_error(
        ts.post(
            &bob,
            &format!("/families/{browns}/board/notes"),
            json!({"kind": "photo", "attachment_id": for_home, "text": "", "color": "yellow",
                   "x": 0.2, "y": 0.2}),
        )
        .await,
        404,
        "attachment_not_found",
    )
    .await;
    assert_eq!(send_with(smiths_chat, for_home).await.status(), 201);

    // A family the uploader is not in.
    let (_, carl_family) = {
        let (carl, _) = ts.register("carl", "Carl").await;
        ts.create_family(&carl, "The Carls").await
    };
    let _ = carl_family;
    let carls: i64 = sqlx::query_scalar("SELECT id FROM families WHERE name = 'The Carls'")
        .fetch_one(&ts.state.pool)
        .await
        .expect("the Carls");
    assert_error(
        upload_for(&ts, &bob, Some(carls)).await,
        404,
        "family_not_found",
    )
    .await;
    let _ = olive;
}

/// One assistant chat per family, each listed in its family; an app from
/// before gets its home family's (D6).
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn each_family_has_its_own_assistant_chat() {
    let ts = spawn_server_with_config(|cfg| {
        cfg.ai.enabled = true;
        cfg.ai.endpoint = "https://example.invalid".to_string();
        cfg.ai.deployment = "test-deployment".to_string();
        cfg.ai.api_key = "test-key".to_string();
        cfg.ai.processor = "Microsoft — Azure OpenAI".to_string();
        cfg.ai.title = "Assistant".to_string();
    })
    .await;
    let (_olive, _, bob, _, smiths, browns) = bob_in_two(&ts).await;
    let ai_chats: Vec<Value> = chats(&ts, &bob, true)
        .await
        .into_iter()
        .filter(|chat| chat["kind"] == "ai")
        .collect();
    let mut families: Vec<Value> = ai_chats
        .iter()
        .map(|chat| chat["family_ids"].clone())
        .collect();
    families.sort_by_key(|ids| ids[0].as_i64());
    assert_eq!(families, vec![json!([smiths]), json!([browns])]);
    assert_ne!(ai_chats[0]["id"], ai_chats[1]["id"]);
    let old: Vec<Value> = chats(&ts, &bob, false)
        .await
        .into_iter()
        .filter(|chat| chat["kind"] == "ai")
        .collect();
    assert_eq!(old.len(), 1);
    assert_eq!(old[0]["family_ids"], json!([smiths]));
}

/// A family the caller is not in is `family_not_found` on every form, the
/// same as one that never existed — and `mine` is home.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_family_the_caller_is_not_in_is_not_found() {
    let ts = spawn_server().await;
    let (olive, _, bob, _, smiths, browns) = bob_in_two(&ts).await;
    for path in [
        format!("/families/{browns}"),
        format!("/families/{browns}/board"),
        format!("/families/{browns}/pack"),
        format!("/families/{browns}/stats"),
        format!("/families/{browns}/join-requests"),
        format!("/families/{browns}/invitations"),
        "/families/999999".to_string(),
        "/families/999999/board".to_string(),
        "/families/abc".to_string(),
        // A segment that does not even decode is not the home family.
        "/families/%FF".to_string(),
        "/families/%FF/board".to_string(),
    ] {
        assert_error(ts.get(&olive, &path).await, 404, "family_not_found").await;
    }
    assert_error(
        ts.post(&olive, &format!("/families/{browns}/leave"), json!({}))
            .await,
        404,
        "family_not_found",
    )
    .await;
    let mine = json_of(ts.get(&bob, "/families/mine").await, "mine").await;
    assert_eq!(mine["family"]["id"].as_i64(), Some(smiths));
    let browns_read = json_of(ts.get(&bob, &format!("/families/{browns}")).await, "by id").await;
    assert_eq!(browns_read["family"]["id"].as_i64(), Some(browns));
    assert!(
        browns_read["family"]["invite_code"].is_string(),
        "Bob owns the Browns"
    );
}

// ---------------------------------------------------------------------------
// Deleting an account in several families
// ---------------------------------------------------------------------------

/// Deleting an account leaves EVERY family, each with its own hand-off, the
/// account in each family's `former_members`, and one `member_deleted` per
/// family (docs/protocol.md, "Several families" and "Deleting an account").
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn deleting_an_account_leaves_every_family() {
    let ts = spawn_server().await;
    let (olive, _, bob, bob_id, smiths, browns) = bob_in_two(&ts).await;
    let (eve, eve_id) = ts.register("eve", "Eve").await;
    open_family(&ts, &bob, browns).await;
    let browns_code = invite_code_of(&ts, &bob, browns).await;
    ts.join(&eve, &browns_code, "joined").await;
    let smiths_code = invite_code_of(&ts, &olive, smiths).await;
    ts.join(&eve, &smiths_code, "joined").await;

    let mut olive_ws = connect_ws(&ts, &olive, true).await;
    let mut eve_ws = connect_ws(&ts, &eve, true).await;

    let response = ts
        .post(&bob, "/me/delete", json!({"password": "password123"}))
        .await;
    assert_eq!(response.status(), 204);

    // Olive, in the Smiths only: one tombstone, the Smiths'.
    let deleted = next_frame_of_type(&mut olive_ws, "member_deleted").await;
    assert_eq!(deleted["family_id"].as_i64(), Some(smiths));
    assert_eq!(deleted["member"]["id"].as_i64(), Some(bob_id));
    // Eve, in both: the Browns pass to her, and she hears of Bob twice.
    let mut families_told = Vec::new();
    let mut owner_frame = None;
    while families_told.len() < 2 || owner_frame.is_none() {
        let frame = next_frame_where(&mut eve_ws, |frame| {
            frame["type"] == "member_deleted" || frame["type"] == "family_owner"
        })
        .await;
        if frame["type"] == "member_deleted" {
            families_told.push(frame["family_id"].as_i64().expect("family id"));
        } else {
            owner_frame = Some(frame);
        }
    }
    families_told.sort_unstable();
    let mut expected = vec![smiths, browns];
    expected.sort_unstable();
    assert_eq!(families_told, expected);
    let owner_frame = owner_frame.expect("the hand-off");
    assert_eq!(owner_frame["family_id"].as_i64(), Some(browns));
    assert_eq!(owner_frame["user_id"].as_i64(), Some(eve_id));

    for (token, family) in [(&olive, smiths), (&eve, browns)] {
        let roster = json_of(
            ts.get(token, &format!("/families/{family}")).await,
            "roster",
        )
        .await;
        let former: Vec<i64> = roster["former_members"]
            .as_array()
            .expect("Bob is a former member")
            .iter()
            .map(|member| member["id"].as_i64().expect("id"))
            .collect();
        assert_eq!(former, vec![bob_id], "former member of family {family}");
    }
    let browns_now = json_of(ts.get(&eve, &format!("/families/{browns}")).await, "Browns").await;
    assert!(
        browns_now["family"]["invite_code"].is_string(),
        "Eve owns the Browns now"
    );
}

// ---------------------------------------------------------------------------
// Every legacy route, for an account in one family
// ---------------------------------------------------------------------------

/// Every form without an id still routes, for an account in ONE family —
/// and so does the same endpoint with that family's id in the path. A route
/// shadowed by `{family}` would answer 404 or 405 here.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn every_route_without_an_id_still_works_for_a_single_family_account() {
    let ts = spawn_server().await;
    let (olive, _) = ts.register("olive", "Olive").await;
    let (bob, bob_id) = ts.register("bob", "Bob").await;
    let (carl, carl_id) = ts.register("carl", "Carl").await;
    let (family, code) = ts.create_family(&olive, "The Smiths").await;
    ts.join(&bob, &code, "pending").await;
    ts.join(&carl, &code, "pending").await;

    for prefix in ["/families", &format!("/families/{family}")] {
        let mine = if prefix == "/families" {
            "/families/mine".to_string()
        } else {
            prefix.to_string()
        };
        assert_eq!(ts.get(&olive, &mine).await.status(), 200, "{mine}");
        assert_eq!(
            ts.patch(&olive, &mine, json!({"language": "ru"}))
                .await
                .status(),
            200,
            "PATCH {mine}"
        );
        assert_eq!(ts.get(&olive, &format!("{mine}/stats")).await.status(), 200);
        assert_eq!(
            ts.post(&olive, &format!("{prefix}/invite-code/rotate"), json!({}))
                .await
                .status(),
            200,
            "{prefix}/invite-code/rotate"
        );
        assert_eq!(
            ts.get(&olive, &format!("{prefix}/join-requests"))
                .await
                .status(),
            200
        );
        assert_eq!(
            ts.get(&olive, &format!("{prefix}/reports")).await.status(),
            200
        );
        assert_eq!(ts.get(&olive, &format!("{mine}/board")).await.status(), 200);
        assert_eq!(
            ts.get(&olive, &format!("{mine}/board/changes?after_seq=0"))
                .await
                .status(),
            200
        );
        assert_eq!(ts.get(&olive, &format!("{mine}/pack")).await.status(), 200);
        assert_eq!(
            ts.get(&olive, &format!("{mine}/pack/changes?after_seq=0"))
                .await
                .status(),
            200
        );
        let note = ts
            .post(
                &olive,
                &format!("{mine}/board/notes"),
                json!({"text": "Milk", "color": "yellow", "x": 0.1, "y": 0.1}),
            )
            .await;
        assert_eq!(note.status(), 201);
        let note: Value = note.json().await.expect("JSON");
        let note_id = note["note"]["id"].as_i64().expect("id");
        assert_eq!(note["note"]["family_id"].as_i64(), Some(family));
        assert_eq!(
            ts.patch(
                &olive,
                &format!("{mine}/board/notes/{note_id}"),
                json!({"x": 0.5})
            )
            .await
            .status(),
            200
        );
        assert_eq!(
            ts.delete(&olive, &format!("{mine}/board/notes/{note_id}"))
                .await
                .status(),
            204
        );
        // A tombstone says whose board it was on, like a live note.
        let changes = json_of(
            ts.get(&olive, &format!("{mine}/board/changes?after_seq=0"))
                .await,
            "board changes",
        )
        .await;
        let tombstone = changes["notes"]
            .as_array()
            .expect("notes")
            .iter()
            .find(|note| note["id"].as_i64() == Some(note_id))
            .expect("the tombstone is in the feed")
            .clone();
        assert_eq!(tombstone["deleted"], true);
        assert_eq!(tombstone["family_id"].as_i64(), Some(family));
    }

    // The request-scoped ones, once per form.
    let requests = json_of(ts.get(&olive, "/families/join-requests").await, "requests").await;
    let ids: Vec<i64> = requests["requests"]
        .as_array()
        .expect("requests")
        .iter()
        .map(|request| request["id"].as_i64().expect("id"))
        .collect();
    assert_eq!(ids.len(), 2);
    assert_eq!(
        ts.post(
            &olive,
            &format!("/families/join-requests/{}/approve", ids[0]),
            json!({})
        )
        .await
        .status(),
        200
    );
    assert_eq!(
        ts.post(
            &olive,
            &format!("/families/{family}/join-requests/{}/approve", ids[1]),
            json!({})
        )
        .await
        .status(),
        200
    );
    for (path, target) in [
        ("/families/members".to_string(), bob_id),
        (format!("/families/{family}/members"), carl_id),
    ] {
        assert_eq!(
            ts.put(
                &olive,
                &format!("{path}/{target}/birthday"),
                json!({"month": 3, "day": 14})
            )
            .await
            .status(),
            200
        );
        assert_eq!(
            ts.delete(&olive, &format!("{path}/{target}/birthday"))
                .await
                .status(),
            204
        );
        assert_eq!(
            ts.put(&olive, &format!("{path}/{target}/block"), json!({}))
                .await
                .status(),
            204
        );
        assert_eq!(
            ts.delete(&olive, &format!("{path}/{target}/block"))
                .await
                .status(),
            204
        );
        assert_eq!(
            ts.post(
                &olive,
                &format!("{path}/{target}/password"),
                json!({"new_password": "newpassword1"})
            )
            .await
            .status(),
            204
        );
    }
    // Reports: filed by a member, resolved by the owner, both forms.
    let bob = ts.login("bob", "newpassword1").await;
    let carl = ts.login("carl", "newpassword1").await;
    let mut report_ids = Vec::new();
    for (token, prefix) in [
        (&bob, "/families".to_string()),
        (&carl, format!("/families/{family}")),
    ] {
        let response = ts
            .post(
                token,
                &format!("{prefix}/reports"),
                json!({"reported_user_id": if prefix == "/families" { carl_id } else { bob_id },
                       "reason": "spam"}),
            )
            .await;
        assert_eq!(response.status(), 201);
        let body: Value = response.json().await.expect("JSON");
        report_ids.push(body["report"]["id"].as_i64().expect("id"));
    }
    assert_eq!(
        ts.post(
            &olive,
            &format!("/families/reports/{}/resolve", report_ids[0]),
            json!({})
        )
        .await
        .status(),
        204
    );
    assert_eq!(
        ts.post(
            &olive,
            &format!("/families/{family}/reports/{}/resolve", report_ids[1]),
            json!({})
        )
        .await
        .status(),
        204
    );
    // And the doors out.
    assert_eq!(
        ts.delete(&olive, &format!("/families/members/{carl_id}"))
            .await
            .status(),
        204
    );
    assert_eq!(
        ts.post(&bob, "/families/leave", json!({})).await.status(),
        204
    );
    let _ = carl;
}

// ---------------------------------------------------------------------------
// The migration
// ---------------------------------------------------------------------------

/// 0057's backfill, run against the shape the schema had before it: every
/// member keeps their family, dated by when they joined its chat; a scrubbed
/// account becomes a former member; an `ai` thread pointing at a family its
/// member left is moved to the family they are in; and the old columns go.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_0057_backfill_carries_every_membership_over() {
    let ts = spawn_server().await;
    let (olive, olive_id) = ts.register("olive", "Olive").await;
    let (bob, bob_id) = ts.register("bob", "Bob").await;
    let (gone, gone_id) = ts.register("gone", "Gone").await;
    let (smiths, code) = ts.create_family(&olive, "The Smiths").await;
    ts.set_open_policy(&olive).await;
    ts.join(&bob, &code, "joined").await;
    ts.join(&gone, &code, "joined").await;
    let response = ts
        .post(&gone, "/me/delete", json!({"password": "password123"}))
        .await;
    assert_eq!(response.status(), 204);
    let pool = &ts.state.pool;
    let joined_at = |user: i64| async move {
        sqlx::query_scalar::<_, time::OffsetDateTime>(
            "SELECT cm.joined_at FROM chat_members cm JOIN chats c ON c.id = cm.chat_id
              WHERE c.family_id = $1 AND c.kind = 'family' AND cm.user_id = $2",
        )
        .bind(smiths)
        .bind(user)
        .fetch_one(pool)
        .await
        .expect("a family-chat membership")
    };
    let bob_joined = joined_at(bob_id).await;
    // An `ai` thread of Bob's that still names a family he is not in.
    let (_, _) = ts
        .create_family(&ts.register("other", "Other").await.0, "Others")
        .await;
    let others: i64 = sqlx::query_scalar("SELECT id FROM families WHERE name = 'Others'")
        .fetch_one(pool)
        .await
        .expect("the other family");
    let thread: i64 = sqlx::query_scalar(
        "INSERT INTO chats (family_id, kind, user_a_id) VALUES ($1, 'ai', $2) RETURNING id",
    )
    .bind(others)
    .bind(bob_id)
    .fetch_one(pool)
    .await
    .expect("a stale thread");

    // Back to the shape before 0057, filled the way the old server wrote it.
    sqlx::raw_sql(
        "ALTER TABLE users ADD COLUMN family_id BIGINT REFERENCES families(id) ON DELETE SET NULL;
         ALTER TABLE users ADD COLUMN deleted_family_id BIGINT
             REFERENCES families(id) ON DELETE SET NULL;
         UPDATE users u SET family_id = fm.family_id
           FROM family_members fm WHERE fm.user_id = u.id AND fm.family_id <> (
               SELECT id FROM families WHERE name = 'Others');
         UPDATE users u SET deleted_family_id = ffm.family_id
           FROM former_family_members ffm WHERE ffm.user_id = u.id;
         DROP TABLE invitations;
         DROP TABLE former_family_members;
         DROP TABLE family_members;
         DROP INDEX chats_ai_owner_uq;
         CREATE UNIQUE INDEX chats_ai_owner_uq ON chats (user_a_id) WHERE kind = 'ai';
         DROP INDEX member_reports_open_message_uq;
         DROP INDEX member_reports_open_person_uq;
         CREATE UNIQUE INDEX member_reports_open_message_uq
             ON member_reports (reporter_user_id, message_id)
             WHERE status = 'open' AND message_id IS NOT NULL;
         CREATE UNIQUE INDEX member_reports_open_person_uq
             ON member_reports (reporter_user_id, reported_user_id)
             WHERE status = 'open' AND message_id IS NULL AND message_excerpt IS NULL;",
    )
    .execute(pool)
    .await
    .expect("rewinding to the shape before 0057");
    let sql = family_connect::migrate::MIGRATIONS
        .iter()
        .find(|migration| migration.version == 57)
        .expect("0057 is in the list")
        .sql;
    sqlx::raw_sql(sql)
        .execute(pool)
        .await
        .expect("0057 runs on the old shape");

    let members: Vec<(i64, time::OffsetDateTime)> = sqlx::query_as(
        "SELECT user_id, joined_at FROM family_members WHERE family_id = $1 ORDER BY user_id",
    )
    .bind(smiths)
    .fetch_all(pool)
    .await
    .expect("the memberships");
    let ids: Vec<i64> = members.iter().map(|(id, _)| *id).collect();
    assert_eq!(ids, vec![olive_id, bob_id], "the tombstone is not a member");
    assert_eq!(members[1].1, bob_joined, "dated by the family-chat join");
    let former: Vec<i64> =
        sqlx::query_scalar("SELECT user_id FROM former_family_members WHERE family_id = $1")
            .bind(smiths)
            .fetch_all(pool)
            .await
            .expect("former members");
    assert_eq!(former, vec![gone_id]);
    let thread_family: i64 = sqlx::query_scalar("SELECT family_id FROM chats WHERE id = $1")
        .bind(thread)
        .fetch_one(pool)
        .await
        .expect("the thread");
    assert_eq!(thread_family, smiths, "the stale thread follows its member");
    let old_columns: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.columns
          WHERE table_name = 'users' AND column_name IN ('family_id', 'deleted_family_id')",
    )
    .fetch_one(pool)
    .await
    .expect("the catalogue");
    assert_eq!(old_columns, 0, "no stale column survives to be read");
    // And the server reads the result as it always did.
    let body = me(&ts, &bob, false).await;
    assert_eq!(body["family"]["id"].as_i64(), Some(smiths));
}
