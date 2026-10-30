//! Subscriptions to fc-core/fc-logic `Event`s that end with their owner: a chat list wired to one
//! connection, a conversation page to one chat. Dropping (or [`Subscriptions::clear`]) takes every
//! handler off again, so a page that went away is never called by the router.

use std::cell::RefCell;
use std::rc::Rc;

use fc_core::rt::Event;

/// Handlers put on events, taken off together.
#[derive(Default)]
pub struct Subscriptions(RefCell<Vec<Box<dyn FnOnce()>>>);

impl std::fmt::Debug for Subscriptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Subscriptions")
            .field(&self.0.borrow().len())
            .finish()
    }
}

impl Subscriptions {
    pub fn new() -> Subscriptions {
        Subscriptions::default()
    }

    /// Subscribes `handler` to the event `event(owner)` names, for as long as these subscriptions
    /// last (the owner is held weakly: an owner gone is a handler gone).
    pub fn on<O: 'static, A: ?Sized + 'static>(
        &self,
        owner: &Rc<O>,
        event: fn(&O) -> &Event<A>,
        handler: impl Fn(&A) + 'static,
    ) {
        let id = event(owner).subscribe(handler);
        let owner = Rc::downgrade(owner);
        self.0.borrow_mut().push(Box::new(move || {
            if let Some(owner) = owner.upgrade() {
                event(&owner).unsubscribe(id);
            }
        }));
    }

    /// Anything else to undo with them (a timer, a signal handler).
    pub fn defer(&self, undo: impl FnOnce() + 'static) {
        self.0.borrow_mut().push(Box::new(undo));
    }

    /// Takes every handler off now.
    pub fn clear(&self) {
        let undo: Vec<Box<dyn FnOnce()>> = self.0.borrow_mut().drain(..).collect();
        for undo in undo {
            undo();
        }
    }
}

impl Drop for Subscriptions {
    fn drop(&mut self) {
        self.clear();
    }
}
