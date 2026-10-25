//! The one-off conversion of a database written before #59: every owner is
//! given a key, and every value migration 0059 left behind a 0x00 byte is
//! sealed under its owner's key.
//!
//! It runs between 0059 and 0060, under the migration lock, before the server
//! answers anything (`migrate::run`). Each batch is its own transaction of at
//! most [`BATCH`] rows, and what a batch did is visible in the data itself — a
//! key on the owner row, a 0x01 where a 0x00 was — so a crash or a restart
//! simply continues: the next run skips what is done. 0060 is applied only once
//! this has finished, and from then on PostgreSQL refuses anything unsealed.
//!
//! THE SQL HERE IS FROZEN AT SCHEMA 59. It runs at exactly one point in the
//! migration history, so it must read the tables as 0059 leaves them and
//! nothing later — it shares no query with the rest of the server on purpose.
//!
//! The plaintext behind each 0x00 is already in the form the server seals it
//! in: 0059 wrote text as UTF-8, coordinates as eight big-endian bytes
//! (`float8send`), a birthday as two bytes and greeting places as a JSON array.
//! This step only seals what follows the 0x00. It logs counts, never content.

use anyhow::{Context, Result};
use sqlx::{Acquire, PgConnection, Row};
use tracing::info;

use crate::keyring::Keyring;
use crate::vault::{Lookup, NOT_YET_SEALED};

/// Rows per transaction.
pub const BATCH: i64 = 500;

/// What the conversion found and did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Outcome {
    /// Owners given a key in this run.
    pub keys: u64,
    /// Rows sealed in this run.
    pub rows: u64,
    /// Whether the database holds anything a person wrote — when it does, the
    /// converted tables are compacted afterwards so their old plaintext rows
    /// leave the table files. A fresh database (only the assistant's account)
    /// is not.
    pub had_data: bool,
    /// Whether everything is sealed. Only [`run_limited`] stops early.
    pub finished: bool,
}

/// How many more batches a run may commit: `None` is no limit. The upgrade
/// test uses a limit to stop a conversion half-way and prove the next run
/// continues where it stopped.
struct Budget(Option<u64>);

impl Budget {
    /// Whether another batch may run.
    fn allows(&self) -> bool {
        self.0 != Some(0)
    }

    /// One batch committed.
    fn spend(&mut self) {
        if let Some(left) = &mut self.0 {
            *left = left.saturating_sub(1);
        }
    }
}

/// A table with sealed columns, as 0059 leaves it.
struct Table {
    name: &'static str,
    /// Its primary key: one or two BIGINT columns, walked in order.
    key: &'static [&'static str],
    /// The owner's `key_id`, as an expression over the row `t`.
    owner_key: &'static str,
    /// Its sealed columns.
    columns: &'static [&'static str],
    /// Lookup columns filled from a sealed column's plaintext:
    /// (sealed column, lookup column, what it is).
    lookups: &'static [(&'static str, &'static str, Lookup)],
}

/// The key of whatever holds an attachment, as an expression over its row.
const HOLDER_KEY: &str = "COALESCE(
    (SELECT c.key_id FROM messages m JOIN chats c ON c.id = m.chat_id WHERE m.id = t.message_id),
    (SELECT f.key_id FROM notes n JOIN families f ON f.id = n.family_id WHERE n.id = t.note_id),
    (SELECT f.key_id FROM pack_items p JOIN families f ON f.id = p.family_id
      WHERE p.id = t.pack_item_id),
    (SELECT key_id FROM families WHERE id = t.family_id),
    (SELECT key_id FROM users WHERE id = t.uploader_id))";

