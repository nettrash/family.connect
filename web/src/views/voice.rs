//! A voice message from the Send slot — Phase 1 of the plan for #79
//! (docs/audio-video-messages-2026-10-04.md, "the plan" below: S1, S2, S6,
//! S8.7, S8.8).
//!
//! The rules are fc_text::record's, run as they are. Every activation of the
//! slot, every "Record Voice Message", every Stop, Delete and answer, the
//! five-minute limit and every interruption is an event for
//! [`record::hold_step`]; what it says to do is done here — the microphone
//! opened, the note sent, staged, kept as not sent or deleted, the sentence
//! shown, the change said. Nothing here decides anything the module decides:
//! the 600 ms activation guard, the one-second floor, "Delete this
//! recording?" from ten seconds, what an interruption does.
//!
//! The browser has no hold in this version (S8.8: a touch `pointerdown` is
//! not user activation, and Safari's audio needs one), so a press of any
//! length is a click — `Activate` — and the hold, the slides and the Undo
//! window never happen here; nor can a page tell that a screen reader runs
//! (S6), which a held release would have to.
//!
//! The browser asks for the microphone itself, the first time and whenever
//! it has not been told to remember; a page cannot know beforehand whether
//! it will. So every start here goes through the module's "not asked yet":
//! the microphone is asked for (`AskPermission`), and the answer — the
//! browser's prompt, or its silent grant — is what starts the recording
//! (`PermissionAnswer`). Work that outlives the asking is let go of if
//! anything moved the recording on meanwhile.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use fc_text::i18n::{t, t1, tn};
use fc_text::media;
use fc_text::record::{
    self, Announcement, Dimmed, HoldConstants, HoldEffect, HoldEvent, HoldState, Permission, Phase,
    Situation,
};
use futures::future::{FutureExt, LocalBoxFuture, Shared};
use wasm_bindgen_futures::spawn_local;
use yew::prelude::*;

use crate::actions::Action;
use crate::prep;
use crate::recorder::{self, Failure, Handover, Listening, Lost, Meter, Recording};
use crate::staged::Prepared;

/// "We can't hear anything. Is the microphone muted?" — S2.9's line, shown
/// and said three seconds into a recording that has heard nothing louder
/// than digital silence, and gone once sound arrives.
pub const SILENCE: &str = "We can't hear anything. Is the microphone muted?";

/// What the pane is at its latest render, for work that outlives the render
/// that started it.
#[derive(Clone)]
pub struct Setup {
    pub chat_id: i64,
    /// The sign-in the pane belongs to (`AppState::session`).
    pub session: u64,
    pub on_action: Callback<Action>,
    /// How many items are staged in this chat now.
    pub staged: usize,
    /// Why the microphone is dimmed, if it is (S1.3 rows 7–9) — the reason
    /// the module says instead of recording.
    pub blocked: Option<Dimmed>,
    /// A call in any phase but ended.
    pub on_call: bool,
    /// What the notice line says now.
    pub notice_now: Option<String>,
}

/// Why the microphone is dimmed (S1.3 rows 7–9, in that order): a call, the
/// composer's attachment guard, a voice message that was not sent.
pub fn blocked(on_call: bool, busy: bool, not_sent: bool) -> Option<Dimmed> {
    if on_call {
        Some(Dimmed::Call)
    } else if busy {
        Some(Dimmed::Busy)
    } else if not_sent {
        Some(Dimmed::NotSent)
    } else {
        None
    }
}

/// A recording stopped and being finished into a note — shared, because the
/// one stopped by Delete at ten seconds or more is finished while "Delete
/// this recording?" waits for an answer that decides what becomes of it.
type Finishing = Shared<LocalBoxFuture<'static, Option<Finished>>>;

/// A finished recording: the note, or why it cannot be one — and its length
/// by the samples. None at all (in [`Finishing`]): too short to be anything.
#[derive(Clone)]
struct Finished {
    note: Result<Prepared, String>,
    duration_ms: i64,
}

/// Stop `recording` — its microphone goes off now — and finish what it
/// heard into a note the way the composer stages one.
/// Whether the module placed `effect` to describe the effect before it:
/// shown, said, felt or learnt about it.
fn describes(effect: HoldEffect) -> bool {
    matches!(
        effect,
        HoldEffect::Announce(_)
            | HoldEffect::Hint(_)
            | HoldEffect::Haptic(_)
            | HoldEffect::FirstReleaseDone
    )
}

