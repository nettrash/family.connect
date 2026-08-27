//! The family board: a wall of sticker notes (docs/protocol.md, "Board").
//!
//! Two authorship rules, deliberately different: **anyone in the family may
//! MOVE a note; only its author may change its text, colour or size, or
//! delete it.** Tidying the wall is a shared act; rewriting someone's words
//! is not — and neither is deciding how loudly they speak, since a size
//! anyone could change is a size anyone could shrink to nothing.
//!
//! Every mutation takes the next `family_board_seq` and stamps it on the
//! note, with the family's `last_board_seq` following GREATEST-guarded —
//! the third cursor of the same shape as reactions and edits, because
//! `after_id` cannot see a change to an older row and a board is nothing
//! but changes to older rows.
//!
//! Deletes leave TOMBSTONES: the row keeps its id, takes a new seq, and
//! reports `deleted: true` in the change feed. Without that, a client who
//! was offline when a note was removed has no way to learn it is gone.

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
use crate::models::Note;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct CreateNoteRequest {
    pub text: String,
    pub color: String,
    /// Absent means `medium`: a client that predates sizes keeps making
    /// the notes it always made.
    #[serde(default)]
    pub size: Option<String>,
    pub x: f64,
    pub y: f64,
}

/// Every field optional: a move sends only `x`/`y`, an edit any of `text`,
/// `color` and `size`. Which fields are present is what decides whether
/// the caller needs to be the author.
#[derive(Debug, Deserialize)]
pub struct PatchNoteRequest {
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub size: Option<String>,
    #[serde(default)]
    pub x: Option<f64>,
    #[serde(default)]
    pub y: Option<f64>,
}

/// The family the caller belongs to, or `not_in_family`.
async fn caller_family(state: &AppState, user_id: i64) -> Result<i64, ApiError> {
    let row = sqlx::query("SELECT family_id FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(&state.pool)
        .await?;
    row.and_then(|row| row.get::<Option<i64>, _>("family_id"))
        .ok_or_else(|| ApiError::conflict(codes::NOT_IN_FAMILY, "you are not in a family"))
}

fn validate_text(text: &str) -> Result<String, ApiError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(ApiError::validation("note text is empty"));
    }
    if text.chars().count() > Note::MAX_TEXT_CHARS {
        return Err(ApiError::validation(format!(
            "note text exceeds {} characters",
            Note::MAX_TEXT_CHARS
        )));
    }
    Ok(text.to_string())
}

fn validate_color(color: &str) -> Result<String, ApiError> {
    if Note::COLORS.contains(&color) {
        Ok(color.to_string())
    } else {
        Err(ApiError::bad_request(
            codes::INVALID_NOTE_COLOR,
            format!("color must be one of {}", Note::COLORS.join(", ")),
        ))
    }
}

fn validate_size(size: &str) -> Result<String, ApiError> {
    if Note::SIZES.contains(&size) {
        Ok(size.to_string())
    } else {
        Err(ApiError::bad_request(
            codes::INVALID_NOTE_SIZE,
            format!("size must be one of {}", Note::SIZES.join(", ")),
        ))
    }
}

const NOTE_COLS: &str =
    "id, author_id, text, color, size, x, y, board_seq, created_at, updated_at, deleted_at";

