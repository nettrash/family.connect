//
//  AccountDeletionTests.swift
//  FamilyConnectTests
//
//  Deleting an account, from the client's side (docs/protocol.md,
//  "Deleting an account"): the two DTO changes it brings (`deleted` on
//  User/Member, an OPTIONAL `role`, and the second `former_members` array
//  on GET /families/mine), the endpoint, and the two frames — one of which
//  is the only frame in the protocol whose job is to WIPE stored fields.
//
//  The session teardown itself lives in AppSessionTests, beside the other
//  purge tests, because it is a phase-machine transition and those tests
//  are serialized against the real UserDefaults and keychain.
//

import Foundation
import SwiftData
import Testing
@testable import FamilyConnect

// MARK: - Wire shapes

@Suite("Account deletion DTOs")
struct AccountDeletionDTOTests {

    private func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
        try APICoding.decoder().decode(type, from: Data(json.utf8))
    }

    @Test("a live member: role present, deleted absent ⇒ false")
    func liveMemberDecodes() throws {
        let member = try decode(MemberDTO.self, #"""
        {"id": 7, "username": "anna", "display_name": "Anna", "role": "owner",
         "avatar_version": 3, "birthday": {"month": 3, "day": 14}}
        """#)
        #expect(member.role == "owner")
        #expect(!member.deleted)
        #expect(member.avatarVersion == 3)
        #expect(member.birthday == BirthdayDTO(month: 3, day: 14))
    }

    @Test("a tombstone member: deleted true, NO role, no birthday, version 0")
    func tombstoneMemberDecodes() throws {
        let member = try decode(MemberDTO.self, #"""
        {"id": 11, "username": "", "display_name": "Deleted account",
         "avatar_version": 0, "deleted": true}
        """#)
        #expect(member.deleted)
        // The whole reason `role` became an Optional: the wire omits it.
        #expect(member.role == nil)
        #expect(member.birthday == nil)
        #expect(member.avatarVersion == 0)
    }

    @Test("UserDTO gains deleted the same way; absent is false")
    func userDeletedFlagDecodes() throws {
        let live = try decode(UserDTO.self, #"""
        {"id": 7, "username": "anna", "display_name": "Anna", "avatar_version": 1}
        """#)
        #expect(!live.deleted)

        let gone = try decode(UserDTO.self, #"""
        {"id": 11, "username": "", "display_name": "Deleted account",
         "avatar_version": 0, "deleted": true}
        """#)
        #expect(gone.deleted)
    }

    @Test("GET /families/mine: former_members is absent on most families ⇒ empty")
    func familyMineWithoutFormerMembers() throws {
        let response = try decode(FamilyMineResponse.self, #"""
        {"family": {"id": 3, "name": "The Smiths", "join_policy": "open",
                    "created_at": "2026-08-01T10:00:00Z"},
         "members": [{"id": 7, "username": "anna", "display_name": "Anna", "role": "owner"}]}
        """#)
        #expect(response.members.count == 1)
        #expect(response.formerMembers.isEmpty)
    }

    @Test("GET /families/mine: former_members decodes as tombstones, separately from members")
    func familyMineWithFormerMembers() throws {
        let response = try decode(FamilyMineResponse.self, #"""
        {"family": {"id": 3, "name": "The Smiths", "join_policy": "open",
                    "created_at": "2026-08-01T10:00:00Z"},
         "members": [{"id": 7, "username": "anna", "display_name": "Anna", "role": "owner"}],
         "former_members": [{"id": 11, "username": "", "display_name": "Deleted account",
                             "avatar_version": 0, "deleted": true}],
         "max_board_seq": 88}
        """#)
        // "Nothing else counts them as members" — including this array.
        #expect(response.members.map(\.id) == [7])
        #expect(response.formerMembers.map(\.id) == [11])
        #expect(response.formerMembers.first?.deleted == true)
        #expect(response.maxBoardSeq == 88)
    }

    @Test("member_deleted and family_owner decode into their own frames")
    func framesDecode() throws {
        let deleted = try decode(ServerFrame.self, #"""
        {"type": "member_deleted", "family_id": 3,
         "member": {"id": 11, "username": "", "display_name": "Deleted account",
                    "avatar_version": 0, "deleted": true}}
        """#)
        guard case .memberDeleted(let payload) = deleted else {
            Issue.record("expected .memberDeleted, got \(deleted)")
            return
        }
        #expect(payload.familyID == 3)
        #expect(payload.member.id == 11)
        #expect(payload.member.deleted)
        #expect(payload.member.role == nil)

        let owner = try decode(ServerFrame.self, #"""
        {"type": "family_owner", "family_id": 3, "user_id": 9}
        """#)
        #expect(owner == .familyOwner(familyID: 3, userID: 9))
    }

    @Test("member_deleted decodes with NO family_id — the account had no family")
    func memberDeletedWithoutFamilyDecodes() throws {
        // The protocol says outright that `family_id` is absent when the
        // deleted account belonged to no family, and that the frame still
        // reaches anybody who only ever shared a DIRECT CHAT with them —
        // somebody in another family entirely, or in none. Required
        // decoding threw on exactly those frames, and an undecodable
        // frame is skipped without a trace, so the peer never learned.
        let frame = try decode(ServerFrame.self, #"""
        {"type": "member_deleted",
         "member": {"id": 11, "username": "", "display_name": "Deleted account",
                    "avatar_version": 0, "deleted": true}}
        """#)
        guard case .memberDeleted(let payload) = frame else {
            Issue.record("expected .memberDeleted, got \(frame)")
            return
        }
        #expect(payload.familyID == nil)
        // Keyed on the MEMBER, never on the family — which is the whole
        // reason the missing id costs nothing.
        #expect(payload.member.id == 11)
        #expect(payload.member.deleted)
    }
}

// MARK: - The endpoint

@Suite("POST /me/delete")
struct DeleteAccountEndpointTests {

    @Test("takes the password in the body and answers 204")
    func deleteAccountRequestShape() async throws {
        let host = "delete-account-api.test"
        StubURLProtocol.register(host: host) { _ in .empty(204) }
        defer { StubURLProtocol.unregister(host: host) }
        let api = APIClient(
            serverURL: URL(string: "https://\(host)")!, session: StubURLProtocol.makeSession())

        try await api.deleteAccount(password: "hunter2hunter2")

        let request = try #require(StubURLProtocol.requests(host: host).first)
        // A POST, not a DELETE /me: the request carries a body.
        #expect(request.method == "POST")
        #expect(request.url.path() == "/api/v1/me/delete")
        #expect(request.bodyJSON()?["password"] as? String == "hunter2hunter2")
    }

    @Test("a wrong password surfaces as .unauthorized for the caller to explain")
    func wrongPasswordIsUnauthorized() async throws {
        let host = "delete-account-api-401.test"
        StubURLProtocol.register(host: host) { _ in
            .json(401, #"{"error": {"code": "invalid_credentials", "message": "wrong password"}}"#)
        }
        defer { StubURLProtocol.unregister(host: host) }
        let api = APIClient(
            serverURL: URL(string: "https://\(host)")!, session: StubURLProtocol.makeSession())

        await #expect(throws: APIError.unauthorized) {
            try await api.deleteAccount(password: "nope")
        }
    }
}

// MARK: - Tombstones in the store

@MainActor
@Suite("Deleted members in the local roster")
struct DeletedMemberStoreTests {

    @MainActor
    private func makeCoordinator(host: String, handler: @escaping StubURLProtocol.Handler) throws
        -> (ChatSyncCoordinator, ModelContainer) {
        StubURLProtocol.register(host: host, handler: handler)
        let container = try ModelContainer(
            for: ChatEntity.self, MessageEntity.self, MemberEntity.self,
            PendingMediaItemEntity.self,
            configurations: ModelConfiguration(isStoredInMemoryOnly: true))
        let api = APIClient(
            serverURL: URL(string: "https://\(host)")!,
            session: StubURLProtocol.makeSession())
        let coordinator = ChatSyncCoordinator(modelContainer: container, api: api)
        coordinator.currentUserIDOverride = 7
        return (coordinator, container)
    }

    @MainActor
    private func member(_ container: ModelContainer, _ userID: Int64) -> MemberEntity? {
        let descriptor = FetchDescriptor<MemberEntity>(predicate: #Predicate { $0.userID == userID })
        return (try? container.mainContext.fetch(descriptor))?.first
    }

    /// The roster every people-listing screen draws (New Chat's predicate,
    /// which the Mac's family screen matches minus the self clause).
    @MainActor
    private func liveRoster(_ container: ModelContainer) -> [MemberEntity] {
        let descriptor = FetchDescriptor<MemberEntity>(
            predicate: #Predicate { !$0.isCurrentUser && !$0.hasLeft && !$0.accountDeleted },
            sortBy: [SortDescriptor(\.userID)])
        return (try? container.mainContext.fetch(descriptor)) ?? []
    }

    /// `nonisolated`: the suite is @MainActor, so a plain `static let`
    /// inherits that isolation and cannot be read from the stub handler,
    /// which runs on URLProtocol's thread. A `String` is Sendable, so
    /// there is nothing to protect.
    private nonisolated static let meJSON = #"""
    {"user": {"id": 7, "username": "anna", "display_name": "Anna",
              "created_at": "2026-08-01T10:00:00Z"},
     "family": {"id": 3, "name": "The Smiths", "join_policy": "open",
                "created_at": "2026-08-01T10:00:00Z"},
     "role": "member"}
    """#

    private func tombstoneFrame(userID: Int64) throws -> ServerFrame {
        try APICoding.decoder().decode(ServerFrame.self, from: Data(#"""
        {"type": "member_deleted", "family_id": 3,
         "member": {"id": \#(userID), "username": "", "display_name": "Deleted account",
                    "avatar_version": 0, "deleted": true}}
        """#.utf8))
    }

    @Test("member_deleted wipes that row's fields — and nobody else's")
    func memberDeletedWritesTheTombstone() throws {
        let host = "member-deleted-frame.test"
        defer { StubURLProtocol.unregister(host: host) }
        let (coordinator, container) = try makeCoordinator(host: host) { _ in .empty(204) }
        container.mainContext.insert(MemberEntity(
            userID: 9, username: "kid", displayName: "Kid", role: "owner",
            isCurrentUser: false, avatarVersion: 4,
            birthday: BirthdayDTO(month: 2, day: 29)))
        container.mainContext.insert(MemberEntity(
            userID: 11, username: "gran", displayName: "Gran", role: "member",
            isCurrentUser: false, avatarVersion: 2,
            birthday: BirthdayDTO(month: 5, day: 1)))
        try container.mainContext.save()

        coordinator.handle(frame: try tombstoneFrame(userID: 9))

        let gone = try #require(member(container, 9))
        #expect(gone.accountDeleted)
        // Not a member of anything any more.
        #expect(gone.hasLeft)
        #expect(gone.role == "")
        // The three things the frame exists to clear.
        #expect(gone.avatarVersion == 0)
        #expect(gone.birthday == nil)
        #expect(gone.displayName == "Deleted account")
        // Drawn translated rather than as the server's English placeholder.
        #expect(gone.resolvedDisplayName == MemberDisplay.deletedAccountName)
        #expect(MemberSnapshot(gone).resolvedDisplayName == MemberDisplay.deletedAccountName)

        // The frame names ONE member; everybody else is untouched.
        let gran = try #require(member(container, 11))
        #expect(!gran.accountDeleted)
        #expect(gran.displayName == "Gran")
        #expect(gran.avatarVersion == 2)
        #expect(gran.birthday == BirthdayDTO(month: 5, day: 1))
        #expect(gran.role == "member")

        // And it is not a roster entry.
        #expect(liveRoster(container).map(\.userID) == [11])
    }

    @Test("a family-less member_deleted still writes the tombstone")
    func memberDeletedWithoutFamilyStillApplies() throws {
        let host = "member-deleted-nofamily.test"
        defer { StubURLProtocol.unregister(host: host) }
        let (coordinator, container) = try makeCoordinator(host: host) { _ in .empty(204) }
        container.mainContext.insert(MemberEntity(
            userID: 21, username: "expeer", displayName: "Ex Peer", role: "member",
            isCurrentUser: false, avatarVersion: 3))

        // The peer of a direct chat, in no family at all: the whole point
        // of the optional id is that this frame arrives and is applied.
        coordinator.handle(frame: try APICoding.decoder().decode(
            ServerFrame.self, from: Data(#"""
            {"type": "member_deleted",
             "member": {"id": 21, "username": "", "display_name": "Deleted account",
                        "avatar_version": 0, "deleted": true}}
            """#.utf8)))

        let row = try #require(member(container, 21))
        #expect(row.accountDeleted)
        #expect(row.avatarVersion == 0)
        #expect(row.resolvedDisplayName == MemberDisplay.deletedAccountName)
    }

    @Test("member_deleted for somebody this device never met still gets a row")
    func memberDeletedInsertsAnUnknownMember() throws {
        let host = "member-deleted-unknown.test"
        defer { StubURLProtocol.unregister(host: host) }
        let (coordinator, container) = try makeCoordinator(host: host) { _ in .empty(204) }

        coordinator.handle(frame: try tombstoneFrame(userID: 42))

        // Their messages may well be in the family chat; without the row
        // those bubbles would read "Someone".
        let inserted = try #require(member(container, 42))
        #expect(inserted.accountDeleted)
        #expect(inserted.resolvedDisplayName == MemberDisplay.deletedAccountName)
        #expect(liveRoster(container).isEmpty)
    }

    @Test("former_members are stored beside members, and are not members")
    func formerMembersAreStoredButNotRostered() async throws {
        let host = "former-members-resync.test"
        defer { StubURLProtocol.unregister(host: host) }
        let (coordinator, container) = try makeCoordinator(host: host) { request in
            switch request.url.path() {
            case "/api/v1/me":
                return .json(200, Self.meJSON)
            case "/api/v1/families/mine":
                return .json(200, #"""
                {"family": {"id": 3, "name": "The Smiths", "join_policy": "open",
                            "created_at": "2026-08-01T10:00:00Z"},
                 "members": [{"id": 7, "username": "anna", "display_name": "Anna", "role": "owner"},
                             {"id": 9, "username": "kid", "display_name": "Kid", "role": "member"}],
                 "former_members": [{"id": 11, "username": "", "display_name": "Deleted account",
                                     "avatar_version": 0, "deleted": true}]}
                """#)
            case "/api/v1/chats":
                return .json(200, #"{"chats": []}"#)
            default:
                return .empty(204)
            }
        }

        await coordinator.resync()

        // Stored, so a message from 11 can still be given a sender…
        let former = try #require(member(container, 11))
        #expect(former.accountDeleted)
        #expect(former.hasLeft)
        #expect(former.avatarVersion == 0)
        // …and drawn nowhere that lists people.
        #expect(liveRoster(container).map(\.userID) == [9])
        #expect(member(container, 9)?.accountDeleted == false)
    }

    @Test("a stale roster response cannot resurrect a tombstone")
    func tombstoneSurvivesAStaleRosterRead() async throws {
        let host = "tombstone-stale-roster.test"
        defer { StubURLProtocol.unregister(host: host) }
        // A GET /families/mine that was already in flight when the member
        // deleted their account: it still lists them, with their real name
        // and picture.
        let (coordinator, container) = try makeCoordinator(host: host) { request in
            switch request.url.path() {
            case "/api/v1/me":
                return .json(200, Self.meJSON)
            case "/api/v1/families/mine":
                return .json(200, #"""
                {"family": {"id": 3, "name": "The Smiths", "join_policy": "open",
                            "created_at": "2026-08-01T10:00:00Z"},
                 "members": [{"id": 7, "username": "anna", "display_name": "Anna", "role": "owner"},
                             {"id": 9, "username": "kid", "display_name": "Kid",
                              "role": "member", "avatar_version": 4,
                              "birthday": {"month": 2, "day": 29}}]}
                """#)
            case "/api/v1/chats":
                return .json(200, #"{"chats": []}"#)
            default:
                return .empty(204)
            }
        }

        coordinator.handle(frame: try tombstoneFrame(userID: 9))
        await coordinator.resync()

        let gone = try #require(member(container, 9))
        #expect(gone.accountDeleted)
        #expect(gone.displayName == "Deleted account")
        #expect(gone.avatarVersion == 0)
        #expect(gone.birthday == nil)
        #expect(liveRoster(container).isEmpty)
    }

    @Test("family_owner moves the owner badge on the stored roster")
    func familyOwnerFrameUpdatesTheRoster() throws {
        let host = "family-owner-frame.test"
        defer { StubURLProtocol.unregister(host: host) }
        let (coordinator, container) = try makeCoordinator(host: host) { _ in .empty(204) }
        container.mainContext.insert(MemberEntity(
            userID: 11, username: "gran", displayName: "Gran", role: "owner",
            isCurrentUser: false))
        container.mainContext.insert(MemberEntity(
            userID: 9, username: "kid", displayName: "Kid", role: "member",
            isCurrentUser: false))
        try container.mainContext.save()

        // The owner deleted their account; ownership passed to the
        // longest-standing remaining member.
        coordinator.handle(frame: try tombstoneFrame(userID: 11))
        coordinator.handle(frame: try APICoding.decoder().decode(
            ServerFrame.self, from: Data(#"{"type": "family_owner", "family_id": 3, "user_id": 9}"#.utf8)))

        #expect(member(container, 9)?.role == "owner")
        // The old owner is a tombstone and holds no role at all.
        #expect(member(container, 11)?.role == "")
        #expect(member(container, 11)?.accountDeleted == true)
    }
}

// MARK: - Drawing a tombstone

@Suite("Deleted account naming")
struct DeletedMemberNamingTests {

    @Test("a deleted row draws the app's own translation, not the server's placeholder")
    func resolvedNameForTombstone() {
        let live = MemberSnapshot(
            userID: 9, username: "kid", displayName: "Kid", role: "member",
            isCurrentUser: false, hasLeft: false)
        #expect(live.resolvedDisplayName == "Kid")

        let gone = MemberSnapshot(
            userID: 11, username: "", displayName: "Deleted account", role: "",
            isCurrentUser: false, hasLeft: true, avatarVersion: 0, accountDeleted: true)
        #expect(gone.resolvedDisplayName == MemberDisplay.deletedAccountName)
    }

    @Test("a tombstone's dto carries no role and the deleted flag")
    func tombstoneEntityDTO() {
        let entity = MemberEntity(
            userID: 11, username: "", displayName: "Deleted account", role: "",
            isCurrentUser: false, hasLeft: true, accountDeleted: true)
        #expect(entity.dto.role == nil)
        #expect(entity.dto.deleted)
        #expect(entity.dto.avatarVersion == 0)
    }
}

// MARK: - The chats a deleted account takes with it

/// Account deletion is the FIRST thing in this protocol that can make a
/// chat genuinely vanish. Leaving a family does not — that history is
/// retained and resurfaces on rejoin — so nothing ever had to prune
/// before, and the two halves of the fix are tested here together
/// because they are one primitive with two triggers (docs/protocol.md,
/// "Deleting an account"):
///
///   the GENERAL REPAIR — a full `GET /chats` that succeeded and does not
///   list a direct chat means that chat is gone, which is the only thing
///   that can heal a device that was asleep when it happened;
///   the IMMEDIATE one — the `member_deleted` frame, so the member
///   watching their chat list sees the row go rather than a row under the
///   peer's old name that answers 404 to everything.
///
/// The negatives are the load-bearing half: a FAILED read prunes nothing,
/// and neither the family chat nor the assistant thread is ever pruned.
@MainActor
@Suite("Chats a deleted account takes with it")
struct DeletedAccountChatPruneTests {

    private static let sentAt = ISO8601DateFormatter().date(from: "2026-08-19T17:05:00Z")!

    /// `nonisolated`: read from the stub handler, which runs on
    /// URLProtocol's thread. Strings are Sendable.
    private nonisolated static let meJSON = #"""
    {"user": {"id": 7, "username": "anna", "display_name": "Anna",
              "created_at": "2026-08-01T10:00:00Z"},
     "family": {"id": 3, "name": "The Smiths", "join_policy": "open",
                "created_at": "2026-08-01T10:00:00Z"},
     "role": "member"}
    """#

    private nonisolated static let familyJSON = #"""
    {"family": {"id": 3, "name": "The Smiths", "join_policy": "open",
                "created_at": "2026-08-01T10:00:00Z"},
     "members": [{"id": 7, "username": "anna", "display_name": "Anna", "role": "owner"},
                 {"id": 9, "username": "kid", "display_name": "Kid", "role": "member"}]}
    """#

    /// One `GET /chats` body listing exactly the chats named — the whole
    /// point being what it does NOT list. The endpoint is unpaginated, so
    /// one body is a complete answer.
    private nonisolated static func chatsJSON(_ ids: [Int64]) -> String {
        let items: [String] = ids.map { id in
            switch id {
            case 1: return #"{"chat": {"id": 1, "kind": "family", "title": "The Smiths", "peer_user_id": null}, "last_message": null, "unread_count": 0}"#
            case 2: return #"{"chat": {"id": 2, "kind": "ai", "title": "Assistant", "peer_user_id": null}, "last_message": null, "unread_count": 0}"#
            case 42: return #"{"chat": {"id": 42, "kind": "direct", "title": "Gran", "peer_user_id": 11}, "last_message": null, "unread_count": 0}"#
            default: return #"{"chat": {"id": 43, "kind": "direct", "title": "Kid", "peer_user_id": 9}, "last_message": null, "unread_count": 0}"#
            }
        }
        return #"{"chats": [\#(items.joined(separator: ", "))]}"#
    }

    @MainActor
    private struct Harness {
        /// Retained: the coordinator holds only the mainContext, and
        /// SwiftData traps if the container backing a context goes away.
        let container: ModelContainer
        let coordinator: ChatSyncCoordinator
        let context: ModelContext
        let host: String

        func chatIDs() -> [Int64] {
            ((try? context.fetch(FetchDescriptor<ChatEntity>())) ?? []).map(\.chatID).sorted()
        }

        /// EVERY message row, keyed by the chat it belongs to and fetched
        /// with NO chat predicate — which is the orphan check. A chat row
        /// deleted on its own leaves its messages behind, invisible to
        /// every screen and still in the store.
        func messageChatIDs() -> [Int64] {
            ((try? context.fetch(FetchDescriptor<MessageEntity>())) ?? []).map(\.chatID).sorted()
        }

        func member(_ userID: Int64) -> MemberEntity? {
            let descriptor = FetchDescriptor<MemberEntity>(predicate: #Predicate { $0.userID == userID })
            return (try? context.fetch(descriptor))?.first
        }

        func tearDown() {
            StubURLProtocol.unregister(host: host)
        }
    }

    /// The store every test here starts from: the family chat, this
    /// member's own assistant thread, and two direct chats — one with the
    /// peer who is about to delete their account (42, peer 11) and one
    /// with somebody who is not (43, peer 9). Every chat holds a message,
    /// so pruning can be shown to take the right ones with it.
    private func makeHarness(host: String, handler: @escaping StubURLProtocol.Handler) throws -> Harness {
        StubURLProtocol.register(host: host, handler: handler)
        let container = try ModelContainer(
            for: ChatEntity.self, MessageEntity.self, MemberEntity.self,
            PendingMediaItemEntity.self,
            configurations: ModelConfiguration(isStoredInMemoryOnly: true))
        let api = APIClient(
            serverURL: URL(string: "https://\(host)")!,
            session: StubURLProtocol.makeSession())
        let coordinator = ChatSyncCoordinator(modelContainer: container, api: api)
        coordinator.currentUserIDOverride = 7

        let context = container.mainContext
        context.insert(ChatEntity(chatID: 1, kind: "family", pinRank: 0, title: "The Smiths"))
        context.insert(ChatEntity(chatID: 2, kind: "ai", pinRank: 1, title: "Assistant"))
        context.insert(ChatEntity(
            chatID: 42, kind: "direct", pinRank: 1, peerUserID: 11, title: "Gran",
            unreadCount: 4))
        context.insert(ChatEntity(
            chatID: 43, kind: "direct", pinRank: 1, peerUserID: 9, title: "Kid"))
        for (index, chatID) in [1, 2, 42, 43].enumerated() {
            context.insert(MessageEntity(
                localID: "s:\(100 + index)", serverID: Int64(100 + index),
                chatID: Int64(chatID), senderID: 11, body: "hello",
                createdAt: Self.sentAt, status: .sent))
        }
        context.insert(MemberEntity(
            userID: 11, username: "gran", displayName: "Gran", role: "member",
            isCurrentUser: false, avatarVersion: 2))
        context.insert(MemberEntity(
            userID: 9, username: "kid", displayName: "Kid", role: "member",
            isCurrentUser: false))
        try context.save()
        return Harness(container: container, coordinator: coordinator, context: context, host: host)
    }

    private func tombstoneFrame(userID: Int64, familyID: Int64? = 3) throws -> ServerFrame {
        let family = familyID.map { #""family_id": \#($0), "# } ?? ""
        return try APICoding.decoder().decode(ServerFrame.self, from: Data(#"""
        {"type": "member_deleted", \#(family)
         "member": {"id": \#(userID), "username": "", "display_name": "Deleted account",
                    "avatar_version": 0, "deleted": true}}
        """#.utf8))
    }

    // MARK: - The general repair

    @Test("a successful list that omits a direct chat prunes it AND its messages")
    func successfulListPrunesTheAbsentDirectChat() async throws {
        let host = "prune-absent-direct.test"
        let harness = try makeHarness(host: host) { request in
            switch request.url.path() {
            case "/api/v1/me": return .json(200, Self.meJSON)
            case "/api/v1/families/mine": return .json(200, Self.familyJSON)
            // 42 is gone: its peer deleted their account while this
            // device was asleep, so no frame was ever delivered.
            case "/api/v1/chats": return .json(200, Self.chatsJSON([1, 2, 43]))
            default: return .json(200, #"{"messages": []}"#)
            }
        }
        defer { harness.tearDown() }

        await harness.coordinator.resync()

        #expect(harness.chatIDs() == [1, 2, 43])
        // The messages went with it, and nothing else did.
        #expect(harness.messageChatIDs() == [1, 2, 43])
        // The roster row STAYS: their messages are still in the family
        // chat and still have to be given a name.
        #expect(harness.member(11) != nil)
    }

    @Test("a FAILED list prunes nothing — not an error, not a torn body")
    func failedListPrunesNothing() async throws {
        let host = "prune-failed-list.test"
        // A server that is having a bad day: the chat list 500s. Every
        // other step still answers, so the only difference from the test
        // above is that this response is not an answer.
        let harness = try makeHarness(host: host) { request in
            switch request.url.path() {
            case "/api/v1/me": return .json(200, Self.meJSON)
            case "/api/v1/families/mine": return .json(200, Self.familyJSON)
            case "/api/v1/chats":
                return .json(500, #"{"error": {"code": "internal", "message": "…"}}"#)
            default: return .json(200, #"{"messages": []}"#)
            }
        }
        defer { harness.tearDown() }

        await harness.coordinator.resync()

        // Everything survives. A flaky connection that pruned would be
        // somebody's history gone for a dropped packet.
        #expect(harness.chatIDs() == [1, 2, 42, 43])
        #expect(harness.messageChatIDs() == [1, 2, 42, 43])
    }

    @Test("a truncated chat list prunes nothing either")
    func tornListPrunesNothing() async throws {
        let host = "prune-torn-list.test"
        // 200, and the body is the FIRST HALF of a list that would have
        // omitted 42. A response that cannot be read is not evidence
        // about what the server holds.
        let harness = try makeHarness(host: host) { request in
            switch request.url.path() {
            case "/api/v1/me": return .json(200, Self.meJSON)
            case "/api/v1/families/mine": return .json(200, Self.familyJSON)
            case "/api/v1/chats":
                return .json(200, #"{"chats": [{"chat": {"id": 1, "kind": "family","#)
            default: return .json(200, #"{"messages": []}"#)
            }
        }
        defer { harness.tearDown() }

        await harness.coordinator.resync()

        #expect(harness.chatIDs() == [1, 2, 42, 43])
        #expect(harness.messageChatIDs() == [1, 2, 42, 43])
    }

    @Test("the family chat and the assistant chat are NEVER pruned")
    func onlyDirectChatsAreEverPruned() async throws {
        let host = "prune-only-direct.test"
        // A response listing neither the family chat nor the assistant
        // thread — which the server cannot really send, and that is the
        // point: a client must not act on it if it does. The direct chat
        // in the same body IS pruned, so this is not just an inert run.
        let harness = try makeHarness(host: host) { request in
            switch request.url.path() {
            case "/api/v1/me": return .json(200, Self.meJSON)
            case "/api/v1/families/mine": return .json(200, Self.familyJSON)
            case "/api/v1/chats": return .json(200, Self.chatsJSON([43]))
            default: return .json(200, #"{"messages": []}"#)
            }
        }
        defer { harness.tearDown() }

        await harness.coordinator.resync()

        #expect(harness.chatIDs() == [1, 2, 43])
        #expect(harness.messageChatIDs() == [1, 2, 43])
    }

    // MARK: - The frame

    @Test("member_deleted drops the direct chat with that peer, there and then")
    func memberDeletedDropsTheDirectChat() throws {
        let host = "member-deleted-drops-chat.test"
        let harness = try makeHarness(host: host) { _ in .empty(204) }
        defer { harness.tearDown() }

        harness.coordinator.handle(frame: try tombstoneFrame(userID: 11))

        // Gran's chat and everything in it, and NOTHING else: the family
        // chat keeps its history (the protocol retains it), the other
        // direct chat is not theirs, and the assistant thread is mine.
        #expect(harness.chatIDs() == [1, 2, 43])
        #expect(harness.messageChatIDs() == [1, 2, 43])
        // Written first and untouched by the prune — the tombstone is
        // what puts a name on their messages in the family chat.
        let gone = try #require(harness.member(11))
        #expect(gone.accountDeleted)
        #expect(gone.resolvedDisplayName == MemberDisplay.deletedAccountName)
    }

    @Test("a family-less member_deleted drops the chat too — that peer is why it was sent")
    func memberDeletedWithoutFamilyDropsTheDirectChat() throws {
        let host = "member-deleted-nofamily-chat.test"
        let harness = try makeHarness(host: host) { _ in .empty(204) }
        defer { harness.tearDown() }

        // A direct chat outliving the family that created it: the frame
        // arrives with no family_id at all, and it is still the only
        // thing that will ever say why the chat is about to answer 404.
        harness.coordinator.handle(frame: try tombstoneFrame(userID: 11, familyID: nil))

        #expect(harness.chatIDs() == [1, 2, 43])
        #expect(harness.messageChatIDs() == [1, 2, 43])
    }

    @Test("member_deleted for somebody this device shares no direct chat with changes no chat")
    func memberDeletedWithoutADirectChatPrunesNothing() throws {
        let host = "member-deleted-no-chat.test"
        let harness = try makeHarness(host: host) { _ in .empty(204) }
        defer { harness.tearDown() }

        // Somebody in the family this device never messaged privately.
        harness.coordinator.handle(frame: try tombstoneFrame(userID: 21))

        #expect(harness.chatIDs() == [1, 2, 42, 43])
        #expect(harness.messageChatIDs() == [1, 2, 42, 43])
        #expect(harness.member(21)?.accountDeleted == true)
    }
}
