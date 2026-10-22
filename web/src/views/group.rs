//! Groups inside a family (#107; docs/protocol.md, "Groups"): "New Group"
//! from the chat list's header, and "Group Info" from a group's title — its
//! name, its members with the admin marked, the admin's Rename, Add,
//! Remove and Delete, and every member's Leave.
//!
//! Who may do what is the shared table's (fc_text::groups::may), so this
//! file draws a button only where the server would take it; the server
//! still decides, and its refusal is said in the dialog that asked.

use std::collections::HashSet;

use fc_text::groups::{self, Power};
use fc_text::i18n::{t, t1, tn};
use web_sys::HtmlInputElement;
use yew::prelude::*;

use crate::actions::Action;
use crate::api::ApiError;
use crate::model::{Chat, Member};
use crate::views::avatar::Avatar;
use crate::views::dialog::{generic_failure, Confirm, Modal};

/// What was being asked, for the words a refusal is told in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Asking {
    Create,
    Rename,
    /// Adding, removing, leaving or deleting.
    Change,
}

/// What a refused group request says. A REFUSAL is the server's code,
/// worded by the shared table; the network, a busy server or a server's
/// own trouble is not a refusal and is said as what it is — the same words
/// every other dialog says for it — so nobody is told the group "couldn't"
/// be made when the request never arrived.
pub fn failure(error: &ApiError, asking: Asking) -> String {
    match error {
        ApiError::Server { code, .. } if code != "internal" => match asking {
            Asking::Create => groups::create_refusal(Some(code)),
            Asking::Rename => groups::change_refusal(Some(code), true),
            Asking::Change => groups::change_refusal(Some(code), false),
        }
        .to_string(),
        _ => generic_failure(error),
    }
}

/// Who may be put in a group: the family's members now — not this member
/// (they are in it already), not a deleted account, and not anybody
/// already in it. A block is no reason to leave somebody out: a refusal
/// would tell the adder about a block they are not party to (docs/protocol.md,
/// "Groups").
pub fn addable(family: &[Member], me: i64, already: &[i64]) -> Vec<Member> {
    family
        .iter()
        .filter(|member| !member.deleted && member.id != me && !already.contains(&member.id))
        .cloned()
        .collect()
}

#[derive(Properties, PartialEq)]
pub struct NewGroupProps {
    /// The family's roster.
    pub members: Vec<Member>,
    pub my_user_id: i64,
    pub on_action: Callback<Action>,
    pub on_close: Callback<()>,
}

/// "New Group": a name, and the members to add. Checked here before it is
/// sent — the name trimmed and 1 to 64 characters, somebody chosen — so the
/// server's `validation` is never the first anybody hears of either.
#[function_component(NewGroupDialog)]
pub fn new_group_dialog(props: &NewGroupProps) -> Html {
    let name = use_state(String::new);
    let chosen = use_state(Vec::<i64>::new);
    let busy = use_state(|| false);
    let error = use_state(|| Option::<String>::None);
    let candidates = addable(&props.members, props.my_user_id, &[]);

    let on_name = {
        let name = name.clone();
        Callback::from(move |event: InputEvent| {
            name.set(event.target_unchecked_into::<HtmlInputElement>().value())
        })
    };
    let submit = {
        let (name, chosen, busy, error) =
            (name.clone(), chosen.clone(), busy.clone(), error.clone());
        let on_action = props.on_action.clone();
        let on_close = props.on_close.clone();
        Callback::from(move |event: SubmitEvent| {
            event.prevent_default();
            if *busy {
                return;
            }
            let title = match groups::check_new(&name, chosen.len()) {
                Ok(title) => title.to_string(),
                Err(problem) => {
                    error.set(Some(problem.sentence().to_string()));
                    return;
                }
            };
            error.set(None);
            busy.set(true);
            let (busy, error, on_close) = (busy.clone(), error.clone(), on_close.clone());
            on_action.emit(Action::CreateGroup {
                title,
                member_ids: (*chosen).clone(),
                done: Callback::from(move |refused: Option<ApiError>| {
                    busy.set(false);
                    match refused {
                        Some(refused) => error.set(Some(failure(&refused, Asking::Create))),
                        None => on_close.emit(()),
                    }
                }),
            });
        })
    };
    let cancel = props.on_close.reform(|_: MouseEvent| ());
    html! {
        <Modal title={t("New Group")} class={classes!("group-dialog")} on_cancel={props.on_close.clone()} busy={*busy}>
            <form class="dialog-form new-group" onsubmit={submit}>
                <label class="field">
                    { t("Group Name") }
                    <input type="text" autocomplete="off" value={(*name).clone()} oninput={on_name} />
                </label>
                <fieldset>
                    <legend>{ t("Add Members") }</legend>
                    <AddPicks candidates={candidates} chosen={(*chosen).clone()} on_change={{
                        let chosen = chosen.clone();
                        Callback::from(move |now: Vec<i64>| chosen.set(now))
                    }} />
                </fieldset>
                if let Some(message) = (*error).clone() {
                    <p class="error" role="alert">{ message }</p>
                }
                <div class="dialog-actions">
                    <button type="button" class="secondary" disabled={*busy} onclick={cancel}>{ t("Cancel") }</button>
                    <button type="submit" class="primary" disabled={*busy}>{ t("Create Group") }</button>
                </div>
            </form>
        </Modal>
    }
}

