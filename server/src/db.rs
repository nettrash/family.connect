//! PostgreSQL pool construction.
//!
//! Connects via `PgConnectOptions` rather than a URL so passwords never need
//! URL-escaping; the string form exists only for human-readable log/error
//! context, and the variant used there always masks the password.

use std::time::Duration;

use anyhow::{Context, Result};
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

use crate::config::DatabaseConfig;

/// Render the connection parameters as a `postgres://` URL for display.
///
/// With `mask_password` the password becomes `***` — that variant is the
/// only one that should ever reach logs or error messages.
pub fn connection_string(cfg: &DatabaseConfig, mask_password: bool) -> String {
    let password = if mask_password { "***" } else { &cfg.password };
    format!(
        "postgres://{}:{}@{}:{}/{}",
        cfg.user, password, cfg.host, cfg.port, cfg.database
    )
}

/// Open a connection pool, eagerly establishing one connection so an
/// unreachable database fails startup instead of the first request.
pub async fn connect(cfg: &DatabaseConfig) -> Result<PgPool> {
    // new_without_pgpass: the password always comes from the config file,
    // so the libpq-style ~/.pgpass lookup is never wanted — and under the
    // hardened unit (ProtectHome=true, no-home service user) that probe
    // fails with EACCES and logs a spurious WARN on every connect.
    let options = PgConnectOptions::new_without_pgpass()
        .host(&cfg.host)
        .port(cfg.port)
        .username(&cfg.user)
        .password(&cfg.password)
        .database(&cfg.database);

    PgPoolOptions::new()
        .max_connections(cfg.max_connections)
        .acquire_timeout(Duration::from_secs(cfg.connect_timeout_secs))
        .connect_with(options)
        .await
        .with_context(|| format!("connecting to {}", connection_string(cfg, true)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config() -> DatabaseConfig {
        DatabaseConfig {
            host: "db.internal".to_string(),
            port: 5433,
            user: "svc".to_string(),
            password: "s3cret".to_string(),
            database: "family".to_string(),
            max_connections: 3,
            connect_timeout_secs: 5,
        }
    }

    #[test]
    fn connection_string_renders_all_parameters() {
        let s = connection_string(&sample_config(), false);
        assert_eq!(s, "postgres://svc:s3cret@db.internal:5433/family");
    }

    #[test]
    fn masked_connection_string_never_contains_the_password() {
        let s = connection_string(&sample_config(), true);
        assert!(!s.contains("s3cret"), "password leaked into {s}");
        assert!(s.contains("***"));
    }
}
