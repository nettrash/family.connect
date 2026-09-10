//! The realtime half: `{base}/api/v1/ws`, and the frames this client speaks.
//!
//! The frames are the protocol's, tagged by `"type"` — and the rule that
//! matters most is the SECOND compatibility rule: a client must ignore
//! unknown frame types. That is what let voice calls be added without
//! breaking v1 clients, and it is why `ServerFrame` has an `Unknown`
//! variant instead of a decode that fails: a frame this client has never
//! heard of is a frame it steps over, not an error it reports.
//!
//! The browser cannot send an `Authorization` header on a WebSocket
//! upgrade — the API takes no headers. It CAN offer subprotocols, which
//! travel in `Sec-WebSocket-Protocol`, so the token rides there as
//! `bearer.<token>` beside the product's own name (docs/protocol.md, "A
//! browser is a client too"). Never in the URL: a token in the query string
//! is the whole credential written into every access log on the way, and
//! the server refuses it. See `protocols_for`.

use serde::{Deserialize, Serialize};

/// What this client says on the socket. Only the frames it actually uses —
/// and no `send`: a browser sends every message over REST and only LISTENS
/// here (docs/protocol.md, "A browser is a client too"; see outbox.rs). What
/// is left is momentary, and is dropped rather than saved up while the
/// socket is down.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientFrame {
    Read {
        chat_id: i64,
        last_read_message_id: i64,
    },
    Typing {
        chat_id: i64,
    },
    Ping,
}

/// What this client reads.
///
/// `Unknown` is not a failure mode — it is the protocol's compatibility
/// rule made a type. Everything this client has not learned yet (reactions,
/// polls, board notes, call signalling) lands there and is dropped, exactly
/// as the protocol requires. There is no `ack`: that answers a `send` frame,
/// and this client never sends one. `pong` lands in `Unknown` too — any
/// frame at all is the proof of life the heartbeat is listening for.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerFrame {
    Message {
        message: crate::model::Message,
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
    #[serde(other)]
    Unknown,
}

/// Where the socket lives, for a page served from the same origin.
///
/// `https` becomes `wss` and `http` becomes `ws`, which is the protocol's
/// rule; the host is whatever served the page, because a browser client has
/// no server URL of its own (docs/protocol.md, "A browser is a client too").
/// No token here — see `protocols_for`.
pub fn url_for(origin: &str) -> String {
    let scheme = if origin.starts_with("https://") {
        "wss://"
    } else {
        "ws://"
    };
    let host = origin
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');
    format!("{scheme}{host}/api/v1/ws")
}

/// The subprotocols this client offers: the product's name, which the
/// server echoes back, and the one that carries the token.
///
/// The server must echo exactly one of them or the browser fails the
/// handshake on its own side, and it echoes the NAME — so the token goes up
/// and never comes back.
pub fn protocols_for(token: &str) -> [String; 2] {
    ["family-connect".to_string(), format!("bearer.{token}")]
}

