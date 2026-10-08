//! Integration: a VIDEO MESSAGE — a short square video sent to be drawn as
//! a circle (protocol.md, "Video messages").
//!
//! Modelled on pack_flow.rs's sticker-message half, because the wire is the
//! sticker's pattern: an ordinary `kind=video` attachment with one flag,
//! `round`, set by the send, stored on the attachment and carried by every
//! read. What is under test is that flag on every read path and ABSENT on
//! an ordinary video; the five checks of "What the server checks", each a
//! 400 that leaves the upload unclaimed and unflagged; the edit refusal,
//! asked last; the push word; retention; and the two discovery keys.

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

/// How long anything asynchronous is waited for: a deadline polled to,
/// never a sleep that is trusted.
const WAIT: Duration = Duration::from_secs(5);

/// The upload protocol.md's example makes: a 480 x 480 clip of 23.4 s.
const ROUND_QUERY: &str = "kind=video&width=480&height=480&duration_ms=23400";

// --- Minimal WebSocket client (mirrors the helpers in pack_flow.rs) ---------

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

/// An ISO base media file by its magic number — `ftyp` at offset 4 — and
/// nothing more, which is all the server ever looks at. `seed` fills the
/// rest, so two clips in one test are two files.
fn mp4_bytes(len: usize, seed: u8) -> Vec<u8> {
    let mut bytes = vec![0x00, 0x00, 0x00, 0x18, b'f', b't', b'y', b'p'];
    bytes.extend_from_slice(b"isom");
    bytes.resize(len.max(12), seed);
    bytes
}

fn jpeg_bytes(len: usize, seed: u8) -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE0];
    bytes.resize(len.max(4), seed);
    bytes
}

fn webp_bytes(len: usize, seed: u8) -> Vec<u8> {
    let mut bytes = b"RIFF\x00\x00\x00\x00WEBP".to_vec();
    bytes.resize(len.max(12), seed);
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

async fn my_family(ts: &TestServer, token: &str) -> Value {
    let response = ts.get(token, "/families/mine").await;
    assert_eq!(response.status(), 200, "reading the family");
    response.json().await.expect("family response is JSON")
}

/// Upload raw bytes with any query and type; returns the attachment id.
async fn upload_as(ts: &TestServer, token: &str, query: &str, mime: &str, bytes: Vec<u8>) -> i64 {
    let response = ts
        .put_bytes_method("POST", token, &format!("/attachments?{query}"), mime, bytes)
        .await;
    assert_eq!(response.status(), 201, "uploading {query} as {mime:?}");
    let body: Value = response.json().await.expect("JSON");
    body["attachment"]["id"].as_i64().expect("attachment id")
}

/// Upload a clip as a client recording a video message does.
async fn upload_round(ts: &TestServer, token: &str, seed: u8) -> i64 {
    upload_as(ts, token, ROUND_QUERY, "video/mp4", mp4_bytes(512, seed)).await
}

/// Send one attachment as a VIDEO MESSAGE over REST.
async fn send_round(
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
            "round": true,
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

async fn last_message(ts: &TestServer, token: &str, chat_id: i64) -> Value {
    let chats: Value = ts.get(token, "/chats").await.json().await.expect("JSON");
    chats["chats"]
        .as_array()
        .expect("chats")
        .iter()
        .find(|entry| entry["chat"]["id"] == chat_id)
        .expect("the chat is listed")["last_message"]
        .clone()
}

/// How many uploads a send has claimed or flagged — the "nothing landed"
/// half of every refusal.
async fn claimed_or_flagged(ts: &TestServer) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM attachments WHERE message_id IS NOT NULL OR round OR sticker",
    )
    .fetch_one(&ts.state.pool)
    .await
    .expect("counting")
}

// --- The flag on every read ----------------------------------------------------

