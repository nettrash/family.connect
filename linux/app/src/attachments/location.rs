//! A shared place in a bubble (shell-design §5.2, §8.5, D11 — Windows' `LocationElement` and
//! `MapElement`, the Apple apps' 140-pt MapKit map drawn from OpenStreetMap's tiles): a 260-pt card
//! whose row is the pin disc, the place's name and its coordinates (`fc_text::media::
//! location_line`, always with a POINT), and — when the place is known AND the reader has "Map
//! Previews" on — the 240 × 132 map above it: the streets around the place with the place at the
//! middle, a red dot ringed in white, rounded, not interactive (a pannable map inside a scrolling
//! conversation fights every drag), and credited to OpenStreetMap as its licence requires. Tiles
//! appear as they arrive; before then the square is a quiet grey. With maps off nothing is asked of
//! OpenStreetMap: the pin, the name and the link only.
//!
//! A click opens the place on openstreetmap.org (`map_tiles::place_url`); a card with no
//! coordinates is not clickable. Inside the reader's own balloon nothing is drawn in the accent.

use std::rc::Rc;

use adw::prelude::*;
use fc_core::protocol::{AttachmentDto, MessageDto};
use fc_logic::map_view;
use fc_text::media;

use super::map_tiles::{self, MapTiles};
use super::pictures;
use crate::services::settings_store::keys;
use crate::shell::Shell;

/// The card's width.
pub const CARD_WIDTH: i32 = 260;
/// The map inside it, less the card's padding and border.
pub const MAP_WIDTH: f64 = 240.0;
pub const MAP_HEIGHT: f64 = 132.0;

/// OpenStreetMap's licence asks for its name on every map drawn from it — a proper name and a
/// licence notice, the same in every language (Windows draws it in English too), so it is not a
/// catalogue sentence.
pub const CREDIT: &str = "© OpenStreetMap contributors";

/// The location card of `attachment` (`ConversationSlots::location`).
pub fn card(message: &MessageDto, attachment: &AttachmentDto, own: bool) -> gtk::Widget {
    let _ = (message, own);
    let shell = Shell::get();
    let name = media::display_name("location", attachment.name.as_deref()).into_owned();
    let place = match (attachment.latitude, attachment.longitude) {
        (Some(latitude), Some(longitude)) if latitude.is_finite() && longitude.is_finite() => {
            Some((latitude, longitude))
        }
        _ => None,
    };
    let line = place
        .map(|(latitude, longitude)| {
            media::location_line(latitude, longitude, attachment.accuracy_m)
        })
        .unwrap_or_default();

    let disc = gtk::Box::builder()
        .width_request(36)
        .height_request(36)
        .valign(gtk::Align::Center)
        .css_classes(["fc-attach-pin-disc"])
        .build();
    let pin = gtk::Image::builder()
        .icon_name("mark-location-symbolic")
        .pixel_size(16)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .hexpand(true)
        .build();
    disc.append(&pin);
    let title = gtk::Label::builder()
        .label(&name)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .single_line_mode(true)
        .build();
    let lines = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .valign(gtk::Align::Center)
        .hexpand(true)
        .build();
    lines.append(&title);
    if !line.is_empty() {
        let numbers = gtk::Label::builder()
            .label(&line)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .single_line_mode(true)
            .css_classes(["fc-attach-card-line"])
            .build();
        lines.append(&numbers);
    }
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .build();
    row.append(&disc);
    row.append(&lines);
    if place.is_some() {
        // "Opens elsewhere": the click hands the place to a map.
        let away = gtk::Image::builder()
            .icon_name("adw-external-link-symbolic")
            .pixel_size(12)
            .opacity(0.6)
            .valign(gtk::Align::Center)
            .build();
        row.append(&away);
    }

    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .width_request(CARD_WIDTH)
        .halign(gtk::Align::Start)
        .valign(gtk::Align::Start)
        .css_classes(["fc-attach-card"])
        .build();
    if let Some((latitude, longitude)) = place
        && shell.settings.flag(keys::MAP_PREVIEWS)
    {
        card.append(&map(latitude, longitude));
    }
    card.append(&row);
    card.set_accessible_role(gtk::AccessibleRole::Button);
    let spoken = if line.is_empty() {
        name.clone()
    } else {
        format!("{name}. {line}")
    };
    card.update_property(&[gtk::accessible::Property::Label(&spoken)]);
    if let Some((latitude, longitude)) = place {
        card.set_focusable(true);
        let weak = card.downgrade();
        super::on_click(&card, move || {
            let parent = weak
                .upgrade()
                .and_then(|card| card.root())
                .and_downcast::<gtk::Window>();
            let launcher = gtk::UriLauncher::new(&map_tiles::place_url(latitude, longitude));
            glib::spawn_future_local(async move {
                if launcher.launch_future(parent.as_ref()).await.is_err() {
                    crate::fc_info!("location: nothing opened the map");
                }
            });
        });
    } else {
        card.set_sensitive(false);
    }
    card.upcast()
}

/// The map: the tiles `map_view::tiles` places, the dot at the middle, the credit.
fn map(latitude: f64, longitude: f64) -> gtk::Widget {
    let (width, height) = (pictures::logical(MAP_WIDTH), pictures::logical(MAP_HEIGHT));
    let frame = gtk::Overlay::builder()
        .width_request(width)
        .height_request(height)
        .halign(gtk::Align::Start)
        .overflow(gtk::Overflow::Hidden)
        .css_classes(["fc-attach-map"])
        .build();
    // The frame's size comes from this box alone: the tiles hang outside it and are clipped.
    frame.set_child(Some(
        &gtk::Box::builder()
            .width_request(width)
            .height_request(height)
            .build(),
    ));
    let fixed = gtk::Fixed::new();
    frame.add_overlay(&fixed);
    let size = f64::from(map_view::TILE_SIZE);
    let tiles = MapTiles::get();
    for tile in map_view::tiles(latitude, longitude, MAP_WIDTH, MAP_HEIGHT, map_view::ZOOM) {
        let picture = gtk::Picture::builder()
            .can_shrink(true)
            .content_fit(gtk::ContentFit::Fill)
            .width_request(map_view::TILE_SIZE)
            .height_request(map_view::TILE_SIZE)
            .paintable(&pictures::empty(size, size))
            .build();
        fixed.put(&picture, tile.left, tile.top);
        let (weak, tiles) = (picture.downgrade(), Rc::clone(&tiles));
        glib::spawn_future_local(async move {
            let Some(bytes) = tiles.bytes(&tile).await else {
                return;
            };
            let texture = gtk::gdk::Texture::from_bytes(&glib::Bytes::from(&*bytes)).ok();
            if let (Some(texture), Some(picture)) = (texture, weak.upgrade()) {
                picture.set_paintable(Some(&texture));
            }
        });
    }
    let ring = gtk::Box::builder()
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .css_classes(["fc-attach-map-ring"])
        .build();
    frame.add_overlay(&ring);
    let dot = gtk::Box::builder()
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .css_classes(["fc-attach-map-dot"])
        .build();
    frame.add_overlay(&dot);
    let credit = gtk::Label::builder()
        .label(CREDIT)
        .halign(gtk::Align::End)
        .valign(gtk::Align::End)
        .css_classes(["fc-attach-map-credit"])
        .build();
    frame.add_overlay(&credit);
    // Decorative: the card's own name already says the place and its coordinates.
    frame.set_accessible_role(gtk::AccessibleRole::Presentation);
    frame.upcast()
}
