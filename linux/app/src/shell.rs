//! The process's one [`Shell`] (shell-design §2.4, W.3 — FROZEN) and the per-server
//! [`Connection`] it composes (carry-forward, "For the shell (wave 3) — the session layer" and "For
//! fc-logic's LiveConnection and the app's composition"; Windows' `AppServices` + `Connection`).
//!
//! **The `Shell`** lives from `startup` to `shutdown`, in an `Rc` reachable from everywhere on the
//! main thread through [`Shell::get`]. It holds what outlives a server: the paths, the settings, the
//! catalogue (`say`), the date culture and zone, the clock and `spawn`, the libsoup transports, the
//! link previews, and every replaceable service of W.3 in a [`Slot`] — S0's stand-ins until the
//! packages install their own (`seams`). And it holds the current [`Connection`], if a server is
//! chosen.
//!
//! **A `Connection`** is everything that talks to ONE server, wired together; changing server is a
//! new connection, never a mutation of this one ([`Shell::use_server`]). Its composition, in the
//! order carry-forward requires:
//!
//! 1. `VaultedTokens` over the installed `TokenVault` (read once: the gate stays Booting);
//! 2. `ApiClient` over [`SoupHttp`], the server's base, those tokens, the clock;
//! 3. `Database::open($XDG_STATE_HOME/family-connect/cache.db)` — the outbox is in it;
//! 4. `AppSession::new(api, tokens, cache).with_awaiting_join(SettingsAwaitingJoin)
//!    .with_current_family(SettingsCurrentFamily)` — both memories in `settings.conf`;
//! 5. the stores — `ChatStore::new(&cache).with_me(…).with_current(…).with_home_family(…)`,
//!    `BoardStore`/`PackStore` `.with_current(…)`, `OutboxStore` — reading the session's state
//!    when asked, never captured (a sign-in as somebody else must not be drawn as the last
//!    person's "You");
//! 6. `ChatSocket::new(SoupSockets, || api.socket_url(), tokens, clock)`;
//! 7. `FolderMediaStore($XDG_STATE_HOME/family-connect/media)` and `MediaOutbox::new(outbox, api,
//!    staging).with_family_for(family_rules::upload_family …)`; its `pump()` followed by the sweep
//!    (Windows' `PushMediaAsync`) is the pipeline's uploads;
//! 8. `LiveParts::compose(socket, api, chats, board, Some(pack), outbox, clock, Some(uploads))` —
//!    the send pipeline FIRST, then the router, then the resync — and
//!    `LiveConnection::new(session, parts, spawn)`, its `on_cache_failed` to the log;
//! 9. `CallEngine::new(socket, || api.ice_servers(), Shell::call_media, clock, spawn)` with the
//!    router's `on_call` feeding `engine.hear`;
//! 10. the per-connection models Windows' `Connection` keeps — `AttachmentCache` and `AvatarCache`
//!     over [`FileBlobStore`]s, `PackModel`, `PeerReads`, `AssistantAnswers`, `TranscriptModel`,
//!     `TypingRoster`, `NotificationRules` — and their router wiring; a sent sticker's, video
//!     message's or animation's bytes remembered under the attachment the server gave them;
//! 11. [`Attention`]: notifications from LIVE frames only (the router raises `arrived` and
//!     `board_changed` for those alone), the unread count to the title, the tray and the badge.
//!
//! On a session ending the connection forgets what it held for that account (peer reads, the
//! streamed answers, transcripts) and the parked recordings go (`ParkedRecordings::wipe_all`).

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use fc_core::protocol::{
    ApiClient, ChatSocket, FrameRouter, FrameSender, SendPipeline, ServerUrl, TokenStore,
};
use fc_core::rt::{Clock, Event, LocalBoxFuture};
use fc_core::store::{
    BoardStore, ChatStore, Database, OutboxStore, PackStore, StoreResult,
    transcript_store::TranscriptStore,
};
use fc_logic::app_session::{AppSession, Gate};
use fc_logic::assistant_answers::AssistantAnswers;
use fc_logic::attachment_cache::AttachmentCache;
use fc_logic::avatars::AvatarCache;
use fc_logic::calls::{CallEngine, CallMedia, CallStage, Spawn};
use fc_logic::date_culture::DateCulture;
use fc_logic::families::family_rules;
use fc_logic::folder_media_store::FolderMediaStore;
use fc_logic::link_previews::LinkPreviews;
use fc_logic::live_connection::{LiveConnection, LiveParts};
use fc_logic::media_outbox::MediaOutbox;
use fc_logic::notifications::NotificationRules;
use fc_logic::pack::PackModel;
use fc_logic::parked_recordings::ParkedRecordings;
use fc_logic::peer_reads::PeerReads;
use fc_logic::toast_arguments;
use fc_logic::transcripts::TranscriptModel;
use fc_logic::typing::TypingRoster;

use crate::seams::Slot;
use crate::seams::calls::NoCallMedia;
use crate::seams::composer::{ComposerSlots, NoRecording};
use crate::seams::conversation::{ConversationSlots, PlainSlots};
use crate::seams::desktop::{
    AutostartState, BadgeState, HotKeyState, NoDesktop, Notifier, TrayState,
};
use crate::seams::media::{Camera, NoMedia, NowPlaying, Players};
use crate::seams::routes::{
    CallRoutes, ChatRoutes, FamilyRoutes, InboxRoutes, NoRoutes, WindowPart,
};
use crate::seams::tokens::{MemoryVault, TokenVault, VaultRead, VaultedTokens};
use crate::services::catalogue::{self, Catalogue};
use crate::services::files::{self, FileBlobStore};
use crate::services::glib_clock::{self, GlibClock, GlibZone};
use crate::services::log;
use crate::services::paths::Paths;
use crate::services::settings_store::{
    SettingsAwaitingJoin, SettingsCurrentFamily, SettingsStore, keys,
};
use crate::services::soup_http::SoupHttp;
use crate::services::soup_socket::SoupSockets;
use crate::windows::main_window::MainWindow;

