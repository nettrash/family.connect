//! A poll in the family chat — its options and votes under the question
//! the bubble already draws as its body — and the composer that asks one
//! (docs/protocol.md, "Polls"; ios Views/PollBubbleView.swift,
//! Views/PollComposerView.swift, `PollPresentation`).

use std::collections::{HashMap, HashSet};

use yew::prelude::*;

use crate::model::{Message, Poll};

pub const MIN_OPTIONS: usize = 2;
pub const MAX_OPTIONS: usize = 10;
pub const MAX_OPTION_CHARS: usize = 100;

/// How many faces a voter row draws before "+N".
const MAX_FACES: usize = 5;

/// The option this reader holds, if any — one choice per member.
pub fn my_option(poll: &Poll, me: i64) -> Option<i64> {
    poll.options
        .iter()
        .find(|option| option.votes.contains(&me))
        .map(|option| option.id)
}

/// Everyone who has voted, counted once.
pub fn voter_count(poll: &Poll) -> usize {
    poll.options
        .iter()
        .flat_map(|option| option.votes.iter())
        .collect::<HashSet<_>>()
        .len()
}

/// How full one option's bar is: its share of the votes CAST, not of the
/// family — 0 while nobody has voted, so a poll opens with every bar empty.
pub fn fraction(poll: &Poll, votes: usize) -> f64 {
    let total: usize = poll.options.iter().map(|option| option.votes.len()).sum();
    if total == 0 {
        0.0
    } else {
        votes as f64 / total as f64
    }
}

/// The voters whose names may be DRAWN — minus anyone this reader has
/// blocked. Identity only: every count and bar goes on counting a blocked
/// member's vote, because a tally that moved when you blocked somebody
/// would tell you they had voted (docs/protocol.md, "Blocking a member").
pub fn drawable_voters(votes: &[i64], blocked: &HashSet<i64>) -> Vec<i64> {
    votes
        .iter()
        .copied()
        .filter(|voter| !blocked.contains(voter))
        .collect()
}

/// Check a poll the way the server will, so the composer refuses locally
/// what would come back `invalid_poll`: a question, 2–10 options, each
/// trimmed (Rust's `trim`, the server's own), non-empty, at most 100
/// characters, no two the same ignoring case (Rust's `to_lowercase`, the
/// server's own — not a locale's). Answers the options as they will be
/// sent, or what is wrong.
pub fn validate(question: &str, options: &[String]) -> Result<Vec<String>, &'static str> {
    if question.trim().is_empty() {
        return Err("Ask a question.");
    }
    let trimmed: Vec<String> = options
        .iter()
        .map(|option| option.trim().to_string())
        .filter(|option| !option.is_empty())
        .collect();
    if trimmed.len() < MIN_OPTIONS {
        return Err("Give at least two options.");
    }
    if trimmed.len() > MAX_OPTIONS {
        return Err("A poll has at most ten options.");
    }
    if trimmed
        .iter()
        .any(|option| option.chars().count() > MAX_OPTION_CHARS)
    {
        return Err("An option can be at most 100 characters.");
    }
    let mut seen = HashSet::new();
    if !trimmed
        .iter()
        .all(|option| seen.insert(option.to_lowercase()))
    {
        return Err("Two options are the same.");
    }
    Ok(trimmed)
}

#[derive(Properties, PartialEq)]
pub struct PollViewProps {
    pub message: Message,
    pub my_user_id: i64,
    /// The live roster's size — the M in "N of M voted".
    pub member_count: usize,
    pub names: HashMap<i64, String>,
    pub blocked: HashSet<i64>,
    /// (option id) to vote for it.
    pub on_vote: Callback<i64>,
    pub on_unvote: Callback<()>,
    pub on_close: Callback<()>,
}

