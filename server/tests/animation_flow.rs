//! Integration: an ANIMATION — an animated picture the sender turned into a
//! silent looping video, drawn as a picture that moves (protocol.md,
//! "Animations"; issue #96).
//!
//! Modelled on round_flow.rs, because the wire is the video message's
//! pattern: an ordinary `kind=video` attachment with one flag, `animation`,
//! set by the send, stored on the attachment and carried by every read. What
//! is under test is that flag on every read path and ABSENT on an ordinary
//! video; the four checks of "What the server checks", each a 400 that
//! leaves the upload unclaimed and unflagged; the two ways an animation is
//! NOT a video message — a caption is allowed, and so is an edit, which
//! keeps the flag — its size held only to `max_attachment_bytes`, with no
//! discovery key; and the push word.

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

/// The upload protocol.md's example makes: a 480 x 270 clip of 2.1 s.
const ANIMATION_QUERY: &str = "kind=video&width=480&height=270&duration_ms=2100";

// --- Minimal WebSocket client (mirrors the helpers in round_flow.rs) --------

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

/// A GIF by its magic number: what an animation WAS before the sender
/// turned it into a video — and, sent as itself, never one. This server
/// takes a GIF only as a FILE (it is no photo type here), which is exactly
/// how a still or unconvertible one still goes (protocol.md, "The file").
fn gif_bytes(len: usize, seed: u8) -> Vec<u8> {
    let mut bytes = b"GIF89a".to_vec();
    bytes.resize(len.max(6), seed);
    bytes
}

fn jpeg_bytes(len: usize, seed: u8) -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE0];
    bytes.resize(len.max(4), seed);
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

/// Upload raw bytes with any query and type; returns the attachment id.
async fn upload_as(ts: &TestServer, token: &str, query: &str, mime: &str, bytes: Vec<u8>) -> i64 {
    let response = ts
        .put_bytes_method("POST", token, &format!("/attachments?{query}"), mime, bytes)
        .await;
    assert_eq!(response.status(), 201, "uploading {query} as {mime:?}");
    let body: Value = response.json().await.expect("JSON");
    body["attachment"]["id"].as_i64().expect("attachment id")
}

/// Upload a clip as a client that converted an animated GIF does.
async fn upload_animation(ts: &TestServer, token: &str, seed: u8) -> i64 {
    upload_as(
        ts,
        token,
        ANIMATION_QUERY,
        "video/mp4",
        mp4_bytes(512, seed),
    )
    .await
}

