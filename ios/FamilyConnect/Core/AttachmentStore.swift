//
//  AttachmentStore.swift
//  FamilyConnect
//
//  Previews and full photos, fetched once and kept.
//
//  ON DISK, not in memory — the difference from AvatarStore, and the whole
//  reason this is a separate type. A family album is hundreds of photos; a
//  memory cache would either evict constantly or grow until the app is
//  jettisoned. The bytes are immutable (an attachment's content never
//  changes), so a file cached under its id is valid forever and survives
//  relaunches, which is what makes scrolling back through a year of photos
//  feel instant instead of re-downloading.
//
//  Videos are NOT cached here: they stream straight from the server through
//  AVPlayer, which does its own buffering and would gain nothing from a
//  second copy on the phone.
//
//  A VIDEO'S POSTER IS THE ONE THING HERE THE SERVER MIGHT NOT HAVE.
//  Everything else in this cache is a copy of bytes the server already
//  holds. A poster is made on this device and pushed up in its own request
//  (`ChatSyncCoordinator.performSendMedia`), and that request can fail
//  while the send itself succeeds — which used to mean no poster for
//  anybody, ever (issue #54). So this store also keeps the small amount of
//  bookkeeping needed to finish the job later: see "Repairing a poster the
//  server never got".
//
//  Android counterpart: data/repo/AttachmentRepository.kt
//

import SwiftUI
import os

@MainActor
@Observable
final class AttachmentStore {

    private let api: APIClient
    private let directory: URL
    /// Bumped when a fetch lands, so views drawing one re-render.
    private(set) var generation = 0
    /// In-flight ids, so N bubbles of the same photo fetch once.
    @ObservationIgnored private var inFlight: Set<String> = []
    /// Ids the server has nothing for; not retried.
    @ObservationIgnored private var missing: Set<String> = []
    /// How long to wait before asking again for a video poster the server
    /// did not have, and — by its length — how many times.
    ///
    /// Short and finite on purpose. A poster is uploaded just before the
    /// message that claims it (`ChatSyncCoordinator.sendMedia`), so the
    /// window in which a reader can be told "no" and be wrong is seconds
    /// wide. After the last rung the key settles like any other 404, which
    /// is the half that matters most: a video whose poster never made it —
    /// the frame grab failed, or its upload did — costs three small
    /// requests once per launch and then nothing at all.
    ///
    /// A test seam like `directory`: a test that has to watch the ladder
    /// run cannot spend twelve seconds doing it. Matches Android's
    /// `AttachmentRepository.POSTER_RETRY_DELAYS_MS`.
    private let posterRetryDelays: [Duration]
    /// Small hot cache in front of the disk, so a visible bubble does not
    /// re-read and re-decode on every scroll frame.
    @ObservationIgnored private var hot: [String: Image] = [:]
    @ObservationIgnored private var hotOrder: [String] = []

    private static let hotLimit = 40

    /// Longest edge kept decoded. A bubble draws 240pt and the viewer a
    /// screen width, so the uploaded 2048 would be several times the
    /// pixels anyone sees. Matches Android's DISPLAY_PIXELS.
    static let displayPixels = 1440

    /// `directory` is a test seam, in the same spirit as `APIClient`'s
    /// injected `URLSession`: the app passes nothing and gets the caches
    /// folder, while a test gives each case its own empty directory so one
    /// case's cached bytes cannot answer another's fetch.
    init(
        api: APIClient,
        directory: URL? = nil,
        posterRetryDelays: [Duration] = [.seconds(2), .seconds(10)]
    ) {
        self.api = api
        self.posterRetryDelays = posterRetryDelays
        if let directory {
            self.directory = directory
        } else {
            let caches = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
            self.directory = caches.appendingPathComponent("attachments", isDirectory: true)
        }
        try? FileManager.default.createDirectory(at: self.directory, withIntermediateDirectories: true)
    }

