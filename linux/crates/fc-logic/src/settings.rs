//! The words the settings screen says, and the two questions behind its rows (`Settings.cs`):
//! what leaving means ([`LeaveContext`]), a birthday's rules ([`birthday_rules`]) and the sentences
//! ([`settings_text`]) — each the web client's (`web/src/views/settings.rs`, `password.rs`,
//! `birthday.rs`), which are the Mac's.

use fc_core::protocol::MemberDto;

/// What leaving means for the person leaving.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaveKind {
    /// A member: they lose the chats, and their history returns if they rejoin.
    Member,
    /// The owner, and somebody inherits.
    Successor,
    /// The owner and the last member: leaving DELETES the family.
    LastMember,
}

/// The leave dialog's question, built from a FRESH roster.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaveContext {
    /// What leaving means for the person leaving.
    pub kind: LeaveKind,
    /// Who inherits, when an owner leaves and somebody does.
    pub successor_name: Option<String>,
    /// The family being left, when the account is in SEVERAL (docs/protocol.md, "Several
    /// families"): the dialog then names it and says what leaving one of them costs. `None` with
    /// one family — today's dialog, exactly.
    pub family_name: Option<String>,
}

impl LeaveContext {
    /// `new LeaveContext(kind)`.
    pub fn new(kind: LeaveKind) -> Self {
        LeaveContext {
            kind,
            successor_name: None,
            family_name: None,
        }
    }

    /// The same question, naming the family being left (one of several).
    pub fn in_family(mut self, family_name: &str) -> Self {
        self.family_name = Some(family_name.to_string());
        self
    }

    /// `LeaveContext.For` — the web client's `leave_context`. A successor the fresh roster does not
    /// name is NO answer (`None`): the dialog says leaving failed rather than guess — and a read
    /// that FAILED is not a family with nobody left in it, so "leaving deletes the family" is said
    /// only when the server said so.
    pub fn for_leaving(
        owner: bool,
        deletes_family: bool,
        successor: Option<&MemberDto>,
    ) -> Option<LeaveContext> {
        if !owner {
            return Some(LeaveContext::new(LeaveKind::Member));
        }
        if deletes_family {
            return Some(LeaveContext::new(LeaveKind::LastMember));
        }
        successor.map(|heir| LeaveContext {
            kind: LeaveKind::Successor,
            successor_name: Some(heir.display_name.clone()),
            family_name: None,
        })
    }
}

/// A birthday as the server takes it: a day and a month, no year (`BirthdayRules`).
pub mod birthday_rules {
    use fc_core::protocol::BirthdayDto;
    use fc_core::text::string_catalog::StringCatalog;
    use time::{Date, Month, PrimitiveDateTime, Time};

    use crate::date_culture::DateCulture;

    /// February has 29: a birthday has no year for the 29th to fail to exist in.
    pub fn days_in(month: i32) -> i32 {
        match month {
            2 => 29,
            4 | 6 | 9 | 11 => 30,
            _ => 31,
        }
    }

    pub fn ok(month: i32, day: i32) -> bool {
        (1..=12).contains(&month) && day >= 1 && day <= days_in(month)
    }

    /// Where the picker opens: unset on the 1st of January, a held one on itself, held inside the
    /// calendar. Answers `(month, day)`.
    pub fn start(held: Option<&BirthdayDto>) -> (i32, i32) {
        let Some(held) = held else {
            return (1, 1);
        };
        let month = held.month.clamp(1, 12);
        (month, held.day.clamp(1, days_in(month)))
    }

    /// "March 12", "12 марта" — the reader's own way of writing a day of a month (.NET's
    /// standard format `M` on a day of 2024, a leap year).
    pub fn text(
        birthday: Option<&BirthdayDto>,
        say: &dyn StringCatalog,
        culture: &DateCulture,
    ) -> String {
        match birthday {
            Some(held) if ok(held.month, held.day) => {
                let date = u8::try_from(held.month)
                    .ok()
                    .and_then(|month| Month::try_from(month).ok())
                    .zip(u8::try_from(held.day).ok())
                    .and_then(|(month, day)| Date::from_calendar_date(2024, month, day).ok());
                match date {
                    Some(date) => culture.format(PrimitiveDateTime::new(date, Time::MIDNIGHT), "M"),
                    None => say.get("Not set").to_string(),
                }
            }
            _ => say.get("Not set").to_string(),
        }
    }

