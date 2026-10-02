//! Integration: the family's sticker pack, and a sticker sent in a chat
//! (protocol.md, "Sticker pack").
//!
//! "Sticker" here is the CHAT kind — a small picture sent as its own
//! message — and not a board note, which the rest of this suite also calls
//! one. Two things are under test and they are deliberately separate: the
//! PACK, which is family property with the board's cursor, tombstones and
//! frames; and a sticker MESSAGE, which is an ordinary photo message with
//! one flag and its own copy of the bytes.

mod common;

use std::time::Duration;

use common::{TestServer, assert_error, spawn_server, spawn_server_with_config};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::http::header::AUTHORIZATION;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use uuid::Uuid;

type WsClient = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// How long anything asynchronous is waited for. A DEADLINE that is polled
/// to, never a sleep that is trusted: this suite has been bitten by the
/// second kind on a loaded machine.
const WAIT: Duration = Duration::from_secs(5);

// --- Minimal WebSocket client (mirrors the helpers in board_flow.rs) --------

async fn connect_ws(ts: &TestServer, token: &str) -> WsClient {
    let mut request = ts
        .ws_url
        .as_str()
        .into_client_request()
        .expect("building the ws request");
    request.headers_mut().insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {token}")).expect("header value"),
    );
    let (mut ws, _response) = tokio_tungstenite::connect_async(request)
        .await
        .expect("websocket upgrade succeeds");
    // A pong proves the server-side connection task is registered — later
    // fan-outs cannot race past a connection that already answered a frame.
    ws.send(Message::text(json!({"type": "ping"}).to_string()))
        .await
        .expect("sending ping");
    let pong = next_frame_of_type(&mut ws, "pong").await;
    assert_eq!(pong, json!({"type": "pong"}));
    ws
}

async fn next_frame_of_type(ws: &mut WsClient, wanted: &str) -> Value {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let message = tokio::time::timeout_at(deadline, ws.next())
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for a {wanted:?} frame"))
            .expect("socket closed while waiting for a frame")
            .expect("socket errored while waiting for a frame");
        if let Message::Text(text) = message {
            let value: Value = serde_json::from_str(text.as_str()).expect("frames are JSON");
            if value["type"] == wanted {
                return value;
            }
        }
    }
}

// --- Fixtures ----------------------------------------------------------------

/// A WebP by its magic number — `RIFF`, four bytes of length, `WEBP` — and
/// nothing more, which is all the server ever looks at. `seed` fills the
/// rest, so two stickers in one test are two files: identical bytes are
/// one file per family, and one ITEM per pack.
fn webp_bytes(len: usize, seed: u8) -> Vec<u8> {
    let mut bytes = b"RIFF\x00\x00\x00\x00WEBP".to_vec();
    bytes.resize(len.max(12), seed);
    bytes
}

fn png_bytes(len: usize, seed: u8) -> Vec<u8> {
    let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    bytes.resize(len.max(8), seed);
    bytes
}

fn jpeg_bytes(len: usize, seed: u8) -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE0];
    bytes.resize(len.max(4), seed);
    bytes
}

fn mp4_bytes(len: usize) -> Vec<u8> {
    let mut bytes = vec![0x00, 0x00, 0x00, 0x18, b'f', b't', b'y', b'p'];
    bytes.extend_from_slice(b"isom");
    bytes.resize(len.max(12), 0x00);
    bytes
}

/// Family of two; returns `(owner_token, member_token, family_chat_id)`.
async fn family_of_two(ts: &TestServer) -> (String, String, i64) {
    let (owner, _) = ts.register("owner", "Olive").await;
    let (member, _) = ts.register("junior", "Junior").await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&member, &invite_code, "joined").await;
    let chat_id = ts.family_chat_id(&owner).await;
    (owner, member, chat_id)
}

/// A third member of the family `family_of_two` made.
async fn third_member(ts: &TestServer, owner: &str) -> String {
    let (third, _) = ts.register("auntie", "Auntie").await;
    let invite_code = my_family(ts, owner).await["family"]["invite_code"]
        .as_str()
        .expect("the owner sees the invite code")
        .to_string();
    ts.join(&third, &invite_code, "joined").await;
    third
}

async fn my_family(ts: &TestServer, token: &str) -> Value {
    let response = ts.get(token, "/families/mine").await;
    assert_eq!(response.status(), 200, "reading the family");
    response.json().await.expect("family response is JSON")
}

/// Upload bytes as `kind=photo`; returns the attachment id.
async fn upload(ts: &TestServer, token: &str, mime: &str, bytes: Vec<u8>) -> i64 {
    let response = ts
        .put_bytes_method(
            "POST",
            token,
            "/attachments?kind=photo&width=512&height=512",
            mime,
            bytes,
        )
        .await;
    assert_eq!(response.status(), 201, "uploading a {mime}");
    let body: Value = response.json().await.expect("JSON");
    body["attachment"]["id"].as_i64().expect("attachment id")
}

async fn upload_webp(ts: &TestServer, token: &str, seed: u8) -> i64 {
    upload(ts, token, "image/webp", webp_bytes(256, seed)).await
}

async fn claim(ts: &TestServer, token: &str, attachment_id: i64) -> reqwest::Response {
    ts.post(
        token,
        "/families/mine/pack",
        json!({"attachment_id": attachment_id}),
    )
    .await
}

/// Upload a distinct WebP and add it to the pack; returns the `PackItem`.
async fn add_sticker(ts: &TestServer, token: &str, seed: u8) -> Value {
    let attachment_id = upload_webp(ts, token, seed).await;
    let response = claim(ts, token, attachment_id).await;
    assert_eq!(response.status(), 201, "adding a sticker");
    let body: Value = response.json().await.expect("JSON");
    body["item"].clone()
}

async fn pack(ts: &TestServer, token: &str) -> Value {
    let response = ts.get(token, "/families/mine/pack").await;
    assert_eq!(response.status(), 200, "reading the pack");
    response.json().await.expect("JSON")
}

async fn pack_ids(ts: &TestServer, token: &str) -> Vec<i64> {
    pack(ts, token).await["items"]
        .as_array()
        .expect("items is an array")
        .iter()
        .map(|item| item["id"].as_i64().expect("id"))
        .collect()
}

async fn changes(ts: &TestServer, token: &str, after_seq: i64) -> Vec<Value> {
    let response = ts
        .get(
            token,
            &format!("/families/mine/pack/changes?after_seq={after_seq}"),
        )
        .await;
    assert_eq!(response.status(), 200, "reading the pack's changes");
    let body: Value = response.json().await.expect("JSON");
    body["items"].as_array().expect("items").clone()
}

async fn attachment_rows(ts: &TestServer, attachment_id: i64) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM attachments WHERE id = $1")
        .bind(attachment_id)
        .fetch_one(&ts.state.pool)
        .await
        .expect("counting the attachment row")
}

async fn storage_key(ts: &TestServer, attachment_id: i64) -> String {
    sqlx::query_scalar("SELECT storage_key FROM attachments WHERE id = $1")
        .bind(attachment_id)
        .fetch_one(&ts.state.pool)
        .await
        .expect("the attachment row names a file")
}

/// Send one attachment as a STICKER over REST.
async fn send_sticker(
    ts: &TestServer,
    token: &str,
    chat_id: i64,
    attachment_id: i64,
) -> reqwest::Response {
    ts.post(
        token,
        &format!("/chats/{chat_id}/messages"),
        json!({
            "client_msg_id": Uuid::new_v4().to_string(),
            "body": "",
            "attachment_ids": [attachment_id],
            "sticker": true,
        }),
    )
    .await
}

async fn newest_message(ts: &TestServer, token: &str, chat_id: i64) -> Value {
    let page: Value = ts
        .get(token, &format!("/chats/{chat_id}/messages"))
        .await
        .json()
        .await
        .expect("JSON");
    page["messages"][0].clone()
}

// --- The pack ------------------------------------------------------------------

