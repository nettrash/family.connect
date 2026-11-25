//
//  ReactionEmojiSearch.swift
//  FamilyConnect
//
//  Indexes bundled catalogue annotations and shortcodes for reaction search.
//

import Foundation

/// The Apple port of the shared reaction search in web/text/src/emoji_search.rs.
nonisolated struct ReactionEmojiSearch: Sendable {
    struct Match: Equatable, Identifiable, Sendable {
        let emoji: String
        let name: String
        var id: String { emoji }
    }

    private struct Annotation: Sendable {
        let name: String
        let normalizedName: String
        let terms: String
    }

    private static let catalog = {
        var seen: Set<String> = []
        return EmojiCatalog.categories.flatMap(\.emoji).filter { seen.insert($0).inserted }
    }()
    private static let byKey = Dictionary(uniqueKeysWithValues: catalog.map { (key($0), $0) })

    private let english: [String: Annotation]
    private let local: [String: Annotation]
    private let aliases: [String: [String]]

    static let bundled = Task.detached(priority: .userInitiated) {
        ReactionEmojiSearch(bundle: .main, language: PushRegistrationLogic.shownLanguage)
    }

    init(bundle: Bundle, language: String) {
        let locale = Self.tag(for: language)
        func annotations(_ tag: String) -> String? {
            guard let url = bundle.url(forResource: tag, withExtension: "tsv", subdirectory: "emoji-search")
            else { return nil }
            return try? String(contentsOf: url, encoding: .utf8)
        }
        let aliases: [[UInt8]: String]
        if bundle == .main {
            aliases = EmojiTyping.names
        } else {
            aliases = bundle.url(forResource: "emoji-names", withExtension: "tsv")
                .flatMap { try? Data(contentsOf: $0) }
                .map(EmojiTyping.parseNames) ?? [:]
        }
        self.init(english: annotations("en"), local: locale == "en" ? nil : annotations(locale),
                  locale: locale, aliases: aliases)
    }

    init(english: String?, local: String?, locale: String, aliases: [[UInt8]: String]) {
        self.english = english.flatMap { Self.parse($0, locale: "en") } ?? [:]
        self.local = local.flatMap { Self.parse($0, locale: locale) } ?? [:]

        var byEmoji: [String: [String]] = [:]
        for (alias, glyph) in aliases {
            guard let emoji = Self.catalogEmoji(for: glyph) else { continue }
            byEmoji[emoji, default: []].append(Self.normalize(String(decoding: alias, as: UTF8.self)))
        }
        self.aliases = byEmoji
    }

    func name(for emoji: String) -> String {
        guard let glyph = Self.catalogEmoji(for: emoji) else { return emoji }
        return local[glyph]?.name ?? english[glyph]?.name ?? emoji
    }

    func search(_ query: String) -> [Match] {
        let normalized = Self.normalize(query)
        guard !normalized.isEmpty else { return [] }

        let words = normalized.split(separator: " ")
        let exactGlyph = Self.catalogEmoji(for: normalized)
        var exact: [Match] = []
        var prefixes: [Match] = []
        var keywords: [Match] = []
        for emoji in Self.catalog {
            let local = local[emoji]
            let english = english[emoji]
            let aliases = aliases[emoji, default: []]
            let match = Match(emoji: emoji, name: local?.name ?? english?.name ?? emoji)
            if exactGlyph == emoji || local?.normalizedName == normalized
                || english?.normalizedName == normalized || aliases.contains(normalized) {
                exact.append(match)
            } else if local?.normalizedName.hasPrefix(normalized) == true
                || english?.normalizedName.hasPrefix(normalized) == true {
                prefixes.append(match)
            } else if words.allSatisfy({ word in
                local?.terms.contains(word) == true || english?.terms.contains(word) == true
                    || aliases.contains { $0.contains(word) }
            }) {
                keywords.append(match)
            }
        }
        return exact + prefixes + keywords
    }

    static func catalogEmoji(for emoji: String) -> String? {
        byKey[key(emoji)]
    }

    static func tag(for language: String) -> String {
        let parts = language.lowercased().split(whereSeparator: { $0 == "-" || $0 == "_" })
        switch parts.first {
        case "sr": return parts.contains("latn") ? "sr-Latn" : "sr"
        case "zh": return parts.contains("hant") ? "en" : "zh-Hans"
        default: return PushRegistrationLogic.pushLanguage(shown: parts.first.map(String.init))
        }
    }

    private static func key(_ emoji: String) -> String {
        emoji.replacingOccurrences(of: "\u{FE0F}", with: "", options: .literal)
    }

    private static func normalize(_ text: String) -> String {
        text.lowercased()
            .replacingOccurrences(of: "ß", with: "ss")
            .split { $0.isWhitespace || "_-:|".contains($0) }
            .joined(separator: " ")
    }

    private static func parse(_ tsv: String, locale: String) -> [String: Annotation]? {
        var annotations: [String: Annotation] = [:]
        var lines = tsv.utf8.split(separator: UInt8(ascii: "\n"), omittingEmptySubsequences: false)[...]
        if lines.last?.isEmpty == true { lines = lines.dropLast() }
        for rawLine in lines {
            let bytes = rawLine.last == UInt8(ascii: "\r") ? rawLine.dropLast() : rawLine
            let line = String(decoding: bytes, as: UTF8.self)
            if line.hasPrefix("#") { continue }
            let fields = line.split(separator: "\t", omittingEmptySubsequences: false)
            guard fields.count == 4, fields[0] == locale,
                  let emoji = catalogEmoji(for: String(fields[1])),
                  fields[1].utf8.elementsEqual(emoji.utf8),
                  !fields[2].isEmpty, !fields[3].isEmpty, annotations[emoji] == nil
            else { return nil }
            let name = String(fields[2])
            annotations[emoji] = Annotation(name: name, normalizedName: normalize(name),
                                            terms: normalize("\(name) \(fields[3])"))
        }
        return annotations.count == catalog.count ? annotations : nil
    }
}
