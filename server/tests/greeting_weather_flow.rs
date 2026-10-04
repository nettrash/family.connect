//! Integration: today's weather in the daily greeting, for the places the
//! family's owner chose (docs/protocol.md, "Today's weather, for places the
//! owner chose").
//!
//! Every provider is a STUB on a local listener — the text deployment and
//! Open-Meteo's geocoder and forecast — that records every request it is
//! sent. Nothing here reaches the real Open-Meteo: the endpoints are pointed
//! at the stub through `[ai.lookups]`'s test-only `endpoints`.
//!
//! What matters is WHAT LEFT THE SERVER: the owner's place names to the
//! geocoder and the geocoder's own coordinates to the forecast, and nothing
//! at all when there are no places or no weather source; that the model gets
//! the forecast as data and the family gets the credit; that a failure costs
//! the weather and never the greeting; and that no place name reaches a log.

mod common;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use common::{TestServer, assert_error, spawn_server_with_config};
use family_connect::config::{Config, LookupEndpoints};
use serde_json::{Value, json};
use time::OffsetDateTime;
use time::macros::datetime;

/// The moment every greeting here is written at: 06:00 UTC, when it is the
/// 3rd in Moscow (UTC+3) and Belgrade (UTC+2) alike.
const NOW: OffsetDateTime = datetime!(2026-10-03 06:00 UTC);

/// What the stub's text deployment writes, link and all — the link is there
/// to show the filter runs on a greeting that was handed a forecast.
const GREETING: &str =
    "Good morning. Rain in Belgrade, storms in Moscow — see https://evil.example.com/x.";

const CREDIT_EN: &str = "[Weather data by Open-Meteo.com](https://open-meteo.com/)";
const CREDIT_RU: &str = "[Данные о погоде: Open-Meteo.com](https://open-meteo.com/)";

// -- the stub ------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Seen {
    path: String,
    query: Vec<(String, String)>,
    body: Value,
}

impl Seen {
    fn param(&self, key: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    fn param_keys(&self) -> Vec<&str> {
        let mut keys: Vec<&str> = self.query.iter().map(|(k, _)| k.as_str()).collect();
        keys.sort();
        keys
    }

    fn system_prompt(&self) -> String {
        self.body["messages"][0]["content"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    }

    fn user_turn(&self) -> String {
        self.body["messages"][1]["content"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    }
}

#[derive(Default)]
struct Stub {
    seen: Mutex<Vec<Seen>>,
    /// While set, the text deployment fails.
    chat_fails: Mutex<bool>,
}

impl Stub {
    fn to(&self, path: &str) -> Vec<Seen> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|seen| seen.path == path)
            .cloned()
            .collect()
    }

    fn chat(&self) -> Vec<Seen> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|seen| seen.path.ends_with("/chat/completions"))
            .cloned()
            .collect()
    }

    /// Everything that was not the text deployment.
    fn weather_requests(&self) -> Vec<Seen> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|seen| !seen.path.starts_with("/openai/"))
            .cloned()
            .collect()
    }
}

fn place(name: &str, latitude: f64, longitude: f64, country: &str, timezone: &str) -> Value {
    json!({"results": [{
        "name": name, "latitude": latitude, "longitude": longitude, "country": country,
        "admin1": format!("{name} region"), "feature_code": "PPLC", "timezone": timezone,
    }]})
}

fn forecast(offset: i64, codes: [i64; 2], max: [f64; 2], min: [f64; 2], rain: [i64; 2]) -> Value {
    json!({
        "utc_offset_seconds": offset,
        "daily_units": {"temperature_2m_max": "°C", "precipitation_probability_max": "%"},
        "daily": {
            "time": ["2026-10-03", "2026-10-04"],
            "weather_code": codes,
            "temperature_2m_max": max,
            "temperature_2m_min": min,
            "precipitation_sum": [0.0, 0.0],
            "precipitation_probability_max": rain,
            "wind_speed_10m_max": [10.0, 10.0],
        },
        "current": {"time": "2026-10-03T09:00", "temperature_2m": 9.0,
                    "weather_code": codes[0], "wind_speed_10m": 5.0},
    })
}

