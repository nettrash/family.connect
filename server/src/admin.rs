//! The operator's commands for encryption at rest (#59, docs/operations.md,
//! "Encryption at rest" and "Restoring"): rotating the server key, forgetting
//! what a restore left without keys, and reading the assistant reports that
//! `psql` can only show sealed — and, since #89, seeing the families and
//! moving one between plans (docs/operations.md, "Paid families").
//!
//! They run from the same binary as the server (`family-connect storage-key
//! rotate`, `family-connect storage forget-missing`, `family-connect
//! reports`), with the server stopped — rotation takes the migration lock so a
//! server starting in the middle waits rather than reads half a table — and
//! print counts, never content, except `reports`, whose whole job is to show
//! the operator what was reported.

use anyhow::{Context, Result, bail};
use sqlx::{PgPool, Row};
use time::OffsetDateTime;

use crate::handlers_auth::{Scrubbed, scrub_account};
use crate::keyring::{Keyring, SealedRow};
use crate::migrate::{ADVISORY_LOCK_KEY, MIGRATIONS};
use crate::state::AppState;
use crate::vault::{Lookup, ServerKey};

/// What a rotation moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rotated {
    /// Storage keys re-wrapped under the new server key.
    pub keys: u64,
    /// Lookup values recomputed — they are keyed by the server key too.
    pub lookups: u64,
}

/// Move the database from server key `old` to `new`, in ONE transaction:
/// every storage key is re-wrapped, and every lookup column — usernames,
/// invite codes, push and VoIP tokens and App Store transaction ids, which
/// are HMACs under the server key — is recomputed from the value beside it. Nothing else is touched: the data
/// keys, and so every sealed value and file, stay as they are.
///
/// Refuses a database that is not at this version's schema (start the server
/// once first) or whose keys `old` does not open.
pub async fn rotate(pool: &PgPool, old: &ServerKey, new: &ServerKey) -> Result<Rotated> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(ADVISORY_LOCK_KEY)
        .execute(&mut *tx)
        .await
        .context("taking the migration lock")?;
    let latest: Option<i32> = sqlx::query_scalar("SELECT max(version) FROM schema_migrations")
        .fetch_one(&mut *tx)
        .await
        .context("reading the schema version")?;
    let expected = MIGRATIONS.last().map(|migration| migration.version);
    if latest != expected {
        bail!(
            "the database is at schema {latest:?}, this version expects {expected:?}: start the \
             server with this version once before rotating its key"
        );
    }

    let keyring = Keyring::new(old.clone());
    keyring.load(&mut tx).await?;
    let keys = keyring.rewrap(&mut tx, new).await?;

    let mut lookups = 0;
    let users = sqlx::query("SELECT id, username FROM users")
        .fetch_all(&mut *tx)
        .await?;
    for row in &users {
        let username = row.text(&keyring, "username")?;
        sqlx::query("UPDATE users SET username_key = $2 WHERE id = $1")
            .bind(row.get::<i64, _>("id"))
            .bind(new.lookup(Lookup::Username, &username))
            .execute(&mut *tx)
            .await?;
        lookups += 1;
    }
    let families = sqlx::query("SELECT id, invite_code FROM families")
        .fetch_all(&mut *tx)
        .await?;
    for row in &families {
        let code = row.text(&keyring, "invite_code")?;
        sqlx::query("UPDATE families SET invite_code_key = $2 WHERE id = $1")
            .bind(row.get::<i64, _>("id"))
            .bind(new.lookup(Lookup::InviteCode, &code))
            .execute(&mut *tx)
            .await?;
        lookups += 1;
    }
    let devices = sqlx::query("SELECT id, push_token, voip_token FROM devices")
        .fetch_all(&mut *tx)
        .await?;
    for row in &devices {
        let push = row.text_opt(&keyring, "push_token")?;
        let voip = row.text_opt(&keyring, "voip_token")?;
        sqlx::query("UPDATE devices SET push_token_key = $2, voip_token_key = $3 WHERE id = $1")
            .bind(row.get::<i64, _>("id"))
            .bind(push.map(|token| new.lookup(Lookup::PushToken, &token)))
            .bind(voip.map(|token| new.lookup(Lookup::VoipToken, &token)))
            .execute(&mut *tx)
            .await?;
        lookups += 1;
    }
    // App Store transaction ids (docs/protocol.md, "Paid families"): the
    // only way a notification names a subscription.
    let subscriptions =
        sqlx::query("SELECT id, original_transaction_id, latest_transaction_id FROM subscriptions")
            .fetch_all(&mut *tx)
            .await?;
    for row in &subscriptions {
        let original = row.text(&keyring, "original_transaction_id")?;
        let latest = row.text(&keyring, "latest_transaction_id")?;
        sqlx::query(
            "UPDATE subscriptions
                SET original_transaction_key = $2, latest_transaction_key = $3
              WHERE id = $1",
        )
        .bind(row.get::<i64, _>("id"))
        .bind(new.lookup(Lookup::AppStoreTransaction, &original))
        .bind(new.lookup(Lookup::AppStoreTransaction, &latest))
        .execute(&mut *tx)
        .await?;
        lookups += 1;
    }
    tx.commit().await?;
    Ok(Rotated { keys, lookups })
}

