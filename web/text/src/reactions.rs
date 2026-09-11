//! A message's reactions as the bubble draws them: chips under it, the rows
//! of "See who reacted", and the optimistic rewrite a tap makes before the
//! server answers.
//!
//! Ported from the Apple app — `MessagePresentation.reactionChips` and
//! `reactionDetails` in ios/FamilyConnect/Models/Snapshots.swift, and the
//! pure half of `ChatSyncCoordinator.toggleReaction`. Android has the same
//! rules in ui/chat/ChatItems.kt and MessageRepository.toggleReaction.
//!
//! The server holds ONE reaction per user per message and sends a
//! message's full reaction list, in the order the reactions were made
//! (protocol.md: `{"user_id": 9, "emoji": "❤️"}`). Everything here is a
//! pure function of that list.
//!
//! Emoji are compared by bytes, as Kotlin and the server compare them.
//! Swift compares `String`s by canonical equivalence, so a reaction that
//! is canonically equivalent to another but spelled differently (possible
//! only for non-emoji text such as "Å" U+00C5 against "A" + U+030A — the
//! server takes any string of at most 32 bytes) is one chip on the Apple
//! apps and two here and on Android. No emoji has a canonical
//! decomposition, so real emoji never differ.
use crate::i18n::t;

use std::collections::{HashMap, HashSet};

/// One user's reaction to one message, in the wire shape
/// `{"user_id": 9, "emoji": "❤️"}`. Plain on purpose, so this file carries no
/// serde dependency; the crate it lands in derives or maps as it likes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reaction {
    pub user_id: i64,
    pub emoji: String,
}

impl Reaction {
    pub fn new(user_id: i64, emoji: &str) -> Self {
        Self {
            user_id,
            emoji: emoji.to_string(),
        }
    }
}

/// The emoji size inside a chip: 18 on the phone and on Android (points and
/// sp), and 18 × 13/17 on the Mac, which scales it like the emoji ladder.
/// Pinned because the two apps once drifted apart for this same element.
pub const REACTION_CHIP_EMOJI_SIZE: f64 = 18.0;

/// One chip in the row under a bubble: an emoji, how many members chose it,
/// and whether the reader is among them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReactionChip {
    pub emoji: String,
    pub count: usize,
    /// The reader chose this emoji: the chip is outlined, and a tap shows
    /// who reacted instead of joining.
    pub includes_me: bool,
}

/// What a tap (or click) on a chip does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChipTap {
    /// Add the reader's reaction with this chip's emoji.
    Join,
    /// Show who reacted — where the reader's own row is the remove control.
    ShowReactors,
}

impl ReactionChip {
    /// A tap JOINS a chip and never removes: on a chip the reader is part
    /// of it shows who reacted, and taking a reaction off is a deliberate
    /// tap on their own row there. Undoing something you never meant to do
    /// is a worse failure than one extra tap. The phone, the Mac and Android
    /// all do this; the Mac once did not (its click went straight to the
    /// toggle, which REMOVES a reaction that is already yours).
    pub fn tap(&self) -> ChipTap {
        if self.includes_me {
            ChipTap::ShowReactors
        } else {
            ChipTap::Join
        }
    }

    /// The count is drawn only from two up: a lone reaction's chip is just
    /// its emoji, so the outline is the only mark of "mine".
    pub fn shows_count(&self) -> bool {
        self.count > 1
    }
}

/// Group a message's reactions into chips: one per distinct emoji, in the
/// order each emoji FIRST appears in the list.
///
/// First-seen, never by popularity: the server keeps reactions in the order
/// they were made, so this order is stable across renders and a chip does
/// not jump when others pile onto a later one.
///
/// Swift: `MessagePresentation.reactionChips(_:currentUserID:)`. A linear
/// search rather than a map: a message holds at most one reaction per
/// family member, so the lists are tiny.
pub fn reaction_chips(reactions: &[Reaction], current_user_id: i64) -> Vec<ReactionChip> {
    let mut chips: Vec<ReactionChip> = Vec::new();
    for reaction in reactions {
        let mine = reaction.user_id == current_user_id;
        match chips.iter_mut().find(|chip| chip.emoji == reaction.emoji) {
            Some(chip) => {
                chip.count += 1;
                chip.includes_me |= mine;
            }
            None => chips.push(ReactionChip {
                emoji: reaction.emoji.clone(),
                count: 1,
                includes_me: mine,
            }),
        }
    }
    chips
}

/// How the reader is named in "See who reacted". A function, not a const:
/// a translated string is not a constant.
pub fn you_label() -> &'static str {
    t("You")
}

