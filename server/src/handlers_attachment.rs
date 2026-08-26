//! Photos and videos (docs/protocol.md, "Photos, videos and files").
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
use tokio::io::AsyncReadExt;
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
    /// Required for `kind=file`, ignored on a photo or video: a document's
    /// name is its whole identity (protocol.md, "Files"). Optional on audio
    /// and on a location, where it is a label somebody typed ("Home").
    pub name: Option<String>,
    /// `kind=location` only, and both required there. Degrees, WGS 84.
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    /// `kind=location` only, optional: the radius in metres the sending
    /// device believed its fix good to.
    pub accuracy_m: Option<i32>,
}

const ATTACHMENT_COLS: &str = "id, kind, mime, size_bytes, width, height, duration_ms, \
                               has_preview, name, latitude, longitude, accuracy_m";

/// The declared type must match the bytes. Same rule as avatars: a magic
/// number is the whole check, because deciding otherwise would mean an
/// image and video codec in the server.
fn matches_magic(mime: &str, head: &[u8]) -> bool {
    match mime {
        "image/jpeg" => head.starts_with(&[0xFF, 0xD8, 0xFF]),
        "image/png" => head.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]),
        // HEIC/HEIF and MP4/MOV are all ISO base media: "ftyp" at offset 4,
        // with the brand that follows telling them apart.
        // HEIC/HEIF, MP4/MOV and m4a/aac are all ISO base media: "ftyp" at
        // offset 4, with the brand that follows telling them apart.
        "image/heic" | "image/heif" | "video/mp4" | "video/quicktime" | "audio/mp4"
        | "audio/m4a" => head.len() >= 12 && &head[4..8] == b"ftyp",
        // An MP3 is either an ID3 tag or a raw frame sync (11 set bits).
        "audio/mpeg" => {
            head.starts_with(b"ID3")
                || (head.len() >= 2 && head[0] == 0xFF && (head[1] & 0xE0) == 0xE0)
        }
        "audio/wav" => head.len() >= 12 && head.starts_with(b"RIFF") && &head[8..12] == b"WAVE",
        "audio/ogg" => head.starts_with(b"OggS"),
        _ => false,
    }
}

