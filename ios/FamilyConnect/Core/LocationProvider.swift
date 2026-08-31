//
//  LocationProvider.swift
//  FamilyConnect
//
//  One fix, on demand — the whole of what "share my location" needs.
//
//  Deliberately NOT a running location service. This app shares a place
//  once, at the moment somebody asks it to (docs/protocol.md, "Locations":
//  a location is decided at send time and never changes), so the manager
//  starts, waits for a fix good enough to send, and stops. Nothing here
//  runs in the background, nothing subscribes, and the app asks for
//  when-in-use authorization only, which is all a one-shot needs.
//
//  Platform-free: CoreLocation is a system framework on both iOS and
//  macOS. The Mac needs the sandbox entitlement
//  `com.apple.security.personal-information.location` as well — without it
//  a sandboxed app is refused, and the refusal arrives as a plain denial
//  rather than an error that explains itself.
//

import CoreLocation
import Foundation

@MainActor
@Observable
final class LocationProvider: NSObject {

    enum Failure: Error {
        /// The person said no, or the device is configured not to allow it.
        case denied
        /// A fix never arrived in time.
        case unavailable
    }

    /// A place, as the wire wants it.
    struct Fix: Equatable, Sendable {
        let latitude: Double
        let longitude: Double
        /// Metres, or nil when the device did not report a usable one — in
        /// which case a bubble draws a plain pin rather than claiming a
        /// precision nobody measured.
        let accuracyM: Int?
    }

    /// How long to wait before giving up. A first fix indoors can take a
    /// while; beyond this the honest answer is "could not".
    private static let defaultTimeout: Duration = .seconds(20)

    /// Overridable so a test can reach the timeout branch — the one that
    /// decides between "here is the coarse fix I did get" and "could not
    /// find your location" — without sitting out twenty real seconds.
    var timeout: Duration = LocationProvider.defaultTimeout

    /// Good enough to stop waiting. A hundred metres names a street, which
    /// is what "where are you?" is asking — holding out for ten would keep
    /// somebody staring at a spinner indoors for no benefit.
    static let goodEnoughMetres: CLLocationAccuracy = 100

    /// How old a fix may be and still answer the question.
    ///
    /// CoreLocation hands over its cached last-known position the instant
    /// updates start, and that fix can be hours old and miles away. Sending
    /// one into a family chat — where the whole point is "here is where I am
    /// now" — is the location failure that actually matters. Two minutes is
    /// Android's `FRESH_ENOUGH_MS`, kept identical on purpose.
    static let freshEnough: TimeInterval = 2 * 60

    /// What to do with one delivery from CoreLocation.
    ///
    /// Pure and static so the rule can be tested: neither half of it was
    /// covered before, and both halves were wrong — the accuracy bar was
    /// dead code, and there was no freshness rule at all.
    nonisolated enum Verdict: Equatable {
        /// Good enough to answer with now.
        case accept
        /// Fresh, but coarser than the bar. Worth holding in case something
        /// better arrives, and worth sending at the timeout rather than
        /// failing.
        case hold
        /// Not an answer at all: no fix yet, or a cached position old enough
        /// that sending it would say something untrue.
        case ignore
    }

    nonisolated static func judge(
        accuracy: CLLocationAccuracy,
        age: TimeInterval,
        goodEnough: CLLocationAccuracy = LocationProvider.goodEnoughMetres,
        freshEnough: TimeInterval = LocationProvider.freshEnough
    ) -> Verdict {
        // Negative accuracy is CoreLocation's way of saying "no fix yet".
        guard accuracy >= 0 else { return .ignore }
        // Staleness rejects outright, and is checked BEFORE accuracy: a
        // pinpoint fix from an hour ago is precisely wrong, which is worse
        // than roughly right.
        guard age <= freshEnough else { return .ignore }
        return accuracy <= goodEnough ? .accept : .hold
    }

    /// The best fix seen so far that was fresh but not accurate enough.
    ///
    /// Why keep it at all: enforcing the accuracy bar by REJECTING coarse
    /// fixes would turn "±150 m" into "Could not find your location", which
    /// is worse than the imprecision — and worse than Android, which has no
    /// accuracy bar and returns whatever it gets. So the bar decides how
    /// long to keep looking, not whether to answer: a good fix finishes the
    /// wait immediately, a coarse one is held, and the timeout sends the
    /// best held rather than failing.
    /// Kept with its accuracy: `Fix.accuracyM` is optional because Android
    /// can genuinely not know, and comparing optionals here would be
    /// gymnastics over a value this path always has.
    private(set) var bestSoFar: (fix: Fix, accuracy: CLLocationAccuracy)?

    private let manager = CLLocationManager()
    private var waiters: [CheckedContinuation<Fix, Error>] = []
    private var isRunning = false

    override init() {
        super.init()
        manager.delegate = self
        manager.desiredAccuracy = kCLLocationAccuracyHundredMeters
    }

