//! Hand-rolled migration runner.
//!
//! sqlx's `migrate` feature is deliberately not used: this runner is ~60
//! lines, has no directory-scanning or checksum magic, and its behavior is
//! fully visible here. Concurrency safety comes from a Postgres advisory
//! lock (several server instances may start against the same database, e.g.
//! during a rolling deploy); the lock is session-scoped, so the code is
//! careful to take and release it on the *same* pooled connection and to
//! release it even when a migration fails — otherwise the lock would leak
//! back into the pool still held.

use anyhow::{Context, Result};
use sqlx::{Acquire, PgConnection, PgPool};
use tracing::info;

/// A single migration step, embedded in the binary at compile time so the
/// deployed artifact never depends on files on disk.
pub struct Migration {
    pub version: i32,
    pub name: &'static str,
    pub sql: &'static str,
}

/// All migrations, in application order. Append-only.
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "init",
        sql: include_str!("../migrations/0001_init.sql"),
    },
    Migration {
        version: 2,
        name: "reactions",
        sql: include_str!("../migrations/0002_reactions.sql"),
    },
    Migration {
        version: 3,
        name: "avatars",
        sql: include_str!("../migrations/0003_avatars.sql"),
    },
    Migration {
        version: 4,
        name: "avatar_seq",
        sql: include_str!("../migrations/0004_avatar_seq.sql"),
    },
    Migration {
        version: 5,
        name: "replies",
        sql: include_str!("../migrations/0005_replies.sql"),
    },
    Migration {
        version: 6,
        name: "reply_index",
        sql: include_str!("../migrations/0006_reply_index.sql"),
    },
    Migration {
        version: 7,
        name: "edits",
        sql: include_str!("../migrations/0007_edits.sql"),
    },
    Migration {
        version: 8,
        name: "board",
        sql: include_str!("../migrations/0008_board.sql"),
    },
    Migration {
        version: 9,
        name: "attachments",
        sql: include_str!("../migrations/0009_attachments.sql"),
    },
    Migration {
        version: 10,
        name: "attachment_files",
        sql: include_str!("../migrations/0010_attachment_files.sql"),
    },
];

/// Arbitrary but stable key identifying "family-connect migrations" among
/// advisory locks on the database ("famconn" left-padded, as it were).
const ADVISORY_LOCK_KEY: i64 = 0x66616d_636f6e6e;

/// Apply every migration that is not yet recorded in `schema_migrations`.
pub async fn run(pool: &PgPool) -> Result<()> {
    let mut conn = pool
        .acquire()
        .await
        .context("acquiring a connection for migrations")?;

    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(ADVISORY_LOCK_KEY)
        .execute(&mut *conn)
        .await
        .context("taking the migration advisory lock")?;

    // Run the body, then unlock unconditionally before propagating any error.
    let result = apply_pending(&mut conn).await;

    let unlock = sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(ADVISORY_LOCK_KEY)
        .execute(&mut *conn)
        .await
        .context("releasing the migration advisory lock");

    result?;
    unlock?;
    Ok(())
}

async fn apply_pending(conn: &mut PgConnection) -> Result<()> {
    sqlx::raw_sql(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
             version    INTEGER PRIMARY KEY,
             name       TEXT NOT NULL,
             applied_at TIMESTAMPTZ NOT NULL DEFAULT now()
         )",
    )
    .execute(&mut *conn)
    .await
    .context("creating schema_migrations")?;

    let applied: Vec<i32> = sqlx::query_scalar("SELECT version FROM schema_migrations")
        .fetch_all(&mut *conn)
        .await
        .context("reading applied migrations")?;

    for migration in MIGRATIONS {
        if applied.contains(&migration.version) {
            continue;
        }
        // Each migration commits atomically together with its bookkeeping
        // row: either both land or neither does.
        let mut tx = conn.begin().await.context("beginning migration tx")?;
        sqlx::raw_sql(migration.sql)
            .execute(&mut *tx)
            .await
            .with_context(|| {
                format!(
                    "applying migration {} ({})",
                    migration.version, migration.name
                )
            })?;
        sqlx::query("INSERT INTO schema_migrations (version, name) VALUES ($1, $2)")
            .bind(migration.version)
            .bind(migration.name)
            .execute(&mut *tx)
            .await
            .context("recording migration")?;
        tx.commit().await.context("committing migration")?;
        info!(
            version = migration.version,
            name = migration.name,
            "migration applied"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_migration_list_is_nonempty_with_strictly_increasing_versions() {
        assert!(!MIGRATIONS.is_empty());
        for pair in MIGRATIONS.windows(2) {
            assert!(
                pair[0].version < pair[1].version,
                "migration versions must strictly increase"
            );
        }
    }

    #[test]
    fn every_migration_embeds_nonempty_sql_and_a_name() {
        for m in MIGRATIONS {
            assert!(
                !m.sql.trim().is_empty(),
                "migration {} has empty SQL",
                m.version
            );
            assert!(!m.name.is_empty(), "migration {} has no name", m.version);
        }
    }
}
