//! Every action of the app: its name, scope, target type, accelerator and owner (shell-design
//! §2.2, §3.4, W.3 — FROZEN).
//!
//! `app.*` actions live on the `adw::Application` (S0 registers them in `app.rs`); they are what a
//! notification, the tray menu, the global shortcut and a second launch reach. `win.*` actions
//! live on a window: S0 registers the main window's; P1 registers the same names on each
//! conversation window it opens, and the `win.message-*` family on every window that shows a
//! conversation. A target type is a GVariant type string; untrusted targets (a notification's)
//! are read through `fc_logic::notification_target::parse`.
//!
//! | Action | Target | Accelerator | Does | Handled by |
//! | --- | --- | --- | --- | --- |
//! | `app.toggle-window` | — | (the portal's Ctrl+Alt+Shift+F; `family-connect --toggle`) | `GlobalHotKeyRules::action` → present or hide the main window | S0 |
//! | `app.open-chat` | `x` chat id | — | present, switch to the chat's family, then [`ChatRoutes::open_chat`] — held until the gate is Chats and the recorder is closed | S0 → P1 |
//! | `app.open-board` | `x` family id (≤ 0: the family shown) | — | present, switch family, [`FamilyRoutes::open_board`] | S0 → P4 |
//! | `app.open-family` | — | — | present, [`FamilyRoutes::open_family`] | S0 → P4 |
//! | `app.answer-call` | `s` call id | — | [`CallRoutes::answer`] (that call only) | S0 → P6 |
//! | `app.decline-call` | `s` call id | — | [`CallRoutes::decline`] (that call only) | S0 → P6 |
//! | `app.return-to-call` | — | — | [`CallRoutes::return_to_call`] | S0 → P6 |
//! | `app.show-settings` | — | Ctrl+, | [`FamilyRoutes::show_settings`] | S0 → P4 |
//! | `app.shortcuts` | — | Ctrl+? | the Keyboard Shortcuts dialog | S0 |
//! | `app.about` | — | — | About Family Connect | S0 |
//! | `app.quit` | — | Ctrl+Q | §3.8: `Shell::on_quitting`, then hang up, close the socket, withdraw the icon, quit | S0 |
//! | `window.close` | — | Ctrl+W | GTK's own; the main window's close rule (`TrayRules::close_hides`) decides hide or quit | GTK, S0 |
//! | `win.refresh` | — | Ctrl+R | `LiveConnection::refresh` (a resync pass) | S0 |
//! | `win.new-group` | — | — | [`FamilyRoutes::new_group`] | S0 → P4 |
//! | `win.record-voice` | — | Ctrl+Shift+R | [`ChatRoutes::record_voice`] — never sends; during a recording it stops into review | S0 → P1 (+P3) |
//! | `win.record-video` | — | none: no shortcut ever opens the camera | [`ChatRoutes::record_video`] | S0 → P1 (+P3) |
//! | `win.toggle-fullscreen` | — | F11 | the window full screen and back (the Mac's View ▸ Enter Full Screen) | S0, P1 |
//! | `win.paste-attachment` | — | Ctrl+V | [`ChatRoutes::paste_attachment`]. DISABLED by default: P1 enables it (`MainWindow::set_paste_attachment_enabled`) only while a conversation is shown and its field is NOT focused, so the field's own Ctrl+V pastes words | S0 → P1 |
//! | `win.message-reply` | `x` message id | — | Reply | P1 |
//! | `win.message-react` | `(xs)` message id, emoji | — | toggle that reaction (held → DELETE, else PUT) | P1 |
//! | `win.message-show-text` | `x` | — | Show / Hide text (a recording's transcript) | P1 (+P3) |
//! | `win.message-playback-speed` | `x` | — | the next voice speed | P1 (+P3) |
//! | `win.message-save` | `x` | — | Save… | P1 (+P2) |
//! | `win.message-open-full-screen` | `x` | — | the viewer | P1 (+P2) |
//! | `win.message-copy` | `x` | — | Copy | P1 |
//! | `win.message-view-thread` | `x` | — | Reply in Thread / the thread | P1 |
//! | `win.message-edit` | `x` | — | Edit | P1 |
//! | `win.message-delete` | `x` pending row's token (< 0) | — | give up a REFUSED send (the protocol deletes no message): the target is the pending row's negative token; a positive id does nothing | P1 |
//! | `win.message-report` | `x` | — | Report… | P1 (+P4) |
//! | `win.message-block` | `x` | — | Block the sender (asks) | P1 |
//!
//! Escape is never an accelerator (§3.4): the composer, the recorder and dialogs take it in a
//! bubble-phase `gtk::EventControllerKey`. F10 opens the primary menu (libadwaita's, on the menu
//! button marked primary).
//!
//! [`ChatRoutes`]: super::routes::ChatRoutes
//! [`ChatRoutes::open_chat`]: super::routes::ChatRoutes::open_chat
//! [`ChatRoutes::record_voice`]: super::routes::ChatRoutes::record_voice
//! [`ChatRoutes::record_video`]: super::routes::ChatRoutes::record_video
//! [`ChatRoutes::paste_attachment`]: super::routes::ChatRoutes::paste_attachment
//! [`FamilyRoutes::open_board`]: super::routes::FamilyRoutes::open_board
//! [`FamilyRoutes::open_family`]: super::routes::FamilyRoutes::open_family
//! [`FamilyRoutes::show_settings`]: super::routes::FamilyRoutes::show_settings
//! [`FamilyRoutes::new_group`]: super::routes::FamilyRoutes::new_group
//! [`CallRoutes::answer`]: super::routes::CallRoutes::answer
//! [`CallRoutes::decline`]: super::routes::CallRoutes::decline
//! [`CallRoutes::return_to_call`]: super::routes::CallRoutes::return_to_call

