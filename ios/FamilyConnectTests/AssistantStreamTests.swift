//
//  AssistantStreamTests.swift
//  FamilyConnectTests
//
//  The assistant's reply arrives twice over: as fragments while it is being
//  written, and as one authoritative body when it is done
//  (docs/protocol.md, "The assistant").
//
//  The rule that matters is which one wins. Deltas carry no `edit_seq`, so
//  they must never be able to outlive or overwrite the finished text — and a
//  client that missed every fragment must still end up with exactly the same
//  body as one that saw them all. That is the whole reason the reply is an
//  ordinary message finished through the edit path rather than a bespoke
//  streaming object.
//

import Foundation
import SwiftData
import Testing
@testable import FamilyConnect

@MainActor
@Suite("Assistant streaming")
struct AssistantStreamTests {

    private static let stamp = ISO8601DateFormatter().date(from: "2026-08-23T12:00:00Z")!

    @MainActor
    private struct Harness {
        /// HELD, not just used. Letting the container deallocate while the
        /// coordinator still has a context traps inside SwiftData and takes
        /// the whole test process with it — every case in the suite then
        /// reports "failed" at 0.000 s with no message, which is the only
        /// symptom you get.
        let container: ModelContainer
        let coordinator: ChatSyncCoordinator
        let context: ModelContext
        let host: String

        func message(_ serverID: Int64) -> MessageEntity? {
            let descriptor = FetchDescriptor<MessageEntity>(
                predicate: #Predicate { $0.serverID == serverID })
            return (try? context.fetch(descriptor))?.first
        }

        func tearDown() { StubURLProtocol.unregister(host: host) }
    }

    /// `chatKind` decides which half of the "is this the assistant" rule
    /// applies: in an `ai` chat, two participants mean anything not mine is
    /// its; in the family chat there are many senders and only its reserved
    /// account names it.
    private func makeHarness(host: String, chatKind: String = "ai") throws -> Harness {
        StubURLProtocol.register(host: host, handler: { _ in .empty(204) })
        let configuration = ModelConfiguration(isStoredInMemoryOnly: true)
        let container = try ModelContainer(
            for: ChatEntity.self, MessageEntity.self, MemberEntity.self, NoteEntity.self,
            PendingMediaItemEntity.self,
            configurations: configuration)
        let api = APIClient(
            serverURL: URL(string: "https://\(host)")!,
            session: StubURLProtocol.makeSession())
        let coordinator = ChatSyncCoordinator(modelContainer: container, api: api)
        coordinator.currentUserIDOverride = 7
        let chat = ChatEntity(chatID: 42, kind: chatKind, pinRank: 0, title: "Assistant")
        container.mainContext.insert(chat)
        try container.mainContext.save()
        return Harness(
            container: container, coordinator: coordinator,
            context: container.mainContext, host: host)
    }

    /// The empty row the server creates before it starts generating; the
    /// assistant sends under its own reserved account.
    private func placeholder(
        id: Int64,
        body: String = "",
        editSeq: Int64? = nil
    ) -> MessageDTO {
        MessageDTO(
            id: id, chatID: 42, senderID: 99, clientMsgID: nil,
            body: body, createdAt: Self.stamp,
            editedAt: editSeq == nil ? nil : Self.stamp, editSeq: editSeq)
    }

    @Test("fragments accumulate into the placeholder")
    func deltasAccumulate() throws {
        let harness = try makeHarness(host: "ai-deltas.test")
        defer { harness.tearDown() }
        let coordinator = harness.coordinator

        coordinator.handle(frame: .message(placeholder(id: 1339)))
        for fragment in ["Sure", " — the ", "answer."] {
            coordinator.handle(frame: .aiDelta(chatID: 42, messageID: 1339, text: fragment))
        }

        #expect(harness.message(1339)?.body == "Sure — the answer.")
        #expect(coordinator.streamingMessageIDs.contains(1339))
    }

