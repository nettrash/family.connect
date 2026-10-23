//
//  SeveralFamiliesTests.swift
//  FamilyConnectTests
//
//  Several families (docs/protocol.md, "Several families", 2026-10-22; the
//  decisions are docs/families-2026-10-22.md): the rules table, the wire —
//  the header, the query, the `/families/{id}/…` paths, the invitations and
//  the new fields — the session's family list and its current family, and
//  the store: a resync that reads every family, a switch that re-projects
//  the roster, and a `member_left` naming this account that drops ONE
//  family's chats, board, pack and roster and nobody else's.
//
//  The server is being written at the same time from the same protocol, so
//  everything here runs against fixtures written from the protocol's text.
//

import Foundation
import SwiftData
import Testing
@testable import FamilyConnect

// MARK: - Fixtures

private enum Fixture {
    static func family(_ id: Int64, _ name: String, role: String? = nil) -> FamilyDTO {
        FamilyDTO(
            id: id, name: name, joinPolicy: "open", createdAt: nil, inviteCode: nil,
            aiVision: false, aiHistoryPhotos: false, aiGreeting: false, aiFaces: false,
            aiTranscripts: false, aiLookups: false, greetingPlaces: [], maxMembers: nil, role: role)
    }

    static let user = UserDTO(id: 7, username: "anna", displayName: "Anna", createdAt: nil)
    static let smiths = family(3, "The Smiths", role: "owner")
    static let ivanovs = family(5, "The Ivanovs", role: "member")

    static func me(
        families: [FamilyDTO]?,
        pending: [PendingJoinRequestDTO] = [],
        invitations: [InvitationDTO] = [],
        memberFamiliesEnabled: Bool = true,
        max: Int = 5
    ) -> MeResponse {
        MeResponse(
            user: user, family: families?.first, role: families?.first?.role,
            pendingJoinRequest: pending.first,
            families: families, pendingJoinRequests: pending, invitations: invitations,
            memberFamiliesEnabled: memberFamiliesEnabled, maxFamiliesPerAccount: max)
    }

    static func invitation(_ id: Int64, family: Int64 = 5, name: String = "The Ivanovs") -> InvitationDTO {
        InvitationDTO(
            id: id, familyID: family, familyName: name,
            invitedBy: UserDTO(id: 11, username: "olga", displayName: "Olga", createdAt: nil),
            user: user, createdAt: nil)
    }

    static func chat(_ familyIDs: [Int64], unread: Int) -> FamilyRules.ChatCount {
        FamilyRules.ChatCount(familyIDs: familyIDs, unread: unread)
    }
}

// MARK: - The rules

@Suite("Several families: the rules")
struct FamilyRulesTests {

