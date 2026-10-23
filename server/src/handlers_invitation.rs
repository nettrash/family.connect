//! Invitations: an owner asks somebody they share ANOTHER family with into
//! this one, and the invited person accepts or declines (docs/protocol.md,
//! "Several families"; decision D4 in docs/families-2026-10-22.md).
//!
//! Nobody is ever put into a family without agreeing, because a family's
//! owner has powers over its members and anyone may start a family. So an
//! invitation is a REQUEST in the other direction from a join request: the
//! owner has already said yes, the person has not, and until they do they are
//! not in the family and nobody there is told about them.
//!
//! Pending only, in the table as in every list: accepting, declining,
//! withdrawing and every way an invitation lapses DELETE its row, so there is
//! no status column for a reader to forget (migration 0057). Membership
//! itself goes through `handlers_family::grant_membership`, the one door
//! every way into a family shares, which is also where an invitation lapses
//! when its person comes in by another door.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;
use sqlx::postgres::PgRow;
use sqlx::{PgExecutor, Row};

use crate::auth::AuthUser;
use crate::error::{ApiError, AppJson, codes};
use crate::events;
use crate::handlers_family::{Granted, IdPath, LapsedInvitation};
use crate::models::{Birthday, Invitation, User};
use crate::state::AppState;

/// At most this many pending invitations in one family (docs/protocol.md,
/// "Limits"). Fixed: an owner with fifty unanswered invitations has a
/// different problem from a missing fifty-first.
pub const MAX_PENDING_INVITATIONS: i64 = 50;

#[derive(Debug, Deserialize)]
pub struct CreateInvitationRequest {
    pub user_id: i64,
}

/// Every column an `Invitation` is built from: the row, its family's name,
/// the invited person (`u_*`) and the owner who sent it (`b_*`, NULL only
/// for an inviter whose row is gone — see `Invitation::invited_by`).
const SELECT_INVITATION: &str = "SELECT i.id, i.family_id, i.created_at, f.name AS family_name,
            u.id AS u_id, u.username AS u_username, u.display_name AS u_display_name,
            u.created_at AS u_created_at, u.avatar_version AS u_avatar_version,
            u.birthday_month AS u_birthday_month, u.birthday_day AS u_birthday_day,
            u.deleted_at AS u_deleted_at,
            b.id AS b_id, b.username AS b_username, b.display_name AS b_display_name,
            b.created_at AS b_created_at, b.avatar_version AS b_avatar_version,
            b.birthday_month AS b_birthday_month, b.birthday_day AS b_birthday_day,
            b.deleted_at AS b_deleted_at
       FROM invitations i
       JOIN families f ON f.id = i.family_id
       JOIN users u ON u.id = i.user_id
       LEFT JOIN users b ON b.id = i.invited_by";

/// One side of an invitation, out of its prefixed columns.
fn user_from_prefixed(row: &PgRow, prefix: &str) -> Option<User> {
    let id: Option<i64> = row.get(format!("{prefix}id").as_str());
    let month: Option<i16> = row.get(format!("{prefix}birthday_month").as_str());
    let day: Option<i16> = row.get(format!("{prefix}birthday_day").as_str());
    id.map(|id| User {
        id,
        username: row.get(format!("{prefix}username").as_str()),
        display_name: row.get(format!("{prefix}display_name").as_str()),
        created_at: row.get(format!("{prefix}created_at").as_str()),
        avatar_version: row.get(format!("{prefix}avatar_version").as_str()),
        birthday: match (month, day) {
            (Some(month), Some(day)) => Some(Birthday { month, day }),
            _ => None,
        },
        deleted: row
            .get::<Option<time::OffsetDateTime>, _>(format!("{prefix}deleted_at").as_str())
            .is_some(),
    })
}

