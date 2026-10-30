//! `GroupsTests`: groups (docs/protocol.md, "Groups"; docs/groups-2026-10-21.md) — who may do what,
//! what a name may be, whom the lists offer, what a refusal says, and the model that asks the
//! server and keeps what it answered.
//!
//! `AGroupSharesTheFamilyChatsRoom` holds its two `AttachMenu.Groups` assertions too (a poll and
//! "Ask the Assistant" in a group's paperclip menu).

mod common;

use std::rc::Rc;

use fc_core::protocol::{
    ApiError, ChatDto, ChatResponse, ChatRowDto, MemberDto, MessageDto, error_codes,
};
use fc_core::store::{ChatStore, Database, StoreResult};
use fc_core::testing::{FakeClock, drive};
use fc_core::text::string_catalog::EnglishCatalog;
use fc_logic::attach_menu::{self, AttachItem};
use fc_logic::bubble_rules;
use fc_logic::groups::{GroupAction, GroupModel, chat_kinds, group_rules, group_text};
use time::macros::datetime;

use common::server::{Server, api};

const ME: i64 = 7;
const ASSISTANT: i64 = 1;

fn say() -> EnglishCatalog {
    common::english()
}

fn member(id: i64, username: &str, name: &str) -> MemberDto {
    MemberDto::new(id, username, name)
}

fn roster() -> Vec<MemberDto> {
    vec![
        MemberDto {
            role: Some("owner".to_string()),
            ..member(ME, "anna", "Anna")
        },
        member(9, "bob", "Bob"),
        member(11, "cleo", "Cleo"),
        member(12, "dan", "Dan"),
        MemberDto {
            has_left: true,
            ..member(13, "eve", "Eve")
        },
        MemberDto {
            deleted: true,
            ..member(14, "gone", "Deleted account")
        },
        member(ASSISTANT, "assistant", "Assistant"),
    ]
}

/// Bob made it; Anna (the reader) and Cleo are in it too.
fn cousins() -> ChatDto {
    ChatDto {
        member_ids: Some(vec![9, ME, 11]),
        admin_user_id: Some(9),
        ..ChatDto::new(77, "group", "Cousins")
    }
}

fn mine() -> ChatDto {
    ChatDto {
        admin_user_id: Some(ME),
        ..cousins()
    }
}

struct Rig {
    chats: Rc<ChatStore>,
    clock: FakeClock,
}

impl Rig {
    fn new() -> StoreResult<Rig> {
        let cache = Database::open_in_memory()?;
        let chats = Rc::new(ChatStore::new(&cache).with_me(|| ME));
        chats.replace_members(&roster(), &[])?;
        chats.replace(
            &[
                ChatRowDto::new(ChatDto::new(42, "family", "The Smiths")),
                ChatRowDto::new(cousins()),
            ],
            None,
        )?;
        Ok(Rig {
            chats,
            clock: FakeClock::at(datetime!(2026-09-12 12:00 UTC)),
        })
    }

    fn build(&self, server: &Server) -> GroupModel {
        GroupModel::new(api(server, &self.clock), self.chats.clone())
    }

    fn chat(&self, id: i64) -> StoreResult<Option<ChatDto>> {
        Ok(self.chats.chat(id)?.map(|row| row.chat))
    }
}

fn answer(chat: &ChatDto) -> String {
    serde_json::to_string(&ChatResponse { chat: chat.clone() }).expect("a chat encodes")
}

fn ids(members: &[MemberDto]) -> Vec<i64> {
    members.iter().map(|member| member.id).collect()
}

// ---- who may do what ------------------------------------------------------------------------

/// THE TABLE (docs/protocol.md, "Who may do what"): the admin renames, adds, removes and deletes;
/// anybody in it leaves; nobody may do anything to a chat that is not a group, nor while the
/// reader is not known.
#[test]
fn the_admin_manages_it_and_any_member_may_leave() {
    for (action, admin, member) in [
        (GroupAction::Rename, true, false),
        (GroupAction::AddMembers, true, false),
        (GroupAction::RemoveMember, true, false),
        (GroupAction::Delete, true, false),
        (GroupAction::Leave, true, true),
    ] {
        assert_eq!(admin, group_rules::may(action, &mine(), ME), "{action:?}");
        assert_eq!(
            member,
            group_rules::may(action, &cousins(), ME),
            "{action:?}"
        );
        assert!(!group_rules::may(
            action,
            &ChatDto::new(42, "family", "The Smiths"),
            ME
        ));
        assert!(!group_rules::may(
            action,
            &ChatDto {
                peer_user_id: Some(9),
                ..ChatDto::new(43, "direct", "Bob")
            },
            ME
        ));
        assert!(!group_rules::may(
            action,
            &ChatDto::new(44, "ai", "Assistant"),
            ME
        ));
        assert!(!group_rules::may(action, &mine(), 0));
    }
}