    @Test("the family list: `families` from a new server, the one `family` from an old one")
    func familiesFromMe() {
        let new = Fixture.me(families: [Fixture.smiths, Fixture.ivanovs])
        #expect(FamilyRules.families(from: new).map(\.id) == [3, 5])
        #expect(!FamilyRules.isLegacy(new))

        // An older server: no `families` key at all, one family beside its role.
        let old = MeResponse(
            user: Fixture.user, family: Fixture.family(3, "The Smiths"), role: "owner", pendingJoinRequest: nil)
        #expect(FamilyRules.isLegacy(old))
        #expect(FamilyRules.families(from: old).map(\.id) == [3])
        #expect(FamilyRules.families(from: old).first?.role == "owner")

        // An account in none, on either kind of server.
        #expect(FamilyRules.families(from: Fixture.me(families: [])).isEmpty)
        #expect(FamilyRules.families(from: MeResponse(
            user: Fixture.user, family: nil, role: nil, pendingJoinRequest: nil)).isEmpty)
    }

    @Test("pending requests: the list, or an old server's one")
    func pendingRequests() {
        let request = PendingJoinRequestDTO(familyID: 9, familyName: "The Lees", createdAt: nil)
        #expect(FamilyRules.pendingRequests(from: Fixture.me(families: [Fixture.smiths], pending: [request])) == [request])
        let old = MeResponse(user: Fixture.user, family: nil, role: nil, pendingJoinRequest: request)
        #expect(FamilyRules.pendingRequests(from: old) == [request])
    }

    @Test("the current family: the stored one while held, else the first")
    func currentFamily() {
        let both = [Fixture.smiths, Fixture.ivanovs]
        #expect(FamilyRules.currentFamilyID(stored: 5, families: both) == 5)
        #expect(FamilyRules.currentFamilyID(stored: nil, families: both) == 3)
        // Left, removed, deleted: back to the first.
        #expect(FamilyRules.currentFamilyID(stored: 8, families: both) == 3)
        #expect(FamilyRules.currentFamilyID(stored: 5, families: []) == nil)
    }

    @Test("a path names the family, except on a server that knows only `mine`")
    func pathFamily() {
        #expect(FamilyRules.pathFamilyID(5, legacy: false) == 5)
        #expect(FamilyRules.pathFamilyID(5, legacy: true) == nil)
        #expect(FamilyRules.pathFamilyID(nil, legacy: false) == nil)
    }

    @Test("New Family…: the server's switch, owning none, and under the cap", arguments: [
        // (enabled, roles, max, offered)
        (true, ["member"], 5, true),
        (false, ["member"], 5, false),
        (true, ["owner"], 5, false),
        (true, ["member", "owner"], 5, false),
        (true, ["member", "member"], 2, false),
        (true, [String](), 5, true),
    ])
    func offersNewFamily(enabled: Bool, roles: [String], max: Int, offered: Bool) {
        let families = roles.enumerated().map { Fixture.family(Int64($0.offset + 1), "F", role: $0.element) }
        #expect(FamilyRules.offersNewFamily(
            memberFamiliesEnabled: enabled, families: families, maxFamilies: max) == offered)
    }

    @Test("Join Another Family…: while under the cap")
    func offersJoinAnother() {
        #expect(FamilyRules.offersJoinAnother(families: [Fixture.smiths], maxFamilies: 5))
        #expect(!FamilyRules.offersJoinAnother(families: [Fixture.smiths, Fixture.ivanovs], maxFamilies: 2))
        // An older server's cap of one: nothing to offer.
        #expect(!FamilyRules.offersJoinAnother(families: [Fixture.smiths], maxFamilies: 1))
    }

    @Test("the switcher: two families, or something pending — never for one family alone", arguments: [
        (1, 0, 0, false), (2, 0, 0, true), (1, 1, 0, true), (1, 0, 1, true), (3, 0, 0, true),
    ])
    func showsSwitcher(families: Int, pending: Int, invitations: Int, shown: Bool) {
        #expect(FamilyRules.showsSwitcher(
            familyCount: families, pendingRequests: pending, invitations: invitations) == shown)
    }

    @Test("a family lists the chats that name it")
    func isListed() {
        #expect(FamilyRules.isListed(chatFamilyIDs: [3], in: 3, home: 3))
        #expect(!FamilyRules.isListed(chatFamilyIDs: [5], in: 3, home: 3))
        // A direct chat is listed in every family the two people share.
        #expect(FamilyRules.isListed(chatFamilyIDs: [3, 5], in: 3, home: 3))
        #expect(FamilyRules.isListed(chatFamilyIDs: [3, 5], in: 5, home: 3))
        // With no family known yet, everything held is shown.
        #expect(FamilyRules.isListed(chatFamilyIDs: [5], in: nil, home: nil))
    }

    @Test("a chat naming no family (absent family_ids) is the HOME family's, and listed there alone")
    func absentFamilyIDsMeanTheHomeFamily() {
        // docs/protocol.md, `Chat.family_ids`: absent, "it means the one
        // family the account has" — the home family, first in `GET /me`.
        #expect(FamilyRules.isListed(chatFamilyIDs: nil, in: 3, home: 3))
        #expect(!FamilyRules.isListed(chatFamilyIDs: nil, in: 5, home: 3),
                "a chat cached before the upgrade was shown in every family")
        // An offline launch before the first `/me`: no family on screen, or
        // no list to say which one is home — shown, as it always was.
        #expect(FamilyRules.isListed(chatFamilyIDs: nil, in: nil, home: nil))
        #expect(FamilyRules.isListed(chatFamilyIDs: nil, in: 5, home: nil))
        // And it counts towards the home family's unread only.
        let chats = [Fixture.chat([5], unread: 4), FamilyRules.ChatCount(familyIDs: nil, unread: 2)]
        #expect(FamilyRules.unread(in: 3, chats: chats, home: 3) == 2)
        #expect(FamilyRules.unread(in: 5, chats: chats, home: 3) == 4)
        let entries = FamilyRules.switcherEntries(
            families: [Fixture.smiths, Fixture.ivanovs], chats: chats, current: 5)
        #expect(entries.map(\.unread) == [2, 4], "the switcher's home is the first family /me lists")
        // The session's home is that same first family.
        #expect(FamilyRules.families(from: Fixture.me(families: [Fixture.ivanovs, Fixture.smiths])).first?.id == 5)
    }

    @Test("a chat the server lists in NO family is listed nowhere, not everywhere")
    func listedInNoFamily() {
        // `family_ids: []` — a direct chat closed for the side that left —
        // is not an absent one: not even the home family lists it.
        #expect(!FamilyRules.isListed(chatFamilyIDs: [], in: 3, home: 3))
        #expect(!FamilyRules.isListed(chatFamilyIDs: [], in: 5, home: 3))
        #expect(!FamilyRules.isListed(chatFamilyIDs: [], in: nil, home: nil))
        // And it counts towards no family's unread.
        let chats = [Fixture.chat([3], unread: 2), FamilyRules.ChatCount(familyIDs: [], unread: 9)]
        #expect(FamilyRules.unread(in: 3, chats: chats, home: 3) == 2)
        #expect(FamilyRules.unread(in: 5, chats: chats, home: 3) == 0)
    }

    @Test("the store keeps a chat's absent family_ids and its empty family_ids apart")
    func listedEncoding() {
        #expect(FamilyIDList.encodeListed(nil) == "")
        #expect(FamilyIDList.encodeListed([]) == FamilyIDList.noFamily)
        #expect(FamilyIDList.encodeListed([3, 5]) == "3,5")
        #expect(FamilyIDList.decodeListed("") == nil)
        #expect(FamilyIDList.decodeListed(FamilyIDList.noFamily) == [])
        #expect(FamilyIDList.decodeListed("3,5") == [3, 5])
        let chat = ChatEntity(chatID: 44, kind: "direct", pinRank: 1, title: "Boris")
        #expect(chat.familyIDs == nil)
        chat.familyIDs = []
        #expect(chat.familyIDsText == FamilyIDList.noFamily)
        #expect(chat.familyIDs == [])
    }

    @Test("unread per family: a direct chat counts in EACH family that lists it")
    func unreadPerFamily() {
        let chats = [
            Fixture.chat([3], unread: 2),     // the Smiths' family chat
            Fixture.chat([5], unread: 4),     // the Ivanovs' family chat
            Fixture.chat([3, 5], unread: 1),  // a direct chat with somebody in both
        ]
        #expect(FamilyRules.unread(in: 3, chats: chats, home: 3) == 3)
        #expect(FamilyRules.unread(in: 5, chats: chats, home: 3) == 5)
        let entries = FamilyRules.switcherEntries(
            families: [Fixture.smiths, Fixture.ivanovs], chats: chats, current: 5)
        #expect(entries.map(\.unread) == [3, 5])
        #expect(entries.map(\.isCurrent) == [false, true])
        #expect(entries.map(\.name) == ["The Smiths", "The Ivanovs"])
    }

    @Test("leaving: a direct chat stays while another held family lists it")
    func survivesLeaving() {
        #expect(FamilyRules.survivesLeaving(chatFamilyIDs: [3, 5], left: 5, held: [3]))
        #expect(!FamilyRules.survivesLeaving(chatFamilyIDs: [5], left: 5, held: [3]))
        // Listed in another family this device does NOT hold: gone for us.
        #expect(!FamilyRules.survivesLeaving(chatFamilyIDs: [5, 9], left: 5, held: [3]))
        #expect(FamilyRules.survivesLeaving(chatFamilyIDs: [3], left: 5, held: [3]))
        // Listed in no family (`[]`, sent just before the `member_left`):
        // gone, whichever family is being left.
        #expect(!FamilyRules.survivesLeaving(chatFamilyIDs: [], left: 5, held: [3]))
        // Nobody said (a row from before several families): not this family's to take.
        #expect(FamilyRules.survivesLeaving(chatFamilyIDs: nil, left: 5, held: [3]))
    }

    @Test("an upload is for the family on screen when the chat is listed there, else the chat's first")
    func uploadFamily() {
        #expect(FamilyRules.uploadFamily(chatFamilyIDs: [3, 5], current: 5) == 5)
        #expect(FamilyRules.uploadFamily(chatFamilyIDs: [3, 5], current: 9) == 3)
        #expect(FamilyRules.uploadFamily(chatFamilyIDs: [], current: 5) == 5)
        #expect(FamilyRules.uploadFamily(chatFamilyIDs: [], current: nil) == nil)
    }

    @Test("a push opens in its family; a direct chat's opens where it is listed")
    func pushFamily() {
        let held: [Int64] = [3, 5]
        #expect(FamilyRules.pushFamily(pushFamilyID: 5, chatFamilyIDs: [], current: 3, held: held) == 5)
        // Already there, or a family this device does not hold: stay.
        #expect(FamilyRules.pushFamily(pushFamilyID: 5, chatFamilyIDs: [], current: 5, held: held) == nil)
        #expect(FamilyRules.pushFamily(pushFamilyID: 8, chatFamilyIDs: [], current: 3, held: held) == nil)
        // No family on the push (a direct chat's).
        #expect(FamilyRules.pushFamily(pushFamilyID: nil, chatFamilyIDs: [3, 5], current: 5, held: held) == nil)
        #expect(FamilyRules.pushFamily(pushFamilyID: nil, chatFamilyIDs: [5], current: 3, held: held) == 5)
        #expect(FamilyRules.pushFamily(pushFamilyID: nil, chatFamilyIDs: [], current: 3, held: held) == nil)
    }

    @Test("a family chat's notification is titled with THAT chat's family")
    func familyName() {
        let families = [Fixture.smiths, Fixture.ivanovs]
        #expect(FamilyRules.familyName(chatFamilyIDs: [5], families: families, fallback: "Chat") == "The Ivanovs")
        #expect(FamilyRules.familyName(chatFamilyIDs: [], families: families, fallback: "Chat") == "Chat")
    }

    @Test("Invite…: members of the owner's OTHER families who are not in this one, and nobody else")
    func invitees() {
        let rows = [
            FamilyRules.RosterRow(userID: 7, name: "Anna", families: [3, 5], accountDeleted: false),   // me
            FamilyRules.RosterRow(userID: 9, name: "Kid", families: [3], accountDeleted: false),        // already here
            FamilyRules.RosterRow(userID: 11, name: "Olga", families: [5], accountDeleted: false),
            FamilyRules.RosterRow(userID: 13, name: "Boris", families: [5], accountDeleted: false),
            FamilyRules.RosterRow(userID: 15, name: "Gone", families: [5], accountDeleted: true),
            FamilyRules.RosterRow(userID: 17, name: "Nobody", families: [], accountDeleted: false),
        ]
        let invitees = FamilyRules.invitees(
            rows: rows, into: 3, currentUserID: 7, pendingInvitedUserIDs: [13])
        #expect(invitees.map(\.id) == [13, 11])  // sorted by name: Boris, Olga
        #expect(invitees.map(\.isInvited) == [true, false])
        #expect(FamilyRules.offersInvite(isOwner: true, familyCount: 2))
        #expect(!FamilyRules.offersInvite(isOwner: true, familyCount: 1))
        #expect(!FamilyRules.offersInvite(isOwner: false, familyCount: 2))
    }

    @Test("a password reset is offered only where the roster says it will work")
    func passwordReset() {
        #expect(FamilyRules.offersPasswordReset(true))
        #expect(!FamilyRules.offersPasswordReset(false))
        // A server from before several families never says, and never refuses.
        #expect(FamilyRules.offersPasswordReset(nil))
    }

    @Test("the leave dialog keeps today's title with one family and names the family with several")
    func leaveTitle() {
        #expect(FamilyRules.leaveTitle(familyName: "The Smiths", familyCount: 1) == nil)
        #expect(FamilyRules.leaveTitle(familyName: "The Smiths", familyCount: 2) == String(localized: "Leave “The Smiths”?"))
    }

    @Test("every new refusal has its sentence; other errors are left to the caller", arguments: [
        ("too_many_families", true), ("already_own_a_family", true), ("member_families_disabled", false),
        ("member_in_other_families", false), ("invitation_not_pending", true),
    ])
    func failureMessages(code: String, isConflict: Bool) {
        let error: APIError = isConflict ? .conflict(code: code, message: nil) : .forbidden(code: code)
        #expect(FamilyRules.failureMessage(error, maxFamilies: 5) != nil, "\(code)")
    }

    @Test("the cap is said with its number; an unknown code says nothing")
    func failureMessageDetails() {
        #expect(FamilyRules.failureMessage(APIError.conflict(code: "too_many_families", message: nil), maxFamilies: 5)
            == String(localized: "You've reached the most families one account can be in (\(5))."))
        #expect(FamilyRules.failureMessage(APIError.conflict(code: "family_full", message: nil), maxFamilies: 5) == nil)
        #expect(FamilyRules.failureMessage(APIError.decoding, maxFamilies: 5) == nil)
        #expect(FamilyRules.isFamilyGone(APIError.notFound(code: "family_not_found")))
        #expect(!FamilyRules.isFamilyGone(APIError.notFound(code: "chat_not_found")))
    }

    @Test("the store's id lists: a row in no family is not a row nobody described")
    func idLists() {
        #expect(FamilyIDList.encode([3, 5]) == "3,5")
        #expect(FamilyIDList.decode("3,5") == [3, 5])
        #expect(FamilyIDList.decode("") == [])
        #expect(FamilyIDList.encodeRoles([5: "member", 3: "owner"]) == "3=owner;5=member")
        #expect(FamilyIDList.decodeRoles("3=owner;5=member") == [3: "owner", 5: "member"])
        #expect(FamilyIDList.encodeRoles([:]) == FamilyIDList.noFamily)
        #expect(FamilyIDList.decodeRoles(FamilyIDList.noFamily).isEmpty)
    }

    @Test("a roster row projects onto the family on screen; an undescribed row is left alone")
    func memberProjection() {
        let row = MemberEntity(userID: 11, username: "olga", displayName: "Olga", role: "member", isCurrentUser: false)
        row.project(onto: 3)
        #expect(!row.hasLeft, "a row from before several families was rewritten")
        row.familyRoles = [3: "member", 5: "owner"]
        row.project(onto: 5)
        #expect(row.role == "owner")
        #expect(!row.hasLeft)
        row.familyRoles = [5: "owner"]
        row.project(onto: 3)
        #expect(row.hasLeft)
        row.familyRoles = [:]
        row.project(onto: 5)
        #expect(row.hasLeft)
        row.passwordResetDeniedText = "5"
        #expect(!row.passwordResetAllowed(in: 5))
        #expect(row.passwordResetAllowed(in: 3))
    }
}

