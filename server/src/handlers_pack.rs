//! The family's sticker pack (docs/protocol.md, "Sticker pack").
//!
//! THE WORD. Everywhere else in this codebase "sticker" means a board NOTE;
//! here it means the chat kind, a small picture sent as its own message.
//! That is why nothing in this file is named `sticker`: the collection is
//! the `pack`, its rows are `pack_items`, and the only `sticker` on the
//! wire is the flag `handlers_chat` stamps on a sent one.
//!
//! The pack is family PROPERTY, like the board, and it borrows the board's
//! machinery whole: every add and every removal takes the next
//! `family_pack_seq` and stamps it on the item, the family's
//! `last_pack_seq` follows GREATEST-guarded, a removal leaves a TOMBSTONE,
//! and the family's row is locked from before the seq is drawn until the
//! commit so changes become visible in seq order. No client learns a second
//! sync idea.
//!
//! Two rules the board does not have, each for a reason:
//!
//! * **Anybody may add; whoever added it OR THE OWNER may remove.** A note
//!   is its author's alone, but a pack is the family's, and a member who
//!   has left — or deleted their account — leaves their stickers behind.
//!   Under an author-only rule nobody could ever remove those.
//! * **Adding what the pack already holds is not an error and not a second
//!   item.** The claim answers with the item that is there. It is what lets
//!   a client offer "Add to family stickers" on a sticker in a chat without
//!   first proving the pack lacks it, and what stops two members who both
//!   liked one from filling the panel with copies.
//!
//! A sent sticker is NOT here. It is an ordinary message carrying its own
//! copy of the picture (`handlers_chat::create_message`), so removing an
//! item breaks nothing that was ever sent and retention sweeping a message
//! takes nothing from the pack.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;
use sqlx::Row;
use std::collections::HashMap;

use crate::auth::AuthUser;
use crate::error::{ApiError, AppJson, codes};
use crate::events;
use crate::handlers_chat::{clamp_limit, parse_pagination_param};
use crate::models::{Attachment, PackItem};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct AddPackItemRequest {
    /// A `kind=photo` upload of the caller's, WebP or PNG, that nothing
    /// else has claimed.
    pub attachment_id: i64,
    /// A few words for a screen reader. Optional, and an empty one is none.
    #[serde(default)]
    pub label: Option<String>,
}

const PACK_COLS: &str = "id, added_by, label, pack_seq, created_at, deleted_at";

/// What a pack item's picture carries on the wire. No coordinates — it is
/// never a location — and no `sticker` column: that flag is a message's,
/// and `Attachment::from_row` reads an unselected one as false.
const PICTURE_COLS: &str =
    "id, kind, mime, size_bytes, width, height, duration_ms, has_preview, name";

