//! Integration: the retention sweep (protocol.md, "Retention"). Messages
//! past the configured age go, and the attachment files they owned go with
//! them — unless somebody else's message still points at the same bytes.

mod common;

use common::{TestServer, spawn_server, spawn_server_with_config};
use serde_json::{Value, json};
use uuid::Uuid;

fn jpeg_bytes(len: usize) -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE0];
    bytes.resize(len.max(4), 0x00);
    bytes
}

async fn family_of_two(ts: &TestServer) -> (String, String, i64) {
    let (owner, _) = ts.register("owner", "Olive").await;
    let (member, _) = ts.register("junior", "Junior").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&member, &invite_code, "joined").await;
    let chat_id = ts.family_chat_id(&owner).await;
    (owner, member, chat_id)
}

async fn send(ts: &TestServer, token: &str, chat_id: i64, body: &str) -> i64 {
    let response = ts
        .post(
            token,
            &format!("/chats/{chat_id}/messages"),
            json!({"client_msg_id": Uuid::new_v4().to_string(), "body": body}),
        )
        .await;
    assert_eq!(response.status(), 201);
    let value: Value = response.json().await.expect("JSON");
    value["message"]["id"].as_i64().expect("id")
}

/// Age a message by rewriting its timestamp — the sweep reads `created_at`,
/// so this is the whole of what "old" means to it.
async fn age(server: &TestServer, message_id: i64, days: i64) {
    sqlx::query("UPDATE messages SET created_at = now() - make_interval(days => $2) WHERE id = $1")
        .bind(message_id)
        .bind(days as i32)
        .execute(&server.state.pool)
        .await
        .expect("aging the message");
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn messages_past_the_retention_age_are_deleted() {
    let server = spawn_server().await;
    let (owner, _, chat_id) = family_of_two(&server).await;
    let days = server.state.cfg.limits.retention_days;

    let old = send(&server, &owner, chat_id, "from the spring").await;
    let recent = send(&server, &owner, chat_id, "from this morning").await;
    age(&server, old, days + 1).await;
    // Right on the boundary stays: "older than 100 days", not "100 days".
    age(&server, recent, days - 1).await;

    let swept = family_connect::handlers_chat::sweep_expired_messages(&server.state)
        .await
        .expect("sweep");
    assert_eq!(swept, 1);

    let page: Value = server
        .get(&owner, &format!("/chats/{chat_id}/messages"))
        .await
        .json()
        .await
        .expect("JSON");
    let bodies: Vec<&str> = page["messages"]
        .as_array()
        .expect("array")
        .iter()
        .map(|m| m["body"].as_str().unwrap_or_default())
        .collect();
    assert!(bodies.contains(&"from this morning"), "{bodies:?}");
    assert!(!bodies.contains(&"from the spring"), "{bodies:?}");
}

/// A reply is a message in its own right. Deleting a recent one because it
/// quotes something old would be destroying current conversation to
/// enforce a policy about old conversation.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_reply_outlives_the_message_it_quoted() {
    let server = spawn_server().await;
    let (owner, member, chat_id) = family_of_two(&server).await;
    let days = server.state.cfg.limits.retention_days;

    let parent = send(&server, &owner, chat_id, "See you at six").await;
    let response = server
        .post(
            &member,
            &format!("/chats/{chat_id}/messages"),
            json!({
                "client_msg_id": Uuid::new_v4().to_string(),
                "body": "Six works",
                "reply_to_message_id": parent,
            }),
        )
        .await;
    assert_eq!(response.status(), 201);

    age(&server, parent, days + 1).await;

    // Without ON DELETE SET NULL (migration 0012) this is where the whole
    // sweep would fail on a foreign-key violation.
    let swept = family_connect::handlers_chat::sweep_expired_messages(&server.state)
        .await
        .expect("sweep");
    assert_eq!(swept, 1);

    let page: Value = server
        .get(&owner, &format!("/chats/{chat_id}/messages"))
        .await
        .json()
        .await
        .expect("JSON");
    let messages = page["messages"].as_array().expect("array");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["body"], "Six works");
    // The reply survives and simply has no quote any more.
    assert!(messages[0].get("reply_to").is_none(), "{:?}", messages[0]);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_expired_message_takes_its_attachment_file_with_it() {
    let server = spawn_server().await;
    let (owner, _, chat_id) = family_of_two(&server).await;
    let days = server.state.cfg.limits.retention_days;

    let uploaded: Value = server
        .put_bytes_method(
            "POST",
            &owner,
            "/attachments?kind=photo",
            "image/jpeg",
            jpeg_bytes(512),
        )
        .await
        .json()
        .await
        .expect("JSON");
    let attachment_id = uploaded["attachment"]["id"].as_i64().expect("id");
    let key: String = sqlx::query_scalar("SELECT storage_key FROM attachments WHERE id = $1")
        .bind(attachment_id)
        .fetch_one(&server.state.pool)
        .await
        .expect("key");
    let path = server.state.storage.blob_path(&key);

    let response = server
        .post(
            &owner,
            &format!("/chats/{chat_id}/messages"),
            json!({
                "client_msg_id": Uuid::new_v4().to_string(),
                "body": "",
                "attachment_id": attachment_id,
            }),
        )
        .await;
    assert_eq!(response.status(), 201);
    let message_id = response.json::<Value>().await.expect("JSON")["message"]["id"]
        .as_i64()
        .expect("id");
    assert!(path.exists());

    age(&server, message_id, days + 1).await;
    family_connect::handlers_chat::sweep_expired_messages(&server.state)
        .await
        .expect("sweep");

    // Row and bytes both gone — the whole point is reclaiming the disk.
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM attachments WHERE id = $1")
        .bind(attachment_id)
        .fetch_one(&server.state.pool)
        .await
        .expect("count");
    assert_eq!(rows, 0);
    assert!(!path.exists(), "the file outlived its only message");
}

