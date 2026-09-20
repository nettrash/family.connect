//! Nobody's words reach the model before that person has said yes
//! (docs/protocol.md, "Consenting to the assistant").
//!
//! The rule Apple's review asked for and the one this product would want
//! anyway: the assistant is the only place where something a member writes
//! leaves the server the family chose, so it is the only place that asks.
//! What is pinned here is the SERVER's half — the refusal, the endpoint
//! that lifts it, the withdrawal, and the fact that the permission belongs
//! to the member and not to whoever owns the family. The clients' half,
//! the screen that names the processor before anybody answers, is theirs.
//!
//! No provider is ever called: every assertion below is about what happens
//! BEFORE the call, which is the whole point of the feature.

mod common;

use common::{TestServer, assert_error, spawn_server, spawn_server_with_config};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::http::header::AUTHORIZATION;

/// A server with the assistant configured, processor and all. The endpoint
/// is unreachable on purpose — nothing here is allowed to get far enough to
/// call it.
async fn server_with_assistant() -> TestServer {
    spawn_server_with_config(|cfg| {
        cfg.ai.enabled = true;
        cfg.ai.endpoint = "https://example.invalid".to_string();
        cfg.ai.deployment = "test-deployment".to_string();
        cfg.ai.api_key = "test-key".to_string();
        cfg.ai.processor = "Microsoft — Azure OpenAI".to_string();
        cfg.ai.title = "Assistant".to_string();
    })
    .await
}

async fn ai_chat_id(ts: &TestServer, token: &str) -> i64 {
    let body: Value = ts.get(token, "/chats").await.json().await.expect("JSON");
    body["chats"]
        .as_array()
        .expect("a chat list")
        .iter()
        .map(|entry| entry["chat"].clone())
        .find(|chat| chat["kind"] == "ai")
        .and_then(|chat| chat["id"].as_i64())
        .expect("the assistant chat exists on a configured server")
}

/// What was SAID in a chat, oldest first.
///
/// The endpoint answers newest-first, so this reverses it; and the empty
/// bodies are dropped, because a send to an `ai` chat leaves the
/// assistant's placeholder row behind before the provider is called and
/// this file is about what a member's own words do.
async fn bodies_in(ts: &TestServer, token: &str, chat_id: i64) -> Vec<String> {
    let body: Value = ts
        .get(token, &format!("/chats/{chat_id}/messages"))
        .await
        .json()
        .await
        .expect("JSON");
    let mut said: Vec<String> = body["messages"]
        .as_array()
        .expect("a message list")
        .iter()
        .filter_map(|message| message["body"].as_str())
        .filter(|body| !body.is_empty())
        .map(str::to_string)
        .collect();
    said.reverse();
    said
}

/// Every body, placeholders included — for the assertions that are about
/// nothing having been written at all.
async fn rows_in(ts: &TestServer, token: &str, chat_id: i64) -> usize {
    let body: Value = ts
        .get(token, &format!("/chats/{chat_id}/messages"))
        .await
        .json()
        .await
        .expect("JSON");
    body["messages"].as_array().expect("a message list").len()
}

async fn consent(ts: &TestServer, token: &str, granted: bool) -> reqwest::Response {
    ts.post(token, "/me/assistant-consent", json!({"granted": granted}))
        .await
}

/// `assistant_consent_at` as `GET /me` reports it.
async fn agreed_at(ts: &TestServer, token: &str) -> Value {
    let body: Value = ts.get(token, "/me").await.json().await.expect("JSON");
    body["assistant_consent_at"].clone()
}

/// A member who has been asked nothing has agreed to nothing, and their
/// first word to the assistant is refused rather than swallowed.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_assistant_chat_refuses_a_member_who_has_not_agreed() {
    let ts = server_with_assistant().await;
    let (owner, _) = ts
        .register_without_assistant_consent("owner", "Olive")
        .await;
    ts.create_family(&owner, "The Smiths").await;
    let ai = ai_chat_id(&ts, &owner).await;

    assert_eq!(
        agreed_at(&ts, &owner).await,
        Value::Null,
        "nobody asked yet"
    );

    let response = ts
        .post_message(
            &owner,
            ai,
            &uuid::Uuid::new_v4().to_string(),
            "are you there?",
        )
        .await;
    assert_error(response, 403, "assistant_consent_required").await;

    // A REFUSAL, not a silent drop: nothing was written, so the client can
    // show the consent screen and offer to send the same words again.
    assert_eq!(
        rows_in(&ts, &owner, ai).await,
        0,
        "a refused send stores neither the message nor a placeholder for a \
         reply that is never coming"
    );
}

