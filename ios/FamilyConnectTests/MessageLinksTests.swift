//
//  MessageLinksTests.swift
//  FamilyConnectTests
//
//  Pins the tappable-data layer over message bodies: which categories
//  are detected (web URLs, emails, phone numbers — the same three
//  Android detects), what URL each category opens (scheme'd web,
//  mailto:, sanitized tel:), and how link runs are styled (underlined;
//  forced white on own bubbles where the accent color would drown in
//  the tint). The detector itself is NSDataDetector, so these vectors
//  deliberately use robust, locale-independent shapes rather than
//  pinning the platform's whole grammar.
//

import Foundation
import SwiftUI
import Testing
@testable import FamilyConnect

@Suite("Message link detection")
struct MessageLinksTests {

    private func linkURLs(in text: String) -> [URL] {
        MessageLinks.attributedBody(text, isMine: false).runs.compactMap(\.link)
    }

    @Test("web URLs become tappable links")
    func webURLs() {
        let urls = linkURLs(in: "release notes at https://example.com/notes?v=1 today")
        #expect(urls.map(\.absoluteString) == ["https://example.com/notes?v=1"])
    }

    @Test("schemeless www hosts gain a scheme")
    func schemelessWeb() {
        let urls = linkURLs(in: "see www.example.com")
        #expect(urls.count == 1)
        #expect(urls.first?.host() == "www.example.com")
        #expect(urls.first?.scheme?.isEmpty == false)
    }

    @Test("email addresses become mailto links")
    func emails() {
        let urls = linkURLs(in: "write to nettrash@nettrash.me please")
        #expect(urls.map(\.absoluteString) == ["mailto:nettrash@nettrash.me"])
    }

    @Test("phone numbers become sanitized tel links")
    func phoneNumbers() {
        let urls = linkURLs(in: "call +1 (555) 123-4567 tonight")
        #expect(urls.map(\.absoluteString) == ["tel:+15551234567"])
    }

    @Test("vanity letters dial as their keypad digits")
    func vanityNumbers() {
        let urls = linkURLs(in: "call 1-800-GOT-JUNK now")
        #expect(urls.map(\.absoluteString) == ["tel:18004685865"])
    }

    @Test("extensions ride separately instead of corrupting the number")
    func phoneExtensions() {
        // The detector normalizes "x89" to a ';' suffix; naive digit
        // concatenation would dial 555123456789.
        let urls = linkURLs(in: "call 555-123-4567 x89")
        #expect(urls.map(\.absoluteString) == ["tel:5551234567;ext=89"])
    }

    @Test("plain text yields no links")
    func plainText() {
        #expect(linkURLs(in: "just words, nothing else. really").isEmpty)
        #expect(linkURLs(in: "😀😀").isEmpty)
        #expect(linkURLs(in: "").isEmpty)
    }

    @Test("multiple matches keep their order")
    func multipleMatches() {
        let urls = linkURLs(in: "docs: https://example.com and mail nettrash@nettrash.me")
        #expect(urls.map(\.scheme) == ["https", "mailto"])
    }

    @Test("link runs are underlined; own bubbles force white")
    func linkStyling() {
        let mine = MessageLinks.attributedBody("see https://example.com", isMine: true)
        let mineLink = mine.runs.first { $0.link != nil }
        #expect(mineLink?.underlineStyle == .single)
        #expect(mineLink?.foregroundColor == .white)

        let theirs = MessageLinks.attributedBody("see https://example.com", isMine: false)
        let theirsLink = theirs.runs.first { $0.link != nil }
        #expect(theirsLink?.underlineStyle == .single)
        // Theirs keeps the default accent — no forced color.
        #expect(theirsLink?.foregroundColor == nil)
    }

    @Test("non-link text keeps no link attribute")
    func surroundingTextUnaffected() {
        let attributed = MessageLinks.attributedBody("before https://example.com after", isMine: false)
        let plainRuns = attributed.runs.filter { $0.link == nil }
        #expect(!plainRuns.isEmpty)
        #expect(plainRuns.allSatisfy { $0.underlineStyle == nil })
    }

    // MARK: - The tokens the assistant acts on

    /// Every run this body draws with the assistant's own mark on it.
    ///
    /// Selected by the ACCENT COLOUR, not by the emphasis: markdown's own
    /// bold carries `.stronglyEmphasized` too, so a body like a `/draw`
    /// wrapped in asterisks would answer "marked" on the strength of its
    /// own markup and this test would pass whatever the grammar did. The
    /// colour is applied by `highlightMentions` and by nothing else on a
    /// bubble that is not mine.
    private func marked(_ body: String) -> [String] {
        let attributed = MessageLinks.attributedBody(body, isMine: false)
        return attributed.runs.compactMap { run in
            guard run.foregroundColor == .accentColor else { return nil }
            return String(attributed[run.range].characters)
        }
    }

    /// THE MARK COMES OFF THE RAW BODY, not off the rendered one.
    ///
    /// The server reads `/draw` from the body as typed. Everything else in
    /// `MessageLinks` reads the body as DRAWN, because markdown deletes
    /// characters and every link offset has to index what is on screen.
    /// For this one token the two strings are not interchangeable:
    /// `**/draw** a cat` renders to `/draw a cat`, so a mark computed from
    /// the rendered text sits on a picture request the server — looking at
    /// `**/draw**` — reads as an ordinary message and never acts on. A
    /// family would watch a marked request go unanswered with no way to
    /// tell why, which is the exact failure this grammar exists to prevent.
    ///
    /// Android's `MessageLinksTest.theDrawMarkIsDecidedFromTheRawBody`
    /// pins the same bodies.
    @Test("the picture mark is decided from the raw body, not the rendered one")
    func drawMarkComesFromTheRawBody() {
        // Markdown ahead of the token: the server sees the markup, so
        // there is no request and nothing may be marked.
        #expect(marked("**/draw** a cat").isEmpty)
        #expect(marked("*/draw* a cat").isEmpty)
        #expect(marked("`/draw` a cat").isEmpty)
        #expect(marked("~~/draw~~ a cat").isEmpty)
        #expect(marked("# /draw a cat").isEmpty)
        #expect(marked("- /draw a cat").isEmpty)
        // And a real request still is one, markdown in the PROMPT and all
        // — nothing has been removed ahead of the token.
        #expect(marked("/draw a cat") == ["/draw"])
        #expect(marked("/draw a **fluffy** cat") == ["/draw"])
        #expect(marked("@ai /draw a cat") == ["@ai", "/draw"])
    }

    /// `/draw` counts at the very beginning of the WHOLE body and nowhere
    /// else, so a paragraph that follows a table cannot start one — even
    /// though it is the start of its own block's offset space.
    @Test("only the first block may carry the picture mark")
    func onlyTheFirstBlockCarriesThePictureMark() {
        let body = "| a |\n| --- |\n| 1 |\n/draw a cat"
        let blocks = MessageLinks.blocks(body, isMine: false)
        let marks: [String] = blocks.flatMap { block -> [String] in
            guard case .text(let attributed) = block else { return [] }
            return attributed.runs.compactMap { run in
                guard run.foregroundColor == .accentColor else { return nil }
                return String(attributed[run.range].characters)
            }
        }
        #expect(marks.isEmpty, "got \(marks)")
    }
}