/// Every table with sealed columns. A table added here must also be converted
/// to `bytea` in 0059 and checked in 0060.
const TABLES: &[Table] = &[
    Table {
        name: "families",
        key: &["id"],
        owner_key: "t.key_id",
        columns: &["name", "invite_code", "greeting_places"],
        lookups: &[("invite_code", "invite_code_key", Lookup::InviteCode)],
    },
    Table {
        name: "join_requests",
        key: &["id"],
        owner_key: "(SELECT key_id FROM families WHERE id = t.family_id)",
        columns: &["invite_code_used"],
        lookups: &[],
    },
    Table {
        name: "chats",
        key: &["id"],
        owner_key: "t.key_id",
        columns: &["title"],
        lookups: &[],
    },
    Table {
        name: "messages",
        key: &["id"],
        owner_key: "(SELECT key_id FROM chats WHERE id = t.chat_id)",
        columns: &["body"],
        lookups: &[],
    },
    Table {
        name: "message_reactions",
        key: &["message_id", "user_id"],
        owner_key: "(SELECT c.key_id FROM messages m JOIN chats c ON c.id = m.chat_id
                  WHERE m.id = t.message_id)",
        columns: &["emoji"],
        lookups: &[],
    },
    Table {
        name: "message_mentions",
        key: &["message_id", "user_id"],
        owner_key: "(SELECT c.key_id FROM messages m JOIN chats c ON c.id = m.chat_id
                  WHERE m.id = t.message_id)",
        columns: &["name"],
        lookups: &[],
    },
    Table {
        name: "poll_options",
        key: &["id"],
        owner_key: "(SELECT c.key_id FROM polls p JOIN chats c ON c.id = p.chat_id
                  WHERE p.message_id = t.poll_id)",
        columns: &["text"],
        lookups: &[],
    },
    Table {
        // The recording's message's chat; an attachment no message claimed (a
        // transcript cannot be asked of one, but the schema does not say so)
        // falls back to the family it was uploaded for.
        name: "transcripts",
        key: &["attachment_id"],
        owner_key: "(SELECT COALESCE(c.key_id, f.key_id)
                   FROM attachments a
                   LEFT JOIN messages m ON m.id = a.message_id
                   LEFT JOIN chats c ON c.id = m.chat_id
                   LEFT JOIN families f ON f.id = a.family_id
                  WHERE a.id = t.attachment_id)",
        columns: &["text"],
        lookups: &[],
    },
    Table {
        name: "notes",
        key: &["id"],
        owner_key: "(SELECT key_id FROM families WHERE id = t.family_id)",
        columns: &["text", "place"],
        lookups: &[],
    },
    Table {
        name: "note_task_items",
        key: &["id"],
        owner_key: "(SELECT f.key_id FROM notes n JOIN families f ON f.id = n.family_id
                  WHERE n.id = t.note_id)",
        columns: &["text"],
        lookups: &[],
    },
    Table {
        name: "note_mentions",
        key: &["note_id", "user_id"],
        owner_key: "(SELECT f.key_id FROM notes n JOIN families f ON f.id = n.family_id
                  WHERE n.id = t.note_id)",
        columns: &["name"],
        lookups: &[],
    },
    Table {
        name: "pack_items",
        key: &["id"],
        owner_key: "(SELECT key_id FROM families WHERE id = t.family_id)",
        columns: &["label"],
        lookups: &[],
    },
    Table {
        name: "member_reports",
        key: &["id"],
        owner_key: "(SELECT key_id FROM families WHERE id = t.family_id)",
        columns: &["message_excerpt"],
        lookups: &[],
    },
    Table {
        // The reported reply's conversation; one whose message retention has
        // already swept (`message_id` went NULL) cannot be traced to it, and is
        // sealed under the reporter's own key instead.
        name: "assistant_reports",
        key: &["id"],
        owner_key: "COALESCE(
                  (SELECT c.key_id FROM messages m JOIN chats c ON c.id = m.chat_id
                    WHERE m.id = t.message_id),
                  (SELECT key_id FROM users WHERE id = t.reporter_user_id))",
        columns: &["message_excerpt", "note"],
        lookups: &[],
    },
    Table {
        // Whatever holds it: a message's conversation, a board note's or a
        // sticker's family, the family an unclaimed upload was made for — and,
        // for an unclaimed upload whose family has since been deleted, the
        // uploader's own key.
        name: "attachments",
        key: &["id"],
        owner_key: HOLDER_KEY,
        columns: &["name", "latitude", "longitude", "waveform"],
        lookups: &[],
    },
    Table {
        name: "users",
        key: &["id"],
        owner_key: "t.key_id",
        columns: &["username", "display_name", "password_hash", "birthday"],
        lookups: &[("username", "username_key", Lookup::Username)],
    },
    Table {
        name: "user_avatars",
        key: &["user_id"],
        owner_key: "(SELECT key_id FROM users WHERE id = t.user_id)",
        columns: &["bytes"],
        lookups: &[],
    },
    Table {
        name: "devices",
        key: &["id"],
        owner_key: "(SELECT key_id FROM users WHERE id = t.user_id)",
        columns: &["push_token", "voip_token"],
        lookups: &[
            ("push_token", "push_token_key", Lookup::PushToken),
            ("voip_token", "voip_token_key", Lookup::VoipToken),
        ],
    },
];

