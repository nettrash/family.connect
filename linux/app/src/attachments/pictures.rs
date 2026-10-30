//! Pictures from attachments (shell-design §8.1): bytes come ONCE through the connection's
//! `AttachmentCache` (whole or not at all, into `$XDG_CACHE_HOME/family-connect/blobs`), are
//! decoded near the size they are drawn at (`picture_memory::tile_width` → gdk-pixbuf's
//! `from_stream_at_scale`), and are kept decoded in a bounded memory (`RecentPictures`, the least
//! recently drawn going first) so a rebuilt conversation does not decode them again.
//!
//! **One fetch per picture however many ask**: two tiles of the same photo (a bubble and its
//! thread) share one download and one decode — the asks in flight are joined.
//!
//! **Decoding** is gdk-pixbuf (glycin underneath on Ubuntu 26.04) on its own thread
//! (`from_stream_*_future`), its EXIF turn applied, then copied into a `gdk::MemoryTexture`;
//! anything gdk-pixbuf refuses is given to `gdk::Texture::from_bytes`, which reads what GTK reads
//! itself. A picture nothing on this machine decodes stays a labelled tile.
//!
//! **Drawn at an exact logical size**: a texture's intrinsic size is its pixel size, so a 600-px
//! preview in a 320-pt tile (or a 2× drawing) would ask for 600 pt. [`cover`] and [`fit`] render the
//! texture into a paintable whose intrinsic size IS the tile's, cropped to fill or fitted whole —
//! crisp at any scale, and a row never changes height when its picture lands.
//!
//! Logs carry ids and error codes only.

use std::cell::RefCell;
use std::collections::HashMap;

use adw::prelude::*;
use fc_core::protocol::AttachmentDto;
use fc_logic::attachment_cache::AttachmentCache;
use fc_logic::picture_memory::RecentPictures;
use futures_util::FutureExt;
use futures_util::future::{LocalBoxFuture, Shared};
use gtk::{gdk, gdk_pixbuf, graphene, gsk};

use crate::shell::Shell;

/// How many decoded pictures the process keeps (tiles, posters, viewer pictures).
const KEPT: usize = 160;

type Loading = Shared<LocalBoxFuture<'static, Option<gdk::Texture>>>;

thread_local! {
    static RECENT: RefCell<RecentPictures<gdk::Texture>> = RefCell::new(RecentPictures::new(KEPT));
    static LOADING: RefCell<HashMap<String, Loading>> = RefCell::new(HashMap::new());
}

/// The decoded picture of `attachment`: its preview when `preview` and it has one, else its own
/// bytes; decoded `decode_width` wide (aspect kept) when given, whole otherwise. `None` when there
/// is no connection, the download failed (not cached: the next look asks again) or nothing here
/// decodes it.
pub async fn of_attachment(
    attachment: &AttachmentDto,
    preview: bool,
    decode_width: Option<i32>,
) -> Option<gdk::Texture> {
    let small = preview && attachment.has_preview;
    let key = format!(
        "{}@{}",
        AttachmentCache::key_for(attachment.id, small),
        decode_width.unwrap_or(0)
    );
    if let Some(held) = RECENT.with(|recent| recent.borrow_mut().try_get(&key)) {
        return Some(held);
    }
    let joined = LOADING.with(|loading| loading.borrow().get(&key).cloned());
    let loading = match joined {
        Some(loading) => loading,
        None => {
            let attachment = attachment.clone();
            let (key_for_task, id) = (key.clone(), attachment.id);
            let task: LocalBoxFuture<'static, Option<gdk::Texture>> = Box::pin(async move {
                let connection = Shell::try_get()?.connection()?;
                let bytes = match connection.attachments.bytes(&attachment, preview).await {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        crate::fc_info!("picture {id}: not fetched ({})", error.code);
                        return None;
                    }
                };
                let texture = decode(bytes, decode_width).await;
                if texture.is_none() {
                    crate::fc_info!("picture {id}: nothing here decodes it");
                }
                if let Some(texture) = &texture {
                    RECENT
                        .with(|recent| recent.borrow_mut().insert(&key_for_task, texture.clone()));
                }
                texture
            });
            let shared = task.shared();
            LOADING.with(|loading| loading.borrow_mut().insert(key.clone(), shared.clone()));
            shared
        }
    };
    let texture = loading.await;
    LOADING.with(|loading| loading.borrow_mut().remove(&key));
    texture
}

