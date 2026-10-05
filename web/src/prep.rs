//! Turning what somebody picked, dropped or pasted into something worth
//! uploading — the browser's half of ios `Core/MediaPrep.swift`.
//!
//! A PHOTO is decoded with its EXIF orientation applied, drawn onto a canvas
//! no bigger than 2048 on its longest edge over white (a transparent PNG has
//! no black background waiting for it on a phone), and re-encoded as JPEG at
//! 0.85 — which also leaves every piece of EXIF behind, the place it was
//! taken included. Its preview is the same at 600 and 0.7. A photo this
//! browser cannot decode — a HEIC outside Safari — is REFUSED, as the Mac
//! refuses one it cannot read: its original bytes would carry every piece
//! of EXIF the re-encode exists to leave behind, GPS first among them.
//!
//! A VIDEO is brought to the protocol's profile — H.264 and AAC in an MP4,
//! 720 on its short side, 30 frames a second — unless it is within it
//! already, in which case it goes untouched (docs/protocol.md, "Preparing
//! media before upload"). What it becomes is fc_text::media_plan's decision,
//! the same one every other client makes; making it is crate::encode's job,
//! with the browser's own codecs. Where this browser cannot — no encoder, a
//! codec it does not decode, a failure half way — the file goes exactly as
//! it went before there was a profile: untouched within the ceiling, refused
//! over it (rule C). Either way it carries its size, length and a poster
//! frame read by the browser's own player. A transcode can take minutes,
//! so whoever asks for a file to be prepared holds a [`Job`] it can stop:
//! a stopped preparation is `Cancelled`, and sends nothing at all.
//!
//! AUDIO follows the audio rules the same way: uncompressed and lossless
//! sound, Ogg, and MP3 or AAC above 192 kbit/s become M4A; everything else
//! goes untouched. FILES go untouched; what kind each is, and what it is
//! called, are fc_text::media's rules.
//!
//! A STICKER is none of the above, and above all not a photo
//! (docs/protocol.md, "A sticker is NOT prepared before upload"): a JPEG has
//! no transparency and one frame, which is exactly what makes a sticker one.
//! A WebP or PNG that is already a sticker goes up BYTE FOR BYTE — animated
//! or not, whatever its pixel size — and only a still picture that is not
//! one yet is made into one: fitted whole into 512 × 512 on a canvas that
//! is left TRANSPARENT, and written as PNG (or WebP, where this browser can
//! write one and the PNG is over the pack's ceiling). Neither has a preview.
//! Which of the two a picked file is, is fc_text::pack's decision.
//!
//! A PROFILE PICTURE is its own thing (ios `Core/AvatarImage.swift`): the
//! largest centred square, at most 512 across, over white, as a JPEG stepped
//! down in quality until it fits the byte budget fc_text::avatar keeps.

use fc_text::i18n::t;
use std::cell::RefCell;
use std::rc::Rc;

use fc_text::avatar;
use fc_text::media::{self, Route};
use fc_text::media_plan::{self, AudioPlan, Upload};
use fc_text::pack;
use futures::channel::oneshot;
use futures::future::{select, Either};
use gloo_timers::future::TimeoutFuture;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    Blob, CanvasRenderingContext2d, EventTarget, File, HtmlCanvasElement, HtmlMediaElement,
    HtmlVideoElement, ImageBitmap, ImageBitmapOptions, ImageOrientation, Url,
};

pub use crate::encode::Job;
use crate::encode::{self, Planned};
use crate::staged::Prepared;

/// Why something could not be staged — each with the Mac's sentence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrepError {
    TooLarge,
    Unreadable,
    /// Offered to the board, which pins photos and nothing else.
    NotAPhoto,
    /// Its [`Job`] was stopped while it was being prepared. Not a failure,
    /// and above all not rule C: whoever stopped it wants NOTHING sent, not
    /// the original in place of the transcode they called off.
    Cancelled,
}

impl PrepError {
    pub fn message(self) -> &'static str {
        match self {
            PrepError::TooLarge => t("That file is over the 100 MB limit."),
            PrepError::Unreadable => t("Couldn't read that file."),
            PrepError::NotAPhoto => t("The board pins photos only."),
            // Nothing to say: the person who cancelled knows they did.
            PrepError::Cancelled => "",
        }
    }
}

/// Prepare one picked, dropped or pasted file. `job` is how the caller
/// stops it and hears how far it has got — a video outside the profile is
/// transcoded here, which for a long clip is minutes.
pub async fn prepare(file: &File, job: &Job) -> Result<Prepared, PrepError> {
    let name = file.name();
    let head = head(file, 12).await;
    match media::route(&file.type_(), &name, &head) {
        Route::Photo => photo(file).await,
        Route::Video => video(file, job).await,
        Route::Audio(mime) => audio(file, mime, job).await,
        // AIFF and FLAC are not types the server takes as audio, so the
        // router calls them files — and the audio rules say to re-encode
        // them into one it does.
        Route::File => match media::unaccepted_audio(&file.type_(), &name, &head) {
            Some(container) => audio(file, container, job).await,
            None => as_file(file),
        },
    }
}

/// Prepare a picture for the family board — a PHOTO or nothing. A wall pins
/// pictures (docs/protocol.md, "Board"): what the message rules would send
/// as a video, a voice note or a file is refused, and so is a photo this
/// browser cannot decode, exactly as a message's is.
pub async fn prepare_photo(file: &File) -> Result<Prepared, PrepError> {
    let head = head(file, 12).await;
    match media::route(&file.type_(), &file.name(), &head) {
        Route::Photo => photo(file).await,
        _ => Err(PrepError::NotAPhoto),
    }
}

fn within_limit(blob: &Blob) -> Result<(), PrepError> {
    if blob.size() as u64 > media::SIZE_LIMIT {
        Err(PrepError::TooLarge)
    } else {
        Ok(())
    }
}

/// Whether these bytes can still be read — false for a picked file that has
/// since been moved, changed or deleted.
pub async fn readable(blob: &Blob) -> bool {
    let Ok(slice) = blob.slice_with_i32_and_i32(0, 1) else {
        return false;
    };
    JsFuture::from(slice.array_buffer()).await.is_ok()
}

/// The first `count` bytes — what the server's magic-number check reads.
pub async fn head(blob: &Blob, count: i32) -> Vec<u8> {
    let Ok(slice) = blob.slice_with_i32_and_i32(0, count) else {
        return Vec::new();
    };
    match JsFuture::from(slice.array_buffer()).await {
        Ok(buffer) => js_sys::Uint8Array::new(&buffer).to_vec(),
        Err(_) => Vec::new(),
    }
}

/// A file sent as it is: its type is metadata, its name its identity.
fn as_file(file: &File) -> Result<Prepared, PrepError> {
    within_limit(file)?;
    let name = file.name();
    Ok(Prepared {
        kind: "file".into(),
        mime: media::declared_type(&file.type_(), &name),
        size: file.size() as i64,
        name: Some(media::sanitized_name(&name).unwrap_or_else(|| "file".into())),
        file: Some(file.clone().into()),
        ..Prepared::default()
    })
}

async fn photo(file: &File) -> Result<Prepared, PrepError> {
    let bitmap = decode(file).await.ok_or(PrepError::Unreadable)?;
    let (width, height) = media::fit_within(bitmap.width(), bitmap.height(), media::PHOTO_EDGE);
    let jpeg = draw_jpeg(
        &Source::Bitmap(&bitmap),
        width,
        height,
        media::PHOTO_QUALITY,
    )
    .await
    .ok_or(PrepError::Unreadable)?;
    // A downscaled photo is far under any ceiling, but a pathological one
    // is better refused here than by the server.
    within_limit(&jpeg)?;
    let (preview_width, preview_height) =
        media::fit_within(bitmap.width(), bitmap.height(), media::PREVIEW_EDGE);
    let preview = draw_jpeg(
        &Source::Bitmap(&bitmap),
        preview_width,
        preview_height,
        media::PREVIEW_QUALITY,
    )
    .await;
    bitmap.close();
    Ok(Prepared {
        kind: "photo".into(),
        mime: "image/jpeg".into(),
        size: jpeg.size() as i64,
        width: Some(i64::from(width)),
        height: Some(i64::from(height)),
        file: Some(jpeg),
        preview,
        ..Prepared::default()
    })
}

/// How much of a picked file is read to decide what it is: the magic
/// number, a WebP's animation flag, and a PNG's chunks up to its first
/// `IDAT` — past any ordinary metadata (fc_text::pack::is_animated).
const STICKER_HEAD: i32 = 64 * 1024;

/// How much of a picked GIF is read to find out whether it MOVES. A GIF
/// says so only by holding a second picture, which may be anywhere behind
/// the first, so it is read whole — up to this, past which the looping
/// extension in front of the first picture is taken at its word
/// (fc_text::pack::is_animated).
const GIF_READ: i32 = 32 * 1024 * 1024;