fn finish(recording: Recording) -> Finishing {
    async move {
        let recorded = recording.stop().await?;
        let duration_ms = recorded.duration_ms;
        let note = prep::recording(recorded.blob, recorded.mime, duration_ms, recorded.waveform)
            .await
            .map_err(|error| error.message().to_string());
        Some(Finished { note, duration_ms })
    }
    .boxed_local()
    .shared()
}

#[derive(Default)]
struct Machine {
    hold: HoldState,
    /// The recording while it runs.
    live: Option<Recording>,
    /// Stopped by Delete at ten seconds or more, its question open.
    asked: Option<Finishing>,
    /// Why the microphone was not given, for the `Denied` that follows.
    refused: Option<Failure>,
}

/// What ended a recording, for where the focus goes after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ending {
    /// The person, in the composer: the slot's Send arrow or Stop square
    /// (or "too short" from it), the row's Stop and Delete, Esc, the
    /// question answered. The cursor goes back into the box (S2.4, S2.7).
    ByThePerson,
    /// The five-minute limit, into review: back into the box only if the
    /// focus is still the composer's — the person may have put it elsewhere
    /// in five minutes, and a limit running out takes nothing from them.
    ByTheLimit,
    /// A call, a hidden tab, the pane going, the microphone lost: "stop →
    /// not sent", and nothing else (S4). A ringing call has put the focus on
    /// its Accept — "Return answers it" — and the box must not take it back
    /// from there: Return in the box sends what is in it.
    Interrupted,
}

impl Ending {
    /// What ended it, by the event that did. Anything not named is taken
    /// for an interruption, which moves nothing.
    fn of(event: &HoldEvent) -> Ending {
        match event {
            HoldEvent::Activate { .. }
            | HoldEvent::Stop { .. }
            | HoldEvent::Delete { .. }
            | HoldEvent::Answer { .. }
            | HoldEvent::Record { .. }
            | HoldEvent::Up { .. } => Ending::ByThePerson,
            HoldEvent::Cap { .. } => Ending::ByTheLimit,
            _ => Ending::Interrupted,
        }
    }
}

/// What the view draws of it.
#[derive(Clone, PartialEq, Default)]
struct View {
    hold: HoldState,
    started_ms: f64,
    meter: Option<Meter>,
}

struct Driver {
    machine: RefCell<Machine>,
    setup: RefCell<Setup>,
    alive: Rc<RefCell<bool>>,
    replying: UseStateHandle<Option<i64>>,
    replying_now: Rc<RefCell<Option<i64>>>,
    notice: UseStateHandle<Option<String>>,
    view: UseStateHandle<View>,
    announcement: UseStateHandle<(u32, String)>,
    said: Cell<u32>,
    focus: UseStateHandle<u32>,
    focused: Cell<u32>,
    refocus: UseStateHandle<u32>,
    refocused: Cell<u32>,
    /// Notes stopped by Send or into review and still being finished — the
    /// seconds a long one takes to encode once it stops (`Recording::stop`).
    /// The composer is busy meanwhile: Stop beside words stages the note
    /// beside them (S1.3 row 3, S2.4), and a Send pressed before it lands
    /// would send the words without it.
    finishing: UseStateHandle<u32>,
    finishing_now: Cell<u32>,
}

/// The monotonic clock, as the module counts it.
fn at() -> u64 {
    recorder::now_ms().max(0.0) as u64
}

/// What is SAID for an announcement, in the reader's language.
fn said(announcement: Announcement) -> String {
    match announcement {
        Announcement::ReadyToReview { recorded_ms } => t1(
            announcement.text(),
            &media::time_label(recorded_ms as f64 / 1000.0),
        ),
        other => t(other.text()).to_string(),
    }
}

impl Driver {
    fn alive(&self) -> bool {
        *self.alive.borrow()
    }

    /// How long the running recording is, by its own clock.
    fn recorded(&self) -> u64 {
        self.machine
            .borrow()
            .live
            .as_ref()
            .map_or(0, |live| live.elapsed_ms().max(0.0) as u64)
    }

