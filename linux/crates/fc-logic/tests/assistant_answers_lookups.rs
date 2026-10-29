//! `LookupsTests`, the one test whose subject is `AssistantAnswers`: an answer still being written
//! is the model's raw words, until the finished row lands.
//!
//! The rest of `LookupsTests` is other modules': `lookups_tests.rs`, `assistant_consent_lookups.rs`,
//! `bubble_body_lookups.rs`, `family_lookups.rs`, `app_session_lookups.rs`, and the statistics' two
//! tests in `settings_lookups.rs`.

use fc_core::protocol::MessageDto;
use fc_logic::assistant_answers::AssistantAnswers;

const ASSISTANT_ID: i64 = 1;

const FOOTER: &str = "Snow showers tomorrow, around −2 °C.\n\n\
    Sources: [Tromsø – Wikipedia](https://en.wikipedia.org/wiki/Troms%C3%B8) · [Weather in Tromsø](https://example.org/tromso)\n\
    [Weather data by Open-Meteo.com](https://open-meteo.com/) · Wikipedia, [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/) · Powered by Brave";

/// `Message(sender, body, chatId: 5, editSeq)`.
fn message(sender: i64, body: &str, edit_seq: Option<i64>) -> MessageDto {
    MessageDto {
        edit_seq,
        ..MessageDto::new(42, 5, sender, None, body, "2026-10-03T09:30:00Z")
    }
}

#[test]
fn writing_lasts_until_the_finished_row_lands() {
    let answers = AssistantAnswers::new();
    let held = message(ASSISTANT_ID, "", None);
    assert!(!answers.is_writing(&held));
    answers.delta(held.chat_id, held.id, "Here: https://", Some(&held));
    assert!(answers.is_writing(&held));
    // Stopped part-way: its words are never replaced, so it stays the raw stream.
    answers.stopped(held.chat_id, held.id, None);
    assert!(answers.is_writing(&held));
    let finished = MessageDto {
        body: FOOTER.to_string(),
        edit_seq: Some(9),
        ..held.clone()
    };
    answers.finished(&finished);
    assert!(!answers.is_writing(&finished));
    assert!(!answers.is_writing(&held));
}
