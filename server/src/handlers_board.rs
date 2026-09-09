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
//!
//! A note carries a SECOND stamp from the same sequence, `content_seq`,
//! reset only when its `text` changes. `board_seq` answers "what has
//! changed?" and must move for a drag, or the move never reaches the other
//! devices; `content_seq` answers "is there anything to READ?", which is
//! the only question a badge may ask (migration 0031, protocol.md,
//! "Board").

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
use crate::models::{Attachment, Note, Rsvp};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct CreateNoteRequest {
    pub text: String,
    pub color: String,
    /// Absent means `medium`: a client that predates sizes keeps making
    /// the notes it always made.
    #[serde(default)]
    pub size: Option<String>,
    /// Absent means `plain`, for the same reason.
    #[serde(default)]
    pub font: Option<String>,
    /// Absent means `text`: a client that predates kinds keeps pinning the
    /// notes it always pinned.
    #[serde(default)]
    pub kind: Option<String>,
    /// The picture: required by a `photo` note, optional on an `event`
    /// (the backdrop), refused on a text one.
    #[serde(default)]
    pub attachment_id: Option<i64>,
    /// An `event` and nowhere else. `starts_at` is required there.
    #[serde(default)]
    pub starts_at: Option<String>,
    #[serde(default)]
    pub ends_at: Option<String>,
    #[serde(default)]
    pub place: Option<String>,
    pub x: f64,
    pub y: f64,
}

/// Every field optional: a move sends only `x`/`y`, an edit any of `text`,
/// `color`, `size` and `font`. Which fields are present is what decides whether
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
    pub font: Option<String>,
    /// An event's own three. `place` may be sent empty to clear it;
    /// `ends_at` may be sent null, which is why it is a double option —
    /// absent leaves it alone, null clears it.
    #[serde(default)]
    pub starts_at: Option<String>,
    #[serde(default, deserialize_with = "present_option")]
    pub ends_at: Option<Option<String>>,
    #[serde(default)]
    pub place: Option<String>,
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

/// The event's three fields, checked together because they only make
/// sense together (docs/protocol.md, "Board").
///
/// Returns what to store. `place` comes back trimmed, and an empty one
/// comes back as `Some("")` rather than `None` so a PATCH can CLEAR it —
/// absent means "leave it alone" everywhere on this wire, and the two must
/// not be the same value.
type EventFields = (
    Option<time::OffsetDateTime>,
    Option<time::OffsetDateTime>,
    Option<String>,
);

fn validate_event_fields(
    is_event: bool,
    starts_at: Option<&str>,
    ends_at: Option<&str>,
    place: Option<&str>,
) -> Result<EventFields, ApiError> {
    if !is_event {
        if starts_at.is_some() || ends_at.is_some() || place.is_some() {
            return Err(ApiError::validation(
                "starts_at, ends_at and place are only accepted on an event",
            ));
        }
        return Ok((None, None, None));
    }
    let parse = |value: &str, field: &str| {
        time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
            .map_err(|_| ApiError::validation(format!("{field} is not an RFC3339 timestamp")))
    };
    let starts = starts_at.map(|value| parse(value, "starts_at")).transpose()?;
    let ends = ends_at.map(|value| parse(value, "ends_at")).transpose()?;
    // Only when BOTH are known: a PATCH may send one and not the other, and
    // the caller checks the stored pair afterwards.
    if let (Some(starts), Some(ends)) = (starts, ends)
        && ends < starts
    {
        return Err(ApiError::validation("ends_at is before starts_at"));
    }
    let place = match place {
        None => None,
        Some(value) => {
            let trimmed = value.trim();
            if trimmed.chars().count() > Note::MAX_PLACE_CHARS {
                return Err(ApiError::validation(format!(
                    "place exceeds {} characters",
                    Note::MAX_PLACE_CHARS
                )));
            }
            Some(trimmed.to_string())
        }
    };
    Ok((starts, ends, place))
}

/// Deserialize a present key into `Some(...)`, so `#[serde(default)]` keeps
/// an ABSENT key as `None`. The same three lines as `handlers_device` and
/// `handlers_family`, and here for the same reason: an `ends_at` somebody
/// CLEARED and an `ends_at` nobody touched are different instructions.
fn present_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