/// The round trip: a message sent with `round: true` carries the flag on its
/// attachment on every read — the answer, the frame, a page of history, the
/// legacy field, the chat list, a thread, the edits feed, a retry — and an
/// ordinary video, the same square MP4, never carries the key at all.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_video_message_carries_the_flag_and_an_ordinary_video_does_not() {
    let server = spawn_server().await;
    let (owner, member, chat_id) = family_of_two(&server).await;
    let mut member_ws = connect_ws(&server, &member).await;

    let clip = upload_round(&server, &owner, 0xA1).await;
    let response = send_round(&server, &owner, chat_id, clip).await;
    assert_eq!(response.status(), 201);
    let sent: Value = response.json().await.expect("JSON");
    let message = &sent["message"];
    let message_id = message["id"].as_i64().expect("id");
    assert_eq!(message["body"], "");
    assert_eq!(message["attachments"].as_array().expect("list").len(), 1);
    let attachment = &message["attachments"][0];
    assert_eq!(attachment["id"], clip);
    assert_eq!(attachment["kind"], "video");
    assert_eq!(attachment["mime"], "video/mp4");
    assert_eq!(attachment["width"], 480);
    assert_eq!(attachment["height"], 480);
    assert_eq!(attachment["duration_ms"], 23400);
    assert_eq!(attachment["round"], true);
    assert!(attachment.get("sticker").is_none(), "{attachment}");
    assert_eq!(
        message["attachment"], message["attachments"][0],
        "the legacy field carries it too"
    );

    // The frame every other member gets.
    let frame = next_frame_of_type(&mut member_ws, "message").await;
    assert_eq!(frame["message"]["id"], message_id);
    assert_eq!(frame["message"]["attachments"][0]["round"], true);
    assert_eq!(frame["message"]["attachment"]["round"], true);

    // A page of history, read by the member.
    let read = newest_message(&server, &member, chat_id).await;
    assert_eq!(read["id"], message_id);
    assert_eq!(read["attachments"][0]["round"], true);
    assert_eq!(read["attachment"]["round"], true);

    // The chat-list preview, so a row can say "Video message".
    let last = last_message(&server, &member, chat_id).await;
    assert_eq!(last["id"], message_id);
    assert_eq!(last["attachments"][0]["round"], true);
    assert_eq!(last["attachment"]["round"], true);

    // A thread: a reply makes it a root, and the thread read carries it.
    let reply = server
        .post(
            &member,
            &format!("/chats/{chat_id}/messages"),
            json!({"client_msg_id": Uuid::new_v4().to_string(), "body": "Lovely!",
                   "reply_to_message_id": message_id}),
        )
        .await;
    assert_eq!(reply.status(), 201);
    let thread: Value = server
        .get(
            &member,
            &format!("/chats/{chat_id}/messages/{message_id}/thread"),
        )
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(thread["messages"][0]["id"], message_id);
    assert_eq!(thread["messages"][0]["attachments"][0]["round"], true);

    // The edits feed. A video message cannot be edited, so only the
    // database can put one there — which is the point: whatever reaches
    // that feed is hydrated by the same reader, and must carry the flag.
    sqlx::query(
        "UPDATE messages SET edit_seq = nextval('message_edit_seq'), edited_at = now()
         WHERE id = $1",
    )
    .bind(message_id)
    .execute(&server.state.pool)
    .await
    .expect("stamping an edit seq");
    let edits: Value = server
        .get(&member, &format!("/chats/{chat_id}/edits?after_seq=0"))
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(edits["messages"][0]["id"], message_id);
    assert_eq!(edits["messages"][0]["attachments"][0]["round"], true);

    // The member can fetch the bytes, as for any message's attachment.
    assert_eq!(
        server
            .get(&member, &format!("/attachments/{clip}"))
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
                   "attachment_ids": [clip], "round": true}),
        )
        .await;
    assert_eq!(retry.status(), 200);
    let retry: Value = retry.json().await.expect("JSON");
    assert_eq!(retry["message"]["id"], message_id);
    assert_eq!(retry["message"]["attachments"][0]["round"], true);

    // THE SAME SQUARE MP4 AS AN ORDINARY VIDEO: no flag, and not `false`
    // either — absent, by the wire's usual rule. `round: false` is the same
    // statement as leaving it out, and a caption is an ordinary video's.
    for (seed, body) in [
        (0xA2, json!({"body": ""})),
        (0xA3, json!({"body": "", "round": false})),
        (0xA4, json!({"body": "look at this"})),
    ] {
        let video = upload_round(&server, &owner, seed).await;
        let mut request = body.clone();
        request["client_msg_id"] = json!(Uuid::new_v4().to_string());
        request["attachment_ids"] = json!([video]);
        let response = server
            .post(&owner, &format!("/chats/{chat_id}/messages"), request)
            .await;
        assert_eq!(response.status(), 201, "{body}");
        let sent: Value = response.json().await.expect("JSON");
        assert!(
            sent["message"]["attachments"][0].get("round").is_none(),
            "an ordinary video carries no round key: {sent}"
        );
        assert!(sent["message"]["attachment"].get("round").is_none());
        let frame = next_frame_of_type(&mut member_ws, "message").await;
        assert!(frame["message"]["attachments"][0].get("round").is_none());
        let read = newest_message(&server, &member, chat_id).await;
        assert_eq!(read["id"], sent["message"]["id"]);
        assert!(read["attachments"][0].get("round").is_none());
    }
    let last = last_message(&server, &member, chat_id).await;
    assert!(last["attachments"][0].get("round").is_none(), "{last}");
    // And the row says so too: only the one video message is flagged.
    let flagged: i64 = sqlx::query_scalar("SELECT count(*) FROM attachments WHERE round")
        .fetch_one(&server.state.pool)
        .await
        .expect("counting");
    assert_eq!(flagged, 1);
}

