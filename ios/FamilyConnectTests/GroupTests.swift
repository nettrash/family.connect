//
//  GroupTests.swift
//  FamilyConnectTests
//
//  Groups (docs/protocol.md, "Groups", 2026-10-22; the decisions are
//  docs/groups-2026-10-21.md): the rules table — what a group offers beside
//  the family chat, a direct chat and the assistant's, and who may manage
//  one — the wire fields, the five endpoints, the two frames, the prune of
//  any chat `GET /chats` stops listing, and the notification title.
//
//  The table is the point. Every view asks `ChatRules` and `GroupRules`
//  rather than spelling a kind out, so a row here that changes is a screen
//  that changes everywhere at once — and a group that quietly lost its poll
//  button, or gained a call button, fails here first.
//

import Foundation
import SwiftData
import Testing
@testable import FamilyConnect

// MARK: - What each kind offers

@Suite("Group rules: what a chat offers")
struct ChatRulesTableTests {

    /// One row per kind: [family/group surface, sender names, mentions,
    /// polls, @ai, recordings (voice + the video door), calls, seen tick].
    private static let table: [(kind: String?, row: [Bool])] = [
        ("family", [true, true, true, true, true, true, false, false]),
        ("group", [true, true, true, true, true, true, false, false]),
        ("direct", [false, false, false, false, false, true, true, true]),
        ("ai", [false, false, false, false, false, false, false, true]),
        // A chat this device does not hold yet, and a kind from the future:
        // nothing of the family chat's, and no call.
        (nil, [false, false, false, false, false, true, false, true]),
        ("someday", [false, false, false, false, false, true, false, true]),
    ]

    @Test("each kind's offers, as one table", arguments: table.indices)
    func offers(_ index: Int) {
        let (kind, expected) = Self.table[index]
        let actual = [
            ChatRules.isFamilyOrGroup(kind),
            ChatRules.showsSenderNames(kind),
            ChatRules.offersMemberMentions(kind),
            ChatRules.offersPolls(kind),
            ChatRules.offersAssistantMention(kind),
            ChatRules.offersRecordings(kind),
            ChatRules.offersCalls(kind),
            ChatRules.drawsSeen(kind),
        ]
        #expect(actual == expected, "kind \(kind ?? "nil")")
    }

    @Test("transcripts: a group follows the DIRECT chat's rule (D4)")
    func transcriptRule() {
        #expect(ChatRules.transcriptRule("group") == .sendersConsent)
        #expect(ChatRules.transcriptRule("direct") == .sendersConsent)
        #expect(ChatRules.transcriptRule("family") == .ownersSwitch)
        #expect(ChatRules.transcriptRule("ai") == .never)
        #expect(ChatRules.transcriptRule(nil) == .never)
    }