fn validate_kind(kind: &str) -> Result<String, ApiError> {
    if Note::KINDS.contains(&kind) {
        Ok(kind.to_string())
    } else {
        Err(ApiError::bad_request(
            codes::INVALID_NOTE_KIND,
            format!("kind must be one of {}", Note::KINDS.join(", ")),
        ))
    }
}

fn validate_font(font: &str) -> Result<String, ApiError> {
    if Note::FONTS.contains(&font) {
        Ok(font.to_string())
    } else {
        Err(ApiError::bad_request(
            codes::INVALID_NOTE_FONT,
            format!("font must be one of {}", Note::FONTS.join(", ")),
        ))
    }
}

const NOTE_COLS: &str = "id, author_id, text, color, size, font, kind, starts_at, ends_at, \
     place, x, y, board_seq, content_seq, created_at, updated_at, deleted_at";

/// Pin one uploaded picture to a note — the board's twin of
/// `handlers_chat::claim_attachment`, and the same three answers.
///
/// Runs inside the caller's transaction so a refusal takes the note with
/// it: a photo note committed beside an unclaimed picture would point at
/// nothing, and the sweep would remove the picture hours later.
///
/// A WALL PINS PICTURES. A video or a voice message on a corkboard is a
/// thing to play rather than a thing to look at, and a location has no
/// bytes at all — so the kind is checked here, before the claim.
async fn claim_picture(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    attachment_id: i64,
    uploader_id: i64,
    note_id: i64,
) -> Result<Attachment, ApiError> {
    let row = sqlx::query(
        "UPDATE attachments SET note_id = $3
         WHERE id = $1 AND uploader_id = $2 AND message_id IS NULL AND note_id IS NULL
           AND kind = $4
         RETURNING id, kind, mime, size_bytes, width, height, duration_ms, has_preview, name,
                   latitude, longitude, accuracy_m",
    )
    .bind(attachment_id)
    .bind(uploader_id)
    .bind(note_id)
    .bind(Attachment::KIND_PHOTO)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(row) = row {
        return Ok(Attachment::from_row(&row));
    }

    // Which of the four it was. `kind` first, because "that is a video"
    // is a different instruction from "that one is taken".
    let existing = sqlx::query(
        "SELECT kind, message_id, note_id FROM attachments WHERE id = $1 AND uploader_id = $2",
    )
    .bind(attachment_id)
    .bind(uploader_id)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(existing) = existing {
        if existing.get::<String, _>("kind") != Attachment::KIND_PHOTO {
            return Err(ApiError::bad_request(
                codes::INVALID_ATTACHMENT,
                "a board note pins a photo",
            ));
        }
        return Err(ApiError::conflict(
            codes::ATTACHMENT_ALREADY_USED,
            "that attachment is already on a message or another note",
        ));
    }
    // The one a client can act on: an upload this caller made and the
    // unclaimed sweep took (0035). "Upload the bytes again", not "give up".
    let expired: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM expired_attachments WHERE id = $1 AND uploader_id = $2)",
    )
    .bind(attachment_id)
    .bind(uploader_id)
    .fetch_one(&mut **tx)
    .await?;
    if expired {
        return Err(ApiError::not_found(
            codes::ATTACHMENT_EXPIRED,
            "that upload was not claimed in time and has been removed — upload it again",
        ));
    }
    Err(ApiError::not_found(
        codes::ATTACHMENT_NOT_FOUND,
        "no such attachment",
    ))
}

/// The picture a `photo` note carries, looked up per note.
///
/// A second query rather than a join on every board read: a wall is at most
/// `max_board_notes` rows and only some of them are pictures, and widening
/// `NOTE_COLS` with twelve attachment columns would put them on every text
/// note's SELECT as well.
async fn attach_picture(state: &AppState, note: &mut Note) -> Result<(), ApiError> {
    if note.kind.as_deref() != Some(Note::KIND_PHOTO) {
        return Ok(());
    }
    let row = sqlx::query(
        "SELECT id, kind, mime, size_bytes, width, height, duration_ms, has_preview, name,
                latitude, longitude, accuracy_m
         FROM attachments WHERE note_id = $1",
    )
    .bind(note.id)
    .fetch_optional(&state.pool)
    .await?;
    note.attachment = row.as_ref().map(Attachment::from_row);
    Ok(())
}