/// Dedup means one file can serve several messages. Expiring one of them
/// must not delete bytes the others still need.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_shared_file_survives_until_its_last_message_expires() {
    let server = spawn_server().await;
    let (owner, _, chat_id) = family_of_two(&server).await;
    let days = server.state.cfg.limits.retention_days;
    let bytes = jpeg_bytes(512);

    let mut message_ids = Vec::new();
    for _ in 0..2 {
        let uploaded: Value = server
            .put_bytes_method(
                "POST",
                &owner,
                "/attachments?kind=photo",
                "image/jpeg",
                bytes.clone(),
            )
            .await
            .json()
            .await
            .expect("JSON");
        let attachment_id = uploaded["attachment"]["id"].as_i64().expect("id");
        let response = server
            .post(
                &owner,
                &format!("/chats/{chat_id}/messages"),
                json!({
                    "client_msg_id": Uuid::new_v4().to_string(),
                    "body": "",
                    "attachment_id": attachment_id,
                }),
            )
            .await;
        message_ids.push(
            response.json::<Value>().await.expect("JSON")["message"]["id"]
                .as_i64()
                .expect("id"),
        );
    }

    let key: String = sqlx::query_scalar("SELECT storage_key FROM attachments ORDER BY id LIMIT 1")
        .fetch_one(&server.state.pool)
        .await
        .expect("key");
    let path = server.state.storage.blob_path(&key);

    // Expire only the first.
    age(&server, message_ids[0], days + 1).await;
    family_connect::handlers_chat::sweep_expired_messages(&server.state)
        .await
        .expect("sweep");
    assert!(
        path.exists(),
        "the survivor's bytes were deleted with the other message"
    );
    // And the surviving message can still be read WITH its attachment —
    // the row is intact, not just the file.
    let page: Value = server
        .get(&owner, &format!("/chats/{chat_id}/messages"))
        .await
        .json()
        .await
        .expect("JSON");
    let remaining = page["messages"].as_array().expect("array");
    assert_eq!(remaining.len(), 1);
    let survivor = remaining[0]["attachment"]["id"]
        .as_i64()
        .expect("attachment");
    let fetched = server
        .get(&owner, &format!("/attachments/{survivor}"))
        .await;
    assert_eq!(fetched.status(), 200);
    assert_eq!(fetched.bytes().await.unwrap().as_ref(), bytes.as_slice());

    // Now the other, and the file goes.
    age(&server, message_ids[1], days + 1).await;
    family_connect::handlers_chat::sweep_expired_messages(&server.state)
        .await
        .expect("sweep");
    assert!(!path.exists(), "the shared file outlived its last message");
}