    @Test("the finished body replaces whatever the fragments built")
    func theEditWins() throws {
        let harness = try makeHarness(host: "ai-edit-wins.test")
        defer { harness.tearDown() }
        let coordinator = harness.coordinator

        coordinator.handle(frame: .message(placeholder(id: 1339)))
        coordinator.handle(frame: .aiDelta(chatID: 42, messageID: 1339, text: "Sure — the "))

        let finished = placeholder(id: 1339, body: "Sure — the answer.", editSeq: 91)
        coordinator.handle(frame: .messageEdited(finished))

        #expect(harness.message(1339)?.body == "Sure — the answer.")
        // No longer being written, so the bubble stops showing a cursor.
        #expect(!coordinator.streamingMessageIDs.contains(1339))
    }

    /// The point of finishing through the edit path: a device that was
    /// asleep for the whole reply needs no special path to catch up.
    @Test("a client that missed every fragment still ends up correct")
    func missingEveryDeltaIsSurvivable() throws {
        let harness = try makeHarness(host: "ai-missed.test")
        defer { harness.tearDown() }
        let coordinator = harness.coordinator

        coordinator.handle(frame: .message(placeholder(id: 1339)))
        let finished = placeholder(id: 1339, body: "Sure — the answer.", editSeq: 91)
        coordinator.handle(frame: .messageEdited(finished))

        #expect(harness.message(1339)?.body == "Sure — the answer.")
    }

    @Test("a failed reply keeps what arrived and stops streaming")
    func errorKeepsThePartialAnswer() throws {
        let harness = try makeHarness(host: "ai-error.test")
        defer { harness.tearDown() }
        let coordinator = harness.coordinator

        coordinator.handle(frame: .message(placeholder(id: 1339)))
        coordinator.handle(frame: .aiDelta(chatID: 42, messageID: 1339, text: "Half an "))
        coordinator.handle(frame: .aiError(chatID: 42, messageID: 1339))

        // A partial answer is worth more than a bubble that never resolves.
        #expect(harness.message(1339)?.body == "Half an ")
        #expect(!coordinator.streamingMessageIDs.contains(1339))
    }

    // MARK: - A picture answer, which streams nothing at all

    /// The empty row IS the progress indicator.
    ///
    /// A text answer reaches that state a beat later, on its first
    /// fragment. A picture answer never does: an image model produces no
    /// token stream and the server sends no `ai_delta` frames for one, so
    /// the row would sit blank from the moment it appeared until the
    /// picture landed — which can be many seconds (protocol.md, "How a
    /// picture comes back", step 3). Marking it working when it is FANNED
    /// OUT rather than when the first fragment arrives is what gives both
    /// kinds of answer the same "still working" bubble.
    @Test("an empty assistant row is working from the moment it appears")
    func theEmptyRowIsTheWorkingState() throws {
        let harness = try makeHarness(host: "ai-empty-working.test")
        defer { harness.tearDown() }

        harness.coordinator.handle(frame: .message(placeholder(id: 1339)))
        #expect(harness.coordinator.isAwaitingAssistant(messageID: 1339))
        #expect(!harness.coordinator.assistantAnswerFailed(messageID: 1339))
    }

    /// THE RELAUNCH CASE, which is the one the in-memory set could never
    /// answer.
    ///
    /// A history page and a cold start apply through `upsert` and raise no
    /// frame, so nothing populates `streamingMessageIDs` — and a member who
    /// quit while a `/draw` was still generating came back to a completely
    /// blank bubble: no cursor, no error, nothing, for as long as it never
    /// arrived. protocol.md says the empty row IS the "still working"
    /// state, and says it about the message rather than about one process,
    /// so the row is what answers.
    @Test("a waiting row loaded from history is working, with no live frame")
    func aHistoryRowIsTheWorkingState() throws {
        let harness = try makeHarness(host: "ai-history-working.test")
        defer { harness.tearDown() }
        let coordinator = harness.coordinator

        // The path a page of history takes: no `.message` frame, so
        // `noteAssistantAnswerStarted` never runs.
        _ = coordinator.upsert(placeholder(id: 1339), bumpUnread: false)

        #expect(coordinator.streamingMessageIDs.isEmpty, "nothing marked this row live")
        #expect(coordinator.isAwaitingAssistant(messageID: 1339))
    }

