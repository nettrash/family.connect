//! The REST half of the protocol, as a browser speaks it.
//!
//! `{base}` for a browser is ITS OWN ORIGIN (docs/protocol.md, "A browser is
//! a client too"): the page was served by the same nginx that proxies
//! `/api/v1`, so there is no server URL to enter and no CORS anywhere. Every
//! path here is therefore relative — `/api/v1/...` — and the browser
//! resolves it against wherever it is running, which is the one thing a
//! browser knows that an app has to be told.
//!
//! Only the endpoints this client actually uses live here. The rest of the
//! protocol is not stubbed: an unused wrapper is a claim that something
//! works, and nothing here has been tested against a server it does not
//! call.

use gloo_net::http::Request;
use serde::{Deserialize, Serialize};

use crate::model::{ChatListItem, Me, Message};

/// Everything under one prefix, so a change of base is one line.
const API: &str = "/api/v1";

/// What went wrong, in the shape a view can act on.
///
/// `Unauthorized` is called out because the protocol gives it a meaning no
/// other status has: "the session is gone — wipe local state and return to
/// login" (docs/protocol.md, "Authentication"). Every other failure is a
/// message to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiError {
    /// 401: the session is gone.
    Unauthorized,
    /// The server answered, with a protocol error code and message.
    Server { code: String, message: String },
    /// It did not answer, or the answer was not what this client can read.
    Network(String),
}

impl ApiError {
    /// What to put in front of a person. The protocol's `message` is
    /// English and meant for developers, so the caller decides whether to
    /// use it; this is the fallback when there is nothing better.
    pub fn detail(&self) -> String {
        match self {
            ApiError::Unauthorized => "Your session has expired.".to_string(),
            ApiError::Server { message, .. } => message.clone(),
            ApiError::Network(detail) => detail.clone(),
        }
    }

    pub fn code(&self) -> Option<&str> {
        match self {
            ApiError::Server { code, .. } => Some(code),
            _ => None,
        }
    }
}

/// The protocol's error body: `{"error": {"code": "...", "message": "..."}}`.
#[derive(Debug, Deserialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Debug, Deserialize)]
struct ErrorBody {
    code: String,
    message: String,
}

#[derive(Debug, Serialize)]
struct LoginRequest<'a> {
    username: &'a str,
    password: &'a str,
}

#[derive(Debug, Deserialize)]
pub struct AuthResponse {
    pub token: String,
}

#[derive(Debug, Deserialize)]
struct ChatsResponse {
    chats: Vec<ChatListItem>,
}

#[derive(Debug, Deserialize)]
struct MessagesResponse {
    messages: Vec<Message>,
}

#[derive(Debug, Deserialize)]
struct MessageResponse {
    message: Message,
}

#[derive(Debug, Serialize)]
struct SendRequest<'a> {
    client_msg_id: &'a str,
    body: &'a str,
}

#[derive(Debug, Serialize)]
struct ReadRequest {
    last_read_message_id: i64,
}

/// Turn a response into either its JSON body or the protocol's error.
///
/// A 401 is separated from every other failure HERE rather than at each call
/// site, because there is exactly one right response to it and thirty places
/// that would otherwise have to remember what it is.
async fn read<T: for<'de> Deserialize<'de>>(
    response: gloo_net::http::Response,
) -> Result<T, ApiError> {
    if response.status() == 401 {
        return Err(ApiError::Unauthorized);
    }
    if !response.ok() {
        let status = response.status();
        return match response.json::<ErrorEnvelope>().await {
            Ok(envelope) => Err(ApiError::Server {
                code: envelope.error.code,
                message: envelope.error.message,
            }),
            // A body that is not the protocol's shape is still a failure,
            // and saying which status it was beats saying nothing: nginx
            // answers its own rate limit with HTML.
            Err(_) => Err(ApiError::Network(format!("The server answered {status}."))),
        };
    }
    response
        .json::<T>()
        .await
        .map_err(|error| ApiError::Network(error.to_string()))
}

fn network(error: gloo_net::Error) -> ApiError {
    ApiError::Network(error.to_string())
}