    /// A month standing alone, as a picker lists it: "January", "январь".
    pub fn month_name(month: i32, culture: &DateCulture) -> String {
        u8::try_from(month)
            .map(|month| culture.month_name(month).to_string())
            .unwrap_or_default()
    }
}

/// The words the settings screen says (`SettingsText`).
pub mod settings_text {
    use fc_core::protocol::requests::{StatsAiDto, StatsMediaDto, StatsMemberDto};
    use fc_core::protocol::{ApiError, error_codes};
    use fc_core::text::media_text;
    use fc_core::text::string_catalog::StringCatalog;

    use super::{LeaveContext, LeaveKind};

    /// The fewest characters a password may have, counted in SCALARS as the server counts.
    pub const MIN_PASSWORD_CHARS: usize = 8;

    pub fn leave_message(context: &LeaveContext, say: &dyn StringCatalog) -> String {
        match context.kind {
            // One of several: direct chats stay with anybody shared elsewhere, so neither sentence
            // below is true of them. Leaving as the LAST member still deletes the family, and that
            // is still said in so many words.
            LeaveKind::Member | LeaveKind::Successor if context.family_name.is_some() => say
                .get("You'll lose access to this family's chats. Direct chats stay with anyone you share another family with.")
                .to_string(),
            LeaveKind::Successor => say.format(
                "%@ becomes the owner. You'll lose access to the family chat and your direct chats; your history returns if you rejoin.",
                &[&context.successor_name.as_deref().unwrap_or("")],
            ),
            LeaveKind::LastMember => say
                .get("You're the only member left. Leaving deletes the family and everything in it.")
                .to_string(),
            LeaveKind::Member => say
                .get("You'll lose access to the family chat and your direct chats. Your history returns if you rejoin.")
                .to_string(),
        }
    }

    /// Leaving as the last member DELETES the family: a different question, and a different
    /// button.
    pub fn leave_title(context: &LeaveContext, say: &dyn StringCatalog) -> String {
        if context.kind == LeaveKind::LastMember {
            say.get("Delete the family?").to_string()
        } else if let Some(name) = &context.family_name {
            say.format("Leave “%@”?", &[name])
        } else {
            say.get("Leave the family?").to_string()
        }
    }

    pub fn leave_button(context: &LeaveContext, say: &dyn StringCatalog) -> String {
        if context.kind == LeaveKind::LastMember {
            say.get("Leave and Delete").to_string()
        } else {
            say.get("Leave Family").to_string()
        }
    }

    pub fn leave_failed(say: &dyn StringCatalog) -> String {
        say.get("Couldn't leave right now. Try again.").to_string()
    }

    /// An owner who left handed the family on: who has it now (ios and web "Ownership passed
    /// on"). Answers `(title, message)`.
    pub fn ownership_passed(heir: &str, say: &dyn StringCatalog) -> (String, String) {
        (
            say.get("Ownership passed on").to_string(),
            say.format("%@ is now the owner of the family.", &[&heir]),
        )
    }

    /// Why a picture did not go up, or come down (ios AvatarFailure).
    pub fn picture_failure(error: &ApiError, uploading: bool, say: &dyn StringCatalog) -> String {
        let code = error.code.as_str();
        if code == error_codes::AVATAR_TOO_LARGE || error.status == 413 {
            say.get("That photo is too large for this server.")
        } else if code == error_codes::INVALID_IMAGE {
            say.get("That file isn't a photo we can use.")
        } else if code == error_codes::TRANSPORT {
            say.get("Can't reach the server. Check your connection.")
        } else if code == error_codes::TOO_MANY_REQUESTS || error.status == 429 {
            say.get("The server is busy. Try again in a moment.")
        } else if uploading {
            say.get("Couldn't upload the photo.")
        } else {
            say.get("Couldn't remove the photo.")
        }
        .to_string()
    }