/// `GET /families/mine/board` — the whole board, tombstones excluded.
pub async fn get_board(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Response, ApiError> {
    let family_id = caller_family(&state, auth.user_id).await?;
    let rows = sqlx::query(&format!(
        "SELECT {NOTE_COLS} FROM notes
         WHERE family_id = $1 AND deleted_at IS NULL
         ORDER BY board_seq DESC"
    ))
    .bind(family_id)
    .fetch_all(&state.pool)
    .await?;
    let notes: Vec<Note> = rows.iter().map(Note::from_row).collect();

    let max_board_seq: i64 =
        sqlx::query_scalar("SELECT last_board_seq FROM families WHERE id = $1")
            .bind(family_id)
            .fetch_one(&state.pool)
            .await?;

    Ok((
        StatusCode::OK,
        Json(json!({"notes": notes, "max_board_seq": max_board_seq})),
    )
        .into_response())
}

/// `GET /families/mine/board/changes?after_seq=` — the board catch-up,
/// tombstones INCLUDED. Looped by the client until a short page.
pub async fn get_board_changes(
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

    let rows = sqlx::query(&format!(
        "SELECT {NOTE_COLS} FROM notes
         WHERE family_id = $1 AND board_seq > $2
         ORDER BY board_seq ASC LIMIT $3"
    ))
    .bind(family_id)
    .bind(after_seq)
    .bind(limit)
    .fetch_all(&state.pool)
    .await?;
    let notes: Vec<Note> = rows.iter().map(Note::from_row).collect();

    Ok((StatusCode::OK, Json(json!({"notes": notes}))).into_response())
}

/// `POST /families/mine/board/notes` — add a note. The caller is its author.
pub async fn create_note(
    auth: AuthUser,
    State(state): State<AppState>,
    AppJson(req): AppJson<CreateNoteRequest>,
) -> Result<Response, ApiError> {
    let family_id = caller_family(&state, auth.user_id).await?;
    let text = validate_text(&req.text)?;
    let color = validate_color(&req.color)?;
    let size = match req.size.as_deref() {
        Some(size) => validate_size(size)?,
        None => Note::DEFAULT_SIZE.to_string(),
    };

    let mut tx = state.pool.begin().await?;
    let live: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM notes WHERE family_id = $1 AND deleted_at IS NULL",
    )
    .bind(family_id)
    .fetch_one(&mut *tx)
    .await?;
    if live >= state.cfg.limits.max_board_notes {
        return Err(ApiError::conflict(
            codes::BOARD_FULL,
            format!(
                "the board is full ({} notes); delete one first",
                state.cfg.limits.max_board_notes
            ),
        ));
    }

    let seq: i64 = sqlx::query_scalar("SELECT nextval('family_board_seq')")
        .fetch_one(&mut *tx)
        .await?;
    let row = sqlx::query(&format!(
        "INSERT INTO notes (family_id, author_id, text, color, size, x, y, board_seq)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         RETURNING {NOTE_COLS}"
    ))
    .bind(family_id)
    .bind(auth.user_id)
    .bind(&text)
    .bind(&color)
    .bind(&size)
    .bind(Note::clamp_position(req.x))
    .bind(Note::clamp_position(req.y))
    .bind(seq)
    .fetch_one(&mut *tx)
    .await?;
    advance_family_seq(&mut tx, family_id, seq).await?;
    tx.commit().await?;

    let note = Note::from_row(&row);
    // Creation is the one board event that notifies (protocol.md, "Board").
    events::log_fanout_error(
        "board_note",
        events::deliver_new_board_note(&state, family_id, &note).await,
    );
    Ok((StatusCode::CREATED, Json(json!({"note": note}))).into_response())
}

