//! Integration: photos and videos — upload, claim, access rules, the size
//! ceiling, and the sweeper that stops abandoned uploads from living
//! forever (protocol.md, "Photos and videos").

mod common;

use common::{TestServer, assert_error, spawn_server};
use serde_json::{Value, json};
use uuid::Uuid;

/// Smallest bytes that pass each magic-number gate. The server never
/// decodes, so a real photo would only make the test slower.
fn jpeg_bytes(len: usize) -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE0];
    bytes.resize(len.max(4), 0x00);
    bytes
}

/// ISO base media: "ftyp" at offset 4, which is what an MP4 and a HEIC
/// share and how the server tells them from anything else.
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

async fn upload(
    ts: &TestServer,
    token: &str,
    query: &str,
    mime: &str,
    bytes: Vec<u8>,
) -> reqwest::Response {
    ts.put_bytes_method("POST", token, &format!("/attachments{query}"), mime, bytes)
        .await
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_photo_is_uploaded_claimed_and_readable_by_the_family() {
    let server = spawn_server().await;
    let (owner, member, chat_id) = family_of_two(&server).await;

    let response = upload(
        &server,
        &owner,
        "?kind=photo&width=1600&height=1200",
        "image/jpeg",
        jpeg_bytes(4096),
    )
    .await;
    assert_eq!(response.status(), 201);
    let body: Value = response.json().await.expect("JSON");
    let attachment = &body["attachment"];
    let attachment_id = attachment["id"].as_i64().expect("id");
    assert_eq!(attachment["kind"], "photo");
    assert_eq!(attachment["mime"], "image/jpeg");
    assert_eq!(attachment["size"].as_i64(), Some(4096));
    assert_eq!(attachment["width"].as_i64(), Some(1600));
    assert_eq!(attachment["has_preview"], false);

    // Before a message claims it, only the uploader may read it — and the
    // refusal is the same 404 a nonexistent id gets.
    assert_error(
        server
            .get(&member, &format!("/attachments/{attachment_id}"))
            .await,
        404,
        "attachment_not_found",
    )
    .await;
    assert_eq!(
        server
            .get(&owner, &format!("/attachments/{attachment_id}"))
            .await
            .status(),
        200
    );

    // A photo needs no caption: an empty body is allowed WITH an attachment.
    let sent = server
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
    assert_eq!(sent.status(), 201);
    let body: Value = sent.json().await.expect("JSON");
    assert_eq!(
        body["message"]["attachment"]["id"].as_i64(),
        Some(attachment_id)
    );

    // Now every member of the chat can read the bytes, and sees it on the
    // message.
    let fetched = server
        .get(&member, &format!("/attachments/{attachment_id}"))
        .await;
    assert_eq!(fetched.status(), 200);
    assert_eq!(fetched.headers()["content-type"], "image/jpeg");
    assert_eq!(fetched.bytes().await.unwrap().len(), 4096);

    let page: Value = server
        .get(&member, &format!("/chats/{chat_id}/messages"))
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(
        page["messages"][0]["attachment"]["id"].as_i64(),
        Some(attachment_id)
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_video_carries_its_duration() {
    let server = spawn_server().await;
    let (owner, _, _) = family_of_two(&server).await;

    let body: Value = upload(
        &server,
        &owner,
        "?kind=video&width=1920&height=1080&duration_ms=8400",
        "video/mp4",
        mp4_bytes(2048),
    )
    .await
    .json()
    .await
    .expect("JSON");
    assert_eq!(body["attachment"]["kind"], "video");
    assert_eq!(body["attachment"]["duration_ms"].as_i64(), Some(8400));
}

/// The ceiling nettrash asked for. The route's own body limit sits slightly
/// above it so the handler can answer with the protocol's error rather than
/// a bare 413 that says nothing.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_oversized_upload_is_refused_with_the_protocol_error() {
    let server = spawn_server().await;
    let (owner, _, _) = family_of_two(&server).await;

    // The test server's ceiling is lowered so this stays fast; what is
    // being checked is that the refusal carries a code, not the number.
    let over = server.state.cfg.limits.max_attachment_bytes + 1;
    assert_error(
        upload(
            &server,
            &owner,
            "?kind=photo",
            "image/jpeg",
            jpeg_bytes(over),
        )
        .await,
        413,
        "attachment_too_large",
    )
    .await;

    // And nothing is left behind: no row, and no file.
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM attachments")
        .fetch_one(&server.state.pool)
        .await
        .expect("count");
    assert_eq!(rows, 0);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn bad_uploads_are_refused() {
    let server = spawn_server().await;
    let (owner, _, _) = family_of_two(&server).await;

    // A type the protocol does not accept.
    assert_error(
        upload(&server, &owner, "", "application/pdf", jpeg_bytes(64)).await,
        415,
        "invalid_attachment",
    )
    .await;
    // Bytes that do not match the declared type.
    assert_error(
        upload(&server, &owner, "?kind=photo", "image/png", jpeg_bytes(64)).await,
        400,
        "invalid_attachment",
    )
    .await;
    // A kind that contradicts the media type.
    assert_error(
        upload(&server, &owner, "?kind=video", "image/jpeg", jpeg_bytes(64)).await,
        400,
        "invalid_attachment",
    )
    .await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_attachment_can_be_claimed_only_once() {
    let server = spawn_server().await;
    let (owner, member, chat_id) = family_of_two(&server).await;

    let body: Value = upload(
        &server,
        &owner,
        "?kind=photo",
        "image/jpeg",
        jpeg_bytes(128),
    )
    .await
    .json()
    .await
    .expect("JSON");
    let attachment_id = body["attachment"]["id"].as_i64().expect("id");

    async fn send(
        server: &TestServer,
        token: &str,
        chat_id: i64,
        attachment_id: i64,
    ) -> reqwest::Response {
        server
            .post(
                token,
                &format!("/chats/{chat_id}/messages"),
                json!({
                    "client_msg_id": Uuid::new_v4().to_string(),
                    "body": "",
                    "attachment_id": attachment_id,
                }),
            )
            .await
    }

    assert_eq!(
        send(&server, &owner, chat_id, attachment_id).await.status(),
        201
    );
    assert_error(
        send(&server, &owner, chat_id, attachment_id).await,
        409,
        "attachment_already_used",
    )
    .await;
    // And somebody else's attachment is not theirs to claim.
    assert_error(
        send(&server, &member, chat_id, attachment_id).await,
        404,
        "attachment_not_found",
    )
    .await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_preview_is_uploaded_separately_and_flagged() {
    let server = spawn_server().await;
    let (owner, _, _) = family_of_two(&server).await;

    let body: Value = upload(&server, &owner, "?kind=video", "video/mp4", mp4_bytes(512))
        .await
        .json()
        .await
        .expect("JSON");
    let attachment_id = body["attachment"]["id"].as_i64().expect("id");

    // No preview yet.
    assert_error(
        server
            .get(&owner, &format!("/attachments/{attachment_id}/preview"))
            .await,
        404,
        "attachment_not_found",
    )
    .await;

    assert_eq!(
        server
            .put_bytes(
                &owner,
                &format!("/attachments/{attachment_id}/preview"),
                "image/jpeg",
                jpeg_bytes(256),
            )
            .await
            .status(),
        204
    );

    let fetched = server
        .get(&owner, &format!("/attachments/{attachment_id}/preview"))
        .await;
    assert_eq!(fetched.status(), 200);
    assert_eq!(fetched.headers()["content-type"], "image/jpeg");

    // The flag follows, so a bubble knows there is something to draw.
    let page: Value = server
        .get(&owner, "/chats")
        .await
        .json()
        .await
        .expect("JSON");
    let _ = page; // the flag is on the attachment, checked on the message below
    let body: Value = server
        .post(
            &owner,
            &format!("/chats/{}/messages", server.family_chat_id(&owner).await),
            json!({
                "client_msg_id": Uuid::new_v4().to_string(),
                "body": "",
                "attachment_id": attachment_id,
            }),
        )
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(body["message"]["attachment"]["has_preview"], true);
}

/// A send the user abandoned must not leave 100 MB on the server forever.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn unclaimed_uploads_are_swept() {
    let server = spawn_server().await;
    let (owner, _, chat_id) = family_of_two(&server).await;

    let orphan: Value = upload(
        &server,
        &owner,
        "?kind=photo",
        "image/jpeg",
        jpeg_bytes(128),
    )
    .await
    .json()
    .await
    .expect("JSON");
    let orphan_id = orphan["attachment"]["id"].as_i64().expect("id");

    let claimed: Value = upload(
        &server,
        &owner,
        "?kind=photo",
        "image/jpeg",
        jpeg_bytes(128),
    )
    .await
    .json()
    .await
    .expect("JSON");
    let claimed_id = claimed["attachment"]["id"].as_i64().expect("id");
    server
        .post(
            &owner,
            &format!("/chats/{chat_id}/messages"),
            json!({
                "client_msg_id": Uuid::new_v4().to_string(),
                "body": "",
                "attachment_id": claimed_id,
            }),
        )
        .await;

    // Age the unclaimed one past the grace period.
    sqlx::query("UPDATE attachments SET created_at = now() - interval '48 hours' WHERE id = $1")
        .bind(orphan_id)
        .execute(&server.state.pool)
        .await
        .expect("age the orphan");

    let swept = family_connect::handlers_attachment::sweep_unclaimed(&server.state)
        .await
        .expect("sweep");
    assert_eq!(swept, 1);

    // The orphan is gone; the claimed one is untouched however old it is.
    assert_error(
        server
            .get(&owner, &format!("/attachments/{orphan_id}"))
            .await,
        404,
        "attachment_not_found",
    )
    .await;
    assert_eq!(
        server
            .get(&owner, &format!("/attachments/{claimed_id}"))
            .await
            .status(),
        200
    );
}

/// A message with neither text nor attachment is still empty.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_empty_message_with_no_attachment_is_still_refused() {
    let server = spawn_server().await;
    let (owner, _, chat_id) = family_of_two(&server).await;

    assert_error(
        server
            .post(
                &owner,
                &format!("/chats/{chat_id}/messages"),
                json!({"client_msg_id": Uuid::new_v4().to_string(), "body": "   "}),
            )
            .await,
        400,
        "message_empty",
    )
    .await;
}
