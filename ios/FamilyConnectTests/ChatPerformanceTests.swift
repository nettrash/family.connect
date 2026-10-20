//
//  ChatPerformanceTests.swift
//  FamilyConnectTests
//
//  The chat performance pass: the store's per-picture invalidation and
//  off-main decoding, the moving sticker's frame cache, a history page
//  applied in one transaction with its lookups prefetched, the decoded-JSON
//  memo, an animation tile that pauses instead of rebuilding, and a bubble
//  that can be skipped when nothing about it changed.
//
//  Every wait here waits on the EVENT (an observation firing, a decode
//  landing) under a `.timeLimit`, never on a wall-clock poll — the lesson
//  of the CI runs where a 5-second bound was spent by main-actor contention
//  before the thing it waited for could happen.
//

import AVFoundation
import Foundation
import Observation
import SwiftData
import SwiftUI
import Testing
@testable import FamilyConnect

/// Resolves when anything `read` touched changes — Observation's own
/// signal, so a test waits for the change and not for a clock.
@MainActor
private func nextChange(_ read: @escaping @MainActor () -> Void) async {
    await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
        withObservationTracking {
            read()
        } onChange: {
            continuation.resume()
        }
    }
}

/// Set from Observation's `onChange`, which is `@Sendable` — so a plain
/// captured `var` cannot be written from it.
private final class ChangeFlag: @unchecked Sendable {
    private let lock = NSLock()
    private var raised = false
    nonisolated func raise() {
        lock.lock(); defer { lock.unlock() }
        raised = true
    }
    nonisolated var value: Bool {
        lock.lock(); defer { lock.unlock() }
        return raised
    }
}

@MainActor
@Suite("Chat performance")
struct ChatPerformanceTests {

