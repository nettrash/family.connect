//! Shared integration-test harness.
//!
//! Each test spawns a fully real server: a scratch database named
//! `family_connect_test_<pid>_<counter>` (created here, dropped on
//! teardown), the hand-rolled migrator, and `axum::serve` of the production
//! router on an ephemeral loopback port. Connection parameters come from
//! PGHOST/PGPORT/PGUSER/PGPASSWORD with the usual local-postgres defaults.
//! The push seam is replaced by a recording sender so tests can assert the
//! offline-member hook fires — unless a test supplies a `[push]` config via
//! `spawn_server_with_push`, in which case the real APNs/FCM dispatcher is
//! built (pointed at a mock server). Throwaway credentials for that mode
//! come from `write_test_apns_key` / `write_test_service_account`.

// Each integration-test binary compiles this module independently and uses
// a different subset of the helpers; unused ones are expected, not dead.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;
use serde_json::{Value, json};

use family_connect::app::build_router;
use family_connect::config::{Config, DatabaseConfig, PushConfig};
use family_connect::push::{DevicePush, PushSender};
use family_connect::push_payload::Notification;
use family_connect::state::AppState;
use family_connect::{db, migrate, push};

/// One recorded `notify` call.
#[derive(Debug, Clone)]
pub struct PushCall {
    pub devices: Vec<DevicePush>,
    pub note: Notification,
}

/// Test double for the push seam: records every call instead of delivering.
#[derive(Default)]
pub struct RecordingPushSender {
    calls: Mutex<Vec<PushCall>>,
}

impl RecordingPushSender {
    pub fn calls(&self) -> Vec<PushCall> {
        self.calls.lock().expect("push call log lock").clone()
    }
}

#[async_trait]
impl PushSender for RecordingPushSender {
    async fn notify(&self, devices: &[DevicePush], note: &Notification) -> Vec<i64> {
        self.calls
            .lock()
            .expect("push call log lock")
            .push(PushCall {
                devices: devices.to_vec(),
                note: note.clone(),
            });
        Vec::new()
    }
}

/// A live server under test plus everything needed to talk to and tear it
/// down. Dropping it aborts the server and drops the scratch database.
pub struct TestServer {
    /// REST base, e.g. `http://127.0.0.1:49152/api/v1`.
    pub base_url: String,
    /// WebSocket endpoint, e.g. `ws://127.0.0.1:49152/api/v1/ws`.
    pub ws_url: String,
    pub client: reqwest::Client,
    pub state: AppState,
    pub push: Arc<RecordingPushSender>,
    db_name: String,
    admin: DatabaseConfig,
    server_task: tokio::task::JoinHandle<()>,
}

static DB_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

/// Connection parameters for the maintenance database, from the standard
/// PG* environment variables.
fn admin_config() -> DatabaseConfig {
    DatabaseConfig {
        host: env_or("PGHOST", "127.0.0.1"),
        port: env_or("PGPORT", "5432")
            .parse()
            .expect("PGPORT must be a port number"),
        user: env_or("PGUSER", "postgres"),
        password: env_or("PGPASSWORD", "postgres"),
        database: "postgres".to_string(),
        max_connections: 2,
        connect_timeout_secs: 5,
    }
}

/// Boot a complete server against a fresh scratch database, with the push
/// seam replaced by the recording test double.
pub async fn spawn_server() -> TestServer {
    spawn_server_inner(None).await
}

/// Boot a server whose push seam is the *real* dispatcher built from
/// `push_cfg` — endpoints pointed at a mock by the caller. The recording
/// sender in `TestServer::push` is not wired in this mode.
pub async fn spawn_server_with_push(push_cfg: PushConfig) -> TestServer {
    spawn_server_inner(Some(push_cfg)).await
}

