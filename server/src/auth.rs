//! Authentication: argon2id password hashing, opaque bearer sessions, and
//! the `AuthUser` extractor.
//!
//! Design notes:
//! - Concurrency is BOUNDED (see `configure_hash_concurrency`): argon2id
//!   allocates 19 MiB per hash and both endpoints that reach it are
//!   unauthenticated, so without a bound a stranger decides how much memory
//!   this process asks for.
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
use tokio::sync::{Semaphore, SemaphorePermit};
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

/// How many argon2 hashes may run at once.
///
/// `Argon2::default()` is 19 MiB of memory per hash and a bare
/// `#[tokio::main]` hands `spawn_blocking` a 512-thread pool, so an
/// unbounded flood of logins can ask the allocator for ~9.7 GiB and take
/// the process out. Both endpoints that hash are unauthenticated — and
/// login hashes even for a username that does not exist, on purpose, to
/// keep the unknown-user and wrong-password paths indistinguishable by
/// timing — so the amount of memory a stranger can make this server
/// allocate is exactly what this bounds.
///
/// Waiters QUEUE rather than being refused. A queued request costs a task,
/// not 19 MiB, and refusing a legitimate sign-in during a burst would be a
/// worse answer than making it wait. Controlling the arrival RATE is
/// nginx's `limit_req` (see `nginx/family-connect.conf`); this bounds the
/// peak memory whatever gets through.
static HASH_SLOTS: OnceLock<Semaphore> = OnceLock::new();

/// Set the bound, once, at boot. Later calls are ignored — the value comes
/// from config and config is read once.
pub fn configure_hash_concurrency(permits: usize) {
    let _ = HASH_SLOTS.set(Semaphore::new(permits.max(1)));
}

/// The bound, defaulting for tests and for any embedder that never called
/// `configure_hash_concurrency`. A default of 0 would deadlock, so this is
/// deliberately a real number rather than "unbounded".
fn hash_slots() -> &'static Semaphore {
    HASH_SLOTS.get_or_init(|| Semaphore::new(8))
}

/// Wait for a slot. The permit is held by the caller for as long as the
/// hash runs, which is what makes the bound mean peak MEMORY rather than
/// peak requests-in-flight.
async fn hash_permit() -> Result<SemaphorePermit<'static>, ApiError> {
    hash_slots()
        .acquire()
        .await
        .map_err(|_| ApiError::Internal(anyhow::anyhow!("the password-hash semaphore was closed")))
}

