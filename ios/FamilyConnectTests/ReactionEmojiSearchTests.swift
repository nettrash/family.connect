import Foundation
import Testing
@testable import FamilyConnect

@Suite("Reaction emoji search")
struct ReactionEmojiSearchTests {
    private final class FixtureBundleAnchor {}

    private struct Oracle: Decodable {
        struct Search: Decodable {
            struct Match: Decodable {
                let emoji: String
                let name: String
            }

            let id: String
            let locale: String
            let query: String
            let matches: [Match]
        }

        let search: [Search]
    }

    @Test func matchesSharedOracleVectors() throws {
        let bundle = Bundle(for: FixtureBundleAnchor.self)
        let url = try #require(bundle.url(forResource: "reaction-search-vectors", withExtension: "json"))
        let oracle = try JSONDecoder().decode(Oracle.self, from: Data(contentsOf: url))
        try #require(!oracle.search.isEmpty)
        for (locale, cases) in Dictionary(grouping: oracle.search, by: \.locale) {
            let index = ReactionEmojiSearch(bundle: .main, language: locale)
            for vector in cases {
                let actual = index.search(vector.query)
                let description = "\(locale): \(vector.id)"
                #expect(actual.map { Array($0.emoji.utf8) } == vector.matches.map { Array($0.emoji.utf8) },
                        "Emoji sequences and order for \(description)")
                #expect(actual.map { Array($0.name.utf8) } == vector.matches.map { Array($0.name.utf8) },
                        "Localized names for \(description)")
            }
        }
    }

    @Test(arguments: [
        ("en", "fire", "fire"),
        ("de", "Feuer", "Feuer"),
        ("es", "fuego", "fuego"),
        ("fr", "feu", "feu"),
        ("ja", "火", "火"),
        ("ru", "огонь", "огонь"),
        ("sr", "ватра", "ватра"),
        ("sr-Latn", "vatra", "vatra"),
        ("zh-Hans", "火", "火焰"),
    ])
    func findsFireInEveryBundledLanguage(language: String, query: String, name: String) {
        let index = ReactionEmojiSearch(bundle: .main, language: language)
        #expect(index.search(query).contains(.init(emoji: "🔥", name: name)))
        #expect(index.search("  :FIRE:  ").first?.emoji == "🔥")
    }

    @Test func matchesEnglishFallbackAndStableRanking() throws {
        let index = ReactionEmojiSearch(bundle: .main, language: "ru")
        #expect(index.search(":ICE:").first == .init(emoji: "🧊", name: "кубик льда"))
        #expect(index.search("like").first?.emoji == "👍")
        let ranked = index.search("ic").map(\.emoji)
        let ice = try #require(ranked.firstIndex(of: "🧊"))
        let smile = try #require(ranked.firstIndex(of: "😀"))
        #expect(ice < smile)
        #expect(index.search("ic").map(\.emoji) == ranked)
        #expect(Set(ranked).count == ranked.count)

        let german = ReactionEmojiSearch(bundle: .main, language: "de")
        #expect(german.search("FUSSBALL").contains { $0.emoji == "⚽" })
    }

    @Test func handlesEmptyQueriesAndReturnsExactCatalogSequences() throws {
        let index = ReactionEmojiSearch(bundle: .main, language: "en")
        let heart = try #require(index.search("❤").first)
        #expect(Array(heart.emoji.utf8) == Array("❤️".utf8))
        #expect(index.search("red heart").contains { $0.emoji == "❤️" })
        #expect(index.search("dizzy").filter { $0.emoji == "💫" }.count == 1)
        for query in ["", "  ", "__::", "fire nonexistentkeyword", "👍🏽"] {
            #expect(index.search(query).isEmpty)
        }
    }

    @Test func invalidLocalizedCatalogFallsBackToEnglish() throws {
        let url = try #require(Bundle.main.url(
            forResource: "en", withExtension: "tsv", subdirectory: "emoji-search"))
        let english = try String(contentsOf: url, encoding: .utf8)
        let russianURL = try #require(Bundle.main.url(
            forResource: "ru", withExtension: "tsv", subdirectory: "emoji-search"))
        let russian = try String(contentsOf: russianURL, encoding: .utf8)
        let lines = russian.split(whereSeparator: \.isNewline)
        let last = try #require(lines.last)
        let blankLine = lines.prefix(2).joined(separator: "\n")
            + "\n\n" + lines.dropFirst(2).joined(separator: "\n") + "\n"
        let invalid = [english, lines.dropLast().joined(separator: "\n"), russian + last + "\n", blankLine]
        for table in invalid {
            let index = ReactionEmojiSearch(english: english, local: table, locale: "ru", aliases: [:])
            #expect(index.search("fire").first == .init(emoji: "🔥", name: "fire"))
            #expect(index.search("огонь").isEmpty)
        }
    }

    @Test func preservesSerbianScriptsAndShipsTheUnicodeNotice() {
        #expect(ReactionEmojiSearch.tag(for: "sr-Latn-RS") == "sr-Latn")
        #expect(ReactionEmojiSearch.tag(for: "sr-Cyrl-RS") == "sr")
        #expect(ReactionEmojiSearch.tag(for: "zh-Hans-CN") == "zh-Hans")
        #expect(Bundle.main.url(
            forResource: "UNICODE-LICENSE", withExtension: "txt", subdirectory: "emoji-search") != nil)
    }

    @Test func recentsAreBoundedCanonicalAndScopedUntilSessionPurge() throws {
        let domain = "ReactionEmojiSearchTests.\(UUID())"
        let defaults = try #require(UserDefaults(suiteName: domain))
        defer { defaults.removePersistentDomain(forName: domain) }
        let history = ReactionRecents(defaults: defaults)
        let server = try #require(URL(string: "https://example.test/one"))
        let otherServer = try #require(URL(string: "https://example.test/two"))
        let me = ReactionRecents.Account(server: server, userID: 1)
        let otherUser = ReactionRecents.Account(server: server, userID: 2)
        let elsewhere = ReactionRecents.Account(server: otherServer, userID: 1)

        let glyphs = EmojiCatalog.categories[0].emoji.prefix(25)
        for emoji in glyphs { history.remember(emoji, for: me) }
        #expect(history.choices(for: me) == Array(glyphs.suffix(24).reversed()))

        history.remember("❤️", for: me)
        history.remember("🔥", for: me)
        history.remember("❤", for: me)
        history.remember("👍🏽", for: me)
        #expect(Array(history.choices(for: me).prefix(2)) == ["❤️", "🔥"])
        #expect(history.choices(for: me).count == 24)
        #expect(history.choices(for: me).filter { $0 == "❤️" }.count == 1)
        #expect(history.choices(for: otherUser).isEmpty)
        #expect(history.choices(for: elsewhere).isEmpty)

        history.remember("👍", for: otherUser)
        history.clear()
        #expect(history.choices(for: me).isEmpty)
        #expect(history.choices(for: otherUser).isEmpty)
    }
}
