//! One account in several families (docs/protocol.md, "Several families";
//! docs/families-2026-10-22.md) — `Families.cs`: the rules that are not a screen
//! ([`family_rules`]), the family switcher, the invitation banner, the refusals several families
//! add, and the owner's "Invite…".

use std::cmp::Ordering;
use std::collections::HashSet;
use std::rc::Rc;

use fc_core::protocol::{
    ApiClient, ApiError, ChatDto, ChatRowDto, FamilyDto, InvitationDto, MemberDto, error_codes,
};
use fc_core::store::{ChatStore, StoreResult};
use fc_core::text::string_catalog::StringCatalog;

use crate::app_session::SessionState;

/// The rules of one account in several families that are not a screen: which chats a family shows,
/// which family an upload or a notification is for, and which doors to a further family are open
/// (`FamilyRules`).
pub mod family_rules {
    use super::*;

    /// The families `chat` is listed in: its `family_ids` — or, for a chat with none (cached
    /// before them, or from a server that predates them, where absent "means the one family the
    /// account has": docs/protocol.md, `Chat`), the HOME family `home` alone, as Android's
    /// `listedIn` and the web's `listed_in` read it. An explicit `[]` stays empty: a direct chat
    /// closed for the one who left is listed nowhere. (`fc_text::families::families_of`.)
    pub fn families_of(chat: &ChatDto, home: Option<i64>) -> Vec<i64> {
        fc_text::families::families_of(chat.family_ids.as_deref(), home)
    }

    /// Whether `chat` is shown in `family`: the chats whose families ([`families_of`]) name it. A
    /// direct chat names every family the two share, so it is shown in each; a chat with no
    /// `family_ids` is shown in the home family only. A session with no family shown shows it.
    pub fn lists(chat: &ChatDto, family: Option<i64>, home: Option<i64>) -> bool {
        family.is_none_or(|shown| families_of(chat, home).contains(&shown))
    }

    /// The family an upload for `chat` is FOR: the one shown when the chat is listed there, else
    /// the first the chat is listed in — a message may use an attachment only in a chat its family
    /// lists, and a chat with no `family_ids` is the home family's — else the one shown.
    pub fn upload_family(
        chat: Option<&ChatDto>,
        shown: Option<i64>,
        home: Option<i64>,
    ) -> Option<i64> {
        if let Some(chat) = chat {
            let families = families_of(chat, home);
            if let Some(first) = families.first()
                && !shown.is_some_and(|current| families.contains(&current))
            {
                return Some(*first);
            }
        }
        shown
    }

    /// The family to SHOW to open `chat` — from a notification, say: the one shown when it lists
    /// the chat, else the first that does (docs/protocol.md, "Pushes": "in the family the app is
    /// showing when the chat is listed there, else in the first that lists it"). `None` when
    /// nothing needs to change — and for a chat listed in NO family (`family_ids: []`, a direct
    /// chat closed when the last family the two shared went), which has no family to move to: it
    /// opens where the reader is, as Android's `familyForChat` leaves the app where it is.
    pub fn family_to_open(chat: &ChatDto, shown: Option<i64>, home: Option<i64>) -> Option<i64> {
        if lists(chat, shown, home) {
            return None;
        }
        families_of(chat, home).first().copied()
    }