    @Test("the door: another member's recording in a group, whatever the family switch")
    func transcriptDoorInAGroup() {
        let me: Int64 = 7, other: Int64 = 9, assistant: Int64 = 1
        for familyAllows in [true, false] {
            #expect(TranscriptDoor.allows(
                chatKind: "group", senderID: other, currentUserID: me,
                assistantUserID: assistant, familyAllowsTranscripts: familyAllows))
        }
        // "The other person" needs a reader who is known, as in a direct chat.
        #expect(!TranscriptDoor.allows(
            chatKind: "group", senderID: other, currentUserID: -1,
            assistantUserID: assistant, familyAllowsTranscripts: true))
        // Never the assistant's own answer.
        #expect(!TranscriptDoor.allows(
            chatKind: "group", senderID: assistant, currentUserID: me,
            assistantUserID: assistant, familyAllowsTranscripts: true))
        // The family chat still needs the switch.
        #expect(!TranscriptDoor.allows(
            chatKind: "family", senderID: other, currentUserID: me,
            assistantUserID: assistant, familyAllowsTranscripts: false))
    }

    @Test("@ai in a group reaches the model; plain words do not")
    func assistantMentionInAGroup() {
        #expect(AssistantConsent.reachesTheModel(chatKind: "group", body: "@ai what's for dinner?"))
        #expect(!AssistantConsent.reachesTheModel(chatKind: "group", body: "what's for dinner?"))
        #expect(!AssistantConsent.reachesTheModel(chatKind: "direct", body: "@ai hi"))
        #expect(AssistantConsent.isRequired(
            chatKind: "group", body: "@ai hi", processor: "Somebody", agreedAt: nil))
    }

    @Test("the paperclip in a group: Ask the Assistant and Poll, like the family chat")
    func attachMenuInAGroup() {
        let groups = AttachMenu.groups(
            isAssistantChat: false, showsPictureAttach: false, hasCamera: true,
            offersVoice: true, roundAvailable: true,
            isFamilyOrGroup: ChatRules.isFamilyOrGroup("group"), offersAssistant: true)
        #expect(groups.last == [.askAssistant, .location, .poll])
        let direct = AttachMenu.groups(
            isAssistantChat: false, showsPictureAttach: false, hasCamera: true,
            offersVoice: true, roundAvailable: true,
            isFamilyOrGroup: ChatRules.isFamilyOrGroup("direct"), offersAssistant: true)
        #expect(direct.last == [.location])
    }

    @Test("sender names show and the seen tick never does, in a group")
    func bubblesInAGroup() {
        let section = [
            MessageSnapshot(
                localID: "a", serverID: 10, chatID: 77, senderID: 9, body: "hi",
                createdAt: .now, state: .sent),
            MessageSnapshot(
                localID: "b", serverID: 11, chatID: 77, senderID: 7, body: "yo",
                createdAt: .now, state: .sent),
        ]
        let isGroup = ChatRules.isFamilyOrGroup("group")
        #expect(MessagePresentation.showsSenderName(
            at: 0, in: section, isFamilyOrGroup: isGroup, currentUserID: 7, blockedUserIDs: []))
        #expect(!MessagePresentation.isRead(section[1], othersReadUpTo: 99, isFamilyOrGroup: isGroup))
    }

    @Test("mentions in a group name its members only; the family chat names everybody")
    func mentionRoster() {
        let roster = [
            MentionDTO(userID: 7, name: "Anna"),
            MentionDTO(userID: 9, name: "Kid"),
            MentionDTO(userID: 11, name: "Gran"),
        ]
        #expect(ChatRules.mentionRoster(roster, kind: "group", groupMemberIDs: [7, 11]).map(\.userID) == [7, 11])
        #expect(ChatRules.mentionRoster(roster, kind: "family", groupMemberIDs: []) == roster)
        // The reader and the blocked go later, exactly as in the family
        // chat; the assistant still leads the strip.
        let strip = MemberMentions.suggestions(
            in: ChatRules.mentionRoster(roster, kind: "group", groupMemberIDs: [7, 9, 11]),
            matching: "", excluding: [7, 11], assistantName: "Assistant")
        #expect(strip.first == .assistant(name: "Assistant"))
        #expect(strip.dropFirst().map(\.id) == [9])
    }

    @Test("a poll's denominator in a group is the group")
    func pollAudience() {
        #expect(ChatRules.pollAudience(kind: "group", groupMemberIDs: [7, 9, 11], liveFamilyMembers: 6) == 3)
        #expect(ChatRules.pollAudience(kind: "family", groupMemberIDs: [], liveFamilyMembers: 6) == 6)
    }

    @Test("a group's banner is titled like the family chat's")
    func notificationTitle() {
        #expect(ChatNotifier.title(chatKind: "group", chatTitle: "Cousins", senderName: "Anna")
            == "Cousins — Anna")
        #expect(ChatNotifier.title(chatKind: "direct", chatTitle: "Anna", senderName: "Anna") == "Anna")
        let mentioned = ChatNotifier.title(
            chatKind: "group", chatTitle: "Cousins", senderName: "Anna", namesMe: true)
        #expect(mentioned.contains("Cousins"))
        #expect(mentioned.contains("Anna"))
        #expect(mentioned != "Cousins — Anna")
    }
}

// MARK: - Who may manage one

@Suite("Group rules: who may do what")
struct GroupPermissionTests {
    private static let admin: Int64 = 7
    private static let member: Int64 = 9
    private static let outsider: Int64 = 11
    private static let members: [Int64] = [7, 9]

