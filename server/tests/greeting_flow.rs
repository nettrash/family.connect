//! Integration: the assistant's one unprompted message a day
//! (docs/protocol.md, "The daily greeting").
//!
//! The four things worth an integration test are the four that cannot be
//! checked by reading: that it takes TWO switches, that it posts exactly ONCE
//! however many times the job runs, that it never wakes a device, and that
//! what reaches the model carries signs and nothing that names anybody.
//!
//! The job is driven by calling `post_daily_greetings` directly, exactly as
//! `retention_flow` calls the retention sweep: the ticker in `main.rs` is
//! three lines and is never built by the test harness.

mod common;

use axum::Router;
use axum::extract::State;
use axum::response::Json;
use axum::routing::post;
use common::{TestServer, spawn_server, spawn_server_with_config};
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use time::macros::datetime;

/// Records every request body the server sends, and answers with a fixed
/// stream — the greeting's text is not what these tests are about.
#[derive(Default)]
struct MockProvider {
    bodies: Mutex<Vec<Value>>,
    /// When true the endpoint fails, for the test that a failure writes
    /// nothing at all.
    fail: Mutex<bool>,
}

impl MockProvider {
    fn bodies(&self) -> Vec<Value> {
        self.bodies.lock().expect("mock lock").clone()
    }

