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

    init(api: APIClient) {
        self.api = api
        let caches = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
        directory = caches.appendingPathComponent("attachments", isDirectory: true)
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
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
            guard let data = try? Data(contentsOf: url), let ui = UIImage(data: data) else {
                try? FileManager.default.removeItem(at: url)
                return nil
            }
            remember(key, Image(uiImage: ui))
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
            let data = try? await api.attachmentData(id: id, preview: preview)
            inFlight.remove(key)
            guard let data, let ui = UIImage(data: data) else {
                // A 404 is final (no preview, or not ours to see); a
                // transport failure is not, but distinguishing them here
                // would need the error, and a scroll away and back retries
                // anyway once the hot entry is gone.
                missing.insert(key)
                return
            }
            try? data.write(to: fileURL(key), options: .atomic)
            remember(key, Image(uiImage: ui))
            generation &+= 1
        }
    }

    /// Put bytes we already hold into the cache.
    ///
    /// The sender just made this preview; making their own device fetch it
    /// back from the server to draw its own bubble is a round trip for
    /// something already in hand, and it is what left the sender staring
    /// at a spinner while everyone else saw the picture.
    func seed(_ data: Data, id: Int64, preview: Bool) {
        let key = key(id, preview: preview)
        guard let ui = UIImage(data: data) else { return }
        try? data.write(to: fileURL(key), options: .atomic)
        missing.remove(key)
        remember(key, Image(uiImage: ui))
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
