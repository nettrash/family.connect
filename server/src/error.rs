//! Unified API error type and the canonical error-code vocabulary.
//!
//! Every non-2xx response carries `{"error":{"code","message"}}` per
//! protocol.md; funneling all failures through one enum guarantees no
//! handler can accidentally invent a different shape. `Internal` is special:
//! the underlying error is logged server-side with full detail but the
//! response body stays generic — internals (SQL, hostnames, driver messages)
//! must never leak to clients.

use axum::Json;
use axum::extract::{FromRequest, Request};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;
use serde_json::json;
use tracing::error;

/// Canonical machine-readable error codes (protocol.md "Error shape").
/// Handlers reference these constants instead of string literals so a typo
/// becomes a compile error.
pub mod codes {
    pub const UNAUTHORIZED: &str = "unauthorized";
    pub const INVALID_CREDENTIALS: &str = "invalid_credentials";
    pub const USERNAME_TAKEN: &str = "username_taken";
    pub const VALIDATION: &str = "validation";
    pub const ALREADY_IN_FAMILY: &str = "already_in_family";
    pub const FAMILY_REGISTRATION_DISABLED: &str = "family_registration_disabled";
    pub const FAMILY_FULL: &str = "family_full";
    pub const NOT_IN_FAMILY: &str = "not_in_family";
    pub const NOT_FAMILY_OWNER: &str = "not_family_owner";
    pub const INVALID_INVITE_CODE: &str = "invalid_invite_code";
    pub const JOIN_REQUEST_PENDING: &str = "join_request_pending";
    pub const JOIN_REQUEST_NOT_PENDING: &str = "join_request_not_pending";
    pub const USER_ALREADY_IN_FAMILY: &str = "user_already_in_family";
    /// RETIRED: no endpoint raises this any more. An owner who leaves now
    /// hands the family on rather than being refused (protocol.md,
    /// `POST /families/leave`). The constant stays because clients that
    /// predate the hand-off still branch on it, and a code deleted here is
    /// a code somebody deletes from a client still talking to an old
    /// server.
    pub const OWNER_CANNOT_LEAVE: &str = "owner_cannot_leave";
    pub const CANNOT_REMOVE_OWNER: &str = "cannot_remove_owner";
    pub const CANNOT_DM_SELF: &str = "cannot_dm_self";
    pub const CANNOT_BLOCK_SELF: &str = "cannot_block_self";
    /// Raised only ever TOWARDS the blocker — never towards the person they
    /// blocked, who must not be able to tell (protocol.md, "Blocking a
    /// member").
    pub const BLOCKED: &str = "blocked";
    pub const CANNOT_REPORT_SELF: &str = "cannot_report_self";
    pub const REPORT_NOT_PENDING: &str = "report_not_pending";
    pub const NOT_SAME_FAMILY: &str = "not_same_family";
    pub const USER_NOT_FOUND: &str = "user_not_found";
    pub const CHAT_NOT_FOUND: &str = "chat_not_found";
    pub const NOT_CHAT_MEMBER: &str = "not_chat_member";
    pub const MESSAGE_EMPTY: &str = "message_empty";
    pub const MESSAGE_TOO_LONG: &str = "message_too_long";
    pub const MESSAGE_NOT_FOUND: &str = "message_not_found";
    pub const NOT_MESSAGE_AUTHOR: &str = "not_message_author";
    pub const INVALID_POLL: &str = "invalid_poll";
    pub const POLL_CLOSED: &str = "poll_closed";
    pub const ATTACHMENT_TOO_LARGE: &str = "attachment_too_large";
    pub const INVALID_ATTACHMENT: &str = "invalid_attachment";
    pub const ATTACHMENT_NOT_FOUND: &str = "attachment_not_found";
    pub const ATTACHMENT_ALREADY_USED: &str = "attachment_already_used";
    /// The disk is nearly full. Says nothing about the request — the same
    /// upload succeeds unchanged once the operator frees something.
    pub const STORAGE_FULL: &str = "storage_full";
    pub const NOTE_NOT_FOUND: &str = "note_not_found";
    pub const NOT_NOTE_AUTHOR: &str = "not_note_author";
    pub const INVALID_NOTE_COLOR: &str = "invalid_note_color";
    pub const INVALID_NOTE_SIZE: &str = "invalid_note_size";
    pub const INVALID_LANGUAGE: &str = "invalid_language";
    pub const BOARD_FULL: &str = "board_full";
    pub const INVALID_EMOJI: &str = "invalid_emoji";
    pub const INVALID_PAGINATION: &str = "invalid_pagination";
    pub const DEVICE_NOT_FOUND: &str = "device_not_found";
    pub const CALLS_DISABLED: &str = "calls_disabled";
    pub const VIDEO_CALLS_DISABLED: &str = "video_calls_disabled";
    pub const INVALID_CALL: &str = "invalid_call";
    pub const CALL_NOT_FOUND: &str = "call_not_found";
    pub const CALL_BUSY: &str = "call_busy";
    pub const PEER_BUSY: &str = "peer_busy";
    pub const PEER_UNREACHABLE: &str = "peer_unreachable";
    pub const AVATAR_TOO_LARGE: &str = "avatar_too_large";
    pub const INVALID_IMAGE: &str = "invalid_image";
    pub const INTERNAL: &str = "internal";
}

