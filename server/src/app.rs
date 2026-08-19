//! Router assembly — the single place the URL surface is defined.
//!
//! Routes mirror the endpoint table in protocol.md one-to-one, in the same
//! order, so a diff against the document is a line-by-line read. axum 0.8
//! path parameters use `{id}` syntax.

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, post};

use crate::state::AppState;
use crate::{handlers_auth, handlers_chat, handlers_device, handlers_family, ws};

/// Build the full application router for the given state. Used identically
/// by the binary and by the integration tests.
pub fn build_router(state: AppState) -> Router {
    let body_limit = state.cfg.limits.max_body_bytes;
    Router::new()
        // Auth
        .route("/api/v1/auth/register", post(handlers_auth::register))
        .route("/api/v1/auth/login", post(handlers_auth::login))
        .route("/api/v1/auth/logout", post(handlers_auth::logout))
        .route("/api/v1/me", get(handlers_auth::me))
        // Families
        .route("/api/v1/families", post(handlers_family::create_family))
        .route("/api/v1/families/join", post(handlers_family::join_family))
        .route(
            "/api/v1/families/mine",
            get(handlers_family::my_family).patch(handlers_family::patch_family),
        )
        .route(
            "/api/v1/families/invite-code/rotate",
            post(handlers_family::rotate_invite_code),
        )
        .route(
            "/api/v1/families/join-requests",
            get(handlers_family::list_join_requests),
        )
        .route(
            "/api/v1/families/join-requests/{id}/approve",
            post(handlers_family::approve_join_request),
        )
        .route(
            "/api/v1/families/join-requests/{id}/reject",
            post(handlers_family::reject_join_request),
        )
        .route(
            "/api/v1/families/leave",
            post(handlers_family::leave_family),
        )
        .route(
            "/api/v1/families/members/{user_id}",
            delete(handlers_family::remove_member),
        )
        // Chats & messages
        .route("/api/v1/chats", get(handlers_chat::list_chats))
        .route("/api/v1/chats/direct", post(handlers_chat::direct_chat))
        .route(
            "/api/v1/chats/{id}/messages",
            get(handlers_chat::get_messages).post(handlers_chat::post_message),
        )
        .route("/api/v1/chats/{id}/read", post(handlers_chat::mark_read))
        // Devices
        .route("/api/v1/devices", post(handlers_device::register_device))
        .route(
            "/api/v1/devices/{id}",
            delete(handlers_device::delete_device),
        )
        // Ops
        .route("/api/v1/healthz", get(handlers_device::healthz))
        // Realtime
        .route("/api/v1/ws", get(ws::ws_upgrade))
        .layer(DefaultBodyLimit::max(body_limit))
        .with_state(state)
}
