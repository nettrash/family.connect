//! A voice note in a bubble (shell-design §10.3; `VoiceLook`, the approved design of 2026-10-05):
//! a round accent play button, the sender's waveform (or the neutral placeholder) with the played
//! bars lit — ⌊position · bars / duration⌋ — the time in tabular digits, the speed chip while it is
//! the one playing (1×, 1.5×, 2×, kept in `settings.conf`'s `voice-speed`), and the dot until this
//! device has played someone else's (`PlayedVoiceStore`). **The waveform is the seek**: an
//! accessible slider, clicked or dragged, Left/Right a step.
//!
//! The contract of `ConversationSlots::voice`: its final size at once (from metadata); the bytes
//! fetched through the connection's `AttachmentCache` on the click, never before — and replayable
//! offline from there; one thing plays at a time (`NowPlaying`); nothing plays while a recording
//! runs (S1.7); stopped when unmapped; nothing logged but ids.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use fc_core::protocol::{AttachmentDto, MessageDto};
use fc_core::store::PlayedVoiceStore;
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::attachment_cache::AttachmentCache;
use fc_logic::round_look::Playing;
use fc_logic::voice_look::{self, voice_speed};
use fc_logic::voice_notes;
use gtk::prelude::*;

use super::waves::{Ink, Waves};
use crate::seams::media::{MediaSource, NowPlaying, Player, PlayerState};
use crate::services::settings_store::keys;
use crate::shell::Shell;

/// How far Left/Right moves the waveform's seek, seconds.
const STEP_SECONDS: f64 = 5.0;

/// One voice note's bubble body.
pub struct VoiceBubble {
    /// Held weakly: the widget owns the bubble (its `destroy` handler holds it), never the other
    /// way round, so a bubble scrolled out of a bounded timeline is freed with its row.
    root: glib::WeakRef<gtk::Box>,
    button: gtk::Button,
    waves: Waves,
    time: gtk::Label,
    speed: gtk::Button,
    dot: gtk::Box,
    attachment: AttachmentDto,
    mine: bool,
    total: f64,
    bars: usize,
    player: RefCell<Option<Rc<dyn Player>>>,
    claim: Cell<Option<u64>>,
    /// Where the waveform was seeked to while it was not playing.
    seeked: Cell<Option<f64>>,
    /// Playing, or paused part way: the speed chip shows, the time counts.
    active: Cell<bool>,
    loading: Cell<bool>,
    played: Cell<bool>,
}

impl std::fmt::Debug for VoiceBubble {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VoiceBubble")
            .field("attachment", &self.attachment.id)
            .finish_non_exhaustive()
    }
}

/// `ConversationSlots::voice`: the widget, its final size already.
pub fn widget(message: &MessageDto, attachment: &AttachmentDto, own: bool) -> gtk::Widget {
    let (bubble, root) = VoiceBubble::new(message, attachment, own);
    let root: gtk::Widget = root.upcast();
    // The widget keeps its bubble alive; the bubble's handlers hold it weakly.
    let keep = RefCell::new(Some(bubble));
    root.connect_destroy(move |_| {
        if let Some(bubble) = keep.borrow_mut().take() {
            bubble.stop();
        }
    });
    root
}

