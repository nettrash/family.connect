//! The dialogs behind Settings' rows (the Mac's sheets): Change Password…, Delete Account…, New
//! Family…, Join Another Family…, and Leave Family's question. Every sentence and every refusal is
//! fc-logic's (`settings_text`, `door_sentences`, `another_family`, `LeaveContext`).

use std::rc::Rc;

use adw::prelude::*;
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::doorway::FamilyDoor;
use fc_logic::families::another_family;
use fc_logic::family::FamilyModel;
use fc_logic::paid_families::{self, Days};
use fc_logic::settings::{LeaveContext, settings_text};

use crate::services::log;
use crate::settings::ui;
use crate::shell::Shell;

/// Change Password… (`PasswordView`): the current password is the proof, a new one of at least 8
/// scalars typed twice; the other devices are signed out, this one stays.
pub fn change_password(parent: &impl IsA<gtk::Widget>) {
    let shell = Shell::get();
    let say = Rc::clone(&shell.say);
    let (dialog, view, _) = ui::dialog(say.get("Change Password"), 420, 380);
    let (header, save) = ui::dialog_bar(&dialog, say.get("Cancel"), say.get("Save"), false);
    view.add_top_bar(&header);
    let group = adw::PreferencesGroup::new();
    let current = adw::PasswordEntryRow::builder()
        .title(say.get("Current Password"))
        .build();
    let fresh = adw::PasswordEntryRow::builder()
        .title(say.get("New Password"))
        .build();
    let again = adw::PasswordEntryRow::builder()
        .title(say.get("Confirm New Password"))
        .build();
    group.add(&current);
    group.add(&fresh);
    group.add(&again);
    group.add(&ui::footnote(say.get(
        "Your other devices will be signed out. This one stays signed in.",
    )));
    let problem = ui::problem_label();
    group.add(&problem);
    let page = adw::PreferencesPage::new();
    page.add(&group);
    view.set_content(Some(&page));

    let weak = dialog.downgrade();
    let busy = ui::busy(&save);
    let told: gtk::Widget = parent.clone().upcast();
    save.connect_clicked(move |_| {
        let shell = Shell::get();
        let say: &dyn StringCatalog = shell.say.as_ref();
        let (old, new, confirm) = (
            current.text().to_string(),
            fresh.text().to_string(),
            again.text().to_string(),
        );
        if let Some(text) = settings_text::password_problem(&new, &confirm, say) {
            ui::show_problem(&problem, &text);
            return;
        }
        let Some(connection) = shell.connection() else {
            return;
        };
        let (weak, problem, busy, parent) = (
            weak.clone(),
            problem.clone(),
            Rc::clone(&busy),
            told.clone(),
        );
        busy(true);
        ui::spawn(async move {
            let refused = connection.session.change_password(&old, &new).await;
            busy(false);
            let shell = Shell::get();
            let say: &dyn StringCatalog = shell.say.as_ref();
            match refused {
                Some(error) => {
                    crate::fc_info!("password: change refused ({})", error.code);
                    ui::show_problem(
                        &problem,
                        &settings_text::password_change_failure(&error, say),
                    );
                }
                None => {
                    if let Some(dialog) = weak.upgrade() {
                        dialog.close();
                    }
                    ui::tell(
                        &parent,
                        say.get("Password changed"),
                        say.get("Your other devices have been signed out."),
                        say.get("OK"),
                    );
                }
            }
        });
    });
    dialog.present(Some(parent));
}

