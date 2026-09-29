//
//  MediaPlanVectorTests.swift
//  FamilyConnectTests
//
//  AN ORACLE, NOT FOUR READINGS. docs/protocol.md's "Preparing media before
//  upload" is exact arithmetic that four codebases have to agree on for the
//  same file, and the reference is `web/text/src/media_plan.rs`. Its answers
//  are printed by `win/tools/board-oracle` (`cargo run -- media-plan`) into
//  Fixtures/media-plan-vectors.json — the same bytes Android and Windows
//  test against, and CI `cmp`s the three copies — so every case here is a
//  case every port gets right, or none of them does.
//
//  The cases that earn their keep are the ones a "reasonable" port gets
//  wrong: a bitrate that lands on exactly half a thousand (Swift's default
//  rounding is to even), 3618×128 at 25 fps (which rounds the other way if
//  the formula is evaluated left to right), a one-pixel-wide clip, a frame
//  rate of exactly 30.5, and an audio track whose rate nobody stated.
//
//  The file is a JSON array, one case per line, keys sorted — so it is
//  PARSED, never compared as text. Its null and its 0 are both "unknown",
//  and both are fed in as they are, because telling them apart is the
//  planner's job, not the loader's.
//

import Foundation
import Testing
@testable import FamilyConnect

@Suite("Media plan vectors")
struct MediaPlanVectorTests {

    // MARK: - Loading

    /// One case: which function, what went in, what the reference said.
    struct Case {
        let name: String
        let function: String
        let input: [String: Any]
        let expected: [String: Any]
    }

    /// Bundle(for:) needs a class; a Swift Testing suite is a struct.
    private final class Anchor {}

    /// The synchronized group flattens Fixtures/ into the bundle's root.
    /// Falls back to the source tree, as LinkPreviewFixtures does, for a
    /// build that decided not to copy a .json.
    private static func vectorsURL() -> URL {
        if let bundled = Bundle(for: Anchor.self)
            .url(forResource: "media-plan-vectors", withExtension: "json") {
            return bundled
        }
        return URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .appending(path: "Fixtures")
            .appending(path: "media-plan-vectors.json")
    }

