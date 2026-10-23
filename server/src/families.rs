//! Membership questions, asked of `family_members` (migration 0057).
//!
//! Until 2026-10-22 every one of these was a comparison against the single
//! `users.family_id`, written out wherever it was needed. An account may now
//! be in several families (docs/protocol.md, "Several families"), and each
//! question has become a small join that is easy to get subtly wrong — "is
//! in a family" is not "is in THIS family", and "shares a family" is not "is
//! in my family". So the questions live here, once, with their answers'
//! exact meaning in their names, and the rest of the server asks them.
//!
//! A scrubbed account holds no membership (the scrub removes its rows), and
//! the assistant has never held one, so neither question below can ever
//! answer "yes" about them without any `deleted_at` test of its own — the
//! same property `users.family_id IS NULL` gave every caller before.

use std::collections::HashMap;

use sqlx::{PgExecutor, PgPool};

use crate::error::ApiError;

/// The families of a CHAT, as `Chat.family_ids` carries them: the one family
/// of a family chat, a group or an `ai` chat; for a direct chat, every family
/// both people are in, ascending — which is empty for a pair that shares
/// none, whose chat is closed (docs/protocol.md, "Several families").
///
/// A SQL expression over a `chats` row aliased `c`, so a listing computes it
/// in the same statement that reads the rows rather than once per chat.
pub const CHAT_FAMILY_IDS_SQL: &str = "CASE WHEN c.kind = 'direct' THEN ARRAY(
         SELECT fa.family_id FROM family_members fa
           JOIN family_members fb ON fb.family_id = fa.family_id AND fb.user_id = c.user_b_id
          WHERE fa.user_id = c.user_a_id
          ORDER BY fa.family_id)
     ELSE ARRAY[c.family_id] END";

/// Every family `user_id` is in, home first: the earliest membership, ties
/// to the lower family id — the order `AuthUser.family_ids` carries.
pub async fn family_ids_of<'e, E: PgExecutor<'e>>(
    executor: E,
    user_id: i64,
) -> Result<Vec<i64>, ApiError> {
    let ids = sqlx::query_scalar(
        "SELECT family_id FROM family_members WHERE user_id = $1
          ORDER BY joined_at, family_id",
    )
    .bind(user_id)
    .fetch_all(executor)
    .await?;
    Ok(ids)
}

/// Is `user_id` a member of `family_id` right now?
pub async fn is_member<'e, E: PgExecutor<'e>>(
    executor: E,
    family_id: i64,
    user_id: i64,
) -> Result<bool, ApiError> {
    let found = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM family_members WHERE family_id = $1 AND user_id = $2)",
    )
    .bind(family_id)
    .bind(user_id)
    .fetch_one(executor)
    .await?;
    Ok(found)
}

/// Every current member of a family.
pub async fn member_ids<'e, E: PgExecutor<'e>>(
    executor: E,
    family_id: i64,
) -> Result<Vec<i64>, ApiError> {
    let ids = sqlx::query_scalar("SELECT user_id FROM family_members WHERE family_id = $1")
        .bind(family_id)
        .fetch_all(executor)
        .await?;
    Ok(ids)
}

/// The families two accounts are BOTH in, ascending.
pub async fn shared_families<'e, E: PgExecutor<'e>>(
    executor: E,
    a: i64,
    b: i64,
) -> Result<Vec<i64>, ApiError> {
    let ids = sqlx::query_scalar(
        "SELECT fa.family_id FROM family_members fa
           JOIN family_members fb ON fb.family_id = fa.family_id AND fb.user_id = $2
          WHERE fa.user_id = $1
          ORDER BY fa.family_id",
    )
    .bind(a)
    .bind(b)
    .fetch_all(executor)
    .await?;
    Ok(ids)
}

/// Do two accounts share at least one family? What "the same family" means
/// now for everything that is a PAIR rather than a family's — a direct chat,
/// a call, a block, a profile picture.
pub async fn share_a_family<'e, E: PgExecutor<'e>>(
    executor: E,
    a: i64,
    b: i64,
) -> Result<bool, ApiError> {
    let shared = sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1 FROM family_members fa
               JOIN family_members fb ON fb.family_id = fa.family_id AND fb.user_id = $2
              WHERE fa.user_id = $1)",
    )
    .bind(a)
    .bind(b)
    .fetch_one(executor)
    .await?;
    Ok(shared)
}