    /// The name of the family a chat belongs to — the first it is listed in, the home family (the
    /// account's first) for a chat with no `family_ids` — from the account's list, for a
    /// notification's title; `None` when the list does not name it.
    pub fn name_of_chat<'a>(chat: &ChatDto, families: &'a [FamilyDto]) -> Option<&'a str> {
        let ids = families_of(chat, families.first().map(|family| family.id));
        name_of(ids.first().copied(), families)
    }

    /// One family's name from the account's list, or `None`.
    pub fn name_of(family_id: Option<i64>, families: &[FamilyDto]) -> Option<&str> {
        let id = family_id?;
        families
            .iter()
            .find(|family| family.id == id)
            .map(|family| family.name.as_str())
    }

    /// Whether the switcher is drawn at all: two families or more, or a pending invitation or join
    /// request to show. An account in ONE family with nothing pending sees nothing of several
    /// families.
    ///
    /// Not `fc_text::families::shows_switcher`, which also draws it for an account in NO family
    /// with something pending — the C# (and the Mac) leave that to the family door.
    pub fn shows_switcher(state: &SessionState) -> bool {
        let families = state.all_families().len();
        families >= 2
            || (families == 1
                && (!state.all_invitations().is_empty() || !state.all_pending().is_empty()))
    }

    /// "New Family…" (docs/protocol.md, `GET /me`): the operator lets members start families, the
    /// account owns none of its families — ownership that came by hand-off counts — and it is under
    /// the cap.
    pub fn offers_new_family(state: &SessionState) -> bool {
        let families = state.all_families();
        state.member_families_enabled
            && !families.is_empty()
            && !families.iter().any(FamilyDto::is_owner)
            && under_cap(families.len(), state.max_families_per_account)
    }

    /// "Join Another Family…": while under the cap.
    pub fn offers_join_another(state: &SessionState) -> bool {
        let families = state.all_families();
        !families.is_empty() && under_cap(families.len(), state.max_families_per_account)
    }

    /// Whether an owner is offered "Reset Password…" for a member (D7): only where the server said
    /// this owner owns every family the member is in (`password_reset: true`). A roster that does
    /// not say — a server from before several families, where an owner always could — keeps
    /// offering it; never for oneself, a former member or a deleted account.
    pub fn may_reset_password(member: &MemberDto, me: i64, owner: bool) -> bool {
        owner && member.id != me && !member.is_former() && member.password_reset != Some(false)
    }

    /// "Invite…" is the owner's, and only while they have ANOTHER family to invite from.
    pub fn offers_invite(state: &SessionState) -> bool {
        state.is_owner() && state.all_families().len() >= 2
    }

    fn under_cap(count: usize, cap: i32) -> bool {
        i64::try_from(count).unwrap_or(i64::MAX) < i64::from(cap)
    }
}

/// What one row of the family switcher is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SwitcherRowKind {
    /// A family the account is in: shows it.
    Family,
    /// A join request waiting for an owner: says so, and does nothing.
    Pending,
    /// An invitation to the account: Join or Decline.
    Invitation,
    /// "New Family…".
    NewFamily,
    /// "Join Another Family…".
    JoinAnother,
}

/// One row of the switcher. [`id`](Self::id) is the family's for a family and a pending request,
/// the invitation's for an invitation, and `None` for the two doors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitcherRow {
    pub kind: SwitcherRowKind,
    pub id: Option<i64>,
    pub title: String,
    pub detail: Option<String>,
    pub unread: i32,
    pub current: bool,
    pub accessible_name: Option<String>,
}

impl SwitcherRow {
    /// `new SwitcherRow(kind, id, title)`, the rest at the C#'s defaults.
    pub fn new(kind: SwitcherRowKind, id: Option<i64>, title: &str) -> Self {
        SwitcherRow {
            kind,
            id,
            title: title.to_string(),
            detail: None,
            unread: 0,
            current: false,
            accessible_name: None,
        }
    }
}

/// The family switcher (docs/protocol.md, "Switching"): every family with its unread count, the
/// pending join requests and invitations as rows of their own, and the two doors to a further
/// family when they are open (`FamilySwitcher`).
pub mod family_switcher {
    use super::*;

    /// A family's unread count: the unread of the chats LISTED in it. A direct chat counts in each
    /// family that lists it — what the reader would find unread there.
    pub fn unread(family_id: i64, chats: &[ChatRowDto], home: Option<i64>) -> i32 {
        chats
            .iter()
            .filter(|row| family_rules::lists(&row.chat, Some(family_id), home))
            .map(|row| row.unread_count)
            .sum()
    }

