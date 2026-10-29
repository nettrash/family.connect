//! Tiles in a bubble (shell-design §8.1): a photo or a video at ITS OWN SHAPE FROM METADATA
//! (`fc_text::media::tile_size`), so a row never changes height when its picture lands; several as
//! Windows' grid of four 156-pt cells with "+n" over the last; a file as a row with Save… and Open
//! ([`super::file_row`]); a location as its card ([`super::location`]).
//!
//! **What a tile draws** is `attachment_files::source_for`: the preview when the attachment says it
//! has one, a photo's own bytes when not (decoded near the tile's size, `picture_memory`), and for a
//! video with no poster nothing at all — a tile never downloads a whole video. A video carries the
//! play disc and its length; an ANIMATION (docs/protocol.md, "Animations") plays muted and looping
//! in its tile while it is on screen, through P3's player, when the system wants motion and the
//! file is within `animation_prep::TILE_BYTES`; otherwise its poster stays.
//!
//! **A click opens the viewer** at that item (`viewer::open_message`); a double click is the
//! bubble's heart (P1's gesture), so a single click waits `bubble_body::LINK_DELAY` and a second
//! press calls it off — the Mac's "count 2 before count 1". Clickable tiles show the pointer
//! cursor (the Mac's `MacCursor`).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use fc_core::protocol::{AttachmentDto, MessageDto};
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::animated_pictures::animation_prep;
use fc_logic::attachment_cache::AttachmentCache;
use fc_logic::attachment_files::{self, TileSource};
use fc_logic::media_album::MediaAlbum;
use fc_text::media;

use super::pictures;
use crate::seams::conversation::{ConversationSlots, TileLook};
use crate::seams::media::{MediaSource, Player, PlayerState};
use crate::shell::Shell;

/// One cell of the grid of four (Windows' `TileElement(…, 156, 156)`).
pub const GRID_CELL: f64 = 156.0;
/// Between the grid's cells.
pub const GRID_SPACING: i32 = 4;
/// A file row's box, as a `TileLook` says it (the row sizes itself from its words).
const FILE_ROW: (f64, f64) = (260.0, 56.0);

/// A photo, a video or a file of `message`, drawn at `look`'s size (`ConversationSlots::tile`).
pub fn tile(message: &MessageDto, attachment: &AttachmentDto, look: &TileLook) -> gtk::Widget {
    match attachment.kind.as_str() {
        "location" => super::location::card(message, attachment, look.own),
        "file" | "audio" => super::file_row::row(attachment, look.own),
        _ => media_tile(message, attachment, look),
    }
}

/// What a message carries, laid out (Windows' `MediaElement`): an animation alone in a photo's box;
/// otherwise the pictures first — one at its own shape, several as the grid of four with the rest
/// counted — then the rows that are read rather than looked at (locations, files; a voice note
/// through P3's slot). A sticker or a video message is its own block (P2's sticker, P3's circle).
/// Every piece comes through `Shell::slots()`, as P1's bubble asks for it (`ConversationSlots`).
pub fn media_block(message: &MessageDto, own: bool) -> gtk::Widget {
    let column = gtk::Box::new(gtk::Orientation::Vertical, GRID_SPACING);
    column.set_halign(gtk::Align::Start);
    let slots = Shell::get().slots();
    let all = message.media();
    if let Some(first) = all.first()
        && first.sticker
    {
        column.append(&slots.sticker(message, first));
        return column.upcast();
    }
    if let Some(round) = message.round_video() {
        column.append(&slots.circle(message, round));
        return column.upcast();
    }
    if let Some(animation) = message.animation_video() {
        let (width, height) = media::tile_size(
            animation.width.map(i64::from),
            animation.height.map(i64::from),
        );
        let look = TileLook {
            width,
            height,
            own,
            more: 0,
        };
        column.append(&slots.tile(message, animation, &look));
        return column.upcast();
    }
    let looked = MediaAlbum::of(all);
    if looked.len() == 1 {
        let (width, height) = media::tile_size(
            looked[0].width.map(i64::from),
            looked[0].height.map(i64::from),
        );
        let look = TileLook {
            width,
            height,
            own,
            more: 0,
        };
        column.append(&slots.tile(message, &looked[0], &look));
    } else if looked.len() > 1 {
        let grid = gtk::Grid::builder()
            .row_spacing(GRID_SPACING)
            .column_spacing(GRID_SPACING)
            .halign(gtk::Align::Start)
            .build();
        let shown = looked.len().min(4);
        for (at, item) in looked.iter().take(shown).enumerate() {
            let more = if at + 1 == shown {
                u32::try_from(looked.len() - shown).unwrap_or(u32::MAX)
            } else {
                0
            };
            let look = TileLook {
                width: GRID_CELL,
                height: GRID_CELL,
                own,
                more,
            };
            let cell = slots.tile(message, item, &look);
            grid.attach(&cell, (at % 2) as i32, (at / 2) as i32, 1, 1);
        }
        column.append(&grid);
    }
    for attachment in all.iter().filter(|a| !media::is_media(&a.kind)) {
        let row = match attachment.kind.as_str() {
            "location" => slots.location(message, attachment),
            "audio" => slots.voice(message, attachment, own),
            _ => slots.tile(
                message,
                attachment,
                &TileLook {
                    width: FILE_ROW.0,
                    height: FILE_ROW.1,
                    own,
                    more: 0,
                },
            ),
        };
        column.append(&row);
    }
    column.upcast()
}