/// The product's name, the same in every language.
pub const BRAND: &str = "Family Connect";

/// The server a build opens on before the reader chose one, compiled in from
/// `FAMILY_CONNECT_DEFAULT_SERVER` at build time (Windows' `DefaultServer`, the iOS scheme's
/// `FCDefaultServerURL`). A build from source carries none and asks for an address; a server the
/// reader chose is always kept over it.
pub const DEFAULT_SERVER: Option<&str> = option_env!("FAMILY_CONNECT_DEFAULT_SERVER");

/// How long a quit waits for a call to be hung up before it goes anyway.
const LEAVE_LIMIT: Duration = Duration::from_secs(2);

/// What the gate's page is decided from besides the session (`windows::main_window::page_for`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GateFacts {
    /// Between choosing a server and the first answer of `GET /me` (the token read, the first
    /// refresh).
    pub booting: bool,
    /// The launch could not reach the server with a token in hand: "a flaky network is not a
    /// sign-out" (Windows' `OfflineView`).
    pub offline: bool,
}

/// Every process-wide service (shell-design §2.4). One per process; see the module's comment.
pub struct Shell {
    pub app: adw::Application,
    pub paths: Paths,
    pub settings: Rc<SettingsStore>,
    /// The desktop's language (`services::catalogue`). Ask it as `say.get("…")`.
    pub say: Rc<Catalogue>,
    /// The reader's date and time formats (`LC_TIME`, `DateCulture::for_locale`).
    pub culture: DateCulture,
    /// How the reader writes numbers (`LC_NUMERIC`): sizes' decimal separator, grouped counts.
    pub numbers: catalogue::Numbers,
    pub zone: Rc<GlibZone>,
    pub clock: Rc<GlibClock>,
    /// `glib::MainContext::spawn_local`, as fc-logic wants it.
    pub spawn: Spawn,
    /// The API's transport (fc-core `rt::Http`).
    pub http: Rc<SoupHttp>,
    /// The live socket's connector, on the API's session.
    pub sockets: Rc<SoupSockets>,
    /// Link cards (`LinkPreviews` over their own session: no redirects, no cookies), switched by
    /// the `link-previews` setting.
    pub link_previews: Rc<LinkPreviews>,

    /// P5: the Secret Service (S0: memory only).
    pub vault: Slot<dyn TokenVault>,
    /// P5: notifications.
    pub notifier: Slot<dyn Notifier>,
    /// P5: the top-bar icon (S0: none — so a close quits).
    pub tray: Slot<dyn TrayState>,
    /// P5: the dock's badge.
    pub badge: Slot<dyn BadgeState>,
    /// P5: the global shortcut.
    pub hot_key: Slot<dyn HotKeyState>,
    /// P5: launch at login.
    pub autostart: Slot<dyn AutostartState>,
    /// P3: players.
    pub players: Slot<dyn Players>,
    /// P3: one thing plays at a time.
    pub now_playing: Slot<dyn NowPlaying>,
    /// P3: the camera (P6 uses it too).
    pub camera: Slot<dyn Camera>,
    /// P3: recording inside P1's composer.
    pub composer_slots: Slot<dyn ComposerSlots>,
    /// P2: tiles, stickers, rich blocks, link cards, locations inside P1's bubbles.
    pub attachment_slots: Slot<dyn ConversationSlots>,
    /// P3: voice notes and circles inside P1's bubbles.
    pub media_slots: Slot<dyn ConversationSlots>,
    /// P6: the call media every connection's engine runs on (S0: none).
    pub call_media: Slot<dyn CallMedia>,
    /// P1: where chat routes go.
    pub chat_routes: Slot<dyn ChatRoutes>,
    /// P4: where board, family, settings and group routes go.
    pub family_routes: Slot<dyn FamilyRoutes>,
    /// P6: where call routes go (S0: the engine, no window).
    pub call_routes: Slot<dyn CallRoutes>,
    /// P2: where files handed to the app go.
    pub inbox: Slot<dyn InboxRoutes>,
    /// P3: asked first by [`quit`](Self::quit) — the main window's close when it quits, Ctrl+Q,
    /// the tray's Quit: `true` lets the quit go on now; `false` holds it, the guard having put a
    /// question to the person (§10.6's "Delete video message?" over a take), and the guard quits
    /// again itself once they agree. S0's stand-in always lets it go. (Added at integration.)
    pub close_guard: Slot<dyn Fn() -> bool>,

    window_parts: RefCell<Vec<Rc<dyn WindowPart>>>,
    connection: RefCell<Option<Rc<Connection>>>,
    connection_changed: Event<()>,
    gate: Cell<GateFacts>,
    gate_changed: Event<()>,
    main_window: RefCell<Option<Rc<MainWindow>>>,
    quitting: Cell<bool>,
    quitting_event: Event<()>,
    /// This process was started with `--background` (the autostart entry's argument).
    pub launched_at_login: Cell<bool>,
}

thread_local! {
    static SHELL: RefCell<Option<Rc<Shell>>> = const { RefCell::new(None) };
}

impl Shell {
    /// The process's shell. Panics before `startup` (a bug: nothing runs before it).
    pub fn get() -> Rc<Shell> {
        Shell::try_get().expect("Shell::get() before the application's startup")
    }

    /// The shell, or `None` before `startup` / after `shutdown`.
    pub fn try_get() -> Option<Rc<Shell>> {
        SHELL.with(|shell| shell.borrow().clone())
    }

