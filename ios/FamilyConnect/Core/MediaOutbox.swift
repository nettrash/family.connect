//
//  MediaOutbox.swift
//  FamilyConnect
//
//  Who owns a media send while it is in the air.
//
//  THE PROBLEM THIS SOLVES. `sendMedia` uploads every attachment and only
//  then enqueues the message row — deliberate, because the row claims the
//  whole set at once. The consequence is that a send in flight lives ONLY
//  as a running Task's local: `sendStaged` clears the composer's `staged`
//  before the upload begins, so if the reader leaves the chat mid-upload,
//  the failure path writes its restore into `@State` a dismissed view no
//  longer owns. The caption, the reply target and the prepared files then
//  vanish with nothing left holding them, and nothing to delete them.
//
//  THE RULE THIS TYPE EXISTS TO ENFORCE: a prepared file has exactly one
//  owner at a time. From the moment a send begins, that owner is this
//  outbox — not the composer, not the Task. `sendMedia` still consumes the
//  files on success, and `finish(_:)` is how it says so.
//
//  WHY A TOKEN AND NOT A CHAT ID. A chat can have two sends in flight at
//  once (leave the chat mid-upload, come back, send something else — the
//  second composer starts at `.idle` and knows nothing of the first). An
//  earlier attempt at this keyed everything by chat id, and every defect it
//  produced came from that one choice: a successful send deleted an
//  unrelated failed set's files; a retry never consumed its own entry, so
//  photos that DID send came back forever; a set the reader explicitly
//  discarded reappeared with dead file URLs. Ownership needs identity.
//
//  WHY @Observable AND NOT A DICTIONARY READ ONCE. A failure that lands
//  while a composer is on screen has to reach it. Reading a static store in
//  `.task` means the reader learns their send failed only if they happen to
//  navigate away and back.
//
//  WHAT THIS DELIBERATELY DOES NOT DO: survive a process kill. The entries
//  are in memory, exactly like `ComposerDrafts` — a send is a fragment of
//  an intention, not data. `sweepOrphans()` is what cleans up after the
//  kill, and issue #11's own Fix asks for a stash, not persistence. Making
//  a media send resumable across a relaunch means representing it in the
//  model, which is a different and much larger change.
//

import Foundation
import Observation
import os

@MainActor
@Observable
final class MediaOutbox {

    /// One send, from the moment it starts until it lands or is thrown away.
    struct Entry: Identifiable {
        let id: UUID
        let chatID: Int64
        let caption: String
        let replyTo: ReplyToDTO?
        let prepared: [MediaPrep.Prepared]
        /// False while uploading; true once the send has failed and the set
        /// is waiting for a composer to take it back.
        var isFailed: Bool
    }

    private(set) var entries: [Entry] = []

    /// The prefix `MediaPrep` gives every file it writes, and the only
    /// thing `sweepOrphans` will delete.
    static let filePrefix = "fc-upload-"

    // MARK: - The lifecycle of one send

    /// Take ownership of a set as its upload begins.
    ///
    /// Called BEFORE the first byte goes out, not on failure: between those
    /// two moments the set belongs to nobody, and that gap is the bug.
    func begin(
        chatID: Int64,
        caption: String,
        replyTo: ReplyToDTO?,
        prepared: [MediaPrep.Prepared]
    ) -> UUID {
        let token = UUID()
        entries.append(Entry(id: token, chatID: chatID, caption: caption,
                             replyTo: replyTo, prepared: prepared, isFailed: false))
        return token
    }

    /// The send landed. `sendMedia` has already deleted the files, so this
    /// drops the entry WITHOUT touching disk — deleting here would be a
    /// second delete of files that are gone, and, if the token were ever
    /// wrong, a delete of somebody else's set.
    func finish(_ token: UUID) {
        entries.removeAll { $0.id == token }
    }

    /// The send failed. The files are still on disk — `sendMedia` consumes
    /// them only on whole-send success — so the entry stays, and a composer
    /// for this chat can offer them back.
    func fail(_ token: UUID) {
        guard let index = entries.firstIndex(where: { $0.id == token }) else { return }
        entries[index].isFailed = true
    }

