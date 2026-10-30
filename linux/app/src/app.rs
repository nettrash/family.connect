//! The `adw::Application` and its signals (shell-design §2.1, §2.2, §3.7).
//!
//! `me.nettrash.FamilyConnect`, `HANDLES_OPEN`, single instance over D-Bus (`NON_UNIQUE` for a
//! self-test), `register-session` on. The command line:
//!
//! | Option | Does |
//! | --- | --- |
//! | `--version` | prints `family-connect 1.3.0` and exits — before the bus or a display (the package checks run it in containers that have neither) |
//! | `--background` | the autostart entry's launch: the window starts hidden in the top bar when `StartupSetting::starts_hidden(true, an icon exists)`; a second `--background` launch while running does nothing |
//! | `--toggle` | hands `app.toggle-window` to the running instance and exits; a first launch with it is an ordinary launch |
//! | `FILE…` | `open`: the window comes forward and the files go to the inbox (P2, §8.6) |
//! | `--selftest OUT [--only g,…] [--no-input]` | the §27 self-test (feature `selftest`; `main.rs` takes it before GApplication sees it) |
//!
//! **Activation routes** (§2.2), all ending in the ONE main window: `activate` (launcher, dock, a
//! second launch) presents it — unless this is the first activation of a `--background` launch
//! and an icon exists, with the Mac's backstop: no icon two seconds later and the window is shown
//! after all; `app.toggle-window`, `app.open-chat(x)`, `app.open-board(x)`, `app.open-family`,
//! `app.answer-call(s)`, `app.decline-call(s)`, `app.return-to-call`, `app.show-settings`,
//! `app.shortcuts`, `app.about`, `app.quit` (`seams::actions`).
//!
//! **Start-up order** (§3.7): paths → settings → catalogue (`Shell::start`) → GStreamer → the
//! packages' `install` → CSS → actions and accelerators → the main window built and every window
//! part attached, NOT presented → the hot key bound, the flush timer, the network monitor, the
//! temporaries swept → the saved server opened (the token read, then `GET /me`).

use std::cell::Cell;
use std::ffi::OsString;
use std::ops::ControlFlow;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use fc_core::protocol::send_rules::FlushTrigger;
use fc_logic::notification_target::{self, NotificationTarget, TargetValue};
use fc_logic::tray_rules;

use crate::seams::actions::{self, ActionSpec};
use crate::services::files;
use crate::services::paths;
use crate::services::settings_store::keys;
use crate::shell::{BRAND, Shell};
use crate::theme;
use crate::windows::main_window::{MainWindow, Route};

/// The application id: also the desktop entry's, the icon's, the D-Bus name and the Wayland
/// `app_id` that ties windows to `me.nettrash.FamilyConnect.desktop` in the dock.
pub const APP_ID: &str = "me.nettrash.FamilyConnect";
/// What `--version` prints: the executable's name and the workspace's version — the packages'
/// checks (`scripts/package.sh`) compare it exactly.
pub const VERSION_LINE: &str = concat!(env!("CARGO_PKG_NAME"), " ", env!("CARGO_PKG_VERSION"));
/// How often the outbox is asked to flush on its own (Windows' 5 s timer; `SendRules` decides what
/// is due).
const FLUSH_EVERY: Duration = Duration::from_secs(5);

thread_local! {
    /// `--background` was given (read in `startup`).
    static LAUNCHED_AT_LOGIN: Cell<bool> = const { Cell::new(false) };
    /// The first `activate` has happened.
    static ACTIVATED: Cell<bool> = const { Cell::new(false) };
}

/// How this process was started.
pub enum Launch {
    /// A desktop launch, a file manager's open, a second launch handing over.
    Normal,
    /// `family-connect --selftest OUT …` (feature `selftest`): a private, `NON_UNIQUE` process
    /// whose runner drives the real window and exits with the report's verdict.
    #[cfg(feature = "selftest")]
    SelfTest(crate::selftest::Options),
}

