//! Paid families (docs/protocol.md, "Paid families"; issue #89).
//!
//! Everything here is about one question asked many ways: is this family
//! paid for? A family is `free` or `paid`; a paid one is COVERED while its
//! current owner holds an ACTIVE App Store subscription — one not revoked
//! whose expiry, or Apple's grace end, is still ahead. What Apple signed is
//! believed in `crate::appstore`; this module keeps what was believed, says
//! who is covered, runs the hourly deadline, and measures a paid family's
//! allowance.
//!
//! With `[subscriptions] enabled` false, nothing in here changes anything:
//! no plan on the wire, no ceiling beyond the operator's, no deadlines and
//! no allowance.

use serde::Serialize;
use sqlx::{PgConnection, PgPool, Row};
use time::{Date, Duration, Month, OffsetDateTime, Time};
use tracing::{info, warn};
use uuid::Uuid;

use crate::appstore::{Renewal, Transaction};
use crate::config::Config;
use crate::error::ApiError;
use crate::events;
use crate::keyring::{Keyring, Owner, SealedRow};
use crate::push_payload::PlanStage;
use crate::state::AppState;
use crate::vault::Lookup;

pub const FREE: &str = "free";
pub const PAID: &str = "paid";

/// The plan a family created now is given.
pub fn plan_for_new_family(cfg: &Config) -> &'static str {
    if cfg.subscriptions.enabled {
        PAID
    } else {
        FREE
    }
}

/// The most members a family of `plan` may have, whatever its owner sets:
/// the operator's ceiling, and for a paid family the lower paid one.
pub fn member_ceiling(cfg: &Config, plan: &str) -> i64 {
    let operator = cfg.limits.max_family_members;
    match cfg.subscriptions.member_limit() {
        Some(paid) if cfg.subscriptions.enabled && plan == PAID => operator.min(paid),
        _ => operator,
    }
}

/// The three `Family` fields this feature adds, as one family shows them.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanView {
    pub member_ceiling: i64,
    pub plan: Option<String>,
    pub delete_at: Option<OffsetDateTime>,
}

pub fn plan_view(cfg: &Config, plan: &str, unpaid_since: Option<OffsetDateTime>) -> PlanView {
    let enabled = cfg.subscriptions.enabled;
    PlanView {
        member_ceiling: member_ceiling(cfg, plan),
        plan: enabled.then(|| plan.to_string()),
        delete_at: unpaid_since
            .filter(|_| enabled && plan == PAID)
            .map(|since| since + Duration::days(cfg.subscriptions.unpaid_days)),
    }
}

/// The first instant of `at`'s UTC month.
pub fn month_start(at: OffsetDateTime) -> OffsetDateTime {
    let at = at.to_offset(time::UtcOffset::UTC);
    let first = Date::from_calendar_date(at.year(), at.month(), 1).expect("the 1st exists");
    first.with_time(Time::MIDNIGHT).assume_utc()
}

/// The first instant of the UTC month after `at`'s — when a monthly
/// allowance starts again.
pub fn next_month_start(at: OffsetDateTime) -> OffsetDateTime {
    let start = month_start(at);
    let (year, month) = match start.month() {
        Month::December => (start.year() + 1, Month::January),
        month => (start.year(), month.next()),
    };
    Date::from_calendar_date(year, month, 1)
        .expect("the 1st exists")
        .with_time(Time::MIDNIGHT)
        .assume_utc()
}

/// SQL for "does `owner` hold an active subscription right now?". `owner` is
/// an SQL expression. The environments are spliced in as literals: config
/// validation holds them to "Production" and "Sandbox", so nothing else can
/// reach this string.
pub fn active_sql(cfg: &Config, owner: &str) -> String {
    format!(
        "EXISTS (SELECT 1 FROM subscriptions s
                  WHERE s.user_id = {owner} AND {}
                    AND s.environment IN ({}))",
        LIVE_SQL,
        environments_sql(cfg)
    )
}

/// Not revoked, and ending in the future — Apple's grace period included.
const LIVE_SQL: &str = "s.revoked_at IS NULL
                    AND GREATEST(s.expires_at, COALESCE(s.grace_until, s.expires_at)) > now()";

