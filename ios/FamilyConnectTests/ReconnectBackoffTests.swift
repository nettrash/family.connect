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

    // MARK: - What earns a reset

    /// The bug: a proxy that accepts the upgrade and drops immediately used
    /// to reset the ceiling every cycle, so it never climbed and the socket
    /// reconnected about twice a second forever, resyncing each time.
    @Test("an accept-then-drop connection earns nothing")
    func acceptThenDropEarnsNothing() {
        let connectedAt = Date()
        #expect(!ReconnectBackoff.earnsReset(
            connectedAt: connectedAt,
            now: connectedAt.addingTimeInterval(0.05),
            durableAfter: 10))
    }

    @Test("a connection that lasted earns its reset")
    func durableConnectionEarnsReset() {
        let connectedAt = Date()
        #expect(ReconnectBackoff.earnsReset(
            connectedAt: connectedAt,
            now: connectedAt.addingTimeInterval(10),
            durableAfter: 10))
        #expect(ReconnectBackoff.earnsReset(
            connectedAt: connectedAt,
            now: connectedAt.addingTimeInterval(3600),
            durableAfter: 10))
    }

    /// A dial that never handshook cannot have proved anything.
    @Test("never having connected earns nothing")
    func noConnectionEarnsNothing() {
        #expect(!ReconnectBackoff.earnsReset(connectedAt: nil, durableAfter: 10))
    }

    /// The storm, end to end: repeated accept-then-drop must climb to the
    /// cap rather than sitting at the first ceiling forever.
    @Test("repeated accept-then-drop climbs to the cap")
    func stormClimbsToCap() {
        var backoff = ReconnectBackoff(base: 1, cap: 30, random: { $0.upperBound })
        var ceilings: [TimeInterval] = []
        for _ in 0..<8 {
            let connectedAt = Date()
            // The endpoint accepts and drops in milliseconds, every time.
            if ReconnectBackoff.earnsReset(
                connectedAt: connectedAt,
                now: connectedAt.addingTimeInterval(0.02),
                durableAfter: 10) {
                backoff.reset()
            }
            ceilings.append(backoff.nextDelay())
        }
        #expect(ceilings == [1, 2, 4, 8, 16, 30, 30, 30])
    }

    /// …and one good connection puts it back to cheap.
    @Test("a durable connection between drops restores the cheap ceiling")
    func durableConnectionResetsTheClimb() {
        var backoff = ReconnectBackoff(base: 1, cap: 30, random: { $0.upperBound })
        for _ in 0..<5 { _ = backoff.nextDelay() }
        #expect(backoff.nextDelay() == 30)

        let connectedAt = Date()
        if ReconnectBackoff.earnsReset(
            connectedAt: connectedAt,
            now: connectedAt.addingTimeInterval(11),
            durableAfter: 10) {
            backoff.reset()
        }
        #expect(backoff.nextDelay() == 1)
    }
}
