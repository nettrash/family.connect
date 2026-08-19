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
use crate::handlers_chat;
use crate::models::{Message, UserBrief};
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
    },
    Read {
        chat_id: i64,
        last_read_message_id: i64,
    },
    Typing {
        chat_id: i64,
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
    Pong,
    Error {
        code: String,
        message: String,
        /// Present when the error answers a `send` frame.
        #[serde(skip_serializing_if = "Option::is_none")]
        client_msg_id: Option<Uuid>,
    },
}

impl ServerFrame {
    fn error(err: ApiError, client_msg_id: Option<Uuid>) -> Self {
        let (code, message) = err.into_ws_parts();
        Self::Error {
            code,
            message,
            client_msg_id,
        }
    }
}

/// `GET /api/v1/ws` — authenticated upgrade. A bad token is rejected by the
/// `AuthUser` extractor with a plain HTTP 401 before any upgrade happens.
pub async fn ws_upgrade(
    auth: AuthUser,
    State(state): State<AppState>,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
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
    Ok(ws.on_upgrade(move |socket| run_connection(state, auth, expires_at, socket)))
}

/// The per-connection task. Runs until the socket dies, the client idles
/// out, the session expires, the registry kicks us, or the server shuts
/// down — and unregisters on every one of those paths.
async fn run_connection(
    state: AppState,
    auth: AuthUser,
    expires_at: OffsetDateTime,
    mut socket: WebSocket,
) {
    let registry = state.registry.clone();
    let mut registration = registry.register(auth.user_id, auth.session_id).await;
    let shutdown = registry.shutdown_token();

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

    registry
        .unregister(auth.user_id, registration.conn_id)
        .await;
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
            } = frame
            else {
                unreachable!("type tag was \"send\"");
            };
            match handlers_chat::create_message(state, chat_id, auth.user_id, client_msg_id, &body)
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
            // Best-effort: membership failures are dropped silently rather
            // than answered — erroring on every keystroke would spam.
            if let Err(err) = events::deliver_typing(state, chat_id, auth.user_id).await {
                debug!(error = ?err, chat_id, "typing frame dropped");
            }
            None
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
            }
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
            },
        };
        assert_serializes_to(
            &frame,
            r#"{"type": "member_joined", "family_id": 3,
                "user": {"id": 11, "username": "junior", "display_name": "Junior"}}"#,
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
