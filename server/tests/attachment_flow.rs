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

/// A message and its attachment claim are one transaction.
///
/// Without that, the row is already committed when the claim fails — and
/// since a message with an attachment is allowed an empty body, what the
/// family is left with is a permanent, blank, undeletable bubble that is
/// also the chat's newest message.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_refused_claim_leaves_no_message_behind() {
    let server = spawn_server().await;
    let (owner, member, chat_id) = family_of_two(&server).await;

    async fn count(server: &TestServer, token: &str, chat_id: i64) -> usize {
        let page: Value = server
            .get(token, &format!("/chats/{chat_id}/messages"))
            .await
            .json()
            .await
            .expect("JSON");
        page["messages"].as_array().expect("array").len()
    }

    let before = count(&server, &owner, chat_id).await;

    // An id that never existed — what a client retrying past the sweeper's
    // 24 h grace period sends.
    assert_error(
        server
            .post(
                &owner,
                &format!("/chats/{chat_id}/messages"),
                json!({
                    "client_msg_id": Uuid::new_v4().to_string(),
                    "body": "",
                    "attachment_id": 999_999,
                }),
            )
            .await,
        404,
        "attachment_not_found",
    )
    .await;
    assert_eq!(count(&server, &owner, chat_id).await, before);

    // Somebody else's attachment: refused, and again nothing is left.
    let body: Value = upload(&server, &owner, "?kind=photo", "image/jpeg", jpeg_bytes(64))
        .await
        .json()
        .await
        .expect("JSON");
    let attachment_id = body["attachment"]["id"].as_i64().expect("id");
    assert_error(
        server
            .post(
                &member,
                &format!("/chats/{chat_id}/messages"),
                json!({
                    "client_msg_id": Uuid::new_v4().to_string(),
                    "body": "",
                    "attachment_id": attachment_id,
                }),
            )
            .await,
        404,
        "attachment_not_found",
    )
    .await;
    assert_eq!(count(&server, &member, chat_id).await, before);

    // And the double-claim path, which is what the claim-once test drives.
    let send = |token: String| {
        let server = &server;
        async move {
            server
                .post(
                    &token,
                    &format!("/chats/{chat_id}/messages"),
                    json!({
                        "client_msg_id": Uuid::new_v4().to_string(),
                        "body": "",
                        "attachment_id": attachment_id,
                    }),
                )
                .await
        }
    };
    assert_eq!(send(owner.clone()).await.status(), 201);
    let after_first = count(&server, &owner, chat_id).await;
    assert_error(send(owner.clone()).await, 409, "attachment_already_used").await;
    // The second attempt added nothing.
    assert_eq!(count(&server, &owner, chat_id).await, after_first);

    // Nothing blank anywhere: every message either has text or an attachment.
    let page: Value = server
        .get(&owner, &format!("/chats/{chat_id}/messages"))
        .await
        .json()
        .await
        .expect("JSON");
    for message in page["messages"].as_array().expect("array") {
        let empty = message["body"].as_str().unwrap_or("").is_empty();
        let has_attachment = message.get("attachment").is_some();
        assert!(
            !empty || has_attachment,
            "a blank message with no attachment survived: {message}"
        );
    }
}

