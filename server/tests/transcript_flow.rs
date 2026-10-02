//! Transcripts on request (docs/protocol.md, "Transcripts on request").
//!
//! Every test here talks to a STUB transcription deployment on a local
//! listener — the pattern `assistant_flow.rs` uses for chat and pictures —
//! so what is checked is what actually LEFT this server: which bytes, under
//! which file name, with which language hint, how many times.

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::{Multipart, Path, RawQuery, State};
use axum::response::IntoResponse;
use axum::routing::post;
use common::{TestServer, assert_error, spawn_server_with_config};
use family_connect::config::Config;
use serde_json::{Value, json};

/// What the stub answers with unless a test says otherwise. Words no other
/// test in the tree says, so "not in the log" means these words.
const SPOKEN: &str = "Bring the purple umbrella from Aunt Zinnia's attic";
/// A language the stub reports — shaped like one, and nothing the server
/// would ever log on its own.
const HEARD: &str = "klingon";

const TEXT_DEPLOYMENT: &str = "test-gpt-oss";
const TRANSCRIBE_DEPLOYMENT: &str = "test-whisper";

// --- the stub provider ------------------------------------------------------

/// One multipart field as the stub received it.
#[derive(Debug, Clone)]
struct Field {
    filename: Option<String>,
    content_type: Option<String>,
    bytes: Vec<u8>,
}

impl Field {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }
}

/// One request the stub captured.
#[derive(Debug, Clone)]
struct Call {
    path: String,
    query: Option<String>,
    fields: HashMap<String, Field>,
}

#[derive(Default)]
struct Stub {
    calls: Mutex<Vec<Call>>,
    /// An HTTP failure to answer with instead, while a test says so.
    failure: Mutex<Option<(u16, Value)>>,
    /// How long each answer takes, while a test says so — recorded BEFORE
    /// the wait, so a test can see a call started.
    delay: Mutex<Option<Duration>>,
}

impl Stub {
    fn calls(&self) -> Vec<Call> {
        self.calls.lock().expect("stub lock").clone()
    }

    fn fail_with(&self, status: u16, body: Value) {
        *self.failure.lock().expect("stub lock") = Some((status, body));
    }

    fn slow(&self, by: Duration) {
        *self.delay.lock().expect("stub lock") = Some(by);
    }
}

async fn stub_transcribe(
    Path(deployment): Path<String>,
    RawQuery(query): RawQuery,
    State(stub): State<Arc<Stub>>,
    mut multipart: Multipart,
) -> axum::response::Response {
    let mut fields = HashMap::new();
    while let Some(field) = multipart.next_field().await.expect("a multipart body") {
        let name = field.name().unwrap_or_default().to_string();
        let filename = field.file_name().map(str::to_string);
        let content_type = field.content_type().map(str::to_string);
        let bytes = field.bytes().await.expect("field bytes").to_vec();
        fields.insert(
            name,
            Field {
                filename,
                content_type,
                bytes,
            },
        );
    }
    stub.calls.lock().expect("stub lock").push(Call {
        path: format!("/openai/deployments/{deployment}/audio/transcriptions"),
        query,
        fields,
    });
    let delay = *stub.delay.lock().expect("stub lock");
    if let Some(delay) = delay {
        tokio::time::sleep(delay).await;
    }
    let failure = stub.failure.lock().expect("stub lock").clone();
    if let Some((status, body)) = failure {
        return (
            axum::http::StatusCode::from_u16(status).expect("status"),
            axum::Json(body),
        )
            .into_response();
    }
    axum::Json(json!({"text": SPOKEN, "language": HEARD})).into_response()
}

