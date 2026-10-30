//! Recording inside P1's composer (shell-design §10.2, §6.1; W.3 `ComposerSlots` — its six-step
//! protocol is the contract): the voice recording's whole life, the "Voice message not sent" rows,
//! and parking.
//!
//! The rules are fc-logic's and are called, never re-implemented: `voice_notes` (the floor, the
//! five-minute limit, "30 seconds left", the refusals, `start_recording` with its lead-in and its
//! remembered interruption, `KeepAwake`), `not_sent` (where an ended recording goes and what is
//! said), `parked_recordings` (the not-sent store, per account, written whole or held in memory),
//! `voice_look` (the live line, the chip's fit, the speed), `voice_waveform::shaped` (the sender's
//! waveform, measured from the file).
//!
//! **Interruptions STOP AND KEEP** (decision 12): leaving the chat, a call, the window hidden or
//! minimised, the session locking, sleep, the recorder failing — kept as "not sent"; only the
//! person's Send sends and only their Delete deletes; a sign-out takes everything with it.
//! **Quitting parks SYNCHRONOUSLY** ([`MediaComposerSlots::park_now`], from `Shell::on_quitting`):
//! the muxer's EOS is awaited on the bus itself and the file written to the parked store before
//! the handler returns — nothing awaited.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::Duration;

use adw::prelude::*;
use fc_core::protocol::ServerUrl;
use fc_core::rt::Clock;
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::composer_button::Recording;
use fc_logic::media_outbox::StagedMedia;
use fc_logic::not_sent::{self, RecordingEnd, RecordingFate};
use fc_logic::parked_recordings::{ParkedRecording, ParkedRecordings};
use fc_logic::round_look::Playing;
use fc_logic::round_video_rules;
use fc_logic::voice_look::{self, LiveLevels, voice_waveform};
use fc_logic::voice_notes::{self, RecordingStart, Started, VoiceRecorder, VoiceRecording};

use super::voice_recorder::{GstVoiceDecoder, GstVoiceRecorder, GstVoiceRecording};
use super::waves::{Ink, Waves};
use crate::composer::recording_row::{RecordingRow, RowPress};
use crate::desktop::keep_awake::{self, Awake, AwakeHold};
use crate::seams::composer::{ComposerHost, ComposerRecording, ComposerSlots};
use crate::seams::media::{MediaSource, NowPlaying, Player, PlayerState};
use crate::shell::Shell;

/// The row's clock and meter tick: an eighth of a second.
const TICK: Duration = Duration::from_millis(125);
/// How long quitting waits for the muxer to finish a recording.
const PARK_LIMIT: Duration = Duration::from_secs(3);

/// P3's `ComposerSlots` (installed in `Shell::composer_slots`): one [`ComposerVoice`] per
/// composer, and the account's not-sent store.
pub struct MediaComposerSlots {
    composers: RefCell<Vec<Weak<ComposerVoice>>>,
    recorder: Rc<GstVoiceRecorder>,
    decoder: Rc<GstVoiceDecoder>,
    /// The not-sent store of the account signed in, by owner (server and account): one object for
    /// the run, so what the disk refused stays held in memory.
    stores: RefCell<HashMap<(String, i64), Rc<ParkedRecordings>>>,
    /// Waveforms of not-sent notes measured this run, by entry id (`None`: none could be).
    shapes: RefCell<HashMap<String, Option<String>>>,
}

impl std::fmt::Debug for MediaComposerSlots {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaComposerSlots")
            .field("composers", &self.composers.borrow().len())
            .finish_non_exhaustive()
    }
}

impl MediaComposerSlots {
    pub fn new(shell: &Rc<Shell>) -> Rc<MediaComposerSlots> {
        let tmp = shell.paths.tmp_dir();
        Rc::new(MediaComposerSlots {
            composers: RefCell::new(Vec::new()),
            recorder: Rc::new(GstVoiceRecorder::new(tmp.clone())),
            decoder: Rc::new(GstVoiceDecoder::new(tmp)),
            stores: RefCell::new(HashMap::new()),
            shapes: RefCell::new(HashMap::new()),
        })
    }

    /// Every composer still alive.
    pub fn composers(&self) -> Vec<Rc<ComposerVoice>> {
        let mut composers = self.composers.borrow_mut();
        composers.retain(|weak| weak.strong_count() > 0);
        composers.iter().filter_map(Weak::upgrade).collect()
    }

    /// Something other than the person ends every recording (§10.8 — the session watch: a lock,
    /// sleep, a minimised or hidden window, a call; a sign-out).
    pub fn interrupt_all(&self, why: RecordingEnd) {
        for composer in self.composers() {
            composer.interrupt(why);
        }
    }

    /// Whether any composer records now (or is starting to).
    pub fn recording(&self) -> bool {
        self.composers()
            .iter()
            .any(|composer| composer.recording() != Recording::None || composer.start.starting())
    }