/// Files are the kind that accepts anything — the point of the feature.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn any_file_is_accepted_and_carries_its_name() {
    let server = spawn_server().await;
    let (owner, member, chat_id) = family_of_two(&server).await;

    // Bytes that are not any accepted media type, declared as something
    // the server has never heard of. Both are fine for a file.
    let bytes = b"%PDF-1.7\nnot really a pdf either".to_vec();
    let response = upload(
        &server,
        &owner,
        "?kind=file&name=Rechnung%20M%C3%A4rz.pdf",
        "application/pdf",
        bytes.clone(),
    )
    .await;
    assert_eq!(response.status(), 201);
    let body: Value = response.json().await.expect("JSON");
    let attachment = &body["attachment"];
    let id = attachment["id"].as_i64().expect("id");
    assert_eq!(attachment["kind"], "file");
    assert_eq!(attachment["mime"], "application/pdf");
    assert_eq!(attachment["name"], "Rechnung März.pdf");
    assert_eq!(attachment["has_preview"], false);

    // A type the server has no opinion about at all.
    assert_eq!(
        upload(
            &server,
            &owner,
            "?kind=file&name=notes.xyz",
            "application/x-nettrash-invented",
            b"anything at all".to_vec(),
        )
        .await
        .status(),
        201
    );

    // It rides on a message like any other attachment.
    let sent = server
        .post(
            &owner,
            &format!("/chats/{chat_id}/messages"),
            json!({
                "client_msg_id": Uuid::new_v4().to_string(),
                "body": "",
                "attachment_id": id,
            }),
        )
        .await;
    assert_eq!(sent.status(), 201);
    let body: Value = sent.json().await.expect("JSON");
    assert_eq!(body["message"]["attachment"]["name"], "Rechnung März.pdf");

    // And the family can fetch it — as a DOWNLOAD, never as something a
    // browser might render.
    let fetched = server.get(&member, &format!("/attachments/{id}")).await;
    assert_eq!(fetched.status(), 200);
    assert_eq!(fetched.headers()["content-type"], "application/pdf");
    assert_eq!(fetched.headers()["x-content-type-options"], "nosniff");
    let disposition = fetched.headers()["content-disposition"]
        .to_str()
        .expect("ASCII");
    assert!(disposition.starts_with("attachment;"), "{disposition}");
    // The ASCII form for old clients, the real name in filename*.
    assert!(
        disposition.contains(r#"filename="Rechnung M_rz.pdf""#),
        "{disposition}"
    );
    assert!(
        disposition.contains("filename*=UTF-8''Rechnung%20M%C3%A4rz.pdf"),
        "{disposition}"
    );
    assert_eq!(fetched.bytes().await.unwrap().as_ref(), bytes.as_slice());
}

/// A photo is still served for rendering: the defensive headers are for
/// files, whose type nobody checked.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_photo_is_not_served_as_a_download() {
    let server = spawn_server().await;
    let (owner, _, _) = family_of_two(&server).await;

    let body: Value = upload(&server, &owner, "?kind=photo", "image/jpeg", jpeg_bytes(64))
        .await
        .json()
        .await
        .expect("JSON");
    let id = body["attachment"]["id"].as_i64().expect("id");
    // `name` is a file's field; a photo does not carry the key at all.
    assert!(body["attachment"].get("name").is_none());

    let fetched = server.get(&owner, &format!("/attachments/{id}")).await;
    assert_eq!(fetched.status(), 200);
    assert!(fetched.headers().get("content-disposition").is_none());
}

