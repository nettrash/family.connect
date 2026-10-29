//! `SeveralFamiliesTests`, the rules' part (docs/protocol.md, "Several families"): which chats a
//! family lists, which family an upload or a notification is for, which doors are open, the
//! switcher, the invitation banner, and the owner's "Invite…".

mod common;

use std::rc::Rc;

use fc_core::protocol::{
    ChatDto, ChatRowDto, FamilyDto, InvitationDto, MemberDto, MemoryTokenStore, PendingJoinDto,
    UserDto,
};
use fc_core::store::{ChatStore, Database, StoreResult};
use fc_core::testing::{FakeClock, drive};
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::app_session::{Gate, SessionState};
use fc_logic::families::{
    InviteModel, SwitcherRowKind, family_rules, family_switcher, invitation_text,
};
use time::macros::datetime;

use common::server::Server;
use common::session::api_with;

const INVITATION: &str = r#"{"id": 4, "family_id": 9, "family_name": "Sidorovs", "invited_by": {"id": 11, "username": "carl", "display_name": "Carl"}, "user": {"id": 7, "username": "anna", "display_name": "Anna"}}"#;

fn invitation() -> InvitationDto {
    serde_json::from_str(INVITATION).expect("an invitation")
}

fn family(id: i64, name: &str, owner: bool) -> FamilyDto {
    FamilyDto {
        role: Some(if owner { "owner" } else { "member" }.to_string()),
        ..FamilyDto::new(id, name)
    }
}

/// `State(families, current, enabled, cap, pending, invitations)`.
struct State {
    families: Vec<FamilyDto>,
    current: Option<i64>,
    enabled: bool,
    cap: i32,
    pending: Option<Vec<PendingJoinDto>>,
    invitations: Option<Vec<InvitationDto>>,
}

impl State {
    fn of(families: Vec<FamilyDto>) -> State {
        State {
            families,
            current: None,
            enabled: true,
            cap: 5,
            pending: None,
            invitations: None,
        }
    }

    fn build(self) -> SessionState {
        let shown = self
            .current
            .or_else(|| self.families.first().map(|family| family.id));
        let family = self
            .families
            .iter()
            .find(|held| Some(held.id) == shown)
            .cloned();
        let gate = match &family {
            None => Gate::NoFamily,
            Some(family) if family.is_owner() => Gate::Owner,
            Some(_) => Gate::Member,
        };
        SessionState {
            me: Some(UserDto::new(7, "anna", "Anna")),
            family,
            families: Some(self.families),
            current_family_id: shown,
            pending_joins: self.pending,
            invitations: self.invitations,
            member_families_enabled: self.enabled,
            max_families_per_account: self.cap,
            names_families: true,
            ..SessionState::new(gate)
        }
    }
}

fn chat(id: i64, kind: &str, title: &str, families: Option<Vec<i64>>) -> ChatDto {
    ChatDto {
        family_ids: families,
        ..ChatDto::new(id, kind, title)
    }
}

fn direct(families: Vec<i64>) -> ChatDto {
    ChatDto {
        peer_user_id: Some(9),
        family_ids: Some(families),
        ..ChatDto::new(44, "direct", "Bob")
    }
}

/// A family shows the chats that name it — a direct chat in each family it names — and an unnamed
/// chat in the home family.
#[test]
fn a_family_lists_the_chats_that_name_it() {
    let direct = direct(vec![3, 5]);
    let ivanovs = chat(43, "family", "Ivanovs", Some(vec![5]));
    assert!(family_rules::lists(&direct, Some(3), Some(3)));
    assert!(family_rules::lists(&direct, Some(5), Some(3)));
    assert!(!family_rules::lists(&ivanovs, Some(3), Some(3)));
    assert!(family_rules::lists(
        &chat(42, "family", "The Smiths", None),
        Some(3),
        Some(3)
    ));
    assert!(family_rules::lists(&ivanovs, None, Some(3)));
    // An upload is for the family shown when the chat is listed there, else the chat's first.
    assert_eq!(
        Some(5),
        family_rules::upload_family(Some(&direct), Some(5), Some(3))
    );
    assert_eq!(
        Some(5),
        family_rules::upload_family(Some(&ivanovs), Some(3), Some(3))
    );
    assert_eq!(Some(3), family_rules::upload_family(None, Some(3), Some(3)));
    // A notification opens its chat where it is listed.
    assert_eq!(
        None,
        family_rules::family_to_open(&direct, Some(5), Some(3))
    );
    assert_eq!(
        Some(5),
        family_rules::family_to_open(&ivanovs, Some(3), Some(3))
    );
    assert_eq!(
        Some("Ivanovs"),
        family_rules::name_of_chat(
            &ivanovs,
            &[family(3, "The Smiths", false), family(5, "Ivanovs", false)]
        )
    );
}