impl VoiceBubble {
    pub fn new(
        message: &MessageDto,
        attachment: &AttachmentDto,
        own: bool,
    ) -> (Rc<VoiceBubble>, gtk::Box) {
        let shell = Shell::get();
        let say = &shell.say;
        let total = voice_notes::total_seconds(attachment.duration_ms);
        let bars = voice_look::bar_count(voice_look::WAVE_WIDTH);
        let played = !own
            && shell.connection().is_some_and(|connection| {
                PlayedVoiceStore::new(&connection.cache)
                    .played(attachment.id)
                    .unwrap_or(false)
            });

        let button = gtk::Button::builder()
            .icon_name("media-playback-start-symbolic")
            .width_request(voice_look::PLAY_DISC as i32)
            .height_request(voice_look::PLAY_DISC as i32)
            .valign(gtk::Align::Center)
            .css_classes(["circular", "fc-media-play"])
            .tooltip_text(say.get("Play"))
            .build();
        if own {
            button.add_css_class("fc-media-on-accent");
        }
        button.update_property(&[gtk::accessible::Property::Label(say.get("Play"))]);

        let waves = Waves::new(
            voice_look::WAVE_WIDTH,
            voice_look::WAVE_HEIGHT,
            if own { Ink::OnAccent } else { Ink::Accent },
        );
        waves.set_levels(voice_look::bars(attachment.waveform.as_deref(), bars));
        let wave_box = gtk::Box::builder()
            .accessible_role(gtk::AccessibleRole::Slider)
            .focusable(true)
            .build();
        wave_box.append(waves.widget());
        wave_box.update_property(&[
            gtk::accessible::Property::Label(&voice_look::name(attachment, say.as_ref())),
            gtk::accessible::Property::ValueMin(0.0),
            gtk::accessible::Property::ValueMax(total),
            gtk::accessible::Property::ValueNow(0.0),
        ]);

        let time = gtk::Label::builder()
            .label(voice_look::time(false, 0.0, total))
            .xalign(0.0)
            .css_classes(["caption", "numeric", "fc-media-time"])
            .build();
        let rate = current_rate(&shell);
        let speed = gtk::Button::builder()
            .label(voice_speed::label(rate, say.as_ref()))
            .visible(false)
            .valign(gtk::Align::Center)
            .css_classes(["flat", "pill", "caption", "fc-media-speed"])
            .tooltip_text(voice_speed::name(rate, say.as_ref()))
            .build();
        speed.update_property(&[gtk::accessible::Property::Label(&voice_speed::name(
            rate,
            say.as_ref(),
        ))]);
        let dot = gtk::Box::builder()
            .width_request(voice_look::DOT as i32)
            .height_request(voice_look::DOT as i32)
            .valign(gtk::Align::Center)
            .css_classes(["fc-media-dot"])
            .visible(voice_look::shows_dot(own, played))
            .build();
        let foot = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        foot.append(&time);
        foot.append(&dot);
        foot.append(&speed);
        let column = gtk::Box::new(gtk::Orientation::Vertical, 2);
        column.set_valign(gtk::Align::Center);
        column.append(&wave_box);
        column.append(&foot);
        let root = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(10)
            .width_request(voice_look::WIDTH as i32)
            .margin_start(8)
            .margin_end(12)
            .margin_top(6)
            .margin_bottom(6)
            .accessible_role(gtk::AccessibleRole::Group)
            .css_classes(["fc-media-voice"])
            .build();
        root.append(&button);
        root.append(&column);
        root.update_property(&[gtk::accessible::Property::Label(&voice_look::name(
            attachment,
            say.as_ref(),
        ))]);
        if let Some(status) = voice_look::status(own, played, say.as_ref()) {
            root.update_property(&[gtk::accessible::Property::Description(&status)]);
        }

        let bubble = Rc::new(VoiceBubble {
            root: root.downgrade(),
            button,
            waves,
            time,
            speed,
            dot,
            attachment: attachment.clone(),
            mine: own,
            total,
            bars,
            player: RefCell::new(None),
            claim: Cell::new(None),
            seeked: Cell::new(None),
            active: Cell::new(false),
            loading: Cell::new(false),
            played: Cell::new(played),
        });
        let _ = message;
        bubble.wire(&root, &wave_box);
        (bubble, root)
    }

