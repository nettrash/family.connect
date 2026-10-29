//! The calls' part of the shell: ONE per process, following the current connection's
//! `CallEngine` and turning every change of its call into what the person sees and what the
//! desktop is told (shell-design §14):
//!
//! - **the Call window** (`call_window`) drawn from `call_view::view` on every `on_changed` — the
//!   engine passes the media's `on_media_changed` on, so the far side's picture and the self-view
//!   are re-read there — brought forward on a new call and on every live stage (the Mac), hidden
//!   when the engine lets the call go (two seconds after it ended, saying why); its duration
//!   redrawn once a second while the call is up;
//! - **the ring's notice** (`call_notice`): the notification with Decline/Answer, the top-bar
//!   icon's attention and the dock's urgency, while a call rings here and not a moment longer;
//! - **keeping awake** (P5's `desktop::keep_awake`, `Awake::Call`) from the answer to the end;
//! - **Return to Call** in the main window, shown while there is a call.
//!
//! The engine is the connection's, the media the window's (ONE `GstCallMedia`, kept across calls
//! and servers — `Shell::call_media`): a new server is a new engine, followed here from
//! `Shell::on_connection_changed`. Quitting (§3.8) takes the notice and the inhibitor back at
//! once; `Shell::quit` hangs the call up within two seconds and destroys the window with every
//! other one — nothing here holds the application or waits.

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::time::Duration;

use adw::prelude::*;
use fc_core::rt::{Clock, Event, Subscription};
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::calls::{CallEngine, CallStage, CallState, call_text};

use super::GstCallMedia;
use super::call_notice::RingNotice;
use super::call_view::{self, Role};
use super::call_window::CallWindow;
use crate::desktop::keep_awake::{self, Awake, AwakeHold};
use crate::shell::Shell;

thread_local! {
    static CALLS: RefCell<Option<Rc<Calls>>> = const { RefCell::new(None) };
}

/// The process's calls part, once installed.
pub fn get() -> Option<Rc<Calls>> {
    CALLS.with(|calls| calls.borrow().clone())
}

/// Installs the part over `media` (already in `Shell::call_media`) and follows the connections.
pub fn install(shell: &Rc<Shell>, media: Rc<GstCallMedia>) {
    let calls = Rc::new(Calls {
        media: RefCell::new(media),
        window: RefCell::new(None),
        notice: RingNotice::default(),
        awake: RefCell::new(None),
        returns: RefCell::new(Vec::new()),
        engine: RefCell::new(None),
        presented: RefCell::new(None),
        ticking: RefCell::new(None),
        changed: Rc::new(Event::new()),
    });
    CALLS.with(|slot| *slot.borrow_mut() = Some(Rc::clone(&calls)));
    shell.on_connection_changed().subscribe(|_| {
        if let Some(calls) = get() {
            calls.attach();
        }
    });
    shell.on_quitting().subscribe(|_| {
        if let Some(calls) = get() {
            calls.quitting();
        }
    });
    calls.attach();
}

/// The Call window of the call under way, brought forward (`app.return-to-call`, an Answer from a
/// notification). Nothing without a call.
pub fn present() {
    let (Some(calls), Some(shell)) = (get(), Shell::try_get()) else {
        return;
    };
    if shell
        .connection()
        .and_then(|connection| connection.calls.call())
        .is_none()
    {
        return;
    }
    calls.refresh();
    if let Some(window) = calls.window.borrow().clone() {
        window.present();
    }
}

/// A Return to Call button (the main window's part): shown while there is a call.
pub fn add_return_button(button: gtk::Button) {
    if let Some(calls) = get() {
        calls.returns.borrow_mut().push(button);
        calls.refresh();
    }
}

/// The calls part (see the module's comment).
pub struct Calls {
    media: RefCell<Rc<GstCallMedia>>,
    window: RefCell<Option<Rc<CallWindow>>>,
    notice: RingNotice,
    /// The machine kept from idling and suspending from the answer to the end (§14.5): P5's one
    /// keep-awake, `IDLE | SUSPEND`, its own cookie (a recording's never releases it).
    awake: RefCell<Option<AwakeHold>>,
    returns: RefCell<Vec<gtk::Button>>,
    /// The engine followed, and the subscription on its `on_changed`.
    engine: RefCell<Option<(Weak<CallEngine>, Subscription)>>,
    /// The call and stage the window last came forward for.
    presented: RefCell<Option<(String, CallStage)>>,
    ticking: RefCell<Option<glib::SourceId>>,
    changed: Rc<Event<()>>,
}

