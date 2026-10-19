//
//  AnimationTiming.swift
//  FamilyConnect
//
//  The TIMING half of an animation sent in a chat (issue #96,
//  docs/protocol.md "Animations"): which source frame is shown when, how
//  long the clip is, its size and its bitrate. A port of
//  `web/text/src/animation.rs` (`fc_text::animation`), held to it by
//  Fixtures/animation-plan-vectors.json — the same bytes Android, Windows
//  and the web test against, because four encoders have to make the same
//  clip of the same picture.
//
//  THE GRID. The animation is SAMPLED thirty times a second: at each tick,
//  the frame that is showing at that instant; ticks in a row that show the
//  same frame are one video frame, that many ticks long. A frame held for a
//  second after a 20 ms flash stays held, nothing runs faster than 30 a
//  second, and a slow animation is never given frames it did not have.
//
//  THE SHORTEST FRAME. A frame stated at 10 ms or less is shown for 100 ms,
//  which is what every browser does with the many GIFs that say "0".
//
//  Decoding the frames and encoding the video are `AnimatedPicture` and
//  `AnimationEncoder`; this file is integers only.
//

import Foundation

nonisolated enum AnimationTiming {

    /// Samples a second: the grid the animation is read on, and the
    /// timescale every duration in a plan is counted in.
    static let ticksPerSecond = 30

    /// A frame stated at this or less is shown for `defaultFrameMS`.
    static let shortestFrameMS = 10

    /// How long such a frame is shown.
    static let defaultFrameMS = 100

    /// What shows through a transparent pixel: a video has no transparency.
    static let backgroundRGB: [UInt8] = [0xFF, 0xFF, 0xFF]

    /// A GIF frame's delay as its Graphic Control Extension states it, in
    /// hundredths of a second, in milliseconds — before `frameDurationMS`.
    static func gifDelayMS(centiseconds: UInt16) -> Int {
        Int(centiseconds) * 10
    }

    /// An APNG frame's delay from its `fcTL`: `num ÷ den` seconds, a `den` of
    /// 0 meaning hundredths, in whole milliseconds rounded half up — before
    /// `frameDurationMS`. Integers throughout, as the reference computes it.
    static func apngDelayMS(num: UInt16, den: UInt16) -> Int {
        let den = den == 0 ? 100 : Int(den)
        return (2 * Int(num) * 1000 + den) / (2 * den)
    }

    /// How long a frame stated at `statedMS` is shown: as stated, unless it
    /// is `shortestFrameMS` or less. (A WebP's `ANMF` states milliseconds.)
    static func frameDurationMS(_ statedMS: Int) -> Int {
        statedMS <= shortestFrameMS ? defaultFrameMS : statedMS
    }

    /// One frame of the video: which source frame (0-based, in the source's
    /// order, composited) and for how many ticks.
    struct Sample: Equatable, Sendable {
        let frame: Int
        let ticks: Int
    }

    /// What an animation becomes.
    struct Plan: Equatable, Sendable {
        /// Even, the source's orientation, never larger than the source:
        /// the video profile's size rule.
        let width: Int
        let height: Int
        /// In order; together they last `totalTicks`.
        let samples: [Sample]
        let totalTicks: Int
        /// `totalTicks` in milliseconds, rounded half up — the upload's
        /// `duration_ms`.
        let durationMS: Int
        /// The profile's bitrate for the target size at 30 frames a second.
        let videoBitrate: Int
    }

    /// The plan for an animation of `width × height` whose frames are stated
    /// to last `statedMS` each, in the source's order. nil when there is no
    /// size or no frame — or a size that rounds to nothing.
    static func plan(width: Int, height: Int, statedMS: [Int]) -> Plan? {
        guard width > 0, height > 0, !statedMS.isEmpty else { return nil }
        let target = MediaPlan.targetSize(width: width, height: height)
        guard target.width > 0, target.height > 0 else { return nil }
        // Where each frame starts, in milliseconds, and where the last ends.
        // Int is 64-bit on every platform this builds for, the reference's
        // u64; a negative stated length (nothing parses one) counts as 0.
        var starts: [Int] = []
        starts.reserveCapacity(statedMS.count)
        var total = 0
        for stated in statedMS {
            starts.append(total)
            total += frameDurationMS(max(stated, 0))
        }
        let tps = ticksPerSecond
        // ceil(total × 30 ÷ 1000), at least one tick.
        let totalTicks = max((total * tps + 999) / 1000, 1)
        // The reference counts ticks in a u32 and plans nothing past it.
        guard totalTicks <= Int(UInt32.max) else { return nil }
        var samples: [Sample] = []
        var frame = 0
        var runFrame = 0
        var runTicks = 0
        for tick in 0..<totalTicks {
            // The frame showing at tick ÷ 30 s: the last one that has
            // started, compared as tick × 1000 against start × 30.
            while frame + 1 < starts.count, starts[frame + 1] * tps <= tick * 1000 {
                frame += 1
            }
            if runTicks > 0, runFrame == frame {
                runTicks += 1
            } else {
                if runTicks > 0 { samples.append(Sample(frame: runFrame, ticks: runTicks)) }
                runFrame = frame
                runTicks = 1
            }
        }
        samples.append(Sample(frame: runFrame, ticks: runTicks))
        return Plan(
            width: target.width,
            height: target.height,
            samples: samples,
            totalTicks: totalTicks,
            durationMS: (2 * totalTicks * 1000 + tps) / (2 * tps),
            videoBitrate: MediaPlan.profileVideoBitrate(
                width: target.width, height: target.height, frameRate: MediaPlan.maxFrameRate))
    }
}