/// The whole loop in one family: a member adds, everybody reads the item
/// AND its bytes, and a stranger reads neither.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_sticker_is_added_and_the_whole_family_can_read_it() {
    let server = spawn_server().await;
    let (owner, member, _) = family_of_two(&server).await;
    let member_id = server.user_id(&member).await;

    // ANY member may add — it is the member who does here, not the owner.
    let bytes = webp_bytes(300, 0x11);
    let attachment_id = upload(&server, &member, "image/webp", bytes.clone()).await;
    let response = server
        .post(
            &member,
            "/families/mine/pack",
            json!({"attachment_id": attachment_id, "label": "  party cat "}),
        )
        .await;
    assert_eq!(response.status(), 201);
    let body: Value = response.json().await.expect("JSON");
    let item = &body["item"];
    assert_eq!(item["added_by"], member_id);
    assert_eq!(item["label"], "party cat", "the label is trimmed");
    assert_eq!(item["attachment"]["id"], attachment_id);
    assert_eq!(item["attachment"]["kind"], "photo");
    assert_eq!(item["attachment"]["mime"], "image/webp");
    assert_eq!(item["attachment"]["size"], 300);
    assert!(item["pack_seq"].as_i64().expect("pack_seq") > 0);
    assert!(item["created_at"].is_string());
    assert!(
        item.get("deleted").is_none(),
        "a live item is not a tombstone"
    );
    // The flag is a MESSAGE's. A pack item's picture never carries it.
    assert!(
        item["attachment"].get("sticker").is_none(),
        "a pack item's picture is not a sent sticker: {item}"
    );

    // The owner — who did not upload it — reads the item and the bytes.
    let read = pack(&server, &owner).await;
    assert_eq!(read["items"].as_array().expect("items").len(), 1);
    assert_eq!(read["items"][0], *item);
    assert_eq!(read["max_pack_seq"], item["pack_seq"]);
    let fetched = server
        .get(&owner, &format!("/attachments/{attachment_id}"))
        .await;
    assert_eq!(fetched.status(), 200, "the family can fetch a pack picture");
    assert_eq!(
        fetched
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("image/webp")
    );
    assert_eq!(
        fetched.bytes().await.expect("bytes").to_vec(),
        bytes,
        "stored as uploaded, byte for byte"
    );

    // A stranger in another family gets the 404 a missing id gets.
    let (stranger, _) = server.register("stranger", "Stranger").await;
    server.create_family(&stranger, "The Joneses").await;
    assert_error(
        server
            .get(&stranger, &format!("/attachments/{attachment_id}"))
            .await,
        404,
        "attachment_not_found",
    )
    .await;
    assert!(
        pack_ids(&server, &stranger).await.is_empty(),
        "packs do not leak between families"
    );
}

/// PNG is a sticker too: an Apple device can decode a WebP and cannot write
/// one, and a pack only some members could add to is not the family's.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_png_is_a_sticker_too_and_a_label_is_optional() {
    let server = spawn_server().await;
    let (owner, _, _) = family_of_two(&server).await;

    let attachment_id = upload(&server, &owner, "image/png", png_bytes(200, 0x21)).await;
    let response = claim(&server, &owner, attachment_id).await;
    assert_eq!(response.status(), 201);
    let body: Value = response.json().await.expect("JSON");
    assert_eq!(body["item"]["attachment"]["mime"], "image/png");
    assert!(
        body["item"].get("label").is_none(),
        "no label is absent, never null or empty: {body}"
    );

    // An empty label is no label; one over the cap is refused, and the
    // upload it named is still there to claim properly.
    let blank = upload_webp(&server, &owner, 0x22).await;
    let response = server
        .post(
            &owner,
            "/families/mine/pack",
            json!({"attachment_id": blank, "label": "   "}),
        )
        .await;
    assert_eq!(response.status(), 201);
    let body: Value = response.json().await.expect("JSON");
    assert!(body["item"].get("label").is_none());

    let long = upload_webp(&server, &owner, 0x23).await;
    assert_error(
        server
            .post(
                &owner,
                "/families/mine/pack",
                json!({"attachment_id": long, "label": "x".repeat(65)}),
            )
            .await,
        400,
        "validation",
    )
    .await;
    assert_eq!(pack_ids(&server, &owner).await.len(), 2);
    assert_eq!(claim(&server, &owner, long).await.status(), 201);
}

/// WebP is a photo type by its magic number, and only by it.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_webp_is_checked_by_its_magic_number() {
    let server = spawn_server().await;
    let (owner, _, _) = family_of_two(&server).await;

    // RIFF is also what a WAV is. The form type at offset 8 is the check.
    let mut wav = b"RIFF\x00\x00\x00\x00WAVE".to_vec();
    wav.resize(64, 0);
    assert_error(
        server
            .put_bytes_method("POST", &owner, "/attachments?kind=photo", "image/webp", wav)
            .await,
        400,
        "invalid_attachment",
    )
    .await;
    // A JPEG calling itself a WebP is refused the same way.
    assert_error(
        server
            .put_bytes_method(
                "POST",
                &owner,
                "/attachments?kind=photo",
                "image/webp",
                jpeg_bytes(64, 0),
            )
            .await,
        400,
        "invalid_attachment",
    )
    .await;
    // And a WebP is a PHOTO: declared as a video it contradicts its kind.
    assert_error(
        server
            .put_bytes_method(
                "POST",
                &owner,
                "/attachments?kind=video",
                "image/webp",
                webp_bytes(64, 0),
            )
            .await,
        400,
        "invalid_attachment",
    )
    .await;
}

/// `GET /families/mine` carries the pack's cursor — omitted until the pack
/// has ever been written to — and its two ceilings, always.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn families_mine_reports_the_pack_cursor_and_its_limits() {
    let server = spawn_server_with_config(|cfg| cfg.limits.max_pack_items = 7).await;
    let (owner, member, _) = family_of_two(&server).await;

    let before = my_family(&server, &member).await;
    assert!(
        before.get("max_pack_seq").is_none(),
        "an untouched pack reports no cursor: {before}"
    );
    assert_eq!(before["max_pack_items"], 7);
    assert_eq!(
        before["max_pack_item_bytes"],
        server.state.cfg.limits.max_pack_item_bytes
    );
    let empty = pack(&server, &member).await;
    assert_eq!(empty["items"], json!([]));
    assert_eq!(empty["max_pack_seq"], 0);

    let item = add_sticker(&server, &owner, 0x31).await;
    let after = my_family(&server, &member).await;
    assert_eq!(after["max_pack_seq"], item["pack_seq"]);

    // A removal moves the mark as well, and the mark never goes back down:
    // an emptied pack still reports one, so a client still catches up and
    // learns of the tombstone.
    let deleted = server
        .delete(&owner, &format!("/families/mine/pack/{}", item["id"]))
        .await;
    assert_eq!(deleted.status(), 204);
    let emptied = my_family(&server, &member).await;
    assert!(
        emptied["max_pack_seq"].as_i64().expect("still reported")
            > item["pack_seq"].as_i64().expect("seq")
    );
}

/// The catch-up feed: each item once, in the state it is now in, in seq
/// order, with removals as tombstones — and the full read without them.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_change_feed_reports_removals_as_tombstones() {
    let server = spawn_server().await;
    let (owner, member, _) = family_of_two(&server).await;

    let first = add_sticker(&server, &owner, 0x41).await;
    let second = add_sticker(&server, &member, 0x42).await;
    let third = add_sticker(&server, &owner, 0x43).await;
    let mark = third["pack_seq"].as_i64().expect("seq");

    // The full read is in the order they were added, whoever added them.
    assert_eq!(
        pack_ids(&server, &member).await,
        vec![
            first["id"].as_i64().expect("id"),
            second["id"].as_i64().expect("id"),
            third["id"].as_i64().expect("id"),
        ]
    );

    let first_id = first["id"].as_i64().expect("id");
    let removed = server
        .delete(&owner, &format!("/families/mine/pack/{first_id}"))
        .await;
    assert_eq!(removed.status(), 204);

    // A client that was away across the removal asks for what came after
    // its cursor and gets exactly the tombstone.
    let missed = changes(&server, &member, mark).await;
    assert_eq!(missed.len(), 1, "one change since the mark: {missed:?}");
    let tombstone = &missed[0];
    assert_eq!(tombstone["id"], first_id);
    assert_eq!(tombstone["deleted"], true);
    assert!(tombstone["pack_seq"].as_i64().expect("seq") > mark);
    for gone in ["added_by", "label", "attachment", "created_at"] {
        assert!(
            tombstone.get(gone).is_none(),
            "a tombstone carries no {gone}: {tombstone}"
        );
    }

    // From zero the feed is the whole history collapsed: three items, the
    // removed one LAST because its seq is now the newest, in seq order.
    let all = changes(&server, &member, 0).await;
    let seqs: Vec<i64> = all
        .iter()
        .map(|item| item["pack_seq"].as_i64().expect("seq"))
        .collect();
    let mut sorted = seqs.clone();
    sorted.sort_unstable();
    assert_eq!(seqs, sorted, "ascending by pack_seq");
    assert_eq!(all.len(), 3);
    assert_eq!(all[2]["id"], first_id);
    assert_eq!(all[0]["attachment"]["id"], second["attachment"]["id"]);

    // The full read never returns a tombstone, and its mark is past it.
    let read = pack(&server, &member).await;
    assert_eq!(read["items"].as_array().expect("items").len(), 2);
    assert_eq!(read["max_pack_seq"], tombstone["pack_seq"]);

    // Paged, like every feed: a short page ends the loop.
    let response = server
        .get(&member, "/families/mine/pack/changes?after_seq=0&limit=2")
        .await;
    let page: Value = response.json().await.expect("JSON");
    assert_eq!(page["items"].as_array().expect("items").len(), 2);
    assert_error(
        server
            .get(&member, "/families/mine/pack/changes?after_seq=nope")
            .await,
        400,
        "invalid_pagination",
    )
    .await;
}