fn invitation_from_row(row: &PgRow) -> Invitation {
    Invitation {
        id: row.get("id"),
        family_id: row.get("family_id"),
        family_name: row.get("family_name"),
        invited_by: user_from_prefixed(row, "b_"),
        user: user_from_prefixed(row, "u_").expect("an invitation always names its person"),
        created_at: row.get("created_at"),
    }
}

/// One invitation by id.
async fn fetch_invitation<'e, E: PgExecutor<'e>>(
    executor: E,
    invitation_id: i64,
) -> Result<Option<Invitation>, ApiError> {
    let row = sqlx::query(&format!("{SELECT_INVITATION} WHERE i.id = $1"))
        .bind(invitation_id)
        .fetch_optional(executor)
        .await?;
    Ok(row.as_ref().map(invitation_from_row))
}

/// The caller's own pending invitations, oldest first — what `GET /me`
/// carries as `invitations`.
pub async fn invitations_for_user(
    pool: &sqlx::PgPool,
    user_id: i64,
) -> Result<Vec<Invitation>, ApiError> {
    let rows = sqlx::query(&format!(
        "{SELECT_INVITATION} WHERE i.user_id = $1 ORDER BY i.created_at, i.id"
    ))
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(invitation_from_row).collect())
}

/// `POST /families/{family}/invitations` (owner) `{user_id}`.
///
/// Who may be invited is the issue's own sentence: "the owner of the family
/// can invite members of families to which he belongs". So the person must
/// share at least one OTHER family with the owner, and nobody else on the
/// server can be named — an id that does not exist answers exactly as a
/// stranger does, `not_same_family`, so the endpoint never says which
/// accounts exist.
///
/// A block between the two, either way, refuses nothing: a refusal would
/// tell the owner about a block they may not be party to.
pub async fn create_invitation(
    auth: AuthUser,
    State(state): State<AppState>,
    AppJson(req): AppJson<CreateInvitationRequest>,
) -> Result<Response, ApiError> {
    let family_id = crate::handlers_family::require_owner_family(&state, &auth).await?;
    let target = req.user_id;

    // Already in: the owner can see them in the roster, so saying so tells
    // them nothing new — and it is checked first, so the owner themselves
    // (who is in) is answered this way too.
    if crate::families::is_member(&state.pool, family_id, target).await? {
        return Err(ApiError::conflict(
            codes::USER_ALREADY_IN_FAMILY,
            "this person is already in the family",
        ));
    }
    let shares_another: bool = sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1 FROM family_members mine
               JOIN family_members theirs ON theirs.family_id = mine.family_id
              WHERE mine.user_id = $1 AND theirs.user_id = $2 AND mine.family_id <> $3)",
    )
    .bind(auth.user_id)
    .bind(target)
    .bind(family_id)
    .fetch_one(&state.pool)
    .await?;
    if !shares_another {
        return Err(ApiError::forbidden(
            codes::NOT_SAME_FAMILY,
            "you can invite only people from your other families",
        ));
    }

    let mut tx = state.pool.begin().await?;
    // The family row, locked: the count and the insert are one decision, so
    // two invitations racing for the fiftieth slot cannot both get it. The
    // same lock every membership change takes, so an invitation cannot be
    // created into a family half way through being deleted either.
    let owner: Option<i64> =
        sqlx::query_scalar("SELECT owner_user_id FROM families WHERE id = $1 FOR UPDATE")
            .bind(family_id)
            .fetch_optional(&mut *tx)
            .await?;
    if owner != Some(auth.user_id) {
        return Err(ApiError::forbidden(
            codes::NOT_FAMILY_OWNER,
            "only the family owner may do this",
        ));
    }
    // Already invited: the pending one comes back, `200` rather than `201`,
    // so a double tap is one invitation and one push.
    let existing: Option<i64> =
        sqlx::query_scalar("SELECT id FROM invitations WHERE family_id = $1 AND user_id = $2")
            .bind(family_id)
            .bind(target)
            .fetch_optional(&mut *tx)
            .await?;
    if let Some(existing) = existing {
        let invitation = fetch_invitation(&mut *tx, existing)
            .await?
            .ok_or_else(|| ApiError::Internal(anyhow::anyhow!("invitation vanished under lock")))?;
        tx.commit().await?;
        return Ok((StatusCode::OK, Json(json!({"invitation": invitation}))).into_response());
    }
    let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM invitations WHERE family_id = $1")
        .bind(family_id)
        .fetch_one(&mut *tx)
        .await?;
    if pending >= MAX_PENDING_INVITATIONS {
        return Err(ApiError::validation(format!(
            "a family may have at most {MAX_PENDING_INVITATIONS} pending invitations"
        )));
    }
    // The invited account must still be a live one — read under a share
    // lock on its row for the reason `join_family` gives: a scrub that
    // committed while this request was in flight must not leave an
    // invitation addressed to "Deleted account".
    let alive: Option<i64> =
        sqlx::query_scalar("SELECT id FROM users WHERE id = $1 AND deleted_at IS NULL FOR SHARE")
            .bind(target)
            .fetch_optional(&mut *tx)
            .await?;
    if alive.is_none() {
        return Err(ApiError::forbidden(
            codes::NOT_SAME_FAMILY,
            "you can invite only people from your other families",
        ));
    }
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO invitations (family_id, user_id, invited_by) VALUES ($1, $2, $3)
         RETURNING id",
    )
    .bind(family_id)
    .bind(target)
    .bind(auth.user_id)
    .fetch_one(&mut *tx)
    .await?;
    let invitation = fetch_invitation(&mut *tx, id)
        .await?
        .ok_or_else(|| ApiError::Internal(anyhow::anyhow!("invitation vanished in its own tx")))?;
    tx.commit().await?;

    events::log_fanout_error(
        "invitation",
        events::deliver_invitation(&state, &invitation, auth.user_id).await,
    );
    events::log_fanout_error(
        "invitation_push",
        events::push_invitation(&state, &invitation).await,
    );
    Ok((StatusCode::CREATED, Json(json!({"invitation": invitation}))).into_response())
}

