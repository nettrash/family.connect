//! `GET /families/mine/stats` (docs/protocol.md, "Family statistics").
//!
//! The two things worth pinning are the ones that are easy to get quietly
//! wrong: totals that multiply when messages and attachments are joined
//! together, and the gap between what was SENT and what is STORED, which is
//! the whole point of counting both.

mod common;

use common::{TestServer, spawn_server};
use serde_json::{Value, json};
use uuid::Uuid;

fn jpeg_bytes(len: usize) -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE0];
    bytes.resize(len.max(4), 0x00);
    bytes
}

fn mp4_bytes(len: usize) -> Vec<u8> {
    let mut bytes = vec![0x00, 0x00, 0x00, 0x18];
    bytes.extend_from_slice(b"ftypmp42");
    bytes.resize(len.max(12), 0x00);
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

async fn send(ts: &TestServer, token: &str, chat_id: i64, body: &str) {
    let response = ts
        .post_message(token, chat_id, &Uuid::new_v4().to_string(), body)
        .await;
    assert_eq!(response.status(), 201);
}

/// Upload something and send it, answering with the attachment id.
async fn send_attachment(
    ts: &TestServer,
    token: &str,
    chat_id: i64,
    query: &str,
    mime: &str,
    bytes: Vec<u8>,
) -> i64 {
    let uploaded = ts
        .put_bytes_method("POST", token, &format!("/attachments{query}"), mime, bytes)
        .await;
    assert_eq!(uploaded.status(), 201);
    let body: Value = uploaded.json().await.expect("JSON");
    let id = body["attachment"]["id"].as_i64().expect("id");
    let sent = ts
        .post(
            token,
            &format!("/chats/{chat_id}/messages"),
            json!({"client_msg_id": Uuid::new_v4().to_string(), "body": "",
                   "attachment_id": id}),
        )
        .await;
    assert_eq!(sent.status(), 201);
    id
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn stats_count_messages_per_member_and_in_total() {
    let server = spawn_server().await;
    let (owner, member, chat_id) = family_of_two(&server).await;

    send(&server, &owner, chat_id, "one").await;
    send(&server, &owner, chat_id, "two").await;
    send(&server, &member, chat_id, "three").await;

    let stats: Value = server
        .get(&member, "/families/mine/stats")
        .await
        .json()
        .await
        .expect("JSON");

    assert_eq!(stats["totals"]["messages"].as_i64(), Some(3));
    assert_eq!(stats["totals"]["members"].as_i64(), Some(2));

    let members = stats["members"].as_array().expect("array");
    assert_eq!(members.len(), 2);
    // Ordered by messages sent, so the busiest is first.
    assert_eq!(members[0]["display_name"], "Olive");
    assert_eq!(members[0]["messages"].as_i64(), Some(2));
    assert_eq!(members[1]["display_name"], "Junior");
    assert_eq!(members[1]["messages"].as_i64(), Some(1));
}

/// Every member sees the same numbers — not an owner's dashboard.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_plain_member_sees_the_same_statistics_as_the_owner() {
    let server = spawn_server().await;
    let (owner, member, chat_id) = family_of_two(&server).await;
    send(&server, &owner, chat_id, "hello").await;

    let as_owner: Value = server
        .get(&owner, "/families/mine/stats")
        .await
        .json()
        .await
        .expect("JSON");
    let as_member: Value = server
        .get(&member, "/families/mine/stats")
        .await
        .json()
        .await
        .expect("JSON");

    assert_eq!(as_owner["totals"], as_member["totals"]);
    assert_eq!(as_owner["members"], as_member["members"]);
}

/// Attachments must not multiply the message count, and each kind is
/// counted separately.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn attachments_are_counted_by_kind_without_inflating_messages() {
    let server = spawn_server().await;
    let (owner, _, chat_id) = family_of_two(&server).await;

    send(&server, &owner, chat_id, "just words").await;
    send_attachment(
        &server,
        &owner,
        chat_id,
        "?kind=photo&width=100&height=80",
        "image/jpeg",
        jpeg_bytes(2048),
    )
    .await;
    send_attachment(
        &server,
        &owner,
        chat_id,
        "?kind=video&duration_ms=1000",
        "video/mp4",
        mp4_bytes(4096),
    )
    .await;
    send_attachment(
        &server,
        &owner,
        chat_id,
        "?kind=audio&duration_ms=1000",
        "audio/mp4",
        mp4_bytes(1024),
    )
    .await;

    let stats: Value = server
        .get(&owner, "/families/mine/stats")
        .await
        .json()
        .await
        .expect("JSON");

    let totals = &stats["totals"];
    // Four messages, not four multiplied by anything.
    assert_eq!(totals["messages"].as_i64(), Some(4));
    assert_eq!(totals["attachments"]["count"].as_i64(), Some(3));
    assert_eq!(totals["attachments"]["photo"].as_i64(), Some(1));
    assert_eq!(totals["attachments"]["video"].as_i64(), Some(1));
    assert_eq!(totals["attachments"]["audio"].as_i64(), Some(1));
    assert_eq!(totals["attachments"]["file"].as_i64(), Some(0));
    assert_eq!(
        totals["attachments"]["bytes"].as_i64(),
        Some(2048 + 4096 + 1024)
    );

    let owner_row = &stats["members"][0];
    assert_eq!(owner_row["messages"].as_i64(), Some(4));
    assert_eq!(owner_row["attachments"]["count"].as_i64(), Some(3));
}

