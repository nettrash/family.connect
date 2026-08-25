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

use std::collections::{BTreeMap, HashSet};

use sqlx::{PgPool, Row};
use tracing::{info, warn};

use crate::error::ApiError;
use crate::handlers_chat::ReactionState;
use crate::models::{Message, Note, UserBrief};
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

/// One board note — created, moved, edited or tombstoned — to every member
/// of the family.
///
/// Family-wide rather than chat-wide: the board belongs to the family, not
/// to any conversation. Like `message_edited` it deliberately raises no
/// push and touches no unread count; a note appearing on the wall is
/// ambient, not mail.
pub async fn deliver_board_note(
    state: &AppState,
    family_id: i64,
    note: &Note,
) -> Result<(), ApiError> {
    deliver_board_note_inner(state, family_id, note, false).await
}

/// The same fan-out, but this note is NEW and therefore notifies.
///
/// Creation only: a push for every drag would make the board unusable, and
/// tidying the wall is the shared act (protocol.md, "Board"). Deletes carry
/// no content to announce and edits are the author's own correction.
pub async fn deliver_new_board_note(
    state: &AppState,
    family_id: i64,
    note: &Note,
) -> Result<(), ApiError> {
    deliver_board_note_inner(state, family_id, note, true).await
}

async fn deliver_board_note_inner(
    state: &AppState,
    family_id: i64,
    note: &Note,
    notify: bool,
) -> Result<(), ApiError> {
    let members: Vec<i64> = sqlx::query_scalar("SELECT id FROM users WHERE family_id = $1")
        .bind(family_id)
        .fetch_all(&state.pool)
        .await?;
    let frame = ServerFrame::BoardNote { note: note.clone() };
    state.registry.fan_out(&members, &frame, None).await;
    if !notify {
        return Ok(());
    }

    // Same rule as messages: device by device, and never the author.
    let author_id = note.author_id;
    let live_sessions = state.registry.live_sessions(&members).await;
    let devices = devices_for_users(&state.pool, &members).await?;
    let devices = devices_to_wake(devices, &live_sessions, author_id);
    if devices.is_empty() {
        return Ok(());
    }

    let row = sqlx::query(
        "SELECT f.name AS family_name, u.display_name AS author_name
         FROM families f JOIN users u ON u.id = $2
         WHERE f.id = $1",
    )
    .bind(family_id)
    .bind(author_id)
    .fetch_one(&state.pool)
    .await?;
    let family_name: String = row.get("family_name");
    let author_name: String = row.get("author_name");
    let text = note.text.clone().unwrap_or_default();

    let mut by_user: BTreeMap<i64, Vec<DevicePush>> = BTreeMap::new();
    for device in devices {
        by_user.entry(device.user_id).or_default().push(device);
    }
    let mut batch = Vec::with_capacity(by_user.len());
    for (user_id, user_devices) in by_user {
        // The badge stays the user's UNREAD MESSAGE count: a note is not a
        // message and must not inflate it (protocol.md keeps notes out of
        // unread entirely). The board's own count is drawn by the client.
        let badge = unread_badge(&state.pool, user_id).await?;
        let notification = push_payload::board_note_notification(
            state.cfg.push.include_message_body,
            &family_name,
            &author_name,
            family_id,
            note.id,
            &text,
            badge,
        );
        batch.push((user_devices, notification));
    }
    spawn_notify(state, batch);
    Ok(())
}

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

/// Fan a freshly inserted message out to every chat member.
///
/// `origin_conn` is the WS connection the `send` frame arrived on, if any —
/// it is skipped because it receives its `ack` directly (a REST post has no
/// origin connection, so all connections, including the sender's own,
/// receive `message`, per protocol.md). Every member is then handed to the
/// push seam, which decides device by device — not member by member; see
/// `devices_to_wake`.
pub async fn deliver_new_message(
    state: &AppState,
    message: &Message,
    origin_conn: Option<u64>,
) -> Result<(), ApiError> {
    let members = fan_out_new_message(state, message, origin_conn).await?;
    push_message_to(state, message, members).await
}

/// A new message to every member connection, raising NO notification.
///
/// For a message that is not yet worth waking anyone for — today that is
/// the assistant's empty placeholder, which exists only so every device has
/// a row to stream into (protocol.md, "The assistant"). Pushing it would
/// alert the family to a blank line, and the finished text arrives as an
/// edit, which deliberately never pushes: the alert has to be raised later,
/// by `push_message_late`, once there is something to read.
pub async fn deliver_message_without_push(
    state: &AppState,
    message: &Message,
    origin_conn: Option<u64>,
) -> Result<(), ApiError> {
    fan_out_new_message(state, message, origin_conn).await?;
    Ok(())
}

/// Raise the notification for a message whose fan-out already happened.
///
/// The pair to `deliver_message_without_push`. Who is already looking is
/// decided here rather than carried over from that call, because between
/// the two a member may have put their phone down — which is exactly the
/// person the alert is for.
pub async fn push_message_late(state: &AppState, message: &Message) -> Result<(), ApiError> {
    let members = chat_member_ids(&state.pool, message.chat_id).await?;
    push_message_to(state, message, members).await
}

