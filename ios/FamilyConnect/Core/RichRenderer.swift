//
//  RichRenderer.swift
//  FamilyConnect
//
//  Formulas, diagrams and graphs, drawn into pictures for the bubble
//  (issue #97, docs/protocol.md "Rich blocks", and the design in
//  docs/capitals-and-rich-markdown-2026-10-17.md, "The native host").
//
//  EVERY client draws with the same files — KaTeX, Mermaid, Viz.js, md's
//  plotter and `fc-rich.js`, copied byte for byte into `Rich/` — so a
//  formula looks the same on a phone, a Mac, Android, Windows and the web.
//  A native client cannot run them itself; it hands one block at a time to
//  a hidden web view (RichWebHost) and keeps what comes back as a PNG.
//
//  This file is everything around the web view:
//
//  - `RichJob`, the one object `fcRich.draw` takes, built from a parsed
//    block and the bubble's look (the design doc's job table);
//  - `RichSlot`, one per job, OBSERVABLE ON ITS OWN, so a drawing landing
//    re-renders the one bubble that asked for it rather than every bubble
//    in the 60-row window;
//  - the cache: slots in memory (a small LRU), PNGs on disk in Caches,
//    keyed by SHA-256 of `fcRich.version` and every job field. A FAILURE
//    is remembered in memory only, so a newer engine gets another try;
//  - `generation`, which the chat watches the way it watches link-preview
//    cards: a drawing landing grows a bubble, and a bubble growing above
//    the viewport would slide what the reader is looking at.
//
//  Nothing here logs a source or an engine's message — an engine's error
//  quotes the formula, and a formula is message text.
//

import CryptoKit
import Foundation
import ImageIO
import Observation
import os
import SwiftUI
import UniformTypeIdentifiers

/// One block to draw, exactly as `fcRich.draw` takes it (fc-rich.js's
/// header, and the job table in the design doc).
nonisolated struct RichJob: Hashable, Sendable {
    /// `math` (a display formula), `math-inline`, `mermaid`, `graphviz`,
    /// `plot` — the parsed block's kind word, with `math-inline` for a
    /// formula inside a line.
    let kind: String
    /// Graphviz's layout; nil for every other kind.
    let engine: String?
    let source: String
    let dark: Bool
    /// The ink: the bubble's text colour for a formula, the card's for a
    /// diagram or a graph.
    let color: String
    /// `transparent` for a formula; the card for a diagram or a graph.
    let background: String
    /// The bubble's body text size, in points (= CSS pixels).
    let fontSize: Double

    var isFormula: Bool { kind == "math" || kind == "math-inline" }

    /// A rich block of the bubble's.
    init(block: MessageMarkdown.RichBlock, look: RichLook) {
        kind = block.kind.rawValue
        engine = block.engine
        source = block.source
        dark = look.dark
        fontSize = look.fontSize
        if block.kind == .math {
            color = look.formulaColor
            background = "transparent"
        } else {
            // The card, in fixed colours: a diagram's own palette (Mermaid's
            // theme) is chosen against these, not against the balloon.
            color = look.dark ? "#F2F2F7" : "#1C1C1E"
            background = look.dark ? "#1C1C1E" : "#FFFFFF"
        }
    }

    /// A formula inside a line.
    init(inlineFormula source: String, look: RichLook) {
        kind = "math-inline"
        engine = nil
        self.source = source
        dark = look.dark
        color = look.formulaColor
        background = "transparent"
        fontSize = look.fontSize
    }

    /// What `fcRich.draw` receives. Plain property-list values, which is
    /// what WebKit turns into a JavaScript object.
    var arguments: [String: Any] {
        var job: [String: Any] = [
            "kind": kind,
            "source": source,
            "dark": dark,
            "color": color,
            "background": background,
            "fontSize": fontSize,
        ]
        if let engine { job["engine"] = engine }
        return job
    }

    /// The cache key: SHA-256 of the renderer's version and every field —
    /// the design doc's list, so the same job is the same key on every
    /// launch, and a new engine (a new `VERSION`) is a new key for all of
    /// them.
    func cacheKey(version: String) -> String {
        let fields = [
            version, kind, engine ?? "", dark ? "1" : "0", color, background,
            String(fontSize), source,
        ]
        // A separator no field can hold, so ("ab", "c") and ("a", "bc")
        // never hash alike.
        let digest = SHA256.hash(data: Data(fields.joined(separator: "\u{0}").utf8))
        return digest.map { String(format: "%02x", $0) }.joined()
    }
}