fn environments_sql(cfg: &Config) -> String {
    cfg.subscriptions
        .environments
        .iter()
        .map(|e| format!("'{}'", e.replace('\'', "")))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A caller's subscription as `GET /me` and `PUT /me/subscription` show it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SubscriptionView {
    pub product_id: String,
    pub environment: String,
    /// When it stops covering anything: the expiry, or Apple's grace end when
    /// that is later — or, for one refunded or revoked, the moment it was.
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub renews: Option<bool>,
}

/// The caller's active subscription, or else the one that ended last.
pub async fn subscription_of(
    pool: &PgPool,
    cfg: &Config,
    user_id: i64,
) -> Result<Option<SubscriptionView>, ApiError> {
    let row = sqlx::query(&format!(
        "SELECT s.product_id, s.environment, s.renews,
                CASE WHEN s.revoked_at IS NULL
                     THEN GREATEST(s.expires_at, COALESCE(s.grace_until, s.expires_at))
                     ELSE s.revoked_at END AS ends,
                ({LIVE_SQL} AND s.environment IN ({envs})) AS active
           FROM subscriptions s
          WHERE s.user_id = $1
          ORDER BY active DESC, ends DESC, s.id DESC
          LIMIT 1",
        envs = environments_sql(cfg)
    ))
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|row| SubscriptionView {
        product_id: row.get("product_id"),
        environment: row.get("environment"),
        expires_at: row.get("ends"),
        active: row.get("active"),
        renews: row.get("renews"),
    }))
}

/// The UUID this account's purchases carry as Apple's `appAccountToken`,
/// made the first time it is asked for and sealed under the account's key.
///
/// Two first reads at once must not hand out two tokens — a purchase made
/// with the loser would never match — so the write only fills an EMPTY
/// column, and whatever is there afterwards is the answer.
pub async fn app_account_token(state: &AppState, user_id: i64) -> Result<Uuid, ApiError> {
    if let Some(token) = read_token(state, user_id).await? {
        return Ok(token);
    }
    let key = state.keys.key(&state.pool, Owner::Account(user_id)).await?;
    let fresh = Uuid::new_v4();
    sqlx::query(
        "UPDATE users SET app_account_token = $2 WHERE id = $1 AND app_account_token IS NULL",
    )
    .bind(user_id)
    .bind(key.seal_str(&fresh.to_string()))
    .execute(&state.pool)
    .await?;
    read_token(state, user_id)
        .await?
        .ok_or_else(|| ApiError::Internal(anyhow::anyhow!("account {user_id} has no token")))
}

