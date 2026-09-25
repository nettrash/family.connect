//! Passwords: changing one's own, and — the owner's tool — setting a new
//! one for a member who has lost theirs (ios Views/PasswordView.swift).
//!
//! The one rule is the server's, counted as it counts: at least eight
//! Unicode scalars (fc_text::account). The apps count graphemes, which
//! refuses passwords the server would take.

use fc_text::account;
use web_sys::HtmlInputElement;
use yew::prelude::*;

use crate::actions::Action;
use crate::api::ApiError;
use crate::views::dialog::Modal;
use fc_text::i18n::{t, t1, tn};

/// What is wrong with a new password and its confirmation, if anything.
pub fn new_password_problem(new: &str, confirmation: &str) -> Option<String> {
    if !account::password_ok(new) {
        return Some(tn(
            "Use at least %lld characters.",
            account::MIN_PASSWORD_CHARS as i64,
        ));
    }
    if new != confirmation {
        return Some(t("Those two do not match.").to_string());
    }
    None
}

fn field(handle: &UseStateHandle<String>) -> Callback<InputEvent> {
    let handle = handle.clone();
    Callback::from(move |event: InputEvent| {
        let input: HtmlInputElement = event.target_unchecked_into();
        handle.set(input.value());
    })
}

#[derive(Properties, PartialEq)]
pub struct ChangePasswordProps {
    pub on_action: Callback<Action>,
    pub on_close: Callback<()>,
}

#[function_component(ChangePasswordDialog)]
pub fn change_password_dialog(props: &ChangePasswordProps) -> Html {
    let current = use_state(String::new);
    let new = use_state(String::new);
    let confirmation = use_state(String::new);
    let busy = use_state(|| false);
    let error = use_state(|| Option::<String>::None);
    let changed = use_state(|| false);
    let close = props.on_close.reform(|_: MouseEvent| ());

    if *changed {
        return html! {
            <Modal key="changed" title={t("Password changed")} on_cancel={props.on_close.clone()}>
                <p class="dialog-message">{ t("Your other devices have been signed out.") }</p>
                <div class="dialog-actions">
                    <button class="primary" onclick={close}>{ "OK" }</button>
                </div>
            </Modal>
        };
    }

    let submit = {
        let (current, new, confirmation) = (current.clone(), new.clone(), confirmation.clone());
        let (busy, error, changed) = (busy.clone(), error.clone(), changed.clone());
        let on_action = props.on_action.clone();
        Callback::from(move |event: SubmitEvent| {
            event.prevent_default();
            if *busy || current.is_empty() || new.is_empty() {
                return;
            }
            if let Some(problem) = new_password_problem(&new, &confirmation) {
                error.set(Some(problem));
                return;
            }
            busy.set(true);
            error.set(None);
            let (busy, error, changed) = (busy.clone(), error.clone(), changed.clone());
            on_action.emit(Action::ChangePassword {
                current: (*current).clone(),
                new: (*new).clone(),
                done: Callback::from(move |failure: Option<ApiError>| {
                    busy.set(false);
                    match failure {
                        None => changed.set(true),
                        Some(failure) => error.set(Some(match failure.code() {
                            Some("invalid_credentials") => {
                                t("That current password is not right.").to_string()
                            }
                            _ => t("Couldn't change your password. Try again.").to_string(),
                        })),
                    }
                }),
            });
        })
    };
    html! {
        <Modal key="form" title={t("Change Password")} on_cancel={props.on_close.clone()} busy={*busy}>
            <form class="dialog-form" onsubmit={submit}>
                <label class="field">
                    { t("Current Password") }
                    <input type="password" autocomplete="current-password" value={(*current).clone()} oninput={field(&current)} />
                </label>
                <p class="footnote">{ t("Your other devices will be signed out. This one stays signed in.") }</p>
                <label class="field">
                    { t("New Password") }
                    <input type="password" autocomplete="new-password" value={(*new).clone()} oninput={field(&new)} />
                </label>
                <label class="field">
                    { t("Confirm New Password") }
                    <input type="password" autocomplete="new-password" value={(*confirmation).clone()} oninput={field(&confirmation)} />
                </label>
                if let Some(message) = (*error).clone() {
                    <p class="error" role="alert">{ message }</p>
                }
                <div class="dialog-actions">
                    <button type="button" class="secondary" disabled={*busy} onclick={close}>{ t("Cancel") }</button>
                    <button type="submit" class="primary" disabled={*busy || current.is_empty() || new.is_empty()}>
                        { t("Save") }
                    </button>
                </div>
            </form>
        </Modal>
    }
}