/// Hash a password with argon2id (PHC string output).
pub async fn hash_password(password: String) -> Result<String, ApiError> {
    let _permit = hash_permit().await?;
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
    let _permit = hash_permit().await?;
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

    /// The bound is process-wide by design, so the tests that manipulate
    /// it cannot run beside each other: each one takes "every permit", and
    /// a sibling holding some makes that a different — and silently
    /// weaker — assertion. Cargo runs tests in threads within one process,
    /// so this is a real hazard rather than a theoretical one.
    static SEMAPHORE_TESTS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    /// The bound has to be REAL, not decorative: without it a login flood
    /// can ask the allocator for 19 MiB × 512 threads ≈ 9.7 GiB. So this
    /// takes every permit and proves a hash cannot start, rather than
    /// asserting that a number was stored somewhere.
    #[tokio::test]
    async fn hashing_waits_when_every_slot_is_taken() {
        let _serialized = SEMAPHORE_TESTS.lock().await;
        let slots = hash_slots();
        let total = slots.available_permits();
        assert!(total > 0, "a zero bound would deadlock every login");

        let held = slots
            .acquire_many(total as u32)
            .await
            .expect("the semaphore is open");

        let mut hashing = tokio::spawn(hash_password("correct horse battery".to_string()));

        // Long enough that a hash which was NOT bounded would have finished
        // — argon2id at default parameters is tens of milliseconds.
        let blocked =
            tokio::time::timeout(std::time::Duration::from_millis(750), &mut hashing).await;
        assert!(
            blocked.is_err(),
            "a hash started while every slot was held: the bound is not enforced"
        );

        // And it is a queue, not a refusal: releasing lets it through.
        drop(held);
        let hash = tokio::time::timeout(std::time::Duration::from_secs(10), hashing)
            .await
            .expect("the queued hash runs once a slot frees")
            .expect("the task did not panic")
            .expect("hashing succeeds");
        assert!(hash.starts_with("$argon2id$"));
    }

    /// Acquiring a permit is not the same as HOLDING one, and only the
    /// second bounds anything.
    ///
    /// `let _ = hash_permit()` compiles, acquires, and drops the permit on
    /// the same line — so every hash still passes through the semaphore
    /// and 512 of them still run at once. The bound reads as present and
    /// is worth nothing, which is the failure this pins.
    ///
    /// Timing, but against a baseline measured on the same machine in the
    /// same build, so the assertion is "three hashes through one slot take
    /// materially longer than one" rather than any absolute number. argon2
    /// runs on the blocking pool, so an unheld permit really does let them
    /// overlap even on a current-thread runtime.
    #[tokio::test]
    async fn the_permit_is_held_for_the_whole_hash_not_just_taken() {
        let _serialized = SEMAPHORE_TESTS.lock().await;
        let slots = hash_slots();
        let total = slots.available_permits();

        let start = std::time::Instant::now();
        hash_password("correct horse battery".to_string())
            .await
            .expect("hashing succeeds");
        let one = start.elapsed();

        // Leave exactly one slot, so three hashes must go through it in
        // single file.
        let held = slots
            .acquire_many((total - 1) as u32)
            .await
            .expect("the semaphore is open");

        let start = std::time::Instant::now();
        let mut running = Vec::new();
        for _ in 0..3 {
            running.push(tokio::spawn(hash_password(
                "correct horse battery".to_string(),
            )));
        }
        for handle in running {
            handle
                .await
                .expect("the task did not panic")
                .expect("hashing succeeds");
        }
        let three = start.elapsed();
        drop(held);

        assert!(
            three > one * 2,
            "three hashes through one slot took {three:?}, barely more than one ({one:?}) \
             — the permit is not held across the hash, so the bound is decorative"
        );
    }

    /// A leaked permit would not fail anything today and would deadlock
    /// every login on the box some weeks later, which is the worst shape a
    /// bug can have.
    #[tokio::test]
    async fn a_finished_hash_gives_its_slot_back() {
        let _serialized = SEMAPHORE_TESTS.lock().await;
        let slots = hash_slots();
        let before = slots.available_permits();

        hash_password("correct horse battery".to_string())
            .await
            .expect("hashing succeeds");
        let _ = verify_login_password(None, "anything".to_string())
            .await
            .expect("verification runs");

        assert_eq!(
            slots.available_permits(),
            before,
            "a permit was not returned: hashing leaks slots until logins deadlock"
        );
    }

    /// Verification is the OTHER unauthenticated door, and it hashes even
    /// for a username that does not exist. It must be bounded too — a bound
    /// on registration alone would leave the cheaper attack wide open.
    #[tokio::test]
    async fn verification_is_bounded_too() {
        let _serialized = SEMAPHORE_TESTS.lock().await;
        let slots = hash_slots();
        let total = slots.available_permits();
        let held = slots
            .acquire_many(total as u32)
            .await
            .expect("the semaphore is open");

        let mut verifying = tokio::spawn(verify_login_password(None, "guess".to_string()));
        let blocked =
            tokio::time::timeout(std::time::Duration::from_millis(750), &mut verifying).await;
        assert!(
            blocked.is_err(),
            "an unknown-user verification ran while every slot was held"
        );

        drop(held);
        let matched = tokio::time::timeout(std::time::Duration::from_secs(10), verifying)
            .await
            .expect("the queued verification runs")
            .expect("the task did not panic")
            .expect("verification runs");
        assert!(!matched, "an unknown user must never verify");
    }

    #[tokio::test]
    async fn a_hashed_password_verifies_and_a_wrong_one_does_not() {
        // Shares the process-wide bound; see SEMAPHORE_TESTS.
        let _serialized = SEMAPHORE_TESTS.lock().await;
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
        // Shares the process-wide bound; see SEMAPHORE_TESTS.
        let _serialized = SEMAPHORE_TESTS.lock().await;
        assert!(
            !verify_login_password(None, "anything".to_string())
                .await
                .expect("dummy verification runs")
        );
    }
}
