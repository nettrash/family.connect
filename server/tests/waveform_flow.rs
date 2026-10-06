//! Integration: a voice note's WAVEFORM (protocol.md, "A voice note's
//! waveform") — 48 lowercase hex digits the sender computes and sends on
//! `POST /attachments?kind=audio&…&waveform=…`, which the server checks for
//! form, stores (0053) and echoes on the Attachment, and never computes.
//!
//! What is under test: the round trip on every read that carries an
//! attachment whole; ABSENT — the key, not an empty string — when none was
//! given and on every other kind, so the old shape is exactly the old
//! shape; the refusals, each a `validation` 400 that stores nothing; and
//! 0053's CHECK, the half no request can reach.

mod common;

use std::time::Duration;

use common::{TestServer, assert_error, spawn_server};
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

const WAIT: Duration = Duration::from_secs(5);

/// protocol.md's example: a 14.2 s voice note's shape.
const SHAPE: &str = "0124689abcddeeedcba987654321001245678aabbba98642";

// --- Minimal WebSocket client (mirrors round_flow.rs) ---------------------------

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

/// An ISO base media file by its magic number and nothing more — all the
/// server ever looks at. `seed` fills the rest, so two clips are two files.
fn m4a_bytes(len: usize, seed: u8) -> Vec<u8> {
    let mut bytes = vec![0x00, 0x00, 0x00, 0x18, b'f', b't', b'y', b'p'];
    bytes.extend_from_slice(b"M4A ");
    bytes.resize(len.max(12), seed);
    bytes
}

fn jpeg_bytes(len: usize, seed: u8) -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xE0];
    bytes.resize(len.max(4), seed);
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

async fn upload_raw(
    ts: &TestServer,
    token: &str,
    query: &str,
    mime: &str,
    bytes: Vec<u8>,
) -> reqwest::Response {
    ts.put_bytes_method("POST", token, &format!("/attachments?{query}"), mime, bytes)
        .await
}

/// Upload and return the attachment the server answered with.
async fn upload(ts: &TestServer, token: &str, query: &str, mime: &str, bytes: Vec<u8>) -> Value {
    let response = upload_raw(ts, token, query, mime, bytes).await;
    assert_eq!(response.status(), 201, "uploading {query} as {mime:?}");
    let body: Value = response.json().await.expect("JSON");
    body["attachment"].clone()
}

async fn send(ts: &TestServer, token: &str, chat_id: i64, attachment_id: i64) -> Value {
    let response = ts
        .post(
            token,
            &format!("/chats/{chat_id}/messages"),
            json!({"client_msg_id": Uuid::new_v4().to_string(), "body": "",
                   "attachment_ids": [attachment_id]}),
        )
        .await;
    assert_eq!(response.status(), 201);
    let body: Value = response.json().await.expect("JSON");
    body["message"].clone()
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

async fn attachment_rows(ts: &TestServer) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM attachments")
        .fetch_one(&ts.state.pool)
        .await
        .expect("counting")
}

fn keys(attachment: &Value) -> Vec<String> {
    let mut keys: Vec<String> = attachment
        .as_object()
        .expect("an attachment is an object")
        .keys()
        .cloned()
        .collect();
    keys.sort();
    keys
}

// --- The round trip -----------------------------------------------------------

