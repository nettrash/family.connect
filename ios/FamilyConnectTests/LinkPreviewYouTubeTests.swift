//
//  LinkPreviewYouTubeTests.swift
//  FamilyConnectTests
//
//  #50: a YouTube link showed no card. Not a parse bug, not a User-Agent
//  problem, not a consent wall — youtube.com answers this app's bot
//  User-Agent with a plain 200 and complete og: tags. The tags just sit
//  ~706KB into a 1.3MB page, behind ~700KB of inline player JSON, and
//  the fetcher stopped at 256KB while the parser stopped at 200K
//  characters. The first 256KB of that page contain no og:title and not
//  even a <title>, so the parser correctly refused to build a card and
//  the bubble showed a bare link.
//
//  These tests run on the captured bytes (LinkPreviewFixtures), never on
//  the network, and pin all three halves of the fix: the parser reaching
//  past the old scan limit, the fetch reading far enough to get there,
//  and the head stop that keeps the bigger cap from costing ordinary
//  pages anything.
//
//  Android mirror: LinkPreviewYouTubeTest.kt.
//

import Foundation
import Testing
@testable import FamilyConnect

@Suite("YouTube link previews")
struct LinkPreviewYouTubeTests {

    private let watchURL = URL(string: "https://www.youtube.com/watch?v=dQw4w9WgXcQ")!

    // MARK: - The page itself

    @Test("The captured page really does hide its tags past a quarter megabyte")
    func fixtureMatchesTheLivePage() throws {
        let html = try LinkPreviewFixtures.youTubeWatchPage()
        let bytes = Array(html.utf8)
        // Byte offsets, because that is the unit the fetcher's cap is in.
        #expect(offset(of: "<title", in: bytes) == LinkPreviewFixtures.YouTube.titleTagOffset)
        #expect(offset(of: "og:title", in: bytes) == LinkPreviewFixtures.YouTube.ogTitleOffset)
        #expect(offset(of: "</head", in: bytes) == LinkPreviewFixtures.YouTube.headEndOffset)
        // The old cap, spelled out: nothing usable inside it.
        #expect(LinkPreviewFixtures.YouTube.ogTitleOffset > 256 * 1024)
    }

    @Test("A watch page yields the full card")
    func watchPageParses() throws {
        let html = try LinkPreviewFixtures.youTubeWatchPage()
        let preview = try #require(LinkPreviewParser.parse(html: html, pageURL: watchURL))
        #expect(preview.title == LinkPreviewFixtures.YouTube.title)
        #expect(preview.siteName == "YouTube")
        #expect(preview.imageURL?.absoluteString == LinkPreviewFixtures.YouTube.imageURL)
        #expect(preview.description?.hasPrefix("The official video for") == true)
    }

