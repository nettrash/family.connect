//! One account in several families, as the frames and the resync apply it (docs/protocol.md,
//! "Several families", issue #88): a family frame goes to the family it names, one naming the
//! reader says they are in or out of a family, and a resync reads every family into its own rows —
//! the six router tests and the two resync tests of
//! `win/tests/FamilyConnect.Core.Tests/Protocol/SeveralFamiliesProtocolTests.cs`. (Its client tests
//! are in `protocol_several_families_client.rs`, its model tests in
//! `protocol_several_families_model.rs`.)

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use fc_core::protocol::{
    ApiClient, AttachmentDto, ChatDto, ChatRowDto, FrameRouter, InvitationDto, MemberDto,
    MemoryTokenStore, NoteDto, PackItemDto, Resync, ServerFrame, ServerUrl,
};
use fc_core::store::{BoardStore, ChatStore, Database, PackStore, StoreResult};
use fc_core::testing::{FakeClock, FakeServer, drive};
use time::macros::datetime;

/// The family shown, as a session says it — changed by the test as a session would change it.
type Shown = Rc<Cell<Option<i64>>>;

fn shown(family: i64) -> Shown {
    Rc::new(Cell::new(Some(family)))
}

fn chats_showing(database: &Rc<Database>, shown: &Shown) -> Rc<ChatStore> {
    let current = shown.clone();
    Rc::new(
        ChatStore::new(database)
            .with_me(|| 7)
            .with_current(move || current.get()),
    )
}

fn board_showing(database: &Rc<Database>, shown: &Shown) -> Rc<BoardStore> {
    let current = shown.clone();
    Rc::new(BoardStore::new(database).with_current(move || current.get()))
}

fn pack_showing(database: &Rc<Database>, shown: &Shown) -> Rc<PackStore> {
    let current = shown.clone();
    Rc::new(PackStore::new(database).with_current(move || current.get()))
}

fn person(id: i64, username: &str, name: &str, role: &str) -> MemberDto {
    MemberDto {
        role: Some(role.to_string()),
        ..MemberDto::new(id, username, name)
    }
}

fn frame(json: &str) -> ServerFrame {
    ServerFrame::parse(json).unwrap_or_else(|| panic!("a frame: {json}"))
}

fn find(members: &[MemberDto], id: i64) -> MemberDto {
    let matching: Vec<&MemberDto> = members.iter().filter(|member| member.id == id).collect();
    assert_eq!(matching.len(), 1, "exactly one member {id}");
    matching[0].clone()
}

fn ids<T>(items: &[T], id: impl Fn(&T) -> i64) -> Vec<i64> {
    items.iter().map(id).collect()
}

fn one<T>(mut items: Vec<T>) -> T {
    assert_eq!(items.len(), 1, "exactly one");
    items.remove(0)
}

// ---- the frames -----------------------------------------------------------------------------------

/// The flag a join carries reaches the roster, and a join WITHOUT one — everybody else's copy, or
/// the owner's racing a roster read that already said — keeps the flag held: a newcomer also in a
/// family this owner does not own is never offered a reset the server would refuse with
/// `member_in_other_families`.
#[test]
fn a_join_without_the_flag_keeps_the_one_held() -> StoreResult<()> {
    let database = Database::open_in_memory()?;
    let shown = shown(5);
    let chats = chats_showing(&database, &shown);
    chats.replace_members_of(Some(5), &[person(7, "anna", "Anna", "owner")], &[])?;
    let router = FrameRouter::new(chats.clone(), board_showing(&database, &shown), None);
    let flag =
        || -> StoreResult<Option<bool>> { Ok(find(&chats.members_of(5)?, 12).password_reset) };

    router.hear(&frame(
        r#"{"type": "member_joined", "family_id": 5, "user": {"id": 12, "username": "dana", "display_name": "Dana"}, "password_reset": false}"#,
    ))?;
    assert_eq!(flag()?, Some(false));
    assert_eq!(find(&chats.members()?, 12).password_reset, Some(false));

    router.hear(&frame(
        r#"{"type": "member_joined", "family_id": 5, "user": {"id": 12, "username": "dana", "display_name": "Dana"}}"#,
    ))?;
    assert_eq!(flag()?, Some(false));

    router.hear(&frame(
        r#"{"type": "member_joined", "family_id": 5, "user": {"id": 12, "username": "dana", "display_name": "Dana"}, "password_reset": true}"#,
    ))?;
    assert_eq!(flag()?, Some(true));

    // A newcomer nothing was ever said about: the old rule, where the reset is offered.
    router.hear(&frame(
        r#"{"type": "member_joined", "family_id": 5, "user": {"id": 13, "username": "eve", "display_name": "Eve"}}"#,
    ))?;
    assert_eq!(find(&chats.members_of(5)?, 13).password_reset, None);
    Ok(())
}

