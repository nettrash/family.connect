//
//  RoundCacheRepairTests.swift
//  FamilyConnectTests
//
//  The video message's flag in the STORE (#79, docs/audio-video-messages-
//  2026-10-04.md, S5.6, S5.8), on the phone and on the Mac alike:
//
//    - the sender's own circle keeps `round` through the socket's ack, a
//      history page and a re-delivery — so the sender's row is drawn round
//      after the send as before it;
//    - a circle an OLDER build cached is read once more and drawn round.
//      A build before #79 wrote its cached attachment set back with only the
//      fields it knew, and the catch-up only ever adds, so a circle that
//      arrived before the upgrade drew square for good — on a phone that
//      had the old app when the circle came in, upgrading changed nothing.
//      The first test fails on the old code: the row stays square.
//    - one message that can never be read (a 403 or 404 on every resync)
//      is settled and the pass goes on, instead of being asked first every
//      time and blocking every older one; only what says nothing about
//      the one message (no network, a lost session, a throttle) ends it.
//

import Foundation
import SwiftData
import Testing
@testable import FamilyConnect

@MainActor
@Suite("Video message: the flag in the store", .serialized)
struct RoundCacheRepairTests {

    @MainActor
    final class Harness {
        let container: ModelContainer
        let coordinator: ChatSyncCoordinator
        let host: String

        init(host: String, handler: @escaping StubURLProtocol.Handler) throws {
            self.host = host
            StubURLProtocol.register(host: host, handler: handler)
            container = try ModelContainer(
                for: ChatEntity.self, MessageEntity.self, MemberEntity.self, NoteEntity.self,
                PendingMediaItemEntity.self, BlockEntity.self, GoneNoteEntity.self,
                PackItemEntity.self, GonePackItemEntity.self,
                configurations: ModelConfiguration(isStoredInMemoryOnly: true))
            let api = APIClient(
                serverURL: URL(string: "https://\(host)")!, session: StubURLProtocol.makeSession())
            coordinator = ChatSyncCoordinator(modelContainer: container, api: api)
            coordinator.currentUserIDOverride = 7
            coordinator.ackTimeout = 0.2
            container.mainContext.insert(
                ChatEntity(chatID: 42, kind: "family", pinRank: 0, title: "The Smiths"))
            try container.mainContext.save()
        }

        var context: ModelContext { container.mainContext }

        func tearDown() { StubURLProtocol.unregister(host: host) }

        /// What the server was asked for one message, by `before_id`.
        func pageRequests() -> [String] {
            StubURLProtocol.requests(host: host)
                .filter { $0.url.path() == "/api/v1/chats/42/messages" }
                .compactMap { $0.url.query() }
                .filter { $0.contains("before_id") }
        }

        /// A row as a build before #79 left it: a video with no body, its
        /// set written back WITHOUT `round` — that build's encoder knew no
        /// such key.
        @discardableResult
        func cachedByAnOldBuild(serverID: Int64, body: String = "", kind: String = "video") throws -> MessageEntity {
            let attachment = AttachmentDTO(
                id: serverID + 1000, kind: kind, mime: kind == "video" ? "video/mp4" : "image/jpeg",
                size: 1_649_700, width: 480, height: 480, durationMS: kind == "video" ? 23_400 : nil,
                hasPreview: true, name: nil, latitude: nil, longitude: nil, accuracyM: nil)
            let row = MessageEntity(
                localID: "s:\(serverID)", serverID: serverID, chatID: 42, senderID: 9, body: body,
                createdAt: Date(timeIntervalSince1970: 1_000 + Double(serverID)), status: .sent,
                attachment: attachment, attachments: [attachment])
            row.attachmentsKnowRound = false
            context.insert(row)
            try context.save()
            return row
        }
    }

