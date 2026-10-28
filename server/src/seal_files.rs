//! Sealing the attachment files written before #59, in the background, once
//! the server is serving (docs/encryption-at-rest-2026-10-24.md, "Upgrading a
//! live server", step 5).
//!
//! The database is sealed before the first request (`seal_existing`); the
//! files can wait, because a file whose row has no `file_key` is served as
//! the plaintext it still is. Each file is sealed into a COPY under a new name
//! — the old name with an `s` in front, so it stays in the same shard
//! directory — and only then do its rows move to the copy, in one transaction
//! that also notes the old name in `storage_cleanup`. The old file is removed
//! after the commit, and the note with it. So:
//!
//! - a crash before the commit leaves a sealed copy nothing names, which the
//!   next pass overwrites, since the name is the same;
//! - a crash after it leaves the old name in `storage_cleanup`, which the next
//!   pass removes first — no plaintext file outlives its conversion.
//!
//! The rows are locked while they move, and so are their previews: a preview
//! upload takes the same row lock (`handlers_attachment::upload_preview`), so
//! no preview can land beside a file that has just moved away from it.
//!
//! The log counts files; a file that will not seal is named by its row id and
//! its storage key's path, never by anything a person wrote.

use std::collections::HashSet;

use anyhow::{Context, Result};
use sqlx::Row;
use tracing::{info, warn};

use crate::keyring::KeyError;
use crate::state::AppState;
use crate::vault::DataKey;

/// The key of whatever holds an attachment, as an expression over its row
/// `a`: a message's conversation, a board note's or a sticker's family, the
/// family an unclaimed upload was made for — or, once that family is gone,
/// the uploader's own.
pub const HOLDER_KEY_SQL: &str = "COALESCE(
    (SELECT c.key_id FROM messages m JOIN chats c ON c.id = m.chat_id WHERE m.id = a.message_id),
    (SELECT f.key_id FROM notes n JOIN families f ON f.id = n.family_id WHERE n.id = a.note_id),
    (SELECT f.key_id FROM pack_items p JOIN families f ON f.id = p.family_id
      WHERE p.id = a.pack_item_id),
    (SELECT key_id FROM families WHERE id = a.family_id),
    (SELECT key_id FROM users WHERE id = a.uploader_id))";

/// Start the conversion. Returns at once; the work runs on its own task and
/// ends when nothing is left to seal.
pub fn spawn(state: AppState) {
    tokio::spawn(async move {
        match run(&state).await {
            Ok(0) => {}
            Ok(sealed) => info!(files = sealed, "sealed the stored files (#59)"),
            Err(error) => warn!(error = %format!("{error:#}"), "sealing the stored files stopped"),
        }
    });
}

/// Remove what an earlier pass left in `storage_cleanup`, then seal every
/// file that is still plaintext, oldest first. Answers how many were sealed.
pub async fn run(state: &AppState) -> Result<u64> {
    finish_cleanup(state).await?;
    let mut sealed = 0_u64;
    let mut failed: HashSet<String> = HashSet::new();
    let mut after = 0_i64;
    loop {
        let next: Option<(i64, String)> = sqlx::query_as(
            "SELECT id, storage_key FROM attachments
              WHERE id > $1 AND file_key IS NULL AND kind <> 'location'
              ORDER BY id
              LIMIT 1",
        )
        .bind(after)
        .fetch_optional(&state.pool)
        .await
        .context("finding a file to seal")?;
        let Some((id, storage_key)) = next else {
            break;
        };
        after = id;
        if failed.contains(&storage_key) {
            continue;
        }
        match seal_one(state, &storage_key).await {
            Ok(true) => {
                sealed += 1;
                if sealed.is_multiple_of(100) {
                    info!(files = sealed, "sealing the stored files (#59)");
                }
            }
            Ok(false) => {}
            // One file that will not seal — missing from the disk, say —
            // must not keep every other one plaintext. It is tried again on
            // the next start.
            Err(error) => {
                warn!(attachment_id = id, error = %format!("{error:#}"), "a stored file would not seal");
                failed.insert(storage_key);
            }
        }
    }
    if !failed.is_empty() {
        warn!(
            files = failed.len(),
            "stored files left unsealed until the next start"
        );
    }
    Ok(sealed)
}

