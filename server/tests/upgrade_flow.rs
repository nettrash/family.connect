//! Integration: upgrading a server that holds plaintext (#59).
//!
//! A database is built the way the last version before encryption at rest
//! left it — every migration up to 0058, and rows written in the clear
//! straight into the tables, the way that server wrote them — together with
//! attachment files in the clear on disk. Then the real start-up runs: 0059,
//! the conversion, 0060, the keyring. What the test then holds the server to
//! is the whole promise of docs/protocol.md, "Stored encrypted":
//!
//! - everything reads back through the API exactly as it was written;
//! - nothing a person wrote is left readable in any column of any table, or,
//!   once the background pass has run, in any file;
//! - a conversion stopped half-way continues on the next start.
//!
//! Every value seeded carries the word `Zebra`, so the at-rest scan has one
//! needle to look for, in every column `information_schema` names — a table
//! added next year is covered without touching this file.

mod common;

use common::{Before, TestServer, spawn_server_after};
use family_connect::config::Config;
use family_connect::keyring::Owner;
use family_connect::vault::Lookup;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::Row;

/// The one word every seeded value carries.
const NEEDLE: &str = "Zebra";

/// What the photo, its preview, the voice note, the document and the sticker
/// were, byte for byte. Each opens with its kind's magic number and carries
/// the needle, so a file left in the clear is found by the scan.
fn photo() -> Vec<u8> {
    [
        &[0xFF, 0xD8, 0xFF, 0xE0][..],
        b"Zebra photo bytes ",
        &[7u8; 70_000],
    ]
    .concat()
}
fn preview() -> Vec<u8> {
    [&[0xFF, 0xD8, 0xFF, 0xE0][..], b"Zebra preview"].concat()
}
fn voice() -> Vec<u8> {
    [&[0, 0, 0, 0x18][..], b"ftypM4A Zebra voice"].concat()
}
fn document() -> Vec<u8> {
    b"%PDF-1.4 Zebra document".to_vec()
}
fn sticker() -> Vec<u8> {
    [
        &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A][..],
        b"Zebra sticker",
    ]
    .concat()
}
fn avatar() -> Vec<u8> {
    [
        &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A][..],
        b"Zebra face",
    ]
    .concat()
}

const WAVEFORM: &str = "0123456789abcdef0123456789abcdef0123456789abcdef";

