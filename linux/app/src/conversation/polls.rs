//! Polls (shell-design §5.4; docs/protocol.md, "Polls"): the card a poll's bubble draws — votes in
//! place, the bars, who chose what, the footer, the author's "Close poll" — and the two
//! `adw::Dialog`s that are the Mac's sheets: the poll composer (`PollDraft`, the question a prose
//! field with capitals and emoji typing, the options single-line fields with emoji typing) and
//! "Open polls" (`OpenPollsModel`, oldest first, re-read after every action).

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use fc_core::protocol::{MessageDto, PollDto};
use fc_core::text::polls;
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::composer_mentions;
use fc_logic::open_polls::OpenPollsModel;
use fc_logic::poll_draft::PollDraft;
use fc_logic::poll_text;
use fc_logic::poll_voting;

use super::bubble::{BubbleContext, BubbleHost};
use super::page::ConversationPage;
use crate::shell::Shell;
use crate::text::typing_hooks;

/// A poll's card inside its bubble.
pub fn card(ctx: &BubbleContext, message: &MessageDto, poll: &PollDto) -> gtk::Widget {
    let say = &ctx.shell.say;
    let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
    // A readable card whatever the question's length (the Mac's poll fills its bubble).
    column.set_width_request(260);
    column.append(
        &gtk::Label::builder()
            .label(&message.body)
            .css_classes(["heading"])
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .xalign(0.0)
            .selectable(true)
            .build(),
    );
    let votable = polls::votable(message);
    let mine = polls::my_option(poll, ctx.me);
    let blocked = Rc::clone(&ctx.blocked);
    let (me, members, say_rc) = (ctx.me, Rc::clone(&ctx.members), Rc::clone(&ctx.shell.say));
    let name = move |user_id: i64| {
        poll_text::name(
            user_id,
            me,
            |id| members.iter().find(|member| member.id == id).cloned(),
            say_rc.as_ref(),
        )
    };
    for option in &poll.options {
        let chosen = mine == Some(option.id);
        let line = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        if chosen {
            top.append(&gtk::Image::from_icon_name("object-select-symbolic"));
        }
        top.append(
            &gtk::Label::builder()
                .label(&option.text)
                .wrap(true)
                .xalign(0.0)
                .hexpand(true)
                .build(),
        );
        top.append(
            &gtk::Label::builder()
                .label(option.votes.len().to_string())
                .css_classes(["fc-chat-time"])
                .build(),
        );
        line.append(&top);
        let bar = gtk::LevelBar::builder()
            .min_value(0.0)
            .max_value(1.0)
            .value(polls::fraction(poll, option.votes.len()))
            .build();
        line.append(&bar);
        let drawable = polls::drawable_voters(&option.votes, |id| blocked.contains(&id));
        let button = gtk::Button::builder()
            .child(&line)
            .css_classes(["flat", "fc-chat-poll-option"])
            .sensitive(votable)
            .build();
        if chosen {
            button.add_css_class("fc-chat-mine");
        }
        button.update_property(&[gtk::accessible::Property::Label(&poll_text::option_label(
            option,
            chosen,
            say.as_ref(),
        ))]);
        let (host, id, option_id) = (Rc::clone(&ctx.host), message.id, option.id);
        button.connect_clicked(move |_| host.vote(id, option_id));
        column.append(&button);
        // Who chose it: the first names, and a click for every one of them — the half "+N" cannot
        // show (the Mac's "See who voted").
        if !drawable.is_empty() {
            let voters = gtk::Button::builder()
                .child(
                    &gtk::Label::builder()
                        .label(poll_text::voters(&drawable, &name, say.as_ref()))
                        .css_classes(["fc-chat-time"])
                        .xalign(0.0)
                        .wrap(true)
                        .build(),
                )
                .css_classes(["flat", "fc-chat-poll-voters"])
                .halign(gtk::Align::Start)
                .tooltip_text(say.get("See who voted"))
                .build();
            voters.update_property(&[gtk::accessible::Property::Label(say.get("See who voted"))]);
            let (text, name) = (option.text.clone(), name.clone());
            voters.connect_clicked(move |anchor| voter_list(anchor, &text, &drawable, &name));
            column.append(&voters);
        }
    }
    let count = poll_text::member_count(
        composer_mentions::candidates(Some(&ctx.chat), &ctx.members).iter(),
    );
    let mut footer = poll_text::footer(poll, count, say.as_ref());
    if poll.closed {
        footer = format!("{footer} · {}", say.get("Poll closed"));
    }
    column.append(
        &gtk::Label::builder()
            .label(&footer)
            .css_classes(["fc-chat-time"])
            .xalign(0.0)
            .build(),
    );
    if poll_voting::may_close(message, ctx.me) {
        let close = gtk::Button::builder()
            .label(say.get("Close poll"))
            .css_classes(["flat"])
            .halign(gtk::Align::Start)
            .build();
        let (host, id) = (Rc::clone(&ctx.host), message.id);
        close.connect_clicked(move |_| host.close_poll(id));
        column.append(&close);
    }
    column.upcast()
}