/// `PATCH /families/mine/board/notes/{id}` — move it (anyone) or rewrite,
/// recolour or resize it (the author).
pub async fn patch_note(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(note_id): Path<i64>,
    AppJson(req): AppJson<PatchNoteRequest>,
) -> Result<Response, ApiError> {
    let family_id = caller_family(&state, auth.user_id).await?;
    let text = req.text.as_deref().map(validate_text).transpose()?;
    let color = req.color.as_deref().map(validate_color).transpose()?;
    let size = req.size.as_deref().map(validate_size).transpose()?;

    let mut tx = state.pool.begin().await?;
    let locked = sqlx::query(
        "SELECT author_id, text, color, size, x, y, deleted_at FROM notes
         WHERE id = $1 AND family_id = $2 FOR UPDATE",
    )
    .bind(note_id)
    .bind(family_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(locked) = locked else {
        return Err(ApiError::not_found(
            codes::NOTE_NOT_FOUND,
            "no such note on this board",
        ));
    };
    if locked
        .get::<Option<time::OffsetDateTime>, _>("deleted_at")
        .is_some()
    {
        // A tombstone is gone as far as anyone editing is concerned.
        return Err(ApiError::not_found(
            codes::NOTE_NOT_FOUND,
            "no such note on this board",
        ));
    }

    // The split rule: content — text, colour and size — is the author's,
    // position is everyone's.
    if (text.is_some() || color.is_some() || size.is_some())
        && locked.get::<i64, _>("author_id") != auth.user_id
    {
        return Err(ApiError::forbidden(
            codes::NOT_NOTE_AUTHOR,
            "only the author can change this note's text, colour or size",
        ));
    }

    let next_text = text.unwrap_or_else(|| locked.get("text"));
    let next_color = color.unwrap_or_else(|| locked.get("color"));
    let next_size = size.unwrap_or_else(|| locked.get("size"));
    let next_x = req
        .x
        .map(Note::clamp_position)
        .unwrap_or_else(|| locked.get("x"));
    let next_y = req
        .y
        .map(Note::clamp_position)
        .unwrap_or_else(|| locked.get("y"));

    let changed = next_text != locked.get::<String, _>("text")
        || next_color != locked.get::<String, _>("color")
        || next_size != locked.get::<String, _>("size")
        || next_x != locked.get::<f64, _>("x")
        || next_y != locked.get::<f64, _>("y");

    let row = if changed {
        let seq: i64 = sqlx::query_scalar("SELECT nextval('family_board_seq')")
            .fetch_one(&mut *tx)
            .await?;
        let row = sqlx::query(&format!(
            "UPDATE notes SET text = $2, color = $3, size = $4, x = $5, y = $6,
                              board_seq = $7, updated_at = now()
             WHERE id = $1
             RETURNING {NOTE_COLS}"
        ))
        .bind(note_id)
        .bind(&next_text)
        .bind(&next_color)
        .bind(&next_size)
        .bind(next_x)
        .bind(next_y)
        .bind(seq)
        .fetch_one(&mut *tx)
        .await?;
        advance_family_seq(&mut tx, family_id, seq).await?;
        row
    } else {
        // A no-op takes no sequence value and raises no fan-out, exactly
        // like re-sending a message's existing body.
        sqlx::query(&format!("SELECT {NOTE_COLS} FROM notes WHERE id = $1"))
            .bind(note_id)
            .fetch_one(&mut *tx)
            .await?
    };
    tx.commit().await?;

    let note = Note::from_row(&row);
    if changed {
        events::log_fanout_error(
            "board_note",
            events::deliver_board_note(&state, family_id, &note).await,
        );
    }
    Ok((StatusCode::OK, Json(json!({"note": note}))).into_response())
}

/// `DELETE /families/mine/board/notes/{id}` — author only, tombstoned.
pub async fn delete_note(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(note_id): Path<i64>,
) -> Result<Response, ApiError> {
    let family_id = caller_family(&state, auth.user_id).await?;

    let mut tx = state.pool.begin().await?;
    let locked = sqlx::query(
        "SELECT author_id, deleted_at FROM notes WHERE id = $1 AND family_id = $2 FOR UPDATE",
    )
    .bind(note_id)
    .bind(family_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(locked) = locked else {
        return Err(ApiError::not_found(
            codes::NOTE_NOT_FOUND,
            "no such note on this board",
        ));
    };
    if locked.get::<i64, _>("author_id") != auth.user_id {
        return Err(ApiError::forbidden(
            codes::NOT_NOTE_AUTHOR,
            "only the author can delete this note",
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

    let seq: i64 = sqlx::query_scalar("SELECT nextval('family_board_seq')")
        .fetch_one(&mut *tx)
        .await?;
    let row = sqlx::query(&format!(
        "UPDATE notes SET deleted_at = now(), board_seq = $2 WHERE id = $1
         RETURNING {NOTE_COLS}"
    ))
    .bind(note_id)
    .bind(seq)
    .fetch_one(&mut *tx)
    .await?;
    advance_family_seq(&mut tx, family_id, seq).await?;
    tx.commit().await?;

    let note = Note::from_row(&row);
    events::log_fanout_error(
        "board_note",
        events::deliver_board_note(&state, family_id, &note).await,
    );
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// GREATEST, not plain SET: two notes can commit out of seq order and the
/// family's cursor must never move backwards.
async fn advance_family_seq(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    family_id: i64,
    seq: i64,
) -> Result<(), ApiError> {
    sqlx::query("UPDATE families SET last_board_seq = GREATEST(last_board_seq, $2) WHERE id = $1")
        .bind(family_id)
        .bind(seq)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