/// Who has said they are coming — on an event, and `[]` rather than absent
/// there, because "nobody has answered yet" is an answer and a missing
/// field is not (docs/protocol.md, "Board").
async fn attach_rsvps(state: &AppState, note: &mut Note) -> Result<(), ApiError> {
    if note.kind.as_deref() != Some(Note::KIND_EVENT) {
        return Ok(());
    }
    let rows = sqlx::query(
        "SELECT user_id, answer FROM note_rsvps WHERE note_id = $1 ORDER BY user_id",
    )
    .bind(note.id)
    .fetch_all(&state.pool)
    .await?;
    note.rsvps = Some(rows.iter().map(Rsvp::from_row).collect());
    Ok(())
}

/// Everything a note carries beside its own row.
async fn hydrate(state: &AppState, note: &mut Note) -> Result<(), ApiError> {
    attach_picture(state, note).await?;
    attach_rsvps(state, note).await?;
    Ok(())
}

/// The same, for a page of them.
async fn attach_pictures(state: &AppState, notes: &mut [Note]) -> Result<(), ApiError> {
    for note in notes.iter_mut() {
        hydrate(state, note).await?;
    }
    Ok(())
}

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
    let mut notes: Vec<Note> = rows.iter().map(Note::from_row).collect();
    attach_pictures(&state, &mut notes).await?;

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
    let mut notes: Vec<Note> = rows.iter().map(Note::from_row).collect();
    attach_pictures(&state, &mut notes).await?;

    Ok((StatusCode::OK, Json(json!({"notes": notes}))).into_response())
}

