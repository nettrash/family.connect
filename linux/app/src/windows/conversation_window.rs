//! A conversation in its own window (shell-design §3.2): the Mac's `WindowGroup(for: chatID)` —
//! an `adw::ApplicationWindow` with the same conversation page, 720 × 640 by default and at least
//! 420 × 320, keyed by chat id (asking twice presents the one already open). It shares the process,
//! the cache and the socket with the main window, registers the main window's `win.*` names on
//! itself (`win.record-voice`, `win.record-video`, `win.paste-attachment` for its own page, and the
//! `win.message-*` family), and is not remembered across launches (only the main window's size is).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use adw::prelude::*;

use crate::conversation::menu::{self, MessageActions};
use crate::conversation::{ConversationPage, PageHost};
use crate::seams::actions;
use crate::shell::Shell;

/// The default size.
pub const DEFAULT_SIZE: (i32, i32) = (720, 640);
/// The smallest it may be.
pub const MINIMUM_SIZE: (i32, i32) = (420, 320);

/// One conversation window.
pub struct ConversationWindow {
    window: adw::ApplicationWindow,
    title: adw::WindowTitle,
    header: adw::HeaderBar,
    page: RefCell<Option<Rc<ConversationPage>>>,
}

impl std::fmt::Debug for ConversationWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConversationWindow").finish_non_exhaustive()
    }
}

thread_local! {
    static OPEN: RefCell<HashMap<i64, Rc<ConversationWindow>>> = RefCell::new(HashMap::new());
}

/// The window open for `chat_id`, if any (the self-test reads it).
pub fn window_for(chat_id: i64) -> Option<Rc<ConversationWindow>> {
    OPEN.with(|open| open.borrow().get(&chat_id).cloned())
}

/// Opens (or presents) `chat_id` in its own window.
pub fn open(chat_id: i64) {
    if let Some(existing) = window_for(chat_id) {
        existing.window.present();
        return;
    }
    let shell = Shell::get();
    let Some(connection) = shell.connection() else {
        return;
    };
    let title = adw::WindowTitle::new("", "");
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&title));
    let window = adw::ApplicationWindow::builder()
        .application(&shell.app)
        .default_width(DEFAULT_SIZE.0)
        .default_height(DEFAULT_SIZE.1)
        .build();
    window.set_size_request(MINIMUM_SIZE.0, MINIMUM_SIZE.1);
    let opened = Rc::new(ConversationWindow {
        window: window.clone(),
        title,
        header: header.clone(),
        page: RefCell::new(None),
    });
    let host: Rc<dyn PageHost> = Rc::new(WindowHost(Rc::downgrade(&opened)));
    let Some(page) = ConversationPage::open(&shell, &connection, chat_id, host) else {
        window.destroy();
        return;
    };
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&page.widget()));
    window.set_content(Some(&view));
    *opened.page.borrow_mut() = Some(Rc::clone(&page));
    add_actions(&opened);
    {
        let weak = Rc::downgrade(&opened);
        window.connect_close_request(move |_| {
            if let Some(opened) = weak.upgrade()
                && let Some(page) = opened.page.borrow_mut().take()
            {
                page.close();
            }
            OPEN.with(|open| open.borrow_mut().remove(&chat_id));
            glib::Propagation::Proceed
        });
    }
    OPEN.with(|open| open.borrow_mut().insert(chat_id, Rc::clone(&opened)));
    crate::fc_info!("conversation window: chat {chat_id}");
    // The field takes the keys from the start: left to itself the window would focus the first
    // selectable bubble, and GTK selects a label's words when it takes focus.
    GtkWindowExt::set_focus(&window, Some(page.composer().field()));
    window.present();
}

/// The `win.*` names the main window has, on this window, for its own page.
fn add_actions(opened: &Rc<ConversationWindow>) {
    let window = &opened.window;
    let page_of = {
        let weak = Rc::downgrade(opened);
        move || weak.upgrade().and_then(|o| o.page.borrow().clone())
    };
    let add = |spec: actions::ActionSpec, run: Box<dyn Fn()>| {
        let action = gio::SimpleAction::new(spec.name, spec.variant_type());
        action.connect_activate(move |_, _| run());
        window.add_action(&action);
        action
    };
    add(
        actions::REFRESH,
        Box::new(|| {
            if let Some(connection) = Shell::get().connection() {
                connection.live.refresh();
            }
        }),
    );
    add(
        actions::NEW_GROUP,
        Box::new(|| Shell::get().family_routes.get().new_group()),
    );
    {
        let page_of = page_of.clone();
        add(
            actions::RECORD_VOICE,
            Box::new(move || {
                if let Some(page) = page_of() {
                    page.composer().record_voice();
                }
            }),
        );
    }
    {
        let page_of = page_of.clone();
        add(
            actions::RECORD_VIDEO,
            Box::new(move || {
                if let Some(page) = page_of() {
                    page.composer().record_video();
                }
            }),
        );
    }
    {
        let page_of = page_of.clone();
        let paste = add(
            actions::PASTE_ATTACHMENT,
            Box::new(move || {
                if let Some(page) = page_of() {
                    page.composer().paste_attachment();
                }
            }),
        );
        // Off while the field has focus (it opens focused): its own Ctrl+V pastes words.
        paste.set_enabled(false);
    }
    {
        let full = window.downgrade();
        add(
            actions::TOGGLE_FULLSCREEN,
            Box::new(move || {
                if let Some(window) = full.upgrade() {
                    window.set_fullscreened(!window.is_fullscreen());
                }
            }),
        );
    }
    menu::install(
        window,
        Rc::new(move || page_of().map(|page| page as Rc<dyn MessageActions>)),
    );
}

impl ConversationWindow {
    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "the chat self-test reads it")
    )]
    pub fn window(&self) -> &adw::ApplicationWindow {
        &self.window
    }

    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "the chat self-test reads it")
    )]
    pub fn page(&self) -> Option<Rc<ConversationPage>> {
        self.page.borrow().clone()
    }
}

/// A conversation window as its page's host.
struct WindowHost(Weak<ConversationWindow>);

impl PageHost for WindowHost {
    fn window(&self) -> Option<gtk::Window> {
        self.0.upgrade().map(|o| o.window.clone().upcast())
    }

    fn set_title(&self, title: &str, subtitle: &str) {
        if let Some(opened) = self.0.upgrade() {
            opened.title.set_title(title);
            opened.title.set_subtitle(subtitle);
            if !title.is_empty() {
                opened.window.set_title(Some(title));
            }
        }
    }

    fn header(&self) -> Option<adw::HeaderBar> {
        self.0.upgrade().map(|o| o.header.clone())
    }

    fn is_shown(&self) -> bool {
        self.0.upgrade().is_some_and(|o| o.window.is_visible())
    }

    fn set_paste_enabled(&self, enabled: bool) {
        if let Some(opened) = self.0.upgrade()
            && let Some(action) = opened
                .window
                .lookup_action(actions::PASTE_ATTACHMENT.name)
                .and_then(|a| a.downcast::<gio::SimpleAction>().ok())
        {
            action.set_enabled(enabled);
        }
    }

    fn set_record_enabled(&self, voice: bool, video: bool) {
        let Some(opened) = self.0.upgrade() else {
            return;
        };
        for (spec, enabled) in [
            (actions::RECORD_VOICE, voice),
            (actions::RECORD_VIDEO, video),
        ] {
            if let Some(action) = opened
                .window
                .lookup_action(spec.name)
                .and_then(|a| a.downcast::<gio::SimpleAction>().ok())
            {
                action.set_enabled(enabled);
            }
        }
    }
}
