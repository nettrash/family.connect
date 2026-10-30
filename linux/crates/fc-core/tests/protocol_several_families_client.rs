//! One account in several families, as the API client asks it (docs/protocol.md, "Several
//! families", issue #88): the capability on every request and on the socket, the family named in
//! the path, the upload's and the new group's family, and the invitation endpoints — the seven
//! `ApiClient` tests of `win/tests/FamilyConnect.Core.Tests/Protocol/SeveralFamiliesProtocolTests.cs`.
//! (Its model tests are in `protocol_several_families_model.rs`; its router and resync tests in
//! `protocol_several_families_sync.rs`.)

use std::future::Future;
use std::rc::Rc;

use fc_core::protocol::{
    ApiClient, FAMILIES_HEADER, MemoryTokenStore, NoteRequest, ServerUrl, UploadFacts,
};
use fc_core::rt::Method;
use fc_core::testing::{FakeClock, FakeHttp, drive};
use time::macros::datetime;

/// The C# tests' routing `Server`: answers by path — the whole path, query aside, any method —
/// and keeps every request.
struct Server {
    http: FakeHttp,
    clock: FakeClock,
    api: ApiClient,
}

impl Server {
    fn new() -> Self {
        let http = FakeHttp::new();
        let clock = FakeClock::at(datetime!(2026-10-28 12:00 UTC));
        let api = ApiClient::new(
            Rc::new(http.clone()),
            ServerUrl::normalise(Some("chat.example.com")).expect("a server"),
            Rc::new(MemoryTokenStore::new(Some("t0ken"))),
            Rc::new(clock.clone()),
        );
        Server { http, clock, api }
    }

    fn on(&self, what: &str, json: &str) -> &Self {
        self.on_with(what, json, 200)
    }

    fn on_with(&self, what: &str, json: &str, status: u16) -> &Self {
        self.http.serve(&format!("/api/v1{what}"), status, json);
        self
    }

    fn run<T>(&self, future: impl Future<Output = T>) -> T {
        drive(&self.clock, future)
    }

    /// Each request's body as text, `None` where it had none.
    fn bodies(&self) -> Vec<Option<String>> {
        self.http
            .sent()
            .iter()
            .map(|request| {
                request
                    .body
                    .as_ref()
                    .map(|body| String::from_utf8_lossy(body).into_owned())
            })
            .collect()
    }
}