/// `GET /families/{family}/invitations` (owner) — pending only, oldest first.
pub async fn list_invitations(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Response, ApiError> {
    let family_id = crate::handlers_family::require_owner_family(&state, &auth).await?;
    let rows = sqlx::query(&format!(
        "{SELECT_INVITATION} WHERE i.family_id = $1 ORDER BY i.created_at, i.id"
    ))
    .bind(family_id)
    .fetch_all(&state.pool)
    .await?;
    let invitations: Vec<Invitation> = rows.iter().map(invitation_from_row).collect();
    Ok((StatusCode::OK, Json(json!({"invitations": invitations}))).into_response())
}

/// `DELETE /families/{family}/invitations/{id}` (owner) — withdraw one.
/// Idempotent: `204` whether or not there was anything to withdraw.
pub async fn withdraw_invitation(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(IdPath { id }): Path<IdPath>,
) -> Result<Response, ApiError> {
    let family_id = crate::handlers_family::require_owner_family(&state, &auth).await?;
    let withdrawn: Option<i64> = sqlx::query_scalar(
        "DELETE FROM invitations WHERE id = $1 AND family_id = $2 RETURNING user_id",
    )
    .bind(id)
    .bind(family_id)
    .fetch_optional(&state.pool)
    .await?;
    if let Some(invited) = withdrawn {
        events::log_fanout_error(
            "invitation_removed",
            events::deliver_invitation_removed(&state, id, family_id, invited, Some(auth.user_id))
                .await,
        );
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `POST /invitations/{id}/accept` (the invited person) → `200 {family}`.
///
/// Membership is immediate whatever the family's join policy — the owner
/// already said yes. The two caps are read here, as at a join, and both
/// leave the invitation PENDING: `family_full` and `too_many_families` are
/// facts about today, and somebody leaving makes the same accept work.
pub async fn accept_invitation(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(IdPath { id }): Path<IdPath>,
) -> Result<Response, ApiError> {
    let not_pending = || {
        ApiError::conflict(
            codes::INVITATION_NOT_PENDING,
            "this invitation is not pending",
        )
    };
    // Somebody else's invitation answers exactly as one that never existed.
    let family_id: Option<i64> =
        sqlx::query_scalar("SELECT family_id FROM invitations WHERE id = $1 AND user_id = $2")
            .bind(id)
            .bind(auth.user_id)
            .fetch_optional(&state.pool)
            .await?;
    let Some(family_id) = family_id else {
        return Err(not_pending());
    };

    let mut tx = state.pool.begin().await?;
    // The grant takes the family row's lock and then the account's; the
    // invitation is re-read after it, under that same lock, so a withdrawal
    // that landed in between is honoured rather than overridden.
    let granted = crate::handlers_family::grant_membership(
        &mut tx,
        family_id,
        auth.user_id,
        state.cfg.limits.max_family_members,
        state.cfg.families.max_families_per_account,
    )
    .await?;
    match granted {
        // The grant lapses the invitation itself — it is the person coming
        // in, which is every door's rule — so it hands this very row back.
        Granted::Yes(Some(lapsed)) if lapsed.id == id => {
            tx.commit().await?;
            crate::handlers_family::announce_lapsed(&state, &[lapsed]).await;
            let joined = crate::handlers_family::user_brief(&state, auth.user_id).await?;
            crate::handlers_family::announce_joined(&state, family_id, joined).await;
            let family =
                crate::handlers_family::family_api(&state, family_id, auth.user_id).await?;
            Ok((StatusCode::OK, Json(json!({"family": family}))).into_response())
        }
        // Withdrawn while this request waited for the lock: nobody is let
        // in on an invitation that no longer exists.
        Granted::Yes(_) => {
            tx.rollback().await?;
            Err(not_pending())
        }
        // In the family already (and so the invitation is moot), or the
        // account went. The invitation is cleared, committed, and the
        // answer is the one a cleared invitation gets.
        Granted::AlreadyInAFamily => {
            let removed: Option<i64> = sqlx::query_scalar(
                "DELETE FROM invitations WHERE id = $1 AND user_id = $2 RETURNING family_id",
            )
            .bind(id)
            .bind(auth.user_id)
            .fetch_optional(&mut *tx)
            .await?;
            tx.commit().await?;
            if removed.is_some() {
                crate::handlers_family::announce_lapsed(
                    &state,
                    &[LapsedInvitation {
                        id,
                        family_id,
                        user_id: auth.user_id,
                    }],
                )
                .await;
            }
            Err(not_pending())
        }
        Granted::FamilyFull => {
            tx.rollback().await?;
            Err(ApiError::conflict(
                codes::FAMILY_FULL,
                "this family is full",
            ))
        }
        Granted::TooManyFamilies => {
            tx.rollback().await?;
            Err(ApiError::conflict(
                codes::TOO_MANY_FAMILIES,
                "you are already in as many families as an account may be",
            ))
        }
    }
}

/// `POST /invitations/{id}/decline` (the invited person) → `204`.
///
/// Idempotent for the invited person, and the owner is NOT pushed — only
/// their open screens are told, by `invitation_removed`. Anybody else's id
/// is a quiet `204` too: the endpoint never says whose an invitation is.
pub async fn decline_invitation(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(IdPath { id }): Path<IdPath>,
) -> Result<Response, ApiError> {
    let declined: Option<i64> = sqlx::query_scalar(
        "DELETE FROM invitations WHERE id = $1 AND user_id = $2 RETURNING family_id",
    )
    .bind(id)
    .bind(auth.user_id)
    .fetch_optional(&state.pool)
    .await?;
    if let Some(family_id) = declined {
        crate::handlers_family::announce_lapsed(
            &state,
            &[LapsedInvitation {
                id,
                family_id,
                user_id: auth.user_id,
            }],
        )
        .await;
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}
