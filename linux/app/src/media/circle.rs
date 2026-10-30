//! A received video message, in place (shell-design §10.4; `RoundPlayback`, `round_inline`,
//! `RoundLook`): the circle alone on the chat's ground, 240 across, the poster until the first frame,
//! a click (a beat late — a double click is the heart) plays it INSIDE its circle with sound; one
//! accent ring runs round OUTSIDE the edge as it plays (stepping once a second under Reduce Motion);
//! the end brings the poster back and takes the dot away; scrolled wholly out of view, unmapped
//! (the chat left, the window hidden) or replaced by another player, it stops.
//!
//! GTK clips the playing video round itself (measured, §1.5): a `gtk::Picture` of
//! `gtk4paintablesink`'s paintable (`ContentFit::Cover`) in a box with `fc-round` and
//! `overflow: hidden`. The picture's natural size is the video's (480), so the clip is held at the
//! diameter by two `adw::Clamp`s (one per orientation) over its size request — no widget subclass
//! (the workspace forbids `unsafe`, which the subclass macros expand to). The ring is drawn by the
//! PARENT, outside the clipped box, so the clip never cuts it.
//!
//! The MP4 is fetched through the attachment cache on the click only (and replays offline from
//! there); the poster is the attachment's preview, asked for only when it says it has one.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use fc_core::protocol::{AttachmentDto, MessageDto};
use fc_core::store::PlayedRoundStore;
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::round_look::{self, Playing};
use fc_logic::round_playback::{
    RoundAct, RoundPhase, RoundPlayback, RoundPlayer, RoundPlayerNews, RoundStep, RoundTap,
    round_inline,
};
use gtk::prelude::*;

use super::player::{GstRoundPlayer, GstRoundPlayers};
use crate::seams::media::NowPlaying;
use crate::shell::Shell;

thread_local! {
    /// Where "Open Full Screen" goes (P2's viewer, set by `attachments::install`): the message and
    /// the attachment. No hook: the expand control is not drawn.
    static OPEN_VIEWER: RefCell<Option<Rc<dyn Fn(&MessageDto, &AttachmentDto)>>> = const { RefCell::new(None) };
}

/// The viewer's door for "Open Full Screen" (P2 sets it in its `install` — W.3 has no route for it).
pub fn set_open_viewer(open: Rc<dyn Fn(&MessageDto, &AttachmentDto)>) {
    OPEN_VIEWER.with(|slot| *slot.borrow_mut() = Some(open));
}

/// One circle in a conversation.
pub struct Circle {
    clip: gtk::Overlay,
    picture: gtk::Picture,
    disc: gtk::Box,
    spinner: adw::Spinner,
    capsule: gtk::Label,
    dot: gtk::Box,
    expand: gtk::Button,
    ring: gtk::DrawingArea,
    failure: gtk::Label,
    message: MessageDto,
    attachment: AttachmentDto,
    mine: bool,
    played: Cell<bool>,
    machine: RefCell<RoundPlayback>,
    player: RefCell<Option<Rc<GstRoundPlayer>>>,
    poster: RefCell<Option<gtk::gdk::Texture>>,
    claim: Cell<Option<u64>>,
    tick: RefCell<Option<glib::SourceId>>,
    pending_click: RefCell<Option<glib::SourceId>>,
    scroll: RefCell<Option<(gtk::Adjustment, glib::SignalHandlerId)>>,
}

impl std::fmt::Debug for Circle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Circle")
            .field("attachment", &self.attachment.id)
            .field("phase", &self.machine.borrow().phase())
            .finish_non_exhaustive()
    }
}

/// `ConversationSlots::circle`: the widget, its final size already.
pub fn widget(message: &MessageDto, attachment: &AttachmentDto) -> gtk::Widget {
    let (circle, root) = Circle::new(message, attachment);
    let keep = RefCell::new(Some(circle));
    root.connect_destroy(move |_| {
        if let Some(circle) = keep.borrow_mut().take() {
            circle.stop();
        }
    });
    root.upcast()
}

/// The circle's diameter in a conversation.
pub fn diameter() -> f64 {
    round_look::DIAMETER
}