    /// One message as the server holds it: a video, round or not.
    nonisolated static func messageJSON(id: Int64, round: Bool, body: String = "", clientID: String? = nil) -> String {
        let client = clientID.map { #""\#($0)""# } ?? "null"
        return """
            {"id": \(id), "chat_id": 42, "sender_id": 9, "body": "\(body)",
             "created_at": "2026-10-05T10:00:00Z", "client_msg_id": \(client),
             "attachments": [{"id": \(id + 1000), "kind": "video", "mime": "video/mp4", "size": 1649700,
                              "width": 480, "height": 480, "duration_ms": 23400,
                              "has_preview": true\(round ? #", "round": true"# : "")}]}
            """
    }

    /// `before_id = id + 1, limit = 1` → that message, round.
    nonisolated static func onePage(_ request: RecordedRequest, missing: Set<Int64> = []) -> StubResponse? {
        guard request.url.path() == "/api/v1/chats/42/messages",
              let items = URLComponents(url: request.url, resolvingAgainstBaseURL: false)?.queryItems,
              let before = items.first(where: { $0.name == "before_id" })?.value.flatMap(Int64.init)
        else { return nil }
        let id = before - 1
        if missing.contains(id) { return .json(200, #"{"messages": []}"#) }
        return .json(200, #"{"messages": [\#(messageJSON(id: id, round: true))]}"#)
    }

    // MARK: - An older build's cache

    @Test("a circle an older build cached as a plain video is drawn round after the next resync")
    func anOldBuildsCircleBecomesRound() async throws {
        let harness = try Harness(host: "round-repair-resync.test") { request in
            if let page = Self.onePage(request) { return page }
            switch request.url.path() {
            case "/api/v1/me":
                return .json(200, """
                {"user": {"id": 7, "username": "anna", "display_name": "Anna"},
                 "family": {"id": 3, "name": "The Smiths", "join_policy": "open",
                            "created_at": "2026-08-19T17:00:00Z"},
                 "role": "member"}
                """)
            case "/api/v1/families/mine":
                return .json(200, """
                {"family": {"id": 3, "name": "The Smiths", "join_policy": "open",
                            "created_at": "2026-08-19T17:00:00Z"},
                 "members": [{"id": 7, "username": "anna", "display_name": "Anna", "role": "member"}]}
                """)
            case "/api/v1/chats":
                return .json(200, """
                {"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths",
                                     "peer_user_id": null},
                            "last_message": {"id": 500, "chat_id": 42, "sender_id": 9,
                                             "client_msg_id": null, "body": "",
                                             "created_at": "2026-10-05T10:00:00Z"},
                            "unread_count": 0}]}
                """)
            case "/api/v1/chats/42/messages":
                return .json(200, #"{"messages": []}"#)
            default:
                return .empty(204)
            }
        }
        defer { harness.tearDown() }
        let row = try harness.cachedByAnOldBuild(serverID: 500)
        #expect(!MessagePresentation.isRoundVideo(MessageSnapshot(row)), "the setup: square, as cached")
        let chat = try #require(try harness.context.fetch(FetchDescriptor<ChatEntity>()).first)
        chat.maxServerMessageID = 500
        chat.oldestLoadedMessageID = 500
        try harness.context.save()

        await harness.coordinator.resync()

        let after = try #require(harness.coordinator.fetchMessage(localID: "s:500"))
        #expect(MessagePresentation.isRoundVideo(MessageSnapshot(after)),
                "a circle cached by the old app stays square after the upgrade")
        #expect(after.attachmentsKnowRound)
        #expect(harness.pageRequests().count == 1)
    }

    @Test("only what could be a circle is asked about, once, newest first; the rest are left alone")
    func onlyPossibleCirclesOnce() async throws {
        let harness = try Harness(host: "round-repair-which.test") { request in
            Self.onePage(request, missing: [300]) ?? .json(404, #"{"error": {"code": "not_found", "message": "?"}}"#)
        }
        defer { harness.tearDown() }
        try harness.cachedByAnOldBuild(serverID: 100)                 // a possible circle
        try harness.cachedByAnOldBuild(serverID: 200, body: "look")   // words: never a circle
        try harness.cachedByAnOldBuild(serverID: 250, kind: "photo")  // a photo: never a circle
        try harness.cachedByAnOldBuild(serverID: 300)                 // gone from the server
        let known = try harness.cachedByAnOldBuild(serverID: 400)     // written by this build
        known.attachmentsKnowRound = true
        try harness.context.save()

        await harness.coordinator.repairUnknownRoundFlags()

        #expect(harness.pageRequests().count == 2, "\(harness.pageRequests())")
        #expect(harness.pageRequests().first?.contains("before_id=301") == true, "newest first")
        #expect(MessagePresentation.isRoundVideo(MessageSnapshot(
            try #require(harness.coordinator.fetchMessage(localID: "s:100")))))
        let gone = try #require(harness.coordinator.fetchMessage(localID: "s:300"))
        #expect(gone.attachmentsKnowRound, "a message the server no longer has is asked about once")
        #expect(!MessagePresentation.isRoundVideo(MessageSnapshot(gone)))

        // Asked once: a second pass asks nothing.
        await harness.coordinator.repairUnknownRoundFlags()
        #expect(harness.pageRequests().count == 2)
    }

    @Test("with no network the pass stops, and the rows wait for the next resync")
    func offlineWaits() async throws {
        let harness = try Harness(host: "round-repair-offline.test") { _ in
            .failure(URLError(.notConnectedToInternet))
        }
        defer { harness.tearDown() }
        let row = try harness.cachedByAnOldBuild(serverID: 100)
        try harness.cachedByAnOldBuild(serverID: 90)

        await harness.coordinator.repairUnknownRoundFlags()

        #expect(!row.attachmentsKnowRound, "a failed read is not an answer")
        #expect(StubURLProtocol.requests(host: harness.host).count == 1, "one failure ends the pass")
    }

    /// One message that can never be read must not block the rest.
    ///
    /// The pass goes newest first, and it used to end on ANY error. A
    /// message in a chat this device still caches but may no longer read
    /// (403), or one the server has lost (404), fails the same way on every
    /// resync — so it was asked FIRST every time and every older candidate
    /// waited behind it for good. Fails on the old code: one request, and
    /// the circle at 200 stays square.
    @Test("a message the server refuses or has lost is settled and the pass goes on to the older ones")
    func aRefusalDoesNotBlockTheRest() async throws {
        let harness = try Harness(host: "round-repair-refused.test") { request in
            guard let before = URLComponents(url: request.url, resolvingAgainstBaseURL: false)?
                .queryItems?.first(where: { $0.name == "before_id" })?.value
            else { return .empty(204) }
            switch before {
            case "501": return .json(403, #"{"error": {"code": "not_chat_member", "message": "?"}}"#)
            case "401": return .json(404, #"{"error": {"code": "not_found", "message": "?"}}"#)
            case "301": return .json(503, #"{"error": {"code": "unavailable", "message": "?"}}"#)
            default: return Self.onePage(request) ?? .empty(204)
            }
        }
        defer { harness.tearDown() }
        let forbidden = try harness.cachedByAnOldBuild(serverID: 500)
        let lost = try harness.cachedByAnOldBuild(serverID: 400)
        let unavailable = try harness.cachedByAnOldBuild(serverID: 300)
        try harness.cachedByAnOldBuild(serverID: 200)

        await harness.coordinator.repairUnknownRoundFlags()

        let asked = Set(harness.pageRequests().compactMap { query in
            query.split(separator: "&").first { $0.hasPrefix("before_id=") }.map(String.init)
        })
        #expect(asked == ["before_id=501", "before_id=401", "before_id=301", "before_id=201"],
                "one refusal ended the pass: \(harness.pageRequests())")
        #expect(MessagePresentation.isRoundVideo(MessageSnapshot(
            try #require(harness.coordinator.fetchMessage(localID: "s:200")))),
                "an older circle stayed square behind a message the server refused")
        #expect(forbidden.attachmentsKnowRound, "a 403 is an answer: asking again changes nothing")
        #expect(lost.attachmentsKnowRound, "a 404 is an answer: asking again changes nothing")
        #expect(!unavailable.attachmentsKnowRound, "a 5xx is not an answer: it is asked again")

        // The next resync asks only the one the server could not answer.
        let before = harness.pageRequests().count
        await harness.coordinator.repairUnknownRoundFlags()
        let again = harness.pageRequests().dropFirst(before)
        #expect(!again.isEmpty && again.allSatisfy { $0.contains("before_id=301") }, "\(Array(again))")
        #expect(!unavailable.attachmentsKnowRound)
    }

    /// The other half of the rule: what says nothing about ONE message —
    /// a lost session, a server asking us to slow down — ends the pass, as
    /// the network being down does (`offlineWaits`). Asking the next 24
    /// would only collect the same answer 24 times.
    @Test("a lost session or a throttle ends the pass, and nothing is settled")
    func aFailureAboutEverythingEndsThePass() async throws {
        for (host, status) in [("round-repair-401.test", 401), ("round-repair-429.test", 429)] {
            let harness = try Harness(host: host) { _ in
                .json(status, #"{"error": {"code": "x", "message": "?"}}"#)
            }
            defer { harness.tearDown() }
            let newest = try harness.cachedByAnOldBuild(serverID: 100)
            let older = try harness.cachedByAnOldBuild(serverID: 90)

            await harness.coordinator.repairUnknownRoundFlags()

            #expect(!newest.attachmentsKnowRound && !older.attachmentsKnowRound, "\(status)")
            #expect(!harness.pageRequests().contains { $0.contains("before_id=91") },
                    "\(status) did not end the pass")
        }
    }

    @Test("every set this build writes knows the flag — a new row is never asked about")
    func newRowsKnow() throws {
        let attachment = AttachmentDTO(
            id: 1, kind: "video", mime: "video/mp4", size: 1, width: 480, height: 480,
            durationMS: 1_000, hasPreview: false, name: nil, latitude: nil, longitude: nil,
            accuracyM: nil)
        let row = MessageEntity(
            localID: "s:1", serverID: 1, chatID: 42, senderID: 9, body: "",
            createdAt: Date(), status: .sent, attachment: attachment, attachments: [attachment])
        #expect(row.attachmentsKnowRound)
        let bare = MessageEntity(
            localID: "s:2", serverID: 2, chatID: 42, senderID: 9, body: "hi",
            createdAt: Date(), status: .sent)
        #expect(!bare.attachmentsKnowRound, "nothing to know on a message with no set")
    }

    // MARK: - The sender's own

    @Test("the sender's own circle stays round through the socket's ack, a page and a re-delivery")
    func theSendersOwnStaysRound() async throws {
        let host = "round-own.test"
        let harness = try Harness(host: host) { request in
            switch (request.method, request.url.path()) {
            case ("POST", "/api/v1/attachments"):
                return .json(201, """
                    {"attachment": {"id": 1900, "kind": "video", "mime": "video/mp4", "size": 2048,
                                    "width": 480, "height": 480, "duration_ms": 23400,
                                    "has_preview": false}}
                    """)
            case ("PUT", "/api/v1/attachments/1900/preview"):
                return .empty(204)
            default:
                // No REST send: the socket's ack is what lands it here.
                return .failure(URLError(.notConnectedToInternet))
            }
        }
        defer { harness.tearDown() }
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("round-own-\(UUID().uuidString).mp4")
        try Data(repeating: 7, count: 2048).write(to: url)
        let clip = MediaPrep.Prepared(
            fileURL: url, mime: "video/mp4", kind: "video", width: 480, height: 480,
            durationMS: 23_400, previewJPEG: TestImages.photograph(width: 48, height: 48))

        let localID = try #require(harness.coordinator.sendRoundVideo(clip, in: 42))
        await harness.coordinator.pendingDelivery?.value
        let row = try #require(harness.coordinator.fetchMessage(localID: localID))
        #expect(MessagePresentation.isRoundVideo(MessageSnapshot(row)), "drawn round while it goes up")
        let clientID = try #require(row.clientMsgID)

        let decoder = APICoding.decoder()
        let copy = try decoder.decode(
            MessageDTO.self, from: Data(Self.messageJSON(id: 900, round: true, clientID: clientID).utf8))
        harness.coordinator.handle(frame: .ack(clientMsgID: clientID, message: copy))
        var stored = try #require(harness.coordinator.fetchMessage(localID: localID))
        #expect(stored.serverID == 900 && stored.state == .sent)
        #expect(MessagePresentation.isRoundVideo(MessageSnapshot(stored)), "the ack made it square")
        #expect(stored.attachmentsKnowRound)

        // A history page and a resync's re-delivery are the same copy again.
        harness.coordinator.upsert(copy, bumpUnread: false)
        harness.coordinator.upsert(copy, bumpUnread: false, live: true)
        stored = try #require(harness.coordinator.fetchMessage(localID: localID))
        #expect(MessagePresentation.isRoundVideo(MessageSnapshot(stored)), "a page made it square")
        #expect(stored.body == "", "the body a circle is tested by")
    }
}
