//
//  MediaOutbox.swift
//  FamilyConnect
//
//  What is left of the in-memory media outbox: the sweep that cleans up
//  after the composer.
//
//  WHAT THIS USED TO BE. A media send used to exist nowhere but a running
//  Task, so this type carried it: it took ownership of the prepared files
//  the moment Send was pressed, held a failed set until some composer for
//  that chat adopted it back, and stated plainly in its own header that it
//  deliberately did not survive a process kill. That was the best a
//  send-shaped-as-a-Task could do, and it is why suspending the app
//  mid-upload lost the message outright.
//
//  WHAT IT IS NOW. A media send is a row: `sendMedia` writes the message
//  and its `PendingMediaItemEntity` rows and moves the bytes into
//  `PendingMediaStaging` before the first byte goes out, so ownership
//  belongs to the store and recovery is the ordinary outbox. Nothing needs
//  to be held in memory, and a failure is a red bubble with tap-to-retry
//  rather than a set this type has to hand back.
//
//  WHAT REMAINS IS THE COMPOSER'S OWN LITTER. Files `MediaPrep` wrote for
//  a set the user staged and never sent are still ephemeral, still in
//  `tmp`, and still nobody's on a kill. That is correct — a staged photo is
//  a fragment of an intention, not a message — and this sweep is what
//  collects them at launch.
//

import Foundation
import os

nonisolated enum MediaOutbox {

    /// The prefix `MediaPrep` gives every file it writes for an upload.
    static let filePrefix = "fc-upload-"

    /// Delete prepared upload files nobody owns.
    ///
    /// - Parameters:
    ///   - age: only files older than this are taken. The default of `0`
    ///     means "take everything", which is right at launch: a process
    ///     that has just started owns no prepared file, so every one of
    ///     them is by definition an orphan.
    ///   - directory: `tmp` in the app, a scratch directory in tests.
    ///
    /// It must never be pointed at `PendingMediaStaging`'s root. Those
    /// files belong to messages somebody has pressed Send on, and sweeping
    /// them at launch would destroy exactly the sends this app now goes to
    /// the trouble of resuming.
    @discardableResult
    static func sweepOrphans(
        olderThan age: TimeInterval = 0,
        now: Date = Date(),
        in directory: URL = FileManager.default.temporaryDirectory
    ) -> Int {
        let manager = FileManager.default
        guard let names = try? manager.contentsOfDirectory(atPath: directory.path) else { return 0 }
        var removed = 0
        for name in names where name.hasPrefix(filePrefix) {
            let url = directory.appendingPathComponent(name)
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
}
