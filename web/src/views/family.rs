//! The family: who is in it, and — for its owner — the door and the house
//! rules (ios MacFamilyView, with the iPhone's FamilyManageView where the
//! Mac's is the one that disagrees with the protocol).
//!
//! What the owner sees: the invite code (copy, rotate), the join requests
//! waiting, the report inbox, the join policy, the member limit, the
//! assistant's switches, and — where the server can fetch weather — the
//! places whose forecast the daily greeting mentions. What everybody sees: the members, with a way to
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
use fc_text::greeting_places::{self, Write};
use fc_text::i18n::{t, t1, t2, tn};
use gloo_timers::callback::Timeout;
use wasm_bindgen::JsCast;
use web_sys::{HtmlInputElement, HtmlSelectElement};
use yew::prelude::*;

use crate::actions::{Action, Done};
use crate::api::{ApiError, FamilyPatch};
use crate::model::{
    Assistant, Family, JoinRequest, Me, Member, PackItem, Report, ReportedAttachment,
};
use crate::time;
use crate::views::avatar::Avatar;
use crate::views::birthday::BirthdayDialog;
use crate::views::dialog::{generic_failure, Confirm};
use crate::views::password::ResetPasswordDialog;
use crate::views::report::{ReportDialog, ReportTarget, REASONS};
use crate::views::stickers::PackSection;

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
    /// The family's sticker pack, in the order it was added to, with the
    /// server's limits — None on a server that predates packs, where no
    /// section is drawn at all (docs/protocol.md, "Sticker pack").
    #[prop_or_default]
    pub pack: Option<(Vec<PackItem>, crate::pack::Limits)>,
    /// A sticker is on its way into the pack.
    #[prop_or_default]
    pub pack_adding: bool,
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
                // The pack is the family's, like the board: everybody sees
                // it here and may add to it, not only the owner.
                if let Some((items, limits)) = props.pack.clone() {
                    <PackSection
                        {items}
                        {limits}
                        my_user_id={me}
                        {owner}
                        adding={props.pack_adding}
                        on_action={props.on_action.clone()}
                    />
                }
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
    // Tied to no other switch: vision going off leaves it as it was.
    if let Some(on) = patch.ai_transcripts {
        shown.ai_transcripts = on;
    }
    // Tied to no other switch either.
    if let Some(on) = patch.ai_lookups {
        shown.ai_lookups = on;
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
            <TranscriptsSwitch
                on={family.ai_transcripts}
                transcribe={props.assistant.transcribe}
                processor={props
                    .assistant
                    .processor
                    .clone()
                    .filter(|processor| !processor.trim().is_empty())
                    .unwrap_or_else(|| props.assistant.display_name.clone())}
                on_change={switch(|on| FamilyPatch { ai_transcripts: Some(on), ..FamilyPatch::default() })}
            />
            if fc_text::lookups::offered(&props.assistant.lookups) {
                <LookupsSwitch
                    on={family.ai_lookups}
                    lookups={props.assistant.lookups.clone()}
                    on_change={switch(|on| FamilyPatch { ai_lookups: Some(on), ..FamilyPatch::default() })}
                />
            }
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
            // Drawn inside the owner's sections only, so the owner half of
            // the rule holds here; the server's half is these two keys.
            if greeting_places::offered(true, props.greetings, props.assistant.greeting_weather) {
                <GreetingPlaces
                    saved={props.family.greeting_places.clone()}
                    greeting_on={family.ai_greeting}
                    on_action={props.on_action.clone()}
                />
            }
            if let Some(message) = (*error).clone() {
                <p class="error" role="alert">{ message }</p>
            }
        </>
    }
}

#[derive(Properties, PartialEq)]
pub struct TranscriptsSwitchProps {
    /// The family's `ai_transcripts`, as a change on its way leaves it.
    pub on: bool,
    /// Whether this SERVER can turn a recording into text at all.
    pub transcribe: bool,
    /// Who the sound goes to — `assistant.processor`, verbatim.
    pub processor: String,
    pub on_change: Callback<Event>,
}

