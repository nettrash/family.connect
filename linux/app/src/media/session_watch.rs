//! Interruptions (shell-design §10.8, §22; Windows' `SessionWatch`): each STOPS AND KEEPS a voice
//! recording (`not_sent`), stops the video recorder into REVIEW (or closes PREVIEW), and pauses what
//! plays as `playback_pauses` says.
//!
//! | Event | Linux |
//! | --- | --- |
//! | the session locks, the screen saver starts | `gtk::Application`'s `screensaver-active` (`register-session` on) |
//! | sleep | logind's `PrepareForSleep(true)` on the SYSTEM bus |
//! | the window minimised | the main window's `notify::suspended` (GTK 4.12+) |
//! | the window hidden in the top bar | its `notify::visible` going false (§3.3) |
//! | a call in any phase | the connection's `CallEngine` busy |
//!
//! Leaving the chat, Board, Family, Settings are P1's: it calls `ComposerRecording::interrupt`.
//! None of it can be produced in a container (no lock, no logind, no compositor): the self-test
//! drives [`interrupt`] directly; the real events are §C's.
//!
//! Also here: whether a screen reader runs (`org.a11y.Status.ScreenReaderEnabled` on the session's
//! `org.a11y.Bus`, read and watched over GDBus — Windows' `ScreenReader`), which gives a recording
//! its one-second lead-in after "Recording" is said (S6).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use fc_logic::not_sent::RecordingEnd;
use fc_logic::round_look::playback_pauses;
use fc_logic::round_video_rules;
use gtk::prelude::*;

use crate::seams::media::NowPlaying;
use crate::seams::routes::WindowPart;
use crate::shell::Shell;
use crate::windows::main_window::MainWindow;

thread_local! {
    static SCREEN_READER: Cell<bool> = const { Cell::new(false) };
    /// What is subscribed per connection (the call engine), so a new connection subscribes again.
    static CALLS: RefCell<Option<fc_core::rt::Subscription>> = const { RefCell::new(None) };
    /// The D-Bus subscriptions, kept for the process.
    static KEPT: RefCell<Vec<gio::SignalSubscription>> = const { RefCell::new(Vec::new()) };
}

/// Whether a screen reader runs (read at start-up, then watched).
pub fn screen_reader() -> bool {
    SCREEN_READER.with(Cell::get)
}

/// Something other than the person ends what records and pauses what plays (§10.8).
pub fn interrupt(why: RecordingEnd) {
    crate::fc_info!("session: {why:?}");
    if let Some(slots) = super::recording::installed() {
        slots.interrupt_all(why);
    }
    if let Some(heard) = round_video_rules::interruption_of(why) {
        super::recorder_layer::interrupt(heard);
    }
    if let Some(event) = playback_pauses::of(why) {
        super::now_playing().interrupt(event);
        super::recorder_layer::playback_interrupted(event);
    }
}

/// `media::install`: the screen saver, sleep, calls, the screen reader; the window's own signals
/// come with [`SessionPart`].
pub fn start(shell: &Rc<Shell>) {
    shell.app.connect_screensaver_active_notify(|app| {
        if app.is_screensaver_active() {
            interrupt(RecordingEnd::SessionLocked);
        }
    });
    shell.add_window_part(Rc::new(SessionPart));
    follow_calls();
    shell.on_connection_changed().subscribe(|_| follow_calls());
    glib::spawn_future_local(watch_sleep());
    glib::spawn_future_local(watch_screen_reader());
}

/// A call in any phase interrupts (S4).
fn follow_calls() {
    let Some(connection) = Shell::get().connection() else {
        return;
    };
    let calls = Rc::downgrade(&connection.calls);
    let subscription = connection.calls.on_changed().subscribe(move |_| {
        if calls.upgrade().is_some_and(|calls| calls.busy()) {
            interrupt(RecordingEnd::Call);
        }
    });
    CALLS.with(|slot| *slot.borrow_mut() = Some(subscription));
}

/// logind's `PrepareForSleep(true)` on the system bus.
async fn watch_sleep() {
    let Ok(system) = gio::bus_get_future(gio::BusType::System).await else {
        crate::fc_info!("session: no system bus; sleep is not heard");
        return;
    };
    let subscription = system.subscribe_to_signal(
        Some("org.freedesktop.login1"),
        Some("org.freedesktop.login1.Manager"),
        Some("PrepareForSleep"),
        Some("/org/freedesktop/login1"),
        None,
        gio::DBusSignalFlags::NONE,
        |signal| {
            if signal.parameters.child_value(0).get::<bool>() == Some(true) {
                interrupt(RecordingEnd::SessionLocked);
            }
        },
    );
    KEPT.with(|kept| kept.borrow_mut().push(subscription));
}

/// `org.a11y.Status.ScreenReaderEnabled`, read once and watched.
async fn watch_screen_reader() {
    let Ok(session) = gio::bus_get_future(gio::BusType::Session).await else {
        return;
    };
    let read = session
        .call_future(
            Some("org.a11y.Bus"),
            "/org/a11y/bus",
            "org.freedesktop.DBus.Properties",
            "Get",
            Some(&("org.a11y.Status", "ScreenReaderEnabled").to_variant()),
            glib::VariantTy::new("(v)").ok(),
            gio::DBusCallFlags::NO_AUTO_START,
            2_000,
        )
        .await;
    if let Ok(answer) = read
        && let Some(enabled) = answer
            .child_value(0)
            .as_variant()
            .and_then(|value| value.get::<bool>())
    {
        SCREEN_READER.with(|cell| cell.set(enabled));
    }
    let subscription = session.subscribe_to_signal(
        Some("org.a11y.Bus"),
        Some("org.freedesktop.DBus.Properties"),
        Some("PropertiesChanged"),
        Some("/org/a11y/bus"),
        Some("org.a11y.Status"),
        gio::DBusSignalFlags::NONE,
        |signal| {
            let changed = glib::VariantDict::new(Some(&signal.parameters.child_value(1)));
            if let Ok(Some(enabled)) = changed.lookup::<bool>("ScreenReaderEnabled") {
                SCREEN_READER.with(|cell| cell.set(enabled));
            }
        },
    );
    KEPT.with(|kept| kept.borrow_mut().push(subscription));
}

/// The main window's own signals: minimised (`suspended`), hidden in the top bar (`visible`).
pub struct SessionPart;

impl WindowPart for SessionPart {
    fn attach(&self, window: &Rc<MainWindow>) {
        let gtk_window = window.window();
        gtk_window.connect_suspended_notify(|window| {
            if window.is_suspended() {
                interrupt(RecordingEnd::WindowHidden);
            }
        });
        gtk_window.connect_visible_notify(|window| {
            if !window.is_visible() && !Shell::get().quitting() {
                interrupt(RecordingEnd::WindowHidden);
            }
        });
    }
}
