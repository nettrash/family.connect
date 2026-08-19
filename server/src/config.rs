//! Configuration loading and validation.
//!
//! Every key has a compiled-in default matching `config.example.toml`, so an
//! empty file (or missing sections) is a valid configuration — the example
//! file is documentation, not a requirement. Validation happens once at
//! startup and fails fast with a precise message rather than letting a bad
//! value surface later as a confusing runtime error (e.g. an idle timeout
//! shorter than the ping interval would silently kill every socket).

use std::net::SocketAddr;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

/// Top-level configuration document.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,

    #[serde(default)]
    pub database: DatabaseConfig,

    #[serde(default)]
    pub auth: AuthConfig,

    #[serde(default)]
    pub limits: LimitsConfig,

    #[serde(default)]
    pub push: PushConfig,
}

/// `[server]` — the HTTP + WebSocket listener.
#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    /// Bind address, e.g. `127.0.0.1:8080`. Loopback in production; nginx
    /// terminates TLS and proxies to this address.
    #[serde(default = "default_bind")]
    pub bind: String,
}

/// `[database]` — PostgreSQL connection parameters.
#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseConfig {
    #[serde(default = "default_db_host")]
    pub host: String,

    #[serde(default = "default_db_port")]
    pub port: u16,

    #[serde(default = "default_db_user")]
    pub user: String,

    /// Empty by default: local trust/peer auth setups need no password.
    #[serde(default)]
    pub password: String,

    #[serde(default = "default_db_database")]
    pub database: String,

    #[serde(default = "default_db_max_connections")]
    pub max_connections: u32,

    #[serde(default = "default_db_connect_timeout_secs")]
    pub connect_timeout_secs: u64,
}

/// `[auth]` — session lifetime policy.
#[derive(Debug, Clone, Deserialize)]
pub struct AuthConfig {
    /// Sliding session TTL. Every "touch" pushes expiry out this far again.
    #[serde(default = "default_session_ttl_days")]
    pub session_ttl_days: i64,

    /// Minimum age of `last_used_at` before a request triggers a renewal
    /// write — batches the sliding-expiry UPDATEs so chatty clients don't
    /// rewrite their session row on every request.
    #[serde(default = "default_session_touch_interval_mins")]
    pub session_touch_interval_mins: i64,
}

/// `[limits]` — protocol limits (protocol.md "Limits" table).
#[derive(Debug, Clone, Deserialize)]
pub struct LimitsConfig {
    /// Maximum message body length in characters (not bytes).
    #[serde(default = "default_max_message_chars")]
    pub max_message_chars: usize,

    /// Page size for GET /chats/{id}/messages when the client sends none.
    #[serde(default = "default_default_page_size")]
    pub default_page_size: i64,

    /// Ceiling a client-supplied limit is clamped to.
    #[serde(default = "default_max_page_size")]
    pub max_page_size: i64,

    /// Maximum accepted HTTP request body in bytes.
    #[serde(default = "default_max_body_bytes")]
    pub max_body_bytes: usize,

    /// Outbound frames buffered per WebSocket before the connection is
    /// declared too slow and dropped.
    #[serde(default = "default_ws_send_queue")]
    pub ws_send_queue: usize,

    /// Protocol-level ping cadence.
    #[serde(default = "default_ws_ping_interval_secs")]
    pub ws_ping_interval_secs: u64,

    /// Idle cutoff after which a silent socket is closed.
    #[serde(default = "default_ws_idle_timeout_secs")]
    pub ws_idle_timeout_secs: u64,
}

/// `[push]` — push notification driver selection.
#[derive(Debug, Clone, Deserialize)]
pub struct PushConfig {
    /// v1 supports only `"log"`; validation rejects anything else so a typo
    /// fails at startup instead of silently dropping notifications.
    #[serde(default = "default_push_driver")]
    pub driver: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: default_bind(),
        }
    }
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            host: default_db_host(),
            port: default_db_port(),
            user: default_db_user(),
            password: String::new(),
            database: default_db_database(),
            max_connections: default_db_max_connections(),
            connect_timeout_secs: default_db_connect_timeout_secs(),
        }
    }
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            session_ttl_days: default_session_ttl_days(),
            session_touch_interval_mins: default_session_touch_interval_mins(),
        }
    }
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            max_message_chars: default_max_message_chars(),
            default_page_size: default_default_page_size(),
            max_page_size: default_max_page_size(),
            max_body_bytes: default_max_body_bytes(),
            ws_send_queue: default_ws_send_queue(),
            ws_ping_interval_secs: default_ws_ping_interval_secs(),
            ws_idle_timeout_secs: default_ws_idle_timeout_secs(),
        }
    }
}

