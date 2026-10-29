//! A poll's arithmetic and the composer's check (`Polls.cs`; docs/protocol.md, "Polls") — the
//! rules the web client keeps in `views/poll.rs` and the apps in `PollPresentation`, in one place
//! the bubble and the open-polls sheet both draw from. No `fc_text` module has them.
//!
//! COUNTS COUNT EVERYONE; NAMES SKIP THE BLOCKED. The voter count, the bars and "N of M voted" go
//! on counting a blocked member's vote, because a tally that moved when you blocked somebody would
//! tell you they had voted. Only [`drawable_voters`] filters, and it filters identity alone
//! (docs/protocol.md, "Blocking a member").
//!
//! THE CHECK IS THE SERVER'S, IN RUST'S OWN STD. The server trims with `str::trim`, counts
//! `chars()` — scalars, not UTF-16 units — and folds with `str::to_lowercase`, which maps `İ` to
//! two scalars and applies the final-sigma rule. Here those ARE the functions called (the C#
//! carried tables printed from Rust's std to imitate them), and the chat oracle pins all three.
//!
//! The poll and its message are the protocol's `PollDto` and `MessageDto` in the C#. The rules read
//! them through [`Poll`] and [`PollMessage`] — exactly the fields the rules read — which the
//! protocol's records implement at the end of this file (and test records do in the tests).

use std::collections::HashSet;

pub const MIN_OPTIONS: usize = 2;
pub const MAX_OPTIONS: usize = 10;
pub const MAX_OPTION_CHARS: usize = 100;

/// How many voters a row names before "+N".
pub const MAX_FACES: usize = 5;

/// What the rules read of a poll (`PollDto`).
pub trait Poll {
    /// Each option's id and the members who chose it, in the wire's order.
    fn options(&self) -> Vec<(i64, &[i64])>;
    fn closed(&self) -> bool;
    /// The poll's own sequence: a newer copy of the same message has a higher one.
    fn poll_seq(&self) -> i64;
}

/// What the rules read of a message (`MessageDto`).
pub trait PollMessage {
    type Poll: Poll;
    /// 0 for a message the server has not numbered yet.
    fn id(&self) -> i64;
    fn sender_id(&self) -> i64;
    fn poll(&self) -> Option<&Self::Poll>;
}

/// The option this reader holds, if any — one choice per member.
pub fn my_option(poll: &impl Poll, me: i64) -> Option<i64> {
    poll.options()
        .into_iter()
        .find(|(_, votes)| votes.contains(&me))
        .map(|(id, _)| id)
}

/// Everyone who has voted, counted once.
pub fn voter_count(poll: &impl Poll) -> usize {
    poll.options()
        .into_iter()
        .flat_map(|(_, votes)| votes.iter().copied())
        .collect::<HashSet<i64>>()
        .len()
}

/// How full one option's bar is: its share of the votes CAST, not of the family — 0 while nobody
/// has voted, so a poll opens with every bar empty.
pub fn fraction(poll: &impl Poll, votes: usize) -> f64 {
    let total: usize = poll.options().iter().map(|(_, votes)| votes.len()).sum();
    if total == 0 {
        0.0
    } else {
        votes as f64 / total as f64
    }
}

/// The voters whose names may be DRAWN, in the order they voted: everyone but the blocked.
pub fn drawable_voters(votes: &[i64], blocked: impl Fn(i64) -> bool) -> Vec<i64> {
    votes
        .iter()
        .copied()
        .filter(|&voter| !blocked(voter))
        .collect()
}

/// What a tap means: the option already held clears the vote, any other casts it. The server's
/// vote is a state-set and not a toggle, so this is the client's to decide.
pub fn tap_retracts(poll: &impl Poll, option_id: i64, me: i64) -> bool {
    my_option(poll, me) == Some(option_id)
}

/// A poll the server has numbered and nobody has closed.
pub fn votable(message: &impl PollMessage) -> bool {
    message.id() != 0 && message.poll().is_some_and(|poll| !poll.closed())
}

/// Open polls this reader has not answered — the badge on the way to the list. A message held
/// twice (the chat's copy and the list's) counts once, by its newest state (the highest
/// `poll_seq`, the first copy on a tie); a blocked member's poll is not one to chase.
pub fn unanswered<M: PollMessage>(
    held: &[M],
    listed: &[M],
    me: i64,
    blocked: impl Fn(i64) -> bool,
) -> usize {
    let mut newest: Vec<&M> = Vec::new();
    for message in held.iter().chain(listed) {
        let Some(poll) = message.poll() else {
            continue;
        };
        if message.id() == 0 {
            continue;
        }
        match newest.iter_mut().find(|kept| kept.id() == message.id()) {
            Some(kept) => {
                if kept
                    .poll()
                    .is_some_and(|was| poll.poll_seq() > was.poll_seq())
                {
                    *kept = message;
                }
            }
            None => newest.push(message),
        }
    }
    newest
        .into_iter()
        .filter(|message| !blocked(message.sender_id()))
        .filter(|message| {
            message
                .poll()
                .is_some_and(|poll| !poll.closed() && my_option(poll, me).is_none())
        })
        .count()
}

/// Rust's `str::trim`: the White_Space property, as the server trims.
pub fn trimmed(text: &str) -> &str {
    text.trim()
}

/// A question is there once it is more than white space.
pub fn has_question(question: &str) -> bool {
    !trimmed(question).is_empty()
}

/// The options as they will be sent — trimmed, blank rows dropped — or `None` when the server
/// would answer `invalid_poll`: fewer than 2 or more than 10, one over 100 characters, or two the
/// same ignoring case. The CLEANED list rather than a yes, so what was checked is what leaves.
pub fn sanitized<S: AsRef<str>>(options: &[S]) -> Option<Vec<String>> {
    let trimmed: Vec<String> = options
        .iter()
        .map(|option| trimmed(option.as_ref()).to_string())
        .filter(|option| !option.is_empty())
        .collect();
    if !(MIN_OPTIONS..=MAX_OPTIONS).contains(&trimmed.len()) {
        return None;
    }
    if trimmed
        .iter()
        .any(|option| option.chars().count() > MAX_OPTION_CHARS)
    {
        return None;
    }
    let mut seen = HashSet::new();
    trimmed
        .iter()
        .all(|option| seen.insert(lowercase(option)))
        .then_some(trimmed)
}

/// Rust's `str::to_lowercase`: each scalar's full lowercase mapping — `İ` becomes `i` and a
/// combining dot — and a capital sigma that ends a word becomes `ς`.
pub fn lowercase(text: &str) -> String {
    text.to_lowercase()
}

impl Poll for crate::protocol::PollDto {
    fn options(&self) -> Vec<(i64, &[i64])> {
        self.options
            .iter()
            .map(|option| (option.id, option.votes.as_slice()))
            .collect()
    }

    fn closed(&self) -> bool {
        self.closed
    }

    fn poll_seq(&self) -> i64 {
        self.poll_seq
    }
}

impl PollMessage for crate::protocol::MessageDto {
    type Poll = crate::protocol::PollDto;

    fn id(&self) -> i64 {
        self.id
    }

    fn sender_id(&self) -> i64 {
        self.sender_id
    }

    fn poll(&self) -> Option<&crate::protocol::PollDto> {
        self.poll.as_ref()
    }
}
