//! The assistant's consent question (`assistant_consent::disclosure`; the Mac's
//! `AssistantConsentSheet`): what goes to whom, then Not Now, or I Agree — Agree With / Without
//! Lookups where the server offers them. Asked before a message goes to the assistant (the
//! composer), before a recording's text is asked for (`media::transcript`) and from Settings'
//! Assistant group, which also asks about lookups alone ([`ask_lookups`]); the answer is recorded
//! on the server (`lookups::record`) and `/me` read again, so a yes is a yes only once the session
//! holds it.

use std::rc::Rc;

use adw::prelude::*;
use fc_core::rt::LocalBoxFuture;
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::{assistant_consent, lookups};

use crate::services::log;
use crate::shell::{Connection, Shell};

/// How the question ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// A yes the session now holds.
    Agreed,
    /// Not Now, the dialog closed, or a server that names nobody to agree to.
    NotNow,
    /// A yes the server would not keep: "Couldn't save your answer. Try again." is the caller's.
    NotSaved,
}

/// Asks the whole question over `parent`'s window — the assistant, and lookups beside it where the
/// server offers them — answering once the dialog is closed and the answer recorded.
pub fn ask(
    parent: &impl IsA<gtk::Widget>,
    connection: &Rc<Connection>,
) -> LocalBoxFuture<'static, Answer> {
    let shell = Shell::get();
    let say = &shell.say;
    let state = connection.session.state();
    let Some(processor) = state
        .assistant
        .as_ref()
        .and_then(|assistant| assistant.processor.clone())
    else {
        return Box::pin(std::future::ready(Answer::NotNow));
    };
    let offered = lookups::offered(state.assistant.as_ref());
    let providers = if offered {
        lookups::providers(state.assistant.as_ref()).unwrap_or_default()
    } else {
        Vec::new()
    };
    let lines = assistant_consent::disclosure(
        &processor,
        state.family.as_ref().is_some_and(|f| f.ai_history),
        state.family.as_ref().is_some_and(|f| f.ai_vision),
        say.as_ref(),
        state.assistant.as_ref().is_some_and(|a| a.transcribe),
        &providers,
    );
    let dialog = question(say.get("Before the assistant answers"), &lines);
    if offered {
        dialog.add_response("without", say.get("Agree Without Lookups"));
        dialog.add_response("agree", say.get("Agree With Lookups"));
    } else {
        dialog.add_response("agree", say.get("I Agree"));
    }
    answered(dialog, parent, connection, Mode::Assistant { offered })
}

/// Asks only about lookups (Settings' "Review and Allow Lookups…", the Mac's `.lookupsOnly`): for
/// a member who agreed to the assistant already, the line naming the providers and the way back
/// out — the rest of the disclosure they have read and agreed to.
pub fn ask_lookups(
    parent: &impl IsA<gtk::Widget>,
    connection: &Rc<Connection>,
) -> LocalBoxFuture<'static, Answer> {
    let shell = Shell::get();
    let say = &shell.say;
    let state = connection.session.state();
    let Some(names) =
        lookups::providers(state.assistant.as_ref()).filter(|_| lookups::in_settings(&state))
    else {
        return Box::pin(std::future::ready(Answer::NotNow));
    };
    let lines = lookups::disclosure(
        &names,
        state.family.as_ref().is_some_and(|f| f.ai_history),
        say.as_ref(),
    );
    let dialog = question(say.get("Looking things up"), &lines);
    dialog.add_response("agree", say.get("I Agree"));
    answered(dialog, parent, connection, Mode::LookupsOnly)
}

/// Which question was asked.
#[derive(Debug, Clone, Copy)]
enum Mode {
    /// The assistant's; `offered`: with lookups beside it.
    Assistant { offered: bool },
    /// Lookups alone, on top of a yes already held.
    LookupsOnly,
}

/// The dialog's frame: "The Assistant", `heading`, the lines, the policy, and Not Now.
fn question(heading: &str, lines: &[String]) -> adw::AlertDialog {
    let say = Shell::get().say.clone();
    let dialog = adw::AlertDialog::new(Some(say.get("The Assistant")), Some(heading));
    let column = gtk::Box::new(gtk::Orientation::Vertical, 8);
    for line in lines {
        column.append(
            &gtk::Label::builder()
                .label(line)
                .wrap(true)
                .xalign(0.0)
                .build(),
        );
    }
    // The policy too: the disclosure where the answer is given, and a policy that holds the same
    // promises (the Mac's sheet).
    let policy = gtk::LinkButton::with_label(PRIVACY_URL, say.get("Privacy Policy"));
    policy.set_halign(gtk::Align::Start);
    column.append(&policy);
    dialog.set_extra_child(Some(&column));
    dialog.add_response("not-now", say.get("Not Now"));
    dialog.set_default_response(Some("not-now"));
    dialog.set_close_response("not-now");
    dialog
}

/// Presents `dialog` and records what was pressed; the future answers once the session holds it.
fn answered(
    dialog: adw::AlertDialog,
    parent: &impl IsA<gtk::Widget>,
    connection: &Rc<Connection>,
    mode: Mode,
) -> LocalBoxFuture<'static, Answer> {
    dialog.set_response_appearance("agree", adw::ResponseAppearance::Suggested);
    let (sender, receiver) = futures_channel::oneshot::channel();
    let sender = std::cell::RefCell::new(Some(sender));
    let connection = Rc::clone(connection);
    dialog.connect_response(None, move |_, response| {
        let Some(sender) = sender.borrow_mut().take() else {
            return;
        };
        let (answer, assistant_agreed) = match mode {
            Mode::Assistant { offered } => (
                lookups::answer_for(response == "agree", response == "without", offered),
                false,
            ),
            Mode::LookupsOnly if response == "agree" => {
                (lookups::ConsentAnswer::AgreeWithLookups, true)
            }
            Mode::LookupsOnly => (lookups::ConsentAnswer::NotNow, true),
        };
        if answer == lookups::ConsentAnswer::NotNow {
            let _ = sender.send(Answer::NotNow);
            return;
        }
        let connection = Rc::clone(&connection);
        glib::spawn_future_local(async move {
            let refused = lookups::record(&connection.api, answer, assistant_agreed).await;
            if let Err(error) = connection.session.refresh().await {
                crate::fc_warn!("consent: the cache refused ({})", log::store_error(&error));
            }
            let state = connection.session.state();
            let held = match mode {
                Mode::Assistant { .. } => state
                    .assistant_consent_at
                    .as_deref()
                    .is_some_and(|at| !at.trim().is_empty()),
                Mode::LookupsOnly => lookups::allowed(&state),
            };
            let answer = if let Some(error) = refused {
                crate::fc_info!("consent: refused ({})", error.code);
                Answer::NotSaved
            } else if held {
                Answer::Agreed
            } else {
                Answer::NotNow
            };
            let _ = sender.send(answer);
        });
    });
    dialog.present(Some(parent));
    Box::pin(async move { receiver.await.unwrap_or(Answer::NotNow) })
}

/// The Privacy Policy — the Mac's and Windows' own page.
const PRIVACY_URL: &str = "https://nettrash.me/appstore/familyconnect/privacy.html";
