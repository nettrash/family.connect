//! The `family` group (P4, §27): the Family dialog over the main window lists the members (the
//! owner's console with them: invite code, join requests, reports, policy, limit); Invite… offers
//! the members of the owner's other family and invites one; a join request is approved; a member's
//! birthday is drawn in the roster; a member is removed (asked first); a group is created from the
//! New Group dialog and its Group Info drawn; the dialog follows a switch to another family; and
//! the account leaves that family (Leave's question, the fresh read). Screenshots: light, dark,
//! narrow.

use std::time::Duration;

use adw::prelude::*;
use fc_core::protocol::{ReportDto, UserDto};
use fc_core::text::string_catalog::StringCatalog;

use super::Harness;
use super::settings::{button, descendants, find, install, person, sign_in_as, world};
use crate::family;
use crate::windows::main_window::GatePage;

const WAIT: Duration = Duration::from_secs(15);

pub async fn run(h: &Harness) {
    let shell = h.shell();
    let fake = h.fake_server();
    install(&fake);
    let frank = person(&fake, "frank", "Frank");
    let gina = person(&fake, "gina", "Gina");
    let hank = person(&fake, "hank", "Hank");
    let ivy = person(&fake, "ivy", "Ivy");
    let jack = person(&fake, "jack", "Jack");
    let kim = person(&fake, "kim", "Kim");
    let own = fake.add_family("Frank's Family", frank);
    fake.join(gina, own, "member");
    fake.join(hank, own, "member");
    let other = fake.add_family("Ivy's Family", ivy);
    fake.join(frank, other, "member");
    fake.join(jack, other, "member");
    {
        let world = world();
        let mut world = world.borrow_mut();
        let id = world.next_id();
        world.requests.push((id, own, kim));
        let id = world.next_id();
        world.reports.push((
            own,
            ReportDto {
                id,
                reporter: UserDto::new(gina, "gina", "Gina"),
                reported: UserDto::new(hank, "hank", "Hank"),
                reason: "harassment".into(),
                message_excerpt: Some("A message the owner must read whole.".into()),
                ..ReportDto::default()
            },
        ));
    }

    // Gina has a picture (P2's avatars in P4's rows), and the server keeps a sticker pack (the
    // Family dialog's "Family Stickers").
    {
        let mut state = fake.state();
        state.pack_limits = Some((48, 512 * 1024));
        if let Some(user) = state.users.iter_mut().find(|u| u.id == gina) {
            user.avatar_version = 2;
        }
    }
    let Some(connection) = sign_in_as(h, "frank").await else {
        h.check("signed_in", false, "frank could not sign in");
        return;
    };
    let _ = connection.session.switch_family(own).await;
    connection.live.refresh();
    h.sleep(Duration::from_millis(800)).await;
    let main = shell.main_window().expect("the main window");
    let window: gtk::Window = main.window().clone().upcast();

    // ---- the dialog and its roster ------------------------------------------------------------------
    shell
        .app
        .activate_action(crate::seams::actions::OPEN_FAMILY.name, None);
    let listed = h
        .wait_until(WAIT, || {
            family::current().is_some_and(|open| {
                let ids: Vec<i64> = open
                    .member_rows
                    .borrow()
                    .iter()
                    .map(|(id, _)| *id)
                    .collect();
                [frank, gina, hank].iter().all(|id| ids.contains(id))
            })
        })
        .await;
    let Some(open) = family::current() else {
        h.check("members_listed", false, "no Family dialog");
        return;
    };
    // The console's requests and reports come from the server: wait for the read.
    let owner_console = h
        .wait_until(WAIT, || {
            button(&open.dialog, "Rotate").is_some()
                && button(&open.dialog, "Approve").is_some()
                && button(&open.dialog, "Mark as handled").is_some()
        })
        .await;
    h.check(
        "members_listed",
        listed && open.title.text() == "Frank's Family" && owner_console,
        format!(
            "title {:?}, rows {:?}, owner console {owner_console}",
            open.title.text(),
            open.member_rows
                .borrow()
                .iter()
                .map(|(id, _)| *id)
                .collect::<Vec<_>>()
        ),
    );
    // The dialog's entrance has finished.
    h.sleep(Duration::from_millis(900)).await;
    h.screenshot("dialog").await;

    // ---- Invite…: the other family's members, one invited -------------------------------------------
    let picker = open.invite_picker();
    h.sleep(Duration::from_millis(300)).await;
    let jack_row = picker
        .as_ref()
        .and_then(|picker| find::<adw::ActionRow>(picker, |row| row.title() == "Jack"));
    let ivy_offered = picker
        .as_ref()
        .and_then(|picker| find::<adw::ActionRow>(picker, |row| row.title() == "Ivy"))
        .is_some();
    h.screenshot("invite").await;
    if let Some(invite) = jack_row
        .as_ref()
        .and_then(|row| find::<gtk::Button>(row, |_| true))
    {
        invite.emit_clicked();
    }
    let invited = h
        .wait_until(WAIT, || {
            world()
                .borrow()
                .invitations
                .iter()
                .any(|invitation| invitation.user.id == jack && invitation.family_id == own)
        })
        .await;
    let pending_drawn = h
        .wait_until(Duration::from_secs(5), || {
            button(&open.dialog, "Withdraw Invitation").is_some()
        })
        .await;
    h.check(
        "invite",
        jack_row.is_some() && ivy_offered && invited && pending_drawn,
        format!(
            "offered jack {}, ivy {ivy_offered}; invited {invited}; pending drawn {pending_drawn}",
            jack_row.is_some()
        ),
    );
    if let Some(picker) = picker {
        picker.close();
    }

    // ---- a join request approved --------------------------------------------------------------------
    if let Some(approve) = button(&open.dialog, "Approve") {
        approve.emit_clicked();
    }
    let approved = h
        .wait_until(WAIT, || {
            open.member_rows.borrow().iter().any(|(id, _)| *id == kim)
        })
        .await;
    h.check(
        "join_request_approved",
        approved,
        "Kim joins Frank's Family",
    );

    // ---- a member's birthday in the roster ---------------------------------------------------------------
    let saved = connection
        .api
        .set_member_birthday(gina, 5, 9)
        .await
        .into_result()
        .is_ok();
    open.reload();
    let drawn = h
        .wait_until(WAIT, || {
            open.member_rows
                .borrow()
                .iter()
                .find(|(id, _)| *id == gina)
                .and_then(|(_, row)| row.subtitle())
                .is_some_and(|subtitle| subtitle.contains('·'))
        })
        .await;
    if let Some((_, row)) = open.member_rows.borrow().iter().find(|(id, _)| *id == gina) {
        let _ = WidgetExt::activate_action(row, "member.birthday", None);
    }
    h.sleep(Duration::from_millis(300)).await;
    h.screenshot("birthday").await;
    if let Some(dialog) = main.window().visible_dialog()
        && dialog.title() != open.dialog.title()
    {
        dialog.close();
    }
    h.check(
        "member_birthday",
        saved && drawn,
        "the owner's birthday for Gina, day and month in the roster",
    );

    // ---- remove a member (asked first) -------------------------------------------------------------------
    if let Some((_, row)) = open.member_rows.borrow().iter().find(|(id, _)| *id == hank) {
        let _ = WidgetExt::activate_action(row, "member.remove", None);
    }
    h.sleep(Duration::from_millis(300)).await;
    let alert = main
        .window()
        .visible_dialog()
        .and_downcast::<adw::AlertDialog>();
    let question = alert
        .as_ref()
        .and_then(|a| a.heading())
        .map(|t| t.to_string());
    h.screenshot("remove").await;
    if let Some(alert) = &alert {
        // What a click on its button does: the response, and the alert goes.
        alert.emit_by_name::<()>("response", &[&crate::settings::ui::CONFIRM]);
        alert.close();
    }
    let removed = h
        .wait_until(WAIT, || {
            !open.member_rows.borrow().iter().any(|(id, _)| *id == hank)
                && fake
                    .state()
                    .user(hank)
                    .is_some_and(|user| user.families.iter().all(|(f, _)| *f != own))
        })
        .await;
    h.check(
        "remove_member",
        question.as_deref() == Some("Remove Hank from the family?") && removed,
        format!("asked {question:?}"),
    );

    // ---- P2 in P4's dialog: Gina's picture in her row; "Family Stickers" → Manage… opens the pack ----
    let gina_row = open
        .member_rows
        .borrow()
        .iter()
        .find(|(id, _)| *id == gina)
        .map(|(_, row)| row.clone());
    let pictured = h
        .wait_until(WAIT, || {
            gina_row.as_ref().is_some_and(|row| {
                descendants(row)
                    .into_iter()
                    .filter_map(|widget| widget.downcast::<adw::Avatar>().ok())
                    .any(|avatar| avatar.custom_image().is_some())
            })
        })
        .await;
    h.check(
        "member_row_shows_the_picture",
        pictured,
        "Gina's face (avatar version 2) over her initials",
    );
    let manage = h
        .wait_until(WAIT, || {
            descendants(&open.dialog)
                .iter()
                .any(|row| row.has_css_class("fc-family-stickers"))
        })
        .await;
    if let Some(row) = descendants(&open.dialog)
        .into_iter()
        .filter_map(|widget| widget.downcast::<adw::ButtonRow>().ok())
        .find(|row| row.has_css_class("fc-family-stickers"))
    {
        row.emit_by_name::<()>("activated", &[]);
    }
    let pack = h
        .wait_until(WAIT, || {
            main.window()
                .visible_dialog()
                .is_some_and(|dialog| dialog.title() == shell.say.get("Family Stickers"))
        })
        .await;
    h.sleep(Duration::from_millis(500)).await;
    h.screenshot("family-stickers").await;
    if let Some(dialog) = main.window().visible_dialog()
        && dialog.title() != open.dialog.title()
    {
        dialog.close();
    }
    h.check(
        "family_stickers_opens_the_pack",
        manage && pack,
        format!("Manage… drawn {manage}; the pack dialog came up {pack}"),
    );
    let _ = h
        .wait_until(WAIT, || {
            main.window()
                .visible_dialog()
                .is_some_and(|dialog| dialog.title() == open.dialog.title())
        })
        .await;

    // ---- Statistics… ---------------------------------------------------------------------------------------
    crate::settings::dialogs::statistics(&open.dialog);
    let counted = h
        .wait_until(WAIT, || {
            main.window().visible_dialog().is_some_and(|dialog| {
                find::<adw::PreferencesGroup>(&dialog, |group| group.title() == "Who sends what")
                    .is_some()
            })
        })
        .await;
    h.screenshot("statistics").await;
    if let Some(dialog) = main.window().visible_dialog()
        && dialog.title() != open.dialog.title()
    {
        dialog.close();
    }
    h.check(
        "statistics",
        counted,
        "totals, attachments and who sends what",
    );

    // ---- dark, narrow ------------------------------------------------------------------------------------
    let styles = adw::StyleManager::default();
    styles.set_color_scheme(adw::ColorScheme::ForceDark);
    h.sleep(Duration::from_millis(300)).await;
    h.screenshot("dialog-dark").await;
    styles.set_color_scheme(adw::ColorScheme::Default);
    if h.input() {
        h.resize(&window, 440, 600).await;
        h.screenshot("dialog-narrow").await;
        h.resize(&window, 1000, 680).await;
    }
    open.dialog.close();

    // ---- New Group, then its info --------------------------------------------------------------------------
    gio::prelude::ActionGroupExt::activate_action(
        main.window(),
        crate::seams::actions::NEW_GROUP.name,
        None,
    );
    h.sleep(Duration::from_millis(400)).await;
    let new_group = main.window().visible_dialog();
    let named = new_group
        .as_ref()
        .and_then(|dialog| find::<adw::EntryRow>(dialog, |_| true));
    if let Some(entry) = &named {
        entry.set_text("Weekend plans");
    }
    if let Some(dialog) = &new_group {
        for check in descendants(dialog)
            .into_iter()
            .filter_map(|w| w.downcast::<gtk::CheckButton>().ok())
            .take(1)
        {
            check.set_active(true);
        }
    }
    h.screenshot("new-group").await;
    if let Some(create) = new_group
        .as_ref()
        .and_then(|dialog| button(dialog, "Create"))
    {
        create.emit_clicked();
    }
    let created = h
        .wait_until(WAIT, || {
            world()
                .borrow()
                .groups
                .iter()
                .any(|chat| chat.title == "Weekend plans")
        })
        .await;
    let group_id = world()
        .borrow()
        .groups
        .iter()
        .find(|chat| chat.title == "Weekend plans")
        .map(|chat| chat.id);
    let cached = group_id.is_some_and(|id| connection.chats.chat(id).ok().flatten().is_some());
    let info = group_id.and_then(|id| family::groups::group_info(main.window(), id));
    h.sleep(Duration::from_millis(300)).await;
    let admin_drawn = info.as_ref().is_some_and(|dialog| {
        find::<gtk::Label>(dialog, |label| label.text() == "Admin").is_some()
            && find::<adw::ActionRow>(dialog, |row| row.title() == "You").is_some()
    });
    h.screenshot("group-info").await;
    if let Some(info) = info {
        info.close();
    }
    h.check(
        "create_group",
        named.is_some() && created && cached && admin_drawn,
        format!("created {created}, in the cache {cached}, info drawn {admin_drawn}"),
    );

    // ---- the dialog follows a switch to another family ---------------------------------------------------------
    family::open();
    let _ = connection.session.switch_family(other).await;
    let switched = h
        .wait_until(WAIT, || {
            family::current().is_some_and(|open| {
                open.title.text() == "Ivy's Family"
                    && open.member_rows.borrow().iter().any(|(id, _)| *id == jack)
                    && button(&open.dialog, "Rotate").is_none()
            })
        })
        .await;
    h.screenshot("switched").await;
    h.check(
        "switch_family",
        switched,
        "the dialog redraws for Ivy's Family, a member's view (no console)",
    );
    if let Some(open) = family::current() {
        open.dialog.close();
    }

    // ---- leave that family ---------------------------------------------------------------------------------------
    let toasts = adw::ToastOverlay::new();
    crate::settings::dialogs::leave_family(main.window(), &toasts);
    let asked = h
        .wait_until(Duration::from_secs(5), || {
            main.window()
                .visible_dialog()
                .and_downcast::<adw::AlertDialog>()
                .is_some()
        })
        .await;
    let alert = main
        .window()
        .visible_dialog()
        .and_downcast::<adw::AlertDialog>();
    let heading = alert
        .as_ref()
        .and_then(|a| a.heading())
        .map(|t| t.to_string());
    h.screenshot("leave").await;
    if let Some(alert) = &alert {
        // What a click on its button does: the response, and the alert goes.
        alert.emit_by_name::<()>("response", &[&crate::settings::ui::CONFIRM]);
        alert.close();
    }
    let left = h
        .wait_until(WAIT, || {
            let state = connection.session.state();
            state.all_families().iter().all(|f| f.id != other)
                && state.current_family_id == Some(own)
        })
        .await;
    h.check(
        "leave_family",
        asked && heading.as_deref() == Some("Leave “Ivy's Family”?") && left,
        format!(
            "asked {heading:?}, families {:?}",
            connection
                .session
                .state()
                .all_families()
                .iter()
                .map(|f| f.id)
                .collect::<Vec<_>>()
        ),
    );
    h.check(
        "still_in_the_chats",
        main.page() == GatePage::Chats,
        format!("{:?}", main.page()),
    );
    let unhandled = fake.unhandled();
    h.check(
        "fake_server_knew_every_request",
        unhandled.is_empty(),
        format!("{unhandled:?}"),
    );
    // The pack was this group's: the groups after it meet a server that keeps none.
    fake.state().pack_limits = None;
}