/// Whoever added it, or the family owner — and nobody else. The shape the
/// board does not have.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_author_or_the_owner_may_remove_and_nobody_else() {
    let server = spawn_server().await;
    let (owner, member, _) = family_of_two(&server).await;
    let third = third_member(&server, &owner).await;

    let mine = add_sticker(&server, &member, 0x51).await;
    let mine_id = mine["id"].as_i64().expect("id");
    let picture_id = mine["attachment"]["id"].as_i64().expect("id");
    let theirs = add_sticker(&server, &member, 0x52).await;
    let theirs_id = theirs["id"].as_i64().expect("id");
    let owners = add_sticker(&server, &owner, 0x53).await;
    let owners_id = owners["id"].as_i64().expect("id");

    // A third member is neither the author nor the owner.
    assert_error(
        server
            .delete(&third, &format!("/families/mine/pack/{mine_id}"))
            .await,
        403,
        "not_pack_item_author",
    )
    .await;
    // And a member may not remove the OWNER's sticker: the owner's extra
    // power runs one way.
    assert_error(
        server
            .delete(&member, &format!("/families/mine/pack/{owners_id}"))
            .await,
        403,
        "not_pack_item_author",
    )
    .await;
    assert_eq!(pack_ids(&server, &owner).await.len(), 3, "nothing went");

    // The author removes their own; the picture goes with it.
    let removed = server
        .delete(&member, &format!("/families/mine/pack/{mine_id}"))
        .await;
    assert_eq!(removed.status(), 204);
    assert_eq!(attachment_rows(&server, picture_id).await, 0);
    assert_error(
        server
            .get(&owner, &format!("/attachments/{picture_id}"))
            .await,
        404,
        "attachment_not_found",
    )
    .await;

    // Idempotent: a second removal is still 204 and burns no seq.
    let mark = pack(&server, &owner).await["max_pack_seq"]
        .as_i64()
        .expect("mark");
    let again = server
        .delete(&member, &format!("/families/mine/pack/{mine_id}"))
        .await;
    assert_eq!(again.status(), 204);
    assert_eq!(pack(&server, &owner).await["max_pack_seq"], mark);

    // The OWNER removes a member's sticker.
    let removed = server
        .delete(&owner, &format!("/families/mine/pack/{theirs_id}"))
        .await;
    assert_eq!(removed.status(), 204);
    assert_eq!(pack_ids(&server, &third).await, vec![owners_id]);

    // No such item, and another family's item, are one answer.
    assert_error(
        server.delete(&owner, "/families/mine/pack/999999").await,
        404,
        "pack_item_not_found",
    )
    .await;
    let (stranger, _) = server.register("stranger", "Stranger").await;
    server.create_family(&stranger, "The Joneses").await;
    assert_error(
        server
            .delete(&stranger, &format!("/families/mine/pack/{owners_id}"))
            .await,
        404,
        "pack_item_not_found",
    )
    .await;
    // Somebody with no family at all has no pack.
    let (loner, _) = server.register("loner", "Loner").await;
    assert_error(
        server.get(&loner, "/families/mine/pack").await,
        409,
        "not_in_family",
    )
    .await;
    assert_error(
        server
            .delete(&loner, &format!("/families/mine/pack/{owners_id}"))
            .await,
        409,
        "not_in_family",
    )
    .await;
}

/// The ceiling counts LIVE items, refuses the next one out loud, and gives
/// the slot back when one is removed.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_full_pack_refuses_the_next_sticker() {
    let server = spawn_server_with_config(|cfg| cfg.limits.max_pack_items = 2).await;
    let (owner, member, _) = family_of_two(&server).await;

    let first = add_sticker(&server, &owner, 0x61).await;
    add_sticker(&server, &member, 0x62).await;

    let extra = upload_webp(&server, &member, 0x63).await;
    assert_error(claim(&server, &member, extra).await, 409, "pack_full").await;
    assert_eq!(pack_ids(&server, &owner).await.len(), 2);
    // The refused upload was not consumed: it is still the member's to use.
    assert_eq!(attachment_rows(&server, extra).await, 1);

    // Adding what the pack ALREADY holds is not a new item, so a full pack
    // answers it with the item rather than with `pack_full`.
    let same_again = upload_webp(&server, &member, 0x61).await;
    let response = claim(&server, &member, same_again).await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.expect("JSON");
    assert_eq!(body["item"]["id"], first["id"]);

    // A removal frees the slot.
    let removed = server
        .delete(&owner, &format!("/families/mine/pack/{}", first["id"]))
        .await;
    assert_eq!(removed.status(), 204);
    assert_eq!(claim(&server, &member, extra).await.status(), 201);
    assert_eq!(pack_ids(&server, &owner).await.len(), 2);
}

/// The byte ceiling is checked at the CLAIM: the upload was accepted under
/// the attachment ceiling and did not know what it was for.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_sticker_over_the_byte_ceiling_is_refused_at_the_claim() {
    let server = spawn_server().await;
    let (owner, _, _) = family_of_two(&server).await;
    let ceiling = server.state.cfg.limits.max_pack_item_bytes;
    assert!(
        ceiling < server.state.cfg.limits.max_attachment_bytes,
        "the test needs room between the two ceilings"
    );

    // One byte over: a perfectly good PHOTO, and not a sticker.
    let over = upload(&server, &owner, "image/webp", webp_bytes(ceiling + 1, 0x71)).await;
    assert_error(
        claim(&server, &owner, over).await,
        413,
        "pack_item_too_large",
    )
    .await;
    assert!(pack_ids(&server, &owner).await.is_empty());

    // Exactly at it is fine.
    let at = upload(&server, &owner, "image/webp", webp_bytes(ceiling, 0x72)).await;
    assert_eq!(claim(&server, &owner, at).await.status(), 201);
}