/// Every way a request can fail, keyed by HTTP status class.
#[derive(Debug)]
pub enum ApiError {
    /// 400 — malformed or invalid input.
    BadRequest { code: &'static str, message: String },
    /// 401 — missing/expired session or bad credentials. Carries a code
    /// because the protocol distinguishes `unauthorized` from
    /// `invalid_credentials` (both 401).
    Unauthorized { code: &'static str, message: String },
    /// 403 — authenticated but not allowed.
    Forbidden { code: &'static str, message: String },
    /// 404 — the referenced thing does not exist (for this caller).
    NotFound { code: &'static str, message: String },
    /// 409 — the request conflicts with current state.
    Conflict { code: &'static str, message: String },
    /// 413 — the body is larger than the route accepts (profile pictures).
    PayloadTooLarge { code: &'static str, message: String },
    /// 415 — the body's content type is not one this route stores.
    UnsupportedMediaType { code: &'static str, message: String },
    /// 507 — the server has no room. The one refusal that is about the
    /// SERVER rather than the request, which is why it is not a 4xx: the
    /// caller did nothing wrong and retrying later is the right response.
    InsufficientStorage { code: &'static str, message: String },
    /// 500 — anything unexpected. Logged in full, reported generically.
    Internal(anyhow::Error),
}

impl ApiError {
    pub fn bad_request(code: &'static str, message: impl Into<String>) -> Self {
        Self::BadRequest {
            code,
            message: message.into(),
        }
    }

    /// Shorthand for the generic input-validation failure.
    pub fn validation(message: impl Into<String>) -> Self {
        Self::bad_request(codes::VALIDATION, message)
    }

    pub fn unauthorized() -> Self {
        Self::Unauthorized {
            code: codes::UNAUTHORIZED,
            message: "authentication required".to_string(),
        }
    }

    pub fn invalid_credentials() -> Self {
        Self::Unauthorized {
            code: codes::INVALID_CREDENTIALS,
            message: "unknown username or wrong password".to_string(),
        }
    }

    pub fn forbidden(code: &'static str, message: impl Into<String>) -> Self {
        Self::Forbidden {
            code,
            message: message.into(),
        }
    }

    pub fn not_found(code: &'static str, message: impl Into<String>) -> Self {
        Self::NotFound {
            code,
            message: message.into(),
        }
    }

    pub fn conflict(code: &'static str, message: impl Into<String>) -> Self {
        Self::Conflict {
            code,
            message: message.into(),
        }
    }

    pub fn payload_too_large(code: &'static str, message: impl Into<String>) -> Self {
        Self::PayloadTooLarge {
            code,
            message: message.into(),
        }
    }

    pub fn unsupported_media_type(code: &'static str, message: impl Into<String>) -> Self {
        Self::UnsupportedMediaType {
            code,
            message: message.into(),
        }
    }

    /// 507 — out of disk. Built from the free-space floor rather than taken
    /// as free text, so every route refuses in the same words.
    pub fn storage_full() -> Self {
        Self::InsufficientStorage {
            code: codes::STORAGE_FULL,
            message: "the server is out of storage space; try again later".to_string(),
        }
    }

    /// The HTTP status this error maps to.
    pub fn status(&self) -> StatusCode {
        match self {
            Self::BadRequest { .. } => StatusCode::BAD_REQUEST,
            Self::Unauthorized { .. } => StatusCode::UNAUTHORIZED,
            Self::Forbidden { .. } => StatusCode::FORBIDDEN,
            Self::NotFound { .. } => StatusCode::NOT_FOUND,
            Self::Conflict { .. } => StatusCode::CONFLICT,
            Self::PayloadTooLarge { .. } => StatusCode::PAYLOAD_TOO_LARGE,
            Self::UnsupportedMediaType { .. } => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Self::InsufficientStorage { .. } => StatusCode::INSUFFICIENT_STORAGE,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// `(code, message)` for the WebSocket `error` frame. Consumes the error
    /// so `Internal` can be logged here exactly once, mirroring the HTTP
    /// path: full detail to the log, a generic message to the client.
    pub fn into_ws_parts(self) -> (String, String) {
        match self {
            Self::Internal(err) => {
                error!(error = ?err, "internal error on websocket frame");
                (
                    codes::INTERNAL.to_string(),
                    "internal server error".to_string(),
                )
            }
            Self::BadRequest { code, message }
            | Self::Unauthorized { code, message }
            | Self::Forbidden { code, message }
            | Self::NotFound { code, message }
            | Self::Conflict { code, message }
            | Self::PayloadTooLarge { code, message }
            | Self::UnsupportedMediaType { code, message }
            | Self::InsufficientStorage { code, message } => (code.to_string(), message),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status();
        let (code, message) = match self {
            Self::Internal(err) => {
                // The only place internal errors become visible: the log.
                error!(error = ?err, "internal server error");
                (codes::INTERNAL, "internal server error".to_string())
            }
            Self::BadRequest { code, message }
            | Self::Unauthorized { code, message }
            | Self::Forbidden { code, message }
            | Self::NotFound { code, message }
            | Self::Conflict { code, message }
            | Self::PayloadTooLarge { code, message }
            | Self::UnsupportedMediaType { code, message }
            | Self::InsufficientStorage { code, message } => (code, message),
        };
        let body = json!({"error": {"code": code, "message": message}});
        (status, Json(body)).into_response()
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(err: sqlx::Error) -> Self {
        Self::Internal(anyhow::Error::new(err).context("database error"))
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(err: anyhow::Error) -> Self {
        Self::Internal(err)
    }
}

/// JSON body extractor whose rejection is our error shape.
///
/// axum's stock `Json` rejects with plain-text bodies; wrapping it keeps the
/// protocol invariant that *every* non-2xx response is
/// `{"error":{"code","message"}}` — malformed JSON, a bad UUID, or a missing
/// field all surface as a 400 `validation` error.
pub struct AppJson<T>(pub T);

impl<T, S> FromRequest<S> for AppJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(value)) => Ok(AppJson(value)),
            Err(rejection) => Err(ApiError::validation(rejection.body_text())),
        }
    }
}

#[cfg(test)]
mod tests {

    /// `storage_full` is the one refusal in this protocol that is about the
    /// SERVER rather than the request, and the status has to say so: a 4xx
    /// would tell a client its upload was wrong and make it give up, when
    /// the same bytes succeed unchanged once the operator frees a
    /// gigabyte. protocol.md, "Photos, videos, audio, files and locations".
    #[test]
    fn storage_full_is_a_507_and_not_a_client_error() {
        let error = ApiError::storage_full();
        assert_eq!(error.status(), StatusCode::INSUFFICIENT_STORAGE);
        assert_eq!(error.status().as_u16(), 507);
        assert!(
            !error.status().is_client_error(),
            "a 4xx tells the sender they did something wrong; they did not"
        );

        let (code, message) = ApiError::storage_full().into_ws_parts();
        assert_eq!(code, codes::STORAGE_FULL);
        // Not "too large" in any form: shrinking the photo will not help,
        // and a client that says so sends somebody off to re-encode a
        // video for nothing.
        assert!(!message.contains("large"), "misleading message: {message}");
    }
    use super::*;

    async fn body_json(response: Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("reading response body");
        serde_json::from_slice(&bytes).expect("response body must be JSON")
    }

    #[tokio::test]
    async fn a_conflict_serializes_to_the_protocol_error_shape() {
        let response =
            ApiError::conflict(codes::USERNAME_TAKEN, "username is already in use").into_response();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = body_json(response).await;
        assert_eq!(
            body,
            json!({"error": {"code": "username_taken", "message": "username is already in use"}})
        );
    }

    #[tokio::test]
    async fn internal_errors_do_not_leak_details_to_the_client() {
        let secret = "password=hunter2 host=db.internal";
        let response = ApiError::Internal(anyhow::anyhow!("{secret}")).into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = body_json(response).await;
        assert_eq!(body["error"]["code"], "internal");
        assert!(
            !body.to_string().contains("hunter2"),
            "internal detail leaked: {body}"
        );
    }

    #[tokio::test]
    async fn unauthorized_and_invalid_credentials_share_status_but_not_code() {
        assert_eq!(ApiError::unauthorized().status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            ApiError::invalid_credentials().status(),
            StatusCode::UNAUTHORIZED
        );
        let body = body_json(ApiError::invalid_credentials().into_response()).await;
        assert_eq!(body["error"]["code"], "invalid_credentials");
    }

    #[test]
    fn ws_parts_keep_internal_errors_generic() {
        let (code, message) = ApiError::Internal(anyhow::anyhow!("secret")).into_ws_parts();
        assert_eq!(code, "internal");
        assert!(!message.contains("secret"));
    }
}