async fn spawn_stub() -> (Arc<Stub>, SocketAddr) {
    let stub = Arc::new(Stub::default());
    let router = Router::new()
        .route(
            "/openai/deployments/{deployment}/audio/transcriptions",
            post(stub_transcribe),
        )
        .with_state(stub.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("binding the stub port");
    let addr = listener.local_addr().expect("stub addr");
    tokio::spawn(async move {
        axum::serve(listener, router).await.expect("stub crashed");
    });
    (stub, addr)
}

// --- the server and the family ----------------------------------------------

/// A server whose assistant can transcribe, pointed at the stub.
async fn server(addr: SocketAddr) -> TestServer {
    server_tweaked(addr, |_| {}).await
}

async fn server_tweaked(addr: SocketAddr, extra: impl FnOnce(&mut Config)) -> TestServer {
    spawn_server_with_config(move |cfg| {
        cfg.ai.enabled = true;
        cfg.ai.endpoint = format!("http://{addr}");
        cfg.ai.deployment = TEXT_DEPLOYMENT.to_string();
        cfg.ai.api_key = "test-key".to_string();
        cfg.ai.processor = "Microsoft — Azure OpenAI".to_string();
        cfg.ai.title = "Assistant".to_string();
        cfg.ai.transcribe.deployment.deployment = TRANSCRIBE_DEPLOYMENT.to_string();
        extra(cfg);
    })
    .await
}

struct Family {
    owner: String,
    owner_id: i64,
    member: String,
    member_id: i64,
    chat: i64,
}

/// An owner and a member, both of whom have agreed to the assistant.
async fn family(ts: &TestServer) -> Family {
    let (owner, owner_id) = ts.register("olive", "Olive").await;
    let (_, code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    let (member, member_id) = ts.register("mark", "Mark").await;
    ts.join(&member, &code, "joined").await;
    let chat = ts.family_chat_id(&owner).await;
    Family {
        owner,
        owner_id,
        member,
        member_id,
        chat,
    }
}

/// ISO base media with a marker, so two uploads are never the same bytes
/// (identical bytes are stored once per family) and a test can recognise
/// exactly these bytes on the stub's side.
fn mp4_bytes(marker: u8, len: usize) -> Vec<u8> {
    let mut bytes = vec![0x00, 0x00, 0x00, 0x18];
    bytes.extend_from_slice(b"ftypM4A ");
    bytes.push(marker);
    bytes.resize(len, marker);
    bytes
}

async fn upload(ts: &TestServer, token: &str, query: &str, mime: &str, bytes: Vec<u8>) -> i64 {
    let response = ts
        .put_bytes_method("POST", token, &format!("/attachments{query}"), mime, bytes)
        .await;
    assert_eq!(response.status(), 201, "uploading {mime}");
    let body: Value = response.json().await.expect("JSON");
    body["attachment"]["id"].as_i64().expect("id")
}

/// A voice note, as every recording client sends one.
async fn voice_note(ts: &TestServer, token: &str, marker: u8) -> i64 {
    upload(
        ts,
        token,
        "?kind=audio&duration_ms=4200",
        "audio/mp4",
        mp4_bytes(marker, 2048),
    )
    .await
}

/// Send one attachment into a chat; returns the message id.
async fn send(ts: &TestServer, token: &str, chat: i64, attachment_id: i64) -> i64 {
    let response = ts
        .post(
            token,
            &format!("/chats/{chat}/messages"),
            json!({"client_msg_id": uuid::Uuid::new_v4().to_string(), "body": "",
                   "attachment_id": attachment_id}),
        )
        .await;
    assert_eq!(response.status(), 201, "sending attachment {attachment_id}");
    let body: Value = response.json().await.expect("JSON");
    body["message"]["id"].as_i64().expect("message id")
}

async fn invite_code(ts: &TestServer, owner: &str) -> String {
    let mine: Value = ts
        .get(owner, "/families/mine")
        .await
        .json()
        .await
        .expect("JSON");
    mine["family"]["invite_code"]
        .as_str()
        .expect("code")
        .to_string()
}

fn path(chat: i64, message: i64, attachment: i64) -> String {
    format!("/chats/{chat}/messages/{message}/attachments/{attachment}/transcript")
}

/// The no-body form: the server's own stored bytes.
async fn ask(
    ts: &TestServer,
    token: &str,
    chat: i64,
    message: i64,
    attachment: i64,
) -> reqwest::Response {
    ts.client
        .post(ts.url(&path(chat, message, attachment)))
        .bearer_auth(token)
        .send()
        .await
        .expect("request sends")
}

/// The multipart form: sound the device supplies.
async fn ask_with_sound(
    ts: &TestServer,
    token: &str,
    (chat, message, attachment): (i64, i64, i64),
    part_name: &str,
    sound: Vec<u8>,
) -> reqwest::Response {
    let part = reqwest::multipart::Part::bytes(sound)
        .file_name("sound.m4a")
        .mime_str("audio/mp4")
        .expect("mime");
    let form = reqwest::multipart::Form::new().part(part_name.to_string(), part);
    ts.client
        .post(ts.url(&path(chat, message, attachment)))
        .bearer_auth(token)
        .multipart(form)
        .send()
        .await
        .expect("request sends")
}

async fn transcript_of(response: reqwest::Response) -> Value {
    assert_eq!(response.status(), 200, "a transcript");
    let body: Value = response.json().await.expect("JSON");
    body["transcript"].clone()
}

async fn kept(ts: &TestServer, attachment: i64) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM transcripts WHERE attachment_id = $1")
        .bind(attachment)
        .fetch_one(&ts.state.pool)
        .await
        .expect("counting transcripts")
}

async fn usage_rows(ts: &TestServer) -> Vec<(i64, i64, i64)> {
    sqlx::query_as("SELECT user_id, transcripts::BIGINT, audio_ms FROM ai_usage ORDER BY id")
        .fetch_all(&ts.state.pool)
        .await
        .expect("reading usage")
}

async fn eventually<T>(what: &str, mut probe: impl AsyncFnMut() -> Option<T>) -> T {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(found) = probe().await {
            return found;
        }
        assert!(tokio::time::Instant::now() < deadline, "{what}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

// --- log capture ------------------------------------------------------------

/// Everything the server logs on THIS test's thread — the request and the
/// spawned call alike, because a `#[tokio::test]` polls both on the one
/// thread this subscriber is the default of.
#[derive(Clone, Default)]
struct CapturedLog(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for CapturedLog {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("log lock").extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl CapturedLog {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().expect("log lock")).into_owned()
    }
}

fn capture_log() -> (CapturedLog, tracing::subscriber::DefaultGuard) {
    let log = CapturedLog::default();
    let writer = log.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .finish();
    (log, tracing::subscriber::set_default(subscriber))
}

// --- the tests --------------------------------------------------------------

/// The simplest case, end to end: your own voice note in the family chat,
/// sent as stored, answered, KEPT, counted — and nothing of it in the log
/// but the outcome.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn your_own_voice_note_is_transcribed_from_the_stored_bytes_and_kept() {
    let (log, _guard) = capture_log();
    let (stub, addr) = spawn_stub().await;
    let ts = server(addr).await;
    let f = family(&ts).await;
    let bytes = mp4_bytes(7, 2048);
    let note = upload(
        &ts,
        &f.member,
        "?kind=audio&duration_ms=4200",
        "audio/mp4",
        bytes.clone(),
    )
    .await;
    let message = send(&ts, &f.member, f.chat, note).await;

    let transcript = transcript_of(ask(&ts, &f.member, f.chat, message, note).await).await;
    assert_eq!(transcript, json!({"text": SPOKEN, "language": HEARD}));

    // What left: the stored bytes, under a name whose EXTENSION says what
    // they are, the json format, the deployment as the model — and no
    // language, because this family has not chosen one.
    let calls = stub.calls();
    assert_eq!(calls.len(), 1);
    let call = &calls[0];
    assert!(call.path.contains(TRANSCRIBE_DEPLOYMENT), "{}", call.path);
    assert_eq!(call.query.as_deref(), Some("api-version=2024-10-21"));
    let file = &call.fields["file"];
    assert_eq!(file.bytes, bytes, "the stored bytes, exactly");
    assert_eq!(file.filename.as_deref(), Some("audio.m4a"));
    assert_eq!(file.content_type.as_deref(), Some("audio/mp4"));
    assert_eq!(call.fields["response_format"].text(), "json");
    assert_eq!(call.fields["model"].text(), TRANSCRIBE_DEPLOYMENT);
    assert!(
        !call.fields.contains_key("language"),
        "no hint without a family language"
    );
    let mut names: Vec<&String> = call.fields.keys().collect();
    names.sort();
    assert_eq!(
        names,
        ["file", "model", "response_format"],
        "and nothing else"
    );

    // Kept, and counted once against the asker, with the recording's length.
    assert_eq!(kept(&ts, note).await, 1);
    assert_eq!(usage_rows(&ts).await, vec![(f.member_id, 1, 4200)]);
    let stats: Value = ts
        .get(&f.owner, "/families/mine/stats")
        .await
        .json()
        .await
        .expect("JSON");
    let ai = &stats["totals"]["ai"];
    assert_eq!(ai["transcripts"], 1, "{stats}");
    assert_eq!(ai["transcript_duration_ms"], 4200, "{stats}");
    assert_eq!(
        ai["questions"], 0,
        "a transcript is not a question: {stats}"
    );
    let row = stats["members"]
        .as_array()
        .expect("members")
        .iter()
        .find(|row| row["user_id"] == f.member_id)
        .expect("the asker's row")
        .clone();
    assert_eq!(row["ai"]["transcripts"], 1, "{row}");
    assert_eq!(row["ai"]["transcript_duration_ms"], 4200, "{row}");

    // The log says what happened, and nothing that was said.
    let text = log.text();
    assert!(
        text.contains("outcome=\"stored\""),
        "the outcome is logged:\n{text}"
    );
    for words in [SPOKEN, "umbrella", "Zinnia", HEARD] {
        assert!(!text.contains(words), "{words:?} reached the log:\n{text}");
    }
}

/// Somebody else's voice needs the owner's switch — refused without it, with
/// nothing sent; answered with it.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn another_members_voice_note_needs_the_owners_switch() {
    let (stub, addr) = spawn_stub().await;
    let ts = server(addr).await;
    let f = family(&ts).await;
    let note = voice_note(&ts, &f.member, 1).await;
    let message = send(&ts, &f.member, f.chat, note).await;

    assert_error(
        ask(&ts, &f.owner, f.chat, message, note).await,
        403,
        "transcript_not_allowed",
    )
    .await;
    assert!(stub.calls().is_empty(), "nothing left the server");

    let on = ts
        .patch(&f.owner, "/families/mine", json!({"ai_transcripts": true}))
        .await;
    assert_eq!(on.status(), 200);
    let family: Value = on.json().await.expect("JSON");
    assert_eq!(family["family"]["ai_transcripts"], true);

    let transcript = transcript_of(ask(&ts, &f.owner, f.chat, message, note).await).await;
    assert_eq!(transcript["text"], SPOKEN);
    assert_eq!(stub.calls().len(), 1);
}

/// The switch is not the only key: a sender who has not agreed to the
/// assistant keeps their voice to themselves whatever the owner chose.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_sender_who_has_not_agreed_is_not_transcribed_for_anybody_else() {
    let (stub, addr) = spawn_stub().await;
    let ts = server(addr).await;
    let f = family(&ts).await;
    let (quiet, _) = ts
        .register_without_assistant_consent("quinn", "Quinn")
        .await;
    let code = invite_code(&ts, &f.owner).await;
    ts.join(&quiet, &code, "joined").await;
    let on = ts
        .patch(&f.owner, "/families/mine", json!({"ai_transcripts": true}))
        .await;
    assert_eq!(on.status(), 200);

    let note = voice_note(&ts, &quiet, 2).await;
    let message = send(&ts, &quiet, f.chat, note).await;
    assert_error(
        ask(&ts, &f.owner, f.chat, message, note).await,
        403,
        "transcript_not_allowed",
    )
    .await;
    assert!(stub.calls().is_empty());
}

