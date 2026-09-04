//
//  DraggedAttachmentTests.swift
//  FamilyConnectTests
//
//  Dragging an attachment OUT to the Finder (issue #47, the Mac).
//
//  WHAT THESE CANNOT DO, said plainly so nobody reads a green run as
//  proof the feature works: a drag cannot be synthesised. `NSDraggingSession`
//  needs a real pointer, a file promise needs a real receiver to ask for
//  the file, and neither exists in a simulator or on a test runner. That
//  the tile can be picked up, that the Finder accepts the drop, and that
//  the file lands under the name asserted below are all things only a hand
//  on a trackpad against a live server can confirm.
//
//  WHAT THEY DO PIN is the pair of decisions the promise makes before any
//  of that, which are also the pair that would rot silently:
//
//    1. WHAT THE FILE IS CALLED. The receiver takes the name off the
//       promised file's own URL, so the promise's idea of the name and the
//       name `ChatSyncCoordinator.localFileURL` writes have to be the same
//       string — asserted here against that function's own output rather
//       than against a literal, so a change to either breaks this.
//
//    2. WHERE THE BYTES MAY COME FROM. The trap the whole feature is built
//       around: what a thread has on disk is almost always
//       `AttachmentStore`'s 600px PREVIEW under a `.jpg` name, and handing
//       that over is worse than handing over nothing — it opens fine and
//       is simply the wrong picture. `refusesAPreviewInPlaceOfTheOriginal`
//       is the regression test for exactly that, and the one test here
//       worth keeping if every other one were deleted.
//

import CoreTransferable
import Foundation
import Testing
@testable import FamilyConnect

/// Counts what the promise did, from inside the `@MainActor` closures it
/// is handed. A class rather than captured locals because the closures are
/// `@Sendable` and outlive the statement that made them.
@MainActor
private final class DragRecorder {
    var downloads = 0
    var failures = 0
}

@MainActor
@Suite("Drag an attachment out")
struct DraggedAttachmentTests {

    // MARK: - Fixtures

    /// Decoded from wire JSON rather than built with the memberwise init,
    /// like the rest of the attachment tests: the fields a drag reads
    /// (`kind`, `mime`, `name`) are wire fields, and a fixture that cannot
    /// come off the wire proves nothing about one that did.
    private func attachment(_ json: String) throws -> AttachmentDTO {
        try JSONDecoder().decode(AttachmentDTO.self, from: Data(json.utf8))
    }

