//
//  Waveform.swift
//  FamilyConnect
//
//  A voice note's WAVEFORM (#79; docs/protocol.md, "A voice note's
//  waveform"): 48 levels of 0…15 that the SENDER computes from the peaks its
//  meter read while recording, sends on the upload as 48 lowercase hex digits
//  (`waveform=`), and every reader draws as bars before a byte of the
//  recording is downloaded.
//
//  The Swift port of `fc_text::waveform` (web/text/src/waveform.rs), held to
//  it by Fixtures/waveform-vectors.json, which `win/tools/board-oracle`
//  prints from the reference (`cargo run -- waveform`) — the same bytes the
//  Android and Windows ports check. An oracle, not four readings.
//
//  The arithmetic is portable bit for bit: a level is one IEEE addition, an
//  exact division by 4 and a floor — no logarithm, no `pow` — and every
//  slice boundary is integer arithmetic. Every function is total: nothing
//  here traps, and a value a reader cannot parse draws as `placeholder`.
//

import Foundation

nonisolated enum Waveform {
    /// How many levels the wire carries, one hex digit each.
    static let levelCount = 48
    /// The loudest level: full scale, 0 dBFS, written `f`.
    static let maxLevel: UInt8 = 15
    /// Level 0's centre: the same −60 dBFS the silence check uses.
    static let floorDBFS: Double = -60
    /// One level is this many dB.
    static let dbPerLevel: Double = 4
    /// The level of every bar of `placeholder`.
    static let placeholderLevel: UInt8 = 4
    /// What a reader draws for audio WITHOUT a waveform — a picked sound
    /// file, an old message, an old server's echo — and for one it cannot
    /// parse: a flat row, which claims no shape.
    static let placeholder = [UInt8](repeating: placeholderLevel, count: levelCount)

    /// One metered peak, in dBFS, as a level of 0…15: `x = (clamp(p, −60, 0)
    /// + 60) ÷ 4`, rounded half UP on the fraction. NaN is silence; +∞ is 15,
    /// −∞ is 0.
    static func level(_ dbfs: Double) -> UInt8 {
        if dbfs.isNaN { return 0 }
        let clamped = min(max(dbfs, floorDBFS), 0)
        let x = (clamped - floorDBFS) / dbPerLevel
        let whole = x.rounded(.down)
        let rounded = x - whole >= 0.5 ? whole + 1 : whole
        return min(UInt8(rounded), maxLevel)
    }

    /// Slice `i` of `count` over `n ≥ 1` items: `s = ⌊i·n/count⌋` up to, not
    /// including, `max(s + 1, ⌊(i+1)·n/count⌋)`. Integer arithmetic, widened
    /// so no product overflows.
    private static func slice(_ i: Int, n: Int, count: Int) -> Range<Int> {
        func at(_ k: Int) -> Int {
            let product = UInt64(k).multipliedFullWidth(by: UInt64(n))
            return Int(UInt64(count).dividingFullWidth(product).quotient)
        }
        let start = at(i)
        let end = max(at(i + 1), start + 1)
        return start..<end
    }

    /// `levels` reduced to `count`, each the HIGHEST its slice covers. No
    /// levels at all is `count` zeros; a level above 15 reads as 15.
    static func reduce(_ levels: [UInt8], count: Int) -> [UInt8] {
        guard count > 0 else { return [] }
        guard !levels.isEmpty else { return [UInt8](repeating: 0, count: count) }
        return (0..<count).map { i in
            min(levels[slice(i, n: levels.count, count: count)].max() ?? 0, maxLevel)
        }
    }

    /// Levels as the wire writes them: one lowercase hex digit each.
    static func encode(_ levels: [UInt8]) -> String {
        let digits = Array("0123456789abcdef")
        return String(levels.map { digits[Int(min($0, maxLevel))] })
    }

    /// The waveform a sender uploads: every metered peak (dBFS, in time
    /// order) as a `level`, reduced to `levels` slices and encoded. The wire
    /// takes exactly 48.
    static func fromPeaks(_ samplesDBFS: [Double], levels: Int = levelCount) -> String {
        encode(reduce(samplesDBFS.map(level), count: levels))
    }

    /// Exactly 48 lowercase hex digits and nothing else, as levels — or nil.
    static func parse(_ waveform: String) -> [UInt8]? {
        let bytes = Array(waveform.utf8)
        guard bytes.count == levelCount else { return nil }
        var levels = [UInt8]()
        levels.reserveCapacity(levelCount)
        for byte in bytes {
            switch byte {
            case UInt8(ascii: "0")...UInt8(ascii: "9"): levels.append(byte - UInt8(ascii: "0"))
            case UInt8(ascii: "a")...UInt8(ascii: "f"): levels.append(byte - UInt8(ascii: "a") + 10)
            default: return nil
            }
        }
        return levels
    }

    /// What a reader draws for an attachment's `waveform`: its levels, or
    /// the placeholder when it is absent or unparseable.
    static func levelsOrPlaceholder(_ waveform: String?) -> [UInt8] {
        waveform.flatMap(parse) ?? placeholder
    }

    /// The 48 levels as `count` bars.
    static func bars(_ levels: [UInt8], count: Int) -> [UInt8] {
        reduce(levels, count: count)
    }

    /// A bar's height as a fraction of the waveform's: `(2 + level) ÷ 17`.
    static func barFraction(_ level: UInt8) -> Double {
        Double(2 + min(level, maxLevel)) / 17
    }

    /// How many of `bars` bars are PLAYED at `positionMS` into a recording
    /// of `durationMS`: `⌊position·bars ÷ duration⌋`, at most `bars`; none
    /// for a recording of no known length.
    static func playedBars(positionMS: UInt64, durationMS: UInt64, bars: Int) -> Int {
        guard durationMS > 0, bars > 0 else { return 0 }
        let product = positionMS.multipliedFullWidth(by: UInt64(bars))
        // The quotient can exceed 64 bits only far past `bars`; cap first.
        if product.high >= durationMS { return bars }
        let played = durationMS.dividingFullWidth(product).quotient
        return Int(min(played, UInt64(bars)))
    }
}
