//! The words a poll is drawn with, in the apps' own sentences (`PollText.cs`).

use fc_core::protocol::{MemberDto, PollDto, PollOptionDto};
use fc_core::text::polls;
use fc_core::text::string_catalog::StringCatalog;

/// "3 of 5 voted" — or "3 voted" before the roster is known. The count that chooses the plural
/// form is the one that VARIES; the roster's size is only the second number.
pub fn footer(poll: &PollDto, member_count: usize, say: &dyn StringCatalog) -> String {
    let voted = polls::voter_count(poll);
    if member_count > 0 {
        say.plural("%lld of %lld voted", count(voted), &[&voted, &member_count])
    } else {
        say.plural("%lld voted", count(voted), &[&voted])
    }
}

/// What an option says to a screen reader: its words, its votes, and whether it is the reader's.
pub fn option_label(option: &PollOptionDto, chosen: bool, say: &dyn StringCatalog) -> String {
    let tally = option.votes.len();
    let votes = say.plural("%@. %lld votes", count(tally), &[&option.text, &tally]);
    if chosen {
        format!("{votes}. {}", say.get("Your choice"))
    } else {
        votes
    }
}

/// "You" for the reader, "Deleted account" for an account that is gone, the roster's name, or
/// "Someone".
pub fn name(
    user_id: i64,
    reader: i64,
    member: impl Fn(i64) -> Option<MemberDto>,
    say: &dyn StringCatalog,
) -> String {
    if user_id == reader {
        return say.get("You").to_string();
    }
    match member(user_id) {
        Some(found) if found.deleted => say.get("Deleted account").to_string(),
        Some(found) if !found.display_name.is_empty() => found.display_name,
        _ => say.get("Someone").to_string(),
    }
}

/// Who chose an option, named up to `polls::MAX_FACES` and "+N" past that. The "+N" is the
/// overflow of the DRAWABLE names, never a tally: counted from the raw list it would print how
/// many blocked people chose that option.
pub fn voters(drawable: &[i64], name: impl Fn(i64) -> String, say: &dyn StringCatalog) -> String {
    let shown = drawable
        .iter()
        .take(polls::MAX_FACES)
        .map(|&id| name(id))
        .collect::<Vec<_>>()
        .join(", ");
    let more = drawable.len().saturating_sub(polls::MAX_FACES);
    if more > 0 {
        format!("{shown} {}", say.format("+%lld", &[&more]))
    } else {
        shown
    }
}

/// The M in "N of M voted": the live roster — neither the people who have left nor accounts that
/// were deleted, because a tally must not go on counting somebody who no longer exists. In a
/// group, the group's members (docs/protocol.md, "Groups"): the caller passes
/// `ComposerMentions::candidates`.
pub fn member_count<'a>(members: impl IntoIterator<Item = &'a MemberDto>) -> usize {
    members
        .into_iter()
        .filter(|member| !member.has_left && !member.deleted)
        .count()
}

/// A count as the catalogue's plural rule takes it.
fn count(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}
