//! A file in a bubble (shell-design §8.1): its name — a long one with its MIDDLE given up, so its
//! start and its extension stay readable (`attachment_files::middle_truncate`) — its size in the
//! reader's decimal format (`media_text::display_size`: "Zero KB" and "%lld bytes" are the
//! catalogue's), and two buttons: Save… (where the person chooses) and Open (a copy handed to
//! whatever opens that kind of file here). Inside the reader's own balloon the card goes
//! translucent white (`fc-own-bubble .fc-attach-card`).

use adw::prelude::*;
use fc_core::protocol::AttachmentDto;
use fc_core::text::media_text;
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::attachment_files;
use fc_text::media;

use super::saving::{self, Outcome};
use crate::shell::Shell;

/// How many characters of a name a row shows before it gives up the middle.
pub const NAME_ROOM: usize = 32;
/// The row's width (the Mac's file row in a bubble).
pub const WIDTH: i32 = 260;

/// The file row of `attachment`.
pub fn row(attachment: &AttachmentDto, own: bool) -> gtk::Widget {
    let shell = Shell::get();
    let say = &shell.say;
    let _ = own;
    let name = media::display_name(&attachment.kind, attachment.name.as_deref()).into_owned();
    let icon = gtk::Image::builder()
        .icon_name(if attachment.kind == "audio" {
            "audio-x-generic-symbolic"
        } else {
            "text-x-generic-symbolic"
        })
        .pixel_size(24)
        .valign(gtk::Align::Center)
        .build();
    let title = gtk::Label::builder()
        .label(attachment_files::middle_truncate(&name, NAME_ROOM))
        .xalign(0.0)
        .single_line_mode(true)
        .build();
    let size = gtk::Label::builder()
        .label(media_text::display_size(
            attachment.size.unwrap_or(0),
            shell.numbers.decimal,
        ))
        .xalign(0.0)
        .css_classes(["fc-attach-card-line"])
        .build();
    let lines = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .valign(gtk::Align::Center)
        .hexpand(true)
        .build();
    lines.append(&title);
    lines.append(&size);
    let save = gtk::Button::builder()
        .icon_name("document-save-symbolic")
        .tooltip_text(say.get("Save…"))
        .valign(gtk::Align::Center)
        .css_classes(["flat", "circular"])
        .build();
    save.update_property(&[gtk::accessible::Property::Label(say.get("Save…"))]);
    let open = gtk::Button::builder()
        .label(say.get("Open"))
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(10)
        .width_request(WIDTH)
        .halign(gtk::Align::Start)
        .css_classes(["fc-attach-card"])
        .build();
    card.append(&icon);
    card.append(&lines);
    card.append(&save);
    card.append(&open);
    card.set_accessible_role(gtk::AccessibleRole::Group);
    card.update_property(&[gtk::accessible::Property::Label(&format!(
        "{name}, {}",
        size.label()
    ))]);

    let held = attachment.clone();
    save.connect_clicked(move |button| {
        let (held, button) = (held.clone(), button.clone());
        glib::spawn_future_local(async move {
            button.set_sensitive(false);
            let parent = button.root().and_downcast::<gtk::Window>();
            let outcome = saving::save(parent, &held).await;
            button.set_sensitive(true);
            if let Outcome::Failed(sentence) = outcome {
                saving::alert(Some(button.upcast_ref()), &sentence);
            }
        });
    });
    let held = attachment.clone();
    open.connect_clicked(move |button| {
        let (held, button) = (held.clone(), button.clone());
        glib::spawn_future_local(async move {
            button.set_sensitive(false);
            let parent = button.root().and_downcast::<gtk::Window>();
            let outcome = saving::open(parent, &held).await;
            button.set_sensitive(true);
            if let Outcome::Failed(sentence) = outcome {
                saving::alert(Some(button.upcast_ref()), &sentence);
            }
        });
    });
    card.upcast()
}