    /// The switcher's rows, top to bottom.
    pub fn rows(
        state: &SessionState,
        chats: &[ChatRowDto],
        say: &dyn StringCatalog,
    ) -> Vec<SwitcherRow> {
        let mut rows = Vec::new();
        for family in state.all_families() {
            let unread = unread(family.id, chats, state.home_family_id());
            rows.push(SwitcherRow {
                unread,
                current: Some(family.id) == state.current_family_id,
                accessible_name: Some(say.format("%@, %lld unread", &[&family.name, &unread])),
                ..SwitcherRow::new(SwitcherRowKind::Family, Some(family.id), &family.name)
            });
        }
        for pending in state.all_pending() {
            rows.push(SwitcherRow {
                detail: Some(say.get("Waiting for approval").to_string()),
                ..SwitcherRow::new(
                    SwitcherRowKind::Pending,
                    Some(pending.family_id),
                    &pending.family_name,
                )
            });
        }
        for invitation in state.all_invitations() {
            rows.push(SwitcherRow {
                detail: Some(invitation_text::banner(invitation, say)),
                ..SwitcherRow::new(
                    SwitcherRowKind::Invitation,
                    Some(invitation.id),
                    &invitation.family_name,
                )
            });
        }
        if family_rules::offers_new_family(state) {
            rows.push(SwitcherRow::new(
                SwitcherRowKind::NewFamily,
                None,
                say.get("New Family…"),
            ));
        }
        if family_rules::offers_join_another(state) {
            rows.push(SwitcherRow::new(
                SwitcherRowKind::JoinAnother,
                None,
                say.get("Join Another Family…"),
            ));
        }
        rows
    }
}

/// What the invitation banner and its rows say (`InvitationText`).
pub mod invitation_text {
    use super::*;

    /// "Anna invited you to “The Smiths”" — the inviter's name, the family's.
    pub fn banner(invitation: &InvitationDto, say: &dyn StringCatalog) -> String {
        let inviter = inviter(invitation, say);
        say.format(
            "%@ invited you to “%@”",
            &[&inviter, &invitation.family_name],
        )
    }

    fn inviter(invitation: &InvitationDto, say: &dyn StringCatalog) -> String {
        let by = &invitation.invited_by;
        if by.deleted {
            say.get("Deleted account").to_string()
        } else if !by.display_name.is_empty() {
            by.display_name.clone()
        } else {
            say.get("Someone").to_string()
        }
    }
}

/// The sentences several families add to the refusals a client has to say (docs/protocol.md, the
/// error list) — `FamilySentences`.
pub mod family_sentences {
    use super::*;

    /// The several-families refusal, or `None` when the error is not one of them — the caller's
    /// own sentence then. `cap` is `max_families_per_account` (`FamilySentences.For`).
    pub fn for_error(error: &ApiError, cap: i32, say: &dyn StringCatalog) -> Option<String> {
        Some(match error.code.as_str() {
            error_codes::TOO_MANY_FAMILIES => say.format(
                "You've reached the most families one account can be in (%lld).",
                &[&cap],
            ),
            error_codes::ALREADY_OWN_A_FAMILY => say
                .get("You already own a family, so you can't start another.")
                .to_string(),
            error_codes::MEMBER_FAMILIES_DISABLED => say
                .get("This server doesn't let members start new families.")
                .to_string(),
            error_codes::MEMBER_IN_OTHER_FAMILIES => say
                .get("This member is also in a family you don't own, so only they can change their password.")
                .to_string(),
            error_codes::INVITATION_NOT_PENDING => {
                say.get("This invitation is no longer available.").to_string()
            }
            _ => return None,
        })
    }
}

/// The doors to a FURTHER family from inside the app (docs/protocol.md, "Getting another family"):
/// "New Family…" and "Join Another Family…" — the existing create and join, which add the family
/// and SHOW it (`AnotherFamily`).
pub mod another_family {
    use super::*;

    /// What "New Family…" says above the existing create screen's field and button.
    pub fn explanation(say: &dyn StringCatalog) -> String {
        say.get(
            "Start your own family. You'll be its owner and can invite members of your other families.",
        )
        .to_string()
    }
}