    pub fn delete_failure(error: &ApiError, say: &dyn StringCatalog) -> String {
        match error.code.as_str() {
            error_codes::INVALID_CREDENTIALS => say.get("That password is not right."),
            error_codes::VALIDATION => say.get("Type your password to confirm."),
            _ => say.get("Couldn't delete your account. Try again."),
        }
        .to_string()
    }

    /// What is wrong with a new password and its confirmation, or `None`.
    pub fn password_problem(
        new_password: &str,
        confirmation: &str,
        say: &dyn StringCatalog,
    ) -> Option<String> {
        if new_password.chars().count() < MIN_PASSWORD_CHARS {
            return Some(say.format("Use at least %lld characters.", &[&MIN_PASSWORD_CHARS]));
        }
        if new_password == confirmation {
            None
        } else {
            Some(say.get("Those two do not match.").to_string())
        }
    }

    pub fn password_change_failure(error: &ApiError, say: &dyn StringCatalog) -> String {
        if error.code == error_codes::INVALID_CREDENTIALS {
            say.get("That current password is not right.").to_string()
        } else {
            say.get("Couldn't change your password. Try again.")
                .to_string()
        }
    }

    pub fn birthday_failure(error: &ApiError, say: &dyn StringCatalog) -> String {
        match error.code.as_str() {
            error_codes::VALIDATION => say.get("That date doesn't exist."),
            error_codes::NOT_FAMILY_OWNER => say.get("Only the family owner can do that."),
            _ => say.get("Couldn't save that birthday. Try again."),
        }
        .to_string()
    }

    /// What one member sends, besides words (ios StatisticsView). `decimal_separator` is the
    /// reader's, for the size ("1,2 MB" in German) — the C#'s culture argument.
    pub fn member_line(
        member: &StatsMemberDto,
        say: &dyn StringCatalog,
        decimal_separator: &str,
    ) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(files) = member.attachments.as_ref().filter(|files| files.count > 0) {
            // The count chooses the form; the arguments are the key's own, and a translation may
            // say the size first.
            let size = media_text::display_size(files.bytes.max(0), decimal_separator);
            parts.push(say.plural(
                "%lld attachments, %@",
                i64::from(files.count),
                &[&files.count, &size],
            ));
        }
        if let Some(ai) = &member.ai {
            if ai.questions > 0 {
                parts.push(say.plural(
                    "%lld questions to the assistant",
                    i64::from(ai.questions),
                    &[&ai.questions],
                ));
            }
            if ai.images > 0 {
                parts.push(say.plural(
                    "%lld pictures from the assistant",
                    i64::from(ai.images),
                    &[&ai.images],
                ));
            }
            if ai.transcripts > 0 {
                // Charged to the member who ASKED, whoever's voice it was (docs/protocol.md,
                // "Family statistics").
                parts.push(say.plural(
                    "%lld recordings as text",
                    i64::from(ai.transcripts),
                    &[&ai.transcripts],
                ));
            }
            if ai.searches > 0 {
                // The PAID web searches made for their questions, charged to whoever asked; weather
                // and Wikipedia are free and not counted (docs/protocol.md, "Family statistics",
                // amended 2026-10-03).
                parts.push(say.plural(
                    "%lld web searches",
                    i64::from(ai.searches),
                    &[&ai.searches],
                ));
            }
        }
        if parts.is_empty() {
            say.get("Words only").to_string()
        } else {
            parts.join(" · ")
        }
    }

    /// Whether the family's totals have an Assistant group to show: anything the assistant was
    /// asked for, drew, heard or looked up. A family that never used it has no group, rather than
    /// a group of zeros.
    pub fn assistant_used(ai: Option<&StatsAiDto>) -> bool {
        ai.is_some_and(|ai| {
            ai.questions > 0 || ai.images > 0 || ai.transcripts > 0 || ai.searches > 0
        })
    }

    /// What storing one copy of identical files saved, or `None` when it saved nothing — the
    /// family's own totals, never the rows added up: the gap between those is the reader's block
    /// list.
    pub fn saved(files: Option<&StatsMediaDto>) -> Option<i64> {
        let files = files?;
        let stored = files.stored_bytes?;
        let saved = files.bytes.max(0) - stored.max(0);
        (saved > 0).then_some(saved)
    }
}