/// A JOIN NEVER DOWNGRADES. A family started in the app is announced by a `member_joined` naming
/// its CREATOR, who is its owner — and that frame may land after the roster read that said so. The
/// role held stays; a newcomer, and a former member back again, are members; the reset flag still
/// follows the frame when it carries one.
#[test]
fn a_join_naming_the_owner_leaves_them_the_owner() -> StoreResult<()> {
    let database = Database::open_in_memory()?;
    let shown = shown(5);
    let chats = chats_showing(&database, &shown);
    chats.replace_members_of(
        Some(3),
        &[
            person(7, "anna", "Anna", "member"),
            person(9, "bob", "Bob", "owner"),
        ],
        &[],
    )?;
    chats.replace_members_of(
        Some(5),
        &[
            person(7, "anna", "Anna", "owner"),
            MemberDto {
                password_reset: Some(true),
                ..person(11, "carl", "Carl", "member")
            },
        ],
        &[],
    )?;
    let router = FrameRouter::new(chats.clone(), board_showing(&database, &shown), None);

    router.hear(&frame(
        r#"{"type": "member_joined", "family_id": 5, "user": {"id": 7, "username": "anna", "display_name": "Anna"}}"#,
    ))?;
    assert!(find(&chats.members_of(5)?, 7).owner());
    assert!(find(&chats.members()?, 7).owner());
    // Family 3's role is family 3's.
    assert!(!find(&chats.members_of(3)?, 7).owner());

    // A member named again stays a member, and the flag the owner's copy carries still lands.
    router.hear(&frame(
        r#"{"type": "member_joined", "family_id": 5, "user": {"id": 11, "username": "carl", "display_name": "Carl"}, "password_reset": false}"#,
    ))?;
    let carl = find(&chats.members_of(5)?, 11);
    assert_eq!(carl.role.as_deref(), Some("member"));
    assert_eq!(carl.password_reset, Some(false));

    // A newcomer is a member; so is somebody who left and came back, whatever they were before.
    router.hear(&frame(
        r#"{"type": "member_joined", "family_id": 3, "user": {"id": 12, "username": "dana", "display_name": "Dana"}}"#,
    ))?;
    assert_eq!(
        find(&chats.members_of(3)?, 12).role.as_deref(),
        Some("member")
    );
    router.hear(&frame(
        r#"{"type": "member_left", "family_id": 3, "user_id": 9}"#,
    ))?;
    router.hear(&frame(
        r#"{"type": "member_joined", "family_id": 3, "user": {"id": 9, "username": "bob", "display_name": "Bob"}}"#,
    ))?;
    let bob = find(&chats.members_of(3)?, 9);
    assert!(!bob.has_left);
    assert_eq!(bob.role.as_deref(), Some("member"));
    Ok(())
}