    private func photo(id: Int64 = 34) throws -> AttachmentDTO {
        try attachment(
            #"{"id": \#(id), "kind": "photo", "mime": "image/jpeg", "size": 900, "has_preview": true}"#)
    }

    private func video(id: Int64 = 7) throws -> AttachmentDTO {
        try attachment(
            #"{"id": \#(id), "kind": "video", "mime": "video/quicktime", "size": 90, "has_preview": true}"#)
    }

    private func file(id: Int64 = 40, name: String = "Invoice 2026.pdf") throws -> AttachmentDTO {
        try attachment(
            #"{"id": \#(id), "kind": "file", "mime": "application/pdf", "size": 1536, "has_preview": false, "name": "\#(name)"}"#)
    }

    private func audio(id: Int64 = 12) throws -> AttachmentDTO {
        try attachment(
            #"{"id": \#(id), "kind": "audio", "mime": "audio/mp4", "size": 400, "has_preview": false}"#)
    }

    private func location(id: Int64 = 61) throws -> AttachmentDTO {
        try attachment(
            #"{"id": \#(id), "kind": "location", "mime": "application/json", "size": 0, "has_preview": false, "latitude": 55.7558, "longitude": 37.6173}"#)
    }

    /// A private caches root per case, so one case's files cannot answer
    /// another's — the seam `AttachmentStore`'s injected `directory` is.
    private func cachesRoot() -> URL {
        FileManager.default.temporaryDirectory
            .appendingPathComponent("fc-drag-\(UUID().uuidString)", isDirectory: true)
    }

    /// Exactly where `ChatSyncCoordinator.localFileURL` would put the
    /// bytes. Every "good" URL in this file comes from here rather than
    /// from a hand-written path, so the tests move with the layout.
    private func original(of attachment: AttachmentDTO, in caches: URL) -> URL {
        ChatSyncCoordinator.cachedFileURL(for: attachment, in: caches)
    }

    /// A promise wired to answer with `url` (or nothing), recording both.
    private func promise(
        for attachment: AttachmentDTO,
        downloads url: URL?,
        into recorder: DragRecorder
    ) -> DraggedAttachment {
        DraggedAttachment(
            attachment: attachment,
            download: {
                recorder.downloads += 1
                return url
            },
            onFailure: { recorder.failures += 1 })
    }

    // MARK: - What the file is called

    /// A file's name is its whole identity, and it survives the round trip
    /// untouched — including the spaces, which nothing here may "clean".
    @Test("A file is promised under its own name")
    func fileKeepsItsName() throws {
        let attachment = try file()
        #expect(DraggedAttachment.suggestedFileName(for: attachment) == "Invoice 2026.pdf")
    }

    /// A photo and a video carry no name, so the promise falls back — and
    /// the EXTENSION is the part that matters, because it is the only thing
    /// telling the Finder (and whatever opens the file next) what it is.
    /// `.data` is all the drag itself advertises; see DraggedAttachment.
    @Test("A nameless photo or video is promised with the right extension")
    func mediaGetsAnExtension() throws {
        #expect(DraggedAttachment.suggestedFileName(for: try photo()) == "photo-34.jpg")
        #expect(DraggedAttachment.suggestedFileName(for: try video()) == "video-7.mov")
    }

    /// The name comes off the wire, and a drag puts it on somebody else's
    /// filesystem. Same treatment `localFileURL` gives it — because it IS
    /// that function's, not a second copy of the rule.
    @Test("A hostile filename cannot become a path in a drop")
    func hostileNameIsSanitised() throws {
        let attachment = try file(name: "../../etc/passwd")
        #expect(DraggedAttachment.suggestedFileName(for: attachment) == "_.._etc_passwd")
    }

    /// THE point of asserting the name at all: the receiver reads it off
    /// the promised file's URL, so the promise and the download have to
    /// agree on it. Two spellings of the name is how a dragged photo
    /// arrives called something Save… would never have called it.
    @Test("The promised name is the name the download writes")
    func nameMatchesWhatIsWrittenToDisk() throws {
        let caches = cachesRoot()
        for attachment in [try photo(), try video(), try file(), try audio()] {
            #expect(
                original(of: attachment, in: caches).lastPathComponent
                    == DraggedAttachment.suggestedFileName(for: attachment))
        }
    }

    // MARK: - What may be dragged

    /// Photo, video and file have original bytes on the server and a tile
    /// that is nothing but a picture or a row.
    @Test("Photos, videos and files can be dragged")
    func mediaAndFilesAreDraggable() throws {
        #expect(DraggedAttachment.isDraggable(try photo()))
        #expect(DraggedAttachment.isDraggable(try video()))
        #expect(DraggedAttachment.isDraggable(try file()))
    }

    /// The protocol's answer, not a preference: a location is the only kind
    /// with no bytes, and `GET /attachments/{id}` on one is
    /// `invalid_attachment` (docs/protocol.md, "Locations"). A drag that
    /// could only ever fail is worse than no drag.
    @Test("A location has no bytes, so it is never draggable")
    func locationIsNotDraggable() throws {
        #expect(!DraggedAttachment.isDraggable(try location()))
    }

    /// A gesture decision rather than a protocol one: a voice note's
    /// rectangle IS a scrubber, and a drag source in the same rectangle
    /// fights the slider for the same pointer movement.
    @Test("A voice note is not draggable, because its tile is a scrubber")
    func audioIsNotDraggable() throws {
        #expect(!DraggedAttachment.isDraggable(try audio()))
    }

    // MARK: - Where the bytes may come from

    /// The URL `localFileURL` produces is the one acceptable source.
    @Test("The coordinator's own file is accepted")
    func acceptsTheDownloadedOriginal() throws {
        let attachment = try photo()
        let caches = cachesRoot()
        #expect(DraggedAttachment.isOriginal(original(of: attachment, in: caches), of: attachment))
    }

    /// THE TRAP, written down. `AttachmentStore` keeps both of these, both
    /// under `.jpg` whatever the real type, and the preview-first bubble
    /// means the one a scrolled thread actually holds is the 600px preview.
    /// Neither may ever be handed to a drag.
    @Test("Neither preview-cache spelling can pass as an original")
    func rejectsTheAttachmentCache() throws {
        let attachment = try photo(id: 34)
        let cache = cachesRoot().appendingPathComponent("attachments", isDirectory: true)
        #expect(!DraggedAttachment.isOriginal(
            cache.appendingPathComponent("34.jpg"), of: attachment))
        #expect(!DraggedAttachment.isOriginal(
            cache.appendingPathComponent("34-preview.jpg"), of: attachment))
    }

