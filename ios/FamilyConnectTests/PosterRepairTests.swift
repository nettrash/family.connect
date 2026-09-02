//
//  PosterRepairTests.swift
//  FamilyConnectTests
//
//  A video's poster is uploaded in its own request, best-effort, so that
//  a thumbnail never costs the send. Until issue #54 "best-effort" meant
//  EXACTLY ONCE: if that one `PUT /attachments/{id}/preview` failed, the
//  server kept `has_preview = false` and nothing ever repaired it — not a
//  relaunch, not a resync, not re-opening the chat. The sender saw nothing
//  wrong, because its own bubble draws from the bytes it seeded into the
//  cache; every recipient got a grey tile with a play badge, for good.
//  That asymmetry is the whole of the "video from the opponent is not
//  showing" report (#38), and the half #38 fixed — treating `has_preview`
//  as a hint — cannot conjure a poster that was never uploaded.
//
//  These pin the second half. The device that made the poster still holds
//  it, so it finishes the job on the next connect; and the cases where it
//  CANNOT finish the job SETTLE rather than retry for ever:
//
//    - nothing held locally  → no request at all, ever
//    - the server refuses    → a fixed budget of ANSWERED attempts, then
//                              the video is given up on for good
//    - nobody answered       → nothing learned, nothing spent
//    - the session is gone   → the pass stops instead of burning every
//                              waiting poster's budget on one dead token
//
//  Android counterpart: app/src/test/java/me/nettrash/familyconnect/
//  data/repo/PosterRepairTest.kt
//

import Foundation
import Testing
@testable import FamilyConnect

@MainActor
struct PosterRepairTests {

    /// Each case gets its own empty directory, so one case's marker can
    /// never be found by another's repair pass.
    private func makeStore(host: String?) -> (AttachmentStore, URL) {
        let directory = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("poster-repair-\(UUID().uuidString)")
        let api = APIClient(
            serverURL: host.flatMap { URL(string: "https://\($0)") },
            session: StubURLProtocol.makeSession())
        return (AttachmentStore(api: api, directory: directory), directory)
    }

    private func previewPuts(host: String) -> [RecordedRequest] {
        StubURLProtocol.requests(host: host)
            .filter { $0.method == "PUT" && $0.url.path.hasSuffix("/preview") }
    }

    /// What the send path holds when it reaches the preview upload: real
    /// JPEG bytes, because `seed` refuses anything it cannot decode.
    private let poster = TestImages.photograph(width: 48, height: 32)

    /// Put a store in the state a failed poster upload leaves behind: the
    /// bytes seeded for this device's own bubble, and a note that the
    /// server never got them.
    private func stageFailedUpload(_ store: AttachmentStore, id: Int64) {
        store.seed(poster, id: id, preview: true)
        store.notePosterUpload(id: id, landed: false)
    }

    // MARK: - The repair

