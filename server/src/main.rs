//! family-connect binary — thin wrapper around the `family_connect` library.
//!
//! With no subcommand it is the server, so the systemd unit runs it as it
//! always has. The subcommands are the operator's tools for encryption at
//! rest (#59, docs/operations.md, "Encryption at rest" and "Restoring"):
//! `storage-key new|show|rotate`, `storage forget-missing` and `reports`.
//!
//! Startup: config -> pool -> migrations -> router -> serve. Shutdown:
//! SIGTERM/SIGINT cancels one token that both stops accepting connections
//! (axum graceful shutdown) and tells every WebSocket task to close with
//! 1001; the drain is bounded so a wedged connection cannot hold the
//! process hostage past systemd's TimeoutStopSec.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
#[cfg(unix)]
use tokio::signal::unix::{SignalKind, signal};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use family_connect::config::Config;
use family_connect::keyring::Keyring;
use family_connect::state::AppState;
use family_connect::vault::ServerKey;
use family_connect::{
    app, calls, db, greetings, handlers_attachment, handlers_auth, handlers_chat, handlers_family,
    migrate, push,
};

#[derive(Parser, Debug)]
#[command(
    name = "family-connect",
    version,
    about = "Self-hosted family chat server"
)]
struct Cli {
    /// Path to the TOML configuration file.
    #[arg(
        short,
        long,
        default_value = "/etc/family-connect/config.toml",
        global = true
    )]
    config: PathBuf,

    /// Log level filter (overrides RUST_LOG).
    #[arg(long, global = true)]
    log_level: Option<String>,

    /// Without one, run the server.
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// The server key everything stored is sealed under.
    #[command(subcommand)]
    StorageKey(StorageKeyCommand),
    /// What a restore needs.
    #[command(subcommand)]
    Storage(StorageCommand),
    /// The families on this server, and their plans (docs/operations.md,
    /// "Paid families").
    #[command(subcommand)]
    Families(FamiliesCommand),
    /// The newest reports about the assistant, opened — psql shows them sealed.
    Reports {
        /// How many, newest first.
        #[arg(long, default_value_t = 50)]
        limit: i64,
    },
}

#[derive(Subcommand, Debug)]
enum StorageKeyCommand {
    /// Write a new key file, readable by its owner only. Refuses to overwrite one.
    New {
        /// Where to write it, e.g. /etc/family-connect/storage.key.
        path: PathBuf,
    },
    /// Print the configured key and its fingerprint, to copy off the box by hand.
    Show,
    /// Move to a new key (server stopped): the storage keys are re-wrapped and
    /// the lookups recomputed in one transaction, the old file is kept as
    /// `<key_file>.old`, and the new one takes its place.
    Rotate,
}

#[derive(Subcommand, Debug)]
enum FamiliesCommand {
    /// Every family: its plan, owner, members, disk, this month's assistant
    /// tokens, the owner's subscription and any deletion date. Prints names,
    /// so it prints to this terminal only.
    List,
    /// Move a family between plans. A paid family whose owner has no
    /// subscription starts its days at the server's next hourly sweep.
    Plan {
        /// The family's id, from `families list`.
        id: i64,
        /// `free` or `paid`.
        plan: String,
    },
}

#[derive(Subcommand, Debug)]
enum StorageCommand {
    /// Delete again what a restore brought back without its key.
    ForgetMissing,
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
    if let Some(command) = cli.command {
        init_tracing(Some(cli.log_level.as_deref().unwrap_or("warn")));
        return run_command(&cli.config, command).await;
    }
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

    // The upgrade footgun, said out loud at boot: an `[ai]` section filled
    // in before `processor` existed is an assistant this server will not
    // advertise and no client may offer, because nobody can be asked
    // permission to send their words to an unnamed recipient
    // (docs/protocol.md, "Consenting to the assistant"). A WARN rather than
    // a refusal — a quiet assistant beats a chat that will not start.
    if cfg.ai.configured_but_nameless() {
        warn!(
            "[ai] is configured but names no processor, so the assistant is OFF. \
             Set `processor` to who actually answers — e.g. \
             processor = \"Microsoft — Azure OpenAI (Sweden Central)\" — and restart."
        );
    }