async fn stub(State(stub): State<Arc<Stub>>, uri: Uri, body: Bytes) -> Response {
    let path = uri.path().to_string();
    let query: Vec<(String, String)> = uri
        .query()
        .and_then(|q| reqwest::Url::parse(&format!("http://x/?{q}")).ok())
        .map(|url| url.query_pairs().into_owned().collect())
        .unwrap_or_default();
    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let seen = Seen {
        path: path.clone(),
        query,
        body,
    };
    stub.seen.lock().unwrap().push(seen.clone());

    if path.ends_with("/chat/completions") {
        if *stub.chat_fails.lock().unwrap() {
            return (StatusCode::INTERNAL_SERVER_ERROR, "nope").into_response();
        }
        let events = [
            json!({"choices": [{"delta": {"content": GREETING}}]}),
            json!({"choices": [{"delta": {}, "finish_reason": "stop"}]}),
            json!({"choices": [], "usage": {"prompt_tokens": 40, "completion_tokens": 12}}),
        ];
        let mut out = String::new();
        for event in events {
            out.push_str(&format!("data: {event}\n\n"));
        }
        out.push_str("data: [DONE]\n\n");
        return (
            [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
            out,
        )
            .into_response();
    }
    if path == "/geo/search" {
        return match seen.param("name").unwrap_or_default() {
            "Moscow" => axum::Json(place(
                "Moscow",
                55.75222,
                37.61556,
                "Russia",
                "Europe/Moscow",
            ))
            .into_response(),
            "Belgrade" => axum::Json(place(
                "Belgrade",
                44.80401,
                20.46513,
                "Serbia",
                "Europe/Belgrade",
            ))
            .into_response(),
            // Open-Meteo leaves `results` out when nothing matched.
            "Nowhere" => axum::Json(json!({"generationtime_ms": 0.1})).into_response(),
            "Slowville" => {
                tokio::time::sleep(Duration::from_secs(40)).await;
                axum::Json(place("Slowville", 1.0, 1.0, "Nowhereland", "UTC")).into_response()
            }
            // "Atlantis", and anything else.
            _ => (StatusCode::INTERNAL_SERVER_ERROR, "geocoder down").into_response(),
        };
    }
    if path == "/forecast" {
        return match seen.param("latitude").unwrap_or_default() {
            "55.75" => axum::Json(forecast(
                3 * 3600,
                [95, 3],
                [17.3, 12.0],
                [8.1, 5.0],
                [65, 10],
            ))
            .into_response(),
            "44.80" | "44.8" => axum::Json(forecast(
                2 * 3600,
                [61, 0],
                [21.4, 23.0],
                [12.2, 13.0],
                [80, 0],
            ))
            .into_response(),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, "forecast down").into_response(),
        };
    }
    (StatusCode::NOT_FOUND, "no such stub").into_response()
}

async fn spawn_stub() -> (Arc<Stub>, SocketAddr) {
    let state = Arc::new(Stub::default());
    let router = Router::new().fallback(stub).with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("binding the stub");
    let addr = listener.local_addr().expect("stub address");
    tokio::spawn(async move {
        axum::serve(listener, router).await.expect("stub crashed");
    });
    (state, addr)
}

fn endpoints(addr: SocketAddr) -> LookupEndpoints {
    LookupEndpoints {
        brave_web: format!("http://{addr}/brave/web"),
        brave_news: format!("http://{addr}/brave/news"),
        geocoding: format!("http://{addr}/geo/search"),
        forecast: format!("http://{addr}/forecast"),
        geocoding_keyed: format!("http://{addr}/geo/search"),
        forecast_keyed: format!("http://{addr}/forecast"),
        wikipedia: format!("http://{addr}/wiki/{{lang}}"),
    }
}