/// Every voter of one option beside `anchor`, faces and names in the order they voted.
fn voter_list(anchor: &gtk::Button, option: &str, voters: &[i64], name: &dyn Fn(i64) -> String) {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 8);
    column.set_margin_top(8);
    column.set_margin_bottom(8);
    column.set_margin_start(8);
    column.set_margin_end(8);
    column.append(
        &gtk::Label::builder()
            .label(option)
            .css_classes(["heading"])
            .xalign(0.0)
            .wrap(true)
            .max_width_chars(32)
            .build(),
    );
    for &voter in voters {
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let shown = name(voter);
        line.append(&crate::chats::avatar::avatar(
            &shown,
            fc_logic::groups::ChatFace::Initials,
            Some(voter),
            24,
        ));
        line.append(&gtk::Label::builder().label(&shown).xalign(0.0).build());
        column.append(&line);
    }
    let popover = gtk::Popover::builder().child(&column).build();
    popover.set_parent(anchor);
    popover.connect_closed(|popover| {
        let popover = popover.clone();
        glib::idle_add_local_once(move || popover.unparent());
    });
    popover.popup();
}

/// The poll composer ("New poll"): `send(question, options)` when Create is pressed on a draft
/// `PollDraft::checked` takes.
pub fn compose(parent: &impl IsA<gtk::Widget>, send: Rc<dyn Fn(String, Vec<String>)>) {
    let shell = Shell::get();
    let say = &shell.say;
    let draft = Rc::new(RefCell::new(PollDraft::new()));
    let dialog = adw::Dialog::builder()
        .title(say.get("New poll"))
        .content_width(440)
        .build();
    let header = adw::HeaderBar::builder()
        .show_end_title_buttons(false)
        .show_start_title_buttons(false)
        .build();
    let cancel = gtk::Button::with_label(say.get("Cancel"));
    let create = gtk::Button::builder()
        .label(say.get("Create"))
        .css_classes(["suggested-action"])
        .sensitive(false)
        .build();
    header.pack_start(&cancel);
    header.pack_end(&create);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_top(12);
    content.set_margin_bottom(18);
    content.set_margin_start(18);
    content.set_margin_end(18);
    content.append(
        &gtk::Label::builder()
            .label(say.get("Question"))
            .css_classes(["heading"])
            .xalign(0.0)
            .build(),
    );
    let question = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(8)
        .right_margin(8)
        .accepts_tab(false)
        .css_classes(["card"])
        .build();
    question.update_property(&[gtk::accessible::Property::Label(
        say.get("Ask the family something…"),
    )]);
    let question_hooks = typing_hooks::attach_text_view(&question, true);
    content.append(&question);
    content.append(
        &gtk::Label::builder()
            .label(say.get("The question is the message everyone sees."))
            .css_classes(["dim-label", "caption"])
            .xalign(0.0)
            .wrap(true)
            .build(),
    );
    content.append(
        &gtk::Label::builder()
            .label(say.get("Options"))
            .css_classes(["heading"])
            .xalign(0.0)
            .build(),
    );
    let options = gtk::Box::new(gtk::Orientation::Vertical, 6);
    content.append(&options);
    let add = gtk::Button::builder()
        .label(say.get("Add option"))
        .css_classes(["flat"])
        .halign(gtk::Align::Start)
        .build();
    content.append(&add);
    content.append(
        &gtk::Label::builder()
            .label(
                say.get("Between 2 and 10 options. They can't be changed once the poll is sent."),
            )
            .css_classes(["dim-label", "caption"])
            .xalign(0.0)
            .wrap(true)
            .build(),
    );
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(
        &gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .child(&content)
            .build(),
    ));
    dialog.set_child(Some(&view));

    let assess = {
        let (draft, create) = (Rc::clone(&draft), create.clone());
        Rc::new(move || create.set_sensitive(draft.borrow().checked().is_some()))
    };
    {
        let (draft, assess) = (Rc::clone(&draft), Rc::clone(&assess));
        question.buffer().connect_changed(move |buffer| {
            draft.borrow_mut().question = typing_hooks::buffer_text(buffer);
            assess();
        });
    }
    // The option rows, drawn again whenever one is added or removed.
    let hooks: Rc<RefCell<Vec<Rc<typing_hooks::TypingHooks>>>> = Rc::new(RefCell::new(Vec::new()));
    type Redraw = Rc<RefCell<Option<Rc<dyn Fn()>>>>;
    let redraw: Redraw = Rc::new(RefCell::new(None));
    {
        let (draft, options, assess, hooks, redraw_ref) = (
            Rc::clone(&draft),
            options.clone(),
            Rc::clone(&assess),
            Rc::clone(&hooks),
            Rc::clone(&redraw),
        );
        let shell = Rc::clone(&shell);
        let draw: Rc<dyn Fn()> = Rc::new(move || {
            while let Some(child) = options.first_child() {
                options.remove(&child);
            }
            hooks.borrow_mut().clear();
            let texts: Vec<String> = draft.borrow().options().to_vec();
            let removable = draft.borrow().may_remove();
            for (index, text) in texts.iter().enumerate() {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                let entry = gtk::Entry::builder()
                    .text(text)
                    .placeholder_text(shell.say.get("Option"))
                    .max_length(i32::try_from(polls::MAX_OPTION_CHARS).unwrap_or(100))
                    .hexpand(true)
                    .build();
                hooks
                    .borrow_mut()
                    .push(typing_hooks::attach_editable(&entry, &entry));
                {
                    let (draft, assess) = (Rc::clone(&draft), Rc::clone(&assess));
                    let at = isize::try_from(index).unwrap_or(-1);
                    entry.connect_changed(move |entry| {
                        draft.borrow_mut().set(at, &entry.text());
                        assess();
                    });
                }
                row.append(&entry);
                if removable {
                    let remove = gtk::Button::builder()
                        .icon_name("list-remove-symbolic")
                        .css_classes(["flat"])
                        .tooltip_text(shell.say.get("Remove option"))
                        .build();
                    remove.update_property(&[gtk::accessible::Property::Label(
                        shell.say.get("Remove option"),
                    )]);
                    let (draft, redraw, assess) = (
                        Rc::clone(&draft),
                        Rc::clone(&redraw_ref),
                        Rc::clone(&assess),
                    );
                    let at = isize::try_from(index).unwrap_or(-1);
                    remove.connect_clicked(move |_| {
                        draft.borrow_mut().remove(at);
                        if let Some(redraw) = redraw.borrow().as_ref() {
                            redraw();
                        }
                        assess();
                    });
                    row.append(&remove);
                }
                options.append(&row);
            }
        });
        *redraw.borrow_mut() = Some(Rc::clone(&draw));
        draw();
    }
    {
        let (draft, redraw, assess, add_button) = (
            Rc::clone(&draft),
            Rc::clone(&redraw),
            Rc::clone(&assess),
            add.clone(),
        );
        add.connect_clicked(move |_| {
            draft.borrow_mut().add();
            add_button.set_sensitive(draft.borrow().may_add());
            if let Some(redraw) = redraw.borrow().as_ref() {
                redraw();
            }
            assess();
        });
    }
    {
        let dialog = dialog.clone();
        cancel.connect_clicked(move |_| {
            dialog.close();
        });
    }
    {
        let (dialog, draft) = (dialog.clone(), Rc::clone(&draft));
        create.connect_clicked(move |_| {
            let checked = draft.borrow().checked();
            if let Some((question, options)) = checked {
                send(question, options);
                dialog.close();
            }
        });
    }
    // Kept alive with the dialog.
    let keep = (question_hooks, hooks, redraw);
    dialog.connect_closed(move |_| {
        let _ = &keep;
    });
    dialog.present(Some(parent));
    question.grab_focus();
}

