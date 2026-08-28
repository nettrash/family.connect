//
//  RingbackToneTests.swift
//  FamilyConnectTests
//
//  The cadence table and the synthesis, pinned: which country hears
//  which ring, that a cycle is exactly as long as its pattern says, that
//  the bursts sound and the gaps are silent, that nothing clips, and that
//  the WAV wrapper is one AVAudioPlayer will take. The Android side pins
//  the same numbers (RingbackToneTest.kt) — the table is shared by value.
//

import Foundation
import Testing
@testable import FamilyConnect

@Suite("Ringback tone")
struct RingbackToneTests {

    @Test("the region picks the cadence; unknown, empty and nil are CEPT")
    func regionTable() {
        #expect(RingbackTone.cadence(region: "US") == .ansi)
        #expect(RingbackTone.cadence(region: "CA") == .ansi)
        #expect(RingbackTone.cadence(region: "PR") == .ansi)
        #expect(RingbackTone.cadence(region: "GB") == .uk)
        #expect(RingbackTone.cadence(region: "AU") == .uk)
        #expect(RingbackTone.cadence(region: "JP") == .japan)
        #expect(RingbackTone.cadence(region: "RS") == .cept)
        #expect(RingbackTone.cadence(region: "DE") == .cept)
        #expect(RingbackTone.cadence(region: "CN") == .cept)
        #expect(RingbackTone.cadence(region: "ZZ") == .cept)
        #expect(RingbackTone.cadence(region: "") == .cept)
        #expect(RingbackTone.cadence(region: nil) == .cept)
        // Case is the locale's problem, not the table's.
        #expect(RingbackTone.cadence(region: "us") == .ansi)
        #expect(RingbackTone.cadence(region: "gb") == .uk)
    }

    @Test("the four cadences carry the standard tones and patterns")
    func cadenceTable() {
        #expect(RingbackTone.Cadence.ansi.frequenciesHz == [440, 480])
        #expect(RingbackTone.Cadence.ansi.pattern == [2.0, 4.0])
        #expect(RingbackTone.Cadence.uk.frequenciesHz == [400, 450])
        #expect(RingbackTone.Cadence.uk.pattern == [0.4, 0.2, 0.4, 2.0])
        #expect(RingbackTone.Cadence.japan.frequenciesHz == [400])
        #expect(RingbackTone.Cadence.japan.pattern == [1.0, 2.0])
        #expect(RingbackTone.Cadence.cept.frequenciesHz == [425])
        #expect(RingbackTone.Cadence.cept.pattern == [1.0, 4.0])
        for cadence in RingbackTone.Cadence.allCases {
            #expect(cadence.pattern.count % 2 == 0, "\(cadence): on/off pairs")
            #expect(cadence.pattern.allSatisfy { $0 > 0 }, "\(cadence)")
        }
        #expect(RingbackTone.Cadence.uk.cycleSeconds == 3.0)
        #expect(RingbackTone.Cadence.cept.cycleSeconds == 5.0)
    }

    @Test("a cycle is exactly as long as its pattern, at 8 kHz")
    func cycleLength() {
        for cadence in RingbackTone.Cadence.allCases {
            let pcm = RingbackTone.cycle(cadence)
            #expect(pcm.count == Int((cadence.cycleSeconds * 8000).rounded()), "\(cadence)")
        }
        #expect(RingbackTone.cycle(.ansi).count == 48_000)
        #expect(RingbackTone.cycle(.uk).count == 24_000)
        #expect(RingbackTone.cycle(.japan).count == 24_000)
        #expect(RingbackTone.cycle(.cept).count == 40_000)
        #expect(RingbackTone.cycle(.cept, sampleRate: 16000).count == 80_000)
    }

    @Test("CEPT by literal sample index: the burst is frames 0..<8000, the gap 8000..<40000")
    func ceptLiteralBoundaries() {
        let pcm = RingbackTone.cycle(.cept)
        #expect(pcm.count == 40_000)
        #expect(pcm[0] == 0, "foot of the ramp")
        // Not 4000: at 0.5 s a 425 Hz tone is exactly at a zero crossing.
        #expect(abs(Int(pcm[4004])) > 3000, "mid-burst is loud")
        #expect(abs(Int(pcm[7999])) < 100, "last burst sample is at the foot of the ramp")
        #expect(pcm[8000] == 0)
        #expect(pcm[8000..<40_000].allSatisfy { $0 == 0 })
        // Something in the burst, not just the ends.
        #expect(pcm[64..<7936].contains { abs(Int($0)) > 3000 })
    }