/// How a reactor the roster does not know is named — the name the web
/// client already gives an unknown typist. Swift says the same; Android
/// says "Member <id>".
pub fn someone_label() -> &'static str {
    t("Someone")
}

/// One emoji's row in "See who reacted".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReactionDetail {
    pub emoji: String,
    /// "You" first when the reader chose this emoji, then everybody else in
    /// reaction order.
    pub names: Vec<String>,
    /// The first person listed, so the row can lead with their face. `None`
    /// when nobody is listed — which happens when every reactor on this
    /// emoji is blocked: the row stays (the chip still counts them) but
    /// names nobody.
    pub lead_user_id: Option<i64>,
}

/// Expand a message's reactions into the rows of "See who reacted": one per
/// distinct emoji in the SAME first-seen order as [`reaction_chips`] (the
/// two views sit side by side and must agree).
///
/// Within an emoji the reader shows as "You" and comes first; everybody else
/// keeps reaction order, under their name from `names`, or "Someone".
///
/// A blocked reactor is dropped from these rows and from nowhere else:
/// [`reaction_chips`] keeps their COUNT, so a chip may read 3 while its row
/// names two. That gap is deliberate and visible only to the blocker, who
/// knows they blocked somebody — a count that moved would tell the BLOCKED
/// person they had been (protocol.md, "Blocking a member"). Hence
/// `blocked_user_ids` has no default: every caller has to decide.
///
/// Swift: `MessagePresentation.reactionDetails(_:names:currentUserID:blockedUserIDs:)`.
pub fn reaction_details(
    reactions: &[Reaction],
    names: &HashMap<i64, String>,
    current_user_id: i64,
    blocked_user_ids: &HashSet<i64>,
) -> Vec<ReactionDetail> {
    struct Row {
        emoji: String,
        mine: bool,
        others: Vec<String>,
        other_ids: Vec<i64>,
    }
    let mut rows: Vec<Row> = Vec::new();
    for reaction in reactions {
        // The row exists from the emoji's first reaction, whoever made it —
        // even a blocked member's — so rows and chips line up one to one.
        let index = match rows.iter().position(|row| row.emoji == reaction.emoji) {
            Some(index) => index,
            None => {
                rows.push(Row {
                    emoji: reaction.emoji.clone(),
                    mine: false,
                    others: Vec::new(),
                    other_ids: Vec::new(),
                });
                rows.len() - 1
            }
        };
        let row = &mut rows[index];
        if reaction.user_id == current_user_id {
            row.mine = true;
        } else if !blocked_user_ids.contains(&reaction.user_id) {
            let name = names
                .get(&reaction.user_id)
                .map_or(someone_label(), String::as_str);
            row.others.push(name.to_string());
            row.other_ids.push(reaction.user_id);
        }
    }
    rows.into_iter()
        .map(|row| {
            // "You" leads its emoji, so the reader's own id leads it too.
            let (names, lead_user_id) = if row.mine {
                let mut names = vec![you_label().to_string()];
                names.extend(row.others);
                (names, Some(current_user_id))
            } else {
                (row.others, row.other_ids.first().copied())
            };
            ReactionDetail {
                emoji: row.emoji,
                names,
                lead_user_id,
            }
        })
        .collect()
}

/// The reader's own reaction on a message — the capsule marks it selected
/// and offers it even when it is not a quick one.
///
/// Swift: `message.reactionList.first { $0.userID == currentUserID }?.emoji`
/// (ConversationView). The server allows one per user, so "first" is "only".
pub fn my_reaction(reactions: &[Reaction], current_user_id: i64) -> Option<&str> {
    reactions
        .iter()
        .find(|reaction| reaction.user_id == current_user_id)
        .map(|reaction| reaction.emoji.as_str())
}

/// What a toggle does before the server has answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReactionToggle {
    /// The reader tapped the emoji they already have: send DELETE. Otherwise
    /// send PUT with the emoji — the server's set is a state-set, not a
    /// toggle, so the client decides which one a tap means.
    pub removing: bool,
    /// The list to show until the server's answer lands.
    pub reactions: Vec<Reaction>,
}

