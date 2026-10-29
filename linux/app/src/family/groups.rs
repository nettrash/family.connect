//! New Group and Group Info (shell-design §12, last line; the Mac's `GroupViews`): `adw::Dialog`s
//! over the main window. Who may do what, what a name may be and whom the lists offer are
//! `group_rules`' (the admin renames, adds, removes and deletes; anybody may leave); the writes are
//! `GroupModel`'s, which put the server's answer into the cache at once.

use std::rc::Rc;

use adw::prelude::*;
use fc_core::protocol::{ChatDto, MemberDto};
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::groups::{GroupAction, GroupModel, group_rules, group_text};

use crate::services::log;
use crate::settings::ui;
use crate::shell::{Connection, Shell};

/// `win.new-group` (the sidebar's "+", the primary menu): a name and the members to start with;
/// Create opens the new group in the main window.
pub fn new_group() {
    let Some(parent) = ui::main_parent() else {
        return;
    };
    let shell = Shell::get();
    let Some(connection) = shell.connection() else {
        return;
    };
    let state = connection.session.state();
    if !state.can_chat() {
        return;
    }
    let say = Rc::clone(&shell.say);
    let me = state.me.as_ref().map_or(0, |me| me.id);
    let roster: Vec<MemberDto> = connection
        .chats
        .members()
        .map(|members| members.to_vec())
        .unwrap_or_default();
    let choosable = group_rules::choosable(
        &roster,
        me,
        state.assistant.as_ref().map(|assistant| assistant.user_id),
    );

    let (dialog, view, _) = ui::dialog(say.get("New Group"), 420, 480);
    let (header, create) = ui::dialog_bar(&dialog, say.get("Cancel"), say.get("Create"), false);
    view.add_top_bar(&header);
    let name_group = adw::PreferencesGroup::new();
    let name = adw::EntryRow::builder()
        .title(say.get("Group Name"))
        .build();
    name_group.add(&name);
    let members = adw::PreferencesGroup::builder()
        .title(say.get("Members"))
        .build();
    let mut checks = Vec::new();
    for person in &choosable {
        let check = gtk::CheckButton::new();
        let row = adw::ActionRow::builder()
            .title(&person.display_name)
            .activatable_widget(&check)
            .build();
        row.add_prefix(&check);
        row.add_suffix(&ui::face(&person.display_name, person.id, 28));
        members.add(&row);
        checks.push((person.id, check));
    }
    let problem = ui::problem_label();
    members.add(&problem);
    let page = adw::PreferencesPage::new();
    page.add(&name_group);
    page.add(&members);
    view.set_content(Some(&page));

    let checks = Rc::new(checks);
    let weak = dialog.downgrade();
    let busy = ui::busy(&create);
    {
        let checks = Rc::clone(&checks);
        create.connect_clicked(move |_| {
            let shell = Shell::get();
            let typed = name.text().to_string();
            let chosen: Vec<i64> = checks
                .iter()
                .filter(|(_, check)| check.is_active())
                .map(|(id, _)| *id)
                .collect();
            if let Some(text) =
                group_rules::create_problem(Some(&typed), &chosen, me, shell.say.as_ref())
            {
                ui::show_problem(&problem, &text);
                return;
            }
            let Some(connection) = shell.connection() else {
                return;
            };
            let (weak, problem, busy) = (weak.clone(), problem.clone(), Rc::clone(&busy));
            busy(true);
            ui::spawn(async move {
                let answer = model(&connection).create(&typed, &chosen).await;
                busy(false);
                match answer {
                    Ok(Ok(chat_id)) => {
                        crate::fc_info!("group: {chat_id} created");
                        if let Some(dialog) = weak.upgrade() {
                            dialog.close();
                        }
                        Shell::get().app.activate_action(
                            crate::seams::actions::OPEN_CHAT.name,
                            Some(&chat_id.to_variant()),
                        );
                    }
                    Ok(Err(error)) => {
                        crate::fc_info!("group: create refused ({})", error.code);
                        let shell = Shell::get();
                        ui::show_problem(
                            &problem,
                            &group_text::failure(&error, true, shell.say.as_ref()),
                        );
                    }
                    Err(error) => {
                        crate::fc_warn!("group: the cache refused ({})", log::store_error(&error))
                    }
                }
            });
        });
    }
    dialog.present(Some(&parent));
}

