//
//  ParkedRecordingsTests.swift
//  FamilyConnectTests
//
//  The "Voice message not sent" store (#79, docs/audio-video-messages-2026-10-04.md,
//  S2.8): per account, on disk, surviving the app being closed, swept at
//  launch and emptied at sign-out. Every test works in a scratch root of
//  its own — never the real Application Support, which on an unsigned Mac
//  test host is the developer's own.
//

import Foundation
import Testing
@testable import FamilyConnect

@MainActor
@Suite("Parked recordings: the not-sent store")
struct ParkedRecordingsTests {

    final class Account {
        var key: String?
        init(_ key: String?) { self.key = key }
    }

    private func scratchRoot() -> URL {
        URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("parked-\(UUID().uuidString)", isDirectory: true)
    }

    private func store(root: URL, account: Account) -> ParkedRecordings {
        ParkedRecordings(root: { root }, account: { account.key })
    }

    /// A recording as the recorder leaves it: a file in a temporary place.
    private func recordingFile(bytes: Int = 2048) throws -> URL {
        let dir = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("parked-src-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let url = dir.appendingPathComponent("\(AudioRecorder.filePrefix)\(UUID().uuidString).m4a")
        try Data(count: bytes).write(to: url)
        return url
    }

    private static let reply = ReplyToDTO(messageID: 41, senderID: 7, excerpt: "Are you coming?")

    // MARK: - Keeping one

    @Test("parking moves the file in, and the entry belongs to its chat alone")
    func parkMovesTheFile() throws {
        let root = scratchRoot()
        let parked = store(root: root, account: Account("u7-a"))
        let source = try recordingFile()

        let entry = try #require(parked.park(
            fileAt: source, duration: 42.4, chatID: 5, replyTo: nil, caption: nil))

