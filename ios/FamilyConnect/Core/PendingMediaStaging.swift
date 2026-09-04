//
//  PendingMediaStaging.swift
//  FamilyConnect
//
//  Where the bytes of a queued media send live until the server has them.
//
//  WHY NOT `tmp`. Everything `MediaPrep` produces is written to the
//  temporary directory, which the system may empty at any moment the app
//  is not running and does empty under disk pressure. That is the right
//  home for a file the composer is still holding — a fragment of an
//  intention — and the wrong home for a message somebody has pressed Send
//  on. Once a send is a row in the store, its bytes have to outlive a
//  relaunch, so they move here: Application Support, which the system does
//  not purge.
//
//  ONE DIRECTORY PER ITEM, and the file keeps its REAL NAME inside it. The
//  name is a file attachment's whole identity — it is what the recipient
//  downloads — and two people sending `Invoice.pdf` must not collide. Same
//  shape as the download cache's `cachedFileURL(for:in:)`.
//
//  RELATIVE PATHS ONLY. The stored path is `<itemID>/<name>`, resolved
//  against the CURRENT root at read time: an app container's absolute path
//  changes across reinstalls and OS upgrades, so a persisted absolute URL
//  is a dangling path waiting to happen.
//
//  EXCLUDED FROM BACKUP. A 100 MB video queued on a dead network must not
//  ride into somebody's iCloud backup on its way to being sent.
//

import Foundation
import os

nonisolated enum PendingMediaStaging {

    /// `<Application Support>/FamilyConnect/PendingMedia`, created on first
    /// use and marked so backups skip it.
    static func root() throws -> URL {
        let support = try FileManager.default.url(
            for: .applicationSupportDirectory, in: .userDomainMask,
            appropriateFor: nil, create: true)
        var url = support.appendingPathComponent("FamilyConnect/PendingMedia", isDirectory: true)
        if !FileManager.default.fileExists(atPath: url.path) {
            try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
            var values = URLResourceValues()
            values.isExcludedFromBackup = true
            try? url.setResourceValues(values)
        }
        return url
    }

    /// Resolve a stored relative path. nil when the root cannot be reached
    /// or the file is gone — callers treat a missing file as a send that
    /// can no longer be completed, not as a reason to crash.
    static func url(for relativePath: String) -> URL? {
        guard let root = try? root() else { return nil }
        let url = root.appendingPathComponent(relativePath)
        return FileManager.default.fileExists(atPath: url.path) ? url : nil
    }

    /// What `adopt` recorded: the two relative paths the row stores.
    struct Adopted {
        let fileName: String
        let previewFileName: String?
    }

    /// Take ownership of a prepared item's bytes.
    ///
    /// A MOVE ONLY WHEN THE FILE IS OURS, and that distinction is not a
    /// nicety. `prepareVideo` hands back the ORIGINAL url untouched when
    /// the video already fits the ceiling, and both drag-and-drop and a
    /// share import hand `MediaPrep` a URL the person owns — so a
    /// `Prepared` may be pointing straight at a file in somebody's Movies
    /// folder. Moving that is not staging, it is taking their video away
    /// from them. Anything under the temporary directory was written by
    /// `MediaPrep` for exactly this purpose and is moved; everything else
    /// is copied and left where it was found.
    ///
    /// The in-memory preview JPEG is written out beside it. That is what
    /// lets a poster survive a relaunch; today it exists only for the life
    /// of the `Prepared` value.
    static func adopt(_ prepared: MediaPrep.Prepared, itemID: String) throws -> Adopted {
        let directory = try root().appendingPathComponent(itemID, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)

        let fileName = prepared.fileURL.lastPathComponent
        let destination = directory.appendingPathComponent(fileName)
        if FileManager.default.fileExists(atPath: destination.path) {
            try? FileManager.default.removeItem(at: destination)
        }
        if isOurs(prepared.fileURL) {
            do {
                try FileManager.default.moveItem(at: prepared.fileURL, to: destination)
            } catch {
                // A cross-volume move is a copy anyway.
                try FileManager.default.copyItem(at: prepared.fileURL, to: destination)
                try? FileManager.default.removeItem(at: prepared.fileURL)
            }
        } else {
            try FileManager.default.copyItem(at: prepared.fileURL, to: destination)
        }

        var previewName: String?
        if let jpeg = prepared.previewJPEG {
            let preview = directory.appendingPathComponent("preview.jpg")
            try jpeg.write(to: preview, options: .atomic)
            previewName = "\(itemID)/preview.jpg"
        }
        return Adopted(fileName: "\(itemID)/\(fileName)", previewFileName: previewName)
    }

    /// Whether this file was written by `MediaPrep` for a send, and is
    /// therefore ours to move rather than somebody's own document.
    private static func isOurs(_ url: URL) -> Bool {
        let temporary = FileManager.default.temporaryDirectory
            .standardizedFileURL.path
        return url.standardizedFileURL.path.hasPrefix(temporary)
    }

    /// Everything one item owns, gone.
    static func remove(itemID: String) {
        guard let root = try? root() else { return }
        try? FileManager.default.removeItem(at: root.appendingPathComponent(itemID, isDirectory: true))
    }

    /// Delete every staged directory no row names any more.
    ///
    /// The counterpart of `MediaOutbox.sweepOrphans`, and deliberately NOT
    /// the same sweep: that one runs at launch before anything is open and
    /// deletes every `fc-upload-*` in `tmp` unconditionally, on the sound
    /// reasoning that a just-started process owns none of them. This one
    /// needs the store, so it can only run once the container is open, and
    /// it must never be pointed at `tmp` or vice versa — a staging
    /// directory swept by the launch sweep is a resume that loses its files
    /// on precisely the launch that was supposed to perform it.
    ///
    /// `directory` is a seam: the app sweeps the real root, and a test
    /// sweeps a scratch directory of its own rather than one shared with
    /// every other test running beside it.
    @discardableResult
    static func sweepOrphans(
        keeping itemIDs: Set<String>,
        in directory: URL? = nil,
        youngerThan floor: TimeInterval = 600,
        now: Date = Date()
    ) -> Int {
        guard let root = directory ?? (try? root()),
              let entries = try? FileManager.default.contentsOfDirectory(
                  at: root, includingPropertiesForKeys: [.contentModificationDateKey])
        else { return 0 }
        var removed = 0
        for entry in entries where !itemIDs.contains(entry.lastPathComponent) {
            // An AGE FLOOR as well as the keep-set. A send stages its bytes
            // and THEN writes its rows, so a sweep landing between the two
            // sees files nothing names yet and would delete the message
            // somebody has just pressed Send on. Nothing this young can be
            // an orphan. (In the app this sweep runs once at launch, before
            // any send exists — the floor is what makes it safe to call
            // from anywhere else later.)
            let modified = (try? entry.resourceValues(forKeys: [.contentModificationDateKey]))?
                .contentModificationDate
            if let modified, now.timeIntervalSince(modified) < floor { continue }
            try? FileManager.default.removeItem(at: entry)
            removed += 1
        }
        if removed > 0 {
            AppLog.sync.info("Swept \(removed, privacy: .public) orphaned media staging director(ies)")
        }
        return removed
    }
}