/// A `chat_updated` does not un-hide a direct chat with somebody this reader has BLOCKED. The
/// server sends a direct chat's frame with no regard to blocks — a held chat moving family when the
/// reader leaves one — while `GET /chats` and `member_blocked` keep it off the list, for the
/// blocker alone (docs/protocol.md, "Blocking a member"). The frame is still written, so the chat
/// comes back whole — in the families it said — on unblock.
#[test]
fn a_chat_updated_keeps_a_blocked_peers_chat_hidden() -> StoreResult<()> {
    let database = Database::open_in_memory()?;
    let shown = shown(3);
    let chats = chats_showing(&database, &shown);
    let listed_in = |id: i64, kind: &str, title: &str, peer: Option<i64>, families: &[i64]| {
        ChatRowDto::new(ChatDto {
            peer_user_id: peer,
            family_ids: Some(families.to_vec()),
            ..ChatDto::new(id, kind, title)
        })
    };
    chats.replace(
        &[
            listed_in(42, "family", "The Smiths", None, &[3]),
            listed_in(44, "direct", "Bob", Some(9), &[3, 5]),
            listed_in(45, "direct", "Carl", Some(11), &[3]),
        ],
        None,
    )?;
    let router = FrameRouter::new(chats.clone(), board_showing(&database, &shown), None);
    router.hear(&frame(
        r#"{"type": "member_blocked", "user_id": 9, "blocked": true}"#,
    ))?;
    assert!(!chats.is_listed(44)?);

    router.hear(&frame(
        r#"{"type": "chat_updated", "chat": {"id": 44, "kind": "direct", "title": "Bob", "peer_user_id": 9, "family_ids": [5]}}"#,
    ))?;
    assert!(!chats.is_listed(44)?);
    assert!(!chats.chats()?.iter().any(|row| row.chat.id == 44));
    assert_eq!(
        chats.chat(44)?.and_then(|row| row.chat.family_ids),
        Some(vec![5])
    );

    // A direct chat this device never held, with a blocked peer, arrives hidden too.
    router.hear(&frame(
        r#"{"type": "member_blocked", "user_id": 14, "blocked": true}"#,
    ))?;
    router.hear(&frame(
        r#"{"type": "chat_updated", "chat": {"id": 46, "kind": "direct", "title": "Eve", "peer_user_id": 14, "family_ids": [3]}}"#,
    ))?;
    assert!(chats.chat(46)?.is_some());
    assert!(!chats.is_listed(46)?);

    // Anybody else's chat, a group's included, is put on the list as before.
    router.hear(&frame(
        r#"{"type": "chat_updated", "chat": {"id": 45, "kind": "direct", "title": "Carl", "peer_user_id": 11, "family_ids": [3, 5]}}"#,
    ))?;
    assert!(chats.is_listed(45)?);
    router.hear(&frame(
        r#"{"type": "chat_updated", "chat": {"id": 47, "kind": "group", "title": "Cousins", "member_ids": [7, 9], "admin_user_id": 9, "family_ids": [3]}}"#,
    ))?;
    assert!(chats.is_listed(47)?);

    // Unblocked, it is back — with what the frame said.
    router.hear(&frame(
        r#"{"type": "member_blocked", "user_id": 9, "blocked": false}"#,
    ))?;
    assert!(chats.is_listed(44)?);
    assert_eq!(
        chats.chat(44)?.and_then(|row| row.chat.family_ids),
        Some(vec![5])
    );
    Ok(())
}

/// A family frame is applied to the family it NAMES (docs/protocol.md, "Frames"): a join, a leave
/// and an owner in family 5 leave family 3's roster alone, and a note and an item go onto family
/// 5's wall and pack whatever is shown.
#[test]
fn family_frames_go_to_the_family_they_name() -> StoreResult<()> {
    let database = Database::open_in_memory()?;
    let shown = shown(3);
    let chats = chats_showing(&database, &shown);
    let board = board_showing(&database, &shown);
    let pack = pack_showing(&database, &shown);
    chats.replace_members_of(
        Some(3),
        &[
            person(7, "anna", "Anna", "owner"),
            person(9, "bob", "Bob", "member"),
        ],
        &[],
    )?;
    chats.replace_members_of(
        Some(5),
        &[
            person(7, "anna", "Anna", "member"),
            person(11, "carl", "Carl", "owner"),
        ],
        &[],
    )?;
    let router = FrameRouter::new(chats.clone(), board.clone(), Some(pack.clone()));

    router.hear(&frame(
        r#"{"type": "member_joined", "family_id": 5, "user": {"id": 12, "username": "dana", "display_name": "Dana"}}"#,
    ))?;
    router.hear(&frame(
        r#"{"type": "member_left", "family_id": 5, "user_id": 11}"#,
    ))?;
    router.hear(&frame(
        r#"{"type": "family_owner", "family_id": 5, "user_id": 7}"#,
    ))?;
    router.hear(&frame(
        r#"{"type": "board_note", "note": {"id": 40, "author_id": 11, "text": "Milk", "board_seq": 6, "content_seq": 6, "family_id": 5}}"#,
    ))?;
    router.hear(&frame(
        r#"{"type": "pack_item", "item": {"id": 50, "added_by": 11, "pack_seq": 3, "family_id": 5, "attachment": {"id": 70, "kind": "photo"}}}"#,
    ))?;

    assert_eq!(ids(&chats.members()?, |member| member.id), [7, 9]);
    assert!(find(&chats.members()?, 7).owner());
    let other = chats.members_of(5)?;
    assert!(
        other
            .iter()
            .any(|member| member.id == 12 && !member.is_former())
    );
    assert!(find(&other, 11).has_left);
    assert!(find(&other, 7).owner());
    assert!(board.notes()?.is_empty());
    assert_eq!(one(board.in_family(Some(5)).notes()?).id, 40);
    assert!(pack.items()?.is_empty());
    assert_eq!(one(pack.in_family(Some(5)).items()?).id, 50);
    // Shown, the other family is the one drawn.
    shown.set(Some(5));
    assert_eq!(one(board.notes()?).id, 40);
    assert!(chats.members()?.iter().any(|member| member.id == 12));
    Ok(())
}

