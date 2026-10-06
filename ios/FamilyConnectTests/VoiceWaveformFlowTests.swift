//
//  VoiceWaveformFlowTests.swift
//  FamilyConnectTests
//
//  A voice note's WAVEFORM from the meter to the wire and back (#79;
//  docs/protocol.md, "A voice note's waveform"):
//
//    - the recorder keeps every peak its ticker reads, and a recording hands
//      back `Waveform.fromPeaks` of exactly those;
//    - the upload sends it as `waveform=` — on audio only, and only a value
//      the server will take, so a recording is never refused for its drawing;
//    - the outbox holds it across a relaunch (PendingMediaItemEntity), the
//      sender's own bubble draws it before the server answers, and a voice
//      note parked as "not sent" keeps it;
//    - an attachment carries it in and out in the wire shape, absent when
//      there is none — and an old server that echoes none is no failure.
//

import Foundation
import SwiftData
import Testing
@testable import FamilyConnect

@MainActor
@Suite("Voice waveform: meter to wire")
struct VoiceWaveformFlowTests {

    nonisolated static let shape = "0124689abcddeeedcba987654321001245678aabbba98642"

    // MARK: - The recorder

    final class Engine: VoiceRecordingEngine {
        let url: URL
        var currentTime: TimeInterval = 0
        var isRecording = false
        var onFinish: ((Bool) -> Void)?
        var peaks: [Float] = []

        init(url: URL) { self.url = url }

        func record(forDuration duration: TimeInterval) -> Bool {
            try? Data(count: 4096).write(to: url)
            isRecording = true
            return true
        }

        func stop() { isRecording = false }

        func peakPower() -> Float {
            peaks.isEmpty ? AudioRecorder.quietest : peaks.removeFirst()
        }
    }