impl Default for PushConfig {
    fn default() -> Self {
        Self {
            driver: default_push_driver(),
        }
    }
}

impl Config {
    /// Read, parse, and validate a config file.
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("reading config file {}", path.display()))?;
        Self::from_toml_str(&raw).with_context(|| format!("in config file {}", path.display()))
    }

    /// Parse and validate from a TOML string (shared by `load` and tests).
    pub fn from_toml_str(raw: &str) -> Result<Self> {
        let cfg: Config = toml::from_str(raw).context("parsing TOML config")?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Reject configurations that would misbehave at runtime.
    pub fn validate(&self) -> Result<()> {
        self.server.bind.parse::<SocketAddr>().with_context(|| {
            format!("server.bind is not a socket address: {}", self.server.bind)
        })?;
        if self.database.host.is_empty() {
            anyhow::bail!("database.host must not be empty");
        }
        if self.database.user.is_empty() {
            anyhow::bail!("database.user must not be empty");
        }
        if self.database.database.is_empty() {
            anyhow::bail!("database.database must not be empty");
        }
        if self.database.max_connections == 0 {
            anyhow::bail!("database.max_connections must be at least 1");
        }
        if self.auth.session_ttl_days < 1 {
            anyhow::bail!("auth.session_ttl_days must be at least 1");
        }
        if self.auth.session_touch_interval_mins < 1 {
            anyhow::bail!("auth.session_touch_interval_mins must be at least 1");
        }
        if self.limits.max_message_chars == 0 {
            anyhow::bail!("limits.max_message_chars must be at least 1");
        }
        if self.limits.default_page_size < 1 {
            anyhow::bail!("limits.default_page_size must be at least 1");
        }
        if self.limits.max_page_size < self.limits.default_page_size {
            anyhow::bail!(
                "limits.max_page_size ({}) must be >= limits.default_page_size ({})",
                self.limits.max_page_size,
                self.limits.default_page_size
            );
        }
        // Below 1 KiB even a login request wouldn't fit; treat as a typo.
        if self.limits.max_body_bytes < 1024 {
            anyhow::bail!("limits.max_body_bytes must be at least 1024");
        }
        if self.limits.ws_send_queue == 0 {
            anyhow::bail!("limits.ws_send_queue must be at least 1");
        }
        if self.limits.ws_ping_interval_secs == 0 {
            anyhow::bail!("limits.ws_ping_interval_secs must be at least 1");
        }
        if self.limits.ws_idle_timeout_secs <= self.limits.ws_ping_interval_secs {
            anyhow::bail!(
                "limits.ws_idle_timeout_secs ({}) must exceed ws_ping_interval_secs ({}) — \
                 otherwise a healthy client can be dropped before it sees a single ping",
                self.limits.ws_idle_timeout_secs,
                self.limits.ws_ping_interval_secs
            );
        }
        if self.push.driver != "log" {
            anyhow::bail!(
                "push.driver must be \"log\" (the only driver in v1), got {:?}",
                self.push.driver
            );
        }
        Ok(())
    }
}

fn default_bind() -> String {
    "127.0.0.1:8080".to_string()
}

fn default_db_host() -> String {
    "127.0.0.1".to_string()
}

fn default_db_port() -> u16 {
    5432
}

fn default_db_user() -> String {
    "family_connect".to_string()
}

fn default_db_database() -> String {
    "family_connect".to_string()
}

fn default_db_max_connections() -> u32 {
    10
}

fn default_db_connect_timeout_secs() -> u64 {
    10
}

fn default_session_ttl_days() -> i64 {
    180
}

fn default_session_touch_interval_mins() -> i64 {
    15
}

fn default_max_message_chars() -> usize {
    4000
}

fn default_default_page_size() -> i64 {
    50
}