    fn situation(&self) -> Situation {
        // A note still being finished is an attachment not done yet: S1.3's
        // row 8, after a call and before a note that was not sent.
        let blocked = match self.setup.borrow().blocked {
            None | Some(Dimmed::NotSent) if self.finishing_now.get() > 0 => Some(Dimmed::Busy),
            blocked => blocked,
        };
        Situation {
            permission: Permission::NotAsked,
            blocked,
            ..Situation::default()
        }
    }

    /// A note begins being finished into the composer…
    fn finishing_began(&self) {
        self.finishing_now.set(self.finishing_now.get() + 1);
        if self.alive() {
            self.finishing.set(self.finishing_now.get());
        }
    }

    /// …and has landed, or come to nothing.
    fn finishing_ended(&self) {
        self.finishing_now
            .set(self.finishing_now.get().saturating_sub(1));
        if self.alive() {
            self.finishing.set(self.finishing_now.get());
        }
    }

    /// One step of the module, and what it says done — in its order.
    fn step(self: &Rc<Self>, event: HoldEvent) -> Vec<HoldEffect> {
        let before = self.machine.borrow().hold;
        let (after, effects) = record::hold_step(before, event, &HoldConstants::default());
        self.machine.borrow_mut().hold = after;
        let recording =
            |phase: Phase| matches!(phase, Phase::HandsFree { .. } | Phase::Holding { .. });
        if recording(before.phase) && !recording(after.phase) {
            // The silence line is about a recording that runs.
            self.quiet();
        }
        // What the module says ABOUT a Send or a Review — the words, the
        // hint, the haptic it always places right after the thing — waits
        // for the thing to have happened (S2.5, S6): the note is finished
        // asynchronously, and may turn out too short or too big, so "Voice
        // message sent" is said once the note is handed to the outbox, and
        // "Ready to review" once it is in the chip — never before a failure
        // that says something else.
        let mut rest = effects.iter().copied().peekable();
        while let Some(effect) = rest.next() {
            let mut told = Vec::new();
            if matches!(effect, HoldEffect::Send | HoldEffect::Review) {
                while let Some(about) = rest.next_if(|next| describes(*next)) {
                    told.push(about);
                }
            }
            self.run(effect, told);
        }
        self.publish(before.phase, &event);
        effects
    }

    /// Run what the module said about an effect, now that it happened.
    fn tell(self: &Rc<Self>, told: Vec<HoldEffect>) {
        for effect in told {
            self.run(effect, Vec::new());
        }
    }

    /// `told` is what the module said about `effect` (see [`Driver::step`]),
    /// for a Send or a Review to say once it has happened.
    fn run(self: &Rc<Self>, effect: HoldEffect, told: Vec<HoldEffect>) {
        match effect {
            // The microphone is already open: it was asked for, and the
            // answer is what started this.
            HoldEffect::Start { .. } => crate::views::attach::pause_all_playing(None),
            HoldEffect::Send => self.send(told),
            HoldEffect::Review => self.review(told),
            HoldEffect::Park => self.park(),
            HoldEffect::Delete => self.delete(),
            HoldEffect::AskDelete => self.ask_delete(),
            HoldEffect::AskPermission => self.open_microphone(),
            HoldEffect::Denied => {
                let failure = self
                    .machine
                    .borrow_mut()
                    .refused
                    .take()
                    .unwrap_or(Failure::CouldNotStart);
                self.say(failure.message().to_string());
            }
            HoldEffect::Explain(reason) => self.say(t(reason.notice()).to_string()),
            HoldEffect::Hint(hint) => self.say(t(hint.text()).to_string()),
            // Always by the announcement node, which is always there to say
            // it (S6) — "That recording was too short." and "Recording
            // stopped at five minutes." too. The notice line showing the
            // same words is quiet for them, so they are said once
            // (conversation.rs).
            HoldEffect::Announce(announcement) => self.announce(said(announcement)),
            // No hold in a browser (S8.8) — no lock, no slide, no Undo
            // window — and no haptics off a phone (S2.9).
            HoldEffect::Lock
            | HoldEffect::Arm
            | HoldEffect::Disarm
            | HoldEffect::UndoWindow
            | HoldEffect::UndoSend
            | HoldEffect::UndoReview
            | HoldEffect::FirstReleaseDone
            | HoldEffect::Haptic(_) => {}
        }
    }