/// The family chat still takes their ordinary words — only the mention is
/// refused, because only the mention would call the model.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_family_chat_keeps_working_and_only_the_mention_is_refused() {
    let ts = server_with_assistant().await;
    let (owner, _) = ts
        .register_without_assistant_consent("owner", "Olive")
        .await;
    ts.create_family(&owner, "The Smiths").await;
    let family = ts.family_chat_id(&owner).await;

    let ordinary = ts
        .post_message(
            &owner,
            family,
            &uuid::Uuid::new_v4().to_string(),
            "Dinner at 7?",
        )
        .await;
    assert_eq!(
        ordinary.status(),
        201,
        "declining the assistant is not leaving the conversation"
    );

    let mention = ts
        .post_message(
            &owner,
            family,
            &uuid::Uuid::new_v4().to_string(),
            "@ai when did we say dinner?",
        )
        .await;
    assert_error(mention, 403, "assistant_consent_required").await;

    assert_eq!(
        bodies_in(&ts, &owner, family).await,
        vec!["Dinner at 7?".to_string()],
        "the refused mention left no line in the family chat"
    );
}

/// The socket refuses it too, and says why — the transport a connected
/// client actually sends over. An `error` frame carrying the same code and
/// the `client_msg_id` that asked, so the client can point the consent
/// screen at the message it still holds.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_socket_refuses_it_too_and_names_the_message() {
    let ts = server_with_assistant().await;
    let (owner, _) = ts
        .register_without_assistant_consent("owner", "Olive")
        .await;
    ts.create_family(&owner, "The Smiths").await;
    let ai = ai_chat_id(&ts, &owner).await;

    let mut request = ts
        .ws_url
        .as_str()
        .into_client_request()
        .expect("building the ws request");
    request.headers_mut().insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {owner}")).expect("header value"),
    );
    let (mut ws, _response) = tokio_tungstenite::connect_async(request)
        .await
        .expect("websocket upgrade succeeds");

    let client_msg_id = uuid::Uuid::new_v4().to_string();
    ws.send(Message::text(
        json!({
            "type": "send",
            "chat_id": ai,
            "client_msg_id": client_msg_id,
            "body": "are you there?",
        })
        .to_string(),
    ))
    .await
    .expect("sending over the socket");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let frame = loop {
        let message = tokio::time::timeout_at(deadline, ws.next())
            .await
            .expect("timed out waiting for the refusal")
            .expect("socket closed while waiting")
            .expect("socket errored while waiting");
        if let Message::Text(text) = message {
            let value: Value = serde_json::from_str(text.as_str()).expect("frames are JSON");
            if value["type"] == "error" {
                break value;
            }
            assert_ne!(
                value["type"], "ack",
                "the socket must refuse it, not accept it: {value}"
            );
        }
    };
    assert_eq!(frame["code"], "assistant_consent_required", "{frame}");
    assert_eq!(frame["client_msg_id"], client_msg_id, "{frame}");
    assert_eq!(
        rows_in(&ts, &owner, ai).await,
        0,
        "nothing was written over the socket either"
    );
}

/// Agreeing lifts the refusal; agreeing twice keeps the FIRST timestamp,
/// because when somebody agreed is a fact and not a counter.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn agreeing_lifts_the_refusal_and_the_first_yes_is_the_one_kept() {
    let ts = server_with_assistant().await;
    let (owner, _) = ts
        .register_without_assistant_consent("owner", "Olive")
        .await;
    ts.create_family(&owner, "The Smiths").await;
    let ai = ai_chat_id(&ts, &owner).await;

    let response = consent(&ts, &owner, true).await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.expect("JSON");
    let first = body["assistant_consent_at"].clone();
    assert!(
        first.is_string(),
        "the endpoint answers with the stamp: {body}"
    );
    assert_eq!(agreed_at(&ts, &owner).await, first, "and `/me` agrees");

    let sent = ts
        .post_message(
            &owner,
            ai,
            &uuid::Uuid::new_v4().to_string(),
            "are you there?",
        )
        .await;
    assert_eq!(sent.status(), 201, "the refusal is lifted");

    let again: Value = consent(&ts, &owner, true).await.json().await.expect("JSON");
    assert_eq!(
        again["assistant_consent_at"], first,
        "a second yes must not re-date the first — an operator asked when a \
         member consented is not owed the date they last reinstalled"
    );
}