    /// THE REGRESSION. One failed `uploadPreview` used to be the end of
    /// it. The bytes never left this device, and this device is the only
    /// one that has them.
    @Test("a poster the server never got is re-sent on the next connect")
    func aFailedPosterIsResent() async throws {
        let host = "poster-repair-resent.test"
        StubURLProtocol.register(host: host) { _ in .empty(204) }
        defer { StubURLProtocol.unregister(host: host) }

        let (store, _) = makeStore(host: host)
        stageFailedUpload(store, id: 70)
        #expect(store.isPosterUnsent(id: 70))

        await store.repairPosters()?.value

        let puts = previewPuts(host: host)
        #expect(puts.count == 1, "the poster was never re-sent")
        #expect(puts.first?.url.path.hasSuffix("/attachments/70/preview") == true)
        #expect(puts.first?.body == poster, "something other than the poster went up")
        #expect(!store.isPosterUnsent(id: 70),
                "a repaired poster must not be sent again on every reconnect")
    }

    /// The pass costs nothing at all when there is nothing owed. This is
    /// the state every normal send leaves: the upload landed.
    @Test("a poster that landed leaves nothing to repair")
    func aLandedPosterIsNotResent() async throws {
        let host = "poster-repair-landed.test"
        StubURLProtocol.register(host: host) { _ in .empty(204) }
        defer { StubURLProtocol.unregister(host: host) }

        let (store, _) = makeStore(host: host)
        store.seed(poster, id: 71, preview: true)
        store.notePosterUpload(id: 71, landed: true)

        await store.repairPosters()?.value

        #expect(previewPuts(host: host).isEmpty,
                "a poster the server already has was uploaded again")
        #expect(!store.isPosterUnsent(id: 71))
    }

    /// A retry that succeeded, then a second connect: the marker is gone,
    /// so the pass does not walk the same id again for the life of the
    /// install.
    @Test("a repaired poster is not re-sent on the connect after it")
    func aRepairedPosterSettles() async throws {
        let host = "poster-repair-settles-ok.test"
        StubURLProtocol.register(host: host) { _ in .empty(204) }
        defer { StubURLProtocol.unregister(host: host) }

        let (store, _) = makeStore(host: host)
        stageFailedUpload(store, id: 72)

        await store.repairPosters()?.value
        await store.repairPosters()?.value
        await store.repairPosters()?.value

        #expect(previewPuts(host: host).count == 1)
    }

    // MARK: - Settling when it cannot be repaired

    /// The frame grab returned nil, or the caches were purged: there are
    /// no pixels on this device and no number of tries produces any. It
    /// must cost ZERO requests — not a bounded few, none — and the note
    /// asking for them must go, or every connect re-reads it.
    @Test("a poster this device no longer holds is never chased")
    func aPosterWithNoBytesIsNotChased() async throws {
        let host = "poster-repair-no-bytes.test"
        StubURLProtocol.register(host: host) { _ in .empty(204) }
        defer { StubURLProtocol.unregister(host: host) }

        let (store, directory) = makeStore(host: host)
        stageFailedUpload(store, id: 73)
        // What a caches purge does: the bytes go, the note stays.
        try FileManager.default.removeItem(
            at: directory.appendingPathComponent("73-preview.jpg"))

        await store.repairPosters()?.value

        #expect(previewPuts(host: host).isEmpty,
                "an empty body was uploaded as a poster")
        #expect(!store.isPosterUnsent(id: 73),
                "a poster with no bytes behind it is re-read on every connect")
    }

    /// THE SETTLE. A server that keeps refusing — out of disk, say — must
    /// not be asked for ever. The budget is spent in ANSWERED attempts and
    /// then the video is abandoned: it keeps its play badge over a plain
    /// placeholder, exactly as a video whose poster genuinely never
    /// existed does on the read side.
    @Test("a poster the server keeps refusing is given up on for good")
    func aRefusedPosterIsGivenUpOn() async throws {
        let host = "poster-repair-refused.test"
        StubURLProtocol.register(host: host) { _ in .empty(500) }
        defer { StubURLProtocol.unregister(host: host) }

        let (store, _) = makeStore(host: host)
        stageFailedUpload(store, id: 74)

        // One pass per connect. Well past the budget.
        for _ in 0..<(AttachmentStore.posterRepairAttempts + 4) {
            await store.repairPosters()?.value
        }

        #expect(previewPuts(host: host).count == AttachmentStore.posterRepairAttempts,
                "the repair either gave up early or is retrying for ever")
        #expect(!store.isPosterUnsent(id: 74),
                "an abandoned poster is still queued and costs a directory read")
    }

    /// The counterpart rule, and the one that keeps the budget honest:
    /// nobody answered, so nothing was learned. Spending an attempt here
    /// is what would burn a perfectly repairable poster's whole budget
    /// during an outage — the same mistake `finish(_:data:settled:)`
    /// refuses to make on the read side.
    @Test("a request nobody answered does not spend the budget")
    func aTransportFailureSpendsNothing() async throws {
        // No server configured: the request cannot leave, and the client
        // says so without a status. Same shape as a refused connection.
        let (store, _) = makeStore(host: nil)
        stageFailedUpload(store, id: 75)

        for _ in 0..<(AttachmentStore.posterRepairAttempts + 4) {
            await store.repairPosters()?.value
        }

        #expect(store.isPosterUnsent(id: 75),
                "an outage was allowed to spend the repair budget")
    }

    /// A dead session fails every waiting poster identically, so the pass
    /// stops on the first one rather than spending every marker's budget
    /// on the same 401.
    @Test("a dead session stops the pass instead of burning every budget")
    func aDeadSessionStopsThePass() async throws {
        let host = "poster-repair-401.test"
        StubURLProtocol.register(host: host) { _ in .empty(401) }
        defer { StubURLProtocol.unregister(host: host) }

        let (store, _) = makeStore(host: host)
        stageFailedUpload(store, id: 76)
        stageFailedUpload(store, id: 77)

        await store.repairPosters()?.value

        #expect(previewPuts(host: host).count == 1,
                "the pass kept going after the session turned out to be gone")
        #expect(store.isPosterUnsent(id: 76) && store.isPosterUnsent(id: 77),
                "a 401 discarded posters that a fresh sign-in could still send")
    }

    /// An attachment the server does not have is as terminal as an answer
    /// gets — swept, or never ours to replace. No budget, no second look.
    @Test("a poster for an attachment the server has never seen is dropped")
    func anUnknownAttachmentIsDropped() async throws {
        let host = "poster-repair-404.test"
        StubURLProtocol.register(host: host) { _ in .empty(404) }
        defer { StubURLProtocol.unregister(host: host) }

        let (store, _) = makeStore(host: host)
        stageFailedUpload(store, id: 78)

        await store.repairPosters()?.value
        await store.repairPosters()?.value

        #expect(previewPuts(host: host).count == 1)
        #expect(!store.isPosterUnsent(id: 78))
    }

    // MARK: - Housekeeping

    /// The chat vanished (a peer deleted their account), so the poster
    /// went with it. A marker left pointing at bytes that are gone would
    /// be read on every connect and repair nothing.
    @Test("forgetting an attachment forgets what it still owed the server")
    func forgettingClearsTheMarker() async throws {
        let host = "poster-repair-forget.test"
        StubURLProtocol.register(host: host) { _ in .empty(204) }
        defer { StubURLProtocol.unregister(host: host) }

        let (store, _) = makeStore(host: host)
        stageFailedUpload(store, id: 79)
        store.forget(attachmentIDs: [79])

        #expect(!store.isPosterUnsent(id: 79))
        await store.repairPosters()?.value
        #expect(previewPuts(host: host).isEmpty)
    }

    /// Logout takes the whole directory, markers included: the next
    /// account must not push the previous one's frames anywhere.
    @Test("logout drops what was still owed")
    func clearDropsTheMarker() async throws {
        let host = "poster-repair-clear.test"
        StubURLProtocol.register(host: host) { _ in .empty(204) }
        defer { StubURLProtocol.unregister(host: host) }

        let (store, _) = makeStore(host: host)
        stageFailedUpload(store, id: 80)
        store.clear()

        #expect(!store.isPosterUnsent(id: 80))
        await store.repairPosters()?.value
        #expect(previewPuts(host: host).isEmpty)
    }
}