/// Everything the older server would have stored for one family: three
/// accounts and the assistant, a family with a pending join request, all
/// four kinds of chat, messages with a reply, a reaction, a mention, a poll
/// and four attachments, a transcript, a board with an event, a task list
/// and a mention, a sticker, both kinds of report, a device and a picture.
async fn before_encryption(interrupted_after: Option<u64>) -> Before {
    let hash = family_connect::auth::hash_password("password123".to_string())
        .await
        .expect("hashing a password");
    let photo_sha = hex::encode(Sha256::digest(photo()));
    let seed_sql = format!(
        "INSERT INTO users (id, username, display_name, password_hash, birthday_month, birthday_day,
                            assistant_consent_at)
         VALUES (101, 'Olive', 'Zebra Olive', '{hash}', 3, 14, now()),
                (102, 'bob', 'Zebra Bob', '{hash}', NULL, NULL, now()),
                (103, 'carol', 'Zebra Carol', '{hash}', NULL, NULL, now());
         INSERT INTO user_avatars (user_id, content_type, bytes)
         VALUES (101, 'image/png', decode('{avatar}', 'hex'));
         UPDATE users SET avatar_seq = 1, avatar_version = 1 WHERE id = 101;
         INSERT INTO families (id, name, invite_code, owner_user_id, join_policy, greeting_places)
         VALUES (201, 'Zebra Family', 'ZEBRACODE', 101, 'approval', ARRAY['Zebraville']);
         INSERT INTO family_members (family_id, user_id) VALUES (201, 101), (201, 102);
         INSERT INTO join_requests (id, family_id, user_id, invite_code_used)
         VALUES (701, 201, 103, 'ZEBRACODE');
         INSERT INTO devices (id, user_id, platform, push_token, voip_token, language)
         VALUES (1201, 101, 'ios', 'Zebra-push-token', 'Zebra-voip-token', 'en');

         INSERT INTO chats (id, family_id, kind) VALUES (301, 201, 'family');
         INSERT INTO chats (id, family_id, kind, title, admin_user_id)
         VALUES (302, 201, 'group', 'Zebra Group', 101);
         INSERT INTO chats (id, family_id, kind, user_a_id, user_b_id)
         VALUES (303, 201, 'direct', 101, 102);
         INSERT INTO chats (id, family_id, kind, user_a_id) VALUES (304, 201, 'ai', 101);
         INSERT INTO chat_members (chat_id, user_id)
         VALUES (301, 101), (301, 102), (302, 101), (302, 102), (303, 101), (303, 102),
                (304, 101);

         INSERT INTO messages (id, chat_id, sender_id, client_msg_id, body) VALUES
             (401, 301, 101, gen_random_uuid(), 'Zebra hello family'),
             (402, 301, 102, gen_random_uuid(), 'Zebra reply'),
             (403, 302, 101, gen_random_uuid(), 'Zebra in the group'),
             (404, 303, 102, gen_random_uuid(), 'Zebra direct words'),
             (405, 304, 101, gen_random_uuid(), 'Zebra question'),
             (406, 304, (SELECT id FROM users WHERE username = 'assistant'), gen_random_uuid(),
              'Zebra answer'),
             (407, 301, 101, gen_random_uuid(), 'Zebra poll question'),
             (408, 301, 101, gen_random_uuid(), ''),
             (409, 303, 101, gen_random_uuid(), ''),
             (410, 301, 101, gen_random_uuid(), ''),
             (411, 302, 102, gen_random_uuid(), '');
         UPDATE messages SET reply_to_message_id = 401, thread_root_id = 401 WHERE id = 402;
         INSERT INTO message_reactions (message_id, user_id, emoji) VALUES (401, 102, 'Zebra');
         UPDATE messages SET reaction_seq = nextval('message_reaction_seq') WHERE id = 401;
         UPDATE chats SET last_reaction_seq = (SELECT reaction_seq FROM messages WHERE id = 401)
          WHERE id = 301;
         INSERT INTO message_mentions (message_id, user_id, name, position)
         VALUES (403, 102, 'Zebra Bob', 0);
         INSERT INTO polls (message_id, chat_id, poll_seq)
         VALUES (407, 301, nextval('message_poll_seq'));
         INSERT INTO poll_options (id, poll_id, position, text)
         VALUES (801, 407, 0, 'Zebra yes'), (802, 407, 1, 'Zebra no');
         UPDATE chats SET last_poll_seq = (SELECT poll_seq FROM polls WHERE message_id = 407)
          WHERE id = 301;

         INSERT INTO attachments (id, uploader_id, message_id, kind, mime, size_bytes, storage_key,
                                  has_preview, content_hash, family_id, position)
         VALUES (501, 101, 408, 'photo', 'image/jpeg', {photo_len}, '101-zebraphoto', true,
                 '{photo_sha}', 201, 0);
         INSERT INTO attachments (id, uploader_id, message_id, kind, mime, size_bytes, storage_key,
                                  family_id, position, waveform, duration_ms)
         VALUES (502, 101, 409, 'audio', 'audio/mp4', {voice_len}, '101-zebravoice', 201, 0,
                 '{WAVEFORM}', 1500);
         INSERT INTO attachments (id, uploader_id, message_id, kind, mime, size_bytes, storage_key,
                                  family_id, position, name, latitude, longitude, accuracy_m)
         VALUES (503, 101, 410, 'location', 'application/vnd.family-connect.location', 0,
                 '101-zebraplace', 201, 0, 'Zebra Home', 55.7558, 37.6173, 12);
         INSERT INTO attachments (id, uploader_id, message_id, kind, mime, size_bytes, storage_key,
                                  family_id, position, name)
         VALUES (504, 102, 411, 'file', 'application/pdf', {document_len}, '102-zebrafile', 201, 0,
                 'Zebra notes.pdf');
         INSERT INTO transcripts (attachment_id, text, language) VALUES (502, 'Zebra transcript', 'en');

         INSERT INTO notes (id, family_id, author_id, text, color, size, x, y, board_seq,
                            content_seq)
         VALUES (601, 201, 101, 'Zebra note', 'yellow', 'medium', 0.1, 0.2,
                 nextval('family_board_seq'), currval('family_board_seq'));
         INSERT INTO notes (id, family_id, author_id, text, color, size, x, y, board_seq,
                            content_seq, kind, starts_at, place)
         VALUES (602, 201, 101, 'Zebra party', 'yellow', 'medium', 0.3, 0.4,
                 nextval('family_board_seq'), currval('family_board_seq'), 'event',
                 now() + interval '1 day', 'Zebra Hall');
         INSERT INTO notes (id, family_id, author_id, text, color, size, x, y, board_seq,
                            content_seq, kind)
         VALUES (603, 201, 101, 'Zebra chores', 'yellow', 'medium', 0.5, 0.6,
                 nextval('family_board_seq'), currval('family_board_seq'), 'tasks');
         INSERT INTO note_task_items (id, note_id, position, text)
         VALUES (6031, 603, 0, 'Zebra milk'), (6032, 603, 1, 'Zebra bread');
         INSERT INTO note_mentions (note_id, user_id, name, position) VALUES (601, 102, 'Zebra Bob', 0);
         UPDATE families SET last_board_seq = (SELECT max(board_seq) FROM notes) WHERE id = 201;

         INSERT INTO pack_items (id, family_id, added_by, label, pack_seq)
         VALUES (901, 201, 101, 'Zebra wave', nextval('family_pack_seq'));
         INSERT INTO attachments (id, uploader_id, pack_item_id, kind, mime, size_bytes,
                                  storage_key, family_id, position)
         VALUES (505, 101, 901, 'photo', 'image/png', {sticker_len}, '101-zebrasticker', 201, 0);
         UPDATE families SET last_pack_seq = (SELECT pack_seq FROM pack_items) WHERE id = 201;

         INSERT INTO member_reports (id, family_id, reporter_user_id, reported_user_id, message_id,
                                     message_excerpt, reason)
         VALUES (1001, 201, 101, 102, 402, 'Zebra reply', 'spam');
         INSERT INTO assistant_reports (id, reporter_user_id, message_id, message_excerpt,
                                        chat_kind, reason, note)
         VALUES (1101, 101, 406, 'Zebra answer', 'ai', 'other', 'Zebra says it is wrong');

         SELECT setval('users_id_seq', 5000), setval('families_id_seq', 5000),
                setval('chats_id_seq', 5000), setval('messages_id_seq', 5000),
                setval('attachments_id_seq', 5000), setval('notes_id_seq', 5000),
                setval('note_task_items_id_seq', 5000), setval('pack_items_id_seq', 5000),
                setval('member_reports_id_seq', 5000), setval('assistant_reports_id_seq', 5000),
                setval('devices_id_seq', 5000), setval('join_requests_id_seq', 5000),
                setval('poll_options_id_seq', 5000);",
        avatar = hex::encode(avatar()),
        photo_len = photo().len(),
        voice_len = voice().len(),
        document_len = document().len(),
        sticker_len = sticker().len(),
    );
    Before {
        version: 58,
        seed_sql,
        files: vec![
            ("101-zebraphoto".to_string(), photo()),
            ("101-zebraphoto.preview".to_string(), preview()),
            ("101-zebravoice".to_string(), voice()),
            ("102-zebrafile".to_string(), document()),
            ("101-zebrasticker".to_string(), sticker()),
        ],
        interrupted_after,
    }
}