    /// The view drawn again — and, a recording over or its question
    /// answered by the person, the cursor back in the box (S2.4): a second
    /// Enter cannot open the microphone again, and the words of a caption go
    /// where they belong (S2.7). What else ended it decides where the focus
    /// goes — see [`Ending`].
    fn publish(&self, before: Phase, event: &HoldEvent) {
        if !self.alive() {
            return;
        }
        let machine = self.machine.borrow();
        let live = machine.live.as_ref();
        self.view.set(View {
            hold: machine.hold,
            started_ms: live.map_or(0.0, Recording::started_ms),
            meter: live.and_then(Recording::meter),
        });
        let ended = matches!(
            before,
            Phase::HandsFree { .. } | Phase::Holding { .. } | Phase::AskingDelete { .. }
        );
        if !ended || machine.hold.phase != Phase::Idle {
            return;
        }
        match Ending::of(event) {
            Ending::ByThePerson => {
                self.focused.set(self.focused.get() + 1);
                self.focus.set(self.focused.get());
            }
            Ending::ByTheLimit => {
                self.refocused.set(self.refocused.get() + 1);
                self.refocus.set(self.refocused.get());
            }
            Ending::Interrupted => {}
        }
    }

    /// "That recording was too short." — shown, and said by the node. (The
    /// key is written out, `Hint::TooShort`'s own words, so that the web's
    /// catalogue scan finds it: web/i18n/generate.py reads `t("…")` calls.)
    fn too_short(&self) {
        let words = t("That recording was too short.").to_string();
        self.say(words.clone());
        self.announce(words);
    }

    fn say(&self, text: String) {
        if self.alive() {
            self.notice.set(Some(text));
        }
    }

    fn announce(&self, text: String) {
        if self.alive() {
            self.said.set(self.said.get() + 1);
            self.announcement.set((self.said.get(), text));
        }
    }

    /// The silence line taken down — if it is still what the line says.
    fn quiet(&self) {
        if self.setup.borrow().notice_now.as_deref() == Some(t(SILENCE)) {
            self.say_nothing();
        }
    }

    fn say_nothing(&self) {
        if self.alive() {
            self.notice.set(None);
        }
    }

    fn take_live(&self) -> Option<Recording> {
        self.machine.borrow_mut().live.take()
    }

    fn take_asked(&self) -> Option<Finishing> {
        self.machine.borrow_mut().asked.take()
    }

    /// The reply the box is answering — given to what is recorded under it,
    /// and so taken off the box.
    fn take_reply(&self) -> Option<i64> {
        let reply = *self.replying_now.borrow();
        if self.alive() && reply.is_some() {
            self.replying.set(None);
        }
        reply
    }

    /// Ask the browser for the microphone — IN the click that asked for a
    /// recording, which is what lets its audio run (`Listening`) — and let
    /// the answer start the recording, or say why not.
    fn open_microphone(self: &Rc<Self>) {
        let listening = Listening::in_the_click();
        let this = self.clone();
        spawn_local(async move {
            let started = Recording::start(listening).await;
            let waiting = matches!(
                this.machine.borrow().hold.phase,
                Phase::AwaitingPermission { .. }
            );
            // Overtaken while the browser asked — the pane gone, the tab
            // hidden, a call, each of which put the recording back to
            // nothing: let go of, and the microphone with it. There is
            // nothing in it to keep.
            if !this.alive() || !waiting {
                return;
            }
            let overtaken = this.setup.borrow().on_call || !crate::sync::page_visible();
            match started {
                Ok(active) if overtaken => {
                    drop(active);
                    this.interrupt();
                }
                Ok(mut active) => {
                    let weak = Rc::downgrade(&this);
                    active.on_lost(move |lost| {
                        // Not from inside the track's own event: the
                        // recording is handed on, and whatever finishes it
                        // takes these listeners off.
                        let weak = weak.clone();
                        spawn_local(async move {
                            if let Some(this) = weak.upgrade() {
                                this.lost(lost);
                            }
                        });
                    });
                    this.machine.borrow_mut().live = Some(active);
                    this.step(HoldEvent::PermissionAnswer {
                        at_ms: at(),
                        granted: true,
                    });
                }
                Err(failure) => {
                    this.machine.borrow_mut().refused = Some(failure);
                    this.step(HoldEvent::PermissionAnswer {
                        at_ms: at(),
                        granted: false,
                    });
                }
            }
        });
    }

