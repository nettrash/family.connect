//! The family: who is in it, and — for its owner — the door and the house
//! rules (ios MacFamilyView, with the iPhone's FamilyManageView where the
//! Mac's is the one that disagrees with the protocol).
//!
//! What the owner sees: the invite code (copy, rotate), the join requests
//! waiting, the report inbox, the join policy, the member limit, and the
//! assistant's switches. What everybody sees: the members, with a way to
//! message, report or block each — and, for the owner, a birthday, a
//! password reset and a removal on each, the removal asked about first.
//!
//! Where this parts from the Mac on purpose: a removal is confirmed (the
//! iPhone asks, the Mac does not); a member the reader has blocked is not
//! offered "Message", which would only be refused; the member limit counts
//! against the LOWER of the owner's cap and the server's ceiling, which is
//! what the door does; turning the limit off does not spring back on while
//! its write waits; and a report says what the reported message carried,
//! which for a photo sent without words is all there is to say.
//!
//! Not built, here as on the Mac: the jump from a report to the message it
//! names, which the protocol offers while `message_id` survives.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use fc_text::account::{self, cap_footer, cap_state};
use fc_text::i18n::{t, t1, t2, tn};
use gloo_timers::callback::Timeout;
use wasm_bindgen::JsCast;
use web_sys::{HtmlInputElement, HtmlSelectElement};
use yew::prelude::*;

use crate::actions::{Action, Done};
use crate::api::{ApiError, FamilyPatch};
use crate::model::{Assistant, Family, JoinRequest, Me, Member, Report, ReportedAttachment};
use crate::time;
use crate::views::avatar::Avatar;
use crate::views::birthday::BirthdayDialog;
use crate::views::dialog::{generic_failure, Confirm};
use crate::views::password::ResetPasswordDialog;
use crate::views::report::{ReportDialog, ReportTarget, REASONS};

/// How long the member limit waits after the last step before it is sent:
/// a run of clicks is one write, of the value it ended on.
pub const CAP_DEBOUNCE_MS: u32 = 600;

#[derive(Properties, PartialEq)]
pub struct FamilyProps {
    pub account: Me,
    pub family: Family,
    pub members: Vec<Member>,
    pub assistant: Option<Assistant>,
    pub blocked: HashSet<i64>,
    pub join_requests: Vec<JoinRequest>,
    pub reports: Vec<Report>,
    pub support_contact: Option<String>,
    pub on_action: Callback<Action>,
    pub on_close: Callback<()>,
}

/// What one of the member rows' tools is doing.
#[derive(Clone, PartialEq)]
enum Tool {
    Nothing,
    Menu(i64),
    Safety(i64),
    Birthday(i64),
    Password(i64),
    Remove(i64),
    Report(i64),
    Rotate,
}

