//! A thread (shell-design §5.3): the Mac's sheet (`MacConversationView`'s `ThreadView`) as an
//! `adw::Dialog` over the conversation — content 520 × 640 — showing the root and its replies
//! through `fc_logic::threads::ThreadModel` (opened at once on what the cache holds, then read; its
//! rows kept out of the cache) with its own composer: the capsule and the Send disc, Send
//! insensitive while blank, no paperclip and no microphone (`composer-and-wide-layout` §2). Every
//! send answers the root. Escape with words in the field clears focus first and closes only on a
//! second Escape (the web's fix). Its rows' menus use the `thread.` prefix, so they act here.

use std::cell::Cell;
use std::rc::{Rc, Weak};

use adw::prelude::*;
use fc_core::protocol::MessageDto;
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::conversation_rows::{self, RowMark};
use fc_logic::quotes::{self, Quote, QuoteClick, QuoteLevel};
use fc_logic::threads::ThreadModel;
use fc_logic::timeline;

use super::bubble::{self, BubbleHost};
use super::menu::{self, MessageActions, Safety};
use super::page::ConversationPage;
use super::safety;
use super::timeline::Timeline;
use crate::composer::{Composer, ComposerKind};
use crate::services::log;

/// The thread over one conversation page.
pub struct ThreadDialog {
    page: Weak<ConversationPage>,
    model: Rc<ThreadModel>,
    dialog: adw::Dialog,
    timeline: Rc<Timeline>,
    composer: Rc<Composer>,
    queued: Cell<bool>,
    me: Weak<ThreadDialog>,
}

impl std::fmt::Debug for ThreadDialog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ThreadDialog").finish_non_exhaustive()
    }
}

struct ThreadHost(Weak<ThreadDialog>);

impl ThreadDialog {
    /// Opens the thread `message_id` is in (or roots), over `page`.
    pub fn open(page: &Rc<ConversationPage>, message_id: i64) -> Option<Rc<ThreadDialog>> {
        let connection = page.connection();
        let shell = page.shell();
        let say = &shell.say;
        let model = match ThreadModel::new(
            page.chat_id,
            message_id,
            Rc::clone(&connection.chats),
            Rc::clone(&connection.api),
        ) {
            Ok(model) => Rc::new(model.with_outbox(Rc::clone(&connection.outbox))),
            Err(error) => {
                crate::fc_warn!("thread: the cache refused ({})", log::store_error(&error));
                return None;
            }
        };
        let chat = connection.chats.chat(page.chat_id).ok().flatten()?.chat;
        let dialog = adw::Dialog::builder()
            .title(say.get("Thread"))
            .content_width(520)
            .content_height(640)
            .follows_content_size(false)
            .build();
        let thread = Rc::new_cyclic(|me: &Weak<ThreadDialog>| {
            let timeline = Timeline::new(say.get("Jump to the newest message"));
            let on_sent = {
                let me = me.clone();
                Rc::new(move || {
                    if let Some(thread) = me.upgrade() {
                        thread.timeline.scroll_to_bottom();
                        thread.schedule();
                    }
                }) as Rc<dyn Fn()>
            };
            let window = {
                let dialog = dialog.clone();
                Rc::new(move || {
                    dialog
                        .root()
                        .and_then(|root| root.downcast::<gtk::Window>().ok())
                }) as Rc<dyn Fn() -> Option<gtk::Window>>
            };
            let composer = Composer::new(
                shell,
                connection,
                Rc::clone(page.model()),
                chat.clone(),
                ComposerKind::Thread {
                    root_id: model.root_id(),
                },
                window,
                on_sent,
                Rc::new(|_| {}),
            );
            ThreadDialog {
                page: Rc::downgrade(page),
                model: Rc::clone(&model),
                dialog: dialog.clone(),
                timeline,
                composer,
                queued: Cell::new(false),
                me: me.clone(),
            }
        });
        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.append(thread.timeline.widget());
        let composer_box = thread.composer.widget();
        // The timeline's own inset, so the field lines up with the bubbles above it.
        let inset = fc_logic::composer_row::conversation_column::PANE_INSET as i32;
        composer_box.set_margin_start(inset);
        composer_box.set_margin_end(inset);
        column.append(&composer_box);
        let view = adw::ToolbarView::new();
        view.add_top_bar(&adw::HeaderBar::new());
        view.set_content(Some(&column));
        dialog.set_child(Some(&view));
        // Its rows' menus.
        let group = gio::SimpleActionGroup::new();
        {
            let weak = Rc::downgrade(&thread);
            menu::install(
                &group,
                Rc::new(move || weak.upgrade().map(|t| t as Rc<dyn MessageActions>)),
            );
        }
        dialog.insert_action_group("thread", Some(&group));
        // Escape with a draft: clear focus first, close on the second.
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        {
            let weak = Rc::downgrade(&thread);
            keys.connect_key_pressed(move |_, key, _, _| {
                if key != gtk::gdk::Key::Escape {
                    return glib::Propagation::Proceed;
                }
                let Some(thread) = weak.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                if thread.composer.field().has_focus() && !thread.composer.draft().trim().is_empty()
                {
                    thread.dialog.set_focus(None::<&gtk::Widget>);
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
        }
        dialog.add_controller(keys);
        {
            let weak = Rc::downgrade(&thread);
            dialog.connect_closed(move |_| {
                if let Some(thread) = weak.upgrade() {
                    thread.composer.close();
                    if let Some(page) = thread.page.upgrade() {
                        page.thread_closed();
                    }
                }
            });
        }
        thread.redraw();
        thread.timeline.scroll_to_bottom();
        {
            let weak = Rc::downgrade(&thread);
            glib::spawn_future_local(async move {
                let Some(model) = weak.upgrade().map(|t| Rc::clone(&t.model)) else {
                    return;
                };
                let failed = model.load().await;
                let Some(thread) = weak.upgrade() else {
                    return;
                };
                if let Ok(Some(error)) = failed {
                    crate::fc_info!("thread: the read failed ({})", error.code);
                    thread
                        .composer
                        .notice(thread.composer.say().get("Couldn't load the whole thread."));
                }
                thread.composer.set_root(thread.model.root_id());
                thread.redraw();
                thread.timeline.scroll_to_bottom();
            });
        }
        let parent = page.host().window();
        dialog.present(parent.as_ref());
        thread.composer.focus();
        crate::fc_info!("thread: opened in chat {}", page.chat_id);
        Some(thread)
    }

    /// The composer (the self-test types into it).
    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "the chat self-test reads it")
    )]
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