    /// The Send arrow (S2.5): stopped, finished and handed to the outbox
    /// with the reply the box was answering — alone, as nothing else can be
    /// in a composer that was empty. Too short, or too big to send, it says
    /// so.
    fn send(self: &Rc<Self>, told: Vec<HoldEffect>) {
        let Some(active) = self.take_live() else {
            return;
        };
        let reply_to_message_id = self.take_reply();
        let (chat_id, session, on_action) = {
            let setup = self.setup.borrow();
            (setup.chat_id, setup.session, setup.on_action.clone())
        };
        let this = self.clone();
        let finishing = finish(active);
        self.finishing_began();
        spawn_local(async move {
            let finished = finishing.await;
            this.finishing_ended();
            match finished {
                Some(Finished { note: Ok(note), .. }) => {
                    on_action.emit(Action::SendRecorded {
                        session,
                        chat_id,
                        note,
                        reply_to_message_id,
                    });
                    this.tell(told);
                }
                Some(Finished {
                    note: Err(error), ..
                }) => this.say(error),
                None => this.too_short(),
            }
        });
    }

    /// Stop, or Keep (S2.5, S2.7): staged in the chip above the box, for an
    /// optional caption — unless there is no room left on the strip, or the
    /// chat was left while it was finished, when it is kept as not sent
    /// rather than lost (S2.8): a recording cannot be made again.
    fn review(self: &Rc<Self>, told: Vec<HoldEffect>) {
        let finishing = match self.take_live() {
            Some(active) => finish(active),
            None => match self.take_asked() {
                Some(finishing) => finishing,
                None => return,
            },
        };
        let this = self.clone();
        self.finishing_began();
        spawn_local(async move {
            let finished = finishing.await;
            this.finishing_ended();
            let (chat_id, session, on_action, staged) = {
                let setup = this.setup.borrow();
                (
                    setup.chat_id,
                    setup.session,
                    setup.on_action.clone(),
                    setup.staged,
                )
            };
            match finished {
                None => this.too_short(),
                Some(Finished {
                    note: Err(error), ..
                }) => this.say(error),
                Some(Finished {
                    note: Ok(note),
                    duration_ms,
                }) => {
                    let full = !media::can_stage(staged);
                    if full {
                        this.say(tn(
                            "You can attach up to %lld items.",
                            media::MAX_PER_MESSAGE as i64,
                        ));
                    }
                    if full || !this.alive() {
                        on_action.emit(Action::ParkStopped {
                            session,
                            chat_id,
                            note,
                            duration_ms,
                            reply_to_message_id: *this.replying_now.borrow(),
                        });
                    } else {
                        on_action.emit(Action::Stage {
                            chat_id,
                            item: note,
                        });
                        this.tell(told);
                    }
                }
            }
        });
    }

    /// Stopped by something other than the person (S2.8, S4): handed to the
    /// app to be kept as "Voice message not sent", with the reply it was
    /// recorded under — and a recording already stopped by Delete, its
    /// question unanswered, is kept the same way.
    fn park(self: &Rc<Self>) {
        let reply_to_message_id = self.take_reply();
        let (chat_id, session, on_action) = {
            let setup = self.setup.borrow();
            (setup.chat_id, setup.session, setup.on_action.clone())
        };
        if let Some(active) = self.take_live() {
            on_action.emit(Action::Park {
                session,
                chat_id,
                recording: Handover::of(active),
                reply_to_message_id,
            });
        } else if let Some(finishing) = self.take_asked() {
            spawn_local(async move {
                if let Some(Finished {
                    note: Ok(note),
                    duration_ms,
                }) = finishing.await
                {
                    on_action.emit(Action::ParkStopped {
                        session,
                        chat_id,
                        note,
                        duration_ms,
                        reply_to_message_id,
                    });
                }
            });
        }
    }

    fn delete(&self) {
        if let Some(active) = self.take_live() {
            active.cancel();
        }
        // Being finished in the background; what it finishes into goes
        // nowhere.
        self.take_asked();
    }

    /// Delete at ten seconds or more: the recording STOPS first — its
    /// microphone goes off now — and the question is asked over it (S2.5).
    fn ask_delete(&self) {
        if let Some(active) = self.take_live() {
            let finishing = finish(active);
            spawn_local(finishing.clone().map(|_| ()));
            self.machine.borrow_mut().asked = Some(finishing);
        }
    }