/// The socket sends one the same way: `round: true` on the `send` frame,
/// the ack carries the flag back, and a refusal is an error frame naming
/// the send, with nothing landed.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_video_message_is_sent_over_the_socket_too() {
    let server = spawn_server().await;
    let (owner, member, chat_id) = family_of_two(&server).await;
    let mut owner_ws = connect_ws(&server, &owner).await;
    let mut member_ws = connect_ws(&server, &member).await;

    let clip = upload_round(&server, &owner, 0xB1).await;
    let client_msg_id = Uuid::new_v4().to_string();
    owner_ws
        .send(Message::text(
            json!({"type": "send", "chat_id": chat_id, "client_msg_id": client_msg_id,
                   "body": "", "attachment_ids": [clip], "round": true})
            .to_string(),
        ))
        .await
        .expect("sending");
    let ack = next_frame_of_type(&mut owner_ws, "ack").await;
    assert_eq!(ack["client_msg_id"], client_msg_id);
    assert_eq!(ack["message"]["attachments"][0]["round"], true);
    assert_eq!(ack["message"]["attachments"][0]["kind"], "video");
    let frame = next_frame_of_type(&mut member_ws, "message").await;
    assert_eq!(frame["message"]["attachments"][0]["round"], true);

    // Refusals come back as error frames naming the send: words beside it
    // (`validation`), and a picture that is not a video
    // (`invalid_attachment`, from the check after the claim).
    let worded = upload_round(&server, &owner, 0xB2).await;
    let photo = upload_as(
        &server,
        &owner,
        "kind=photo&width=480&height=480",
        "image/jpeg",
        jpeg_bytes(256, 0xB3),
    )
    .await;
    for (request, code) in [
        (
            json!({"body": "hello", "attachment_ids": [worded], "round": true}),
            "validation",
        ),
        (
            json!({"body": "", "attachment_ids": [photo], "round": true}),
            "invalid_attachment",
        ),
    ] {
        let refused_id = Uuid::new_v4().to_string();
        let mut frame = request;
        frame["type"] = json!("send");
        frame["chat_id"] = json!(chat_id);
        frame["client_msg_id"] = json!(refused_id);
        owner_ws
            .send(Message::text(frame.to_string()))
            .await
            .expect("sending");
        let error = next_frame_of_type(&mut owner_ws, "error").await;
        assert_eq!(error["code"], code, "{error}");
        assert_eq!(error["client_msg_id"], refused_id);
    }
    assert_eq!(
        newest_message(&server, &member, chat_id).await["id"],
        ack["message"]["id"],
        "the refused sends left no message"
    );
    let free: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM attachments
         WHERE id = ANY($1) AND message_id IS NULL AND NOT round",
    )
    .bind(vec![worded, photo])
    .fetch_one(&server.state.pool)
    .await
    .expect("counting");
    assert_eq!(free, 2, "both refused uploads are still free and unflagged");
}

// --- What the server refuses -----------------------------------------------------

