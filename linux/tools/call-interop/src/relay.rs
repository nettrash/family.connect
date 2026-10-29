//! The server's part of a call between two people, and nothing more: what docs/protocol.md
//! ("Voice calls", "The sequence") says the server does with each client frame — `call_ringing`
//! back to the caller and the offer (with `from_user_id`) to the callee, the answer to the
//! caller, candidates and `call_end` to the other side, each relayed as it came. No ring timeout,
//! no busy, no record, no replay; and — as the server — nothing at all when a connection of an
//! ANSWERED call drops (its own `failed` needs neither party connected for 60 s), which is what
//! makes the dead-peer case the media's to notice.
//!
//! Peers connect to `ws://<host>:<port>/?user=<id>` and speak the protocol's call frames.

use std::collections::HashMap;
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tungstenite::handshake::server::{Request, Response};

/// Who a server frame goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum To {
    /// The connection the client frame came from.
    Sender,
    /// The other person.
    Other,
}

/// The server frames one client frame from user `from` turns into.
pub fn route(from: i64, frame: &str) -> Vec<(To, String)> {
    let Ok(Value::Object(frame)) = serde_json::from_str::<Value>(frame) else {
        return Vec::new();
    };
    let Some(call_id) = frame.get("call_id").and_then(Value::as_str) else {
        return Vec::new();
    };
    let field = |name: &str| frame.get(name).cloned().unwrap_or(Value::Null);
    match frame.get("type").and_then(Value::as_str) {
        Some("call_offer") => vec![
            (
                To::Sender,
                json!({"type": "call_ringing", "call_id": call_id}).to_string(),
            ),
            (
                To::Other,
                json!({
                    "type": "call_offer",
                    "call_id": call_id,
                    "chat_id": field("chat_id"),
                    "from_user_id": from,
                    "sdp": field("sdp"),
                    "video": frame.get("video").and_then(Value::as_bool).unwrap_or(false),
                })
                .to_string(),
            ),
        ],
        Some("call_answer") => vec![(
            To::Other,
            json!({"type": "call_answer", "call_id": call_id, "sdp": field("sdp")}).to_string(),
        )],
        Some("call_ice") => vec![(
            To::Other,
            json!({"type": "call_ice", "call_id": call_id, "candidate": field("candidate")})
                .to_string(),
        )],
        Some("call_end") => vec![(
            To::Other,
            json!({"type": "call_end", "call_id": call_id, "reason": field("reason")}).to_string(),
        )],
        _ => Vec::new(),
    }
}

/// The user id in `/?user=<id>`.
pub fn user_of(path_and_query: &str) -> Option<i64> {
    let (_, query) = path_and_query.split_once('?')?;
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix("user="))
        .and_then(|id| id.parse().ok())
}

type Registry = Arc<Mutex<HashMap<i64, Sender<String>>>>;

/// Serve the relay on `port` for ever.
pub fn serve(port: u16) -> std::io::Result<()> {
    let listener = TcpListener::bind(("0.0.0.0", port))?;
    println!("relay: listening on {port}");
    let registry: Registry = Arc::new(Mutex::new(HashMap::new()));
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let registry = registry.clone();
        std::thread::spawn(move || connection(stream, &registry));
    }
    Ok(())
}