/// The waveform an uploader sent comes back on the upload's answer, the
/// send's, the frame every member gets, a page of history (both fields), a
/// thread and the edits feed — byte for byte the string that went up.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_voice_notes_waveform_round_trips_on_every_read() {
    let server = spawn_server().await;
    let (owner, member, chat_id) = family_of_two(&server).await;
    let mut member_ws = connect_ws(&server, &member).await;

    let uploaded = upload(
        &server,
        &owner,
        &format!("kind=audio&duration_ms=14200&waveform={SHAPE}"),
        "audio/mp4",
        m4a_bytes(512, 0x11),
    )
    .await;
    assert_eq!(uploaded["kind"], "audio");
    assert_eq!(uploaded["waveform"], SHAPE, "{uploaded}");
    assert_eq!(uploaded["has_preview"], false);
    let id = uploaded["id"].as_i64().expect("id");

    let message = send(&server, &owner, chat_id, id).await;
    let message_id = message["id"].as_i64().expect("id");
    assert_eq!(message["attachments"][0]["waveform"], SHAPE);
    assert_eq!(message["attachment"]["waveform"], SHAPE);
    assert_eq!(message["attachments"][0]["duration_ms"], 14200);

    let frame = next_frame_of_type(&mut member_ws, "message").await;
    assert_eq!(frame["message"]["id"], message_id);
    assert_eq!(frame["message"]["attachments"][0]["waveform"], SHAPE);
    assert_eq!(frame["message"]["attachment"]["waveform"], SHAPE);

    let read = newest_message(&server, &member, chat_id).await;
    assert_eq!(read["id"], message_id);
    assert_eq!(read["attachments"][0]["waveform"], SHAPE);
    assert_eq!(read["attachment"]["waveform"], SHAPE);

    // The chat list's preview draws no bubble, and carries no more of the
    // shape than of the duration.
    let chats: Value = server
        .get(&member, "/chats")
        .await
        .json()
        .await
        .expect("JSON");
    let last = chats["chats"]
        .as_array()
        .expect("chats")
        .iter()
        .find(|entry| entry["chat"]["id"] == chat_id)
        .expect("listed")["last_message"]
        .clone();
    assert_eq!(last["id"], message_id);
    assert_eq!(last["attachments"][0]["kind"], "audio");
    assert!(last["attachments"][0].get("waveform").is_none(), "{last}");

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
    assert_eq!(thread["messages"][0]["attachments"][0]["waveform"], SHAPE);

    // The edits feed: only the database can put an audio message there,
    // which is the point — whatever reaches it is hydrated by the same
    // reader and must carry the waveform.
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
    assert_eq!(edits["messages"][0]["attachments"][0]["waveform"], SHAPE);

    // Stored as given.
    let stored: Option<String> =
        sqlx::query_scalar("SELECT waveform FROM attachments WHERE id = $1")
            .bind(id)
            .fetch_one(&server.state.pool)
            .await
            .expect("reading the row");
    assert_eq!(stored.as_deref(), Some(SHAPE));
}

