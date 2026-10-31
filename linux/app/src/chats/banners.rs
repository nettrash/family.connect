//! The lines around the chat list (shell-design §3.1): above it the UNPAID banner — "“Ivanovs” will
//! be deleted on 14 November because its subscription has ended…" while the shown family has
//! `delete_at` (docs/protocol.md, "Paid families"; `paid_families::banner`) — and the INVITATION
//! banner — "Anna invited you to “The Smiths”" with Join and Decline (the Mac's
//! `InvitationBanner`); under it the CONNECTION line — "Connecting…" while the socket is on its
//! way, "Offline" while the network is gone (the Mac's `ConnectionBanner`), nothing while all is
//! well.

use std::rc::Rc;

use adw::prelude::*;
use fc_core::protocol::{ApiError, MemberDto, error_codes};
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::app_session::SessionState;
use fc_logic::families::{family_sentences, invitation_text};
use fc_logic::live_connection::Link;
use fc_logic::paid_families::{self, Days};

use crate::services::log;
use crate::shell::Shell;

/// The family-level notices above the chat list: the unpaid banner of the family shown, when it
/// has a deadline, then the invitations.
pub fn above_list(shell: &Rc<Shell>, state: &SessionState, members: &[MemberDto]) -> gtk::Widget {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
    if let Some(banner) = unpaid(shell, state, members) {
        column.append(&banner);
    }
    column.append(&invitations(shell, state));
    column.upcast()
}

/// The unpaid banner (docs/protocol.md, "Paid families"): the day the family shown will be deleted
/// and who can renew it — or where, for its owner. `None` while it is paid for, free, or on a server
/// that charges nothing. It has no button: renewing happens in the Apple app.
pub fn unpaid(
    shell: &Rc<Shell>,
    state: &SessionState,
    members: &[MemberDto],
) -> Option<gtk::Widget> {
    let days = Days::new(
        &shell.culture,
        shell.zone.as_ref(),
        time::OffsetDateTime::now_utc(),
    );
    let text = paid_families::banner(state, members, shell.say.as_ref(), &days)?;
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    line.add_css_class("fc-chat-banner");
    line.add_css_class("fc-unpaid-banner");
    line.set_margin_start(8);
    line.set_margin_end(8);
    line.set_margin_top(4);
    let mark = gtk::Image::builder()
        .icon_name("dialog-warning-symbolic")
        .valign(gtk::Align::Start)
        .css_classes(["warning"])
        .accessible_role(gtk::AccessibleRole::Presentation)
        .build();
    line.append(&mark);
    let words = gtk::Label::builder()
        .label(&text)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .xalign(0.0)
        .hexpand(true)
        .build();
    line.append(&words);
    Some(line.upcast())
}

/// The invitations as rows (none drawn when there are none).
pub fn invitations(shell: &Rc<Shell>, state: &SessionState) -> gtk::Widget {
    let say = &shell.say;
    let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
    for invitation in state.all_invitations() {
        let line = gtk::Box::new(gtk::Orientation::Vertical, 6);
        line.add_css_class("fc-chat-banner");
        line.set_margin_start(8);
        line.set_margin_end(8);
        line.set_margin_top(4);
        let words = gtk::Label::builder()
            .label(invitation_text::banner(invitation, say.as_ref()))
            .wrap(true)
            .xalign(0.0)
            .build();
        line.append(&words);
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        buttons.set_halign(gtk::Align::End);
        let decline = gtk::Button::builder()
            .label(say.get("Decline"))
            .css_classes(["flat"])
            .build();
        let join = gtk::Button::builder()
            .label(say.get("Join"))
            .css_classes(["suggested-action"])
            .build();
        buttons.append(&decline);
        buttons.append(&join);
        line.append(&buttons);
        let id = invitation.id;
        join.connect_clicked(move |button| answer(button, id, true));
        decline.connect_clicked(move |button| answer(button, id, false));
        column.append(&line);
    }
    column.upcast()
}

/// Join or Decline one invitation; a refusal is said in an alert (the Mac's).
pub fn answer(anchor: &impl IsA<gtk::Widget>, invitation_id: i64, accept: bool) {
    let shell = Shell::get();
    let Some(connection) = shell.connection() else {
        return;
    };
    let anchor = anchor.clone().upcast::<gtk::Widget>();
    anchor.set_sensitive(false);
    glib::spawn_future_local(async move {
        let refused = if accept {
            match connection.session.accept_invitation(invitation_id).await {
                Ok(refused) => refused,
                Err(error) => {
                    crate::fc_warn!(
                        "invitation: the cache refused ({})",
                        log::store_error(&error)
                    );
                    None
                }
            }
        } else {
            connection.session.decline_invitation(invitation_id).await
        };
        anchor.set_sensitive(true);
        if let Some(error) = refused {
            crate::fc_info!("invitation: refused ({})", error.code);
            let state = connection.session.state();
            let shell = Shell::get();
            alert(
                &anchor,
                &refusal(&error, state.max_families_per_account, shell.say.as_ref()),
            );
        }
    });
}

/// What a refused Join or Decline says (the Mac's `FamilySwitcher`).
fn refusal(error: &ApiError, cap: i32, say: &dyn StringCatalog) -> String {
    if let Some(sentence) = family_sentences::for_error(error, cap, say) {
        return sentence;
    }
    if error.code == error_codes::TRANSPORT {
        say.get("Can't reach the server. Try again.").to_string()
    } else {
        say.get("The server refused it.").to_string()
    }
}

/// A one-button alert over `anchor`'s window — `present()` and the `response` signal, never
/// `choose_future` (md §7.8).
pub fn alert(anchor: &impl IsA<gtk::Widget>, text: &str) {
    let shell = Shell::get();
    let dialog = adw::AlertDialog::new(Some(text), None);
    dialog.add_response("ok", shell.say.get("OK"));
    dialog.set_default_response(Some("ok"));
    dialog.present(Some(anchor));
}

/// The connection line's words for the link and the network, or `None` when nothing is wrong.
pub fn connection_text(link: Link, network: bool, say: &dyn StringCatalog) -> Option<String> {
    if !network {
        return Some(say.get("Offline").to_string());
    }
    match link {
        Link::Connecting => Some(say.get("Connecting…").to_string()),
        Link::Up | Link::Down => None,
    }
}