/// The owner's `ai_transcripts` (docs/protocol.md, "Transcripts on
/// request"): whether members may ask for the text of OTHER members'
/// recordings in the family chat. A section of its own, beside the
/// pictures; inert-but-explained on a server that cannot transcribe, as the
/// greeting is on one that posts none.
#[function_component(TranscriptsSwitch)]
pub fn transcripts_switch(props: &TranscriptsSwitchProps) -> Html {
    html! {
        <section class="group" aria-label={t("Voice and video as text")}>
            <label class="setting-row toggle">
                <span>{ t("Voice and video as text") }</span>
                <input type="checkbox" role="switch" class="transcripts-switch"
                    disabled={!props.transcribe} checked={props.on}
                    onchange={props.on_change.clone()} />
            </label>
            <p class="footnote">
                { t1("With this on, members can ask for the text of other members' voice notes, audio and videos in the family chat, and that recording's sound is then sent to %@. It is sent only when someone asks, and only if the member who sent it has agreed to the assistant. Everyone can get the text of their own recordings without this. It is off unless you turn it on.", &props.processor) }
                if !props.transcribe {
                    { " " }{ t("Not available here: this server can't turn recordings into text.") }
                }
            </p>
        </section>
    }
}

#[derive(Properties, PartialEq)]
pub struct LookupsSwitchProps {
    /// The family's `ai_lookups`, as a change on its way leaves it.
    pub on: bool,
    /// `assistant.lookups` — who the queries go to. The switch is drawn
    /// only where there is somebody to name.
    pub lookups: Vec<String>,
    pub on_change: Callback<Event>,
}

/// The owner's `ai_lookups` (docs/protocol.md, "Looking things up"):
/// whether the assistant may look things up for this family at all. A
/// section of its own, offered only on a server with a lookup source — off
/// one, it would be a switch that does nothing — with a footnote naming the
/// providers, the way the consent screen names `processor`.
#[function_component(LookupsSwitch)]
pub fn lookups_switch(props: &LookupsSwitchProps) -> Html {
    let named = fc_text::lookups::names(&fc_text::lookups::providers(&props.lookups));
    html! {
        <section class="group" aria-labelledby="assistant-lookups">
            <h3 id="assistant-lookups">{ fc_text::lookups::heading() }</h3>
            <label class="setting-row toggle">
                <span>{ t("Can look things up") }</span>
                <input type="checkbox" role="switch" class="lookups-switch"
                    checked={props.on}
                    onchange={props.on_change.clone()} />
            </label>
            <p class="footnote">{ fc_text::lookups::switch_footnote(&named) }</p>
        </section>
    }
}

#[derive(Properties, PartialEq)]
pub struct GreetingPlacesProps {
    /// The family's `greeting_places`, as the server last answered — what
    /// is drawn whenever nothing is being edited.
    pub saved: Vec<String>,
    /// Whether the family's greeting is on. Off, the field is still the
    /// owner's to fill in — the places wait for the greeting — and is drawn
    /// dimmed beneath the switch that would use them.
    pub greeting_on: bool,
    pub on_action: Callback<Action>,
}

/// What the places field draws: the family's list with any empty rows the
/// owner added, or the rows as they are being typed.
#[derive(Clone, PartialEq)]
enum PlacesDraft {
    Untouched { blanks: usize },
    Editing(Vec<String>),
}

struct PlacesCell {
    draft: PlacesDraft,
    /// Bumped by every keystroke and every write, so only the answer to the
    /// LAST write — with nothing typed since — puts the server's list back.
    generation: u64,
    /// Set by "Add place", so the row it adds takes the focus once drawn.
    focus_last: bool,
}

impl PlacesCell {
    fn rows(&self, saved: &[String]) -> Vec<String> {
        match &self.draft {
            PlacesDraft::Untouched { blanks } => greeting_places::rows(saved, *blanks),
            PlacesDraft::Editing(rows) => rows.clone(),
        }
    }
}