    /// The same row asked the way a BUBBLE asks it — off the laid-out
    /// snapshot, with the chat's kind in hand and no store lookup.
    @Test("a waiting row reads as working from the snapshot a bubble holds")
    func aSnapshotRowIsTheWorkingState() throws {
        let harness = try makeHarness(host: "ai-history-snapshot.test")
        defer { harness.tearDown() }
        let coordinator = harness.coordinator

        _ = coordinator.upsert(placeholder(id: 1339), bumpUnread: false)
        let row = try #require(harness.message(1339))
        let snapshot = MessageSnapshot(row)

        #expect(coordinator.isAwaitingAssistant(snapshot, isAssistantChat: true))
        // …and the same row in a chat where the assistant is not the only
        // other participant is nobody's answer.
        #expect(!coordinator.isAwaitingAssistant(snapshot, isAssistantChat: false))
    }

    /// A row that carries something is finished, however it was loaded —
    /// which is what stops a picture answer pulsing under its own picture
    /// after the relaunch that drew it.
    @Test("a history row that carries a picture is not working")
    func aHistoryRowWithAPictureIsNotWorking() throws {
        let harness = try makeHarness(host: "ai-history-picture.test")
        defer { harness.tearDown() }
        let coordinator = harness.coordinator

        let picture = AttachmentDTO(
            id: 77, kind: AttachmentDTO.Kind.photo, mime: "image/png", size: 1,
            width: 512, height: 512, durationMS: nil, hasPreview: false, name: nil,
            latitude: nil, longitude: nil, accuracyM: nil)
        _ = coordinator.upsert(
            MessageDTO(
                id: 1339, chatID: 42, senderID: 99, clientMsgID: nil,
                body: "", createdAt: Self.stamp, attachments: [picture]),
            bumpUnread: false)

        #expect(!coordinator.isAwaitingAssistant(messageID: 1339))
    }

    /// An `ai_error` this process saw outranks the row's own shape: the two
    /// are drawn in the same place, and the newer fact wins.
    @Test("a failure this launch saw beats the row's empty shape")
    func aFailureBeatsTheEmptyRow() throws {
        let harness = try makeHarness(host: "ai-failed-beats-row.test")
        defer { harness.tearDown() }
        let coordinator = harness.coordinator

        _ = coordinator.upsert(placeholder(id: 1339), bumpUnread: false)
        coordinator.handle(frame: .aiError(chatID: 42, messageID: 1339))

        #expect(!coordinator.isAwaitingAssistant(messageID: 1339))
        #expect(coordinator.assistantAnswerFailed(messageID: 1339))
    }

    /// …and stops the moment the picture does arrive, so the cursor is not
    /// left sitting under a finished answer.
    @Test("the picture arriving ends the working state")
    func thePictureEndsTheWorkingState() throws {
        let harness = try makeHarness(host: "ai-picture-done.test")
        defer { harness.tearDown() }

        harness.coordinator.handle(frame: .message(placeholder(id: 1339)))
        let picture = AttachmentDTO(
            id: 77, kind: AttachmentDTO.Kind.photo, mime: "image/png", size: 1,
            width: 512, height: 512, durationMS: nil, hasPreview: false, name: nil,
            latitude: nil, longitude: nil, accuracyM: nil)
        harness.coordinator.handle(frame: .messageEdited(
            MessageDTO(
                id: 1339, chatID: 42, senderID: 99, clientMsgID: nil,
                body: "", createdAt: Self.stamp,
                editedAt: Self.stamp, editSeq: 12, attachments: [picture])))

        #expect(!harness.coordinator.isAwaitingAssistant(messageID: 1339))
        #expect(harness.message(1339)?.attachmentList.count == 1)
    }

