//
//  TranscriptTests.swift
//  FamilyConnectTests
//
//  The text of a voice note or an audio file, on request (docs/protocol.md,
//  "Transcripts on request"): who may ask (`TranscriptDoor`), the call
//  (its own timeout, its request shape, what each refusal becomes), the
//  wire fields the feature adds, and the copy this device keeps.
//

import Foundation
import Testing
@testable import FamilyConnect

// MARK: - Fixtures

enum TranscriptFixtures {
    static let me: Int64 = 7
    static let other: Int64 = 9
    static let assistant: Int64 = 1
    static let processor = "Microsoft — Azure OpenAI (Sweden Central)"
    static let agreed = Date(timeIntervalSince1970: 1_790_000_000)

    static func audio(
        id: Int64 = 34, kind: String = "audio", mime: String = "audio/mp4", size: Int64 = 120_000
    ) -> AttachmentDTO {
        AttachmentDTO(
            id: id, kind: kind, mime: mime, size: size, width: nil, height: nil,
            durationMS: 8400, hasPreview: false, name: nil,
            latitude: nil, longitude: nil, accuracyM: nil)
    }

    /// The door with every fact at its "yes" value, and any one overridden.
    static func door(
        serverTranscribes: Bool = true,
        maxBytes: Int64? = 26_214_400,
        processor: String? = processor,
        agreedAt: Date? = agreed,
        attachment: AttachmentDTO = audio(),
        messageID: Int64? = 1338,
        chatKind: String? = "family",
        senderID: Int64 = other,
        currentUserID: Int64 = me,
        assistantUserID: Int64? = assistant,
        familyAllowsTranscripts: Bool = true
    ) -> TranscriptDoor {
        TranscriptDoor.of(
            serverTranscribes: serverTranscribes, maxBytes: maxBytes, processor: processor,
            agreedAt: agreedAt, attachment: attachment, messageID: messageID,
            chatKind: chatKind, senderID: senderID, currentUserID: currentUserID,
            assistantUserID: assistantUserID, familyAllowsTranscripts: familyAllowsTranscripts)
    }

    static func error(_ code: String) -> String {
        #"{"error": {"code": "\#(code)", "message": "no"}}"#
    }
}

// MARK: - Who may ask

@Suite("Transcript door")
struct TranscriptDoorTests {
    typealias F = TranscriptFixtures