/// A direct chat: your own messages only, and the switch does not reach it.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn in_a_direct_chat_only_your_own_voice_notes() {
    let (stub, addr) = spawn_stub().await;
    let ts = server(addr).await;
    let f = family(&ts).await;
    let on = ts
        .patch(&f.owner, "/families/mine", json!({"ai_transcripts": true}))
        .await;
    assert_eq!(on.status(), 200);
    let direct: Value = ts
        .post(&f.member, "/chats/direct", json!({"user_id": f.owner_id}))
        .await
        .json()
        .await
        .expect("JSON");
    let dm = direct["chat"]["id"].as_i64().expect("chat id");

    let note = voice_note(&ts, &f.member, 3).await;
    let message = send(&ts, &f.member, dm, note).await;

    let own = transcript_of(ask(&ts, &f.member, dm, message, note).await).await;
    assert_eq!(own["text"], SPOKEN);
    assert_error(
        ask(&ts, &f.owner, dm, message, note).await,
        403,
        "transcript_not_allowed",
    )
    .await;
    assert_eq!(
        stub.calls().len(),
        1,
        "only the sender's own ask reached the provider"
    );
}

/// The ASKER's consent, asked first and on its own code — even for their
/// own recording, and before anything is sent.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_asker_who_has_not_agreed_is_asked_to_first() {
    let (stub, addr) = spawn_stub().await;
    let ts = server(addr).await;
    let f = family(&ts).await;
    let (shy, shy_id) = ts.register_without_assistant_consent("shy", "Shy").await;
    let code = invite_code(&ts, &f.owner).await;
    ts.join(&shy, &code, "joined").await;
    let note = voice_note(&ts, &shy, 4).await;
    let message = send(&ts, &shy, f.chat, note).await;

    assert_error(
        ask(&ts, &shy, f.chat, message, note).await,
        403,
        "assistant_consent_required",
    )
    .await;
    assert!(stub.calls().is_empty());

    ts.agree_to_the_assistant(shy_id).await;
    let transcript = transcript_of(ask(&ts, &shy, f.chat, message, note).await).await;
    assert_eq!(transcript["text"], SPOKEN);
}

