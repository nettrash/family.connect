//! Mentioning a member from the composer (docs/protocol.md, "Mentioning a member") — the web
//! client's composer rules: who the strip offers, what a send carries, and whom a name in a bubble
//! opens (`ComposerMentions.cs`).
//!
//! ONLY IN THE FAMILY CHAT OR A GROUP, where a mention means anything, and never while a message is
//! being edited. In a group the names are THAT GROUP's members alone (docs/protocol.md, "Mentioning
//! a member", amended 2026-10-22): the server refuses anybody else, who would not even receive the
//! message — so every rule here takes the chat's [`candidates`] rather than the roster.
//!
//! THE NAMES ARE RESOLVED FROM THE TEXT AT SEND: a name typed by hand mentions too, a name deleted
//! after being picked does not. The strip only helps write the whole name. The grammar is
//! `fc_text::mentions`' (and `fc_core::text::mentions`'), called rather than copied.

use std::collections::HashMap;

use fc_core::protocol::{AssistantDto, ChatDto, MemberDto, MentionDto};
use fc_text::mentions::Member as Named;

use crate::attach_menu;
use crate::groups::chat_kinds;

/// One stretch of a text: plain words, or a name — and whether that name opens a chat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameRun {
    pub text: String,
    pub user_id: Option<i64>,
    pub opens: bool,
}

/// One line of the strip over the composer: a member, or — `member` `None` — the assistant. `name`
/// is what it is drawn as, `handle` the quieter words after it, and `writes` what a pick puts after
/// the `@`: a member's display name, and the assistant's `ai`, never its display name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MentionChoice {
    pub member: Option<MemberDto>,
    pub name: String,
    pub handle: String,
    pub writes: String,
}

/// The members a name may name: the live roster — never a member who has left or been deleted.
pub fn live(members: &[MemberDto]) -> Vec<MemberDto> {
    members
        .iter()
        .filter(|member| !member.deleted && !member.is_former())
        .cloned()
        .collect()
}

/// Whether a mention means anything in a chat of this kind: the family chat or a group.
pub fn enabled(chat_kind: Option<&str>) -> bool {
    chat_kinds::is_shared(chat_kind)
}

/// Who a name in this chat may name — and who its polls count: the live roster in the family chat;
/// in a group, the live members of THAT GROUP, in the order they joined; nobody anywhere else. A
/// group whose members this device has not read yet (one an older build cached) offers nobody
/// rather than the whole family.
pub fn candidates(chat: Option<&ChatDto>, roster: &[MemberDto]) -> Vec<MemberDto> {
    let Some(chat) = chat else {
        return Vec::new();
    };
    match chat.kind.as_str() {
        chat_kinds::FAMILY => live(roster),
        chat_kinds::GROUP => {
            // The first of each id, as the C#'s `GroupBy(…).First()`.
            let mut by_id: HashMap<i64, MemberDto> = HashMap::new();
            for member in live(roster) {
                by_id.entry(member.id).or_insert(member);
            }
            let mut seen = Vec::new();
            let mut members = Vec::new();
            for id in chat.member_ids.as_deref().unwrap_or_default() {
                if seen.contains(id) {
                    continue;
                }
                seen.push(*id);
                if let Some(member) = by_id.get(id) {
                    members.push(member.clone());
                }
            }
            members
        }
        _ => Vec::new(),
    }
}

/// The names a text carries, resolved against the live members given, at most
/// [`MAX_PER_MESSAGE`](fc_core::text::mentions::MAX_PER_MESSAGE).
pub fn resolve(text: &str, members: &[MemberDto]) -> Vec<MentionDto> {
    let live = live(members);
    let named: Vec<Named<'_>> = live
        .iter()
        .map(|member| Named {
            user_id: member.id,
            name: &member.display_name,
        })
        .collect();
    fc_text::mentions::resolve(text, &named)
        .into_iter()
        .take(fc_core::text::mentions::MAX_PER_MESSAGE)
        .map(|member| MentionDto::new(member.user_id, member.name))
        .collect()
}

/// What a send carries: the names of `candidates` it says, where mentions are `enabled` — the
/// family chat, or a group with its own members as the candidates — and nothing anywhere else or
/// when it names nobody. Resolving against the group's members is what keeps a send from naming
/// somebody the server would refuse.
pub fn for_send(body: &str, candidates: &[MemberDto], enabled: bool) -> Option<Vec<MentionDto>> {
    if !enabled {
        return None;
    }
    let named = resolve(body, candidates);
    (!named.is_empty()).then_some(named)
}

/// Whether a name in a bubble is a DOOR: a live member who is not the reader and not somebody they
/// blocked. A name outside that is drawn like any other and simply does not open.
pub fn opens_chat(
    user_id: i64,
    members: &[MemberDto],
    me: i64,
    blocked: &dyn Fn(i64) -> bool,
) -> bool {
    user_id != me
        && !blocked(user_id)
        && members
            .iter()
            .any(|member| member.id == user_id && !member.deleted && !member.is_former())
}

