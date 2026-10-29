//! The main window's FRAME (shell-design §3.1, §3.3, §3.4, §3.6; W.2 "S0 — skeleton"): an
//! `adw::ApplicationWindow` whose root `gtk::Overlay` holds an `adw::ViewStack` with one page per
//! gate — Booting, Server, SignIn, Offline, Family, Pending, Chats — switched by [`page_for`] from
//! `AppSession`'s gate and the shell's [`GateFacts`]. The Chats page is the
//! `adw::NavigationSplitView` (sidebar 240–380, fraction 0.28; one pane below 680 sp). Every page's
//! content is an EMPTY placeholder in an `adw::Bin` slot that P1 fills (`seams::routes::WindowPart`).
//!
//! **Frozen accessors** (W.3) — what the packages' window parts use:
//!
//! | Accessor | What | Who fills it |
//! | --- | --- | --- |
//! | [`MainWindow::window`] | the `adw::ApplicationWindow` | — |
//! | [`MainWindow::overlay`] | the root overlay: P2's hidden rich renderer (opacity 0, `can_target(false)`, §9.3), P3's recorder layer over the content (§10.6) | P2, P3 |
//! | [`MainWindow::page_slot`] | each gate page's `adw::Bin` (not Chats) — Server, SignIn, Offline, Family, Pending, Booting | P1 |
//! | [`MainWindow::sidebar_title_slot`] | the sidebar header's title: the family switcher | P1 |
//! | [`MainWindow::sidebar_banner_slot`] | above the chat list: `InvitationBanner` | P1 |
//! | [`MainWindow::sidebar_slot`] | the chat list | P1 |
//! | [`MainWindow::sidebar_footer_slot`] | under the list: `ConnectionBanner` | P1 |
//! | [`MainWindow::content_slot`] | the conversation (its placeholder: "No conversation selected") | P1 |
//! | [`MainWindow::content_title`] | the content header's `adw::WindowTitle`: chat name, typing line | P1 |
//! | [`MainWindow::content_header`] | the content header: `pack_end` Return to Call, Board, Family, Call, Video Call, Open polls, Group info | P6, P4, P1 |
//! | [`MainWindow::split_view`] | `set_show_content(true)` when a chat opens in a collapsed window | P1 |
//! | [`MainWindow::set_recorder_open`] | header buttons insensitive, routes held (§10.6) | P3 |
//! | [`MainWindow::set_paste_attachment_enabled`] | `win.paste-attachment` only while the field is not focused (§6.3) | P1 |
//! | [`MainWindow::page`], [`MainWindow::on_page_changed`] | the gate page shown | everyone |
//!
//! **Close, hide, quit** (§3.3, carry-forward decision 3 "No top-bar icon: closing the window
//! QUITS"): the close button and Ctrl+W hide the window when `tray_rules::close_hides(keep running,
//! an icon exists, quitting)`, holding the application so GTK does not end `run()`; otherwise the
//! close is a Quit (§3.8). The window is never destroyed while the process runs.
//!
//! **Size**: 1000 × 680 by default, at least 420 × 400; the last size and maximised state are kept
//! in `settings.conf` — never a position (Wayland, D13).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use fc_core::rt::Event;
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::app_session::Gate;
use fc_logic::families::family_rules;
use fc_logic::global_hot_key_rules::{self, HotKeyAction};
use fc_logic::tray_rules;

use crate::seams::actions::{self, ActionSpec};
use crate::services::settings_store::keys;
use crate::shell::{BRAND, GateFacts, Shell};

/// The default size (the Mac's `.defaultSize(1000, 680)`).
pub const DEFAULT_SIZE: (i32, i32) = (1000, 680);
/// The smallest the window may be.
pub const MINIMUM_SIZE: (i32, i32) = (420, 400);
/// Below this width (sp) the split view shows one pane at a time.
pub const COLLAPSE_BELOW_SP: f64 = 680.0;

/// One page of the gate (§3.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GatePage {
    /// A server chosen, its first answer not in yet.
    Booting,
    /// No server: the address field (`doorway::server_check`).
    Server,
    /// A server, nobody signed in: sign in / create an account.
    SignIn,
    /// A token, and the server unreachable at launch: "Try Again" (not a sign-out).
    Offline,
    /// Signed in, in no family: create, join, invitations.
    Family,
    /// A join request waiting for an owner.
    Pending,
    /// In a family: the chats.
    Chats,
}

