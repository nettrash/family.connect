//
//  LocationFreshnessTests.swift
//  FamilyConnectTests
//
//  Two rules that were both broken and both untested.
//
//  The accuracy bar was dead code: `accuracy <= goodEnough || !waiters.isEmpty`
//  is always true for the whole of a wait — waiters is non-empty from the
//  first line of it — so the FIRST delivery always won, however coarse.
//
//  Worse, there was no freshness rule at all. CoreLocation hands over its
//  cached last-known position the instant updates start; it can be hours old
//  and miles away, and it arrives first. So "share my location" would answer
//  a family chat with wherever the phone last had a fix — precisely, with a
//  ±5 m badge on it. Precisely wrong is worse than roughly right, which is
//  why staleness rejects and coarseness only waits.
//

import CoreLocation
import Foundation
import Testing
@testable import FamilyConnect

@MainActor
struct LocationFreshnessTests {

    private let fresh: TimeInterval = 3
    private let stale = LocationProvider.freshEnough + 1

    // MARK: - the rule

    @Test("a fresh fix inside the accuracy bar answers immediately")
    func freshAndSharpAccepts() {
        #expect(LocationProvider.judge(accuracy: 12, age: fresh) == .accept)
        // The bar itself is inclusive: exactly 100 m names a street.
        #expect(
            LocationProvider.judge(accuracy: LocationProvider.goodEnoughMetres, age: fresh)
                == .accept)
    }

    @Test("a fresh but coarse fix is held, not sent — the bar was dead code")
    func freshAndCoarseHolds() {
        #expect(LocationProvider.judge(accuracy: 800, age: fresh) == .hold)
        #expect(
            LocationProvider.judge(accuracy: LocationProvider.goodEnoughMetres + 1, age: fresh)
                == .hold)
    }

    @Test("a stale fix is refused however sharp it is")
    func staleIsRefused() {
        // The cached fix, and the shape of the bug: pinpoint, and an hour old.
        #expect(LocationProvider.judge(accuracy: 5, age: 3600) == .ignore)
        #expect(LocationProvider.judge(accuracy: 5, age: stale) == .ignore)
        // Staleness outranks accuracy — not the other way round.
        #expect(LocationProvider.judge(accuracy: 900, age: stale) == .ignore)
        // And the boundary is still an answer.
        #expect(
            LocationProvider.judge(accuracy: 5, age: LocationProvider.freshEnough) == .accept)
    }

    @Test("a fix with no valid coordinate is not an answer at any age")
    func negativeAccuracyIsRefused() {
        #expect(LocationProvider.judge(accuracy: -1, age: 0) == .ignore)
        #expect(LocationProvider.judge(accuracy: -1, age: stale) == .ignore)
    }

    @Test("freshness matches Android's FRESH_ENOUGH_MS")
    func freshnessMatchesAndroid() {
        // android/.../LocationProvider.kt: FRESH_ENOUGH_MS = 2 * 60 * 1000L
        #expect(LocationProvider.freshEnough == 120)
    }

    // MARK: - what it does with what it holds

    @Test("a stale delivery is not held either — it must not become the answer")
    func staleIsNotHeld() {
        let provider = LocationProvider()
        provider.deliver(accuracy: 5, age: 3600, latitude: 51.5, longitude: -0.12)
        #expect(provider.bestSoFar == nil)
    }

    @Test("of two coarse fixes the sharper is held, whichever arrives first")
    func sharperCoarseFixWins() {
        let a = LocationProvider()
        a.deliver(accuracy: 900, age: 1, latitude: 51.5, longitude: -0.12)
        a.deliver(accuracy: 300, age: 1, latitude: 55.0, longitude: -1.0)
        #expect(a.bestSoFar?.accuracy == 300)
        #expect(a.bestSoFar?.fix.latitude == 55.0)

        let b = LocationProvider()
        b.deliver(accuracy: 300, age: 1, latitude: 55.0, longitude: -1.0)
        b.deliver(accuracy: 900, age: 1, latitude: 51.5, longitude: -0.12)
        #expect(b.bestSoFar?.accuracy == 300)
        #expect(b.bestSoFar?.fix.latitude == 55.0)
    }

    @Test("the held fix carries the accuracy it was actually measured at")
    func heldFixReportsItsAccuracy() {
        let provider = LocationProvider()
        provider.deliver(accuracy: 348.6, age: 1, latitude: 51.5, longitude: -0.12)
        // Rounded, not truncated: a bubble showing "±348 m" for 348.6 is fine,
        // but the value must come from the delivery rather than the bar.
        #expect(provider.bestSoFar?.fix.accuracyM == 349)
    }

    @Test(
        "a coarse fix is sent at the timeout rather than failing the send",
        .enabled(if: LocationFreshnessTests.locationNotRefused))
    func timeoutSendsTheHeldFix() async throws {
        let provider = LocationProvider()
        provider.timeout = .milliseconds(250)

        // Held, because 400 m is past the bar — the wait keeps looking.
        provider.deliver(accuracy: 400, age: 1, latitude: 51.5074, longitude: -0.1278)
        #expect(provider.bestSoFar != nil)

        let fix = try await provider.currentFix()

        #expect(fix.accuracyM == 400)
        #expect(fix.latitude == 51.5074)
        // Per wait, not per provider: the next "where are you?" must not be
        // answered with a fix held from the last one.
        #expect(provider.bestSoFar == nil)
    }

    @Test(
        "with nothing held the timeout still reports a failure",
        .enabled(if: LocationFreshnessTests.locationNotRefused))
    func timeoutWithNothingHeldFails() async throws {
        let provider = LocationProvider()
        provider.timeout = .milliseconds(250)
        // Only a stale fix arrived, so there is genuinely nothing to send.
        provider.deliver(accuracy: 5, age: 3600, latitude: 51.5, longitude: -0.12)

        await #expect(throws: LocationProvider.Failure.unavailable) {
            _ = try await provider.currentFix()
        }
    }

    /// The two waiting tests need CoreLocation not to short-circuit before
    /// the wait even starts. A machine that refused location once remembers
    /// it, so this is a SKIP condition rather than an assertion — a build
    /// machine's location setting is not a defect in the location rule.
    nonisolated(unsafe) static let locationNotRefused: Bool = {
        let status = CLLocationManager().authorizationStatus
        return status != .denied && status != .restricted
    }()
}
