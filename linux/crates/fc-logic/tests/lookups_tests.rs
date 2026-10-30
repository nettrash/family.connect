//! `LookupsTests`, the providers, what is said about them, the answer and its writes, and which
//! answers keep their links out of a preview card (docs/protocol.md, "Looking things up"; issue #72;
//! design decision 7).
//!
//! The rest of the file: the consent screen's extra line is `assistant_consent_lookups.rs`, the
//! footer's links `bubble_body_lookups.rs`, the session's stamp `app_session_lookups.rs`, the
//! owner's switch `family_lookups.rs`, the statistics `settings_lookups.rs`, and an answer still
//! being written `assistant_answers_lookups.rs`.

mod common;

use fc_core::protocol::{AssistantDto, MessageDto};
use fc_core::testing::drive;
use fc_core::text::string_catalog::{EnglishCatalog, StringCatalog};
use fc_logic::app_session::{Gate, SessionState};
use fc_logic::bubble_body::preview_link;
use fc_logic::lookups::{
    self, ConsentAnswer, allowed, answer_for, in_settings, may_preview, names, offered, providers,
    record, settings_footnote, switch_footnote,
};

use common::consent::{PROCESSOR, assistant, clock, strings};
use common::server::{Server, api};

const ME: i64 = 7;
const ASSISTANT_ID: i64 = 1;

const THREE_PROVIDERS: [&str; 3] = ["Brave Search", "Open-Meteo", "Wikipedia"];

const FOOTER: &str = "Snow showers tomorrow, around −2 °C.\n\n\
    Sources: [Tromsø – Wikipedia](https://en.wikipedia.org/wiki/Troms%C3%B8) · [Weather in Tromsø](https://example.org/tromso)\n\
    [Weather data by Open-Meteo.com](https://open-meteo.com/) · Wikipedia, [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/) · Powered by Brave";

fn with_lookups(lookups: Option<&[&str]>, processor: Option<&str>) -> AssistantDto {
    AssistantDto {
        processor: processor.map(str::to_string),
        lookups: lookups.map(strings),
        ..assistant(ASSISTANT_ID, "Assistant")
    }
}

fn message(sender: i64, body: &str, edit_seq: Option<i64>) -> MessageDto {
    MessageDto {
        edit_seq,
        ..MessageDto::new(42, 5, sender, None, body, "2026-10-03T09:30:00Z")
    }
}

fn say() -> EnglishCatalog {
    EnglishCatalog::new()
}

// ---- the providers ----------------------------------------------------------------------------

/// A client that cannot name the providers does not ask: absent, empty and blank all name nobody.
#[test]
fn providers_are_named_or_not_offered_at_all() {
    assert_eq!(None, providers(None));
    assert_eq!(None, providers(Some(&with_lookups(None, Some(PROCESSOR)))));
    assert_eq!(
        None,
        providers(Some(&with_lookups(Some(&[]), Some(PROCESSOR))))
    );
    assert_eq!(
        None,
        providers(Some(&with_lookups(Some(&["", "  "]), Some(PROCESSOR))))
    );
    assert_eq!(
        Some(strings(&["SearXNG"])),
        providers(Some(&with_lookups(
            Some(&[" SearXNG ", ""]),
            Some(PROCESSOR)
        )))
    );
    // The server's order, kept.
    assert_eq!(
        Some(strings(&THREE_PROVIDERS)),
        providers(Some(&with_lookups(Some(&THREE_PROVIDERS), Some(PROCESSOR))))
    );

    assert!(offered(Some(&with_lookups(
        Some(&THREE_PROVIDERS),
        Some(PROCESSOR)
    ))));
    assert!(!offered(Some(&with_lookups(None, Some(PROCESSOR)))));
    // The lookup consent stands on the assistant's, which a server naming no processor cannot ask
    // for.
    assert!(!offered(Some(&with_lookups(Some(&THREE_PROVIDERS), None))));
    assert!(!offered(Some(&with_lookups(
        Some(&THREE_PROVIDERS),
        Some(" ")
    ))));
}

#[test]
fn the_names_are_joined_the_way_the_language_joins_them() {
    let say = say();
    assert_eq!("", names(&[], &say));
    assert_eq!("SearXNG", names(&strings(&["SearXNG"]), &say));
    assert_eq!(
        "Open-Meteo and Wikipedia",
        names(&strings(&["Open-Meteo", "Wikipedia"]), &say)
    );
    assert_eq!(
        "Brave Search, Open-Meteo and Wikipedia",
        names(&strings(&THREE_PROVIDERS), &say)
    );
    // More than the server ever sends still names everybody.
    assert_eq!(
        "A, B, C and D",
        names(&strings(&["A", "B", "C", "D"]), &say)
    );

    let japanese = common::catalogues().for_language("ja");
    assert_eq!(
        japanese.format(
            "%@, %@ and %@",
            &[&"Brave Search", &"Open-Meteo", &"Wikipedia"]
        ),
        names(&strings(&THREE_PROVIDERS), &japanese)
    );
    assert!(!names(&strings(&THREE_PROVIDERS), &japanese).contains(" and "));
    assert_eq!(
        "Open-Meteo und Wikipedia",
        names(
            &strings(&["Open-Meteo", "Wikipedia"]),
            &common::catalogues().for_language("de")
        )
    );
}

