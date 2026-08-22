//! Photos and videos (docs/protocol.md, "Photos and videos").
//!
//! Uploading is a separate request from sending: the bytes go up on their
//! own, and the message that follows names the attachment by id. A 100 MB
//! video and a 30-byte message have nothing in common, and coupling them
//! would put the whole upload inside the send retry with nowhere to show
//! progress.
//!
//! THE SERVER NEVER DECODES AN IMAGE OR A VIDEO. It checks the declared
//! type against the file's magic number and stores what it is given, the
//! same rule avatars follow — which is why the preview (the downscaled
//! photo, or a video's poster frame) is produced and uploaded by the
//! client.
//!
//! Bytes stream to disk in both directions and are never held whole in
//! memory. See `storage.rs`.

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;
use sqlx::Row;
use tokio_util::io::ReaderStream;

use crate::auth::AuthUser;
use crate::error::{ApiError, codes};
use crate::models::Attachment;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct UploadParams {
    /// Advisory: the server derives the kind from the media type and only
    /// uses this to reject an obvious mismatch.
    pub kind: Option<String>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub duration_ms: Option<i32>,
}

const ATTACHMENT_COLS: &str = "id, kind, mime, size_bytes, width, height, duration_ms, has_preview";

/// The declared type must match the bytes. Same rule as avatars: a magic
/// number is the whole check, because deciding otherwise would mean an
/// image and video codec in the server.
fn matches_magic(mime: &str, head: &[u8]) -> bool {
    match mime {
        "image/jpeg" => head.starts_with(&[0xFF, 0xD8, 0xFF]),
        "image/png" => head.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]),
        // HEIC/HEIF and MP4/MOV are all ISO base media: "ftyp" at offset 4,
        // with the brand that follows telling them apart.
        "image/heic" | "image/heif" | "video/mp4" | "video/quicktime" => {
            head.len() >= 12 && &head[4..8] == b"ftyp"
        }
        _ => false,
    }
}

/// `POST /attachments` — stream a photo or video to disk.
pub async fn upload_attachment(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(params): Query<UploadParams>,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
    let mime = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.split(';').next().unwrap_or(value).trim().to_string())
        .unwrap_or_default();

    let Some(kind) = Attachment::kind_for(&mime) else {
        return Err(ApiError::unsupported_media_type(
            codes::INVALID_ATTACHMENT,
            format!("unsupported media type {mime:?}"),
        ));
    };
    if let Some(declared) = params.kind.as_deref()
        && declared != kind
    {
        return Err(ApiError::bad_request(
            codes::INVALID_ATTACHMENT,
            format!("{mime} is a {kind}, not a {declared}"),
        ));
    }

    // The row is created first so its id names the file — one identifier,
    // no second allocation scheme to keep in step.
    let storage_key = format!("{}-{}", auth.user_id, crate::tokens::gen_session_token());
    let row = sqlx::query(&format!(
        "INSERT INTO attachments
            (uploader_id, kind, mime, size_bytes, width, height, duration_ms, storage_key)
         VALUES ($1, $2, $3, 0, $4, $5, $6, $7)
         RETURNING {ATTACHMENT_COLS}"
    ))
    .bind(auth.user_id)
    .bind(kind)
    .bind(&mime)
    .bind(params.width)
    .bind(params.height)
    .bind(params.duration_ms)
    .bind(&storage_key)
    .fetch_one(&state.pool)
    .await?;
    let id: i64 = row.get("id");

    let path = state.storage.blob_path(&storage_key);
    let written = match state
        .storage
        .write_stream(
            &path,
            body.into_data_stream(),
            state.cfg.limits.max_attachment_bytes,
        )
        .await
    {
        Ok(written) => written,
        Err(err) => {
            // The row exists but has no bytes: drop it rather than leave a
            // zero-length attachment somebody can attach to a message.
            let _ = sqlx::query("DELETE FROM attachments WHERE id = $1")
                .bind(id)
                .execute(&state.pool)
                .await;
            return Err(err);
        }
    };

    // Magic check AFTER the write: the head is on disk by then and reading
    // 12 bytes back is cheaper than buffering the stream to inspect it.
    let head = read_head(&path).await;
    if !matches_magic(&mime, &head) {
        let _ = sqlx::query("DELETE FROM attachments WHERE id = $1")
            .bind(id)
            .execute(&state.pool)
            .await;
        state.storage.remove(&storage_key).await;
        return Err(ApiError::bad_request(
            codes::INVALID_ATTACHMENT,
            format!("body is not a readable {mime}"),
        ));
    }

    let row = sqlx::query(&format!(
        "UPDATE attachments SET size_bytes = $2 WHERE id = $1 RETURNING {ATTACHMENT_COLS}"
    ))
    .bind(id)
    .bind(written as i64)
    .fetch_one(&state.pool)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({"attachment": Attachment::from_row(&row)})),
    )
        .into_response())
}

async fn read_head(path: &std::path::Path) -> Vec<u8> {
    use tokio::io::AsyncReadExt;
    let Ok(mut file) = tokio::fs::File::open(path).await else {
        return Vec::new();
    };
    let mut head = vec![0u8; 12];
    match file.read(&mut head).await {
        Ok(read) => {
            head.truncate(read);
            head
        }
        Err(_) => Vec::new(),
    }
}

