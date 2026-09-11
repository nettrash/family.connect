//! The rules an account and a family are held to, the server's own
//! (server/src/handlers_auth.rs, handlers_family.rs) — checked here first so
//! a person is told at the field, not by a refused request — and the owner's
//! member cap (ios Models/MemberCap.swift, less its one mistake: it reads the
//! owner's cap alone, where the door reads the lower of it and the server's
//! ceiling).
//!
//! Counts are in Unicode SCALARS and trimming is Rust's, because both are
//! what the server does: a web client written in the same language cannot
//! disagree with it about where a name starts or how long it is.
use crate::i18n::{t, tn, tp};

/// The fewest characters a password may have.
pub const MIN_PASSWORD_CHARS: usize = 8;

/// Why a username cannot be registered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsernameProblem {
    /// Not 3–32 characters.
    Length,
    /// Something other than letters, digits, `_` and `.`.
    Characters,
    /// A name the server keeps for itself — the assistant's.
    Reserved,
}

impl UsernameProblem {
    pub fn message(self) -> &'static str {
        match self {
            UsernameProblem::Length => t("A username is 3 to 32 characters."),
            UsernameProblem::Characters => {
                t("A username may use only letters, digits, “_” and “.”.")
            }
            UsernameProblem::Reserved => t("That username is reserved."),
        }
    }
}

/// What is wrong with `username`, if anything. Not trimmed: the server takes
/// it exactly as sent, so a stray space is a character it refuses.
pub fn username_problem(username: &str) -> Option<UsernameProblem> {
    let length = username.chars().count();
    if !(3..=32).contains(&length) {
        return Some(UsernameProblem::Length);
    }
    if !username
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
    {
        return Some(UsernameProblem::Characters);
    }
    if username.eq_ignore_ascii_case("assistant") {
        return Some(UsernameProblem::Reserved);
    }
    None
}

/// Whether `password` is long enough to set.
pub fn password_ok(password: &str) -> bool {
    password.chars().count() >= MIN_PASSWORD_CHARS
}

/// A display name or a family name as the server will store it — trimmed,
/// 1–64 characters — or None when it would be refused.
pub fn name(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (1..=64)
        .contains(&trimmed.chars().count())
        .then_some(trimmed)
}

/// An invite code as the server reads it: trimmed, and uppercased — so a
/// code typed in lower case, or pasted with a space, is the same code.
pub fn invite_code(value: &str) -> String {
    value.trim().to_uppercase()
}

