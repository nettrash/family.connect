//! `SeveralFamiliesTests`, the session's part: one account in several families (docs/protocol.md,
//! "Several families"; issue #88) — the list of families and the one shown, switching, a family that
//! went, the older server, and invitations to the reader. The rules, the switcher and the picker are
//! `families_several_families.rs`; the doors, `doorway_several_families.rs`; the chat list,
//! `chat_list_several_families.rs`; the frames, `live_connection_several_families.rs`.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use fc_core::protocol::{
    ApiClient, ChatDto, ChatRowDto, InvitationDto, MemoryTokenStore, NoteDto, UserDto, error_codes,
};
use fc_core::store::{BoardStore, ChatStore, Database, StoreResult};
use fc_core::testing::{FakeClock, drive};
use fc_logic::app_session::{AppSession, CurrentFamily, Gate, MemoryCurrentFamily, SessionEnd};
use fc_logic::families::family_sentences;
use time::macros::datetime;

use common::server::Server;
use common::session::api_with;

const SMITHS: &str = r#"{"id": 3, "name": "The Smiths", "role": "member"}"#;
const IVANOVS: &str = r#"{"id": 5, "name": "Ivanovs", "role": "owner"}"#;

const INVITATION: &str = r#"{"id": 4, "family_id": 9, "family_name": "Sidorovs", "invited_by": {"id": 11, "username": "carl", "display_name": "Carl"}, "user": {"id": 7, "username": "anna", "display_name": "Anna"}}"#;

/// `GET /me` of a server that knows several families.
fn me(families: &str, pending: &str, invitations: &str) -> String {
    format!(
        r#"{{"user": {{"id": 7, "username": "anna", "display_name": "Anna"}}, "blocked_user_ids": [],
         "families": [{families}], "pending_join_requests": [{pending}], "invitations": [{invitations}],
         "member_families_enabled": true, "max_families_per_account": 5}}"#
    )
}

fn me_of(families: &str) -> String {
    me(families, "", "")
}

struct Rig {
    session: AppSession,
    api: Rc<ApiClient>,
    server: Server,
    remembered: Rc<MemoryCurrentFamily>,
    clock: FakeClock,
}

impl Rig {
    fn refresh(&self) -> StoreResult<Option<fc_core::protocol::ApiError>> {
        drive(&self.clock, self.session.refresh())
    }
}

fn build(
    server: &Server,
    cache: &Rc<Database>,
    remembered: Option<Rc<MemoryCurrentFamily>>,
) -> Rig {
    let clock = FakeClock::at(datetime!(2026-10-22 10:00 UTC));
    let tokens = Rc::new(MemoryTokenStore::new(Some("t0ken")));
    let api = api_with(Rc::new(server.clone()), &clock, tokens.clone());
    let remembered = remembered.unwrap_or_default();
    Rig {
        session: AppSession::new(api.clone(), tokens, cache.clone())
            .with_current_family(remembered.clone()),
        api,
        server: server.clone(),
        remembered,
        clock,
    }
}

fn invitation() -> InvitationDto {
    serde_json::from_str(INVITATION).expect("an invitation")
}

fn ids(session: &AppSession) -> Vec<i64> {
    session
        .state()
        .all_families()
        .iter()
        .map(|family| family.id)
        .collect()
}

/// The model is a LIST: every family from `GET /me`, the home family shown first by default, the
/// gate decided by the role in the family SHOWN — and that family named on every family endpoint
/// from here on.
#[test]
fn the_first_family_is_shown_and_its_role_decides_the_gate() -> StoreResult<()> {
    let server = Server::new();
    server.on("/me", &me_of(&format!("{SMITHS}, {IVANOVS}")));
    let rig = build(&server, &Database::open_in_memory()?, None);
    assert_eq!(None, rig.refresh()?);

    let state = rig.session.state();
    assert_eq!(vec![3, 5], ids(&rig.session));
    assert_eq!(Some(3), state.current_family_id);
    assert_eq!("The Smiths", state.family.as_ref().expect("shown").name);
    assert_eq!(Gate::Member, state.gate);
    assert!(state.names_families);
    assert_eq!(5, state.max_families_per_account);
    assert_eq!(Some(3), rig.api.family_scope());
    assert_eq!(Some(3), rig.remembered.family_for(7));
    Ok(())
}