fn default_max_page_size() -> i64 {
    200
}

fn default_max_body_bytes() -> usize {
    16384
}

fn default_ws_send_queue() -> usize {
    64
}

fn default_ws_ping_interval_secs() -> u64 {
    30
}

fn default_ws_idle_timeout_secs() -> u64 {
    75
}

fn default_push_driver() -> String {
    "log".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn the_checked_in_example_config_parses_and_validates() {
        let cfg = Config::from_toml_str(include_str!("../config.example.toml"))
            .expect("config.example.toml must always parse and validate");
        // The example documents the defaults — hold it to that.
        let defaults = Config::default();
        assert_eq!(cfg.server.bind, defaults.server.bind);
        assert_eq!(
            cfg.limits.max_message_chars,
            defaults.limits.max_message_chars
        );
        assert_eq!(cfg.auth.session_ttl_days, defaults.auth.session_ttl_days);
        assert_eq!(cfg.push.driver, defaults.push.driver);
    }

    #[test]
    fn an_empty_config_uses_all_defaults_and_validates() {
        let cfg = Config::from_toml_str("").expect("empty config is valid");
        assert_eq!(cfg.server.bind, "127.0.0.1:8080");
        assert_eq!(cfg.database.port, 5432);
        assert_eq!(cfg.database.max_connections, 10);
        assert_eq!(cfg.auth.session_ttl_days, 180);
        assert_eq!(cfg.auth.session_touch_interval_mins, 15);
        assert_eq!(cfg.limits.max_message_chars, 4000);
        assert_eq!(cfg.limits.default_page_size, 50);
        assert_eq!(cfg.limits.max_page_size, 200);
        assert_eq!(cfg.limits.max_body_bytes, 16384);
        assert_eq!(cfg.limits.ws_send_queue, 64);
        assert_eq!(cfg.limits.ws_ping_interval_secs, 30);
        assert_eq!(cfg.limits.ws_idle_timeout_secs, 75);
        assert_eq!(cfg.push.driver, "log");
    }

    #[test]
    fn load_reads_and_validates_a_file_on_disk() {
        let mut f = NamedTempFile::new().expect("tempfile");
        f.write_all(b"[server]\nbind = \"0.0.0.0:9999\"\n")
            .expect("write");
        f.flush().expect("flush");
        let cfg = Config::load(f.path()).expect("load");
        assert_eq!(cfg.server.bind, "0.0.0.0:9999");
    }

    #[test]
    fn load_reports_a_missing_file_with_its_path() {
        let err = Config::load(Path::new("/nonexistent/family-connect/cfg.toml")).unwrap_err();
        assert!(format!("{err:#}").contains("reading config file"));
    }

    #[test]
    fn validate_rejects_an_unparseable_bind_address() {
        let err = Config::from_toml_str("[server]\nbind = \"not-an-addr\"\n").unwrap_err();
        assert!(format!("{err:#}").contains("server.bind"));
    }

    #[test]
    fn validate_rejects_an_idle_timeout_not_exceeding_the_ping_interval() {
        let err = Config::from_toml_str(
            "[limits]\nws_ping_interval_secs = 30\nws_idle_timeout_secs = 30\n",
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("ws_idle_timeout_secs"));
    }

    #[test]
    fn validate_rejects_a_max_page_size_below_the_default_page_size() {
        let err = Config::from_toml_str("[limits]\ndefault_page_size = 50\nmax_page_size = 10\n")
            .unwrap_err();
        assert!(format!("{err:#}").contains("max_page_size"));
    }

    #[test]
    fn validate_rejects_an_unknown_push_driver() {
        let err = Config::from_toml_str("[push]\ndriver = \"apns\"\n").unwrap_err();
        assert!(format!("{err:#}").contains("push.driver"));
    }

    #[test]
    fn validate_rejects_zero_valued_knobs() {
        for body in [
            "[database]\nmax_connections = 0\n",
            "[auth]\nsession_ttl_days = 0\n",
            "[limits]\nws_send_queue = 0\n",
            "[limits]\nmax_body_bytes = 100\n",
        ] {
            assert!(
                Config::from_toml_str(body).is_err(),
                "expected rejection for {body:?}"
            );
        }
    }
}
