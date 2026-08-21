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
}