/// The family last shown is remembered per account, so a relaunch opens where the reader left off.
#[test]
fn the_family_last_shown_is_shown_again() -> StoreResult<()> {
    let remembered = Rc::new(MemoryCurrentFamily::new());
    remembered.remember(7, 5);
    let server = Server::new();
    server.on("/me", &me_of(&format!("{SMITHS}, {IVANOVS}")));
    let rig = build(
        &server,
        &Database::open_in_memory()?,
        Some(remembered.clone()),
    );
    rig.refresh()?;
    assert_eq!(Some(5), rig.session.state().current_family_id);
    assert_eq!(Gate::Owner, rig.session.state().gate);
    // A remembered family the account is no longer in is no answer: the first is.
    remembered.remember(7, 99);
    let other_server = Server::new();
    other_server.on("/me", &me_of(SMITHS));
    let other = build(
        &other_server,
        &Database::open_in_memory()?,
        Some(remembered),
    );
    other.refresh()?;
    assert_eq!(Some(3), other.session.state().current_family_id);
    Ok(())
}

/// Switching shows the other family: its role decides the gate, it is remembered, it is named on
/// the wire, and its own document is read for what only it carries.
#[test]
fn switching_shows_the_other_family() -> StoreResult<()> {
    let server = Server::new();
    server
        .on("/me", &me_of(&format!("{SMITHS}, {IVANOVS}")))
        .on(
            "/families/5",
            r#"{"family": {"id": 5, "name": "Ivanovs", "invite_code": "ABCD2345"}, "assistant": {"user_id": 99, "display_name": "Assistant", "mention": "ai"}}"#,
        );
    let rig = build(&server, &Database::open_in_memory()?, None);
    rig.refresh()?;
    let gates = Rc::new(RefCell::new(Vec::new()));
    let seen = gates.clone();
    rig.session
        .on_changed()
        .subscribe(move |state| seen.borrow_mut().push(state.gate));

    assert_eq!(None, drive(&rig.clock, rig.session.switch_family(5))?);

    let state = rig.session.state();
    assert_eq!(Some(5), state.current_family_id);
    assert_eq!(Gate::Owner, state.gate);
    let family = state.family.as_ref().expect("shown");
    assert_eq!(Some("ABCD2345"), family.invite_code.as_deref());
    assert!(family.is_owner());
    assert_eq!(99, state.assistant.as_ref().expect("an assistant").user_id);
    assert_eq!(Some(5), rig.api.family_scope());
    assert_eq!(Some(5), rig.remembered.family_for(7));
    assert!(
        rig.server
            .asked()
            .contains(&"/api/v1/families/5".to_string())
    );
    assert!(gates.borrow().iter().all(|gate| *gate == Gate::Owner));
    // A family that is not the account's, or the one already shown, is no switch.
    assert!(!rig.session.switch(5));
    assert!(!rig.session.switch(42));
    Ok(())
}

/// A family that is no longer in `GET /me` — left on another device, removed — goes with its rows,
/// and only its; the first that remains is shown, and nothing says the session ended.
#[test]
fn a_family_gone_from_me_takes_only_its_rows() -> StoreResult<()> {
    let cache = Database::open_in_memory()?;
    let server = Server::new();
    server.then(
        "/me",
        &[
            (200, Some(&me_of(&format!("{SMITHS}, {IVANOVS}")))),
            (200, Some(&me_of(SMITHS))),
        ],
    );
    let rig = Rc::new(build(&server, &cache, None));
    rig.remembered.remember(7, 5);
    rig.refresh()?;
    assert_eq!(Some(5), rig.session.state().current_family_id);
    let shown = rig.clone();
    let chats = ChatStore::new(&cache)
        .with_me(|| 7)
        .with_current(move || shown.session.state().current_family_id);
    chats.replace(
        &[
            ChatRowDto::new(ChatDto {
                family_ids: Some(vec![3]),
                ..ChatDto::new(42, "family", "The Smiths")
            }),
            ChatRowDto::new(ChatDto {
                family_ids: Some(vec![5]),
                ..ChatDto::new(43, "family", "Ivanovs")
            }),
        ],
        None,
    )?;
    BoardStore::new(&cache)
        .in_family(Some(5))
        .replace(&[bread()], 9)?;
    let ended = Rc::new(RefCell::new(Vec::new()));
    let dropped = Rc::new(RefCell::new(Vec::new()));
    let (heard, gone) = (ended.clone(), dropped.clone());
    rig.session
        .on_ended()
        .subscribe(move |end| heard.borrow_mut().push(*end));
    rig.session
        .on_family_dropped()
        .subscribe(move |family| gone.borrow_mut().push(*family));

    rig.refresh()?;

    assert_eq!(Some(3), rig.session.state().current_family_id);
    assert_eq!(Gate::Member, rig.session.state().gate);
    assert!(ended.borrow().is_empty());
    assert_eq!(vec![5], *dropped.borrow());
    assert_eq!(None, chats.chat(43)?);
    assert!(chats.chat(42)?.is_some());
    assert!(
        BoardStore::new(&cache)
            .in_family(Some(5))
            .notes()?
            .is_empty()
    );
    Ok(())
}