    @Test("The old 200K scan limit is what made the card vanish")
    func theOldScanLimitMissedEverything() throws {
        let html = try LinkPreviewFixtures.youTubeWatchPage()
        // Exactly what the parser used to see, and it is not enough for
        // even the <title> fallback — so the state was .unavailable and
        // the bubble drew a bare link.
        #expect(LinkPreviewParser.parse(html: String(html.prefix(200_000)), pageURL: watchURL) == nil)
        // And what it sees now.
        #expect(LinkPreviewParser.parse(html: String(html.prefix(LinkPreviewParser.scanLimit)),
                                        pageURL: watchURL) != nil)
    }

    @Test("The scan limit is never below the fetch cap")
    func scanLimitCoversTheFetch() {
        // A page is UTF-8: it can only ever decode to FEWER characters
        // than it has bytes, so a scan limit at least as large as the
        // byte cap always sees everything that was paid for. Raising one
        // alone re-opens #50.
        #expect(LinkPreviewParser.scanLimit >= 1024 * 1024)
    }

    // MARK: - The fetch

    @Test("The loader fetches deep enough to build the card")
    @MainActor
    func loaderBuildsTheCard() async throws {
        let host = "youtube-preview.test"
        let page = try LinkPreviewFixtures.youTubeWatchPage()
        let image = TestImages.solid(width: 64, height: 36)
        StubURLProtocol.register(host: host) { _ in
            StubResponse(status: 200, headers: ["Content-Type": "text/html; charset=utf-8"],
                         body: Data(page.utf8))
        }
        // The og:image lives on a different host, and an unregistered
        // host would fall through to the real network.
        StubURLProtocol.register(host: "i.ytimg.com") { _ in
            StubResponse(status: 200, headers: ["Content-Type": "image/jpeg"], body: image)
        }
        defer {
            StubURLProtocol.unregister(host: host)
            StubURLProtocol.unregister(host: "i.ytimg.com")
        }

        let loader = LinkPreviewLoader(session: StubURLProtocol.makeSession())
        let url = URL(string: "https://\(host)/watch?v=dQw4w9WgXcQ")!
        #expect(loader.state(for: url) == .loading)
        let preview = try await settled(loader, url)
        #expect(preview.title == LinkPreviewFixtures.YouTube.title)
        #expect(preview.siteName == "YouTube")
        // A PARTIAL card would mean the image fetch failed, not the parse.
        #expect(loader.image(for: preview.url) != nil)
    }

    @Test("Tags below </head> are not read, and that is the deal")
    @MainActor
    func theReadReallyStopsAtTheHead() async throws {
        // The price of stopping at </head>: a page that puts its og:
        // tags in the BODY loses its card, where the old flat 256K read
        // would have found them. It is the documented contract of the
        // parser ("It reads the <head> only"), no page in the sample
        // does it, and it is what keeps a 1MB ceiling from meaning a 1MB
        // download. Pinned so the trade stays a decision, not an
        // accident.
        let host = "head-stop.test"
        let page = "<html><head><meta name=\"nothing\" content=\"x\"></head><body>"
            + "<meta property=\"og:title\" content=\"Too Late\"></body></html>"
        StubURLProtocol.register(host: host) { _ in
            StubResponse(status: 200, headers: ["Content-Type": "text/html"], body: Data(page.utf8))
        }
        defer { StubURLProtocol.unregister(host: host) }

        let loader = LinkPreviewLoader(session: StubURLProtocol.makeSession())
        let url = URL(string: "https://\(host)/a")!
        _ = loader.state(for: url)
        await #expect(throws: Settling.unavailable) { try await settled(loader, url) }
    }

    @Test("The 1MB ceiling still holds when a page never closes its head")
    @MainActor
    func capStillApplies() async throws {
        let host = "no-head-end.test"
        // No </head> and no <body>, so only the byte cap can stop this —
        // and the og:title deliberately sits past it.
        let page = "<html><head><script>"
            + String(repeating: "a", count: 1_100_000)
            + "</script><meta property=\"og:title\" content=\"Past The Cap\">"
        StubURLProtocol.register(host: host) { _ in
            StubResponse(status: 200, headers: ["Content-Type": "text/html"], body: Data(page.utf8))
        }
        defer { StubURLProtocol.unregister(host: host) }

        let loader = LinkPreviewLoader(session: StubURLProtocol.makeSession())
        let url = URL(string: "https://\(host)/a")!
        _ = loader.state(for: url)
        await #expect(throws: Settling.unavailable) { try await settled(loader, url) }
    }

    /// Poll until the fetch settles — the loader publishes from a Task,
    /// and there is no completion to await.
    @MainActor
    private func settled(_ loader: LinkPreviewLoader, _ url: URL) async throws -> LinkPreview {
        for _ in 0..<200 {
            switch loader.state(for: url) {
            case .loaded(let preview): return preview
            case .unavailable: throw Settling.unavailable
            default: try await Task.sleep(for: .milliseconds(25))
            }
        }
        throw Settling.timedOut
    }

    private enum Settling: Error, Equatable {
        /// The loader gave up: the fetch failed, or the parse found no
        /// title in what it read — which is exactly the #50 symptom.
        case unavailable
        case timedOut
    }

    // MARK: - The head stop

    @Test("The read stops at the end of the real page's head")
    func headStopLandsOnTheRealHeadEnd() throws {
        let bytes = Array(try LinkPreviewFixtures.youTubeWatchPage().utf8)
        // The `>` of `</head>`: the byte that proves the name ended.
        #expect(stopIndex(in: bytes) == LinkPreviewFixtures.YouTube.headEndOffset + 6)
    }

    @Test("A tag whose name merely starts with body or head is not the end")
    func headStopIgnoresLookalikes() {
        #expect(stopIndex(in: Array("<header><bodyguard>".utf8)) == nil)
        #expect(stopIndex(in: Array("<head><meta><bodyx".utf8)) == nil)
    }

    @Test("A page that omits </head> still stops at <body>")
    func headStopFallsBackToBody() {
        let html = "<head><title>x</title><body class=\"a\">"
        // 22 is the "<" of <body>; +5 is the byte after its name.
        #expect(stopIndex(in: Array(html.utf8)) == 27)
    }

    @Test("An uppercase head end tag stops the read too")
    func headStopFoldsCase() {
        #expect(stopIndex(in: Array("<HEAD><TITLE>x</TITLE></HEAD>".utf8)) == 28)
    }

    @Test("A page with neither sentinel is read to the end")
    func headStopNeverFires() {
        #expect(stopIndex(in: Array("<html><meta name=\"a\" content=\"b\">".utf8)) == nil)
    }

    // MARK: - Helpers

    /// Index of the byte at which HeadEndDetector says the head is over.
    private func stopIndex(in bytes: [UInt8]) -> Int? {
        var detector = HeadEndDetector()
        for index in bytes.indices {
            if detector.consume(bytes[index]) { return index }
        }
        return nil
    }

    /// Byte offset of `needle`, compared byte-for-byte — a Character
    /// offset would not be the unit the fetcher's cap counts in.
    private func offset(of needle: String, in bytes: [UInt8]) -> Int? {
        let pattern = Array(needle.utf8)
        guard bytes.count >= pattern.count else { return nil }
        for start in 0...(bytes.count - pattern.count) {
            var matched = 0
            while matched < pattern.count, bytes[start + matched] == pattern[matched] {
                matched += 1
            }
            if matched == pattern.count { return start }
        }
        return nil
    }
}