/// A server that greets at 00:00 UTC, with the assistant on the stub, and
/// the weather source on or off.
async fn server(addr: SocketAddr, weather: bool, tweak: impl FnOnce(&mut Config)) -> TestServer {
    spawn_server_with_config(move |cfg| {
        cfg.ai.enabled = true;
        cfg.ai.endpoint = format!("http://{addr}");
        cfg.ai.deployment = "test-gpt".to_string();
        cfg.ai.api_key = "test-key".to_string();
        cfg.ai.processor = "Microsoft — Azure OpenAI".to_string();
        cfg.ai.title = "Assistant".to_string();
        cfg.greetings.enabled = true;
        cfg.greetings.hour_utc = 0;
        cfg.greetings.minute = 0;
        cfg.ai.lookups.weather = weather;
        cfg.ai.lookups.endpoints = endpoints(addr);
        tweak(cfg);
    })
    .await
}

/// A greeted family: the owner's token and the family chat. A Leo birthday,
/// so the request carries a sign as well.
async fn greeted_family(ts: &TestServer, language: &str) -> (String, i64) {
    let (owner, _) = ts.register("owner", "Olive").await;
    ts.create_family(&owner, "The Smiths").await;
    let chat_id = ts.family_chat_id(&owner).await;
    let response = ts
        .put(&owner, "/me/birthday", json!({"month": 8, "day": 8}))
        .await;
    assert_eq!(response.status(), 200, "setting a birthday");
    let response = ts
        .patch(
            &owner,
            "/families/mine",
            json!({"language": language, "ai_greeting": true}),
        )
        .await;
    assert_eq!(response.status(), 200, "turning the greeting on");
    (owner, chat_id)
}

async fn set_places(ts: &TestServer, owner: &str, places: Value) -> Value {
    let response = ts
        .patch(owner, "/families/mine", json!({"greeting_places": places}))
        .await;
    assert_eq!(response.status(), 200, "setting greeting_places");
    let body: Value = response.json().await.expect("JSON");
    body["family"]["greeting_places"].clone()
}