/// What `forget-missing` deleted again.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Forgotten {
    pub families: u64,
    pub conversations: u64,
    pub accounts: u64,
}

/// Delete again what a restore brought back without its key — what was
/// deleted after the dump was taken, whose key copy no longer holds its key
/// (docs/operations.md, "Restoring"). A family is deleted as a family is
/// (its direct chats first moved to a family the pair still shares), a
/// group, direct or assistant chat as a chat is, and an account is scrubbed
/// as a deleted account is — its tombstone sealed under a new key. Files no
/// row names any more are removed after.
pub async fn forget_missing(state: &AppState) -> Result<Forgotten> {
    let mut forgotten = Forgotten::default();

    let families: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM families f
          WHERE NOT EXISTS (SELECT 1 FROM storage_keys k WHERE k.id = f.key_id)
          ORDER BY id",
    )
    .fetch_all(&state.pool)
    .await?;
    for family_id in families {
        let mut tx = state.pool.begin().await?;
        let deleted = crate::handlers_family::delete_family_in_tx(&mut tx, family_id)
            .await
            .map_err(|err| anyhow::anyhow!("deleting family {family_id}: {err:?}"))?;
        tx.commit().await?;
        crate::handlers_attachment::remove_all_if_unreferenced(state, &deleted.storage_keys)
            .await
            .map_err(|err| anyhow::anyhow!("removing family {family_id}'s files: {err:?}"))?;
        forgotten.families += 1;
    }

    let chats: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM chats c
          WHERE c.kind <> 'family'
            AND NOT EXISTS (SELECT 1 FROM storage_keys k WHERE k.id = c.key_id)
          ORDER BY id",
    )
    .fetch_all(&state.pool)
    .await?;
    for chat_id in chats {
        let mut tx = state.pool.begin().await?;
        let storage_keys: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT a.storage_key FROM attachments a
               JOIN messages m ON m.id = a.message_id
              WHERE m.chat_id = $1",
        )
        .bind(chat_id)
        .fetch_all(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM chats WHERE id = $1")
            .bind(chat_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        crate::handlers_attachment::remove_all_if_unreferenced(state, &storage_keys)
            .await
            .map_err(|err| anyhow::anyhow!("removing chat {chat_id}'s files: {err:?}"))?;
        forgotten.conversations += 1;
    }

    let accounts: Vec<(i64, bool)> = sqlx::query_as(
        "SELECT id, deleted_at IS NOT NULL FROM users u
          WHERE NOT EXISTS (SELECT 1 FROM storage_keys k WHERE k.id = u.key_id)
          ORDER BY id",
    )
    .fetch_all(&state.pool)
    .await?;
    for (user_id, tombstone) in accounts {
        if tombstone {
            reseal_tombstone(state, user_id).await?;
        } else if scrub_account(state, user_id, None)
            .await
            .map_err(|err| anyhow::anyhow!("scrubbing account {user_id}: {err:?}"))?
            != Scrubbed::Done
        {
            bail!("account {user_id} has no key and could not be scrubbed");
        }
        forgotten.accounts += 1;
    }
    Ok(forgotten)
}

/// A tombstone whose key is gone: the same few words, sealed under a new
/// one. Nothing else of the account is left to lose.
async fn reseal_tombstone(state: &AppState, user_id: i64) -> Result<()> {
    let mut tx = state.pool.begin().await?;
    let key = state.keys.create(&mut tx).await?;
    let tombstone = format!("deleted-{user_id}");
    sqlx::query(
        "UPDATE users
            SET username = $2, username_key = $3, display_name = $4, password_hash = $5,
                key_id = $6, birthday = NULL
          WHERE id = $1",
    )
    .bind(user_id)
    .bind(key.seal_str(&tombstone))
    .bind(state.keys.lookup(Lookup::Username, &tombstone))
    .bind(key.seal_str("Deleted account"))
    .bind(key.seal_str("!"))
    .bind(key.id())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// One assistant report, as the operator reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct AssistantReportLine {
    pub id: i64,
    pub created_at: OffsetDateTime,
    pub reporter_user_id: i64,
    pub message_id: Option<i64>,
    pub chat_kind: String,
    pub reason: String,
    /// The frozen reply — `None` when its conversation has been deleted and
    /// its key with it, which is what deleting a conversation promises.
    pub excerpt: Option<String>,
    pub note: Option<String>,
}

