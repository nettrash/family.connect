//! The one screen a signed-out browser shows.
//!
//! There is no server-URL field, unlike the phone apps: the page was served
//! by the server it talks to (docs/protocol.md, "A browser is a client
//! too"). Registration is not here either — this slice signs in an account
//! that exists; creating one is the app's job until the web client learns
//! the family gate.

use wasm_bindgen_futures::spawn_local;
use web_sys::HtmlInputElement;
use yew::prelude::*;

use crate::api;

#[derive(Properties, PartialEq)]
pub struct LoginProps {
    /// Handed the token, once.
    pub on_signed_in: Callback<String>,
}

#[function_component(Login)]
pub fn login(props: &LoginProps) -> Html {
    let username = use_state(String::new);
    let password = use_state(String::new);
    let error = use_state(|| Option::<String>::None);
    let busy = use_state(|| false);

    let on_username = {
        let username = username.clone();
        Callback::from(move |event: InputEvent| {
            let input: HtmlInputElement = event.target_unchecked_into();
            username.set(input.value());
        })
    };
    let on_password = {
        let password = password.clone();
        Callback::from(move |event: InputEvent| {
            let input: HtmlInputElement = event.target_unchecked_into();
            password.set(input.value());
        })
    };

    let submit = {
        let username = username.clone();
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
            let (name, secret) = ((*username).clone(), (*password).clone());
            if name.trim().is_empty() || secret.is_empty() {
                return;
            }
            busy.set(true);
            error.set(None);
            let error = error.clone();
            let busy = busy.clone();
            let on_signed_in = on_signed_in.clone();
            spawn_local(async move {
                match api::login(name.trim(), &secret).await {
                    Ok(auth) => on_signed_in.emit(auth.token),
                    Err(failure) => {
                        // The protocol's own answer, said the way a person
                        // would: `invalid_credentials` is the ordinary case
                        // and its English message is for developers.
                        error.set(Some(match failure.code() {
                            Some("invalid_credentials") => {
                                "That username and password do not match.".to_string()
                            }
                            _ => failure.detail(),
                        }));
                        busy.set(false);
                    }
                }
            });
        })
    };

    html! {
        <main class="login">
            <form class="login-card" onsubmit={submit}>
                <h1>{ "Family Connect" }</h1>
                <label for="username">{ "Username" }</label>
                <input
                    id="username"
                    type="text"
                    autocomplete="username"
                    autocapitalize="none"
                    spellcheck="false"
                    value={(*username).clone()}
                    oninput={on_username}
                />
                <label for="password">{ "Password" }</label>
                <input
                    id="password"
                    type="password"
                    autocomplete="current-password"
                    value={(*password).clone()}
                    oninput={on_password}
                />
                if let Some(message) = (*error).clone() {
                    <p class="error" role="alert">{ message }</p>
                }
                <button type="submit" disabled={*busy}>
                    { if *busy { "Signing in…" } else { "Sign in" } }
                </button>
            </form>
        </main>
    }
}