async fn spawn_server_inner(push_cfg: Option<PushConfig>) -> TestServer {
    let admin = admin_config();
    let db_name = format!(
        "family_connect_test_{}_{}",
        std::process::id(),
        DB_COUNTER.fetch_add(1, Ordering::SeqCst)
    );

    let admin_pool = db::connect(&admin)
        .await
        .expect("connecting to the maintenance database (set PGHOST/PGPORT/PGUSER/PGPASSWORD)");
    sqlx::query(&format!("CREATE DATABASE \"{db_name}\""))
        .execute(&admin_pool)
        .await
        .expect("creating the scratch database");
    admin_pool.close().await;

    let mut cfg = Config {
        database: DatabaseConfig {
            database: db_name.clone(),
            ..admin.clone()
        },
        ..Config::default()
    };
    cfg.server.bind = "127.0.0.1:0".to_string();
    if let Some(push_cfg) = push_cfg {
        cfg.push = push_cfg;
    }
    cfg.validate().expect("test config is valid");

    let pool = db::connect(&cfg.database)
        .await
        .expect("connecting to the scratch database");
    migrate::run(&pool).await.expect("running migrations");

    let push = Arc::new(RecordingPushSender::default());
    let push_sender: Arc<dyn PushSender> = if cfg.push.apns.is_some() || cfg.push.fcm.is_some() {
        push::build(&cfg.push).expect("building the push dispatcher")
    } else {
        push.clone()
    };
    let state = AppState::new(pool, Arc::new(cfg), push_sender);
    let router = build_router(state.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("binding an ephemeral test port");
    let addr = listener.local_addr().expect("reading the bound address");
    let server_task = tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("test server crashed");
    });

    TestServer {
        base_url: format!("http://{addr}/api/v1"),
        ws_url: format!("ws://{addr}/api/v1/ws"),
        client: reqwest::Client::new(),
        state,
        push,
        db_name,
        admin,
        server_task,
    }
}

impl TestServer {
    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    /// POST a JSON body with a bearer token.
    pub async fn post(&self, token: &str, path: &str, body: Value) -> reqwest::Response {
        self.client
            .post(self.url(path))
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .expect("request sends")
    }

    /// POST a JSON body without authentication.
    pub async fn post_unauth(&self, path: &str, body: Value) -> reqwest::Response {
        self.client
            .post(self.url(path))
            .json(&body)
            .send()
            .await
            .expect("request sends")
    }

    pub async fn get(&self, token: &str, path: &str) -> reqwest::Response {
        self.client
            .get(self.url(path))
            .bearer_auth(token)
            .send()
            .await
            .expect("request sends")
    }

    pub async fn put(&self, token: &str, path: &str, body: Value) -> reqwest::Response {
        self.client
            .put(self.url(path))
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .expect("request sends")
    }

    /// Raw-body PUT — the avatar upload is the one endpoint that takes
    /// bytes rather than JSON.
    pub async fn put_bytes(
        &self,
        token: &str,
        path: &str,
        content_type: &str,
        body: Vec<u8>,
    ) -> reqwest::Response {
        self.client
            .put(self.url(path))
            .bearer_auth(token)
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .body(body)
            .send()
            .await
            .expect("request sends")
    }

    /// GET with extra headers, for conditional requests.
    pub async fn get_with(
        &self,
        token: &str,
        path: &str,
        headers: &[(&str, &str)],
    ) -> reqwest::Response {
        let mut request = self.client.get(self.url(path)).bearer_auth(token);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        request.send().await.expect("request sends")
    }

    pub async fn patch(&self, token: &str, path: &str, body: Value) -> reqwest::Response {
        self.client
            .patch(self.url(path))
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .expect("request sends")
    }

    pub async fn delete(&self, token: &str, path: &str) -> reqwest::Response {
        self.client
            .delete(self.url(path))
            .bearer_auth(token)
            .send()
            .await
            .expect("request sends")
    }

    /// Register a user; returns `(token, user_id)`.
    pub async fn register(&self, username: &str, display_name: &str) -> (String, i64) {
        let response = self
            .post_unauth(
                "/auth/register",
                json!({
                    "username": username,
                    "display_name": display_name,
                    "password": "password123",
                }),
            )
            .await;
        assert_eq!(response.status(), 201, "registering {username}");
        let body: Value = response.json().await.expect("register response is JSON");
        (
            body["token"].as_str().expect("token present").to_string(),
            body["user"]["id"].as_i64().expect("user id present"),
        )
    }

    /// Create a family as `token`; returns `(family_id, invite_code)`.
    pub async fn create_family(&self, token: &str, name: &str) -> (i64, String) {
        let response = self.post(token, "/families", json!({"name": name})).await;
        assert_eq!(response.status(), 201, "creating family {name}");
        let body: Value = response.json().await.expect("family response is JSON");
        (
            body["family"]["id"].as_i64().expect("family id present"),
            body["family"]["invite_code"]
                .as_str()
                .expect("invite code present for the owner")
                .to_string(),
        )
    }

    /// Flip the caller's family to the `open` join policy.
    pub async fn set_open_policy(&self, owner_token: &str) {
        let response = self
            .patch(
                owner_token,
                "/families/mine",
                json!({"join_policy": "open"}),
            )
            .await;
        assert_eq!(response.status(), 200, "patching join policy");
    }

