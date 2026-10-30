//! Stickers and animated pictures (shell-design §8.4): a chat sticker is drawn with NO balloon,
//! fitted whole into the one 160-pt box from metadata (`StickerLook::fit`), so its row never
//! changes height when the picture lands — and ANIMATED WHERE THIS MACHINE CAN, frame zero where it
//! cannot or should not (Reduce Motion), both being correct drawings of a sticker.
//!
//! **The frames** come from fc-logic, exactly as for the animations the composer prepares
//! (carry-forward, "Animations"): `animation_prep::layout` walks a GIF, an animated WebP or an APNG,
//! `animation_preparing::frame_source` cuts it into stills for THIS module's [`GdkStills`] decoder
//! (gdk-pixbuf, glycin underneath — which also covers APNG, which gdk-pixbuf would not animate),
//! and `animation_frames::sticker_frames::compose` composites them at the size they are drawn
//! (`StickerAnimation::decode_size`), premultiplied, into `gdk::MemoryTexture`s. Whether animating
//! is worth the memory is `StickerAnimation::worth`; how long each frame stands is
//! `StickerAnimation::clock` (a 0 or 10 ms frame is the browsers' 100 ms).
//!
//! **The shelf** (`StickerShelf`, one per process): decoded stickers by attachment id, at most 64,
//! their frames within a 96 MB budget — past it the sticker drawn longest ago is let go, or made
//! still (frame zero) if an image still shows it. "Nothing here decodes it" is an answer, kept.
//!
//! **One timer moves every sticker** ([`Animator`]): it advances only the stickers that are ON
//! SCREEN — mapped, in a shown window, and inside their scrolled view's visible part — so a sticker
//! scrolled out of the conversation stops (a GTK widget scrolled out of a `ScrolledWindow` stays
//! mapped, so mapping alone is not enough), and one that is unmapped (chat left, window hidden)
//! leaves the timer altogether. It waits for the soonest frame due (`StickerAnimation::tick_after`,
//! 33–250 ms) instead of waking thirty times a second. A sticker joins the animation where it was
//! (`StickerShelf::started`), so a rebuilt conversation does not send them all back to frame zero.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use adw::prelude::*;
use fc_core::protocol::{AttachmentDto, MessageDto};
use fc_core::rt::{Cancel, LocalBoxFuture};
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::animated_pictures::animation_prep;
use fc_logic::animation_frames::sticker_frames;
use fc_logic::animation_preparing::{self, DecodedStill, StillDecoder};
use fc_logic::sticker_look::{StickerAnimation, StickerLook, StickerShelf};
use gtk::{gdk, gdk_pixbuf};

use super::pictures;
use crate::shell::Shell;

/// The decoded frames of one sticker, as paintables at the size they are drawn.
pub struct Frames {
    /// Frame zero first; a still sticker has one.
    paintables: RefCell<Vec<gdk::Paintable>>,
    /// How long each frame stands, ms (empty for a still).
    clock: Vec<i32>,
    /// Made still by the shelf (over budget while shown): frame zero only from now on.
    stilled: Cell<bool>,
}

impl Frames {
    fn still(paintable: gdk::Paintable) -> Rc<Frames> {
        Rc::new(Frames {
            paintables: RefCell::new(vec![paintable]),
            clock: Vec::new(),
            stilled: Cell::new(true),
        })
    }

    fn moving(&self) -> bool {
        !self.stilled.get() && self.paintables.borrow().len() > 1 && self.clock.len() > 1
    }

    fn frame(&self, index: usize) -> Option<gdk::Paintable> {
        let paintables = self.paintables.borrow();
        paintables
            .get(index)
            .or_else(|| paintables.first())
            .cloned()
    }

    /// Frame zero kept, the rest given back.
    fn make_still(&self) {
        self.stilled.set(true);
        self.paintables.borrow_mut().truncate(1);
    }

    fn let_go(&self) {
        self.paintables.borrow_mut().clear();
    }
}