#[function_component(FamilyPane)]
pub fn family_pane(props: &FamilyProps) -> Html {
    let tool = use_state(|| Tool::Nothing);
    let busy = use_state(|| false);
    let error = use_state(|| Option::<String>::None);
    let owner = props.account.is_owner();
    let me = props.account.user.id;
    let family = &props.family;
    // Live members, as the roster has them, sorted by name as the Mac sorts.
    let mut members: Vec<Member> = props
        .members
        .iter()
        .filter(|member| !member.deleted)
        .cloned()
        .collect();
    members.sort_by_key(|member| member.display_name.to_lowercase());
    let count = members.len() as i64;
    let names: HashMap<i64, String> = members
        .iter()
        .map(|member| (member.id, member.display_name.clone()))
        .collect();

    let set_tool = |to: Tool| {
        let tool = tool.clone();
        Callback::from(move |_: MouseEvent| tool.set(to.clone()))
    };
    let close_tool = {
        let tool = tool.clone();
        Callback::from(move |_: ()| tool.set(Tool::Nothing))
    };
    // A change that reports back here: busy while it is out, and what went
    // wrong said on the one line the pane keeps for it.
    let doing = {
        let busy = busy.clone();
        let error = error.clone();
        move |wording: fn(&ApiError) -> String| -> Done {
            busy.set(true);
            error.set(None);
            let busy = busy.clone();
            let error = error.clone();
            Callback::from(move |failure: Option<ApiError>| {
                busy.set(false);
                error.set(failure.map(|failure| wording(&failure)));
            })
        }
    };

    let rotate = {
        let tool = tool.clone();
        let on_action = props.on_action.clone();
        let doing = doing.clone();
        Callback::from(move |_: ()| {
            tool.set(Tool::Nothing);
            on_action.emit(Action::RotateInviteCode {
                done: doing(generic_failure),
            });
        })
    };
    // The pane itself, for finding a menu's items and triggers in it.
    let pane = use_node_ref();
    // The control a member's menu was opened from: where the focus goes
    // back to once the menu, or the dialog one of its items opened, is gone
    // — the item itself went with the menu, so a dialog has nothing of its
    // own to hand the focus back to.
    let trigger = use_mut_ref(|| Option::<String>::None);
    {
        let pane = pane.clone();
        let trigger = trigger.clone();
        use_effect_with((*tool).clone(), move |open| {
            let root = pane.cast::<web_sys::Element>();
            let focus = |css: &str| {
                if let Some(element) = root
                    .as_ref()
                    .and_then(|root| root.query_selector(css).ok().flatten())
                    .and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok())
                {
                    let _ = element.focus();
                }
            };
            match open {
                // A menu takes the focus to its first item, so the keyboard
                // can use it.
                Tool::Menu(_) | Tool::Safety(_) => focus(".menu [role=menuitem]"),
                Tool::Nothing => {
                    if let Some(css) = trigger.borrow_mut().take() {
                        focus(&css);
                    }
                }
                _ => {}
            }
        });
    }
    let open_menu = |to: Tool, css: String| {
        let tool = tool.clone();
        let trigger = trigger.clone();
        Callback::from(move |_: MouseEvent| {
            if *tool == to {
                tool.set(Tool::Nothing);
            } else {
                *trigger.borrow_mut() = Some(css.clone());
                tool.set(to.clone());
            }
        })
    };
    let menu_keys = {
        let tool = tool.clone();
        let trigger = trigger.clone();
        Callback::from(move |event: KeyboardEvent| {
            let Some(menu) = event
                .target()
                .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
                .and_then(|element| element.closest(".menu").ok().flatten())
            else {
                return;
            };
            let items = menu.query_selector_all("[role=menuitem]").ok();
            let items: Vec<web_sys::HtmlElement> = items
                .map(|list| {
                    (0..list.length())
                        .filter_map(|index| list.item(index))
                        .filter_map(|node| node.dyn_into().ok())
                        .collect()
                })
                .unwrap_or_default();
            let active = web_sys::window()
                .and_then(|window| window.document())
                .and_then(|document| document.active_element());
            let at = items.iter().position(|item| {
                active
                    .as_ref()
                    .is_some_and(|active| active == item.unchecked_ref::<web_sys::Element>())
            });
            let step = |by: isize| {
                if items.is_empty() {
                    return;
                }
                let count = items.len() as isize;
                let next = (at.map_or(-1, |at| at as isize) + by).rem_euclid(count);
                let _ = items[next as usize].focus();
            };
            match event.key().as_str() {
                // Back to where the menu came from.
                "Escape" => {
                    event.prevent_default();
                    tool.set(Tool::Nothing);
                }
                // On to wherever Tab goes — the menu closes behind it, and
                // the focus is not pulled back.
                "Tab" => {
                    trigger.borrow_mut().take();
                    tool.set(Tool::Nothing);
                }
                "ArrowDown" => {
                    event.prevent_default();
                    step(1);
                }
                "ArrowUp" => {
                    event.prevent_default();
                    step(-1);
                }
                _ => {}
            }
        })
    };

    let header = html! {
        <div class="identity">
            <Avatar title={family.name.clone()} family={true} size={44} />
            <div class="identity-text">
                <strong>{ family.name.clone() }</strong>
                <span class="muted">
                    { tn("%lld members", count) }
                </span>
            </div>
        </div>
    };

    let owner_sections = owner.then(|| {
        let code = family.invite_code.clone();
        let copy = {
            let code = code.clone();
            Callback::from(move |_: MouseEvent| {
                if let (Some(code), Some(window)) = (code.clone(), web_sys::window()) {
                    let _ = window.navigator().clipboard().write_text(&code);
                }
            })
        };
        html! {
            <>
                <section class="group" aria-labelledby="family-invite">
                    <h3 id="family-invite">{ t("Invite code") }</h3>
                    <div class="setting-row">
                        <span class="code invite-code">{ code.clone().unwrap_or_else(|| "…".to_string()) }</span>
                        <span class="row-actions">
                            <button class="link" disabled={code.is_none()} onclick={copy}>{ t("Copy") }</button>
                            <button class="link" disabled={*busy} onclick={set_tool(Tool::Rotate)}>{ t("Rotate") }</button>
                        </span>
                    </div>
                    <p class="footnote">{ t("Rotating invalidates the current code immediately.") }</p>
                </section>
                <JoinRequests
                    requests={props.join_requests.clone()}
                    on_action={props.on_action.clone()}
                />
                <Reports reports={props.reports.clone()} on_action={props.on_action.clone()} />
                <JoinPolicy family={family.clone()} on_action={props.on_action.clone()} />
                if let Some(ceiling) = props.account.max_family_members {
                    <MemberLimit
                        family={family.clone()}
                        {count}
                        {ceiling}
                        on_action={props.on_action.clone()}
                    />
                }
                // The assistant's switches, where there is an assistant for
                // them to be about.
                if let Some(assistant) = props.assistant.clone() {
                    <AssistantSettings
                        family={family.clone()}
                        {assistant}
                        greetings={props.account.greetings_enabled}
                        on_action={props.on_action.clone()}
                    />
                }
            </>
        }
    });

    let member_rows = members.iter().map(|member| {
        let id = member.id;
        let is_me = id == me;
        let blocked = props.blocked.contains(&id);
        let message = props
            .on_action
            .reform(move |_: MouseEvent| Action::OpenDirect { user_id: id });
        let block = {
            let on_action = props.on_action.clone();
            let tool = tool.clone();
            Callback::from(move |_: MouseEvent| {
                tool.set(Tool::Nothing);
                on_action.emit(Action::Block {
                    user_id: id,
                    blocked: !blocked,
                });
            })
        };
        let menu_open = *tool == Tool::Menu(id);
        let safety_open = *tool == Tool::Safety(id);
        let removable = owner && !is_me && !member.is_owner();
        html! {
            <li class="member-row" key={id.to_string()}>
                <Avatar title={member.display_name.clone()} user_id={Some(id)} version={member.avatar_version} size={28} />
                <div class="identity-text">
                    <span>
                        <strong>{ member.display_name.clone() }</strong>
                        if member.is_owner() {
                            <span class="capsule">{ t("Owner") }</span>
                        }
                    </span>
                    <span class="muted">{ format!("@{}", member.username) }</span>
                    if let Some(birthday) = member.birthday {
                        <span class="muted birthday">{ format!("🎂 {}", time::birthday(birthday.month, birthday.day)) }</span>
                    }
                </div>
                <span class="row-actions">
                    if !is_me && !blocked {
                        <button class="link" aria-label={t1("Message %@", &member.display_name)} onclick={message}>{ t("Message") }</button>
                    }
                    if !is_me {
                        <span class="menu-anchor">
                            <button class="link" data-trigger={format!("safety-{id}")}
                                aria-label={t1("Safety for %@", &member.display_name)}
                                aria-haspopup="menu" aria-expanded={if safety_open { "true" } else { "false" }}
                                onclick={open_menu(Tool::Safety(id), format!("[data-trigger=safety-{id}]"))}>
                                { t("Safety") }
                            </button>
                            if safety_open {
                                <div class="menu-backdrop" onclick={set_tool(Tool::Nothing)} aria-hidden="true"></div>
                                <div class="menu" role="menu" onkeydown={menu_keys.clone()}>
                                    <button role="menuitem" onclick={set_tool(Tool::Report(id))}>{ t("Report…") }</button>
                                    <button role="menuitem" class={classes!((!blocked).then_some("danger"))} onclick={block}>
                                        { if blocked { t("Unblock") } else { t("Block") } }
                                    </button>
                                </div>
                            }
                        </span>
                    }
                    if owner {
                        <span class="menu-anchor">
                            <button class="link" data-trigger={format!("more-{id}")}
                                aria-label={t1("More for %@", &member.display_name)} aria-haspopup="menu"
                                aria-expanded={if menu_open { "true" } else { "false" }}
                                onclick={open_menu(Tool::Menu(id), format!("[data-trigger=more-{id}]"))}>
                                { "⋯" }
                            </button>
                            if menu_open {
                                <div class="menu-backdrop" onclick={set_tool(Tool::Nothing)} aria-hidden="true"></div>
                                <div class="menu" role="menu" onkeydown={menu_keys.clone()}>
                                    <button role="menuitem" onclick={set_tool(Tool::Birthday(id))}>{ t("Birthday…") }</button>
                                    if removable {
                                        <button role="menuitem" onclick={set_tool(Tool::Password(id))}>{ t("Reset Password…") }</button>
                                        <button role="menuitem" class="danger" onclick={set_tool(Tool::Remove(id))}>{ t("Remove from Family") }</button>
                                    }
                                </div>
                            }
                        </span>
                    }
                </span>
            </li>
        }
    });

    let named = |id: i64| names.get(&id).cloned().unwrap_or_default();
    let tool_dialog = match &*tool {
        Tool::Rotate => html! {
            <Confirm
                title={t("Rotate the invite code?")}
                message={t("Rotating invalidates the current code immediately.")}
                confirm={t("Rotate Code")}
                on_confirm={rotate}
                on_cancel={close_tool.clone()}
            />
        },
        Tool::Birthday(id) => html! {
            <BirthdayDialog
                user_id={Some(*id)}
                name={Some(named(*id))}
                current={members.iter().find(|member| member.id == *id).and_then(|member| member.birthday)}
                on_action={props.on_action.clone()}
                on_close={close_tool.clone()}
            />
        },
        Tool::Password(id) => html! {
            <ResetPasswordDialog
                user_id={*id}
                name={named(*id)}
                on_action={props.on_action.clone()}
                on_close={close_tool.clone()}
            />
        },
        Tool::Remove(id) => {
            let remove = {
                let tool = tool.clone();
                let on_action = props.on_action.clone();
                let doing = doing.clone();
                let id = *id;
                Callback::from(move |_: ()| {
                    tool.set(Tool::Nothing);
                    on_action.emit(Action::RemoveMember {
                        user_id: id,
                        done: doing(generic_failure),
                    });
                })
            };
            html! {
                <Confirm
                    title={t1("Remove %@ from the family?", &named(*id))}
                    confirm={t("Remove")}
                    on_confirm={remove}
                    on_cancel={close_tool.clone()}
                />
            }
        }
        Tool::Report(id) => {
            let id = *id;
            let submit = {
                let tool = tool.clone();
                let on_action = props.on_action.clone();
                Callback::from(move |reason: String| {
                    tool.set(Tool::Nothing);
                    on_action.emit(Action::Report {
                        user_id: id,
                        message_id: None,
                        reason,
                    });
                })
            };
            html! {
                <ReportDialog
                    target={ReportTarget { user_id: id, name: named(id), message_id: None }}
                    support_contact={props.support_contact.clone()}
                    on_submit={submit}
                    on_cancel={close_tool.clone()}
                />
            }
        }
        _ => Html::default(),
    };

    let close = props.on_close.reform(|_: MouseEvent| ());
    html! {
        <section class="pane family-pane" aria-labelledby="family-title" ref={pane}>
            <header class="pane-head">
                <h2 id="family-title">{ t("Family") }</h2>
                <button class="link" onclick={close}>{ t("Done") }</button>
            </header>
            <div class="pane-body">
                { header }
                if let Some(message) = (*error).clone() {
                    <p class="error" role="alert">{ message }</p>
                }
                { owner_sections.unwrap_or_default() }
                <section class="group" aria-labelledby="family-members">
                    <h3 id="family-members">{ t("Members") }</h3>
                    <ul class="members">{ for member_rows }</ul>
                </section>
            </div>
            { tool_dialog }
        </section>
    }
}

