//! Exercises the viewer's Copy action with synthetic server and disk-cache images.

use adw::prelude::*;
use fc_core::protocol::{AttachmentDto, MessageDto};
use fc_logic::attachment_cache::AttachmentCache;
use gtk::gdk;

use super::{WAIT, still};
use crate::attachments::pictures;
use crate::selftest::Harness;
use crate::viewer;

const WARM_ID: i64 = 7402;
const COLD_ID: i64 = 7401;
const EVICTED_ID: i64 = 7403;
const COLD_PATH: &str = "/api/v1/attachments/7401";
const EVICTED_PATH: &str = "/api/v1/attachments/7403";
const SENTINEL: &str = "Clipboard before Copy Image";

pub(super) async fn run(
    h: &Harness,
    chat: i64,
    author: i64,
    blobs: &std::cell::RefCell<std::collections::HashMap<String, Vec<u8>>>,
) {
    let shell = h.shell();
    let Some(connection) = shell.connection() else {
        h.check("copy_image_fixture", false, "no connection");
        return;
    };
    let fake = h.fake_server();
    let say = &shell.say;
    let cold = photo(COLD_ID, "image/jpeg", true);
    let warm = photo(WARM_ID, "image/png", false);
    let evicted = photo(EVICTED_ID, "image/png", true);
    let png = still(30, 18, 0x245e_f0ff, 0x0000_0000, true, "png");
    let small = still(12, 8, 0x8844_22ff, 0x3020_80ff, false, "jpeg");
    let jpeg = oriented_jpeg();
    if png.is_empty() || small.is_empty() || jpeg.is_empty() {
        h.check(
            "copy_image_fixture",
            false,
            "synthetic image encoding failed",
        );
        return;
    }
    blobs
        .borrow_mut()
        .insert("7401/preview".into(), small.clone());
    blobs.borrow_mut().insert(COLD_ID.to_string(), jpeg);
    connection.attachments.remember(&warm, &png);
    connection.attachments.remember(&evicted, &png);
    connection.attachments.remember_preview(&evicted, &small);
    let evicted_key = AttachmentCache::key_for(evicted.id, false);
    let removed = std::fs::remove_file(shell.paths.blobs_dir().join(evicted_key)).is_ok();

    let before_inline = fake.requests().len();
    let Some(preview) = pictures::of_attachment(&cold, true, None).await else {
        h.check("copy_image_fixture", false, "cold preview did not load");
        return;
    };
    h.check(
        "copy_image_cold_load_is_counted_separately",
        preview.width() == 12
            && preview.height() == 8
            && fake.requests()[before_inline..] == ["GET /api/v1/attachments/7401/preview"],
        format!("inline requests: {:?}", &fake.requests()[before_inline..]),
    );

    let mut message = MessageDto::new(8044, chat, author, None, "", "2026-11-28T09:00:00Z");
    message.attachments = Some(vec![cold.clone(), warm.clone(), evicted.clone()]);
    fake.delay_next_response(COLD_PATH, std::time::Duration::from_secs(3));
    viewer::open_message(&message, 0);
    let Some(viewer) = viewer::find((message.id, 0)) else {
        h.check("copy_image_fixture", false, "viewer missing");
        return;
    };
    let clipboard = viewer.window.window.clipboard();
    let preview_only = h
        .wait_until(WAIT, || {
            viewer.showing_picture()
                && !viewer.window.copy.is_sensitive()
                && fake
                    .requests()
                    .iter()
                    .any(|r| r == &format!("GET {COLD_PATH}"))
        })
        .await;
    clipboard.set_text(SENTINEL);
    let before_unavailable = fake.requests().len();
    let window: gtk::Window = viewer.window.window.clone().upcast();
    let key_delivered = !h.input() || (h.focus(&window).await && h.key("ctrl+c").await);
    let sentinel_kept = h
        .with_timeout(WAIT, clipboard.read_text_future())
        .await
        .is_some_and(|value| value.ok().flatten().as_deref() == Some(SENTINEL));
    h.check(
        "copy_image_preview_preserves_clipboard",
        preview_only
            && key_delivered
            && sentinel_kept
            && !viewer.window.copy.is_sensitive()
            && viewer.window.title.subtitle() == say.get("Loading full image…")
            && fake.requests().len() == before_unavailable,
        format!(
            "preview only {preview_only}, sentinel kept {sentinel_kept}, requests {:?}",
            &fake.requests()[before_unavailable..]
        ),
    );

    let before_switch = fake.requests().len();
    let warm_only_on_disk = !pictures::is_kept(&warm, false, None);
    viewer.step(1);
    let disabled_on_turn = !viewer.window.copy.is_sensitive();
    let warm_ready = h
        .wait_until(WAIT, || {
            viewer.window.copy.is_sensitive() && pictures::is_kept(&cold, false, None)
        })
        .await;
    h.check(
        "copy_image_warm_album_page_ignores_late_original",
        warm_only_on_disk
            && disabled_on_turn
            && warm_ready
            && viewer.album().current().id == WARM_ID
            && fake.requests().len() == before_switch,
        format!(
            "disk-only {warm_only_on_disk}, warm and late cold ready {warm_ready}, requests {:?}",
            &fake.requests()[before_switch..]
        ),
    );

    fake.set_down(true);
    let before_copy = fake.requests().len();
    let clicked = if h.input() {
        h.click(
            &window,
            &viewer.window.copy,
            f64::from(viewer.window.copy.width()) / 2.0,
            f64::from(viewer.window.copy.height()) / 2.0,
        )
        .await
    } else {
        viewer.window.copy.emit_clicked();
        true
    };
    let after_copy = fake.requests().len();
    h.screenshot("copy-image-current-album-page").await;
    viewer.window.window.close();
    drop(window);
    drop(viewer);
    let pasted = h
        .with_timeout(WAIT, clipboard.read_texture_future())
        .await
        .and_then(|result| result.ok().flatten());
    let correct = pasted.as_ref().is_some_and(|texture| {
        (texture.width(), texture.height()) == (30, 18) && has_alpha(texture)
    });
    h.check(
        "copy_image_paste_after_dismiss_offline",
        clicked
            && correct
            && viewer::find((message.id, 0)).is_none()
            && before_copy == after_copy
            && after_copy == fake.requests().len(),
        format!("selected transparent PNG {correct}; request counts before Copy {before_copy}, after Copy {after_copy}, after paste {}", fake.requests().len()),
    );
    external_paste(h, &fake).await;

    viewer::open_message(&message, 0);
    if let Some(viewer) = viewer::find((message.id, 0)) {
        let ready = h
            .wait_until(WAIT, || viewer.window.copy.is_sensitive())
            .await;
        clipboard.set_text(SENTINEL);
        let before = fake.requests().len();
        let window: gtk::Window = viewer.window.window.clone().upcast();
        let sent = if h.input() {
            h.focus(&window).await && h.key("ctrl+c").await
        } else {
            viewer.window.copy.emit_clicked();
            true
        };
        let pasted = h
            .with_timeout(WAIT, clipboard.read_texture_future())
            .await
            .and_then(|result| result.ok().flatten());
        let rotated = pasted.is_some_and(|texture| (texture.width(), texture.height()) == (20, 32));
        h.check(
            "copy_image_keyboard_uses_oriented_full_texture",
            ready && sent && rotated && fake.requests().len() == before,
            format!(
                "oriented dimensions 20 × 32: {rotated}; real keys: {}",
                h.input()
            ),
        );
        viewer.window.window.close();
    } else {
        h.check(
            "copy_image_keyboard_uses_oriented_full_texture",
            false,
            "viewer missing",
        );
    }

    let before_miss = fake.requests().len();
    viewer::open_message(&message, 2);
    if let Some(viewer) = viewer::find((message.id, 2)) {
        let unavailable = h
            .wait_until(WAIT, || {
                viewer.window.title.subtitle() == say.get("Full image unavailable")
            })
            .await;
        let load_requests = fake.requests();
        clipboard.set_text(SENTINEL);
        let before = load_requests.len();
        let window: gtk::Window = viewer.window.window.clone().upcast();
        let key_delivered = !h.input() || (h.focus(&window).await && h.key("ctrl+c").await);
        let kept = h
            .with_timeout(WAIT, clipboard.read_text_future())
            .await
            .is_some_and(|value| value.ok().flatten().as_deref() == Some(SENTINEL));
        h.check(
            "copy_image_eviction_has_no_copy_fallback",
            removed
                && unavailable
                && !viewer.window.copy.is_sensitive()
                && load_requests[before_miss..] == [format!("GET {EVICTED_PATH}")]
                && key_delivered
                && kept
                && fake.requests().len() == before,
            format!("evicted {removed}, unavailable {unavailable}, sentinel kept {kept}, viewer load {:?}", &load_requests[before_miss..]),
        );
        viewer.window.window.close();
    } else {
        h.check(
            "copy_image_eviction_has_no_copy_fallback",
            false,
            "viewer missing",
        );
    }
    fake.set_down(false);
}

