//! Profile pictures — the only binary surface in the protocol.
//!
//! Bytes live in `user_avatars` (see migration 0003) rather than on disk:
//! nothing in this server ever deletes a `users` row, so filesystem blobs
//! would have no natural GC trigger, and this way the family's backup
//! stays a single `pg_dump`.
//!
//! `users.avatar_version` is the cheap half that every user-shaped
//! response carries. It only ever goes up (and back to 0 on delete), so a
//! client can key its cache on `(user_id, avatar_version)` and never
//! revalidate — which is why the GET below answers `immutable`.
//!
//! The server validates and stores; it never transcodes. Clients downscale
//! before uploading, and the size ceiling here is the backstop.

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::json;
use sqlx::Row;

use crate::auth::AuthUser;
use crate::error::{ApiError, codes};
use crate::models::User;
use crate::state::AppState;

/// `PUT /me/avatar` — set or replace the caller's profile picture.
pub async fn put_avatar(
    auth: AuthUser,
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        // Tolerate parameters: "image/jpeg; charset=binary" is still JPEG.
        .map(|value| {
            value
                .split(';')
                .next()
                .unwrap_or(value)
                .trim()
                .to_ascii_lowercase()
        });

    let content_type = match content_type.as_deref() {
        Some("image/jpeg") | Some("image/jpg") => "image/jpeg",
        Some("image/png") => "image/png",
        _ => {
            return Err(ApiError::unsupported_media_type(
                codes::INVALID_IMAGE,
                "profile pictures must be image/jpeg or image/png",
            ));
        }
    };

    if body.len() > state.cfg.limits.max_avatar_bytes {
        return Err(ApiError::payload_too_large(
            codes::AVATAR_TOO_LARGE,
            format!(
                "profile picture must be at most {} bytes",
                state.cfg.limits.max_avatar_bytes
            ),
        ));
    }
    // Bytes must actually be the image they claim to be — the magic number
    // is the whole check. Decoding here would mean an image codec in the
    // server, and the clients already re-encode what they send.
    if !matches_magic(content_type, &body) {
        return Err(ApiError::bad_request(
            codes::INVALID_IMAGE,
            "body is not a readable JPEG or PNG",
        ));
    }

    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "INSERT INTO user_avatars (user_id, content_type, bytes, updated_at)
         VALUES ($1, $2, $3, now())
         ON CONFLICT (user_id) DO UPDATE
             SET content_type = EXCLUDED.content_type,
                 bytes = EXCLUDED.bytes,
                 updated_at = now()",
    )
    .bind(auth.user_id)
    .bind(content_type)
    .bind(body.as_ref())
    .execute(&mut *tx)
    .await?;
    // The version comes off `avatar_seq`, which only ever goes up —
    // including across a delete, which resets the *version* to 0 but
    // leaves the counter alone. That is what makes it safe for clients to
    // cache a picture forever under (user_id, avatar_version): a number
    // they already hold can never come back meaning a different picture.
    let row = sqlx::query(
        "UPDATE users SET avatar_seq = avatar_seq + 1, avatar_version = avatar_seq + 1
         WHERE id = $1
         RETURNING id, username, display_name, created_at, avatar_version,
                   birthday_month, birthday_day",
    )
    .bind(auth.user_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;

    let user = User::from_row(&row);
    Ok((StatusCode::OK, Json(json!({"user": user}))).into_response())
}

/// `DELETE /me/avatar` — drop the caller's picture. Idempotent.
pub async fn delete_avatar(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Response, ApiError> {
    let mut tx = state.pool.begin().await?;
    sqlx::query("DELETE FROM user_avatars WHERE user_id = $1")
        .bind(auth.user_id)
        .execute(&mut *tx)
        .await?;
    // Back to 0 — "no picture" — rather than another increment: 0 is the
    // sentinel every client checks before it bothers fetching.
    // `avatar_seq` is deliberately NOT reset, so the next upload lands
    // above every version anyone has already cached.
    sqlx::query("UPDATE users SET avatar_version = 0 WHERE id = $1")
        .bind(auth.user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `GET /users/{id}/avatar` — the bytes, for the user themselves or anyone
/// in their family.
pub async fn get_avatar(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(user_id): Path<i64>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    // One query decides both visibility and existence, and the same 404
    // answers "no such user", "not your family" and "no picture" — the
    // endpoint must not reveal which.
    let row = sqlx::query(
        "SELECT a.content_type, a.bytes, u.avatar_version
         FROM users u
         JOIN user_avatars a ON a.user_id = u.id
         WHERE u.id = $1
           AND (u.id = $2 OR (u.family_id IS NOT NULL AND u.family_id = $3))",
    )
    .bind(user_id)
    .bind(auth.user_id)
    .bind(auth.family_id)
    .fetch_optional(&state.pool)
    .await?;

    let Some(row) = row else {
        return Err(ApiError::not_found(
            codes::USER_NOT_FOUND,
            "no profile picture for that user",
        ));
    };

    let content_type: String = row.get("content_type");
    let version: i64 = row.get("avatar_version");
    let bytes: Vec<u8> = row.get("bytes");
    let etag = format!("\"{user_id}-{version}\"");

    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.split(',').any(|candidate| candidate.trim() == etag))
    {
        return Ok((StatusCode::NOT_MODIFIED, [(header::ETAG, etag)]).into_response());
    }

    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, content_type),
            // The URL carries ?v=<avatar_version>, so a hit is good forever.
            (
                header::CACHE_CONTROL,
                "private, max-age=31536000, immutable".to_string(),
            ),
            (header::ETAG, etag),
        ],
        bytes,
    )
        .into_response())
}

/// Whether the bytes open with the signature their content type promises.
fn matches_magic(content_type: &str, bytes: &[u8]) -> bool {
    match content_type {
        "image/jpeg" => bytes.starts_with(&[0xFF, 0xD8, 0xFF]),
        "image/png" => bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::matches_magic;

    #[test]
    fn magic_numbers_gate_the_content_type() {
        let jpeg = [0xFFu8, 0xD8, 0xFF, 0xE0, 0x00];
        let png = [0x89u8, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00];
        assert!(matches_magic("image/jpeg", &jpeg));
        assert!(matches_magic("image/png", &png));
        // Right shape, wrong type — a PNG announced as a JPEG is refused.
        assert!(!matches_magic("image/jpeg", &png));
        assert!(!matches_magic("image/png", &jpeg));
        // Not an image at all, and too short to even look like one.
        assert!(!matches_magic("image/jpeg", b"{\"not\":\"an image\"}"));
        assert!(!matches_magic("image/png", &[]));
        assert!(!matches_magic("image/gif", &jpeg));
    }
}
