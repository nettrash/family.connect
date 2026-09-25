//
//  BoardGroundTests.swift
//  FamilyConnectTests
//
//  THE WALL IS CORK, and it has texture (docs/protocol.md, "Board": the pin
//  and the ground are the client's own).
//
//  Pinned with pixels, because that is the only thing that can tell a wall
//  with a weave in it from a flat brown rectangle — which is what the apps
//  drew until 2026-09-12, while the web painted four layers and looked like
//  a corkboard. The web is the reference here; these are its numbers.
//
//  `ImageRenderer` renders a SwiftUI view off-screen with no host and no
//  simulator window, which is what makes a drawing testable at all.
//

import CoreGraphics
import SwiftUI
import Testing
@testable import FamilyConnect

@MainActor
@Suite("Board ground")
struct BoardGroundTests {

    /// Every pixel of a 120×120 render, as RGBA bytes.
    private func pixels(of view: some View, side: Int = 120) throws -> [UInt8] {
        let renderer = ImageRenderer(content: view.frame(width: CGFloat(side), height: CGFloat(side)))
        renderer.scale = 1
        let image = try #require(renderer.cgImage, "the ground renders")
        #expect(image.width == side)
        var bytes = [UInt8](repeating: 0, count: side * side * 4)
        let space = CGColorSpace(name: CGColorSpace.sRGB)!
        let context = try #require(
            CGContext(
                data: &bytes, width: side, height: side, bitsPerComponent: 8,
                bytesPerRow: side * 4, space: space,
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
        context.draw(image, in: CGRect(x: 0, y: 0, width: side, height: side))
        return bytes
    }

    @Test("the wall is cork, and it is not one flat colour")
    func theWallHasTexture() throws {
        let bytes = try pixels(of: BoardGround())
        // Cork: warm, and never the theme's surface — red above green above
        // blue in every pixel, with the lamp on or off.
        var reds: [Int] = []
        var warm = 0
        var samples = 0
        for index in stride(from: 0, to: bytes.count, by: 4) {
            let (red, green, blue) = (Int(bytes[index]), Int(bytes[index + 1]), Int(bytes[index + 2]))
            reds.append(red)
            if red > green && green > blue { warm += 1 }
            samples += 1
        }
        #expect(samples == 120 * 120)
        #expect(Double(warm) / Double(samples) > 0.98, "a corkboard is warm everywhere")
        #expect((reds.max() ?? 0) - (reds.min() ?? 0) > 20, "lit in one corner, shaded in the other")
        // TEXTURE, measured against the thing this replaced rather than
        // against a magic number: a plain two-stop gradient of the same
        // colours, which is what the apps drew. A gradient varies SMOOTHLY,
        // so neighbouring pixels are nearly equal; a weave puts a hairline
        // every few pixels in two directions, so they are not. Reverting the
        // ground to a gradient makes these two numbers the same and this
        // assertion fails, which is the whole point of writing it this way.
        let ground = try texture(of: BoardGround())
        let plain = try texture(
            of: LinearGradient(
                colors: [
                    Color(red: 0.796, green: 0.702, blue: 0.569),
                    Color(red: 0.796, green: 0.702, blue: 0.569),
                    Color(red: 0.749, green: 0.639, blue: 0.510),
                ],
                startPoint: .topLeading,
                endPoint: .bottomTrailing))
        // BOTH AXES, because the weave is two sets of lines at OPPOSING
        // angles: one set alone is corduroy, and it would pass a test that
        // only looked across the rows. The near-vertical set is what the
        // rows see and the near-horizontal set is what the columns see, so
        // asserting both is asserting that both sets are drawn.
        #expect(
            ground.acrossRows > plain.acrossRows * 3,
            "the near-vertical hairlines: \(ground.acrossRows) against \(plain.acrossRows)")
        #expect(
            ground.downColumns > plain.downColumns * 3,
            "and the near-horizontal ones: \(ground.downColumns) against \(plain.downColumns)")
    }

    /// How much neighbouring pixels differ, as a fraction of the render, in
    /// each direction — high-frequency variation, which is what a weave is
    /// and what a gradient has almost none of.
    private func texture(
        of view: some View,
        side: Int = 120
    ) throws -> (acrossRows: Double, downColumns: Double) {
        let bytes = try pixels(of: view, side: side)
        func red(_ x: Int, _ y: Int) -> Int { Int(bytes[(y * side + x) * 4]) }
        var rows = 0
        var columns = 0
        for y in 0..<side {
            for x in 0..<(side - 1) where abs(red(x, y) - red(x + 1, y)) >= 2 { rows += 1 }
        }
        for x in 0..<side {
            for y in 0..<(side - 1) where abs(red(x, y) - red(x, y + 1)) >= 2 { columns += 1 }
        }
        let pairs = Double(side * (side - 1))
        return (Double(rows) / pairs, Double(columns) / pairs)
    }

    /// The lamp is in the top-left on every client: the highlight belongs
    /// where the light falls, and the shadow in the far corner.
    @Test("the wall is lit from the top-left and shaded at the bottom-right")
    func theWallIsLitFromTheCorner() throws {
        let side = 120
        let bytes = try pixels(of: BoardGround(), side: side)
        func brightness(x: Int, y: Int) -> Int {
            let index = (y * side + x) * 4
            return Int(bytes[index]) + Int(bytes[index + 1]) + Int(bytes[index + 2])
        }
        // Averaged over a patch, so one hairline of the weave cannot decide
        // it: the lines are deliberately faint, and a single pixel may land
        // on one.
        func patch(x: Int, y: Int) -> Int {
            var total = 0
            for dx in 0..<12 {
                for dy in 0..<12 {
                    total += brightness(x: x + dx, y: y + dy)
                }
            }
            return total / 144
        }
        #expect(patch(x: 8, y: 4) > patch(x: 100, y: 104))
    }

    @Test("the pin draws, and draws the same wherever it is")
    func thePinDraws() throws {
        let bytes = try pixels(of: NotePin(), side: 24)
        // Something red is there, and it is not the whole box: a pin is a
        // small round head over the card's edge, not a field.
        let painted = stride(from: 3, to: bytes.count, by: 4).filter { bytes[$0] > 0 }.count
        #expect(painted > 20)
        #expect(painted < 24 * 24)
    }
}