/// Fan a `message` frame out to every member of its chat, answering with
/// the member ids it went to — the candidate list the push gate then
/// narrows. Handing it back saves the caller re-running the same query.
async fn fan_out_new_message(
    state: &AppState,
    message: &Message,
    origin_conn: Option<u64>,
) -> Result<Vec<i64>, ApiError> {
    let members = chat_member_ids(&state.pool, message.chat_id).await?;
    let frame = ServerFrame::Message {
        message: message.clone(),
    };
    state.registry.fan_out(&members, &frame, origin_conn).await;
    Ok(members)
}

/// Compose and send the notification for a message to the devices of
/// `candidates` that are worth waking. Split out of `deliver_new_message`
/// so a message whose alert has to wait for its body can reuse it unchanged.
async fn push_message_to(
    state: &AppState,
    message: &Message,
    candidates: Vec<i64>,
) -> Result<(), ApiError> {
    if candidates.is_empty() {
        return Ok(());
    }

    // The live-session snapshot is taken BEFORE the device rows are read,
    // and that order is the safe one: a socket that comes up in between is
    // not yet in the snapshot, so its device is pushed — and it has to be,
    // because it was not there for the fan-out either. The other order
    // would let it swallow the alert on the strength of a socket that never
    // carried the message.
    let live_sessions = state.registry.live_sessions(&candidates).await;
    let devices = devices_for_users(&state.pool, &candidates).await?;
    let devices = devices_to_wake(devices, &live_sessions, Some(message.sender_id));
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

/// Push the family owner when a join request is created — device by
/// device, skipping only the ones whose own session is holding a socket,
/// the same rule messages follow (protocol.md, "Push notifications"). This
/// used to bail out entirely the moment the owner had a socket ANYWHERE, so
/// a desktop app left running swallowed the alert for the owner's phone as
/// well; somebody knocking at the family's door is not a thing an owner
/// should have to go looking for.
pub async fn push_join_request_created(
    state: &AppState,
    family_id: i64,
    family_name: &str,
    owner_user_id: i64,
    requester_id: i64,
) -> Result<(), ApiError> {
    let live_sessions = state.registry.live_sessions(&[owner_user_id]).await;
    let devices = devices_for_users(&state.pool, &[owner_user_id]).await?;
    let devices = devices_to_wake(devices, &live_sessions, None);
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

/// Push the requester when their join request is approved — again device
/// by device. The one that was connected already had the `member_joined`
/// frame; the phone that was not connected had nothing, and being let into
/// a family is not news to find out about days later.
pub async fn push_join_approved(
    state: &AppState,
    family_id: i64,
    family_name: &str,
    requester_id: i64,
) -> Result<(), ApiError> {
    let live_sessions = state.registry.live_sessions(&[requester_id]).await;
    let devices = devices_for_users(&state.pool, &[requester_id]).await?;
    let devices = devices_to_wake(devices, &live_sessions, None);
    if devices.is_empty() {
        return Ok(());
    }
    let badge = unread_badge(&state.pool, requester_id).await?;
    let note = push_payload::joined_notification(family_name, family_id, badge);
    spawn_notify(state, vec![(devices, note)]);
    Ok(())
}

/// One candidate device: what the push seam needs to reach it, plus the
/// session it registered itself from. `None` means the row cannot say — it
/// was written before the column existed (migration 0020) and heals on the
/// device's next launch. It no longer means "revoked": since 0021 a deleted
/// session takes its device rows with it.
struct DeviceTarget {
    push: DevicePush,
    session_id: Option<i64>,
}

/// The devices (with push tokens) of the listed users, minus the ones whose
/// session is no longer valid.
///
/// A device that was SIGNED OUT must not be pushed, and there are two ways
/// to be signed out. One is revocation — logout, a password change, an
/// owner resetting a member's password — and migration 0021 answers that
/// with `ON DELETE CASCADE`: the device row goes with the session, so it is
/// simply not among these rows.
///
/// The other is EXPIRY, which no delete covers. Sessions have a sliding
/// expiry and nothing ever removes an expired row (auth.rs authenticates
/// with `expires_at > now()` and renews it in place), so a device whose
/// session ran out still names a session that exists — and would otherwise
/// be pushed forever on the strength of a link that stopped meaning
/// anything. Hence the join and the `expires_at > now()`: the row has to
/// name a session that is still ALIVE, not merely one that is still there.
///
/// `session_id IS NULL` survives this filter on purpose. That is the
/// pre-0020 row that never told us anything, and the direction of the doubt
/// has not changed for it.
async fn devices_for_users(pool: &PgPool, user_ids: &[i64]) -> Result<Vec<DeviceTarget>, ApiError> {
    let rows = sqlx::query(
        "SELECT d.id, d.user_id, d.platform, d.push_token, d.session_id
         FROM devices d
         LEFT JOIN sessions s ON s.id = d.session_id
         WHERE d.user_id = ANY($1)
           AND d.push_token IS NOT NULL
           AND (d.session_id IS NULL OR s.expires_at > now())",
    )
    .bind(user_ids)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|row| DeviceTarget {
            push: DevicePush {
                device_id: row.get("id"),
                user_id: row.get("user_id"),
                platform: row.get("platform"),
                push_token: row.get("push_token"),
            },
            session_id: row.get("session_id"),
        })
        .collect())
}

