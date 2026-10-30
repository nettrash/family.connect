//! **P4 — board, family, settings** (shell-design §W.2): where the package plugs in.
//!
//! [`install`] (one line in `app::install_packages`) puts [`P4Routes`] into `Shell::family_routes`
//! — `app.open-board`, `app.open-family`, `app.show-settings` and `win.new-group` land there — adds
//! the Board and Family buttons to the main window's content header (a [`WindowPart`], the board's
//! unread badge on its button), and registers the package's CSS.
//!
//! | Module | What |
//! | --- | --- |
//! | [`rows`] | what each Settings row shows (pure, tested) |
//! | [`ui`] | the alert, dialog and row helpers every P4 surface draws with |
//! | [`dialogs`] | Change Password…, Delete Account…, New Family…, Join Another Family…, Leave Family |
//! | `windows::settings_window` | the Settings window (§13) |
//! | `family` | the Family dialog and the owner's console (§12), the birthday editor, the group dialogs |
//! | `board` | the board's wall, notes, note sheet and calendar file (§11); `windows::board_window` |

pub mod dialogs;
pub mod rows;
pub mod ui;

use std::rc::Rc;

use adw::prelude::*;
use fc_core::text::string_catalog::StringCatalog;

use crate::seams::routes::{FamilyRoutes, WindowPart};
use crate::shell::Shell;
use crate::windows::main_window::MainWindow;

/// Plugs P4 in (W.3): the routes, the header buttons, the CSS.
pub fn install(shell: &Rc<Shell>) {
    shell.family_routes.install(Rc::new(P4Routes));
    shell.add_window_part(Rc::new(HeaderButtons));
    crate::theme::register(|palette| crate::board::css(palette.dark));
}

/// Where `FamilyRoutes` go (seams::routes).
#[derive(Debug, Default)]
pub struct P4Routes;

impl FamilyRoutes for P4Routes {
    fn open_board(&self, family: Option<i64>) {
        crate::windows::board_window::open(family);
    }

    fn open_family(&self) {
        crate::family::open();
    }

    fn show_settings(&self) {
        crate::windows::settings_window::show();
    }

    fn new_group(&self) {
        crate::family::groups::new_group();
    }

    /// Over the window the button was pressed in (the main window, or a conversation window).
    fn group_info(&self, chat_id: i64) {
        let shell = Shell::get();
        let parent = shell.app.active_window().or_else(|| {
            shell
                .main_window()
                .map(|main| main.window().clone().upcast::<gtk::Window>())
        });
        if let Some(parent) = parent {
            crate::family::groups::group_info(&parent, chat_id);
        }
    }
}

/// The content header's Board (with its badge) and Family buttons (§3.1).
struct HeaderButtons;

impl WindowPart for HeaderButtons {
    fn attach(&self, window: &Rc<MainWindow>) {
        let shell = Shell::get();
        let say = &shell.say;
        let header = window.content_header();

        // Named for what it holds (the Mac's toolbar help), spoken as "Family".
        let family = gtk::Button::builder()
            .icon_name("system-users-symbolic")
            .tooltip_text(say.get("Members, invites and direct chats"))
            .action_name(crate::seams::actions::OPEN_FAMILY.detailed())
            .build();
        family.update_property(&[gtk::accessible::Property::Label(say.get("Family"))]);

        // `app.open-board(0)`: 0 is "the family shown" (seams::actions).
        let board = gtk::Button::builder()
            .icon_name("view-grid-symbolic")
            .tooltip_text(say.get("Board"))
            .action_name(crate::seams::actions::OPEN_BOARD.detailed())
            .action_target(&0i64.to_variant())
            .build();
        board.update_property(&[gtk::accessible::Property::Label(say.get("Board"))]);
        let badge = gtk::Label::new(None);
        badge.add_css_class("fc-badge");
        badge.set_halign(gtk::Align::End);
        badge.set_valign(gtk::Align::Start);
        badge.set_can_target(false);
        badge.set_visible(false);
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&board));
        overlay.add_overlay(&badge);

        // pack_end puts the first packed at the far end: Family rightmost, Board beside it.
        header.pack_end(&family);
        header.pack_end(&overlay);
        crate::board::follow_badge(badge);
    }
}