/// Send one attachment as an ANIMATION over REST, with a caption or none.
async fn send_animation(
    ts: &TestServer,
    token: &str,
    chat_id: i64,
    attachment_id: i64,
    body: &str,
) -> reqwest::Response {
    ts.post(
        token,
        &format!("/chats/{chat_id}/messages"),
        json!({
            "client_msg_id": Uuid::new_v4().to_string(),
            "body": body,
            "attachment_ids": [attachment_id],
            "animation": true,
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
        "SELECT count(*) FROM attachments
         WHERE message_id IS NOT NULL OR animation OR round OR sticker",
    )
    .fetch_one(&ts.state.pool)
    .await
    .expect("counting")
}

// --- The flag on every read ----------------------------------------------------

/// The round trip: a message sent with `animation: true` — and a caption,
/// which an animation may carry — has the flag on its attachment on every
/// read: the answer, the frame, a page of history, the legacy field, the
/// chat list, a thread, a retry. An ordinary video, the same MP4, never
/// carries the key at all.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_animation_carries_the_flag_and_an_ordinary_video_does_not() {
    let server = spawn_server().await;
    let (owner, member, chat_id) = family_of_two(&server).await;
    let mut member_ws = connect_ws(&server, &member).await;

    let clip = upload_animation(&server, &owner, 0xA1).await;
    let response = send_animation(&server, &owner, chat_id, clip, "when the pizza arrives").await;
    assert_eq!(response.status(), 201);
    let sent: Value = response.json().await.expect("JSON");
    let message = &sent["message"];
    let message_id = message["id"].as_i64().expect("id");
    assert_eq!(message["body"], "when the pizza arrives");
    assert_eq!(message["attachments"].as_array().expect("list").len(), 1);
    let attachment = &message["attachments"][0];
    assert_eq!(attachment["id"], clip);
    assert_eq!(attachment["kind"], "video");
    assert_eq!(attachment["mime"], "video/mp4");
    assert_eq!(attachment["width"], 480);
    assert_eq!(attachment["height"], 270);
    assert_eq!(attachment["duration_ms"], 2100);
    assert_eq!(attachment["animation"], true);
    assert!(attachment.get("round").is_none(), "{attachment}");
    assert!(attachment.get("sticker").is_none(), "{attachment}");
    assert_eq!(
        message["attachment"], message["attachments"][0],
        "the legacy field carries it too"
    );

    // The frame every other member gets.
    let frame = next_frame_of_type(&mut member_ws, "message").await;
    assert_eq!(frame["message"]["id"], message_id);
    assert_eq!(frame["message"]["attachments"][0]["animation"], true);
    assert_eq!(frame["message"]["attachment"]["animation"], true);

    // A page of history, read by the member.
    let read = newest_message(&server, &member, chat_id).await;
    assert_eq!(read["id"], message_id);
    assert_eq!(read["body"], "when the pizza arrives");
    assert_eq!(read["attachments"][0]["animation"], true);
    assert_eq!(read["attachment"]["animation"], true);

    // The chat-list preview, so a row can say "GIF".
    let last = last_message(&server, &member, chat_id).await;
    assert_eq!(last["id"], message_id);
    assert_eq!(last["attachments"][0]["animation"], true);
    assert_eq!(last["attachment"]["animation"], true);

    // A thread: a reply makes it a root, and the thread read carries it.
    let reply = server
        .post(
            &member,
            &format!("/chats/{chat_id}/messages"),
            json!({"client_msg_id": Uuid::new_v4().to_string(), "body": "Ha!",
                   "reply_to_message_id": message_id}),
        )
        .await;
    assert_eq!(reply.status(), 201);
    // A REST send reaches every connection, the sender's own included.
    next_frame_of_type(&mut member_ws, "message").await;
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
    assert_eq!(thread["messages"][0]["attachments"][0]["animation"], true);

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
            json!({"client_msg_id": client_msg_id, "body": "when the pizza arrives",
                   "attachment_ids": [clip], "animation": true}),
        )
        .await;
    assert_eq!(retry.status(), 200);
    let retry: Value = retry.json().await.expect("JSON");
    assert_eq!(retry["message"]["id"], message_id);
    assert_eq!(retry["message"]["attachments"][0]["animation"], true);

    // A caption-less one is an animation too.
    let bare = upload_animation(&server, &owner, 0xA5).await;
    let response = send_animation(&server, &owner, chat_id, bare, "").await;
    assert_eq!(response.status(), 201);
    let sent: Value = response.json().await.expect("JSON");
    assert_eq!(sent["message"]["body"], "");
    assert_eq!(sent["message"]["attachments"][0]["animation"], true);
    next_frame_of_type(&mut member_ws, "message").await;

    // THE SAME MP4 AS AN ORDINARY VIDEO: no flag, and not `false` either —
    // absent, by the wire's usual rule. `animation: false` is the same
    // statement as leaving it out.
    for (seed, body) in [
        (0xA2, json!({"body": ""})),
        (0xA3, json!({"body": "", "animation": false})),
        (0xA4, json!({"body": "look at this", "animation": false})),
    ] {
        let video = upload_animation(&server, &owner, seed).await;
        let mut request = body.clone();
        request["client_msg_id"] = json!(Uuid::new_v4().to_string());
        request["attachment_ids"] = json!([video]);
        let response = server
            .post(&owner, &format!("/chats/{chat_id}/messages"), request)
            .await;
        assert_eq!(response.status(), 201, "{body}");
        let sent: Value = response.json().await.expect("JSON");
        assert!(
            sent["message"]["attachments"][0].get("animation").is_none(),
            "an ordinary video carries no animation key: {sent}"
        );
        assert!(sent["message"]["attachment"].get("animation").is_none());
        let frame = next_frame_of_type(&mut member_ws, "message").await;
        assert!(
            frame["message"]["attachments"][0]
                .get("animation")
                .is_none()
        );
        let read = newest_message(&server, &member, chat_id).await;
        assert_eq!(read["id"], sent["message"]["id"]);
        assert!(read["attachments"][0].get("animation").is_none());
    }
    let last = last_message(&server, &member, chat_id).await;
    assert!(last["attachments"][0].get("animation").is_none(), "{last}");
    // And the rows say so too: only the two animations are flagged.
    let flagged: i64 = sqlx::query_scalar("SELECT count(*) FROM attachments WHERE animation")
        .fetch_one(&server.state.pool)
        .await
        .expect("counting");
    assert_eq!(flagged, 2);
}

