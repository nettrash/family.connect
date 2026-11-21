import Foundation

/// A device-local history of choices made in the full reaction picker.
nonisolated struct ReactionRecents {
    struct Account: Equatable, Sendable {
        let server: URL
        let userID: Int64

        static var current: Self? {
            guard let server = AppSettings.serverURL,
                  let userID = AppSettings.currentUserID, userID > 0
            else { return nil }
            return Self(server: server, userID: userID)
        }

        fileprivate var storageKey: String { "\(userID):\(server.absoluteString)" }
    }

    private static let key = "v1.reactionRecents"
    private static let limit = 24

    let defaults: UserDefaults

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
    }

    func choices(for account: Account) -> [String] {
        let stored = defaults.dictionary(forKey: Self.key)?[account.storageKey] as? [String] ?? []
        return Self.clean(stored)
    }

    func remember(_ emoji: String, for account: Account) {
        guard let glyph = ReactionEmojiSearch.catalogEmoji(for: emoji) else { return }
        var accounts = defaults.dictionary(forKey: Self.key) ?? [:]
        accounts[account.storageKey] = Self.clean([glyph] + choices(for: account))
        defaults.set(accounts, forKey: Self.key)
    }

    func clear() {
        defaults.removeObject(forKey: Self.key)
    }

    private static func clean(_ stored: [String]) -> [String] {
        var seen: Set<String> = []
        var recent: [String] = []
        for value in stored {
            guard let glyph = ReactionEmojiSearch.catalogEmoji(for: value),
                  seen.insert(glyph).inserted
            else { continue }
            recent.append(glyph)
            if recent.count == limit { break }
        }
        return recent
    }
}
