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
                "notification": {"channel_id": "messages", "tag": format!("chat-{chat_id}")},
            },
        }})
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
