//! An account with no family yet: the fork — make one or join one — and the
//! waiting room after asking to join a family that approves its members
//! (ios FamilyGateView, CreateFamilyView, JoinFamilyView,
//! PendingApprovalView). The same create and join screens are how an
//! account already in a family gets another ("New Family…", "Join Another
//! Family…"; docs/protocol.md, "Several families"), in a pane beside the
//! list rather than on a gate.
//!
//! A server may take no new families at all (docs/protocol.md, "Starting a
//! family"): then there is no Create, and the gate says where to run a
//! server of one's own instead. A server may charge for them ("Paid
//! families"): this client cannot sell the subscription, so without an
//! active one there is no Create either — and no "New Family…" form — and
//! the gate says where a family on this server is started. And a server may
//! remove an account that never joins one ("Accounts without a family"): the
//! gate says so, with the number of days, before the deadline rather than
//! after it.

use fc_text::account;
use gloo_timers::callback::Interval;
use web_sys::HtmlInputElement;
use yew::prelude::*;

use crate::actions::Action;
use crate::api::ApiError;
use crate::model::Me;
use crate::views::dialog::{server_trouble, server_trouble_line};
use fc_text::i18n::{t, t1, tn};

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
    // A shut door is said first, in its own words; a server that charges
    // for families is only asked about once the door is open.
    let paid_for = open && account.may_start_family();
    let ttl = account.familyless_account_ttl_days;

    let body = match *step {
        Step::Choose => html! {
            <>
                <p class="greeting">{ t1("Hi, %@", &account.user.display_name) }</p>
                if !open {
                    <section class="closed-server">
                        <h2>{ t("This server doesn't take new families.") }</h2>
                        <p class="hint">
                            { t("Family Connect is built for one family on a server of its own. To start yours, run your own server and invite everyone from there.") }
                        </p>
                        <a href={REPOSITORY} target="_blank" rel="noopener noreferrer">
                            { t("How to run your own server") }
                        </a>
                    </section>
                } else if !paid_for {
                    <StartElsewhere />
                }
                <div class="doors">
                    if paid_for {
                        <button class="door" onclick={go(Step::Create)}>
                            <strong>{ t("Create a family") }</strong>
                            <span>{ t("Start fresh — you'll be the owner and can invite everyone else.") }</span>
                        </button>
                    }
                    <button class="door" onclick={go(Step::Join)}>
                        <strong>{ t("Join a family") }</strong>
                        <span>{ t("Enter the invite code a family member shared with you.") }</span>
                    </button>
                </div>
                if ttl > 0 {
                    <p class="hint">{ ttl_line(ttl) }</p>
                }
            </>
        },
        Step::Create => html! {
            <CreateFamily on_action={props.on_action.clone()} on_back={Some(go(Step::Choose))} />
        },
        Step::Join => html! {
            <JoinFamily on_action={props.on_action.clone()} on_back={Some(go(Step::Choose))} />
        },
    };

    html! {
        <main class="login gate">
            <section class="login-card gate-card" aria-labelledby="gate-title">
                <h1 id="gate-title">{ t("Your Family") }</h1>
                if let Some(failure) = props.failure.clone() {
                    <p class="error" role="alert">
                        { failure }
                        <button class="link" onclick={dismiss.clone()} aria-label={t("Dismiss")}>{ "✕" }</button>
                    </p>
                } else if let Some(notice) = props.notice.clone() {
                    <p class="notice" role="status">
                        { notice }
                        <button class="link" onclick={dismiss} aria-label={t("Dismiss")}>{ "✕" }</button>
                    </p>
                }
                if props.declined {
                    <p class="banner" role="status">
                        { t("Your request to join was declined. You can ask for a new invite code and try again.") }
                    </p>
                }
                // An invitation outlives the family the two shared: the
                // last door left to an account that has left every family
                // (docs/protocol.md, "Several families").
                { for account.invitations.iter().map(|invitation| {
                    let id = invitation.id;
                    let join = props.on_action.reform(move |_: MouseEvent| Action::AcceptInvitation { invitation_id: id });
                    let decline = props.on_action.reform(move |_: MouseEvent| Action::DeclineInvitation { invitation_id: id });
                    html! {
                        <div class="banner invitation-banner" role="status" key={id.to_string()}>
                            <span>{ fc_text::families::invited(&invitation.invited_by.display_name, &invitation.family_name) }</span>
                            <span class="row-actions">
                                <button type="button" onclick={join}>{ t("Join") }</button>
                                <button type="button" class="link" onclick={decline}>{ t("Decline") }</button>
                            </span>
                        </div>
                    }
                }) }
                { body }
                <button class="link signout" onclick={sign_out}>{ t("Log out") }</button>
            </section>
        </main>
    }
}

