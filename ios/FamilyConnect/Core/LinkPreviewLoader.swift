//
//  LinkPreviewLoader.swift
//  FamilyConnect
//
//  Fetches the card shown under a message's first web link.
//
//  PRIVACY, up front: this is the one place the app talks to a host the
//  family does not own. Every device that displays a linked message
//  contacts that link — chosen deliberately (the alternative designs
//  need the server or the wire format to carry the preview), and
//  switchable off in Settings, which is why every entry point checks
//  AppSettings.linkPreviewsEnabled before asking for anything. Requests
//  are stripped down accordingly: no cookies, no credentials, no
//  referrer, and a GET that stops at the end of the page's <head>
//  (or after `maxBytes`, whichever comes first).
//
//  One loader for the app, so a link posted in a busy chat is fetched
//  once no matter how many bubbles render it. Results — including
//  failures, which are cached so a dead link is not retried on every
//  scroll — live in memory only: a preview is derived data, and
//  persisting it would outlive the message it belongs to.
//
//  `generation` ticks whenever a fetch lands. ConversationView watches
//  it because a card appearing changes bubble heights, and the thread
//  has to re-pin to the bottom when that happens while the reader is
//  there (same reason reaction catch-up re-pins).
//
//  Android counterpart: android/…/data/net/LinkPreviewRepository.kt.
//

import Foundation
import ImageIO
import SwiftUI

@MainActor @Observable
final class LinkPreviewLoader {

    /// What the UI knows about one link.
    enum State: Equatable {
        case loading
        case loaded(LinkPreview)
        case unavailable
    }

    /// Bumped every time a fetch settles, so views that need to react to
    /// "a card just appeared" have something to observe.
    private(set) var generation = 0

    private var states: [URL: State] = [:]
    private var inFlight: Set<URL> = []
    private var images: [URL: Image] = [:]
    /// Insertion order of settled entries, so eviction can drop the
    /// oldest instead of everything.
    private var order: [URL] = []

    /// Page bytes read before giving up. The read normally stops long
    /// before this, at the end of <head> (see limitedData) — the ceiling
    /// is only for a page that never closes its head.
    ///
    /// It was 256K, and that is precisely why a YouTube link showed no
    /// card at all (#50): youtube.com/watch ships ~700K of inline player
    /// JSON ahead of its og: tags, so the first 256K contain no og:title
    /// and not even a <title>, and the parser correctly refused to build
    /// a card out of nothing. Measured on a real watch page: <title> at
    /// 704,923, og:title at 706,842, </head> at 715,108 of 1,310,787.
    /// 1MB clears that with room, and the head stop is what keeps the
    /// raise from costing anything on ordinary pages — most close their
    /// head inside 30K, so they now transfer far LESS than they used to.
    private static let maxBytes = 1024 * 1024
    private static let maxImageBytes = 4 * 1024 * 1024
    /// Longest edge the card image is decoded to — the card is 120pt
    /// tall, so anything beyond this is memory nobody sees.
    private nonisolated static let maxImagePixels = 1200
    private static let maxCacheEntries = 200

    private let session: URLSession

    init(session: URLSession? = nil) {
        if let session {
            self.session = session
        } else {
            let configuration = URLSessionConfiguration.ephemeral
            configuration.timeoutIntervalForRequest = 10
            configuration.timeoutIntervalForResource = 20
            configuration.httpCookieStorage = nil
            configuration.httpShouldSetCookies = false
            configuration.httpAdditionalHeaders = [
                "Accept": "text/html,application/xhtml+xml",
                // Bots get the metadata without the consent walls and
                // personalization that a browser UA invites.
                "User-Agent": "FamilyConnect/1.0 (+link-preview; like WhatsApp)",
            ]
            self.session = URLSession(configuration: configuration)
        }
    }

    /// Current knowledge about `url`, kicking off a fetch the first time
    /// it is asked for. Returns nil when previews are switched off.
    func state(for url: URL) -> State? {
        guard AppSettings.linkPreviewsEnabled else { return nil }
        if let known = states[url] { return known }
        load(url)
        return .loading
    }

