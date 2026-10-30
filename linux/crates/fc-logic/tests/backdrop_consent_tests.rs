//! `BackdropConsentTests`: "Draw a backdrop" is asked about exactly as a `/draw` is
//! (docs/protocol.md, "Consenting to the assistant", amended 2026-09-30): asked first when `/me`
//! says no, asked after when the server says `assistant_consent_required`, and drawn only on a yes
//! the server kept.
//!
//! `AnUnansweredAgreeFirstIsThePlainFailure`, whose subject is `NoteSheetText.BackdropFailure`
//! (`note_draft`), is `note_draft_backdrop_consent.rs`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use fc_core::protocol::{ApiError, AttachmentDto, error_codes};
use fc_logic::backdrop_consent::{self, Outcome};
use futures::executor::block_on;

const PROCESSOR: &str = "Microsoft — Azure OpenAI";
const AGREED: &str = "2026-09-19T08:12:04Z";

fn not_agreed() -> ApiError {
    ApiError::new(error_codes::ASSISTANT_CONSENT_REQUIRED, "agree first", 403)
}

fn picture() -> AttachmentDto {
    AttachmentDto {
        width: Some(1024),
        height: Some(512),
        ..AttachmentDto::new(77, "photo")
    }
}

/// What happened, in order: "ask" for the question, "draw" for the request.
struct Script {
    asks_first: bool,
    answers: Vec<bool>,
    draws: Vec<Result<AttachmentDto, ApiError>>,
    steps: Rc<RefCell<Vec<&'static str>>>,
}

impl Script {
    fn new(
        asks_first: bool,
        answers: &[bool],
        draws: Vec<Result<AttachmentDto, ApiError>>,
    ) -> Self {
        Script {
            asks_first,
            answers: answers.to_vec(),
            draws,
            steps: Rc::default(),
        }
    }

    fn run(&self) -> Outcome {
        let asked = Cell::new(0);
        let drawn = Cell::new(0);
        block_on(backdrop_consent::run(
            || self.asks_first,
            || {
                self.steps.borrow_mut().push("ask");
                let answer = self.answers[asked.get()];
                asked.set(asked.get() + 1);
                async move { answer }
            },
            || {
                self.steps.borrow_mut().push("draw");
                let answer = self.draws[drawn.get()].clone();
                drawn.set(drawn.get() + 1);
                async move { answer }
            },
        ))
    }

    fn steps(&self) -> Vec<&'static str> {
        self.steps.borrow().clone()
    }
}

#[test]
fn asked_first_only_where_somebody_is_named_and_nobody_has_agreed() {
    assert!(backdrop_consent::asks_first(Some(PROCESSOR), None));
    assert!(backdrop_consent::asks_first(Some(PROCESSOR), Some("  ")));
    assert!(!backdrop_consent::asks_first(Some(PROCESSOR), Some(AGREED)));
    // Nobody named: nothing can be asked, and the button is not offered there at all
    // (picture_hint::offers_backdrop).
    assert!(!backdrop_consent::asks_first(None, None));
    assert!(!backdrop_consent::asks_first(Some(""), None));
}

#[test]
fn only_the_servers_agree_first_is_answered_with_the_question() {
    assert!(backdrop_consent::asks_after(Some(&not_agreed()), false));
    // A yes went in and the server still refused: said as a failure, not asked again in a loop.
    assert!(!backdrop_consent::asks_after(Some(&not_agreed()), true));
    assert!(!backdrop_consent::asks_after(
        Some(&ApiError::new(error_codes::PICTURE_REFUSED, "x", 400)),
        false
    ));
    assert!(!backdrop_consent::asks_after(
        Some(&ApiError::new(error_codes::PICTURES_UNAVAILABLE, "x", 403)),
        false
    ));
    assert!(!backdrop_consent::asks_after(
        Some(&ApiError::transport("x")),
        false
    ));
    assert!(!backdrop_consent::asks_after(None, false));
}

/// Agreed already: drawn at once, nothing asked.
#[test]
fn an_author_who_agreed_is_not_asked() {
    let script = Script::new(false, &[], vec![Ok(picture())]);
    let outcome = script.run();
    assert_eq!(vec!["draw"], script.steps());
    assert_eq!(77, outcome.drawn.expect("drawn").id);
    assert_eq!(None, outcome.error);
    assert!(!outcome.declined);
}

/// NOTHING REACHES THE MODEL UNASKED: the question first, and the title sent only on a yes.
#[test]
fn not_yet_agreed_is_asked_before_anything_is_sent() {
    let script = Script::new(true, &[true], vec![Ok(picture())]);
    let outcome = script.run();
    assert_eq!(vec!["ask", "draw"], script.steps());
    assert_eq!(77, outcome.drawn.expect("drawn").id);
}

/// Not Now sends nothing, and there is nothing to be told: no request, no failure line.
#[test]
fn not_now_sends_nothing_and_says_nothing() {
    let script = Script::new(true, &[false], vec![]);
    let outcome = script.run();
    assert_eq!(vec!["ask"], script.steps());
    assert!(outcome.declined);
    assert_eq!(None, outcome.drawn);
    assert_eq!(None, outcome.error);
}

/// The server says "agree first" to a member this device thought had agreed — withdrawn somewhere
/// else: the question, then the drawing again on a yes.
#[test]
fn the_servers_agree_first_is_asked_and_then_drawn_again() {
    let script = Script::new(false, &[true], vec![Err(not_agreed()), Ok(picture())]);
    let outcome = script.run();
    assert_eq!(vec!["draw", "ask", "draw"], script.steps());
    assert_eq!(77, outcome.drawn.expect("drawn").id);
    assert_eq!(None, outcome.error);
}

#[test]
fn the_servers_agree_first_answered_not_now_draws_nothing_more() {
    let script = Script::new(false, &[false], vec![Err(not_agreed())]);
    let outcome = script.run();
    assert_eq!(vec!["draw", "ask"], script.steps());
    assert!(outcome.declined);
    assert_eq!(None, outcome.error);
}

/// Asked once per click, whichever way: a refusal after a yes is the failure it is, never a loop.
#[test]
fn asked_at_most_once_per_click() {
    let first = Script::new(true, &[true], vec![Err(not_agreed())]);
    let outcome = first.run();
    assert_eq!(vec!["ask", "draw"], first.steps());
    assert_eq!(
        error_codes::ASSISTANT_CONSENT_REQUIRED,
        outcome.error.expect("refused").code
    );
    assert!(!outcome.declined);

    let after = Script::new(false, &[true], vec![Err(not_agreed()), Err(not_agreed())]);
    let outcome = after.run();
    assert_eq!(vec!["draw", "ask", "draw"], after.steps());
    assert_eq!(
        error_codes::ASSISTANT_CONSENT_REQUIRED,
        outcome.error.expect("refused").code
    );
}

/// Every other failure is said as it always was, and not asked about.
#[test]
fn other_failures_are_not_asked_about() {
    let refused = ApiError::new(error_codes::PICTURE_REFUSED, "x", 400);
    let script = Script::new(false, &[], vec![Err(refused.clone())]);
    let outcome = script.run();
    assert_eq!(vec!["draw"], script.steps());
    assert_eq!(Some(refused), outcome.error);
    assert!(!outcome.declined);
}