/// A chat with NO `family_ids` — cached before them, or from a server that predates them, where
/// absent "means the one family the account has" (docs/protocol.md, `Chat`) — is the HOME family's
/// alone, as Android's `listedIn` and the web's `listed_in` read it: not shown in another family, not
/// counted in its unread, its uploads for the home family (not the one shown — the server would
/// answer `attachment_not_found`), and a notification opens it there. An explicit `[]` is still
/// listed nowhere.
#[test]
fn a_chat_with_no_families_is_the_home_familys_only() -> StoreResult<()> {
    let unnamed = chat(42, "family", "The Smiths", None);
    assert!(family_rules::lists(&unnamed, Some(3), Some(3)));
    assert!(!family_rules::lists(&unnamed, Some(5), Some(3)));
    assert!(!family_rules::lists(&unnamed, Some(5), None));
    assert!(!family_rules::lists(&direct(vec![]), Some(3), Some(3)));
    assert_eq!(vec![3], family_rules::families_of(&unnamed, Some(3)));
    assert!(family_rules::families_of(&direct(vec![]), Some(3)).is_empty());

    // An attachment sent there from family 5 is uploaded for the home family, where the chat is.
    assert_eq!(
        Some(3),
        family_rules::upload_family(Some(&unnamed), Some(5), Some(3))
    );
    assert_eq!(
        Some(3),
        family_rules::upload_family(Some(&unnamed), Some(3), Some(3))
    );
    assert_eq!(
        Some(5),
        family_rules::upload_family(Some(&unnamed), Some(5), None)
    );
    assert_eq!(
        Some(3),
        family_rules::family_to_open(&unnamed, Some(5), Some(3))
    );
    assert_eq!(
        None,
        family_rules::family_to_open(&unnamed, Some(3), Some(3))
    );
    // Titled with the home family's name — the account's first — not the shown one's.
    assert_eq!(
        Some("The Smiths"),
        family_rules::name_of_chat(
            &unnamed,
            &[family(3, "The Smiths", false), family(5, "Ivanovs", false)]
        )
    );
    assert_eq!(None, family_rules::name_of_chat(&unnamed, &[]));

    // The switcher counts it in the home family only.
    let state = State {
        current: Some(5),
        ..State::of(vec![
            family(3, "The Smiths", false),
            family(5, "Ivanovs", false),
        ])
    }
    .build();
    assert_eq!(Some(3), state.home_family_id());
    let rows = family_switcher::rows(
        &state,
        &[ChatRowDto {
            unread_count: 2,
            ..ChatRowDto::new(unnamed.clone())
        }],
        &common::english(),
    );
    assert_eq!(
        vec![2, 0],
        rows.iter()
            .filter(|row| row.kind == SwitcherRowKind::Family)
            .map(|row| row.unread)
            .collect::<Vec<_>>()
    );

    // And the list of family 5 does not show it, where family 3's does.
    let cache = Database::open_in_memory()?;
    let shown = Rc::new(std::cell::Cell::new(Some(5)));
    let reading = shown.clone();
    let chats = Rc::new(
        ChatStore::new(&cache)
            .with_me(|| 7)
            .with_current(move || reading.get())
            .with_home_family(|| Some(3)),
    );
    chats.replace(
        &[
            ChatRowDto::new(unnamed),
            ChatRowDto::new(chat(43, "family", "Ivanovs", Some(vec![5]))),
        ],
        None,
    )?;
    let list = fc_logic::chat_list::ChatListModel::new(chats, || 7, Rc::new(common::english()));
    let now = datetime!(2026-10-22 10:00 UTC);
    let ids = |list: &fc_logic::chat_list::ChatListModel| -> StoreResult<Vec<i64>> {
        Ok(list.rows(now)?.iter().map(|row| row.chat.id).collect())
    };
    assert_eq!(vec![43], ids(&list)?);
    shown.set(Some(3));
    assert_eq!(vec![42], ids(&list)?);
    Ok(())
}