/// Give every owner a key, then seal every table.
pub async fn run(conn: &mut PgConnection, keys: &Keyring) -> Result<Outcome> {
    run_limited(conn, keys, None).await
}

/// [`run`], stopping after `max_batches` committed batches when given one —
/// a conversion interrupted at a chosen point, for the upgrade test.
pub async fn run_limited(
    conn: &mut PgConnection,
    keys: &Keyring,
    max_batches: Option<u64>,
) -> Result<Outcome> {
    let mut budget = Budget(max_batches);
    // Refuse a different key file BEFORE sealing anything: an interrupted
    // conversion restarted with another key would otherwise seal half the
    // database under each.
    keys.load(&mut *conn).await?;

    let mut outcome = Outcome {
        // A fresh database holds one account, the assistant's (0015).
        had_data: sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM families) OR (SELECT count(*) FROM users) > 1",
        )
        .fetch_one(&mut *conn)
        .await
        .context("looking for data to convert")?,
        ..Outcome::default()
    };

    outcome.keys += give_keys(conn, keys, "families", &mut budget).await?;
    outcome.keys += give_keys(conn, keys, "users", &mut budget).await?;
    // A family chat seals with its family's key; every other chat has its own.
    let shared = sqlx::query(
        "UPDATE chats c SET key_id = f.key_id
           FROM families f
          WHERE c.kind = 'family' AND c.family_id = f.id AND c.key_id IS NULL",
    )
    .execute(&mut *conn)
    .await
    .context("giving family chats their family's key")?
    .rows_affected();
    outcome.keys += shared;
    outcome.keys += give_keys(conn, keys, "chats", &mut budget).await?;

    for table in TABLES {
        outcome.rows += seal_table(conn, keys, table, &mut budget).await?;
    }
    outcome.rows += key_content_hashes(conn, keys, &mut budget).await?;
    outcome.finished = budget.allows();

    if outcome.keys + outcome.rows > 0 {
        info!(
            keys = outcome.keys,
            rows = outcome.rows,
            "sealed the database (#59)"
        );
    }
    Ok(outcome)
}

/// A key for every row of `table` that has none. Family chats were given
/// their family's before this runs for `chats`.
async fn give_keys(
    conn: &mut PgConnection,
    keys: &Keyring,
    table: &str,
    budget: &mut Budget,
) -> Result<u64> {
    let mut given = 0;
    while budget.allows() {
        let mut tx = conn.begin().await?;
        let ids: Vec<i64> = sqlx::query_scalar(&format!(
            "SELECT id FROM {table} WHERE key_id IS NULL ORDER BY id LIMIT $1 FOR UPDATE"
        ))
        .bind(BATCH)
        .fetch_all(&mut *tx)
        .await
        .with_context(|| format!("finding {table} without a key"))?;
        if ids.is_empty() {
            return Ok(given);
        }
        for id in &ids {
            let key = keys.create(&mut tx).await?;
            sqlx::query(&format!("UPDATE {table} SET key_id = $2 WHERE id = $1"))
                .bind(id)
                .bind(key.id())
                .execute(&mut *tx)
                .await
                .with_context(|| format!("giving {table} {id} a key"))?;
        }
        tx.commit().await?;
        budget.spend();
        given += ids.len() as u64;
    }
    Ok(given)
}