    // The server key before the database: without it nothing stored can be
    // opened, and an upgrade must not begin that it cannot finish.
    let key_file = cfg.storage.key_file.clone().context(
        "[storage] key_file is not set. Everything stored is sealed under the server key \
         (docs/operations.md, \"Encryption at rest\"): make one with \
         `family-connect storage-key new /etc/family-connect/storage.key` and name it there",
    )?;
    let keys = Arc::new(ServerKey::load(&key_file).map(Keyring::new)?);
    info!(server_key = keys.fingerprint(), "storage key loaded");

    let pool = db::connect(&cfg.database).await?;
    migrate::run(&pool, &keys)
        .await
        .context("running migrations")?;
    keys.start(&mut *pool.acquire().await?)
        .await
        .context("loading the storage keys")?;

    // With subscriptions off there are no deadlines; any left from when
    // they were on go now, so turning them on again later starts every
    // family's days afresh (docs/protocol.md, "Paid families").
    if !cfg.subscriptions.enabled {
        let dropped = family_connect::subscriptions::drop_deadlines(&pool)
            .await
            .map_err(|error| anyhow::anyhow!("dropping paid-family deadlines: {error:?}"))?;
        if dropped > 0 {
            info!(
                dropped,
                "subscriptions are off: paid-family deadlines dropped"
            );
        }
    }
    let push_sender = push::build(&cfg.push).context("building the push transports")?;
    let state = AppState::new(pool.clone(), Arc::new(cfg), push_sender, keys);
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