    /// QUITTING (§3.8, `Shell::on_quitting`): every running recording stopped and parked NOW,
    /// synchronously, then what the disk refused is offered to it once more.
    pub fn park_now(&self) {
        for composer in self.composers() {
            composer.park_now();
        }
        for store in self.stores.borrow().values() {
            if store.holds_in_memory() {
                let left = store.write_held();
                if left > 0 {
                    crate::fc_warn!(
                        "not sent: {left} recording(s) could not be written before quitting"
                    );
                }
            }
        }
    }

    /// The not-sent store of the account signed in now, or `None` with nobody signed in.
    pub fn store(&self) -> Option<Rc<ParkedRecordings>> {
        let shell = Shell::get();
        let connection = shell.connection()?;
        let account = connection.session.state().me.map(|me| me.id)?;
        let server = owner_server(&connection.server);
        let key = (server.clone(), account);
        if let Some(store) = self.stores.borrow().get(&key) {
            return Some(Rc::clone(store));
        }
        let store = Rc::new(ParkedRecordings::for_account(
            &shell.paths.parked_dir(),
            &server,
            account,
        ));
        // At launch (the first look this run): parts a crash left, halves of entries.
        store.sweep();
        self.stores.borrow_mut().insert(key, Rc::clone(&store));
        Some(store)
    }

    /// A session ended: everything recorded and not sent goes with it (the shell wipes the disk;
    /// what memory held goes here).
    pub fn forget_session(&self) {
        for store in self.stores.borrow().values() {
            store.forget_held();
        }
        self.stores.borrow_mut().clear();
        self.shapes.borrow_mut().clear();
        for composer in self.composers() {
            composer.refresh_rows();
        }
    }
}

impl ComposerSlots for MediaComposerSlots {
    fn attach(&self, host: Rc<dyn ComposerHost>) -> Rc<dyn ComposerRecording> {
        let shell = Shell::get();
        let composer = ComposerVoice::new(host, &shell);
        self.composers.borrow_mut().push(Rc::downgrade(&composer));
        composer.refresh_rows();
        composer
    }
}

/// The server as the owner key spells it, the same every time (carry-forward, "Parked
/// recordings"): `ServerUrl`'s own text, which always ends a bare origin with `/`.
fn owner_server(server: &ServerUrl) -> String {
    server.to_string()
}

/// The installed slots, concretely (the session watch and quitting reach every composer).
fn slots() -> Option<Rc<MediaComposerSlots>> {
    SLOTS.with(|slot| slot.borrow().as_ref().and_then(Weak::upgrade))
}

thread_local! {
    static SLOTS: RefCell<Option<Weak<MediaComposerSlots>>> = const { RefCell::new(None) };
}

/// The installed composer slots, for the session watch and the self-test.
pub fn installed() -> Option<Rc<MediaComposerSlots>> {
    slots()
}

/// Parking on quit and forgetting at a session's end (`media::install`).
pub fn follow_the_session(shell: &Rc<Shell>, slots: &Rc<MediaComposerSlots>) {
    SLOTS.with(|slot| *slot.borrow_mut() = Some(Rc::downgrade(slots)));
    {
        let weak = Rc::downgrade(slots);
        shell.on_quitting().subscribe(move |_| {
            if let Some(slots) = weak.upgrade() {
                slots.park_now();
            }
        });
    }
    let weak = Rc::downgrade(slots);
    let watch = move || {
        let Some(connection) = Shell::get().connection() else {
            return;
        };
        let weak = weak.clone();
        connection.session.on_ended().subscribe(move |_| {
            if let Some(slots) = weak.upgrade() {
                // A recording belongs to whoever made it: discarded, never parked (`not_sent`).
                slots.interrupt_all(RecordingEnd::SignedOut);
                slots.forget_session();
            }
        });
    };
    watch();
    shell.on_connection_changed().subscribe(move |_| watch());
}

/// A recording running in one composer.
struct Running {
    recording: Rc<dyn VoiceRecording>,
    /// The same, concretely: what quitting stops synchronously.
    concrete: Option<Rc<GstVoiceRecording>>,
    chat_id: i64,
    beside_draft: bool,
    levels: LiveLevels,
    warned: bool,
    /// A peak rose above the silence line (`fc_text::record::SILENCE_PEAK_DBFS`) — S2.9.
    heard_sound: bool,
    /// "We can't hear anything…" was said aloud once.
    said_silence: bool,
    timer: Option<glib::SourceId>,
    /// The screen kept on while it records (§10.9, P5's hold; released when it stops).
    awake: Option<AwakeHold>,
}

/// The recording half of ONE composer (W.3 `ComposerRecording`).
pub struct ComposerVoice {
    host: Rc<dyn ComposerHost>,
    row: Rc<RecordingRow>,
    above: gtk::Box,
    rows: gtk::Box,
    start: RecordingStart,
    running: RefCell<Option<Running>>,
    /// Stops still landing (a Send, a Stop, a Delete being written down).
    settling: Cell<u32>,
    /// A not-sent note playing from its row: (entry id, player, claim).
    playing: RefCell<Option<(String, Rc<dyn Player>, u64)>>,
}