/// A photo's or a video's tile.
fn media_tile(message: &MessageDto, attachment: &AttachmentDto, look: &TileLook) -> gtk::Widget {
    let shell = Shell::get();
    let say = &shell.say;
    let (width, height) = (
        pictures::logical(look.width),
        pictures::logical(look.height),
    );
    let video = attachment.kind == "video";
    let frame = gtk::Overlay::builder()
        .width_request(width)
        .height_request(height)
        .halign(gtk::Align::Start)
        .valign(gtk::Align::Start)
        .overflow(gtk::Overflow::Hidden)
        .css_classes(["fc-attach-tile"])
        .build();
    // The quiet stand-in until the picture lands: the kind's icon, dimmed.
    let stand_in = gtk::Image::builder()
        .icon_name(if video {
            "video-x-generic-symbolic"
        } else {
            "image-x-generic-symbolic"
        })
        .pixel_size(32)
        .opacity(0.45)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    let base = gtk::Box::builder()
        .width_request(width)
        .height_request(height)
        .build();
    base.append(&stand_in);
    stand_in.set_hexpand(true);
    frame.set_child(Some(&base));
    let picture = gtk::Picture::builder()
        .can_shrink(true)
        .content_fit(gtk::ContentFit::Fill)
        .paintable(&pictures::empty(look.width, look.height))
        .build();
    frame.add_overlay(&picture);

    let animation = video && attachment.animation;
    if video && !animation {
        // The play disc says what it is; the kind's icon behind it would only crowd it.
        stand_in.set_visible(false);
        let disc = gtk::Image::builder()
            .icon_name("media-playback-start-symbolic")
            .pixel_size(20)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .css_classes(["fc-attach-play"])
            .build();
        frame.add_overlay(&disc);
        if let Some(ms) = attachment.duration_ms.filter(|ms| *ms > 0) {
            let length = gtk::Label::builder()
                .label(media::time_label(f64::from(ms) / 1000.0))
                .halign(gtk::Align::Start)
                .valign(gtk::Align::End)
                .margin_start(6)
                .margin_bottom(6)
                .css_classes(["fc-attach-duration"])
                .build();
            frame.add_overlay(&length);
        }
    }
    if look.more > 0 {
        let more = gtk::Label::builder()
            .label(format!("+{}", look.more))
            .halign(gtk::Align::Fill)
            .valign(gtk::Align::Fill)
            .css_classes(["fc-attach-tile-more"])
            .build();
        frame.add_overlay(&more);
    }

    // The picture: what `source_for` says, decoded near the tile's size.
    let source = attachment_files::source_for(attachment);
    if source != TileSource::None {
        let original = source == TileSource::Original;
        let scale = f64::from(crate::theme::bitmap_scale(&frame));
        let decode_width = pictures::tile_decode_width(attachment, original, scale);
        let (attachment_held, weak_picture, weak_stand_in) = (
            attachment.clone(),
            picture.downgrade(),
            stand_in.downgrade(),
        );
        let (w, h) = (look.width, look.height);
        glib::spawn_future_local(async move {
            let texture = pictures::of_attachment(&attachment_held, !original, decode_width).await;
            if let (Some(texture), Some(picture)) = (texture, weak_picture.upgrade()) {
                picture.set_paintable(Some(&pictures::cover(&texture, w, h)));
                if let Some(stand_in) = weak_stand_in.upgrade() {
                    stand_in.set_visible(false);
                }
            }
        });
    }
    if animation {
        AnimationTile::attach(&frame, &picture, attachment);
    }

    // The name a screen reader says.
    let name = media::display_name(&attachment.kind, attachment.name.as_deref()).into_owned();
    let label = if animation {
        say.get("GIF").to_string()
    } else {
        match attachment.duration_ms.filter(|ms| *ms > 0 && video) {
            Some(ms) => format!("{name}, {}", media::time_label(f64::from(ms) / 1000.0)),
            None => name,
        }
    };
    frame.set_accessible_role(gtk::AccessibleRole::Button);
    frame.update_property(&[gtk::accessible::Property::Label(&label)]);
    frame.set_focusable(true);

    let index = MediaAlbum::of_message(message)
        .iter()
        .position(|item| item.id == attachment.id);
    let (message_held, attachment_held) = (message.clone(), attachment.clone());
    super::on_click(&frame, move || match index {
        Some(index) => crate::viewer::open_message(&message_held, index),
        None => crate::viewer::open(
            vec![attachment_held.clone()],
            0,
            false,
            (message_held.id, 0),
        ),
    });
    frame.upcast()
}