    /// `startup`, first step (§3.7): paths → settings → catalogue, the transports, and every slot
    /// at its stand-in. The packages' `install` runs right after; the server is opened later.
    pub(crate) fn start(app: &adw::Application) -> Rc<Shell> {
        let paths = Paths::new();
        let settings = Rc::new(SettingsStore::open(paths.settings_file()));
        let say = Rc::new(Catalogue::for_desktop(&paths));
        let clock = Rc::new(GlibClock);
        let spawn = glib_clock::spawn();
        let http = Rc::new(SoupHttp::new());
        let sockets = Rc::new(SoupSockets::new(http.session().clone()));
        let link_previews = {
            let settings = Rc::clone(&settings);
            LinkPreviews::new(
                move || settings.flag(keys::LINK_PREVIEWS),
                Rc::new(SoupHttp::for_link_previews()),
                clock.clone(),
                spawn.clone(),
            )
        };
        let desktop = Rc::new(NoDesktop::default());
        let media = Rc::new(NoMedia::default());
        let routes = Rc::new(NoRoutes);
        let shell = Rc::new(Shell {
            app: app.clone(),
            culture: DateCulture::for_locale(&catalogue::time_locale()),
            numbers: catalogue::Numbers::for_locale(&catalogue::numeric_locale()),
            zone: Rc::new(GlibZone::local()),
            paths,
            settings,
            say,
            clock,
            spawn,
            http,
            sockets,
            link_previews,
            vault: Slot::new(Rc::new(MemoryVault::default())),
            notifier: Slot::new(desktop.clone()),
            tray: Slot::new(desktop.clone()),
            badge: Slot::new(desktop.clone()),
            hot_key: Slot::new(desktop.clone()),
            autostart: Slot::new(desktop),
            players: Slot::new(media.clone()),
            now_playing: Slot::new(media.clone()),
            camera: Slot::new(media),
            composer_slots: Slot::new(Rc::new(NoRecording)),
            attachment_slots: Slot::new(Rc::new(PlainSlots)),
            media_slots: Slot::new(Rc::new(PlainSlots)),
            call_media: Slot::new(Rc::new(NoCallMedia::default())),
            chat_routes: Slot::new(routes.clone()),
            family_routes: Slot::new(routes.clone()),
            call_routes: Slot::new(Rc::new(EngineCallRoutes)),
            inbox: Slot::new(routes),
            close_guard: Slot::new(Rc::new(|| true)),
            window_parts: RefCell::new(Vec::new()),
            connection: RefCell::new(None),
            connection_changed: Event::new(),
            gate: Cell::new(GateFacts::default()),
            gate_changed: Event::new(),
            main_window: RefCell::new(None),
            quitting: Cell::new(false),
            quitting_event: Event::new(),
            launched_at_login: Cell::new(false),
        });
        SHELL.with(|slot| *slot.borrow_mut() = Some(Rc::clone(&shell)));
        crate::fc_info!(
            "started: language {}, culture {}",
            shell.say.language(),
            shell.culture.name
        );
        shell
    }

    /// `shutdown`: the windows are gone; drop the services before GTK goes.
    pub(crate) fn stop() {
        let shell = SHELL.with(|slot| slot.borrow_mut().take());
        if let Some(shell) = shell {
            shell.main_window.borrow_mut().take();
            shell.connection.borrow_mut().take();
        }
    }

    /// The bubble's slots, dispatched by kind: P2's for tiles, stickers, rich blocks, link cards
    /// and locations; P3's for voice notes and circles.
    pub fn slots(&self) -> BubbleSlots {
        BubbleSlots {
            attachments: self.attachment_slots.get(),
            media: self.media_slots.get(),
        }
    }

    /// Registers a package's part of the main window (W.3). Call it from `install`; parts added
    /// after the window was built are attached at once.
    pub fn add_window_part(&self, part: Rc<dyn WindowPart>) {
        self.window_parts.borrow_mut().push(Rc::clone(&part));
        if let Some(window) = self.main_window() {
            part.attach(&window);
        }
    }

    /// The parts registered so far, in order.
    pub fn window_parts(&self) -> Vec<Rc<dyn WindowPart>> {
        self.window_parts.borrow().clone()
    }

    /// The main window, once built (it is never destroyed while the process runs).
    pub fn main_window(&self) -> Option<Rc<MainWindow>> {
        self.main_window.borrow().clone()
    }

    pub(crate) fn set_main_window(&self, window: Rc<MainWindow>) {
        *self.main_window.borrow_mut() = Some(window);
    }

    /// The server's connection, while a server is chosen.
    pub fn connection(&self) -> Option<Rc<Connection>> {
        self.connection.borrow().clone()
    }

    /// Raised when the connection is replaced (another server) or first made.
    pub fn on_connection_changed(&self) -> &Event<()> {
        &self.connection_changed
    }

    /// Booting / offline, beside the session's gate.
    pub fn gate_facts(&self) -> GateFacts {
        self.gate.get()
    }

    /// Raised when [`gate_facts`](Self::gate_facts) change.
    pub fn on_gate_changed(&self) -> &Event<()> {
        &self.gate_changed
    }

    fn set_gate(&self, facts: GateFacts) {
        if self.gate.replace(facts) != facts {
            self.gate_changed.emit(&());
        }
    }

    /// The server to open at launch: the reader's choice, else the build's default.
    pub fn saved_server(&self) -> Option<ServerUrl> {
        let chosen = self.settings.string(keys::SERVER);
        chosen
            .as_deref()
            .and_then(|text| ServerUrl::normalise(Some(text)))
            .or_else(|| DEFAULT_SERVER.and_then(|text| ServerUrl::normalise(Some(text))))
    }