/// An album is ONE message carrying N attachments — the message total must
/// count it once, and the attachment totals must count each of the N.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_album_counts_one_message_and_every_attachment() {
    let server = spawn_server().await;
    let (owner, _, chat_id) = family_of_two(&server).await;

    send(&server, &owner, chat_id, "just words").await;

    // Three photos with distinct bytes, on ONE message.
    let mut ids = Vec::new();
    for len in [512usize, 640, 768] {
        let uploaded = server
            .put_bytes_method(
                "POST",
                &owner,
                "/attachments?kind=photo",
                "image/jpeg",
                jpeg_bytes(len),
            )
            .await;
        assert_eq!(uploaded.status(), 201);
        let body: Value = uploaded.json().await.expect("JSON");
        ids.push(body["attachment"]["id"].as_i64().expect("id"));
    }
    let sent = server
        .post(
            &owner,
            &format!("/chats/{chat_id}/messages"),
            json!({"client_msg_id": Uuid::new_v4().to_string(), "body": "",
                   "attachment_ids": ids}),
        )
        .await;
    assert_eq!(sent.status(), 201);

    let stats: Value = server
        .get(&owner, "/families/mine/stats")
        .await
        .json()
        .await
        .expect("JSON");
    let totals = &stats["totals"];
    assert_eq!(
        totals["messages"].as_i64(),
        Some(2),
        "two messages, however many attachments one carried: {totals}"
    );
    assert_eq!(totals["attachments"]["count"].as_i64(), Some(3));
    assert_eq!(totals["attachments"]["photo"].as_i64(), Some(3));
    assert_eq!(
        totals["attachments"]["bytes"].as_i64(),
        Some(512 + 640 + 768)
    );

    let owner_row = &stats["members"][0];
    assert_eq!(owner_row["messages"].as_i64(), Some(2));
    assert_eq!(owner_row["attachments"]["count"].as_i64(), Some(3));
}

/// `bytes` is what was sent; `stored_bytes` counts each distinct file once.
/// The gap between them is what one-copy-per-family saved, and reporting
/// only one of the two would hide it.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn identical_files_are_sent_twice_but_stored_once() {
    let server = spawn_server().await;
    let (owner, member, chat_id) = family_of_two(&server).await;

    // The SAME bytes, from two different members.
    send_attachment(
        &server,
        &owner,
        chat_id,
        "?kind=photo&width=100&height=80",
        "image/jpeg",
        jpeg_bytes(4096),
    )
    .await;
    send_attachment(
        &server,
        &member,
        chat_id,
        "?kind=photo&width=100&height=80",
        "image/jpeg",
        jpeg_bytes(4096),
    )
    .await;

    let stats: Value = server
        .get(&owner, "/families/mine/stats")
        .await
        .json()
        .await
        .expect("JSON");
    let attachments = &stats["totals"]["attachments"];

    assert_eq!(attachments["count"].as_i64(), Some(2), "two were sent");
    assert_eq!(
        attachments["bytes"].as_i64(),
        Some(8192),
        "what was sent adds up twice"
    );
    assert_eq!(
        attachments["stored_bytes"].as_i64(),
        Some(4096),
        "but the disk holds one copy"
    );

    // Each member is credited with what THEY sent; stored_bytes is a family
    // total only, because a shared file belongs to neither alone.
    let members = stats["members"].as_array().expect("array");
    for row in members {
        assert_eq!(row["attachments"]["bytes"].as_i64(), Some(4096));
        assert!(row["attachments"].get("stored_bytes").is_none());
    }
}

/// A member who has sent nothing still appears, with zeros — "who has sent
/// the fewest" is as interesting a question as the opposite.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_member_who_has_sent_nothing_still_appears() {
    let server = spawn_server().await;
    let (owner, _, chat_id) = family_of_two(&server).await;
    send(&server, &owner, chat_id, "hello").await;

    let stats: Value = server
        .get(&owner, "/families/mine/stats")
        .await
        .json()
        .await
        .expect("JSON");

    let quiet = stats["members"]
        .as_array()
        .expect("array")
        .iter()
        .find(|row| row["display_name"] == "Junior")
        .expect("the quiet member is listed");
    assert_eq!(quiet["messages"].as_i64(), Some(0));
    assert_eq!(quiet["attachments"]["count"].as_i64(), Some(0));
    assert_eq!(quiet["ai"]["questions"].as_i64(), Some(0));
}

/// No family, nothing to count — the same answer every family endpoint
/// gives.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn someone_with_no_family_is_refused() {
    let server = spawn_server().await;
    let (loner, _) = server.register("loner", "Lee").await;

    let response = server.get(&loner, "/families/mine/stats").await;
    assert_eq!(response.status(), 409);
    let body: Value = response.json().await.expect("JSON");
    assert_eq!(body["error"]["code"], "not_in_family");
}
