//! WebSocket endpoint: frame types and the per-connection task.
//!
//! Frame shapes are pinned 1:1 against the JSON in protocol.md by the unit
//! tests at the bottom — the document is the contract, serde derives are
//! just the implementation. Inbound routing reads the `"type"` tag from a
//! raw `serde_json::Value` first so unknown types can be ignored at debug
//! level (forward compatibility for future call-signaling frames) instead of
//! surfacing as deserialization errors.
//!
//! Each connection is a single task `select!`-ing over four event sources:
//! the socket, the registry's frame queue, a ping ticker (which also checks
//! idle and session expiry), and the close/shutdown signals. One task per
//! socket keeps every per-connection concern — typing throttle, activity
//! clock — in plain local state with no locking.

use std::collections::HashMap;

use axum::body::Bytes;
use axum::extract::State;
use axum::extract::ws::{CloseFrame, Message as WsMessage, Utf8Bytes, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tokio::time::{Duration, Instant, MissedTickBehavior};
use tracing::{debug, warn};
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::error::ApiError;
use crate::events;
use crate::handlers_call;
use crate::handlers_chat;
use crate::handlers_chat::NewPoll;
use crate::models::{IceCandidate, Member, Message, Note, Poll, Reaction, UserBrief};
use crate::registry::{CLOSE_GOING_AWAY, CLOSE_SESSION_GONE};
use crate::state::AppState;

/// Server-side throttle for `typing` frames: at most one relay per chat per
/// connection in this window (protocol.md fixes 3 s).
const TYPING_THROTTLE: Duration = Duration::from_secs(3);

/// Client -> server frames (protocol.md "Client → server").
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientFrame {
    Send {
        chat_id: i64,
        client_msg_id: Uuid,
        body: String,
        /// Optional: the message being answered (protocol.md, "Replies").
        #[serde(default)]
        reply_to_message_id: Option<i64>,
        /// Optional: the legacy spelling of a one-element `attachment_ids`,
        /// still accepted. Sending BOTH is `validation`.
        #[serde(default)]
        attachment_id: Option<i64>,
        /// Optional: attachments this caller uploaded, claimed by this
        /// message in the order given (protocol.md, "Photos, videos, audio,
        /// files and locations"). The bytes never travel in a frame.
        #[serde(default)]
        attachment_ids: Option<Vec<i64>>,
        /// Optional: makes this message a poll (protocol.md, "Polls"). The
        /// body is then the QUESTION, and a poll excludes an attachment.
        #[serde(default)]
        poll: Option<NewPoll>,
    },
    Read {
        chat_id: i64,
        last_read_message_id: i64,
    },
    Typing {
        chat_id: i64,
    },
    /// Place a call (protocol.md, "Voice calls"). `call_id` is a UUID the
    /// CALLER minted, exactly as `client_msg_id` is; the record written when
    /// the call ends carries it as its `client_msg_id`.
    CallOffer {
        call_id: Uuid,
        chat_id: i64,
        sdp: String,
    },
    /// The callee takes the call.
    CallAnswer {
        call_id: Uuid,
        sdp: String,
    },
    /// One ICE candidate, trickled after the offer/answer.
    CallIce {
        call_id: Uuid,
        candidate: IceCandidate,
    },
    /// End a call. `reason` is one of `hangup`, `decline`, `cancel`,
    /// `failed`; anything else is `invalid_call`.
    CallEnd {
        call_id: Uuid,
        reason: String,
    },
    Ping,
}

/// Server -> client frames (protocol.md "Server → client").
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerFrame {
    Ack {
        client_msg_id: Uuid,
        message: Message,
    },
    Message {
        message: Message,
    },
    Read {
        chat_id: i64,
        user_id: i64,
        last_read_message_id: i64,
    },
    Typing {
        chat_id: i64,
        user_id: i64,
    },
    MemberJoined {
        family_id: i64,
        user: UserBrief,
    },
    MemberLeft {
        family_id: i64,
        user_id: i64,
    },
    /// A member deleted their account, carrying the WHOLE tombstone
    /// `Member` — `deleted: true`, the placeholder display name,
    /// `avatar_version: 0`, no role and no birthday.
    ///
    /// The whole object, and not just an id, because that is exactly what a
    /// client has to overwrite: protocol.md calls this "the one frame in
    /// this protocol whose job is to WIPE stored fields", so a client
    /// applies it by writing the tombstone deliberately rather than by
    /// feeding it through the ordinary member upsert — which everywhere
    /// else must never let an absent field clear a stored one. A
    /// `member_left` frame is sent alongside it, so a client that predates
    /// this frame still fixes its roster (protocol.md, "Deleting an
    /// account").
    ///
    /// It reaches every member of the deleted account's family AND every
    /// member of any chat it was part of — a direct chat can outlive the
    /// family that created it, and its peer has to be told too. So
    /// `family_id` is OPTIONAL, and absent when the account belonged to no
    /// family at all: a client keys this frame on the `member`, never on
    /// the family. Absent rather than null, like every other optional
    /// field on this wire.
    MemberDeleted {
        #[serde(skip_serializing_if = "Option::is_none", default)]
        family_id: Option<i64>,
        member: Member,
    },
    /// The family's new owner. Sent when an owner deletes their account and
    /// ownership passes to the longest-standing remaining member — the one
    /// place this protocol moves `owner_user_id` without the owner naming a
    /// successor. It reaches every member of the family, so the new owner
    /// gains the owner's screens immediately rather than at their next
    /// `GET /me` (protocol.md, "Deleting an account").
    FamilyOwner {
        family_id: i64,
        user_id: i64,
    },
    /// A message's full current reaction state — state transfer, never a
    /// delta, so delivery order races resolve client-side by comparing
    /// `reaction_seq`.
    MessageEdited {
        message: Message,
    },
    /// A poll's full current state — state transfer, never a delta, so
    /// delivery-order races resolve client-side by comparing `poll_seq`.
    /// It never notifies and never counts as unread: a vote is not a
    /// message (protocol.md, "Polls").
    Poll {
        chat_id: i64,
        message_id: i64,
        poll: Poll,
    },
    BoardNote {
        note: Note,
    },
    /// One fragment of the assistant's reply, as it is generated
    /// (docs/protocol.md, "The assistant").
    ///
    /// Cosmetic: the row named by `message_id` is the truth, and its final
    /// body arrives as `message_edited` whether or not any of these were
    /// seen. A client that was asleep needs no special path.
    AiDelta {
        chat_id: i64,
        message_id: i64,
        text: String,
    },
    /// The reply stopped early. Whatever text arrived is already on the
    /// row; the member can ask again.
    AiError {
        chat_id: i64,
        message_id: i64,
    },
    Reaction {
        chat_id: i64,
        message_id: i64,
        reaction_seq: i64,
        reactions: Vec<Reaction>,
    },
    /// An incoming call, delivered to every connection of the callee
    /// (protocol.md, "Voice calls"). Replayed to a callee connection that
    /// registers while the call still rings.
    CallOffer {
        call_id: Uuid,
        chat_id: i64,
        from_user_id: i64,
        sdp: String,
    },
    /// The offer reached the callee: answered back on the CALLER's
    /// originating connection.
    CallRinging {
        call_id: Uuid,
    },
    /// The callee answered — to the caller's connections.
    CallAnswer {
        call_id: Uuid,
        sdp: String,
    },
    /// A relayed ICE candidate.
    CallIce {
        call_id: Uuid,
        candidate: IceCandidate,
    },
    /// The call ended, with the reason. A client `reason` (`hangup`,
    /// `decline`, `cancel`, `failed`) or a server one (`timeout`,
    /// `answered_elsewhere`).
    CallEnd {
        call_id: Uuid,
        reason: String,
    },
    Pong,
    Error {
        code: String,
        message: String,
        /// Present when the error answers a `send` frame.
        #[serde(skip_serializing_if = "Option::is_none")]
        client_msg_id: Option<Uuid>,
        /// Present when the error answers a call frame. Never both at once
        /// (protocol.md, "Client → server").
        #[serde(skip_serializing_if = "Option::is_none", default)]
        call_id: Option<Uuid>,
    },
}

impl ServerFrame {
    fn error(err: ApiError, client_msg_id: Option<Uuid>) -> Self {
        let (code, message) = err.into_ws_parts();
        Self::Error {
            code,
            message,
            client_msg_id,
            call_id: None,
        }
    }

    /// An error frame that answers a call frame: carries the `call_id`, and
    /// never a `client_msg_id`.
    fn call_error(err: ApiError, call_id: Uuid) -> Self {
        let (code, message) = err.into_ws_parts();
        Self::Error {
            code,
            message,
            client_msg_id: None,
            call_id: Some(call_id),
        }
    }
}

/// `GET /api/v1/ws` — authenticated upgrade. A bad token is rejected by the
/// `AuthUser` extractor with a plain HTTP 401 before any upgrade happens.
pub async fn ws_upgrade(
    auth: AuthUser,
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    // Captured at UPGRADE and kept for the life of the connection: a socket
    // frame carries no headers, and the assistant has to answer in the
    // language of the device that asked (docs/protocol.md). Per-connection
    // is the right grain anyway — it IS one device.
    let language = headers
        .get(axum::http::header::ACCEPT_LANGUAGE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    // Cache the session's expiry for the mid-connection check. Sliding
    // renewal can only push the real expiry further out, so the cached value
    // errs on the strict side — and a socket outliving a 180-day TTL is not
    // a case worth an extra query per ping for.
    let expires_at: OffsetDateTime =
        sqlx::query_scalar("SELECT expires_at FROM sessions WHERE id = $1")
            .bind(auth.session_id)
            .fetch_optional(&state.pool)
            .await?
            .ok_or_else(ApiError::unauthorized)?;
    Ok(ws.on_upgrade(move |socket| run_connection(state, auth, expires_at, language, socket)))
}

/// The per-connection task. Runs until the socket dies, the client idles
/// out, the session expires, the registry kicks us, or the server shuts
/// down — and unregisters on every one of those paths.
async fn run_connection(
    state: AppState,
    auth: AuthUser,
    expires_at: OffsetDateTime,
    language: Option<String>,
    mut socket: WebSocket,
) {
    let registry = state.registry.clone();
    let mut registration = registry.register(auth.user_id, auth.session_id).await;
    let shutdown = registry.shutdown_token();

    // Late arrivals (protocol.md, "Voice calls"): a connection is REGISTERED
    // some seconds after the push that woke its device, and the frames that
    // would have told it what to do were sent before it existed. Replay a
    // ringing offer (with its buffered candidates) and any very recent
    // `call_end` — so a phone woken for a call that has meanwhile been taken
    // on another device stops ringing at once rather than at its own
    // guard timeout.
    for frame in call_replays(&state, auth.user_id) {
        if send_frame(&mut socket, &frame).await.is_err() {
            registry
                .unregister(auth.user_id, registration.conn_id)
                .await;
            return;
        }
    }

    let mut ping_ticker =
        tokio::time::interval(Duration::from_secs(state.cfg.limits.ws_ping_interval_secs));
    // The first tick fires immediately; skip straight to the cadence, and
    // don't try to "catch up" missed ticks after a stall.
    ping_ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    ping_ticker.tick().await;

    let idle_timeout = Duration::from_secs(state.cfg.limits.ws_idle_timeout_secs);
    let mut last_activity = Instant::now();
    // Per-chat timestamps of the last relayed typing frame on this conn.
    let mut typing_last: HashMap<i64, Instant> = HashMap::new();

    loop {
        tokio::select! {
            inbound = socket.recv() => {
                match inbound {
                    Some(Ok(message)) => {
                        last_activity = Instant::now();
                        match message {
                            WsMessage::Text(text) => {
                                if let Some(reply) = handle_client_text(
                                    &state,
                                    &auth,
                                    registration.conn_id,
                                    &mut typing_last,
                                    language.as_deref(),
                                    text.as_str(),
                                )
                                .await
                                    && send_frame(&mut socket, &reply).await.is_err()
                                {
                                    break;
                                }
                            }
                            WsMessage::Close(_) => break,
                            // Ping/Pong are answered/consumed by the ws
                            // layer; Binary is not part of the protocol.
                            // All already refreshed last_activity above.
                            _ => {}
                        }
                    }
                    Some(Err(err)) => {
                        debug!(error = %err, user_id = auth.user_id, "websocket read error");
                        break;
                    }
                    None => break,
                }
            }

            frame = registration.frames.recv() => {
                match frame {
                    Some(frame) => {
                        if send_frame(&mut socket, &frame).await.is_err() {
                            break;
                        }
                    }
                    // The registry dropped our sender — treat as a close.
                    None => break,
                }
            }

            _ = ping_ticker.tick() => {
                if OffsetDateTime::now_utc() >= expires_at {
                    let _ = send_close(&mut socket, CLOSE_SESSION_GONE, "session expired").await;
                    break;
                }
                if last_activity.elapsed() >= idle_timeout {
                    debug!(user_id = auth.user_id, "dropping idle websocket");
                    let _ = send_close(&mut socket, CLOSE_GOING_AWAY, "idle timeout").await;
                    break;
                }
                if socket.send(WsMessage::Ping(Bytes::new())).await.is_err() {
                    break;
                }
            }

            changed = registration.close.changed() => {
                // The registry asked us to close: logout (4401) or a full
                // send queue (1001). A closed watch channel means the same.
                let code = registration.close.borrow_and_update().unwrap_or(CLOSE_GOING_AWAY);
                if changed.is_ok() {
                    let _ = send_close(&mut socket, code, "connection closed by server").await;
                }
                break;
            }

            _ = shutdown.cancelled() => {
                let _ = send_close(&mut socket, CLOSE_GOING_AWAY, "server shutting down").await;
                break;
            }
        }
    }

    // A connection that placed a call and then vanished while it still rang
    // cancels that call: nothing should ring for somebody who is no longer
    // there (protocol.md, "Voice calls"). Answered calls are untouched —
    // their audio is peer to peer and survives a reconnect.
    for ended in state.calls.connection_closed(registration.conn_id) {
        handlers_call::finish_call(&state, ended).await;
    }

    registry
        .unregister(auth.user_id, registration.conn_id)
        .await;
}

/// The frames to replay to a connection the moment it registers: a ringing
/// offer it is the callee of, then the caller's buffered candidates, then a
/// `call_end` for each call it was the callee of that ended in the last
/// couple of minutes.
fn call_replays(state: &AppState, user_id: i64) -> Vec<ServerFrame> {
    let mut frames = Vec::new();
    if let Some(pending) = state.calls.pending_offer_for(user_id) {
        frames.push(ServerFrame::CallOffer {
            call_id: pending.call_id,
            chat_id: pending.chat_id,
            from_user_id: pending.from_user_id,
            sdp: pending.sdp,
        });
        for candidate in pending.candidates {
            frames.push(ServerFrame::CallIce {
                call_id: pending.call_id,
                candidate,
            });
        }
    }
    for (call_id, reason) in state.calls.recent_ends_for(user_id) {
        frames.push(ServerFrame::CallEnd {
            call_id,
            reason: reason.as_str().to_string(),
        });
    }
    frames
}

async fn send_frame(socket: &mut WebSocket, frame: &ServerFrame) -> Result<(), axum::Error> {
    let json = serde_json::to_string(frame).expect("server frames always serialize");
    socket.send(WsMessage::Text(Utf8Bytes::from(json))).await
}

async fn send_close(socket: &mut WebSocket, code: u16, reason: &str) -> Result<(), axum::Error> {
    socket
        .send(WsMessage::Close(Some(CloseFrame {
            code,
            reason: Utf8Bytes::from(reason.to_string()),
        })))
        .await
}

/// Route one inbound text frame. Returns a frame to write directly back on
/// this socket (`pong`, `ack`, or `error`); fan-out to other connections
/// goes through events.rs. Direct replies bypass the registry queue so the
/// sender still hears back even if its queue is momentarily full.
async fn handle_client_text(
    state: &AppState,
    auth: &AuthUser,
    conn_id: u64,
    typing_last: &mut HashMap<i64, Instant>,
    language: Option<&str>,
    text: &str,
) -> Option<ServerFrame> {
    let value: serde_json::Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(err) => {
            debug!(error = %err, "ignoring unparseable websocket frame");
            return None;
        }
    };
    let frame_type = value.get("type").and_then(|t| t.as_str()).unwrap_or("");

    match frame_type {
        "ping" => Some(ServerFrame::Pong),

        "send" => {
            // Salvage the client_msg_id for the error frame even when the
            // full frame fails to parse — the client needs it to correlate.
            let client_msg_id = value
                .get("client_msg_id")
                .and_then(|v| v.as_str())
                .and_then(|s| Uuid::parse_str(s).ok());
            let frame: ClientFrame = match serde_json::from_value(value) {
                Ok(frame) => frame,
                Err(err) => {
                    return Some(ServerFrame::error(
                        ApiError::validation(format!("malformed send frame: {err}")),
                        client_msg_id,
                    ));
                }
            };
            let ClientFrame::Send {
                chat_id,
                client_msg_id,
                body,
                reply_to_message_id,
                attachment_id,
                attachment_ids,
                poll,
            } = frame
            else {
                unreachable!("type tag was \"send\"");
            };
            // The same merge REST performs: array or legacy single, never
            // both, and the error frame carries the client_msg_id so the
            // sender can correlate the refusal.
            let attachment_ids =
                match handlers_chat::merge_attachment_ids(attachment_id, attachment_ids) {
                    Ok(ids) => ids,
                    Err(err) => return Some(ServerFrame::error(err, Some(client_msg_id))),
                };
            match handlers_chat::create_message(
                state,
                chat_id,
                auth.user_id,
                client_msg_id,
                &body,
                reply_to_message_id,
                &attachment_ids,
                poll.as_ref(),
                language,
            )
            .await
            {
                Ok((message, created)) => {
                    if created {
                        // Everyone except this connection gets `message`;
                        // a dedup hit re-acks without re-fanning-out.
                        if let Err(err) =
                            events::deliver_new_message(state, &message, Some(conn_id)).await
                        {
                            warn!(error = ?err, chat_id, "message fan-out failed");
                        }
                    }
                    Some(ServerFrame::Ack {
                        client_msg_id,
                        message,
                    })
                }
                Err(err) => Some(ServerFrame::error(err, Some(client_msg_id))),
            }
        }

        "read" => {
            let frame: ClientFrame = match serde_json::from_value(value) {
                Ok(frame) => frame,
                Err(err) => {
                    return Some(ServerFrame::error(
                        ApiError::validation(format!("malformed read frame: {err}")),
                        None,
                    ));
                }
            };
            let ClientFrame::Read {
                chat_id,
                last_read_message_id,
            } = frame
            else {
                unreachable!("type tag was \"read\"");
            };
            match handlers_chat::apply_read_marker(
                state,
                chat_id,
                auth.user_id,
                last_read_message_id,
            )
            .await
            {
                Ok(effective) => {
                    if let Err(err) =
                        events::deliver_read(state, chat_id, auth.user_id, effective).await
                    {
                        warn!(error = ?err, chat_id, "read fan-out failed");
                    }
                    None
                }
                Err(err) => Some(ServerFrame::error(err, None)),
            }
        }

        "typing" => {
            let frame: ClientFrame = match serde_json::from_value(value) {
                Ok(frame) => frame,
                Err(err) => {
                    debug!(error = %err, "ignoring malformed typing frame");
                    return None;
                }
            };
            let ClientFrame::Typing { chat_id } = frame else {
                unreachable!("type tag was \"typing\"");
            };
            // Throttle before touching the database: typing is pure noise
            // and must stay cheap under key-mashing.
            let now = Instant::now();
            if let Some(last) = typing_last.get(&chat_id)
                && now.duration_since(*last) < TYPING_THROTTLE
            {
                return None;
            }
            typing_last.insert(chat_id, now);
            // Membership is checked HERE, after the throttle, and the answer
            // is silence either way.
            //
            // After, because the throttle is what keeps a key-masher from
            // turning every keystroke into a database round trip — a check
            // placed before it would be unthrottled.
            //
            // Silent, because this frame has no reply on ANY path: telling a
            // caller "not_chat_member" for one id and nothing for another
            // would turn the indicator into an oracle for which chat ids
            // exist, and erroring on every keystroke would spam a client
            // whose membership simply lapsed mid-connection.
            //
            // The check itself is not optional: without it any authenticated
            // account — including one in no family at all — could make any
            // chat in the database show them typing, since `deliver_typing`
            // reads the member list only to address the fan-out.
            if handlers_chat::ensure_chat_access(state, chat_id, auth.user_id)
                .await
                .is_err()
            {
                debug!(chat_id, "typing frame for a chat the sender is not in");
                return None;
            }
            // Best-effort: fan-out failures are dropped silently too.
            if let Err(err) = events::deliver_typing(state, chat_id, auth.user_id).await {
                debug!(error = ?err, chat_id, "typing frame dropped");
            }
            None
        }

        "call_offer" | "call_answer" | "call_ice" | "call_end" => {
            // Salvage the call_id for the error frame even when the rest of
            // the frame fails to parse — a client needs it to correlate.
            let call_id = value
                .get("call_id")
                .and_then(|v| v.as_str())
                .and_then(|s| Uuid::parse_str(s).ok());
            let frame: ClientFrame = match serde_json::from_value(value) {
                Ok(frame) => frame,
                Err(err) => {
                    let error = ApiError::bad_request(
                        crate::error::codes::INVALID_CALL,
                        format!("malformed call frame: {err}"),
                    );
                    // With a call_id, answer on the call channel; without
                    // one there is nothing to correlate, so stay silent.
                    return call_id.map(|id| ServerFrame::call_error(error, id));
                }
            };
            match frame {
                ClientFrame::CallOffer {
                    call_id,
                    chat_id,
                    sdp,
                } => handlers_call::offer(state, auth, conn_id, call_id, chat_id, sdp)
                    .await
                    .err()
                    .map(|err| ServerFrame::call_error(err, call_id)),
                ClientFrame::CallAnswer { call_id, sdp } => {
                    handlers_call::answer(state, auth, conn_id, call_id, sdp)
                        .await
                        .err()
                        .map(|err| ServerFrame::call_error(err, call_id))
                }
                ClientFrame::CallIce { call_id, candidate } => {
                    // A candidate for an unknown call is dropped in silence:
                    // it is noise-level, and answering per candidate would be
                    // a flood (protocol.md, "Semantics: Calls").
                    handlers_call::ice(state, auth.user_id, call_id, candidate).await;
                    None
                }
                ClientFrame::CallEnd { call_id, reason } => {
                    handlers_call::end(state, auth, conn_id, call_id, reason)
                        .await
                        .err()
                        .map(|err| ServerFrame::call_error(err, call_id))
                }
                _ => unreachable!("type tag was a call frame"),
            }
        }

        other => {
            // Forward compatibility: future frame types must not break us.
            debug!(frame_type = other, "ignoring unknown websocket frame type");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    //! Frame-shape tests pinned to the literal JSON in protocol.md. Each
    //! test parses the document's example string and asserts symmetric
    //! (de)serialization, so a drive-by rename in the derives fails here
    //! before it breaks a client.

    use super::*;
    use time::macros::datetime;

    fn sample_message() -> Message {
        Message {
            id: 1338,
            chat_id: 42,
            sender_id: 7,
            client_msg_id: Uuid::parse_str("8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01")
                .expect("valid uuid"),
            body: "Dinner at 7?".to_string(),
            created_at: datetime!(2026-08-19 17:03:12 UTC),
            reactions: None,
            reaction_seq: None,
            reply_to: None,
            edited_at: None,
            edit_seq: None,
            attachment: None,
            attachments: None,
            poll: None,
            call: None,
        }
    }

    /// The tombstone `Member` a `member_deleted` frame carries: no role, no
    /// birthday, no picture, the placeholder name — and `"deleted": true`,
    /// which is the only field a live member never carries (protocol.md,
    /// "Deleting an account").
    fn tombstone() -> Member {
        Member {
            id: 11,
            username: "deleted-11".to_string(),
            display_name: "Deleted account".to_string(),
            role: None,
            avatar_version: 0,
            birthday: None,
            deleted: true,
        }
    }

    fn assert_serializes_to(frame: &ServerFrame, expected_json: &str) {
        let expected: serde_json::Value =
            serde_json::from_str(expected_json).expect("expected JSON parses");
        let actual = serde_json::to_value(frame).expect("frame serializes");
        assert_eq!(actual, expected);
    }

    #[test]
    fn client_send_frame_parses_the_protocol_example() {
        let json = r#"{"type": "send", "chat_id": 42, "client_msg_id": "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01", "body": "Dinner at 7?"}"#;
        let frame: ClientFrame = serde_json::from_str(json).expect("parses");
        assert_eq!(
            frame,
            ClientFrame::Send {
                chat_id: 42,
                client_msg_id: Uuid::parse_str("8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01")
                    .expect("valid uuid"),
                body: "Dinner at 7?".to_string(),
                reply_to_message_id: None,
                attachment_id: None,
                attachment_ids: None,
                poll: None,
            }
        );
    }

    /// protocol.md's second `send` example. The field is optional, so a
    /// client that predates replies keeps parsing (the test above) — and
    /// one that sends it must reach the handler intact.
    #[test]
    fn client_send_frame_carries_a_reply_target() {
        let json = r#"{"type": "send", "chat_id": 42, "client_msg_id": "1c4a9b02-0000-4000-8000-000000000001", "body": "Six works", "reply_to_message_id": 1337}"#;
        let frame: ClientFrame = serde_json::from_str(json).expect("parses");
        assert_eq!(
            frame,
            ClientFrame::Send {
                chat_id: 42,
                client_msg_id: Uuid::parse_str("1c4a9b02-0000-4000-8000-000000000001")
                    .expect("valid uuid"),
                body: "Six works".to_string(),
                reply_to_message_id: Some(1337),
                attachment_id: None,
                attachment_ids: None,
                poll: None,
            }
        );
    }

    /// protocol.md's fourth `send` example. A poll rides the ordinary send
    /// frame — there is no "create poll" frame — because a poll IS a
    /// message and its question IS the body.
    #[test]
    fn client_send_frame_carries_a_poll() {
        let json = r#"{"type": "send", "chat_id": 42, "client_msg_id": "5b2e0c14-0000-4000-8000-000000000001", "body": "Pizza or pasta?", "poll": {"options": ["Pizza", "Pasta"]}}"#;
        let frame: ClientFrame = serde_json::from_str(json).expect("parses");
        assert_eq!(
            frame,
            ClientFrame::Send {
                chat_id: 42,
                client_msg_id: Uuid::parse_str("5b2e0c14-0000-4000-8000-000000000001")
                    .expect("valid uuid"),
                body: "Pizza or pasta?".to_string(),
                reply_to_message_id: None,
                attachment_id: None,
                attachment_ids: None,
                poll: Some(NewPoll {
                    options: vec!["Pizza".to_string(), "Pasta".to_string()],
                }),
            }
        );
    }

    /// protocol.md's third `send` example: an album rides the ordinary
    /// frame as an `attachment_ids` array, in the sender's order, and the
    /// body may be empty. The legacy `attachment_id` stays parseable
    /// beside it (the tests above), because deployed clients still send it.
    #[test]
    fn client_send_frame_carries_attachment_ids() {
        let json = r#"{"type": "send", "chat_id": 42, "client_msg_id": "9d3f1e77-0000-4000-8000-000000000001", "body": "", "attachment_ids": [34, 35, 36]}"#;
        let frame: ClientFrame = serde_json::from_str(json).expect("parses");
        assert_eq!(
            frame,
            ClientFrame::Send {
                chat_id: 42,
                client_msg_id: Uuid::parse_str("9d3f1e77-0000-4000-8000-000000000001")
                    .expect("valid uuid"),
                body: String::new(),
                reply_to_message_id: None,
                attachment_id: None,
                attachment_ids: Some(vec![34, 35, 36]),
                poll: None,
            }
        );
    }

    /// A message carrying attachments serialises BOTH keys the protocol
    /// ties together: `attachments` (1–10, sender's order, never an empty
    /// array) and the legacy `attachment`, which is its FIRST element for
    /// clients that predate plurality. Never one without the other — and
    /// neither at all on a bare message.
    #[test]
    fn a_message_frame_carries_the_attachment_list_and_its_legacy_first() {
        fn photo(id: i64) -> crate::models::Attachment {
            crate::models::Attachment {
                id,
                kind: "photo".to_string(),
                mime: "image/jpeg".to_string(),
                size: 4096,
                width: None,
                height: None,
                duration_ms: None,
                has_preview: false,
                name: None,
                latitude: None,
                longitude: None,
                accuracy_m: None,
            }
        }
        let mut message = sample_message();
        message.body = String::new();
        message.attachments = Some(vec![photo(35), photo(34)]);
        message.attachment = Some(photo(35));
        let json = serde_json::to_value(ServerFrame::Message { message }).expect("serializes");
        let attachments = json["message"]["attachments"]
            .as_array()
            .expect("attachments is an array");
        assert_eq!(
            attachments
                .iter()
                .map(|a| a["id"].as_i64().expect("id"))
                .collect::<Vec<_>>(),
            vec![35, 34],
            "the sender's order, not the id order: {json}"
        );
        assert_eq!(
            json["message"]["attachment"], attachments[0],
            "the legacy key is the FIRST element, byte for byte: {json}"
        );

        let bare = serde_json::to_value(sample_message()).expect("serializes");
        assert!(
            bare.get("attachments").is_none() && bare.get("attachment").is_none(),
            "both keys must be absent, never null or [], on a bare message: {bare}"
        );
    }

    /// The quote rides the ordinary message frame — there is no separate
    /// reply frame — and is absent, not null, on a message that is not one.
    #[test]
    fn message_frame_carries_the_reply_snippet() {
        let mut message = sample_message();
        message.reply_to = Some(crate::models::ReplyTo {
            message_id: 41,
            sender_id: 9,
            excerpt: "See you at six".to_string(),
            parent: None,
        });
        assert_serializes_to(
            &ServerFrame::Message { message },
            r#"{"type": "message", "message": {"id": 1338, "chat_id": 42, "sender_id": 7,
                 "client_msg_id": "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01",
                 "body": "Dinner at 7?", "created_at": "2026-08-19T17:03:12Z",
                 "reply_to": {"message_id": 41, "sender_id": 9, "excerpt": "See you at six"}}}"#,
        );

        let plain = serde_json::to_value(ServerFrame::Message {
            message: sample_message(),
        })
        .expect("serializes");
        assert!(
            plain["message"].get("reply_to").is_none(),
            "a message that is not a reply must omit reply_to entirely"
        );
    }

    /// The second level rides inside the first, and is ABSENT rather than
    /// null when the quoted message was not itself a reply — the same
    /// absent-not-null rule every optional field here follows.
    #[test]
    fn a_reply_to_a_reply_carries_two_levels() {
        let mut message = sample_message();
        message.reply_to = Some(crate::models::ReplyTo {
            message_id: 41,
            sender_id: 9,
            excerpt: "See you at six".to_string(),
            parent: Some(crate::models::QuotedParent {
                message_id: 38,
                sender_id: 4,
                excerpt: "What time?".to_string(),
            }),
        });
        assert_serializes_to(
            &ServerFrame::Message { message },
            r#"{"type": "message", "message": {"id": 1338, "chat_id": 42, "sender_id": 7,
                 "client_msg_id": "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01",
                 "body": "Dinner at 7?", "created_at": "2026-08-19T17:03:12Z",
                 "reply_to": {"message_id": 41, "sender_id": 9, "excerpt": "See you at six",
                              "parent": {"message_id": 38, "sender_id": 4,
                                         "excerpt": "What time?"}}}}"#,
        );

        let one_level = serde_json::to_value(ServerFrame::Message {
            message: {
                let mut m = sample_message();
                m.reply_to = Some(crate::models::ReplyTo::new(41, 9, "See you at six"));
                m
            },
        })
        .expect("serializes");
        assert!(
            one_level["message"]["reply_to"].get("parent").is_none(),
            "a quote with nothing behind it must omit parent entirely"
        );
    }

    /// The assistant's fragments, exactly as protocol.md prints them.
    #[test]
    fn the_assistant_frames_match_the_protocol() {
        assert_serializes_to(
            &ServerFrame::AiDelta {
                chat_id: 42,
                message_id: 1339,
                text: "Sure — the ".to_string(),
            },
            r#"{"type": "ai_delta", "chat_id": 42, "message_id": 1339,
                 "text": "Sure — the "}"#,
        );
        assert_serializes_to(
            &ServerFrame::AiError {
                chat_id: 42,
                message_id: 1339,
            },
            r#"{"type": "ai_error", "chat_id": 42, "message_id": 1339}"#,
        );
    }

    #[test]
    fn client_read_typing_and_ping_frames_parse_the_protocol_examples() {
        let read: ClientFrame = serde_json::from_str(
            r#"{"type": "read", "chat_id": 42, "last_read_message_id": 1337}"#,
        )
        .expect("read parses");
        assert_eq!(
            read,
            ClientFrame::Read {
                chat_id: 42,
                last_read_message_id: 1337
            }
        );
        let typing: ClientFrame =
            serde_json::from_str(r#"{"type": "typing", "chat_id": 42}"#).expect("typing parses");
        assert_eq!(typing, ClientFrame::Typing { chat_id: 42 });
        let ping: ClientFrame = serde_json::from_str(r#"{"type": "ping"}"#).expect("ping parses");
        assert_eq!(ping, ClientFrame::Ping);
    }

    #[test]
    fn ack_frame_matches_the_protocol_shape() {
        let frame = ServerFrame::Ack {
            client_msg_id: sample_message().client_msg_id,
            message: sample_message(),
        };
        assert_serializes_to(
            &frame,
            r#"{"type": "ack", "client_msg_id": "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01",
                "message": {"id": 1338, "chat_id": 42, "sender_id": 7,
                            "client_msg_id": "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01",
                            "body": "Dinner at 7?", "created_at": "2026-08-19T17:03:12Z"}}"#,
        );
    }

    #[test]
    fn message_frame_matches_the_protocol_shape() {
        let frame = ServerFrame::Message {
            message: sample_message(),
        };
        assert_serializes_to(
            &frame,
            r#"{"type": "message",
                "message": {"id": 1338, "chat_id": 42, "sender_id": 7,
                            "client_msg_id": "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01",
                            "body": "Dinner at 7?", "created_at": "2026-08-19T17:03:12Z"}}"#,
        );
    }

    #[test]
    fn read_frame_matches_the_protocol_shape() {
        let frame = ServerFrame::Read {
            chat_id: 42,
            user_id: 9,
            last_read_message_id: 1338,
        };
        assert_serializes_to(
            &frame,
            r#"{"type": "read", "chat_id": 42, "user_id": 9, "last_read_message_id": 1338}"#,
        );
    }

    #[test]
    fn typing_frame_matches_the_protocol_shape() {
        let frame = ServerFrame::Typing {
            chat_id: 42,
            user_id: 9,
        };
        assert_serializes_to(&frame, r#"{"type": "typing", "chat_id": 42, "user_id": 9}"#);
    }

    #[test]
    fn member_joined_frame_matches_the_protocol_shape() {
        let frame = ServerFrame::MemberJoined {
            family_id: 3,
            user: UserBrief {
                id: 11,
                username: "junior".to_string(),
                display_name: "Junior".to_string(),
                avatar_version: 0,
            },
        };
        assert_serializes_to(
            &frame,
            r#"{"type": "member_joined", "family_id": 3,
                "user": {"id": 11, "username": "junior", "display_name": "Junior",
                         "avatar_version": 0}}"#,
        );
    }

    #[test]
    fn member_left_frame_matches_the_protocol_shape() {
        let frame = ServerFrame::MemberLeft {
            family_id: 3,
            user_id: 11,
        };
        assert_serializes_to(
            &frame,
            r#"{"type": "member_left", "family_id": 3, "user_id": 11}"#,
        );
    }

    /// The tombstone `Member` the frame carries: no role, no birthday, no
    /// picture, the placeholder name — and `"deleted": true`, which is the
    /// only field a live member never carries (protocol.md, "Deleting an
    /// account").
    #[test]
    fn member_deleted_frame_carries_the_whole_tombstone_member() {
        let frame = ServerFrame::MemberDeleted {
            family_id: Some(3),
            member: tombstone(),
        };
        assert_serializes_to(
            &frame,
            r#"{"type": "member_deleted", "family_id": 3,
                "member": {"id": 11, "username": "deleted-11",
                           "display_name": "Deleted account",
                           "avatar_version": 0, "deleted": true}}"#,
        );
    }

    /// An account that belonged to NO family still sends this frame — to
    /// the peers of its direct chats, which are about to vanish from under
    /// them. There is no family to name, so the key is ABSENT rather than
    /// null: the same rule every other optional field on this wire follows,
    /// and here also the only honest answer (protocol.md, "Deleting an
    /// account").
    #[test]
    fn member_deleted_omits_the_family_id_when_there_was_no_family() {
        let frame = ServerFrame::MemberDeleted {
            family_id: None,
            member: tombstone(),
        };
        assert_serializes_to(
            &frame,
            r#"{"type": "member_deleted",
                "member": {"id": 11, "username": "deleted-11",
                           "display_name": "Deleted account",
                           "avatar_version": 0, "deleted": true}}"#,
        );
        let json = serde_json::to_value(&frame).expect("serializes");
        assert!(
            json.get("family_id").is_none(),
            "family_id must be absent, not null: {json}"
        );
    }

    #[test]
    fn family_owner_frame_matches_the_protocol_shape() {
        let frame = ServerFrame::FamilyOwner {
            family_id: 3,
            user_id: 9,
        };
        assert_serializes_to(
            &frame,
            r#"{"type": "family_owner", "family_id": 3, "user_id": 9}"#,
        );
    }

    #[test]
    fn reaction_frame_matches_the_protocol_shape() {
        let frame = ServerFrame::Reaction {
            chat_id: 42,
            message_id: 1338,
            reaction_seq: 124,
            reactions: vec![Reaction {
                user_id: 9,
                emoji: "❤️".to_string(),
            }],
        };
        assert_serializes_to(
            &frame,
            r#"{"type": "reaction", "chat_id": 42, "message_id": 1338, "reaction_seq": 124,
                "reactions": [{"user_id": 9, "emoji": "❤️"}]}"#,
        );
    }

    #[test]
    fn poll_frame_matches_the_protocol_shape() {
        let frame = ServerFrame::Poll {
            chat_id: 42,
            message_id: 1340,
            poll: Poll {
                poll_seq: 89,
                closed: false,
                options: vec![
                    crate::models::PollOption {
                        id: 5,
                        text: "Pizza".to_string(),
                        votes: vec![7, 9],
                    },
                    crate::models::PollOption {
                        id: 6,
                        text: "Pasta".to_string(),
                        votes: vec![],
                    },
                ],
            },
        };
        assert_serializes_to(
            &frame,
            r#"{"type": "poll", "chat_id": 42, "message_id": 1340,
                "poll": {"poll_seq": 89, "closed": false,
                         "options": [{"id": 5, "text": "Pizza", "votes": [7, 9]},
                                     {"id": 6, "text": "Pasta", "votes": []}]}}"#,
        );
    }

    #[test]
    fn a_message_with_reactions_serializes_them_and_a_bare_message_omits_them() {
        let mut message = sample_message();
        message.reactions = Some(vec![Reaction {
            user_id: 9,
            emoji: "❤️".to_string(),
        }]);
        message.reaction_seq = Some(124);
        let json = serde_json::to_value(&message).expect("serializes");
        assert_eq!(
            json["reactions"],
            serde_json::json!([{"user_id": 9, "emoji": "❤️"}])
        );
        assert_eq!(json["reaction_seq"], 124);

        let bare = serde_json::to_value(sample_message()).expect("serializes");
        assert!(
            bare.get("reactions").is_none() && bare.get("reaction_seq").is_none(),
            "reaction fields must be absent, not null: {bare}"
        );
    }

    /// A poll rides the ORDINARY `message` frame — there is no separate
    /// "poll created" frame — and the whole object goes with it, so a client
    /// draws the buttons from the frame that delivered the question. The key
    /// is absent, never null, on a message that is not a poll: a client that
    /// has never heard of polls sees a plain message and loses only the
    /// buttons (protocol.md, "Polls").
    #[test]
    fn a_message_frame_carries_a_whole_poll_and_a_bare_message_omits_it() {
        let mut message = sample_message();
        message.body = "Pizza or pasta?".to_string();
        message.poll = Some(Poll {
            poll_seq: 88,
            closed: false,
            options: vec![
                crate::models::PollOption {
                    id: 5,
                    text: "Pizza".to_string(),
                    votes: vec![7, 9],
                },
                crate::models::PollOption {
                    id: 6,
                    text: "Pasta".to_string(),
                    votes: vec![],
                },
            ],
        });
        assert_serializes_to(
            &ServerFrame::Message { message },
            r#"{"type": "message",
                "message": {"id": 1338, "chat_id": 42, "sender_id": 7,
                            "client_msg_id": "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01",
                            "body": "Pizza or pasta?",
                            "created_at": "2026-08-19T17:03:12Z",
                            "poll": {"poll_seq": 88, "closed": false,
                                     "options": [{"id": 5, "text": "Pizza", "votes": [7, 9]},
                                                 {"id": 6, "text": "Pasta", "votes": []}]}}}"#,
        );

        let bare = serde_json::to_value(sample_message()).expect("serializes");
        assert!(
            bare.get("poll").is_none(),
            "the poll key must be absent, not null, on an ordinary message: {bare}"
        );
    }

    #[test]
    fn client_call_frames_parse_the_protocol_examples() {
        let offer: ClientFrame = serde_json::from_str(
            r#"{"type": "call_offer", "call_id": "6a1f0c3e-0000-4000-8000-000000000001", "chat_id": 42, "sdp": "v=0\r\n"}"#,
        )
        .expect("call_offer parses");
        assert_eq!(
            offer,
            ClientFrame::CallOffer {
                call_id: Uuid::parse_str("6a1f0c3e-0000-4000-8000-000000000001").unwrap(),
                chat_id: 42,
                sdp: "v=0\r\n".to_string(),
            }
        );
        let answer: ClientFrame = serde_json::from_str(
            r#"{"type": "call_answer", "call_id": "6a1f0c3e-0000-4000-8000-000000000001", "sdp": "v=0\r\n"}"#,
        )
        .expect("call_answer parses");
        assert_eq!(
            answer,
            ClientFrame::CallAnswer {
                call_id: Uuid::parse_str("6a1f0c3e-0000-4000-8000-000000000001").unwrap(),
                sdp: "v=0\r\n".to_string(),
            }
        );
        let ice: ClientFrame = serde_json::from_str(
            r#"{"type": "call_ice", "call_id": "6a1f0c3e-0000-4000-8000-000000000001", "candidate": {"candidate": "candidate:1", "sdp_mid": "0", "sdp_mline_index": 0}}"#,
        )
        .expect("call_ice parses");
        assert_eq!(
            ice,
            ClientFrame::CallIce {
                call_id: Uuid::parse_str("6a1f0c3e-0000-4000-8000-000000000001").unwrap(),
                candidate: IceCandidate {
                    candidate: "candidate:1".to_string(),
                    sdp_mid: Some("0".to_string()),
                    sdp_mline_index: Some(0),
                },
            }
        );
        let end: ClientFrame = serde_json::from_str(
            r#"{"type": "call_end", "call_id": "6a1f0c3e-0000-4000-8000-000000000001", "reason": "hangup"}"#,
        )
        .expect("call_end parses");
        assert_eq!(
            end,
            ClientFrame::CallEnd {
                call_id: Uuid::parse_str("6a1f0c3e-0000-4000-8000-000000000001").unwrap(),
                reason: "hangup".to_string(),
            }
        );
    }

    /// A candidate may carry only one of the two sdp locators — a WebRTC
    /// stack supplies one, the other, or both — and the absent one is not
    /// serialized as null.
    #[test]
    fn an_ice_candidate_omits_the_sdp_locators_it_was_not_given() {
        let frame = ServerFrame::CallIce {
            call_id: Uuid::parse_str("6a1f0c3e-0000-4000-8000-000000000001").unwrap(),
            candidate: IceCandidate {
                candidate: "candidate:1".to_string(),
                sdp_mid: None,
                sdp_mline_index: None,
            },
        };
        let json = serde_json::to_value(&frame).expect("serializes");
        assert!(
            json["candidate"].get("sdp_mid").is_none()
                && json["candidate"].get("sdp_mline_index").is_none(),
            "the sdp locators must be absent, not null: {json}"
        );
    }

    #[test]
    fn server_call_frames_match_the_protocol_shape() {
        let call_id = Uuid::parse_str("6a1f0c3e-0000-4000-8000-000000000001").unwrap();
        assert_serializes_to(
            &ServerFrame::CallOffer {
                call_id,
                chat_id: 42,
                from_user_id: 7,
                sdp: "v=0".to_string(),
            },
            r#"{"type": "call_offer", "call_id": "6a1f0c3e-0000-4000-8000-000000000001",
                "chat_id": 42, "from_user_id": 7, "sdp": "v=0"}"#,
        );
        assert_serializes_to(
            &ServerFrame::CallRinging { call_id },
            r#"{"type": "call_ringing", "call_id": "6a1f0c3e-0000-4000-8000-000000000001"}"#,
        );
        assert_serializes_to(
            &ServerFrame::CallAnswer {
                call_id,
                sdp: "v=0".to_string(),
            },
            r#"{"type": "call_answer", "call_id": "6a1f0c3e-0000-4000-8000-000000000001", "sdp": "v=0"}"#,
        );
        assert_serializes_to(
            &ServerFrame::CallEnd {
                call_id,
                reason: "answered_elsewhere".to_string(),
            },
            r#"{"type": "call_end", "call_id": "6a1f0c3e-0000-4000-8000-000000000001", "reason": "answered_elsewhere"}"#,
        );
    }

    /// An error answering a call frame carries the `call_id` and never a
    /// `client_msg_id`; the two are mutually exclusive on the wire.
    #[test]
    fn a_call_error_carries_the_call_id_and_not_a_client_msg_id() {
        let call_id = Uuid::parse_str("6a1f0c3e-0000-4000-8000-000000000001").unwrap();
        let frame = ServerFrame::call_error(
            ApiError::conflict(
                crate::error::codes::PEER_BUSY,
                "the person you called is on another call",
            ),
            call_id,
        );
        let json = serde_json::to_value(&frame).expect("serializes");
        assert_eq!(json["type"], "error");
        assert_eq!(json["code"], "peer_busy");
        assert_eq!(json["call_id"], "6a1f0c3e-0000-4000-8000-000000000001");
        assert!(
            json.get("client_msg_id").is_none(),
            "a call error must not carry a client_msg_id: {json}"
        );
    }

    /// A message that is the record of a call carries the whole `call`
    /// object; an ordinary message omits the key entirely.
    #[test]
    fn a_message_frame_carries_a_call_record_and_a_bare_message_omits_it() {
        let mut message = sample_message();
        message.body = "Voice call".to_string();
        message.call = Some(crate::models::CallRecord {
            outcome: "completed".to_string(),
            duration_secs: Some(222),
        });
        let json = serde_json::to_value(ServerFrame::Message { message }).expect("serializes");
        assert_eq!(
            json["message"]["call"],
            serde_json::json!({"outcome": "completed", "duration_secs": 222})
        );

        let bare = serde_json::to_value(sample_message()).expect("serializes");
        assert!(
            bare.get("call").is_none(),
            "the call key must be absent, not null, on an ordinary message: {bare}"
        );
    }

    /// A missed call's record carries no duration — the call was never
    /// answered, and that absence is meaningful, so it is omitted not null.
    #[test]
    fn a_missed_call_record_omits_the_duration() {
        let record = crate::models::CallRecord {
            outcome: "missed".to_string(),
            duration_secs: None,
        };
        let json = serde_json::to_value(&record).expect("serializes");
        assert_eq!(json, serde_json::json!({"outcome": "missed"}));
    }

    #[test]
    fn pong_frame_matches_the_protocol_shape() {
        assert_serializes_to(&ServerFrame::Pong, r#"{"type": "pong"}"#);
    }

    #[test]
    fn error_frame_includes_client_msg_id_only_when_answering_a_send() {
        let with_id = ServerFrame::Error {
            code: "not_chat_member".to_string(),
            message: "you are not a member of this chat".to_string(),
            client_msg_id: Some(sample_message().client_msg_id),
            call_id: None,
        };
        assert_serializes_to(
            &with_id,
            r#"{"type": "error", "code": "not_chat_member",
                "message": "you are not a member of this chat",
                "client_msg_id": "8f14e45f-ceea-4e17-a91c-0d9f8e7b2a01"}"#,
        );

        let without_id = ServerFrame::Error {
            code: "not_chat_member".to_string(),
            message: "you are not a member of this chat".to_string(),
            client_msg_id: None,
            call_id: None,
        };
        let json = serde_json::to_value(&without_id).expect("serializes");
        assert!(
            json.get("client_msg_id").is_none(),
            "client_msg_id must be absent, not null: {json}"
        );
    }

    #[test]
    fn server_frames_round_trip_through_their_own_json() {
        let frames = vec![
            ServerFrame::Ack {
                client_msg_id: sample_message().client_msg_id,
                message: sample_message(),
            },
            ServerFrame::MemberJoined {
                family_id: 3,
                user: UserBrief {
                    id: 11,
                    username: "junior".to_string(),
                    display_name: "Junior".to_string(),
                    avatar_version: 0,
                },
            },
            ServerFrame::Pong,
        ];
        for frame in frames {
            let json = serde_json::to_string(&frame).expect("serializes");
            let back: ServerFrame = serde_json::from_str(&json).expect("parses back");
            assert_eq!(back, frame);
        }
    }
}