/// Checks 1 to 5 of "What the server checks", each a 400 — never a 500 an
/// outbox would retry for ever — and each leaving NOTHING behind: no
/// message, every upload unclaimed and unflagged. Then each upload is still
/// good for what it is, and the edges of the declared shape are accepted.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_ways_of_sending_a_video_message_wrong() {
    let server = spawn_server().await;
    let (owner, _, chat_id) = family_of_two(&server).await;
    let path = format!("/chats/{chat_id}/messages");
    let send = |body: Value| {
        let mut request = body;
        request["client_msg_id"] = json!(Uuid::new_v4().to_string());
        server.post(&owner, &path, request)
    };

    let good = upload_round(&server, &owner, 0xC1).await;
    let other = upload_round(&server, &owner, 0xC2).await;

    // 1. Beside a poll — with or without a video named.
    assert_error(
        send(json!({"body": "Pizza?", "poll": {"options": ["Yes", "No"]}, "round": true})).await,
        400,
        "invalid_poll",
    )
    .await;
    assert_error(
        send(json!({"body": "Pizza?", "poll": {"options": ["Yes", "No"]},
                    "attachment_ids": [good], "round": true}))
        .await,
        400,
        "invalid_poll",
    )
    .await;
    // 2. Beside `sticker`: `validation`, whatever else the send got wrong —
    // asked before the sticker's own shape rules and before any id is read.
    assert_error(
        send(json!({"body": "", "attachment_ids": [good], "round": true, "sticker": true})).await,
        400,
        "validation",
    )
    .await;
    assert_error(
        send(json!({"body": "", "attachment_ids": [good, other],
                    "round": true, "sticker": true}))
        .await,
        400,
        "validation",
    )
    .await;
    // 3. Not exactly one attachment: none, and two.
    assert_error(
        send(json!({"body": "", "round": true})).await,
        400,
        "invalid_attachment",
    )
    .await;
    assert_error(
        send(json!({"body": "", "attachment_ids": [good, other], "round": true})).await,
        400,
        "invalid_attachment",
    )
    .await;
    // 4. Words beside it — in either spelling of the one attachment.
    assert_error(
        send(json!({"body": "hi", "attachment_ids": [good], "round": true})).await,
        400,
        "validation",
    )
    .await;
    assert_error(
        send(json!({"body": "hi", "attachment_id": good, "round": true})).await,
        400,
        "validation",
    )
    .await;

    // 5. After the claim — the WRONG KIND, each refused as
    // `invalid_attachment` with nothing claimed or flagged. The claim writes
    // `round = ($6 AND kind = 'video')`, so none of these can reach 0052's
    // CHECK: a 500 here would be the bug this test exists for.
    let photo = upload_as(
        &server,
        &owner,
        "kind=photo&width=480&height=480",
        "image/jpeg",
        jpeg_bytes(256, 0xC3),
    )
    .await;
    let sticker_picture = upload_as(
        &server,
        &owner,
        "kind=photo&width=480&height=480",
        "image/webp",
        webp_bytes(256, 0xC4),
    )
    .await;
    let audio = upload_as(
        &server,
        &owner,
        "kind=audio&duration_ms=23400",
        "audio/mp4",
        mp4_bytes(256, 0xC5),
    )
    .await;
    // A FILE that says it is an MP4, square and timed: still a file.
    let file = upload_as(
        &server,
        &owner,
        "kind=file&name=clip.mp4&width=480&height=480&duration_ms=23400",
        "video/mp4",
        mp4_bytes(256, 0xC6),
    )
    .await;
    let location = upload_as(
        &server,
        &owner,
        "kind=location&latitude=55.7558&longitude=37.6173",
        "",
        Vec::new(),
    )
    .await;
    // The wrong video: QuickTime is never within the profile.
    let quicktime = upload_as(
        &server,
        &owner,
        ROUND_QUERY,
        "video/quicktime",
        mp4_bytes(256, 0xC7),
    )
    .await;
    // The wrong declared shape or length.
    let mut wrong_shapes = Vec::new();
    for (seed, query) in [
        (0xD0, "kind=video&height=480&duration_ms=23400"),
        (0xD1, "kind=video&width=480&duration_ms=23400"),
        (0xD2, "kind=video&duration_ms=23400"),
        (0xD3, "kind=video&width=640&height=480&duration_ms=23400"),
        (0xD4, "kind=video&width=0&height=0&duration_ms=23400"),
        (0xD5, "kind=video&width=-480&height=-480&duration_ms=23400"),
        (0xD6, "kind=video&width=721&height=721&duration_ms=23400"),
        (0xD7, "kind=video&width=480&height=480"),
        (0xD8, "kind=video&width=480&height=480&duration_ms=0"),
        (0xD9, "kind=video&width=480&height=480&duration_ms=-1"),
        (0xDA, "kind=video&width=480&height=480&duration_ms=60001"),
    ] {
        let id = upload_as(&server, &owner, query, "video/mp4", mp4_bytes(256, seed)).await;
        wrong_shapes.push((id, query));
    }
    let mut refused: Vec<(i64, String)> = vec![
        (photo, "a photo".to_string()),
        (sticker_picture, "a WebP photo".to_string()),
        (audio, "an audio".to_string()),
        (file, "a file".to_string()),
        (location, "a location".to_string()),
        (quicktime, "a QuickTime video".to_string()),
    ];
    refused.extend(
        wrong_shapes
            .iter()
            .map(|(id, query)| (*id, query.to_string())),
    );
    for (id, what) in &refused {
        let response = send_round(&server, &owner, chat_id, *id).await;
        assert_eq!(
            response.status(),
            400,
            "{what} sent as a video message must be a 400, never a 500"
        );
        let body: Value = response.json().await.expect("JSON");
        assert_eq!(
            body["error"]["code"], "invalid_attachment",
            "{what}: {body}"
        );
    }

    // NOTHING LANDED. The chat is empty, and every refused upload is still
    // unclaimed and unflagged — the refusal rolled the claim back.
    let page: Value = server.get(&owner, &path).await.json().await.expect("JSON");
    assert_eq!(page["messages"], json!([]));
    assert_eq!(
        claimed_or_flagged(&server).await,
        0,
        "a refused video message claimed or flagged something"
    );

    // So each is still good for what it IS: the photo, the audio, the file,
    // the location, the QuickTime clip and every odd shape as the ordinary
    // thing it is — and the good clip as a video message.
    for (id, what) in &refused {
        assert_eq!(
            send(json!({"body": "", "attachment_ids": [id]}))
                .await
                .status(),
            201,
            "{what} as an ordinary message"
        );
    }
    let response = send_round(&server, &owner, chat_id, good).await;
    assert_eq!(response.status(), 201);

    // THE EDGES ARE IN: a 1 x 1 and a 720 x 720 square, 1 ms and exactly
    // the 60 000 ms limit. Declared, never measured.
    for (seed, query) in [
        (0xE0, "kind=video&width=1&height=1&duration_ms=1"),
        (0xE1, "kind=video&width=720&height=720&duration_ms=60000"),
    ] {
        let id = upload_as(&server, &owner, query, "video/mp4", mp4_bytes(256, seed)).await;
        let response = send_round(&server, &owner, chat_id, id).await;
        assert_eq!(response.status(), 201, "{query}");
        let sent: Value = response.json().await.expect("JSON");
        assert_eq!(sent["message"]["attachments"][0]["round"], true, "{query}");
    }
}

