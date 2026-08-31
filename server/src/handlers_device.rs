//! Device registration and the health endpoint.
//!
//! Devices exist so the message fan-out can find push tokens for the
//! devices that are not already being fed by a socket. Delivery is real —
//! APNs, PushKit VoIP and FCM (see `push.rs`); a platform with no
//! credentials configured logs what it would have sent, which is the only
//! remnant of the old `log` driver. Upsert-by-token matters because APNs/FCM
//! tokens migrate between accounts on shared devices — the token, not the
//! row, is the identity.
//!
//! Registration is also where a device learns which SESSION it is, which is
//! what makes push targeting per-device rather than per-user
//! (docs/protocol.md, "Push notifications"). Nothing new is sent to get it:
//! the caller's bearer token already names a session, and the socket that
//! device opens later authenticates with the same token, so the two ends
//! meet on the same row.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde::Deserializer;
use serde_json::json;
use tokio::time::Duration;

use crate::auth::AuthUser;
use crate::error::{ApiError, AppJson, codes};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct RegisterDeviceRequest {
    pub platform: String,
    pub push_token: Option<String>,
    /// The iOS PushKit VoIP token (docs/protocol.md, "Incoming calls"). A
    /// DOUBLE option, the same idiom `PATCH /families/mine` uses for
    /// `language`: the outer is "was the key sent", the inner is the value.
    /// Absent leaves whatever the row holds alone; `null` or `""` clears it;
    /// a string sets it. The two tokens arrive from the OS at different
    /// moments, so a launch that has only one of them must not wipe the
    /// other.
    #[serde(default, deserialize_with = "present_option")]
    pub voip_token: Option<Option<String>>,
}

/// Deserialize a present key into `Some(...)`, so `#[serde(default)]` keeps
/// an ABSENT key as `None`. Same three lines as `handlers_family`.
fn present_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// `POST /devices`
pub async fn register_device(
    auth: AuthUser,
    State(state): State<AppState>,
    AppJson(req): AppJson<RegisterDeviceRequest>,
) -> Result<Response, ApiError> {
    if !matches!(req.platform.as_str(), "ios" | "android" | "macos") {
        return Err(ApiError::validation(
            "platform must be \"ios\", \"macos\" or \"android\"",
        ));
    }
    // An empty token is a client that has none yet; store NULL, not "".
    let push_token = req
        .push_token
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty());

    // The VoIP token's double option: `voip_set` is "was the key present",
    // `voip_value` is the value to write (an empty string clears, exactly
    // as a `null` does). Absent leaves the stored token untouched.
    let voip_set = req.voip_token.is_some();
    let voip_value: Option<String> = req
        .voip_token
        .as_ref()
        .and_then(|inner| inner.as_deref())
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string);

    let device_id: i64 = match push_token {
        Some(token) => {
            // Upsert by token: if the token moved to another account (same
            // physical device, new login), re-home the row.
            //
            // The UPDATE branch is the one that carries the session, and it
            // is the one that matters: a device re-registers its token on
            // every launch, which is what keeps the link pointing at the
            // session that is actually opening sockets rather than at one
            // that was revoked at some previous login.
            // The VoIP token rides the same upsert: set it only when the
            // key was present, so a launch carrying only the push token
            // does not wipe a VoIP token registered a moment before.
            sqlx::query_scalar(
                "INSERT INTO devices (user_id, platform, push_token, session_id, voip_token)
                 VALUES ($1, $2, $3, $4, CASE WHEN $5 THEN $6 ELSE NULL END)
                 ON CONFLICT (push_token) WHERE push_token IS NOT NULL
                 DO UPDATE SET user_id = EXCLUDED.user_id,
                               platform = EXCLUDED.platform,
                               session_id = EXCLUDED.session_id,
                               voip_token = CASE WHEN $5 THEN $6 ELSE devices.voip_token END,
                               updated_at = now()
                 RETURNING id",
            )
            .bind(auth.user_id)
            .bind(&req.platform)
            .bind(token)
            .bind(auth.session_id)
            .bind(voip_set)
            .bind(&voip_value)
            .fetch_one(&state.pool)
            .await?
        }
        None => {
            sqlx::query_scalar(
                "INSERT INTO devices (user_id, platform, session_id, voip_token)
                 VALUES ($1, $2, $3, CASE WHEN $4 THEN $5 ELSE NULL END) RETURNING id",
            )
            .bind(auth.user_id)
            .bind(&req.platform)
            .bind(auth.session_id)
            .bind(voip_set)
            .bind(&voip_value)
            .fetch_one(&state.pool)
            .await?
        }
    };

    Ok((StatusCode::CREATED, Json(json!({"device_id": device_id}))).into_response())
}

/// `DELETE /devices/{id}` — only the caller's own devices; anything else is
/// a 404 so device ids of other users are not probeable.
pub async fn delete_device(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(device_id): Path<i64>,
) -> Result<Response, ApiError> {
    let deleted = sqlx::query("DELETE FROM devices WHERE id = $1 AND user_id = $2")
        .bind(device_id)
        .bind(auth.user_id)
        .execute(&state.pool)
        .await?
        .rows_affected();
    if deleted == 0 {
        return Err(ApiError::not_found(
            codes::DEVICE_NOT_FOUND,
            "no such device",
        ));
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `GET /healthz` — no auth; proves the database answers. The 2 s timeout
/// keeps a wedged pool from turning the health check itself into a hang
/// (load balancers treat slow as dead anyway — better to say so quickly).
pub async fn healthz(State(state): State<AppState>) -> Result<Response, ApiError> {
    let check = sqlx::query("SELECT 1").execute(&state.pool);
    match tokio::time::timeout(Duration::from_secs(2), check).await {
        Ok(Ok(_)) => Ok((StatusCode::OK, Json(json!({"status": "ok"}))).into_response()),
        Ok(Err(err)) => Err(err.into()),
        Err(_elapsed) => Err(ApiError::Internal(anyhow::anyhow!(
            "database health check timed out after 2s"
        ))),
    }
}
