//
//  PackSyncTests.swift
//  FamilyConnectTests
//
//  The sticker pack through the coordinator (docs/protocol.md, "Sticker
//  pack"): the apply path, the full read, the catch-up and its cursor,
//  adding, removing — and sending one, which is the ordinary media outbox
//  with one flag and NO preparation.
//
//  "The board's machinery unchanged" is the protocol's own description of
//  the sync half, so these are BoardSyncTests' cases asked of the pack: an
//  older copy must not undo a newer one, a tombstone is the last word even
//  against a copy with a HIGHER seq, a full read replaces what is held
//  except what arrived after it was taken, and the cursor moves in three
//  ways and no others.
//
//  The cursor lives on the coordinator here (`packCursorOverride`), never
//  in the UserDefaults every suite in this bundle shares.
//

import Foundation
import SwiftData
import Testing
@testable import FamilyConnect

@MainActor
@Suite("Sticker pack sync")
struct PackSyncTests {

    private static let stamp = ISO8601DateFormatter().date(from: "2026-09-13T10:00:00Z")!

    @MainActor
    private struct Harness {
        let container: ModelContainer
        let coordinator: ChatSyncCoordinator
        let context: ModelContext
        let host: String

        func items() -> [PackItemEntity] {
            let descriptor = FetchDescriptor<PackItemEntity>(sortBy: [SortDescriptor(\.itemID)])
            return (try? context.fetch(descriptor)) ?? []
        }

        func gone() -> [Int64] {
            ((try? context.fetch(FetchDescriptor<GonePackItemEntity>())) ?? [])
                .map(\.itemID).sorted()
        }

        func messages() -> [MessageEntity] {
            (try? context.fetch(FetchDescriptor<MessageEntity>())) ?? []
        }

        func pending() -> [PendingMediaItemEntity] {
            (try? context.fetch(FetchDescriptor<PendingMediaItemEntity>())) ?? []
        }

        /// Wait for the detached delivery a send starts — see
        /// `pendingDelivery` for why a container must outlive it.
        func settle() async {
            await coordinator.pendingDelivery?.value
        }

        func paths() -> [String] {
            StubURLProtocol.requests(host: host).map { "\($0.method) \($0.url.path())" }
        }

        func tearDown() { StubURLProtocol.unregister(host: host) }
    }

    private func makeHarness(
        host: String,
        handler: @escaping StubURLProtocol.Handler = { _ in .empty(204) }
    ) throws -> Harness {
        StubURLProtocol.register(host: host, handler: handler)
        let configuration = ModelConfiguration(isStoredInMemoryOnly: true)
        let container = try ModelContainer(
            for: ChatEntity.self, MessageEntity.self, MemberEntity.self, NoteEntity.self,
            PendingMediaItemEntity.self, GoneNoteEntity.self,
            PackItemEntity.self, GonePackItemEntity.self,
            configurations: configuration)
        let api = APIClient(
            serverURL: URL(string: "https://\(host)")!,
            session: StubURLProtocol.makeSession())
        let coordinator = ChatSyncCoordinator(modelContainer: container, api: api)
        coordinator.currentUserIDOverride = 7
        coordinator.packCursorOverride = 0
        coordinator.ackTimeout = 0.2
        container.mainContext.insert(
            ChatEntity(chatID: 42, kind: "family", pinRank: 0, title: "The Smiths"))
        try container.mainContext.save()
        return Harness(
            container: container, coordinator: coordinator,
            context: container.mainContext, host: host)
    }

    private func item(
        id: Int64, packSeq: Int64, attachmentID: Int64? = nil, addedBy: Int64 = 7,
        label: String? = nil
    ) -> PackItemDTO {
        PackItemDTO(
            id: id, addedBy: addedBy,
            attachment: AttachmentDTO(
                id: attachmentID ?? (70 + id), kind: "photo", mime: "image/webp", size: 38,
                width: 8, height: 8, durationMS: nil, hasPreview: false, name: nil,
                latitude: nil, longitude: nil, accuracyM: nil),
            label: label, createdAt: Self.stamp, packSeq: packSeq)
    }

    private func tombstone(id: Int64, packSeq: Int64) -> PackItemDTO {
        PackItemDTO(id: id, packSeq: packSeq, deleted: true)
    }

    nonisolated private static func itemJSON(
        id: Int64, packSeq: Int64, attachmentID: Int64
    ) -> String {
        """
        {"id": \(id), "added_by": 7, "created_at": "2026-09-13T10:00:00Z", "pack_seq": \(packSeq),
         "attachment": {"id": \(attachmentID), "kind": "photo", "mime": "image/webp", "size": 38,
                        "width": 8, "height": 8, "has_preview": false}}
        """
    }

    // MARK: - Apply

    @Test("a new item is stored with everything a panel and a send need")
    func applyInserts() throws {
        let harness = try makeHarness(host: "pack-apply.test")
        defer { harness.tearDown() }

        #expect(harness.coordinator.applyPackItem(item(id: 5, packSeq: 12, label: "party cat")))

        let row = try #require(harness.items().first)
        #expect(row.itemID == 5)
        #expect(row.addedBy == 7)
        #expect(row.attachmentID == 75)
        #expect(row.mime == "image/webp")
        #expect(row.size == 38)
        #expect(row.label == "party cat")
        #expect(row.packSeq == 12)
    }