    private static func load() throws -> [Case] {
        let data = try Data(contentsOf: vectorsURL())
        let array = try #require(try JSONSerialization.jsonObject(with: data) as? [[String: Any]])
        return try array.map { raw in
            Case(
                name: try #require(raw["name"] as? String),
                function: try #require(raw["function"] as? String),
                input: try #require(raw["input"] as? [String: Any]),
                expected: try #require(raw["expected"] as? [String: Any]))
        }
    }

    /// The cases for one function — and never none of them: a renamed
    /// function in the generator would otherwise make its test pass by
    /// asserting nothing.
    private static func cases(for function: String) throws -> [Case] {
        let selected = try load().filter { $0.function == function }
        #expect(!selected.isEmpty, "no vectors for \(function)")
        return selected
    }

    // MARK: - JSON, read the way the reference wrote it

    /// An integer, or nil for JSON null. `Int` is 64-bit on every platform
    /// this builds for, which holds every value the u64/u32 reference emits.
    private static func int(_ value: Any?) -> Int? {
        guard let number = value as? NSNumber else { return nil }
        return number.intValue
    }

    /// A frame rate: written as an integer when whole (30) and as the
    /// shortest round-trip decimal otherwise, so it is read as a Double
    /// either way.
    private static func double(_ value: Any?) -> Double? {
        guard let number = value as? NSNumber else { return nil }
        return number.doubleValue
    }

    private static func string(_ value: Any?) -> String? {
        value as? String
    }

    private static func videoSource(_ input: [String: Any]) -> MediaPlan.VideoSource {
        MediaPlan.VideoSource(
            width: int(input["width"]) ?? 0,
            height: int(input["height"]) ?? 0,
            frameRate: double(input["frame_rate"]),
            container: string(input["container"]) ?? "",
            videoCodec: string(input["video_codec"]) ?? "",
            audioCodec: string(input["audio_codec"]),
            audioChannels: int(input["audio_channels"]),
            videoBitrate: int(input["video_bitrate"]),
            audioBitrate: int(input["audio_bitrate"]),
            sizeBytes: int(input["size_bytes"]) ?? 0,
            durationMS: int(input["duration_ms"]))
    }

    private static func audioSource(_ input: [String: Any]) -> MediaPlan.AudioSource {
        MediaPlan.AudioSource(
            container: string(input["container"]) ?? "",
            codec: string(input["codec"]) ?? "",
            channels: int(input["channels"]),
            bitrate: int(input["bitrate"]),
            sizeBytes: int(input["size_bytes"]) ?? 0,
            durationMS: int(input["duration_ms"]))
    }

    // MARK: - The file itself

    /// Every case names a function this suite knows, so a new function in
    /// the reference cannot arrive unasserted.
    @Test("every vector names a function this port checks")
    func everyFunctionIsCovered() throws {
        let all = try Self.load()
        let known: Set<String> = [
            "target_size", "target_frame_rate", "profile_video_bitrate",
            "target_video_bitrate", "target_audio_bitrate", "estimated_bitrate",
            "plan_video", "plan_audio", "sendable", "on_failure", "keep_smaller",
        ]
        #expect(all.count >= 464, "the reference printed 464 cases; \(all.count) were read")
        for vector in all {
            #expect(known.contains(vector.function), "\(vector.name): unknown function \(vector.function)")
        }
    }

    // MARK: - One test per function

    @Test("target_size")
    func targetSize() throws {
        for vector in try Self.cases(for: "target_size") {
            let size = MediaPlan.targetSize(
                width: Self.int(vector.input["width"]) ?? 0,
                height: Self.int(vector.input["height"]) ?? 0)
            #expect(size.width == Self.int(vector.expected["width"]), "\(vector.name)")
            #expect(size.height == Self.int(vector.expected["height"]), "\(vector.name)")
        }
    }

    @Test("target_frame_rate")
    func targetFrameRate() throws {
        for vector in try Self.cases(for: "target_frame_rate") {
            let rate = MediaPlan.targetFrameRate(Self.double(vector.input["frame_rate"]))
            #expect(rate == Self.double(vector.expected["frame_rate"]), "\(vector.name)")
        }
    }

    @Test("profile_video_bitrate")
    func profileVideoBitrate() throws {
        for vector in try Self.cases(for: "profile_video_bitrate") {
            let bitrate = MediaPlan.profileVideoBitrate(
                width: Self.int(vector.input["width"]) ?? 0,
                height: Self.int(vector.input["height"]) ?? 0,
                frameRate: try #require(Self.double(vector.input["frame_rate"])))
            #expect(bitrate == Self.int(vector.expected["bitrate"]), "\(vector.name)")
        }
    }

    @Test("target_video_bitrate")
    func targetVideoBitrate() throws {
        for vector in try Self.cases(for: "target_video_bitrate") {
            let bitrate = MediaPlan.targetVideoBitrate(
                width: Self.int(vector.input["width"]) ?? 0,
                height: Self.int(vector.input["height"]) ?? 0,
                frameRate: try #require(Self.double(vector.input["frame_rate"])),
                sourceBitrate: Self.int(vector.input["source_bitrate"]))
            #expect(bitrate == Self.int(vector.expected["bitrate"]), "\(vector.name)")
        }
    }

    @Test("target_audio_bitrate")
    func targetAudioBitrate() throws {
        for vector in try Self.cases(for: "target_audio_bitrate") {
            let bitrate = MediaPlan.targetAudioBitrate(
                channels: Self.int(vector.input["channels"]),
                sourceBitrate: Self.int(vector.input["source_bitrate"]))
            #expect(bitrate == Self.int(vector.expected["bitrate"]), "\(vector.name)")
        }
    }

    @Test("estimated_bitrate")
    func estimatedBitrate() throws {
        for vector in try Self.cases(for: "estimated_bitrate") {
            let bitrate = MediaPlan.estimatedBitrate(
                sizeBytes: Self.int(vector.input["size_bytes"]) ?? 0,
                durationMS: Self.int(vector.input["duration_ms"]),
                audioBitrate: Self.int(vector.input["audio_bitrate"]) ?? 0)
            #expect(bitrate == Self.int(vector.expected["bitrate"]), "\(vector.name)")
        }
    }

    @Test("plan_video: the plan, rule A, V and the target")
    func planVideo() throws {
        for vector in try Self.cases(for: "plan_video") {
            let source = Self.videoSource(vector.input)
            let plan = MediaPlan.planVideo(source)
            let word: String
            switch plan {
            case .keep: word = "keep"
            case .transcode: word = "transcode"
            case .fallback: word = "fallback"
            }
            #expect(word == Self.string(vector.expected["plan"]), "\(vector.name)")
            #expect(
                MediaPlan.withinProfile(source) == (vector.expected["within_profile"] as? Bool),
                "\(vector.name)")
            #expect(
                MediaPlan.sourceVideoBitrate(source) == Self.int(vector.expected["source_video_bitrate"]),
                "\(vector.name)")

            // The target is given for a KEPT source too, and a transcode must
            // carry exactly that target.
            let target = MediaPlan.videoTarget(source)
            if let expected = vector.expected["target"] as? [String: Any] {
                let target = try #require(target, "\(vector.name): no target, one expected")
                #expect(target.width == Self.int(expected["width"]), "\(vector.name)")
                #expect(target.height == Self.int(expected["height"]), "\(vector.name)")
                #expect(target.frameRate == Self.double(expected["frame_rate"]), "\(vector.name)")
                #expect(target.videoBitrate == Self.int(expected["video_bitrate"]), "\(vector.name)")
                #expect(target.audioBitrate == Self.int(expected["audio_bitrate"]), "\(vector.name)")
                if case .transcode(let carried) = plan {
                    #expect(carried == target, "\(vector.name)")
                }
            } else {
                #expect(target == nil, "\(vector.name): a target, none expected")
            }
        }
    }