/// Everything else that is wrong with a claim, each with its own answer.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_ways_of_adding_a_sticker_wrong() {
    let server = spawn_server().await;
    let (owner, member, chat_id) = family_of_two(&server).await;

    // A JPEG is a photo and not a sticker: no transparency, one frame.
    let jpeg = upload(&server, &owner, "image/jpeg", jpeg_bytes(128, 0x81)).await;
    assert_error(
        claim(&server, &owner, jpeg).await,
        400,
        "invalid_attachment",
    )
    .await;

    // A video is not even a photo.
    let response = server
        .put_bytes_method(
            "POST",
            &owner,
            "/attachments?kind=video",
            "video/mp4",
            mp4_bytes(128),
        )
        .await;
    assert_eq!(response.status(), 201);
    let video: Value = response.json().await.expect("JSON");
    let video_id = video["attachment"]["id"].as_i64().expect("id");
    assert_error(
        claim(&server, &owner, video_id).await,
        400,
        "invalid_attachment",
    )
    .await;

    // Somebody else's upload is the same 404 as one that never existed.
    let theirs = upload_webp(&server, &member, 0x82).await;
    assert_error(
        claim(&server, &owner, theirs).await,
        404,
        "attachment_not_found",
    )
    .await;
    assert_error(
        claim(&server, &owner, 999_999).await,
        404,
        "attachment_not_found",
    )
    .await;
    // …and it is still the member's own to add.
    assert_eq!(claim(&server, &member, theirs).await.status(), 201);

    // An upload a MESSAGE already carries is taken.
    let sent = upload_webp(&server, &owner, 0x83).await;
    let response = server
        .post(
            &owner,
            &format!("/chats/{chat_id}/messages"),
            json!({
                "client_msg_id": Uuid::new_v4().to_string(),
                "body": "",
                "attachment_ids": [sent],
            }),
        )
        .await;
    assert_eq!(response.status(), 201);
    assert_error(
        claim(&server, &owner, sent).await,
        409,
        "attachment_already_used",
    )
    .await;

    // So is one pinned to the board.
    let pinned = upload(&server, &owner, "image/png", png_bytes(128, 0x84)).await;
    let response = server
        .post(
            &owner,
            "/families/mine/board/notes",
            json!({"text": "", "color": "yellow", "x": 0.5, "y": 0.5,
                   "kind": "photo", "attachment_id": pinned}),
        )
        .await;
    assert_eq!(response.status(), 201);
    assert_error(
        claim(&server, &owner, pinned).await,
        409,
        "attachment_already_used",
    )
    .await;

    // And the other way round: a pack picture is as taken as either. A
    // message naming it is refused, and so is a board note.
    let item = add_sticker(&server, &owner, 0x85).await;
    let picture = item["attachment"]["id"].as_i64().expect("id");
    assert_error(
        server
            .post(
                &owner,
                &format!("/chats/{chat_id}/messages"),
                json!({
                    "client_msg_id": Uuid::new_v4().to_string(),
                    "body": "",
                    "attachment_ids": [picture],
                }),
            )
            .await,
        409,
        "attachment_already_used",
    )
    .await;
    assert_error(
        server
            .post(
                &owner,
                "/families/mine/board/notes",
                json!({"text": "", "color": "yellow", "x": 0.5, "y": 0.5,
                       "kind": "photo", "attachment_id": picture}),
            )
            .await,
        409,
        "attachment_already_used",
    )
    .await;

    // An upload the unclaimed sweep took says "upload it again".
    let stale = upload_webp(&server, &owner, 0x86).await;
    sqlx::query("UPDATE attachments SET created_at = now() - interval '30 days' WHERE id = $1")
        .bind(stale)
        .execute(&server.state.pool)
        .await
        .expect("ageing the upload");
    family_connect::handlers_attachment::sweep_unclaimed(&server.state)
        .await
        .expect("the sweep runs");
    assert_error(
        claim(&server, &owner, stale).await,
        404,
        "attachment_expired",
    )
    .await;

    // Only the two good ones are in the pack after all that.
    assert_eq!(pack_ids(&server, &member).await.len(), 2);

    // Somebody with no family cannot add anything.
    let (loner, _) = server.register("loner", "Loner").await;
    assert_error(claim(&server, &loner, picture).await, 409, "not_in_family").await;
}

/// Adding what the pack already holds answers with the item that is there:
/// the same upload claimed twice (a retry), and the same BYTES uploaded
/// again (two members who both liked it).
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn adding_the_same_sticker_twice_is_one_item() {
    let server = spawn_server().await;
    let (owner, member, _) = family_of_two(&server).await;
    let mut member_ws = connect_ws(&server, &member).await;

    let attachment_id = upload_webp(&server, &owner, 0x91).await;
    let first = claim(&server, &owner, attachment_id).await;
    assert_eq!(first.status(), 201);
    let first: Value = first.json().await.expect("JSON");
    let item = first["item"].clone();
    next_frame_of_type(&mut member_ws, "pack_item").await;
    let mark = pack(&server, &owner).await["max_pack_seq"].clone();

    // The retry of a claim whose answer was lost.
    let retry = claim(&server, &owner, attachment_id).await;
    assert_eq!(retry.status(), 200, "the same claim again is not a refusal");
    let retry: Value = retry.json().await.expect("JSON");
    assert_eq!(retry["item"], item);

    // The member uploads the very same bytes and adds them.
    let copy = upload_webp(&server, &member, 0x91).await;
    assert_ne!(copy, attachment_id);
    let second = claim(&server, &member, copy).await;
    assert_eq!(second.status(), 200);
    let second: Value = second.json().await.expect("JSON");
    assert_eq!(
        second["item"], item,
        "the item that was already there, picture id and adder included"
    );
    // The fresh upload had nothing left to do and is gone — but the FILE
    // it shared with the item is not.
    assert_eq!(attachment_rows(&server, copy).await, 0);
    assert_eq!(
        server
            .get(&member, &format!("/attachments/{attachment_id}"))
            .await
            .status(),
        200
    );

    // One item, no new seq — and no frame, which is proved by the next
    // frame the member sees being the one for a genuinely new sticker.
    assert_eq!(pack_ids(&server, &member).await.len(), 1);
    assert_eq!(pack(&server, &owner).await["max_pack_seq"], mark);
    let fresh = add_sticker(&server, &owner, 0x92).await;
    let frame = next_frame_of_type(&mut member_ws, "pack_item").await;
    assert_eq!(frame["item"]["id"], fresh["id"]);

    // Once REMOVED, the same bytes are a new item again.
    let removed = server
        .delete(&owner, &format!("/families/mine/pack/{}", item["id"]))
        .await;
    assert_eq!(removed.status(), 204);
    let back = upload_webp(&server, &member, 0x91).await;
    let readded = claim(&server, &member, back).await;
    assert_eq!(readded.status(), 201);
    let readded: Value = readded.json().await.expect("JSON");
    assert_ne!(
        readded["item"]["id"], item["id"],
        "item ids are never reused"
    );
}

/// The frames: an add and a removal each reach EVERY member, the actor's
/// own connection included, carrying the item in the state it now has.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn pack_frames_reach_every_member() {
    let server = spawn_server().await;
    let (owner, member, _) = family_of_two(&server).await;
    let third = third_member(&server, &owner).await;
    let owner_id = server.user_id(&owner).await;

    // The third member has BLOCKED the owner. It changes nothing here: a
    // pack item is the family's picture, not something a person said.
    let blocked = server
        .put(
            &third,
            &format!("/families/members/{owner_id}/block"),
            json!({}),
        )
        .await;
    assert_eq!(blocked.status(), 204);

    // A stranger's socket must stay silent throughout.
    let (stranger, _) = server.register("stranger", "Stranger").await;
    server.create_family(&stranger, "The Joneses").await;

    let mut owner_ws = connect_ws(&server, &owner).await;
    let mut member_ws = connect_ws(&server, &member).await;
    let mut third_ws = connect_ws(&server, &third).await;
    let mut stranger_ws = connect_ws(&server, &stranger).await;

    let item = add_sticker(&server, &owner, 0xA1).await;
    for ws in [&mut owner_ws, &mut member_ws, &mut third_ws] {
        let frame = next_frame_of_type(ws, "pack_item").await;
        assert_eq!(frame, json!({"type": "pack_item", "item": item}));
    }

    let removed = server
        .delete(&owner, &format!("/families/mine/pack/{}", item["id"]))
        .await;
    assert_eq!(removed.status(), 204);
    for ws in [&mut owner_ws, &mut member_ws, &mut third_ws] {
        let frame = next_frame_of_type(ws, "pack_item").await;
        assert_eq!(frame["item"]["id"], item["id"]);
        assert_eq!(frame["item"]["deleted"], true);
        assert!(
            frame["item"]["pack_seq"].as_i64().expect("seq")
                > item["pack_seq"].as_i64().expect("seq")
        );
        assert!(frame["item"].get("attachment").is_none());
    }

    // The stranger saw neither. A ping's pong is the fence: frames are
    // ordered on a socket, so anything fanned out before it would have
    // arrived before it.
    stranger_ws
        .send(Message::text(json!({"type": "ping"}).to_string()))
        .await
        .expect("sending ping");
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let message = tokio::time::timeout_at(deadline, stranger_ws.next())
            .await
            .expect("the pong arrives")
            .expect("socket open")
            .expect("socket healthy");
        if let Message::Text(text) = message {
            let value: Value = serde_json::from_str(text.as_str()).expect("JSON");
            assert_ne!(
                value["type"], "pack_item",
                "another family's pack reached a stranger: {value}"
            );
            if value["type"] == "pong" {
                break;
            }
        }
    }

    // Nothing about the pack notifies anybody.
    assert!(
        server.push.calls().is_empty(),
        "a pack change raised a push: {:?}",
        server.push.calls()
    );
}