async fn run(ts: &TestServer) -> u64 {
    family_connect::greetings::post_daily_greetings_at(&ts.state, NOW)
        .await
        .expect("the greeting job runs")
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

async fn family_mine(ts: &TestServer, token: &str) -> Value {
    ts.get(token, "/families/mine")
        .await
        .json()
        .await
        .expect("JSON")
}

/// The forecast list handed to the model, parsed back out of the user turn.
fn forecast_data(request: &Seen) -> Vec<Value> {
    let turn = request.user_turn();
    let Some(start) = turn.find("\n[") else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<Value>>(&turn[start + 1..]).expect("the data is a JSON list")
}

#[derive(Clone, Default)]
struct CapturedLog(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for CapturedLog {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl CapturedLog {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

fn capture_log() -> (CapturedLog, tracing::subscriber::DefaultGuard) {
    let log = CapturedLog::default();
    let writer = log.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        // No timestamps: a clock reading like `…:17.3…` would match a
        // number this file looks for.
        .without_time()
        .finish();
    (log, tracing::subscriber::set_default(subscriber))
}

// -- the weather reaches the model as data, and the family gets the credit ------

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn two_places_reach_the_model_as_data_and_the_greeting_credits_open_meteo() {
    let (log, _guard) = capture_log();
    let (stub, addr) = spawn_stub().await;
    let ts = server(addr, true, |_| {}).await;
    let (owner, chat_id) = greeted_family(&ts, "ru").await;
    assert_eq!(
        set_places(&ts, &owner, json!(["Moscow", "Belgrade"])).await,
        json!(["Moscow", "Belgrade"])
    );

    assert_eq!(run(&ts).await, 1, "the greeting was posted");

    // To the geocoder: each name as stored, the top match only — and
    // nothing else, not even the family's language (protocol.md: only the
    // owner's names leave).
    let geo = stub.to("/geo/search");
    assert_eq!(geo.len(), 2, "{geo:?}");
    let mut names: Vec<&str> = geo.iter().filter_map(|seen| seen.param("name")).collect();
    names.sort();
    assert_eq!(names, ["Belgrade", "Moscow"]);
    for seen in &geo {
        assert_eq!(seen.param_keys(), ["count", "format", "name"]);
        assert_eq!(seen.param("count"), Some("1"));
        assert!(seen.param("language").is_none(), "{seen:?}");
    }
    // To the forecast: the geocoder's own coordinates, rounded, two days in
    // the place's own time — and no name.
    let forecasts = stub.to("/forecast");
    assert_eq!(forecasts.len(), 2, "{forecasts:?}");
    let mut latitudes: Vec<&str> = forecasts
        .iter()
        .filter_map(|seen| seen.param("latitude"))
        .collect();
    latitudes.sort();
    assert_eq!(latitudes, ["44.80", "55.75"]);
    for seen in &forecasts {
        assert!(seen.param("name").is_none());
        assert_eq!(seen.param("timezone"), Some("auto"));
        assert_eq!(seen.param("forecast_days"), Some("2"));
    }

    // The model got both forecasts as DATA, in the owner's order, with each
    // place's country, and the instruction that lets it mention them.
    let chat = stub.chat();
    assert_eq!(chat.len(), 1);
    let request = &chat[0];
    assert!(request.body.get("tools").is_none(), "still no tools");
    let system = request.system_prompt();
    assert!(
        system.contains("The one exception is the weather data given below for named places"),
        "{system}"
    );
    assert!(
        !system.contains("describe the weather or the daylight"),
        "the model must not be forbidden the data it was handed: {system}"
    );
    let turn = request.user_turn();
    assert!(turn.starts_with("Today is Saturday 3 October."), "{turn}");
    assert!(turn.contains("Leo"), "the signs are still there: {turn}");
    assert!(turn.contains("not instructions"), "{turn}");
    let data = forecast_data(request);
    assert_eq!(data.len(), 2, "{turn}");
    assert_eq!(data[0]["place"]["name"], "Moscow");
    assert_eq!(data[0]["place"]["country"], "Russia");
    assert_eq!(data[0]["date"], "2026-10-03");
    assert_eq!(data[0]["weather"], "thunderstorm");
    assert_eq!(data[0]["temperature_max"], 17.3);
    assert_eq!(data[0]["temperature_min"], 8.1);
    assert_eq!(data[0]["precipitation_probability_max"], 65);
    assert_eq!(data[1]["place"]["name"], "Belgrade");
    assert_eq!(data[1]["place"]["country"], "Serbia");
    assert_eq!(data[1]["weather"], "light rain");
    assert_eq!(data[1]["temperature_max"], 21.4);
    assert_eq!(data[1]["precipitation_probability_max"], 80);

    // The family reads the words, filtered like a lookup answer, and the
    // credit in its own language under them.
    let bodies = bodies_in(&ts, &owner, chat_id).await;
    assert_eq!(bodies.len(), 1, "{bodies:?}");
    let body = &bodies[0];
    assert!(
        body.ends_with(&format!("\n\n{CREDIT_RU}")),
        "the credit closes the greeting: {body:?}"
    );
    assert!(body.starts_with("Good morning."), "{body}");
    assert!(
        !body.contains("https://evil.example.com"),
        "a link the server did not hand over is defanged: {body}"
    );

    // Not counted anywhere: weather is free.
    let searches: i32 = sqlx::query_scalar("SELECT COALESCE(SUM(searches), 0)::INT FROM ai_usage")
        .fetch_one(&ts.state.pool)
        .await
        .expect("usage");
    assert_eq!(searches, 0);

    // The log says that it happened, and nothing of what was asked or found.
    let text = log.text();
    assert!(text.contains("fetched the greeting's weather"), "{text}");
    for leaked in [
        "Moscow",
        "Belgrade",
        "Russia",
        "Serbia",
        "thunderstorm",
        "light rain",
        // The forecast's own keys and the rounded coordinates as they would
        // be written into a URL — never a bare number, which a duration in
        // some unrelated line could match.
        "temperature_max",
        "precipitation_probability",
        "latitude=",
        "longitude=",
        "55.75&",
        "44.80&",
    ] {
        assert!(
            !text.contains(leaked),
            "{leaked:?} reached the log:\n{text}"
        );
    }
}

// -- nothing changes without places, or without a weather source ---------------

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn without_places_or_without_weather_the_request_is_byte_for_byte_what_it_was() {
    // (a) the weather source on, no places;
    let (stub_a, addr_a) = spawn_stub().await;
    let a = server(addr_a, true, |_| {}).await;
    let (owner_a, chat_a) = greeted_family(&a, "en").await;
    assert_eq!(
        family_mine(&a, &owner_a).await["family"]["greeting_places"],
        json!([]),
        "always present, empty by default"
    );
    assert_eq!(run(&a).await, 1);

    // (b) places, and no weather source on the server;
    let (stub_b, addr_b) = spawn_stub().await;
    let b = server(addr_b, false, |_| {}).await;
    let (owner_b, chat_b) = greeted_family(&b, "en").await;
    set_places(&b, &owner_b, json!(["Moscow", "Belgrade"])).await;
    assert_eq!(run(&b).await, 1);

    // (c) neither — the greeting as it was before any of this.
    let (stub_c, addr_c) = spawn_stub().await;
    let c = server(addr_c, false, |_| {}).await;
    let (owner_c, chat_c) = greeted_family(&c, "en").await;
    assert_eq!(run(&c).await, 1);

    for stub in [&stub_a, &stub_b, &stub_c] {
        assert!(
            stub.weather_requests().is_empty(),
            "nothing may leave for the weather provider: {:?}",
            stub.weather_requests()
        );
    }
    let request_c = stub_c.chat()[0].body.clone();
    assert_eq!(stub_a.chat()[0].body, request_c, "(a) differs");
    assert_eq!(stub_b.chat()[0].body, request_c, "(b) differs");
    assert!(
        stub_c.chat()[0]
            .system_prompt()
            .contains("describe the weather or the daylight"),
        "the old instruction, still forbidding the weather"
    );

    // No credit, and the model's words untouched — link and all.
    for (ts, owner, chat) in [
        (&a, &owner_a, chat_a),
        (&b, &owner_b, chat_b),
        (&c, &owner_c, chat_c),
    ] {
        assert_eq!(bodies_in(ts, owner, chat).await, [GREETING]);
    }

    // The availability flag: on only where the weather source is.
    assert_eq!(
        family_mine(&a, &owner_a).await["assistant"]["greeting_weather"],
        true
    );
    assert_eq!(
        family_mine(&b, &owner_b).await["assistant"]["greeting_weather"],
        false
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_flag_needs_a_server_that_greets_at_all() {
    let (_stub, addr) = spawn_stub().await;
    let ts = server(addr, true, |cfg| cfg.greetings.enabled = false).await;
    let (owner, _) = ts.register("owner", "Olive").await;
    ts.create_family(&owner, "The Smiths").await;
    let mine = family_mine(&ts, &owner).await;
    assert_eq!(mine["assistant"]["greeting_weather"], false, "{mine}");
}

// -- a failure costs the weather, never the greeting ---------------------------

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_failing_place_is_left_out_and_the_others_are_used() {
    let (log, _guard) = capture_log();
    let (stub, addr) = spawn_stub().await;
    let ts = server(addr, true, |_| {}).await;
    let (owner, chat_id) = greeted_family(&ts, "en").await;
    set_places(&ts, &owner, json!(["Atlantis", "Belgrade", "Nowhere"])).await;

    assert_eq!(run(&ts).await, 1);

    let request = &stub.chat()[0];
    let data = forecast_data(request);
    assert_eq!(data.len(), 1, "{}", request.user_turn());
    assert_eq!(data[0]["place"]["name"], "Belgrade");
    let bodies = bodies_in(&ts, &owner, chat_id).await;
    assert_eq!(bodies.len(), 1);
    assert!(
        bodies[0].ends_with(&format!("\n\n{CREDIT_EN}")),
        "{bodies:?}"
    );

    let text = log.text();
    assert!(text.contains("a greeting place had no forecast"), "{text}");
    for leaked in ["Atlantis", "Nowhere", "Belgrade", "Serbia"] {
        assert!(
            !text.contains(leaked),
            "{leaked:?} reached the log:\n{text}"
        );
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn when_every_place_fails_the_greeting_is_the_usual_one() {
    let (stub, addr) = spawn_stub().await;
    let ts = server(addr, true, |_| {}).await;
    let (owner, chat_id) = greeted_family(&ts, "en").await;
    set_places(&ts, &owner, json!(["Atlantis", "Nowhere"])).await;
    assert_eq!(run(&ts).await, 1);
    assert_eq!(stub.to("/geo/search").len(), 2, "both were asked");

    // The same request a family with no places sends.
    let (plain, plain_addr) = spawn_stub().await;
    let other = server(plain_addr, true, |_| {}).await;
    greeted_family(&other, "en").await;
    assert_eq!(run(&other).await, 1);
    assert_eq!(stub.chat()[0].body, plain.chat()[0].body);
    assert_eq!(
        bodies_in(&ts, &owner, chat_id).await,
        [GREETING],
        "no credit"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_whole_fetch_has_one_deadline_and_keeps_what_arrived() {
    let (stub, addr) = spawn_stub().await;
    // Each request may take 30 s; the greeting waits 8 s for all of them.
    let ts = server(addr, true, |cfg| cfg.ai.lookups.timeout_secs = 30).await;
    let (owner, chat_id) = greeted_family(&ts, "en").await;
    set_places(&ts, &owner, json!(["Slowville", "Belgrade"])).await;

    let started = Instant::now();
    assert_eq!(run(&ts).await, 1, "the greeting was still posted");
    let took = started.elapsed();
    assert!(
        took >= Duration::from_secs(7) && took < Duration::from_secs(15),
        "the greeting waited {took:?}, not about 8 s"
    );

    let data = forecast_data(&stub.chat()[0]);
    assert_eq!(data.len(), 1, "the place that arrived in time is kept");
    assert_eq!(data[0]["place"]["name"], "Belgrade");
    let bodies = bodies_in(&ts, &owner, chat_id).await;
    assert!(bodies[0].ends_with(CREDIT_EN), "{bodies:?}");
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_greeting_retried_after_a_model_failure_does_not_ask_again() {
    let (stub, addr) = spawn_stub().await;
    let ts = server(addr, true, |_| {}).await;
    let (owner, chat_id) = greeted_family(&ts, "en").await;
    set_places(&ts, &owner, json!(["Moscow"])).await;

    *stub.chat_fails.lock().unwrap() = true;
    assert_eq!(run(&ts).await, 0, "the model failed, nothing was posted");
    assert!(bodies_in(&ts, &owner, chat_id).await.is_empty());
    *stub.chat_fails.lock().unwrap() = false;
    assert_eq!(run(&ts).await, 1);

    // The next tick found the geocoder's match and the forecast in memory.
    assert_eq!(stub.to("/geo/search").len(), 1);
    assert_eq!(stub.to("/forecast").len(), 1);
    assert_eq!(forecast_data(&stub.chat()[1]).len(), 1);
}

// -- the owner's setting -------------------------------------------------------

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_place_list_is_the_owners_and_is_validated_before_anything_is_written() {
    let (log, _guard) = capture_log();
    let (stub, addr) = spawn_stub().await;
    let ts = server(addr, true, |_| {}).await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (_, invite) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    let (member, _) = ts.register("member", "Mark").await;
    ts.join(&member, &invite, "joined").await;

    // Kept as the server keeps it: trimmed, folded, the first spelling of a
    // repeat, counted after the repeats.
    assert_eq!(
        set_places(
            &ts,
            &owner,
            json!(["  Moscow ", "Novi \n Sad", "moscow", "Belgrade"])
        )
        .await,
        json!(["Moscow", "Novi Sad", "Belgrade"])
    );

    let refusals = [
        json!(["Moscow", "Belgrade", "Paris", "Tokyo"]),
        json!(["Moscow", "  "]),
        json!([""]),
        json!(["x".repeat(81)]),
        json!(["Bel\u{7}grade"]),
        json!(null),
        json!("Moscow"),
        json!([42]),
    ];
    for refused in refusals {
        let response = ts
            .patch(
                &owner,
                "/families/mine",
                json!({"greeting_places": refused, "ai_greeting": true}),
            )
            .await;
        assert_error(response, 400, "validation").await;
    }
    // A non-owner cannot set it, even to something valid.
    let response = ts
        .patch(
            &member,
            "/families/mine",
            json!({"greeting_places": ["Paris"]}),
        )
        .await;
    assert_error(response, 403, "not_family_owner").await;

    // Nothing was written by any of them — neither the list nor the switch
    // sent beside it — and every member reads the list, on both reads.
    let mine = family_mine(&ts, &member).await;
    assert_eq!(
        mine["family"]["greeting_places"],
        json!(["Moscow", "Novi Sad", "Belgrade"])
    );
    assert_eq!(mine["family"]["ai_greeting"], false);
    let me: Value = ts.get(&member, "/me").await.json().await.expect("JSON");
    assert_eq!(
        me["family"]["greeting_places"],
        json!(["Moscow", "Novi Sad", "Belgrade"])
    );

    // 80 characters is fine; absent leaves the list alone; [] clears it.
    let eighty = "ж".repeat(80);
    assert_eq!(
        set_places(&ts, &owner, json!([eighty])).await,
        json!([eighty])
    );
    let response = ts
        .patch(&owner, "/families/mine", json!({"ai_greeting": true}))
        .await;
    let body: Value = response.json().await.expect("JSON");
    assert_eq!(body["family"]["greeting_places"], json!([eighty]));
    assert_eq!(set_places(&ts, &owner, json!([])).await, json!([]));

    // Setting it sends nothing anywhere, and logs no name.
    assert!(stub.weather_requests().is_empty());
    let text = log.text();
    for leaked in ["Moscow", "Novi Sad", "Belgrade", "Tokyo", "Paris"] {
        assert!(
            !text.contains(leaked),
            "{leaked:?} reached the log:\n{text}"
        );
    }
}

/// The CHECK holds the shape even against a write that skips the handler.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_database_refuses_a_fourth_place_or_a_null() {
    let (_stub, addr) = spawn_stub().await;
    let ts = server(addr, true, |_| {}).await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (family_id, _) = ts.create_family(&owner, "The Smiths").await;
    for bad in [
        "ARRAY['a','b','c','d']::TEXT[]",
        "ARRAY['a',NULL]::TEXT[]",
        "ARRAY[ARRAY['a'],ARRAY['b']]::TEXT[]",
    ] {
        let refused = sqlx::query(&format!(
            "UPDATE families SET greeting_places = {bad} WHERE id = $1"
        ))
        .bind(family_id)
        .execute(&ts.state.pool)
        .await;
        let Err(sqlx::Error::Database(error)) = refused else {
            panic!("{bad} was written");
        };
        assert_eq!(
            error.constraint(),
            Some("families_greeting_places_check"),
            "{bad}"
        );
    }
}
