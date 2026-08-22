//! Device registration (the v1 push hook) and the health endpoint.
//!
//! Devices exist so the message fan-out can find push tokens for offline
//! members; nothing is delivered in v1 (the `log` driver). Upsert-by-token
//! matters because APNs/FCM tokens migrate between accounts on shared
//! devices — the token, not the row, is the identity.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;
use tokio::time::Duration;

use crate::auth::AuthUser;
use crate::error::{ApiError, AppJson, codes};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct RegisterDeviceRequest {
    pub platform: String,
    pub push_token: Option<String>,
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

    let device_id: i64 = match push_token {
        Some(token) => {
            // Upsert by token: if the token moved to another account (same
            // physical device, new login), re-home the row.
            sqlx::query_scalar(
                "INSERT INTO devices (user_id, platform, push_token)
                 VALUES ($1, $2, $3)
                 ON CONFLICT (push_token) WHERE push_token IS NOT NULL
                 DO UPDATE SET user_id = EXCLUDED.user_id,
                               platform = EXCLUDED.platform,
                               updated_at = now()
                 RETURNING id",
            )
            .bind(auth.user_id)
            .bind(&req.platform)
            .bind(token)
            .fetch_one(&state.pool)
            .await?
        }
        None => {
            sqlx::query_scalar(
                "INSERT INTO devices (user_id, platform) VALUES ($1, $2) RETURNING id",
            )
            .bind(auth.user_id)
            .bind(&req.platform)
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