/// The admin may take out somebody else who is in it — never themselves (that is leaving), never a
/// stranger.
#[test]
fn the_admin_removes_somebody_else_who_is_in_it() {
    assert!(group_rules::may_remove(&mine(), ME, 9));
    assert!(group_rules::may_remove(&mine(), ME, 11));
    assert!(!group_rules::may_remove(&mine(), ME, ME));
    assert!(!group_rules::may_remove(&mine(), ME, 12));
    assert!(!group_rules::may_remove(&cousins(), ME, 11));
}

// ---- the name ----------------------------------------------------------------------------------

/// TRIMMED, THEN 1 TO 64 CHARACTERS, counted as the server counts — scalar values, so 64 emoji are a
/// name and 65 are not. Two groups may share a name; nothing here asks.
#[test]
fn a_name_is_trimmed_and_then_one_to_sixty_four_characters() {
    assert_eq!(Some("Cousins"), group_rules::name(Some("  Cousins \n")));
    assert_eq!(None, group_rules::name(Some("")));
    assert_eq!(None, group_rules::name(Some("   ")));
    assert_eq!(None, group_rules::name(None));
    let sixty_four = "a".repeat(64);
    assert_eq!(
        Some(sixty_four.as_str()),
        group_rules::name(Some(&sixty_four))
    );
    assert_eq!(None, group_rules::name(Some(&"a".repeat(65))));
    let emoji = "😀".repeat(64);
    assert_eq!(Some(emoji.as_str()), group_rules::name(Some(&emoji)));
    assert_eq!(None, group_rules::name(Some(&format!("{emoji}😀"))));
}

/// What stops "Create Group", said before anything is sent: the name first, then nobody chosen.
#[test]
fn creating_needs_a_name_and_at_least_one_other_member() {
    let say = say();
    assert_eq!(
        None,
        group_rules::create_problem(Some("Cousins"), &[9], ME, &say)
    );
    assert_eq!(
        Some("A group's name can be 1 to 64 characters.".to_string()),
        group_rules::create_problem(Some(" "), &[9], ME, &say)
    );
    assert_eq!(
        Some("A group's name can be 1 to 64 characters.".to_string()),
        group_rules::create_problem(Some(" "), &[], ME, &say)
    );
    assert_eq!(
        Some("Choose at least one member.".to_string()),
        group_rules::create_problem(Some("Cousins"), &[], ME, &say)
    );
    assert_eq!(
        Some("Choose at least one member.".to_string()),
        group_rules::create_problem(Some("Cousins"), &[ME], ME, &say)
    );
}

// ---- whom the lists offer ----------------------------------------------------------------------

/// A new group is made from the live family: never the reader (added anyway), never the assistant,
/// never somebody who left, never a deleted account. "Add Members" offers the same people, less
/// those already in.
#[test]
fn the_lists_offer_the_live_family_and_nobody_else() {
    let roster = roster();
    assert_eq!(
        vec![9, 11, 12],
        ids(&group_rules::choosable(&roster, ME, Some(ASSISTANT)))
    );
    assert_eq!(
        vec![12],
        ids(&group_rules::addable(&mine(), &roster, ME, Some(ASSISTANT)))
    );
    let everyone = ChatDto {
        member_ids: Some(vec![9, ME, 11, 12]),
        ..mine()
    };
    assert!(group_rules::addable(&everyone, &roster, ME, Some(ASSISTANT)).is_empty());
    assert_eq!(
        "Everyone in the family is already in this group.",
        group_text::everyone_is_in(&say())
    );
}

/// The info's member list: join order, the admin marked, the reader marked, and who the admin may
/// remove.
#[test]
fn the_members_are_listed_in_the_order_they_joined() {
    let say = say();
    let roster = roster();
    let chat = ChatDto {
        member_ids: Some(vec![9, ME, 11, 99]),
        ..mine()
    };
    let members = group_rules::members(&chat, &roster, ME, &say);

    assert_eq!(
        vec![9, ME, 11, 99],
        members.iter().map(|m| m.user_id).collect::<Vec<_>>()
    );
    assert_eq!(
        vec!["Bob", "Anna", "Cleo", "Someone"],
        members.iter().map(|m| m.name.as_str()).collect::<Vec<_>>()
    );
    assert_eq!(
        vec![false, true, false, false],
        members.iter().map(|m| m.admin).collect::<Vec<_>>()
    );
    assert_eq!(
        vec![false, true, false, false],
        members.iter().map(|m| m.me).collect::<Vec<_>>()
    );
    assert_eq!(
        vec![true, false, true, true],
        members.iter().map(|m| m.removable).collect::<Vec<_>>()
    );
    // Somebody else's group: Bob is its admin, and the reader may remove nobody.
    let theirs = group_rules::members(&cousins(), &roster, ME, &say);
    assert!(theirs[0].admin);
    assert!(theirs.iter().all(|member| !member.removable));
    assert_eq!("Members: 3", group_text::count(&cousins(), &say));
}