/// The newest assistant reports, opened. A report whose conversation is gone
/// says so instead of failing the list.
pub async fn assistant_reports(
    pool: &PgPool,
    keys: &Keyring,
    limit: i64,
) -> Result<Vec<AssistantReportLine>> {
    let rows = sqlx::query(
        "SELECT id, created_at, reporter_user_id, message_id, chat_kind, reason,
                message_excerpt, note
           FROM assistant_reports
          ORDER BY created_at DESC, id DESC
          LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|row| AssistantReportLine {
            id: row.get("id"),
            created_at: row.get("created_at"),
            reporter_user_id: row.get("reporter_user_id"),
            message_id: row.get("message_id"),
            chat_kind: row.get("chat_kind"),
            reason: row.get("reason"),
            excerpt: row.text(keys, "message_excerpt").ok(),
            note: row.text_opt(keys, "note").ok().flatten(),
        })
        .collect())
}

/// One family as `family-connect families list` prints it.
#[derive(Debug, Clone, PartialEq)]
pub struct FamilyLine {
    pub id: i64,
    pub name: String,
    pub plan: String,
    pub owner_user_id: i64,
    pub members: i64,
    pub storage_bytes: i64,
    pub month_tokens: i64,
    /// The owner's subscription: when it ends and whether it covers now.
    pub subscription: Option<(OffsetDateTime, bool)>,
    /// When the server will delete it, while it is unpaid.
    pub delete_at: Option<OffsetDateTime>,
}

/// Every family, oldest first, with what payment means for it. Names are
/// opened for the operator's own terminal — this never reaches the journal.
pub async fn families(
    pool: &PgPool,
    keys: &Keyring,
    cfg: &crate::config::Config,
) -> Result<Vec<FamilyLine>> {
    let rows = sqlx::query(
        "SELECT f.id, f.name, f.plan, f.owner_user_id, f.unpaid_since,
                (SELECT count(*) FROM family_members fm WHERE fm.family_id = f.id) AS members
           FROM families f ORDER BY f.id",
    )
    .fetch_all(pool)
    .await?;
    let mut lines = Vec::with_capacity(rows.len());
    for row in rows {
        let id: i64 = row.get("id");
        let owner_user_id: i64 = row.get("owner_user_id");
        let plan: String = row.get("plan");
        let view = crate::subscriptions::plan_view(cfg, &plan, row.get("unpaid_since"));
        let subscription = crate::subscriptions::subscription_of(pool, cfg, owner_user_id)
            .await
            .map_err(|error| anyhow::anyhow!("reading a subscription: {error:?}"))?
            .map(|s| (s.expires_at, s.active));
        let storage_bytes = crate::subscriptions::storage_used(pool, id)
            .await
            .map_err(|error| anyhow::anyhow!("measuring storage: {error:?}"))?;
        let month_tokens = crate::subscriptions::month_use(pool, id)
            .await
            .map_err(|error| anyhow::anyhow!("reading usage: {error:?}"))?
            .tokens;
        lines.push(FamilyLine {
            id,
            name: row.text(keys, "name")?,
            plan,
            owner_user_id,
            members: row.get("members"),
            storage_bytes,
            month_tokens,
            subscription,
            delete_at: view.delete_at,
        });
    }
    Ok(lines)
}

/// Move a family between plans. Returns the plan it had. A family moved to
/// `paid` whose owner holds no subscription starts its days at the running
/// server's next sweep, which tells every member; one moved to `free` has its
/// deadline dropped there too — this command writes the plan and nothing
/// else, because the sweep is the one place that announces a deadline.
pub async fn set_plan(pool: &PgPool, family_id: i64, plan: &str) -> Result<String> {
    if plan != crate::subscriptions::FREE && plan != crate::subscriptions::PAID {
        bail!("a plan is \"free\" or \"paid\"");
    }
    let before: Option<String> = sqlx::query_scalar(
        "UPDATE families f SET plan = $2 FROM families old
          WHERE f.id = $1 AND old.id = f.id
          RETURNING old.plan",
    )
    .bind(family_id)
    .bind(plan)
    .fetch_optional(pool)
    .await?;
    before.with_context(|| format!("no family with id {family_id}"))
}
