//! Profile pictures end to end (protocol.md "Profile pictures").
//!
//! The rules worth pinning are the ones a reader of the endpoint table
//! would otherwise have to trust: the version only ever goes up (and is
//! never reused, even across a delete that reports 0), the same 404 hides
//! "no such user", "not your family" and "no picture" from each other,
//! and the content type is believed only when the bytes agree with it.

mod common;

use common::{assert_error, spawn_server};

/// Smallest bytes that pass the magic-number gate. The server never
/// decodes, so a real image would only make the test slower.
fn jpeg_bytes(len: usize) -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE0];
    bytes.resize(len.max(4), 0x00);
    bytes
}

fn png_bytes() -> Vec<u8> {
    let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    bytes.extend_from_slice(&[0x00; 16]);
    bytes
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn upload_bumps_version_and_serves_the_bytes() {
    let server = spawn_server().await;
    let (token, user_id) = server.register("anna", "Anna").await;

    // No picture yet: version 0, and the GET 404s.
    let me = server.get(&token, "/me").await;
    assert_eq!(me.status(), 200);
    let body: serde_json::Value = me.json().await.unwrap();
    assert_eq!(body["user"]["avatar_version"], 0);
    assert_error(
        server
            .get(&token, &format!("/users/{user_id}/avatar"))
            .await,
        404,
        "user_not_found",
    )
    .await;

    let uploaded = jpeg_bytes(64);
    let response = server
        .put_bytes(&token, "/me/avatar", "image/jpeg", uploaded.clone())
        .await;
    assert_eq!(response.status(), 200);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["user"]["avatar_version"], 1);

    let fetched = server
        .get(&token, &format!("/users/{user_id}/avatar"))
        .await;
    assert_eq!(fetched.status(), 200);
    assert_eq!(fetched.headers()["content-type"], "image/jpeg");
    assert_eq!(
        fetched.headers()["cache-control"],
        "private, max-age=31536000, immutable"
    );
    let etag = fetched.headers()["etag"].to_str().unwrap().to_string();
    assert_eq!(etag, format!("\"{user_id}-1\""));
    assert_eq!(fetched.bytes().await.unwrap().to_vec(), uploaded);

    // A second upload replaces the bytes and moves the version on, which
    // is what makes the immutable caching above safe.
    let replaced = png_bytes();
    let response = server
        .put_bytes(&token, "/me/avatar", "image/png", replaced.clone())
        .await;
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["user"]["avatar_version"], 2);
    let fetched = server
        .get(&token, &format!("/users/{user_id}/avatar"))
        .await;
    assert_eq!(fetched.headers()["content-type"], "image/png");
    assert_eq!(fetched.bytes().await.unwrap().to_vec(), replaced);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn if_none_match_answers_304() {
    let server = spawn_server().await;
    let (token, user_id) = server.register("anna", "Anna").await;
    server
        .put_bytes(&token, "/me/avatar", "image/jpeg", jpeg_bytes(32))
        .await;

    let path = format!("/users/{user_id}/avatar");
    let etag = server.get(&token, &path).await.headers()["etag"]
        .to_str()
        .unwrap()
        .to_string();

    let cached = server
        .get_with(&token, &path, &[("If-None-Match", &etag)])
        .await;
    assert_eq!(cached.status(), 304);

    // A stale validator must NOT be honoured — that is the case where a
    // client holds the previous picture.
    let stale = server
        .get_with(&token, &path, &[("If-None-Match", "\"0-0\"")])
        .await;
    assert_eq!(stale.status(), 200);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn delete_resets_the_version_and_is_idempotent() {
    let server = spawn_server().await;
    let (token, user_id) = server.register("anna", "Anna").await;
    server
        .put_bytes(&token, "/me/avatar", "image/jpeg", jpeg_bytes(32))
        .await;

    assert_eq!(server.delete(&token, "/me/avatar").await.status(), 204);
    // Deleting nothing is still a 204.
    assert_eq!(server.delete(&token, "/me/avatar").await.status(), 204);

    let me = server.get(&token, "/me").await;
    let body: serde_json::Value = me.json().await.unwrap();
    assert_eq!(body["user"]["avatar_version"], 0);
    assert_error(
        server
            .get(&token, &format!("/users/{user_id}/avatar"))
            .await,
        404,
        "user_not_found",
    )
    .await;
}

/// The delete resets the reported version to 0, but NOT the counter
/// behind it. Clients are told they may cache a picture forever under
/// `(user_id, avatar_version)`, so handing version 1 to a second, different
/// picture would leave every peer that cached the first one showing it
/// for the rest of the session.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_re_upload_after_a_delete_does_not_reuse_the_version() {
    let server = spawn_server().await;
    let (token, user_id) = server.register("anna", "Anna").await;

    let first = jpeg_bytes(32);
    let response = server
        .put_bytes(&token, "/me/avatar", "image/jpeg", first.clone())
        .await;
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["user"]["avatar_version"], 1);

    assert_eq!(server.delete(&token, "/me/avatar").await.status(), 204);

    let second = png_bytes();
    let response = server
        .put_bytes(&token, "/me/avatar", "image/png", second.clone())
        .await;
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(
        body["user"]["avatar_version"], 2,
        "version 1 was already handed out for a different picture"
    );

    // And the bytes served are the new ones, under the new ETag.
    let fetched = server
        .get(&token, &format!("/users/{user_id}/avatar"))
        .await;
    assert_eq!(fetched.status(), 200);
    assert_eq!(fetched.headers()["content-type"], "image/png");
    assert_eq!(
        fetched.headers()["etag"],
        format!("\"{user_id}-2\"").as_str()
    );
    assert_eq!(fetched.bytes().await.unwrap().as_ref(), second.as_slice());
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn family_members_see_each_other_and_outsiders_do_not() {
    let server = spawn_server().await;
    let (owner, owner_id) = server.register("anna", "Anna").await;
    let (member, _) = server.register("ben", "Ben").await;
    let (stranger, _) = server.register("carol", "Carol").await;
    let (_, invite) = server.create_family(&owner, "The Smiths").await;
    server.set_open_policy(&owner).await;
    server.join(&member, &invite, "joined").await;

    server
        .put_bytes(&owner, "/me/avatar", "image/jpeg", jpeg_bytes(32))
        .await;

    let path = format!("/users/{owner_id}/avatar");
    assert_eq!(server.get(&owner, &path).await.status(), 200);
    assert_eq!(server.get(&member, &path).await.status(), 200);
    // Same 404 an unknown user id gets: the endpoint never reveals which.
    assert_error(server.get(&stranger, &path).await, 404, "user_not_found").await;

    // The roster carries the version so a member knows to fetch at all.
    let mine = server.get(&member, "/families/mine").await;
    let body: serde_json::Value = mine.json().await.unwrap();
    let owner_member = body["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == owner_id)
        .expect("owner is in the roster");
    assert_eq!(owner_member["avatar_version"], 1);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn bad_uploads_are_refused() {
    let server = spawn_server().await;
    let (token, _) = server.register("anna", "Anna").await;

    // Wrong content type entirely.
    assert_error(
        server
            .put_bytes(&token, "/me/avatar", "image/gif", jpeg_bytes(32))
            .await,
        415,
        "invalid_image",
    )
    .await;

    // Right content type, bytes that are not that image.
    assert_error(
        server
            .put_bytes(&token, "/me/avatar", "image/png", jpeg_bytes(32))
            .await,
        400,
        "invalid_image",
    )
    .await;
    assert_error(
        server
            .put_bytes(
                &token,
                "/me/avatar",
                "image/jpeg",
                b"{\"not\":\"an image\"}".to_vec(),
            )
            .await,
        400,
        "invalid_image",
    )
    .await;

    // Over the size ceiling.
    assert_error(
        server
            .put_bytes(&token, "/me/avatar", "image/jpeg", jpeg_bytes(262_145))
            .await,
        413,
        "avatar_too_large",
    )
    .await;

    // Nothing stuck: still no picture.
    let me = server.get(&token, "/me").await;
    let body: serde_json::Value = me.json().await.unwrap();
    assert_eq!(body["user"]["avatar_version"], 0);
}