/// Seal every 0x00 value of one table, walking its primary key.
async fn seal_table(
    conn: &mut PgConnection,
    keys: &Keyring,
    table: &Table,
    budget: &mut Budget,
) -> Result<u64> {
    let list = |prefix: &str, columns: &mut dyn Iterator<Item = &str>| {
        columns
            .map(|column| format!("{prefix}{column}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let key_list = list("t.", &mut table.key.iter().copied());
    let after = (1..=table.key.len())
        .map(|n| format!("${n}"))
        .collect::<Vec<_>>()
        .join(", ");
    let unsealed = table
        .columns
        .iter()
        .map(|column| format!("get_byte(t.{column}, 0) = 0"))
        .collect::<Vec<_>>()
        .join(" OR ");
    let select = format!(
        "SELECT {key_list}, {owner} AS owner_key, {columns}
           FROM {name} t
          WHERE ({key_list}) > ({after}) AND ({unsealed})
          ORDER BY {key_list}
          LIMIT {BATCH}
          FOR UPDATE OF t",
        owner = table.owner_key,
        columns = list("t.", &mut table.columns.iter().copied()),
        name = table.name,
    );

    // The UPDATE takes the whole batch at once, one array per column. A
    // lookup is written only where its column was sealed in this batch.
    let lookup_names: Vec<&str> = table.lookups.iter().map(|(_, lookup, _)| *lookup).collect();
    let mut n = 0;
    let mut param = |kind: &str| {
        n += 1;
        format!("${n}::{kind}[]")
    };
    let mut params = Vec::new();
    for _ in table.key {
        params.push(param("bigint"));
    }
    for _ in table.columns.iter().chain(&lookup_names) {
        params.push(param("bytea"));
    }
    let sets = table
        .columns
        .iter()
        .map(|column| format!("{column} = v.{column}"))
        .chain(
            lookup_names
                .iter()
                .map(|lookup| format!("{lookup} = COALESCE(v.{lookup}, t.{lookup})")),
        )
        .collect::<Vec<_>>()
        .join(", ");
    let matches = table
        .key
        .iter()
        .map(|column| format!("t.{column} = v.{column}"))
        .collect::<Vec<_>>()
        .join(" AND ");
    let update = format!(
        "UPDATE {name} t SET {sets}
           FROM UNNEST({params}) AS v({names})
          WHERE {matches}",
        name = table.name,
        params = params.join(", "),
        names = list(
            "",
            &mut table
                .key
                .iter()
                .chain(table.columns)
                .copied()
                .chain(lookup_names.iter().copied())
        ),
    );

    let mut sealed_rows = 0;
    let mut cursor: Vec<i64> = vec![i64::MIN; table.key.len()];
    while budget.allows() {
        let mut tx = conn.begin().await?;
        let mut query = sqlx::query(&select);
        for value in &cursor {
            query = query.bind(*value);
        }
        let rows = query
            .fetch_all(&mut *tx)
            .await
            .with_context(|| format!("reading {} to seal", table.name))?;
        if rows.is_empty() {
            tx.commit().await?;
            return Ok(sealed_rows);
        }

        let mut key_values: Vec<Vec<i64>> = vec![Vec::with_capacity(rows.len()); table.key.len()];
        let mut sealed_values: Vec<Vec<Option<Vec<u8>>>> =
            vec![Vec::with_capacity(rows.len()); table.columns.len()];
        let mut lookup_values: Vec<Vec<Option<Vec<u8>>>> =
            vec![Vec::with_capacity(rows.len()); table.lookups.len()];
        for row in &rows {
            for (values, column) in key_values.iter_mut().zip(table.key) {
                values.push(row.get(*column));
            }
            let owner_key: Option<i64> = row.get("owner_key");
            let key = owner_key
                .with_context(|| format!("a row of {} whose owner has no key", table.name))
                .and_then(|key_id| {
                    keys.by_id(key_id)
                        .with_context(|| format!("the key of a row of {}", table.name))
                })?;
            for (values, column) in sealed_values.iter_mut().zip(table.columns) {
                let value: Option<Vec<u8>> = row.get(*column);
                let plain = match value.as_deref().and_then(<[u8]>::split_first) {
                    Some((&NOT_YET_SEALED, plain)) => Some(plain),
                    _ => None,
                };
                for (lookups, (from, _, what)) in lookup_values.iter_mut().zip(table.lookups) {
                    if from == column {
                        lookups.push(
                            plain.map(|plain| keys.lookup(*what, &String::from_utf8_lossy(plain))),
                        );
                    }
                }
                values.push(match plain {
                    Some(plain) => Some(key.seal(plain)),
                    None => value,
                });
            }
        }

        let mut query = sqlx::query(&update);
        for values in &key_values {
            query = query.bind(values);
        }
        for values in sealed_values.iter().chain(&lookup_values) {
            query = query.bind(values);
        }
        query
            .execute(&mut *tx)
            .await
            .with_context(|| format!("sealing {}", table.name))?;
        tx.commit().await?;
        budget.spend();

        sealed_rows += rows.len() as u64;
        cursor = key_values
            .iter()
            .map(|values| *values.last().expect("the batch is not empty"))
            .collect();
    }
    Ok(sealed_rows)
}

/// Key every attachment's dedup digest with its family's key: the HMAC of
/// the plaintext SHA-256 that 0059 put aside, as uploads write it now. A row
/// whose family is gone keeps none — it can never be matched again anyway,
/// since dedup is per family.
async fn key_content_hashes(
    conn: &mut PgConnection,
    keys: &Keyring,
    budget: &mut Budget,
) -> Result<u64> {
    let mut keyed = 0;
    let mut after = 0_i64;
    while budget.allows() {
        let mut tx = conn.begin().await?;
        let rows: Vec<(i64, String, i64)> = sqlx::query_as(
            "SELECT a.id, a.content_sha256, f.key_id
               FROM attachments a JOIN families f ON f.id = a.family_id
              WHERE a.id > $1 AND a.content_sha256 IS NOT NULL AND a.content_hash IS NULL
              ORDER BY a.id
              LIMIT $2
              FOR UPDATE OF a",
        )
        .bind(after)
        .bind(BATCH)
        .fetch_all(&mut *tx)
        .await
        .context("reading attachment digests to key")?;
        let Some(&(last, _, _)) = rows.last() else {
            tx.commit().await?;
            return Ok(keyed);
        };
        let mut ids = Vec::with_capacity(rows.len());
        let mut hashes = Vec::with_capacity(rows.len());
        for (id, sha256, key_id) in &rows {
            let digest = hex::decode(sha256)
                .with_context(|| format!("attachment {id}'s digest is not hex"))?;
            ids.push(*id);
            hashes.push(keys.by_id(*key_id)?.content_hash(&digest));
        }
        sqlx::query(
            "UPDATE attachments a SET content_hash = v.hash
               FROM UNNEST($1::bigint[], $2::text[]) AS v(id, hash)
              WHERE a.id = v.id",
        )
        .bind(&ids)
        .bind(&hashes)
        .execute(&mut *tx)
        .await
        .context("keying attachment digests")?;
        tx.commit().await?;
        budget.spend();
        keyed += rows.len() as u64;
        after = last;
    }
    Ok(keyed)
}
