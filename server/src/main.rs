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
use family_connect::{app, calls, db, handlers_attachment, handlers_chat, migrate, push};

#[derive(Parser, Debug)]
#[command(name = "family-connect", version, about = "Self-hosted family chat server")]
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

    // Bound argon2 before anything can serve a request: both endpoints that
    // hash are unauthenticated, and 19 MiB per hash across a 512-thread
    // blocking pool is how an unbounded login flood OOMs this process.
    family_connect::auth::configure_hash_concurrency(cfg.limits.max_password_hashes_in_flight);

    let pool = db::connect(&cfg.database).await?;
    migrate::run(&pool).await.context("running migrations")?;

    let push_sender = push::build(&cfg.push).context("building the push transports")?;
    let state = AppState::new(pool.clone(), Arc::new(cfg), push_sender);
    // Fail at BOOT if the attachments directory is unusable, rather than
    // on the family's first photo.
    state
        .storage
        .ensure_root()
        .await
        .context("preparing the attachments directory")?;

    // The registry owns the shutdown token because the WS connection tasks
    // (spawned by axum, out of our reach) must be able to observe it.
    let shutdown = state.registry.shutdown_token();

    // Two sweeps, one loop, at boot and hourly after: unclaimed uploads (a
    // send the user abandoned, up to 100 MB each) and messages past the
    // retention age. Nothing else in the system would ever remove either.
    {
        let sweeper_state = state.clone();
        let sweeper_shutdown = shutdown.clone();
        tokio::spawn(async move {
            loop {
                match handlers_attachment::sweep_unclaimed(&sweeper_state).await {
                    Ok(0) => {}
                    Ok(count) => info!(count, "swept unclaimed attachments"),
                    Err(err) => warn!(error = ?err, "sweeping unclaimed attachments failed"),
                }
                // Retention runs on the same clock: both are "delete what
                // nothing should be holding any more", and a second timer
                // would only be a second thing to reason about.
                match handlers_chat::sweep_expired_messages(&sweeper_state).await {
                    Ok(0) => {}
                    Ok(count) => info!(count, "swept messages past the retention age"),
                    Err(err) => warn!(error = ?err, "sweeping expired messages failed"),
                }
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(3600)) => {}
                    _ = sweeper_shutdown.cancelled() => break,
                }
            }
        });
    }

    // The calls sweeper: ends calls that rang out and the answered ones
    // nobody is connected to any more (docs/protocol.md, "Voice calls").
    // A 1 s tick, cancelled on shutdown.
    calls::spawn_sweeper(state.clone());

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
