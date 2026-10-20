//
//  StickerFramesTests.swift
//  FamilyConnectTests
//
//  A moving sticker decoded once and played by Core Animation
//  (StickerFrames, StickerLayerAnimation). The timing must be the
//  protocol's — the same rule the old ImageIO player applied — and the
//  memory bounds must hold, because every frame is now kept decoded.
//

import CoreGraphics
import Foundation
import QuartzCore
import Testing
@testable import FamilyConnect

@Suite("Sticker frames")
struct StickerFramesTests {

    // MARK: - Timing

    @Test("each frame lasts what the file states, and 10 ms or less lasts 100")
    func durationsFollowTheProtocol() {
        #expect(StickerFrames.durationsMS(stated: [40, 0, 10, 11, 250], count: 5)
            == [40, 100, 100, 11, 250])
    }

    @Test("a statement that disagrees with the frame count is padded or cut")
    func durationsPadAndCut() {
        #expect(StickerFrames.durationsMS(stated: [70], count: 3) == [70, 100, 100])
        #expect(StickerFrames.durationsMS(stated: [70, 80, 90], count: 2) == [70, 80])
        #expect(StickerFrames.durationsMS(stated: nil, count: 2) == [100, 100])
    }

    @Test("key times are where each frame starts, plus the closing 1")
    func keyTimes() {
        let times = StickerFrames.keyTimes(durationsMS: [100, 300, 100])
        #expect(times == [0, 0.2, 0.8, 1])
        // One frame, or no time at all, is nothing to animate.
        #expect(StickerFrames.keyTimes(durationsMS: [100]) == nil)
        #expect(StickerFrames.keyTimes(durationsMS: [0, 0]) == nil)
    }

    // MARK: - Sizes and the budget

    @Test("the asked edge is the box in pixels, rounded up to 64")
    func requestedEdge() {
        #expect(StickerFrames.requestedEdge(points: 160, scale: 3) == 512)
        #expect(StickerFrames.requestedEdge(points: 79.3, scale: 3) == 256)
        #expect(StickerFrames.requestedEdge(points: 80.1, scale: 3) == 256)
        #expect(StickerFrames.requestedEdge(points: 64, scale: 1) == 64)
        #expect(StickerFrames.requestedEdge(points: 0, scale: 3) == 0)
        #expect(StickerFrames.requestedEdge(points: .infinity, scale: 3) == 0)
    }

    @Test("never decoded larger than the file, and shrunk to fit the budget")
    func decodeEdge() {
        // A small file is decoded at its own size.
        #expect(StickerFrames.decodeEdge(requested: 512, sourceEdge: 300, frameCount: 10) == 300)
        // 60 frames of 512 px would be 60 MB: shrunk until 60 frames fit 24 MB.
        let edge = StickerFrames.decodeEdge(requested: 512, sourceEdge: 512, frameCount: 60)
        #expect(edge != nil)
        if let edge {
            #expect(edge < 512)
            #expect(60 * edge * edge * 4 <= StickerFrames.perStickerBudget)
        }
        // So many frames that not even the floor fits: hold still instead.
        #expect(StickerFrames.decodeEdge(
            requested: 512, sourceEdge: 512, frameCount: 5000, budget: 1024 * 1024) == nil)
        #expect(StickerFrames.decodeEdge(requested: 0, sourceEdge: 512, frameCount: 2) == nil)
    }

    // MARK: - Decoding

    @Test("an animated WebP decodes into every frame at the file's own timing")
    func decodesAnimatedWebP() throws {
        let frames = try #require(StickerFrames.decode(StickerFixtures.animatedWebP, maxEdge: 512))
        #expect(frames.frames.count == 2)
        #expect(frames.durationsMS == [40, 40])
        #expect(frames.totalMS == 80)
        #expect(frames.frames.allSatisfy { $0.width == 8 && $0.height == 8 })
        #expect(frames.byteCost >= 2 * 8 * 8 * 4)
    }

    @Test("an animated GIF and APNG decode too; a still one and a TIFF do not")
    func decodesOtherFormats() {
        #expect(StickerFrames.decode(StickerFixtures.animatedGIF, maxEdge: 64)?.frames.count == 2)
        #expect(StickerFrames.decode(StickerFixtures.animatedPNG, maxEdge: 64)?.frames.count == 2)
        #expect(StickerFrames.decode(StickerFixtures.stillWebP, maxEdge: 64) == nil)
        // Two pages, but a TIFF is not a format where frames mean motion —
        // the async entry point asks `StickerPack.isAnimated` first.
        #expect(StickerPack.isAnimated(StickerFixtures.twoPageTIFF) == false)
    }

    @Test("the file entry point reads a cached sticker off the main actor", .timeLimit(.minutes(1)))
    func decodesFromFile() async throws {
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("sticker-frames-\(UUID().uuidString).webp")
        try StickerFixtures.animatedWebP.write(to: url)
        defer { try? FileManager.default.removeItem(at: url) }
        let frames = await StickerFrames.decode(fileAt: url, maxEdge: 64)
        #expect(frames?.frames.count == 2)
        let missing = await StickerFrames.decode(
            fileAt: url.appendingPathExtension("absent"), maxEdge: 64)
        #expect(missing == nil)
    }

    // MARK: - The animation Core Animation plays

    @Test("the keyframe animation is discrete, endless and timed by the file")
    @MainActor
    func keyframeAnimation() throws {
        let frames = try #require(StickerFrames.decode(StickerFixtures.animatedWebP, maxEdge: 64))
        let animation = try #require(StickerLayerAnimation.animation(for: frames))
        #expect(animation.keyPath == "contents")
        #expect(animation.calculationMode == .discrete)
        #expect(animation.values?.count == 2)
        // `.discrete` wants one key time more than values.
        #expect(animation.keyTimes?.map(\.doubleValue) == [0, 0.5, 1])
        #expect(abs(animation.duration - 0.08) < 0.000_1)
        #expect(animation.repeatCount == .infinity)
        #expect(animation.isRemovedOnCompletion == false)

        // And it lands on a layer, with frame zero as the plain contents
        // for the moments the animation is not attached.
        let layer = CALayer()
        StickerLayerAnimation.play(frames, on: layer)
        #expect(layer.animation(forKey: StickerLayerAnimation.key) != nil)
        #expect(layer.contents != nil)
    }
}
