//! Shared application state.
//!
//! One cheaply-clonable struct handed to every handler via axum's `State`.
//! The `PushSender` is injected (rather than constructed inside) so the
//! integration tests can substitute a recording implementation and assert
//! that the offline-member push hook actually fires.

use std::sync::Arc;

use sqlx::PgPool;

use crate::config::Config;
use crate::push::PushSender;
use crate::registry::Registry;

/// Everything a request handler needs. All fields are shared handles, so
/// `Clone` is a few `Arc` bumps.
#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub registry: Arc<Registry>,
    pub push: Arc<dyn PushSender>,
    pub cfg: Arc<Config>,
}

impl AppState {
    /// Assemble the state; the registry's per-connection queue size comes
    /// from `[limits] ws_send_queue`.
    pub fn new(pool: PgPool, cfg: Arc<Config>, push: Arc<dyn PushSender>) -> Self {
        let registry = Arc::new(Registry::new(cfg.limits.ws_send_queue));
        Self {
            pool,
            registry,
            push,
            cfg,
        }
    }
}
