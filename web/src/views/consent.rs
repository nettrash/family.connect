//! Asking, once, before anything a member writes goes to the model
//! (docs/protocol.md, "Consenting to the assistant").
//!
//! The apps' screen, in the browser (ios Views/AssistantConsentSheet.swift):
//! everything the person needs is HERE and not only behind the policy link
//! — who receives the words, what travels with them, where the answer
//! lands, and that stopping later cannot recall what has already gone.
//!
//! It saves nothing itself. The buttons hand the answer back, and the app
//! writes it through `Action::SetAssistantConsent`, which holds the
//! server's own timestamp.

use fc_text::assistant_consent;
use fc_text::i18n::{t, t1};
use yew::prelude::*;

use crate::views::dialog::Modal;

#[derive(Properties, PartialEq)]
pub struct ConsentProps {
    /// Who answers, verbatim as the operator wrote it. The dialog is never
    /// opened without one (`assistant_consent::is_available`).
    pub processor: String,
    /// Whether an `@ai` in the family chat takes that chat's recent
    /// history with it, which changes what this screen PROMISES rather
    /// than merely how much it says.
    pub family_history: bool,
    /// Whether a photograph may be shown to the model at all.
    pub family_vision: bool,
    pub on_agree: Callback<()>,
    pub on_cancel: Callback<()>,
}

#[function_component(AssistantConsentDialog)]
pub fn assistant_consent_dialog(props: &ConsentProps) -> Html {
    let agree = {
        let on_agree = props.on_agree.clone();
        Callback::from(move |_: MouseEvent| on_agree.emit(()))
    };
    let cancel = {
        let on_cancel = props.on_cancel.clone();
        Callback::from(move |_: MouseEvent| on_cancel.emit(()))
    };
    let lines =
        assistant_consent::disclosure(&props.processor, props.family_history, props.family_vision);
    html! {
        <Modal title={t("The Assistant")} on_cancel={props.on_cancel.clone()}>
            <h3>{ t("Before the assistant answers") }</h3>
            <ul class="disclosure">
                { for lines.into_iter().map(|line| html! { <li>{ line }</li> }) }
            </ul>
            // The policy is still linked, because the guideline asks for
            // both: the disclosure where the answer is given, and a policy
            // that holds the same promises.
            <p class="footnote">
                <a href="https://nettrash.me/appstore/familyconnect/privacy.html"
                   target="_blank" rel="noopener noreferrer">{ t("Privacy Policy") }</a>
            </p>
            <div class="dialog-actions">
                <button class="secondary" onclick={cancel}>{ t("Not Now") }</button>
                <button class="primary" onclick={agree}>{ t("I Agree") }</button>
            </div>
        </Modal>
    }
}

/// The one line a composer shows above the box while the question is
/// unanswered — and the door on it.
#[derive(Properties, PartialEq)]
pub struct ConsentBarProps {
    pub processor: String,
    pub on_review: Callback<()>,
}

#[function_component(AssistantConsentBar)]
pub fn assistant_consent_bar(props: &ConsentBarProps) -> Html {
    let review = {
        let on_review = props.on_review.clone();
        Callback::from(move |_: MouseEvent| on_review.emit(()))
    };
    html! {
        <p class="consent-notice" role="note">
            <span aria-hidden="true">{ "✋ " }</span>
            { t1("This goes to %@. You haven't agreed to that yet.", &props.processor) }
            { " " }
            <button class="link" onclick={review}>{ t("Review…") }</button>
        </p>
    }
}
