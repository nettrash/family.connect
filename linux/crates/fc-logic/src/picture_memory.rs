//! How big a photo tile's picture is decoded, and how many decoded pictures a view keeps
//! (`PictureMemory.cs`: `PictureDecode` and `RecentPictures`).

use std::collections::{HashMap, VecDeque};

/// The largest box any tile of a picture is drawn in, in effective pixels: a lone tile's longest
/// side (`MediaText.TileMax`, `fc_text::media::TILE_MAX`).
pub const TILE_BOX: f64 = fc_text::media::TILE_MAX;

/// How big a photo tile's picture is decoded: near the size it is drawn at, not the size the
/// camera took it at.
///
/// ONLY A PHOTO WITH NO SMALL COPY IS DECODED WHOLE OTHERWISE. A tile draws an attachment's
/// preview when it has one — already small — and the photo's own bytes when it does not
/// (`AttachmentFiles::source_for`), and a twelve megapixel original decoded for a 320-point tile
/// is forty-eight megabytes of pixels nobody sees, kept for as long as the view keeps the picture.
///
/// NEVER BLURRIER THAN BEFORE, WHICHEVER WAY UP IT IS. A decoder is told one side, and which side
/// is "width" depends on an EXIF turn the attachment's own numbers may or may not include. So the
/// width asked for is the box times the picture's long side over its short one: whichever way the
/// decoder holds it, both sides come out at least the box — the most any tile of it is drawn at, a
/// lone tile's 320 either way or an album cell's 156 — and the tile, which crops to fill, never
/// stretches a picture smaller than itself. Where that is no smaller than the picture, or its size
/// is not known, it is decoded whole, as it always was.
///
/// The width, in effective pixels, to decode a photo of `width` × `height` at for a tile on a
/// screen of `raster_scale` — or `None` to decode it whole. `box_side` is [`TILE_BOX`] in the
/// C#'s default.
pub fn tile_width(
    width: Option<i32>,
    height: Option<i32>,
    raster_scale: f64,
    box_side: f64,
) -> Option<i32> {
    let (width, height) = match (width, height) {
        (Some(width), Some(height)) if width > 0 && height > 0 && box_side > 0.0 => (width, height),
        _ => return None,
    };
    let long_side = f64::from(width.max(height));
    let short_side = f64::from(width.min(height));
    let asked = (box_side * long_side / short_side).ceil();
    // Effective pixels become device pixels at the screen's scale; a decode that would not come
    // out smaller than the picture's SHORT side — whichever side the decoder calls width — saves
    // nothing and could scale up. `Math.Max(rasterScale, 1)` — and .NET's `Max` keeps a NaN, which
    // then compares false and decodes whole; Rust's `f64::max` would drop it, so it is spelled out.
    let scale = if raster_scale.is_nan() || raster_scale > 1.0 {
        raster_scale
    } else {
        1.0
    };
    if asked * scale < short_side {
        Some(asked as i32)
    } else {
        None
    }
}

/// Decoded pictures a view keeps so a redraw does not decode them again — at most so many, the
/// least recently drawn going first. The window's thread's.
///
/// A picture let go here is still drawn wherever an image holds it — an image keeps its own
/// source — and is decoded again only if it is drawn anew. Without a ceiling the view kept every
/// photo, poster and tile it had ever drawn for as long as it lived.
#[derive(Debug, Clone)]
pub struct RecentPictures<P> {
    most: usize,
    kept: HashMap<String, P>,
    /// Most recently drawn first.
    order: VecDeque<String>,
}

impl<P: Clone> RecentPictures<P> {
    /// At most `most` pictures (one, whatever is asked below that).
    pub fn new(most: usize) -> Self {
        RecentPictures {
            most,
            kept: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    pub fn count(&self) -> usize {
        self.kept.len()
    }

    /// The picture kept under this key, which now counts as the most recently drawn.
    pub fn try_get(&mut self, key: &str) -> Option<P> {
        let picture = self.kept.get(key)?.clone();
        self.touch(key);
        Some(picture)
    }

    /// Whether a picture is kept under this key. Asking is not drawing.
    pub fn contains_key(&self, key: &str) -> bool {
        self.kept.contains_key(key)
    }

    /// Keep a picture, the newest; past the ceiling the least recently drawn goes.
    pub fn insert(&mut self, key: &str, picture: P) {
        if self.kept.insert(key.to_string(), picture).is_some() {
            self.order.retain(|held| held != key);
        }
        self.order.push_front(key.to_string());
        while self.kept.len() > self.most.max(1) {
            let Some(last) = self.order.pop_back() else {
                break;
            };
            self.kept.remove(&last);
        }
    }

    pub fn clear(&mut self) {
        self.kept.clear();
        self.order.clear();
    }

    fn touch(&mut self, key: &str) {
        if let Some(at) = self.order.iter().position(|held| held == key)
            && let Some(held) = self.order.remove(at)
        {
            self.order.push_front(held);
        }
    }
}
