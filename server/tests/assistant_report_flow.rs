//! Reporting the assistant (docs/protocol.md, "Reporting the assistant").
//!
//! This path exists because the assistant writes new text and a member has to
//! be able to say that what it wrote was wrong — a product duty, and the one
//! Microsoft Store policy 11.16 held 1.1 in certification for.
//!
//! The two things worth testing are the two that would be quietly wrong: that
//! the report reaches the OPERATOR and never the family owner, and that an id
//! the caller cannot see is answered exactly as one that does not exist.
//!
//! No reply is generated here — that needs the real provider — so the
//! assistant's messages are planted with SQL, which is also the only way to
//! get one into a chat belonging to somebody else.

mod common;

use common::{TestServer, assert_error, spawn_server_with_config};
use serde_json::{Value, json};

/// A fresh client id, as `report_flow` does it: the column is a real uuid.
fn uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

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

async fn assistant_id(ts: &TestServer) -> i64 {
    family_connect::handlers_ai::assistant_user_id(&ts.state)
        .await
        .expect("the query runs")
        .expect("migration 0015 inserted the assistant account")
}

/// The chat of a kind, from the caller's own list.
async fn chat_id(ts: &TestServer, token: &str, kind: &str) -> i64 {
    let body: Value = ts.get(token, "/chats").await.json().await.expect("JSON");
    body["chats"]
        .as_array()
        .expect("chats")
        .iter()
        .find(|entry| entry["chat"]["kind"] == kind)
        .map(|entry| entry["chat"]["id"].as_i64().expect("id"))
        .unwrap_or_else(|| panic!("no {kind} chat in the list"))
}

/// An assistant reply, planted. Returns its message id.
async fn plant_reply(ts: &TestServer, chat_id: i64, body: &str) -> i64 {
    let assistant = assistant_id(ts).await;
    sqlx::query_scalar::<_, i64>(
        "INSERT INTO messages (chat_id, sender_id, body, client_msg_id)
         VALUES ($1, $2, $3, gen_random_uuid())
         RETURNING id",
    )
    .bind(chat_id)
    .bind(assistant)
    .bind(body)
    .fetch_one(&ts.state.pool)
    .await
    .expect("plant an assistant reply")
}

/// The whole of the happy path: the reply is frozen into the row, the surface
/// is recorded, and the reporter's own note survives.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_private_reply_can_be_reported_and_is_frozen_into_the_row() {
    let ts = server_with_assistant().await;
    let (member, _) = ts.register("nora", "Nora").await;
    ts.create_family(&member, "The Harpers").await;
    let ai = chat_id(&ts, &member, "ai").await;
    let reply = plant_reply(&ts, ai, "Your grandmother was born in 1812.").await;

    let response = ts
        .post(
            &member,
            "/reports/assistant",
            json!({"message_id": reply, "reason": "other", "note": "It invented a person."}),
        )
        .await;
    assert_eq!(response.status(), 201);
    let body: Value = response.json().await.expect("JSON");
    let report = &body["report"];
    assert_eq!(report["message_id"], reply);
    assert_eq!(
        report["message_excerpt"],
        "Your grandmother was born in 1812."
    );
    assert_eq!(report["chat_kind"], "ai", "the private thread names itself");
    assert_eq!(report["reason"], "other");
    assert_eq!(report["note"], "It invented a person.");
    assert!(report["id"].is_i64());
    assert!(report["created_at"].is_string());
}

/// THE PRIVACY RULE, and the reason this is not a member report: the owner's
/// inbox must never show it, or a private thread's words would reach the very
/// person the thread exists to keep them from.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_family_owner_never_sees_an_assistant_report() {
    let ts = server_with_assistant().await;
    let (owner, _) = ts.register("olive", "Olive").await;
    let (_, code) = ts.create_family(&owner, "The Harpers").await;
    let (member, _) = ts.register("nora", "Nora").await;
    ts.set_open_policy(&owner).await;
    ts.join(&member, &code, "joined").await;

    let ai = chat_id(&ts, &member, "ai").await;
    let reply = plant_reply(&ts, ai, "Something the owner must not read.").await;
    let response = ts
        .post(
            &member,
            "/reports/assistant",
            json!({"message_id": reply, "reason": "inappropriate"}),
        )
        .await;
    assert_eq!(response.status(), 201);

    let inbox: Value = ts
        .get(&owner, "/families/reports")
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(
        inbox["reports"].as_array().map(Vec::len),
        Some(0),
        "an assistant report is the operator's, and appears in no client read: {inbox}"
    );
}

/// An `@ai` answer in the family chat is reportable too, and says which
/// surface it came from — the whole family could already read that one.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_family_chat_answer_is_reportable_and_says_so() {
    let ts = server_with_assistant().await;
    let (member, _) = ts.register("nora", "Nora").await;
    ts.create_family(&member, "The Harpers").await;
    let family = chat_id(&ts, &member, "family").await;
    let reply = plant_reply(&ts, family, "Sunday is the 20th of September.").await;

    let response = ts
        .post(
            &member,
            "/reports/assistant",
            json!({"message_id": reply, "reason": "spam"}),
        )
        .await;
    assert_eq!(response.status(), 201);
    let body: Value = response.json().await.expect("JSON");
    assert_eq!(body["report"]["chat_kind"], "family");
    assert!(
        body["report"]["note"].is_null(),
        "no note was sent, so none comes back: {body}"
    );
}