    /// The row that "has to have somewhere to fail".
    ///
    /// This is the reason the empty row is created BEFORE the provider is
    /// called rather than the finished picture arriving as a message of its
    /// own. A text answer that stops midway has its partial text to show
    /// for itself; a picture answer has nothing — the body stays empty by
    /// design and the attachment never came — so the failure has to be
    /// remembered or the bubble is simply blank forever.
    @Test("a picture answer that fails is remembered, not left blank")
    func aFailedPictureAnswerIsRemembered() throws {
        let harness = try makeHarness(host: "ai-picture-failed.test")
        defer { harness.tearDown() }

        harness.coordinator.handle(frame: .message(placeholder(id: 1339)))
        harness.coordinator.handle(frame: .aiError(chatID: 42, messageID: 1339))

        #expect(!harness.coordinator.isAwaitingAssistant(messageID: 1339))
        #expect(harness.coordinator.assistantAnswerFailed(messageID: 1339))
        // The row itself keeps whatever it has, which here is nothing.
        #expect(harness.message(1339)?.body.isEmpty == true)
        #expect(harness.message(1339)?.attachmentList.isEmpty == true)
    }

    /// A late edit racing an `ai_error` must not leave the row apologising
    /// underneath a finished answer.
    @Test("an answer that lands after the error clears it")
    func aLateAnswerClearsTheFailure() throws {
        let harness = try makeHarness(host: "ai-late-answer.test")
        defer { harness.tearDown() }

        harness.coordinator.handle(frame: .message(placeholder(id: 1339)))
        harness.coordinator.handle(frame: .aiError(chatID: 42, messageID: 1339))
        harness.coordinator.handle(frame: .messageEdited(
            placeholder(id: 1339, body: "Here you go.", editSeq: 12)))

        #expect(!harness.coordinator.assistantAnswerFailed(messageID: 1339))
        #expect(harness.message(1339)?.body == "Here you go.")
    }

    /// MY OWN message is never the assistant's answer, whatever it carries.
    /// An empty own row is a send in flight, not something to draw a cursor
    /// on.
    @Test("my own empty message is not an assistant answer")
    func myOwnEmptyMessageIsNotAnAnswer() throws {
        let harness = try makeHarness(host: "ai-own-empty.test")
        defer { harness.tearDown() }

        harness.coordinator.handle(frame: .message(
            MessageDTO(
                id: 500, chatID: 42, senderID: 7, clientMsgID: nil,
                body: "", createdAt: Self.stamp)))
        #expect(!harness.coordinator.isAwaitingAssistant(messageID: 500))
    }

    /// In the FAMILY chat there is no two-participants shortcut, so an
    /// empty message from an ordinary member is not an assistant answer
    /// either — only its reserved account names it there.
    @Test("an empty family-chat message from a member is not an assistant answer")
    func anEmptyFamilyMessageIsNotAnAnswer() throws {
        let harness = try makeHarness(host: "ai-family-empty.test", chatKind: "family")
        defer { harness.tearDown() }

        harness.coordinator.handle(frame: .message(placeholder(id: 1339)))
        #expect(!harness.coordinator.isAwaitingAssistant(messageID: 1339))
    }

    /// A message that carries something is not a waiting row, even in the
    /// assistant's own chat — the state exists for a row with nothing in it.
    @Test("a message that already carries something is not a waiting row")
    func aFullMessageIsNotAWaitingRow() throws {
        let harness = try makeHarness(host: "ai-nonempty.test")
        defer { harness.tearDown() }

        harness.coordinator.handle(frame: .message(placeholder(id: 1339, body: "Hello.")))
        #expect(!harness.coordinator.isAwaitingAssistant(messageID: 1339))
    }

    @Test("a fragment for a row this device does not have is ignored")
    func unknownMessageIsIgnored() throws {
        let harness = try makeHarness(host: "ai-unknown.test")
        defer { harness.tearDown() }
        let coordinator = harness.coordinator

        // No crash, no phantom row.
        coordinator.handle(frame: .aiDelta(chatID: 42, messageID: 4242, text: "hello"))
        #expect(harness.message(4242) == nil)
    }
}