    /// Give a failed set back to a composer, which becomes its owner again.
    ///
    /// Only a FAILED entry can be taken: one still uploading is not waiting
    /// to be handed back, and handing it over would let the composer send
    /// the same files a second time while the first send is still running.
    func take(_ token: UUID) -> Entry? {
        guard let index = entries.firstIndex(where: { $0.id == token }),
              entries[index].isFailed
        else { return nil }
        return entries.remove(at: index)
    }

    /// How many failed sets are waiting for this chat.
    ///
    /// The composer observes THIS rather than taking on failure. A take
    /// removes the entry whether or not the view asking is still on screen,
    /// so the send path must never take — it marks the entry failed and
    /// lets observation decide who adopts it. Only a view that is actually
    /// installed reacts to the change, which is precisely the distinction
    /// the send path cannot make for itself.
    func failedCount(for chatID: Int64) -> Int {
        entries.reduce(0) { $0 + (($1.chatID == chatID && $1.isFailed) ? 1 : 0) }
    }

    /// The most recently failed set for a chat, if one is waiting.
    ///
    /// Most recent, not oldest. A composer whose own send just failed calls
    /// this, and handing it a different, older stranded set would restore
    /// the wrong photos AND leave its own behind — with the notice on
    /// screen describing neither of them.
    func mostRecentFailed(for chatID: Int64) -> Entry? {
        entries.last { $0.chatID == chatID && $0.isFailed }
    }

    /// Throw a set away at the reader's request, deleting its files.
    ///
    /// This is the counterpart to the composer's own discard: whoever holds
    /// the set deletes it, and only one of them holds it at a time.
    func discard(_ token: UUID) {
        guard let index = entries.firstIndex(where: { $0.id == token }) else { return }
        let entry = entries.remove(at: index)
        Self.deleteFiles(of: entry)
    }

    // MARK: - Identity changes

    /// Everything, files included. A set composed in one account's family
    /// must never be handed to the next — the same reason `AppSession.purge`
    /// clears the chat store, the avatar cache and the contact links.
    func purgeAll() {
        for entry in entries { Self.deleteFiles(of: entry) }
        entries.removeAll()
    }

    // MARK: - What survives a kill

    /// Delete prepared files that no live entry owns.
    ///
    /// Called once at launch. A process killed mid-upload leaves its files
    /// behind with the entry that owned them gone, and nothing else in the
    /// app will ever look at them again; without this they accumulate for
    /// the life of the install. The age floor keeps it away from files a
    /// send in this run is actively using.
    ///
    /// Only `fc-upload-*` in the temporary directory, and only entries
    /// older than `olderThan` — never a blanket clear of tmp, which is
    /// shared with the share extension's own staging.
    /// - Parameter olderThan: a floor for callers that run while sends may
    ///   be in flight. **At launch it should be zero**: a process that has
    ///   just started owns no prepared file, so every one of them is by
    ///   definition an orphan, and a 24-hour floor spares exactly the files
    ///   the crash that just happened left behind — the case this exists
    ///   for.
    static func sweepOrphans(
        olderThan age: TimeInterval = 0,
        now: Date = Date(),
        in directory: URL = FileManager.default.temporaryDirectory,
        keeping live: Set<URL> = []
    ) -> Int {
        let manager = FileManager.default
        guard let names = try? manager.contentsOfDirectory(atPath: directory.path) else { return 0 }
        var removed = 0
        for name in names where name.hasPrefix(filePrefix) {
            let url = directory.appendingPathComponent(name)
            if live.contains(url) { continue }
            if age > 0 {
                let created = (try? manager.attributesOfItem(atPath: url.path)[.creationDate]) as? Date
                // No creation date and a floor to honour: leave it rather
                // than guess. With no floor (the launch case) the question
                // does not arise — nothing in this process owns it.
                guard let created, now.timeIntervalSince(created) > age else { continue }
            }
            if (try? manager.removeItem(at: url)) != nil { removed += 1 }
        }
        if removed > 0 {
            AppLog.app.info("Swept \(removed, privacy: .public) orphaned upload file(s)")
        }
        return removed
    }

    /// The files every live entry owns — what a sweep must not touch.
    var liveFileURLs: Set<URL> {
        Set(entries.flatMap { $0.prepared.map(\.fileURL) })
    }

    private static func deleteFiles(of entry: Entry) {
        for item in entry.prepared {
            try? FileManager.default.removeItem(at: item.fileURL)
        }
    }
}