    pub fn close(&self) {
        self.dialog.close();
    }

    /// A redraw on the next idle turn.
    pub fn schedule(&self) {
        if self.queued.replace(true) {
            return;
        }
        let weak = self.me.clone();
        glib::idle_add_local_once(move || {
            if let Some(thread) = weak.upgrade() {
                thread.queued.set(false);
                thread.redraw();
            }
        });
    }

    fn redraw(&self) {
        let Some(page) = self.page.upgrade() else {
            return;
        };
        let bubbles = match self.model.bubbles() {
            Ok(bubbles) => bubbles,
            Err(error) => {
                crate::fc_warn!("thread: the cache refused ({})", log::store_error(&error));
                return;
            }
        };
        let host: Rc<dyn BubbleHost> = Rc::new(ThreadHost(self.me.clone()));
        let model = Rc::clone(&self.model);
        let revealed: Rc<dyn Fn(i64, QuoteLevel) -> bool> =
            Rc::new(move |id, level| model.quote_revealed(id, level));
        let Some(ctx) = page.thread_context(host, revealed) else {
            return;
        };
        let rows = timeline::rows(
            bubbles,
            ctx.shared,
            ctx.me,
            None,
            page.shell().zone.as_ref(),
        );
        let pending = self.model.pending().unwrap_or_default();
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
        for row in &pending {
            marks.push(RowMark::new(
                conversation_rows::pending_key(&row.client_msg_id),
                format!("{}|{}", row.body, row.failed()),
            ));
        }
        let message_rows = rows.len();
        let say = Rc::clone(&page.shell().say);
        self.timeline.update(marks, |index| {
            if index < message_rows {
                bubble::row_widget(&ctx, &rows[index], joins[index])
            } else {
                let row = &pending[index - message_rows];
                let label = gtk::Label::builder()
                    .label(format!("{} · {}", row.body, say.get("Sending…")))
                    .css_classes(["fc-chat-pending"])
                    .halign(gtk::Align::End)
                    .wrap(true)
                    .build();
                label.upcast()
            }
        });
    }

    fn message(&self, id: i64) -> Option<MessageDto> {
        self.model
            .bubbles()
            .ok()?
            .into_iter()
            .map(|bubble| bubble.message)
            .find(|message| message.id == id)
    }

    fn react(&self, message_id: i64, emoji: &str) {
        let (model, weak, emoji) = (Rc::clone(&self.model), self.me.clone(), emoji.to_string());
        glib::spawn_future_local(async move {
            if let Ok(Some(error)) = model.react(message_id, &emoji).await {
                crate::fc_info!("thread react: refused ({})", error.code);
            }
            if let Some(thread) = weak.upgrade() {
                thread.schedule();
                if let Some(page) = thread.page.upgrade() {
                    page.schedule();
                }
            }
        });
    }
}