/// The socket sends one the same way: `animation: true` on the `send`
/// frame, the ack carries the flag back, and a refusal is an error frame
/// naming the send, with nothing landed.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_animation_is_sent_over_the_socket_too() {
    let server = spawn_server().await;
    let (owner, member, chat_id) = family_of_two(&server).await;
    let mut owner_ws = connect_ws(&server, &owner).await;
    let mut member_ws = connect_ws(&server, &member).await;

    let clip = upload_animation(&server, &owner, 0xB1).await;
    let client_msg_id = Uuid::new_v4().to_string();
    owner_ws
        .send(Message::text(
            json!({"type": "send", "chat_id": chat_id, "client_msg_id": client_msg_id,
                   "body": "me on Mondays", "attachment_ids": [clip], "animation": true})
            .to_string(),
        ))
        .await
        .expect("sending");
    let ack = next_frame_of_type(&mut owner_ws, "ack").await;
    assert_eq!(ack["client_msg_id"], client_msg_id);
    assert_eq!(ack["message"]["body"], "me on Mondays");
    assert_eq!(ack["message"]["attachments"][0]["animation"], true);
    assert_eq!(ack["message"]["attachments"][0]["kind"], "video");
    let frame = next_frame_of_type(&mut member_ws, "message").await;
    assert_eq!(frame["message"]["attachments"][0]["animation"], true);

    // Refusals come back as error frames naming the send: beside `round`
    // (`validation`), and a GIF that was never turned into a video
    // (`invalid_attachment`, from the check after the claim).
    let beside = upload_animation(&server, &owner, 0xB2).await;
    let gif = upload_as(
        &server,
        &owner,
        "kind=file&name=cat.gif&width=480&height=270",
        "image/gif",
        gif_bytes(256, 0xB3),
    )
    .await;
    for (request, code) in [
        (
            json!({"body": "", "attachment_ids": [beside], "animation": true, "round": true}),
            "validation",
        ),
        (
            json!({"body": "", "attachment_ids": [gif], "animation": true}),
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
         WHERE id = ANY($1) AND message_id IS NULL AND NOT animation AND NOT round",
    )
    .bind(vec![beside, gif])
    .fetch_one(&server.state.pool)
    .await
    .expect("counting");
    assert_eq!(free, 2, "both refused uploads are still free and unflagged");
}

// --- What the server refuses -----------------------------------------------------

