//
//  PollSyncTests.swift
//  FamilyConnectTests
//
//  The poll pipeline against an in-memory ModelContainer and a stubbed
//  URLSession, harnessed exactly like ReactionSyncTests — the machinery it
//  is a copy of. The coordinator's ChatSocket is real but never started,
//  live frames are injected through `handle(frame:)`, and REST is routed
//  per-host through StubURLProtocol (each suite picks a unique fake host,
//  because Swift Testing runs suites in parallel).
//
//  What matters here is exactly what the protocol says is load-bearing:
//  the poll_seq guard on ONE shared apply path, the per-chat cursor that
//  advances on a live frame (even for a message we do not hold) but NEVER
//  on a poll embedded on a fetched Message, an absent poll never wiping a
//  stored one, and the optimistic vote that must not touch the seq.
//

import Foundation
import SwiftData
import Testing
@testable import FamilyConnect

@MainActor
@Suite("Poll sync")
struct PollSyncTests {

    private static let serverDate = ISO8601DateFormatter().date(from: "2026-08-19T17:05:00Z")!

    // MARK: - Harness

    @MainActor
    private struct Harness {
        /// Retained here: the coordinator holds only the mainContext, and
        /// SwiftData traps if the container backing a context deallocates.
        let container: ModelContainer
        let coordinator: ChatSyncCoordinator
        let context: ModelContext
        let host: String

        func messages() -> [MessageEntity] {
            (try? context.fetch(FetchDescriptor<MessageEntity>())) ?? []
        }

