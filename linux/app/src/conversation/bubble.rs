//! One drawn row of a conversation (shell-design §5.2): the day pill, the unread divider, the
//! sender's face and name, the bubble and the reactions under it — every decision fc-logic's
//! (`timeline`, `bubble_body`, `bubble_rules`, `quotes`, `doorway::bubble_text`, `poll_text`,
//! `assistant_answers`), every block that is not words drawn by P2 and P3 through
//! `Shell::slots()` (tiles, voice, circles, stickers, rich blocks, link cards, places).
//!
//! The row is focusable (Up/Down walk the bubbles, §24) and an `AccessibleRole::Group` named
//! "<sender>, <time>: <excerpt>" (Windows' `NamedGroup`). Its menu opens on a secondary click, a
//! long press, Shift+F10 or the Menu key; a double click is the heart (Windows' rule).

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use fc_core::protocol::{ChatDto, MemberDto, MessageDto};
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::bubble_body::{self, BodyBlock};
use fc_logic::bubble_rules;
use fc_logic::chat_list::ChatListModel;
use fc_logic::conversation::Bubble;
use fc_logic::doorway::bubble_text;
use fc_logic::groups::{ChatFace, chat_kinds};
use fc_logic::lookups;
use fc_logic::message_menu::{self, RecordingKind};
use fc_logic::poll_text;
use fc_logic::quotes::{self, Quote, QuoteLevel};
use fc_logic::rich_drawing::RichLook;
use fc_logic::timeline::{self, TimelineRow};
use fc_text::reactions::{ChipTap, Reaction, reaction_chips};

use super::markup::{self, Piece};
use crate::chats::avatar;
use crate::seams::conversation::ConversationSlots;
use crate::shell::{Connection, Shell};

/// What a bubble asks of the surface it is drawn on (the conversation page, the thread dialog).
pub trait BubbleHost {
    /// Toggle the reader's `emoji` on a message (held → DELETE, else PUT).
    fn react(&self, message_id: i64, emoji: &str);
    /// Show a hidden bubble after all.
    fn reveal(&self, message_id: i64);
    /// A click on a quote (`quotes::click_on` decides what it does).
    fn quote_clicked(&self, message_id: i64, quote: &Quote);
    /// "%lld replies": the thread.
    fn open_thread(&self, message_id: i64);
    /// A tap on a poll's option, and its author's "Close poll".
    fn vote(&self, message_id: i64, option_id: i64);
    fn close_poll(&self, message_id: i64);
    /// A mention that opens: the direct chat with that member.
    fn open_member(&self, user_id: i64);
    /// The message menu at (`x`, `y`) of `anchor`.
    fn menu(&self, message: &MessageDto, anchor: &gtk::Widget, x: f64, y: f64);
    /// Who reacted with `emoji` (a chip the reader is part of).
    fn reactors(&self, message: &MessageDto, emoji: &str, anchor: &gtk::Widget);
}

/// What every row of one drawing shares.
pub struct BubbleContext {
    pub shell: Rc<Shell>,
    pub connection: Rc<Connection>,
    pub chat: ChatDto,
    pub me: i64,
    pub shared: bool,
    pub assistant_chat: bool,
    pub assistant_user_id: Option<i64>,
    pub members: Rc<[MemberDto]>,
    pub blocked: Rc<[i64]>,
    pub peer_read_up_to: i64,
    pub in_thread: bool,
    pub host: Rc<dyn BubbleHost>,
    pub quote_revealed: Rc<dyn Fn(i64, QuoteLevel) -> bool>,
    pub list: Rc<ChatListModel>,
}

impl BubbleContext {
    fn is_blocked(&self, user_id: i64) -> bool {
        self.blocked.contains(&user_id)
    }