    /// Ask for one fix. Requests authorization first if it has never been
    /// asked, and answers `.denied` rather than hanging when refused.
    func currentFix() async throws -> Fix {
        switch manager.authorizationStatus {
        case .restricted, .denied:
            throw Failure.denied
        case .notDetermined:
            #if os(iOS)
            manager.requestWhenInUseAuthorization()
            #else
            // macOS has no "when in use" variant on the manager; asking for
            // a location is itself the prompt.
            manager.requestWhenInUseAuthorization()
            #endif
        default:
            break
        }

        // The timeout ends the wait through `finish`, NOT by throwing out
        // of a sibling task, and that is load-bearing.
        //
        // `withCheckedThrowingContinuation` cannot observe cancellation:
        // a task group cancelling the waiting child would leave the
        // continuation parked forever, and the group's implicit await of
        // its children means `currentFix()` would then never return at all
        // — a composer stuck on "Preparing…" and a location manager left
        // sampling for the life of the view. Routing the timeout through
        // the one teardown path resumes every waiter AND stops the manager.
        let waitFor = timeout
        let deadline = Task { @MainActor [weak self] in
            try? await Task.sleep(for: waitFor)
            guard !Task.isCancelled, let self else { return }
            // A coarse fix beats no fix: if anything fresh arrived while we
            // held out for something better, send that rather than telling
            // the reader their location could not be found.
            if let best = self.bestSoFar {
                self.finish(with: .success(best.fix))
            } else {
                self.finish(with: .failure(Failure.unavailable))
            }
        }
        defer { deadline.cancel() }
        return try await awaitFix()
    }

    private func awaitFix() async throws -> Fix {
        // The cancellation handler is the other half of the same problem:
        // a caller whose own Task goes away — a conversation view that
        // disappears mid-fix — must not leave the manager running either.
        // Android's twin does this with `invokeOnCancellation`.
        try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                waiters.append(continuation)
                if !isRunning {
                    isRunning = true
                    manager.startUpdatingLocation()
                }
            }
        } onCancel: {
            Task { @MainActor in self.finish(with: .failure(Failure.unavailable)) }
        }
    }

    /// Apply `judge` to one delivery. Separate from the delegate so tests
    /// can hand it a fix of any age and accuracy — CoreLocation will not
    /// manufacture a two-hour-old one on demand.
    func deliver(
        accuracy: CLLocationAccuracy, age: TimeInterval,
        latitude: Double, longitude: Double
    ) {
        let verdict = Self.judge(accuracy: accuracy, age: age)
        guard verdict != .ignore else { return }

        let fix = Fix(
            latitude: latitude, longitude: longitude,
            accuracyM: Int(accuracy.rounded()))

        if verdict == .accept {
            finish(with: .success(fix))
        } else {
            // Coarse but fresh: keep the best one seen. The timeout sends it
            // rather than reporting a failure.
            guard bestSoFar.map({ accuracy < $0.accuracy }) ?? true else { return }
            bestSoFar = (fix, accuracy)
        }
    }

    /// Resume everybody waiting and stop the manager.
    ///
    /// The ONE way a wait ends — a fix, a denial, the timeout, or the
    /// caller going away. Idempotent: an empty `waiters` list resumes
    /// nobody, and stopping a stopped manager is harmless.
    private func finish(with result: Result<Fix, Error>) {
        // Stop the moment there is an answer: this is a one-shot, and a
        // manager left running is a battery cost with no purpose.
        manager.stopUpdatingLocation()
        isRunning = false
        // Per-wait, not per-provider: the next "where are you?" must not be
        // answered with a fix held from the last one.
        bestSoFar = nil
        let pending = waiters
        waiters = []
        for continuation in pending {
            continuation.resume(with: result)
        }
    }
}

extension LocationProvider: CLLocationManagerDelegate {
    nonisolated func locationManager(
        _ manager: CLLocationManager, didUpdateLocations locations: [CLLocation]
    ) {
        guard let newest = locations.last else { return }
        let accuracy = newest.horizontalAccuracy
        let coordinate = newest.coordinate
        // Read here, not after the hop: `CLLocation` is not Sendable, so
        // only the values it holds may cross.
        let age = -newest.timestamp.timeIntervalSinceNow
        Task { @MainActor in
            self.deliver(
                accuracy: accuracy, age: age,
                latitude: coordinate.latitude, longitude: coordinate.longitude)
        }
    }

    nonisolated func locationManager(_ manager: CLLocationManager, didFailWithError error: Error) {
        Task { @MainActor in
            // A transient failure while waiters remain is not fatal — the
            // timeout is what ends the wait. Only a denial ends it early.
            guard (error as? CLError)?.code == .denied else { return }
            self.finish(with: .failure(Failure.denied))
        }
    }

    nonisolated func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) {
        let status = manager.authorizationStatus
        Task { @MainActor in
            switch status {
            case .restricted, .denied:
                self.finish(with: .failure(Failure.denied))
            case .authorizedAlways, .authorizedWhenInUse:
                // The prompt was answered while somebody was waiting: start
                // now, since the request that arrived before authorization
                // could not.
                if !self.waiters.isEmpty, !self.isRunning {
                    self.isRunning = true
                    manager.startUpdatingLocation()
                }
            default:
                break
            }
        }
    }
}