/// `PUT /attachments/{id}/preview` — the downscaled photo or poster frame.
pub async fn upload_preview(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
    let mime = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if !mime.starts_with("image/jpeg") {
        return Err(ApiError::unsupported_media_type(
            codes::INVALID_ATTACHMENT,
            "a preview must be image/jpeg",
        ));
    }

    // Uploader only, and only before the row is claimed or after — either
    // way it is theirs; a member who did not upload it has no business
    // replacing what a bubble draws.
    let row = sqlx::query("SELECT storage_key FROM attachments WHERE id = $1 AND uploader_id = $2")
        .bind(id)
        .bind(auth.user_id)
        .fetch_optional(&state.pool)
        .await?;
    let Some(row) = row else {
        return Err(ApiError::not_found(
            codes::ATTACHMENT_NOT_FOUND,
            "no such attachment",
        ));
    };
    let storage_key: String = row.get("storage_key");

    let path = state.storage.preview_path(&storage_key);
    state
        .storage
        .write_stream(
            &path,
            body.into_data_stream(),
            state.cfg.limits.max_preview_bytes,
        )
        .await?;

    sqlx::query("UPDATE attachments SET has_preview = true WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `GET /attachments/{id}` and `/preview` — stream the bytes back.
///
/// Readable by the uploader always, and by every member of the chat once a
/// message claims it. Anyone else gets the same 404 a nonexistent id does,
/// so the endpoint never confirms that an attachment exists.
pub async fn get_attachment(
    auth: AuthUser,
    state: State<AppState>,
    id: Path<i64>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    serve(auth, state, id, headers, false).await
}

pub async fn get_preview(
    auth: AuthUser,
    state: State<AppState>,
    id: Path<i64>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    serve(auth, state, id, headers, true).await
}

async fn serve(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<i64>,
    headers: HeaderMap,
    preview: bool,
) -> Result<Response, ApiError> {
    let row = sqlx::query(
        "SELECT a.storage_key, a.mime, a.has_preview
         FROM attachments a
         LEFT JOIN messages m ON m.id = a.message_id
         WHERE a.id = $1
           AND (a.uploader_id = $2
                OR EXISTS (SELECT 1 FROM chat_members cm
                           WHERE cm.chat_id = m.chat_id AND cm.user_id = $2))",
    )
    .bind(id)
    .bind(auth.user_id)
    .fetch_optional(&state.pool)
    .await?;
    let Some(row) = row else {
        return Err(ApiError::not_found(
            codes::ATTACHMENT_NOT_FOUND,
            "no such attachment",
        ));
    };
    let storage_key: String = row.get("storage_key");
    let mime: String = row.get("mime");
    if preview && !row.get::<bool, _>("has_preview") {
        return Err(ApiError::not_found(
            codes::ATTACHMENT_NOT_FOUND,
            "no preview for this attachment",
        ));
    }

    let path = if preview {
        state.storage.preview_path(&storage_key)
    } else {
        state.storage.blob_path(&storage_key)
    };
    let file = tokio::fs::File::open(&path).await.map_err(|err| {
        // The row says it exists and the file does not: a partial delete,
        // or a restore that missed the attachments directory.
        ApiError::Internal(anyhow::anyhow!("opening {path:?}: {err}"))
    })?;
    let len = file.metadata().await.map(|meta| meta.len()).unwrap_or(0);

    // Content is immutable: an attachment's bytes never change, so the id
    // alone is a complete cache key.
    let etag = format!("\"{id}\"");
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.split(',').any(|candidate| candidate.trim() == etag))
    {
        return Ok((StatusCode::NOT_MODIFIED, [(header::ETAG, etag)]).into_response());
    }

    let content_type = if preview {
        "image/jpeg".to_string()
    } else {
        mime
    };
    let stream = ReaderStream::new(file);
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, content_type),
            (header::CONTENT_LENGTH, len.to_string()),
            (header::ETAG, etag),
            (
                header::CACHE_CONTROL,
                "private, max-age=31536000, immutable".to_string(),
            ),
            (header::ACCEPT_RANGES, "bytes".to_string()),
        ],
        Body::from_stream(stream),
    )
        .into_response())
}

/// Delete unclaimed uploads past the grace period.
///
/// A send the user abandoned — picked a video, changed their mind — leaves
/// a row and 100 MB with no message pointing at it. Nothing else in the
/// system would ever remove them.
pub async fn sweep_unclaimed(state: &AppState) -> Result<u64, ApiError> {
    let hours = state.cfg.limits.attachment_grace_hours;
    let rows = sqlx::query(
        "DELETE FROM attachments
         WHERE message_id IS NULL
           AND created_at < now() - make_interval(hours => $1)
         RETURNING storage_key",
    )
    .bind(hours as i32)
    .fetch_all(&state.pool)
    .await?;

    for row in &rows {
        let key: String = row.get("storage_key");
        state.storage.remove(&key).await;
    }
    Ok(rows.len() as u64)
}