    private func key(_ id: Int64, preview: Bool) -> String {
        preview ? "\(id)-preview" : "\(id)"
    }

    private func fileURL(_ key: String) -> URL {
        directory.appendingPathComponent(key).appendingPathExtension("jpg")
    }

    /// Where the "this poster never reached the server" marker for an
    /// attachment lives: beside its bytes, under the same id.
    private func posterMarkerURL(_ id: Int64) -> URL {
        directory.appendingPathComponent(key(id, preview: true))
            .appendingPathExtension(Self.posterMarkerExtension)
    }

    /// The image if it is already here, else nil — and a fetch is started.
    /// Safe to call from a view body.
    ///
    /// - Parameter mayArriveLate: whether the server answering "no" is
    ///   necessarily the end of it. True for a VIDEO POSTER and nothing
    ///   else: a poster is the one image uploaded separately from the
    ///   message that carries it, and unlike a photo a video has no second
    ///   source of pixels to fall back on. Such a key is re-asked over
    ///   `posterRetryDelays` and then settles into `missing` exactly like
    ///   any other 404 — a poster that failed for good must stop being
    ///   asked for, not polled forever.
    func image(id: Int64, preview: Bool, mayArriveLate: Bool = false) -> Image? {
        let key = key(id, preview: preview)
        if let cached = hot[key] { return cached }
        guard !missing.contains(key) else { return nil }

        let url = fileURL(key)
        if FileManager.default.fileExists(atPath: url.path) {
            guard let data = try? Data(contentsOf: url),
                  let decoded = PlatformImage.decode(data, maxPixels: Self.displayPixels)
            else {
                try? FileManager.default.removeItem(at: url)
                return nil
            }
            remember(key, PlatformImage.view(decoded))
            return hot[key]
        }
        guard !inFlight.contains(key) else { return nil }
        fetch(id: id, preview: preview, key: key, mayArriveLate: mayArriveLate)
        return nil
    }

    private func fetch(id: Int64, preview: Bool, key: String, mayArriveLate: Bool) {
        inFlight.insert(key)
        Task { [weak self] in
            guard let self else { return }
            // The key stays in `inFlight` for the whole ladder, so the
            // views asking for it wait on this one chain rather than
            // starting their own — and the budget is spent once per key
            // per launch however many bubbles draw it.
            for attempt in 0...posterRetryDelays.count {
                do {
                    // nil = a real 404: no preview, or not ours to see.
                    let data = try await api.attachmentData(id: id, preview: preview)
                    if data == nil, mayArriveLate, attempt < posterRetryDelays.count {
                        // Not there YET. Nothing observable happens here on
                        // purpose: `generation` is what re-renders the
                        // views, and a bump while the key is neither
                        // cached nor gated is a tight loop, not a retry.
                        try? await Task.sleep(for: posterRetryDelays[attempt])
                        continue
                    }
                    finish(key, data: data, settled: true)
                    return
                } catch {
                    // A timeout, a refused connection or a 5xx says nothing
                    // about whether the bytes exist. Marking the key missing
                    // here is what left a permanent spinner on every photo
                    // whose fetch happened to land during an outage: the gate
                    // in `image(id:preview:)` sits in front of the disk cache
                    // as well as the fetch, and nothing cleared it short of a
                    // logout. Only a real answer settles a key.
                    finish(key, data: nil, settled: false)
                    return
                }
            }
            // Not reachable: the last rung has no `continue`. A floor all
            // the same, because the cost of falling out of this loop is a
            // key stuck in `inFlight` — an image that never loads again
            // for the rest of the launch, and nothing to see that says so.
            finish(key, data: nil, settled: true)
        }
    }