/// Whether the decoded picture is already in memory (asking is not drawing).
pub fn is_kept(attachment: &AttachmentDto, preview: bool, decode_width: Option<i32>) -> bool {
    let small = preview && attachment.has_preview;
    let key = format!(
        "{}@{}",
        AttachmentCache::key_for(attachment.id, small),
        decode_width.unwrap_or(0)
    );
    RECENT.with(|recent| recent.borrow().contains_key(&key))
}

/// Decodes a still picture: `width` wide with its aspect kept when given, whole otherwise; its EXIF
/// turn applied. `None` when nothing here reads it.
pub async fn decode(bytes: Vec<u8>, width: Option<i32>) -> Option<gdk::Texture> {
    let bytes = glib::Bytes::from_owned(recognisable(bytes));
    let stream = gio::MemoryInputStream::from_bytes(&bytes);
    let decoded = match width.filter(|width| *width > 0) {
        Some(width) => {
            gdk_pixbuf::Pixbuf::from_stream_at_scale_future(&stream, width, -1, true).await
        }
        None => gdk_pixbuf::Pixbuf::from_stream_future(&stream).await,
    };
    match decoded {
        Ok(pixbuf) => {
            let turned = pixbuf.apply_embedded_orientation().unwrap_or(pixbuf);
            Some(texture_of(&turned))
        }
        // GTK's own loaders (and glycin through them) read a few things gdk-pixbuf's do not.
        Err(_) => gdk::Texture::from_bytes(&bytes).ok(),
    }
}

/// Decodes a still picture fitted within `edge` × `edge` (aspect kept, never scaled up past its
/// own size by gdk-pixbuf's rule) — an avatar's face.
pub async fn decode_square(bytes: Vec<u8>, edge: i32) -> Option<gdk::Texture> {
    let bytes = glib::Bytes::from_owned(recognisable(bytes));
    let stream = gio::MemoryInputStream::from_bytes(&bytes);
    match gdk_pixbuf::Pixbuf::from_stream_at_scale_future(&stream, edge, edge, true).await {
        Ok(pixbuf) => {
            let turned = pixbuf.apply_embedded_orientation().unwrap_or(pixbuf);
            Some(texture_of(&turned))
        }
        Err(_) => gdk::Texture::from_bytes(&bytes).ok(),
    }
}

/// `bytes` as gdk-pixbuf will RECOGNISE them. It does not read its loaders' own signatures: GIO
/// sniffs the first bytes against shared-mime-info, and a WebP's bytes 4–7 are its RIFF size — so a
/// WebP whose size happens to spell another format's magic there is that format. Measured: a
/// sticker frame rewrapped as a still of 27,458 bytes (`0x6B42`, "Bk\0\0") is WiiWare's
/// `application/x-wii-wad`, "Unrecognized image file format", and the whole animation fell back to
/// its first frame; 25,193 ("ib") and 29,513 ("Is") bytes are the same.
///
/// Such a WebP is given a size of its own: one unknown chunk at its end, which every WebP reader
/// skips in the extended format — a simple file (a bare `VP8 ` or `VP8L`) is made extended first,
/// with the `VP8X` its bitstream's own size calls for. Anything else, and a WebP GIO already reads
/// as one, is returned as it is.
pub fn recognisable(bytes: Vec<u8>) -> Vec<u8> {
    let mut bytes = bytes;
    for _ in 0..4 {
        if !is_webp(&bytes) || sniffs_as_webp(&bytes) {
            return bytes;
        }
        match padded(&bytes) {
            Some(longer) => bytes = longer,
            None => return bytes,
        }
    }
    bytes
}

