//
//  DraggedAttachment.swift
//  FamilyConnect
//
//  Dragging an attachment OUT of a conversation to the Finder — the Mac's
//  other half of drag and drop, and the mirror of `DroppedAttachment`.
//
//  NOTHING NEW GOES OVER THE WIRE. A drag out is `GET /attachments/{id}`,
//  the same request the viewer's Save… already makes, through the same
//  `ChatSyncCoordinator.localFileURL(for:)`. docs/protocol.md needs no
//  amendment for this and got none.
//
//  THE TRAP — why this is a file PROMISE and not `.draggable(someURL)`:
//
//  A thread almost never holds an attachment's real bytes on disk under
//  its real name. `AttachmentStore` writes `caches/attachments/<id>.jpg`
//  and `<id>-preview.jpg` — ALWAYS with a `.jpg` extension whatever the
//  real type, and only when `PlatformImage.decode` succeeds, so a video, a
//  voice note and a PDF are never in that cache at all. A bubble asks
//  preview-first (`store.image(id:preview:true) ?? …(preview:false)`), so
//  what a scrolled thread has actually cached is the 600px PREVIEW.
//
//  The ONE thing that puts original bytes on disk under their real name is
//  `ChatSyncCoordinator.localFileURL(for:)` (`caches/files/<id>/<name>`),
//  and until now exactly three places called it: the Mac viewer's Save…
//  and Share…, and opening a file row.
//
//  So a URL handed straight to `.draggable` would either drop a THUMBNAIL
//  into the Finder wearing the photograph's name — silent, and worse than
//  a broken file, because it opens fine and is simply the wrong picture —
//  or work only for whoever happened to open the viewer first. Hence a
//  promise: the drag starts with no bytes anywhere, the download runs
//  while the pointer is still moving, and the receiver gets the original
//  or it gets nothing.
//
//  "OR IT GETS NOTHING" IS THE INVARIANT. Every failure here ends in a
//  throw. A preview is never substituted, an empty file is never written,
//  and `isOriginal` re-checks the URL's shape even though the only caller
//  that produces one is `localFileURL` — because the cost of being wrong
//  is a family photo silently replaced by its thumbnail on someone's
//  Desktop, which nobody would ever notice was a bug.
//
//  Split out of the Mac views for the reason `DroppedAttachment` is: the
//  parts worth pinning are pure rules over an `AttachmentDTO` and a URL,
//  and a drag cannot be synthesised — `NSDraggingSession` needs a real
//  pointer, and a file promise needs a real receiver on the other end of
//  it. What a test can hold is the promise's two decisions: what the file
//  is CALLED, and where its bytes may come from. Both live here.
//
//  Android counterpart: none. Android's chat has no drag source.
//

import CoreTransferable
import Foundation
import UniformTypeIdentifiers
import os