/// Which family an upload belongs to — the dedup scope, and the membership
/// check every write endpoint makes.
///
/// An account with no family has nobody to send anything to. Without this,
/// a stranger who can register — which is the whole point of an open
/// self-hosted server — can fill the disk 100 MB at a time, and the sweeper
/// would not touch it for 24 hours. protocol.md has listed `not_in_family`
/// here all along.
async fn uploader_family(state: &AppState, user_id: i64) -> Result<i64, ApiError> {
    let family_id: Option<i64> = sqlx::query_scalar("SELECT family_id FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(&state.pool)
        .await?
        .flatten();
    family_id.ok_or_else(|| {
        ApiError::forbidden(
            codes::NOT_IN_FAMILY,
            "join a family before sending attachments",
        )
    })
}

/// `POST /attachments?kind=location` — the one upload with nothing to upload.
///
/// A location is three numbers, and they arrive in the query string like
/// every other piece of attachment metadata. The body is ignored: there are
/// no bytes, no file is written, and `GET /attachments/{id}` on the result
/// refuses rather than looking for one (migration 0016 carries the same
/// rules as constraints, so a future write path cannot forget them).
///
/// It is a separate function rather than a third arm of the branch below
/// because it shares almost nothing with the others — no media type, no
/// magic number, no streaming, no hash, no dedup. Threading a no-bytes case
/// through all of that would leave five `if is_location` guards in a
/// pipeline whose whole subject is bytes.
async fn upload_location(
    state: &AppState,
    user_id: i64,
    params: &UploadParams,
) -> Result<Response, ApiError> {
    let (Some(latitude), Some(longitude)) = (params.latitude, params.longitude) else {
        return Err(ApiError::bad_request(
            codes::INVALID_ATTACHMENT,
            "a location needs latitude and longitude",
        ));
    };
    // Checked here as well as in the CHECK constraint, so the caller gets
    // the protocol's error rather than a 500 from a violated constraint.
    // NaN fails every comparison, which is what refuses it.
    if !(-90.0..=90.0).contains(&latitude) || !(-180.0..=180.0).contains(&longitude) {
        return Err(ApiError::bad_request(
            codes::INVALID_ATTACHMENT,
            "latitude must be -90..=90 and longitude -180..=180",
        ));
    }
    if let Some(accuracy) = params.accuracy_m
        && accuracy < 0
    {
        return Err(ApiError::bad_request(
            codes::INVALID_ATTACHMENT,
            "accuracy_m cannot be negative",
        ));
    }
    let label = params
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty() && name.chars().count() <= Attachment::MAX_NAME_LEN);

    let family_id = uploader_family(state, user_id).await?;

    // A storage_key is still allocated, because the column is NOT NULL and
    // every shared path reads it. Nothing is ever written there, and
    // `Storage::remove` treats a missing file as normal — so the sweeper
    // and the retention pass need no special case at all.
    let storage_key = format!("{}-{}", user_id, crate::tokens::gen_session_token());
    let row = sqlx::query(&format!(
        "INSERT INTO attachments
            (uploader_id, kind, mime, size_bytes, storage_key, name, family_id,
             latitude, longitude, accuracy_m)
         VALUES ($1, $2, $3, 0, $4, $5, $6, $7, $8, $9)
         RETURNING {ATTACHMENT_COLS}"
    ))
    .bind(user_id)
    .bind(Attachment::KIND_LOCATION)
    .bind(Attachment::LOCATION_MIME)
    .bind(&storage_key)
    .bind(label)
    .bind(family_id)
    .bind(latitude)
    .bind(longitude)
    .bind(params.accuracy_m)
    .fetch_one(&state.pool)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({"attachment": Attachment::from_row(&row)})),
    )
        .into_response())
}