/// A second tap is the same report: the stored row comes back, the stored
/// reason is not overwritten, and there is only ever one row.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn reporting_the_same_reply_twice_is_one_report() {
    let ts = server_with_assistant().await;
    let (member, _) = ts.register("nora", "Nora").await;
    ts.create_family(&member, "The Harpers").await;
    let ai = chat_id(&ts, &member, "ai").await;
    let reply = plant_reply(&ts, ai, "Twice-reported.").await;

    let first: Value = ts
        .post(
            &member,
            "/reports/assistant",
            json!({"message_id": reply, "reason": "harassment"}),
        )
        .await
        .json()
        .await
        .expect("JSON");
    let again = ts
        .post(
            &member,
            "/reports/assistant",
            json!({"message_id": reply, "reason": "spam"}),
        )
        .await;
    assert_eq!(again.status(), 200, "a repeat is not a second row");
    let second: Value = again.json().await.expect("JSON");
    assert_eq!(second["report"]["id"], first["report"]["id"]);
    assert_eq!(
        second["report"]["reason"], "harassment",
        "the stored reason stands, whatever the repeat asked for"
    );

    let rows = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM assistant_reports")
        .fetch_one(&ts.state.pool)
        .await
        .expect("count");
    assert_eq!(rows, 1);
}

/// Everything the endpoint must refuse, and the two that must be refused
/// IDENTICALLY: a member's own message and a reply in somebody else's private
/// thread are both "no such message", so the answer never confirms that an id
/// exists elsewhere.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn only_an_assistant_reply_this_member_can_see_may_be_reported() {
    let ts = server_with_assistant().await;
    let (owner, _) = ts.register("olive", "Olive").await;
    let (_, code) = ts.create_family(&owner, "The Harpers").await;
    let (member, _) = ts.register("nora", "Nora").await;
    ts.set_open_policy(&owner).await;
    ts.join(&member, &code, "joined").await;

    let family = chat_id(&ts, &member, "family").await;
    let mine: Value = ts
        .post_message(&member, family, &uuid(), "A member wrote this.")
        .await
        .json()
        .await
        .expect("JSON");
    let my_message = mine["message"]["id"].as_i64().expect("id");

    // A member's message, through the assistant's door.
    assert_error(
        ts.post(
            &member,
            "/reports/assistant",
            json!({"message_id": my_message, "reason": "other"}),
        )
        .await,
        404,
        "message_not_found",
    )
    .await;

    // Somebody else's private thread: the same answer, deliberately.
    let owner_ai = chat_id(&ts, &owner, "ai").await;
    let not_mine = plant_reply(&ts, owner_ai, "The owner's own thread.").await;
    assert_error(
        ts.post(
            &member,
            "/reports/assistant",
            json!({"message_id": not_mine, "reason": "other"}),
        )
        .await,
        404,
        "message_not_found",
    )
    .await;

    // An id that exists nowhere at all: still the same answer.
    assert_error(
        ts.post(
            &member,
            "/reports/assistant",
            json!({"message_id": 9_999_999, "reason": "other"}),
        )
        .await,
        404,
        "message_not_found",
    )
    .await;

    // The vocabulary is the product's four words.
    let ai = chat_id(&ts, &member, "ai").await;
    let reply = plant_reply(&ts, ai, "A reply.").await;
    assert_error(
        ts.post(
            &member,
            "/reports/assistant",
            json!({"message_id": reply, "reason": "made-up"}),
        )
        .await,
        400,
        "validation",
    )
    .await;

    // A note is free text, but not unbounded.
    assert_error(
        ts.post(
            &member,
            "/reports/assistant",
            json!({"message_id": reply, "reason": "other", "note": "x".repeat(1001)}),
        )
        .await,
        400,
        "validation",
    )
    .await;

    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM assistant_reports")
            .fetch_one(&ts.state.pool)
            .await
            .expect("count"),
        0,
        "nothing refused may have been stored"
    );
}

/// Retention takes the message; the report still means something. `message_id`
/// is ON DELETE SET NULL and the excerpt is what outlives it — the same
/// arrangement a member report makes.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_swept_reply_leaves_the_report_standing() {
    let ts = server_with_assistant().await;
    let (member, _) = ts.register("nora", "Nora").await;
    ts.create_family(&member, "The Harpers").await;
    let ai = chat_id(&ts, &member, "ai").await;
    let reply = plant_reply(&ts, ai, "Swept later.").await;
    ts.post(
        &member,
        "/reports/assistant",
        json!({"message_id": reply, "reason": "other"}),
    )
    .await;

    sqlx::query("DELETE FROM messages WHERE id = $1")
        .bind(reply)
        .execute(&ts.state.pool)
        .await
        .expect("the sweep must go on working");

    let (kept_id, excerpt) = sqlx::query_as::<_, (Option<i64>, String)>(
        "SELECT message_id, message_excerpt FROM assistant_reports",
    )
    .fetch_one(&ts.state.pool)
    .await
    .expect("the row survives");
    assert_eq!(kept_id, None, "the pointer goes");
    assert_eq!(excerpt, "Swept later.", "the evidence stays");
}