use fc_logic::notification_target;

/// Where an action lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scope {
    /// On the application: `app.<name>`.
    App,
    /// On a window: `win.<name>`.
    Win,
}

impl Scope {
    pub fn prefix(self) -> &'static str {
        match self {
            Scope::App => "app",
            Scope::Win => "win",
        }
    }
}

/// One action of the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionSpec {
    pub scope: Scope,
    /// The name without its prefix (`open-chat`).
    pub name: &'static str,
    /// The target's GVariant type string, or `None` for no target.
    pub target: Option<&'static str>,
    /// GTK accelerators, the first the one shown.
    pub accels: &'static [&'static str],
}

impl ActionSpec {
    /// `app.open-chat`, `win.refresh`.
    pub fn detailed(&self) -> String {
        format!("{}.{}", self.scope.prefix(), self.name)
    }

    /// The target's type, for `gio::SimpleAction::new`.
    pub fn variant_type(&self) -> Option<&'static glib::VariantTy> {
        self.target
            .map(|t| glib::VariantTy::new(t).expect("a valid GVariant type string"))
    }
}

pub const TOGGLE_WINDOW: ActionSpec = app("toggle-window", None, &[]);
pub const OPEN_CHAT: ActionSpec = app(notification_target::OPEN_CHAT, Some("x"), &[]);
pub const OPEN_BOARD: ActionSpec = app(notification_target::OPEN_BOARD, Some("x"), &[]);
pub const OPEN_FAMILY: ActionSpec = app("open-family", None, &[]);
pub const ANSWER_CALL: ActionSpec = app(notification_target::ANSWER_CALL, Some("s"), &[]);
pub const DECLINE_CALL: ActionSpec = app(notification_target::DECLINE_CALL, Some("s"), &[]);
pub const RETURN_TO_CALL: ActionSpec = app("return-to-call", None, &[]);
pub const SHOW_SETTINGS: ActionSpec = app("show-settings", None, &["<Control>comma"]);
pub const SHORTCUTS: ActionSpec = app("shortcuts", None, &["<Control>question"]);
pub const ABOUT: ActionSpec = app("about", None, &[]);
pub const QUIT: ActionSpec = app("quit", None, &["<Control>q"]);