    /// The decoded card image, once it has arrived.
    func image(for url: URL) -> Image? {
        images[url]
    }

    private func load(_ url: URL) {
        guard !inFlight.contains(url) else { return }
        // https only, on both platforms. Plain http would be blocked by
        // ATS here anyway (so the card would appear on Android and not
        // on iOS for the same message), and silently fetching a
        // cleartext URL somebody else chose is the wrong default for
        // the one request this app makes off the family's own server.
        guard url.scheme?.lowercased() == "https" else {
            states[url] = .unavailable
            return
        }
        inFlight.insert(url)
        states[url] = .loading
        Task { [weak self] in
            guard let self else { return }
            let preview = await Self.fetchPreview(url, session: session)
            // The image is fetched BEFORE publishing, so a card appears
            // in one step. Landing text first and the image 200ms later
            // grows the bubble twice, and every growth is a scroll
            // correction the thread has to make (LinkPreviewCard).
            var image: Image?
            if let imageURL = preview?.imageURL {
                image = await Self.fetchImage(imageURL, session: session)
            }
            finish(url, preview: preview, image: image)
        }
    }

    private func finish(_ url: URL, preview: LinkPreview?, image: Image?) {
        inFlight.remove(url)
        evictIfNeeded()
        states[url] = preview.map(State.loaded) ?? .unavailable
        // Keyed by the PREVIEW's url, which is where redirects landed —
        // the card reads it back by that same key, and keying by the
        // requested url means a redirecting link never shows its image.
        if let image, let preview {
            images[preview.url] = image
        }
        order.append(url)
        generation &+= 1
    }

    /// Drop the oldest entries when the table fills, rather than wiping
    /// it: a wipe makes every card currently on screen vanish at once,
    /// and on Android the equivalent never comes back.
    private func evictIfNeeded() {
        guard states.count >= Self.maxCacheEntries else { return }
        let dropping = order.prefix(Self.maxCacheEntries / 2)
        for url in dropping {
            states[url] = nil
            images[url] = nil
        }
        order.removeFirst(min(dropping.count, order.count))
    }

    // MARK: - Network (off the main actor)

    private nonisolated static func fetchPreview(_ url: URL, session: URLSession) async -> LinkPreview? {
        var request = URLRequest(url: url)
        request.httpShouldHandleCookies = false
        request.setValue(nil, forHTTPHeaderField: "Referer")
        guard let (data, response) = try? await limitedData(
            for: request, session: session, cap: maxBytes, stoppingAtEndOfHead: true) else {
            return nil
        }
        guard let http = response as? HTTPURLResponse, (200..<300).contains(http.statusCode) else {
            return nil
        }
        // Only HTML can carry the tags; anything else (a PDF, an image,
        // a download) would just be bytes burned.
        let mime = http.mimeType?.lowercased() ?? ""
        guard mime.contains("html") || mime.isEmpty else { return nil }

        let encoding = http.textEncodingName
            .map { String.Encoding(ianaName: $0) } ?? .utf8
        let html = String(data: data, encoding: encoding)
            ?? String(data: data, encoding: .utf8)
            ?? String(decoding: data, as: UTF8.self)
        // Redirects land on `http.url`; resolve relative images and the
        // host label against where we actually ended up.
        return LinkPreviewParser.parse(html: html, pageURL: http.url ?? url)
    }

    private nonisolated static func fetchImage(_ url: URL, session: URLSession) async -> Image? {
        var request = URLRequest(url: url)
        request.httpShouldHandleCookies = false
        guard let (data, response) = try? await limitedData(
            for: request, session: session, cap: maxImageBytes) else { return nil }
        guard let http = response as? HTTPURLResponse, (200..<300).contains(http.statusCode) else {
            return nil
        }
        return downsampled(data).map { PlatformImage.view($0) }
    }