fn photo(id: i64, mime: &str, preview: bool) -> AttachmentDto {
    AttachmentDto {
        mime: Some(mime.into()),
        has_preview: preview,
        ..AttachmentDto::new(id, "photo")
    }
}

fn has_alpha(texture: &gdk::Texture) -> bool {
    let mut downloader = gdk::TextureDownloader::new(texture);
    downloader.set_format(gdk::MemoryFormat::R8g8b8a8);
    let (bytes, stride) = downloader.download_bytes();
    bytes.get(3) == Some(&255) && bytes.get(17 * stride + 3) == Some(&0)
}

fn oriented_jpeg() -> Vec<u8> {
    let mut jpeg = still(32, 20, 0xe020_20ff, 0x2040_d0ff, false, "jpeg");
    if jpeg.starts_with(&[0xff, 0xd8]) {
        // APP1 EXIF, little-endian TIFF IFD with Orientation=6 (90° clockwise).
        jpeg.splice(2..2, b"\xff\xe1\x00\x22Exif\0\0II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0".iter().copied());
    }
    jpeg
}

/// An optional separate GTK process supplied by the verification environment.
async fn external_paste(h: &Harness, fake: &super::super::fake_server::FakeServer) {
    let Ok(reader) = std::env::var("FC_SELFTEST_IMAGE_RECEIVER") else {
        return;
    };
    let before = fake.requests().len();
    let path = h.out().join("copy-image-external.png");
    let path = path.to_string_lossy();
    let (ok, detail) = h
        .with_timeout(WAIT, h.run(&[&reader, &path]))
        .await
        .unwrap_or((false, "external receiver timed out".into()));
    h.check(
        "copy_image_external_paste_after_dismiss_offline",
        ok && fake.requests().len() == before,
        format!("{detail}; extra requests {:?}", &fake.requests()[before..]),
    );
}