/// The ring's reach past the circle on each side.
pub fn reach() -> f64 {
    round_inline::RING_REACH
}

/// How far in from the clip's top and trailing edges the expand control of `target` px stands in
/// a circle of `side` px: on the circle's 45° line, wholly inside it and 8 px clear of the rim,
/// so the round clip never cuts it. Pure.
fn expand_inset(side: f64, target: f64) -> i32 {
    const CLEARANCE: f64 = 8.0;
    let (radius, half) = (side / 2.0, target / 2.0);
    let inset = radius - (radius - half - CLEARANCE) / std::f64::consts::SQRT_2 - half;
    // A small, non-negative number of pixels: the cast cannot truncate anything real.
    inset.round().max(0.0) as i32
}

impl Circle {
    pub fn new(message: &MessageDto, attachment: &AttachmentDto) -> (Rc<Circle>, gtk::Box) {
        let shell = Shell::get();
        let say = &shell.say;
        let mine = shell
            .connection()
            .and_then(|connection| connection.session.state().me)
            .is_some_and(|me| me.id == message.sender_id);
        let played = !mine
            && shell.connection().is_some_and(|connection| {
                PlayedRoundStore::new(&connection.cache)
                    .played(attachment.id)
                    .unwrap_or(false)
            });
        let side = diameter().round() as i32;
        let picture = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Cover)
            .can_shrink(true)
            .css_classes(["fc-media-poster"])
            .build();
        let disc = gtk::Box::builder()
            .width_request(round_look::PLAY_DISC as i32)
            .height_request(round_look::PLAY_DISC as i32)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .can_target(false)
            .css_classes(["fc-media-circle-disc"])
            .build();
        let glyph = gtk::Image::builder()
            .icon_name("media-playback-start-symbolic")
            .pixel_size(22)
            .hexpand(true)
            .halign(gtk::Align::Center)
            .build();
        disc.append(&glyph);
        let spinner = adw::Spinner::builder()
            .width_request(round_look::PLAY_DISC as i32)
            .height_request(round_look::PLAY_DISC as i32)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .can_target(false)
            .visible(false)
            .build();
        let capsule = gtk::Label::builder()
            .label(round_look::capsule(attachment).unwrap_or_default())
            .css_classes(["caption", "fc-media-capsule"])
            .can_target(false)
            .build();
        let dot = gtk::Box::builder()
            .width_request(round_look::DOT as i32)
            .height_request(round_look::DOT as i32)
            .valign(gtk::Align::Center)
            .css_classes(["fc-media-dot"])
            .visible(round_look::shows_dot(mine, played))
            .build();
        let foot = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(4)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::End)
            .margin_bottom(12)
            .can_target(false)
            .build();
        foot.append(&capsule);
        foot.append(&dot);
        let inset = expand_inset(f64::from(side), round_inline::EXPAND_TARGET);
        let expand = gtk::Button::builder()
            .icon_name("view-fullscreen-symbolic")
            .width_request(round_inline::EXPAND_TARGET as i32)
            .height_request(round_inline::EXPAND_TARGET as i32)
            .halign(gtk::Align::End)
            .valign(gtk::Align::Start)
            .margin_top(inset)
            .margin_end(inset)
            .visible(false)
            .css_classes(["circular", "osd"])
            .tooltip_text(say.get("Open Full Screen"))
            .build();
        expand.update_property(&[gtk::accessible::Property::Label(
            say.get("Open Full Screen"),
        )]);
        let clip = gtk::Overlay::builder()
            .width_request(side)
            .height_request(side)
            .overflow(gtk::Overflow::Hidden)
            .css_classes(["fc-round"])
            .accessible_role(gtk::AccessibleRole::Button)
            .focusable(true)
            .build();
        clip.set_child(Some(&picture));
        clip.add_overlay(&disc);
        clip.add_overlay(&spinner);
        clip.add_overlay(&foot);
        clip.add_overlay(&expand);
        clip.set_cursor_from_name(Some("pointer"));
        clip.update_property(&[gtk::accessible::Property::Label(&round_look::name(
            attachment,
            say.as_ref(),
        ))]);
        if let Some(value) = round_look::value(mine, played, say.as_ref()) {
            clip.update_property(&[gtk::accessible::Property::Description(&value)]);
        }
        // The picture's natural size is the video's: two clamps hold the clip at the diameter.
        let tall = adw::Clamp::builder()
            .orientation(gtk::Orientation::Vertical)
            .maximum_size(side)
            .tightening_threshold(side)
            .child(&clip)
            .build();
        let wide = adw::Clamp::builder()
            .orientation(gtk::Orientation::Horizontal)
            .maximum_size(side)
            .tightening_threshold(side)
            .child(&tall)
            .build();
        let reach = reach().round() as i32;
        wide.set_margin_start(reach);
        wide.set_margin_end(reach);
        wide.set_margin_top(reach);
        wide.set_margin_bottom(reach);
        let ring = gtk::DrawingArea::builder()
            .can_target(false)
            .hexpand(true)
            .vexpand(true)
            .build();
        let frame = gtk::Overlay::builder()
            .width_request(side + 2 * reach)
            .height_request(side + 2 * reach)
            .halign(gtk::Align::Start)
            .build();
        frame.set_child(Some(&wide));
        frame.add_overlay(&ring);
        let failure = gtk::Label::builder()
            .label(say.get("Couldn't load the video. Tap to try again."))
            .wrap(true)
            .max_width_chars(28)
            .css_classes(["caption", "dim-label"])
            .visible(false)
            .build();
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .halign(gtk::Align::Start)
            .accessible_role(gtk::AccessibleRole::Group)
            .css_classes(["fc-media-circle"])
            .build();
        root.append(&frame);
        root.append(&failure);
        let circle = Rc::new(Circle {
            clip,
            picture,
            disc,
            spinner,
            capsule,
            dot,
            expand,
            ring,
            failure,
            message: message.clone(),
            attachment: attachment.clone(),
            mine,
            played: Cell::new(played),
            machine: RefCell::new(RoundPlayback::new()),
            player: RefCell::new(None),
            poster: RefCell::new(None),
            claim: Cell::new(None),
            tick: RefCell::new(None),
            pending_click: RefCell::new(None),
            scroll: RefCell::new(None),
        });
        circle.wire(&root);
        circle.fetch_poster();
        (circle, root)
    }

    fn wire(self: &Rc<Self>, root: &gtk::Box) {
        let weak = Rc::downgrade(self);
        {
            let weak = weak.clone();
            self.ring.set_draw_func(move |area, cr, width, height| {
                if let Some(circle) = weak.upgrade() {
                    circle.draw_ring(area, cr, f64::from(width), f64::from(height));
                }
            });
        }
        // A click a beat late: a double click is the heart (P1's), never a play.
        let click = gtk::GestureClick::new();
        click.connect_released({
            let weak = weak.clone();
            move |gesture, presses, _, _| {
                let Some(circle) = weak.upgrade() else {
                    return;
                };
                if presses > 1 {
                    if let Some(pending) = circle.pending_click.borrow_mut().take() {
                        pending.remove();
                    }
                    return;
                }
                let wait = gesture
                    .widget()
                    .map(|widget| widget.settings().gtk_double_click_time())
                    .unwrap_or(400);
                let later = Rc::downgrade(&circle);
                let source = glib::timeout_add_local_once(
                    Duration::from_millis(u64::try_from(wait).unwrap_or(400)),
                    move || {
                        if let Some(circle) = later.upgrade() {
                            circle.pending_click.borrow_mut().take();
                            circle.click();
                        }
                    },
                );
                if let Some(old) = circle.pending_click.borrow_mut().replace(source) {
                    old.remove();
                }
            }
        });
        self.clip.add_controller(click);
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed({
            let weak = weak.clone();
            move |_, key, _, _| match (weak.upgrade(), key) {
                (Some(circle), gtk::gdk::Key::space | gtk::gdk::Key::Return) => {
                    circle.click();
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        self.clip.add_controller(keys);
        self.expand.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(circle) = weak.upgrade() {
                    circle.stop();
                    let open = OPEN_VIEWER.with(|slot| slot.borrow().clone());
                    if let Some(open) = open {
                        open(&circle.message, &circle.attachment);
                    }
                }
            }
        });
        root.connect_map({
            let weak = weak.clone();
            move |root| {
                if let Some(circle) = weak.upgrade() {
                    circle.follow_scroll(root);
                }
            }
        });
        root.connect_unmap(move |_| {
            if let Some(circle) = weak.upgrade() {
                circle.unfollow_scroll();
                circle.stop();
            }
        });
    }

    /// The poster: the attachment's preview, when it says it has one.
    fn fetch_poster(self: &Rc<Self>) {
        if !self.attachment.has_preview {
            return;
        }
        let weak = Rc::downgrade(self);
        let attachment = self.attachment.clone();
        glib::spawn_future_local(async move {
            let Some(connection) = Shell::get().connection() else {
                return;
            };
            let Ok(bytes) = connection.attachments.bytes(&attachment, true).await else {
                return;
            };
            let Ok(texture) = gtk::gdk::Texture::from_bytes(&glib::Bytes::from_owned(bytes)) else {
                return;
            };
            if let Some(circle) = weak.upgrade() {
                *circle.poster.borrow_mut() = Some(texture.clone());
                if !round_inline::shows_frames(
                    circle.machine.borrow().phase(),
                    circle.machine.borrow().has_frames(),
                ) {
                    circle.picture.set_paintable(Some(&texture));
                }
            }
        });
    }

    /// Stops the circle when its row is scrolled wholly out of view (S5.3).
    fn follow_scroll(self: &Rc<Self>, root: &gtk::Box) {
        self.unfollow_scroll();
        let Some(scroller) = root
            .ancestor(gtk::ScrolledWindow::static_type())
            .and_then(|widget| widget.downcast::<gtk::ScrolledWindow>().ok())
        else {
            return;
        };
        let adjustment = scroller.vadjustment();
        let weak = Rc::downgrade(self);
        let scroller_weak = scroller.downgrade();
        let handler = adjustment.connect_value_changed(move |_| {
            let (Some(circle), Some(scroller)) = (weak.upgrade(), scroller_weak.upgrade()) else {
                return;
            };
            if circle.machine.borrow().active() && !circle.in_view(&scroller) {
                circle.stop();
            }
        });
        *self.scroll.borrow_mut() = Some((adjustment, handler));
    }

    fn unfollow_scroll(&self) {
        if let Some((adjustment, handler)) = self.scroll.borrow_mut().take() {
            adjustment.disconnect(handler);
        }
    }

    /// Whether any of the circle is in the scrolled window's view.
    fn in_view(&self, scroller: &gtk::ScrolledWindow) -> bool {
        let Some(bounds) = self.clip.compute_bounds(scroller) else {
            return true;
        };
        // The view's rectangle in the circle's own coordinates.
        round_inline::in_view(
            -f64::from(bounds.x()),
            -f64::from(bounds.y()),
            f64::from(scroller.width()),
            f64::from(scroller.height()),
            f64::from(bounds.width()),
            f64::from(bounds.height()),
        )
    }

    /// The single click.
    pub fn click(self: &Rc<Self>) {
        let recording = super::now_playing().refuses();
        let id = self.attachment.id;
        let step = self.machine.borrow_mut().tap(id, recording);
        if step.tap == RoundTap::Refused {
            let say = &Shell::get().say;
            self.clip.announce(
                say.get("You can play this after recording."),
                gtk::AccessibleAnnouncementPriority::Medium,
            );
        }
        self.act(step);
    }

    fn act(self: &Rc<Self>, step: RoundStep) {
        match step.act {
            RoundAct::None => {}
            RoundAct::Load => self.load(),
            RoundAct::Play => self.play(),
            RoundAct::Pause => {
                if let Some(player) = self.player.borrow().as_ref() {
                    player.pause();
                }
                self.release();
            }
            RoundAct::Stop => self.let_go(),
        }
        self.draw();
    }

    fn load(self: &Rc<Self>) {
        self.let_go_player();
        // One thing at a time: whatever plays makes way now.
        super::now_playing().quiet();
        let load = self.machine.borrow().load();
        let weak = Rc::downgrade(self);
        let attachment = self.attachment.clone();
        glib::spawn_future_local(async move {
            let file = super::voice_bubble::fetch(&attachment).await;
            let Some(circle) = weak.upgrade() else {
                return;
            };
            {
                let machine = circle.machine.borrow();
                if machine.load() != load || machine.phase() != RoundPhase::Loading {
                    return;
                }
            }
            let player = file.and_then(|file| GstRoundPlayers.open_round(&file));
            let Some(player) = player else {
                crate::fc_info!("circle: attachment {} could not be played", attachment.id);
                let act = circle.machine.borrow_mut().failed(load);
                circle.act(RoundStep::new(RoundTap::Started, act));
                return;
            };
            let news = Rc::downgrade(&circle);
            player.news().subscribe(move |heard| {
                if let Some(circle) = news.upgrade() {
                    circle.hear(load, *heard);
                }
            });
            *circle.player.borrow_mut() = Some(player);
            circle.draw();
        });
    }

    fn hear(self: &Rc<Self>, load: u32, news: RoundPlayerNews) {
        match news {
            RoundPlayerNews::Opened { total } => {
                if self.machine.borrow_mut().opened(load, total) {
                    if super::now_playing().refuses() {
                        let act = self.machine.borrow_mut().make_way();
                        self.act(RoundStep::new(RoundTap::Started, act));
                        return;
                    }
                    self.play();
                }
            }
            RoundPlayerNews::State(fc_logic::round_playback::PlayerState::Playing) => {
                self.machine.borrow_mut().playing(load);
                self.start_tick();
            }
            RoundPlayerNews::State(fc_logic::round_playback::PlayerState::Paused) => {
                self.machine.borrow_mut().paused_by_player(load);
            }
            RoundPlayerNews::State(fc_logic::round_playback::PlayerState::Buffering) => {
                self.machine.borrow_mut().waiting(load);
            }
            RoundPlayerNews::FirstFrame => {
                self.machine.borrow_mut().first_frame(load);
                if let Some(paintable) = self.player.borrow().as_ref().and_then(|p| p.paintable()) {
                    self.picture.set_paintable(Some(&paintable));
                }
            }
            RoundPlayerNews::Ended => {
                let (act, through) = self.machine.borrow_mut().ended(load);
                if through {
                    self.mark_played();
                }
                self.act(RoundStep::new(RoundTap::Started, act));
                return;
            }
            RoundPlayerNews::Failed => {
                let act = self.machine.borrow_mut().failed(load);
                self.act(RoundStep::new(RoundTap::Started, act));
                return;
            }
        }
        self.draw();
    }

    fn play(self: &Rc<Self>) {
        let weak: Weak<Circle> = Rc::downgrade(self);
        let claim = super::now_playing().claim(
            Playing::RoundVideo,
            Rc::new(move || {
                if let Some(circle) = weak.upgrade() {
                    circle.claim.set(None);
                    let act = circle.machine.borrow_mut().make_way();
                    circle.act(RoundStep::new(RoundTap::Started, act));
                }
            }),
        );
        self.claim.set(Some(claim));
        if let Some(player) = self.player.borrow().as_ref() {
            player.play();
        }
    }

    fn start_tick(self: &Rc<Self>) {
        if self.tick.borrow().is_some() {
            return;
        }
        let weak = Rc::downgrade(self);
        let source =
            glib::timeout_add_local(Duration::from_millis(round_inline::TICK_MS), move || {
                let Some(circle) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                let (position, total) = match circle.player.borrow().as_ref() {
                    Some(player) => (player.position(), player.duration()),
                    None => {
                        circle.tick.borrow_mut().take();
                        return glib::ControlFlow::Break;
                    }
                };
                let load = circle.machine.borrow().load();
                circle.machine.borrow_mut().progress(load, position, total);
                circle.draw();
                glib::ControlFlow::Continue
            });
        *self.tick.borrow_mut() = Some(source);
    }

    fn stop_tick(&self) {
        if let Some(tick) = self.tick.borrow_mut().take() {
            tick.remove();
        }
    }

    fn release(&self) {
        if let Some(claim) = self.claim.take() {
            super::now_playing().release(claim);
        }
    }

    /// The player let go of, the poster back.
    fn let_go(&self) {
        self.let_go_player();
        self.release();
    }

    fn let_go_player(&self) {
        self.stop_tick();
        // The poster back BEFORE the player (and its paintable) goes.
        let poster = self.poster.borrow().clone();
        self.picture.set_paintable(poster.as_ref());
        self.player.borrow_mut().take();
    }

    /// Its row left the screen, its chat closed, the window let go of it.
    pub fn stop(&self) {
        if let Some(pending) = self.pending_click.borrow_mut().take() {
            pending.remove();
        }
        if self.machine.borrow_mut().stop().is_some() {
            self.let_go();
            self.draw();
        }
    }

    fn mark_played(&self) {
        if self.mine || self.played.replace(true) {
            return;
        }
        if let Some(connection) = Shell::get().connection()
            && let Err(error) =
                PlayedRoundStore::new(&connection.cache).mark_played(self.attachment.id)
        {
            crate::fc_warn!(
                "circle: the played mark was refused ({})",
                crate::services::log::store_error(&error)
            );
        }
        self.dot.set_visible(false);
        let say = &Shell::get().say;
        if let Some(value) = round_look::value(self.mine, true, say.as_ref()) {
            self.clip
                .update_property(&[gtk::accessible::Property::Description(&value)]);
        }
    }

    fn draw(&self) {
        let machine = self.machine.borrow();
        let phase = machine.phase();
        let say = &Shell::get().say;
        self.disc.set_visible(round_inline::shows_disc(phase));
        self.spinner.set_visible(round_inline::shows_spinner(phase));
        self.failure.set_visible(round_inline::shows_failure(phase));
        let open = OPEN_VIEWER.with(|slot| slot.borrow().is_some());
        self.expand
            .set_visible(open && round_inline::shows_expand(phase, machine.has_frames()));
        self.capsule.set_label(
            &round_inline::capsule(
                &self.attachment,
                phase,
                machine.seen_playing(),
                machine.position(),
            )
            .unwrap_or_default(),
        );
        let action = round_inline::action(phase, say.as_ref());
        self.clip.set_tooltip_text(Some(&action));
        drop(machine);
        self.ring.queue_draw();
    }

    fn draw_ring(
        &self,
        _area: &gtk::DrawingArea,
        cr: &gtk::cairo::Context,
        width: f64,
        height: f64,
    ) {
        let machine = self.machine.borrow();
        let reduced = !gtk::Settings::default().is_some_and(|s| s.is_gtk_enable_animations());
        let fraction = round_inline::ring(
            machine.phase(),
            machine.seen_playing(),
            machine.position(),
            machine.total(),
            reduced,
        );
        if fraction <= 0.0 {
            return;
        }
        let accent = crate::theme::palette().accent;
        cr.set_source_rgb(
            f64::from(accent.0) / 255.0,
            f64::from(accent.1) / 255.0,
            f64::from(accent.2) / 255.0,
        );
        cr.set_line_width(round_look::RING);
        cr.set_line_cap(gtk::cairo::LineCap::Round);
        let radius = diameter() / 2.0 + round_inline::RING_GAP + round_look::RING / 2.0;
        let start = -std::f64::consts::FRAC_PI_2;
        cr.arc(
            width / 2.0,
            height / 2.0,
            radius,
            start,
            start + fraction * 2.0 * std::f64::consts::PI,
        );
        let _ = cr.stroke();
    }

    /// Where the machine is (the self-test).
    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "the media self-test reads it")
    )]
    pub fn phase(&self) -> RoundPhase {
        self.machine.borrow().phase()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_expand_control_stands_wholly_inside_the_circle() {
        let (side, target) = (round_look::DIAMETER, round_inline::EXPAND_TARGET);
        let inset = f64::from(expand_inset(side, target));
        let half = target / 2.0;
        // Its centre, from the clip's top-left corner, then from the circle's middle.
        let (x, y) = (side - inset - half, inset + half);
        let from_middle = ((x - side / 2.0).powi(2) + (y - side / 2.0).powi(2)).sqrt();
        assert!(
            from_middle + half <= side / 2.0 - 7.0,
            "{inset} {from_middle}"
        );
        assert_eq!(expand_inset(240.0, 44.0), 34);
    }
}
