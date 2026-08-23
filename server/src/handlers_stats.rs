//! Family statistics (docs/protocol.md, "Family statistics").
//!
//! Every member sees the same numbers — it is a shared curiosity, not an
//! owner's dashboard.
//!
//! Everything here counts what is STILL HERE. Retention sweeps messages past
//! `retention_days` and takes their attachments with them, so these are the
//! family's current history rather than everything it has ever said. A
//! member who has left keeps their rows and so keeps appearing, which is
//! deliberate: their messages are still in the thread.

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::{Json, http::StatusCode};
use serde_json::json;
use sqlx::Row;

use crate::auth::AuthUser;
use crate::error::ApiError;
use crate::state::AppState;

/// `GET /families/mine/stats`
pub async fn family_stats(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Response, ApiError> {
    // Same rule every family endpoint follows: no family, nothing to count.
    let family_id = sqlx::query("SELECT family_id FROM users WHERE id = $1")
        .bind(auth.user_id)
        .fetch_optional(&state.pool)
        .await?
        .and_then(|row| row.get::<Option<i64>, _>("family_id"))
        .ok_or_else(|| {
            ApiError::conflict(
                crate::error::codes::NOT_IN_FAMILY,
                "you are not in a family",
            )
        })?;

    // Every SUM is cast to BIGINT: PostgreSQL widens SUM() over a bigint to
    // NUMERIC, which does not decode to i64 and fails at the row read
    // rather than at the query.
    //
    // Per member, in one pass. LEFT JOINs so a member who has sent nothing
    // still appears with zeros rather than vanishing — "who has sent the
    // fewest" is as interesting a question as the opposite.
    //
    // The attachment aggregates are a SUBQUERY rather than another join:
    // joining messages and attachments together multiplies the message
    // count by the number of attachments, which is the classic way to get
    // confidently wrong totals.
    let members = sqlx::query(
        "SELECT u.id, u.display_name,
                COALESCE(m.count, 0)            AS messages,
                COALESCE(a.count, 0)            AS att_count,
                COALESCE(a.bytes, 0)            AS att_bytes,
                COALESCE(a.photo, 0)            AS att_photo,
                COALESCE(a.video, 0)            AS att_video,
                COALESCE(a.audio, 0)            AS att_audio,
                COALESCE(a.file, 0)             AS att_file,
                COALESCE(ai.questions, 0)       AS ai_questions,
                COALESCE(ai.prompt_tokens, 0)   AS ai_prompt_tokens,
                COALESCE(ai.completion_tokens, 0) AS ai_completion_tokens
         FROM users u
         LEFT JOIN (
             SELECT msg.sender_id, COUNT(*) AS count
             FROM messages msg
             JOIN chats c ON c.id = msg.chat_id
             WHERE c.family_id = $1
             GROUP BY msg.sender_id
         ) m ON m.sender_id = u.id
         LEFT JOIN (
             SELECT att.uploader_id,
                    COUNT(*) AS count,
                    SUM(att.size_bytes)::BIGINT AS bytes,
                    COUNT(*) FILTER (WHERE att.kind = 'photo') AS photo,
                    COUNT(*) FILTER (WHERE att.kind = 'video') AS video,
                    COUNT(*) FILTER (WHERE att.kind = 'audio') AS audio,
                    COUNT(*) FILTER (WHERE att.kind = 'file')  AS file
             FROM attachments att
             JOIN messages msg ON msg.id = att.message_id
             JOIN chats c ON c.id = msg.chat_id
             WHERE c.family_id = $1
             GROUP BY att.uploader_id
         ) a ON a.uploader_id = u.id
         LEFT JOIN (
             SELECT user_id,
                    COUNT(*) AS questions,
                    SUM(prompt_tokens)::BIGINT AS prompt_tokens,
                    SUM(completion_tokens)::BIGINT AS completion_tokens
             FROM ai_usage
             WHERE family_id = $1
             GROUP BY user_id
         ) ai ON ai.user_id = u.id
         WHERE u.family_id = $1
         ORDER BY messages DESC, u.display_name ASC",
    )
    .bind(family_id)
    .fetch_all(&state.pool)
    .await?;

    let member_rows: Vec<serde_json::Value> = members
        .iter()
        .map(|row| {
            json!({
                "user_id": row.get::<i64, _>("id"),
                "display_name": row.get::<String, _>("display_name"),
                "messages": row.get::<i64, _>("messages"),
                "attachments": {
                    "count": row.get::<i64, _>("att_count"),
                    "bytes": row.get::<i64, _>("att_bytes"),
                    "photo": row.get::<i64, _>("att_photo"),
                    "video": row.get::<i64, _>("att_video"),
                    "audio": row.get::<i64, _>("att_audio"),
                    "file":  row.get::<i64, _>("att_file"),
                },
                "ai": {
                    "questions": row.get::<i64, _>("ai_questions"),
                    "prompt_tokens": row.get::<i64, _>("ai_prompt_tokens"),
                    "completion_tokens": row.get::<i64, _>("ai_completion_tokens"),
                },
            })
        })
        .collect();

    // Family totals. Summed in SQL rather than from the rows above, because
    // `stored_bytes` cannot be: identical bytes are stored once per family,
    // so counting each DISTINCT file once is a different question from
    // adding up what each member sent — and the gap between them is exactly
    // what dedup saved.
    let totals = sqlx::query(
        "SELECT
            (SELECT COUNT(*) FROM users WHERE family_id = $1) AS members,
            (SELECT COUNT(*) FROM messages msg
                JOIN chats c ON c.id = msg.chat_id
                WHERE c.family_id = $1) AS messages,
            (SELECT COUNT(*) FROM notes WHERE family_id = $1 AND deleted_at IS NULL)
                AS board_notes,
            (SELECT COUNT(*) FROM attachments att
                JOIN messages msg ON msg.id = att.message_id
                JOIN chats c ON c.id = msg.chat_id
                WHERE c.family_id = $1) AS att_count,
            (SELECT COALESCE(SUM(att.size_bytes), 0)::BIGINT FROM attachments att
                JOIN messages msg ON msg.id = att.message_id
                JOIN chats c ON c.id = msg.chat_id
                WHERE c.family_id = $1) AS att_bytes,
            (SELECT COALESCE(SUM(size_bytes), 0)::BIGINT FROM (
                SELECT DISTINCT ON (att.storage_key) att.storage_key, att.size_bytes
                FROM attachments att
                JOIN messages msg ON msg.id = att.message_id
                JOIN chats c ON c.id = msg.chat_id
                WHERE c.family_id = $1
             ) distinct_files) AS stored_bytes,
            (SELECT COUNT(*) FROM attachments att
                JOIN messages msg ON msg.id = att.message_id
                JOIN chats c ON c.id = msg.chat_id
                WHERE c.family_id = $1 AND att.kind = 'photo') AS att_photo,
            (SELECT COUNT(*) FROM attachments att
                JOIN messages msg ON msg.id = att.message_id
                JOIN chats c ON c.id = msg.chat_id
                WHERE c.family_id = $1 AND att.kind = 'video') AS att_video,
            (SELECT COUNT(*) FROM attachments att
                JOIN messages msg ON msg.id = att.message_id
                JOIN chats c ON c.id = msg.chat_id
                WHERE c.family_id = $1 AND att.kind = 'audio') AS att_audio,
            (SELECT COUNT(*) FROM attachments att
                JOIN messages msg ON msg.id = att.message_id
                JOIN chats c ON c.id = msg.chat_id
                WHERE c.family_id = $1 AND att.kind = 'file') AS att_file,
            (SELECT COUNT(*) FROM ai_usage WHERE family_id = $1) AS ai_questions,
            (SELECT COALESCE(SUM(prompt_tokens), 0)::BIGINT FROM ai_usage WHERE family_id = $1)
                AS ai_prompt_tokens,
            (SELECT COALESCE(SUM(completion_tokens), 0)::BIGINT FROM ai_usage WHERE family_id = $1)
                AS ai_completion_tokens",
    )
    .bind(family_id)
    .fetch_one(&state.pool)
    .await?;

    let body = json!({
        "generated_at": time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default(),
        "totals": {
            "members": totals.get::<i64, _>("members"),
            "messages": totals.get::<i64, _>("messages"),
            "board_notes": totals.get::<i64, _>("board_notes"),
            "attachments": {
                "count": totals.get::<i64, _>("att_count"),
                "bytes": totals.get::<i64, _>("att_bytes"),
                // Each distinct file once: the gap from `bytes` is what
                // one-copy-per-family saved.
                "stored_bytes": totals.get::<i64, _>("stored_bytes"),
                "photo": totals.get::<i64, _>("att_photo"),
                "video": totals.get::<i64, _>("att_video"),
                "audio": totals.get::<i64, _>("att_audio"),
                "file":  totals.get::<i64, _>("att_file"),
            },
            "ai": {
                "questions": totals.get::<i64, _>("ai_questions"),
                "prompt_tokens": totals.get::<i64, _>("ai_prompt_tokens"),
                "completion_tokens": totals.get::<i64, _>("ai_completion_tokens"),
            },
        },
        "members": member_rows,
    });

    Ok((StatusCode::OK, Json(body)).into_response())
}
