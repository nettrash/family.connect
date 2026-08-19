//! Chat and message endpoints, plus the message-write core shared with the
//! WebSocket path.
//!
//! `create_message` and `apply_read_marker` are the single implementations
//! of "post a message" and "advance a read marker" — REST handlers and WS
//! frame handlers both call them, so validation, dedup, and monotonicity
//! semantics cannot drift between transports. Idempotency is enforced by
//! the database (`ON CONFLICT ... DO NOTHING` on the dedup index), never by
//! an application-level existence check.

use std::collections::HashMap;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::error::{ApiError, AppJson, codes};
use crate::events;
use crate::models::{Chat, ChatListEntry, Message};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct DirectChatRequest {
    pub user_id: i64,
}

#[derive(Debug, Deserialize)]
pub struct PostMessageRequest {
    pub client_msg_id: Uuid,
    pub body: String,
}

#[derive(Debug, Deserialize)]
pub struct ReadRequest {
    pub last_read_message_id: i64,
}

/// Order a user-id pair for the `direct` chat shape constraint
/// (`user_a_id < user_b_id`), making (a,b) and (b,a) the same chat.
pub fn ordered_pair(a: i64, b: i64) -> (i64, i64) {
    if a < b { (a, b) } else { (b, a) }
}

/// Clamp a requested page size into `[1, max_limit]`, defaulting when the
/// client sent none.
pub fn clamp_limit(requested: Option<i64>, default_limit: i64, max_limit: i64) -> i64 {
    requested.unwrap_or(default_limit).clamp(1, max_limit)
}

/// Verify the chat exists and the caller is a member.
///
/// 404 when the chat does not exist at all, 403 when it exists but the
/// caller is not in it — a member that left sees 403, not a phantom 404.
pub async fn ensure_chat_access(
    state: &AppState,
    chat_id: i64,
    user_id: i64,
) -> Result<(), ApiError> {
    let row = sqlx::query(
        "SELECT (cm.user_id IS NOT NULL) AS is_member
         FROM chats c
         LEFT JOIN chat_members cm ON cm.chat_id = c.id AND cm.user_id = $2
         WHERE c.id = $1",
    )
    .bind(chat_id)
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await?;
    match row {
        None => Err(ApiError::not_found(codes::CHAT_NOT_FOUND, "no such chat")),
        Some(row) if !row.get::<bool, _>("is_member") => Err(ApiError::forbidden(
            codes::NOT_CHAT_MEMBER,
            "you are not a member of this chat",
        )),
        Some(_) => Ok(()),
    }
}

/// Insert a message, idempotently.
///
/// Returns `(message, created)`: `created == false` means the
/// `(chat, sender, client_msg_id)` triple already existed and the original
/// message is returned — the caller must not fan out again.
pub async fn create_message(
    state: &AppState,
    chat_id: i64,
    sender_id: i64,
    client_msg_id: Uuid,
    body: &str,
) -> Result<(Message, bool), ApiError> {
    ensure_chat_access(state, chat_id, sender_id).await?;

    let body = body.trim();
    if body.is_empty() {
        return Err(ApiError::bad_request(
            codes::MESSAGE_EMPTY,
            "message body is empty",
        ));
    }
    let max_chars = state.cfg.limits.max_message_chars;
    if body.chars().count() > max_chars {
        return Err(ApiError::bad_request(
            codes::MESSAGE_TOO_LONG,
            format!("message body exceeds {max_chars} characters"),
        ));
    }

    let inserted = sqlx::query(
        "INSERT INTO messages (chat_id, sender_id, client_msg_id, body)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (chat_id, sender_id, client_msg_id) DO NOTHING
         RETURNING id, chat_id, sender_id, client_msg_id, body, created_at",
    )
    .bind(chat_id)
    .bind(sender_id)
    .bind(client_msg_id)
    .bind(body)
    .fetch_optional(&state.pool)
    .await?;

    if let Some(row) = inserted {
        return Ok((Message::from_row(&row), true));
    }

    // Dedup hit: a retry of a message that already landed. Return the
    // original — possibly with a different body if the client mutated the
    // retry, which is the client's bug; the first write wins.
    let row = sqlx::query(
        "SELECT id, chat_id, sender_id, client_msg_id, body, created_at
         FROM messages
         WHERE chat_id = $1 AND sender_id = $2 AND client_msg_id = $3",
    )
    .bind(chat_id)
    .bind(sender_id)
    .bind(client_msg_id)
    .fetch_one(&state.pool)
    .await?;
    Ok((Message::from_row(&row), false))
}

