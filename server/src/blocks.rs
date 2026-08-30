//! One member deciding not to read another (docs/protocol.md, "Blocking a
//! member").
//!
//! Three functions rather than one, because three different questions are
//! asked in three different places and each wants a different shape:
//!
//! - [`blocked_by`] — "who has THIS caller blocked", the list shipped on
//!   `GET /me` and `GET /families/mine`.
//! - [`blockers_of`] — "which of these candidates have blocked THIS
//!   sender", the push gate, asked once per outgoing message against a
//!   whole recipient list.
//! - [`blocks`] — "has A blocked B", the direct-chat and call refusals.
//!
//! **Every refusal built on these is aimed at the BLOCKER.** A block is
//! silent: nothing here is ever used to change what the blocked person
//! sees, is told, or can do, because any of those is a signal they could
//! test for.

use std::collections::HashSet;

use sqlx::PgPool;

use crate::error::ApiError;

/// Everyone `user_id` has blocked, ascending.
///
/// A complete state-set and never a delta: the caller REPLACES what it
/// stores with what this returns, which is why the wire key is always
/// present and `[]` when empty (protocol.md, `GET /me`). Ordered so two
/// reads of an unchanged list are byte-identical and a client diffing them
/// sees nothing move.
pub async fn blocked_by(pool: &PgPool, user_id: i64) -> Result<Vec<i64>, ApiError> {
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT blocked_user_id FROM member_blocks
          WHERE blocker_user_id = $1
          ORDER BY blocked_user_id",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(ids)
}

/// Which of `candidates` have blocked `sender` — the push gate.
///
/// Asked once per outgoing message with the whole candidate list, rather
/// than once per candidate, because the alternative is a query per
/// recipient on the hot path. The index that makes this cheap is
/// `member_blocks_blocked_idx` (migration 0028), which exists for exactly
/// this direction: the primary key leads on the blocker and cannot serve
/// "who has blocked this sender".
///
/// Returns a set because the caller's question is membership, and an empty
/// set — the overwhelmingly common answer — costs one round trip.
pub async fn blockers_of(
    pool: &PgPool,
    sender: i64,
    candidates: &[i64],
) -> Result<HashSet<i64>, ApiError> {
    if candidates.is_empty() {
        return Ok(HashSet::new());
    }
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT blocker_user_id FROM member_blocks
          WHERE blocked_user_id = $1 AND blocker_user_id = ANY($2)",
    )
    .bind(sender)
    .bind(candidates)
    .fetch_all(pool)
    .await?;
    Ok(ids.into_iter().collect())
}

/// Has `blocker` blocked `blocked`?
///
/// Deliberately one-directional. The two directions are NOT interchangeable
/// and a caller that passed them the wrong way round would build the leak
/// this whole feature is written to avoid: refusing the blocked person
/// something they could otherwise do is how they find out.
pub async fn blocks(pool: &PgPool, blocker: i64, blocked: i64) -> Result<bool, ApiError> {
    // Selects the BIGINT column rather than `SELECT 1`: a bare literal 1 is
    // `int4` in PostgreSQL and decoding it as `i64` fails at runtime, which
    // is the kind of error that hides wherever the caller happens to
    // swallow it.
    let found: Option<i64> = sqlx::query_scalar(
        "SELECT blocked_user_id FROM member_blocks
          WHERE blocker_user_id = $1 AND blocked_user_id = $2",
    )
    .bind(blocker)
    .bind(blocked)
    .fetch_optional(pool)
    .await?;
    Ok(found.is_some())
}
