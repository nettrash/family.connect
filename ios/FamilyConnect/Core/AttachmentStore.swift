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
//  second copy on the phone. The ONE exception is an ANIMATION (#96,
//  docs/protocol.md "How it is drawn"): a tile plays it by itself, on a
//  loop, so it downloads the video once — at most 8 MiB, the protocol's
//  bound — and keeps it like a photo (see "Animations").
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
#if os(iOS)
import UIKit
#endif

@MainActor
@Observable
final class AttachmentStore {

    private let api: APIClient
    private let directory: URL
    /// The text of recordings this device asked for (docs/protocol.md,
    /// "Transcripts on request"), kept in a folder inside this store's own
    /// so `forget` and `clear` take it with the attachments it is about.
    let transcripts: TranscriptStore
    /// Bumped whenever anything lands — a fetch, a 404, a seed, a purge.
    ///
    /// NO VIEW READS THIS ANY MORE, and none should. It used to be the only
    /// observable thing the store had, so every tile, sticker, album and
    /// poster in the window read it, and every fetch that landed — for any
    /// picture, anywhere — re-rendered all of them: opening a chat after a
    /// long absence, a hundred previews arriving one by one was a hundred
    /// passes over every image view on screen. Each lookup below now
    /// registers a dependency on ITS OWN key (`track`), and a landing
    /// invalidates exactly the views drawing that key (`touch`). This
    /// counter stays as the store-wide "something happened", which is what
    /// the tests wait on.
    private(set) var generation = 0
    /// One observable stamp per key, which is what makes invalidation
    /// per picture rather than per store (see `generation`). The dictionary
    /// itself is ignored by observation — only a stamp's `value` is read
    /// from a view — so adding a key while a body runs invalidates nobody.
    @ObservationIgnored private var stamps: [String: KeyStamp] = [:]
    /// Files on disk being decoded off the main actor, so the views asking
    /// for one start one decode between them.
    @ObservationIgnored private var decoding: Set<String> = []
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
    /// The decoded pictures in front of the disk, so a visible bubble does
    /// not re-read and re-decode on every pass. Least recently used, and
    /// bounded by what the pictures COST decoded rather than by how many
    /// there are (`ByteBudgetCache` has why forty-first-in-first-out was
    /// the wrong rule on both counts).
    @ObservationIgnored private var hot = ByteBudgetCache<String, Image>(budget: AttachmentStore.hotBudget)
    /// Animations being downloaded, and those the server has none of.
    @ObservationIgnored private var animationsInFlight: Set<Int64> = []
    @ObservationIgnored private var animationsMissing: Set<Int64> = []
    /// A moving sticker's frames, decoded once per drawn size
    /// (`StickerFrames`); keyed `"<id>@<edge>"`.
    @ObservationIgnored private var frames = ByteBudgetCache<String, StickerFrames>(
        budget: AttachmentStore.framesBudget)
    /// Frame decodes in flight, and the keys that have none to give — a
    /// still sticker, or one that cannot move here. The bytes behind an id
    /// never change, so a settled "no" is final until a forget or a clear.
    @ObservationIgnored private var framesDecoding: Set<String> = []
    @ObservationIgnored private var framesNone: Set<String> = []
    /// The system's memory warning, observed for as long as the store lives.
    @ObservationIgnored private var memoryWarning: NSObjectProtocol?

    /// What the decoded pictures may cost together. Room for about ninety
    /// bubble previews (600 px, ~1 MB each) or a dozen full photos at
    /// display size — comfortably more than a window of rows draws at once,
    /// which is the property that matters: nothing on screen is evicted by
    /// its own neighbours.
    #if os(macOS)
    static let hotBudget = 192 * 1024 * 1024
    static let framesBudget = 128 * 1024 * 1024
    #else
    static let hotBudget = 96 * 1024 * 1024
    /// What every moving sticker's frames may cost together: a panel's worth
    /// of cells plus the few in a chat. An animating layer keeps its own
    /// frames, so this bounds what is kept for LATER, not what plays.
    static let framesBudget = 64 * 1024 * 1024
    #endif