/// Advance a read marker, monotonically (the max ever reported wins).
/// Returns the effective marker after the write.
pub async fn apply_read_marker(
    state: &AppState,
    chat_id: i64,
    user_id: i64,
    last_read_message_id: i64,
) -> Result<i64, ApiError> {
    ensure_chat_access(state, chat_id, user_id).await?;
    if last_read_message_id < 0 {
        return Err(ApiError::validation("last_read_message_id must be >= 0"));
    }
    let effective: i64 = sqlx::query_scalar(
        "INSERT INTO chat_reads (chat_id, user_id, last_read_message_id)
         VALUES ($1, $2, $3)
         ON CONFLICT (chat_id, user_id) DO UPDATE
         SET last_read_message_id =
                 GREATEST(chat_reads.last_read_message_id, EXCLUDED.last_read_message_id),
             updated_at = now()
         RETURNING last_read_message_id",
    )
    .bind(chat_id)
    .bind(user_id)
    .bind(last_read_message_id)
    .fetch_one(&state.pool)
    .await?;
    Ok(effective)
}

/// `GET /chats` — every chat the caller is a member of, with a last-message
/// preview and the authoritative unread count.
pub async fn list_chats(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Response, ApiError> {
    let rows = sqlx::query(
        "SELECT c.id AS chat_id, c.kind,
                f.name AS family_name,
                CASE WHEN c.kind = 'direct' THEN
                    CASE WHEN c.user_a_id = $1 THEN c.user_b_id ELSE c.user_a_id END
                END AS peer_user_id,
                pu.display_name AS peer_display_name,
                lm.id AS last_id, lm.sender_id AS last_sender_id,
                lm.client_msg_id AS last_client_msg_id, lm.body AS last_body,
                lm.created_at AS last_created_at,
                uc.unread AS unread_count
         FROM chat_members m
         JOIN chats c ON c.id = m.chat_id
         JOIN families f ON f.id = c.family_id
         LEFT JOIN users pu
                ON c.kind = 'direct'
               AND pu.id = CASE WHEN c.user_a_id = $1 THEN c.user_b_id ELSE c.user_a_id END
         LEFT JOIN chat_reads cr ON cr.chat_id = c.id AND cr.user_id = $1
         LEFT JOIN LATERAL (
             SELECT id, sender_id, client_msg_id, body, created_at
             FROM messages WHERE chat_id = c.id ORDER BY id DESC LIMIT 1
         ) lm ON TRUE
         LEFT JOIN LATERAL (
             SELECT count(*) AS unread
             FROM messages msg
             WHERE msg.chat_id = c.id
               AND msg.id > COALESCE(cr.last_read_message_id, 0)
               AND msg.sender_id <> $1
         ) uc ON TRUE
         WHERE m.user_id = $1
         ORDER BY COALESCE(lm.id, 0) DESC, c.id",
    )
    .bind(auth.user_id)
    .fetch_all(&state.pool)
    .await?;

    let chats: Vec<ChatListEntry> = rows
        .iter()
        .map(|row| {
            let kind: String = row.get("kind");
            let peer_user_id: Option<i64> = row.get("peer_user_id");
            let title: String = if kind == "family" {
                row.get("family_name")
            } else {
                row.get("peer_display_name")
            };
            let chat_id: i64 = row.get("chat_id");
            let last_message = row.get::<Option<i64>, _>("last_id").map(|last_id| Message {
                id: last_id,
                chat_id,
                sender_id: row.get("last_sender_id"),
                client_msg_id: row.get("last_client_msg_id"),
                body: row.get("last_body"),
                created_at: row.get("last_created_at"),
            });
            ChatListEntry {
                chat: Chat {
                    id: chat_id,
                    kind,
                    title,
                    peer_user_id,
                },
                last_message,
                unread_count: row.get("unread_count"),
            }
        })
        .collect();

    Ok((StatusCode::OK, Json(json!({"chats": chats}))).into_response())
}

/// `POST /chats/direct` — get-or-create the direct chat with another family
/// member. Idempotent by the `(user_a, user_b)` unique index; re-creating
/// after a leave/rejoin also re-attaches both members.
pub async fn direct_chat(
    auth: AuthUser,
    State(state): State<AppState>,
    AppJson(req): AppJson<DirectChatRequest>,
) -> Result<Response, ApiError> {
    if req.user_id == auth.user_id {
        return Err(ApiError::bad_request(
            codes::CANNOT_DM_SELF,
            "cannot open a direct chat with yourself",
        ));
    }
    let Some(family_id) = auth.family_id else {
        // Without a family there is no shared family to be in.
        return Err(ApiError::conflict(
            codes::NOT_IN_FAMILY,
            "you do not belong to a family",
        ));
    };
    let peer = sqlx::query("SELECT id, family_id, display_name FROM users WHERE id = $1")
        .bind(req.user_id)
        .fetch_optional(&state.pool)
        .await?;
    let Some(peer) = peer else {
        return Err(ApiError::not_found(codes::USER_NOT_FOUND, "no such user"));
    };
    if peer.get::<Option<i64>, _>("family_id") != Some(family_id) {
        return Err(ApiError::conflict(
            codes::NOT_SAME_FAMILY,
            "this user is not in your family",
        ));
    }

    let (user_a, user_b) = ordered_pair(auth.user_id, req.user_id);
    let mut tx = state.pool.begin().await?;
    // On conflict the family_id is refreshed: a pair that left family X and
    // reunited in family Y keeps one chat (and its history) across moves.
    let chat_id: i64 = sqlx::query_scalar(
        "INSERT INTO chats (family_id, kind, user_a_id, user_b_id)
         VALUES ($1, 'direct', $2, $3)
         ON CONFLICT (user_a_id, user_b_id) WHERE kind = 'direct'
         DO UPDATE SET family_id = EXCLUDED.family_id
         RETURNING id",
    )
    .bind(family_id)
    .bind(user_a)
    .bind(user_b)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO chat_members (chat_id, user_id)
         VALUES ($1, $2), ($1, $3)
         ON CONFLICT DO NOTHING",
    )
    .bind(chat_id)
    .bind(user_a)
    .bind(user_b)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    let chat = Chat {
        id: chat_id,
        kind: "direct".to_string(),
        title: peer.get("display_name"),
        peer_user_id: Some(req.user_id),
    };
    Ok((StatusCode::OK, Json(json!({"chat": chat}))).into_response())
}

