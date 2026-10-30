//! One member row of the Family dialog and its menu (§12; the Mac's roster row): Message (a direct
//! chat, opened in the main window — the dialog closes), Report… and Block/Unblock (anybody about
//! anybody but themselves — the owner included), and the owner's Birthday…, Reset Password… (only
//! where `family_rules::may_reset_password` says the server will take it) and Remove from Family.
//! What a row offers is `family_text::tools_for`'s answer.

use std::rc::Rc;

use adw::prelude::*;
use fc_core::protocol::MemberDto;
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::family_console::{MemberTools, family_text};

use super::birthday::{self, Whose};
use super::{FamilyDialog, family_model, member_subtitle, words};
use crate::services::log;
use crate::settings::ui;
use crate::shell::Shell;

/// The row: face, name, "@username · birthday", the Owner pill, and the menu.
pub fn row(
    family: &Rc<FamilyDialog>,
    person: &MemberDto,
    tools: MemberTools,
    blocked: bool,
) -> adw::ActionRow {
    let shell = Shell::get();
    let say: &dyn StringCatalog = shell.say.as_ref();
    let row = adw::ActionRow::builder()
        .title(&person.display_name)
        .subtitle(member_subtitle(person, say))
        .build();
    row.add_prefix(&ui::face(&person.display_name, person.id, 32));
    if person.owner() {
        row.add_suffix(&ui::owner_pill(say.get("Owner")));
    }
    let menu = gio::Menu::new();
    let actions = gio::SimpleActionGroup::new();
    let add = |name: &str, label: &str, run: Box<dyn Fn()>| {
        let action = gio::SimpleAction::new(name, None);
        action.connect_activate(move |_, _| run());
        actions.add_action(&action);
        menu.append(Some(label), Some(&format!("member.{name}")));
    };
    let weak = Rc::downgrade(family);
    let them = person.clone();
    if tools.message {
        let (weak, them) = (weak.clone(), them.clone());
        add(
            "message",
            say.get("Message"),
            Box::new(move || {
                if let Some(family) = weak.upgrade() {
                    message(&family, &them);
                }
            }),
        );
    }
    if tools.safety {
        let (weak, reported) = (weak.clone(), them.clone());
        add(
            "report",
            say.get("Report…"),
            Box::new(move || {
                if let Some(family) = weak.upgrade() {
                    report(&family, &reported);
                }
            }),
        );
        let (weak_block, id) = (Rc::downgrade(family), them.id);
        add(
            "block",
            if blocked {
                say.get("Unblock")
            } else {
                say.get("Block")
            },
            Box::new(move || {
                if let Some(family) = weak_block.upgrade() {
                    block(&family, id, !blocked);
                }
            }),
        );
    }
    if tools.owner_menu {
        let (weak, them) = (weak.clone(), them.clone());
        add(
            "birthday",
            say.get("Birthday…"),
            Box::new(move || {
                if let Some(family) = weak.upgrade() {
                    let refresh = Rc::downgrade(&family);
                    birthday::show(
                        &family.dialog,
                        Whose::Member(them.clone()),
                        them.birthday,
                        move || {
                            if let Some(family) = refresh.upgrade() {
                                family.rebuild();
                            }
                        },
                    );
                }
            }),
        );
    }
    if tools.reset_password {
        let (weak, them) = (weak.clone(), them.clone());
        add(
            "reset",
            say.get("Reset Password…"),
            Box::new(move || {
                if let Some(family) = weak.upgrade() {
                    reset_password(&family, &them);
                }
            }),
        );
    }
    if tools.removable {
        let (weak, them) = (weak.clone(), them.clone());
        add(
            "remove",
            say.get("Remove from Family"),
            Box::new(move || {
                if let Some(family) = weak.upgrade() {
                    remove(&family, &them);
                }
            }),
        );
    }
    if menu.n_items() > 0 {
        let button = gtk::MenuButton::builder()
            .icon_name("view-more-symbolic")
            .menu_model(&menu)
            .valign(gtk::Align::Center)
            .tooltip_text(&person.display_name)
            .build();
        button.add_css_class("flat");
        button.insert_action_group("member", Some(&actions));
        row.add_suffix(&button);
        // The same menu on the row itself, for the keyboard (Menu key, Shift+F10).
        row.insert_action_group("member", Some(&actions));
    }
    row
}