async fn read_token(state: &AppState, user_id: i64) -> Result<Option<Uuid>, ApiError> {
    let row = sqlx::query("SELECT app_account_token FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&state.pool)
        .await?;
    let Some(text) = row.text_opt(&state.keys, "app_account_token")? else {
        return Ok(None);
    };
    Uuid::parse_str(&text)
        .map(Some)
        .map_err(|_| ApiError::Internal(anyhow::anyhow!("account {user_id} has a broken token")))
}

/// What applying a transaction did.
#[derive(Debug, PartialEq, Eq)]
pub enum Applied {
    /// Stored, or already known and brought up to date.
    Stored,
    /// It belongs to another account (`subscription_taken`).
    Taken,
    /// Nobody has presented it yet — a notification for a purchase the app
    /// has not handed over. Dropped; the app will.
    Unknown,
}

/// Fold a believed transaction into the subscription it belongs to, inside
/// the caller's transaction. `presenter` is the account handing it over
/// (`PUT /me/subscription`), or `None` for Apple's notification, which may
/// update a subscription and never creates one.
///
/// By STATE: the subscription remembers the transaction with the latest
/// expiry. A later period replaces it; the SAME transaction signed later
/// (a refund, a reversed refund) updates its revocation; anything else — an
/// older period, an older copy — changes nothing. So notifications may
/// arrive in any order, and a refund of last month does not end this one.
pub async fn apply_transaction(
    tx: &mut PgConnection,
    keys: &Keyring,
    presenter: Option<i64>,
    txn: &Transaction,
) -> Result<Applied, ApiError> {
    let original = keys.lookup(Lookup::AppStoreTransaction, &txn.original_transaction_id);
    let latest = keys.lookup(Lookup::AppStoreTransaction, &txn.transaction_id);
    let held = sqlx::query(
        "SELECT id, user_id, latest_transaction_key, expires_at, signed_at
           FROM subscriptions WHERE original_transaction_key = $1 FOR UPDATE",
    )
    .bind(&original)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(held) = held else {
        let Some(user_id) = presenter else {
            return Ok(Applied::Unknown);
        };
        // The ids sealed beside their lookups, under the account's key, so a
        // key rotation can recompute the lookups (see migration 0061).
        let key = keys.key(&mut *tx, Owner::Account(user_id)).await?;
        // Two first presentations at once: the unique key makes the second
        // a no-op, and the re-read below decides whose it is.
        sqlx::query(
            "INSERT INTO subscriptions (user_id, original_transaction_id, original_transaction_key,
                                        latest_transaction_id, latest_transaction_key,
                                        product_id, environment, expires_at, revoked_at, signed_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
             ON CONFLICT (original_transaction_key) DO NOTHING",
        )
        .bind(user_id)
        .bind(key.seal_str(&txn.original_transaction_id))
        .bind(&original)
        .bind(key.seal_str(&txn.transaction_id))
        .bind(&latest)
        .bind(&txn.product_id)
        .bind(&txn.environment)
        .bind(txn.expires)
        .bind(txn.revoked)
        .bind(txn.signed)
        .execute(&mut *tx)
        .await?;
        let owner: i64 = sqlx::query_scalar(
            "SELECT user_id FROM subscriptions WHERE original_transaction_key = $1",
        )
        .bind(&original)
        .fetch_one(&mut *tx)
        .await?;
        return Ok(if owner == user_id {
            Applied::Stored
        } else {
            Applied::Taken
        });
    };
    let id: i64 = held.get("id");
    let owner: i64 = held.get("user_id");
    if presenter.is_some_and(|user_id| user_id != owner) {
        return Ok(Applied::Taken);
    }
    let held_expires: OffsetDateTime = held.get("expires_at");
    let held_signed: OffsetDateTime = held.get("signed_at");
    let held_latest: Vec<u8> = held.get("latest_transaction_key");
    if txn.expires > held_expires {
        let key = keys.key(&mut *tx, Owner::Account(owner)).await?;
        sqlx::query(
            "UPDATE subscriptions
                SET latest_transaction_key = $2, product_id = $3, environment = $4,
                    expires_at = $5, revoked_at = $6, signed_at = $7, updated_at = now(),
                    latest_transaction_id = $8
              WHERE id = $1",
        )
        .bind(id)
        .bind(&latest)
        .bind(&txn.product_id)
        .bind(&txn.environment)
        .bind(txn.expires)
        .bind(txn.revoked)
        .bind(txn.signed)
        .bind(key.seal_str(&txn.transaction_id))
        .execute(&mut *tx)
        .await?;
    } else if latest == held_latest && txn.signed > held_signed {
        sqlx::query(
            "UPDATE subscriptions
                SET product_id = $2, environment = $3, revoked_at = $4, signed_at = $5,
                    updated_at = now()
              WHERE id = $1",
        )
        .bind(id)
        .bind(&txn.product_id)
        .bind(&txn.environment)
        .bind(txn.revoked)
        .bind(txn.signed)
        .execute(&mut *tx)
        .await?;
    }
    Ok(Applied::Stored)
}

/// Fold believed renewal info in: whether it renews, and Apple's grace end.
/// Only a newer copy (by `signedDate`) changes anything. Returns the account
/// the subscription belongs to, or `None` when nobody has presented it.
pub async fn apply_renewal(
    conn: &mut PgConnection,
    keys: &Keyring,
    renewal: &Renewal,
) -> Result<Option<i64>, ApiError> {
    let original = keys.lookup(
        Lookup::AppStoreTransaction,
        &renewal.original_transaction_id,
    );
    sqlx::query(
        "UPDATE subscriptions
            SET renews = $2, grace_until = $3, renewal_signed_at = $4, updated_at = now()
          WHERE original_transaction_key = $1
            AND (renewal_signed_at IS NULL OR renewal_signed_at < $4)",
    )
    .bind(&original)
    .bind(renewal.renews)
    .bind(renewal.grace_until)
    .bind(renewal.signed)
    .execute(&mut *conn)
    .await?;
    Ok(
        sqlx::query_scalar("SELECT user_id FROM subscriptions WHERE original_transaction_key = $1")
            .bind(&original)
            .fetch_optional(&mut *conn)
            .await?,
    )
}

/// The account a subscription belongs to, by Apple's original id.
pub async fn owner_of_in(
    conn: &mut PgConnection,
    keys: &Keyring,
    original_transaction_id: &str,
) -> Result<Option<i64>, ApiError> {
    Ok(
        sqlx::query_scalar("SELECT user_id FROM subscriptions WHERE original_transaction_key = $1")
            .bind(keys.lookup(Lookup::AppStoreTransaction, original_transaction_id))
            .fetch_optional(&mut *conn)
            .await?,
    )
}

/// Whether `user_id` holds an active subscription now.
pub async fn is_active(pool: &PgPool, cfg: &Config, user_id: i64) -> Result<bool, ApiError> {
    Ok(
        sqlx::query_scalar(&format!("SELECT {}", active_sql(cfg, "$1")))
            .bind(user_id)
            .fetch_one(pool)
            .await?,
    )
}

/// The families this account owns that are covered AGAIN: their deadline
/// goes at once, and every member is told by `family_plan`. Called after
/// `PUT /me/subscription` and an App Store notification; everything else
/// that can cover a family reaches it at the next sweep.
pub async fn settle_owner(state: &AppState, user_id: i64) -> Result<(), ApiError> {
    if !state.cfg.subscriptions.enabled {
        return Ok(());
    }
    let cleared: Vec<i64> = sqlx::query_scalar(&format!(
        "UPDATE families f SET unpaid_since = NULL, unpaid_warned = false
          WHERE f.owner_user_id = $1 AND f.unpaid_since IS NOT NULL AND {}
          RETURNING f.id",
        active_sql(&state.cfg, "f.owner_user_id")
    ))
    .bind(user_id)
    .fetch_all(&state.pool)
    .await?;
    for family_id in cleared {
        info!(family_id, "family covered again");
        events::log_fanout_error(
            "family_plan",
            events::deliver_family_plan(state, family_id, PAID, None).await,
        );
    }
    Ok(())
}

/// With subscriptions OFF there are no deadlines: any left from when they
/// were on is dropped at boot, so turning them on again later starts every
/// family's 14 days afresh, with fresh warnings — never a deletion the
/// moment the switch flips because a deadline passed while it was off.
pub async fn drop_deadlines(pool: &PgPool) -> Result<u64, ApiError> {
    Ok(sqlx::query(
        "UPDATE families SET unpaid_since = NULL, unpaid_warned = false
          WHERE unpaid_since IS NOT NULL",
    )
    .execute(pool)
    .await?
    .rows_affected())
}

/// What one sweep did, for the log and the tests.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SweepReport {
    pub unpaid: usize,
    pub covered: usize,
    pub warned: usize,
    pub deleted: usize,
}