// ---- what is said -----------------------------------------------------------------------------

/// A member who already agreed to the assistant reads the lookup line and the way out, and nothing
/// else.
#[test]
fn the_lookup_question_alone_is_the_line_and_the_way_out() {
    let lines = lookups::disclosure(&strings(&["SearXNG"]), false, &say());
    assert_eq!(2, lines.len());
    assert!(lines[0].contains("to SearXNG,"));
    assert_eq!(
        "You can stop this at any time in Settings. What has already been sent cannot be taken back.",
        lines[1]
    );
}

#[test]
fn the_switch_and_settings_name_who_would_receive_a_query() {
    let say = say();
    assert!(
        switch_footnote(&strings(&["Open-Meteo", "Wikipedia"]), &say).starts_with(
            "With this on, the assistant can look things up when a question needs it — the weather, the news, a fact it isn't sure of — in Open-Meteo and Wikipedia."
        )
    );
    assert_eq!(
        "Until you allow lookups, the assistant answers you from what it already knows, and nothing from your questions is sent to SearXNG.",
        settings_footnote(&strings(&["SearXNG"]), false, &say)
    );
    assert!(
        settings_footnote(&strings(&["SearXNG"]), true, &say).starts_with(
            "The assistant may send a short search query or place name it writes from your questions to SearXNG."
        )
    );
    // Translated where the catalogue has it.
    let russian = common::catalogues().for_language("ru");
    let said = settings_footnote(&strings(&["SearXNG"]), false, &russian);
    assert!(said.contains("SearXNG"));
    assert!(!said.contains("Until"));
}

// ---- the answer and its writes ------------------------------------------------------------------

#[test]
fn a_pressed_button_is_the_answer_it_says() {
    assert_eq!(
        ConsentAnswer::AgreeWithLookups,
        answer_for(true, false, true)
    );
    assert_eq!(ConsentAnswer::Agree, answer_for(false, true, true));
    assert_eq!(ConsentAnswer::NotNow, answer_for(false, false, true));
    // Without lookups the one yes is "I Agree", and there is no second button to have pressed.
    assert_eq!(ConsentAnswer::Agree, answer_for(true, false, false));
    assert_eq!(ConsentAnswer::NotNow, answer_for(false, true, false));
}

#[test]
fn not_now_writes_nothing() {
    let server = Server::new();
    let clock = clock();
    let api = api(&server, &clock);
    assert_eq!(
        None,
        drive(&clock, record(&api, ConsentAnswer::NotNow, false))
    );
    assert!(server.asked().is_empty());
}

#[test]
fn agreeing_without_lookups_is_the_assistants_consent_alone() {
    let server = Server::new();
    server.on(
        "/me/assistant-consent",
        r#"{"assistant_consent_at": "2026-10-03T09:30:00Z"}"#,
    );
    let clock = clock();
    let api = api(&server, &clock);
    assert_eq!(
        None,
        drive(&clock, record(&api, ConsentAnswer::Agree, false))
    );
    assert_eq!(vec!["/api/v1/me/assistant-consent"], server.asked());
    assert_eq!(vec!["{\"granted\":true}"], server.bodies());
}

/// The lookup consent may only be granted on top of the assistant's, so it is written second.
#[test]
fn agreeing_with_lookups_writes_both_in_order() {
    let server = Server::new();
    server
        .on(
            "/me/assistant-consent",
            r#"{"assistant_consent_at": "2026-10-03T09:30:00Z"}"#,
        )
        .on(
            "/me/assistant-lookup-consent",
            r#"{"assistant_lookup_consent_at": "2026-10-03T09:30:00Z"}"#,
        );
    let clock = clock();
    let api = api(&server, &clock);
    assert_eq!(
        None,
        drive(&clock, record(&api, ConsentAnswer::AgreeWithLookups, false))
    );
    assert_eq!(
        vec![
            "/api/v1/me/assistant-consent",
            "/api/v1/me/assistant-lookup-consent"
        ],
        server.asked()
    );
    assert_eq!(
        vec!["{\"granted\":true}", "{\"granted\":true}"],
        server.bodies()
    );
}

/// From Settings, a member who already agreed to the assistant is asked — and written — the lookups
/// alone.
#[test]
fn an_assistant_consent_already_held_is_not_written_again() {
    let server = Server::new();
    server.on(
        "/me/assistant-lookup-consent",
        r#"{"assistant_lookup_consent_at": "2026-10-03T09:30:00Z"}"#,
    );
    let clock = clock();
    let api = api(&server, &clock);
    assert_eq!(
        None,
        drive(&clock, record(&api, ConsentAnswer::AgreeWithLookups, true))
    );
    assert_eq!(vec!["/api/v1/me/assistant-lookup-consent"], server.asked());
}