    /// Decode at roughly card width rather than full resolution. The
    /// 4MB byte cap is not a pixel cap — a 6000×4000 JPEG is well under
    /// it and still costs ~96MB decoded, and up to `maxCacheEntries` of
    /// those would be held at once.
    private nonisolated static func downsampled(_ data: Data) -> CGImage? {
        let source = CGImageSourceCreateWithData(data as CFData, [
            kCGImageSourceShouldCache: false,
        ] as CFDictionary)
        guard let source else { return nil }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceShouldCacheImmediately: true,
            kCGImageSourceThumbnailMaxPixelSize: maxImagePixels,
        ]
        guard let thumbnail = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else {
            return nil
        }
        return thumbnail
    }

    /// A GET that stops reading at `cap` bytes instead of trusting the
    /// far end's Content-Length — and, for a page, at the end of its
    /// <head> before that.
    ///
    /// The head stop is what pays for the 1MB `maxBytes`. Everything the
    /// parser reads lives in <head>, so the rest of a page is bytes
    /// burned on somebody's mobile data, and heads are small even when
    /// pages are not: measured over ten real sites, </head> lands at
    /// 347B (Hacker News), 6.8K (Vimeo), 9.5K (Wikipedia, in a 650K
    /// page), 13K (apple.com), 22K (GitHub), 31K (amazon.com, 762K
    /// page), 90K (BBC News), 353K (Reddit, 874K page), 619K (the
    /// Guardian, 1.28M page) and 715K (YouTube, 1.31M page). Seven of
    /// those ten now transfer less than the flat 256K they used to.
    ///
    /// The body arrives through a task delegate rather than
    /// `session.bytes(for:)`, and that is not a style preference.
    /// AsyncBytes is an AsyncSequence of UInt8 — one async element per
    /// BYTE — and measured against a local HTTP server it moves about
    /// 140KB/s: 1.9s to read the old 256K cap, 5.0s to reach YouTube's
    /// og: tags at 715K, 7.5s for 1MB, all of it iteration overhead
    /// rather than network. The same reads through this delegate take
    /// 8ms, 2ms and 1ms. At 1MB the old loop would have spent most of
    /// the session's 20s resource timeout on a phone doing nothing but
    /// stepping a Swift async iterator.
    private nonisolated static func limitedData(
        for request: URLRequest,
        session: URLSession,
        cap: Int,
        stoppingAtEndOfHead: Bool = false
    ) async throws -> (Data, URLResponse) {
        let reader = LimitedBodyReader(cap: cap, stoppingAtEndOfHead: stoppingAtEndOfHead)
        let task = session.dataTask(with: request)
        // Per-task delegate, so the caller's session — the injected stub
        // in tests, the ephemeral one in the app — needs no delegate of
        // its own.
        task.delegate = reader
        return try await withCheckedThrowingContinuation { continuation in
            reader.begin(continuation)
            task.resume()
        }
    }
}

/// Collects a response body in the chunks the network hands over,
/// stopping at `cap` bytes and — for a page — at the end of its <head>
/// before that, then cancelling the task so the rest is never
/// transferred.
///
/// The lock is not ceremony: the delegate callbacks run on the session's
/// own queue while `limitedData` waits on another thread entirely.
private nonisolated final class LimitedBodyReader: NSObject, URLSessionDataDelegate, @unchecked Sendable {

    private let cap: Int
    private let stoppingAtEndOfHead: Bool
    private let lock = NSLock()
    private var head = HeadEndDetector()
    private var collected = Data()
    private var response: URLResponse?
    private var continuation: CheckedContinuation<(Data, URLResponse), Error>?
    /// Set the moment we have enough. Cancelling a task does not unqueue
    /// the callbacks already on the delegate's queue, and without this a
    /// late chunk would append bytes from BELOW the head we stopped at.
    private var stopped = false

    init(cap: Int, stoppingAtEndOfHead: Bool) {
        self.cap = cap
        self.stoppingAtEndOfHead = stoppingAtEndOfHead
    }

    /// Handed the continuation BEFORE the task is resumed, so a response
    /// that arrives immediately still finds somebody to resume.
    func begin(_ continuation: CheckedContinuation<(Data, URLResponse), Error>) {
        lock.lock()
        self.continuation = continuation
        lock.unlock()
    }

    func urlSession(
        _ session: URLSession,
        dataTask: URLSessionDataTask,
        didReceive response: URLResponse,
        completionHandler: @escaping (URLSession.ResponseDisposition) -> Void
    ) {
        lock.lock()
        // Redirects land here too; the LAST response is where we ended
        // up, which is what relative images resolve against.
        self.response = response
        lock.unlock()
        completionHandler(.allow)
    }

    func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
        lock.lock()
        guard !stopped else {
            lock.unlock()
            return
        }
        var slice = data.prefix(max(0, cap - collected.count))
        var done = collected.count + slice.count >= cap
        if stoppingAtEndOfHead {
            // A plain synchronous walk of the chunk: nanoseconds a byte,
            // which is the whole point of not doing it asynchronously.
            var offset = 0
            for byte in slice {
                offset += 1
                if head.consume(byte) {
                    slice = slice.prefix(offset)
                    done = true
                    break
                }
            }
        }
        collected.append(slice)
        stopped = done
        lock.unlock()
        // Cancelling surfaces as an error in didCompleteWithError, where
        // a response already in hand means "enough", not "failed".
        if done { dataTask.cancel() }
    }

    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        lock.lock()
        let waiting = continuation
        continuation = nil
        let body = collected
        let response = self.response
        lock.unlock()
        guard let waiting else { return }
        if let response {
            waiting.resume(returning: (body, response))
        } else {
            waiting.resume(throwing: error ?? URLError(.badServerResponse))
        }
    }
}