impl GatePage {
    pub const ALL: [GatePage; 7] = [
        GatePage::Booting,
        GatePage::Server,
        GatePage::SignIn,
        GatePage::Offline,
        GatePage::Family,
        GatePage::Pending,
        GatePage::Chats,
    ];

    /// The page's name in the `adw::ViewStack`.
    pub fn name(self) -> &'static str {
        match self {
            GatePage::Booting => "booting",
            GatePage::Server => "server",
            GatePage::SignIn => "sign-in",
            GatePage::Offline => "offline",
            GatePage::Family => "family",
            GatePage::Pending => "pending",
            GatePage::Chats => "chats",
        }
    }
}

/// What [`page_for`] decides from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GateInputs {
    /// A connection exists (a server is chosen and its cache opened).
    pub has_server: bool,
    pub facts: GateFacts,
    /// The session's gate (`AppSession::state().gate`).
    pub gate: Gate,
}

/// The page the window shows — the Mac's `RootView` order: booting → needs server → offline →
/// needs auth → needs family → pending approval → active. Pure.
pub fn page_for(inputs: GateInputs) -> GatePage {
    if inputs.facts.booting {
        return GatePage::Booting;
    }
    if !inputs.has_server {
        return GatePage::Server;
    }
    if inputs.facts.offline {
        return GatePage::Offline;
    }
    match inputs.gate {
        Gate::NoServer => GatePage::Server,
        Gate::SignedOut => GatePage::SignIn,
        Gate::NoFamily => GatePage::Family,
        Gate::Pending => GatePage::Pending,
        Gate::Member | Gate::Owner => GatePage::Chats,
    }
}

/// A route that waits for the chats (§2.2, §15.4 — Windows' `ToastActivation`, the Mac's
/// `consumePendingRoute`): held while the gate is not Chats or the recorder is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Chat(i64),
    /// A family's board; `None`: the family shown.
    Board(Option<i64>),
    Family,
}

/// The main window (see the module's comment).
pub struct MainWindow {
    window: adw::ApplicationWindow,
    overlay: gtk::Overlay,
    stack: adw::ViewStack,
    pages: HashMap<GatePage, adw::Bin>,
    split: adw::NavigationSplitView,
    sidebar_title: adw::Bin,
    sidebar_banner: adw::Bin,
    sidebar_list: adw::Bin,
    sidebar_footer: adw::Bin,
    content: adw::Bin,
    content_title: adw::WindowTitle,
    content_header: adw::HeaderBar,
    headers: Vec<adw::HeaderBar>,
    shown: Cell<GatePage>,
    page_changed: Event<GatePage>,
    recorder_open: Cell<bool>,
    pending: RefCell<Option<Route>>,
    hold: RefCell<Option<gio::ApplicationHoldGuard>>,
}

impl std::fmt::Debug for MainWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MainWindow")
            .field("page", &self.shown.get())
            .finish_non_exhaustive()
    }
}