/// The sweeper's predicate is the trap, a third time: a pack picture has
/// no `message_id` and no `note_id` and is NOT unclaimed.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_unclaimed_sweep_leaves_a_pack_picture_alone() {
    let server = spawn_server().await;
    let (owner, member, _) = family_of_two(&server).await;

    let item = add_sticker(&server, &owner, 0xB1).await;
    let picture = item["attachment"]["id"].as_i64().expect("id");
    let loose = upload_webp(&server, &owner, 0xB2).await;
    let loose_path = server
        .state
        .storage
        .blob_path(&storage_key(&server, loose).await);
    let picture_path = server
        .state
        .storage
        .blob_path(&storage_key(&server, picture).await);

    // Age both uploads past the grace period.
    sqlx::query("UPDATE attachments SET created_at = now() - interval '30 days'")
        .execute(&server.state.pool)
        .await
        .expect("ageing the uploads");

    let swept = family_connect::handlers_attachment::sweep_unclaimed(&server.state)
        .await
        .expect("the sweep runs");
    assert_eq!(swept, 1, "the loose upload goes and the pack's stays");

    assert_eq!(attachment_rows(&server, picture).await, 1);
    assert!(picture_path.exists(), "the pack's file is still on disk");
    assert_eq!(attachment_rows(&server, loose).await, 0);
    assert!(!loose_path.exists(), "the loose upload's file went with it");
    // And the family can still fetch it.
    assert_eq!(
        server
            .get(&member, &format!("/attachments/{picture}"))
            .await
            .status(),
        200
    );
    assert_eq!(pack_ids(&server, &member).await.len(), 1);
}

// --- A sticker in a chat -------------------------------------------------------

/// The round trip: a message sent with `sticker: true` carries the flag on
/// its attachment on every read, and an ordinary photo — the same WebP —
/// never carries it at all.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_sticker_message_carries_the_flag_and_a_photo_does_not() {
    let server = spawn_server().await;
    let (owner, member, chat_id) = family_of_two(&server).await;
    let mut member_ws = connect_ws(&server, &member).await;

    // The pack holds it; the SEND is a copy with its own upload.
    let item = add_sticker(&server, &owner, 0xC1).await;
    let copy = upload_webp(&server, &owner, 0xC1).await;
    assert_ne!(copy, item["attachment"]["id"].as_i64().expect("id"));

    let response = send_sticker(&server, &owner, chat_id, copy).await;
    assert_eq!(response.status(), 201);
    let sent: Value = response.json().await.expect("JSON");
    let message = &sent["message"];
    assert_eq!(message["body"], "");
    assert_eq!(message["attachments"].as_array().expect("list").len(), 1);
    assert_eq!(message["attachments"][0]["id"], copy);
    assert_eq!(message["attachments"][0]["kind"], "photo");
    assert_eq!(message["attachments"][0]["mime"], "image/webp");
    assert_eq!(message["attachments"][0]["sticker"], true);
    assert_eq!(
        message["attachment"], message["attachments"][0],
        "the legacy field carries it too"
    );

    // The frame every other member gets.
    let frame = next_frame_of_type(&mut member_ws, "message").await;
    assert_eq!(frame["message"]["attachments"][0]["sticker"], true);

    // A page of history, read by the member.
    let read = newest_message(&server, &member, chat_id).await;
    assert_eq!(read["id"], message["id"]);
    assert_eq!(read["attachments"][0]["sticker"], true);
    assert_eq!(read["attachment"]["sticker"], true);

    // The chat-list preview, so a row can say "Sticker".
    let chats: Value = server
        .get(&member, "/chats")
        .await
        .json()
        .await
        .expect("JSON");
    let entry = chats["chats"]
        .as_array()
        .expect("chats")
        .iter()
        .find(|entry| entry["chat"]["id"] == chat_id)
        .expect("the family chat");
    assert_eq!(entry["last_message"]["attachments"][0]["sticker"], true);
    assert_eq!(entry["last_message"]["attachment"]["sticker"], true);

    // The member can fetch the bytes, as for any message's attachment.
    assert_eq!(
        server
            .get(&member, &format!("/attachments/{copy}"))
            .await
            .status(),
        200
    );

    // A retry of the same send is the same message, flag and all.
    let client_msg_id = message["client_msg_id"].as_str().expect("id");
    let retry = server
        .post(
            &owner,
            &format!("/chats/{chat_id}/messages"),
            json!({"client_msg_id": client_msg_id, "body": "",
                   "attachment_ids": [copy], "sticker": true}),
        )
        .await;
    assert_eq!(retry.status(), 200);
    let retry: Value = retry.json().await.expect("JSON");
    assert_eq!(retry["message"]["id"], message["id"]);
    assert_eq!(retry["message"]["attachments"][0]["sticker"], true);

    // THE SAME WebP AS AN ORDINARY PHOTO: no flag, and not `false` either —
    // absent, by the wire's usual rule. `sticker: false` is the same
    // statement as leaving it out.
    for body in [
        json!({"body": ""}),
        json!({"body": "", "sticker": false}),
        json!({"body": "look at this"}),
    ] {
        let photo = upload_webp(&server, &owner, 0xC1).await;
        let mut request = body.clone();
        request["client_msg_id"] = json!(Uuid::new_v4().to_string());
        request["attachment_ids"] = json!([photo]);
        let response = server
            .post(&owner, &format!("/chats/{chat_id}/messages"), request)
            .await;
        assert_eq!(response.status(), 201);
        let sent: Value = response.json().await.expect("JSON");
        assert!(
            sent["message"]["attachments"][0].get("sticker").is_none(),
            "an ordinary photo carries no sticker key: {sent}"
        );
        assert!(sent["message"]["attachment"].get("sticker").is_none());
        let frame = next_frame_of_type(&mut member_ws, "message").await;
        assert!(frame["message"]["attachments"][0].get("sticker").is_none());
        let read = newest_message(&server, &member, chat_id).await;
        assert!(read["attachments"][0].get("sticker").is_none());
    }
    let chats: Value = server
        .get(&member, "/chats")
        .await
        .json()
        .await
        .expect("JSON");
    let entry = chats["chats"]
        .as_array()
        .expect("chats")
        .iter()
        .find(|entry| entry["chat"]["id"] == chat_id)
        .expect("the family chat");
    assert!(
        entry["last_message"]["attachments"][0]
            .get("sticker")
            .is_none()
    );
}

/// The socket sends one the same way: `sticker: true` on the `send` frame,
/// and the ack carries the flag back.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_sticker_is_sent_over_the_socket_too() {
    let server = spawn_server().await;
    let (owner, member, chat_id) = family_of_two(&server).await;
    let mut owner_ws = connect_ws(&server, &owner).await;
    let mut member_ws = connect_ws(&server, &member).await;

    let picture = upload(&server, &owner, "image/png", png_bytes(128, 0xC5)).await;
    let client_msg_id = Uuid::new_v4().to_string();
    owner_ws
        .send(Message::text(
            json!({"type": "send", "chat_id": chat_id, "client_msg_id": client_msg_id,
                   "body": "", "attachment_ids": [picture], "sticker": true})
            .to_string(),
        ))
        .await
        .expect("sending");
    let ack = next_frame_of_type(&mut owner_ws, "ack").await;
    assert_eq!(ack["client_msg_id"], client_msg_id);
    assert_eq!(ack["message"]["attachments"][0]["sticker"], true);
    assert_eq!(ack["message"]["attachments"][0]["mime"], "image/png");
    let frame = next_frame_of_type(&mut member_ws, "message").await;
    assert_eq!(frame["message"]["attachments"][0]["sticker"], true);

    // A refusal comes back as an error frame naming the send, and nothing
    // lands: a sticker with words is not a sticker.
    let worded = upload_webp(&server, &owner, 0xC6).await;
    let refused_id = Uuid::new_v4().to_string();
    owner_ws
        .send(Message::text(
            json!({"type": "send", "chat_id": chat_id, "client_msg_id": refused_id,
                   "body": "hello", "attachment_ids": [worded], "sticker": true})
            .to_string(),
        ))
        .await
        .expect("sending");
    let error = next_frame_of_type(&mut owner_ws, "error").await;
    assert_eq!(error["code"], "validation");
    assert_eq!(error["client_msg_id"], refused_id);
    assert_eq!(
        newest_message(&server, &member, chat_id).await["id"],
        ack["message"]["id"],
        "the refused send left no message"
    );
}

