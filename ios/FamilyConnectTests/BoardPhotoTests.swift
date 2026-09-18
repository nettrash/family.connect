//
//  BoardPhotoTests.swift
//  FamilyConnectTests
//
//  Issue #69: a picture pinned to the board is seen — by the family, and by
//  the device that pinned it — and the board's cache follows the rules
//  docs/protocol.md "Board" writes down.
//
//  The drawing half of the fix (NotePicture reading `store.generation`) is a
//  view and is not reached from here; what it draws through is:
//  `AttachmentStore.previewOrPhoto`, which is.
//

import Foundation
import SwiftData
import Testing
@testable import FamilyConnect

@MainActor
@Suite("Board photos and the board's rules")
struct BoardPhotoTests {

    @MainActor
    private struct Harness {
        let container: ModelContainer
        let coordinator: ChatSyncCoordinator
        let store: AttachmentStore
        let host: String

        func notes() -> [NoteEntity] {
            (try? container.mainContext.fetch(FetchDescriptor<NoteEntity>())) ?? []
        }

        func note(_ id: Int64) -> NoteEntity? {
            notes().first { $0.noteID == id }
        }

        func requests() -> [RecordedRequest] { StubURLProtocol.requests(host: host) }

        func tearDown() { StubURLProtocol.unregister(host: host) }
    }

    private func makeHarness(
        host: String,
        handler: @escaping StubURLProtocol.Handler
    ) throws -> Harness {
        StubURLProtocol.register(host: host, handler: handler)
        let container = try ModelContainer(
            for: ChatEntity.self, MessageEntity.self, MemberEntity.self, NoteEntity.self,
            PendingMediaItemEntity.self,
            configurations: ModelConfiguration(isStoredInMemoryOnly: true))
        let api = APIClient(
            serverURL: URL(string: "https://\(host)")!,
            session: StubURLProtocol.makeSession())
        let coordinator = ChatSyncCoordinator(modelContainer: container, api: api)
        coordinator.currentUserIDOverride = 7
        let directory = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("board-photo-\(UUID().uuidString)")
        let store = AttachmentStore(api: api, directory: directory)
        coordinator.bind(attachmentStore: store)
        return Harness(container: container, coordinator: coordinator, store: store, host: host)
    }

    private func waitUntil(_ condition: @MainActor () -> Bool, timeout: TimeInterval = 5) async {
        let deadline = Date().addingTimeInterval(timeout)
        while !condition() && Date() < deadline {
            try? await Task.sleep(for: .milliseconds(20))
        }
    }

    nonisolated private static let photoJSON = """
        {"id": 34, "kind": "photo", "mime": "image/jpeg", "size": 1234, "width": 32,
         "height": 32, "has_preview": true}
        """

    nonisolated private static func noteJSON(
        id: Int64, boardSeq: Int64, kind: String = "photo", withPicture: Bool = true
    ) -> String {
        """
        {"id": \(id), "author_id": 7, "kind": "\(kind)", "text": "", "color": "yellow",
         "size": "medium", "font": "plain", "x": 0.35, "y": 0.3,
         "created_at": "2026-09-10T12:00:00Z", "updated_at": "2026-09-10T12:00:00Z",
         "board_seq": \(boardSeq), "content_seq": \(boardSeq)\(withPicture ? ", \"attachment\": \(photoJSON)" : "")}
        """
    }

    private func dto(_ json: String) throws -> NoteDTO {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return try decoder.decode(NoteDTO.self, from: Data(json.utf8))
    }

    private func prepared() throws -> MediaPrep.Prepared {
        let url = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("pin-\(UUID().uuidString).jpg")
        try TestImages.photograph(width: 32, height: 32).write(to: url)
        return MediaPrep.Prepared(
            fileURL: url, mime: "image/jpeg", kind: "photo", width: 32, height: 32,
            previewJPEG: TestImages.photograph(width: 16, height: 16))
    }

    // MARK: - Drawing a board photo

