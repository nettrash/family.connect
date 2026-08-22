//! Family endpoints: create, join, membership management, invite codes.
//!
//! Concurrency discipline: membership changes never trust a read-then-write.
//! The single source of truth for "is this user in a family" is
//! `users.family_id`, and every path that grants membership runs the guarded
//! `UPDATE ... WHERE family_id IS NULL` — zero affected rows means somebody
//! else won the race and the request gets a clean 409 instead of corrupting
//! state. The approve path additionally takes `FOR UPDATE` on the join
//! request so two owner devices cannot approve the same request twice.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;
use sqlx::postgres::PgRow;
use sqlx::{PgConnection, Row};
use time::OffsetDateTime;

use crate::auth::AuthUser;
use crate::error::{ApiError, AppJson, codes};
use crate::events;
use crate::models::{Family, JoinRequest, Member, User, UserBrief};
use crate::state::AppState;
use crate::tokens;

#[derive(Debug, Deserialize)]
pub struct CreateFamilyRequest {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct JoinFamilyRequest {
    pub invite_code: String,
}

#[derive(Debug, Deserialize)]
pub struct PatchFamilyRequest {
    pub join_policy: String,
}

/// Internal view of a `families` row.
struct FamilyRecord {
    id: i64,
    name: String,
    invite_code: String,
    join_policy: String,
    owner_user_id: i64,
    created_at: OffsetDateTime,
}

impl FamilyRecord {
    fn from_row(row: &PgRow) -> Self {
        Self {
            id: row.get("id"),
            name: row.get("name"),
            invite_code: row.get("invite_code"),
            join_policy: row.get("join_policy"),
            owner_user_id: row.get("owner_user_id"),
            created_at: row.get("created_at"),
        }
    }

