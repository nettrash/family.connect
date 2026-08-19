//! Auth endpoints: register, login, logout, and `GET /me`.
//!
//! Registration relies on the database (the `users_username_lower_uq` index)
//! for username uniqueness rather than a check-then-insert — the unique
//! violation is mapped to `409 username_taken`, so two concurrent
//! registrations of the same name can never both succeed.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;
use sqlx::Row;
use time::OffsetDateTime;

use crate::auth::{self, AuthUser};
use crate::error::{ApiError, AppJson, codes};
use crate::models::{Family, PendingJoinRequest, User};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub display_name: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

/// `POST /auth/register`
pub async fn register(
    State(state): State<AppState>,
    AppJson(req): AppJson<RegisterRequest>,
) -> Result<Response, ApiError> {
    validate_username(&req.username)?;
    let display_name = validate_display_name(&req.display_name)?;
    if req.password.chars().count() < 8 {
        return Err(ApiError::validation(
            "password must be at least 8 characters",
        ));
    }

    let password_hash = auth::hash_password(req.password).await?;

    let inserted = sqlx::query(
        "INSERT INTO users (username, display_name, password_hash)
         VALUES ($1, $2, $3)
         RETURNING id, username, display_name, created_at",
    )
    .bind(&req.username)
    .bind(&display_name)
    .bind(&password_hash)
    .fetch_one(&state.pool)
    .await;

    let row = match inserted {
        Ok(row) => row,
        Err(sqlx::Error::Database(db_err))
            if db_err.constraint() == Some("users_username_lower_uq") =>
        {
            return Err(ApiError::conflict(
                codes::USERNAME_TAKEN,
                "username is already in use",
            ));
        }
        Err(err) => return Err(err.into()),
    };

    let user = User::from_row(&row);
    let token = auth::create_session(&state.pool, &state.cfg, user.id).await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"token": token, "user": user})),
    )
        .into_response())
}

/// `POST /auth/login`
pub async fn login(
    State(state): State<AppState>,
    AppJson(req): AppJson<LoginRequest>,
) -> Result<Response, ApiError> {
    let row = sqlx::query(
        "SELECT id, username, display_name, password_hash, created_at
         FROM users WHERE lower(username) = lower($1)",
    )
    .bind(&req.username)
    .fetch_optional(&state.pool)
    .await?;

    // Verify even when the user is unknown (dummy hash inside) so timing
    // does not reveal which usernames exist.
    let stored_hash: Option<String> = row.as_ref().map(|r| r.get("password_hash"));
    if !auth::verify_login_password(stored_hash, req.password).await? {
        return Err(ApiError::invalid_credentials());
    }
    let row = row.expect("password verified, so the user row exists");

    let user = User::from_row(&row);
    let token = auth::create_session(&state.pool, &state.cfg, user.id).await?;
    Ok((StatusCode::OK, Json(json!({"token": token, "user": user}))).into_response())
}

/// `POST /auth/logout` — revokes the calling session and closes its sockets.
pub async fn logout(auth: AuthUser, State(state): State<AppState>) -> Result<Response, ApiError> {
    sqlx::query("DELETE FROM sessions WHERE id = $1")
        .bind(auth.session_id)
        .execute(&state.pool)
        .await?;
    state.registry.close_session(auth.session_id).await;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `GET /me`
pub async fn me(auth: AuthUser, State(state): State<AppState>) -> Result<Response, ApiError> {
    let row = sqlx::query(
        "SELECT u.id, u.username, u.display_name, u.created_at,
                f.id AS family_id, f.name AS family_name, f.join_policy,
                f.created_at AS family_created_at, f.owner_user_id, f.invite_code
         FROM users u
         LEFT JOIN families f ON f.id = u.family_id
         WHERE u.id = $1",
    )
    .bind(auth.user_id)
    .fetch_one(&state.pool)
    .await?;

    let user = User::from_row(&row);
    let family_id: Option<i64> = row.get("family_id");
    let (family, role) = match family_id {
        Some(id) => {
            let owner_user_id: i64 = row.get("owner_user_id");
            let is_owner = owner_user_id == auth.user_id;
            let family = Family {
                id,
                name: row.get("family_name"),
                join_policy: row.get("join_policy"),
                created_at: row.get("family_created_at"),
                // The invite code is owner-only information.
                invite_code: is_owner.then(|| row.get("invite_code")),
            };
            let role = if is_owner { "owner" } else { "member" };
            (Some(family), Some(role))
        }
        None => (None, None),
    };

    // The caller's live join request, if any — a waiting client that sees
    // neither `family` nor `pending_join_request` knows it was rejected.
    let pending = sqlx::query(
        "SELECT jr.family_id, f.name AS family_name, jr.created_at
         FROM join_requests jr
         JOIN families f ON f.id = jr.family_id
         WHERE jr.user_id = $1 AND jr.status = 'pending'
         ORDER BY jr.id DESC
         LIMIT 1",
    )
    .bind(auth.user_id)
    .fetch_optional(&state.pool)
    .await?
    .map(|row| PendingJoinRequest {
        family_id: row.get("family_id"),
        family_name: row.get("family_name"),
        created_at: row.get::<OffsetDateTime, _>("created_at"),
    });

    Ok((
        StatusCode::OK,
        Json(json!({
            "user": user,
            "family": family,
            "role": role,
            "pending_join_request": pending,
        })),
    )
        .into_response())
}

/// Username: 3–32 chars from `[a-zA-Z0-9_.]` (protocol.md).
fn validate_username(username: &str) -> Result<(), ApiError> {
    let len = username.chars().count();
    if !(3..=32).contains(&len) {
        return Err(ApiError::validation("username must be 3-32 characters"));
    }
    if !username
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
    {
        return Err(ApiError::validation(
            "username may only contain letters, digits, '_' and '.'",
        ));
    }
    Ok(())
}

/// Display name: trimmed, 1–64 chars. Returns the trimmed value, which is
/// what gets stored.
fn validate_display_name(display_name: &str) -> Result<String, ApiError> {
    let trimmed = display_name.trim();
    let len = trimmed.chars().count();
    if !(1..=64).contains(&len) {
        return Err(ApiError::validation("display_name must be 1-64 characters"));
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usernames_outside_the_allowed_charset_or_length_are_rejected() {
        assert!(validate_username("anna").is_ok());
        assert!(validate_username("An.na_42").is_ok());
        assert!(validate_username("ab").is_err(), "too short");
        assert!(validate_username(&"a".repeat(33)).is_err(), "too long");
        assert!(validate_username("anna smith").is_err(), "space");
        assert!(validate_username("anna@home").is_err(), "symbol");
    }

    #[test]
    fn display_names_are_trimmed_and_length_checked() {
        assert_eq!(validate_display_name("  Anna  ").expect("valid"), "Anna");
        assert!(validate_display_name("   ").is_err(), "empty after trim");
        assert!(validate_display_name(&"x".repeat(65)).is_err(), "too long");
    }
}