/// Builds the application, connects its signals and runs the main loop. `args` are the process's
/// arguments with the self-test's own already taken out.
pub fn run(args: &[OsString], launch: Launch) -> glib::ExitCode {
    #[cfg(feature = "selftest")]
    let selftest = match launch {
        Launch::Normal => None,
        Launch::SelfTest(options) => Some(options),
    };
    #[cfg(not(feature = "selftest"))]
    let Launch::Normal = launch;

    glib::set_application_name(BRAND);
    let flags = gio::ApplicationFlags::HANDLES_OPEN;
    // Never the primary instance, never handing its arguments to a running app.
    #[cfg(feature = "selftest")]
    let flags = if selftest.is_some() {
        flags | gio::ApplicationFlags::NON_UNIQUE
    } else {
        flags
    };
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(flags)
        // `query-end` at logout; `screensaver-active` for P3's session watch.
        .register_session(true)
        .build();
    let option = |name: &str, help: &str| {
        app.add_main_option(
            name,
            glib::Char::from(0),
            glib::OptionFlags::NONE,
            glib::OptionArg::None,
            help,
            None,
        );
    };
    option("background", "Start with the window hidden (used at login)");
    option(
        "toggle",
        "Show the window, or put it away if it is in front",
    );
    option("version", "Print the version and exit");
    app.connect_handle_local_options(handle_local_options);

    #[cfg(feature = "selftest")]
    let is_selftest = selftest.is_some();
    #[cfg(feature = "selftest")]
    let selftest = std::cell::RefCell::new(selftest);
    app.connect_startup(move |app| {
        let shell = startup(app);
        #[cfg(feature = "selftest")]
        if let Some(options) = selftest.borrow_mut().take() {
            crate::selftest::start(&shell, options);
        }
        #[cfg(not(feature = "selftest"))]
        let _ = shell;
    });
    app.connect_activate(|_| activate(&Shell::get()));
    app.connect_open(|_, files, _hint| {
        let shell = Shell::get();
        if let Some(window) = shell.main_window() {
            window.present();
        }
        crate::fc_info!("open: {} file(s) handed over", files.len());
        shell.inbox.get().receive(files);
    });
    app.connect_query_end(|_| Shell::get().query_end());
    app.connect_shutdown(|_| Shell::stop());

    let code = app.run_with_args_os(args);
    #[cfg(feature = "selftest")]
    if is_selftest {
        return crate::selftest::exit_code();
    }
    code
}

/// `--version`, `--background` and `--toggle` — answered before the window exists.
fn handle_local_options(
    app: &adw::Application,
    options: &glib::VariantDict,
) -> ControlFlow<glib::ExitCode> {
    if options.contains("version") {
        println!("{VERSION_LINE}");
        return ControlFlow::Break(glib::ExitCode::SUCCESS);
    }
    let background = options.contains("background");
    let toggle = options.contains("toggle");
    if background || toggle {
        // Registering tells us whether another instance owns the name. Without a session bus
        // every launch is its own primary (Docker only).
        if let Err(error) = app.register(None::<&gio::Cancellable>) {
            crate::fc_warn!(
                "could not register on the session bus ({})",
                error.message()
            );
        }
        if app.is_remote() {
            if toggle {
                app.activate_action(actions::TOGGLE_WINDOW.name, None);
            }
            // A second `--background` (a login while running) has nothing to do.
            return ControlFlow::Break(glib::ExitCode::SUCCESS);
        }
        LAUNCHED_AT_LOGIN.with(|flag| flag.set(background));
    }
    ControlFlow::Continue(())
}

/// `startup` (§3.7). Answers the shell for the self-test's runner.
fn startup(app: &adw::Application) -> Rc<Shell> {
    let shell = Shell::start(app);
    shell
        .launched_at_login
        .set(LAUNCHED_AT_LOGIN.with(Cell::get));
    // GStreamer carries calls and media. Started here, not in `main`, so `--version` never scans
    // the plugin registry.
    if let Err(error) = gst::init() {
        crate::fc_warn!("GStreamer could not start ({error})");
    }
    gtk::Window::set_default_icon_name(APP_ID);
    add_icons_beside_executable();
    install_packages(&shell);
    theme::install();
    add_app_actions(&shell);
    for (action, accels) in actions::accelerators() {
        app.set_accels_for_action(&action, accels);
    }
    let window = MainWindow::build(&shell);
    shell.set_main_window(Rc::clone(&window));
    for part in shell.window_parts() {
        part.attach(&window);
    }
    follow_settings(&shell);
    keep_flushing(&shell);
    files::wipe_dir(&shell.paths.tmp_dir());
    if let Some(server) = shell.saved_server() {
        shell.open_server(server);
    }
    shell
}