/// How the bubble around a block looks — the three things a job takes
/// from it.
nonisolated struct RichLook: Hashable, Sendable {
    let dark: Bool
    /// The bubble's text colour, as CSS: white on my tinted balloon, the
    /// label colour on theirs.
    let formulaColor: String
    let fontSize: Double

    init(isMine: Bool, dark: Bool, fontSize: Double) {
        self.dark = dark
        self.fontSize = fontSize
        if isMine {
            formulaColor = "#FFFFFF"
        } else {
            // `.primary`, which is what the balloon's text is drawn in. On
            // a Mac that is `labelColor` — black or white at 85% — where
            // iOS's label is solid.
            #if os(macOS)
            formulaColor = dark ? "rgba(255,255,255,0.85)" : "rgba(0,0,0,0.85)"
            #else
            formulaColor = dark ? "#FFFFFF" : "#000000"
            #endif
        }
    }
}

/// A finished drawing: the picture, and its size and baseline in points
/// (CSS pixels).
nonisolated struct RichDrawing: @unchecked Sendable {
    let image: CGImage
    let width: Double
    let height: Double
    /// From the top to the formula's baseline; the height for a diagram.
    let baseline: Double

    /// Pixels per point — what `Image(_:scale:)` needs to draw it at its
    /// natural size.
    var scale: CGFloat { CGFloat(image.width) / CGFloat(max(width, 1)) }
}

/// What became of a job.
nonisolated enum RichOutcome: Sendable {
    case drawn(RichDrawing)
    /// The engine refused it, it drew nothing, or it took too long. The
    /// bubble shows the source, for good (until the next launch).
    case failed
}

/// One job's state, observed on its own (see the file header).
@MainActor @Observable
final class RichSlot {
    /// Nil until drawn or failed.
    var outcome: RichOutcome?
    /// A draw is under way; a second ask waits for it rather than queueing
    /// the same job twice.
    @ObservationIgnored var isDrawing = false

    init(outcome: RichOutcome? = nil) {
        self.outcome = outcome
    }
}

@MainActor @Observable
final class RichRenderer {

    static let shared = RichRenderer()

    /// Bumped every time a drawing lands — what the chat watches to keep a
    /// reader at the bottom when a bubble above grows (ConversationView,
    /// MacConversationView, the same hook as a link-preview card's).
    private(set) var generation = 0

    /// `fcRich.version`, read from the bundled fc-rich.js itself — so a
    /// cache hit needs no web view at all, and the key changes the moment
    /// the file does. Nil if the file is missing, and then nothing draws.
    @ObservationIgnored let version: String?

    @ObservationIgnored private var slots: [String: RichSlot] = [:]
    /// Least recently asked first.
    @ObservationIgnored private var order: [String] = []
    @ObservationIgnored private let host: RichWebHost
    @ObservationIgnored private let disk: RichDiskCache

    /// Drawings kept in memory. A chat's bounded row window holds a few
    /// dozen bubbles; this keeps several windows' worth so scrolling back
    /// and forth never shows a source again.
    static let memoryLimit = 300

    /// `directory` is a test seam, as AttachmentStore's is: the app passes
    /// nothing and gets Caches/rich.
    init(host: RichWebHost? = nil, directory: URL? = nil) {
        self.host = host ?? RichWebHost()
        version = Self.bundledVersion()
        let caches = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
        disk = RichDiskCache(directory: directory ?? caches.appendingPathComponent("rich", isDirectory: true))
    }