/// A message may carry a whole album, and the sweep must reclaim EVERY
/// file it owned — except bytes another, newer message still shares.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_expired_album_takes_all_its_unshared_files_with_it() {
    let server = spawn_server().await;
    let (owner, _, chat_id) = family_of_two(&server).await;
    let days = server.state.cfg.limits.retention_days;

    // Three photos with distinct bytes: three files on disk.
    let mut album_ids = Vec::new();
    for len in [512usize, 640, 768] {
        let uploaded: Value = server
            .put_bytes_method(
                "POST",
                &owner,
                "/attachments?kind=photo",
                "image/jpeg",
                jpeg_bytes(len),
            )
            .await
            .json()
            .await
            .expect("JSON");
        album_ids.push(uploaded["attachment"]["id"].as_i64().expect("id"));
    }
    let response = server
        .post(
            &owner,
            &format!("/chats/{chat_id}/messages"),
            json!({
                "client_msg_id": Uuid::new_v4().to_string(),
                "body": "",
                "attachment_ids": album_ids,
            }),
        )
        .await;
    assert_eq!(response.status(), 201);
    let album_message = response.json::<Value>().await.expect("JSON")["message"]["id"]
        .as_i64()
        .expect("id");

    // A NEWER message shares the third photo's bytes (dedup, 0011): that
    // one file must survive the album's expiry.
    let shared_upload: Value = server
        .put_bytes_method(
            "POST",
            &owner,
            "/attachments?kind=photo",
            "image/jpeg",
            jpeg_bytes(768),
        )
        .await
        .json()
        .await
        .expect("JSON");
    let shared_id = shared_upload["attachment"]["id"].as_i64().expect("id");
    let response = server
        .post(
            &owner,
            &format!("/chats/{chat_id}/messages"),
            json!({
                "client_msg_id": Uuid::new_v4().to_string(),
                "body": "",
                "attachment_id": shared_id,
            }),
        )
        .await;
    assert_eq!(response.status(), 201);

    let mut paths = Vec::new();
    for id in &album_ids {
        let key: String = sqlx::query_scalar("SELECT storage_key FROM attachments WHERE id = $1")
            .bind(id)
            .fetch_one(&server.state.pool)
            .await
            .expect("key");
        paths.push(server.state.storage.blob_path(&key));
    }
    for path in &paths {
        assert!(path.exists());
    }

    age(&server, album_message, days + 1).await;
    let swept = family_connect::handlers_chat::sweep_expired_messages(&server.state)
        .await
        .expect("sweep");
    assert_eq!(swept, 1, "one MESSAGE went, however many files it owned");

    // All three rows went with the message…
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM attachments WHERE message_id = $1")
        .bind(album_message)
        .fetch_one(&server.state.pool)
        .await
        .expect("count");
    assert_eq!(rows, 0);
    // …and so did every file nothing else shares. The third survives for
    // the newer message that points at the same bytes.
    assert!(!paths[0].exists(), "the first photo's file leaked");
    assert!(!paths[1].exists(), "the second photo's file leaked");
    assert!(
        paths[2].exists(),
        "the shared file went down with the wrong message"
    );
}

/// 0 means keep everything — a state, not a very large number.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn retention_zero_disables_the_sweep() {
    let server = spawn_server_with_config(|cfg| cfg.limits.retention_days = 0).await;
    let (owner, _, chat_id) = family_of_two(&server).await;

    let ancient = send(&server, &owner, chat_id, "from years ago").await;
    age(&server, ancient, 5_000).await;

    let swept = family_connect::handlers_chat::sweep_expired_messages(&server.state)
        .await
        .expect("sweep");
    assert_eq!(swept, 0);

    let page: Value = server
        .get(&owner, &format!("/chats/{chat_id}/messages"))
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(page["messages"].as_array().expect("array").len(), 1);
}