/// Checks 1 to 4 of "Animations" — "What the server checks", each a 400 —
/// never a 500 an outbox would retry for ever — and each leaving NOTHING
/// behind: no message, every upload unclaimed and unflagged. Then each
/// upload is still good for what it is, and the edges of the declared shape
/// are accepted.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_ways_of_sending_an_animation_wrong() {
    let server = spawn_server().await;
    let (owner, _, chat_id) = family_of_two(&server).await;
    let path = format!("/chats/{chat_id}/messages");
    let send = |body: Value| {
        let mut request = body;
        request["client_msg_id"] = json!(Uuid::new_v4().to_string());
        server.post(&owner, &path, request)
    };

    let good = upload_animation(&server, &owner, 0xC1).await;
    let other = upload_animation(&server, &owner, 0xC2).await;

    // 1. Beside a poll — with or without a video named.
    assert_error(
        send(json!({"body": "Pizza?", "poll": {"options": ["Yes", "No"]}, "animation": true}))
            .await,
        400,
        "invalid_poll",
    )
    .await;
    assert_error(
        send(json!({"body": "Pizza?", "poll": {"options": ["Yes", "No"]},
                    "attachment_ids": [good], "animation": true}))
        .await,
        400,
        "invalid_poll",
    )
    .await;
    // 2. Beside `sticker` or `round` — or both: `validation`, whatever else
    // the send got wrong, and asked before any id is read. A caption and two
    // attachments, each of which would be refused for its own reason, do
    // not change the answer.
    for flags in [
        json!({"sticker": true}),
        json!({"round": true}),
        json!({"sticker": true, "round": true}),
    ] {
        for (body, ids) in [
            ("", json!([good])),
            ("a caption", json!([good])),
            ("", json!([good, other])),
            ("", json!([])),
            // An id nobody uploaded: never read, so never `attachment_not_found`.
            ("", json!([987_654_321])),
        ] {
            let mut request = json!({"body": body, "attachment_ids": ids, "animation": true});
            for (key, value) in flags.as_object().expect("flags") {
                request[key] = value.clone();
            }
            assert_error(send(request.clone()).await, 400, "validation").await;
        }
    }
    // 3. Not exactly one attachment: none, and two — in either spelling.
    assert_error(
        send(json!({"body": "", "animation": true})).await,
        400,
        "invalid_attachment",
    )
    .await;
    assert_error(
        send(json!({"body": "", "attachment_ids": [], "animation": true})).await,
        400,
        "invalid_attachment",
    )
    .await;
    assert_error(
        send(json!({"body": "", "attachment_ids": [good, other], "animation": true})).await,
        400,
        "invalid_attachment",
    )
    .await;

    // 4. After the claim — the WRONG KIND, each refused as
    // `invalid_attachment` with nothing claimed or flagged. The claim writes
    // `animation = ($7 AND kind = 'video')`, so none of these can reach
    // 0055's CHECK: a 500 here would be the bug this test exists for.
    let gif = upload_as(
        &server,
        &owner,
        "kind=file&name=cat.gif&width=480&height=270",
        "image/gif",
        gif_bytes(256, 0xC3),
    )
    .await;
    let photo = upload_as(
        &server,
        &owner,
        "kind=photo&width=480&height=270",
        "image/jpeg",
        jpeg_bytes(256, 0xC4),
    )
    .await;
    let audio = upload_as(
        &server,
        &owner,
        "kind=audio&duration_ms=2100",
        "audio/mp4",
        mp4_bytes(256, 0xC5),
    )
    .await;
    // A FILE that says it is an MP4, sized and timed: still a file.
    let file = upload_as(
        &server,
        &owner,
        "kind=file&name=cat.mp4&width=480&height=270&duration_ms=2100",
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
        ANIMATION_QUERY,
        "video/quicktime",
        mp4_bytes(256, 0xC7),
    )
    .await;
    // The wrong declared shape or length.
    let mut wrong_shapes = Vec::new();
    for (seed, query) in [
        (0xD0, "kind=video&height=270&duration_ms=2100"),
        (0xD1, "kind=video&width=480&duration_ms=2100"),
        (0xD2, "kind=video&duration_ms=2100"),
        (0xD3, "kind=video&width=0&height=270&duration_ms=2100"),
        (0xD4, "kind=video&width=480&height=0&duration_ms=2100"),
        (0xD5, "kind=video&width=-480&height=-270&duration_ms=2100"),
        // A short side over 720, either way round, and a square over it.
        (0xD6, "kind=video&width=1280&height=721&duration_ms=2100"),
        (0xD7, "kind=video&width=721&height=1280&duration_ms=2100"),
        (0xD8, "kind=video&width=721&height=721&duration_ms=2100"),
        (0xD9, "kind=video&width=480&height=270"),
        (0xDA, "kind=video&width=480&height=270&duration_ms=0"),
        (0xDB, "kind=video&width=480&height=270&duration_ms=-1"),
    ] {
        let id = upload_as(&server, &owner, query, "video/mp4", mp4_bytes(256, seed)).await;
        wrong_shapes.push((id, query));
    }
    let mut refused: Vec<(i64, String)> = vec![
        (gif, "a GIF file".to_string()),
        (photo, "a JPEG photo".to_string()),
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
        // With a caption and without: the caption is never the reason.
        for body in ["", "a caption"] {
            let response = send_animation(&server, &owner, chat_id, *id, body).await;
            assert_eq!(
                response.status(),
                400,
                "{what} sent as an animation must be a 400, never a 500"
            );
            let answer: Value = response.json().await.expect("JSON");
            assert_eq!(
                answer["error"]["code"], "invalid_attachment",
                "{what}: {answer}"
            );
        }
    }

    // NOTHING LANDED. The chat is empty, and every refused upload is still
    // unclaimed and unflagged — the refusal rolled the claim back.
    let page: Value = server.get(&owner, &path).await.json().await.expect("JSON");
    assert_eq!(page["messages"], json!([]));
    assert_eq!(
        claimed_or_flagged(&server).await,
        0,
        "a refused animation claimed or flagged something"
    );

    // So each is still good for what it IS: the GIF as the picture it is,
    // the photo, the audio, the file, the location, the QuickTime clip and
    // every odd shape as the ordinary thing it is — and the good clip as an
    // animation.
    for (id, what) in &refused {
        assert_eq!(
            send(json!({"body": "", "attachment_ids": [id]}))
                .await
                .status(),
            201,
            "{what} as an ordinary message"
        );
    }
    let response = send_animation(&server, &owner, chat_id, good, "").await;
    assert_eq!(response.status(), 201);

    // THE EDGES ARE IN: a 1 x 1 of 1 ms, a short side of exactly 720 either
    // way round with a long side far past it, a 720 square, and a long
    // animation — nothing caps the length. Declared, never measured.
    for (seed, query) in [
        (0xE0, "kind=video&width=1&height=1&duration_ms=1"),
        (0xE1, "kind=video&width=1280&height=720&duration_ms=2100"),
        (0xE2, "kind=video&width=720&height=1280&duration_ms=2100"),
        (0xE3, "kind=video&width=720&height=720&duration_ms=2100"),
        (0xE4, "kind=video&width=4000&height=2&duration_ms=600000"),
    ] {
        let id = upload_as(&server, &owner, query, "video/mp4", mp4_bytes(256, seed)).await;
        let response = send_animation(&server, &owner, chat_id, id, "").await;
        assert_eq!(response.status(), 201, "{query}");
        let sent: Value = response.json().await.expect("JSON");
        assert_eq!(
            sent["message"]["attachments"][0]["animation"], true,
            "{query}"
        );
    }
    // The legacy single spelling names one attachment just as well.
    let legacy = upload_animation(&server, &owner, 0xE5).await;
    let response = send(json!({"body": "", "attachment_id": legacy, "animation": true})).await;
    assert_eq!(response.status(), 201);
    let sent: Value = response.json().await.expect("JSON");
    assert_eq!(sent["message"]["attachment"]["animation"], true);
}