/// One attachment, offered to the rest of the Mac as a file that does not
/// exist yet.
///
/// The two closures are injected rather than reached for so this type
/// carries no view and no coordinator of its own: it is what lets a test
/// answer "the download failed" or "the download handed back a preview"
/// without a server, and it keeps the promise honest about being pure
/// policy over whatever bytes someone else produced.
nonisolated struct DraggedAttachment: Transferable {
    let attachment: AttachmentDTO

    /// Puts the ORIGINAL bytes on disk under their real name and answers
    /// with where, or answers nil. In the app this is
    /// `ChatSyncCoordinator.localFileURL(for:)` and must stay so — see
    /// `isOriginal`, which refuses anything that does not look like its
    /// output.
    ///
    /// `@MainActor` because that is where the coordinator lives; the
    /// download itself is already off the main actor inside `APIClient`,
    /// exactly as it is for the viewer's Save… and Share….
    let download: @MainActor @Sendable () async -> URL?

    /// Said out loud when there is nothing to hand over. A thrown promise
    /// is SILENT — the drag simply springs back, which is indistinguishable
    /// from a drag that never started — so the window is told and shows it
    /// where every other media failure on that window appears.
    let onFailure: @MainActor @Sendable () -> Void

    /// Why a promise came back empty. Not shown to anybody: the user-facing
    /// half is `onFailure`, and these exist so a log line and a test can say
    /// which of the three it was.
    enum Failure: Error, Equatable {
        /// The view offered a drag for something that has no bytes at all.
        /// A programming error, not a runtime condition.
        case notDraggable
        /// Offline, a 404, or an attachment retention has already swept.
        /// The three are deliberately one case: `localFileURL` cannot tell
        /// them apart either (a 404 answers nil and a transport failure
        /// answers nil), and all three mean the same thing to somebody with
        /// a photo half-way to their Desktop.
        case unavailable
        /// The bytes handed over were not the original's. Unreachable
        /// unless someone wires a second byte source in; see the header.
        case notTheOriginal
    }

    // MARK: - The rules

    /// Which attachments a tile offers to the Finder at all.
    ///
    /// A LOCATION never, and that one is the protocol's answer rather than
    /// a preference: `kind=location` is the only kind with no bytes, and
    /// `GET /attachments/{id}` on one is `invalid_attachment` (400)
    /// — docs/protocol.md, "Locations". A promise for it could only ever
    /// fail, and offering a drag that cannot succeed is worse than not
    /// offering one.
    ///
    /// AUDIO never, and that one is a gesture decision: a voice note's
    /// rectangle IS a scrubber — `AudioPlayerView` draws a `Slider` across
    /// the whole of it — and a drag source and a slider in the same
    /// rectangle fight over the same pointer movement. Losing the scrubber
    /// to buy a drag is a bad trade for the one attachment kind whose tile
    /// is a control.
    ///
    /// Everything else — photo, video, file — has original bytes on the
    /// server and a tile that does nothing but sit there.
    static func isDraggable(_ attachment: AttachmentDTO) -> Bool {
        !attachment.isLocation && !attachment.isAudio
    }

    /// What the dropped file is CALLED.
    ///
    /// `ChatSyncCoordinator.cachedFileName(for:)` and not a second spelling
    /// of the same idea, because the receiver takes the name off the
    /// promised file itself: a `SentTransferredFile` carries a URL, and its
    /// last path component is what lands in the Finder. One function, so a
    /// dragged photo cannot arrive called "34.jpg" while Save… writes
    /// "photo-34.jpg" for the same picture.
    static func suggestedFileName(for attachment: AttachmentDTO) -> String {
        ChatSyncCoordinator.cachedFileName(for: attachment)
    }

    /// True only for a URL of the exact shape `localFileURL` writes: the
    /// attachment's own id directory, holding a file under the attachment's
    /// real name.
    ///
    /// Written as a SHAPE rather than as "not the preview cache" because
    /// `AttachmentStore`'s directory is an injectable test seam and can be
    /// anywhere, while the one thing every legitimate source has is this
    /// shape. Neither of the store's two spellings can pass it: `<id>.jpg`
    /// and `<id>-preview.jpg` sit directly in one flat directory, so
    /// neither has an id directory above it, and neither is named what an
    /// attachment is actually called.
    static func isOriginal(_ url: URL, of attachment: AttachmentDTO) -> Bool {
        url.lastPathComponent == suggestedFileName(for: attachment)
            && url.deletingLastPathComponent().lastPathComponent == String(attachment.id)
    }

    // MARK: - The promise

    /// The body of the file promise, as a named method so a test can await
    /// it without a drag. Throws rather than returning an optional: a
    /// `FileRepresentation` has no other way to say "nothing", and
    /// "nothing" is the only acceptable alternative to the original.
    func promisedFile() async throws -> SentTransferredFile {
        guard Self.isDraggable(attachment) else {
            // Not `onFailure`: nobody asked for this, a view offered a drag
            // it should not have. Log it and stay quiet on screen.
            AppLog.ui.info(
                "Drag refused for attachment \(attachment.id, privacy: .public) of kind \(attachment.kind, privacy: .public)")
            throw Failure.notDraggable
        }
        guard let url = await download() else {
            // A drag somebody CALLED OFF cancels this task, and a cancelled
            // URLSession request answers nil through `localFileURL` exactly
            // as a 404 does. Reporting that would put a red error above the
            // composer for a drag the reader deliberately abandoned — an
            // error about nothing, which is how a notice strip stops being
            // read. `checkCancellation` throws before `onFailure`, so the
            // abandoned case ends the way it should: silently.
            try Task.checkCancellation()
            await onFailure()
            throw Failure.unavailable
        }
        guard Self.isOriginal(url, of: attachment) else {
            AppLog.ui.error(
                "Drag source for attachment \(attachment.id, privacy: .public) was not the original; refusing")
            await onFailure()
            throw Failure.notTheOriginal
        }
        // COPIED, not lent (`allowAccessingOriginalFile: false`). This file
        // is in a CACHES directory, which the system may reclaim whenever
        // it likes and without asking — so a handle into it is a handle
        // into something that can go away mid-copy, and lending one also
        // lets a receiver that moves rather than copies take this device's
        // only local copy with it. The framework makes its own copy first,
        // keeping the name.
        return SentTransferredFile(url, allowAccessingOriginalFile: false)
    }

    static var transferRepresentation: some TransferRepresentation {
        // `.data`, and not the attachment's own type, because a
        // `TransferRepresentation` is STATIC — one list for the whole type
        // — while an attachment's real type is per instance: this same
        // struct carries a JPEG, an MP4 and a PDF. `public.data` is the one
        // thing all three truthfully are, and the Finder accepts it.
        // Declaring `.image` here to please a photo would be a lie about
        // every PDF, and a lie a receiver acts on: it would accept a drop
        // it cannot use.
        //
        // The real type is not lost by saying so. It rides on the file's
        // EXTENSION, which is what `ChatSyncCoordinator.fallbackName`
        // exists to get right — the same mechanism that already makes
        // Save… and Share… hand on something openable.
        FileRepresentation(exportedContentType: .data) { dragged in
            try await dragged.promisedFile()
        }
    }
}
