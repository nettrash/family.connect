//! Reporting a member, or one message of theirs, to the family owner
//! (docs/protocol.md, "Reporting a member") — the same four reasons, the
//! same disclosure and the same escalation line the apps show
//! (ios Views/ReportSheet.swift).

use yew::prelude::*;

/// Who is being reported, and for which message if any.
#[derive(Debug, Clone, PartialEq)]
pub struct ReportTarget {
    pub user_id: i64,
    pub name: String,
    pub message_id: Option<i64>,
}

/// The protocol's fixed four, in the apps' order, with harassment chosen
/// to start with — the reason this feature most exists for.
pub const REASONS: [(&str, &str); 4] = [
    ("spam", "Spam"),
    ("harassment", "Harassment"),
    ("inappropriate", "Inappropriate"),
    ("other", "Something else"),
];

#[derive(Properties, PartialEq)]
pub struct ReportProps {
    pub target: ReportTarget,
    /// The operator's published contact, shown VERBATIM and never made a
    /// link: an address, a URL or a whole sentence all read as sent.
    pub support_contact: Option<String>,
    /// (reason code) when the person confirms.
    pub on_submit: Callback<String>,
    pub on_cancel: Callback<()>,
}

#[function_component(ReportDialog)]
pub fn report_dialog(props: &ReportProps) -> Html {
    let reason = use_state(|| "harassment".to_string());
    let choose = |code: &'static str| {
        let reason = reason.clone();
        Callback::from(move |_: Event| reason.set(code.to_string()))
    };
    let submit = {
        let reason = reason.clone();
        let on_submit = props.on_submit.clone();
        Callback::from(move |_: MouseEvent| on_submit.emit((*reason).clone()))
    };
    let cancel = {
        let on_cancel = props.on_cancel.clone();
        Callback::from(move |_: MouseEvent| on_cancel.emit(()))
    };
    let on_key = {
        let on_cancel = props.on_cancel.clone();
        Callback::from(move |event: KeyboardEvent| {
            if event.key() == "Escape" {
                on_cancel.emit(());
            }
        })
    };
    // MANDATORY, and a protocol requirement rather than a nicety: somebody
    // who reports a message without knowing the owner will read it has been
    // surprised by their own app — most of all in a direct chat.
    let disclosure = if props.target.message_id.is_some() {
        "Your family owner will see this message and its text."
    } else {
        "Your family owner will be told you reported this member."
    };
    html! {
        <div class="dialog-backdrop" onkeydown={on_key}>
            <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="report-title">
                <h2 id="report-title">{ "Report" }</h2>
                <fieldset class="reasons">
                    <legend>{ format!("Why are you reporting {}?", props.target.name) }</legend>
                    { for REASONS.iter().map(|(code, label)| html! {
                        <label class="reason">
                            <input
                                type="radio"
                                name="reason"
                                value={*code}
                                checked={*reason == *code}
                                onchange={choose(code)}
                            />
                            { *label }
                        </label>
                    }) }
                </fieldset>
                <p class="footnote">{ disclosure }</p>
                if let Some(contact) = props.support_contact.clone().filter(|contact| !contact.is_empty()) {
                    <section class="escalation">
                        <h3>{ "If the problem is the owner" }</h3>
                        <p class="verbatim">{ contact }</p>
                        <p class="footnote">{ "This server's operator published this contact." }</p>
                    </section>
                }
                <div class="dialog-actions">
                    <button class="secondary" onclick={cancel}>{ "Cancel" }</button>
                    <button class="primary" onclick={submit}>{ "Report" }</button>
                </div>
            </div>
        </div>
    }
}
