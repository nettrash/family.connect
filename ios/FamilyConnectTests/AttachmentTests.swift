//
//  AttachmentTests.swift
//  FamilyConnectTests
//
//  Photos and videos: the upload request the client builds, the send order
//  (bytes first, message second), what a bubble reads back out of the
//  store, and the media preparation that happens before any of it.
//
//  The size ceiling is checked on the CLIENT side here — the server's own
//  refusal is server/tests/attachment_flow.rs. What matters on this side is
//  that a photo is downscaled below the ceiling rather than shipped raw,
//  and that a video too big to compress is refused with the one error the
//  composer turns into advice.
//

import AVFoundation
import Foundation
import SwiftData
import Testing
import CoreGraphics
@testable import FamilyConnect

@MainActor
@Suite("Attachments")
struct AttachmentTests {

    // MARK: - Harness

    @MainActor
    private struct Harness {
        let container: ModelContainer
        let coordinator: ChatSyncCoordinator
        let context: ModelContext
        let host: String

        func messages() -> [MessageEntity] {
            (try? context.fetch(FetchDescriptor<MessageEntity>())) ?? []
        }

        /// Wait for the detached delivery `sendMedia` starts before the
        /// container goes out of scope — SwiftData traps (and takes the
        /// whole test process down) if a context outlives its container
        /// with work still running. See `pendingDelivery`.
        func settle() async {
            await coordinator.pendingDelivery?.value
        }

        func tearDown() {
            StubURLProtocol.unregister(host: host)
        }
    }

    private func makeHarness(host: String, handler: @escaping StubURLProtocol.Handler) throws -> Harness {
        StubURLProtocol.register(host: host, handler: handler)
        let configuration = ModelConfiguration(isStoredInMemoryOnly: true)
        let container = try ModelContainer(
            for: ChatEntity.self, MessageEntity.self, MemberEntity.self,
            configurations: configuration)
        let api = APIClient(
            serverURL: URL(string: "https://\(host)")!,
            session: StubURLProtocol.makeSession())
        let coordinator = ChatSyncCoordinator(modelContainer: container, api: api)
        coordinator.currentUserIDOverride = 7
        coordinator.ackTimeout = 0.2
        let chat = ChatEntity(chatID: 42, kind: "family", pinRank: 0, title: "The Smiths")
        container.mainContext.insert(chat)
        try container.mainContext.save()
        return Harness(container: container, coordinator: coordinator, context: container.mainContext, host: host)
    }

    nonisolated private static func attachmentJSON(
        id: Int64 = 34,
        kind: String = "photo",
        hasPreview: Bool = false
    ) -> String {
        """
        {"attachment": {"id": \(id), "kind": "\(kind)", "mime": "image/jpeg",
         "size": 4096, "width": 1600, "height": 1200, "has_preview": \(hasPreview)}}
        """
    }

    /// A prepared photo on disk, as MediaPrep would leave one.
    private func preparedPhoto(previewJPEG: Data? = Data([0xFF, 0xD8, 0xFF])) throws -> MediaPrep.Prepared {
        let url = MediaPrep.temporaryURL(extension: "jpg")
        try Data([0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10]).write(to: url)
        return MediaPrep.Prepared(
            fileURL: url,
            mime: "image/jpeg",
            kind: "photo",
            width: 1600,
            height: 1200,
            durationMS: nil,
            previewJPEG: previewJPEG)
    }

    // MARK: - The upload request