thread_local! {
    static SHELF: RefCell<StickerShelf<Frames>> = RefCell::new(
        StickerShelf::new(StickerAnimation::MAX_HELD_BYTES, StickerAnimation::MAX_KEPT)
            .with_showing(Animator::shows)
            .with_still(|frames: &Rc<Frames>| frames.make_still())
            .with_let_go(|frames: &Rc<Frames>| frames.let_go()),
    );
    static ANIMATOR: RefCell<Animator> = RefCell::new(Animator::default());
}

/// A sticker of `message` in a bubble (`ConversationSlots::sticker`): no balloon, fitted from
/// metadata; a click shows it larger, with the offer to keep it ([`show`]); a double click is the
/// bubble's heart.
pub fn sticker(message: &MessageDto, attachment: &AttachmentDto) -> gtk::Widget {
    let (width, height) = StickerLook::fit(attachment.width, attachment.height);
    let widget = picture(attachment, width, height);
    let say = &Shell::get().say;
    widget.set_accessible_role(gtk::AccessibleRole::Button);
    widget.update_property(&[gtk::accessible::Property::Label(say.get("Sticker"))]);
    widget.set_focusable(true);
    let attachment = attachment.clone();
    let _ = message;
    let anchor = widget.downgrade();
    super::on_click(&widget, move || {
        if let Some(anchor) = anchor.upgrade() {
            show(&anchor, &attachment);
        }
    });
    widget
}

/// A sticker from a chat, shown larger — and, where the family's pack does not hold it, "Add to
/// family stickers" (the Mac's `StickerViewer`; docs/protocol.md, "Tapping one shows it larger").
/// Not the photo viewer, which pages and zooms: a sticker is a small picture with one thing to do
/// about it. Whether the pack holds it is decided from the bytes this device has
/// (`PackModel::holds`); a wrong guess costs nothing — the server answers an add of bytes it
/// already holds with the item that was there.
pub fn show(anchor: &impl IsA<gtk::Widget>, attachment: &AttachmentDto) {
    let shell = Shell::get();
    let say = Rc::clone(&shell.say);
    let dialog = adw::Dialog::builder()
        .title(say.get("Sticker"))
        .content_width(420)
        .content_height(460)
        .build();
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    let column = gtk::Box::new(gtk::Orientation::Vertical, 20);
    column.set_margin_top(24);
    column.set_margin_bottom(24);
    column.set_margin_start(24);
    column.set_margin_end(24);
    let (width, height) = StickerLook::fit(attachment.width, attachment.height);
    let scale = (320.0 / width.max(height).max(1.0)).min(2.5);
    let big = picture(attachment, width * scale, height * scale);
    big.set_halign(gtk::Align::Center);
    big.update_property(&[gtk::accessible::Property::Label(say.get("Sticker"))]);
    column.append(&big);
    let footer = gtk::Box::new(gtk::Orientation::Vertical, 10);
    footer.set_halign(gtk::Align::Center);
    column.append(&footer);
    view.set_content(Some(&column));
    dialog.set_child(Some(&view));
    dialog.present(Some(anchor));

    let Some(connection) = shell.connection() else {
        return;
    };
    let model = Rc::clone(&connection.stickers);
    // A server with no packs has nowhere to add to.
    if !super::pack::offered(&model) {
        return;
    }
    let (attachment, footer) = (attachment.clone(), footer.clone());
    glib::spawn_future_local(async move {
        if model.holds(&attachment).await.unwrap_or(true) {
            return;
        }
        offer(&footer, &model, &attachment);
    });
}

