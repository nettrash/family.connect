//
//  AnimationPlanVectorTests.swift
//  FamilyConnectTests
//
//  AN ORACLE, NOT FOUR READINGS (issue #96). docs/protocol.md's
//  "Animations" › "The file" is integer arithmetic four encoders have to
//  agree on, so that the same GIF becomes the same clip everywhere; the
//  reference is `web/text/src/animation.rs`, whose answers
//  `win/tools/board-oracle` prints into Fixtures/animation-plan-vectors.json
//  — the same bytes Android, Windows and the web test against, compared by
//  CI. MediaPlanVectorTests' shape, read the same way: the file is PARSED,
//  never compared as text, and a JSON null `expected` is "no plan".
//

import Foundation
import Testing
@testable import FamilyConnect

@Suite("Animation plan vectors")
struct AnimationPlanVectorTests {

    struct Case {
        let name: String
        let function: String
        let input: [String: Any]
        /// nil for the reference's `None`.
        let expected: [String: Any]?
    }

    /// Bundle(for:) needs a class; a Swift Testing suite is a struct.
    private final class Anchor {}

    /// The synchronized group flattens Fixtures/ into the bundle's root;
    /// the source tree is the fallback, as MediaPlanVectorTests' is.
    private static func vectorsURL() -> URL {
        if let bundled = Bundle(for: Anchor.self)
            .url(forResource: "animation-plan-vectors", withExtension: "json") {
            return bundled
        }
        return URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .appending(path: "Fixtures")
            .appending(path: "animation-plan-vectors.json")
    }

    private static func load() throws -> [Case] {
        let data = try Data(contentsOf: vectorsURL())
        let array = try #require(try JSONSerialization.jsonObject(with: data) as? [[String: Any]])
        return try array.map { raw in
            Case(
                name: try #require(raw["name"] as? String),
                function: try #require(raw["function"] as? String),
                input: try #require(raw["input"] as? [String: Any]),
                expected: raw["expected"] as? [String: Any])
        }
    }

    /// The cases for one function — and never none: a renamed function in
    /// the generator would otherwise make its test pass by asserting nothing.
    private static func cases(for function: String) throws -> [Case] {
        let selected = try load().filter { $0.function == function }
        #expect(!selected.isEmpty, "no vectors for \(function)")
        return selected
    }

    private static func int(_ value: Any?) -> Int? {
        (value as? NSNumber)?.intValue
    }

    @Test("every vector names a function this port checks")
    func everyFunctionIsCovered() throws {
        let all = try Self.load()
        let known: Set<String> = [
            "constants", "gif_delay_ms", "apng_delay_ms", "frame_duration_ms", "plan",
        ]
        #expect(all.count >= 46, "the reference printed 46 cases; \(all.count) were read")
        for vector in all {
            #expect(known.contains(vector.function), "\(vector.name): unknown function \(vector.function)")
        }
    }

    @Test("the constants")
    func constants() throws {
        let vector = try #require(try Self.cases(for: "constants").first)
        let expected = try #require(vector.expected)
        #expect(Self.int(expected["ticks_per_second"]) == AnimationTiming.ticksPerSecond)
        #expect(Self.int(expected["shortest_frame_ms"]) == AnimationTiming.shortestFrameMS)
        #expect(Self.int(expected["default_frame_ms"]) == AnimationTiming.defaultFrameMS)
        let rgb = try #require(expected["background_rgb"] as? [NSNumber]).map { $0.uint8Value }
        #expect(rgb == AnimationTiming.backgroundRGB)
    }

    @Test("gif_delay_ms")
    func gifDelay() throws {
        for vector in try Self.cases(for: "gif_delay_ms") {
            let centiseconds = try #require(Self.int(vector.input["centiseconds"]))
            #expect(
                AnimationTiming.gifDelayMS(centiseconds: UInt16(centiseconds))
                    == Self.int(vector.expected?["ms"]),
                "\(vector.name)")
        }
    }

    @Test("apng_delay_ms")
    func apngDelay() throws {
        for vector in try Self.cases(for: "apng_delay_ms") {
            let num = try #require(Self.int(vector.input["num"]))
            let den = try #require(Self.int(vector.input["den"]))
            #expect(
                AnimationTiming.apngDelayMS(num: UInt16(num), den: UInt16(den))
                    == Self.int(vector.expected?["ms"]),
                "\(vector.name)")
        }
    }

    @Test("frame_duration_ms")
    func frameDuration() throws {
        for vector in try Self.cases(for: "frame_duration_ms") {
            let stated = try #require(Self.int(vector.input["stated_ms"]))
            #expect(
                AnimationTiming.frameDurationMS(stated) == Self.int(vector.expected?["ms"]),
                "\(vector.name)")
        }
    }

    @Test("plan")
    func plan() throws {
        for vector in try Self.cases(for: "plan") {
            let stated = try #require(vector.input["stated_ms"] as? [NSNumber]).map(\.intValue)
            let plan = AnimationTiming.plan(
                width: Self.int(vector.input["width"]) ?? 0,
                height: Self.int(vector.input["height"]) ?? 0,
                statedMS: stated)
            guard let expected = vector.expected else {
                #expect(plan == nil, "\(vector.name): the reference plans nothing")
                continue
            }
            let got = try #require(plan, "\(vector.name): no plan where the reference has one")
            #expect(got.width == Self.int(expected["width"]), "\(vector.name)")
            #expect(got.height == Self.int(expected["height"]), "\(vector.name)")
            #expect(got.totalTicks == Self.int(expected["total_ticks"]), "\(vector.name)")
            #expect(got.durationMS == Self.int(expected["duration_ms"]), "\(vector.name)")
            #expect(got.videoBitrate == Self.int(expected["video_bitrate"]), "\(vector.name)")
            let samples = try #require(expected["samples"] as? [[NSNumber]]).map {
                AnimationTiming.Sample(frame: $0[0].intValue, ticks: $0[1].intValue)
            }
            #expect(got.samples == samples, "\(vector.name)")
        }
    }
}