/// Without one — a picked sound file, an old client — the key is ABSENT,
/// never `""` or `null`, and the attachment is exactly the shape it always
/// was. The same for every other kind. And a waveform belongs to its own
/// row: identical bytes uploaded twice, once with one, deduplicate the
/// FILE but not the shape.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn without_a_waveform_the_attachment_is_the_old_shape() {
    let server = spawn_server().await;
    let (owner, member, chat_id) = family_of_two(&server).await;

    let plain = upload(
        &server,
        &owner,
        "kind=audio&duration_ms=9000",
        "audio/mp4",
        m4a_bytes(512, 0x21),
    )
    .await;
    assert_eq!(
        keys(&plain),
        ["duration_ms", "has_preview", "id", "kind", "mime", "size"],
        "{plain}"
    );
    let named = upload(
        &server,
        &owner,
        "kind=audio&duration_ms=9000&name=song.mp3",
        "audio/mpeg",
        b"ID3\x03\x00\x00\x00\x00\x00\x00rest".to_vec(),
    )
    .await;
    assert!(named.get("waveform").is_none(), "{named}");

    let message = send(&server, &owner, chat_id, plain["id"].as_i64().expect("id")).await;
    assert!(message["attachments"][0].get("waveform").is_none());
    let read = newest_message(&server, &member, chat_id).await;
    assert_eq!(read["attachments"][0], plain, "history reads the old shape");

    let photo = upload(
        &server,
        &owner,
        "kind=photo",
        "image/jpeg",
        jpeg_bytes(256, 1),
    )
    .await;
    let video = upload(
        &server,
        &owner,
        "kind=video&width=480&height=480&duration_ms=3000",
        "video/mp4",
        m4a_bytes(512, 0x22),
    )
    .await;
    let file = upload(
        &server,
        &owner,
        "kind=file&name=notes.txt",
        "text/plain",
        b"hello".to_vec(),
    )
    .await;
    let place = upload(
        &server,
        &owner,
        "kind=location&latitude=55.7558&longitude=37.6173",
        "",
        Vec::new(),
    )
    .await;
    for other in [&photo, &video, &file, &place] {
        assert!(other.get("waveform").is_none(), "{other}");
    }

    // Identical bytes, twice: the second points at the first's file, and
    // each row keeps its own (non-)waveform.
    let first = upload(
        &server,
        &owner,
        "kind=audio&duration_ms=5000",
        "audio/mp4",
        m4a_bytes(700, 0x23),
    )
    .await;
    let second = upload(
        &server,
        &owner,
        &format!("kind=audio&duration_ms=5000&waveform={SHAPE}"),
        "audio/mp4",
        m4a_bytes(700, 0x23),
    )
    .await;
    assert!(first.get("waveform").is_none());
    assert_eq!(second["waveform"], SHAPE);
    let keys_of: Vec<String> =
        sqlx::query_scalar("SELECT storage_key FROM attachments WHERE id = ANY($1) ORDER BY id")
            .bind(vec![
                first["id"].as_i64().expect("id"),
                second["id"].as_i64().expect("id"),
            ])
            .fetch_all(&server.state.pool)
            .await
            .expect("reading keys");
    assert_eq!(keys_of[0], keys_of[1], "the bytes were deduplicated");
}

// --- The refusals ---------------------------------------------------------------

/// On audio, anything but 48 lowercase hex digits is `validation` (400) —
/// and nothing is stored: no row, no file.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_malformed_waveform_on_audio_is_refused() {
    let server = spawn_server().await;
    let (owner, _member, _chat_id) = family_of_two(&server).await;
    let before = attachment_rows(&server).await;

    let refused = [
        String::new(),
        "0".repeat(47),
        "0".repeat(49),
        "0".repeat(96),
        SHAPE.to_uppercase(),
        format!("{}F", &SHAPE[..47]),
        format!("{}g", &SHAPE[..47]),
        format!("{}%20", &SHAPE[..47]),
        format!("{}%2C", &SHAPE[..47]),
        format!("{}-", &SHAPE[..47]),
        // 48 characters, 49 bytes.
        format!("{}%C3%A9", &SHAPE[..47]),
    ];
    for value in &refused {
        let response = upload_raw(
            &server,
            &owner,
            &format!("kind=audio&duration_ms=1000&waveform={value}"),
            "audio/mp4",
            m4a_bytes(256, 0x31),
        )
        .await;
        assert_error(response, 400, "validation").await;
    }
    // And when `kind` is left out: the server derives audio from the type,
    // and holds it to the same rule.
    let response = upload_raw(
        &server,
        &owner,
        "duration_ms=1000&waveform=abc",
        "audio/mp4",
        m4a_bytes(256, 0x32),
    )
    .await;
    assert_error(response, 400, "validation").await;

    assert_eq!(attachment_rows(&server).await, before, "nothing was stored");
}