    fn wire(self: &Rc<Self>, root: &gtk::Box, wave_box: &gtk::Box) {
        let weak = Rc::downgrade(self);
        self.button.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(bubble) = weak.upgrade() {
                    bubble.toggle();
                }
            }
        });
        self.speed.connect_clicked({
            let weak = weak.clone();
            move |_| {
                if let Some(bubble) = weak.upgrade() {
                    bubble.next_speed();
                }
            }
        });
        // The waveform is the seek.
        let click = gtk::GestureClick::new();
        click.connect_pressed({
            let weak = weak.clone();
            move |gesture, _, x, _| {
                if let Some(bubble) = weak.upgrade() {
                    let width = f64::from(gesture.widget().map_or(1, |w| w.width()));
                    bubble.seek_fraction(x / width.max(1.0));
                }
            }
        });
        self.waves.widget().add_controller(click);
        let drag = gtk::GestureDrag::new();
        drag.connect_drag_update({
            let weak = weak.clone();
            move |gesture, offset_x, _| {
                if let (Some(bubble), Some((start_x, _))) = (weak.upgrade(), gesture.start_point())
                {
                    let width = f64::from(gesture.widget().map_or(1, |w| w.width()));
                    bubble.seek_fraction((start_x + offset_x) / width.max(1.0));
                }
            }
        });
        self.waves.widget().add_controller(drag);
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed({
            let weak = weak.clone();
            move |_, key, _, _| {
                let Some(bubble) = weak.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                let step = match key {
                    gtk::gdk::Key::Left => -STEP_SECONDS,
                    gtk::gdk::Key::Right => STEP_SECONDS,
                    _ => return glib::Propagation::Proceed,
                };
                let at = (bubble.position() + step).clamp(0.0, bubble.total);
                bubble.seek_fraction(at / bubble.total.max(0.1));
                glib::Propagation::Stop
            }
        });
        wave_box.add_controller(keys);
        // Scrolled away, the chat left, the window hidden: stopped (§10.4's rule, the contract's).
        root.connect_unmap(move |_| {
            if let Some(bubble) = weak.upgrade() {
                bubble.stop();
            }
        });
    }

    /// Where the note is now, seconds.
    fn position(&self) -> f64 {
        match self.player.borrow().as_ref() {
            Some(player) if self.active.get() => player.position(),
            _ => self.seeked.get().unwrap_or(0.0),
        }
    }

    /// The play button.
    fn toggle(self: &Rc<Self>) {
        let playing = self
            .player
            .borrow()
            .as_ref()
            .is_some_and(|player| player.state() == PlayerState::Playing);
        if playing || self.loading.get() {
            self.pause();
        } else {
            self.start();
        }
    }

    fn start(self: &Rc<Self>) {
        let registry = super::now_playing();
        if registry.refuses() {
            let say = &Shell::get().say;
            if let Some(root) = self.root.upgrade() {
                root.announce(
                    say.get("You can play this after recording."),
                    gtk::AccessibleAnnouncementPriority::Medium,
                );
            }
            return;
        }
        let weak = Rc::downgrade(self);
        let claim = registry.claim(
            Playing::VoiceNote,
            Rc::new({
                let weak = weak.clone();
                move || {
                    if let Some(bubble) = weak.upgrade() {
                        bubble.paused_by_another();
                    }
                }
            }),
        );
        self.claim.set(Some(claim));
        let existing = self.player.borrow().clone();
        if let Some(player) = existing {
            player.set_rate(current_rate(&Shell::get()));
            if !self.active.get() {
                let at = voice_look::start_at(self.seeked.take(), self.total);
                player.seek(at);
            }
            player.play();
            return;
        }
        self.loading.set(true);
        self.draw_button();
        glib::spawn_future_local(async move {
            let Some(bubble) = weak.upgrade() else {
                return;
            };
            let path = fetch(&bubble.attachment).await;
            if !bubble.loading.get() {
                return;
            }
            let Some(path) = path else {
                crate::fc_info!(
                    "voice: attachment {} could not be fetched",
                    bubble.attachment.id
                );
                bubble.loading.set(false);
                bubble.release();
                bubble.draw_button();
                return;
            };
            let shell = Shell::get();
            let player = shell.players.get().create(false);
            bubble.listen(&player);
            player.load(MediaSource::File(path));
            player.set_rate(current_rate(&shell));
            let at = voice_look::start_at(bubble.seeked.take(), bubble.total);
            if at > 0.0 {
                player.seek(at);
            }
            player.play();
            *bubble.player.borrow_mut() = Some(player);
        });
    }

    fn listen(self: &Rc<Self>, player: &Rc<dyn Player>) {
        let weak: Weak<VoiceBubble> = Rc::downgrade(self);
        player.on_state().subscribe({
            let weak = weak.clone();
            move |state| {
                if let Some(bubble) = weak.upgrade() {
                    bubble.state_changed(*state);
                }
            }
        });
        player.on_position().subscribe(move |position| {
            if let Some(bubble) = weak.upgrade() {
                bubble.draw_position(*position);
            }
        });
    }

    fn state_changed(&self, state: PlayerState) {
        match state {
            PlayerState::Playing => {
                self.loading.set(false);
                self.active.set(true);
                self.mark_played();
            }
            PlayerState::Ended => {
                self.active.set(false);
                self.loading.set(false);
                self.release();
                self.draw_position(0.0);
            }
            PlayerState::Failed => {
                self.active.set(false);
                self.loading.set(false);
                self.release();
            }
            _ => {}
        }
        self.draw_button();
    }

    /// Something else began to play: this one pauses where it is.
    fn paused_by_another(&self) {
        self.claim.set(None);
        if let Some(player) = self.player.borrow().as_ref() {
            player.pause();
        }
        if self.loading.get() {
            self.loading.set(false);
        }
        self.draw_button();
    }

    fn pause(&self) {
        if self.loading.get() {
            self.loading.set(false);
        }
        if let Some(player) = self.player.borrow().as_ref() {
            player.pause();
        }
        self.release();
        self.draw_button();
    }

    /// Unmapped or destroyed: let go of the player.
    fn stop(&self) {
        self.loading.set(false);
        self.release();
        let player = self.player.borrow_mut().take();
        if let Some(player) = player {
            player.stop();
        }
        self.active.set(false);
        self.seeked.set(None);
        self.draw_position(0.0);
        self.draw_button();
    }

    fn release(&self) {
        if let Some(claim) = self.claim.take() {
            super::now_playing().release(claim);
        }
    }

    fn seek_fraction(&self, fraction: f64) {
        let at = (fraction.clamp(0.0, 1.0) * self.total).max(0.0);
        match self.player.borrow().as_ref() {
            Some(player) if self.active.get() => player.seek(at),
            _ => self.seeked.set(Some(at)),
        }
        self.draw_position(at);
    }

    fn next_speed(&self) {
        let shell = Shell::get();
        let rate = voice_speed::next(current_rate(&shell));
        shell
            .settings
            .set_string(keys::VOICE_SPEED, &voice_speed::store(rate));
        if let Some(player) = self.player.borrow().as_ref() {
            player.set_rate(rate);
        }
        let say = &shell.say;
        self.speed
            .set_label(&voice_speed::label(rate, say.as_ref()));
        let name = voice_speed::name(rate, say.as_ref());
        self.speed.set_tooltip_text(Some(&name));
        self.speed
            .update_property(&[gtk::accessible::Property::Label(&name)]);
    }

    fn mark_played(&self) {
        if self.mine || self.played.replace(true) {
            return;
        }
        if let Some(connection) = Shell::get().connection()
            && let Err(error) =
                PlayedVoiceStore::new(&connection.cache).mark_played(self.attachment.id)
        {
            crate::fc_warn!(
                "voice: the played mark was refused ({})",
                crate::services::log::store_error(&error)
            );
        }
        self.dot.set_visible(false);
        let say = &Shell::get().say;
        if let (Some(status), Some(root)) = (
            voice_look::status(self.mine, true, say.as_ref()),
            self.root.upgrade(),
        ) {
            root.update_property(&[gtk::accessible::Property::Description(&status)]);
        }
    }

    fn draw_position(&self, position: f64) {
        let active = self.active.get() || self.seeked.get().is_some();
        self.waves
            .set_lit(voice_look::played_bars(position, self.total, self.bars));
        self.time
            .set_label(&voice_look::time(active, position, self.total));
    }

    fn draw_button(&self) {
        let say = &Shell::get().say;
        let playing = self.loading.get()
            || self
                .player
                .borrow()
                .as_ref()
                .is_some_and(|player| player.state() == PlayerState::Playing);
        let (icon, word) = if playing {
            ("media-playback-pause-symbolic", say.get("Pause"))
        } else {
            ("media-playback-start-symbolic", say.get("Play"))
        };
        self.button.set_icon_name(icon);
        self.button.set_tooltip_text(Some(word));
        self.button
            .update_property(&[gtk::accessible::Property::Label(word)]);
        self.speed
            .set_visible(voice_look::shows_speed(self.active.get()));
    }
}

