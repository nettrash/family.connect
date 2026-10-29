//! The chats (shell-design §3.1, §4; P1): the sidebar — the family switcher in its header
//! ([`switcher`]), the invitation banner above the list and the connection line under it
//! ([`banners`]), the chat list ([`list`]) — and the conversation in the content pane
//! (`conversation::ConversationPage`). [`Chats`] is P1's `ChatRoutes` and its main-window part.
//!
//! **Selection** opens a chat a beat after the list's own handler (Windows' lesson); a family
//! switch that takes the open chat off the list selects the family chat; at launch the family chat
//! is selected but not read until it is in front of somebody (`ChatPresence`, the Mac's rule).
//! In a collapsed window (one pane) a chosen chat brings the content pane forward.
//!
//! **Rows** come from `ChatListModel` over the cache, redrawn on an idle after any router event,
//! resync or session change — never more than once a turn.

pub mod avatar;
pub mod banners;
pub mod list;
pub mod subs;
pub mod switcher;

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use adw::prelude::*;
use fc_core::rt::{Event, LocalBoxFuture};
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::chat_list::ChatListModel;
use fc_logic::doorway::row_time_text;
use fc_logic::notifications::NotificationRules;

use crate::conversation::menu::{self, MessageActions};
use crate::conversation::{ConversationPage, PageHost};
use crate::seams::routes::{ChatRoutes, WindowPart};
use crate::services::log;
use crate::shell::{Connection, Shell};
use crate::windows::main_window::{GatePage, MainWindow};
use list::{ChatList, ListHandlers};
use subs::Subscriptions;

thread_local! {
    static LIST_CHANGED: Rc<Event<()>> = Rc::new(Event::new());
    static INSTANCE: RefCell<Weak<Chats>> = const { RefCell::new(Weak::new()) };
}

/// The process's chats (one, made by `windows::gate::install`).
#[cfg_attr(
    not(feature = "selftest"),
    allow(dead_code, reason = "the chat self-test reads it")
)]
pub fn instance() -> Option<Rc<Chats>> {
    INSTANCE.with(|instance| instance.borrow().upgrade())
}

/// Raised by whoever changed what the list shows without a router event (a read reported).
pub fn list_changed() -> Rc<Event<()>> {
    LIST_CHANGED.with(Rc::clone)
}

/// P1's chats: `ChatRoutes` and the main window's sidebar and content.
pub struct Chats {
    window: RefCell<Weak<MainWindow>>,
    list: RefCell<Option<Rc<ChatList>>>,
    model: RefCell<Option<Rc<ChatListModel>>>,
    wired: RefCell<Option<Rc<Connection>>>,
    selected: Cell<Option<i64>>,
    page: RefCell<Option<Rc<ConversationPage>>>,
    subs: Subscriptions,
    queued: Cell<bool>,
    me: Weak<Chats>,
}

impl std::fmt::Debug for Chats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Chats")
            .field("selected", &self.selected.get())
            .finish_non_exhaustive()
    }
}

impl Chats {
    pub fn new() -> Rc<Chats> {
        let chats = Rc::new_cyclic(|me| Chats {
            window: RefCell::new(Weak::new()),
            list: RefCell::new(None),
            model: RefCell::new(None),
            wired: RefCell::new(None),
            selected: Cell::new(None),
            page: RefCell::new(None),
            subs: Subscriptions::new(),
            queued: Cell::new(false),
            me: me.clone(),
        });
        INSTANCE.with(|instance| *instance.borrow_mut() = Rc::downgrade(&chats));
        chats
    }

    /// The page open in the main window.
    pub fn page(&self) -> Option<Rc<ConversationPage>> {
        self.page.borrow().clone()
    }

