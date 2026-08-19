//! Event delivery — the single seam between "something happened" and "who
//! hears about it".
//!
//! Both the REST handlers and the WS frame handlers call these functions, so
//! fan-out rules (who gets `message` vs `ack`, who is excluded from `read`/
//! `typing` relays, when the push hook fires) exist exactly once. Push
//! dispatch is `tokio::spawn`ed fire-and-forget: notification delivery must
//! never add latency or failure modes to the message write path.

use sqlx::{PgPool, Row};
use tracing::warn;

use crate::error::ApiError;
use crate::models::{Message, UserBrief};
use crate::push::DevicePush;
use crate::state::AppState;
use crate::ws::ServerFrame;

/// All member user ids of a chat (family chats have explicit rows too).
pub async fn chat_member_ids(pool: &PgPool, chat_id: i64) -> Result<Vec<i64>, ApiError> {
    let ids = sqlx::query_scalar("SELECT user_id FROM chat_members WHERE chat_id = $1")
        .bind(chat_id)
        .fetch_all(pool)
        .await?;
    Ok(ids)
}

/// All current member user ids of a family.
async fn family_member_ids(pool: &PgPool, family_id: i64) -> Result<Vec<i64>, ApiError> {
    let ids = sqlx::query_scalar("SELECT id FROM users WHERE family_id = $1")
        .bind(family_id)
        .fetch_all(pool)
        .await?;
    Ok(ids)
}

/// Fan a freshly inserted message out to every chat member.
///
/// `origin_conn` is the WS connection the `send` frame arrived on, if any —
/// it is skipped because it receives its `ack` directly (a REST post has no
/// origin connection, so all connections, including the sender's own,
/// receive `message`, per protocol.md). Members with no live connection at
/// all are handed to the push seam, except the sender.
pub async fn deliver_new_message(
    state: &AppState,
    message: &Message,
    origin_conn: Option<u64>,
) -> Result<(), ApiError> {
    let members = chat_member_ids(&state.pool, message.chat_id).await?;
    let frame = ServerFrame::Message {
        message: message.clone(),
    };
    let offline = state.registry.fan_out(&members, &frame, origin_conn).await;

    let push_targets: Vec<i64> = offline
        .into_iter()
        .filter(|user_id| *user_id != message.sender_id)
        .collect();
    if push_targets.is_empty() {
        return Ok(());
    }

    let rows = sqlx::query(
        "SELECT id, user_id, platform, push_token
         FROM devices
         WHERE user_id = ANY($1) AND push_token IS NOT NULL",
    )
    .bind(push_targets)
    .fetch_all(&state.pool)
    .await?;
    let devices: Vec<DevicePush> = rows
        .iter()
        .map(|row| DevicePush {
            device_id: row.get("id"),
            user_id: row.get("user_id"),
            platform: row.get("platform"),
            push_token: row.get("push_token"),
        })
        .collect();
    if devices.is_empty() {
        return Ok(());
    }

    // Fire-and-forget: push is best-effort by design in v1.
    let push = state.push.clone();
    let message = message.clone();
    tokio::spawn(async move {
        push.notify_new_message(&devices, &message).await;
    });
    Ok(())
}

/// Relay a read marker to the *other* members of the chat (all connections
/// of the reader are excluded — protocol.md relays read/typing to other
/// members only; the reader's own devices resync over REST).
pub async fn deliver_read(
    state: &AppState,
    chat_id: i64,
    reader_id: i64,
    last_read_message_id: i64,
) -> Result<(), ApiError> {
    let recipients = others(chat_member_ids(&state.pool, chat_id).await?, reader_id);
    let frame = ServerFrame::Read {
        chat_id,
        user_id: reader_id,
        last_read_message_id,
    };
    state.registry.send_to_users(&recipients, &frame).await;
    Ok(())
}

/// Relay a typing notification to the other members of the chat.
pub async fn deliver_typing(
    state: &AppState,
    chat_id: i64,
    typist_id: i64,
) -> Result<(), ApiError> {
    let recipients = others(chat_member_ids(&state.pool, chat_id).await?, typist_id);
    let frame = ServerFrame::Typing {
        chat_id,
        user_id: typist_id,
    };
    state.registry.send_to_users(&recipients, &frame).await;
    Ok(())
}

/// Announce a new family member to everyone now in the family — including
/// the joiner, whose own devices use the frame as a "membership changed,
/// refresh" signal.
pub async fn deliver_member_joined(
    state: &AppState,
    family_id: i64,
    user: UserBrief,
) -> Result<(), ApiError> {
    let recipients = family_member_ids(&state.pool, family_id).await?;
    let frame = ServerFrame::MemberJoined { family_id, user };
    state.registry.send_to_users(&recipients, &frame).await;
    Ok(())
}

/// Announce a departure to the remaining members *and* to the departed user
/// — a removed member's own devices learn about the removal this way.
pub async fn deliver_member_left(
    state: &AppState,
    family_id: i64,
    left_user_id: i64,
) -> Result<(), ApiError> {
    let mut recipients = family_member_ids(&state.pool, family_id).await?;
    // The departed user is no longer in the family, so they are never in
    // the query result; add them explicitly.
    recipients.push(left_user_id);
    let frame = ServerFrame::MemberLeft {
        family_id,
        user_id: left_user_id,
    };
    state.registry.send_to_users(&recipients, &frame).await;
    Ok(())
}

/// Delivery-failure logging shared by REST handlers: fan-out problems must
/// not fail the HTTP request that already committed its write.
pub fn log_fanout_error(context: &'static str, result: Result<(), ApiError>) {
    if let Err(err) = result {
        warn!(error = ?err, context, "event fan-out failed");
    }
}

fn others(mut member_ids: Vec<i64>, excluded: i64) -> Vec<i64> {
    member_ids.retain(|id| *id != excluded);
    member_ids
}