impl Calls {
    /// Raised after every redraw: a call began, moved on or ended, or the connection changed —
    /// when `calls::can_place` may answer differently.
    pub fn changed(&self) -> Rc<Event<()>> {
        Rc::clone(&self.changed)
    }

    /// Follows the current connection's engine (and lets go of the last one's).
    fn attach(self: &Rc<Self>) {
        if let Some((engine, subscription)) = self.engine.borrow_mut().take()
            && let Some(engine) = engine.upgrade()
        {
            engine.on_changed().unsubscribe(subscription);
        }
        if let Some(connection) = Shell::try_get().and_then(|shell| shell.connection()) {
            let weak = Rc::downgrade(self);
            let subscription = connection.calls.on_changed().subscribe(move |_| {
                if let Some(calls) = weak.upgrade() {
                    calls.refresh();
                }
            });
            *self.engine.borrow_mut() = Some((Rc::downgrade(&connection.calls), subscription));
        }
        self.refresh();
    }

    /// Everything brought in line with the engine's call.
    pub fn refresh(self: &Rc<Self>) {
        let Some(shell) = Shell::try_get() else {
            return;
        };
        if shell.quitting() {
            return;
        }
        let connection = shell.connection();
        let call = connection
            .as_ref()
            .and_then(|connection| connection.calls.call());
        let name = match (&call, &connection) {
            (Some(call), Some(connection)) => call_view::peer_name(
                connection
                    .chats
                    .member(call.peer_user_id)
                    .ok()
                    .flatten()
                    .as_ref(),
                &*shell.say,
            ),
            _ => String::new(),
        };
        self.notice.update(&shell, call.as_ref(), &name);
        for button in self.returns.borrow().iter() {
            button.set_visible(call.is_some());
        }
        match &call {
            Some(call) => self.draw(&shell, call, &name),
            None => {
                if let Some(window) = self.window.borrow().as_ref() {
                    window.hide();
                }
                self.presented.replace(None);
                self.stop_ticking();
            }
        }
        let wanted = call_view::keeps_awake(call.as_ref());
        let held = self.awake.borrow().is_some();
        match &call {
            Some(call) if wanted && !held => {
                let window = self
                    .window
                    .borrow()
                    .as_ref()
                    .map(|window| window.window().clone().upcast::<gtk::Window>());
                *self.awake.borrow_mut() = Some(keep_awake::hold(
                    &shell.app,
                    window.as_ref(),
                    Awake::Call { video: call.video },
                    &*shell.say,
                ));
                crate::fc_info!("call: keeping the machine awake");
            }
            _ if !wanted && held => {
                self.awake.borrow_mut().take();
                crate::fc_info!("call: no longer keeping the machine awake");
            }
            _ => {}
        }
        self.changed.emit(&());
    }

    fn draw(self: &Rc<Self>, shell: &Rc<Shell>, call: &CallState, name: &str) {
        let window = self.window_or_build(shell);
        let media = self.media.borrow().clone();
        let (remote, own) = (media.remote_video(), media.self_view());
        let view = call_view::view(
            call,
            name,
            remote.is_some(),
            own.is_some(),
            shell.clock.now(),
            &*shell.say,
        );
        window.show(&view, remote, own);
        let key = (call.call_id.clone(), call.stage);
        let previous = self.presented.replace(Some(key.clone()));
        if previous.as_ref() != Some(&key) {
            // The person's picture over the initials `show` set (P2's, kept per user and version).
            crate::attachments::avatars::show_member(window.avatar(), call.peer_user_id);
            crate::fc_info!("call {}: {:?} on screen", call.call_id, call.stage);
            if call_view::presents(previous.as_ref(), call, window.is_visible()) {
                window.present();
            }
            if call.stage == CallStage::Incoming {
                window.focus_accept();
            }
            window.announce(&view.status);
        }
        if view.ticking {
            self.start_ticking();
        } else {
            self.stop_ticking();
        }
    }