/// A TOMBSTONE names its family too (2026-10-22): a note or item taken down in family 5 leaves
/// family 5's wall and pack, and moves THAT family's cursor — never the shown family's.
#[test]
fn tombstones_go_to_the_family_they_name() -> StoreResult<()> {
    let database = Database::open_in_memory()?;
    let shown = shown(3);
    let board = board_showing(&database, &shown);
    let pack = pack_showing(&database, &shown);
    let note = |id: i64, author: i64, text: &str, seq: i64| NoteDto {
        kind: Some("text".to_string()),
        text: Some(text.to_string()),
        board_seq: seq,
        ..NoteDto::new(id, author)
    };
    board.replace(&[note(1, 9, "Milk", 4)], 4)?;
    board
        .in_family(Some(5))
        .replace(&[note(2, 11, "Bread", 9)], 9)?;
    pack.in_family(Some(5)).replace(
        &[PackItemDto {
            added_by: 11,
            attachment: Some(AttachmentDto::new(70, "photo")),
            pack_seq: 2,
            ..PackItemDto::new(20)
        }],
        2,
    )?;
    pack.reconnected();
    pack.in_family(Some(5)).caught_up(pack.connection());
    let router = FrameRouter::new(
        chats_showing(&database, &shown),
        board.clone(),
        Some(pack.clone()),
    );

    router.hear(&frame(
        r#"{"type": "board_note", "note": {"id": 2, "deleted": true, "board_seq": 10, "family_id": 5}}"#,
    ))?;
    router.hear(&frame(
        r#"{"type": "pack_item", "item": {"id": 20, "deleted": true, "pack_seq": 3, "family_id": 5}}"#,
    ))?;

    assert!(board.in_family(Some(5)).notes()?.is_empty());
    assert_eq!(board.in_family(Some(5)).cursor()?, 10);
    assert_eq!(board.cursor()?, 4);
    assert_eq!(board.notes()?.len(), 1);
    assert!(pack.in_family(Some(5)).items()?.is_empty());
    assert_eq!(pack.in_family(Some(5)).cursor()?, 3);
    assert_eq!(pack.cursor()?, 0);
    Ok(())
}

