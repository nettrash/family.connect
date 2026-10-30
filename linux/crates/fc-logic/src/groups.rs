//! The protocol's four kinds of chat and the questions the window asks of a kind; who may do what
//! to a group, what a name may be, and whom the lists offer; making and managing one (`Groups.cs`;
//! docs/protocol.md, "Objects" and "Groups").
//!
//! A GROUP IS THE FAMILY CHAT WITH FEWER PEOPLE (docs/protocol.md, "Groups"): sender names,
//! threads, polls, member mentions, the assistant by `@ai`, video messages — and no "seen", because
//! it has many readers. What it does NOT share with the family chat is the transcript rule, which
//! follows the DIRECT chat (`Transcripts.MayAsk`), and calls, which live in a direct chat alone.

use std::rc::Rc;

use fc_core::protocol::{ApiClient, ApiError, ApiResult, ChatResponse, error_codes};
use fc_core::store::{ChatStore, StoreResult};

/// The protocol's four kinds of chat (`ChatKinds`). Every "is this the family chat?" that a group
/// now shares lives here, once, so a fifth place cannot forget it. The names are `fc_text::groups`'.
pub mod chat_kinds {
    pub const FAMILY: &str = fc_text::groups::FAMILY;
    pub const DIRECT: &str = fc_text::groups::DIRECT;
    pub const ASSISTANT: &str = fc_text::groups::AI;
    pub const GROUP: &str = fc_text::groups::GROUP;

    pub fn is_group(kind: Option<&str>) -> bool {
        kind == Some(GROUP)
    }

    /// A room with more than one other person in it — the family chat or a group: names over
    /// bubbles, member mentions, polls, `@ai`, and no "seen".
    pub fn is_shared(kind: Option<&str>) -> bool {
        matches!(kind, Some(FAMILY | GROUP))
    }

    /// Where the video button is offered (S1.4): a family, direct or group chat — never the
    /// assistant's.
    pub fn takes_video_messages(kind: Option<&str>) -> bool {
        matches!(kind, Some(FAMILY | DIRECT | GROUP))
    }
}

/// What a chat's circle shows: the family's house, a group's people, or a name's initials (and a
/// picture over them).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChatFace {
    /// The family chat: a house.
    House,
    /// A group: the "people" glyph (docs/groups-2026-10-21.md, §6 — "the initials of the name, or
    /// a people glyph").
    People,
    /// A direct chat or the assistant's: the initials of the title, with the peer's picture over
    /// them when there is one.
    Initials,
}

/// Something a member may do to a group (docs/protocol.md, "Groups", "Who may do what").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GroupAction {
    Rename,
    AddMembers,
    RemoveMember,
    Delete,
    Leave,
}

/// One member of a group as its info draws them: in the order they joined, the admin marked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupMember {
    pub user_id: i64,
    pub name: String,
    pub admin: bool,
    pub me: bool,
    /// The reader is the admin and this is somebody else: "Remove from Group" is offered for them.
    pub removable: bool,
}

/// Who may do what to a group, what a name may be, and whom the lists offer (docs/protocol.md,
/// "Groups") — the one table every client keeps (`GroupRules`).
///
/// THE ADMIN IS ONE MEMBER, the one who created it until they leave it: they rename, add, remove and
/// delete. Anybody in it may leave. Nobody else may do anything, the family owner included — they
/// cannot even see a group they are not in. The server answers `not_group_admin` to the rest, and a
/// client that offered the button anyway would be offering a refusal.
///
/// THE NAME IS TRIMMED AND THEN 1 TO 64 CHARACTERS, counted as the server counts — scalar values,
/// the way a pack label is — so thirty-three emoji that the server takes are not refused here.
///
/// `fc_text::groups::may` differs in one way the C# does not share: it also asks that the reader
/// be in `member_ids`. Holding a group is being in it (the server lists it to its members alone),
/// so the C#'s rule is ported as written; the two agree on every group a device holds.
pub mod group_rules {
    use std::collections::HashMap;

    use fc_core::protocol::{ChatDto, MemberDto};
    use fc_core::text::string_catalog::StringCatalog;

    use super::{GroupAction, GroupMember, chat_kinds, group_text};
    use crate::composer_mentions;

    /// The most scalar values a group's name may be, after trimming.
    pub const MAX_NAME_LENGTH: usize = fc_text::groups::MAX_TITLE_CHARS;

    /// The most groups one family may have; the 51st is `too_many_groups`.
    pub const MAX_GROUPS: usize = fc_text::groups::MAX_GROUPS;

    /// Whether the reader may do this to this chat. Nothing at all on a chat that is not a group.
    pub fn may(action: GroupAction, chat: &ChatDto, me: i64) -> bool {
        if !chat_kinds::is_group(Some(&chat.kind)) || me <= 0 {
            return false;
        }
        // Holding a group is being in it: the server lists it to its members alone.
        action == GroupAction::Leave || chat.admin_user_id == Some(me)
    }

