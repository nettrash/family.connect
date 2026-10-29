//! The member limit while its write waits: what is drawn, and the ONE write a run of changes
//! becomes (`CapDraft.cs`; the web client's `MemberLimit`).
//!
//! A RUN OF CLICKS IS ONE WRITE, OF THE VALUE IT ENDED ON. Every change restarts a short wait, and
//! only the last one sends — stepped from what is drawn NOW, so two clicks before the next redraw
//! are two steps and not the same step twice.
//!
//! TURNED OFF STAYS OFF WHILE ITS NULL WAITS. The Mac's springs back on for the wait, and a second
//! click then cancels the removal.
//!
//! ONLY THE ANSWER TO THE LAST WRITE CLEARS WHAT IS DRAWN. An older answer arriving after a newer
//! change must not wipe the newer one, still on its way (the web client's tests, ported).
//!
//! On one thread: the C#'s `_ = SendAfterAsync(…)` is a future handed to `spawn`, and its
//! `CancellationTokenSource` an [`rt::Cancel`](Cancel) the next change raises — which drops the
//! wait it was racing. The C# caught an exception out of `send` as a transport error; a `send` here
//! answers its error instead.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use fc_core::protocol::ApiError;
use fc_core::rt::{Cancel, Event, LocalBoxFuture};
use fc_text::account;
use futures_util::future::{Either, select};

use crate::calls::Spawn;

/// The write — which should also bring the family back, so the value drawn after it is the
/// server's. `None` removes the limit.
pub type CapSend = Rc<dyn Fn(Option<i32>) -> LocalBoxFuture<'static, Option<ApiError>>>;

/// The pause: the clock's `sleep` in the app, a gate in a test. Dropping the future ends it.
pub type Pause = Rc<dyn Fn(Duration) -> LocalBoxFuture<'static, ()>>;

#[derive(Debug, Default)]
struct DraftState {
    draft: Option<i32>,
    touched: bool,
    generation: u64,
    waiting: Option<Cancel>,
}

/// The member limit's pending change (`CapDraft`). Hold it in the `Rc` [`CapDraft::new`] gives.
pub struct CapDraft {
    me: Weak<CapDraft>,
    send: CapSend,
    wait: Pause,
    spawn: Spawn,
    state: RefCell<DraftState>,
    failed: Cell<bool>,
    changed: Event<()>,
}

impl std::fmt::Debug for CapDraft {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CapDraft")
            .field("state", &self.state)
            .field("failed", &self.failed)
            .finish_non_exhaustive()
    }
}

impl CapDraft {
    /// How long a run of changes waits for the next before its one write.
    pub const DEBOUNCE: Duration = Duration::from_millis(600);

    /// `send` is the write, `wait` the pause, and `spawn` how the write is started (the app's
    /// `glib::MainContext::spawn_local`, a `LocalPool`'s spawner in a test).
    pub fn new(send: CapSend, wait: Pause, spawn: Spawn) -> Rc<CapDraft> {
        Rc::new_cyclic(|me| CapDraft {
            me: me.clone(),
            send,
            wait,
            spawn,
            state: RefCell::new(DraftState::default()),
            failed: Cell::new(false),
            changed: Event::new(),
        })
    }

    /// Whether the LAST write was refused or failed.
    pub fn failed(&self) -> bool {
        self.failed.get()
    }

    /// Something drawn changed.
    pub fn changed(&self) -> &Event<()> {
        &self.changed
    }

    /// What to draw: the change on its way, or the family's own cap (`held`) when nothing is.
    pub fn drawn(&self, held: Option<i32>) -> Option<i32> {
        let state = self.state.borrow();
        if state.touched { state.draft } else { held }
    }

    /// The switch: on proposes the family frozen where it stands, inside the ceiling
    /// (`fc_text::account::seed_cap`); off removes the limit.
    pub fn toggle(&self, on: bool, members: i32, ceiling: i32) {
        self.change(on.then(|| cap(account::seed_cap(i64::from(members), i64::from(ceiling)))));
    }

    /// One step of the stepper, from what is drawn now — nothing when the limit is off.
    pub fn step(&self, by: i32, held: Option<i32>, ceiling: i32) {
        if let Some(now) = self.drawn(held) {
            self.change(Some(cap(account::clamp_cap(
                i64::from(now) + i64::from(by),
                i64::from(ceiling),
            ))));
        }
    }

    /// A value typed into the field, held inside one to the ceiling.
    pub fn typed(&self, value: i32, ceiling: i32) {
        self.change(Some(cap(account::clamp_cap(
            i64::from(value),
            i64::from(ceiling),
        ))));
    }

    fn change(&self, to: Option<i32>) {
        let cancel = Cancel::new();
        let mine = {
            let mut state = self.state.borrow_mut();
            state.generation += 1;
            state.draft = to;
            state.touched = true;
            self.failed.set(false);
            if let Some(older) = state.waiting.replace(cancel.clone()) {
                older.cancel();
            }
            state.generation
        };
        // Asked for now, as the C# asked before its first await; raced against the next change.
        let pause = (self.wait)(Self::DEBOUNCE);
        if let Some(me) = self.me.upgrade() {
            (self.spawn)(Box::pin(async move {
                me.send_after(mine, to, pause, cancel).await
            }));
        }
        self.changed.emit(&());
    }

    async fn send_after(
        &self,
        mine: u64,
        to: Option<i32>,
        pause: LocalBoxFuture<'static, ()>,
        cancel: Cancel,
    ) {
        if cancel.is_cancelled() {
            return;
        }
        if let Either::Right(_) = select(pause, cancel.cancelled()).await {
            // A newer change took over the wait.
            return;
        }
        let error = (self.send)(to).await;
        {
            let mut state = self.state.borrow_mut();
            if mine != state.generation {
                return;
            }
            state.touched = false;
            self.failed.set(error.is_some());
        }
        self.changed.emit(&());
    }
}

/// A cap from `fc_text::account`, which counts in `i64`, back in the `i32` the wire carries: it is
/// held inside 1 and an `i32` ceiling, so it always fits.
fn cap(value: i64) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}