#[derive(Properties, PartialEq)]
struct RequestsProps {
    requests: Vec<JoinRequest>,
    on_action: Callback<Action>,
}

/// The requests waiting on the owner — drawn with initials only: the server
/// will not show a stranger's picture to anybody, and a refusal cached now
/// would outlive the approval (ios FamilyManageView).
#[function_component(JoinRequests)]
fn join_requests(props: &RequestsProps) -> Html {
    let deciding = use_state(|| Option::<i64>::None);
    let error = use_state(|| Option::<String>::None);
    if props.requests.is_empty() {
        return Html::default();
    }
    let decide = |id: i64, approve: bool| {
        let deciding = deciding.clone();
        let error = error.clone();
        let on_action = props.on_action.clone();
        Callback::from(move |_: MouseEvent| {
            if deciding.is_some() {
                return;
            }
            deciding.set(Some(id));
            error.set(None);
            let deciding = deciding.clone();
            let error = error.clone();
            on_action.emit(Action::DecideJoinRequest {
                id,
                approve,
                done: Callback::from(move |failure: Option<ApiError>| {
                    deciding.set(None);
                    error.set(failure.map(|failure| request_failure(&failure)));
                }),
            });
        })
    };
    html! {
        <section class="group" aria-labelledby="family-requests">
            <h3 id="family-requests">{ t("Join requests") }</h3>
            <ul class="members">
                { for props.requests.iter().map(|request| {
                    let busy = deciding.is_some();
                    html! {
                        <li class="member-row" key={request.id.to_string()}>
                            <Avatar title={request.user.display_name.clone()} size={28} />
                            <div class="identity-text">
                                <strong>{ request.user.display_name.clone() }</strong>
                                <span class="muted">{ format!("@{}", request.user.username) }</span>
                            </div>
                            <span class="row-actions">
                                <button class="link" disabled={busy} onclick={decide(request.id, true)}>{ t("Approve") }</button>
                                <button class="link danger" disabled={busy} onclick={decide(request.id, false)}>{ t("Decline") }</button>
                            </span>
                        </li>
                    }
                }) }
            </ul>
            if let Some(message) = (*error).clone() {
                <p class="error" role="alert">{ message }</p>
            }
        </section>
    }
}