    /// The "Keep running in the top bar" switch.
    pub fn keeps_running(&self) -> bool {
        self.settings.flag(keys::KEEP_RUNNING)
    }

    /// Whether closing the main window may hide it: the switch is on AND an icon exists to come
    /// back from (`tray_rules::close_hides` without the quitting term).
    pub fn can_hide_to_tray(&self) -> bool {
        fc_logic::tray_rules::close_hides(self.keeps_running(), self.tray.get().available(), false)
    }

    /// Whether the app is on its way out.
    pub fn quitting(&self) -> bool {
        self.quitting.get()
    }

    /// Raised once, synchronously, when the app starts to quit (Quit, logout's `query-end`), before
    /// anything is closed: P3 stops and parks a voice recording here with SYNCHRONOUS file writes
    /// (§3.8); nothing may await in a handler.
    pub fn on_quitting(&self) -> &Event<()> {
        &self.quitting_event
    }

    /// [`use_server`](Self::use_server), started now: the gate shows Booting at once (no flash of
    /// the server page while the old connection winds down), the rest runs on the main context.
    pub fn open_server(self: &Rc<Self>, server: ServerUrl) {
        self.set_gate(GateFacts {
            booting: true,
            offline: false,
        });
        let shell = Rc::clone(self);
        (self.spawn)(Box::pin(async move {
            let _ = shell.use_server(server).await;
        }));
    }

    /// Talk to `server` from now on (Windows' `UseAsync` + `UseServerAsync` + `RefreshAsync`): the
    /// old connection disposed, the address saved, another server's downloads and parked
    /// recordings wiped and its cache emptied, the new connection composed, the token read once,
    /// then `GET /me` decides the gate. Answers the cache's error when it cannot be opened (the
    /// gate then stays on the server page).
    pub async fn use_server(self: &Rc<Self>, server: ServerUrl) -> StoreResult<()> {
        let previous = self.settings.string(keys::SERVER);
        let old = self.connection.borrow_mut().take();
        if let Some(old) = old {
            old.dispose().await;
        }
        self.settings.set_string(keys::SERVER, &server.to_string());
        let moved = previous.is_some_and(|previous| previous != server.to_string());
        if moved {
            // Attachment ids are one server's own; nor may another server's chats hold voice
            // messages recorded for this one's.
            for folder in self.paths.server_bound_dirs() {
                files::wipe_dir(&folder);
            }
            ParkedRecordings::wipe_all(&self.paths.parked_dir());
        }
        self.set_gate(GateFacts {
            booting: true,
            offline: false,
        });
        let connection = match Connection::open(self, server) {
            Ok(connection) => connection,
            Err(error) => {
                crate::fc_warn!("cache: could not be opened ({})", log::store_error(&error));
                self.set_gate(GateFacts::default());
                self.connection_changed.emit(&());
                return Err(error);
            }
        };
        if moved {
            // One cache, one server: another family's history must not be drawn for this one.
            connection.cache.wipe_all()?;
        }
        *self.connection.borrow_mut() = Some(Rc::clone(&connection));
        self.connection_changed.emit(&());
        if let VaultRead::Unavailable = connection.tokens.load().await {
            crate::fc_info!("token: no keyring answered; the sign-in lives in memory");
        }
        self.refresh_gate(&connection).await;
        Ok(())
    }

    /// `GET /me` decides — and A FLAKY NETWORK IS NOT A SIGN-OUT: a launch that cannot reach the
    /// server with a token shows the offline page with a way to try again, never a sign-in form.
    pub async fn refresh_gate(self: &Rc<Self>, connection: &Rc<Connection>) {
        if connection.tokens.token().is_some() {
            self.set_gate(GateFacts {
                booting: true,
                offline: false,
            });
        }
        let answer = connection.session.refresh().await;
        if !self.is_current(connection) {
            return;
        }
        let offline = match answer {
            Ok(Some(error)) => {
                crate::fc_info!("session: GET /me failed ({})", error.code);
                !error.session_gone()
                    && connection.session.state().gate == Gate::SignedOut
                    && connection.tokens.token().is_some()
            }
            Ok(None) => false,
            Err(error) => {
                crate::fc_warn!("session: the cache refused ({})", log::store_error(&error));
                false
            }
        };
        self.set_gate(GateFacts {
            booting: false,
            offline,
        });
        connection.live.start();
    }

    /// The offline page's Try Again.
    pub fn retry(self: &Rc<Self>) {
        let Some(connection) = self.connection() else {
            return;
        };
        let shell = Rc::clone(self);
        (self.spawn)(Box::pin(async move {
            shell.refresh_gate(&connection).await;
        }));
    }

    fn is_current(&self, connection: &Rc<Connection>) -> bool {
        self.connection
            .borrow()
            .as_ref()
            .is_some_and(|current| Rc::ptr_eq(current, connection))
    }

    /// Quit (§3.8), from the primary menu, the tray menu, Ctrl+Q or a close with no icon: the
    /// `on_quitting` handlers (a recording parked), the window's size saved, a call hung up, the
    /// socket closed, the icon withdrawn, `app.quit()`. Idempotent.
    pub fn quit(self: &Rc<Self>) {
        if self.quitting.get() {
            return;
        }
        if !(self.close_guard.get())() {
            crate::fc_info!("quit: held for a question");
            return;
        }
        self.quitting.set(true);
        crate::fc_info!("quitting");
        self.quitting_event.emit(&());
        if let Some(window) = self.main_window() {
            window.remember_size();
        }
        self.tray.get().withdraw();
        let app = self.app.clone();
        let hold = app.hold();
        let connection = self.connection();
        (self.spawn)(Box::pin(async move {
            if let Some(connection) = connection {
                let _ = glib::future_with_timeout(LEAVE_LIMIT, connection.calls.leave()).await;
                connection.dispose().await;
            }
            for window in app.windows() {
                window.destroy();
            }
            drop(hold);
            app.quit();
        }));
    }