    /// What, beyond the message itself and its place, a row is drawn from — the rest of its
    /// `conversation_rows::signature`.
    pub fn drawn_with(&self, bubble: &Bubble) -> String {
        let message = &bubble.message;
        let answers = &self.connection.answers;
        let seen = bubble_rules::seen(message, self.me, self.shared, self.peer_read_up_to);
        let reply = (self.quote_revealed)(message.id, QuoteLevel::Reply);
        let parent = (self.quote_revealed)(message.id, QuoteLevel::Parent);
        format!(
            "{}|{}|{}|{}|{}|{}",
            u8::from(seen),
            u8::from(reply),
            u8::from(parent),
            u8::from(answers.is_writing(message)),
            u8::from(answers.failed(message)),
            answers.body_of(message)
        )
    }
}

/// The time on the bubble's footer.
fn time_text(ctx: &BubbleContext, message: &MessageDto) -> String {
    bubble_text::when(
        message,
        &ctx.shell.culture,
        ctx.shell.zone.as_ref(),
        ctx.shell.say.as_ref(),
    )
}

/// The name a sender is drawn as.
fn sender_name(ctx: &BubbleContext, bubble: &Bubble) -> String {
    if bubble_rules::is_assistant(
        &bubble.message,
        ctx.me,
        ctx.assistant_chat,
        ctx.assistant_user_id,
    ) {
        if let Some(name) = ctx
            .connection
            .session
            .state()
            .assistant
            .map(|assistant| assistant.display_name)
            .filter(|name| !name.is_empty())
        {
            return name;
        }
        if ctx.assistant_chat {
            return ctx.chat.title.clone();
        }
    }
    bubble_text::sender(bubble, &ctx.connection.chats, ctx.shell.say.as_ref()).unwrap_or_default()
}

/// The whole row for `row`.
pub fn row_widget(ctx: &BubbleContext, row: &TimelineRow) -> gtk::Widget {
    let say = &ctx.shell.say;
    let bubble = &row.bubble;
    let message = &bubble.message;
    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .accessible_role(gtk::AccessibleRole::Group)
        .css_classes(["fc-chat-row"])
        .build();
    if row.run_start {
        column.add_css_class("fc-chat-turn");
    }
    if let Some(day) = row.day_above {
        let today = timeline_today(ctx);
        let pill = gtk::Label::builder()
            .label(timeline::day_label(
                day,
                today,
                &ctx.shell.culture,
                say.as_ref(),
            ))
            .css_classes(["fc-chat-day"])
            .halign(gtk::Align::Center)
            .build();
        column.append(&pill);
    }
    if row.unread_divider_above {
        let unread = ctx
            .connection
            .chats
            .chat(ctx.chat.id)
            .ok()
            .flatten()
            .map_or(0, |chat| i64::from(chat.unread_count));
        let divider = gtk::Label::builder()
            .label(timeline::divider_text(unread.max(1), say.as_ref()))
            .css_classes(["fc-chat-divider"])
            .halign(gtk::Align::Center)
            .build();
        column.append(&divider);
    }
    let name = sender_name(ctx, bubble);
    if row.shows_sender {
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        line.add_css_class("fc-chat-sender");
        line.append(&avatar::avatar(
            &name,
            ChatFace::Initials,
            Some(message.sender_id),
            20,
        ));
        line.append(&gtk::Label::new(Some(&name)));
        column.append(&line);
    }
    let (body, words) = bubble_body_widget(ctx, bubble);
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    let clamp = adw::Clamp::builder()
        .maximum_size(fc_logic::composer_row::conversation_column::BUBBLE_MAX as i32)
        .tightening_threshold(0)
        .child(&body)
        .build();
    let spacer = fc_logic::composer_row::conversation_column::BUBBLE_SPACER as i32;
    if bubble.mine {
        line.set_halign(gtk::Align::End);
        clamp.set_margin_start(spacer);
    } else {
        line.set_halign(gtk::Align::Start);
        clamp.set_margin_end(spacer);
    }
    line.append(&clamp);
    column.append(&line);
    if bubble.reads()
        && let Some(reactions) = reactions_row(ctx, message, bubble.mine)
    {
        column.append(&reactions);
    }
    // The row: focusable, a named group, with its menu and its heart.
    column.set_focusable(true);
    let time = time_text(ctx, message);
    let excerpt: String = words.chars().take(240).collect();
    column.update_property(&[gtk::accessible::Property::Label(&format!(
        "{name}, {time}: {excerpt}"
    ))]);
    attach_gestures(ctx, &column, &body, message);
    column.upcast()
}