impl std::fmt::Debug for ComposerVoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ComposerVoice")
            .field("chat", &self.host.chat_id())
            .field("recording", &self.recording())
            .finish_non_exhaustive()
    }
}

impl ComposerVoice {
    fn new(host: Rc<dyn ComposerHost>, shell: &Rc<Shell>) -> Rc<ComposerVoice> {
        let say = shell.say.as_ref();
        let row = RecordingRow::new(say);
        let rows = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let above = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(4)
            .css_classes(["fc-media-not-sent"])
            .build();
        above.append(&rows);
        let composer = Rc::new(ComposerVoice {
            host,
            row,
            above,
            rows,
            start: RecordingStart::new(),
            running: RefCell::new(None),
            settling: Cell::new(0),
            playing: RefCell::new(None),
        });
        let weak = Rc::downgrade(&composer);
        composer.row.pressed().subscribe(move |press| {
            if let Some(composer) = weak.upgrade() {
                match press {
                    RowPress::Stop => composer.stop(),
                    RowPress::Delete => composer.delete(),
                }
            }
        });
        composer
    }

    /// The chat the composer writes to now.
    pub fn chat_id(&self) -> i64 {
        self.host.chat_id()
    }

    /// The row's live line has drawn bars (the self-test's meter check).
    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "the media self-test reads it")
    )]
    pub fn meter_heard(&self) -> usize {
        self.running
            .borrow()
            .as_ref()
            .map_or(0, |running| running.levels.heard())
    }

    /// Whether the recording that runs holds the screen on (P5's `keep_awake`; the self-test's
    /// question — in a container nothing answers the inhibit, so the hold is asked, not granted).
    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "the media self-test reads it")
    )]
    pub fn keeps_awake(&self) -> bool {
        self.running
            .borrow()
            .as_ref()
            .is_some_and(|running| running.awake.is_some())
    }

    fn say(&self) -> Rc<crate::services::catalogue::Catalogue> {
        Shell::get().say.clone()
    }

    /// Something the composer says: on P1's notice line (`ComposerHost::notice`), which tells a
    /// screen reader too unless a recording runs (`voice_notes::notice_said`).
    fn tell(self: &Rc<Self>, sentence: &str) {
        self.host.notice(sentence);
        self.host.changed();
    }

    /// Said to a screen reader and never drawn (S6).
    fn say_aloud(&self, sentence: &str) {
        self.row
            .widget()
            .announce(sentence, gtk::AccessibleAnnouncementPriority::Medium);
    }

    fn call_busy() -> bool {
        Shell::get()
            .connection()
            .is_some_and(|connection| connection.calls.busy())
    }

    /// The window is away: hidden, minimised, or the screen saver runs.
    fn window_away(&self) -> bool {
        let window = self.host.window();
        let hidden = window
            .as_ref()
            .is_some_and(|window| !window.is_visible() || window.is_suspended());
        hidden || Shell::get().app.is_screensaver_active()
    }

    // ---- starting -----------------------------------------------------------------------------

    fn begin(self: &Rc<Self>, beside_draft: bool) {
        if self.running.borrow().is_some() {
            self.stop();
            return;
        }
        if self.start.starting() || self.settling.get() > 0 {
            return;
        }
        let shell = Shell::get();
        let say = self.say();
        let chat_id = self.host.chat_id();
        let not_sent = self.not_sent();
        let full = self.host.staged() >= fc_text::media::MAX_PER_MESSAGE;
        if let Some(refused) = voice_notes::refusal(
            false,
            Self::call_busy(),
            false,
            not_sent,
            full,
            say.as_ref(),
        ) {
            self.tell(&refused);
            return;
        }
        // Whatever plays goes quiet first, and nothing starts while the microphone opens (S1.7).
        let registry = super::now_playing();
        registry.set_recording(true);
        self.stop_row_player();
        let weak = Rc::downgrade(self);
        let recorder = Rc::clone(&slots_recorder());
        let screen_reader = super::session_watch::screen_reader();
        let clock: Rc<dyn Clock> = shell.clock.clone();
        glib::spawn_future_local(async move {
            let Some(composer) = weak.upgrade() else {
                registry.set_recording(false);
                return;
            };
            let speaker = Rc::downgrade(&composer);
            let say_recording: Rc<dyn Fn()> = Rc::new(move || {
                if let Some(composer) = speaker.upgrade() {
                    composer.say_aloud(Shell::get().say.get("Recording"));
                }
            });
            let wanted = Rc::downgrade(&composer);
            let started = voice_notes::start_recording(
                recorder.as_ref(),
                &composer.start,
                screen_reader,
                clock,
                say_recording,
                move || {
                    wanted.upgrade().is_some_and(|composer| {
                        composer.host.chat_id() == chat_id
                            && composer.running.borrow().is_none()
                            && !Self::call_busy()
                            && !composer.window_away()
                    })
                },
            )
            .await;
            match started {
                Started::Recording { recording, spoken } => {
                    let concrete = recorder.take_last().filter(|last| same(last, &recording));
                    composer.recording_began(recording, concrete, chat_id, beside_draft, spoken);
                }
                Started::Failed(_) => {
                    registry.set_recording(false);
                    let say = composer.say();
                    let (sentence, _) =
                        voice_notes::microphone_refusal(recorder.microphone_access(), say.as_ref());
                    composer.tell(
                        sentence
                            .as_deref()
                            .unwrap_or(say.get("Couldn't start recording.")),
                    );
                }
                Started::LetGo => {
                    registry.set_recording(false);
                    if Self::call_busy() {
                        let say = composer.say();
                        composer.tell(say.get("You can record a message after the call."));
                    }
                }
            }
            composer.host.changed();
        });
        self.host.changed();
    }

    fn recording_began(
        self: &Rc<Self>,
        recording: Rc<dyn VoiceRecording>,
        concrete: Option<Rc<GstVoiceRecording>>,
        chat_id: i64,
        beside_draft: bool,
        spoken: bool,
    ) {
        let say = self.say();
        let still = !gtk::Settings::default().is_some_and(|s| s.is_gtk_enable_animations());
        self.row.begin(beside_draft, still, say.as_ref());
        let levels = LiveLevels::new(self.row.capacity().max(1));
        {
            let weak = Rc::downgrade(self);
            recording.failed().subscribe(move |_| {
                if let Some(composer) = weak.upgrade() {
                    composer.end(RecordingEnd::RecorderFailed);
                }
            });
        }
        let weak = Rc::downgrade(self);
        let timer = glib::timeout_add_local(TICK, move || match weak.upgrade() {
            Some(composer) => composer.tick(),
            None => glib::ControlFlow::Break,
        });
        *self.running.borrow_mut() = Some(Running {
            recording,
            concrete,
            chat_id,
            beside_draft,
            levels,
            warned: false,
            heard_sound: false,
            said_silence: false,
            timer: Some(timer),
            awake: Some(keep_awake::hold(
                &Shell::get().app,
                self.host.window().as_ref(),
                Awake::VoiceMessage,
                say.as_ref(),
            )),
        });
        if !spoken {
            self.say_aloud(say.get("Recording"));
        }
        crate::fc_info!("voice: recording in chat {chat_id}");
        self.host.changed();
    }

    /// The clock, the meter, the five-minute limit and the screen saver (S2.5, S4).
    fn tick(self: &Rc<Self>) -> glib::ControlFlow {
        let elapsed = {
            let mut running = self.running.borrow_mut();
            let Some(running) = running.as_mut() else {
                return glib::ControlFlow::Break;
            };
            if let Some(peak) = running.recording.take_peak_dbfs() {
                running.levels.push(peak);
                running.heard_sound |= peak > fc_text::record::SILENCE_PEAK_DBFS;
            }
            let elapsed = running.recording.elapsed();
            let silent = !running.heard_sound
                && elapsed.as_millis() >= u128::from(fc_text::record::SILENCE_WARNING_AFTER_MS);
            let say = self.say();
            self.row
                .tick(elapsed, &running.levels, silent, say.as_ref());
            let warn_now = voice_notes::warns(elapsed) && !running.warned;
            if warn_now {
                running.warned = true;
            }
            let silence_now = silent && !running.said_silence;
            if silence_now {
                running.said_silence = true;
            }
            (elapsed, warn_now, silence_now)
        };
        let (elapsed, warn_now, silence_now) = elapsed;
        if warn_now {
            self.say_aloud(self.say().get("30 seconds left"));
        }
        if silence_now {
            crate::fc_info!("voice: nothing heard {} ms in", elapsed.as_millis());
            self.say_aloud(
                self.say()
                    .get("We can't hear anything. Is the microphone muted?"),
            );
        }
        if voice_notes::is_done(elapsed) {
            self.end(RecordingEnd::Capped);
            return glib::ControlFlow::Break;
        }
        if Shell::get().app.is_screensaver_active() {
            self.end(RecordingEnd::SessionLocked);
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    }

    // ---- ending -------------------------------------------------------------------------------

    /// The microphone's part of every ending: let go now, the row gives the field back.
    fn stopped(&self) -> Option<Running> {
        let mut running = self.running.borrow_mut().take()?;
        if let Some(timer) = running.timer.take() {
            timer.remove();
        }
        drop(running.awake.take());
        super::now_playing().set_recording(false);
        self.row.end();
        Some(running)
    }

    /// However a recording ends, what it recorded goes where `not_sent::ended` says.
    fn end(self: &Rc<Self>, why: RecordingEnd) {
        let Some(running) = self.stopped() else {
            return;
        };
        self.settling.set(self.settling.get() + 1);
        self.host.changed();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let recorded = running.recording.stop().await;
            let Some(composer) = weak.upgrade() else {
                return;
            };
            let say = composer.say();
            let outcome = not_sent::ended(
                why,
                recorded.elapsed,
                recorded.bytes.as_ref().map(Vec::len),
                say.as_ref(),
            );
            let duration_ms = voice_notes::duration_ms(recorded.elapsed);
            let went = match (outcome.fate, recorded.bytes) {
                (RecordingFate::Discard, _) | (_, None) => RecordingFate::Discard,
                (fate, Some(bytes)) => {
                    composer
                        .settle(running.chat_id, bytes, recorded.elapsed, fate)
                        .await
                }
            };
            composer
                .settling
                .set(composer.settling.get().saturating_sub(1));
            if let Some(sentence) = outcome.sentence {
                composer.tell(&sentence);
            } else if let Some(said) = not_sent::said(why, went, duration_ms, say.as_ref()) {
                composer.say_aloud(&said);
            }
            composer.refresh_rows();
            composer.host.changed();
        });
    }

    /// What was recorded, where it goes: sent, staged for review, or parked. Answers where it went.
    async fn settle(
        self: &Rc<Self>,
        chat_id: i64,
        bytes: Vec<u8>,
        elapsed: Duration,
        fate: RecordingFate,
    ) -> RecordingFate {
        let Some(note) = voice_notes::staged(bytes.clone(), elapsed) else {
            return RecordingFate::Discard;
        };
        let same_chat = self.host.chat_id() == chat_id;
        match fate {
            RecordingFate::Send | RecordingFate::Review if same_chat => {
                let note = shaped(note).await;
                if fate == RecordingFate::Send {
                    self.host.send(note);
                } else {
                    self.host.stage(note);
                }
                fate
            }
            // The chat changed meanwhile: it waits there as not sent, never lost.
            _ => {
                self.park(chat_id, &bytes, voice_notes::duration_ms(elapsed));
                RecordingFate::Park
            }
        }
    }

    /// Written down as a voice message that was not sent; held in memory where the disk refuses.
    /// In the composer's own chat it keeps the reply the composer is primed with, which leaves the
    /// composer when nothing else there would carry it (`not_sent::for_recording`).
    fn park(&self, chat_id: i64, bytes: &[u8], duration_ms: i32) {
        let Some(store) = slots().and_then(|slots| slots.store()) else {
            crate::fc_warn!("not sent: nobody is signed in; a recording was let go");
            return;
        };
        let now = Shell::get().clock.now();
        let take = if self.host.chat_id() == chat_id {
            not_sent::for_recording(
                &self.host.draft(),
                self.host.reply_to(),
                false,
                self.host.staged() > 0,
            )
        } else {
            not_sent::ComposerTake::NOTHING
        };
        if take.clears_reply {
            self.host.release_reply();
        }
        let reply = take.reply_to;
        match store.park(chat_id, bytes, duration_ms, reply, None, now) {
            Ok(entry) => crate::fc_info!(
                "not sent: kept in chat {} ({} ms)",
                entry.chat_id,
                entry.duration_ms
            ),
            Err(error) => {
                crate::fc_warn!(
                    "not sent: the disk refused ({}); held in memory",
                    error.kind()
                );
                store.hold(chat_id, bytes, duration_ms, reply, None, now);
            }
        }
    }

    /// The row's Delete (S2.5): under ten seconds at once; from ten it stops first, then asks.
    fn delete(self: &Rc<Self>) {
        let elapsed = match self.running.borrow().as_ref() {
            Some(running) => running.recording.elapsed(),
            None => return,
        };
        if elapsed < voice_notes::DELETE_ASKS_FROM {
            self.end(RecordingEnd::Deleted);
            return;
        }
        let Some(running) = self.stopped() else {
            return;
        };
        self.settling.set(self.settling.get() + 1);
        self.host.changed();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let recorded = running.recording.stop().await;
            let Some(composer) = weak.upgrade() else {
                return;
            };
            let say = composer.say();
            let outcome = not_sent::ended(
                RecordingEnd::Stopped,
                recorded.elapsed,
                recorded.bytes.as_ref().map(Vec::len),
                say.as_ref(),
            );
            let (Some(bytes), RecordingFate::Review) = (recorded.bytes, outcome.fate) else {
                composer
                    .settling
                    .set(composer.settling.get().saturating_sub(1));
                if let Some(sentence) = outcome.sentence {
                    composer.tell(&sentence);
                }
                composer.host.changed();
                return;
            };
            let answer = composer.ask_delete().await;
            let duration_ms = voice_notes::duration_ms(recorded.elapsed);
            let (why, went) = if answer {
                (RecordingEnd::Deleted, RecordingFate::Discard)
            } else {
                let went = composer
                    .settle(
                        running.chat_id,
                        bytes,
                        recorded.elapsed,
                        RecordingFate::Review,
                    )
                    .await;
                (RecordingEnd::Stopped, went)
            };
            composer
                .settling
                .set(composer.settling.get().saturating_sub(1));
            if let Some(said) = not_sent::said(why, went, duration_ms, say.as_ref()) {
                composer.say_aloud(&said);
            }
            composer.refresh_rows();
            composer.host.changed();
        });
    }

    /// "Delete this recording?" [Delete] [Keep] — answers true for Delete. Presented with
    /// `present` and its `response` signal (never `choose_future`, §3.2).
    async fn ask_delete(&self) -> bool {
        let say = self.say();
        let dialog = adw::AlertDialog::new(Some(say.get("Delete this recording?")), None);
        dialog.add_response("keep", say.get("Keep"));
        dialog.add_response("delete", say.get("Delete"));
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("keep"));
        dialog.set_close_response("keep");
        let (sender, receiver) = futures_channel::oneshot::channel::<bool>();
        let sender = RefCell::new(Some(sender));
        dialog.connect_response(None, move |_, response| {
            if let Some(sender) = sender.borrow_mut().take() {
                let _ = sender.send(response == "delete");
            }
        });
        dialog.present(self.host.window().as_ref());
        receiver.await.unwrap_or(false)
    }

    /// Quitting: the running recording stopped and parked synchronously.
    fn park_now(self: &Rc<Self>) {
        let Some(running) = self.stopped() else {
            return;
        };
        let recorded = match &running.concrete {
            Some(concrete) => concrete.stop_now(PARK_LIMIT),
            None => {
                running.recording.abandon();
                return;
            }
        };
        let say = self.say();
        let outcome = not_sent::ended(
            RecordingEnd::WindowClosed,
            recorded.elapsed,
            recorded.bytes.as_ref().map(Vec::len),
            say.as_ref(),
        );
        if let (RecordingFate::Park, Some(bytes)) = (outcome.fate, recorded.bytes) {
            self.park(
                running.chat_id,
                &bytes,
                voice_notes::duration_ms(recorded.elapsed),
            );
        }
    }

    // ---- the not-sent rows (S2.8) -------------------------------------------------------------

    /// Rebuilds the rows of this composer's chat from the store.
    pub fn refresh_rows(self: &Rc<Self>) {
        while let Some(child) = self.rows.first_child() {
            self.rows.remove(&child);
        }
        let Some(store) = slots().and_then(|slots| slots.store()) else {
            return;
        };
        for entry in store.of(self.host.chat_id()) {
            let row = self.not_sent_row(&store, entry);
            self.rows.append(&row);
        }
    }

    fn not_sent_row(
        self: &Rc<Self>,
        store: &Rc<ParkedRecordings>,
        entry: ParkedRecording,
    ) -> gtk::Widget {
        let say = self.say();
        let line = not_sent::line(entry.duration_ms, say.as_ref());
        // What it carries, on one line under it: the reply it was recorded under and its caption
        // (the Mac's `NotSentVoiceRow`).
        let carried: Vec<String> = entry
            .reply_to_message_id
            .and_then(|reply| self.host.reply_line(reply))
            .into_iter()
            .chain(entry.caption.clone())
            .collect();
        let play = gtk::Button::builder()
            .icon_name("media-playback-start-symbolic")
            .valign(gtk::Align::Center)
            .css_classes(["circular", "flat"])
            .tooltip_text(say.get("Play"))
            .build();
        play.update_property(&[gtk::accessible::Property::Label(say.get("Play"))]);
        let bars = voice_look::bar_count(120.0);
        let waves = Waves::new(120.0, voice_look::CHIP_WAVE_HEIGHT, Ink::Accent);
        waves.set_levels(voice_look::bars(None, bars));
        let label = gtk::Label::builder()
            .label(&line)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["caption", "fc-media-warning"])
            .build();
        let words = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .valign(gtk::Align::Center)
            .hexpand(true)
            .build();
        words.append(&label);
        if !carried.is_empty() {
            words.append(
                &gtk::Label::builder()
                    .label(carried.join(" · "))
                    .xalign(0.0)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .css_classes(["caption", "dim-label"])
                    .build(),
            );
        }
        let send = gtk::Button::builder()
            .icon_name(fc_logic::composer_button::SlotIcon::Send.icon_name())
            .valign(gtk::Align::Center)
            .css_classes(["circular", "suggested-action"])
            .tooltip_text(say.get("Send voice message"))
            .build();
        send.update_property(&[gtk::accessible::Property::Label(
            say.get("Send voice message"),
        )]);
        let remove = gtk::Button::builder()
            .icon_name("window-close-symbolic")
            .valign(gtk::Align::Center)
            .css_classes(["circular", "flat"])
            .tooltip_text(say.get("Delete recording"))
            .build();
        remove.update_property(&[gtk::accessible::Property::Label(
            say.get("Delete recording"),
        )]);
        let chip = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .css_classes(["card", "fc-media-chip"])
            .accessible_role(gtk::AccessibleRole::Group)
            .build();
        let spoken = std::iter::once(line.clone())
            .chain(carried.iter().cloned())
            .collect::<Vec<_>>()
            .join(" · ");
        chip.update_property(&[gtk::accessible::Property::Label(&spoken)]);
        chip.append(&play);
        chip.append(waves.widget());
        chip.append(&words);
        chip.append(&send);
        chip.append(&remove);
        // Its shape: measured from its bytes once per run.
        self.shape_of(store, &entry, &waves, bars);
        let weak = Rc::downgrade(self);
        {
            let (weak, store, entry) = (weak.clone(), Rc::clone(store), entry.clone());
            play.connect_clicked(move |button| {
                if let Some(composer) = weak.upgrade() {
                    composer.play_row(&store, &entry, button);
                }
            });
        }
        {
            let (weak, store, entry) = (weak.clone(), Rc::clone(store), entry.clone());
            send.connect_clicked(move |_| {
                if let Some(composer) = weak.upgrade() {
                    composer.send_row(&store, &entry);
                }
            });
        }
        {
            let (store, entry) = (Rc::clone(store), entry);
            remove.connect_clicked(move |_| {
                if let Some(composer) = weak.upgrade() {
                    composer.remove_row(&store, &entry);
                }
            });
        }
        chip.upcast()
    }

    fn shape_of(
        &self,
        store: &Rc<ParkedRecordings>,
        entry: &ParkedRecording,
        waves: &Waves,
        bars: usize,
    ) {
        let Some(owner) = slots() else {
            return;
        };
        if let Some(known) = owner.shapes.borrow().get(&entry.id) {
            waves.set_levels(voice_look::bars(known.as_deref(), bars));
            return;
        }
        let Some(media) = store.staged(entry) else {
            return;
        };
        let (waves, id, decoder) = (waves.clone(), entry.id.clone(), Rc::clone(&owner.decoder));
        glib::spawn_future_local(async move {
            let clock = Shell::get().clock.clone();
            let shape = voice_waveform::measure(
                Rc::from(media.bytes.as_slice()),
                decoder.as_ref(),
                clock.as_ref(),
            )
            .await;
            waves.set_levels(voice_look::bars(shape.as_deref(), bars));
            if let Some(slots) = slots() {
                slots.shapes.borrow_mut().insert(id, shape);
            }
        });
    }

    fn play_row(
        self: &Rc<Self>,
        store: &Rc<ParkedRecordings>,
        entry: &ParkedRecording,
        button: &gtk::Button,
    ) {
        let registry = super::now_playing();
        if self
            .playing
            .borrow()
            .as_ref()
            .is_some_and(|(id, _, _)| *id == entry.id)
        {
            self.stop_row_player();
            button.set_icon_name("media-playback-start-symbolic");
            return;
        }
        if registry.refuses() {
            self.tell(self.say().get("You can play this after recording."));
            return;
        }
        self.stop_row_player();
        let Some(media) = store.staged(entry) else {
            return;
        };
        let shell = Shell::get();
        let Ok(file) = super::gst_util::temp_file(&shell.paths.tmp_dir(), "not-sent", "m4a") else {
            return;
        };
        if std::fs::write(&file, &media.bytes).is_err() {
            super::gst_util::remove_file(&file);
            return;
        }
        let player = shell.players.get().create(false);
        let (weak, icon) = (Rc::downgrade(self), button.downgrade());
        let claim = registry.claim(
            Playing::VoiceNote,
            Rc::new({
                let weak = weak.clone();
                move || {
                    if let Some(composer) = weak.upgrade() {
                        composer.stop_row_player();
                    }
                }
            }),
        );
        {
            let (weak, icon) = (weak.clone(), icon.clone());
            player.on_state().subscribe(move |state| {
                if matches!(state, PlayerState::Ended | PlayerState::Failed)
                    && let Some(composer) = weak.upgrade()
                {
                    composer.stop_row_player();
                    if let Some(icon) = icon.upgrade() {
                        icon.set_icon_name("media-playback-start-symbolic");
                    }
                }
            });
        }
        player.load(MediaSource::File(file.clone()));
        player.set_rate(super::voice_bubble::current_rate(&shell));
        player.play();
        button.set_icon_name("media-playback-pause-symbolic");
        *self.playing.borrow_mut() = Some((entry.id.clone(), player, claim));
        // The temporary goes once the player has it open; the sweep at launch takes a leftover.
        glib::timeout_add_local_once(Duration::from_secs(2), move || {
            super::gst_util::remove_file(&file)
        });
    }

    fn stop_row_player(&self) {
        let playing = self.playing.borrow_mut().take();
        if let Some((_, player, claim)) = playing {
            player.stop();
            super::now_playing().release(claim);
        }
    }

    /// The row's own Send: never carried by another Send (S2.8).
    fn send_row(self: &Rc<Self>, store: &Rc<ParkedRecordings>, entry: &ParkedRecording) {
        let Some(media) = store.staged(entry) else {
            self.refresh_rows();
            return;
        };
        self.stop_row_player();
        let (weak, store, entry) = (Rc::downgrade(self), Rc::clone(store), entry.clone());
        glib::spawn_future_local(async move {
            let note = shaped(media).await;
            let Some(composer) = weak.upgrade() else {
                return;
            };
            let (caption, reply) = (entry.caption.clone(), entry.reply_to_message_id);
            let sent = {
                let weak = Rc::downgrade(&composer);
                Box::new(move || {
                    if !store.remove(&entry) {
                        crate::fc_warn!(
                            "not sent: a sent recording could not be taken out of the store"
                        );
                    }
                    let Some(composer) = weak.upgrade() else {
                        return;
                    };
                    let say = composer.say();
                    let (_, said) = not_sent::row_ended(true, entry.duration_ms, say.as_ref());
                    composer.say_aloud(&said);
                    composer.refresh_rows();
                    composer.host.changed();
                })
            };
            composer.host.send_parked(note, caption, reply, sent);
        });
    }

    /// The row's ✕: asks first from ten seconds (it cannot be made again).
    fn remove_row(self: &Rc<Self>, store: &Rc<ParkedRecordings>, entry: &ParkedRecording) {
        self.stop_row_player();
        let (weak, store, entry) = (Rc::downgrade(self), Rc::clone(store), entry.clone());
        glib::spawn_future_local(async move {
            let Some(composer) = weak.upgrade() else {
                return;
            };
            if voice_notes::asks_before_deleting(entry.duration_ms) && !composer.ask_delete().await
            {
                return;
            }
            store.remove(&entry);
            let say = composer.say();
            let (_, said) = not_sent::row_ended(false, entry.duration_ms, say.as_ref());
            composer.say_aloud(&said);
            composer.refresh_rows();
            composer.host.changed();
        });
    }
}

