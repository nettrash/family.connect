//! Integration: replies — the quote is resolved at send time, recomputed on
//! every read, scoped to the chat it was sent in, and carried by the ordinary
//! message shape (protocol.md, "Replies").

mod common;

use common::{TestServer, assert_error, spawn_server};
use serde_json::{Value, json};
use uuid::Uuid;

/// Family of two with one message from the owner; returns
/// `(owner_token, owner_id, member_token, member_id, chat_id, message_id)`.
async fn family_with_a_message(ts: &TestServer) -> (String, i64, String, i64, i64, i64) {
    let (owner, owner_id) = ts.register("owner", "Olive").await;
    let (member, member_id) = ts.register("junior", "Junior").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&member, &invite_code, "joined").await;
    let chat_id = ts.family_chat_id(&owner).await;
    let response = ts
        .post_message(
            &owner,
            chat_id,
            &Uuid::new_v4().to_string(),
            "See you at six",
        )
        .await;
    assert_eq!(response.status(), 201);
    let body: Value = response.json().await.expect("message response is JSON");
    let message_id = body["message"]["id"].as_i64().expect("message id");
    (owner, owner_id, member, member_id, chat_id, message_id)
}

async fn post_reply(
    ts: &TestServer,
    token: &str,
    chat_id: i64,
    body: &str,
    reply_to: i64,
) -> reqwest::Response {
    ts.post(
        token,
        &format!("/chats/{chat_id}/messages"),
        json!({
            "client_msg_id": Uuid::new_v4().to_string(),
            "body": body,
            "reply_to_message_id": reply_to,
        }),
    )
    .await
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_reply_carries_the_quoted_author_and_excerpt() {
    let server = spawn_server().await;
    let (owner, owner_id, member, _, chat_id, quoted_id) = family_with_a_message(&server).await;

    let response = post_reply(&server, &member, chat_id, "Six works", quoted_id).await;
    assert_eq!(response.status(), 201);
    let body: Value = response.json().await.expect("JSON");
    let reply_to = &body["message"]["reply_to"];
    assert_eq!(reply_to["message_id"].as_i64(), Some(quoted_id));
    assert_eq!(reply_to["sender_id"].as_i64(), Some(owner_id));
    assert_eq!(reply_to["excerpt"], "See you at six");

    // And it survives a read, for a different member than the sender.
    let page: Value = server
        .get(&owner, &format!("/chats/{chat_id}/messages"))
        .await
        .json()
        .await
        .expect("JSON");
    let messages = page["messages"].as_array().expect("array");
    let fetched = messages
        .iter()
        .find(|m| m["body"] == "Six works")
        .expect("the reply is in the page");
    assert_eq!(fetched["reply_to"]["message_id"].as_i64(), Some(quoted_id));
    assert_eq!(fetched["reply_to"]["excerpt"], "See you at six");
}

/// The field is absent, never null: a client distinguishes "not a reply"
/// by the key not being there at all.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_ordinary_message_omits_the_quote_entirely() {
    let server = spawn_server().await;
    let (owner, _, _, _, chat_id, _) = family_with_a_message(&server).await;

    let response = server
        .post_message(&owner, chat_id, &Uuid::new_v4().to_string(), "Just talking")
        .await;
    let body: Value = response.json().await.expect("JSON");
    assert!(
        body["message"].get("reply_to").is_none(),
        "reply_to must be absent on a message that is not a reply, got {}",
        body["message"]
    );
}

/// Scoped to the chat on purpose: a real message the caller cannot see must
/// be indistinguishable from one that does not exist, or the endpoint would
/// confirm ids from other people's conversations.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn quoting_a_message_from_another_chat_is_not_found() {
    let server = spawn_server().await;
    let (owner, _, _member, member_id, family_chat, family_message) =
        family_with_a_message(&server).await;

    // A direct chat between the two.
    let direct: Value = server
        .post(&owner, "/chats/direct", json!({"user_id": member_id}))
        .await
        .json()
        .await
        .expect("JSON");
    let direct_chat = direct["chat"]["id"].as_i64().expect("chat id");

    // Quoting the family-chat message from inside the direct chat: 404.
    assert_error(
        post_reply(&server, &owner, direct_chat, "Nope", family_message).await,
        404,
        "message_not_found",
    )
    .await;

    // A wholly nonexistent id answers identically.
    assert_error(
        post_reply(&server, &owner, family_chat, "Nope", 999_999).await,
        404,
        "message_not_found",
    )
    .await;
}

/// The quote is recomputed from the parent row on every read, never stored
/// beside the reply — so the day editing ships, a quote cannot go on showing
/// text its author has changed. Proven by editing the parent underneath.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_excerpt_follows_the_quoted_message() {
    let server = spawn_server().await;
    let (owner, _, member, _, chat_id, quoted_id) = family_with_a_message(&server).await;
    post_reply(&server, &member, chat_id, "Six works", quoted_id).await;

    // Stand in for a future edit endpoint.
    sqlx::query("UPDATE messages SET body = 'See you at seven' WHERE id = $1")
        .bind(quoted_id)
        .execute(&server.state.pool)
        .await
        .expect("edit the quoted message");

    let page: Value = server
        .get(&owner, &format!("/chats/{chat_id}/messages"))
        .await
        .json()
        .await
        .expect("JSON");
    let reply = page["messages"]
        .as_array()
        .expect("array")
        .iter()
        .find(|m| m["body"] == "Six works")
        .expect("the reply");
    assert_eq!(
        reply["reply_to"]["excerpt"], "See you at seven",
        "the excerpt must be recomputed, not frozen at send time"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_long_quote_is_cut_to_the_documented_length() {
    let server = spawn_server().await;
    let (owner, _, member, _, chat_id, _) = family_with_a_message(&server).await;

    let long_body = "é中😀".repeat(100);
    let posted: Value = server
        .post_message(&owner, chat_id, &Uuid::new_v4().to_string(), &long_body)
        .await
        .json()
        .await
        .expect("JSON");
    let long_id = posted["message"]["id"].as_i64().expect("id");

    let reply: Value = post_reply(&server, &member, chat_id, "Wow", long_id)
        .await
        .json()
        .await
        .expect("JSON");
    let excerpt = reply["message"]["reply_to"]["excerpt"]
        .as_str()
        .expect("excerpt");
    assert_eq!(excerpt.chars().count(), 120);
    // Multi-byte throughout: a byte-based cut would have produced invalid
    // UTF-8 and this response would not have parsed at all.
    assert!(excerpt.starts_with("é中😀"));
}

/// Replies are ordinary messages: they page, they catch up, they carry
/// reactions. Nothing about them needs a cursor of its own.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn replies_take_part_in_catch_up_paging() {
    let server = spawn_server().await;
    let (owner, _, member, _, chat_id, quoted_id) = family_with_a_message(&server).await;

    let reply: Value = post_reply(&server, &member, chat_id, "Six works", quoted_id)
        .await
        .json()
        .await
        .expect("JSON");
    let reply_id = reply["message"]["id"].as_i64().expect("id");

    let page: Value = server
        .get(
            &owner,
            &format!("/chats/{chat_id}/messages?after_id={quoted_id}"),
        )
        .await
        .json()
        .await
        .expect("JSON");
    let messages = page["messages"].as_array().expect("array");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["id"].as_i64(), Some(reply_id));
    assert_eq!(
        messages[0]["reply_to"]["message_id"].as_i64(),
        Some(quoted_id)
    );
}
