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
//! A VIDEO goes untouched when it fits the ceiling, with its size, length
//! and a poster frame read by the browser's own player. One over the
//! ceiling is refused: re-encoding in a tab is not something a browser does
//! well, and the Mac's 1080p export has no honest equivalent here.
//!
//! AUDIO and FILES go untouched; what kind each is, and what it is called,
//! are fc_text::media's rules.

use std::cell::RefCell;
use std::rc::Rc;

use fc_text::media::{self, Route};
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

use crate::staged::Prepared;

/// Why something could not be staged — each with the Mac's sentence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrepError {
    TooLarge,
    Unreadable,
    /// Offered to the board, which pins photos and nothing else.
    NotAPhoto,
}

impl PrepError {
    pub fn message(self) -> &'static str {
        match self {
            PrepError::TooLarge => "That file is over the 100 MB limit.",
            PrepError::Unreadable => "Couldn't read that file.",
            PrepError::NotAPhoto => "The board pins photos only.",
        }
    }
}

/// Prepare one picked, dropped or pasted file.
pub async fn prepare(file: &File) -> Result<Prepared, PrepError> {
    let name = file.name();
    let head = head(file, 12).await;
    match media::route(&file.type_(), &name, &head) {
        Route::Photo => photo(file).await,
        Route::Video => video(file).await,
        Route::Audio(mime) => audio(file, mime).await,
        Route::File => as_file(file),
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

async fn video(file: &File) -> Result<Prepared, PrepError> {
    within_limit(file)?;
    let mime = media::declared_type(&file.type_(), &file.name());
    let mut prepared = Prepared {
        kind: "video".into(),
        mime: if mime == "video/quicktime" {
            mime
        } else {
            "video/mp4".into()
        },
        size: file.size() as i64,
        file: Some(file.clone().into()),
        ..Prepared::default()
    };
    // Everything below is what the browser can READ of it. A codec it
    // cannot play still goes — the server checks the container, not the
    // codec — just without its size, length or poster.
    let Some((element, url)) = media_element("video", file) else {
        return Ok(prepared);
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
    Ok(prepared)
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

async fn audio(file: &File, mime: &'static str) -> Result<Prepared, PrepError> {
    within_limit(file)?;
    let name = file.name();
    let mut prepared = Prepared {
        kind: "audio".into(),
        mime: mime.into(),
        size: file.size() as i64,
        // A track's title is worth showing; a recording has none.
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
        let prepared = prepare(&file(&source, "wide.png", "image/png"))
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

        let prepared = prepare(&file(&turned, "IMG_0003.JPG", "image/jpeg"))
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
        let prepared = prepare(&file(&source, "small.png", "image/png"))
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
        let refused = prepare(&file(&blob, "IMG_0001.heic", "image/heic")).await;
        assert_eq!(refused, Err(PrepError::Unreadable));
        assert_eq!(PrepError::Unreadable.message(), "Couldn't read that file.");
    }

    /// A GIF animates: its original bytes go, as a file.
    #[wasm_bindgen_test]
    async fn a_gif_goes_as_itself() {
        let parts = js_sys::Array::of1(&JsValue::from_str("GIF89a…"));
        let blob = Blob::new_with_str_sequence(&parts).unwrap();
        let prepared = prepare(&file(&blob, "dance:party.gif", "image/gif"))
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
        let prepared = prepare(&file(&clear, "clear.png", "image/png"))
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
}