impl ComposerRecording for ComposerVoice {
    fn row(&self) -> gtk::Widget {
        self.row.widget()
    }

    fn recording(&self) -> Recording {
        match self.running.borrow().as_ref() {
            Some(running) if running.beside_draft => Recording::HandsFreeBesideDraft,
            Some(_) => Recording::HandsFree,
            None => Recording::None,
        }
    }

    fn can_record(&self) -> bool {
        let microphone = super::test_sources() || super::gst_util::has_element("pulsesrc");
        microphone
            && super::gst_util::has_element("mp4mux")
            && super::pipelines::AacEncoder::ORDER
                .iter()
                .any(|encoder| super::gst_util::has_element(encoder.factory()))
    }

    fn not_sent(&self) -> bool {
        slots()
            .and_then(|slots| slots.store())
            .is_some_and(|store| store.any(self.host.chat_id()))
    }

    fn not_sent_rows(&self) -> Option<gtk::Widget> {
        let any = self.rows.first_child().is_some();
        any.then(|| self.above.clone().upcast())
    }

    fn recorder_open(&self) -> bool {
        super::recorder_layer::is_open()
    }

    fn video_offered(&self) -> bool {
        let shell = Shell::get();
        let limits = shell
            .connection()
            .and_then(|connection| connection.session.state().round_video);
        round_video_rules::available(
            round_video_rules::RECORDING_ENABLED,
            limits.as_ref(),
            shell.camera.get().present(),
        )
    }

