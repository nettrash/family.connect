//! One open chat (shell-design §5): the timeline over the composer, in the main window's content
//! pane or in a conversation window of its own (§3.2). Built when the chat opens and dropped when
//! the selection moves, so a reply, an edit or a scroll position never leaks into the next chat
//! (the Mac's `.id(selectedChatID)`); the draft itself survives in `ComposerDrafts`.
//!
//! It owns the chat's `ConversationModel` (paging back, the read marker, typing, reactions, polls,
//! edits) and listens to the router for this chat only. A message is read only when the window is
//! active, the page is the one shown and the newest message is on screen — the Mac's three facts.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};

use adw::prelude::*;
use fc_core::protocol::{ChatDto, FrameSender, MessageDto};
use fc_core::store::OutboxRow;
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::bubble_rules;
use fc_logic::chat_list::ChatListModel;
use fc_logic::conversation::{Bubble, ConversationModel};
use fc_logic::conversation_rows::{self, RowMark};
use fc_logic::groups::chat_kinds;
use fc_logic::open_polls::OpenPollsModel;
use fc_logic::quotes::{self, Quote, QuoteClick, QuoteLevel};
use fc_logic::timeline;
use fc_text::reactions::{Reaction, my_reaction};

use super::bubble::{self, BubbleContext, BubbleHost};
use super::menu::{self, MenuFacts, MessageActions, Safety};
use super::{polls, safety, thread};
use crate::chats::subs::Subscriptions;
use crate::composer::{Composer, ComposerKind};
use crate::services::log;
use crate::shell::{Connection, Shell};

/// Where a page is shown: the main window's content pane, or a conversation window.
pub trait PageHost {
    /// The window the page is in.
    fn window(&self) -> Option<gtk::Window>;
    /// The chat's name and the typing line under it.
    fn set_title(&self, title: &str, subtitle: &str);
    /// The header the page's own buttons go in (Open polls).
    fn header(&self) -> Option<adw::HeaderBar>;
    /// Whether the page is in front in its window (the content pane shown).
    fn is_shown(&self) -> bool;
    /// `win.paste-attachment` on or off (§6.3: only while the field is not focused).
    fn set_paste_enabled(&self, enabled: bool);
    /// `win.record-voice` / `win.record-video` on or off: greyed where they would do nothing
    /// (the Mac's `MacVoiceCommands`).
    fn set_record_enabled(&self, voice: bool, video: bool);
}

/// The open chat (see the module's comment).
pub struct ConversationPage {
    pub chat_id: i64,
    shell: Rc<Shell>,
    connection: Rc<Connection>,
    host: Rc<dyn PageHost>,
    model: Rc<ConversationModel>,
    list: Rc<ChatListModel>,
    root: gtk::Overlay,
    timeline: Rc<Timeline>,
    composer: Rc<Composer>,
    subs: Subscriptions,
    queued: Cell<bool>,
    header_widgets: RefCell<Vec<gtk::Widget>>,
    /// A direct chat's Call (false) and Video Call (true) in the header.
    call_buttons: RefCell<Vec<(gtk::Button, bool)>>,
    first_unread: Cell<Option<i64>>,
    /// How many were unread when the anchor was decided — the divider's number, kept, since the
    /// chat's own count is 0 the moment it is read (the Mac's `unreadAtOpen`).
    unread_at_open: Cell<i32>,
    /// The Open polls button and its count, redrawn with the page (the Mac's live query).
    polls_button: RefCell<Option<(gtk::Button, gtk::Label)>>,
    /// The Open polls dialog's redraw while it is open: it follows the page's (a vote, a close, a
    /// new poll), as the Mac's list is a live query.
    polls_redraw: RefCell<Option<Rc<dyn Fn()>>>,
    opened: Cell<bool>,
    loading_older: Cell<bool>,
    closed: Cell<bool>,
    typing_tick: RefCell<Option<glib::SourceId>>,
    pending_tokens: RefCell<HashMap<String, i64>>,
    next_token: Cell<i64>,
    me: Weak<ConversationPage>,
    thread: RefCell<Option<Rc<thread::ThreadDialog>>>,
}

use super::timeline::Timeline;

impl std::fmt::Debug for ConversationPage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConversationPage")
            .field("chat_id", &self.chat_id)
            .finish_non_exhaustive()
    }
}

/// The page as the bubbles' and the composer's host (held weakly).
struct Host(Weak<ConversationPage>);

/// Whether anything records now — a voice note in any composer, or the video message recorder
/// open (P3's): no call is placed over a recording (`call_records::can_place_call`).
pub fn recording_now() -> bool {
    crate::media::recording::installed().is_some_and(|slots| slots.recording())
        || crate::media::recorder_layer::is_open()
}

thread_local! {
    /// Every page open, in any window — asked which chat is being read.
    static PAGES: RefCell<Vec<Weak<ConversationPage>>> = const { RefCell::new(Vec::new()) };
}

/// The chat the reader is reading now, in whichever window: shown, in the window in front, at its
/// newest (the Mac's `ChatPresence.isReading`) — a message there needs no notification.
pub fn being_read() -> Option<i64> {
    PAGES.with(|pages| {
        let mut pages = pages.borrow_mut();
        pages.retain(|page| page.strong_count() > 0);
        pages
            .iter()
            .filter_map(Weak::upgrade)
            .find(|page| page.is_being_read())
            .map(|page| page.chat_id)
    })
}

