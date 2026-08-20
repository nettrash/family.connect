//! family-connect — self-hosted family chat server (protocol v1).
//!
//! The crate is a library so the integration tests under `tests/` can run
//! the real router, migrator, and connection registry in-process against a
//! scratch database; `src/main.rs` is a thin binary wrapper. Modules are
//! flat — one file per concern — because the server is small enough that a
//! deeper hierarchy would only add navigation cost.
//!
//! The authoritative wire contract is `docs/protocol.md` at the repo root;
//! when this code and that document disagree, the document wins.

pub mod app;
pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod events;
pub mod handlers_auth;
pub mod handlers_chat;
pub mod handlers_device;
pub mod handlers_family;
pub mod migrate;
pub mod models;
pub mod push;
pub mod push_payload;
pub mod registry;
pub mod state;
#[cfg(test)]
pub(crate) mod test_keys;
pub mod tokens;
pub mod ws;