/// A picture for the family's sticker pack (docs/protocol.md, "What a
/// sticker is made of"). `max_bytes` is the server's `max_pack_item_bytes`.
///
/// NEVER through [`photo`]: that path draws over white and writes a JPEG.
/// What comes back is `kind=photo` with the type the BYTES are, the
/// original `Blob` itself when the file was a sticker already, and no
/// preview — a preview is a JPEG, the same destruction by another door.
pub async fn sticker(file: &Blob, max_bytes: u64) -> Result<Prepared, pack::Refusal> {
    let mut head = head(file, STICKER_HEAD).await;
    let size = file.size() as u64;
    if pack::is_gif(&head) && size > head.len() as u64 {
        head = self::head(file, GIF_READ).await;
    }
    match pack::plan(&head, size, max_bytes)? {
        pack::Plan::AsGiven(mime) => {
            // Decoded only to learn its size in pixels, and to find out NOW
            // that it is a picture at all: the server checks twelve bytes,
            // and a file cut short would otherwise become an empty square
            // in everybody's panel. The bitmap is thrown away; what goes up
            // is the file.
            let bitmap = decode(file).await.ok_or(pack::Refusal::Unreadable)?;
            let (width, height) = (bitmap.width(), bitmap.height());
            bitmap.close();
            Ok(Prepared {
                kind: "photo".into(),
                mime: mime.into(),
                size: size as i64,
                width: Some(i64::from(width)),
                height: Some(i64::from(height)),
                file: Some(file.clone()),
                ..Prepared::default()
            })
        }
        pack::Plan::Remake => {
            let bitmap = decode(file).await.ok_or(pack::Refusal::Unreadable)?;
            let (width, height) = pack::fit(bitmap.width(), bitmap.height());
            let made = remade_sticker(&bitmap, width, height, max_bytes).await;
            bitmap.close();
            let (blob, mime) = made?;
            Ok(Prepared {
                kind: "photo".into(),
                mime: mime.into(),
                size: blob.size() as i64,
                width: Some(i64::from(width)),
                height: Some(i64::from(height)),
                file: Some(blob),
                ..Prepared::default()
            })
        }
    }
}

/// `bitmap` drawn at `width`×`height` on a canvas with NOTHING under it, so
/// what was transparent stays transparent, and written as a PNG — or, when
/// that is over the ceiling, as the first WebP that fits, where this
/// browser writes WebP at all. One that asks for WebP and is handed a PNG
/// (Safari) has said it cannot, and the ladder stops there.
async fn remade_sticker(
    bitmap: &ImageBitmap,
    width: u32,
    height: u32,
    max_bytes: u64,
) -> Result<(Blob, &'static str), pack::Refusal> {
    let unreadable = pack::Refusal::Unreadable;
    let document = web_sys::window()
        .and_then(|window| window.document())
        .ok_or(unreadable)?;
    let canvas: HtmlCanvasElement = document
        .create_element("canvas")
        .ok()
        .and_then(|canvas| canvas.dyn_into().ok())
        .ok_or(unreadable)?;
    canvas.set_width(width);
    canvas.set_height(height);
    let context: CanvasRenderingContext2d = canvas
        .get_context("2d")
        .ok()
        .flatten()
        .and_then(|context| context.dyn_into().ok())
        .ok_or(unreadable)?;
    let _ = js_sys::Reflect::set(
        &context,
        &JsValue::from_str("imageSmoothingQuality"),
        &JsValue::from_str("high"),
    );
    context
        .draw_image_with_image_bitmap_and_dw_and_dh(
            bitmap,
            0.0,
            0.0,
            f64::from(width),
            f64::from(height),
        )
        .map_err(|_| unreadable)?;
    let png = to_blob(&canvas, pack::PNG, 1.0).await.ok_or(unreadable)?;
    if png.size() as u64 <= max_bytes {
        return Ok((png, pack::PNG));
    }
    for quality in pack::WEBP_QUALITIES {
        let Some(webp) = to_blob(&canvas, pack::WEBP, quality).await else {
            break;
        };
        if webp.type_() != pack::WEBP {
            break;
        }
        if webp.size() as u64 <= max_bytes {
            return Ok((webp, pack::WEBP));
        }
    }
    Err(pack::Refusal::TooLarge)
}

/// Whether two blobs hold the same bytes — how a sticker in a chat is
/// known to be one the pack already holds (docs/protocol.md, "Tapping one
/// shows it larger"): nothing on the wire names the item a message was
/// sent from, so the bytes are all there is to go by. Blobs that cannot be
/// read are not the same.
pub async fn same_bytes(one: &Blob, other: &Blob) -> bool {
    if one.size() != other.size() {
        return false;
    }
    let read = |blob: &Blob| JsFuture::from(blob.array_buffer());
    let (Ok(one), Ok(other)) = (read(one).await, read(other).await) else {
        return false;
    };
    js_sys::Uint8Array::new(&one).to_vec() == js_sys::Uint8Array::new(&other).to_vec()
}

/// A profile picture made from `file`, ready for `PUT /me/avatar`.
pub async fn avatar(file: &File) -> Result<Blob, PrepError> {
    let bitmap = decode(file).await.ok_or(PrepError::Unreadable)?;
    let jpeg = avatar_jpeg(&bitmap).await;
    bitmap.close();
    jpeg.ok_or(PrepError::Unreadable)
}

async fn avatar_jpeg(bitmap: &ImageBitmap) -> Option<Blob> {
    let square = avatar::square(bitmap.width(), bitmap.height())?;
    let document = web_sys::window()?.document()?;
    let canvas: HtmlCanvasElement = document.create_element("canvas").ok()?.dyn_into().ok()?;
    canvas.set_width(square.edge);
    canvas.set_height(square.edge);
    let context: CanvasRenderingContext2d = canvas.get_context("2d").ok()??.dyn_into().ok()?;
    let edge = f64::from(square.edge);
    context.set_fill_style_str("#ffffff");
    context.fill_rect(0.0, 0.0, edge, edge);
    let _ = js_sys::Reflect::set(
        &context,
        &JsValue::from_str("imageSmoothingQuality"),
        &JsValue::from_str("high"),
    );
    let side = f64::from(square.side);
    context
        .draw_image_with_image_bitmap_and_sw_and_sh_and_dx_and_dy_and_dw_and_dh(
            bitmap,
            f64::from(square.x),
            f64::from(square.y),
            side,
            side,
            0.0,
            0.0,
            edge,
            edge,
        )
        .ok()?;
    // The first quality that fits — and when none does, the last one tried:
    // better a larger upload the server may still take than no picture.
    let mut last = None;
    for quality in avatar::QUALITIES {
        let Some(jpeg) = to_blob(&canvas, "image/jpeg", quality).await else {
            continue;
        };
        if jpeg.size() <= avatar::MAX_BYTES as f64 {
            return Some(jpeg);
        }
        last = Some(jpeg);
    }
    last
}

/// The image, decoded the right way up. `from-image` is what honours EXIF
/// orientation; a browser that does not know the option decodes without it.
async fn decode(blob: &Blob) -> Option<ImageBitmap> {
    let window = web_sys::window()?;
    let options = ImageBitmapOptions::new();
    options.set_image_orientation(ImageOrientation::FromImage);
    let promise = window
        .create_image_bitmap_with_blob_and_image_bitmap_options(blob, &options)
        .or_else(|_| window.create_image_bitmap_with_blob(blob))
        .ok()?;
    JsFuture::from(promise).await.ok()?.dyn_into().ok()
}