    /// THE READ HALF OF #69. A pin whose preview upload was lost still has
    /// its picture: the photo itself is asked for — but only once the
    /// server has said there is no preview, so a wall of photos does not
    /// fetch every full picture on it.
    @Test("a board photo with no preview draws the photo itself")
    func previewOrPhotoFallsBack() async throws {
        let host = "board-photo-fallback.test"
        let harness = try makeHarness(host: host) { request in
            if request.url.path.hasSuffix("/preview") { return .empty(404) }
            return StubResponse(
                status: 200, headers: ["Content-Type": "image/jpeg"],
                body: TestImages.photograph(width: 32, height: 32))
        }
        defer { harness.tearDown() }

        #expect(harness.store.previewOrPhoto(id: 34) == nil)
        await waitUntil { !harness.store.isFetching(id: 34, preview: true) }
        #expect(
            !harness.requests().contains { $0.url.path == "/api/v1/attachments/34" },
            "the full photo is not asked for before the preview has answered")
        // The 404 bumped `generation`; the view asks again and gets here.
        _ = harness.store.previewOrPhoto(id: 34)
        await waitUntil { harness.store.previewOrPhoto(id: 34) != nil }
        #expect(harness.store.previewOrPhoto(id: 34) != nil, "the photo itself is drawn")
    }

    @Test("a board photo with a preview never fetches the photo itself")
    func previewOrPhotoPrefersThePreview() async throws {
        let host = "board-photo-preview.test"
        let harness = try makeHarness(host: host) { _ in
            StubResponse(
                status: 200, headers: ["Content-Type": "image/jpeg"],
                body: TestImages.photograph(width: 16, height: 16))
        }
        defer { harness.tearDown() }

        _ = harness.store.previewOrPhoto(id: 34)
        await waitUntil { harness.store.previewOrPhoto(id: 34) != nil }
        #expect(harness.store.previewOrPhoto(id: 34) != nil)
        #expect(!harness.requests().contains { $0.url.path == "/api/v1/attachments/34" })
    }

    // MARK: - Pinning one

    /// THE WRITE HALF OF #69. Upload, preview, note, in that order — and the
    /// pinning device draws its own sticker from the preview it already
    /// holds, never fetching it back.
    @Test("pinning a photo uploads, previews, pins — and the pinner sees it at once")
    func pinPhotoInOrder() async throws {
        let host = "board-photo-pin.test"
        let harness = try makeHarness(host: host) { request in
            switch (request.method, request.url.path) {
            case ("POST", "/api/v1/attachments"):
                return .json(201, "{\"attachment\": \(Self.photoJSON)}")
            case ("PUT", "/api/v1/attachments/34/preview"):
                return .empty(204)
            case ("POST", "/api/v1/families/mine/board/notes"):
                return .json(201, "{\"note\": \(Self.noteJSON(id: 12, boardSeq: 100))}")
            default:
                return .empty(404)
            }
        }
        defer { harness.tearDown() }

        let failure = await harness.coordinator.pinPhoto(
            try prepared(), color: "yellow", x: 0.35, y: 0.3)

        #expect(failure == nil)
        let steps = harness.requests().map { "\($0.method) \($0.url.path)" }
        #expect(steps == [
            "POST /api/v1/attachments",
            "PUT /api/v1/attachments/34/preview",
            "POST /api/v1/families/mine/board/notes",
        ])
        let pinned = try #require(harness.note(12))
        #expect(pinned.kind == NoteKind.photo.name)
        #expect(pinned.attachmentID == 34)
        #expect(harness.store.previewOrPhoto(id: 34) != nil, "seeded: drawn at once")
        #expect(!harness.requests().contains { $0.method == "GET" }, "nothing fetched back")
    }

    /// `board_full` is said, never swallowed — and a request that never got
    /// an answer is not reported as a refusal.
    @Test("a pin that did not go up says why")
    func pinFailuresAreSaid() async throws {
        let host = "board-photo-full.test"
        let harness = try makeHarness(host: host) { request in
            switch (request.method, request.url.path) {
            case ("POST", "/api/v1/attachments"):
                return .json(201, "{\"attachment\": \(Self.photoJSON)}")
            case ("PUT", _):
                return .empty(204)
            default:
                return .json(409, #"{"error": {"code": "board_full", "message": "the board is full"}}"#)
            }
        }
        defer { harness.tearDown() }

        let failure = await harness.coordinator.pinPhoto(
            try prepared(), color: "yellow", x: 0.35, y: 0.3)

        #expect(failure == String(localized: "The board is full. Take a note down to make room for this one."))
        #expect(harness.notes().isEmpty)
        #expect(
            ChatSyncCoordinator.pinFailure(APIError.transport(URLError(.timedOut)))
                == String(localized: "The photo didn't reach the server. Check your connection and try again."))
        #expect(
            ChatSyncCoordinator.pinFailure(APIError.conflict(code: "validation", message: nil))
                == String(localized: "The server refused it."))
    }

    // MARK: - The board's rules

    /// THE CACHE HALF OF #69. A device that ran a build from before kinds
    /// cached a photo note as a blank text note — at the same seq the server
    /// still has. The identical copy must repair it, not be refused as "not
    /// newer".
    @Test("a note cached before kinds is repaired by the same seq")
    func sameSeqRepairsAStaleRow() throws {
        let harness = try makeHarness(host: "board-photo-repair.test") { _ in .empty(204) }
        defer { harness.tearDown() }

        harness.coordinator.applyNote(
            try dto(Self.noteJSON(id: 12, boardSeq: 90, kind: "text", withPicture: false)))
        #expect(harness.note(12)?.attachmentID == nil)

        harness.coordinator.applyNote(try dto(Self.noteJSON(id: 12, boardSeq: 90)))

        #expect(harness.note(12)?.kind == NoteKind.photo.name)
        #expect(harness.note(12)?.attachmentID == 34)
        // An OLDER copy is still refused.
        harness.coordinator.applyNote(
            try dto(Self.noteJSON(id: 12, boardSeq: 80, kind: "text", withPicture: false)))
        #expect(harness.note(12)?.attachmentID == 34)
    }

    /// A full read REPLACES what is held: a note it leaves out is gone — one
    /// deleted while this device was not listening — except a note held above
    /// the read's mark, which arrived after the read was taken.
    ///
    /// Through `replaceBoard`, not `loadBoard`: the cursor lives in the
    /// shared UserDefaults, and another suite asserts on it concurrently.
    @Test("a full board read removes what it no longer lists")
    func fullReadReplaces() throws {
        let harness = try makeHarness(host: "board-photo-full-read.test") { _ in .empty(204) }
        defer { harness.tearDown() }

        harness.coordinator.applyNote(try dto(Self.noteJSON(id: 1, boardSeq: 10)))
        harness.coordinator.applyNote(try dto(Self.noteJSON(id: 3, boardSeq: 60)))

        harness.coordinator.replaceBoard(
            with: BoardResponse(notes: [try dto(Self.noteJSON(id: 2, boardSeq: 20))], maxBoardSeq: 50))

        let ids = Set(harness.notes().map(\.noteID))
        #expect(ids == [2, 3], "1 was deleted meanwhile; 3 came after the read")
    }
}