#[cfg_attr(
    not(feature = "selftest"),
    allow(dead_code, reason = "the media self-test presses and reads the bubble")
)]
impl VoiceBubble {
    /// The play button, pressed (the self-test).
    pub fn press_play(self: &Rc<Self>) {
        self.toggle();
    }

    /// The player's state, once it has one (the self-test).
    pub fn player_state(&self) -> Option<PlayerState> {
        self.player.borrow().as_ref().map(|player| player.state())
    }
}

/// The voice speed this device keeps (1× when absent or unreadable).
pub fn current_rate(shell: &Shell) -> f64 {
    voice_speed::parse(shell.settings.string(keys::VOICE_SPEED).as_deref())
}

/// The attachment's bytes through the connection's cache, as the file the cache keeps them in (or
/// a temporary when the cache could not write them). `None` when they cannot be had.
pub async fn fetch(attachment: &AttachmentDto) -> Option<std::path::PathBuf> {
    let shell = Shell::get();
    let connection = shell.connection()?;
    let bytes = connection.attachments.bytes(attachment, false).await.ok()?;
    let kept = shell
        .paths
        .blobs_dir()
        .join(AttachmentCache::key_for(attachment.id, false));
    if kept.is_file() {
        return Some(kept);
    }
    let temp = super::gst_util::temp_file(&shell.paths.tmp_dir(), "play", "bin").ok()?;
    std::fs::write(&temp, &bytes).ok()?;
    Some(temp)
}