/// The deadline, said before it is met.
pub fn ttl_line(days: i64) -> String {
    tn(
        "An account that doesn't join a family within %lld days is removed from this server.",
        days,
    )
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
pub struct StepProps {
    pub on_action: Callback<Action>,
    /// The way back: the gate's fork. None in a pane, whose header has Done.
    #[prop_or_default]
    pub on_back: Option<Callback<MouseEvent>>,
    /// What the step means, said above its field — for another family, which
    /// is not the first.
    #[prop_or_default]
    pub explanation: Option<AttrValue>,
    /// The most families one account may be in, which a refusal names.
    #[prop_or(fc_text::families::DEFAULT_MAX)]
    pub max_families: i64,
    /// It went in: the pane that asked goes.
    #[prop_or_default]
    pub on_done: Callback<()>,
}

#[derive(Properties, PartialEq)]
pub struct AnotherFamilyProps {
    /// Join with a code, rather than start one.
    pub join: bool,
    pub max_families: i64,
    /// This account may start a family on this server as far as paying
    /// goes (`Me::may_start_family`). When not, "New Family…" says where a
    /// family here is started instead of offering the form.
    #[prop_or(true)]
    pub may_start: bool,
    pub on_action: Callback<Action>,
    pub on_close: Callback<()>,
}

/// Where a family on this server is started, said in the create action's
/// place — on the gate, where the shut door's words go, and in "New
/// Family…" (docs/protocol.md, "Paid families").
#[function_component(StartElsewhere)]
pub fn start_elsewhere() -> Html {
    html! {
        <section class="closed-server start-elsewhere">
            <p>{ fc_text::paid::start_elsewhere() }</p>
        </section>
    }
}

/// "New Family…" and "Join Another Family…": the gate's own create and join
/// screens, in a pane (docs/protocol.md, "Getting another family"). A
/// family made or joined is shown at once; a request waiting on an owner is
/// a row in the switcher.
#[function_component(AnotherFamilyPane)]
pub fn another_family_pane(props: &AnotherFamilyProps) -> Html {
    let close = props.on_close.reform(|_: MouseEvent| ());
    html! {
        <section class="pane another-family-pane" aria-labelledby="another-family-title">
            <header class="pane-head">
                <h2 id="another-family-title">{ t("Family") }</h2>
                <button class="link" onclick={close}>{ t("Done") }</button>
            </header>
            <div class="pane-body">
                if props.join {
                    <JoinFamily
                        on_action={props.on_action.clone()}
                        max_families={props.max_families}
                        on_done={props.on_close.clone()}
                    />
                } else if !props.may_start {
                    <StartElsewhere />
                } else {
                    <CreateFamily
                        on_action={props.on_action.clone()}
                        explanation={AttrValue::from(t("Start your own family. You'll be its owner and can invite members of your other families."))}
                        max_families={props.max_families}
                        on_done={props.on_close.clone()}
                    />
                }
            </div>
        </section>
    }
}

#[function_component(CreateFamily)]
pub fn create_family(props: &StepProps) -> Html {
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
        let on_done = props.on_done.clone();
        let max = props.max_families;
        Callback::from(move |event: SubmitEvent| {
            event.prevent_default();
            if *busy || name.trim().is_empty() {
                return;
            }
            let Some(chosen) = account::name(&name) else {
                error.set(Some(t("A family name is 1 to 64 characters.").to_string()));
                return;
            };
            busy.set(true);
            error.set(None);
            let busy = busy.clone();
            let error = error.clone();
            let refresh = on_action.clone();
            let on_done = on_done.clone();
            on_action.emit(Action::CreateFamily {
                name: chosen.to_string(),
                done: Callback::from(move |failure: Option<ApiError>| {
                    busy.set(false);
                    match failure {
                        None => on_done.emit(()),
                        Some(failure) => {
                            // Already in one — from another device, a moment
                            // ago: `/me` says which, and the app goes there.
                            // Or the subscription ended since `/me` was read:
                            // it says so, and the door shows where to start
                            // one instead.
                            if matches!(
                                failure.code(),
                                Some("already_in_family") | Some("subscription_required")
                            ) {
                                refresh.emit(Action::RefreshAccount);
                            }
                            error.set(Some(create_failure(&failure, max)));
                        }
                    }
                }),
            });
        })
    };
    html! {
        <form class="gate-step" onsubmit={submit}>
            if let Some(back) = props.on_back.clone() {
                <button type="button" class="link back" onclick={back}>{ format!("‹ {}", t("Back")) }</button>
            }
            <h2>{ t("Create a Family") }</h2>
            if let Some(explanation) = props.explanation.clone() {
                <p class="hint">{ explanation }</p>
            }
            <label for="family-name">{ t("Family name") }</label>
            <input
                id="family-name"
                ref={field}
                type="text"
                placeholder={t("The Smiths")}
                autocomplete="off"
                autocapitalize="words"
                value={(*name).clone()}
                oninput={on_input}
            />
            if let Some(message) = (*error).clone() {
                <p class="error" role="alert">{ message }</p>
            } else {
                <p class="hint">{ t("This names your family chat too. 1–64 characters.") }</p>
            }
            <button type="submit" disabled={*busy || name.trim().is_empty()}>
                { if *busy { t("Creating…") } else { t("Create Family") } }
            </button>
        </form>
    }
}