/// What the stored-bytes form will not send: a video, an Ogg file, a
/// recording over the ceiling, a photo — each `not_transcribable`, each with
/// nothing sent.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_stored_form_refuses_what_the_provider_cannot_take() {
    let (stub, addr) = spawn_stub().await;
    // A ceiling below the harness's 64 KiB upload ceiling, so an upload can
    // be over one and under the other.
    let ts = server_tweaked(addr, |cfg| cfg.ai.transcribe.max_bytes = 1024).await;
    let f = family(&ts).await;

    let video = upload(
        &ts,
        &f.member,
        "?kind=video&duration_ms=8400",
        "video/mp4",
        mp4_bytes(5, 512),
    )
    .await;
    let mut ogg = b"OggS".to_vec();
    ogg.resize(512, 9);
    let ogg = upload(
        &ts,
        &f.member,
        "?kind=audio&duration_ms=1000",
        "audio/ogg",
        ogg,
    )
    .await;
    let big = upload(
        &ts,
        &f.member,
        "?kind=audio&duration_ms=1000",
        "audio/mp4",
        mp4_bytes(6, 2048),
    )
    .await;
    let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0];
    jpeg.resize(256, 1);
    let photo = upload(&ts, &f.member, "?kind=photo", "image/jpeg", jpeg).await;

    for attachment in [video, ogg, big, photo] {
        let message = send(&ts, &f.member, f.chat, attachment).await;
        assert_error(
            ask(&ts, &f.member, f.chat, message, attachment).await,
            400,
            "not_transcribable",
        )
        .await;
    }
    assert!(stub.calls().is_empty(), "nothing left the server");

    // And at the ceiling exactly, it goes.
    let fits = upload(
        &ts,
        &f.member,
        "?kind=audio&duration_ms=1000",
        "audio/mp4",
        mp4_bytes(8, 1024),
    )
    .await;
    let message = send(&ts, &f.member, f.chat, fits).await;
    transcript_of(ask(&ts, &f.member, f.chat, message, fits).await).await;
}