    /// - Parameter settled: whether the answer is final. Only a real 404 is.
    private func finish(_ key: String, data: Data?, settled: Bool) {
        inFlight.remove(key)
        if let data, let decoded = PlatformImage.decode(data, maxPixels: Self.displayPixels) {
            try? data.write(to: fileURL(key), options: .atomic)
            remember(key, PlatformImage.view(decoded))
            generation &+= 1
        } else if settled {
            // Remember the miss: an attachment with no preview must not be
            // re-requested on every render pass.
            missing.insert(key)
            // A 404 IS a fetch landing, which is what `generation` means, so
            // it bumps like a success does — and safely, because the key is
            // now gated: the redraw this causes calls back in and gets nil
            // without starting anything. AvatarStore's `finish` bumps
            // unconditionally for the same reason.
            generation &+= 1
        }
        // The UNSETTLED case is the one that must stay silent. Bumping
        // there re-renders every view drawing this key, each of which calls
        // back in and starts the fetch again — which offline is a tight loop
        // rather than a retry, because nothing gates the key. Recovery is
        // driven by the socket reconnecting instead; see
        // `retryAfterReconnect()`.
    }

    /// Whether a fetch for this key is still running. Test-facing: an
    /// unsettled failure bumps nothing observable by design, so this is the
    /// only way to wait for one to have been processed rather than merely
    /// sent.
    func isFetching(id: Int64, preview: Bool) -> Bool {
        inFlight.contains(key(id, preview: preview))
    }

    /// The socket came back, so the network did. Forget what failed while
    /// it was gone and let the views ask again.
    ///
    /// This clears real 404s too, exactly as the Android counterpart does
    /// (`AttachmentRepository.kt`, `connectivity.onAvailable`): re-checking
    /// a genuinely absent preview costs one request that 404s again, and
    /// keeping two sets apart to save it is not worth the divergence.
    func retryAfterReconnect() {
        guard !missing.isEmpty else { return }
        missing.removeAll()
        generation &+= 1
    }

    /// Put bytes we already hold into the cache.
    ///
    /// The sender just made this preview; making their own device fetch it
    /// back from the server to draw its own bubble is a round trip for
    /// something already in hand, and it is what left the sender staring
    /// at a spinner while everyone else saw the picture.
    func seed(_ data: Data, id: Int64, preview: Bool) {
        let key = key(id, preview: preview)
        guard let decoded = PlatformImage.decode(data, maxPixels: Self.displayPixels) else {
            return
        }
        try? data.write(to: fileURL(key), options: .atomic)
        missing.remove(key)
        remember(key, PlatformImage.view(decoded))
        generation &+= 1
    }

    // MARK: - Repairing a poster the server never got

    /// The marker's file extension. `34-preview.unsent` sits beside
    /// `34-preview.jpg`, which is the whole design: the note and the bytes
    /// it refers to live and die together, so a caches purge cannot leave
    /// a work item pointing at material that is gone. Same name on
    /// Android (`AttachmentRepository.POSTER_MARKER_EXTENSION`).
    static let posterMarkerExtension = "unsent"

    /// How many ANSWERED attempts a poster repair is worth before the
    /// video is given up on for good.
    ///
    /// A budget, not a schedule — nothing here is on a timer. The pass
    /// runs when the socket connects, which is once per launch plus once
    /// per recovery from a network drop, so three answered failures is a
    /// video the server is refusing rather than one that lost a coin
    /// flip. After that the marker is deleted and the tile keeps its play
    /// badge for good, which is the honest outcome: `image(id:preview:)`
    /// settles the same key after its own bounded re-check, and nothing
    /// on either side polls.
    ///
    /// A TRANSPORT failure does not spend the budget, exactly as it does
    /// not settle a fetch in `finish(_:data:settled:)`: nobody answered,
    /// so nothing was learned. It cannot spin — the pass is only ever
    /// started by an event, never by itself.
    static let posterRepairAttempts = 3

    /// One pass at a time, so a flapping socket cannot start a second.
    @ObservationIgnored private var posterRepair: Task<Void, Never>?