/// The hourly deadline (docs/protocol.md, "Paid families"): mark the paid
/// families that are no longer covered, clear the ones that are again, warn
/// the day before, and delete the ones whose day has come.
pub async fn sweep(state: &AppState) -> Result<SweepReport, ApiError> {
    let mut report = SweepReport::default();
    let cfg = &state.cfg;
    if !cfg.subscriptions.enabled {
        return Ok(report);
    }
    let active = active_sql(cfg, "f.owner_user_id");

    // Covered again — or no longer paid: the operator made it free.
    let cleared: Vec<(i64, String)> = sqlx::query_as(&format!(
        "UPDATE families f SET unpaid_since = NULL, unpaid_warned = false
          WHERE f.unpaid_since IS NOT NULL AND (f.plan <> 'paid' OR {active})
          RETURNING f.id, f.plan"
    ))
    .fetch_all(&state.pool)
    .await?;
    for (family_id, plan) in cleared {
        report.covered += 1;
        info!(family_id, "family covered again");
        events::log_fanout_error(
            "family_plan",
            events::deliver_family_plan(state, family_id, &plan, None).await,
        );
    }

    // No longer covered. The clock starts NOW, when the server noticed.
    let marked: Vec<(i64, OffsetDateTime)> = sqlx::query_as(&format!(
        "UPDATE families f SET unpaid_since = now(), unpaid_warned = false
          WHERE f.plan = 'paid' AND f.unpaid_since IS NULL AND NOT {active}
          RETURNING f.id, f.unpaid_since"
    ))
    .fetch_all(&state.pool)
    .await?;
    let days = cfg.subscriptions.unpaid_days;
    for (family_id, since) in marked {
        report.unpaid += 1;
        info!(family_id, "family unpaid");
        events::log_fanout_error(
            "family_plan",
            events::deliver_family_plan(state, family_id, PAID, Some(since + Duration::days(days)))
                .await,
        );
        events::log_fanout_error(
            "family_plan push",
            events::push_family_plan(state, family_id, PlanStage::Unpaid).await,
        );
    }

    // The day before. Not for a one-day deadline: its "tomorrow" is the
    // moment it started, and the push that said so went out above.
    if days > 1 {
        let warned: Vec<i64> = sqlx::query_scalar(
            "UPDATE families SET unpaid_warned = true
              WHERE plan = 'paid' AND unpaid_since IS NOT NULL AND NOT unpaid_warned
                AND unpaid_since + make_interval(days => $1) <= now()
              RETURNING id",
        )
        .bind((days - 1) as i32)
        .fetch_all(&state.pool)
        .await?;
        for family_id in warned {
            report.warned += 1;
            events::log_fanout_error(
                "family_plan push",
                events::push_family_plan(state, family_id, PlanStage::Tomorrow).await,
            );
        }
    }

    let due: Vec<i64> = sqlx::query_scalar(
        "SELECT id FROM families
          WHERE plan = 'paid' AND unpaid_since IS NOT NULL
            AND unpaid_since + make_interval(days => $1) <= now()
          ORDER BY id",
    )
    .bind(days as i32)
    .fetch_all(&state.pool)
    .await?;
    for family_id in due {
        match crate::handlers_family::delete_unpaid_family(state, family_id).await {
            Ok(true) => {
                report.deleted += 1;
                info!(family_id, "family deleted (unpaid)");
            }
            Ok(false) => {}
            Err(error) => warn!(?error, family_id, "could not delete an unpaid family"),
        }
    }
    Ok(report)
}