    /// Logout's `query-end`: what must happen before the process may end, SYNCHRONOUSLY — no
    /// awaits (§3.8).
    pub fn query_end(&self) {
        self.quitting.set(true);
        self.quitting_event.emit(&());
        if let Some(window) = self.main_window() {
            window.remember_size();
        }
    }

    /// The count in the title, beside the icon and on the dock — and none while nobody is chatting
    /// (Windows' `RefreshAttention`).
    pub fn refresh_attention(&self) {
        let (title, unread) = match self.connection() {
            Some(connection) if connection.session.state().can_chat() => {
                let title = connection
                    .notifications
                    .window_title(BRAND)
                    .unwrap_or_else(|_| BRAND.to_string());
                (title, connection.chats.unread().unwrap_or(0))
            }
            _ => (BRAND.to_string(), 0),
        };
        if let Some(window) = self.main_window() {
            window.set_title(&title);
        }
        self.tray.get().set_unread(unread, &title);
        self.badge.get().set_count(unread);
        if let Some(connection) = self.connection() {
            self.withdraw_read(&connection);
        }
    }

    /// Withdraws the notification of every chat that has been read since it was raised.
    fn withdraw_read(&self, connection: &Connection) {
        let read: Vec<i64> = connection
            .raised_chats
            .borrow()
            .iter()
            .copied()
            .filter(|chat_id| {
                matches!(connection.chats.chat(*chat_id), Ok(Some(row)) if row.unread_count == 0)
            })
            .collect();
        for chat_id in read {
            connection.raised_chats.borrow_mut().remove(&chat_id);
            self.notifier
                .get()
                .withdraw(&NotificationRules::chat_tag(chat_id));
            crate::fc_debug!("notify: chat {chat_id} read, withdrawn");
        }
    }

    /// Spawn the outbox's flush for `trigger`, when anybody is chatting.
    pub fn flush(&self, trigger: fc_core::protocol::send_rules::FlushTrigger) {
        let Some(connection) = self.connection() else {
            return;
        };
        if !connection.session.state().can_chat() {
            return;
        }
        (self.spawn)(Box::pin(async move {
            if let Err(error) = connection.live.flush(trigger).await {
                crate::fc_warn!("outbox: flush failed ({})", log::store_error(&error));
            }
        }));
    }
}

/// [`Shell::slots`]: `ConversationSlots` sent to the package that draws each kind of block.
pub struct BubbleSlots {
    attachments: Rc<dyn ConversationSlots>,
    media: Rc<dyn ConversationSlots>,
}

impl ConversationSlots for BubbleSlots {
    fn tile(
        &self,
        message: &fc_core::protocol::MessageDto,
        attachment: &fc_core::protocol::AttachmentDto,
        look: &crate::seams::conversation::TileLook,
    ) -> gtk::Widget {
        self.attachments.tile(message, attachment, look)
    }

    fn voice(
        &self,
        message: &fc_core::protocol::MessageDto,
        attachment: &fc_core::protocol::AttachmentDto,
        own: bool,
    ) -> gtk::Widget {
        self.media.voice(message, attachment, own)
    }

    fn circle(
        &self,
        message: &fc_core::protocol::MessageDto,
        attachment: &fc_core::protocol::AttachmentDto,
    ) -> gtk::Widget {
        self.media.circle(message, attachment)
    }

    fn sticker(
        &self,
        message: &fc_core::protocol::MessageDto,
        attachment: &fc_core::protocol::AttachmentDto,
    ) -> gtk::Widget {
        self.attachments.sticker(message, attachment)
    }

    fn rich(
        &self,
        block: &fc_logic::bubble_body::BodyRichBlock,
        look: &fc_logic::rich_drawing::RichLook,
    ) -> crate::seams::conversation::RichPicture {
        self.attachments.rich(block, look)
    }

    fn rich_inline(
        &self,
        source: &str,
        typed: &str,
        look: &fc_logic::rich_drawing::RichLook,
    ) -> crate::seams::conversation::RichPicture {
        self.attachments.rich_inline(source, typed, look)
    }

    fn link_card(
        &self,
        message: &fc_core::protocol::MessageDto,
        url: &str,
        own: bool,
    ) -> Option<gtk::Widget> {
        self.attachments.link_card(message, url, own)
    }

    fn location(
        &self,
        message: &fc_core::protocol::MessageDto,
        attachment: &fc_core::protocol::AttachmentDto,
    ) -> gtk::Widget {
        self.attachments.location(message, attachment)
    }
}

/// S0's `CallRoutes`: the connection's engine, for the call it holds only, with no window (P6's
/// replaces it with the Call window's).
#[derive(Debug, Default)]
pub struct EngineCallRoutes;

impl CallRoutes for EngineCallRoutes {
    fn answer(&self, call_id: &str) {
        let shell = Shell::get();
        let Some(connection) = shell.connection() else {
            return;
        };
        let ringing = connection
            .calls
            .call()
            .is_some_and(|call| call.call_id == call_id && call.stage == CallStage::Incoming);
        if ringing {
            (shell.spawn)(Box::pin(connection.calls.answer()));
        }
    }

    fn decline(&self, call_id: &str) {
        let Some(connection) = Shell::get().connection() else {
            return;
        };
        if connection
            .calls
            .call()
            .is_some_and(|call| call.call_id == call_id)
        {
            connection.calls.decline();
        }
    }
}