        func message(localID: String) -> MessageEntity? {
            let descriptor = FetchDescriptor<MessageEntity>(predicate: #Predicate { $0.localID == localID })
            return (try? context.fetch(descriptor))?.first
        }

        func chat(_ chatID: Int64) -> ChatEntity? {
            let descriptor = FetchDescriptor<ChatEntity>(predicate: #Predicate { $0.chatID == chatID })
            return (try? context.fetch(descriptor))?.first
        }

        /// Wait for the detached delivery `sendPoll` starts, before the
        /// container goes out of scope — SwiftData traps (and takes the
        /// whole test process with it) if a context outlives its container
        /// with work still running.
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

    // MARK: - Fixtures

    private func poll(
        seq: Int64,
        closed: Bool = false,
        pizzaVotes: [Int64] = [],
        pastaVotes: [Int64] = []
    ) -> PollDTO {
        PollDTO(
            pollSeq: seq,
            closed: closed,
            options: [
                PollOptionDTO(id: 5, text: "Pizza", votes: pizzaVotes),
                PollOptionDTO(id: 6, text: "Pasta", votes: pastaVotes),
            ])
    }

    private func dto(
        id: Int64,
        senderID: Int64 = 9,
        clientMsgID: String? = nil,
        body: String = "Pizza or pasta?",
        poll: PollDTO? = nil
    ) -> MessageDTO {
        MessageDTO(
            id: id, chatID: 42, senderID: senderID, clientMsgID: clientMsgID,
            body: body, createdAt: Self.serverDate, poll: poll)
    }

    /// The stored poll, decoded back out of the row.
    private func stored(_ row: MessageEntity?) -> PollSnapshot? {
        row?.poll
    }

    // MARK: - The seq guard

    @Test("a stale poll_seq is a no-op on both apply paths, and an absent poll never wipes")
    func staleSeqIsNoOp() throws {
        let harness = try makeHarness(host: "poll-stale.test", handler: { _ in .empty(204) })
        defer { harness.tearDown() }

        _ = harness.coordinator.upsert(dto(id: 100, poll: poll(seq: 88)), bumpUnread: false)
        harness.coordinator.applyPollState(
            messageServerID: 100, poll: poll(seq: 90, pizzaVotes: [7, 9]))

        // A stale full-state apply (an out-of-order frame): dropped.
        harness.coordinator.applyPollState(
            messageServerID: 100, poll: poll(seq: 89, pastaVotes: [11]))
        var row = try #require(harness.message(localID: "s:100"))
        #expect(row.pollSeq == 90)
        #expect(stored(row)?.options.first?.votes == [7, 9])

        // Stale EMBEDDED state (a history page fetched before the frame
        // landed): the upsert guard drops it too.
        _ = harness.coordinator.upsert(
            dto(id: 100, poll: poll(seq: 5, pastaVotes: [11])), bumpUnread: false)
        row = try #require(harness.message(localID: "s:100"))
        #expect(row.pollSeq == 90)
        #expect(stored(row)?.options.first?.votes == [7, 9])

        // A re-delivery with NO poll at all (a chat-list preview shape, or
        // any endpoint that omits it): absence is silence, never an
        // erasure — a poll dies only with its message.
        _ = harness.coordinator.upsert(dto(id: 100), bumpUnread: false)
        row = try #require(harness.message(localID: "s:100"))
        #expect(row.pollSeq == 90)
        #expect(stored(row)?.options.first?.votes == [7, 9])
        #expect(row.pollJSON != nil)
    }

    @Test("a poll first seen on a history page is stored — the first-sight insert carries it")
    func firstSightInsertCarriesPoll() throws {
        let harness = try makeHarness(host: "poll-firstsight.test", handler: { _ in .empty(204) })
        defer { harness.tearDown() }

        // The write site that is easiest to miss: the row does not exist
        // yet, so nothing UPDATES it — the poll has to ride in on the
        // initialiser, or every poll this device sees for the first time
        // in a page is silently dropped.
        _ = harness.coordinator.upsert(
            dto(id: 100, poll: poll(seq: 88, pizzaVotes: [9])), bumpUnread: false)

        let row = try #require(harness.message(localID: "s:100"))
        #expect(row.pollSeq == 88)
        let poll = try #require(stored(row))
        #expect(poll.options.map(\.text) == ["Pizza", "Pasta"])
        #expect(poll.options[0].votes == [9])
        #expect(poll.closed == false)
    }

    @Test("an embedded poll applies under the guard but must NOT advance the chat cursor")
    func embeddedPollDoesNotAdvanceCursor() throws {
        let harness = try makeHarness(host: "poll-embedded.test", handler: { _ in .empty(204) })
        defer { harness.tearDown() }

        _ = harness.coordinator.upsert(
            dto(id: 100, poll: poll(seq: 88, pizzaVotes: [9])), bumpUnread: false)
        #expect(harness.message(localID: "s:100")?.pollSeq == 88)
        // A history page proves nothing about OTHER polls' lower seqs, so
        // the catch-up feed still owes us everything after the cursor.
        #expect(harness.chat(42)?.maxPollSeq == 0)

        // …and a newer embedded copy still applies to the row.
        _ = harness.coordinator.upsert(
            dto(id: 100, poll: poll(seq: 92, pizzaVotes: [9], pastaVotes: [7])),
            bumpUnread: false)
        let row = try #require(harness.message(localID: "s:100"))
        #expect(row.pollSeq == 92)
        #expect(stored(row)?.options[1].votes == [7])
        #expect(harness.chat(42)?.maxPollSeq == 0)
    }

    // MARK: - Live frames

    @Test("a poll frame applies full state and MAX-advances the chat cursor")
    func frameAppliesAndBumpsCursor() throws {
        let harness = try makeHarness(host: "poll-frame.test", handler: { _ in .empty(204) })
        defer { harness.tearDown() }

        _ = harness.coordinator.upsert(dto(id: 100, poll: poll(seq: 88)), bumpUnread: false)
        harness.coordinator.handle(frame: .poll(PollPayload(
            chatID: 42, messageID: 100, poll: poll(seq: 89, pizzaVotes: [7, 9]))))

        var row = try #require(harness.message(localID: "s:100"))
        #expect(row.pollSeq == 89)
        #expect(stored(row)?.options[0].votes == [7, 9])
        #expect(harness.chat(42)?.maxPollSeq == 89)

        // Full-state REPLACEMENT, never a merge: a retraction arrives as
        // the whole poll with one fewer id in it.
        harness.coordinator.handle(frame: .poll(PollPayload(
            chatID: 42, messageID: 100, poll: poll(seq: 90, pizzaVotes: [9]))))
        row = try #require(harness.message(localID: "s:100"))
        #expect(stored(row)?.options[0].votes == [9])
        #expect(harness.chat(42)?.maxPollSeq == 90)

        // An out-of-order frame cannot undo a newer vote, and cannot pull
        // the cursor back either.
        harness.coordinator.handle(frame: .poll(PollPayload(
            chatID: 42, messageID: 100, poll: poll(seq: 89, pizzaVotes: [7, 9]))))
        row = try #require(harness.message(localID: "s:100"))
        #expect(row.pollSeq == 90)
        #expect(stored(row)?.options[0].votes == [9])
        #expect(harness.chat(42)?.maxPollSeq == 90)
    }

    @Test("a frame for a message we don't hold still advances the cursor, silently")
    func frameForUnknownMessageBumpsCursorOnly() throws {
        let harness = try makeHarness(host: "poll-unknown.test", handler: { _ in .empty(204) })
        defer { harness.tearDown() }

        harness.coordinator.handle(frame: .poll(PollPayload(
            chatID: 42, messageID: 999, poll: poll(seq: 94, pizzaVotes: [9]))))

        #expect(harness.messages().isEmpty)
        // Dropped state, moved cursor: the cursor is what we have
        // PROCESSED, never what we have kept. History paging re-delivers
        // the poll embedded on the Message.
        #expect(harness.chat(42)?.maxPollSeq == 94)
    }

    @Test("a closing frame closes the stored poll")
    func frameCloses() throws {
        let harness = try makeHarness(host: "poll-closeframe.test", handler: { _ in .empty(204) })
        defer { harness.tearDown() }

        _ = harness.coordinator.upsert(
            dto(id: 100, poll: poll(seq: 88, pizzaVotes: [9])), bumpUnread: false)
        harness.coordinator.handle(frame: .poll(PollPayload(
            chatID: 42, messageID: 100, poll: poll(seq: 95, closed: true, pizzaVotes: [9]))))

        let row = try #require(harness.message(localID: "s:100"))
        #expect(stored(row)?.closed == true)
        // A closed poll keeps its result.
        #expect(stored(row)?.options[0].votes == [9])
    }

    // MARK: - Voting

    @Test("voting PUTs the option, applies the authoritative reply, and never bumps the seq itself")
    func votePutPath() async throws {
        let harness = try makeHarness(host: "poll-vote.test", handler: { request in
            guard request.method == "PUT", request.url.path().hasSuffix("/vote") else {
                return .empty(204)
            }
            return .json(200, """
            {"message_id": 100,
             "poll": {"poll_seq": 91, "closed": false,
                      "options": [{"id": 5, "text": "Pizza", "votes": [9, 7]},
                                  {"id": 6, "text": "Pasta", "votes": []}]}}
            """)
        })
        defer { harness.tearDown() }

        _ = harness.coordinator.upsert(
            dto(id: 100, poll: poll(seq: 88, pizzaVotes: [9])), bumpUnread: false)
        await harness.coordinator.vote(localID: "s:100", optionID: 5)

        let puts = StubURLProtocol.requests(host: harness.host).filter { $0.method == "PUT" }
        #expect(puts.count == 1)
        #expect(puts.first?.url.path() == "/api/v1/chats/42/messages/100/vote")
        #expect(puts.first?.bodyJSON()?["option_id"] as? Int == 5)

        let row = try #require(harness.message(localID: "s:100"))
        #expect(row.pollSeq == 91)
        #expect(stored(row)?.options[0].votes == [9, 7])
    }

    @Test("the optimistic write leaves poll_seq alone, so the authoritative reply still lands")
    func optimisticWriteKeepsSeq() async throws {
        // The reply carries the seq the server minted. If the optimistic
        // write had bumped the stored seq to anything, this state would
        // fail the guard and be DROPPED — the bug this test exists for.
        let harness = try makeHarness(host: "poll-optimistic.test", handler: { request in
            guard request.method == "PUT" else { return .empty(204) }
            return .json(200, """
            {"message_id": 100,
             "poll": {"poll_seq": 89, "closed": false,
                      "options": [{"id": 5, "text": "Pizza", "votes": [7]},
                                  {"id": 6, "text": "Pasta", "votes": [11]}]}}
            """)
        })
        defer { harness.tearDown() }

        _ = harness.coordinator.upsert(
            dto(id: 100, poll: poll(seq: 88)), bumpUnread: false)
        await harness.coordinator.vote(localID: "s:100", optionID: 5)

        let row = try #require(harness.message(localID: "s:100"))
        #expect(row.pollSeq == 89)
        // The reply is authoritative in full: it also carries somebody
        // else's vote the optimistic write knew nothing about.
        #expect(stored(row)?.options[0].votes == [7])
        #expect(stored(row)?.options[1].votes == [11])
    }

    @Test("a vote reply moves the ROW but never the chat-wide poll cursor")
    func voteReplyLeavesTheCursorAlone() async throws {
        let harness = try makeHarness(host: "poll-vote-cursor.test", handler: { request in
            guard request.method == "PUT" else { return .empty(204) }
            return .json(200, """
            {"message_id": 100,
             "poll": {"poll_seq": 100, "closed": false,
                      "options": [{"id": 5, "text": "Pizza", "votes": []},
                                  {"id": 6, "text": "Pasta", "votes": [7]}]}}
            """)
        })
        defer { harness.tearDown() }

        _ = harness.coordinator.upsert(
            dto(id: 100, poll: poll(seq: 88, pizzaVotes: [9])), bumpUnread: false)
        await harness.coordinator.vote(localID: "s:100", optionID: 6)

        // The row takes the authoritative state…
        let row = try #require(harness.message(localID: "s:100"))
        #expect(row.pollSeq == 100)
        #expect(stored(row)?.options[1].votes == [7])

        // …and the cursor does NOT move, exactly as the reaction reply
        // leaves the reaction cursor alone. A cursor is a CHAT-WIDE
        // watermark and one poll's seq says nothing about another's: REST
        // works while the socket is down, so a reply carrying seq 100
        // would otherwise push the cursor past somebody else's seq 99
        // whose frame never arrived, and the next resync — comparing
        // max_poll_seq against a cursor already at 100 — would ask for
        // nothing at all. One redundant catch-up page is the cheaper
        // mistake.
        #expect(harness.chat(42)?.maxPollSeq == 0)
    }

    @Test("tapping the option already held retracts it with a DELETE")
    func voteDeletePath() async throws {
        let harness = try makeHarness(host: "poll-retract.test", handler: { request in
            guard request.method == "DELETE" else { return .empty(204) }
            return .json(200, """
            {"message_id": 100,
             "poll": {"poll_seq": 92, "closed": false,
                      "options": [{"id": 5, "text": "Pizza", "votes": []},
                                  {"id": 6, "text": "Pasta", "votes": []}]}}
            """)
        })
        defer { harness.tearDown() }

        _ = harness.coordinator.upsert(
            dto(id: 100, poll: poll(seq: 88, pizzaVotes: [7])), bumpUnread: false)
        await harness.coordinator.vote(localID: "s:100", optionID: 5)

        let deletes = StubURLProtocol.requests(host: harness.host).filter { $0.method == "DELETE" }
        #expect(deletes.count == 1)
        #expect(deletes.first?.url.path() == "/api/v1/chats/42/messages/100/vote")
        #expect(harness.message(localID: "s:100").flatMap(stored)?.options[0].votes == [])
    }

    @Test("moving my vote takes it off the option I held — one choice per member")
    func voteMovesBetweenOptions() async throws {
        let harness = try makeHarness(host: "poll-move.test", handler: { _ in
            // Answer nothing useful: the point here is the OPTIMISTIC
            // state, so the request is left to fail and the assertions run
            // against what was drawn before the reply.
            .json(500, #"{"error": {"code": "internal", "message": "boom"}}"#)
        })
        defer { harness.tearDown() }

        _ = harness.coordinator.upsert(
            dto(id: 100, poll: poll(seq: 88, pizzaVotes: [7, 9])), bumpUnread: false)

        // Drive the pure rule the optimistic write uses, so this asserts
        // the rewrite rather than the timing of an await.
        let before = try #require(harness.message(localID: "s:100").flatMap(stored))
        let after = PollPresentation.applyingVote(
            before, optionID: 6, userID: 7, retracting: false)
        #expect(after.options[0].votes == [9])
        // Appended, not inserted: the server orders votes by when they
        // were cast, and mine has just been.
        #expect(after.options[1].votes == [7])
    }

    @Test("a failed vote reverts the optimistic write")
    func voteRevertsOnError() async throws {
        let harness = try makeHarness(host: "poll-revert.test", handler: { _ in
            .json(500, #"{"error": {"code": "internal", "message": "boom"}}"#)
        })
        defer { harness.tearDown() }

        _ = harness.coordinator.upsert(
            dto(id: 100, poll: poll(seq: 88, pizzaVotes: [9])), bumpUnread: false)
        await harness.coordinator.vote(localID: "s:100", optionID: 6)

        // Back to the pre-vote state, seq untouched: a vote nobody took
        // must not stay on screen.
        let row = try #require(harness.message(localID: "s:100"))
        #expect(row.pollSeq == 88)
        #expect(stored(row)?.options[0].votes == [9])
        #expect(stored(row)?.options[1].votes == [])
    }

    @Test("a revert never clobbers a newer authoritative state that landed mid-vote")
    func voteRevertYieldsToNewerState() async throws {
        // The frame lands WHILE the request is in flight: the stub's
        // handler runs on the URLProtocol's thread, so the frame is
        // injected from the handler and is applied before the failure
        // comes back.
        // A reference box, not a captured `var`: mutating a local after a
        // Sendable closure has captured it is a warning (and an error in
        // the Swift 6 language mode), and the closure genuinely has to see
        // a harness that does not exist until makeHarness returns.
        final class Box: @unchecked Sendable { var harness: Harness? }
        let box = Box()
        let harness = try makeHarness(host: "poll-revert-race.test", handler: { _ in
            if let harnessBox = box.harness {
                DispatchQueue.main.sync {
                    MainActor.assumeIsolated {
                        harnessBox.coordinator.handle(frame: .poll(PollPayload(
                            chatID: 42, messageID: 100,
                            poll: PollDTO(
                                pollSeq: 95, closed: false,
                                options: [
                                    PollOptionDTO(id: 5, text: "Pizza", votes: [11]),
                                    PollOptionDTO(id: 6, text: "Pasta", votes: []),
                                ]))))
                    }
                }
            }
            return .json(500, #"{"error": {"code": "internal", "message": "boom"}}"#)
        })
        box.harness = harness
        defer { harness.tearDown() }

        _ = harness.coordinator.upsert(
            dto(id: 100, poll: poll(seq: 88, pizzaVotes: [9])), bumpUnread: false)
        await harness.coordinator.vote(localID: "s:100", optionID: 6)

        // The revert saw a moved seq and stood down.
        let row = try #require(harness.message(localID: "s:100"))
        #expect(row.pollSeq == 95)
        #expect(stored(row)?.options[0].votes == [11])
    }

    @Test("a closed poll takes no vote at all — no request, no optimistic write")
    func closedPollRefusesVotes() async throws {
        let harness = try makeHarness(host: "poll-closed.test", handler: { _ in .empty(204) })
        defer { harness.tearDown() }

        _ = harness.coordinator.upsert(
            dto(id: 100, poll: poll(seq: 95, closed: true, pizzaVotes: [9])), bumpUnread: false)
        await harness.coordinator.vote(localID: "s:100", optionID: 5)

        #expect(StubURLProtocol.requests(host: harness.host).isEmpty)
        #expect(harness.message(localID: "s:100").flatMap(stored)?.options[0].votes == [9])
    }

    @Test("voting refuses a message the server has never seen")
    func voteNeedsServerID() async throws {
        let harness = try makeHarness(host: "poll-noserver.test", handler: { _ in .empty(204) })
        defer { harness.tearDown() }

        let localID = try #require(harness.coordinator.enqueue(
            body: "Pizza or pasta?", in: 42, pollOptions: ["Pizza", "Pasta"]))
        await harness.coordinator.vote(localID: localID, optionID: -1)

        #expect(StubURLProtocol.requests(host: harness.host).isEmpty)
    }

    @Test("an unknown option id is refused locally")
    func voteRefusesUnknownOption() async throws {
        let harness = try makeHarness(host: "poll-badoption.test", handler: { _ in .empty(204) })
        defer { harness.tearDown() }

        _ = harness.coordinator.upsert(dto(id: 100, poll: poll(seq: 88)), bumpUnread: false)
        await harness.coordinator.vote(localID: "s:100", optionID: 999)

        #expect(StubURLProtocol.requests(host: harness.host).isEmpty)
    }

    // MARK: - Closing

    @Test("a close reply moves the ROW but never the chat-wide poll cursor")
    func closeReplyLeavesTheCursorAlone() async throws {
        let harness = try makeHarness(host: "poll-close-cursor.test", handler: { request in
            guard request.method == "POST" else { return .empty(204) }
            return .json(200, """
            {"message_id": 100,
             "poll": {"poll_seq": 101, "closed": true,
                      "options": [{"id": 5, "text": "Pizza", "votes": [9]},
                                  {"id": 6, "text": "Pasta", "votes": []}]}}
            """)
        })
        defer { harness.tearDown() }

        _ = harness.coordinator.upsert(
            dto(id: 100, poll: poll(seq: 88, pizzaVotes: [9])), bumpUnread: false)
        #expect(await harness.coordinator.closePoll(localID: "s:100"))

        let row = try #require(harness.message(localID: "s:100"))
        #expect(row.pollSeq == 101)
        #expect(stored(row)?.closed == true)
        // Same rule as the vote reply — see above.
        #expect(harness.chat(42)?.maxPollSeq == 0)
    }

    @Test("closing a poll POSTs and applies the authoritative reply")
    func closePoll() async throws {
        let harness = try makeHarness(host: "poll-close.test", handler: { request in
            guard request.method == "POST" else { return .empty(204) }
            return .json(200, """
            {"message_id": 100,
             "poll": {"poll_seq": 96, "closed": true,
                      "options": [{"id": 5, "text": "Pizza", "votes": [7, 9]},
                                  {"id": 6, "text": "Pasta", "votes": []}]}}
            """)
        })
        defer { harness.tearDown() }

        _ = harness.coordinator.upsert(
            dto(id: 100, senderID: 7, poll: poll(seq: 88, pizzaVotes: [7, 9])), bumpUnread: false)
        let closed = await harness.coordinator.closePoll(localID: "s:100")

        #expect(closed)
        let posts = StubURLProtocol.requests(host: harness.host).filter { $0.method == "POST" }
        #expect(posts.first?.url.path() == "/api/v1/chats/42/messages/100/poll/close")
        let row = try #require(harness.message(localID: "s:100"))
        #expect(stored(row)?.closed == true)
        #expect(row.pollSeq == 96)
    }

    @Test("a refused close changes nothing and says so")
    func closeRefused() async throws {
        let harness = try makeHarness(host: "poll-close-refused.test", handler: { _ in
            .json(403, #"{"error": {"code": "not_message_author", "message": "nope"}}"#)
        })
        defer { harness.tearDown() }

        _ = harness.coordinator.upsert(dto(id: 100, poll: poll(seq: 88)), bumpUnread: false)
        let closed = await harness.coordinator.closePoll(localID: "s:100")

        #expect(!closed)
        // NOT optimistic: a poll that stopped taking votes it would in
        // fact still take is worse than a button that visibly did nothing.
        #expect(harness.message(localID: "s:100").flatMap(stored)?.closed == false)
    }

    // MARK: - Sending

    @Test("sending a poll posts an ordinary message whose body is the question")
    func sendPoll() async throws {
        let harness = try makeHarness(host: "poll-send.test", handler: { request in
            guard request.method == "POST", request.url.path().hasSuffix("/messages") else {
                return .empty(204)
            }
            return .json(201, """
            {"message": {"id": 1340, "chat_id": 42, "sender_id": 7,
                         "client_msg_id": "\(request.bodyJSON()?["client_msg_id"] as? String ?? "")",
                         "body": "Pizza or pasta?", "created_at": "2026-08-19T17:05:00Z",
                         "poll": {"poll_seq": 88, "closed": false,
                                  "options": [{"id": 5, "text": "Pizza", "votes": []},
                                              {"id": 6, "text": "Pasta", "votes": []}]}}}
            """)
        })
        defer { harness.tearDown() }

        let localID = try #require(harness.coordinator.sendPoll(
            question: "Pizza or pasta?", options: ["Pizza", " Pasta "], in: 42))

        // The pending row draws as a poll straight away: provisional
        // NEGATIVE ids (the server's are positive) and seq 0, which is
        // what lets the ack's real poll pass the guard.
        let pending = try #require(harness.message(localID: localID))
        #expect(pending.body == "Pizza or pasta?")
        #expect(pending.pollSeq == 0)
        let provisional = try #require(stored(pending))
        #expect(provisional.options.map(\.text) == ["Pizza", "Pasta"])
        #expect(provisional.options.allSatisfy { $0.id < 0 })
        #expect(provisional.closed == false)
        // The chat-list preview is the question, with no new case.
        #expect(harness.chat(42)?.lastMessagePreview == "Pizza or pasta?")

        await harness.settle()

        let posts = StubURLProtocol.requests(host: harness.host).filter { $0.method == "POST" }
        #expect(posts.count == 1)
        let body = try #require(posts.first?.bodyJSON())
        #expect(body["body"] as? String == "Pizza or pasta?")
        #expect((body["poll"] as? [String: Any])?["options"] as? [String] == ["Pizza", "Pasta"])

        // The ack reconciles into the SAME row, and the authoritative poll
        // (seq 88 > 0) replaces the provisional one.
        let row = try #require(harness.message(localID: localID))
        #expect(row.serverID == 1340)
        #expect(row.pollSeq == 88)
        #expect(stored(row)?.options.map(\.id) == [5, 6])
    }

    @Test("a poll may be a reply, and carries the quote it was primed with")
    func sendPollCarriesTheReply() async throws {
        // A poll started while a reply was primed used to drop the quote on
        // the floor: `sendPoll` had no `replyTo` at all. The server takes
        // `reply_to_message_id` beside `poll` — only `poll` and
        // `attachment_id` are mutually exclusive — so the quote belongs on
        // the wire, and the composer that armed it clears it at the door.
        let harness = try makeHarness(host: "poll-reply.test", handler: { request in
            guard request.method == "POST", request.url.path().hasSuffix("/messages") else {
                return .empty(204)
            }
            return .json(201, """
            {"message": {"id": 1341, "chat_id": 42, "sender_id": 7,
                         "client_msg_id": "\(request.bodyJSON()?["client_msg_id"] as? String ?? "")",
                         "body": "Pizza or pasta?", "created_at": "2026-08-19T17:05:00Z",
                         "reply_to": {"message_id": 900, "sender_id": 9, "excerpt": "What's for dinner?"},
                         "poll": {"poll_seq": 88, "closed": false,
                                  "options": [{"id": 5, "text": "Pizza", "votes": []},
                                              {"id": 6, "text": "Pasta", "votes": []}]}}}
            """)
        })
        defer { harness.tearDown() }

        let quote = ReplyToDTO(messageID: 900, senderID: 9, excerpt: "What's for dinner?")
        let localID = try #require(harness.coordinator.sendPoll(
            question: "Pizza or pasta?", options: ["Pizza", "Pasta"], in: 42, replyTo: quote))

        // The pending bubble shows its quote at once, like every other
        // optimistic send.
        let pending = try #require(harness.message(localID: localID))
        #expect(pending.replyToMessageID == 900)
        #expect(pending.replyExcerpt == "What's for dinner?")

        await harness.settle()

        let posts = StubURLProtocol.requests(host: harness.host).filter { $0.method == "POST" }
        let body = try #require(posts.first?.bodyJSON())
        #expect(body["reply_to_message_id"] as? Int == 900)
        #expect((body["poll"] as? [String: Any])?["options"] as? [String] == ["Pizza", "Pasta"])
    }