    @Test("the admin does everything; a member only leaves; an outsider nothing")
    func table() {
        for action in GroupRules.Action.allCases {
            #expect(GroupRules.may(action, userID: Self.admin, adminUserID: Self.admin, memberIDs: Self.members))
            #expect(GroupRules.may(action, userID: Self.member, adminUserID: Self.admin, memberIDs: Self.members)
                == (action == .leave), "member, \(action)")
            #expect(!GroupRules.may(action, userID: Self.outsider, adminUserID: Self.admin, memberIDs: Self.members))
        }
        // An unknown reader is nobody.
        #expect(!GroupRules.may(.leave, userID: -1, adminUserID: Self.admin, memberIDs: [-1]))
        // No admin known (yet): nobody is offered the admin's controls.
        #expect(!GroupRules.may(.rename, userID: Self.admin, adminUserID: nil, memberIDs: Self.members))
    }

    @Test("the admin removes others, never themself and never a stranger")
    func removal() {
        #expect(GroupRules.mayRemove(Self.member, userID: Self.admin, adminUserID: Self.admin, memberIDs: Self.members))
        #expect(!GroupRules.mayRemove(Self.admin, userID: Self.admin, adminUserID: Self.admin, memberIDs: Self.members))
        #expect(!GroupRules.mayRemove(Self.outsider, userID: Self.admin, adminUserID: Self.admin, memberIDs: Self.members))
        #expect(!GroupRules.mayRemove(Self.admin, userID: Self.member, adminUserID: Self.admin, memberIDs: Self.members))
    }

    @Test("a name is trimmed and then 1 to 64 characters")
    func names() {
        #expect(GroupRules.validTitle("  Cousins \n") == "Cousins")
        #expect(GroupRules.validTitle("") == nil)
        #expect(GroupRules.validTitle("   ") == nil)
        #expect(GroupRules.validTitle(String(repeating: "a", count: 64)) != nil)
        #expect(GroupRules.validTitle(String(repeating: "a", count: 65)) == nil)
        // Counted as the server counts — Unicode scalars, not bytes.
        #expect(GroupRules.validTitle(String(repeating: "я", count: 64)) != nil)
    }

    @Test("the New Group form: the name first, then at least one member")
    func creationProblems() {
        #expect(GroupRules.creationProblem(title: " ", memberIDs: [9])
            == String(localized: "A group's name can be 1 to 64 characters."))
        #expect(GroupRules.creationProblem(title: "Cousins", memberIDs: [])
            == String(localized: "Choose at least one member."))
        #expect(GroupRules.creationProblem(title: "", memberIDs: [])
            == String(localized: "A group's name can be 1 to 64 characters."))
        #expect(GroupRules.creationProblem(title: "Cousins", memberIDs: [9]) == nil)
    }

    @Test("refusals are said; a transient failure is not one")
    func failures() {
        #expect(GroupRules.createFailure(APIError.conflict(code: "too_many_groups", message: nil))
            == String(localized: "Your family already has 50 groups."))
        #expect(GroupRules.createFailure(APIError.conflict(code: "validation", message: nil))
            == String(localized: "A group's name can be 1 to 64 characters."))
        #expect(GroupRules.createFailure(APIError.conflict(code: "not_same_family", message: nil))
            == String(localized: "Couldn't create the group."))
        #expect(GroupRules.changeFailure(APIError.forbidden(code: "not_group_admin"))
            == String(localized: "Only the group's admin can do this."))
        #expect(GroupRules.changeFailure(APIError.conflict(code: "validation", message: nil))
            == String(localized: "Couldn't change the group."))
        let unreachable = String(localized: "Can't reach the server. Try again.")
        for transient in [APIError.transport(URLError(.timedOut)), .throttled(retryAfter: 2),
                          .server(status: 500, message: nil)] {
            #expect(GroupRules.createFailure(transient) == unreachable)
            #expect(GroupRules.changeFailure(transient) == unreachable)
        }
    }
}

// MARK: - The wire