/// `Chat.family_ids` as one READER is sent it — [`CHAT_FAMILY_IDS_SQL`], except
/// for a direct chat whose pair shares no family any more. The side that left
/// no longer holds it at all; the side that STAYED still does, exactly as this
/// server has always answered, and a list that named no family would hide it
/// from every app that sorts by `family_ids`. So the stayer is sent the family
/// the chat was opened in when they are still in it, and their home family when
/// they are not (docs/protocol.md, "Several families"). What is SENT reads it,
/// and so does what the reader may attach there — an upload is made for the
/// family the reader's app lists the chat under; frames and access keep
/// reading the pair's shared list, in which a closed chat names nothing.
/// `viewer` is the SQL placeholder of the reader's id, over a `chats` row
/// aliased `c`.
pub fn viewer_chat_family_ids_sql(viewer: &str) -> String {
    format!(
        "COALESCE(NULLIF({CHAT_FAMILY_IDS_SQL}, '{{}}'::bigint[]),
                  CASE WHEN EXISTS (SELECT 1 FROM family_members vf
                                     WHERE vf.user_id = {viewer} AND vf.family_id = c.family_id)
                       THEN ARRAY[c.family_id]
                       ELSE ARRAY(SELECT vh.family_id FROM family_members vh
                                   WHERE vh.user_id = {viewer}
                                   ORDER BY vh.joined_at, vh.family_id LIMIT 1)
                  END)"
    )
}

/// The families of one chat — see [`CHAT_FAMILY_IDS_SQL`]. Empty for a chat
/// that does not exist.
pub async fn chat_family_ids<'e, E: PgExecutor<'e>>(
    executor: E,
    chat_id: i64,
) -> Result<Vec<i64>, ApiError> {
    let ids: Option<Vec<i64>> = sqlx::query_scalar(&format!(
        "SELECT {CHAT_FAMILY_IDS_SQL} FROM chats c WHERE c.id = $1"
    ))
    .bind(chat_id)
    .fetch_optional(executor)
    .await?;
    Ok(ids.unwrap_or_default())
}

/// The families of one chat as `viewer` is sent them — see
/// [`viewer_chat_family_ids_sql`]. Empty for a chat that does not exist.
pub async fn viewer_chat_family_ids<'e, E: PgExecutor<'e>>(
    executor: E,
    chat_id: i64,
    viewer: i64,
) -> Result<Vec<i64>, ApiError> {
    let ids: Option<Vec<i64>> = sqlx::query_scalar(&format!(
        "SELECT {} FROM chats c WHERE c.id = $1",
        viewer_chat_family_ids_sql("$2")
    ))
    .bind(chat_id)
    .bind(viewer)
    .fetch_optional(executor)
    .await?;
    Ok(ids.unwrap_or_default())
}

/// The HOME family of each of `user_ids` that has one — what an app that
/// predates several families is shown, and so what decides which frames its
/// connection may receive (docs/protocol.md, "Several families"). An account
/// in no family is simply absent from the map.
pub async fn homes(pool: &PgPool, user_ids: &[i64]) -> Result<HashMap<i64, i64>, ApiError> {
    if user_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT DISTINCT ON (user_id) user_id, family_id
           FROM family_members
          WHERE user_id = ANY($1)
          ORDER BY user_id, joined_at, family_id",
    )
    .bind(user_ids)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().collect())
}

/// Of `family_ids` (a frame's scope), which may a connection whose account's
/// home family is `home` be sent? The whole rule for an app that predates
/// several families, pure so it is pinned by a unit test: only a frame about
/// its home family, and nothing at all to an account in none.
pub fn home_in_scope(home: Option<i64>, family_ids: &[i64]) -> bool {
    home.is_some_and(|home| family_ids.contains(&home))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_legacy_connection_hears_only_about_its_home_family() {
        assert!(home_in_scope(Some(3), &[3]));
        assert!(
            home_in_scope(Some(3), &[1, 3, 9]),
            "a direct chat listed at home"
        );
        assert!(!home_in_scope(Some(3), &[4]), "another family's frame");
        assert!(!home_in_scope(Some(3), &[]), "an empty scope names no home");
        assert!(!home_in_scope(None, &[3]), "an account in no family");
    }
}