/// The owner's places for the greeting's weather (docs/protocol.md,
/// "Today's weather, for places the owner chose"; ios
/// FamilyAssistantSettings): up to three names, each its own row with a
/// remove button, "Add place" below them, and the footnote saying where the
/// names go. A row is written when it is committed — Enter, or leaving it —
/// and removing a row writes at once; each write sends the WHOLE list, as
/// the protocol's PATCH replaces it, and the server's answer is what is then
/// drawn.
#[function_component(GreetingPlaces)]
pub fn greeting_places_field(props: &GreetingPlacesProps) -> Html {
    let cell = use_mut_ref(|| PlacesCell {
        draft: PlacesDraft::Untouched { blanks: 0 },
        generation: 0,
        focus_last: false,
    });
    let redraw = use_force_update();
    let error = use_state(|| Option::<String>::None);
    let list = use_node_ref();
    {
        let cell = cell.clone();
        let list = list.clone();
        use_effect(move || {
            if std::mem::take(&mut cell.borrow_mut().focus_last) {
                let last = list
                    .cast::<web_sys::Element>()
                    .and_then(|list| list.query_selector_all("input.place-name").ok())
                    .and_then(|inputs| inputs.item(inputs.length().saturating_sub(1)))
                    .and_then(|node| node.dyn_into::<web_sys::HtmlElement>().ok());
                if let Some(last) = last {
                    let _ = last.focus();
                }
            }
        });
    }
    let rows = cell.borrow().rows(&props.saved);
    // Commit `rows`: send them if they change the family's list, put the
    // server's list back if they say the same, refuse what it would refuse.
    let commit = {
        let cell = cell.clone();
        let redraw = redraw.clone();
        let error = error.clone();
        let on_action = props.on_action.clone();
        let saved = props.saved.clone();
        Rc::new(move |rows: Vec<String>| {
            error.set(None);
            match greeting_places::write(&rows, &saved) {
                Write::Nothing => {
                    let mut held = cell.borrow_mut();
                    held.generation += 1;
                    held.draft = PlacesDraft::Untouched {
                        blanks: greeting_places::blanks(&rows),
                    };
                }
                Write::Refused(_) => {
                    cell.borrow_mut().draft = PlacesDraft::Editing(rows);
                    error.set(Some(t("Couldn't save that. Try again.").to_string()));
                }
                Write::Send(places) => {
                    let generation = {
                        let mut held = cell.borrow_mut();
                        held.draft = PlacesDraft::Editing(rows);
                        held.generation += 1;
                        held.generation
                    };
                    let cell = cell.clone();
                    let redraw = redraw.clone();
                    let error = error.clone();
                    on_action.emit(Action::ChangeFamily {
                        patch: FamilyPatch {
                            greeting_places: Some(places),
                            ..FamilyPatch::default()
                        },
                        done: Callback::from(move |failure: Option<ApiError>| {
                            let mut held = cell.borrow_mut();
                            if held.generation != generation {
                                return;
                            }
                            match failure {
                                // The answer is the truth now: the list the
                                // server KEPT, which the family holds by the
                                // time this is drawn. Only the empty rows
                                // still waiting for a name stay.
                                None => {
                                    let blanks = match &held.draft {
                                        PlacesDraft::Editing(rows) => greeting_places::blanks(rows),
                                        PlacesDraft::Untouched { blanks } => *blanks,
                                    };
                                    held.draft = PlacesDraft::Untouched { blanks };
                                }
                                // What was typed stays, to be tried again.
                                Some(failure) => {
                                    error.set(Some(
                                        match failure.code() {
                                            Some("not_family_owner") => {
                                                t("Only the family owner can change this.")
                                            }
                                            _ => t("Couldn't save that. Try again."),
                                        }
                                        .to_string(),
                                    ));
                                }
                            }
                            drop(held);
                            redraw.force_update();
                        }),
                    });
                }
            }
            redraw.force_update();
        })
    };
    let input = |index: usize| {
        let cell = cell.clone();
        let saved = props.saved.clone();
        Callback::from(move |event: InputEvent| {
            let field: HtmlInputElement = event.target_unchecked_into();
            let raw = field.value();
            let kept = greeting_places::typed(&raw);
            if kept != raw {
                field.set_value(&kept);
            }
            let mut held = cell.borrow_mut();
            let mut rows = held.rows(&saved);
            if let Some(row) = rows.get_mut(index) {
                *row = kept;
            }
            held.draft = PlacesDraft::Editing(rows);
            held.generation += 1;
        })
    };
    let change = {
        let cell = cell.clone();
        let commit = commit.clone();
        let saved = props.saved.clone();
        Callback::from(move |_: Event| {
            let rows = cell.borrow().rows(&saved);
            commit(rows);
        })
    };
    let remove = |index: usize| {
        let cell = cell.clone();
        let commit = commit.clone();
        let saved = props.saved.clone();
        Callback::from(move |_: MouseEvent| {
            let mut rows = cell.borrow().rows(&saved);
            if index < rows.len() {
                rows.remove(index);
            }
            commit(rows);
        })
    };
    let add = {
        let cell = cell.clone();
        let redraw = redraw.clone();
        let saved = props.saved.clone();
        Callback::from(move |_: MouseEvent| {
            let mut held = cell.borrow_mut();
            let rows = held.rows(&saved);
            if !greeting_places::can_add(rows.len()) {
                return;
            }
            held.draft = match &held.draft {
                PlacesDraft::Untouched { blanks } => PlacesDraft::Untouched { blanks: blanks + 1 },
                PlacesDraft::Editing(rows) => {
                    let mut rows = rows.clone();
                    rows.push(String::new());
                    PlacesDraft::Editing(rows)
                }
            };
            held.focus_last = true;
            drop(held);
            redraw.force_update();
        })
    };
    let heading = greeting_places::heading();
    let remove_label = greeting_places::remove_label();
    html! {
        <section class={classes!("group", "greeting-places", (!props.greeting_on).then_some("is-dimmed"))}
            aria-labelledby="assistant-greeting-places">
            <h3 id="assistant-greeting-places">{ heading }</h3>
            <div class="place-rows" ref={list}>
                { for rows.iter().enumerate().map(|(index, name)| html! {
                    <div class="setting-row place-row" key={index}>
                        <input type="text" class="place-name" value={name.clone()}
                            placeholder={greeting_places::placeholder()}
                            aria-label={greeting_places::placeholder()}
                            autocomplete="off" spellcheck="false"
                            oninput={input(index)} onchange={change.clone()} />
                        <button class="link danger place-remove" aria-label={remove_label}
                            title={remove_label} onclick={remove(index)}>{ "\u{2212}" }</button>
                    </div>
                }) }
            </div>
            if greeting_places::can_add(rows.len()) {
                <div class="setting-row">
                    <button class="link place-add" onclick={add}>{ greeting_places::add_label() }</button>
                </div>
            } else {
                <p class="footnote place-limit">{ greeting_places::limit_note() }</p>
            }
            <p class="footnote">{ greeting_places::footnote() }</p>
            if let Some(message) = (*error).clone() {
                <p class="error" role="alert">{ message }</p>
            }
        </section>
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

    /// The transcripts switch rides on nothing: vision going off leaves it
    /// as it was, and its own patch is the only thing that moves it.
    #[wasm_bindgen_test]
    fn the_transcripts_switch_is_tied_to_no_other() {
        let family = Family {
            ai_vision: true,
            ai_transcripts: true,
            ..Default::default()
        };
        let vision_off = FamilyPatch {
            ai_vision: Some(false),
            ..FamilyPatch::default()
        };
        assert!(overlay(&family, Some(&vision_off)).ai_transcripts);
        let off = FamilyPatch {
            ai_transcripts: Some(false),
            ..FamilyPatch::default()
        };
        let shown = overlay(&family, Some(&off));
        assert!(!shown.ai_transcripts && shown.ai_vision);
        assert_eq!(
            serde_json::to_value(&off).unwrap(),
            serde_json::json!({"ai_transcripts": false}),
            "the one key that changed"
        );
    }

    /// THE OWNER'S SWITCH, drawn and pressed: one write of exactly its own
    /// key; the footer names who the sound goes to; and on a server that
    /// cannot transcribe it is drawn inert, with the reason.
    #[wasm_bindgen_test]
    async fn the_owners_transcripts_switch_writes_its_own_key() {
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
        let assistant = |transcribe: bool| Assistant {
            user_id: 2,
            display_name: "Assistant".into(),
            mention: Some("@ai".into()),
            draw: None,
            vision: true,
            images: false,
            processor: Some("Microsoft — Azure OpenAI".into()),
            transcribe,
            transcribe_max_bytes: transcribe.then_some(26_214_400),
            lookups: Vec::new(),
            greeting_weather: false,
        };
        let props = AssistantProps {
            family: Family::default(),
            assistant: assistant(true),
            greetings: false,
            on_action: on_action.clone(),
        };
        yew::Renderer::<AssistantSettings>::with_root_and_props(root.clone(), props).render();
        gloo_timers::future::TimeoutFuture::new(30).await;
        let switch: web_sys::HtmlInputElement = root
            .query_selector(".transcripts-switch")
            .unwrap()
            .expect("the switch")
            .dyn_into()
            .unwrap();
        assert!(!switch.checked(), "off unless the owner turns it on");
        assert!(!switch.disabled());
        let text = root.text_content().unwrap_or_default();
        assert!(text.contains("Voice and video as text"));
        assert!(text.contains("that recording's sound is then sent to Microsoft — Azure OpenAI."));
        assert!(!text.contains("Not available here: this server can't turn recordings into text."));
        switch.click();
        gloo_timers::future::TimeoutFuture::new(30).await;
        assert_eq!(
            *sent.borrow(),
            vec![FamilyPatch {
                ai_transcripts: Some(true),
                ..FamilyPatch::default()
            }]
        );
        root.remove();

        let root = document.create_element("div").unwrap();
        document.body().unwrap().append_child(&root).unwrap();
        let props = AssistantProps {
            family: Family::default(),
            assistant: assistant(false),
            greetings: false,
            on_action,
        };
        yew::Renderer::<AssistantSettings>::with_root_and_props(root.clone(), props).render();
        gloo_timers::future::TimeoutFuture::new(30).await;
        let switch: web_sys::HtmlInputElement = root
            .query_selector(".transcripts-switch")
            .unwrap()
            .expect("the switch")
            .dyn_into()
            .unwrap();
        assert!(switch.disabled(), "inert where the server cannot");
        assert!(root
            .text_content()
            .unwrap_or_default()
            .contains("Not available here: this server can't turn recordings into text."));
        root.remove();
    }

    /// The lookups switch rides on nothing: vision or transcripts going
    /// off leave it as it was, and its own patch is one key.
    #[wasm_bindgen_test]
    fn the_lookups_switch_is_tied_to_no_other() {
        let family = Family {
            ai_vision: true,
            ai_transcripts: true,
            ai_lookups: true,
            ..Default::default()
        };
        for other in [
            FamilyPatch {
                ai_vision: Some(false),
                ..FamilyPatch::default()
            },
            FamilyPatch {
                ai_transcripts: Some(false),
                ..FamilyPatch::default()
            },
            FamilyPatch {
                ai_history: Some(false),
                ..FamilyPatch::default()
            },
        ] {
            assert!(overlay(&family, Some(&other)).ai_lookups, "{other:?}");
        }
        let off = FamilyPatch {
            ai_lookups: Some(false),
            ..FamilyPatch::default()
        };
        let shown = overlay(&family, Some(&off));
        assert!(!shown.ai_lookups && shown.ai_vision && shown.ai_transcripts);
    }

    /// THE OWNER'S LOOKUPS SWITCH, drawn and pressed: present only where
    /// the server names providers, off unless turned on, its footnote
    /// naming them, and one write of exactly its own key.
    #[wasm_bindgen_test]
    async fn the_owners_lookups_switch_names_the_providers_and_writes_its_own_key() {
        let document = web_sys::window().unwrap().document().unwrap();
        let sent = Rc::new(RefCell::new(Vec::new()));
        let into = sent.clone();
        let on_action = Callback::from(move |action: Action| {
            if let Action::ChangeFamily { patch, done } = action {
                into.borrow_mut().push(patch);
                done.emit(None);
            }
        });
        let assistant = |lookups: &[&str]| Assistant {
            user_id: 2,
            display_name: "Assistant".into(),
            mention: Some("@ai".into()),
            draw: None,
            vision: false,
            images: false,
            processor: Some("Microsoft — Azure OpenAI".into()),
            transcribe: false,
            transcribe_max_bytes: None,
            lookups: lookups.iter().map(|name| name.to_string()).collect(),
            greeting_weather: false,
        };
        let draw = |assistant: Assistant| {
            let root = document.create_element("div").unwrap();
            document.body().unwrap().append_child(&root).unwrap();
            let props = AssistantProps {
                family: Family::default(),
                assistant,
                greetings: false,
                on_action: on_action.clone(),
            };
            yew::Renderer::<AssistantSettings>::with_root_and_props(root.clone(), props).render();
            root
        };

        let root = draw(assistant(&["Brave Search", "Open-Meteo", "Wikipedia"]));
        gloo_timers::future::TimeoutFuture::new(30).await;
        let switch: web_sys::HtmlInputElement = root
            .query_selector(".lookups-switch")
            .unwrap()
            .expect("the switch")
            .dyn_into()
            .unwrap();
        assert!(!switch.checked(), "off unless the owner turns it on");
        let text = root.text_content().unwrap_or_default();
        assert!(text.contains("Looking things up"));
        assert!(text.contains("Can look things up"));
        assert!(
            text.contains("in Brave Search, Open-Meteo and Wikipedia. Only a short search query"),
            "{text}"
        );
        switch.click();
        gloo_timers::future::TimeoutFuture::new(30).await;
        assert_eq!(
            *sent.borrow(),
            vec![FamilyPatch {
                ai_lookups: Some(true),
                ..FamilyPatch::default()
            }]
        );
        root.remove();

        // No source on this server — absent and `[]` alike: no switch.
        for nobody in [&[][..], &["  "][..]] {
            let root = draw(assistant(nobody));
            gloo_timers::future::TimeoutFuture::new(30).await;
            assert!(root.query_selector(".lookups-switch").unwrap().is_none());
            assert!(!root
                .text_content()
                .unwrap_or_default()
                .contains("Can look things up"));
            root.remove();
        }
    }

    /// Every place-field test's server: it keeps a list as the real one
    /// does (or, as one that predates the key, answers with none), and the
    /// family it answers with is what the field is drawn from next.
    #[derive(Properties, PartialEq)]
    struct PlacesHostProps {
        initial: Vec<String>,
        greeting_on: bool,
        old_server: bool,
        refuse: bool,
        sent: Rc<RefCell<Vec<FamilyPatch>>>,
    }

    #[function_component(PlacesHost)]
    fn places_host(props: &PlacesHostProps) -> Html {
        let saved = use_state(|| props.initial.clone());
        let on_action = {
            let saved = saved.clone();
            let sent = props.sent.clone();
            let old_server = props.old_server;
            let refuse = props.refuse;
            Callback::from(move |action: Action| {
                if let Action::ChangeFamily { patch, done } = action {
                    sent.borrow_mut().push(patch.clone());
                    if refuse {
                        done.emit(Some(ApiError::Server {
                            code: "validation".into(),
                            message: "greeting place 1 is empty".into(),
                        }));
                        return;
                    }
                    let list = patch.greeting_places.unwrap_or_default();
                    saved.set(if old_server {
                        Vec::new()
                    } else {
                        greeting_places::places(&list).expect("the server keeps it")
                    });
                    done.emit(None);
                }
            })
        };
        html! {
            <GreetingPlaces saved={(*saved).clone()} greeting_on={props.greeting_on} {on_action} />
        }
    }

    fn places_host(
        initial: &[&str],
        greeting_on: bool,
        old_server: bool,
        refuse: bool,
    ) -> (web_sys::Element, Rc<RefCell<Vec<FamilyPatch>>>) {
        let document = web_sys::window().unwrap().document().unwrap();
        let root = document.create_element("div").unwrap();
        document.body().unwrap().append_child(&root).unwrap();
        let sent = Rc::new(RefCell::new(Vec::new()));
        let props = PlacesHostProps {
            initial: initial.iter().map(|name| name.to_string()).collect(),
            greeting_on,
            old_server,
            refuse,
            sent: sent.clone(),
        };
        yew::Renderer::<PlacesHost>::with_root_and_props(root.clone(), props).render();
        (root, sent)
    }

    fn place_fields(root: &web_sys::Element) -> Vec<HtmlInputElement> {
        let found = root.query_selector_all("input.place-name").unwrap();
        (0..found.length())
            .map(|index| found.item(index).unwrap().dyn_into().unwrap())
            .collect()
    }

    fn place_values(root: &web_sys::Element) -> Vec<String> {
        place_fields(root)
            .iter()
            .map(|field| field.value())
            .collect()
    }

    /// Typed, then committed — what Enter or leaving the field does.
    fn type_place(field: &HtmlInputElement, value: &str) {
        field.set_value(value);
        let init = web_sys::EventInit::new();
        init.set_bubbles(true);
        field
            .dispatch_event(&web_sys::Event::new_with_event_init_dict("input", &init).unwrap())
            .unwrap();
    }

    fn commit_place(field: &HtmlInputElement) {
        let init = web_sys::EventInit::new();
        init.set_bubbles(true);
        field
            .dispatch_event(&web_sys::Event::new_with_event_init_dict("change", &init).unwrap())
            .unwrap();
    }

    fn places_sent(patch: &FamilyPatch) -> Vec<String> {
        assert_eq!(
            *patch,
            FamilyPatch {
                greeting_places: patch.greeting_places.clone(),
                ..FamilyPatch::default()
            },
            "the places go alone"
        );
        patch.greeting_places.clone().expect("the places key")
    }

    async fn settle() {
        gloo_timers::future::TimeoutFuture::new(30).await;
    }

    /// THE PLACES FIELD'S VISIBILITY: drawn beside the greeting only where
    /// the server posts greetings AND says it can fetch their weather —
    /// never on a server that predates the key (absent reads as false) —
    /// with the family's places in it and the footnote saying where they
    /// go. A member sees none of the owner's assistant settings at all.
    #[wasm_bindgen_test]
    async fn the_places_field_is_drawn_only_where_the_server_uses_it() {
        let document = web_sys::window().unwrap().document().unwrap();
        let assistant = |greeting_weather: bool| Assistant {
            user_id: 2,
            display_name: "Assistant".into(),
            mention: Some("@ai".into()),
            draw: None,
            vision: false,
            images: false,
            processor: Some("Microsoft — Azure OpenAI".into()),
            transcribe: false,
            transcribe_max_bytes: None,
            lookups: Vec::new(),
            greeting_weather,
        };
        let draw = |greetings: bool, greeting_weather: bool| {
            let root = document.create_element("div").unwrap();
            document.body().unwrap().append_child(&root).unwrap();
            let props = AssistantProps {
                family: Family {
                    ai_greeting: true,
                    greeting_places: vec!["Moscow".into(), "Belgrade".into()],
                    ..Default::default()
                },
                assistant: assistant(greeting_weather),
                greetings,
                on_action: Callback::noop(),
            };
            yew::Renderer::<AssistantSettings>::with_root_and_props(root.clone(), props).render();
            root
        };

        let root = draw(true, true);
        settle().await;
        assert!(root.query_selector(".greeting-places").unwrap().is_some());
        assert_eq!(place_values(&root), vec!["Moscow", "Belgrade"]);
        let text = root.text_content().unwrap_or_default();
        assert!(text.contains("Weather in the greeting"));
        assert!(text.contains("Only the place names are sent to Open-Meteo to fetch the forecast"));
        assert!(text.contains("Add place"));
        // It follows the greeting's own section.
        let html = root.inner_html();
        assert!(
            html.find("assistant-greeting\"").unwrap()
                < html.find("assistant-greeting-places").unwrap()
        );
        root.remove();

        for (greetings, greeting_weather) in [(true, false), (false, true), (false, false)] {
            let root = draw(greetings, greeting_weather);
            settle().await;
            assert!(
                root.query_selector(".greeting-places").unwrap().is_none(),
                "{greetings} {greeting_weather}"
            );
            assert!(!root
                .text_content()
                .unwrap_or_default()
                .contains("Weather in the greeting"));
            root.remove();
        }
    }

    /// ADD, TYPE, COMMIT: a row is added empty and focused; committing it
    /// writes the WHOLE list, folded as the server folds it; at three rows
    /// "Add place" gives way to the limit; a repeat is no change and
    /// writes nothing; and what is drawn after is what the server kept.
    #[wasm_bindgen_test]
    async fn adding_and_committing_a_place_writes_the_whole_list() {
        let (root, sent) = places_host(&["Moscow"], true, false, false);
        settle().await;
        assert_eq!(place_values(&root), vec!["Moscow"]);
        element(&root, ".place-add").click();
        settle().await;
        let fields = place_fields(&root);
        assert_eq!(place_values(&root), vec!["Moscow", ""]);
        let focused = web_sys::window()
            .unwrap()
            .document()
            .unwrap()
            .active_element();
        assert_eq!(
            focused.map(|focused| focused.is_same_node(Some(&fields[1]))),
            Some(true),
            "the added row has the focus"
        );
        // An empty row committed is no place, and nothing is written.
        commit_place(&fields[1]);
        settle().await;
        assert!(sent.borrow().is_empty());
        let fields = place_fields(&root);
        type_place(&fields[1], "  Novi   Sad ");
        commit_place(&fields[1]);
        settle().await;
        assert_eq!(sent.borrow().len(), 1);
        assert_eq!(places_sent(&sent.borrow()[0]), vec!["Moscow", "Novi Sad"]);
        assert_eq!(place_values(&root), vec!["Moscow", "Novi Sad"], "as kept");

        element(&root, ".place-add").click();
        settle().await;
        assert_eq!(place_fields(&root).len(), 3);
        assert!(root.query_selector(".place-add").unwrap().is_none());
        assert_eq!(
            element(&root, ".place-limit").text_content().unwrap(),
            "Up to 3 places."
        );
        let fields = place_fields(&root);
        type_place(&fields[2], "MOSCOW");
        commit_place(&fields[2]);
        settle().await;
        assert_eq!(sent.borrow().len(), 1, "a repeat changes nothing");
        assert_eq!(place_values(&root), vec!["Moscow", "Novi Sad"]);
        assert!(root.query_selector(".place-add").unwrap().is_some());
        root.remove();
    }

    /// REMOVE: the row goes and the list without it is written at once;
    /// removing the last writes `[]`, which clears it; removing an empty
    /// row writes nothing.
    #[wasm_bindgen_test]
    async fn removing_a_place_writes_the_list_without_it() {
        let (root, sent) = places_host(&["Moscow", "Belgrade"], true, false, false);
        settle().await;
        let remove: Vec<web_sys::HtmlElement> = {
            let found = root.query_selector_all(".place-remove").unwrap();
            (0..found.length())
                .map(|index| found.item(index).unwrap().dyn_into().unwrap())
                .collect()
        };
        assert_eq!(remove.len(), 2);
        assert_eq!(
            remove[0].get_attribute("aria-label").as_deref(),
            Some("Remove place")
        );
        remove[0].click();
        settle().await;
        assert_eq!(places_sent(&sent.borrow()[0]), vec!["Belgrade"]);
        assert_eq!(place_values(&root), vec!["Belgrade"]);

        element(&root, ".place-add").click();
        settle().await;
        assert_eq!(place_values(&root), vec!["Belgrade", ""]);
        let found = root.query_selector_all(".place-remove").unwrap();
        found
            .item(1)
            .unwrap()
            .dyn_into::<web_sys::HtmlElement>()
            .unwrap()
            .click();
        settle().await;
        assert_eq!(sent.borrow().len(), 1, "an empty row goes without a write");
        assert_eq!(place_values(&root), vec!["Belgrade"]);

        element(&root, ".place-remove").click();
        settle().await;
        assert_eq!(places_sent(&sent.borrow()[1]), Vec::<String>::new());
        assert!(place_fields(&root).is_empty());
        root.remove();
    }

    /// A field keeps the server's 80 characters — characters, not bytes —
    /// and no control character.
    #[wasm_bindgen_test]
    async fn a_place_field_stops_at_eighty_characters() {
        let (root, sent) = places_host(&[], true, false, false);
        settle().await;
        element(&root, ".place-add").click();
        settle().await;
        let field = place_fields(&root).remove(0);
        type_place(&field, &format!("Bel\u{1}grade{}", "ж".repeat(90)));
        assert_eq!(field.value().chars().count(), 80);
        assert!(field.value().starts_with("Belgradeж"));
        commit_place(&field);
        settle().await;
        let written = places_sent(&sent.borrow()[0]);
        assert_eq!(written.len(), 1);
        assert_eq!(written[0].chars().count(), 80);
        root.remove();
    }

    /// The answer is what is drawn, not what was sent: a server older than
    /// the key ignores it, and its answer carries no list.
    #[wasm_bindgen_test]
    async fn the_list_drawn_is_the_one_the_server_answered_with() {
        let (root, sent) = places_host(&[], true, true, false);
        settle().await;
        element(&root, ".place-add").click();
        settle().await;
        let field = place_fields(&root).remove(0);
        type_place(&field, "Moscow");
        commit_place(&field);
        settle().await;
        assert_eq!(places_sent(&sent.borrow()[0]), vec!["Moscow"]);
        assert!(place_values(&root).is_empty(), "the server kept none");
        root.remove();
    }

    /// A refused write keeps what was typed, and says so.
    #[wasm_bindgen_test]
    async fn a_refused_write_keeps_what_was_typed() {
        let (root, sent) = places_host(&[], true, false, true);
        settle().await;
        element(&root, ".place-add").click();
        settle().await;
        let field = place_fields(&root).remove(0);
        type_place(&field, "Moscow");
        commit_place(&field);
        settle().await;
        assert_eq!(sent.borrow().len(), 1);
        assert_eq!(place_values(&root), vec!["Moscow"]);
        assert_eq!(
            element(&root, ".greeting-places .error")
                .text_content()
                .unwrap(),
            "Couldn't save that. Try again."
        );
        root.remove();
    }

    /// With the greeting off the field is still the owner's to fill in,
    /// drawn dimmed beneath the switch that would use it.
    #[wasm_bindgen_test]
    async fn with_the_greeting_off_the_field_is_dimmed_and_still_editable() {
        let (root, _) = places_host(&["Moscow"], false, false, false);
        settle().await;
        assert!(element(&root, ".greeting-places")
            .class_list()
            .contains("is-dimmed"));
        assert!(place_fields(&root).iter().all(|field| !field.disabled()));
        root.remove();
        let (root, _) = places_host(&["Moscow"], true, false, false);
        settle().await;
        assert!(!element(&root, ".greeting-places")
            .class_list()
            .contains("is-dimmed"));
        root.remove();
    }
}