    @Test("Metadata rides in the query string and the bytes are the body")
    func uploadRequestShape() async throws {
        let host = "attach-upload.test"
        defer { StubURLProtocol.unregister(host: host) }
        StubURLProtocol.register(host: host) { _ in
            .json(201, Self.attachmentJSON(kind: "video"))
        }
        let api = APIClient(
            serverURL: URL(string: "https://\(host)")!,
            session: StubURLProtocol.makeSession())
        await api.setToken("sekret")

        let url = MediaPrep.temporaryURL(extension: "mp4")
        try Data(repeating: 0x11, count: 2048).write(to: url)
        defer { try? FileManager.default.removeItem(at: url) }

        let attachment = try await api.uploadAttachment(
            fileURL: url,
            mime: "video/mp4",
            kind: "video",
            width: 1920,
            height: 1080,
            durationMS: 8400)
        #expect(attachment.id == 34)
        #expect(attachment.isVideo)

        let request = try #require(StubURLProtocol.requests(host: host).first)
        #expect(request.method == "POST")
        #expect(request.url.path() == "/api/v1/attachments")
        let query = URLComponents(url: request.url, resolvingAgainstBaseURL: false)?.queryItems ?? []
        #expect(query.contains(URLQueryItem(name: "kind", value: "video")))
        #expect(query.contains(URLQueryItem(name: "width", value: "1920")))
        #expect(query.contains(URLQueryItem(name: "height", value: "1080")))
        #expect(query.contains(URLQueryItem(name: "duration_ms", value: "8400")))
        // No multipart: the declared type is the body's type, and the body
        // is the file itself — that is what lets the server stream to disk.
        #expect(request.headers["Content-Type"] == "video/mp4")
        #expect(request.headers["Authorization"] == "Bearer sekret")
        #expect(request.body?.count == 2048)
    }

    @Test("Dimensions the picker could not determine are simply absent")
    func uploadOmitsUnknownDimensions() async throws {
        let host = "attach-nodims.test"
        defer { StubURLProtocol.unregister(host: host) }
        StubURLProtocol.register(host: host) { _ in .json(201, Self.attachmentJSON()) }
        let api = APIClient(
            serverURL: URL(string: "https://\(host)")!,
            session: StubURLProtocol.makeSession())

        let url = MediaPrep.temporaryURL(extension: "jpg")
        try Data([0xFF, 0xD8, 0xFF, 0xE0]).write(to: url)
        defer { try? FileManager.default.removeItem(at: url) }

        _ = try await api.uploadAttachment(
            fileURL: url, mime: "image/jpeg", kind: "photo",
            width: nil, height: nil, durationMS: nil)

        let request = try #require(StubURLProtocol.requests(host: host).first)
        let names = (URLComponents(url: request.url, resolvingAgainstBaseURL: false)?.queryItems ?? [])
            .map(\.name)
        #expect(names == ["kind"])
    }

    // MARK: - Send order