    /// Everything the model was told, as one string: the system prompt and
    /// every turn's content run together. What the tests assert on is what
    /// is NOT in here.
    fn everything_sent(&self) -> String {
        self.bodies()
            .iter()
            .flat_map(|body| {
                body["messages"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
            })
            .map(|message| message["content"].to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

async fn mock_chat(
    axum::extract::Path(_deployment): axum::extract::Path<String>,
    State(mock): State<Arc<MockProvider>>,
    Json(body): Json<Value>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    mock.bodies.lock().expect("mock lock").push(body);
    if *mock.fail.lock().expect("mock lock") {
        return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "nope").into_response();
    }
    (
        [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
        concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Good morning. \"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"A gentle day to you all.\"}}],",
            "\"usage\":{\"prompt_tokens\":31,\"completion_tokens\":7}}\n\n",
            "data: [DONE]\n\n",
        )
        .to_string(),
    )
        .into_response()
}

async fn spawn_mock_provider() -> (Arc<MockProvider>, SocketAddr) {
    let mock = Arc::new(MockProvider::default());
    let router = Router::new()
        .route(
            "/openai/deployments/{deployment}/chat/completions",
            post(mock_chat),
        )
        .with_state(mock.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("binding the mock provider port");
    let addr = listener.local_addr().expect("mock local addr");
    tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("mock provider crashed");
    });
    (mock, addr)
}

/// A server whose operator has turned greetings on, at an hour that has
/// certainly passed — 00:00, so the job is due whenever the suite runs.
async fn server_with_greetings(addr: SocketAddr) -> TestServer {
    server_with_greetings_tweaked(addr, |_| {}).await
}

async fn server_with_greetings_tweaked(
    addr: SocketAddr,
    extra: impl FnOnce(&mut family_connect::config::Config),
) -> TestServer {
    spawn_server_with_config(move |cfg| {
        cfg.ai.enabled = true;
        cfg.ai.endpoint = format!("http://{addr}");
        cfg.ai.deployment = "test-gpt".to_string();
        cfg.ai.api_key = "test-key".to_string();
        cfg.ai.title = "Assistant".to_string();
        cfg.greetings.enabled = true;
        cfg.greetings.hour_utc = 0;
        cfg.greetings.minute = 0;
        extra(cfg);
    })
    .await
}

/// A family whose owner has set a birthday, so there is a sign to mention.
/// Returns the owner's token and the family chat id.
async fn family(ts: &TestServer) -> (String, i64) {
    let (owner, _) = ts.register("owner", "Olive").await;
    let (_, _invite) = ts.create_family(&owner, "The Smiths").await;
    let chat_id = ts.family_chat_id(&owner).await;
    // 8 August is a Leo, and the display name is Olive — so a test can ask
    // whether "Leo" travelled and whether "Olive" did not.
    let response = ts
        .put(&owner, "/me/birthday", json!({"month": 8, "day": 8}))
        .await;
    assert_eq!(response.status(), 200, "setting a birthday");
    (owner, chat_id)
}

async fn set_language(ts: &TestServer, owner: &str, language: &str) {
    let response = ts
        .patch(owner, "/families/mine", json!({"language": language}))
        .await;
    assert_eq!(response.status(), 200, "setting the family language");
}

async fn set_greeting(ts: &TestServer, owner: &str, on: bool) {
    let response = ts
        .patch(owner, "/families/mine", json!({"ai_greeting": on}))
        .await;
    assert_eq!(response.status(), 200, "setting ai_greeting");
}

async fn bodies_in(ts: &TestServer, token: &str, chat_id: i64) -> Vec<String> {
    let page: Value = ts
        .get(token, &format!("/chats/{chat_id}/messages"))
        .await
        .json()
        .await
        .expect("JSON");
    page["messages"]
        .as_array()
        .expect("array")
        .iter()
        .map(|m| m["body"].as_str().unwrap_or_default().to_string())
        .collect()
}

async fn run(ts: &TestServer) -> u64 {
    family_connect::greetings::post_daily_greetings(&ts.state)
        .await
        .expect("the greeting job runs")
}

// ---------------------------------------------------------------------------
// Two switches

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_family_that_did_not_ask_is_not_greeted() {
    let (mock, addr) = spawn_mock_provider().await;
    let server = server_with_greetings(addr).await;
    let (owner, chat_id) = family(&server).await;
    set_language(&server, &owner, "en").await;
    // ai_greeting is FALSE — nobody has turned it on.

    assert_eq!(run(&server).await, 0);
    assert!(
        bodies_in(&server, &owner, chat_id).await.is_empty(),
        "a family that never asked for a greeting got one"
    );
    assert!(
        mock.bodies().is_empty(),
        "the model was called for a family that had not opted in — that is the operator's bill"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_server_that_did_not_ask_greets_nobody() {
    let (mock, addr) = spawn_mock_provider().await;
    // Operator's half OFF, family's half on.
    let server = server_with_greetings_tweaked(addr, |cfg| cfg.greetings.enabled = false).await;
    let (owner, chat_id) = family(&server).await;
    set_language(&server, &owner, "en").await;
    set_greeting(&server, &owner, true).await;

    assert_eq!(run(&server).await, 0);
    assert!(bodies_in(&server, &owner, chat_id).await.is_empty());
    assert!(mock.bodies().is_empty());
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_server_with_no_assistant_configured_greets_nobody() {
    // No [ai] at all: the greeting is written by that deployment, so there is
    // nothing to write it with.
    let server = spawn_server_with_config(|cfg| {
        cfg.greetings.enabled = true;
        cfg.greetings.hour_utc = 0;
    })
    .await;
    let (owner, chat_id) = family(&server).await;
    set_language(&server, &owner, "en").await;
    set_greeting(&server, &owner, true).await;

    assert_eq!(run(&server).await, 0);
    assert!(bodies_in(&server, &owner, chat_id).await.is_empty());
}

// ---------------------------------------------------------------------------
// Once a day, whatever happens

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn both_switches_on_posts_exactly_one_greeting_however_often_the_job_runs() {
    let (mock, addr) = spawn_mock_provider().await;
    let server = server_with_greetings(addr).await;
    let (owner, chat_id) = family(&server).await;
    set_language(&server, &owner, "en").await;
    set_greeting(&server, &owner, true).await;

    assert_eq!(run(&server).await, 1, "the first run posts the greeting");
    // The ticker fires every five minutes all day. Every run after the first
    // must be a no-op — and must not spend a model call finding that out.
    assert_eq!(run(&server).await, 0, "the second run posted a second time");
    assert_eq!(run(&server).await, 0);

    let bodies = bodies_in(&server, &owner, chat_id).await;
    assert_eq!(
        bodies.len(),
        1,
        "the family chat should hold exactly one greeting, holds: {bodies:?}"
    );
    assert!(bodies[0].starts_with("Good morning."));
    assert_eq!(
        mock.bodies().len(),
        1,
        "the model was called again after the greeting already existed"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn nothing_is_posted_before_the_configured_hour_and_something_is_at_it() {
    let (mock, addr) = spawn_mock_provider().await;
    let server = server_with_greetings_tweaked(addr, |cfg| {
        cfg.greetings.hour_utc = 6;
        cfg.greetings.minute = 30;
    })
    .await;
    let (owner, chat_id) = family(&server).await;
    set_language(&server, &owner, "en").await;
    set_greeting(&server, &owner, true).await;

    // The clock is handed IN, so this is the same test at any wall-clock
    // time: the first version skipped itself for an hour a day, which is a
    // test that goes green by not running.
    let early = datetime!(2026-09-08 06:29:59 UTC);
    assert_eq!(
        family_connect::greetings::post_daily_greetings_at(&server.state, early)
            .await
            .expect("runs"),
        0
    );
    assert!(bodies_in(&server, &owner, chat_id).await.is_empty());
    assert!(
        mock.bodies().is_empty(),
        "the model was called before the hour"
    );

    let due = datetime!(2026-09-08 06:30:00 UTC);
    assert_eq!(
        family_connect::greetings::post_daily_greetings_at(&server.state, due)
            .await
            .expect("runs"),
        1
    );
    assert_eq!(bodies_in(&server, &owner, chat_id).await.len(), 1);
    // The id and the line the model was told name the SAME day — the one
    // the clock said, not a second reading of it.
    assert!(
        mock.everything_sent().contains("Tuesday 8 September"),
        "{}",
        mock.everything_sent()
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_provider_failure_leaves_no_message_at_all() {
    let (mock, addr) = spawn_mock_provider().await;
    let server = server_with_greetings(addr).await;
    let (owner, chat_id) = family(&server).await;
    set_language(&server, &owner, "en").await;
    set_greeting(&server, &owner, true).await;
    *mock.fail.lock().expect("mock lock") = true;

    assert_eq!(run(&server).await, 0, "a failure must post nothing");
    // The whole reason this job does not reuse the mention path: that one
    // creates the row FIRST and streams into it, so a failure leaves a blank
    // bubble every client draws as an assistant still thinking, for ever.
    assert!(
        bodies_in(&server, &owner, chat_id).await.is_empty(),
        "a failed greeting left a message in the family chat"
    );

    // And the day is not burnt: the next run tries again and succeeds.
    *mock.fail.lock().expect("mock lock") = false;
    assert_eq!(run(&server).await, 1);
    assert_eq!(bodies_in(&server, &owner, chat_id).await.len(), 1);
}

// ---------------------------------------------------------------------------
// It never wakes anybody

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_greeting_never_pushes() {
    let (_mock, addr) = spawn_mock_provider().await;
    let server = server_with_greetings(addr).await;
    let (owner, _chat_id) = family(&server).await;
    set_language(&server, &owner, "en").await;
    set_greeting(&server, &owner, true).await;
    // A registered device is what makes this test able to fail: without one
    // there would be nothing to wake and the assertion would pass vacuously.
    let response = server
        .post(
            &owner,
            "/devices",
            json!({"platform": "ios", "push_token": "aaaabbbbccccdddd"}),
        )
        .await;
    assert_eq!(response.status(), 201, "registering a device");

    assert_eq!(run(&server).await, 1);
    assert!(
        server.push.calls().is_empty(),
        "the greeting woke a device: no timezone travels on the wire, so the server \
         cannot know whose night its configured hour is — and the assistant cannot be \
         blocked or muted, so there would be no off switch"
    );
}

// ---------------------------------------------------------------------------
// What reaches the model

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_sign_travels_and_the_person_does_not() {
    let (mock, addr) = spawn_mock_provider().await;
    let server = server_with_greetings(addr).await;
    let (owner, _chat_id) = family(&server).await;
    set_language(&server, &owner, "en").await;
    set_greeting(&server, &owner, true).await;

    assert_eq!(run(&server).await, 1);
    let sent = mock.everything_sent();
    assert!(
        sent.contains("Leo"),
        "the family's sign should have been named: {sent}"
    );
    // The three things the protocol says never travel. A greeting has no
    // member's deliberate act behind it, so it sends less than a mention
    // does — and a mention already refuses to send the roster.
    assert!(
        !sent.contains("Olive"),
        "a member's display name reached the model: {sent}"
    );
    assert!(
        !sent.contains("The Smiths"),
        "the family's name reached the model: {sent}"
    );
    assert!(
        !sent.contains("owner"),
        "a member's username reached the model: {sent}"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_family_with_no_birthdays_is_still_greeted() {
    let (mock, addr) = spawn_mock_provider().await;
    let server = server_with_greetings(addr).await;
    // Deliberately NOT the `family` helper: nobody here has set a birthday.
    let (owner, _) = server.register("owner", "Olive").await;
    server.create_family(&owner, "The Smiths").await;
    let chat_id = server.family_chat_id(&owner).await;
    set_language(&server, &owner, "en").await;
    set_greeting(&server, &owner, true).await;

    assert_eq!(
        run(&server).await,
        1,
        "a family where nobody set a birthday should still get the note about the day"
    );
    assert_eq!(bodies_in(&server, &owner, chat_id).await.len(), 1);
    let sent = mock.everything_sent();
    assert!(
        sent.contains("No star signs"),
        "the model should be told plainly that there are none: {sent}"
    );
}

// ---------------------------------------------------------------------------
// Language

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_family_with_no_language_and_no_operator_default_is_not_greeted() {
    let (mock, addr) = spawn_mock_provider().await;
    let server = server_with_greetings(addr).await;
    let (owner, chat_id) = family(&server).await;
    set_greeting(&server, &owner, true).await;
    // No family language, and the operator named none either.

    assert_eq!(run(&server).await, 0);
    assert!(
        bodies_in(&server, &owner, chat_id).await.is_empty(),
        "a family that never chose a language was greeted anyway — in which language?"
    );
    assert!(mock.bodies().is_empty());
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_operator_default_covers_a_family_that_chose_none() {
    let (mock, addr) = spawn_mock_provider().await;
    let server =
        server_with_greetings_tweaked(addr, |cfg| cfg.greetings.language = Some("ru".to_string()))
            .await;
    let (owner, chat_id) = family(&server).await;
    set_greeting(&server, &owner, true).await;

    assert_eq!(run(&server).await, 1);
    assert_eq!(bodies_in(&server, &owner, chat_id).await.len(), 1);
    assert!(
        mock.everything_sent().contains("Answer in Russian."),
        "the operator's language should have reached the prompt: {}",
        mock.everything_sent()
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_family_language_wins_over_the_operator_default() {
    let (mock, addr) = spawn_mock_provider().await;
    let server =
        server_with_greetings_tweaked(addr, |cfg| cfg.greetings.language = Some("ru".to_string()))
            .await;
    let (owner, _chat_id) = family(&server).await;
    set_language(&server, &owner, "de").await;
    set_greeting(&server, &owner, true).await;

    assert_eq!(run(&server).await, 1);
    let sent = mock.everything_sent();
    assert!(sent.contains("Answer in German."), "{sent}");
    assert!(!sent.contains("Answer in Russian."), "{sent}");
}

// ---------------------------------------------------------------------------
// The switch itself

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_switch_is_off_by_default_and_survives_its_neighbours() {
    let server = spawn_server().await;
    let (owner, _) = server.register("owner", "Olive").await;
    server.create_family(&owner, "The Smiths").await;
    let mine: Value = server
        .get(&owner, "/families/mine")
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(
        mine["family"]["ai_greeting"],
        json!(false),
        "a new family must not be opted in by a migration"
    );

    let response = server
        .patch(&owner, "/families/mine", json!({"ai_greeting": true}))
        .await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.expect("JSON");
    assert_eq!(body["family"]["ai_greeting"], json!(true));

    // It is bound to none of the three AI-disclosure switches, so turning the
    // one with a rule attached off must not disturb it. `ai_vision` going off
    // clears `ai_history_photos`; it may not clear this.
    let response = server
        .patch(
            &owner,
            "/families/mine",
            json!({"ai_vision": true, "ai_history_photos": true}),
        )
        .await;
    assert_eq!(response.status(), 200);
    let response = server
        .patch(&owner, "/families/mine", json!({"ai_vision": false}))
        .await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.expect("JSON");
    assert_eq!(
        body["family"]["ai_history_photos"],
        json!(false),
        "ai_vision going off should still clear its dependent"
    );
    assert_eq!(
        body["family"]["ai_greeting"],
        json!(true),
        "ai_greeting is bound to nothing and must not be cleared as a side effect"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_member_who_is_not_the_owner_cannot_set_it() {
    let server = spawn_server().await;
    let (owner, _) = server.register("owner", "Olive").await;
    let (member, _) = server.register("junior", "Junior").await;
    let (_, invite_code) = server.create_family(&owner, "The Smiths").await;
    server.set_open_policy(&owner).await;
    server.join(&member, &invite_code, "joined").await;

    let response = server
        .patch(&member, "/families/mine", json!({"ai_greeting": true}))
        .await;
    assert_eq!(response.status(), 403);
}

// ---------------------------------------------------------------------------
// The operator's half on the wire

/// `greetings_enabled` on `GET /me` is the one wire claim the protocol makes
/// about the operator's half, and it is a single conjunction — so each side
/// of the AND is pinned, and so is presence on a server that has neither.
async fn me_says_greetings_enabled(ts: &TestServer, token: &str) -> Value {
    let me: Value = ts.get(token, "/me").await.json().await.expect("JSON");
    me["greetings_enabled"].clone()
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn me_reports_the_operators_half() {
    // Both halves of the conjunction on.
    let (_mock, addr) = spawn_mock_provider().await;
    let server = server_with_greetings(addr).await;
    let (owner, _) = family(&server).await;
    assert_eq!(
        me_says_greetings_enabled(&server, &owner).await,
        json!(true)
    );

    // [greetings] off, assistant on.
    let (_mock, addr) = spawn_mock_provider().await;
    let server = server_with_greetings_tweaked(addr, |cfg| cfg.greetings.enabled = false).await;
    let (owner, _) = family(&server).await;
    assert_eq!(
        me_says_greetings_enabled(&server, &owner).await,
        json!(false)
    );

    // [greetings] on, no assistant to write one with.
    let server = spawn_server_with_config(|cfg| {
        cfg.greetings.enabled = true;
        cfg.greetings.hour_utc = 0;
    })
    .await;
    let (owner, _) = family(&server).await;
    assert_eq!(
        me_says_greetings_enabled(&server, &owner).await,
        json!(false),
        "a server that cannot write a greeting must not say it posts them"
    );

    // Neither: present, and false — never absent, never null.
    let server = spawn_server().await;
    let (owner, _) = family(&server).await;
    assert_eq!(
        me_says_greetings_enabled(&server, &owner).await,
        json!(false)
    );
}

// ---------------------------------------------------------------------------
// What the model is NOT told, and what it costs

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_model_is_not_invited_to_guess_the_season() {
    let (mock, addr) = spawn_mock_provider().await;
    let server = server_with_greetings(addr).await;
    let (owner, _) = family(&server).await;
    set_language(&server, &owner, "en").await;
    set_greeting(&server, &owner, true).await;

    assert_eq!(run(&server).await, 1);
    let sent = mock.everything_sent().to_lowercase();
    // No location and no timezone is stored for a family, so a line about
    // autumn is wrong for half the planet. The instruction must forbid it
    // rather than invite it — an earlier draft invited it.
    assert!(sent.contains("do not name a season"), "{sent}");
    assert!(!sent.contains("the season, the light"), "{sent}");
}

/// The second stated reason for not reusing the mention path: its usage
/// row is keyed to the asking user's family, which is NULL for the
/// assistant, so a greeting posted that way would cost nothing on paper.
/// Here the cost lands on the FAMILY, where Family Statistics shows it.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_greetings_cost_is_the_familys() {
    let (mock, addr) = spawn_mock_provider().await;
    let server = server_with_greetings(addr).await;
    let (owner, _) = family(&server).await;
    set_language(&server, &owner, "en").await;
    set_greeting(&server, &owner, true).await;

    let ai = |stats: Value| stats["totals"]["ai"].clone();
    let before: Value = server
        .get(&owner, "/families/mine/stats")
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(ai(before)["questions"], json!(0));

    *mock.fail.lock().expect("mock lock") = true;
    assert_eq!(run(&server).await, 0);
    let failed: Value = server
        .get(&owner, "/families/mine/stats")
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(
        ai(failed)["questions"],
        json!(0),
        "a failure that posted nothing costs nothing"
    );

    *mock.fail.lock().expect("mock lock") = false;
    assert_eq!(run(&server).await, 1);
    assert_eq!(run(&server).await, 0);
    let after: Value = server
        .get(&owner, "/families/mine/stats")
        .await
        .json()
        .await
        .expect("JSON");
    let ai = ai(after);
    assert_eq!(
        ai["questions"],
        json!(1),
        "one greeting, one question — and the no-op run adds none"
    );
    assert_eq!(ai["prompt_tokens"], json!(31));
    assert_eq!(ai["completion_tokens"], json!(7));
    assert_eq!(ai["images"], json!(0));
}

// ---------------------------------------------------------------------------
// Two families, one day

/// The idempotency design's central claim: the id is derived from the DATE
/// alone and is the same for every family, and `(chat_id, sender_id,
/// client_msg_id)` is what makes that unique per family. Two families on one
/// server must each get exactly one.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn two_families_on_one_server_each_get_exactly_one() {
    let (mock, addr) = spawn_mock_provider().await;
    let server = server_with_greetings(addr).await;
    let (owner_a, chat_a) = family(&server).await;
    let (owner_b, _) = server.register("owner_b", "Beatrix").await;
    server.create_family(&owner_b, "The Joneses").await;
    let chat_b = server.family_chat_id(&owner_b).await;
    for owner in [&owner_a, &owner_b] {
        set_language(&server, owner, "en").await;
        set_greeting(&server, owner, true).await;
    }

    assert_eq!(run(&server).await, 2, "one per family");
    assert_eq!(run(&server).await, 0, "and no second round");
    assert_eq!(bodies_in(&server, &owner_a, chat_a).await.len(), 1);
    assert_eq!(bodies_in(&server, &owner_b, chat_b).await.len(), 1);
    assert_eq!(mock.bodies().len(), 2);
}

/// Two members born under the same sign are one word to the model, not two:
/// the set is distinct, and it is in calendar order.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn signs_are_distinct_and_in_calendar_order() {
    let (mock, addr) = spawn_mock_provider().await;
    let server = server_with_greetings(addr).await;
    let (owner, _) = family(&server).await; // Olive: 8 Aug, Leo
    let (_, invite) = {
        let mine: Value = server
            .get(&owner, "/families/mine")
            .await
            .json()
            .await
            .expect("JSON");
        (
            mine["family"]["id"].as_i64().unwrap(),
            mine["family"]["invite_code"].as_str().unwrap().to_string(),
        )
    };
    server.set_open_policy(&owner).await;
    let (kid, kid_id) = server.register("kid", "Kid").await;
    server.join(&kid, &invite, "joined").await;
    let (gran, gran_id) = server.register("gran", "Gran").await;
    server.join(&gran, &invite, "joined").await;
    // Kid: another Leo (2 Aug). Gran: Aries (1 April), which sorts BEFORE Leo.
    for (id, month, day) in [(kid_id, 8, 2), (gran_id, 4, 1)] {
        let response = server
            .put(
                &owner,
                &format!("/families/members/{id}/birthday"),
                json!({"month": month, "day": day}),
            )
            .await;
        assert_eq!(response.status(), 200);
    }
    set_language(&server, &owner, "en").await;
    set_greeting(&server, &owner, true).await;

    assert_eq!(run(&server).await, 1);
    let sent = mock.everything_sent();
    assert!(
        sent.contains("Aries, Leo"),
        "distinct, calendar order: {sent}"
    );
    assert_eq!(
        sent.matches("Leo").count(),
        1,
        "one Leo for two Leos: {sent}"
    );
}