/// Is this family's allowance the paid one? Only a PAID family on a server
/// whose subscriptions are on has one.
pub async fn is_paid(pool: &PgPool, cfg: &Config, family_id: i64) -> Result<bool, ApiError> {
    if !cfg.subscriptions.enabled {
        return Ok(false);
    }
    let plan: Option<String> = sqlx::query_scalar("SELECT plan FROM families WHERE id = $1")
        .bind(family_id)
        .fetch_optional(pool)
        .await?;
    Ok(plan.as_deref() == Some(PAID))
}

/// The bytes of the files the family keeps on the disk, each distinct file
/// once: what its chats' messages carry and what was uploaded for it — its
/// board, its pack, uploads not sent yet. Previews are not measured.
pub async fn storage_used(pool: &PgPool, family_id: i64) -> Result<i64, ApiError> {
    Ok(sqlx::query_scalar(
        "SELECT COALESCE(SUM(size_bytes), 0)::BIGINT FROM (
             SELECT DISTINCT ON (a.storage_key) a.storage_key, a.size_bytes
               FROM attachments a
               LEFT JOIN messages m ON m.id = a.message_id
               LEFT JOIN chats c ON c.id = m.chat_id
              WHERE a.family_id = $1 OR c.family_id = $1
         ) files",
    )
    .bind(family_id)
    .fetch_one(pool)
    .await?)
}

/// This month's assistant use by the family, counted the way Family
/// Statistics counts it.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MonthUse {
    pub tokens: i64,
    pub images: i64,
    pub transcript_ms: i64,
}