/// Why a family was not made (ios CreateFamilyView) — including the three
/// refusals of starting ANOTHER one (docs/protocol.md, "Getting another
/// family"), the cap named as `max`.
pub fn create_failure(error: &ApiError, max: i64) -> String {
    if server_trouble(error) {
        return server_trouble_line().to_string();
    }
    if let Some(said) = fc_text::families::refusal(error.code(), max) {
        return said;
    }
    // `subscription_required`: a server that charges for families, and no
    // active subscription — terminal, said as it is (docs/protocol.md,
    // "Paid families").
    if let Some(said) = fc_text::paid::refusal(error.code()) {
        return said.to_string();
    }
    match error.code() {
        // Reachable only on a server that shut its door after the gate was
        // drawn: the gate itself offers no Create there.
        Some("family_registration_disabled") => {
            t("This server doesn't take new families.").to_string()
        }
        Some("already_in_family") => t("You're already in a family.").to_string(),
        Some(_) => match error {
            ApiError::Server { message, .. } if !message.is_empty() => message.clone(),
            _ => t("The server rejected that name.").to_string(),
        },
        None => unreached(error),
    }
}

#[function_component(JoinFamily)]
pub fn join_family(props: &StepProps) -> Html {
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
        let on_done = props.on_done.clone();
        let max = props.max_families;
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
            let on_done = on_done.clone();
            on_action.emit(Action::JoinFamily {
                invite_code: typed,
                done: Callback::from(move |answer: Result<String, ApiError>| {
                    busy.set(false);
                    match answer {
                        // Answered, and `/me` read after it: the app is on
                        // its way to the family or the waiting room — or,
                        // had that read failed, the gate says so above. A
                        // pane that asked for another family goes: the
                        // family is shown, or its request waits in the
                        // switcher.
                        Ok(_) => on_done.emit(()),
                        Err(failure) => {
                            // Already waiting, or already in: this screen
                            // is out of date, and `/me` says where to be.
                            if matches!(
                                failure.code(),
                                Some("join_request_pending") | Some("already_in_family")
                            ) {
                                refresh.emit(Action::RefreshAccount);
                            }
                            // From inside a family, `already_in_family` means
                            // THAT family (docs/protocol.md, "Several families").
                            error.set(Some(if failure.code() == Some("already_in_family") {
                                t("You're already in that family.").to_string()
                            } else {
                                fc_text::families::refusal(failure.code(), max)
                                    .unwrap_or_else(|| join_failure(&failure))
                            }));
                        }
                    }
                }),
            });
        })
    };
    html! {
        <form class="gate-step" onsubmit={submit}>
            if let Some(back) = props.on_back.clone() {
                <button type="button" class="link back" onclick={back}>{ format!("‹ {}", t("Back")) }</button>
            }
            <h2>{ t("Join a Family") }</h2>
            <label for="invite-code">{ t("Invite code") }</label>
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
                <p class="hint">{ t("Any family member can read the code to you; the owner finds it under Family.") }</p>
            }
            <button type="submit" disabled={*busy || code.trim().is_empty()}>
                { if *busy { t("Joining…") } else { t("Join") } }
            </button>
        </form>
    }
}

