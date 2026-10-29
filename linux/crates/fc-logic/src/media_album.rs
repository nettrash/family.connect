//! A message's photos and videos at full size, one at a time (`MediaAlbum.cs`; web
//! `views/viewer.rs`, the Mac's `MacAttachmentViewer`): paging that stops at the ends, a photo's
//! zoom from 1× to 6×, and the words over it. The shell's viewer window (shell-design §8.2) draws
//! it.
//!
//! A page turn is also a zoom reset: the next picture is a different shape, and a 3× window into it
//! would show a corner of nothing. Save hands over the ORIGINAL, never the preview a bubble draws —
//! that is the window's business, not this.

use fc_core::protocol::{AttachmentDto, MessageDto};
use fc_core::text::string_catalog::StringCatalog;
use fc_text::media;

/// What a key does in the viewer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ViewerKey {
    None,
    Close,
    Previous,
    Next,
}

/// An album with nothing in it (the C#'s `ArgumentException` on `items`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmptyAlbum;

impl std::fmt::Display for EmptyAlbum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("an album needs something in it")
    }
}

impl std::error::Error for EmptyAlbum {}

/// The viewer's album.
#[derive(Debug, Clone, PartialEq)]
pub struct MediaAlbum {
    items: Vec<AttachmentDto>,
    index: usize,
    zoom: f64,
    round: bool,
    animation: bool,
}

impl MediaAlbum {
    /// The furthest a photo zooms.
    pub const MAX_ZOOM: f64 = 6.0;

    /// One step of the zoom buttons.
    pub const ZOOM_FACTOR: f64 = 1.25;

    /// An album of `items` opened at `index` (held inside it).
    ///
    /// `round`: opened onto a VIDEO MESSAGE — the attachment `MessageDto::round_video` found, S5.1's
    /// one test, which only the whole message can answer. Taken only for a lone video carrying the
    /// flag; an album opened from a message's tiles is never one, whatever its attachments carry.
    pub fn new(
        items: Vec<AttachmentDto>,
        index: i64,
        round: bool,
    ) -> Result<MediaAlbum, EmptyAlbum> {
        if items.is_empty() {
            return Err(EmptyAlbum);
        }
        let index = index.clamp(0, items.len() as i64 - 1) as usize;
        let lone_video = |flag: fn(&AttachmentDto) -> bool| matches!(items.as_slice(), [only] if only.kind == "video" && flag(only));
        let round = round && lone_video(|item| item.round);
        let animation = !round && lone_video(|item| item.animation);
        Ok(MediaAlbum {
            items,
            index,
            zoom: 1.0,
            round,
            animation,
        })
    }

    /// The photos and videos a message carries, in the order they were sent — what its tiles open
    /// onto.
    pub fn of_message(message: &MessageDto) -> Vec<AttachmentDto> {
        MediaAlbum::of(message.media())
    }

    /// The photos and videos among `media`, in order.
    pub fn of(media: &[AttachmentDto]) -> Vec<AttachmentDto> {
        media
            .iter()
            .filter(|attachment| media::is_media(&attachment.kind))
            .cloned()
            .collect()
    }

    pub fn items(&self) -> &[AttachmentDto] {
        &self.items
    }

    pub fn index(&self) -> usize {
        self.index
    }

    pub fn count(&self) -> usize {
        self.items.len()
    }

    pub fn current(&self) -> &AttachmentDto {
        &self.items[self.index]
    }

    pub fn is_video(&self) -> bool {
        self.current().kind == "video"
    }

    /// A video message, shown in its circle and called one (S5.1); everything else is the ordinary
    /// item it is.
    pub fn is_round(&self) -> bool {
        self.round
    }

    /// Opened onto an ANIMATION (docs/protocol.md, "Animations"): one video carrying the flag,
    /// alone — shown larger, playing, muted and looping, with no player controls.
    pub fn is_animation(&self) -> bool {
        self.animation
    }

    pub fn has_previous(&self) -> bool {
        self.index > 0
    }

    pub fn has_next(&self) -> bool {
        self.index + 1 < self.count()
    }

