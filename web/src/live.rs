//! The app's state, and the one way to change it.
//!
//! Every part of the client reads and writes the SAME `AppState` through a
//! `Live` handle: the view when it draws, the callbacks a person triggers,
//! and the tasks that outlive the render that started them — the socket
//! loop, the outbox, a fetch in flight.

use std::cell::RefCell;
use std::rc::Rc;

use crate::store::Store;

/// Everything the app knows, right now.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct AppState {
    /// Which sign-in this is. Every sign-out moves it on, so that a task
    /// started for one session cannot write into the next — see
    /// `Live::update`.
    pub session: u64,
    pub token: Option<String>,
    pub store: Store,
    pub open_chat: Option<i64>,
    /// Whether the socket is up — through its handshake, not merely
    /// dialled. A browser's dial "succeeds" before the server has answered
    /// at all; the handshake is where a bad token is refused.
    pub connected: bool,
    /// Something that went wrong, for the person to read.
    pub failure: Option<String>,
    /// Something that went RIGHT and is worth a word — "Reported to the
    /// family owner." — which is not an error and is not drawn as one.
    pub notice: Option<String>,
    /// Whether the reader of the open chat is at its newest message, as its
    /// view last said. A chat is READ only while this holds (see
    /// `sync::reading`), and opening a chat sets it false until the view
    /// has decided where the chat opens and looked.
    pub at_newest: bool,
    /// The open chat's unread state once its messages are in — None while
    /// they are still loading. Taken BEFORE anything may read the chat,
    /// because the "N new messages" divider is decided from it: a read
    /// reported first leaves nothing unread to draw a divider over.
    pub opening: Option<Opening>,
}

/// A chat, as it stood when it had loaded and before it was read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Opening {
    pub chat_id: i64,
    pub unread_count: i64,
    pub last_read_message_id: i64,
}

/// A handle on the one `AppState`.
///
/// NOT `use_state`, and that is the whole reason this type exists. A
/// `UseStateHandle` derefs to the value AS OF THE RENDER that made it, so a
/// task that outlives that render reads a snapshot, changes it, and sets it
/// back — undoing everything that happened in between. This client's first
/// version did exactly that: the chat list landing after `/me` put
/// `my_user_id` back to 0, so every message of your own drew as somebody
/// else's, and the socket loop rebuilt the whole store from the empty one it
/// was started with on every frame it received.
#[derive(Clone)]
pub struct Live {
    state: Rc<RefCell<AppState>>,
    redraw: Rc<dyn Fn()>,
}

impl Live {
    /// `redraw` is what draws the state again after a change — the app's
    /// `use_force_update`, or a counter in a test.
    pub fn new(state: AppState, redraw: Rc<dyn Fn()>) -> Self {
        Live {
            state: Rc::new(RefCell::new(state)),
            redraw,
        }
    }

    /// Look at the state as it is NOW.
    pub fn read<R>(&self, look: impl FnOnce(&AppState) -> R) -> R {
        look(&self.state.borrow())
    }

    pub fn session(&self) -> u64 {
        self.state.borrow().session
    }

    /// Whether `session` is still the session.
    pub fn is_live(&self, session: u64) -> bool {
        self.session() == session
    }

    /// Change the state and draw it — IF `session` is still the session.
    ///
    /// An answer that arrives after a sign-out belongs to nobody. Written
    /// anyway, it would land in whoever signs in next: one person's chat
    /// list in another's tab. Dropped, it is simply gone. Returns what
    /// `change` returned, or None when the change was dropped.
    pub fn update<R>(&self, session: u64, change: impl FnOnce(&mut AppState) -> R) -> Option<R> {
        let result = {
            let mut state = self.state.borrow_mut();
            if state.session != session {
                return None;
            }
            change(&mut state)
        };
        // After the borrow is released: drawing reads the state.
        (self.redraw)();
        Some(result)
    }

    /// A change a person made, which is by definition to the session in
    /// front of them.
    pub fn now<R>(&self, change: impl FnOnce(&mut AppState) -> R) -> R {
        let session = self.session();
        self.update(session, change)
            .expect("the current session is live")
    }

    /// Sign out: everything goes, and the session moves on so that nothing
    /// started for the old one can write again.
    pub fn end_session(&self) {
        {
            let mut state = self.state.borrow_mut();
            let session = state.session + 1;
            *state = AppState {
                session,
                ..AppState::default()
            };
        }
        (self.redraw)();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use wasm_bindgen_test::*;

    fn live() -> (Live, Rc<Cell<u32>>) {
        let draws = Rc::new(Cell::new(0));
        let counter = draws.clone();
        let live = Live::new(
            AppState::default(),
            Rc::new(move || counter.set(counter.get() + 1)),
        );
        (live, draws)
    }

    /// THE BUG THIS TYPE EXISTS FOR. Two tasks, each holding the handle
    /// from before the other's write, each changing a DIFFERENT part. Both
    /// changes stand — the second does not put back the first one's world.
    #[wasm_bindgen_test]
    fn a_task_holding_an_old_handle_does_not_undo_another_tasks_write() {
        let (live, _) = live();
        let session = live.session();
        let me_task = live.clone();
        let chats_task = live.clone();

        me_task.update(session, |state| state.store.my_user_id = 7);
        chats_task.update(session, |state| state.store.names.insert(9, "Anna".into()));

        live.read(|state| {
            assert_eq!(
                state.store.my_user_id, 7,
                "the first write survived the second"
            );
            assert_eq!(state.store.names.get(&9).map(String::as_str), Some("Anna"));
        });
    }

    #[wasm_bindgen_test]
    fn every_change_is_drawn() {
        let (live, draws) = live();
        let session = live.session();
        live.update(session, |state| state.connected = true);
        live.now(|state| state.open_chat = Some(42));
        assert_eq!(draws.get(), 2);
    }

    /// An answer from a session that has ended is DROPPED — not written into
    /// whoever signs in next, and not drawn.
    #[wasm_bindgen_test]
    fn an_answer_for_a_signed_out_session_is_dropped() {
        let (live, draws) = live();
        let old = live.session();
        live.now(|state| state.token = Some("first".into()));
        live.end_session();
        assert!(!live.is_live(old));
        let before = draws.get();

        let written = live.update(old, |state| state.store.my_user_id = 7);

        assert_eq!(written, None);
        assert_eq!(draws.get(), before, "nothing to draw");
        live.read(|state| {
            assert_eq!(state.store.my_user_id, 0);
            assert_eq!(state.token, None, "signing out forgot the token");
        });
    }

    #[wasm_bindgen_test]
    fn signing_out_forgets_everything_but_moves_the_session_on() {
        let (live, _) = live();
        live.now(|state| {
            state.token = Some("t".into());
            state.store.my_user_id = 7;
            state.open_chat = Some(42);
            state.connected = true;
        });
        let session = live.session();
        live.end_session();
        live.read(|state| {
            assert_eq!(state.session, session + 1);
            assert_eq!(
                *state,
                AppState {
                    session: session + 1,
                    ..AppState::default()
                }
            );
        });
    }
}