fn is_webp(bytes: &[u8]) -> bool {
    bytes.len() >= 20 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP"
}

/// What GIO makes of the first bytes, as gdk-pixbuf asks it.
fn sniffs_as_webp(bytes: &[u8]) -> bool {
    let (kind, _) = gio::content_type_guess(None::<&str>, &bytes[..bytes.len().min(4096)]);
    gio::content_type_is_a(&kind, "image/webp")
}

/// The same picture, ten bytes longer: an empty unknown chunk (`FCPD`) appended — after a `VP8X`
/// put in front of a simple file's bitstream. `None` for a simple file whose size cannot be read.
fn padded(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut body = b"WEBP".to_vec();
    if &bytes[12..16] == b"VP8X" {
        body.extend_from_slice(&bytes[12..]);
    } else {
        let (width, height, alpha) = simple_size(bytes)?;
        let mut extended = [0u8; 10];
        extended[0] = if alpha { 0x10 } else { 0 };
        extended[4..7].copy_from_slice(&(width - 1).to_le_bytes()[..3]);
        extended[7..10].copy_from_slice(&(height - 1).to_le_bytes()[..3]);
        body.extend_from_slice(b"VP8X");
        body.extend_from_slice(&10u32.to_le_bytes());
        body.extend_from_slice(&extended);
        body.extend_from_slice(&bytes[12..]);
    }
    if !body.len().is_multiple_of(2) {
        body.push(0);
    }
    body.extend_from_slice(b"FCPD");
    body.extend_from_slice(&2u32.to_le_bytes());
    body.extend_from_slice(&[0, 0]);
    let mut file = b"RIFF".to_vec();
    file.extend_from_slice(&u32::try_from(body.len()).ok()?.to_le_bytes());
    file.extend_from_slice(&body);
    Some(file)
}

/// A simple WebP's size from its bitstream, and whether it may hold alpha (a `VP8L` may; a bare
/// `VP8 ` never does).
fn simple_size(bytes: &[u8]) -> Option<(u32, u32, bool)> {
    let payload = bytes.get(20..)?;
    match bytes.get(12..16)? {
        b"VP8 " => {
            // A key frame: three bytes of tag, the start code, then 14-bit width and height.
            if payload.get(3..6)? != [0x9d, 0x01, 0x2a] {
                return None;
            }
            let width = u32::from(u16::from_le_bytes([payload[6], payload[7]]) & 0x3fff);
            let height =
                u32::from(u16::from_le_bytes([*payload.get(8)?, *payload.get(9)?]) & 0x3fff);
            (width > 0 && height > 0).then_some((width, height, false))
        }
        b"VP8L" => {
            if *payload.first()? != 0x2f {
                return None;
            }
            let bits = u32::from_le_bytes(payload.get(1..5)?.try_into().ok()?);
            Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1, true))
        }
        _ => None,
    }
}

/// A pixbuf as a texture, without the deprecated `Texture::for_pixbuf`: its rows copied as they
/// are (gdk-pixbuf's alpha is straight, as `R8g8b8a8` is).
pub fn texture_of(pixbuf: &gdk_pixbuf::Pixbuf) -> gdk::Texture {
    let format = if pixbuf.has_alpha() {
        gdk::MemoryFormat::R8g8b8a8
    } else {
        gdk::MemoryFormat::R8g8b8
    };
    let stride = usize::try_from(pixbuf.rowstride()).unwrap_or(0);
    gdk::MemoryTexture::new(
        pixbuf.width(),
        pixbuf.height(),
        format,
        &pixbuf.read_pixel_bytes(),
        stride,
    )
    .upcast()
}