/// A name is a header value, and a header is a LINE. An uploader who can
/// put a newline in one can write headers of their own.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_hostile_filename_cannot_inject_a_header() {
    let server = spawn_server().await;
    let (owner, _, _) = family_of_two(&server).await;

    let hostile = "eco\r\nSet-Cookie: pwned=1\r\n\r\n<script>alert(1)</script>.html";
    let query = format!("?kind=file&name={}", urlencoding(hostile));
    let body: Value = upload(
        &server,
        &owner,
        &query,
        "text/html",
        b"<h1>hi</h1>".to_vec(),
    )
    .await
    .json()
    .await
    .expect("JSON");
    let id = body["attachment"]["id"].as_i64().expect("id");
    // Stored verbatim — the name is data; it is the HEADER that is escaped.
    assert_eq!(body["attachment"]["name"], hostile);

    let fetched = server.get(&owner, &format!("/attachments/{id}")).await;
    assert_eq!(fetched.status(), 200);
    // No header the uploader wrote survived.
    assert!(fetched.headers().get("set-cookie").is_none());
    let disposition = fetched.headers()["content-disposition"]
        .to_str()
        .expect("ASCII");
    // The words "Set-Cookie" DO survive inside the quoted filename, and
    // that is fine — they are a value, not a header. What must not survive
    // is the line break that would have made them one, or the quote that
    // would have ended the string early.
    assert!(!disposition.contains('\r') && !disposition.contains('\n'));
    assert_eq!(disposition.matches('"').count(), 2, "{disposition}");
    // Path separators go too: the name is a label, and a client that
    // joins it onto a directory must never be handed `../`.
    let quoted = disposition
        .split_once("filename=\"")
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(name, _)| name)
        .expect("a quoted filename");
    assert!(!quoted.contains('/') && !quoted.contains('\\'), "{quoted}");
    // And an uploaded page is a download, never a rendered page.
    assert_eq!(fetched.headers()["x-content-type-options"], "nosniff");
    assert!(disposition.starts_with("attachment;"));
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_file_needs_a_name_and_has_no_preview() {
    let server = spawn_server().await;
    let (owner, _, _) = family_of_two(&server).await;

    // Nameless: nobody recognises "attachment 34".
    assert_error(
        upload(
            &server,
            &owner,
            "?kind=file",
            "application/pdf",
            b"x".to_vec(),
        )
        .await,
        400,
        "invalid_attachment",
    )
    .await;
    assert_error(
        upload(
            &server,
            &owner,
            "?kind=file&name=%20%20",
            "application/pdf",
            b"x".to_vec(),
        )
        .await,
        400,
        "invalid_attachment",
    )
    .await;

    let body: Value = upload(
        &server,
        &owner,
        "?kind=file&name=taxes.zip",
        "application/zip",
        b"PK\x03\x04".to_vec(),
    )
    .await
    .json()
    .await
    .expect("JSON");
    let id = body["attachment"]["id"].as_i64().expect("id");

    assert_error(
        server
            .put_bytes(
                &owner,
                &format!("/attachments/{id}/preview"),
                "image/jpeg",
                jpeg_bytes(64),
            )
            .await,
        400,
        "invalid_attachment",
    )
    .await;
}

/// The ceiling is the same for every kind.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_oversized_file_is_refused_too() {
    let server = spawn_server().await;
    let (owner, _, _) = family_of_two(&server).await;

    let over = server.state.cfg.limits.max_attachment_bytes + 1;
    assert_error(
        upload(
            &server,
            &owner,
            "?kind=file&name=big.bin",
            "application/octet-stream",
            vec![0u8; over],
        )
        .await,
        413,
        "attachment_too_large",
    )
    .await;
}

