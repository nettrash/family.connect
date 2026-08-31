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
//  Android counterpart: data/repo/AttachmentRepository.kt
//

import SwiftUI

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
    init(api: APIClient, directory: URL? = nil) {
        self.api = api
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

    /// The image if it is already here, else nil — and a fetch is started.
    /// Safe to call from a view body.
    func image(id: Int64, preview: Bool) -> Image? {
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
        fetch(id: id, preview: preview, key: key)
        return nil
    }

    private func fetch(id: Int64, preview: Bool, key: String) {
        inFlight.insert(key)
        Task { [weak self] in
            guard let self else { return }
            do {
                // nil = a real 404: no preview, or not ours to see.
                let data = try await api.attachmentData(id: id, preview: preview)
                finish(key, data: data, settled: true)
            } catch {
                // A timeout, a refused connection or a 5xx says nothing
                // about whether the bytes exist. Marking the key missing
                // here is what left a permanent spinner on every photo
                // whose fetch happened to land during an outage: the gate
                // in `image(id:preview:)` sits in front of the disk cache
                // as well as the fetch, and nothing cleared it short of a
                // logout. Only a real answer settles a key.
                finish(key, data: nil, settled: false)
            }
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
        }
        // Deliberately NOT bumping `generation` on an unsettled failure.
        // The bump re-renders every view drawing this key, each of which
        // calls back in and starts the fetch again — which offline is a
        // tight loop rather than a retry. Recovery is driven by the socket
        // reconnecting instead; see `retryAfterReconnect()`.
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