/// What a sticker message may not be — and that a refusal leaves nothing
/// behind: no message, and an upload still free and still unflagged.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_ways_of_sending_a_sticker_wrong() {
    let server = spawn_server().await;
    let (owner, _, chat_id) = family_of_two(&server).await;
    let path = format!("/chats/{chat_id}/messages");
    let send = |body: Value| {
        let mut request = body;
        request["client_msg_id"] = json!(Uuid::new_v4().to_string());
        server.post(&owner, &path, request)
    };

    let good = upload_webp(&server, &owner, 0xD1).await;
    let other = upload_webp(&server, &owner, 0xD2).await;

    // Words beside it.
    assert_error(
        send(json!({"body": "hi", "attachment_ids": [good], "sticker": true})).await,
        400,
        "validation",
    )
    .await;
    // No picture at all, and two of them.
    assert_error(
        send(json!({"body": "", "sticker": true})).await,
        400,
        "invalid_attachment",
    )
    .await;
    assert_error(
        send(json!({"body": "", "attachment_ids": [good, other], "sticker": true})).await,
        400,
        "invalid_attachment",
    )
    .await;
    // A poll is not a sticker, with or without a picture named.
    assert_error(
        send(json!({"body": "Pizza?", "poll": {"options": ["Yes", "No"]}, "sticker": true})).await,
        400,
        "invalid_poll",
    )
    .await;

    // The wrong kind of picture: a JPEG, and a video.
    let jpeg = upload(&server, &owner, "image/jpeg", jpeg_bytes(128, 0xD3)).await;
    assert_error(
        send_sticker(&server, &owner, chat_id, jpeg).await,
        400,
        "invalid_attachment",
    )
    .await;
    let response = server
        .put_bytes_method(
            "POST",
            &owner,
            "/attachments?kind=video",
            "video/mp4",
            mp4_bytes(128),
        )
        .await;
    let video: Value = response.json().await.expect("JSON");
    let video = video["attachment"]["id"].as_i64().expect("id");
    assert_error(
        send_sticker(&server, &owner, chat_id, video).await,
        400,
        "invalid_attachment",
    )
    .await;

    // Over the per-item ceiling: a fine photo, not a sticker.
    let ceiling = server.state.cfg.limits.max_pack_item_bytes;
    let big = upload(&server, &owner, "image/webp", webp_bytes(ceiling + 1, 0xD4)).await;
    assert_error(
        send_sticker(&server, &owner, chat_id, big).await,
        400,
        "invalid_attachment",
    )
    .await;

    // NOTHING LANDED. The chat is empty, and every refused upload is still
    // unclaimed and unflagged — the refusal rolled the claim back.
    let page: Value = server.get(&owner, &path).await.json().await.expect("JSON");
    assert_eq!(page["messages"], json!([]));
    let touched: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM attachments WHERE message_id IS NOT NULL OR sticker",
    )
    .fetch_one(&server.state.pool)
    .await
    .expect("counting");
    assert_eq!(touched, 0, "a refused sticker claimed or flagged something");

    // So each is still good for what it IS: the JPEG as a photo, the WebP
    // as a sticker, the big one as an ordinary photo.
    assert_eq!(
        send(json!({"body": "", "attachment_ids": [jpeg]}))
            .await
            .status(),
        201
    );
    assert_eq!(
        send_sticker(&server, &owner, chat_id, good).await.status(),
        201
    );
    assert_eq!(
        send(json!({"body": "", "attachment_ids": [big]}))
            .await
            .status(),
        201
    );
}

/// The pack is not consulted by a send: the message is a COPY, so a
/// sticker the pack never held — or no longer holds — still goes, and a
/// sticker is sent in a direct chat and as a reply like any message.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_sticker_needs_no_pack_item_and_is_a_message_like_any_other() {
    let server = spawn_server().await;
    let (owner, member, chat_id) = family_of_two(&server).await;
    let member_id = server.user_id(&member).await;

    // Never in the pack at all.
    let loose = upload(&server, &owner, "image/png", png_bytes(128, 0xE1)).await;
    let response = send_sticker(&server, &owner, chat_id, loose).await;
    assert_eq!(response.status(), 201);
    let first: Value = response.json().await.expect("JSON");
    let first_id = first["message"]["id"].as_i64().expect("id");
    assert!(pack_ids(&server, &owner).await.is_empty());

    // As a REPLY, which is how one answers something with a sticker.
    let reply = upload_webp(&server, &member, 0xE2).await;
    let response = server
        .post(
            &member,
            &format!("/chats/{chat_id}/messages"),
            json!({"client_msg_id": Uuid::new_v4().to_string(), "body": "",
                   "attachment_ids": [reply], "sticker": true,
                   "reply_to_message_id": first_id}),
        )
        .await;
    assert_eq!(response.status(), 201);
    let reply: Value = response.json().await.expect("JSON");
    assert_eq!(reply["message"]["reply_to"]["message_id"], first_id);
    assert_eq!(reply["message"]["thread_root_id"], first_id);
    assert_eq!(reply["message"]["attachments"][0]["sticker"], true);
    let reply_id = reply["message"]["id"].as_i64().expect("id");

    // It takes a reaction like any message.
    let reacted = server
        .put(
            &owner,
            &format!("/chats/{chat_id}/messages/{reply_id}/reaction"),
            json!({"emoji": "❤️"}),
        )
        .await;
    assert_eq!(reacted.status(), 200);
    // And a report names it, with what it carried.
    let report = server
        .post(
            &owner,
            "/families/reports",
            json!({"reported_user_id": member_id, "reason": "other", "message_id": reply_id}),
        )
        .await;
    assert_eq!(report.status(), 201);
    let report: Value = report.json().await.expect("JSON");
    assert_eq!(report["report"]["message_attachments"][0]["kind"], "photo");

    // In a DIRECT chat.
    let direct = server
        .post(&owner, "/chats/direct", json!({"user_id": member_id}))
        .await;
    assert_eq!(direct.status(), 200);
    let direct: Value = direct.json().await.expect("JSON");
    let direct_id = direct["chat"]["id"].as_i64().expect("id");
    let picture = upload_webp(&server, &owner, 0xE3).await;
    let response = send_sticker(&server, &owner, direct_id, picture).await;
    assert_eq!(response.status(), 201);
    let read = newest_message(&server, &member, direct_id).await;
    assert_eq!(read["attachments"][0]["sticker"], true);
}