/// Delete Account… (`DeleteAccountView`): what happens — every consequence the server carries out,
/// the owner's own among them — the password as proof, and one more question: it is immediate and
/// final.
pub fn delete_account(parent: &impl IsA<gtk::Widget>) {
    let shell = Shell::get();
    let say = Rc::clone(&shell.say);
    let owner = shell
        .connection()
        .is_some_and(|connection| connection.session.state().is_owner());
    let (dialog, view, _) = ui::dialog(
        say.get("Delete Account"),
        440,
        if owner { 640 } else { 560 },
    );
    let (header, delete) = ui::dialog_bar(&dialog, say.get("Cancel"), say.get("Delete"), true);
    view.add_top_bar(&header);
    let what = adw::PreferencesGroup::builder()
        .title(say.get("What happens"))
        .build();
    // Literal keys, each its own `say.get`: the catalogue's generator collects only those.
    let mut happens = vec![
        ("avatar-default-symbolic", say.get("Your account, password, profile picture and birthday are deleted, and every device you are signed in on is signed out.")),
        ("user-trash-symbolic", say.get("Your direct chats are deleted — for the other person too. So is your private chat with the assistant.")),
        ("chat-message-new-symbolic", say.get("Your messages in the family chat, your board notes and your reactions stay. They are shown from then on as “Deleted account”.")),
    ];
    if owner {
        happens.push(("go-home-symbolic", say.get("You own this family: ownership passes to the longest-standing remaining member. If you are its last member, the family is deleted with you — its chat, its board and its invite code.")));
    }
    for (icon, line) in happens {
        let row = adw::ActionRow::builder()
            .title(line)
            .use_markup(false)
            .title_lines(0)
            .activatable(false)
            .build();
        row.add_prefix(&gtk::Image::from_icon_name(icon));
        what.add(&row);
    }
    what.add(&ui::footnote(say.get(
        "There is no grace period and no way to cancel afterwards.",
    )));
    let proof = adw::PreferencesGroup::new();
    let password = adw::PasswordEntryRow::builder()
        .title(say.get("Password"))
        .build();
    proof.add(&password);
    proof.add(&ui::footnote(say.get(
        "Type your password to confirm it is you. Being signed in is not proof.",
    )));
    let problem = ui::problem_label();
    proof.add(&problem);
    let page = adw::PreferencesPage::new();
    page.add(&what);
    page.add(&proof);
    view.set_content(Some(&page));

    let weak = dialog.downgrade();
    let busy = ui::busy(&delete);
    delete.connect_clicked(move |button| {
        let shell = Shell::get();
        let say = Rc::clone(&shell.say);
        let typed = password.text().to_string();
        if typed.is_empty() {
            ui::show_problem(&problem, say.get("Type your password to confirm."));
            return;
        }
        let (weak, problem, busy) = (weak.clone(), problem.clone(), Rc::clone(&busy));
        ui::ask(
            button,
            say.get("Delete Account"),
            say.get("This happens immediately and cannot be undone."),
            say.get("Cancel"),
            say.get("Delete Account"),
            true,
            move || {
                let Some(connection) = Shell::get().connection() else {
                    return;
                };
                let (typed, weak, problem, busy) = (
                    typed.clone(),
                    weak.clone(),
                    problem.clone(),
                    Rc::clone(&busy),
                );
                busy(true);
                ui::spawn(async move {
                    let answer = connection.session.delete_account(&typed).await;
                    busy(false);
                    match answer {
                        Ok(None) => {
                            crate::fc_info!("account: deleted");
                            if let Some(dialog) = weak.upgrade() {
                                dialog.close();
                            }
                        }
                        Ok(Some(error)) => {
                            crate::fc_info!("account: delete refused ({})", error.code);
                            let shell = Shell::get();
                            ui::show_problem(
                                &problem,
                                &settings_text::delete_failure(&error, shell.say.as_ref()),
                            );
                        }
                        Err(error) => {
                            crate::fc_warn!(
                                "account: the cache refused ({})",
                                log::store_error(&error)
                            );
                        }
                    }
                });
            },
        );
    });
    dialog.present(Some(parent));
}

/// New Family… (`CreateFamilyView(anotherFamily: true)`): the family is added and SHOWN. On a
/// server that charges for a new family, without an active subscription, the entry stays and says
/// where a family is started instead of offering the form (docs/protocol.md, "Paid families").
pub fn new_family(parent: &impl IsA<gtk::Widget>) {
    let shell = Shell::get();
    let say = Rc::clone(&shell.say);
    let (dialog, view, _) = ui::dialog(say.get("Create a Family"), 440, 320);
    let charged = shell.connection().is_some_and(|connection| {
        paid_families::create_door(&connection.session.state(), true)
            == paid_families::CreateDoor::StartInAppleApp
    });
    if charged {
        view.add_top_bar(&adw::HeaderBar::new());
        let group = adw::PreferencesGroup::builder()
            .description(paid_families::start_in_apple_app(say.as_ref()))
            .build();
        let page = adw::PreferencesPage::new();
        page.add(&group);
        view.set_content(Some(&page));
        dialog.present(Some(parent));
        return;
    }
    let (header, create) =
        ui::dialog_bar(&dialog, say.get("Cancel"), say.get("Create Family"), false);
    create.set_sensitive(false);
    view.add_top_bar(&header);
    let group = adw::PreferencesGroup::builder()
        .description(another_family::explanation(say.as_ref()))
        .build();
    let name = adw::EntryRow::builder()
        .title(say.get("Family name"))
        .build();
    {
        let create = create.clone();
        name.connect_changed(move |row| {
            create.set_sensitive(FamilyDoor::may_create(Some(&row.text())));
        });
    }
    group.add(&name);
    group.add(&ui::footnote(
        say.get("This names your family chat too. 1–64 characters."),
    ));
    let problem = ui::problem_label();
    group.add(&problem);
    let page = adw::PreferencesPage::new();
    page.add(&group);
    view.set_content(Some(&page));
    let weak = dialog.downgrade();
    let busy = ui::busy(&create);
    create.connect_clicked(move |_| {
        let Some(connection) = Shell::get().connection() else {
            return;
        };
        let typed = name.text().to_string();
        let (weak, problem, busy) = (weak.clone(), problem.clone(), Rc::clone(&busy));
        busy(true);
        ui::spawn(async move {
            let shell = Shell::get();
            let door = FamilyDoor::new(
                Rc::clone(&connection.api),
                Rc::clone(&connection.session),
                shell.say.clone(),
            );
            let answer = door.create(&typed).await;
            busy(false);
            door_answer(answer, &weak, &problem, "create");
        });
    });
    dialog.present(Some(parent));
}

