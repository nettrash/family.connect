//! Settings: who is signed in, their picture, birthday and password, the
//! family they are in and the way out of it, the family's numbers, and the
//! account itself (ios MacSettingsView, with the iPhone's picture controls —
//! the Mac has none).
//!
//! Leaving is built from a FRESH read of the roster, never the one held:
//! who inherits, and whether anybody is left to, may have changed since
//! (docs/protocol.md, `POST /families/leave`) — and when the roster moves
//! while the dialog is open, it is built again. A read that FAILED is not a
//! family with nobody left in it: that dialog says leaving deletes the
//! family, and it is shown only when the server said so.

use fc_text::media::display_size;
use wasm_bindgen_futures::spawn_local;
use web_sys::HtmlInputElement;
use yew::prelude::*;

use crate::actions::{Action, LeaveContext};
use crate::api::ApiError;
use crate::model::{Family, Me, MemberStats, Stats};
use crate::prep;
use crate::time;
use crate::views::avatar::Avatar;
use crate::views::birthday::BirthdayDialog;
use crate::views::dialog::{Confirm, Modal};
use crate::views::password::ChangePasswordDialog;

pub const PRIVACY_URL: &str = "https://nettrash.me/appstore/familyconnect/privacy.html";
pub const SUPPORT_URL: &str = "https://nettrash.me/appstore/familyconnect/support.html";

#[derive(Properties, PartialEq)]
pub struct SettingsProps {
    pub account: Me,
    pub family: Option<Family>,
    /// Bumped by every frame that changes who is in the family or owns it.
    pub roster_changes: u64,
    pub on_action: Callback<Action>,
    pub on_close: Callback<()>,
    /// Asks the app to sign out — which asks first.
    pub on_sign_out: Callback<()>,
}

#[derive(Clone, PartialEq)]
enum Open {
    Nothing,
    Birthday,
    Password,
    Statistics,
    Delete,
}

#[derive(Clone, PartialEq)]
enum Leave {
    Closed,
    /// The fresh roster is being read.
    Reading,
    Ready(LeaveContext),
    Leaving(LeaveContext),
}

/// What the leave dialog says (ios MacSettingsView).
pub fn leave_message(context: &LeaveContext) -> String {
    match context {
        LeaveContext::Member => "You'll lose access to the family chat and your direct chats. Your history returns if you rejoin.".to_string(),
        LeaveContext::Successor(name) => format!("{name} becomes the owner. You'll lose access to the family chat and your direct chats; your history returns if you rejoin."),
        LeaveContext::LastMember => "You're the only member left. Leaving deletes the family and everything in it.".to_string(),
    }
}

const LEAVE_FAILED: &str = "Couldn't leave right now. Try again.";

/// Leaving as the last member DELETES the family, which the protocol makes
/// "a different dialog and a different confirmation" (`next_owner_user_id`)
/// — not the ordinary leave with one sentence changed, which is the Mac's.
pub fn leave_title(context: &LeaveContext) -> &'static str {
    match context {
        LeaveContext::LastMember => "Delete the family?",
        _ => "Leave the family?",
    }
}

pub fn leave_button(context: &LeaveContext) -> &'static str {
    match context {
        LeaveContext::LastMember => "Leave and Delete",
        _ => "Leave Family",
    }
}