/// Group Info for `chat_id` (P1's "Group info" header button, through `FamilyRoutes::group_info`):
/// the members in the order they joined, the admin marked; the admin renames, adds, removes and
/// deletes; anybody leaves.
pub fn group_info(parent: &impl IsA<gtk::Widget>, chat_id: i64) -> Option<adw::Dialog> {
    let shell = Shell::get();
    let connection = shell.connection()?;
    connection.chats.chat(chat_id).ok().flatten()?;
    let say = Rc::clone(&shell.say);
    let (dialog, view, toasts) = ui::dialog(say.get("Group Info"), 420, 480);
    let header = adw::HeaderBar::new();
    view.add_top_bar(&header);
    let page = adw::PreferencesPage::new();
    toasts.set_child(Some(&page));
    let holder = Rc::new(std::cell::RefCell::new(Vec::<adw::PreferencesGroup>::new()));
    let draw: Rc<dyn Fn()> = {
        let (page, toasts, holder, weak_dialog) = (
            page.clone(),
            toasts.clone(),
            Rc::clone(&holder),
            dialog.downgrade(),
        );
        Rc::new(move || {
            for group in holder.borrow_mut().drain(..) {
                page.remove(&group);
            }
            let shell = Shell::get();
            let Some(connection) = shell.connection() else {
                return;
            };
            let Some(chat) = connection
                .chats
                .chat(chat_id)
                .ok()
                .flatten()
                .map(|row| row.chat)
            else {
                // Gone — left, deleted, or removed: nothing more to show.
                if let Some(dialog) = weak_dialog.upgrade() {
                    dialog.close();
                }
                return;
            };
            for group in info_groups(&connection, &chat, &toasts, &weak_dialog) {
                page.add(&group);
                holder.borrow_mut().push(group);
            }
        })
    };
    REDRAW.with(|redraw| *redraw.borrow_mut() = Some(Rc::clone(&draw)));
    draw();
    dialog.connect_closed(|_| {
        REDRAW.with(|redraw| redraw.borrow_mut().take());
    });
    dialog.present(Some(parent));
    Some(dialog)
}

thread_local! {
    /// The open Group Info's redraw, called after each change it made.
    static REDRAW: std::cell::RefCell<Option<Rc<dyn Fn()>>> = const { std::cell::RefCell::new(None) };
}

fn redraw() {
    if let Some(redraw) = REDRAW.with(|redraw| redraw.borrow().clone()) {
        redraw();
    }
}

fn info_groups(
    connection: &Rc<Connection>,
    chat: &ChatDto,
    toasts: &adw::ToastOverlay,
    dialog: &glib::WeakRef<adw::Dialog>,
) -> Vec<adw::PreferencesGroup> {
    let shell = Shell::get();
    let say: &dyn StringCatalog = shell.say.as_ref();
    let me = connection.chats.reader();
    let roster: Vec<MemberDto> = connection
        .chats
        .members()
        .map(|members| members.to_vec())
        .unwrap_or_default();
    let name = adw::PreferencesGroup::builder()
        .title(&chat.title)
        .description(group_text::count(chat, say))
        .build();
    if group_rules::may(GroupAction::Rename, chat, me) {
        let entry = adw::EntryRow::builder()
            .title(say.get("Group Name"))
            .text(&chat.title)
            .show_apply_button(true)
            .build();
        let (connection, toasts, id) = (Rc::clone(connection), toasts.clone(), chat.id);
        entry.connect_apply(move |entry| {
            let (connection, toasts, typed) = (
                Rc::clone(&connection),
                toasts.clone(),
                entry.text().to_string(),
            );
            ui::spawn(async move {
                let answer = model(&connection).rename(id, &typed).await;
                after(answer, &toasts);
            });
        });
        name.add(&entry);
    }
    let members = adw::PreferencesGroup::builder()
        .title(say.get("Members"))
        .build();
    for person in group_rules::members(chat, &roster, me, say) {
        let title = if person.me {
            say.get("You").to_string()
        } else {
            person.name.clone()
        };
        let row = adw::ActionRow::builder().title(&title).build();
        row.add_prefix(&ui::face(&person.name, person.user_id, 28));
        if person.admin {
            row.add_suffix(&ui::owner_pill(say.get("Admin")));
        }
        if person.removable {
            let button = gtk::Button::builder()
                .icon_name("list-remove-symbolic")
                .tooltip_text(say.get("Remove from Group"))
                .valign(gtk::Align::Center)
                .build();
            button.add_css_class("flat");
            button.update_property(&[gtk::accessible::Property::Label(
                say.get("Remove from Group"),
            )]);
            let (connection, toasts, id, user) = (
                Rc::clone(connection),
                toasts.clone(),
                chat.id,
                person.user_id,
            );
            button.connect_clicked(move |button| {
                let say = Rc::clone(&Shell::get().say);
                let (connection, toasts) = (Rc::clone(&connection), toasts.clone());
                ui::ask(
                    button,
                    say.get("Remove from Group"),
                    "",
                    say.get("Cancel"),
                    say.get("Remove from Group"),
                    true,
                    move || {
                        let (connection, toasts) = (Rc::clone(&connection), toasts.clone());
                        ui::spawn(async move {
                            let answer = model(&connection).remove(id, user).await;
                            after(answer, &toasts);
                        });
                    },
                );
            });
            row.add_suffix(&button);
        }
        members.add(&row);
    }
    let actions = adw::PreferencesGroup::new();
    if group_rules::may(GroupAction::AddMembers, chat, me) {
        let (connection, chat, toasts) = (Rc::clone(connection), chat.clone(), toasts.clone());
        actions.add(&ui::button_row(say.get("Add Members"), false, move || {
            add_members(&connection, &chat, &toasts);
        }));
    }
    if group_rules::may(GroupAction::Leave, chat, me) {
        let (connection, title, toasts, id, dialog) = (
            Rc::clone(connection),
            chat.title.clone(),
            toasts.clone(),
            chat.id,
            dialog.clone(),
        );
        actions.add(&ui::button_row(say.get("Leave Group"), true, move || {
            let say = Rc::clone(&Shell::get().say);
            let (connection, toasts, dialog) =
                (Rc::clone(&connection), toasts.clone(), dialog.clone());
            let Some(parent) = dialog.upgrade() else {
                return;
            };
            ui::ask(
                &parent,
                say.get("Leave Group"),
                &group_text::leave_question(&title, say.as_ref()),
                say.get("Cancel"),
                say.get("Leave Group"),
                true,
                move || {
                    let (connection, toasts) = (Rc::clone(&connection), toasts.clone());
                    ui::spawn(async move {
                        let answer = model(&connection).leave(id).await;
                        after(answer, &toasts);
                    });
                },
            );
        }));
    }
    if group_rules::may(GroupAction::Delete, chat, me) {
        let (connection, title, toasts, id, dialog) = (
            Rc::clone(connection),
            chat.title.clone(),
            toasts.clone(),
            chat.id,
            dialog.clone(),
        );
        actions.add(&ui::button_row(say.get("Delete Group"), true, move || {
            let say = Rc::clone(&Shell::get().say);
            let (connection, toasts) = (Rc::clone(&connection), toasts.clone());
            let Some(parent) = dialog.upgrade() else {
                return;
            };
            ui::ask(
                &parent,
                say.get("Delete Group"),
                &group_text::delete_question(&title, say.as_ref()),
                say.get("Cancel"),
                say.get("Delete Group"),
                true,
                move || {
                    let (connection, toasts) = (Rc::clone(&connection), toasts.clone());
                    ui::spawn(async move {
                        let answer = model(&connection).delete(id).await;
                        after(answer, &toasts);
                    });
                },
            );
        }));
    }
    vec![name, members, actions]
}

