//! Member mentions (docs/protocol.md, "Mentioning a member").
//!
//! What is pinned: that a mention list rides the message on every read and
//! is decided once; every `validation` the section promises, and the two
//! refusals it deliberately does NOT make (blocks); the `mentioned` mark on
//! the chat list — a filter over the unread rows, cleared by reading, never
//! lit by a blocked member; a dedup re-ack that still carries the list; and
//! the list dying with its message.

mod common;

use common::{TestServer, assert_error, spawn_server, spawn_server_with_config};
use serde_json::{Value, json};
use uuid::Uuid;

/// Owner "Olive", member "Junior", a third member "Gran"; the family chat.
async fn family_of_three(ts: &TestServer) -> (String, String, String, i64, i64, i64, i64) {
    let (owner, owner_id) = ts.register("owner", "Olive").await;
    let (member, member_id) = ts.register("junior", "Junior").await;
    let (gran, gran_id) = ts.register("gran", "Gran").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&member, &invite_code, "joined").await;
    ts.join(&gran, &invite_code, "joined").await;
    let chat_id = ts.family_chat_id(&owner).await;
    (owner, member, gran, owner_id, member_id, gran_id, chat_id)
}

async fn post(
    ts: &TestServer,
    token: &str,
    chat_id: i64,
    body: &str,
    mentions: Value,
) -> reqwest::Response {
    ts.post(
        token,
        &format!("/chats/{chat_id}/messages"),
        json!({
            "client_msg_id": Uuid::new_v4().to_string(),
            "body": body,
            "mentions": mentions,
        }),
    )
    .await
}

async fn sent(ts: &TestServer, token: &str, chat_id: i64, body: &str, mentions: Value) -> Value {
    let response = post(ts, token, chat_id, body, mentions).await;
    assert_eq!(
        response.status(),
        201,
        "{}",
        response.text().await.unwrap_or_default()
    );
    let value: Value = response.json().await.expect("JSON");
    value["message"].clone()
}

async fn page(ts: &TestServer, token: &str, chat_id: i64) -> Vec<Value> {
    let response = ts.get(token, &format!("/chats/{chat_id}/messages")).await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.expect("JSON");
    body["messages"].as_array().expect("messages").clone()
}

fn find(messages: &[Value], id: i64) -> &Value {
    messages
        .iter()
        .find(|m| m["id"] == id)
        .expect("the message on the page")
}