    /// 1 = the whole picture fits the window.
    pub fn zoom(&self) -> f64 {
        self.zoom
    }

    /// One page in `direction` (-1 or +1), stopping at the ends. Whether it moved.
    pub fn step(&mut self, direction: i32) -> bool {
        let next = self.index as i64 + i64::from(direction.signum());
        if next < 0 || next >= self.count() as i64 || next == self.index as i64 {
            return false;
        }
        self.index = next as usize;
        self.zoom = 1.0;
        true
    }

    /// The next zoom after a step in `direction` (+1 in, -1 out), clamped to 1–6.
    pub fn zoom_step(scale: f64, direction: f64) -> f64 {
        let next = if direction > 0.0 {
            scale * MediaAlbum::ZOOM_FACTOR
        } else {
            scale / MediaAlbum::ZOOM_FACTOR
        };
        clamp_zoom(next)
    }

    pub fn zoom_in(&mut self) {
        self.zoom = MediaAlbum::zoom_step(self.zoom, 1.0);
    }

    pub fn zoom_out(&mut self) {
        self.zoom = MediaAlbum::zoom_step(self.zoom, -1.0);
    }

    /// A double click: back to the whole picture when zoomed at all, 2× when not.
    pub fn toggle_zoom(&mut self) {
        self.zoom = if self.zoom > 1.0 { 1.0 } else { 2.0 };
    }

    /// A zoom the window arrived at by itself — a pinch (`gtk::GestureZoom`), a ctrl-scroll — kept
    /// within 1–6.
    pub fn zoomed_to(&mut self, scale: f64) {
        self.zoom = clamp_zoom(scale);
    }

    pub fn can_zoom_in(&self) -> bool {
        !self.is_video() && self.zoom < MediaAlbum::MAX_ZOOM
    }

    pub fn can_zoom_out(&self) -> bool {
        !self.is_video() && self.zoom > 1.0
    }

    /// The zoom as the window shows it: "125%". Rounded half to even, as .NET's `Math.Round`; the
    /// C# took a culture, but its `0` format writes ASCII digits with no separator in every one,
    /// so there is nothing for a culture to change.
    pub fn zoom_text(&self) -> String {
        format!("{}%", (self.zoom * 100.0).round_ties_even() as i64)
    }

    /// What the item is called — its name, or its kind when it has none.
    pub fn title(&self, say: &dyn StringCatalog) -> String {
        let current = self.current();
        if current.sticker {
            // A sticker is a photo on the wire and a sticker to the person looking at it.
            say.get("Sticker").to_string()
        } else if self.round {
            // And a circle is a video on the wire and a video message to the person watching it —
            // only when S5.1's test said so of the whole message: a flagged photo, or a flagged
            // video among several, is the ordinary item it otherwise is.
            say.get("Video message").to_string()
        } else if self.animation {
            // And an animation is a video on the wire and a GIF to the person looking at it.
            say.get("GIF").to_string()
        } else {
            media::display_name(&current.kind, current.name.as_deref()).into_owned()
        }
    }

    /// "2 of 5" while there is somewhere to page, and nothing when there is not.
    pub fn position(&self, say: &dyn StringCatalog) -> Option<String> {
        (self.count() > 1).then(|| say.format("%lld of %lld", &[&(self.index + 1), &self.count()]))
    }

    /// Esc closes; ← and → page, and do nothing at the ends. `key` is the key's name as GDK names
    /// it (`gdk::Key::name`: "Escape", "Left", "Right") — the same names as Windows' `VirtualKey`.
    pub fn key(&self, key: &str) -> ViewerKey {
        match key {
            "Escape" => ViewerKey::Close,
            "Left" if self.has_previous() => ViewerKey::Previous,
            "Right" if self.has_next() => ViewerKey::Next,
            _ => ViewerKey::None,
        }
    }
}

/// `Math.Clamp(scale, 1, MaxZoom)` — which answers NaN for NaN, as `f64::clamp` does.
fn clamp_zoom(scale: f64) -> f64 {
    scale.clamp(1.0, MediaAlbum::MAX_ZOOM)
}