impl MainWindow {
    /// Builds the frame (not presented), registers the window's actions, attaches every package's
    /// window part, and follows the gate from here on.
    pub fn build(shell: &Rc<Shell>) -> Rc<MainWindow> {
        let say = &shell.say;
        let window = adw::ApplicationWindow::builder()
            .application(&shell.app)
            .title(BRAND)
            .build();
        let (width, height) = shell.settings.window_size().unwrap_or(DEFAULT_SIZE);
        window.set_default_size(width.max(MINIMUM_SIZE.0), height.max(MINIMUM_SIZE.1));
        window.set_size_request(MINIMUM_SIZE.0, MINIMUM_SIZE.1);
        if shell.settings.flag(keys::WINDOW_MAXIMIZED) {
            window.maximize();
        }

        let stack = adw::ViewStack::new();
        let mut pages = HashMap::new();
        let mut headers = Vec::new();
        for page in GatePage::ALL.into_iter().filter(|p| *p != GatePage::Chats) {
            let slot = adw::Bin::new();
            slot.set_child(Some(&placeholder(shell, page)));
            let header = adw::HeaderBar::new();
            header.pack_end(&primary_menu_button(shell, false));
            let view = adw::ToolbarView::new();
            view.add_top_bar(&header);
            view.set_content(Some(&slot));
            stack.add_named(&view, Some(page.name()));
            pages.insert(page, slot);
            headers.push(header);
        }

        // The Chats page: the split view.
        let sidebar_title = adw::Bin::new();
        sidebar_title.set_child(Some(&adw::WindowTitle::new(BRAND, "")));
        let new_group = gtk::Button::builder()
            .icon_name("list-add-symbolic")
            .action_name(actions::NEW_GROUP.detailed())
            .tooltip_text(say.get("New Group"))
            .build();
        new_group.update_property(&[gtk::accessible::Property::Label(say.get("New Group"))]);
        let sidebar_header = adw::HeaderBar::new();
        sidebar_header.set_title_widget(Some(&sidebar_title));
        sidebar_header.pack_start(&new_group);
        sidebar_header.pack_end(&primary_menu_button(shell, true));
        let sidebar_banner = adw::Bin::new();
        let sidebar_list = adw::Bin::new();
        sidebar_list.set_vexpand(true);
        sidebar_list.set_child(Some(&placeholder(shell, GatePage::Chats)));
        let sidebar_footer = adw::Bin::new();
        let sidebar_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        sidebar_box.append(&sidebar_banner);
        sidebar_box.append(&sidebar_list);
        sidebar_box.append(&sidebar_footer);
        let sidebar_view = adw::ToolbarView::new();
        sidebar_view.add_top_bar(&sidebar_header);
        sidebar_view.set_content(Some(&sidebar_box));
        let sidebar_page = adw::NavigationPage::builder()
            .title(say.get("Chats"))
            .tag("sidebar")
            .child(&sidebar_view)
            .build();

        // Hidden while empty (an empty `adw::WindowTitle` still draws two specks); it shows itself
        // the moment P1 gives it a chat's name.
        let content_title = adw::WindowTitle::new("", "");
        content_title.set_visible(false);
        content_title.connect_title_notify(|title| title.set_visible(!title.title().is_empty()));
        let content_header = adw::HeaderBar::new();
        content_header.set_title_widget(Some(&content_title));
        let content = adw::Bin::new();
        content.set_child(Some(&no_conversation(shell)));
        let content_view = adw::ToolbarView::new();
        content_view.add_top_bar(&content_header);
        content_view.set_content(Some(&content));
        let content_page = adw::NavigationPage::builder()
            .title(BRAND)
            .tag("content")
            .child(&content_view)
            .build();

        let split = adw::NavigationSplitView::builder()
            .min_sidebar_width(240.0)
            .max_sidebar_width(380.0)
            .sidebar_width_fraction(0.28)
            .sidebar(&sidebar_page)
            .content(&content_page)
            .build();
        stack.add_named(&split, Some(GatePage::Chats.name()));
        headers.push(sidebar_header);
        headers.push(content_header.clone());

        let breakpoint = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
            adw::BreakpointConditionLengthType::MaxWidth,
            COLLAPSE_BELOW_SP,
            adw::LengthUnit::Sp,
        ));
        breakpoint.add_setter(&split, "collapsed", Some(&true.to_value()));
        window.add_breakpoint(breakpoint);

        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&stack));
        window.set_content(Some(&overlay));

        let main = Rc::new(MainWindow {
            window,
            overlay,
            stack,
            pages,
            split,
            sidebar_title,
            sidebar_banner,
            sidebar_list,
            sidebar_footer,
            content,
            content_title,
            content_header,
            headers,
            shown: Cell::new(GatePage::Server),
            page_changed: Event::new(),
            recorder_open: Cell::new(false),
            pending: RefCell::new(None),
            hold: RefCell::new(None),
        });
        main.add_window_actions(shell);
        main.follow(shell);
        main.update_page(shell);
        main
    }

    /// The window's own actions (`win.*`, `seams::actions::WINDOW_ACTIONS`).
    fn add_window_actions(self: &Rc<Self>, shell: &Rc<Shell>) {
        let add = |spec: ActionSpec, run: Box<dyn Fn()>| {
            let action = gio::SimpleAction::new(spec.name, spec.variant_type());
            action.connect_activate(move |_, _| run());
            self.window.add_action(&action);
            action
        };
        let weak = Rc::downgrade(shell);
        add(
            actions::REFRESH,
            Box::new(move || {
                if let Some(connection) = weak.upgrade().and_then(|shell| shell.connection()) {
                    connection.live.refresh();
                }
            }),
        );
        add(
            actions::NEW_GROUP,
            Box::new(|| Shell::get().family_routes.get().new_group()),
        );
        add(
            actions::RECORD_VOICE,
            Box::new(|| Shell::get().chat_routes.get().record_voice()),
        );
        add(
            actions::RECORD_VIDEO,
            Box::new(|| Shell::get().chat_routes.get().record_video()),
        );
        let paste = add(
            actions::PASTE_ATTACHMENT,
            Box::new(|| Shell::get().chat_routes.get().paste_attachment()),
        );
        // Off until P1 says a conversation is shown with its field unfocused: the field's own
        // Ctrl+V must paste words.
        paste.set_enabled(false);
    }

    /// The gate, the close rule, activation, the icon going away.
    fn follow(self: &Rc<Self>, shell: &Rc<Shell>) {
        let weak = Rc::downgrade(self);
        shell.on_gate_changed().subscribe({
            let weak = weak.clone();
            move |_| {
                if let Some(main) = weak.upgrade() {
                    main.update_page(&Shell::get());
                }
            }
        });
        shell.on_connection_changed().subscribe({
            let weak = weak.clone();
            move |_| {
                let shell = Shell::get();
                if let Some(main) = weak.upgrade() {
                    main.update_page(&shell);
                    if let Some(connection) = shell.connection() {
                        let weak = weak.clone();
                        connection.session.on_changed().subscribe(move |_| {
                            if let Some(main) = weak.upgrade() {
                                main.update_page(&Shell::get());
                            }
                        });
                    }
                }
            }
        });
        if let Some(connection) = shell.connection() {
            let weak = weak.clone();
            connection.session.on_changed().subscribe(move |_| {
                if let Some(main) = weak.upgrade() {
                    main.update_page(&Shell::get());
                }
            });
        }
        self.window.connect_close_request({
            let weak = weak.clone();
            move |_| {
                let shell = Shell::get();
                if shell.quitting() {
                    return glib::Propagation::Proceed;
                }
                let hides = tray_rules::close_hides(
                    shell.keeps_running(),
                    shell.tray.get().available(),
                    shell.quitting(),
                );
                match weak.upgrade() {
                    Some(main) if hides => main.hide_to_tray(),
                    _ => shell.quit(),
                }
                glib::Propagation::Stop
            }
        });
        self.window.connect_is_active_notify(|window| {
            let shell = Shell::get();
            if window.is_active() {
                shell.flush(fc_core::protocol::send_rules::FlushTrigger::WindowActivated);
                if let Some(connection) = shell.connection() {
                    connection.live.start();
                }
            }
            shell.refresh_attention();
        });
        self.window.connect_maximized_notify(|_| {
            if let Some(main) = Shell::get().main_window() {
                main.remember_size();
            }
        });
        // With the icon gone a hidden window could never come back: show it (§16.4).
        shell
            .tray
            .get()
            .on_available_changed()
            .subscribe(move |available| {
                if let Some(main) = weak.upgrade()
                    && !*available
                    && !main.window.is_visible()
                {
                    main.present();
                }
            });
    }

    fn update_page(self: &Rc<Self>, shell: &Rc<Shell>) {
        let connection = shell.connection();
        let page = page_for(GateInputs {
            has_server: connection.is_some(),
            facts: shell.gate_facts(),
            gate: connection
                .as_ref()
                .map_or(Gate::NoServer, |c| c.session.state().gate),
        });
        if self.stack.visible_child_name().as_deref() != Some(page.name()) {
            self.stack.set_visible_child_name(page.name());
        }
        if self.shown.replace(page) != page {
            crate::fc_info!("gate: page {}", page.name());
            self.page_changed.emit(&page);
            shell.refresh_attention();
        }
        self.apply_pending();
    }
}