/// A video's sound, supplied by the asking device: answered, NEVER kept, and
/// a second ask is a second call — the server cannot vouch for sound it did
/// not store.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn supplied_sound_for_a_video_is_answered_and_never_kept() {
    let (stub, addr) = spawn_stub().await;
    let ts = server(addr).await;
    let f = family(&ts).await;
    let video = upload(
        &ts,
        &f.member,
        "?kind=video&duration_ms=8400",
        "video/mp4",
        mp4_bytes(11, 4096),
    )
    .await;
    let message = send(&ts, &f.member, f.chat, video).await;
    let sound = mp4_bytes(12, 1500);

    let first = transcript_of(
        ask_with_sound(
            &ts,
            &f.member,
            (f.chat, message, video),
            "audio",
            sound.clone(),
        )
        .await,
    )
    .await;
    assert_eq!(first["text"], SPOKEN);
    let calls = stub.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].fields["file"].bytes, sound,
        "the SUPPLIED bytes went, not the video"
    );
    assert_eq!(
        calls[0].fields["file"].filename.as_deref(),
        Some("audio.m4a")
    );
    assert_eq!(kept(&ts, video).await, 0, "never kept");

    transcript_of(
        ask_with_sound(
            &ts,
            &f.member,
            (f.chat, message, video),
            "audio",
            sound.clone(),
        )
        .await,
    )
    .await;
    assert_eq!(stub.calls().len(), 2, "a second ask is a second call");
    assert_eq!(kept(&ts, video).await, 0);
    // Both calls were bills.
    assert_eq!(
        usage_rows(&ts).await,
        vec![(f.member_id, 1, 8400), (f.member_id, 1, 8400)]
    );

    // What the multipart form refuses: no `audio` part, an empty one, one
    // that is not MPEG-4 — and a kind with no sound at all.
    assert_error(
        ask_with_sound(
            &ts,
            &f.member,
            (f.chat, message, video),
            "sound",
            sound.clone(),
        )
        .await,
        400,
        "not_transcribable",
    )
    .await;
    assert_error(
        ask_with_sound(
            &ts,
            &f.member,
            (f.chat, message, video),
            "audio",
            Vec::new(),
        )
        .await,
        400,
        "not_transcribable",
    )
    .await;
    assert_error(
        ask_with_sound(
            &ts,
            &f.member,
            (f.chat, message, video),
            "audio",
            b"OggS and not mp4".to_vec(),
        )
        .await,
        400,
        "not_transcribable",
    )
    .await;
    let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0];
    jpeg.resize(256, 3);
    let photo = upload(&ts, &f.member, "?kind=photo", "image/jpeg", jpeg).await;
    let photo_message = send(&ts, &f.member, f.chat, photo).await;
    assert_error(
        ask_with_sound(
            &ts,
            &f.member,
            (f.chat, photo_message, photo),
            "audio",
            sound,
        )
        .await,
        400,
        "not_transcribable",
    )
    .await;
    assert_eq!(stub.calls().len(), 2, "none of those were sent");
}

