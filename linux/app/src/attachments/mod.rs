//! **P2 — attachments and rich** (shell-design §8, §9; W.2): what P1's bubbles draw that is not
//! words, and the ways files come in and go out.
//!
//! | Module | What | § |
//! | --- | --- | --- |
//! | [`tiles`] | photo and video tiles at their own shape from metadata, the grid of four with "+n", animations playing in place; [`tiles::media_block`] lays out a whole message | 8.1 |
//! | [`file_row`] | a file: name, size, Save…, Open | 8.1 |
//! | [`saving`] | Save… through `gtk::FileDialog`, Open through `gtk::FileLauncher` | 8.1, 8.2 |
//! | [`pictures`] | bytes through the `AttachmentCache`, decoding near the drawn size, the decoded memory | 8.1 |
//! | [`avatars`] | [`avatars::show`]: a profile picture into P1's `adw::Avatar` | 8.3 |
//! | [`stickers`] | a sticker in a bubble, animated where this machine can; the shelf and the one timer | 8.4 |
//! | [`sticker_panel`] | the panel P1's sticker button opens, with a pick callback | 8.4 |
//! | [`pack`] | the family's pack: add, remove (P4's Family dialog opens it) | 8.4 |
//! | [`link_card`] | the card under a message's link (`LinkPreviews`, the `link-previews` setting) | 5.2 |
//! | [`location`], [`map_tiles`] | a shared place: OpenStreetMap tiles behind `map-previews`, the pin, the link | 5.2, 8.5 |
//! | [`location_portal`] | "where I am, once" through the XDG Location portal (P1's paperclip) | 8.5 |
//! | [`inbox`] | files handed to the app → "Send to" → staged in that chat | 8.6 |
//!
//! The viewer window is `crate::viewer` (+ `windows::viewer_window`), the rich renderer
//! `crate::rich`. [`install`] puts all of it into the shell (one line in `app::install_packages`).
//!
//! **The slot contract** (`seams::conversation`): every widget comes back at once at its FINAL
//! size from metadata, fetches through the connection's `AttachmentCache`, stops whatever it plays
//! when unmapped, and never logs a name, a file name or words — ids, counts and codes only.

pub mod avatars;
pub mod drag_out;
pub mod file_row;
pub mod inbox;
pub mod link_card;
pub mod location;
pub mod location_portal;
pub mod map_tiles;
pub mod pack;
pub mod pictures;
pub mod saving;
pub mod sticker_panel;
pub mod stickers;
mod style;
pub mod tiles;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use fc_core::protocol::{AttachmentDto, MessageDto};
use fc_logic::bubble_body::{BodyRichBlock, LINK_DELAY};
use fc_logic::link_previews::LinkPreviews;
use fc_logic::rich_drawing::RichLook;

use crate::seams::conversation::{ConversationSlots, RichPicture, TileLook};
use crate::shell::Shell;

/// P2's whole installation (W.3): the bubble slots, the inbox, the rich renderer's window part and
/// banner, the CSS; and the copies Open left last time are swept.
pub fn install(shell: &Rc<Shell>) {
    crate::theme::register(style::css);
    shell
        .attachment_slots
        .install(Rc::new(AttachmentSlots::default()));
    shell.inbox.install(inbox::Inbox::new(shell));
    crate::rich::install(shell);
    crate::services::files::wipe_dir(&shell.paths.cache_dir().join(saving::OPENED_FOLDER));
    // A circle's "Open Full Screen" (P3's expand control): the viewer, round, on that one item.
    crate::media::circle::set_open_viewer(Rc::new(|message: &MessageDto, attachment| {
        crate::viewer::open(vec![attachment.clone()], 0, true, (message.id, 0));
    }));
}

/// P2's `ConversationSlots`: tiles, stickers, rich blocks, link cards, locations.
#[derive(Default)]
pub struct AttachmentSlots {
    /// The link previews to ask (the shell's own unless the self-test gives others, which answer
    /// from memory instead of the network).
    link_previews: RefCell<Option<Rc<LinkPreviews>>>,
}

