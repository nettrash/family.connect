//
//  ReconnectBackoff.swift
//  FamilyConnect
//
//  Full-jitter exponential backoff for the WebSocket reconnect loop
//  (the AWS-blessed variant: delay = random(0, min(cap, base·2ⁿ))).
//
//  Full jitter — rather than the deterministic delay Geo's one-shot HTTP
//  retry uses — because reconnects here ARE a thundering-herd problem: a
//  family's devices all lose the same server at the same moment (server
//  restart, router blip) and would otherwise stampede back in lockstep.
//
//  A pure struct with an injectable random source so tests can pin the
//  exact ceiling sequence (1, 2, 4, … 30, 30) without statistics.
//

import Foundation

nonisolated struct ReconnectBackoff: Sendable {
    /// First ceiling, in seconds.
    let base: TimeInterval
    /// Ceilings never exceed this, in seconds.
    let cap: TimeInterval

    /// Injectable random source. Defaults to the system RNG; tests inject
    /// e.g. `{ range in range.upperBound }` to observe the raw ceilings.
    private let random: @Sendable (ClosedRange<Double>) -> Double

    /// Consecutive failures since the last `reset()`.
    private(set) var attempt: Int = 0

    init(
        base: TimeInterval = 1,
        cap: TimeInterval = 30,
        random: @escaping @Sendable (ClosedRange<Double>) -> Double = { Double.random(in: $0) }
    ) {
        self.base = base
        self.cap = cap
        self.random = random
    }

    /// The delay before the next reconnect attempt, advancing the attempt
    /// counter. Uniform in [0, min(cap, base·2^attempt)].
    mutating func nextDelay() -> TimeInterval {
        let ceiling = min(cap, base * pow(2, Double(attempt)))
        // Saturate rather than overflow if a socket flaps for days.
        if attempt < 62 { attempt += 1 }
        return random(0...ceiling)
    }

    /// Call on a successful connection so the next drop starts cheap again.
    mutating func reset() {
        attempt = 0
    }
}