    @Test("your own recording: any chat you are in, with or without the owner's switch")
    func ownRecordingAnywhere() {
        for kind in ["family", "direct", "ai"] {
            for family in [true, false] {
                #expect(
                    F.door(chatKind: kind, senderID: F.me, familyAllowsTranscripts: family) == .open,
                    "own recording in \(kind), switch \(family)")
            }
        }
    }

    @Test("somebody else's: the family chat only, and only with the owner's switch on")
    func othersRecording() {
        #expect(F.door(chatKind: "family", familyAllowsTranscripts: true) == .open)
        #expect(F.door(chatKind: "family", familyAllowsTranscripts: false) == .absent)
        // A direct chat: never, whatever the switch says.
        #expect(F.door(chatKind: "direct", familyAllowsTranscripts: true) == .absent)
        #expect(F.door(chatKind: "direct", familyAllowsTranscripts: false) == .absent)
        // An unknown or unheld chat is not the family chat.
        #expect(F.door(chatKind: nil) == .absent)
        #expect(F.door(chatKind: "something-new") == .absent)
    }

    @Test("the assistant's own messages: never")
    func assistantNever() {
        #expect(F.door(chatKind: "ai", senderID: F.assistant) == .absent)
        #expect(F.door(chatKind: "family", senderID: F.assistant) == .absent)
    }

    /// Without a known user id (-1 before the session settles) nothing is
    /// "mine", so only the family-chat rule can open the door.
    @Test("an unknown reader owns nothing")
    func unknownReader() {
        #expect(F.door(chatKind: "direct", senderID: -1, currentUserID: -1) == .absent)
    }

    @Test("the server has to be able to transcribe, and to name who hears it")
    func serverHalf() {
        #expect(F.door(serverTranscribes: false, senderID: F.me) == .absent)
        #expect(F.door(processor: nil, senderID: F.me) == .absent)
        #expect(F.door(processor: "  ", senderID: F.me) == .absent)
    }

    @Test("no consent yet: the action is there and asks first")
    func asksFirst() {
        #expect(F.door(agreedAt: nil, senderID: F.me) == .asksFirst)
        #expect(F.door(agreedAt: nil) == .asksFirst)
        #expect(F.door(agreedAt: nil).isOffered)
        #expect(!F.door(serverTranscribes: false).isOffered)
    }

    @Test("a message still on its way has nothing to ask about")
    func pendingMessage() {
        #expect(F.door(messageID: nil, senderID: F.me) == .absent)
        #expect(F.door(attachment: F.audio(id: 0), senderID: F.me) == .absent)
    }

    /// Every voice note, audio file and video the RULES allow has the
    /// action — by the stored copy where the server can send it, by sound
    /// this device supplies everywhere else. Never a photo, file or place.
    @Test("voice notes, audio and video have the action; nothing else does")
    func kinds() {
        for mime in ["audio/mp4", "audio/m4a", "audio/mpeg", "audio/wav", "audio/ogg", "audio/flac"] {
            #expect(F.door(attachment: F.audio(mime: mime)) == .open, "\(mime)")
        }
        for mime in ["video/mp4", "video/quicktime"] {
            #expect(F.door(attachment: F.audio(kind: "video", mime: mime)) == .open, "\(mime)")
        }
        #expect(F.door(attachment: F.audio(kind: "photo", mime: "image/jpeg")) == .absent)
        #expect(F.door(attachment: F.audio(kind: "file", mime: "audio/mp4")) == .absent)
        #expect(F.door(attachment: F.audio(kind: "location", mime: "")) == .absent)
        // A video is still under the rules: another member's in a direct
        // chat is never offered.
        #expect(F.door(attachment: F.audio(kind: "video", mime: "video/mp4"), chatKind: "direct")
            == .absent)
        #expect(F.door(
            attachment: F.audio(kind: "video", mime: "video/mp4"), chatKind: "direct",
            senderID: F.me) == .open)
    }

    @Test("the route: the stored copy where it qualifies, supplied sound otherwise")
    func routes() {
        typealias D = TranscriptDoor
        for mime in ["audio/mp4", "audio/m4a", "audio/mpeg", "audio/wav"] {
            #expect(D.route(for: F.audio(mime: mime), maxBytes: nil) == .stored, "\(mime)")
        }
        // Ogg is not in the provider's list; neither is anything else.
        #expect(D.route(for: F.audio(mime: "audio/ogg"), maxBytes: nil) == .supplied)
        #expect(D.route(for: F.audio(mime: "audio/flac"), maxBytes: nil) == .supplied)
        #expect(D.route(for: F.audio(kind: "video", mime: "video/mp4"), maxBytes: nil) == .supplied)
        #expect(D.route(for: F.audio(kind: "video", mime: "video/mp4", size: 100), maxBytes: nil)
            == .supplied, "a video never goes by the stored copy, however small")
        #expect(D.route(for: F.audio(kind: "photo", mime: "image/jpeg"), maxBytes: nil) == nil)
        #expect(D.route(for: F.audio(kind: "file", mime: "audio/mp4"), maxBytes: nil) == nil)
        #expect(D.route(for: F.audio(size: 0), maxBytes: nil) == nil)
    }

    @Test("over the ceiling the stored copy cannot go, and the device supplies the sound")
    func sizeCeiling() {
        typealias D = TranscriptDoor
        #expect(D.route(for: F.audio(size: 1000), maxBytes: 1000) == .stored)
        #expect(D.route(for: F.audio(size: 1001), maxBytes: 1000) == .supplied)
        // The server did not say: the protocol's default, 25 MiB.
        #expect(D.route(for: F.audio(size: 26_214_400), maxBytes: nil) == .stored)
        #expect(D.route(for: F.audio(size: 26_214_401), maxBytes: nil) == .supplied)
        // Never above the ceiling, whatever a server claims.
        #expect(D.route(for: F.audio(size: 30_000_000), maxBytes: 90_000_000) == .supplied)
        #expect(D.ceiling(maxBytes: 90_000_000) == 26_214_400)
        #expect(D.ceiling(maxBytes: nil) == 26_214_400)
        #expect(D.ceiling(maxBytes: 0) == 26_214_400)
        #expect(D.ceiling(maxBytes: 1000) == 1000)
        // Over the ceiling, still offered: the size is the device's to fix.
        #expect(F.door(maxBytes: 1000, attachment: F.audio(size: 1001)) == .open)
        #expect(F.door(attachment: F.audio(size: 0)) == .absent)
    }

    /// As Android, the web and Windows offer it: every video in a pile
    /// gets its own action, numbered among the pile's videos when there is
    /// more than one, so each says which it is the text of.
    @Test("every video in a pile gets its own action, numbered when there are several")
    func pileVideos() {
        let photo = F.audio(id: 1, kind: "photo", mime: "image/jpeg")
        let video = F.audio(id: 2, kind: "video", mime: "video/mp4")
        let second = F.audio(id: 3, kind: "video", mime: "video/mp4")
        #expect(TranscriptDoor.pileVideos([photo, video])
            == [TranscriptDoor.PileVideo(attachment: video, number: nil)])
        #expect(TranscriptDoor.pileVideos([photo, photo]).isEmpty)
        #expect(TranscriptDoor.pileVideos([video, photo, second]) == [
            TranscriptDoor.PileVideo(attachment: video, number: 1),
            TranscriptDoor.PileVideo(attachment: second, number: 2),
        ])
    }
}