/// A notification for a chat listed in NO family opens where the reader is, and does not panic: a
/// toast for a direct chat with Bob, then the last family the two shared left — `chat_updated` with
/// `family_ids: []`, the row kept, because only a group is removed — and the old toast clicked. The
/// rule used to index the empty array.
#[test]
fn a_notification_for_a_chat_listed_nowhere_stays_in_the_family_shown() -> StoreResult<()> {
    let cache = Database::open_in_memory()?;
    let chats = ChatStore::new(&cache)
        .with_me(|| 7)
        .with_current(|| Some(3))
        .with_home_family(|| Some(3));
    chats.replace(&[ChatRowDto::new(direct(vec![3, 5]))], None)?;
    chats.upsert(&direct(vec![]))?;
    assert!(!chats.remove(44)?);
    let held = chats.chat(44)?.expect("kept").chat;
    assert!(held.family_ids.as_deref().is_some_and(<[i64]>::is_empty));

    assert_eq!(None, family_rules::family_to_open(&held, Some(3), Some(3)));
    assert_eq!(None, family_rules::family_to_open(&held, Some(5), Some(3)));
    assert_eq!(None, family_rules::family_to_open(&held, None, None));
    assert_eq!(
        None,
        family_rules::name_of_chat(
            &held,
            &[family(3, "The Smiths", false), family(5, "Ivanovs", false)]
        )
    );
    assert_eq!(
        Some(5),
        family_rules::upload_family(Some(&held), Some(5), Some(3))
    );
    Ok(())
}

/// "New Family…" while the members' door is open, the account owns none of its families and is
/// under the cap; "Join Another Family…" while under the cap; "Invite…" for an owner with another
/// family to invite from.
#[test]
fn the_doors_are_offered_only_while_open() {
    let smiths = || family(3, "The Smiths", false);
    let ivanovs = || family(5, "Ivanovs", true);
    let one = State::of(vec![smiths()]).build();
    assert!(family_rules::offers_new_family(&one));
    assert!(family_rules::offers_join_another(&one));
    assert!(!family_rules::shows_switcher(&one));
    assert!(!family_rules::offers_new_family(
        &State {
            enabled: false,
            ..State::of(vec![smiths()])
        }
        .build()
    ));
    assert!(!family_rules::offers_new_family(
        &State::of(vec![smiths(), ivanovs()]).build()
    ));
    assert!(!family_rules::offers_new_family(
        &State {
            cap: 1,
            ..State::of(vec![smiths()])
        }
        .build()
    ));
    assert!(!family_rules::offers_join_another(
        &State {
            cap: 1,
            ..State::of(vec![smiths()])
        }
        .build()
    ));
    assert!(!family_rules::offers_new_family(&State::of(vec![]).build()));
    assert!(!family_rules::offers_join_another(
        &State::of(vec![]).build()
    ));
    let owner = State {
        current: Some(5),
        ..State::of(vec![smiths(), ivanovs()])
    }
    .build();
    assert!(family_rules::offers_invite(&owner));
    assert!(!family_rules::offers_invite(
        &State::of(vec![ivanovs()]).build()
    ));
    assert!(!family_rules::offers_invite(
        &State {
            current: Some(3),
            ..State::of(vec![smiths(), ivanovs()])
        }
        .build()
    ));
    // One family and something pending: the switcher shows, to show it.
    assert!(family_rules::shows_switcher(
        &State {
            invitations: Some(vec![invitation()]),
            ..State::of(vec![smiths()])
        }
        .build()
    ));
    assert!(family_rules::shows_switcher(
        &State {
            pending: Some(vec![PendingJoinDto {
                family_id: 8,
                family_name: "Petrovs".to_string(),
                created_at: None,
            }]),
            ..State::of(vec![smiths()])
        }
        .build()
    ));
    assert!(family_rules::shows_switcher(&owner));
}

/// The switcher: every family with the unread of the chats listed in it — a direct chat counting in
/// each family that lists it — its accessibility label, the requests waiting, the invitations, and
/// the doors.
#[test]
fn the_switcher_lists_every_family_with_its_unread() {
    let say = common::english();
    let state = State {
        pending: Some(vec![PendingJoinDto {
            family_id: 8,
            family_name: "Petrovs".to_string(),
            created_at: None,
        }]),
        invitations: Some(vec![invitation()]),
        ..State::of(vec![
            family(3, "The Smiths", false),
            family(5, "Ivanovs", false),
        ])
    }
    .build();
    let chats = [
        ChatRowDto {
            unread_count: 2,
            ..ChatRowDto::new(chat(42, "family", "The Smiths", Some(vec![3])))
        },
        ChatRowDto {
            unread_count: 1,
            ..ChatRowDto::new(chat(43, "family", "Ivanovs", Some(vec![5])))
        },
        ChatRowDto {
            unread_count: 4,
            ..ChatRowDto::new(direct(vec![3, 5]))
        },
    ];
    let rows = family_switcher::rows(&state, &chats, &say);

    assert_eq!(
        vec![
            SwitcherRowKind::Family,
            SwitcherRowKind::Family,
            SwitcherRowKind::Pending,
            SwitcherRowKind::Invitation,
            SwitcherRowKind::NewFamily,
            SwitcherRowKind::JoinAnother
        ],
        rows.iter().map(|row| row.kind).collect::<Vec<_>>()
    );
    assert_eq!(6, rows[0].unread);
    assert!(rows[0].current);
    assert_eq!(5, rows[1].unread);
    assert_eq!(
        Some("Ivanovs, 5 unread"),
        rows[1].accessible_name.as_deref()
    );
    assert_eq!(Some("Waiting for approval"), rows[2].detail.as_deref());
    assert_eq!(
        Some("Carl invited you to “Sidorovs”"),
        rows[3].detail.as_deref()
    );
    assert_eq!("New Family…", rows[4].title);
    assert_eq!("Join Another Family…", rows[5].title);
    assert_eq!("Switch Family", say.get("Switch Family"));
}