#[function_component(PollView)]
pub fn poll_view(props: &PollViewProps) -> Html {
    let Some(poll) = props.message.poll.as_ref() else {
        return Html::default();
    };
    let me = props.my_user_id;
    let mine = my_option(poll, me);
    // A poll the server has not numbered yet cannot be voted on, and a
    // closed one is a result.
    let votable = props.message.id != 0 && !poll.closed;
    let voted = voter_count(poll);
    let footer = if props.member_count > 0 {
        format!("{voted} of {} voted", props.member_count)
    } else {
        format!("{voted} voted")
    };
    let is_author = props.message.sender_id == me;
    let name = |user: i64| -> String {
        if user == me {
            "You".to_string()
        } else {
            props
                .names
                .get(&user)
                .cloned()
                .unwrap_or_else(|| "Someone".to_string())
        }
    };
    html! {
        <div class={classes!("poll", poll.closed.then_some("is-closed"))}>
            { for poll.options.iter().map(|option| {
                let count = option.votes.len();
                let chosen = mine == Some(option.id);
                let width = format!("width:{:.1}%", fraction(poll, count) * 100.0);
                let voters = drawable_voters(&option.votes, &props.blocked);
                let shown: Vec<String> = voters.iter().take(MAX_FACES).map(|v| name(*v)).collect();
                let more = voters.len().saturating_sub(MAX_FACES);
                // Tapping the option you hold clears it; any other casts it.
                let onclick = {
                    let on_vote = props.on_vote.clone();
                    let on_unvote = props.on_unvote.clone();
                    let id = option.id;
                    Callback::from(move |_: MouseEvent| {
                        if chosen {
                            on_unvote.emit(());
                        } else {
                            on_vote.emit(id);
                        }
                    })
                };
                let label = if chosen {
                    format!("{}. {count} votes. Your choice", option.text)
                } else {
                    format!("{}. {count} votes", option.text)
                };
                html! {
                    <div class="poll-option">
                        <button
                            class={classes!("poll-choice", chosen.then_some("is-mine"))}
                            disabled={!votable}
                            aria-pressed={chosen.to_string()}
                            aria-label={label}
                            {onclick}
                        >
                            <span class="poll-bar" style={width}></span>
                            <span class="poll-text">{ &option.text }</span>
                            <span class="poll-count">{ count }</span>
                        </button>
                        if !shown.is_empty() {
                            <span class="poll-voters">
                                { shown.join(", ") }
                                if more > 0 { { format!(" +{more}") } }
                            </span>
                        }
                    </div>
                }
            }) }
            <div class="poll-foot">
                <span>{ footer }</span>
                if poll.closed {
                    <span class="poll-closed">{ "🔒 Poll closed" }</span>
                } else if is_author && votable {
                    <button
                        class="link"
                        title="Ends the poll. This cannot be undone."
                        onclick={let on_close = props.on_close.clone(); Callback::from(move |_: MouseEvent| on_close.emit(()))}
                    >
                        { "Close poll" }
                    </button>
                }
            </div>
        </div>
    }
}

#[derive(Properties, PartialEq)]
pub struct PollComposerProps {
    /// (question, options) — already checked.
    pub on_submit: Callback<(String, Vec<String>)>,
    pub on_cancel: Callback<()>,
}