enum Source<'a> {
    Bitmap(&'a ImageBitmap),
    Video(&'a HtmlVideoElement),
}

/// `source` drawn at `width`×`height` over white, as a JPEG.
async fn draw_jpeg(source: &Source<'_>, width: u32, height: u32, quality: f64) -> Option<Blob> {
    let document = web_sys::window()?.document()?;
    let canvas: HtmlCanvasElement = document.create_element("canvas").ok()?.dyn_into().ok()?;
    canvas.set_width(width);
    canvas.set_height(height);
    let context: CanvasRenderingContext2d = canvas.get_context("2d").ok()??.dyn_into().ok()?;
    context.set_fill_style_str("#ffffff");
    context.fill_rect(0.0, 0.0, f64::from(width), f64::from(height));
    let _ = js_sys::Reflect::set(
        &context,
        &JsValue::from_str("imageSmoothingQuality"),
        &JsValue::from_str("high"),
    );
    let (w, h) = (f64::from(width), f64::from(height));
    match source {
        Source::Bitmap(bitmap) => context
            .draw_image_with_image_bitmap_and_dw_and_dh(bitmap, 0.0, 0.0, w, h)
            .ok()?,
        Source::Video(video) => context
            .draw_image_with_html_video_element_and_dw_and_dh(video, 0.0, 0.0, w, h)
            .ok()?,
    }
    to_blob(&canvas, "image/jpeg", quality).await
}

/// `canvas.toBlob`, as a future.
async fn to_blob(canvas: &HtmlCanvasElement, mime: &str, quality: f64) -> Option<Blob> {
    let (sender, receiver) = oneshot::channel::<Option<Blob>>();
    let sender = Rc::new(RefCell::new(Some(sender)));
    let callback = Closure::<dyn FnMut(JsValue)>::new(move |value: JsValue| {
        if let Some(sender) = sender.borrow_mut().take() {
            let _ = sender.send(value.dyn_into::<Blob>().ok());
        }
    });
    canvas
        .to_blob_with_type_and_encoder_options(
            callback.as_ref().unchecked_ref(),
            mime,
            &JsValue::from_f64(quality),
        )
        .ok()?;
    let blob = receiver.await.ok().flatten();
    drop(callback);
    blob.filter(|blob| blob.size() > 0.0)
}

/// Wait for `event` on `target`, up to `timeout_ms`; false on `error` or
/// on the timeout.
pub async fn wait_for(target: &EventTarget, event: &str, timeout_ms: u32) -> bool {
    let (sender, receiver) = oneshot::channel::<bool>();
    let sender = Rc::new(RefCell::new(Some(sender)));
    let fired = {
        let sender = sender.clone();
        Closure::<dyn FnMut()>::new(move || {
            if let Some(sender) = sender.borrow_mut().take() {
                let _ = sender.send(true);
            }
        })
    };
    let failed = Closure::<dyn FnMut()>::new(move || {
        if let Some(sender) = sender.borrow_mut().take() {
            let _ = sender.send(false);
        }
    });
    let _ = target.add_event_listener_with_callback(event, fired.as_ref().unchecked_ref());
    let _ = target.add_event_listener_with_callback("error", failed.as_ref().unchecked_ref());
    let outcome = match select(receiver, TimeoutFuture::new(timeout_ms)).await {
        Either::Left((answer, _)) => answer.unwrap_or(false),
        Either::Right(_) => false,
    };
    let _ = target.remove_event_listener_with_callback(event, fired.as_ref().unchecked_ref());
    let _ = target.remove_event_listener_with_callback("error", failed.as_ref().unchecked_ref());
    outcome
}

/// A media element loading `blob`, and the URL to let go of after.
fn media_element(tag: &str, blob: &Blob) -> Option<(HtmlMediaElement, String)> {
    let document = web_sys::window()?.document()?;
    let element: HtmlMediaElement = document.create_element(tag).ok()?.dyn_into().ok()?;
    let url = Url::create_object_url_with_blob(blob).ok()?;
    element.set_muted(true);
    element.set_preload("auto");
    let _ = element.set_attribute("playsinline", "");
    element.set_src(&url);
    Some((element, url))
}

fn finite_ms(seconds: f64) -> Option<i64> {
    (seconds.is_finite() && seconds > 0.0).then(|| (seconds * 1000.0).round() as i64)
}

/// What a send does with a transcode's outcome: the result, or the source
/// as it went before (docs/protocol.md, rules C and D).
///
/// `result_bytes` is None when nothing was made — the plan kept the source
/// (rule A), or this browser could not transcode it (rule C). A result
/// BIGGER than a source that could itself be sent is thrown away (rule D);
/// one bigger than a source that could not is still the only thing there
/// is to send.
fn chosen(kind: &str, container: &str, source_bytes: u64, result_bytes: Option<u64>) -> Upload {
    // `honest`: the router has already held these bytes to the server's
    // own magic-number check, or they would not be on this path.
    let sendable = media_plan::sendable(kind, container, true, source_bytes, media::SIZE_LIMIT);
    match result_bytes {
        Some(result_bytes) => media_plan::keep_smaller(source_bytes, sendable, result_bytes),
        None => Upload::Source,
    }
}

async fn video(file: &File, job: &Job) -> Result<Prepared, PrepError> {
    let mime = media::declared_type(&file.type_(), &file.name());
    let container = if mime == "video/quicktime" {
        "video/quicktime"
    } else {
        "video/mp4"
    };
    let made = match encode::video(file, container, job).await {
        Some(Planned::Made(blob)) => Some(blob),
        Some(Planned::Keep) | None => None,
    };
    // Asked BEFORE rule C is: a stopped transcode also made nothing, and
    // must not be mistaken for one this browser could not do.
    if job.stopped() {
        return Err(PrepError::Cancelled);
    }
    sent_video(file, container, made).await
}

/// What goes for a picked video once the transcode has had its say: `made`
/// is its result, or None when there is none — the plan kept the source
/// (rule A) or this browser could not transcode it (rule C).
async fn sent_video(
    file: &File,
    container: &'static str,
    made: Option<Blob>,
) -> Result<Prepared, PrepError> {
    let result_bytes = made.as_ref().map(|blob| blob.size() as u64);
    match (
        chosen("video", container, file.size() as u64, result_bytes),
        made,
    ) {
        (Upload::Result, Some(blob)) => {
            within_limit(&blob)?;
            Ok(described_video(blob, "video/mp4").await)
        }
        // Rule A, rule C, or rule D's "the source instead": what this did
        // before any of them — untouched within the ceiling, refused over it.
        _ => {
            within_limit(file)?;
            Ok(described_video(file.clone().into(), container).await)
        }
    }
}

/// `blob`, going as a video of type `mime`, with what the browser's own
/// player can read of it.
async fn described_video(blob: Blob, mime: &str) -> Prepared {
    let mut prepared = Prepared {
        kind: "video".into(),
        mime: mime.into(),
        size: blob.size() as i64,
        file: Some(blob.clone()),
        ..Prepared::default()
    };
    // Everything below is what the browser can READ of it. A codec it
    // cannot play still goes — the server checks the container, not the
    // codec — just without its size, length or poster.
    let Some((element, url)) = media_element("video", &blob) else {
        return prepared;
    };
    let video: HtmlVideoElement = element.clone().unchecked_into();
    if wait_for(&element, "loadedmetadata", 10_000).await {
        let (width, height) = (video.video_width(), video.video_height());
        if width > 0 && height > 0 {
            prepared.width = Some(i64::from(width));
            prepared.height = Some(i64::from(height));
        }
        prepared.duration_ms = finite_ms(element.duration());
        prepared.preview = poster(&video, element.duration()).await;
    }
    element.set_src("");
    let _ = Url::revoke_object_url(&url);
    prepared
}

/// A frame worth drawing: past a fade-in first, then the very start, then a
/// little later (MediaPrep.posterFrame's seek points).
async fn poster(video: &HtmlVideoElement, duration: f64) -> Option<Blob> {
    let (width, height) = media::fit_within(
        video.video_width().max(1),
        video.video_height().max(1),
        media::PREVIEW_EDGE,
    );
    for seconds in media::POSTER_SEEK_SECONDS {
        let at = if duration.is_finite() && duration > 0.0 {
            seconds.min((duration - 0.05).max(0.0))
        } else {
            seconds
        };
        video.set_current_time(at);
        if !wait_for(video, "seeked", 5_000).await {
            continue;
        }
        if let Some(frame) =
            draw_jpeg(&Source::Video(video), width, height, media::PREVIEW_QUALITY).await
        {
            return Some(frame);
        }
    }
    log::warn!("No poster frame could be read from a video; it will be sent without one");
    None
}

/// A picked sound file, which would be sent as `container`: one of the
/// types the server takes as audio, or AIFF or FLAC, which it does not.
async fn audio(file: &File, container: &'static str, job: &Job) -> Result<Prepared, PrepError> {
    let name = file.name();
    let probe = encode::probe_audio(file, container).await;
    let made = match media_plan::plan_audio(&probe.source) {
        AudioPlan::Transcode { .. } => encode::audio(file, &probe, job).await,
        AudioPlan::Keep => None,
    };
    if job.stopped() {
        return Err(PrepError::Cancelled);
    }
    let result_bytes = made.as_ref().map(|track| track.blob.size() as u64);
    let size = file.size() as u64;
    if let (Upload::Result, Some(track)) = (chosen("audio", container, size, result_bytes), made) {
        within_limit(&track.blob)?;
        return Ok(Prepared {
            kind: "audio".into(),
            mime: "audio/mp4".into(),
            size: track.blob.size() as i64,
            duration_ms: Some(track.duration_ms),
            // A track's title is worth showing; a recording has none.
            name: media::sanitized_name(&media::m4a_name(&name)),
            file: Some(track.blob),
            ..Prepared::default()
        });
    }
    // What went before there were audio rules (rule C). For a type the
    // server takes that is the file itself, untouched; AIFF and FLAC were
    // never audio to it, and go as the files they always went as.
    if media::audio_mime(container, "").is_none() {
        return as_file(file);
    }
    within_limit(file)?;
    let mut prepared = Prepared {
        kind: "audio".into(),
        mime: container.into(),
        size: file.size() as i64,
        name: media::sanitized_name(&name),
        file: Some(file.clone().into()),
        ..Prepared::default()
    };
    if let Some((element, url)) = media_element("audio", file) {
        if wait_for(&element, "loadedmetadata", 10_000).await {
            prepared.duration_ms = finite_ms(element.duration());
        }
        element.set_src("");
        let _ = Url::revoke_object_url(&url);
    }
    Ok(prepared)
}

/// A voice note, recorded here: no name — its length is its identity.
///
/// Checked the way the server will check it. A recorder that promised MP4
/// and wrote something else sends it as a file — heard by nobody's player
/// here, but not refused, and not lost.
pub async fn recording(blob: Blob, mime: &str, duration_ms: i64) -> Result<Prepared, PrepError> {
    within_limit(&blob)?;
    let bytes = head(&blob, 12).await;
    let checks = media::matches_magic(mime, &bytes);
    Ok(Prepared {
        kind: if checks { "audio" } else { "file" }.into(),
        mime: mime.into(),
        size: blob.size() as i64,
        duration_ms: checks.then_some(duration_ms),
        name: (!checks).then(|| "Voice note.m4a".to_string()),
        file: Some(blob),
        ..Prepared::default()
    })
}

/// A video message, recorded here (round_video): sent EXACTLY as recorded —
/// it is the profile already, and never meets the planner (the plan for #79,
/// "The recording profile for a round video") — with its square poster from
/// 0.5 s in, else the start, else 2 s, as every video's is: 480 × 480, the
/// clip's own size, which is inside the 600 a preview may be.
pub async fn round_video(blob: Blob, duration_ms: i64) -> Prepared {
    let edge = i64::from(crate::encode::ROUND_EDGE);
    let mut prepared = Prepared {
        kind: "video".into(),
        mime: "video/mp4".into(),
        size: blob.size() as i64,
        width: Some(edge),
        height: Some(edge),
        duration_ms: Some(duration_ms),
        file: Some(blob.clone()),
        ..Prepared::default()
    };
    if let Some((element, url)) = media_element("video", &blob) {
        let video: HtmlVideoElement = element.clone().unchecked_into();
        if wait_for(&element, "loadedmetadata", 10_000).await {
            prepared.preview = poster(&video, element.duration()).await;
        }
        element.set_src("");
        let _ = Url::revoke_object_url(&url);
    }
    prepared
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    /// A picture made in the browser, the way a canvas makes one.
    async fn png(width: u32, height: u32) -> Blob {
        let document = web_sys::window().unwrap().document().unwrap();
        let canvas: HtmlCanvasElement = document
            .create_element("canvas")
            .unwrap()
            .dyn_into()
            .unwrap();
        canvas.set_width(width);
        canvas.set_height(height);
        let context: CanvasRenderingContext2d = canvas
            .get_context("2d")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        context.set_fill_style_str("rgba(200, 30, 30, 0.5)");
        context.fill_rect(0.0, 0.0, f64::from(width), f64::from(height));
        to_blob(&canvas, "image/png", 1.0).await.unwrap()
    }

    fn file(blob: &Blob, name: &str, mime: &str) -> File {
        let parts = js_sys::Array::of1(blob);
        let options = web_sys::FilePropertyBag::new();
        options.set_type(mime);
        File::new_with_blob_sequence_and_options(&parts, name, &options).unwrap()
    }

    /// A big PNG becomes a JPEG no longer than 2048 on its longest edge,
    /// with a preview no longer than 600 — both ones the server's magic
    /// check takes.
    #[wasm_bindgen_test]
    async fn a_photo_is_downscaled_to_jpeg_with_a_preview() {
        let source = png(3000, 1500).await;
        let prepared = prepare(&file(&source, "wide.png", "image/png"), &Job::default())
            .await
            .unwrap();
        assert_eq!(prepared.kind, "photo");
        assert_eq!(prepared.mime, "image/jpeg");
        assert_eq!((prepared.width, prepared.height), (Some(2048), Some(1024)));
        let bytes = prepared.file.clone().unwrap();
        assert!(media::matches_magic("image/jpeg", &head(&bytes, 12).await));
        assert_eq!(prepared.size, bytes.size() as i64);
        let preview = prepared.preview.clone().expect("a preview");
        assert!(media::matches_magic(
            "image/jpeg",
            &head(&preview, 12).await
        ));
        let small = decode(&preview).await.unwrap();
        assert_eq!((small.width(), small.height()), (600, 300));
    }

    /// A phone's portrait photo is a landscape image with an EXIF note
    /// saying "turn me". Decoded the right way up, it arrives the right way
    /// up — and the note, with everything else EXIF carries, stays behind.
    #[wasm_bindgen_test]
    async fn a_photo_is_turned_the_way_its_camera_said() {
        let document = web_sys::window().unwrap().document().unwrap();
        let canvas: HtmlCanvasElement = document
            .create_element("canvas")
            .unwrap()
            .dyn_into()
            .unwrap();
        canvas.set_width(80);
        canvas.set_height(40);
        let context: CanvasRenderingContext2d = canvas
            .get_context("2d")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        context.set_fill_style_str("#c81e1e");
        context.fill_rect(0.0, 0.0, 80.0, 40.0);
        let plain = to_blob(&canvas, "image/jpeg", 0.9).await.unwrap();
        let mut bytes =
            js_sys::Uint8Array::new(&JsFuture::from(plain.array_buffer()).await.unwrap()).to_vec();
        // APP1, Exif, big-endian TIFF, one IFD entry: Orientation = 6
        // (rotate 90° clockwise to view).
        let tiff: [u8; 26] = [
            b'M', b'M', 0, 0x2A, 0, 0, 0, 8, 0, 1, 0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, 6, 0, 0, 0, 0,
            0, 0,
        ];
        let mut app1 = vec![0xFF, 0xE1];
        let length = (2 + 6 + tiff.len()) as u16;
        app1.extend_from_slice(&length.to_be_bytes());
        app1.extend_from_slice(b"Exif\0\0");
        app1.extend_from_slice(&tiff);
        bytes.splice(2..2, app1);
        let array = js_sys::Uint8Array::from(bytes.as_slice());
        let turned = Blob::new_with_u8_array_sequence(&js_sys::Array::of1(&array)).unwrap();

        let prepared = prepare(
            &file(&turned, "IMG_0003.JPG", "image/jpeg"),
            &Job::default(),
        )
        .await
        .unwrap();

        assert_eq!(
            (prepared.width, prepared.height),
            (Some(40), Some(80)),
            "portrait"
        );
        let out = js_sys::Uint8Array::new(
            &JsFuture::from(prepared.file.unwrap().array_buffer())
                .await
                .unwrap(),
        )
        .to_vec();
        assert!(
            !out.windows(6).any(|window| window == b"Exif\0\0"),
            "nothing of the camera's notes travels"
        );
    }

    /// Small stays small: nothing is ever scaled up.
    #[wasm_bindgen_test]
    async fn a_small_photo_keeps_its_size() {
        let source = png(640, 480).await;
        let prepared = prepare(&file(&source, "small.png", "image/png"), &Job::default())
            .await
            .unwrap();
        assert_eq!((prepared.width, prepared.height), (Some(640), Some(480)));
    }

    /// A photo this browser cannot decode is refused — never sent as its
    /// original bytes, which would carry the EXIF the re-encode leaves
    /// behind, the place it was taken included.
    #[wasm_bindgen_test]
    async fn an_undecodable_photo_is_refused_not_sent_as_it_was() {
        let parts = js_sys::Array::of1(&JsValue::from_str("not really a picture"));
        let blob = Blob::new_with_str_sequence(&parts).unwrap();
        let refused = prepare(&file(&blob, "IMG_0001.heic", "image/heic"), &Job::default()).await;
        assert_eq!(refused, Err(PrepError::Unreadable));
        assert_eq!(PrepError::Unreadable.message(), "Couldn't read that file.");
    }

    /// A GIF animates: its original bytes go, as a file.
    #[wasm_bindgen_test]
    async fn a_gif_goes_as_itself() {
        let parts = js_sys::Array::of1(&JsValue::from_str("GIF89a…"));
        let blob = Blob::new_with_str_sequence(&parts).unwrap();
        let prepared = prepare(
            &file(&blob, "dance:party.gif", "image/gif"),
            &Job::default(),
        )
        .await
        .unwrap();
        assert_eq!(prepared.kind, "file");
        assert_eq!(prepared.name.as_deref(), Some("dance_party.gif"));
        assert!(prepared.preview.is_none());
    }

    /// Transparency is laid over WHITE, as the Mac lays it: a see-through
    /// PNG must not arrive on a black background.
    #[wasm_bindgen_test]
    async fn transparency_becomes_white() {
        let document = web_sys::window().unwrap().document().unwrap();
        let canvas: HtmlCanvasElement = document
            .create_element("canvas")
            .unwrap()
            .dyn_into()
            .unwrap();
        canvas.set_width(64);
        canvas.set_height(64);
        // Nothing drawn at all: every pixel fully transparent.
        let clear = to_blob(&canvas, "image/png", 1.0).await.unwrap();
        let prepared = prepare(&file(&clear, "clear.png", "image/png"), &Job::default())
            .await
            .unwrap();
        let bitmap = decode(prepared.file.as_ref().unwrap()).await.unwrap();
        let check: HtmlCanvasElement = document
            .create_element("canvas")
            .unwrap()
            .dyn_into()
            .unwrap();
        check.set_width(64);
        check.set_height(64);
        let context: CanvasRenderingContext2d = check
            .get_context("2d")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        context
            .draw_image_with_image_bitmap(&bitmap, 0.0, 0.0)
            .unwrap();
        let pixel = context.get_image_data(32.0, 32.0, 1.0, 1.0).unwrap().data();
        assert!(
            pixel[0] > 240 && pixel[1] > 240 && pixel[2] > 240,
            "{:?}",
            &pixel[..3]
        );
    }

    // --- sound and video (docs/protocol.md, "Preparing media before upload") ---

    use crate::encode::testing::{blob, film, noise, read, whole, PORTRAIT, QUICKTIME, WITHIN};
    use crate::webcodecs::testing::{refusing, without, Lack};
    use fc_text::{mp4_read, wav};
    use std::cell::Cell;

    const AAC_256K: &[u8] = include_bytes!("../text/fixtures/aac-256k.m4a");
    const AAC_128K: &[u8] = include_bytes!("../text/fixtures/aac-128k.m4a");
    const FLAC: &[u8] = include_bytes!("../text/fixtures/lossless.flac");
    const AIFF: &[u8] = include_bytes!("../text/fixtures/uncompressed.aiff");
    const ALAC: &[u8] = include_bytes!("../text/fixtures/lossless-alac.m4a");

    fn picked(bytes: &[u8], name: &str, mime: &str) -> File {
        file(&blob(bytes, mime), name, mime)
    }

    /// What an M4A this client made holds: channels, rate, bit/s.
    async fn sound_of(prepared: &Prepared) -> (u8, u32, u64) {
        let bytes = whole(prepared.file.as_ref().unwrap()).await;
        assert!(media::matches_magic("audio/mp4", &bytes[..12]));
        let (kinds, movie) = read(&bytes);
        assert_eq!(
            kinds,
            vec![*b"ftyp", *b"moov", *b"mdat"],
            "the index comes first"
        );
        let track = movie.audio().unwrap();
        let config = mp4_read::audio_config(&track.entry.as_ref().unwrap().config).unwrap();
        assert_eq!(config.object_type, 2, "AAC-LC");
        (
            config.channels,
            config.sample_rate,
            track.data_rate().unwrap(),
        )
    }

    /// Rule D, and what stands behind it: a result is used unless it is
    /// bigger than a source that could itself have gone.
    #[wasm_bindgen_test]
    fn a_result_bigger_than_a_sendable_source_is_thrown_away() {
        let limit = media::SIZE_LIMIT;
        assert_eq!(chosen("video", "video/mp4", 1_000, None), Upload::Source);
        assert_eq!(
            chosen("video", "video/mp4", 1_000, Some(999)),
            Upload::Result
        );
        assert_eq!(
            chosen("video", "video/mp4", 1_000, Some(1_000)),
            Upload::Result
        );
        assert_eq!(
            chosen("video", "video/quicktime", 1_000, Some(1_001)),
            Upload::Source
        );
        // Over the ceiling the source cannot go, so the result does.
        assert_eq!(
            chosen("video", "video/mp4", limit + 1, Some(limit + 2)),
            Upload::Result
        );
        // AIFF and FLAC are not audio to the server at any size.
        assert_eq!(
            chosen("audio", "audio/flac", 1_000, Some(5_000)),
            Upload::Result
        );
        assert_eq!(
            chosen("audio", "audio/wav", 1_000, Some(5_000)),
            Upload::Source
        );
    }

    /// AAC above 192 kbit/s is re-encoded — stereo, so at 128 000 — and AAC
    /// at 128 kbit/s goes byte for byte as it was.
    #[wasm_bindgen_test]
    async fn aac_over_192k_is_re_encoded_and_under_it_left_alone() {
        let prepared = prepare(
            &picked(AAC_256K, "Song.m4a", "audio/x-m4a"),
            &Job::default(),
        )
        .await
        .unwrap();
        assert_eq!(
            (prepared.kind.as_str(), prepared.mime.as_str()),
            ("audio", "audio/mp4")
        );
        assert_eq!(prepared.name.as_deref(), Some("Song.m4a"));
        assert!(
            (prepared.size as usize) < AAC_256K.len(),
            "{} bytes",
            prepared.size
        );
        assert_eq!(prepared.size, prepared.file.as_ref().unwrap().size() as i64);
        let (channels, sample_rate, rate) = sound_of(&prepared).await;
        assert_eq!(
            (channels, sample_rate),
            (2, 44_100),
            "its own rate, never raised"
        );
        assert!(rate <= 128_000 * 9 / 8, "{rate} bit/s");
        let length = prepared.duration_ms.unwrap();
        assert!((500..=600).contains(&length), "{length} ms");

        let kept = prepare(&picked(AAC_128K, "Quiet.m4a", "audio/mp4"), &Job::default())
            .await
            .unwrap();
        assert_eq!(
            (kept.kind.as_str(), kept.mime.as_str()),
            ("audio", "audio/mp4")
        );
        assert_eq!(
            whole(kept.file.as_ref().unwrap()).await,
            AAC_128K,
            "untouched"
        );
    }

    /// Uncompressed sound becomes M4A: mono at 64 000, at a rate the
    /// encoder takes, as long as it was — and under a name that says so.
    #[wasm_bindgen_test]
    async fn a_wav_becomes_an_m4a_a_fraction_of_its_size() {
        let pcm = wav::encode(&noise(16_000 * 3, 16_000, 5), 16_000);
        let prepared = prepare(&picked(&pcm, "Memo 12.wav", "audio/wav"), &Job::default())
            .await
            .unwrap();
        assert_eq!(
            (prepared.kind.as_str(), prepared.mime.as_str()),
            ("audio", "audio/mp4")
        );
        assert_eq!(prepared.name.as_deref(), Some("Memo 12.m4a"));
        assert_eq!(prepared.duration_ms, Some(3_000));
        let (channels, sample_rate, rate) = sound_of(&prepared).await;
        assert_eq!((channels, sample_rate), (1, 48_000));
        assert!((40_000..=72_000).contains(&rate), "{rate} bit/s");
        assert!(
            (prepared.size as usize) < pcm.len() / 3,
            "{} of {}",
            prepared.size,
            pcm.len()
        );
    }

    /// FLAC is not a type the server takes as audio: before the audio rules
    /// it went as a file. Re-encoded, it goes as audio — even where the
    /// result is BIGGER, because the source was never sendable as audio.
    #[wasm_bindgen_test]
    async fn a_flac_goes_as_audio_once_it_is_re_encoded() {
        let prepared = prepare(&picked(FLAC, "Concert.flac", "audio/flac"), &Job::default())
            .await
            .unwrap();
        assert_eq!(
            (prepared.kind.as_str(), prepared.mime.as_str()),
            ("audio", "audio/mp4")
        );
        assert_eq!(prepared.name.as_deref(), Some("Concert.m4a"));
        // The fixture is one channel: the mono row, 64 000.
        let (channels, _, rate) = sound_of(&prepared).await;
        assert_eq!(channels, 1);
        assert!(rate <= 64_000 * 9 / 8, "{rate} bit/s");
    }

    /// Whether this browser's own decoder reads `bytes` as sound — asked of
    /// the browser, so that a test knows which of two right answers to
    /// hold it to instead of accepting either.
    async fn decodes(bytes: &[u8]) -> bool {
        let context =
            web_sys::OfflineAudioContext::new_with_number_of_channels_and_length_and_sample_rate(
                1, 1, 48_000.0,
            )
            .unwrap();
        let buffer = js_sys::Uint8Array::from(bytes).buffer();
        match context.decode_audio_data(&buffer) {
            Ok(decoding) => JsFuture::from(decoding).await.is_ok(),
            Err(_) => false,
        }
    }

    /// Rule C for sound: what this browser cannot decode or cannot encode
    /// goes exactly as it went before — an accepted type untouched, AIFF
    /// and FLAC as the files they always were.
    #[wasm_bindgen_test]
    async fn sound_this_browser_cannot_re_encode_goes_as_it_did_before() {
        // AIFF and ALAC decode in some browsers and not in others. Which
        // this one is, it says — and is then held to: re-encoded where it
        // decodes them, untouched where it does not. Never "either".
        let aiff = prepare(&picked(AIFF, "Take.aiff", "audio/aiff"), &Job::default())
            .await
            .unwrap();
        if decodes(AIFF).await {
            assert_eq!(
                (aiff.kind.as_str(), aiff.mime.as_str()),
                ("audio", "audio/mp4")
            );
            assert_eq!(aiff.name.as_deref(), Some("Take.m4a"));
            assert_eq!(aiff.duration_ms, Some(500));
            let (channels, _, rate) = sound_of(&aiff).await;
            assert_eq!(channels, 1);
            assert!(rate <= 64_000 * 9 / 8, "{rate} bit/s");
        } else {
            console_log!("This browser does not decode AIFF: it goes as the file it was.");
            assert_eq!(
                (aiff.kind.as_str(), aiff.mime.as_str()),
                ("file", "audio/aiff")
            );
            assert_eq!(aiff.name.as_deref(), Some("Take.aiff"));
            assert_eq!(whole(aiff.file.as_ref().unwrap()).await, AIFF);
        }
        let alac = prepare(&picked(ALAC, "Album.m4a", "audio/mp4"), &Job::default())
            .await
            .unwrap();
        assert_eq!(
            (alac.kind.as_str(), alac.mime.as_str()),
            ("audio", "audio/mp4")
        );
        if decodes(ALAC).await {
            assert_ne!(whole(alac.file.as_ref().unwrap()).await, ALAC);
            let (channels, _, rate) = sound_of(&alac).await;
            assert_eq!(channels, 1);
            assert!(rate <= 64_000 * 9 / 8, "{rate} bit/s");
        } else {
            console_log!("This browser does not decode ALAC: it goes untouched.");
            assert_eq!(whole(alac.file.as_ref().unwrap()).await, ALAC);
            assert_eq!(alac.size as usize, ALAC.len());
        }

        // A browser with no AAC encoder re-encodes nothing.
        for stand in [without as Lack, refusing] {
            let _stand = stand("AudioEncoder");
            let aac = prepare(&picked(AAC_256K, "Song.m4a", "audio/mp4"), &Job::default())
                .await
                .unwrap();
            assert_eq!(
                (aac.kind.as_str(), aac.mime.as_str()),
                ("audio", "audio/mp4")
            );
            assert_eq!(whole(aac.file.as_ref().unwrap()).await, AAC_256K);
            let flac = prepare(&picked(FLAC, "Concert.flac", "audio/flac"), &Job::default())
                .await
                .unwrap();
            assert_eq!(
                (flac.kind.as_str(), flac.mime.as_str()),
                ("file", "audio/flac")
            );
            assert_eq!(flac.name.as_deref(), Some("Concert.flac"));
            assert_eq!(whole(flac.file.as_ref().unwrap()).await, FLAC);
        }

        // Rule D: a WAV so short that its M4A — an index and a frame —
        // would be the bigger of the two goes as the WAV it is.
        let tiny = wav::encode(&[0.0; 64], 16_000);
        let kept = prepare(&picked(&tiny, "blip.wav", "audio/wav"), &Job::default())
            .await
            .unwrap();
        assert_eq!(
            (kept.kind.as_str(), kept.mime.as_str()),
            ("audio", "audio/wav")
        );
        assert_eq!(whole(kept.file.as_ref().unwrap()).await, tiny);
        assert_eq!(kept.name.as_deref(), Some("blip.wav"));
    }

    /// A picked video outside the profile goes as the profile's MP4, with
    /// the size, length and poster of what is actually being sent.
    #[wasm_bindgen_test]
    async fn a_video_outside_the_profile_is_sent_as_the_profiles_mp4() {
        let source = film(1920, 1080, 60, 2, 8_000_000, Some(256_000)).await;
        let picked = file(&source, "IMG_0042.mp4", "video/mp4");
        let prepared = prepare(&picked, &Job::default()).await.unwrap();
        assert_eq!(
            (prepared.kind.as_str(), prepared.mime.as_str()),
            ("video", "video/mp4")
        );
        let sent = prepared.file.as_ref().unwrap();
        assert_eq!(prepared.size, sent.size() as i64);
        assert!(
            sent.size() < source.size() / 2.0,
            "{} of {}",
            sent.size(),
            source.size()
        );
        assert_eq!((prepared.width, prepared.height), (Some(1280), Some(720)));
        // Two seconds — and an AAC encoder's priming and padding, twice
        // over: the fixture's own sound carries its encoder's (with no edit
        // list to say so), and the re-encode adds this browser's.
        let length = prepared.duration_ms.unwrap();
        assert!((2_000..=2_150).contains(&length), "{length} ms");
        let poster = prepared.preview.clone().expect("a poster");
        let poster = decode(&poster).await.unwrap();
        assert_eq!((poster.width(), poster.height()), (600, 338));
        let bytes = whole(sent).await;
        assert!(media::matches_magic("video/mp4", &bytes[..12]));
        let (kinds, movie) = read(&bytes);
        assert_eq!(kinds, vec![*b"ftyp", *b"moov", *b"mdat"]);
        assert_eq!(
            movie.video().unwrap().samples.len(),
            60,
            "30 frames a second"
        );
    }

    /// Rule A: a clip within the profile goes byte for byte as it is.
    #[wasm_bindgen_test]
    async fn a_video_within_the_profile_goes_untouched() {
        let prepared = prepare(&picked(WITHIN, "clip.mp4", "video/mp4"), &Job::default())
            .await
            .unwrap();
        assert_eq!(
            (prepared.kind.as_str(), prepared.mime.as_str()),
            ("video", "video/mp4")
        );
        assert_eq!(whole(prepared.file.as_ref().unwrap()).await, WITHIN);
        assert_eq!((prepared.width, prepared.height), (Some(1280), Some(720)));
    }

    /// Rule C for video: with no encoder, or no decoder, a QuickTime movie
    /// goes as the QuickTime movie it is — exactly as before.
    #[wasm_bindgen_test]
    async fn a_video_this_browser_cannot_transcode_goes_as_it_did_before() {
        let stands: [(Lack, &'static str); 3] = [
            (without, "VideoEncoder"),
            (refusing, "VideoEncoder"),
            (without, "VideoDecoder"),
        ];
        for (stand, class) in stands {
            let _stand = stand(class);
            let prepared = prepare(
                &picked(QUICKTIME, "IMG_0001.MOV", "video/quicktime"),
                &Job::default(),
            )
            .await
            .unwrap();
            assert_eq!(
                (prepared.kind.as_str(), prepared.mime.as_str()),
                ("video", "video/quicktime")
            );
            assert_eq!(whole(prepared.file.as_ref().unwrap()).await, QUICKTIME);
        }
    }

    /// Rule D for video, decided: a result BIGGER than a source that could
    /// itself go is thrown away and the source goes, as the type it is; a
    /// result no bigger goes; and a bigger result still goes when the
    /// source could not — it is the only thing there is to send.
    #[wasm_bindgen_test]
    async fn a_transcode_that_came_out_bigger_is_thrown_away() {
        let source = picked(WITHIN, "IMG_0002.MOV", "video/quicktime");
        // Bigger than its source by one byte.
        let mut grown = WITHIN.to_vec();
        grown.push(0);
        let sent = sent_video(&source, "video/quicktime", Some(blob(&grown, "video/mp4")))
            .await
            .unwrap();
        assert_eq!(
            (sent.kind.as_str(), sent.mime.as_str()),
            ("video", "video/quicktime")
        );
        assert_eq!(
            whole(sent.file.as_ref().unwrap()).await,
            WITHIN,
            "the source, untouched"
        );
        assert_eq!(sent.size as usize, WITHIN.len());
        // Exactly its size is not bigger: the result goes.
        let same = sent_video(&source, "video/quicktime", Some(blob(WITHIN, "video/mp4")))
            .await
            .unwrap();
        assert_eq!(same.mime, "video/mp4");
        // Smaller: the result, described as what IT is.
        assert!(QUICKTIME.len() < WITHIN.len());
        let smaller = sent_video(
            &source,
            "video/quicktime",
            Some(blob(QUICKTIME, "video/mp4")),
        )
        .await
        .unwrap();
        assert_eq!(smaller.mime, "video/mp4");
        assert_eq!(whole(smaller.file.as_ref().unwrap()).await, QUICKTIME);
        assert_eq!((smaller.width, smaller.height), (Some(640), Some(360)));
        // Nothing made (rule A, rule C): the source.
        let kept = sent_video(&source, "video/quicktime", None).await.unwrap();
        assert_eq!(kept.mime, "video/quicktime");
        assert_eq!(whole(kept.file.as_ref().unwrap()).await, WITHIN);

        // And through the whole path, on the fixtures: they are flat colour,
        // so small that an encoder may not get under them. Whichever this
        // browser's does, what goes is never the bigger of the two — the
        // profile's MP4 when it is the smaller, the source when it is not.
        for (source, mime) in [
            (QUICKTIME, "video/quicktime"),
            (PORTRAIT, "video/quicktime"),
        ] {
            let prepared = prepare(&picked(source, "IMG_0002.MOV", mime), &Job::default())
                .await
                .unwrap();
            let sent = whole(prepared.file.as_ref().unwrap()).await;
            if prepared.mime == "video/mp4" {
                assert!(sent.len() <= source.len());
                let (kinds, movie) = read(&sent);
                assert_eq!(kinds, vec![*b"ftyp", *b"moov", *b"mdat"]);
                let entry = movie.video().unwrap().entry.clone().unwrap();
                assert_eq!(&entry.format, b"avc1");
            } else {
                assert_eq!(prepared.mime, mime);
                assert_eq!(sent, source, "the source, untouched");
            }
        }
    }

    /// A file over the ceiling: `clip`, with enough behind it — a box no
    /// player reads — to weigh more than the server takes.
    fn over_the_ceiling(clip: &Blob, name: &str) -> File {
        let weight = media::SIZE_LIMIT as u32 + 1024;
        let mut header = (weight + 8).to_be_bytes().to_vec();
        header.extend_from_slice(b"free");
        let parts = js_sys::Array::of3(
            clip,
            &js_sys::Uint8Array::from(header.as_slice()),
            &js_sys::Uint8Array::new_with_length(weight),
        );
        let options = web_sys::FilePropertyBag::new();
        options.set_type("video/mp4");
        File::new_with_blob_sequence_and_options(&parts, name, &options).unwrap()
    }

    /// A video over the ceiling could not go at all before there was a
    /// profile. Now it is transcoded, and what comes out under the ceiling
    /// goes — the only case in which this section makes a send WORK. Where
    /// this browser cannot transcode it, it is refused exactly as before.
    #[wasm_bindgen_test]
    async fn a_video_over_the_ceiling_goes_once_it_is_transcoded_under_it() {
        let clip = film(1280, 720, 60, 1, 6_000_000, Some(256_000)).await;
        let heavy = over_the_ceiling(&clip, "Holiday.mp4");
        assert!(heavy.size() as u64 > media::SIZE_LIMIT);
        let prepared = prepare(&heavy, &Job::default()).await.unwrap();
        assert_eq!(
            (prepared.kind.as_str(), prepared.mime.as_str()),
            ("video", "video/mp4")
        );
        assert!((prepared.size as u64) < media::SIZE_LIMIT / 50);
        assert_eq!(prepared.size, prepared.file.as_ref().unwrap().size() as i64);
        assert_eq!((prepared.width, prepared.height), (Some(1280), Some(720)));
        let bytes = whole(prepared.file.as_ref().unwrap()).await;
        let (_, movie) = read(&bytes);
        assert_eq!(movie.video().unwrap().samples.len(), 30);

        let stands: [(Lack, &'static str); 2] =
            [(without, "VideoEncoder"), (refusing, "VideoDecoder")];
        for (stand, class) in stands {
            let _stand = stand(class);
            assert_eq!(
                prepare(&heavy, &Job::default()).await,
                Err(PrepError::TooLarge),
                "{class}"
            );
        }
    }

    /// The Ogg checksum: CRC-32 with the polynomial 0x04C11DB7, neither
    /// reflected nor inverted.
    fn ogg_crc(bytes: &[u8]) -> u32 {
        let mut crc = 0u32;
        for byte in bytes {
            crc ^= u32::from(*byte) << 24;
            for _ in 0..8 {
                crc = if crc & 0x8000_0000 != 0 {
                    (crc << 1) ^ 0x04C1_1DB7
                } else {
                    crc << 1
                };
            }
        }
        crc
    }

    /// One Ogg page holding one packet.
    fn ogg_page(packet: &[u8], sequence: u32, granule: u64, flags: u8) -> Vec<u8> {
        let mut page = b"OggS".to_vec();
        page.push(0);
        page.push(flags);
        page.extend_from_slice(&granule.to_le_bytes());
        page.extend_from_slice(&0x4643_3734u32.to_le_bytes()); // the stream's serial
        page.extend_from_slice(&sequence.to_le_bytes());
        page.extend_from_slice(&[0; 4]); // the checksum, once the page is whole
        let mut lacing = vec![255u8; packet.len() / 255];
        lacing.push((packet.len() % 255) as u8);
        page.push(lacing.len() as u8);
        page.extend_from_slice(&lacing);
        page.extend_from_slice(packet);
        let crc = ogg_crc(&page);
        page[22..26].copy_from_slice(&crc.to_le_bytes());
        page
    }

    /// An Ogg Opus file of `seconds` of mono noise at 96 kbit/s — what a
    /// voice-message app or a Linux recorder saves. The browser's own Opus
    /// encoder makes the packets; the container is written here. None in a
    /// browser that encodes no Opus.
    async fn ogg_opus(seconds: u32) -> Option<Vec<u8>> {
        let samples = js_sys::Float32Array::from(noise(48_000 * seconds, 48_000, 13).as_slice());
        let encode = js_sys::Function::new_with_args(
            "samples",
            "return (async () => { \
               const config = {codec: 'opus', sampleRate: 48000, numberOfChannels: 1, \
                               bitrate: 96000}; \
               if (typeof AudioEncoder !== 'function' || \
                   !(await AudioEncoder.isConfigSupported(config)).supported) return null; \
               const packets = []; \
               const encoder = new AudioEncoder({ \
                 output: (chunk) => { const bytes = new Uint8Array(chunk.byteLength); \
                                      chunk.copyTo(bytes); packets.push(bytes); }, \
                 error: () => {} }); \
               encoder.configure(config); \
               encoder.encode(new AudioData({format: 'f32-planar', sampleRate: 48000, \
                 numberOfFrames: samples.length, numberOfChannels: 1, timestamp: 0, \
                 data: samples})); \
               await encoder.flush(); encoder.close(); return packets; })();",
        );
        let packets = JsFuture::from(js_sys::Promise::from(
            encode.call1(&JsValue::NULL, &samples).unwrap(),
        ))
        .await
        .unwrap();
        if packets.is_null() {
            return None;
        }
        let packets: Vec<Vec<u8>> = js_sys::Array::from(&packets)
            .iter()
            .map(|packet| js_sys::Uint8Array::from(packet).to_vec())
            .collect();
        // OpusHead: version 1, one channel, 312 samples of pre-skip, 48 kHz.
        let mut head = b"OpusHead".to_vec();
        head.extend_from_slice(&[1, 1]);
        head.extend_from_slice(&312u16.to_le_bytes());
        head.extend_from_slice(&48_000u32.to_le_bytes());
        head.extend_from_slice(&[0, 0, 0]);
        let mut tags = b"OpusTags".to_vec();
        tags.extend_from_slice(&4u32.to_le_bytes());
        tags.extend_from_slice(b"test");
        tags.extend_from_slice(&0u32.to_le_bytes());
        let mut file = ogg_page(&head, 0, 0, 2);
        file.extend(ogg_page(&tags, 1, 0, 0));
        let count = packets.len();
        for (index, packet) in packets.iter().enumerate() {
            // 20 ms to a packet: 960 samples.
            let granule = 960 * (index as u64 + 1);
            let last = if index + 1 == count { 4 } else { 0 };
            file.extend(ogg_page(packet, index as u32 + 2, granule, last));
        }
        Some(file)
    }

    /// Ogg does not play on an iPhone or a Mac, so Ogg audio — Opus here —
    /// is re-encoded wherever this browser can decode it, and goes as the
    /// Ogg it is where it cannot.
    #[wasm_bindgen_test]
    async fn an_ogg_file_becomes_an_m4a_where_this_browser_decodes_it() {
        let Some(ogg) = ogg_opus(3).await else {
            console_log!("This browser encodes no Opus: no Ogg file to pick.");
            return;
        };
        assert!(media::matches_magic("audio/ogg", &ogg[..12]));
        let probe = encode::probe_audio(&blob(&ogg, "audio/ogg"), "audio/ogg").await;
        assert_eq!(probe.source.codec, "opus");
        assert!(matches!(
            media_plan::plan_audio(&probe.source),
            AudioPlan::Transcode { bitrate: 64_000 }
        ));
        let prepared = prepare(&picked(&ogg, "Voice 004.ogg", "audio/ogg"), &Job::default())
            .await
            .unwrap();
        if decodes(&ogg).await {
            assert_eq!(
                (prepared.kind.as_str(), prepared.mime.as_str()),
                ("audio", "audio/mp4")
            );
            assert_eq!(prepared.name.as_deref(), Some("Voice 004.m4a"));
            let (channels, sample_rate, rate) = sound_of(&prepared).await;
            assert_eq!((channels, sample_rate), (1, 48_000));
            assert!(rate <= 64_000 * 9 / 8, "{rate} bit/s");
            let length = prepared.duration_ms.unwrap();
            assert!((2_950..=3_100).contains(&length), "{length} ms");
            assert!((prepared.size as usize) < ogg.len());
        } else {
            console_log!("This browser does not decode Ogg Opus: it goes untouched.");
            assert_eq!(
                (prepared.kind.as_str(), prepared.mime.as_str()),
                ("audio", "audio/ogg")
            );
            assert_eq!(whole(prepared.file.as_ref().unwrap()).await, ogg);
        }
    }

    /// A preparation that is called off sends NOTHING — not the result,
    /// and not the original in its place, which is what "this browser
    /// could not transcode it" would send.
    #[wasm_bindgen_test]
    async fn a_cancelled_preparation_is_not_a_failed_transcode() {
        let clip = film(1280, 720, 60, 2, 6_000_000, Some(256_000)).await;
        let video = file(&clip, "IMG_0042.mp4", "video/mp4");
        // Called off a third of the way through the picture.
        let stopper: Rc<RefCell<Option<Job>>> = Rc::new(RefCell::new(None));
        let reached = Rc::new(Cell::new(0));
        let job = {
            let stopper = stopper.clone();
            let reached = reached.clone();
            Job::watched(move |percent| {
                reached.set(percent);
                if percent >= 30 {
                    if let Some(job) = stopper.borrow().as_ref() {
                        job.stop();
                    }
                }
            })
        };
        *stopper.borrow_mut() = Some(job.clone());
        assert_eq!(prepare(&video, &job).await, Err(PrepError::Cancelled));
        assert!((30..60).contains(&reached.get()), "{} %", reached.get());
        // Called off before it began — a video, and a sound file.
        let stopped = Job::default();
        stopped.stop();
        assert_eq!(prepare(&video, &stopped).await, Err(PrepError::Cancelled));
        let pcm = wav::encode(&noise(16_000, 16_000, 5), 16_000);
        assert_eq!(
            prepare(&picked(&pcm, "Memo.wav", "audio/wav"), &stopped).await,
            Err(PrepError::Cancelled)
        );
        // The same files, not called off, go.
        assert!(prepare(&video, &Job::default()).await.is_ok());
        // A photo has nothing to call off, and is quick: it is prepared.
        let photo = png(64, 64).await;
        assert!(
            prepare(&file(&photo, "a.png", "image/png"), &Job::default())
                .await
                .is_ok()
        );
    }

    // --- Stickers ---------------------------------------------------------

    /// A half-transparent picture of `mime`, the way a canvas writes one.
    async fn picture(width: u32, height: u32, mime: &str) -> Blob {
        let document = web_sys::window().unwrap().document().unwrap();
        let canvas: HtmlCanvasElement = document
            .create_element("canvas")
            .unwrap()
            .dyn_into()
            .unwrap();
        canvas.set_width(width);
        canvas.set_height(height);
        let context: CanvasRenderingContext2d = canvas
            .get_context("2d")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        context.set_fill_style_str("rgba(200, 30, 30, 0.5)");
        context.fill_rect(0.0, 0.0, f64::from(width), f64::from(height));
        to_blob(&canvas, mime, 0.9).await.unwrap()
    }

    fn bytes(bytes: &[u8]) -> Blob {
        let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
        Blob::new_with_u8_array_sequence(&parts).unwrap()
    }

    /// The alpha of a picture's first pixel, as this browser decodes it.
    async fn alpha(blob: &Blob) -> u8 {
        let bitmap = decode(blob).await.expect("decodes");
        let document = web_sys::window().unwrap().document().unwrap();
        let canvas: HtmlCanvasElement = document
            .create_element("canvas")
            .unwrap()
            .dyn_into()
            .unwrap();
        canvas.set_width(bitmap.width());
        canvas.set_height(bitmap.height());
        let context: CanvasRenderingContext2d = canvas
            .get_context("2d")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        context
            .draw_image_with_image_bitmap(&bitmap, 0.0, 0.0)
            .unwrap();
        context.get_image_data(0.0, 0.0, 1.0, 1.0).unwrap().data()[3]
    }

    const PACK_CEILING: u64 = 524_288;

    /// A WebP or PNG that is a sticker already goes up AS IT IS: the very
    /// blob that was picked, declared as what its bytes are, at its own
    /// pixel size, with no preview. The photo path would have made the same
    /// file an opaque JPEG with one.
    #[wasm_bindgen_test]
    async fn a_finished_sticker_goes_up_byte_for_byte() {
        let source = picture(96, 64, "image/png").await;
        // Whatever the browser or the name said it was: the bytes decide.
        let picked = file(&source, "sticker.jpg", "image/jpeg");
        let prepared = sticker(&picked, PACK_CEILING).await.unwrap();
        assert_eq!(prepared.kind, "photo");
        assert_eq!(prepared.mime, "image/png");
        assert_eq!((prepared.width, prepared.height), (Some(96), Some(64)));
        assert_eq!(prepared.size, source.size() as i64);
        assert!(prepared.preview.is_none(), "a sticker has no preview");
        let sent = prepared.file.expect("its bytes");
        assert!(same_bytes(&sent, &source).await, "untouched");
        assert_eq!(alpha(&sent).await, alpha(&source).await);

        // The same file through the PHOTO path, for the contrast that is
        // the whole reason a sticker goes round it.
        let photo = prepare(&file(&source, "a.png", "image/png"), &Job::default())
            .await
            .unwrap();
        assert_eq!(photo.mime, "image/jpeg");
        assert_eq!(alpha(photo.file.as_ref().unwrap()).await, 255, "opaque");

        // A WebP, where this browser can make one to test with.
        let webp = picture(200, 100, "image/webp").await;
        if webp.type_() == "image/webp" {
            let prepared = sticker(&webp, PACK_CEILING).await.unwrap();
            assert_eq!(prepared.mime, "image/webp");
            assert_eq!((prepared.width, prepared.height), (Some(200), Some(100)));
            assert!(same_bytes(prepared.file.as_ref().unwrap(), &webp).await);
        }
    }

    /// A still picture that is NOT a sticker yet is made into one: fitted
    /// whole into 512 × 512, its proportions kept — and its TRANSPARENCY
    /// kept, which is what the photo path's white canvas would have cost.
    #[wasm_bindgen_test]
    async fn a_larger_still_picture_is_fitted_into_512_with_its_transparency() {
        // A PNG over the ceiling (the ceiling brought down to meet it).
        let big = picture(1200, 600, "image/png").await;
        let ceiling = big.size() as u64 - 1;
        let prepared = sticker(&big, ceiling).await.unwrap();
        assert_eq!((prepared.width, prepared.height), (Some(512), Some(256)));
        assert!(matches!(prepared.mime.as_str(), "image/png" | "image/webp"));
        assert!(prepared.size as u64 <= ceiling);
        assert!(prepared.preview.is_none());
        let made = prepared.file.expect("its bytes");
        let head = head(&made, 64).await;
        assert_eq!(fc_text::pack::type_of(&head), Some(prepared.mime.as_str()));
        let seen = alpha(&made).await;
        assert!(
            (100..=155).contains(&seen),
            "half-transparent in, half-transparent out: {seen}"
        );

        // A JPEG becomes a PNG, never scaled up.
        let jpeg = picture(300, 200, "image/jpeg").await;
        let prepared = sticker(&jpeg, PACK_CEILING).await.unwrap();
        assert_eq!(prepared.mime, "image/png");
        assert_eq!((prepared.width, prepared.height), (Some(300), Some(200)));
    }

    /// What cannot be a sticker is refused beside the picker, saying which:
    /// an animated one over the ceiling is never flattened to fit, a file
    /// that is not a picture is not uploaded as one, and a picture still
    /// too big at 512 × 512 is too big.
    #[wasm_bindgen_test]
    async fn what_cannot_be_a_sticker_is_refused() {
        use fc_text::pack::Refusal;
        let mut animated = b"RIFF\x00\x00\x00\x00WEBPVP8X\x0a\x00\x00\x00\x02\x00\x00\x00".to_vec();
        animated.resize(400, 0);
        assert_eq!(
            sticker(&bytes(&animated), 100).await,
            Err(Refusal::TooLarge),
            "never re-encoded, so never made smaller"
        );
        // An animated PNG over the ceiling would have to be REMADE, which
        // is one frame of it: refused as an animation, never flattened.
        let mut apng = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        for (kind, length) in [(b"IHDR", 13u32), (b"acTL", 8), (b"IDAT", 4)] {
            apng.extend_from_slice(&length.to_be_bytes());
            apng.extend_from_slice(kind);
            apng.resize(apng.len() + length as usize + 4, 0);
        }
        assert_eq!(
            sticker(&bytes(&apng), 16).await,
            Err(Refusal::Animated),
            "an animated sticker must be a WebP"
        );
        // Twelve honest bytes and nothing behind them.
        let mut cut_short = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        cut_short.extend_from_slice(&[0; 40]);
        assert_eq!(
            sticker(&bytes(&cut_short), PACK_CEILING).await,
            Err(Refusal::Unreadable)
        );
        assert_eq!(
            sticker(&bytes(b"just some words"), PACK_CEILING).await,
            Err(Refusal::Unreadable)
        );
        let picture = picture(300, 300, "image/jpeg").await;
        assert_eq!(sticker(&picture, 16).await, Err(Refusal::TooLarge));
    }

    /// A real 1 × 1 GIF of `frames` pictures, with `padding` bytes of
    /// comment in front of the first — enough to push the second picture
    /// past what is read of any other type.
    fn gif(frames: usize, padding: usize) -> Blob {
        let mut gif = b"GIF89a\x01\x00\x01\x00\x80\x00\x00\xFF\x00\x00\x00\x00\x00".to_vec();
        let mut left = padding;
        if left > 0 {
            gif.extend_from_slice(&[0x21, 0xFE]);
            while left > 0 {
                let run = left.min(255);
                gif.push(run as u8);
                gif.extend(std::iter::repeat_n(b' ', run));
                left -= run;
            }
            gif.push(0);
        }
        for _ in 0..frames {
            gif.extend_from_slice(&[0x21, 0xF9, 4, 0, 10, 0, 0, 0]);
            gif.extend_from_slice(&[0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0]);
            gif.extend_from_slice(&[2, 2, 0x44, 0x01, 0]);
        }
        gif.push(0x3B);
        bytes(&gif)
    }

    /// AN ANIMATED GIF IS REFUSED, IN WORDS, NEVER FLATTENED: made into a
    /// sticker it would be frame zero of what was picked, stored as a still
    /// PNG, with nothing said. A still GIF is a still picture and is made
    /// into one like any other.
    #[wasm_bindgen_test]
    async fn an_animated_gif_is_refused_and_a_still_one_is_made_a_sticker() {
        use fc_text::pack::Refusal;
        let still = sticker(&gif(1, 0), PACK_CEILING).await.unwrap();
        assert_eq!(still.mime, "image/png");
        assert_eq!((still.width, still.height), (Some(1), Some(1)));
        assert_eq!(
            sticker(&gif(3, 0), PACK_CEILING).await,
            Err(Refusal::Animated)
        );
        // The second picture is behind more than is read of any other
        // type, and there is no loop block to give it away: a GIF is read
        // whole before it is believed to be a still.
        let padding = STICKER_HEAD as usize + 4_096;
        assert_eq!(
            sticker(&gif(2, padding), PACK_CEILING).await,
            Err(Refusal::Animated)
        );
        assert!(sticker(&gif(1, padding), PACK_CEILING).await.is_ok());
    }

    #[wasm_bindgen_test]
    async fn the_same_bytes_are_the_same_whatever_blob_holds_them() {
        assert!(same_bytes(&bytes(b"abc"), &bytes(b"abc")).await);
        assert!(!same_bytes(&bytes(b"abc"), &bytes(b"abd")).await);
        assert!(!same_bytes(&bytes(b"abc"), &bytes(b"abcd")).await);
    }
}