/// One naming the READER is a different fact — in a family, or out of one — and is the session's
/// to act on; a frame about somebody else raises neither.
#[test]
fn frames_naming_the_reader_say_they_are_in_or_out_of_a_family() -> StoreResult<()> {
    let database = Database::open_in_memory()?;
    let shown = shown(3);
    let router = FrameRouter::new(
        chats_showing(&database, &shown),
        board_showing(&database, &shown),
        None,
    );
    let joined = Rc::new(RefCell::new(Vec::new()));
    let left = Rc::new(RefCell::new(Vec::new()));
    let invited = Rc::new(RefCell::new(Vec::new()));
    let lapsed = Rc::new(RefCell::new(Vec::new()));
    let (j, l, i, g) = (
        joined.clone(),
        left.clone(),
        invited.clone(),
        lapsed.clone(),
    );
    router
        .on_joined_family()
        .subscribe(move |family: &i64| j.borrow_mut().push(*family));
    router
        .on_left_family()
        .subscribe(move |family: &i64| l.borrow_mut().push(*family));
    router
        .on_invitation_changed()
        .subscribe(move |invitation: &InvitationDto| i.borrow_mut().push(invitation.id));
    router
        .on_invitation_removed()
        .subscribe(move |invitation: &i64| g.borrow_mut().push(*invitation));

    router.hear(&frame(
        r#"{"type": "member_joined", "family_id": 5, "user": {"id": 9, "username": "bob", "display_name": "Bob"}}"#,
    ))?;
    router.hear(&frame(
        r#"{"type": "member_left", "family_id": 5, "user_id": 9}"#,
    ))?;
    router.hear(&frame(
        r#"{"type": "member_joined", "family_id": 8, "user": {"id": 7, "username": "anna", "display_name": "Anna"}}"#,
    ))?;
    router.hear(&frame(
        r#"{"type": "member_left", "family_id": 5, "user_id": 7}"#,
    ))?;
    router.hear(&frame(
        r#"{"type": "invitation", "invitation": {"id": 4, "family_id": 9, "family_name": "Sidorovs", "invited_by": {"id": 11, "username": "carl", "display_name": "Carl"}, "user": {"id": 7, "username": "anna", "display_name": "Anna"}}}"#,
    ))?;
    router.hear(&frame(
        r#"{"type": "invitation_removed", "invitation_id": 4}"#,
    ))?;

    assert_eq!(*joined.borrow(), [8]);
    assert_eq!(*left.borrow(), [5]);
    assert_eq!(*invited.borrow(), [4]);
    assert_eq!(*lapsed.borrow(), [4]);
    Ok(())
}

// ---- the resync -----------------------------------------------------------------------------------

/// The C# tests' routing `Server`: answers by path — the whole path, query aside — and keeps every
/// request; a path nobody routed never arrives.
fn client(server: &FakeServer, clock: &FakeClock) -> Rc<ApiClient> {
    Rc::new(ApiClient::new(
        Rc::new(server.clone()),
        ServerUrl::normalise(Some("chat.example.com")).expect("a server"),
        Rc::new(MemoryTokenStore::new(Some("t0ken"))),
        Rc::new(clock.clone()),
    ))
}

