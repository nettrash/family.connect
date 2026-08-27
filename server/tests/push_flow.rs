//! Integration: the real APNs/FCM push transports against a mock push
//! service.
//!
//! A tiny axum server stands in for both Apple and Google on one ephemeral
//! port: it captures every request (path, headers, body) and can be flipped
//! into "unregistered token" mode. The family-connect server under test gets
//! `[push.apns]`/`[push.fcm]` sections whose endpoints (and the service
//! account's `token_uri`) point at the mock, with throwaway keys generated
//! by tests/common. The mock speaks HTTP/1.1 — the push client deliberately
//! does not force HTTP/2 (against the real APNs host, TLS ALPN negotiates
//! h2; a plain-HTTP override stays on http1), so this works without an h2c
//! server.

mod common;

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::Json;
use axum::routing::post;
use common::{
    TestServer, assert_error, spawn_server, spawn_server_with_push, write_test_apns_key,
    write_test_service_account,
};
use family_connect::config::{ApnsConfig, FcmConfig, PushConfig};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::http::header::AUTHORIZATION;
use uuid::Uuid;

const PUSH_WAIT: Duration = Duration::from_secs(5);

/// One request the mock push service captured.
#[derive(Debug, Clone)]
struct CapturedRequest {
    path: String,
    /// Lower-cased header name → value, for the handful the tests inspect.
    headers: Vec<(String, String)>,
    /// JSON bodies parse; the OAuth form body is kept as a raw string.
    body: Value,
    raw_body: String,
}

impl CapturedRequest {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

/// Shared state of the mock APNs/FCM service.
#[derive(Default)]
struct MockPush {
    requests: Mutex<Vec<CapturedRequest>>,
    /// When set, device sends answer "this token is unregistered":
    /// APNs → 410 {"reason": "Unregistered"}, FCM → 404 UNREGISTERED.
    reply_unregistered: AtomicBool,
}

impl MockPush {
    fn capture(&self, uri: &Uri, headers: &HeaderMap, raw_body: String) {
        let captured = CapturedRequest {
            path: uri.path().to_string(),
            headers: headers
                .iter()
                .map(|(name, value)| {
                    (
                        name.as_str().to_string(),
                        value.to_str().unwrap_or("<binary>").to_string(),
                    )
                })
                .collect(),
            body: serde_json::from_str(&raw_body).unwrap_or(Value::Null),
            raw_body,
        };
        self.requests.lock().expect("mock lock").push(captured);
    }

    fn requests(&self) -> Vec<CapturedRequest> {
        self.requests.lock().expect("mock lock").clone()
    }

    /// Poll until at least `count` requests whose path passes `filter`
    /// arrived — push delivery is spawned fire-and-forget server-side.
    async fn wait_for(&self, count: usize, filter: fn(&str) -> bool) -> Vec<CapturedRequest> {
        let deadline = tokio::time::Instant::now() + PUSH_WAIT;
        loop {
            let matching: Vec<CapturedRequest> = self
                .requests()
                .into_iter()
                .filter(|r| filter(&r.path))
                .collect();
            if matching.len() >= count {
                return matching;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for {count} mock push request(s); got {:?}",
                self.requests()
                    .iter()
                    .map(|r| r.path.clone())
                    .collect::<Vec<_>>()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

async fn apns_route(
    State(mock): State<Arc<MockPush>>,
    uri: Uri,
    headers: HeaderMap,
    body: String,
) -> (StatusCode, Json<Value>) {
    mock.capture(&uri, &headers, body);
    if mock.reply_unregistered.load(Ordering::SeqCst) {
        (StatusCode::GONE, Json(json!({"reason": "Unregistered"})))
    } else {
        (StatusCode::OK, Json(json!({})))
    }
}

async fn token_route(
    State(mock): State<Arc<MockPush>>,
    uri: Uri,
    headers: HeaderMap,
    body: String,
) -> (StatusCode, Json<Value>) {
    mock.capture(&uri, &headers, body);
    (
        StatusCode::OK,
        Json(json!({
            "access_token": "mock-access-token",
            "token_type": "Bearer",
            "expires_in": 3600,
        })),
    )
}

async fn fcm_route(
    State(mock): State<Arc<MockPush>>,
    uri: Uri,
    headers: HeaderMap,
    body: String,
) -> (StatusCode, Json<Value>) {
    mock.capture(&uri, &headers, body);
    if mock.reply_unregistered.load(Ordering::SeqCst) {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": {
                "code": 404,
                "status": "NOT_FOUND",
                "details": [{"errorCode": "UNREGISTERED"}],
            }})),
        )
    } else {
        (
            StatusCode::OK,
            Json(json!({"name": "projects/test-project/messages/0:mock"})),
        )
    }
}

/// Start the mock push service on an ephemeral loopback port.
async fn spawn_mock_push() -> (Arc<MockPush>, SocketAddr) {
    let mock = Arc::new(MockPush::default());
    let router = Router::new()
        .route("/3/device/{token}", post(apns_route))
        .route("/oauth/token", post(token_route))
        .route("/v1/projects/{project}/messages:send", post(fcm_route))
        .with_state(mock.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("binding the mock push port");
    let addr = listener.local_addr().expect("mock local addr");
    tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("mock push server crashed");
    });
    (mock, addr)
}

fn apns_config(mock_addr: SocketAddr, key_file: std::path::PathBuf) -> PushConfig {
    PushConfig {
        driver: None,
        include_message_body: true,
        apns: Some(ApnsConfig {
            team_id: "TEAMID9999".to_string(),
            key_id: "KEYID12345".to_string(),
            key_file,
            bundle_id: "me.nettrash.familyconnect".to_string(),
            environment: "production".to_string(),
            endpoint: Some(format!("http://{mock_addr}")),
        }),
        fcm: None,
    }
}

fn fcm_config(mock_addr: SocketAddr, credentials_file: std::path::PathBuf) -> PushConfig {
    PushConfig {
        driver: None,
        include_message_body: true,
        apns: None,
        fcm: Some(FcmConfig {
            credentials_file,
            endpoint: Some(format!("http://{mock_addr}")),
        }),
    }
}