    @Test("Bytes go up first, then the preview, then the message")
    func sendMediaOrdersTheThreeCalls() async throws {
        let host = "attach-send.test"
        let harness = try makeHarness(host: host) { request in
            switch (request.method, request.url.path()) {
            case ("POST", "/api/v1/attachments"):
                return .json(201, Self.attachmentJSON())
            case ("PUT", "/api/v1/attachments/34/preview"):
                return .empty(204)
            case ("POST", "/api/v1/chats/42/messages"):
                let clientID = request.bodyJSON()?["client_msg_id"] as? String ?? ""
                return .json(201, """
                    {"message": {"id": 900, "chat_id": 42, "sender_id": 7,
                     "body": "", "created_at": "2026-08-22T09:00:00Z",
                     "client_msg_id": "\(clientID)",
                     "attachment": {"id": 34, "kind": "photo", "mime": "image/jpeg",
                       "size": 4096, "width": 1600, "height": 1200, "has_preview": true}}}
                    """)
            default:
                return .json(404, #"{"error": {"code": "not_found", "message": "no"}}"#)
            }
        }
        defer { harness.tearDown() }

        let prepared = try preparedPhoto()
        let sent = await harness.coordinator.sendMedia(prepared, caption: "", in: 42)
        #expect(sent)
        await harness.settle()

        // The message must not exist before the bytes do: a bubble pointing
        // at an upload that failed is worse than a busy composer.
        let paths = StubURLProtocol.requests(host: host).map { "\($0.method) \($0.url.path())" }
        #expect(paths.first == "POST /api/v1/attachments")
        #expect(paths.contains("PUT /api/v1/attachments/34/preview"))
        let messageIndex = try #require(paths.firstIndex(of: "POST /api/v1/chats/42/messages"))
        let previewIndex = try #require(paths.firstIndex(of: "PUT /api/v1/attachments/34/preview"))
        #expect(previewIndex < messageIndex)

        // The prepared file is cleaned up whatever happened.
        #expect(!FileManager.default.fileExists(atPath: prepared.fileURL.path))

        // And the row carries the attachment, with an empty body — a photo
        // needs no caption.
        let row = try #require(harness.messages().first)
        #expect(row.body.isEmpty)
        #expect(row.attachmentID == 34)
        let snapshot = try #require(row.attachmentSnapshot)
        #expect(snapshot.kind == "photo")
        #expect(snapshot.width == 1600)
    }

    @Test("A caption rides along with the photo")
    func captionIsSentWithTheAttachment() async throws {
        let host = "attach-caption.test"
        let harness = try makeHarness(host: host) { request in
            switch (request.method, request.url.path()) {
            case ("POST", "/api/v1/attachments"):
                return .json(201, Self.attachmentJSON())
            case ("PUT", "/api/v1/attachments/34/preview"):
                return .empty(204)
            default:
                let clientID = request.bodyJSON()?["client_msg_id"] as? String ?? ""
                return .json(201, """
                    {"message": {"id": 901, "chat_id": 42, "sender_id": 7,
                     "body": "at the lake", "created_at": "2026-08-22T09:00:00Z",
                     "client_msg_id": "\(clientID)",
                     "attachment": {"id": 34, "kind": "photo", "mime": "image/jpeg",
                       "size": 4096, "width": 1600, "height": 1200, "has_preview": true}}}
                    """)
            }
        }
        defer { harness.tearDown() }

        #expect(await harness.coordinator.sendMedia(try preparedPhoto(), caption: "at the lake", in: 42))
        await harness.settle()

        let send = try #require(
            StubURLProtocol.requests(host: host)
                .first { $0.url.path() == "/api/v1/chats/42/messages" })
        let body = try #require(send.bodyJSON())
        #expect(body["body"] as? String == "at the lake")
        #expect(body["attachment_id"] as? Int == 34)
    }

