//! Integration: the storage keys (#59) — what the server refuses to start
//! with, and what takes a key away.
//!
//! The keys are what makes deleting reach the backups (docs/protocol.md,
//! "Stored encrypted"): a family, a group, a direct or assistant chat, an
//! account each has its own, and each goes with its owner. And a server that
//! cannot open what it holds must say so at start, naming what is wrong,
//! rather than answer every request with a 500.

mod common;

use common::{TestServer, spawn_server};
use family_connect::keyring::{Keyring, Owner};
use family_connect::vault::{Lookup, ServerKey};
use serde_json::{Value, json};

async fn key_exists(ts: &TestServer, key_id: i64) -> bool {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM storage_keys WHERE id = $1)")
        .bind(key_id)
        .fetch_one(&ts.state.pool)
        .await
        .expect("looking for a key")
}

async fn owner_key(ts: &TestServer, owner: Owner) -> i64 {
    ts.state
        .keys
        .key(&ts.state.pool, owner)
        .await
        .expect("the owner's key")
        .id()
}

async fn start(ts: &TestServer, keys: &Keyring) -> anyhow::Result<()> {
    let mut conn = ts.state.pool.acquire().await.expect("a connection");
    keys.start(&mut conn).await
}

/// A server started with another key file opens nothing, so it does not
/// start — and the refusal names the key it wanted and the one it was given.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_wrong_key_file_is_refused_naming_both_keys() {
    let ts = spawn_server().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    ts.create_family(&owner, "The Smiths").await;

    let other = Keyring::new(ServerKey::generate());
    let err = start(&ts, &other)
        .await
        .expect_err("another key file is refused");
    let text = format!("{err:#}");
    assert!(text.contains(ts.state.keys.fingerprint()), "{text}");
    assert!(text.contains(other.fingerprint()), "{text}");
}

/// An owner whose key is gone — a key copy from the wrong day, after a
/// restore — stops the start, with what is affected counted and both ways
/// out named. Nothing is deleted by the refusal.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_owner_without_its_key_stops_the_start() {
    let ts = spawn_server().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (family_id, _) = ts.create_family(&owner, "The Smiths").await;
    let family_key = owner_key(&ts, Owner::Family(family_id)).await;

    sqlx::query("DELETE FROM storage_keys WHERE id = $1")
        .bind(family_key)
        .execute(&ts.state.pool)
        .await
        .expect("losing the family's key");

    let err = start(&ts, &ts.state.keys)
        .await
        .expect_err("a lost key stops the start");
    let text = format!("{err:#}");
    // The family, and its family chat, which shares the family's key.
    assert!(
        text.contains("1 families, 1 conversations and 0 accounts"),
        "{text}"
    );
    assert!(text.contains("forget-missing"), "{text}");
    let families: i64 = sqlx::query_scalar("SELECT count(*) FROM families")
        .fetch_one(&ts.state.pool)
        .await
        .expect("counting families");
    assert_eq!(families, 1, "the refusal deletes nothing");
}

/// A key no owner names — what a key copy newer than the database brings
/// back for a family deleted since — is dropped at start.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_key_nobody_names_is_dropped_at_start() {
    let ts = spawn_server().await;
    let orphan = ts.storage_key().await;
    assert!(key_exists(&ts, orphan.id()).await);
    start(&ts, &ts.state.keys).await.expect("the start goes on");
    assert!(!key_exists(&ts, orphan.id()).await);
}

/// A sealed value somebody altered does not open, and the request answers
/// `internal` — never the bytes, never a guess.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_altered_value_answers_internal() {
    let ts = spawn_server().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    ts.create_family(&owner, "The Smiths").await;
    let chat_id = ts.family_chat_id(&owner).await;
    let sent: Value = ts
        .post(
            &owner,
            &format!("/chats/{chat_id}/messages"),
            json!({"client_msg_id": uuid::Uuid::new_v4(), "body": "Dinner at 7?"}),
        )
        .await
        .json()
        .await
        .expect("JSON");
    let message_id = sent["message"]["id"].as_i64().expect("an id");

    // The last byte is the tag's.
    sqlx::query(
        "UPDATE messages
            SET body = set_byte(body, length(body) - 1, get_byte(body, length(body) - 1) # 1)
          WHERE id = $1",
    )
    .bind(message_id)
    .execute(&ts.state.pool)
    .await
    .expect("altering one byte");

    let response = ts.get(&owner, &format!("/chats/{chat_id}/messages")).await;
    assert_eq!(response.status(), 500);
    let body = response.text().await.expect("a body");
    assert!(!body.contains("Dinner"), "{body}");
    assert!(body.contains("internal"), "{body}");
}

