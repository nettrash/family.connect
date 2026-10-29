//! A call ringing here, told to the desktop (shell-design §14.4, §14.6, §15): the notification
//! with Decline and Answer — P5's [`Notifier::ring`](crate::seams::desktop::Notifier::ring), the
//! reliable door on Wayland, where the Call window may not take the focus. The notifier also
//! raises the top-bar icon's attention and the dock's urgency while any call rings, and calms them
//! in `stop_ringing` (P5's contract: nobody else sets them for a call).
//! HOW each is shown is P5's; WHEN is decided here, from the engine's state: raised when a call
//! starts ringing, taken back the moment it stops ringing for ANY reason — answered here or on
//! another device, declined, rung out, ended by the caller, the connection gone, the app quitting.
//! Who calls, never anything else (`call_view::ring_words`).

use std::cell::RefCell;

use fc_logic::calls::CallState;

use super::call_view;
use crate::shell::Shell;

/// The ring's notice (see the module's comment).
#[derive(Debug, Default)]
pub struct RingNotice {
    /// The call the desktop was told rings.
    ringing: RefCell<Option<String>>,
}

impl RingNotice {
    /// Brings the desktop in line with `call` (the engine's, `None` for no call), `name` the
    /// person's.
    pub fn update(&self, shell: &Shell, call: Option<&CallState>, name: &str) {
        let wanted = call_view::ringing(call).map(str::to_string);
        let held = self.ringing.borrow().clone();
        if wanted == held {
            return;
        }
        if let Some(old) = &held {
            shell.notifier.get().stop_ringing(old);
            crate::fc_info!("call {old}: no longer ringing; its notice withdrawn");
        }
        if let (Some(id), Some(call)) = (&wanted, call) {
            let (title, body) = call_view::ring_words(call, name, &*shell.say);
            shell.notifier.get().ring(id, &title, &body);
            crate::fc_info!("call {id}: ringing; its notice raised");
        }
        *self.ringing.borrow_mut() = wanted;
    }

    /// Nothing rings any more (the connection went, the app quits).
    pub fn stop(&self, shell: &Shell) {
        self.update(shell, None, "");
    }
}
