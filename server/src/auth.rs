//! Authentication: argon2id password hashing, opaque bearer sessions, and
//! the `AuthUser` extractor.
//!
//! Design notes:
//! - Hashing/verification run in `spawn_blocking` — argon2id at default
//!   params costs tens of milliseconds of pure CPU, which would stall the
//!   async worker threads.
//! - Login is timing-equalized: when the username is unknown we still verify
//!   the password against a dummy hash, so "user exists" and "wrong
//!   password" take the same time. The dummy hash is computed once at first
//!   use with `Argon2::default()` (rather than embedding a PHC literal) so
//!   its parameters can never drift from the real ones.
//! - Sessions slide: a request more than `session_touch_interval_mins` after
//!   the previous touch renews `expires_at` via a fire-and-forget task —
//!   the request must not wait on bookkeeping.

use std::sync::OnceLock;

use argon2::Argon2;
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use axum::extract::FromRequestParts;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use sqlx::{PgPool, Row};
use time::{Duration, OffsetDateTime};
use tracing::warn;

use crate::config::Config;
use crate::error::ApiError;
use crate::state::AppState;
use crate::tokens;

/// The authenticated caller, extracted from `Authorization: Bearer <token>`.
pub struct AuthUser {
    pub user_id: i64,
    pub family_id: Option<i64>,
    pub session_id: i64,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer_token(parts).ok_or_else(ApiError::unauthorized)?;
        authenticate(state, &token).await
    }
}

/// Pull the bearer token out of the Authorization header, if any.
fn bearer_token(parts: &Parts) -> Option<String> {
    let value = parts.headers.get(AUTHORIZATION)?.to_str().ok()?;
    value.strip_prefix("Bearer ").map(str::to_string)
}

/// Resolve a bearer token to its session + user, sliding the expiry.
pub async fn authenticate(state: &AppState, token: &str) -> Result<AuthUser, ApiError> {
    let token_hash = tokens::hash_token(token);
    let row = sqlx::query(
        "SELECT s.id AS session_id, s.user_id, s.last_used_at, u.family_id
         FROM sessions s
         JOIN users u ON u.id = s.user_id
         WHERE s.token_hash = $1 AND s.expires_at > now()",
    )
    .bind(&token_hash)
    .fetch_optional(&state.pool)
    .await?;

    let Some(row) = row else {
        return Err(ApiError::unauthorized());
    };

    let session_id: i64 = row.get("session_id");
    let user_id: i64 = row.get("user_id");
    let family_id: Option<i64> = row.get("family_id");
    let last_used_at: OffsetDateTime = row.get("last_used_at");

    let touch_interval = Duration::minutes(state.cfg.auth.session_touch_interval_mins);
    if OffsetDateTime::now_utc() - last_used_at > touch_interval {
        // Fire-and-forget: a lost renewal only means the next request
        // renews instead; not worth adding latency to this one.
        let pool = state.pool.clone();
        let ttl = Duration::days(state.cfg.auth.session_ttl_days);
        tokio::spawn(async move {
            let expires_at = OffsetDateTime::now_utc() + ttl;
            if let Err(err) = sqlx::query(
                "UPDATE sessions SET last_used_at = now(), expires_at = $2 WHERE id = $1",
            )
            .bind(session_id)
            .bind(expires_at)
            .execute(&pool)
            .await
            {
                warn!(error = %err, session_id, "failed to slide session expiry");
            }
        });
    }

    Ok(AuthUser {
        user_id,
        family_id,
        session_id,
    })
}

/// Create a session for `user_id` and return the opaque token — the only
/// moment the token exists in plaintext on the server.
pub async fn create_session(pool: &PgPool, cfg: &Config, user_id: i64) -> Result<String, ApiError> {
    let token = tokens::gen_session_token();
    let token_hash = tokens::hash_token(&token);
    let expires_at = OffsetDateTime::now_utc() + Duration::days(cfg.auth.session_ttl_days);
    sqlx::query("INSERT INTO sessions (user_id, token_hash, expires_at) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(&token_hash)
        .bind(expires_at)
        .execute(pool)
        .await?;
    Ok(token)
}

/// Hash a password with argon2id (PHC string output).
pub async fn hash_password(password: String) -> Result<String, ApiError> {
    tokio::task::spawn_blocking(move || {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|hash| hash.to_string())
            .map_err(|err| anyhow::anyhow!("hashing password: {err}"))
    })
    .await
    .map_err(|err| ApiError::Internal(anyhow::anyhow!("hashing task panicked: {err}")))?
    .map_err(ApiError::Internal)
}

/// Verify a login attempt.
///
/// `stored_hash` is `None` when the username does not exist; the dummy hash
/// is verified instead and the result forced to `false`, keeping the
/// unknown-user and wrong-password paths indistinguishable by timing.
pub async fn verify_login_password(
    stored_hash: Option<String>,
    password: String,
) -> Result<bool, ApiError> {
    tokio::task::spawn_blocking(move || {
        // A row whose hash is not a PHC string is a row nobody can log in
        // as — the reserved `assistant` account is seeded with a literal
        // `'!'` for exactly that reason (migration 0015). It is treated as
        // "no such user", not as a server fault: raising a 500 here would
        // answer differently for that one username than for every other,
        // which is a per-account existence oracle, and it would skip the
        // argon2 work that keeps the paths indistinguishable by timing.
        let mut can_match = stored_hash.is_some();
        let hash_string = match stored_hash {
            Some(stored) if PasswordHash::new(&stored).is_ok() => stored,
            Some(_) => {
                can_match = false;
                dummy_hash().to_string()
            }
            None => dummy_hash().to_string(),
        };
        let parsed = PasswordHash::new(&hash_string).map_err(|err| {
            anyhow::anyhow!("the timing-equalization hash is not a valid PHC string: {err}")
        })?;
        let matches = Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok();
        Ok(can_match && matches)
    })
    .await
    .map_err(|err| ApiError::Internal(anyhow::anyhow!("verification task panicked: {err}")))?
    .map_err(ApiError::Internal)
}

/// Dummy PHC hash for timing equalization, computed once with the same
/// `Argon2::default()` parameters used for real hashes.
fn dummy_hash() -> &'static str {
    static DUMMY: OnceLock<String> = OnceLock::new();
    DUMMY.get_or_init(|| {
        let salt = SaltString::generate(&mut OsRng);
        Argon2::default()
            .hash_password(b"family-connect-timing-equalization-dummy", &salt)
            .expect("argon2 with default params cannot fail on a fixed input")
            .to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_hashed_password_verifies_and_a_wrong_one_does_not() {
        let hash = hash_password("correct horse battery".to_string())
            .await
            .expect("hashing succeeds");
        assert!(
            hash.starts_with("$argon2id$"),
            "PHC argon2id string: {hash}"
        );
        assert!(
            verify_login_password(Some(hash.clone()), "correct horse battery".to_string())
                .await
                .expect("verification runs")
        );
        assert!(
            !verify_login_password(Some(hash), "wrong password".to_string())
                .await
                .expect("verification runs")
        );
    }

    #[tokio::test]
    async fn an_unknown_user_always_fails_verification_after_dummy_work() {
        assert!(
            !verify_login_password(None, "anything".to_string())
                .await
                .expect("dummy verification runs")
        );
    }
}