/// The optimistic rewrite of a toggle: the reader's reaction comes off, and
/// goes back on — APPENDED, as the newest reaction, like the server does —
/// unless they tapped the one they already had.
///
/// The input is left alone on purpose: it is what a failed request restores.
/// The caller must not bump the message's `reaction_seq` for this list —
/// only the server mints sequences, and a bumped one would make the
/// authoritative reply look stale and be dropped.
///
/// Swift: the first lines of `ChatSyncCoordinator.toggleReaction`, which
/// compares against the reader's FIRST reaction. Android asks whether ANY
/// of theirs matches; with the server's one-per-user rule the two are the
/// same.
pub fn toggle_reaction(reactions: &[Reaction], user_id: i64, emoji: &str) -> ReactionToggle {
    let removing = my_reaction(reactions, user_id) == Some(emoji);
    let mut rewritten: Vec<Reaction> = reactions
        .iter()
        .filter(|reaction| reaction.user_id != user_id)
        .cloned()
        .collect();
    if !removing {
        rewritten.push(Reaction::new(user_id, emoji));
    }
    ReactionToggle {
        removing,
        reactions: rewritten,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEART: &str = "\u{2764}\u{FE0F}"; // ❤️
    const THUMBS_UP: &str = "\u{1F44D}"; // 👍
    const JOY: &str = "\u{1F602}"; // 😂
    const OPEN_MOUTH: &str = "\u{1F62E}"; // 😮

    fn chip(emoji: &str, count: usize, includes_me: bool) -> ReactionChip {
        ReactionChip {
            emoji: emoji.to_string(),
            count,
            includes_me,
        }
    }

    fn detail(emoji: &str, names: &[&str], lead_user_id: Option<i64>) -> ReactionDetail {
        ReactionDetail {
            emoji: emoji.to_string(),
            names: names.iter().map(|name| name.to_string()).collect(),
            lead_user_id,
        }
    }

    fn roster(entries: &[(i64, &str)]) -> HashMap<i64, String> {
        entries
            .iter()
            .map(|&(id, name)| (id, name.to_string()))
            .collect()
    }

    fn nobody_blocked() -> HashSet<i64> {
        HashSet::new()
    }

    // MARK: - Ported from MessageGroupingTests.swift

    /// Swift: blockedReactorKeepsTheCount — "a blocked reactor keeps their
    /// count and loses their name".
    #[test]
    fn blocked_reactor_keeps_the_count() {
        let reactions = [
            Reaction::new(9, HEART),
            Reaction::new(11, HEART),
            Reaction::new(13, HEART),
        ];
        let chips = reaction_chips(&reactions, 7);
        assert_eq!(chips.len(), 1);
        assert_eq!(chips[0].count, 3, "integers are not presence");

        let names = roster(&[(9, "Anna"), (11, "Bob"), (13, "Cara")]);
        let unfiltered = reaction_details(&reactions, &names, 7, &nobody_blocked());
        assert_eq!(unfiltered[0].names, ["Anna", "Bob", "Cara"]);

        let filtered = reaction_details(&reactions, &names, 7, &HashSet::from([11]));
        assert_eq!(filtered[0].names, ["Anna", "Cara"], "the identity goes");
        // And the chip is computed from the SAME list and is unchanged, so
        // the popover deliberately names fewer people than the chip counts.
        let chips_again = reaction_chips(&reactions, 7);
        assert_eq!(chips_again[0].count, 3, "the chip still reads 3");
    }

    /// Swift: reactionChipAggregation — "chips aggregate per emoji in
    /// first-seen order with counts and includesMe".
    #[test]
    fn reaction_chip_aggregation() {
        let reactions = [
            Reaction::new(9, HEART),
            Reaction::new(11, THUMBS_UP),
            Reaction::new(7, HEART),
            Reaction::new(12, JOY),
        ];
        assert_eq!(
            reaction_chips(&reactions, 7),
            [
                chip(HEART, 2, true),
                chip(THUMBS_UP, 1, false),
                chip(JOY, 1, false),
            ]
        );
    }

    /// Swift: reactionChipOrderIsFirstSeenNotPopularity — "first-seen order
    /// holds even when a later emoji outnumbers an earlier one".
    #[test]
    fn reaction_chip_order_is_first_seen_not_popularity() {
        let reactions = [
            Reaction::new(1, THUMBS_UP),
            Reaction::new(2, HEART),
            Reaction::new(3, HEART),
            Reaction::new(4, HEART),
        ];
        let chips = reaction_chips(&reactions, 99);
        let emoji: Vec<&str> = chips.iter().map(|chip| chip.emoji.as_str()).collect();
        let counts: Vec<usize> = chips.iter().map(|chip| chip.count).collect();
        assert_eq!(emoji, [THUMBS_UP, HEART]);
        assert_eq!(counts, [1, 3]);
        assert!(chips.iter().all(|chip| !chip.includes_me));
    }

    /// Swift: reactionChipsEmpty — "no reactions yield no chips".
    #[test]
    fn reaction_chips_empty() {
        assert!(reaction_chips(&[], 7).is_empty());
    }

    /// Swift: reactionDetailsOrder — "details follow chip order; names
    /// resolve in reaction order".
    #[test]
    fn reaction_details_order() {
        let reactions = [
            Reaction::new(9, HEART),
            Reaction::new(11, THUMBS_UP),
            Reaction::new(12, HEART),
        ];
        let names = roster(&[(9, "Anna"), (11, "Ben"), (12, "Kim")]);
        assert_eq!(
            reaction_details(&reactions, &names, 7, &nobody_blocked()),
            [
                detail(HEART, &["Anna", "Kim"], Some(9)),
                detail(THUMBS_UP, &["Ben"], Some(11)),
            ]
        );
    }

    /// Swift: reactionDetailsMatchChipOrder — "detail order matches the chips
    /// for the same input".
    #[test]
    fn reaction_details_match_chip_order() {
        let reactions = [
            Reaction::new(1, THUMBS_UP),
            Reaction::new(2, HEART),
            Reaction::new(3, JOY),
            Reaction::new(4, HEART),
        ];
        let chips = reaction_chips(&reactions, 2);
        let details = reaction_details(&reactions, &HashMap::new(), 2, &nobody_blocked());
        let chip_emoji: Vec<&str> = chips.iter().map(|chip| chip.emoji.as_str()).collect();
        let detail_emoji: Vec<&str> = details.iter().map(|detail| detail.emoji.as_str()).collect();
        assert_eq!(detail_emoji, chip_emoji);
    }

    /// Swift: reactionDetailsYouFirst(position:), arguments [0, 1, 2] —
    /// "You leads its emoji no matter when I reacted".
    #[test]
    fn reaction_details_you_first() {
        for position in [0, 1, 2] {
            let mut reactions = vec![Reaction::new(9, HEART), Reaction::new(11, HEART)];
            reactions.insert(position, Reaction::new(7, HEART));
            let details = reaction_details(
                &reactions,
                &roster(&[(9, "Anna"), (11, "Ben")]),
                7,
                &nobody_blocked(),
            );
            // "You" leads the names, so my own id leads the row.
            assert_eq!(
                details,
                [detail(HEART, &["You", "Anna", "Ben"], Some(7))],
                "position {position}"
            );
        }
    }

    /// Swift: reactionDetailsYouPerEmoji — "You substitutes only in the
    /// emoji I chose, others keep their names".
    #[test]
    fn reaction_details_you_per_emoji() {
        let reactions = [
            Reaction::new(9, THUMBS_UP),
            Reaction::new(7, JOY),
            Reaction::new(11, JOY),
        ];
        assert_eq!(
            reaction_details(
                &reactions,
                &roster(&[(9, "Anna"), (11, "Ben")]),
                7,
                &nobody_blocked()
            ),
            [
                detail(THUMBS_UP, &["Anna"], Some(9)),
                detail(JOY, &["You", "Ben"], Some(7)),
            ]
        );
    }

    /// Swift: reactionDetailsUnknownReactor — "a reactor missing from the
    /// member list falls back to Someone".
    #[test]
    fn reaction_details_unknown_reactor() {
        let reactions = [Reaction::new(99, THUMBS_UP)];
        assert_eq!(
            reaction_details(&reactions, &HashMap::new(), 7, &nobody_blocked()),
            [detail(THUMBS_UP, &["Someone"], Some(99))]
        );
    }

    /// Swift: reactionDetailsEmpty — "no reactions yield no details".
    #[test]
    fn reaction_details_empty() {
        assert!(reaction_details(&[], &HashMap::new(), 7, &nobody_blocked()).is_empty());
    }

    // MARK: - The pure halves of ReactionSyncTests.swift
    //
    // Those tests drive the coordinator against a store and a stubbed
    // server; what is pure in them is the optimistic list and the choice
    // between PUT and DELETE, which is all that is pinned here.

    /// Swift: togglePutPath — a new emoji is a PUT, and mine is appended
    /// after user 9's (the same order the server's reply then confirms).
    #[test]
    fn toggle_put_path() {
        let toggle = toggle_reaction(&[Reaction::new(9, THUMBS_UP)], 7, HEART);
        assert!(!toggle.removing);
        assert_eq!(
            toggle.reactions,
            [Reaction::new(9, THUMBS_UP), Reaction::new(7, HEART)]
        );
    }

    /// Swift: toggleDeletePath — my current emoji is a DELETE.
    #[test]
    fn toggle_delete_path() {
        let toggle = toggle_reaction(&[Reaction::new(7, HEART)], 7, HEART);
        assert!(toggle.removing);
        assert!(toggle.reactions.is_empty());
    }

    /// Swift: toggleRevertsOnError — the pre-toggle list is what a failure
    /// restores, so the rewrite must not touch it.
    #[test]
    fn toggle_reverts_on_error() {
        let before = vec![Reaction::new(9, THUMBS_UP)];
        let toggle = toggle_reaction(&before, 7, HEART);
        assert_eq!(before, [Reaction::new(9, THUMBS_UP)]);
        assert_ne!(toggle.reactions, before);
    }

    // MARK: - Beyond the Swift suites

    /// Android's reactionChipsAggregateInFirstSeenOrderWithCountsAndIncludesMe:
    /// "mine" on the SECOND chip, joined after someone else started it.
    #[test]
    fn chips_mark_mine_when_i_joined_late() {
        let reactions = [
            Reaction::new(11, HEART),
            Reaction::new(12, THUMBS_UP),
            Reaction::new(13, HEART),
            Reaction::new(7, THUMBS_UP),
        ];
        assert_eq!(
            reaction_chips(&reactions, 7),
            [chip(HEART, 2, false), chip(THUMBS_UP, 2, true)]
        );
    }

    /// "Mine" sticks once set: someone else joining my emoji after me must
    /// not clear it.
    #[test]
    fn chips_keep_mine_when_others_join_after_me() {
        let reactions = [Reaction::new(7, HEART), Reaction::new(9, HEART)];
        assert_eq!(reaction_chips(&reactions, 7), [chip(HEART, 2, true)]);
    }

    /// Bytes decide what is the same emoji: a bare U+2764 heart is not the
    /// quick heart. Swift agrees — a variation selector has no canonical
    /// decomposition, so the two are not canonically equivalent either.
    #[test]
    fn text_heart_and_emoji_heart_are_different_chips() {
        let reactions = [Reaction::new(9, "\u{2764}"), Reaction::new(11, HEART)];
        assert_eq!(
            reaction_chips(&reactions, 7),
            [chip("\u{2764}", 1, false), chip(HEART, 1, false)]
        );
    }

    /// An emoji only blocked members chose keeps its row, naming nobody and
    /// leading with no face — exactly what the Swift code builds.
    #[test]
    fn a_blocked_only_emoji_keeps_a_nameless_row() {
        let reactions = [Reaction::new(11, HEART), Reaction::new(9, THUMBS_UP)];
        let details = reaction_details(
            &reactions,
            &roster(&[(9, "Anna"), (11, "Bob")]),
            7,
            &HashSet::from([11]),
        );
        assert_eq!(
            details,
            [
                detail(HEART, &[], None),
                detail(THUMBS_UP, &["Anna"], Some(9))
            ]
        );
        // The reader is never filtered, even from a corrupt block list.
        let mine = reaction_details(
            &[Reaction::new(7, HEART)],
            &HashMap::new(),
            7,
            &HashSet::from([7]),
        );
        assert_eq!(mine, [detail(HEART, &["You"], Some(7))]);
    }

    #[test]
    fn chip_taps_join_or_show_reactors_and_never_remove() {
        assert_eq!(chip(HEART, 3, false).tap(), ChipTap::Join);
        assert_eq!(chip(HEART, 3, true).tap(), ChipTap::ShowReactors);
        assert!(!chip(HEART, 1, true).shows_count());
        assert!(chip(HEART, 2, false).shows_count());
    }

    #[test]
    fn my_reaction_is_the_readers_own() {
        let reactions = [Reaction::new(9, HEART), Reaction::new(7, OPEN_MOUTH)];
        assert_eq!(my_reaction(&reactions, 7), Some(OPEN_MOUTH));
        assert_eq!(my_reaction(&reactions, 8), None);
        assert_eq!(my_reaction(&[], 7), None);
    }

    /// Replacing is remove-then-append: my new emoji moves to the END, it is
    /// not edited in place.
    #[test]
    fn toggle_replaces_by_appending() {
        let before = [Reaction::new(7, THUMBS_UP), Reaction::new(9, HEART)];
        let toggle = toggle_reaction(&before, 7, HEART);
        assert!(!toggle.removing);
        assert_eq!(
            toggle.reactions,
            [Reaction::new(9, HEART), Reaction::new(7, HEART)]
        );
        // Toggling somebody else's emoji is joining it, not removing it.
        let join = toggle_reaction(&[Reaction::new(9, HEART)], 7, HEART);
        assert!(!join.removing);
        assert_eq!(join.reactions.len(), 2);
    }
}