/// One bill per recording, not one per reader: the second member who asks
/// gets the kept answer, and the provider hears nothing more.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_kept_answer_is_handed_to_the_next_asker_with_no_second_call() {
    let (log, _guard) = capture_log();
    let (stub, addr) = spawn_stub().await;
    let ts = server(addr).await;
    let f = family(&ts).await;
    ts.patch(&f.owner, "/families/mine", json!({"ai_transcripts": true}))
        .await;
    let note = voice_note(&ts, &f.member, 13).await;
    let message = send(&ts, &f.member, f.chat, note).await;

    let first = transcript_of(ask(&ts, &f.member, f.chat, message, note).await).await;
    let second = transcript_of(ask(&ts, &f.owner, f.chat, message, note).await).await;
    assert_eq!(first, second);
    assert_eq!(stub.calls().len(), 1, "no second call");
    assert_eq!(
        usage_rows(&ts).await,
        vec![(f.member_id, 1, 4200)],
        "no second bill"
    );

    // A device that SUPPLIES sound for an audio file that already has a
    // kept answer gets the kept answer, and its sound goes nowhere.
    let third = transcript_of(
        ask_with_sound(
            &ts,
            &f.owner,
            (f.chat, message, note),
            "audio",
            mp4_bytes(14, 900),
        )
        .await,
    )
    .await;
    assert_eq!(third, first);
    assert_eq!(stub.calls().len(), 1);
    assert!(log.text().contains("outcome=\"shared\""), "{}", log.text());

    // And the kept answer is still behind the rule: with the switch off
    // again the owner is refused, whatever is stored.
    ts.patch(&f.owner, "/families/mine", json!({"ai_transcripts": false}))
        .await;
    assert_error(
        ask(&ts, &f.owner, f.chat, message, note).await,
        403,
        "transcript_not_allowed",
    )
    .await;
}

/// Two asks at once for the same recording share ONE call.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn two_asks_at_once_share_one_call() {
    let (stub, addr) = spawn_stub().await;
    let ts = server(addr).await;
    let f = family(&ts).await;
    ts.patch(&f.owner, "/families/mine", json!({"ai_transcripts": true}))
        .await;
    let note = voice_note(&ts, &f.member, 15).await;
    let message = send(&ts, &f.member, f.chat, note).await;
    stub.slow(Duration::from_millis(600));

    let (a, b) = tokio::join!(
        ask(&ts, &f.member, f.chat, message, note),
        ask(&ts, &f.owner, f.chat, message, note),
    );
    let (a, b) = (transcript_of(a).await, transcript_of(b).await);
    assert_eq!(a, b);
    assert_eq!(stub.calls().len(), 1, "one call for both");
    assert_eq!(usage_rows(&ts).await.len(), 1, "and one bill");
    assert_eq!(kept(&ts, note).await, 1);
}

