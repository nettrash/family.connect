//! The Call window (shell-design §14.3, §3.2): the Mac's own window for a call — 780 × 560, at
//! least 480 × 400, ONE for the process — drawing a [`CallView`] and handing its discs back to the
//! hub. It knows nothing of the engine: what it shows is decided by `call_view`, what a disc does
//! by the hub.
//!
//! ```text
//! adw::ApplicationWindow  "Bob"                       (the person — the Mac's navigationTitle)
//! └ adw::ToolbarView: adw::HeaderBar
//!   └ gtk::Overlay
//!     ├ gtk::GraphicsOffload → gtk::Picture           the far side, ContentFit::Contain
//!     ├ column: avatar 96 · name · status · note      a voice call, and a video call until the
//!     │         …                                     far side's picture is up
//!     │         gtk::Stack: incoming | active | ended the discs (56 px, a label under each)
//!     ├ pill: name · status                           top-left, while the picture is up
//!     └ self-view (160 × 120, mirrored by the media)  top-right, while the camera is on
//! ```
//!
//! **Closing** hides it (`hide-on-close`): the call goes on, Return to Call brings it back (§3.3,
//! the Mac: "closing it does not hang up"). Quitting destroys it with every other window
//! (`Shell::quit`). **Keys** (the Mac's ring: `.keyboardShortcut(.cancelAction)` on Decline,
//! `.defaultAction` on Accept): while it rings, Accept has the focus — Enter answers (Windows does
//! the same) — and Escape declines, in a bubble-phase key controller (Escape is never an
//! accelerator, §3.4). In a call no key ends anything: Tab and Space reach every disc.

use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use gtk::gdk;

use super::call_view::{CallView, Look, Role, Row};
use crate::theme::palette::Palette;

/// The Mac's `.defaultSize(width: 780, height: 560)`.
pub const DEFAULT_SIZE: (i32, i32) = (780, 560);
/// The Mac's `.frame(minWidth: 480, minHeight: 400)`.
pub const MINIMUM_SIZE: (i32, i32) = (480, 400);
/// The avatar's side (§14.3).
const AVATAR: i32 = 96;
/// A disc's side (the Mac's video disc; §14.3: 56 px).
const DISC: i32 = 56;
/// The share of the active row one disc gets (the Mac's `slotWidth`).
const SLOT: i32 = 100;
/// The self-view's tile: the camera's 4:3 picture, 160 wide (§14.3).
const SELF_VIEW: (i32, i32) = (160, 120);

/// The window's own classes (`theme::register`).
pub fn css(palette: &Palette) -> String {
    let accent_bg = palette.accent_bg.hex();
    let accent_fg = palette.accent_fg.hex();
    format!(
        "window.fc-call-video,
window.fc-call-video headerbar {{
  background-color: #000000;
  color: #ffffff;
}}
window.fc-call-video headerbar {{
  box-shadow: none;
}}
.fc-call-name {{
  font-weight: 600;
}}
.fc-call-status,
.fc-call-pill-status {{
  font-feature-settings: \"tnum\";
}}
.fc-call-pill {{
  background-color: rgba(0, 0, 0, 0.55);
  color: #ffffff;
  border-radius: 18px;
  padding: 7px 12px;
}}
.fc-call-self {{
  border-radius: 12px;
  box-shadow: 0 0 0 1px rgba(255, 255, 255, 0.25);
  background-color: #000000;
}}
.fc-call-plate {{
  background-color: rgba(0, 0, 0, 0.45);
  border-radius: 28px;
  padding: 12px;
}}
button.fc-call-disc {{
  min-width: {DISC}px;
  min-height: {DISC}px;
  padding: 0;
  border-radius: 9999px;
}}
button.fc-call-disc image {{
  -gtk-icon-size: 20px;
}}
button.fc-call-toggle {{
  background-color: alpha(currentColor, 0.08);
  box-shadow: inset 0 0 0 1px alpha(currentColor, 0.15);
}}
button.fc-call-toggle:hover {{
  background-color: alpha(currentColor, 0.14);
}}
button.fc-call-active {{
  background-color: {accent_bg};
  color: {accent_fg};
}}
button.fc-call-hangup {{
  background-color: var(--destructive-bg-color);
  color: var(--destructive-fg-color);
}}
button.fc-call-accept {{
  background-color: var(--success-bg-color);
  color: var(--success-fg-color);
}}
button.fc-call-active:hover,
button.fc-call-hangup:hover,
button.fc-call-accept:hover {{
  background-image: image(alpha(currentColor, 0.12));
}}
button.fc-call-disc:disabled {{
  opacity: 0.45;
}}
"
    )
}