/// Deleting a family takes its key — and, through the cascade to its chats,
/// the keys of its groups and assistant chats — in the same transaction.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_deleted_family_takes_its_keys_with_it() {
    let ts = spawn_server().await;
    let (owner, owner_id) = ts.register("owner", "Olive").await;
    let (family_id, code) = ts.create_family(&owner, "The Smiths").await;
    let (member, member_id) = ts.register("member", "Bob").await;
    ts.join(&member, &code, "pending").await;
    let requests: Value = ts
        .get(&owner, "/families/join-requests")
        .await
        .json()
        .await
        .expect("JSON");
    let request_id = requests["requests"][0]["id"].as_i64().expect("a request");
    assert_eq!(
        ts.post(
            &owner,
            &format!("/families/join-requests/{request_id}/approve"),
            json!({})
        )
        .await
        .status(),
        200
    );
    let group: Value = ts
        .post(
            &owner,
            "/chats/groups",
            json!({"title": "Cousins", "member_ids": [member_id]}),
        )
        .await
        .json()
        .await
        .expect("JSON");
    let group_id = group["chat"]["id"].as_i64().expect("a group");
    let family_key = owner_key(&ts, Owner::Family(family_id)).await;
    let group_key = owner_key(&ts, Owner::Chat(group_id)).await;
    let owner_account_key = owner_key(&ts, Owner::Account(owner_id)).await;

    // Both leave; the last one out takes the family with them.
    for token in [&member, &owner] {
        let status = ts.post(token, "/families/leave", json!({})).await.status();
        assert!(status.is_success(), "{status}");
    }
    assert!(!key_exists(&ts, family_key).await, "the family's key went");
    assert!(
        !key_exists(&ts, group_key).await,
        "its group's key went too"
    );
    assert!(
        key_exists(&ts, owner_account_key).await,
        "accounts keep theirs"
    );
}

/// Deleting an account replaces its key: everything the old one sealed went
/// with the account's rows, and the tombstone's few words are sealed under a
/// new one.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_deleted_account_has_its_key_replaced() {
    let ts = spawn_server().await;
    let (token, user_id) = ts.register("leaving", "Lee").await;
    let old_key = owner_key(&ts, Owner::Account(user_id)).await;
    let response = ts
        .post(&token, "/me/delete", json!({"password": "password123"}))
        .await;
    assert!(response.status().is_success(), "{}", response.status());

    assert!(!key_exists(&ts, old_key).await, "the old key is gone");
    let (key_id, username): (i64, Vec<u8>) =
        sqlx::query_as("SELECT key_id, username FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&ts.state.pool)
            .await
            .expect("the tombstone");
    assert_ne!(key_id, old_key);
    assert!(key_exists(&ts, key_id).await);
    assert_eq!(ts.open_text(&username), format!("deleted-{user_id}"));
}