#[derive(Properties, PartialEq)]
pub struct GroupInfoProps {
    pub chat: Chat,
    /// The family's roster, for the members' names and faces.
    pub members: Vec<Member>,
    pub my_user_id: i64,
    pub on_action: Callback<Action>,
    pub on_close: Callback<()>,
}

/// Which part of Group Info is in front.
#[derive(Debug, Clone, PartialEq)]
enum Step {
    Members,
    Renaming(String),
    Adding(Vec<i64>),
    ConfirmLeave,
    ConfirmDelete,
}

/// "Group Info", from a group's title.
#[function_component(GroupInfoDialog)]
pub fn group_info_dialog(props: &GroupInfoProps) -> Html {
    let step = use_state(|| Step::Members);
    let busy = use_state(|| false);
    let error = use_state(|| Option::<String>::None);
    let chat = &props.chat;
    let chat_id = chat.id;
    let me = props.my_user_id;
    let name = chat.display_title(None);
    let member_ids = chat.group_member_ids().to_vec();
    let may = |power| chat.may(power, me);

    // Every request here is heard the same way: busy while it is out, the
    // refusal said in the dialog, and on success back to the members — or,
    // for a leave or a delete, nothing at all: the group goes, and this
    // dialog with the pane it was opened from.
    let send = {
        let (step, busy, error) = (step.clone(), busy.clone(), error.clone());
        let on_action = props.on_action.clone();
        Callback::from(move |(asking, make): (Asking, ActionMaker)| {
            if *busy {
                return;
            }
            busy.set(true);
            error.set(None);
            let (step, busy, error) = (step.clone(), busy.clone(), error.clone());
            let done = Callback::from(move |refused: Option<ApiError>| {
                busy.set(false);
                match refused {
                    Some(refused) => {
                        error.set(Some(failure(&refused, asking)));
                        // A refused leave or delete goes back to the list,
                        // where the sentence is read.
                        if matches!(*step, Step::ConfirmLeave | Step::ConfirmDelete) {
                            step.set(Step::Members);
                        }
                    }
                    None => step.set(Step::Members),
                }
            });
            on_action.emit((make.0)(done));
        })
    };

    let to = |next: Step| {
        let (step, error) = (step.clone(), error.clone());
        Callback::from(move |_: MouseEvent| {
            error.set(None);
            step.set(next.clone());
        })
    };
    let close = props.on_close.reform(|_: MouseEvent| ());
    let error_line = (*error)
        .clone()
        .map(|message| html! { <p class="error" role="alert">{ message }</p> });

    let body = match (*step).clone() {
        Step::Renaming(draft) => {
            let on_input = {
                let step = step.clone();
                Callback::from(move |event: InputEvent| {
                    step.set(Step::Renaming(
                        event.target_unchecked_into::<HtmlInputElement>().value(),
                    ))
                })
            };
            let submit = {
                let (send, error, draft) = (send.clone(), error.clone(), draft.clone());
                Callback::from(move |event: SubmitEvent| {
                    event.prevent_default();
                    let Some(title) = groups::title(&draft).map(str::to_string) else {
                        error.set(Some(groups::Problem::Name.sentence().to_string()));
                        return;
                    };
                    send.emit((
                        Asking::Rename,
                        ActionMaker::new(move |done| Action::RenameGroup {
                            chat_id,
                            title: title.clone(),
                            done,
                        }),
                    ));
                })
            };
            html! {
                <form class="dialog-form" onsubmit={submit}>
                    <label class="field">
                        { t("Group Name") }
                        <input type="text" autocomplete="off" value={draft} oninput={on_input} />
                    </label>
                    { error_line.clone().unwrap_or_default() }
                    <div class="dialog-actions">
                        <button type="button" class="secondary" disabled={*busy} onclick={to(Step::Members)}>{ t("Cancel") }</button>
                        <button type="submit" class="primary" disabled={*busy}>{ t("Rename Group") }</button>
                    </div>
                </form>
            }
        }
        Step::Adding(chosen) => {
            let candidates = addable(&props.members, me, &member_ids);
            let nobody_left = candidates.is_empty();
            let submit = {
                let (send, error, chosen) = (send.clone(), error.clone(), chosen.clone());
                Callback::from(move |event: SubmitEvent| {
                    event.prevent_default();
                    if chosen.is_empty() {
                        error.set(Some(groups::Problem::NoMembers.sentence().to_string()));
                        return;
                    }
                    let user_ids = chosen.clone();
                    send.emit((
                        Asking::Change,
                        ActionMaker::new(move |done| Action::AddGroupMembers {
                            chat_id,
                            user_ids: user_ids.clone(),
                            done,
                        }),
                    ));
                })
            };
            html! {
                <form class="dialog-form" onsubmit={submit}>
                    <h3>{ t("Add Members") }</h3>
                    if nobody_left {
                        <p class="footnote">{ t("Everyone in the family is already in this group.") }</p>
                    } else {
                        <AddPicks {candidates} {chosen} on_change={{
                            let step = step.clone();
                            Callback::from(move |now: Vec<i64>| step.set(Step::Adding(now)))
                        }} />
                    }
                    { error_line.clone().unwrap_or_default() }
                    <div class="dialog-actions">
                        <button type="button" class="secondary" disabled={*busy} onclick={to(Step::Members)}>{ t("Cancel") }</button>
                        if !nobody_left {
                            <button type="submit" class="primary" disabled={*busy}>{ t("Add Members") }</button>
                        }
                    </div>
                </form>
            }
        }
        Step::Members | Step::ConfirmLeave | Step::ConfirmDelete => {
            let rows = member_ids.iter().map(|id| {
                let id = *id;
                let member = props.members.iter().find(|member| member.id == id);
                let shown = member
                    .map(|member| member.display_name.clone())
                    .unwrap_or_else(|| t("Someone").to_string());
                let remove = groups::may_remove(me, id, chat.admin_user_id, &member_ids).then(|| {
                    let send = send.clone();
                    Callback::from(move |_: MouseEvent| {
                        send.emit((
                            Asking::Change,
                            ActionMaker::new(move |done| Action::RemoveGroupMember {
                                chat_id,
                                user_id: id,
                                done,
                            }),
                        ))
                    })
                });
                html! {
                    <li class="member-row" key={id.to_string()}>
                        <Avatar title={shown.clone()} user_id={Some(id)} version={member.map_or(0, |member| member.avatar_version)} size={28} />
                        <span class="identity-text">
                            <span>
                                <strong>{ shown.clone() }</strong>
                                if chat.admin_user_id == Some(id) {
                                    <span class="capsule">{ t("Admin") }</span>
                                }
                            </span>
                        </span>
                        if let Some(remove) = remove {
                            <button type="button" class="link group-remove" disabled={*busy} onclick={remove}
                                    aria-label={format!("{} — {}", t("Remove from Group"), shown)}>
                                { t("Remove from Group") }
                            </button>
                        }
                    </li>
                }
            });
            let adding = {
                let (step, error) = (step.clone(), error.clone());
                Callback::from(move |_: MouseEvent| {
                    error.set(None);
                    step.set(Step::Adding(Vec::new()));
                })
            };
            let renaming = to(Step::Renaming(name.clone()));
            html! {
                <>
                    <p class="group-name"><strong>{ name.clone() }</strong></p>
                    <h3>{ tn("Members: %lld", member_ids.len() as i64) }</h3>
                    <ul class="members group-members">{ for rows }</ul>
                    { error_line.clone().unwrap_or_default() }
                    <div class="group-tools">
                        if may(Power::Rename) {
                            <button type="button" class="link" disabled={*busy} onclick={renaming}>{ t("Rename Group") }</button>
                        }
                        if may(Power::AddMembers) {
                            <button type="button" class="link" disabled={*busy} onclick={adding}>{ t("Add Members") }</button>
                        }
                        if may(Power::Leave) {
                            <button type="button" class="link danger" disabled={*busy} onclick={to(Step::ConfirmLeave)}>{ t("Leave Group") }</button>
                        }
                        if may(Power::Delete) {
                            <button type="button" class="link danger" disabled={*busy} onclick={to(Step::ConfirmDelete)}>{ t("Delete Group") }</button>
                        }
                    </div>
                    <div class="dialog-actions">
                        <button type="button" class="primary" onclick={close}>{ t("Done") }</button>
                    </div>
                </>
            }
        }
    };

    let back = {
        let step = step.clone();
        Callback::from(move |_: ()| step.set(Step::Members))
    };
    let confirm = match *step {
        Step::ConfirmLeave => {
            let send = send.clone();
            Some(html! {
                <Confirm
                    title={t("Leave Group")}
                    message={t1("Leave “%@”? You can come back only if its admin adds you again.", &name)}
                    confirm={t("Leave Group")}
                    busy={*busy}
                    on_confirm={Callback::from(move |_: ()| send.emit((
                        Asking::Change,
                        ActionMaker::new(move |done| Action::LeaveGroup { chat_id, done }),
                    )))}
                    on_cancel={back.clone()}
                />
            })
        }
        Step::ConfirmDelete => {
            let send = send.clone();
            Some(html! {
                <Confirm
                    title={t("Delete Group")}
                    message={t1("Delete “%@”? Its messages are deleted for everyone.", &name)}
                    confirm={t("Delete Group")}
                    busy={*busy}
                    on_confirm={Callback::from(move |_: ()| send.emit((
                        Asking::Change,
                        ActionMaker::new(move |done| Action::DeleteGroup { chat_id, done }),
                    )))}
                    on_cancel={back.clone()}
                />
            })
        }
        _ => None,
    };

    html! {
        <>
            <Modal title={t("Group Info")} class={classes!("group-dialog")} on_cancel={props.on_close.clone()} busy={*busy}>
                { body }
            </Modal>
            { confirm.unwrap_or_default() }
        </>
    }
}