/// `POST /attachments` — stream a photo, video, piece of audio or file to
/// disk, or record a location, which has no bytes at all.
pub async fn upload_attachment(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(params): Query<UploadParams>,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, ApiError> {
    // Taken before anything reads a header or a byte: a location declares
    // itself in the query string and has neither.
    if params.kind.as_deref() == Some(Attachment::KIND_LOCATION) {
        return upload_location(&state, auth.user_id, &params).await;
    }

    let mime = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.split(';').next().unwrap_or(value).trim().to_string())
        .unwrap_or_default();

    // A FILE is whatever the sender says it is. The type is metadata, not
    // a claim the server verifies, and no list is consulted — a family
    // sending each other documents must never be told their file is not
    // allowed (protocol.md, "Files"). What makes that safe is `serve`
    // below, which hands a file back as an attachment that cannot render.
    let is_file = params.kind.as_deref() == Some(Attachment::KIND_FILE);
    let (kind, mime, name) = if is_file {
        let name =
            params.name.as_deref().map(str::trim).filter(|name| {
                !name.is_empty() && name.chars().count() <= Attachment::MAX_NAME_LEN
            });
        let Some(name) = name else {
            return Err(ApiError::bad_request(
                codes::INVALID_ATTACHMENT,
                format!(
                    "a file needs a name of 1..={} characters",
                    Attachment::MAX_NAME_LEN
                ),
            ));
        };
        let mime = if mime.is_empty() {
            Attachment::DEFAULT_FILE_MIME.to_string()
        } else {
            mime
        };
        (Attachment::KIND_FILE, mime, Some(name.to_string()))
    } else {
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
        (kind, mime, None)
    };

    // Which family the bytes are being uploaded INTO — the dedup scope.
    // Read now rather than derived later: the uploader can leave or move
    // family, and the file belongs to the family that received it.
    let family_id = uploader_family(&state, auth.user_id).await?;

    // The row is created first so its id names the file — one identifier,
    // no second allocation scheme to keep in step.
    let storage_key = format!("{}-{}", auth.user_id, crate::tokens::gen_session_token());
    let row = sqlx::query(&format!(
        "INSERT INTO attachments
            (uploader_id, kind, mime, size_bytes, width, height, duration_ms, storage_key, name,
             family_id)
         VALUES ($1, $2, $3, 0, $4, $5, $6, $7, $8, $9)
         RETURNING {ATTACHMENT_COLS}"
    ))
    .bind(auth.user_id)
    .bind(kind)
    .bind(&mime)
    .bind(params.width)
    .bind(params.height)
    .bind(params.duration_ms)
    .bind(&storage_key)
    .bind(name.as_deref())
    .bind(family_id)
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
    // Files skip it entirely — there is no type to contradict.
    let head = if is_file {
        Vec::new()
    } else {
        read_head(&path).await
    };
    if !is_file && !matches_magic(&mime, &head) {
        let _ = sqlx::query("DELETE FROM attachments WHERE id = $1")
            .bind(id)
            .execute(&state.pool)
            .await;
        // Safe to remove unconditionally: this key was minted for this
        // upload moments ago and dedup has not run, so nothing else can
        // reference it.
        state.storage.remove(&storage_key).await;
        return Err(ApiError::bad_request(
            codes::INVALID_ATTACHMENT,
            format!("body is not a readable {mime}"),
        ));
    }

    // Does this family already hold these exact bytes? If so, point this
    // row at the file that is already there and drop the copy just
    // written. Scoped to the family and matched on size as well as digest
    // — belt and braces against a hash collision costing somebody the
    // wrong photo.
    //
    // A concurrent pair of identical uploads can both miss here and keep
    // two copies. That is the acceptable outcome: a missed saving, never a
    // wrong or shared-too-far file.
    let existing: Option<(String, bool)> = {
        sqlx::query_as(
            "SELECT storage_key, has_preview FROM attachments
             WHERE family_id = $1 AND content_hash = $2 AND size_bytes = $3 AND id <> $4
             ORDER BY id
             LIMIT 1",
        )
        .bind(family_id)
        .bind(&written.sha256)
        .bind(written.bytes as i64)
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
    };

    let (final_key, inherited_preview) = match existing {
        Some((key, has_preview)) => {
            state.storage.discard(&path).await;
            // The preview belongs to bytes that are identical, so it is
            // this attachment's preview too — the sender's client will
            // upload its own anyway, but the bubble can draw before it.
            (key, has_preview)
        }
        None => (storage_key.clone(), false),
    };

    let row = sqlx::query(&format!(
        "UPDATE attachments
         SET size_bytes = $2, content_hash = $3, storage_key = $4, has_preview = $5
         WHERE id = $1
         RETURNING {ATTACHMENT_COLS}"
    ))
    .bind(id)
    .bind(written.bytes as i64)
    .bind(&written.sha256)
    .bind(&final_key)
    .bind(inherited_preview)
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
    let row =
        sqlx::query("SELECT storage_key, kind FROM attachments WHERE id = $1 AND uploader_id = $2")
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
    let kind: String = row.get("kind");
    if kind == Attachment::KIND_FILE
        || kind == Attachment::KIND_AUDIO
        || kind == Attachment::KIND_LOCATION
    {
        // Nothing draws a file, a piece of audio or a location as a
        // picture, so a preview on one is a client bug worth reporting
        // rather than silently storing. Audio gets a play control and a
        // duration; a waveform is deliberately not part of the wire. A
        // location is drawn from its coordinates by each device, which is
        // what makes a stored map image the wrong artefact — it would be
        // one sender's idea of zoom, frozen (protocol.md).
        return Err(ApiError::bad_request(
            codes::INVALID_ATTACHMENT,
            format!("a {kind} has no preview"),
        ));
    }
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
        "SELECT a.storage_key, a.mime, a.has_preview, a.kind, a.name
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
    // A location has no file, and never had one — every field it has was
    // already delivered with the message. Refused explicitly rather than
    // left to fall through to the open() below, which would report a
    // missing file as an internal error and put a 500 in the log for a
    // client doing something merely pointless.
    if row.get::<String, _>("kind") == Attachment::KIND_LOCATION {
        return Err(ApiError::bad_request(
            codes::INVALID_ATTACHMENT,
            "a location has no bytes; it is carried on the attachment itself",
        ));
    }
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
    // A FILE is served so that it cannot be anything but a download.
    //
    // This is the other half of accepting any type at all: without it, a
    // member could upload HTML or SVG and hand the family a link that runs
    // script from the family server's own origin. `attachment` disposition
    // stops it rendering, `nosniff` stops a browser deciding for itself
    // that the bytes look like HTML, and the filename is sanitised because
    // a header is a line and a newline in a name would let the uploader
    // write headers of their own.
    let disposition =
        (!preview && row.get::<String, _>("kind") == Attachment::KIND_FILE).then(|| {
            let name: Option<String> = row.get("name");
            let name = name.unwrap_or_else(|| "download".to_string());
            let ascii = Attachment::ascii_filename(&name);
            // RFC 5987: the ASCII form for old clients, filename* for the
            // real name, which is usually the one anybody sees.
            let encoded = percent_encode(&name);
            format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{encoded}")
        });

    // A video player asks for byte ranges — that is how seeking works, and
    // how playback starts before the whole file has arrived. Answering the
    // full body to every request would make a scrub through a 90 MB clip
    // re-download it from the beginning.
    let range = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok())
        .map_or(RangeSpec::Whole, |value| parse_range(value, len));
    let (status, start, count) = match range {
        RangeSpec::Whole => (StatusCode::OK, 0, len),
        RangeSpec::Bytes(start, end) => (StatusCode::PARTIAL_CONTENT, start, end - start + 1),
        RangeSpec::Unsatisfiable => {
            // A range this file cannot satisfy. 416 carries the real size
            // so the player can ask again correctly.
            return Ok((
                StatusCode::RANGE_NOT_SATISFIABLE,
                [
                    (header::CONTENT_RANGE, format!("bytes */{len}")),
                    (header::ACCEPT_RANGES, "bytes".to_string()),
                ],
            )
                .into_response());
        }
    };

    let mut file = file;
    if start > 0 {
        use tokio::io::AsyncSeekExt;
        file.seek(std::io::SeekFrom::Start(start))
            .await
            .map_err(|err| ApiError::Internal(anyhow::anyhow!("seeking {path:?}: {err}")))?;
    }
    let stream = ReaderStream::new(file.take(count));

    let mut response = (
        status,
        [
            (header::CONTENT_TYPE, content_type),
            (header::CONTENT_LENGTH, count.to_string()),
            (header::ETAG, etag),
            (
                header::CACHE_CONTROL,
                "private, max-age=31536000, immutable".to_string(),
            ),
            (header::ACCEPT_RANGES, "bytes".to_string()),
        ],
        Body::from_stream(stream),
    )
        .into_response();
    if let Some(disposition) = disposition {
        let headers = response.headers_mut();
        if let Ok(value) = disposition.parse() {
            headers.insert(header::CONTENT_DISPOSITION, value);
        }
        headers.insert(
            header::X_CONTENT_TYPE_OPTIONS,
            "nosniff".parse().expect("ASCII"),
        );
    }
    if status == StatusCode::PARTIAL_CONTENT {
        let end = start + count - 1;
        response.headers_mut().insert(
            header::CONTENT_RANGE,
            format!("bytes {start}-{end}/{len}").parse().expect("ASCII"),
        );
    }
    Ok(response)
}