#[test]
fn the_banner_names_the_inviter_and_the_family() {
    let say = common::english();
    let invitation = invitation();
    assert_eq!(
        "Carl invited you to “Sidorovs”",
        invitation_text::banner(&invitation, &say)
    );
    let mut gone = invitation.clone();
    gone.invited_by.deleted = true;
    assert_eq!(
        "Deleted account invited you to “Sidorovs”",
        invitation_text::banner(&gone, &say)
    );
}

/// "Invite…" lists the members of the owner's OTHER families who are not in this one, each once and
/// in name order, marked "Invited" when an invitation is pending — never the owner, a former member,
/// or somebody already here.
#[test]
fn the_invite_picker_lists_the_other_families_members() -> StoreResult<()> {
    let cache = Database::open_in_memory()?;
    let chats = Rc::new(
        ChatStore::new(&cache)
            .with_me(|| 7)
            .with_current(|| Some(5)),
    );
    let member = |id: i64, username: &str, name: &str, role: Option<&str>| MemberDto {
        role: role.map(str::to_string),
        ..MemberDto::new(id, username, name)
    };
    chats.replace_members_of(
        Some(5),
        &[
            member(7, "anna", "Anna", Some("owner")),
            member(9, "bob", "Bob", None),
        ],
        &[],
    )?;
    chats.replace_members_of(
        Some(3),
        &[
            member(7, "anna", "Anna", None),
            member(9, "bob", "Bob", None),
            member(12, "zoe", "Zoe", None),
            member(11, "carl", "Carl", Some("owner")),
        ],
        &[MemberDto {
            has_left: true,
            ..MemberDto::new(13, "dan", "Dan")
        }],
    )?;
    chats.replace_members_of(
        Some(8),
        &[
            member(12, "zoe", "Zoe", None),
            member(14, "eve", "Eve", None),
        ],
        &[],
    )?;
    let state = State {
        current: Some(5),
        ..State::of(vec![
            family(3, "The Smiths", false),
            family(5, "Ivanovs", true),
            family(8, "Petrovs", false),
        ])
    }
    .build();
    let pending = vec![InvitationDto {
        user: UserDto::new(14, "eve", "Eve"),
        ..invitation()
    }];
    let clock = FakeClock::at(datetime!(2026-10-22 10:00 UTC));
    let api = api_with(
        Rc::new(Server::new()),
        &clock,
        Rc::new(MemoryTokenStore::new(None)),
    );

    let candidates = InviteModel::new(api, chats).candidates(&state, &pending)?;

    assert_eq!(
        vec!["Carl", "Eve", "Zoe"],
        candidates
            .iter()
            .map(|candidate| candidate.member.display_name.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        vec![false, true, false],
        candidates
            .iter()
            .map(|candidate| candidate.invited)
            .collect::<Vec<_>>()
    );
    Ok(())
}

/// The owner's three invitation calls, for the family shown.
#[test]
fn the_owner_invites_lists_and_withdraws() -> StoreResult<()> {
    let server = Server::new();
    server
        .on(
            "/families/5/invitations",
            &format!(r#"{{"invitation": {INVITATION}, "invitations": [{INVITATION}]}}"#),
        )
        .on_status("/families/5/invitations/4", 204, None);
    let clock = FakeClock::at(datetime!(2026-10-22 10:00 UTC));
    let api = api_with(
        Rc::new(server.clone()),
        &clock,
        Rc::new(MemoryTokenStore::new(Some("t0ken"))),
    );
    let invites = InviteModel::new(api, Rc::new(ChatStore::new(&Database::open_in_memory()?)));
    let invited = drive(&clock, invites.invite(5, 9)).expect("invited");
    assert_eq!(4, invited.id);
    let pending = drive(&clock, invites.pending(5)).expect("listed");
    assert_eq!(1, pending.len());
    assert_eq!(None, drive(&clock, invites.withdraw(5, 4)));
    assert_eq!(r#"{"user_id":9}"#, server.bodies()[0]);
    Ok(())
}