/// The footer's "Add to family stickers", and what became of it.
fn offer(footer: &gtk::Box, model: &Rc<fc_logic::pack::PackModel>, attachment: &AttachmentDto) {
    let say = Rc::clone(&Shell::get().say);
    while let Some(child) = footer.first_child() {
        footer.remove(&child);
    }
    let add = gtk::Button::builder()
        .label(say.get("Add to family stickers"))
        .css_classes(["pill", "suggested-action"])
        .build();
    footer.append(&add);
    let (footer, model, attachment) = (footer.clone(), Rc::clone(model), attachment.clone());
    add.connect_clicked(move |_| {
        while let Some(child) = footer.first_child() {
            footer.remove(&child);
        }
        footer.append(&adw::Spinner::new());
        let (footer, model, attachment, say) = (
            footer.clone(),
            Rc::clone(&model),
            attachment.clone(),
            Rc::clone(&say),
        );
        glib::spawn_future_local(async move {
            // The message's own bytes, uploaded again as they are: they already ARE a sticker.
            let answer = model.add_from_message(&attachment, None).await;
            let failure = match answer {
                Ok(Ok(_)) => None,
                Ok(Err(error)) => {
                    crate::fc_info!("pack: adding a sent sticker refused ({})", error.code);
                    Some(fc_logic::pack::pack_text::sentence(&error, say.as_ref()))
                }
                Err(error) => {
                    crate::fc_warn!(
                        "pack: the cache refused ({})",
                        crate::services::log::store_error(&error)
                    );
                    Some(say.get("Couldn't save that. Try again.").to_string())
                }
            };
            match failure {
                None => {
                    while let Some(child) = footer.first_child() {
                        footer.remove(&child);
                    }
                    let done = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                    done.append(&gtk::Image::from_icon_name("object-select-symbolic"));
                    done.append(&gtk::Label::new(Some(say.get("Added to family stickers"))));
                    done.add_css_class("dim-label");
                    footer.append(&done);
                }
                Some(sentence) => {
                    offer(&footer, &model, &attachment);
                    footer.prepend(
                        &gtk::Label::builder()
                            .label(&sentence)
                            .wrap(true)
                            .justify(gtk::Justification::Center)
                            .max_width_chars(36)
                            .css_classes(["error"])
                            .build(),
                    );
                }
            }
        });
    });
}

/// A sticker's picture at `width` × `height` logical pixels — a bubble's, a panel cell's, a pack
/// cell's — loaded, decoded and animated as the module's comment says.
pub fn picture(attachment: &AttachmentDto, width: f64, height: f64) -> gtk::Widget {
    let picture = gtk::Picture::builder()
        .can_shrink(true)
        .content_fit(gtk::ContentFit::Fill)
        .paintable(&pictures::empty(width, height))
        .width_request(pictures::logical(width))
        .height_request(pictures::logical(height))
        .halign(gtk::Align::Start)
        .valign(gtk::Align::Start)
        .build();
    let view = Rc::new(View {
        picture: picture.downgrade(),
        frames: RefCell::new(None),
        id: attachment.id,
        started: Cell::new(0),
        shown: Cell::new(usize::MAX),
        advanced: Cell::new(0),
    });
    let on_map = Rc::clone(&view);
    picture.connect_map(move |_| Animator::join(&on_map));
    let on_unmap = Rc::clone(&view);
    picture.connect_unmap(move |_| Animator::leave(&on_unmap));
    VIEWS.with(|views| {
        let mut views = views.borrow_mut();
        views.retain(|held| held.picture.upgrade().is_some());
        views.push(Rc::clone(&view));
    });

    let scale = f64::from(crate::theme::bitmap_scale(&picture));
    let attachment = attachment.clone();
    glib::spawn_future_local(async move {
        let frames = frames_of(&attachment, width, height, scale).await;
        let Some(frames) = frames else {
            return;
        };
        view.show(frames);
    });
    picture.upcast()
}

thread_local! {
    /// Every sticker picture alive, for the self-test's question "is it moving?".
    static VIEWS: RefCell<Vec<Rc<View>>> = const { RefCell::new(Vec::new()) };
}

/// How many frame changes the sticker `widget` (as [`picture`] made it) has been through, and
/// whether its frames are animated — `None` when it is not a sticker picture.
#[cfg_attr(
    not(feature = "selftest"),
    allow(dead_code, reason = "the attachments self-test reads it")
)]
pub fn progress(widget: &gtk::Widget) -> Option<(u64, bool)> {
    VIEWS.with(|views| {
        views
            .borrow()
            .iter()
            .find(|view| {
                view.picture
                    .upgrade()
                    .is_some_and(|p| p.upcast_ref::<gtk::Widget>() == widget)
            })
            .map(|view| {
                let moving = view.frames.borrow().as_ref().is_some_and(|f| f.moving());
                (view.advanced.get(), moving)
            })
    })
}