/// One person an owner may invite into the family shown, and whether they already are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteCandidate {
    pub member: MemberDto,
    pub invited: bool,
}

/// How two names are put in order for the picker: the reader's collation without regard to case
/// (the C#'s `StringComparer.CurrentCultureIgnoreCase`).
pub type Collation = Rc<dyn Fn(&str, &str) -> Ordering>;

/// The owner's "Invite…" (docs/protocol.md, "Several families", invitations): the members of the
/// owner's OTHER families who are not in this one — from the rosters this device holds, and nobody
/// else on the server — and the pending invitations (`InviteModel`).
pub struct InviteModel {
    api: Rc<ApiClient>,
    chats: Rc<ChatStore>,
    collation: Collation,
}

impl std::fmt::Debug for InviteModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InviteModel").finish_non_exhaustive()
    }
}

impl InviteModel {
    /// Names in order by their lower case, code point by code point — until
    /// [`with_collation`](Self::with_collation) gives the reader's own (the shell's
    /// `g_utf8_collate` over case-folded names): Rust has no culture, and the two differ only for
    /// letters outside ASCII ("Émile" before "Zoe" in French; after it here).
    pub fn new(api: Rc<ApiClient>, chats: Rc<ChatStore>) -> Self {
        InviteModel {
            api,
            chats,
            collation: Rc::new(|a, b| a.to_lowercase().cmp(&b.to_lowercase())),
        }
    }

    pub fn with_collation(mut self, collation: Collation) -> Self {
        self.collation = collation;
        self
    }

    /// Who may be invited into `state`'s shown family: present members of the account's other
    /// families, not the reader, not in the shown family now, each once, in name order — marked
    /// when an invitation is pending.
    pub fn candidates(
        &self,
        state: &SessionState,
        pending: &[InvitationDto],
    ) -> StoreResult<Vec<InviteCandidate>> {
        let (Some(shown), Some(me)) = (state.current_family_id, state.me.as_ref()) else {
            return Ok(Vec::new());
        };
        let here: HashSet<i64> = self
            .chats
            .members_of(shown)?
            .iter()
            .filter(|member| !member.is_former())
            .map(|member| member.id)
            .collect();
        let invited: HashSet<i64> = pending
            .iter()
            .map(|invitation| invitation.user.id)
            .collect();
        let mut found: Vec<MemberDto> = Vec::new();
        for family in state
            .all_families()
            .iter()
            .filter(|family| family.id != shown)
        {
            for member in self.chats.members_of(family.id)? {
                if !member.is_former()
                    && member.id != me.id
                    && !here.contains(&member.id)
                    && !found.iter().any(|held| held.id == member.id)
                {
                    found.push(member);
                }
            }
        }
        found.sort_by(|a, b| {
            (self.collation)(&a.display_name, &b.display_name).then(a.id.cmp(&b.id))
        });
        Ok(found
            .into_iter()
            .map(|member| InviteCandidate {
                invited: invited.contains(&member.id),
                member,
            })
            .collect())
    }

    /// The shown family's pending invitations, oldest first.
    pub async fn pending(&self, family_id: i64) -> Result<Vec<InvitationDto>, ApiError> {
        self.api
            .invitations(family_id)
            .await
            .into_result()
            .map(|answer| answer.invitations.unwrap_or_default())
    }

    /// Invite one person; a `200` with the invitation already pending is success too.
    pub async fn invite(&self, family_id: i64, user_id: i64) -> Result<InvitationDto, ApiError> {
        self.api
            .invite(family_id, user_id)
            .await
            .into_result()
            .map(|answer| answer.invitation)
    }

    /// Withdraw one; idempotent.
    pub async fn withdraw(&self, family_id: i64, invitation_id: i64) -> Option<ApiError> {
        self.api
            .withdraw_invitation(family_id, invitation_id)
            .await
            .into_result()
            .err()
    }
}