/// Decode one text frame, forgiving what this client has not learned.
///
/// A frame that is not JSON at all is `None`; a frame that is JSON but not a
/// shape this client knows is `Unknown`. The two are different: the first is
/// a broken server or a proxy injecting something, the second is an ordinary
/// newer server, and only the first is worth a log line.
pub fn decode(text: &str) -> Option<ServerFrame> {
    serde_json::from_str(text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    /// The copy of this device's own REST send that fans out to its socket
    /// carries the `client_msg_id` — which is what lets it settle the
    /// outbox row if it lands first.
    #[wasm_bindgen_test]
    fn my_own_message_frame_carries_its_client_msg_id() {
        let frame = decode(
            r#"{"type": "message", "message": {"id": 1339, "chat_id": 42, "sender_id": 7,
                "client_msg_id": "8f14e45f-ceea", "body": "Six works",
                "created_at": "2026-08-19T17:05:00Z"}}"#,
        );
        match frame {
            Some(ServerFrame::Message { message }) => {
                assert_eq!(message.client_msg_id.as_deref(), Some("8f14e45f-ceea"));
            }
            other => panic!("expected a message frame, got {other:?}"),
        }
        // And the heartbeat's answer is a frame like any other unknown one.
        assert_eq!(decode(r#"{"type": "pong"}"#), Some(ServerFrame::Unknown));
    }

    #[wasm_bindgen_test]
    fn the_read_typing_and_ping_frames_are_the_protocols() {
        assert_eq!(
            serde_json::to_value(ClientFrame::Read {
                chat_id: 42,
                last_read_message_id: 1337
            })
            .expect("encodes"),
            serde_json::json!({"type": "read", "chat_id": 42, "last_read_message_id": 1337})
        );
        assert_eq!(
            serde_json::to_value(ClientFrame::Typing { chat_id: 42 }).expect("encodes"),
            serde_json::json!({"type": "typing", "chat_id": 42})
        );
        assert_eq!(
            serde_json::to_value(ClientFrame::Ping).expect("encodes"),
            serde_json::json!({"type": "ping"})
        );
    }

    #[wasm_bindgen_test]
    fn a_message_frame_round_trips() {
        let frame = decode(
            r#"{"type": "message", "message": {"id": 1338, "chat_id": 42, "sender_id": 9,
                "client_msg_id": null, "body": "Six works", "created_at": "2026-08-19T17:05:00Z"}}"#,
        )
        .expect("a frame this client knows");
        match frame {
            ServerFrame::Message { message } => {
                assert_eq!(message.id, 1338);
                assert_eq!(message.body, "Six works");
            }
            other => panic!("expected a message frame, got {other:?}"),
        }
    }

    /// THE COMPATIBILITY RULE, as a test. A frame type this client has
    /// never heard of must be stepped over, not fail the connection — this
    /// is what let call signalling be added to v1.
    #[wasm_bindgen_test]
    fn an_unknown_frame_type_is_ignored_rather_than_failing() {
        for text in [
            r#"{"type": "call_offer", "call_id": "6a1f0c3e", "chat_id": 42, "sdp": "v=0"}"#,
            r#"{"type": "board_note", "note": {"id": 12, "board_seq": 88}}"#,
            r#"{"type": "something_invented_next_year"}"#,
        ] {
            assert_eq!(
                decode(text),
                Some(ServerFrame::Unknown),
                "unknown frames are ignored, not errors: {text}"
            );
        }
        // And an unknown FIELD on a known frame is ignored too.
        let frame = decode(
            r#"{"type": "read", "chat_id": 42, "user_id": 9, "last_read_message_id": 1338,
                "invented": true}"#,
        );
        assert_eq!(
            frame,
            Some(ServerFrame::Read {
                chat_id: 42,
                user_id: 9,
                last_read_message_id: 1338
            })
        );
    }

    /// Not JSON at all is a different thing from a frame this client does
    /// not know, and only this one is worth complaining about.
    #[wasm_bindgen_test]
    fn a_frame_that_is_not_json_is_none() {
        assert_eq!(decode("<html>502 Bad Gateway</html>"), None);
        assert_eq!(decode(""), None);
    }

    #[wasm_bindgen_test]
    fn the_socket_url_follows_the_pages_own_scheme_and_host() {
        assert_eq!(
            url_for("https://chat.example.com"),
            "wss://chat.example.com/api/v1/ws"
        );
        assert_eq!(
            url_for("http://192.168.1.10:8080"),
            "ws://192.168.1.10:8080/api/v1/ws"
        );
        // A trailing slash on the origin must not double up in the path.
        assert_eq!(
            url_for("https://chat.example.com/"),
            "wss://chat.example.com/api/v1/ws"
        );
    }

    /// THE TOKEN IS NOT IN THE URL. A URL is written into every access log
    /// on the way; a subprotocol travels in a header, which is not.
    #[wasm_bindgen_test]
    fn the_token_rides_in_a_subprotocol_and_never_in_the_url() {
        assert!(!url_for("https://chat.example.com").contains("token"));
        assert_eq!(
            protocols_for("t0ken-_A9"),
            ["family-connect".to_string(), "bearer.t0ken-_A9".to_string()]
        );
    }
}
