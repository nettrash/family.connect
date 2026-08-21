//
//  EmojiCatalogTests.swift
//  FamilyConnectTests
//
//  Sanity over the embedded cross-platform emoji catalog: the full
//  picker must never offer an entry the server would reject (the
//  32-byte UTF-8 limit), and the grid's ForEach identity requires
//  uniqueness within each category — the canonical source JSON once
//  carried a duplicate 💢 in Hearts, which these tests now pin out.
//

import Foundation
import Testing
@testable import FamilyConnect

@Suite("Emoji catalog")
struct EmojiCatalogTests {

    @Test("the catalog has categories and none is empty")
    func categoriesNonEmpty() {
        #expect(!EmojiCatalog.categories.isEmpty)
        for category in EmojiCatalog.categories {
            #expect(!category.emoji.isEmpty, "\(category.name) is empty")
        }
    }

    @Test("every entry is a non-empty string")
    func entriesNonEmpty() {
        for category in EmojiCatalog.categories {
            #expect(category.emoji.allSatisfy { !$0.isEmpty }, "\(category.name) has an empty entry")
        }
    }

    @Test("entries are unique within their category (ForEach identity)")
    func entriesUniqueWithinCategory() {
        for category in EmojiCatalog.categories {
            var seen: Set<String> = []
            let duplicates = category.emoji.filter { !seen.insert($0).inserted }
            #expect(duplicates.isEmpty, "\(category.name) repeats \(duplicates)")
        }
    }

    @Test("category names are unique (section identity)")
    func categoryNamesUnique() {
        let names = EmojiCatalog.categories.map(\.name)
        #expect(Set(names).count == names.count)
    }

    @Test("every entry fits the server's 32-byte UTF-8 limit")
    func entriesFitServerLimit() {
        for category in EmojiCatalog.categories {
            for emoji in category.emoji {
                #expect(emoji.utf8.count <= 32, "\(category.name): \(emoji) is \(emoji.utf8.count) bytes")
            }
        }
    }

    @Test("the quick set is also within the server limit")
    func quickReactionsFitServerLimit() {
        for emoji in MessagePresentation.quickReactions {
            #expect(emoji.utf8.count <= 32)
        }
    }

    @Test("the double-tap reaction is a quick reaction within the limit")
    func doubleTapReactionValid() {
        #expect(MessagePresentation.doubleTapReaction.utf8.count <= 32)
        // Inside the quick set so the capsule shows it selected after
        // a double-tap. Android pins the same value.
        #expect(MessagePresentation.quickReactions.contains(MessagePresentation.doubleTapReaction))
    }
}
