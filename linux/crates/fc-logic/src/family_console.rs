//! The family console's rules and words — the web client's family pane (`web/src/views/family.rs`,
//! `report.rs`), which is the Mac's with the iPhone's corrections (`FamilyConsole.cs`).

/// What one member row offers the person looking at it (`MemberTools`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct MemberTools {
    /// A direct chat — never with yourself, and never with somebody you blocked, which would only
    /// be refused.
    pub message: bool,
    /// Report and block: anybody but yourself.
    pub safety: bool,
    /// The owner's tools: a birthday for anyone, themselves included.
    pub owner_menu: bool,
    /// A removal: never yourself, never the owner.
    pub removable: bool,
    /// A password reset: where a removal is, and only for a member whose every family this owner
    /// owns — the server's `password_reset` (docs/protocol.md, "Several families", D7;
    /// [`family_rules::may_reset_password`](crate::families::family_rules::may_reset_password)).
    pub reset_password: bool,
}

impl MemberTools {
    /// `new MemberTools(message, safety, ownerMenu, removable, resetPassword)`.
    pub const fn new(
        message: bool,
        safety: bool,
        owner_menu: bool,
        removable: bool,
        reset_password: bool,
    ) -> Self {
        MemberTools {
            message,
            safety,
            owner_menu,
            removable,
            reset_password,
        }
    }
}

/// The family console's rules and words (`FamilyText`).
pub mod family_text {
    use fc_core::protocol::{ApiError, AttachmentDto, MemberDto, ReportDto, error_codes};
    use fc_core::text::string_catalog::StringCatalog;

    use super::MemberTools;
    use crate::families::family_rules;

    /// The protocol's four reasons, in the apps' order, each with the key it is said by.
    pub const REASONS: [(&str, &str); 4] = [
        ("spam", "Spam"),
        ("harassment", "Harassment"),
        ("inappropriate", "Inappropriate"),
        ("other", "Something else"),
    ];

    /// Chosen to start with: the reason reporting most exists for.
    pub const FIRST_REASON: &str = "harassment";

    /// The members still in the family, sorted by name as the Mac sorts — without regard to case.
    ///
    /// The C#'s key, kept: `DisplayName.ToLowerInvariant()` compared ORDINALLY — .NET's simple
    /// (one character for one) lower-case mapping, then UTF-16 code units — and a stable sort, so
    /// two names that fold alike keep the roster's order.
    pub fn roster<'a>(members: impl IntoIterator<Item = &'a MemberDto>) -> Vec<MemberDto> {
        let mut still: Vec<(Vec<u16>, MemberDto)> = members
            .into_iter()
            .filter(|member| !member.deleted && !member.is_former())
            .map(|member| (sort_key(&member.display_name), member.clone()))
            .collect();
        still.sort_by(|a, b| a.0.cmp(&b.0));
        still.into_iter().map(|(_, member)| member).collect()
    }

    /// `ToLowerInvariant()` as UTF-16 units: each character's SIMPLE lower case, which is the first
    /// character of Rust's full mapping (the one difference, `İ`, is `i` followed by a combining
    /// dot in the full mapping and `i` in the simple one).
    fn sort_key(name: &str) -> Vec<u16> {
        let lowered: String = name
            .chars()
            .map(|c| c.to_lowercase().next().unwrap_or(c))
            .collect();
        lowered.encode_utf16().collect()
    }

    pub fn tools_for(member: &MemberDto, me: i64, owner: bool, blocked: bool) -> MemberTools {
        let mine = member.id == me;
        MemberTools {
            message: !mine && !blocked,
            safety: !mine,
            owner_menu: owner,
            removable: owner && !mine && !member.owner(),
            reset_password: owner
                && !member.owner()
                && family_rules::may_reset_password(member, me, owner),
        }
    }

    pub fn members_line(count: i64, say: &dyn StringCatalog) -> String {
        say.plural("%lld members", count, &[&count])
    }

    /// A report's reason in words — one this client does not know is "Something else".
    pub fn reason_label(reason: &str, say: &dyn StringCatalog) -> String {
        let key = REASONS
            .iter()
            .find(|(code, _)| *code == reason)
            .map_or("Something else", |(_, key)| key);
        say.get(key).to_string()
    }

    pub fn reported(report: &ReportDto, say: &dyn StringCatalog) -> String {
        say.format(
            "%@ reported %@",
            &[&report.reporter.display_name, &report.reported.display_name],
        )
    }

    /// What a reported message carried, as a chat-list preview says it — for a photo sent without
    /// words, all there is to say about it.
    pub fn carried(
        attachments: Option<&[AttachmentDto]>,
        say: &dyn StringCatalog,
    ) -> Option<String> {
        let all = attachments.filter(|all| !all.is_empty())?;
        let first = &all[0];
        Some(match first.kind.as_str() {
            "photo" if all.len() > 1 => {
                let count = i64::try_from(all.len()).unwrap_or(i64::MAX);
                say.plural("%lld Photos", count, &[&count])
            }
            "photo" => say.get("Photo").to_string(),
            "video" => say.get("Video").to_string(),
            "audio" => say.get("Voice message").to_string(),
            "location" => say.get("Location").to_string(),
            _ => first
                .name
                .clone()
                .unwrap_or_else(|| say.get("File").to_string()),
        })
    }

    /// MANDATORY, and a protocol requirement rather than a nicety: somebody who reports without
    /// knowing the owner will read it has been surprised by their own app.
    pub fn report_disclosure(about_message: bool, say: &dyn StringCatalog) -> String {
        if about_message {
            say.get("Your family owner will see this message and its text.")
        } else {
            say.get("Your family owner will be told you reported this member.")
        }
        .to_string()
    }

    /// Who reads a report about the ASSISTANT, said before it is sent. NOT the family owner: a
    /// private assistant thread belongs to its member alone, so the people who run the server are
    /// the ones told — and a member reporting a reply out of that thread has to know that before
    /// they send it, not afterwards (docs/protocol.md, "Reporting the assistant").
    pub fn assistant_report_disclosure(say: &dyn StringCatalog) -> String {
        say.get(
            "The people who run this server will see this reply and what you write here. Your family will not.",
        )
        .to_string()
    }

    /// A failure with nothing more particular to say. The server ANSWERING with trouble of its own
    /// — `internal`, or a status with no protocol body, a proxy's 502 while it restarts — is not
    /// "can't reach the server", which would send somebody to check a network that is fine.
    pub fn generic_failure(error: &ApiError, say: &dyn StringCatalog) -> String {
        let code = error.code.as_str();
        if code == error_codes::TOO_MANY_REQUESTS || error.status == 429 {
            say.get("The server is busy. Try again in a moment.")
        } else if code == error_codes::INTERNAL || error.status >= 500 {
            say.get("The server had a problem. Try again in a moment.")
        } else if code == error_codes::TRANSPORT {
            say.get("Can't reach the server. Check your connection.")
        } else if error.status > 0 && !error.canonical() {
            say.get("The server had a problem. Try again in a moment.")
        } else {
            say.get("That didn't work. Try again.")
        }
        .to_string()
    }

    /// A full family leaves the request WAITING: full is a condition, not an answer.
    pub fn request_failure(error: &ApiError, say: &dyn StringCatalog) -> String {
        if error.code == error_codes::FAMILY_FULL {
            say.get(
                "The family is full. Raise the member limit or wait for somebody to leave — the request is still waiting.",
            )
            .to_string()
        } else {
            generic_failure(error, say)
        }
    }
}