/// `GET /chats/{id}/messages` — keyset pagination on the message id.
pub async fn get_messages(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(chat_id): Path<i64>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, ApiError> {
    ensure_chat_access(&state, chat_id, auth.user_id).await?;

    let before_id = parse_pagination_param(&params, "before_id")?;
    let after_id = parse_pagination_param(&params, "after_id")?;
    let requested_limit = parse_pagination_param(&params, "limit")?;
    if before_id.is_some() && after_id.is_some() {
        return Err(ApiError::bad_request(
            codes::INVALID_PAGINATION,
            "before_id and after_id are mutually exclusive",
        ));
    }
    let limit = clamp_limit(
        requested_limit,
        state.cfg.limits.default_page_size,
        state.cfg.limits.max_page_size,
    );

    const COLS: &str = "id, chat_id, sender_id, client_msg_id, body, created_at";
    let rows = if let Some(before) = before_id {
        // History paging: strictly older, newest first.
        sqlx::query(&format!(
            "SELECT {COLS} FROM messages WHERE chat_id = $1 AND id < $2 ORDER BY id DESC LIMIT $3"
        ))
        .bind(chat_id)
        .bind(before)
        .bind(limit)
        .fetch_all(&state.pool)
        .await?
    } else if let Some(after) = after_id {
        // Reconnect catch-up: strictly newer, oldest first.
        sqlx::query(&format!(
            "SELECT {COLS} FROM messages WHERE chat_id = $1 AND id > $2 ORDER BY id ASC LIMIT $3"
        ))
        .bind(chat_id)
        .bind(after)
        .bind(limit)
        .fetch_all(&state.pool)
        .await?
    } else {
        sqlx::query(&format!(
            "SELECT {COLS} FROM messages WHERE chat_id = $1 ORDER BY id DESC LIMIT $2"
        ))
        .bind(chat_id)
        .bind(limit)
        .fetch_all(&state.pool)
        .await?
    };

    let messages: Vec<Message> = rows.iter().map(Message::from_row).collect();
    Ok((StatusCode::OK, Json(json!({"messages": messages}))).into_response())
}

/// `POST /chats/{id}/messages` — 201 on first write, 200 on a dedup retry.
pub async fn post_message(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(chat_id): Path<i64>,
    AppJson(req): AppJson<PostMessageRequest>,
) -> Result<Response, ApiError> {
    let (message, created) =
        create_message(&state, chat_id, auth.user_id, req.client_msg_id, &req.body).await?;
    if created {
        // REST has no originating WS connection: every connection of every
        // member — the sender's own devices included — receives `message`;
        // this HTTP response is the sender's ack.
        events::log_fanout_error(
            "new_message",
            events::deliver_new_message(&state, &message, None).await,
        );
    }
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(json!({"message": message}))).into_response())
}