/// One disc: the round button and the word under it.
struct DiscWidgets {
    column: gtk::Box,
    button: gtk::Button,
    icon: gtk::Image,
    label: gtk::Label,
}

/// The Call window (see the module's comment).
pub struct CallWindow {
    window: adw::ApplicationWindow,
    picture: gtk::Picture,
    offload: gtk::GraphicsOffload,
    identity: gtk::Box,
    avatar: adw::Avatar,
    name: gtk::Label,
    status: gtk::Label,
    note: gtk::Label,
    pill: gtk::Box,
    pill_name: gtk::Label,
    pill_status: gtk::Label,
    self_frame: gtk::Box,
    self_picture: gtk::Picture,
    rows: gtk::Stack,
    active_row: gtk::Box,
    discs: HashMap<Role, DiscWidgets>,
}

impl CallWindow {
    /// Builds the window (not presented). `pressed` is told which disc was pressed; `escape` is
    /// Escape while it rings.
    pub fn build(
        app: &adw::Application,
        camera_note: &str,
        pressed: Rc<dyn Fn(Role)>,
        escape: Rc<dyn Fn()>,
    ) -> CallWindow {
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .default_width(DEFAULT_SIZE.0)
            .default_height(DEFAULT_SIZE.1)
            .hide_on_close(true)
            .build();
        window.set_size_request(MINIMUM_SIZE.0, MINIMUM_SIZE.1);

        let picture = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Contain)
            .can_shrink(true)
            .hexpand(true)
            .vexpand(true)
            .build();
        let offload = gtk::GraphicsOffload::new(Some(&picture));
        offload.set_black_background(true);
        offload.set_visible(false);

        let avatar = adw::Avatar::new(AVATAR, None, true);
        let name = gtk::Label::builder()
            .css_classes(["title-1", "fc-call-name"])
            .wrap(true)
            .justify(gtk::Justification::Center)
            .build();
        let status = gtk::Label::builder()
            .css_classes(["heading", "dim-label", "fc-call-status"])
            .wrap(true)
            .justify(gtk::Justification::Center)
            .build();
        let note = gtk::Label::builder()
            .label(camera_note)
            .css_classes(["caption", "dim-label"])
            .wrap(true)
            .justify(gtk::Justification::Center)
            .visible(false)
            .build();
        let identity = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(4)
            .halign(gtk::Align::Center)
            .margin_start(32)
            .margin_end(32)
            .build();
        avatar.set_margin_bottom(8);
        avatar.set_halign(gtk::Align::Center);
        identity.append(&avatar);
        identity.append(&name);
        identity.append(&status);
        identity.append(&note);

        let mut discs = HashMap::new();
        for role in Role::ALL {
            discs.insert(role, disc(role, &pressed));
        }
        let incoming_row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(64)
            .halign(gtk::Align::Center)
            .build();
        incoming_row.append(&discs[&Role::Decline].column);
        incoming_row.append(&discs[&Role::Accept].column);
        let active_row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .homogeneous(true)
            .halign(gtk::Align::Center)
            .build();
        for role in [Role::Mute, Role::Camera, Role::HangUp] {
            let widgets = &discs[&role];
            widgets.column.set_width_request(SLOT);
            active_row.append(&widgets.column);
        }
        let rows = gtk::Stack::builder()
            .vhomogeneous(true)
            .hhomogeneous(false)
            .halign(gtk::Align::Center)
            .margin_top(12)
            .margin_bottom(28)
            .build();
        rows.add_named(&incoming_row, Some(Row::Incoming.name()));
        rows.add_named(&active_row, Some(Row::Active.name()));
        rows.add_named(
            &gtk::Box::new(gtk::Orientation::Horizontal, 0),
            Some(Row::Ended.name()),
        );

        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let above = gtk::Box::new(gtk::Orientation::Vertical, 0);
        above.set_vexpand(true);
        let below = gtk::Box::new(gtk::Orientation::Vertical, 0);
        below.set_vexpand(true);
        column.append(&above);
        column.append(&identity);
        column.append(&below);
        column.append(&rows);