/// Join Another Family… (`JoinFamilyView(anotherFamily: true)`): `joined` adds the family and
/// shows it; `pending` leaves the family shown where it is.
pub fn join_another(parent: &impl IsA<gtk::Widget>) {
    let shell = Shell::get();
    let say = Rc::clone(&shell.say);
    let (dialog, view, _) = ui::dialog(say.get("Join a Family"), 440, 300);
    let (header, join) = ui::dialog_bar(&dialog, say.get("Cancel"), say.get("Join"), false);
    join.set_sensitive(false);
    view.add_top_bar(&header);
    let group = adw::PreferencesGroup::new();
    let code = adw::EntryRow::builder()
        .title(say.get("Invite code"))
        .build();
    {
        let join = join.clone();
        code.connect_changed(move |row| {
            join.set_sensitive(FamilyDoor::may_join(Some(&row.text())));
        });
    }
    group.add(&code);
    group.add(&ui::footnote(say.get(
        "Any family member can read the code to you; the owner finds it in Settings.",
    )));
    let problem = ui::problem_label();
    group.add(&problem);
    let page = adw::PreferencesPage::new();
    page.add(&group);
    view.set_content(Some(&page));
    let weak = dialog.downgrade();
    let busy = ui::busy(&join);
    join.connect_clicked(move |_| {
        let Some(connection) = Shell::get().connection() else {
            return;
        };
        let typed = code.text().to_string();
        let (weak, problem, busy) = (weak.clone(), problem.clone(), Rc::clone(&busy));
        busy(true);
        ui::spawn(async move {
            let shell = Shell::get();
            let door = FamilyDoor::new(
                Rc::clone(&connection.api),
                Rc::clone(&connection.session),
                shell.say.clone(),
            );
            let answer = door.join(&typed).await;
            busy(false);
            door_answer(answer, &weak, &problem, "join");
        });
    });
    dialog.present(Some(parent));
}

fn door_answer(
    answer: fc_core::store::StoreResult<Option<String>>,
    dialog: &glib::WeakRef<adw::Dialog>,
    problem: &gtk::Label,
    what: &str,
) {
    match answer {
        Ok(None) => {
            crate::fc_info!("family: {what} taken");
            if let Some(dialog) = dialog.upgrade() {
                dialog.close();
            }
        }
        Ok(Some(text)) => ui::show_problem(problem, &text),
        Err(error) => crate::fc_warn!("family: the cache refused ({})", log::store_error(&error)),
    }
}

