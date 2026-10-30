//! Family Connect for Linux — the `family-connect` binary: the GTK 4 / libadwaita / WebKitGTK 6.0 /
//! GStreamer shell over `fc-core` and `fc-logic` (docs/shell-design.md). Each module is owned by
//! one work package (§W.2); they meet only at the interfaces S0 froze (`seams`, W.3).
//!
//! | Module | What | Owner |
//! | --- | --- | --- |
//! | [`app`] | the `adw::Application`: options, signals, actions, accelerators, activation | S0 |
//! | [`attachments`] | tiles, file rows, avatars, stickers and the pack, link and map cards, the inbox | P2 |
//! | [`board`] | the board's wall, the note sheet, the calendar file | P4 |
//! | [`calls`] | `GstCallMedia` (webrtcbin), the ring tones, the Call window, call notices and routes | P6 |
//! | [`chats`] | the chat list, the family switcher, the banners, `ChatRoutes` | P1 |
//! | [`composer`] | the composer: keys, paste and drop, staging, mentions, drafts (its recording row is P3's) | P1 |
//! | [`conversation`] | the conversation page: timeline, bubbles, menus, threads, polls | P1 |
//! | [`desktop`] | notifications, the top-bar icon, the badge, launch at login, the shortcut, the keyring | P5 |
//! | [`family`] | the Family dialog and the owner's console, members, groups | P4 |
//! | [`media`] | the player, voice notes and their recording, circles, the camera, preparing media | P3 |
//! | [`rich`] | formulas and diagrams through WebKit: the renderer, its scheme, the sandbox probe | P2 |
//! | [`shell`] | `Shell` (every process-wide service, `Shell::get`) and the per-server `Connection` | S0 |
//! | [`seams`] | the frozen interfaces (W.3) and S0's stand-ins | S0 |
//! | [`services`] | paths, settings, catalogue, log, files, the libsoup and GLib seams | S0 |
//! | [`settings`] | the Settings window's rows and dialogs, `FamilyRoutes` | P4 |
//! | [`text`] | caret offsets (UTF-16 ↔ characters) and the typing aids' hooks | P1 |
//! | [`theme`] | the palette, the CSS, dark mode, HiDPI | S0 |
//! | [`viewer`] | the attachment viewer | P2 |
//! | [`windows`] | the main window's frame (S0); the other windows (P1, P2, P4) | S0 → packages |
//! | `selftest` | `family-connect --selftest OUT` (feature `selftest`): the runner, the fake server, the `shell` group | S0 → each package's group |
//!
//! Each package added its module here with one line; since the integration pass no module carries a
//! blanket `allow(dead_code)` — what only the self-test or `tools/call-interop` reads says so on the
//! item itself.

mod app;
mod attachments;
mod board;
mod calls;
mod chats;
mod composer;
mod conversation;
mod desktop;
mod family;
mod media;
mod rich;
mod seams;
#[cfg(feature = "selftest")]
mod selftest;
mod services;
mod settings;
mod shell;
mod text;
mod theme;
mod viewer;
mod windows;

fn main() -> glib::ExitCode {
    let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    #[cfg(feature = "selftest")]
    let mut args = args;

    #[cfg(feature = "selftest")]
    {
        match selftest::fake_server::take(&mut args) {
            Err(message) => {
                eprintln!("family-connect: {message}");
                return glib::ExitCode::FAILURE;
            }
            Ok(Some(port)) => return selftest::fake_server::serve_forever(port),
            Ok(None) => {}
        }
        match selftest::Options::take(&mut args) {
            Err(message) => {
                eprintln!("family-connect: {message}");
                return glib::ExitCode::FAILURE;
            }
            Ok(Some(options)) => {
                if let Err(message) = selftest::check_home(&options.out) {
                    eprintln!("family-connect: --selftest refuses to start: {message}");
                    return glib::ExitCode::FAILURE;
                }
                return app::run(&args, app::Launch::SelfTest(options));
            }
            Ok(None) => {}
        }
    }

    app::run(&args, app::Launch::Normal)
}
