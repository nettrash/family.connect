//! The attachment viewer (`viewer::AttachmentViewer`, shell-design §8.2 — LINUX FOLLOWS THE MAC):
//! a message's photos and videos at full size in their own window (`windows::viewer_window`), one
//! at a time, paging through the message's album with ‹ › and the arrow keys (stopping at the
//! ends), Save… (the ORIGINAL, where the person chooses — a failure said in a toast), Copy (a
//! photo onto the clipboard), Open Full Screen (F11), and Escape leaving full screen — or, out of
//! it, closing the window (`MediaAlbum::key`).
//!
//! **What decides** is fc-logic's `MediaAlbum`: paging that stops at the ends, a photo's zoom from
//! 1× to 6× (a pinch through `gtk::GestureZoom` relative to where IT started — the Mac's fix —
//! Ctrl+scroll, Ctrl+plus/minus/0, the zoom buttons, a double click toggling 1×/2×), a page turn
//! starting the next picture at fit, the title ("2 of 5", else the item's name: "Sticker", "Video
//! message", "GIF" for what the wire calls a photo or a video).
//!
//! **A video** plays through P3's player (`Shell::players`), streamed from the server with
//! authorised Range reads (`MediaSource::Stream`, the session's bearer), one thing playing at a
//! time (`NowPlaying`, `Playing::Video`); an ANIMATION plays muted and looping with no controls; a
//! video message in its circle. The player stops when the window closes or the page turns.
//!
//! **Keyed by (message, index)** as the Mac's `WindowGroup(for: album)`: asking twice presents the
//! window already open, and paging does not move the key. No Share: Linux has no share sheet (D12).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use fc_core::protocol::{AttachmentDto, MessageDto, TokenStore};
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::media_album::{MediaAlbum, ViewerKey};
use fc_logic::round_look::Playing;
use gtk::gdk;

use crate::attachments::pictures;
use crate::attachments::saving::{self, Outcome};
use crate::seams::media::{MediaSource, Player, PlayerState};
use crate::shell::Shell;
use crate::windows::viewer_window::{
    PAGE_FAILED, PAGE_LOADING, PAGE_PHOTO, PAGE_VIDEO, ViewerWindow,
};

/// Which message and which of its items a viewer was opened at.
pub type ViewerKeyed = (i64, usize);

thread_local! {
    static OPEN: RefCell<Vec<(ViewerKeyed, Rc<Viewer>)>> = const { RefCell::new(Vec::new()) };
}

/// Opens (or presents) the viewer on `message`'s photos and videos at `index`.
pub fn open_message(message: &MessageDto, index: usize) {
    let items = MediaAlbum::of_message(message);
    let round = message.round_video().is_some();
    open(items, index, round, (message.id, index));
}

/// Opens (or presents) a viewer on `items` at `index` under `key` (P1's "Open Full Screen", P3's
/// expand on a circle with `round`).
pub fn open(items: Vec<AttachmentDto>, index: usize, round: bool, key: ViewerKeyed) {
    if let Some(open) = find(key) {
        open.window.window.present();
        return;
    }
    let Ok(album) = MediaAlbum::new(items, i64::try_from(index).unwrap_or(0), round) else {
        return;
    };
    let viewer = Viewer::new(album, key);
    OPEN.with(|open| open.borrow_mut().push((key, Rc::clone(&viewer))));
    viewer.show_item();
    viewer.window.window.present();
}

/// The viewer open under `key`, if any.
pub fn find(key: ViewerKeyed) -> Option<Rc<Viewer>> {
    OPEN.with(|open| {
        open.borrow()
            .iter()
            .find(|(held, _)| *held == key)
            .map(|(_, viewer)| Rc::clone(viewer))
    })
}