impl MainWindow {
    // ---- frozen accessors (W.3) ----------------------------------------------------------------

    pub fn window(&self) -> &adw::ApplicationWindow {
        &self.window
    }

    pub fn overlay(&self) -> &gtk::Overlay {
        &self.overlay
    }

    /// The slot of a gate page other than Chats (Chats has the split view's slots). Panics for
    /// `GatePage::Chats` — a bug in the caller.
    pub fn page_slot(&self, page: GatePage) -> adw::Bin {
        self.pages
            .get(&page)
            .cloned()
            .expect("GatePage::Chats has no page slot: use the sidebar and content slots")
    }

    pub fn sidebar_title_slot(&self) -> adw::Bin {
        self.sidebar_title.clone()
    }

    pub fn sidebar_banner_slot(&self) -> adw::Bin {
        self.sidebar_banner.clone()
    }

    pub fn sidebar_slot(&self) -> adw::Bin {
        self.sidebar_list.clone()
    }

    pub fn sidebar_footer_slot(&self) -> adw::Bin {
        self.sidebar_footer.clone()
    }

    pub fn content_slot(&self) -> adw::Bin {
        self.content.clone()
    }

    pub fn content_title(&self) -> adw::WindowTitle {
        self.content_title.clone()
    }

    pub fn content_header(&self) -> adw::HeaderBar {
        self.content_header.clone()
    }