/// Everything that talks to ONE server (see the module's comment for the composition).
pub struct Connection {
    pub server: ServerUrl,
    pub tokens: Rc<VaultedTokens>,
    pub api: Rc<ApiClient>,
    pub cache: Rc<Database>,
    pub session: Rc<AppSession>,
    pub chats: Rc<ChatStore>,
    pub board: Rc<BoardStore>,
    #[cfg_attr(
        not(feature = "selftest"),
        allow(
            dead_code,
            reason = "the attachments self-test seeds the pack through it"
        )
    )]
    pub pack: Rc<PackStore>,
    pub outbox: Rc<OutboxStore>,
    pub socket: Rc<ChatSocket>,
    pub staging: Rc<FolderMediaStore>,
    pub media: Rc<MediaOutbox>,
    pub sending: Rc<SendPipeline>,
    pub router: Rc<FrameRouter>,
    pub live: Rc<LiveConnection>,
    pub calls: Rc<CallEngine>,
    pub attachments: Rc<AttachmentCache>,
    pub avatars: Rc<AvatarCache>,
    pub stickers: Rc<PackModel>,
    pub peer_reads: Rc<PeerReads>,
    pub answers: Rc<AssistantAnswers>,
    pub transcripts: Rc<TranscriptModel>,
    pub typing: Rc<TypingRoster>,
    pub notifications: Rc<NotificationRules>,
    /// The chats whose notification is up: each is withdrawn once the chat is read — here, in any
    /// window, on another device, or by a resync (the Mac's `ChatNotifier.removeDelivered`).
    raised_chats: RefCell<std::collections::HashSet<i64>>,
    undo: RefCell<Vec<Box<dyn Fn()>>>,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The origin only: no path, no token.
        f.debug_struct("Connection")
            .field("server", &self.server.to_string())
            .field("gate", &self.session.state().gate)
            .finish_non_exhaustive()
    }
}

