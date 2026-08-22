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
//  referrer, and a GET that stops reading after `maxBytes`.
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

    /// Page bytes read before giving up: <head> is all that is parsed,
    /// and this keeps a mis-typed link to a huge file cheap.
    private static let maxBytes = 256 * 1024
    private static let maxImageBytes = 4 * 1024 * 1024
    /// Longest edge the card image is decoded to — the card is 120pt
    /// tall, so anything beyond this is memory nobody sees.
    private static let maxImagePixels = 1200
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
        guard let (data, response) = try? await limitedData(for: request, session: session, cap: maxBytes) else {
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
    /// far end's Content-Length.
    private nonisolated static func limitedData(
        for request: URLRequest,
        session: URLSession,
        cap: Int
    ) async throws -> (Data, URLResponse) {
        let (stream, response) = try await session.bytes(for: request)
        var data = Data()
        data.reserveCapacity(min(cap, 64 * 1024))
        // Chunked, not byte-by-byte: AsyncBytes yields one element per
        // byte, and appending 256K times individually is pure overhead.
        var chunk = [UInt8]()
        chunk.reserveCapacity(16 * 1024)
        for try await byte in stream {
            chunk.append(byte)
            if chunk.count == chunk.capacity {
                data.append(contentsOf: chunk)
                chunk.removeAll(keepingCapacity: true)
            }
            if data.count + chunk.count >= cap { break }
        }
        data.append(contentsOf: chunk)
        return (data, response)
    }
}

private extension String.Encoding {
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