    @Test("a poll with too few or duplicate options never leaves the device")
    func sendPollRefusesIllegalOptions() async throws {
        let harness = try makeHarness(host: "poll-send-invalid.test", handler: { _ in .empty(204) })
        defer { harness.tearDown() }

        #expect(harness.coordinator.sendPoll(question: "One?", options: ["Only"], in: 42) == nil)
        #expect(harness.coordinator.sendPoll(
            question: "Same?", options: ["Pizza", "pizza"], in: 42) == nil)
        #expect(harness.coordinator.sendPoll(
            question: "  ", options: ["Pizza", "Pasta"], in: 42) == nil)
        #expect(harness.coordinator.sendPoll(
            question: "Too many?",
            options: (1...11).map { "Option \($0)" },
            in: 42) == nil)

        await harness.settle()
        #expect(harness.messages().isEmpty)
        #expect(StubURLProtocol.requests(host: harness.host).isEmpty)
    }

    // MARK: - Resync catch-up

    @Test("resync repairs a missed vote and advances the cursor past unknown messages")
    func resyncPollCatchUp() async throws {
        let harness = try makeHarness(host: "poll-resync.test", handler: { request in
            switch request.url.path() {
            case "/api/v1/me":
                return .json(200, """
                {"user": {"id": 7, "username": "anna", "display_name": "Anna",
                          "created_at": "2026-08-19T17:00:00Z"},
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
                // last_message id 100 == the local cursor, so no message
                // catch-up runs; max_poll_seq 94 > local 0 triggers the
                // poll catch-up and nothing else.
                return .json(200, """
                {"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths",
                                     "peer_user_id": null},
                            "last_message": {"id": 100, "chat_id": 42, "sender_id": 9,
                                             "client_msg_id": null, "body": "Pizza or pasta?",
                                             "created_at": "2026-08-19T17:05:00Z"},
                            "unread_count": 0, "max_poll_seq": 94}]}
                """)
            case "/api/v1/chats/42/polls":
                // One state for a held message, one for a message this
                // client never loaded (deep history) — the latter is
                // dropped but its seq still moves the cursor.
                return .json(200, """
                {"polls": [
                  {"message_id": 100,
                   "poll": {"poll_seq": 92, "closed": false,
                            "options": [{"id": 5, "text": "Pizza", "votes": [9, 11]},
                                        {"id": 6, "text": "Pasta", "votes": []}]}},
                  {"message_id": 999,
                   "poll": {"poll_seq": 94, "closed": true,
                            "options": [{"id": 8, "text": "Beach", "votes": [7]}]}}]}
                """)
            default:
                return .json(404, #"{"error": {"code": "chat_not_found", "message": "?"}}"#)
            }
        })
        defer { harness.tearDown() }

        _ = harness.coordinator.upsert(
            dto(id: 100, poll: poll(seq: 88, pizzaVotes: [9])), bumpUnread: false)
        await harness.coordinator.resync()

        // The held poll got the vote it missed…
        let row = try #require(harness.message(localID: "s:100"))
        #expect(row.pollSeq == 92)
        #expect(stored(row)?.options[0].votes == [9, 11])

        // …the unknown one was dropped without inventing a row…
        #expect(harness.message(localID: "s:999") == nil)

        // …and the cursor covers the whole page, so the next resync
        // (server max still 94) plans no poll step at all.
        #expect(harness.chat(42)?.maxPollSeq == 94)

        let pollGets = StubURLProtocol.requests(host: harness.host)
            .filter { $0.url.path() == "/api/v1/chats/42/polls" }
        #expect(pollGets.count == 1)
        #expect(pollGets.first?.url.query()?.contains("after_seq=0") == true)
    }

    @Test("a chat with no polls costs no catch-up request")
    func resyncSkipsPolllessChat() async throws {
        let harness = try makeHarness(host: "poll-resync-none.test", handler: { request in
            switch request.url.path() {
            case "/api/v1/me":
                return .json(200, """
                {"user": {"id": 7, "username": "anna", "display_name": "Anna",
                          "created_at": "2026-08-19T17:00:00Z"},
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
                // max_poll_seq omitted entirely — the family has never run
                // a poll.
                return .json(200, """
                {"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths",
                                     "peer_user_id": null},
                            "last_message": null, "unread_count": 0}]}
                """)
            default:
                return .json(404, #"{"error": {"code": "chat_not_found", "message": "?"}}"#)
            }
        })
        defer { harness.tearDown() }

        await harness.coordinator.resync()

        let pollGets = StubURLProtocol.requests(host: harness.host)
            .filter { $0.url.path().hasSuffix("/polls") }
        #expect(pollGets.isEmpty)
    }
}

