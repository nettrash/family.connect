//! The card under a message's link (shell-design §5.2; the Mac's `LinkPreviewCard`): the page's
//! picture across the top (120 pt, cropped to fill), then its site, its title (two lines) and its
//! description (two lines). A click opens the link; a double click is the bubble's heart.
//!
//! **Fetched by fc-logic's `LinkPreviews`** over the shell's OWN stripped-down session (no
//! redirects followed by libsoup, no cookies, no credentials, https only — `SoupHttp::
//! for_link_previews`), and only while the reader's "Link Previews" setting is on: with it off
//! [`card`] answers `None` and nothing is asked of anybody. P1 decides which link and whether a
//! message may have a card at all (`lookups::may_preview`, `bubble_body::preview_link`).
//!
//! **No image leaks** (carry-forward, LinkPreviews in the Windows client): the picture's bytes
//! stay in `LinkPreviews`' own bounded table; a card decodes them for itself and holds the texture
//! only as long as the card lives — nothing here keeps a picture per link.
//!
//! **It may appear later**: the card is a container that stays empty (and takes no room) until
//! the fetch lands; a page that gave nothing leaves it empty for good.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use fc_core::protocol::MessageDto;
use fc_core::rt::Subscription;
use fc_core::text::link_preview::LinkPreview;
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::link_previews::{LinkPreviews, PreviewStatus};

use super::pictures;
use crate::shell::Shell;

/// The card's width in a bubble.
pub const WIDTH: f64 = 280.0;
/// The picture's height across its top.
pub const IMAGE_HEIGHT: f64 = 120.0;

/// The card for `url` — `None` while Link Previews is off (nothing is fetched then). Otherwise a
/// container filled when `previews` lands the page.
pub fn card(
    previews: &Rc<LinkPreviews>,
    message: &MessageDto,
    url: &str,
    own: bool,
) -> Option<gtk::Widget> {
    let _ = (message, own);
    let (status, preview) = previews.state(url)?;
    let holder = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .halign(gtk::Align::Start)
        .build();
    match (status, preview) {
        (PreviewStatus::Loaded, Some(preview)) => fill(&holder, previews, &preview),
        (PreviewStatus::Loading, _) => wait(&holder, previews, url),
        _ => {}
    }
    Some(holder.upcast())
}

/// Fills `holder` once the fetch for `url` lands (and stops listening then, or when the holder
/// goes).
fn wait(holder: &gtk::Box, previews: &Rc<LinkPreviews>, url: &str) {
    let subscription: Rc<RefCell<Option<Subscription>>> = Rc::new(RefCell::new(None));
    let (weak_holder, weak_previews) = (holder.downgrade(), Rc::downgrade(previews));
    let (url, held_id) = (url.to_string(), Rc::clone(&subscription));
    let id = previews.landed().subscribe(move |_| {
        let (Some(holder), Some(previews)) = (weak_holder.upgrade(), weak_previews.upgrade())
        else {
            return;
        };
        match previews.state(&url) {
            Some((PreviewStatus::Loading, _)) => {}
            Some((PreviewStatus::Loaded, Some(preview))) => {
                fill(&holder, &previews, &preview);
                stop(&previews, &held_id);
            }
            _ => stop(&previews, &held_id),
        }
    });
    *subscription.borrow_mut() = Some(id);
    let previews = Rc::downgrade(previews);
    holder.connect_destroy(move |_| {
        if let Some(previews) = previews.upgrade() {
            stop(&previews, &subscription);
        }
    });
}

/// Stops listening (safe inside the event's own emission: `Event::emit` works on a copy).
fn stop(previews: &LinkPreviews, subscription: &Rc<RefCell<Option<Subscription>>>) {
    if let Some(id) = subscription.borrow_mut().take() {
        previews.landed().unsubscribe(id);
    }
}

/// The card itself, into `holder`.
fn fill(holder: &gtk::Box, previews: &Rc<LinkPreviews>, preview: &LinkPreview) {
    if holder.first_child().is_some() {
        return;
    }
    let say = &Shell::get().say;
    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .width_request(pictures::logical(WIDTH))
        .overflow(gtk::Overflow::Hidden)
        .css_classes(["fc-attach-link-card"])
        .build();
    if let Some(bytes) = previews.image(&preview.url) {
        let picture = gtk::Picture::builder()
            .can_shrink(true)
            .content_fit(gtk::ContentFit::Fill)
            .paintable(&pictures::empty(WIDTH, IMAGE_HEIGHT))
            .height_request(pictures::logical(IMAGE_HEIGHT))
            .visible(false)
            .build();
        card.append(&picture);
        let weak = picture.downgrade();
        let bytes = bytes.to_vec();
        let scale = f64::from(crate::theme::bitmap_scale(holder));
        glib::spawn_future_local(async move {
            let width = (WIDTH * scale).ceil() as i32;
            if let (Some(texture), Some(picture)) =
                (pictures::decode(bytes, Some(width)).await, weak.upgrade())
            {
                picture.set_paintable(Some(&pictures::cover(&texture, WIDTH, IMAGE_HEIGHT)));
                picture.set_visible(true);
            }
        });
    }
    let text = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .margin_start(10)
        .margin_end(10)
        .margin_top(8)
        .margin_bottom(8)
        .build();
    let site = gtk::Label::builder()
        .label(&preview.site_name)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .single_line_mode(true)
        .css_classes(["fc-attach-link-site"])
        .build();
    let title = gtk::Label::builder()
        .label(&preview.title)
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .lines(2)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .max_width_chars(36)
        .css_classes(["fc-attach-link-title"])
        .build();
    text.append(&site);
    text.append(&title);
    if let Some(description) = preview.description.as_deref().filter(|d| !d.is_empty()) {
        let line = gtk::Label::builder()
            .label(description)
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .lines(2)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars(40)
            .css_classes(["fc-attach-link-text"])
            .build();
        text.append(&line);
    }
    card.append(&text);
    card.set_accessible_role(gtk::AccessibleRole::Link);
    card.update_property(&[
        gtk::accessible::Property::Label(&format!("{}, {}", preview.title, preview.site_name)),
        gtk::accessible::Property::Description(say.get("Opens the link")),
    ]);
    card.set_focusable(true);
    let url = preview.url.clone();
    let weak = card.downgrade();
    super::on_click(&card, move || {
        let parent = weak
            .upgrade()
            .and_then(|card| card.root())
            .and_downcast::<gtk::Window>();
        let launcher = gtk::UriLauncher::new(&url);
        glib::spawn_future_local(async move {
            if launcher.launch_future(parent.as_ref()).await.is_err() {
                crate::fc_info!("link: nothing opened it");
            }
        });
    });
    holder.append(&card);
}
