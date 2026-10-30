//! Where the call actions go (`seams::routes::CallRoutes`, shell-design §2.2, §14.6) and the main
//! window's way back into a call (`WindowPart`, §3.1).
//!
//! - `app.answer-call(s)` — a notification's Answer: THIS call only, and only while it rings here
//!   (`call_view::applies`): the Call window comes up — GTK forwards the activation token the
//!   notification server put in the platform data, so on Wayland it may take the focus — and the
//!   engine answers. A stale notification's id, a call answered or declined meanwhile, one already
//!   over: nothing at all.
//! - `app.decline-call(s)` — a notification's Decline: the same rule; `call_end` `decline`,
//!   nothing brought forward.
//! - `app.return-to-call` — the Call window of the call under way, presented.
//! - **Return to Call** in the main window's content header, only while there is a call (the Mac's
//!   toolbar item: "without this a closed window was a call with no screen at all").

use std::rc::Rc;

use adw::prelude::*;
use fc_core::text::string_catalog::StringCatalog;

use super::{call_view, hub};
use crate::seams::actions;
use crate::seams::routes::{CallRoutes, WindowPart};
use crate::shell::Shell;
use crate::windows::main_window::MainWindow;

/// P6's [`CallRoutes`] (see the module's comment).
#[derive(Debug, Default)]
pub struct WindowCallRoutes;

impl CallRoutes for WindowCallRoutes {
    fn answer(&self, call_id: &str) {
        let shell = Shell::get();
        let Some(connection) = shell.connection() else {
            return;
        };
        if !call_view::applies(connection.calls.call().as_ref(), call_id) {
            crate::fc_info!("call {call_id}: answer ignored — it does not ring here");
            return;
        }
        crate::fc_info!("call {call_id}: answered from a route");
        hub::present();
        (shell.spawn)(Box::pin(connection.calls.answer()));
    }

    fn decline(&self, call_id: &str) {
        let shell = Shell::get();
        let Some(connection) = shell.connection() else {
            return;
        };
        if !call_view::applies(connection.calls.call().as_ref(), call_id) {
            crate::fc_info!("call {call_id}: decline ignored — it does not ring here");
            return;
        }
        crate::fc_info!("call {call_id}: declined from a route");
        connection.calls.decline();
    }

    fn return_to_call(&self) {
        hub::present();
    }
}

/// Return to Call in the main window (see the module's comment).
#[derive(Debug, Default)]
pub struct ReturnToCall;

impl WindowPart for ReturnToCall {
    fn attach(&self, window: &Rc<MainWindow>) {
        let shell = Shell::get();
        let say = &shell.say;
        let button = gtk::Button::builder()
            .icon_name("call-start-symbolic")
            .action_name(actions::RETURN_TO_CALL.detailed())
            .tooltip_text(say.get("Return to Call"))
            .visible(false)
            .build();
        button.update_property(&[gtk::accessible::Property::Label(say.get("Return to Call"))]);
        window.content_header().pack_end(&button);
        hub::add_return_button(button);
    }
}