/// Withdrawing puts the refusal back at once and deletes nothing.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn withdrawing_stops_the_assistant_and_keeps_the_conversation() {
    let ts = server_with_assistant().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    ts.create_family(&owner, "The Smiths").await;
    let ai = ai_chat_id(&ts, &owner).await;

    let sent = ts
        .post_message(
            &owner,
            ai,
            &uuid::Uuid::new_v4().to_string(),
            "first question",
        )
        .await;
    assert_eq!(sent.status(), 201);

    let withdrawn: Value = consent(&ts, &owner, false)
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(withdrawn["assistant_consent_at"], Value::Null);
    assert_eq!(agreed_at(&ts, &owner).await, Value::Null);

    let refused = ts
        .post_message(
            &owner,
            ai,
            &uuid::Uuid::new_v4().to_string(),
            "second question",
        )
        .await;
    assert_error(refused, 403, "assistant_consent_required").await;

    // Withdrawal is not deletion: the chat and what was already said stay
    // where they are, and consenting again resumes from there.
    assert_eq!(
        bodies_in(&ts, &owner, ai).await,
        vec!["first question".to_string()],
        "the conversation survives the withdrawal"
    );
    let again: Value = consent(&ts, &owner, false)
        .await
        .json()
        .await
        .expect("JSON");
    assert_eq!(
        again["assistant_consent_at"],
        Value::Null,
        "withdrawing twice is not an error"
    );

    let regranted: Value = consent(&ts, &owner, true).await.json().await.expect("JSON");
    assert!(
        regranted["assistant_consent_at"].is_string(),
        "and one can agree again: {regranted}"
    );
    let resumed = ts
        .post_message(
            &owner,
            ai,
            &uuid::Uuid::new_v4().to_string(),
            "third question",
        )
        .await;
    assert_eq!(resumed.status(), 201);
    assert_eq!(
        bodies_in(&ts, &owner, ai).await,
        vec!["first question".to_string(), "third question".to_string()],
        "resuming continues the same conversation"
    );
}

/// The permission is the MEMBER's. There is no shape of request in which
/// one person grants it for another — not even the owner, who owns the
/// `ai_history` switch over what the family's chat exposes.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_owner_cannot_agree_on_a_members_behalf() {
    let ts = server_with_assistant().await;
    let (owner, _) = ts
        .register_without_assistant_consent("owner", "Olive")
        .await;
    let (member, _) = ts
        .register_without_assistant_consent("junior", "Junior")
        .await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&member, &invite_code, "joined").await;

    assert_eq!(consent(&ts, &owner, true).await.status(), 200);
    assert_eq!(
        agreed_at(&ts, &member).await,
        Value::Null,
        "the owner's yes is the owner's alone"
    );

    let ai = ai_chat_id(&ts, &member).await;
    let refused = ts
        .post_message(&member, ai, &uuid::Uuid::new_v4().to_string(), "hello?")
        .await;
    assert_error(refused, 403, "assistant_consent_required").await;
}

/// `ai_history` carries only the words of members who have consented — the
/// half that is easy to miss. Somebody else's `@ai` must not send a
/// declining member's words to the model, and the model is told nothing
/// about what was withheld.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_history_a_mention_carries_leaves_out_whoever_declined() {
    // No assistant configured: `@ai` is three characters of ordinary text,
    // nothing is spawned to race with, and what a mention WOULD carry is
    // built directly — the same trick the other transcript tests use.
    let ts = spawn_server().await;
    let (owner, _) = ts.register("owner", "Olive").await;
    let (declined, _) = ts
        .register_without_assistant_consent("junior", "Junior")
        .await;
    let (_, invite_code) = ts.create_family(&owner, "The Smiths").await;
    ts.set_open_policy(&owner).await;
    ts.join(&declined, &invite_code, "joined").await;
    let chat = ts.family_chat_id(&owner).await;

    for (token, body) in [
        (&owner, "Dinner at 7?"),
        (&declined, "I would rather eat at 8"),
        (&owner, "Eight it is"),
    ] {
        let response = ts
            .post_message(token, chat, &uuid::Uuid::new_v4().to_string(), body)
            .await;
        assert_eq!(response.status(), 201, "saying {body:?}");
    }

    let mention = ts
        .post_message(
            &owner,
            chat,
            &uuid::Uuid::new_v4().to_string(),
            "@ai when are we eating?",
        )
        .await;
    assert_eq!(mention.status(), 201);
    let mention_id = mention.json::<Value>().await.expect("JSON")["message"]["id"]
        .as_i64()
        .expect("the question has an id");

    let assistant_id = family_connect::handlers_ai::assistant_user_id(&ts.state)
        .await
        .expect("the query runs")
        .expect("migration 0015 inserted the assistant account");
    let note =
        family_connect::handlers_ai::family_chat_history(&ts.state, chat, mention_id, assistant_id)
            .await
            .expect("the query runs")
            .expect("two messages of history");

    assert!(note.contains("Dinner at 7?"), "{note}");
    assert!(note.contains("Eight it is"), "{note}");
    assert!(
        !note.contains("rather eat at 8"),
        "a member who declined does not send their words through somebody \
         else's question: {note}"
    );
    assert!(
        !note.contains("Junior"),
        "nor their name, which is half of what a line carries: {note}"
    );
    // Silently: "three messages were withheld" is itself information about
    // who declined.
    for giveaway in ["withheld", "omitted", "declined", "consent"] {
        assert!(
            !note.to_lowercase().contains(giveaway),
            "the model is told nothing about what is missing ({giveaway}): {note}"
        );
    }

    // And the asker's own words travel whether or not anybody else agreed —
    // which is what makes the filter a per-sender one and not a switch.
    assert!(note.contains("Olive"), "{note}");
}