/// Leave Family: the question is drawn from a FRESH read (who inherits is a prediction any join or
/// leave changes; no successor on it means leaving deletes the family), names the family when the
/// account has several, and the answer names who the family went to.
pub fn leave_family(parent: &impl IsA<gtk::Widget>, toasts: &adw::ToastOverlay) {
    let Some(connection) = Shell::get().connection() else {
        return;
    };
    let (parent, toasts): (gtk::Widget, adw::ToastOverlay) =
        (parent.clone().upcast(), toasts.clone());
    ui::spawn(async move {
        let shell = Shell::get();
        let say = Rc::clone(&shell.say);
        let family = FamilyModel::new(Rc::clone(&connection.api), Rc::clone(&connection.chats));
        let state = connection.session.state();
        let context = match family.leave_would().await {
            Ok(Ok(would)) => LeaveContext::for_leaving(
                state.is_owner(),
                would.deletes_family,
                would.successor.as_ref(),
            ),
            Ok(Err(error)) => {
                crate::fc_info!("leave: the fresh read failed ({})", error.code);
                None
            }
            Err(error) => {
                crate::fc_warn!("leave: the cache refused ({})", log::store_error(&error));
                None
            }
        };
        let Some(mut context) = context else {
            ui::toast(&toasts, &settings_text::leave_failed(say.as_ref()));
            return;
        };
        if state.all_families().len() >= 2
            && let Some(name) = state.family.as_ref().map(|family| family.name.clone())
        {
            context = context.in_family(&name);
        }
        let leaving = state.current_family_id;
        let toasts_after = toasts.clone();
        let parent_after = parent.clone();
        ui::ask(
            &parent,
            &settings_text::leave_title(&context, say.as_ref()),
            &settings_text::leave_message(&context, say.as_ref()),
            say.get("Cancel"),
            &settings_text::leave_button(&context, say.as_ref()),
            true,
            move || {
                let (connection, toasts, parent) = (
                    Rc::clone(&connection),
                    toasts_after.clone(),
                    parent_after.clone(),
                );
                ui::spawn(async move {
                    leave(connection, leaving, &parent, &toasts).await;
                });
            },
        );
    });
}

async fn leave(
    connection: Rc<crate::shell::Connection>,
    family_id: Option<i64>,
    parent: &gtk::Widget,
    toasts: &adw::ToastOverlay,
) {
    let shell = Shell::get();
    let say = Rc::clone(&shell.say);
    let family = FamilyModel::new(Rc::clone(&connection.api), Rc::clone(&connection.chats));
    match family.leave(family_id).await {
        Err(error) => {
            crate::fc_info!("leave: refused ({})", error.code);
            ui::toast(toasts, &settings_text::leave_failed(say.as_ref()));
        }
        Ok(heir) => {
            crate::fc_info!("leave: left family {family_id:?}");
            let heir_name = heir
                .and_then(|id| connection.chats.member(id).ok().flatten())
                .map(|member| member.display_name);
            let gone = match family_id {
                Some(id) => connection.session.family_gone(id).await,
                None => connection.session.refresh().await.map(|_| ()),
            };
            if let Err(error) = gone {
                crate::fc_warn!("leave: the cache refused ({})", log::store_error(&error));
            }
            if let Some(name) = heir_name {
                let (title, message) = settings_text::ownership_passed(&name, say.as_ref());
                ui::tell(parent, &title, &message, say.get("OK"));
            }
        }
    }
}

/// Statistics… (the Mac's `StatisticsView`, Windows' dialog): the family's totals, what it sent,
/// what the assistant did, and who sends what — the rows exactly as the server answered them,
/// never summed into the totals (the gap is the reader's block list).
pub fn statistics(parent: &impl IsA<gtk::Widget>) {
    let shell = Shell::get();
    let say = Rc::clone(&shell.say);
    let (dialog, view, _) = ui::dialog(say.get("Statistics"), 420, 520);
    view.add_top_bar(&adw::HeaderBar::new());
    let page = adw::PreferencesPage::new();
    view.set_content(Some(&page));
    let spinner = adw::Spinner::builder()
        .width_request(32)
        .height_request(32)
        .margin_top(24)
        .build();
    let waiting = adw::PreferencesGroup::new();
    waiting.add(&spinner);
    page.add(&waiting);
    dialog.present(Some(parent));
    load_statistics(page, waiting);
}