/// Why a code did not get somebody in (ios JoinFamilyView). A CLOSED family
/// answers `invalid_invite_code`, byte for byte a code that never existed,
/// so the first sentence is right for it too.
pub fn join_failure(error: &ApiError) -> String {
    if server_trouble(error) {
        return server_trouble_line().to_string();
    }
    match error.code() {
        Some("invalid_invite_code") => {
            t("That code doesn't match any family. Check it and try again.").to_string()
        }
        // The second is the one join that can answer it: an account scrubbed
        // while its join was on the way (the action signs it out).
        Some("already_in_family") | Some("user_already_in_family") => {
            t("You're already in a family.").to_string()
        }
        Some("join_request_pending") => t("You already have a pending request.").to_string(),
        Some("family_full") => {
            t("That family is full right now. Ask them to make room, then try the code again.")
                .to_string()
        }
        Some(_) => match error {
            ApiError::Server { message, .. } if !message.is_empty() => message.clone(),
            _ => t("The server rejected that code.").to_string(),
        },
        None => unreached(error),
    }
}

/// A request that got no protocol answer at all — or the server's own
/// trouble, which is not a network to check.
fn unreached(error: &ApiError) -> String {
    match error {
        ApiError::Throttled { .. } => error.detail(),
        _ if server_trouble(error) => server_trouble_line().to_string(),
        _ => t("Can't reach the server. Try again.").to_string(),
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
                <h1 id="pending-title">{ t("Almost There") }</h1>
                <div class="hourglass" aria-hidden="true">{ "⌛" }</div>
                <h2>{ t("Waiting for approval") }</h2>
                <p class="hint">
                    { t("The family owner needs to approve your request. This page updates automatically — or check right now.") }
                </p>
                <button onclick={check}>{ t("Check now") }</button>
                <button class="link signout" onclick={sign_out}>{ t("Log out") }</button>
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
            join_failure(&ApiError::Answered { status: 502 }),
            "The server had a problem. Try again in a moment."
        );
    }

    #[wasm_bindgen_test]
    fn a_create_is_refused_in_the_apps_words() {
        assert_eq!(
            create_failure(&server("family_registration_disabled", "x"), 5),
            "This server doesn't take new families."
        );
        assert_eq!(
            create_failure(
                &server("validation", "family name must be 1-64 characters"),
                5
            ),
            "family name must be 1-64 characters"
        );
        // Starting ANOTHER family (docs/protocol.md, "Getting another
        // family"): the cap named is the server's.
        assert_eq!(
            create_failure(&server("too_many_families", "x"), 3),
            "You've reached the most families one account can be in (3)."
        );
        assert_eq!(
            create_failure(&server("already_own_a_family", "x"), 5),
            "You already own a family, so you can't start another."
        );
        assert_eq!(
            create_failure(&server("member_families_disabled", "x"), 5),
            "This server doesn't let members start new families."
        );
        // A server that charges for families, and no active subscription
        // (docs/protocol.md, "Paid families").
        assert_eq!(
            create_failure(&server("subscription_required", "x"), 5),
            "A new family on this server needs a subscription."
        );
    }

    async fn drawn<C>(props: C::Properties) -> web_sys::Element
    where
        C: BaseComponent,
    {
        let document = web_sys::window().unwrap().document().unwrap();
        let root = document.create_element("div").unwrap();
        document.body().unwrap().append_child(&root).unwrap();
        yew::Renderer::<C>::with_root_and_props(root.clone(), props).render();
        gloo_timers::future::TimeoutFuture::new(20).await;
        root
    }

    fn account(registration: bool, charging: bool, active: Option<bool>) -> Me {
        Me {
            user: crate::model::User {
                id: 7,
                display_name: "Anna".into(),
                ..Default::default()
            },
            family_registration_enabled: registration,
            subscriptions_enabled: charging,
            subscription: active.map(|active| crate::model::Subscription { active }),
            ..Me::default()
        }
    }

    fn doors(root: &web_sys::Element) -> String {
        root.query_selector(".doors")
            .unwrap()
            .and_then(|doors| doors.text_content())
            .unwrap_or_default()
    }

    /// THE GATE ON A SERVER THAT CHARGES (docs/protocol.md, "Paid
    /// families"): without an active subscription the create door is gone
    /// and the sentence stands where a shut door's words stand; "Join a
    /// family" is untouched. A shut door keeps its own words first, and an
    /// active subscription — or a server that does not charge — creates as
    /// before.
    #[wasm_bindgen_test]
    async fn the_gate_offers_create_only_to_an_account_that_can_pay() {
        let gate = |account: Me| GateProps {
            account,
            declined: false,
            notice: None,
            failure: None,
            on_action: Callback::noop(),
            on_sign_out: Callback::noop(),
        };
        const ELSEWHERE: &str =
            "On this server, a new family is started in the Family Connect app on iPhone, iPad or Mac.";

        for unpaid in [None, Some(false)] {
            let root = drawn::<FamilyGate>(gate(account(true, true, unpaid))).await;
            let said = root
                .query_selector(".closed-server.start-elsewhere")
                .unwrap()
                .expect("the sentence, where a shut door's words go");
            assert_eq!(said.text_content().as_deref(), Some(ELSEWHERE));
            assert!(!doors(&root).contains("Create a family"), "{unpaid:?}");
            assert!(doors(&root).contains("Join a family"), "joining stays");
            root.remove();
        }

        for (registration, charging, active) in [(true, true, Some(true)), (true, false, None)] {
            let root = drawn::<FamilyGate>(gate(account(registration, charging, active))).await;
            assert!(
                doors(&root).contains("Create a family"),
                "{charging} {active:?}"
            );
            assert!(doors(&root).contains("Join a family"));
            assert!(!root.text_content().unwrap_or_default().contains(ELSEWHERE));
            root.remove();
        }

        // A server closed to new families says so in its own words, paid or not.
        let root = drawn::<FamilyGate>(gate(account(false, true, None))).await;
        let said = root.text_content().unwrap_or_default();
        assert!(
            said.contains("This server doesn't take new families."),
            "{said}"
        );
        assert!(!said.contains(ELSEWHERE), "the shut door has priority");
        assert!(!doors(&root).contains("Create a family"));
        assert!(doors(&root).contains("Join a family"));
        root.remove();
    }

    /// "New Family…" stays on the menu; without an active subscription on
    /// a server that charges, it opens on the sentence instead of the form.
    #[wasm_bindgen_test]
    async fn new_family_says_where_to_start_one_without_a_subscription() {
        let pane = |may_start: bool, join: bool| AnotherFamilyProps {
            join,
            max_families: 5,
            may_start,
            on_action: Callback::noop(),
            on_close: Callback::noop(),
        };
        let root = drawn::<AnotherFamilyPane>(pane(false, false)).await;
        assert!(
            root.query_selector("#family-name").unwrap().is_none(),
            "no form"
        );
        assert!(root
            .query_selector(".start-elsewhere")
            .unwrap()
            .and_then(|said| said.text_content())
            .is_some_and(|said| said.contains("Family Connect app on iPhone, iPad or Mac")));
        root.remove();

        let root = drawn::<AnotherFamilyPane>(pane(true, false)).await;
        assert!(
            root.query_selector("#family-name").unwrap().is_some(),
            "the form"
        );
        assert!(root.query_selector(".start-elsewhere").unwrap().is_none());
        root.remove();

        // Joining another family is never about paying.
        let root = drawn::<AnotherFamilyPane>(pane(false, true)).await;
        assert!(root.query_selector("#invite-code").unwrap().is_some());
        assert!(root.query_selector(".start-elsewhere").unwrap().is_none());
        root.remove();
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