    private func makeStore(host: String) -> (AttachmentStore, URL) {
        StubURLProtocol.register(host: host) { _ in
            .json(404, #"{"error": {"code": "attachment_not_found", "message": "no"}}"#)
        }
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("perf-store-\(UUID().uuidString)", isDirectory: true)
        let store = AttachmentStore(
            api: APIClient(
                serverURL: URL(string: "https://\(host)")!,
                session: StubURLProtocol.makeSession()),
            directory: directory)
        return (store, directory)
    }

    // MARK: - Per-picture invalidation

    /// The whole point of the stamps: a picture landing redraws the views
    /// drawing THAT picture, and no others.
    @Test("a landing invalidates only the views reading that picture", .timeLimit(.minutes(1)))
    func perKeyInvalidation() async {
        let host = "perf-per-key.test"
        defer { StubURLProtocol.unregister(host: host) }
        let (store, _) = makeStore(host: host)
        defer { store.clear() }

        let first = ChangeFlag()
        let second = ChangeFlag()
        withObservationTracking {
            _ = store.image(id: 1, preview: true)
        } onChange: { first.raise() }
        withObservationTracking {
            _ = store.image(id: 2, preview: true)
        } onChange: { second.raise() }

        store.seed(TestImages.solid(width: 40, height: 30), id: 2, preview: true)
        #expect(second.value)
        #expect(!first.value, "seeding picture 2 redrew a view that only draws picture 1")
    }

    /// A file already on disk is decoded off the main actor: the first ask
    /// answers nil and starts the decode, and the view is woken when it
    /// lands — never a synchronous read inside the body that asked.
    @Test("a cached file is decoded off the main actor and the view woken", .timeLimit(.minutes(1)))
    func diskDecodeIsAsync() async throws {
        let host = "perf-disk-decode.test"
        defer { StubURLProtocol.unregister(host: host) }
        let (store, directory) = makeStore(host: host)
        defer { store.clear() }
        try TestImages.solid(width: 60, height: 40)
            .write(to: directory.appendingPathComponent("31-preview").appendingPathExtension("jpg"))

        await nextChange {
            #expect(store.image(id: 31, preview: true) == nil)
        }
        #expect(store.image(id: 31, preview: true) != nil)
        // From the disk, so no request was made for it.
        #expect(StubURLProtocol.requests(host: host).isEmpty)
    }

    @Test("decoded pictures are charged by bytes, within the budget")
    func hotCacheIsByteBounded() {
        let host = "perf-hot-bytes.test"
        defer { StubURLProtocol.unregister(host: host) }
        let (store, _) = makeStore(host: host)
        defer { store.clear() }
        store.seed(TestImages.solid(width: 100, height: 50), id: 41, preview: true)
        #expect(store.isHot(id: 41, preview: true))
        #expect(store.hotCost >= 100 * 50 * 4)
        #expect(store.hotCost <= AttachmentStore.hotBudget)
        store.purgeMemory()
        #expect(store.hotCost == 0)
        #expect(!store.isHot(id: 41, preview: true))
    }

    // MARK: - A moving sticker's frames

    @Test("a moving sticker's frames are decoded once and then served from memory", .timeLimit(.minutes(1)))
    func stickerFramesAreCached() async throws {
        let host = "perf-sticker-frames.test"
        defer { StubURLProtocol.unregister(host: host) }
        let (store, _) = makeStore(host: host)
        defer { store.clear() }
        try StickerFixtures.animatedWebP.write(to: store.originalURL(id: 51))

        await nextChange {
            #expect(store.stickerFrames(id: 51, edge: 64) == nil)
        }
        let frames = try #require(store.stickerFrames(id: 51, edge: 64))
        #expect(frames.frames.count == 2)
        // The same frames object back — a cache hit, not a second decode.
        #expect(store.stickerFrames(id: 51, edge: 64)?.frames.first === frames.frames.first)

        // Forgetting the attachment forgets its frames with its bytes.
        store.forget(attachmentIDs: [51])
        #expect(store.stickerFrames(id: 51, edge: 64) == nil)
    }

    @Test("a still sticker settles on no frames and is not decoded again", .timeLimit(.minutes(1)))
    func stillStickerSettles() async throws {
        let host = "perf-sticker-still.test"
        defer { StubURLProtocol.unregister(host: host) }
        let (store, _) = makeStore(host: host)
        defer { store.clear() }
        try StickerFixtures.stillWebP.write(to: store.originalURL(id: 52))

        await nextChange {
            _ = store.stickerFrames(id: 52, edge: 64)
        }
        // Settled: asking again changes nothing and starts nothing.
        let changedAgain = ChangeFlag()
        withObservationTracking {
            #expect(store.stickerFrames(id: 52, edge: 64) == nil)
        } onChange: { changedAgain.raise() }
        await Task.yield()
        #expect(!changedAgain.value)
    }

    // MARK: - A page in one transaction

    @MainActor
    private struct Harness {
        let container: ModelContainer
        let coordinator: ChatSyncCoordinator
        let host: String

        var context: ModelContext { container.mainContext }

        func rows() -> [MessageEntity] {
            (try? context.fetch(FetchDescriptor<MessageEntity>(
                sortBy: [SortDescriptor(\.createdAt), SortDescriptor(\.localID)]))) ?? []
        }

        func tearDown() { StubURLProtocol.unregister(host: host) }
    }

    private func makeHarness(host: String, page: String) throws -> Harness {
        StubURLProtocol.register(host: host) { request in
            request.url.path().hasSuffix("/messages") ? .json(200, page) : .empty(204)
        }
        let container = try ModelContainer(
            for: ChatEntity.self, MessageEntity.self, MemberEntity.self, PendingMediaItemEntity.self,
            configurations: ModelConfiguration(isStoredInMemoryOnly: true))
        let api = APIClient(
            serverURL: URL(string: "https://\(host)")!,
            session: StubURLProtocol.makeSession())
        let coordinator = ChatSyncCoordinator(modelContainer: container, api: api)
        coordinator.currentUserIDOverride = 7
        container.mainContext.insert(ChatEntity(chatID: 42, kind: "family", pinRank: 0, title: "The Smiths"))
        try container.mainContext.save()
        return Harness(container: container, coordinator: coordinator, host: host)
    }

    /// The prefetched page answers exactly what the per-row lookups did: an
    /// own pending send reconciles into its row, a held row is updated in
    /// place, and two copies of one message in a page make one row.
    @Test("a page applied in one go dedups exactly as row by row did", .timeLimit(.minutes(1)))
    func pageDedups() async throws {
        let page = """
            {"messages": [
              {"id": 99, "chat_id": 42, "sender_id": 9, "client_msg_id": "held", "body": "edited",
               "edit_seq": 2, "edited_at": "2026-08-22T09:05:00Z", "created_at": "2026-08-22T09:00:00Z"},
              {"id": 100, "chat_id": 42, "sender_id": 7, "client_msg_id": "mine", "body": "hello",
               "created_at": "2026-08-22T09:01:00Z"},
              {"id": 101, "chat_id": 42, "sender_id": 9, "client_msg_id": "theirs", "body": "hi",
               "created_at": "2026-08-22T09:02:00Z"},
              {"id": 101, "chat_id": 42, "sender_id": 9, "client_msg_id": "theirs", "body": "hi",
               "created_at": "2026-08-22T09:02:00Z"}
            ]}
            """
        let harness = try makeHarness(host: "perf-page-dedup.test", page: page)
        defer { harness.tearDown() }
        harness.context.insert(MessageEntity(
            localID: "s:99", serverID: 99, clientMsgID: "held", chatID: 42, senderID: 9,
            body: "original", createdAt: Date(timeIntervalSince1970: 1), status: .sent))
        harness.context.insert(MessageEntity(
            localID: "c:mine", clientMsgID: "mine", chatID: 42, senderID: 7,
            body: "hello", createdAt: Date(), status: .pending))
        try harness.context.save()

        let fetched = await harness.coordinator.loadOlder(chatID: 42)
        #expect(fetched == 4)

        let rows = harness.rows()
        #expect(rows.count == 3)
        let held = try #require(rows.first { $0.localID == "s:99" })
        #expect(held.body == "edited")
        let mine = try #require(rows.first { $0.localID == "c:mine" })
        #expect(mine.serverID == 100)
        #expect(mine.state == .sent)
        #expect(rows.filter { $0.serverID == 101 }.count == 1)
        // One transaction for the page, and it was committed.
        #expect(!harness.context.hasChanges)
    }

    // MARK: - The decoded-JSON memo

    @Test("decoded JSON is remembered by its text, and a new text decodes afresh")
    func jsonMemo() {
        let memo = DecodedJSONMemo<[ReactionSnapshot]>()
        let decoder = JSONDecoder()
        let one = #"[{"user_id":9,"emoji":"❤️"}]"#
        let first = memo.decode(one, with: decoder)
        #expect(first == [ReactionSnapshot(userID: 9, emoji: "❤️")])
        #expect(memo.decode(one, with: decoder) == first)
        let two = #"[{"user_id":9,"emoji":"👍"}]"#
        #expect(memo.decode(two, with: decoder) == [ReactionSnapshot(userID: 9, emoji: "👍")])
        // A corrupt row is not remembered as anything.
        #expect(memo.decode("not json", with: decoder) == nil)
    }

    @Test("a row's reactions follow its JSON through the memo")
    func entityReadsThroughMemo() {
        let row = MessageEntity(
            localID: "s:1", serverID: 1, chatID: 42, senderID: 9, body: "x",
            createdAt: Date(), status: .sent)
        row.reactionList = [ReactionSnapshot(userID: 9, emoji: "❤️")]
        #expect(row.reactionList == [ReactionSnapshot(userID: 9, emoji: "❤️")])
        row.reactionList = [ReactionSnapshot(userID: 7, emoji: "😂")]
        #expect(row.reactionList == [ReactionSnapshot(userID: 7, emoji: "😂")])
    }

    // MARK: - An animation tile pauses rather than rebuilds

    @Test("a tile keeps its player on screen and for the grace period after")
    func keepsPlayerRule() {
        #expect(AnimationPlayback.keepsPlayer(mayPlay: true, visible: true, withinGrace: false))
        #expect(AnimationPlayback.keepsPlayer(mayPlay: true, visible: false, withinGrace: true))
        #expect(!AnimationPlayback.keepsPlayer(mayPlay: true, visible: false, withinGrace: false))
        // Reduce Motion, or over the ceiling: never, whatever the geometry.
        #expect(!AnimationPlayback.keepsPlayer(mayPlay: false, visible: true, withinGrace: true))
    }

    @Test("showing the same video again pauses and plays the SAME player")
    func samePlayerAcrossVisibility() throws {
        let video = LoopingVideo()
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("loop-\(UUID().uuidString).mp4")
        video.show(url, playing: true)
        let player = try #require(video.player)
        video.show(url, playing: false)
        #expect(video.player === player, "a visibility flip rebuilt the player")
        #expect(player.rate == 0)
        video.show(url, playing: true)
        #expect(video.player === player)
        video.show(nil, playing: false)
        #expect(video.player == nil)
    }

    // MARK: - A bubble that can be skipped

    #if os(iOS)
    private func bubble(
        body: String = "hello",
        chips: [ReactionChip] = [],
        onCallBack: (() -> Void)? = nil,
        onRetry: @escaping () -> Void = {}
    ) -> MessageBubbleView {
        MessageBubbleView(
            message: MessageSnapshot(
                localID: "s:1", serverID: 1, chatID: 42, senderID: 9, body: body,
                createdAt: Date(timeIntervalSince1970: 0), state: .sent),
            isMine: false,
            showsSenderName: false,
            senderName: nil,
            isRead: false,
            reactionChips: chips,
            onCallBack: onCallBack,
            onRetry: onRetry)
    }

    @Test("bubbles with the same inputs are equal whatever their closures")
    func bubbleEquality() {
        #expect(bubble(onRetry: {}) == bubble(onRetry: { print("other") }))
        #expect(bubble(body: "hello") != bubble(body: "edited"))
        #expect(bubble() != bubble(chips: [ReactionChip(emoji: "❤️", count: 1, includesMe: false)]))
        // The one closure whose PRESENCE is drawn: the call record's button.
        #expect(bubble(onCallBack: nil) != bubble(onCallBack: {}))
    }
    #endif
}