    /// The chat list widget.
    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "the chat self-test reads it")
    )]
    pub fn list(&self) -> Option<Rc<ChatList>> {
        self.list.borrow().clone()
    }

    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "the chat self-test reads it")
    )]
    pub fn selected(&self) -> Option<i64> {
        self.selected.get()
    }

    fn main(&self) -> Option<Rc<MainWindow>> {
        self.window.borrow().upgrade()
    }

    /// A redraw of the list on the next idle turn.
    pub fn schedule(&self) {
        if self.queued.replace(true) {
            return;
        }
        let weak = self.me.clone();
        glib::idle_add_local_once(move || {
            if let Some(chats) = weak.upgrade() {
                chats.queued.set(false);
                chats.refresh();
            }
        });
    }

    /// Wires the list to the shell's connection (another server: everything forgotten first).
    fn rewire(&self) {
        let shell = Shell::get();
        let connection = shell.connection();
        let same = match (&connection, self.wired.borrow().as_ref()) {
            (Some(a), Some(b)) => Rc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }
        self.subs.clear();
        self.close_page();
        self.selected.set(None);
        crate::composer::drafts::forget_all();
        *self.wired.borrow_mut() = connection.clone();
        let Some(connection) = connection else {
            *self.model.borrow_mut() = None;
            return;
        };
        let chats = Rc::clone(&connection.chats);
        let say: Rc<dyn StringCatalog> = shell.say.clone();
        let model = Rc::new(
            ChatListModel::new(Rc::clone(&connection.chats), move || chats.reader(), say)
                .with_zone(shell.zone.clone()),
        );
        *self.model.borrow_mut() = Some(model);
        let again = {
            let weak = self.me.clone();
            move || {
                if let Some(chats) = weak.upgrade() {
                    chats.schedule();
                }
            }
        };
        let router = &connection.router;
        {
            let again = again.clone();
            self.subs
                .on(router, |r| r.on_chat_changed(), move |_| again());
        }
        {
            let again = again.clone();
            self.subs.on(router, |r| r.on_arrived(), move |_| again());
        }
        {
            let again = again.clone();
            self.subs.on(router, |r| r.on_edited(), move |_| again());
        }
        {
            let again = again.clone();
            self.subs
                .on(router, |r| r.on_chat_removed(), move |_| again());
        }
        {
            let again = again.clone();
            self.subs
                .on(router, |r| r.on_roster_changed(), move |_| again());
        }
        {
            let again = again.clone();
            self.subs
                .on(router, |r| r.on_block_changed(), move |_| again());
        }
        {
            let again = again.clone();
            self.subs
                .on(router, |r| r.on_invitation_changed(), move |_| again());
        }
        {
            let again = again.clone();
            self.subs
                .on(&connection.live, |l| l.on_resynced(), move |_| again());
        }
        {
            let again = again.clone();
            self.subs
                .on(&connection.live, |l| l.on_link_changed(), move |_| again());
        }
        {
            let again = again.clone();
            self.subs
                .on(&connection.session, |s| s.on_changed(), move |_| again());
        }
        {
            let again = again.clone();
            self.subs.on(&list_changed(), |e| e, move |_| again());
        }
        self.schedule();
    }

    /// Draws the list, the switcher and the two banners from the cache and the session.
    fn refresh(&self) {
        let Some(main) = self.main() else {
            return;
        };
        let shell = Shell::get();
        let Some(connection) = shell.connection() else {
            return;
        };
        let Some(model) = self.model.borrow().clone() else {
            return;
        };
        let state = connection.session.state();
        let rows = match model.rows(time::OffsetDateTime::now_utc()) {
            Ok(rows) => rows,
            Err(error) => {
                crate::fc_warn!(
                    "chat list: the cache refused ({})",
                    log::store_error(&error)
                );
                return;
            }
        };
        let times: Vec<String> = rows
            .iter()
            .map(|row| {
                row_time_text::format(
                    row.when,
                    row.at,
                    &shell.culture,
                    shell.zone.as_ref(),
                    shell.say.as_ref(),
                )
            })
            .collect();
        if let Some(list) = self.list.borrow().as_ref() {
            list.update(&rows, &times);
        }
        main.sidebar_title_slot()
            .set_child(Some(&switcher::title(&shell, &state, &model.held())));
        main.sidebar_banner_slot()
            .set_child(Some(&banners::invitations(&shell, &state)));
        let network = gio::NetworkMonitor::default().is_network_available();
        let line = banners::connection_text(connection.live.link(), network, shell.say.as_ref());
        match line {
            Some(text) => {
                let label = gtk::Label::builder()
                    .label(&text)
                    .css_classes(["fc-chat-connection"])
                    .build();
                main.sidebar_footer_slot().set_child(Some(&label));
            }
            None => main.sidebar_footer_slot().set_child(None::<&gtk::Widget>),
        }
        // The open chat left the list (a family switch, a chat removed): the family chat.
        let listed: Vec<i64> = rows.iter().map(|row| row.chat.id).collect();
        let open = self.selected.get().filter(|id| listed.contains(id));
        match open {
            Some(id) => {
                if let Some(list) = self.list.borrow().as_ref() {
                    list.select_quietly(id);
                }
            }
            None if main.page() == GatePage::Chats => {
                if let Some(first) = listed.first().copied() {
                    self.select(first, false);
                } else {
                    self.close_page();
                    self.selected.set(None);
                }
            }
            None => {}
        }
    }

    /// Opens `chat_id` in the content pane; `forward`: the reader chose it (a collapsed window
    /// shows the content pane).
    fn select(&self, chat_id: i64, forward: bool) {
        let Some(main) = self.main() else {
            return;
        };
        let shell = Shell::get();
        let Some(connection) = shell.connection() else {
            return;
        };
        if let Some(list) = self.list.borrow().as_ref() {
            list.select_quietly(chat_id);
        }
        let current = self.page.borrow().as_ref().map(|page| page.chat_id);
        if current != Some(chat_id) {
            self.close_page();
            let host: Rc<dyn PageHost> = Rc::new(MainHost(Rc::downgrade(&main)));
            match ConversationPage::open(&shell, &connection, chat_id, host) {
                Some(page) => {
                    main.content_slot().set_child(Some(&page.widget()));
                    main.set_paste_attachment_enabled(!page.composer().field().has_focus());
                    *self.page.borrow_mut() = Some(page);
                    self.selected.set(Some(chat_id));
                    // A notification for this chat has done its work.
                    shell
                        .notifier
                        .get()
                        .withdraw(&NotificationRules::chat_tag(chat_id));
                }
                None => {
                    self.selected.set(None);
                    return;
                }
            }
        }
        if forward {
            let split = main.split_view();
            split.set_show_content(true);
            if let Some(page) = self.page.borrow().as_ref() {
                page.maybe_read();
                // One pane: the conversation came forward and its field takes the keys (left to
                // itself the navigation view would focus the first selectable bubble).
                if split.is_collapsed() {
                    let field = page.composer().field().clone();
                    glib::idle_add_local_once(move || {
                        field.grab_focus();
                    });
                }
            }
        }
    }

    fn close_page(&self) {
        let page = self.page.borrow_mut().take();
        if let Some(page) = page {
            page.close();
        }
        if let Some(main) = self.main() {
            // The focus must not die with the page: libadwaita would try to give it back later.
            let slot = main.content_slot();
            let focus_inside = gtk::prelude::GtkWindowExt::focus(main.window())
                .is_some_and(|focus| focus.is_ancestor(&slot));
            if focus_inside {
                gtk::prelude::GtkWindowExt::set_focus(main.window(), None::<&gtk::Widget>);
            }
            slot.set_child(Some(&no_conversation()));
            main.set_paste_attachment_enabled(false);
        }
    }
}