    /// Whether the reader may take this member out: the admin, somebody else, and somebody who is
    /// in it.
    pub fn may_remove(chat: &ChatDto, me: i64, member: i64) -> bool {
        may(GroupAction::RemoveMember, chat, me)
            && member != me
            && chat
                .member_ids
                .as_deref()
                .is_some_and(|ids| ids.contains(&member))
    }

    /// The name as it is sent: trimmed, or `None` when it is not 1 to 64 characters once trimmed
    /// (`fc_text::groups::title`, the same rule).
    pub fn name(typed: Option<&str>) -> Option<&str> {
        fc_text::groups::title(typed?)
    }

    /// The members a NEW group can be made with: the live roster, never the reader (who is added
    /// anyway) and never the assistant — "never anybody outside the family, never somebody who has
    /// left, never a deleted account".
    pub fn choosable(roster: &[MemberDto], me: i64, assistant_id: Option<i64>) -> Vec<MemberDto> {
        composer_mentions::live(roster)
            .into_iter()
            .filter(|member| member.id != me && Some(member.id) != assistant_id)
            .collect()
    }

    /// The members "Add Members" offers: everyone [`choosable`] who is not in the group already.
    /// Blocks change nothing here, in either direction: refusing an add would tell the adder about
    /// a block they are not party to.
    pub fn addable(
        chat: &ChatDto,
        roster: &[MemberDto],
        me: i64,
        assistant_id: Option<i64>,
    ) -> Vec<MemberDto> {
        let in_it = chat.member_ids.as_deref().unwrap_or_default();
        choosable(roster, me, assistant_id)
            .into_iter()
            .filter(|member| !in_it.contains(&member.id))
            .collect()
    }

    /// The member list of the group's info, in the order they joined (the server's order). A
    /// member the roster cannot name — one who joined the family while this device slept — is
    /// "Someone" until the roster is read.
    pub fn members(
        chat: &ChatDto,
        roster: &[MemberDto],
        me: i64,
        say: &dyn StringCatalog,
    ) -> Vec<GroupMember> {
        // The first of each id, as the C#'s `GroupBy(…).First()`.
        let mut by_id: HashMap<i64, &MemberDto> = HashMap::new();
        for member in roster {
            by_id.entry(member.id).or_insert(member);
        }
        let mut seen: Vec<i64> = Vec::new();
        let mut members = Vec::new();
        for &id in chat.member_ids.as_deref().unwrap_or_default() {
            if seen.contains(&id) {
                continue;
            }
            seen.push(id);
            let name = match by_id.get(&id) {
                Some(member) if member.deleted => say.get("Deleted account").to_string(),
                Some(member) if !member.display_name.is_empty() => member.display_name.clone(),
                _ => say.get("Someone").to_string(),
            };
            members.push(GroupMember {
                user_id: id,
                name,
                admin: chat.admin_user_id == Some(id),
                me: id == me,
                removable: may_remove(chat, me, id),
            });
        }
        members
    }

    /// What stops a group being created, said before anything is sent: the name first, then nobody
    /// chosen. `None` when it may be asked for.
    pub fn create_problem(
        typed_name: Option<&str>,
        chosen: &[i64],
        me: i64,
        say: &dyn StringCatalog,
    ) -> Option<String> {
        if name(typed_name).is_none() {
            return Some(group_text::name_problem(say));
        }
        if chosen.iter().any(|id| *id != me) {
            None
        } else {
            Some(group_text::nobody_chosen(say))
        }
    }
}

/// Every sentence about a group, from the apps' catalogue (`GroupText`).
pub mod group_text {
    use fc_core::protocol::{ApiError, ChatDto, error_codes};
    use fc_core::text::string_catalog::StringCatalog;

    pub fn name_problem(say: &dyn StringCatalog) -> String {
        say.get("A group's name can be 1 to 64 characters.")
            .to_string()
    }

    pub fn nobody_chosen(say: &dyn StringCatalog) -> String {
        say.get("Choose at least one member.").to_string()
    }

    pub fn everyone_is_in(say: &dyn StringCatalog) -> String {
        say.get("Everyone in the family is already in this group.")
            .to_string()
    }

    /// "Members: 3" — the count over the group's info.
    pub fn count(chat: &ChatDto, say: &dyn StringCatalog) -> String {
        let mut ids: Vec<i64> = chat.member_ids.clone().unwrap_or_default();
        ids.sort_unstable();
        ids.dedup();
        say.format("Members: %lld", &[&ids.len()])
    }

    /// Asked before the admin deletes it: final, and for everyone (docs/protocol.md, "Groups").
    pub fn delete_question(name: &str, say: &dyn StringCatalog) -> String {
        say.format(
            "Delete “%@”? Its messages are deleted for everyone.",
            &[&name],
        )
    }

    /// Asked before anybody leaves: a group is joined by being added, so coming back is the
    /// admin's to do.
    pub fn leave_question(name: &str, say: &dyn StringCatalog) -> String {
        say.format(
            "Leave “%@”? You can come back only if its admin adds you again.",
            &[&name],
        )
    }