// MARK: - The wire

@Suite("Several families: the wire")
struct SeveralFamiliesWireTests {

    private func client(_ host: String, handler: @escaping StubURLProtocol.Handler) -> APIClient {
        StubURLProtocol.register(host: host, handler: handler)
        return APIClient(serverURL: URL(string: "https://\(host)")!, session: StubURLProtocol.makeSession())
    }

    @Test("every request says X-FC-Families: 1, uploads included; the socket says families=1")
    func header() async throws {
        let host = "families-header.test"
        defer { StubURLProtocol.unregister(host: host) }
        let api = client(host) { request in
            if request.url.path() == "/api/v1/attachments" {
                return .json(201, #"{"attachment": {"id": 1, "kind": "location", "mime": "", "size": 0, "has_preview": false}}"#)
            }
            return .json(200, #"{"user": {"id": 7, "username": "anna", "display_name": "Anna"}, "family": null, "role": null, "pending_join_request": null}"#)
        }
        _ = try await api.me()
        _ = try? await api.uploadLocation(latitude: 1, longitude: 2, accuracyM: nil, name: nil, family: 5)
        let request = try await api.attachmentUploadRequest(
            mime: "image/jpeg", kind: "photo", width: 1, height: 1, durationMS: nil, family: 5)
        let requests = StubURLProtocol.requests(host: host)
        #expect(requests.count == 2)
        #expect(requests.allSatisfy { $0.headers["X-FC-Families"] == "1" })
        #expect(requests[1].url.query()?.contains("family_id=5") == true)
        #expect(request.value(forHTTPHeaderField: "X-FC-Families") == "1")
        #expect(request.url?.query()?.contains("family_id=5") == true)
        let stream = try #require(await api.attachmentStreamURL(id: 1))
        #expect(stream.headers["X-FC-Families"] == "1")
        let socket = try #require(await ChatSyncCoordinator.webSocketURL(base: URL(string: "https://x.test")!))
        #expect(socket.absoluteString == "wss://x.test/api/v1/ws?families=1")
    }

    @Test("family requests name the family in the path; nil keeps the form an older server knows")
    func paths() async throws {
        let host = "families-paths.test"
        defer { StubURLProtocol.unregister(host: host) }
        let familyJSON = #"{"id": 5, "name": "The Ivanovs", "join_policy": "open"}"#
        let api = client(host) { request in
            switch request.url.path() {
            case "/api/v1/families/5", "/api/v1/families/mine":
                if request.method == "PATCH" { return .json(200, #"{"family": \#(familyJSON)}"#) }
                return .json(200, #"{"family": \#(familyJSON), "members": []}"#)
            case "/api/v1/families/5/board", "/api/v1/families/mine/board":
                return .json(200, #"{"notes": [], "max_board_seq": 0}"#)
            case "/api/v1/families/5/invite-code/rotate", "/api/v1/families/invite-code/rotate":
                return .json(200, #"{"invite_code": "ABCD2345"}"#)
            case "/api/v1/chats/groups":
                return .json(201, #"{"chat": {"id": 77, "kind": "group", "title": "Cousins", "family_ids": [5]}}"#)
            default:
                return .empty(204)
            }
        }
        _ = try await api.myFamily(family: 5)
        _ = try await api.myFamily()
        _ = try await api.board(family: 5)
        _ = try await api.board()
        _ = try await api.leaveFamily(family: 5)
        _ = try await api.leaveFamily()
        _ = try await api.rotateInviteCode(family: 5)
        _ = try await api.rotateInviteCode()
        _ = try await api.setAIHistory(true, family: 5)
        try await api.removeMember(userID: 9, family: 5)
        try await api.blockMember(userID: 9, family: 5)
        let group = try await api.createGroup(title: "Cousins", memberIDs: [9], family: 5)
        let requests = StubURLProtocol.requests(host: host)
        #expect(requests.map { "\($0.method) \($0.url.path())" } == [
            "GET /api/v1/families/5",
            "GET /api/v1/families/mine",
            "GET /api/v1/families/5/board",
            "GET /api/v1/families/mine/board",
            "POST /api/v1/families/5/leave",
            "POST /api/v1/families/leave",
            "POST /api/v1/families/5/invite-code/rotate",
            "POST /api/v1/families/invite-code/rotate",
            "PATCH /api/v1/families/5",
            "DELETE /api/v1/families/5/members/9",
            "PUT /api/v1/families/5/members/9/block",
            "POST /api/v1/chats/groups",
        ])
        #expect(requests.last?.bodyJSON()?["family_id"] as? Int == 5)
        #expect(group.familyIDs == [5])
    }

    @Test("the invitation endpoints, end to end")
    func invitations() async throws {
        let host = "families-invitations.test"
        defer { StubURLProtocol.unregister(host: host) }
        let invitationJSON = #"""
        {"id": 4, "family_id": 3, "family_name": "The Smiths",
         "invited_by": {"id": 7, "username": "anna", "display_name": "Anna"},
         "user": {"id": 13, "username": "boris", "display_name": "Boris"}, "created_at": "2026-10-22T08:00:00Z"}
        """#
        let api = client(host) { request in
            switch (request.method, request.url.path()) {
            case ("POST", "/api/v1/families/3/invitations"): return .json(201, #"{"invitation": \#(invitationJSON)}"#)
            case ("GET", "/api/v1/families/3/invitations"): return .json(200, #"{"invitations": [\#(invitationJSON)]}"#)
            case ("POST", "/api/v1/invitations/4/accept"):
                return .json(200, #"{"family": {"id": 3, "name": "The Smiths", "join_policy": "open"}}"#)
            default: return .empty(204)
            }
        }
        let sent = try await api.invite(userID: 13, family: 3)
        #expect(sent.user.id == 13)
        #expect(try await api.invitations(family: 3).map(\.id) == [4])
        try await api.withdrawInvitation(id: 4, family: 3)
        #expect(try await api.acceptInvitation(id: 4).id == 3)
        try await api.declineInvitation(id: 4)
        let requests = StubURLProtocol.requests(host: host)
        #expect(requests.map { "\($0.method) \($0.url.path())" } == [
            "POST /api/v1/families/3/invitations",
            "GET /api/v1/families/3/invitations",
            "DELETE /api/v1/families/3/invitations/4",
            "POST /api/v1/invitations/4/accept",
            "POST /api/v1/invitations/4/decline",
        ])
        #expect(requests[0].bodyJSON()?["user_id"] as? Int == 13)
    }

    @Test("GET /me's several-families fields decode, and an older /me leaves them absent")
    func meDecodes() throws {
        let me = try APICoding.decoder().decode(MeResponse.self, from: Data(#"""
        {"user": {"id": 7, "username": "anna", "display_name": "Anna"},
         "family": {"id": 3, "name": "The Smiths", "join_policy": "open"}, "role": "owner",
         "pending_join_request": null,
         "families": [{"id": 3, "name": "The Smiths", "join_policy": "open", "role": "owner"},
                      {"id": 5, "name": "The Ivanovs", "join_policy": "approval", "role": "member"}],
         "pending_join_requests": [{"family_id": 9, "family_name": "The Lees", "created_at": "2026-10-22T08:00:00Z"}],
         "invitations": [{"id": 4, "family_id": 11, "family_name": "The Kims",
                          "invited_by": {"id": 21, "username": "min", "display_name": "Min"},
                          "user": {"id": 7, "username": "anna", "display_name": "Anna"}}],
         "member_families_enabled": true, "max_families_per_account": 5}
        """#.utf8))
        #expect(me.families?.map(\.id) == [3, 5])
        #expect(me.families?.map(\.role) == ["owner", "member"])
        #expect(me.pendingJoinRequests.map(\.familyID) == [9])
        #expect(me.invitations.map(\.familyName) == ["The Kims"])
        #expect(me.memberFamiliesEnabled)
        #expect(me.maxFamiliesPerAccount == 5)

        let old = try APICoding.decoder().decode(MeResponse.self, from: Data(#"""
        {"user": {"id": 7, "username": "anna", "display_name": "Anna"}, "family": null, "role": null, "pending_join_request": null}
        """#.utf8))
        #expect(old.families == nil)
        #expect(old.invitations.isEmpty)
        #expect(!old.memberFamiliesEnabled)
        #expect(old.maxFamiliesPerAccount == 1)
    }

    @Test("the new object fields: Chat.family_ids, Note/PackItem.family_id, Member.password_reset")
    func objectFields() throws {
        let decoder = APICoding.decoder()
        let chat = try decoder.decode(ChatDTO.self, from: Data(
            #"{"id": 42, "kind": "direct", "title": "Olga", "peer_user_id": 11, "family_ids": [3, 5]}"#.utf8))
        #expect(chat.familyIDs == [3, 5])
        let note = try decoder.decode(NoteDTO.self, from: Data(
            #"{"id": 1, "board_seq": 2, "family_id": 5}"#.utf8))
        #expect(note.familyID == 5)
        // A tombstone says whose it was too.
        let tombstone = try decoder.decode(NoteDTO.self, from: Data(
            #"{"id": 1, "board_seq": 3, "deleted": true, "family_id": 5}"#.utf8))
        #expect(tombstone.isTombstone && tombstone.familyID == 5)
        let joined = try decoder.decode(JoinResponse.self, from: Data(#"{"status": "pending", "family_id": 9}"#.utf8))
        #expect(joined.familyID == 9)
        #expect(try decoder.decode(JoinResponse.self, from: Data(#"{"status": "joined"}"#.utf8)).familyID == nil)
        let item = try decoder.decode(PackItemDTO.self, from: Data(
            #"{"id": 1, "pack_seq": 2, "family_id": 5}"#.utf8))
        #expect(item.familyID == 5)
        let member = try decoder.decode(MemberDTO.self, from: Data(
            #"{"id": 9, "username": "kid", "display_name": "Kid", "role": "member", "password_reset": false}"#.utf8))
        #expect(member.passwordReset == false)
        let older = try decoder.decode(ChatDTO.self, from: Data(#"{"id": 1, "kind": "family", "title": "F"}"#.utf8))
        #expect(older.familyIDs == nil)
    }

    @Test("an invitation push routes to the invitations, and every push says its family")
    func pushes() {
        #expect(PushRoute.parse(userInfo: ["kind": "invitation", "family_id": 5, "invitation_id": 4]) == .invitations)
        #expect(PushRoute.familyID(userInfo: ["kind": "message", "chat_id": 1, "family_id": 5]) == 5)
        #expect(PushRoute.familyID(userInfo: ["kind": "message", "chat_id": 1, "family_id": "5"]) == 5)
        #expect(PushRoute.familyID(userInfo: ["kind": "message", "chat_id": 1]) == nil)
    }
}

// MARK: - The session

@MainActor
@Suite("Several families: the session", .serialized)
struct SeveralFamiliesSessionTests {

    @MainActor
    private final class Spies {
        var dropped: [Int64] = []
        var adopted: [Int64] = []
        var current: [Int64?] = []
        var resyncs = 0
        var chatStoreCleared = 0
    }

    private func makeSession(api: APIClient? = nil) -> (AppSession, Spies) {
        let spies = Spies()
        let session = AppSession(api: api ?? APIClient(serverURL: nil), defaultServerURL: { nil })
        session.clearChatStore = { spies.chatStoreCleared += 1 }
        session.dropFamilyData = { spies.dropped.append($0) }
        session.adoptUnattributedFamilyData = { spies.adopted.append($0) }
        session.onCurrentFamilyChanged = { spies.current.append($0) }
        session.requestResync = { spies.resyncs += 1 }
        session.chatFamilyIDs = { $0 == 42 ? [5] : nil }
        return (session, spies)
    }

    private func forgetStoredFamily() {
        AppSettings.setCurrentFamilyID(nil, forUser: Fixture.user.id)
    }

    @Test("two families: active, the home family current, the rows from before handed to it")
    func applyTwoFamilies() {
        forgetStoredFamily()
        defer { forgetStoredFamily() }
        let (session, spies) = makeSession()
        session.apply(me: Fixture.me(families: [Fixture.smiths, Fixture.ivanovs]))
        #expect(session.phase == .active)
        #expect(session.families.map(\.id) == [3, 5])
        #expect(session.currentFamilyID == 3)
        #expect(session.family?.name == "The Smiths")
        #expect(session.isOwner)
        #expect(session.showsFamilySwitcher)
        #expect(session.apiFamilyID == 3)
        #expect(spies.adopted == [3])
        #expect(!session.offersNewFamily, "an owner may not start another family")
        #expect(session.offersJoinAnotherFamily)
    }

    @Test("a switch re-points everything and is remembered for this account")
    func switchFamily() {
        forgetStoredFamily()
        defer { forgetStoredFamily() }
        let (session, spies) = makeSession()
        session.apply(me: Fixture.me(families: [Fixture.smiths, Fixture.ivanovs]))
        session.switchFamily(to: 5)
        #expect(session.currentFamilyID == 5)
        #expect(session.homeFamilyID == 3, "a switch moved the home family")
        #expect(session.role == "member")
        #expect(!session.isOwner)
        #expect(spies.current.last == 5)
        #expect(AppSettings.currentFamilyID(forUser: 7) == 5)
        // A family this account is not in is not a switch.
        session.switchFamily(to: 99)
        #expect(session.currentFamilyID == 5)

        // The next launch comes back to it.
        let (again, _) = makeSession()
        again.apply(me: Fixture.me(families: [Fixture.smiths, Fixture.ivanovs]))
        #expect(again.currentFamilyID == 5)
    }

    @Test("a current family that disappears falls back to the first, and its data goes")
    func currentFamilyGone() {
        forgetStoredFamily()
        defer { forgetStoredFamily() }
        let (session, spies) = makeSession()
        session.apply(me: Fixture.me(families: [Fixture.smiths, Fixture.ivanovs]))
        session.switchFamily(to: 5)
        session.apply(me: Fixture.me(families: [Fixture.smiths]))
        #expect(session.currentFamilyID == 3)
        #expect(spies.dropped == [5])
        #expect(spies.chatStoreCleared == 0, "leaving one family purged them all")
        #expect(!session.showsFamilySwitcher)
    }

    @Test("no families: the gate, or the waiting screen — exactly as before")
    func noFamilies() {
        let (session, _) = makeSession()
        session.apply(me: Fixture.me(families: []))
        #expect(session.phase == .needsFamily)
        let request = PendingJoinRequestDTO(familyID: 9, familyName: "The Lees", createdAt: nil)
        session.apply(me: Fixture.me(families: [], pending: [request]))
        #expect(session.phase == .pendingApproval)
    }

    @Test("a pending request to another family is a switcher row, not the waiting screen")
    func pendingWhileActive() {
        forgetStoredFamily()
        defer { forgetStoredFamily() }
        let (session, _) = makeSession()
        let request = PendingJoinRequestDTO(familyID: 9, familyName: "The Lees", createdAt: nil)
        session.apply(me: Fixture.me(families: [Fixture.ivanovs], pending: [request]))
        #expect(session.phase == .active)
        #expect(session.pendingJoinRequests == [request])
        #expect(session.showsFamilySwitcher)
        #expect(session.offersNewFamily)
    }

    @Test("an older server: one family, spoken to by the old paths")
    func legacyServer() {
        forgetStoredFamily()
        defer { forgetStoredFamily() }
        let (session, _) = makeSession()
        session.apply(me: MeResponse(
            user: Fixture.user, family: Fixture.family(3, "The Smiths"), role: "member", pendingJoinRequest: nil))
        #expect(session.isLegacyServer)
        #expect(session.currentFamilyID == 3)
        #expect(session.apiFamilyID == nil)
        #expect(session.role == "member")
        #expect(!session.showsFamilySwitcher)
        #expect(!session.offersJoinAnotherFamily)
    }

    @Test("family_owner moves the role in THAT family only")
    func familyOwnerPerFamily() {
        forgetStoredFamily()
        defer { forgetStoredFamily() }
        let (session, _) = makeSession()
        session.apply(me: Fixture.me(families: [Fixture.smiths, Fixture.ivanovs]))
        session.applyFamilyOwner(familyID: 5, userID: 7)
        #expect(session.families.first { $0.id == 5 }?.role == "owner")
        #expect(session.families.first { $0.id == 3 }?.role == "owner")
        session.applyFamilyOwner(familyID: 3, userID: 9)
        #expect(session.families.first { $0.id == 3 }?.role == "member")
        #expect(session.families.first { $0.id == 5 }?.role == "owner")
        // A family this account is not in changes nothing.
        let generation = session.familyOwnerGeneration
        session.applyFamilyOwner(familyID: 99, userID: 7)
        #expect(session.familyOwnerGeneration == generation)
    }

    @Test("accepting an invitation adds the family and shows it; a stale one is dropped and said")
    func acceptInvitation() async throws {
        forgetStoredFamily()
        defer { forgetStoredFamily() }
        let host = "families-accept.test"
        defer { StubURLProtocol.unregister(host: host) }
        StubURLProtocol.register(host: host) { request in
            switch request.url.path() {
            case "/api/v1/invitations/4/accept":
                return .json(200, #"{"family": {"id": 5, "name": "The Ivanovs", "join_policy": "open"}}"#)
            default:
                return .json(409, #"{"error": {"code": "invitation_not_pending", "message": "gone"}}"#)
            }
        }
        let api = APIClient(serverURL: URL(string: "https://\(host)")!, session: StubURLProtocol.makeSession())
        let (session, spies) = makeSession(api: api)
        session.apply(me: Fixture.me(
            families: [Fixture.smiths], invitations: [Fixture.invitation(4), Fixture.invitation(6, family: 8, name: "The Lees")]))
        #expect(session.showsFamilySwitcher)

        try await session.acceptInvitation(Fixture.invitation(4))
        #expect(session.families.map(\.id) == [3, 5])
        #expect(session.currentFamilyID == 5)
        #expect(session.role == "member")
        #expect(session.invitations.map(\.id) == [6])
        #expect(spies.resyncs == 1)

        await #expect(throws: APIError.conflict(code: "invitation_not_pending", message: "gone")) {
            try await session.acceptInvitation(Fixture.invitation(6, family: 8, name: "The Lees"))
        }
        #expect(session.invitations.isEmpty)
    }

    @Test("declining takes the row away, even when the server says it already went")
    func declineInvitation() async throws {
        let host = "families-decline.test"
        defer { StubURLProtocol.unregister(host: host) }
        StubURLProtocol.register(host: host) { _ in
            .json(409, #"{"error": {"code": "invitation_not_pending", "message": "gone"}}"#)
        }
        let api = APIClient(serverURL: URL(string: "https://\(host)")!, session: StubURLProtocol.makeSession())
        let (session, _) = makeSession(api: api)
        session.apply(me: Fixture.me(families: [Fixture.smiths], invitations: [Fixture.invitation(4)]))
        try await session.declineInvitation(Fixture.invitation(4))
        #expect(session.invitations.isEmpty)
    }

    @Test("invitation frames keep the list live")
    func invitationFrames() {
        let (session, _) = makeSession()
        session.apply(me: Fixture.me(families: [Fixture.smiths]))
        let generation = session.invitationsGeneration
        session.applyInvitation(Fixture.invitation(4))
        #expect(session.invitations.map(\.id) == [4])
        // The owner's copy of an invitation they sent is not THEIR invitation.
        session.applyInvitation(InvitationDTO(
            id: 9, familyID: 3, familyName: "The Smiths", invitedBy: Fixture.user,
            user: UserDTO(id: 13, username: "boris", displayName: "Boris", createdAt: nil), createdAt: nil))
        #expect(session.invitations.map(\.id) == [4])
        session.removeInvitation(id: 4)
        #expect(session.invitations.isEmpty)
        #expect(session.invitationsGeneration == generation &+ 3)
    }

    @Test("New Family… from inside the app: owned, shown, and fetched")
    func createAnother() async throws {
        forgetStoredFamily()
        defer { forgetStoredFamily() }
        let host = "families-create.test"
        defer { StubURLProtocol.unregister(host: host) }
        StubURLProtocol.register(host: host) { _ in
            .json(201, #"{"family": {"id": 12, "name": "Our Own", "join_policy": "open"}}"#)
        }
        let api = APIClient(serverURL: URL(string: "https://\(host)")!, session: StubURLProtocol.makeSession())
        let (session, spies) = makeSession(api: api)
        session.apply(me: Fixture.me(families: [Fixture.ivanovs]))
        #expect(session.offersNewFamily)
        try await session.createFamily(name: "Our Own")
        #expect(session.families.map(\.id) == [5, 12])
        #expect(session.currentFamilyID == 12)
        #expect(session.isOwner)
        #expect(!session.offersNewFamily)
        #expect(spies.resyncs == 1)
    }

    @Test("Join Another Family…: the answer's family_id says which family to show")
    func joinAnother() async throws {
        forgetStoredFamily()
        defer { forgetStoredFamily() }
        let host = "families-join.test"
        defer { StubURLProtocol.unregister(host: host) }
        StubURLProtocol.register(host: host) { request in
            switch request.url.path() {
            case "/api/v1/families/join": return .json(200, #"{"status": "joined", "family_id": 5}"#)
            default:
                return .json(200, #"""
                {"user": {"id": 7, "username": "anna", "display_name": "Anna"},
                 "family": {"id": 3, "name": "The Smiths", "join_policy": "open"}, "role": "member",
                 "pending_join_request": null,
                 "families": [{"id": 3, "name": "The Smiths", "join_policy": "open", "role": "member"},
                              {"id": 5, "name": "The Ivanovs", "join_policy": "open", "role": "member"},
                              {"id": 8, "name": "The Lees", "join_policy": "open", "role": "member"}],
                 "pending_join_requests": [], "invitations": [], "member_families_enabled": true,
                 "max_families_per_account": 5}
                """#)
            }
        }
        let api = APIClient(serverURL: URL(string: "https://\(host)")!, session: StubURLProtocol.makeSession())
        let (session, spies) = makeSession(api: api)
        session.apply(me: Fixture.me(families: [Fixture.family(3, "The Smiths", role: "member")]))
        // Two families appear at once (one joined on another device); the
        // answer, not a guess, says which one this code was for.
        #expect(try await session.join(code: "ABCD2345") == .joined)
        #expect(session.currentFamilyID == 5)
        #expect(session.phase == .active)
        #expect(spies.resyncs == 1)
    }

    @Test("leaving one of several leaves that family only; leaving the last is the old way to the gate")
    func leave() async throws {
        forgetStoredFamily()
        defer { forgetStoredFamily() }
        let host = "families-leave.test"
        defer { StubURLProtocol.unregister(host: host) }
        StubURLProtocol.register(host: host) { _ in .empty(204) }
        let api = APIClient(serverURL: URL(string: "https://\(host)")!, session: StubURLProtocol.makeSession())
        let (session, spies) = makeSession(api: api)
        session.apply(me: Fixture.me(families: [Fixture.smiths, Fixture.ivanovs]))
        session.switchFamily(to: 5)

        _ = try await session.leaveFamily { _ in nil }
        #expect(StubURLProtocol.requests(host: host).last?.url.path() == "/api/v1/families/5/leave")
        #expect(session.phase == .active)
        #expect(session.families.map(\.id) == [3])
        #expect(session.currentFamilyID == 3)
        #expect(spies.dropped == [5])
        #expect(spies.chatStoreCleared == 0)

        _ = try await session.leaveFamily { _ in nil }
        #expect(StubURLProtocol.requests(host: host).last?.url.path() == "/api/v1/families/3/leave")
        #expect(session.phase == .needsFamily)
        #expect(session.families.isEmpty)
        #expect(spies.chatStoreCleared == 1)
    }

    @Test("removed from the last family: the gate, and every family's marks go with it")
    func removedFromLast() {
        forgetStoredFamily()
        defer { forgetStoredFamily() }
        let (session, spies) = makeSession()
        session.apply(me: Fixture.me(families: [Fixture.ivanovs]))
        AppSettings.setBoardCursor(40, family: 5)
        session.removeFamily(5)
        #expect(session.phase == .needsFamily)
        #expect(spies.chatStoreCleared == 1)
        #expect(AppSettings.boardCursor(family: 5) == 0, "a board cursor outlived the board it counted")
    }

    /// A leave whose own `member_left` beats its answer home. The stub
    /// handler runs on the URLProtocol's thread while `leaveFamily` is
    /// suspended on the request, and applies the frame on the main actor
    /// exactly as the coordinator's handler does (`removeFamily`) — so the
    /// echo is in before the 204 is.
    private func leaveRacedByItsEcho(
        host: String, families: [FamilyDTO], pending: [PendingJoinRequestDTO] = [], leaving: Int64
    ) async throws -> (AppSession, Spies) {
        final class Box: @unchecked Sendable { var session: AppSession? }
        let box = Box()
        StubURLProtocol.register(host: host) { _ in
            if let session = box.session {
                DispatchQueue.main.sync { MainActor.assumeIsolated { session.removeFamily(leaving) } }
            }
            return .empty(204)
        }
        let api = APIClient(serverURL: URL(string: "https://\(host)")!, session: StubURLProtocol.makeSession())
        let (session, spies) = makeSession(api: api)
        box.session = session
        session.apply(me: Fixture.me(families: families, pending: pending))
        session.switchFamily(to: leaving)
        _ = try await session.leaveFamily { _ in nil }
        #expect(StubURLProtocol.requests(host: host).last?.url.path() == "/api/v1/families/\(leaving)/leave")
        return (session, spies)
    }

    @Test("leaving one of two families when its member_left lands first: the other family stays, nothing else is purged")
    func leaveRacedByEchoKeepsTheOtherFamily() async throws {
        forgetStoredFamily()
        defer { forgetStoredFamily() }
        let host = "families-leave-echo.test"
        defer { StubURLProtocol.unregister(host: host) }
        let (session, spies) = try await leaveRacedByItsEcho(
            host: host, families: [Fixture.smiths, Fixture.ivanovs], leaving: 5)
        #expect(session.phase == .active, "a leave of one family raced by its echo went to the gate")
        #expect(session.families.map(\.id) == [3])
        #expect(session.currentFamilyID == 3)
        #expect(spies.chatStoreCleared == 0, "the whole store was purged while still in the Smiths")
        #expect(spies.dropped == [5], "the family left was dropped twice, or not at all")
    }

    @Test("leaving the LAST family when its member_left lands first: one purge, and the gate")
    func leaveLastRacedByEcho() async throws {
        forgetStoredFamily()
        defer { forgetStoredFamily() }
        let host = "families-leave-last-echo.test"
        defer { StubURLProtocol.unregister(host: host) }
        let (session, spies) = try await leaveRacedByItsEcho(host: host, families: [Fixture.ivanovs], leaving: 5)
        #expect(session.phase == .needsFamily)
        #expect(session.families.isEmpty)
        #expect(spies.chatStoreCleared == 1)
    }

    @Test("losing the last family with a join request pending elsewhere is the waiting screen, not the gate")
    func lastFamilyGoneWhilePending() {
        forgetStoredFamily()
        defer { forgetStoredFamily(); AppSettings.joinPending = false }
        AppSettings.joinPending = false
        let (session, spies) = makeSession()
        let request = PendingJoinRequestDTO(familyID: 9, familyName: "The Lees", createdAt: nil)
        session.apply(me: Fixture.me(families: [Fixture.ivanovs], pending: [request]))
        #expect(session.phase == .active)

        // A `member_left` naming this account (the coordinator's call).
        session.removeFamily(5)
        #expect(session.phase == .pendingApproval)
        #expect(AppSettings.joinPending, "the waiting screen's poll and the decline both read it")
        #expect(session.families.isEmpty)
        #expect(session.pendingJoinRequests == [request])
        #expect(spies.chatStoreCleared == 1)

        // The owner approves: the poll's `/me` routes the app in.
        session.apply(me: Fixture.me(families: [Fixture.family(9, "The Lees", role: "member")]))
        #expect(session.phase == .active)
        #expect(session.currentFamilyID == 9)
        #expect(!AppSettings.joinPending)
    }

    @Test("leaving the last family with a join request pending elsewhere waits on it; a decline then says so")
    func leaveLastWhilePending() async throws {
        forgetStoredFamily()
        defer { forgetStoredFamily(); AppSettings.joinPending = false }
        AppSettings.joinPending = false
        let host = "families-leave-pending.test"
        defer { StubURLProtocol.unregister(host: host) }
        StubURLProtocol.register(host: host) { _ in .empty(204) }
        let api = APIClient(serverURL: URL(string: "https://\(host)")!, session: StubURLProtocol.makeSession())
        let (session, spies) = makeSession(api: api)
        let request = PendingJoinRequestDTO(familyID: 9, familyName: "The Lees", createdAt: nil)
        session.apply(me: Fixture.me(families: [Fixture.ivanovs], pending: [request]))

        _ = try await session.leaveFamily { _ in nil }
        #expect(session.phase == .pendingApproval)
        #expect(AppSettings.joinPending)
        #expect(spies.chatStoreCleared == 1)

        // The Lees decline: neither a family nor the request any more.
        session.apply(me: Fixture.me(families: []))
        #expect(session.phase == .needsFamily)
        #expect(session.joinDeclined)
        #expect(spies.chatStoreCleared == 1, "nothing was left to purge a second time")
    }

    @Test("removed from the last family while offline, with a request pending: /me purges what was held")
    func removedWhileOfflineWithPendingRequest() {
        forgetStoredFamily()
        defer { forgetStoredFamily(); AppSettings.joinPending = false }
        AppSettings.joinPending = false
        let (session, spies) = makeSession()
        let request = PendingJoinRequestDTO(familyID: 9, familyName: "The Lees", createdAt: nil)
        session.apply(me: Fixture.me(families: [Fixture.smiths], pending: [request]))
        #expect(session.phase == .active)

        // No `member_left` ever reached this device; the next `/me` (a
        // resync's step 1) lists no family and the request still waiting.
        session.apply(me: Fixture.me(families: [], pending: [request]))
        #expect(session.phase == .pendingApproval)
        #expect(session.families.isEmpty)
        #expect(spies.chatStoreCleared == 1, "the Smiths' chats, board and pack stayed under the waiting screen")
        #expect(AppSettings.joinPending)

        // And when the Lees then decline, the gate — purged once, already.
        session.apply(me: Fixture.me(families: []))
        #expect(session.phase == .needsFamily)
        #expect(session.joinDeclined)
        #expect(spies.chatStoreCleared == 1)
    }

    @Test("a push opens the app in its family; a direct chat's in the family that lists it")
    func pushRouting() {
        forgetStoredFamily()
        defer { forgetStoredFamily() }
        let (session, _) = makeSession()
        session.apply(me: Fixture.me(families: [Fixture.smiths, Fixture.ivanovs]))
        session.routePush(.board, familyID: 5)
        #expect(session.currentFamilyID == 5)
        #expect(session.pendingPushRoute == .board)
        session.switchFamily(to: 3)
        // No family on the push: chat 42 is listed in 5 only (the spy).
        session.routePush(.chat(42), familyID: nil)
        #expect(session.currentFamilyID == 5)
    }

    @Test("a cold-start push waits for /me to say the family is held")
    func coldStartPush() {
        forgetStoredFamily()
        defer { forgetStoredFamily() }
        let (session, _) = makeSession()
        session.routePush(.board, familyID: 5)
        #expect(session.currentFamilyID == nil)
        session.apply(me: Fixture.me(families: [Fixture.smiths, Fixture.ivanovs]))
        #expect(session.currentFamilyID == 5)
    }
}

// MARK: - The store

@MainActor
@Suite("Several families: the store", .serialized)
struct SeveralFamiliesStoreTests {

    /// What `GET /me` says, which a test changes between steps.
    private final class Server: @unchecked Sendable {
        private let lock = NSLock()
        private var _families: [Int64] = [3, 5]
        var families: [Int64] {
            get { lock.lock(); defer { lock.unlock() }; return _families }
            set { lock.lock(); _families = newValue; lock.unlock() }
        }
    }

    private struct Harness {
        let container: ModelContainer
        let coordinator: ChatSyncCoordinator
        let session: AppSession
        let context: ModelContext
        let host: String
        let server: Server

        func chat(_ id: Int64) -> ChatEntity? {
            let descriptor = FetchDescriptor<ChatEntity>(predicate: #Predicate { $0.chatID == id })
            return try? context.fetch(descriptor).first
        }

        func member(_ id: Int64) -> MemberEntity? {
            let descriptor = FetchDescriptor<MemberEntity>(predicate: #Predicate { $0.userID == id })
            return try? context.fetch(descriptor).first
        }

        func notes() -> [(id: Int64, family: Int64)] {
            ((try? context.fetch(FetchDescriptor<NoteEntity>())) ?? [])
                .map { ($0.noteID, $0.familyID) }.sorted { $0.id < $1.id }
        }

        func packItems() -> [(id: Int64, family: Int64)] {
            ((try? context.fetch(FetchDescriptor<PackItemEntity>())) ?? [])
                .map { ($0.itemID, $0.familyID) }.sorted { $0.id < $1.id }
        }
    }

    nonisolated private static let familyJSON: [Int64: String] = [
        3: #"{"id": 3, "name": "The Smiths", "join_policy": "open", "role": "owner"}"#,
        5: #"{"id": 5, "name": "The Ivanovs", "join_policy": "open", "role": "member"}"#,
    ]

    nonisolated private static func meJSON(_ families: [Int64]) -> String {
        let list = families.compactMap { familyJSON[$0] }.joined(separator: ", ")
        return #"""
        {"user": {"id": 7, "username": "anna", "display_name": "Anna"},
         "family": \#(families.first.flatMap { familyJSON[$0] } ?? "null"), "role": "owner",
         "pending_join_request": null, "families": [\#(list)], "pending_join_requests": [],
         "invitations": [], "member_families_enabled": true, "max_families_per_account": 5}
        """#
    }

    nonisolated private static func rosterJSON(_ family: Int64) -> String {
        let members = family == 3
            ? #"""
              {"id": 7, "username": "anna", "display_name": "Anna", "role": "owner"},
              {"id": 9, "username": "kid", "display_name": "Kid", "role": "member", "password_reset": true},
              {"id": 11, "username": "olga", "display_name": "Olga", "role": "member", "password_reset": false}
              """#
            : #"""
              {"id": 7, "username": "anna", "display_name": "Anna", "role": "member"},
              {"id": 11, "username": "olga", "display_name": "Olga", "role": "owner"},
              {"id": 13, "username": "boris", "display_name": "Boris", "role": "member"}
              """#
        return #"""
        {"family": \#(familyJSON[family]!), "members": [\#(members)],
         "max_board_seq": \#(family * 10), "max_pack_seq": \#(family * 10), "max_pack_items": 200,
         "max_pack_item_bytes": 524288}
        """#
    }

    nonisolated private static let chatsJSON = #"""
    {"chats": [
      {"chat": {"id": 1, "kind": "family", "title": "The Smiths", "family_ids": [3]}, "last_message": null, "unread_count": 2},
      {"chat": {"id": 2, "kind": "family", "title": "The Ivanovs", "family_ids": [5]}, "last_message": null, "unread_count": 4},
      {"chat": {"id": 42, "kind": "direct", "title": "Olga", "peer_user_id": 11, "family_ids": [3, 5]}, "last_message": null, "unread_count": 1},
      {"chat": {"id": 44, "kind": "direct", "title": "Boris", "peer_user_id": 13, "family_ids": [5]}, "last_message": null, "unread_count": 0},
      {"chat": {"id": 50, "kind": "group", "title": "Cousins", "member_ids": [7, 13], "admin_user_id": 7, "family_ids": [5]}, "last_message": null, "unread_count": 0}
    ]}
    """#

    nonisolated private static func boardJSON(_ family: Int64) -> String {
        #"""
        {"notes": [{"id": \#(family * 100), "author_id": 7, "text": "Milk", "color": "yellow",
                    "x": 0.1, "y": 0.2, "board_seq": \#(family * 10), "family_id": \#(family)}],
         "max_board_seq": \#(family * 10)}
        """#
    }

    nonisolated private static func packJSON(_ family: Int64) -> String {
        #"""
        {"items": [{"id": \#(family * 1000), "added_by": 7, "pack_seq": \#(family * 10), "family_id": \#(family),
                    "attachment": {"id": \#(family * 1000 + 1), "kind": "photo", "mime": "image/png", "size": 10,
                                   "width": 10, "height": 10, "has_preview": false}}],
         "max_pack_seq": \#(family * 10)}
        """#
    }

    /// `extra` answers first when it answers at all — for a test that has to
    /// do something while one particular request is in flight.
    private func makeHarness(
        host: String, extra: (@Sendable (RecordedRequest) -> StubResponse?)? = nil
    ) throws -> Harness {
        let server = Server()
        StubURLProtocol.register(host: host) { request in
            if let answer = extra?(request) { return answer }
            let path = request.url.path()
            switch path {
            case "/api/v1/me": return .json(200, Self.meJSON(server.families))
            case "/api/v1/chats": return .json(200, Self.chatsJSON)
            default: break
            }
            for family: Int64 in [3, 5] {
                switch path {
                case "/api/v1/families/\(family)": return .json(200, Self.rosterJSON(family))
                case "/api/v1/families/\(family)/board": return .json(200, Self.boardJSON(family))
                case "/api/v1/families/\(family)/pack": return .json(200, Self.packJSON(family))
                default: continue
                }
            }
            return .json(200, #"{"messages": []}"#)
        }
        let container = try ModelContainer(
            for: ChatEntity.self, MessageEntity.self, MemberEntity.self, NoteEntity.self,
            PendingMediaItemEntity.self, BlockEntity.self, GoneNoteEntity.self,
            PackItemEntity.self, GonePackItemEntity.self,
            configurations: ModelConfiguration(isStoredInMemoryOnly: true))
        let api = APIClient(serverURL: URL(string: "https://\(host)")!, session: StubURLProtocol.makeSession())
        let coordinator = ChatSyncCoordinator(modelContainer: container, api: api)
        coordinator.currentUserIDOverride = 7
        coordinator.packCursorOverride = 0
        let session = AppSession(api: api, defaultServerURL: { nil })
        coordinator.bind(session: session)
        // The app's wiring (FamilyConnectApp), store-side.
        session.onCurrentFamilyChanged = { [weak coordinator] in coordinator?.projectRoster(onto: $0 ?? 0) }
        session.dropFamilyData = { [weak coordinator] in coordinator?.dropFamily($0) }
        session.adoptUnattributedFamilyData = { [weak coordinator] in coordinator?.adoptUnattributed(into: $0) }
        session.chatFamilyIDs = { [weak coordinator] in coordinator?.chatFamilyIDs(of: $0) }
        AppSettings.setCurrentFamilyID(nil, forUser: 7)
        AppSettings.forgetFamily(3)
        AppSettings.forgetFamily(5)
        return Harness(
            container: container, coordinator: coordinator, session: session,
            context: container.mainContext, host: host, server: server)
    }

    private func tearDown(_ harness: Harness) {
        StubURLProtocol.unregister(host: harness.host)
        AppSettings.setCurrentFamilyID(nil, forUser: 7)
        AppSettings.forgetFamily(3)
        AppSettings.forgetFamily(5)
    }

    @Test("a resync reads EVERY family's roster, board and pack, and files each under its family")
    func resyncReadsEveryFamily() async throws {
        let harness = try makeHarness(host: "families-resync.test")
        defer { tearDown(harness) }

        await harness.coordinator.resync()

        let paths = Set(StubURLProtocol.requests(host: harness.host).map { $0.url.path() })
        for path in ["/api/v1/families/3", "/api/v1/families/5", "/api/v1/families/3/board",
                     "/api/v1/families/5/board", "/api/v1/families/3/pack", "/api/v1/families/5/pack"] {
            #expect(paths.contains(path), "\(path) was never read")
        }
        #expect(!paths.contains("/api/v1/families/mine"))
        // ONE read of each family: the roster's step and the board/pack step
        // share it (it used to be read twice, once for each).
        let familyReads = StubURLProtocol.requests(host: harness.host)
            .filter { $0.method == "GET" && ["/api/v1/families/3", "/api/v1/families/5"].contains($0.url.path()) }
            .map { $0.url.path() }
        #expect(familyReads.sorted() == ["/api/v1/families/3", "/api/v1/families/5"])
        #expect(harness.notes().map(\.family) == [3, 5])
        #expect(harness.packItems().map(\.family) == [3, 5])
        #expect(harness.chat(42)?.familyIDs == [3, 5])
        #expect(harness.chat(2)?.familyIDs == [5])

        // One row per person, and their place in each family.
        let olga = try #require(harness.member(11))
        #expect(olga.familyRoles == [3: "member", 5: "owner"])
        #expect(!olga.passwordResetAllowed(in: 3))
        #expect(harness.member(9)?.passwordResetAllowed(in: 3) == true)
        // Projected onto the family on screen, the Smiths.
        #expect(olga.role == "member")
        #expect(harness.member(13)?.hasLeft == true)
        #expect(harness.member(9)?.hasLeft == false)
    }

    @Test("a switch re-projects the roster onto the new family")
    func switchReprojects() async throws {
        let harness = try makeHarness(host: "families-switch.test")
        defer { tearDown(harness) }
        await harness.coordinator.resync()

        harness.session.switchFamily(to: 5)

        #expect(harness.member(11)?.role == "owner")
        #expect(harness.member(13)?.hasLeft == false)
        #expect(harness.member(9)?.hasLeft == true)
        #expect(harness.coordinator.currentFamilyKey == 5)
        #expect(harness.coordinator.packItems().map(\.id) == [5000])
        #expect(harness.coordinator.uploadFamily(forChat: 42) == 5)
        #expect(harness.coordinator.uploadFamily(forChat: 1) == 3)
    }

    @Test("member_left naming me drops THAT family's chats, board, pack and roster — and nobody else's")
    func memberLeftMeDropsOneFamily() async throws {
        let harness = try makeHarness(host: "families-left.test")
        defer { tearDown(harness) }
        await harness.coordinator.resync()
        harness.session.switchFamily(to: 5)
        harness.server.families = [3]

        harness.coordinator.handle(frame: .memberLeft(familyID: 5, userID: 7))

        #expect(harness.session.families.map(\.id) == [3])
        #expect(harness.session.currentFamilyID == 3)
        // The Ivanovs' family chat, their group and a direct chat listed only
        // there go; the direct chat shared through the Smiths stays.
        #expect(harness.chat(2) == nil)
        #expect(harness.chat(44) == nil)
        #expect(harness.chat(50) == nil)
        #expect(harness.chat(1) != nil)
        #expect(harness.chat(42)?.familyIDs == [3])
        #expect(harness.notes().map(\.family) == [3])
        #expect(harness.packItems().map(\.family) == [3])
        #expect(harness.member(11)?.familyRoles == [3: "member"])
        #expect(harness.member(13)?.hasLeft == true)
    }

    @Test("removed from a family: a direct chat closed by `family_ids: []` is listed nowhere, then dropped with the family")
    func closedDirectChatDropsWithFamily() async throws {
        let harness = try makeHarness(host: "families-closed-direct.test")
        defer { tearDown(harness) }
        await harness.coordinator.resync()
        #expect(harness.chat(44)?.familyIDs == [5])

        // The server closes the direct chat with Boris for this side — they
        // shared only the Ivanovs — BEFORE the `member_left` that drops it.
        let closed = try APICoding.decoder().decode(ServerFrame.self, from: Data(#"""
        {"type": "chat_updated",
         "chat": {"id": 44, "kind": "direct", "title": "Boris", "peer_user_id": 13, "family_ids": []}}
        """#.utf8))
        harness.coordinator.handle(frame: closed)
        let chat = try #require(harness.chat(44))
        #expect(chat.familyIDs == [])
        #expect(!FamilyRules.isListed(chatFamilyIDs: chat.familyIDs, in: 3, home: harness.session.homeFamilyID))
        #expect(!FamilyRules.isListed(chatFamilyIDs: chat.familyIDs, in: 5, home: harness.session.homeFamilyID))

        harness.server.families = [3]
        harness.coordinator.handle(frame: .memberLeft(familyID: 5, userID: 7))

        #expect(harness.chat(44) == nil)
        #expect(harness.chat(42)?.familyIDs == [3])
        #expect(harness.chat(1) != nil)
    }

    @Test("member_joined: the owner's copy stores password_reset; a copy without it keeps what is held")
    func memberJoinedPasswordReset() async throws {
        let harness = try makeHarness(host: "families-joined-reset.test")
        defer { tearDown(harness) }
        await harness.coordinator.resync()

        func joined(_ userID: Int64, family: Int64, flag: String?) throws -> ServerFrame {
            let extra = flag.map { #", "password_reset": \#($0)"# } ?? ""
            return try APICoding.decoder().decode(ServerFrame.self, from: Data(#"""
            {"type": "member_joined", "family_id": \#(family),
             "user": {"id": \#(userID), "username": "u\#(userID)", "display_name": "U\#(userID)"}\#(extra)}
            """#.utf8))
        }

        // A newcomer the owner may NOT reset (they are in a family of their own).
        let frame = try joined(21, family: 3, flag: "false")
        guard case .memberJoined(let payload) = frame else {
            Issue.record("expected .memberJoined, got \(frame)")
            return
        }
        #expect(payload.passwordReset == false)
        harness.coordinator.handle(frame: frame)
        #expect(harness.member(21)?.passwordResetAllowed(in: 3) == false)

        // The same member's frame again, as another reader's copy carries it
        // (no flag): the refusal stands.
        harness.coordinator.handle(frame: try joined(21, family: 3, flag: nil))
        #expect(harness.member(21)?.passwordResetAllowed(in: 3) == false)
        // Olga's refusal from the roster survives a flagless frame too.
        harness.coordinator.handle(frame: try joined(11, family: 3, flag: nil))
        #expect(harness.member(11)?.passwordResetAllowed(in: 3) == false)

        // The owner's copy saying true lifts it.
        harness.coordinator.handle(frame: try joined(21, family: 3, flag: "true"))
        #expect(harness.member(21)?.passwordResetAllowed(in: 3) == true)
        // And a frame for one family leaves the other alone.
        harness.coordinator.handle(frame: try joined(21, family: 5, flag: "false"))
        #expect(harness.member(21)?.passwordResetAllowed(in: 3) == true)
        #expect(harness.member(21)?.passwordResetAllowed(in: 5) == false)
    }

    @Test("member_joined never demotes: an owner already on that roster stays owner; a newcomer is a member")
    func memberJoinedKeepsHeldRole() async throws {
        let harness = try makeHarness(host: "families-joined-role.test")
        defer { tearDown(harness) }
        await harness.coordinator.resync()
        #expect(harness.member(7)?.familyRoles[3] == "owner")

        func joined(_ userID: Int64, family: Int64) throws -> ServerFrame {
            try APICoding.decoder().decode(ServerFrame.self, from: Data(#"""
            {"type": "member_joined", "family_id": \#(family),
             "user": {"id": \#(userID), "username": "u\#(userID)", "display_name": "U\#(userID)"}}
            """#.utf8))
        }

        // "New Family…" from inside the app: the server's member_joined names
        // the CREATOR, for a family this device already holds and whose
        // roster it has read with the creator as owner.
        harness.coordinator.handle(frame: try joined(7, family: 3))
        #expect(harness.member(7)?.familyRoles[3] == "owner", "the creator was demoted by their own member_joined")
        #expect(harness.member(7)?.role == "owner")
        #expect(harness.session.isOwner)
        // Anybody else's role in the family the frame names stands too —
        // Olga owns the Ivanovs and is a member of the Smiths.
        harness.coordinator.handle(frame: try joined(11, family: 5))
        harness.coordinator.handle(frame: try joined(11, family: 3))
        #expect(harness.member(11)?.familyRoles == [3: "member", 5: "owner"])
        // A row new to that roster is a member: somebody held from the other
        // family, and somebody this device has never seen.
        harness.coordinator.handle(frame: try joined(13, family: 3))
        #expect(harness.member(13)?.familyRoles == [3: "member", 5: "member"])
        harness.coordinator.handle(frame: try joined(23, family: 3))
        #expect(harness.member(23)?.familyRoles == [3: "member"])
        // And the role still moves when something says so outright.
        harness.coordinator.handle(frame: .familyOwner(familyID: 3, userID: 13))
        #expect(harness.member(13)?.familyRoles[3] == "owner")
    }

    @Test("a leave whose member_left beats its 204 home, through the coordinator: the other family's data stays")
    func leaveRacedByEchoThroughTheCoordinator() async throws {
        // The frame is injected from the leave's own handler, on the
        // URLProtocol's thread, while `leaveFamily` waits on the answer.
        final class Box: @unchecked Sendable { var coordinator: ChatSyncCoordinator? }
        let box = Box()
        let harness = try makeHarness(host: "families-leave-echo-store.test", extra: { request in
            guard request.method == "POST", request.url.path() == "/api/v1/families/5/leave" else { return nil }
            if let coordinator = box.coordinator {
                DispatchQueue.main.sync {
                    MainActor.assumeIsolated { coordinator.handle(frame: .memberLeft(familyID: 5, userID: 7)) }
                }
            }
            return .empty(204)
        })
        box.coordinator = harness.coordinator
        defer { tearDown(harness) }
        // The harness leaves the store-wide wipe unwired; counted here, since
        // a wipe is exactly what the race used to cause.
        final class Purges { var count = 0 }
        let purges = Purges()
        harness.session.clearChatStore = { purges.count += 1 }
        await harness.coordinator.resync()
        harness.session.switchFamily(to: 5)
        harness.server.families = [3]

        _ = try await harness.session.leaveFamily { _ in nil }

        #expect(harness.session.phase == .active, "a leave of one family raced by its echo went to the gate")
        #expect(harness.session.families.map(\.id) == [3])
        #expect(harness.session.currentFamilyID == 3)
        #expect(purges.count == 0, "the whole store was purged while still in the Smiths")
        // The Smiths' chats, board, pack and roster survived; the Ivanovs' went.
        #expect(harness.chat(1) != nil)
        #expect(harness.chat(42)?.familyIDs == [3])
        #expect(harness.chat(2) == nil)
        #expect(harness.notes().map(\.family) == [3])
        #expect(harness.packItems().map(\.family) == [3])
        #expect(harness.member(11)?.familyRoles == [3: "member"])
    }

    @Test("somebody else leaving one family is still in the other")
    func memberLeftOtherFamily() async throws {
        let harness = try makeHarness(host: "families-other-left.test")
        defer { tearDown(harness) }
        await harness.coordinator.resync()

        harness.coordinator.handle(frame: .memberLeft(familyID: 5, userID: 11))
        #expect(harness.member(11)?.familyRoles == [3: "member"])
        #expect(harness.member(11)?.hasLeft == false, "left the Ivanovs, shown as gone from the Smiths")

        harness.coordinator.handle(frame: .memberLeft(familyID: 3, userID: 9))
        #expect(harness.member(9)?.hasLeft == true)
    }

    @Test("member_joined and family_owner land on the family they name")
    func framesNameTheirFamily() async throws {
        let harness = try makeHarness(host: "families-frames.test")
        defer { tearDown(harness) }
        await harness.coordinator.resync()

        let joined = try APICoding.decoder().decode(ServerFrame.self, from: Data(#"""
        {"type": "member_joined", "family_id": 5,
         "user": {"id": 9, "username": "kid", "display_name": "Kid", "avatar_version": 0}}
        """#.utf8))
        harness.coordinator.handle(frame: joined)
        #expect(harness.member(9)?.familyRoles == [3: "member", 5: "member"])

        harness.coordinator.handle(frame: .familyOwner(familyID: 5, userID: 13))
        #expect(harness.member(13)?.familyRoles[5] == "owner")
        #expect(harness.member(11)?.familyRoles == [3: "member", 5: "member"])
        // The Smiths' owner is untouched, and so is this account's role there.
        #expect(harness.member(7)?.familyRoles[3] == "owner")
        #expect(harness.session.families.first { $0.id == 3 }?.role == "owner")
    }

    @Test("a note and a sticker are filed under the family their frame names")
    func boardAndPackFrames() async throws {
        let harness = try makeHarness(host: "families-board-frames.test")
        defer { tearDown(harness) }
        await harness.coordinator.resync()

        let note = try APICoding.decoder().decode(NoteDTO.self, from: Data(#"""
        {"id": 777, "author_id": 11, "text": "Dacha", "color": "blue", "x": 0.5, "y": 0.5,
         "board_seq": 99, "family_id": 5}
        """#.utf8))
        harness.coordinator.handle(frame: .boardNote(note))
        #expect(harness.notes().first { $0.id == 777 }?.family == 5)

        let cursorThree = harness.coordinator.packCursor(for: 3)
        harness.coordinator.handle(frame: .packItem(PackItemDTO(id: 5000, packSeq: 77, deleted: true, familyID: 5)))
        #expect(harness.packItems().map(\.id) == [3000])
        #expect(harness.coordinator.packCursor(for: 3) == cursorThree, "another family's frame moved this pack's cursor")
    }

    @Test("a block uses the form without a family: anybody shared in any family")
    func blockHasNoFamily() async throws {
        let harness = try makeHarness(host: "families-block.test")
        defer { tearDown(harness) }
        await harness.coordinator.resync()
        harness.session.switchFamily(to: 5)
        #expect(await harness.coordinator.block(userID: 13))
        #expect(await harness.coordinator.unblock(userID: 13))
        let paths = StubURLProtocol.requests(host: harness.host)
            .filter { $0.url.path().hasSuffix("/block") }.map { "\($0.method) \($0.url.path())" }
        #expect(paths == ["PUT /api/v1/families/members/13/block", "DELETE /api/v1/families/members/13/block"])
    }

    @Test("rows cached before several families are the home family's")
    func adoptsUnattributedRows() throws {
        let harness = try makeHarness(host: "families-adopt.test")
        defer { tearDown(harness) }
        harness.context.insert(NoteEntity(
            noteID: 1, authorID: 7, text: "Old", color: "yellow", x: 0, y: 0,
            createdAt: .now, updatedAt: .now, boardSeq: 1))
        harness.context.insert(MemberEntity(
            userID: 9, username: "kid", displayName: "Kid", role: "member", isCurrentUser: false))
        try harness.context.save()

        harness.session.apply(me: try APICoding.decoder().decode(MeResponse.self, from: Data(Self.meJSON([3, 5]).utf8)))

        #expect(harness.notes().map(\.family) == [3])
        #expect(harness.member(9)?.familyRoles == [3: "member"])
        #expect(harness.member(9)?.hasLeft == false)
    }
}
