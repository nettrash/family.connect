//! "Draw a backdrop" is asked about exactly as a `/draw` is (docs/protocol.md, "Consenting to the
//! assistant", amended 2026-09-30) — `BackdropConsent.cs`: the event's title is the author's own
//! words going to the model, so the server answers `assistant_consent_required` (403) and sends
//! nothing until the author has agreed.
//!
//! ASKED FIRST when `/me` says this member has not agreed, the way the Send button asks: the
//! question, and the drawing only on a yes the server has recorded. ASKED AFTER when the server says
//! so anyway — an answer withdrawn on another device, which this one's `/me` has not heard about —
//! and then drawn again, ONCE: a second refusal after a yes is shown as the failure it is, never
//! asked about in a loop. The order is [`crate::consented_ask`]'s.

use std::future::Future;

use fc_core::protocol::{ApiError, AttachmentDto, error_codes};

use crate::consented_ask;

/// How a click on "Draw a backdrop" ended.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    /// The new picture, when one was drawn.
    pub drawn: Option<AttachmentDto>,
    /// Why nothing was drawn, to be said — `None` when drawn, and `None` when the member said Not
    /// Now.
    pub error: Option<ApiError>,
    /// The member was asked and did not agree: nothing was sent, and there is nothing to say.
    pub declined: bool,
}

/// Must the author be asked before the title goes to the model? Only where there is somebody named
/// to agree to (`fc_text::assistant_consent::is_required_for_backdrop`). `agreed_at` is `/me`'s
/// `assistant_consent_at`; a blank one is no answer.
pub fn asks_first(processor: Option<&str>, agreed_at: Option<&str>) -> bool {
    let agreed = agreed_at.is_some_and(|at| !at.trim().is_empty());
    fc_text::assistant_consent::is_required_for_backdrop(processor, agreed)
}

/// Does this failure call for the question — the server's own "agree first" — rather than a
/// sentence? Not when the question was already asked for this click: the yes went in and the server
/// still refused, and asking again would be a loop.
pub fn asks_after(error: Option<&ApiError>, asked_already: bool) -> bool {
    !asked_already
        && error.is_some_and(|error| error.code == error_codes::ASSISTANT_CONSENT_REQUIRED)
}

/// One click, from the question (when it is needed) to the answer — in [`consented_ask::run`]'s
/// order. `asks_first` is read when the click lands, from the session as it is then
/// ([`asks_first`]); `ask` is the consent question and the answer recorded on the server (`true`
/// only on a yes it has kept); `draw` is the request itself.
pub async fn run<AskFirst, Ask, Asked, Draw, Drawn>(
    asks_first: AskFirst,
    ask: Ask,
    draw: Draw,
) -> Outcome
where
    AskFirst: FnOnce() -> bool,
    Ask: FnMut() -> Asked,
    Asked: Future<Output = bool>,
    Draw: FnMut() -> Drawn,
    Drawn: Future<Output = Result<AttachmentDto, ApiError>>,
{
    let outcome = consented_ask::run(asks_first, ask, draw).await;
    Outcome {
        drawn: outcome.value,
        error: outcome.error,
        declined: outcome.declined,
    }
}