/// WHERE EACH PACKAGE PLUGS IN (W.3, `seams`): one line per package, added at integration — its
/// `install` fills the shell's slots, registers its window parts, its CSS and its self-test group.
/// Runs before the main window is built and before any server is opened.
fn install_packages(shell: &Rc<Shell>) {
    crate::windows::gate::install(shell); // P1 — chats, conversation, composer, gate pages
    crate::attachments::install(shell); // P2 — tiles, viewer, stickers, rich, inbox
    crate::media::install(shell); // P3 — players, camera, recording, circles
    crate::settings::install(shell); // P4 — board, family, settings, groups
    crate::desktop::install(shell); // P5 — notifier, tray, badge, hot key, autostart, keyring
    crate::calls::install(shell); // P6 — GstCallMedia, Call window, call routes
    let _ = shell;
}

/// `activate`: present the main window — unless this is a `--background` launch's first and an
/// icon exists (`tray_rules::starts_hidden`), with the backstop.
fn activate(shell: &Rc<Shell>) {
    let Some(window) = shell.main_window() else {
        return;
    };
    let first = !ACTIVATED.with(|flag| flag.replace(true));
    if first
        && tray_rules::starts_hidden(shell.launched_at_login.get(), shell.tray.get().available())
    {
        crate::fc_info!("started at login: staying in the top bar");
        let weak = Rc::downgrade(&window);
        glib::timeout_add_local_once(tray_rules::WINDOW_ARRIVAL_GRACE, move || {
            // No icon appeared (the extension is off, the watcher never answered): never a window
            // nobody can reach.
            if let Some(window) = weak.upgrade()
                && !Shell::get().tray.get().available()
            {
                window.present();
            }
        });
        // Hidden, but alive: hold the application.
        window.hide_to_tray();
        return;
    }
    window.present();
}

/// The application's actions (`seams::actions::APP_ACTIONS`).
fn add_app_actions(shell: &Rc<Shell>) {
    let app = &shell.app;
    let add = |spec: ActionSpec, run: Box<dyn Fn(Option<&glib::Variant>)>| {
        let action = gio::SimpleAction::new(spec.name, spec.variant_type());
        action.connect_activate(move |_, parameter| run(parameter));
        app.add_action(&action);
    };
    add(
        actions::TOGGLE_WINDOW,
        Box::new(|_| {
            if let Some(window) = Shell::get().main_window() {
                window.toggle();
            }
        }),
    );
    add(
        actions::OPEN_CHAT,
        Box::new(|parameter| {
            if let Some(NotificationTarget::Open(target)) =
                parse(actions::OPEN_CHAT.name, parameter)
                && let Some(chat_id) = target.chat_id
            {
                route(Route::Chat(chat_id));
            }
        }),
    );
    add(
        actions::OPEN_BOARD,
        Box::new(|parameter| {
            if let Some(NotificationTarget::Open(target)) =
                parse(actions::OPEN_BOARD.name, parameter)
            {
                route(Route::Board(target.family_id));
            }
        }),
    );
    add(actions::OPEN_FAMILY, Box::new(|_| route(Route::Family)));
    add(
        actions::ANSWER_CALL,
        Box::new(|parameter| {
            if let Some(NotificationTarget::Answer(call_id)) =
                parse(actions::ANSWER_CALL.name, parameter)
            {
                Shell::get().call_routes.get().answer(&call_id);
            }
        }),
    );
    add(
        actions::DECLINE_CALL,
        Box::new(|parameter| {
            if let Some(NotificationTarget::Decline(call_id)) =
                parse(actions::DECLINE_CALL.name, parameter)
            {
                Shell::get().call_routes.get().decline(&call_id);
            }
        }),
    );
    add(
        actions::RETURN_TO_CALL,
        Box::new(|_| Shell::get().call_routes.get().return_to_call()),
    );
    add(
        actions::SHOW_SETTINGS,
        Box::new(|_| Shell::get().family_routes.get().show_settings()),
    );
    add(
        actions::SHORTCUTS,
        Box::new(|_| {
            if let Some(window) = Shell::get().main_window() {
                window.show_shortcuts();
            }
        }),
    );
    add(
        actions::ABOUT,
        Box::new(|_| {
            if let Some(window) = Shell::get().main_window() {
                window.show_about();
            }
        }),
    );
    add(actions::QUIT, Box::new(|_| Shell::get().quit()));
}

/// A notification's (untrusted) target, read by fc-logic's rule: anything but a positive id opens
/// nothing.
fn parse(action: &str, parameter: Option<&glib::Variant>) -> Option<NotificationTarget> {
    let target = parameter.map(target_value);
    notification_target::parse(action, target.as_ref())
}