    @Test("plan_audio: the plan, the source's rate and the target's")
    func planAudio() throws {
        for vector in try Self.cases(for: "plan_audio") {
            let source = Self.audioSource(vector.input)
            let plan = MediaPlan.planAudio(source)
            let expectedTarget = Self.int(vector.expected["target_bitrate"])
            switch plan {
            case .keep:
                #expect(Self.string(vector.expected["plan"]) == "keep", "\(vector.name)")
            case .transcode(let bitrate):
                #expect(Self.string(vector.expected["plan"]) == "transcode", "\(vector.name)")
                #expect(bitrate == expectedTarget, "\(vector.name)")
            }
            let sourceBitrate = MediaPlan.sourceAudioBitrate(source)
            #expect(sourceBitrate == Self.int(vector.expected["source_bitrate"]), "\(vector.name)")
            // Printed for kept files too: what a transcode WOULD aim at.
            #expect(
                MediaPlan.targetAudioBitrate(channels: source.channels, sourceBitrate: sourceBitrate)
                    == expectedTarget,
                "\(vector.name)")
        }
    }

    @Test("sendable")
    func sendable() throws {
        for vector in try Self.cases(for: "sendable") {
            let answer = MediaPlan.sendable(
                kind: Self.string(vector.input["kind"]) ?? "",
                container: Self.string(vector.input["container"]) ?? "",
                honest: try #require(vector.input["honest"] as? Bool),
                sizeBytes: Self.int(vector.input["size_bytes"]) ?? 0,
                ceilingBytes: Self.int(vector.input["ceiling_bytes"]) ?? 0)
            #expect(answer == (vector.expected["sendable"] as? Bool), "\(vector.name)")
        }
    }

    @Test("on_failure: rule C")
    func onFailure() throws {
        for vector in try Self.cases(for: "on_failure") {
            let answer = MediaPlan.onFailure(
                sourceSendable: try #require(vector.input["source_sendable"] as? Bool))
            let word = answer == .original ? "original" : "todays_path"
            #expect(word == Self.string(vector.expected["send"]), "\(vector.name)")
        }
    }

    @Test("keep_smaller: rule D")
    func keepSmaller() throws {
        for vector in try Self.cases(for: "keep_smaller") {
            let answer = MediaPlan.keepSmaller(
                sourceBytes: Self.int(vector.input["source_bytes"]) ?? 0,
                sourceSendable: try #require(vector.input["source_sendable"] as? Bool),
                resultBytes: Self.int(vector.input["result_bytes"]) ?? 0)
            let word = answer == .source ? "source" : "result"
            #expect(word == Self.string(vector.expected["upload"]), "\(vector.name)")
        }
    }
}