/// One viewer window and its state.
pub struct Viewer {
    pub window: ViewerWindow,
    key: ViewerKeyed,
    album: RefCell<MediaAlbum>,
    /// Counts page turns, so a picture that lands for a page already left is dropped.
    turn: Cell<u64>,
    player: RefCell<Option<Rc<dyn Player>>>,
    claim: Cell<Option<u64>>,
    /// The full picture shown (Copy hands it over).
    texture: RefCell<Option<gdk::Texture>>,
    /// The zoom when a pinch began.
    pinch_from: Cell<f64>,
    /// The scrubber is being moved by the player, not the person.
    seeking_quietly: Cell<bool>,
}

impl Viewer {
    fn new(album: MediaAlbum, key: ViewerKeyed) -> Rc<Viewer> {
        let viewer = Rc::new(Viewer {
            window: ViewerWindow::build(),
            key,
            album: RefCell::new(album),
            turn: Cell::new(0),
            player: RefCell::new(None),
            claim: Cell::new(None),
            texture: RefCell::new(None),
            pinch_from: Cell::new(1.0),
            seeking_quietly: Cell::new(false),
        });
        viewer.connect();
        viewer
    }

    /// The album as it stands (the self-test reads it).
    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "the attachments and chat self-tests read it")
    )]
    pub fn album(&self) -> MediaAlbum {
        self.album.borrow().clone()
    }

    /// The video's player as it stands (`None` for a photo) — the self-test's eyes on playback.
    #[cfg(feature = "selftest")]
    pub fn playback(&self) -> Option<PlayerState> {
        self.player.borrow().as_ref().map(|player| player.state())
    }

    /// Whether the video's picture is on screen (the player's paintable in the window).
    #[cfg(feature = "selftest")]
    pub fn showing_video(&self) -> bool {
        self.window.stack.visible_child_name().as_deref() == Some(PAGE_VIDEO)
            && self.window.video.paintable().is_some()
    }

    /// Whether the photo being shown has landed.
    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "the attachments self-test reads it")
    )]
    pub fn showing_picture(&self) -> bool {
        self.window.stack.visible_child_name().as_deref() == Some(PAGE_PHOTO)
            && self.window.photo.paintable().is_some()
    }

    fn connect(self: &Rc<Self>) {
        let w = &self.window;
        let weak = Rc::downgrade(self);
        let on = move |run: fn(&Rc<Viewer>)| {
            let weak = weak.clone();
            move || {
                if let Some(viewer) = weak.upgrade() {
                    run(&viewer);
                }
            }
        };
        let previous = on(|v| v.step(-1));
        w.previous.connect_clicked(move |_| previous());
        let next = on(|v| v.step(1));
        w.next.connect_clicked(move |_| next());
        let save = on(Viewer::save);
        w.save.connect_clicked(move |_| save());
        let copy = on(Viewer::copy);
        w.copy.connect_clicked(move |_| copy());
        let zoom_in = on(|v| v.zoom_with(MediaAlbum::zoom_in));
        w.zoom_in.connect_clicked(move |_| zoom_in());
        let zoom_out = on(|v| v.zoom_with(MediaAlbum::zoom_out));
        w.zoom_out.connect_clicked(move |_| zoom_out());
        let full_screen = on(Viewer::toggle_full_screen);
        w.full_screen.connect_clicked(move |_| full_screen());
        let play = on(Viewer::toggle_play);
        w.play.connect_clicked(move |_| play());
        {
            let weak = Rc::downgrade(self);
            w.mute.connect_toggled(move |button| {
                if let Some(viewer) = weak.upgrade()
                    && let Some(player) = viewer.player.borrow().as_ref()
                {
                    player.set_muted(button.is_active());
                }
            });
        }
        {
            let weak = Rc::downgrade(self);
            w.scrubber.connect_value_changed(move |scale| {
                if let Some(viewer) = weak.upgrade()
                    && !viewer.seeking_quietly.get()
                    && let Some(player) = viewer.player.borrow().as_ref()
                {
                    player.seek(scale.value());
                }
            });
        }

        // Keys: ← → page, Escape leaves full screen or closes, Space plays, F11 full screen,
        // Ctrl+plus/minus/0 zoom. Captured, so a focused button does not take Space or an arrow.
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        {
            let weak = Rc::downgrade(self);
            keys.connect_key_pressed(move |_, key, _, state| {
                let Some(viewer) = weak.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                if viewer.key(key, state) {
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            });
        }
        w.window.add_controller(keys);

        // A pinch is relative to where IT began (the Mac's fix: used as the zoom itself, every pinch
        // snapped the picture back to 1× before growing again).
        let pinch = gtk::GestureZoom::new();
        {
            let weak = Rc::downgrade(self);
            pinch.connect_begin(move |_, _| {
                if let Some(viewer) = weak.upgrade() {
                    viewer.pinch_from.set(viewer.album.borrow().zoom());
                }
            });
        }
        {
            let weak = Rc::downgrade(self);
            pinch.connect_scale_changed(move |_, scale| {
                if let Some(viewer) = weak.upgrade() {
                    let from = viewer.pinch_from.get();
                    viewer.zoom_with(move |album| album.zoomed_to(from * scale));
                }
            });
        }
        w.scrolled.add_controller(pinch);
        let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        {
            let weak = Rc::downgrade(self);
            wheel.connect_scroll(move |controller, _, dy| {
                let ctrl = controller
                    .current_event_state()
                    .contains(gdk::ModifierType::CONTROL_MASK);
                let Some(viewer) = weak.upgrade().filter(|_| ctrl) else {
                    return glib::Propagation::Proceed;
                };
                if dy < 0.0 {
                    viewer.zoom_with(MediaAlbum::zoom_in);
                } else if dy > 0.0 {
                    viewer.zoom_with(MediaAlbum::zoom_out);
                }
                glib::Propagation::Stop
            });
        }
        w.scrolled.add_controller(wheel);
        let double = gtk::GestureClick::new();
        {
            let weak = Rc::downgrade(self);
            double.connect_pressed(move |_, n_press, _, _| {
                if n_press == 2
                    && let Some(viewer) = weak.upgrade()
                {
                    viewer.zoom_with(MediaAlbum::toggle_zoom);
                }
            });
        }
        w.photo.add_controller(double);

        {
            let weak = Rc::downgrade(self);
            w.window.connect_fullscreened_notify(move |window| {
                if let Some(viewer) = weak.upgrade() {
                    let icon = if window.is_fullscreen() {
                        "view-restore-symbolic"
                    } else {
                        "view-fullscreen-symbolic"
                    };
                    viewer.window.full_screen.set_icon_name(icon);
                }
            });
        }
        {
            let weak = Rc::downgrade(self);
            w.window.connect_close_request(move |_| {
                if let Some(viewer) = weak.upgrade() {
                    viewer.closed();
                }
                glib::Propagation::Proceed
            });
        }
    }

    /// A key: whether it was this window's.
    fn key(self: &Rc<Self>, key: gdk::Key, state: gdk::ModifierType) -> bool {
        let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
        if ctrl {
            match key {
                gdk::Key::plus | gdk::Key::equal | gdk::Key::KP_Add => {
                    self.zoom_with(MediaAlbum::zoom_in)
                }
                gdk::Key::minus | gdk::Key::KP_Subtract => self.zoom_with(MediaAlbum::zoom_out),
                gdk::Key::_0 | gdk::Key::KP_0 => self.zoom_with(|album| album.zoomed_to(1.0)),
                _ => return false,
            }
            return true;
        }
        match key {
            gdk::Key::F11 => {
                self.toggle_full_screen();
                return true;
            }
            gdk::Key::space if self.album.borrow().is_video() => {
                self.toggle_play();
                return true;
            }
            _ => {}
        }
        let name = key.name().map(|n| n.to_string()).unwrap_or_default();
        let wanted = self.album.borrow().key(&name);
        match wanted {
            ViewerKey::Close => {
                if self.window.window.is_fullscreen() {
                    self.window.window.unfullscreen();
                } else {
                    self.window.window.close();
                }
                true
            }
            ViewerKey::Previous => {
                self.step(-1);
                true
            }
            ViewerKey::Next => {
                self.step(1);
                true
            }
            ViewerKey::None => matches!(name.as_str(), "Left" | "Right"),
        }
    }

    /// One page in `direction`, stopping at the ends.
    pub fn step(self: &Rc<Self>, direction: i32) {
        if self.album.borrow_mut().step(direction) {
            self.show_item();
        }
    }

    /// Draws the album's current item: the title, the buttons, the picture or the video.
    pub fn show_item(self: &Rc<Self>) {
        self.turn.set(self.turn.get() + 1);
        self.stop_video();
        *self.texture.borrow_mut() = None;
        let shell = Shell::get();
        let say = &shell.say;
        let album = self.album.borrow().clone();
        let w = &self.window;
        let title = album
            .position(say.as_ref())
            .unwrap_or_else(|| album.title(say.as_ref()));
        w.title.set_title(&title);
        w.window.set_title(Some(&title));
        let paging = album.count() > 1;
        w.previous.set_visible(paging);
        w.next.set_visible(paging);
        w.previous.set_sensitive(album.has_previous());
        w.next.set_sensitive(album.has_next());
        let photo = !album.is_video();
        w.copy.set_visible(photo);
        w.zoom_in.set_visible(photo);
        w.zoom_out.set_visible(photo);
        self.apply_zoom();
        if photo {
            self.show_photo(album.current().clone());
        } else {
            self.show_video(&album);
        }
    }

    fn show_photo(self: &Rc<Self>, attachment: AttachmentDto) {
        let w = &self.window;
        w.photo.set_paintable(None::<&gdk::Paintable>);
        w.stack.set_visible_child_name(PAGE_LOADING);
        let turn = self.turn.get();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            // The small copy first when it is already here, so the window is never blank.
            if attachment.has_preview
                && !attachment.sticker
                && pictures::is_kept(&attachment, true, None)
                && let Some(viewer) = weak.upgrade()
                && let Some(small) = pictures::of_attachment(&attachment, true, None).await
                && viewer.turn.get() == turn
            {
                viewer.window.photo.set_paintable(Some(&small));
                viewer.window.stack.set_visible_child_name(PAGE_PHOTO);
            }
            let whole = pictures::of_attachment(&attachment, false, None).await;
            let Some(viewer) = weak.upgrade() else {
                return;
            };
            if viewer.turn.get() != turn {
                return;
            }
            match whole {
                Some(texture) => {
                    viewer.window.photo.set_paintable(Some(&texture));
                    *viewer.texture.borrow_mut() = Some(texture);
                    viewer.window.stack.set_visible_child_name(PAGE_PHOTO);
                }
                None if viewer.window.photo.paintable().is_none() => {
                    let shell = Shell::get();
                    viewer
                        .window
                        .failed
                        .set_title(shell.say.get("The file could not be downloaded."));
                    viewer.window.stack.set_visible_child_name(PAGE_FAILED);
                }
                None => {}
            }
        });
    }

    fn show_video(self: &Rc<Self>, album: &MediaAlbum) {
        let shell = Shell::get();
        let w = &self.window;
        let attachment = album.current().clone();
        let Some(connection) = shell.connection() else {
            w.stack.set_visible_child_name(PAGE_FAILED);
            return;
        };
        let quiet = album.is_animation();
        w.controls.set_visible(!quiet);
        // A circle held square, with air above it and the controls' room below; a film at its
        // own ratio, as `Contain` letterboxes it.
        let round = album.is_round();
        w.video_aspect.set_obey_child(!round);
        w.video_aspect.set_margin_top(if round { 24 } else { 0 });
        w.video_aspect.set_margin_bottom(if round { 76 } else { 0 });
        if round {
            w.video_frame.add_css_class("fc-round");
            w.video_frame.set_overflow(gtk::Overflow::Hidden);
            w.video.set_content_fit(gtk::ContentFit::Cover);
        } else {
            w.video_frame.remove_css_class("fc-round");
            w.video_frame.set_overflow(gtk::Overflow::Visible);
            w.video.set_content_fit(gtk::ContentFit::Contain);
        }
        w.stack.set_visible_child_name(PAGE_VIDEO);
        let player = shell.players.get().create(true);
        player.set_muted(quiet || w.mute.is_active());
        player.load(MediaSource::Stream {
            url: format!("{}/attachments/{}", connection.server.rest(), attachment.id),
            bearer: connection.tokens.token(),
        });
        if let Some(paintable) = player.paintable() {
            w.video.set_paintable(Some(&paintable));
        }
        let weak = Rc::downgrade(self);
        let turn = self.turn.get();
        player.on_state().subscribe(move |state| {
            let Some(viewer) = weak.upgrade().filter(|v| v.turn.get() == turn) else {
                return;
            };
            viewer.player_state(*state, quiet);
        });
        let weak = Rc::downgrade(self);
        player.on_position().subscribe(move |seconds| {
            if let Some(viewer) = weak.upgrade().filter(|v| v.turn.get() == turn) {
                viewer.player_position(*seconds);
            }
        });
        *self.player.borrow_mut() = Some(Rc::clone(&player));
        self.claim_playing();
        player.play();
    }

    fn player_state(self: &Rc<Self>, state: PlayerState, quiet: bool) {
        let shell = Shell::get();
        let say = &shell.say;
        let w = &self.window;
        match state {
            PlayerState::Playing => {
                w.play.set_icon_name("media-playback-pause-symbolic");
                w.play.set_tooltip_text(Some(say.get("Pause")));
                if let Some(paintable) = self.player.borrow().as_ref().and_then(|p| p.paintable()) {
                    w.video.set_paintable(Some(&paintable));
                }
            }
            PlayerState::Ended if quiet => {
                // An animation loops.
                if let Some(player) = self.player.borrow().as_ref() {
                    player.seek(0.0);
                    player.play();
                }
            }
            PlayerState::Failed => {
                self.release_playing();
                let album = self.album.borrow().clone();
                w.failed.set_icon_name(Some("video-x-generic-symbolic"));
                w.failed.set_title(&album.title(say.as_ref()));
                w.stack.set_visible_child_name(PAGE_FAILED);
            }
            PlayerState::Paused | PlayerState::Ended | PlayerState::Idle => {
                w.play.set_icon_name("media-playback-start-symbolic");
                w.play.set_tooltip_text(Some(say.get("Play")));
                if state != PlayerState::Paused {
                    self.release_playing();
                }
            }
            PlayerState::Loading => {}
        }
    }

    fn player_position(&self, seconds: f64) {
        let Some(player) = self.player.borrow().clone() else {
            return;
        };
        let duration = player.duration();
        let w = &self.window;
        self.seeking_quietly.set(true);
        if duration > 0.0 {
            w.scrubber.adjustment().set_upper(duration);
        }
        w.scrubber.set_value(seconds);
        self.seeking_quietly.set(false);
        w.time.set_label(&format!(
            "{} / {}",
            fc_text::media::time_label(seconds),
            fc_text::media::time_label(duration)
        ));
    }

    fn toggle_play(self: &Rc<Self>) {
        let Some(player) = self.player.borrow().clone() else {
            return;
        };
        if player.state() == PlayerState::Playing {
            player.pause();
        } else {
            self.claim_playing();
            player.play();
        }
    }

    /// One thing plays at a time: a circle or a voice note pauses this, and the other way round.
    fn claim_playing(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        let pause: Rc<dyn Fn()> = Rc::new(move || {
            if let Some(viewer) = weak.upgrade()
                && let Some(player) = viewer.player.borrow().as_ref()
            {
                player.pause();
            }
        });
        let kind = if self.album.borrow().is_round() {
            Playing::RoundVideo
        } else {
            Playing::Video
        };
        let claim = Shell::get().now_playing.get().claim(kind, pause);
        self.claim.set(Some(claim));
    }

    fn release_playing(&self) {
        if let Some(claim) = self.claim.take() {
            Shell::get().now_playing.get().release(claim);
        }
    }

    fn stop_video(&self) {
        if let Some(player) = self.player.borrow_mut().take() {
            player.stop();
        }
        self.release_playing();
        self.window.video.set_paintable(None::<&gdk::Paintable>);
    }

    /// Changes the zoom by `change` (a step, a toggle, a pinch) and draws it.
    pub fn zoom_with(self: &Rc<Self>, change: impl FnOnce(&mut MediaAlbum)) {
        if self.album.borrow().is_video() {
            return;
        }
        change(&mut self.album.borrow_mut());
        self.apply_zoom();
    }

    /// The picture at the window's size times the zoom — which is what makes 1× show all of it —
    /// scrollable only when there is something to scroll to; the middle stays where it was.
    fn apply_zoom(&self) {
        let album = self.album.borrow();
        let w = &self.window;
        let zoom = album.zoom();
        w.zoom_in.set_sensitive(album.can_zoom_in());
        w.zoom_out.set_sensitive(album.can_zoom_out());
        if zoom <= 1.0 {
            w.photo.set_size_request(-1, -1);
            w.scrolled.set_hscrollbar_policy(gtk::PolicyType::Never);
            w.scrolled.set_vscrollbar_policy(gtk::PolicyType::Never);
            return;
        }
        let (width, height) = (
            f64::from(w.scrolled.width().max(1)),
            f64::from(w.scrolled.height().max(1)),
        );
        let middle = |adjustment: &gtk::Adjustment| {
            let span = adjustment.upper() - adjustment.lower();
            if span > 0.0 {
                (adjustment.value() + adjustment.page_size() / 2.0) / span
            } else {
                0.5
            }
        };
        let (mx, my) = (
            middle(&w.scrolled.hadjustment()),
            middle(&w.scrolled.vadjustment()),
        );
        w.scrolled.set_hscrollbar_policy(gtk::PolicyType::Automatic);
        w.scrolled.set_vscrollbar_policy(gtk::PolicyType::Automatic);
        w.photo.set_size_request(
            pictures::logical(width * zoom),
            pictures::logical(height * zoom),
        );
        let scrolled = w.scrolled.clone();
        glib::idle_add_local_once(move || {
            for (adjustment, middle) in [(scrolled.hadjustment(), mx), (scrolled.vadjustment(), my)]
            {
                let span = adjustment.upper() - adjustment.lower();
                adjustment.set_value(middle * span - adjustment.page_size() / 2.0);
            }
        });
    }

    fn toggle_full_screen(self: &Rc<Self>) {
        let window = &self.window.window;
        if window.is_fullscreen() {
            window.unfullscreen();
        } else {
            window.fullscreen();
        }
    }

    /// Save…: the page up when the button was clicked, even if the person pages on meanwhile.
    fn save(self: &Rc<Self>) {
        let attachment = self.album.borrow().current().clone();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let Some(viewer) = weak.upgrade() else {
                return;
            };
            viewer.window.save.set_sensitive(false);
            let parent: gtk::Window = viewer.window.window.clone().upcast();
            let outcome = saving::save(Some(parent), &attachment).await;
            viewer.window.save.set_sensitive(true);
            if let Outcome::Failed(sentence) = outcome {
                viewer.window.toasts.add_toast(adw::Toast::new(&sentence));
            }
        });
    }

    /// Copy: the whole photo onto the clipboard.
    fn copy(self: &Rc<Self>) {
        if let Some(texture) = self.texture.borrow().as_ref() {
            self.window.window.clipboard().set_texture(texture);
        }
    }

    fn closed(&self) {
        self.stop_video();
        let key = self.key;
        OPEN.with(|open| open.borrow_mut().retain(|(held, _)| *held != key));
    }
}
