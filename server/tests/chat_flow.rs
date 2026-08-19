//! Integration: direct chats, message posting with dedup, keyset pagination,
//! and read markers / unread counts.

mod common;

use common::{TestServer, assert_error, spawn_server};
use serde_json::{Value, json};
use uuid::Uuid;

/// A family of two (open policy) — the standard fixture for chat tests.
/// Returns `(owner_token, owner_id, member_token, member_id)`.
async fn family_of_two(ts: &TestServer) -> (String, i64, String, i64) {
    let (owner, owner_id) = ts.register("owner", "Olive").await;
    let (member, member_id) = ts.register("junior", "Junior").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&member, &invite_code, "joined").await;
    (owner, owner_id, member, member_id)
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn direct_chats_are_get_or_create_and_symmetric() {
    let ts = spawn_server().await;
    let (owner, owner_id, member, member_id) = family_of_two(&ts).await;

    let first: Value = ts
        .post(&owner, "/chats/direct", json!({"user_id": member_id}))
        .await
        .json()
        .await
        .expect("chat body");
    assert_eq!(first["chat"]["kind"], "direct");
    assert_eq!(first["chat"]["title"], "Junior", "peer display name");
    assert_eq!(first["chat"]["peer_user_id"], member_id);
    let chat_id = first["chat"]["id"].as_i64().expect("chat id");

    // Idempotent from either side; title flips to the other peer.
    let repeat: Value = ts
        .post(&owner, "/chats/direct", json!({"user_id": member_id}))
        .await
        .json()
        .await
        .expect("chat body");
    assert_eq!(repeat["chat"]["id"], chat_id);
    let mirrored: Value = ts
        .post(&member, "/chats/direct", json!({"user_id": owner_id}))
        .await
        .json()
        .await
        .expect("chat body");
    assert_eq!(mirrored["chat"]["id"], chat_id);
    assert_eq!(mirrored["chat"]["title"], "Olive");
    assert_eq!(mirrored["chat"]["peer_user_id"], owner_id);
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn direct_chat_guards_reject_self_strangers_and_the_family_less() {
    let ts = spawn_server().await;
    let (owner, owner_id, _member, _) = family_of_two(&ts).await;
    let (outsider, outsider_id) = ts.register("outsider", "Out Sider").await;

    let dm_self = ts
        .post(&owner, "/chats/direct", json!({"user_id": owner_id}))
        .await;
    assert_error(dm_self, 400, "cannot_dm_self").await;

    let unknown = ts
        .post(&owner, "/chats/direct", json!({"user_id": 99999999}))
        .await;
    assert_error(unknown, 404, "user_not_found").await;

    let stranger = ts
        .post(&owner, "/chats/direct", json!({"user_id": outsider_id}))
        .await;
    assert_error(stranger, 409, "not_same_family").await;

    let family_less = ts
        .post(&outsider, "/chats/direct", json!({"user_id": owner_id}))
        .await;
    assert_error(family_less, 409, "not_in_family").await;
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn posting_a_message_is_idempotent_by_client_msg_id() {
    let ts = spawn_server().await;
    let (owner, _, _, _) = family_of_two(&ts).await;
    let chat_id = ts.family_chat_id(&owner).await;
    let client_msg_id = Uuid::new_v4().to_string();

    let first = ts
        .post_message(&owner, chat_id, &client_msg_id, "  Dinner at 7?  ")
        .await;
    assert_eq!(first.status(), 201, "first write creates");
    let first: Value = first.json().await.expect("message body");
    assert_eq!(first["message"]["body"], "Dinner at 7?", "body is trimmed");
    let message_id = first["message"]["id"].as_i64().expect("message id");

    // The retry returns the original as 200 — never a duplicate.
    let retry = ts
        .post_message(&owner, chat_id, &client_msg_id, "  Dinner at 7?  ")
        .await;
    assert_eq!(retry.status(), 200, "retry is a dedup hit");
    let retry: Value = retry.json().await.expect("message body");
    assert_eq!(retry["message"]["id"], message_id);

    let listed: Value = ts
        .get(&owner, &format!("/chats/{chat_id}/messages"))
        .await
        .json()
        .await
        .expect("messages");
    assert_eq!(listed["messages"].as_array().expect("array").len(), 1);
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn message_bodies_are_validated_and_membership_is_enforced() {
    let ts = spawn_server().await;
    let (owner, _, _, _) = family_of_two(&ts).await;
    let (outsider, _) = ts.register("outsider", "Out Sider").await;
    let chat_id = ts.family_chat_id(&owner).await;

    let empty = ts
        .post_message(&owner, chat_id, &Uuid::new_v4().to_string(), "   ")
        .await;
    assert_error(empty, 400, "message_empty").await;

    let too_long = ts
        .post_message(
            &owner,
            chat_id,
            &Uuid::new_v4().to_string(),
            &"x".repeat(4001),
        )
        .await;
    assert_error(too_long, 400, "message_too_long").await;

    let intruder = ts
        .post_message(&outsider, chat_id, &Uuid::new_v4().to_string(), "hello")
        .await;
    assert_error(intruder, 403, "not_chat_member").await;

    let phantom = ts
        .post_message(&owner, 99999999, &Uuid::new_v4().to_string(), "hello")
        .await;
    assert_error(phantom, 404, "chat_not_found").await;
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn pagination_pages_history_backwards_and_catch_up_forwards() {
    let ts = spawn_server().await;
    let (owner, _, _, _) = family_of_two(&ts).await;
    let chat_id = ts.family_chat_id(&owner).await;

    let mut ids = Vec::new();
    for n in 1..=5 {
        let response = ts
            .post_message(
                &owner,
                chat_id,
                &Uuid::new_v4().to_string(),
                &format!("m{n}"),
            )
            .await;
        assert_eq!(response.status(), 201);
        let body: Value = response.json().await.expect("message body");
        ids.push(body["message"]["id"].as_i64().expect("id"));
    }
    let bodies = |value: &Value| -> Vec<String> {
        value["messages"]
            .as_array()
            .expect("messages array")
            .iter()
            .map(|m| m["body"].as_str().expect("body").to_string())
            .collect()
    };

    // No cursor: the newest page, newest-first.
    let latest: Value = ts
        .get(&owner, &format!("/chats/{chat_id}/messages?limit=2"))
        .await
        .json()
        .await
        .expect("json");
    assert_eq!(bodies(&latest), vec!["m5", "m4"]);

    // before_id: strictly older, newest-first (history paging).
    let history: Value = ts
        .get(
            &owner,
            &format!("/chats/{chat_id}/messages?before_id={}", ids[2]),
        )
        .await
        .json()
        .await
        .expect("json");
    assert_eq!(bodies(&history), vec!["m2", "m1"]);

    // after_id: strictly newer, oldest-first (reconnect catch-up).
    let catch_up: Value = ts
        .get(
            &owner,
            &format!("/chats/{chat_id}/messages?after_id={}", ids[2]),
        )
        .await
        .json()
        .await
        .expect("json");
    assert_eq!(bodies(&catch_up), vec!["m4", "m5"]);

    // Cursor conflicts and garbage are invalid_pagination.
    let both = ts
        .get(
            &owner,
            &format!(
                "/chats/{chat_id}/messages?before_id={}&after_id={}",
                ids[2], ids[2]
            ),
        )
        .await;
    assert_error(both, 400, "invalid_pagination").await;
    let garbage = ts
        .get(&owner, &format!("/chats/{chat_id}/messages?limit=abc"))
        .await;
    assert_error(garbage, 400, "invalid_pagination").await;

    // limit is clamped, not rejected.
    let clamped: Value = ts
        .get(&owner, &format!("/chats/{chat_id}/messages?limit=100000"))
        .await
        .json()
        .await
        .expect("json");
    assert_eq!(bodies(&clamped).len(), 5);
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn read_markers_are_monotonic_and_drive_unread_counts() {
    let ts = spawn_server().await;
    let (owner, _, member, _) = family_of_two(&ts).await;
    let chat_id = ts.family_chat_id(&owner).await;

    let mut ids = Vec::new();
    for n in 1..=3 {
        let body: Value = ts
            .post_message(
                &member,
                chat_id,
                &Uuid::new_v4().to_string(),
                &format!("m{n}"),
            )
            .await
            .json()
            .await
            .expect("json");
        ids.push(body["message"]["id"].as_i64().expect("id"));
    }

    let unread = |chats: &Value| chats["chats"][0]["unread_count"].as_i64().expect("unread");

    // Own messages never count as unread for the sender.
    let member_chats: Value = ts.get(&member, "/chats").await.json().await.expect("json");
    assert_eq!(unread(&member_chats), 0);
    let owner_chats: Value = ts.get(&owner, "/chats").await.json().await.expect("json");
    assert_eq!(unread(&owner_chats), 3);
    assert_eq!(owner_chats["chats"][0]["last_message"]["body"], "m3");

    // Read up to the second message.
    let marked = ts
        .post(
            &owner,
            &format!("/chats/{chat_id}/read"),
            json!({"last_read_message_id": ids[1]}),
        )
        .await;
    assert_eq!(marked.status(), 204);
    let owner_chats: Value = ts.get(&owner, "/chats").await.json().await.expect("json");
    assert_eq!(unread(&owner_chats), 1);

    // Reporting an older marker cannot move the count backwards.
    let stale = ts
        .post(
            &owner,
            &format!("/chats/{chat_id}/read"),
            json!({"last_read_message_id": ids[0]}),
        )
        .await;
    assert_eq!(stale.status(), 204);
    let owner_chats: Value = ts.get(&owner, "/chats").await.json().await.expect("json");
    assert_eq!(unread(&owner_chats), 1, "marker is monotonic");
}

#[tokio::test]
#[ignore = "needs a reachable PostgreSQL server; run with --ignored"]
async fn devices_register_upsert_by_token_and_delete_only_their_own() {
    let ts = spawn_server().await;
    let (owner, _, member, _) = family_of_two(&ts).await;

    let bad_platform = ts
        .post(
            &owner,
            "/devices",
            json!({"platform": "windows", "push_token": null}),
        )
        .await;
    assert_eq!(bad_platform.status(), 400);

    let first: Value = ts
        .post(
            &owner,
            "/devices",
            json!({"platform": "ios", "push_token": "tok-1"}),
        )
        .await
        .json()
        .await
        .expect("json");
    let device_id = first["device_id"].as_i64().expect("device id");

    // Same token re-registered (even by another account) reuses the row.
    let reused: Value = ts
        .post(
            &member,
            "/devices",
            json!({"platform": "android", "push_token": "tok-1"}),
        )
        .await
        .json()
        .await
        .expect("json");
    assert_eq!(reused["device_id"], device_id, "upsert by push token");

    // The row now belongs to the member; the owner cannot delete it.
    let foreign = ts.delete(&owner, &format!("/devices/{device_id}")).await;
    assert_error(foreign, 404, "device_not_found").await;
    let own = ts.delete(&member, &format!("/devices/{device_id}")).await;
    assert_eq!(own.status(), 204);
    let gone = ts.delete(&member, &format!("/devices/{device_id}")).await;
    assert_error(gone, 404, "device_not_found").await;

    // Token-less devices insert plain rows.
    let no_token: Value = ts
        .post(
            &owner,
            "/devices",
            json!({"platform": "ios", "push_token": null}),
        )
        .await
        .json()
        .await
        .expect("json");
    assert!(no_token["device_id"].is_i64());
}