    /// Say what became of a video's poster upload.
    ///
    /// Called by the send path for VIDEOS only. A photo needs none of
    /// this: a bubble whose preview is missing falls back to the full
    /// bytes, so a lost photo preview costs bandwidth rather than the
    /// picture. A video's bytes are a video — there is no second source
    /// of pixels, and that asymmetry is the whole of issue #54.
    ///
    /// - Parameter landed: whether the server took it. False leaves a
    ///   marker for `repairPosters()` to find; true removes one, which is
    ///   what makes a later successful send of the same id idempotent.
    func notePosterUpload(id: Int64, landed: Bool) {
        let marker = posterMarkerURL(id)
        guard !landed else {
            try? FileManager.default.removeItem(at: marker)
            return
        }
        // Only worth a marker if the bytes to send are actually here.
        // `seed` runs just before the upload, so they normally are; when
        // they are not there is nothing to repair from and a marker would
        // only cost a directory read on every reconnect.
        guard FileManager.default.fileExists(
            atPath: fileURL(key(id, preview: true)).path)
        else { return }
        try? Data("0".utf8).write(to: marker, options: .atomic)
    }

    /// Push up the posters this device made and the server never got.
    ///
    /// THE OTHER HALF OF ISSUE #38. That one taught the read side that
    /// `has_preview` is a hint, so a poster that EXISTS is picked up
    /// without a relaunch. This is what makes one exist. The upload is
    /// best-effort by design — a thumbnail must never cost the send — and
    /// before this, best-effort meant exactly once: a failed `uploadPreview`
    /// left `has_preview = false` on the server with nobody able to
    /// correct it, because the only device holding the pixels had already
    /// moved on. The sender saw nothing wrong (its own bubble draws from
    /// the seeded cache), which is why it read as "the video from the
    /// other person is broken".
    ///
    /// NO WIRE CHANGE. `PUT /attachments/{id}/preview` is uploader-only,
    /// idempotent, and legal after the message that claims the attachment
    /// exists (server/src/handlers_attachment.rs) — it overwrites the file
    /// and sets `has_preview = true`. Repairing is the same request the
    /// send makes, sent again.
    ///
    /// Called when the socket connects: once per launch, and again each
    /// time the network comes back — the same signal `retryAfterReconnect`
    /// uses, and the moment we know a request is worth making. Android's
    /// counterpart hangs off `connectivity.onAvailable` for the same
    /// reason.
    ///
    /// - Returns: the running pass, or nil if one was already going. The
    ///   app ignores it; a test awaits it.
    @discardableResult
    func repairPosters() -> Task<Void, Never>? {
        guard posterRepair == nil else { return nil }
        let task = Task { [weak self] in
            await self?.runPosterRepair()
            self?.posterRepair = nil
        }
        posterRepair = task
        return task
    }

    private func runPosterRepair() async {
        let suffix = "-preview." + Self.posterMarkerExtension
        guard let names = try? FileManager.default
            .contentsOfDirectory(atPath: directory.path)
        else { return }
        for name in names where name.hasSuffix(suffix) {
            guard let id = Int64(name.dropLast(suffix.count)) else { continue }
            // A dead session fails every remaining marker the same way,
            // so stop rather than burn the whole queue's budget on it.
            if await repairPoster(id: id) == .sessionGone { return }
        }
    }

    private enum PosterRepairOutcome { case done, sessionGone }

