//
//  ShareHandoffTests.swift
//  FamilyConnectTests
//
//  The share hand-off end to end: what the Share Extension WRITES, read
//  back by the app that consumes it (issue #33).
//
//  Everything else about sharing is tested from the app's side only —
//  ShareImportTests hands `handleShareURL` a URL somebody typed into a
//  test. That cannot catch the failure this pair actually has. The
//  extension and the app are two processes that agree on a scheme, a host,
//  an App Group, a folder name and a ten-item cap, and every one of those
//  used to be declared twice (issue #34). A disagreement is SILENT: the
//  appex copies the files, reports success, dismisses itself, and the app
//  opens on a chat list having found nothing. Nobody sees an error.
//
//  So these tests drive the producer's own code — ShareHandoff, which the
//  appex now compiles rather than reimplementing — into the app's real
//  consumer, and assert the CONTRACT: the files come out the other side
//  with their names, their bytes and their order. Plus the two ends of the
//  contract that no amount of hoisting can unify, because they live in
//  property lists: the URL scheme the app registers, and the activation
//  rule's item count.
//

import Foundation
import Testing
@testable import FamilyConnect

@Suite("Share hand-off, extension to app")
@MainActor
struct ShareHandoffTests {

    // MARK: - Standing in for the App Group container

    /// A directory playing the part of the group container.
    ///
    /// The real one is out of reach here on purpose: a test bundle built
    /// without code signing has no App Group entitlement, so
    /// `ShareHandoff.containerURL()` is nil — which is exactly why
    /// `handleShareURL` takes the container as a parameter. Nothing in
    /// this suite may assert that the real container exists.
    private func makeContainer() throws -> URL {
        let container = FileManager.default.temporaryDirectory
            .appendingPathComponent("fc-test-group-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: container, withIntermediateDirectories: true)
        return container
    }

    private func makeSession() -> AppSession {
        AppSession(api: APIClient(serverURL: nil), defaultServerURL: { nil })
    }

    // MARK: - The producer

    /// The Share Extension's staging step, with only the UIKit removed.
    ///
    /// Every line that touches the contract is the appex's own line:
    /// `stagingDirectory` builds the folder, `safeFileName` cleans the
    /// name, `handoffURL` mints the URL. What is left out is
    /// `NSItemProvider` and the responder-chain walk to `open(_:)` — the
    /// parts that need a share sheet and cannot be wrong quietly.
    private func extensionStages(
        _ files: [(name: String, bytes: String)],
        into container: URL
    ) throws -> URL {
        try FileManager.default.createDirectory(
            at: ShareHandoff.inboxURL(container: container), withIntermediateDirectories: true)
        var ids: [String] = []
        for file in files {
            let id = UUID().uuidString
            let directory = try #require(
                ShareHandoff.stagingDirectory(container: container, id: id))
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            try Data(file.bytes.utf8).write(
                to: directory.appendingPathComponent(ShareHandoff.safeFileName(file.name)))
            ids.append(id)
        }
        return try #require(ShareHandoff.handoffURL(ids: ids))
    }

    // MARK: - Producer → consumer