/// Its size is held only to `max_attachment_bytes`, as any video's is — not
/// to the video message's ceiling, however low that is set — and no
/// discovery key announces animations: a server that predates them simply
/// delivers an ordinary video (protocol.md, "What old clients and old
/// servers do").
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_animation_is_held_only_to_the_attachment_ceiling_and_needs_no_key() {
    let server = spawn_server_with_config(|cfg| {
        cfg.limits.max_round_video_bytes = Some(8 * 1024);
    })
    .await;
    let (owner, _, chat_id) = family_of_two(&server).await;
    let ceiling = server.state.cfg.limits.max_attachment_bytes;
    assert!(ceiling > 8 * 1024);

    // Exactly at the attachment ceiling, far past the video message's.
    let big = upload_as(
        &server,
        &owner,
        ANIMATION_QUERY,
        "video/mp4",
        mp4_bytes(ceiling, 0xF0),
    )
    .await;
    let response = send_animation(&server, &owner, chat_id, big, "").await;
    assert_eq!(response.status(), 201);
    let sent: Value = response.json().await.expect("JSON");
    assert_eq!(sent["message"]["attachments"][0]["size"], ceiling as u64);
    assert_eq!(sent["message"]["attachments"][0]["animation"], true);

    // No key on the family read names animations.
    let family: Value = server
        .get(&owner, "/families/mine")
        .await
        .json()
        .await
        .expect("JSON");
    assert!(!family.to_string().contains("animation"), "{family}");
}

// --- Editing ------------------------------------------------------------------------