/// The decoded frames of a sticker drawn at `width` × `height`: from the shelf, or downloaded and
/// decoded now (and put on the shelf). `None` when it cannot be had or nothing here reads it.
async fn frames_of(
    attachment: &AttachmentDto,
    width: f64,
    height: f64,
    scale: f64,
) -> Option<Rc<Frames>> {
    // The shelf is keyed by attachment: the panel's 72-pt cell and the bubble's 160-pt box of the
    // same sticker are different sizes, so only the bubble's (the largest) is shared.
    let shared = (width - StickerLook::BOX).abs() < 0.5 || (height - StickerLook::BOX).abs() < 0.5;
    if shared {
        match SHELF.with(|shelf| shelf.borrow_mut().try_get(attachment.id)) {
            Some(Some(frames)) if !frames.paintables.borrow().is_empty() => return Some(frames),
            Some(None) => return None,
            _ => {}
        }
    }
    let connection = Shell::try_get()?.connection()?;
    // A sticker is always its original: a preview is a JPEG, one frame, no transparency.
    let bytes = match connection.attachments.bytes(attachment, false).await {
        Ok(bytes) => bytes,
        Err(error) => {
            crate::fc_info!("sticker {}: not fetched ({})", attachment.id, error.code);
            return None;
        }
    };
    let (frames, cost) = match decode_frames(bytes, width, height, scale).await {
        Some(decoded) => decoded,
        None => {
            if shared {
                SHELF.with(|shelf| shelf.borrow_mut().put(attachment.id, None, 0));
            }
            crate::fc_info!("sticker {}: nothing here decodes it", attachment.id);
            return None;
        }
    };
    if shared {
        SHELF.with(|shelf| {
            shelf
                .borrow_mut()
                .put(attachment.id, Some(Rc::clone(&frames)), cost)
        });
    }
    Some(frames)
}

/// Decodes a sticker's bytes for a `width` × `height` box: every frame when it is animated and
/// worth it, else its first frame alone. With what the frames cost (0 for a still).
pub async fn decode_frames(
    bytes: Vec<u8>,
    width: f64,
    height: f64,
    scale: f64,
) -> Option<(Rc<Frames>, i64)> {
    if animation_prep::animated(&bytes).is_some()
        && let Some(layout) = animation_prep::layout(&bytes)
    {
        let count = i32::try_from(layout.frames.len()).unwrap_or(i32::MAX);
        let side = width.max(height);
        let (dw, dh) = StickerAnimation::decode_size(layout.width, layout.height, side, scale);
        if StickerAnimation::worth(count, dw, dh) {
            let decoder: Rc<dyn StillDecoder> = Rc::new(GdkStills);
            let shared: Rc<[u8]> = Rc::from(bytes.clone().into_boxed_slice());
            if let Some(source) = animation_preparing::frame_source(shared, &layout, decoder) {
                let cancel = Cancel::new();
                let composed =
                    sticker_frames::compose(&layout, &source.pixels, dw, dh, &cancel).await;
                if let Ok(Some(drawn)) = composed {
                    let paintables: Vec<gdk::Paintable> = drawn
                        .into_iter()
                        .map(|bgra| {
                            let texture = premultiplied_texture(bgra, dw, dh);
                            pictures::fit(&texture, width, height)
                        })
                        .collect();
                    let stated: Vec<i32> = layout
                        .stated_ms()
                        .into_iter()
                        .map(|ms| i32::try_from(ms).unwrap_or(i32::MAX))
                        .collect();
                    let clock = StickerAnimation::clock(&stated, count);
                    let frames = Rc::new(Frames {
                        paintables: RefCell::new(paintables),
                        clock,
                        stilled: Cell::new(false),
                    });
                    return Some((frames, StickerAnimation::decoded_bytes(count, dw, dh)));
                }
            }
        }
    }
    // Still: the first frame, decoded near the box at this screen's scale.
    let edge = ((width.max(height)) * scale.max(1.0)).ceil() as i32;
    let texture = pictures::decode_square(bytes, edge.max(1)).await?;
    Some((Frames::still(pictures::fit(&texture, width, height)), 0))
}