/// EVERY request says it knows about several families: without it the server answers about the
/// home family alone.
#[test]
fn every_request_carries_the_families_header() {
    let server = Server::new();
    server
        .on(
            "/me",
            r#"{"user": {"id": 7, "username": "anna", "display_name": "Anna"}}"#,
        )
        .on("/chats", r#"{"chats": []}"#);
    assert!(server.run(server.api.me()).ok());
    assert!(server.run(server.api.chats()).ok());
    let sent = server.http.sent();
    assert_eq!(sent.len(), 2);
    for request in &sent {
        assert_eq!(request.header_values(FAMILIES_HEADER), ["1"]);
    }
    assert_eq!(FAMILIES_HEADER, "X-FC-Families");
    // And on the socket, as a query parameter — a browser cannot set headers there.
    assert_eq!(
        server
            .api
            .socket_url()
            .split_once('?')
            .map(|(_, query)| query),
        Some("families=1")
    );
}

// ---- the family in the path ------------------------------------------------------------------

/// With no family shown every family endpoint keeps its old spelling — `mine`, and the forms
/// without an id — so an account in one family on a server from before asks exactly what it always
/// asked.
#[test]
fn with_no_family_shown_the_old_paths_are_asked_unchanged() {
    let server = Server::new();
    server
        .on(
            "/families/mine",
            r#"{"family": {"id": 3, "name": "The Smiths"}}"#,
        )
        .on(
            "/families/mine/board",
            r#"{"notes": [], "max_board_seq": 0}"#,
        )
        .on("/families/join-requests", r#"{"requests": []}"#)
        .on_with("/families/leave", "", 204);
    let api = &server.api;
    assert!(server.run(api.family(None)).ok());
    assert!(server.run(api.board(None)).ok());
    assert!(server.run(api.join_requests()).ok());
    assert!(server.run(api.leave_family(None)).ok());
    assert_eq!(
        server.http.asked(),
        [
            "/api/v1/families/mine",
            "/api/v1/families/mine/board",
            "/api/v1/families/join-requests",
            "/api/v1/families/leave"
        ]
    );
}

/// The family SHOWN is named on every family endpoint (docs/protocol.md, "Naming a family") — the
/// roster, the board, the pack, the owner's settings and lists, leaving, a member's removal,
/// birthday and password, reports and statistics. A block keeps the form without a family: it is
/// a pair, and holds in every family the two share.
#[test]
fn the_family_shown_is_named_on_every_family_endpoint() {
    let server = Server::new();
    server
        .on("/families/5", r#"{"family": {"id": 5, "name": "Ivanovs"}}"#)
        .on("/families/5/board", r#"{"notes": [], "max_board_seq": 0}"#)
        .on(
            "/families/5/board/notes",
            r#"{"note": {"id": 1, "author_id": 7}}"#,
        )
        .on("/families/5/pack", r#"{"items": [], "max_pack_seq": 0}"#)
        .on("/families/5/stats", r#"{"totals": {"members": 1}}"#)
        .on(
            "/families/5/invite-code/rotate",
            r#"{"invite_code": "ABCD2345"}"#,
        )
        .on("/families/5/join-requests", r#"{"requests": []}"#)
        .on("/families/5/reports", r#"{"reports": []}"#)
        .on_with("/families/5/members/9", "", 204)
        .on_with("/families/5/members/9/password", "", 204)
        .on_with("/families/5/leave", "", 204)
        .on_with("/families/members/9/block", "", 204);
    let api = &server.api;
    api.set_family_scope(Some(5));
    assert_eq!(api.family_scope(), Some(5));
    assert!(server.run(api.family(None)).ok());
    assert!(server.run(api.board(None)).ok());
    assert!(
        server
            .run(api.create_note(&NoteRequest::new("Milk", "yellow", 0.5, 0.5)))
            .ok()
    );
    assert!(server.run(api.pack(None)).ok());
    assert!(server.run(api.stats()).ok());
    assert!(server.run(api.rotate_invite_code()).ok());
    assert!(server.run(api.join_requests()).ok());
    assert!(server.run(api.reports()).ok());
    assert!(server.run(api.remove_member(9)).ok());
    assert!(
        server
            .run(api.reset_member_password(9, "correct horse"))
            .ok()
    );
    assert!(server.run(api.leave_family(None)).ok());
    assert!(server.run(api.block(9)).ok());
    assert_eq!(
        server.http.asked(),
        [
            "/api/v1/families/5",
            "/api/v1/families/5/board",
            "/api/v1/families/5/board/notes",
            "/api/v1/families/5/pack",
            "/api/v1/families/5/stats",
            "/api/v1/families/5/invite-code/rotate",
            "/api/v1/families/5/join-requests",
            "/api/v1/families/5/reports",
            "/api/v1/families/5/members/9",
            "/api/v1/families/5/members/9/password",
            "/api/v1/families/5/leave",
            "/api/v1/families/members/9/block"
        ]
    );
    // Cleared, the home family is `mine` again.
    api.set_family_scope(None);
    assert_eq!(api.family_scope(), None);
}

/// A resync names each family itself, whichever is shown: one family's catch-up is never
/// another's.
#[test]
fn a_family_named_outright_is_asked_whatever_is_shown() {
    let server = Server::new();
    server
        .on(
            "/families/3",
            r#"{"family": {"id": 3, "name": "The Smiths"}}"#,
        )
        .on("/families/3/board", r#"{"notes": [], "max_board_seq": 0}"#)
        .on("/families/3/board/changes", r#"{"notes": []}"#)
        .on("/families/3/pack", r#"{"items": [], "max_pack_seq": 0}"#)
        .on("/families/3/pack/changes", r#"{"items": []}"#)
        .on_with("/families/3/leave", "", 204);
    let api = &server.api;
    api.set_family_scope(Some(5));
    assert!(server.run(api.family(Some(3))).ok());
    assert!(server.run(api.board(Some(3))).ok());
    assert!(server.run(api.board_changes(Some(3), 12, 50)).ok());
    assert!(server.run(api.pack(Some(3))).ok());
    assert!(server.run(api.pack_changes(Some(3), 4, 50)).ok());
    assert!(server.run(api.leave_family(Some(3))).ok());
    assert_eq!(
        server.http.asked(),
        [
            "/api/v1/families/3",
            "/api/v1/families/3/board",
            "/api/v1/families/3/board/changes?after_seq=12&limit=50",
            "/api/v1/families/3/pack",
            "/api/v1/families/3/pack/changes?after_seq=4&limit=50",
            "/api/v1/families/3/leave"
        ]
    );
}

/// An upload names the family it is FOR — the one shown, or the one asked — and a new group is
/// made in the family shown; with none shown neither says anything, which is the home family.
#[test]
fn uploads_and_new_groups_name_their_family() {
    let server = Server::new();
    server
        .on(
            "/attachments",
            r#"{"attachment": {"id": 34, "kind": "photo"}}"#,
        )
        .on(
            "/chats/groups",
            r#"{"chat": {"id": 77, "kind": "group", "title": "Cousins", "family_ids": [5]}}"#,
        );
    let api = &server.api;
    let none = UploadFacts::default();
    assert!(
        server
            .run(api.upload("photo", "image/jpeg", vec![1, 2, 3], &none))
            .ok()
    );
    assert!(server.run(api.create_group("Cousins", &[9])).ok());
    api.set_family_scope(Some(5));
    assert!(
        server
            .run(api.upload("photo", "image/jpeg", vec![1, 2, 3], &none))
            .ok()
    );
    let for_three = UploadFacts {
        family_id: Some(3),
        ..UploadFacts::default()
    };
    assert!(
        server
            .run(api.upload("photo", "image/jpeg", vec![1, 2, 3], &for_three))
            .ok()
    );
    assert!(
        server
            .run(api.upload_location(55.75, 37.61, Some(12.0), None, Some(3)))
            .ok()
    );
    let group = server.run(api.create_group("Cousins", &[9]));

    let asked = server.http.asked();
    let bodies = server.bodies();
    assert!(!asked[0].contains("family_id"));
    assert!(!bodies[1].as_deref().expect("a body").contains("family_id"));
    assert!(asked[2].ends_with("&family_id=5"));
    assert!(asked[3].ends_with("&family_id=3"));
    assert!(asked[4].ends_with("&family_id=3"));
    assert!(
        bodies[5]
            .as_deref()
            .expect("a body")
            .contains(r#""family_id":5"#)
    );
    assert_eq!(
        group.value().expect("a group").chat.family_ids,
        Some(vec![5])
    );
}

/// The five invitation endpoints, as the table spells them.
#[test]
fn invitations_are_asked_where_the_table_says() {
    const INVITATION: &str = r#"{"id": 4, "family_id": 5, "family_name": "Ivanovs", "invited_by": {"id": 7, "username": "anna", "display_name": "Anna"}, "user": {"id": 9, "username": "bob", "display_name": "Bob"}, "created_at": "2026-10-22T10:00:00Z"}"#;
    let server = Server::new();
    server
        .on_with(
            "/families/5/invitations",
            &format!(r#"{{"invitation": {INVITATION}}}"#),
            201,
        )
        .on_with("/families/5/invitations/4", "", 204)
        .on(
            "/invitations/4/accept",
            r#"{"family": {"id": 5, "name": "Ivanovs"}}"#,
        )
        .on_with("/invitations/4/decline", "", 204);
    let api = &server.api;

    let invited = server.run(api.invite(5, 9));
    assert!(invited.ok());
    let invitation = &invited.value().expect("an invitation").invitation;
    assert_eq!(invitation.family_name, "Ivanovs");
    assert_eq!(invitation.invited_by.display_name, "Anna");
    assert_eq!(invitation.user.id, 9);
    assert_eq!(server.bodies()[0].as_deref(), Some(r#"{"user_id":9}"#));
    assert!(server.run(api.withdraw_invitation(5, 4)).ok());
    let accepted = server.run(api.accept_invitation(4));
    assert_eq!(accepted.value().expect("a family").family.id, 5);
    assert!(server.run(api.decline_invitation(4)).ok());
    let methods: Vec<Method> = server
        .http
        .sent()
        .iter()
        .map(|request| request.method)
        .collect();
    assert_eq!(
        methods,
        [Method::Post, Method::Delete, Method::Post, Method::Post]
    );
    assert_eq!(
        server.http.asked(),
        [
            "/api/v1/families/5/invitations",
            "/api/v1/families/5/invitations/4",
            "/api/v1/invitations/4/accept",
            "/api/v1/invitations/4/decline"
        ]
    );
}

#[test]
fn a_familys_pending_invitations_are_read_oldest_first() {
    let server = Server::new();
    server.on(
        "/families/5/invitations",
        r#"{"invitations": [{"id": 4, "family_id": 5, "family_name": "Ivanovs", "invited_by": {"id": 7, "username": "anna", "display_name": "Anna"}, "user": {"id": 9, "username": "bob", "display_name": "Bob"}}]}"#,
    );
    let answer = server.run(server.api.invitations(5));
    let invitations = answer
        .value()
        .and_then(|value| value.invitations.clone())
        .unwrap_or_default();
    assert_eq!(invitations.len(), 1);
    assert_eq!(invitations[0].id, 4);
}