    /// The slot for `job` — made, and filled from memory, on first ask.
    /// Safe to call from a view body: it never draws and never publishes
    /// (`draw` does both, from a `.task`).
    func slot(for job: RichJob) -> RichSlot {
        guard let version else { return failedSlot }
        let key = job.cacheKey(version: version)
        if let slot = slots[key] {
            if order.last != key, let index = order.firstIndex(of: key) {
                order.remove(at: index)
                order.append(key)
            }
            return slot
        }
        let slot = RichSlot()
        slots[key] = slot
        order.append(key)
        if order.count > Self.memoryLimit {
            let evicted = order.removeFirst()
            slots[evicted] = nil
        }
        return slot
    }

    /// The one slot every job gets when the renderer's files are missing.
    @ObservationIgnored private let failedSlot = RichSlot(outcome: .failed)

    /// Draw `job` unless it is drawn, failed or being drawn: from disk when
    /// it was drawn before, else in the web view. Returns when the slot has
    /// its outcome.
    func draw(_ job: RichJob) async {
        guard let version else { return }
        let slot = slot(for: job)
        guard slot.outcome == nil, !slot.isDrawing else { return }
        slot.isDrawing = true
        defer { slot.isDrawing = false }
        let key = job.cacheKey(version: version)

        if let drawing = await disk.read(key) {
            land(.drawn(drawing), in: slot)
            return
        }
        let outcome = await host.draw(job)
        land(outcome, in: slot)
        if case .drawn(let drawing) = outcome {
            await disk.write(drawing, key: key)
        }
    }

    private func land(_ outcome: RichOutcome, in slot: RichSlot) {
        slot.outcome = outcome
        if case .drawn = outcome { generation &+= 1 }
    }

    /// The `VERSION` line of the bundled fc-rich.js.
    private static func bundledVersion() -> String? {
        guard let url = RichFiles.url(for: "fc-rich.js"),
              let script = try? String(contentsOf: url, encoding: .utf8)
        else {
            AppLog.ui.error("Rich renderer: fc-rich.js is missing from the bundle")
            return nil
        }
        guard let line = script.split(separator: "\n").first(where: { $0.contains("var VERSION = '") }),
              let open = line.firstIndex(of: "'"),
              let close = line[line.index(after: open)...].firstIndex(of: "'")
        else {
            AppLog.ui.error("Rich renderer: fc-rich.js has no VERSION line")
            return nil
        }
        return String(line[line.index(after: open)..<close])
    }
}

/// The bundled renderer files, by name.
///
/// The synchronized group puts `Rich/` and `Rich/fonts/` FLAT into the
/// app's resources (measured: `KaTeX_Main-Regular.woff2` sits beside
/// `Info.plist`), so a request for `fonts/KaTeX_….woff2` is answered by
/// file name. A `Rich` folder, should a future Xcode keep one, is looked in
/// first. Only the names below are ever served — nothing else in the
/// bundle is reachable from the web view.
nonisolated enum RichFiles {

    /// The page's own files, at the top of the folder.
    static let topLevel: Set<String> = [
        "rich.html", "fc-rich.js", "katex.min.css", "katex.min.js", "mhchem.min.js",
        "mermaid.min.js", "viz-global.js", "plot.js",
    ]

    /// Whether `name` may be served from `fonts/`: KaTeX's woff2 faces and
    /// nothing else.
    static func isFont(_ name: String) -> Bool {
        name.hasPrefix("KaTeX_") && name.hasSuffix(".woff2") && !name.contains("/")
            && !name.contains("..")
    }

    /// The file for a request path (`/rich.html`, `/fonts/KaTeX_Main-Regular.woff2`),
    /// or nil when the path is not one of ours.
    static func url(forPath path: String) -> URL? {
        let parts = path.split(separator: "/", omittingEmptySubsequences: true).map(String.init)
        switch parts.count {
        case 1 where topLevel.contains(parts[0]):
            return url(for: parts[0])
        case 2 where parts[0] == "fonts" && isFont(parts[1]):
            return url(for: parts[1], folder: "fonts")
        default:
            return nil
        }
    }

    static func url(for name: String, folder: String? = nil) -> URL? {
        let bundle = Bundle.main
        let nested = folder.map { "Rich/\($0)" } ?? "Rich"
        return bundle.url(forResource: name, withExtension: nil, subdirectory: nested)
            ?? bundle.url(forResource: name, withExtension: nil)
    }

    static func mimeType(for name: String) -> String {
        switch (name as NSString).pathExtension.lowercased() {
        case "html": return "text/html"
        case "js": return "text/javascript"
        case "css": return "text/css"
        case "woff2": return "font/woff2"
        default: return "application/octet-stream"
        }
    }
}