// MARK: - What each answer becomes

@Suite("Transcript outcome")
struct TranscriptOutcomeTests {

    @Test("each refusal is sorted by what the row can do next")
    func mapping() {
        #expect(TranscriptOutcome(error: APIError.forbidden(code: "assistant_consent_required"))
            == .consentRequired)
        #expect(TranscriptOutcome(error: APIError.conflict(code: "transcript_refused", message: nil))
            == .failed(.refused))
        for error: APIError in [
            .conflict(code: "not_transcribable", message: nil),
            .forbidden(code: "transcript_not_allowed"),
            .forbidden(code: "transcripts_unavailable"),
            .forbidden(code: "not_chat_member"),
            .conflict(code: "blocked", message: nil),
            .notFound(code: "message_not_found"),
            .notFound(code: "attachment_not_found"),
            .notFound(code: "chat_not_found"),
        ] {
            #expect(TranscriptOutcome(error: error) == .failed(.notAvailable), "\(error)")
        }
        for error: APIError in [
            .server(status: 500, message: nil),
            .transport(URLError(.timedOut)),
            .transport(URLError(.notConnectedToInternet)),
            .throttled(retryAfter: nil),
            .decoding,
            .conflict(code: "validation", message: nil),
        ] {
            #expect(TranscriptOutcome(error: error) == .failed(.retry), "\(error)")
        }
        // What this device found when it tried to make the sound itself.
        #expect(TranscriptOutcome(error: TranscriptSound.Failure.tooLong) == .failed(.tooLong))
        #expect(TranscriptOutcome(error: APIError.payloadTooLarge) == .failed(.tooLong))
        #expect(TranscriptOutcome(error: TranscriptSound.Failure.unreadable) == .failed(.unreadable))
        #expect(TranscriptOutcome(error: MediaTranscoder.Failure.noTrack) == .failed(.unreadable))
        #expect(TranscriptOutcome(error: MediaTranscoder.Failure.readerFailed("x"))
            == .failed(.unreadable))
        // The asker went away: nothing was refused.
        #expect(TranscriptOutcome(error: CancellationError()) == .failed(.retry))
    }

    @Test("only a transient failure offers to ask again")
    func retryOffered() {
        #expect(TranscriptFailure.retry.offersRetry)
        #expect(!TranscriptFailure.refused.offersRetry)
        #expect(!TranscriptFailure.notAvailable.offersRetry)
        #expect(!TranscriptFailure.tooLong.offersRetry)
        #expect(!TranscriptFailure.unreadable.offersRetry)
        #expect(!TranscriptFailure.retry.message.isEmpty)
        #expect(TranscriptFailure.refused.message != TranscriptFailure.notAvailable.message)
        let all: [TranscriptFailure] = [.retry, .refused, .notAvailable, .tooLong, .unreadable]
        #expect(Set(all.map(\.message)).count == all.count, "each says something different")
    }

    @Test("the consent refusal asks the question only where it can be asked")
    func consentQuestion() {
        let p = TranscriptFixtures.processor
        #expect(TranscriptOutcome.consentRequired.asksForConsent(processor: p))
        #expect(!TranscriptOutcome.consentRequired.asksForConsent(processor: nil))
        #expect(!TranscriptOutcome.failed(.retry).asksForConsent(processor: p))
    }
}