/// Premultiplied BGRA (`sticker_frames::compose`'s) as a texture.
fn premultiplied_texture(bgra: Vec<u8>, width: i32, height: i32) -> gdk::Texture {
    let stride = usize::try_from(width.max(1)).unwrap_or(1) * 4;
    gdk::MemoryTexture::new(
        width.max(1),
        height.max(1),
        gdk::MemoryFormat::B8g8r8a8Premultiplied,
        &glib::Bytes::from_owned(bgra),
        stride,
    )
    .upcast()
}

/// fc-logic's `StillDecoder` over gdk-pixbuf (glycin): one still PNG or WebP, its own size, straight
/// BGRA with no padding, EXIF ignored, every alpha 255 for a picture with none.
#[derive(Debug, Default, Clone, Copy)]
pub struct GdkStills;

impl StillDecoder for GdkStills {
    fn decode_still(&self, still: Vec<u8>) -> LocalBoxFuture<'static, Option<DecodedStill>> {
        Box::pin(async move {
            let bytes = glib::Bytes::from_owned(still);
            let stream = gio::MemoryInputStream::from_bytes(&bytes);
            let pixbuf = gdk_pixbuf::Pixbuf::from_stream_future(&stream).await.ok()?;
            Some(straight_bgra(&pixbuf))
        })
    }
}

/// A pixbuf's pixels as straight BGRA, `width × 4` bytes a row.
pub fn straight_bgra(pixbuf: &gdk_pixbuf::Pixbuf) -> DecodedStill {
    let (width, height) = (pixbuf.width().max(0), pixbuf.height().max(0));
    let channels = usize::try_from(pixbuf.n_channels()).unwrap_or(4);
    let stride = usize::try_from(pixbuf.rowstride()).unwrap_or(0);
    let alpha = pixbuf.has_alpha();
    let bytes = pixbuf.read_pixel_bytes();
    let (w, h) = (width as usize, height as usize);
    let mut bgra = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let at = y * stride + x * channels;
            let pixel = |offset: usize| bytes.get(at + offset).copied().unwrap_or(0);
            bgra.push(pixel(2));
            bgra.push(pixel(1));
            bgra.push(pixel(0));
            bgra.push(if alpha { pixel(3) } else { 255 });
        }
    }
    DecodedStill {
        width,
        height,
        bgra,
    }
}

/// One sticker picture on screen.
struct View {
    picture: glib::WeakRef<gtk::Picture>,
    frames: RefCell<Option<Rc<Frames>>>,
    id: i64,
    /// When its animation began, on the animator's clock (ms).
    started: Cell<i64>,
    /// The frame drawn now (`usize::MAX`: none yet).
    shown: Cell<usize>,
    /// Frame changes so far (the self-test's question).
    advanced: Cell<u64>,
}

impl View {
    fn show(self: &Rc<Self>, frames: Rc<Frames>) {
        let Some(picture) = self.picture.upgrade() else {
            return;
        };
        if let Some(first) = frames.frame(0) {
            picture.set_paintable(Some(&first));
        }
        self.shown.set(0);
        *self.frames.borrow_mut() = Some(frames);
        if picture.is_mapped() {
            Animator::join(self);
        }
    }
}

/// The one timer that moves every sticker on screen (see the module's comment).
#[derive(Default)]
struct Animator {
    views: Vec<Rc<View>>,
    epoch: Option<Instant>,
    armed: bool,
}

impl Animator {
    fn now_ms() -> i64 {
        ANIMATOR.with(|animator| {
            let mut animator = animator.borrow_mut();
            let epoch = *animator.epoch.get_or_insert_with(Instant::now);
            i64::try_from(epoch.elapsed().as_millis()).unwrap_or(i64::MAX)
        })
    }

    /// A sticker came on screen (mapped) — or its frames landed while it was.
    fn join(view: &Rc<View>) {
        let moving = view.frames.borrow().as_ref().is_some_and(|f| f.moving());
        if !moving {
            return;
        }
        let now = Animator::now_ms();
        let started = SHELF.with(|shelf| shelf.borrow_mut().started(view.id, now));
        view.started.set(started);
        let arm = ANIMATOR.with(|animator| {
            let mut animator = animator.borrow_mut();
            if !animator.views.iter().any(|held| Rc::ptr_eq(held, view)) {
                animator.views.push(Rc::clone(view));
            }
            !std::mem::replace(&mut animator.armed, true)
        });
        if arm {
            Animator::arm(StickerAnimation::FASTEST_TICK_MS);
        }
    }