fn bread() -> NoteDto {
    NoteDto {
        kind: Some("text".to_string()),
        text: Some("Bread".to_string()),
        board_seq: 9,
        ..NoteDto::new(2, 11)
    }
}

fn milk() -> NoteDto {
    NoteDto {
        kind: Some("text".to_string()),
        text: Some("Milk".to_string()),
        board_seq: 4,
        ..NoteDto::new(1, 9)
    }
}

/// A family this cache holds and `GET /me` does not list — left while the app was closed — goes at
/// the first answer.
#[test]
fn a_family_left_while_closed_is_dropped_at_the_first_answer() -> StoreResult<()> {
    let cache = Database::open_in_memory()?;
    BoardStore::new(&cache)
        .in_family(Some(5))
        .replace(&[bread()], 9)?;
    BoardStore::new(&cache)
        .in_family(Some(3))
        .replace(&[milk()], 4)?;
    let server = Server::new();
    server.on("/me", &me_of(SMITHS));
    let rig = build(&server, &cache, None);
    rig.refresh()?;
    assert!(
        BoardStore::new(&cache)
            .in_family(Some(5))
            .notes()?
            .is_empty()
    );
    assert_eq!(1, BoardStore::new(&cache).in_family(Some(3)).notes()?.len());
    Ok(())
}

/// The LAST family gone is today's ending, unchanged: everything wiped, and the reader told.
#[test]
fn the_last_family_gone_ends_as_before() -> StoreResult<()> {
    let cache = Database::open_in_memory()?;
    let server = Server::new();
    server.then(
        "/me",
        &[(200, Some(&me_of(SMITHS))), (200, Some(&me_of("")))],
    );
    let rig = build(&server, &cache, None);
    rig.refresh()?;
    ChatStore::new(&cache).replace(
        &[ChatRowDto::new(ChatDto {
            family_ids: Some(vec![3]),
            ..ChatDto::new(42, "family", "The Smiths")
        })],
        None,
    )?;
    let ended = Rc::new(RefCell::new(Vec::new()));
    let heard = ended.clone();
    rig.session
        .on_ended()
        .subscribe(move |end| heard.borrow_mut().push(*end));
    rig.refresh()?;
    assert_eq!(Gate::NoFamily, rig.session.state().gate);
    assert_eq!(vec![SessionEnd::RemovedFromFamily], *ended.borrow());
    assert!(ChatStore::new(&cache).chats()?.is_empty());
    Ok(())
}

/// An account with NO family gets today's gate; one with a family and a join request elsewhere is in
/// its family, and the request waits in the switcher.
#[test]
fn a_pending_request_is_a_gate_only_without_a_family() -> StoreResult<()> {
    const PETROVS: &str = r#"{"family_id": 8, "family_name": "Petrovs"}"#;
    let alone = Server::new();
    alone.on("/me", &me("", PETROVS, ""));
    let waiting = build(&alone, &Database::open_in_memory()?, None);
    waiting.refresh()?;
    assert_eq!(Gate::Pending, waiting.session.state().gate);
    assert_eq!(
        Some("Petrovs".to_string()),
        waiting.session.state().pending_family_name
    );

    let in_one = Server::new();
    in_one.on("/me", &me(SMITHS, PETROVS, ""));
    let member = build(&in_one, &Database::open_in_memory()?, None);
    member.refresh()?;
    let state = member.session.state();
    assert_eq!(Gate::Member, state.gate);
    assert_eq!(1, state.all_pending().len());
    assert_eq!(8, state.all_pending()[0].family_id);
    Ok(())
}

