//! The attachment viewer's WINDOW (shell-design §3.2, §8.2; the Mac's `MacAttachmentViewer` scene,
//! `WindowGroup(for: album)` 900 × 700, at least 480 × 360): an `adw::ApplicationWindow` whose
//! `adw::ToolbarView` carries a header with ‹ › (insensitive at the ends, hidden when there is
//! nowhere to page), the title — "2 of 5" while there is somewhere to page, else the item's name —
//! and Save…, Copy, zoom out / in and Open Full Screen; under it, on the dark stage, a photo in a
//! `gtk::ScrolledWindow` (`ContentFit::Contain`, 1 = the whole picture fits), or a video with its
//! own controls bar (play/pause, the scrubber, the time, mute), or a spinner, or what could not be
//! shown. What it SHOWS and what its controls DO is `crate::viewer`'s; this file only builds it.

use adw::prelude::*;

use crate::shell::Shell;
use fc_core::text::string_catalog::StringCatalog;

/// The default size (the Mac's 900 × 700).
pub const DEFAULT_SIZE: (i32, i32) = (900, 700);
/// The smallest it may be (the Mac's 480 × 360).
pub const MINIMUM_SIZE: (i32, i32) = (480, 360);

/// The stack's pages.
pub const PAGE_PHOTO: &str = "photo";
pub const PAGE_VIDEO: &str = "video";
pub const PAGE_LOADING: &str = "loading";
pub const PAGE_FAILED: &str = "failed";

/// Every part of one viewer window.
pub struct ViewerWindow {
    pub window: adw::ApplicationWindow,
    pub title: adw::WindowTitle,
    pub previous: gtk::Button,
    pub next: gtk::Button,
    pub save: gtk::Button,
    pub copy: gtk::Button,
    pub zoom_out: gtk::Button,
    pub zoom_in: gtk::Button,
    pub full_screen: gtk::Button,
    pub toasts: adw::ToastOverlay,
    pub stack: gtk::Stack,
    pub scrolled: gtk::ScrolledWindow,
    pub photo: gtk::Picture,
    pub video_frame: gtk::Box,
    pub video: gtk::Picture,
    pub controls: gtk::Box,
    pub play: gtk::Button,
    pub scrubber: gtk::Scale,
    pub time: gtk::Label,
    pub mute: gtk::ToggleButton,
    pub failed: adw::StatusPage,
}

/// An icon button with its tooltip and its accessible name from the catalogue.
fn icon_button(icon: &str, name: &str) -> gtk::Button {
    let button = gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(name)
        .build();
    button.update_property(&[gtk::accessible::Property::Label(name)]);
    button
}

impl ViewerWindow {
    /// Builds a viewer window (not presented).
    pub fn build() -> ViewerWindow {
        let shell = Shell::get();
        let say = &shell.say;
        let window = adw::ApplicationWindow::builder()
            .application(&shell.app)
            .default_width(DEFAULT_SIZE.0)
            .default_height(DEFAULT_SIZE.1)
            .build();
        window.set_size_request(MINIMUM_SIZE.0, MINIMUM_SIZE.1);

        let previous = icon_button("go-previous-symbolic", say.get("Previous"));
        let next = icon_button("go-next-symbolic", say.get("Next"));
        let save = icon_button("document-save-symbolic", say.get("Save…"));
        let copy = icon_button("edit-copy-symbolic", say.get("Copy"));
        let zoom_out = icon_button("zoom-out-symbolic", say.get("Zoom out"));
        let zoom_in = icon_button("zoom-in-symbolic", say.get("Zoom in"));
        let full_screen = icon_button("view-fullscreen-symbolic", say.get("Open Full Screen"));
        let title = adw::WindowTitle::new("", "");
        let header = adw::HeaderBar::new();
        header.set_title_widget(Some(&title));
        header.pack_start(&previous);
        header.pack_start(&next);
        header.pack_end(&full_screen);
        header.pack_end(&save);
        header.pack_end(&copy);
        header.pack_end(&zoom_in);
        header.pack_end(&zoom_out);

        // The photo: sized to the window times the zoom, scrollable only when zoomed in.
        let photo = gtk::Picture::builder()
            .can_shrink(true)
            .content_fit(gtk::ContentFit::Contain)
            .hexpand(true)
            .vexpand(true)
            .build();
        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .child(&photo)
            .build();

        // The video and its controls bar.
        let video = gtk::Picture::builder()
            .can_shrink(true)
            .content_fit(gtk::ContentFit::Contain)
            .hexpand(true)
            .vexpand(true)
            .build();
        let video_frame = gtk::Box::builder()
            .halign(gtk::Align::Fill)
            .valign(gtk::Align::Fill)
            .hexpand(true)
            .vexpand(true)
            .build();
        video_frame.append(&video);
        let play = icon_button("media-playback-start-symbolic", say.get("Play"));
        play.add_css_class("flat");
        let scrubber = gtk::Scale::builder()
            .orientation(gtk::Orientation::Horizontal)
            .hexpand(true)
            .draw_value(false)
            .adjustment(&gtk::Adjustment::new(0.0, 0.0, 1.0, 1.0, 5.0, 0.0))
            .build();
        let time = gtk::Label::builder()
            .label("0:00")
            .css_classes(["numeric"])
            .build();
        let mute = gtk::ToggleButton::builder()
            .icon_name("audio-volume-high-symbolic")
            .tooltip_text(say.get("Mute"))
            .css_classes(["flat"])
            .build();
        mute.update_property(&[gtk::accessible::Property::Label(say.get("Mute"))]);
        let controls = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .margin_start(12)
            .margin_end(12)
            .margin_bottom(12)
            .valign(gtk::Align::End)
            .css_classes(["fc-viewer-controls"])
            .build();
        controls.append(&play);
        controls.append(&scrubber);
        controls.append(&time);
        controls.append(&mute);
        let video_page = gtk::Overlay::new();
        video_page.set_child(Some(&video_frame));
        video_page.add_overlay(&controls);

        let spinner = adw::Spinner::builder()
            .width_request(32)
            .height_request(32)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .build();
        let failed = adw::StatusPage::builder()
            .icon_name("image-missing-symbolic")
            .build();

        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::None)
            .hexpand(true)
            .vexpand(true)
            .css_classes(["fc-viewer-stage"])
            .build();
        stack.add_named(&scrolled, Some(PAGE_PHOTO));
        stack.add_named(&video_page, Some(PAGE_VIDEO));
        stack.add_named(&spinner, Some(PAGE_LOADING));
        stack.add_named(&failed, Some(PAGE_FAILED));
        stack.set_visible_child_name(PAGE_LOADING);

        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&stack));
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&toasts));
        window.set_content(Some(&toolbar));

        ViewerWindow {
            window,
            title,
            previous,
            next,
            save,
            copy,
            zoom_out,
            zoom_in,
            full_screen,
            toasts,
            stack,
            scrolled,
            photo,
            video_frame,
            video,
            controls,
            play,
            scrubber,
            time,
            mute,
            failed,
        }
    }
}
