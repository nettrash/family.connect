//! The one screen a signed-out browser shows: signing in, and — behind the
//! switch beside it — registering an account, as the apps put the two
//! stories on one screen (ios AuthView).
//!
//! There is no server-URL field, unlike the phone apps: the page was served
//! by the server it talks to (docs/protocol.md, "A browser is a client
//! too"). A new account belongs to no family; the family gate after this
//! screen is where it makes or joins one.

use fc_text::account;
use wasm_bindgen_futures::spawn_local;
use web_sys::HtmlInputElement;
use yew::prelude::*;

use crate::api::{self, ApiError};
use crate::views::dialog::{server_trouble, SERVER_TROUBLE};

#[derive(Properties, PartialEq)]
pub struct LoginProps {
    /// Handed the token, once.
    pub on_signed_in: Callback<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    SignIn,
    Register,
}

#[function_component(Login)]
pub fn login(props: &LoginProps) -> Html {
    let mode = use_state(|| Mode::SignIn);
    let username = use_state(String::new);
    let display_name = use_state(String::new);
    let password = use_state(String::new);
    let error = use_state(|| Option::<String>::None);
    let busy = use_state(|| false);
    // A keyboard is how a browser signs in: the first field is where the
    // typing starts (ios AuthView focuses it on the Mac).
    let first = use_node_ref();
    {
        let first = first.clone();
        use_effect_with((), move |_| {
            if let Some(input) = first.cast::<HtmlInputElement>() {
                let _ = input.focus();
            }
        });
    }

    let field = |handle: &UseStateHandle<String>| {
        let handle = handle.clone();
        Callback::from(move |event: InputEvent| {
            let input: HtmlInputElement = event.target_unchecked_into();
            handle.set(input.value());
        })
    };
    let switch_to = |to: Mode| {
        let mode = mode.clone();
        let error = error.clone();
        let busy = busy.clone();
        Callback::from(move |_: MouseEvent| {
            if !*busy {
                mode.set(to);
                error.set(None);
            }
        })
    };

    let submit = {
        let mode = *mode;
        let username = username.clone();
        let display_name = display_name.clone();
        let password = password.clone();
        let error = error.clone();
        let busy = busy.clone();
        let on_signed_in = props.on_signed_in.clone();
        Callback::from(move |event: SubmitEvent| {
            // A form in a browser navigates unless it is told not to, and a
            // navigation here would reload the app mid-login.
            event.prevent_default();
            if *busy {
                return;
            }
            let name = username.trim().to_string();
            let secret = (*password).clone();
            if name.is_empty() || secret.is_empty() {
                return;
            }
            let shown_name = display_name.trim().to_string();
            if mode == Mode::Register {
                // The server's rules, said at the field instead of by a
                // refusal (fc_text::account holds them).
                if let Some(problem) = account::username_problem(&name) {
                    error.set(Some(problem.message().to_string()));
                    return;
                }
                if account::name(&shown_name).is_none() {
                    error.set(Some("A display name is 1 to 64 characters.".to_string()));
                    return;
                }
                if !account::password_ok(&secret) {
                    error.set(Some("A password needs at least 8 characters.".to_string()));
                    return;
                }
            }
            busy.set(true);
            error.set(None);
            let error = error.clone();
            let busy = busy.clone();
            let on_signed_in = on_signed_in.clone();
            spawn_local(async move {
                let answer = match mode {
                    Mode::SignIn => api::login(&name, &secret).await,
                    Mode::Register => api::register(&name, &shown_name, &secret).await,
                };
                match answer {
                    Ok(auth) => on_signed_in.emit(auth.token),
                    Err(failure) => {
                        error.set(Some(match mode {
                            Mode::SignIn => sign_in_failure(&failure),
                            Mode::Register => register_failure(&failure),
                        }));
                        busy.set(false);
                    }
                }
            });
        })
    };

    let registering = *mode == Mode::Register;
    let button = match (*mode, *busy) {
        (Mode::SignIn, false) => "Sign in",
        (Mode::SignIn, true) => "Signing in…",
        (Mode::Register, false) => "Create Account",
        (Mode::Register, true) => "Creating account…",
    };
    html! {
        <main class="login">
            <form class="login-card" onsubmit={submit}>
                <h1>{ "Family Connect" }</h1>
                <div class="segmented" role="group" aria-label="Sign in or register">
                    <button
                        type="button"
                        class={classes!((!registering).then_some("is-chosen"))}
                        aria-pressed={if registering { "false" } else { "true" }}
                        onclick={switch_to(Mode::SignIn)}
                    >{ "Sign in" }</button>
                    <button
                        type="button"
                        class={classes!(registering.then_some("is-chosen"))}
                        aria-pressed={if registering { "true" } else { "false" }}
                        onclick={switch_to(Mode::Register)}
                    >{ "Register" }</button>
                </div>
                <label for="username">{ "Username" }</label>
                <input
                    id="username"
                    ref={first}
                    type="text"
                    autocomplete="username"
                    autocapitalize="none"
                    spellcheck="false"
                    value={(*username).clone()}
                    oninput={field(&username)}
                />
                if registering {
                    <label for="display-name">{ "Display name" }</label>
                    <input
                        id="display-name"
                        type="text"
                        autocomplete="name"
                        value={(*display_name).clone()}
                        oninput={field(&display_name)}
                    />
                }
                <label for="password">{ "Password" }</label>
                <input
                    id="password"
                    type="password"
                    autocomplete={if registering { "new-password" } else { "current-password" }}
                    value={(*password).clone()}
                    oninput={field(&password)}
                />
                if let Some(message) = (*error).clone() {
                    <p class="error" role="alert">{ message }</p>
                } else if registering {
                    <p class="hint">
                        { "Usernames are 3–32 letters, digits, dots or underscores. Passwords need at least 8 characters." }
                    </p>
                }
                <button type="submit" disabled={*busy}>{ button }</button>
            </form>
        </main>
    }
}

