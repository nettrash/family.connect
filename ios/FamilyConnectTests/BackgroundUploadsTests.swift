//
//  BackgroundUploadsTests.swift
//  FamilyConnectTests
//
//  The half of a media send that happens while the app is not running
//  (docs/protocol.md, "Sending on an unreliable network": an upload may be
//  handed to the system and land while the app is not running).
//
//  WHAT CAN AND CANNOT BE TESTED HERE. A real background `URLSession`
//  needs a device and a suspended app; nothing in this suite proves the
//  system keeps uploading. What it does prove is everything on this side
//  of that hand-back: an answer the system brings back is written onto the
//  right row, twice is the same as once, and the ordinary send leg then
//  finishes the message WITHOUT uploading those bytes again — which is the
//  behaviour that makes the two uploaders safe to have at all.
//
//  Android counterpart: MediaUploadWorkerTest.kt
//

import Foundation
import SwiftData
import Testing
@testable import FamilyConnect

@MainActor
@Suite("Background uploads")
struct BackgroundUploadsTests {

    @MainActor
    private struct Harness {
        let container: ModelContainer
        let coordinator: ChatSyncCoordinator
        let context: ModelContext
        let host: String

        func items() -> [PendingMediaItemEntity] {
            (try? context.fetch(FetchDescriptor<PendingMediaItemEntity>())) ?? []
        }

        func messages() -> [MessageEntity] {
            (try? context.fetch(FetchDescriptor<MessageEntity>())) ?? []
        }

        func settle() async {
            await coordinator.pendingDelivery?.value
        }

        func tearDown() {
            StubURLProtocol.unregister(host: host)
        }
    }

    private func makeHarness(
        host: String, handler: @escaping StubURLProtocol.Handler
    ) throws -> Harness {
        StubURLProtocol.register(host: host, handler: handler)
        let configuration = ModelConfiguration(isStoredInMemoryOnly: true)
        let container = try ModelContainer(
            for: ChatEntity.self, MessageEntity.self, MemberEntity.self,
            PendingMediaItemEntity.self,
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
        return Harness(
            container: container, coordinator: coordinator,
            context: container.mainContext, host: host)
    }

    private func preparedPhoto() throws -> MediaPrep.Prepared {
        let url = MediaPrep.temporaryURL(extension: "jpg")
        try Data([0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10]).write(to: url)
        return MediaPrep.Prepared(
            fileURL: url,
            mime: "image/jpeg",
            kind: "photo",
            width: 1600,
            height: 1200,
            durationMS: nil,
            previewJPEG: Data([0xFF, 0xD8, 0xFF]))
    }

    private static func attachment(id: Int64 = 34) -> AttachmentDTO {
        AttachmentDTO(
            id: id,
            kind: AttachmentDTO.Kind.photo,
            mime: "image/jpeg",
            size: 4096,
            width: 1600,
            height: 1200,
            durationMS: nil,
            hasPreview: false,
            name: nil,
            latitude: nil,
            longitude: nil,
            accuracyM: nil)
    }

    /// A send whose upload the app could not finish: the network is down
    /// while somebody is looking at it, so nothing lands and nothing is
    /// posted — the state the system's uploader inherits.
    private func strandedSend(host: String) throws -> Harness {
        let harness = try makeHarness(host: host) { request in
            switch (request.method, request.url.path()) {
            case ("POST", "/api/v1/attachments"):
                return .failure(URLError(.notConnectedToInternet))
            default:
                return .json(503, #"{"error": {"code": "internal", "message": "no"}}"#)
            }
        }
        return harness
    }

    @Test("An upload the system finished is written onto its row")
    func landedUploadIsRecorded() async throws {
        let host = "bg-upload-record.test"
        let harness = try strandedSend(host: host)
        defer { harness.tearDown() }

        _ = harness.coordinator.sendMedia(try preparedPhoto(), caption: "", in: 42)
        await harness.settle()
        let item = try #require(harness.items().first)
        #expect(item.attachmentID == nil, "the app's own leg could not land it")
        let before = try #require(harness.messages().first).pendingAttachmentCount
        #expect(before == 1)

        harness.coordinator.recordBackgroundUpload(
            itemID: item.itemID, attachment: Self.attachment())

        #expect(harness.items().first?.attachmentID == 34)
        // The id and the count move together, or the send is delivered with
        // an attachment missing.
        #expect(harness.messages().first?.pendingAttachmentCount == 0)
    }

    @Test("The same answer twice is the same as once")
    func recordingIsIdempotent() async throws {
        let host = "bg-upload-twice.test"
        let harness = try strandedSend(host: host)
        defer { harness.tearDown() }

        _ = harness.coordinator.sendMedia(try preparedPhoto(), caption: "", in: 42)
        await harness.settle()
        let item = try #require(harness.items().first)

        // A relaunched session can hand the same completion back more than
        // once; a second decrement would take the count below zero and the
        // row would look like it owed nothing while owing one.
        harness.coordinator.recordBackgroundUpload(
            itemID: item.itemID, attachment: Self.attachment())
        harness.coordinator.recordBackgroundUpload(
            itemID: item.itemID, attachment: Self.attachment(id: 99))

        #expect(harness.items().first?.attachmentID == 34, "the first answer stands")
        #expect(harness.messages().first?.pendingAttachmentCount == 0)
    }

    @Test("A send the system uploaded for is finished without uploading again")
    func resumeAfterABackgroundUploadPostsWithoutReUploading() async throws {
        let host = "bg-upload-resume.test"
        // Stranded first: the app's own leg cannot land the bytes.
        let harness = try strandedSend(host: host)
        defer { harness.tearDown() }
        let localID = try #require(
            harness.coordinator.sendMedia(try preparedPhoto(), caption: "", in: 42))
        await harness.settle()
        let item = try #require(harness.items().first)
        #expect(item.attachmentID == nil)

        // Now the network is back, and an upload WOULD be accepted — which
        // is exactly what must not be asked for. Re-registering also
        // clears the recorded requests, so what follows is the resume
        // alone.
        StubURLProtocol.register(host: host) { request in
            switch (request.method, request.url.path()) {
            case ("POST", "/api/v1/attachments"):
                return .json(201, """
                    {"attachment": {"id": 77, "kind": "photo", "mime": "image/jpeg",
                      "size": 4096, "width": 1600, "height": 1200, "has_preview": false}}
                    """)
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

        // What the system brings back while the app is away...
        harness.coordinator.recordBackgroundUpload(
            itemID: item.itemID, attachment: Self.attachment())
        // ...and the ordinary leg finishes the job.
        await harness.coordinator.uploadAndDeliver(localID: localID)
        await harness.settle()

        let paths = StubURLProtocol.requests(host: host).map { "\($0.method) \($0.url.path())" }
        #expect(
            !paths.contains("POST /api/v1/attachments"),
            "the bytes were already up; they must not go twice: \(paths)")
        #expect(paths.contains("PUT /api/v1/attachments/34/preview"), "the poster still goes")
        #expect(paths.contains("POST /api/v1/chats/42/messages"))
        let row = try #require(harness.messages().first)
        #expect(row.serverID == 900)
        #expect(row.attachmentID == 34)
    }
}
