//! `ConversationSlots` (FROZEN, W.3) — how P2 and P3 draw inside P1's bubble.
//!
//! P1 builds a bubble and asks the shell for each block that is not words:
//! `shell.slots().tile(…)`, `.voice(…)`, `.circle(…)`, `.sticker(…)`, `.rich(…)`, `.rich_inline(…)`,
//! `.link_card(…)`, `.location(…)` (`Shell::slots`, which sends tile, sticker, rich, link card and
//! location to P2's installed implementation and voice and circle to P3's). Every method has a
//! default — the block's documented fallback — so P2 and P3 each override only their own, and P1
//! works against S0 before either lands.
//!
//! **Contract, for every method**: called on the main thread while the bubble is built; returns at
//! once with a widget of its FINAL size (from metadata — a row never changes height when its
//! picture lands, §8.1); fetches what it needs itself (through the connection's
//! `AttachmentCache`, never on the click of another widget); stops whatever it plays when the
//! widget is unmapped (scrolled out, chat left, window hidden — §10.4); draws nothing it was not
//! asked for; never logs a name, a file name or words.

use std::rc::Rc;

use fc_core::protocol::{AttachmentDto, MessageDto};
use fc_logic::bubble_body::BodyRichBlock;
use fc_logic::rich_drawing::RichLook;
use gtk::prelude::*;

/// How a tile is to be drawn (P1 decides it from `MediaText::tile_size` and the grid).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TileLook {
    /// The tile's size in logical pixels — its final size.
    pub width: f64,
    pub height: f64,
    /// In the reader's own balloon (the accent behind it).
    pub own: bool,
    /// "+n" over the fourth tile of a grid that has more (0: none).
    pub more: u32,
}

/// What a rich block or an inline formula became (§9.2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RichDrawn {
    /// Drawn: its CSS size and its baseline from the top (an inline formula sits on the text's
    /// baseline), logical pixels.
    Drawn {
        width: f64,
        height: f64,
        baseline: f64,
    },
    /// It could not be drawn: the widget shows the source for good (Windows' `RichDrawn::Failure`).
    Failed,
}

/// A rich block in a bubble (`fn rich(&self, block) -> RichPicture`).
pub struct RichPicture {
    /// The widget to place: the block's `shown` text in the monospaced face until the drawing
    /// lands, then the drawing — or the text for good when it cannot be drawn. It lays itself out
    /// again when the drawing lands; nothing need be told. (A `landed` event the skeleton froze
    /// here had no caller and was taken out at integration.)
    pub widget: gtk::Widget,
    /// What it has become so far: `None` while it is being drawn.
    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "the rich self-test reads it")
    )]
    pub drawn: Rc<std::cell::Cell<Option<RichDrawn>>>,
}

impl RichPicture {
    /// The fallback: `text` in the monospaced face, selectable, wrapping — and already failed.
    pub fn source(text: &str) -> RichPicture {
        let label = gtk::Label::builder()
            .label(text)
            .selectable(true)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .xalign(0.0)
            .css_classes(["monospace"])
            .build();
        RichPicture {
            widget: label.upcast(),
            drawn: Rc::new(std::cell::Cell::new(Some(RichDrawn::Failed))),
        }
    }
}

/// How P2 and P3 draw inside P1's bubble. See the module's comment for the contract.
pub trait ConversationSlots {
    /// A photo, a video or a file of `message` (P2, §8.1): the preview or the photo at `look`'s
    /// size, a video's poster with its length, a file row with Save… and Open. A click opens the
    /// viewer window (P2) for that message and index.
    fn tile(
        &self,
        message: &MessageDto,
        attachment: &AttachmentDto,
        look: &TileLook,
    ) -> gtk::Widget {
        let _ = message;
        icon_box(icon_for(attachment), look.width, look.height)
    }