fn connection(stream: TcpStream, registry: &Registry) {
    let mut user = None;
    let accepted = tungstenite::accept_hdr(stream, |request: &Request, response: Response| {
        user = request
            .uri()
            .path_and_query()
            .and_then(|path| user_of(path.as_str()));
        Ok(response)
    });
    let accepted = accepted.ok();
    let (Some(mut socket), Some(user)) = (accepted, user) else {
        println!("relay: a connection without ?user= was refused");
        return;
    };
    let (out, outgoing): (Sender<String>, Receiver<String>) = channel();
    registry.lock().expect("registry").insert(user, out.clone());
    println!("relay: user {user} connected");
    let _ = socket
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(10)));
    loop {
        while let Ok(text) = outgoing.try_recv() {
            if socket.send(tungstenite::Message::text(text)).is_err() {
                break;
            }
        }
        match socket.read() {
            Ok(tungstenite::Message::Text(text)) => {
                let routed = route(user, text.as_str());
                let kind = serde_json::from_str::<Value>(text.as_str())
                    .ok()
                    .and_then(|value| value["type"].as_str().map(str::to_string))
                    .unwrap_or_default();
                println!("relay: user {user} said {kind} → {} frame(s)", routed.len());
                let peers = registry.lock().expect("registry").clone();
                for (to, frame) in routed {
                    let target = match to {
                        To::Sender => Some(&out),
                        To::Other => peers
                            .iter()
                            .find(|(id, _)| **id != user)
                            .map(|(_, sender)| sender),
                    };
                    if let Some(target) = target {
                        let _ = target.send(frame);
                    }
                }
            }
            Ok(tungstenite::Message::Close(_)) => break,
            Ok(_) => {}
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => break,
        }
    }
    registry.lock().expect("registry").remove(&user);
    println!("relay: user {user} gone");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(frame: &str) -> Value {
        serde_json::from_str(frame).unwrap()
    }

    #[test]
    fn an_offer_rings_the_caller_and_reaches_the_callee_with_who_called() {
        let routed = route(
            1,
            r#"{"type":"call_offer","call_id":"c1","chat_id":7,"sdp":"v=0","video":true}"#,
        );
        assert_eq!(routed.len(), 2);
        assert_eq!(routed[0].0, To::Sender);
        assert_eq!(
            parsed(&routed[0].1),
            json!({"type": "call_ringing", "call_id": "c1"})
        );
        assert_eq!(routed[1].0, To::Other);
        assert_eq!(
            parsed(&routed[1].1),
            json!({"type": "call_offer", "call_id": "c1", "chat_id": 7, "from_user_id": 1,
                   "sdp": "v=0", "video": true})
        );
    }

    #[test]
    fn a_voice_offer_without_the_flag_is_a_voice_call() {
        let routed = route(
            2,
            r#"{"type":"call_offer","call_id":"c2","chat_id":7,"sdp":"v=0"}"#,
        );
        assert_eq!(parsed(&routed[1].1)["video"], json!(false));
        assert_eq!(parsed(&routed[1].1)["from_user_id"], json!(2));
    }

    #[test]
    fn answers_candidates_and_ends_go_to_the_other_side_as_they_came() {
        let answer = route(2, r#"{"type":"call_answer","call_id":"c1","sdp":"v=0 a"}"#);
        assert_eq!(answer[0].0, To::Other);
        assert_eq!(
            parsed(&answer[0].1),
            json!({"type": "call_answer", "call_id": "c1", "sdp": "v=0 a"})
        );
        let ice = route(
            1,
            r#"{"type":"call_ice","call_id":"c1","candidate":{"candidate":"candidate:1","sdp_mline_index":0}}"#,
        );
        assert_eq!(
            parsed(&ice[0].1)["candidate"],
            json!({"candidate": "candidate:1", "sdp_mline_index": 0})
        );
        let end = route(1, r#"{"type":"call_end","call_id":"c1","reason":"hangup"}"#);
        assert_eq!(
            parsed(&end[0].1),
            json!({"type": "call_end", "call_id": "c1", "reason": "hangup"})
        );
    }

    #[test]
    fn anything_else_goes_nowhere() {
        assert!(route(1, r#"{"type":"send","text":"hi"}"#).is_empty());
        assert!(route(1, r#"{"type":"call_end"}"#).is_empty());
        assert!(route(1, "not json").is_empty());
    }

    #[test]
    fn the_user_is_read_from_the_query() {
        assert_eq!(user_of("/?user=2"), Some(2));
        assert_eq!(user_of("/ws?x=1&user=17"), Some(17));
        assert_eq!(user_of("/"), None);
        assert_eq!(user_of("/?user=me"), None);
    }
}