impl ConversationPage {
    /// Opens `chat_id` for `host`, or `None` when the cache does not hold the chat.
    pub fn open(
        shell: &Rc<Shell>,
        connection: &Rc<Connection>,
        chat_id: i64,
        host: Rc<dyn PageHost>,
    ) -> Option<Rc<ConversationPage>> {
        let row = connection.chats.chat(chat_id).ok().flatten()?;
        let socket: Rc<dyn FrameSender> = connection.socket.clone();
        let model = match ConversationModel::new(
            chat_id,
            Rc::clone(&connection.chats),
            Rc::clone(&connection.api),
            Rc::clone(&connection.sending),
            socket,
            shell.clock.clone(),
        ) {
            Ok(model) => Rc::new(model.with_outbox(Rc::clone(&connection.outbox))),
            Err(error) => {
                crate::fc_warn!(
                    "conversation: the cache refused ({})",
                    log::store_error(&error)
                );
                return None;
            }
        };
        let list = {
            let chats = Rc::clone(&connection.chats);
            let say: Rc<dyn StringCatalog> = shell.say.clone();
            Rc::new(
                ChatListModel::new(Rc::clone(&connection.chats), move || chats.reader(), say)
                    .with_zone(shell.zone.clone()),
            )
        };
        let page = Rc::new_cyclic(|me: &Weak<ConversationPage>| {
            let timeline = Timeline::new(shell.say.get("Jump to the newest message"));
            let on_sent = {
                let me = me.clone();
                Rc::new(move || {
                    if let Some(page) = me.upgrade() {
                        page.timeline.scroll_to_bottom();
                        page.schedule();
                    }
                }) as Rc<dyn Fn()>
            };
            let window = {
                let host = Rc::clone(&host);
                Rc::new(move || host.window()) as Rc<dyn Fn() -> Option<gtk::Window>>
            };
            let paste = {
                let host = Rc::clone(&host);
                Rc::new(move |enabled| host.set_paste_enabled(enabled)) as Rc<dyn Fn(bool)>
            };
            let composer = Composer::new(
                shell,
                connection,
                Rc::clone(&model),
                row.chat.clone(),
                ComposerKind::Main,
                window,
                on_sent,
                paste,
            );
            let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
            column.append(timeline.widget());
            let clamp = adw::Clamp::builder()
                .maximum_size(fc_logic::composer_row::conversation_column::MAX_COLUMN as i32)
                .tightening_threshold(
                    fc_logic::composer_row::conversation_column::MAX_COLUMN as i32,
                )
                .child(&composer.widget())
                .margin_start(fc_logic::composer_row::conversation_column::PANE_INSET as i32)
                .margin_end(fc_logic::composer_row::conversation_column::PANE_INSET as i32)
                .build();
            column.append(&clamp);
            let root = gtk::Overlay::new();
            root.set_child(Some(&column));
            ConversationPage {
                chat_id,
                shell: Rc::clone(shell),
                connection: Rc::clone(connection),
                host,
                model,
                list,
                root,
                timeline,
                composer,
                subs: Subscriptions::new(),
                queued: Cell::new(false),
                header_widgets: RefCell::new(Vec::new()),
                call_buttons: RefCell::new(Vec::new()),
                first_unread: Cell::new(None),
                unread_at_open: Cell::new(0),
                polls_button: RefCell::new(None),
                polls_redraw: RefCell::new(None),
                opened: Cell::new(false),
                loading_older: Cell::new(false),
                closed: Cell::new(false),
                typing_tick: RefCell::new(None),
                pending_tokens: RefCell::new(HashMap::new()),
                next_token: Cell::new(-1),
                me: me.clone(),
                thread: RefCell::new(None),
            }
        });
        page.composer.attach_drop(&page.root);
        {
            let weak = Rc::downgrade(&page);
            page.composer.set_on_refresh(Rc::new(move || {
                if let Some(page) = weak.upgrade() {
                    page.refresh_call_buttons();
                    let (voice, video) = page.composer.record_commands();
                    page.host.set_record_enabled(voice, video);
                }
            }));
        }
        PAGES.with(|pages| pages.borrow_mut().push(Rc::downgrade(&page)));
        page.follow();
        page.add_header_buttons();
        page.refresh_title();
        page.start();
        crate::fc_info!("conversation: chat {chat_id} opened");
        Some(page)
    }