/// An animation CAN be edited, unlike a video message: its caption is the
/// author's to rewrite as any message's is — even a caption-less one may be
/// given one — and the flag and the video never change. The edit answer,
/// the `message_edited` frame, the edits feed and a page of history all
/// carry the flag.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_animation_can_be_edited_and_keeps_its_flag() {
    let server = spawn_server().await;
    let (owner, member, chat_id) = family_of_two(&server).await;
    let mut member_ws = connect_ws(&server, &member).await;

    let clip = upload_animation(&server, &owner, 0x41).await;
    let response = send_animation(&server, &owner, chat_id, clip, "monday").await;
    assert_eq!(response.status(), 201);
    let sent: Value = response.json().await.expect("JSON");
    let message_id = sent["message"]["id"].as_i64().expect("id");
    next_frame_of_type(&mut member_ws, "message").await;
    let path = format!("/chats/{chat_id}/messages/{message_id}");

    let edited = server
        .patch(&owner, &path, json!({"body": "me on Mondays"}))
        .await;
    assert_eq!(edited.status(), 200);
    let edited: Value = edited.json().await.expect("JSON");
    assert_eq!(edited["message"]["body"], "me on Mondays");
    assert_eq!(edited["message"]["attachments"][0]["id"], clip);
    assert_eq!(edited["message"]["attachments"][0]["animation"], true);
    assert_eq!(edited["message"]["attachment"]["animation"], true);

    let frame = next_frame_of_type(&mut member_ws, "message_edited").await;
    assert_eq!(frame["message"]["id"], message_id);
    assert_eq!(frame["message"]["body"], "me on Mondays");
    assert_eq!(frame["message"]["attachments"][0]["animation"], true);

    let edits: Value = server
        .get(&member, &format!("/chats/{chat_id}/edits?after_seq=0"))
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(edits["messages"][0]["id"], message_id);
    assert_eq!(edits["messages"][0]["body"], "me on Mondays");
    assert_eq!(edits["messages"][0]["attachments"][0]["animation"], true);

    let read = newest_message(&server, &member, chat_id).await;
    assert_eq!(read["id"], message_id);
    assert_eq!(read["body"], "me on Mondays");
    assert!(read["edited_at"].is_string(), "{read}");
    assert_eq!(read["attachments"][0]["animation"], true);

    // A caption-less animation may be given a caption by an edit.
    let bare = upload_animation(&server, &owner, 0x42).await;
    let sent: Value = send_animation(&server, &owner, chat_id, bare, "")
        .await
        .json()
        .await
        .expect("JSON");
    let bare_id = sent["message"]["id"].as_i64().expect("id");
    let edited = server
        .patch(
            &owner,
            &format!("/chats/{chat_id}/messages/{bare_id}"),
            json!({"body": "the cat, again"}),
        )
        .await;
    assert_eq!(edited.status(), 200);
    let edited: Value = edited.json().await.expect("JSON");
    assert_eq!(edited["message"]["body"], "the cat, again");
    assert_eq!(edited["message"]["attachments"][0]["animation"], true);

    // Somebody else is told what they would be told about any message.
    assert_error(
        server
            .patch(&member, &path, json!({"body": "mine now"}))
            .await,
        403,
        "not_message_author",
    )
    .await;

    // The rows: still the same two videos, still flagged.
    let flagged: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM attachments
         WHERE id = ANY($1) AND animation AND kind = 'video' AND message_id IS NOT NULL",
    )
    .bind(vec![clip, bare])
    .fetch_one(&server.state.pool)
    .await
    .expect("counting");
    assert_eq!(flagged, 2);
}

// --- The push ------------------------------------------------------------------------

/// A push for a caption-less animation says "GIF", where a video says
/// "Video" — written by the server, so every installed app shows it — and
/// one with a caption says the caption, which is the message's own words.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_animation_pushes_the_word_gif() {
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

    let clip = upload_animation(&server, &owner, 0x21).await;
    assert_eq!(
        send_animation(&server, &owner, chat_id, clip, "")
            .await
            .status(),
        201
    );
    assert_eq!(push_number(1).await, "GIF");

    // With a caption, the caption.
    let captioned = upload_animation(&server, &owner, 0x22).await;
    assert_eq!(
        send_animation(&server, &owner, chat_id, captioned, "me on Mondays")
            .await
            .status(),
        201
    );
    assert_eq!(push_number(2).await, "me on Mondays");

    // The same clip as an ordinary video is still "Video".
    let video = upload_animation(&server, &owner, 0x23).await;
    let response = server
        .post(
            &owner,
            &format!("/chats/{chat_id}/messages"),
            json!({"client_msg_id": Uuid::new_v4().to_string(), "body": "",
                   "attachment_ids": [video]}),
        )
        .await;
    assert_eq!(response.status(), 201);
    assert_eq!(push_number(3).await, "Video");
}
