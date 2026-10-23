//! One account in several families (#88; docs/protocol.md, "Several
//! families", and the decisions in docs/families-2026-10-22.md): which
//! family a client shows, which chats it lists there, when the switcher is
//! drawn, when "New Family…" and "Join Another Family…" are offered, and
//! what each of the new refusals is told as.
//!
//! One table every surface reads, as the groups' rules are — a check that
//! one screen spelt `families.len() > 1` and another `>= 1` is how the
//! switcher would appear on one and not the other.

use crate::i18n::{t, t1, t2, tn, tp};

/// How many families one account may be in when the server has not said
/// (`[families] max_families_per_account`, default 5). Only ever the number
/// a refusal names: a client offers nothing on a cap it was not told.
pub const DEFAULT_MAX: i64 = 5;

/// The families a chat is listed in: its own `family_ids`, or — absent, on
/// a server that predates them — "the one family the account has", which
/// is the home family (docs/protocol.md, `Chat`).
pub fn families_of(family_ids: Option<&[i64]>, home: Option<i64>) -> Vec<i64> {
    match family_ids {
        Some(ids) => ids.to_vec(),
        None => home.into_iter().collect(),
    }
}

/// Whether a chat is shown in `family`: exactly the chats that name it. A
/// direct chat names every family the two people share, and so appears in
/// each.
pub fn listed_in(family_ids: Option<&[i64]>, family: i64, home: Option<i64>) -> bool {
    families_of(family_ids, home).contains(&family)
}

/// The family to show after a `GET /me`: the one shown now while it is
/// still listed, else the one this device showed last, else the first —
/// the home family, which `/me` lists first. None for an account in no
/// family, which gets the gate.
pub fn current_after(listed: &[i64], held: Option<i64>, preferred: Option<i64>) -> Option<i64> {
    held.filter(|id| listed.contains(id))
        .or_else(|| preferred.filter(|id| listed.contains(id)))
        .or_else(|| listed.first().copied())
}

/// Whether the switcher is drawn: two families or more, or something
/// waiting to be shown in it — an invitation, or a join request. An account
/// in ONE family with nothing pending sees nothing of this.
pub fn shows_switcher(families: usize, invitations: usize, pending: usize) -> bool {
    families >= 2 || invitations > 0 || pending > 0
}

/// Whether "New Family…" is offered: the server lets members start
/// families, the account owns none of its own, and it is under the cap. A
/// cap the server did not send is a server that predates it, and offers
/// nothing.
pub fn offers_new_family(enabled: bool, owns_one: bool, count: usize, max: Option<i64>) -> bool {
    enabled && !owns_one && under_cap(count, max)
}

/// Whether "Join Another Family…" is offered: while under the cap.
pub fn offers_join_another(count: usize, max: Option<i64>) -> bool {
    under_cap(count, max)
}

fn under_cap(count: usize, max: Option<i64>) -> bool {
    max.is_some_and(|max| (count as i64) < max)
}

/// What one of the new refusals is told as — None for a code that is not
/// one of them, which the surface words its own way. `max` is the cap the
/// account is under (`max_families_per_account`).
pub fn refusal(code: Option<&str>, max: i64) -> Option<String> {
    match code? {
        "too_many_families" => Some(tn(
            "You've reached the most families one account can be in (%lld).",
            max,
        )),
        "already_own_a_family" => {
            Some(t("You already own a family, so you can't start another.").to_string())
        }
        "member_families_disabled" => {
            Some(t("This server doesn't let members start new families.").to_string())
        }
        "member_in_other_families" => Some(
            t("This member is also in a family you don't own, so only they can change their password.")
                .to_string(),
        ),
        "invitation_not_pending" => Some(t("This invitation is no longer available.").to_string()),
        _ => None,
    }
}

/// A family's row in the switcher, as a screen reader says it.
pub fn row_label(name: &str, unread: i64) -> String {
    tp("%@, %lld unread", unread, &[name, &unread.to_string()])
}

/// The invitation, as the banner and the switcher's row say it.
pub fn invited(inviter: &str, family: &str) -> String {
    t2("%@ invited you to “%@”", inviter, family)
}

/// The owner's picker of whom to invite, titled with the family.
pub fn invite_title(family: &str) -> String {
    t1("Invite to “%@”", family)
}

/// The leave dialog's title for an account in several families: WHICH one
/// it is leaving. In one family the dialog is today's, word for word.
pub fn leave_title(family: &str) -> String {
    t1("Leave “%@”?", family)
}

/// And what leaving one of several takes: that family's chats, and none of
/// the direct chats it shares with somebody in another family.
pub fn leave_message() -> &'static str {
    t("You'll lose access to this family's chats. Direct chats stay with anyone you share another family with.")
}