impl ChatRoutes for Chats {
    fn open_chat(&self, chat_id: i64) {
        let shell = Shell::get();
        shell
            .notifier
            .get()
            .withdraw(&NotificationRules::chat_tag(chat_id));
        let held = shell
            .connection()
            .is_some_and(|connection| matches!(connection.chats.chat(chat_id), Ok(Some(_))));
        if !held {
            crate::fc_info!("route: chat {chat_id} is not in the cache");
            return;
        }
        self.refresh();
        self.select(chat_id, true);
        if let Some(main) = self.main() {
            main.present();
        }
    }

    fn record_voice(&self) {
        if let Some(page) = self.page() {
            page.composer().record_voice();
        }
    }

    fn record_video(&self) {
        if let Some(page) = self.page() {
            page.composer().record_video();
        }
    }

    fn paste_attachment(&self) {
        if let Some(page) = self.page() {
            page.composer().paste_attachment();
        }
    }

    /// P2's inbox, after [`open_chat`](ChatRoutes::open_chat): the files staged in that chat's
    /// composer exactly as a drop stages them (§6.4) — the future resolves once the strip has read
    /// them, and the inbox then deletes its copies. Nothing is sent until Send.
    fn stage_files(&self, chat_id: i64, files: Vec<gio::File>) -> LocalBoxFuture<'static, ()> {
        let staging = self
            .page()
            .filter(|page| page.chat_id == chat_id)
            .and_then(|page| page.composer().staging_of(files));
        match staging {
            Some(staging) => staging,
            None => {
                crate::fc_info!("inbox: chat {chat_id} is not open; nothing staged");
                Box::pin(std::future::ready(()))
            }
        }
    }
}

