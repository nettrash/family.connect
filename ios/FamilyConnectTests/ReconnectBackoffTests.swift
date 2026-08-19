//
//  ReconnectBackoffTests.swift
//  FamilyConnectTests
//
//  The backoff is full jitter: delay = random(0, min(cap, base·2ⁿ)).
//  Injecting `random = { $0.upperBound }` exposes the raw ceiling
//  sequence; the real RNG path is bounds-checked statistically.
//

import Foundation
import Testing
@testable import FamilyConnect

@Suite("ReconnectBackoff")
struct ReconnectBackoffTests {

    @Test("ceilings double from base and clamp at cap: 1,2,4,8,16,30,30")
    func ceilingSequence() {
        var backoff = ReconnectBackoff(base: 1, cap: 30, random: { $0.upperBound })
        let ceilings = (0..<7).map { _ in backoff.nextDelay() }
        #expect(ceilings == [1, 2, 4, 8, 16, 30, 30])
    }

    @Test("reset() starts the sequence over")
    func reset() {
        var backoff = ReconnectBackoff(base: 1, cap: 30, random: { $0.upperBound })
        _ = backoff.nextDelay()
        _ = backoff.nextDelay()
        _ = backoff.nextDelay()
        backoff.reset()
        #expect(backoff.attempt == 0)
        #expect(backoff.nextDelay() == 1)
    }

    @Test("jittered delays stay within [0, ceiling] with the real RNG")
    func jitterBounds() {
        var backoff = ReconnectBackoff(base: 1, cap: 30)
        var expectedCeiling = 1.0
        for _ in 0..<10 {
            let delay = backoff.nextDelay()
            #expect(delay >= 0)
            #expect(delay <= expectedCeiling)
            expectedCeiling = min(30, expectedCeiling * 2)
        }
    }

    @Test("the random source receives the full-jitter range from zero")
    func fullJitterRange() {
        var seenRanges: [ClosedRange<Double>] = []
        var backoff = ReconnectBackoff(base: 2, cap: 30, random: { range in
            seenRanges.append(range)
            return range.lowerBound
        })
        _ = backoff.nextDelay()
        _ = backoff.nextDelay()
        #expect(seenRanges == [0...2, 0...4])
    }
}