/// Room for the seeded photo to be uploaded again: it is a little over one
/// sealed chunk (64 KiB), so that a range can cross the boundary, and the
/// harness's ceiling is exactly 64 KiB.
fn room_for_the_photo(cfg: &mut Config) {
    cfg.limits.max_attachment_bytes = 1024 * 1024;
}

async fn json(response: reqwest::Response) -> Value {
    let status = response.status();
    let body = response.text().await.expect("a body");
    assert!(status.is_success(), "{status}: {body}");
    serde_json::from_str(&body).expect("JSON")
}

async fn messages(ts: &TestServer, token: &str, chat_id: i64) -> Vec<Value> {
    json(ts.get(token, &format!("/chats/{chat_id}/messages")).await).await["messages"]
        .as_array()
        .expect("messages")
        .clone()
}

fn by_id(messages: &[Value], id: i64) -> Value {
    messages
        .iter()
        .find(|message| message["id"] == id)
        .unwrap_or_else(|| panic!("message {id} is on the page"))
        .clone()
}

async fn bytes_of(ts: &TestServer, token: &str, path: &str) -> Vec<u8> {
    let response = ts.get(token, path).await;
    assert_eq!(response.status(), 200, "{path}");
    response.bytes().await.expect("the bytes").to_vec()
}