    /// The page's widget.
    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    pub fn composer(&self) -> &Rc<Composer> {
        &self.composer
    }

    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "the chat self-test reads it")
    )]
    pub fn timeline(&self) -> &Rc<Timeline> {
        &self.timeline
    }

    pub fn model(&self) -> &Rc<ConversationModel> {
        &self.model
    }

    pub fn connection(&self) -> &Rc<Connection> {
        &self.connection
    }

    fn host_handle(&self) -> Rc<Host> {
        Rc::new(Host(self.me.clone()))
    }

    /// The chat as the cache holds it now.
    fn chat(&self) -> Option<ChatDto> {
        self.connection
            .chats
            .chat(self.chat_id)
            .ok()
            .flatten()
            .map(|row| row.chat)
    }

    fn follow(self: &Rc<Self>) {
        let id = self.chat_id;
        let router = &self.connection.router;
        let schedule = |page: &Weak<ConversationPage>| {
            let page = page.clone();
            move || {
                if let Some(page) = page.upgrade() {
                    page.schedule();
                }
            }
        };
        let weak = Rc::downgrade(self);
        {
            let weak = weak.clone();
            self.subs.on(
                router,
                |r| r.on_arrived(),
                move |message| {
                    if message.chat_id == id
                        && let Some(page) = weak.upgrade()
                    {
                        page.arrived(message);
                    }
                },
            );
        }
        {
            let again = schedule(&weak);
            self.subs.on(
                router,
                |r| r.on_edited(),
                move |message| {
                    if message.chat_id == id {
                        again();
                    }
                },
            );
        }
        {
            let again = schedule(&weak);
            self.subs.on(
                router,
                |r| r.on_chat_changed(),
                move |chat| {
                    if *chat == id {
                        again();
                    }
                },
            );
        }
        {
            let weak = weak.clone();
            self.subs.on(
                router,
                |r| r.on_typing(),
                move |(chat, _)| {
                    if *chat == id
                        && let Some(page) = weak.upgrade()
                    {
                        page.typing_changed();
                    }
                },
            );
        }
        {
            let weak = weak.clone();
            self.subs.on(
                router,
                |r| r.on_roster_changed(),
                move |_| {
                    if let Some(page) = weak.upgrade() {
                        page.timeline.clear();
                        page.schedule();
                    }
                },
            );
        }
        {
            let weak = weak.clone();
            self.subs.on(
                router,
                |r| r.on_block_changed(),
                move |_| {
                    if let Some(page) = weak.upgrade() {
                        page.timeline.clear();
                        page.schedule();
                    }
                },
            );
        }
        {
            let again = schedule(&weak);
            self.subs
                .on(&self.connection.live, |l| l.on_resynced(), move |_| again());
        }
        {
            let again = schedule(&weak);
            self.subs.on(
                &self.connection.peer_reads,
                |p| &p.changed,
                move |chat| {
                    if *chat == id {
                        again();
                    }
                },
            );
        }
        {
            let again = schedule(&weak);
            self.subs.on(
                &self.connection.answers,
                |a| &a.changed,
                move |chat| {
                    if *chat == id {
                        again();
                    }
                },
            );
        }
        {
            let again = schedule(&weak);
            self.subs.on(
                &self.connection.sending,
                |s| s.on_refused(),
                move |(row, _)| {
                    if row.chat_id == id {
                        again();
                    }
                },
            );
        }
        {
            let again = schedule(&weak);
            self.subs.on(
                &self.connection.media,
                |m| &m.refused,
                move |(row, _)| {
                    if row.chat_id == id {
                        again();
                    }
                },
            );
        }
        {
            let weak = weak.clone();
            self.subs.on(
                &self.connection.session,
                |s| s.on_changed(),
                move |_| {
                    if let Some(page) = weak.upgrade() {
                        page.refresh_title();
                        page.composer.refresh();
                        page.schedule();
                    }
                },
            );
        }
        {
            // A call beginning, moving on or ending: the call buttons are re-asked.
            let weak = weak.clone();
            self.subs.on(
                &crate::calls::on_changed(),
                |e| e,
                move |_| {
                    if let Some(page) = weak.upgrade() {
                        page.refresh_call_buttons();
                    }
                },
            );
        }
        {
            let weak = weak.clone();
            self.subs.on(
                &crate::theme::changed(),
                |e| e,
                move |_| {
                    if let Some(page) = weak.upgrade() {
                        page.timeline.clear();
                        page.schedule();
                    }
                },
            );
        }
        if let Some(window) = self.host.window() {
            let weak = weak.clone();
            let handler = window.connect_is_active_notify(move |_| {
                if let Some(page) = weak.upgrade() {
                    page.maybe_read();
                }
            });
            self.subs.defer(move || window.disconnect(handler));
        }
        {
            let weak = weak.clone();
            self.timeline.set_on_bottom(move || {
                if let Some(page) = weak.upgrade() {
                    page.maybe_read();
                }
            });
        }
        {
            let weak = weak.clone();
            self.timeline.set_on_top(move || {
                if let Some(page) = weak.upgrade() {
                    page.older();
                }
            });
        }
    }

    /// Draws what the cache holds at once, then opens: the newest page if the cache does not
    /// hold the chat in sequence, the anchor decided once (the first unread, under the divider),
    /// and the read reported if the reader is looking.
    fn start(self: &Rc<Self>) {
        self.redraw();
        self.timeline.scroll_to_bottom();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let Some(model) = weak.upgrade().map(|page| Rc::clone(&page.model)) else {
                return;
            };
            if let Ok(Some(error)) = model.open().await {
                crate::fc_info!("conversation: the first page failed ({})", error.code);
            }
            let Some(page) = weak.upgrade() else {
                return;
            };
            let me = page.connection.chats.reader();
            if let (Ok(Some(row)), Ok(bubbles)) = (
                page.connection.chats.chat(page.chat_id),
                page.model.bubbles(),
            ) {
                let messages: Vec<MessageDto> =
                    bubbles.into_iter().map(|bubble| bubble.message).collect();
                page.first_unread.set(timeline::open_anchor(
                    &messages,
                    row.unread_count,
                    row.last_read_message_id,
                    me,
                ));
                page.unread_at_open.set(row.unread_count);
            }
            page.redraw();
            match page.first_unread.get() {
                Some(first) => {
                    page.timeline
                        .scroll_to(&conversation_rows::message_key(first), false);
                }
                None => page.timeline.scroll_to_bottom(),
            }
            page.opened.set(true);
            page.maybe_read();
        });
    }

    /// The Open polls count: polls still waiting for the reader's vote, as the cache holds them now
    /// — a vote, a new poll or a close changes it (the Mac's `OpenPollsToolbarButton`).
    fn refresh_polls_badge(&self) {
        let Some((button, badge)) = self.polls_button.borrow().clone() else {
            return;
        };
        let unanswered = OpenPollsModel::new(
            self.chat_id,
            Rc::clone(&self.connection.chats),
            Rc::clone(&self.connection.api),
        )
        .unanswered()
        .unwrap_or(0);
        badge.set_label(&unanswered.to_string());
        badge.set_visible(unanswered > 0);
        let say = &self.shell.say;
        let described = if unanswered > 0 {
            say.format("%lld to answer", &[&i64::try_from(unanswered).unwrap_or(0)])
        } else {
            String::new()
        };
        button.update_property(&[gtk::accessible::Property::Description(&described)]);
    }

    /// The window's own buttons for this chat: "Open polls" in a shared chat.
    fn add_header_buttons(&self) {
        let Some(header) = self.host.header() else {
            return;
        };
        let Some(chat) = self.chat() else {
            return;
        };
        let say = &self.shell.say;
        if chat_kinds::is_shared(Some(&chat.kind)) {
            let polls_button = gtk::Button::builder()
                .icon_name("view-list-bullet-symbolic")
                .tooltip_text(say.get("Open polls"))
                .build();
            polls_button
                .update_property(&[gtk::accessible::Property::Label(say.get("Open polls"))]);
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&polls_button));
            let badge = gtk::Label::builder()
                .css_classes(["fc-badge"])
                .halign(gtk::Align::End)
                .valign(gtk::Align::Start)
                .can_target(false)
                .visible(false)
                .build();
            overlay.add_overlay(&badge);
            let page = self.me.clone();
            polls_button.connect_clicked(move |button| {
                if let Some(page) = page.upgrade() {
                    polls::open_polls(button, &page);
                }
            });
            header.pack_end(&overlay);
            self.header_widgets.borrow_mut().push(overlay.upcast());
            *self.polls_button.borrow_mut() = Some((polls_button, badge));
            self.refresh_polls_badge();
        }
        // A group's members, its name, leaving it: P4's Group Info (§3.1's content header).
        if chat.kind == chat_kinds::GROUP {
            let info = gtk::Button::builder()
                .icon_name("help-about-symbolic")
                .tooltip_text(say.get("Group Info"))
                .build();
            info.update_property(&[gtk::accessible::Property::Label(say.get("Group Info"))]);
            info.add_css_class("fc-chat-group-info");
            let chat_id = self.chat_id;
            info.connect_clicked(move |_| Shell::get().family_routes.get().group_info(chat_id));
            header.pack_end(&info);
            self.header_widgets.borrow_mut().push(info.upcast());
        }
        // A direct chat's Call and Video Call (P6's `calls`): shown where the server carries them
        // and the person is not blocked, sensitive while no call is under way and nothing records.
        if chat.kind == chat_kinds::DIRECT
            && let Some(peer) = chat.peer_user_id
        {
            for (video, icon, label) in [
                (true, "camera-video-symbolic", say.get("Video Call")),
                (false, "call-start-symbolic", say.get("Call")),
            ] {
                let button = gtk::Button::builder()
                    .icon_name(icon)
                    .tooltip_text(label)
                    .build();
                button.update_property(&[gtk::accessible::Property::Label(label)]);
                button.add_css_class(if video {
                    "fc-chat-video-call"
                } else {
                    "fc-chat-call"
                });
                let chat_id = self.chat_id;
                button.connect_clicked(move |_| {
                    let shell = Shell::get();
                    if crate::calls::can_place(&shell, recording_now()) {
                        crate::calls::place(&shell, chat_id, peer, video);
                    }
                });
                header.pack_end(&button);
                self.header_widgets
                    .borrow_mut()
                    .push(button.clone().upcast());
                self.call_buttons.borrow_mut().push((button, video));
            }
            self.refresh_call_buttons();
        }
    }

    /// The call buttons, asked again (P6's rules): SHOWN by `calls::offered` (the server's
    /// `calls_enabled` / `video_calls_enabled`, the peer not blocked), SENSITIVE by
    /// `calls::can_place` (no call under way, nothing recording — a voice note or a video message).
    pub fn refresh_call_buttons(&self) {
        let Some(peer) = self.chat().and_then(|chat| chat.peer_user_id) else {
            return;
        };
        let sensitive = crate::calls::can_place(&self.shell, recording_now());
        for (button, video) in self.call_buttons.borrow().iter() {
            button.set_visible(crate::calls::offered(&self.shell, peer, *video));
            button.set_sensitive(sensitive);
        }
    }

    /// The header's call buttons: (Video Call, Call) as drawn (the self-test reads them).
    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "the chat self-test reads it")
    )]
    pub fn call_buttons(&self) -> Vec<(gtk::Button, bool)> {
        self.call_buttons.borrow().clone()
    }

    /// The title and the typing line.
    fn refresh_title(&self) {
        let Some(chat) = self.chat() else {
            return;
        };
        let title = self
            .list
            .title(&chat)
            .unwrap_or_else(|_| chat.title.clone());
        let line = self
            .connection
            .typing
            .line(self.chat_id)
            .unwrap_or_default();
        self.host.set_title(&title, &line);
    }

    /// Someone typed: the line, and a tick until it lapses (five seconds a frame).
    fn typing_changed(self: &Rc<Self>) {
        self.refresh_title();
        if self.typing_tick.borrow().is_some() {
            return;
        }
        let weak = Rc::downgrade(self);
        let source = glib::timeout_add_local(std::time::Duration::from_secs(1), move || {
            let Some(page) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            page.refresh_title();
            if page.connection.typing.typists(page.chat_id).is_empty() {
                page.typing_tick.borrow_mut().take();
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
        *self.typing_tick.borrow_mut() = Some(source);
    }

    fn arrived(self: &Rc<Self>, message: &MessageDto) {
        self.refresh_title();
        self.schedule();
        // A new message in the open chat, said to a screen reader when the reader is at the
        // newest (§24's live lines).
        if self.timeline.at_newest() && message.sender_id != self.connection.chats.reader() {
            let bubble = Bubble::new(message.clone(), false, false, false);
            let say = &self.shell.say;
            let name = fc_logic::doorway::bubble_text::sender(
                &bubble,
                &self.connection.chats,
                say.as_ref(),
            )
            .unwrap_or_default();
            let words = fc_logic::doorway::bubble_text::words(&bubble, &self.list, say.as_ref());
            let hidden = self
                .connection
                .chats
                .is_blocked(message.sender_id)
                .unwrap_or(false);
            if !hidden {
                self.root.announce(
                    &format!("{name}: {words}"),
                    gtk::AccessibleAnnouncementPriority::Medium,
                );
            }
        }
    }

    /// A redraw on the next idle turn — many events, one drawing.
    pub fn schedule(&self) {
        if self.queued.replace(true) || self.closed.get() {
            return;
        }
        let weak = self.me.clone();
        glib::idle_add_local_once(move || {
            if let Some(page) = weak.upgrade() {
                page.queued.set(false);
                page.redraw();
                page.refresh_call_buttons();
                page.refresh_polls_badge();
                let polls = page.polls_redraw.borrow().clone();
                if let Some(redraw) = polls {
                    redraw();
                }
                page.maybe_read();
                if let Some(thread) = page.thread() {
                    thread.schedule();
                }
            }
        });
    }

    /// Everything a drawing shares.
    fn context(&self, chat: ChatDto) -> BubbleContext {
        let state = self.connection.session.state();
        let chats = &self.connection.chats;
        let me = chats.reader();
        let id = self.me.clone();
        BubbleContext {
            shell: Rc::clone(&self.shell),
            connection: Rc::clone(&self.connection),
            shared: chat_kinds::is_shared(Some(&chat.kind)),
            assistant_chat: chat.kind == chat_kinds::ASSISTANT,
            assistant_user_id: state.assistant.as_ref().map(|a| a.user_id),
            chat,
            me,
            members: chats.members().unwrap_or_else(|_| Rc::from(Vec::new())),
            blocked: chats.blocked().unwrap_or_else(|_| Rc::from(Vec::new())),
            peer_read_up_to: self.connection.peer_reads.up_to(self.chat_id),
            unread_at_open: self.unread_at_open.get(),
            in_thread: false,
            host: self.host_handle(),
            quote_revealed: Rc::new(move |message, level| {
                id.upgrade()
                    .is_some_and(|page| page.model.quote_revealed(message, level))
            }),
            list: Rc::clone(&self.list),
        }
    }

    /// Draws the chat as the cache holds it now (rows kept where nothing changed).
    pub fn redraw(&self) {
        let Some(chat) = self.chat() else {
            return;
        };
        let bubbles = match self.model.bubbles() {
            Ok(bubbles) => bubbles,
            Err(error) => {
                crate::fc_warn!(
                    "conversation: the cache refused ({})",
                    log::store_error(&error)
                );
                return;
            }
        };
        let pending = self.model.pending().unwrap_or_default();
        let ctx = self.context(chat);
        let rows = timeline::rows(
            bubbles,
            ctx.shared,
            ctx.me,
            self.first_unread.get(),
            self.shell.zone.as_ref(),
        );
        // Corners join across rows, so each row's signature carries its joins: a neighbour
        // gaining chips or a time draws this row again.
        let shapes: Vec<_> = rows.iter().map(|row| bubble::shape(&ctx, row)).collect();
        let joins = bubble::joins(&shapes);
        let mut marks: Vec<RowMark> = rows
            .iter()
            .zip(&joins)
            .map(|(row, joined)| {
                let drawn = format!("{}{}", ctx.drawn_with(row), bubble::joined_mark(*joined));
                RowMark::new(
                    conversation_rows::message_key(row.bubble.message.id),
                    conversation_rows::signature(row, &drawn),
                )
            })
            .collect();
        for (at, row) in pending.iter().enumerate() {
            marks.push(RowMark::new(
                conversation_rows::pending_key(&row.client_msg_id),
                format!(
                    "{}|{}|{}|{}|{}",
                    row.body,
                    row.failed(),
                    row.pending_files.as_ref().map_or(0, Vec::len),
                    row.failed_code.as_deref().unwrap_or_default(),
                    at == 0
                ),
            ));
        }
        let empty = rows.is_empty() && pending.is_empty();
        if empty {
            marks.push(RowMark::new(conversation_rows::EMPTY_KEY, ""));
        }
        let message_rows = rows.len();
        self.timeline.update(marks, |index| {
            if empty {
                return empty_row(&self.shell);
            }
            if index < message_rows {
                bubble::row_widget(&ctx, &rows[index], joins[index])
            } else {
                let at = index - message_rows;
                self.pending_row(&pending[at], at == 0)
            }
        });
    }

    /// A send still on its way, under the newest message: "Sending…", or — refused — "Try
    /// Again · Delete" (the Mac's failed bubble; Delete gives up on a row that never reached the
    /// server, so nobody else has to know — it asks nothing, as the Mac's does not).
    /// `first`: the first of the sends on their way, which starts a group of its own: the newest
    /// message's time stands above it, with that message, not as this one's caption.
    fn pending_row(&self, row: &OutboxRow, first: bool) -> gtk::Widget {
        let say = &self.shell.say;
        let token = self.token_for(&row.client_msg_id);
        let column = gtk::Box::new(gtk::Orientation::Vertical, 2);
        column.add_css_class("fc-chat-row");
        if first {
            column.add_css_class("fc-chat-turn");
        }
        column.set_halign(gtk::Align::End);
        let balloon = gtk::Box::new(gtk::Orientation::Vertical, 4);
        balloon.add_css_class("fc-chat-bubble");
        balloon.add_css_class("fc-own-bubble");
        balloon.add_css_class("fc-chat-pending-balloon");
        let files = row.staged_files.as_ref().map_or(0, Vec::len);
        if files > 0 {
            balloon.append(
                &gtk::Label::builder()
                    .label(format!("📎 {files}"))
                    .xalign(0.0)
                    .build(),
            );
        }
        if !row.body.is_empty() {
            balloon.append(
                &gtk::Label::builder()
                    .label(&row.body)
                    .wrap(true)
                    .wrap_mode(gtk::pango::WrapMode::WordChar)
                    .xalign(0.0)
                    .selectable(true)
                    .build(),
            );
        }
        let clamp = adw::Clamp::builder()
            .maximum_size(fc_logic::composer_row::conversation_column::BUBBLE_MAX as i32)
            .tightening_threshold(fc_logic::composer_row::conversation_column::BUBBLE_MAX as i32)
            .child(&balloon)
            .margin_start(fc_logic::composer_row::conversation_column::BUBBLE_SPACER as i32)
            .build();
        column.append(&clamp);
        let status = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        status.set_halign(gtk::Align::End);
        status.add_css_class("fc-chat-pending-status");
        if row.failed() {
            // Refused, not still on its way: the error's glyph before its two ways out (the Mac
            // marks its failed bubble the same way); the words say it to a screen reader.
            status.append(
                &gtk::Image::builder()
                    .icon_name("dialog-error-symbolic")
                    .accessible_role(gtk::AccessibleRole::Presentation)
                    .build(),
            );
            let again = gtk::Button::builder()
                .label(say.get("Try Again"))
                .css_classes(["flat", "fc-chat-pending"])
                .build();
            let delete = gtk::Button::builder()
                .label(say.get("Delete"))
                .css_classes(["flat", "fc-chat-pending", "error"])
                .action_name(format!(
                    "win.{}",
                    crate::seams::actions::MESSAGE_DELETE.name
                ))
                .action_target(&token.to_variant())
                .build();
            let (weak, key) = (self.me.clone(), row.client_msg_id.clone());
            again.connect_clicked(move |_| {
                if let Some(page) = weak.upgrade() {
                    if let Err(error) = page.model.retry(&key) {
                        crate::fc_warn!("outbox: the cache refused ({})", log::store_error(&error));
                    }
                    page.flush_and_redraw();
                }
            });
            status.append(&again);
            status.append(&gtk::Label::new(Some("·")));
            status.append(&delete);
        } else {
            status.append(
                &gtk::Label::builder()
                    .label(say.get("Sending…"))
                    .css_classes(["fc-chat-pending"])
                    .build(),
            );
        }
        column.append(&status);
        column.set_focusable(true);
        column.upcast()
    }

    fn token_for(&self, key: &str) -> i64 {
        if let Some(token) = self.pending_tokens.borrow().get(key) {
            return *token;
        }
        let token = self.next_token.get();
        self.next_token.set(token - 1);
        self.pending_tokens
            .borrow_mut()
            .insert(key.to_string(), token);
        token
    }

    /// Flushes the outbox now and draws what it did.
    pub fn flush_and_redraw(&self) {
        let (connection, weak) = (Rc::clone(&self.connection), self.me.clone());
        self.schedule();
        glib::spawn_future_local(async move {
            if let Err(error) = connection
                .live
                .flush(fc_core::protocol::send_rules::FlushTrigger::Queued)
                .await
            {
                crate::fc_warn!("outbox: flush failed ({})", log::store_error(&error));
            }
            if let Some(page) = weak.upgrade() {
                page.schedule();
            }
        });
    }

    /// Reports the read when the reader is looking (the Mac's three facts).
    /// Shown, in the window in front, at the newest: the reader is reading this chat.
    pub fn is_being_read(&self) -> bool {
        self.opened.get()
            && !self.closed.get()
            && self.host.is_shown()
            && self.host.window().is_some_and(|window| window.is_active())
            && self.timeline.at_newest()
    }

    pub fn maybe_read(&self) {
        if !self.is_being_read() {
            return;
        }
        let (model, shell) = (Rc::clone(&self.model), Rc::clone(&self.shell));
        glib::spawn_future_local(async move {
            match model.read(None).await {
                Ok(true) => {
                    crate::fc_debug!("conversation: chat {} read", model.chat_id());
                    shell.refresh_attention();
                    crate::chats::list_changed().emit(&());
                }
                Ok(false) => {}
                Err(error) => {
                    crate::fc_warn!("read: the cache refused ({})", log::store_error(&error));
                }
            }
        });
    }

    /// Pages back when the reader reaches the top.
    fn older(&self) {
        if self.loading_older.get() || !self.opened.get() || !self.model.may_have_older() {
            return;
        }
        self.loading_older.set(true);
        self.timeline.set_loading_older(true);
        let weak = self.me.clone();
        let model = Rc::clone(&self.model);
        glib::spawn_future_local(async move {
            if let Ok(Some(error)) = model.older().await {
                crate::fc_info!("conversation: an older page failed ({})", error.code);
            }
            if let Some(page) = weak.upgrade() {
                page.redraw();
                page.loading_older.set(false);
                page.timeline.set_loading_older(false);
            }
        });
    }

    /// The page leaves (another chat, the window closed, the gate moved): the draft kept, the
    /// handlers taken off, the header buttons taken away, a recording interrupted.
    pub fn close(&self) {
        if self.closed.replace(true) {
            return;
        }
        self.subs.clear();
        if let Some(source) = self.typing_tick.borrow_mut().take() {
            source.remove();
        }
        self.composer.close();
        self.call_buttons.borrow_mut().clear();
        if let Some(header) = self.host.header() {
            for widget in self.header_widgets.borrow_mut().drain(..) {
                header.remove(&widget);
            }
        }
        if let Some(thread) = self.thread.borrow_mut().take() {
            thread.close();
        }
        self.host.set_title("", "");
    }

    /// The bubbles' and menus' facts for `message`.
    /// `revealed`: the surface showing it (this page, or a thread) was asked to show it after all.
    fn menu_facts(&self, message: &MessageDto, in_thread: bool, revealed: bool) -> MenuFacts {
        let chats = &self.connection.chats;
        let me = chats.reader();
        let chat = self.chat();
        let assistant_chat = chat
            .as_ref()
            .is_some_and(|chat| chat.kind == chat_kinds::ASSISTANT);
        let assistant = self
            .connection
            .session
            .state()
            .assistant
            .map(|assistant| assistant.user_id);
        let bubble = Bubble::new(message.clone(), false, false, message.sender_id == me);
        let reactions: Vec<Reaction> = message
            .reactions
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .map(|r| Reaction::new(r.user_id, &r.emoji))
            .collect();
        let blocked =
            message.sender_id != me && chats.is_blocked(message.sender_id).unwrap_or(false);
        let safety = if bubble_rules::may_report_assistant(message, me, assistant_chat, assistant) {
            Safety::Assistant
        } else if bubble_rules::is_other_member(message, me, assistant_chat, assistant) {
            Safety::Member {
                may_report: bubble_rules::may_report(message, me, assistant_chat, assistant),
                blocked,
            }
        } else {
            Safety::None
        };
        MenuFacts {
            message: message.clone(),
            may_edit: ConversationModel::may_edit(&bubble),
            in_thread,
            offers_text: crate::composer::integration::offers_text(message),
            my_reaction: my_reaction(&reactions, me).map(str::to_string),
            hidden: blocked && !revealed,
            safety,
        }
    }

    fn message(&self, id: i64) -> Option<MessageDto> {
        self.connection.chats.message(id).ok().flatten()
    }

    /// A refusal or a failure, said under the composer.
    pub fn say_problem(&self, text: &str) {
        self.composer.notice(text);
    }

    /// The thread of `message_id`, as the Mac's sheet.
    pub fn open_thread(self: &Rc<Self>, message_id: i64) {
        let open = self.thread.borrow_mut().take();
        if let Some(open) = open {
            open.close();
        }
        if let Some(dialog) = thread::ThreadDialog::open(self, message_id) {
            *self.thread.borrow_mut() = Some(dialog);
        }
    }

    pub fn shell(&self) -> &Rc<Shell> {
        &self.shell
    }

    /// The thread open over this page, if any.
    pub fn thread(&self) -> Option<Rc<thread::ThreadDialog>> {
        self.thread.borrow().clone()
    }

    /// The thread's dialog closed: let it go.
    pub fn thread_closed(&self) {
        self.thread.borrow_mut().take();
    }

    pub fn host(&self) -> &Rc<dyn PageHost> {
        &self.host
    }

    /// The facts the thread's bubbles share, from this page's.
    pub fn thread_context(
        &self,
        host: Rc<dyn BubbleHost>,
        revealed: Rc<dyn Fn(i64, QuoteLevel) -> bool>,
    ) -> Option<BubbleContext> {
        let chat = self.chat()?;
        let mut ctx = self.context(chat);
        ctx.in_thread = true;
        ctx.host = host;
        ctx.quote_revealed = revealed;
        Some(ctx)
    }

    /// What the Open polls dialog draws its cards from: this chat's, with the dialog's own `host`.
    pub fn polls_context(&self, host: Rc<dyn BubbleHost>) -> Option<BubbleContext> {
        let chat = self.chat()?;
        let mut ctx = self.context(chat);
        ctx.host = host;
        Some(ctx)
    }

    /// The Open polls dialog redraws with the page while it is open (`None`: it closed).
    pub fn set_polls_redraw(&self, redraw: Option<Rc<dyn Fn()>>) {
        *self.polls_redraw.borrow_mut() = redraw;
    }

    /// A message's menu, as the thread asks for it.
    pub fn thread_menu_facts(&self, message: &MessageDto, revealed: bool) -> MenuFacts {
        self.menu_facts(message, true, revealed)
    }

    // ---- what the bubbles and the menu ask -----------------------------------------------------

    pub fn react(&self, message_id: i64, emoji: &str) {
        let (model, weak, emoji) = (Rc::clone(&self.model), self.me.clone(), emoji.to_string());
        glib::spawn_future_local(async move {
            let answer = model.react(message_id, &emoji).await;
            let Some(page) = weak.upgrade() else {
                return;
            };
            match answer {
                Ok(Some(error)) => {
                    crate::fc_info!("react: refused ({})", error.code);
                    page.say_problem(
                        page.shell
                            .say
                            .get("Couldn't change that right now. Try again."),
                    );
                }
                Ok(None) => {}
                Err(error) => {
                    crate::fc_warn!("react: the cache refused ({})", log::store_error(&error));
                }
            }
            page.schedule();
        });
    }

    fn quote_clicked(&self, message_id: i64, quote: &Quote) {
        match quotes::click_on(quote) {
            QuoteClick::RevealReply => self.model.reveal_quote(message_id, QuoteLevel::Reply),
            QuoteClick::RevealParent => self.model.reveal_quote(message_id, QuoteLevel::Parent),
            QuoteClick::GoToMessage => {
                let target = quote.reply.message_id;
                match self.model.draw_to(target) {
                    Ok(true) => {
                        self.redraw();
                        self.timeline
                            .scroll_to(&conversation_rows::message_key(target), true);
                        return;
                    }
                    Ok(false) => {
                        self.say_problem(self.shell.say.get("Not available for this message."));
                    }
                    Err(error) => {
                        crate::fc_warn!("quote: the cache refused ({})", log::store_error(&error));
                    }
                }
            }
        }
        self.schedule();
    }

    pub fn vote(&self, message_id: i64, option_id: i64) {
        let (model, weak) = (Rc::clone(&self.model), self.me.clone());
        glib::spawn_future_local(async move {
            let answer = model.vote(message_id, option_id).await;
            if let Some(page) = weak.upgrade() {
                if let Ok(Some(error)) = answer {
                    crate::fc_info!("vote: refused ({})", error.code);
                    page.say_problem(
                        page.shell
                            .say
                            .get("Couldn't change that right now. Try again."),
                    );
                }
                page.schedule();
            }
        });
    }

    fn close_poll(&self, message_id: i64) {
        self.close_poll_over(message_id, self.root.upcast_ref());
    }

    /// "Close poll", asked over `parent` — this page, or the thread dialog the card is drawn in.
    pub fn close_poll_over(&self, message_id: i64, parent: &gtk::Widget) {
        let say = &self.shell.say;
        let dialog = adw::AlertDialog::new(
            Some(say.get("Close poll")),
            Some(say.get("Ends the poll. This cannot be undone.")),
        );
        dialog.add_response("cancel", say.get("Cancel"));
        dialog.add_response("close", say.get("Close poll"));
        dialog.set_response_appearance("close", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let (model, weak) = (Rc::clone(&self.model), self.me.clone());
        dialog.connect_response(None, move |_, response| {
            if response != "close" {
                return;
            }
            let (model, weak) = (Rc::clone(&model), weak.clone());
            glib::spawn_future_local(async move {
                let answer = model.close_poll(message_id).await;
                if let Some(page) = weak.upgrade() {
                    if let Ok(Some(error)) = answer {
                        crate::fc_info!("close poll: refused ({})", error.code);
                    }
                    page.schedule();
                }
            });
        });
        dialog.present(Some(parent));
    }

    /// A mention that opens: the direct chat with that member (made when there is none yet).
    pub fn open_member(&self, user_id: i64) {
        let connection = Rc::clone(&self.connection);
        glib::spawn_future_local(async move {
            match connection.api.direct_chat(user_id).await.into_result() {
                Ok(answer) => {
                    if let Err(error) = connection.chats.upsert(&answer.chat) {
                        crate::fc_warn!(
                            "direct chat: the cache refused ({})",
                            log::store_error(&error)
                        );
                        return;
                    }
                    Shell::get().chat_routes.get().open_chat(answer.chat.id);
                }
                Err(error) => crate::fc_info!("direct chat: refused ({})", error.code),
            }
        });
    }

    /// Who reacted, beside `anchor`: one row per emoji — `only` that one, from its chip, or all of
    /// them ("See who reacted") — with the first person's face and every name; the reader's own
    /// row carries the one way a reaction comes off, so it never happens by accident (the Mac's
    /// `reactorList`). Blocked reactors are not named (`reaction_details`).
    pub fn reactors(&self, message: &MessageDto, only: Option<&str>, anchor: &gtk::Widget) {
        let say = &self.shell.say;
        let chats = &self.connection.chats;
        let me = chats.reader();
        let reactions: Vec<Reaction> = message
            .reactions
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .map(|r| Reaction::new(r.user_id, &r.emoji))
            .collect();
        let names: HashMap<i64, String> = chats
            .members()
            .map(|members| {
                members
                    .iter()
                    .map(|m| (m.id, m.display_name.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let blocked: HashSet<i64> = chats
            .blocked()
            .map(|b| b.iter().copied().collect())
            .unwrap_or_default();
        let details = fc_text::reactions::reaction_details(&reactions, &names, me, &blocked);
        let column = gtk::Box::new(gtk::Orientation::Vertical, 8);
        column.set_margin_top(8);
        column.set_margin_bottom(8);
        column.set_margin_start(8);
        column.set_margin_end(8);
        let popover = gtk::Popover::new();
        for detail in details
            .iter()
            .filter(|detail| only.is_none_or(|emoji| detail.emoji == emoji))
        {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            if let Some(lead) = detail.lead_user_id {
                let name = detail.names.first().map_or("?", String::as_str);
                line.append(&crate::chats::avatar::avatar(
                    name,
                    fc_logic::groups::ChatFace::Initials,
                    Some(lead),
                    24,
                ));
            }
            line.append(&gtk::Label::new(Some(&detail.emoji)));
            line.append(
                &gtk::Label::builder()
                    .label(fc_logic::lookups::names(&detail.names, say.as_ref()))
                    .hexpand(true)
                    .xalign(0.0)
                    .wrap(true)
                    .max_width_chars(32)
                    .css_classes(["dim-label"])
                    .build(),
            );
            // "You" leads its emoji: the reader's own row is the remove control.
            if detail.lead_user_id == Some(me) {
                let remove = gtk::Button::builder()
                    .label(say.get("Remove reaction"))
                    .css_classes(["flat"])
                    .valign(gtk::Align::Center)
                    .build();
                let (weak, id, emoji, popover) = (
                    self.me.clone(),
                    message.id,
                    detail.emoji.clone(),
                    popover.clone(),
                );
                remove.connect_clicked(move |_| {
                    popover.popdown();
                    if let Some(page) = weak.upgrade() {
                        page.react(id, &emoji);
                    }
                });
                line.append(&remove);
            }
            column.append(&line);
        }
        popover.set_child(Some(&column));
        popover.set_parent(anchor);
        popover.connect_closed(|popover| {
            let popover = popover.clone();
            glib::idle_add_local_once(move || popover.unparent());
        });
        popover.popup();
    }

    /// "See who reacted" over the message's own row, wherever `timeline` draws it.
    pub fn all_reactors(&self, message_id: i64, timeline: &Timeline) {
        let (Some(message), Some(row)) = (
            self.message(message_id),
            timeline.row(&conversation_rows::message_key(message_id)),
        ) else {
            return;
        };
        self.reactors(&message, None, &row);
    }
}

impl BubbleHost for Host {
    fn react(&self, message_id: i64, emoji: &str) {
        if let Some(page) = self.0.upgrade() {
            page.react(message_id, emoji);
        }
    }
    fn reveal(&self, message_id: i64) {
        if let Some(page) = self.0.upgrade() {
            page.model.reveal(message_id);
            page.schedule();
        }
    }
    fn quote_clicked(&self, message_id: i64, quote: &Quote) {
        if let Some(page) = self.0.upgrade() {
            page.quote_clicked(message_id, quote);
        }
    }
    fn open_thread(&self, message_id: i64) {
        if let Some(page) = self.0.upgrade() {
            page.open_thread(message_id);
        }
    }
    fn vote(&self, message_id: i64, option_id: i64) {
        if let Some(page) = self.0.upgrade() {
            page.vote(message_id, option_id);
        }
    }
    fn close_poll(&self, message_id: i64) {
        if let Some(page) = self.0.upgrade() {
            page.close_poll(message_id);
        }
    }
    fn open_member(&self, user_id: i64) {
        if let Some(page) = self.0.upgrade() {
            page.open_member(user_id);
        }
    }
    fn menu(&self, message: &MessageDto, anchor: &gtk::Widget, x: f64, y: f64) {
        if let Some(page) = self.0.upgrade() {
            let facts = page.menu_facts(message, false, page.model.revealed(message.id));
            menu::popup(&facts, "win", anchor, x, y);
        }
    }
    fn reactors(&self, message: &MessageDto, emoji: &str, anchor: &gtk::Widget) {
        if let Some(page) = self.0.upgrade() {
            page.reactors(message, Some(emoji), anchor);
        }
    }
}

impl MessageActions for ConversationPage {
    fn reply(&self, message_id: i64) {
        self.composer.set_reply(Some(message_id));
        self.composer.focus();
    }

    fn react(&self, message_id: i64, emoji: &str) {
        ConversationPage::react(self, message_id, emoji);
    }

    fn show_text(&self, message_id: i64) {
        crate::composer::integration::show_text(self, message_id);
    }

    fn playback_speed(&self, _message_id: i64) {
        crate::composer::integration::next_voice_speed(&self.shell);
    }

    fn save(&self, message_id: i64) {
        if let Some(message) = self.message(message_id) {
            safety::save_attachment(&self.root, &message);
        }
    }

    fn open_full_screen(&self, message_id: i64) {
        if let Some(message) = self.message(message_id) {
            crate::composer::integration::open_full_screen(&message, 0);
        }
    }

    fn copy(&self, message_id: i64) {
        if let Some(message) = self.message(message_id) {
            self.root.clipboard().set_text(&message.body);
        }
    }

    fn view_thread(&self, message_id: i64) {
        if let Some(page) = self.me.upgrade() {
            page.open_thread(message_id);
        }
    }

    fn reactors(&self, message_id: i64) {
        self.all_reactors(message_id, &self.timeline);
    }

    fn edit(&self, message_id: i64) {
        if let Some(message) = self.message(message_id) {
            let bubble = Bubble::new(
                message.clone(),
                false,
                false,
                message.sender_id == self.connection.chats.reader(),
            );
            if ConversationModel::may_edit(&bubble) {
                self.composer.start_edit(message_id, &message.body);
            }
        }
    }

    fn delete(&self, token: i64) {
        let key = self
            .pending_tokens
            .borrow()
            .iter()
            .find(|(_, t)| **t == token)
            .map(|(key, _)| key.clone());
        let Some(key) = key else {
            // The protocol deletes no message: only a send that never reached the server.
            return;
        };
        match self.model.discard(&key) {
            Ok(_) => crate::fc_info!("outbox: a send was given up"),
            Err(error) => {
                crate::fc_warn!("outbox: the cache refused ({})", log::store_error(&error))
            }
        }
        self.pending_tokens.borrow_mut().remove(&key);
        self.schedule();
    }

    fn report(&self, message_id: i64) {
        if let Some(message) = self.message(message_id) {
            let facts = self.menu_facts(&message, false, self.model.revealed(message.id));
            safety::report(
                &self.root,
                &self.connection,
                &message,
                facts.safety == Safety::Assistant,
            );
        }
    }

    fn block(&self, message_id: i64) {
        if let Some(message) = self.message(message_id) {
            safety::block(&self.root, &self.connection, message.sender_id);
        }
    }
}

/// The line an empty chat draws (the Mac's `ContentUnavailableView`).
fn empty_row(shell: &Shell) -> gtk::Widget {
    let say = &shell.say;
    adw::StatusPage::builder()
        .icon_name("chat-message-new-symbolic")
        .title(say.get("No messages yet"))
        .description(say.get("Say something to get started."))
        .css_classes(["compact"])
        .vexpand(true)
        .build()
        .upcast()
}