/// The two discovery keys, ALWAYS present — their absence is how a client
/// knows the server predates video messages — and the byte ceiling the one
/// IN FORCE: on the test servers' 64 KiB attachment ceiling the 12 MiB
/// default clamps to 64 KiB rather than refusing to start; set lower, it is
/// what the claim checks, between that and 64 KiB.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_keys_are_present_and_the_byte_ceiling_is_configurable() {
    // The default, clamped.
    let server = spawn_server().await;
    let (owner, member, _) = family_of_two(&server).await;
    for token in [&owner, &member] {
        let family = my_family(&server, token).await;
        assert_eq!(family["max_round_video_ms"], 60000, "{family}");
        assert_eq!(
            family["max_round_video_bytes"].as_u64(),
            Some(server.state.cfg.limits.max_attachment_bytes as u64),
            "the unset default clamps to the attachment ceiling: {family}"
        );
    }
    drop(server);

    // A written ceiling, below the attachment one.
    let server = spawn_server_with_config(|cfg| {
        cfg.limits.max_round_video_bytes = Some(8 * 1024);
    })
    .await;
    let (owner, _, chat_id) = family_of_two(&server).await;
    let family = my_family(&server, &owner).await;
    assert_eq!(family["max_round_video_ms"], 60000);
    assert_eq!(family["max_round_video_bytes"], 8 * 1024);
    assert!(server.state.cfg.limits.max_attachment_bytes > 8 * 1024 + 1);

    // One byte over it: a fine video, not a video message.
    let over = upload_as(
        &server,
        &owner,
        ROUND_QUERY,
        "video/mp4",
        mp4_bytes(8 * 1024 + 1, 0xF0),
    )
    .await;
    assert_error(
        send_round(&server, &owner, chat_id, over).await,
        400,
        "invalid_attachment",
    )
    .await;
    assert_eq!(claimed_or_flagged(&server).await, 0);
    // Exactly at it: in.
    let at = upload_as(
        &server,
        &owner,
        ROUND_QUERY,
        "video/mp4",
        mp4_bytes(8 * 1024, 0xF1),
    )
    .await;
    let response = send_round(&server, &owner, chat_id, at).await;
    assert_eq!(response.status(), 201);
    // And the refused one still goes as an ordinary video.
    let response = server
        .post(
            &owner,
            &format!("/chats/{chat_id}/messages"),
            json!({"client_msg_id": Uuid::new_v4().to_string(), "body": "",
                   "attachment_ids": [over]}),
        )
        .await;
    assert_eq!(response.status(), 201);
    let sent: Value = response.json().await.expect("JSON");
    assert!(sent["message"]["attachments"][0].get("round").is_none());
}