/// How many days `month` has, for a birthday. February has 29: a birthday
/// has no year for the 29th to fail to exist in (docs/protocol.md,
/// "Birthdays").
pub fn days_in(month: u32) -> u32 {
    match month {
        2 => 29,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Whether `month` and `day` name a birthday the server accepts.
pub fn birthday_ok(month: u32, day: u32) -> bool {
    (1..=12).contains(&month) && (1..=days_in(month)).contains(&day)
}

/// The nine languages a family may declare, as the protocol spells them,
/// each named in itself — a family picks the language they speak by the
/// name they know it by (docs/protocol.md, "The family's language").
pub const LANGUAGES: [(&str, &str); 9] = [
    ("en", "English"),
    ("de", "Deutsch"),
    ("es", "Español"),
    ("fr", "Français"),
    ("ja", "日本語"),
    ("ru", "Русский"),
    ("sr", "Српски"),
    ("sr-Latn", "Srpski (latinica)"),
    ("zh-Hans", "简体中文"),
];

/// What the member-limit footer says: three different sentences.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapState {
    /// No cap of the owner's own: the operator's ceiling binds.
    OpenToCeiling { ceiling: i64 },
    /// The door is at or below the roster. Legal and deliberate — an owner
    /// who inherits a large family must still be able to shut the door —
    /// and NOBODY is removed: the cap is read at the join door, never over
    /// the room.
    Frozen { members: i64 },
    /// Room to spare: `seats` is the lower of the cap and the ceiling.
    Room { members: i64, seats: i64 },
}

/// The seats a family actually has. The door takes the LOWER of the
/// owner's cap and the operator's ceiling, and they may disagree for good: a
/// cap of 40 set under a ceiling of 50 stays 40 after the ceiling drops to
/// 10, because a stored cap is never re-validated (docs/protocol.md,
/// `Family`). So the cap is shown as the owner's own setting, and this is
/// what the footer counts against.
pub fn cap_state(cap: Option<i64>, members: i64, ceiling: i64) -> CapState {
    match cap {
        None => CapState::OpenToCeiling { ceiling },
        Some(cap) => {
            let seats = cap.min(ceiling);
            if seats <= members {
                CapState::Frozen { members }
            } else {
                CapState::Room { members, seats }
            }
        }
    }
}

/// The footer under the member limit, in words (ios MacFamilyView). The
/// singular the Apple catalogue lacks — "1 members now" — is in the web's
/// own English beside it, so the sentence is right in English and still
/// translated everywhere else.
pub fn cap_footer(state: CapState) -> String {
    match state {
        CapState::OpenToCeiling { ceiling } => tn(
            "No limit of your own. This server allows up to %lld members in a family.",
            ceiling,
        ),
        CapState::Frozen { members } => tn(
            "%lld members now. Nobody new can join until somebody leaves; no one is removed.",
            members,
        ),
        CapState::Room { members, seats } => tp(
            "%lld of %lld seats used.",
            members,
            &[&members.to_string(), &seats.to_string()],
        ),
    }
}

/// A cap the owner stepped or typed to, held inside 1..=ceiling.
pub fn clamp_cap(value: i64, ceiling: i64) -> i64 {
    value.max(1).min(ceiling.max(1))
}

/// The cap proposed when the owner first turns the limit on: the family
/// frozen where it stands, which is what reaching for "limit members" almost
/// always means — held inside the same bounds, so a family already larger
/// than a lowered ceiling does not open a control seeded out of its range.
pub fn seed_cap(members: i64, ceiling: i64) -> i64 {
    clamp_cap(members, ceiling)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_username_is_three_to_thirty_two_of_the_allowed_characters() {
        assert_eq!(username_problem("anna"), None);
        assert_eq!(username_problem("a.b_c9"), None);
        assert_eq!(username_problem("ab"), Some(UsernameProblem::Length));
        assert_eq!(username_problem(&"a".repeat(32)), None);
        assert_eq!(
            username_problem(&"a".repeat(33)),
            Some(UsernameProblem::Length)
        );
        assert_eq!(
            username_problem("anna smith"),
            Some(UsernameProblem::Characters)
        );
        assert_eq!(
            username_problem(" anna"),
            Some(UsernameProblem::Characters),
            "not trimmed"
        );
        assert_eq!(username_problem("анна"), Some(UsernameProblem::Characters));
        assert_eq!(
            username_problem("anna-s"),
            Some(UsernameProblem::Characters)
        );
        assert_eq!(
            username_problem("Assistant"),
            Some(UsernameProblem::Reserved)
        );
        assert_eq!(username_problem("assistant2"), None);
        // Counted in scalars, as the server counts: two wide characters are
        // two, and still refused for what they are.
        assert_eq!(username_problem("éé"), Some(UsernameProblem::Length));
    }

    #[test]
    fn a_password_is_eight_characters_or_more() {
        assert!(!password_ok("1234567"));
        assert!(password_ok("12345678"));
        assert!(password_ok("пароль12"), "eight scalars, sixteen bytes");
        assert!(!password_ok("пароль1"));
    }

    #[test]
    fn a_name_is_trimmed_and_one_to_sixty_four() {
        assert_eq!(name("  The Smiths "), Some("The Smiths"));
        assert_eq!(name("   "), None);
        assert_eq!(name(""), None);
        assert_eq!(name(&"я".repeat(64)).map(|n| n.chars().count()), Some(64));
        assert_eq!(name(&"я".repeat(65)), None);
    }

    #[test]
    fn an_invite_code_is_read_as_the_server_reads_it() {
        assert_eq!(invite_code(" abcd2345 "), "ABCD2345");
        assert_eq!(invite_code("ABCD2345"), "ABCD2345");
    }

    #[test]
    fn a_birthday_is_a_day_that_month_has_and_the_29th_of_february_is_one() {
        assert!(birthday_ok(2, 29));
        assert!(!birthday_ok(2, 30));
        assert!(!birthday_ok(4, 31));
        assert!(birthday_ok(12, 31));
        assert!(!birthday_ok(13, 1));
        assert!(!birthday_ok(0, 1));
        assert!(!birthday_ok(1, 0));
        assert_eq!((1..=12).map(days_in).sum::<u32>(), 366);
    }

    #[test]
    fn the_languages_are_the_protocols_nine_in_its_spelling() {
        let codes: Vec<&str> = LANGUAGES.iter().map(|(code, _)| *code).collect();
        assert_eq!(
            codes,
            ["en", "de", "es", "fr", "ja", "ru", "sr", "sr-Latn", "zh-Hans"]
        );
    }

    /// MemberCap, case for case.
    #[test]
    fn the_member_cap_freezes_at_or_below_the_roster_and_stays_in_range() {
        assert_eq!(
            cap_state(None, 4, 50),
            CapState::OpenToCeiling { ceiling: 50 }
        );
        assert_eq!(cap_state(Some(4), 4, 50), CapState::Frozen { members: 4 });
        assert_eq!(cap_state(Some(3), 4, 50), CapState::Frozen { members: 4 });
        assert_eq!(
            cap_state(Some(8), 4, 50),
            CapState::Room {
                members: 4,
                seats: 8
            }
        );
        assert_eq!(seed_cap(4, 50), 4);
        assert_eq!(seed_cap(0, 50), 1, "never 0");
        assert_eq!(seed_cap(60, 50), 50, "never above a lowered ceiling");
        assert_eq!(clamp_cap(0, 50), 1);
        assert_eq!(clamp_cap(99, 50), 50);
        assert_eq!(clamp_cap(5, 0), 1, "a ceiling of nothing is still 1");
    }

    /// The door takes the LOWER of the two, and so does the footer: a cap
    /// of 40 under a ceiling since dropped to 10 is ten seats, and a family
    /// of twelve there is shut — not "12 of 40 seats used", which is what
    /// MemberCap.swift would say.
    #[test]
    fn the_seats_are_the_lower_of_the_cap_and_the_ceiling() {
        assert_eq!(
            cap_state(Some(40), 4, 10),
            CapState::Room {
                members: 4,
                seats: 10
            }
        );
        assert_eq!(
            cap_state(Some(40), 12, 10),
            CapState::Frozen { members: 12 }
        );
        assert_eq!(
            cap_state(Some(40), 10, 10),
            CapState::Frozen { members: 10 }
        );
        assert_eq!(
            cap_state(Some(6), 4, 10),
            CapState::Room {
                members: 4,
                seats: 6
            }
        );
    }

    #[test]
    fn the_footer_says_each_state_in_words() {
        assert_eq!(
            cap_footer(cap_state(None, 3, 50)),
            "No limit of your own. This server allows up to 50 members in a family."
        );
        assert_eq!(
            cap_footer(cap_state(Some(2), 3, 50)),
            "3 members now. Nobody new can join until somebody leaves; no one is removed."
        );
        assert_eq!(
            cap_footer(cap_state(Some(1), 1, 50)),
            "1 member now. Nobody new can join until somebody leaves; no one is removed."
        );
        assert_eq!(
            cap_footer(cap_state(Some(40), 4, 10)),
            "4 of 10 seats used."
        );
    }
}