    /// A voice note's body (P3, §10.3): the play button, the waveform that is also the seek
    /// slider, the time, the speed chip, the unplayed dot. `own`: in the reader's own balloon.
    fn voice(&self, message: &MessageDto, attachment: &AttachmentDto, own: bool) -> gtk::Widget {
        let _ = (message, attachment, own);
        icon_box("audio-x-generic-symbolic", 240.0, 44.0)
    }

    /// A received video message, in place (P3, §10.4): the circle (`RoundLook`'s diameter), the
    /// play disc, the ring outside the edge, the capsule, the expand control.
    fn circle(&self, message: &MessageDto, attachment: &AttachmentDto) -> gtk::Widget {
        let _ = (message, attachment);
        let disc = icon_box("camera-video-symbolic", 240.0, 240.0);
        disc.add_css_class("fc-round");
        disc.set_overflow(gtk::Overflow::Hidden);
        disc
    }

    /// A sticker (P2, §8.4): no balloon, fitted in `StickerLook::BOX` from metadata, animated where
    /// this machine can (frame zero under Reduce Motion).
    fn sticker(&self, message: &MessageDto, attachment: &AttachmentDto) -> gtk::Widget {
        let _ = message;
        let (width, height) =
            fc_logic::sticker_look::StickerLook::fit(attachment.width, attachment.height);
        icon_box("image-x-generic-symbolic", width, height)
    }

    /// A formula, diagram or graph standing on its own (P2, §9): drawn through the rich renderer.
    fn rich(&self, block: &BodyRichBlock, look: &RichLook) -> RichPicture {
        let _ = look;
        RichPicture::source(&block.shown)
    }

    /// An inline formula inside a text block (P2, §9): `typed` is what the reader typed,
    /// delimiters and all (shown until drawn); `source` what is drawn.
    fn rich_inline(&self, source: &str, typed: &str, look: &RichLook) -> RichPicture {
        let _ = (source, look);
        RichPicture::source(typed)
    }

    /// The link preview card under a body (P2, §5.2), or `None` — none while "Link Previews" is
    /// off, for a lookup or a streaming assistant answer, or when the page gave nothing. The card
    /// may appear later: return a container and fill it when `LinkPreviews` lands.
    fn link_card(&self, message: &MessageDto, url: &str, own: bool) -> Option<gtk::Widget> {
        let _ = (message, url, own);
        None
    }

    /// A shared location (P2, §5.2): the OpenStreetMap tiles with the pin and attribution, or the
    /// pin, name and link alone while "Map Previews" is off.
    fn location(&self, message: &MessageDto, attachment: &AttachmentDto) -> gtk::Widget {
        let _ = (message, attachment);
        icon_box("mark-location-symbolic", 240.0, 160.0)
    }
}

/// S0's stand-in: every block's fallback.
#[derive(Debug, Default)]
pub struct PlainSlots;

impl ConversationSlots for PlainSlots {}

/// A box of `width` × `height` with a dimmed symbolic icon in its middle.
fn icon_box(icon: &str, width: f64, height: f64) -> gtk::Widget {
    let image = gtk::Image::builder()
        .icon_name(icon)
        .pixel_size(32)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .hexpand(true)
        .vexpand(true)
        .build();
    let frame = gtk::Box::builder()
        .width_request(logical(width))
        .height_request(logical(height))
        .css_classes(["fc-placeholder", "card"])
        .build();
    frame.append(&image);
    frame.upcast()
}

/// A logical size as GTK's integer request (at least 1).
fn logical(size: f64) -> i32 {
    if size.is_finite() {
        size.round().clamp(1.0, 10_000.0) as i32
    } else {
        1
    }
}

/// The symbolic icon of an attachment's kind.
fn icon_for(attachment: &AttachmentDto) -> &'static str {
    match attachment.kind.as_str() {
        "photo" | "image" => "image-x-generic-symbolic",
        "video" => "video-x-generic-symbolic",
        "audio" => "audio-x-generic-symbolic",
        _ => "text-x-generic-symbolic",
    }
}