/// Who an owner may invite into `family`: the members of their OTHER
/// families who are not in this one, each once, in the order first met —
/// and nobody else on the server (docs/protocol.md, "Several families").
/// `others` is each other family's roster as (user id, name).
pub fn invitable(in_this: &[i64], others: &[Vec<(i64, String)>], me: i64) -> Vec<(i64, String)> {
    let mut found: Vec<(i64, String)> = Vec::new();
    for roster in others {
        for (id, name) in roster {
            if *id != me && !in_this.contains(id) && !found.iter().any(|(held, _)| held == id) {
                found.push((*id, name.clone()));
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chat_is_listed_in_the_families_it_names() {
        assert!(listed_in(Some(&[3]), 3, Some(3)));
        assert!(!listed_in(Some(&[3]), 4, Some(3)));
        // A direct chat between two people who share two families.
        assert!(listed_in(Some(&[3, 4]), 4, Some(3)));
        assert!(listed_in(Some(&[3, 4]), 3, Some(3)));
        // Named in no family: listed nowhere.
        assert!(!listed_in(Some(&[]), 3, Some(3)));
    }

    #[test]
    fn absent_family_ids_mean_the_home_family() {
        assert!(listed_in(None, 3, Some(3)));
        assert!(!listed_in(None, 4, Some(3)));
        assert!(!listed_in(None, 3, None), "an account in no family");
        assert_eq!(families_of(None, Some(3)), vec![3]);
        assert_eq!(families_of(Some(&[4, 9]), Some(3)), vec![4, 9]);
    }

    #[test]
    fn the_family_shown_stays_until_it_is_gone() {
        assert_eq!(current_after(&[3, 4], Some(4), Some(3)), Some(4));
        assert_eq!(
            current_after(&[3, 4], None, Some(4)),
            Some(4),
            "what this device showed last"
        );
        assert_eq!(
            current_after(&[3, 4], None, None),
            Some(3),
            "the home family"
        );
        assert_eq!(
            current_after(&[3], Some(4), Some(4)),
            Some(3),
            "the one shown is gone: the first that is left"
        );
        assert_eq!(current_after(&[3], None, Some(9)), Some(3));
        assert_eq!(current_after(&[], Some(3), Some(3)), None, "the gate");
    }

    #[test]
    fn the_switcher_needs_two_families_or_something_waiting() {
        assert!(!shows_switcher(1, 0, 0), "one family and nothing pending");
        assert!(!shows_switcher(0, 0, 0));
        assert!(shows_switcher(2, 0, 0));
        assert!(shows_switcher(1, 1, 0), "an invitation to show");
        assert!(shows_switcher(1, 0, 1), "a join request to show");
    }

    #[test]
    fn a_new_family_only_while_owning_none_and_under_the_cap() {
        assert!(offers_new_family(true, false, 1, Some(5)));
        assert!(
            !offers_new_family(false, false, 1, Some(5)),
            "the operator's switch"
        );
        assert!(
            !offers_new_family(true, true, 1, Some(5)),
            "owns one already"
        );
        assert!(!offers_new_family(true, false, 5, Some(5)), "at the cap");
        assert!(
            !offers_new_family(true, false, 1, None),
            "a server that predates it"
        );
    }

    #[test]
    fn joining_another_is_offered_under_the_cap() {
        assert!(offers_join_another(1, Some(5)));
        assert!(offers_join_another(4, Some(5)));
        assert!(!offers_join_another(5, Some(5)));
        assert!(!offers_join_another(6, Some(5)));
        assert!(!offers_join_another(1, None));
    }

    #[test]
    fn each_new_refusal_says_what_happened() {
        assert_eq!(
            refusal(Some("too_many_families"), 5).as_deref(),
            Some("You've reached the most families one account can be in (5).")
        );
        assert_eq!(
            refusal(Some("already_own_a_family"), 5).as_deref(),
            Some("You already own a family, so you can't start another.")
        );
        assert_eq!(
            refusal(Some("member_families_disabled"), 5).as_deref(),
            Some("This server doesn't let members start new families.")
        );
        assert_eq!(
            refusal(Some("member_in_other_families"), 5).as_deref(),
            Some("This member is also in a family you don't own, so only they can change their password.")
        );
        assert_eq!(
            refusal(Some("invitation_not_pending"), 5).as_deref(),
            Some("This invitation is no longer available.")
        );
        assert_eq!(
            refusal(Some("family_full"), 5),
            None,
            "worded by its surface"
        );
        assert_eq!(refusal(None, 5), None);
    }

    #[test]
    fn the_rows_and_dialogs_say_which_family() {
        assert_eq!(row_label("The Smiths", 3), "The Smiths, 3 unread");
        assert_eq!(row_label("The Smiths", 0), "The Smiths, 0 unread");
        assert_eq!(invited("Anna", "Ivanovs"), "Anna invited you to “Ivanovs”");
        assert_eq!(invite_title("Ivanovs"), "Invite to “Ivanovs”");
        assert_eq!(leave_title("Ivanovs"), "Leave “Ivanovs”?");
        assert!(leave_message().starts_with("You'll lose access to this family's chats."));
    }

    #[test]
    fn an_owner_may_invite_only_from_their_other_families() {
        let parents = vec![
            (7, "Me".to_string()),
            (9, "Anna".to_string()),
            (11, "Bob".to_string()),
        ];
        let friends = vec![
            (7, "Me".to_string()),
            (11, "Bob".to_string()),
            (12, "Cleo".to_string()),
        ];
        assert_eq!(
            invitable(&[7, 9], &[parents, friends], 7),
            vec![(11, "Bob".to_string()), (12, "Cleo".to_string())],
            "not me, not who is in already, and Bob once"
        );
        assert!(invitable(&[7], &[], 7).is_empty(), "no other family");
    }
}