async fn chat_entry(ts: &TestServer, token: &str, chat_id: i64) -> Value {
    let response = ts.get(token, "/chats").await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.expect("JSON");
    body["chats"]
        .as_array()
        .expect("chats")
        .iter()
        .find(|entry| entry["chat"]["id"] == chat_id)
        .cloned()
        .expect("the chat on the list")
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_mention_rides_the_message_on_every_read_and_is_decided_once() {
    let ts = spawn_server().await;
    let (owner, member, _gran, _, member_id, gran_id, chat_id) = family_of_three(&ts).await;
    let named =
        json!([{"user_id": member_id, "name": "Junior"}, {"user_id": gran_id, "name": "Gran"}]);
    let message = sent(
        &ts,
        &owner,
        chat_id,
        "@Junior and @Gran: dinner at 7?",
        named.clone(),
    )
    .await;
    let id = message["id"].as_i64().expect("id");
    // The send response, in the sender's order.
    assert_eq!(message["mentions"], named, "{message}");
    // A history page.
    assert_eq!(
        find(&page(&ts, &member, chat_id).await, id)["mentions"],
        named
    );
    // The thread read.
    let thread: Value = ts
        .get(&member, &format!("/chats/{chat_id}/messages/{id}/thread"))
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(thread["messages"][0]["mentions"], named, "{thread}");
    // An edit that drops the name changes the body alone: the list is
    // decided once, and the edits catch-up carries it unchanged.
    let edited = ts
        .patch(
            &owner,
            &format!("/chats/{chat_id}/messages/{id}"),
            json!({"body": "dinner at 7?"}),
        )
        .await;
    assert_eq!(edited.status(), 200);
    let edited: Value = edited.json().await.expect("JSON");
    assert_eq!(edited["message"]["mentions"], named, "{edited}");
    let edits: Value = ts
        .get(&member, &format!("/chats/{chat_id}/edits"))
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(edits["messages"][0]["mentions"], named, "{edits}");
    // And a message that names nobody carries no key at all.
    let plain = sent(&ts, &owner, chat_id, "anyone?", json!([])).await;
    assert!(plain.get("mentions").is_none(), "absent, never []: {plain}");
    let unsent = ts
        .post(
            &owner,
            &format!("/chats/{chat_id}/messages"),
            json!({"client_msg_id": Uuid::new_v4().to_string(), "body": "no key either"}),
        )
        .await;
    let unsent: Value = unsent.json().await.expect("JSON");
    assert!(unsent["message"].get("mentions").is_none(), "{unsent}");
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn every_refusal_the_section_promises_is_validation() {
    let ts = spawn_server().await;
    let (owner, member, _gran, owner_id, member_id, gran_id, chat_id) = family_of_three(&ts).await;
    let junior = |name: &str| json!([{"user_id": member_id, "name": name}]);

    // Not the whole name, not at a boundary, not the sender's case.
    for body in [
        "@Juniors here?",
        "@Junio here?",
        "mail@Junior here?",
        "Junior here?",
        "@junior here?",
    ] {
        assert_error(
            post(&ts, &owner, chat_id, body, junior("Junior")).await,
            400,
            "validation",
        )
        .await;
    }
    // The name as typed, empty or absurd.
    assert_error(
        post(&ts, &owner, chat_id, "@ here?", junior("")).await,
        400,
        "validation",
    )
    .await;
    assert_error(
        post(
            &ts,
            &owner,
            chat_id,
            &format!("@{} here?", "x".repeat(65)),
            junior(&"x".repeat(65)),
        )
        .await,
        400,
        "validation",
    )
    .await;
    // Twice, and too many.
    assert_error(
        post(&ts, &owner, chat_id, "@Junior @Junior", json!([{"user_id": member_id, "name": "Junior"}, {"user_id": member_id, "name": "Junior"}])).await,
        400,
        "validation",
    )
    .await;
    let too_many: Vec<Value> = (0..21)
        .map(|i| json!({"user_id": member_id + 1000 + i, "name": "Junior"}))
        .collect();
    assert_error(
        post(&ts, &owner, chat_id, "@Junior", json!(too_many)).await,
        400,
        "validation",
    )
    .await;
    // Not a member of this family: a stranger, the assistant, somebody who
    // left, a deleted account — one predicate.
    let (stranger, stranger_id) = ts.register("stranger", "Sam").await;
    ts.create_family(&stranger, "The Joneses").await;
    assert_error(
        post(
            &ts,
            &owner,
            chat_id,
            "@Sam here?",
            json!([{"user_id": stranger_id, "name": "Sam"}]),
        )
        .await,
        400,
        "validation",
    )
    .await;
    let assistant_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE username = 'assistant'")
        .fetch_one(&ts.state.pool)
        .await
        .expect("the assistant's row");
    assert_error(
        post(
            &ts,
            &owner,
            chat_id,
            "@ai here?",
            json!([{"user_id": assistant_id, "name": "ai"}]),
        )
        .await,
        400,
        "validation",
    )
    .await;
    // Outside the family chat.
    let direct: Value = ts
        .post(&owner, "/chats/direct", json!({"user_id": member_id}))
        .await
        .json()
        .await
        .expect("JSON");
    let direct_id = direct["chat"]["id"].as_i64().expect("direct chat id");
    assert_error(
        post(&ts, &owner, direct_id, "@Junior here?", junior("Junior")).await,
        400,
        "validation",
    )
    .await;
    // Fine: the sender naming themself; a comma after the name; two names.
    let me = sent(
        &ts,
        &owner,
        chat_id,
        "@Olive is cooking",
        json!([{"user_id": owner_id, "name": "Olive"}]),
    )
    .await;
    assert_eq!(me["mentions"][0]["user_id"], owner_id);
    sent(
        &ts,
        &owner,
        chat_id,
        "@Junior, @Gran: 7?",
        json!([{"user_id": member_id, "name": "Junior"}, {"user_id": gran_id, "name": "Gran"}]),
    )
    .await;
    // A member who left is refused from then on.
    let left = ts.post(&member, "/families/leave", json!({})).await;
    assert!(left.status().is_success());
    assert_error(
        post(&ts, &owner, chat_id, "@Junior?", junior("Junior")).await,
        400,
        "validation",
    )
    .await;
}

/// The name has to be in the body that will be STORED, not in the one the
/// request happened to carry.
///
/// `validate_body` trims, so a name whose last character is the body's own
/// trailing whitespace matches the raw string and not the stored one: the
/// end of a body is a boundary, and `"@ "` ends in the name `" "` while the
/// stored `"@"` carries nothing. Accepting it would push "mentioned you"
/// and set the "@" mark for a token no reader can find.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_name_only_the_untrimmed_body_carries_is_refused() {
    let ts = spawn_server().await;
    let (owner, _member, _, _, member_id, _, chat_id) = family_of_three(&ts).await;
    let junior = |name: &str| json!([{"user_id": member_id, "name": name}]);

    // The name is the trailing space itself.
    assert_error(
        post(
            &ts,
            &owner,
            chat_id,
            "@ ",
            json!([{"user_id": member_id, "name": " "}]),
        )
        .await,
        400,
        "validation",
    )
    .await;
    // And the name that only reaches its boundary because of one.
    assert_error(
        post(
            &ts,
            &owner,
            chat_id,
            "hey @Junior ",
            json!([{"user_id": member_id, "name": "Junior "}]),
        )
        .await,
        400,
        "validation",
    )
    .await;
    // The same body without the trailing space is the ordinary mention,
    // and what the trimmed body carries is what the message keeps.
    let message = sent(&ts, &owner, chat_id, "hey @Junior ", junior("Junior")).await;
    assert_eq!(message["body"], "hey @Junior");
    assert_eq!(message["mentions"][0]["name"], "Junior");
}

/// Two refusals the section states and no test used to send: a DELETED
/// account and the assistant's own thread.
///
/// Both ride predicates the other cases share — `u.family_id` is nulled by
/// the scrub, and the chat kind is not `family` — so a change that keeps
/// either predicate working for its tested case and breaks it here would
/// otherwise ship green.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_deleted_account_and_the_assistant_thread_are_both_validation() {
    let ts = spawn_server_with_config(|cfg| {
        cfg.ai.enabled = true;
        cfg.ai.endpoint = "https://example.invalid".to_string();
        cfg.ai.deployment = "test-deployment".to_string();
        cfg.ai.api_key = "test-key".to_string();
        cfg.ai.title = "Assistant".to_string();
    })
    .await;
    let (owner, owner_id) = ts.register("owner", "Olive").await;
    let (member, member_id) = ts.register("junior", "Junior").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&member, &invite_code, "joined").await;
    let chat_id = ts.family_chat_id(&owner).await;
    let junior = |name: &str| json!([{"user_id": member_id, "name": name}]);

    // Inside the owner's PRIVATE assistant thread, naming a real member of
    // the family: the chat is the assistant's, so `validation`.
    let ai_chat = ts
        .get(&owner, "/chats")
        .await
        .json::<Value>()
        .await
        .expect("JSON")["chats"]
        .as_array()
        .expect("chats")
        .iter()
        .find(|entry| entry["chat"]["kind"] == "ai")
        .and_then(|entry| entry["chat"]["id"].as_i64())
        .expect("the assistant chat exists");
    assert_error(
        post(&ts, &owner, ai_chat, "@Junior here?", junior("Junior")).await,
        400,
        "validation",
    )
    .await;

    // Named while a member: accepted. Then the account is scrubbed.
    sent(&ts, &owner, chat_id, "@Junior dinner?", junior("Junior")).await;
    let deleted = ts
        .post(&member, "/me/delete", json!({"password": "password123"}))
        .await;
    assert_eq!(deleted.status(), 204, "deleting the account");
    assert_error(
        post(&ts, &owner, chat_id, "@Junior?", junior("Junior")).await,
        400,
        "validation",
    )
    .await;
    // The tombstone's own name is no better an answer.
    assert_error(
        post(
            &ts,
            &owner,
            chat_id,
            "@Deleted account?",
            json!([{"user_id": member_id, "name": "Deleted account"}]),
        )
        .await,
        400,
        "validation",
    )
    .await;
    let _ = owner_id;
}