/// Everything the scenario seeded, read back through the API.
async fn reads_back(ts: &TestServer) {
    // Signing in finds the account by its lookup, case-insensitively as
    // before: it was stored as "Olive".
    let olive = ts.login("olive", "password123").await;
    let bob = ts.login("BOB", "password123").await;

    let me = json(ts.get(&olive, "/me").await).await;
    assert_eq!(me["user"]["display_name"], "Zebra Olive");
    assert_eq!(me["user"]["username"], "Olive");
    assert_eq!(
        me["user"]["birthday"],
        serde_json::json!({"month": 3, "day": 14})
    );
    assert_eq!(me["family"]["name"], "Zebra Family");
    assert_eq!(me["family"]["invite_code"], "ZEBRACODE");
    assert_eq!(
        me["family"]["greeting_places"],
        serde_json::json!(["Zebraville"])
    );

    let mine = json(ts.get(&olive, "/families/mine").await).await;
    let names: Vec<&str> = mine["members"]
        .as_array()
        .expect("members")
        .iter()
        .map(|member| member["display_name"].as_str().expect("a name"))
        .collect();
    assert_eq!(names, ["Zebra Olive", "Zebra Bob"]);

    let requests = json(ts.get(&olive, "/families/join-requests").await).await;
    assert_eq!(
        requests["requests"][0]["user"]["display_name"],
        "Zebra Carol"
    );

    let chats = json(ts.get(&olive, "/chats").await).await;
    let title_of = |id: i64| {
        chats["chats"]
            .as_array()
            .expect("chats")
            .iter()
            .find(|entry| entry["chat"]["id"] == id)
            .unwrap_or_else(|| panic!("chat {id} is listed"))["chat"]["title"]
            .clone()
    };
    assert_eq!(title_of(301), "Zebra Family");
    assert_eq!(title_of(302), "Zebra Group");
    assert_eq!(title_of(303), "Zebra Bob");

    let family = messages(ts, &olive, 301).await;
    assert_eq!(by_id(&family, 401)["body"], "Zebra hello family");
    assert_eq!(by_id(&family, 401)["reactions"][0]["emoji"], "Zebra");
    assert_eq!(
        by_id(&family, 402)["reply_to"]["excerpt"],
        "Zebra hello family"
    );
    let poll = by_id(&family, 407);
    assert_eq!(poll["body"], "Zebra poll question");
    assert_eq!(poll["poll"]["options"][0]["text"], "Zebra yes");
    assert_eq!(poll["poll"]["options"][1]["text"], "Zebra no");
    let place = &by_id(&family, 410)["attachments"][0];
    assert_eq!(place["name"], "Zebra Home");
    assert_eq!(place["latitude"], 55.7558);
    assert_eq!(place["longitude"], 37.6173);

    let group = messages(ts, &olive, 302).await;
    assert_eq!(by_id(&group, 403)["mentions"][0]["name"], "Zebra Bob");
    assert_eq!(
        by_id(&group, 411)["attachments"][0]["name"],
        "Zebra notes.pdf"
    );

    let direct = messages(ts, &bob, 303).await;
    assert_eq!(by_id(&direct, 404)["body"], "Zebra direct words");
    assert_eq!(by_id(&direct, 409)["attachments"][0]["waveform"], WAVEFORM);

    let ai = messages(ts, &olive, 304).await;
    assert_eq!(by_id(&ai, 406)["body"], "Zebra answer");

    let board = json(ts.get(&olive, "/families/mine/board").await).await;
    let note = |id: i64| {
        board["notes"]
            .as_array()
            .expect("notes")
            .iter()
            .find(|note| note["id"] == id)
            .unwrap_or_else(|| panic!("note {id} is on the board"))
            .clone()
    };
    assert_eq!(note(601)["text"], "Zebra note");
    assert_eq!(note(601)["mentions"][0]["name"], "Zebra Bob");
    assert_eq!(note(602)["place"], "Zebra Hall");
    assert_eq!(note(603)["items"][0]["text"], "Zebra milk");
    assert_eq!(note(603)["items"][1]["text"], "Zebra bread");

    let pack = json(ts.get(&olive, "/families/mine/pack").await).await;
    assert_eq!(pack["items"][0]["label"], "Zebra wave");

    let reports = json(ts.get(&olive, "/families/reports").await).await;
    assert_eq!(reports["reports"][0]["message_excerpt"], "Zebra reply");

    assert_eq!(bytes_of(ts, &olive, "/users/101/avatar").await, avatar());

    // The rows the API does not show, opened directly.
    let transcript: Vec<u8> =
        sqlx::query_scalar("SELECT text FROM transcripts WHERE attachment_id = 502")
            .fetch_one(&ts.state.pool)
            .await
            .expect("the transcript");
    assert_eq!(ts.open_text(&transcript), "Zebra transcript");
    let (excerpt, note): (Vec<u8>, Vec<u8>) =
        sqlx::query_as("SELECT message_excerpt, note FROM assistant_reports WHERE id = 1101")
            .fetch_one(&ts.state.pool)
            .await
            .expect("the assistant report");
    assert_eq!(ts.open_text(&excerpt), "Zebra answer");
    assert_eq!(ts.open_text(&note), "Zebra says it is wrong");
    let device = sqlx::query(
        "SELECT push_token, push_token_key, voip_token, voip_token_key FROM devices WHERE id = 1201",
    )
    .fetch_one(&ts.state.pool)
    .await
    .expect("the device");
    assert_eq!(
        ts.open_text(&device.get::<Vec<u8>, _>("push_token")),
        "Zebra-push-token"
    );
    assert_eq!(
        device.get::<Vec<u8>, _>("push_token_key"),
        ts.lookup(Lookup::PushToken, "Zebra-push-token")
    );
    assert_eq!(
        ts.open_text(&device.get::<Vec<u8>, _>("voip_token")),
        "Zebra-voip-token"
    );
    assert_eq!(
        device.get::<Vec<u8>, _>("voip_token_key"),
        ts.lookup(Lookup::VoipToken, "Zebra-voip-token")
    );
}