// MARK: - The pure rules

@Suite("Poll presentation")
struct PollPresentationTests {

    private func poll(
        closed: Bool = false,
        pizza: [Int64] = [],
        pasta: [Int64] = [],
        salad: [Int64] = []
    ) -> PollSnapshot {
        PollSnapshot(
            pollSeq: 88,
            closed: closed,
            options: [
                PollOptionSnapshot(id: 5, text: "Pizza", votes: pizza),
                PollOptionSnapshot(id: 6, text: "Pasta", votes: pasta),
                PollOptionSnapshot(id: 7, text: "Salad", votes: salad),
            ])
    }

    @Test("the option a reader holds, and what a tap on it means")
    func myOption() {
        let state = poll(pizza: [9], pasta: [7, 11])
        #expect(PollPresentation.myOptionID(in: state, currentUserID: 7) == 6)
        #expect(PollPresentation.myOptionID(in: state, currentUserID: 12) == nil)
        // Tapping my own choice clears it; anything else casts.
        #expect(PollPresentation.tapRetracts(optionID: 6, in: state, currentUserID: 7))
        #expect(!PollPresentation.tapRetracts(optionID: 5, in: state, currentUserID: 7))
        #expect(!PollPresentation.tapRetracts(optionID: 5, in: state, currentUserID: 12))
    }