#[function_component(SettingsPane)]
pub fn settings_pane(props: &SettingsProps) -> Html {
    let open = use_state(|| Open::Nothing);
    let leave = use_state(|| Leave::Closed);
    let leave_error = use_state(|| Option::<String>::None);
    // Only the latest read may build the dialog: a slow answer from before
    // ownership moved is the one that must not.
    let leave_reads = use_mut_ref(|| 0u64);
    let picture_busy = use_state(|| false);
    let picture_error = use_state(|| Option::<String>::None);

    let read_leave = {
        let leave = leave.clone();
        let leave_error = leave_error.clone();
        let leave_reads = leave_reads.clone();
        let on_action = props.on_action.clone();
        Callback::from(move |_: ()| {
            let this_read = {
                let mut reads = leave_reads.borrow_mut();
                *reads += 1;
                *reads
            };
            leave.set(Leave::Reading);
            leave_error.set(None);
            let leave = leave.clone();
            let leave_error = leave_error.clone();
            let leave_reads = leave_reads.clone();
            on_action.emit(Action::ReadLeaveContext {
                done: Callback::from(move |answer: Result<LeaveContext, ApiError>| {
                    if *leave_reads.borrow() != this_read {
                        return;
                    }
                    match answer {
                        Ok(context) => leave.set(Leave::Ready(context)),
                        Err(_) => {
                            leave.set(Leave::Closed);
                            leave_error.set(Some(LEAVE_FAILED.to_string()));
                        }
                    }
                }),
            });
        })
    };
    // The roster moved while the dialog was up — ownership, or somebody
    // joining or leaving: who inherits, and what leaving means, may have
    // changed with it — read again, and say it again.
    {
        let leave = leave.clone();
        let read_leave = read_leave.clone();
        use_effect_with(props.roster_changes, move |_| {
            if matches!(*leave, Leave::Ready(_) | Leave::Reading) {
                read_leave.emit(());
            }
        });
    }

    let account = &props.account;
    let me = &account.user;
    let show = |what: Open| {
        let open = open.clone();
        Callback::from(move |_: MouseEvent| open.set(what.clone()))
    };
    let close_open = {
        let open = open.clone();
        Callback::from(move |_: ()| open.set(Open::Nothing))
    };

    let pick_picture = {
        let busy = picture_busy.clone();
        let error = picture_error.clone();
        let on_action = props.on_action.clone();
        Callback::from(move |event: Event| {
            let input: HtmlInputElement = event.target_unchecked_into();
            let Some(file) = input.files().and_then(|files| files.get(0)) else {
                return;
            };
            // The same file chosen twice is a change the second time too.
            input.set_value("");
            if *busy {
                return;
            }
            busy.set(true);
            error.set(None);
            let busy = busy.clone();
            let error = error.clone();
            let on_action = on_action.clone();
            spawn_local(async move {
                match prep::avatar(&file).await {
                    Ok(jpeg) => on_action.emit(Action::SetAvatar {
                        jpeg: Some(jpeg),
                        done: Callback::from(move |failure: Option<ApiError>| {
                            busy.set(false);
                            error.set(failure.map(|failure| picture_failure(&failure, true)));
                        }),
                    }),
                    Err(_) => {
                        busy.set(false);
                        error.set(Some("That image couldn't be read.".to_string()));
                    }
                }
            });
        })
    };
    let remove_picture = {
        let busy = picture_busy.clone();
        let error = picture_error.clone();
        let on_action = props.on_action.clone();
        Callback::from(move |_: MouseEvent| {
            if *busy {
                return;
            }
            busy.set(true);
            error.set(None);
            let busy = busy.clone();
            let error = error.clone();
            on_action.emit(Action::SetAvatar {
                jpeg: None,
                done: Callback::from(move |failure: Option<ApiError>| {
                    busy.set(false);
                    error.set(failure.map(|failure| picture_failure(&failure, false)));
                }),
            });
        })
    };

    let start_leave = {
        let read_leave = read_leave.clone();
        Callback::from(move |_: MouseEvent| read_leave.emit(()))
    };
    let cancel_leave = {
        let leave = leave.clone();
        let leave_reads = leave_reads.clone();
        let leave_error = leave_error.clone();
        Callback::from(move |_: ()| {
            *leave_reads.borrow_mut() += 1;
            leave.set(Leave::Closed);
            leave_error.set(None);
        })
    };
    let confirm_leave = {
        let leave = leave.clone();
        let leave_error = leave_error.clone();
        let on_action = props.on_action.clone();
        Callback::from(move |_: ()| {
            let Leave::Ready(context) = (*leave).clone() else {
                return;
            };
            leave.set(Leave::Leaving(context.clone()));
            let leave = leave.clone();
            let leave_error = leave_error.clone();
            on_action.emit(Action::LeaveFamily {
                done: Callback::from(move |answer: Result<Option<String>, ApiError>| {
                    // Not the dialog again: what it said was read before
                    // the attempt, and may not be true now. Leaving again
                    // reads afresh.
                    if answer.is_err() {
                        leave.set(Leave::Closed);
                        leave_error.set(Some(LEAVE_FAILED.to_string()));
                    }
                }),
            });
        })
    };

    let birthday = me
        .birthday
        .map(|held| time::birthday(held.month, held.day))
        .unwrap_or_else(|| "Not set".to_string());
    let has_picture = me.avatar_version > 0;
    let close = props.on_close.reform(|_: MouseEvent| ());

    let leave_dialog = match &*leave {
        Leave::Ready(context) | Leave::Leaving(context) => html! {
            <Confirm
                title={leave_title(context)}
                message={leave_message(context)}
                confirm={leave_button(context)}
                busy={matches!(*leave, Leave::Leaving(_))}
                error={(*leave_error).clone()}
                on_confirm={confirm_leave}
                on_cancel={cancel_leave}
            />
        },
        _ => Html::default(),
    };
    let dialog = match &*open {
        Open::Nothing => Html::default(),
        Open::Birthday => html! {
            <BirthdayDialog
                current={me.birthday}
                on_action={props.on_action.clone()}
                on_close={close_open.clone()}
            />
        },
        Open::Password => html! {
            <ChangePasswordDialog on_action={props.on_action.clone()} on_close={close_open.clone()} />
        },
        Open::Statistics => html! {
            <StatisticsDialog on_action={props.on_action.clone()} on_close={close_open.clone()} />
        },
        Open::Delete => html! {
            <DeleteAccountDialog
                owner={account.is_owner()}
                on_action={props.on_action.clone()}
                on_close={close_open.clone()}
            />
        },
    };

    html! {
        <section class="pane settings-pane" aria-labelledby="settings-title">
            <header class="pane-head">
                <h2 id="settings-title">{ "Settings" }</h2>
                <button class="link" onclick={close}>{ "Done" }</button>
            </header>
            <div class="pane-body">
                <div class="identity">
                    <Avatar title={me.display_name.clone()} user_id={Some(me.id)} version={me.avatar_version} size={56} />
                    <div class="identity-text">
                        <strong>{ me.display_name.clone() }</strong>
                        <span class="muted">{ format!("@{}", me.username) }</span>
                    </div>
                    if account.is_owner() {
                        <span class="capsule">{ "Owner" }</span>
                    }
                </div>

                <section class="group" aria-labelledby="settings-profile">
                    <h3 id="settings-profile">{ "Profile" }</h3>
                    <div class="setting-row">
                        <span>{ "Photo" }</span>
                        <span class="row-actions">
                            <label class={classes!("button-like", picture_busy.then_some("is-disabled"))}>
                                { if has_picture { "Change Photo" } else { "Add Photo" } }
                                <input
                                    type="file"
                                    accept="image/*"
                                    class="visually-hidden"
                                    disabled={*picture_busy}
                                    onchange={pick_picture}
                                />
                            </label>
                            if has_picture {
                                <button class="link danger" disabled={*picture_busy} onclick={remove_picture}>
                                    { "Remove Photo" }
                                </button>
                            }
                        </span>
                    </div>
                    if let Some(message) = (*picture_error).clone() {
                        <p class="error" role="alert">{ message }</p>
                    }
                    <div class="setting-row">
                        <span>{ "Birthday" }</span>
                        <span class="row-actions">
                            <span class="muted">{ birthday }</span>
                            <button class="link" onclick={show(Open::Birthday)}>
                                { if me.birthday.is_some() { "Change Birthday…" } else { "Add Birthday…" } }
                            </button>
                        </span>
                    </div>
                    <div class="setting-row">
                        <button class="link" onclick={show(Open::Password)}>{ "Change Password…" }</button>
                    </div>
                </section>

                if let Some(family) = &props.family {
                    <section class="group" aria-labelledby="settings-family">
                        <h3 id="settings-family">{ "Family" }</h3>
                        <div class="setting-row">
                            <span>{ "Name" }</span>
                            <span class="muted">{ family.name.clone() }</span>
                        </div>
                        <div class="setting-row">
                            <button
                                class="link danger"
                                disabled={matches!(*leave, Leave::Reading | Leave::Leaving(_))}
                                onclick={start_leave}
                            >{ "Leave Family" }</button>
                        </div>
                        if let (Some(message), Leave::Closed) = ((*leave_error).clone(), &*leave) {
                            <p class="error" role="alert">{ message }</p>
                        }
                    </section>
                }

                <section class="group" aria-labelledby="settings-statistics">
                    <h3 id="settings-statistics">{ "Statistics" }</h3>
                    <div class="setting-row">
                        <button class="link" onclick={show(Open::Statistics)}>{ "Statistics…" }</button>
                    </div>
                </section>

                <section class="group" aria-labelledby="settings-privacy">
                    <h3 id="settings-privacy">{ "Privacy" }</h3>
                    <div class="setting-row">
                        <a href={PRIVACY_URL} target="_blank" rel="noopener noreferrer">{ "Privacy Policy" }</a>
                    </div>
                    <div class="setting-row">
                        <a href={SUPPORT_URL} target="_blank" rel="noopener noreferrer">{ "Support" }</a>
                    </div>
                </section>

                <section class="group">
                    <div class="setting-row">
                        <button class="link" onclick={props.on_sign_out.reform(|_: MouseEvent| ())}>{ "Sign Out" }</button>
                    </div>
                    <div class="setting-row">
                        <button class="link danger" onclick={show(Open::Delete)}>{ "Delete Account…" }</button>
                    </div>
                </section>

                <p class="version">{ format!("Family Connect for the web {}", env!("CARGO_PKG_VERSION")) }</p>
            </div>
            { leave_dialog }
            { dialog }
        </section>
    }
}