/// A push for a sticker says "Sticker", where a photo says "Photo".
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_sticker_pushes_the_word_sticker() {
    let server = spawn_server().await;
    let (owner, member, chat_id) = family_of_two(&server).await;
    let registered = server
        .post(
            &member,
            "/devices",
            json!({"platform": "ios", "push_token": "member-phone"}),
        )
        .await;
    assert_eq!(registered.status(), 201);

    let picture = upload_webp(&server, &owner, 0xF1).await;
    assert_eq!(
        send_sticker(&server, &owner, chat_id, picture)
            .await
            .status(),
        201
    );

    // Dispatch is spawned fire-and-forget, so a bare read races it: poll
    // to a deadline.
    let deadline = tokio::time::Instant::now() + WAIT;
    let calls = loop {
        let calls = server.push.calls();
        if !calls.is_empty() {
            break calls;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the sticker message raised no push"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    };
    assert_eq!(calls[0].note.body, "Sticker");
}

/// Retention sweeps the MESSAGE and leaves the PACK: the item, its row and
/// its file all stay, and the other way round — removing the item leaves a
/// sent sticker drawing.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn retention_takes_the_sticker_message_and_leaves_the_pack_item() {
    let server = spawn_server().await;
    let (owner, member, chat_id) = family_of_two(&server).await;
    let days = server.state.cfg.limits.retention_days;

    let item = add_sticker(&server, &owner, 0x1A).await;
    let pack_picture = item["attachment"]["id"].as_i64().expect("id");
    // Two sends of it: one that will age out and one that stays.
    let old_copy = upload_webp(&server, &owner, 0x1A).await;
    let old: Value = send_sticker(&server, &owner, chat_id, old_copy)
        .await
        .json()
        .await
        .expect("JSON");
    let old_id = old["message"]["id"].as_i64().expect("id");
    let kept_copy = upload_webp(&server, &member, 0x1A).await;
    let kept: Value = send_sticker(&server, &member, chat_id, kept_copy)
        .await
        .json()
        .await
        .expect("JSON");
    let kept_id = kept["message"]["id"].as_i64().expect("id");

    // All three rows name ONE file: the copies cost an upload, not disk.
    let key = storage_key(&server, pack_picture).await;
    assert_eq!(storage_key(&server, old_copy).await, key);
    assert_eq!(storage_key(&server, kept_copy).await, key);
    let file = server.state.storage.blob_path(&key);
    assert!(file.exists());

    sqlx::query("UPDATE messages SET created_at = now() - make_interval(days => $2) WHERE id = $1")
        .bind(old_id)
        .bind((days + 1) as i32)
        .execute(&server.state.pool)
        .await
        .expect("ageing the message");
    // The pack item is made just as old. It must not matter: retention
    // reads messages, and a pack is not history.
    sqlx::query("UPDATE pack_items SET created_at = now() - interval '5 years'")
        .execute(&server.state.pool)
        .await
        .expect("ageing the pack");
    sqlx::query("UPDATE attachments SET created_at = now() - interval '5 years'")
        .execute(&server.state.pool)
        .await
        .expect("ageing the uploads");

    let swept = family_connect::handlers_chat::sweep_expired_messages(&server.state)
        .await
        .expect("the retention sweep runs");
    assert_eq!(swept, 1);
    // …and the unclaimed sweep beside it, as the hourly pass runs them.
    let unclaimed = family_connect::handlers_attachment::sweep_unclaimed(&server.state)
        .await
        .expect("the unclaimed sweep runs");
    assert_eq!(unclaimed, 0);

    // The message and its own attachment row went.
    assert_eq!(attachment_rows(&server, old_copy).await, 0);
    assert_eq!(
        newest_message(&server, &owner, chat_id).await["id"],
        kept_id
    );
    // The pack did not notice.
    assert_eq!(
        pack_ids(&server, &member).await,
        vec![item["id"].as_i64().expect("id")]
    );
    assert_eq!(attachment_rows(&server, pack_picture).await, 1);
    assert!(file.exists(), "the pack's file outlived the swept message");
    assert_eq!(
        server
            .get(&member, &format!("/attachments/{pack_picture}"))
            .await
            .status(),
        200
    );

    // THE OTHER DIRECTION: remove the item, and the message that was sent
    // with it goes on drawing — its row is its own.
    let removed = server
        .delete(&owner, &format!("/families/mine/pack/{}", item["id"]))
        .await;
    assert_eq!(removed.status(), 204);
    assert_eq!(attachment_rows(&server, pack_picture).await, 0);
    assert!(file.exists(), "a sent sticker still names those bytes");
    let fetched = server
        .get(&owner, &format!("/attachments/{kept_copy}"))
        .await;
    assert_eq!(fetched.status(), 200);
    assert_eq!(
        fetched.bytes().await.expect("bytes").to_vec(),
        webp_bytes(256, 0x1A)
    );
    let read = newest_message(&server, &owner, chat_id).await;
    assert_eq!(read["attachments"][0]["sticker"], true);
}

/// A member who LEAVES leaves their stickers behind, and the owner — and
/// only the owner — can remove them.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_departed_members_sticker_stays_and_the_owner_may_remove_it() {
    let server = spawn_server().await;
    let (owner, member, _) = family_of_two(&server).await;
    let third = third_member(&server, &owner).await;
    let member_id = server.user_id(&member).await;

    let left_behind = add_sticker(&server, &member, 0x2A).await;
    let left_id = left_behind["id"].as_i64().expect("id");
    let also = add_sticker(&server, &member, 0x2B).await;
    let also_id = also["id"].as_i64().expect("id");

    let left = server.post(&member, "/families/leave", json!({})).await;
    assert_eq!(left.status(), 204);

    // Still there, still attributed, still fetchable by the family.
    let read = pack(&server, &third).await;
    assert_eq!(read["items"].as_array().expect("items").len(), 2);
    assert_eq!(read["items"][0]["added_by"], member_id);
    assert_eq!(
        server
            .get(
                &third,
                &format!("/attachments/{}", left_behind["attachment"]["id"])
            )
            .await
            .status(),
        200
    );

    // Whoever added it is outside the family now, and has no pack to act on.
    assert_error(
        server
            .delete(&member, &format!("/families/mine/pack/{left_id}"))
            .await,
        409,
        "not_in_family",
    )
    .await;
    // …and from inside ANOTHER family, the item is simply not there.
    server.create_family(&member, "On my own").await;
    assert_error(
        server
            .delete(&member, &format!("/families/mine/pack/{left_id}"))
            .await,
        404,
        "pack_item_not_found",
    )
    .await;
    // Another member still may not.
    assert_error(
        server
            .delete(&third, &format!("/families/mine/pack/{left_id}"))
            .await,
        403,
        "not_pack_item_author",
    )
    .await;

    // The owner can — which is the whole reason the owner may.
    let removed = server
        .delete(&owner, &format!("/families/mine/pack/{left_id}"))
        .await;
    assert_eq!(removed.status(), 204);
    assert_eq!(pack_ids(&server, &third).await, vec![also_id]);
}

/// The same for a DELETED account: the account's unused uploads go, its
/// pack items do not, and the owner can still remove them.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_deleted_accounts_sticker_stays_in_the_pack() {
    let server = spawn_server().await;
    let (owner, member, _) = family_of_two(&server).await;
    let member_id = server.user_id(&member).await;

    let item = add_sticker(&server, &member, 0x3A).await;
    let item_id = item["id"].as_i64().expect("id");
    let picture = item["attachment"]["id"].as_i64().expect("id");
    // An upload the member never used, which the scrub DOES remove.
    let unused = upload_webp(&server, &member, 0x3B).await;

    let deleted = server
        .post(&member, "/me/delete", json!({"password": "password123"}))
        .await;
    assert_eq!(deleted.status(), 204);

    assert_eq!(attachment_rows(&server, unused).await, 0);
    assert_eq!(
        attachment_rows(&server, picture).await,
        1,
        "a pack picture is not an unused upload"
    );
    let read = pack(&server, &owner).await;
    assert_eq!(read["items"][0]["id"], item_id);
    assert_eq!(read["items"][0]["added_by"], member_id);
    assert_eq!(
        server
            .get(&owner, &format!("/attachments/{picture}"))
            .await
            .status(),
        200
    );
    // The name resolves the way their old messages do.
    let family = my_family(&server, &owner).await;
    assert_eq!(family["former_members"][0]["id"], member_id);

    let removed = server
        .delete(&owner, &format!("/families/mine/pack/{item_id}"))
        .await;
    assert_eq!(removed.status(), 204);
    assert!(pack_ids(&server, &owner).await.is_empty());
}

/// Deleting the family takes its pack: the items, the picture rows and the
/// files on disk.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn deleting_the_family_deletes_its_pack() {
    let server = spawn_server().await;
    let (owner, _) = server.register("owner", "Olive").await;
    let (family_id, _) = server.create_family(&owner, "The Smiths").await;

    let kept = add_sticker(&server, &owner, 0x4A).await;
    let removed = add_sticker(&server, &owner, 0x4B).await;
    // One tombstone too, so the cascade is seen to take those as well.
    let response = server
        .delete(&owner, &format!("/families/mine/pack/{}", removed["id"]))
        .await;
    assert_eq!(response.status(), 204);
    let picture = kept["attachment"]["id"].as_i64().expect("id");
    let file = server
        .state
        .storage
        .blob_path(&storage_key(&server, picture).await);
    assert!(file.exists());

    // The sole member leaving deletes the family.
    let left = server.post(&owner, "/families/leave", json!({})).await;
    assert_eq!(left.status(), 204);
    let families: i64 = sqlx::query_scalar("SELECT count(*) FROM families WHERE id = $1")
        .bind(family_id)
        .fetch_one(&server.state.pool)
        .await
        .expect("counting families");
    assert_eq!(families, 0, "the family went with its last member");

    let items: i64 = sqlx::query_scalar("SELECT count(*) FROM pack_items")
        .fetch_one(&server.state.pool)
        .await
        .expect("counting pack items");
    assert_eq!(items, 0, "live items and tombstones both went");
    assert_eq!(attachment_rows(&server, picture).await, 0);
    assert!(!file.exists(), "the pack's file outlived the family");
}