/// `POST /auth/login` — the one call made without a token.
pub async fn login(username: &str, password: &str) -> Result<AuthResponse, ApiError> {
    let request = Request::post(&format!("{API}/auth/login"))
        .json(&LoginRequest { username, password })
        .map_err(network)?;
    read(request.send().await.map_err(network)?).await
}

/// `POST /auth/logout` — revokes this session and closes its sockets.
///
/// Its failure is deliberately not reported: the client is signing out
/// either way, and a person who has already been told "signed out" cannot
/// act on "but the server did not hear". The token is dropped locally
/// regardless, which is what makes the tab safe on a shared machine.
pub async fn logout(token: &str) {
    let _ = Request::post(&format!("{API}/auth/logout"))
        .header("Authorization", &format!("Bearer {token}"))
        .send()
        .await;
}

/// `GET /me` — who this is, and what the server allows.
pub async fn me(token: &str) -> Result<Me, ApiError> {
    let request =
        Request::get(&format!("{API}/me")).header("Authorization", &format!("Bearer {token}"));
    read(request.send().await.map_err(network)?).await
}

/// `GET /chats` — the family chat always, direct chats once they exist.
pub async fn chats(token: &str) -> Result<Vec<ChatListItem>, ApiError> {
    let request =
        Request::get(&format!("{API}/chats")).header("Authorization", &format!("Bearer {token}"));
    let response: ChatsResponse = read(request.send().await.map_err(network)?).await?;
    Ok(response.chats)
}

/// `GET /chats/{id}/messages` — the newest page, or the page before
/// `before_id` (docs/protocol.md: strictly older, newest-first).
pub async fn messages(
    token: &str,
    chat_id: i64,
    before_id: Option<i64>,
    limit: u32,
) -> Result<Vec<Message>, ApiError> {
    let mut url = format!("{API}/chats/{chat_id}/messages?limit={limit}");
    if let Some(before_id) = before_id {
        url.push_str(&format!("&before_id={before_id}"));
    }
    let request = Request::get(&url).header("Authorization", &format!("Bearer {token}"));
    let response: MessagesResponse = read(request.send().await.map_err(network)?).await?;
    Ok(response.messages)
}

/// `GET /chats/{id}/messages?after_id=` — the reconnect catch-up, oldest
/// first, looped by the caller until a short page.
pub async fn messages_after(
    token: &str,
    chat_id: i64,
    after_id: i64,
    limit: u32,
) -> Result<Vec<Message>, ApiError> {
    let url = format!("{API}/chats/{chat_id}/messages?after_id={after_id}&limit={limit}");
    let request = Request::get(&url).header("Authorization", &format!("Bearer {token}"));
    let response: MessagesResponse = read(request.send().await.map_err(network)?).await?;
    Ok(response.messages)
}

/// `POST /chats/{id}/messages` — the REST leg of a send.
///
/// The socket is the fast path; this is what a send falls back to when the
/// socket is not open. Both carry the SAME `client_msg_id`, which is what
/// makes a retry idempotent rather than a duplicate (docs/protocol.md).
pub async fn send_message(
    token: &str,
    chat_id: i64,
    client_msg_id: &str,
    body: &str,
) -> Result<Message, ApiError> {
    let request = Request::post(&format!("{API}/chats/{chat_id}/messages"))
        .header("Authorization", &format!("Bearer {token}"))
        .json(&SendRequest {
            client_msg_id,
            body,
        })
        .map_err(network)?;
    let response: MessageResponse = read(request.send().await.map_err(network)?).await?;
    Ok(response.message)
}

/// `POST /chats/{id}/read` — monotonic on the server, so a stale report is
/// harmless rather than a marker that walks backwards.
pub async fn post_read(
    token: &str,
    chat_id: i64,
    last_read_message_id: i64,
) -> Result<(), ApiError> {
    let request = Request::post(&format!("{API}/chats/{chat_id}/read"))
        .header("Authorization", &format!("Bearer {token}"))
        .json(&ReadRequest {
            last_read_message_id,
        })
        .map_err(network)?;
    let response = request.send().await.map_err(network)?;
    if response.status() == 401 {
        return Err(ApiError::Unauthorized);
    }
    if !response.ok() {
        return Err(ApiError::Network(format!(
            "The server answered {}.",
            response.status()
        )));
    }
    Ok(())
}
