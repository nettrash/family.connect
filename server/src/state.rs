//! Shared application state.
//!
//! One cheaply-clonable struct handed to every handler via axum's `State`.
//! The `PushSender` is injected (rather than constructed inside) so the
//! integration tests can substitute a recording implementation and assert
//! that the offline-member push hook actually fires.

use std::sync::Arc;

use sqlx::PgPool;

use crate::calls::CallRegistry;
use crate::config::Config;
use crate::push::PushSender;
use crate::registry::Registry;
use crate::storage::Storage;

/// Everything a request handler needs. All fields are shared handles, so
/// `Clone` is a few `Arc` bumps.
#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub registry: Arc<Registry>,
    /// The calls in flight — ringing or answered — and the last few that
    /// ended. In memory only: a call does not survive a restart, and its
    /// only durable trace is the record written when it ends
    /// (docs/protocol.md, "Voice calls").
    pub calls: Arc<CallRegistry>,
    pub push: Arc<dyn PushSender>,
    pub cfg: Arc<Config>,
    /// Where attachment bytes live. On disk, not in PostgreSQL — see
    /// migration 0009.
    pub storage: Storage,
    /// Shared client for outbound calls (today: the assistant). One rather
    /// than one-per-request, so the connection pool and the TLS session
    /// cache are actually reused.
    pub http: reqwest::Client,
}

impl AppState {
    /// Assemble the state; the registry's per-connection queue size comes
    /// from `[limits] ws_send_queue`.
    pub fn new(pool: PgPool, cfg: Arc<Config>, push: Arc<dyn PushSender>) -> Self {
        let registry = Arc::new(Registry::new(cfg.limits.ws_send_queue));
        let calls = Arc::new(CallRegistry::new());
        let storage = Storage::new(cfg.storage.attachments_dir.clone());
        // A generous timeout: a large model streaming a long answer is slow
        // by nature, and cutting it off mid-sentence is worse than waiting.
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(180))
            .build()
            .unwrap_or_default();
        Self {
            pool,
            registry,
            calls,
            push,
            cfg,
            http,
            storage,
        }
    }
}