    fn interrupt(self: &Rc<Self>) -> Vec<HoldEffect> {
        let recorded_ms = self.recorded();
        self.step(HoldEvent::Interruption {
            at_ms: at(),
            recorded_ms,
        })
    }

    /// The microphone stopped being the recording's (S4): another app took
    /// it, or it went away — which is said.
    fn lost(self: &Rc<Self>, lost: Lost) {
        let effects = self.interrupt();
        let stopped = effects
            .iter()
            .any(|effect| matches!(effect, HoldEffect::Park | HoldEffect::Delete));
        if lost == Lost::Ended && stopped {
            self.say(t("The recording stopped unexpectedly.").to_string());
        }
    }
}

/// The voice message, as the pane draws it and drives it.
#[derive(Clone)]
pub struct Voice {
    /// What the composer's slot is told (S1.3).
    pub recording: record::Recording,
    /// The microphone is being asked for.
    pub starting: bool,
    /// "Delete this recording?" is open.
    pub asking: bool,
    /// A note stopped by Send or into review is still being finished: the
    /// composer is busy until it lands (S1.2 **busy**).
    pub finishing: bool,
    /// Where the running recording's timer counts from (`recorder::now_ms`).
    pub started_ms: f64,
    /// How loud it is — None where the recorder has no tap.
    pub meter: Option<Meter>,
    /// What the announcement node says, numbered so that the same words
    /// said twice are said twice.
    pub announcement: (u32, String),
    /// Moves each time the cursor goes back into the box: a recording
    /// ended, or its question answered, by the person.
    pub focus: u32,
    /// Moves each time the five-minute limit ends one: the cursor goes back
    /// into the box only if the focus is still the composer's.
    pub refocus: u32,
    /// The slot activated (S1.3 rows 2, 3, 7–10) — a click, Enter or Space,
    /// a touch's `pointerup`.
    pub activate: Callback<()>,
    /// "Record Voice Message" from the paperclip or the microphone's menu;
    /// `true` beside words typed or items staged (row 3).
    pub record: Callback<bool>,
    /// The recording row's Stop, and Esc (S2.5).
    pub stop: Callback<()>,
    /// The recording row's Delete.
    pub delete: Callback<()>,
    /// "Delete this recording?" answered: `true` to delete.
    pub answer: Callback<bool>,
    /// Five minutes.
    pub cap: Callback<()>,
    /// 4:30: "30 seconds left", said.
    pub warn: Callback<()>,
    /// Three seconds in and nothing heard (`true`); sound at last (`false`).
    pub silence: Callback<bool>,
    /// Anything but the person stopped it: the tab hidden, a call, the pane
    /// gone (S4).
    pub interrupt: Callback<()>,
    /// The composer's own Send or Save emptied it (the activation guard).
    pub emptied: Callback<()>,
    /// The person changed the composer — typed, deleted, pasted, took a
    /// suggestion, staged or took off an item — which is never guarded
    /// (S1.1): the guard is lifted.
    pub changed: Callback<()>,
    /// Until when the slot ignores activation (S1.1), by `recorder::now_ms`.
    pub guard_until: Callback<(), u64>,
}

impl Voice {
    /// A recording runs, or the microphone is being asked for: what makes
    /// the call buttons wait and the other live regions quiet (S1.7, S6).
    pub fn active(&self) -> bool {
        self.recording != record::Recording::None || self.starting
    }
}