    pub fn split_view(&self) -> adw::NavigationSplitView {
        self.split.clone()
    }

    /// The gate page shown.
    pub fn page(&self) -> GatePage {
        self.shown.get()
    }

    /// Raised when the gate page changes.
    pub fn on_page_changed(&self) -> &Event<GatePage> {
        &self.page_changed
    }

    /// P3 while the video recorder covers the window: every header button insensitive and routes
    /// held; closing it lets a held route through (§10.6).
    pub fn set_recorder_open(&self, open: bool) {
        self.recorder_open.set(open);
        for header in &self.headers {
            header.set_sensitive(!open);
        }
        if !open {
            self.apply_pending();
        }
    }

    pub fn recorder_open(&self) -> bool {
        self.recorder_open.get()
    }

    /// P1: `win.paste-attachment` (Ctrl+V) only while a conversation is shown and its field is not
    /// focused (§6.3).
    pub fn set_paste_attachment_enabled(&self, enabled: bool) {
        if let Some(action) = self
            .window
            .lookup_action(actions::PASTE_ATTACHMENT.name)
            .and_then(|a| a.downcast::<gio::SimpleAction>().ok())
        {
            action.set_enabled(enabled);
        }
    }

    // ---- showing and hiding --------------------------------------------------------------------

    /// Bring the window forward (from the top bar, behind others, minimised).
    pub fn present(&self) {
        self.hold.borrow_mut().take();
        self.window.present();
    }

    /// Hide the window in the top bar, holding the application so it runs on with no window.
    pub fn hide_to_tray(&self) {
        self.remember_size();
        let shell = Shell::get();
        if self.hold.borrow().is_none() {
            *self.hold.borrow_mut() = Some(shell.app.hold());
        }
        self.window.set_visible(false);
        crate::fc_info!("window: hidden in the top bar");
    }

    /// `app.toggle-window`: `GlobalHotKeyRules::action(in front, can hide)`.
    pub fn toggle(&self) {
        let shell = Shell::get();
        match global_hot_key_rules::action(self.is_in_front(), shell.can_hide_to_tray()) {
            HotKeyAction::Hide => self.hide_to_tray(),
            HotKeyAction::Show => self.present(),
        }
    }

    /// Visible, focused and not minimised: the reader is looking at it.
    pub fn is_in_front(&self) -> bool {
        self.window.is_visible() && self.window.is_active() && !self.window.is_suspended()
    }

    /// The toplevel's title ("(3) Family Connect"): what the dock and Alt+Tab show.
    pub fn set_title(&self, title: &str) {
        if self.window.title().as_deref() != Some(title) {
            self.window.set_title(Some(title));
        }
    }

    /// Keeps the size (unless maximised) and the maximised state in `settings.conf`.
    pub fn remember_size(&self) {
        let shell = Shell::get();
        let maximized = self.window.is_maximized();
        shell.settings.set_bool(keys::WINDOW_MAXIMIZED, maximized);
        if !maximized {
            let (width, height) = self.window.default_size();
            if width > 0 && height > 0 {
                shell.settings.set_int(keys::WINDOW_WIDTH, i64::from(width));
                shell
                    .settings
                    .set_int(keys::WINDOW_HEIGHT, i64::from(height));
            }
        }
    }

    // ---- routes ----------------------------------------------------------------------------------

    /// Present the window and apply `route` once the chats are shown and the recorder is closed.
    /// A later route replaces one still waiting.
    pub fn route(&self, route: Route) {
        self.present();
        *self.pending.borrow_mut() = Some(route);
        self.apply_pending();
    }

