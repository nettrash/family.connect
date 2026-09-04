//
//  MediaOutboxTests.swift
//  FamilyConnectTests
//
//  Two sweeps and one hand-off, and the whole file is about keeping them
//  apart.
//
//  `MediaOutbox.sweepOrphans` collects the composer's litter in `tmp`:
//  files staged for a set nobody sent. `PendingMediaStaging` owns the bytes
//  of sends somebody DID press Send on, in Application Support, and its own
//  sweep only ever removes directories no row names. Pointing either at the
//  other's directory destroys queued messages on the very launch that was
//  supposed to resume them, so the boundary is pinned here.
//

import Foundation
import Testing
@testable import FamilyConnect

@MainActor
struct MediaOutboxTests {

    /// A prepared item backed by a real file, so ownership can be observed
    /// by whether the file still exists.
    private func makeItem(in directory: URL, previewJPEG: Data? = nil) throws -> MediaPrep.Prepared {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let url = directory.appendingPathComponent("\(MediaOutbox.filePrefix)\(UUID().uuidString).jpg")
        try Data([0xFF, 0xD8, 0xFF]).write(to: url)
        return MediaPrep.Prepared(
            fileURL: url, mime: "image/jpeg", kind: "photo",
            width: 1, height: 1, durationMS: nil, previewJPEG: previewJPEG, name: nil)
    }

    private func scratch() -> URL {
        URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("outbox-\(UUID().uuidString)")
    }

    // MARK: - The composer's litter