// --- A message like any other -----------------------------------------------------

/// Check 6: a video message may be a reply, may be posted into a thread,
/// and is accepted in a direct chat — the server has no reason to refuse
/// any of them, whatever the apps offer. It takes a reaction, and a report
/// names it as the `video` it is.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_video_message_is_a_reply_a_thread_post_and_a_direct_message() {
    let server = spawn_server().await;
    let (owner, member, chat_id) = family_of_two(&server).await;
    let member_id = server.user_id(&member).await;

    let first: Value = server
        .post(
            &owner,
            &format!("/chats/{chat_id}/messages"),
            json!({"client_msg_id": Uuid::new_v4().to_string(), "body": "How was the trip?"}),
        )
        .await
        .json()
        .await
        .expect("JSON");
    let first_id = first["message"]["id"].as_i64().expect("id");

    // As a REPLY, which also puts it in the thread rooted at `first`.
    let clip = upload_round(&server, &member, 0x11).await;
    let response = server
        .post(
            &member,
            &format!("/chats/{chat_id}/messages"),
            json!({"client_msg_id": Uuid::new_v4().to_string(), "body": "",
                   "attachment_ids": [clip], "round": true,
                   "reply_to_message_id": first_id}),
        )
        .await;
    assert_eq!(response.status(), 201);
    let reply: Value = response.json().await.expect("JSON");
    assert_eq!(reply["message"]["reply_to"]["message_id"], first_id);
    assert_eq!(reply["message"]["thread_root_id"], first_id);
    assert_eq!(reply["message"]["attachments"][0]["round"], true);
    let reply_id = reply["message"]["id"].as_i64().expect("id");
    let thread: Value = server
        .get(
            &owner,
            &format!("/chats/{chat_id}/messages/{first_id}/thread"),
        )
        .await
        .json()
        .await
        .expect("JSON");
    let in_thread = thread["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .find(|message| message["id"] == reply_id)
        .expect("the video message is in the thread");
    assert_eq!(in_thread["attachments"][0]["round"], true);

    // A reaction, like any message.
    let reacted = server
        .put(
            &owner,
            &format!("/chats/{chat_id}/messages/{reply_id}/reaction"),
            json!({"emoji": "❤️"}),
        )
        .await;
    assert_eq!(reacted.status(), 200);
    // A report carries kind and name only: it is a video there.
    let report = server
        .post(
            &owner,
            "/families/reports",
            json!({"reported_user_id": member_id, "reason": "other", "message_id": reply_id}),
        )
        .await;
    assert_eq!(report.status(), 201);
    let report: Value = report.json().await.expect("JSON");
    assert_eq!(report["report"]["message_attachments"][0]["kind"], "video");
    assert!(
        report["report"]["message_attachments"][0]
            .get("round")
            .is_none()
    );

    // In a DIRECT chat.
    let direct = server
        .post(&owner, "/chats/direct", json!({"user_id": member_id}))
        .await;
    assert_eq!(direct.status(), 200);
    let direct: Value = direct.json().await.expect("JSON");
    let direct_id = direct["chat"]["id"].as_i64().expect("id");
    let clip = upload_round(&server, &owner, 0x12).await;
    assert_eq!(
        send_round(&server, &owner, direct_id, clip).await.status(),
        201
    );
    let read = newest_message(&server, &member, direct_id).await;
    assert_eq!(read["attachments"][0]["round"], true);
}