#[function_component(PollComposer)]
pub fn poll_composer(props: &PollComposerProps) -> Html {
    let question = use_state(String::new);
    let options = use_state(|| vec![String::new(), String::new()]);
    let problem = use_state(|| Option::<&'static str>::None);

    let on_question = {
        let question = question.clone();
        Callback::from(move |event: InputEvent| {
            let input: web_sys::HtmlInputElement = event.target_unchecked_into();
            question.set(input.value());
        })
    };
    let edit_option = |index: usize| {
        let options = options.clone();
        Callback::from(move |event: InputEvent| {
            let input: web_sys::HtmlInputElement = event.target_unchecked_into();
            let mut next = (*options).clone();
            if let Some(slot) = next.get_mut(index) {
                *slot = input.value();
            }
            options.set(next);
        })
    };
    let remove_option = |index: usize| {
        let options = options.clone();
        Callback::from(move |_: MouseEvent| {
            if options.len() > MIN_OPTIONS {
                let mut next = (*options).clone();
                next.remove(index);
                options.set(next);
            }
        })
    };
    let add_option = {
        let options = options.clone();
        Callback::from(move |_: MouseEvent| {
            if options.len() < MAX_OPTIONS {
                let mut next = (*options).clone();
                next.push(String::new());
                options.set(next);
            }
        })
    };
    let submit = {
        let question = question.clone();
        let options = options.clone();
        let problem = problem.clone();
        let on_submit = props.on_submit.clone();
        Callback::from(move |_: MouseEvent| match validate(&question, &options) {
            Ok(checked) => on_submit.emit((question.trim().to_string(), checked)),
            Err(reason) => problem.set(Some(reason)),
        })
    };
    let cancel = {
        let on_cancel = props.on_cancel.clone();
        Callback::from(move |_: MouseEvent| on_cancel.emit(()))
    };
    html! {
        <div class="dialog-backdrop">
            <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="poll-title">
                <h2 id="poll-title">{ "New poll" }</h2>
                <label class="field">
                    <span>{ "Question" }</span>
                    <input value={(*question).clone()} oninput={on_question} maxlength="4000" />
                </label>
                <p class="footnote">{ "The question is the message everyone sees." }</p>
                <fieldset class="options">
                    <legend>{ "Options" }</legend>
                    { for options.iter().enumerate().map(|(index, option)| html! {
                        <div class="option-row">
                            <input
                                value={option.clone()}
                                oninput={edit_option(index)}
                                maxlength={MAX_OPTION_CHARS.to_string()}
                                aria-label={format!("Option {}", index + 1)}
                            />
                            if options.len() > MIN_OPTIONS {
                                <button class="link" onclick={remove_option(index)} aria-label="Remove option">{ "✕" }</button>
                            }
                        </div>
                    }) }
                    if options.len() < MAX_OPTIONS {
                        <button class="link" onclick={add_option}>{ "Add option" }</button>
                    }
                </fieldset>
                <p class="footnote">{ "Between 2 and 10 options. They can't be changed once the poll is sent." }</p>
                if let Some(reason) = *problem {
                    <p class="error" role="alert">{ reason }</p>
                }
                <div class="dialog-actions">
                    <button class="secondary" onclick={cancel}>{ "Cancel" }</button>
                    <button class="primary" onclick={submit}>{ "Send" }</button>
                </div>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PollOption;
    use wasm_bindgen_test::*;

    fn make_poll(votes: &[&[i64]]) -> Poll {
        Poll {
            poll_seq: 1,
            closed: false,
            options: votes
                .iter()
                .enumerate()
                .map(|(index, votes)| PollOption {
                    id: index as i64 + 1,
                    text: format!("Option {index}"),
                    votes: votes.to_vec(),
                })
                .collect(),
        }
    }

    #[wasm_bindgen_test]
    fn the_arithmetic_counts_everyone_and_the_names_skip_the_blocked() {
        let poll = make_poll(&[&[7, 9], &[11], &[]]);
        assert_eq!(my_option(&poll, 7), Some(1));
        assert_eq!(my_option(&poll, 4), None);
        assert_eq!(voter_count(&poll), 3);
        assert!((fraction(&poll, 2) - 2.0 / 3.0).abs() < 1e-9);
        assert_eq!(
            fraction(&make_poll(&[&[], &[]]), 0),
            0.0,
            "nobody voted: every bar empty"
        );
        let blocked = HashSet::from([9]);
        assert_eq!(drawable_voters(&poll.options[0].votes, &blocked), vec![7]);
        assert_eq!(
            voter_count(&poll),
            3,
            "a blocked member's vote still counts"
        );
    }

    /// The composer refuses exactly what the server would.
    #[wasm_bindgen_test]
    fn a_poll_is_checked_the_servers_way() {
        let options = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            validate("Dinner?", &options(&["  Pizza ", "Pasta"])),
            Ok(options(&["Pizza", "Pasta"]))
        );
        assert!(
            validate("  ", &options(&["a", "b"])).is_err(),
            "no question"
        );
        assert!(
            validate("Q", &options(&["a", "  "])).is_err(),
            "one real option"
        );
        assert!(
            validate("Q", &options(&["Pizza", "PIZZA"])).is_err(),
            "the same ignoring case"
        );
        assert!(
            validate("Q", &options(&["Ärger", "ärger"])).is_err(),
            "not only ASCII case"
        );
        let long = "я".repeat(MAX_OPTION_CHARS);
        assert!(
            validate("Q", &options(&[&long, "b"])).is_ok(),
            "100 characters, not bytes"
        );
        let longer = "я".repeat(MAX_OPTION_CHARS + 1);
        assert!(validate("Q", &options(&[&longer, "b"])).is_err());
        let eleven: Vec<String> = (0..11).map(|n| n.to_string()).collect();
        assert!(validate("Q", &eleven).is_err());
    }
}