    /// Join a family by invite code, asserting the expected status string.
    pub async fn join(&self, token: &str, invite_code: &str, expected_status: &str) {
        let response = self
            .post(token, "/families/join", json!({"invite_code": invite_code}))
            .await;
        assert_eq!(response.status(), 200, "joining a family");
        let body: Value = response.json().await.expect("join response is JSON");
        assert_eq!(body["status"], expected_status);
    }

    /// The caller's family chat id, from `GET /chats`.
    pub async fn family_chat_id(&self, token: &str) -> i64 {
        let response = self.get(token, "/chats").await;
        assert_eq!(response.status(), 200, "listing chats");
        let body: Value = response.json().await.expect("chats response is JSON");
        body["chats"]
            .as_array()
            .expect("chats is an array")
            .iter()
            .find(|entry| entry["chat"]["kind"] == "family")
            .expect("the family chat is always listed")["chat"]["id"]
            .as_i64()
            .expect("chat id present")
    }

    /// Post a message over REST; returns the response for status checks.
    pub async fn post_message(
        &self,
        token: &str,
        chat_id: i64,
        client_msg_id: &str,
        body: &str,
    ) -> reqwest::Response {
        self.post(
            token,
            &format!("/chats/{chat_id}/messages"),
            json!({"client_msg_id": client_msg_id, "body": body}),
        )
        .await
    }
}

/// Write a throwaway P-256 private key in the shape of an Apple `.p8` file;
/// returns its path. The caller owns `dir` and keeps it alive.
pub fn write_test_apns_key(dir: &Path) -> PathBuf {
    use p256::pkcs8::{EncodePrivateKey, LineEnding};
    let key = p256::SecretKey::random(&mut rand::rngs::OsRng);
    let pem = key
        .to_pkcs8_pem(LineEnding::LF)
        .expect("serializing the test P-256 key");
    let path = dir.join("AuthKey_KEYID12345.p8");
    std::fs::write(&path, pem.as_bytes()).expect("writing the test .p8");
    path
}

/// Write a fake Google service-account JSON whose `token_uri` points at the
/// test's mock server; returns its path. RSA-2048 generation is expensive
/// (ring refuses anything shorter for RS256), so one key is generated per
/// test binary and reused.
pub fn write_test_service_account(dir: &Path, project_id: &str, token_uri: &str) -> PathBuf {
    static RSA_PEM: OnceLock<String> = OnceLock::new();
    let private_pem = RSA_PEM.get_or_init(|| {
        use rsa::pkcs8::{EncodePrivateKey, LineEnding};
        rsa::RsaPrivateKey::new(&mut rand::rngs::OsRng, 2048)
            .expect("generating the test RSA key")
            .to_pkcs8_pem(LineEnding::LF)
            .expect("serializing the test RSA key")
            .to_string()
    });
    let document = json!({
        "type": "service_account",
        "project_id": project_id,
        "private_key_id": "test-key-id",
        "private_key": private_pem,
        "client_email": "push@test-project.iam.gserviceaccount.com",
        "token_uri": token_uri,
    });
    let path = dir.join("service-account.json");
    std::fs::write(&path, document.to_string()).expect("writing the test service account");
    path
}

/// Assert a response is the protocol error shape with the expected code.
pub async fn assert_error(response: reqwest::Response, status: u16, code: &str) {
    assert_eq!(response.status(), status, "unexpected status for {code}");
    let body: Value = response.json().await.expect("error body is JSON");
    assert_eq!(body["error"]["code"], code, "unexpected error code: {body}");
    assert!(
        body["error"]["message"].is_string(),
        "error message must be a string: {body}"
    );
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.server_task.abort();
        // Async teardown from a sync Drop: run it on a throwaway thread with
        // its own runtime. DROP DATABASE ... WITH (FORCE) (PostgreSQL 13+)
        // kicks any straggling connections our aborted server left behind.
        let db_name = self.db_name.clone();
        let admin = self.admin.clone();
        let pool = self.state.pool.clone();
        let teardown = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("teardown runtime");
            rt.block_on(async move {
                pool.close().await;
                if let Ok(admin_pool) = db::connect(&admin).await {
                    let _ = sqlx::query(&format!(
                        "DROP DATABASE IF EXISTS \"{db_name}\" WITH (FORCE)"
                    ))
                    .execute(&admin_pool)
                    .await;
                    admin_pool.close().await;
                }
            });
        });
        let _ = teardown.join();
    }
}
