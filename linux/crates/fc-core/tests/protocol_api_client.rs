//! The REST transport: the token on every call, the error shape read as the protocol writes it,
//! and ONE retry on a read that hit a transient status (docs/protocol.md, "Transport", "Error
//! shape") — a port of `win/tests/FamilyConnect.Core.Tests/Protocol/ApiClientTests.cs`, test for
//! test (its one model-only test, `the_limits_are_both_or_nothing`, is in
//! `protocol_api_client_model.rs`), and the pins of what .NET's own parsers did that the client
//! leaned on (`Retry-After`, a body's byte-order mark, a failure's reason phrase).
//!
//! Driven through the fake transport, which is what makes the client testable at all — no server
//! and no network, every request the client made kept for inspection, and every deadline and wait
//! run on the fake clock (`testing::drive`), so nothing here waits for real.

use std::future::Future;
use std::rc::Rc;
use std::time::Duration;

use fc_core::protocol::api_client::{
    BACKDROP_TIMEOUT, ORDINARY_TIMEOUT, TRANSCRIPT_TIMEOUT, retry_after,
};
use fc_core::protocol::client_frames::SendOptions;
use fc_core::protocol::{
    ApiClient, FamilyPatch, MemoryTokenStore, NotePatch, NoteRequest, RoundVideoLimits, ServerUrl,
    TaskLineRequest, UploadFacts, error_codes,
};
use fc_core::rt::{Clock, HttpRequest, Method, TransportError};
use fc_core::testing::{FakeClock, FakeHttp, drive, json_response};
use serde_json::Value;
use time::macros::datetime;

/// A client over a fake transport, with the token the C# tests' `Client` gave it.
struct Harness {
    api: ApiClient,
    http: FakeHttp,
    clock: FakeClock,
}

impl Harness {
    fn new() -> Self {
        Harness::with_token(Some("t0ken"))
    }

    fn with_token(token: Option<&str>) -> Self {
        Harness::configured(token, |api| api)
    }

    /// The C# tests' `new ApiClient(...) { RequestDeadline = …, … }`.
    fn configured(token: Option<&str>, configure: impl FnOnce(ApiClient) -> ApiClient) -> Self {
        let http = FakeHttp::new();
        let clock = FakeClock::at(datetime!(2026-10-28 12:00 UTC));
        let api = ApiClient::new(
            Rc::new(http.clone()),
            ServerUrl::normalise(Some("chat.example.com")).expect("a server"),
            Rc::new(MemoryTokenStore::new(token)),
            Rc::new(clock.clone()),
        );
        Harness {
            api: configure(api),
            http,
            clock,
        }
    }

    fn run<T>(&self, future: impl Future<Output = T>) -> T {
        drive(&self.clock, future)
    }

    fn sent(&self) -> Vec<HttpRequest> {
        self.http.sent()
    }

    /// The one request sent.
    fn single(&self) -> HttpRequest {
        let sent = self.sent();
        assert_eq!(sent.len(), 1, "exactly one request");
        sent.into_iter().next().expect("one request")
    }

    /// Each request's body as text, `None` where it had none.
    fn bodies(&self) -> Vec<Option<String>> {
        self.sent()
            .iter()
            .map(|request| {
                request
                    .body
                    .as_ref()
                    .map(|body| String::from_utf8_lossy(body).into_owned())
            })
            .collect()
    }

    /// The one body sent.
    fn single_body(&self) -> String {
        let bodies = self.bodies();
        assert_eq!(bodies.len(), 1, "exactly one request");
        bodies[0].clone().expect("a body")
    }

    /// Each request's method and URL.
    fn asked(&self) -> Vec<(Method, String)> {
        self.sent()
            .into_iter()
            .map(|request| (request.method, request.url))
            .collect()
    }
}

/// A request's own type, without its parameters (`ContentType.MediaType`).
fn media_type(request: &HttpRequest) -> Option<String> {
    request.header_values("Content-Type").first().map(|value| {
        value
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_string()
    })
}

fn json(text: &str) -> Value {
    serde_json::from_str(text).expect("the body is JSON")
}

fn unreachable() -> Result<fc_core::rt::HttpResponse, TransportError> {
    Err(TransportError::Unreachable("connection reset".into()))
}

const BOARD: &str = "/api/v1/families/mine/board";

#[test]
fn every_call_goes_under_api_v1_with_the_bearer_token() {
    let h = Harness::new();
    h.http.respond(
        Method::Get,
        BOARD,
        200,
        r#"{"notes": [], "max_board_seq": 0}"#,
    );
    let answer = h.run(h.api.board(None));
    assert!(answer.ok());
    let request = h.single();
    assert_eq!(
        request.url,
        "https://chat.example.com/api/v1/families/mine/board"
    );
    assert_eq!(request.header_values("Authorization"), ["Bearer t0ken"]);
    // And the token is NOWHERE in the URL: "a token in the query string is not a token at all".
    assert!(!request.url.contains("t0ken"));
}