@Suite("Group wire shapes")
struct GroupWireTests {
    private func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
        try APICoding.decoder().decode(type, from: Data(json.utf8))
    }

    @Test("a group's Chat carries its members and admin; other kinds carry neither")
    func chatDecodes() throws {
        let group = try decode(ChatDTO.self, #"""
        {"id": 77, "kind": "group", "title": "Cousins", "peer_user_id": null,
         "member_ids": [7, 9, 11], "admin_user_id": 7}
        """#)
        #expect(group.kind == "group")
        #expect(group.memberIDs == [7, 9, 11])
        #expect(group.adminUserID == 7)
        #expect(group.peerUserID == nil)

        let family = try decode(ChatDTO.self, #"""
        {"id": 1, "kind": "family", "title": "The Smiths", "peer_user_id": null}
        """#)
        #expect(family.memberIDs == nil)
        #expect(family.adminUserID == nil)
    }

    @Test("chat_updated and chat_removed decode into their own frames")
    func framesDecode() throws {
        let updated = try decode(ServerFrame.self, #"""
        {"type": "chat_updated",
         "chat": {"id": 77, "kind": "group", "title": "Cousins", "peer_user_id": null,
                  "member_ids": [7, 9], "admin_user_id": 9}}
        """#)
        guard case .chatUpdated(let chat) = updated else {
            Issue.record("expected .chatUpdated, got \(updated)")
            return
        }
        #expect(chat.id == 77)
        #expect(chat.memberIDs == [7, 9])
        #expect(chat.adminUserID == 9)

        let removed = try decode(ServerFrame.self, #"{"type": "chat_removed", "chat_id": 77}"#)
        #expect(removed == .chatRemoved(chatID: 77))
    }

    @Test("the five endpoints: methods, paths and bodies")
    func endpoints() async throws {
        let host = "group-endpoints.test"
        let chatJSON = #"{"chat": {"id": 77, "kind": "group", "title": "Cousins", "peer_user_id": null, "member_ids": [7, 9], "admin_user_id": 7}}"#
        StubURLProtocol.register(host: host) { request in
            switch (request.method, request.url.path()) {
            case ("POST", "/api/v1/chats/groups"): return .json(201, chatJSON)
            case ("DELETE", "/api/v1/chats/77/members/7"), ("DELETE", "/api/v1/chats/77"): return .empty(204)
            default: return .json(200, chatJSON)
            }
        }
        defer { StubURLProtocol.unregister(host: host) }
        let api = APIClient(serverURL: URL(string: "https://\(host)")!, session: StubURLProtocol.makeSession())

        let created = try await api.createGroup(title: "Cousins", memberIDs: [9])
        #expect(created.id == 77)
        _ = try await api.renameGroup(chatID: 77, title: "The Cousins")
        _ = try await api.addGroupMembers(chatID: 77, userIDs: [11])
        _ = try await api.removeGroupMember(chatID: 77, userID: 9)
        try await api.leaveGroup(chatID: 77, userID: 7)
        try await api.deleteGroup(chatID: 77)

        let requests = StubURLProtocol.requests(host: host)
        #expect(requests.map(\.method) == ["POST", "PATCH", "POST", "DELETE", "DELETE", "DELETE"])
        #expect(requests.map { $0.url.path() } == [
            "/api/v1/chats/groups", "/api/v1/chats/77", "/api/v1/chats/77/members",
            "/api/v1/chats/77/members/9", "/api/v1/chats/77/members/7", "/api/v1/chats/77",
        ])
        #expect(requests[0].bodyJSON()?["title"] as? String == "Cousins")
        #expect(requests[0].bodyJSON()?["member_ids"] as? [Int] == [9])
        #expect(requests[1].bodyJSON()?["title"] as? String == "The Cousins")
        #expect(requests[2].bodyJSON()?["user_ids"] as? [Int] == [11])
    }
}

// MARK: - The store

@MainActor
@Suite("Groups in the local store")
struct GroupStoreTests {

    private static let sentAt = ISO8601DateFormatter().date(from: "2026-10-22T10:00:00Z")!

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

    @MainActor
    private struct Harness {
        /// Retained: SwiftData traps if the container behind a context goes.
        let container: ModelContainer
        let coordinator: ChatSyncCoordinator
        let context: ModelContext
        let host: String

        func chat(_ chatID: Int64) -> ChatEntity? {
            let descriptor = FetchDescriptor<ChatEntity>(predicate: #Predicate { $0.chatID == chatID })
            return (try? context.fetch(descriptor))?.first
        }

        func chatIDs() -> [Int64] {
            ((try? context.fetch(FetchDescriptor<ChatEntity>())) ?? []).map(\.chatID).sorted()
        }

        /// Every message, with no chat predicate — the orphan check.
        func messageChatIDs() -> [Int64] {
            ((try? context.fetch(FetchDescriptor<MessageEntity>())) ?? []).map(\.chatID).sorted()
        }

        func tearDown() { StubURLProtocol.unregister(host: host) }
    }

    /// The family chat (1), a direct chat (43) and a group (77), each with a
    /// message in it.
    private func makeHarness(
        host: String, handler: @escaping StubURLProtocol.Handler = { _ in .empty(204) }
    ) throws -> Harness {
        StubURLProtocol.register(host: host, handler: handler)
        let container = try ModelContainer(
            for: ChatEntity.self, MessageEntity.self, MemberEntity.self,
            PendingMediaItemEntity.self,
            configurations: ModelConfiguration(isStoredInMemoryOnly: true))
        let api = APIClient(serverURL: URL(string: "https://\(host)")!, session: StubURLProtocol.makeSession())
        let coordinator = ChatSyncCoordinator(modelContainer: container, api: api)
        coordinator.currentUserIDOverride = 7

        let context = container.mainContext
        context.insert(ChatEntity(chatID: 1, kind: "family", pinRank: 0, title: "The Smiths"))
        context.insert(ChatEntity(chatID: 43, kind: "direct", pinRank: 1, peerUserID: 9, title: "Kid"))
        let group = ChatEntity(chatID: 77, kind: "group", pinRank: 1, title: "Cousins", unreadCount: 2)
        group.groupMemberIDs = [7, 9]
        group.groupAdminUserID = 7
        context.insert(group)
        for (index, chatID) in [1, 43, 77].enumerated() {
            context.insert(MessageEntity(
                localID: "s:\(100 + index)", serverID: Int64(100 + index),
                chatID: Int64(chatID), senderID: 9, body: "hello",
                createdAt: Self.sentAt, status: .sent))
        }
        try context.save()
        return Harness(container: container, coordinator: coordinator, context: context, host: host)
    }

    private func frame(_ json: String) throws -> ServerFrame {
        try APICoding.decoder().decode(ServerFrame.self, from: Data(json.utf8))
    }

    @Test("the members column round-trips the server's order")
    func memberIDsRoundTrip() {
        let chat = ChatEntity(chatID: 5, kind: "group", pinRank: 1, title: "G")
        #expect(chat.groupMemberIDs.isEmpty)
        chat.groupMemberIDs = [11, 7, 9]
        #expect(chat.groupMemberIDs == [11, 7, 9])
        chat.groupMemberIDs = []
        #expect(chat.groupMemberIDsText.isEmpty)
    }

    @Test("chat_updated creates a group this device has never held — at once, below the family chat")
    func chatUpdatedCreates() throws {
        let harness = try makeHarness(host: "group-updated-new.test")
        defer { harness.tearDown() }

        harness.coordinator.handle(frame: try frame(#"""
        {"type": "chat_updated",
         "chat": {"id": 88, "kind": "group", "title": "Book club", "peer_user_id": null,
                  "member_ids": [9, 7], "admin_user_id": 9}}
        """#))

        let chat = try #require(harness.chat(88))
        #expect(chat.kind == "group")
        #expect(chat.title == "Book club")
        #expect(chat.pinRank == 1)
        #expect(chat.groupMemberIDs == [9, 7])
        #expect(chat.groupAdminUserID == 9)
        #expect(chat.unreadCount == 0)
    }

    @Test("chat_updated renames and re-members a held group, and leaves its messages' state alone")
    func chatUpdatedUpdates() throws {
        let harness = try makeHarness(host: "group-updated-held.test")
        defer { harness.tearDown() }

        harness.coordinator.handle(frame: try frame(#"""
        {"type": "chat_updated",
         "chat": {"id": 77, "kind": "group", "title": "The Cousins", "peer_user_id": null,
                  "member_ids": [7, 9, 11], "admin_user_id": 9}}
        """#))

        let chat = try #require(harness.chat(77))
        #expect(chat.title == "The Cousins")
        #expect(chat.groupMemberIDs == [7, 9, 11])
        #expect(chat.groupAdminUserID == 9)
        // A rename is not a message: the count is the message pipeline's.
        #expect(chat.unreadCount == 2)
        #expect(harness.messageChatIDs() == [1, 43, 77])
    }

    @Test("chat_removed drops the group, its messages and its draft — and nothing else")
    func chatRemovedDrops() throws {
        let harness = try makeHarness(host: "group-removed.test")
        defer { harness.tearDown() }
        ComposerDrafts.stash("half a thought", for: 77)
        var dropped: [Int64] = []
        harness.coordinator.onChatDropped = { dropped.append($0) }

        harness.coordinator.handle(frame: try frame(#"{"type": "chat_removed", "chat_id": 77}"#))

        #expect(harness.chatIDs() == [1, 43])
        #expect(harness.messageChatIDs() == [1, 43])
        #expect(ComposerDrafts.take(for: 77) == nil)
        #expect(dropped == [77])
    }

    @Test("chat_removed for a chat this device never held changes nothing")
    func chatRemovedUnknown() throws {
        let harness = try makeHarness(host: "group-removed-unknown.test")
        defer { harness.tearDown() }

        harness.coordinator.handle(frame: try frame(#"{"type": "chat_removed", "chat_id": 999}"#))

        #expect(harness.chatIDs() == [1, 43, 77])
    }

    @Test("a group GET /chats no longer lists is pruned with its messages")
    func resyncPrunesAbsentGroup() async throws {
        let host = "group-prune.test"
        let harness = try makeHarness(host: host) { request in
            switch request.url.path() {
            case "/api/v1/me": return .json(200, Self.meJSON)
            case "/api/v1/families/mine": return .json(200, Self.familyJSON)
            case "/api/v1/chats":
                return .json(200, #"""
                {"chats": [
                  {"chat": {"id": 1, "kind": "family", "title": "The Smiths", "peer_user_id": null}, "last_message": null, "unread_count": 0},
                  {"chat": {"id": 43, "kind": "direct", "title": "Kid", "peer_user_id": 9}, "last_message": null, "unread_count": 0}
                ]}
                """#)
            default: return .json(200, #"{"messages": []}"#)
            }
        }
        defer { harness.tearDown() }

        await harness.coordinator.resync()

        #expect(harness.chatIDs() == [1, 43])
        #expect(harness.messageChatIDs() == [1, 43])
    }

    @Test("GET /chats writes a group's members and admin, and a list without them clears them")
    func resyncWritesGroupFields() async throws {
        let host = "group-resync-fields.test"
        let harness = try makeHarness(host: host) { request in
            switch request.url.path() {
            case "/api/v1/me": return .json(200, Self.meJSON)
            case "/api/v1/families/mine": return .json(200, Self.familyJSON)
            case "/api/v1/chats":
                return .json(200, #"""
                {"chats": [
                  {"chat": {"id": 1, "kind": "family", "title": "The Smiths", "peer_user_id": null}, "last_message": null, "unread_count": 0},
                  {"chat": {"id": 43, "kind": "direct", "title": "Kid", "peer_user_id": 9}, "last_message": null, "unread_count": 0},
                  {"chat": {"id": 77, "kind": "group", "title": "Cousins", "peer_user_id": null, "member_ids": [9, 7, 12], "admin_user_id": 9}, "last_message": null, "unread_count": 0}
                ]}
                """#)
            default: return .json(200, #"{"messages": []}"#)
            }
        }
        defer { harness.tearDown() }

        await harness.coordinator.resync()

        let group = try #require(harness.chat(77))
        #expect(group.groupMemberIDs == [9, 7, 12])
        #expect(group.groupAdminUserID == 9)
        #expect(harness.chat(1)?.groupMemberIDs == [])
        #expect(harness.chat(1)?.groupAdminUserID == nil)
    }

    @Test("leaving drops the group here; a group already gone counts as left")
    func leaveDrops() async throws {
        let host = "group-leave.test"
        let harness = try makeHarness(host: host) { request in
            // The second leave is for a group the server no longer has.
            request.url.path() == "/api/v1/chats/78/members/7"
                ? .json(404, #"{"error": {"code": "chat_not_found", "message": "no"}}"#)
                : .empty(204)
        }
        defer { harness.tearDown() }
        harness.context.insert(ChatEntity(chatID: 78, kind: "group", pinRank: 1, title: "Old"))
        try harness.context.save()

        try await harness.coordinator.leaveGroup(chatID: 77)
        try await harness.coordinator.leaveGroup(chatID: 78)

        #expect(harness.chatIDs() == [1, 43])
        #expect(harness.messageChatIDs() == [1, 43])
    }

    @Test("a refused change is thrown for the screen and changes nothing")
    func refusedChangeKeepsTheGroup() async throws {
        let host = "group-refused.test"
        let harness = try makeHarness(host: host) { _ in
            .json(403, #"{"error": {"code": "not_group_admin", "message": "no"}}"#)
        }
        defer { harness.tearDown() }

        await #expect(throws: APIError.forbidden(code: "not_group_admin")) {
            try await harness.coordinator.deleteGroup(chatID: 77)
        }
        #expect(harness.chatIDs() == [1, 43, 77])
    }

    @Test("an answered rename is applied at once, before its own frame arrives")
    func renameApplies() async throws {
        let host = "group-rename.test"
        let harness = try makeHarness(host: host) { _ in
            .json(200, #"{"chat": {"id": 77, "kind": "group", "title": "The Cousins", "peer_user_id": null, "member_ids": [7, 9], "admin_user_id": 7}}"#)
        }
        defer { harness.tearDown() }

        try await harness.coordinator.renameGroup(chatID: 77, title: "The Cousins")

        #expect(harness.chat(77)?.title == "The Cousins")
    }
}