    private func scratch() throws -> URL {
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("waveform-flow-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }

    @Test("a recording is sent with the waveform of exactly the peaks its meter read")
    func recorderMakesTheWaveform() async throws {
        let recorder = AudioRecorder()
        recorder.directory = try scratch()
        recorder.permissionProvider = { true }
        recorder.callIsActive = { false }
        recorder.audioSession = AudioSessionControl(activate: {}, deactivate: {})
        var engine: Engine?
        recorder.makeEngine = { url, _ in
            let made = Engine(url: url)
            engine = made
            return made
        }
        await recorder.start()
        let live = try #require(engine)
        let fed: [Float] = [-60, -45, -30, -12, -3, -50, -160, -20, -8, -33]
        live.peaks = fed
        for (index, _) in fed.enumerated() {
            live.currentTime = Double(index + 1) * 0.2
            recorder.tick()
        }
        #expect(recorder.peaks == fed, "the recorder did not keep the peaks it read")

        let recording = try #require(recorder.stopRecording())
        defer { try? FileManager.default.removeItem(at: recording.url) }
        #expect(recording.waveform == Waveform.fromPeaks(fed.map(Double.init)))
        #expect(recording.waveform?.count == 48)
        #expect(recording.waveform != Waveform.encode(Waveform.placeholder))

        // A new recording starts from nothing.
        await recorder.start()
        #expect(recorder.peaks.isEmpty)
        recorder.cancel()
    }

    // MARK: - The upload request

    private func query(kind: String, waveform: String?) async throws -> [URLQueryItem] {
        let api = APIClient(serverURL: URL(string: "https://waveform-query.invalid"))
        let request = try await api.attachmentUploadRequest(
            mime: kind == "audio" ? "audio/mp4" : "image/jpeg", kind: kind,
            width: nil, height: nil, durationMS: kind == "audio" ? 4_200 : nil,
            waveform: waveform)
        let url = try #require(request.url)
        return URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems ?? []
    }

    @Test("a voice note's upload carries waveform=, beside its length")
    func audioCarriesIt() async throws {
        let items = try await query(kind: "audio", waveform: Self.shape)
        #expect(items.contains(URLQueryItem(name: "waveform", value: Self.shape)))
        #expect(items.contains(URLQueryItem(name: "duration_ms", value: "4200")))
    }

    @Test("never on anything but audio, never a value the server would refuse, never empty", arguments: [
        ("photo", Optional(Self.shape)),
        ("video", Optional(Self.shape)),
        ("file", Optional(Self.shape)),
        ("audio", Optional(Self.shape.uppercased())),
        ("audio", Optional(String(Self.shape.dropLast()))),
        ("audio", Optional("")),
        ("audio", nil),
    ])
    func refusedValuesStayHome(kind: String, waveform: String?) async throws {
        let items = try await query(kind: kind, waveform: waveform)
        #expect(!items.contains { $0.name == "waveform" }, "\(kind) sent waveform=\(waveform ?? "nil")")
    }

    // MARK: - The outbox, end to end

    private struct Harness {
        let container: ModelContainer
        let coordinator: ChatSyncCoordinator
        let host: String
    }

    private func harness(host: String, echo: Bool) throws -> Harness {
        StubURLProtocol.register(host: host) { request in
            let waveform = echo ? #", "waveform": "\#(Self.shape)""# : ""
            switch (request.method, request.url.path()) {
            case ("POST", "/api/v1/attachments"):
                return .json(201, """
                    {"attachment": {"id": 77, "kind": "audio", "mime": "audio/mp4",
                     "size": 4096, "duration_ms": 4200, "has_preview": false\(waveform)}}
                    """)
            case ("POST", "/api/v1/chats/42/messages"):
                let clientID = request.bodyJSON()?["client_msg_id"] as? String ?? ""
                return .json(201, """
                    {"message": {"id": 950, "chat_id": 42, "sender_id": 7,
                     "body": "", "created_at": "2026-10-05T09:00:00Z",
                     "client_msg_id": "\(clientID)",
                     "attachments": [{"id": 77, "kind": "audio", "mime": "audio/mp4",
                       "size": 4096, "duration_ms": 4200, "has_preview": false\(waveform)}]}}
                    """)
            default:
                return .json(404, #"{"error": {"code": "not_found", "message": "no"}}"#)
            }
        }
        let container = try ModelContainer(
            for: ChatEntity.self, MessageEntity.self, MemberEntity.self, PendingMediaItemEntity.self,
            configurations: ModelConfiguration(isStoredInMemoryOnly: true))
        let api = APIClient(
            serverURL: URL(string: "https://\(host)")!, session: StubURLProtocol.makeSession())
        let coordinator = ChatSyncCoordinator(modelContainer: container, api: api)
        coordinator.currentUserIDOverride = 7
        coordinator.ackTimeout = 0.2
        container.mainContext.insert(ChatEntity(chatID: 42, kind: "family", pinRank: 0, title: "Us"))
        try container.mainContext.save()
        return Harness(container: container, coordinator: coordinator, host: host)
    }

    private func voiceNote(waveform: String?) throws -> MediaPrep.Prepared {
        let url = MediaPrep.temporaryURL(extension: "m4a")
        try Data(repeating: 0x22, count: 4096).write(to: url)
        var prepared = MediaPrep.Prepared(
            fileURL: url, mime: "audio/mp4", kind: AttachmentDTO.Kind.audio, durationMS: 4_200)
        prepared.waveform = waveform
        return prepared
    }

    @Test("a sent voice note: queued with its waveform, drawn with it at once, uploaded with it")
    func outboxCarriesIt() async throws {
        let world = try harness(host: "waveform-outbox.test", echo: true)
        defer { StubURLProtocol.unregister(host: world.host) }

        let localID = try #require(world.coordinator.sendMedia(
            try voiceNote(waveform: Self.shape), caption: "", in: 42))
        // Before a byte has gone: the queued item holds it, and the sender's
        // own bubble draws it from the provisional attachment.
        let items = try world.container.mainContext.fetch(FetchDescriptor<PendingMediaItemEntity>())
        #expect(items.map(\.waveform) == [Self.shape])
        #expect(items.first?.provisionalDTO.waveform == Self.shape)

        await world.coordinator.pendingDelivery?.value

        let upload = try #require(StubURLProtocol.requests(host: world.host)
            .first { $0.method == "POST" && $0.url.path() == "/api/v1/attachments" })
        let query = URLComponents(url: upload.url, resolvingAgainstBaseURL: false)?.queryItems ?? []
        #expect(query.contains(URLQueryItem(name: "waveform", value: Self.shape)))

        let row = try #require(
            try world.container.mainContext.fetch(FetchDescriptor<MessageEntity>())
                .first { $0.localID == localID || $0.serverID == 950 })
        #expect(row.state == .sent)
        #expect(row.attachmentList.first?.waveform == Self.shape)
    }

    @Test("an old server that echoes no waveform: the note is sent all the same")
    func oldServerIsNoFailure() async throws {
        let world = try harness(host: "waveform-old-server.test", echo: false)
        defer { StubURLProtocol.unregister(host: world.host) }

        _ = try #require(world.coordinator.sendMedia(
            try voiceNote(waveform: Self.shape), caption: "", in: 42))
        await world.coordinator.pendingDelivery?.value

        let row = try #require(
            try world.container.mainContext.fetch(FetchDescriptor<MessageEntity>()).first)
        #expect(row.state == .sent)
        #expect(row.serverID == 950)
    }

    @Test("a photo never queues a waveform, whatever it was handed")
    func photoQueuesNone() async throws {
        let world = try harness(host: "waveform-photo.test", echo: false)
        defer { StubURLProtocol.unregister(host: world.host) }
        let url = MediaPrep.temporaryURL(extension: "jpg")
        try Data([0xFF, 0xD8, 0xFF, 0xE0]).write(to: url)
        var photo = MediaPrep.Prepared(fileURL: url, mime: "image/jpeg", kind: "photo", width: 4, height: 4)
        photo.waveform = Self.shape
        world.coordinator.sendMedia(photo, caption: "", in: 42)
        let items = try world.container.mainContext.fetch(FetchDescriptor<PendingMediaItemEntity>())
        #expect(items.map(\.waveform) == [nil])
        // Never let the send outlive its container.
        await world.coordinator.pendingDelivery?.value
    }

    // MARK: - Parked ("not sent")

    @Test("a parked note keeps its waveform, and sends with it")
    func parkedKeepsIt() throws {
        let root = try scratch()
        let store = ParkedRecordings(root: { root }, account: { "s-u7" })
        let file = root.appendingPathComponent("rec.m4a")
        try Data(count: 4096).write(to: file)
        let entry = try #require(store.park(
            fileAt: file, duration: 4.2, chatID: 42, replyTo: nil, caption: nil, waveform: Self.shape))
        #expect(entry.waveform == Self.shape)
        // Read back from disk, by a fresh store.
        let again = ParkedRecordings(root: { root }, account: { "s-u7" })
        #expect(again.entries(for: 42).map(\.waveform) == [Self.shape])
        #expect(again.prepared(for: entry)?.waveform == Self.shape)
    }

    @Test("an index written before the waveform existed still reads, with none")
    func oldIndexReads() throws {
        let json = """
            {"id":"e1","chatID":5,"fileName":"e1.m4a","durationMS":4200,
             "createdAt":0,"sending":false}
            """
        let entry = try JSONDecoder().decode(ParkedRecordings.Entry.self, from: Data(json.utf8))
        #expect(entry.waveform == nil)
        #expect(entry.durationMS == 4200)
    }

    // MARK: - The attachment, in and out

    @Test("an attachment reads its waveform, writes it back, and leaves it out when there is none")
    func attachmentRoundTrip() throws {
        let json = """
            {"id": 77, "kind": "audio", "mime": "audio/mp4", "size": 4096,
             "duration_ms": 4200, "has_preview": false, "waveform": "\(Self.shape)"}
            """
        let read = try JSONDecoder().decode(AttachmentDTO.self, from: Data(json.utf8))
        #expect(read.waveform == Self.shape)
        let written = try #require(
            try JSONSerialization.jsonObject(with: JSONEncoder().encode(read)) as? [String: Any])
        #expect(written["waveform"] as? String == Self.shape)

        let bare = AttachmentDTO(
            id: 1, kind: "audio", mime: "audio/mp4", size: 1, width: nil, height: nil,
            durationMS: 1000, hasPreview: false, name: nil, latitude: nil, longitude: nil,
            accuracyM: nil)
        let bareJSON = try #require(
            try JSONSerialization.jsonObject(with: JSONEncoder().encode(bare)) as? [String: Any])
        #expect(bareJSON["waveform"] == nil, "an absent waveform was written out")
    }

    /// The one copy-with-a-change the attachment has: a field added after it
    /// was written is the field it forgets, and a voice note copied through
    /// it would then draw the flat placeholder.
    @Test("withPreviewFlag keeps the waveform")
    func previewFlagCopyKeepsIt() {
        let note = AttachmentDTO(
            id: 77, kind: "audio", mime: "audio/mp4", size: 4096, width: nil, height: nil,
            durationMS: 4200, hasPreview: false, name: nil, latitude: nil, longitude: nil,
            accuracyM: nil, waveform: Self.shape)
        #expect(note.withPreviewFlag(true).waveform == Self.shape)
        #expect(note.withPreviewFlag(true).hasPreview)
    }

    @Test("a waveform of the wrong JSON type costs the message nothing")
    func malformedTypeIsDropped() throws {
        let json = """
            {"id": 77, "kind": "audio", "mime": "audio/mp4", "size": 4096,
             "has_preview": false, "waveform": 12}
            """
        let read = try JSONDecoder().decode(AttachmentDTO.self, from: Data(json.utf8))
        #expect(read.waveform == nil)
        #expect(Waveform.levelsOrPlaceholder(read.waveform) == Waveform.placeholder)
    }
}