/// Why a request could not be decided. A full family leaves the request
/// WAITING: full is a condition, not an answer.
pub fn request_failure(error: &ApiError) -> String {
    match error.code() {
        Some("family_full") => t("The family is full. Raise the member limit or wait for somebody to leave — the request is still waiting.").to_string(),
        _ => generic_failure(error),
    }
}

#[derive(Properties, PartialEq)]
struct ReportsProps {
    reports: Vec<Report>,
    on_action: Callback<Action>,
}

#[function_component(Reports)]
fn reports(props: &ReportsProps) -> Html {
    let resolving = use_state(|| Option::<i64>::None);
    let error = use_state(|| Option::<String>::None);
    let resolve = |id: i64| {
        let resolving = resolving.clone();
        let error = error.clone();
        let on_action = props.on_action.clone();
        Callback::from(move |_: MouseEvent| {
            if resolving.is_some() {
                return;
            }
            resolving.set(Some(id));
            error.set(None);
            let resolving = resolving.clone();
            let error = error.clone();
            on_action.emit(Action::ResolveReport {
                id,
                done: Callback::from(move |failure: Option<ApiError>| {
                    resolving.set(None);
                    if failure.is_some() {
                        error.set(Some(
                            t("Couldn't mark that as handled. Try again.").to_string(),
                        ));
                    }
                }),
            });
        })
    };
    html! {
        <section class="group" aria-labelledby="family-reports">
            <h3 id="family-reports">{ t("Reports") }</h3>
            if props.reports.is_empty() {
                <p class="footnote">{ t("Members can report a message or a person to you.") }</p>
            }
            // Keyed rows in a fragment of their own: beside the heading and
            // the two `if`s, the keys would otherwise count for nothing.
            <>{ for props.reports.iter().map(|report| {
                let carried = carried(&report.message_attachments);
                html! {
                    <article class="report-row" key={report.id.to_string()}>
                        <strong>{ reason_label(&report.reason) }</strong>
                        <span class="muted">
                            { t2("%@ reported %@", &report.reporter.display_name, &report.reported.display_name) }
                        </span>
                        if let Some(excerpt) = report.message_excerpt.clone().filter(|excerpt| !excerpt.is_empty()) {
                            <p class="excerpt verbatim">{ excerpt }</p>
                        }
                        if let Some(carried) = carried {
                            <span class="muted">{ carried }</span>
                        }
                        <span class="row-actions">
                            <button class="link" disabled={resolving.is_some()} onclick={resolve(report.id)}>
                                { t("Mark as handled") }
                            </button>
                        </span>
                    </article>
                }
            }) }</>
            if let Some(message) = (*error).clone() {
                <p class="error" role="alert">{ message }</p>
            }
        </section>
    }
}

/// A report's reason in words — an unknown one is "Something else".
pub fn reason_label(reason: &str) -> &'static str {
    REASONS
        .iter()
        .find(|(code, _)| *code == reason)
        .map_or(t("Something else"), |(_, label)| t(label))
}

/// What a reported message carried, as a chat-list preview says it.
pub fn carried(attachments: &[ReportedAttachment]) -> Option<String> {
    let first = attachments.first()?;
    let count = attachments.len();
    Some(match first.kind.as_str() {
        "photo" if count > 1 => tn("%lld Photos", count as i64),
        "photo" => t("Photo").to_string(),
        "video" => t("Video").to_string(),
        "audio" => t("Voice message").to_string(),
        "location" => t("Location").to_string(),
        _ => first.name.clone().unwrap_or_else(|| t("File").to_string()),
    })
}

#[derive(Properties, PartialEq)]
struct PolicyProps {
    family: Family,
    on_action: Callback<Action>,
}

/// The three the protocol allows, with the key each is said by (a `const`
/// cannot look a translation up).
pub const POLICIES: [(&str, &str); 3] = [
    ("open", "Join immediately"),
    ("approval", "Need approval"),
    ("closed", "Nobody"),
];

/// The caption under the join policy, for the policy in force.
pub fn policy_caption(policy: &str) -> &'static str {
    match policy {
        "approval" => t("With approval, join requests wait here until you approve them."),
        "closed" => t("The invite code stops working — nobody new can join. Requests already waiting are unaffected, and you can still approve them."),
        _ => t("Anyone with the invite code joins straight away."),
    }
}

