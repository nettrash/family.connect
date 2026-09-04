//! Reporting a member to the family owner (docs/protocol.md, "Reporting a
//! member").
//!
//! The owner is the moderator. That follows from the containment this
//! product already has: invite-code-only membership, approval by default,
//! no cross-family contact, no user directory, and an owner who can
//! already remove a member, reset their password, rotate the code and
//! close the family.
//!
//! Blocking and reporting are independent. Reporting does not block,
//! blocking does not report, and the owner's inbox NEVER shows who blocked
//! whom — a family owner is often a parent and the blocked person is often
//! in the same house.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;
use sqlx::Row;
use sqlx::postgres::PgRow;

use crate::auth::AuthUser;
use crate::error::{ApiError, AppJson, codes};
use crate::events;
use crate::models::{Report, User};
use crate::state::AppState;

/// The four reasons, fixed. Free text would mean an owner's inbox can hold
/// a report they cannot read: this is a nine-language product with a
/// nine-language owner, and a report a moderator cannot read is not a
/// report. It is also what lets both clients render a row from string
/// resources rather than shipping untranslated user text.
const REASONS: [&str; 4] = ["spam", "harassment", "inappropriate", "other"];

#[derive(Debug, Deserialize)]
pub struct CreateReportRequest {
    pub reported_user_id: i64,
    pub reason: String,
    #[serde(default)]
    pub message_id: Option<i64>,
}

fn report_from_row(row: &PgRow) -> Report {
    Report {
        id: row.get("id"),
        reporter: User {
            id: row.get("reporter_id"),
            username: row.get("reporter_username"),
            display_name: row.get("reporter_display_name"),
            created_at: row.get("reporter_created_at"),
            avatar_version: row.get("reporter_avatar_version"),
            birthday: None,
            deleted: false,
        },
        reported: User {
            id: row.get("reported_id"),
            username: row.get("reported_username"),
            display_name: row.get("reported_display_name"),
            created_at: row.get("reported_created_at"),
            avatar_version: row.get("reported_avatar_version"),
            birthday: None,
            deleted: false,
        },
        reason: row.get("reason"),
        message_id: row.get("message_id"),
        message_excerpt: row.get("message_excerpt"),
        created_at: row.get("created_at"),
    }
}

const SELECT_REPORT: &str = "SELECT r.id, r.reason, r.message_id, r.message_excerpt, r.created_at,
        rep.id AS reporter_id, rep.username AS reporter_username,
        rep.display_name AS reporter_display_name, rep.created_at AS reporter_created_at,
        rep.avatar_version AS reporter_avatar_version,
        tgt.id AS reported_id, tgt.username AS reported_username,
        tgt.display_name AS reported_display_name, tgt.created_at AS reported_created_at,
        tgt.avatar_version AS reported_avatar_version
   FROM member_reports r
   JOIN users rep ON rep.id = r.reporter_user_id
   JOIN users tgt ON tgt.id = r.reported_user_id";