/// Rotation moves the database to the new key in one go: every storage key
/// unwraps under it and none under the old, and every lookup — usernames,
/// invite codes, device tokens, all HMACs under the server key — is the new
/// key's. What was sealed is untouched and still opens.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn rotation_moves_every_key_and_lookup_to_the_new_server_key() {
    let ts = spawn_server().await;
    let (owner, owner_id) = ts.register("Owner", "Olive").await;
    let (family_id, code) = ts.create_family(&owner, "The Smiths").await;
    ts.post(
        &owner,
        "/devices",
        json!({"platform": "ios", "push_token": "tok-rotate", "voip_token": "voip-rotate"}),
    )
    .await;
    let new = ServerKey::generate();
    let rotated = family_connect::admin::rotate(&ts.state.pool, &ts.server_key, &new)
        .await
        .expect("rotating");
    assert!(rotated.keys >= 3, "{rotated:?}");

    let fresh = Keyring::new(new.clone());
    start(&ts, &fresh)
        .await
        .expect("the new key opens everything");
    assert!(
        start(&ts, &Keyring::new(ts.server_key.clone()))
            .await
            .is_err(),
        "the old key opens nothing any more"
    );
    let (name, username_key): (Vec<u8>, Vec<u8>) = sqlx::query_as(
        "SELECT name, (SELECT username_key FROM users WHERE id = $2) FROM families WHERE id = $1",
    )
    .bind(family_id)
    .bind(owner_id)
    .fetch_one(&ts.state.pool)
    .await
    .expect("reading back");
    assert_eq!(fresh.open_text(&name).expect("opens"), "The Smiths");
    assert_eq!(username_key, new.lookup(Lookup::Username, "owner"));
    let invite_code_key: Vec<u8> =
        sqlx::query_scalar("SELECT invite_code_key FROM families WHERE id = $1")
            .bind(family_id)
            .fetch_one(&ts.state.pool)
            .await
            .expect("reading the code's lookup");
    assert_eq!(invite_code_key, new.lookup(Lookup::InviteCode, &code));
    let (push_key, voip_key): (Vec<u8>, Vec<u8>) =
        sqlx::query_as("SELECT push_token_key, voip_token_key FROM devices WHERE user_id = $1")
            .bind(owner_id)
            .fetch_one(&ts.state.pool)
            .await
            .expect("reading the device's lookups");
    assert_eq!(push_key, new.lookup(Lookup::PushToken, "tok-rotate"));
    assert_eq!(voip_key, new.lookup(Lookup::VoipToken, "voip-rotate"));
}

/// `forget-missing` deletes again what a restore brought back without its
/// key — a family, a conversation, an account — and the start that refused
/// before goes on after.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn forget_missing_deletes_again_what_has_no_key() {
    let ts = spawn_server().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (family_id, code) = ts.create_family(&owner, "The Smiths").await;
    let (member, member_id) = ts.register("member", "Bob").await;
    ts.join(&member, &code, "pending").await;
    let requests: Value = ts
        .get(&owner, "/families/join-requests")
        .await
        .json()
        .await
        .expect("JSON");
    let request_id = requests["requests"][0]["id"].as_i64().expect("a request");
    ts.post(
        &owner,
        &format!("/families/join-requests/{request_id}/approve"),
        json!({}),
    )
    .await;
    let direct: Value = ts
        .post(&owner, "/chats/direct", json!({"user_id": member_id}))
        .await
        .json()
        .await
        .expect("JSON");
    let direct_id = direct["chat"]["id"].as_i64().expect("a direct chat");
    let (leaving, leaving_id) = ts.register("leaving", "Lee").await;
    let _ = leaving;

    // What a key copy from after these were deleted no longer holds.
    for owner in [Owner::Chat(direct_id), Owner::Account(leaving_id)] {
        let key_id = owner_key(&ts, owner).await;
        sqlx::query("DELETE FROM storage_keys WHERE id = $1")
            .bind(key_id)
            .execute(&ts.state.pool)
            .await
            .expect("losing a key");
    }
    assert!(start(&ts, &ts.state.keys).await.is_err());

    let forgotten = family_connect::admin::forget_missing(&ts.state)
        .await
        .expect("forgetting");
    assert_eq!(forgotten.conversations, 1);
    assert_eq!(forgotten.accounts, 1);
    assert_eq!(forgotten.families, 0);
    start(&ts, &ts.state.keys)
        .await
        .expect("the start goes on once nothing is missing");

    let chat_gone: bool =
        sqlx::query_scalar("SELECT NOT EXISTS (SELECT 1 FROM chats WHERE id = $1)")
            .bind(direct_id)
            .fetch_one(&ts.state.pool)
            .await
            .expect("looking for the chat");
    assert!(chat_gone);
    let tombstone: bool =
        sqlx::query_scalar("SELECT deleted_at IS NOT NULL FROM users WHERE id = $1")
            .bind(leaving_id)
            .fetch_one(&ts.state.pool)
            .await
            .expect("looking for the account");
    assert!(tombstone, "the account was scrubbed again");
    let family_alive: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM families WHERE id = $1)")
            .bind(family_id)
            .fetch_one(&ts.state.pool)
            .await
            .expect("looking for the family");
    assert!(family_alive, "what kept its key is untouched");
}