    /// A refusal in the reader's words. The two codes groups added are SAID — the limit is the
    /// family's, and the admin is somebody the reader can ask — and anything else is the plain
    /// failure of what was being done. `creating`: the refusal answers "Create Group" rather than
    /// a change to a group that exists.
    pub fn failure(error: &ApiError, creating: bool, say: &dyn StringCatalog) -> String {
        match error.code.as_str() {
            error_codes::NOT_GROUP_ADMIN => say.get("Only the group's admin can do this."),
            error_codes::TOO_MANY_GROUPS => say.get("Your family already has 50 groups."),
            _ if creating => say.get("Couldn't create the group."),
            _ => say.get("Couldn't change the group."),
        }
        .to_string()
    }
}

/// Making and managing a group: each call asks the server and writes what it answered into the
/// cache at once — the `chat_updated` or `chat_removed` frame that confirms it reaches this device
/// too, and applying the same group twice is applying it once (docs/protocol.md, "Groups").
///
/// Every call answers `StoreResult<…>`: `Err` is the cache refusing to record the answer (the C#'s
/// exception), and inside it the C#'s own answer.
pub struct GroupModel {
    api: Rc<ApiClient>,
    chats: Rc<ChatStore>,
}

impl std::fmt::Debug for GroupModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GroupModel").finish_non_exhaustive()
    }
}

impl GroupModel {
    pub fn new(api: Rc<ApiClient>, chats: Rc<ChatStore>) -> Self {
        GroupModel { api, chats }
    }

    /// A refusal made here, before anything is sent, with the server's own code.
    fn bad_name() -> ApiError {
        ApiError::new(
            error_codes::VALIDATION,
            "a group's name is 1 to 64 characters",
            400,
        )
    }

    /// A new group with the reader as its admin; answers its id, or the refusal.
    pub async fn create(
        &self,
        typed_name: &str,
        member_ids: &[i64],
    ) -> StoreResult<Result<i64, ApiError>> {
        let Some(name) = group_rules::name(Some(typed_name)) else {
            return Ok(Err(Self::bad_name()));
        };
        let mut distinct: Vec<i64> = Vec::with_capacity(member_ids.len());
        for id in member_ids {
            if !distinct.contains(id) {
                distinct.push(*id);
            }
        }
        match self.api.create_group(name, &distinct).await.into_result() {
            Ok(created) => {
                self.chats.upsert(&created.chat)?;
                Ok(Ok(created.chat.id))
            }
            Err(error) => Ok(Err(error)),
        }
    }

    pub async fn rename(&self, chat_id: i64, typed_name: &str) -> StoreResult<Option<ApiError>> {
        let Some(name) = group_rules::name(Some(typed_name)) else {
            return Ok(Some(Self::bad_name()));
        };
        self.kept(self.api.rename_group(chat_id, name).await)
    }

    pub async fn add(&self, chat_id: i64, user_ids: &[i64]) -> StoreResult<Option<ApiError>> {
        self.kept(self.api.add_group_members(chat_id, user_ids).await)
    }

    /// The admin taking somebody else out. The reader's own way out is [`leave`](Self::leave).
    pub async fn remove(&self, chat_id: i64, user_id: i64) -> StoreResult<Option<ApiError>> {
        match self
            .api
            .remove_group_member(chat_id, user_id)
            .await
            .into_result()
        {
            Err(error) => Ok(Some(error)),
            Ok(answer) => {
                if let Some(chat) = &answer.chat {
                    self.chats.upsert(chat)?;
                }
                Ok(None)
            }
        }
    }

    /// The reader leaves: the server answers `204`, hands the group on if they were its admin, and
    /// deletes it if they were the last one in. Here it goes, its messages with it.
    pub async fn leave(&self, chat_id: i64) -> StoreResult<Option<ApiError>> {
        let answer = self
            .api
            .remove_group_member(chat_id, self.chats.reader())
            .await;
        self.gone(chat_id, answer.error())
    }

    /// The admin deletes it, for everyone; here it goes at once.
    pub async fn delete(&self, chat_id: i64) -> StoreResult<Option<ApiError>> {
        let answer = self.api.delete_group(chat_id).await;
        self.gone(chat_id, answer.error())
    }

    fn kept(&self, answer: ApiResult<ChatResponse>) -> StoreResult<Option<ApiError>> {
        match answer.into_result() {
            Err(error) => Ok(Some(error)),
            Ok(kept) => {
                self.chats.upsert(&kept.chat)?;
                Ok(None)
            }
        }
    }

    /// A group the reader is out of. `chat_not_found` is the same outcome reached first by
    /// somebody else — the admin deleted it, or it was left by its last other member — so it is
    /// dropped here all the same and nothing is said.
    fn gone(&self, chat_id: i64, error: Option<&ApiError>) -> StoreResult<Option<ApiError>> {
        if let Some(error) = error
            && error.code != error_codes::CHAT_NOT_FOUND
        {
            return Ok(Some(error.clone()));
        }
        self.chats.remove(chat_id)?;
        Ok(None)
    }
}
