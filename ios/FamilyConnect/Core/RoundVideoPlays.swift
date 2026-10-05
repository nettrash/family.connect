//
//  RoundVideoPlays.swift
//  FamilyConnect
//
//  Which video messages THIS DEVICE has played — what the unplayed dot on a
//  circle is drawn from (#79, docs/audio-video-messages-2026-10-04.md, S5.2,
//  Decision 22; docs/protocol.md, "How it is drawn").
//
//  The device's own knowledge, and nothing more: never sent — there is no
//  played or watched receipt on the wire — kept PER ACCOUNT, so whoever signs
//  in next starts with every circle unplayed, wiped at sign-out, and
//  remembered for the newest 5 000 videos. "Newest" is by attachment id,
//  which the server hands out in order: the cap drops the oldest videos the
//  thread is least likely to scroll back to, never the one just played.
//
//  WHERE: `UserDefaults`, under a key naming the account (the server and the
//  user, `ParkedRecordings.accountKey`). A sorted id list a few tens of
//  kilobytes long at the cap; read once per account and cached.
//

import Foundation
import Observation

@MainActor
@Observable
final class RoundVideoPlays {

    /// The app's store.
    static let shared = RoundVideoPlays()

    /// How many played videos an account remembers (S5.2).
    nonisolated static let cap = 5_000

    /// Every key this store writes starts with this.
    nonisolated static let keyPrefix = "roundVideoPlays.v1."

    private let defaults: UserDefaults
    private let account: () -> String?
    private let cap: Int

    /// Bumped by every change — what a circle observes to drop its dot.
    private(set) var revision = 0
    @ObservationIgnored private var cache: (account: String, ids: Set<Int64>)?

    init(
        defaults: UserDefaults = .standard,
        account: @escaping () -> String? = {
            ParkedRecordings.accountKey(serverURL: AppSettings.serverURL, userID: AppSettings.currentUserID)
        },
        cap: Int = RoundVideoPlays.cap
    ) {
        self.defaults = defaults
        self.account = account
        self.cap = cap
    }

    /// Has this device played the video message whose attachment this is?
    /// Nobody signed in: nothing has been played.
    func isPlayed(_ attachmentID: Int64) -> Bool {
        _ = revision
        guard let key = account() else { return false }
        return ids(for: key).contains(attachmentID)
    }

    /// It played to its end on this device (S5.3: "at the end it returns to
    /// the poster and loses its dot"). A provisional (negative) id — the
    /// sender's own circle before the server named it — is never kept.
    func markPlayed(_ attachmentID: Int64) {
        guard attachmentID > 0, let key = account() else { return }
        var ids = ids(for: key)
        guard ids.insert(attachmentID).inserted else { return }
        if ids.count > cap {
            ids = Set(ids.sorted().suffix(cap))
        }
        defaults.set(ids.sorted(), forKey: Self.keyPrefix + key)
        cache = (key, ids)
        revision += 1
    }

    /// Sign-out: every account's record goes (S5.2).
    func removeAll() {
        for key in defaults.dictionaryRepresentation().keys where key.hasPrefix(Self.keyPrefix) {
            defaults.removeObject(forKey: key)
        }
        cache = nil
        revision += 1
    }

    private func ids(for key: String) -> Set<Int64> {
        if let cache, cache.account == key { return cache.ids }
        let stored = (defaults.array(forKey: Self.keyPrefix + key) as? [NSNumber]) ?? []
        let ids = Set(stored.map(\.int64Value))
        cache = (key, ids)
        return ids
    }
}