impl WindowPart for Chats {
    fn attach(&self, window: &Rc<MainWindow>) {
        *self.window.borrow_mut() = Rc::downgrade(window);
        let shell = Shell::get();
        let weak = self.me.clone();
        let handlers = ListHandlers {
            select: Box::new({
                let weak = weak.clone();
                move |id| {
                    if let Some(chats) = weak.upgrade() {
                        chats.select(id, true);
                    }
                }
            }),
            activate: Box::new({
                let weak = weak.clone();
                move |id| {
                    if let Some(chats) = weak.upgrade() {
                        chats.select(id, true);
                    }
                }
            }),
            open_window: Box::new(crate::windows::conversation_window::open),
        };
        let say: Rc<dyn StringCatalog> = shell.say.clone();
        let list = ChatList::new(say, handlers);
        window.sidebar_slot().set_child(Some(list.widget()));
        *self.list.borrow_mut() = Some(list);
        window.content_slot().set_child(Some(&no_conversation()));
        // The message menu's actions on the main window, for the page it shows.
        {
            let weak = weak.clone();
            menu::install(
                window.window(),
                Rc::new(move || {
                    weak.upgrade()
                        .and_then(|chats| chats.page())
                        .map(|page| page as Rc<dyn MessageActions>)
                }),
            );
        }
        {
            let weak = weak.clone();
            shell.on_connection_changed().subscribe(move |_| {
                if let Some(chats) = weak.upgrade() {
                    chats.rewire();
                }
            });
        }
        {
            let weak = weak.clone();
            window.on_page_changed().subscribe(move |page| {
                let Some(chats) = weak.upgrade() else {
                    return;
                };
                if *page == GatePage::Chats {
                    chats.rewire();
                    chats.schedule();
                } else {
                    chats.close_page();
                    chats.selected.set(None);
                    if let Some(list) = chats.list.borrow().as_ref() {
                        list.unselect();
                    }
                }
            });
        }
        {
            // The content pane coming forward or going in a collapsed window: the read follows.
            let weak = weak.clone();
            window.split_view().connect_show_content_notify(move |_| {
                if let Some(page) = weak.upgrade().and_then(|chats| chats.page()) {
                    page.maybe_read();
                }
            });
        }
        self.rewire();
    }
}

/// The main window as a page's host.
struct MainHost(Weak<MainWindow>);

impl PageHost for MainHost {
    fn window(&self) -> Option<gtk::Window> {
        self.0.upgrade().map(|main| main.window().clone().upcast())
    }

    fn set_title(&self, title: &str, subtitle: &str) {
        if let Some(main) = self.0.upgrade() {
            let header = main.content_title();
            header.set_title(title);
            header.set_subtitle(subtitle);
        }
    }

    fn header(&self) -> Option<adw::HeaderBar> {
        self.0.upgrade().map(|main| main.content_header())
    }

    fn is_shown(&self) -> bool {
        self.0.upgrade().is_some_and(|main| {
            let split = main.split_view();
            main.page() == GatePage::Chats
                && main.window().is_visible()
                && (!split.is_collapsed() || split.shows_content())
        })
    }

    fn set_paste_enabled(&self, enabled: bool) {
        if let Some(main) = self.0.upgrade() {
            main.set_paste_attachment_enabled(enabled);
        }
    }
}

/// The detail pane with no chat (§3.1, the Mac's `ContentUnavailableView`).
fn no_conversation() -> gtk::Widget {
    let shell = Shell::get();
    let say = &shell.say;
    adw::StatusPage::builder()
        .icon_name("mail-message-new-symbolic")
        .title(say.get("No conversation selected"))
        .description(say.get("Pick a chat from the sidebar."))
        .build()
        .upcast()
}
