//! The message menu's Safety items and Save… (shell-design §5.2): Report… — the four reasons, the
//! disclosure that the family owner reads it, and the operator's contact where the server
//! published one (Windows' `Dialogs.ReportAsync`); "Report this reply…" — the same reasons, an
//! optional note, and the operator as the reader (docs/protocol.md, "Reporting the assistant");
//! Block / Unblock straight away, as the Mac's menu does (`FamilyModel::block`); and Save… for a
//! message's first attachment through `gtk::FileDialog` (portal-friendly).

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use fc_core::protocol::MessageDto;
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::family::FamilyModel;
use fc_logic::family_console::family_text;

use crate::chats::banners;
use crate::services::log;
use crate::shell::{Connection, Shell};
use crate::text::typing_hooks;

/// Report… (a member's message) or "Report this reply…" (the assistant's).
pub fn report(
    anchor: &impl IsA<gtk::Widget>,
    connection: &Rc<Connection>,
    message: &MessageDto,
    assistant: bool,
) {
    let shell = Shell::get();
    let say = &shell.say;
    let name = match connection.chats.member(message.sender_id).ok().flatten() {
        Some(member) if member.deleted => say.get("Deleted account").to_string(),
        Some(member) if !member.display_name.is_empty() => member.display_name,
        _ => say.get("Someone").to_string(),
    };
    let (heading, question) = if assistant {
        (
            say.get("Report this reply").to_string(),
            say.get("What was wrong with this reply?").to_string(),
        )
    } else {
        (
            say.get("Report").to_string(),
            say.format("Why are you reporting %@?", &[&name]),
        )
    };
    let dialog = adw::AlertDialog::new(Some(&heading), Some(&question));
    let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let chosen = Rc::new(RefCell::new(family_text::FIRST_REASON.to_string()));
    let mut first: Option<gtk::CheckButton> = None;
    for (code, key) in family_text::REASONS {
        let check = gtk::CheckButton::with_label(say.get(key));
        if let Some(first) = &first {
            check.set_group(Some(first));
        }
        if code == family_text::FIRST_REASON {
            check.set_active(true);
        }
        let chosen = Rc::clone(&chosen);
        check.connect_toggled(move |check| {
            if check.is_active() {
                *chosen.borrow_mut() = code.to_string();
            }
        });
        column.append(&check);
        if first.is_none() {
            first = Some(check);
        }
    }
    let note = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .height_request(72)
        .accepts_tab(false)
        .css_classes(["card"])
        .top_margin(6)
        .bottom_margin(6)
        .left_margin(6)
        .right_margin(6)
        .build();
    let hooks = if assistant {
        note.update_property(&[gtk::accessible::Property::Label(
            say.get("Say something about it (optional)"),
        )]);
        column.append(&note);
        Some(typing_hooks::attach_text_view(&note, true))
    } else {
        None
    };
    let disclosure = if assistant {
        family_text::assistant_report_disclosure(say.as_ref())
    } else {
        family_text::report_disclosure(true, say.as_ref())
    };
    column.append(
        &gtk::Label::builder()
            .label(&disclosure)
            .css_classes(["dim-label", "caption"])
            .wrap(true)
            .xalign(0.0)
            .build(),
    );
    if let Some(contact) = connection
        .session
        .state()
        .support_contact
        .filter(|contact| !contact.is_empty())
    {
        if !assistant {
            column.append(
                &gtk::Label::builder()
                    .label(say.get("If the problem is the owner"))
                    .css_classes(["heading"])
                    .xalign(0.0)
                    .build(),
            );
        }
        // Verbatim, and never made a link.
        column.append(
            &gtk::Label::builder()
                .label(&contact)
                .wrap(true)
                .selectable(true)
                .xalign(0.0)
                .build(),
        );
        column.append(
            &gtk::Label::builder()
                .label(say.get("This server's operator published this contact."))
                .css_classes(["dim-label", "caption"])
                .wrap(true)
                .xalign(0.0)
                .build(),
        );
    }
    dialog.set_extra_child(Some(&column));
    dialog.add_response("cancel", say.get("Cancel"));
    dialog.add_response("report", say.get("Report"));
    dialog.set_response_appearance("report", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    let (connection, message, anchor_widget) = (
        Rc::clone(connection),
        message.clone(),
        anchor.clone().upcast::<gtk::Widget>(),
    );
    dialog.connect_response(None, move |_, response| {
        let _ = &hooks;
        if response != "report" {
            return;
        }
        let reason = chosen.borrow().clone();
        let text = typing_hooks::buffer_text(&note.buffer());
        let (connection, message, anchor) = (
            Rc::clone(&connection),
            message.clone(),
            anchor_widget.clone(),
        );
        glib::spawn_future_local(async move {
            let ok = if assistant {
                let note = Some(text.trim()).filter(|t| !t.is_empty());
                connection
                    .api
                    .report_assistant(message.id, &reason, note)
                    .await
                    .into_result()
                    .is_ok()
            } else {
                connection
                    .api
                    .report(message.sender_id, &reason, Some(message.id))
                    .await
                    .into_result()
                    .is_ok()
            };
            let shell = Shell::get();
            banners::alert(
                &anchor,
                if ok {
                    shell.say.get("Report sent.")
                } else {
                    shell.say.get("Couldn't send the report. Try again.")
                },
            );
        });
    });
    dialog.present(Some(anchor));
}

/// Block — or Unblock — a message's sender, applied here once the server has it.
pub fn block(anchor: &impl IsA<gtk::Widget>, connection: &Rc<Connection>, user_id: i64) {
    let blocked = connection.chats.is_blocked(user_id).unwrap_or(false);
    let model = FamilyModel::new(Rc::clone(&connection.api), Rc::clone(&connection.chats));
    let anchor = anchor.clone().upcast::<gtk::Widget>();
    glib::spawn_future_local(async move {
        match model.block(user_id, !blocked).await {
            Ok(None) => crate::fc_info!("block: member {user_id} → {}", !blocked),
            Ok(Some(error)) => {
                crate::fc_info!("block: refused ({})", error.code);
                let shell = Shell::get();
                banners::alert(
                    &anchor,
                    shell.say.get("Couldn't change that right now. Try again."),
                );
            }
            Err(error) => {
                crate::fc_warn!("block: the cache refused ({})", log::store_error(&error))
            }
        }
    });
}

/// Save… a message's first attachment (`win.message-save`): P2's `attachments::saving::save` — the
/// ORIGINAL through the attachment cache, written where the person chooses with its own safe name
/// suggested; a failure is said (`Outcome::Failed`'s sentence), never silent.
pub fn save_attachment(anchor: &impl IsA<gtk::Widget>, message: &MessageDto) {
    let Some(attachment) = message.media().first().cloned() else {
        return;
    };
    let window = anchor
        .root()
        .and_then(|root| root.downcast::<gtk::Window>().ok());
    let anchor = anchor.clone().upcast::<gtk::Widget>();
    glib::spawn_future_local(async move {
        if let crate::attachments::saving::Outcome::Failed(sentence) =
            crate::attachments::saving::save(window, &attachment).await
        {
            crate::attachments::saving::alert(Some(&anchor), &sentence);
        }
    });
}
