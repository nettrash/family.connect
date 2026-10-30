//! Where an action goes (FROZEN, W.3): the destinations S0's actions hand over to the package that
//! owns them, and the hook through which packages put their widgets into the main window's frame.
//!
//! Every method has a default that does nothing, so S0's stand-ins are the empty impls below and a
//! package overrides only what it owns. All of it runs on the GTK main thread; a route never
//! blocks — anything slow is spawned (`Shell::spawn`).

use std::rc::Rc;

use fc_core::rt::LocalBoxFuture;

use crate::windows::main_window::MainWindow;

/// P1 — the chats. The main window's `win.*` actions and `app.open-chat` arrive here.
pub trait ChatRoutes {
    /// Show this chat in the main window (the chat list selects it; the conversation opens; in a
    /// collapsed window the content pane comes forward). Called only once the gate is Chats, the
    /// recorder is closed, and the chat's family is the one shown (S0 switched it,
    /// `family_rules::family_to_open`). A chat the cache does not hold opens nothing. Opening a
    /// chat withdraws its notification (`Notifier::withdraw(NotificationRules::chat_tag(id))`).
    fn open_chat(&self, chat_id: i64) {
        let _ = chat_id;
    }

    /// `win.record-voice` (Ctrl+Shift+R) on the main window: start a voice recording in the open
    /// conversation, or — during one — stop it into review. Never sends.
    fn record_voice(&self) {}

    /// `win.record-video`: open the video message recorder over the open conversation.
    fn record_video(&self) {}

    /// `win.paste-attachment`: attach what the clipboard holds to the open conversation (§6.3).
    fn paste_attachment(&self) {}

    /// P2's inbox (§8.6), once the person chose a chat in "Send to" — that chat already opened with
    /// [`open_chat`](Self::open_chat): stage `files` in its composer exactly as a drop of them would
    /// (§6.4, `ComposerStaging::ingest`) — nothing is sent until Send. The files are the inbox's
    /// own copies; the future resolves once staging has READ them (it holds the bytes in memory),
    /// after which the inbox deletes the copies. S0's stand-in stages nothing. (Added by P2 — see
    /// its report.)
    fn stage_files(&self, chat_id: i64, files: Vec<gio::File>) -> LocalBoxFuture<'static, ()> {
        let _ = (chat_id, files);
        Box::pin(std::future::ready(()))
    }
}

/// P4 — the board, the family, groups and Settings.
pub trait FamilyRoutes {
    /// `app.open-board`: the board window of `family` (`None`: the family shown — S0 has already
    /// switched to `family` when it is one of the account's).
    fn open_board(&self, family: Option<i64>) {
        let _ = family;
    }

    /// `app.open-family`: the Family dialog over the main window (an owner's routes — join
    /// requests, reports — open it; a member who gets one stays put).
    fn open_family(&self) {}

    /// `app.show-settings` (Ctrl+,, the primary menu, the tray menu): the Settings window, or
    /// present it when open.
    fn show_settings(&self) {}

    /// `win.new-group` (the sidebar's "+", the primary menu): the New Group dialog.
    fn new_group(&self) {}

    /// The conversation header's "Group info" in a group chat (§3.1): the Group Info dialog of
    /// `chat_id` over the main window. (Added at integration.)
    fn group_info(&self, chat_id: i64) {
        let _ = chat_id;
    }
}

/// P6 — calls. S0's stand-in ([`EngineCallRoutes`](crate::shell::EngineCallRoutes)) answers and
/// declines on the connection's `CallEngine` with no window; P6's adds the Call window.
pub trait CallRoutes {
    /// `app.answer-call(s)`: answer THIS call — only if it is the one ringing (the engine applies a
    /// call action to the call it holds) — and bring the Call window up.
    fn answer(&self, call_id: &str) {
        let _ = call_id;
    }

    /// `app.decline-call(s)`: decline this call, nothing shown.
    fn decline(&self, call_id: &str) {
        let _ = call_id;
    }

    /// `app.return-to-call`: present the Call window of the call under way.
    fn return_to_call(&self) {}
}

/// P2 — files handed to the app (§8.6): `family-connect FILE…`, "Open With" (the `.desktop`
/// entry's `%F`, registered by P7 together with this), a second launch's files. S0's `open` handler
/// presents the main window and hands the files here; P2 copies them into
/// `Paths::inbox_dir` (the receiving process owns them, Windows' rule), asks "Send to" and stages
/// them — nothing is sent until Send.
pub trait InboxRoutes {
    fn receive(&self, files: &[gio::File]) {
        let _ = files;
    }
}

/// A package's part of the main window: called once, right after the frame is built and before the
/// window is first presented, in the order the parts were registered
/// (`Shell::add_window_part`). A part fills the slots the frame exposes ([`MainWindow`]'s
/// accessors): P1 the gate pages, the chat list, the family switcher, the banners and the
/// conversation; P4 the Board and Family buttons; P6 Return to Call, Call and Video Call; P2 its
/// hidden rich renderer in the overlay; P3 its recorder layer in the overlay.
pub trait WindowPart {
    fn attach(&self, window: &Rc<MainWindow>);
}

/// S0's stand-in for [`ChatRoutes`], [`FamilyRoutes`] and [`InboxRoutes`]: nothing happens.
#[derive(Debug, Default)]
pub struct NoRoutes;

impl ChatRoutes for NoRoutes {}
impl FamilyRoutes for NoRoutes {}
impl InboxRoutes for NoRoutes {}