/// A server from before several families: its one family is the list, but it is never asked for a
/// family by id — the paths stay `mine` — and the cache an older build filled becomes that family's.
#[test]
fn an_older_server_is_asked_for_mine() -> StoreResult<()> {
    let cache = Database::open_in_memory()?;
    BoardStore::new(&cache).replace(&[milk()], 4)?;
    let server = Server::new();
    server.on(
        "/me",
        r#"{"user": {"id": 7, "username": "anna", "display_name": "Anna"}, "family": {"id": 3, "name": "The Smiths"}, "role": "owner"}"#,
    );
    let rig = build(&server, &cache, None);
    rig.refresh()?;
    let state = rig.session.state();
    assert_eq!(Gate::Owner, state.gate);
    assert_eq!(Some(3), state.current_family_id);
    assert!(!state.names_families);
    assert_eq!(None, rig.api.family_scope());
    let board = BoardStore::new(&cache).with_current(|| Some(3));
    let notes = board.notes()?;
    assert_eq!(1, notes.len());
    assert_eq!(1, notes[0].id);
    assert_eq!(4, board.cursor()?);
    Ok(())
}

/// Frames keep the invitations live: one to this account is listed, one to somebody else is not this
/// account's.
#[test]
fn invitation_frames_keep_the_list_live() -> StoreResult<()> {
    let server = Server::new();
    server.on("/me", &me_of(SMITHS));
    let rig = build(&server, &Database::open_in_memory()?, None);
    rig.refresh()?;
    let invitation = invitation();
    rig.session.apply_invitation(&invitation);
    rig.session.apply_invitation(&invitation);
    rig.session.apply_invitation(&InvitationDto {
        id: 6,
        user: UserDto::new(12, "dana", "Dana"),
        ..invitation.clone()
    });
    let state = rig.session.state();
    assert_eq!(1, state.all_invitations().len());
    assert_eq!(4, state.all_invitations()[0].id);
    rig.session.invitation_gone(4);
    assert!(rig.session.state().all_invitations().is_empty());
    Ok(())
}

/// Join: the family is added AND shown, and the invitation goes.
#[test]
fn joining_an_invitation_shows_the_family() -> StoreResult<()> {
    let server = Server::new();
    server
        .then(
            "/me",
            &[
                (200, Some(&me(SMITHS, "", INVITATION))),
                (
                    200,
                    Some(&me_of(&format!(
                        r#"{SMITHS}, {{"id": 9, "name": "Sidorovs", "role": "member"}}"#
                    ))),
                ),
            ],
        )
        .on(
            "/invitations/4/accept",
            r#"{"family": {"id": 9, "name": "Sidorovs"}}"#,
        );
    let rig = build(&server, &Database::open_in_memory()?, None);
    rig.refresh()?;
    assert_eq!(1, rig.session.state().all_invitations().len());

    assert_eq!(None, drive(&rig.clock, rig.session.accept_invitation(4))?);

    assert_eq!(Some(9), rig.session.state().current_family_id);
    assert!(rig.session.state().all_invitations().is_empty());
    Ok(())
}

/// `invitation_not_pending` drops it — it is gone, whatever the reason — and comes back for the
/// window to say; a full family or the cap leaves it standing.
#[test]
fn an_invitation_no_longer_pending_is_dropped() -> StoreResult<()> {
    let server = Server::new();
    server.on("/me", &me(SMITHS, "", INVITATION)).then(
        "/invitations/4/accept",
        &[
            (
                409,
                Some(r#"{"error": {"code": "too_many_families", "message": "cap"}}"#),
            ),
            (
                409,
                Some(r#"{"error": {"code": "invitation_not_pending", "message": "gone"}}"#),
            ),
        ],
    );
    let rig = build(&server, &Database::open_in_memory()?, None);
    rig.refresh()?;

    assert_eq!(
        error_codes::TOO_MANY_FAMILIES,
        drive(&rig.clock, rig.session.accept_invitation(4))?
            .expect("refused")
            .code
    );
    assert_eq!(1, rig.session.state().all_invitations().len());
    let gone = drive(&rig.clock, rig.session.accept_invitation(4))?.expect("refused");
    assert_eq!(error_codes::INVITATION_NOT_PENDING, gone.code);
    assert!(rig.session.state().all_invitations().is_empty());
    assert_eq!(
        Some("This invitation is no longer available.".to_string()),
        family_sentences::for_error(&gone, 5, &common::english())
    );
    Ok(())
}

#[test]
fn declining_an_invitation_drops_it() -> StoreResult<()> {
    let server = Server::new();
    server
        .on("/me", &me(SMITHS, "", INVITATION))
        .on_status("/invitations/4/decline", 204, None);
    let rig = build(&server, &Database::open_in_memory()?, None);
    rig.refresh()?;
    assert_eq!(None, drive(&rig.clock, rig.session.decline_invitation(4)));
    assert!(rig.session.state().all_invitations().is_empty());
    assert!(
        rig.server
            .asked()
            .contains(&"/api/v1/invitations/4/decline".to_string())
    );
    Ok(())
}
