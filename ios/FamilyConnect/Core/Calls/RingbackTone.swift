//
//  RingbackTone.swift
//  FamilyConnect
//
//  The sound the CALLER hears while the far side rings: a synthesised
//  ringback tone in the cadence of the caller's own country, so placing a
//  call in the app sounds the way placing one on the phone does. Nothing
//  on the wire — the server's `call_ringing` frame is what starts it, and
//  the answer (or any end) is what stops it (CallManager).
//
//  Pure and platform-free: the cadence table and the synthesis are pinned
//  by RingbackToneTests, and RingbackPlayer only loops the one cycle this
//  produces. The table is shared BY VALUE with Android — same names, same
//  constants, in the same order — so the two files can be diffed side by
//  side.
//
//  Four cadences cover the world well enough for a family app: the North
//  American one (ANSI), the British double ring (also used across the
//  Commonwealth and in Hong Kong / Singapore), Japan's, and the CEPT
//  single 425 Hz burst that most of Europe, Russia and China share — the
//  default for any region not listed. The region comes from the device's
//  locale, passed in so tests can choose.
//
//  Android counterpart: android/app/src/main/java/me/nettrash/familyconnect/calls/RingbackTone.kt
//

import Foundation

nonisolated enum RingbackTone {

    /// Telephone bandwidth; every tone in the table sits well under 4 kHz.
    static let sampleRate = 8000

    /// Per-tone amplitude, about -12 dBFS. Two summed tones peak near
    /// -6 dBFS, so nothing here can ever clip — and the tone sits under
    /// the far side's voice once the call connects, the way a network's
    /// ringback does.
    static let toneAmplitude = 0.25

    /// Raised-cosine ramp at every on/off edge, so a burst never clicks.
    static let rampSeconds = 0.008

    /// One cadence: the tones summed, and the on/off pattern in seconds,
    /// ALTERNATING starting with "on" — so a pattern always has an even
    /// number of entries and one cycle is its sum.
    enum Cadence: String, CaseIterable, Equatable, Sendable {
        /// US, Canada and the rest of the North American Numbering Plan.
        case ansi
        /// The British double ring: UK, Ireland, Australia, New Zealand, Hong Kong, Singapore.
        case uk
        case japan
        /// The European default — Serbia, Germany, France, Spain, Russia, China…
        case cept

        var frequenciesHz: [Double] {
            switch self {
            case .ansi: [440, 480]
            case .uk: [400, 450]
            case .japan: [400]
            case .cept: [425]
            }
        }

        var pattern: [Double] {
            switch self {
            case .ansi: [2.0, 4.0]
            case .uk: [0.4, 0.2, 0.4, 2.0]
            case .japan: [1.0, 2.0]
            case .cept: [1.0, 4.0]
            }
        }

        var cycleSeconds: Double { pattern.reduce(0, +) }
    }

    static let ansiRegions: Set<String> = [
        "US", "CA",
        "AG", "AI", "AS", "BB", "BM", "BS", "DM", "DO", "GD", "GU", "JM", "KN",
        "KY", "LC", "MP", "MS", "PR", "SX", "TC", "TT", "VC", "VG", "VI",
    ]

    static let ukRegions: Set<String> = ["GB", "IE", "AU", "NZ", "HK", "SG"]

    static let japanRegions: Set<String> = ["JP"]

    /// The cadence for an ISO 3166 region code, as `Locale.current.region`
    /// gives it. Case does not matter; nil, empty or unknown is CEPT.
    static func cadence(region: String?) -> Cadence {
        switch (region ?? "").uppercased() {
        case let code where ansiRegions.contains(code): .ansi
        case let code where ukRegions.contains(code): .uk
        case let code where japanRegions.contains(code): .japan
        default: .cept
        }
    }

    /// One full cycle of `cadence` as 16-bit mono PCM at `sampleRate`:
    /// the tones summed through the "on" segments, ramped in and out at
    /// each edge, and EXACT silence through the "off" ones. Looped end to
    /// end it rings for as long as the player lets it — the seam lands in
    /// the trailing silence, so no phase jump is ever heard.
    static func cycle(_ cadence: Cadence, sampleRate: Int = sampleRate) -> [Int16] {
        let frames = Int((cadence.cycleSeconds * Double(sampleRate)).rounded())
        var pcm = [Int16](repeating: 0, count: frames)
        let ramp = Int((rampSeconds * Double(sampleRate)).rounded())
        var cursor = 0.0
        for (index, seconds) in cadence.pattern.enumerated() {
            let start = Int((cursor * Double(sampleRate)).rounded())
            cursor += seconds
            let end = min(Int((cursor * Double(sampleRate)).rounded()), frames)
            // An "off" segment: the zeros already there.
            if index % 2 != 0 { continue }
            for n in start..<end {
                let t = Double(n) / Double(sampleRate)
                var sample = 0.0
                for hz in cadence.frequenciesHz {
                    sample += toneAmplitude * sin(2.0 * .pi * hz * t)
                }
                let envelope = min(edge(n - start, ramp: ramp), edge(end - 1 - n, ramp: ramp))
                // Clamped so a louder table could never trap here (Int16
                // conversion of an out-of-range Double is a crash, mid-ring).
                let value = (sample * envelope).clamped(to: -1...1) * Double(Int16.max)
                pcm[n] = Int16(value.rounded())
            }
        }
        return pcm
    }

    /// The cycle wrapped as a RIFF/WAVE file (PCM, mono, 16-bit), which
    /// is what AVAudioPlayer takes from memory — the whole thing is a few
    /// tens of kilobytes at telephone bandwidth, so nothing is bundled.
    static func wav(_ cadence: Cadence, sampleRate: Int = sampleRate) -> Data {
        let pcm = cycle(cadence, sampleRate: sampleRate)
        let dataSize = UInt32(pcm.count * 2)
        var data = Data(capacity: 44 + Int(dataSize))
        data.append(contentsOf: Array("RIFF".utf8))
        data.appendLittleEndian(36 + dataSize)
        data.append(contentsOf: Array("WAVE".utf8))
        data.append(contentsOf: Array("fmt ".utf8))
        data.appendLittleEndian(UInt32(16))          // PCM format chunk size
        data.appendLittleEndian(UInt16(1))           // PCM
        data.appendLittleEndian(UInt16(1))           // mono
        data.appendLittleEndian(UInt32(sampleRate))
        data.appendLittleEndian(UInt32(sampleRate * 2)) // byte rate
        data.appendLittleEndian(UInt16(2))           // block align
        data.appendLittleEndian(UInt16(16))          // bits per sample
        data.append(contentsOf: Array("data".utf8))
        data.appendLittleEndian(dataSize)
        for sample in pcm {
            data.appendLittleEndian(UInt16(bitPattern: sample))
        }
        return data
    }

    /// The envelope `distance` frames from a segment's edge: 0 at the edge,
    /// 1 past `ramp`, a half-cosine between.
    private static func edge(_ distance: Int, ramp: Int) -> Double {
        distance >= ramp ? 1.0 : 0.5 * (1.0 - cos(.pi * Double(distance) / Double(ramp)))
    }
}

nonisolated private extension Double {
    func clamped(to range: ClosedRange<Double>) -> Double {
        min(max(self, range.lowerBound), range.upperBound)
    }
}

nonisolated private extension Data {
    mutating func appendLittleEndian<T: FixedWidthInteger>(_ value: T) {
        var little = value.littleEndian
        Swift.withUnsafeBytes(of: &little) { append(contentsOf: $0) }
    }
}
