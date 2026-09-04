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
    Migration {
        version: 11,
        name: "attachment_dedup",
        sql: include_str!("../migrations/0011_attachment_dedup.sql"),
    },
    Migration {
        version: 12,
        name: "retention",
        sql: include_str!("../migrations/0012_retention.sql"),
    },
    Migration {
        version: 13,
        name: "macos_devices",
        sql: include_str!("../migrations/0013_macos_devices.sql"),
    },
    Migration {
        version: 14,
        name: "attachment_audio",
        sql: include_str!("../migrations/0014_attachment_audio.sql"),
    },
    Migration {
        version: 15,
        name: "assistant",
        sql: include_str!("../migrations/0015_assistant.sql"),
    },
    Migration {
        version: 16,
        name: "attachment_location",
        sql: include_str!("../migrations/0016_attachment_location.sql"),
    },
    Migration {
        version: 17,
        name: "family_language",
        sql: include_str!("../migrations/0017_family_language.sql"),
    },
    Migration {
        version: 18,
        name: "member_birthday",
        sql: include_str!("../migrations/0018_member_birthday.sql"),
    },
    Migration {
        version: 19,
        name: "family_ai_history",
        sql: include_str!("../migrations/0019_family_ai_history.sql"),
    },
    Migration {
        version: 20,
        name: "device_session",
        sql: include_str!("../migrations/0020_device_session.sql"),
    },
    Migration {
        version: 21,
        name: "device_session_revoked",
        sql: include_str!("../migrations/0021_device_session_revoked.sql"),
    },
    Migration {
        version: 22,
        name: "polls",
        sql: include_str!("../migrations/0022_polls.sql"),
    },
    Migration {
        version: 23,
        name: "account_deletion",
        sql: include_str!("../migrations/0023_account_deletion.sql"),
    },
    Migration {
        version: 24,
        name: "calls",
        sql: include_str!("../migrations/0024_calls.sql"),
    },
    Migration {
        version: 25,
        name: "attachment_sets",
        sql: include_str!("../migrations/0025_attachment_sets.sql"),
    },
    Migration {
        version: 26,
        name: "call_video",
        sql: include_str!("../migrations/0026_call_video.sql"),
    },
    Migration {
        version: 27,
        name: "note_size",
        sql: include_str!("../migrations/0027_note_size.sql"),
    },
    Migration {
        version: 28,
        name: "member_blocks",
        sql: include_str!("../migrations/0028_member_blocks.sql"),
    },
    Migration {
        version: 29,
        name: "member_reports",
        sql: include_str!("../migrations/0029_member_reports.sql"),
    },
    Migration {
        version: 30,
        name: "family_membership",
        sql: include_str!("../migrations/0030_family_membership.sql"),
    },
    Migration {
        version: 31,
        name: "note_content_seq",
        sql: include_str!("../migrations/0031_note_content_seq.sql"),
    },
    Migration {
        version: 32,
        name: "ai_pictures",
        sql: include_str!("../migrations/0032_ai_pictures.sql"),
    },
    Migration {
        version: 33,
        name: "ai_history_photos",
        sql: include_str!("../migrations/0033_ai_history_photos.sql"),
    },
    Migration {
        version: 34,
        name: "familyless_since",
        sql: include_str!("../migrations/0034_familyless_since.sql"),
    },
    Migration {
        version: 35,
        name: "expired_attachments",
        sql: include_str!("../migrations/0035_expired_attachments.sql"),
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