/// The protocol's own answer to a sign-in, said the way a person would
/// (ios AuthView.describe): `invalid_credentials` is the ordinary case, and
/// its English message is for developers.
fn sign_in_failure(failure: &ApiError) -> String {
    match failure {
        ApiError::Server { code, .. } if code == "invalid_credentials" => {
            "Wrong username or password.".to_string()
        }
        other => unanswered(other),
    }
}

/// What is left once the codes a screen has words for are taken: the
/// server's own sentence for a rule it names, else what kind of failure it
/// was.
fn unanswered(failure: &ApiError) -> String {
    match failure {
        _ if server_trouble(failure) => SERVER_TROUBLE.to_string(),
        ApiError::Server { message, .. } if !message.is_empty() => message.clone(),
        ApiError::Server { .. } => "The server rejected the request.".to_string(),
        ApiError::Network(_) => "Can't reach the server. Check your connection.".to_string(),
        ApiError::Throttled { .. } => failure.detail(),
        ApiError::Unauthorized => "The server rejected the request. Try again.".to_string(),
    }
}

/// A registration the server refused. `validation` names the rule it
/// broke, in the server's own words (ios AuthView does the same) — the
/// checks above make it rare.
fn register_failure(failure: &ApiError) -> String {
    match failure {
        ApiError::Server { code, .. } if code == "username_taken" => {
            "That username is taken.".to_string()
        }
        other => unanswered(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test;

    fn server(code: &str, message: &str) -> ApiError {
        ApiError::Server {
            code: code.into(),
            message: message.into(),
        }
    }

    #[wasm_bindgen_test]
    fn a_taken_username_is_said_plainly() {
        assert_eq!(
            register_failure(&server("username_taken", "username is already in use")),
            "That username is taken."
        );
    }

    #[wasm_bindgen_test]
    fn a_broken_rule_is_the_servers_own_sentence() {
        assert_eq!(
            register_failure(&server(
                "validation",
                "display_name must be 1-64 characters"
            )),
            "display_name must be 1-64 characters"
        );
        assert_eq!(
            register_failure(&ApiError::Network("TypeError".into())),
            "Can't reach the server. Check your connection."
        );
    }

    #[wasm_bindgen_test]
    fn wrong_credentials_are_not_the_developers_message() {
        assert_eq!(
            sign_in_failure(&server(
                "invalid_credentials",
                "invalid username or password"
            )),
            "Wrong username or password."
        );
        // Nothing came back, or a proxy answered for a server that is
        // restarting: neither is the developer's text, nor the same thing.
        assert_eq!(
            sign_in_failure(&ApiError::Network("TypeError: Failed to fetch".into())),
            "Can't reach the server. Check your connection."
        );
        assert_eq!(
            sign_in_failure(&ApiError::Network("The server answered 502.".into())),
            "The server had a problem. Try again in a moment."
        );
    }
}