    @Test("an item is written only when the incoming seq is GREATER than the one held")
    func olderAndEqualCopiesAreRefused() throws {
        let harness = try makeHarness(host: "pack-guard.test")
        defer { harness.tearDown() }
        harness.coordinator.applyPackItem(item(id: 5, packSeq: 12, attachmentID: 71))

        #expect(!harness.coordinator.applyPackItem(item(id: 5, packSeq: 11, attachmentID: 99)))
        #expect(!harness.coordinator.applyPackItem(item(id: 5, packSeq: 12, attachmentID: 99)))

        #expect(harness.items().first?.attachmentID == 71)
    }

    @Test("a tombstone removes the item and remembers it")
    func tombstoneRemoves() throws {
        let harness = try makeHarness(host: "pack-tombstone.test")
        defer { harness.tearDown() }
        harness.coordinator.applyPackItem(item(id: 5, packSeq: 12))

        #expect(harness.coordinator.applyPackItem(tombstone(id: 5, packSeq: 14)))

        #expect(harness.items().isEmpty)
        #expect(harness.gone() == [5])
    }

    @Test("a removed item is never brought back, even by a copy with a higher seq")
    func aGoneItemStaysGone() throws {
        let harness = try makeHarness(host: "pack-gone.test")
        defer { harness.tearDown() }
        harness.coordinator.applyPackItem(tombstone(id: 5, packSeq: 14))

        // The per-item guard alone would let this through: 20 > 14.
        #expect(!harness.coordinator.applyPackItem(item(id: 5, packSeq: 20)))

        #expect(harness.items().isEmpty)
    }

    @Test("a tombstone for an item this device never held is still remembered")
    func aTombstoneForAnUnknownItem() throws {
        let harness = try makeHarness(host: "pack-unknown.test")
        defer { harness.tearDown() }

        harness.coordinator.applyPackItem(tombstone(id: 9, packSeq: 3))
        harness.coordinator.applyPackItem(item(id: 9, packSeq: 2))

        #expect(harness.items().isEmpty)
        #expect(harness.gone() == [9])
    }

    // MARK: - The full read

    @Test("a full read replaces what is held, and an item it leaves out is gone for good")
    func fullReadReplaces() throws {
        let harness = try makeHarness(host: "pack-replace.test")
        defer { harness.tearDown() }
        harness.coordinator.applyPackItem(item(id: 1, packSeq: 3))
        harness.coordinator.applyPackItem(item(id: 2, packSeq: 4))

        let applied = harness.coordinator.replacePack(
            with: PackResponse(items: [item(id: 2, packSeq: 4), item(id: 3, packSeq: 6)], maxPackSeq: 7))

        #expect(applied)
        #expect(harness.items().map(\.itemID) == [2, 3])
        #expect(harness.gone() == [1])
    }

    @Test("an item held above the read's mark arrived after the read was taken, and stays")
    func fullReadKeepsWhatCameAfterIt() throws {
        let harness = try makeHarness(host: "pack-newer.test")
        defer { harness.tearDown() }
        harness.coordinator.applyPackItem(item(id: 1, packSeq: 3))
        // A frame that raced the read.
        harness.coordinator.applyPackItem(item(id: 8, packSeq: 9))

        harness.coordinator.replacePack(
            with: PackResponse(items: [item(id: 1, packSeq: 3)], maxPackSeq: 7))

        #expect(harness.items().map(\.itemID) == [1, 8])
        #expect(harness.gone().isEmpty)
    }

    @Test("a full read older than what is already applied is ignored whole")
    func aStaleFullReadIsIgnored() throws {
        let harness = try makeHarness(host: "pack-stale.test")
        defer { harness.tearDown() }
        harness.coordinator.applyPackItem(item(id: 1, packSeq: 3))
        harness.coordinator.applyPackItem(item(id: 2, packSeq: 9))
        harness.coordinator.packCursor = 9

        let applied = harness.coordinator.replacePack(
            with: PackResponse(items: [item(id: 1, packSeq: 3)], maxPackSeq: 5))

        #expect(!applied)
        #expect(harness.items().map(\.itemID) == [1, 2], "an older read took a sticker away")
        #expect(harness.gone().isEmpty)
    }

    // MARK: - Catch-up and the cursor

    @Test("with nothing held the catch-up is one full read, and the cursor takes its mark")
    func firstCatchUpIsAFullRead() async throws {
        let host = "pack-first.test"
        let harness = try makeHarness(host: host) { _ in
            .json(200, """
                {"items": [\(Self.itemJSON(id: 1, packSeq: 3, attachmentID: 71)),
                           \(Self.itemJSON(id: 2, packSeq: 5, attachmentID: 72))],
                 "max_pack_seq": 8}
                """)
        }
        defer { harness.tearDown() }

        await harness.coordinator.catchUpPack(serverMaxSeq: 8)

        #expect(harness.paths() == ["GET /api/v1/families/mine/pack"])
        #expect(harness.items().map(\.itemID) == [1, 2])
        // The read's own mark, not the highest seq among its items: 8 was a
        // removal, and the items that came with it prove nothing about it.
        #expect(harness.coordinator.packCursor == 8)
        #expect(harness.coordinator.packCaughtUp)
    }

