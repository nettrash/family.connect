//! One account in several families on the wire (docs/protocol.md, "Several families", issue #88)
//! — the MODEL half of `win/tests/FamilyConnect.Core.Tests/Protocol/SeveralFamiliesProtocolTests.cs`:
//! `GET /me`'s list, the objects' new fields and the frames. The tests that drive the API client,
//! the frame router, the stores or the resync are ported with them.

use fc_core::protocol::{ChatDto, MeResponse, MemberDto, NoteDto, PackItemDto, ServerFrame, wire};

// ---- GET /me --------------------------------------------------------------------------------

/// `GET /me`'s list, each family with its own role, home first; and the requests, invitations
/// and the two numbers.
#[test]
fn me_carries_every_family_with_its_role() {
    let me: MeResponse = wire::decode(
        r#"
        {"user": {"id": 7, "username": "anna", "display_name": "Anna"},
         "family": {"id": 3, "name": "The Smiths"}, "role": "member",
         "families": [{"id": 3, "name": "The Smiths", "role": "member"}, {"id": 5, "name": "Ivanovs", "role": "owner"}],
         "pending_join_requests": [{"family_id": 8, "family_name": "Petrovs", "created_at": "2026-10-22T10:00:00Z"}],
         "invitations": [{"id": 4, "family_id": 9, "family_name": "Sidorovs", "invited_by": {"id": 11, "username": "carl", "display_name": "Carl"}, "user": {"id": 7, "username": "anna", "display_name": "Anna"}}],
         "member_families_enabled": true, "max_families_per_account": 5}
        "#,
    )
    .expect("GET /me");
    let families = me.all_families();
    let ids: Vec<i64> = families.iter().map(|family| family.id).collect();
    assert_eq!(ids, [3, 5]);
    assert!(!families[0].is_owner());
    assert!(families[1].is_owner());
    assert_eq!(me.all_pending().len(), 1);
    assert_eq!(me.all_pending()[0].family_name, "Petrovs");
    let invitations = me.invitations.as_deref().expect("the invitations");
    assert_eq!(invitations.len(), 1);
    assert_eq!(invitations[0].invited_by.display_name, "Carl");
    assert!(me.member_families_enabled);
    assert_eq!(me.family_cap(), 5);
}

/// A server from before: its one family, with the role beside it, is the list; nothing else is
/// open.
#[test]
fn an_older_me_is_read_as_its_one_family() {
    let me: MeResponse = wire::decode(
        r#"
        {"user": {"id": 7, "username": "anna", "display_name": "Anna"},
         "family": {"id": 3, "name": "The Smiths"}, "role": "owner",
         "pending_join_request": {"family_id": 8, "family_name": "Petrovs"}}
        "#,
    )
    .expect("GET /me");
    let families = me.all_families();
    assert_eq!(families.len(), 1);
    assert_eq!(families[0].id, 3);
    assert!(families[0].is_owner());
    assert_eq!(me.families, None);
    assert_eq!(me.all_pending().len(), 1);
    assert_eq!(me.all_pending()[0].family_id, 8);
    assert!(!me.member_families_enabled);
    assert_eq!(me.family_cap(), 1);
    let alone: MeResponse =
        wire::decode(r#"{"user": {"id": 7, "username": "anna", "display_name": "Anna"}}"#)
            .expect("GET /me");
    assert!(alone.all_families().is_empty());
}

/// The objects' new fields: a chat's families, a note's and an item's family, a member's
/// password_reset.
#[test]
fn the_objects_say_whose_they_are() {
    let direct: ChatDto =
        wire::decode(r#"{"id": 42, "kind": "direct", "title": "Bob", "family_ids": [3, 5]}"#)
            .expect("a chat");
    assert_eq!(direct.family_ids.as_deref(), Some(&[3, 5][..]));
    let family: ChatDto =
        wire::decode(r#"{"id": 42, "kind": "family", "title": "The Smiths"}"#).expect("a chat");
    assert_eq!(family.family_ids, None);
    let note: NoteDto =
        wire::decode(r#"{"id": 1, "author_id": 7, "family_id": 5}"#).expect("a note");
    assert_eq!(note.family_id, Some(5));
    let item: PackItemDto =
        wire::decode(r#"{"id": 1, "pack_seq": 2, "family_id": 5}"#).expect("an item");
    assert_eq!(item.family_id, Some(5));
    let reset: MemberDto = wire::decode(
        r#"{"id": 9, "username": "bob", "display_name": "Bob", "password_reset": false}"#,
    )
    .expect("a member");
    assert_eq!(reset.password_reset, Some(false));
    let silent: MemberDto =
        wire::decode(r#"{"id": 9, "username": "bob", "display_name": "Bob"}"#).expect("a member");
    assert_eq!(silent.password_reset, None);
}

// ---- frames ---------------------------------------------------------------------------------

#[test]
fn the_two_invitation_frames_parse() {
    let invitation = ServerFrame::parse(
        r#"{"type": "invitation", "invitation": {"id": 4, "family_id": 5, "family_name": "Ivanovs", "invited_by": {"id": 7, "username": "anna", "display_name": "Anna"}, "user": {"id": 9, "username": "bob", "display_name": "Bob"}}}"#,
    );
    let Some(ServerFrame::Invitation(value)) = invitation else {
        panic!("expected an invitation, got {invitation:?}");
    };
    assert_eq!(value.id, 4);
    assert_eq!(value.family_id, 5);
    assert_eq!(
        ServerFrame::parse(r#"{"type": "invitation_removed", "invitation_id": 4}"#),
        Some(ServerFrame::InvitationRemoved { invitation_id: 4 })
    );
    // Nothing to apply is nothing at all.
    assert_eq!(
        ServerFrame::parse(r#"{"type": "invitation_removed"}"#),
        None
    );
    assert_eq!(ServerFrame::parse(r#"{"type": "invitation"}"#), None);
}

/// The OWNER's copy of `member_joined` says whether the newcomer is theirs to reset; everybody
/// else's says nothing, and nothing is not "no" — nor is a value that is not a boolean, which a
/// guess would turn into one answer or the other.
#[test]
fn a_join_says_whether_the_newcomer_is_the_owners_to_reset() {
    const USER: &str = r#"{"id": 12, "username": "dana", "display_name": "Dana"}"#;
    let reset = |tail: &str| match ServerFrame::parse(&format!(
        r#"{{"type": "member_joined", "family_id": 5, "user": {USER}{tail}}}"#
    )) {
        Some(ServerFrame::MemberJoined { password_reset, .. }) => password_reset,
        other => panic!("expected a join, got {other:?}"),
    };
    assert_eq!(reset(r#", "password_reset": false"#), Some(false));
    assert_eq!(reset(r#", "password_reset": true"#), Some(true));
    assert_eq!(reset(""), None);
    assert_eq!(reset(r#", "password_reset": "no""#), None);
}