impl Connection {
    /// Composes a connection to `server` (see the module's comment). The cache folder is made
    /// `0700` first.
    pub fn open(shell: &Rc<Shell>, server: ServerUrl) -> StoreResult<Rc<Connection>> {
        let clock: Rc<dyn Clock> = shell.clock.clone();
        let spawn = shell.spawn.clone();
        let say: Rc<dyn fc_core::text::string_catalog::StringCatalog> = shell.say.clone();
        let base = server.to_string();

        let tokens = Rc::new(VaultedTokens::new(
            &server,
            shell.vault.get(),
            spawn.clone(),
        ));
        let token_store: Rc<dyn TokenStore> = tokens.clone();
        let api = Rc::new(ApiClient::new(
            shell.http.clone(),
            server.clone(),
            Rc::clone(&token_store),
            Rc::clone(&clock),
        ));
        if let Err(error) = files::private_dir(shell.paths.state_dir()) {
            crate::fc_warn!("cache: its folder could not be made ({})", error.kind());
        }
        let cache = Database::open(shell.paths.cache_db())?;
        let session = Rc::new(
            AppSession::new(Rc::clone(&api), Rc::clone(&token_store), Rc::clone(&cache))
                .with_awaiting_join(Rc::new(SettingsAwaitingJoin::new(
                    Rc::clone(&shell.settings),
                    &base,
                )))
                .with_current_family(Rc::new(SettingsCurrentFamily::new(
                    Rc::clone(&shell.settings),
                    &base,
                ))),
        );
        let chats = {
            let (me, current, home) = (
                Rc::clone(&session),
                Rc::clone(&session),
                Rc::clone(&session),
            );
            Rc::new(
                ChatStore::new(&cache)
                    .with_me(move || me.state().me.map_or(0, |user| user.id))
                    .with_current(move || current.state().current_family_id)
                    .with_home_family(move || home.state().home_family_id()),
            )
        };
        let board = {
            let current = Rc::clone(&session);
            Rc::new(BoardStore::new(&cache).with_current(move || current.state().current_family_id))
        };
        let pack = {
            let current = Rc::clone(&session);
            Rc::new(PackStore::new(&cache).with_current(move || current.state().current_family_id))
        };
        let outbox = Rc::new(OutboxStore::new(&cache));
        let socket = {
            let api = Rc::clone(&api);
            Rc::new(ChatSocket::new(
                shell.sockets.clone(),
                move || api.socket_url(),
                Rc::clone(&token_store),
                Rc::clone(&clock),
            ))
        };
        let staging = Rc::new(FolderMediaStore::new(shell.paths.staging_dir()));
        let media = {
            let (session, chats) = (Rc::clone(&session), Rc::clone(&chats));
            Rc::new(
                MediaOutbox::new(Rc::clone(&outbox), Rc::clone(&api), staging.clone())
                    .with_family_for(move |chat_id| {
                        let state = session.state();
                        if !state.names_families {
                            return None;
                        }
                        let row = chats.chat(chat_id).ok().flatten();
                        family_rules::upload_family(
                            row.as_ref().map(|row| &row.chat),
                            state.current_family_id,
                            state.home_family_id(),
                        )
                    }),
            )
        };
        let uploads = {
            let (pump, media) = (media.pump(), Rc::clone(&media));
            Rc::new(move || -> LocalBoxFuture<'static, StoreResult<()>> {
                let (pump, media) = (Rc::clone(&pump), Rc::clone(&media));
                Box::pin(async move {
                    pump().await?;
                    // Files no queued row names any more are not kept (Windows' PushMediaAsync).
                    if let Err(error) = media.sweep() {
                        crate::fc_warn!("staging: sweep failed ({})", log::store_error(&error));
                    }
                    Ok(())
                })
            }) as fc_core::protocol::send_pipeline::Uploads
        };
        let parts = LiveParts::compose(
            Rc::clone(&socket),
            Rc::clone(&api),
            Rc::clone(&chats),
            Rc::clone(&board),
            Some(Rc::clone(&pack)),
            Rc::clone(&outbox),
            Rc::clone(&clock),
            Some(uploads),
        );
        let (sending, router) = (Rc::clone(&parts.sending), Rc::clone(&parts.router));
        let live = Rc::new(LiveConnection::new(
            Rc::clone(&session),
            parts,
            spawn.clone(),
        ));
        let calls = {
            let api = Rc::clone(&api);
            let sender: Rc<dyn FrameSender> = socket.clone();
            Rc::new(CallEngine::new(
                sender,
                Rc::new(move || {
                    let api = Rc::clone(&api);
                    Box::pin(async move { api.ice_servers().await })
                }),
                shell.call_media.get(),
                Rc::clone(&clock),
                spawn.clone(),
            ))
        };
        let attachments = Rc::new(AttachmentCache::new(
            Rc::clone(&api),
            Rc::new(FileBlobStore::new(shell.paths.blobs_dir())),
        ));
        let avatars = Rc::new(AvatarCache::new(
            Rc::clone(&api),
            Rc::new(FileBlobStore::new(shell.paths.avatars_dir())),
        ));
        let stickers = Rc::new(PackModel::new(
            Rc::clone(&pack),
            Rc::clone(&chats),
            Rc::clone(&api),
            Rc::clone(&attachments),
            Rc::clone(&clock),
        ));
        let transcripts = Rc::new(TranscriptModel::new(
            Rc::clone(&api),
            Rc::new(TranscriptStore::new(&cache)),
            Rc::clone(&clock),
        ));
        let typing = Rc::new(TypingRoster::new(
            Rc::clone(&chats),
            Rc::clone(&clock),
            Rc::clone(&say),
        ));
        let notifications = Rc::new(NotificationRules::new(Rc::clone(&chats), say));
        let connection = Rc::new(Connection {
            server,
            tokens,
            api,
            cache,
            session,
            chats,
            board,
            pack,
            outbox,
            socket,
            staging,
            media,
            sending,
            router,
            live,
            calls,
            attachments,
            avatars,
            stickers,
            peer_reads: Rc::new(PeerReads::new()),
            answers: Rc::new(AssistantAnswers::new()),
            transcripts,
            typing,
            notifications,
            raised_chats: RefCell::default(),
            undo: RefCell::new(Vec::new()),
        });
        connection.wire(shell);
        Ok(connection)
    }

    /// The router, the outbox and the session wired to the models and to [`Attention`].
    fn wire(self: &Rc<Self>, shell: &Rc<Shell>) {
        let weak = Rc::downgrade(self);
        let shell_weak = Rc::downgrade(shell);
        let mut undo: Vec<Box<dyn Fn()>> = Vec::new();

        {
            let id = self.live.on_cache_failed().subscribe(|error| {
                crate::fc_warn!("live: the cache refused ({})", log::store_error(error));
            });
            let me = weak.clone();
            undo.push(Box::new(move || {
                if let Some(me) = me.upgrade() {
                    me.live.on_cache_failed().unsubscribe(id);
                }
            }));
        }
        {
            let calls = Rc::downgrade(&self.calls);
            let id = self.router.on_call().subscribe(move |frame| {
                if let Some(calls) = calls.upgrade() {
                    calls.hear(frame);
                }
            });
            let me = weak.clone();
            undo.push(Box::new(move || {
                if let Some(me) = me.upgrade() {
                    me.router.on_call().unsubscribe(id);
                }
            }));
        }
        {
            let peer_reads = Rc::clone(&self.peer_reads);
            let id = self
                .router
                .on_peer_read()
                .subscribe(move |(chat_id, _, last_read)| {
                    peer_reads.apply(*chat_id, *last_read);
                });
            let me = weak.clone();
            undo.push(Box::new(move || {
                if let Some(me) = me.upgrade() {
                    me.router.on_peer_read().unsubscribe(id);
                }
            }));
        }
        {
            let (answers, chats) = (Rc::clone(&self.answers), Rc::clone(&self.chats));
            let id = self
                .router
                .on_ai_delta()
                .subscribe(move |(chat_id, message_id, text)| {
                    let held = chats.message(*message_id).ok().flatten();
                    answers.delta(*chat_id, *message_id, text, held.as_ref());
                });
            let answers = Rc::clone(&self.answers);
            let stopped =
                self.router
                    .on_ai_stopped()
                    .subscribe(move |(chat_id, message_id, reason)| {
                        answers.stopped(*chat_id, *message_id, *reason);
                    });
            let answers = Rc::clone(&self.answers);
            let edited = self
                .router
                .on_edited()
                .subscribe(move |message| answers.finished(message));
            let me = weak.clone();
            undo.push(Box::new(move || {
                if let Some(me) = me.upgrade() {
                    me.router.on_ai_delta().unsubscribe(id);
                    me.router.on_ai_stopped().unsubscribe(stopped);
                    me.router.on_edited().unsubscribe(edited);
                }
            }));
        }
        {
            let typing = Rc::clone(&self.typing);
            let heard = self
                .router
                .on_typing()
                .subscribe(move |(chat_id, user_id)| {
                    if let Err(error) = typing.heard(*chat_id, *user_id) {
                        crate::fc_warn!("typing: the cache refused ({})", log::store_error(&error));
                    }
                });
            let (typing, shell) = (Rc::clone(&self.typing), shell_weak.clone());
            let me = weak.clone();
            let arrived = self.router.on_arrived().subscribe(move |message| {
                typing.spoke(message);
                if let (Some(shell), Some(me)) = (shell.upgrade(), me.upgrade()) {
                    Attention::arrived(&shell, &me, message);
                }
            });
            let (shell, me) = (shell_weak.clone(), weak.clone());
            let note = self.router.on_board_changed().subscribe(move |note| {
                if let (Some(shell), Some(me)) = (shell.upgrade(), me.upgrade()) {
                    Attention::note(&shell, &me, note);
                }
            });
            let shell = shell_weak.clone();
            let chat = self.router.on_chat_changed().subscribe(move |_| {
                if let Some(shell) = shell.upgrade() {
                    shell.refresh_attention();
                }
            });
            let shell = shell_weak.clone();
            let resynced = self.live.on_resynced().subscribe(move |_| {
                if let Some(shell) = shell.upgrade() {
                    shell.refresh_attention();
                }
            });
            let shell = shell_weak.clone();
            let changed = self.session.on_changed().subscribe(move |_| {
                if let Some(shell) = shell.upgrade() {
                    shell.refresh_attention();
                }
            });
            let me = weak.clone();
            undo.push(Box::new(move || {
                if let Some(me) = me.upgrade() {
                    me.router.on_typing().unsubscribe(heard);
                    me.router.on_arrived().unsubscribe(arrived);
                    me.router.on_board_changed().unsubscribe(note);
                    me.router.on_chat_changed().unsubscribe(chat);
                    me.live.on_resynced().unsubscribe(resynced);
                    me.session.on_changed().unsubscribe(changed);
                }
            }));
        }
        {
            // A sticker, a video message or an animation on its way out is bytes this device holds:
            // kept under the id the server just gave them, so the message is drawn at once.
            let attachments = Rc::clone(&self.attachments);
            let id = self
                .media
                .landed
                .subscribe(move |(row, attachment, staged)| {
                    if !(row.sticker || row.round || row.animation) {
                        return;
                    }
                    attachments.remember(attachment, &staged.bytes);
                    if (row.round || row.animation)
                        && let Some(poster) = staged.preview.as_deref().filter(|p| !p.is_empty())
                    {
                        attachments.remember_preview(attachment, poster);
                    }
                });
            let me = weak.clone();
            undo.push(Box::new(move || {
                if let Some(me) = me.upgrade() {
                    me.media.landed.unsubscribe(id);
                }
            }));
        }
        {
            let (peer_reads, answers, transcripts) = (
                Rc::clone(&self.peer_reads),
                Rc::clone(&self.answers),
                Rc::clone(&self.transcripts),
            );
            let parked = shell.paths.parked_dir();
            let id = self.session.on_ended().subscribe(move |why| {
                crate::fc_info!("session: ended ({why:?})");
                peer_reads.clear();
                answers.clear();
                transcripts.clear();
                // A recording belongs to whoever made it.
                ParkedRecordings::wipe_all(&parked);
            });
            let me = weak.clone();
            undo.push(Box::new(move || {
                if let Some(me) = me.upgrade() {
                    me.session.on_ended().unsubscribe(id);
                }
            }));
        }
        *self.undo.borrow_mut() = undo;
    }

    /// Done with this server: nothing is listened to any more, the call engine lets go of the
    /// media, the socket's loop and the passes wind down (Windows' `DisposeAsync`).
    pub async fn dispose(&self) {
        for undo in self.undo.borrow_mut().drain(..) {
            undo();
        }
        self.calls.detach();
        self.live.dispose().await;
    }
}