/// Add Members: everyone `group_rules::addable` — or the sentence that everyone is in already.
fn add_members(connection: &Rc<Connection>, chat: &ChatDto, toasts: &adw::ToastOverlay) {
    let shell = Shell::get();
    let say = Rc::clone(&shell.say);
    let state = connection.session.state();
    let roster: Vec<MemberDto> = connection
        .chats
        .members()
        .map(|members| members.to_vec())
        .unwrap_or_default();
    let addable = group_rules::addable(
        chat,
        &roster,
        connection.chats.reader(),
        state.assistant.as_ref().map(|assistant| assistant.user_id),
    );
    if addable.is_empty() {
        ui::toast(toasts, &group_text::everyone_is_in(say.as_ref()));
        return;
    }
    let (dialog, view, _) = ui::dialog(say.get("Add Members"), 400, 420);
    let (header, save) = ui::dialog_bar(&dialog, say.get("Cancel"), say.get("Save"), false);
    view.add_top_bar(&header);
    let group = adw::PreferencesGroup::new();
    let mut checks = Vec::new();
    for person in &addable {
        let check = gtk::CheckButton::new();
        let row = adw::ActionRow::builder()
            .title(&person.display_name)
            .activatable_widget(&check)
            .build();
        row.add_prefix(&check);
        group.add(&row);
        checks.push((person.id, check));
    }
    let page = adw::PreferencesPage::new();
    page.add(&group);
    view.set_content(Some(&page));
    let (weak, connection, toasts, id) = (
        dialog.downgrade(),
        Rc::clone(connection),
        toasts.clone(),
        chat.id,
    );
    save.connect_clicked(move |_| {
        let chosen: Vec<i64> = checks
            .iter()
            .filter(|(_, check)| check.is_active())
            .map(|(id, _)| *id)
            .collect();
        if let Some(dialog) = weak.upgrade() {
            dialog.close();
        }
        if chosen.is_empty() {
            return;
        }
        let (connection, toasts) = (Rc::clone(&connection), toasts.clone());
        ui::spawn(async move {
            let answer = model(&connection).add(id, &chosen).await;
            after(answer, &toasts);
        });
    });
    if let Some(parent) = ui::main_parent() {
        dialog.present(Some(&parent));
    }
}

/// A change's answer: said when refused, redrawn either way.
fn after(
    answer: fc_core::store::StoreResult<Option<fc_core::protocol::ApiError>>,
    toasts: &adw::ToastOverlay,
) {
    match answer {
        Ok(None) => {}
        Ok(Some(error)) => {
            crate::fc_info!("group: change refused ({})", error.code);
            let shell = Shell::get();
            ui::toast(
                toasts,
                &group_text::failure(&error, false, shell.say.as_ref()),
            );
        }
        Err(error) => crate::fc_warn!("group: the cache refused ({})", log::store_error(&error)),
    }
    redraw();
}

fn model(connection: &Connection) -> GroupModel {
    GroupModel::new(Rc::clone(&connection.api), Rc::clone(&connection.chats))
}