/// A `GVariant` target as fc-logic reads it.
pub fn target_value(variant: &glib::Variant) -> TargetValue {
    if let Some(id) = variant.get::<i64>() {
        TargetValue::Int64(id)
    } else if let Some(text) = variant.get::<String>() {
        TargetValue::Str(text)
    } else {
        TargetValue::Other
    }
}

fn route(route: Route) {
    if let Some(window) = Shell::get().main_window() {
        window.route(route);
    }
}

/// Settings that act at once: the hot key's switch binds or unbinds the portal's shortcut (P5's
/// row only writes the setting).
fn follow_settings(shell: &Rc<Shell>) {
    shell
        .hot_key
        .get()
        .apply(shell.settings.flag(keys::HOT_KEY));
    let weak = Rc::downgrade(shell);
    shell.settings.changed().subscribe(move |key: &str| {
        let Some(shell) = weak.upgrade() else {
            return;
        };
        match key {
            keys::HOT_KEY => shell
                .hot_key
                .get()
                .apply(shell.settings.flag(keys::HOT_KEY)),
            keys::NOTIFY => shell.refresh_attention(),
            _ => {}
        }
    });
}

/// The outbox's three triggers besides the socket (`LiveConnection`'s module comment): a timer, a
/// returning network, the window coming forward (`MainWindow`).
fn keep_flushing(shell: &Rc<Shell>) {
    let weak = Rc::downgrade(shell);
    glib::timeout_add_local(FLUSH_EVERY, move || match weak.upgrade() {
        Some(shell) => {
            shell.flush(FlushTrigger::Timer);
            glib::ControlFlow::Continue
        }
        None => glib::ControlFlow::Break,
    });
    let monitor = gio::NetworkMonitor::default();
    let weak = Rc::downgrade(shell);
    monitor.connect_network_changed(move |_, available| {
        if available && let Some(shell) = weak.upgrade() {
            // The Mac's `networkRestored`: dial now, flush now — and an Offline page tries again
            // by itself rather than waiting for its button.
            if let Some(connection) = shell.connection() {
                connection.live.reconnect_now();
            }
            if shell.gate_facts().offline {
                crate::fc_info!("network: back, trying the server again");
                shell.retry();
            }
            shell.flush(FlushTrigger::ConnectivityRestored);
        }
    });
}

/// The portable archive's `bin/family-connect` finds its icon in the `share/icons` beside it
/// (docs/packaging.md §12); an installed `/usr/bin/family-connect` finds `/usr/share/icons`.
fn add_icons_beside_executable() {
    let Some(display) = gtk::gdk::Display::default() else {
        return;
    };
    let Some(icons) = std::env::current_exe()
        .ok()
        .and_then(|exe| paths::icons_beside(&exe))
    else {
        return;
    };
    let theme = gtk::IconTheme::for_display(&display);
    if !theme.search_path().contains(&icons) {
        theme.add_search_path(&icons);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glib::prelude::ToVariant;

    #[test]
    fn the_version_line_is_the_executable_and_the_workspace_version() {
        assert_eq!(VERSION_LINE, "family-connect 1.3.0");
    }

    #[test]
    fn a_notifications_target_is_read_by_fc_logics_rule() {
        let chat = 42i64.to_variant();
        assert_eq!(
            parse(actions::OPEN_CHAT.name, Some(&chat)),
            Some(NotificationTarget::Open(
                fc_logic::notification_target::ToastTarget::chat(42)
            ))
        );
        // Anything but a positive id opens nothing.
        assert_eq!(
            parse(actions::OPEN_CHAT.name, Some(&0i64.to_variant())),
            None
        );
        assert_eq!(
            parse(actions::OPEN_CHAT.name, Some(&true.to_variant())),
            None
        );
        assert_eq!(parse(actions::OPEN_CHAT.name, None), None);
        // A wall with no known family is written as 0 and reads back as the wall shown.
        assert_eq!(
            parse(actions::OPEN_BOARD.name, Some(&0i64.to_variant())),
            Some(NotificationTarget::Open(
                fc_logic::notification_target::ToastTarget::board(None)
            ))
        );
        assert_eq!(
            parse(actions::ANSWER_CALL.name, Some(&"c-1".to_variant())),
            Some(NotificationTarget::Answer("c-1".into()))
        );
        assert_eq!(
            target_value(&"7".to_variant()),
            TargetValue::Str("7".into())
        );
    }
}