        let pill_name = gtk::Label::builder()
            .css_classes(["fc-call-name"])
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .xalign(0.0)
            .build();
        let pill_status = gtk::Label::builder()
            .css_classes(["caption", "fc-call-pill-status"])
            .xalign(0.0)
            .build();
        let pill = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(1)
            .css_classes(["fc-call-pill"])
            .halign(gtk::Align::Start)
            .valign(gtk::Align::Start)
            .margin_start(20)
            .margin_top(20)
            // The self-view's column is kept free on the right.
            .margin_end(SELF_VIEW.0 + 32)
            .visible(false)
            .build();
        pill.append(&pill_name);
        pill.append(&pill_status);

        let self_picture = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Contain)
            .can_shrink(true)
            .width_request(SELF_VIEW.0)
            .height_request(SELF_VIEW.1)
            .build();
        let self_frame = gtk::Box::builder()
            .css_classes(["fc-call-self"])
            .halign(gtk::Align::End)
            .valign(gtk::Align::Start)
            .margin_top(20)
            .margin_end(20)
            .overflow(gtk::Overflow::Hidden)
            .visible(false)
            .build();
        // A picture asks for its paintable's own size (640 × 480): the clamp holds it to the tile's
        // width, and the picture's height follows from its aspect.
        let self_clamp = adw::Clamp::builder()
            .orientation(gtk::Orientation::Horizontal)
            .maximum_size(SELF_VIEW.0)
            .tightening_threshold(SELF_VIEW.0)
            .child(&self_picture)
            .build();
        self_frame.append(&self_clamp);

        let stage = gtk::Overlay::new();
        stage.set_child(Some(&offload));
        stage.add_overlay(&column);
        stage.add_overlay(&pill);
        stage.add_overlay(&self_frame);
        // The column fills the window even while the picture is hidden.
        stage.set_measure_overlay(&column, true);

        let header = adw::HeaderBar::new();
        let view = adw::ToolbarView::new();
        view.add_top_bar(&header);
        view.set_content(Some(&stage));
        window.set_content(Some(&view));

        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Bubble);
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape {
                escape();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        window.add_controller(keys);

        CallWindow {
            window,
            picture,
            offload,
            identity,
            avatar,
            name,
            status,
            note,
            pill,
            pill_name,
            pill_status,
            self_frame,
            self_picture,
            rows,
            active_row,
            discs,
        }
    }

    pub fn window(&self) -> &adw::ApplicationWindow {
        &self.window
    }

    /// The face of the person called: its text is set by [`show`](Self::show); the picture is
    /// P2's (`attachments::avatars`), set by the hub for the call's peer.
    pub fn avatar(&self) -> &adw::Avatar {
        &self.avatar
    }

    /// Draws `view`, with the media's two pictures as they are now.
    pub fn show(
        &self,
        view: &CallView,
        remote: Option<gdk::Paintable>,
        own: Option<gdk::Paintable>,
    ) {
        if self.window.title().as_deref() != Some(view.title.as_str()) {
            self.window.set_title(Some(&view.title));
        }
        if view.video {
            self.window.add_css_class("fc-call-video");
            self.active_row.add_css_class("fc-call-plate");
        } else {
            self.window.remove_css_class("fc-call-video");
            self.active_row.remove_css_class("fc-call-plate");
        }
        set_label(&self.name, &view.name);
        set_label(&self.pill_name, &view.name);
        if self.avatar.text().as_deref() != Some(view.name.as_str()) {
            self.avatar.set_text(Some(&view.name));
        }
        self.set_status(&view.status);

        let remote = remote.filter(|_| view.picture);
        if self.picture.paintable() != remote {
            self.picture.set_paintable(remote.as_ref());
        }
        self.offload.set_visible(view.picture);
        self.identity.set_visible(!view.picture);
        self.pill.set_visible(view.picture);
        self.note.set_visible(view.camera_note);

        let own = own.filter(|_| view.self_view);
        if self.self_picture.paintable() != own {
            self.self_picture.set_paintable(own.as_ref());
        }
        self.self_frame.set_visible(view.self_view);

        if self.rows.visible_child_name().as_deref() != Some(view.row.name()) {
            self.rows.set_visible_child_name(view.row.name());
        }
        for (role, widgets) in &self.discs {
            match view.discs.iter().find(|disc| disc.role == *role) {
                Some(disc) => {
                    widgets.column.set_visible(true);
                    widgets.icon.set_icon_name(Some(disc.icon));
                    set_label(&widgets.label, &disc.label);
                    widgets.button.set_tooltip_text(Some(&disc.label));
                    widgets
                        .button
                        .update_property(&[gtk::accessible::Property::Label(&disc.label)]);
                    for (look, class) in LOOKS {
                        if look == disc.look {
                            widgets.button.add_css_class(class);
                        } else {
                            widgets.button.remove_css_class(class);
                        }
                    }
                    widgets.button.set_sensitive(disc.sensitive);
                }
                None => widgets.column.set_visible(false),
            }
        }
    }

    /// The status line alone — the once-a-second redraw of a call's duration.
    pub fn set_status(&self, status: &str) {
        set_label(&self.status, status);
        set_label(&self.pill_status, status);
    }

    /// Accept takes the focus while it rings: Enter answers it, and a screen reader reads the
    /// call (Windows' `CallCardView`, the Mac's default action).
    pub fn focus_accept(&self) {
        self.discs[&Role::Accept].button.grab_focus();
    }

    /// Says `line` to a screen reader (§24: the polite line).
    pub fn announce(&self, line: &str) {
        self.status
            .announce(line, gtk::AccessibleAnnouncementPriority::Medium);
    }

    pub fn present(&self) {
        self.window.present();
    }

    pub fn hide(&self) {
        self.window.set_visible(false);
    }

    pub fn is_visible(&self) -> bool {
        self.window.is_visible()
    }
}