/// Family of two on the open join policy; returns
/// `(owner_token, member_token, member_id)`.
async fn family_of_two(ts: &TestServer) -> (String, String, i64) {
    let (owner, _) = ts.register("owner", "Olive").await;
    let (member, member_id) = ts.register("junior", "Junior").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&member, &invite_code, "joined").await;
    (owner, member, member_id)
}

/// Register a device with a push token; returns nothing — the status is
/// asserted here.
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

/// Post a message and return its server-assigned id.
async fn post_message_id(ts: &TestServer, token: &str, chat_id: i64, body: &str) -> i64 {
    let response = ts
        .post_message(token, chat_id, &Uuid::new_v4().to_string(), body)
        .await;
    assert_eq!(response.status(), 201, "posting a message");
    let body: Value = response.json().await.expect("message response is JSON");
    body["message"]["id"].as_i64().expect("message id")
}

/// Poll until the user has no device rows left (dead-token cleanup is
/// fire-and-forget server-side).
async fn wait_for_device_deletion(ts: &TestServer, user_id: i64) {
    let deadline = tokio::time::Instant::now() + PUSH_WAIT;
    loop {
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM devices WHERE user_id = $1")
            .bind(user_id)
            .fetch_one(&ts.state.pool)
            .await
            .expect("counting device rows");
        if count == 0 {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the unregistered device row was never deleted"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn an_offline_ios_member_receives_the_exact_apns_request() {
    let (mock, mock_addr) = spawn_mock_push().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let key_file = write_test_apns_key(dir.path());
    let ts = spawn_server_with_push(apns_config(mock_addr, key_file)).await;

    let (owner, member, _member_id) = family_of_two(&ts).await;
    let chat_id = ts.family_chat_id(&owner).await;
    register_device(&ts, &member, "ios", "ios-token-1").await;

    let message_id = post_message_id(&ts, &owner, chat_id, "Dinner at 7?").await;

    let requests = mock
        .wait_for(1, |path| path.starts_with("/3/device/"))
        .await;
    let request = &requests[0];
    assert_eq!(request.path, "/3/device/ios-token-1");
    let authorization = request
        .header("authorization")
        .expect("authorization header");
    assert!(
        authorization.starts_with("bearer "),
        "APNs wants a lowercase bearer scheme: {authorization}"
    );
    assert_eq!(
        authorization
            .trim_start_matches("bearer ")
            .split('.')
            .count(),
        3,
        "the bearer value must be a JWT"
    );
    assert_eq!(
        request.header("apns-topic"),
        Some("me.nettrash.familyconnect")
    );
    assert_eq!(request.header("apns-push-type"), Some("alert"));
    assert_eq!(request.header("apns-priority"), Some("10"));
    assert_eq!(
        request.body,
        json!({
            "aps": {
                "alert": {"title": "The Smiths — Olive", "body": "Dinner at 7?"},
                "sound": "default",
                "badge": 1,
                "thread-id": format!("chat-{chat_id}"),
            },
            "chat_id": chat_id,
            "message_id": message_id,
            "kind": "message",
        })
    );
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn an_offline_android_member_receives_the_exact_fcm_request() {
    let (mock, mock_addr) = spawn_mock_push().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let credentials = write_test_service_account(
        dir.path(),
        "test-project",
        &format!("http://{mock_addr}/oauth/token"),
    );
    let ts = spawn_server_with_push(fcm_config(mock_addr, credentials)).await;

    let (owner, member, _member_id) = family_of_two(&ts).await;
    let chat_id = ts.family_chat_id(&owner).await;
    register_device(&ts, &member, "android", "android-token-1").await;

    let message_id = post_message_id(&ts, &owner, chat_id, "Dinner at 7?").await;

    // The transport first exchanges its RS256 assertion for an access token…
    let token_requests = mock.wait_for(1, |path| path == "/oauth/token").await;
    let token_request = &token_requests[0];
    assert!(
        token_request
            .raw_body
            .contains("grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer"),
        "OAuth exchange must use the jwt-bearer grant: {}",
        token_request.raw_body
    );
    assert!(
        token_request.raw_body.contains("assertion="),
        "OAuth exchange must carry the signed assertion"
    );

    // …then sends the protocol.md message shape with that token.
    let sends = mock
        .wait_for(1, |path| path.ends_with("messages:send"))
        .await;
    let send = &sends[0];
    assert_eq!(send.path, "/v1/projects/test-project/messages:send");
    assert_eq!(
        send.header("authorization"),
        Some("Bearer mock-access-token")
    );
    assert_eq!(
        send.body,
        json!({"message": {
            "token": "android-token-1",
            "notification": {"title": "The Smiths — Olive", "body": "Dinner at 7?"},
            "data": {
                "kind": "message",
                "chat_id": chat_id.to_string(),
                "message_id": message_id.to_string(),
            },
            "android": {
                "priority": "HIGH",
                "notification": {
                    "channel_id": "messages",
                    "tag": format!("chat-{chat_id}"),
                    // The recipient's unread in THIS chat — a number, not a
                    // string: the everything-is-a-string rule is about
                    // `data`, and this field is not in `data`.
                    "notification_count": 1,
                },
            },
        }})
    );
}

/// `notification_count` is the recipient's unread in the chat the push is
/// about, and never their total — Android has no icon badge, so the number
/// rides on a per-chat notification (`tag: "chat-<id>"`) that a launcher
/// SUMS across chats. A total on each of two chats would render as twice
/// the total. This test makes the two numbers different on purpose: the
/// member has 2 unread in a direct chat and 3 in the family chat, so a
/// payload reaching for the badge would say 5 in both.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn the_fcm_notification_count_is_the_chats_unread_and_not_the_total() {
    let (mock, mock_addr) = spawn_mock_push().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let credentials = write_test_service_account(
        dir.path(),
        "test-project",
        &format!("http://{mock_addr}/oauth/token"),
    );
    let ts = spawn_server_with_push(fcm_config(mock_addr, credentials)).await;

    let (owner, member, member_id) = family_of_two(&ts).await;
    let family_chat = ts.family_chat_id(&owner).await;
    register_device(&ts, &member, "android", "android-token-1").await;

    let direct: Value = ts
        .post(&owner, "/chats/direct", json!({"user_id": member_id}))
        .await
        .json()
        .await
        .expect("direct chat body");
    let direct_chat = direct["chat"]["id"].as_i64().expect("direct chat id");

    // Two in the direct chat, three in the family chat — the member reads
    // none of them.
    post_message_id(&ts, &owner, direct_chat, "coffee?").await;
    let last_direct = post_message_id(&ts, &owner, direct_chat, "or tea?").await;
    post_message_id(&ts, &owner, family_chat, "Dinner at 7?").await;
    post_message_id(&ts, &owner, family_chat, "or 8?").await;
    let last_family = post_message_id(&ts, &owner, family_chat, "bring bread").await;

    // The totals the chat list agrees on, so the numbers below are known to
    // be per-chat rather than coincidences.
    let chats: Value = ts.get(&member, "/chats").await.json().await.expect("json");
    let per_chat: Vec<i64> = chats["chats"]
        .as_array()
        .expect("chats array")
        .iter()
        .map(|entry| entry["unread_count"].as_i64().expect("unread"))
        .collect();
    assert_eq!(per_chat.iter().sum::<i64>(), 5, "the total across chats");

    let sends = mock
        .wait_for(5, |path| path.ends_with("messages:send"))
        .await;
    // Delivery is spawned, so the capture order is not the send order:
    // find each push by the message it announces.
    let notification_count = |message_id: i64| -> Value {
        let send = sends
            .iter()
            .find(|send| {
                send.body["message"]["data"]["message_id"] == message_id.to_string().as_str()
            })
            .unwrap_or_else(|| panic!("no FCM send for message {message_id}"));
        send.body["message"]["android"]["notification"]["notification_count"].clone()
    };

    assert_eq!(
        notification_count(last_family),
        json!(3),
        "the family chat's own unread, not the 5 the badge would carry"
    );
    assert_eq!(
        notification_count(last_direct),
        json!(2),
        "and the direct chat's own"
    );
}

/// A board note has no chat to count in, so it carries no count at all —
/// protocol.md omits `notification_count` from every kind but `message`.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_board_note_fcm_push_carries_no_notification_count() {
    let (mock, mock_addr) = spawn_mock_push().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let credentials = write_test_service_account(
        dir.path(),
        "test-project",
        &format!("http://{mock_addr}/oauth/token"),
    );
    let ts = spawn_server_with_push(fcm_config(mock_addr, credentials)).await;

    let (owner, member, _member_id) = family_of_two(&ts).await;
    register_device(&ts, &owner, "android", "owner-android-token").await;

    // An unread message first: the owner's badge is non-zero, so a payload
    // that leaked ANY count into a note push would be visible here.
    let chat_id = ts.family_chat_id(&owner).await;
    post_message_id(&ts, &member, chat_id, "Dinner at 7?").await;

    let response = ts
        .post(
            &member,
            "/families/mine/board/notes",
            json!({"text": "Milk", "color": "yellow", "x": 0.5, "y": 0.5}),
        )
        .await;
    assert_eq!(response.status(), 201);

    let sends = mock
        .wait_for(2, |path| path.ends_with("messages:send"))
        .await;
    let note_push = sends
        .iter()
        .find(|send| send.body["message"]["data"]["kind"] == "board_note")
        .expect("the board note push");
    assert!(
        note_push.body["message"]["android"]["notification"]
            .get("notification_count")
            .is_none(),
        "a board note counts nothing: {}",
        note_push.body
    );
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn an_unregistered_apns_token_gets_its_device_row_deleted() {
    let (mock, mock_addr) = spawn_mock_push().await;
    mock.reply_unregistered.store(true, Ordering::SeqCst);
    let dir = tempfile::tempdir().expect("tempdir");
    let key_file = write_test_apns_key(dir.path());
    let ts = spawn_server_with_push(apns_config(mock_addr, key_file)).await;

    let (owner, member, member_id) = family_of_two(&ts).await;
    let chat_id = ts.family_chat_id(&owner).await;
    register_device(&ts, &member, "ios", "dead-ios-token").await;

    post_message_id(&ts, &owner, chat_id, "hello?").await;
    wait_for_device_deletion(&ts, member_id).await;
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn an_unregistered_fcm_token_gets_its_device_row_deleted() {
    let (mock, mock_addr) = spawn_mock_push().await;
    mock.reply_unregistered.store(true, Ordering::SeqCst);
    let dir = tempfile::tempdir().expect("tempdir");
    let credentials = write_test_service_account(
        dir.path(),
        "test-project",
        &format!("http://{mock_addr}/oauth/token"),
    );
    let ts = spawn_server_with_push(fcm_config(mock_addr, credentials)).await;

    let (owner, member, member_id) = family_of_two(&ts).await;
    let chat_id = ts.family_chat_id(&owner).await;
    register_device(&ts, &member, "android", "dead-android-token").await;

    post_message_id(&ts, &owner, chat_id, "hello?").await;
    wait_for_device_deletion(&ts, member_id).await;
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn online_members_are_never_pushed() {
    let (mock, mock_addr) = spawn_mock_push().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let key_file = write_test_apns_key(dir.path());
    let ts = spawn_server_with_push(apns_config(mock_addr, key_file)).await;

    let (owner, member, _member_id) = family_of_two(&ts).await;
    let chat_id = ts.family_chat_id(&owner).await;
    register_device(&ts, &member, "ios", "ios-token-online").await;

    // The member is online over a live socket; the offline set is computed
    // from the same fan-out that delivers their `message` frame, so once
    // that frame arrives, no push decision can still be pending.
    let mut member_ws = connect_ws(&ts, &member).await;
    post_message_id(&ts, &owner, chat_id, "you there?").await;
    next_frame_of_type(&mut member_ws, "message").await;

    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        mock.requests().is_empty(),
        "an online member must never be pushed: {:?}",
        mock.requests()
            .iter()
            .map(|r| r.path.clone())
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_join_request_pushes_the_offline_family_owner() {
    let (mock, mock_addr) = spawn_mock_push().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let key_file = write_test_apns_key(dir.path());
    let ts = spawn_server_with_push(apns_config(mock_addr, key_file)).await;

    let (owner, _) = ts.register("owner", "Olive").await;
    // Default join policy is "approval", which is what this test needs.
    let (family_id, invite_code) = ts.create_family(&owner, "The Smiths").await;
    register_device(&ts, &owner, "ios", "owner-ios-token").await;

    let (requester, _) = ts.register("junior", "Junior").await;
    ts.join(&requester, &invite_code, "pending").await;

    let requests = mock
        .wait_for(1, |path| path.starts_with("/3/device/"))
        .await;
    let request = &requests[0];
    assert_eq!(request.path, "/3/device/owner-ios-token");
    assert_eq!(
        request.body,
        json!({
            "aps": {
                "alert": {"title": "The Smiths", "body": "Junior asked to join"},
                "sound": "default",
                "badge": 0,
            },
            "family_id": family_id,
            "kind": "join_request",
        })
    );
}

/// A note pinned to the wall reaches the family, and the author is not told
/// about their own note (protocol.md, "Board").
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_new_board_note_pushes_the_other_offline_members() {
    let (mock, mock_addr) = spawn_mock_push().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let key_file = write_test_apns_key(dir.path());
    let ts = spawn_server_with_push(apns_config(mock_addr, key_file)).await;

    let (owner, _) = ts.register("owner", "Olive").await;
    let (family_id, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    register_device(&ts, &owner, "ios", "owner-ios-token").await;

    let (member, _) = ts.register("junior", "Junior").await;
    ts.join(&member, &invite_code, "joined").await;
    // The author's own device must NOT be pushed, so register one.
    register_device(&ts, &member, "ios", "author-ios-token").await;

    let response = ts
        .post(
            &member,
            "/families/mine/board/notes",
            json!({"text": "Milk", "color": "yellow", "x": 0.5, "y": 0.5}),
        )
        .await;
    assert_eq!(response.status(), 201);
    let note: Value = response.json().await.expect("JSON");
    let note_id = note["note"]["id"].as_i64().expect("id");

    let requests = mock
        .wait_for(1, |path| path.starts_with("/3/device/"))
        .await;
    assert_eq!(
        requests.len(),
        1,
        "only the other member is pushed, never the author: {:?}",
        requests.iter().map(|r| r.path.clone()).collect::<Vec<_>>()
    );
    let request = &requests[0];
    assert_eq!(request.path, "/3/device/owner-ios-token");
    assert_eq!(
        request.body,
        json!({
            "aps": {
                "alert": {"title": "The Smiths — Junior", "body": "Milk"},
                "sound": "default",
                // A note is not a message and must not inflate the unread
                // badge — the board draws its own count.
                "badge": 0,
                "thread-id": format!("board-{family_id}"),
            },
            "family_id": family_id,
            "note_id": note_id,
            "kind": "board_note",
        })
    );
}

/// Moving somebody else's note is the shared act of tidying the wall, and
/// tidying must never notify.
#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn moving_a_note_never_pushes() {
    let (mock, mock_addr) = spawn_mock_push().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let key_file = write_test_apns_key(dir.path());
    let ts = spawn_server_with_push(apns_config(mock_addr, key_file)).await;

    let (owner, _) = ts.register("owner", "Olive").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    let (member, _) = ts.register("junior", "Junior").await;
    ts.join(&member, &invite_code, "joined").await;
    register_device(&ts, &owner, "ios", "owner-ios-token").await;

    let note: Value = ts
        .post(
            &member,
            "/families/mine/board/notes",
            json!({"text": "Milk", "color": "yellow", "x": 0.5, "y": 0.5}),
        )
        .await
        .json()
        .await
        .expect("JSON");
    let note_id = note["note"]["id"].as_i64().expect("id");
    // The creation push is the one we expect; wait for it so the move's
    // silence is a real observation rather than a race.
    mock.wait_for(1, |path| path.starts_with("/3/device/"))
        .await;

    let moved = ts
        .patch(
            &owner,
            &format!("/families/mine/board/notes/{note_id}"),
            json!({"x": 0.1, "y": 0.2}),
        )
        .await;
    assert_eq!(moved.status(), 200);

    tokio::time::sleep(Duration::from_millis(300)).await;
    let pushes = mock
        .requests()
        .into_iter()
        .filter(|r| r.path.starts_with("/3/device/"))
        .count();
    assert_eq!(pushes, 1, "the move must not have pushed anything");
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn an_approved_join_request_pushes_the_offline_requester() {
    let (mock, mock_addr) = spawn_mock_push().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let key_file = write_test_apns_key(dir.path());
    let ts = spawn_server_with_push(apns_config(mock_addr, key_file)).await;

    let (owner, _) = ts.register("owner", "Olive").await;
    let (family_id, invite_code) = ts.create_family(&owner, "The Smiths").await;
    let (requester, _) = ts.register("junior", "Junior").await;
    register_device(&ts, &requester, "ios", "requester-ios-token").await;
    ts.join(&requester, &invite_code, "pending").await;

    let list: Value = ts
        .get(&owner, "/families/join-requests")
        .await
        .json()
        .await
        .expect("join requests are JSON");
    let request_id = list["requests"][0]["id"].as_i64().expect("request id");
    let response = ts
        .post(
            &owner,
            &format!("/families/join-requests/{request_id}/approve"),
            json!({}),
        )
        .await;
    assert_eq!(response.status(), 200, "approving the join request");

    let requests = mock
        .wait_for(1, |path| path.starts_with("/3/device/"))
        .await;
    let request = &requests[0];
    assert_eq!(request.path, "/3/device/requester-ios-token");
    assert_eq!(
        request.body,
        json!({
            "aps": {
                "alert": {"title": "The Smiths",
                          "body": "You're in — welcome to The Smiths"},
                "sound": "default",
                "badge": 0,
            },
            "family_id": family_id,
            "kind": "joined",
        })
    );
}

// --- Minimal WebSocket client (the online-member test needs a live socket;
// --- mirrors the helpers in ws_flow.rs).

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
    // A pong proves the server-side connection task is registered — later
    // fan-outs cannot race past a connection that already answered a frame.
    ws.send(Message::text(json!({"type": "ping"}).to_string()))
        .await
        .expect("sending ping");
    let pong = next_frame_of_type(&mut ws, "pong").await;
    assert_eq!(pong, json!({"type": "pong"}));
    ws
}

async fn next_frame_of_type(ws: &mut WsClient, wanted: &str) -> Value {
    let deadline = tokio::time::Instant::now() + PUSH_WAIT;
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

/// A Mac is a third platform in the data and an iPhone on the wire: same
/// bundle id, same APNs topic, same connection. What must NOT happen is
/// the server dropping it because the routing filter only knew two names.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_macos_device_is_pushed_over_apns() {
    let (mock, mock_addr) = spawn_mock_push().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let key_file = write_test_apns_key(dir.path());
    let ts = spawn_server_with_push(apns_config(mock_addr, key_file)).await;

    let (owner, member, _member_id) = family_of_two(&ts).await;
    let chat_id = ts.family_chat_id(&owner).await;
    register_device(&ts, &member, "macos", "macos-token-1").await;

    let message_id = post_message_id(&ts, &owner, chat_id, "Dinner at 7?").await;
    let _ = message_id;

    let requests = mock
        .wait_for(1, |path| path.starts_with("/3/device/"))
        .await;
    assert_eq!(requests[0].path, "/3/device/macos-token-1");
}

/// And a platform nobody implements is still refused.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_unknown_platform_is_refused() {
    let ts = spawn_server().await;
    let (token, _) = ts.register("olive", "Olive").await;
    assert_error(
        ts.post(
            &token,
            "/devices",
            serde_json::json!({"platform": "toaster", "push_token": "t"}),
        )
        .await,
        400,
        "validation",
    )
    .await;
}

// --- One user, several devices.
//
// A desktop app holds its WebSocket open for as long as it is running, and
// for years the push gate asked whether the USER had a socket. A Mac is a
// socket, so a Mac left running answered that question for every phone on
// the account and silenced all of them at once — the phone in a pocket has
// no way to know the Mac is showing anything. The gate is per DEVICE now:
// the Mac stays quiet because ITS OWN session is the live one, and the
// phone, whose session is not connected, is pushed. Each of the four
// pushing events gets the same treatment, because a family that hears about
// messages but never about a board note is only half fixed.

/// Every device the mock was asked to wake, in arrival order.
fn woken_devices(mock: &MockPush) -> Vec<String> {
    mock.requests()
        .into_iter()
        .map(|r| r.path)
        .filter(|path| path.starts_with("/3/device/"))
        .collect()
}

/// Let any second push that was going to happen happen, so asserting on the
/// whole list is an observation rather than a race won.
async fn settle() {
    tokio::time::sleep(Duration::from_millis(300)).await;
}

/// Log in a second time as an already-registered user: a second session,
/// which is what a second device of the same person really is.
async fn second_session(ts: &TestServer, username: &str) -> String {
    ts.login(username, "password123").await
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_live_mac_does_not_silence_the_same_users_iphone() {
    let (mock, mock_addr) = spawn_mock_push().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let key_file = write_test_apns_key(dir.path());
    let ts = spawn_server_with_push(apns_config(mock_addr, key_file)).await;

    let (owner, _) = ts.register("owner", "Olive").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    let (mac, _member_id) = ts.register("junior", "Junior").await;
    ts.join(&mac, &invite_code, "joined").await;
    let phone = second_session(&ts, "junior").await;
    register_device(&ts, &mac, "macos", "junior-macos-token").await;
    register_device(&ts, &phone, "ios", "junior-ios-token").await;

    let chat_id = ts.family_chat_id(&owner).await;
    let mut mac_ws = connect_ws(&ts, &mac).await;
    let message_id = post_message_id(&ts, &owner, chat_id, "Dinner at 7?").await;
    // The Mac has it over the wire. The phone has not, and that is the
    // whole point of the push that must follow.
    next_frame_of_type(&mut mac_ws, "message").await;

    let requests = mock
        .wait_for(1, |path| path.starts_with("/3/device/"))
        .await;
    assert_eq!(
        requests[0].body,
        json!({
            "aps": {
                "alert": {"title": "The Smiths — Olive", "body": "Dinner at 7?"},
                "sound": "default",
                "badge": 1,
                "thread-id": format!("chat-{chat_id}"),
            },
            "chat_id": chat_id,
            "message_id": message_id,
            "kind": "message",
        })
    );
    settle().await;
    assert_eq!(
        woken_devices(&mock),
        vec!["/3/device/junior-ios-token".to_string()],
        "the phone is woken and the Mac, which already has the message, is not"
    );
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_live_mac_does_not_silence_the_same_users_iphone_for_a_board_note() {
    let (mock, mock_addr) = spawn_mock_push().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let key_file = write_test_apns_key(dir.path());
    let ts = spawn_server_with_push(apns_config(mock_addr, key_file)).await;

    let (mac, _) = ts.register("owner", "Olive").await;
    let (family_id, invite_code) = ts.create_family(&mac, "The Smiths").await;
    ts.set_open_policy(&mac).await;
    let phone = second_session(&ts, "owner").await;
    register_device(&ts, &mac, "macos", "olive-macos-token").await;
    register_device(&ts, &phone, "ios", "olive-ios-token").await;

    let (member, _) = ts.register("junior", "Junior").await;
    ts.join(&member, &invite_code, "joined").await;

    let mut mac_ws = connect_ws(&ts, &mac).await;
    let response = ts
        .post(
            &member,
            "/families/mine/board/notes",
            json!({"text": "Milk", "color": "yellow", "x": 0.5, "y": 0.5}),
        )
        .await;
    assert_eq!(response.status(), 201);
    let note: Value = response.json().await.expect("JSON");
    let note_id = note["note"]["id"].as_i64().expect("id");
    next_frame_of_type(&mut mac_ws, "board_note").await;

    let requests = mock
        .wait_for(1, |path| path.starts_with("/3/device/"))
        .await;
    assert_eq!(
        requests[0].body,
        json!({
            "aps": {
                "alert": {"title": "The Smiths — Junior", "body": "Milk"},
                "sound": "default",
                "badge": 0,
                "thread-id": format!("board-{family_id}"),
            },
            "family_id": family_id,
            "note_id": note_id,
            "kind": "board_note",
        })
    );
    settle().await;
    assert_eq!(
        woken_devices(&mock),
        vec!["/3/device/olive-ios-token".to_string()],
        "the note reaches the phone even though the Mac is looking at the board"
    );
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_live_mac_does_not_silence_the_owners_iphone_for_a_join_request() {
    let (mock, mock_addr) = spawn_mock_push().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let key_file = write_test_apns_key(dir.path());
    let ts = spawn_server_with_push(apns_config(mock_addr, key_file)).await;

    let (mac, _) = ts.register("owner", "Olive").await;
    // Default join policy is "approval", which is what this test needs.
    let (family_id, invite_code) = ts.create_family(&mac, "The Smiths").await;
    let phone = second_session(&ts, "owner").await;
    register_device(&ts, &mac, "macos", "olive-macos-token").await;
    register_device(&ts, &phone, "ios", "olive-ios-token").await;

    // A join request raises no frame, so the ping/pong inside `connect_ws`
    // is what proves the Mac's connection is registered before the request
    // is made.
    let _mac_ws = connect_ws(&ts, &mac).await;

    let (requester, _) = ts.register("junior", "Junior").await;
    ts.join(&requester, &invite_code, "pending").await;

    let requests = mock
        .wait_for(1, |path| path.starts_with("/3/device/"))
        .await;
    assert_eq!(
        requests[0].body,
        json!({
            "aps": {
                "alert": {"title": "The Smiths", "body": "Junior asked to join"},
                "sound": "default",
                "badge": 0,
            },
            "family_id": family_id,
            "kind": "join_request",
        })
    );
    settle().await;
    assert_eq!(
        woken_devices(&mock),
        vec!["/3/device/olive-ios-token".to_string()],
        "somebody knocking at the door must reach the owner's phone"
    );
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_live_mac_does_not_silence_the_requesters_iphone_when_approved() {
    let (mock, mock_addr) = spawn_mock_push().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let key_file = write_test_apns_key(dir.path());
    let ts = spawn_server_with_push(apns_config(mock_addr, key_file)).await;

    let (owner, _) = ts.register("owner", "Olive").await;
    let (family_id, invite_code) = ts.create_family(&owner, "The Smiths").await;
    let (mac, _) = ts.register("junior", "Junior").await;
    let phone = second_session(&ts, "junior").await;
    register_device(&ts, &mac, "macos", "junior-macos-token").await;
    register_device(&ts, &phone, "ios", "junior-ios-token").await;
    ts.join(&mac, &invite_code, "pending").await;

    let mut mac_ws = connect_ws(&ts, &mac).await;
    let list: Value = ts
        .get(&owner, "/families/join-requests")
        .await
        .json()
        .await
        .expect("join requests are JSON");
    let request_id = list["requests"][0]["id"].as_i64().expect("request id");
    let response = ts
        .post(
            &owner,
            &format!("/families/join-requests/{request_id}/approve"),
            json!({}),
        )
        .await;
    assert_eq!(response.status(), 200, "approving the join request");
    next_frame_of_type(&mut mac_ws, "member_joined").await;

    let requests = mock
        .wait_for(1, |path| path.starts_with("/3/device/"))
        .await;
    assert_eq!(
        requests[0].body,
        json!({
            "aps": {
                "alert": {"title": "The Smiths",
                          "body": "You're in — welcome to The Smiths"},
                "sound": "default",
                "badge": 0,
            },
            "family_id": family_id,
            "kind": "joined",
        })
    );
    settle().await;
    assert_eq!(
        woken_devices(&mock),
        vec!["/3/device/junior-ios-token".to_string()],
        "being let in is news the phone should carry too"
    );
}

// ---------------------------------------------------------------------------
// A signed-out device is not a push target. The per-device rule above errs
// towards the alert whenever the server cannot prove somebody is looking —
// and being SIGNED OUT is not one of those cases, it is a case where the
// server knows. Read together, "unknown ⇒ push it" and a device row that
// outlived its session made every logged-out phone a guaranteed target for
// family message bodies on its lock screen. Migration 0021 answers the
// revocation half (the row goes with the session) and `devices_for_users`
// answers the expiry half (the link has to name a session that is alive).
// ---------------------------------------------------------------------------

/// The session a device registered itself from, straight out of the
/// database — the only way to reach a session id from a test, since the
/// wire never carries one.
async fn session_of_device(ts: &TestServer, push_token: &str) -> Option<i64> {
    sqlx::query_scalar("SELECT session_id FROM devices WHERE push_token = $1")
        .bind(push_token)
        .fetch_one(&ts.state.pool)
        .await
        .expect("the device row exists")
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_device_whose_session_was_revoked_is_not_pushed() {
    let (mock, mock_addr) = spawn_mock_push().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let key_file = write_test_apns_key(dir.path());
    let ts = spawn_server_with_push(apns_config(mock_addr, key_file)).await;

    let (owner, phone, _) = family_of_two(&ts).await;
    // Two devices of the same person, one per session — and only one of
    // the two sessions is about to be revoked, which is what makes the
    // assertion below an observation rather than "nothing happened".
    let mac = second_session(&ts, "junior").await;
    register_device(&ts, &phone, "ios", "junior-ios-token").await;
    register_device(&ts, &mac, "macos", "junior-macos-token").await;

    // Changing the password from the Mac revokes every OTHER session —
    // here, the phone's.
    assert_eq!(
        ts.post(
            &mac,
            "/me/password",
            json!({"current_password": "password123", "new_password": "brand-new-one"}),
        )
        .await
        .status(),
        204
    );

    let chat_id = ts.family_chat_id(&owner).await;
    post_message_id(&ts, &owner, chat_id, "Dinner at 7?").await;

    // The Mac is still signed in and gets its banner; the phone was signed
    // out and must never hear another word.
    mock.wait_for(1, |path| path.starts_with("/3/device/"))
        .await;
    settle().await;
    assert_eq!(
        woken_devices(&mock),
        vec!["/3/device/junior-macos-token".to_string()],
        "a device whose session was revoked is not woken, and the one that \
         is still signed in still is"
    );
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn an_owner_resetting_a_members_password_silences_their_lock_screen() {
    let (mock, mock_addr) = spawn_mock_push().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let key_file = write_test_apns_key(dir.path());
    let ts = spawn_server_with_push(apns_config(mock_addr, key_file)).await;

    let (owner, member, member_id) = family_of_two(&ts).await;
    register_device(&ts, &member, "ios", "junior-ios-token").await;

    // The endpoint whose documented purpose is that "a device somebody else
    // is holding stops working the moment the reset lands". A lock screen
    // is part of that device.
    assert_eq!(
        ts.post(
            &owner,
            &format!("/families/members/{member_id}/password"),
            json!({"new_password": "fresh-start-42"}),
        )
        .await
        .status(),
        204
    );

    let chat_id = ts.family_chat_id(&owner).await;
    post_message_id(&ts, &owner, chat_id, "Dinner at 7?").await;
    settle().await;
    assert!(
        woken_devices(&mock).is_empty(),
        "the reset revoked every session that phone had, so nothing may \
         reach it: {:?}",
        woken_devices(&mock)
    );

    // And the same phone in the hands of the person who still knows the
    // password comes straight back: signing in and re-registering is all it
    // takes, which is also what proves the silence above was the rule and
    // not a push that never worked in this test.
    let back = ts.login("junior", "fresh-start-42").await;
    register_device(&ts, &back, "ios", "junior-ios-token").await;
    post_message_id(&ts, &owner, chat_id, "Still on for 7?").await;
    mock.wait_for(1, |path| path.starts_with("/3/device/"))
        .await;
    settle().await;
    assert_eq!(
        woken_devices(&mock),
        vec!["/3/device/junior-ios-token".to_string()],
        "a re-registered device is a push target again"
    );
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_device_whose_session_has_expired_is_not_pushed() {
    let (mock, mock_addr) = spawn_mock_push().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let key_file = write_test_apns_key(dir.path());
    let ts = spawn_server_with_push(apns_config(mock_addr, key_file)).await;

    let (owner, phone, _) = family_of_two(&ts).await;
    let mac = second_session(&ts, "junior").await;
    register_device(&ts, &phone, "ios", "junior-ios-token").await;
    register_device(&ts, &mac, "macos", "junior-macos-token").await;

    // Nothing deletes an expired session — the sliding expiry only ever
    // moves `expires_at` forward, and authentication reads it. So a session
    // that has run out is still a row, and the device still names it. Aged
    // by hand here because the alternative is a test that waits days.
    let expired = session_of_device(&ts, "junior-ios-token")
        .await
        .expect("the phone registered from a session");
    sqlx::query("UPDATE sessions SET expires_at = now() - INTERVAL '1 day' WHERE id = $1")
        .bind(expired)
        .execute(&ts.state.pool)
        .await
        .expect("ageing the session");
    // Expired means signed out: the bearer token no longer authenticates.
    assert_eq!(ts.get(&phone, "/me").await.status(), 401);

    let chat_id = ts.family_chat_id(&owner).await;
    post_message_id(&ts, &owner, chat_id, "Dinner at 7?").await;
    mock.wait_for(1, |path| path.starts_with("/3/device/"))
        .await;
    settle().await;
    assert_eq!(
        woken_devices(&mock),
        vec!["/3/device/junior-macos-token".to_string()],
        "a device whose session has expired is as signed out as one that \
         was revoked"
    );
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn a_device_from_before_the_session_column_is_still_pushed() {
    let (mock, mock_addr) = spawn_mock_push().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let key_file = write_test_apns_key(dir.path());
    let ts = spawn_server_with_push(apns_config(mock_addr, key_file)).await;

    let (owner, member, _) = family_of_two(&ts).await;
    register_device(&ts, &member, "ios", "junior-ios-token").await;
    // What a row written before migration 0020 looks like: a real device
    // with a real token that never told anybody which session it was. It
    // must keep the benefit of the doubt — the whole reason the column is
    // nullable — and it heals the moment the app launches again.
    sqlx::query("UPDATE devices SET session_id = NULL WHERE push_token = $1")
        .bind("junior-ios-token")
        .execute(&ts.state.pool)
        .await
        .expect("orphaning the device row");
    assert_eq!(session_of_device(&ts, "junior-ios-token").await, None);

    let chat_id = ts.family_chat_id(&owner).await;
    post_message_id(&ts, &owner, chat_id, "Dinner at 7?").await;
    mock.wait_for(1, |path| path.starts_with("/3/device/"))
        .await;
    settle().await;
    assert_eq!(
        woken_devices(&mock),
        vec!["/3/device/junior-ios-token".to_string()],
        "an unattributed device is still woken; only a session that was \
         revoked or has expired silences one"
    );
}

// --- Ringing an offline phone (docs/protocol.md, "Incoming calls").
//
// A call cannot ring a phone whose app is not running with an alert
// notification. It takes the VoIP path on iOS and a data-only message on
// Android, and the exact wire shape is what these two tests pin against the
// mock.

/// Get-or-create the caller's direct chat with `peer_id`; returns its id.
async fn direct_chat(ts: &TestServer, token: &str, peer_id: i64) -> i64 {
    let response = ts
        .post(token, "/chats/direct", json!({"user_id": peer_id}))
        .await;
    assert_eq!(response.status(), 200, "creating a direct chat");
    let body: Value = response.json().await.expect("direct chat JSON");
    body["chat"]["id"].as_i64().expect("chat id")
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_offline_ios_member_is_rung_over_apns_voip() {
    let (mock, mock_addr) = spawn_mock_push().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let key_file = write_test_apns_key(dir.path());
    let ts = spawn_server_with_push(apns_config(mock_addr, key_file)).await;

    let (owner, member, member_id) = family_of_two(&ts).await;
    // The member is signed in on an iPhone with a VoIP token — but has no
    // socket open, so the offer must reach it by push.
    let response = ts
        .post(
            &member,
            "/devices",
            json!({"platform": "ios", "push_token": "apns-alert", "voip_token": "apns-voip"}),
        )
        .await;
    assert_eq!(response.status(), 201);
    let chat_id = direct_chat(&ts, &owner, member_id).await;

    let mut caller = connect_ws(&ts, &owner).await;
    let call_id = Uuid::new_v4().to_string();
    caller
        .send(Message::text(
            json!({"type": "call_offer", "call_id": call_id, "chat_id": chat_id, "sdp": "v=0"})
                .to_string(),
        ))
        .await
        .expect("sending the offer");

    // The push goes to the VOIP token, at the voip topic and push type.
    let requests = mock.wait_for(1, |path| path == "/3/device/apns-voip").await;
    let request = &requests[0];
    assert_eq!(
        request.header("apns-topic"),
        Some("me.nettrash.familyconnect.voip")
    );
    assert_eq!(request.header("apns-push-type"), Some("voip"));
    assert_eq!(request.header("apns-priority"), Some("10"));
    assert!(
        request.header("apns-expiration").is_some(),
        "a call push must expire when the ring does"
    );
    assert_eq!(
        request.body,
        json!({
            "kind": "call",
            "call_id": call_id,
            "chat_id": chat_id,
            "from_user_id": member_id_of(&ts, &owner).await,
            "caller_name": "Olive",
        })
    );
    // And nothing was sent to the ALERT token — a call is not a banner.
    assert!(
        mock.requests()
            .iter()
            .all(|r| r.path != "/3/device/apns-alert"),
        "the alert token must not be used for a call"
    );
}

/// The caller's own user id, read from /me — the `from_user_id` a call push
/// carries is the caller's, not the callee's.
async fn member_id_of(ts: &TestServer, token: &str) -> i64 {
    let me: Value = ts.get(token, "/me").await.json().await.expect("me JSON");
    me["user"]["id"].as_i64().expect("user id")
}

/// A VIDEO call's VoIP push carries `"video": true` (a JSON boolean) beside
/// the call fields — it is what makes the woken iPhone ring with a camera
/// UI, `hasVideo` on CallKit included (docs/protocol.md, "Incoming calls").
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_offline_ios_member_is_rung_with_the_video_flag_over_apns_voip() {
    let (mock, mock_addr) = spawn_mock_push().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let key_file = write_test_apns_key(dir.path());
    let ts = spawn_server_with_push(apns_config(mock_addr, key_file)).await;

    let (owner, member, member_id) = family_of_two(&ts).await;
    let response = ts
        .post(
            &member,
            "/devices",
            json!({"platform": "ios", "push_token": "apns-alert", "voip_token": "apns-voip"}),
        )
        .await;
    assert_eq!(response.status(), 201);
    let chat_id = direct_chat(&ts, &owner, member_id).await;
    let owner_id = member_id_of(&ts, &owner).await;

    let mut caller = connect_ws(&ts, &owner).await;
    let call_id = Uuid::new_v4().to_string();
    caller
        .send(Message::text(
            json!({"type": "call_offer", "call_id": call_id, "chat_id": chat_id,
                   "sdp": "v=0", "video": true})
            .to_string(),
        ))
        .await
        .expect("sending the offer");

    let requests = mock.wait_for(1, |path| path == "/3/device/apns-voip").await;
    assert_eq!(
        requests[0].body,
        json!({
            "kind": "call",
            "call_id": call_id,
            "chat_id": chat_id,
            "from_user_id": owner_id,
            "caller_name": "Olive",
            "video": true,
        }),
        "the VoIP payload carries the flag as a JSON boolean"
    );
}

/// The FCM shape of the same thing: `data` gains `"video": "true"` — a
/// STRING, because FCM `data` values are strings (docs/protocol.md,
/// "Incoming calls").
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_offline_android_member_is_rung_with_the_video_flag_over_fcm() {
    let (mock, mock_addr) = spawn_mock_push().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let credentials = write_test_service_account(
        dir.path(),
        "test-project",
        &format!("http://{mock_addr}/oauth/token"),
    );
    let ts = spawn_server_with_push(fcm_config(mock_addr, credentials)).await;

    let (owner, member, member_id) = family_of_two(&ts).await;
    register_device(&ts, &member, "android", "android-video-token").await;
    let chat_id = direct_chat(&ts, &owner, member_id).await;
    let owner_id = member_id_of(&ts, &owner).await;

    let mut caller = connect_ws(&ts, &owner).await;
    let call_id = Uuid::new_v4().to_string();
    caller
        .send(Message::text(
            json!({"type": "call_offer", "call_id": call_id, "chat_id": chat_id,
                   "sdp": "v=0", "video": true})
            .to_string(),
        ))
        .await
        .expect("sending the offer");

    mock.wait_for(1, |path| path == "/oauth/token").await;
    let sends = mock
        .wait_for(1, |path| path.ends_with("messages:send"))
        .await;
    assert_eq!(
        sends[0].body,
        json!({"message": {
            "token": "android-video-token",
            "data": {
                "kind": "call",
                "call_id": call_id,
                "chat_id": chat_id.to_string(),
                "from_user_id": owner_id.to_string(),
                "caller_name": "Olive",
                "video": "true",
            },
            "android": {"priority": "HIGH", "ttl": "45s"},
        }}),
        "data-only, and the video flag is the string \"true\" like every data value"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_offline_android_member_is_rung_over_fcm_data_only() {
    let (mock, mock_addr) = spawn_mock_push().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let credentials = write_test_service_account(
        dir.path(),
        "test-project",
        &format!("http://{mock_addr}/oauth/token"),
    );
    let ts = spawn_server_with_push(fcm_config(mock_addr, credentials)).await;

    let (owner, member, member_id) = family_of_two(&ts).await;
    register_device(&ts, &member, "android", "android-call-token").await;
    let chat_id = direct_chat(&ts, &owner, member_id).await;
    let owner_id = member_id_of(&ts, &owner).await;

    let mut caller = connect_ws(&ts, &owner).await;
    let call_id = Uuid::new_v4().to_string();
    caller
        .send(Message::text(
            json!({"type": "call_offer", "call_id": call_id, "chat_id": chat_id, "sdp": "v=0"})
                .to_string(),
        ))
        .await
        .expect("sending the offer");

    mock.wait_for(1, |path| path == "/oauth/token").await;
    let sends = mock
        .wait_for(1, |path| path.ends_with("messages:send"))
        .await;
    assert_eq!(
        sends[0].body,
        json!({"message": {
            "token": "android-call-token",
            "data": {
                "kind": "call",
                "call_id": call_id,
                "chat_id": chat_id.to_string(),
                "from_user_id": owner_id.to_string(),
                "caller_name": "Olive",
            },
            "android": {"priority": "HIGH", "ttl": "45s"},
        }}),
        "a call push is data only — no notification block — so the app rings"
    );
}