/// `POST /families/reports`
pub async fn create_report(
    auth: AuthUser,
    State(state): State<AppState>,
    AppJson(req): AppJson<CreateReportRequest>,
) -> Result<Response, ApiError> {
    if !REASONS.contains(&req.reason.as_str()) {
        return Err(ApiError::validation(format!(
            "reason must be one of {}",
            REASONS.join(", ")
        )));
    }
    let Some(family_id) = auth.family_id else {
        return Err(ApiError::conflict(
            codes::NOT_IN_FAMILY,
            "you do not belong to a family",
        ));
    };
    if req.reported_user_id == auth.user_id {
        return Err(ApiError::bad_request(
            codes::CANNOT_REPORT_SELF,
            "you cannot report yourself",
        ));
    }
    let target_family: Option<i64> =
        sqlx::query_scalar("SELECT family_id FROM users WHERE id = $1")
            .bind(req.reported_user_id)
            .fetch_optional(&state.pool)
            .await?
            .flatten();
    if target_family != Some(family_id) {
        return Err(ApiError::forbidden(
            codes::NOT_SAME_FAMILY,
            "no such member in your family",
        ));
    }

    // A named message must be one the caller can SEE and one the reported
    // member WROTE. Anything else — including a real message in a chat the
    // caller is not in — is `message_not_found`, so the endpoint never
    // confirms that an id exists elsewhere, exactly as `reply_to_message_id`
    // does not.
    //
    // The body is frozen here. Everywhere else in this protocol a quotation
    // is recomputed on every read; this is the one place it is stored, for
    // two reasons that both end with the owner opening an empty screen: the
    // author may edit the body away through the ordinary author-only path,
    // which has no time limit, and retention deletes the message outright.
    let excerpt: Option<String> = match req.message_id {
        None => None,
        Some(message_id) => {
            let body: Option<String> = sqlx::query_scalar(
                "SELECT m.body
                   FROM messages m
                   JOIN chat_members cm ON cm.chat_id = m.chat_id AND cm.user_id = $2
                  WHERE m.id = $1 AND m.sender_id = $3",
            )
            .bind(message_id)
            .bind(auth.user_id)
            .bind(req.reported_user_id)
            .fetch_optional(&state.pool)
            .await?;
            let Some(body) = body else {
                return Err(ApiError::not_found(
                    codes::MESSAGE_NOT_FOUND,
                    "no such message",
                ));
            };
            Some(body)
        }
    };

    // A report is identified by (reporter, reported, message_id). Raising
    // one that matches an OPEN row returns that row and creates nothing,
    // whatever `reason` was sent — so a double tap is not two rows in the
    // owner's list, while reporting a SECOND message of the same member IS
    // a second report, which is what a moderator needs to see a pattern.
    // The two partial unique indexes in migration 0029 make this a database
    // fact rather than a race.
    let existing: Option<i64> = match req.message_id {
        Some(message_id) => {
            sqlx::query_scalar(
                "SELECT id FROM member_reports
              WHERE reporter_user_id = $1 AND message_id = $2 AND status = 'open'",
            )
            .bind(auth.user_id)
            .bind(message_id)
            .fetch_optional(&state.pool)
            .await?
        }
        None => {
            sqlx::query_scalar(
                "SELECT id FROM member_reports
              WHERE reporter_user_id = $1 AND reported_user_id = $2
                AND message_id IS NULL AND message_excerpt IS NULL AND status = 'open'",
            )
            .bind(auth.user_id)
            .bind(req.reported_user_id)
            .fetch_optional(&state.pool)
            .await?
        }
    };
    if let Some(id) = existing {
        let row = sqlx::query(&format!("{SELECT_REPORT} WHERE r.id = $1"))
            .bind(id)
            .fetch_one(&state.pool)
            .await?;
        return Ok((
            StatusCode::OK,
            Json(json!({"report": report_from_row(&row)})),
        )
            .into_response());
    }

    let id: i64 = sqlx::query_scalar(
        "INSERT INTO member_reports
             (family_id, reporter_user_id, reported_user_id, message_id, message_excerpt, reason)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING id",
    )
    .bind(family_id)
    .bind(auth.user_id)
    .bind(req.reported_user_id)
    .bind(req.message_id)
    .bind(&excerpt)
    .bind(&req.reason)
    .fetch_one(&state.pool)
    .await?;

    let row = sqlx::query(&format!("{SELECT_REPORT} WHERE r.id = $1"))
        .bind(id)
        .fetch_one(&state.pool)
        .await?;

    // The owner is pushed and NO WebSocket frame is raised, exactly as for
    // a join request: they read the list on their next visit.
    let family = sqlx::query("SELECT name, owner_user_id FROM families WHERE id = $1")
        .bind(family_id)
        .fetch_one(&state.pool)
        .await?;
    events::log_fanout_error(
        "report_push",
        events::push_report_created(
            &state,
            family_id,
            family.get::<String, _>("name").as_str(),
            family.get("owner_user_id"),
            req.reported_user_id,
        )
        .await,
    );

    Ok((
        StatusCode::CREATED,
        Json(json!({"report": report_from_row(&row)})),
    )
        .into_response())
}

/// `GET /families/reports` (owner) — open only, oldest first.
pub async fn list_reports(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Response, ApiError> {
    let family = crate::handlers_family::require_owner_family(&state, &auth).await?;
    // Reports naming the OWNER are not listed. See
    // `events::push_report_created` for why.
    // Capped at the page maximum, oldest first: "a family that has hit the
    // ceiling has a moderation problem rather than a pagination problem"
    // (protocol.md, `GET /families/reports`). The cap is here rather than
    // at creation on purpose — refusing to STORE a report would tell the
    // reporter something about how many others there are, and would lose
    // the evidence a moderator needs most on the worst day.
    let rows = sqlx::query(&format!(
        "{SELECT_REPORT} WHERE r.family_id = $1 AND r.status = 'open'
           AND r.reported_user_id <> $2
         ORDER BY r.id ASC
         LIMIT $3"
    ))
    .bind(family)
    .bind(auth.user_id)
    .bind(state.cfg.limits.max_page_size)
    .fetch_all(&state.pool)
    .await?;
    let reports: Vec<Report> = rows.iter().map(report_from_row).collect();
    Ok((StatusCode::OK, Json(json!({"reports": reports}))).into_response())
}

/// `POST /families/reports/{id}/resolve` (owner)
///
/// Takes it off the list; what "dealt with" MEANS is the owner's business.
/// This protocol has removing a member, resetting a password and closing
/// the family; it does not have deleting somebody else's message.
pub async fn resolve_report(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(report_id): Path<i64>,
) -> Result<Response, ApiError> {
    let family = crate::handlers_family::require_owner_family(&state, &auth).await?;
    let mut tx = state.pool.begin().await?;
    // ONE answer for four conditions — unknown id, another family's report,
    // already resolved, or one naming the owner — so the endpoint never
    // confirms that an id exists elsewhere.
    let not_pending = || ApiError::conflict(codes::REPORT_NOT_PENDING, "report is not pending");
    let row: Option<(String, i64)> = sqlx::query_as(
        "SELECT status, reported_user_id FROM member_reports
          WHERE id = $1 AND family_id = $2 FOR UPDATE",
    )
    .bind(report_id)
    .bind(family)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((status, reported_user_id)) = row else {
        return Err(not_pending());
    };
    if status != "open" || reported_user_id == auth.user_id {
        return Err(not_pending());
    }
    sqlx::query(
        "UPDATE member_reports SET status = 'resolved', resolved_at = now(), resolved_by = $2
          WHERE id = $1",
    )
    .bind(report_id)
    .bind(auth.user_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}
