//
//  MediaOutboxTests.swift
//  FamilyConnectTests
//
//  The outbox exists to enforce one rule: a prepared file has exactly one
//  owner at a time. An earlier attempt at this keyed everything by chat id
//  and broke that rule six different ways — a successful send deleted an
//  unrelated failed set's files, a retry never consumed its own entry, a
//  discarded set came back with dead file URLs. Every test here pins one of
//  those, so the rule cannot quietly rot back.
//

import Foundation
import Testing
@testable import FamilyConnect

@MainActor
struct MediaOutboxTests {

    /// A prepared item backed by a real file, so ownership can be observed
    /// by whether the file still exists.
    private func makeItem(in directory: URL) throws -> MediaPrep.Prepared {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let url = directory.appendingPathComponent("\(MediaOutbox.filePrefix)\(UUID().uuidString).jpg")
        try Data([0xFF, 0xD8, 0xFF]).write(to: url)
        return MediaPrep.Prepared(
            fileURL: url, mime: "image/jpeg", kind: "photo",
            width: 1, height: 1, durationMS: nil, previewJPEG: nil, name: nil)
    }

    private func scratch() -> URL {
        URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("outbox-\(UUID().uuidString)")
    }

    // MARK: - Ownership

    @Test("a landed send drops its entry without touching another send's files")
    func finishIsScopedToItsOwnToken() throws {
        let dir = scratch()
        let outbox = MediaOutbox()
        let mine = try makeItem(in: dir)
        let theirs = try makeItem(in: dir)

        let a = outbox.begin(chatID: 5, caption: "a", replyTo: nil, prepared: [mine])
        let b = outbox.begin(chatID: 5, caption: "b", replyTo: nil, prepared: [theirs])
        outbox.fail(b)

        // A landed on the SAME chat as the failed B.
        outbox.finish(a)

        #expect(outbox.entries.count == 1)
        #expect(outbox.mostRecentFailed(for: 5)?.id == b)
        #expect(FileManager.default.fileExists(atPath: theirs.fileURL.path),
                "a successful send deleted an unrelated failed set's files")
    }

    @Test("taking a set hands over ownership exactly once")
    func takeIsSingleShot() throws {
        let dir = scratch()
        let outbox = MediaOutbox()
        let token = outbox.begin(chatID: 7, caption: "c", replyTo: nil,
                                 prepared: [try makeItem(in: dir)])
        outbox.fail(token)

        #expect(outbox.take(token) != nil)
        #expect(outbox.take(token) == nil, "the same set was handed out twice")
        #expect(outbox.mostRecentFailed(for: 7) == nil)
    }

    /// The bug that made photos come back forever: a set still uploading is
    /// not a set waiting to be handed back.
    @Test("a set still uploading cannot be taken")
    func inFlightIsNotTakeable() throws {
        let dir = scratch()
        let outbox = MediaOutbox()
        let token = outbox.begin(chatID: 9, caption: "d", replyTo: nil,
                                 prepared: [try makeItem(in: dir)])

        #expect(outbox.take(token) == nil)
        #expect(outbox.mostRecentFailed(for: 9) == nil, "an in-flight send was offered as recoverable")
        outbox.fail(token)
        #expect(outbox.take(token) != nil)
    }

    @Test("two sends on one chat stay separate")
    func twoSendsOneChat() throws {
        let dir = scratch()
        let outbox = MediaOutbox()
        let first = outbox.begin(chatID: 3, caption: "1", replyTo: nil,
                                 prepared: [try makeItem(in: dir)])
        let second = outbox.begin(chatID: 3, caption: "2", replyTo: nil,
                                  prepared: [try makeItem(in: dir)])
        outbox.fail(first)
        outbox.fail(second)

        #expect(outbox.failedCount(for: 3) == 2)
        // Most recent first: a composer whose own send just failed must get
        // ITS set back, not an older stranded one.
        #expect(outbox.mostRecentFailed(for: 3)?.caption == "2")
        #expect(outbox.take(second)?.caption == "2")
        #expect(outbox.mostRecentFailed(for: 3)?.caption == "1")
        #expect(outbox.take(first)?.caption == "1")
    }

    // MARK: - Deletion

    @Test("discarding a set deletes its files and nothing else")
    func discardDeletesOnlyItsOwn() throws {
        let dir = scratch()
        let outbox = MediaOutbox()
        let doomed = try makeItem(in: dir)
        let keeper = try makeItem(in: dir)
        let a = outbox.begin(chatID: 1, caption: "", replyTo: nil, prepared: [doomed])
        _ = outbox.begin(chatID: 1, caption: "", replyTo: nil, prepared: [keeper])

        outbox.discard(a)

        #expect(!FileManager.default.fileExists(atPath: doomed.fileURL.path))
        #expect(FileManager.default.fileExists(atPath: keeper.fileURL.path))
    }

    /// Logout, a server change and a kick all land here. A set composed in
    /// one family must never be offered to the next.
    @Test("purge drops everything, files included")
    func purgeTakesTheFilesToo() throws {
        let dir = scratch()
        let outbox = MediaOutbox()
        let one = try makeItem(in: dir)
        let two = try makeItem(in: dir)
        _ = outbox.begin(chatID: 1, caption: "", replyTo: nil, prepared: [one])
        let failed = outbox.begin(chatID: 2, caption: "", replyTo: nil, prepared: [two])
        outbox.fail(failed)

        outbox.purgeAll()

        #expect(outbox.entries.isEmpty)
        #expect(!FileManager.default.fileExists(atPath: one.fileURL.path))
        #expect(!FileManager.default.fileExists(atPath: two.fileURL.path))
    }

    // MARK: - The sweep

    @Test("the sweep takes old orphans and spares young ones and live ones")
    func sweepIsNarrow() throws {
        let dir = scratch()
        let outbox = MediaOutbox()
        let live = try makeItem(in: dir)
        _ = outbox.begin(chatID: 1, caption: "", replyTo: nil, prepared: [live])
        let orphan = try makeItem(in: dir)
        let young = try makeItem(in: dir)

        // Age the orphan past the floor; leave the others as they are.
        try FileManager.default.setAttributes(
            [.creationDate: Date(timeIntervalSinceNow: -48 * 60 * 60)],
            ofItemAtPath: orphan.fileURL.path)

        // An explicit floor, because this case is about the floor: at
        // launch the floor is 0 and everything unowned goes.
        let removed = MediaOutbox.sweepOrphans(
            olderThan: 24 * 60 * 60, in: dir, keeping: outbox.liveFileURLs)

        #expect(removed == 1)
        #expect(!FileManager.default.fileExists(atPath: orphan.fileURL.path))
        #expect(FileManager.default.fileExists(atPath: live.fileURL.path),
                "the sweep deleted a file a live send still owns")
        #expect(FileManager.default.fileExists(atPath: young.fileURL.path),
                "the sweep deleted a file young enough to belong to this run")
    }

    /// The launch case, which the 24-hour floor used to defeat: a process
    /// that has just started owns nothing, so everything unowned is rubbish
    /// left by the run that died — including the files it died holding.
    @Test("with no floor the sweep takes even files written moments ago")
    func sweepAtLaunchTakesFreshOrphans() throws {
        let dir = scratch()
        let orphan = try makeItem(in: dir)

        let removed = MediaOutbox.sweepOrphans(in: dir)

        #expect(removed == 1)
        #expect(!FileManager.default.fileExists(atPath: orphan.fileURL.path),
                "the launch sweep spared a file from the run that just crashed")
    }

    @Test("the sweep touches nothing that is not ours")
    func sweepLeavesForeignFilesAlone() throws {
        let dir = scratch()
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        // The share extension stages under its own prefix in the same place.
        let foreign = dir.appendingPathComponent("fc-shared-\(UUID().uuidString).jpg")
        try Data([0x00]).write(to: foreign)
        try FileManager.default.setAttributes(
            [.creationDate: Date(timeIntervalSinceNow: -72 * 60 * 60)],
            ofItemAtPath: foreign.path)

        let removed = MediaOutbox.sweepOrphans(olderThan: 24 * 60 * 60, in: dir)

        #expect(removed == 0)
        #expect(FileManager.default.fileExists(atPath: foreign.path),
                "the sweep deleted a file belonging to the share extension")
    }
}