        #expect(!FileManager.default.fileExists(atPath: source.path), "the recording was copied, not kept")
        let kept = try #require(parked.fileURL(for: entry))
        #expect(kept.path.hasPrefix(root.path))
        #expect(entry.durationMS == 42_400)
        #expect(parked.entries(for: 5) == [entry])
        #expect(parked.entries(for: 6).isEmpty)
    }

    @Test("an entry survives the app being closed")
    func survivesARelaunch() throws {
        let root = scratchRoot()
        let account = Account("u7-a")
        let entry = try #require(store(root: root, account: account).park(
            fileAt: try recordingFile(), duration: 12, chatID: 5,
            replyTo: Self.reply, caption: "On my way"))

        let relaunched = store(root: root, account: account)

        #expect(relaunched.entries(for: 5) == [entry])
        #expect(relaunched.fileURL(for: entry) != nil)
    }

    @Test("its reply and caption come back exactly, and a blank caption is no caption")
    func replyAndCaption() throws {
        let parked = store(root: scratchRoot(), account: Account("u7-a"))
        let withWords = try #require(parked.park(
            fileAt: try recordingFile(), duration: 3, chatID: 5, replyTo: Self.reply, caption: "On my way"))
        let blank = try #require(parked.park(
            fileAt: try recordingFile(), duration: 3, chatID: 5, replyTo: nil, caption: "  \n "))

        #expect(withWords.replyTo == Self.reply)
        #expect(withWords.caption == "On my way")
        #expect(blank.caption == nil)
        #expect(blank.replyTo == nil)
    }

    @Test("entries keep the order they were parked in")
    func parkingOrder() throws {
        var clock = Date(timeIntervalSince1970: 1_000)
        let parked = ParkedRecordings(
            root: { [root = scratchRoot()] in root }, account: { "u7-a" },
            now: { clock })
        let first = try #require(parked.park(fileAt: try recordingFile(), duration: 2, chatID: 5, replyTo: nil, caption: nil))
        clock = clock.addingTimeInterval(-500)
        let second = try #require(parked.park(fileAt: try recordingFile(), duration: 2, chatID: 5, replyTo: nil, caption: nil))

        #expect(parked.entries(for: 5).map(\.id) == [first.id, second.id])
    }

    @Test("removing an entry deletes its file too")
    func removeDeletesTheFile() throws {
        let parked = store(root: scratchRoot(), account: Account("u7-a"))
        let entry = try #require(parked.park(
            fileAt: try recordingFile(), duration: 3, chatID: 5, replyTo: nil, caption: nil))
        let file = try #require(parked.fileURL(for: entry))

        parked.remove(entry)

        #expect(parked.entries(for: 5).isEmpty)
        #expect(!FileManager.default.fileExists(atPath: file.path), "a deleted recording left its sound behind")
    }

    @Test("every change is one the views can see")
    func changesAreObservable() throws {
        let parked = store(root: scratchRoot(), account: Account("u7-a"))
        let start = parked.revision
        let entry = try #require(parked.park(
            fileAt: try recordingFile(), duration: 3, chatID: 5, replyTo: nil, caption: nil))
        #expect(parked.revision > start)
        let afterPark = parked.revision
        parked.remove(entry)
        #expect(parked.revision > afterPark)
        let afterRemove = parked.revision
        parked.removeAll()
        #expect(parked.revision > afterRemove)
    }

    // MARK: - Per account

    @Test("nobody signed in: nothing is parked, and the file stays the caller's")
    func nobodySignedIn() throws {
        let parked = store(root: scratchRoot(), account: Account(nil))
        let source = try recordingFile()

        #expect(parked.park(fileAt: source, duration: 3, chatID: 5, replyTo: nil, caption: nil) == nil)
        #expect(FileManager.default.fileExists(atPath: source.path))
        #expect(parked.entries(for: 5).isEmpty)
    }

    @Test("a disk that will not take it hands the file back")
    func unwritableRoot() throws {
        // The root's parent is a FILE, so no directory can be made under it.
        let blocker = scratchRoot()
        try Data([0x00]).write(to: blocker)
        let parked = store(root: blocker.appendingPathComponent("parked"), account: Account("u7-a"))
        let source = try recordingFile()

        #expect(parked.park(fileAt: source, duration: 3, chatID: 5, replyTo: nil, caption: nil) == nil)
        #expect(FileManager.default.fileExists(atPath: source.path))
    }

    @Test("one account never sees another's recordings")
    func accountsAreSeparate() throws {
        let root = scratchRoot()
        let account = Account("u7-a")
        let parked = store(root: root, account: account)
        _ = try #require(parked.park(
            fileAt: try recordingFile(), duration: 3, chatID: 5, replyTo: nil, caption: nil))

        account.key = "u8-a"

        #expect(parked.entries(for: 5).isEmpty, "the next person to sign in was offered somebody else's recording")
    }

    @Test("sign-out empties every account's recordings")
    func removeAllEmptiesEverything() throws {
        let root = scratchRoot()
        let account = Account("u7-a")
        let parked = store(root: root, account: account)
        _ = try #require(parked.park(fileAt: try recordingFile(), duration: 3, chatID: 5, replyTo: nil, caption: nil))
        account.key = "u8-b"
        _ = try #require(parked.park(fileAt: try recordingFile(), duration: 3, chatID: 9, replyTo: nil, caption: nil))

        parked.removeAll()

        #expect(!FileManager.default.fileExists(atPath: root.path))
        #expect(parked.entries(for: 9).isEmpty)
        account.key = "u7-a"
        #expect(parked.entries(for: 5).isEmpty)
    }

    @Test("the account key: none without a user, one per server and per user, and safe as a name")
    func accountKey() throws {
        let server = URL(string: "https://family.example.com")
        #expect(ParkedRecordings.accountKey(serverURL: server, userID: nil) == nil)
        let key = try #require(ParkedRecordings.accountKey(serverURL: server, userID: 7))
        #expect(ParkedRecordings.accountKey(serverURL: server, userID: 7) == key)
        #expect(ParkedRecordings.accountKey(serverURL: server, userID: 8) != key)
        #expect(ParkedRecordings.accountKey(serverURL: URL(string: "https://other.example.com"), userID: 7) != key)
        #expect(ParkedRecordings.accountKey(serverURL: URL(string: "https://family.example.com/b"), userID: 7) != key)
        #expect(key.allSatisfy { $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "-") })
    }

    // MARK: - The launch sweep

    @Test("the launch sweep takes files no entry names and entries whose file is gone")
    func sweepRepairs() throws {
        let root = scratchRoot()
        let account = Account("u7-a")
        let parked = store(root: root, account: account)
        let kept = try #require(parked.park(fileAt: try recordingFile(), duration: 3, chatID: 5, replyTo: nil, caption: nil))
        let lost = try #require(parked.park(fileAt: try recordingFile(), duration: 4, chatID: 5, replyTo: nil, caption: nil))
        try FileManager.default.removeItem(at: try #require(parked.fileURL(for: lost)))
        let directory = root.appendingPathComponent("u7-a")
        let stray = directory.appendingPathComponent("stray.m4a")
        try Data(count: 10).write(to: stray)

        let relaunched = store(root: root, account: account)
        let removed = relaunched.sweep()

        #expect(removed == 1)
        #expect(!FileManager.default.fileExists(atPath: stray.path))
        #expect(relaunched.entries(for: 5).map(\.id) == [kept.id], "an entry with no sound could never be sent")
        #expect(store(root: root, account: account).entries(for: 5).map(\.id) == [kept.id], "the repair was not written down")
    }

    /// An index written while the hold existed (before 2026-10-06) also
    /// carried `sending`, the mark of a note in its Undo window. Such an
    /// entry — one a crash left inside the window — must read as an ordinary
    /// not-sent row on an upgraded device, never be dropped as unreadable.
    @Test("an index written while the hold existed reads its 'sending' entry as an ordinary not-sent one")
    func legacySendingEntry() throws {
        let root = scratchRoot()
        let account = Account("u7-a")
        let entry = try #require(store(root: root, account: account).park(
            fileAt: try recordingFile(), duration: 3, chatID: 5, replyTo: nil, caption: nil))
        // The index as the old build wrote it: the same entry, marked sending.
        let index = root.appendingPathComponent("u7-a").appendingPathComponent(ParkedRecordings.indexName)
        var objects = try #require(
            try JSONSerialization.jsonObject(with: Data(contentsOf: index)) as? [[String: Any]])
        objects[0]["sending"] = true
        try JSONSerialization.data(withJSONObject: objects).write(to: index)

        let relaunched = store(root: root, account: account)
        relaunched.sweep()

        #expect(relaunched.entries(for: 5).map(\.id) == [entry.id], "an upgraded device lost its not-sent note")
        #expect(relaunched.fileURL(for: entry) != nil)
    }

    @Test("the launch sweep takes another account's leftovers")
    func sweepTakesOtherAccounts() throws {
        let root = scratchRoot()
        let account = Account("u7-a")
        _ = try #require(store(root: root, account: account).park(
            fileAt: try recordingFile(), duration: 3, chatID: 5, replyTo: nil, caption: nil))

        account.key = "u8-a"
        store(root: root, account: account).sweep()

        #expect(!FileManager.default.fileExists(atPath: root.appendingPathComponent("u7-a").path))
    }

    @Test("with nobody signed in the launch sweep touches nothing")
    func sweepWithNobody() throws {
        let root = scratchRoot()
        let account = Account("u7-a")
        let entry = try #require(store(root: root, account: account).park(
            fileAt: try recordingFile(), duration: 3, chatID: 5, replyTo: nil, caption: nil))

        account.key = nil
        #expect(store(root: root, account: account).sweep() == 0)

        account.key = "u7-a"
        #expect(store(root: root, account: account).entries(for: 5) == [entry])
    }

    // MARK: - Sending and deleting

    @Test("sending hands over the parked file as the voice note it is")
    func preparedForSending() throws {
        let parked = store(root: scratchRoot(), account: Account("u7-a"))
        let entry = try #require(parked.park(
            fileAt: try recordingFile(), duration: 42, chatID: 5, replyTo: Self.reply, caption: "Hi"))

        let prepared = try #require(parked.prepared(for: entry))

        #expect(prepared.kind == AttachmentDTO.Kind.audio)
        #expect(prepared.mime == "audio/mp4")
        #expect(prepared.durationMS == 42_000)
        #expect(prepared.name == nil, "a voice note's identity is its length, not a scratch file name")
        #expect(prepared.fileURL == parked.fileURL(for: entry))
    }

    /// The outbox MOVES what is ours (tmp) and copies anything else. The real
    /// store lives outside tmp, so a send copies the parked file and the
    /// entry stays whole until the send is queued — and `MediaPrep.discard`,
    /// asked the same question, can never delete one.
    @Test("the real store is not somewhere the outbox may move files out of")
    func realRootIsNotTmp() throws {
        let root = try ParkedRecordings.defaultRoot()
        #expect(!PendingMediaStaging.isOurs(root.appendingPathComponent("u7-a/x.m4a")))
        #expect(root.path.contains("Application Support"))
    }

    @Test("an entry whose file is gone cannot be sent")
    func nothingToSend() throws {
        let parked = store(root: scratchRoot(), account: Account("u7-a"))
        let entry = try #require(parked.park(
            fileAt: try recordingFile(), duration: 3, chatID: 5, replyTo: nil, caption: nil))
        try FileManager.default.removeItem(at: try #require(parked.fileURL(for: entry)))

        #expect(parked.prepared(for: entry) == nil)
    }

    @Test("deleting asks first from ten seconds")
    func deleteAsks() throws {
        let parked = store(root: scratchRoot(), account: Account("u7-a"))
        let short = try #require(parked.park(fileAt: try recordingFile(), duration: 9.999, chatID: 5, replyTo: nil, caption: nil))
        let long = try #require(parked.park(fileAt: try recordingFile(), duration: 10, chatID: 5, replyTo: nil, caption: nil))

        #expect(!short.deleteAsks)
        #expect(long.deleteAsks)
    }
}