#[function_component(JoinPolicy)]
fn join_policy(props: &PolicyProps) -> Html {
    let busy = use_state(|| false);
    let error = use_state(|| Option::<String>::None);
    // What was chosen, drawn while it is on its way: the family still holds
    // the old policy until the answer, and a radio drawn from it would jump
    // back for the whole round trip.
    let asked = use_state(|| Option::<String>::None);
    let current = (*asked)
        .clone()
        .unwrap_or_else(|| props.family.join_policy.clone());
    let choose = |policy: &'static str| {
        let busy = busy.clone();
        let error = error.clone();
        let asked = asked.clone();
        let on_action = props.on_action.clone();
        let current = current.clone();
        Callback::from(move |_: Event| {
            if *busy || current == policy {
                return;
            }
            busy.set(true);
            error.set(None);
            asked.set(Some(policy.to_string()));
            let busy = busy.clone();
            let error = error.clone();
            let asked = asked.clone();
            on_action.emit(Action::ChangeFamily {
                patch: FamilyPatch {
                    join_policy: Some(policy.to_string()),
                    ..FamilyPatch::default()
                },
                done: Callback::from(move |failure: Option<ApiError>| {
                    busy.set(false);
                    asked.set(None);
                    if failure.is_some() {
                        error.set(Some(
                            t("Couldn't change the policy. Try again.").to_string(),
                        ));
                    }
                }),
            });
        })
    };
    html! {
        <section class="group" aria-labelledby="family-policy">
            <h3 id="family-policy">{ t("Join policy") }</h3>
            <fieldset class="segmented">
                <legend class="visually-hidden">{ t("New members") }</legend>
                { for POLICIES.iter().map(|(code, label)| html! {
                    <label class={classes!((current == *code).then_some("is-chosen"))}>
                        <input type="radio" name="join-policy" value={*code} checked={current == *code} onchange={choose(code)} />
                        { t(label) }
                    </label>
                }) }
            </fieldset>
            <p class="footnote">{ policy_caption(&current) }</p>
            if let Some(message) = (*error).clone() {
                <p class="error" role="alert">{ message }</p>
            }
        </section>
    }
}

#[derive(Properties, PartialEq)]
struct LimitProps {
    family: Family,
    count: i64,
    ceiling: i64,
    on_action: Callback<Action>,
}

/// What the member limit shows while its write waits: nothing of its own,
/// "off" (a null on its way — the toggle must not spring back on for the
/// 600 ms it waits), or a number.
#[derive(Clone, Copy, PartialEq)]
enum CapDraft {
    Untouched,
    Off,
    Value(i64),
}

struct CapCell {
    draft: CapDraft,
    /// Bumped by every change, so only the answer to the LAST write clears
    /// what is drawn.
    generation: u64,
    waiting: Option<Timeout>,
}

#[function_component(MemberLimit)]
fn member_limit(props: &LimitProps) -> Html {
    let cell = use_mut_ref(|| CapCell {
        draft: CapDraft::Untouched,
        generation: 0,
        waiting: None,
    });
    let redraw = use_force_update();
    let error = use_state(|| Option::<String>::None);
    let ceiling = props.ceiling;
    let drawn = match cell.borrow().draft {
        CapDraft::Untouched => props.family.max_members,
        CapDraft::Off => None,
        CapDraft::Value(value) => Some(value),
    };
    let change = {
        let cell = cell.clone();
        let redraw = redraw.clone();
        let error = error.clone();
        let on_action = props.on_action.clone();
        Rc::new(move |to: Option<i64>| {
            let generation = {
                let mut held = cell.borrow_mut();
                held.draft = to.map_or(CapDraft::Off, CapDraft::Value);
                held.generation += 1;
                held.generation
            };
            error.set(None);
            let send = {
                let cell = cell.clone();
                let redraw = redraw.clone();
                let error = error.clone();
                let on_action = on_action.clone();
                move || {
                    let cell = cell.clone();
                    let redraw = redraw.clone();
                    let error = error.clone();
                    on_action.emit(Action::ChangeFamily {
                        patch: FamilyPatch {
                            max_members: Some(to),
                            ..FamilyPatch::default()
                        },
                        done: Callback::from(move |failure: Option<ApiError>| {
                            let mut held = cell.borrow_mut();
                            if held.generation == generation {
                                // The answer is the truth now; what was
                                // drawn meanwhile is not needed.
                                held.draft = CapDraft::Untouched;
                                drop(held);
                                if failure.is_some() {
                                    error.set(Some(
                                        t("Couldn't change the member limit. Try again.")
                                            .to_string(),
                                    ));
                                }
                                redraw.force_update();
                            }
                        }),
                    });
                }
            };
            cell.borrow_mut().waiting = Some(Timeout::new(CAP_DEBOUNCE_MS, send));
            redraw.force_update();
        })
    };
    let toggle = {
        let change = change.clone();
        let count = props.count;
        Callback::from(move |event: Event| {
            let input: HtmlInputElement = event.target_unchecked_into();
            // Turned on, it proposes the family frozen where it stands —
            // what reaching for a limit almost always means.
            change(input.checked().then(|| account::seed_cap(count, ceiling)));
        })
    };
    // Stepped from what is drawn NOW — the cell, never this render's copy:
    // two clicks before the next render are two steps, not the same one
    // twice.
    let step = |by: i64| {
        let change = change.clone();
        let cell = cell.clone();
        let held = props.family.max_members;
        Callback::from(move |_: MouseEvent| {
            let now = match cell.borrow().draft {
                CapDraft::Untouched => held,
                CapDraft::Off => None,
                CapDraft::Value(value) => Some(value),
            };
            if let Some(value) = now {
                change(Some(account::clamp_cap(value + by, ceiling)));
            }
        })
    };
    let typed = {
        let change = change.clone();
        Callback::from(move |event: Event| {
            let input: HtmlInputElement = event.target_unchecked_into();
            if let Ok(value) = input.value().trim().parse::<i64>() {
                change(Some(account::clamp_cap(value, ceiling)));
            }
        })
    };
    let footer = cap_footer(cap_state(drawn, props.count, ceiling));
    html! {
        <section class="group" aria-labelledby="family-limit">
            <h3 id="family-limit">{ t("Member limit") }</h3>
            <label class="setting-row toggle">
                <span>{ t("Limit members") }</span>
                <input type="checkbox" role="switch" checked={drawn.is_some()} onchange={toggle} />
            </label>
            if let Some(value) = drawn {
                <div class="setting-row">
                    <label for="most-members">{ t("Most members") }</label>
                    <span class="stepper">
                        <button class="link" aria-label={t("Fewer")} disabled={value <= 1} onclick={step(-1)}>{ "−" }</button>
                        <input id="most-members" type="number" min="1" max={ceiling.to_string()} value={value.to_string()} onchange={typed} />
                        <button class="link" aria-label={t("More")} disabled={value >= ceiling} onclick={step(1)}>{ "+" }</button>
                    </span>
                </div>
            }
            <p class="footnote">{ footer }</p>
            if let Some(message) = (*error).clone() {
                <p class="error" role="alert">{ message }</p>
            }
        </section>
    }
}