/// Why a picture did not go up, or come down (ios AvatarFailure).
pub fn picture_failure(error: &ApiError, uploading: bool) -> String {
    match error.code() {
        Some("avatar_too_large") | Some(crate::api::TOO_LARGE) => {
            "That photo is too large for this server.".to_string()
        }
        Some("invalid_image") => "That file isn't a photo we can use.".to_string(),
        _ => match error {
            ApiError::Network(_) => "Can't reach the server. Check your connection.".to_string(),
            ApiError::Throttled { .. } => error.detail(),
            _ if uploading => "Couldn't upload the photo.".to_string(),
            _ => "Couldn't remove the photo.".to_string(),
        },
    }
}

#[derive(Properties, PartialEq)]
struct DeleteProps {
    owner: bool,
    on_action: Callback<Action>,
    on_close: Callback<()>,
}

/// Deleting the account: what happens, said before it does; the password,
/// because being signed in is not proof; and one last question.
#[function_component(DeleteAccountDialog)]
fn delete_account_dialog(props: &DeleteProps) -> Html {
    let password = use_state(String::new);
    let asking = use_state(|| false);
    let busy = use_state(|| false);
    let error = use_state(|| Option::<String>::None);
    let on_input = {
        let password = password.clone();
        Callback::from(move |event: InputEvent| {
            let input: HtmlInputElement = event.target_unchecked_into();
            password.set(input.value());
        })
    };
    let ask = {
        let asking = asking.clone();
        let password = password.clone();
        Callback::from(move |event: SubmitEvent| {
            event.prevent_default();
            if !password.is_empty() {
                asking.set(true);
            }
        })
    };
    let delete = {
        let (password, asking, busy, error) = (
            password.clone(),
            asking.clone(),
            busy.clone(),
            error.clone(),
        );
        let on_action = props.on_action.clone();
        Callback::from(move |_: ()| {
            if *busy {
                return;
            }
            busy.set(true);
            let (asking, busy, error) = (asking.clone(), busy.clone(), error.clone());
            on_action.emit(Action::DeleteAccount {
                password: (*password).clone(),
                done: Callback::from(move |failure: Option<ApiError>| {
                    busy.set(false);
                    asking.set(false);
                    if let Some(failure) = failure {
                        error.set(Some(match failure.code() {
                            Some("invalid_credentials") => {
                                "That password is not right.".to_string()
                            }
                            Some("validation") => "Type your password to confirm.".to_string(),
                            _ => "Couldn't delete your account. Try again.".to_string(),
                        }));
                    }
                }),
            });
        })
    };
    let back = {
        let asking = asking.clone();
        Callback::from(move |_: ()| asking.set(false))
    };
    let cancel = props.on_close.reform(|_: MouseEvent| ());
    html! {
        <>
            <Modal title="Delete Account" on_cancel={props.on_close.clone()} busy={*busy}>
                <form class="dialog-form" onsubmit={ask}>
                    <h3>{ "What happens" }</h3>
                    <ul class="consequences">
                        <li>{ "Your account, password, profile picture and birthday are deleted, and every device you are signed in on is signed out." }</li>
                        <li>{ "Your direct chats are deleted — for the other person too. So is your private chat with the assistant." }</li>
                        <li>{ "Your messages in the family chat, your board notes and your reactions stay. They are shown from then on as “Deleted account”." }</li>
                        if props.owner {
                            <li>{ "You own this family: ownership passes to the longest-standing remaining member. If you are its last member, the family is deleted with you — its chat, its board and its invite code." }</li>
                        }
                    </ul>
                    <p class="footnote">{ "There is no grace period and no way to cancel afterwards." }</p>
                    <label class="field">
                        { "Password" }
                        <input type="password" autocomplete="current-password" value={(*password).clone()} oninput={on_input} />
                    </label>
                    <p class="footnote">{ "Type your password to confirm it is you. Being signed in is not proof." }</p>
                    if let Some(message) = (*error).clone() {
                        <p class="error" role="alert">{ message }</p>
                    }
                    <div class="dialog-actions">
                        <button type="button" class="secondary" disabled={*busy} onclick={cancel}>{ "Cancel" }</button>
                        <button type="submit" class="danger-button" disabled={*busy || password.is_empty()}>
                            { "Delete" }
                        </button>
                    </div>
                </form>
            </Modal>
            if *asking {
                <Confirm
                    title="Delete your account?"
                    message="This happens immediately and cannot be undone."
                    confirm="Delete Account"
                    busy={*busy}
                    on_confirm={delete}
                    on_cancel={back}
                />
            }
        </>
    }
}

