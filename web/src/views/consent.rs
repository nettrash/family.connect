//! Asking, once, before anything a member writes goes to the model
//! (docs/protocol.md, "Consenting to the assistant").
//!
//! The apps' screen, in the browser (ios Views/AssistantConsentSheet.swift):
//! everything the person needs is HERE and not only behind the policy link
//! — who receives the words, what travels with them, where the answer
//! lands, and that stopping later cannot recall what has already gone.
//!
//! It saves nothing itself. The buttons hand the answer back, and the app
//! writes it through `Action::SetAssistantConsent` — or, for "Agree With
//! Lookups", `Action::AgreeWithLookups` — which hold the server's own
//! timestamps.
//!
//! Where the server can look things up (`assistant.lookups`, #72) the same
//! screen names the providers in one more line and offers lookups BESIDE
//! the assistant, never folded into it: agreeing to the assistant alone
//! stays one press, and nobody's lookups are assumed. A member who agreed
//! to the assistant before is asked about lookups alone, by
//! [`LookupConsentDialog`], from Settings.

use fc_text::i18n::{t, t1};
use fc_text::{assistant_consent, lookups};
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
    /// Whether this server can turn a recording into text
    /// (`assistant.transcribe`) — and so whether a recording's sound may go
    /// to the processor when this member asks for its text
    /// (docs/protocol.md, "Transcripts on request").
    #[prop_or_default]
    pub transcribe: bool,
    /// The providers the assistant may look things up in
    /// (`assistant.lookups`); empty where the server has none, and then
    /// this screen is exactly what it was before lookups.
    #[prop_or_default]
    pub lookups: Vec<String>,
    /// The answer: `true` is "Agree With Lookups", `false` agreeing to the
    /// assistant alone ("I Agree" where there are no lookups to offer).
    pub on_agree: Callback<bool>,
    pub on_cancel: Callback<()>,
}

#[function_component(AssistantConsentDialog)]
pub fn assistant_consent_dialog(props: &ConsentProps) -> Html {
    let agree = |with_lookups: bool| {
        let on_agree = props.on_agree.clone();
        Callback::from(move |_: MouseEvent| on_agree.emit(with_lookups))
    };
    let cancel = {
        let on_cancel = props.on_cancel.clone();
        Callback::from(move |_: MouseEvent| on_cancel.emit(()))
    };
    let lines = assistant_consent::disclosure(
        &props.processor,
        props.family_history,
        props.family_vision,
        props.transcribe,
        &props.lookups,
    );
    let choice = lookups::ask(false, false, &props.lookups) == lookups::Ask::AssistantOrBoth;
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
            // Three answers, two of them long: they wrap rather than run off
            // a phone's edge.
            <div class={classes!("dialog-actions", choice.then_some("consent-choice"))}>
                <button class="secondary" onclick={cancel}>{ t("Not Now") }</button>
                if choice {
                    <button class="secondary agree-without-lookups" onclick={agree(false)}>
                        { t("Agree Without Lookups") }
                    </button>
                    <button class="primary agree-with-lookups" onclick={agree(true)}>
                        { t("Agree With Lookups") }
                    </button>
                } else {
                    <button class="primary" onclick={agree(false)}>{ t("I Agree") }</button>
                }
            </div>
        </Modal>
    }
}

#[derive(Properties, PartialEq)]
pub struct LookupConsentProps {
    /// `assistant.lookups` — never opened with none.
    pub lookups: Vec<String>,
    /// Whether a mention takes the chat's recent history with it, which
    /// decides whether the line may say a query can come from it.
    pub family_history: bool,
    pub on_agree: Callback<()>,
    pub on_cancel: Callback<()>,
}

/// The lookup question alone, for a member who already agreed to the
/// assistant (reached from Settings): who a query may go to, that stopping
/// cannot recall what went, and the policy — then "I Agree" / "Not Now".
#[function_component(LookupConsentDialog)]
pub fn lookup_consent_dialog(props: &LookupConsentProps) -> Html {
    let agree = {
        let on_agree = props.on_agree.clone();
        Callback::from(move |_: MouseEvent| on_agree.emit(()))
    };
    let cancel = {
        let on_cancel = props.on_cancel.clone();
        Callback::from(move |_: MouseEvent| on_cancel.emit(()))
    };
    let named = lookups::names(&lookups::providers(&props.lookups));
    html! {
        <Modal title={t("The Assistant")} on_cancel={props.on_cancel.clone()}>
            <h3>{ lookups::heading() }</h3>
            <ul class="disclosure">
                <li>{ lookups::disclosure_line(&named, props.family_history) }</li>
                <li>{ t("You can stop this at any time in Settings. What has already been sent cannot be taken back.") }</li>
            </ul>
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