// MARK: - The call

@Suite("Transcript call")
struct TranscriptCallTests {
    typealias F = TranscriptFixtures

    private func makeClient(host: String, handler: @escaping StubURLProtocol.Handler) -> APIClient {
        StubURLProtocol.register(host: host, handler: handler)
        return APIClient(
            serverURL: URL(string: "https://\(host)")!,
            session: StubURLProtocol.makeSession())
    }

    /// The stored-bytes form is NO body (protocol.md: a request whose
    /// Content-Type is not multipart is this form), on its own long budget,
    /// while an ordinary call keeps the ordinary 15 s.
    @Test("POST, no body, its own timeout of at least 90 s")
    func requestShape() async throws {
        let host = "transcript-shape.test"
        defer { StubURLProtocol.unregister(host: host) }
        let client = makeClient(host: host) { request in
            request.url.path().hasSuffix("/transcript")
                ? .json(200, #"{"transcript": {"text": "Мы будем в шесть", "language": "ru"}}"#)
                : .json(404, F.error("chat_not_found"))
        }
        let answer = try await client.transcript(chatID: 42, messageID: 1338, attachmentID: 34)
        #expect(answer == TranscriptDTO(text: "Мы будем в шесть", language: "ru"))
        await #expect(throws: APIError.self) {
            _ = try await client.messages(chatID: 42, afterID: 0, limit: 1)
        }

        let log = StubURLProtocol.requests(host: host)
        let call = try #require(log.first { $0.url.path().hasSuffix("/transcript") })
        #expect(call.method == "POST")
        #expect(call.url.path() == "/api/v1/chats/42/messages/1338/attachments/34/transcript")
        #expect(call.body == nil || call.body?.isEmpty == true)
        #expect(call.headers["Content-Type"] == nil)
        #expect(call.timeoutInterval >= 90)
        #expect(APIClient.transcriptTimeout >= 90)
        let ordinary = try #require(log.first { $0.method == "GET" })
        #expect(ordinary.timeoutInterval == 15)
    }