/// A member's own question is not withheld from their own assistant chat
/// by the same filter: it is theirs, and they agreed.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_askers_own_words_are_never_the_ones_filtered_out() {
    let ts = spawn_server().await;
    let (owner, owner_id) = ts
        .register_without_assistant_consent("owner", "Olive")
        .await;
    ts.create_family(&owner, "The Smiths").await;
    let chat = ts.family_chat_id(&owner).await;

    // Said BEFORE agreeing, then agreed: the filter reads the member's
    // consent now, not the state of the world when they typed.
    for body in ["Dinner at 7?", "@ai when are we eating?"] {
        let response = ts
            .post_message(&owner, chat, &uuid::Uuid::new_v4().to_string(), body)
            .await;
        assert_eq!(response.status(), 201, "saying {body:?}");
    }
    ts.agree_to_the_assistant(owner_id).await;

    let messages: Value = ts
        .get(&owner, &format!("/chats/{chat}/messages"))
        .await
        .json()
        .await
        .expect("JSON");
    // Newest first: the question is the FIRST entry, not the last.
    let mention_id = messages["messages"]
        .as_array()
        .expect("a list")
        .first()
        .and_then(|message| message["id"].as_i64())
        .expect("the question has an id");

    let assistant_id = family_connect::handlers_ai::assistant_user_id(&ts.state)
        .await
        .expect("the query runs")
        .expect("migration 0015 inserted the assistant account");
    let note =
        family_connect::handlers_ai::family_chat_history(&ts.state, chat, mention_id, assistant_id)
            .await
            .expect("the query runs")
            .expect("one message of history");
    assert!(note.contains("Dinner at 7?"), "{note}");
}

/// A server with no assistant has nothing to consent to, and says so the
/// same way it says everything else that is not there — so the endpoint
/// cannot be used to discover whether one is configured.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_server_with_no_assistant_has_no_such_endpoint() {
    let ts = spawn_server().await;
    let (owner, _) = ts
        .register_without_assistant_consent("owner", "Olive")
        .await;
    ts.create_family(&owner, "The Smiths").await;

    for granted in [true, false] {
        assert_error(consent(&ts, &owner, granted).await, 404, "not_found").await;
    }
    assert_eq!(
        agreed_at(&ts, &owner).await,
        Value::Null,
        "and `/me` reports null rather than a date nobody can act on"
    );
}

/// An `[ai]` section that names no processor is an assistant nobody can be
/// asked about, so there is no assistant: a client that cannot say who
/// receives the words has no screen to show, and the server does not
/// pretend otherwise.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_unnamed_processor_is_no_assistant_at_all() {
    let ts = spawn_server_with_config(|cfg| {
        cfg.ai.enabled = true;
        cfg.ai.endpoint = "https://example.invalid".to_string();
        cfg.ai.deployment = "test-deployment".to_string();
        cfg.ai.api_key = "test-key".to_string();
        cfg.ai.title = "Assistant".to_string();
        // …and no `processor`.
    })
    .await;
    let (owner, _) = ts.register("owner", "Olive").await;
    ts.create_family(&owner, "The Smiths").await;

    let chats: Value = ts.get(&owner, "/chats").await.json().await.expect("JSON");
    assert!(
        chats["chats"]
            .as_array()
            .expect("a chat list")
            .iter()
            .all(|entry| entry["chat"]["kind"] != "ai"),
        "no assistant chat is offered: {chats}"
    );
    let family: Value = ts
        .get(&owner, "/families/mine")
        .await
        .json()
        .await
        .expect("JSON");
    assert!(
        family["assistant"].is_null(),
        "and the family names none: {family}"
    );
    assert_error(consent(&ts, &owner, true).await, 404, "not_found").await;
}
