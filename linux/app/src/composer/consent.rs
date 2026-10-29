//! The assistant's consent question (`assistant_consent::disclosure`; the Mac's
//! `AssistantConsentSheet`): what goes to whom, then Not Now, or I Agree — Agree With / Without
//! Lookups where the server offers them. Asked before a message goes to the assistant (the
//! composer) and before a recording's text is asked for (`media::transcript`); the answer is
//! recorded on the server (`lookups::record`) and `/me` read again, so a yes is a yes only once the
//! session holds it.

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

/// Asks over `parent`'s window, answering once the dialog is closed and the answer recorded.
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
    let dialog = adw::AlertDialog::new(
        Some(say.get("The Assistant")),
        Some(say.get("Before the assistant answers")),
    );
    let column = gtk::Box::new(gtk::Orientation::Vertical, 8);
    for line in lines {
        column.append(
            &gtk::Label::builder()
                .label(&line)
                .wrap(true)
                .xalign(0.0)
                .build(),
        );
    }
    dialog.set_extra_child(Some(&column));
    dialog.add_response("not-now", say.get("Not Now"));
    if offered {
        dialog.add_response("without", say.get("Agree Without Lookups"));
        dialog.add_response("agree", say.get("Agree With Lookups"));
    } else {
        dialog.add_response("agree", say.get("I Agree"));
    }
    dialog.set_response_appearance("agree", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("not-now"));
    dialog.set_close_response("not-now");
    let (sender, receiver) = futures_channel::oneshot::channel();
    let sender = std::cell::RefCell::new(Some(sender));
    let connection = Rc::clone(connection);
    dialog.connect_response(None, move |_, response| {
        let Some(sender) = sender.borrow_mut().take() else {
            return;
        };
        let answer = lookups::answer_for(response == "agree", response == "without", offered);
        if answer == lookups::ConsentAnswer::NotNow {
            let _ = sender.send(Answer::NotNow);
            return;
        }
        let connection = Rc::clone(&connection);
        glib::spawn_future_local(async move {
            let refused = lookups::record(&connection.api, answer, false).await;
            if let Err(error) = connection.session.refresh().await {
                crate::fc_warn!("consent: the cache refused ({})", log::store_error(&error));
            }
            let answer = if let Some(error) = refused {
                crate::fc_info!("consent: refused ({})", error.code);
                Answer::NotSaved
            } else if connection
                .session
                .state()
                .assistant_consent_at
                .is_some_and(|at| !at.trim().is_empty())
            {
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