/// On anything that is not audio — a photo, a video, a video message's
/// square clip, a file, a location — a waveform, even a well-formed one, is
/// `validation` (400), and nothing is stored.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_waveform_on_any_other_kind_is_refused() {
    let server = spawn_server().await;
    let (owner, _member, _chat_id) = family_of_two(&server).await;
    let before = attachment_rows(&server).await;

    let cases: [(&str, &str, Vec<u8>); 6] = [
        ("kind=photo", "image/jpeg", jpeg_bytes(256, 2)),
        // No `kind`: a JPEG is a photo whatever the query omits.
        ("", "image/jpeg", jpeg_bytes(256, 3)),
        (
            "kind=video&width=1280&height=720&duration_ms=3000",
            "video/mp4",
            m4a_bytes(256, 0x41),
        ),
        (
            "kind=video&width=480&height=480&duration_ms=3000",
            "video/mp4",
            m4a_bytes(256, 0x42),
        ),
        (
            "kind=file&name=voice.m4a",
            "audio/mp4",
            m4a_bytes(256, 0x43),
        ),
        (
            "kind=location&latitude=55.7558&longitude=37.6173",
            "",
            Vec::new(),
        ),
    ];
    for (query, mime, bytes) in cases {
        let query = if query.is_empty() {
            format!("waveform={SHAPE}")
        } else {
            format!("{query}&waveform={SHAPE}")
        };
        let response = upload_raw(&server, &owner, &query, mime, bytes).await;
        assert_error(response, 400, "validation").await;
    }
    assert_eq!(attachment_rows(&server).await, before, "nothing was stored");
}

// --- 0053's CHECK ---------------------------------------------------------------

/// The half no request can reach: a write that bypasses the upload meets
/// the constraint — a waveform only on audio, and only well-formed — rather
/// than a row the protocol forbids. And the column's default is the truth
/// about the past: nothing uploaded without one has one.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_0053_check_keeps_waveforms_on_audio_and_well_formed() {
    let server = spawn_server().await;
    let (owner, _member, _chat_id) = family_of_two(&server).await;

    let audio = upload(
        &server,
        &owner,
        "kind=audio&duration_ms=1000",
        "audio/mp4",
        m4a_bytes(256, 0x51),
    )
    .await["id"]
        .as_i64()
        .expect("id");
    let others = [
        upload(
            &server,
            &owner,
            "kind=photo",
            "image/jpeg",
            jpeg_bytes(256, 5),
        )
        .await,
        upload(
            &server,
            &owner,
            "kind=video&duration_ms=1000",
            "video/mp4",
            m4a_bytes(256, 0x52),
        )
        .await,
        upload(
            &server,
            &owner,
            "kind=file&name=a.bin",
            "application/octet-stream",
            vec![1, 2, 3],
        )
        .await,
        upload(
            &server,
            &owner,
            "kind=location&latitude=1&longitude=2",
            "",
            Vec::new(),
        )
        .await,
    ];

    let with_one: i64 =
        sqlx::query_scalar("SELECT count(*) FROM attachments WHERE waveform IS NOT NULL")
            .fetch_one(&server.state.pool)
            .await
            .expect("counting");
    assert_eq!(
        with_one, 0,
        "nullable, no default: no upload starts with one"
    );

    let constraint_of = |result: Result<sqlx::postgres::PgQueryResult, sqlx::Error>| match result {
        Err(sqlx::Error::Database(error)) => error.constraint().map(str::to_string),
        Ok(_) => None,
        Err(other) => panic!("not a constraint refusal: {other}"),
    };
    let set = |id: i64, value: String| {
        let pool = server.state.pool.clone();
        async move {
            sqlx::query("UPDATE attachments SET waveform = $2 WHERE id = $1")
                .bind(id)
                .bind(value)
                .execute(&pool)
                .await
        }
    };

    for other in &others {
        let id = other["id"].as_i64().expect("id");
        assert_eq!(
            constraint_of(set(id, SHAPE.to_string()).await).as_deref(),
            Some("attachments_waveform_is_audio"),
            "{other}"
        );
    }
    for malformed in [
        String::new(),
        "0".repeat(47),
        SHAPE.to_uppercase(),
        format!("{SHAPE}\n"),
    ] {
        assert_eq!(
            constraint_of(set(audio, malformed.clone()).await).as_deref(),
            Some("attachments_waveform_is_audio"),
            "{malformed:?}"
        );
    }
    set(audio, SHAPE.to_string())
        .await
        .expect("a well-formed waveform on audio is the allowed state");
}