#[hook]
pub fn use_voice(
    setup: Setup,
    alive: Rc<RefCell<bool>>,
    replying: UseStateHandle<Option<i64>>,
    replying_now: Rc<RefCell<Option<i64>>>,
    notice: UseStateHandle<Option<String>>,
) -> Voice {
    let view = use_state(View::default);
    let announcement = use_state(|| (0u32, String::new()));
    let focus = use_state(|| 0u32);
    let refocus = use_state(|| 0u32);
    let finishing = use_state(|| 0u32);
    let driver: Rc<Driver> = {
        let view = view.clone();
        let announcement = announcement.clone();
        let focus = focus.clone();
        let refocus = refocus.clone();
        let finishing = finishing.clone();
        let first = setup.clone();
        let made = use_memo((), move |_| {
            Rc::new(Driver {
                machine: RefCell::new(Machine::default()),
                setup: RefCell::new(first),
                alive,
                replying,
                replying_now,
                notice,
                view,
                announcement,
                said: Cell::new(0),
                focus,
                focused: Cell::new(0),
                refocus,
                refocused: Cell::new(0),
                finishing,
                finishing_now: Cell::new(0),
            })
        });
        (*made).clone()
    };
    *driver.setup.borrow_mut() = setup;

    let with = |act: fn(&Rc<Driver>)| {
        let driver = driver.clone();
        Callback::from(move |_: ()| act(&driver))
    };
    let hold = view.hold;
    Voice {
        recording: hold.recording(),
        starting: matches!(hold.phase, Phase::AwaitingPermission { .. }),
        asking: matches!(hold.phase, Phase::AskingDelete { .. }),
        finishing: *finishing > 0,
        started_ms: view.started_ms,
        meter: view.meter.clone(),
        announcement: (*announcement).clone(),
        focus: *focus,
        refocus: *refocus,
        activate: with(|driver| {
            let (situation, recorded_ms) = (driver.situation(), driver.recorded());
            driver.step(HoldEvent::Activate {
                at_ms: at(),
                situation,
                recorded_ms,
            });
        }),
        record: {
            let driver = driver.clone();
            Callback::from(move |beside_draft: bool| {
                let (situation, recorded_ms) = (driver.situation(), driver.recorded());
                driver.step(HoldEvent::Record {
                    at_ms: at(),
                    beside_draft,
                    situation,
                    recorded_ms,
                });
            })
        },
        stop: with(|driver| {
            let recorded_ms = driver.recorded();
            driver.step(HoldEvent::Stop {
                at_ms: at(),
                recorded_ms,
            });
        }),
        delete: with(|driver| {
            let recorded_ms = driver.recorded();
            driver.step(HoldEvent::Delete {
                at_ms: at(),
                recorded_ms,
            });
        }),
        answer: {
            let driver = driver.clone();
            Callback::from(move |delete: bool| {
                driver.step(HoldEvent::Answer {
                    at_ms: at(),
                    delete,
                });
            })
        },
        cap: with(|driver| {
            driver.step(HoldEvent::Cap { at_ms: at() });
        }),
        warn: with(|driver| driver.announce(t("30 seconds left").to_string())),
        silence: {
            let driver = driver.clone();
            Callback::from(move |silent: bool| {
                if silent {
                    driver.say(t(SILENCE).to_string());
                    driver.announce(t(SILENCE).to_string());
                } else {
                    driver.quiet();
                }
            })
        },
        interrupt: with(|driver| {
            driver.interrupt();
        }),
        emptied: with(|driver| {
            driver.step(HoldEvent::Emptied { at_ms: at() });
        }),
        changed: with(|driver| {
            driver.step(HoldEvent::OtherAction { at_ms: at() });
        }),
        guard_until: {
            let driver = driver.clone();
            Callback::from(move |_: ()| driver.machine.borrow().hold.guard_until_ms)
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    /// Why the microphone is dimmed, in S1.3's order: a call before the
    /// attachment guard, the guard before a note that was not sent.
    #[wasm_bindgen_test]
    fn the_microphone_is_dimmed_by_a_call_then_the_guard_then_a_note_not_sent() {
        assert_eq!(blocked(false, false, false), None);
        assert_eq!(blocked(true, true, true), Some(Dimmed::Call));
        assert_eq!(blocked(false, true, true), Some(Dimmed::Busy));
        assert_eq!(blocked(false, false, true), Some(Dimmed::NotSent));
    }

    /// What is said is the shared rules' word, the length as m:ss.
    #[wasm_bindgen_test]
    fn what_is_said_is_the_shared_rules_word() {
        assert_eq!(said(Announcement::Recording), "Recording");
        assert_eq!(said(Announcement::VoiceMessageSent), "Voice message sent");
        assert_eq!(said(Announcement::RecordingDeleted), "Recording deleted");
        assert_eq!(
            said(Announcement::ReadyToReview {
                recorded_ms: 42_400
            }),
            "Ready to review, 0:42"
        );
        assert_eq!(
            said(Announcement::StoppedAtFiveMinutes),
            "Recording stopped at five minutes."
        );
    }
}