/// `POST /chats/{id}/read`
pub async fn mark_read(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(chat_id): Path<i64>,
    AppJson(req): AppJson<ReadRequest>,
) -> Result<Response, ApiError> {
    let effective =
        apply_read_marker(&state, chat_id, auth.user_id, req.last_read_message_id).await?;
    // Relay the effective (post-GREATEST) marker so receivers never observe
    // a regression even when the client reported a stale value.
    events::log_fanout_error(
        "read",
        events::deliver_read(&state, chat_id, auth.user_id, effective).await,
    );
    Ok(StatusCode::NO_CONTENT.into_response())
}

fn parse_pagination_param(
    params: &HashMap<String, String>,
    name: &'static str,
) -> Result<Option<i64>, ApiError> {
    match params.get(name) {
        None => Ok(None),
        Some(raw) => raw.parse::<i64>().map(Some).map_err(|_| {
            ApiError::bad_request(
                codes::INVALID_PAGINATION,
                format!("{name} must be an integer"),
            )
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordered_pair_sorts_both_argument_orders_the_same_way() {
        assert_eq!(ordered_pair(2, 9), (2, 9));
        assert_eq!(ordered_pair(9, 2), (2, 9));
    }

    #[test]
    fn clamp_limit_defaults_and_clamps_into_range() {
        assert_eq!(clamp_limit(None, 50, 200), 50, "default when unset");
        assert_eq!(
            clamp_limit(Some(75), 50, 200),
            75,
            "in-range passes through"
        );
        assert_eq!(clamp_limit(Some(1000), 50, 200), 200, "clamped to max");
        assert_eq!(clamp_limit(Some(0), 50, 200), 1, "floor of 1");
        assert_eq!(clamp_limit(Some(-5), 50, 200), 1, "negatives floor to 1");
    }

    #[test]
    fn pagination_params_parse_integers_and_reject_garbage() {
        let mut params = HashMap::new();
        params.insert("before_id".to_string(), "42".to_string());
        params.insert("limit".to_string(), "abc".to_string());
        assert_eq!(
            parse_pagination_param(&params, "before_id").expect("parses"),
            Some(42)
        );
        assert_eq!(
            parse_pagination_param(&params, "after_id").expect("absent is fine"),
            None
        );
        assert!(parse_pagination_param(&params, "limit").is_err());
    }
}