/// `POST /families/mine/board/notes` — add a note. The caller is its author.
pub async fn create_note(
    auth: AuthUser,
    State(state): State<AppState>,
    AppJson(req): AppJson<CreateNoteRequest>,
) -> Result<Response, ApiError> {
    let family_id = caller_family(&state, auth.user_id).await?;
    let kind = match req.kind.as_deref() {
        Some(kind) => validate_kind(kind)?,
        None => Note::DEFAULT_KIND.to_string(),
    };
    let is_photo = kind == Note::KIND_PHOTO;
    let is_event = kind == Note::KIND_EVENT;
    // A photo note IS its picture, so it must have one; an event may have
    // a backdrop and need not; a text note has nowhere to put one
    // (protocol.md, "Board").
    match (is_photo, is_event, req.attachment_id) {
        (true, _, None) => {
            return Err(ApiError::validation("a photo note needs an attachment_id"));
        }
        (false, false, Some(_)) => {
            return Err(ApiError::validation(
                "attachment_id is only accepted on a photo or event note",
            ));
        }
        _ => {}
    }
    // The event's own three, required and accepted only there.
    let (starts_at, ends_at, place) = validate_event_fields(
        is_event,
        req.starts_at.as_deref(),
        req.ends_at.as_deref(),
        req.place.as_deref(),
    )?;
    if is_event && starts_at.is_none() {
        return Err(ApiError::validation("an event needs a starts_at"));
    }
    // A photo's caption may be empty, the same relaxation a message
    // carrying an attachment gets; a text note still has to say something.
    let text = if is_photo && req.text.trim().is_empty() {
        String::new()
    } else {
        validate_text(&req.text)?
    };
    let color = validate_color(&req.color)?;
    let size = match req.size.as_deref() {
        Some(size) => validate_size(size)?,
        None => Note::DEFAULT_SIZE.to_string(),
    };
    let font = match req.font.as_deref() {
        Some(font) => validate_font(font)?,
        None => Note::DEFAULT_FONT.to_string(),
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
    // A new note IS new text, so both stamps take the same value: there is
    // nothing on the wall yet that anybody could have read.
    let row = sqlx::query(&format!(
        "INSERT INTO notes (family_id, author_id, text, color, size, font, kind,
                            starts_at, ends_at, place, x, y, board_seq, content_seq)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $13)
         RETURNING {NOTE_COLS}"
    ))
    .bind(family_id)
    .bind(auth.user_id)
    .bind(&text)
    .bind(&color)
    .bind(&size)
    .bind(&font)
    .bind(&kind)
    .bind(starts_at)
    .bind(ends_at)
    .bind(place.as_deref())
    .bind(Note::clamp_position(req.x))
    .bind(Note::clamp_position(req.y))
    .bind(seq)
    .fetch_one(&mut *tx)
    .await?;
    advance_family_seq(&mut tx, family_id, seq).await?;

    let mut note = Note::from_row(&row);
    // Claimed INSIDE the transaction, so a refusal takes the note with it:
    // a photo note committed beside an unclaimed picture would be a note
    // pointing at nothing, and the sweeper would take the picture later.
    if let Some(attachment_id) = req.attachment_id {
        note.attachment =
            Some(claim_picture(&mut tx, attachment_id, auth.user_id, note.id).await?);
    }
    // An event is born with nobody having answered, which is `[]` and not
    // absent (protocol.md, "Board").
    if is_event {
        note.rsvps = Some(Vec::new());
    }
    tx.commit().await?;
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
    // Validated AFTER the row is locked, because whether an empty caption
    // is allowed depends on the note's kind (protocol.md, "Board").
    let raw_text = req.text.clone();
    let color = req.color.as_deref().map(validate_color).transpose()?;
    let size = req.size.as_deref().map(validate_size).transpose()?;
    let font = req.font.as_deref().map(validate_font).transpose()?;

    let mut tx = state.pool.begin().await?;
    let locked = sqlx::query(
        "SELECT author_id, text, color, size, font, kind, starts_at, ends_at, place,
                x, y, content_seq, deleted_at
         FROM notes WHERE id = $1 AND family_id = $2 FOR UPDATE",
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

    // A photo note's caption may be emptied; a text note still has to say
    // something. The kind is fixed at creation, so this is the note's own.
    let locked_kind: String = locked.get("kind");
    let is_photo = locked_kind == Note::KIND_PHOTO;
    let is_event = locked_kind == Note::KIND_EVENT;
    // The event's three, against the note's OWN kind — they are refused on
    // anything else, exactly as at creation.
    let (patch_starts, patch_ends, patch_place) = validate_event_fields(
        is_event,
        req.starts_at.as_deref(),
        req.ends_at.as_ref().and_then(|value| value.as_deref()),
        req.place.as_deref(),
    )?;
    let text = match raw_text.as_deref() {
        None => None,
        Some(candidate) if is_photo && candidate.trim().is_empty() => Some(String::new()),
        Some(candidate) => Some(validate_text(candidate)?),
    };

    // The split rule: content — text, colour, size, font, and an event's
    // when and where — is the author's; position is everyone's, and so is
    // answering an event, which has its own route.
    let touches_content = text.is_some()
        || color.is_some()
        || size.is_some()
        || font.is_some()
        || req.starts_at.is_some()
        || req.ends_at.is_some()
        || req.place.is_some();
    if touches_content && locked.get::<i64, _>("author_id") != auth.user_id {
        return Err(ApiError::forbidden(
            codes::NOT_NOTE_AUTHOR,
            "only the author can change this note's text, colour, size, font, times or place",
        ));
    }

    let next_text = text.unwrap_or_else(|| locked.get("text"));
    let next_color = color.unwrap_or_else(|| locked.get("color"));
    let next_size = size.unwrap_or_else(|| locked.get("size"));
    let next_font = font.unwrap_or_else(|| locked.get("font"));
    let next_starts: Option<time::OffsetDateTime> =
        patch_starts.or_else(|| locked.get("starts_at"));
    // Absent leaves it alone; an explicit null clears it.
    let next_ends: Option<time::OffsetDateTime> = match req.ends_at {
        None => locked.get("ends_at"),
        Some(None) => None,
        Some(Some(_)) => patch_ends,
    };
    let next_place: Option<String> = match patch_place {
        // Empty CLEARS: a place somebody wiped is no place, not a blank one.
        Some(value) if value.is_empty() => None,
        Some(value) => Some(value),
        None => locked.get("place"),
    };
    // Checked against the STORED pair, not only the sent one: a PATCH that
    // moves the start past an untouched end is the ordinary way to get here.
    if let (Some(starts), Some(ends)) = (next_starts, next_ends)
        && ends < starts
    {
        return Err(ApiError::validation("ends_at is before starts_at"));
    }
    let next_x = req
        .x
        .map(Note::clamp_position)
        .unwrap_or_else(|| locked.get("x"));
    let next_y = req
        .y
        .map(Note::clamp_position)
        .unwrap_or_else(|| locked.get("y"));

    // The TEXT is the only field a badge speaks for: a note that was moved,
    // resized or recoloured says exactly what it said before, and telling
    // somebody there is something to read would be a lie (protocol.md,
    // "Board"). Hence two comparisons, not one — `changed` decides whether
    // anything happened at all, `text_changed` whether it is worth a badge.
    let text_changed = next_text != locked.get::<String, _>("text");
    let changed = text_changed
        || next_color != locked.get::<String, _>("color")
        || next_size != locked.get::<String, _>("size")
        || next_font != locked.get::<String, _>("font")
        || next_starts != locked.get::<Option<time::OffsetDateTime>, _>("starts_at")
        || next_ends != locked.get::<Option<time::OffsetDateTime>, _>("ends_at")
        || next_place != locked.get::<Option<String>, _>("place")
        || next_x != locked.get::<f64, _>("x")
        || next_y != locked.get::<f64, _>("y");

    let row = if changed {
        let seq: i64 = sqlx::query_scalar("SELECT nextval('family_board_seq')")
            .fetch_one(&mut *tx)
            .await?;
        // A rewrite moves both stamps to the same value; everything else
        // carries the old `content_seq` forward untouched, which is what
        // makes the pair survive the change feed's collapsing (an edit
        // followed by five drags still reports the edit's seq).
        let next_content_seq = if text_changed {
            seq
        } else {
            locked.get::<i64, _>("content_seq")
        };
        let row = sqlx::query(&format!(
            "UPDATE notes SET text = $2, color = $3, size = $4, font = $5,
                              starts_at = $6, ends_at = $7, place = $8, x = $9, y = $10,
                              board_seq = $11, content_seq = $12, updated_at = now()
             WHERE id = $1
             RETURNING {NOTE_COLS}"
        ))
        .bind(note_id)
        .bind(&next_text)
        .bind(&next_color)
        .bind(&next_size)
        .bind(&next_font)
        .bind(next_starts)
        .bind(next_ends)
        .bind(next_place.as_deref())
        .bind(next_x)
        .bind(next_y)
        .bind(seq)
        .bind(next_content_seq)
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

    let mut note = Note::from_row(&row);
    // A MOVE must not lose the picture. The frame IS the note, and a photo
    // note fanned out without its attachment would blank the picture on
    // every other device until the next full board read (protocol.md,
    // "Board").
    hydrate(&state, &mut note).await?;
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
    // A deleted photo note takes its picture with it (protocol.md,
    // "Board"): the tombstone carries no content, so nothing left could
    // ever show it. The ROW goes here, inside the transaction; the FILE
    // goes after the commit and only if no other row still names it —
    // since 0011 a family's identical uploads share one file.
    let orphaned: Option<String> = sqlx::query_scalar(
        "DELETE FROM attachments WHERE note_id = $1 RETURNING storage_key",
    )
    .bind(note_id)
    .fetch_optional(&mut *tx)
    .await?;
    tx.commit().await?;
    if let Some(storage_key) = orphaned {
        crate::handlers_attachment::remove_if_unreferenced(&state, &storage_key).await?;
    }

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

#[derive(Debug, Deserialize)]
pub struct RsvpRequest {
    pub answer: String,
}

/// `PUT /families/mine/board/notes/{id}/rsvp` — "I am coming", from anyone
/// in the family.
///
/// ANSWERING IS NOT AUTHORSHIP. An event is the family's, and a plan only
/// one person may record is not a plan — so this is the shared act, like
/// moving a note, and the author's own answer is not assumed either
/// (docs/protocol.md, "Board").
///
/// A state-set, not a toggle: re-sending the answer already held is a
/// no-op that burns no seq and fans nothing out, exactly like re-sending a
/// message's body or re-voting the same poll option.
pub async fn put_rsvp(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(note_id): Path<i64>,
    AppJson(req): AppJson<RsvpRequest>,
) -> Result<Response, ApiError> {
    let answer = req.answer.trim();
    if !Rsvp::ANSWERS.contains(&answer) {
        return Err(ApiError::bad_request(
            codes::INVALID_RSVP,
            format!("answer must be one of {}", Rsvp::ANSWERS.join(", ")),
        ));
    }
    set_rsvp(auth, state, note_id, Some(answer.to_string())).await
}

/// `DELETE /families/mine/board/notes/{id}/rsvp` — retract. Idempotent:
/// retracting nothing answers with the event unchanged and burns no seq.
pub async fn delete_rsvp(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(note_id): Path<i64>,
) -> Result<Response, ApiError> {
    set_rsvp(auth, state, note_id, None).await
}

async fn set_rsvp(
    auth: AuthUser,
    state: AppState,
    note_id: i64,
    answer: Option<String>,
) -> Result<Response, ApiError> {
    let family_id = caller_family(&state, auth.user_id).await?;

    let mut tx = state.pool.begin().await?;
    let locked = sqlx::query(
        "SELECT kind, deleted_at FROM notes WHERE id = $1 AND family_id = $2 FOR UPDATE",
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
        return Err(ApiError::not_found(
            codes::NOTE_NOT_FOUND,
            "no such note on this board",
        ));
    }
    // Only an event has anybody to come to it. Refused as `invalid_rsvp`
    // rather than `note_not_found`: the note is right there, and telling a
    // client "no such note" would send it looking for a sync bug.
    if locked.get::<String, _>("kind") != Note::KIND_EVENT {
        return Err(ApiError::bad_request(
            codes::INVALID_RSVP,
            "only an event can be answered",
        ));
    }

    let held: Option<String> =
        sqlx::query_scalar("SELECT answer FROM note_rsvps WHERE note_id = $1 AND user_id = $2")
            .bind(note_id)
            .bind(auth.user_id)
            .fetch_optional(&mut *tx)
            .await?;
    if held == answer {
        // Nothing happened. No seq, no fan-out — the same answer to the
        // same question is not a change to the board.
        let row = sqlx::query(&format!("SELECT {NOTE_COLS} FROM notes WHERE id = $1"))
            .bind(note_id)
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        let mut note = Note::from_row(&row);
        hydrate(&state, &mut note).await?;
        return Ok((StatusCode::OK, Json(json!({"note": note}))).into_response());
    }

    match &answer {
        Some(answer) => {
            sqlx::query(
                "INSERT INTO note_rsvps (note_id, user_id, answer) VALUES ($1, $2, $3)
                 ON CONFLICT (note_id, user_id)
                 DO UPDATE SET answer = EXCLUDED.answer, updated_at = now()",
            )
            .bind(note_id)
            .bind(auth.user_id)
            .bind(answer)
            .execute(&mut *tx)
            .await?;
        }
        None => {
            sqlx::query("DELETE FROM note_rsvps WHERE note_id = $1 AND user_id = $2")
                .bind(note_id)
                .bind(auth.user_id)
                .execute(&mut *tx)
                .await?;
        }
    }

    // A new seq, so the answer reaches the other devices through the one
    // feed — and `content_seq` untouched, because the event says exactly
    // what it said before: a badge claiming there is something to READ
    // would be a lie (protocol.md, "Board").
    let seq: i64 = sqlx::query_scalar("SELECT nextval('family_board_seq')")
        .fetch_one(&mut *tx)
        .await?;
    let row = sqlx::query(&format!(
        "UPDATE notes SET board_seq = $2, updated_at = now() WHERE id = $1
         RETURNING {NOTE_COLS}"
    ))
    .bind(note_id)
    .bind(seq)
    .fetch_one(&mut *tx)
    .await?;
    advance_family_seq(&mut tx, family_id, seq).await?;
    tx.commit().await?;

    let mut note = Note::from_row(&row);
    hydrate(&state, &mut note).await?;
    // Fanned out, never pushed: `deliver_board_note` is the no-push path
    // (only creation notifies), and nobody should be woken because
    // somebody else said they are coming.
    events::log_fanout_error(
        "board_note",
        events::deliver_board_note(&state, family_id, &note).await,
    );
    Ok((StatusCode::OK, Json(json!({"note": note}))).into_response())
}