    fn window_or_build(self: &Rc<Self>, shell: &Rc<Shell>) -> Rc<CallWindow> {
        if let Some(window) = self.window.borrow().as_ref() {
            return Rc::clone(window);
        }
        let say = &shell.say;
        let weak = Rc::downgrade(self);
        let pressed: Rc<dyn Fn(Role)> = Rc::new(move |role| {
            if let Some(calls) = weak.upgrade() {
                calls.pressed(role);
            }
        });
        let weak = Rc::downgrade(self);
        let escape: Rc<dyn Fn()> = Rc::new(move || {
            if let Some(calls) = weak.upgrade() {
                calls.escape();
            }
        });
        let window = Rc::new(CallWindow::build(
            &shell.app,
            say.get("Camera access is off — the call is voice-only for you."),
            pressed,
            escape,
        ));
        *self.window.borrow_mut() = Some(Rc::clone(&window));
        window
    }

    /// A disc was pressed.
    fn pressed(&self, role: Role) {
        let Some(shell) = Shell::try_get() else {
            return;
        };
        let Some(connection) = shell.connection() else {
            return;
        };
        let engine = &connection.calls;
        let ringing = engine
            .call()
            .is_some_and(|call| call.stage == CallStage::Incoming);
        match role {
            Role::Accept if ringing => (shell.spawn)(Box::pin(engine.answer())),
            Role::Decline if ringing => engine.decline(),
            Role::Accept | Role::Decline => {}
            Role::Mute => engine.toggle_mute(),
            Role::Camera => engine.toggle_camera(),
            Role::HangUp => engine.end(),
        }
    }

    /// Escape in the Call window: Decline while it rings (the Mac's `.cancelAction`), nothing
    /// otherwise.
    fn escape(&self) {
        self.pressed(Role::Decline);
    }

    fn start_ticking(self: &Rc<Self>) {
        if self.ticking.borrow().is_some() {
            return;
        }
        let weak = Rc::downgrade(self);
        let source = glib::timeout_add_local(Duration::from_secs(1), move || {
            let Some(calls) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            calls.tick();
            glib::ControlFlow::Continue
        });
        *self.ticking.borrow_mut() = Some(source);
    }

    fn stop_ticking(&self) {
        if let Some(source) = self.ticking.borrow_mut().take() {
            source.remove();
        }
    }

    /// The duration, a second on.
    fn tick(&self) {
        let Some(shell) = Shell::try_get() else {
            return;
        };
        let call = shell
            .connection()
            .and_then(|connection| connection.calls.call());
        if let (Some(call), Some(window)) = (call, self.window.borrow().as_ref()) {
            window.set_status(&call_text::status_line(
                &call,
                shell.clock.now(),
                &*shell.say,
            ));
        }
    }

    /// The app is on its way out (§3.8): the desktop stops being told a call rings, the machine
    /// may sleep again. The hang-up itself is `Shell::quit`'s.
    fn quitting(&self) {
        let Some(shell) = Shell::try_get() else {
            return;
        };
        self.notice.stop(&shell);
        self.awake.borrow_mut().take();
        self.stop_ticking();
    }
}

/// The self-test's hands and eyes.
#[cfg(feature = "selftest")]
impl Calls {
    /// The media every call runs on now.
    pub fn media(&self) -> Rc<GstCallMedia> {
        self.media.borrow().clone()
    }

    /// Another media from now on (the self-test's devices); the caller installs it in
    /// `Shell::call_media` too, before the server is opened.
    pub fn use_media(&self, media: Rc<GstCallMedia>) {
        *self.media.borrow_mut() = media;
    }

    pub fn window(&self) -> Option<Rc<CallWindow>> {
        self.window.borrow().clone()
    }

    /// Whether the machine is being kept awake (asked of the session — in a container nothing
    /// answers, so asked, not granted).
    pub fn awake(&self) -> bool {
        self.awake.borrow().is_some()
    }

    /// The main window's Return to Call, if attached.
    pub fn return_button(&self) -> Option<gtk::Button> {
        self.returns.borrow().first().cloned()
    }
}
