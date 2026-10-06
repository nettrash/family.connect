//
//  ParkedRecordings.swift
//  FamilyConnect
//
//  Where a voice recording nobody finished deciding about waits: the
//  "Voice message not sent" row (#79, docs/audio-video-messages-2026-10-04.md,
//  S2.8, Decision 11).
//
//  A recording stopped by something other than the person — a call, the app
//  going to the background, the screen locking, a Mac sleeping, a window
//  minimised or closed, Siri or another app taking the microphone, the
//  recorder failing — and a voice note still in review when the person
//  leaves the chat, land here instead of being lost. Each waits in its
//  chat's own row until it is sent or deleted, and is NEVER carried by
//  another Send: a recording nobody finished deciding about must not ride
//  out with the next text.
//
//  This reverses ComposerDrafts' "a recording in progress … [is]
//  deliberately let go" — for recordings only, because a photo can be picked
//  again and a recording cannot be made again.
//
//  WHAT AN ENTRY HOLDS is what S2.8 lists — the file, its length, the reply
//  it was recorded under and its caption. The reply is kept WHOLE rather than
//  as an id: the row quotes it, and the send needs the sender and the
//  excerpt for its pending bubble, and a quoted message may long since have
//  scrolled out of the cache.
//
//  WHERE: `<Application Support>/FamilyConnect/ParkedRecordings/<account>/`,
//  one directory per account (server and user), each with its files and an
//  `index.json`. Application Support because it survives the app being
//  closed, which tmp does not promise; per account so one person's recording
//  can never be offered to whoever signs in next, even if a sign-out's clear
//  never ran; excluded from backup, as the outbox's bytes are. File names
//  are stored, never absolute paths (PendingMediaStaging's rule: a
//  container's path moves across reinstalls).
//
//  LIFETIME: swept at launch (files no entry names, entries whose file is
//  gone), and deleted at sign-out — "everything recorded and not sent is
//  deleted" (S4).
//

import Foundation
import Observation

@MainActor
@Observable
final class ParkedRecordings {

    nonisolated struct Entry: Codable, Equatable, Identifiable, Sendable {
        let id: String
        let chatID: Int64
        /// The file's name inside its account's directory.
        let fileName: String
        /// By the recorder's own clock — the one the person watched.
        let durationMS: Int
        /// The reply it was recorded under, sent with it and nothing else.
        let replyTo: ReplyToDTO?
        /// The words that were in the field when a note in review was
        /// parked; never sent without the note.
        let caption: String?
        let createdAt: Date
        // An index written while the hold existed (before 2026-10-06) also
        // carries `sending`, the mark of a note in its Undo window. Decoding
        // ignores the key, so such an entry — one a crash left inside the
        // window — is an ordinary "not sent" row: what the launch sweep
        // made of it then, too.
        /// Its waveform, as the recorder measured it — drawn on the row's
        /// mini waveform and sent with the note (#79). Absent from an index
        /// written before it existed, which decodes as nil: the row draws
        /// the placeholder and the note goes without one.
        var waveform: String? = nil

        var duration: TimeInterval { Double(durationMS) / 1000 }

        /// Deleting it asks first (S1.1, `DELETE_ASKS_FROM_MS`).
        var deleteAsks: Bool { durationMS >= ParkedRecordings.deleteAsksFromMS }
    }

    /// Deleting a recording this long or longer asks "Delete this recording?".
    nonisolated static let deleteAsksFromMS = Int(RecordRules.deleteAsksFromMS)

    /// The app's store.
    static let shared = ParkedRecordings()

    private let root: () throws -> URL
    private let account: () -> String?
    private let now: () -> Date

    /// Bumped by every change. A view reading `entries(for:)` is redrawn by
    /// it — the cache below is deliberately not observed, so that loading it
    /// from disk inside a view's body is not itself a change.
    private(set) var revision = 0
    @ObservationIgnored private var cache: (account: String, entries: [Entry])?