/// Drawings on disk: `<key>.png` and `<key>.json` (width, height and
/// baseline in CSS pixels) in Caches/rich. The system may empty Caches
/// whenever it likes, which only means a block is drawn again.
nonisolated final class RichDiskCache: Sendable {

    let directory: URL

    /// Kept under this many drawings; the oldest go first.
    static let limit = 2000

    init(directory: URL) {
        self.directory = directory
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        Task.detached(priority: .background) { [directory] in
            Self.prune(directory)
        }
    }

    private struct Metrics: Codable {
        let width: Double
        let height: Double
        let baseline: Double
    }

    func read(_ key: String) async -> RichDrawing? {
        let directory = directory
        return await Task.detached(priority: .userInitiated) { () -> RichDrawing? in
            let png = directory.appendingPathComponent(key).appendingPathExtension("png")
            let json = directory.appendingPathComponent(key).appendingPathExtension("json")
            guard let metricsData = try? Data(contentsOf: json),
                  let metrics = try? JSONDecoder().decode(Metrics.self, from: metricsData),
                  let source = CGImageSourceCreateWithURL(png as CFURL, nil),
                  let image = CGImageSourceCreateImageAtIndex(source, 0, nil)
            else { return nil }
            return RichDrawing(
                image: image, width: metrics.width, height: metrics.height, baseline: metrics.baseline)
        }.value
    }

    func write(_ drawing: RichDrawing, key: String) async {
        let directory = directory
        await Task.detached(priority: .utility) {
            let png = directory.appendingPathComponent(key).appendingPathExtension("png")
            let json = directory.appendingPathComponent(key).appendingPathExtension("json")
            guard let data = Self.pngData(drawing.image),
                  let metrics = try? JSONEncoder().encode(
                    Metrics(width: drawing.width, height: drawing.height, baseline: drawing.baseline))
            else { return }
            // The picture first: a reader finds the metrics only once the
            // picture they describe is whole.
            try? data.write(to: png, options: .atomic)
            try? metrics.write(to: json, options: .atomic)
        }.value
    }

    /// PNG, alpha and all.
    static func pngData(_ image: CGImage) -> Data? {
        let output = NSMutableData()
        guard let destination = CGImageDestinationCreateWithData(
            output, UTType.png.identifier as CFString, 1, nil)
        else { return nil }
        CGImageDestinationAddImage(destination, image, nil)
        guard CGImageDestinationFinalize(destination) else { return nil }
        return output as Data
    }

    /// Once per launch, in the background: over the limit, the oldest half
    /// goes.
    private static func prune(_ directory: URL) {
        let keys: [URLResourceKey] = [.contentModificationDateKey]
        guard let files = try? FileManager.default.contentsOfDirectory(
            at: directory, includingPropertiesForKeys: keys)
        else { return }
        let pngs = files.filter { $0.pathExtension == "png" }
        guard pngs.count > limit else { return }
        let dated = pngs.map { url -> (URL, Date) in
            let date = (try? url.resourceValues(forKeys: Set(keys)).contentModificationDate) ?? .distantPast
            return (url, date)
        }
        for (url, _) in dated.sorted(by: { $0.1 < $1.1 }).prefix(pngs.count - limit / 2) {
            try? FileManager.default.removeItem(at: url)
            try? FileManager.default.removeItem(at: url.deletingPathExtension().appendingPathExtension("json"))
        }
    }
}