/// "Today" is the reader's local day now.
fn timeline_today(ctx: &BubbleContext) -> time::Date {
    let now = time::OffsetDateTime::now_utc();
    fc_logic::date_culture::local(now, ctx.shell.zone.as_ref()).date()
}

/// The menu (secondary click, long press, Shift+F10, Menu key) and the heart (a double click).
fn attach_gestures(ctx: &BubbleContext, row: &gtk::Box, body: &gtk::Widget, message: &MessageDto) {
    let host = Rc::clone(&ctx.host);
    let secondary = gtk::GestureClick::builder().button(3).build();
    secondary.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let (host, message, body) = (Rc::clone(&host), message.clone(), body.clone());
        secondary.connect_pressed(move |gesture, _, x, y| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            host.menu(&message, &body, x, y);
        });
    }
    body.add_controller(secondary);
    let long = gtk::GestureLongPress::new();
    long.set_touch_only(true);
    {
        let (host, message, body) = (Rc::clone(&host), message.clone(), body.clone());
        long.connect_pressed(move |_, x, y| host.menu(&message, &body, x, y));
    }
    body.add_controller(long);
    if message.id != 0 {
        let primary = gtk::GestureClick::builder().button(1).build();
        primary.set_propagation_phase(gtk::PropagationPhase::Capture);
        {
            let (host, id) = (Rc::clone(&host), message.id);
            primary.connect_pressed(move |gesture, presses, _, _| {
                if presses == 2 {
                    gesture.set_state(gtk::EventSequenceState::Claimed);
                    host.react(id, fc_text::emoji::DOUBLE_TAP_REACTION);
                }
            });
        }
        body.add_controller(primary);
    }
    let keys = gtk::EventControllerKey::new();
    {
        let (message, body) = (message.clone(), body.clone());
        keys.connect_key_pressed(move |_, key, _, state| {
            let menu = key == gtk::gdk::Key::Menu
                || (key == gtk::gdk::Key::F10
                    && state.contains(gtk::gdk::ModifierType::SHIFT_MASK));
            if menu {
                let (w, h) = (f64::from(body.width()), f64::from(body.height()));
                host.menu(&message, &body, w / 2.0, h / 2.0);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
    }
    row.add_controller(keys);
}

/// The bubble itself, and the words a screen reader is told it says.
fn bubble_body_widget(ctx: &BubbleContext, bubble: &Bubble) -> (gtk::Widget, String) {
    let say = &ctx.shell.say;
    let message = &bubble.message;
    let words = bubble_text::words(bubble, &ctx.list, say.as_ref());
    let own = bubble.mine;
    let balloon = gtk::Box::new(gtk::Orientation::Vertical, 4);
    balloon.add_css_class("fc-chat-bubble");
    balloon.add_css_class(if own { "fc-own-bubble" } else { "fc-bubble" });

    if !bubble.reads() {
        let hidden = gtk::Button::builder()
            .label(say.get("Hidden — blocked member"))
            .css_classes(["flat", "fc-chat-hidden"])
            .tooltip_text(say.get("Click to show"))
            .build();
        hidden.update_property(&[gtk::accessible::Property::Label(
            say.get("Hidden message from a blocked member. Click to show it."),
        )]);
        let (host, id) = (Rc::clone(&ctx.host), message.id);
        hidden.connect_clicked(move |_| host.reveal(id));
        balloon.append(&hidden);
        return (balloon.upcast(), words);
    }

    let slots = ctx.shell.slots();
    // A sticker and a video message stand with no balloon (docs/protocol.md).
    if let Some(sticker) = message.sticker_picture() {
        let column = gtk::Box::new(gtk::Orientation::Vertical, 2);
        if let Some(quote) = quote_widget(ctx, message) {
            column.append(&quote);
        }
        column.append(&slots.sticker(message, sticker));
        column.append(&footer(ctx, message, own));
        return (column.upcast(), words);
    }
    if let Some(round) = message.round_video() {
        let column = gtk::Box::new(gtk::Orientation::Vertical, 2);
        if let Some(quote) = quote_widget(ctx, message) {
            column.append(&quote);
        }
        column.append(&slots.circle(message, round));
        column.append(&footer(ctx, message, own));
        return (column.upcast(), words);
    }

    if let Some(quote) = quote_widget(ctx, message) {
        balloon.append(&quote);
    }
    // Attachments.
    let media = message.media();
    if message_menu::recording(message) == Some(RecordingKind::Voice) {
        if let Some(voice) = media.first() {
            balloon.append(&slots.voice(message, voice, own));
        }
    } else if !media.is_empty() {
        // P2's layout of the whole block (Windows' `MediaElement`): one picture at its own shape or
        // the grid of four with "+n", an animation in a photo's box, then places and files — each
        // through the slots.
        balloon.append(&crate::attachments::tiles::media_block(message, own));
    }
    // A call record reads as a call.
    if let Some(call) = &message.call {
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        line.append(&gtk::Image::from_icon_name(if call.video {
            "camera-video-symbolic"
        } else {
            "call-start-symbolic"
        }));
        line.append(
            &gtk::Label::builder()
                .label(fc_text::call_record::label(
                    &call.outcome,
                    call.duration_secs.map(i64::from),
                    call.video,
                    own,
                ))
                .wrap(true)
                .xalign(0.0)
                .build(),
        );
        balloon.append(&line);
        // "Call back" — what a record is FOR, half the time (`call_records::call_back`): in a
        // direct chat while the server carries calls, never on a thread's panel.
        let state = ctx.connection.session.state();
        let back = fc_logic::call_records::call_back(
            call,
            ctx.chat.kind == chat_kinds::DIRECT,
            state.calls_enabled,
            state.video_calls_enabled,
            ctx.in_thread,
        );
        if back.offered
            && let Some(peer) = ctx.chat.peer_user_id
        {
            let say = &ctx.shell.say;
            let button = gtk::Button::builder()
                .label(say.get("Call back"))
                .css_classes(["flat", "fc-chat-call-back"])
                .halign(gtk::Align::Start)
                .sensitive(crate::calls::can_place(
                    &ctx.shell,
                    super::page::recording_now(),
                ))
                .build();
            let (chat_id, video) = (ctx.chat.id, back.video);
            button.connect_clicked(move |_| {
                let shell = Shell::get();
                if crate::calls::can_place(&shell, super::page::recording_now()) {
                    crate::calls::place(&shell, chat_id, peer, video);
                }
            });
            balloon.append(&button);
        }
    } else if let Some(poll) = &message.poll {
        balloon.append(&super::polls::card(ctx, message, poll));
    } else {
        append_body(ctx, &balloon, message, own);
    }
    balloon.append(&footer(ctx, message, own));
    if !ctx.in_thread
        && let Some(count) = message.reply_count.filter(|count| *count > 0)
    {
        let replies = gtk::Button::builder()
            .label(say.plural("%lld replies", i64::from(count), &[&i64::from(count)]))
            .css_classes(["flat", "caption"])
            .halign(gtk::Align::Start)
            .tooltip_text(say.get("Opens the thread"))
            .build();
        let (host, id) = (Rc::clone(&ctx.host), message.id);
        replies.connect_clicked(move |_| host.open_thread(id));
        balloon.append(&replies);
    }
    (balloon.upcast(), words)
}

/// The words, as text blocks, tables and rich blocks — or the assistant's answer as it streams.
fn append_body(ctx: &BubbleContext, balloon: &gtk::Box, message: &MessageDto, own: bool) {
    let say = &ctx.shell.say;
    let answers = &ctx.connection.answers;
    let body = answers.body_of(message);
    if bubble_rules::awaited(
        message,
        &body,
        ctx.me,
        ctx.assistant_chat,
        ctx.assistant_user_id,
    ) {
        let waiting = gtk::Label::builder()
            .label("…")
            .css_classes(["title-2"])
            .xalign(0.0)
            .build();
        waiting.update_property(&[gtk::accessible::Property::Label(
            say.get("The assistant is answering"),
        )]);
        balloon.append(&waiting);
        return;
    }
    if !body.is_empty() {
        let blocked = Rc::clone(&ctx.blocked);
        let is_blocked = move |id: i64| blocked.contains(&id);
        let blocks = bubble_body::lay(
            &body,
            message.mentions.as_deref(),
            &ctx.members,
            ctx.me,
            &is_blocked,
        );
        let look = rich_look(own);
        let slots = ctx.shell.slots();
        for block in &blocks {
            match block {
                BodyBlock::Text(runs) => {
                    let pieces = markup::split_math(runs);
                    if pieces.len() == 1
                        && let Piece::Words(words) = pieces[0]
                    {
                        balloon.append(&text_label(ctx, &markup::runs(words)));
                        continue;
                    }
                    let flow = adw::WrapBox::builder()
                        .child_spacing(4)
                        .line_spacing(2)
                        .build();
                    for piece in pieces {
                        match piece {
                            Piece::Words(words) => {
                                flow.append(&text_label(ctx, &markup::runs(words)));
                            }
                            Piece::Formula(source, typed) => {
                                flow.append(&slots.rich_inline(source, typed, &look).widget);
                            }
                        }
                    }
                    balloon.append(&flow);
                }
                BodyBlock::Table(table) => balloon.append(&table_widget(table)),
                BodyBlock::Rich(rich) => balloon.append(&slots.rich(rich, &look).widget),
            }
        }
        // The card under the first web link (P2's), unless the assistant is still writing or
        // its answer ends with sources.
        let emoji_only = false;
        if let Some(url) = bubble_body::preview_link(&body, emoji_only)
            && lookups::may_preview(
                message,
                &body,
                ctx.me,
                ctx.assistant_chat,
                ctx.assistant_user_id,
                answers.is_writing(message),
            )
            && let Some(card) = slots.link_card(message, &url, own)
        {
            balloon.append(&card);
        }
    }
    if let Some(failed) = answers.failure_sentence(message, say.as_ref()) {
        balloon.append(
            &gtk::Label::builder()
                .label(&failed)
                .css_classes(["fc-chat-system"])
                .wrap(true)
                .xalign(0.0)
                .build(),
        );
    }
}

/// How a rich block is drawn on this bubble: the ink of its balloon, the body's size.
fn rich_look(own: bool) -> RichLook {
    let dark = crate::theme::is_dark();
    let ink = if own || dark { "#ffffff" } else { "#1e1e1e" };
    RichLook::new(dark, ink, 15.0)
}

/// One text block: wrapping, selectable, its links handled here (a mention opens the chat, a web
/// link opens in the browser a beat later so a double click can be the heart — `LINK_DELAY`).
fn text_label(ctx: &BubbleContext, markup_text: &str) -> gtk::Label {
    let label = gtk::Label::builder()
        .use_markup(true)
        .label(markup_text)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .selectable(true)
        .xalign(0.0)
        .build();
    let host = Rc::clone(&ctx.host);
    let pending = Rc::new(Cell::new(0u64));
    label.connect_activate_link(move |label, uri| {
        if let Some(member) = markup::member_of(uri) {
            host.open_member(member);
            return glib::Propagation::Stop;
        }
        let Some(target) = bubble_body::openable(Some(uri)) else {
            return glib::Propagation::Stop;
        };
        let generation = pending.get().wrapping_add(1);
        pending.set(generation);
        let (pending, label) = (Rc::clone(&pending), label.clone());
        glib::timeout_add_local_once(bubble_body::LINK_DELAY, move || {
            if pending.get() != generation {
                return;
            }
            let window = label.root().and_then(|r| r.downcast::<gtk::Window>().ok());
            gtk::UriLauncher::new(&target).launch(
                window.as_ref(),
                gio::Cancellable::NONE,
                |result| {
                    if result.is_err() {
                        crate::fc_info!("link: the desktop could not open it");
                    }
                },
            );
        });
        glib::Propagation::Stop
    });
    label
}

/// A GFM table, cell by cell.
fn table_widget(table: &fc_text::markdown::Table) -> gtk::Widget {
    let grid = gtk::Grid::builder()
        .row_spacing(4)
        .column_spacing(12)
        .build();
    let align = |column: usize| match table.alignment(column) {
        fc_text::markdown::ColumnAlignment::Leading => 0.0,
        fc_text::markdown::ColumnAlignment::Center => 0.5,
        fc_text::markdown::ColumnAlignment::Trailing => 1.0,
    };
    for (column, cell) in table.header.iter().enumerate() {
        let label = gtk::Label::builder()
            .use_markup(true)
            .label(format!("<b>{}</b>", markup::spans(&cell.spans)))
            .xalign(align(column))
            .wrap(true)
            .build();
        grid.attach(&label, i32::try_from(column).unwrap_or(0), 0, 1, 1);
    }
    for (row_index, row) in table.rows.iter().enumerate() {
        for (column, cell) in row.iter().enumerate() {
            let label = gtk::Label::builder()
                .use_markup(true)
                .label(markup::spans(&cell.spans))
                .xalign(align(column))
                .wrap(true)
                .selectable(true)
                .build();
            grid.attach(
                &label,
                i32::try_from(column).unwrap_or(0),
                i32::try_from(row_index + 1).unwrap_or(0),
                1,
                1,
            );
        }
    }
    gtk::ScrolledWindow::builder()
        .vscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_width(true)
        .child(&grid)
        .build()
        .upcast()
}

/// The quote over a reply — both levels, a blocked member's masked until clicked — or `None`.
fn quote_widget(ctx: &BubbleContext, message: &MessageDto) -> Option<gtk::Widget> {
    let revealed = Rc::clone(&ctx.quote_revealed);
    let id = message.id;
    let quote = quotes::of(
        message,
        &ctx.connection.chats,
        ctx.shell.say.as_ref(),
        move |level| revealed(id, level),
    )
    .ok()
    .flatten()?;
    let column = gtk::Box::new(gtk::Orientation::Vertical, 2);
    column.add_css_class("fc-chat-quote");
    let line = |name: &str, excerpt: &str| {
        let row = gtk::Box::new(gtk::Orientation::Vertical, 0);
        if !name.is_empty() {
            row.append(
                &gtk::Label::builder()
                    .label(name)
                    .css_classes(["fc-chat-quote-name"])
                    .xalign(0.0)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .build(),
            );
        }
        row.append(
            &gtk::Label::builder()
                .label(excerpt)
                .css_classes(["fc-chat-quote-text"])
                .xalign(0.0)
                .lines(2)
                .wrap(true)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build(),
        );
        row
    };
    column.append(&line(&quote.reply.name, &quote.reply.excerpt));
    if let Some(parent) = &quote.parent {
        let under = line(&parent.name, &parent.excerpt);
        under.set_margin_start(8);
        under.set_opacity(0.8);
        column.append(&under);
    }
    column.set_cursor_from_name(Some("pointer"));
    let click = gtk::GestureClick::new();
    {
        let (host, quote) = (Rc::clone(&ctx.host), quote.clone());
        click.connect_released(move |gesture, presses, _, _| {
            if presses == 1 {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                host.quote_clicked(id, &quote);
            }
        });
    }
    column.add_controller(click);
    Some(column.upcast())
}

/// The clock, "edited", and the reader's tick in a direct chat.
fn footer(ctx: &BubbleContext, message: &MessageDto, own: bool) -> gtk::Widget {
    let say = &ctx.shell.say;
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    line.set_halign(if own {
        gtk::Align::End
    } else {
        gtk::Align::Start
    });
    line.append(
        &gtk::Label::builder()
            .label(time_text(ctx, message))
            .css_classes(["fc-chat-time"])
            .build(),
    );
    if bubble_rules::shows_tick(message, ctx.me, ctx.shared) {
        let seen = bubble_rules::seen(message, ctx.me, ctx.shared, ctx.peer_read_up_to);
        let tick = gtk::Label::builder()
            .label(if seen { "✓✓" } else { "✓" })
            .css_classes(["fc-chat-time"])
            .build();
        tick.update_property(&[gtk::accessible::Property::Label(if seen {
            say.get("Read")
        } else {
            say.get("Sent")
        })]);
        line.append(&tick);
    }
    line.upcast()
}

/// The reaction chips under a bubble, or `None` with none. A chip JOINS (never removes); on a
/// chip the reader is part of, it shows who reacted, where their own row removes it
/// (`fc_text::reactions::ChipTap`).
fn reactions_row(ctx: &BubbleContext, message: &MessageDto, own: bool) -> Option<gtk::Widget> {
    let reactions: Vec<Reaction> = message
        .reactions
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .map(|reaction| Reaction::new(reaction.user_id, &reaction.emoji))
        .collect();
    if reactions.is_empty() {
        return None;
    }
    let flow = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .max_children_per_line(12)
        .column_spacing(4)
        .row_spacing(4)
        .halign(if own {
            gtk::Align::End
        } else {
            gtk::Align::Start
        })
        .css_classes(["fc-chat-reactions"])
        .build();
    let names = names_of(ctx, &reactions);
    for chip in reaction_chips(&reactions, ctx.me) {
        let label = if chip.shows_count() {
            format!("{} {}", chip.emoji, chip.count)
        } else {
            chip.emoji.clone()
        };
        let button = gtk::Button::builder()
            .label(&label)
            .css_classes(["fc-chat-reaction"])
            .build();
        if chip.includes_me {
            button.add_css_class("fc-chat-mine");
        }
        let who: Vec<String> = reactions
            .iter()
            .filter(|r| r.emoji == chip.emoji && !ctx.is_blocked(r.user_id))
            .map(|r| names(r.user_id))
            .collect();
        button.set_tooltip_text(Some(&who.join(", ")));
        let (host, message, emoji) = (Rc::clone(&ctx.host), message.clone(), chip.emoji.clone());
        let tap = chip.tap();
        button.connect_clicked(move |button| match tap {
            ChipTap::Join => host.react(message.id, &emoji),
            ChipTap::ShowReactors => host.reactors(&message, &emoji, button.upcast_ref()),
        });
        flow.append(&button);
    }
    Some(flow.upcast())
}

/// A reactor's name: "You", the roster's, or "Someone".
fn names_of(ctx: &BubbleContext, _reactions: &[Reaction]) -> impl Fn(i64) -> String + use<> {
    let (me, members, say) = (ctx.me, Rc::clone(&ctx.members), Rc::clone(&ctx.shell.say));
    move |user_id| {
        poll_text::name(
            user_id,
            me,
            |id| members.iter().find(|member| member.id == id).cloned(),
            say.as_ref(),
        )
    }
}