/// How an action is made once its `done` exists — a request built in a
/// click and finished by the dialog's one way of hearing back.
#[derive(Clone)]
struct ActionMaker(std::rc::Rc<dyn Fn(crate::actions::Done) -> Action>);

impl ActionMaker {
    fn new(make: impl Fn(crate::actions::Done) -> Action + 'static) -> Self {
        ActionMaker(std::rc::Rc::new(make))
    }
}

#[derive(Properties, PartialEq)]
struct AddPicksProps {
    candidates: Vec<Member>,
    chosen: Vec<i64>,
    on_change: Callback<Vec<i64>>,
}

/// The members who could be added, a checkbox each, in the roster's order.
#[function_component(AddPicks)]
fn add_picks(props: &AddPicksProps) -> Html {
    let chosen: HashSet<i64> = props.chosen.iter().copied().collect();
    html! {
        <ul class="members group-picks">
            { for props.candidates.iter().map(|member| {
                let id = member.id;
                let toggle = {
                    let now = props.chosen.clone();
                    let on_change = props.on_change.clone();
                    Callback::from(move |_: Event| {
                        let mut now = now.clone();
                        if let Some(at) = now.iter().position(|held| *held == id) {
                            now.remove(at);
                        } else {
                            now.push(id);
                        }
                        on_change.emit(now);
                    })
                };
                html! {
                    <li class="member-row" key={id.to_string()}>
                        <label class="check">
                            <input type="checkbox" checked={chosen.contains(&id)} onchange={toggle} />
                            <Avatar title={member.display_name.clone()} user_id={Some(id)} version={member.avatar_version} size={28} />
                            <span>{ member.display_name.clone() }</span>
                        </label>
                    </li>
                }
            }) }
        </ul>
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gloo_timers::future::TimeoutFuture;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;
    use web_sys::{Element, HtmlElement};

    use super::*;

    const ME: i64 = 7;
    const ANNA: i64 = 9;
    const BOB: i64 = 11;

    fn member(id: i64, name: &str) -> Member {
        Member {
            id,
            display_name: name.into(),
            username: name.to_lowercase(),
            role: Some("member".into()),
            ..Member::default()
        }
    }

    fn family() -> Vec<Member> {
        vec![
            member(ME, "Me"),
            member(ANNA, "Anna"),
            member(BOB, "Bob"),
            Member {
                deleted: true,
                ..member(12, "Deleted account")
            },
        ]
    }

    fn group(members: &[i64], admin: i64) -> Chat {
        Chat {
            id: 77,
            kind: "group".into(),
            title: Some("Cousins".into()),
            member_ids: Some(members.to_vec()),
            admin_user_id: Some(admin),
            ..Chat::default()
        }
    }

    fn root() -> HtmlElement {
        let document = web_sys::window().unwrap().document().unwrap();
        let root: HtmlElement = document.create_element("div").unwrap().dyn_into().unwrap();
        document.body().unwrap().append_child(&root).unwrap();
        root
    }

    fn log() -> (Rc<RefCell<Vec<Action>>>, Callback<Action>) {
        let log = Rc::new(RefCell::new(Vec::new()));
        let sink = log.clone();
        (
            log,
            Callback::from(move |action| sink.borrow_mut().push(action)),
        )
    }

    fn texts(root: &Element, selector: &str) -> Vec<String> {
        let found = root.query_selector_all(selector).unwrap();
        (0..found.length())
            .filter_map(|at| found.item(at))
            .map(|node| node.text_content().unwrap_or_default().trim().to_string())
            .collect()
    }

    fn click(root: &Element, selector: &str, label: &str) {
        let found = root.query_selector_all(selector).unwrap();
        for at in 0..found.length() {
            let element: HtmlElement = found.item(at).unwrap().dyn_into().unwrap();
            if element.text_content().unwrap_or_default().trim() == label {
                element.click();
                return;
            }
        }
        panic!("no {selector} reading {label:?}");
    }

    fn type_name(root: &Element, value: &str) {
        let input: web_sys::HtmlInputElement = root
            .query_selector("input[type=text]")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        input.set_value(value);
        let init = web_sys::EventInit::new();
        init.set_bubbles(true);
        input
            .dispatch_event(&web_sys::Event::new_with_event_init_dict("input", &init).unwrap())
            .unwrap();
    }

    fn done_of(action: &Action) -> crate::actions::Done {
        match action {
            Action::CreateGroup { done, .. }
            | Action::RenameGroup { done, .. }
            | Action::AddGroupMembers { done, .. }
            | Action::RemoveGroupMember { done, .. }
            | Action::LeaveGroup { done, .. }
            | Action::DeleteGroup { done, .. } => done.clone(),
            other => panic!("not a group action: {other:?}"),
        }
    }

    fn refusal(code: &str) -> ApiError {
        ApiError::Server {
            code: code.into(),
            message: String::new(),
        }
    }

    async fn settle() {
        TimeoutFuture::new(20).await;
    }

    /// A refusal is the server's code in the shared table's words; the
    /// network, a busy or troubled server is said as what it is, never as
    /// "couldn't create the group".
    #[wasm_bindgen_test]
    fn a_transient_failure_is_not_a_refusal() {
        assert_eq!(
            failure(&refusal("too_many_groups"), Asking::Create),
            "Your family already has 50 groups."
        );
        assert_eq!(
            failure(&refusal("not_same_family"), Asking::Create),
            "Couldn't create the group."
        );
        assert_eq!(
            failure(&refusal("not_group_admin"), Asking::Change),
            "Only the group's admin can do this."
        );
        assert_eq!(
            failure(&refusal("validation"), Asking::Rename),
            "A group's name can be 1 to 64 characters."
        );
        assert_eq!(
            failure(&ApiError::Network("down".into()), Asking::Create),
            "Can't reach the server. Check your connection."
        );
        assert_eq!(
            failure(&refusal("internal"), Asking::Change),
            "The server had a problem. Try again in a moment."
        );
        assert_eq!(
            failure(&ApiError::Answered { status: 502 }, Asking::Create),
            "The server had a problem. Try again in a moment."
        );
    }

    /// Who may be put in a group: the family now, not me, not a deleted
    /// account, not anybody already in it.
    #[wasm_bindgen_test]
    fn who_may_be_added() {
        let ids = |members: Vec<Member>| members.iter().map(|m| m.id).collect::<Vec<_>>();
        assert_eq!(ids(addable(&family(), ME, &[])), vec![ANNA, BOB]);
        assert_eq!(ids(addable(&family(), ME, &[ME, ANNA])), vec![BOB]);
        assert!(addable(&family(), ME, &[ME, ANNA, BOB]).is_empty());
    }

    /// NEW GROUP checks before it sends — the name, then somebody chosen —
    /// sends the name trimmed with the members picked, says a refusal in the
    /// dialog, and closes once the group is made.
    #[wasm_bindgen_test]
    async fn new_group_checks_then_sends_then_says_what_happened() {
        let root = root();
        let (actions, on_action) = log();
        let closed = Rc::new(RefCell::new(0));
        let on_close = {
            let closed = closed.clone();
            Callback::from(move |_: ()| *closed.borrow_mut() += 1)
        };
        let handle = yew::Renderer::<NewGroupDialog>::with_root_and_props(
            root.clone().into(),
            NewGroupProps {
                members: family(),
                my_user_id: ME,
                on_action,
                on_close,
            },
        )
        .render();
        settle().await;
        assert_eq!(texts(&root, "h2"), vec!["New Group"]);
        // Everybody who could be in it, and nobody else.
        assert_eq!(
            texts(&root, ".group-picks .check > span:last-child"),
            vec!["Anna", "Bob"]
        );

        click(&root, "button[type=submit]", "Create Group");
        settle().await;
        assert_eq!(
            texts(&root, ".error"),
            vec!["A group's name can be 1 to 64 characters."]
        );
        type_name(&root, &"x".repeat(65));
        settle().await;
        click(&root, "button[type=submit]", "Create Group");
        settle().await;
        assert_eq!(
            texts(&root, ".error"),
            vec!["A group's name can be 1 to 64 characters."]
        );
        type_name(&root, "  Cousins  ");
        settle().await;
        click(&root, "button[type=submit]", "Create Group");
        settle().await;
        assert_eq!(texts(&root, ".error"), vec!["Choose at least one member."]);
        assert!(actions.borrow().is_empty(), "nothing sent yet");

        root.query_selector(".group-picks input[type=checkbox]")
            .unwrap()
            .unwrap()
            .dyn_into::<HtmlElement>()
            .unwrap()
            .click();
        settle().await;
        click(&root, "button[type=submit]", "Create Group");
        settle().await;
        let sent = actions.borrow().last().cloned().expect("sent");
        assert!(
            matches!(&sent, Action::CreateGroup { title, member_ids, .. }
                if title == "Cousins" && member_ids == &vec![ANNA]),
            "{sent:?}"
        );
        assert!(texts(&root, ".error").is_empty());

        done_of(&sent).emit(Some(refusal("too_many_groups")));
        settle().await;
        assert_eq!(
            texts(&root, ".error"),
            vec!["Your family already has 50 groups."]
        );
        assert_eq!(*closed.borrow(), 0);
        done_of(&sent).emit(None);
        settle().await;
        assert_eq!(*closed.borrow(), 1, "made: the dialog goes");
        handle.destroy();
        root.remove();
    }

    async fn info(
        me: i64,
        chat: Chat,
    ) -> (
        HtmlElement,
        yew::AppHandle<GroupInfoDialog>,
        Rc<RefCell<Vec<Action>>>,
    ) {
        let root = root();
        let (actions, on_action) = log();
        let handle = yew::Renderer::<GroupInfoDialog>::with_root_and_props(
            root.clone().into(),
            GroupInfoProps {
                chat,
                members: family(),
                my_user_id: me,
                on_action,
                on_close: Callback::noop(),
            },
        )
        .render();
        settle().await;
        (root, handle, actions)
    }

    /// GROUP INFO: the name, how many are in it, each of them with the
    /// admin marked — and the admin's tools for the admin alone, Leave for
    /// every member.
    #[wasm_bindgen_test]
    async fn group_info_offers_the_admin_their_tools_and_everybody_leave() {
        let (root, handle, _) = info(ME, group(&[ME, ANNA, BOB], ME)).await;
        assert_eq!(texts(&root, "h2"), vec!["Group Info"]);
        assert_eq!(texts(&root, ".group-name"), vec!["Cousins"]);
        assert_eq!(texts(&root, "h3"), vec!["Members: 3"]);
        assert_eq!(
            texts(&root, ".group-members strong"),
            vec!["Me", "Anna", "Bob"]
        );
        assert_eq!(texts(&root, ".group-members .capsule"), vec!["Admin"]);
        assert_eq!(
            texts(&root, ".group-tools button"),
            vec!["Rename Group", "Add Members", "Leave Group", "Delete Group"]
        );
        assert_eq!(
            texts(&root, ".group-remove"),
            vec!["Remove from Group", "Remove from Group"],
            "everybody but the admin themself"
        );
        handle.destroy();
        root.remove();

        let (root, handle, _) = info(ANNA, group(&[ME, ANNA, BOB], ME)).await;
        assert_eq!(texts(&root, ".group-tools button"), vec!["Leave Group"]);
        assert!(texts(&root, ".group-remove").is_empty());
        handle.destroy();
        root.remove();
    }

    /// Each tool sends what it says, and a refusal is said where it was
    /// asked: removing, renaming (checked first), adding — with the line
    /// that says when nobody is left to add.
    #[wasm_bindgen_test]
    async fn group_info_tools_send_what_they_say() {
        let (root, handle, actions) = info(ME, group(&[ME, ANNA], ME)).await;
        click(&root, ".group-remove", "Remove from Group");
        settle().await;
        let sent = actions.borrow().last().cloned().expect("sent");
        assert!(matches!(
            sent,
            Action::RemoveGroupMember {
                chat_id: 77,
                user_id: ANNA,
                ..
            }
        ));
        done_of(&sent).emit(Some(refusal("not_group_admin")));
        settle().await;
        assert_eq!(
            texts(&root, ".error"),
            vec!["Only the group's admin can do this."]
        );

        click(&root, ".group-tools button", "Rename Group");
        settle().await;
        type_name(&root, "   ");
        settle().await;
        click(&root, "button[type=submit]", "Rename Group");
        settle().await;
        assert_eq!(
            texts(&root, ".error"),
            vec!["A group's name can be 1 to 64 characters."]
        );
        type_name(&root, " The cousins ");
        settle().await;
        click(&root, "button[type=submit]", "Rename Group");
        settle().await;
        let sent = actions.borrow().last().cloned().expect("sent");
        assert!(
            matches!(&sent, Action::RenameGroup { chat_id: 77, title, .. } if title == "The cousins"),
            "{sent:?}"
        );
        done_of(&sent).emit(None);
        settle().await;
        assert_eq!(
            texts(&root, "h3"),
            vec!["Members: 2"],
            "back to the members"
        );

        click(&root, ".group-tools button", "Add Members");
        settle().await;
        assert_eq!(
            texts(&root, ".group-picks .check > span:last-child"),
            vec!["Bob"]
        );
        click(&root, "button[type=submit]", "Add Members");
        settle().await;
        assert_eq!(texts(&root, ".error"), vec!["Choose at least one member."]);
        root.query_selector(".group-picks input[type=checkbox]")
            .unwrap()
            .unwrap()
            .dyn_into::<HtmlElement>()
            .unwrap()
            .click();
        settle().await;
        click(&root, "button[type=submit]", "Add Members");
        settle().await;
        let sent = actions.borrow().last().cloned().expect("sent");
        assert!(
            matches!(&sent, Action::AddGroupMembers { chat_id: 77, user_ids, .. } if user_ids == &vec![BOB]),
            "{sent:?}"
        );
        handle.destroy();
        root.remove();

        let (root, handle, _) = info(ME, group(&[ME, ANNA, BOB], ME)).await;
        click(&root, ".group-tools button", "Add Members");
        settle().await;
        assert_eq!(
            texts(&root, ".footnote"),
            vec!["Everyone in the family is already in this group."]
        );
        assert!(root
            .query_selector("button[type=submit]")
            .unwrap()
            .is_none());
        handle.destroy();
        root.remove();
    }

    /// Leave and Delete each ask first, in the app's own dialog, saying
    /// what it means — and only then send.
    #[wasm_bindgen_test]
    async fn leaving_and_deleting_ask_first() {
        let (root, handle, actions) = info(ME, group(&[ME, ANNA], ME)).await;
        click(&root, ".group-tools button", "Delete Group");
        settle().await;
        assert_eq!(
            texts(&root, ".dialog-message"),
            vec!["Delete “Cousins”? Its messages are deleted for everyone."]
        );
        assert!(actions.borrow().is_empty(), "asked, not sent");
        click(&root, ".dialog-actions button", "Delete Group");
        settle().await;
        assert!(matches!(
            actions.borrow().last(),
            Some(Action::DeleteGroup { chat_id: 77, .. })
        ));
        handle.destroy();
        root.remove();

        let (root, handle, actions) = info(ANNA, group(&[ME, ANNA], ME)).await;
        click(&root, ".group-tools button", "Leave Group");
        settle().await;
        assert_eq!(
            texts(&root, ".dialog-message"),
            vec!["Leave “Cousins”? You can come back only if its admin adds you again."]
        );
        click(&root, ".dialog-actions button", "Cancel");
        settle().await;
        assert!(root.query_selector(".dialog-message").unwrap().is_none());
        assert!(actions.borrow().is_empty());
        click(&root, ".group-tools button", "Leave Group");
        settle().await;
        click(&root, ".dialog-actions button", "Leave Group");
        settle().await;
        let sent = actions.borrow().last().cloned().expect("sent");
        assert!(matches!(sent, Action::LeaveGroup { chat_id: 77, .. }));
        // Refused, the sentence is said back on the members.
        done_of(&sent).emit(Some(ApiError::Network("down".into())));
        settle().await;
        assert_eq!(
            texts(&root, ".error"),
            vec!["Can't reach the server. Check your connection."]
        );
        assert!(root.query_selector(".dialog-message").unwrap().is_none());
        handle.destroy();
        root.remove();
    }
}