    @Test("A refused upload sends no message at all")
    func refusedUploadLeavesNoBubble() async throws {
        let host = "attach-refused.test"
        let harness = try makeHarness(host: host) { request in
            if request.url.path() == "/api/v1/attachments" {
                return .json(413, #"{"error": {"code": "attachment_too_large", "message": "too big"}}"#)
            }
            return .json(201, #"{"message": {}}"#)
        }
        defer { harness.tearDown() }

        let prepared = try preparedPhoto()
        #expect(await harness.coordinator.sendMedia(prepared, caption: "", in: 42) == false)
        await harness.settle()
        #expect(harness.messages().isEmpty)
        #expect(StubURLProtocol.requests(host: host).count == 1)
        #expect(!FileManager.default.fileExists(atPath: prepared.fileURL.path))
    }

    /// The preview is best-effort by design: a bubble with no preview just
    /// fetches the full image.
    @Test("A failed preview upload does not fail the send")
    func previewFailureIsSurvivable() async throws {
        let host = "attach-nopreview.test"
        let harness = try makeHarness(host: host) { request in
            switch (request.method, request.url.path()) {
            case ("POST", "/api/v1/attachments"):
                return .json(201, Self.attachmentJSON())
            case ("PUT", "/api/v1/attachments/34/preview"):
                return .json(500, #"{"error": {"code": "internal", "message": "nope"}}"#)
            default:
                let clientID = request.bodyJSON()?["client_msg_id"] as? String ?? ""
                // As the real server answers: the message carries its
                // attachment, and has_preview is false because the preview
                // upload it just refused never landed.
                return .json(201, """
                    {"message": {"id": 902, "chat_id": 42, "sender_id": 7, "body": "",
                     "created_at": "2026-08-22T09:00:00Z", "client_msg_id": "\(clientID)",
                     "attachment": {"id": 34, "kind": "photo", "mime": "image/jpeg",
                       "size": 4096, "width": 1600, "height": 1200, "has_preview": false}}}
                    """)
            }
        }
        defer { harness.tearDown() }

        #expect(await harness.coordinator.sendMedia(try preparedPhoto(), caption: "", in: 42))
        await harness.settle()
        let row = try #require(harness.messages().first)
        #expect(row.attachmentID == 34)
        #expect(row.attachmentHasPreview == false)
    }

    @Test("No preview to upload means no preview request")
    func noPreviewMeansNoRequest() async throws {
        let host = "attach-skippreview.test"
        let harness = try makeHarness(host: host) { request in
            if request.url.path() == "/api/v1/attachments" {
                return .json(201, Self.attachmentJSON())
            }
            let clientID = request.bodyJSON()?["client_msg_id"] as? String ?? ""
            return .json(201, """
                {"message": {"id": 903, "chat_id": 42, "sender_id": 7, "body": "",
                 "created_at": "2026-08-22T09:00:00Z", "client_msg_id": "\(clientID)",
                 "attachment": {"id": 34, "kind": "photo", "mime": "image/jpeg",
                   "size": 4096, "width": 1600, "height": 1200, "has_preview": false}}}
                """)
        }
        defer { harness.tearDown() }

        #expect(await harness.coordinator.sendMedia(try preparedPhoto(previewJPEG: nil), caption: "", in: 42))
        await harness.settle()
        let paths = StubURLProtocol.requests(host: host).map(\.url.path)
        #expect(!paths.contains { $0.hasSuffix("/preview") })
        #expect(harness.messages().first?.attachmentHasPreview == false)
    }

    // MARK: - Decoding and persistence

    @Test("An attachment survives the round trip through the entity")
    func attachmentRoundTripsThroughTheEntity() throws {
        let json = """
            {"id": 34, "kind": "video", "mime": "video/mp4", "size": 12345678,
             "width": 1080, "height": 1920, "duration_ms": 8400, "has_preview": true}
            """
        let dto = try JSONDecoder().decode(AttachmentDTO.self, from: Data(json.utf8))
        #expect(dto.isVideo)
        #expect(dto.durationMS == 8400)
        // Portrait: the bubble must lay out taller than it is wide.
        #expect(dto.aspectRatio < 1)

        let entity = MessageEntity(
            localID: "local-1", chatID: 42, senderID: 7, body: "",
            createdAt: Date(), status: .sent)
        entity.attachmentID = dto.id
        entity.attachmentKind = dto.kind
        entity.attachmentMIME = dto.mime
        entity.attachmentSize = dto.size
        entity.attachmentWidth = dto.width
        entity.attachmentHeight = dto.height
        entity.attachmentDurationMS = dto.durationMS
        entity.attachmentHasPreview = dto.hasPreview

        #expect(entity.attachmentSnapshot == dto)
    }

    @Test("A message with no attachment has no snapshot")
    func noAttachmentNoSnapshot() throws {
        let entity = MessageEntity(
            localID: "local-2", chatID: 42, senderID: 7, body: "hello",
            createdAt: Date(), status: .sent)
        #expect(entity.attachmentSnapshot == nil)
    }

    /// Dimensions are optional on the wire (the uploader may not have known
    /// them); a bubble still has to reserve a shape.
    @Test("An attachment with no dimensions falls back to 4:3")
    func missingDimensionsFallBack() throws {
        let json = #"{"id": 3, "kind": "photo", "mime": "image/jpeg", "size": 10, "has_preview": false}"#
        let dto = try JSONDecoder().decode(AttachmentDTO.self, from: Data(json.utf8))
        #expect(dto.width == nil)
        #expect(dto.aspectRatio == 4.0 / 3.0)
    }

    @Test("The stream URL carries the session token")
    func streamURLCarriesTheToken() async throws {
        let api = APIClient(serverURL: URL(string: "https://stream.test")!)
        await api.setToken("sekret")
        let stream = try #require(await api.attachmentStreamURL(id: 34))
        #expect(stream.url.absoluteString == "https://stream.test/api/v1/attachments/34")
        #expect(stream.headers["Authorization"] == "Bearer sekret")
    }

    // MARK: - The sender's own copy

    /// The sender made these bytes a moment ago; drawing their own bubble
    /// must not cost a round trip back to the server for them.
    @Test("Seeded bytes are readable with no fetch at all")
    func seededBytesNeedNoFetch() async throws {
        let host = "attach-seed.test"
        defer { StubURLProtocol.unregister(host: host) }
        StubURLProtocol.register(host: host) { _ in
            .json(404, #"{"error": {"code": "attachment_not_found", "message": "no"}}"#)
        }
        let store = AttachmentStore(
            api: APIClient(
                serverURL: URL(string: "https://\(host)")!,
                session: StubURLProtocol.makeSession()))
        defer { store.clear() }

        let id: Int64 = 9001
        let before = store.generation
        store.seed(TestImages.solid(width: 40, height: 30), id: id, preview: true)

        #expect(store.image(id: id, preview: true) != nil)
        // The bubble redraws off `generation`; seeding has to bump it or
        // the view that asked has no reason to look again.
        #expect(store.generation > before)
        #expect(StubURLProtocol.requests(host: host).isEmpty)
    }

    @Test("Sending a photo seeds this device's cache from what it just made")
    func sendMediaSeedsTheCache() async throws {
        let host = "attach-seeded-send.test"
        let harness = try makeHarness(host: host) { request in
            switch (request.method, request.url.path()) {
            case ("POST", "/api/v1/attachments"):
                return .json(201, """
                    {"attachment": {"id": 9002, "kind": "photo", "mime": "image/jpeg",
                     "size": 4096, "width": 1600, "height": 1200, "has_preview": false}}
                    """)
            case ("PUT", "/api/v1/attachments/9002/preview"):
                return .empty(204)
            default:
                let clientID = request.bodyJSON()?["client_msg_id"] as? String ?? ""
                return .json(201, """
                    {"message": {"id": 950, "chat_id": 42, "sender_id": 7, "body": "",
                     "created_at": "2026-08-22T09:00:00Z", "client_msg_id": "\(clientID)",
                     "attachment": {"id": 9002, "kind": "photo", "mime": "image/jpeg",
                       "size": 4096, "width": 1600, "height": 1200, "has_preview": true}}}
                    """)
            }
        }
        defer { harness.tearDown() }

        let store = AttachmentStore(api: harness.coordinator.api)
        defer { store.clear() }
        harness.coordinator.bind(attachmentStore: store)

        // A real JPEG on both halves, so the store can decode what it is given.
        let full = TestImages.solid(width: 120, height: 90)
        let url = MediaPrep.temporaryURL(extension: "jpg")
        try full.write(to: url)
        let prepared = MediaPrep.Prepared(
            fileURL: url,
            mime: "image/jpeg",
            kind: "photo",
            width: 1600,
            height: 1200,
            durationMS: nil,
            previewJPEG: TestImages.solid(width: 40, height: 30))

        #expect(await harness.coordinator.sendMedia(prepared, caption: "", in: 42))
        await harness.settle()

        // Both halves are drawable straight away, and no GET was needed for
        // either — the sender never re-downloads their own photo.
        #expect(store.image(id: 9002, preview: true) != nil)
        #expect(store.image(id: 9002, preview: false) != nil)
        let gets = StubURLProtocol.requests(host: host)
            .filter { $0.method == "GET" && $0.url.path().hasPrefix("/api/v1/attachments") }
        #expect(gets.isEmpty)
    }

    // MARK: - The composer's other state

    /// Attaching while a reply was primed used to post the photo with no
    /// quote AND leave the banner armed, so the NEXT ordinary message
    /// silently became the reply.
    @Test("A photo sent while replying carries the quote")
    func mediaSendCarriesTheReply() async throws {
        let host = "attach-reply.test"
        let harness = try makeHarness(host: host) { request in
            switch (request.method, request.url.path()) {
            case ("POST", "/api/v1/attachments"):
                return .json(201, Self.attachmentJSON())
            case ("PUT", "/api/v1/attachments/34/preview"):
                return .empty(204)
            default:
                let clientID = request.bodyJSON()?["client_msg_id"] as? String ?? ""
                return .json(201, """
                    {"message": {"id": 960, "chat_id": 42, "sender_id": 7, "body": "",
                     "created_at": "2026-08-22T09:00:00Z", "client_msg_id": "\(clientID)",
                     "reply_to": {"message_id": 41, "sender_id": 9, "excerpt": "See you at six"},
                     "attachment": {"id": 34, "kind": "photo", "mime": "image/jpeg",
                       "size": 4096, "width": 1600, "height": 1200, "has_preview": true}}}
                    """)
            }
        }
        defer { harness.tearDown() }

        let quote = ReplyToDTO(messageID: 41, senderID: 9, excerpt: "See you at six")
        #expect(await harness.coordinator.sendMedia(
            try preparedPhoto(), caption: "", replyTo: quote, in: 42))
        await harness.settle()

        let send = try #require(
            StubURLProtocol.requests(host: host)
                .first { $0.url.path() == "/api/v1/chats/42/messages" })
        #expect(send.bodyJSON()?["reply_to_message_id"] as? Int == 41)

        let row = try #require(harness.messages().first)
        #expect(row.replyToMessageID == 41)
        #expect(row.attachmentID == 34)
    }

    /// A caption-less photo wrote "" as the chat-list preview, and an
    /// empty string is not nil — so the row rendered blank.
    @Test("The chat-list preview says what arrived when there is no caption")
    func previewFallsBackToTheAttachment() throws {
        func attachment(kind: String, name: String? = nil) -> AttachmentDTO {
            let json = """
                {"id": 1, "kind": "\(kind)", "mime": "image/jpeg", "size": 1,
                 "has_preview": false\(name.map { ", \"name\": \"\($0)\"" } ?? "")}
                """
            return try! JSONDecoder().decode(AttachmentDTO.self, from: Data(json.utf8))
        }

        #expect(ChatSyncCoordinator.preview(
            body: "", attachment: attachment(kind: "photo")) == "Photo")
        #expect(ChatSyncCoordinator.preview(
            body: "", attachment: attachment(kind: "video")) == "Video")
        #expect(ChatSyncCoordinator.preview(
            body: "", attachment: attachment(kind: "file", name: "taxes.zip")) == "taxes.zip")
        // A caption still wins, and a plain message is untouched.
        #expect(ChatSyncCoordinator.preview(
            body: "at the lake", attachment: attachment(kind: "photo")) == "at the lake")
        #expect(ChatSyncCoordinator.preview(body: "hello", attachment: nil) == "hello")
    }

    /// URLComponents leaves '+' literal and the server reads a query as
    /// form-urlencoded, where '+' MEANS a space.
    @Test("A filename with a plus survives the upload")
    func plusInAFilenameSurvives() async throws {
        let host = "attach-plus.test"
        defer { StubURLProtocol.unregister(host: host) }
        StubURLProtocol.register(host: host) { _ in
            .json(201, """
                {"attachment": {"id": 41, "kind": "file", "mime": "application/pdf",
                 "size": 8, "has_preview": false, "name": "C++ notes.pdf"}}
                """)
        }
        let api = APIClient(
            serverURL: URL(string: "https://\(host)")!,
            session: StubURLProtocol.makeSession())
        let url = MediaPrep.temporaryURL(extension: "pdf")
        try Data("%PDF-1.7".utf8).write(to: url)
        defer { try? FileManager.default.removeItem(at: url) }

        _ = try await api.uploadAttachment(
            fileURL: url, mime: "application/pdf", kind: "file",
            width: nil, height: nil, durationMS: nil, name: "C++ notes.pdf")

        let request = try #require(StubURLProtocol.requests(host: host).first)
        let raw = request.url.absoluteString
        // Escaped, not literal — a literal + would arrive as a space.
        #expect(raw.contains("%2B%2B"), "\(raw)")
        #expect(!raw.contains("C++"), "\(raw)")
    }

    // MARK: - Files

    @Test("A file's name rides in the query string")
    func fileUploadCarriesItsName() async throws {
        let host = "attach-file.test"
        defer { StubURLProtocol.unregister(host: host) }
        StubURLProtocol.register(host: host) { _ in
            .json(201, """
                {"attachment": {"id": 40, "kind": "file", "mime": "application/pdf",
                 "size": 8, "has_preview": false, "name": "Rechnung März.pdf"}}
                """)
        }
        let api = APIClient(
            serverURL: URL(string: "https://\(host)")!,
            session: StubURLProtocol.makeSession())

        let url = MediaPrep.temporaryURL(extension: "pdf")
        try Data("%PDF-1.7".utf8).write(to: url)
        defer { try? FileManager.default.removeItem(at: url) }

        let attachment = try await api.uploadAttachment(
            fileURL: url, mime: "application/pdf", kind: "file",
            width: nil, height: nil, durationMS: nil, name: "Rechnung März.pdf")
        #expect(attachment.isFile)
        #expect(attachment.name == "Rechnung März.pdf")

        let request = try #require(StubURLProtocol.requests(host: host).first)
        let query = URLComponents(url: request.url, resolvingAgainstBaseURL: false)?.queryItems ?? []
        #expect(query.contains(URLQueryItem(name: "kind", value: "file")))
        // URLComponents percent-encodes it, so an umlaut survives the trip.
        #expect(query.contains(URLQueryItem(name: "name", value: "Rechnung März.pdf")))
        #expect(request.headers["Content-Type"] == "application/pdf")
    }

    @Test("A picked document is copied, not re-encoded")
    func fileIsCopiedVerbatim() async throws {
        let source = MediaPrep.temporaryURL(extension: "pdf")
        let bytes = Data("%PDF-1.7 and then some".utf8)
        try bytes.write(to: source)
        defer { try? FileManager.default.removeItem(at: source) }

        let prepared = try await MediaPrep.prepareFile(from: source, limit: MediaPrep.sizeLimit)
        defer { try? FileManager.default.removeItem(at: prepared.fileURL) }

        #expect(prepared.kind == "file")
        #expect(prepared.mime == "application/pdf")
        #expect(prepared.name == source.lastPathComponent)
        // A file has nothing to preview and no shape to reserve.
        #expect(prepared.previewJPEG == nil)
        #expect(prepared.width == nil)
        #expect(prepared.durationMS == nil)
        // Byte-for-byte: nothing here re-encodes a document.
        #expect(try Data(contentsOf: prepared.fileURL) == bytes)
        // The copy is OURS — the picker's URL is scoped and goes away.
        #expect(prepared.fileURL != source)
    }

    @Test("A file over the ceiling is refused before any upload")
    func oversizedFileIsRefused() async throws {
        let source = MediaPrep.temporaryURL(extension: "bin")
        try Data(repeating: 0x41, count: 4096).write(to: source)
        defer { try? FileManager.default.removeItem(at: source) }

        await #expect(throws: MediaPrep.PrepError.self) {
            _ = try await MediaPrep.prepareFile(from: source, limit: 1024)
        }
    }

    @Test("An unknown extension still gets a type the server will take")
    func unknownExtensionFallsBackToOctetStream() {
        let url = URL(fileURLWithPath: "/tmp/notes.nettrashthing")
        #expect(MediaPrep.mimeType(for: url) == "application/octet-stream")
        #expect(MediaPrep.mimeType(for: URL(fileURLWithPath: "/tmp/a.pdf")) == "application/pdf")
    }

    @Test("A file decodes with its name and no media fields")
    func fileDTODecodes() throws {
        let json = """
            {"id": 40, "kind": "file", "mime": "application/zip", "size": 1536,
             "has_preview": false, "name": "taxes.zip"}
            """
        let dto = try JSONDecoder().decode(AttachmentDTO.self, from: Data(json.utf8))
        #expect(dto.isFile)
        #expect(!dto.isVideo)
        #expect(dto.displayName == "taxes.zip")
        #expect(dto.displaySize.contains("KB"))
        #expect(dto.width == nil)
    }

    /// The bubble falls back to a word rather than showing "attachment 34".
    @Test("A nameless attachment still has something to call itself")
    func displayNameFallsBack() throws {
        let json = #"{"id": 3, "kind": "photo", "mime": "image/jpeg", "size": 10, "has_preview": false}"#
        let dto = try JSONDecoder().decode(AttachmentDTO.self, from: Data(json.utf8))
        #expect(dto.displayName == "Photo")
    }

    /// The name comes off the wire and lands on the local filesystem, so
    /// it gets the same treatment the server gives its header.
    @Test("A hostile filename cannot become a path")
    func cachedFileNameIsSafe() {
        #expect(ChatSyncCoordinator.safeFileName("../../etc/passwd") == "_.._etc_passwd")
        #expect(ChatSyncCoordinator.safeFileName(".hidden") == "hidden")
        #expect(ChatSyncCoordinator.safeFileName("   ") == "file")
        #expect(ChatSyncCoordinator.safeFileName("Invoice 2026.pdf") == "Invoice 2026.pdf")
    }

    // MARK: - Preparation

    @Test("A photo is downscaled, re-encoded, and given a preview")
    func photoIsDownscaled() async throws {
        let original = TestImages.solid(width: 4032, height: 3024)
        let prepared = try await MediaPrep.preparePhoto(from: original, limit: MediaPrep.sizeLimit)
        defer { try? FileManager.default.removeItem(at: prepared.fileURL) }

        #expect(prepared.kind == "photo")
        #expect(prepared.mime == "image/jpeg")
        // The longest edge is capped; the aspect ratio is not touched.
        #expect(prepared.width == MediaPrep.photoEdge)
        #expect(prepared.height == 1536)
        #expect(prepared.durationMS == nil)

        let uploaded = try Data(contentsOf: prepared.fileURL)
        #expect(uploaded.count < original.count)

        let preview = try #require(prepared.previewJPEG)
        #expect(preview.count < uploaded.count)
        let previewSize = try #require(TestImages.size(of: preview))
        #expect(max(previewSize.width, previewSize.height) <= MediaPrep.previewEdge)
    }

    @Test("A portrait photo stays portrait")
    func portraitPhotoKeepsItsShape() async throws {
        let prepared = try await MediaPrep.preparePhoto(
            from: TestImages.solid(width: 1200, height: 1600),
            limit: MediaPrep.sizeLimit)
        defer { try? FileManager.default.removeItem(at: prepared.fileURL) }
        let width = try #require(prepared.width)
        let height = try #require(prepared.height)
        #expect(height > width)
    }

    @Test("Bytes that are not an image are refused before any upload")
    func unreadableBytesAreRefused() async {
        await #expect(throws: MediaPrep.PrepError.self) {
            _ = try await MediaPrep.preparePhoto(from: Data("not an image".utf8), limit: MediaPrep.sizeLimit)
        }
    }

    /// The one failure the composer turns into advice rather than a shrug.
    @Test("A photo that will not fit is refused with tooLargeAfterCompression")
    func photoOverTheCeilingIsRefused() async throws {
        do {
            _ = try await MediaPrep.preparePhoto(from: TestImages.solid(width: 2000, height: 2000), limit: 64)
            Issue.record("expected a refusal")
        } catch MediaPrep.PrepError.tooLargeAfterCompression(let bytes) {
            #expect(bytes > 64)
        }
    }

    @Test("The default ceiling is the protocol's 100 MB")
    func ceilingMatchesTheProtocol() {
        #expect(MediaPrep.sizeLimit == 100 * 1024 * 1024)
    }

}