    /// A sticker left the screen (unmapped): it stops.
    fn leave(view: &Rc<View>) {
        ANIMATOR.with(|animator| {
            animator
                .borrow_mut()
                .views
                .retain(|held| !Rc::ptr_eq(held, view));
        });
    }

    /// Whether any live view draws these frames.
    fn shows(frames: &Rc<Frames>) -> bool {
        VIEWS.with(|views| {
            views.borrow().iter().any(|view| {
                view.picture.upgrade().is_some()
                    && view
                        .frames
                        .borrow()
                        .as_ref()
                        .is_some_and(|held| Rc::ptr_eq(held, frames))
            })
        })
    }

    fn arm(after_ms: i32) {
        let wait = Duration::from_millis(u64::try_from(after_ms.max(1)).unwrap_or(33));
        glib::timeout_add_local_once(wait, Animator::tick);
    }

    fn tick() {
        let views: Vec<Rc<View>> = ANIMATOR.with(|animator| {
            let mut animator = animator.borrow_mut();
            animator
                .views
                .retain(|view| view.picture.upgrade().is_some());
            if animator.views.is_empty() {
                animator.armed = false;
            }
            animator.views.clone()
        });
        if views.is_empty() {
            return;
        }
        let now = Animator::now_ms();
        let still = super::reduced_motion();
        let mut soonest = i64::MAX;
        for view in &views {
            let (Some(picture), Some(frames)) =
                (view.picture.upgrade(), view.frames.borrow().clone())
            else {
                continue;
            };
            let wanted = if still || !frames.moving() {
                0
            } else if on_screen(picture.upcast_ref()) {
                let elapsed = now - view.started.get();
                soonest = soonest.min(StickerAnimation::until_next(&frames.clock, elapsed));
                StickerAnimation::frame_at(&frames.clock, elapsed)
            } else {
                // Scrolled away: it holds its frame and costs nothing.
                continue;
            };
            if wanted != view.shown.get()
                && let Some(paintable) = frames.frame(wanted)
            {
                picture.set_paintable(Some(&paintable));
                view.shown.set(wanted);
                view.advanced.set(view.advanced.get() + 1);
            }
        }
        let next = if soonest == i64::MAX {
            StickerAnimation::SLOWEST_TICK_MS
        } else {
            StickerAnimation::tick_after(soonest)
        };
        Animator::arm(next);
    }
}

/// Whether `widget` is on screen: drawable (mapped and visible, in a shown window) and inside the
/// visible part of the nearest scrolled view around it.
pub fn on_screen(widget: &gtk::Widget) -> bool {
    if !widget.is_drawable() {
        return false;
    }
    if widget
        .root()
        .and_downcast::<gtk::Window>()
        .is_some_and(|window| !window.is_visible() || window.is_suspended())
    {
        return false;
    }
    let Some(scrolled) = widget.ancestor(gtk::ScrolledWindow::static_type()) else {
        return true;
    };
    let Some(bounds) = widget.compute_bounds(&scrolled) else {
        return false;
    };
    let (w, h) = (scrolled.width() as f32, scrolled.height() as f32);
    bounds.x() < w
        && bounds.y() < h
        && bounds.x() + bounds.width() > 0.0
        && bounds.y() + bounds.height() > 0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pixbuf_row_becomes_straight_bgra() {
        let pixbuf =
            gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, true, 8, 2, 1).expect("a pixbuf");
        pixbuf.put_pixel(0, 0, 10, 20, 30, 40);
        pixbuf.put_pixel(1, 0, 50, 60, 70, 80);
        let decoded = straight_bgra(&pixbuf);
        assert_eq!((decoded.width, decoded.height), (2, 1));
        assert_eq!(decoded.bgra, vec![30, 20, 10, 40, 70, 60, 50, 80]);
        let opaque =
            gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 1, 1).expect("a pixbuf");
        opaque.put_pixel(0, 0, 1, 2, 3, 0);
        assert_eq!(straight_bgra(&opaque).bgra, vec![3, 2, 1, 255]);
    }
}
