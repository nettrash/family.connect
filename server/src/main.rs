//! family-connect binary — thin wrapper around the `family_connect` library.
//!
//! Startup: config -> pool -> migrations -> router -> serve. Shutdown:
//! SIGTERM/SIGINT cancels one token that both stops accepting connections
//! (axum graceful shutdown) and tells every WebSocket task to close with
//! 1001; the drain is bounded so a wedged connection cannot hold the
//! process hostage past systemd's TimeoutStopSec.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use tokio::signal::unix::{SignalKind, signal};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use family_connect::config::Config;
use family_connect::state::AppState;
use family_connect::{app, db, migrate, push};

#[derive(Parser, Debug)]
#[command(name = "family-connect", about = "Self-hosted family chat server")]
struct Cli {
    /// Path to the TOML configuration file.
    #[arg(short, long, default_value = "/etc/family-connect/config.toml")]
    config: PathBuf,

    /// Log level filter (overrides RUST_LOG).
    #[arg(long)]
    log_level: Option<String>,
}

fn init_tracing(level_override: Option<&str>) {
    let env_filter = match level_override {
        Some(level) => EnvFilter::new(level),
        None => EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
    };
    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(false)
        .init();
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.log_level.as_deref());

    info!(
        version = env!("CARGO_PKG_VERSION"),
        "family-connect starting"
    );

    let cfg = Config::load(&cli.config)
        .with_context(|| format!("loading config from {}", cli.config.display()))?;
    info!(
        bind = %cfg.server.bind,
        database = %db::connection_string(&cfg.database, true),
        "config loaded"
    );

    let pool = db::connect(&cfg.database).await?;
    migrate::run(&pool).await.context("running migrations")?;

    let push_sender = push::build(&cfg.push);
    let state = AppState::new(pool.clone(), Arc::new(cfg), push_sender);
    // The registry owns the shutdown token because the WS connection tasks
    // (spawned by axum, out of our reach) must be able to observe it.
    let shutdown = state.registry.shutdown_token();

    let listener = tokio::net::TcpListener::bind(&state.cfg.server.bind)
        .await
        .with_context(|| format!("binding {}", state.cfg.server.bind))?;
    info!(bind = %state.cfg.server.bind, "listening");

    let router = app::build_router(state.clone());
    let serve_shutdown = shutdown.clone();
    let mut server = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(serve_shutdown.cancelled_owned())
            .await
    });

    // Wait for a signal — or for the server to die on its own.
    let mut sigterm = signal(SignalKind::terminate()).context("install SIGTERM handler")?;
    let mut sigint = signal(SignalKind::interrupt()).context("install SIGINT handler")?;
    tokio::select! {
        _ = sigterm.recv() => info!("received SIGTERM, shutting down"),
        _ = sigint.recv()  => info!("received SIGINT, shutting down"),
        result = &mut server => {
            match result {
                Ok(Ok(())) => error!("server exited unexpectedly"),
                Ok(Err(err)) => error!(error = %err, "server failed"),
                Err(err) => error!(error = %err, "server task panicked"),
            }
            std::process::exit(1);
        }
    }
    shutdown.cancel();

    // Bounded drain: graceful shutdown waits for in-flight requests and for
    // the WS tasks (which see the cancelled token and close with 1001), but
    // never longer than this — systemd falls back to SIGKILL at 15s and we
    // want to beat it with a clean exit.
    let drain = tokio::time::Duration::from_secs(10);
    if tokio::time::timeout(drain, &mut server).await.is_err() {
        warn!("graceful shutdown timed out; abandoning remaining connections");
    } else {
        let lingering = state.registry.connection_count().await;
        if lingering > 0 {
            warn!(lingering, "connections still registered after drain");
        }
    }
    pool.close().await;
    info!("family-connect stopped");

    // Exit explicitly rather than returning: returning from `main` drops the
    // Tokio runtime, whose destructor blocks until every `spawn_blocking`
    // task finishes — a straggling argon2 hash would hang the process (and
    // `systemctl stop` with it). Everything worth flushing already was.
    std::process::exit(0);
}