    fn start_voice(&self, beside_draft: bool) {
        if let Some(me) = self.me() {
            me.begin(beside_draft);
        }
    }

    fn stop(&self) {
        if let Some(me) = self.me() {
            me.end(RecordingEnd::Stopped);
        }
    }

    fn send(&self) {
        if let Some(me) = self.me() {
            me.end(RecordingEnd::Sent);
        }
    }

    fn interrupt(&self, why: RecordingEnd) {
        // A start still waiting on the lead-in lets go of the microphone once it is granted (S4).
        self.start.interrupted();
        if let Some(me) = self.me() {
            if why == RecordingEnd::LeftChat || why == RecordingEnd::WindowClosed {
                me.stop_row_player();
            }
            me.end(why);
        }
        if let Some(event) = fc_logic::round_look::playback_pauses::of(why) {
            super::now_playing().interrupt(event);
        }
    }

    fn record_video(&self) {
        // No video message is recorded over a call (§14.4): said, never a camera opened.
        if Self::call_busy() {
            if let Some(me) = self.me() {
                me.tell(
                    Shell::get()
                        .say
                        .get("You can record a message after the call."),
                );
            }
            return;
        }
        super::recorder_layer::open(self.host.window(), Rc::clone(&self.host));
    }
}

impl ComposerVoice {
    /// The `Rc` of this composer (the trait methods take `&self`).
    fn me(&self) -> Option<Rc<ComposerVoice>> {
        slots()?
            .composers()
            .into_iter()
            .find(|composer| std::ptr::eq(Rc::as_ptr(composer), self))
    }
}

/// The note with its shape measured from its own bytes (`voice_waveform::shaped`).
async fn shaped(note: StagedMedia) -> StagedMedia {
    let Some(slots) = slots() else {
        return note;
    };
    let clock = Shell::get().clock.clone();
    let decoder = Rc::clone(&slots.decoder);
    voice_waveform::shaped(note, decoder.as_ref(), clock.as_ref()).await
}

fn slots_recorder() -> Rc<GstVoiceRecorder> {
    slots().map_or_else(
        || Rc::new(GstVoiceRecorder::new(Shell::get().paths.tmp_dir())),
        |slots| Rc::clone(&slots.recorder),
    )
}

/// Whether the trait object and the concrete recording are the same object.
fn same(concrete: &Rc<GstVoiceRecording>, recording: &Rc<dyn VoiceRecording>) -> bool {
    std::ptr::eq(
        Rc::as_ptr(concrete).cast::<()>(),
        Rc::as_ptr(recording).cast::<()>(),
    )
}
