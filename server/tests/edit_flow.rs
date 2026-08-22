//! Integration: editing a message — authorship, the no-op rule, what an
//! edit leaves untouched, and the seq cursor that makes catch-up possible
//! (protocol.md, "Editing").

mod common;

use common::{TestServer, assert_error, spawn_server};
use serde_json::{Value, json};
use uuid::Uuid;

/// Family of two with one message from the owner; returns
/// `(owner_token, member_token, chat_id, message_id)`.
async fn family_with_a_message(ts: &TestServer) -> (String, String, i64, i64) {
    let (owner, _) = ts.register("owner", "Olive").await;
    let (member, _) = ts.register("junior", "Junior").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&member, &invite_code, "joined").await;
    let chat_id = ts.family_chat_id(&owner).await;
    let response = ts
        .post_message(&owner, chat_id, &Uuid::new_v4().to_string(), "Dinner at 7?")
        .await;
    let body: Value = response.json().await.expect("JSON");
    let message_id = body["message"]["id"].as_i64().expect("id");
    (owner, member, chat_id, message_id)
}

async fn edit(
    ts: &TestServer,
    token: &str,
    chat_id: i64,
    message_id: i64,
    body: &str,
) -> reqwest::Response {
    ts.patch(
        token,
        &format!("/chats/{chat_id}/messages/{message_id}"),
        json!({"body": body}),
    )
    .await
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_author_can_edit_and_the_message_says_so() {
    let server = spawn_server().await;
    let (owner, _, chat_id, message_id) = family_with_a_message(&server).await;

    // Not edited yet: both stamps absent, not null.
    let page: Value = server
        .get(&owner, &format!("/chats/{chat_id}/messages"))
        .await
        .json()
        .await
        .expect("JSON");
    let before = &page["messages"][0];
    assert!(before.get("edited_at").is_none(), "got {before}");
    assert!(before.get("edit_seq").is_none(), "got {before}");

    let response = edit(&server, &owner, chat_id, message_id, "Dinner at 8?").await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.expect("JSON");
    assert_eq!(body["message"]["body"], "Dinner at 8?");
    assert_eq!(body["message"]["id"].as_i64(), Some(message_id));
    assert!(body["message"]["edited_at"].is_string());
    let seq = body["message"]["edit_seq"].as_i64().expect("edit_seq");
    assert!(seq > 0);

    // And every reader sees the new text.
    let page: Value = server
        .get(&owner, &format!("/chats/{chat_id}/messages"))
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(page["messages"][0]["body"], "Dinner at 8?");
    assert_eq!(page["messages"][0]["edit_seq"].as_i64(), Some(seq));
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn only_the_author_may_edit() {
    let server = spawn_server().await;
    let (_, member, chat_id, message_id) = family_with_a_message(&server).await;

    assert_error(
        edit(&server, &member, chat_id, message_id, "hijacked").await,
        403,
        "not_message_author",
    )
    .await;
}

/// Re-sending the body it already has burns no sequence value — otherwise a
/// client that retries an edit would advance every other client's cursor and
/// make them re-fetch for nothing.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn re_sending_the_same_body_is_a_no_op() {
    let server = spawn_server().await;
    let (owner, _, chat_id, message_id) = family_with_a_message(&server).await;

    let first: Value = edit(&server, &owner, chat_id, message_id, "Dinner at 8?")
        .await
        .json()
        .await
        .expect("JSON");
    let seq = first["message"]["edit_seq"].as_i64().expect("seq");

    let again: Value = edit(&server, &owner, chat_id, message_id, "Dinner at 8?")
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(
        again["message"]["edit_seq"].as_i64(),
        Some(seq),
        "an unchanged body must not take a new sequence value"
    );
}

/// An edit changes the body and nothing else. Reactions in particular
/// survive: they belong to the message, not to its text.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_edit_leaves_everything_but_the_body_alone() {
    let server = spawn_server().await;
    let (owner, member, chat_id, message_id) = family_with_a_message(&server).await;

    server
        .put(
            &member,
            &format!("/chats/{chat_id}/messages/{message_id}/reaction"),
            json!({"emoji": "❤️"}),
        )
        .await;

    let before: Value = server
        .get(&owner, &format!("/chats/{chat_id}/messages"))
        .await
        .json()
        .await
        .expect("JSON");
    let created_at = before["messages"][0]["created_at"].clone();
    let sender_id = before["messages"][0]["sender_id"].clone();

    let after: Value = edit(&server, &owner, chat_id, message_id, "Dinner at 8?")
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(after["message"]["created_at"], created_at);
    assert_eq!(after["message"]["sender_id"], sender_id);
    assert_eq!(after["message"]["reactions"][0]["emoji"], "❤️");
}