#[test]
fn signing_in_sends_no_token_because_there_is_not_one_yet() {
    let h = Harness::with_token(None);
    h.http.respond(
        Method::Post,
        "/api/v1/auth/login",
        200,
        r#"{"token": "fresh", "user": {"id": 7, "username": "anna", "display_name": "Anna"}}"#,
    );
    let answer = h.run(h.api.log_in("anna", "hunter2"));
    assert!(answer.ok());
    let value = answer.value().expect("an answer");
    assert_eq!(value.token, "fresh");
    assert_eq!(value.user.display_name, "Anna");
    assert!(h.single().header_values("Authorization").is_empty());
    assert!(h.single_body().contains(r#""username":"anna""#));
}

#[test]
fn a_204_is_a_success_with_nothing_in_it() {
    let h = Harness::new();
    h.http.respond(
        Method::Delete,
        "/api/v1/families/mine/board/notes/12",
        204,
        "",
    );
    let answer = h.run(h.api.delete_note(12));
    assert!(answer.ok());
    assert!(answer.error().is_none());
}

#[test]
fn the_error_shape_is_read_as_the_protocol_writes_it() {
    let h = Harness::new();
    h.http.respond(
        Method::Post,
        "/api/v1/families/mine/board/notes/12/backdrop",
        403,
        r#"{"error": {"code": "pictures_unavailable", "message": "this server cannot draw"}}"#,
    );
    let answer = h.run(h.api.draw_backdrop(12));
    assert!(!answer.ok());
    let error = answer.error().expect("a failure");
    assert_eq!(error.code, error_codes::PICTURES_UNAVAILABLE);
    assert_eq!(error.message, "this server cannot draw");
    assert_eq!(error.status, 403);
    // A refusal, so nothing retries it.
    assert!(!error.transient());
}

/// nginx answers its own rate limit with an HTML body, so the status alone has to be enough.
#[test]
fn a_429_with_no_json_at_all_is_still_a_transient_failure_with_its_wait() {
    let h = Harness::new();
    for _ in 0..2 {
        let mut response = json_response(429, "", &[("Retry-After", "2")]);
        response.body = b"<html>429 Too Many Requests</html>".to_vec();
        h.http.answer(Method::Get, BOARD, Ok(response));
    }
    let answer = h.run(h.api.board(None));
    assert!(!answer.ok());
    let error = answer.error().expect("a failure");
    assert_eq!(error.code, error_codes::TOO_MANY_REQUESTS);
    assert!(error.transient());
    assert_eq!(error.retry_after, Some(time::Duration::seconds(2)));
    // The read waited the server's own two seconds before its one retry.
    assert!(h.clock.slept().contains(&Duration::from_secs(2)));
    assert_eq!(h.http.unused(), 0);
}

/// ONE retry, and only on a read: a GET is safe to repeat, and the alternative is a board that
/// empties itself because a proxy hiccuped.
#[test]
fn a_read_is_tried_once_more_after_a_transient_failure() {
    let h = Harness::new();
    h.http
        .respond(Method::Get, BOARD, 502, "<html>502</html>")
        .respond(
            Method::Get,
            BOARD,
            200,
            r#"{"notes": [], "max_board_seq": 4}"#,
        );
    let answer = h.run(h.api.board(None));
    assert!(answer.ok());
    assert_eq!(answer.value().expect("a board").max_board_seq, 4);
    assert_eq!(h.sent().len(), 2);
}

#[test]
fn and_only_once() {
    let h = Harness::new();
    h.http
        .respond(Method::Get, BOARD, 502, "<html>502</html>")
        .respond(Method::Get, BOARD, 502, "<html>502</html>");
    let answer = h.run(h.api.board(None));
    assert!(!answer.ok());
    assert!(answer.error().expect("a failure").transient());
    assert_eq!(h.sent().len(), 2);
}

/// A WRITE is never retried here, whatever went wrong: only the outbox knows whether the request
/// carries a dedup key, and a retried POST without one is a second message.
#[test]
fn a_write_is_never_retried() {
    let h = Harness::new();
    h.http.respond(
        Method::Post,
        "/api/v1/chats/42/messages",
        502,
        "<html>502</html>",
    );
    let answer =
        h.run(
            h.api
                .send_message(42, "8f14e45f-…", "Dinner at 7?", &SendOptions::default()),
        );
    assert!(!answer.ok());
    assert!(
        answer.error().expect("a failure").transient(),
        "still transient — the outbox will try again"
    );
    assert_eq!(h.sent().len(), 1);
}

#[test]
fn a_request_that_never_arrived_is_a_transport_failure() {
    let h = Harness::new();
    h.http
        .answer(Method::Get, BOARD, unreachable())
        .answer(Method::Get, BOARD, unreachable());
    let answer = h.run(h.api.board(None));
    assert!(!answer.ok());
    let error = answer.error().expect("a failure");
    assert_eq!(error.code, error_codes::TRANSPORT);
    assert!(error.transient());
    // A read: tried once more, and then reported.
    assert_eq!(h.sent().len(), 2);
    assert_eq!(h.http.unused(), 0);
}

#[test]
fn a_send_carries_its_dedup_key_and_only_the_fields_that_apply() {
    let h = Harness::new();
    h.http.respond(
        Method::Post,
        "/api/v1/chats/42/messages",
        201,
        r#"{"message": {"id": 1338, "chat_id": 42, "sender_id": 7, "client_msg_id": "k", "body": "hi", "created_at": "…"}}"#,
    );
    let answer = h.run(h.api.send_message(42, "k", "hi", &SendOptions::default()));
    assert!(answer.ok());
    assert_eq!(answer.value().expect("a message").message.id, 1338);
    let body = h.single_body();
    assert!(body.contains(r#""client_msg_id":"k""#));
    // Absent, not null: a field this message does not have is a field the server never sees.
    assert!(!body.contains("reply_to_message_id"));
    assert!(!body.contains("attachment_ids"));
    assert!(!body.contains("poll"));
    assert!(!body.contains("mentions"));
}

#[test]
fn a_patch_sends_only_what_changed() {
    let h = Harness::new();
    h.http.respond(
        Method::Patch,
        "/api/v1/families/mine/board/notes/12",
        200,
        r#"{"note": {"id": 12, "author_id": 7, "text": "Milk", "board_seq": 9}}"#,
    );
    let patch = NotePatch {
        x: Some(0.4),
        y: Some(0.6),
        ..NotePatch::default()
    };
    let answer = h.run(h.api.patch_note(12, &patch));
    assert!(answer.ok());
    assert_eq!(h.single_body(), r#"{"x":0.4,"y":0.6}"#);
    assert_eq!(h.single().method, Method::Patch);
}

#[test]
fn a_new_list_sends_its_lines_without_ids() {
    let h = Harness::new();
    h.http.respond(
        Method::Post,
        "/api/v1/families/mine/board/notes",
        201,
        r#"{"note": {"id": 13, "author_id": 7, "kind": "tasks", "text": "Saturday", "board_seq": 10, "items": []}}"#,
    );
    let note = NoteRequest {
        kind: Some("tasks".into()),
        items: Some(vec![
            TaskLineRequest::new("Milk", None),
            TaskLineRequest::new("Bread", None),
        ]),
        ..NoteRequest::new("Saturday", "green", 0.1, 0.1)
    };
    let answer = h.run(h.api.create_note(&note));
    assert!(answer.ok());
    let body = h.single_body();
    assert!(body.contains(r#""items":[{"text":"Milk"},{"text":"Bread"}]"#));
    // "An `id` on a created item is refused: ids are the server's."
    assert!(!body.contains(r#""id""#));
}

#[test]
fn a_rewritten_line_keeps_its_id_and_therefore_its_tick() {
    let h = Harness::new();
    h.http.respond(
        Method::Patch,
        "/api/v1/families/mine/board/notes/13",
        200,
        r#"{"note": {"id": 13, "author_id": 7, "kind": "tasks", "text": "Saturday", "board_seq": 11}}"#,
    );
    let patch = NotePatch {
        items: Some(vec![
            TaskLineRequest::new("Milk and eggs", Some(11)),
            TaskLineRequest::new("Jam", None),
        ]),
        ..NotePatch::default()
    };
    h.run(h.api.patch_note(13, &patch));
    let body = h.single_body();
    assert!(body.contains(r#"{"text":"Milk and eggs","id":11}"#));
    assert!(body.contains(r#"{"text":"Jam"}"#));
}

/// A backdrop the author has not agreed to (docs/protocol.md, "Consenting to the assistant",
/// amended 2026-09-30): a refusal with nothing sent, read as its code — the one the board answers
/// with the consent question.
#[test]
fn a_backdrop_without_consent_is_refused_by_its_code() {
    let h = Harness::new();
    h.http.respond(
        Method::Post,
        "/api/v1/families/mine/board/notes/12/backdrop",
        403,
        r#"{"error": {"code": "assistant_consent_required", "message": "agree first"}}"#,
    );
    let answer = h.run(h.api.draw_backdrop(12));
    assert!(!answer.ok());
    let error = answer.error().expect("a failure");
    assert_eq!(error.code, error_codes::ASSISTANT_CONSENT_REQUIRED);
    assert_eq!(error.status, 403);
    assert!(!error.transient());
    // A write, and a refusal: asked once, never repeated here.
    assert_eq!(h.sent().len(), 1);
}

/// SLOW (docs/protocol.md, "Board", amended 2026-09-30): the backdrop gets "a timeout of its own,
/// no shorter than 90 s … never its ordinary request timeout" — and nothing in the transport caps
/// it. The C# asserted that the `HttpClient` the app builds has no timeout of its own; here the
/// transport is handed each request's own deadline, so the pin is that the backdrop's request
/// carries the backdrop's and an ordinary one the ordinary.
#[test]
fn the_backdrop_has_a_deadline_of_its_own_that_nothing_caps() {
    assert!(BACKDROP_TIMEOUT >= Duration::from_secs(90));
    assert!(BACKDROP_TIMEOUT > ORDINARY_TIMEOUT);
    let h = Harness::new();
    assert_eq!(h.api.backdrop_deadline(), BACKDROP_TIMEOUT);
    assert_eq!(h.api.request_deadline(), ORDINARY_TIMEOUT);
    h.http
        .respond(
            Method::Post,
            "/api/v1/families/mine/board/notes/12/backdrop",
            200,
            r#"{"note": {"id": 12, "author_id": 7}}"#,
        )
        .respond(Method::Get, BOARD, 200, r#"{"notes": []}"#);
    h.run(h.api.draw_backdrop(12));
    h.run(h.api.board(None));
    let sent = h.sent();
    assert_eq!(sent[0].timeout, Some(BACKDROP_TIMEOUT));
    assert_eq!(sent[1].timeout, Some(ORDINARY_TIMEOUT));
}

/// The same rule, run: a server slower than the ORDINARY deadline fails an ordinary request, and
/// still answers a backdrop — the backdrop does not ride on the timeout every other call gets.
#[test]
fn a_backdrop_outlasts_the_ordinary_deadline() {
    let h = Harness::configured(Some("t0ken"), |api| {
        api.with_request_deadline(Duration::from_millis(50))
            .with_backdrop_deadline(Duration::from_secs(30))
    });
    // One answer only: a request whose deadline passes never takes its answer from the script.
    h.http.respond(
        Method::Post,
        "/api/v1/families/mine/board/notes/12/backdrop",
        200,
        r#"{"note": {"id": 12, "author_id": 7, "kind": "event", "text": "Lunch", "board_seq": 12}}"#,
    );
    // The two reads wait ten seconds, so only the 50 ms deadline can end them.
    h.http
        .delay(&h.clock, Duration::from_secs(10))
        .delay(&h.clock, Duration::from_secs(10))
        // The backdrop is slow too — slower than the ordinary deadline — and finishes inside its
        // own.
        .delay(&h.clock, Duration::from_millis(400));
    // A read, so it is tried twice — and both run out of time.
    let board = h.run(h.api.board(None));
    assert!(!board.ok());
    assert_eq!(
        board.error().expect("a failure").code,
        error_codes::TRANSPORT
    );
    // The deadlines ended the reads, not the server: 50 ms, the 250 ms pause, 50 ms.
    let start = datetime!(2026-10-28 12:00 UTC);
    assert_eq!(h.clock.now(), start + Duration::from_millis(350));
    let drawn = h.run(h.api.draw_backdrop(12));
    assert!(drawn.ok());
    assert_eq!(drawn.value().expect("a note").note.id, 12);
    assert_eq!(h.clock.now(), start + Duration::from_millis(750));
    assert_eq!(h.http.unused(), 0);
}

#[test]
fn the_backdrop_asks_with_no_body_at_all() {
    let h = Harness::new();
    h.http.respond(
        Method::Post,
        "/api/v1/families/mine/board/notes/12/backdrop",
        200,
        r#"{"note": {"id": 12, "author_id": 7, "kind": "event", "text": "Lunch", "board_seq": 12}}"#,
    );
    let answer = h.run(h.api.draw_backdrop(12));
    assert!(answer.ok());
    // The prompt is the note's TITLE: a request body would be a second way to send words to a
    // model from a screen that is not the assistant's chat.
    assert_eq!(h.bodies(), [None]);
    assert_eq!(
        h.single().url,
        "https://chat.example.com/api/v1/families/mine/board/notes/12/backdrop"
    );
}

#[test]
fn ticks_and_answers_are_states_and_not_toggles() {
    let h = Harness::new();
    h.http
        .respond(
            Method::Put,
            "/api/v1/families/mine/board/notes/13/tasks/11",
            200,
            r#"{"note": {"id": 13, "author_id": 7, "board_seq": 13}}"#,
        )
        .respond(
            Method::Put,
            "/api/v1/families/mine/board/notes/12/rsvp",
            200,
            r#"{"note": {"id": 12, "author_id": 7, "board_seq": 14}}"#,
        );
    h.run(h.api.tick_task(13, 11, true));
    h.run(h.api.answer(12, "going"));
    let bodies = h.bodies();
    assert_eq!(bodies[0].as_deref(), Some(r#"{"done":true}"#));
    assert_eq!(bodies[1].as_deref(), Some(r#"{"answer":"going"}"#));
    let sent = h.sent();
    assert_eq!(sent[0].method, Method::Put);
    assert_eq!(
        sent[0].url,
        "https://chat.example.com/api/v1/families/mine/board/notes/13/tasks/11"
    );
}

#[test]
fn an_upload_puts_its_bytes_in_the_body_and_its_facts_in_the_query() {
    let h = Harness::new();
    h.http.respond(
        Method::Post,
        "/api/v1/attachments",
        201,
        r#"{"attachment": {"id": 34, "kind": "photo", "mime": "image/jpeg", "width": 600, "height": 1200}}"#,
    );
    let facts = UploadFacts {
        width: Some(600),
        height: Some(1200),
        ..UploadFacts::default()
    };
    let answer = h.run(h.api.upload("photo", "image/jpeg", vec![1, 2, 3], &facts));
    assert!(answer.ok());
    assert_eq!(
        answer
            .value()
            .expect("an attachment")
            .attachment
            .aspect_ratio(),
        Some(0.5)
    );
    let request = h.single();
    assert_eq!(
        request.url,
        "https://chat.example.com/api/v1/attachments?kind=photo&width=600&height=1200"
    );
    assert_eq!(media_type(&request).as_deref(), Some("image/jpeg"));
    assert_eq!(request.body.as_deref(), Some(&[1u8, 2, 3][..]));
}

/// A voice note's waveform (docs/protocol.md, "A voice note's waveform") rides in the query on
/// AUDIO only, and only when it is exactly the wire; the answer's echo lands on the attachment,
/// and an answer without one (a server from before waveforms) is still a success.
#[test]
fn a_voice_notes_waveform_goes_in_the_query_on_audio_only() {
    const WIRE: &str = "0123456789abcdef0123456789abcdef0123456789abcdef";
    let h = Harness::new();
    h.http
        .respond(
            Method::Post,
            "/api/v1/attachments",
            201,
            &format!(
                r#"{{"attachment": {{"id": 40, "kind": "audio", "mime": "audio/mp4", "duration_ms": 4200, "waveform": "{WIRE}"}}}}"#
            ),
        )
        .respond(
            Method::Post,
            "/api/v1/attachments",
            201,
            r#"{"attachment": {"id": 41, "kind": "audio", "mime": "audio/mp4", "duration_ms": 4200}}"#,
        )
        .respond(
            Method::Post,
            "/api/v1/attachments",
            201,
            r#"{"attachment": {"id": 42, "kind": "audio", "mime": "audio/mp4"}}"#,
        )
        .respond(
            Method::Post,
            "/api/v1/attachments",
            201,
            r#"{"attachment": {"id": 43, "kind": "video", "mime": "video/mp4"}}"#,
        );
    let voice = UploadFacts {
        duration_ms: Some(4200),
        waveform: Some(WIRE),
        ..UploadFacts::default()
    };
    let echoed = h.run(h.api.upload("audio", "audio/mp4", vec![1], &voice));
    assert_eq!(
        echoed
            .value()
            .expect("an attachment")
            .attachment
            .waveform
            .as_deref(),
        Some(WIRE)
    );
    let old = h.run(h.api.upload("audio", "audio/mp4", vec![1], &voice));
    assert!(old.ok());
    assert_eq!(
        old.value().expect("an attachment").attachment.waveform,
        None
    );
    let upper = WIRE.to_uppercase();
    let shouted = UploadFacts {
        waveform: Some(&upper),
        ..UploadFacts::default()
    };
    h.run(h.api.upload("audio", "audio/mp4", vec![1], &shouted));
    let on_video = UploadFacts {
        waveform: Some(WIRE),
        ..UploadFacts::default()
    };
    h.run(h.api.upload("video", "video/mp4", vec![1], &on_video));
    let sent = h.sent();
    assert_eq!(
        sent[0].url,
        format!(
            "https://chat.example.com/api/v1/attachments?kind=audio&duration_ms=4200&waveform={WIRE}"
        )
    );
    assert_eq!(
        sent[2].url,
        "https://chat.example.com/api/v1/attachments?kind=audio"
    );
    assert_eq!(
        sent[3].url,
        "https://chat.example.com/api/v1/attachments?kind=video"
    );
}

/// A preview is PUT as raw JPEG to its attachment, and the server's 204 is success — the empty
/// answer that a reader expecting JSON would call unreadable.
#[test]
fn a_preview_is_put_as_raw_jpeg_and_a_204_is_success() {
    let h = Harness::new();
    h.http
        .respond(Method::Put, "/api/v1/attachments/34/preview", 204, "");
    let answer = h.run(h.api.upload_preview(34, b"jpeg".to_vec()));
    assert!(answer.ok());
    let request = h.single();
    assert_eq!(request.method, Method::Put);
    assert_eq!(
        request.url,
        "https://chat.example.com/api/v1/attachments/34/preview"
    );
    assert_eq!(media_type(&request).as_deref(), Some("image/jpeg"));
    assert_eq!(h.single_body(), "jpeg");
}

#[test]
fn an_attachment_comes_back_as_bytes() {
    let h = Harness::new();
    h.http
        .respond(Method::Get, "/api/v1/attachments/34/preview", 200, "");
    let answer = h.run(h.api.download(34, true));
    assert!(answer.ok());
    assert_eq!(
        h.single().url,
        "https://chat.example.com/api/v1/attachments/34/preview"
    );
}

/// A 401 sends the client back to login — EXCEPT `invalid_credentials`, which is a password that
/// was wrong and leaves the session alone.
#[test]
fn an_expired_session_and_a_wrong_password_are_the_same_status_and_not_the_same_thing() {
    let h = Harness::new();
    h.http.respond(
        Method::Get,
        "/api/v1/me",
        401,
        r#"{"error": {"code": "unauthorized", "message": "session gone"}}"#,
    );
    let gone = h.run(h.api.me());
    assert!(gone.error().expect("a failure").session_gone());

    let again = Harness::with_token(None);
    again.http.respond(
        Method::Post,
        "/api/v1/auth/login",
        401,
        r#"{"error": {"code": "invalid_credentials", "message": "wrong password"}}"#,
    );
    let wrong = again.run(again.api.log_in("anna", "nope"));
    assert!(!wrong.error().expect("a failure").session_gone());
}

#[test]
fn this_servers_capabilities_arrive_with_the_answer_rather_than_as_errors_later() {
    let h = Harness::new();
    h.http.respond(
        Method::Get,
        "/api/v1/families/mine",
        200,
        r#"
        {"family": {"id": 3, "name": "The Smiths", "join_policy": "open", "ai_history": true},
         "members": [{"id": 7, "username": "anna", "display_name": "Anna", "role": "owner"}],
         "blocked_user_ids": [],
         "assistant": {"user_id": 1, "display_name": "Assistant", "mention": "@ai",
                       "draw": "/draw", "vision": true, "images": true}}
        "#,
    );
    let answer = h.run(h.api.family(None));
    assert!(answer.ok());
    let value = answer.value().expect("a family");
    assert_eq!(value.family.name, "The Smiths");
    assert!(value.has_assistant());
    // What the board's backdrop button hangs on.
    assert!(value.can_draw_pictures());
    assert_eq!(value.blocked_user_ids.as_deref(), Some(&[][..]));
    // The owner's cap is ABSENT here, which is not the same as the operator's ceiling.
    assert_eq!(value.family.max_members, None);
}

#[test]
fn a_server_with_no_assistant_offers_none() {
    let h = Harness::new();
    h.http.respond(
        Method::Get,
        "/api/v1/families/mine",
        200,
        r#"{"family": {"id": 3, "name": "The Smiths"}}"#,
    );
    let answer = h.run(h.api.family(None));
    assert!(answer.ok());
    let value = answer.value().expect("a family");
    assert!(!value.has_assistant());
    assert!(!value.can_draw_pictures());
}

#[test]
fn a_2xx_whose_body_cannot_be_read_is_not_a_success() {
    let h = Harness::new();
    h.http.respond(
        Method::Get,
        BOARD,
        200,
        "<html>a proxy's idea of JSON</html>",
    );
    let answer = h.run(h.api.board(None));
    assert!(!answer.ok());
    // Terminal: repeating the call produces the same body.
    assert!(!answer.error().expect("a failure").transient());
}

#[test]
fn the_socket_url_follows_the_same_server() {
    let h = Harness::new();
    assert_eq!(
        h.api.socket_url(),
        "wss://chat.example.com/api/v1/ws?families=1"
    );
    assert_eq!(h.api.base_url().to_string(), "https://chat.example.com/");
}

/// A vote PUTs the option, a retraction DELETEs and a close POSTs, neither with a body — and each
/// is answered with the poll's whole state. The open polls are a plain read of messages.
#[test]
fn a_poll_is_voted_retracted_closed_and_listed_on_its_own_paths() {
    const STATE: &str = r#"{"message_id": 5, "poll": {"poll_seq": 12, "closed": false, "options": [{"id": 1, "text": "Pizza", "votes": [7, 9]}, {"id": 2, "text": "Pasta", "votes": []}]}}"#;
    let h = Harness::new();
    h.http
        .respond(Method::Put, "/api/v1/chats/42/messages/5/vote", 200, STATE)
        .respond(Method::Delete, "/api/v1/chats/42/messages/5/vote", 200, STATE)
        .respond(
            Method::Post,
            "/api/v1/chats/42/messages/5/poll/close",
            200,
            STATE,
        )
        .respond(
            Method::Get,
            "/api/v1/chats/42/polls/open",
            200,
            r#"{"messages": [{"id": 5, "chat_id": 42, "sender_id": 7, "body": "Dinner?", "created_at": "2026-09-14T10:00:00Z", "poll": {"poll_seq": 12, "closed": false, "options": [{"id": 1, "text": "Pizza", "votes": [7]}]}}]}"#,
        );

    let voted = h.run(h.api.vote(42, 5, 1));
    let voted = voted.value().expect("a poll");
    assert_eq!(voted.message_id, 5);
    assert_eq!(voted.poll.poll_seq, 12);
    assert_eq!(voted.poll.options[0].votes, [7, 9]);
    assert!(h.run(h.api.unvote(42, 5)).ok());
    assert!(h.run(h.api.close_poll(42, 5)).ok());
    let open = h.run(h.api.open_polls(42));
    let messages = open
        .value()
        .and_then(|value| value.messages.clone())
        .unwrap_or_default();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].body, "Dinner?");

    assert_eq!(
        h.asked(),
        [
            (
                Method::Put,
                "https://chat.example.com/api/v1/chats/42/messages/5/vote".to_string()
            ),
            (
                Method::Delete,
                "https://chat.example.com/api/v1/chats/42/messages/5/vote".to_string()
            ),
            (
                Method::Post,
                "https://chat.example.com/api/v1/chats/42/messages/5/poll/close".to_string()
            ),
            (
                Method::Get,
                "https://chat.example.com/api/v1/chats/42/polls/open".to_string()
            ),
        ]
    );
    assert_eq!(
        h.bodies(),
        [Some(r#"{"option_id":1}"#.to_string()), None, None, None]
    );
}

#[test]
fn a_direct_chat_is_asked_for_by_the_members_id() {
    let h = Harness::new();
    h.http.respond(
        Method::Post,
        "/api/v1/chats/direct",
        200,
        r#"{"chat": {"id": 42, "kind": "direct", "title": "Bob", "peer_user_id": 9}}"#,
    );
    let answer = h.run(h.api.direct_chat(9));
    assert!(answer.ok());
    let chat = &answer.value().expect("a chat").chat;
    assert_eq!(chat.id, 42);
    assert_eq!(chat.peer_user_id, Some(9));
    let request = h.single();
    assert_eq!(request.method, Method::Post);
    assert_eq!(request.url, "https://chat.example.com/api/v1/chats/direct");
    assert_eq!(h.single_body(), r#"{"user_id":9}"#);
}

/// An event's end taken off is a NULL on the wire; a field left out leaves it alone.
#[test]
fn taking_an_events_end_off_sends_a_null_and_leaving_it_alone_sends_nothing() {
    let h = Harness::new();
    h.http
        .respond(
            Method::Patch,
            "/api/v1/families/mine/board/notes/12",
            200,
            r#"{"note": {"id": 12, "author_id": 7, "board_seq": 9}}"#,
        )
        .respond(
            Method::Patch,
            "/api/v1/families/mine/board/notes/12",
            200,
            r#"{"note": {"id": 12, "author_id": 7, "board_seq": 10}}"#,
        );
    let cleared = NotePatch {
        place: Some(String::new()),
        clears_end: true,
        ..NotePatch::default()
    };
    assert!(h.run(h.api.patch_note(12, &cleared)).ok());
    let alone = NotePatch {
        place: Some("Park".into()),
        ..NotePatch::default()
    };
    assert!(h.run(h.api.patch_note(12, &alone)).ok());

    let bodies = h.bodies();
    let cleared = json(bodies[0].as_deref().expect("a body"));
    assert_eq!(cleared["ends_at"], Value::Null);
    assert!(cleared.get("ends_at").is_some());
    assert_eq!(cleared["place"], "");
    assert!(cleared.get("clears_end").is_none());
    assert!(cleared.get("text").is_none());
    let alone = json(bodies[1].as_deref().expect("a body"));
    assert!(alone.get("ends_at").is_none());
    assert_eq!(h.sent()[1].method, Method::Patch);
}

// ---- the sticker pack (docs/protocol.md, "Sticker pack") ------------------------------------

const PACK_ITEM: &str = r#"
    {"id": 5, "added_by": 7,
     "attachment": {"id": 71, "kind": "photo", "mime": "image/webp", "size": 40960, "width": 512, "height": 512},
     "created_at": "2026-09-13T10:00:00Z", "pack_seq": 12}
    "#;

#[test]
fn the_pack_is_read_whole_and_caught_up_on_by_its_own_sequence() {
    let h = Harness::new();
    h.http
        .respond(
            Method::Get,
            "/api/v1/families/mine/pack",
            200,
            &format!(r#"{{"items": [{PACK_ITEM}], "max_pack_seq": 12}}"#),
        )
        .respond(
            Method::Get,
            "/api/v1/families/mine/pack/changes",
            200,
            r#"{"items": [{"id": 5, "deleted": true, "pack_seq": 14}]}"#,
        );

    let whole = h.run(h.api.pack(None));
    let whole = whole.value().expect("a pack");
    assert_eq!(whole.max_pack_seq, 12);
    let items = whole.items.clone().unwrap_or_default();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].attachment.as_ref().map(|a| a.id), Some(71));

    let changes = h.run(h.api.pack_changes(None, 12, 50));
    let changed = changes
        .value()
        .and_then(|value| value.items.clone())
        .unwrap_or_default();
    assert_eq!(changed.len(), 1);
    assert!(changed[0].deleted);

    let sent = h.sent();
    assert_eq!(
        sent[0].url,
        "https://chat.example.com/api/v1/families/mine/pack"
    );
    assert_eq!(
        sent[1].url,
        "https://chat.example.com/api/v1/families/mine/pack/changes?after_seq=12&limit=50"
    );
}

/// A claim: the attachment's id and, only when one was given, a label. Both `201` and `200`
/// answer an item and both are success — a `200` is the pack already holding it.
#[test]
fn adding_to_the_pack_claims_an_upload_and_takes_either_success() {
    let h = Harness::new();
    let item = format!(r#"{{"item": {PACK_ITEM}}}"#);
    h.http
        .respond(Method::Post, "/api/v1/families/mine/pack", 201, &item)
        .respond(Method::Post, "/api/v1/families/mine/pack", 200, &item)
        .respond(Method::Post, "/api/v1/families/mine/pack", 200, &item);

    let added = h.run(h.api.add_to_pack(71, Some("  party cat ")));
    let again = h.run(h.api.add_to_pack(99, None));
    let blank = h.run(h.api.add_to_pack(99, Some("   ")));

    assert!(added.ok());
    assert!(again.ok());
    // WHICH success it was is the status, and the only thing that says it: 201 is a new item,
    // 200 is one the pack already held. The cache cannot answer that — this device's own frame
    // may land before the claim does.
    assert_eq!(added.status, 201);
    assert_eq!(again.status, 200);
    // The item that came back carries the id the PACK holds, which is not the one a duplicate
    // named.
    assert_eq!(
        again
            .value()
            .and_then(|value| value.item.attachment.as_ref())
            .map(|a| a.id),
        Some(71)
    );
    let sent = h.sent();
    assert_eq!(sent[0].method, Method::Post);
    assert_eq!(
        sent[0].url,
        "https://chat.example.com/api/v1/families/mine/pack"
    );
    let bodies = h.bodies();
    assert_eq!(
        bodies[0].as_deref(),
        Some(r#"{"attachment_id":71,"label":"party cat"}"#)
    );
    // No label is no key — and an empty one is no label.
    assert_eq!(bodies[1].as_deref(), Some(r#"{"attachment_id":99}"#));
    assert_eq!(bodies[2].as_deref(), Some(r#"{"attachment_id":99}"#));
    assert!(blank.ok());
}

#[test]
fn the_packs_refusals_are_read_as_the_protocol_writes_them() {
    let h = Harness::new();
    h.http
        .respond(
            Method::Post,
            "/api/v1/families/mine/pack",
            413,
            r#"{"error": {"code": "pack_item_too_large", "message": "…"}}"#,
        )
        .respond(
            Method::Post,
            "/api/v1/families/mine/pack",
            409,
            r#"{"error": {"code": "pack_full", "message": "…"}}"#,
        )
        .respond(
            Method::Delete,
            "/api/v1/families/mine/pack/5",
            403,
            r#"{"error": {"code": "not_pack_item_author", "message": "…"}}"#,
        )
        .respond(
            Method::Delete,
            "/api/v1/families/mine/pack/5",
            404,
            r#"{"error": {"code": "pack_item_not_found", "message": "…"}}"#,
        )
        .respond(Method::Delete, "/api/v1/families/mine/pack/5", 204, "");

    let code = |answer: Option<&fc_core::protocol::ApiError>| {
        answer.map(|error| error.code.clone()).unwrap_or_default()
    };
    assert_eq!(
        code(h.run(h.api.add_to_pack(71, None)).error()),
        error_codes::PACK_ITEM_TOO_LARGE
    );
    let full = h.run(h.api.add_to_pack(71, None));
    assert_eq!(code(full.error()), error_codes::PACK_FULL);
    // Refusals, every one: the ceiling is the family's, and trying again would refuse again.
    assert!(!full.error().expect("a failure").transient());
    assert_eq!(
        code(h.run(h.api.remove_from_pack(5)).error()),
        error_codes::NOT_PACK_ITEM_AUTHOR
    );
    assert_eq!(
        code(h.run(h.api.remove_from_pack(5)).error()),
        error_codes::PACK_ITEM_NOT_FOUND
    );
    // Removing is idempotent, and a 204 is a success with nothing in it.
    assert!(h.run(h.api.remove_from_pack(5)).ok());
    let sent = h.sent();
    assert_eq!(sent[4].method, Method::Delete);
    assert_eq!(
        sent[4].url,
        "https://chat.example.com/api/v1/families/mine/pack/5"
    );
    // A write is never retried by the transport.
    assert_eq!(sent.len(), 5);
}

#[test]
fn a_rest_send_says_sticker_only_when_it_is_one() {
    const ANSWER: &str = r#"
        {"message": {"id": 1340, "chat_id": 42, "sender_id": 7, "client_msg_id": "k", "body": "",
         "created_at": "2026-09-13T10:00:00Z",
         "attachments": [{"id": 90, "kind": "photo", "mime": "image/webp", "sticker": true}]}}
        "#;
    let h = Harness::new();
    h.http
        .respond(Method::Post, "/api/v1/chats/42/messages", 201, ANSWER)
        .respond(Method::Post, "/api/v1/chats/42/messages", 201, ANSWER);

    let sticker = SendOptions {
        attachment_ids: &[90],
        sticker: true,
        ..SendOptions::default()
    };
    let sent = h.run(h.api.send_message(42, "k", "", &sticker));
    let plain = SendOptions {
        attachment_ids: &[90],
        ..SendOptions::default()
    };
    h.run(h.api.send_message(42, "k2", "Dinner at 7?", &plain));

    assert!(sent.value().expect("a message").message.media()[0].sticker);
    let bodies = h.bodies();
    assert_eq!(
        bodies[0].as_deref(),
        Some(r#"{"client_msg_id":"k","body":"","attachment_ids":[90],"sticker":true}"#)
    );
    // Absent otherwise — never false.
    assert!(!bodies[1].as_deref().unwrap_or_default().contains("sticker"));
}

#[test]
fn a_rest_send_says_round_only_when_it_is_a_video_message() {
    const ANSWER: &str = r#"
        {"message": {"id": 1341, "chat_id": 42, "sender_id": 7, "client_msg_id": "k", "body": "",
         "created_at": "2026-10-05T10:00:00Z",
         "attachments": [{"id": 91, "kind": "video", "mime": "video/mp4", "size": 1649700, "width": 480,
                          "height": 480, "duration_ms": 23400, "has_preview": true, "round": true}]}}
        "#;
    let h = Harness::new();
    h.http
        .respond(Method::Post, "/api/v1/chats/42/messages", 201, ANSWER)
        .respond(Method::Post, "/api/v1/chats/42/messages", 201, ANSWER);

    let round = SendOptions {
        reply_to_message_id: Some(41),
        attachment_ids: &[91],
        round: true,
        ..SendOptions::default()
    };
    let sent = h.run(h.api.send_message(42, "k", "", &round));
    let plain = SendOptions {
        attachment_ids: &[91],
        ..SendOptions::default()
    };
    h.run(h.api.send_message(42, "k2", "", &plain));

    let message = &sent.value().expect("a message").message;
    assert!(message.media()[0].round);
    assert_eq!(message.round_video().map(|video| video.id), Some(91));
    let bodies = h.bodies();
    assert_eq!(
        bodies[0].as_deref(),
        Some(
            r#"{"client_msg_id":"k","body":"","reply_to_message_id":41,"attachment_ids":[91],"round":true}"#
        )
    );
    // Absent otherwise — never false — so a server that predates video messages reads what it
    // always read.
    assert!(!bodies[1].as_deref().unwrap_or_default().contains("round"));
    assert!(!bodies[0].as_deref().unwrap_or_default().contains("sticker"));
}

#[test]
fn a_rest_send_says_animation_only_when_it_is_one() {
    const ANSWER: &str = r#"
        {"message": {"id": 1342, "chat_id": 42, "sender_id": 7, "client_msg_id": "k", "body": "",
         "created_at": "2026-10-19T10:00:00Z",
         "attachments": [{"id": 93, "kind": "video", "mime": "video/mp4", "size": 182330, "width": 480,
                          "height": 270, "duration_ms": 2100, "has_preview": true, "animation": true}]}}
        "#;
    let h = Harness::new();
    h.http
        .respond(Method::Post, "/api/v1/chats/42/messages", 201, ANSWER)
        .respond(Method::Post, "/api/v1/chats/42/messages", 201, ANSWER);

    let animation = SendOptions {
        attachment_ids: &[93],
        animation: true,
        ..SendOptions::default()
    };
    let sent = h.run(h.api.send_message(42, "k", "", &animation));
    let plain = SendOptions {
        attachment_ids: &[93],
        ..SendOptions::default()
    };
    h.run(h.api.send_message(42, "k2", "", &plain));

    let message = &sent.value().expect("a message").message;
    assert!(message.media()[0].animation);
    assert_eq!(message.animation_video().map(|video| video.id), Some(93));
    let bodies = h.bodies();
    assert_eq!(
        bodies[0].as_deref(),
        Some(r#"{"client_msg_id":"k","body":"","attachment_ids":[93],"animation":true}"#)
    );
    // Absent otherwise — never false — so a server that predates animations reads what it always
    // read.
    assert!(
        !bodies[1]
            .as_deref()
            .unwrap_or_default()
            .contains("animation")
    );
}

/// The family's own document: the pack's mark, omitted while the pack is untouched, and its two
/// ceilings — whose ABSENCE is how a client knows the server predates packs.
#[test]
fn the_familys_document_carries_the_packs_mark_and_ceilings_or_neither() {
    let h = Harness::new();
    h.http
        .respond(
            Method::Get,
            "/api/v1/families/mine",
            200,
            r#"
            {"family": {"id": 3, "name": "The Smiths"}, "members": [],
             "max_pack_seq": 14, "max_pack_items": 200, "max_pack_item_bytes": 524288}
            "#,
        )
        .respond(
            Method::Get,
            "/api/v1/families/mine",
            200,
            r#"{"family": {"id": 3, "name": "The Smiths"}, "members": [], "max_pack_items": 200, "max_pack_item_bytes": 524288}"#,
        )
        .respond(
            Method::Get,
            "/api/v1/families/mine",
            200,
            r#"{"family": {"id": 3, "name": "The Smiths"}, "members": []}"#,
        );

    let touched = h.run(h.api.family(None)).into_result().expect("a family");
    assert_eq!(touched.max_pack_seq, Some(14));
    assert_eq!(touched.max_pack_items, Some(200));
    assert_eq!(touched.max_pack_item_bytes, Some(524_288));

    let untouched = h.run(h.api.family(None)).into_result().expect("a family");
    assert_eq!(untouched.max_pack_seq, None);
    assert_eq!(untouched.max_pack_items, Some(200));

    let older = h.run(h.api.family(None)).into_result().expect("a family");
    assert_eq!(older.max_pack_items, None);
    assert_eq!(older.max_pack_item_bytes, None);
}

/// The video message's two limits on the family's document (docs/protocol.md, "Video messages"):
/// read as numbers, and their ABSENCE — an older server — read as nothing to record with.
#[test]
fn the_familys_document_carries_the_video_messages_limits_or_neither() {
    let h = Harness::new();
    h.http
        .respond(
            Method::Get,
            "/api/v1/families/mine",
            200,
            r#"
            {"family": {"id": 3, "name": "The Smiths"}, "members": [],
             "max_round_video_ms": 60000, "max_round_video_bytes": 4194304}
            "#,
        )
        .respond(
            Method::Get,
            "/api/v1/families/mine",
            200,
            r#"{"family": {"id": 3, "name": "The Smiths"}, "members": []}"#,
        );

    let current = h.run(h.api.family(None)).into_result().expect("a family");
    assert_eq!(current.max_round_video_ms, Some(60_000));
    assert_eq!(current.max_round_video_bytes, Some(4_194_304));
    assert_eq!(
        RoundVideoLimits::of(&current),
        Some(RoundVideoLimits {
            max_ms: 60_000,
            max_bytes: 4_194_304
        })
    );

    let older = h.run(h.api.family(None)).into_result().expect("a family");
    assert_eq!(older.max_round_video_ms, None);
    assert_eq!(older.max_round_video_bytes, None);
    assert_eq!(RoundVideoLimits::of(&older), None);
}

// ---- transcripts on request (#62) ---------------------------------------------------------------

const TRANSCRIPT: &str = "/api/v1/chats/42/messages/1338/attachments/34/transcript";

/// The STORED-BYTES form (docs/protocol.md, "Transcripts on request"): a POST to the attachment's
/// own path with NO BODY — the form whose answer the server keeps — and the text read back with
/// the language the provider named.
#[test]
fn a_transcript_is_asked_for_with_no_body_at_the_attachments_own_path() {
    let h = Harness::new();
    h.http.respond(
        Method::Post,
        TRANSCRIPT,
        200,
        r#"{"transcript": {"text": "Мы будем в шесть", "language": "ru"}}"#,
    );
    let answer = h.run(h.api.transcript(42, 1338, 34));
    assert!(answer.ok());
    let transcript = answer
        .value()
        .and_then(|value| value.transcript.clone())
        .expect("a transcript");
    assert_eq!(transcript.text.as_deref(), Some("Мы будем в шесть"));
    assert_eq!(transcript.language.as_deref(), Some("ru"));
    let request = h.single();
    assert_eq!(request.method, Method::Post);
    assert_eq!(
        request.url,
        "https://chat.example.com/api/v1/chats/42/messages/1338/attachments/34/transcript"
    );
    // No body at all: a multipart one would be the OTHER form, whose answer is never kept.
    assert_eq!(h.bodies(), [None]);
    assert_eq!(request.header_values("Authorization"), ["Bearer t0ken"]);
}

/// SILENCE IS AN ANSWER: the empty text, and no language, read as a success.
#[test]
fn silence_is_a_success_with_empty_text() {
    let h = Harness::new();
    h.http.respond(
        Method::Post,
        TRANSCRIPT,
        200,
        r#"{"transcript": {"text": ""}}"#,
    );
    let answer = h.run(h.api.transcript(42, 1338, 34));
    assert!(answer.ok());
    let transcript = answer
        .value()
        .and_then(|value| value.transcript.clone())
        .expect("a transcript");
    assert_eq!(transcript.text.as_deref(), Some(""));
    assert_eq!(transcript.language, None);
}

/// Every refusal the endpoint answers, read as its code and status — the ones the bubble branches
/// on — and only `internal` transient. A write, so nothing here repeats it: not even the 500.
#[test]
fn a_transcript_refusal_is_read_as_its_code() {
    for (code, status, transient) in [
        ("transcripts_unavailable", 403, false),
        ("assistant_consent_required", 403, false),
        ("transcript_not_allowed", 403, false),
        ("not_transcribable", 400, false),
        ("transcript_refused", 400, false),
        ("message_not_found", 404, false),
        ("internal", 500, true),
    ] {
        let h = Harness::new();
        h.http.respond(
            Method::Post,
            TRANSCRIPT,
            status,
            &format!(r#"{{"error": {{"code": "{code}", "message": "no"}}}}"#),
        );
        let answer = h.run(h.api.transcript(42, 1338, 34));
        assert!(!answer.ok(), "{code}");
        let error = answer.error().expect("a failure");
        assert_eq!(error.code, code);
        assert_eq!(error.status, status);
        assert_eq!(error.transient(), transient, "{code}");
        assert_eq!(h.sent().len(), 1, "{code}");
    }
}

/// SLOW: "a timeout of its OWN, no shorter than 90 s … never its ordinary request timeout" — and
/// longer than the ordinary one, with nothing in the transport to cap it (the request carries its
/// own deadline, where the C# asserted an `HttpClient` with none).
#[test]
fn a_transcript_has_a_deadline_of_its_own() {
    assert!(TRANSCRIPT_TIMEOUT >= Duration::from_secs(90));
    assert!(TRANSCRIPT_TIMEOUT > ORDINARY_TIMEOUT);
    let h = Harness::new();
    assert_eq!(h.api.transcript_deadline(), TRANSCRIPT_TIMEOUT);
    h.http.respond(
        Method::Post,
        TRANSCRIPT,
        200,
        r#"{"transcript": {"text": ""}}"#,
    );
    h.run(h.api.transcript(42, 1338, 34));
    assert_eq!(h.single().timeout, Some(TRANSCRIPT_TIMEOUT));
}

/// The deadline covers the whole request, the upload included: a supplied sound of up to 25 MiB
/// goes up BEFORE the provider's own wait (the server's 180 s) begins, and the reference proxy
/// waits 300 s on this route. A client that gives up first never gets an answer for supplied
/// sound — the server drops that call with the connection and keeps nothing — so "try again" would
/// fail the same way every time. Past the proxy's wait, as iOS (310 s), Android (310 s) and the
/// web (300 s) wait: the server, or the proxy answering for it, ends the wait.
#[test]
fn a_transcript_waits_past_the_proxy() {
    assert!(TRANSCRIPT_TIMEOUT > Duration::from_secs(300));
    assert!(TRANSCRIPT_TIMEOUT > Duration::from_secs(180) + Duration::from_secs(60));
}

/// An M4A's first bytes: a size, then `ftyp`, then its brand — what the server checks the part
/// for.
const M4A: [u8; 18] = [
    0, 0, 0, 0x18, b'f', b't', b'y', b'p', b'M', b'4', b'A', b' ', 0, 0, 2, 0, 0xFF, 0x00,
];

/// One part of a `multipart/form-data` body: name, file name, type and bytes — what the C# fake
/// read off each part while the request was still open.
#[derive(Debug)]
struct Part {
    name: Option<String>,
    file_name: Option<String>,
    content_type: Option<String>,
    bytes: Vec<u8>,
}

fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    haystack
        .get(from..)?
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|at| at + from)
}

/// A disposition's parameter, its quotes trimmed as the C# test trimmed them.
fn parameter(disposition: &str, key: &str) -> Option<String> {
    disposition.split(';').find_map(|piece| {
        let (name, value) = piece.trim().split_once('=')?;
        (name == key).then(|| value.trim_matches('"').to_string())
    })
}

/// The parts of a `multipart/form-data` body, read by its boundary (RFC 7578).
fn parts(content_type: &str, body: &[u8]) -> Vec<Part> {
    let boundary = parameter(content_type, "boundary").expect("a boundary");
    let delimiter = format!("--{boundary}");
    let mut parts = Vec::new();
    let mut at = find(body, delimiter.as_bytes(), 0).expect("a first delimiter");
    loop {
        at += delimiter.len();
        if body[at..].starts_with(b"--") {
            break;
        }
        assert!(
            body[at..].starts_with(b"\r\n"),
            "a line after the delimiter"
        );
        let headers_end = find(body, b"\r\n\r\n", at).expect("the end of the part's headers");
        let headers = String::from_utf8_lossy(&body[at + 2..headers_end]).into_owned();
        let content_start = headers_end + 4;
        let next = find(body, format!("\r\n{delimiter}").as_bytes(), content_start)
            .expect("the next delimiter");
        let mut part = Part {
            name: None,
            file_name: None,
            content_type: None,
            bytes: body[content_start..next].to_vec(),
        };
        for line in headers.split("\r\n") {
            let (name, value) = line.split_once(':').expect("a header");
            if name.eq_ignore_ascii_case("Content-Disposition") {
                part.name = parameter(value, "name");
                part.file_name = parameter(value, "filename");
            } else if name.eq_ignore_ascii_case("Content-Type") {
                part.content_type = Some(value.trim().to_string());
            }
        }
        parts.push(part);
        at = next + 2;
    }
    parts
}

/// THE SUPPLIED-SOUND FORM (docs/protocol.md, "Transcripts on request"): the same path, as
/// `multipart/form-data` with ONE part named `audio` — the device's M4A, typed `audio/mp4`, its
/// bytes exactly as made — and the answer read as the stored form's is.
#[test]
fn supplied_sound_goes_as_one_multipart_audio_part() {
    let h = Harness::new();
    h.http.respond(
        Method::Post,
        TRANSCRIPT,
        200,
        r#"{"transcript": {"text": "Back at six", "language": "en"}}"#,
    );
    let answer = h.run(h.api.transcript_of_sound(42, 1338, 34, M4A.to_vec()));
    assert!(answer.ok());
    assert_eq!(
        answer
            .value()
            .and_then(|value| value.transcript.as_ref())
            .and_then(|transcript| transcript.text.as_deref()),
        Some("Back at six")
    );
    let request = h.single();
    assert_eq!(request.method, Method::Post);
    assert_eq!(
        request.url,
        "https://chat.example.com/api/v1/chats/42/messages/1338/attachments/34/transcript"
    );
    assert_eq!(request.header_values("Authorization"), ["Bearer t0ken"]);
    let content_type = request.header_values("Content-Type")[0].to_string();
    assert_eq!(media_type(&request).as_deref(), Some("multipart/form-data"));
    assert!(
        parameter(&content_type, "boundary").is_some_and(|boundary| !boundary.is_empty()),
        "a boundary"
    );
    let parts = parts(&content_type, request.body.as_deref().expect("a body"));
    assert_eq!(parts.len(), 1);
    let part = &parts[0];
    assert_eq!(part.name.as_deref(), Some("audio"));
    assert_eq!(part.file_name.as_deref(), Some("audio.m4a"));
    assert_eq!(part.content_type.as_deref(), Some("audio/mp4"));
    assert_eq!(part.bytes, M4A);
}

/// A write like the stored form: a 500 is said once, never repeated here — and so is a lost
/// connection.
#[test]
fn supplied_sound_is_never_repeated_here() {
    let h = Harness::new();
    h.http.respond(
        Method::Post,
        TRANSCRIPT,
        500,
        r#"{"error": {"code": "internal", "message": "provider"}}"#,
    );
    let answer = h.run(h.api.transcript_of_sound(42, 1338, 34, M4A.to_vec()));
    assert!(!answer.ok());
    assert!(answer.error().expect("a failure").transient());
    assert_eq!(h.sent().len(), 1);

    let unreached = Harness::new();
    unreached
        .http
        .answer(Method::Post, TRANSCRIPT, unreachable());
    let gone = unreached.run(
        unreached
            .api
            .transcript_of_sound(42, 1338, 34, M4A.to_vec()),
    );
    assert_eq!(
        gone.error().expect("a failure").code,
        error_codes::TRANSPORT
    );
    assert_eq!(unreached.sent().len(), 1);
}

/// The server's verdict on supplied sound is read as its code: not MPEG-4, too big or missing —
/// terminal.
#[test]
fn supplied_sound_the_server_refuses_is_read_as_its_code() {
    let h = Harness::new();
    h.http.respond(
        Method::Post,
        TRANSCRIPT,
        400,
        r#"{"error": {"code": "not_transcribable", "message": "not mpeg-4"}}"#,
    );
    let answer = h.run(h.api.transcript_of_sound(42, 1338, 34, M4A.to_vec()));
    let error = answer.error().expect("a failure");
    assert_eq!(error.code, error_codes::NOT_TRANSCRIBABLE);
    assert!(!error.transient());
}

/// It runs under the transcript's deadline too, never the ordinary one.
#[test]
fn supplied_sound_outlasts_the_ordinary_deadline() {
    let h = Harness::configured(Some("t0ken"), |api| {
        api.with_request_deadline(Duration::from_millis(50))
            .with_transcript_deadline(Duration::from_secs(30))
    });
    h.http
        .respond(
            Method::Post,
            TRANSCRIPT,
            200,
            r#"{"transcript": {"text": ""}}"#,
        )
        .delay(&h.clock, Duration::from_millis(300));
    let answer = h.run(h.api.transcript_of_sound(42, 1338, 34, M4A.to_vec()));
    assert!(answer.ok());
    assert_eq!(
        answer
            .value()
            .and_then(|value| value.transcript.as_ref())
            .and_then(|transcript| transcript.text.as_deref()),
        Some("")
    );
}

/// The rule, run: a server slower than the ordinary deadline still answers a transcript.
#[test]
fn a_transcript_outlasts_the_ordinary_deadline() {
    let h = Harness::configured(Some("t0ken"), |api| {
        api.with_request_deadline(Duration::from_millis(50))
            .with_transcript_deadline(Duration::from_secs(30))
    });
    h.http
        .respond(
            Method::Post,
            TRANSCRIPT,
            200,
            r#"{"transcript": {"text": "Back at six"}}"#,
        )
        .delay(&h.clock, Duration::from_millis(300));
    let answer = h.run(h.api.transcript(42, 1338, 34));
    assert!(answer.ok());
    assert_eq!(
        answer
            .value()
            .and_then(|value| value.transcript.as_ref())
            .and_then(|transcript| transcript.text.as_deref()),
        Some("Back at six")
    );
}

/// And its own deadline does end it: a transport failure, transient, which the member may retry.
#[test]
fn a_transcript_past_its_own_deadline_is_a_transport_failure() {
    let h = Harness::configured(Some("t0ken"), |api| {
        api.with_transcript_deadline(Duration::from_millis(50))
    });
    h.http
        .respond(
            Method::Post,
            TRANSCRIPT,
            200,
            r#"{"transcript": {"text": "too late"}}"#,
        )
        .delay(&h.clock, Duration::from_secs(5));
    let answer = h.run(h.api.transcript(42, 1338, 34));
    assert!(!answer.ok());
    let error = answer.error().expect("a failure");
    assert_eq!(error.code, error_codes::TRANSPORT);
    assert!(error.transient());
    // Never repeated here: a write.
    assert_eq!(h.sent().len(), 1);
}

/// The server's capability and the owner's switch, as `GET /families/mine` carries them — and an
/// older server, which carries neither, reads as "no".
#[test]
fn the_transcript_capability_and_the_owners_switch_arrive_with_the_family() {
    let h = Harness::new();
    h.http
        .respond(
            Method::Get,
            "/api/v1/families/mine",
            200,
            r#"
            {"family": {"id": 3, "name": "The Smiths", "ai_transcripts": true},
             "assistant": {"user_id": 1, "display_name": "Assistant", "mention": "@ai",
                           "processor": "Azure OpenAI", "transcribe": true, "transcribe_max_bytes": 26214400}}
            "#,
        )
        .respond(
            Method::Get,
            "/api/v1/families/mine",
            200,
            r#"
            {"family": {"id": 3, "name": "The Smiths"},
             "assistant": {"user_id": 1, "display_name": "Assistant", "mention": "@ai"}}
            "#,
        );
    let newer = h.run(h.api.family(None)).into_result().expect("a family");
    assert!(newer.family.ai_transcripts);
    let assistant = newer.assistant.expect("an assistant");
    assert!(assistant.transcribe);
    assert_eq!(assistant.transcribe_max_bytes, Some(26_214_400));
    let older = h.run(h.api.family(None)).into_result().expect("a family");
    assert!(!older.family.ai_transcripts);
    let assistant = older.assistant.expect("an assistant");
    assert!(!assistant.transcribe);
    assert_eq!(assistant.transcribe_max_bytes, None);
}

/// The owner's switch goes out as its own key, and only when it is in the patch.
#[test]
fn the_transcript_switch_is_patched_as_its_own_key() {
    let h = Harness::new();
    h.http
        .respond(
            Method::Patch,
            "/api/v1/families/mine",
            200,
            r#"{"family": {"id": 3, "name": "The Smiths", "ai_transcripts": true}}"#,
        )
        .respond(
            Method::Patch,
            "/api/v1/families/mine",
            200,
            r#"{"family": {"id": 3, "name": "The Smiths", "ai_vision": true}}"#,
        );
    let on = h.run(h.api.patch_family(&FamilyPatch {
        ai_transcripts: Some(true),
        ..FamilyPatch::default()
    }));
    assert!(on.value().expect("a family").family.ai_transcripts);
    assert_eq!(h.bodies()[0].as_deref(), Some(r#"{"ai_transcripts":true}"#));
    h.run(h.api.patch_family(&FamilyPatch {
        ai_vision: Some(true),
        ..FamilyPatch::default()
    }));
    assert!(
        !h.bodies()[1]
            .as_deref()
            .unwrap_or_default()
            .contains("ai_transcripts")
    );
}

/// The statistics' two new numbers, read where they are and zero where an older server sends
/// neither.
#[test]
fn the_recordings_as_text_are_in_the_statistics() {
    let h = Harness::new();
    h.http.respond(
        Method::Get,
        "/api/v1/families/mine/stats",
        200,
        r#"
        {"generated_at": "2026-10-02T10:00:00Z",
         "totals": {"members": 2, "messages": 10, "board_notes": 0,
                    "ai": {"questions": 3, "transcripts": 2, "transcript_duration_ms": 95000}},
         "members": [{"user_id": 7, "display_name": "Anna", "messages": 6, "ai": {"questions": 1}}]}
        "#,
    );
    let stats = h.run(h.api.stats()).into_result().expect("statistics");
    let totals = stats.totals.ai.expect("the assistant's totals");
    assert_eq!(totals.transcripts, 2);
    assert_eq!(totals.transcript_duration_ms, 95_000);
    let members = stats.members.unwrap_or_default();
    assert_eq!(members[0].ai.as_ref().map(|ai| ai.transcripts), Some(0));
}

/// Looking things up, as `GET /families/mine` and `GET /me` carry it (docs/protocol.md, "Looking
/// things up"): the providers by name in the server's order, the owner's switch, and the member's
/// stamp — and an older server, which carries none of them, reads as "no lookups here".
#[test]
fn the_lookup_providers_switch_and_consent_arrive_tolerantly() {
    let h = Harness::new();
    h.http
        .respond(
            Method::Get,
            "/api/v1/families/mine",
            200,
            r#"
            {"family": {"id": 3, "name": "The Smiths", "ai_lookups": true},
             "assistant": {"user_id": 1, "display_name": "Assistant", "mention": "@ai", "processor": "Azure OpenAI",
                           "lookups": ["Brave Search", "Open-Meteo", "Wikipedia"]}}
            "#,
        )
        .respond(
            Method::Get,
            "/api/v1/families/mine",
            200,
            r#"
            {"family": {"id": 3, "name": "The Smiths"},
             "assistant": {"user_id": 1, "display_name": "Assistant", "mention": "@ai"}}
            "#,
        )
        .respond(
            Method::Get,
            "/api/v1/me",
            200,
            r#"
            {"user": {"id": 7, "username": "anna", "display_name": "Anna"},
             "assistant_consent_at": "2026-09-19T08:12:04Z", "assistant_lookup_consent_at": "2026-10-03T09:30:00Z"}
            "#,
        )
        .respond(
            Method::Get,
            "/api/v1/me",
            200,
            r#"
            {"user": {"id": 7, "username": "anna", "display_name": "Anna"}, "assistant_consent_at": null}
            "#,
        );
    let newer = h.run(h.api.family(None)).into_result().expect("a family");
    assert!(newer.family.ai_lookups);
    assert_eq!(
        newer.assistant.expect("an assistant").lookups,
        Some(vec![
            "Brave Search".to_string(),
            "Open-Meteo".to_string(),
            "Wikipedia".to_string()
        ])
    );
    let older = h.run(h.api.family(None)).into_result().expect("a family");
    assert!(!older.family.ai_lookups);
    assert_eq!(older.assistant.expect("an assistant").lookups, None);
    assert_eq!(
        h.run(h.api.me())
            .into_result()
            .expect("me")
            .assistant_lookup_consent_at
            .as_deref(),
        Some("2026-10-03T09:30:00Z")
    );
    assert_eq!(
        h.run(h.api.me())
            .into_result()
            .expect("me")
            .assistant_lookup_consent_at,
        None
    );
}

/// The owner's switch goes out as its own key, and only when it is in the patch.
#[test]
fn the_lookup_switch_is_patched_as_its_own_key() {
    let h = Harness::new();
    h.http
        .respond(
            Method::Patch,
            "/api/v1/families/mine",
            200,
            r#"{"family": {"id": 3, "name": "The Smiths", "ai_lookups": false}}"#,
        )
        .respond(
            Method::Patch,
            "/api/v1/families/mine",
            200,
            r#"{"family": {"id": 3, "name": "The Smiths", "ai_history": true}}"#,
        );
    let off = h.run(h.api.patch_family(&FamilyPatch {
        ai_lookups: Some(false),
        ..FamilyPatch::default()
    }));
    assert!(!off.value().expect("a family").family.ai_lookups);
    assert_eq!(h.bodies()[0].as_deref(), Some(r#"{"ai_lookups":false}"#));
    assert_eq!(h.sent()[0].method, Method::Patch);
    h.run(h.api.patch_family(&FamilyPatch {
        ai_history: Some(true),
        ..FamilyPatch::default()
    }));
    assert!(
        !h.bodies()[1]
            .as_deref()
            .unwrap_or_default()
            .contains("ai_lookups")
    );
}

/// The greeting's weather, as `GET /families/mine` carries it (docs/protocol.md, "Today's
/// weather, for places the owner chose"): the owner's places on the family, the server's ability
/// on the assistant — and an older server, which carries neither, reads as no places and no
/// greeting weather.
#[test]
fn the_greeting_places_and_weather_arrive_tolerantly() {
    let h = Harness::new();
    h.http
        .respond(
            Method::Get,
            "/api/v1/families/mine",
            200,
            r#"
            {"family": {"id": 3, "name": "The Smiths", "ai_greeting": true, "greeting_places": ["Moscow", "Белград"]},
             "assistant": {"user_id": 1, "display_name": "Assistant", "mention": "@ai", "greeting_weather": true}}
            "#,
        )
        .respond(
            Method::Get,
            "/api/v1/families/mine",
            200,
            r#"
            {"family": {"id": 3, "name": "The Smiths", "greeting_places": []},
             "assistant": {"user_id": 1, "display_name": "Assistant", "mention": "@ai", "greeting_weather": false}}
            "#,
        )
        .respond(
            Method::Get,
            "/api/v1/families/mine",
            200,
            r#"
            {"family": {"id": 3, "name": "The Smiths"},
             "assistant": {"user_id": 1, "display_name": "Assistant", "mention": "@ai"}}
            "#,
        );
    let newer = h.run(h.api.family(None)).into_result().expect("a family");
    assert_eq!(
        newer.family.greeting_places,
        Some(vec!["Moscow".to_string(), "Белград".to_string()])
    );
    assert!(newer.assistant.expect("an assistant").greeting_weather);
    let off = h.run(h.api.family(None)).into_result().expect("a family");
    assert_eq!(off.family.greeting_places, Some(Vec::new()));
    assert!(!off.assistant.expect("an assistant").greeting_weather);
    let older = h.run(h.api.family(None)).into_result().expect("a family");
    assert_eq!(older.family.greeting_places, None);
    assert!(!older.assistant.expect("an assistant").greeting_weather);
}

/// The places go out as one list that REPLACES the stored one; `[]` clears it, and a patch
/// without them never carries the key — never a `null`, which the server refuses. The answer is
/// the list as the server KEPT it.
#[test]
fn the_greeting_places_are_patched_as_one_list_and_never_as_null() {
    let h = Harness::new();
    for json in [
        r#"{"family": {"id": 3, "name": "The Smiths", "greeting_places": ["Moscow"]}}"#,
        r#"{"family": {"id": 3, "name": "The Smiths", "greeting_places": []}}"#,
        r#"{"family": {"id": 3, "name": "The Smiths", "greeting_places": []}}"#,
        r#"{"family": {"id": 3, "name": "The Smiths", "greeting_places": ["Белград"]}}"#,
    ] {
        h.http
            .respond(Method::Patch, "/api/v1/families/mine", 200, json);
    }

    let kept = h.run(h.api.patch_family(&FamilyPatch {
        greeting_places: Some(vec!["Moscow".into(), "moscow".into()]),
        ..FamilyPatch::default()
    }));
    let sent = h.sent();
    assert_eq!(sent[0].method, Method::Patch);
    assert!(
        sent[0]
            .url
            .split('?')
            .next()
            .unwrap_or_default()
            .ends_with("/families/mine")
    );
    assert_eq!(
        h.bodies()[0].as_deref(),
        Some(r#"{"greeting_places":["Moscow","moscow"]}"#)
    );
    // What the server kept, not what was sent.
    assert_eq!(
        kept.value().expect("a family").family.greeting_places,
        Some(vec!["Moscow".to_string()])
    );

    h.run(h.api.patch_family(&FamilyPatch {
        greeting_places: Some(Vec::new()),
        ..FamilyPatch::default()
    }));
    assert_eq!(h.bodies()[1].as_deref(), Some(r#"{"greeting_places":[]}"#));

    h.run(h.api.patch_family(&FamilyPatch {
        ai_greeting: Some(true),
        ..FamilyPatch::default()
    }));
    assert!(
        !h.bodies()[2]
            .as_deref()
            .unwrap_or_default()
            .contains("greeting_places")
    );

    // Any script: the body is JSON whatever the escaping, and it reads back as what was typed.
    h.run(h.api.patch_family(&FamilyPatch {
        greeting_places: Some(vec!["Белград".into()]),
        ..FamilyPatch::default()
    }));
    let sent = json(h.bodies()[3].as_deref().expect("a body"));
    assert_eq!(sent["greeting_places"][0], "Белград");
    assert_eq!(sent.as_object().map(|object| object.len()), Some(1));
}

/// The member's own lookup consent: its own endpoint, the same shape as the assistant's, the stamp
/// read back — and the refusal for granting it without the first one read as its code, asked
/// once.
#[test]
fn the_lookup_consent_is_its_own_endpoint() {
    let h = Harness::new();
    let path = "/api/v1/me/assistant-lookup-consent";
    h.http
        .respond(
            Method::Post,
            path,
            200,
            r#"{"assistant_lookup_consent_at": "2026-10-03T09:30:00Z"}"#,
        )
        .respond(
            Method::Post,
            path,
            200,
            r#"{"assistant_lookup_consent_at": null}"#,
        )
        .respond(
            Method::Post,
            path,
            403,
            r#"{"error": {"code": "assistant_consent_required", "message": "agree first"}}"#,
        );
    let granted = h.run(h.api.set_assistant_lookup_consent(true));
    assert_eq!(
        granted
            .value()
            .expect("a stamp")
            .assistant_lookup_consent_at
            .as_deref(),
        Some("2026-10-03T09:30:00Z")
    );
    let sent = h.sent();
    assert_eq!(sent[0].method, Method::Post);
    assert_eq!(
        sent[0].url,
        "https://chat.example.com/api/v1/me/assistant-lookup-consent"
    );
    assert_eq!(h.bodies()[0].as_deref(), Some(r#"{"granted":true}"#));

    let withdrawn = h.run(h.api.set_assistant_lookup_consent(false));
    assert!(withdrawn.ok());
    assert_eq!(
        withdrawn
            .value()
            .expect("a stamp")
            .assistant_lookup_consent_at,
        None
    );
    assert_eq!(h.bodies()[1].as_deref(), Some(r#"{"granted":false}"#));

    let refused = h.run(h.api.set_assistant_lookup_consent(true));
    assert_eq!(
        refused.error().expect("a failure").code,
        error_codes::ASSISTANT_CONSENT_REQUIRED
    );
    assert_eq!(h.sent().len(), 3);
}

/// The paid web searches, read per member and in the totals — and zero where an older server
/// sends none.
#[test]
fn the_web_searches_are_in_the_statistics() {
    let h = Harness::new();
    h.http.respond(
        Method::Get,
        "/api/v1/families/mine/stats",
        200,
        r#"
        {"generated_at": "2026-10-03T10:00:00Z",
         "totals": {"members": 2, "messages": 10, "board_notes": 0, "ai": {"questions": 3, "searches": 9}},
         "members": [{"user_id": 7, "display_name": "Anna", "messages": 6, "ai": {"questions": 2, "searches": 4}},
                     {"user_id": 11, "display_name": "Bob", "messages": 4, "ai": {"questions": 1}}]}
        "#,
    );
    let stats = h.run(h.api.stats()).into_result().expect("statistics");
    assert_eq!(stats.totals.ai.as_ref().map(|ai| ai.searches), Some(9));
    let members = stats.members.unwrap_or_default();
    assert_eq!(members[0].ai.as_ref().map(|ai| ai.searches), Some(4));
    assert_eq!(members[1].ai.as_ref().map(|ai| ai.searches), Some(0));
}

/// The five group endpoints (docs/protocol.md, "Groups"), each at its path with its body — and
/// LEAVING, a `204` with nothing in it, read as "left" rather than as an answer that could not be
/// read.
#[test]
fn the_group_endpoints_are_asked_as_the_protocol_writes_them() {
    const GROUP: &str = r#"{"chat": {"id": 77, "kind": "group", "title": "Cousins", "peer_user_id": null, "member_ids": [7, 9, 11], "admin_user_id": 7}}"#;
    let h = Harness::new();
    h.http
        .respond(Method::Post, "/api/v1/chats/groups", 201, GROUP)
        .respond(Method::Patch, "/api/v1/chats/77", 200, GROUP)
        .respond(Method::Post, "/api/v1/chats/77/members", 200, GROUP)
        .respond(Method::Delete, "/api/v1/chats/77/members/11", 200, GROUP)
        .respond(Method::Delete, "/api/v1/chats/77/members/7", 204, "")
        .respond(Method::Delete, "/api/v1/chats/77", 204, "");

    let created = h.run(h.api.create_group("Cousins", &[9, 11]));
    assert!(created.ok());
    assert_eq!(created.status, 201);
    let chat = &created.value().expect("a group").chat;
    assert_eq!(chat.member_ids, Some(vec![7, 9, 11]));
    assert_eq!(chat.admin_user_id, Some(7));
    assert_eq!(chat.peer_user_id, None);
    assert!(h.run(h.api.rename_group(77, "The cousins")).ok());
    assert!(h.run(h.api.add_group_members(77, &[12])).ok());
    let removed = h.run(h.api.remove_group_member(77, 11));
    assert!(removed.ok());
    let removed = removed.value().expect("an answer");
    assert!(!removed.left());
    assert_eq!(removed.chat.as_ref().map(|chat| chat.id), Some(77));
    let left = h.run(h.api.remove_group_member(77, 7));
    assert!(left.ok());
    assert!(left.value().expect("an answer").left());
    assert!(h.run(h.api.delete_group(77)).ok());

    let at = |path: &str| format!("https://chat.example.com/api/v1{path}");
    assert_eq!(
        h.asked(),
        [
            (Method::Post, at("/chats/groups")),
            (Method::Patch, at("/chats/77")),
            (Method::Post, at("/chats/77/members")),
            (Method::Delete, at("/chats/77/members/11")),
            (Method::Delete, at("/chats/77/members/7")),
            (Method::Delete, at("/chats/77")),
        ]
    );
    assert_eq!(
        h.bodies(),
        [
            Some(r#"{"title":"Cousins","member_ids":[9,11]}"#.to_string()),
            Some(r#"{"title":"The cousins"}"#.to_string()),
            Some(r#"{"user_ids":[12]}"#.to_string()),
            None,
            None,
            None
        ]
    );
}

/// The two codes groups added arrive as the refusals they are, never as something worth trying
/// again.
#[test]
fn a_group_refusal_carries_its_code() {
    let h = Harness::new();
    h.http
        .respond(
            Method::Patch,
            "/api/v1/chats/77",
            403,
            r#"{"error": {"code": "not_group_admin", "message": "only the group's admin may do that"}}"#,
        )
        .respond(
            Method::Post,
            "/api/v1/chats/groups",
            409,
            r#"{"error": {"code": "too_many_groups", "message": "the family already has 50 groups"}}"#,
        );

    let renamed = h.run(h.api.rename_group(77, "Cousins"));
    assert!(!renamed.ok());
    let error = renamed.error().expect("a failure");
    assert_eq!(error.code, error_codes::NOT_GROUP_ADMIN);
    assert!(!error.transient());
    let created = h.run(h.api.create_group("Cousins", &[9]));
    let error = created.error().expect("a failure");
    assert_eq!(error.code, error_codes::TOO_MANY_GROUPS);
    assert!(!error.transient());
}

// ---- what .NET did that the client leaned on (measured on .NET 10) ----------------------------

/// `Retry-After` as `RetryConditionHeaderValue` read it: every case here is .NET 10's own answer
/// (`date` cases as the wait from a fixed now). The waits a client honours are delta-seconds — the
/// protocol's form — but a date was read too, and is read the same way here.
#[test]
fn retry_after_is_read_as_net_read_it() {
    let now = datetime!(2015-10-21 07:00:00 UTC);
    let at = |text: &str| retry_after(text, now);
    let seconds = |n: i64| Some(time::Duration::seconds(n));
    for (text, expected) in [
        ("2", seconds(2)),
        (" 2", seconds(2)),
        ("2 ", seconds(2)),
        ("\t5", seconds(5)),
        ("5\t", seconds(5)),
        ("02", seconds(2)),
        ("0", seconds(0)),
        ("2147483647", seconds(2_147_483_647)),
        ("2147483648", None),
        ("99999999999", None),
        ("+2", None),
        ("-1", None),
        ("2.5", None),
        ("", None),
        ("abc", None),
        ("2abc", None),
        ("2 3", None),
        ("2, 3", None),
        ("0x10", None),
        ("5\r\n", None),
        ("\u{a0}5", None),
        // A date that starts with a digit is a malformed delta, never a date.
        ("21 Oct 2015 07:28:00 GMT", None),
        ("2015-10-21T07:28:00Z", None),
    ] {
        assert_eq!(at(text), expected, "[{text}]");
    }
    let half_past = seconds(28 * 60);
    for text in [
        "Wed, 21 Oct 2015 07:28:00 GMT",
        "Wednesday, 21-Oct-15 07:28:00 GMT",
        "Wed Oct 21 07:28:00 2015",
        "Wed, 21 Oct 2015 07:28:00 +0000",
        "Wed, 21 Oct 2015 07:28:00 -00:00",
        "Wed, 21 Oct 2015 07:28:00",
        "Wed, 21 Oct 2015 07:28:00 UTC",
        "Wed, 21 Oct 15 07:28:00 GMT",
        "wed, 21 Oct 2015 07:28:00 GMT",
        "Wed, 21 oct 2015 07:28:00 GMT",
        "WED, 21 OCT 2015 07:28:00 GMT",
        "Wed,21 Oct 2015 07:28:00 GMT",
        "Wed , 21 Oct 2015 07:28:00 GMT",
        "Wed, 21Oct 2015 07:28:00 GMT",
        "Wed,\t21 Oct 2015 07:28:00 GMT",
        "Wed, 21  Oct 2015 07:28:00 GMT",
        "Wed, 21 Oct 2015  07:28:00 GMT",
        "Wed, 21 Oct 2015 07:28:00  GMT",
        "Wed, 21 Oct 2015 07 : 28 : 00 GMT",
        "Wed, 21 Oct 2015 07:28 :00 GMT",
        "Wed, 21 Oct 2015 07:28:00GMT",
        "Wed, 21 Oct 2015 7:28:0 GMT",
        "Wed, 21 Oct 2015 07:28:00 GMT ",
        " Wed, 21 Oct 2015 07:28:00 GMT",
        "Wed, 21 Oct 2015 07:28:00 GMT\t",
        "Wed, 21 Oct 2015 07:28:00 +0000 ",
        "Wednesday, 21-Oct-15 07:28:00 UTC",
        "Wednesday, 21-Oct-15 07:28:00",
        "wednesday, 21-Oct-15 07:28:00 GMT",
        "Wednesday,21-Oct-15 07:28:00 GMT",
        "Wednesday, 21 - Oct - 15 07:28:00 GMT",
        "Wednesday, 21-oct-15 07:28:00 GMT",
        "Wed Oct 21 07:28:00  2015",
        "Wed  Oct 21 07:28:00 2015",
    ] {
        assert_eq!(at(text), half_past, "[{text}]");
    }
    assert_eq!(at("Wed, 21 Oct 2015 7:8:0 GMT"), seconds(8 * 60));
    assert_eq!(at("Wednesday, 21-Oct-15 7:8:0 GMT"), seconds(8 * 60));
    assert_eq!(at("Wed Oct 21 7:28:0 2015"), half_past);
    // An offset is the moment it names: 07:28 at +01:00 is 06:28 UTC, half an hour BEFORE now.
    assert_eq!(at("Wed, 21 Oct 2015 07:28:00 +01:00"), seconds(-32 * 60));
    assert_eq!(at("Wednesday, 21-Oct-15 07:28:00 +0100"), seconds(-32 * 60));
    assert_eq!(
        at("Wednesday, 21-Oct-15 07:28:00 +01:00"),
        seconds(-32 * 60)
    );
    assert_eq!(
        at("Wed, 21 Oct 2015 07:28:00 -0500"),
        seconds(28 * 60 + 5 * 3600)
    );
    assert_eq!(
        at("Wed, 21 Oct 2015 07:28:00 +0530"),
        seconds(28 * 60 - 5 * 3600 - 30 * 60)
    );
    assert_eq!(
        at("Wed, 21 Oct 2015 07:28:00 +5:30"),
        seconds(28 * 60 - 5 * 3600 - 30 * 60)
    );
    assert_eq!(
        at("Wed, 21 Oct 2015 07:28:00 +14:00"),
        seconds(28 * 60 - 14 * 3600)
    );
    assert_eq!(
        at("Wed, 21 Oct 2015 07:28:00 +1400"),
        seconds(28 * 60 - 14 * 3600)
    );
    assert_eq!(
        at("Wed, 21 Oct 2015 07:28:00 -14:00"),
        seconds(28 * 60 + 14 * 3600)
    );
    // A day of one digit, padded or not, in each form; a two-digit year's century.
    let first = datetime!(2015-10-01 07:28:00 UTC) - now;
    for text in [
        "Thu, 1 Oct 2015 07:28:00 GMT",
        "Thu, 01 Oct 2015 07:28:00 GMT",
        "Thu,  1 Oct 2015 07:28:00 GMT",
        "Thursday, 1-Oct-15 07:28:00 GMT",
        "Thu Oct  1 07:28:00 2015",
        "Thu Oct 1 07:28:00 2015",
        "Thu Oct 01 07:28:00 2015",
    ] {
        assert_eq!(at(text), Some(first), "[{text}]");
    }
    for (text, moment) in [
        (
            "Thu, 21 Oct 49 07:28:00 GMT",
            datetime!(2049-10-21 07:28:00 UTC),
        ),
        (
            "Mon, 21 Oct 30 07:28:00 GMT",
            datetime!(2030-10-21 07:28:00 UTC),
        ),
        (
            "Thu, 21 Oct 99 07:28:00 GMT",
            datetime!(1999-10-21 07:28:00 UTC),
        ),
        (
            "Sat, 01 Jan 00 00:00:00 GMT",
            datetime!(2000-01-01 00:00:00 UTC),
        ),
        (
            "Sat, 29 Feb 2020 00:00:00 GMT",
            datetime!(2020-02-29 00:00:00 UTC),
        ),
        (
            "Thu, 01 Jan 1970 00:00:00 GMT",
            datetime!(1970-01-01 00:00:00 UTC),
        ),
        (
            "Mon, 01 Jan 0001 00:00:00 GMT",
            datetime!(0001-01-01 00:00:00 UTC),
        ),
        (
            "Fri, 31 Dec 9999 23:59:59 GMT",
            datetime!(9999-12-31 23:59:59 UTC),
        ),
        (
            "Wed Oct 21 07:28:00 0015",
            datetime!(0015-10-21 07:28:00 UTC),
        ),
        (
            "Sunday, 06-Nov-94 08:49:37 GMT",
            datetime!(1994-11-06 08:49:37 UTC),
        ),
        (
            "Sun Nov  6 08:49:37 1994",
            datetime!(1994-11-06 08:49:37 UTC),
        ),
        (
            "Sun, 06 Nov 1994 08:49:37 GMT",
            datetime!(1994-11-06 08:49:37 UTC),
        ),
    ] {
        assert_eq!(at(text), Some(moment - now), "[{text}]");
    }
    // And what .NET refused.
    for text in [
        "Thu, 21 Oct 2015 07:28:00 GMT",
        "Wed, 1 Oct 2015 07:28:00 GMT",
        "Wed Oct  1 07:28:00 2015",
        "Fri, 21 Oct 50 07:28:00 GMT",
        "Tue, 21 Oct 29 07:28:00 GMT",
        "Wed, 21 Oct 99 07:28:00 GMT",
        "Wed, 21 Oct 0001 00:00:00 GMT",
        "Fri, 29 Feb 2019 00:00:00 GMT",
        "Wed, 31 Feb 2015 07:28:00 GMT",
        "Thursday, 21-Oct-15 07:28:00 GMT",
        "Wed, 21 Oct 2015 07:28:00 gmt",
        "Wed, 21 Oct 2015 07:28:00 utc",
        "Wednesday, 21-Oct-15 07:28:00 utc",
        "Wed, 21 Oct 2015 07:28:00 UT",
        "Wed, 21 Oct 2015 07:28:00 EST",
        "Wed, 21 Oct 2015 07:28:00 Z",
        "Wed, 21 Oct 2015 07:28:00 GMT+1",
        "Wed, 21 Oct 2015 07:28:00 +1",
        "Wed, 21 Oct 2015 07:28:00 +14",
        "Wed, 21 Oct 2015 07:28:00 +15:00",
        "Wed, 21 Oct 2015 07:28:00 +14:01",
        "Wed, 21 Oct 2015 07:28:00 +00:60",
        "Wed, 21 Oct 2015 07:28:00 +05:3",
        "Wed, 21 Oct 2015 07:28:00 + 05:30",
        "Wed, 21 Oct 2015 07:28:00 +05:30:00",
        "Wed, 21 Oct 2015 07:28:00 +01:00:00",
        "Wed, 21 Oct 2015 07:28:00 +0000x",
        "Wed, 21 Oct 2015 07:28:00 +00:00 GMT",
        "Wed, 21 Oct 2015 07:28:00 GMT GMT",
        "Wed, 21 Oct 15 07:28:00 +0100",
        "Wed, 21 Oct 015 07:28:00 GMT",
        "Wed, 21 Oct 12015 07:28:00 GMT",
        "Wed, 021 Oct 2015 07:28:00 GMT",
        "Wed, 21 Oct 2015 007:28:00 GMT",
        "Wed, 21 Oct 2015 24:00:00 GMT",
        "Wed, 21 Oct 2015 07:28:60 GMT",
        "Wed, 21 Oct 2015 07:28:00.5 GMT",
        "Wed, 21 Oct 2015 07:28 GMT",
        "Wed, 21 October 2015 07:28:00 GMT",
        "Wednesday, 21 Oct 2015 07:28:00 GMT",
        "Wednesday, 21-Oct-2015 07:28:00 GMT",
        "Wed, 21-Oct-15 07:28:00 GMT",
        "Wed Oct 21 07:28:00 2015 GMT",
        "Wed Oct 21 07:28:00 2015 +0000",
        "Wed Oct 21 07:28:00 15",
        "Wed,, 21 Oct 2015 07:28:00 GMT",
        "Wed. 21 Oct 2015 07:28:00 GMT",
        "Wed, 21 Oct 2015 07:28:00 GMT, Thu, 22 Oct 2015 07:28:00 GMT",
    ] {
        assert_eq!(at(text), None, "[{text}]");
    }
}

/// A date in the past is a NEGATIVE wait, kept signed on the error and clamped to none by the
/// read, which is tried again at once. A date in the future is honoured, capped at two minutes
/// like any other wait. The date is measured against the client's clock.
#[test]
fn a_retry_after_date_is_a_wait_from_the_clock() {
    let h = Harness::new();
    let mut past = json_response(429, "", &[("Retry-After", "Wed, 28 Oct 2026 11:58:30 GMT")]);
    past.body = b"<html>429</html>".to_vec();
    h.http
        .answer(Method::Post, "/api/v1/chats/42/messages", Ok(past))
        .answer(
            Method::Get,
            BOARD,
            Ok(json_response(
                503,
                "",
                &[("Retry-After", "Wed, 28 Oct 2026 11:58:30 GMT")],
            )),
        )
        .respond(Method::Get, BOARD, 200, r#"{"notes": []}"#)
        .answer(
            Method::Get,
            BOARD,
            Ok(json_response(
                503,
                "",
                &[("Retry-After", "Wed, 28 Oct 2026 12:30:00 GMT")],
            )),
        )
        .respond(Method::Get, BOARD, 200, r#"{"notes": []}"#);

    let refused = h.run(h.api.send_message(42, "k", "hi", &SendOptions::default()));
    assert_eq!(
        refused.error().and_then(|error| error.retry_after),
        Some(time::Duration::seconds(-90))
    );

    let before = h.clock.slept().len();
    assert!(h.run(h.api.board(None)).ok());
    // Two requests, each under its deadline, and no wait between them.
    assert_eq!(
        h.clock.slept()[before..],
        [ORDINARY_TIMEOUT, ORDINARY_TIMEOUT]
    );

    let before = h.clock.slept().len();
    assert!(h.run(h.api.board(None)).ok());
    // Half an hour asked, two minutes waited.
    assert_eq!(
        h.clock.slept()[before..],
        [ORDINARY_TIMEOUT, Duration::from_secs(120), ORDINARY_TIMEOUT]
    );
    assert_eq!(h.http.unused(), 0);
}

/// What .NET's `ReadAsStringAsync` made of a body: a UTF-8 byte-order mark dropped, and a byte
/// that is not UTF-8 read as U+FFFD rather than failing the read (both measured).
#[test]
fn a_body_is_read_as_net_read_it() {
    let h = Harness::new();
    let mut marked = json_response(201, "", &[]);
    marked.body = b"\xEF\xBB\xBF{\"message\": {\"id\": 5}}".to_vec();
    let mut broken = json_response(201, "", &[]);
    broken.body = b"{\"message\": {\"id\": 7, \"body\": \"\xFF\"}}".to_vec();
    let mut refused = json_response(403, "", &[]);
    refused.body =
        b"\xEF\xBB\xBF{\"error\": {\"code\": \"blocked\", \"message\": \"no\"}}".to_vec();
    h.http
        .answer(Method::Post, "/api/v1/chats/1/messages", Ok(marked))
        .answer(Method::Post, "/api/v1/chats/1/messages", Ok(broken))
        .answer(Method::Post, "/api/v1/chats/1/messages", Ok(refused));
    let none = SendOptions::default();
    assert_eq!(
        h.run(h.api.send_message(1, "k", "hi", &none))
            .value()
            .map(|value| value.message.id),
        Some(5)
    );
    let replaced = h.run(h.api.send_message(1, "k", "hi", &none));
    assert_eq!(
        replaced.value().map(|value| value.message.body.as_str()),
        Some("\u{FFFD}")
    );
    let error = h.run(h.api.send_message(1, "k", "hi", &none));
    assert_eq!(
        error.error().map(|error| error.code.as_str()),
        Some("blocked")
    );
}

/// A failure with no envelope keeps .NET's reason phrase as its message, and a JSON body without
/// `"error"` is no envelope — the STATUS classifies it (`win/` threw a NullReferenceException for
/// `{}` and `{"error": null}`, measured). A code of the wrong JSON type is no envelope either.
#[test]
fn a_body_that_is_not_the_error_shape_leaves_the_status_to_classify_it() {
    let h = Harness::new();
    for (status, json) in [
        (400, "{}"),
        (400, r#"{"error": null}"#),
        (400, r#"{"message": "a proxy's own words"}"#),
        (400, r#"{"error": {"code": 7, "message": "x"}}"#),
        (502, "<html>502</html>"),
        (418, ""),
    ] {
        h.http
            .respond(Method::Post, "/api/v1/chats/1/messages", status, json);
    }
    let none = SendOptions::default();
    let mut errors = Vec::new();
    for _ in 0..6 {
        let answer = h.run(h.api.send_message(1, "k", "hi", &none));
        errors.push(answer.error().cloned().expect("a failure"));
    }
    for error in &errors[..4] {
        assert_eq!(error.code, "");
        assert_eq!(error.message, "Bad Request");
        assert_eq!(error.status, 400);
        assert!(!error.transient());
    }
    assert_eq!(errors[4].message, "Bad Gateway");
    assert!(errors[4].transient());
    // A status .NET had no phrase for has an empty message.
    assert_eq!(errors[5].message, "");
    assert_eq!(errors[5].status, 418);
}

/// The family-scoped path forms and the header ride on the bytes reads too, which are asked once.
#[test]
fn the_bytes_reads_are_asked_once_with_the_families_header() {
    let h = Harness::new();
    h.http
        .respond(
            Method::Get,
            "/api/v1/users/9/avatar",
            502,
            "<html>502</html>",
        )
        .answer(
            Method::Get,
            "/api/v1/attachments/34",
            Ok(fc_core::rt::HttpResponse {
                status: 200,
                headers: vec![("Content-Type".into(), "image/jpeg".into())],
                body: vec![0xFF, 0xD8, 0xFF],
            }),
        );
    let avatar = h.run(h.api.avatar(9));
    assert!(avatar.error().expect("a failure").transient());
    let bytes = h.run(h.api.download(34, false));
    assert_eq!(
        bytes.value().map(Vec::as_slice),
        Some(&[0xFF, 0xD8, 0xFF][..])
    );
    let sent = h.sent();
    // NOT retried, the 502 included: `Avatar` and `Download` never took the JSON reads' retry.
    assert_eq!(sent.len(), 2);
    for request in &sent {
        assert_eq!(request.header_values("X-FC-Families"), ["1"]);
        assert_eq!(request.header_values("Authorization"), ["Bearer t0ken"]);
    }
}