/// The asker gives up; the call does not. It finishes, is kept, and the next
/// ask is answered at once with no second call.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_asker_who_gives_up_still_leaves_the_answer_kept() {
    let (stub, addr) = spawn_stub().await;
    let ts = server(addr).await;
    let f = family(&ts).await;
    let note = voice_note(&ts, &f.member, 16).await;
    let message = send(&ts, &f.member, f.chat, note).await;
    stub.slow(Duration::from_millis(800));

    let impatient = reqwest::Client::builder()
        .timeout(Duration::from_millis(150))
        .build()
        .expect("client");
    let gave_up = impatient
        .post(ts.url(&path(f.chat, message, note)))
        .bearer_auth(&f.member)
        .send()
        .await;
    assert!(gave_up.is_err(), "the client stopped waiting");

    eventually("the abandoned call was never kept", async || {
        (kept(&ts, note).await == 1).then_some(())
    })
    .await;
    let transcript = transcript_of(ask(&ts, &f.member, f.chat, message, note).await).await;
    assert_eq!(transcript["text"], SPOKEN);
    assert_eq!(stub.calls().len(), 1, "no second call");
    assert_eq!(usage_rows(&ts).await, vec![(f.member_id, 1, 4200)]);
}

/// The provider's filter refusing is its own terminal answer — and nothing
/// is kept or counted.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_refusal_by_the_providers_filter_is_transcript_refused() {
    let (log, _guard) = capture_log();
    let (stub, addr) = spawn_stub().await;
    let ts = server(addr).await;
    let f = family(&ts).await;
    let note = voice_note(&ts, &f.member, 17).await;
    let message = send(&ts, &f.member, f.chat, note).await;
    stub.fail_with(
        400,
        json!({"error": {"code": "content_filter", "message": format!("blocked: {SPOKEN}"),
                         "innererror": {"code": "ResponsibleAIPolicyViolation"}}}),
    );

    assert_error(
        ask(&ts, &f.member, f.chat, message, note).await,
        400,
        "transcript_refused",
    )
    .await;
    assert_eq!(kept(&ts, note).await, 0);
    assert!(usage_rows(&ts).await.is_empty());
    let text = log.text();
    assert!(text.contains("outcome=\"refused\""), "{text}");
    assert!(
        !text.contains(SPOKEN),
        "the provider's message is not logged:\n{text}"
    );

    // Any other failure is `internal`, which a client retries.
    stub.fail_with(503, json!({"error": {"code": "ServiceUnavailable"}}));
    assert_error(
        ask(&ts, &f.member, f.chat, message, note).await,
        500,
        "internal",
    )
    .await;
    assert!(log.text().contains("outcome=\"failed\""));
}

/// The family's language goes as the bare two-letter hint.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_familys_language_goes_as_a_two_letter_hint() {
    let (stub, addr) = spawn_stub().await;
    let ts = server(addr).await;
    let f = family(&ts).await;
    let set = ts
        .patch(&f.owner, "/families/mine", json!({"language": "sr-Latn"}))
        .await;
    assert_eq!(set.status(), 200);
    let note = voice_note(&ts, &f.member, 18).await;
    let message = send(&ts, &f.member, f.chat, note).await;

    transcript_of(ask(&ts, &f.member, f.chat, message, note).await).await;
    assert_eq!(stub.calls()[0].fields["language"].text(), "sr");
}

/// The first three checks: here, and addressed correctly — and a server
/// without the deployment says so on its own code.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_address_is_checked_before_anything_else() {
    let (stub, addr) = spawn_stub().await;
    let ts = server(addr).await;
    let f = family(&ts).await;
    let note = voice_note(&ts, &f.member, 19).await;
    let message = send(&ts, &f.member, f.chat, note).await;
    let other = voice_note(&ts, &f.member, 20).await;
    let other_message = send(&ts, &f.member, f.chat, other).await;

    // An attachment that is not on THIS message.
    assert_error(
        ask(&ts, &f.member, f.chat, message, other).await,
        404,
        "attachment_not_found",
    )
    .await;
    // A message that is not in THIS chat: a real one, in a chat the asker
    // is in, named under the wrong chat.
    let direct: Value = ts
        .post(&f.member, "/chats/direct", json!({"user_id": f.owner_id}))
        .await
        .json()
        .await
        .expect("JSON");
    let dm = direct["chat"]["id"].as_i64().expect("chat id");
    assert_error(
        ask(&ts, &f.member, dm, other_message, other).await,
        404,
        "message_not_found",
    )
    .await;
    assert_error(
        ask(&ts, &f.member, f.chat, 9_999_999, note).await,
        404,
        "message_not_found",
    )
    .await;
    // Somebody not in the chat at all.
    let (stranger, _) = ts.register("stranger", "Stranger").await;
    assert_error(
        ask(&ts, &stranger, f.chat, message, note).await,
        403,
        "not_chat_member",
    )
    .await;
    assert_error(
        ask(&ts, &f.member, 9_999_999, message, note).await,
        404,
        "chat_not_found",
    )
    .await;
    assert!(stub.calls().is_empty());
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_server_without_the_deployment_answers_transcripts_unavailable() {
    let (stub, addr) = spawn_stub().await;
    let ts = server_tweaked(addr, |cfg| {
        cfg.ai.transcribe.deployment.deployment = String::new()
    })
    .await;
    let f = family(&ts).await;
    let note = voice_note(&ts, &f.member, 21).await;
    let message = send(&ts, &f.member, f.chat, note).await;
    assert_error(
        ask(&ts, &f.member, f.chat, message, note).await,
        403,
        "transcripts_unavailable",
    )
    .await;
    assert!(stub.calls().is_empty());

    // And it says so where a client looks first: `transcribe` false, and no
    // ceiling for a capability it does not have.
    let mine: Value = ts
        .get(&f.member, "/families/mine")
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(mine["assistant"]["transcribe"], false, "{mine}");
    assert!(
        mine["assistant"].get("transcribe_max_bytes").is_none(),
        "{mine}"
    );
}

