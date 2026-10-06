//
//  WaveformVectorTests.swift
//  FamilyConnectTests
//
//  AN ORACLE, NOT FOUR READINGS — a voice note's waveform (#79,
//  docs/protocol.md, "A voice note's waveform") is written once, in
//  `web/text/src/waveform.rs`, and printed by `win/tools/board-oracle`
//  (`cargo run -- waveform`) into Fixtures/waveform-vectors.json: the bytes
//  Android and Windows check too, compared by CI. The Swift port
//  (`Waveform`) answers every case exactly as the reference does — the
//  one-ulp ties, the 3001-peak recordings and the 2⁶³ millisecond position
//  included.
//
//  Read with JSONDecoder rather than JSONSerialization: the decoder parses a
//  UInt64 and a Double exactly, where an NSNumber may round 2⁶³ through a
//  double on the way.
//

import Foundation
import Testing
@testable import FamilyConnect

@Suite("Waveform vectors")
struct WaveformVectorTests {

    /// A peak: a number, or one of the three strings JSON spells the
    /// non-finite doubles with.
    struct Peak: Decodable {
        let value: Double

        init(from decoder: Decoder) throws {
            let container = try decoder.singleValueContainer()
            if let number = try? container.decode(Double.self) {
                value = number
                return
            }
            switch try container.decode(String.self) {
            case "NaN": value = .nan
            case "Infinity": value = .infinity
            case "-Infinity": value = -.infinity
            case let other:
                throw DecodingError.dataCorruptedError(
                    in: container, debugDescription: "not a peak: \(other)")
            }
        }
    }

    /// `levels` is a count for `from_peaks` and a list for `bars`.
    enum Levels: Decodable {
        case count(Int)
        case list([Int])

        init(from decoder: Decoder) throws {
            let container = try decoder.singleValueContainer()
            if let count = try? container.decode(Int.self) {
                self = .count(count)
            } else {
                self = .list(try container.decode([Int].self))
            }
        }
    }

    struct Input: Decodable {
        let dbfs: Peak?
        let samplesDbfs: [Peak]?
        let levels: Levels?
        let count: Int?
        let waveform: String?
        let level: Int?
        let positionMs: UInt64?
        let durationMs: UInt64?
        let bars: Int?
    }

    struct Expected: Decodable {
        let level: Int?
        let waveform: String?
        /// A count in `constants`, a list (or null) in `parse`.
        let levels: Levels?
        let bars: [Int]?
        let fraction: Double?
        let played: Int?
        let dbPerLevel: Double?
        let floorDbfs: Double?
        let maxLevel: Int?
        let placeholder: String?
        let placeholderLevel: Int?
    }

    struct Case: Decodable {
        let name: String
        let function: String
        let input: Input
        let expected: Expected
    }

    /// Bundle(for:) needs a class; a Swift Testing suite is a struct.
    private final class Anchor {}

    private static func load() throws -> [Case] {
        let url = Bundle(for: Anchor.self).url(forResource: "waveform-vectors", withExtension: "json")
            ?? URL(fileURLWithPath: #filePath)
                .deletingLastPathComponent()
                .appending(path: "Fixtures")
                .appending(path: "waveform-vectors.json")
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode([Case].self, from: Data(contentsOf: url))
    }

    /// Never none: a renamed function in the generator would otherwise pass
    /// by asserting nothing.
    private static func cases(_ function: String) throws -> [Case] {
        let selected = try load().filter { $0.function == function }
        #expect(!selected.isEmpty, "no vectors for \(function)")
        return selected
    }

    private static func bytes(_ values: [Int]) -> [UInt8] { values.map { UInt8(clamping: $0) } }

    /// The expected levels as a list; nil for the reference's `None`.
    private static func list(_ levels: Levels?) -> [UInt8]? {
        guard case .list(let values) = levels else { return nil }
        return bytes(values)
    }

    @Test("the file holds every function the reference prints")
    func everyFunction() throws {
        let functions = Set(try Self.load().map(\.function))
        #expect(functions == [
            "constants", "level", "from_peaks", "parse", "levels_or_placeholder", "bars",
            "bar_fraction", "played_bars",
        ])
    }

    @Test("the constants and the placeholder")
    func constants() throws {
        for vector in try Self.cases("constants") {
            let expected = vector.expected
            #expect(expected.maxLevel == Int(Waveform.maxLevel))
            #expect(expected.floorDbfs == Waveform.floorDBFS)
            #expect(expected.dbPerLevel == Waveform.dbPerLevel)
            #expect(expected.placeholderLevel == Int(Waveform.placeholderLevel))
            #expect(expected.placeholder == Waveform.encode(Waveform.placeholder))
            guard case .count(let count) = expected.levels else {
                Issue.record("constants: levels is not a count")
                continue
            }
            #expect(Waveform.levelCount == count)
        }
    }

    @Test("level: 4 dB a step, rounded half up on the fraction")
    func level() throws {
        for vector in try Self.cases("level") {
            let dbfs = try #require(vector.input.dbfs).value
            #expect(Int(Waveform.level(dbfs)) == vector.expected.level, "\(vector.name)")
        }
    }

    @Test("from_peaks: the sender's 48 characters")
    func fromPeaks() throws {
        for vector in try Self.cases("from_peaks") {
            let peaks = try #require(vector.input.samplesDbfs).map(\.value)
            guard case .count(let levels) = try #require(vector.input.levels) else {
                Issue.record("\(vector.name): levels is not a count")
                continue
            }
            #expect(Waveform.fromPeaks(peaks, levels: levels) == vector.expected.waveform, "\(vector.name)")
        }
    }

    @Test("parse: exactly 48 lowercase hex digits, or nothing")
    func parse() throws {
        for vector in try Self.cases("parse") {
            let parsed = Waveform.parse(try #require(vector.input.waveform))
            #expect(parsed == Self.list(vector.expected.levels), "\(vector.name)")
        }
    }

    @Test("levels_or_placeholder: what a reader draws")
    func levelsOrPlaceholder() throws {
        for vector in try Self.cases("levels_or_placeholder") {
            let drawn = Waveform.levelsOrPlaceholder(vector.input.waveform)
            #expect(drawn == Self.list(vector.expected.levels), "\(vector.name)")
        }
    }

    @Test("bars: the 48 levels as however many bars fit")
    func bars() throws {
        for vector in try Self.cases("bars") {
            guard case .list(let levels) = try #require(vector.input.levels) else {
                Issue.record("\(vector.name): levels is not a list")
                continue
            }
            let bars = Waveform.bars(Self.bytes(levels), count: try #require(vector.input.count))
            #expect(bars == vector.expected.bars.map(Self.bytes), "\(vector.name)")
        }
    }

    @Test("bar_fraction: (2 + level) ÷ 17, bit for bit")
    func barFraction() throws {
        for vector in try Self.cases("bar_fraction") {
            let level = UInt8(clamping: try #require(vector.input.level))
            #expect(Waveform.barFraction(level) == vector.expected.fraction, "\(vector.name)")
        }
    }

    @Test("played_bars: integer arithmetic, capped at the bar count")
    func playedBars() throws {
        for vector in try Self.cases("played_bars") {
            let played = Waveform.playedBars(
                positionMS: try #require(vector.input.positionMs),
                durationMS: try #require(vector.input.durationMs),
                bars: try #require(vector.input.bars))
            #expect(played == vector.expected.played, "\(vector.name)")
        }
    }
}