/// "Message": the direct chat, get-or-create (idempotent on the server), kept in the cache, then
/// opened in the main window through `app.open-chat` — which waits for the chats as every route
/// does.
pub fn message(family: &Rc<FamilyDialog>, person: &MemberDto) {
    let Some(connection) = Shell::get().connection() else {
        return;
    };
    let (weak, user) = (Rc::downgrade(family), person.id);
    ui::spawn(async move {
        let answer = connection.api.direct_chat(user).await.into_result();
        let Some(family) = weak.upgrade() else {
            return;
        };
        match answer {
            Ok(answer) => {
                if let Err(error) = connection.chats.upsert(&answer.chat) {
                    crate::fc_warn!("family: the cache refused ({})", log::store_error(&error));
                }
                family.dialog.close();
                crate::fc_info!("family: direct chat {} opened", answer.chat.id);
                Shell::get().app.activate_action(
                    crate::seams::actions::OPEN_CHAT.name,
                    Some(&answer.chat.id.to_variant()),
                );
            }
            Err(error) => family.failed(&error),
        }
    });
}

/// Block or unblock — never optimistic: a block that failed silently would hide rows the reader
/// does not know are hidden.
fn block(family: &Rc<FamilyDialog>, user: i64, blocked: bool) {
    let Some(connection) = Shell::get().connection() else {
        return;
    };
    let weak = Rc::downgrade(family);
    ui::spawn(async move {
        let answer = family_model(&connection).block(user, blocked).await;
        let Some(family) = weak.upgrade() else {
            return;
        };
        match answer {
            Ok(None) => family.rebuild(),
            Ok(Some(error)) => {
                crate::fc_info!("family: block refused ({})", error.code);
                let say = Rc::clone(&Shell::get().say);
                ui::toast(
                    &family.toasts,
                    say.get("Couldn't change that right now. Try again."),
                );
            }
            Err(error) => {
                crate::fc_warn!("family: the cache refused ({})", log::store_error(&error))
            }
        }
    });
}

/// Remove from Family — asked first.
pub fn remove(family: &Rc<FamilyDialog>, person: &MemberDto) {
    let say = Rc::clone(&Shell::get().say);
    let (weak, user) = (Rc::downgrade(family), person.id);
    ui::ask(
        &family.dialog,
        &say.format("Remove %@ from the family?", &[&person.display_name]),
        "",
        say.get("Cancel"),
        say.get("Remove from Family"),
        true,
        move || {
            let Some(connection) = Shell::get().connection() else {
                return;
            };
            let weak = weak.clone();
            ui::spawn(async move {
                let answer = connection.api.remove_member(user).await.into_result();
                let Some(family) = weak.upgrade() else {
                    return;
                };
                match answer {
                    Ok(_) => {
                        crate::fc_info!("family: member {user} removed");
                        if let Err(error) = family_model(&connection).read().await {
                            crate::fc_warn!(
                                "family: the cache refused ({})",
                                log::store_error(&error)
                            );
                        }
                        connection.live.refresh();
                        family.rebuild();
                    }
                    Err(error) => family.failed(&error),
                }
            });
        },
    );
}

/// Report… about a PERSON (no message): the reason, who will read it (MANDATORY — the protocol's
/// disclosure), and the operator's own contact for when the problem is the owner.
fn report(family: &Rc<FamilyDialog>, person: &MemberDto) {
    let shell = Shell::get();
    let say = Rc::clone(&shell.say);
    let (dialog, view, _) = ui::dialog(say.get("Report"), 420, 420);
    let (header, send) = ui::dialog_bar(&dialog, say.get("Cancel"), say.get("Report"), true);
    view.add_top_bar(&header);
    let group = adw::PreferencesGroup::builder()
        .title(say.format("Why are you reporting %@?", &[&person.display_name]))
        .description(family_text::report_disclosure(false, say.as_ref()))
        .build();
    let mut first: Option<gtk::CheckButton> = None;
    let mut choices = Vec::new();
    for (code, _) in family_text::REASONS {
        let check = gtk::CheckButton::new();
        if let Some(first) = &first {
            check.set_group(Some(first));
        } else {
            first = Some(check.clone());
        }
        check.set_active(code == family_text::FIRST_REASON);
        let row = adw::ActionRow::builder()
            .title(words::reason_label(code, say.as_ref()))
            .activatable_widget(&check)
            .build();
        row.add_prefix(&check);
        group.add(&row);
        choices.push((code, check));
    }
    let page = adw::PreferencesPage::new();
    page.add(&group);
    if let Some(contact) = shell
        .connection()
        .and_then(|connection| connection.session.state().support_contact)
        .filter(|contact| !contact.is_empty())
    {
        // VERBATIM, selectable, never a link (docs/protocol.md, "Reporting a member").
        let owner = adw::PreferencesGroup::builder()
            .title(say.get("If the problem is the owner"))
            .build();
        let text = gtk::Label::builder()
            .label(contact)
            .selectable(true)
            .wrap(true)
            .xalign(0.0)
            .build();
        owner.add(&text);
        page.add(&owner);
    }
    view.set_content(Some(&page));
    let (weak_dialog, weak_family, user) = (dialog.downgrade(), Rc::downgrade(family), person.id);
    send.connect_clicked(move |_| {
        let Some(connection) = Shell::get().connection() else {
            return;
        };
        let reason = choices
            .iter()
            .find(|(_, check)| check.is_active())
            .map_or(family_text::FIRST_REASON, |(code, _)| *code);
        let (weak_dialog, weak_family) = (weak_dialog.clone(), weak_family.clone());
        ui::spawn(async move {
            let answer = connection
                .api
                .report(user, reason, None)
                .await
                .into_result();
            if let Some(dialog) = weak_dialog.upgrade() {
                dialog.close();
            }
            if let (Err(error), Some(family)) = (answer, weak_family.upgrade()) {
                crate::fc_info!("family: report refused ({})", error.code);
                let say = Rc::clone(&Shell::get().say);
                ui::toast(
                    &family.toasts,
                    say.get("Couldn't send the report. Try again."),
                );
            }
        });
    });
    dialog.present(Some(&family.dialog));
}