    // Three sweeps, one loop, at boot and hourly after: unclaimed uploads (a
    // send the user abandoned, up to 100 MB each), messages past the
    // retention age, and accounts past their familyless grace. Nothing else
    // in the system would ever remove any of them.
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
                // And accounts that never joined a family, or left one and
                // never came back, past their grace (docs/protocol.md,
                // "Accounts without a family"). Hourly rather than nightly
                // for the reason above — and so that "7 days" means seven
                // days, not "some night this week".
                match handlers_auth::sweep_familyless_accounts(&sweeper_state).await {
                    Ok(0) => {}
                    Ok(count) => info!(count, "swept accounts that have no family"),
                    Err(err) => warn!(error = ?err, "sweeping familyless accounts failed"),
                }
                // And invitations whose person and family owner share no
                // other family any more — the backstop for the one a
                // departure could not see because it was created in the
                // same instant (docs/protocol.md, "Several families").
                match handlers_family::sweep_unshared_invitations(&sweeper_state).await {
                    Ok(0) => {}
                    Ok(count) => info!(count, "lapsed invitations nobody shares a family for"),
                    Err(err) => warn!(error = ?err, "sweeping unshared invitations failed"),
                }
                // And paid families: who stopped being covered, who is
                // covered again, the day-before warning, and the families
                // whose deadline has come (docs/protocol.md, "Paid
                // families"). Hourly, so an expiry known in advance is
                // noticed within the hour and "14 days" means fourteen.
                match family_connect::subscriptions::sweep(&sweeper_state).await {
                    Ok(report) if report == Default::default() => {}
                    Ok(report) => info!(?report, "paid families swept"),
                    Err(err) => warn!(error = ?err, "sweeping paid families failed"),
                }
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(3600)) => {}
                    _ = sweeper_shutdown.cancelled() => break,
                }
            }
        });
    }

    // The files written before encryption at rest, sealed in the
    // background now that the database is (#59). Returns at once; a server
    // with nothing left to seal finds so in one query.
    family_connect::seal_files::spawn(state.clone());

    // The calls sweeper: ends calls that rang out and the answered ones
    // nobody is connected to any more (docs/protocol.md, "Voice calls").
    // A 1 s tick, cancelled on shutdown.
    calls::spawn_sweeper(state.clone());

    // The daily greeting (docs/protocol.md, "The daily greeting"). Its own
    // ticker rather than another job in the hourly loop above: that loop is
    // deletes on one clock, and this is a WRITE on a wall-clock time an
    // operator chose to the minute. Returns immediately, having spawned
    // nothing, on a server with greetings off — which is every server that
    // has not been told otherwise.
    greetings::spawn_sweeper(state.clone());

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
    #[cfg(unix)]
    {
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
    }
    // Off Unix there is no SIGTERM: Ctrl+C is how a developer stops a local
    // fixture (the Microsoft Store screenshots are shot against one on
    // Windows, `win/store/seed-store-screenshots.ps1`). The server ships for
    // Linux, where the block above is what runs.
    #[cfg(not(unix))]
    {
        tokio::select! {
            result = tokio::signal::ctrl_c() => {
                result.context("install Ctrl+C handler")?;
                info!("received Ctrl+C, shutting down");
            }
            result = &mut server => {
                match result {
                    Ok(Ok(())) => error!("server exited unexpectedly"),
                    Ok(Err(err)) => error!(error = %err, "server failed"),
                    Err(err) => error!(error = %err, "server task panicked"),
                }
                std::process::exit(1);
            }
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

/// The configured server key: the file `[storage] key_file` names.
fn configured_key(cfg: &Config) -> Result<(PathBuf, ServerKey)> {
    let key_file = cfg
        .storage
        .key_file
        .clone()
        .context("[storage] key_file is not set")?;
    let key = ServerKey::load(&key_file)?;
    Ok((key_file, key))
}

/// One of the operator's commands. Each prints what it did — counts and
/// fingerprints, never content, except the reports it was asked to show.
async fn run_command(config: &std::path::Path, command: Command) -> Result<()> {
    if let Command::StorageKey(StorageKeyCommand::New { path }) = &command {
        let key = ServerKey::generate();
        key.create_file(path)?;
        println!("wrote a new storage key to {}", path.display());
        println!("fingerprint {}", key.fingerprint());
        println!(
            "Make it the service user's (chown family-connect:family-connect), name it in \
             [storage] key_file, and copy it off the box now: family-connect storage-key show"
        );
        return Ok(());
    }
    let cfg = Config::load(config)
        .with_context(|| format!("loading config from {}", config.display()))?;
    let (key_file, key) = configured_key(&cfg)?;
    match command {
        Command::StorageKey(StorageKeyCommand::New { .. }) => unreachable!("handled above"),
        Command::StorageKey(StorageKeyCommand::Show) => {
            println!("storage key  {}", key.encoded());
            println!("fingerprint  {}", key.fingerprint());
        }
        Command::StorageKey(StorageKeyCommand::Rotate) => rotate(&cfg, &key_file, &key).await?,
        Command::Storage(StorageCommand::ForgetMissing) => {
            let keys = Arc::new(Keyring::new(key));
            let pool = db::connect(&cfg.database).await?;
            keys.load(&mut *pool.acquire().await?).await?;
            let state = AppState::new(
                pool.clone(),
                Arc::new(cfg),
                Arc::new(push::LogPushSender),
                keys.clone(),
            );
            let forgotten = family_connect::admin::forget_missing(&state).await?;
            keys.start(&mut *pool.acquire().await?).await?;
            println!(
                "deleted again: {} families, {} conversations, {} accounts",
                forgotten.families, forgotten.conversations, forgotten.accounts
            );
        }
        Command::Families(FamiliesCommand::List) => {
            let keys = Keyring::new(key);
            let pool = db::connect(&cfg.database).await?;
            keys.load(&mut *pool.acquire().await?).await?;
            if !cfg.subscriptions.enabled {
                println!("[subscriptions] is off: plans are kept, and nothing reads them");
            }
            for family in family_connect::admin::families(&pool, &keys, &cfg).await? {
                let subscription = match family.subscription {
                    None => "no subscription".to_string(),
                    Some((ends, true)) => format!("subscribed until {}", ends.date()),
                    Some((ends, false)) => format!("subscription ended {}", ends.date()),
                };
                let deadline = family
                    .delete_at
                    .map(|at| format!(" — DELETED ON {}", at.date()))
                    .unwrap_or_default();
                println!(
                    "#{} {} — {}, owner account {} ({}), {} members, {} MB, {} tokens this \
                     month{}",
                    family.id,
                    family.name,
                    family.plan,
                    family.owner_user_id,
                    subscription,
                    family.members,
                    family.storage_bytes / (1024 * 1024),
                    family.month_tokens,
                    deadline,
                );
            }
        }
        Command::Families(FamiliesCommand::Plan { id, plan }) => {
            let pool = db::connect(&cfg.database).await?;
            let before = family_connect::admin::set_plan(&pool, id, plan.trim()).await?;
            println!("family #{id}: {before} -> {}", plan.trim());
            if cfg.subscriptions.enabled && plan.trim() == family_connect::subscriptions::PAID {
                println!(
                    "if its owner holds no subscription, its days start at the server's next \
                     hourly sweep, and every member is told"
                );
            }
        }
        Command::Reports { limit } => {
            let keys = Keyring::new(key);
            let pool = db::connect(&cfg.database).await?;
            keys.load(&mut *pool.acquire().await?).await?;
            for report in family_connect::admin::assistant_reports(&pool, &keys, limit).await? {
                println!(
                    "#{} {} — {} report by account {} on message {} ({})",
                    report.id,
                    report.created_at,
                    report.reason,
                    report.reporter_user_id,
                    report
                        .message_id
                        .map_or_else(|| "(swept)".to_string(), |id| id.to_string()),
                    report.chat_kind,
                );
                match report.excerpt {
                    Some(excerpt) => println!("  reply: {excerpt}"),
                    None => println!("  reply: (gone with its conversation)"),
                }
                if let Some(note) = report.note {
                    println!("  note:  {note}");
                }
            }
        }
    }
    Ok(())
}

/// `storage-key rotate`: the new key is written beside the old one first,
/// the database moves to it in one transaction, and only then do the files
/// trade places — the old one kept as `<key_file>.old`, for the key copies
/// in the backups that were wrapped by it (docs/operations.md, "Rotating
/// the key").
async fn rotate(cfg: &Config, key_file: &std::path::Path, old: &ServerKey) -> Result<()> {
    let new_file = PathBuf::from(format!("{}.new", key_file.display()));
    let old_file = PathBuf::from(format!("{}.old", key_file.display()));
    if old_file.exists() {
        bail!(
            "{} exists: a previous rotation's old key. Destroy it once its 7 days are over, \
             then rotate again",
            old_file.display()
        );
    }
    let new = ServerKey::generate();
    new.create_file(&new_file)?;
    // The service user must be able to read the new file as it could the old.
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let meta = std::fs::metadata(key_file)?;
        std::os::unix::fs::chown(&new_file, Some(meta.uid()), Some(meta.gid()))
            .with_context(|| format!("giving {} to the old key's owner", new_file.display()))?;
    }
    let pool = db::connect(&cfg.database).await?;
    let rotated = match family_connect::admin::rotate(&pool, old, &new).await {
        Ok(rotated) => rotated,
        Err(error) => {
            // Nothing in the database moved; neither does anything on disk.
            let _ = std::fs::remove_file(&new_file);
            return Err(error);
        }
    };
    std::fs::rename(key_file, &old_file)
        .with_context(|| format!("keeping the old key as {}", old_file.display()))?;
    std::fs::rename(&new_file, key_file)
        .with_context(|| format!("putting the new key at {}", key_file.display()))?;
    println!(
        "rotated: {} storage keys re-wrapped, {} lookups recomputed",
        rotated.keys, rotated.lookups
    );
    println!("new storage key  {}", new.encoded());
    println!("fingerprint      {}", new.fingerprint());
    println!(
        "Copy the new key off the box now. Keep {} for 7 days, then destroy it.",
        old_file.display()
    );
    Ok(())
}