/// Approve every pending request to the caller's home family.
async fn approve_all(ts: &TestServer, owner: &str) {
    let requests: Value = ts
        .get(owner, "/families/join-requests")
        .await
        .json()
        .await
        .expect("JSON");
    for request in requests["requests"].as_array().expect("requests") {
        let id = request["id"].as_i64().expect("an id");
        let status = ts
            .post(
                owner,
                &format!("/families/join-requests/{id}/approve"),
                json!({}),
            )
            .await
            .status();
        assert_eq!(status, 200);
    }
}

async fn send(ts: &TestServer, token: &str, chat_id: i64, body: Value) -> i64 {
    let mut body = body;
    body["client_msg_id"] = json!(uuid::Uuid::new_v4());
    let response = ts
        .post(token, &format!("/chats/{chat_id}/messages"), body)
        .await;
    assert!(response.status().is_success(), "{}", response.status());
    let sent: Value = response.json().await.expect("JSON");
    sent["message"]["id"].as_i64().expect("an id")
}

/// THE PROMISE, end to end: what a backup holds of a deleted family — its
/// name, its chat, its group — opens with no key copy taken after the
/// deletion, even with the key file. What the family's members still share
/// does: a direct chat moved to another family they are both in, and the
/// photo in it that was uploaded for the family that is gone.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn deleting_a_family_reaches_the_backups_and_nothing_else() {
    let ts = spawn_server().await;
    let (olive, _) = ts.register("olive", "Olive").await;
    let (bob, bob_id) = ts.register("bob", "Bob").await;
    let (carol, _) = ts.register("carol", "Carol").await;
    let (smiths, smiths_code) = ts.create_family(&olive, "The Smiths").await;
    ts.join(&bob, &smiths_code, "pending").await;
    approve_all(&ts, &olive).await;
    let (_, browns_code) = ts.create_family(&carol, "The Browns").await;
    ts.join(&olive, &browns_code, "pending").await;
    ts.join(&bob, &browns_code, "pending").await;
    approve_all(&ts, &carol).await;

    let family_chat = ts.family_chat_id(&olive).await;
    let family_message = send(&ts, &olive, family_chat, json!({"body": "Smiths only"})).await;
    let group: Value = ts
        .post(
            &olive,
            "/chats/groups",
            json!({"title": "Smith cousins", "member_ids": [bob_id], "family_id": smiths}),
        )
        .await
        .json()
        .await
        .expect("JSON");
    let group_id = group["chat"]["id"].as_i64().expect("a group");
    let group_message = send(&ts, &olive, group_id, json!({"body": "group words"})).await;
    let direct: Value = ts
        .post(&olive, "/chats/direct", json!({"user_id": bob_id}))
        .await
        .json()
        .await
        .expect("JSON");
    let direct_id = direct["chat"]["id"].as_i64().expect("a direct chat");
    let mut photo = vec![0xFF, 0xD8, 0xFF, 0xE0];
    photo.extend(b"a photo for the Smiths");
    let uploaded: Value = ts
        .put_bytes_method(
            "POST",
            &olive,
            &format!("/attachments?kind=photo&family_id={smiths}"),
            "image/jpeg",
            photo.clone(),
        )
        .await
        .json()
        .await
        .expect("JSON");
    let photo_id = uploaded["attachment"]["id"].as_i64().expect("an id");
    let direct_message = send(
        &ts,
        &olive,
        direct_id,
        json!({"body": "between us", "attachment_ids": [photo_id]}),
    )
    .await;

    // THE BACKUP: the sealed values as a dump holds them, and the photo's
    // file as the attachments copy holds it.
    let sealed = |sql: &'static str, id: i64| {
        let pool = ts.state.pool.clone();
        async move {
            sqlx::query_scalar::<_, Vec<u8>>(sql)
                .bind(id)
                .fetch_one(&pool)
                .await
                .expect("a sealed value")
        }
    };
    let family_name = sealed("SELECT name FROM families WHERE id = $1", smiths).await;
    let family_body = sealed("SELECT body FROM messages WHERE id = $1", family_message).await;
    let group_body = sealed("SELECT body FROM messages WHERE id = $1", group_message).await;
    let direct_body = sealed("SELECT body FROM messages WHERE id = $1", direct_message).await;
    let (storage_key, photo_key): (String, Vec<u8>) =
        sqlx::query_as("SELECT storage_key, file_key FROM attachments WHERE id = $1")
            .bind(photo_id)
            .fetch_one(&ts.state.pool)
            .await
            .expect("the photo's row");
    let photo_file = std::fs::read(ts.state.storage.blob_path(&storage_key)).expect("the file");
    assert!(
        !photo_file.windows(9).any(|window| window == b"the Smith"),
        "the file on disk is sealed"
    );

    // The Smiths go: both leave, the last one out takes the family.
    for token in [&bob, &olive] {
        let response = ts
            .post(token, &format!("/families/{smiths}/leave"), json!({}))
            .await;
        assert!(response.status().is_success(), "{}", response.status());
    }

    // THE KEY COPY, taken after the deletion, with the right key file.
    let copy = Keyring::new(ts.server_key.clone());
    start(&ts, &copy).await.expect("the key copy loads");

    for gone in [&family_name, &family_body, &group_body] {
        assert!(
            copy.open(gone).is_err(),
            "a deleted family's words still open"
        );
    }
    // What the pair still shares opens: the direct chat and its photo.
    assert_eq!(copy.open_text(&direct_body).expect("opens"), "between us");
    let own_key = copy.open(&photo_key).expect("the photo's key opens");
    let own_key =
        family_connect::vault::DataKey::from_bytes(&own_key).expect("a file key is a key");
    let opener = family_connect::vault::FileOpener::new(
        &own_key,
        &photo_file[..family_connect::vault::FILE_HEADER_LEN],
        photo_file.len() as u64,
    )
    .expect("the file's header");
    let (offset, len) = opener.chunk_span(0);
    let plain = opener
        .open_chunk(0, &photo_file[offset as usize..offset as usize + len])
        .expect("the photo opens");
    assert_eq!(plain, photo);
}