/// "Open polls" (the Mac's `OpenPollsView`): every poll still open in the chat, oldest first, each
/// drawn as its card in the conversation — the bars, who chose what, the footer, the author's
/// "Close poll" — voted on in place. A spinner while the list is read, "Couldn't load the polls"
/// when it could not be, "Nothing to decide" when everything is decided. It redraws with the page
/// (`ConversationPage::set_polls_redraw`): a vote, a close, a new poll.
pub fn open_polls(parent: &impl IsA<gtk::Widget>, page: &Rc<ConversationPage>) {
    let shell = Shell::get();
    let say = &shell.say;
    let connection = Rc::clone(page.connection());
    let model = Rc::new(OpenPollsModel::new(
        page.chat_id,
        Rc::clone(&connection.chats),
        Rc::clone(&connection.api),
    ));
    let dialog = adw::Dialog::builder()
        .title(say.get("Open polls"))
        .content_width(480)
        .content_height(560)
        .build();
    let list = gtk::Box::new(gtk::Orientation::Vertical, 12);
    list.set_margin_top(12);
    list.set_margin_bottom(18);
    list.set_margin_start(18);
    list.set_margin_end(18);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(
        &gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&list)
            .build(),
    ));
    dialog.set_child(Some(&view));
    let host: Rc<dyn BubbleHost> = Rc::new(DialogHost {
        page: Rc::downgrade(page),
        dialog: dialog.downgrade(),
    });
    // The redraw holds itself (a reveal redraws), until the dialog closes.
    let me_fill: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::default();
    let fill: Rc<dyn Fn()> = {
        let (model, list, page, me_fill) = (
            Rc::clone(&model),
            list.clone(),
            Rc::downgrade(page),
            Rc::clone(&me_fill),
        );
        let fill: Rc<dyn Fn()> = Rc::new({
            let me_fill = Rc::clone(&me_fill);
            move || {
                let Some(page) = page.upgrade() else {
                    return;
                };
                let shell = Shell::get();
                let say = &shell.say;
                while let Some(child) = list.first_child() {
                    list.remove(&child);
                }
                let messages = model.messages().unwrap_or_default();
                let status = |icon: &str, title: &str, description: &str| {
                    let status = adw::StatusPage::builder()
                        .icon_name(icon)
                        .title(title)
                        .description(description)
                        .vexpand(true)
                        .build();
                    status.add_css_class("compact");
                    status
                };
                if messages.is_empty() {
                    if let Some(error) = model.failure() {
                        crate::fc_info!("open polls: the read failed ({})", error.code);
                        list.append(&status(
                            "dialog-warning-symbolic",
                            say.get("Couldn't load the polls"),
                            say.get("Try again in a moment."),
                        ));
                    } else if model.loaded() {
                        // Not a failure, and said as such: everything is decided.
                        list.append(&status(
                            "object-select-symbolic",
                            say.get("Nothing to decide"),
                            say.get(
                                "Open polls stay here until they're closed. There are none right now.",
                            ),
                        ));
                    } else {
                        list.append(
                            &adw::Spinner::builder()
                                .width_request(32)
                                .height_request(32)
                                .halign(gtk::Align::Center)
                                .valign(gtk::Align::Center)
                                .vexpand(true)
                                .build(),
                        );
                    }
                    return;
                }
                let Some(ctx) = page.polls_context(Rc::clone(&host)) else {
                    return;
                };
                for message in messages {
                    let card_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
                    card_box.add_css_class("card");
                    let inner = gtk::Box::new(gtk::Orientation::Vertical, 6);
                    inner.set_margin_top(10);
                    inner.set_margin_bottom(10);
                    inner.set_margin_start(12);
                    inner.set_margin_end(12);
                    card_box.append(&inner);
                    if model.is_hidden(&message).unwrap_or(false) {
                        let reveal = gtk::Button::builder()
                            .label(say.get("Hidden — blocked member"))
                            .css_classes(["flat", "fc-chat-hidden"])
                            .build();
                        let (model, me_fill, id) =
                            (Rc::clone(&model), Rc::clone(&me_fill), message.id);
                        reveal.connect_clicked(move |_| {
                            model.reveal(id);
                            let fill = me_fill.borrow().clone();
                            if let Some(fill) = fill {
                                fill();
                            }
                        });
                        inner.append(&reveal);
                        list.append(&card_box);
                        continue;
                    }
                    let sender = poll_text::name(
                        message.sender_id,
                        ctx.me,
                        |id| ctx.connection.chats.member(id).ok().flatten(),
                        say.as_ref(),
                    );
                    inner.append(
                        &gtk::Label::builder()
                            .label(&sender)
                            .css_classes(["fc-chat-sender"])
                            .xalign(0.0)
                            .build(),
                    );
                    if let Some(poll) = &message.poll {
                        inner.append(&card(&ctx, &message, poll));
                    }
                    list.append(&card_box);
                }
            }
        });
        *me_fill.borrow_mut() = Some(Rc::clone(&fill));
        fill
    };
    fill();
    page.set_polls_redraw(Some(Rc::clone(&fill)));
    {
        let (model, fill) = (Rc::clone(&model), Rc::clone(&fill));
        glib::spawn_future_local(async move {
            let _ = model.load().await;
            fill();
        });
    }
    {
        let page = Rc::downgrade(page);
        dialog.connect_closed(move |_| {
            me_fill.borrow_mut().take();
            if let Some(page) = page.upgrade() {
                page.set_polls_redraw(None);
            }
        });
    }
    dialog.present(Some(parent));
}