    @Test("voters are counted once, however the state arrived")
    func voterCount() {
        #expect(PollPresentation.voterIDs(in: poll(pizza: [9], pasta: [7, 11])).count == 3)
        #expect(PollPresentation.voterIDs(in: poll()).isEmpty)
        // A member who somehow appears twice is still one voter, so the
        // footer can never claim more people than the family has.
        #expect(PollPresentation.voterIDs(in: poll(pizza: [9], pasta: [9])).count == 1)
    }

    @Test("bars are a share of the votes cast, and an untouched poll draws empty")
    func fractions() {
        let state = poll(pizza: [9, 11], pasta: [7])
        #expect(PollPresentation.fraction(of: state.options[0], in: state) == 2.0 / 3.0)
        #expect(PollPresentation.fraction(of: state.options[1], in: state) == 1.0 / 3.0)
        #expect(PollPresentation.fraction(of: state.options[2], in: state) == 0)
        // Nobody has voted: every bar empty, not every bar full.
        let fresh = poll()
        #expect(fresh.options.allSatisfy { PollPresentation.fraction(of: $0, in: fresh) == 0 })
    }

    @Test("the optimistic vote moves one member's id and appends it")
    func applyingVote() {
        let state = poll(pizza: [9, 7], pasta: [11])
        let moved = PollPresentation.applyingVote(state, optionID: 6, userID: 7, retracting: false)
        #expect(moved.options[0].votes == [9])
        // Appended, matching the server's cast order.
        #expect(moved.options[1].votes == [11, 7])
        // One choice per member: the id is nowhere else.
        #expect(moved.options[2].votes.isEmpty)

        let retracted = PollPresentation.applyingVote(state, optionID: 5, userID: 7, retracting: true)
        #expect(retracted.options[0].votes == [9])
        #expect(retracted.options[1].votes == [11])

        // The seq is untouched, which is what lets the authoritative reply
        // pass the guard rather than be dropped as stale.
        #expect(moved.pollSeq == state.pollSeq)
        #expect(retracted.pollSeq == state.pollSeq)
    }

