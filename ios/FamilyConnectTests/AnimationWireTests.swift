//
//  AnimationWireTests.swift
//  FamilyConnectTests
//
//  Animations on the wire and on screen (issue #96, docs/protocol.md
//  "Animations" › "Sending one" and "How it is drawn"): the one new field
//  on an attachment, `animation`; the flag on a send — REST and the socket
//  — carried through the outbox so that a retry or a relaunch still says
//  it; the words a chat list and a quote use; and the rules for when a
//  tile, and a sticker, move. RoundWireTests' cases, asked of the GIF.
//
//  `animation` is absent-not-false, so an ordinary video and an ordinary
//  send stay byte-identical to what they were.
//

import Foundation
import SwiftData
import Testing
@testable import FamilyConnect

@MainActor
@Suite("Animation wire shapes and drawing rules")
struct AnimationWireTests {

    private func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
        try APICoding.decoder().decode(type, from: Data(json.utf8))
    }

    private func fields(of frame: ClientFrame) throws -> [String: Any] {
        let data = Data(try frame.encodedString().utf8)
        return try #require(try JSONSerialization.jsonObject(with: data) as? [String: Any])
    }

    private func video(animation: Bool, id: Int64 = 93, size: Int64 = 182_330) -> AttachmentDTO {
        AttachmentDTO(
            id: id, kind: "video", mime: "video/mp4", size: size, width: 480, height: 270,
            durationMS: 2100, hasPreview: true, name: nil, latitude: nil, longitude: nil,
            accuracyM: nil, animation: animation)
    }

    // MARK: - The flag on an attachment

    @Test("animation: true decodes; its absence is false")
    func attachmentFlagDecodes() throws {
        let animation = try decode(AttachmentDTO.self, """
            {"id": 93, "kind": "video", "mime": "video/mp4", "size": 182330,
             "width": 480, "height": 270, "duration_ms": 2100, "has_preview": true,
             "animation": true}
            """)
        #expect(animation.animation)
        #expect(animation.isAnimation)
        #expect(!animation.isRound && !animation.sticker)
        let plain = try decode(AttachmentDTO.self, """
            {"id": 92, "kind": "video", "mime": "video/mp4", "size": 4096,
             "width": 1280, "height": 720, "duration_ms": 9000, "has_preview": true}
            """)
        #expect(!plain.animation && !plain.isAnimation)
    }

    @Test("the flag on anything but a video draws as what it is")
    func onlyAVideoIsAnAnimation() throws {
        let photo = try decode(AttachmentDTO.self, """
            {"id": 94, "kind": "photo", "mime": "image/jpeg", "size": 100,
             "width": 10, "height": 10, "has_preview": true, "animation": true}
            """)
        #expect(photo.animation)
        #expect(!photo.isAnimation)
    }

    @Test("the flag is written only when true, so a stored video is unchanged")
    func attachmentFlagEncodesOnlyWhenTrue() throws {
        func keys(_ dto: AttachmentDTO) throws -> [String: Any] {
            let object = try JSONSerialization.jsonObject(with: JSONEncoder().encode(dto))
            return try #require(object as? [String: Any])
        }
        #expect(try keys(video(animation: true))["animation"] as? Bool == true)
        #expect(try Set(keys(video(animation: false)).keys)
            == ["id", "kind", "mime", "size", "width", "height", "duration_ms", "has_preview"])
    }

    @Test("a stored message keeps the flag through the store and back, and through withPreviewFlag")
    func theRowKeepsTheFlag() {
        let dto = video(animation: true)
        let row = MessageEntity(
            localID: "a:1", serverID: 1, chatID: 42, senderID: 9, body: "",
            createdAt: Date(timeIntervalSince1970: 0), status: .sent, attachments: [dto])
        #expect(row.attachmentList.first?.isAnimation == true)
        #expect(row.attachmentList == [dto])
        #expect(dto.withPreviewFlag(false).isAnimation)
        #expect(MessagePresentation.isAnimation(MessageSnapshot(row)))
    }

    @Test("a queued item carries the flag onto its provisional and uploaded attachments")
    func thePendingItemCarriesTheFlag() {
        let item = PendingMediaItemEntity(
            messageLocalID: "a:1", chatID: 42, position: 0, fileName: "clip.mp4",
            previewFileName: "preview.jpg", mime: "video/mp4", kind: "video",
            width: 480, height: 270, durationMS: 2100, animation: true)
        #expect(item.provisionalDTO.isAnimation)
        item.attachmentID = 93
        #expect(item.uploadedDTO?.isAnimation == true)
        let plain = PendingMediaItemEntity(
            messageLocalID: "a:2", chatID: 42, position: 0, mime: "video/mp4", kind: "video")
        #expect(!plain.provisionalDTO.isAnimation)
    }

    // MARK: - Frames

    @Test("an animation goes as a send frame with animation, one attachment, and its caption")
    func animationSendFrame() throws {
        let json = try fields(of: .sendAnimation(
            chatID: 42, clientMsgID: "4f9e21c0", body: "look @Mum", replyToMessageID: 41,
            attachmentID: 93, mentions: [MentionDTO(userID: 4, name: "Mum")]))
        #expect(json["type"] as? String == "send")
        #expect(json["chat_id"] as? Int == 42)
        #expect(json["client_msg_id"] as? String == "4f9e21c0")
        #expect(json["body"] as? String == "look @Mum")
        #expect(json["attachment_ids"] as? [Int] == [93])
        #expect(json["animation"] as? Bool == true)
        #expect(json["reply_to_message_id"] as? Int == 41)
        #expect((json["mentions"] as? [Any])?.count == 1)
        #expect(json["sticker"] == nil)
        #expect(json["round"] == nil)
        #expect(json["poll"] == nil)
    }

    @Test("an ordinary send frame, a sticker's and a circle's say nothing about animation")
    func otherFramesAreUnchanged() throws {
        let plain = try fields(of: .send(
            chatID: 42, clientMsgID: "abc", body: "", replyToMessageID: nil,
            attachmentIDs: [92], pollOptions: nil, mentions: nil))
        #expect(plain["animation"] == nil)
        let sticker = try fields(of: .sendSticker(
            chatID: 42, clientMsgID: "abc", replyToMessageID: nil, attachmentID: 90))
        #expect(sticker["animation"] == nil)
        let round = try fields(of: .sendRound(
            chatID: 42, clientMsgID: "abc", replyToMessageID: nil, attachmentID: 91))
        #expect(round["animation"] == nil)
    }

    // MARK: - REST

    nonisolated private static func messageJSON(
        clientID: String, body: String = "", animation: Bool = true
    ) -> String {
        """
        {"message": {"id": 900, "chat_id": 42, "sender_id": 7, "body": "\(body)",
         "created_at": "2026-10-19T10:00:00Z", "client_msg_id": "\(clientID)",
         "attachments": [{"id": 93, "kind": "video", "mime": "video/mp4", "size": 182330,
                          "width": 480, "height": 270, "duration_ms": 2100,
                          "has_preview": true\(animation ? #", "animation": true"# : "")}]}}
        """
    }

    @Test("POST /messages carries animation: true for an animation and no such key otherwise")
    func sendMessageCarriesTheFlag() async throws {
        let host = "animation-rest-send.test"
        StubURLProtocol.register(host: host) { request in
            .json(201, Self.messageJSON(
                clientID: request.bodyJSON()?["client_msg_id"] as? String ?? ""))
        }
        defer { StubURLProtocol.unregister(host: host) }
        let api = APIClient(
            serverURL: URL(string: "https://\(host)")!, session: StubURLProtocol.makeSession())

        let sent = try await api.sendMessage(
            chatID: 42, clientMsgID: "abc", body: "", attachmentIDs: [93], animation: true)
        #expect(sent.attachmentList.first?.isAnimation == true)
        _ = try await api.sendMessage(chatID: 42, clientMsgID: "def", body: "", attachmentIDs: [92])

        let bodies = StubURLProtocol.requests(host: host).compactMap { $0.bodyJSON() }
        #expect(bodies.count == 2)
        #expect(bodies[0]["animation"] as? Bool == true)
        #expect(bodies[0]["attachment_ids"] as? [Int] == [93])
        #expect(bodies[0]["sticker"] == nil && bodies[0]["round"] == nil)
        #expect(bodies[1]["animation"] == nil)
    }

    // MARK: - Words

    @Test("the chat list says GIF for an animation, Video for a video, the caption over both")
    func chatListWord() {
        #expect(ChatSyncCoordinator.preview(body: "", attachment: video(animation: true))
            == String(localized: "GIF"))
        #expect(ChatSyncCoordinator.preview(body: "", attachment: video(animation: false))
            == String(localized: "Video"))
        #expect(ChatSyncCoordinator.preview(body: "ha", attachment: video(animation: true)) == "ha")
    }

    @Test("a quote of an animation with no words says GIF; words keep their words")
    func quoteWord() {
        let word = String(localized: "GIF")
        #expect(MessagePresentation.quoteWord(
            excerpt: "", messageID: 1, roundIDs: [], animationIDs: [1]) == word)
        #expect(MessagePresentation.quoteWord(
            excerpt: "ha", messageID: 1, roundIDs: [], animationIDs: [1]) == "ha")
        #expect(MessagePresentation.quoteWord(
            excerpt: "", messageID: 2, roundIDs: [], animationIDs: [1]) == "")
        #expect(MessagePresentation.quoteWord(excerpt: "", messageID: 1, roundIDs: [1])
            == String(localized: "Video message"))
    }

    @Test("an animation's message is recognised, and named in a reply's quote")
    func namingQuotes() {
        let animation = MessageSnapshot(MessageEntity(
            localID: "a:1", serverID: 1, chatID: 42, senderID: 9, body: "",
            createdAt: Date(timeIntervalSince1970: 0), status: .sent,
            attachments: [video(animation: true)]))
        let twoVideos = MessageSnapshot(MessageEntity(
            localID: "a:2", serverID: 2, chatID: 42, senderID: 9, body: "",
            createdAt: Date(timeIntervalSince1970: 0), status: .sent,
            attachments: [video(animation: true), video(animation: true, id: 95)]))
        #expect(MessagePresentation.isAnimation(animation))
        #expect(!MessagePresentation.isAnimation(twoVideos), "exactly ONE attachment")
        #expect(MessagePresentation.animationMessageIDs([animation, twoVideos]) == [1])

        let reply = MessageEntity(
            localID: "a:3", serverID: 3, chatID: 42, senderID: 7, body: "lol",
            createdAt: Date(timeIntervalSince1970: 1), status: .sent,
            replyToMessageID: 1, replySenderID: 9, replyExcerpt: "")
        let named = MessagePresentation.namingRoundQuotes(
            MessageSnapshot(reply), roundIDs: [], animationIDs: [1])
        #expect(named.replyTo?.excerpt == String(localized: "GIF"))
    }

    @Test("an animation offers no Show text: it has no sound")
    func noTranscript() {
        #expect(TranscriptDoor.route(for: video(animation: true), maxBytes: nil) == nil)
        #expect(TranscriptDoor.route(for: video(animation: false), maxBytes: nil) == .supplied)
    }

    // MARK: - When things move

    @Test("a tile plays by itself up to 8 MiB, and never under Reduce Motion")
    func tilePlaybackRule() {
        let ceiling = AnimationPlayback.tileCeilingBytes
        #expect(ceiling == 8_388_608)
        #expect(AnimationPlayback.playsInTile(size: 182_330, reduceMotion: false))
        #expect(AnimationPlayback.playsInTile(size: ceiling, reduceMotion: false))
        #expect(!AnimationPlayback.playsInTile(size: ceiling + 1, reduceMotion: false))
        #expect(!AnimationPlayback.playsInTile(size: 182_330, reduceMotion: true))
        // A pending send's own row (size 0) plays from the file it holds.
        #expect(AnimationPlayback.playsInTile(size: 0, reduceMotion: false))
    }

    @Test("a sticker moves only when asked to, on screen, and not under Reduce Motion")
    func stickerMotionRule() {
        #expect(StickerMotion.animates(requested: true, reduceMotion: false, onScreen: true))
        #expect(!StickerMotion.animates(requested: true, reduceMotion: true, onScreen: true),
                "frame zero whenever the system asks for less motion")
        #expect(!StickerMotion.animates(requested: true, reduceMotion: false, onScreen: false),
                "only while on screen")
        #expect(!StickerMotion.animates(requested: false, reduceMotion: false, onScreen: true))
    }

    // MARK: - The video the tile downloads

    @Test("the store downloads an animation's video once, and keeps it as an mp4")
    func theStoreDownloadsTheVideo() async throws {
        let host = "animation-store.test"
        let bytes = Data(repeating: 9, count: 4096)
        StubURLProtocol.register(host: host) { _ in
            StubResponse(status: 200, headers: ["Content-Type": "video/mp4"], body: bytes)
        }
        defer { StubURLProtocol.unregister(host: host) }
        let api = APIClient(
            serverURL: URL(string: "https://\(host)")!, session: StubURLProtocol.makeSession())
        let directory = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("animation-store-\(UUID().uuidString)")
        let store = AttachmentStore(api: api, directory: directory)
        defer { try? FileManager.default.removeItem(at: directory) }

        #expect(store.animationURL(id: 93) == nil, "nothing yet; the download starts")
        let deadline = Date().addingTimeInterval(5)
        while store.animationURL(id: 93) == nil, Date() < deadline {
            try await Task.sleep(for: .milliseconds(20))
        }
        let url = try #require(store.animationURL(id: 93))
        #expect(url.pathExtension == "mp4")
        #expect(try Data(contentsOf: url) == bytes)
        #expect(StubURLProtocol.requests(host: host).count == 1)

        // A provisional id is never asked of the server; a seeded file is.
        #expect(store.animationURL(id: -5) == nil)
        store.seedAnimation(copying: url, id: -5)
        #expect(store.animationURL(id: -5) != nil)
        #expect(StubURLProtocol.requests(host: host).count == 1)

        store.forget(attachmentIDs: [93])
        #expect(!FileManager.default.fileExists(atPath: url.path))
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

    /// An animation as `MediaPrep.prepareAnimation` hands one over.
    private func clip(animation: Bool = true) throws -> MediaPrep.Prepared {
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("animation-\(UUID().uuidString).mp4")
        try Data(repeating: 7, count: 2048).write(to: url)
        return MediaPrep.Prepared(
            fileURL: url, mime: "video/mp4", kind: "video", width: 480, height: 270,
            durationMS: 2100, previewJPEG: TestImages.photograph(width: 48, height: 27),
            animation: animation)
    }

    nonisolated private static func routes(_ request: RecordedRequest) -> StubResponse {
        switch (request.method, request.url.path()) {
        case ("POST", "/api/v1/attachments"):
            let id = StubURLProtocol.requests(host: request.url.host() ?? "")
                .filter { $0.method == "POST" && $0.url.path() == "/api/v1/attachments" }.count + 92
            return .json(201, """
                {"attachment": {"id": \(id), "kind": "video", "mime": "video/mp4", "size": 2048,
                                "width": 480, "height": 270, "duration_ms": 2100,
                                "has_preview": false}}
                """)
        case ("PUT", _):
            return .empty(204)
        case ("POST", "/api/v1/chats/42/messages"):
            let json = request.bodyJSON()
            let clientID = json?["client_msg_id"] as? String ?? ""
            return .json(201, Self.messageJSON(
                clientID: clientID, body: json?["body"] as? String ?? "",
                animation: json?["animation"] as? Bool == true))
        default:
            return .json(404, #"{"error": {"code": "not_found", "message": "no"}}"#)
        }
    }

    @Test("an animation goes through the outbox with its poster and caption, saying animation: true")
    func sendingAnAnimation() async throws {
        let host = "animation-send.test"
        let harness = try makeHarness(host: host) { request in Self.routes(request) }
        defer { harness.tearDown() }

        let localID = try #require(harness.coordinator.sendMedia(
            [try clip()], caption: "so true", in: 42))
        await harness.settle()

        let requests = StubURLProtocol.requests(host: host)
        let upload = try #require(requests.first)
        let query = upload.url.query() ?? ""
        #expect(query.contains("kind=video"))
        #expect(query.contains("width=480") && query.contains("height=270"))
        #expect(query.contains("duration_ms=2100"))
        #expect(requests.contains { $0.method == "PUT" && $0.url.path().hasSuffix("/preview") },
                "the animation's poster was not sent")
        let send = try #require(requests.last?.bodyJSON())
        #expect(send["animation"] as? Bool == true)
        #expect(send["body"] as? String == "so true", "an animation may carry a caption")
        #expect(send["attachment_ids"] as? [Int] == [93])
        #expect(send["round"] == nil && send["sticker"] == nil)

        let row = try #require(harness.coordinator.fetchMessage(localID: localID))
        #expect(row.state == .sent)
        #expect(MessagePresentation.isAnimation(MessageSnapshot(row)))
    }

    @Test("beside other items it goes as the ordinary video it also is")
    func inASetItIsAVideo() async throws {
        let host = "animation-send-set.test"
        let harness = try makeHarness(host: host) { request in Self.routes(request) }
        defer { harness.tearDown() }

        _ = try #require(harness.coordinator.sendMedia(
            [try clip(), try clip(animation: false)], caption: "", in: 42))
        #expect(harness.pending().allSatisfy { !$0.animation })
        await harness.settle()
        let send = try #require(StubURLProtocol.requests(host: host).last?.bodyJSON())
        #expect(send["animation"] == nil)
        #expect((send["attachment_ids"] as? [Int])?.count == 2)
    }

    @Test("with no network it waits in the outbox, still an animation after a relaunch")
    func anAnimationSentOfflineIsQueued() async throws {
        let host = "animation-send-offline.test"
        let harness = try makeHarness(host: host) { _ in .failure(URLError(.notConnectedToInternet)) }
        defer { harness.tearDown() }

        let localID = try #require(harness.coordinator.sendMedia([try clip()], caption: "", in: 42))
        await harness.settle()
        defer { harness.coordinator.deleteLocalMessage(localID: localID) }

        let row = try #require(harness.coordinator.fetchMessage(localID: localID))
        #expect(row.state == .pending)
        #expect(MessagePresentation.isAnimation(MessageSnapshot(row)))
        #expect((row.attachmentList.first?.id ?? 0) < 0)

        let staged = try #require(harness.pending().first)
        #expect(staged.animation, "a send resumed after a relaunch would go as an ordinary video")
        #expect(!staged.isRound && !staged.sticker)
    }
}