/// `texture` cropped to FILL a `width` × `height` logical box, centred, as a paintable whose
/// intrinsic size is exactly the box (a tile's rule: it crops to fill).
pub fn cover(texture: &gdk::Texture, width: f64, height: f64) -> gdk::Paintable {
    let (tw, th) = (f64::from(texture.width()), f64::from(texture.height()));
    let scale = if tw > 0.0 && th > 0.0 {
        (width / tw).max(height / th)
    } else {
        1.0
    };
    let (dw, dh) = (tw * scale, th * scale);
    let target = graphene::Rect::new(
        ((width - dw) / 2.0) as f32,
        ((height - dh) / 2.0) as f32,
        dw as f32,
        dh as f32,
    );
    painted(texture, width, height, &target, true)
}

/// `texture` fitted WHOLE into a `width` × `height` logical box (a sticker's, a drawing's), as a
/// paintable whose intrinsic size is the box; the picture is centred in it.
pub fn fit(texture: &gdk::Texture, width: f64, height: f64) -> gdk::Paintable {
    let (tw, th) = (f64::from(texture.width()), f64::from(texture.height()));
    let scale = if tw > 0.0 && th > 0.0 {
        (width / tw).min(height / th)
    } else {
        1.0
    };
    let (dw, dh) = (tw * scale, th * scale);
    let target = graphene::Rect::new(
        ((width - dw) / 2.0) as f32,
        ((height - dh) / 2.0) as f32,
        dw as f32,
        dh as f32,
    );
    painted(texture, width, height, &target, false)
}

/// The texture drawn at `target` inside a `width` × `height` box, clipped to it.
fn painted(
    texture: &gdk::Texture,
    width: f64,
    height: f64,
    target: &graphene::Rect,
    clip: bool,
) -> gdk::Paintable {
    let snapshot = gtk::Snapshot::new();
    let bounds = graphene::Rect::new(0.0, 0.0, width as f32, height as f32);
    if clip {
        snapshot.push_clip(&bounds);
    }
    snapshot.append_scaled_texture(texture, gsk::ScalingFilter::Trilinear, target);
    if clip {
        snapshot.pop();
    }
    let size = graphene::Size::new(width as f32, height as f32);
    snapshot
        .to_paintable(Some(&size))
        .unwrap_or_else(|| gdk::Paintable::new_empty(width as i32, height as i32))
}

/// The paintable of an empty box of that size (what a picture shows before anything lands).
pub fn empty(width: f64, height: f64) -> gdk::Paintable {
    gdk::Paintable::new_empty(logical(width), logical(height))
}

/// A logical size as GTK's integer request: rounded, at least 1, at most 10 000.
pub fn logical(size: f64) -> i32 {
    if size.is_finite() {
        size.round().clamp(1.0, 10_000.0) as i32
    } else {
        1
    }
}

