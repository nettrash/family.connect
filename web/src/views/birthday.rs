//! A birthday: the reader's own, or — the owner's tool — any member's
//! (ios Views/BirthdayView.swift). A day and a month and never a year: the
//! protocol carries no year, so a birthday wished is never an age published
//! (docs/protocol.md, "Birthdays").

use fc_text::account;
use web_sys::HtmlSelectElement;
use yew::prelude::*;

use crate::actions::Action;
use crate::api::ApiError;
use crate::model::Birthday;
use crate::time;
use crate::views::dialog::Modal;
use fc_text::i18n::{t, t1};

#[derive(Properties, PartialEq)]
pub struct BirthdayProps {
    /// None for the reader's own; the member's id for the owner's tool.
    #[prop_or_default]
    pub user_id: Option<i64>,
    /// Whose it is, for the owner's title.
    #[prop_or_default]
    pub name: Option<String>,
    pub current: Option<Birthday>,
    pub on_action: Callback<Action>,
    pub on_close: Callback<()>,
}

#[function_component(BirthdayDialog)]
pub fn birthday_dialog(props: &BirthdayProps) -> Html {
    // Unset opens on the 1st of January; a held one opens on itself, held
    // inside the calendar in case it came from somewhere that was not.
    let start = props.current.map_or((1, 1), |held| {
        let month = held.month.clamp(1, 12);
        (month, held.day.clamp(1, account::days_in(month)))
    });
    let month = use_state(|| start.0);
    let day = use_state(|| start.1);
    let busy = use_state(|| false);
    let error = use_state(|| Option::<String>::None);

    let pick_month = {
        let month = month.clone();
        let day = day.clone();
        Callback::from(move |event: Event| {
            let select: HtmlSelectElement = event.target_unchecked_into();
            let chosen = select.value().parse().unwrap_or(1u32).clamp(1, 12);
            month.set(chosen);
            // The 31st of a month with 30 is the 30th.
            day.set((*day).min(account::days_in(chosen)));
        })
    };
    let pick_day = {
        let day = day.clone();
        Callback::from(move |event: Event| {
            let select: HtmlSelectElement = event.target_unchecked_into();
            day.set(select.value().parse().unwrap_or(1));
        })
    };
    let send = {
        let busy = busy.clone();
        let error = error.clone();
        let on_action = props.on_action.clone();
        let on_close = props.on_close.clone();
        let user_id = props.user_id;
        move |birthday: Option<Birthday>| {
            if *busy {
                return;
            }
            busy.set(true);
            error.set(None);
            let busy = busy.clone();
            let error = error.clone();
            let on_close = on_close.clone();
            on_action.emit(Action::SetBirthday {
                user_id,
                birthday,
                done: Callback::from(move |failure: Option<ApiError>| match failure {
                    None => on_close.emit(()),
                    Some(failure) => {
                        busy.set(false);
                        error.set(Some(birthday_failure(&failure)));
                    }
                }),
            });
        }
    };
    let save = {
        let send = send.clone();
        let (month, day) = (*month, *day);
        Callback::from(move |_: MouseEvent| send(Some(Birthday { month, day })))
    };
    let remove = Callback::from(move |_: MouseEvent| send(None));
    let cancel = props.on_close.reform(|_: MouseEvent| ());

    let title = match (&props.user_id, &props.name) {
        (Some(_), Some(name)) => t1("Birthday for %@", name),
        _ => t("Birthday").to_string(),
    };
    let footnote = if props.user_id.is_some() {
        t("A day and a month, with no year. Everyone in the family sees it.")
    } else {
        t("A day and a month, with no year — so being wished a happy birthday never means publishing your age.")
    };
    html! {
        <Modal title={title} on_cancel={props.on_close.clone()} busy={*busy}>
            <div class="birthday-fields">
                <label class="field">
                    { t("Month") }
                    <select onchange={pick_month}>
                        { for (1..=12u32).map(|value| html! {
                            <option value={value.to_string()} selected={value == *month}>
                                { time::month_name(value) }
                            </option>
                        }) }
                    </select>
                </label>
                <label class="field">
                    { t("Day") }
                    <select onchange={pick_day}>
                        { for (1..=account::days_in(*month)).map(|value| html! {
                            <option value={value.to_string()} selected={value == *day}>
                                { value }
                            </option>
                        }) }
                    </select>
                </label>
            </div>
            <p class="footnote">{ footnote }</p>
            if let Some(message) = (*error).clone() {
                <p class="error" role="alert">{ message }</p>
            }
            <div class="dialog-actions">
                if props.current.is_some() {
                    <button class="secondary danger push-left" disabled={*busy} onclick={remove}>
                        { t("Remove Birthday") }
                    </button>
                }
                <button class="secondary" disabled={*busy} onclick={cancel}>{ t("Cancel") }</button>
                <button class="primary" disabled={*busy} onclick={save}>{ t("Save") }</button>
            </div>
        </Modal>
    }
}

/// Why a birthday did not save (ios BirthdayFailure).
pub fn birthday_failure(error: &ApiError) -> String {
    match error.code() {
        Some("validation") => t("That date doesn't exist.").to_string(),
        Some("not_family_owner") => t("Only the family owner can do that.").to_string(),
        _ => t("Couldn't save that birthday. Try again.").to_string(),
    }
}
