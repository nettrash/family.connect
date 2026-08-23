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
use serde_json::Value;

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