    fn apply_pending(&self) {
        if self.shown.get() != GatePage::Chats || self.recorder_open.get() {
            return;
        }
        let Some(route) = self.pending.borrow_mut().take() else {
            return;
        };
        let shell = Shell::get();
        let Some(connection) = shell.connection() else {
            return;
        };
        let state = connection.session.state();
        // A notification opens the app in ITS family (docs/protocol.md, "Several families").
        let family = match route {
            Route::Chat(chat_id) => connection
                .chats
                .chat(chat_id)
                .ok()
                .flatten()
                .and_then(|row| {
                    family_rules::family_to_open(
                        &row.chat,
                        state.current_family_id,
                        state.home_family_id(),
                    )
                }),
            Route::Board(family) => family,
            Route::Family => None,
        };
        if let Some(family) = family
            && connection.session.switch(family)
        {
            let session = connection.session.clone();
            (shell.spawn)(Box::pin(async move {
                if let Err(error) = session.refresh_family().await {
                    crate::fc_warn!(
                        "session: the cache refused ({})",
                        crate::services::log::store_error(&error)
                    );
                }
            }));
        }
        match route {
            Route::Chat(chat_id) => shell.chat_routes.get().open_chat(chat_id),
            Route::Board(family) => shell.family_routes.get().open_board(family),
            Route::Family => shell.family_routes.get().open_family(),
        }
    }

    // ---- the primary menu's dialogs --------------------------------------------------------------

    /// `app.shortcuts`: the Keyboard Shortcuts dialog (libadwaita's), its chords read from the
    /// actions themselves.
    pub fn show_shortcuts(&self) {
        let shell = Shell::get();
        let say = &shell.say;
        let section = adw::ShortcutsSection::new(None);
        for (title, action) in [
            (say.get("Settings…"), actions::SHOW_SETTINGS.detailed()),
            (say.get("Refresh"), actions::REFRESH.detailed()),
            (
                say.get("Record Voice Message"),
                actions::RECORD_VOICE.detailed(),
            ),
            (say.get("Close Window"), actions::WINDOW_CLOSE.to_string()),
            (say.get("Quit Family Connect"), actions::QUIT.detailed()),
            (say.get("Keyboard Shortcuts"), actions::SHORTCUTS.detailed()),
        ] {
            section.add(adw::ShortcutsItem::from_action(title, &action));
        }
        section.add(adw::ShortcutsItem::new(say.get("Main Menu"), "F10"));
        let dialog = adw::ShortcutsDialog::new();
        dialog.add(section);
        dialog.present(Some(&self.window));
    }

    /// `app.about`: About Family Connect.
    pub fn show_about(&self) {
        let dialog = adw::AboutDialog::builder()
            .application_name(BRAND)
            .application_icon(crate::app::APP_ID)
            .version(env!("CARGO_PKG_VERSION"))
            .developer_name("nettrash")
            .copyright("© 2026 nettrash")
            .license_type(gtk::License::MitX11)
            .build();
        dialog.present(Some(&self.window));
    }
}

/// The primary menu button (`open-menu-symbolic`, F10): the full menu on the chats, the short one
/// (Keyboard Shortcuts, About, Quit) on the gate's other pages.
fn primary_menu_button(shell: &Shell, full: bool) -> gtk::MenuButton {
    let say = &shell.say;
    let menu = gio::Menu::new();
    if full {
        let record = gio::Menu::new();
        record.append(
            Some(say.get("Record Voice Message")),
            Some(&actions::RECORD_VOICE.detailed()),
        );
        record.append(
            Some(say.get("Record Video Message")),
            Some(&actions::RECORD_VIDEO.detailed()),
        );
        record.append(Some(say.get("Refresh")), Some(&actions::REFRESH.detailed()));
        record.append(
            Some(say.get("New Group")),
            Some(&actions::NEW_GROUP.detailed()),
        );
        menu.append_section(None, &record);
        let app = gio::Menu::new();
        app.append(
            Some(say.get("Settings…")),
            Some(&actions::SHOW_SETTINGS.detailed()),
        );
        menu.append_section(None, &app);
    }
    let help = gio::Menu::new();
    help.append(
        Some(say.get("Keyboard Shortcuts")),
        Some(&actions::SHORTCUTS.detailed()),
    );
    help.append(
        Some(say.get("About Family Connect")),
        Some(&actions::ABOUT.detailed()),
    );
    menu.append_section(None, &help);
    let quit = gio::Menu::new();
    quit.append(
        Some(say.get("Quit Family Connect")),
        Some(&actions::QUIT.detailed()),
    );
    menu.append_section(None, &quit);
    let button = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .menu_model(&menu)
        .primary(true)
        .tooltip_text(say.get("Main Menu"))
        .build();
    button.update_property(&[gtk::accessible::Property::Label(say.get("Main Menu"))]);
    button
}