/// Seal the one plaintext file named `storage_key`, and move every row that
/// names it to the sealed copy. `false` when there was nothing to do — the
/// rows went, or another pass got there first.
///
/// The rows are locked FIRST, before a byte is sealed: two passes over the
/// same file (the background one and a preview upload that cannot wait for
/// it) would otherwise write the same copy at once, and the loser, finding
/// the rows already moved, would remove the winner's file. Locked, the second
/// waits, finds `file_key` set, and touches nothing.
pub async fn seal_one(state: &AppState, storage_key: &str) -> Result<bool> {
    let mut tx = state.pool.begin().await?;
    let rows = sqlx::query(&format!(
        "SELECT a.id, {HOLDER_KEY_SQL} AS holder_key, a.has_preview
           FROM attachments a
          WHERE a.storage_key = $1 AND a.file_key IS NULL
          ORDER BY a.id
          FOR UPDATE OF a"
    ))
    .bind(storage_key)
    .fetch_all(&mut *tx)
    .await
    .context("locking a stored file's rows")?;
    if rows.is_empty() {
        tx.rollback().await?;
        return Ok(false);
    }

    let sealed_key = format!("s{storage_key}");
    let own_key = DataKey::generate();
    state
        .storage
        .seal_file(
            &state.storage.blob_path(storage_key),
            &state.storage.blob_path(&sealed_key),
            &own_key,
        )
        .await
        .context("sealing a stored file")?;
    // The preview, under the same lock: whatever a preview upload wrote
    // before the lock was taken is here now, and none can be written until
    // the rows have moved.
    let preview = state.storage.preview_path(storage_key);
    if rows.iter().any(|row| row.get::<bool, _>("has_preview"))
        && tokio::fs::try_exists(&preview).await.unwrap_or(false)
    {
        state
            .storage
            .seal_file(&preview, &state.storage.preview_path(&sealed_key), &own_key)
            .await
            .context("sealing a stored preview")?;
    }
    for row in &rows {
        let id: i64 = row.get("id");
        let holder: Option<i64> = row.get("holder_key");
        let holder = holder
            .with_context(|| format!("attachment {id} has nothing whose key could hold it"))
            .and_then(|key_id| {
                state
                    .keys
                    .by_id(key_id)
                    .map_err(|err: KeyError| anyhow::Error::new(err))
            })?;
        sqlx::query("UPDATE attachments SET storage_key = $2, file_key = $3 WHERE id = $1")
            .bind(id)
            .bind(&sealed_key)
            .bind(crate::handlers_attachment::seal_file_key(&holder, &own_key))
            .execute(&mut *tx)
            .await
            .context("moving a row to its sealed file")?;
    }
    sqlx::query("INSERT INTO storage_cleanup (storage_key) VALUES ($1) ON CONFLICT DO NOTHING")
        .bind(storage_key)
        .execute(&mut *tx)
        .await
        .context("noting a plaintext file to remove")?;
    tx.commit().await?;

    remove_noted(state, storage_key).await?;
    Ok(true)
}

/// Remove the files an earlier pass moved away from and did not get to.
async fn finish_cleanup(state: &AppState) -> Result<()> {
    let noted: Vec<String> = sqlx::query_scalar("SELECT storage_key FROM storage_cleanup")
        .fetch_all(&state.pool)
        .await
        .context("reading storage_cleanup")?;
    for storage_key in &noted {
        remove_noted(state, storage_key).await?;
    }
    if !noted.is_empty() {
        info!(
            files = noted.len(),
            "removed plaintext files an earlier pass had moved away from"
        );
    }
    Ok(())
}

/// Remove one noted file and its preview — unless a row still names it,
/// which no row can once the move has committed — and forget the note.
async fn remove_noted(state: &AppState, storage_key: &str) -> Result<()> {
    let still_named: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM attachments WHERE storage_key = $1)")
            .bind(storage_key)
            .fetch_one(&state.pool)
            .await?;
    if !still_named {
        state.storage.remove(storage_key).await;
    }
    sqlx::query("DELETE FROM storage_cleanup WHERE storage_key = $1")
        .bind(storage_key)
        .execute(&state.pool)
        .await?;
    Ok(())
}