    /// The launch case: a process that has just started owns nothing, so
    /// everything staged and unsent is rubbish left by the run that died.
    @Test("with no floor the sweep takes even files written moments ago")
    func sweepAtLaunchTakesFreshOrphans() throws {
        let dir = scratch()
        let orphan = try makeItem(in: dir)

        let removed = MediaOutbox.sweepOrphans(in: dir)

        #expect(removed == 1)
        #expect(!FileManager.default.fileExists(atPath: orphan.fileURL.path),
                "the launch sweep spared a file from the run that just crashed")
    }

    @Test("an age floor spares files young enough to belong to this run")
    func sweepHonoursItsFloor() throws {
        let dir = scratch()
        let young = try makeItem(in: dir)
        let old = try makeItem(in: dir)
        try FileManager.default.setAttributes(
            [.creationDate: Date(timeIntervalSinceNow: -48 * 60 * 60)],
            ofItemAtPath: old.fileURL.path)

        let removed = MediaOutbox.sweepOrphans(olderThan: 24 * 60 * 60, in: dir)

        #expect(removed == 1)
        #expect(!FileManager.default.fileExists(atPath: old.fileURL.path))
        #expect(FileManager.default.fileExists(atPath: young.fileURL.path))
    }

    @Test("the sweep touches nothing that is not ours")
    func sweepLeavesForeignFilesAlone() throws {
        let dir = scratch()
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        // The share extension stages under its own prefix in the same place.
        let foreign = dir.appendingPathComponent("fc-shared-\(UUID().uuidString).jpg")
        try Data([0x00]).write(to: foreign)

        let removed = MediaOutbox.sweepOrphans(in: dir)

        #expect(removed == 0)
        #expect(FileManager.default.fileExists(atPath: foreign.path),
                "the sweep took a file belonging to somebody else")
    }

    // MARK: - Taking ownership of a send's bytes

    /// THE ONE THAT DESTROYS SOMEBODY'S OWN VIDEO. `prepareVideo` hands
    /// back the ORIGINAL url untouched when the clip already fits the
    /// ceiling, and a drag-and-drop or a share import hands `MediaPrep` a
    /// URL the person owns. Moving that out of their Movies folder is not
    /// staging, it is taking their video away.
    @Test("adopt copies a file it does not own and leaves the original alone")
    func adoptCopiesForeignFiles() throws {
        // OUTSIDE tmp: `scratch()` is a temp directory, and anything under
        // tmp is ours to move by design. A file the sender owns is not.
        let theirs = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("theirs-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: theirs, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: theirs) }
        let original = theirs.appendingPathComponent("Holiday.mov")
        try Data([0x00, 0x01]).write(to: original)
        let prepared = MediaPrep.Prepared(
            fileURL: original, mime: "video/quicktime", kind: "video",
            width: 4, height: 3, durationMS: 1000, previewJPEG: nil, name: nil)
        let itemID = UUID().uuidString.lowercased()
        defer { PendingMediaStaging.remove(itemID: itemID) }

        let adopted = try PendingMediaStaging.adopt(prepared, itemID: itemID)

        #expect(FileManager.default.fileExists(atPath: original.path),
                "staging moved a file the sender owns out from under them")
        #expect(PendingMediaStaging.url(for: adopted.fileName) != nil)
        #expect(adopted.fileName.hasSuffix("Holiday.mov"), "the real name is the file's identity")
    }

    /// Anything under `tmp` was written by `MediaPrep` for this send, so
    /// moving it is right and costs nothing.
    @Test("adopt moves a file we staged ourselves")
    func adoptMovesOurOwnFiles() throws {
        let prepared = try makeItem(in: FileManager.default.temporaryDirectory)
        let itemID = UUID().uuidString.lowercased()
        defer { PendingMediaStaging.remove(itemID: itemID) }

        let adopted = try PendingMediaStaging.adopt(prepared, itemID: itemID)

        #expect(!FileManager.default.fileExists(atPath: prepared.fileURL.path),
                "the staged copy was left behind in tmp, where the launch sweep will take it")
        #expect(PendingMediaStaging.url(for: adopted.fileName) != nil)
    }

    /// The poster used to exist only in memory, which is exactly why it
    /// could not survive a relaunch.
    @Test("adopt writes the preview out beside the file")
    func adoptPersistsThePreview() throws {
        let jpeg = Data([0xFF, 0xD8, 0xFF, 0xE0])
        let prepared = try makeItem(in: FileManager.default.temporaryDirectory, previewJPEG: jpeg)
        let itemID = UUID().uuidString.lowercased()
        defer { PendingMediaStaging.remove(itemID: itemID) }

        let adopted = try PendingMediaStaging.adopt(prepared, itemID: itemID)

        let previewName = try #require(adopted.previewFileName)
        let url = try #require(PendingMediaStaging.url(for: previewName))
        #expect(try Data(contentsOf: url) == jpeg)
    }

    @Test("removing an item takes its whole directory")
    func removeTakesEverything() throws {
        let prepared = try makeItem(in: FileManager.default.temporaryDirectory, previewJPEG: Data([0x01]))
        let itemID = UUID().uuidString.lowercased()
        let adopted = try PendingMediaStaging.adopt(prepared, itemID: itemID)
        #expect(PendingMediaStaging.url(for: adopted.fileName) != nil)

        PendingMediaStaging.remove(itemID: itemID)

        #expect(PendingMediaStaging.url(for: adopted.fileName) == nil)
    }

    /// The staging sweep is the opposite of the launch sweep: it removes
    /// only what no row names, and a queued send's bytes are named.
    @Test("the staging sweep keeps what a row still names")
    func stagingSweepKeepsLiveItems() throws {
        // Its OWN directory, never the shared root: a sweep is a bulk
        // delete, and pointing one at the real staging root would take the
        // bytes of whatever send another test is running beside this one.
        let root = scratch()
        let live = UUID().uuidString.lowercased()
        let dead = UUID().uuidString.lowercased()
        for id in [live, dead] {
            let directory = root.appendingPathComponent(id, isDirectory: true)
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            try Data([0xFF]).write(to: directory.appendingPathComponent("photo.jpg"))
        }
        defer { try? FileManager.default.removeItem(at: root) }

        // No age floor here: the directories were made a moment ago, and
        // the floor exists to protect a send being staged right now.
        let removed = PendingMediaStaging.sweepOrphans(keeping: [live], in: root, youngerThan: 0)

        #expect(removed == 1)
        #expect(FileManager.default.fileExists(
            atPath: root.appendingPathComponent("\(live)/photo.jpg").path),
                "the sweep deleted the bytes of a send that is still queued")
        #expect(!FileManager.default.fileExists(
            atPath: root.appendingPathComponent("\(dead)/photo.jpg").path))
    }
}