/// A gate page's empty placeholder until P1 fills its slot: an `adw::StatusPage` with the page's
/// title (a sentence the apps already translate), the Offline page's Try Again, the Booting page's
/// spinner.
fn placeholder(shell: &Rc<Shell>, page: GatePage) -> gtk::Widget {
    let say = &shell.say;
    let status = adw::StatusPage::new();
    // Dimmed as a placeholder — except the Offline page, whose Try Again works today.
    if page != GatePage::Offline {
        status.add_css_class("fc-placeholder");
    }
    match page {
        GatePage::Booting => {
            status.set_child(Some(
                &adw::Spinner::builder()
                    .width_request(32)
                    .height_request(32)
                    .build(),
            ));
        }
        GatePage::Server => {
            status.set_icon_name(Some("network-server-symbolic"));
            status.set_title(say.get("Server"));
        }
        GatePage::SignIn => {
            status.set_icon_name(Some("system-users-symbolic"));
            status.set_title(say.get("Log In"));
        }
        GatePage::Offline => {
            status.set_icon_name(Some("network-offline-symbolic"));
            status.set_title(say.get("Offline"));
            let again = gtk::Button::builder()
                .label(say.get("Try Again"))
                .halign(gtk::Align::Center)
                .css_classes(["pill", "suggested-action"])
                .build();
            let weak = Rc::downgrade(shell);
            again.connect_clicked(move |_| {
                if let Some(shell) = weak.upgrade() {
                    shell.retry();
                }
            });
            status.set_child(Some(&again));
        }
        GatePage::Family => {
            status.set_icon_name(Some("system-users-symbolic"));
            status.set_title(say.get("Join a Family"));
        }
        GatePage::Pending => {
            status.set_icon_name(Some("content-loading-symbolic"));
            status.set_title(say.get("Waiting for approval"));
        }
        GatePage::Chats => {
            status.set_icon_name(Some("chat-message-new-symbolic"));
            status.set_title(say.get("Chats"));
            status.add_css_class("compact");
        }
    }
    status.upcast()
}

/// The detail pane with no chat (§3.1, the Mac's `ContentUnavailableView`).
fn no_conversation(shell: &Shell) -> gtk::Widget {
    let say = &shell.say;
    adw::StatusPage::builder()
        .icon_name("mail-message-new-symbolic")
        .title(say.get("No conversation selected"))
        .description(say.get("Pick a chat from the sidebar."))
        .build()
        .upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(has_server: bool, booting: bool, offline: bool, gate: Gate) -> GateInputs {
        GateInputs {
            has_server,
            facts: GateFacts { booting, offline },
            gate,
        }
    }

    #[test]
    fn the_gate_picks_one_page_in_the_macs_order() {
        assert_eq!(
            page_for(inputs(false, false, false, Gate::SignedOut)),
            GatePage::Server
        );
        assert_eq!(
            page_for(inputs(false, true, false, Gate::SignedOut)),
            GatePage::Booting
        );
        assert_eq!(
            page_for(inputs(true, true, true, Gate::Member)),
            GatePage::Booting
        );
        assert_eq!(
            page_for(inputs(true, false, true, Gate::SignedOut)),
            GatePage::Offline
        );
        assert_eq!(
            page_for(inputs(true, false, false, Gate::NoServer)),
            GatePage::Server
        );
        assert_eq!(
            page_for(inputs(true, false, false, Gate::SignedOut)),
            GatePage::SignIn
        );
        assert_eq!(
            page_for(inputs(true, false, false, Gate::NoFamily)),
            GatePage::Family
        );
        assert_eq!(
            page_for(inputs(true, false, false, Gate::Pending)),
            GatePage::Pending
        );
        assert_eq!(
            page_for(inputs(true, false, false, Gate::Member)),
            GatePage::Chats
        );
        assert_eq!(
            page_for(inputs(true, false, false, Gate::Owner)),
            GatePage::Chats
        );
    }

    #[test]
    fn every_page_has_its_own_name() {
        let mut names: Vec<&str> = GatePage::ALL.iter().map(|p| p.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), GatePage::ALL.len());
    }
}
