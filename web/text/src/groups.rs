//! Groups inside a family (#107; docs/protocol.md, "Groups", and the
//! decisions in docs/groups-2026-10-21.md): what a chat of each kind
//! offers, who in a group may do what, the name's bounds, and what each of
//! the server's refusals is told as.
//!
//! One table every surface reads rather than a `kind == "family"` at each
//! call site — the design asks for exactly that ("one shared table per
//! client, pinned by tests, as the transcript and consent rules are"), and
//! a check that forgot the group is how a group would quietly lose its
//! mentions or grow a call button.

use crate::i18n::t;

/// The chat kinds on the wire (docs/protocol.md, "Objects" — `Chat`).
pub const FAMILY: &str = "family";
pub const DIRECT: &str = "direct";
pub const AI: &str = "ai";
pub const GROUP: &str = "group";

/// The kind as one of the four this client knows, for a rule that keeps
/// it past the moment it was read — and `""`, which offers nothing and
/// reaches nothing, for a kind from a newer server. Never a guess: a kind
/// it does not know is not a direct chat.
pub fn known(kind: &str) -> &'static str {
    match kind {
        FAMILY => FAMILY,
        DIRECT => DIRECT,
        AI => AI,
        GROUP => GROUP,
        _ => "",
    }
}

/// A group's name is trimmed and must then be 1 to 64 characters.
pub const MAX_TITLE_CHARS: usize = 64;

/// A family has at most this many groups; one more is `too_many_groups`.
pub const MAX_GROUPS: usize = 50;

/// What a chat of one kind offers, in the words the surfaces use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Offers {
    /// A sender's name above their bubbles — many people write here.
    pub sender_names: bool,
    /// "Seen" ticks under my messages — one reader, so it means something.
    pub seen: bool,
    /// `@Name` naming a member (docs/protocol.md, "Mentioning a member").
    pub mentions: bool,
    /// Starting a poll (docs/protocol.md, "Polls").
    pub polls: bool,
    /// `@ai` answered in the chat, for everyone in it to read.
    pub assistant_mention: bool,
    /// A voice or video call (docs/protocol.md, "Voice calls").
    pub calls: bool,
    /// A video message — never in the assistant's own chat, which cannot
    /// watch one.
    pub video_messages: bool,
}

/// What a chat of `kind` offers. A group is the family chat with fewer
/// people — everything the family chat offers, and not calls or "seen"
/// (docs/protocol.md, "Groups"). A kind this client does not know offers
/// nothing beyond text, as the protocol says an old client treats one.
pub fn offers(kind: &str) -> Offers {
    match kind {
        FAMILY | GROUP => Offers {
            sender_names: true,
            seen: false,
            mentions: true,
            polls: true,
            assistant_mention: true,
            calls: false,
            video_messages: true,
        },
        DIRECT => Offers {
            seen: true,
            calls: true,
            video_messages: true,
            ..Offers::default()
        },
        AI => Offers {
            seen: true,
            ..Offers::default()
        },
        _ => Offers {
            seen: true,
            ..Offers::default()
        },
    }
}

/// Something a member may do to a group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Power {
    Rename,
    AddMembers,
    RemoveMember,
    Delete,
    Leave,
}

/// May `me` do `power` to a group with this admin and these members?
///
/// D1 (a): the admin — the member who created it, or the one who inherited
/// it — renames it, adds and removes members, and deletes it; every member
/// may leave. Nobody outside it may do anything, because nobody outside it
/// can even see it.
pub fn may(power: Power, me: i64, admin_user_id: Option<i64>, member_ids: &[i64]) -> bool {
    if me <= 0 || !member_ids.contains(&me) {
        return false;
    }
    match power {
        Power::Leave => true,
        Power::Rename | Power::AddMembers | Power::RemoveMember | Power::Delete => {
            admin_user_id == Some(me)
        }
    }
}

/// May `me`, the admin, remove `member`? Anybody in it but themself — the
/// admin leaves by Leave, which hands the group on.
pub fn may_remove(me: i64, member: i64, admin_user_id: Option<i64>, member_ids: &[i64]) -> bool {
    member != me
        && member_ids.contains(&member)
        && may(Power::RemoveMember, me, admin_user_id, member_ids)
}

/// The name as it goes to the server — trimmed, 1 to 64 characters — or
/// None when the server would answer `validation`. Characters as the server
/// counts them: Unicode scalar values.
pub fn title(raw: &str) -> Option<&str> {
    let trimmed = raw.trim();
    let count = trimmed.chars().count();
    (1..=MAX_TITLE_CHARS).contains(&count).then_some(trimmed)
}

/// Why a new group cannot be asked for yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    /// The name, trimmed, is empty or longer than 64 characters.
    Name,
    /// Nobody else is chosen: a group starts with at least two people.
    NoMembers,
}

impl Problem {
    pub fn sentence(self) -> &'static str {
        match self {
            Problem::Name => t("A group's name can be 1 to 64 characters."),
            Problem::NoMembers => t("Choose at least one member."),
        }
    }
}

/// What stands between this form and `POST /chats/groups`, the name first.
pub fn check_new(raw_title: &str, chosen: usize) -> Result<&str, Problem> {
    let title = title(raw_title).ok_or(Problem::Name)?;
    if chosen == 0 {
        return Err(Problem::NoMembers);
    }
    Ok(title)
}