/// `family-connect reports`: the assistant reports the operator can no
/// longer read with psql, opened — and one about a conversation that has
/// since been deleted says so instead of failing the list.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_operator_reads_assistant_reports_opened() {
    let ts = spawn_server().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (family_id, _) = ts.create_family(&owner, "The Smiths").await;
    let chat_id = ts.family_chat_id(&owner).await;
    let assistant: i64 = sqlx::query_scalar("SELECT id FROM users WHERE is_assistant")
        .fetch_one(&ts.state.pool)
        .await
        .expect("the assistant");
    let reply: i64 = sqlx::query_scalar(
        "INSERT INTO messages (chat_id, sender_id, client_msg_id, body)
         VALUES ($1, $2, gen_random_uuid(), $3) RETURNING id",
    )
    .bind(chat_id)
    .bind(assistant)
    .bind(ts.seal(Owner::Chat(chat_id), "A made-up fact.").await)
    .fetch_one(&ts.state.pool)
    .await
    .expect("planting a reply");
    let status = ts
        .post(
            &owner,
            "/reports/assistant",
            json!({"message_id": reply, "reason": "other", "note": "it invented a person"}),
        )
        .await
        .status();
    assert_eq!(status, 201);

    let reports = family_connect::admin::assistant_reports(&ts.state.pool, &ts.state.keys, 10)
        .await
        .expect("reading the reports");
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].excerpt.as_deref(), Some("A made-up fact."));
    assert_eq!(reports[0].note.as_deref(), Some("it invented a person"));

    // The family's key gone — as after its deletion, in a backup.
    let family_key = owner_key(&ts, Owner::Family(family_id)).await;
    sqlx::query("DELETE FROM storage_keys WHERE id = $1")
        .bind(family_key)
        .execute(&ts.state.pool)
        .await
        .expect("losing the key");
    let fresh = Keyring::new(ts.server_key.clone());
    fresh
        .load(&mut ts.state.pool.acquire().await.expect("a connection"))
        .await
        .expect("loading what is left");
    let reports = family_connect::admin::assistant_reports(&ts.state.pool, &fresh, 10)
        .await
        .expect("reading the reports");
    assert_eq!(reports[0].excerpt, None, "gone with its conversation");
    assert_eq!(reports[0].reason, "other");
}