    @Test("bursts sound, gaps are exactly silent, the edges are ramped")
    func onAndOff() {
        for cadence in RingbackTone.Cadence.allCases {
            let rate = RingbackTone.sampleRate
            let pcm = RingbackTone.cycle(cadence)
            var cursor = 0.0
            for (index, seconds) in cadence.pattern.enumerated() {
                let start = Int((cursor * Double(rate)).rounded())
                cursor += seconds
                let end = Int((cursor * Double(rate)).rounded())
                let segment = pcm[start..<end]
                if index % 2 == 0 {
                    let peak = segment.map { abs(Int($0)) }.max() ?? 0
                    #expect(peak > 3000, "\(cadence) burst \(index) is audible")
                    // The first and last samples of a burst are at the
                    // foot of the ramp — a click would be a full-scale
                    // sample there.
                    #expect(abs(Int(segment.first ?? 0)) < 100, "\(cadence) burst \(index) ramps in")
                    #expect(abs(Int(segment.last ?? 0)) < 100, "\(cadence) burst \(index) ramps out")
                } else {
                    #expect(segment.allSatisfy { $0 == 0 }, "\(cadence) gap \(index) is silent")
                }
            }
        }
    }

    @Test("two summed tones stay under -6 dBFS; one under -12 dBFS")
    func amplitudeBudget() {
        let twoTones = RingbackTone.cycle(.ansi).map { abs(Int($0)) }.max() ?? 0
        let oneTone = RingbackTone.cycle(.cept).map { abs(Int($0)) }.max() ?? 0
        #expect(twoTones <= Int(Double(Int16.max) * 0.5) + 1)
        #expect(oneTone <= Int(Double(Int16.max) * 0.25) + 1)
        #expect(twoTones > oneTone)
    }

    @Test("the WAV wrapper is a well-formed 8 kHz mono 16-bit RIFF file")
    func wavHeader() {
        let data = RingbackTone.wav(.cept)
        let frames = RingbackTone.cycle(.cept).count
        #expect(data.count == 44 + frames * 2)
        #expect(String(decoding: data[0..<4], as: UTF8.self) == "RIFF")
        #expect(data.uint32(at: 4) == UInt32(36 + frames * 2))
        #expect(String(decoding: data[8..<12], as: UTF8.self) == "WAVE")
        #expect(String(decoding: data[12..<16], as: UTF8.self) == "fmt ")
        #expect(data.uint32(at: 16) == 16)
        #expect(data.uint16(at: 20) == 1, "PCM")
        #expect(data.uint16(at: 22) == 1, "mono")
        #expect(data.uint32(at: 24) == 8000)
        #expect(data.uint32(at: 28) == 16000, "byte rate")
        #expect(data.uint16(at: 32) == 2, "block align")
        #expect(data.uint16(at: 34) == 16, "bits per sample")
        #expect(String(decoding: data[36..<40], as: UTF8.self) == "data")
        #expect(data.uint32(at: 40) == UInt32(frames * 2))
        // The first sample of the burst is the foot of the ramp: zero.
        #expect(data.uint16(at: 44) == 0)
        // And the payload is the cycle, little-endian.
        let pcm = RingbackTone.cycle(.cept)
        let probe = 4000 // 0.5 s in: mid-burst
        #expect(data.uint16(at: 44 + probe * 2) == UInt16(bitPattern: pcm[probe]))
    }
}

private extension Data {
    func uint32(at offset: Int) -> UInt32 {
        UInt32(self[offset]) | UInt32(self[offset + 1]) << 8 | UInt32(self[offset + 2]) << 16 | UInt32(self[offset + 3]) << 24
    }
    func uint16(at offset: Int) -> UInt16 {
        UInt16(self[offset]) | UInt16(self[offset + 1]) << 8
    }
}
