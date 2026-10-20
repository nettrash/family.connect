//
//  StickerFrames.swift
//  FamilyConnect
//
//  A moving sticker, decoded ONCE: every frame, already composited and
//  already the size it is drawn at, with how long each is shown — what
//  `StickerImage` hands to Core Animation so the render server plays it.
//
//  WHY THIS EXISTS. A sticker used to move through
//  `CGAnimateImageDataWithBlock`, one call per view: ImageIO decoded every
//  frame at the file's FULL size, on the main run loop, and handed each to
//  a SwiftUI `@Observable`, so every tick of every moving sticker was a
//  decode, a new CGImage and a SwiftUI invalidation on the actor the
//  scroll and the taps are handled on. A panel of moving stickers kept the
//  main thread busy enough that the whole app felt slow while one was on
//  screen, and leaving and coming back started the decoding over. Now the
//  frames are decoded off the main actor, once per sticker per drawn size,
//  kept in a byte-bounded cache (`AttachmentStore.stickerFrames`), and
//  played by a `CAKeyframeAnimation` on the layer's `contents` — after
//  which nothing on the main thread or in SwiftUI runs per frame at all.
//
//  THE TIMING IS THE PROTOCOL'S, unchanged: each frame lasts what the file
//  states (`AnimatedPicture.statedDelaysMS`), and a frame stated at 10 ms
//  or less lasts 100 ms (`AnimationTiming.frameDurationMS`) — the browser
//  rule ImageIO's own animation applied to the old player. A file whose
//  delays cannot be read (an image sequence, which the sticker test also
//  calls moving) shows each frame for that same 100 ms.
//
//  THE COST, and how it is bounded. Holding every frame decoded is the
//  price of not decoding on every tick: a 480-pixel frame is ~0.9 MB, and
//  some stickers have sixty frames. Two bounds keep that honest. Frames are
//  decoded at the size they are DRAWN (the box in pixels, never larger than
//  the file), so a panel cell holds a quarter of what a chat bubble does;
//  and one sticker may cost at most `perStickerBudget` — a long one is
//  decoded smaller rather than holding the cache hostage, which is softer
//  at worst and never a sticker that does not move. The cache's own budget
//  bounds the total, and an animating layer keeps its own frames alive, so
//  eviction can only ever cost a re-decode of something off screen.
//

import CoreGraphics
import Foundation
import ImageIO

