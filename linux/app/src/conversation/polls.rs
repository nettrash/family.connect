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

use super::bubble::BubbleContext;
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
        if !drawable.is_empty() {
            line.append(
                &gtk::Label::builder()
                    .label(poll_text::voters(&drawable, &name, say.as_ref()))
                    .css_classes(["fc-chat-time"])
                    .xalign(0.0)
                    .wrap(true)
                    .build(),
            );
        }
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

/// "Open polls": the family chat's open polls, voted on in place.
pub fn open_polls(parent: &impl IsA<gtk::Widget>, chat_id: i64) {
    let shell = Shell::get();
    let Some(connection) = shell.connection() else {
        return;
    };
    let say = &shell.say;
    let model = Rc::new(OpenPollsModel::new(
        chat_id,
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
    type Draw = Rc<RefCell<Option<Rc<dyn Fn()>>>>;
    let draw: Draw = Rc::new(RefCell::new(None));
    {
        let (model, list, draw_ref) = (Rc::clone(&model), list.clone(), Rc::clone(&draw));
        let connection = Rc::clone(&connection);
        let fill: Rc<dyn Fn()> = Rc::new(move || {
            let shell = Shell::get();
            let say = &shell.say;
            while let Some(child) = list.first_child() {
                list.remove(&child);
            }
            let messages = model.messages().unwrap_or_default();
            if model.loaded() && messages.is_empty() {
                list.append(
                    &adw::StatusPage::builder()
                        .icon_name("view-list-bullet-symbolic")
                        .title(say.get("Open polls"))
                        .description(say.get(
                            "Open polls stay here until they're closed. There are none right now.",
                        ))
                        .build(),
                );
            }
            if let Some(error) = model.failure() {
                crate::fc_info!("open polls: the read failed ({})", error.code);
                list.append(
                    &gtk::Label::builder()
                        .label(say.get("Couldn't load the polls"))
                        .css_classes(["error"])
                        .build(),
                );
            }
            let me = connection.chats.reader();
            for message in messages {
                let card_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
                card_box.add_css_class("card");
                card_box.set_margin_bottom(6);
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
                    let (model, draw, id) = (Rc::clone(&model), Rc::clone(&draw_ref), message.id);
                    reveal.connect_clicked(move |_| {
                        model.reveal(id);
                        if let Some(draw) = draw.borrow().as_ref() {
                            draw();
                        }
                    });
                    inner.append(&reveal);
                    list.append(&card_box);
                    continue;
                }
                let sender = poll_text::name(
                    message.sender_id,
                    me,
                    |id| connection.chats.member(id).ok().flatten(),
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
                    inner.append(
                        &gtk::Label::builder()
                            .label(&message.body)
                            .css_classes(["heading"])
                            .wrap(true)
                            .xalign(0.0)
                            .build(),
                    );
                    let mine = polls::my_option(poll, me);
                    for option in &poll.options {
                        let chosen = mine == Some(option.id);
                        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                        if chosen {
                            row.append(&gtk::Image::from_icon_name("object-select-symbolic"));
                        }
                        row.append(
                            &gtk::Label::builder()
                                .label(&option.text)
                                .hexpand(true)
                                .xalign(0.0)
                                .wrap(true)
                                .build(),
                        );
                        row.append(&gtk::Label::new(Some(&option.votes.len().to_string())));
                        let button = gtk::Button::builder()
                            .child(&row)
                            .css_classes(["flat", "fc-chat-poll-option"])
                            .build();
                        if chosen {
                            button.add_css_class("fc-chat-mine");
                        }
                        button.update_property(&[gtk::accessible::Property::Label(
                            &poll_text::option_label(option, chosen, say.as_ref()),
                        )]);
                        let (model, draw, id, option_id) = (
                            Rc::clone(&model),
                            Rc::clone(&draw_ref),
                            message.id,
                            option.id,
                        );
                        button.connect_clicked(move |_| {
                            let (model, draw) = (Rc::clone(&model), Rc::clone(&draw));
                            glib::spawn_future_local(async move {
                                if let Ok(Some(error)) = model.vote(id, option_id).await {
                                    crate::fc_info!("open polls: vote refused ({})", error.code);
                                }
                                if let Some(draw) = draw.borrow().as_ref() {
                                    draw();
                                }
                            });
                        });
                        inner.append(&button);
                    }
                }
                list.append(&card_box);
            }
        });
        *draw.borrow_mut() = Some(Rc::clone(&fill));
    }
    {
        let (model, draw) = (Rc::clone(&model), Rc::clone(&draw));
        glib::spawn_future_local(async move {
            let _ = model.load().await;
            if let Some(draw) = draw.borrow().as_ref() {
                draw();
            }
        });
    }
    let keep = Rc::clone(&draw);
    dialog.connect_closed(move |_| {
        keep.borrow_mut().take();
    });
    dialog.present(Some(parent));
}
