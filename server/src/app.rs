//! Router assembly — the single place the URL surface is defined.
//!
//! Routes mirror the endpoint table in protocol.md one-to-one, in the same
//! order, so a diff against the document is a line-by-line read. axum 0.8
//! path parameters use `{id}` syntax.

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, patch, post, put};

use crate::state::AppState;
use crate::{
    handlers_attachment, handlers_auth, handlers_avatar, handlers_board, handlers_call,
    handlers_chat, handlers_device, handlers_family, handlers_poll, handlers_report,
    handlers_stats, ws,
};

/// Build the full application router for the given state. Used identically
/// by the binary and by the integration tests.
pub fn build_router(state: AppState) -> Router {
    let body_limit = state.cfg.limits.max_body_bytes;
    // A little headroom over the payload limit the handler enforces, so
    // an oversized upload is answered with `avatar_too_large` from our
    // own error shape rather than axum's bare 413.
    let avatar_limit = state.cfg.limits.max_avatar_bytes + 4096;
    // Slack above the ceiling on purpose, exactly as the avatar route does:
    // the layer's own refusal is a bare 413 with no protocol error body, so
    // the handler counts the bytes itself and produces the refusal a client
    // can explain. The layer is the backstop against an endless body.
    let attachment_limit = state.cfg.limits.max_attachment_bytes + 65_536;
    let preview_limit = state.cfg.limits.max_preview_bytes + 4096;
    Router::new()
        // Auth
        .route("/api/v1/auth/register", post(handlers_auth::register))
        .route("/api/v1/auth/login", post(handlers_auth::login))
        .route("/api/v1/auth/logout", post(handlers_auth::logout))
        .route("/api/v1/me", get(handlers_auth::me))
        .route("/api/v1/me/password", post(handlers_auth::change_password))
        // A POST rather than a DELETE /me: the request carries a body, and
        // RFC 9110 gives content on a DELETE no defined semantics
        // (protocol.md, "Deleting an account").
        .route("/api/v1/me/delete", post(handlers_auth::delete_account))
        // A birthday is a day and a month, so it is set and cleared rather
        // than edited — the same PUT/DELETE pair the avatar uses.
        .route(
            "/api/v1/me/birthday",
            put(handlers_auth::set_birthday).delete(handlers_auth::delete_birthday),
        )
        // Profile pictures. The avatar PUT is the one route that may
        // exceed max_body_bytes, so it carries its own limit — raising
        // the global one would weaken every JSON route with it.
        .route(
            "/api/v1/me/avatar",
            put(handlers_avatar::put_avatar)
                .delete(handlers_avatar::delete_avatar)
                .layer(DefaultBodyLimit::max(avatar_limit)),
        )
        .route(
            "/api/v1/users/{id}/avatar",
            get(handlers_avatar::get_avatar),
        )
        // Families
        .route("/api/v1/families", post(handlers_family::create_family))
        .route("/api/v1/families/join", post(handlers_family::join_family))
        .route(
            "/api/v1/families/mine",
            get(handlers_family::my_family).patch(handlers_family::patch_family),
        )
        // Every member sees the same numbers — a shared curiosity, not an
        // owner's dashboard (protocol.md, "Family statistics").
        .route(
            "/api/v1/families/mine/stats",
            get(handlers_stats::family_stats),
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
        .route(
            "/api/v1/families/members/{user_id}/password",
            post(handlers_family::reset_member_password),
        )
        .route(
            "/api/v1/families/members/{user_id}/birthday",
            put(handlers_family::set_member_birthday)
                .delete(handlers_family::delete_member_birthday),
        )
        // NOT owner-gated, unlike every other route on this path: any
        // member may block any other, the owner included (protocol.md,
        // "Blocking a member").
        .route(
            "/api/v1/families/members/{user_id}/block",
            put(handlers_family::block_member).delete(handlers_family::unblock_member),
        )
        // Reporting: any member may file, only the owner may read or
        // resolve — and a report naming the owner is never listed to them
        // (protocol.md, "Reporting a member").
        .route(
            "/api/v1/families/reports",
            post(handlers_report::create_report).get(handlers_report::list_reports),
        )
        .route(
            "/api/v1/families/reports/{id}/resolve",
            post(handlers_report::resolve_report),
        )
        // Chats & messages
        .route("/api/v1/chats", get(handlers_chat::list_chats))
        .route("/api/v1/chats/direct", post(handlers_chat::direct_chat))
        .route(
            "/api/v1/chats/{id}/messages",
            get(handlers_chat::get_messages).post(handlers_chat::post_message),
        )
        .route("/api/v1/chats/{id}/read", post(handlers_chat::mark_read))
        .route(
            "/api/v1/chats/{id}/messages/{message_id}/reaction",
            put(handlers_chat::put_reaction).delete(handlers_chat::delete_reaction),
        )
        .route(
            "/api/v1/chats/{id}/messages/{message_id}",
            patch(handlers_chat::patch_message),
        )
        .route(
            "/api/v1/chats/{id}/reactions",
            get(handlers_chat::get_reactions),
        )
        .route("/api/v1/chats/{id}/edits", get(handlers_chat::get_edits))
        // Polls. A vote is set and retracted like a reaction; closing is the
        // author's, and one-way (protocol.md, "Polls").
        .route(
            "/api/v1/chats/{id}/messages/{message_id}/vote",
            put(handlers_poll::put_vote).delete(handlers_poll::delete_vote),
        )
        .route(
            "/api/v1/chats/{id}/messages/{message_id}/poll/close",
            post(handlers_poll::close_poll_handler),
        )
        .route("/api/v1/chats/{id}/polls", get(handlers_poll::get_polls))
        // Attachments
        .route(
            "/api/v1/attachments",
            post(handlers_attachment::upload_attachment)
                .layer(DefaultBodyLimit::max(attachment_limit)),
        )
        .route(
            "/api/v1/attachments/{id}",
            get(handlers_attachment::get_attachment),
        )
        .route(
            "/api/v1/attachments/{id}/preview",
            axum::routing::put(handlers_attachment::upload_preview)
                .layer(DefaultBodyLimit::max(preview_limit))
                .get(handlers_attachment::get_preview),
        )
        // Board
        .route(
            "/api/v1/families/mine/board",
            get(handlers_board::get_board),
        )
        .route(
            "/api/v1/families/mine/board/changes",
            get(handlers_board::get_board_changes),
        )
        .route(
            "/api/v1/families/mine/board/notes",
            post(handlers_board::create_note),
        )
        .route(
            "/api/v1/families/mine/board/notes/{note_id}",
            patch(handlers_board::patch_note).delete(handlers_board::delete_note),
        )
        // Devices
        .route("/api/v1/devices", post(handlers_device::register_device))
        .route(
            "/api/v1/devices/{id}",
            delete(handlers_device::delete_device),
        )
        // Calls
        .route("/api/v1/calls/ice", get(handlers_call::get_ice_servers))
        // Ops
        .route("/api/v1/healthz", get(handlers_device::healthz))
        // Realtime
        .route("/api/v1/ws", get(ws::ws_upgrade))
        .layer(DefaultBodyLimit::max(body_limit))
        .with_state(state)
}