    /// Longest edge kept decoded. A bubble draws 240pt and the viewer a
    /// screen width, so the uploaded 2048 would be several times the
    /// pixels anyone sees. Matches Android's DISPLAY_PIXELS.
    nonisolated static let displayPixels = 1440

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
        self.transcripts = TranscriptStore(
            api: api, directory: self.directory.appendingPathComponent("transcripts", isDirectory: true))
        #if os(iOS)
        // Everything in memory here is a copy of a file on disk, so the
        // right answer to the system asking for memory back is all of it:
        // what is on screen keeps drawing what it already rendered, and
        // costs one re-decode the next time it is asked for.
        memoryWarning = NotificationCenter.default.addObserver(
            forName: UIApplication.didReceiveMemoryWarningNotification,
            object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.purgeMemory() }
        }
        #endif
    }

    /// Drop every decoded picture and sticker frame, keeping the files.
    func purgeMemory() {
        hot.removeAll()
        frames.removeAll()
    }

    // MARK: - Per-key observation

    /// Register the calling view's dependency on one key: reading the
    /// stamp's `value` inside a body is what Observation records.
    private func track(_ key: String) {
        if let stamp = stamps[key] {
            _ = stamp.value
        } else {
            let stamp = KeyStamp()
            stamps[key] = stamp
            _ = stamp.value
        }
    }

    /// Invalidate the views drawing one key — and only those. A key no view
    /// has ever asked about has no stamp and nothing to invalidate.
    private func touch(_ key: String) {
        stamps[key]?.value &+= 1
    }

    /// Something about this attachment's ORIGINAL bytes changed — for a view
    /// that waits on their arrival without drawing them through `image` or
    /// `stickerImage` (the enlarged sticker's "add to the pack" offer).
    /// Reading it registers the dependency, like every lookup here.
    func version(id: Int64) -> Int {
        let key = key(id, preview: false)
        track(key)
        return stamps[key]?.value ?? 0
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
    ///
    /// A file already on disk is decoded OFF the main actor too, and the
    /// view told when it lands — never read and decoded right here. That
    /// used to happen inside the body that asked, so opening a chat whose
    /// pictures had fallen out of the in-memory cache decoded every one of
    /// them, synchronously, on the actor the scroll is driven from; with
    /// the window widened over a few hundred unread rows that was the
    /// opening itself stalling. The tile reserves its exact shape before
    /// any bytes arrive (AttachmentView), so the one pass this now spends
    /// on the placeholder moves nothing.
    func image(id: Int64, preview: Bool, mayArriveLate: Bool = false) -> Image? {
        let key = key(id, preview: preview)
        track(key)
        if let cached = hot.value(for: key) { return cached }
        guard !missing.contains(key) else { return nil }

        let url = fileURL(key)
        if FileManager.default.fileExists(atPath: url.path) {
            decodeFromDisk(key, url: url)
            return nil
        }
        guard !inFlight.contains(key) else { return nil }
        fetch(id: id, preview: preview, key: key, mayArriveLate: mayArriveLate)
        return nil
    }

    /// Read and decode a cached file off the main actor, then wake the views
    /// drawing it. A file that is no picture is deleted, and the views asking
    /// again find no file and fetch it — what the synchronous read used to
    /// do, one pass later. Either way the wake cannot loop: the next ask hits
    /// the cache, or starts a fetch that `inFlight` then gates.
    private func decodeFromDisk(_ key: String, url: URL) {
        guard !decoding.contains(key) else { return }
        decoding.insert(key)
        Task { [weak self] in
            let still = await Self.decodedFile(at: url)
            guard let self else { return }
            decoding.remove(key)
            if let still {
                remember(key, still.image)
            } else {
                try? FileManager.default.removeItem(at: url)
            }
            touch(key)
        }
    }

    /// A cached file, decoded to display size, off the caller's actor.
    @concurrent
    private static func decodedFile(at url: URL) async -> StickerPack.Still? {
        guard let data = try? Data(contentsOf: url) else { return nil }
        return decoded(data)
    }

    /// Bytes decoded to display size — the one decode every path shares.
    private nonisolated static func decoded(_ data: Data) -> StickerPack.Still? {
        PlatformImage.decode(data, maxPixels: displayPixels).map { StickerPack.Still(image: $0) }
    }

    /// A PHOTO's preview — or, once the server has said it has none, the
    /// photo itself. Safe to call from a view body, like `image`.
    ///
    /// For the one caller that cannot know better up front: a board note
    /// keeps its picture's id and nothing else (NoteEntity stores no
    /// `has_preview`), and the pin that made it may have lost its preview
    /// upload — both apps treat that upload as best effort. Asking for the
    /// preview alone left such a note a spinner for good; asking for both at
    /// once would fetch every full photo on the wall. So the full bytes are
    /// asked for only after the preview's 404 has settled — which bumps
    /// `generation`, so a view reading it asks again and gets here.
    func previewOrPhoto(id: Int64) -> Image? {
        if let preview = image(id: id, preview: true) { return preview }
        guard missing.contains(key(id, preview: true)) else { return nil }
        return image(id: id, preview: false)
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
                    // Decoded off the main actor, like a file on disk.
                    let still: StickerPack.Still? = if let data {
                        await Self.decodedBytes(data)
                    } else {
                        nil
                    }
                    finish(key, data: data, still: still, settled: true)
                    return
                } catch {
                    // A timeout, a refused connection or a 5xx says nothing
                    // about whether the bytes exist. Marking the key missing
                    // here is what left a permanent spinner on every photo
                    // whose fetch happened to land during an outage: the gate
                    // in `image(id:preview:)` sits in front of the disk cache
                    // as well as the fetch, and nothing cleared it short of a
                    // logout. Only a real answer settles a key.
                    finish(key, data: nil, still: nil, settled: false)
                    return
                }
            }
            // Not reachable: the last rung has no `continue`. A floor all
            // the same, because the cost of falling out of this loop is a
            // key stuck in `inFlight` — an image that never loads again
            // for the rest of the launch, and nothing to see that says so.
            finish(key, data: nil, still: nil, settled: true)
        }
    }

    @concurrent
    private static func decodedBytes(_ data: Data) async -> StickerPack.Still? {
        decoded(data)
    }

    /// - Parameter settled: whether the answer is final. Only a real 404 is.
    private func finish(_ key: String, data: Data?, still: StickerPack.Still?, settled: Bool) {
        inFlight.remove(key)
        if let data, let still {
            try? data.write(to: fileURL(key), options: .atomic)
            remember(key, still.image)
            generation &+= 1
            touch(key)
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
            touch(key)
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
        guard !missing.isEmpty || !animationsMissing.isEmpty else { return }
        let settled = missing
        let animations = animationsMissing
        missing.removeAll()
        animationsMissing.removeAll()
        generation &+= 1
        // Only the views that were drawing a "no" have anything to ask again.
        for key in settled { touch(key) }
        for id in animations { touch(animationKey(id)) }
    }

    // MARK: - Animations: the video, downloaded

    /// Where an animation's video is kept, whether or not it is there yet.
    /// `.mp4`, unlike everything else here: AVFoundation reads the
    /// extension.
    func animationFileURL(id: Int64) -> URL {
        directory.appendingPathComponent("\(id)-animation").appendingPathExtension("mp4")
    }

    /// An animation's video on this device, or nil — and then the download
    /// is started, and `generation` moves when it lands. Safe to call from
    /// a view body, like `image`.
    ///
    /// The tile's caller decides WHETHER to ask: not above the protocol's
    /// 8 MiB, and not while the system asks for less motion. A provisional
    /// (negative) id is never asked for — the server has never heard of
    /// it; a pending send's own file is `seedAnimation`'s.
    func animationURL(id: Int64) -> URL? {
        track(animationKey(id))
        let url = animationFileURL(id: id)
        if FileManager.default.fileExists(atPath: url.path) { return url }
        guard id > 0, !animationsInFlight.contains(id), !animationsMissing.contains(id) else {
            return nil
        }
        animationsInFlight.insert(id)
        Task { [weak self] in
            guard let self else { return }
            let scratch = url.deletingPathExtension().appendingPathExtension("part")
            do {
                try await api.downloadAttachment(id: id, to: scratch)
                try? FileManager.default.removeItem(at: url)
                try FileManager.default.moveItem(at: scratch, to: url)
                animationsInFlight.remove(id)
                generation &+= 1
                touch(animationKey(id))
            } catch APIError.notFound(_) {
                try? FileManager.default.removeItem(at: scratch)
                animationsInFlight.remove(id)
                animationsMissing.insert(id)
                // Settled, like a photo's 404: the tile keeps its poster.
                generation &+= 1
                touch(animationKey(id))
            } catch {
                // Says nothing about whether the video exists — `fetch`'s
                // rule: no bump, so no tight loop; the next draw asks again.
                try? FileManager.default.removeItem(at: scratch)
                animationsInFlight.remove(id)
            }
        }
        return nil
    }

    /// Put an animation this device already holds — its own send — where
    /// the tile looks for it, so the sender's bubble moves without
    /// fetching back what it just made.
    func seedAnimation(copying source: URL, id: Int64) {
        let url = animationFileURL(id: id)
        guard !FileManager.default.fileExists(atPath: url.path) else { return }
        guard (try? FileManager.default.copyItem(at: source, to: url)) != nil else { return }
        animationsMissing.remove(id)
        generation &+= 1
        touch(animationKey(id))
    }

    /// The observation key of an animation's video — its own, so the video
    /// landing does not redraw the poster drawn under it, nor the reverse.
    private func animationKey(_ id: Int64) -> String {
        "\(id)-animation"
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
        remember(key, decoded)
        generation &+= 1
        touch(key)
    }

    // MARK: - Stickers: the original bytes

    /// The ORIGINAL bytes of an attachment, if this device holds them.
    ///
    /// For stickers (docs/protocol.md, "Sticker pack"), which are the one
    /// kind of picture whose bytes matter beyond their first frame: an
    /// animated WebP is drawn from them, a send uploads them again as the
    /// message's own copy, and "does the pack already hold this?" compares
    /// them. The cache has always kept exactly what the server sent — only
    /// the hot `Image` in front of it is a decoded frame — so this is a
    /// read of the same file `image(id:preview:false)` draws from.
    ///
    /// NEVER the preview. A preview is a JPEG, and a sticker's `has_preview`
    /// can be true by inheritance from a photograph with the same bytes.
    ///
    /// Disk-only: nil means "not here yet". A view calls `stickerImage(id:)`
    /// to start the fetch and comes back when `generation` moves; a send
    /// calls `originalBytes(id:)`.
    ///
    /// ASYNC, and read off the main actor (`StickerPack.bytes`). It was a
    /// plain `Data(contentsOf:)` called from view code — half a megabyte
    /// per sticker, from the actor the taps are handled on.
    func cachedBytes(id: Int64) async -> Data? {
        await StickerPack.bytes(fileAt: originalURL(id: id))
    }

    /// Where an attachment's original bytes are cached, whether or not
    /// they are there yet — for the callers that read the file somewhere
    /// other than the main actor.
    func originalURL(id: Int64) -> URL {
        fileURL(key(id, preview: false))
    }

    /// The same bytes, fetched when they are not here. nil when the server
    /// has none or could not be reached — a caller with no network and no
    /// cached copy has nothing to send, and says so.
    ///
    /// Its own request rather than a wait on `fetch`'s: a tap on a sticker
    /// is a person waiting, and the view-driven fetch reports nothing to
    /// anybody but `generation`. The two may overlap and write the same
    /// immutable bytes to the same file, which is harmless.
    func originalBytes(id: Int64) async -> Data? {
        if let cached = await cachedBytes(id: id) { return cached }
        guard let data = try? await api.attachmentData(id: id, preview: false) ?? nil,
              !data.isEmpty
        else { return nil }
        seed(data, id: id, preview: false)
        return data
    }

    /// A sticker's still frame if it is already decoded, else nil — and the
    /// decode (or, with nothing cached, the fetch) is started. Safe to call
    /// from a view body, like `image`.
    ///
    /// `image(id:preview:false)` with ONE difference, which is the reason
    /// it exists: that one reads the file and decodes it right there, in
    /// the body that asked. For a photograph in a chat that is one tile; a
    /// sticker panel is a grid of up to two hundred, every cell a read and
    /// a decode on the actor the scroll is driven from, and the hot cache
    /// in front of it holds forty. So a sticker's file is read and decoded
    /// OFF the main actor (`StickerPack.still`) and the view is told by
    /// `generation`, exactly as it is told of a fetch.
    ///
    /// Never the preview — the key is the original's, unconditionally.
    func stickerImage(id: Int64) -> Image? {
        let key = key(id, preview: false)
        track(key)
        if let cached = hot.value(for: key) { return cached }
        guard !missing.contains(key), !inFlight.contains(key) else { return nil }
        inFlight.insert(key)
        let url = fileURL(key)
        Task { [weak self] in
            if let still = await StickerPack.still(fileAt: url, maxPixels: Self.displayPixels) {
                self?.finishSticker(key, still: still, settled: true)
                return
            }
            // Not cached (or cached and unreadable, which the fetch
            // overwrites). One attempt: a sticker has no late-arriving
            // poster to wait for.
            guard let self else { return }
            do {
                // nil = a real 404.
                guard let data = try await api.attachmentData(id: id, preview: false) else {
                    finishSticker(key, still: nil, settled: true)
                    return
                }
                let still = await StickerPack.still(
                    storing: data, at: url, maxPixels: Self.displayPixels)
                // Bytes that are no picture settle the key as a 404 does:
                // asking again would be answered with the same bytes.
                finishSticker(key, still: still, settled: true)
            } catch {
                // Says nothing about whether the bytes exist — see `fetch`.
                finishSticker(key, still: nil, settled: false)
            }
        }
        return nil
    }

    /// `finish`'s twin for a frame that was decoded somewhere else. The
    /// same three outcomes and the same rule about which of them may bump
    /// `generation`: a success and a settled miss do, an unsettled failure
    /// must not (see `finish`).
    private func finishSticker(_ key: String, still: StickerPack.Still?, settled: Bool) {
        inFlight.remove(key)
        if let still {
            remember(key, still.image)
            generation &+= 1
            touch(key)
        } else if settled {
            missing.insert(key)
            generation &+= 1
            touch(key)
        }
    }

    // MARK: - Stickers: the frames of one that moves

    /// A moving sticker's frames at `edge` pixels, if they are decoded,
    /// else nil — and the decode is started. Safe to call from a view body,
    /// like `image`; the view is told by its key when they land.
    ///
    /// nil is ALSO the settled answer for a sticker that does not move (or
    /// cannot, here), and stays nil without decoding anything again: the
    /// caller draws the still either way, so the two never need telling
    /// apart. Ask only for a sticker that is on screen and allowed to move
    /// (`StickerMotion`) — asking is what spends the decode.
    ///
    /// `edge` is `StickerFrames.requestedEdge`: a few sizes, each cached on
    /// its own, so the panel's small cells and the chat's larger box do not
    /// evict one another by asking for different resolutions of one entry.
    func stickerFrames(id: Int64, edge: Int) -> StickerFrames? {
        guard edge > 0 else { return nil }
        let key = "\(id)@\(edge)"
        track(key)
        if let cached = frames.value(for: key) { return cached }
        guard !framesNone.contains(key), !framesDecoding.contains(key) else { return nil }
        framesDecoding.insert(key)
        let url = originalURL(id: id)
        Task { [weak self] in
            let decoded = await StickerFrames.decode(fileAt: url, maxEdge: edge)
            guard let self else { return }
            framesDecoding.remove(key)
            if let decoded {
                frames.insert(decoded, for: key, cost: decoded.byteCost)
            } else {
                framesNone.insert(key)
            }
            touch(key)
        }
        return nil
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
            touch(key(id, preview: true))
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

    /// Keep a decoded picture, charged at what its pixels cost.
    private func remember(_ key: String, _ image: CGImage) {
        hot.insert(PlatformImage.view(image), for: key, cost: image.bytesPerRow * image.height)
    }

    /// Test-facing: what the decoded pictures cost right now, and whether
    /// one key is among them.
    var hotCost: Int { hot.totalCost }
    func isHot(id: Int64, preview: Bool) -> Bool { hot.contains(key(id, preview: preview)) }

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
                hot.remove(key)
                missing.remove(key)
                try? FileManager.default.removeItem(at: fileURL(key))
                touch(key)
            }
            // A moving sticker's frames are its pixels too.
            let prefix = "\(id)@"
            frames.removeAll { $0.hasPrefix(prefix) }
            framesNone = framesNone.filter { !$0.hasPrefix(prefix) }
            // The poster is gone with the chat, so the note asking for it
            // to be re-sent must go too — the repair pass would otherwise
            // find a marker with nothing behind it.
            try? FileManager.default.removeItem(at: posterMarkerURL(id))
            // An animation's video goes with its message, like its poster.
            try? FileManager.default.removeItem(at: animationFileURL(id: id))
            animationsMissing.remove(id)
            touch(animationKey(id))
        }
        // A kept transcript is about one of these attachments too, and the
        // words in it are the last thing that should outlive the chat.
        transcripts.forget(attachmentIDs: attachmentIDs)
        generation &+= 1
    }

    /// Logout: the next account must not see the previous one's photos.
    /// The FILES go too — they are the part that would otherwise survive.
    func clear() {
        hot.removeAll()
        frames.removeAll()
        framesNone.removeAll()
        missing.removeAll()
        animationsMissing.removeAll()
        try? FileManager.default.removeItem(at: directory)
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        // The kept transcripts were inside that directory; this empties
        // their memory too, so the next account reads none of them.
        transcripts.clear()
        generation &+= 1
        // Every view drawing anything from the previous account asks again,
        // and finds nothing.
        for stamp in stamps.values { stamp.value &+= 1 }
    }
}

/// One key's observable stamp (`AttachmentStore.track`). A class, so the
/// dictionary holding it can stay invisible to observation while each
/// stamp's `value` is seen by exactly the views that read it.
@MainActor @Observable
final class KeyStamp {
    var value = 0
}
