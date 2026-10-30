//! What a call answered, and where the session token lives — a port of
//! `win/src/FamilyConnect.Core/Protocol/ApiResult.cs`.

use std::cell::RefCell;

use super::api_error::ApiError;

/// What a call answered: a value, or the reason there is none.
///
/// A result rather than a panic or an early return, because on this wire a refusal is an
/// ORDINARY answer that the caller has to read — [`ApiError::transient`] decides whether to try
/// again, and an error raised for a 429 would lose that distinction on the way up
/// (docs/protocol.md, "Error shape").
///
/// `win/`'s `ApiResult<T>` held a nullable value beside a nullable error, so its readers wrote
/// `answer.Error ?? ApiError.Transport("no answer")` for the case of neither; here it is a
/// `Result`, and that case does not exist.
#[derive(Debug, Clone, PartialEq)]
pub struct ApiResult<T> {
    pub outcome: Result<T, ApiError>,
    /// The HTTP status of a SUCCESS, where the transport knew one (0 otherwise). Nearly every
    /// caller ignores it; the few endpoints whose protocol row gives two successes two meanings —
    /// the pack's `201` "added" against its `200` "already there" — read it rather than guess.
    pub status: u16,
}

impl<T> ApiResult<T> {
    pub fn success(value: T, status: u16) -> Self {
        ApiResult {
            outcome: Ok(value),
            status,
        }
    }

    pub fn failure(error: ApiError) -> Self {
        ApiResult {
            outcome: Err(error),
            status: 0,
        }
    }

    pub fn ok(&self) -> bool {
        self.outcome.is_ok()
    }

    /// The value, when the call succeeded.
    pub fn value(&self) -> Option<&T> {
        self.outcome.as_ref().ok()
    }

    /// The failure, when it did not.
    pub fn error(&self) -> Option<&ApiError> {
        self.outcome.as_ref().err()
    }

    /// The value, or `fallback` when the call failed.
    pub fn or(self, fallback: T) -> T {
        self.outcome.unwrap_or(fallback)
    }

    pub fn into_result(self) -> Result<T, ApiError> {
        self.outcome
    }
}

/// An answer with no body — a `204`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Nothing;

/// Where the session token lives. Core holds the TRAIT and never the secret: on the desktop the
/// token belongs in the system's secret store, which is a platform API, and Core builds and
/// tests anywhere. Single-threaded, like everything here: the setter takes `&self`.
pub trait TokenStore {
    /// The token, or `None` when nobody is signed in.
    fn token(&self) -> Option<String>;

    fn set_token(&self, token: Option<String>);
}

/// For tests and for the moment before a real store is wired up.
#[derive(Debug, Default)]
pub struct MemoryTokenStore(RefCell<Option<String>>);

impl MemoryTokenStore {
    pub fn new(token: Option<&str>) -> Self {
        MemoryTokenStore(RefCell::new(token.map(str::to_string)))
    }
}

impl TokenStore for MemoryTokenStore {
    fn token(&self) -> Option<String> {
        self.0.borrow().clone()
    }

    fn set_token(&self, token: Option<String>) {
        *self.0.borrow_mut() = token;
    }
}
