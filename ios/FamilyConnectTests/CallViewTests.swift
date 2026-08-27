//
//  CallViewTests.swift
//  FamilyConnectTests
//
//  The call screen's control disc, drawn. The regression this pins was
//  reported from a phone in dark mode: the active speaker button was
//  "totally white" — a white disc under a glyph that .primary had also
//  made white. No rule about colours in code can see that; only the
//  pixels can, so the disc is rendered in the dark appearance and read.
//
//  WHAT IS READ. A ring of samples well inside the disc's edge but well
//  outside the glyph gives the disc's own colour; the pixels around the
//  centre give the glyph. Two assertions: the disc is not near-white, and
//  something at the centre differs from the disc in luminance — a glyph
//  that is there and can be seen. Both states are checked, since both
//  have a disc and a glyph that must stay apart.
//

import CoreGraphics
import Foundation
import SwiftUI
import Testing
@testable import FamilyConnect

@MainActor
struct CallViewTests {

    private static let side: CGFloat = 64
    /// Room around the disc, so the ring samples cannot fall off the
    /// canvas and the background behind the disc is what the screen has
    /// behind it in dark mode.
    private static let inset: CGFloat = 8

    @Test("the ACTIVE toggle is not a white disc in dark mode, and its glyph shows on it")
    func activeToggleInDark() throws {
        let pixels = try render(isActive: true)
        let ring = discSamples(pixels)
        for sample in ring {
            #expect(
                !isNearWhite(sample),
                "an active toggle's disc came out near-white \(sample) in dark mode — the regression: a white glyph on a white disc")
        }
        #expect(glyphContrast(pixels, disc: ring) > 0.25, "the active glyph does not stand out from its disc")
    }

    @Test("the INACTIVE toggle is not a white disc in dark mode either, and its glyph shows on it")
    func inactiveToggleInDark() throws {
        let pixels = try render(isActive: false)
        let ring = discSamples(pixels)
        for sample in ring {
            #expect(!isNearWhite(sample), "an inactive toggle's disc came out near-white \(sample) in dark mode")
        }
        #expect(glyphContrast(pixels, disc: ring) > 0.25, "the inactive glyph does not stand out from its disc")
    }

    // MARK: - Row geometry

    /// The three control rows — the ring's pair, the active row, the
    /// ended placeholder — replace each other in the same centred column,
    /// so they share one height per call kind. The ring row used to be
    /// the odd one out (64pt discs, no inset) and the identity block
    /// hopped 8pt on Accept and on a missed video call.
    @Test("the ring, active and ended rows share one height per call kind")
    func rowHeightsAgree() {
        // Voice: the classic 64 disc, no plate.
        #expect(CallControlRowMetrics.rowHeight(video: false) == 64)
        #expect(CallControlRowMetrics.discSide(video: false) + CallControlRowMetrics.inset(video: false) * 2 == 64)
        // Video: the 56 disc inside the plate's 12pt inset on both sides.
        #expect(CallControlRowMetrics.rowHeight(video: true) == 80)
        #expect(CallControlRowMetrics.discSide(video: true) + CallControlRowMetrics.inset(video: true) * 2 == 80)
    }

    /// The row is as wide as its discs want, not as wide as a phone's
    /// five: a Mac voice call's two discs were 210pt apart in a 420 row.
    @Test("the active row's width follows the disc count, capped at the phone's five")
    func rowWidthFollowsCount() {
        #expect(CallControlRowMetrics.rowWidth(buttons: 5) == 420)
        #expect(CallControlRowMetrics.rowWidth(buttons: 4) == 400)
        #expect(CallControlRowMetrics.rowWidth(buttons: 3) == 300)
        #expect(CallControlRowMetrics.rowWidth(buttons: 2) == 200)
    }

    // MARK: - Rendering

    private struct Pixels {
        let width: Int
        let height: Int
        let data: [UInt8]

        subscript(x: Int, y: Int) -> (r: Int, g: Int, b: Int) {
            let index = (y * width + x) * 4
            return (Int(data[index]), Int(data[index + 1]), Int(data[index + 2]))
        }
    }

    /// The disc as the app draws it, over the dark appearance's own
    /// background so a material disc has something to be a material on.
    private func render(isActive: Bool) throws -> Pixels {
        let view = CallControlButton(
            symbol: isActive ? "mic.slash.fill" : "mic.fill",
            label: "Mute",
            isActive: isActive,
            side: Self.side
        ) {}
            .padding(Self.inset)
            .background(Color.black)
            .environment(\.colorScheme, .dark)
        let renderer = ImageRenderer(content: view)
        // Pixels, not points — every coordinate below is a pixel.
        renderer.scale = 1
        let image = try #require(renderer.cgImage, "the disc did not render")

        let width = image.width
        let height = image.height
        var data = [UInt8](repeating: 0, count: width * height * 4)
        let context = try #require(
            CGContext(
                data: &data,
                width: width,
                height: height,
                bitsPerComponent: 8,
                bytesPerRow: width * 4,
                space: CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue),
            "could not read the rendered pixels")
        context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
        return Pixels(width: width, height: height, data: data)
    }

    /// Eight points on a ring at 0.42 of the diameter from the centre:
    /// inside the disc (its edge is at 0.5) AND inside its hairline (a
    /// 1pt stroke at the edge — 0.484 on a 64pt disc), outside the glyph
    /// (0.34 of the side, so it reaches ~0.2).
    private func discSamples(_ pixels: Pixels) -> [(r: Int, g: Int, b: Int)] {
        let centre = CGPoint(x: CGFloat(pixels.width) / 2, y: CGFloat(pixels.height) / 2)
        let radius = Self.side * 0.42
        return (0..<8).map { step in
            let angle = CGFloat(step) / 8 * 2 * .pi
            let x = Int((centre.x + cos(angle) * radius).rounded())
            let y = Int((centre.y + sin(angle) * radius).rounded())
            return pixels[x, y]
        }
    }

    private func isNearWhite(_ p: (r: Int, g: Int, b: Int)) -> Bool {
        p.r > 200 && p.g > 200 && p.b > 200
    }

    private func luminance(_ p: (r: Int, g: Int, b: Int)) -> Double {
        (0.2126 * Double(p.r) + 0.7152 * Double(p.g) + 0.0722 * Double(p.b)) / 255
    }

    /// The largest luminance gap between the disc's colour and anything
    /// within a small square around the centre — where the glyph is.
    private func glyphContrast(_ pixels: Pixels, disc: [(r: Int, g: Int, b: Int)]) -> Double {
        let discLuminance = disc.map(luminance).sorted()[disc.count / 2]
        let cx = pixels.width / 2
        let cy = pixels.height / 2
        let reach = Int(Self.side * 0.2)
        var best = 0.0
        for y in (cy - reach)...(cy + reach) {
            for x in (cx - reach)...(cx + reach) {
                best = max(best, abs(luminance(pixels[x, y]) - discLuminance))
            }
        }
        return best
    }
}