/// Every attachment file, read back whole and by range.
async fn files_read_back(ts: &TestServer) {
    let olive = ts.login("olive", "password123").await;
    let bob = ts.login("bob", "password123").await;
    assert_eq!(bytes_of(ts, &olive, "/attachments/501").await, photo());
    assert_eq!(
        bytes_of(ts, &olive, "/attachments/501/preview").await,
        preview()
    );
    assert_eq!(bytes_of(ts, &bob, "/attachments/502").await, voice());
    assert_eq!(bytes_of(ts, &olive, "/attachments/504").await, document());
    assert_eq!(bytes_of(ts, &olive, "/attachments/505").await, sticker());

    // A range across the first chunk boundary of a sealed file (64 KiB),
    // answered with the PLAINTEXT's offsets and size.
    let response = ts
        .client
        .get(ts.url("/attachments/501"))
        .bearer_auth(&olive)
        .header(reqwest::header::RANGE, "bytes=65530-65545")
        .send()
        .await
        .expect("request sends");
    assert_eq!(response.status(), 206);
    assert_eq!(
        response.headers()[reqwest::header::CONTENT_RANGE],
        format!("bytes 65530-65545/{}", photo().len()).as_str()
    );
    assert_eq!(
        response.bytes().await.expect("the range").to_vec(),
        photo()[65530..=65545]
    );
}

/// Every column of every table that holds the needle, as `table.column`.
async fn columns_holding(ts: &TestServer, needle: &str) -> Vec<String> {
    let columns: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT c.table_name, c.column_name, c.data_type
           FROM information_schema.columns c
           JOIN information_schema.tables t
             ON t.table_schema = c.table_schema AND t.table_name = c.table_name
          WHERE c.table_schema = 'public' AND t.table_type = 'BASE TABLE'",
    )
    .fetch_all(&ts.state.pool)
    .await
    .expect("reading the catalogue");
    let mut holding = Vec::new();
    for (table, column, data_type) in columns {
        let condition = if data_type == "bytea" {
            format!("position(convert_to($1, 'UTF8') in \"{column}\") > 0")
        } else {
            format!("CAST(\"{column}\" AS text) LIKE '%' || $1 || '%'")
        };
        let found: bool = sqlx::query_scalar(&format!(
            "SELECT EXISTS (SELECT 1 FROM \"{table}\" WHERE {condition})"
        ))
        .bind(needle)
        .fetch_one(&ts.state.pool)
        .await
        .unwrap_or_else(|err| panic!("scanning {table}.{column}: {err}"));
        if found {
            holding.push(format!("{table}.{column}"));
        }
    }
    holding
}