/// The self-test's eyes on the window.
#[cfg(feature = "selftest")]
impl CallWindow {
    /// The disc of `role`.
    pub fn disc(&self, role: Role) -> gtk::Button {
        self.discs[&role].button.clone()
    }

    /// The labels of the discs shown, in their row's order.
    pub fn shown_discs(&self) -> Vec<String> {
        let row = match self.rows.visible_child_name().as_deref() {
            Some("incoming") => vec![Role::Decline, Role::Accept],
            Some("active") => vec![Role::Mute, Role::Camera, Role::HangUp],
            _ => Vec::new(),
        };
        row.into_iter()
            .filter(|role| self.discs[role].column.is_visible())
            .map(|role| self.discs[&role].label.label().to_string())
            .collect()
    }

    /// The label of the disc holding the keyboard focus.
    /// The label of the disc the window gives the keyboard to (its focus widget, whether or not the
    /// window is active).
    pub fn focused_disc(&self) -> Option<String> {
        let focus = GtkWindowExt::focus(&self.window)?;
        self.discs
            .values()
            .find(|widgets| focus == widgets.button.clone().upcast::<gtk::Widget>())
            .map(|widgets| widgets.label.label().to_string())
    }

    pub fn status(&self) -> String {
        self.status.label().to_string()
    }

    pub fn name(&self) -> String {
        self.name.label().to_string()
    }

    /// The far side's picture is on screen, with a paintable in it.
    pub fn picture_shown(&self) -> bool {
        self.offload.is_visible() && self.picture.paintable().is_some()
    }

    /// The self-view is on screen, with a paintable in it.
    pub fn self_view_shown(&self) -> bool {
        self.self_frame.is_visible() && self.self_picture.paintable().is_some()
    }

    pub fn identity_shown(&self) -> bool {
        self.identity.is_visible()
    }

    pub fn note_shown(&self) -> bool {
        self.note.is_visible()
    }

    pub fn is_dark(&self) -> bool {
        self.window.has_css_class("fc-call-video")
    }
}

/// Each look's class.
const LOOKS: [(Look, &str); 4] = [
    (Look::Toggle, "fc-call-toggle"),
    (Look::Active, "fc-call-active"),
    (Look::HangUp, "fc-call-hangup"),
    (Look::Accept, "fc-call-accept"),
];

fn disc(role: Role, pressed: &Rc<dyn Fn(Role)>) -> DiscWidgets {
    let icon = gtk::Image::new();
    let button = gtk::Button::builder()
        .child(&icon)
        .css_classes(["circular", "fc-call-disc"])
        .width_request(DISC)
        .height_request(DISC)
        .halign(gtk::Align::Center)
        .build();
    let pressed = Rc::clone(pressed);
    button.connect_clicked(move |_| pressed(role));
    // The words are the disc's own name already: a screen reader need not read them twice.
    let label = gtk::Label::builder()
        .css_classes(["caption"])
        .wrap(true)
        .justify(gtk::Justification::Center)
        .max_width_chars(12)
        .accessible_role(gtk::AccessibleRole::Presentation)
        .build();
    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .valign(gtk::Align::Start)
        .build();
    column.append(&button);
    column.append(&label);
    DiscWidgets {
        column,
        button,
        icon,
        label,
    }
}

fn set_label(label: &gtk::Label, text: &str) {
    if label.label().as_str() != text {
        label.set_label(text);
    }
}