    /// - Parameters:
    ///   - root: the directory every account's lives in. A test passes a
    ///     scratch directory of its own.
    ///   - account: whose recordings these are — nil when nobody is signed
    ///     in, and then nothing can be parked.
    init(
        root: @escaping () throws -> URL = ParkedRecordings.defaultRoot,
        account: @escaping () -> String? = {
            ParkedRecordings.accountKey(serverURL: AppSettings.serverURL, userID: AppSettings.currentUserID)
        },
        now: @escaping () -> Date = Date.init
    ) {
        self.root = root
        self.account = account
        self.now = now
    }

    // MARK: - Reading

    /// The chat's not-sent voice messages, oldest first.
    func entries(for chatID: Int64) -> [Entry] {
        _ = revision
        return currentEntries().filter { $0.chatID == chatID }
    }

    /// Where an entry's file is, if it is still there.
    func fileURL(for entry: Entry) -> URL? {
        guard let key = account(), let directory = try? directory(for: key) else { return nil }
        let url = directory.appendingPathComponent(entry.fileName)
        return FileManager.default.fileExists(atPath: url.path) ? url : nil
    }

    /// The entry as something the outbox can send — the file where it is,
    /// as the voice note it already is (recorded to the profile, so nothing
    /// is re-encoded), with no name: a voice note's identity is its length.
    ///
    /// `sendMedia` COPIES a file it does not own (PendingMediaStaging.adopt:
    /// only tmp is ours to move), so the entry is still whole if the send
    /// could not be queued, and is removed once it was.
    func prepared(for entry: Entry) -> MediaPrep.Prepared? {
        guard let url = fileURL(for: entry) else { return nil }
        var prepared = MediaPrep.Prepared(
            fileURL: url,
            mime: MediaPrep.audioMIME(for: url),
            kind: AttachmentDTO.Kind.audio,
            durationMS: entry.durationMS,
            name: nil)
        prepared.waveform = entry.waveform
        return prepared
    }

    // MARK: - Writing

    /// Keep a recording: its file MOVES here.
    ///
    /// Returns nil, with the file left exactly where it was, when nobody is
    /// signed in or the disk would not take it — the caller still owns it
    /// then, and decides.
    @discardableResult
    func park(
        fileAt source: URL,
        duration: TimeInterval,
        chatID: Int64,
        replyTo: ReplyToDTO?,
        caption: String?,
        waveform: String? = nil
    ) -> Entry? {
        guard let key = account(), let directory = try? directory(for: key) else { return nil }
        let id = UUID().uuidString.lowercased()
        let ext = source.pathExtension.isEmpty ? "m4a" : source.pathExtension
        let fileName = "\(id).\(ext)"
        let destination = directory.appendingPathComponent(fileName)
        do {
            try Self.move(source, to: destination)
        } catch {
            return nil
        }
        let words = caption?.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty == false ? caption : nil
        let entry = Entry(
            id: id,
            chatID: chatID,
            fileName: fileName,
            durationMS: Int((max(0, duration) * 1000).rounded()),
            replyTo: replyTo,
            caption: words,
            createdAt: now(),
            waveform: waveform)
        var entries = load(key)
        entries.append(entry)
        guard save(entries, for: key) else {
            // The index would not take it: give the caller its file back.
            try? Self.move(destination, to: source)
            return nil
        }
        cache = (key, entries)
        revision += 1
        return entry
    }

    /// Sent or deleted: the entry and its file go.
    func remove(_ entry: Entry) {
        guard let key = account() else { return }
        if let directory = try? directory(for: key) {
            try? FileManager.default.removeItem(at: directory.appendingPathComponent(entry.fileName))
        }
        var entries = load(key)
        entries.removeAll { $0.id == entry.id }
        _ = save(entries, for: key)
        cache = (key, entries)
        revision += 1
    }

    /// Sign-out: every parked recording of every account on this device.
    func removeAll() {
        if let root = try? root() {
            try? FileManager.default.removeItem(at: root)
        }
        cache = nil
        revision += 1
    }

