//! The birthday editor (the Mac's `MyBirthdayView` and `MemberBirthdayView`): a day and a month,
//! with NO year (docs/protocol.md, "Birthdays"). Settings opens it for the reader's own; the owner's
//! console opens it for any member, the owner included. February has 29 (`birthday_rules`).

use std::rc::Rc;

use adw::prelude::*;
use fc_core::protocol::{BirthdayDto, MemberDto};
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::family::FamilyModel;
use fc_logic::settings::{birthday_rules, settings_text};

use crate::services::log;
use crate::settings::ui;
use crate::shell::Shell;

/// Whose birthday is being edited.
#[derive(Debug, Clone)]
pub enum Whose {
    /// The reader's own (`PUT /me/birthday`).
    Mine,
    /// A member's, by the owner (`PUT /families/…/members/{id}/birthday`).
    Member(MemberDto),
}

/// Presents the editor over `parent`; `done` runs after a save or a removal was taken and the
/// session (own) or the roster (a member's) was read again.
pub fn show(
    parent: &impl IsA<gtk::Widget>,
    whose: Whose,
    held: Option<BirthdayDto>,
    done: impl Fn() + 'static,
) {
    let shell = Shell::get();
    let say = Rc::clone(&shell.say);
    let (dialog, view, _toasts) = ui::dialog(say.get("Birthday"), 420, 340);
    let (header, save) = ui::dialog_bar(&dialog, say.get("Cancel"), say.get("Save"), false);
    view.add_top_bar(&header);

    let group = adw::PreferencesGroup::new();
    match &whose {
        Whose::Mine => group.set_description(Some(say.get(
            "A day and a month, with no year — so being wished a happy birthday never means publishing your age.",
        ))),
        Whose::Member(member) => {
            group.set_title(&say.format("Birthday for %@", &[&member.display_name]));
            group.set_description(Some(say.get(
                "A day and a month, with no year. Everyone in the family sees it.",
            )));
        }
    }
    let (month, day) = birthday_rules::start(held.as_ref());
    let months: Vec<String> = (1..=12)
        .map(|m| birthday_rules::month_name(m, &shell.culture))
        .collect();
    let month_names: Vec<&str> = months.iter().map(String::as_str).collect();
    let month_row = adw::ComboRow::builder()
        .title(say.get("Month"))
        .model(&gtk::StringList::new(&month_names))
        .selected(u32::try_from(month - 1).unwrap_or(0))
        .build();
    let day_row = adw::SpinRow::with_range(1.0, f64::from(birthday_rules::days_in(month)), 1.0);
    day_row.set_title(say.get("Day"));
    day_row.set_value(f64::from(day));
    {
        let day_row = day_row.clone();
        month_row.connect_selected_notify(move |row| {
            let month = i32::try_from(row.selected()).unwrap_or(0) + 1;
            let days = f64::from(birthday_rules::days_in(month));
            day_row.adjustment().set_upper(days);
            if day_row.value() > days {
                day_row.set_value(days);
            }
        });
    }
    group.add(&month_row);
    group.add(&day_row);
    let problem = ui::problem_label();
    group.add(&problem);

    let page = adw::PreferencesPage::new();
    page.add(&group);
    let done: Rc<dyn Fn()> = Rc::new(done);
    if held.is_some() {
        let removal = adw::PreferencesGroup::new();
        let whose = whose.clone();
        let weak = dialog.downgrade();
        let problem = problem.clone();
        let done_removing = Rc::clone(&done);
        removal.add(&ui::button_row(
            say.get("Remove Birthday"),
            true,
            move || {
                let (whose, weak, problem, done) = (
                    whose.clone(),
                    weak.clone(),
                    problem.clone(),
                    Rc::clone(&done_removing),
                );
                ui::spawn(async move {
                    match write(&whose, None).await {
                        Ok(()) => {
                            done();
                            if let Some(dialog) = weak.upgrade() {
                                dialog.close();
                            }
                        }
                        Err(text) => ui::show_problem(&problem, &text),
                    }
                });
            },
        ));
        page.add(&removal);
    }
    connect_save(&dialog, &save, whose, &month_row, &day_row, &problem, done);
    view.set_content(Some(&page));
    dialog.present(Some(parent));
}

fn connect_save(
    dialog: &adw::Dialog,
    save: &gtk::Button,
    whose: Whose,
    month_row: &adw::ComboRow,
    day_row: &adw::SpinRow,
    problem: &gtk::Label,
    done: Rc<dyn Fn()>,
) {
    let weak = dialog.downgrade();
    let (month_row, day_row, problem) = (month_row.clone(), day_row.clone(), problem.clone());
    let busy = ui::busy(save);
    save.connect_clicked(move |_| {
        let month = i32::try_from(month_row.selected()).unwrap_or(0) + 1;
        // A spin row's value is a whole number between 1 and 31.
        let day = day_row.value().round() as i32;
        let (whose, weak, problem, done, busy) = (
            whose.clone(),
            weak.clone(),
            problem.clone(),
            Rc::clone(&done),
            Rc::clone(&busy),
        );
        busy(true);
        ui::spawn(async move {
            let answer = write(&whose, Some(BirthdayDto { month, day })).await;
            busy(false);
            match answer {
                Ok(()) => {
                    done();
                    if let Some(dialog) = weak.upgrade() {
                        dialog.close();
                    }
                }
                Err(text) => ui::show_problem(&problem, &text),
            }
        });
    });
}

/// Sends the change (`None`: remove), then reads back what it changed. Answers the sentence to
/// show when it failed.
async fn write(whose: &Whose, birthday: Option<BirthdayDto>) -> Result<(), String> {
    let shell = Shell::get();
    let Some(connection) = shell.connection() else {
        return Err(String::new());
    };
    let say: &dyn StringCatalog = shell.say.as_ref();
    if let Some(day) = birthday
        && !birthday_rules::ok(day.month, day.day)
    {
        return Err(say.get("That date doesn't exist.").to_string());
    }
    let api = &connection.api;
    let failed = match (whose, birthday) {
        (Whose::Mine, Some(day)) => api
            .set_my_birthday(day.month, day.day)
            .await
            .error()
            .cloned(),
        (Whose::Mine, None) => api.clear_my_birthday().await.error().cloned(),
        (Whose::Member(member), Some(day)) => api
            .set_member_birthday(member.id, day.month, day.day)
            .await
            .error()
            .cloned(),
        (Whose::Member(member), None) => {
            api.clear_member_birthday(member.id).await.error().cloned()
        }
    };
    if let Some(error) = failed {
        crate::fc_info!("birthday: refused ({})", error.code);
        return Err(settings_text::birthday_failure(&error, say));
    }
    let read = match whose {
        Whose::Mine => connection.session.refresh().await.map(|_| ()),
        Whose::Member(_) => {
            FamilyModel::new(Rc::clone(&connection.api), Rc::clone(&connection.chats))
                .read()
                .await
                .map(|_| ())
        }
    };
    if let Err(error) = read {
        crate::fc_warn!("birthday: the cache refused ({})", log::store_error(&error));
    }
    Ok(())
}