pub async fn month_use(pool: &PgPool, family_id: i64) -> Result<MonthUse, ApiError> {
    let row = sqlx::query(
        "SELECT COALESCE(SUM(prompt_tokens + completion_tokens), 0)::BIGINT AS tokens,
                COALESCE(SUM(images), 0)::BIGINT AS images,
                COALESCE(SUM(audio_ms), 0)::BIGINT AS transcript_ms
           FROM ai_usage WHERE family_id = $1 AND created_at >= $2",
    )
    .bind(family_id)
    .bind(month_start(OffsetDateTime::now_utc()))
    .fetch_one(pool)
    .await?;
    Ok(MonthUse {
        tokens: row.get("tokens"),
        images: row.get("images"),
        transcript_ms: row.get("transcript_ms"),
    })
}

/// What a call to the assistant would spend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Spend {
    /// A question to the text model.
    Tokens,
    /// A picture.
    Images,
    /// A transcript.
    Transcript,
}

/// May this family spend `spend` now? Always for a free family, or with the
/// limit off; for a paid one, while this month's use is under the limit.
/// Checked BEFORE the call, so one answer may overshoot by its own size and
/// none is ever cut off halfway.
pub async fn may_spend(state: &AppState, family_id: i64, spend: Spend) -> Result<bool, ApiError> {
    let subs = &state.cfg.subscriptions;
    let limit = match spend {
        Spend::Tokens => subs.ai_token_limit(),
        Spend::Images => subs.image_limit(),
        Spend::Transcript => subs.transcript_ms_limit(),
    };
    let Some(limit) = limit else {
        return Ok(true);
    };
    if !is_paid(&state.pool, &state.cfg, family_id).await? {
        return Ok(true);
    }
    let used = month_use(&state.pool, family_id).await?;
    let used = match spend {
        Spend::Tokens => used.tokens,
        Spend::Images => used.images,
        Spend::Transcript => used.transcript_ms,
    };
    Ok(used < limit)
}

/// May `incoming` more bytes be stored for this family? `None` when the
/// size is not known yet: then only a family already at its limit is
/// refused.
pub async fn may_store(
    state: &AppState,
    family_id: i64,
    incoming: Option<u64>,
) -> Result<bool, ApiError> {
    let Some(limit) = state.cfg.subscriptions.storage_limit() else {
        return Ok(true);
    };
    if !is_paid(&state.pool, &state.cfg, family_id).await? {
        return Ok(true);
    }
    let used = storage_used(&state.pool, family_id).await?;
    let incoming = i64::try_from(incoming.unwrap_or(0)).unwrap_or(i64::MAX);
    Ok(used < limit && used.saturating_add(incoming) <= limit)
}

/// The `allowance` block of Family Statistics, for a paid family only.
pub async fn allowance_json(
    state: &AppState,
    family_id: i64,
) -> Result<Option<serde_json::Value>, ApiError> {
    if !is_paid(&state.pool, &state.cfg, family_id).await? {
        return Ok(None);
    }
    let subs = &state.cfg.subscriptions;
    let now = OffsetDateTime::now_utc();
    let used = month_use(&state.pool, family_id).await?;
    let renews_at = next_month_start(now)
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|error| ApiError::Internal(error.into()))?;
    let mut block = serde_json::json!({"renews_at": renews_at});
    if let Some(limit) = subs.storage_limit() {
        block["storage_bytes"] = serde_json::json!({
            "used": storage_used(&state.pool, family_id).await?, "limit": limit,
        });
    }
    for (key, used, limit) in [
        ("ai_tokens", used.tokens, subs.ai_token_limit()),
        ("images", used.images, subs.image_limit()),
        (
            "transcript_ms",
            used.transcript_ms,
            subs.transcript_ms_limit(),
        ),
    ] {
        if let Some(limit) = limit {
            block[key] = serde_json::json!({"used": used, "limit": limit});
        }
    }
    Ok(Some(block))
}