/// Reset Password… (the owner, for a member who forgot): a new password typed twice; the member
/// is signed out everywhere and needs it to sign back in.
fn reset_password(family: &Rc<FamilyDialog>, person: &MemberDto) {
    let shell = Shell::get();
    let say = Rc::clone(&shell.say);
    let (dialog, view, _) = ui::dialog(say.get("Reset Password"), 420, 380);
    let (header, reset) = ui::dialog_bar(&dialog, say.get("Cancel"), say.get("Reset"), true);
    view.add_top_bar(&header);
    let group = adw::PreferencesGroup::builder()
        .title(say.format("New password for %@", &[&person.display_name]))
        .description(say.format(
            "%@ will be signed out on every device and will need this password to sign back in. Tell it to them somewhere safe — the server has no way to email it.",
            &[&person.display_name],
        ))
        .build();
    let fresh = adw::PasswordEntryRow::builder()
        .title(say.get("New Password"))
        .build();
    let again = adw::PasswordEntryRow::builder()
        .title(say.get("Confirm New Password"))
        .build();
    group.add(&fresh);
    group.add(&again);
    let problem = ui::problem_label();
    group.add(&problem);
    let page = adw::PreferencesPage::new();
    page.add(&group);
    view.set_content(Some(&page));
    let (weak_dialog, weak_family, user, name) = (
        dialog.downgrade(),
        Rc::downgrade(family),
        person.id,
        person.display_name.clone(),
    );
    reset.connect_clicked(move |_| {
        let shell = Shell::get();
        let say: &dyn StringCatalog = shell.say.as_ref();
        let (new, confirm) = (fresh.text().to_string(), again.text().to_string());
        if let Some(text) = fc_logic::settings::settings_text::password_problem(&new, &confirm, say)
        {
            ui::show_problem(&problem, &text);
            return;
        }
        let Some(connection) = shell.connection() else {
            return;
        };
        let (weak_dialog, weak_family, problem, name) = (
            weak_dialog.clone(),
            weak_family.clone(),
            problem.clone(),
            name.clone(),
        );
        ui::spawn(async move {
            let answer = connection
                .api
                .reset_member_password(user, &new)
                .await
                .into_result();
            let shell = Shell::get();
            let say: &dyn StringCatalog = shell.say.as_ref();
            match answer {
                Ok(_) => {
                    if let Some(dialog) = weak_dialog.upgrade() {
                        dialog.close();
                    }
                    if let Some(family) = weak_family.upgrade() {
                        ui::tell(
                            &family.dialog,
                            say.get("Password reset"),
                            &say.format("%@ has been signed out everywhere.", &[&name]),
                            say.get("OK"),
                        );
                    }
                }
                Err(error) => {
                    crate::fc_info!("family: reset refused ({})", error.code);
                    let cap = connection.session.state().max_families_per_account;
                    let text = fc_logic::families::family_sentences::for_error(&error, cap, say)
                        .unwrap_or_else(|| {
                            say.get("Couldn't reset that password. Try again.")
                                .to_string()
                        });
                    ui::show_problem(&problem, &text);
                }
            }
        });
    });
    dialog.present(Some(&family.dialog));
}