    @Test("the composer refuses exactly what the server would refuse")
    func sanitizedOptions() {
        // Trimmed, and the cleaned list is what comes back.
        #expect(PollPresentation.sanitizedOptions([" Pizza ", "Pasta"]) == ["Pizza", "Pasta"])
        // Blank rows are dropped, not counted.
        #expect(PollPresentation.sanitizedOptions(["Pizza", "Pasta", "  ", ""]) == ["Pizza", "Pasta"])
        // Fewer than two, more than ten.
        #expect(PollPresentation.sanitizedOptions(["Pizza"]) == nil)
        #expect(PollPresentation.sanitizedOptions([]) == nil)
        #expect(PollPresentation.sanitizedOptions((1...10).map { "Option \($0)" })?.count == 10)
        #expect(PollPresentation.sanitizedOptions((1...11).map { "Option \($0)" }) == nil)
        // No two the same ignoring case.
        #expect(PollPresentation.sanitizedOptions(["Pizza", "PIZZA"]) == nil)
        #expect(PollPresentation.sanitizedOptions(["Pizza", " pizza "]) == nil)
        // 100 characters is the ceiling.
        #expect(PollPresentation.sanitizedOptions([String(repeating: "a", count: 100), "Pasta"])?.count == 2)
        #expect(PollPresentation.sanitizedOptions([String(repeating: "a", count: 101), "Pasta"]) == nil)
    }

    @Test("a stored poll round-trips through the wire shape it is persisted in")
    func storedShapeIsTheWireShape() throws {
        // MessageEntity persists the poll as the server's own JSON, so a
        // stored row can be diffed against protocol.md — and so the same
        // decoder reads a poll whichever direction it came from.
        let json = """
        {"poll_seq": 88, "closed": false,
         "options": [{"id": 5, "text": "Pizza", "votes": [7, 9]},
                     {"id": 6, "text": "Pasta", "votes": []}]}
        """
        let snapshot = try JSONDecoder().decode(PollSnapshot.self, from: Data(json.utf8))
        #expect(snapshot.pollSeq == 88)
        #expect(snapshot.closed == false)
        #expect(snapshot.options[0].votes == [7, 9])
        #expect(snapshot.options[1].votes.isEmpty)

        let reencoded = try JSONEncoder().encode(snapshot)
        let fields = try #require(
            (try JSONSerialization.jsonObject(with: reencoded)) as? [String: Any])
        #expect(fields["poll_seq"] as? Int == 88)
        #expect(fields["closed"] as? Bool == false)
        #expect((fields["options"] as? [[String: Any]])?.count == 2)
    }
}