/// `paid_family_limits` on `GET /me`: what a paid family gets, each key only
/// when that limit is on.
pub fn paid_family_limits(cfg: &Config) -> serde_json::Value {
    let subs = &cfg.subscriptions;
    let mut limits = serde_json::Map::new();
    for (key, value) in [
        ("members", subs.member_limit()),
        ("storage_bytes", subs.storage_limit()),
        ("ai_tokens", subs.ai_token_limit()),
        ("images", subs.image_limit()),
        ("transcript_ms", subs.transcript_ms_limit()),
    ] {
        if let Some(value) = value {
            limits.insert(key.to_string(), value.into());
        }
    }
    serde_json::Value::Object(limits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn enabled() -> Config {
        let mut cfg = Config::default();
        cfg.subscriptions.enabled = true;
        cfg
    }

    #[test]
    fn months_start_on_the_first_in_utc() {
        assert_eq!(
            month_start(datetime!(2026-10-30 23:59:59 UTC)),
            datetime!(2026-10-01 00:00:00 UTC)
        );
        // An hour after midnight in Moscow is still the previous UTC day —
        // and, on the 1st, the previous UTC MONTH.
        assert_eq!(
            month_start(datetime!(2026-11-01 01:00:00 +3)),
            datetime!(2026-10-01 00:00:00 UTC)
        );
        assert_eq!(
            next_month_start(datetime!(2026-10-30 12:00:00 UTC)),
            datetime!(2026-11-01 00:00:00 UTC)
        );
        assert_eq!(
            next_month_start(datetime!(2026-12-31 23:00:00 UTC)),
            datetime!(2027-01-01 00:00:00 UTC)
        );
    }

    #[test]
    fn the_ceiling_is_the_lower_of_the_two_only_for_a_paid_family_with_subscriptions_on() {
        let off = Config::default();
        assert_eq!(member_ceiling(&off, PAID), 50);
        let on = enabled();
        assert_eq!(member_ceiling(&on, PAID), 25);
        assert_eq!(member_ceiling(&on, FREE), 50);
        let mut low_operator = enabled();
        low_operator.limits.max_family_members = 10;
        assert_eq!(member_ceiling(&low_operator, PAID), 10);
        let mut unlimited = enabled();
        unlimited.subscriptions.max_members = 0;
        assert_eq!(member_ceiling(&unlimited, PAID), 50);
    }

    #[test]
    fn the_wire_shows_a_plan_and_a_deadline_only_while_subscriptions_are_on() {
        let since = datetime!(2026-10-30 09:00:00 UTC);
        let off = plan_view(&Config::default(), PAID, Some(since));
        assert_eq!(off.plan, None);
        assert_eq!(off.delete_at, None);
        assert_eq!(off.member_ceiling, 50);

        let on = enabled();
        let unpaid = plan_view(&on, PAID, Some(since));
        assert_eq!(unpaid.plan.as_deref(), Some(PAID));
        assert_eq!(unpaid.delete_at, Some(datetime!(2026-11-13 09:00:00 UTC)));
        assert_eq!(unpaid.member_ceiling, 25);

        let covered = plan_view(&on, PAID, None);
        assert_eq!(covered.delete_at, None);
        // A free family never shows a deadline, even one left in the row.
        let free = plan_view(&on, FREE, Some(since));
        assert_eq!(free.plan.as_deref(), Some(FREE));
        assert_eq!(free.delete_at, None);
    }

    #[test]
    fn a_new_family_is_paid_only_while_subscriptions_are_on() {
        assert_eq!(plan_for_new_family(&Config::default()), FREE);
        assert_eq!(plan_for_new_family(&enabled()), PAID);
    }

    #[test]
    fn paid_family_limits_list_only_the_limits_that_are_on() {
        let mut cfg = enabled();
        cfg.subscriptions.images_per_month = 0;
        let limits = paid_family_limits(&cfg);
        assert_eq!(limits["members"], 25);
        assert_eq!(limits["storage_bytes"], 10_737_418_240_i64);
        assert_eq!(limits["ai_tokens"], 2_000_000);
        assert_eq!(limits["transcript_ms"], 7_200_000);
        assert!(limits.get("images").is_none());
    }

    #[test]
    fn the_environments_are_spliced_as_the_validated_literals() {
        assert_eq!(environments_sql(&enabled()), "'Production', 'Sandbox'");
    }
}