#[derive(Properties, PartialEq)]
struct AssistantProps {
    family: Family,
    assistant: Assistant,
    /// Whether this server posts the daily greeting at all.
    greetings: bool,
    on_action: Callback<Action>,
}

/// The family as a change on its way will leave it — for drawing, while the
/// answer is out. Turning vision off takes the two switches that ride on it
/// with it, as the server does in the same write.
pub fn overlay(family: &Family, patch: Option<&FamilyPatch>) -> Family {
    let mut shown = family.clone();
    let Some(patch) = patch else {
        return shown;
    };
    if let Some(language) = &patch.language {
        shown.language = language.clone();
    }
    if let Some(on) = patch.ai_history {
        shown.ai_history = on;
    }
    if let Some(on) = patch.ai_vision {
        shown.ai_vision = on;
        if !on {
            shown.ai_history_photos = false;
            shown.ai_faces = false;
        }
    }
    if let Some(on) = patch.ai_history_photos {
        shown.ai_history_photos = on;
    }
    if let Some(on) = patch.ai_faces {
        shown.ai_faces = on;
    }
    if let Some(on) = patch.ai_greeting {
        shown.ai_greeting = on;
    }
    shown
}

/// The assistant's switches (ios FamilyAssistantSettings): the language it
/// answers in, and how much of the family it may be shown. Each write sends
/// the one key that changed.
#[function_component(AssistantSettings)]
fn assistant_settings(props: &AssistantProps) -> Html {
    let busy = use_state(|| false);
    let error = use_state(|| Option::<String>::None);
    // The change on its way, drawn over the family until it answers.
    let pending = use_state(|| Option::<FamilyPatch>::None);
    let shown = overlay(&props.family, (*pending).as_ref());
    let family = &shown;
    let token = props
        .assistant
        .mention
        .clone()
        .unwrap_or_else(|| "@ai".to_string());
    let vision = props.assistant.vision;
    let limit = fc_text::assistant_pictures::MAX_PER_QUESTION;
    let send = {
        let busy = busy.clone();
        let error = error.clone();
        let pending = pending.clone();
        let on_action = props.on_action.clone();
        Rc::new(move |patch: FamilyPatch| {
            if *busy {
                return;
            }
            busy.set(true);
            error.set(None);
            pending.set(Some(patch.clone()));
            let busy = busy.clone();
            let error = error.clone();
            let pending = pending.clone();
            on_action.emit(Action::ChangeFamily {
                patch,
                done: Callback::from(move |failure: Option<ApiError>| {
                    busy.set(false);
                    pending.set(None);
                    error.set(failure.map(|failure| match failure.code() {
                        Some("not_family_owner") => {
                            t("Only the family owner can change this.").to_string()
                        }
                        _ => t("Couldn't save that. Try again.").to_string(),
                    }));
                }),
            });
        })
    };
    let language = {
        let send = send.clone();
        Callback::from(move |event: Event| {
            let select: HtmlSelectElement = event.target_unchecked_into();
            let value = select.value();
            send(FamilyPatch {
                language: Some((!value.is_empty()).then_some(value)),
                ..FamilyPatch::default()
            });
        })
    };
    let switch = |make: fn(bool) -> FamilyPatch| {
        let send = send.clone();
        Callback::from(move |event: Event| {
            let input: HtmlInputElement = event.target_unchecked_into();
            send(make(input.checked()));
        })
    };
    let current_language = family.language.clone().unwrap_or_default();
    // The two switches that ride on vision: offered only with it, and
    // inert-but-explained while the history they draw from is not sent.
    let dependent = |extra_history: &str| -> Option<String> {
        if !vision {
            Some(
                t("Not available here: the assistant on this server can't look at pictures.")
                    .to_string(),
            )
        } else if !family.ai_vision {
            Some(
                t("Turn on Can be shown photos first — the server refuses this while that is off.")
                    .to_string(),
            )
        } else if !family.ai_history {
            Some(extra_history.to_string())
        } else {
            None
        }
    };
    let photos_note = dependent(t(
        "While Sees recent history is off this does nothing: the chat's history isn't sent, so no photo from it is either.",
    ));
    let faces_note = dependent(t(
        "While Sees recent history is off this does nothing: no names are sent, so no faces are either.",
    ));
    let pictures_on = vision && family.ai_vision;
    html! {
        <>
            <section class="group" aria-labelledby="assistant-language">
                <h3 id="assistant-language">{ t("Assistant language") }</h3>
                <label class="setting-row">
                    <span>{ t("Answers in") }</span>
                    <select onchange={language}>
                        <option value="" selected={current_language.is_empty()}>{ t("Not set") }</option>
                        { for account::LANGUAGES.iter().map(|(code, name)| html! {
                            <option value={*code} selected={current_language.eq_ignore_ascii_case(code)}>{ *name }</option>
                        }) }
                    </select>
                </label>
                <p class="footnote">{ t1("The language %@ answers in when it is asked in the family chat. It is not this app's language — that follows the device. With none chosen, it answers in the language of whoever asked.", &token) }</p>
                <label class="setting-row toggle">
                    <span>{ t("Sees recent history") }</span>
                    <input type="checkbox" role="switch" checked={family.ai_history}
                        onchange={switch(|on| FamilyPatch { ai_history: Some(on), ..FamilyPatch::default() })} />
                </label>
                <p class="footnote">{ t1("With this on, mentioning %@ in the family chat sends the last month of that chat to the assistant, so it can answer questions about what was said earlier. With it off, only the message that mentions it is sent.", &token) }</p>
            </section>
            <section class="group" aria-labelledby="assistant-pictures">
                <h3 id="assistant-pictures">{ t("Pictures") }</h3>
                if vision {
                    <label class="setting-row toggle">
                        <span>{ t("Can be shown photos") }</span>
                        <input type="checkbox" role="switch" checked={family.ai_vision}
                            onchange={switch(|on| FamilyPatch { ai_vision: Some(on), ..FamilyPatch::default() })} />
                    </label>
                    <p class="footnote">{ t2("With this on, a photo is sent to the model your server is set up to use when a member attaches it to a question in their own chat with the assistant, attaches it to an %@ message in the family chat, or replies to a photo with %@ — never a photo the assistant was not pointed at, never from an earlier message unless Recent photos is on, and never a video, file or place. With it off, no photo is ever sent.", &token, &token) }</p>
                }
                <label class="setting-row toggle">
                    <span>{ t("Recent photos") }</span>
                    <input type="checkbox" role="switch" disabled={!pictures_on} checked={family.ai_history_photos}
                        onchange={switch(|on| FamilyPatch { ai_history_photos: Some(on), ..FamilyPatch::default() })} />
                </label>
                <p class="footnote">
                    { t2("With this on, whenever anyone mentions %@ in the family chat, the most recent photos in that chat — up to %lld, from anyone, that nobody pointed the assistant at — also go to the model your server is set up to use, after any photo on the message itself or on the one it replies to. Nearly every mention then sends pictures, which costs more. It is off unless you turn it on.", &token, &limit.to_string()) }
                    if let Some(note) = photos_note {
                        { format!(" {note}") }
                    }
                </p>
                <label class="setting-row toggle">
                    <span>{ t("Member faces") }</span>
                    <input type="checkbox" role="switch" disabled={!pictures_on} checked={family.ai_faces}
                        onchange={switch(|on| FamilyPatch { ai_faces: Some(on), ..FamilyPatch::default() })} />
                </label>
                <p class="footnote">
                    { t2("With this on, whenever anyone mentions %@ in the family chat, the profile pictures of the members named in that chat's recent history — up to %lld — also go to the model your server is set up to use, so it can tell who is who. They are the pictures members chose for themselves, not photos anyone attached; never a member who has left, and never anyone outside this family. Most mentions then send pictures, which costs more. It is off unless you turn it on; with it off, no face is ever sent.", &token, &limit.to_string()) }
                    if let Some(note) = faces_note {
                        { format!(" {note}") }
                    }
                </p>
            </section>
            <section class="group" aria-labelledby="assistant-greeting">
                <h3 id="assistant-greeting">{ t("Daily greeting") }</h3>
                <label class="setting-row toggle">
                    <span>{ t("Good morning message") }</span>
                    <input type="checkbox" role="switch" disabled={!props.greetings} checked={family.ai_greeting}
                        onchange={switch(|on| FamilyPatch { ai_greeting: Some(on), ..FamilyPatch::default() })} />
                </label>
                <p class="footnote">
                    { t("With this on, the assistant posts one short good-morning message into the family chat each day, mentioning the star signs of the birthdays your family has set. It never sends anyone's name or birth date, only the signs; it makes no claims about the date; and it never sounds a notification — it is simply there when you next open the chat.") }
                    if !props.greetings {
                        { " " }{ t("Not available here: this server doesn't post daily greetings.") }
                    }
                </p>
            </section>
            if let Some(message) = (*error).clone() {
                <p class="error" role="alert">{ message }</p>
            }
        </>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use wasm_bindgen_test::wasm_bindgen_test;

    /// A member limit drawn into the page, and every patch it sends —
    /// each answered at once, as a server would.
    fn limit(cap: Option<i64>) -> (web_sys::Element, Rc<RefCell<Vec<FamilyPatch>>>) {
        let document = web_sys::window().unwrap().document().unwrap();
        let root = document.create_element("div").unwrap();
        document.body().unwrap().append_child(&root).unwrap();
        let sent = Rc::new(RefCell::new(Vec::new()));
        let into = sent.clone();
        let on_action = Callback::from(move |action: Action| {
            if let Action::ChangeFamily { patch, done } = action {
                into.borrow_mut().push(patch);
                done.emit(None);
            }
        });
        let props = LimitProps {
            family: Family {
                max_members: cap,
                ..Default::default()
            },
            count: 3,
            ceiling: 10,
            on_action,
        };
        yew::Renderer::<MemberLimit>::with_root_and_props(root.clone(), props).render();
        (root, sent)
    }

    fn element(root: &web_sys::Element, css: &str) -> web_sys::HtmlElement {
        root.query_selector(css)
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap()
    }

    /// Two clicks before the next render are two steps — the second must
    /// not step from what the first render drew — and one write.
    #[wasm_bindgen_test]
    async fn two_quick_steps_are_one_write_of_where_they_ended() {
        let (root, sent) = limit(Some(3));
        gloo_timers::future::TimeoutFuture::new(30).await;
        let more = element(&root, ".stepper button[aria-label=More]");
        more.click();
        more.click();
        gloo_timers::future::TimeoutFuture::new(CAP_DEBOUNCE_MS + 250).await;
        assert_eq!(sent.borrow().len(), 1, "one write for the run");
        assert_eq!(sent.borrow()[0].max_members, Some(Some(5)));
        root.remove();
    }

    /// An answer to an OLDER write must not wipe what was changed since: the
    /// draft it would clear is the newer one, still on its way.
    #[wasm_bindgen_test]
    async fn an_older_answer_does_not_undo_a_newer_change() {
        let document = web_sys::window().unwrap().document().unwrap();
        let root = document.create_element("div").unwrap();
        document.body().unwrap().append_child(&root).unwrap();
        let held: Rc<RefCell<Vec<Done>>> = Rc::new(RefCell::new(Vec::new()));
        let into = held.clone();
        let on_action = Callback::from(move |action: Action| {
            if let Action::ChangeFamily { done, .. } = action {
                into.borrow_mut().push(done);
            }
        });
        let props = LimitProps {
            family: Family::default(),
            count: 3,
            ceiling: 10,
            on_action,
        };
        yew::Renderer::<MemberLimit>::with_root_and_props(root.clone(), props).render();
        gloo_timers::future::TimeoutFuture::new(30).await;
        let switch: web_sys::HtmlInputElement = root
            .query_selector("input[type=checkbox]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        switch.click();
        gloo_timers::future::TimeoutFuture::new(CAP_DEBOUNCE_MS + 150).await;
        assert_eq!(held.borrow().len(), 1, "the first write is out");
        element(&root, ".stepper button[aria-label=More]").click();
        gloo_timers::future::TimeoutFuture::new(30).await;
        // The first write answers now — after the step.
        let first = held.borrow_mut().remove(0);
        first.emit(None);
        gloo_timers::future::TimeoutFuture::new(30).await;
        let value: web_sys::HtmlInputElement = root
            .query_selector("#most-members")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        assert!(switch.checked(), "still limited");
        assert_eq!(value.value(), "4", "the step is still drawn");
        root.remove();
    }

    /// The policy chosen is drawn while its save is out — not the old one,
    /// which the family still holds until the answer.
    #[wasm_bindgen_test]
    async fn a_chosen_policy_is_drawn_while_it_is_saved() {
        let document = web_sys::window().unwrap().document().unwrap();
        let root = document.create_element("div").unwrap();
        document.body().unwrap().append_child(&root).unwrap();
        let held: Rc<RefCell<Vec<Done>>> = Rc::new(RefCell::new(Vec::new()));
        let into = held.clone();
        let on_action = Callback::from(move |action: Action| {
            if let Action::ChangeFamily { done, .. } = action {
                into.borrow_mut().push(done);
            }
        });
        let props = PolicyProps {
            family: Family {
                join_policy: "open".into(),
                ..Default::default()
            },
            on_action,
        };
        yew::Renderer::<JoinPolicy>::with_root_and_props(root.clone(), props).render();
        gloo_timers::future::TimeoutFuture::new(30).await;
        let closed: web_sys::HtmlInputElement = root
            .query_selector("input[value=closed]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        closed.click();
        gloo_timers::future::TimeoutFuture::new(30).await;
        assert_eq!(held.borrow().len(), 1, "sent");
        assert!(closed.checked(), "drawn as chosen while it is out");
        assert!(root
            .text_content()
            .unwrap_or_default()
            .contains("The invite code stops working"));
        root.remove();
    }

    #[wasm_bindgen_test]
    fn turning_vision_off_draws_its_two_switches_off_too() {
        let family = Family {
            ai_vision: true,
            ai_history_photos: true,
            ai_faces: true,
            ..Default::default()
        };
        let off = FamilyPatch {
            ai_vision: Some(false),
            ..FamilyPatch::default()
        };
        let shown = overlay(&family, Some(&off));
        assert!(!shown.ai_vision && !shown.ai_history_photos && !shown.ai_faces);
        assert_eq!(overlay(&family, None), family);
    }

    /// Turned off, the switch stays off for the 600 ms its null waits —
    /// the Mac's springs back on, and a second click then cancels the
    /// removal.
    #[wasm_bindgen_test]
    async fn turned_off_stays_off_while_its_write_waits() {
        let (root, sent) = limit(Some(4));
        gloo_timers::future::TimeoutFuture::new(30).await;
        let switch: web_sys::HtmlInputElement = root
            .query_selector("input[type=checkbox]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        assert!(switch.checked());
        switch.click();
        gloo_timers::future::TimeoutFuture::new(60).await;
        assert!(!switch.checked(), "still off while the write waits");
        assert!(sent.borrow().is_empty(), "nothing sent before the pause");
        gloo_timers::future::TimeoutFuture::new(CAP_DEBOUNCE_MS + 250).await;
        assert_eq!(sent.borrow().len(), 1);
        assert_eq!(
            sent.borrow()[0].max_members,
            Some(None),
            "a null, which clears"
        );
        root.remove();
    }

    #[wasm_bindgen_test]
    fn a_report_says_what_the_message_carried() {
        let photo = ReportedAttachment {
            kind: "photo".into(),
            name: None,
        };
        assert_eq!(carried(&[]), None);
        assert_eq!(
            carried(std::slice::from_ref(&photo)).as_deref(),
            Some("Photo")
        );
        assert_eq!(
            carried(&[photo.clone(), photo]).as_deref(),
            Some("2 Photos"),
            "the apps' own words for a pile of them"
        );
        assert_eq!(
            carried(&[ReportedAttachment {
                kind: "file".into(),
                name: Some("minutes.pdf".into())
            }])
            .as_deref(),
            Some("minutes.pdf")
        );
    }

    #[wasm_bindgen_test]
    fn an_unknown_reason_is_something_else() {
        assert_eq!(reason_label("spam"), "Spam");
        assert_eq!(reason_label("threats"), "Something else");
    }

    #[wasm_bindgen_test]
    fn a_full_family_leaves_the_request_waiting() {
        let full = ApiError::Server {
            code: "family_full".into(),
            message: String::new(),
        };
        assert!(request_failure(&full).ends_with("the request is still waiting."));
    }

    #[wasm_bindgen_test]
    fn the_policy_caption_follows_the_policy() {
        assert_eq!(
            policy_caption("open"),
            "Anyone with the invite code joins straight away."
        );
        assert!(policy_caption("closed").starts_with("The invite code stops working"));
        assert!(policy_caption("approval").starts_with("With approval"));
    }
}
