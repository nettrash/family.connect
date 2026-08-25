//
//  UnreadRulesTests.swift
//  FamilyConnectTests
//
//  What is allowed to clear the red badge, and — mostly — what is not.
//
//  The server's read marker is monotonic, so every one of these is a test
//  about something PERMANENT: a read this client reports by mistake can
//  never be taken back, on any device that person owns. The suite is
//  therefore written from the other side: each test names a thing that
//  looks like reading and is not (appearing, being selected in a window
//  nobody is looking at, coming back to the foreground, a resync), and
//  asserts the count survives it.
//
//  Harnessed like SendPipelineTests: an in-memory ModelContainer, a
//  stubbed URLSession routed per fake host, and a real-but-never-started
//  ChatSocket — so every socket send throws `notConnected` and the read
//  report falls through to REST, which is the deterministic path these
//  tests want to assert on.
//

import Foundation
import SwiftData
import Testing
@testable import FamilyConnect

@MainActor
@Suite("Unread rules")
struct UnreadRulesTests {

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

        func chat(_ chatID: Int64) -> ChatEntity? {
            let descriptor = FetchDescriptor<ChatEntity>(predicate: #Predicate { $0.chatID == chatID })
            return (try? context.fetch(descriptor))?.first
        }

        /// Every read report that reached the wire.
        func readPosts() -> [RecordedRequest] {
            StubURLProtocol.requests(host: host)
                .filter { $0.method == "POST" && $0.url.path().hasSuffix("/read") }
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

    private func dto(
        id: Int64,
        senderID: Int64 = 9,
        body: String = "hello",
        attachment: AttachmentDTO? = nil
    ) -> MessageDTO {
        MessageDTO(
            id: id, chatID: 42, senderID: senderID, clientMsgID: nil,
            body: body, createdAt: Self.serverDate, attachment: attachment)
    }

    /// Two unread messages waiting, which is the state every test below
    /// starts from because it is the state the user complained about.
    private func deliverTwoUnread(_ harness: Harness) {
        harness.coordinator.handle(frame: .message(dto(id: 10)))
        harness.coordinator.handle(frame: .message(dto(id: 11)))
    }

    // MARK: - Things that look like reading and are not

    @Test("a chat on screen that nobody is looking at reads nothing")
    func presenceWithoutVisibilityReadsNothing() async throws {
        let harness = try makeHarness(host: "unread-claim.test", handler: { _ in .empty(204) })
        defer { harness.tearDown() }
        deliverTwoUnread(harness)

        // Appearing: the view claims the chat before its layout has
        // settled, so it publishes "not at the newest message". This is
        // iOS pushing the conversation onto the stack, and it used to be
        // the whole test for "read".
        harness.coordinator.updatePresence(chatID: 42, isAtNewest: false, isFrontmost: true)
        #expect(harness.chat(42)?.unreadCount == 2)

        // The Mac's cold start and its focus-regain, which are the same
        // shape: the thread is at its newest message, in a window that is
        // not key. A Mac launched at login behind everything else auto-
        // selects the family chat, and clicking the Dock icon to see what
        // arrived used to destroy the count in the same gesture.
        harness.coordinator.updatePresence(chatID: 42, isAtNewest: true, isFrontmost: false)
        #expect(harness.chat(42)?.unreadCount == 2)

        // A chat somebody IS reading, in a window that is not the one
        // holding the claim: still nothing, because presence is about one
        // chat and this is not it.
        harness.coordinator.updatePresence(chatID: 99, isAtNewest: true, isFrontmost: true)
        #expect(harness.chat(42)?.unreadCount == 2)

        #expect(harness.chat(42)?.myLastReadID == 0)
        #expect(harness.readPosts().isEmpty, "nothing may have reached the server's monotonic marker")
    }

    @Test("a message arriving while the chat is open but scrolled away stays unread")
    func scrolledAwayStaysUnread() async throws {
        let harness = try makeHarness(host: "unread-scrolled.test", handler: { _ in .empty(204) })
        defer { harness.tearDown() }

        // The reader is deep in history with the app in front of them. The
        // thread deliberately does NOT scroll to a new message from here
        // (both platforms guard that on being pinned to the bottom), so
        // the message is never on screen and was never seen.
        harness.coordinator.updatePresence(chatID: 42, isAtNewest: false, isFrontmost: true)
        harness.coordinator.handle(frame: .message(dto(id: 10)))
        harness.coordinator.handle(frame: .message(dto(id: 11)))

        #expect(harness.chat(42)?.unreadCount == 2)
        #expect(harness.chat(42)?.myLastReadID == 0)
        #expect(harness.readPosts().isEmpty)

        // Scrolling back down is what reads them.
        harness.coordinator.updatePresence(chatID: 42, isAtNewest: true, isFrontmost: true)
        await harness.coordinator.pendingReadPost?.value
        #expect(harness.chat(42)?.unreadCount == 0)
        #expect(harness.chat(42)?.myLastReadID == 11)
    }

    @Test("backgrounding revokes the authority to read without dropping the claim")
    func backgroundingRevokesTheAuthorityToRead() async throws {
        let harness = try makeHarness(host: "unread-background.test", handler: { _ in .empty(204) })
        defer { harness.tearDown() }

        harness.coordinator.updatePresence(chatID: 42, isAtNewest: true, isFrontmost: true)
        harness.coordinator.enterBackground()

        // The view still owns the chat — `onDisappear` does not fire when
        // an app goes to the background, and taking the claim away here
        // would hand it to nobody — but it can no longer see anything.
        #expect(harness.coordinator.presence?.chatID == 42)
        #expect(harness.coordinator.isReading(42) == false)

        // What arrives now is genuinely unread, however it arrives.
        harness.coordinator.handle(frame: .message(dto(id: 10)))
        #expect(harness.chat(42)?.unreadCount == 1)
        #expect(harness.chat(42)?.myLastReadID == 0)
    }

    @Test("a catch-up page never marks anything read, even for the open chat")
    func catchUpNeverReads() async throws {
        let harness = try makeHarness(host: "unread-catchup.test", handler: { _ in .empty(204) })
        defer { harness.tearDown() }

        // The conversation is open and in front of the reader, at its
        // newest message — which is exactly the state a foregrounding
        // leaves behind, because the view re-establishes it from the same
        // geometry it had before.
        harness.coordinator.updatePresence(chatID: 42, isAtNewest: true, isFrontmost: true)

        // Step 3 of the resync writes the server's count…
        harness.chat(42)?.unreadCount = 5
        try harness.context.save()

        // …and step 4 walks the missed messages in through
        // `bumpUnread: false`. That used to mark each one read on its way
        // past — throwing away the count step 3 had just written and
        // pushing the server's marker to a message nobody had seen.
        for id in Int64(10)...Int64(14) {
            _ = harness.coordinator.upsert(dto(id: id), bumpUnread: false)
        }

        #expect(harness.chat(42)?.unreadCount == 5)
        #expect(harness.chat(42)?.myLastReadID == 0)
        #expect(harness.readPosts().isEmpty)
    }

    // MARK: - Reporting the read

    @Test("a failed read report leaves the marker behind, and the next attempt sends it again")
    func aFailedReadReportIsNotLost() async throws {
        let failFirst = Counter()
        let harness = try makeHarness(host: "unread-failedpost.test", handler: { request in
            guard request.method == "POST", request.url.path().hasSuffix("/read") else {
                return .empty(204)
            }
            return failFirst.increment() == 1
                ? .json(500, #"{"error": {"code": "internal", "message": "boom"}}"#)
                : .empty(204)
        })
        defer { harness.tearDown() }
        deliverTwoUnread(harness)

        harness.coordinator.updatePresence(chatID: 42, isAtNewest: true, isFrontmost: true)
        await harness.coordinator.pendingReadPost?.value

        // The badge is a local display concern and clears either way — it
        // must never be held hostage by the wire.
        #expect(harness.chat(42)?.unreadCount == 0)
        // But the server never took it, so the marker stays where it was.
        // Advancing it here was the bug: the next GET /chats re-inflated
        // the badge and the monotonic guard then refused to send the read
        // again, leaving a badge no amount of opening the chat could clear.
        #expect(harness.chat(42)?.myLastReadID == 0)
        #expect(harness.readPosts().count == 1)

        // The next time the view says the same thing, it is sent again.
        harness.coordinator.updatePresence(chatID: 42, isAtNewest: true, isFrontmost: true)
        await harness.coordinator.pendingReadPost?.value
        #expect(harness.readPosts().count == 2)
        #expect(harness.chat(42)?.myLastReadID == 11)

        // And now that the server has it, nothing re-sends it.
        harness.coordinator.updatePresence(chatID: 42, isAtNewest: true, isFrontmost: true)
        await harness.coordinator.pendingReadPost?.value
        #expect(harness.readPosts().count == 2)
    }

    // MARK: - The resync race

    @Test("the badge survives a resync that races a live message")
    func liveMessageDuringChatListRefreshIsCounted() async throws {
        // The chat-list response is held on the wire until this test lets
        // it go, rather than delayed by a clock — the interleaving IS the
        // test, and a clock would make it a coin toss on a loaded machine.
        let holdChatList = DispatchSemaphore(value: 0)
        let harness = try makeHarness(host: "unread-resyncrace.test", handler: { request in
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
                // Parked here until the live message has landed — which is
                // the whole race: the count below was computed BEFORE that
                // message existed, and assigning it flat dropped the
                // message from the badge for good, because the catch-up
                // that follows never bumps.
                holdChatList.wait()
                return .json(200, """
                {"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths",
                                     "peer_user_id": null},
                            "last_message": {"id": 10, "chat_id": 42, "sender_id": 9,
                                             "client_msg_id": null, "body": "hello",
                                             "created_at": "2026-08-19T17:05:00Z"},
                            "unread_count": 1}]}
                """)
            default:
                return .empty(204)
            }
        })
        defer { harness.tearDown() }

        // The one message the server's count knows about.
        harness.coordinator.handle(frame: .message(dto(id: 10)))
        #expect(harness.chat(42)?.unreadCount == 1)

        let resync = Task { await harness.coordinator.resync() }
        // Wait for GET /chats to be genuinely on the wire; the stub holds
        // it there, so there is no clock to overshoot.
        while !StubURLProtocol.requests(host: harness.host)
            .contains(where: { $0.url.path() == "/api/v1/chats" }) {
            try await Task.sleep(nanoseconds: 5_000_000)
        }
        harness.coordinator.handle(frame: .message(dto(id: 11)))
        #expect(harness.chat(42)?.unreadCount == 2)
        holdChatList.signal()
        await resync.value

        #expect(
            harness.chat(42)?.unreadCount == 2,
            "the message that arrived during the request must be added back to the server's count")
    }

    /// The badge and the marker must survive the response having already
    /// counted the message that arrived during it. The server COMMITS a
    /// message and then broadcasts it, and the chat-list query is served
    /// concurrently — so "arrived during the request" does not mean "the
    /// server had not counted it", and a blind delta counted it twice.
    @Test("a message the response already counted is not added to the badge again")
    func aMessageTheServerCountedIsNotCountedTwice() async throws {
        let holdChatList = DispatchSemaphore(value: 0)
        let harness = try makeHarness(host: "unread-doublecount.test", handler: { request in
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
                holdChatList.wait()
                // The other ordering, and the one the counter got wrong:
                // message 11 was committed BEFORE this query ran, so it is
                // in `unread_count` AND in `last_message` — and it still
                // reaches the socket after the response was built.
                return .json(200, """
                {"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths",
                                     "peer_user_id": null},
                            "last_message": {"id": 11, "chat_id": 42, "sender_id": 9,
                                             "client_msg_id": null, "body": "hello",
                                             "created_at": "2026-08-19T17:05:00Z"},
                            "unread_count": 2}]}
                """)
            default:
                return .empty(204)
            }
        })
        defer { harness.tearDown() }

        harness.coordinator.handle(frame: .message(dto(id: 10)))
        #expect(harness.chat(42)?.unreadCount == 1)

        let resync = Task { await harness.coordinator.resync() }
        while !StubURLProtocol.requests(host: harness.host)
            .contains(where: { $0.url.path() == "/api/v1/chats" }) {
            try await Task.sleep(nanoseconds: 5_000_000)
        }
        harness.coordinator.handle(frame: .message(dto(id: 11)))
        #expect(harness.chat(42)?.unreadCount == 2)
        holdChatList.signal()
        await resync.value

        #expect(
            harness.chat(42)?.unreadCount == 2,
            "two unread messages, however the frame and the response raced")
    }

    // MARK: - Reporting a read that becomes due mid-flight

    /// Coalescing a second read behind the first is right; DROPPING it is
    /// not, and dropping it is what the in-flight guard did. Nothing
    /// re-enters `markRead` afterwards on its own — the reader has not
    /// moved, so no geometry changes, and the message that would have
    /// re-asked is the one that was dropped.
    @Test("a read that becomes due while an earlier one is on the wire is still sent")
    func aReadThatBecomesDueMidFlightIsNotDropped() async throws {
        let harness = try makeHarness(host: "unread-coalesce.test", handler: { _ in .empty(204) })
        defer { harness.tearDown() }

        // Reading the chat, at the bottom, app in front of them.
        harness.coordinator.updatePresence(chatID: 42, isAtNewest: true, isFrontmost: true)

        // Two messages land back to back. No `await` separates them, so the
        // first report cannot possibly have settled — its continuation needs
        // this actor — which is exactly the real 200 ms case.
        harness.coordinator.handle(frame: .message(dto(id: 10)))
        harness.coordinator.handle(frame: .message(dto(id: 11)))

        await harness.coordinator.pendingReadPost?.value
        // Started from inside the first one, as it finished.
        await harness.coordinator.pendingReadPost?.value

        let posts = harness.readPosts()
        #expect(posts.count == 2, "the second read was dropped, not queued: \(posts.count) post(s)")
        #expect(
            posts.last?.bodyJSON()?["last_read_message_id"] as? Int == 11,
            "got \(String(describing: posts.last?.bodyJSON()))")
        #expect(harness.chat(42)?.myLastReadID == 11)
        #expect(harness.chat(42)?.unreadCount == 0)
    }

    // MARK: - A window that is torn down while it opens

    /// The Mac's opening sequence, and the one thing it must not do.
    ///
    /// `.task(id:)` is cancelled when the window closes or the sidebar
    /// selection changes, and a cancelled `Task.sleep` THROWS. Written
    /// `try?` that throw was swallowed and the line after it ran anyway —
    /// in a view that no longer exists, publishing presence for the chat it
    /// used to show at the optimistic `isAtNewest: true` it started with,
    /// with `onDisappear` having already released the claim so nothing
    /// refused it and nothing would ever release it again. Every later
    /// message in that chat was then marked read unseen, permanently and on
    /// every device, and on the Mac announced to nobody.
    @Test("a window torn down while it is opening claims nothing and reads nothing")
    func aCancelledOpeningPublishesNothing() async throws {
        let harness = try makeHarness(host: "unread-opening.test", handler: { _ in .empty(204) })
        defer { harness.tearDown() }
        deliverTwoUnread(harness)

        let settled = Counter()
        let opening = Task { @MainActor in
            await ChatPresenceOpening.run(
                claim: {
                    harness.coordinator.updatePresence(
                        chatID: 42, isAtNewest: false, isFrontmost: true)
                },
                loadOlder: {},
                settled: {
                    settled.increment()
                    // The view's own state at this point: `isPinnedToBottom`
                    // is still the initial optimistic `true`.
                    harness.coordinator.updatePresence(
                        chatID: 42, isAtNewest: true, isFrontmost: true)
                })
        }
        // Let the claim happen, so what follows is a teardown mid-settle
        // rather than a task that never started.
        while harness.coordinator.presence == nil {
            try await Task.sleep(nanoseconds: 1_000_000)
        }

        // The window closes: `onDisappear` releases the claim, and the task
        // sleeping through the settle delay is cancelled with the view.
        harness.coordinator.releasePresence(chatID: 42)
        opening.cancel()
        await opening.value

        // `Counter` only counts up, so "it never ran" is "the first
        // increment is the first".
        #expect(settled.increment() == 1, "the settle step ran in a window that is gone")
        #expect(harness.coordinator.presence == nil, "a released claim stays released")
        #expect(harness.chat(42)?.unreadCount == 2)
        #expect(harness.chat(42)?.myLastReadID == 0)
        #expect(harness.readPosts().isEmpty, "nothing may have reached the server's monotonic marker")

        // ...and the same sequence uncancelled still publishes, or the
        // guard above would be "never open a chat".
        let settledCount = Counter()
        await ChatPresenceOpening.run(
            settleDelay: 1_000_000,
            claim: {},
            loadOlder: {},
            settled: { settledCount.increment() })
        // Ran once, so the next increment is the second.
        #expect(settledCount.increment() == 2, "an uncancelled opening publishes exactly once")
    }

    // MARK: - What the Mac says (mirrors server/src/push_payload.rs)

    @Test("a Mac says what a phone says about the same message")
    func notificationWordingMirrorsTheServer() {
        // Title rules, straight from push_payload.rs: the family chat
        // combines family and sender with an em dash, a direct chat is just
        // the sender.
        #expect(
            ChatNotifier.title(chatKind: "family", chatTitle: "The Smiths", senderName: "Anna")
                == "The Smiths — Anna")
        #expect(
            ChatNotifier.title(chatKind: "direct", chatTitle: "Anna", senderName: "Anna") == "Anna")

        // The body is the text when there is one.
        #expect(ChatNotifier.body(text: "Dinner at 7?", attachment: nil) == "Dinner at 7?")

        // A photo is normally sent with no caption at all, so this is the
        // ordinary case and not an edge one — an alert with a name and a
        // blank line tells the reader nothing.
        #expect(ChatNotifier.body(text: "", attachment: attachment(kind: "photo")) == "Photo")
        #expect(ChatNotifier.body(text: "", attachment: attachment(kind: "video")) == "Video")
        #expect(ChatNotifier.body(text: "", attachment: attachment(kind: "audio")) == "Audio")
        // A file's name IS the thing worth saying.
        #expect(
            ChatNotifier.body(text: "", attachment: attachment(kind: "file", name: "Rechnung.pdf"))
                == "Rechnung.pdf")
        // A caption still wins over the summary.
        #expect(ChatNotifier.body(text: "at the lake", attachment: attachment(kind: "photo")) == "at the lake")
        // And nothing at all still says something.
        #expect(ChatNotifier.body(text: "", attachment: nil) == "New message")
    }

    @Test("a shared location never puts its coordinates in a notification")
    func aLocationNeverLeaksItsCoordinates() {
        let pin = attachment(
            kind: "location", name: nil, latitude: 51.500729, longitude: -0.124625)
        let anonymous = ChatNotifier.body(text: "", attachment: pin)
        #expect(anonymous == "Location")
        #expect(!anonymous.contains("51"))
        #expect(!anonymous.contains("0.12"))

        // A label is the one thing a location may say about itself.
        let labelled = attachment(
            kind: "location", name: "Home", latitude: 51.500729, longitude: -0.124625)
        #expect(ChatNotifier.body(text: "", attachment: labelled) == "Home")
    }

    @Test("clicking a locally raised notification routes through PushRoute")
    func aLocalNotificationRoutesLikeAPush() {
        let userInfo = ChatNotifier.userInfo(chatID: 42, messageID: 1338)
        // The same parser a tapped push goes through — there is no second
        // routing scheme to keep in step with this one.
        #expect(PushRoute.parse(userInfo: userInfo) == .chat(42))
        // And the marker that stops the foreground delegate from
        // suppressing it the way it suppresses a remote notification.
        #expect(userInfo[ChatNotifier.localKey] as? Bool == true)
        // Grouping matches the server's APNs thread-id, so dismissing a
        // chat's notifications catches APNs-delivered ones too.
        #expect(ChatNotifier.threadIdentifier(chatID: 42) == "chat-42")
    }

    private func attachment(
        kind: String,
        name: String? = nil,
        latitude: Double? = nil,
        longitude: Double? = nil
    ) -> AttachmentDTO {
        AttachmentDTO(
            id: 34, kind: kind, mime: "application/octet-stream", size: 4096,
            width: nil, height: nil, durationMS: nil, hasPreview: false, name: name,
            latitude: latitude, longitude: longitude, accuracyM: nil)
    }
}