/// The decode width a tile of this photo needs (`picture_memory::tile_width`), for a widget whose
/// screen scale is `scale`: only a photo's OWN bytes are decoded small — a preview is small
/// already, and a sticker's frames are its own business.
pub fn tile_decode_width(attachment: &AttachmentDto, original: bool, scale: f64) -> Option<i32> {
    if !original || attachment.kind != "photo" || attachment.sticker {
        return None;
    }
    fc_logic::picture_memory::tile_width(
        attachment.width,
        attachment.height,
        scale,
        fc_logic::picture_memory::TILE_BOX,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real WebP, as gdk-pixbuf writes one (lossless, simple: a bare `VP8L`), or `None` where no
    /// WebP saver is installed.
    fn a_webp() -> Option<Vec<u8>> {
        let pixbuf = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, true, 8, 6, 4)?;
        pixbuf.fill(0x3366_99ff);
        pixbuf.save_to_bufferv("webp", &[]).ok()
    }

    fn riff_size(bytes: &[u8]) -> u32 {
        u32::from_le_bytes(bytes[4..8].try_into().unwrap())
    }

    fn decodes(bytes: Vec<u8>) -> Option<(i32, i32)> {
        let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from_owned(bytes));
        gdk_pixbuf::Pixbuf::from_stream(&stream, gio::Cancellable::NONE)
            .ok()
            .map(|pixbuf| (pixbuf.width(), pixbuf.height()))
    }

    #[test]
    fn a_simple_webp_s_size_is_read_from_its_bitstream() {
        let Some(webp) = a_webp() else {
            return;
        };
        assert_eq!(&webp[12..16], b"VP8L");
        assert_eq!(simple_size(&webp), Some((6, 4, true)));
        // Made extended and padded, it is still the same picture.
        let longer = padded(&webp).expect("padded");
        assert_eq!(&longer[12..16], b"VP8X");
        assert_eq!(riff_size(&longer) as usize, longer.len() - 8);
        assert_eq!(decodes(longer), Some((6, 4)));
    }

    /// THE STICKER THAT STOOD STILL: a WebP of exactly 27,458 bytes after `RIFF` and the size —
    /// "Bk\0\0" at offset 4 — sniffs as WiiWare's bundle and gdk-pixbuf cannot recognise it;
    /// `recognisable` gives it a size of its own and it decodes.
    #[test]
    fn a_webp_whose_size_spells_another_format_is_made_recognisable() {
        let Some(webp) = a_webp() else {
            return;
        };
        let extended = padded(&webp).expect("extended");
        // An unknown chunk sized so that the RIFF size is 0x6B42.
        let wanted = 0x6B42usize;
        let filler = wanted - (extended.len() - 8) - 8;
        let mut body = extended[8..].to_vec();
        body.extend_from_slice(b"JUNK");
        body.extend_from_slice(&u32::try_from(filler).unwrap().to_le_bytes());
        body.extend(std::iter::repeat_n(0u8, filler));
        let mut clash = b"RIFF".to_vec();
        clash.extend_from_slice(&u32::try_from(body.len()).unwrap().to_le_bytes());
        clash.extend_from_slice(&body);
        assert_eq!(&clash[4..8], b"Bk\0\0");
        if sniffs_as_webp(&clash) {
            // This machine's MIME database knows no such bundle: nothing to show here.
            return;
        }
        assert_eq!(
            decodes(clash.clone()),
            None,
            "the clash itself is unreadable"
        );
        let fixed = recognisable(clash.clone());
        assert!(sniffs_as_webp(&fixed));
        assert_eq!(fixed.len(), clash.len() + 10);
        assert_eq!(decodes(fixed), Some((6, 4)));
        // A WebP GIO reads already, and anything that is not a WebP, are left alone.
        assert_eq!(recognisable(webp.clone()), webp);
        assert_eq!(recognisable(b"GIF89a....".to_vec()), b"GIF89a....".to_vec());
    }

    #[test]
    fn a_logical_size_is_a_sane_integer() {
        assert_eq!(logical(320.0), 320);
        assert_eq!(logical(155.6), 156);
        assert_eq!(logical(0.2), 1);
        assert_eq!(logical(f64::NAN), 1);
        assert_eq!(logical(1e9), 10_000);
    }

    #[test]
    fn only_a_photos_own_bytes_are_decoded_small() {
        let mut photo = AttachmentDto::new(1, "photo");
        photo.width = Some(4000);
        photo.height = Some(3000);
        assert!(tile_decode_width(&photo, true, 1.0).is_some());
        assert_eq!(tile_decode_width(&photo, false, 1.0), None);
        let mut sticker = photo.clone();
        sticker.sticker = true;
        assert_eq!(tile_decode_width(&sticker, true, 1.0), None);
        let video = AttachmentDto::new(2, "video");
        assert_eq!(tile_decode_width(&video, true, 1.0), None);
    }
}