nonisolated struct StickerFrames: @unchecked Sendable {
    // `@unchecked` for exactly the reason `StickerPack.Still` gives: a
    // CGImage is immutable, and nothing here is ever mutated after decode.

    /// Every frame, composited (ImageIO hands back the canvas as a viewer
    /// shows it — measured on a WebP whose second frame covers one corner).
    let frames: [CGImage]
    /// How long each frame is shown, in milliseconds, after
    /// `AnimationTiming.frameDurationMS`. Same count as `frames`.
    let durationsMS: [Int]

    /// One loop, in milliseconds.
    var totalMS: Int { durationsMS.reduce(0, +) }

    /// Width over height of the canvas, for laying the playing layer out
    /// exactly where the still was. 1 for a degenerate frame.
    var aspectRatio: CGFloat {
        guard let first = frames.first, first.height > 0 else { return 1 }
        return CGFloat(first.width) / CGFloat(first.height)
    }

    /// What the frames cost decoded — the cache's unit.
    var byteCost: Int {
        frames.reduce(0) { $0 + $1.bytesPerRow * $1.height }
    }

    /// Most bytes ONE sticker's frames may cost; a longer sticker is decoded
    /// smaller instead (see the header).
    static let perStickerBudget = 24 * 1024 * 1024

    /// Smallest edge a sticker is ever decoded at to fit that budget. Below
    /// this a sticker would rather hold still than be a smear: a file with
    /// so many frames that even this does not fit is drawn as its first.
    static let minimumEdge = 96

    // MARK: - The rules, pure

    /// The `keyTimes` of a discrete keyframe animation over these
    /// durations: where each frame STARTS as a fraction of the loop, plus
    /// the closing 1 — Core Animation wants one more key time than values
    /// in `.discrete` mode. nil when there is nothing to animate.
    static func keyTimes(durationsMS: [Int]) -> [Double]? {
        let total = durationsMS.reduce(0, +)
        guard durationsMS.count > 1, total > 0 else { return nil }
        var times: [Double] = []
        times.reserveCapacity(durationsMS.count + 1)
        var elapsed = 0
        for duration in durationsMS {
            times.append(Double(elapsed) / Double(total))
            elapsed += duration
        }
        times.append(1)
        return times
    }

    /// Each frame's shown length from what the file states — the protocol's
    /// rule, padded or cut to `count` with the default where the statement
    /// and ImageIO disagree about how many frames there are.
    static func durationsMS(stated: [Int]?, count: Int) -> [Int] {
        (0..<count).map { index in
            let stated = stated.flatMap { index < $0.count ? $0[index] : nil }
            return stated.map { AnimationTiming.frameDurationMS(max($0, 0)) }
                ?? AnimationTiming.defaultFrameMS
        }
    }

    /// The pixel edge a box of `points` asks for at `scale`, rounded UP to a
    /// multiple of 64. Rounded so a grid whose cells are 79.3 points one
    /// layout and 80.1 the next asks the cache for one size rather than two;
    /// up, so a frame is never drawn larger than it was decoded.
    static func requestedEdge(points: CGFloat, scale: CGFloat) -> Int {
        guard points.isFinite, scale.isFinite, points > 0, scale > 0 else { return 0 }
        let pixels = Int((points * scale).rounded(.up))
        return ((pixels + 63) / 64) * 64
    }

    /// The edge to decode at: what the box asks, never more than the file
    /// has, and shrunk until `frameCount` frames fit `budget` bytes. nil
    /// when not even `minimumEdge` fits — hold still rather than smear.
    ///
    /// The estimate takes the frame as square at the edge, which is the
    /// most a frame of that edge can cost; a tall or wide sticker costs
    /// less than it says, never more.
    static func decodeEdge(
        requested: Int, sourceEdge: Int, frameCount: Int, budget: Int = perStickerBudget
    ) -> Int? {
        guard requested > 0, sourceEdge > 0, frameCount > 0 else { return nil }
        var edge = min(requested, sourceEdge)
        let perFrame = budget / frameCount
        if edge * edge * 4 > perFrame {
            edge = Int(Double(perFrame / 4).squareRoot())
        }
        guard edge >= min(minimumEdge, sourceEdge) else { return nil }
        return edge
    }

    // MARK: - Decoding, off the main actor

    /// Every frame of the sticker cached at `url`, at most `maxEdge` pixels
    /// on its longer side — or nil for a still sticker, a missing or
    /// unreadable file, or one that cannot be held within the budget. nil
    /// always means "draw the still", which is the protocol's own answer
    /// for a sticker that cannot move here.
    @concurrent
    static func decode(fileAt url: URL, maxEdge: Int, budget: Int = perStickerBudget) async -> StickerFrames? {
        guard let data = try? Data(contentsOf: url), StickerPack.isAnimated(data) else { return nil }
        return decode(data, maxEdge: maxEdge, budget: budget)
    }

    /// The same, from bytes already in memory — a sticker that has been
    /// made and not yet added, which has no cached file yet.
    @concurrent
    static func decode(data: Data, maxEdge: Int, budget: Int = perStickerBudget) async -> StickerFrames? {
        guard StickerPack.isAnimated(data) else { return nil }
        return decode(data, maxEdge: maxEdge, budget: budget)
    }

    /// The synchronous body of both, for tests.
    static func decode(_ data: Data, maxEdge: Int, budget: Int = perStickerBudget) -> StickerFrames? {
        guard let source = CGImageSourceCreateWithData(data as CFData, [
                  // Each frame is decoded exactly once, into the thumbnail
                  // below; ImageIO keeping its own copy would double it.
                  kCGImageSourceShouldCache: false,
              ] as CFDictionary)
        else { return nil }
        let count = CGImageSourceGetCount(source)
        guard count > 1,
              let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
              let width = properties[kCGImagePropertyPixelWidth] as? Int,
              let height = properties[kCGImagePropertyPixelHeight] as? Int,
              let edge = decodeEdge(
                  requested: maxEdge, sourceEdge: max(width, height),
                  frameCount: count, budget: budget)
        else { return nil }
        let options = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceShouldCacheImmediately: true,
            kCGImageSourceThumbnailMaxPixelSize: edge,
        ] as CFDictionary
        var frames: [CGImage] = []
        frames.reserveCapacity(count)
        for index in 0..<count {
            // One unreadable frame and the sticker holds still: a gap in
            // the loop would be a flicker, which is worse than no motion.
            guard let frame = CGImageSourceCreateThumbnailAtIndex(source, index, options) else {
                return nil
            }
            frames.append(frame)
        }
        let durations = durationsMS(
            stated: AnimatedPicture.statedDelaysMS(data, source: source), count: count)
        return StickerFrames(frames: frames, durationsMS: durations)
    }
}