/// A push for a video message says "Video message", where a video says
/// "Video" — written by the server, so every installed app shows it.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_video_message_pushes_the_words_video_message() {
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

    // Dispatch is spawned fire-and-forget, so a bare read races it: poll
    // to a deadline for the n-th push.
    let push_number = |n: usize| {
        let server = &server;
        async move {
            let deadline = tokio::time::Instant::now() + WAIT;
            loop {
                let calls = server.push.calls();
                if calls.len() >= n {
                    break calls[n - 1].note.body.clone();
                }
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "push number {n} never came"
                );
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        }
    };

    let clip = upload_round(&server, &owner, 0x21).await;
    assert_eq!(
        send_round(&server, &owner, chat_id, clip).await.status(),
        201
    );
    assert_eq!(push_number(1).await, "Video message");

    // The same clip as an ordinary video is still "Video".
    let video = upload_round(&server, &owner, 0x22).await;
    let response = server
        .post(
            &owner,
            &format!("/chats/{chat_id}/messages"),
            json!({"client_msg_id": Uuid::new_v4().to_string(), "body": "",
                   "attachment_ids": [video]}),
        )
        .await;
    assert_eq!(response.status(), 201);
    assert_eq!(push_number(2).await, "Video");
}

/// Retention sweeps a video message like any message: past
/// `retention_days` the message goes, its attachment row with it.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn retention_takes_a_video_message_like_any_other() {
    let server = spawn_server().await;
    let (owner, member, chat_id) = family_of_two(&server).await;
    let days = server.state.cfg.limits.retention_days;

    let old_clip = upload_round(&server, &owner, 0x31).await;
    let old: Value = send_round(&server, &owner, chat_id, old_clip)
        .await
        .json()
        .await
        .expect("JSON");
    let old_id = old["message"]["id"].as_i64().expect("id");
    let kept_clip = upload_round(&server, &member, 0x32).await;
    let kept: Value = send_round(&server, &member, chat_id, kept_clip)
        .await
        .json()
        .await
        .expect("JSON");
    let kept_id = kept["message"]["id"].as_i64().expect("id");

    sqlx::query("UPDATE messages SET created_at = now() - make_interval(days => $2) WHERE id = $1")
        .bind(old_id)
        .bind((days + 1) as i32)
        .execute(&server.state.pool)
        .await
        .expect("ageing the message");

    let swept = family_connect::handlers_chat::sweep_expired_messages(&server.state)
        .await
        .expect("the retention sweep runs");
    assert_eq!(swept, 1);

    let rows = |id: i64| {
        let pool = server.state.pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM attachments WHERE id = $1")
                .bind(id)
                .fetch_one(&pool)
                .await
                .expect("counting")
        }
    };
    assert_eq!(rows(old_clip).await, 0, "the swept message took its video");
    assert_eq!(rows(kept_clip).await, 1);
    let read = newest_message(&server, &owner, chat_id).await;
    assert_eq!(read["id"], kept_id);
    assert_eq!(read["attachments"][0]["round"], true);
}

// --- Editing ------------------------------------------------------------------------