    private func repairPoster(id: Int64) async -> PosterRepairOutcome {
        let marker = posterMarkerURL(id)
        guard let jpeg = try? Data(contentsOf: fileURL(key(id, preview: true))),
              !jpeg.isEmpty
        else {
            // Nothing to send. The bytes were purged with the caches, or
            // the frame grab never produced any — and no number of tries
            // conjures pixels. Says so, because "this video never had a
            // poster" and "its upload failed" need different fixes.
            try? FileManager.default.removeItem(at: marker)
            AppLog.sync.error(
                "Poster repair impossible for attachment \(id, privacy: .public): no poster bytes held on this device")
            return .done
        }

        do {
            try await api.uploadPreview(attachmentID: id, jpeg: jpeg)
            try? FileManager.default.removeItem(at: marker)
            AppLog.sync.info("Poster repaired for attachment \(id, privacy: .public)")
            // Every reader that settled on "there is no poster" may now be
            // wrong; this device's own bubbles included.
            missing.remove(key(id, preview: true))
            generation &+= 1
            return .done
        } catch {
            switch error as? APIError {
            case .unauthorized:
                return .sessionGone
            case .transport, .notConfigured:
                // Nobody answered. Nothing was learned, so nothing is
                // spent; the next reconnect asks again.
                return .done
            case .notFound:
                // The attachment is gone, or was never ours to replace.
                // As terminal as an answer gets.
                try? FileManager.default.removeItem(at: marker)
                AppLog.sync.error(
                    "Poster repair abandoned for attachment \(id, privacy: .public): the server has no such attachment")
                return .done
            default:
                let spent = posterRepairAttemptsSpent(marker) + 1
                if spent >= Self.posterRepairAttempts {
                    try? FileManager.default.removeItem(at: marker)
                    AppLog.sync.error(
                        "Poster repair given up for attachment \(id, privacy: .public) after \(spent, privacy: .public) answered attempts: \(String(describing: error), privacy: .public)")
                } else {
                    try? Data(String(spent).utf8).write(to: marker, options: .atomic)
                }
                return .done
            }
        }
    }

    /// The marker's whole contents: a decimal count of answered attempts.
    /// An unreadable or absent one reads as zero — a repair that runs once
    /// too often is a few kilobytes; one that never runs is the bug.
    private func posterRepairAttemptsSpent(_ marker: URL) -> Int {
        guard let data = try? Data(contentsOf: marker),
              let text = String(data: data, encoding: .utf8),
              let value = Int(text.trimmingCharacters(in: .whitespacesAndNewlines))
        else { return 0 }
        return max(0, value)
    }

    /// Test-facing: whether this id is still waiting to be repaired.
    func isPosterUnsent(id: Int64) -> Bool {
        FileManager.default.fileExists(atPath: posterMarkerURL(id).path)
    }

    private func remember(_ key: String, _ image: Image) {
        hot[key] = image
        hotOrder.append(key)
        while hotOrder.count > Self.hotLimit {
            let oldest = hotOrder.removeFirst()
            hot[oldest] = nil
        }
    }

    /// Forget the bytes of attachments whose messages have just been
    /// deleted locally.
    ///
    /// The narrow twin of `clear()`, and it exists for the one thing in
    /// this protocol that can make a chat genuinely vanish: a direct chat
    /// whose peer deleted their account goes, both halves (docs/
    /// protocol.md, "Deleting an account"). The rows go with the chat, and
    /// a cached file keyed by an attachment id nothing names any more can
    /// never be drawn again — only found, by whoever goes looking through
    /// the caches directory. The FILES are the part that would otherwise
    /// survive, exactly as at logout.
    ///
    /// Both keys per id: a photo is cached twice, as its preview and at
    /// display size.
    func forget(attachmentIDs: [Int64]) {
        guard !attachmentIDs.isEmpty else { return }
        for id in attachmentIDs {
            for key in [key(id, preview: true), key(id, preview: false)] {
                hot[key] = nil
                hotOrder.removeAll { $0 == key }
                missing.remove(key)
                try? FileManager.default.removeItem(at: fileURL(key))
            }
            // The poster is gone with the chat, so the note asking for it
            // to be re-sent must go too — the repair pass would otherwise
            // find a marker with nothing behind it.
            try? FileManager.default.removeItem(at: posterMarkerURL(id))
        }
        generation &+= 1
    }

    /// Logout: the next account must not see the previous one's photos.
    /// The FILES go too — they are the part that would otherwise survive.
    func clear() {
        hot.removeAll()
        hotOrder.removeAll()
        missing.removeAll()
        try? FileManager.default.removeItem(at: directory)
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        generation &+= 1
    }
}