#[derive(Properties, PartialEq)]
struct StatisticsProps {
    on_action: Callback<Action>,
    on_close: Callback<()>,
}

#[derive(Clone, PartialEq)]
enum Numbers {
    Loading,
    Failed,
    Shown(Box<Stats>),
}

/// The family's numbers, read afresh on every opening. The rows are never
/// added up into the totals: a member the reader blocked is left out of the
/// rows and not of the totals (docs/protocol.md, `GET /families/mine/stats`).
#[function_component(StatisticsDialog)]
fn statistics_dialog(props: &StatisticsProps) -> Html {
    let numbers = use_state(|| Numbers::Loading);
    let load = {
        let numbers = numbers.clone();
        let on_action = props.on_action.clone();
        Callback::from(move |_: ()| {
            numbers.set(Numbers::Loading);
            let numbers = numbers.clone();
            on_action.emit(Action::LoadStats {
                done: Callback::from(move |answer: Result<Stats, ApiError>| {
                    numbers.set(match answer {
                        Ok(stats) => Numbers::Shown(Box::new(stats)),
                        Err(_) => Numbers::Failed,
                    })
                }),
            });
        })
    };
    {
        let load = load.clone();
        use_effect_with((), move |_| load.emit(()));
    }
    let retry = load.reform(|_: MouseEvent| ());
    let close = props.on_close.reform(|_: MouseEvent| ());
    let body = match &*numbers {
        Numbers::Loading => html! { <p class="muted" role="status">{ "Loading…" }</p> },
        Numbers::Failed => html! {
            <div class="unavailable">
                <strong>{ "Couldn't load statistics" }</strong>
                <p class="muted">{ "Check your connection and try again." }</p>
                <button onclick={retry}>{ "Retry" }</button>
            </div>
        },
        Numbers::Shown(stats) => statistics(stats),
    };
    html! {
        <Modal title="Statistics" class={classes!("statistics")} on_cancel={props.on_close.clone()}>
            { body }
            <div class="dialog-actions">
                <button class="primary" onclick={close}>{ "Done" }</button>
            </div>
        </Modal>
    }
}

