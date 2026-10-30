//! Small pieces every P4 surface draws with (Settings, the Family dialog, the group dialogs, the
//! board): the alert of §3.2 (`adw::AlertDialog` presented with `present` and answered through
//! its `response` signal — NEVER `choose_future`, which libadwaita-rs 0.9 aborts on when a pending
//! choice is dropped), a face, the Owner pill, a footnote, a toast, and `spawn`.

use std::future::Future;
use std::rc::Rc;

use adw::prelude::*;

/// Runs `work` on the GLib main context (port-plan "Async": one thread, `spawn_local`).
pub fn spawn(work: impl Future<Output = ()> + 'static) {
    glib::MainContext::default().spawn_local(work);
}

/// The response id of an alert's confirming button.
pub const CONFIRM: &str = "confirm";
/// The response id of an alert's cancel button.
pub const CANCEL: &str = "cancel";

/// A question with a Cancel and one confirming button (`destructive` paints it red). `confirmed`
/// runs only when that button is chosen; Escape and Cancel do nothing.
pub fn ask(
    parent: &impl IsA<gtk::Widget>,
    heading: &str,
    body: &str,
    cancel: &str,
    confirm: &str,
    destructive: bool,
    confirmed: impl Fn() + 'static,
) -> adw::AlertDialog {
    let dialog = adw::AlertDialog::new(Some(heading), (!body.is_empty()).then_some(body));
    dialog.add_response(CANCEL, cancel);
    dialog.add_response(CONFIRM, confirm);
    dialog.set_response_appearance(
        CONFIRM,
        if destructive {
            adw::ResponseAppearance::Destructive
        } else {
            adw::ResponseAppearance::Suggested
        },
    );
    dialog.set_close_response(CANCEL);
    dialog.set_default_response(Some(if destructive { CANCEL } else { CONFIRM }));
    dialog.connect_response(None, move |_, response| {
        if response == CONFIRM {
            confirmed();
        }
    });
    dialog.present(Some(parent));
    dialog
}

/// A statement with one OK button.
pub fn tell(parent: &impl IsA<gtk::Widget>, heading: &str, body: &str, ok: &str) {
    let dialog = adw::AlertDialog::new(Some(heading), (!body.is_empty()).then_some(body));
    dialog.add_response(CANCEL, ok);
    dialog.set_close_response(CANCEL);
    dialog.present(Some(parent));
}

/// A person's or a family's face: initials over the name's own colour (libadwaita's), `size`
/// logical pixels. The picture over it is P2's (`AvatarCache`, §8.3).
pub fn avatar(name: &str, size: i32) -> adw::Avatar {
    adw::Avatar::new(size, Some(name), true)
}

/// `face` in a round frame that carries a hero's shadow (`board::css`, `fc-hero-face`) — the face
/// at the top of Settings and of the Family dialog: an avatar draws no box-shadow of its own.
pub fn hero_face(face: &adw::Avatar) -> adw::Bin {
    let frame = adw::Bin::builder()
        .child(face)
        .halign(gtk::Align::Center)
        .build();
    frame.add_css_class("fc-hero-face");
    frame
}

/// A person's face: the initials of `name`, then `user_id`'s picture once it lands (P2's
/// `attachments::avatars`, keyed by user and the roster's avatar version, §8.3).
pub fn face(name: &str, user_id: i64, size: i32) -> adw::Avatar {
    let face = avatar(name, size);
    crate::attachments::avatars::show_member(&face, user_id);
    face
}

/// The accent-tinted "Owner" pill beside a name (the Mac's capsule).
pub fn owner_pill(text: &str) -> gtk::Label {
    let pill = gtk::Label::new(Some(text));
    pill.add_css_class("fc-owner-pill");
    pill.set_valign(gtk::Align::Center);
    pill
}

/// A dimmed, wrapping sentence under a group or a field (the Mac's `.footnote`).
pub fn footnote(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_xalign(0.0);
    label.add_css_class("dim-label");
    label.add_css_class("caption");
    label.set_margin_top(6);
    label
}

/// A sentence of a failure, red, wrapping — hidden while empty.
pub fn problem_label() -> gtk::Label {
    let label = gtk::Label::new(None);
    label.set_wrap(true);
    label.set_xalign(0.0);
    label.add_css_class("error");
    label.set_visible(false);
    label
}

/// Shows `text` in a [`problem_label`] (an empty text hides it).
pub fn show_problem(label: &gtk::Label, text: &str) {
    label.set_text(text);
    label.set_visible(!text.is_empty());
}

/// A toast over `overlay` that leaves on its own.
pub fn toast(overlay: &adw::ToastOverlay, text: &str) {
    let toast = adw::Toast::new(text);
    toast.set_timeout(5);
    overlay.add_toast(toast);
}

/// A row that does one thing when activated (libadwaita's button row), `destructive` in red.
pub fn button_row(title: &str, destructive: bool, run: impl Fn() + 'static) -> adw::ButtonRow {
    let row = adw::ButtonRow::builder().title(title).build();
    if destructive {
        row.add_css_class("destructive-action");
    }
    row.connect_activated(move |_| run());
    row
}

/// A row that shows a value under its title (libadwaita's `property` look), the value selectable.
pub fn property_row(title: &str, value: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(title)
        .subtitle(value)
        .subtitle_selectable(true)
        .build();
    row.add_css_class("property");
    row
}

/// A dialog's bottom bar: Cancel at the start, the confirming button at the end.
pub fn dialog_bar(
    dialog: &adw::Dialog,
    cancel: &str,
    confirm: &str,
    destructive: bool,
) -> (adw::HeaderBar, gtk::Button) {
    let header = adw::HeaderBar::new();
    header.set_show_end_title_buttons(false);
    header.set_show_start_title_buttons(false);
    let cancel_button = gtk::Button::with_label(cancel);
    let weak = dialog.downgrade();
    cancel_button.connect_clicked(move |_| {
        if let Some(dialog) = weak.upgrade() {
            dialog.close();
        }
    });
    let confirm_button = gtk::Button::with_label(confirm);
    confirm_button.add_css_class(if destructive {
        "destructive-action"
    } else {
        "suggested-action"
    });
    header.pack_start(&cancel_button);
    header.pack_end(&confirm_button);
    (header, confirm_button)
}

/// A dialog of `width` × `height` holding `content` under `header` in a toolbar view.
pub fn dialog(
    title: &str,
    width: i32,
    height: i32,
) -> (adw::Dialog, adw::ToolbarView, adw::ToastOverlay) {
    let dialog = adw::Dialog::builder()
        .title(title)
        .content_width(width)
        .content_height(height)
        .build();
    let view = adw::ToolbarView::new();
    let overlay = adw::ToastOverlay::new();
    view.set_content(Some(&overlay));
    dialog.set_child(Some(&view));
    (dialog, view, overlay)
}

/// A shared "a call is under way" flag for a dialog: the confirming button insensitive while it
/// holds.
pub fn busy(button: &gtk::Button) -> Rc<dyn Fn(bool)> {
    let button = button.clone();
    Rc::new(move |busy: bool| button.set_sensitive(!busy))
}

/// The window a P4 dialog is presented over when nothing nearer asked: the main window.
pub fn main_parent() -> Option<gtk::Widget> {
    crate::shell::Shell::try_get()
        .and_then(|shell| shell.main_window())
        .map(|main| main.window().clone().upcast())
}