pub const REFRESH: ActionSpec = win("refresh", None, &["<Control>r"]);
pub const NEW_GROUP: ActionSpec = win("new-group", None, &[]);
pub const RECORD_VOICE: ActionSpec = win("record-voice", None, &["<Control><Shift>r"]);
pub const RECORD_VIDEO: ActionSpec = win("record-video", None, &[]);
pub const PASTE_ATTACHMENT: ActionSpec = win("paste-attachment", None, &["<Control>v"]);
pub const TOGGLE_FULLSCREEN: ActionSpec = win("toggle-fullscreen", None, &["F11"]);

pub const MESSAGE_REPLY: ActionSpec = win("message-reply", Some("x"), &[]);
pub const MESSAGE_REACT: ActionSpec = win("message-react", Some("(xs)"), &[]);
pub const MESSAGE_SHOW_TEXT: ActionSpec = win("message-show-text", Some("x"), &[]);
pub const MESSAGE_PLAYBACK_SPEED: ActionSpec = win("message-playback-speed", Some("x"), &[]);
pub const MESSAGE_SAVE: ActionSpec = win("message-save", Some("x"), &[]);
pub const MESSAGE_OPEN_FULL_SCREEN: ActionSpec = win("message-open-full-screen", Some("x"), &[]);
pub const MESSAGE_COPY: ActionSpec = win("message-copy", Some("x"), &[]);
pub const MESSAGE_VIEW_THREAD: ActionSpec = win("message-view-thread", Some("x"), &[]);
pub const MESSAGE_EDIT: ActionSpec = win("message-edit", Some("x"), &[]);
pub const MESSAGE_DELETE: ActionSpec = win("message-delete", Some("x"), &[]);
pub const MESSAGE_REPORT: ActionSpec = win("message-report", Some("x"), &[]);
pub const MESSAGE_BLOCK: ActionSpec = win("message-block", Some("x"), &[]);
pub const MESSAGE_REACTORS: ActionSpec = win("message-reactors", Some("x"), &[]);

/// GTK's own window action Ctrl+W is bound to.
pub const WINDOW_CLOSE: &str = "window.close";
/// Its accelerator.
pub const WINDOW_CLOSE_ACCELS: &[&str] = &["<Control>w"];

/// The application's actions, in the order S0 registers them.
pub const APP_ACTIONS: [ActionSpec; 11] = [
    TOGGLE_WINDOW,
    OPEN_CHAT,
    OPEN_BOARD,
    OPEN_FAMILY,
    ANSWER_CALL,
    DECLINE_CALL,
    RETURN_TO_CALL,
    SHOW_SETTINGS,
    SHORTCUTS,
    ABOUT,
    QUIT,
];

/// The main window's own actions (S0 registers them; P1 the same on a conversation window).
pub const WINDOW_ACTIONS: [ActionSpec; 6] = [
    REFRESH,
    NEW_GROUP,
    RECORD_VOICE,
    RECORD_VIDEO,
    PASTE_ATTACHMENT,
    TOGGLE_FULLSCREEN,
];

/// The message menu's actions (P1 registers them on every window that shows a conversation).
pub const MESSAGE_ACTIONS: [ActionSpec; 13] = [
    MESSAGE_REPLY,
    MESSAGE_REACT,
    MESSAGE_SHOW_TEXT,
    MESSAGE_PLAYBACK_SPEED,
    MESSAGE_SAVE,
    MESSAGE_OPEN_FULL_SCREEN,
    MESSAGE_COPY,
    MESSAGE_VIEW_THREAD,
    MESSAGE_EDIT,
    MESSAGE_DELETE,
    MESSAGE_REPORT,
    MESSAGE_BLOCK,
    MESSAGE_REACTORS,
];