/// Narrow a set of candidate devices down to the ones actually worth
/// waking. The whole per-device rule lives here, and nowhere else.
///
/// A device is skipped only when it can PROVE it is already being fed: its
/// own session is one of the live ones. The question used to be asked of
/// the user instead — "are they connected anywhere" — and the three clients
/// hold their sockets differently enough to make that ruinous: a desktop
/// app holds one open for as long as it runs, iOS suspends its socket on
/// backgrounding and Android closes its outright. One Mac left running at
/// home therefore answered for every phone on the account and silenced all
/// of them, all day, which is the bug this function exists to end.
///
/// A device whose `session_id` is `None` is woken. That is the deliberate
/// direction of the doubt: an unattributed device is one the server cannot
/// show anybody is looking at, and an alert too many beats an alert that
/// never came. The same goes for a stale link — a device pointing at a
/// session that is live-but-elsewhere or simply not connected takes one
/// redundant push and heals on its next launch.
///
/// Doubt is NOT the rule for a device that was signed out, and the
/// difference is the whole of `devices_for_users` above: a revoked session
/// takes its device row away (0021) and an expired one is filtered out, so
/// neither ever reaches this function. This one errs towards the alert
/// only for devices somebody could still be holding, signed in.
///
/// What this costs is redundancy: someone reading on their Mac also gets a
/// banner on the phone in their pocket for the message in front of them.
/// The phone cannot know what the Mac is showing, nothing in the protocol
/// tells it, and every desktop chat client makes the same trade for the
/// same reason.
///
/// The SENDER is excluded WHOLESALE, by user id rather than device by
/// device: you are never told about your own message on your own other
/// devices, and that has not changed. `None` means the event has no author
/// to exclude — nobody sends themselves a join request.
fn devices_to_wake(
    devices: Vec<DeviceTarget>,
    live_sessions: &HashSet<i64>,
    sender_id: Option<i64>,
) -> Vec<DevicePush> {
    devices
        .into_iter()
        .filter(|device| Some(device.push.user_id) != sender_id)
        .filter(|device| {
            !device
                .session_id
                .is_some_and(|session_id| live_sessions.contains(&session_id))
        })
        .map(|device| device.push)
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    /// One candidate device. `session` is what the row knows about itself:
    /// `None` is the row written before migration 0020 added the column.
    fn device(device_id: i64, user_id: i64, session: Option<i64>) -> DeviceTarget {
        DeviceTarget {
            push: DevicePush {
                device_id,
                user_id,
                platform: "ios".to_string(),
                push_token: format!("token-{device_id}"),
            },
            session_id: session,
        }
    }

    fn live(sessions: &[i64]) -> HashSet<i64> {
        sessions.iter().copied().collect()
    }

    /// The device ids that would be woken, in the order they were offered.
    fn woken(devices: Vec<DevicePush>) -> Vec<i64> {
        devices.into_iter().map(|device| device.device_id).collect()
    }

    #[test]
    fn a_mac_on_a_live_session_is_quiet_while_the_phone_beside_it_is_woken() {
        // The bug in one assertion: one user, two devices, one socket.
        let devices = vec![device(1, 7, Some(100)), device(2, 7, Some(101))];
        assert_eq!(
            woken(devices_to_wake(devices, &live(&[100]), Some(9))),
            vec![2]
        );
    }

    #[test]
    fn a_device_whose_session_has_no_socket_is_woken() {
        // A valid session with nothing connected on it — the app was
        // killed, or the phone is asleep. This is the ordinary case the
        // whole feature exists for. (A REVOKED session is not this: its
        // device row is gone before `devices_for_users` ever sees it.)
        let devices = vec![device(1, 7, Some(100))];
        assert_eq!(
            woken(devices_to_wake(devices, &live(&[999]), Some(9))),
            vec![1]
        );
    }

    #[test]
    fn a_device_with_no_session_is_woken() {
        // Nothing can show anybody is looking at it, so it gets the alert.
        let devices = vec![device(1, 7, None)];
        assert_eq!(
            woken(devices_to_wake(devices, &live(&[]), Some(9))),
            vec![1]
        );
    }

    #[test]
    fn every_device_of_the_sender_is_skipped_however_it_is_connected() {
        // Wholesale, by user id: an unattributed device of the sender's is
        // still the sender's.
        let devices = vec![
            device(1, 7, Some(100)),
            device(2, 7, None),
            device(3, 9, None),
        ];
        assert_eq!(
            woken(devices_to_wake(devices, &live(&[]), Some(7))),
            vec![3]
        );
    }

    #[test]
    fn with_no_sender_to_exclude_only_the_live_session_is_spared() {
        // The join events have no author among their targets.
        let devices = vec![device(1, 7, Some(100)), device(2, 7, Some(101))];
        assert_eq!(
            woken(devices_to_wake(devices, &live(&[101]), None)),
            vec![1]
        );
    }
}