/// Step 1 is `GET /me` and then EACH family's own document, board and pack, by id, each against its
/// own cursor — and then the chats, which arrive for every family at once (docs/protocol.md,
/// "Best-effort delivery", step 1).
#[test]
fn a_resync_reads_every_family_into_its_own_rows() -> StoreResult<()> {
    let database = Database::open_in_memory()?;
    let server = FakeServer::new();
    server
        .always("/me", r#"
            {"user": {"id": 7, "username": "anna", "display_name": "Anna"}, "blocked_user_ids": [],
             "family": {"id": 3, "name": "The Smiths"}, "role": "member",
             "families": [{"id": 3, "name": "The Smiths", "role": "member"}, {"id": 5, "name": "Ivanovs", "role": "owner"}],
             "pending_join_requests": [], "invitations": [], "member_families_enabled": true, "max_families_per_account": 5}
            "#)
        .always("/families/3", r#"{"family": {"id": 3, "name": "The Smiths"}, "members": [{"id": 7, "username": "anna", "display_name": "Anna", "role": "member"}, {"id": 9, "username": "bob", "display_name": "Bob", "role": "owner"}], "max_board_seq": 4, "max_pack_items": 200, "max_pack_item_bytes": 524288, "max_pack_seq": 2, "assistant": {"user_id": 99, "display_name": "Assistant", "mention": "ai"}}"#)
        .always("/families/5", r#"{"family": {"id": 5, "name": "Ivanovs"}, "members": [{"id": 7, "username": "anna", "display_name": "Anna", "role": "owner"}, {"id": 11, "username": "carl", "display_name": "Carl", "role": "member", "password_reset": false}], "max_board_seq": 9, "max_pack_items": 200, "max_pack_item_bytes": 524288}"#)
        .always("/families/3/board", r#"{"notes": [{"id": 1, "author_id": 9, "text": "Milk", "board_seq": 4, "family_id": 3}], "max_board_seq": 4}"#)
        .always("/families/5/board", r#"{"notes": [{"id": 2, "author_id": 11, "text": "Bread", "board_seq": 9, "family_id": 5}], "max_board_seq": 9}"#)
        .always("/families/3/pack", r#"{"items": [{"id": 20, "added_by": 9, "pack_seq": 2, "family_id": 3, "attachment": {"id": 70, "kind": "photo"}}], "max_pack_seq": 2}"#)
        .always("/chats", r#"{"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths", "family_ids": [3]}}, {"chat": {"id": 43, "kind": "family", "title": "Ivanovs", "family_ids": [5]}}, {"chat": {"id": 44, "kind": "direct", "title": "Bob", "peer_user_id": 9, "family_ids": [3, 5]}}]}"#);
    let shown = shown(5);
    let chats = chats_showing(&database, &shown);
    let board = board_showing(&database, &shown);
    let pack = pack_showing(&database, &shown);
    let clock = FakeClock::at(datetime!(2026-10-28 12:00 UTC));
    let api = client(&server, &clock);
    api.set_family_scope(shown.get());

    let report = drive(
        &clock,
        Resync::new(api, chats.clone(), board.clone())
            .with_pack(pack.clone())
            .run(),
    )?;

    assert!(
        report.complete(),
        "{:?}",
        report.stopped.as_ref().map(|error| &error.code)
    );
    assert_eq!(report.families_read, Some(vec![3, 5]));
    assert_eq!(report.shown_family, Some(5));
    // The assistant is the SHOWN family's document's; family 5's names none.
    assert_eq!(report.assistant, None);
    assert_eq!(report.me.expect("who I am").all_families()[1].id, 5);
    let asked = server.asked();
    assert!(asked.iter().any(|path| path == "/api/v1/families/3/board"));
    assert!(asked.iter().any(|path| path == "/api/v1/families/5/board"));
    // Family 5's pack is empty and untouched: nothing is asked of it.
    assert!(!asked.iter().any(|path| path == "/api/v1/families/5/pack"));
    assert_eq!(one(board.notes()?).id, 2);
    assert_eq!(one(board.in_family(Some(3)).notes()?).id, 1);
    assert_eq!(board.cursor()?, 9);
    assert_eq!(board.in_family(Some(3)).cursor()?, 4);
    assert_eq!(one(pack.in_family(Some(3)).items()?).id, 20);
    assert!(pack.items()?.is_empty());
    assert_eq!(ids(&chats.members()?, |member| member.id), [7, 11]);
    assert_eq!(find(&chats.members()?, 11).password_reset, Some(false));
    assert_eq!(ids(&chats.members_of(3)?, |member| member.id), [7, 9]);
    assert_eq!(
        chats.chat(44)?.and_then(|row| row.chat.family_ids),
        Some(vec![3, 5])
    );
    Ok(())
}

/// A family gone between `GET /me` and its own read is skipped, not a reason to stop: `/me`
/// reconciles next time.
#[test]
fn a_family_gone_mid_pass_is_skipped() -> StoreResult<()> {
    let database = Database::open_in_memory()?;
    let server = FakeServer::new();
    server
        .always("/me", r#"{"user": {"id": 7, "username": "anna", "display_name": "Anna"}, "families": [{"id": 3, "name": "The Smiths", "role": "member"}, {"id": 5, "name": "Ivanovs", "role": "member"}]}"#)
        .always("/families/3", r#"{"family": {"id": 3, "name": "The Smiths"}, "members": []}"#)
        .always_with(
            "/families/5",
            r#"{"error": {"code": "family_not_found", "message": "no such family"}}"#,
            404,
        )
        .always("/chats", r#"{"chats": []}"#);
    let shown = shown(3);
    let clock = FakeClock::at(datetime!(2026-10-28 12:00 UTC));
    let report = drive(
        &clock,
        Resync::new(
            client(&server, &clock),
            chats_showing(&database, &shown),
            board_showing(&database, &shown),
        )
        .run(),
    )?;
    assert!(report.complete());
    assert_eq!(report.families_read, Some(vec![3]));
    Ok(())
}