#[derive(Properties, PartialEq)]
pub struct ResetPasswordProps {
    pub user_id: i64,
    pub name: String,
    pub on_action: Callback<Action>,
    pub on_close: Callback<()>,
}

/// The owner's reset. The member is signed out everywhere and needs this
/// password to come back, and the server has no way to send it to them —
/// so the owner is told to, somewhere safe.
#[function_component(ResetPasswordDialog)]
pub fn reset_password_dialog(props: &ResetPasswordProps) -> Html {
    let new = use_state(String::new);
    let confirmation = use_state(String::new);
    let busy = use_state(|| false);
    let error = use_state(|| Option::<String>::None);
    let done = use_state(|| false);
    let close = props.on_close.reform(|_: MouseEvent| ());

    if *done {
        return html! {
            <Modal key="done" title={t("Password reset")} on_cancel={props.on_close.clone()}>
                <p class="dialog-message">{ t1("%@ has been signed out everywhere.", &props.name) }</p>
                <div class="dialog-actions">
                    <button class="primary" onclick={close}>{ "OK" }</button>
                </div>
            </Modal>
        };
    }

    let submit = {
        let (new, confirmation) = (new.clone(), confirmation.clone());
        let (busy, error, finished) = (busy.clone(), error.clone(), done.clone());
        let on_action = props.on_action.clone();
        let user_id = props.user_id;
        Callback::from(move |event: SubmitEvent| {
            event.prevent_default();
            if *busy || new.is_empty() {
                return;
            }
            if let Some(problem) = new_password_problem(&new, &confirmation) {
                error.set(Some(problem));
                return;
            }
            busy.set(true);
            error.set(None);
            let (busy, error, finished) = (busy.clone(), error.clone(), finished.clone());
            on_action.emit(Action::ResetMemberPassword {
                user_id,
                new_password: (*new).clone(),
                done: Callback::from(move |failure: Option<ApiError>| {
                    busy.set(false);
                    match failure {
                        None => finished.set(true),
                        Some(_) => error.set(Some(
                            t("Couldn't reset that password. Try again.").to_string(),
                        )),
                    }
                }),
            });
        })
    };
    html! {
        <Modal key="form" title={t("Reset Password")} on_cancel={props.on_close.clone()} busy={*busy}>
            <form class="dialog-form" onsubmit={submit}>
                <p class="dialog-message">{ t1("New password for %@", &props.name) }</p>
                <label class="field">
                    { t("New Password") }
                    <input type="password" autocomplete="new-password" value={(*new).clone()} oninput={field(&new)} />
                </label>
                <label class="field">
                    { t("Confirm New Password") }
                    <input type="password" autocomplete="new-password" value={(*confirmation).clone()} oninput={field(&confirmation)} />
                </label>
                <p class="footnote">
                    { t1("%@ will be signed out on every device and will need this password to sign back in. Tell it to them somewhere safe — the server has no way to email it.", &props.name) }
                </p>
                if let Some(message) = (*error).clone() {
                    <p class="error" role="alert">{ message }</p>
                }
                <div class="dialog-actions">
                    <button type="button" class="secondary" disabled={*busy} onclick={close}>{ t("Cancel") }</button>
                    <button type="submit" class="danger-button" disabled={*busy || new.is_empty()}>
                        { t("Reset") }
                    </button>
                </div>
            </form>
        </Modal>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test;

    #[wasm_bindgen_test]
    fn a_new_password_is_eight_scalars_and_typed_twice() {
        assert_eq!(
            new_password_problem("1234567", "1234567").as_deref(),
            Some("Use at least 8 characters.")
        );
        assert_eq!(
            new_password_problem("12345678", "12345679").as_deref(),
            Some("Those two do not match.")
        );
        assert_eq!(new_password_problem("12345678", "12345678"), None);
        // Counted in characters, as the server counts: seven Cyrillic
        // letters are seven, however many bytes they take.
        assert_eq!(
            new_password_problem("пароль1", "пароль1").as_deref(),
            Some("Use at least 8 characters.")
        );
        // Four family emoji: four graphemes, which the apps refuse, and
        // twenty-eight scalars, which the server takes.
        let family = "👨‍👩‍👧‍👦".repeat(4);
        assert_eq!(new_password_problem(&family, &family), None);
    }
}