fn load_statistics(page: adw::PreferencesPage, waiting: adw::PreferencesGroup) {
    let Some(connection) = Shell::get().connection() else {
        return;
    };
    ui::spawn(async move {
        let answer = FamilyModel::new(Rc::clone(&connection.api), Rc::clone(&connection.chats))
            .stats()
            .await;
        page.remove(&waiting);
        let shell = Shell::get();
        let say: &dyn StringCatalog = shell.say.as_ref();
        let stats = match answer {
            Ok(stats) => stats,
            Err(error) => {
                crate::fc_info!("statistics: refused ({})", error.code);
                let failed = adw::PreferencesGroup::builder()
                    .title(say.get("Couldn't load statistics"))
                    .description(say.get("Check your connection and try again."))
                    .build();
                let (page_again, failed_again) = (page.clone(), failed.clone());
                failed.add(&ui::button_row(say.get("Retry"), false, move || {
                    page_again.remove(&failed_again);
                    let waiting = adw::PreferencesGroup::new();
                    page_again.add(&waiting);
                    load_statistics(page_again.clone(), waiting);
                }));
                page.add(&failed);
                return;
            }
        };
        let row = |group: &adw::PreferencesGroup, title: &str, value: String| {
            let row = adw::ActionRow::builder().title(title).build();
            let label = gtk::Label::new(Some(&value));
            label.add_css_class("dim-label");
            row.add_suffix(&label);
            group.add(&row);
        };
        let separator = shell.numbers.decimal;
        let grouped = |n: i64| shell.numbers.group(n);
        let totals = &stats.totals;
        let family = adw::PreferencesGroup::builder()
            .title(say.get("The family"))
            .build();
        row(
            &family,
            say.get("Members"),
            grouped(i64::from(totals.members)),
        );
        row(&family, say.get("Messages"), grouped(totals.messages));
        row(&family, say.get("Board notes"), grouped(totals.board_notes));
        page.add(&family);

        // A PAID family's allowance (docs/protocol.md, "Family statistics", amended 2026-10-30):
        // a row per limit the server has on, and the day the monthly ones start again. No section
        // for a free family, or on a server that charges nothing.
        if let Some(allowance) = &stats.allowance {
            let used = adw::PreferencesGroup::builder()
                .title(paid_families::allowance_title(say))
                .build();
            for (title, value) in paid_families::allowance_rows(allowance, say, separator, &grouped)
            {
                row(&used, &title, value);
            }
            let days = Days::new(
                &shell.culture,
                shell.zone.as_ref(),
                time::OffsetDateTime::now_utc(),
            );
            if let Some(renews) = paid_families::allowance_renews(allowance, say, &days) {
                used.add(&ui::footnote(&renews));
            }
            page.add(&used);
        }

        let files = totals.attachments.clone().unwrap_or_default();
        let sent = adw::PreferencesGroup::builder()
            .title(say.get("Attachments"))
            .build();
        row(&sent, say.get("Photos"), grouped(i64::from(files.photo)));
        row(&sent, say.get("Videos"), grouped(i64::from(files.video)));
        row(&sent, say.get("Audio"), grouped(i64::from(files.audio)));
        row(&sent, say.get("Files"), grouped(i64::from(files.file)));
        row(
            &sent,
            say.get("Locations"),
            grouped(i64::from(files.location)),
        );
        let size = |bytes: i64| fc_core::text::media_text::display_size(bytes.max(0), separator);
        row(&sent, say.get("Sent"), size(files.bytes));
        if let Some(stored) = files.stored_bytes {
            row(&sent, say.get("On disk"), size(stored));
        }
        if let Some(saved) = settings_text::saved(totals.attachments.as_ref()) {
            sent.add(&ui::footnote(&say.format(
                "%@ saved by storing one copy of identical files.",
                &[&size(saved)],
            )));
        }
        page.add(&sent);

        if let Some(ai) = totals
            .ai
            .as_ref()
            .filter(|ai| settings_text::assistant_used(Some(ai)))
        {
            let assistant = adw::PreferencesGroup::builder()
                .title(say.get("Assistant"))
                .build();
            row(
                &assistant,
                say.get("Questions"),
                grouped(i64::from(ai.questions)),
            );
            row(
                &assistant,
                say.get("Tokens"),
                grouped(ai.prompt_tokens + ai.completion_tokens),
            );
            if ai.images > 0 {
                row(
                    &assistant,
                    say.get("Pictures"),
                    grouped(i64::from(ai.images)),
                );
            }
            if ai.transcripts > 0 {
                row(
                    &assistant,
                    say.get("Recordings as text"),
                    grouped(i64::from(ai.transcripts)),
                );
                // A speech model is paid by the length of the sound, so the time too (the Mac's
                // StatisticsView); written as a clock, as Windows and the web write it.
                row(
                    &assistant,
                    say.get("Recording time"),
                    fc_text::call_record::duration((ai.transcript_duration_ms.max(0) + 500) / 1000),
                );
            }
            if ai.searches > 0 {
                row(
                    &assistant,
                    say.get("Web searches"),
                    grouped(i64::from(ai.searches)),
                );
            }
            page.add(&assistant);
        }

        let who = adw::PreferencesGroup::builder()
            .title(say.get("Who sends what"))
            .build();
        for member in stats.members.as_deref().unwrap_or_default() {
            let line = adw::ActionRow::builder()
                .title(&member.display_name)
                .subtitle(settings_text::member_line(member, say, separator))
                .build();
            let count = gtk::Label::new(Some(&grouped(member.messages)));
            count.add_css_class("dim-label");
            line.add_suffix(&count);
            who.add(&line);
        }
        page.add(&who);
    });
}
