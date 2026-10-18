//
//  RichRendererTests.swift
//  FamilyConnectTests
//
//  #97: the hidden web view really draws — a formula, an inline formula, a
//  Mermaid diagram, two Graphviz layouts and a plot, through the bundled
//  files and nothing else — and what comes back is a picture of a
//  plausible size whose alpha is where it should be: a formula is ink on
//  nothing, a diagram is on its card.
//
//  Hosted in the app, on both platforms: the same host the bubbles use,
//  with the same scheme handler and the same bundle.
//

import CoreGraphics
import Foundation
import Testing
@testable import FamilyConnect

@MainActor
@Suite("Rich renderer", .serialized)
struct RichRendererTests {

    private static let look = RichLook(isMine: false, dark: false, fontSize: 17)

    private static func block(_ kind: MessageMarkdown.RichBlock.Kind, _ source: String, engine: String? = nil)
        -> RichJob
    {
        RichJob(block: MessageMarkdown.RichBlock(kind: kind, engine: engine, source: source), look: look)
    }

    /// The pixels of `image` as RGBA, premultiplied, row by row.
    private struct Pixels {
        let width: Int
        let height: Int
        let bytes: [UInt8]

        init?(_ image: CGImage) {
            let width = image.width
            let height = image.height
            self.width = width
            self.height = height
            var bytes = [UInt8](repeating: 0, count: width * height * 4)
            let drawn = bytes.withUnsafeMutableBytes { buffer -> Bool in
                guard let context = CGContext(
                    data: buffer.baseAddress, width: width, height: height, bitsPerComponent: 8,
                    bytesPerRow: width * 4, space: CGColorSpaceCreateDeviceRGB(),
                    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
                else { return false }
                context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
                return true
            }
            guard drawn else { return nil }
            self.bytes = bytes
        }

        func alpha(_ x: Int, _ y: Int) -> UInt8 { bytes[(y * width + x) * 4 + 3] }

        var inkedPixels: Int { stride(from: 3, to: bytes.count, by: 4).filter { bytes[$0] > 0 }.count }

        var cornersAlpha: [UInt8] {
            [alpha(0, 0), alpha(width - 1, 0), alpha(0, height - 1), alpha(width - 1, height - 1)]
        }
    }

    private func drawing(_ outcome: RichOutcome) throws -> RichDrawing {
        guard case .drawn(let drawing) = outcome else {
            Issue.record("expected a drawing, got a failure")
            throw CancellationError()
        }
        return drawing
    }

    @Test("formulas, diagrams, graphs and a plot are drawn, with alpha where it belongs")
    func drawsEveryKind() async throws {
        let host = RichWebHost()

        // A display formula: transparent around the ink.
        let formula = try drawing(await host.draw(Self.block(.math, "\\frac{a}{b} = \\sqrt{x^2 + \\ce{H2O}}")))
        #expect(formula.width > 20 && formula.width < 600, "width \(formula.width)")
        #expect(formula.height > 15 && formula.height < 200, "height \(formula.height)")
        #expect(formula.scale >= 1, "scale \(formula.scale)")
        let formulaPixels = try #require(Pixels(formula.image))
        #expect(formulaPixels.cornersAlpha.allSatisfy { $0 == 0 }, "a formula has no background")
        #expect(formulaPixels.inkedPixels > 50, "and it has ink")

        // An inline formula: a baseline inside its height.
        let inline = try drawing(await host.draw(RichJob(inlineFormula: "x^2_i", look: Self.look)))
        #expect(inline.baseline > 0 && inline.baseline < inline.height,
                "baseline \(inline.baseline) of \(inline.height)")
        #expect(inline.height < 60)

        // KaTeX's own faces reached the page — through the scheme handler,
        // flattened bundle and all — rather than a fallback font: every one
        // of them, on a page no diagram has restyled yet.
        let faces = try #require(await host.fontFaces())
        #expect(faces.total >= 20, "the page declares \(faces.total) faces")
        #expect(faces.loaded == faces.total, "\(faces.loaded) of \(faces.total) faces loaded")

        // A diagram: on its card, which is opaque.
        let mermaid = try drawing(await host.draw(Self.block(.mermaid, "graph TD\nA[Start] --> B{Choice}\nB --> C[Done]")))
        #expect(mermaid.width > 60 && mermaid.height > 100, "\(mermaid.width)×\(mermaid.height)")
        let mermaidPixels = try #require(Pixels(mermaid.image))
        #expect(mermaidPixels.cornersAlpha.allSatisfy { $0 == 255 }, "a diagram is drawn on its card")

        let dot = try drawing(await host.draw(Self.block(.graphviz, "digraph { a -> b; b -> c; a -> c }", engine: "dot")))
        #expect(dot.width > 40 && dot.height > 80, "\(dot.width)×\(dot.height)")
        let neato = try drawing(await host.draw(Self.block(.graphviz, "graph { a -- b; b -- c; c -- a }", engine: "neato")))
        #expect(neato.width > 40 && neato.height > 40, "\(neato.width)×\(neato.height)")