/// What asks for the reader's attention (Windows' `Attention`): a notification for a LIVE message
/// or a note that says something new, and the count in the title, beside the icon and on the dock.
struct Attention;

impl Attention {
    /// The rules primed with what the session and the window say now.
    fn prime(shell: &Shell, connection: &Connection) {
        let state = connection.session.state();
        let rules = &connection.notifications;
        rules.set_family_name(state.family.as_ref().map(|family| family.name.as_str()));
        rules.set_families(state.all_families());
        rules.set_assistant_user_id(state.assistant.as_ref().map(|assistant| assistant.user_id));
        rules.set_reading(crate::conversation::page::being_read());
        rules.set_board_in_front(
            crate::windows::board_window::current()
                .is_some_and(|board| board.window.is_visible() && board.window.is_active()),
        );
        rules.set_wanted(shell.notifier.get().available() && shell.settings.flag(keys::NOTIFY));
    }

    fn arrived(
        shell: &Rc<Shell>,
        connection: &Rc<Connection>,
        message: &fc_core::protocol::MessageDto,
    ) {
        Attention::prime(shell, connection);
        match connection.notifications.for_message(message) {
            Ok(Some(toast)) => {
                crate::fc_info!("notify: {} raised", toast.tag);
                shell.notifier.get().show(&toast);
                connection.raised_chats.borrow_mut().insert(message.chat_id);
            }
            Ok(None) => crate::fc_debug!("notify: message {} skipped", message.id),
            Err(error) => {
                crate::fc_warn!("notify: the cache refused ({})", log::store_error(&error))
            }
        }
        shell.refresh_attention();
    }

    fn note(shell: &Rc<Shell>, connection: &Rc<Connection>, note: &fc_core::protocol::NoteDto) {
        Attention::prime(shell, connection);
        // News by the badge of the wall the note is ON — each family keeps its own marks.
        let wall = match (note.family_id, connection.board.family()) {
            (Some(family), Some(_)) => connection.board.in_family(Some(family)),
            _ => connection.board.in_family(connection.board.family()),
        };
        let raised = wall.marks().and_then(|marks| {
            connection
                .notifications
                .for_note(note, toast_arguments::is_news(note, marks))
        });
        match raised {
            Ok(Some(toast)) => {
                crate::fc_info!("notify: {} raised", toast.tag);
                shell.notifier.get().show(&toast);
            }
            Ok(None) => crate::fc_debug!("notify: note {} skipped", note.id),
            Err(error) => {
                crate::fc_warn!("notify: the cache refused ({})", log::store_error(&error))
            }
        }
        shell.refresh_attention();
    }
}