/// Percent-encode for a query string; enough for these tests' names.
fn urlencoding(value: &str) -> String {
    value
        .bytes()
        .map(|byte| {
            let c = byte as char;
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '~') {
                c.to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

/// One copy per family: forwarding the same photo three times must not
/// put three copies on the disk.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn identical_uploads_in_one_family_share_one_file() {
    let server = spawn_server().await;
    let (owner, member, chat_id) = family_of_two(&server).await;
    let bytes = jpeg_bytes(4096);

    let mut ids = Vec::new();
    for token in [&owner, &owner, &member] {
        let body: Value = upload(&server, token, "?kind=photo", "image/jpeg", bytes.clone())
            .await
            .json()
            .await
            .expect("JSON");
        ids.push(body["attachment"]["id"].as_i64().expect("id"));
    }

    let keys: Vec<String> = sqlx::query_scalar("SELECT storage_key FROM attachments ORDER BY id")
        .fetch_all(&server.state.pool)
        .await
        .expect("keys");
    let distinct: std::collections::HashSet<&String> = keys.iter().collect();
    assert_eq!(distinct.len(), 1, "three identical uploads kept {keys:?}");
    // Including across MEMBERS of the family, not just one uploader.
    assert_eq!(keys.len(), 3);

    // Every row still serves its own bytes, to its own uploader.
    for (id, token) in ids.iter().zip([&owner, &owner, &member]) {
        let fetched = server.get(token, &format!("/attachments/{id}")).await;
        assert_eq!(fetched.status(), 200);
        assert_eq!(fetched.bytes().await.unwrap().as_ref(), bytes.as_slice());
    }

    // SHARING BYTES MUST NOT SHARE ACCESS. The member's upload is
    // unclaimed, so it is theirs alone — even though the file on disk is
    // the very one the owner may read through their own row.
    assert_error(
        server
            .get(&owner, &format!("/attachments/{}", ids[2]))
            .await,
        404,
        "attachment_not_found",
    )
    .await;

    // Different bytes are still a different file.
    let other: Value = upload(
        &server,
        &owner,
        "?kind=photo",
        "image/jpeg",
        jpeg_bytes(8192),
    )
    .await
    .json()
    .await
    .expect("JSON");
    let other_id = other["attachment"]["id"].as_i64().expect("id");
    let other_key: String = sqlx::query_scalar("SELECT storage_key FROM attachments WHERE id = $1")
        .bind(other_id)
        .fetch_one(&server.state.pool)
        .await
        .expect("key");
    assert!(!distinct.contains(&other_key));

    let _ = chat_id;
}

/// Two families holding the same bytes keep their own copies — a family's
/// attachments stay a self-contained set of files.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn two_families_do_not_share_a_file() {
    let server = spawn_server().await;
    let (owner, _) = server.register("owner", "Olive").await;
    server.create_family(&owner, "The Smiths").await;
    let (other, _) = server.register("stranger", "Sam").await;
    server.create_family(&other, "The Joneses").await;

    let bytes = jpeg_bytes(2048);
    for token in [&owner, &other] {
        assert_eq!(
            upload(&server, token, "?kind=photo", "image/jpeg", bytes.clone())
                .await
                .status(),
            201
        );
    }

    let keys: Vec<String> = sqlx::query_scalar("SELECT storage_key FROM attachments ORDER BY id")
        .fetch_all(&server.state.pool)
        .await
        .expect("keys");
    assert_eq!(keys.len(), 2);
    assert_ne!(keys[0], keys[1], "two families shared a file");
}

/// THE DANGEROUS CASE. Sweeping one abandoned upload must not delete bytes
/// another message is still pointing at — the database would look fine and
/// the photo would be gone.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn sweeping_a_shared_upload_leaves_the_other_intact() {
    let server = spawn_server().await;
    let (owner, _, chat_id) = family_of_two(&server).await;
    let bytes = jpeg_bytes(1024);

    async fn upload_id(server: &TestServer, token: &str, bytes: Vec<u8>) -> i64 {
        let body: Value = upload(server, token, "?kind=photo", "image/jpeg", bytes)
            .await
            .json()
            .await
            .expect("JSON");
        body["attachment"]["id"].as_i64().expect("id")
    }

    let kept = upload_id(&server, &owner, bytes.clone()).await;
    let abandoned = upload_id(&server, &owner, bytes.clone()).await;

    // One of them is claimed by a message; the other is the send the user
    // gave up on.
    server
        .post(
            &owner,
            &format!("/chats/{chat_id}/messages"),
            json!({
                "client_msg_id": Uuid::new_v4().to_string(),
                "body": "",
                "attachment_id": kept,
            }),
        )
        .await;

    sqlx::query("UPDATE attachments SET created_at = now() - interval '48 hours' WHERE id = $1")
        .bind(abandoned)
        .execute(&server.state.pool)
        .await
        .expect("age the orphan");

    let swept = family_connect::handlers_attachment::sweep_unclaimed(&server.state)
        .await
        .expect("sweep");
    assert_eq!(swept, 1);

    // The surviving message's bytes are still there and still correct.
    let fetched = server.get(&owner, &format!("/attachments/{kept}")).await;
    assert_eq!(fetched.status(), 200);
    assert_eq!(fetched.bytes().await.unwrap().as_ref(), bytes.as_slice());
}