/// What a refusal of `POST /chats/groups` is told as.
pub fn create_refusal(code: Option<&str>) -> &'static str {
    match code {
        Some("too_many_groups") => t("Your family already has 50 groups."),
        Some("validation") => t("A group's name can be 1 to 64 characters."),
        _ => t("Couldn't create the group."),
    }
}

/// What a refusal of renaming, adding, removing, leaving or deleting is
/// told as. `renaming`: a `validation` there is the name.
pub fn change_refusal(code: Option<&str>, renaming: bool) -> &'static str {
    match code {
        Some("not_group_admin") => t("Only the group's admin can do this."),
        Some("validation") if renaming => t("A group's name can be 1 to 64 characters."),
        _ => t("Couldn't change the group."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: i64 = 7;
    const ANNA: i64 = 9;
    const BOB: i64 = 11;

    /// What every kind offers — the table of docs/protocol.md "Groups":
    /// a group is the family chat less calls and "seen".
    #[test]
    fn what_each_kind_offers() {
        // (kind, sender names, seen, mentions, polls, @ai, calls, video)
        for (kind, names, seen, mentions, polls, ai, calls, video) in [
            (FAMILY, true, false, true, true, true, false, true),
            (GROUP, true, false, true, true, true, false, true),
            (DIRECT, false, true, false, false, false, true, true),
            (AI, false, true, false, false, false, false, false),
            ("channel", false, true, false, false, false, false, false),
        ] {
            assert_eq!(
                offers(kind),
                Offers {
                    sender_names: names,
                    seen,
                    mentions,
                    polls,
                    assistant_mention: ai,
                    calls,
                    video_messages: video,
                },
                "{kind}"
            );
        }
    }

    #[test]
    fn a_kind_it_does_not_know_is_nothing_in_particular() {
        for kind in [FAMILY, DIRECT, AI, GROUP] {
            assert_eq!(known(kind), kind);
        }
        assert_eq!(known("channel"), "");
        assert_eq!(offers(known("channel")), offers("channel"));
    }

    /// Who may do what (D1 a): the admin everything, every member Leave,
    /// anybody outside nothing.
    #[test]
    fn who_may_do_what() {
        let members = [ME, ANNA, BOB];
        // (power, as admin, as a member, as somebody outside)
        for (power, admin, member, outside) in [
            (Power::Rename, true, false, false),
            (Power::AddMembers, true, false, false),
            (Power::RemoveMember, true, false, false),
            (Power::Delete, true, false, false),
            (Power::Leave, true, true, false),
        ] {
            assert_eq!(may(power, ME, Some(ME), &members), admin, "{power:?} admin");
            assert_eq!(
                may(power, ANNA, Some(ME), &members),
                member,
                "{power:?} member"
            );
            assert_eq!(
                may(power, 42, Some(42), &members),
                outside,
                "{power:?} outside"
            );
        }
        // An admin who is no longer in it is nobody's admin here, and a
        // group whose admin went with ON DELETE SET NULL has none.
        assert!(!may(Power::Rename, ANNA, None, &members));
        assert!(may(Power::Leave, ANNA, None, &members));
        assert!(!may(Power::Leave, 0, None, &[0]));
    }

    #[test]
    fn the_admin_removes_anybody_but_themself() {
        let members = [ME, ANNA];
        assert!(may_remove(ME, ANNA, Some(ME), &members));
        assert!(!may_remove(ME, ME, Some(ME), &members));
        assert!(!may_remove(ME, BOB, Some(ME), &members), "not in it");
        assert!(!may_remove(ANNA, ME, Some(ME), &members), "not the admin");
    }

    #[test]
    fn a_name_is_trimmed_and_then_1_to_64_characters() {
        assert_eq!(title("  Cousins  "), Some("Cousins"));
        assert_eq!(title(""), None);
        assert_eq!(title("   \n\t "), None);
        let longest = "é".repeat(MAX_TITLE_CHARS);
        assert_eq!(
            title(&longest),
            Some(longest.as_str()),
            "characters, not bytes"
        );
        assert_eq!(title(&"a".repeat(MAX_TITLE_CHARS + 1)), None);
        assert_eq!(
            title(&format!(" {} ", "a".repeat(64))).map(str::len),
            Some(64)
        );
    }

    #[test]
    fn a_new_group_needs_a_name_and_somebody_else() {
        assert_eq!(check_new("Cousins", 1), Ok("Cousins"));
        assert_eq!(check_new("  ", 2), Err(Problem::Name));
        assert_eq!(check_new("Cousins", 0), Err(Problem::NoMembers));
        assert_eq!(check_new("", 0), Err(Problem::Name), "the name first");
        assert_eq!(Problem::NoMembers.sentence(), "Choose at least one member.");
    }

    #[test]
    fn each_refusal_says_what_happened() {
        assert_eq!(
            create_refusal(Some("too_many_groups")),
            "Your family already has 50 groups."
        );
        assert_eq!(
            create_refusal(Some("validation")),
            "A group's name can be 1 to 64 characters."
        );
        for code in [Some("not_same_family"), Some("internal"), None] {
            assert_eq!(create_refusal(code), "Couldn't create the group.");
        }
        assert_eq!(
            change_refusal(Some("not_group_admin"), false),
            "Only the group's admin can do this."
        );
        assert_eq!(
            change_refusal(Some("validation"), true),
            "A group's name can be 1 to 64 characters."
        );
        for code in [Some("validation"), Some("chat_not_found"), None] {
            assert_eq!(change_refusal(code, false), "Couldn't change the group.");
        }
    }
}