    /// At launch. Deletes the files no entry names and drops the entries
    /// whose file is gone; and deletes other accounts' directories — a
    /// sign-out whose clear never finished. With nobody signed in it does
    /// nothing at all, rather than guess.
    ///
    /// Returns how many files and directories it removed.
    @discardableResult
    func sweep() -> Int {
        guard let key = account(), let base = try? root() else { return 0 }
        let manager = FileManager.default
        var removed = 0
        for other in (try? manager.contentsOfDirectory(at: base, includingPropertiesForKeys: nil)) ?? []
        where other.lastPathComponent != key {
            if (try? manager.removeItem(at: other)) != nil { removed += 1 }
        }
        guard let directory = try? directory(for: key) else { return removed }
        let before = load(key)
        let entries = before.filter {
            manager.fileExists(atPath: directory.appendingPathComponent($0.fileName).path)
        }
        let named = Set(entries.map(\.fileName)).union([Self.indexName])
        for file in (try? manager.contentsOfDirectory(at: directory, includingPropertiesForKeys: nil)) ?? []
        where !named.contains(file.lastPathComponent) {
            if (try? manager.removeItem(at: file)) != nil { removed += 1 }
        }
        if entries != before { _ = save(entries, for: key) }
        cache = (key, entries)
        revision += 1
        return removed
    }

    // MARK: - The account

    /// The directory name for one account on one server, or nil when nobody
    /// is signed in. The server is folded to a short stable hash: two
    /// servers on one host, or one behind a path, are still two.
    nonisolated static func accountKey(serverURL: URL?, userID: Int64?) -> String? {
        guard let userID else { return nil }
        var hash: UInt64 = 0xcbf2_9ce4_8422_2325
        for byte in (serverURL?.absoluteString ?? "").utf8 {
            hash ^= UInt64(byte)
            hash = hash &* 0x0000_0100_0000_01b3
        }
        return "u\(userID)-\(String(hash, radix: 16))"
    }

    // MARK: - Disk

    nonisolated static let indexName = "index.json"

    /// `<Application Support>/FamilyConnect/ParkedRecordings`.
    nonisolated static func defaultRoot() throws -> URL {
        let support = try FileManager.default.url(
            for: .applicationSupportDirectory, in: .userDomainMask,
            appropriateFor: nil, create: true)
        return support.appendingPathComponent("FamilyConnect/ParkedRecordings", isDirectory: true)
    }

    private func directory(for key: String) throws -> URL {
        let base = try root()
        let manager = FileManager.default
        if !manager.fileExists(atPath: base.path) {
            try manager.createDirectory(at: base, withIntermediateDirectories: true)
            // A recording on its way to being sent is not something to
            // carry into somebody's backup.
            var values = URLResourceValues()
            values.isExcludedFromBackup = true
            var marked = base
            try? marked.setResourceValues(values)
        }
        let url = base.appendingPathComponent(key, isDirectory: true)
        if !manager.fileExists(atPath: url.path) {
            try manager.createDirectory(at: url, withIntermediateDirectories: true)
        }
        return url
    }

    private func currentEntries() -> [Entry] {
        guard let key = account() else { return [] }
        if let cache, cache.account == key { return cache.entries }
        let entries = load(key)
        cache = (key, entries)
        return entries
    }

    private func load(_ key: String) -> [Entry] {
        guard let base = try? root() else { return [] }
        let index = base.appendingPathComponent(key).appendingPathComponent(Self.indexName)
        guard let data = try? Data(contentsOf: index) else { return [] }
        return (try? JSONDecoder().decode([Entry].self, from: data)) ?? []
    }

    private func save(_ entries: [Entry], for key: String) -> Bool {
        guard let directory = try? directory(for: key),
              let data = try? JSONEncoder().encode(entries)
        else { return false }
        return (try? data.write(to: directory.appendingPathComponent(Self.indexName), options: .atomic)) != nil
    }

    /// A move, or a copy and a delete where a move cannot cross volumes.
    nonisolated private static func move(_ source: URL, to destination: URL) throws {
        let manager = FileManager.default
        do {
            try manager.moveItem(at: source, to: destination)
        } catch {
            try manager.copyItem(at: source, to: destination)
            try? manager.removeItem(at: source)
        }
    }
}