        let plot = try drawing(await host.draw(Self.block(.plot, "x: -3..3\nsin(x)\nenvelope = exp(-abs(x)/5)")))
        #expect(plot.width > 100 && plot.height > 80, "\(plot.width)×\(plot.height)")
        let plotPixels = try #require(Pixels(plot.image))
        #expect(plotPixels.cornersAlpha.allSatisfy { $0 == 255 })

        // …and a formula drawn after a diagram restyled the page (which
        // rebuilds the faces as `unloaded`) still finds its own: none
        // failed, and the ones it used are loaded.
        _ = try drawing(await host.draw(Self.block(.math, "\\int_0^1 x\\,dx")))
        let after = try #require(await host.fontFaces())
        #expect(after.failed == 0, "\(after.failed) faces failed to load")
        #expect(after.loaded >= 2, "\(after.loaded) of \(after.total) faces loaded")
    }

    @Test("a block the engine refuses, or that is empty, fails rather than drawing nothing")
    func failures() async {
        let host = RichWebHost()
        guard case .failed = await host.draw(Self.block(.math, "\\frac{a}{")) else {
            Issue.record("a broken formula drew")
            return
        }
        guard case .failed = await host.draw(Self.block(.mermaid, "   ")) else {
            Issue.record("an empty diagram drew")
            return
        }
        guard case .failed = await host.draw(Self.block(.graphviz, "digraph {", engine: "dot")) else {
            Issue.record("a broken graph drew")
            return
        }
        // …and the host is still good for the next one.
        guard case .drawn = await host.draw(Self.block(.math, "e^{i\\pi}")) else {
            Issue.record("the host did not recover")
            return
        }
    }

    @Test("a big diagram is capped at 4096 pixels on its longest side")
    func capped() async throws {
        let host = RichWebHost()
        let chain = (0..<40).map { "n\($0) -> n\($0 + 1)" }.joined(separator: "; ")
        let tall = try drawing(await host.draw(Self.block(.graphviz, "digraph { \(chain) }", engine: "dot")))
        #expect(max(tall.image.width, tall.image.height) <= 4096)
        #expect(tall.height > 1000, "a 41-node chain is tall: \(tall.height)")
    }

    @Test("a drawing is kept on disk and read back by a renderer that never drew it")
    func diskCache() async throws {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("rich-cache-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let job = Self.block(.math, "a^2 + b^2 = c^2")

        let first = RichRenderer(directory: directory)
        await first.draw(job)
        guard case .drawn(let drawn)? = first.slot(for: job).outcome else {
            Issue.record("the first renderer did not draw")
            return
        }
        let files = try FileManager.default.contentsOfDirectory(atPath: directory.path)
        #expect(files.contains { $0.hasSuffix(".png") } && files.contains { $0.hasSuffix(".json") })

        let second = RichRenderer(directory: directory)
        await second.draw(job)
        guard case .drawn(let reread)? = second.slot(for: job).outcome else {
            Issue.record("the second renderer found nothing")
            return
        }
        #expect(reread.width == drawn.width && reread.height == drawn.height && reread.baseline == drawn.baseline)
        #expect(reread.image.width == drawn.image.width)
    }

    @Test("the scheme serves the renderer's files and nothing else")
    func schemeFiles() {
        #expect(RichFiles.url(forPath: "/rich.html") != nil)
        #expect(RichFiles.url(forPath: "/mermaid.min.js") != nil)
        #expect(RichFiles.url(forPath: "/fonts/KaTeX_Main-Regular.woff2") != nil)
        #expect(RichFiles.url(forPath: "/Info.plist") == nil)
        #expect(RichFiles.url(forPath: "/../Info.plist") == nil)
        #expect(RichFiles.url(forPath: "/fonts/../rich.html") == nil)
        #expect(RichFiles.url(forPath: "/fonts/Inter.woff2") == nil)
        #expect(RichFiles.url(forPath: "/") == nil)
    }

    @Test("the cache key covers every field and the version")
    func cacheKey() {
        let job = Self.block(.math, "x")
        #expect(job.cacheKey(version: "1") == job.cacheKey(version: "1"))
        #expect(job.cacheKey(version: "1") != job.cacheKey(version: "2"))
        let dark = RichJob(
            block: MessageMarkdown.RichBlock(kind: .math, source: "x"),
            look: RichLook(isMine: false, dark: true, fontSize: 17))
        #expect(job.cacheKey(version: "1") != dark.cacheKey(version: "1"))
        let mine = RichJob(
            block: MessageMarkdown.RichBlock(kind: .math, source: "x"),
            look: RichLook(isMine: true, dark: false, fontSize: 17))
        #expect(job.cacheKey(version: "1") != mine.cacheKey(version: "1"))
        #expect(RichJob(inlineFormula: "x", look: Self.look).cacheKey(version: "1") != job.cacheKey(version: "1"))
        #expect(RichRenderer.shared.version?.hasPrefix("1;katex-0.17.0") == true)
    }
}