    @Test("with a cursor the catch-up loops the change feed to a short page, tombstones and all")
    func catchUpLoopsTheFeed() async throws {
        let host = "pack-feed.test"
        // A page of 200 (the limit the client asks for) and then a short
        // one carrying a removal.
        // Typed step by step: as one expression this was past what Xcode 26's type checker would
        // solve "in reasonable time" on CI, though Xcode 27 here compiled it without complaint.
        let rows: [String] = (1...200).map { (number: Int) -> String in
            let id = Int64(number)
            return Self.itemJSON(id: id, packSeq: 10 + id, attachmentID: 1000 + id)
        }
        let first: String = rows.joined(separator: ",")
        let harness = try makeHarness(host: host) { request in
            let after = URLComponents(url: request.url, resolvingAgainstBaseURL: false)?
                .queryItems?.first { $0.name == "after_seq" }?.value
            if after == "10" { return .json(200, #"{"items": [\#(first)]}"#) }
            return .json(200, #"{"items": [{"id": 3, "deleted": true, "pack_seq": 215}]}"#)
        }
        defer { harness.tearDown() }
        harness.coordinator.packCursor = 10

        await harness.coordinator.catchUpPack(serverMaxSeq: 215)

        let queries = StubURLProtocol.requests(host: host).map { $0.url.query() ?? "" }
        #expect(harness.paths().allSatisfy { $0 == "GET /api/v1/families/mine/pack/changes" })
        #expect(queries == ["after_seq=10&limit=200", "after_seq=210&limit=200"])
        #expect(harness.items().count == 199)
        #expect(harness.gone() == [3])
        #expect(harness.coordinator.packCursor == 215)
        #expect(harness.coordinator.packCaughtUp)
    }

    @Test("a pack nothing has happened to costs no request at all")
    func nothingNewCostsNothing() async throws {
        let host = "pack-idle.test"
        let harness = try makeHarness(host: host)
        defer { harness.tearDown() }

        // Never written to: the server omits the field, which reads as 0.
        await harness.coordinator.catchUpPack(serverMaxSeq: 0)
        harness.coordinator.packCursor = 14
        await harness.coordinator.catchUpPack(serverMaxSeq: 14)

        #expect(StubURLProtocol.requests(host: host).isEmpty)
        #expect(harness.coordinator.packCaughtUp)
    }

    @Test("a frame moves the cursor only once this connection has caught up")
    func aFrameMovesTheCursorOnlyWhenCaughtUp() async throws {
        let host = "pack-frame.test"
        let harness = try makeHarness(host: host)
        defer { harness.tearDown() }
        harness.coordinator.packCursor = 10

        // Before the catch-up: the item lands, the cursor does not move —
        // seqs 11…19 have not been fetched, and 20 would skip them.
        harness.coordinator.handle(frame: .packItem(item(id: 5, packSeq: 20)))
        #expect(harness.items().map(\.itemID) == [5])
        #expect(harness.coordinator.packCursor == 10)

        await harness.coordinator.catchUpPack(serverMaxSeq: 10)
        harness.coordinator.handle(frame: .packItem(item(id: 6, packSeq: 21)))
        #expect(harness.coordinator.packCursor == 21)

        // And a dropped socket takes the permission away again.
        harness.coordinator.handle(event: .disconnected)
        harness.coordinator.handle(frame: .packItem(item(id: 7, packSeq: 30)))
        #expect(harness.coordinator.packCursor == 21)
        #expect(harness.items().map(\.itemID) == [5, 6, 7])
    }

    // MARK: - "Caught up" belongs to ONE connection

    /// `GET /families/mine` as a server with packs answers it.
    nonisolated private static func mineJSON(maxPackSeq: Int64) -> String {
        """
        {"family": {"id": 1, "name": "The Smiths", "join_policy": "open"},
         "members": [], "blocked_user_ids": [],
         "max_pack_seq": \(maxPackSeq), "max_pack_items": 200, "max_pack_item_bytes": 524288}
        """
    }

    @Test("a catch-up a reconnect has overtaken applies what it read and earns no flag")
    func anOvertakenCatchUpDoesNotMarkTheNewConnection() async throws {
        let host = "pack-overtaken.test"
        // The feed's answer is held in the air until the test lets it go,
        // so the socket can drop while the pass is genuinely in flight. No
        // clock anywhere: the handler says when the request has arrived.
        let gate = DispatchSemaphore(value: 0)
        let (arrived, arrival) = AsyncStream<Void>.makeStream()
        let harness = try makeHarness(host: host) { _ in
            arrival.yield()
            gate.wait()
            return .json(200, #"{"items": [\#(Self.itemJSON(id: 4, packSeq: 11, attachmentID: 74))]}"#)
        }
        defer { harness.tearDown() }
        let coordinator = harness.coordinator
        coordinator.packCursor = 10

        // The pass starts under the connection there is now …
        let pass = Task { await coordinator.catchUpPack(serverMaxSeq: 11) }
        for await _ in arrived { break }
        // … and the socket drops before the feed answers. Whatever the pack
        // did from here on, this pass did not ask about.
        coordinator.handle(event: .disconnected)
        gate.signal()
        await pass.value

        // What it read is applied, and the page moved the cursor — those
        // are facts about the pack, whichever socket they were read under.
        #expect(harness.items().map(\.itemID) == [4])
        #expect(coordinator.packCursor == 11)
        // But "caught up" is a fact about a CONNECTION, and this was not
        // that connection's pass. Setting the flag here is the bug: the
        // disconnect had cleared it, and a pass finishing afterwards put it
        // back.
        #expect(!coordinator.packCaughtUp, "an overtaken pass marked the new connection caught up")

        // So the new connection's frames move no cursor: 12…29 are unread.
        coordinator.handle(frame: .packItem(item(id: 9, packSeq: 30)))
        #expect(coordinator.packCursor == 11, "a frame skipped what the gap held")
        #expect(harness.items().map(\.itemID) == [4, 9])

        // Its OWN pass earns the flag, and then a frame moves the cursor.
        await coordinator.catchUpPack(serverMaxSeq: 11)
        #expect(coordinator.packCaughtUp)
        coordinator.handle(frame: .packItem(item(id: 10, packSeq: 31)))
        #expect(coordinator.packCursor == 31)
    }

    @Test("a mark read under the old socket earns nothing for the new one, even with nothing to fetch")
    func aStaleMarkIsNotCaughtUp() async throws {
        let host = "pack-stale-mark.test"
        let harness = try makeHarness(host: host)
        defer { harness.tearDown() }
        let coordinator = harness.coordinator
        coordinator.packCursor = 10

        // `GET /families/mine` said 10 under this connection …
        let readUnder = coordinator.packConnection
        // … the socket dropped, and the pack moved on to 11 in the gap.
        coordinator.handle(event: .disconnected)

        // The pass that was about to run has nothing to fetch — 10 is not
        // above 10 — and that used to be "caught up", on a number that was
        // already out of date.
        await coordinator.catchUpPack(serverMaxSeq: 10, connection: readUnder)

        #expect(StubURLProtocol.requests(host: host).isEmpty)
        #expect(!coordinator.packCaughtUp)
        coordinator.handle(frame: .packItem(item(id: 9, packSeq: 12)))
        #expect(coordinator.packCursor == 10, "seq 11 would never have been fetched")
    }

    @Test("an overtaken pass is run again for the connection that overtook it, and THAT one is caught up")
    func theOvertakingConnectionGetsItsOwnPass() async throws {
        let host = "pack-overtaken-rerun.test"
        let gate = DispatchSemaphore(value: 0)
        let (arrived, arrival) = AsyncStream<Void>.makeStream()
        let harness = try makeHarness(host: host) { request in
            switch request.url.path() {
            case "/api/v1/families/mine":
                // Read again under the new socket: the pack has moved on.
                return .json(200, Self.mineJSON(maxPackSeq: 12))
            case "/api/v1/families/mine/pack/changes":
                let after = URLComponents(url: request.url, resolvingAgainstBaseURL: false)?
                    .queryItems?.first { $0.name == "after_seq" }?.value
                if after == "10" {
                    arrival.yield()
                    gate.wait()
                    return .json(200, #"{"items": [\#(Self.itemJSON(id: 4, packSeq: 11, attachmentID: 74))]}"#)
                }
                return .json(200, #"{"items": [\#(Self.itemJSON(id: 5, packSeq: 12, attachmentID: 75))]}"#)
            default:
                // The resync the `.connected` below starts: refused at its
                // first read, so it is this test's pass and no other.
                return .json(401, #"{"error": {"code": "unauthorized", "message": "no"}}"#)
            }
        }
        defer { harness.tearDown() }
        let coordinator = harness.coordinator
        coordinator.packCursor = 10
        let mine = try APICoding.decoder().decode(
            FamilyMineResponse.self, from: Data(Self.mineJSON(maxPackSeq: 11).utf8))

        let readUnder = coordinator.packConnection
        let pass = Task {
            await coordinator.catchUpPackForThisConnection(connection: readUnder, mine: mine)
        }
        for await _ in arrived { break }
        // The socket drops AND comes back while the feed is in the air. The
        // resync the new connection asks for is the one a running resync
        // swallows, which is why the pack runs its own pass again.
        coordinator.handle(event: .disconnected)
        coordinator.handle(event: .connected)
        let overtaking = coordinator.packConnection
        #expect(overtaking != readUnder)
        gate.signal()
        await pass.value
        await coordinator.pendingConnectResync?.value

        let feeds = StubURLProtocol.requests(host: host)
            .filter { $0.url.path() == "/api/v1/families/mine/pack/changes" }
            .map { $0.url.query() ?? "" }
        #expect(feeds == ["after_seq=10&limit=200", "after_seq=11&limit=200"])
        #expect(harness.items().map(\.itemID) == [4, 5])
        #expect(coordinator.packCursor == 12)
        #expect(coordinator.packConnection == overtaking)
        #expect(coordinator.packCaughtUp, "the connection that overtook the pass never caught up")
        coordinator.handle(frame: .packItem(item(id: 6, packSeq: 13)))
        #expect(coordinator.packCursor == 13)
    }

    @Test("a live item with no picture, or nobody who added it, is dropped and not drawn")
    func aLiveItemMissingItsFieldsIsDropped() async throws {
        let host = "pack-incomplete.test"
        // One page of the feed: a whole item, one with no `attachment`, one
        // with no `added_by`, and a tombstone — which is SUPPOSED to have
        // neither.
        let harness = try makeHarness(host: host) { _ in
            .json(200, """
                {"items": [
                  \(Self.itemJSON(id: 1, packSeq: 11, attachmentID: 71)),
                  {"id": 2, "added_by": 7, "created_at": "2026-09-13T10:00:00Z", "pack_seq": 12},
                  {"id": 3, "created_at": "2026-09-13T10:00:00Z", "pack_seq": 13,
                   "attachment": {"id": 73, "kind": "photo", "mime": "image/webp", "size": 38,
                                  "has_preview": false}},
                  {"id": 8, "deleted": true, "pack_seq": 14}]}
                """)
        }
        defer { harness.tearDown() }
        harness.coordinator.applyPackItem(item(id: 8, packSeq: 5))
        harness.coordinator.packCursor = 10

        await harness.coordinator.catchUpPack(serverMaxSeq: 14)

        // Not a failed page: the whole item and the tombstone are applied,
        // the two broken ones are simply not there.
        #expect(harness.items().map(\.itemID) == [1])
        #expect(harness.gone() == [8])
        #expect(harness.coordinator.packCursor == 14)

        // And the same by a frame.
        #expect(!harness.coordinator.applyPackItem(
            PackItemDTO(id: 20, addedBy: nil, attachment: nil, label: nil,
                        createdAt: Self.stamp, packSeq: 40)))
        harness.coordinator.handle(frame: .packItem(
            PackItemDTO(id: 21, addedBy: 7, attachment: nil, label: nil,
                        createdAt: Self.stamp, packSeq: 41)))
        #expect(harness.items().map(\.itemID) == [1])
    }

    // MARK: - Letting go of the pack

    @Test("a purge takes the cursor with the items, so the next catch-up reads the whole pack")
    func aPurgedPackIsReadWholeAgain() async throws {
        let host = "pack-purged.test"
        // The family this member comes back to — or another one: pack seqs
        // are server-wide, so its mark may sit BELOW the cursor this device
        // had reached in the family it left.
        let harness = try makeHarness(host: host) { _ in
            .json(200, """
                {"items": [\(Self.itemJSON(id: 1, packSeq: 3, attachmentID: 71)),
                           \(Self.itemJSON(id: 2, packSeq: 450, attachmentID: 72))],
                 "max_pack_seq": 450}
                """)
        }
        defer { harness.tearDown() }
        harness.coordinator.applyPackItem(item(id: 1, packSeq: 3))
        harness.coordinator.applyPackItem(tombstone(id: 9, packSeq: 400))
        harness.coordinator.packCursor = 500
        await harness.coordinator.catchUpPack(serverMaxSeq: 500)
        #expect(harness.coordinator.packCaughtUp)

        // What a kick or a leave does: the chat store goes, the session —
        // and every default, the cursor's home among them — stays.
        harness.coordinator.forgetPack()

        #expect(harness.items().isEmpty)
        #expect(harness.gone().isEmpty)
        #expect(harness.coordinator.packCursor == 0, "a cursor outlived the pack it counted")
        #expect(!harness.coordinator.packCaughtUp)

        await harness.coordinator.catchUpPack(serverMaxSeq: 450)

        // A full read, and one that is APPLIED: with the cursor left at 500
        // this made no request at all, and a read marked 450 would have
        // been refused as older than what was held.
        #expect(harness.paths() == ["GET /api/v1/families/mine/pack"])
        #expect(harness.items().map(\.itemID) == [1, 2], "the panel stayed empty after rejoining")
        #expect(harness.coordinator.packCursor == 450)
        #expect(harness.coordinator.packCaughtUp)
    }

    @Test("a frame that arrives between a purge and the next catch-up moves no cursor")
    func aFrameAfterAPurgeMovesNoCursor() async throws {
        let harness = try makeHarness(host: "pack-purged-frame.test")
        defer { harness.tearDown() }
        harness.coordinator.packCursor = 10
        await harness.coordinator.catchUpPack(serverMaxSeq: 10)

        harness.coordinator.forgetPack()
        // Were the flag still up, this would put the cursor at 20 over a
        // pack of one item — and the full read would never be asked for.
        harness.coordinator.handle(frame: .packItem(item(id: 5, packSeq: 20)))

        #expect(harness.coordinator.packCursor == 0)
    }

    // MARK: - Adding

    @Test("adding uploads the ORIGINAL bytes as a photo, claims them, and uploads no preview")
    func addUploadsUnpreparedAndClaims() async throws {
        let host = "pack-add.test"
        let harness = try makeHarness(host: host) { request in
            switch (request.method, request.url.path()) {
            case ("POST", "/api/v1/attachments"):
                return .json(201, """
                    {"attachment": {"id": 71, "kind": "photo", "mime": "image/webp", "size": 140,
                                    "width": 8, "height": 8, "has_preview": false}}
                    """)
            case ("POST", "/api/v1/families/mine/pack"):
                return .json(201, #"{"item": \#(Self.itemJSON(id: 5, packSeq: 12, attachmentID: 71))}"#)
            default:
                return .json(404, #"{"error": {"code": "not_found", "message": "no"}}"#)
            }
        }
        defer { harness.tearDown() }
        let bytes = StickerFixtures.animatedWebP
        let made = try StickerPack.make(from: bytes, maxBytes: 512 * 1024)

        let outcome = await harness.coordinator.addSticker(made, label: "  party cat ")

        #expect(outcome == .added)
        #expect(harness.paths() == ["POST /api/v1/attachments", "POST /api/v1/families/mine/pack"],
                "a sticker uploads no preview — a preview is a JPEG")
        let requests = StubURLProtocol.requests(host: host)
        let upload = requests[0]
        #expect(upload.headers["Content-Type"] == "image/webp")
        let query = URLComponents(url: upload.url, resolvingAgainstBaseURL: false)?.queryItems ?? []
        #expect(query.first { $0.name == "kind" }?.value == "photo")
        #expect(query.first { $0.name == "width" }?.value == "8")
        // Byte for byte, animation included: not a JPEG, not a first frame.
        let body = try #require(upload.body)
        #expect(body == bytes, "the sticker was re-encoded on its way up")
        #expect(requests[1].bodyJSON()?["attachment_id"] as? Int == 71)
        #expect(requests[1].bodyJSON()?["label"] as? String == "party cat")

        #expect(harness.items().map(\.itemID) == [5])
        // The answer to this client's own add moves no cursor.
        #expect(harness.coordinator.packCursor == 0)
    }

    @Test("a 200 is the pack already holding it: no second sticker, and the pack's own attachment id")
    func addingTwiceIsNotTwoStickers() async throws {
        let host = "pack-add-held.test"
        let harness = try makeHarness(host: host) { request in
            switch (request.method, request.url.path()) {
            case ("POST", "/api/v1/attachments"):
                return .json(201, """
                    {"attachment": {"id": 88, "kind": "photo", "mime": "image/webp", "size": 38,
                                    "has_preview": false}}
                    """)
            default:
                return .json(200, #"{"item": \#(Self.itemJSON(id: 5, packSeq: 12, attachmentID: 71))}"#)
            }
        }
        defer { harness.tearDown() }
        harness.coordinator.applyPackItem(item(id: 5, packSeq: 12, attachmentID: 71))
        let made = try StickerPack.make(from: StickerFixtures.stillWebP, maxBytes: 512 * 1024)

        let outcome = await harness.coordinator.addSticker(made, label: nil)

        #expect(outcome == .alreadyHeld)
        #expect(harness.items().count == 1)
        #expect(harness.items().first?.attachmentID == 71, "the dropped upload's id was kept")
    }

    @Test("a full pack and an oversized sticker are said, never swallowed")
    func addFailuresAreSaid() async throws {
        let host = "pack-add-full.test"
        let harness = try makeHarness(host: host) { request in
            switch (request.method, request.url.path()) {
            case ("POST", "/api/v1/attachments"):
                return .json(201, """
                    {"attachment": {"id": 88, "kind": "photo", "mime": "image/webp", "size": 38,
                                    "has_preview": false}}
                    """)
            default:
                return .json(409, #"{"error": {"code": "pack_full", "message": "full"}}"#)
            }
        }
        defer { harness.tearDown() }
        let made = try StickerPack.make(from: StickerFixtures.stillWebP, maxBytes: 512 * 1024)

        let outcome = await harness.coordinator.addSticker(made, label: nil)

        #expect(outcome == .failed(
            String(localized: "The family's stickers are full. Remove one to make room.")))
        #expect(harness.items().isEmpty)
        // The server's 413 is `pack_item_too_large`, and has its own words.
        #expect(ChatSyncCoordinator.packFailure(APIError.payloadTooLarge)
            == String(localized: "That picture is too big to be a sticker."))
    }

    @Test("an upload the sweep took between the two requests is simply uploaded again")
    func anExpiredUploadIsRetriedOnce() async throws {
        let host = "pack-add-expired.test"
        let claims = PackClaimCounter()
        let harness = try makeHarness(host: host) { request in
            switch (request.method, request.url.path()) {
            case ("POST", "/api/v1/attachments"):
                return .json(201, """
                    {"attachment": {"id": 71, "kind": "photo", "mime": "image/webp", "size": 38,
                                    "has_preview": false}}
                    """)
            default:
                if claims.next() == 1 {
                    return .json(404, #"{"error": {"code": "attachment_expired", "message": "gone"}}"#)
                }
                return .json(201, #"{"item": \#(Self.itemJSON(id: 5, packSeq: 12, attachmentID: 71))}"#)
            }
        }
        defer { harness.tearDown() }
        let made = try StickerPack.make(from: StickerFixtures.stillWebP, maxBytes: 512 * 1024)

        #expect(await harness.coordinator.addSticker(made, label: nil) == .added)
        #expect(harness.paths().filter { $0 == "POST /api/v1/attachments" }.count == 2)
    }

    @Test("a SECOND expired upload is no longer a race: it is shown, and not retried for ever")
    func aSecondExpiredUploadIsShown() async throws {
        let host = "pack-add-expired-twice.test"
        let harness = try makeHarness(host: host) { request in
            switch (request.method, request.url.path()) {
            case ("POST", "/api/v1/attachments"):
                return .json(201, """
                    {"attachment": {"id": 71, "kind": "photo", "mime": "image/webp", "size": 38,
                                    "has_preview": false}}
                    """)
            default:
                return .json(404, #"{"error": {"code": "attachment_expired", "message": "gone"}}"#)
            }
        }
        defer { harness.tearDown() }
        let made = try StickerPack.make(from: StickerFixtures.stillWebP, maxBytes: 512 * 1024)

        let outcome = await harness.coordinator.addSticker(made, label: nil)

        guard case .failed(let sentence) = outcome else {
            Issue.record("a twice-expired upload was reported as \(outcome)")
            return
        }
        #expect(!sentence.isEmpty)
        // Uploaded again ONCE — two uploads, two claims — and then it stops.
        #expect(harness.paths() == [
            "POST /api/v1/attachments", "POST /api/v1/families/mine/pack",
            "POST /api/v1/attachments", "POST /api/v1/families/mine/pack",
        ])
        #expect(harness.items().isEmpty)
    }

    @Test("a label over 64 is refused in words before anything is uploaded")
    func anOverLongLabelCostsNoRequest() async throws {
        let host = "pack-add-label.test"
        let harness = try makeHarness(host: host)
        defer { harness.tearDown() }
        let made = try StickerPack.make(from: StickerFixtures.stillWebP, maxBytes: 512 * 1024)

        let outcome = await harness.coordinator.addSticker(
            made, label: String(repeating: "a", count: 65))

        #expect(outcome == .failed(StickerPack.labelTooLongNotice))
        #expect(StubURLProtocol.requests(host: host).isEmpty, "the picture went up for nothing")
    }

    @Test("the label goes up trimmed, and spaces alone are no label")
    func theLabelIsSentTrimmed() async throws {
        let host = "pack-add-label-trim.test"
        let harness = try makeHarness(host: host) { request in
            switch (request.method, request.url.path()) {
            case ("POST", "/api/v1/attachments"):
                return .json(201, """
                    {"attachment": {"id": 71, "kind": "photo", "mime": "image/webp", "size": 38,
                                    "has_preview": false}}
                    """)
            default:
                return .json(201, #"{"item": \#(Self.itemJSON(id: 5, packSeq: 12, attachmentID: 71))}"#)
            }
        }
        defer { harness.tearDown() }
        let made = try StickerPack.make(from: StickerFixtures.stillWebP, maxBytes: 512 * 1024)

        // 64 after trimming is within the limit, whatever surrounds it.
        let sixtyFour = String(repeating: "я", count: 64)
        #expect(await harness.coordinator.addSticker(made, label: "  \(sixtyFour)\n") == .added)
        #expect(await harness.coordinator.addSticker(made, label: "   ") == .added)

        let claims = StubURLProtocol.requests(host: host)
            .filter { $0.url.path() == "/api/v1/families/mine/pack" }
            .compactMap { $0.bodyJSON() }
        #expect(claims.count == 2)
        #expect(claims.first?["label"] as? String == sixtyFour)
        #expect(claims.last?["label"] == nil)
    }

    // MARK: - Removing

    @Test("removing deletes the row, remembers the id, and asks the documented path")
    func removeDeletesAndRemembers() async throws {
        let host = "pack-remove.test"
        let harness = try makeHarness(host: host)
        defer { harness.tearDown() }
        harness.coordinator.applyPackItem(item(id: 5, packSeq: 12))

        #expect(await harness.coordinator.removeSticker(id: 5) == nil)

        #expect(harness.paths() == ["DELETE /api/v1/families/mine/pack/5"])
        #expect(harness.items().isEmpty)
        #expect(harness.gone() == [5])
        // The tombstone frame that follows finds nothing to do, and a late
        // copy of the item cannot put it back.
        #expect(!harness.coordinator.applyPackItem(item(id: 5, packSeq: 12)))
    }

    @Test("pack_item_not_found on a removal is already gone: dropped here, and nothing is said")
    func removingWhatIsAlreadyGoneIsNotAnError() async throws {
        let host = "pack-remove-404.test"
        let harness = try makeHarness(host: host) { _ in
            .json(404, #"{"error": {"code": "pack_item_not_found", "message": "no such item"}}"#)
        }
        defer { harness.tearDown() }
        harness.coordinator.applyPackItem(item(id: 5, packSeq: 12))

        #expect(await harness.coordinator.removeSticker(id: 5) == nil, "an error was shown")

        #expect(harness.items().isEmpty, "a sticker the family no longer has stayed in the panel")
        #expect(harness.gone() == [5])
        #expect(!harness.coordinator.applyPackItem(item(id: 5, packSeq: 12)))
    }

    @Test("any OTHER 404 on a removal is a failure: said, and the sticker stays")
    func aBare404IsNotAlreadyGone() async throws {
        let host = "pack-remove-404-other.test"
        // What a server with no such route answers — or a proxy in front of
        // one. Nothing about it says the item is gone.
        let harness = try makeHarness(host: host) { _ in .empty(404) }
        defer { harness.tearDown() }
        harness.coordinator.applyPackItem(item(id: 5, packSeq: 12))

        #expect(await harness.coordinator.removeSticker(id: 5) != nil)

        #expect(harness.items().map(\.itemID) == [5])
        #expect(harness.gone().isEmpty)
    }

    @Test("somebody who neither added it nor owns the family is told why, and the sticker stays")
    func removeRefusedKeepsTheSticker() async throws {
        let host = "pack-remove-403.test"
        let harness = try makeHarness(host: host) { _ in
            .json(403, #"{"error": {"code": "not_pack_item_author", "message": "no"}}"#)
        }
        defer { harness.tearDown() }
        harness.coordinator.applyPackItem(item(id: 5, packSeq: 12, addedBy: 9))

        let answer = await harness.coordinator.removeSticker(id: 5)

        #expect(answer == String(
            localized: "Only whoever added a sticker, or the family owner, can remove it."))
        #expect(harness.items().map(\.itemID) == [5])
        #expect(harness.gone().isEmpty)
    }

    // MARK: - Sending one

    nonisolated private static func stickerMessageJSON(clientID: String) -> String {
        """
        {"message": {"id": 900, "chat_id": 42, "sender_id": 7, "body": "",
         "created_at": "2026-09-13T10:00:00Z", "client_msg_id": "\(clientID)",
         "attachments": [{"id": 90, "kind": "photo", "mime": "image/webp", "size": 140,
                          "width": 8, "height": 8, "has_preview": false, "sticker": true}]}}
        """
    }

    @Test("a sticker goes through the outbox untouched: original bytes, no preview, sticker: true")
    func sendingASticker() async throws {
        let host = "pack-send.test"
        let harness = try makeHarness(host: host) { request in
            switch (request.method, request.url.path()) {
            case ("POST", "/api/v1/attachments"):
                return .json(201, """
                    {"attachment": {"id": 90, "kind": "photo", "mime": "image/webp", "size": 140,
                                    "width": 8, "height": 8, "has_preview": false}}
                    """)
            case ("POST", "/api/v1/chats/42/messages"):
                let clientID = request.bodyJSON()?["client_msg_id"] as? String ?? ""
                return .json(201, Self.stickerMessageJSON(clientID: clientID))
            default:
                return .json(404, #"{"error": {"code": "not_found", "message": "no"}}"#)
            }
        }
        defer { harness.tearDown() }
        let bytes = StickerFixtures.animatedWebP

        let localID = try #require(harness.coordinator.sendSticker(
            bytes: bytes, mime: "image/webp", width: 8, height: 8, in: 42))
        await harness.settle()

        #expect(harness.paths() == ["POST /api/v1/attachments", "POST /api/v1/chats/42/messages"],
                "a sticker message uploads no preview")
        let requests = StubURLProtocol.requests(host: host)
        #expect(requests[0].headers["Content-Type"] == "image/webp")
        let body = try #require(requests[0].body)
        #expect(body == bytes, "the sticker was re-encoded on its way up")
        let send = try #require(requests[1].bodyJSON())
        #expect(send["sticker"] as? Bool == true)
        #expect(send["body"] as? String == "")
        #expect(send["attachment_ids"] as? [Int] == [90])

        let row = try #require(harness.coordinator.fetchMessage(localID: localID))
        #expect(row.state == .sent)
        #expect(row.attachmentList.count == 1)
        #expect(row.attachmentList.first?.sticker == true)
        #expect(MessagePresentation.isSticker(MessageSnapshot(row)))
    }

    @Test("with no network the sticker waits in the outbox, still a sticker, its bytes intact")
    func aStickerSentOfflineIsQueued() async throws {
        let host = "pack-send-offline.test"
        let harness = try makeHarness(host: host) { _ in .failure(URLError(.notConnectedToInternet)) }
        defer { harness.tearDown() }
        let bytes = StickerFixtures.animatedWebP

        let localID = try #require(harness.coordinator.sendSticker(
            bytes: bytes, mime: "image/webp", width: 8, height: 8, in: 42))
        await harness.settle()
        defer { harness.coordinator.deleteLocalMessage(localID: localID) }

        // Pending, not failed: a transport failure says nothing about the
        // message, and the sweep will try again on the next connect.
        let row = try #require(harness.coordinator.fetchMessage(localID: localID))
        #expect(row.state == .pending)
        #expect(row.pendingAttachmentCount == 1)
        // The bubble already draws as a sticker, from a provisional id.
        #expect(row.attachmentList.first?.sticker == true)
        #expect((row.attachmentList.first?.id ?? 0) < 0)

        let staged = try #require(harness.pending().first)
        #expect(staged.sticker, "a send resumed after a relaunch would go as a photograph")
        #expect(staged.previewFileName == nil)
        let url = try #require(staged.fileName.flatMap(PendingMediaStaging.url(for:)))
        #expect(try Data(contentsOf: url) == bytes, "the queued bytes are not the sticker's own")
        // And the chat list says what is waiting.
        let chat = try #require(try harness.context.fetch(FetchDescriptor<ChatEntity>()).first)
        #expect(chat.lastMessagePreview == String(localized: "Sticker"))
    }

    @Test("a sticker may be a reply, and the quote rides on the send")
    func aStickerReply() async throws {
        let host = "pack-send-reply.test"
        let harness = try makeHarness(host: host) { request in
            switch (request.method, request.url.path()) {
            case ("POST", "/api/v1/attachments"):
                return .json(201, """
                    {"attachment": {"id": 90, "kind": "photo", "mime": "image/png", "size": 70,
                                    "has_preview": false}}
                    """)
            default:
                let clientID = request.bodyJSON()?["client_msg_id"] as? String ?? ""
                return .json(201, Self.stickerMessageJSON(clientID: clientID))
            }
        }
        defer { harness.tearDown() }
        let png = StickerFixtures.png(width: 8, height: 8)

        _ = try #require(harness.coordinator.sendSticker(
            bytes: png, mime: "image/png", width: 8, height: 8,
            replyTo: ReplyToDTO(messageID: 600, senderID: 9, excerpt: "See you at six"), in: 42))
        await harness.settle()

        let requests = StubURLProtocol.requests(host: host)
        #expect(requests.first?.headers["Content-Type"] == "image/png")
        let send = try #require(requests.last?.bodyJSON())
        #expect(send["reply_to_message_id"] as? Int == 600)
        #expect(send["sticker"] as? Bool == true)
    }

    @Test("bytes that are neither WebP nor PNG are not sent as a sticker at all")
    func onlyStickerTypesAreSent() throws {
        let harness = try makeHarness(host: "pack-send-type.test")
        defer { harness.tearDown() }

        #expect(harness.coordinator.sendSticker(
            bytes: Data([0xFF, 0xD8, 0xFF]), mime: "image/jpeg", width: 1, height: 1, in: 42) == nil)
        #expect(harness.messages().isEmpty)
    }
}

/// A counter a `@Sendable` stub handler can bump.
private final class PackClaimCounter: @unchecked Sendable {
    private let lock = NSLock()
    private var value = 0
    func next() -> Int {
        lock.lock(); defer { lock.unlock() }
        value += 1
        return value
    }
}