/// Every file under the attachments directory that holds the needle.
fn files_holding(ts: &TestServer, needle: &[u8]) -> Vec<String> {
    fn walk(dir: &std::path::Path, needle: &[u8], out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, needle, out);
            } else if std::fs::read(&path)
                .map(|bytes| bytes.windows(needle.len()).any(|window| window == needle))
                .unwrap_or(false)
            {
                out.push(path.display().to_string());
            }
        }
    }
    let mut out = Vec::new();
    walk(&ts.attachments_dir, needle, &mut out);
    out
}

/// The whole upgrade: every value reads back, nothing is left readable in
/// the database, and once the background pass has run, nothing in the files
/// either — with the files served correctly before it, from the plaintext
/// they still were, and after it, from their seals.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_server_with_plaintext_upgrades_to_sealed_and_reads_back_the_same() {
    let ts = spawn_server_after(before_encryption(None).await, room_for_the_photo).await;

    assert_eq!(
        columns_holding(&ts, NEEDLE).await,
        Vec::<String>::new(),
        "content left readable in the database"
    );

    // Every owner has its key, and every key has its owner.
    let (owners, keys): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM families) + (SELECT count(*) FROM users)
                + (SELECT count(*) FROM chats WHERE kind <> 'family'),
                (SELECT count(*) FROM storage_keys)",
    )
    .fetch_one(&ts.state.pool)
    .await
    .expect("counting keys");
    assert_eq!(owners, keys);

    reads_back(&ts).await;

    // The files are still plaintext until the background pass: served as
    // they are.
    assert!(!files_holding(&ts, NEEDLE.as_bytes()).is_empty());
    files_read_back(&ts).await;

    let sealed = family_connect::seal_files::run(&ts.state)
        .await
        .expect("sealing the stored files");
    assert_eq!(
        sealed, 4,
        "the photo, the voice note, the document, the sticker"
    );
    assert_eq!(
        files_holding(&ts, NEEDLE.as_bytes()),
        Vec::<String>::new(),
        "content left readable on disk"
    );
    let unsealed: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM attachments WHERE file_key IS NULL AND kind <> 'location'",
    )
    .fetch_one(&ts.state.pool)
    .await
    .expect("counting unsealed files");
    assert_eq!(unsealed, 0);
    let noted: i64 = sqlx::query_scalar("SELECT count(*) FROM storage_cleanup")
        .fetch_one(&ts.state.pool)
        .await
        .expect("counting noted files");
    assert_eq!(noted, 0, "every plaintext file was removed");
    files_read_back(&ts).await;
    // A second pass finds nothing to do.
    assert_eq!(
        family_connect::seal_files::run(&ts.state)
            .await
            .expect("again"),
        0
    );

    // Dedup still finds the converted photo: the same bytes uploaded again
    // share its (sealed) file.
    let olive = ts.login("olive", "password123").await;
    let uploaded = json(
        ts.put_bytes_method("POST", &olive, "/attachments", "image/jpeg", photo())
            .await,
    )
    .await;
    let (new_key, old_key): (String, String) = sqlx::query_as(
        "SELECT (SELECT storage_key FROM attachments WHERE id = $1),
                (SELECT storage_key FROM attachments WHERE id = 501)",
    )
    .bind(uploaded["attachment"]["id"].as_i64().expect("an id"))
    .fetch_one(&ts.state.pool)
    .await
    .expect("reading both keys");
    assert_eq!(
        new_key, old_key,
        "the identical upload shares the converted file"
    );
}

