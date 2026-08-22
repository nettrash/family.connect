//! Event delivery — the single seam between "something happened" and "who
//! hears about it".
//!
//! Both the REST handlers and the WS frame handlers call these functions, so
//! fan-out rules (who gets `message` vs `ack`, who is excluded from `read`/
//! `typing` relays, when the push hook fires) exist exactly once. Push
//! composition (titles, badge) happens here on the request path — it is a
//! couple of indexed queries — but the actual transport calls are
//! `tokio::spawn`ed fire-and-forget: notification *delivery* must never add
//! latency or failure modes to the write path. Devices a transport reports
//! as unregistered are deleted from the same spawned task.

use std::collections::BTreeMap;

use sqlx::{PgPool, Row};
use tracing::{info, warn};

use crate::error::ApiError;
use crate::handlers_chat::ReactionState;
use crate::models::{Message, UserBrief};
use crate::push::DevicePush;
use crate::push_payload::{self, Notification};
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
/// An edited message to every member connection.
///
/// A separate frame from `message` on purpose (protocol.md, "Editing"): an
/// edit must not raise a notification or bump an unread count, so it
/// deliberately skips the push fan-out that `deliver_new_message` performs.
/// Nobody is told twice about the same message.
pub async fn deliver_message_edited(
    state: &AppState,
    message: &Message,
    origin_conn: Option<u64>,
) -> Result<(), ApiError> {
    let members = chat_member_ids(&state.pool, message.chat_id).await?;
    let frame = ServerFrame::MessageEdited {
        message: message.clone(),
    };
    state.registry.fan_out(&members, &frame, origin_conn).await;
    Ok(())
}

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

    let devices = devices_for_users(&state.pool, &push_targets).await?;
    if devices.is_empty() {
        return Ok(());
    }

    // Composition context: the chat kind picks the title rule, the family
    // and sender names fill it in (protocol.md "Push notifications").
    let row = sqlx::query(
        "SELECT c.kind, f.name AS family_name, u.display_name AS sender_name
         FROM chats c
         JOIN families f ON f.id = c.family_id
         JOIN users u ON u.id = $2
         WHERE c.id = $1",
    )
    .bind(message.chat_id)
    .bind(message.sender_id)
    .fetch_one(&state.pool)
    .await?;
    let chat_kind: String = row.get("kind");
    let family_name: String = row.get("family_name");
    let sender_name: String = row.get("sender_name");

    // One notification per recipient user: the badge is that user's total
    // unread, so it differs between recipients of the same message.
    let mut by_user: BTreeMap<i64, Vec<DevicePush>> = BTreeMap::new();
    for device in devices {
        by_user.entry(device.user_id).or_default().push(device);
    }
    let mut batch = Vec::with_capacity(by_user.len());
    for (user_id, user_devices) in by_user {
        let badge = unread_badge(&state.pool, user_id).await?;
        let note = push_payload::message_notification(
            state.cfg.push.include_message_body,
            &chat_kind,
            &family_name,
            &sender_name,
            message,
            badge,
        );
        batch.push((user_devices, note));
    }
    spawn_notify(state, batch);
    Ok(())
}

/// Push to the family owner when a join request is created — but only when
/// the owner has no live socket, the same rule messages follow (protocol.md
/// pushes exclusively to offline users). Join requests have no WS frame, so
/// the check is a plain registry probe rather than a fan-out result.
pub async fn push_join_request_created(
    state: &AppState,
    family_id: i64,
    family_name: &str,
    owner_user_id: i64,
    requester_id: i64,
) -> Result<(), ApiError> {
    if state.registry.has_connections(owner_user_id).await {
        return Ok(());
    }
    let devices = devices_for_users(&state.pool, &[owner_user_id]).await?;
    if devices.is_empty() {
        return Ok(());
    }
    let requester_name: String = sqlx::query_scalar("SELECT display_name FROM users WHERE id = $1")
        .bind(requester_id)
        .fetch_one(&state.pool)
        .await?;
    let badge = unread_badge(&state.pool, owner_user_id).await?;
    let note =
        push_payload::join_request_notification(family_name, &requester_name, family_id, badge);
    spawn_notify(state, vec![(devices, note)]);
    Ok(())
}

/// Push to the requester when their join request is approved — only when
/// they have no live socket (an open socket already received the
/// `member_joined` frame).
pub async fn push_join_approved(
    state: &AppState,
    family_id: i64,
    family_name: &str,
    requester_id: i64,
) -> Result<(), ApiError> {
    if state.registry.has_connections(requester_id).await {
        return Ok(());
    }
    let devices = devices_for_users(&state.pool, &[requester_id]).await?;
    if devices.is_empty() {
        return Ok(());
    }
    let badge = unread_badge(&state.pool, requester_id).await?;
    let note = push_payload::joined_notification(family_name, family_id, badge);
    spawn_notify(state, vec![(devices, note)]);
    Ok(())
}

/// The devices (with push tokens) of the listed users.
async fn devices_for_users(pool: &PgPool, user_ids: &[i64]) -> Result<Vec<DevicePush>, ApiError> {
    let rows = sqlx::query(
        "SELECT id, user_id, platform, push_token
         FROM devices
         WHERE user_id = ANY($1) AND push_token IS NOT NULL",
    )
    .bind(user_ids)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|row| DevicePush {
            device_id: row.get("id"),
            user_id: row.get("user_id"),
            platform: row.get("platform"),
            push_token: row.get("push_token"),
        })
        .collect())
}

/// The recipient's total unread across chats — the APNs badge value.
async fn unread_badge(pool: &PgPool, user_id: i64) -> Result<i64, ApiError> {
    let badge = sqlx::query_scalar(push_payload::build_unread_badge_query())
        .bind(user_id)
        .fetch_one(pool)
        .await?;
    Ok(badge)
}

/// Fire-and-forget delivery of composed notifications: push is best-effort
/// by design in v1 — the write path never waits on a push service. Devices
/// the transports report as unregistered (APNs 410/BadDeviceToken, FCM
/// 404/UNREGISTERED) are deleted here, per protocol.md.
fn spawn_notify(state: &AppState, batch: Vec<(Vec<DevicePush>, Notification)>) {
    let push = state.push.clone();
    let pool = state.pool.clone();
    tokio::spawn(async move {
        let mut dead: Vec<i64> = Vec::new();
        for (devices, note) in &batch {
            dead.extend(push.notify(devices, note).await);
        }
        if dead.is_empty() {
            return;
        }
        match sqlx::query("DELETE FROM devices WHERE id = ANY($1)")
            .bind(&dead)
            .execute(&pool)
            .await
        {
            Ok(result) => info!(
                deleted = result.rows_affected(),
                "removed devices with unregistered push tokens"
            ),
            Err(err) => warn!(error = ?err, "failed to delete unregistered devices"),
        }
    });
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

/// Fan a message's full reaction state out to every chat member — the
/// actor's connections included: the acting request is answered by its HTTP
/// response, but the actor's *other* devices learn about the change only
/// from this frame. Reactions never reach the push seam (protocol.md:
/// typing, reads, and reactions never push) — `send_to_users` discards the
/// offline list.
pub async fn deliver_reaction(state: &AppState, reaction: &ReactionState) -> Result<(), ApiError> {
    let members = chat_member_ids(&state.pool, reaction.chat_id).await?;
    let frame = ServerFrame::Reaction {
        chat_id: reaction.chat_id,
        message_id: reaction.message_id,
        reaction_seq: reaction.reaction_seq,
        reactions: reaction.reactions.clone(),
    };
    state.registry.send_to_users(&members, &frame).await;
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
