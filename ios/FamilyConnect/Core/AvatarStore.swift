//
//  AvatarStore.swift
//  FamilyConnect
//
//  Profile pictures, fetched once and kept for the session.
//
//  The cache key is `(userID, avatarVersion)` and the server guarantees
//  the version only ever moves forward, so a hit can never be stale —
//  which is why nothing here revalidates, and why a changed picture is
//  simply a different key rather than an invalidation problem.
//
//  Unlike LinkPreviewLoader this talks only to the family's OWN server,
//  so there is no privacy switch and no third party involved: the bytes
//  come from the same host the messages do.
//
//  In memory only. A picture is derived data — cheap to re-fetch on the
//  next launch, and persisting it would mean a second copy of something
//  the server already keeps (and a second thing to wipe on logout).
//
//  Android counterpart: android/…/data/net/AvatarRepository.kt.
//

import ImageIO
import SwiftUI
import UIKit

@MainActor @Observable
final class AvatarStore {

    /// Bumped when a picture lands, so views that draw one re-render.
    private(set) var generation = 0

    /// Bumped on every account change — see `finish(_:image:settled:startedAt:)`.
    private var session = 0

    /// Called when an avatar request meets a 401. The store has no idea
    /// what a session is; the app wires this to AppSession so a dead
    /// session is noticed here too, and not only by the chat sync.
    var onUnauthorized: (() -> Void)?

    private struct Key: Hashable {
        let userID: Int64
        let version: Int64
    }

    private var images: [Key: Image] = [:]
    private var missing: Set<Key> = []
    private var inFlight: Set<Key> = []
    private var order: [Key] = []

    /// A family is small; this is a runaway guard, not a working set.
    private static let maxEntries = 64

    /// Longest edge kept in memory. The largest avatar drawn anywhere is
    /// a 56pt circle, so even on a 3x screen this is generous.
    /// `nonisolated` because the decode deliberately runs off the main
    /// actor — the module defaults to MainActor isolation, which would
    /// otherwise make this constant unreachable from there.
    private nonisolated static let avatarMaxPixels = 256

    private let api: APIClient

    init(api: APIClient) {
        self.api = api
    }

    /// The picture for a user at a version, fetching it the first time it
    /// is asked for. `version == 0` means they have none.
    func image(userID: Int64, version: Int64) -> Image? {
        guard version > 0 else { return nil }
        let key = Key(userID: userID, version: version)
        if let image = images[key] { return image }
        guard !missing.contains(key), !inFlight.contains(key) else { return nil }
        load(key)
        return nil
    }

    /// Drop everything — called on logout, where the next user must not
    /// inherit this one's faces. In-flight fetches are left to finish;
    /// the bumped `session` is what stops their results from landing.
    func clear() {
        images.removeAll()
        missing.removeAll()
        order.removeAll()
        session &+= 1
        generation &+= 1
    }

    private func load(_ key: Key) {
        inFlight.insert(key)
        let startedAt = session
        Task { [weak self] in
            guard let self else { return }
            do {
                // nil = 404: no picture, or one we may not see.
                let data = try await api.avatar(userID: key.userID, version: key.version)
                finish(key, image: data.flatMap(Self.downsample), settled: true, startedAt: startedAt)
            } catch APIError.unauthorized {
                // The session died under us. Nothing about this face is
                // settled, and the app has to leave for the login screen.
                finish(key, image: nil, settled: false, startedAt: startedAt)
                onUnauthorized?()
            } catch {
                // A timeout or a 5xx says nothing about whether the
                // picture exists. Marking it missing here is what would
                // freeze a whole roster on initials after one bad minute
                // on cellular, with nothing to undo it but a relaunch.
                finish(key, image: nil, settled: false, startedAt: startedAt)
            }
        }
    }

    /// - Parameters:
    ///   - settled: whether the answer is final. Only a real 404 is.
    ///   - startedAt: the session this fetch began in; a result that
    ///     outlives its session is dropped rather than cached, so the
    ///     next account never inherits a face.
    private func finish(_ key: Key, image: Image?, settled: Bool, startedAt: Int) {
        inFlight.remove(key)
        guard startedAt == session else { return }
        if let image {
            evictIfNeeded()
            images[key] = image
            order.append(key)
        } else if settled {
            // Remember the miss: a member with no picture (or one we may
            // not see) must not be re-requested on every render pass.
            missing.insert(key)
        }
        generation &+= 1
    }

    private func evictIfNeeded() {
        guard order.count >= Self.maxEntries else { return }
        let dropping = order.prefix(Self.maxEntries / 2)
        for key in dropping { images[key] = nil }
        order.removeFirst(dropping.count)
    }

    /// Decode at avatar size rather than full resolution: the bytes are
    /// capped at 256 KiB but a 512px JPEG still costs ~1MB decoded, and
    /// every family member's face is held at once.
    private nonisolated static func downsample(_ data: Data) -> Image? {
        let source = CGImageSourceCreateWithData(data as CFData, [
            kCGImageSourceShouldCache: false,
        ] as CFDictionary)
        guard let source else { return nil }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceShouldCacheImmediately: true,
            kCGImageSourceThumbnailMaxPixelSize: Self.avatarMaxPixels,
        ]
        guard let thumbnail = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else {
            return nil
        }
        return Image(uiImage: UIImage(cgImage: thumbnail))
    }
}