/// An animation's tile playing in place: muted, looping, only while mapped and on screen, through
/// P3's player (`Shell::players`), from the cached file — the one video a tile downloads, and only
/// within `animation_prep::TILE_BYTES`. Nothing plays under Reduce Motion: the poster stays.
struct AnimationTile {
    picture: glib::WeakRef<gtk::Picture>,
    attachment: AttachmentDto,
    player: RefCell<Option<Rc<dyn Player>>>,
    /// The download asked for (once).
    fetched: Cell<bool>,
    path: RefCell<Option<std::path::PathBuf>>,
}

impl AnimationTile {
    fn attach(frame: &gtk::Overlay, picture: &gtk::Picture, attachment: &AttachmentDto) {
        let wanted = !super::reduced_motion();
        if !animation_prep::plays_in_tile(wanted, attachment.size) {
            return;
        }
        let tile = Rc::new(AnimationTile {
            picture: picture.downgrade(),
            attachment: attachment.clone(),
            player: RefCell::new(None),
            fetched: Cell::new(false),
            path: RefCell::new(None),
        });
        let on_map = Rc::clone(&tile);
        frame.connect_map(move |_| on_map.start());
        let on_unmap = Rc::clone(&tile);
        frame.connect_unmap(move |_| on_unmap.stop());
    }

    fn start(self: &Rc<Self>) {
        if let Some(path) = self.path.borrow().clone() {
            self.play(path);
            return;
        }
        if self.fetched.replace(true) {
            return;
        }
        let me = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let Some(shell) = Shell::try_get() else {
                return;
            };
            let Some(connection) = shell.connection() else {
                return;
            };
            let Some(attachment) = me.upgrade().map(|me| me.attachment.clone()) else {
                return;
            };
            if connection
                .attachments
                .bytes(&attachment, false)
                .await
                .is_err()
            {
                return;
            }
            // The cache wrote it whole under its key: the player reads that file.
            let path = shell
                .paths
                .blobs_dir()
                .join(AttachmentCache::key_for(attachment.id, false));
            if let Some(me) = me.upgrade() {
                *me.path.borrow_mut() = Some(path.clone());
                if me.picture.upgrade().is_some_and(|p| p.is_mapped()) {
                    me.play(path);
                }
            }
        });
    }

    fn play(self: &Rc<Self>, path: std::path::PathBuf) {
        let Some(picture) = self.picture.upgrade() else {
            return;
        };
        let player = Shell::get().players.get().create(true);
        player.set_muted(true);
        player.load(MediaSource::File(path));
        let weak_player = Rc::downgrade(&player);
        let weak_picture = picture.downgrade();
        player.on_state().subscribe(move |state| {
            let Some(player) = weak_player.upgrade() else {
                return;
            };
            match state {
                // A loop: an animation is not a film with an end.
                PlayerState::Ended => {
                    player.seek(0.0);
                    player.play();
                }
                PlayerState::Playing => {
                    if let (Some(picture), Some(paintable)) =
                        (weak_picture.upgrade(), player.paintable())
                    {
                        picture.set_paintable(Some(&paintable));
                    }
                }
                _ => {}
            }
        });
        player.play();
        *self.player.borrow_mut() = Some(player);
    }

    fn stop(&self) {
        if let Some(player) = self.player.borrow_mut().take() {
            player.stop();
        }
    }
}