/// The names a half-typed `@` could mean, in the order of `candidates` — never the reader, never
/// somebody they blocked. Empty where mentions are not `enabled`, while editing, and when the draft
/// is not mid-mention. `candidates` is the chat's [`candidates`]: the roster in the family chat, the
/// group's members in a group. The `@`-strip grammar is `fc_text::mentions::{query, candidates}`.
pub fn offered(
    draft: &str,
    candidates: &[MemberDto],
    me: i64,
    blocked: &dyn Fn(i64) -> bool,
    enabled: bool,
    editing: bool,
) -> Vec<MemberDto> {
    if !enabled || editing {
        return Vec::new();
    }
    let Some(query) = fc_text::mentions::query(draft) else {
        return Vec::new();
    };
    let roster = live(candidates);
    let excluding: Vec<i64> = roster
        .iter()
        .filter(|member| member.id == me || blocked(member.id))
        .map(|member| member.id)
        .collect();
    let named: Vec<Named<'_>> = roster
        .iter()
        .map(|member| Named {
            user_id: member.id,
            name: &member.display_name,
        })
        .collect();
    let offered: Vec<i64> = fc_text::mentions::candidates(&named, query, &excluding)
        .into_iter()
        .map(|member| member.user_id)
        .collect();
    roster
        .iter()
        .filter(|member| offered.contains(&member.id))
        .cloned()
        .collect()
}

/// The strip's lines for a half-typed `@`: THE ASSISTANT FIRST, above the members (docs/protocol.md,
/// "Mentioning a member", amended 2026-10-20, #106) — exactly where `@ai` may be written at all: the
/// family chat or a group (amended 2026-10-22), not while editing, on a server whose assistant names
/// who answers — narrowed by its display name and by `ai` as a member is by theirs; then
/// [`offered`]. The assistant is never a member here: if the roster carries its account, that row is
/// left out rather than offered as `@<display_name>`.
pub fn choices(
    draft: &str,
    candidates: &[MemberDto],
    me: i64,
    blocked: &dyn Fn(i64) -> bool,
    enabled: bool,
    editing: bool,
    assistant: Option<&AssistantDto>,
) -> Vec<MentionChoice> {
    let people = offered(draft, candidates, me, blocked, enabled, editing)
        .into_iter()
        .filter(|member| assistant.is_none_or(|assistant| member.id != assistant.user_id))
        .map(|member| MentionChoice {
            name: member.display_name.clone(),
            handle: if member.username.is_empty() {
                String::new()
            } else {
                format!("@{}", member.username)
            },
            writes: member.display_name.clone(),
            member: Some(member),
        });
    let asked = match (assistant, fc_text::mentions::query(draft)) {
        (Some(assistant), Some(query))
            if enabled && !editing && attach_menu::assistant_available(Some(assistant)) =>
        {
            // `AssistantText.Token[1..]`: what a pick writes after the `@`.
            let token = &fc_text::assistant::TOKEN[1..];
            let name = if assistant.display_name.is_empty() {
                token
            } else {
                assistant.display_name.as_str()
            };
            let narrowed = fc_text::mentions::candidates(
                &[
                    Named {
                        user_id: assistant.user_id,
                        name,
                    },
                    Named {
                        user_id: assistant.user_id,
                        name: token,
                    },
                ],
                query,
                &[],
            );
            (!narrowed.is_empty()).then(|| MentionChoice {
                member: None,
                name: name.to_string(),
                handle: fc_text::assistant::TOKEN.to_string(),
                writes: token.to_string(),
            })
        }
        _ => None,
    };
    asked.into_iter().chain(people).collect()
}

/// A text cut into its plain stretches and the names it says, each name marked with whether it is a
/// door ([`opens_chat`]) — the same answer for a bubble and for a note somebody has opened. A text
/// that names nobody is one plain stretch (`fc_core::text::mentions::runs`).
pub fn runs(
    text: &str,
    named: Option<&[MentionDto]>,
    members: &[MemberDto],
    me: i64,
    blocked: &dyn Fn(i64) -> bool,
) -> Vec<NameRun> {
    let Some(named) = named.filter(|named| !named.is_empty()) else {
        return vec![NameRun {
            text: text.to_string(),
            user_id: None,
            opens: false,
        }];
    };
    let mentions: Vec<Named<'_>> = named
        .iter()
        .map(|mention| Named {
            user_id: mention.user_id,
            name: &mention.name,
        })
        .collect();
    fc_core::text::mentions::runs(text, &mentions)
        .into_iter()
        .map(|(part, user_id)| NameRun {
            opens: user_id.is_some_and(|id| opens_chat(id, members, me, blocked)),
            text: part,
            user_id,
        })
        .collect()
}
