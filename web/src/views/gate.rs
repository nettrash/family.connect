//! An account with no family yet: the fork — make one or join one — and the
//! waiting room after asking to join a family that approves its members
//! (ios FamilyGateView, CreateFamilyView, JoinFamilyView,
//! PendingApprovalView).
//!
//! A server may take no new families at all (docs/protocol.md, "Starting a
//! family"): then there is no Create, and the gate says where to run a
//! server of one's own instead. And a server may remove an account that
//! never joins one ("Accounts without a family"): the gate says so, with the
//! number of days, before the deadline rather than after it.

use fc_text::account;
use gloo_timers::callback::Interval;
use web_sys::HtmlInputElement;
use yew::prelude::*;

use crate::actions::Action;
use crate::api::ApiError;
use crate::model::Me;
use crate::views::dialog::{server_trouble, SERVER_TROUBLE};

/// Where the project — the server and how to install it — lives.
pub const REPOSITORY: &str = "https://github.com/nettrash/family.connect";

/// How often the waiting room asks whether the owner has answered — the
/// apps' five seconds. The server also tells the family, the newcomer
/// included, the moment a request is approved; this is for the refusal,
/// which nothing announces.
pub const POLL_MS: u32 = 5_000;

#[derive(Properties, PartialEq)]
pub struct GateProps {
    pub account: Me,
    /// The request this account was waiting on was declined.
    pub declined: bool,
    /// Something to say on arrival — "Ownership passed on" after leaving.
    pub notice: Option<String>,
    /// Something that went wrong reading the account — said here too, or
    /// the gate would sit on a screen that is out of date and not say why.
    #[prop_or_default]
    pub failure: Option<String>,
    pub on_action: Callback<Action>,
    pub on_sign_out: Callback<()>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Step {
    Choose,
    Create,
    Join,
}

#[function_component(FamilyGate)]
pub fn family_gate(props: &GateProps) -> Html {
    let step = use_state(|| Step::Choose);
    let go = |to: Step| {
        let step = step.clone();
        Callback::from(move |_: MouseEvent| step.set(to))
    };
    let sign_out = props.on_sign_out.reform(|_: MouseEvent| ());
    let dismiss = props
        .on_action
        .reform(|_: MouseEvent| Action::DismissNotice);
    let account = &props.account;
    let open = account.family_registration_enabled;
    let ttl = account.familyless_account_ttl_days;

    let body = match *step {
        Step::Choose => html! {
            <>
                <p class="greeting">{ format!("Hi, {}", account.user.display_name) }</p>
                if !open {
                    <section class="closed-server">
                        <h2>{ "This server doesn't take new families." }</h2>
                        <p class="hint">
                            { "Family Connect is built for one family on a server of its own. To start yours, run your own server and invite everyone from there." }
                        </p>
                        <a href={REPOSITORY} target="_blank" rel="noopener noreferrer">
                            { "How to run your own server" }
                        </a>
                    </section>
                }
                <div class="doors">
                    if open {
                        <button class="door" onclick={go(Step::Create)}>
                            <strong>{ "Create a family" }</strong>
                            <span>{ "Start fresh — you'll be the owner and can invite everyone else." }</span>
                        </button>
                    }
                    <button class="door" onclick={go(Step::Join)}>
                        <strong>{ "Join a family" }</strong>
                        <span>{ "Enter the invite code a family member shared with you." }</span>
                    </button>
                </div>
                if ttl > 0 {
                    <p class="hint">{ ttl_line(ttl) }</p>
                }
            </>
        },
        Step::Create => html! {
            <CreateFamily on_action={props.on_action.clone()} on_back={go(Step::Choose)} />
        },
        Step::Join => html! {
            <JoinFamily on_action={props.on_action.clone()} on_back={go(Step::Choose)} />
        },
    };

    html! {
        <main class="login gate">
            <section class="login-card gate-card" aria-labelledby="gate-title">
                <h1 id="gate-title">{ "Your Family" }</h1>
                if let Some(failure) = props.failure.clone() {
                    <p class="error" role="alert">
                        { failure }
                        <button class="link" onclick={dismiss.clone()} aria-label="Dismiss">{ "✕" }</button>
                    </p>
                } else if let Some(notice) = props.notice.clone() {
                    <p class="notice" role="status">
                        { notice }
                        <button class="link" onclick={dismiss} aria-label="Dismiss">{ "✕" }</button>
                    </p>
                }
                if props.declined {
                    <p class="banner" role="status">
                        { "Your request to join was declined. You can ask for a new invite code and try again." }
                    </p>
                }
                { body }
                <button class="link signout" onclick={sign_out}>{ "Sign out" }</button>
            </section>
        </main>
    }
}

/// The deadline, said before it is met.
pub fn ttl_line(days: i64) -> String {
    if days == 1 {
        "An account that doesn't join a family within 1 day is removed from this server."
            .to_string()
    } else {
        format!(
            "An account that doesn't join a family within {days} days is removed from this server."
        )
    }
}

/// A field that takes the focus as its step opens: one field, and on a
/// computer it is already where the typing goes (ios CreateFamilyView).
#[hook]
fn use_focused() -> NodeRef {
    let field = use_node_ref();
    {
        let field = field.clone();
        use_effect_with((), move |_| {
            if let Some(input) = field.cast::<HtmlInputElement>() {
                let _ = input.focus();
            }
        });
    }
    field
}

#[derive(Properties, PartialEq)]
struct StepProps {
    on_action: Callback<Action>,
    on_back: Callback<MouseEvent>,
}

#[function_component(CreateFamily)]
fn create_family(props: &StepProps) -> Html {
    let name = use_state(String::new);
    let busy = use_state(|| false);
    let error = use_state(|| Option::<String>::None);
    let field = use_focused();
    let on_input = {
        let name = name.clone();
        Callback::from(move |event: InputEvent| {
            let input: HtmlInputElement = event.target_unchecked_into();
            name.set(input.value());
        })
    };
    let submit = {
        let name = name.clone();
        let busy = busy.clone();
        let error = error.clone();
        let on_action = props.on_action.clone();
        Callback::from(move |event: SubmitEvent| {
            event.prevent_default();
            if *busy || name.trim().is_empty() {
                return;
            }
            let Some(chosen) = account::name(&name) else {
                error.set(Some("A family name is 1 to 64 characters.".to_string()));
                return;
            };
            busy.set(true);
            error.set(None);
            let busy = busy.clone();
            let error = error.clone();
            let refresh = on_action.clone();
            on_action.emit(Action::CreateFamily {
                name: chosen.to_string(),
                done: Callback::from(move |failure: Option<ApiError>| {
                    busy.set(false);
                    if let Some(failure) = failure {
                        // Already in one — from another device, a moment
                        // ago: `/me` says which, and the app goes there.
                        if failure.code() == Some("already_in_family") {
                            refresh.emit(Action::RefreshAccount);
                        }
                        error.set(Some(create_failure(&failure)));
                    }
                }),
            });
        })
    };
    html! {
        <form class="gate-step" onsubmit={submit}>
            <button type="button" class="link back" onclick={props.on_back.clone()}>{ "‹ Back" }</button>
            <h2>{ "Create a Family" }</h2>
            <label for="family-name">{ "Family name" }</label>
            <input
                id="family-name"
                ref={field}
                type="text"
                placeholder="The Smiths"
                autocomplete="off"
                autocapitalize="words"
                value={(*name).clone()}
                oninput={on_input}
            />
            if let Some(message) = (*error).clone() {
                <p class="error" role="alert">{ message }</p>
            } else {
                <p class="hint">{ "This names your family chat too. 1–64 characters." }</p>
            }
            <button type="submit" disabled={*busy || name.trim().is_empty()}>
                { if *busy { "Creating…" } else { "Create Family" } }
            </button>
        </form>
    }
}

/// Why a family was not made (ios CreateFamilyView).
pub fn create_failure(error: &ApiError) -> String {
    if server_trouble(error) {
        return SERVER_TROUBLE.to_string();
    }
    match error.code() {
        // Reachable only on a server that shut its door after the gate was
        // drawn: the gate itself offers no Create there.
        Some("family_registration_disabled") => {
            "This server doesn't take new families.".to_string()
        }
        Some("already_in_family") => "You're already in a family.".to_string(),
        Some(_) => match error {
            ApiError::Server { message, .. } if !message.is_empty() => message.clone(),
            _ => "The server rejected that name.".to_string(),
        },
        None => unreached(error),
    }
}

#[function_component(JoinFamily)]
fn join_family(props: &StepProps) -> Html {
    let code = use_state(String::new);
    let busy = use_state(|| false);
    let error = use_state(|| Option::<String>::None);
    let field = use_focused();
    let on_input = {
        let code = code.clone();
        Callback::from(move |event: InputEvent| {
            let input: HtmlInputElement = event.target_unchecked_into();
            code.set(input.value());
        })
    };
    let submit = {
        let code = code.clone();
        let busy = busy.clone();
        let error = error.clone();
        let on_action = props.on_action.clone();
        Callback::from(move |event: SubmitEvent| {
            event.prevent_default();
            let typed = account::invite_code(&code);
            if *busy || typed.is_empty() {
                return;
            }
            busy.set(true);
            error.set(None);
            let busy = busy.clone();
            let error = error.clone();
            let refresh = on_action.clone();
            on_action.emit(Action::JoinFamily {
                invite_code: typed,
                done: Callback::from(move |answer: Result<String, ApiError>| {
                    busy.set(false);
                    match answer {
                        // Answered, and `/me` read after it: the app is on
                        // its way to the family or the waiting room — or,
                        // had that read failed, the gate says so above.
                        Ok(_) => {}
                        Err(failure) => {
                            // Already waiting, or already in: this screen
                            // is out of date, and `/me` says where to be.
                            if matches!(
                                failure.code(),
                                Some("join_request_pending") | Some("already_in_family")
                            ) {
                                refresh.emit(Action::RefreshAccount);
                            }
                            error.set(Some(join_failure(&failure)));
                        }
                    }
                }),
            });
        })
    };
    html! {
        <form class="gate-step" onsubmit={submit}>
            <button type="button" class="link back" onclick={props.on_back.clone()}>{ "‹ Back" }</button>
            <h2>{ "Join a Family" }</h2>
            <label for="invite-code">{ "Invite code" }</label>
            // Shown in capitals as it is typed and sent in capitals: people
            // read a code off somebody else's screen and should not have to
            // fight the keyboard's idea of case either way.
            <input
                id="invite-code"
                ref={field}
                class="code"
                type="text"
                placeholder="ABCD2345"
                autocomplete="off"
                autocapitalize="characters"
                spellcheck="false"
                value={(*code).clone()}
                oninput={on_input}
            />
            if let Some(message) = (*error).clone() {
                <p class="error" role="alert">{ message }</p>
            } else {
                <p class="hint">{ "Any family member can read the code to you; the owner finds it under Family." }</p>
            }
            <button type="submit" disabled={*busy || code.trim().is_empty()}>
                { if *busy { "Joining…" } else { "Join" } }
            </button>
        </form>
    }
}

/// Why a code did not get somebody in (ios JoinFamilyView). A CLOSED family
/// answers `invalid_invite_code`, byte for byte a code that never existed,
/// so the first sentence is right for it too.
pub fn join_failure(error: &ApiError) -> String {
    if server_trouble(error) {
        return SERVER_TROUBLE.to_string();
    }
    match error.code() {
        Some("invalid_invite_code") => {
            "That code doesn't match any family. Check it and try again.".to_string()
        }
        // The second is the one join that can answer it: an account scrubbed
        // while its join was on the way (the action signs it out).
        Some("already_in_family") | Some("user_already_in_family") => {
            "You're already in a family.".to_string()
        }
        Some("join_request_pending") => "You already have a pending request.".to_string(),
        Some("family_full") => {
            "That family is full right now. Ask them to make room, then try the code again."
                .to_string()
        }
        Some(_) => match error {
            ApiError::Server { message, .. } if !message.is_empty() => message.clone(),
            _ => "The server rejected that code.".to_string(),
        },
        None => unreached(error),
    }
}

/// A request that got no protocol answer at all — or the server's own
/// trouble, which is not a network to check.
fn unreached(error: &ApiError) -> String {
    match error {
        ApiError::Throttled { .. } => error.detail(),
        _ if server_trouble(error) => SERVER_TROUBLE.to_string(),
        _ => "Can't reach the server. Try again.".to_string(),
    }
}

#[derive(Properties, PartialEq)]
pub struct PendingProps {
    pub on_action: Callback<Action>,
    pub on_sign_out: Callback<()>,
}

/// The waiting room. It asks every five seconds while the tab is in front,
/// and at once on "Check now".
#[function_component(PendingApproval)]
pub fn pending_approval(props: &PendingProps) -> Html {
    {
        let on_action = props.on_action.clone();
        use_effect_with((), move |_| {
            let ticker = Interval::new(POLL_MS, move || {
                if crate::sync::page_visible() {
                    on_action.emit(Action::PollAccount);
                }
            });
            move || drop(ticker)
        });
    }
    let check = props.on_action.reform(|_: MouseEvent| Action::PollAccount);
    let sign_out = props.on_sign_out.reform(|_: MouseEvent| ());
    html! {
        <main class="login gate">
            <section class="login-card gate-card pending" aria-labelledby="pending-title">
                <h1 id="pending-title">{ "Almost There" }</h1>
                <div class="hourglass" aria-hidden="true">{ "⌛" }</div>
                <h2>{ "Waiting for approval" }</h2>
                <p class="hint">
                    { "The family owner needs to approve your request. This page updates automatically — or check right now." }
                </p>
                <button onclick={check}>{ "Check now" }</button>
                <button class="link signout" onclick={sign_out}>{ "Sign out" }</button>
            </section>
        </main>
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
    fn a_join_is_refused_in_the_apps_words() {
        assert_eq!(
            join_failure(&server("invalid_invite_code", "invite code not found")),
            "That code doesn't match any family. Check it and try again."
        );
        assert_eq!(
            join_failure(&server("family_full", "family is full")),
            "That family is full right now. Ask them to make room, then try the code again."
        );
        assert_eq!(
            join_failure(&server("join_request_pending", "x")),
            "You already have a pending request."
        );
        assert_eq!(
            join_failure(&server(
                "user_already_in_family",
                "user is already in a family"
            )),
            "You're already in a family."
        );
        assert_eq!(
            join_failure(&server("some_new_code", "a rule the server names")),
            "a rule the server names"
        );
        assert_eq!(
            join_failure(&ApiError::Network("TypeError".into())),
            "Can't reach the server. Try again."
        );
        assert_eq!(
            join_failure(&ApiError::Network("The server answered 502.".into())),
            "The server had a problem. Try again in a moment."
        );
    }

    #[wasm_bindgen_test]
    fn a_create_is_refused_in_the_apps_words() {
        assert_eq!(
            create_failure(&server("family_registration_disabled", "x")),
            "This server doesn't take new families."
        );
        assert_eq!(
            create_failure(&server("validation", "family name must be 1-64 characters")),
            "family name must be 1-64 characters"
        );
    }

    #[wasm_bindgen_test]
    fn the_deadline_has_a_singular() {
        assert_eq!(
            ttl_line(1),
            "An account that doesn't join a family within 1 day is removed from this server."
        );
        assert_eq!(
            ttl_line(7),
            "An account that doesn't join a family within 7 days is removed from this server."
        );
    }
}