    /// A video is the case that would hurt most and the case the flat cache
    /// cannot even hold: `PlatformImage.decode` only ever succeeds for the
    /// POSTER, so `7.jpg` beside a video is a still frame, and dropping it
    /// on a Desktop would produce a photograph where a film was asked for.
    @Test("A video's poster cannot pass as the video")
    func rejectsAPosterForAVideo() throws {
        let attachment = try video(id: 7)
        let cache = cachesRoot().appendingPathComponent("attachments", isDirectory: true)
        #expect(!DraggedAttachment.isOriginal(
            cache.appendingPathComponent("7.jpg"), of: attachment))
        #expect(!DraggedAttachment.isOriginal(
            cache.appendingPathComponent("7-preview.jpg"), of: attachment))
    }

    /// The right name in the wrong place is still the wrong place: the
    /// per-attachment id directory is what the flat preview cache has no
    /// equivalent of, and it is half the shape.
    @Test("The right name outside its id directory is refused")
    func rejectsTheRightNameInTheWrongDirectory() throws {
        let attachment = try file()
        let loose = cachesRoot().appendingPathComponent("Invoice 2026.pdf")
        #expect(!DraggedAttachment.isOriginal(loose, of: attachment))
    }

    /// And one attachment's file is not another's, even in the right shape.
    @Test("Another attachment's file is refused")
    func rejectsAnotherAttachmentsFile() throws {
        let caches = cachesRoot()
        let mine = try photo(id: 34)
        let theirs = try photo(id: 35)
        #expect(!DraggedAttachment.isOriginal(original(of: theirs, in: caches), of: mine))
    }

    // MARK: - The promise itself

    /// The happy path: the original, copied rather than lent, under its
    /// real name.
    @Test("A promise hands over the downloaded original")
    func promisesTheOriginal() async throws {
        let attachment = try file()
        let caches = cachesRoot()
        let recorder = DragRecorder()
        let sent = try await promise(
            for: attachment, downloads: original(of: attachment, in: caches), into: recorder
        ).promisedFile()

        #expect(sent.file.lastPathComponent == "Invoice 2026.pdf")
        // Copied, never lent: the source is in a *caches* directory the OS
        // may purge and `AttachmentStore.forget` deletes on a retention
        // sweep, so a handle into it can go out from under the receiver.
        #expect(!sent.allowAccessingOriginalFile)
        #expect(recorder.failures == 0)
    }

    /// Offline, a 404, and an attachment retention has already swept all
    /// arrive here as the same nil — `localFileURL` cannot tell them apart
    /// and neither can anyone reading the notice. What matters is that all
    /// three END the drag rather than completing it with something else,
    /// and that the window is told: a thrown promise is silent, and a
    /// silent failure looks exactly like a drag that never took hold.
    @Test("A download that fails ends the drag and says so")
    func failedDownloadEndsTheDrag() async throws {
        let recorder = DragRecorder()
        let subject = promise(for: try photo(), downloads: nil, into: recorder)

        await #expect(throws: DraggedAttachment.Failure.unavailable) {
            try await subject.promisedFile()
        }
        #expect(recorder.downloads == 1)
        #expect(recorder.failures == 1)
    }

    /// A drag somebody called off must not shout about it. Cancelling the
    /// export task is how an abandoned drag arrives here, and a cancelled
    /// URLSession request answers nil through `localFileURL` exactly as a
    /// 404 does — so without the cancellation check the composer would
    /// grow a red error about a drag the reader deliberately gave up on.
    ///
    /// Deterministic despite looking like a race: the suite is `@MainActor`,
    /// so the task's body cannot start until this function suspends, and
    /// `cancel()` is called before it does.
    @Test("A drag that is called off fails silently")
    func cancelledDragSaysNothing() async throws {
        let recorder = DragRecorder()
        let subject = promise(for: try photo(), downloads: nil, into: recorder)

        let task = Task { try await subject.promisedFile() }
        task.cancel()

        await #expect(throws: CancellationError.self) { try await task.value }
        // The download was attempted and came back empty — so this really
        // is the cancellation check talking, not an earlier bail out.
        #expect(recorder.downloads == 1)
        #expect(recorder.failures == 0)
    }

    /// THE regression test. If a future edit swaps the download for
    /// something that answers with what the tile is DRAWING — which is what
    /// `AttachmentStore` holds, which is the preview — the promise refuses
    /// it rather than dropping a thumbnail on somebody's Desktop wearing
    /// the photograph's name. Nobody would ever notice that as a bug.
    @Test("A preview is refused even when the download offers one")
    func refusesAPreviewInPlaceOfTheOriginal() async throws {
        let attachment = try photo(id: 34)
        let preview = cachesRoot()
            .appendingPathComponent("attachments", isDirectory: true)
            .appendingPathComponent("34-preview.jpg")
        let recorder = DragRecorder()
        let subject = promise(for: attachment, downloads: preview, into: recorder)

        await #expect(throws: DraggedAttachment.Failure.notTheOriginal) {
            try await subject.promisedFile()
        }
        // Refused, and said out loud — not quietly downgraded.
        #expect(recorder.failures == 1)
    }

    /// The view never offers these, and the promise refuses them anyway —
    /// without a download, and without a notice, because nobody asked for
    /// this and there is nothing for a reader to do about it.
    @Test("A location or a voice note is refused before any download")
    func refusesWhatHasNoTileToDragFrom() async throws {
        for attachment in [try location(), try audio()] {
            let recorder = DragRecorder()
            let caches = cachesRoot()
            let subject = promise(
                for: attachment, downloads: original(of: attachment, in: caches), into: recorder)

            await #expect(throws: DraggedAttachment.Failure.notDraggable) {
                try await subject.promisedFile()
            }
            #expect(recorder.downloads == 0)
            #expect(recorder.failures == 0)
        }
    }
}