/// Nothing about blocks is checked, deliberately: a refusal keyed on who
/// blocked the sender would tell them. The mention is accepted and then
/// does nothing for the blocker — no push, no mark (the push half is
/// pinned in push_flow.rs).
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn mentioning_somebody_who_blocked_you_is_accepted_and_marks_nothing() {
    let ts = spawn_server().await;
    let (owner, member, gran, _, member_id, gran_id, chat_id) = family_of_three(&ts).await;
    // Junior blocks Olive.
    let blocked = ts
        .put(
            &member,
            &format!("/families/members/{}/block", ts.user_id(&owner).await),
            json!({}),
        )
        .await;
    assert!(blocked.status().is_success(), "{}", blocked.status());

    let message = sent(
        &ts,
        &owner,
        chat_id,
        "@Junior @Gran dinner?",
        json!([{"user_id": member_id, "name": "Junior"}, {"user_id": gran_id, "name": "Gran"}]),
    )
    .await;
    assert_eq!(
        message["mentions"].as_array().map(Vec::len),
        Some(2),
        "accepted whole: {message}"
    );

    // Gran is marked; Junior — the blocker — is not, though unread moved.
    let grans = chat_entry(&ts, &gran, chat_id).await;
    assert_eq!(grans["mentioned"], true, "{grans}");
    let juniors = chat_entry(&ts, &member, chat_id).await;
    assert!(
        juniors.get("mentioned").is_none(),
        "a hidden row lights nothing: {juniors}"
    );
    assert_eq!(
        juniors["unread_count"], 1,
        "but the count is the count: {juniors}"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_mark_on_the_chat_row_is_a_filter_over_the_unread_rows() {
    let ts = spawn_server().await;
    let (owner, member, gran, _, member_id, _gran_id, chat_id) = family_of_three(&ts).await;
    // Nothing yet.
    assert!(
        chat_entry(&ts, &member, chat_id)
            .await
            .get("mentioned")
            .is_none()
    );

    let message = sent(
        &ts,
        &owner,
        chat_id,
        "@Junior dinner?",
        json!([{"user_id": member_id, "name": "Junior"}]),
    )
    .await;
    let id = message["id"].as_i64().expect("id");
    // Named: marked. The sender and a member not named: not.
    assert_eq!(chat_entry(&ts, &member, chat_id).await["mentioned"], true);
    assert!(
        chat_entry(&ts, &owner, chat_id)
            .await
            .get("mentioned")
            .is_none(),
        "the sender is never marked by their own message"
    );
    assert!(
        chat_entry(&ts, &gran, chat_id)
            .await
            .get("mentioned")
            .is_none()
    );

    // Reading clears it, with the count — the same threshold.
    let read = ts
        .post(
            &member,
            &format!("/chats/{chat_id}/read"),
            json!({"last_read_message_id": id}),
        )
        .await;
    assert!(read.status().is_success());
    let entry = chat_entry(&ts, &member, chat_id).await;
    assert!(entry.get("mentioned").is_none(), "{entry}");
    assert_eq!(entry["unread_count"], 0);

    // A later, unnamed message leaves it off; a later mention lights it again.
    sent(&ts, &owner, chat_id, "anyone?", json!([])).await;
    assert!(
        chat_entry(&ts, &member, chat_id)
            .await
            .get("mentioned")
            .is_none()
    );
    sent(
        &ts,
        &owner,
        chat_id,
        "@Junior?",
        json!([{"user_id": member_id, "name": "Junior"}]),
    )
    .await;
    let entry = chat_entry(&ts, &member, chat_id).await;
    assert_eq!(entry["mentioned"], true, "{entry}");
    assert_eq!(
        entry["unread_count"], 2,
        "a chat with the mark always has a count: {entry}"
    );
    // Never `false`: absent is the only other answer.
    assert_ne!(
        chat_entry(&ts, &gran, chat_id).await.get("mentioned"),
        Some(&json!(false))
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_dedup_re_ack_still_carries_the_list_and_the_list_dies_with_the_message() {
    let ts = spawn_server().await;
    let (owner, _member, _gran, _, member_id, _gran_id, chat_id) = family_of_three(&ts).await;
    let client_msg_id = Uuid::new_v4().to_string();
    let request = json!({
        "client_msg_id": client_msg_id,
        "body": "@Junior dinner?",
        "mentions": [{"user_id": member_id, "name": "Junior"}],
    });
    let first = ts
        .post(
            &owner,
            &format!("/chats/{chat_id}/messages"),
            request.clone(),
        )
        .await;
    assert_eq!(first.status(), 201);
    let first: Value = first.json().await.expect("JSON");
    let id = first["message"]["id"].as_i64().expect("id");
    // The retry: 200, the original, list and all.
    let again = ts
        .post(&owner, &format!("/chats/{chat_id}/messages"), request)
        .await;
    assert_eq!(again.status(), 200);
    let again: Value = again.json().await.expect("JSON");
    assert_eq!(again["message"]["id"], id);
    assert_eq!(
        again["message"]["mentions"], first["message"]["mentions"],
        "{again}"
    );

    // Retention takes the list with the message.
    let days = ts.state.cfg.limits.retention_days;
    sqlx::query("UPDATE messages SET created_at = now() - make_interval(days => $2) WHERE id = $1")
        .bind(id)
        .bind((days + 1) as i32)
        .execute(&ts.state.pool)
        .await
        .expect("aging");
    let swept = family_connect::handlers_chat::sweep_expired_messages(&ts.state)
        .await
        .expect("sweep");
    assert_eq!(swept, 1);
    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM message_mentions WHERE message_id = $1")
            .bind(id)
            .fetch_one(&ts.state.pool)
            .await
            .expect("count");
    assert_eq!(rows, 0);
}
