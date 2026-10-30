//! `LiveMarksTests`, the assistant's half: an answer mid-stream, and one that stopped.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use fc_core::protocol::{AiErrorReason, MessageDto, ServerFrame};
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::assistant_answers::AssistantAnswers;

const AT: &str = "2026-09-16T10:00:00Z";

fn message(id: i64, body: &str) -> MessageDto {
    MessageDto::new(id, 42, 99, None, body, AT)
}

fn with_edit(message: &MessageDto, edit_seq: i64) -> MessageDto {
    MessageDto {
        edit_seq: Some(edit_seq),
        ..message.clone()
    }
}

fn with_id(message: &MessageDto, id: i64) -> MessageDto {
    MessageDto {
        id,
        ..message.clone()
    }
}

/// Streamed text is drawn after the held body until the answer is finished; late and unknown
/// deltas change nothing.
#[test]
fn streamed_text_follows_the_body_until_the_answer_is_finished() {
    let answers = AssistantAnswers::new();
    let changed = Rc::new(RefCell::new(Vec::new()));
    let heard = changed.clone();
    answers
        .changed
        .subscribe(move |chat_id| heard.borrow_mut().push(*chat_id));
    let writing = message(5, "");

    answers.delta(42, 5, "Sure — ", Some(&writing));
    answers.delta(42, 5, "the park.", Some(&writing));
    assert_eq!(answers.body_of(&writing), "Sure — the park.");
    assert_eq!(*changed.borrow(), vec![42, 42]);

    let before = answers.version();
    answers.delta(42, 5, "late", Some(&with_edit(&writing, 3)));
    answers.delta(42, 6, "stranger", None);
    answers.delta(42, 5, "", Some(&writing));
    assert_eq!(answers.version(), before);
    assert_eq!(answers.body_of(&writing), "Sure — the park.");

    // The finished row is drawn as itself, and forgets what streamed.
    let finished = MessageDto {
        body: "Sure — the park at noon.".to_string(),
        edit_seq: Some(3),
        ..writing.clone()
    };
    assert_eq!(answers.body_of(&finished), "Sure — the park at noon.");
    answers.finished(&finished);
    assert_eq!(answers.body_of(&writing), "");
    assert!(answers.version() > before);
}

#[test]
fn an_answer_that_stopped_says_so_until_it_is_finished() {
    let answers = AssistantAnswers::new();
    let writing = message(5, "Half");

    assert!(!answers.failed(&writing));
    answers.stopped(42, 5, None);
    assert!(answers.failed(&writing));
    assert!(!answers.failed(&with_id(&writing, 6)));
    // However the finished row reached the cache — a frame, or the edits catch-up — it is not a
    // failure.
    assert!(!answers.failed(&with_edit(&writing, 4)));

    answers.finished(&with_edit(&writing, 4));
    assert!(!answers.failed(&writing));

    // An answer that streamed AND stopped, once finished, forgets both — not only the first it
    // looked at.
    answers.delta(42, 5, " more", Some(&writing));
    // Streamed text follows the words the row already holds.
    assert_eq!(answers.body_of(&writing), "Half more");
    answers.stopped(42, 5, None);
    answers.finished(&with_edit(&writing, 5));
    assert_eq!(answers.body_of(&writing), "Half");
    assert!(!answers.failed(&writing));

    answers.stopped(42, 5, None);
    answers.delta(42, 5, " again", Some(&writing));
    let version = answers.version();
    answers.clear();
    assert!(!answers.failed(&writing));
    assert_eq!(answers.body_of(&writing), "Half");
    assert!(answers.version() > version);
    // Finishing something never streamed changes nothing.
    let version = answers.version();
    answers.finished(&with_id(&writing, 77));
    assert_eq!(answers.version(), version);
}

/// A failed row REMEMBERS WHICH SENTENCE it says (docs/protocol.md, `ai_error`'s `reason`): a
/// refusal by the provider's own filter is not worth asking again in the same words, and every
/// other failure — a frame with no reason, or one this client could not read — says "ask again",
/// as it always did.
#[test]
fn a_failed_answer_remembers_which_sentence_it_says() {
    let say = common::english();
    let answers = AssistantAnswers::new();
    let refused = message(5, "");
    let broken = message(6, "Half");
    const REFUSAL: &str = "The assistant's provider refused that. Try putting it another way.";
    const ASK_AGAIN: &str = "Couldn't answer that. Ask again.";

    assert_eq!(answers.failure_sentence(&refused, &say), None);
    answers.stopped(42, 5, Some(AiErrorReason::Refused));
    answers.stopped(42, 6, None);
    assert!(answers.failed(&refused));
    assert_eq!(
        answers.failure_sentence(&refused, &say).as_deref(),
        Some(REFUSAL)
    );
    assert_eq!(
        answers.failure_sentence(&broken, &say).as_deref(),
        Some(ASK_AGAIN)
    );
    // Asked again — as every redraw does — it says the same thing.
    assert_eq!(
        answers.failure_sentence(&refused, &say).as_deref(),
        Some(REFUSAL)
    );

    // The reason is the frame's own, through the router: an unknown one arrives as none at all.
    let Some(ServerFrame::AiError {
        chat_id,
        message_id,
        reason,
    }) = ServerFrame::parse(
        r#"{"type": "ai_error", "chat_id": 42, "message_id": 7, "reason": "tired"}"#,
    )
    else {
        panic!("an ai_error frame");
    };
    answers.stopped(chat_id, message_id, reason);
    assert_eq!(
        answers
            .failure_sentence(&with_id(&broken, 7), &say)
            .as_deref(),
        Some(ASK_AGAIN)
    );

    // A finished answer says nothing, and forgets the reason with the failure.
    assert_eq!(
        answers.failure_sentence(&with_edit(&refused, 2), &say),
        None
    );
    answers.finished(&with_edit(&refused, 2));
    assert_eq!(answers.failure_sentence(&refused, &say), None);
    answers.stopped(42, 5, None);
    assert_eq!(
        answers.failure_sentence(&refused, &say).as_deref(),
        Some(ASK_AGAIN)
    );

    answers.clear();
    assert_eq!(answers.failure_sentence(&broken, &say), None);

    // Translated, not left in English.
    let german = common::catalogues().for_language("de");
    assert_eq!(
        AssistantAnswers::sentence(Some(AiErrorReason::Refused), &german),
        german.get(REFUSAL)
    );
    assert_ne!(
        AssistantAnswers::sentence(Some(AiErrorReason::Refused), &german),
        REFUSAL
    );
    assert_eq!(
        AssistantAnswers::sentence(None, &german),
        german.get(ASK_AGAIN)
    );
}