fn number_row(label: &str, value: String) -> Html {
    html! {
        <div class="setting-row">
            <span>{ label.to_string() }</span>
            <span class="number">{ value }</span>
        </div>
    }
}

fn statistics(stats: &Stats) -> Html {
    let totals = &stats.totals;
    let files = &totals.attachments;
    let ai = &totals.ai;
    let sent = files.bytes.max(0) as u64;
    let saved = files
        .stored_bytes
        .map(|stored| sent.saturating_sub(stored.max(0) as u64))
        .filter(|saved| *saved > 0);
    html! {
        <>
            <section class="group">
                <h3>{ "The family" }</h3>
                { number_row("Members", totals.members.to_string()) }
                { number_row("Messages", totals.messages.to_string()) }
                { number_row("Board notes", totals.board_notes.to_string()) }
            </section>
            <section class="group">
                <h3>{ "Attachments" }</h3>
                { number_row("Photos", files.photo.to_string()) }
                { number_row("Videos", files.video.to_string()) }
                { number_row("Audio", files.audio.to_string()) }
                { number_row("Files", files.file.to_string()) }
                { number_row("Locations", files.location.to_string()) }
                { number_row("Sent", display_size(sent)) }
                if let Some(stored) = files.stored_bytes {
                    { number_row("On disk", display_size(stored.max(0) as u64)) }
                }
                if let Some(saved) = saved {
                    <p class="footnote">{ format!("{} saved by storing one copy of identical files.", display_size(saved)) }</p>
                }
            </section>
            if ai.questions > 0 || ai.images > 0 {
                <section class="group">
                    <h3>{ "Assistant" }</h3>
                    { number_row("Questions", ai.questions.to_string()) }
                    { number_row("Tokens", (ai.prompt_tokens + ai.completion_tokens).to_string()) }
                    if ai.images > 0 {
                        { number_row("Pictures", ai.images.to_string()) }
                    }
                </section>
            }
            <section class="group">
                <h3>{ "Who sends what" }</h3>
                { for stats.members.iter().map(|member| html! {
                    <div class="setting-row stat-member">
                        <span>
                            <strong>{ member.display_name.clone() }</strong>
                            <span class="footnote">{ member_line(member) }</span>
                        </span>
                        <span class="number">{ member.messages }</span>
                    </div>
                }) }
            </section>
        </>
    }
}