const fn app(
    name: &'static str,
    target: Option<&'static str>,
    accels: &'static [&'static str],
) -> ActionSpec {
    ActionSpec {
        scope: Scope::App,
        name,
        target,
        accels,
    }
}

const fn win(
    name: &'static str,
    target: Option<&'static str>,
    accels: &'static [&'static str],
) -> ActionSpec {
    ActionSpec {
        scope: Scope::Win,
        name,
        target,
        accels,
    }
}

/// Every action with an accelerator, as `(detailed name, accels)` — what `startup` hands to
/// `set_accels_for_action`, and what the shell self-test checks.
pub fn accelerators() -> Vec<(String, &'static [&'static str])> {
    let mut all: Vec<(String, &'static [&'static str])> = APP_ACTIONS
        .iter()
        .chain(WINDOW_ACTIONS.iter())
        .filter(|spec| !spec.accels.is_empty())
        .map(|spec| (spec.detailed(), spec.accels))
        .collect();
    all.push((WINDOW_CLOSE.to_string(), WINDOW_CLOSE_ACCELS));
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_notification_actions_are_the_ones_fc_logic_writes() {
        // `toast_arguments::for_toast` and `notification_target::parse` speak these names.
        assert_eq!(OPEN_CHAT.detailed(), "app.open-chat");
        assert_eq!(OPEN_BOARD.detailed(), "app.open-board");
        assert_eq!(ANSWER_CALL.detailed(), "app.answer-call");
        assert_eq!(DECLINE_CALL.detailed(), "app.decline-call");
        assert_eq!(OPEN_CHAT.target, Some("x"));
        assert_eq!(OPEN_BOARD.target, Some("x"));
        assert_eq!(ANSWER_CALL.target, Some("s"));
        assert_eq!(DECLINE_CALL.target, Some("s"));
        // The global shortcut's id is the action's name.
        assert_eq!(
            TOGGLE_WINDOW.name,
            fc_logic::global_hot_key_rules::SHORTCUT_ID
        );
    }

    #[test]
    fn every_target_type_is_a_valid_gvariant_type_and_names_are_unique() {
        let all: Vec<ActionSpec> = APP_ACTIONS
            .iter()
            .chain(WINDOW_ACTIONS.iter())
            .chain(MESSAGE_ACTIONS.iter())
            .copied()
            .collect();
        for spec in &all {
            if let Some(target) = spec.target {
                assert!(glib::VariantTy::new(target).is_ok(), "{target}");
            }
            assert!(gio::Action::name_is_valid(spec.name), "{}", spec.name);
        }
        let mut names: Vec<String> = all.iter().map(ActionSpec::detailed).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), all.len());
    }

    #[test]
    fn the_design_chords_are_bound_and_no_accelerator_is_shared() {
        let accels = accelerators();
        let find = |name: &str| {
            accels
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, a)| a.to_vec())
        };
        assert_eq!(find("app.show-settings"), Some(vec!["<Control>comma"]));
        assert_eq!(find("win.refresh"), Some(vec!["<Control>r"]));
        assert_eq!(find("win.record-voice"), Some(vec!["<Control><Shift>r"]));
        assert_eq!(find("window.close"), Some(vec!["<Control>w"]));
        assert_eq!(find("app.quit"), Some(vec!["<Control>q"]));
        assert_eq!(find("app.shortcuts"), Some(vec!["<Control>question"]));
        // No shortcut ever opens the camera, and Escape is never an accelerator.
        assert_eq!(find("win.record-video"), None);
        let mut chords: Vec<&str> = accels.iter().flat_map(|(_, a)| a.iter().copied()).collect();
        assert!(chords.iter().all(|c| !c.contains("Escape")));
        let count = chords.len();
        chords.sort_unstable();
        chords.dedup();
        assert_eq!(chords.len(), count);
        // That GTK parses each chord is the shell self-test's check (it needs GTK initialised).
    }
}