/// The whole point of the second cursor: `after_id` is `WHERE id > cursor`
/// and can never see a change to an OLDER row, so a client that was away
/// learns about it here or not at all.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_edit_cursor_replays_what_after_id_cannot_see() {
    let server = spawn_server().await;
    let (owner, member, chat_id, first_id) = family_with_a_message(&server).await;

    // A newer message, so the edited one is no longer the latest.
    let newer: Value = server
        .post_message(&owner, chat_id, &Uuid::new_v4().to_string(), "Anyone home?")
        .await
        .json()
        .await
        .expect("JSON");
    let newer_id = newer["message"]["id"].as_i64().expect("id");

    let edited: Value = edit(&server, &owner, chat_id, first_id, "Dinner at 8?")
        .await
        .json()
        .await
        .expect("JSON");
    let seq = edited["message"]["edit_seq"].as_i64().expect("seq");

    // after_id from the newest message: the edit is invisible, by design.
    let catch_up: Value = server
        .get(
            &member,
            &format!("/chats/{chat_id}/messages?after_id={newer_id}"),
        )
        .await
        .json()
        .await
        .expect("JSON");
    assert!(
        catch_up["messages"].as_array().expect("array").is_empty(),
        "after_id can only see NEWER rows"
    );

    // The edit cursor sees it.
    let edits: Value = server
        .get(&member, &format!("/chats/{chat_id}/edits?after_seq=0"))
        .await
        .json()
        .await
        .expect("JSON");
    let messages = edits["messages"].as_array().expect("array");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["id"].as_i64(), Some(first_id));
    assert_eq!(messages[0]["body"], "Dinner at 8?");

    // And a client already at that seq gets nothing more.
    let empty: Value = server
        .get(&member, &format!("/chats/{chat_id}/edits?after_seq={seq}"))
        .await
        .json()
        .await
        .expect("JSON");
    assert!(empty["messages"].as_array().expect("array").is_empty());
}

/// `max_edit_seq` is how a client knows there is anything to catch up on at
/// all — omitted entirely until something has been edited.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_chat_list_reports_the_edit_cursor() {
    let server = spawn_server().await;
    let (owner, _, chat_id, message_id) = family_with_a_message(&server).await;

    let before: Value = server
        .get(&owner, "/chats")
        .await
        .json()
        .await
        .expect("JSON");
    let entry = before["chats"]
        .as_array()
        .expect("array")
        .iter()
        .find(|c| c["chat"]["id"].as_i64() == Some(chat_id))
        .expect("the family chat");
    assert!(entry.get("max_edit_seq").is_none(), "got {entry}");

    let edited: Value = edit(&server, &owner, chat_id, message_id, "Dinner at 8?")
        .await
        .json()
        .await
        .expect("JSON");
    let seq = edited["message"]["edit_seq"].as_i64().expect("seq");

    let after: Value = server
        .get(&owner, "/chats")
        .await
        .json()
        .await
        .expect("JSON");
    let entry = after["chats"]
        .as_array()
        .expect("array")
        .iter()
        .find(|c| c["chat"]["id"].as_i64() == Some(chat_id))
        .expect("the family chat");
    assert_eq!(entry["max_edit_seq"].as_i64(), Some(seq));
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn bad_edits_are_refused() {
    let server = spawn_server().await;
    let (owner, _, chat_id, message_id) = family_with_a_message(&server).await;

    assert_error(
        edit(&server, &owner, chat_id, message_id, "   ").await,
        400,
        "message_empty",
    )
    .await;
    assert_error(
        edit(&server, &owner, chat_id, message_id, &"x".repeat(4001)).await,
        400,
        "message_too_long",
    )
    .await;
    // A message in another chat is indistinguishable from one that does
    // not exist — the same rule replies follow.
    assert_error(
        edit(&server, &owner, chat_id, 999_999, "nope").await,
        404,
        "message_not_found",
    )
    .await;
}

/// A quote is a snapshot taken at READ time, so editing a quoted message
/// changes what later readers see quoted — the property that let replies
/// ship without a stored excerpt.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn editing_a_quoted_message_updates_the_quote() {
    let server = spawn_server().await;
    let (owner, member, chat_id, quoted_id) = family_with_a_message(&server).await;

    server
        .post(
            &member,
            &format!("/chats/{chat_id}/messages"),
            json!({
                "client_msg_id": Uuid::new_v4().to_string(),
                "body": "Works for me",
                "reply_to_message_id": quoted_id,
            }),
        )
        .await;

    edit(&server, &owner, chat_id, quoted_id, "Dinner at 8?").await;

    let page: Value = server
        .get(&member, &format!("/chats/{chat_id}/messages"))
        .await
        .json()
        .await
        .expect("JSON");
    let reply = page["messages"]
        .as_array()
        .expect("array")
        .iter()
        .find(|m| m["body"] == "Works for me")
        .expect("the reply");
    assert_eq!(reply["reply_to"]["excerpt"], "Dinner at 8?");
}