// ---- what is said ------------------------------------------------------------------------------

#[test]
fn a_refusal_is_said_in_the_readers_words() {
    let say = say();
    assert_eq!(
        "Only the group's admin can do this.",
        group_text::failure(
            &ApiError::new(error_codes::NOT_GROUP_ADMIN, "x", 403),
            false,
            &say
        )
    );
    assert_eq!(
        "Your family already has 50 groups.",
        group_text::failure(
            &ApiError::new(error_codes::TOO_MANY_GROUPS, "x", 409),
            true,
            &say
        )
    );
    assert_eq!(
        "Couldn't create the group.",
        group_text::failure(
            &ApiError::new(error_codes::NOT_SAME_FAMILY, "x", 409),
            true,
            &say
        )
    );
    assert_eq!(
        "Couldn't change the group.",
        group_text::failure(&ApiError::transport("offline"), false, &say)
    );
    assert_eq!(
        "Delete “Cousins”? Its messages are deleted for everyone.",
        group_text::delete_question("Cousins", &say)
    );
    assert_eq!(
        "Leave “Cousins”? You can come back only if its admin adds you again.",
        group_text::leave_question("Cousins", &say)
    );
}

// ---- what a group offers -----------------------------------------------------------------------

/// A GROUP IS THE FAMILY CHAT WITH FEWER PEOPLE: sender names, mentions, polls, `@ai`, no "seen" —
/// and the video button, which a direct chat has too and the assistant's chat does not. (The C#'s
/// two `AttachMenu.Groups` lines are not here — see the file's header.)
#[test]
fn a_group_shares_the_family_chats_room() {
    assert!(chat_kinds::is_shared(Some("family")));
    assert!(chat_kinds::is_shared(Some("group")));
    assert!(!chat_kinds::is_shared(Some("direct")));
    assert!(!chat_kinds::is_shared(Some("ai")));
    assert!(!chat_kinds::is_shared(None));
    assert!(chat_kinds::takes_video_messages(Some("group")));
    assert!(chat_kinds::takes_video_messages(Some("family")));
    assert!(chat_kinds::takes_video_messages(Some("direct")));
    assert!(!chat_kinds::takes_video_messages(Some("ai")));
    let offered = attach_menu::groups(
        false,
        false,
        chat_kinds::is_shared(Some("group")),
        false,
        true,
    )
    .concat();
    assert!(offered.contains(&AttachItem::Poll));
    assert!(offered.contains(&AttachItem::AskAssistant));
    let mine = MessageDto::new(1500, 77, ME, None, "hi", "2026-09-12T10:00:00Z");
    assert!(!bubble_rules::shows_tick(
        &mine,
        ME,
        chat_kinds::is_shared(Some("group"))
    ));
}

// ---- the model ---------------------------------------------------------------------------------

/// A group made here is in the cache at once, with the reader as its admin, before any frame
/// lands.
#[test]
fn creating_keeps_the_group_the_server_answered() -> StoreResult<()> {
    let rig = Rig::new()?;
    let made = ChatDto {
        member_ids: Some(vec![ME, 12]),
        admin_user_id: Some(ME),
        ..ChatDto::new(78, "group", "Book club")
    };
    let server = Server::new();
    server.on_status("/chats/groups", 201, Some(&answer(&made)));
    let groups = rig.build(&server);

    let created = drive(&rig.clock, groups.create("  Book club ", &[12, 12]))?;

    assert_eq!(Ok(78), created);
    assert_eq!("Book club", rig.chat(78)?.expect("kept").title);
    assert_eq!(Some(ME), rig.chat(78)?.expect("kept").admin_user_id);
    // Trimmed, and every id once.
    assert_eq!(
        vec!["{\"title\":\"Book club\",\"member_ids\":[12]}".to_string()],
        server.bodies()
    );
    Ok(())
}