    /// The whole point: three files staged by the extension's code come
    /// out of the app's importer as three files, same names, same bytes,
    /// same order. Order is contract, not accident — it is the order the
    /// message will carry the attachments in.
    @Test("What the extension stages is what the app imports, in order")
    func roundTrip() throws {
        let session = makeSession()
        let container = try makeContainer()
        defer {
            session.discardShareImports()
            try? FileManager.default.removeItem(at: container)
        }

        let url = try extensionStages([
            (name: "IMG_0001.HEIC", bytes: "first"),
            (name: "clip.mov", bytes: "second"),
            (name: "notes.pdf", bytes: "third"),
        ], into: container)

        session.handleShareURL(url, container: container)

        let imported = try #require(session.pendingShareImport)
        #expect(imported.map(\.lastPathComponent) == ["IMG_0001.HEIC", "clip.mov", "notes.pdf"])
        #expect(try imported.map { try String(contentsOf: $0, encoding: .utf8) }
                == ["first", "second", "third"])
    }

    /// The app MOVES the files out rather than copying them: the group
    /// container is a staging area, and a share that has been imported
    /// must not still be sitting in it the next time the app launches.
    @Test("Importing empties the inbox it read from")
    func importDrainsTheInbox() throws {
        let session = makeSession()
        let container = try makeContainer()
        defer {
            session.discardShareImports()
            try? FileManager.default.removeItem(at: container)
        }

        let url = try extensionStages([(name: "a.txt", bytes: "a")], into: container)
        let inbox = ShareHandoff.inboxURL(container: container)
        #expect(try FileManager.default.contentsOfDirectory(atPath: inbox.path).count == 1)

        session.handleShareURL(url, container: container)

        #expect(session.pendingShareImport?.count == 1)
        #expect(try FileManager.default.contentsOfDirectory(atPath: inbox.path).isEmpty)
    }

    /// A share arriving through the extension is still just staged: the
    /// picker has to choose a chat and the person still has to press Send.
    @Test("An imported share is parked for the picker, not addressed")
    func nothingIsAddressedYet() throws {
        let session = makeSession()
        let container = try makeContainer()
        defer {
            session.discardShareImports()
            try? FileManager.default.removeItem(at: container)
        }

        let url = try extensionStages([(name: "a.txt", bytes: "a")], into: container)
        session.handleShareURL(url, container: container)

        #expect(session.pendingShareImport?.count == 1)
        #expect(session.shareImportTarget == nil)
    }

    // MARK: - The URL, both directions

    /// The drift test in its smallest form: whatever the producer mints,
    /// the consumer reads back identically. If either side's spelling of
    /// the scheme, the host or the query key moved, this is where it dies
    /// — rather than in a share that quietly evaporates.
    @Test("Every URL the producer mints, the consumer reads back exactly")
    func producerAndConsumerAgree() throws {
        for count in [1, 2, ShareHandoff.maxAttachmentsPerMessage] {
            let ids = (0..<count).map { _ in UUID().uuidString }
            let url = try #require(ShareHandoff.handoffURL(ids: ids))
            #expect(ShareHandoff.ids(from: url) == ids, "round trip of \(count) ids")
        }
    }

    /// The producer refuses to mint what its consumer would refuse to
    /// read: an empty hand-off, or an id that is not a UUID. A path is
    /// built from that string on BOTH sides of the boundary.
    @Test("The producer cannot mint a URL its own consumer would refuse")
    func producerRefusesWhatConsumerWould() {
        #expect(ShareHandoff.handoffURL(ids: []) == nil)
        #expect(ShareHandoff.handoffURL(ids: ["../../Documents"]) == nil)
        #expect(ShareHandoff.handoffURL(ids: [UUID().uuidString, "not-a-uuid"]) == nil)
        #expect(ShareHandoff.stagingDirectory(
            container: URL(fileURLWithPath: "/tmp/group"), id: "../evil") == nil)
    }

    /// Both ends cap at the same ten — the protocol's
    /// `max_attachments_per_message`. The producer trims before it writes
    /// the URL, so an eleventh id never travels at all.
    @Test("Producer and consumer cap at the same ten")
    func bothEndsCapAtTen() throws {
        let ids = (0..<12).map { _ in UUID().uuidString }
        let url = try #require(ShareHandoff.handoffURL(ids: ids))
        #expect(ShareHandoff.ids(from: url) == Array(ids.prefix(10)))
        #expect(ShareHandoff.maxAttachmentsPerMessage == StagedAttachment.maxPerMessage)
    }

    // MARK: - A hostile filename

    /// The extension writes the name; the app reads back whatever it
    /// finds. A shared file called "../../evil.txt" must therefore not be
    /// able to place itself outside its own staging directory — cleaning
    /// happens on the producing side, and this walks it through to where
    /// the app puts the file.
    @Test("A filename that tries to climb out lands as a plain name")
    func hostileFilenameStaysPut() throws {
        let session = makeSession()
        let container = try makeContainer()
        defer {
            session.discardShareImports()
            try? FileManager.default.removeItem(at: container)
        }

        let url = try extensionStages([(name: "../../evil.txt", bytes: "x")], into: container)
        session.handleShareURL(url, container: container)

        let imported = try #require(session.pendingShareImport?.first)
        let name = imported.lastPathComponent
        #expect(!name.contains("/"))
        #expect(!name.hasPrefix("."))
        #expect(imported.deletingLastPathComponent().lastPathComponent.hasPrefix("fc-shared-"))
        #expect(try String(contentsOf: imported, encoding: .utf8) == "x")
    }

    // MARK: - The hand-offs that never complete

    /// Move an entry's clock back. Sleeping to age something would put a
    /// wall clock in a suite that runs concurrently on one actor; the
    /// modification date is what the sweep actually reads, so it is what
    /// a test sets.
    private func backdate(_ url: URL, by interval: TimeInterval) throws {
        try FileManager.default.setAttributes(
            [.modificationDate: Date().addingTimeInterval(-interval)],
            ofItemAtPath: url.path)
    }

    /// The staging directories a hand-off URL names — rebuilt the way the
    /// app rebuilds them, from the ids in the URL.
    private func stagedDirectories(of url: URL, in container: URL) throws -> [URL] {
        let ids = try #require(ShareHandoff.ids(from: url))
        return try ids.map {
            try #require(ShareHandoff.stagingDirectory(container: container, id: $0))
        }
    }

    /// The leak: the appex stages, and then nothing completes the
    /// hand-off — the open is dropped, the app is killed, the app is
    /// never opened. A completed import removes its own directory; this
    /// is what removes everybody else's (issue #35).
    @Test("A staging directory nothing ever claimed is swept once it is old")
    func abandonedStagingIsSwept() throws {
        let container = try makeContainer()
        defer { try? FileManager.default.removeItem(at: container) }

        let url = try extensionStages([(name: "big.mov", bytes: "abandoned")], into: container)
        let staged = try stagedDirectories(of: url, in: container)
        for directory in staged { try backdate(directory, by: 3 * 24 * 60 * 60) }

        #expect(ShareHandoff.sweepOrphanedStaging(container: container) == 1)
        let inbox = ShareHandoff.inboxURL(container: container)
        #expect(try FileManager.default.contentsOfDirectory(atPath: inbox.path).isEmpty)
        // The inbox itself stays: the appex expects to find it there.
        #expect(FileManager.default.fileExists(atPath: inbox.path))
    }

    /// THE case that matters. The extension stages and the app opens some
    /// time later; on a cold launch that gap is real, and a sweep that
    /// took a fresh directory would make shares evaporate at random. So
    /// the fresh one survives the sweep AND still imports afterwards —
    /// asserting the survival alone would pass on a sweep that left an
    /// empty husk behind.
    @Test("A staging directory still in flight is never swept")
    func inFlightStagingSurvives() throws {
        let session = makeSession()
        let container = try makeContainer()
        defer {
            session.discardShareImports()
            try? FileManager.default.removeItem(at: container)
        }

        let url = try extensionStages([(name: "IMG_0002.HEIC", bytes: "in flight")], into: container)
        #expect(ShareHandoff.sweepOrphanedStaging(container: container) == 0)

        session.handleShareURL(url, container: container)
        let imported = try #require(session.pendingShareImport?.first)
        #expect(imported.lastPathComponent == "IMG_0002.HEIC")
        #expect(try String(contentsOf: imported, encoding: .utf8) == "in flight")
    }

    /// The floor itself, from both sides, with an injected clock rather
    /// than a real one: at the grace it stays, a second past it goes.
    @Test("The grace period is a floor, not a rounding")
    func gracePeriodIsAFloor() throws {
        let container = try makeContainer()
        defer { try? FileManager.default.removeItem(at: container) }

        let url = try extensionStages([(name: "a.txt", bytes: "a")], into: container)
        let staged = try #require(stagedDirectories(of: url, in: container).first)
        let stamped = Date()
        try FileManager.default.setAttributes(
            [.modificationDate: stamped], ofItemAtPath: staged.path)

        #expect(ShareHandoff.sweepOrphanedStaging(
            container: container,
            now: stamped.addingTimeInterval(ShareHandoff.stagingGrace)) == 0)
        #expect(FileManager.default.fileExists(atPath: staged.path))

        #expect(ShareHandoff.sweepOrphanedStaging(
            container: container,
            now: stamped.addingTimeInterval(ShareHandoff.stagingGrace + 1)) == 1)
        #expect(!FileManager.default.fileExists(atPath: staged.path))
    }

    /// The sweep is total over whatever is in there. A directory the
    /// consumer would refuse to name (only a UUID becomes a path) and a
    /// bare FILE in the inbox are both things no import will ever claim,
    /// so age is the only question asked of them — and asked of them the
    /// same way, so a fresh one is still left alone.
    @Test("A non-UUID directory and a stray file are swept on age alone")
    func strayEntriesAreSweptOnAgeAlone() throws {
        let container = try makeContainer()
        defer { try? FileManager.default.removeItem(at: container) }
        let manager = FileManager.default
        let inbox = ShareHandoff.inboxURL(container: container)
        try manager.createDirectory(at: inbox, withIntermediateDirectories: true)

        let notAUUID = inbox.appendingPathComponent("scratch", isDirectory: true)
        try manager.createDirectory(at: notAUUID, withIntermediateDirectories: true)
        try Data("x".utf8).write(to: notAUUID.appendingPathComponent("leftover.jpg"))
        let strayFile = inbox.appendingPathComponent(".DS_Store")
        try Data("junk".utf8).write(to: strayFile)
        let fresh = inbox.appendingPathComponent("also-not-a-uuid", isDirectory: true)
        try manager.createDirectory(at: fresh, withIntermediateDirectories: true)

        // Backdate the directory AFTER writing into it: creating an entry
        // moves the parent's modification date, which is exactly the
        // stamp the sweep reads.
        try backdate(notAUUID, by: 2 * 24 * 60 * 60)
        try backdate(strayFile, by: 2 * 24 * 60 * 60)

        #expect(ShareHandoff.sweepOrphanedStaging(container: container) == 2)
        #expect(!manager.fileExists(atPath: notAUUID.path))
        #expect(!manager.fileExists(atPath: strayFile.path))
        #expect(manager.fileExists(atPath: fresh.path))
    }

    /// A clock moved backwards stamps things in the future. The sweep's
    /// one job is not to take what is in flight, so an entry it cannot
    /// place in the past is left — the subtraction goes negative and
    /// matches nothing.
    @Test("An entry stamped in the future is left alone")
    func futureStampsAreLeft() throws {
        let container = try makeContainer()
        defer { try? FileManager.default.removeItem(at: container) }

        let url = try extensionStages([(name: "a.txt", bytes: "a")], into: container)
        let staged = try #require(stagedDirectories(of: url, in: container).first)
        try FileManager.default.setAttributes(
            [.modificationDate: Date().addingTimeInterval(7 * 24 * 60 * 60)],
            ofItemAtPath: staged.path)

        #expect(ShareHandoff.sweepOrphanedStaging(container: container) == 0)
        #expect(FileManager.default.fileExists(atPath: staged.path))
    }

    /// The other half of the contract: a hand-off that DOES complete
    /// still cleans up after itself, at import time, exactly as it did
    /// before there was a sweep. The sweep is for what is left over, and
    /// after a completed import there is nothing left over.
    @Test("A completed import leaves the sweep nothing to do")
    func completedImportNeedsNoSweep() throws {
        let session = makeSession()
        let container = try makeContainer()
        defer {
            session.discardShareImports()
            try? FileManager.default.removeItem(at: container)
        }

        let url = try extensionStages([(name: "a.txt", bytes: "a")], into: container)
        session.handleShareURL(url, container: container)
        #expect(session.pendingShareImport?.count == 1)

        let inbox = ShareHandoff.inboxURL(container: container)
        #expect(try FileManager.default.contentsOfDirectory(atPath: inbox.path).isEmpty)
        // Even with every floor removed, because the directories are gone.
        #expect(ShareHandoff.sweepOrphanedStaging(container: container, olderThan: 0) == 0)
    }

    /// Both call sites are paths that must not fail — an app launch and
    /// an appex about to stage — so no container (an unsigned build with
    /// no App Group) and no inbox (nothing was ever shared) are answers,
    /// not errors.
    @Test("No container and no inbox are both a quiet zero")
    func sweepIsTotal() throws {
        #expect(ShareHandoff.sweepOrphanedStaging(container: nil) == 0)

        let container = try makeContainer()
        defer { try? FileManager.default.removeItem(at: container) }
        // Never shared: the inbox has not been created yet.
        #expect(ShareHandoff.sweepOrphanedStaging(container: container) == 0)
        #expect(ShareHandoff.sweepOrphanedStaging(
            container: container.appendingPathComponent("nowhere")) == 0)
    }

    /// 24 hours, which is the server's `limits.attachment_grace_hours`
    /// and the "Unclaimed attachments are deleted after 24 hours" promise
    /// in docs/protocol.md — the same abandoned send, one hop later. A
    /// client sweeping sooner than the server would promise more than the
    /// protocol does.
    @Test("The staging grace matches the server's unclaimed-upload grace")
    func graceMatchesTheServer() {
        #expect(ShareHandoff.stagingGrace == 24 * 60 * 60)
    }

    // MARK: - The halves a property list holds

    /// The constants are literals on purpose: the App Group is written
    /// out in both .entitlements files and the scheme in both Info.plists,
    /// and neither of those can read a Swift constant. Changing one of
    /// these strings is changing a contract with an already-installed
    /// build, so it should have to be done here first, deliberately.
    @Test("The hand-off constants are the shipped spellings")
    func constantsAreTheShippedSpellings() {
        #expect(ShareHandoff.scheme == "familyconnect")
        #expect(ShareHandoff.host == "share")
        #expect(ShareHandoff.appGroup == "group.me.nettrash.FamilyConnect")
        #expect(ShareHandoff.inboxFolder == "ShareInbox")
    }

    /// iOS will not hand the app a URL whose scheme the app does not
    /// register, so the extension would open nothing at all. Read from the
    /// BUILT bundle, not from the source plist.
    @Test("The app registers the scheme the extension opens")
    func theSchemeIsRegistered() throws {
        let types = try #require(
            Bundle.main.object(forInfoDictionaryKey: "CFBundleURLTypes") as? [[String: Any]])
        let schemes = types.flatMap { $0["CFBundleURLSchemes"] as? [String] ?? [] }
        #expect(schemes.contains(ShareHandoff.scheme))
    }

    /// The one number that HAS to be written twice: the activation rule
    /// caps how many items the share sheet will even offer the appex, and
    /// a property list cannot read `maxAttachmentsPerMessage`. Pinned
    /// against the built .appex so the two cannot drift apart unnoticed.
    @Test("The extension's activation rule caps at the same ten")
    func theActivationRuleMatches() throws {
        let plugIns = try #require(Bundle.main.builtInPlugInsURL)
        let appex = plugIns.appendingPathComponent("FamilyConnectShareExtension.appex")
        let bundle = try #require(Bundle(url: appex), "the share extension must ship inside the app")
        let extensionInfo = try #require(
            bundle.object(forInfoDictionaryKey: "NSExtension") as? [String: Any])
        let attributes = try #require(extensionInfo["NSExtensionAttributes"] as? [String: Any])
        let rule = try #require(attributes["NSExtensionActivationRule"] as? [String: Any])

        for key in [
            "NSExtensionActivationSupportsFileWithMaxCount",
            "NSExtensionActivationSupportsImageWithMaxCount",
            "NSExtensionActivationSupportsMovieWithMaxCount",
        ] {
            #expect(rule[key] as? Int == ShareHandoff.maxAttachmentsPerMessage, "\(key)")
        }
    }
}