    @Test("silence is an answer, and the language is optional")
    func silence() async throws {
        let host = "transcript-silence.test"
        defer { StubURLProtocol.unregister(host: host) }
        let client = makeClient(host: host) { _ in .json(200, #"{"transcript": {"text": ""}}"#) }
        let answer = try await client.transcript(chatID: 1, messageID: 2, attachmentID: 3)
        #expect(answer.text.isEmpty)
        #expect(answer.language == nil)
    }

    /// Each refusal arrives with its code, refused ONCE — a POST is never
    /// retried by the client, not even the 500 that is transient.
    @Test("refusals keep their codes and are never retried")
    func refusals() async throws {
        let cases: [(Int, String, TranscriptOutcome)] = [
            (403, "assistant_consent_required", .consentRequired),
            (403, "transcript_not_allowed", .failed(.notAvailable)),
            (403, "transcripts_unavailable", .failed(.notAvailable)),
            (400, "not_transcribable", .failed(.notAvailable)),
            (400, "transcript_refused", .failed(.refused)),
            (404, "attachment_not_found", .failed(.notAvailable)),
            (500, "internal", .failed(.retry)),
        ]
        for (index, (status, code, expected)) in cases.enumerated() {
            let host = "transcript-refusal-\(index).test"
            defer { StubURLProtocol.unregister(host: host) }
            let client = makeClient(host: host) { _ in .json(status, F.error(code)) }
            do {
                _ = try await client.transcript(chatID: 1, messageID: 2, attachmentID: 3)
                Issue.record("\(code) did not throw")
            } catch {
                #expect(TranscriptOutcome(error: error) == expected, "\(code)")
            }
            #expect(StubURLProtocol.requests(host: host).count == 1, "\(code)")
        }
    }

    @Test("no connection is worth asking again")
    func transport() async throws {
        let host = "transcript-transport.test"
        defer { StubURLProtocol.unregister(host: host) }
        let client = makeClient(host: host) { _ in .failure(URLError(.timedOut)) }
        do {
            _ = try await client.transcript(chatID: 1, messageID: 2, attachmentID: 3)
            Issue.record("did not throw")
        } catch {
            #expect(TranscriptOutcome(error: error) == .failed(.retry))
        }
    }
}

// MARK: - The wire fields

@Suite("Transcript wire")
struct TranscriptWireTests {

    private func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
        try APICoding.decoder().decode(T.self, from: Data(json.utf8))
    }

    @Test("ai_transcripts reads as off from a server that predates it, and is read when sent")
    func familySwitch() throws {
        let old = try decode(FamilyDTO.self, #"{"id": 3, "name": "S", "join_policy": "open"}"#)
        #expect(!old.aiTranscripts)
        let on = try decode(
            FamilyDTO.self, #"{"id": 3, "name": "S", "join_policy": "open", "ai_transcripts": true}"#)
        #expect(on.aiTranscripts)
        // Tied to no other switch: vision off does not read it as off.
        let alone = try decode(
            FamilyDTO.self,
            #"{"id": 3, "name": "S", "join_policy": "open", "ai_vision": false, "ai_transcripts": true}"#)
        #expect(alone.aiTranscripts && !alone.aiVision)
    }

    @Test("assistant.transcribe and its ceiling")
    func assistantFields() throws {
        let old = try decode(
            AssistantDTO.self, #"{"user_id": 1, "display_name": "AI", "mention": "@ai"}"#)
        #expect(!old.transcribe)
        #expect(old.transcribeMaxBytes == nil)
        let on = try decode(
            AssistantDTO.self,
            #"{"user_id": 1, "display_name": "AI", "mention": "@ai", "transcribe": true, "transcribe_max_bytes": 26214400}"#)
        #expect(on.transcribe)
        #expect(on.transcribeMaxBytes == 26_214_400)
    }

    @Test("statistics: recordings as text and their length, zero from an older server")
    func statistics() throws {
        let old = try decode(
            AiStatsDTO.self, #"{"questions": 2, "prompt_tokens": 10, "completion_tokens": 5}"#)
        #expect(old.transcripts == 0 && old.transcriptDurationMS == 0)
        let new = try decode(
            AiStatsDTO.self,
            #"{"questions": 2, "prompt_tokens": 10, "completion_tokens": 5, "transcripts": 3, "transcript_duration_ms": 61500}"#)
        #expect(new.transcripts == 3)
        #expect(new.transcriptDurationMS == 61_500)
        #expect(!StatisticsView.duration(milliseconds: 61_500).isEmpty)
    }

    /// One key and nothing else, like its five neighbours.
    @Test("setting the switch sends exactly {ai_transcripts: …}")
    func patchOneKey() async throws {
        let host = "transcript-switch.test"
        defer { StubURLProtocol.unregister(host: host) }
        StubURLProtocol.register(host: host) { _ in
            .json(200, #"{"family": {"id": 3, "name": "S", "join_policy": "open", "ai_transcripts": true}}"#)
        }
        let api = APIClient(
            serverURL: URL(string: "https://\(host)")!, session: StubURLProtocol.makeSession())
        let family = try await api.setAITranscripts(true)
        #expect(family.aiTranscripts)
        let patch = try #require(StubURLProtocol.requests(host: host).first { $0.method == "PATCH" })
        #expect(patch.url.path() == "/api/v1/families/mine")
        let body = try #require(patch.bodyJSON())
        #expect(body["ai_transcripts"] as? Bool == true)
        #expect(body.count == 1, "one key and nothing else, got \(body)")
    }

    @Test("the consent screen says a recording's sound goes too — only where it can")
    func consentLine() {
        let p = TranscriptFixtures.processor
        let with = AssistantConsent.disclosure(
            processor: p, familyHistory: true, familyVision: false, transcribes: true)
        let without = AssistantConsent.disclosure(
            processor: p, familyHistory: true, familyVision: false, transcribes: false)
        #expect(with.count == without.count + 1)
        let line = with.first { !without.contains($0) }
        #expect(line?.contains(p) == true)
    }
}

// MARK: - The copy this device keeps

@MainActor
@Suite("Transcript store", .serialized)
struct TranscriptStoreTests {
    typealias F = TranscriptFixtures

    private func directory() -> URL {
        URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("transcripts-\(UUID().uuidString)", isDirectory: true)
    }

    private func api(host: String, handler: @escaping StubURLProtocol.Handler) -> APIClient {
        StubURLProtocol.register(host: host, handler: handler)
        return APIClient(
            serverURL: URL(string: "https://\(host)")!, session: StubURLProtocol.makeSession())
    }

    @Test("an answer is kept, survives a relaunch, and is not asked for again")
    func keptAcrossInstances() async throws {
        let host = "transcript-store-keep.test"
        defer { StubURLProtocol.unregister(host: host) }
        let client = api(host: host) { _ in
            .json(200, #"{"transcript": {"text": "See you at six", "language": "en"}}"#)
        }
        let folder = directory()
        let store = TranscriptStore(api: client, directory: folder)
        #expect(store.kept(34) == nil)

        let outcome = await store.ask(chatID: 42, messageID: 1338, attachmentID: 34)
        #expect(outcome == .text(TranscriptDTO(text: "See you at six", language: "en")))
        #expect(store.kept(34)?.text == "See you at six")
        #expect(store.kept(34)?.source == .stored)
        #expect(store.activity(for: 34) == .idle)

        // A new process: read from disk, with no request at all.
        let reopened = TranscriptStore(api: client, directory: folder)
        #expect(reopened.kept(34)?.text == "See you at six")
        #expect(reopened.kept(34)?.language == "en")
        #expect(StubURLProtocol.requests(host: host).count == 1)
    }

    @Test("hide and show are kept too, and keep the text")
    func hiddenPersists() async throws {
        let host = "transcript-store-hide.test"
        defer { StubURLProtocol.unregister(host: host) }
        let client = api(host: host) { _ in .json(200, #"{"transcript": {"text": ""}}"#) }
        let folder = directory()
        let store = TranscriptStore(api: client, directory: folder)
        await store.ask(chatID: 1, messageID: 2, attachmentID: 3)
        #expect(store.kept(3)?.isSilence == true)

        store.setHidden(true, for: 3)
        #expect(store.kept(3)?.hidden == true)
        #expect(TranscriptStore(api: client, directory: folder).kept(3)?.hidden == true)
        store.setHidden(false, for: 3)
        #expect(TranscriptStore(api: client, directory: folder).kept(3)?.hidden == false)
        #expect(TranscriptStore(api: client, directory: folder).kept(3)?.isSilence == true)
    }

    @Test("an answer from supplied sound is kept on this device as supplied")
    func suppliedSource() {
        let host = "transcript-store-supplied.test"
        defer { StubURLProtocol.unregister(host: host) }
        let client = api(host: host) { _ in .empty(500) }
        let folder = directory()
        let store = TranscriptStore(api: client, directory: folder)
        store.keep(TranscriptDTO(text: "hello", language: nil), source: .supplied, for: 8)
        #expect(TranscriptStore(api: client, directory: folder).kept(8)?.source == .supplied)
    }

    @Test("a failure stays on the row and keeps nothing; consent leaves it idle")
    func failures() async throws {
        final class Script: @unchecked Sendable { var reply = StubResponse.empty(500) }
        let script = Script()
        let host = "transcript-store-fail.test"
        defer { StubURLProtocol.unregister(host: host) }
        let client = api(host: host) { _ in script.reply }
        let store = TranscriptStore(api: client, directory: directory())

        script.reply = .json(500, F.error("internal"))
        #expect(await store.ask(chatID: 1, messageID: 2, attachmentID: 3) == .failed(.retry))
        #expect(store.activity(for: 3) == .failed(.retry))
        #expect(store.kept(3) == nil)

        script.reply = .json(400, F.error("transcript_refused"))
        #expect(await store.ask(chatID: 1, messageID: 2, attachmentID: 3) == .failed(.refused))
        #expect(store.activity(for: 3) == .failed(.refused))

        script.reply = .json(403, F.error("assistant_consent_required"))
        #expect(await store.ask(chatID: 1, messageID: 2, attachmentID: 3) == .consentRequired)
        #expect(store.activity(for: 3) == .idle)
        #expect(store.kept(3) == nil)

        // A refusal is not written down: a fresh store has nothing.
        script.reply = .json(403, F.error("transcript_not_allowed"))
        #expect(await store.ask(chatID: 1, messageID: 2, attachmentID: 3) == .failed(.notAvailable))
    }

    @Test("one request per recording at a time")
    func singleFlight() async throws {
        let host = "transcript-store-flight.test"
        defer { StubURLProtocol.unregister(host: host) }
        let client = api(host: host) { _ in
            // Slow enough that the second ask lands while the first is out.
            Thread.sleep(forTimeInterval: 0.4)
            return .json(200, #"{"transcript": {"text": "once"}}"#)
        }
        let store = TranscriptStore(api: client, directory: directory())
        async let first = store.ask(chatID: 1, messageID: 2, attachmentID: 3)
        // Let the first one start.
        while store.activity(for: 3) != .asking { await Task.yield() }
        let second = await store.ask(chatID: 1, messageID: 2, attachmentID: 3)
        #expect(second == nil)
        #expect(await first == .text(TranscriptDTO(text: "once", language: nil)))
        #expect(StubURLProtocol.requests(host: host).count == 1)
    }

    /// A request still out when the account logs out must not write its
    /// answer back: the store lives as long as the app, `clear()` has wiped
    /// the folder, and the next account would be shown the last one's words
    /// (perhaps from sound it supplied, which nobody else may be handed)
    /// without asking. The same for a chat that went away meanwhile.
    @Test("an answer that lands after logout, or after its chat went away, is not kept")
    func answerAfterClearIsDropped() async throws {
        final class Gate: @unchecked Sendable {
            let lock = NSLock()
            var open = false
            func wait() { while !lock.withLock({ open }) { Thread.sleep(forTimeInterval: 0.01) } }
            func release() { lock.withLock { open = true } }
        }
        let gate = Gate()
        let host = "transcript-store-logout.test"
        defer { StubURLProtocol.unregister(host: host) }
        let client = api(host: host) { _ in
            gate.wait()
            return .json(200, #"{"transcript": {"text": "the last account's"}}"#)
        }
        let folder = directory()
        let store = TranscriptStore(api: client, directory: folder)

        async let first = store.ask(chatID: 1, messageID: 2, attachmentID: 3)
        while store.activity(for: 3) != .asking { await Task.yield() }
        store.clear()
        #expect(store.activity(for: 3) == .idle)
        gate.release()
        _ = await first

        #expect(store.kept(3) == nil)
        #expect(store.activity(for: 3) == .idle)
        #expect(TranscriptStore(api: client, directory: folder).kept(3) == nil)

        // And a chat that went away while its request was out.
        let gate2 = Gate()
        let host2 = "transcript-store-forgot.test"
        defer { StubURLProtocol.unregister(host: host2) }
        let client2 = api(host: host2) { _ in
            gate2.wait()
            return .json(200, #"{"transcript": {"text": "gone"}}"#)
        }
        let store2 = TranscriptStore(api: client2, directory: directory())
        async let second = store2.ask(chatID: 1, messageID: 2, attachmentID: 5)
        while store2.activity(for: 5) != .asking { await Task.yield() }
        store2.forget(attachmentIDs: [5])
        gate2.release()
        _ = await second
        #expect(store2.kept(5) == nil)
    }

    /// A chat that went away, and a logout: the words must not outlive
    /// either. Both go through `AttachmentStore`, which owns the store.
    @Test("forgetting the attachment, or logging out, takes the text")
    func forgetAndClear() {
        let host = "transcript-store-forget.test"
        defer { StubURLProtocol.unregister(host: host) }
        let client = api(host: host) { _ in .empty(500) }
        let folder = directory()
        let attachments = AttachmentStore(api: client, directory: folder)
        let store = attachments.transcripts
        store.keep(TranscriptDTO(text: "one", language: nil), source: .stored, for: 1)
        store.keep(TranscriptDTO(text: "two", language: nil), source: .stored, for: 2)

        attachments.forget(attachmentIDs: [1])
        #expect(store.kept(1) == nil)
        #expect(store.kept(2)?.text == "two")
        #expect(AttachmentStore(api: client, directory: folder).transcripts.kept(1) == nil)

        attachments.clear()
        #expect(store.kept(2) == nil)
        #expect(AttachmentStore(api: client, directory: folder).transcripts.kept(2) == nil)
    }
}