/// A conversion stopped half-way — some owners keyed, some tables sealed —
/// continues on the next start and ends exactly where an uninterrupted one
/// does.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_interrupted_conversion_continues_on_the_next_start() {
    for stopped_after in [1, 4, 9] {
        let ts = spawn_server_after(
            before_encryption(Some(stopped_after)).await,
            room_for_the_photo,
        )
        .await;
        assert_eq!(
            columns_holding(&ts, NEEDLE).await,
            Vec::<String>::new(),
            "stopped after {stopped_after} batches"
        );
        reads_back(&ts).await;
    }
}

async fn owner_key(ts: &TestServer, owner: Owner) -> i64 {
    ts.state
        .keys
        .key(&ts.state.pool, owner)
        .await
        .expect("the owner's key")
        .id()
}

async fn body_of(ts: &TestServer, id: i64) -> Vec<u8> {
    sqlx::query_scalar("SELECT body FROM messages WHERE id = $1")
        .bind(id)
        .fetch_one(&ts.state.pool)
        .await
        .expect("a body")
}

/// The values the conversion sealed are under the keys of what they belong
/// to: the group's words under the group's key, the direct chat's under its
/// own, an account's under the account's — so deleting any of them can take
/// its words from every backup.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_conversion_seals_each_value_under_its_owners_key() {
    let ts = spawn_server_after(before_encryption(None).await, room_for_the_photo).await;
    let key_of = |sealed: Vec<u8>| family_connect::vault::key_id_of(&sealed).expect("sealed");
    assert_eq!(
        key_of(body_of(&ts, 401).await),
        owner_key(&ts, Owner::Family(201)).await
    );
    assert_eq!(
        key_of(body_of(&ts, 403).await),
        owner_key(&ts, Owner::Chat(302)).await
    );
    assert_eq!(
        key_of(body_of(&ts, 404).await),
        owner_key(&ts, Owner::Chat(303)).await
    );
    assert_eq!(
        key_of(body_of(&ts, 405).await),
        owner_key(&ts, Owner::Chat(304)).await
    );
    assert_ne!(
        owner_key(&ts, Owner::Chat(302)).await,
        owner_key(&ts, Owner::Family(201)).await
    );
    // The family chat shares its family's key.
    assert_eq!(
        owner_key(&ts, Owner::Chat(301)).await,
        owner_key(&ts, Owner::Family(201)).await
    );
    let name: Vec<u8> = sqlx::query_scalar("SELECT display_name FROM users WHERE id = 102")
        .fetch_one(&ts.state.pool)
        .await
        .expect("a name");
    assert_eq!(key_of(name), owner_key(&ts, Owner::Account(102)).await);
    // A file in a group is held by the group.
    let file_name: Vec<u8> = sqlx::query_scalar("SELECT name FROM attachments WHERE id = 504")
        .fetch_one(&ts.state.pool)
        .await
        .expect("a file name");
    assert_eq!(key_of(file_name), owner_key(&ts, Owner::Chat(302)).await);
}

/// A preview uploaded for a file the background pass has not reached yet —
/// an upload begun before the upgrade, finished after it — seals that file
/// on the spot, and the preview with it, rather than writing a plaintext
/// preview beside it or refusing.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_preview_for_a_file_from_before_seals_that_file_first() {
    let ts = spawn_server_after(before_encryption(None).await, room_for_the_photo).await;
    let olive = ts.login("olive", "password123").await;
    let mut new_preview = vec![0xFF, 0xD8, 0xFF, 0xE0];
    new_preview.extend(b"a newer Zebra preview");
    let response = ts
        .put_bytes_method(
            "PUT",
            &olive,
            "/attachments/501/preview",
            "image/jpeg",
            new_preview.clone(),
        )
        .await;
    assert_eq!(response.status(), 204);

    let sealed: bool =
        sqlx::query_scalar("SELECT file_key IS NOT NULL FROM attachments WHERE id = 501")
            .fetch_one(&ts.state.pool)
            .await
            .expect("reading the row");
    assert!(sealed, "the file was sealed before its preview was written");
    assert_eq!(bytes_of(&ts, &olive, "/attachments/501").await, photo());
    assert_eq!(
        bytes_of(&ts, &olive, "/attachments/501/preview").await,
        new_preview
    );
    let unsealed_photo = files_holding(&ts, b"Zebra photo");
    assert_eq!(unsealed_photo, Vec::<String>::new());
    let unsealed_preview = files_holding(&ts, b"Zebra preview");
    assert_eq!(unsealed_preview, Vec::<String>::new());
}