    /// Project to the API object; the invite code is owner-only.
    fn to_api(&self, caller_is_owner: bool) -> Family {
        Family {
            id: self.id,
            name: self.name.clone(),
            join_policy: self.join_policy.clone(),
            created_at: self.created_at,
            invite_code: caller_is_owner.then(|| self.invite_code.clone()),
        }
    }
}

const SELECT_FAMILY: &str =
    "SELECT id, name, invite_code, join_policy, owner_user_id, created_at FROM families";

async fn fetch_family(state: &AppState, family_id: i64) -> Result<FamilyRecord, ApiError> {
    let row = sqlx::query(&format!("{SELECT_FAMILY} WHERE id = $1"))
        .bind(family_id)
        .fetch_one(&state.pool)
        .await?;
    Ok(FamilyRecord::from_row(&row))
}

/// Resolve the caller's family and require them to be its owner.
async fn require_owner(state: &AppState, auth: &AuthUser) -> Result<FamilyRecord, ApiError> {
    let Some(family_id) = auth.family_id else {
        // Not being in a family at all also means not being an owner.
        return Err(ApiError::forbidden(
            codes::NOT_FAMILY_OWNER,
            "only the family owner may do this",
        ));
    };
    let family = fetch_family(state, family_id).await?;
    if family.owner_user_id != auth.user_id {
        return Err(ApiError::forbidden(
            codes::NOT_FAMILY_OWNER,
            "only the family owner may do this",
        ));
    }
    Ok(family)
}

/// Grant `user_id` membership of `family_id` inside `tx`.
///
/// Runs the race-proof guarded update, joins the family chat, and
/// auto-rejects the user's other pending join requests (they just got a
/// family; a later approval elsewhere would only 409).
async fn grant_membership(
    tx: &mut PgConnection,
    family_id: i64,
    user_id: i64,
) -> Result<bool, ApiError> {
    let updated =
        sqlx::query("UPDATE users SET family_id = $1 WHERE id = $2 AND family_id IS NULL")
            .bind(family_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    if updated == 0 {
        return Ok(false);
    }
    let chat_id: i64 =
        sqlx::query_scalar("SELECT id FROM chats WHERE family_id = $1 AND kind = 'family'")
            .bind(family_id)
            .fetch_one(&mut *tx)
            .await?;
    sqlx::query(
        "INSERT INTO chat_members (chat_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(chat_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE join_requests SET status = 'rejected', decided_at = now()
         WHERE user_id = $1 AND status = 'pending'",
    )
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    Ok(true)
}

async fn user_brief(state: &AppState, user_id: i64) -> Result<UserBrief, ApiError> {
    let row =
        sqlx::query("SELECT id, username, display_name, avatar_version FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(&state.pool)
            .await?;
    Ok(UserBrief {
        id: row.get("id"),
        username: row.get("username"),
        display_name: row.get("display_name"),
        avatar_version: row.get("avatar_version"),
    })
}

/// `POST /families` — create a family; the caller becomes owner and the
/// family chat comes to life in the same transaction.
pub async fn create_family(
    auth: AuthUser,
    State(state): State<AppState>,
    AppJson(req): AppJson<CreateFamilyRequest>,
) -> Result<Response, ApiError> {
    let name = req.name.trim().to_string();
    let name_len = name.chars().count();
    if !(1..=64).contains(&name_len) {
        return Err(ApiError::validation("family name must be 1-64 characters"));
    }
    if auth.family_id.is_some() {
        return Err(ApiError::conflict(
            codes::ALREADY_IN_FAMILY,
            "you already belong to a family",
        ));
    }

    // Invite codes have 30^8 combinations, so a unique-index collision is
    // effectively impossible — but "effectively" isn't "actually", and the
    // retry costs three lines.
    for _attempt in 0..5 {
        let invite_code = tokens::gen_invite_code();
        let mut tx = state.pool.begin().await?;

        let inserted = sqlx::query(
            "INSERT INTO families (name, invite_code, owner_user_id)
             VALUES ($1, $2, $3)
             RETURNING id, name, invite_code, join_policy, owner_user_id, created_at",
        )
        .bind(&name)
        .bind(&invite_code)
        .bind(auth.user_id)
        .fetch_one(&mut *tx)
        .await;

        let family = match inserted {
            Ok(row) => FamilyRecord::from_row(&row),
            Err(sqlx::Error::Database(db_err))
                if db_err.constraint() == Some("families_invite_code_uq") =>
            {
                continue; // regenerate and retry
            }
            Err(err) => return Err(err.into()),
        };

        sqlx::query("INSERT INTO chats (family_id, kind) VALUES ($1, 'family')")
            .bind(family.id)
            .execute(&mut *tx)
            .await?;

        if !grant_membership(&mut tx, family.id, auth.user_id).await? {
            // Lost a race against another create/join from the same account.
            tx.rollback().await?;
            return Err(ApiError::conflict(
                codes::ALREADY_IN_FAMILY,
                "you already belong to a family",
            ));
        }
        tx.commit().await?;

        return Ok((
            StatusCode::CREATED,
            Json(json!({"family": family.to_api(true)})),
        )
            .into_response());
    }
    Err(ApiError::Internal(anyhow::anyhow!(
        "could not generate a unique invite code after 5 attempts"
    )))
}

/// `POST /families/join` — immediate membership for `open` families, a join
/// request for `approval` ones.
pub async fn join_family(
    auth: AuthUser,
    State(state): State<AppState>,
    AppJson(req): AppJson<JoinFamilyRequest>,
) -> Result<Response, ApiError> {
    // Codes are generated from an uppercase alphabet; accept lowercase input
    // because humans type them from a screen across the room.
    let invite_code = req.invite_code.trim().to_uppercase();

    let row = sqlx::query(&format!("{SELECT_FAMILY} WHERE invite_code = $1"))
        .bind(&invite_code)
        .fetch_optional(&state.pool)
        .await?;
    let Some(row) = row else {
        return Err(ApiError::not_found(
            codes::INVALID_INVITE_CODE,
            "no family with this invite code",
        ));
    };
    let family = FamilyRecord::from_row(&row);

    if auth.family_id.is_some() {
        return Err(ApiError::conflict(
            codes::ALREADY_IN_FAMILY,
            "you already belong to a family",
        ));
    }

    if family.join_policy == "open" {
        let mut tx = state.pool.begin().await?;
        if !grant_membership(&mut tx, family.id, auth.user_id).await? {
            tx.rollback().await?;
            return Err(ApiError::conflict(
                codes::ALREADY_IN_FAMILY,
                "you already belong to a family",
            ));
        }
        tx.commit().await?;

        let joined = user_brief(&state, auth.user_id).await?;
        events::log_fanout_error(
            "member_joined",
            events::deliver_member_joined(&state, family.id, joined).await,
        );
        return Ok((StatusCode::OK, Json(json!({"status": "joined"}))).into_response());
    }

    // approval policy: create a pending request; the partial unique index
    // (family_id, user_id) WHERE pending turns a duplicate into a 409.
    let inserted = sqlx::query(
        "INSERT INTO join_requests (family_id, user_id, invite_code_used) VALUES ($1, $2, $3)",
    )
    .bind(family.id)
    .bind(auth.user_id)
    .bind(&invite_code)
    .execute(&state.pool)
    .await;
    match inserted {
        Ok(_) => {
            // The owner learns about the request by push when offline; the
            // request itself is already committed, so failures here must
            // not fail the response.
            events::log_fanout_error(
                "join_request_push",
                events::push_join_request_created(
                    &state,
                    family.id,
                    &family.name,
                    family.owner_user_id,
                    auth.user_id,
                )
                .await,
            );
            Ok((StatusCode::OK, Json(json!({"status": "pending"}))).into_response())
        }
        Err(sqlx::Error::Database(db_err))
            if db_err.constraint() == Some("join_requests_pending_uq") =>
        {
            Err(ApiError::conflict(
                codes::JOIN_REQUEST_PENDING,
                "a join request for this family is already pending",
            ))
        }
        Err(err) => Err(err.into()),
    }
}

/// `GET /families/mine`
pub async fn my_family(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Response, ApiError> {
    let Some(family_id) = auth.family_id else {
        return Err(ApiError::not_found(
            codes::NOT_IN_FAMILY,
            "you do not belong to a family",
        ));
    };
    let family = fetch_family(&state, family_id).await?;
    let rows = sqlx::query(
        "SELECT id, username, display_name, avatar_version
         FROM users WHERE family_id = $1 ORDER BY id",
    )
    .bind(family_id)
    .fetch_all(&state.pool)
    .await?;
    let members: Vec<Member> = rows
        .iter()
        .map(|row| {
            let id: i64 = row.get("id");
            Member {
                id,
                username: row.get("username"),
                display_name: row.get("display_name"),
                role: member_role(id, family.owner_user_id).to_string(),
                avatar_version: row.get("avatar_version"),
            }
        })
        .collect();
    let is_owner = family.owner_user_id == auth.user_id;
    // The board cursor rides along: this is the call every client already
    // makes on resync, so learning whether a board catch-up is needed
    // costs no extra request. Omitted while the board has never been
    // written to, like max_reaction_seq on an unreacted chat.
    let last_board_seq: i64 =
        sqlx::query_scalar("SELECT last_board_seq FROM families WHERE id = $1")
            .bind(family.id)
            .fetch_one(&state.pool)
            .await?;
    let mut body = json!({"family": family.to_api(is_owner), "members": members});
    if last_board_seq > 0 {
        body["max_board_seq"] = json!(last_board_seq);
    }
    Ok((StatusCode::OK, Json(body)).into_response())
}

/// `POST /families/invite-code/rotate` (owner)
pub async fn rotate_invite_code(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Response, ApiError> {
    let family = require_owner(&state, &auth).await?;
    for _attempt in 0..5 {
        let invite_code = tokens::gen_invite_code();
        let updated = sqlx::query("UPDATE families SET invite_code = $1 WHERE id = $2")
            .bind(&invite_code)
            .bind(family.id)
            .execute(&state.pool)
            .await;
        match updated {
            Ok(_) => {
                return Ok(
                    (StatusCode::OK, Json(json!({"invite_code": invite_code}))).into_response()
                );
            }
            Err(sqlx::Error::Database(db_err))
                if db_err.constraint() == Some("families_invite_code_uq") =>
            {
                continue;
            }
            Err(err) => return Err(err.into()),
        }
    }
    Err(ApiError::Internal(anyhow::anyhow!(
        "could not generate a unique invite code after 5 attempts"
    )))
}

/// `PATCH /families/mine` (owner) — change the join policy.
pub async fn patch_family(
    auth: AuthUser,
    State(state): State<AppState>,
    AppJson(req): AppJson<PatchFamilyRequest>,
) -> Result<Response, ApiError> {
    if req.join_policy != "open" && req.join_policy != "approval" {
        return Err(ApiError::validation(
            "join_policy must be \"open\" or \"approval\"",
        ));
    }
    let family = require_owner(&state, &auth).await?;
    let row = sqlx::query(
        "UPDATE families SET join_policy = $1 WHERE id = $2
         RETURNING id, name, invite_code, join_policy, owner_user_id, created_at",
    )
    .bind(&req.join_policy)
    .bind(family.id)
    .fetch_one(&state.pool)
    .await?;
    let family = FamilyRecord::from_row(&row);
    Ok((StatusCode::OK, Json(json!({"family": family.to_api(true)}))).into_response())
}

/// `GET /families/join-requests` (owner) — pending only.
pub async fn list_join_requests(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Response, ApiError> {
    let family = require_owner(&state, &auth).await?;
    let rows = sqlx::query(
        "SELECT jr.id, jr.created_at,
                u.id AS user_id, u.username, u.display_name, u.avatar_version,
                u.created_at AS user_created_at
         FROM join_requests jr
         JOIN users u ON u.id = jr.user_id
         WHERE jr.family_id = $1 AND jr.status = 'pending'
         ORDER BY jr.id",
    )
    .bind(family.id)
    .fetch_all(&state.pool)
    .await?;
    let requests: Vec<JoinRequest> = rows
        .iter()
        .map(|row| JoinRequest {
            id: row.get("id"),
            user: User {
                id: row.get("user_id"),
                username: row.get("username"),
                display_name: row.get("display_name"),
                created_at: row.get("user_created_at"),
                avatar_version: row.get("avatar_version"),
            },
            created_at: row.get("created_at"),
        })
        .collect();
    Ok((StatusCode::OK, Json(json!({"requests": requests}))).into_response())
}

/// `POST /families/join-requests/{id}/approve` (owner)
pub async fn approve_join_request(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(request_id): Path<i64>,
) -> Result<Response, ApiError> {
    let family = require_owner(&state, &auth).await?;

    let mut tx = state.pool.begin().await?;
    // FOR UPDATE serializes concurrent decisions on the same request.
    let row = sqlx::query(
        "SELECT user_id, status FROM join_requests
         WHERE id = $1 AND family_id = $2
         FOR UPDATE",
    )
    .bind(request_id)
    .bind(family.id)
    .fetch_optional(&mut *tx)
    .await?;
    // A request that does not exist (for this family) is indistinguishable
    // from one already decided — same code either way.
    let not_pending = || {
        ApiError::conflict(
            codes::JOIN_REQUEST_NOT_PENDING,
            "join request is not pending",
        )
    };
    let Some(row) = row else {
        return Err(not_pending());
    };
    let status: String = row.get("status");
    if status != "pending" {
        return Err(not_pending());
    }
    let applicant_id: i64 = row.get("user_id");

    if !grant_membership(&mut tx, family.id, applicant_id).await? {
        // The applicant found a family elsewhere in the meantime. The
        // request can never be satisfied, so it is closed as rejected —
        // committed, not rolled back — before reporting the conflict.
        sqlx::query(
            "UPDATE join_requests SET status = 'rejected', decided_at = now(), decided_by = $2
             WHERE id = $1",
        )
        .bind(request_id)
        .bind(auth.user_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        return Err(ApiError::conflict(
            codes::USER_ALREADY_IN_FAMILY,
            "this user already belongs to a family",
        ));
    }
    // grant_membership auto-rejected the applicant's *other* pending
    // requests; this one becomes the approved record.
    sqlx::query(
        "UPDATE join_requests SET status = 'approved', decided_at = now(), decided_by = $2
         WHERE id = $1",
    )
    .bind(request_id)
    .bind(auth.user_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    let joined = user_brief(&state, applicant_id).await?;
    let member = Member {
        id: joined.id,
        username: joined.username.clone(),
        display_name: joined.display_name.clone(),
        role: "member".to_string(),
        avatar_version: joined.avatar_version,
    };
    events::log_fanout_error(
        "member_joined",
        events::deliver_member_joined(&state, family.id, joined).await,
    );
    // An offline requester learns they're in by push ("joined").
    events::log_fanout_error(
        "join_approved_push",
        events::push_join_approved(&state, family.id, &family.name, applicant_id).await,
    );
    Ok((StatusCode::OK, Json(json!({"member": member}))).into_response())
}

/// `POST /families/join-requests/{id}/reject` (owner)
pub async fn reject_join_request(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(request_id): Path<i64>,
) -> Result<Response, ApiError> {
    let family = require_owner(&state, &auth).await?;
    let mut tx = state.pool.begin().await?;
    let status: Option<String> = sqlx::query_scalar(
        "SELECT status FROM join_requests WHERE id = $1 AND family_id = $2 FOR UPDATE",
    )
    .bind(request_id)
    .bind(family.id)
    .fetch_optional(&mut *tx)
    .await?;
    if status.as_deref() != Some("pending") {
        return Err(ApiError::conflict(
            codes::JOIN_REQUEST_NOT_PENDING,
            "join request is not pending",
        ));
    }
    sqlx::query(
        "UPDATE join_requests SET status = 'rejected', decided_at = now(), decided_by = $2
         WHERE id = $1",
    )
    .bind(request_id)
    .bind(auth.user_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `POST /families/leave`
pub async fn leave_family(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Response, ApiError> {
    let Some(family_id) = auth.family_id else {
        return Err(ApiError::conflict(
            codes::NOT_IN_FAMILY,
            "you do not belong to a family",
        ));
    };
    let family = fetch_family(&state, family_id).await?;

    if family.owner_user_id == auth.user_id {
        let member_count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM users WHERE family_id = $1")
                .bind(family_id)
                .fetch_one(&state.pool)
                .await?;
        if member_count > 1 {
            return Err(ApiError::conflict(
                codes::OWNER_CANNOT_LEAVE,
                "the owner may leave only as the sole member",
            ));
        }
        // Sole member: delete the family. Cascades remove chats, messages,
        // members, and join requests; users.family_id resets via
        // ON DELETE SET NULL. Nobody is left to notify.
        sqlx::query("DELETE FROM families WHERE id = $1")
            .bind(family_id)
            .execute(&state.pool)
            .await?;
        return Ok(StatusCode::NO_CONTENT.into_response());
    }

    remove_membership(&state, family_id, auth.user_id).await?;
    events::log_fanout_error(
        "member_left",
        events::deliver_member_left(&state, family_id, auth.user_id).await,
    );
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `DELETE /families/members/{user_id}` (owner)
pub async fn remove_member(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(target_user_id): Path<i64>,
) -> Result<Response, ApiError> {
    let family = require_owner(&state, &auth).await?;
    // The caller is the owner, so "target is the owner" == "target is me".
    if target_user_id == auth.user_id {
        return Err(ApiError::conflict(
            codes::CANNOT_REMOVE_OWNER,
            "the owner cannot be removed from the family",
        ));
    }
    let target_family: Option<i64> =
        sqlx::query_scalar("SELECT family_id FROM users WHERE id = $1")
            .bind(target_user_id)
            .fetch_optional(&state.pool)
            .await?
            .flatten();
    if target_family != Some(family.id) {
        return Err(ApiError::not_found(
            codes::USER_NOT_FOUND,
            "no such member in your family",
        ));
    }

    remove_membership(&state, family.id, target_user_id).await?;
    events::log_fanout_error(
        "member_left",
        events::deliver_member_left(&state, family.id, target_user_id).await,
    );
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// Shared leave/remove write: detach the user and drop them from every chat
/// of the family (family chat + their direct chats). Chat and message rows
/// stay — history is retained and resurfaces on rejoin per protocol.md.
async fn remove_membership(state: &AppState, family_id: i64, user_id: i64) -> Result<(), ApiError> {
    let mut tx = state.pool.begin().await?;
    let updated = sqlx::query("UPDATE users SET family_id = NULL WHERE id = $1 AND family_id = $2")
        .bind(user_id)
        .bind(family_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if updated == 0 {
        // They already left in a parallel request; nothing to undo.
        return Err(ApiError::conflict(
            codes::NOT_IN_FAMILY,
            "the user no longer belongs to this family",
        ));
    }
    sqlx::query(
        "DELETE FROM chat_members cm
         USING chats c
         WHERE cm.chat_id = c.id AND c.family_id = $1 AND cm.user_id = $2",
    )
    .bind(family_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

fn member_role(user_id: i64, owner_user_id: i64) -> &'static str {
    if user_id == owner_user_id {
        "owner"
    } else {
        "member"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn member_role_is_owner_only_for_the_owner_user_id() {
        assert_eq!(member_role(7, 7), "owner");
        assert_eq!(member_role(9, 7), "member");
    }
}
