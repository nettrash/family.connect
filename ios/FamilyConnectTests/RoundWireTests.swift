//
//  RoundWireTests.swift
//  FamilyConnectTests
//
//  Video messages on the wire (#79, docs/protocol.md, "Video messages";
//  docs/audio-video-messages-2026-10-04.md, Phase 2): the one new field on
//  an attachment, `round`, and the flag on a send — REST and the socket —
//  carried through the outbox so that a retry, a sweep or a relaunch still
//  says it. StickerWireTests' cases, asked of the circle.
//
//  Half of these are about what is NOT written: `round` is absent-not-false,
//  so an ordinary video and an ordinary send stay byte-identical to what
//  they were before video messages existed.
//

import Foundation
import SwiftData
import Testing
@testable import FamilyConnect

@MainActor
@Suite("Video message wire shapes")
struct RoundWireTests {

    private func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
        try APICoding.decoder().decode(type, from: Data(json.utf8))
    }

    private func fields(of frame: ClientFrame) throws -> [String: Any] {
        let data = Data(try frame.encodedString().utf8)
        return try #require(try JSONSerialization.jsonObject(with: data) as? [String: Any])
    }

    private func video(round: Bool, id: Int64 = 91) -> AttachmentDTO {
        AttachmentDTO(
            id: id, kind: "video", mime: "video/mp4", size: 1_649_700, width: 480, height: 480,
            durationMS: 23_400, hasPreview: true, name: nil, latitude: nil, longitude: nil,
            accuracyM: nil, isRound: round)
    }

    // MARK: - The flag on an attachment

    @Test("round: true decodes; its absence is false")
    func attachmentFlagDecodes() throws {
        let round = try decode(AttachmentDTO.self, """
            {"id": 91, "kind": "video", "mime": "video/mp4", "size": 1649700,
             "width": 480, "height": 480, "duration_ms": 23400, "has_preview": true,
             "round": true}
            """)
        #expect(round.isRound)
        #expect(!round.sticker)
        let plain = try decode(AttachmentDTO.self, """
            {"id": 92, "kind": "video", "mime": "video/mp4", "size": 4096,
             "width": 1280, "height": 720, "duration_ms": 9000, "has_preview": true}
            """)
        #expect(!plain.isRound)
    }

    @Test("the flag is written only when true, so a stored video is unchanged")
    func attachmentFlagEncodesOnlyWhenTrue() throws {
        func keys(_ dto: AttachmentDTO) throws -> [String: Any] {
            let object = try JSONSerialization.jsonObject(with: JSONEncoder().encode(dto))
            return try #require(object as? [String: Any])
        }
        #expect(try keys(video(round: true))["round"] as? Bool == true)
        #expect(try Set(keys(video(round: false)).keys)
            == ["id", "kind", "mime", "size", "width", "height", "duration_ms", "has_preview"])
    }

    @Test("a stored message keeps the flag through the store and back, and through withPreviewFlag")
    func theRowKeepsTheFlag() {
        let dto = video(round: true)
        let row = MessageEntity(
            localID: "r:1", serverID: 1, chatID: 42, senderID: 9, body: "",
            createdAt: Date(timeIntervalSince1970: 0), status: .sent, attachments: [dto])
        #expect(row.attachmentList.first?.isRound == true)
        #expect(row.attachmentList == [dto])
        #expect(dto.withPreviewFlag(false).isRound)
        #expect(!video(round: false).withPreviewFlag(true).isRound)
    }

    @Test("a queued item carries the flag onto its provisional and uploaded attachments")
    func thePendingItemCarriesTheFlag() {
        let item = PendingMediaItemEntity(
            messageLocalID: "r:1", chatID: 42, position: 0, fileName: "clip.mp4",
            previewFileName: "preview.jpg", mime: "video/mp4", kind: "video",
            width: 480, height: 480, durationMS: 23_400, isRound: true)
        #expect(item.provisionalDTO.isRound)
        item.attachmentID = 91
        #expect(item.uploadedDTO?.isRound == true)
        let plain = PendingMediaItemEntity(
            messageLocalID: "r:2", chatID: 42, position: 0, mime: "video/mp4", kind: "video")
        #expect(!plain.provisionalDTO.isRound)
    }

    // MARK: - Frames

    @Test("a video message goes as a send frame with round, one attachment and an empty body")
    func roundSendFrame() throws {
        let json = try fields(of: .sendRound(
            chatID: 42, clientMsgID: "4f9e21c0", replyToMessageID: nil, attachmentID: 91))
        #expect(json["type"] as? String == "send")
        #expect(json["chat_id"] as? Int == 42)
        #expect(json["client_msg_id"] as? String == "4f9e21c0")
        #expect(json["body"] as? String == "")
        #expect(json["attachment_ids"] as? [Int] == [91])
        #expect(json["round"] as? Bool == true)
        #expect(json["sticker"] == nil)
        #expect(json["reply_to_message_id"] == nil)
        #expect(json["poll"] == nil)
        #expect(json["mentions"] == nil)
    }

    @Test("a video message may be a reply")
    func roundReplyFrame() throws {
        let json = try fields(of: .sendRound(
            chatID: 42, clientMsgID: "abc", replyToMessageID: 41, attachmentID: 91))
        #expect(json["reply_to_message_id"] as? Int == 41)
    }

    @Test("an ordinary send frame and a sticker's say nothing about round")
    func otherFramesAreUnchanged() throws {
        let plain = try fields(of: .send(
            chatID: 42, clientMsgID: "abc", body: "", replyToMessageID: nil,
            attachmentIDs: [92], pollOptions: nil, mentions: nil))
        #expect(plain["round"] == nil)
        let sticker = try fields(of: .sendSticker(
            chatID: 42, clientMsgID: "abc", replyToMessageID: nil, attachmentID: 90))
        #expect(sticker["round"] == nil)
    }

    // MARK: - REST

    nonisolated private static func messageJSON(clientID: String, round: Bool = true) -> String {
        """
        {"message": {"id": 900, "chat_id": 42, "sender_id": 7, "body": "",
         "created_at": "2026-10-05T10:00:00Z", "client_msg_id": "\(clientID)",
         "attachments": [{"id": 91, "kind": "video", "mime": "video/mp4", "size": 1649700,
                          "width": 480, "height": 480, "duration_ms": 23400,
                          "has_preview": true\(round ? #", "round": true"# : "")}]}}
        """
    }

    @Test("POST /messages carries round: true for a video message and no such key otherwise")
    func sendMessageCarriesTheFlag() async throws {
        let host = "round-rest-send.test"
        StubURLProtocol.register(host: host) { request in
            .json(201, Self.messageJSON(
                clientID: request.bodyJSON()?["client_msg_id"] as? String ?? ""))
        }
        defer { StubURLProtocol.unregister(host: host) }
        let api = APIClient(
            serverURL: URL(string: "https://\(host)")!, session: StubURLProtocol.makeSession())

        let sent = try await api.sendMessage(
            chatID: 42, clientMsgID: "abc", body: "", attachmentIDs: [91], round: true)
        #expect(sent.attachmentList.first?.isRound == true)
        _ = try await api.sendMessage(chatID: 42, clientMsgID: "def", body: "", attachmentIDs: [92])

        let bodies = StubURLProtocol.requests(host: host).compactMap { $0.bodyJSON() }
        #expect(bodies.count == 2)
        #expect(bodies[0]["round"] as? Bool == true)
        #expect(bodies[0]["body"] as? String == "")
        #expect(bodies[0]["attachment_ids"] as? [Int] == [91])
        #expect(bodies[0]["sticker"] == nil)
        #expect(bodies[1]["round"] == nil)
    }

    // MARK: - The chat list

    @Test("the chat list says Video message for a circle, before the plain video's word")
    func chatListWord() {
        #expect(ChatSyncCoordinator.preview(body: "", attachment: video(round: true))
            == String(localized: "Video message"))
        #expect(ChatSyncCoordinator.preview(body: "", attachment: video(round: false))
            == String(localized: "Video"))
        // Two of them are a count, as any two videos are.
        #expect(ChatSyncCoordinator.preview(
            body: "", attachments: [video(round: true), video(round: true, id: 93)])
            == String(localized: "\(2) Videos"))
        // Words beside the flag are what the row says, as for any message.
        #expect(ChatSyncCoordinator.preview(body: "hi", attachment: video(round: true)) == "hi")
    }

    // MARK: - Through the outbox

    @MainActor
    private struct Harness {
        let container: ModelContainer
        let coordinator: ChatSyncCoordinator
        let context: ModelContext
        let host: String

        func pending() -> [PendingMediaItemEntity] {
            (try? context.fetch(FetchDescriptor<PendingMediaItemEntity>())) ?? []
        }

        func settle() async { await coordinator.pendingDelivery?.value }

        func tearDown() { StubURLProtocol.unregister(host: host) }
    }

    private func makeHarness(
        host: String, handler: @escaping StubURLProtocol.Handler
    ) throws -> Harness {
        StubURLProtocol.register(host: host, handler: handler)
        let container = try ModelContainer(
            for: ChatEntity.self, MessageEntity.self, MemberEntity.self, NoteEntity.self,
            PendingMediaItemEntity.self,
            configurations: ModelConfiguration(isStoredInMemoryOnly: true))
        let api = APIClient(
            serverURL: URL(string: "https://\(host)")!, session: StubURLProtocol.makeSession())
        let coordinator = ChatSyncCoordinator(modelContainer: container, api: api)
        coordinator.currentUserIDOverride = 7
        coordinator.ackTimeout = 0.2
        container.mainContext.insert(
            ChatEntity(chatID: 42, kind: "family", pinRank: 0, title: "The Smiths"))
        try container.mainContext.save()
        return Harness(
            container: container, coordinator: coordinator, context: container.mainContext,
            host: host)
    }

    /// A clip as the recorder will hand one over: a file, its square size,
    /// its length and its poster.
    private func clip() throws -> MediaPrep.Prepared {
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("round-\(UUID().uuidString).mp4")
        try Data(repeating: 7, count: 2048).write(to: url)
        return MediaPrep.Prepared(
            fileURL: url, mime: "video/mp4", kind: "video", width: 480, height: 480,
            durationMS: 23_400, previewJPEG: TestImages.photograph(width: 48, height: 48))
    }

    @Test("a video message goes through the outbox with its poster and says round: true")
    func sendingAVideoMessage() async throws {
        let host = "round-send.test"
        let harness = try makeHarness(host: host) { request in
            switch (request.method, request.url.path()) {
            case ("POST", "/api/v1/attachments"):
                return .json(201, """
                    {"attachment": {"id": 91, "kind": "video", "mime": "video/mp4", "size": 2048,
                                    "width": 480, "height": 480, "duration_ms": 23400,
                                    "has_preview": false}}
                    """)
            case ("PUT", "/api/v1/attachments/91/preview"):
                return .empty(204)
            case ("POST", "/api/v1/chats/42/messages"):
                let clientID = request.bodyJSON()?["client_msg_id"] as? String ?? ""
                return .json(201, Self.messageJSON(clientID: clientID))
            default:
                return .json(404, #"{"error": {"code": "not_found", "message": "no"}}"#)
            }
        }
        defer { harness.tearDown() }

        let localID = try #require(harness.coordinator.sendRoundVideo(
            try clip(), replyTo: ReplyToDTO(messageID: 41, senderID: 9, excerpt: "Hi"), in: 42))
        await harness.settle()

        let requests = StubURLProtocol.requests(host: host)
        let upload = try #require(requests.first)
        let query = upload.url.query() ?? ""
        #expect(query.contains("kind=video"))
        #expect(query.contains("width=480") && query.contains("height=480"))
        #expect(query.contains("duration_ms=23400"))
        #expect(requests.contains { $0.method == "PUT" && $0.url.path() == "/api/v1/attachments/91/preview" },
                "the circle's poster was not sent")
        let send = try #require(requests.last?.bodyJSON())
        #expect(send["round"] as? Bool == true)
        #expect(send["sticker"] == nil)
        #expect(send["body"] as? String == "")
        #expect(send["attachment_ids"] as? [Int] == [91])
        #expect(send["reply_to_message_id"] as? Int == 41)

        let row = try #require(harness.coordinator.fetchMessage(localID: localID))
        #expect(row.state == .sent)
        #expect(MessagePresentation.isRoundVideo(MessageSnapshot(row)))
    }

    @Test("with no network it waits in the outbox, drawn round from its own poster, still a circle")
    func aVideoMessageSentOfflineIsQueued() async throws {
        let host = "round-send-offline.test"
        let harness = try makeHarness(host: host) { _ in .failure(URLError(.notConnectedToInternet)) }
        defer { harness.tearDown() }

        let localID = try #require(harness.coordinator.sendRoundVideo(try clip(), in: 42))
        await harness.settle()
        defer { harness.coordinator.deleteLocalMessage(localID: localID) }

        let row = try #require(harness.coordinator.fetchMessage(localID: localID))
        #expect(row.state == .pending)
        // The sender's own circle draws at once, under a provisional id.
        #expect(MessagePresentation.isRoundVideo(MessageSnapshot(row)))
        #expect((row.attachmentList.first?.id ?? 0) < 0)
        #expect(row.attachmentList.first?.hasPreview == true)

        let staged = try #require(harness.pending().first)
        #expect(staged.isRound, "a send resumed after a relaunch would go as a square video")
        #expect(!staged.sticker)
        let chat = try #require(try harness.context.fetch(FetchDescriptor<ChatEntity>()).first)
        #expect(chat.lastMessagePreview == String(localized: "Video message"))
    }

    @Test("only a video is sent as a video message")
    func onlyAVideo() throws {
        let harness = try makeHarness(host: "round-send-kind.test") { _ in .empty(204) }
        defer { harness.tearDown() }
        let photo = MediaPrep.Prepared(
            fileURL: URL(fileURLWithPath: "/tmp/nothing.jpg"), mime: "image/jpeg", kind: "photo")
        #expect(harness.coordinator.sendRoundVideo(photo, in: 42) == nil)
        #expect(harness.pending().isEmpty)
    }
}
