//! `CapDraftTests`: the member limit's wait and write — the web client's `MemberLimit` tests,
//! ported.
//!
//! The C# test's pause is a `TaskCompletionSource` cancelled through its token, and its writes are
//! `TaskCompletionSource`s answered by hand; here a pause is a [`common::Gate`] whose future marks
//! itself cancelled when the draft drops it unfinished, a write a oneshot channel, and the draft's
//! spawned work runs on a `LocalPool` the test turns (`run_until_stalled`) where the C# continued
//! synchronously.

mod common;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use fc_core::protocol::{ApiError, error_codes};
use fc_logic::cap_draft::CapDraft;
use futures::channel::oneshot;
use futures::executor::LocalPool;
use futures::task::LocalSpawnExt;

use common::Gate;

/// One pause the draft asked for: its gate, and whether it was dropped before it ended.
struct Pause {
    gate: Gate,
    cancelled: Rc<Cell<bool>>,
}

/// Marks a pause cancelled when it is dropped before it ended.
struct Unfinished {
    done: bool,
    cancelled: Rc<Cell<bool>>,
}

impl Drop for Unfinished {
    fn drop(&mut self) {
        if !self.done {
            self.cancelled.set(true);
        }
    }
}

type Sent = Rc<RefCell<Vec<(Option<i32>, Option<oneshot::Sender<Option<ApiError>>>)>>>;

struct Rig {
    pool: LocalPool,
    pauses: Rc<RefCell<Vec<Pause>>>,
    sent: Sent,
    changes: Rc<Cell<i32>>,
    draft: Rc<CapDraft>,
}

impl Rig {
    fn new() -> Rig {
        let pool = LocalPool::new();
        let spawner = pool.spawner();
        let pauses: Rc<RefCell<Vec<Pause>>> = Rc::default();
        let sent: Sent = Rc::default();
        let changes = Rc::new(Cell::new(0));
        let (writes, waits) = (sent.clone(), pauses.clone());
        let draft = CapDraft::new(
            Rc::new(move |to| {
                let (answer, answered) = oneshot::channel();
                writes.borrow_mut().push((to, Some(answer)));
                Box::pin(async move { answered.await.unwrap_or(None) })
            }),
            Rc::new(move |_| {
                let gate = Gate::new();
                let cancelled = Rc::new(Cell::new(false));
                waits.borrow_mut().push(Pause {
                    gate: gate.clone(),
                    cancelled: cancelled.clone(),
                });
                let unfinished = Unfinished {
                    done: false,
                    cancelled,
                };
                Box::pin(async move {
                    // The whole guard, moved in: dropped with the future, ended or not.
                    let mut guard = unfinished;
                    gate.wait().await;
                    guard.done = true;
                })
            }),
            Rc::new(move |future| spawner.spawn_local(future).expect("the pool takes work")),
        );
        let counted = changes.clone();
        draft
            .changed()
            .subscribe(move |()| counted.set(counted.get() + 1));
        Rig {
            pool,
            pauses,
            sent,
            changes,
            draft,
        }
    }

    fn run(&mut self) {
        self.pool.run_until_stalled();
    }

    fn release(&mut self, pause: usize) {
        self.pauses.borrow()[pause].gate.open();
        self.run();
    }

    fn answer(&mut self, write: usize, error: Option<ApiError>) {
        let answer = self.sent.borrow_mut()[write]
            .1
            .take()
            .expect("answered once");
        answer.send(error).expect("the write waits");
        self.run();
    }

    fn sent_to(&self) -> Vec<Option<i32>> {
        self.sent.borrow().iter().map(|(to, _)| *to).collect()
    }
}

/// Two clicks before the next redraw are two steps — and one write, of where they ended.
#[test]
fn two_quick_steps_are_one_write_of_where_they_ended() {
    let mut rig = Rig::new();

    rig.draft.step(1, Some(3), 10);
    rig.draft.step(1, Some(3), 10);

    assert_eq!(rig.draft.drawn(Some(3)), Some(5));
    assert_eq!(rig.pauses.borrow().len(), 2);
    rig.run();
    assert!(
        rig.pauses.borrow()[0].cancelled.get(),
        "the first wait gave way to the second"
    );
    assert!(rig.sent.borrow().is_empty());
    rig.release(1);
    assert_eq!(rig.sent_to(), vec![Some(5)]);
    assert!(
        !rig.pauses.borrow()[1].cancelled.get(),
        "the second wait ended"
    );
}

/// An answer to an OLDER write must not wipe what was changed since.
#[test]
fn an_older_answer_does_not_undo_a_newer_change() {
    let mut rig = Rig::new();
    rig.draft.toggle(true, 3, 10);
    rig.run();
    rig.release(0);
    assert_eq!(rig.sent_to(), vec![Some(3)]);

    rig.draft.step(1, None, 10);
    rig.answer(0, None);

    assert_eq!(rig.draft.drawn(None), Some(4));
    rig.release(1);
    assert_eq!(rig.sent_to()[1], Some(4));
    rig.answer(1, None);
    // The last answer is the truth now: what is drawn is the family's own again.
    assert_eq!(rig.draft.drawn(Some(7)), Some(7));
}

/// Turned off, it stays off for the wait, and what goes is a null — which clears.
#[test]
fn turned_off_stays_off_while_its_write_waits() {
    let mut rig = Rig::new();

    rig.draft.toggle(false, 3, 10);

    assert_eq!(rig.draft.drawn(Some(4)), None);
    rig.run();
    assert!(rig.sent.borrow().is_empty());
    rig.release(0);
    assert_eq!(rig.sent_to(), vec![None]);
}

#[test]
fn turning_it_on_freezes_the_family_where_it_stands_inside_the_ceiling() {
    let rig = Rig::new();
    rig.draft.toggle(true, 12, 10);
    assert_eq!(rig.draft.drawn(None), Some(10));
    rig.draft.toggle(true, 3, 10);
    assert_eq!(rig.draft.drawn(None), Some(3));
    rig.draft.toggle(true, 0, 10);
    assert_eq!(rig.draft.drawn(None), Some(1));
}

#[test]
fn a_typed_value_is_held_inside_one_to_the_ceiling() {
    let rig = Rig::new();
    rig.draft.typed(0, 10);
    assert_eq!(rig.draft.drawn(None), Some(1));
    rig.draft.typed(99, 10);
    assert_eq!(rig.draft.drawn(None), Some(10));
}

#[test]
fn stepping_a_limit_that_is_off_does_nothing() {
    let mut rig = Rig::new();
    rig.draft.step(1, None, 10);
    rig.run();
    assert!(rig.pauses.borrow().is_empty());
    assert_eq!(rig.draft.drawn(None), None);
    assert_eq!(rig.changes.get(), 0);
}

/// Only the LAST write's failure is said: an older one was overtaken by what came after it.
#[test]
fn only_the_last_writes_failure_is_said() {
    let mut rig = Rig::new();
    rig.draft.typed(4, 10);
    rig.run();
    rig.release(0);
    rig.draft.typed(5, 10);
    rig.answer(0, Some(ApiError::transport("reset")));
    assert!(!rig.draft.failed());

    rig.release(1);
    let before = rig.changes.get();
    rig.answer(1, Some(ApiError::new(error_codes::VALIDATION, "x", 400)));
    assert!(rig.draft.failed());
    assert_eq!(rig.changes.get(), before + 1);
    assert_eq!(rig.draft.drawn(Some(6)), Some(6));

    rig.draft.typed(6, 10);
    assert!(!rig.draft.failed(), "a new change clears the old failure");
}