/// What one member sends, besides words (ios StatisticsView).
pub fn member_line(member: &MemberStats) -> String {
    let count = |n: i64, one: &str, many: &str| {
        if n == 1 {
            format!("1 {one}")
        } else {
            format!("{n} {many}")
        }
    };
    let mut parts = Vec::new();
    let files = &member.attachments;
    if files.count > 0 {
        parts.push(format!(
            "{}, {}",
            count(files.count, "attachment", "attachments"),
            display_size(files.bytes.max(0) as u64)
        ));
    }
    if member.ai.questions > 0 {
        parts.push(count(
            member.ai.questions,
            "question to the assistant",
            "questions to the assistant",
        ));
    }
    if member.ai.images > 0 {
        parts.push(count(
            member.ai.images,
            "picture from the assistant",
            "pictures from the assistant",
        ));
    }
    if parts.is_empty() {
        "Words only".to_string()
    } else {
        parts.join(" · ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AiCounts, AttachmentCounts};
    use wasm_bindgen_test::wasm_bindgen_test;

    #[wasm_bindgen_test]
    fn leaving_says_what_it_means_for_who_is_leaving() {
        assert_eq!(
            leave_message(&LeaveContext::Successor("Bob".into())),
            "Bob becomes the owner. You'll lose access to the family chat and your direct chats; your history returns if you rejoin."
        );
        assert!(leave_message(&LeaveContext::LastMember).contains("deletes the family"));
        assert!(!leave_message(&LeaveContext::Member).contains("deletes"));
        // Deleting the family is its own question, and its own button.
        assert_eq!(leave_title(&LeaveContext::LastMember), "Delete the family?");
        assert_eq!(leave_button(&LeaveContext::LastMember), "Leave and Delete");
        assert_eq!(
            leave_button(&LeaveContext::Successor("Bob".into())),
            "Leave Family"
        );
    }

    #[wasm_bindgen_test]
    fn a_members_line_counts_what_they_send_besides_words() {
        let mut member = MemberStats {
            user_id: 1,
            display_name: "Anna".into(),
            messages: 3,
            attachments: AttachmentCounts::default(),
            ai: AiCounts::default(),
        };
        assert_eq!(member_line(&member), "Words only");
        member.attachments.count = 1;
        member.attachments.bytes = 1_234_567;
        member.ai.questions = 2;
        member.ai.images = 1;
        assert_eq!(
            member_line(&member),
            "1 attachment, 1.2 MB · 2 questions to the assistant · 1 picture from the assistant"
        );
    }

    #[wasm_bindgen_test]
    fn a_picture_refused_is_said_for_what_it_was() {
        let server = |code: &str| ApiError::Server {
            code: code.into(),
            message: String::new(),
        };
        assert_eq!(
            picture_failure(&server("avatar_too_large"), true),
            "That photo is too large for this server."
        );
        assert_eq!(
            picture_failure(&server("internal"), false),
            "Couldn't remove the photo."
        );
    }
}