impl AttachmentSlots {
    /// The same slots asking `previews` for link cards.
    #[cfg_attr(
        not(feature = "selftest"),
        allow(
            dead_code,
            reason = "the attachments self-test asks link cards of pages from memory"
        )
    )]
    pub fn with_link_previews(previews: Rc<LinkPreviews>) -> AttachmentSlots {
        AttachmentSlots {
            link_previews: RefCell::new(Some(previews)),
        }
    }

    fn previews(&self) -> Rc<LinkPreviews> {
        self.link_previews
            .borrow()
            .clone()
            .unwrap_or_else(|| Rc::clone(&Shell::get().link_previews))
    }
}

impl ConversationSlots for AttachmentSlots {
    fn tile(
        &self,
        message: &MessageDto,
        attachment: &AttachmentDto,
        look: &TileLook,
    ) -> gtk::Widget {
        tiles::tile(message, attachment, look)
    }

    fn sticker(&self, message: &MessageDto, attachment: &AttachmentDto) -> gtk::Widget {
        stickers::sticker(message, attachment)
    }

    fn rich(&self, block: &BodyRichBlock, look: &RichLook) -> RichPicture {
        crate::rich::block(block, look)
    }

    fn rich_inline(&self, source: &str, typed: &str, look: &RichLook) -> RichPicture {
        crate::rich::inline(source, typed, look)
    }

    fn link_card(&self, message: &MessageDto, url: &str, own: bool) -> Option<gtk::Widget> {
        link_card::card(&self.previews(), message, url, own)
    }

    fn location(&self, message: &MessageDto, attachment: &AttachmentDto) -> gtk::Widget {
        let own = Shell::get()
            .connection()
            .and_then(|connection| connection.session.state().me.map(|me| me.id))
            .is_some_and(|me| me == message.sender_id);
        location::card(message, attachment, own)
    }
}

/// A single click on `widget` runs `action` — after `bubble_body::LINK_DELAY`, so that the second
/// press of a double click (the bubble's heart, P1's gesture) calls it off. Enter and Space on a
/// focused widget run it at once. The pointer becomes a hand over it (the Mac's `MacCursor`).
pub fn on_click(widget: &impl IsA<gtk::Widget>, action: impl Fn() + 'static) {
    let widget = widget.as_ref();
    widget.set_cursor_from_name(Some("pointer"));
    let action = Rc::new(action);
    let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
    let presses = Rc::new(Cell::new(0));
    let click = gtk::GestureClick::new();
    click.set_button(gdk_primary());
    {
        let (pending, presses) = (Rc::clone(&pending), Rc::clone(&presses));
        click.connect_pressed(move |_, n_press, _, _| {
            presses.set(n_press);
            if n_press >= 2
                && let Some(source) = pending.borrow_mut().take()
            {
                source.remove();
            }
        });
    }
    {
        let (pending, presses, action) =
            (Rc::clone(&pending), Rc::clone(&presses), Rc::clone(&action));
        click.connect_released(move |_, n_press, _, _| {
            if n_press != 1 || presses.get() != 1 {
                return;
            }
            let (action, slot) = (Rc::clone(&action), Rc::clone(&pending));
            let source = glib::timeout_add_local_once(LINK_DELAY, move || {
                slot.borrow_mut().take();
                action();
            });
            if let Some(old) = pending.borrow_mut().replace(source) {
                old.remove();
            }
        });
    }
    widget.add_controller(click);
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(move |_, key, _, _| match key {
        gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter | gtk::gdk::Key::space => {
            action();
            glib::Propagation::Stop
        }
        _ => glib::Propagation::Proceed,
    });
    widget.add_controller(keys);
}

/// The primary pointer button.
fn gdk_primary() -> u32 {
    gtk::gdk::BUTTON_PRIMARY
}

/// Whether the desktop asks for less motion: GTK's animations off, or GTK 4.22's
/// `gtk-interface-reduced-motion` (§24). Animated stickers then show frame zero and animations do
/// not play in their tiles.
pub fn reduced_motion() -> bool {
    gtk::Settings::default().is_some_and(|settings| {
        !settings.is_gtk_enable_animations()
            || settings.gtk_interface_reduced_motion() == gtk::ReducedMotion::Reduce
    })
}