/// And when the LAST row goes, the file goes with it — dedup must not turn
/// the sweeper into a no-op that leaks disk forever.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_last_reference_takes_the_file_with_it() {
    let server = spawn_server().await;
    let (owner, _, _) = family_of_two(&server).await;
    let bytes = jpeg_bytes(1024);

    for _ in 0..2 {
        assert_eq!(
            upload(&server, &owner, "?kind=photo", "image/jpeg", bytes.clone())
                .await
                .status(),
            201
        );
    }
    let key: String = sqlx::query_scalar("SELECT storage_key FROM attachments ORDER BY id LIMIT 1")
        .fetch_one(&server.state.pool)
        .await
        .expect("key");
    let path = server.state.storage.blob_path(&key);
    assert!(path.exists());

    sqlx::query("UPDATE attachments SET created_at = now() - interval '48 hours'")
        .execute(&server.state.pool)
        .await
        .expect("age both");
    let swept = family_connect::handlers_attachment::sweep_unclaimed(&server.state)
        .await
        .expect("sweep");
    assert_eq!(swept, 2);
    assert!(!path.exists(), "the shared file outlived its last row");
}

/// A player seeks by asking for byte ranges; the endpoint advertises
/// `Accept-Ranges: bytes`, so it has to mean it.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_video_is_served_in_ranges() {
    let server = spawn_server().await;
    let (owner, _, _) = family_of_two(&server).await;

    // Bytes with a recognisable tail, so a suffix range proves it seeked
    // rather than re-sending the head.
    let mut bytes = mp4_bytes(1000);
    for (index, byte) in bytes.iter_mut().enumerate().skip(12) {
        *byte = (index % 251) as u8;
    }
    let body: Value = upload(&server, &owner, "?kind=video", "video/mp4", bytes.clone())
        .await
        .json()
        .await
        .expect("JSON");
    let id = body["attachment"]["id"].as_i64().expect("id");
    let path = format!("/attachments/{id}");

    // A middle range.
    let response = server
        .get_with(&owner, &path, &[("range", "bytes=100-199")])
        .await;
    assert_eq!(response.status(), 206);
    assert_eq!(response.headers()["content-range"], "bytes 100-199/1000");
    assert_eq!(response.headers()["content-length"], "100");
    assert_eq!(response.bytes().await.unwrap().as_ref(), &bytes[100..200]);

    // Open-ended: "from here to the end", which is what a player sends
    // after seeking.
    let response = server
        .get_with(&owner, &path, &[("range", "bytes=900-")])
        .await;
    assert_eq!(response.status(), 206);
    assert_eq!(response.headers()["content-range"], "bytes 900-999/1000");
    assert_eq!(response.bytes().await.unwrap().as_ref(), &bytes[900..]);

    // A suffix range: the LAST 50 bytes — where an MP4 keeps its index.
    let response = server
        .get_with(&owner, &path, &[("range", "bytes=-50")])
        .await;
    assert_eq!(response.status(), 206);
    assert_eq!(response.headers()["content-range"], "bytes 950-999/1000");
    assert_eq!(response.bytes().await.unwrap().as_ref(), &bytes[950..]);

    // Past the end is 416 carrying the real size, not a truncated 206.
    let response = server
        .get_with(&owner, &path, &[("range", "bytes=5000-6000")])
        .await;
    assert_eq!(response.status(), 416);
    assert_eq!(response.headers()["content-range"], "bytes */1000");

    // No Range header still means the whole file, and a range unit the
    // server does not speak is ignored rather than refused (RFC 9110).
    let response = server.get(&owner, &path).await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.bytes().await.unwrap().len(), 1000);
    let response = server
        .get_with(&owner, &path, &[("range", "furlongs=1-2")])
        .await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.bytes().await.unwrap().len(), 1000);
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