/// A name that cannot be one is refused here, and nothing is sent.
#[test]
fn a_bad_name_is_never_sent() -> StoreResult<()> {
    let rig = Rig::new()?;
    let server = Server::new();
    let groups = rig.build(&server);

    let created = drive(&rig.clock, groups.create("   ", &[12]))?;

    assert_eq!(error_codes::VALIDATION, created.expect_err("refused").code);
    assert_eq!(
        Some(error_codes::VALIDATION.to_string()),
        drive(&rig.clock, groups.rename(77, &"a".repeat(65)))?.map(|error| error.code)
    );
    assert!(server.asked().is_empty());
    Ok(())
}

/// A rename and an add are written as the server answered them; a refusal changes nothing and is
/// handed back.
#[test]
fn changes_are_kept_and_refusals_change_nothing() -> StoreResult<()> {
    let rig = Rig::new()?;
    let renamed = ChatDto {
        title: "The cousins".to_string(),
        ..mine()
    };
    let server = Server::new();
    server
        .then(
            "/chats/77",
            &[
                (200, Some(&answer(&renamed))),
                (
                    403,
                    Some(r#"{"error": {"code": "not_group_admin", "message": "no"}}"#),
                ),
            ],
        )
        .on(
            "/chats/77/members",
            &answer(&ChatDto {
                member_ids: Some(vec![9, ME, 11, 12]),
                ..renamed.clone()
            }),
        )
        .on(
            "/chats/77/members/11",
            &answer(&ChatDto {
                member_ids: Some(vec![9, ME]),
                ..renamed.clone()
            }),
        );
    let groups = rig.build(&server);

    assert_eq!(None, drive(&rig.clock, groups.rename(77, "The cousins"))?);
    assert_eq!("The cousins", rig.chat(77)?.expect("held").title);
    assert_eq!(None, drive(&rig.clock, groups.add(77, &[12]))?);
    assert_eq!(
        Some(vec![9, ME, 11, 12]),
        rig.chat(77)?.expect("held").member_ids
    );
    assert_eq!(None, drive(&rig.clock, groups.remove(77, 11))?);
    assert_eq!(Some(vec![9, ME]), rig.chat(77)?.expect("held").member_ids);

    assert_eq!(
        Some(error_codes::NOT_GROUP_ADMIN.to_string()),
        drive(&rig.clock, groups.rename(77, "Ours"))?.map(|error| error.code)
    );
    assert_eq!("The cousins", rig.chat(77)?.expect("held").title);
    Ok(())
}

/// LEAVING asks with the reader's own id and is a `204`; the group goes from the cache at once. A
/// group already gone — deleted by its admin first — is the same outcome, and nothing is said.
#[test]
fn leaving_drops_the_group_even_if_it_was_already_gone() -> StoreResult<()> {
    let rig = Rig::new()?;
    let server = Server::new();
    server.then("/chats/77/members/7", &[(204, None)]);
    let groups = rig.build(&server);

    assert_eq!(None, drive(&rig.clock, groups.leave(77))?);
    assert_eq!(None, rig.chat(77)?);
    assert_eq!(
        vec!["/api/v1/chats/77/members/7".to_string()],
        server.asked()
    );

    rig.chats.upsert(&cousins())?;
    let gone = Server::new();
    gone.on_status(
        "/chats/77/members/7",
        404,
        Some(r#"{"error": {"code": "chat_not_found", "message": "gone"}}"#),
    );
    let again = rig.build(&gone);
    assert_eq!(None, drive(&rig.clock, again.leave(77))?);
    assert_eq!(None, rig.chat(77)?);
    Ok(())
}

/// Deleting drops it at once; a refusal keeps it, and says why; a transport failure keeps it too.
#[test]
fn deleting_drops_it_and_a_refusal_keeps_it() -> StoreResult<()> {
    let rig = Rig::new()?;
    let refusing = Server::new();
    refusing.on_status(
        "/chats/77",
        403,
        Some(r#"{"error": {"code": "not_group_admin", "message": "no"}}"#),
    );
    let refused = rig.build(&refusing);
    assert_eq!(
        Some(error_codes::NOT_GROUP_ADMIN.to_string()),
        drive(&rig.clock, refused.delete(77))?.map(|error| error.code)
    );
    assert!(rig.chat(77)?.is_some());

    let server = Server::new();
    server.on_status("/chats/77", 204, None);
    let groups = rig.build(&server);
    assert_eq!(None, drive(&rig.clock, groups.delete(77))?);
    assert_eq!(None, rig.chat(77)?);
    // The family chat is never taken this way, whatever answered.
    assert!(rig.chat(42)?.is_some());
    Ok(())
}