impl BubbleHost for ThreadHost {
    fn react(&self, message_id: i64, emoji: &str) {
        if let Some(thread) = self.0.upgrade() {
            thread.react(message_id, emoji);
        }
    }
    fn reveal(&self, message_id: i64) {
        if let Some(thread) = self.0.upgrade() {
            thread.model.reveal(message_id);
            thread.schedule();
        }
    }
    fn quote_clicked(&self, message_id: i64, quote: &Quote) {
        let Some(thread) = self.0.upgrade() else {
            return;
        };
        match quotes::click_on(quote) {
            QuoteClick::RevealReply => thread.model.reveal_quote(message_id, QuoteLevel::Reply),
            QuoteClick::RevealParent => thread.model.reveal_quote(message_id, QuoteLevel::Parent),
            QuoteClick::GoToMessage => {
                let target = quote.reply.message_id;
                if !thread
                    .timeline
                    .scroll_to(&conversation_rows::message_key(target), true)
                {
                    thread
                        .composer
                        .notice(thread.composer.say().get("Not available for this message."));
                }
            }
        }
        thread.schedule();
    }
    fn open_thread(&self, _message_id: i64) {}
    fn vote(&self, message_id: i64, option_id: i64) {
        if let Some(page) = self.0.upgrade().and_then(|t| t.page.upgrade()) {
            BubbleHostForwarder(&page).vote(message_id, option_id);
        }
    }
    fn close_poll(&self, _message_id: i64) {}
    fn open_member(&self, user_id: i64) {
        if let Some(page) = self.0.upgrade().and_then(|t| t.page.upgrade()) {
            page.open_member(user_id);
        }
    }
    fn menu(&self, message: &MessageDto, anchor: &gtk::Widget, x: f64, y: f64) {
        if let Some(page) = self.0.upgrade().and_then(|t| t.page.upgrade()) {
            let facts = page.thread_menu_facts(message);
            menu::popup(&facts, "thread", anchor, x, y);
        }
    }
    fn reactors(&self, message: &MessageDto, emoji: &str, _anchor: &gtk::Widget) {
        // In the thread a chip the reader is part of takes their reaction off — the reactor list
        // is the conversation's.
        self.react(message.id, emoji);
    }
}

/// The page's own vote, from the thread.
struct BubbleHostForwarder<'a>(&'a Rc<ConversationPage>);

impl BubbleHostForwarder<'_> {
    fn vote(&self, message_id: i64, option_id: i64) {
        let model = Rc::clone(self.0.model());
        let page = Rc::downgrade(self.0);
        glib::spawn_future_local(async move {
            let _ = model.vote(message_id, option_id).await;
            if let Some(page) = page.upgrade() {
                page.schedule();
            }
        });
    }
}

impl MessageActions for ThreadDialog {
    fn reply(&self, message_id: i64) {
        self.composer.set_reply(Some(message_id));
        self.composer.focus();
    }
    fn react(&self, message_id: i64, emoji: &str) {
        ThreadDialog::react(self, message_id, emoji);
    }
    fn show_text(&self, message_id: i64) {
        if let Some(page) = self.page.upgrade() {
            page.show_text(message_id);
        }
    }
    fn playback_speed(&self, message_id: i64) {
        if let Some(page) = self.page.upgrade() {
            page.playback_speed(message_id);
        }
    }
    fn save(&self, message_id: i64) {
        if let Some(message) = self.message(message_id) {
            safety::save_attachment(&self.dialog, &message);
        }
    }
    fn open_full_screen(&self, message_id: i64) {
        if let Some(message) = self.message(message_id) {
            crate::composer::integration::open_full_screen(&message, 0);
        }
    }
    fn copy(&self, message_id: i64) {
        if let Some(message) = self.message(message_id) {
            self.dialog.clipboard().set_text(&message.body);
        }
    }
    fn view_thread(&self, _message_id: i64) {}
    fn edit(&self, _message_id: i64) {}
    fn delete(&self, token: i64) {
        if let Some(page) = self.page.upgrade() {
            page.delete(token);
        }
    }
    fn report(&self, message_id: i64) {
        if let (Some(page), Some(message)) = (self.page.upgrade(), self.message(message_id)) {
            let facts = page.thread_menu_facts(&message);
            safety::report(
                &self.dialog,
                page.connection(),
                &message,
                facts.safety == Safety::Assistant,
            );
        }
    }
    fn block(&self, message_id: i64) {
        if let (Some(page), Some(message)) = (self.page.upgrade(), self.message(message_id)) {
            safety::block(&self.dialog, page.connection(), message.sender_id);
        }
    }
}