/// A video message cannot be edited — and the question is asked LAST: the
/// body's own rules, `message_not_found` and `not_message_author` all
/// answer first, so it is only ever the author, with a body that would
/// otherwise have been accepted, who hears `validation`. Nothing moves: no
/// body, no `edit_seq`, no `message_edited` frame.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_video_message_cannot_be_edited_and_is_told_so_last() {
    let server = spawn_server().await;
    let (owner, member, chat_id) = family_of_two(&server).await;
    let member_id = server.user_id(&member).await;
    let mut member_ws = connect_ws(&server, &member).await;

    let clip = upload_round(&server, &owner, 0x41).await;
    let response = send_round(&server, &owner, chat_id, clip).await;
    assert_eq!(response.status(), 201);
    let sent: Value = response.json().await.expect("JSON");
    let round_id = sent["message"]["id"].as_i64().expect("id");
    next_frame_of_type(&mut member_ws, "message").await;
    let path = format!("/chats/{chat_id}/messages/{round_id}");

    // The author, with a body that would otherwise be accepted.
    assert_error(
        server.patch(&owner, &path, json!({"body": "hello"})).await,
        400,
        "validation",
    )
    .await;
    // The body's own rules answer first.
    assert_error(
        server.patch(&owner, &path, json!({"body": "   "})).await,
        400,
        "message_empty",
    )
    .await;
    // Somebody else is told what they would be told about any message.
    assert_error(
        server.patch(&member, &path, json!({"body": "hello"})).await,
        403,
        "not_message_author",
    )
    .await;
    // And the id named under ANOTHER chat is not found there, as any is.
    let direct: Value = server
        .post(&owner, "/chats/direct", json!({"user_id": member_id}))
        .await
        .json()
        .await
        .expect("JSON");
    let direct_id = direct["chat"]["id"].as_i64().expect("id");
    assert_error(
        server
            .patch(
                &owner,
                &format!("/chats/{direct_id}/messages/{round_id}"),
                json!({"body": "hello"}),
            )
            .await,
        404,
        "message_not_found",
    )
    .await;

    // Nothing moved: no body, no edit mark, no seq, still a video message.
    // (0 is "never edited": the column is NOT NULL DEFAULT 0, 0007.)
    let (body, edit_seq): (String, i64) =
        sqlx::query_as("SELECT body, edit_seq FROM messages WHERE id = $1")
            .bind(round_id)
            .fetch_one(&server.state.pool)
            .await
            .expect("reading the row");
    assert_eq!(body, "");
    assert_eq!(edit_seq, 0, "a refused edit takes no edit_seq");
    let read = newest_message(&server, &member, chat_id).await;
    assert_eq!(read["id"], round_id);
    assert!(read.get("edited_at").is_none_or(Value::is_null));
    assert_eq!(read["attachments"][0]["round"], true);

    // No `message_edited` reached the member: the next frame after a ping
    // is its pong, with nothing in between.
    member_ws
        .send(Message::text(json!({"type": "ping"}).to_string()))
        .await
        .expect("sending ping");
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let message = tokio::time::timeout_at(deadline, member_ws.next())
            .await
            .expect("a frame before the deadline")
            .expect("socket open")
            .expect("socket fine");
        if let Message::Text(text) = message {
            let value: Value = serde_json::from_str(text.as_str()).expect("JSON");
            assert_ne!(value["type"], "message_edited", "{value}");
            if value["type"] == "pong" {
                break;
            }
        }
    }

    // The SAME clip sent as an ordinary video with a caption is a video,
    // and its caption is the author's to rewrite as it always was.
    let video = upload_round(&server, &owner, 0x42).await;
    let response = server
        .post(
            &owner,
            &format!("/chats/{chat_id}/messages"),
            json!({"client_msg_id": Uuid::new_v4().to_string(), "body": "look",
                   "attachment_ids": [video]}),
        )
        .await;
    assert_eq!(response.status(), 201);
    let sent: Value = response.json().await.expect("JSON");
    let video_id = sent["message"]["id"].as_i64().expect("id");
    let edited = server
        .patch(
            &owner,
            &format!("/chats/{chat_id}/messages/{video_id}"),
            json!({"body": "look at this"}),
        )
        .await;
    assert_eq!(edited.status(), 200);
    let edited: Value = edited.json().await.expect("JSON");
    assert_eq!(edited["message"]["body"], "look at this");
}