/// What a client reads: the new keys, beside every key that was already
/// there — on both objects a client bootstraps from.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_family_and_the_assistant_carry_the_new_keys_beside_the_old() {
    let (_stub, addr) = spawn_stub().await;
    let ts = server(addr).await;
    let f = family(&ts).await;

    let mine: Value = ts
        .get(&f.member, "/families/mine")
        .await
        .json()
        .await
        .expect("JSON");
    let me: Value = ts.get(&f.member, "/me").await.json().await.expect("JSON");
    for family in [&mine["family"], &me["family"]] {
        for key in [
            "id",
            "name",
            "join_policy",
            "created_at",
            "ai_history",
            "ai_vision",
            "ai_history_photos",
            "ai_greeting",
            "ai_faces",
        ] {
            assert!(family.get(key).is_some(), "{key} is still there: {family}");
        }
        assert_eq!(
            family["ai_transcripts"], false,
            "present, and off by default: {family}"
        );
    }
    let assistant = &mine["assistant"];
    for key in [
        "user_id",
        "display_name",
        "mention",
        "draw",
        "vision",
        "images",
        "processor",
    ] {
        assert!(
            assistant.get(key).is_some(),
            "{key} is still there: {assistant}"
        );
    }
    assert_eq!(assistant["transcribe"], true, "{assistant}");
    assert_eq!(
        assistant["transcribe_max_bytes"],
        25 * 1024 * 1024,
        "{assistant}"
    );

    // A member may read the switch and may not change it.
    let refused = ts
        .patch(&f.member, "/families/mine", json!({"ai_transcripts": true}))
        .await;
    assert_error(refused, 403, "not_family_owner").await;
    // The owner may, and it is bound to no other switch: turning
    // `ai_vision` on and off leaves it where it was.
    let on: Value = ts
        .patch(&f.owner, "/families/mine", json!({"ai_transcripts": true}))
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(on["family"]["ai_transcripts"], true);
    let after: Value = ts
        .patch(&f.owner, "/families/mine", json!({"ai_vision": true}))
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(after["family"]["ai_transcripts"], true);
    let after: Value = ts
        .patch(&f.owner, "/families/mine", json!({"ai_vision": false}))
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(after["family"]["ai_transcripts"], true);
    let me: Value = ts.get(&f.member, "/me").await.json().await.expect("JSON");
    assert_eq!(me["family"]["ai_transcripts"], true, "both reads agree");
}

/// Retention and deletion need nothing new: the kept answer goes with the
/// attachment row, by cascade.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_kept_answer_goes_with_its_recording() {
    let (_stub, addr) = spawn_stub().await;
    let ts = server(addr).await;
    let f = family(&ts).await;
    let note = voice_note(&ts, &f.member, 22).await;
    let message = send(&ts, &f.member, f.chat, note).await;
    transcript_of(ask(&ts, &f.member, f.chat, message, note).await).await;
    assert_eq!(kept(&ts, note).await, 1);

    sqlx::query("DELETE FROM messages WHERE id = $1")
        .bind(message)
        .execute(&ts.state.pool)
        .await
        .expect("deleting the message as retention does");
    assert_eq!(kept(&ts, note).await, 0, "gone with the attachment row");
}