#[test]
fn a_failed_first_write_is_never_followed_by_the_second() {
    let server = Server::new();
    server
        .on_status(
            "/me/assistant-consent",
            500,
            Some(r#"{"error": {"code": "internal", "message": "x"}}"#),
        )
        .on(
            "/me/assistant-lookup-consent",
            r#"{"assistant_lookup_consent_at": "2026-10-03T09:30:00Z"}"#,
        );
    let clock = clock();
    let api = api(&server, &clock);
    let error = drive(&clock, record(&api, ConsentAnswer::AgreeWithLookups, false));
    assert!(error.is_some());
    assert!(
        !server
            .asked()
            .contains(&"/api/v1/me/assistant-lookup-consent".to_string())
    );
}

#[test]
fn a_failed_second_write_is_reported() {
    let server = Server::new();
    server
        .on(
            "/me/assistant-consent",
            r#"{"assistant_consent_at": "2026-10-03T09:30:00Z"}"#,
        )
        .on_status(
            "/me/assistant-lookup-consent",
            404,
            Some(r#"{"error": {"code": "not_found", "message": "no source"}}"#),
        );
    let clock = clock();
    let api = api(&server, &clock);
    let error = drive(&clock, record(&api, ConsentAnswer::AgreeWithLookups, false));
    assert_eq!("not_found", error.expect("refused").code);
}

/// Settings offers the lookup agreement where the server names providers AND the member agreed to
/// the assistant.
#[test]
fn settings_offers_lookups_on_top_of_the_assistant() {
    let state = SessionState {
        assistant: Some(with_lookups(Some(&THREE_PROVIDERS), Some(PROCESSOR))),
        assistant_consent_at: Some("2026-09-19T08:12:04Z".to_string()),
        ..SessionState::new(Gate::Member)
    };
    assert!(in_settings(&state));
    assert!(!in_settings(&SessionState {
        assistant_consent_at: None,
        ..state.clone()
    }));
    assert!(!in_settings(&SessionState {
        assistant: Some(with_lookups(None, Some(PROCESSOR))),
        ..state.clone()
    }));
    assert!(!allowed(&state));
    assert!(allowed(&SessionState {
        assistant_lookup_consent_at: Some("2026-10-03T09:30:00Z".to_string()),
        ..state
    }));
}

// ---- the answer and its links -------------------------------------------------------------------

/// A finished lookup answer draws no card: its links are its sources and credits, and a card would
/// have every device that shows it contact a cited page. Every other message is as it was.
#[test]
fn a_lookup_answer_draws_no_preview_card() {
    let answer = message(ASSISTANT_ID, FOOTER, Some(3));
    assert!(preview_link(FOOTER, false).is_some());
    assert!(!may_preview(
        &answer,
        FOOTER,
        ME,
        false,
        Some(ASSISTANT_ID),
        false
    ));
    // In the member's own assistant chat, where the assistant is whoever is not the reader.
    assert!(!may_preview(
        &message(99, FOOTER, Some(3)),
        FOOTER,
        ME,
        true,
        Some(ASSISTANT_ID),
        false
    ));

    // An answer that looked nothing up keeps its card, as before.
    let plain = "The docs are at https://example.com/docs";
    assert!(may_preview(
        &message(ASSISTANT_ID, plain, Some(3)),
        plain,
        ME,
        false,
        Some(ASSISTANT_ID),
        false
    ));
    // A MEMBER's message that happens to look like a footer is a member's message.
    assert!(may_preview(
        &message(11, FOOTER, Some(3)),
        FOOTER,
        ME,
        false,
        Some(ASSISTANT_ID),
        false
    ));
    assert!(may_preview(
        &message(ME, FOOTER, Some(3)),
        FOOTER,
        ME,
        true,
        Some(ASSISTANT_ID),
        false
    ));
}

/// An answer still being written is the model's raw words — the server filters links out of the
/// finished body, not out of the stream — so it draws no card until the finished row replaces them.
#[test]
fn an_answer_still_being_written_draws_no_card() {
    let raw = "Here: https://evil.example/?q=family+secrets";
    let streaming = message(ASSISTANT_ID, "", None);
    assert!(!may_preview(
        &streaming,
        raw,
        ME,
        false,
        Some(ASSISTANT_ID),
        true
    ));
    let finished = MessageDto {
        body: raw.to_string(),
        edit_seq: Some(4),
        ..streaming
    };
    assert!(may_preview(
        &finished,
        raw,
        ME,
        false,
        Some(ASSISTANT_ID),
        false
    ));
    // A member's message is never "being written" by the assistant.
    assert!(may_preview(
        &message(11, raw, Some(3)),
        raw,
        ME,
        false,
        Some(ASSISTANT_ID),
        true
    ));
}