/// The family the caller belongs to, or `not_in_family`.
async fn caller_family(state: &AppState, user_id: i64) -> Result<i64, ApiError> {
    let family_id: Option<i64> = sqlx::query_scalar("SELECT family_id FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(&state.pool)
        .await?
        .flatten();
    family_id.ok_or_else(|| ApiError::conflict(codes::NOT_IN_FAMILY, "you are not in a family"))
}

/// Trimmed; empty is no label at all; over the cap is `validation`.
fn validate_label(label: Option<&str>) -> Result<Option<String>, ApiError> {
    let Some(label) = label.map(str::trim).filter(|label| !label.is_empty()) else {
        return Ok(None);
    };
    if label.chars().count() > PackItem::MAX_LABEL_CHARS {
        return Err(ApiError::validation(format!(
            "a sticker's label is at most {} characters",
            PackItem::MAX_LABEL_CHARS
        )));
    }
    Ok(Some(label.to_string()))
}

/// Take the family's row lock for a pack change, FIRST in its transaction —
/// before the item's own row and before a seq is drawn.
///
/// The board's `lock_board`, and for the board's reasons (protocol.md,
/// "Board"): `pack_seq` comes from a sequence, which hands values out
/// before commit, so without this a removal drawing 14 could commit after
/// an add drawing 15 and a full read in between would set a client's cursor
/// past a tombstone no catch-up would ever fetch. It also makes the ceiling
/// exact — two adds at the ceiling cannot both see room — and the duplicate
/// check race-free: two members adding the same picture at once get one
/// item. The SAME row the board locks, so a family's board and pack changes
/// queue behind one another, which costs nothing at a family's pace and
/// adds no lock order to get wrong.
async fn lock_pack(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    family_id: i64,
) -> Result<(), ApiError> {
    sqlx::query("SELECT id FROM families WHERE id = $1 FOR UPDATE")
        .bind(family_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// GREATEST, not plain SET: the family's cursor must never move backwards.
async fn advance_family_seq(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    family_id: i64,
    seq: i64,
) -> Result<(), ApiError> {
    sqlx::query("UPDATE families SET last_pack_seq = GREATEST(last_pack_seq, $2) WHERE id = $1")
        .bind(family_id)
        .bind(seq)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Open the transaction a pack READ runs in: one snapshot for the items and
/// their pictures, which are two statements.
///
/// A removal deletes the picture's row in the commit that tombstones the
/// item. Read a statement apart under the default READ COMMITTED, an item
/// the first statement saw live had no row for the second to find, and went
/// out live with no `attachment` — a shape `PackItem` does not have, and one
/// a client that decodes the field as required fails the whole read on.
/// REPEATABLE READ makes both statements see the same instant: the item is
/// live WITH its picture or a tombstone without one, never between. Read
/// only, so it takes no lock, waits for nobody and cannot fail to serialise.
async fn begin_pack_read(
    state: &AppState,
) -> Result<sqlx::Transaction<'static, sqlx::Postgres>, ApiError> {
    let mut tx = state.pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *tx)
        .await?;
    Ok(tx)
}

/// The pictures for a page of items, in ONE query — a pack is up to
/// `max_pack_items` rows and every live one has a picture, so a read per
/// item would be two hundred of them to open a panel. In the transaction
/// that read the items (`begin_pack_read`), and never on the pool.
async fn attach_pictures(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    items: &mut [PackItem],
) -> Result<(), ApiError> {
    let ids: Vec<i64> = items
        .iter()
        .filter(|item| !item.deleted)
        .map(|item| item.id)
        .collect();
    if ids.is_empty() {
        return Ok(());
    }
    let rows = sqlx::query(&format!(
        "SELECT pack_item_id, {PICTURE_COLS} FROM attachments WHERE pack_item_id = ANY($1)"
    ))
    .bind(&ids)
    .fetch_all(&mut **tx)
    .await?;
    let mut by_item: HashMap<i64, Attachment> = rows
        .iter()
        .map(|row| (row.get("pack_item_id"), Attachment::from_row(row)))
        .collect();
    for item in items.iter_mut() {
        item.attachment = by_item.remove(&item.id);
    }
    Ok(())
}

/// Remove a pack picture's file once no row names it — AFTER the commit, and
/// never as the request's answer.
///
/// By the time this runs the change is committed and cannot be taken back,
/// so a failure here must not become the response: a `500` would tell the
/// caller that a removal which happened did not, and — returning before the
/// fan-out — would leave every connected member showing, and sending, a
/// sticker the pack no longer has, because the retry takes the idempotent
/// branch and raises no frame. What a failure leaves behind is a file with
/// no rows, which `remove_if_unreferenced` already names as the cheaper way
/// to be wrong. Logged with the error alone: the key names a member's file
/// and is no use to whoever reads the log.
async fn remove_file_after_commit(state: &AppState, storage_key: &str) {
    if let Err(err) = crate::handlers_attachment::remove_if_unreferenced(state, storage_key).await {
        tracing::warn!(error = ?err, "removing a pack picture's file failed");
    }
}

/// One LIVE item with its picture, read inside the caller's transaction —
/// the answer to a claim that found the pack already holding what it named.
async fn read_item(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    item_id: i64,
) -> Result<PackItem, ApiError> {
    let row = sqlx::query(&format!("SELECT {PACK_COLS} FROM pack_items WHERE id = $1"))
        .bind(item_id)
        .fetch_one(&mut **tx)
        .await?;
    let mut item = PackItem::from_row(&row);
    let picture = sqlx::query(&format!(
        "SELECT {PICTURE_COLS} FROM attachments WHERE pack_item_id = $1"
    ))
    .bind(item_id)
    .fetch_optional(&mut **tx)
    .await?;
    item.attachment = picture.as_ref().map(Attachment::from_row);
    Ok(item)
}

/// `GET /families/mine/pack` — the whole pack, tombstones excluded, in the
/// order the items were added.
pub async fn get_pack(auth: AuthUser, State(state): State<AppState>) -> Result<Response, ApiError> {
    let family_id = caller_family(&state, auth.user_id).await?;
    let mut tx = begin_pack_read(&state).await?;
    // The high-water mark FIRST, then the items — the order
    // `handlers_board::get_board` reads in, and the order protocol.md
    // promises. Inside one snapshot the three statements see the same
    // instant and the order decides nothing; it is kept so that the promise
    // does not rest on the isolation level alone: read this way round, a
    // mark can only ever be BELOW a change the items already show, which is
    // harmless under a client's per-item guard.
    let max_pack_seq: i64 = sqlx::query_scalar("SELECT last_pack_seq FROM families WHERE id = $1")
        .bind(family_id)
        .fetch_one(&mut *tx)
        .await?;
    // By id, which is the order they were added in and the order a panel
    // shows them — NOT by seq, as the board is: a pack has no "most
    // recently touched", and a sticker that jumped about the grid would be
    // one nobody could find twice.
    let rows = sqlx::query(&format!(
        "SELECT {PACK_COLS} FROM pack_items
         WHERE family_id = $1 AND deleted_at IS NULL
         ORDER BY id ASC"
    ))
    .bind(family_id)
    .fetch_all(&mut *tx)
    .await?;
    let mut items: Vec<PackItem> = rows.iter().map(PackItem::from_row).collect();
    attach_pictures(&mut tx, &mut items).await?;
    tx.commit().await?;

    Ok((
        StatusCode::OK,
        Json(json!({"items": items, "max_pack_seq": max_pack_seq})),
    )
        .into_response())
}

/// `GET /families/mine/pack/changes?after_seq=` — the pack catch-up,
/// tombstones INCLUDED. Looped by the client until a short page.
pub async fn get_pack_changes(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, ApiError> {
    let family_id = caller_family(&state, auth.user_id).await?;
    let after_seq = parse_pagination_param(&params, "after_seq")?.unwrap_or(0);
    let requested_limit = parse_pagination_param(&params, "limit")?;
    let limit = clamp_limit(
        requested_limit,
        state.cfg.limits.default_page_size,
        state.cfg.limits.max_page_size,
    );

    let mut tx = begin_pack_read(&state).await?;
    let rows = sqlx::query(&format!(
        "SELECT {PACK_COLS} FROM pack_items
         WHERE family_id = $1 AND pack_seq > $2
         ORDER BY pack_seq ASC LIMIT $3"
    ))
    .bind(family_id)
    .bind(after_seq)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await?;
    let mut items: Vec<PackItem> = rows.iter().map(PackItem::from_row).collect();
    attach_pictures(&mut tx, &mut items).await?;
    tx.commit().await?;

    Ok((StatusCode::OK, Json(json!({"items": items}))).into_response())
}

/// `POST /families/mine/pack` — add a sticker. Any member may.
///
/// The picture went up first, as any attachment does, and is CLAIMED here —
/// the third way an upload is claimed, beside a message and a board note.
/// Everything about what a sticker may be is therefore checked here and not
/// at the upload, which did not know what it was for: its type, and its
/// size against the pack's own ceiling.
///
/// The checks run in the order protocol.md gives, so the answer is always
/// the most basic thing wrong.
pub async fn add_pack_item(
    auth: AuthUser,
    State(state): State<AppState>,
    AppJson(req): AppJson<AddPackItemRequest>,
) -> Result<Response, ApiError> {
    let family_id = caller_family(&state, auth.user_id).await?;
    let label = validate_label(req.label.as_deref())?;

    let mut tx = state.pool.begin().await?;
    lock_pack(&mut tx, family_id).await?;

    // The caller's own upload INTO THIS FAMILY, and nobody else's — the
    // same 404 either way, so the endpoint never confirms an id exists.
    // The family is part of the question because the file is: an upload is
    // deduplicated against, and deleted with, the family it went up into,
    // and a member who changed family inside the grace period must not pin
    // the old family's file into the new family's pack.
    let upload = sqlx::query(
        "SELECT kind, mime, size_bytes, content_hash, storage_key, message_id, note_id,
                pack_item_id
         FROM attachments
         WHERE id = $1 AND uploader_id = $2 AND family_id = $3
         FOR UPDATE",
    )
    .bind(req.attachment_id)
    .bind(auth.user_id)
    .bind(family_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(upload) = upload else {
        // The one a client can act on: an upload this caller made and the
        // unclaimed sweep took (0035). "Upload the bytes again", not "give
        // up".
        let expired: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM expired_attachments WHERE id = $1 AND uploader_id = $2)",
        )
        .bind(req.attachment_id)
        .bind(auth.user_id)
        .fetch_one(&mut *tx)
        .await?;
        if expired {
            return Err(ApiError::not_found(
                codes::ATTACHMENT_EXPIRED,
                "that upload was not claimed in time and has been removed — upload it again",
            ));
        }
        return Err(ApiError::not_found(
            codes::ATTACHMENT_NOT_FOUND,
            "no such attachment",
        ));
    };

    // THE SAME CLAIM, AGAIN. A retry after a lost answer names an upload
    // that is already this pack's item, and the truthful reply is that
    // item — `attachment_already_used` would tell a client its sticker was
    // refused when it was in fact added.
    if let Some(item_id) = upload.get::<Option<i64>, _>("pack_item_id") {
        let item = read_item(&mut tx, item_id).await?;
        tx.commit().await?;
        return Ok((StatusCode::OK, Json(json!({"item": item}))).into_response());
    }

    // What it IS, before whether it is free: "that is a video" is a
    // different instruction from "that one is taken" — the board's order.
    let mime: String = upload.get("mime");
    if upload.get::<String, _>("kind") != Attachment::KIND_PHOTO
        || !Attachment::is_sticker_mime(&mime)
    {
        return Err(ApiError::bad_request(
            codes::INVALID_ATTACHMENT,
            "a sticker is a WebP or PNG photo",
        ));
    }
    if upload.get::<Option<i64>, _>("message_id").is_some()
        || upload.get::<Option<i64>, _>("note_id").is_some()
    {
        return Err(ApiError::conflict(
            codes::ATTACHMENT_ALREADY_USED,
            "that attachment is already on a message or pinned to the board",
        ));
    }
    // HERE, because the upload did not know it would become a sticker: it
    // was accepted under the attachment ceiling, which is two hundred times
    // this one.
    let size: i64 = upload.get("size_bytes");
    if size > state.cfg.limits.max_pack_item_bytes as i64 {
        return Err(ApiError::payload_too_large(
            codes::PACK_ITEM_TOO_LARGE,
            format!(
                "a sticker is at most {} bytes",
                state.cfg.limits.max_pack_item_bytes
            ),
        ));
    }

    // DOES THE PACK ALREADY HOLD THESE BYTES? Then it holds this sticker,
    // and the answer is the item that is there. Matched on size as well as
    // digest, the belt and braces the upload's own dedup wears. Before the
    // ceiling on purpose: a claim that adds nothing cannot overfill
    // anything, and "the pack is full" would be the wrong thing to tell
    // somebody about a sticker the pack has.
    let content_hash: Option<String> = upload.get("content_hash");
    if let Some(content_hash) = &content_hash {
        let held: Option<i64> = sqlx::query_scalar(
            "SELECT p.id
             FROM pack_items p
             JOIN attachments a ON a.pack_item_id = p.id
             WHERE p.family_id = $1 AND p.deleted_at IS NULL
               AND a.content_hash = $2 AND a.size_bytes = $3
             ORDER BY p.id
             LIMIT 1",
        )
        .bind(family_id)
        .bind(content_hash)
        .bind(size)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(item_id) = held {
            // The fresh upload has nothing left to do: drop its row now
            // rather than leave it for the sweep. Its FILE is almost always
            // the very one the item names (the upload deduplicated against
            // it), so it is removed only if no row still does.
            let storage_key: String = upload.get("storage_key");
            sqlx::query("DELETE FROM attachments WHERE id = $1")
                .bind(req.attachment_id)
                .execute(&mut *tx)
                .await?;
            let item = read_item(&mut tx, item_id).await?;
            tx.commit().await?;
            remove_file_after_commit(&state, &storage_key).await;
            return Ok((StatusCode::OK, Json(json!({"item": item}))).into_response());
        }
    }

    let live: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pack_items WHERE family_id = $1 AND deleted_at IS NULL",
    )
    .bind(family_id)
    .fetch_one(&mut *tx)
    .await?;
    if live >= state.cfg.limits.max_pack_items {
        return Err(ApiError::conflict(
            codes::PACK_FULL,
            format!(
                "the sticker pack is full ({} stickers); remove one first",
                state.cfg.limits.max_pack_items
            ),
        ));
    }

    let seq: i64 = sqlx::query_scalar("SELECT nextval('family_pack_seq')")
        .fetch_one(&mut *tx)
        .await?;
    let row = sqlx::query(&format!(
        "INSERT INTO pack_items (family_id, added_by, label, pack_seq)
         VALUES ($1, $2, $3, $4)
         RETURNING {PACK_COLS}"
    ))
    .bind(family_id)
    .bind(auth.user_id)
    .bind(label.as_deref())
    .bind(seq)
    .fetch_one(&mut *tx)
    .await?;
    advance_family_seq(&mut tx, family_id, seq).await?;
    let mut item = PackItem::from_row(&row);

    // The claim, in the transaction that made the item: an item committed
    // beside an unclaimed picture would point at nothing, and the sweep
    // would take the picture hours later. The row is locked and was just
    // seen free, so this cannot miss.
    let picture = sqlx::query(&format!(
        "UPDATE attachments SET pack_item_id = $2 WHERE id = $1 RETURNING {PICTURE_COLS}"
    ))
    .bind(req.attachment_id)
    .bind(item.id)
    .fetch_one(&mut *tx)
    .await?;
    item.attachment = Some(Attachment::from_row(&picture));
    tx.commit().await?;

    events::log_fanout_error(
        "pack_item",
        events::deliver_pack_item(&state, family_id, &item).await,
    );
    Ok((StatusCode::CREATED, Json(json!({"item": item}))).into_response())
}

/// `DELETE /families/mine/pack/{id}` — whoever added it, or the family
/// owner. Tombstoned, idempotent.
pub async fn remove_pack_item(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(item_id): Path<i64>,
) -> Result<Response, ApiError> {
    let family_id = caller_family(&state, auth.user_id).await?;

    let mut tx = state.pool.begin().await?;
    lock_pack(&mut tx, family_id).await?;
    // Scoped to the CALLER'S family: an item of another family is the same
    // 404 as one that never existed — and so is the caller's own item in a
    // family they have since left, which is the owner's to remove now.
    let locked = sqlx::query(
        "SELECT p.added_by, p.deleted_at, f.owner_user_id
         FROM pack_items p
         JOIN families f ON f.id = p.family_id
         WHERE p.id = $1 AND p.family_id = $2
         FOR UPDATE OF p",
    )
    .bind(item_id)
    .bind(family_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(locked) = locked else {
        return Err(ApiError::not_found(
            codes::PACK_ITEM_NOT_FOUND,
            "no such sticker in this family's pack",
        ));
    };
    // The author OR THE OWNER — the shape the board does not have, because
    // a departed member's stickers would otherwise be nobody's to remove.
    if locked.get::<i64, _>("added_by") != auth.user_id
        && locked.get::<i64, _>("owner_user_id") != auth.user_id
    {
        return Err(ApiError::forbidden(
            codes::NOT_PACK_ITEM_AUTHOR,
            "only whoever added this sticker, or the family owner, can remove it",
        ));
    }
    if locked
        .get::<Option<time::OffsetDateTime>, _>("deleted_at")
        .is_some()
    {
        // Idempotent: already a tombstone, so no new seq and no fan-out.
        tx.commit().await?;
        return Ok(StatusCode::NO_CONTENT.into_response());
    }

    let seq: i64 = sqlx::query_scalar("SELECT nextval('family_pack_seq')")
        .fetch_one(&mut *tx)
        .await?;
    let row = sqlx::query(&format!(
        "UPDATE pack_items SET deleted_at = now(), pack_seq = $2 WHERE id = $1
         RETURNING {PACK_COLS}"
    ))
    .bind(item_id)
    .bind(seq)
    .fetch_one(&mut *tx)
    .await?;
    advance_family_seq(&mut tx, family_id, seq).await?;
    // The pack's picture goes with the item: the tombstone carries no
    // content, so nothing left could show it. The ROW goes here, inside the
    // transaction; the FILE after the commit and only if no other row names
    // it — which is what keeps every message ever sent with this sticker
    // drawing, since each holds a row of its own over the same bytes.
    let orphaned: Option<String> =
        sqlx::query_scalar("DELETE FROM attachments WHERE pack_item_id = $1 RETURNING storage_key")
            .bind(item_id)
            .fetch_optional(&mut *tx)
            .await?;
    tx.commit().await?;
    // The frame FIRST, then the file: from the commit on, nothing may stand
    // between a removal and the members being told of it.
    let item = PackItem::from_row(&row);
    events::log_fanout_error(
        "pack_item",
        events::deliver_pack_item(&state, family_id, &item).await,
    );
    if let Some(storage_key) = orphaned {
        remove_file_after_commit(&state, &storage_key).await;
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A label is a few words for a screen reader: trimmed, optional, and
    /// an empty one is NO label rather than an empty string on the wire.
    #[test]
    fn a_label_is_trimmed_optional_and_capped() {
        assert_eq!(validate_label(None).expect("absent is fine"), None);
        assert_eq!(validate_label(Some("   ")).expect("blank is none"), None);
        assert_eq!(
            validate_label(Some("  party cat ")).expect("trimmed"),
            Some("party cat".to_string())
        );
        // Characters, not bytes: 64 Cyrillic letters are 128 bytes and fit.
        let longest = "я".repeat(PackItem::MAX_LABEL_CHARS);
        assert_eq!(
            validate_label(Some(&longest)).expect("exactly the cap"),
            Some(longest.clone())
        );
        let over = format!("{longest}я");
        assert!(validate_label(Some(&over)).is_err(), "one over is refused");
    }
}