/// What a card in the Open polls dialog does: votes and closes through the page — whose redraw
/// redraws the dialog — and the "Close poll" question over the dialog itself.
struct DialogHost {
    page: std::rc::Weak<ConversationPage>,
    dialog: glib::WeakRef<adw::Dialog>,
}

impl BubbleHost for DialogHost {
    fn react(&self, _message_id: i64, _emoji: &str) {}
    fn reveal(&self, _message_id: i64) {}
    fn quote_clicked(&self, _message_id: i64, _quote: &fc_logic::quotes::Quote) {}
    fn open_thread(&self, _message_id: i64) {}
    fn vote(&self, message_id: i64, option_id: i64) {
        if let Some(page) = self.page.upgrade() {
            page.vote(message_id, option_id);
        }
    }
    fn close_poll(&self, message_id: i64) {
        if let (Some(page), Some(dialog)) = (self.page.upgrade(), self.dialog.upgrade()) {
            page.close_poll_over(message_id, dialog.upcast_ref());
        }
    }
    fn open_member(&self, _user_id: i64) {}
    fn menu(&self, _message: &MessageDto, _anchor: &gtk::Widget, _x: f64, _y: f64) {}
    fn reactors(&self, _message: &MessageDto, _emoji: &str, _anchor: &gtk::Widget) {}
}