/// Two members adding the SAME picture at the same moment get one item,
/// and two adding different ones at a ceiling of one get one item and one
/// `pack_full` — the family's row lock is what makes both exact.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn concurrent_adds_neither_duplicate_nor_overfill() {
    let server = spawn_server_with_config(|cfg| cfg.limits.max_pack_items = 1).await;
    let (owner, member, _) = family_of_two(&server).await;

    // Different pictures, one slot.
    let a = upload_webp(&server, &owner, 0x5A).await;
    let b = upload_webp(&server, &member, 0x5B).await;
    let (first, second) = tokio::join!(claim(&server, &owner, a), claim(&server, &member, b));
    let mut statuses = vec![first.status().as_u16(), second.status().as_u16()];
    statuses.sort_unstable();
    assert_eq!(statuses, vec![201, 409], "one lands and one is refused");
    let ids = pack_ids(&server, &owner).await;
    assert_eq!(ids.len(), 1);

    // The same picture, from both at once: one item, whoever won.
    let removed = server
        .delete(&owner, &format!("/families/mine/pack/{}", ids[0]))
        .await;
    assert_eq!(removed.status(), 204);
    let a = upload_webp(&server, &owner, 0x5C).await;
    let b = upload_webp(&server, &member, 0x5C).await;
    let (first, second) = tokio::join!(claim(&server, &owner, a), claim(&server, &member, b));
    let mut statuses = vec![first.status().as_u16(), second.status().as_u16()];
    statuses.sort_unstable();
    assert_eq!(statuses, vec![200, 201], "one adds it and one is told so");
    let first: Value = first.json().await.expect("JSON");
    let second: Value = second.json().await.expect("JSON");
    assert_eq!(first["item"]["id"], second["item"]["id"]);
    assert_eq!(pack_ids(&server, &member).await.len(), 1);
}

/// A pack claim is an upload made in the family the caller is in NOW. A
/// member who changed family inside the unclaimed grace still holds the
/// row — it is theirs, it is unclaimed, it is a WebP — and may not pin the
/// old family's file into the new family's pack: that file is
/// deduplicated against, and deleted with, the family it went up into.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_upload_from_a_previous_family_is_not_a_sticker_here() {
    let server = spawn_server().await;
    let (owner, member, _) = family_of_two(&server).await;

    let carried = upload_webp(&server, &member, 0x61).await;
    let left = server.post(&member, "/families/leave", json!({})).await;
    assert_eq!(left.status(), 204);
    server.create_family(&member, "On my own").await;

    // The premise, or the refusal below proves nothing: leaving took
    // nothing, and the row is still the caller's own unclaimed upload.
    assert_eq!(attachment_rows(&server, carried).await, 1);
    assert_error(
        claim(&server, &member, carried).await,
        404,
        "attachment_not_found",
    )
    .await;
    assert!(pack_ids(&server, &member).await.is_empty());
    // Refused, not consumed: the row is left for the sweep, unclaimed.
    let claimed: Option<i64> =
        sqlx::query_scalar("SELECT pack_item_id FROM attachments WHERE id = $1")
            .bind(carried)
            .fetch_one(&server.state.pool)
            .await
            .expect("the upload's row");
    assert_eq!(claimed, None);

    // The same bytes uploaded again, HERE, are a sticker like any other.
    let fresh = upload_webp(&server, &member, 0x61).await;
    assert_eq!(claim(&server, &member, fresh).await.status(), 201);
    // And the family they left never saw any of it.
    assert!(pack_ids(&server, &owner).await.is_empty());
}

/// A sticker has no body and an edit cannot give it one: `PATCH` refuses
/// an empty body, so words put on a sticker could never be taken off
/// again, and five clients draw a sticker with no bubble to put them in.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_sticker_message_cannot_be_edited() {
    let server = spawn_server().await;
    let (owner, member, chat_id) = family_of_two(&server).await;

    let picture = upload_webp(&server, &owner, 0x71).await;
    let response = send_sticker(&server, &owner, chat_id, picture).await;
    assert_eq!(response.status(), 201);
    let sent: Value = response.json().await.expect("JSON");
    let sticker_id = sent["message"]["id"].as_i64().expect("id");
    let path = format!("/chats/{chat_id}/messages/{sticker_id}");

    assert_error(
        server.patch(&owner, &path, json!({"body": "hello"})).await,
        400,
        "validation",
    )
    .await;
    // Somebody else is told what they would be told about any message:
    // the refusal is the author's to hear, and says nothing to the rest.
    assert_error(
        server.patch(&member, &path, json!({"body": "hello"})).await,
        403,
        "not_message_author",
    )
    .await;

    // Nothing moved: no body, no edit mark, still a sticker.
    let read = newest_message(&server, &member, chat_id).await;
    assert_eq!(read["id"], sticker_id);
    assert_eq!(read["body"], "");
    assert!(read.get("edited_at").is_none_or(Value::is_null));
    assert_eq!(read["attachments"][0]["sticker"], true);

    // The SAME bytes sent as a photo are a photo, and its caption is the
    // author's to rewrite as it always was.
    let photo = upload_webp(&server, &owner, 0x71).await;
    let response = server
        .post(
            &owner,
            &format!("/chats/{chat_id}/messages"),
            json!({"client_msg_id": Uuid::new_v4().to_string(), "body": "look",
                   "attachment_ids": [photo]}),
        )
        .await;
    assert_eq!(response.status(), 201);
    let sent: Value = response.json().await.expect("JSON");
    let photo_id = sent["message"]["id"].as_i64().expect("id");
    let edited = server
        .patch(
            &owner,
            &format!("/chats/{chat_id}/messages/{photo_id}"),
            json!({"body": "look at this"}),
        )
        .await;
    assert_eq!(edited.status(), 200);
    let edited: Value = edited.json().await.expect("JSON");
    assert_eq!(edited["message"]["body"], "look at this");
}

/// A live item always comes with its picture, however a read interleaves
/// with a removal. The items and their pictures are two statements, and a
/// removal deletes the picture's row in the commit that tombstones the
/// item — so read a statement apart, an item came back live with no
/// `attachment`, a shape `PackItem` does not have.
///
/// A race, so this is a hammer and not a proof: one member adds and removes
/// while another reads both feeds as fast as they answer.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_live_item_is_never_read_without_its_picture() {
    let server = spawn_server().await;
    let (owner, member, _) = family_of_two(&server).await;

    // Some standing items, so every read has rows to fetch pictures for.
    for seed in 0..8u8 {
        add_sticker(&server, &owner, seed).await;
    }

    let churn = async {
        for round in 0..60u8 {
            let mut ids = Vec::new();
            for seed in 0..4u8 {
                let item = add_sticker(&server, &owner, 0x80 + (round % 2) * 4 + seed).await;
                ids.push(item["id"].as_i64().expect("id"));
            }
            for id in ids {
                let removed = server
                    .delete(&owner, &format!("/families/mine/pack/{id}"))
                    .await;
                assert_eq!(removed.status(), 204);
            }
        }
    };
    let done = std::sync::atomic::AtomicBool::new(false);
    let reads = async {
        let mut reads = 0usize;
        while !done.load(std::sync::atomic::Ordering::Relaxed) {
            let whole = pack(&server, &member).await;
            let feed = changes(&server, &member, 0).await;
            for item in whole["items"]
                .as_array()
                .expect("items")
                .iter()
                .chain(&feed)
            {
                if item["deleted"] == true {
                    assert!(item.get("attachment").is_none(), "a tombstone has none");
                } else {
                    assert!(
                        item["attachment"]["id"].is_i64(),
                        "a live item with no picture: {item}"
                    );
                }
            }
            reads += 1;
        }
        reads
    };
    let (_, reads) = tokio::join!(
        async {
            churn.await;
            done.store(true, std::sync::atomic::Ordering::Relaxed);
        },
        reads
    );
    assert!(reads > 0, "the reader ran beside the churn");
}