/// Spots the end of a page's `<head>` in a byte stream, one byte at a
/// time, so a fetch can stop there without buffering the page first.
///
/// Deliberately dumber than an HTML tokenizer, and matched to the
/// scanner in LinkPreviewParser: ASCII-only case folding, and a match
/// counts only when the tag NAME ends there, so `<bodyguard>` is not the
/// body. `<body` is honoured as well as `</head`, because the head end
/// tag is optional in HTML and a page that omits it would otherwise be
/// read to the cap.
///
/// The cost of being dumb is a page that writes the literal text
/// `</head>` or `<body>` inside an inline script in its head: the fetch
/// stops early and the card is lost. That is rare enough to accept
/// (none of the ten pages measured above does it, YouTube included —
/// its inline JSON escapes every `<` as a \u003C escape), and the
/// alternative is a real tokenizer for a nicety feature.
///
/// Android counterpart: HeadEndScanner in LinkPreviewRepository.kt.
nonisolated struct HeadEndDetector {
    /// The last few folded bytes — one longer than the longest needle,
    /// because the byte AFTER the name is what proves the name ended.
    private var window: [UInt8] = []

    private static let head = Array("</head".utf8)
    private static let body = Array("<body".utf8)
    private static let windowSize = 7

    /// True the moment `byte` completes `</head` or `<body`.
    mutating func consume(_ byte: UInt8) -> Bool {
        window.append(Self.folded(byte))
        if window.count > Self.windowSize { window.removeFirst() }
        // The newest byte is the boundary; the needle sits just before it.
        guard let boundary = window.last, !Self.isNameCharacter(boundary) else { return false }
        return endsWithNeedle(Self.head) || endsWithNeedle(Self.body)
    }

    private func endsWithNeedle(_ needle: [UInt8]) -> Bool {
        // -1 for the boundary byte, which is not part of the name.
        let end = window.count - 1
        guard end >= needle.count else { return false }
        return Array(window[(end - needle.count)..<end]) == needle
    }

    private static func folded(_ byte: UInt8) -> UInt8 {
        (byte >= 65 && byte <= 90) ? byte + 32 : byte
    }

    private static func isNameCharacter(_ byte: UInt8) -> Bool {
        (byte >= 97 && byte <= 122) || (byte >= 48 && byte <= 57)
    }
}

nonisolated private extension String.Encoding {
    /// The IANA charset name from a response header, or UTF-8.
    init(ianaName name: String) {
        let cf = CFStringConvertIANACharSetNameToEncoding(name as CFString)
        guard cf != kCFStringEncodingInvalidId else {
            self = .utf8
            return
        }
        self = String.Encoding(rawValue: CFStringConvertEncodingToNSStringEncoding(cf))
    }
}
