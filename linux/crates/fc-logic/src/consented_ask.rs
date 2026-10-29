//! One request that sends something of the member's to the model, asked about the way a `/draw` is
//! (docs/protocol.md, "Consenting to the assistant") — `ConsentedAsk.cs`: ASKED FIRST when `/me`
//! says this member has not agreed, and ASKED AFTER — then sent again, ONCE — when the server
//! answers `assistant_consent_required` anyway, which is an answer withdrawn on another device that
//! this one has not heard about. A second refusal after a yes is the failure it is, never a loop.
//!
//! The board's "Draw a backdrop" ([`crate::backdrop_consent`]) and a recording's "Show text" (the
//! transcript model) are both this, which is why the order lives in one place and is pinned once.
//!
//! The C#'s request answered a `(Value, Error)` pair; here it answers a `Result` — the pair's
//! "neither" did not exist in any caller, and `ApiResult` reads the same way.

use std::future::Future;

use fc_core::protocol::ApiError;

use crate::backdrop_consent;

/// How one click ended.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome<T> {
    /// The answer, when there is one.
    pub value: Option<T>,
    /// Why there is none, to be said — `None` on an answer, and `None` when the member said Not
    /// Now.
    pub error: Option<ApiError>,
    /// The member was asked and did not agree: nothing was sent, and there is nothing to say.
    pub declined: bool,
}

impl<T> Outcome<T> {
    /// The member said Not Now.
    pub fn declined() -> Self {
        Outcome {
            value: None,
            error: None,
            declined: true,
        }
    }

    /// What the request answered.
    pub fn answered(answer: Result<T, ApiError>) -> Self {
        match answer {
            Ok(value) => Outcome {
                value: Some(value),
                error: None,
                declined: false,
            },
            Err(error) => Outcome::failed(error),
        }
    }

    /// A failure with no answer — what the transcript model makes of a request that could not even
    /// be made (the C#'s `new Outcome<T>(null, error, false)`).
    pub fn failed(error: ApiError) -> Self {
        Outcome {
            value: None,
            error: Some(error),
            declined: false,
        }
    }
}

/// One click, from the question (when it is needed) to the answer.
///
/// - `asks_first`: read when the click lands, from the session as it is then
///   ([`backdrop_consent::asks_first`]).
/// - `ask`: the consent question, and the answer recorded on the server — `true` only on a yes it
///   has kept. Called at most once.
/// - `request`: the request itself. Called at most twice: once, and once more after a yes the
///   server's own "agree first" asked for.
///
/// Cancelling is dropping the future: whatever was in flight goes with it.
pub async fn run<T, AskFirst, Ask, Asked, Request, Requested>(
    asks_first: AskFirst,
    mut ask: Ask,
    mut request: Request,
) -> Outcome<T>
where
    AskFirst: FnOnce() -> bool,
    Ask: FnMut() -> Asked,
    Asked: Future<Output = bool>,
    Request: FnMut() -> Requested,
    Requested: Future<Output = Result<T, ApiError>>,
{
    let mut asked = false;
    if asks_first() {
        asked = true;
        if !ask().await {
            return Outcome::declined();
        }
    }
    let mut answer = request().await;
    if backdrop_consent::asks_after(answer.as_ref().err(), asked) {
        if !ask().await {
            return Outcome::declined();
        }
        answer = request().await;
    }
    Outcome::answered(answer)
}