/// Percent-encode a filename for RFC 5987's `filename*` form.
///
/// Hand-rolled rather than pulling in a crate: the attr-char set is small,
/// and everything outside it — including every byte of a non-ASCII name —
/// becomes %XX, which is exactly what makes the value safe to put in a
/// header no matter what the uploader called their file.
fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        let c = *byte as char;
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '~') {
            out.push(c);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// What a `Range` header asks of a file of `len` bytes.
enum RangeSpec {
    /// No range, or one to ignore — send the whole body with a 200.
    Whole,
    /// An inclusive byte range: 206 with a Content-Range.
    Bytes(u64, u64),
    /// A range this file cannot satisfy: 416.
    Unsatisfiable,
}

/// Parse one `Range` header.
///
/// Anything not understood — a unit other than bytes, several ranges, a
/// malformed value — is [`RangeSpec::Whole`] rather than an error: RFC 9110
/// says to ignore such a header and send the whole body, which is also the
/// safest answer for a player that asked for something exotic. Multiple
/// ranges would mean a multipart body no media player asks for. Only a
/// well-formed range that lies outside the file is refused.
fn parse_range(value: &str, len: u64) -> RangeSpec {
    let Some(spec) = value.trim().strip_prefix("bytes=").map(str::trim) else {
        return RangeSpec::Whole;
    };
    if spec.contains(',') {
        return RangeSpec::Whole;
    }
    let Some((first, last)) = spec.split_once('-') else {
        return RangeSpec::Whole;
    };
    // An empty file can satisfy no range at all.
    if len == 0 {
        return RangeSpec::Unsatisfiable;
    }
    let parsed = match (first.trim(), last.trim()) {
        ("", "") => return RangeSpec::Whole,
        // "bytes=-500": the LAST 500 bytes, not "from 0 to 500".
        ("", suffix) => suffix
            .parse::<u64>()
            .ok()
            .map(|wanted| (len.saturating_sub(wanted), len - 1, wanted > 0)),
        (first, "") => first
            .parse::<u64>()
            .ok()
            .map(|start| (start, len - 1, true)),
        (first, last) => match (first.parse::<u64>(), last.parse::<u64>()) {
            (Ok(start), Ok(end)) => Some((start, end.min(len - 1), true)),
            _ => None,
        },
    };
    match parsed {
        // Unparseable numbers are a malformed header, not a refusal.
        None => RangeSpec::Whole,
        Some((start, end, wanted)) if wanted && start <= end && start < len => {
            RangeSpec::Bytes(start, end)
        }
        Some(_) => RangeSpec::Unsatisfiable,
    }
}

/// Remove a file only once no attachment row names it any more.
///
/// The check and the delete are not atomic, and deliberately so: taking a
/// lock across a filesystem operation for every swept row would be a lot of
/// machinery for a race that needs an upload to dedup against a row being
/// deleted in the same instant. The failure mode of losing that race is a
/// file with no rows (the sweeper's own problem, and a `du` away from being
/// noticed) rather than a row with no file.
pub(crate) async fn remove_if_unreferenced(
    state: &AppState,
    storage_key: &str,
) -> Result<(), ApiError> {
    let still_used: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM attachments WHERE storage_key = $1)")
            .bind(storage_key)
            .fetch_one(&state.pool)
            .await?;
    if !still_used {
        state.storage.remove(storage_key).await;
    }
    Ok(())
}

/// Remove every file in a collected key list that no row names any more.
///
/// The shape every bulk delete in this server follows: collect the keys
/// BEFORE the rows go (afterwards nothing can name the files), delete, and
/// sweep AFTER the commit — never before, because a rollback would leave
/// rows pointing at bytes that are gone. Each key is still checked one at a
/// time rather than removed outright: since 0011 several rows may name one
/// file.
pub(crate) async fn remove_all_if_unreferenced(
    state: &AppState,
    storage_keys: &[String],
) -> Result<(), ApiError> {
    for key in storage_keys {
        remove_if_unreferenced(state, key).await?;
    }
    Ok(())
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

    // REFCOUNTED, and this is the part that must never regress: since
    // 0011 a family's identical uploads share one file, so removing it
    // with the first row that goes would break every other message
    // pointing at it — silently, and with the database still looking
    // perfectly consistent.
    for row in &rows {
        let key: String = row.get("storage_key");
        remove_if_unreferenced(state, &key).await?;
    }
    Ok(rows.len() as u64)
}
