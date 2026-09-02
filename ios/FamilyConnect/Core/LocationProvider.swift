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

/// The six members of `CLLocationManager` this file actually uses.
///
/// It exists so tests can drive the real timeout, hold and teardown logic
/// without a real `CLLocationManager` underneath. They used to construct
/// one, which made them correct only where CoreLocation happens to be
/// inert — the author's Mac. On a fresh simulator, authorization is
/// `.notDetermined`, so the suite's own `requestWhenInUseAuthorization()`
/// went unanswered, turned the status `.denied`, and the timeout tests
/// failed with the wrong error. CI found that on its first run.
///
/// Production is unchanged: the initializer below defaults to a real
/// `CLLocationManager`, so both app call sites are byte-identical.
protocol LocationHardware: AnyObject {
    var authorizationStatus: CLAuthorizationStatus { get }
    var desiredAccuracy: CLLocationAccuracy { get set }
    var delegate: (any CLLocationManagerDelegate)? { get set }
    func requestWhenInUseAuthorization()
    func startUpdatingLocation()
    func stopUpdatingLocation()
}

extension CLLocationManager: LocationHardware {}

@MainActor
@Observable
final class LocationProvider: NSObject {

    enum Failure: Error {
        /// The person said no, or the device is configured not to allow it.
        case denied
        /// A fix never arrived in time.
        case unavailable
    }

    /// What the person has decided about location — settled BEFORE
    /// anything on screen starts saying it is busy.
    ///
    /// Three answers rather than two, because "not answered" is a real
    /// state here and is NOT first-run-only: "Allow Once" drops the app
    /// back to `.notDetermined` once it goes to the background, so a
    /// member who taps it meets the prompt on EVERY launch, and Location
    /// Services switched off system-wide reads `.notDetermined` too.
    /// Pricing the prompt as "once per install" is exactly what made it
    /// look affordable to spend the fix budget on (#41).
    nonisolated enum Permission: Equatable, Sendable {
        /// When-in-use or always — a hunt may start.
        case allowed
        /// Refused, now or earlier, or refused by configuration (Screen
        /// Time, a managed device). The one answer worth pointing at
        /// Settings for.
        case denied
        /// The prompt went up and nothing came back: iOS tears the alert
        /// down when the app goes to the background and does not put it
        /// back, and a device with Location Services off may never show
        /// one. Nothing was taken from the composer and nothing is
        /// running, so this is not a failure to report — it is a tap that
        /// can simply be made again.
        case unanswered
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
    ///
    /// Twenty where Android's `TIMEOUT_MS` is twenty-five, and the gap is
    /// deliberate rather than drift. Android's comment gives its own
    /// reason — `LocationManager` has no fused fast path — and it fails
    /// outright at the deadline, while this one SENDS the best coarse fix
    /// it is holding, so the extra five seconds would buy a sharper pin at
    /// most, not the difference between a pin and an error. What has to
    /// match is what the budget MEASURES, and since #41 it does: the hunt,
    /// never the prompt.
    ///
    /// Android's other head start is more apparent than real. It opens
    /// with a freshness-filtered `lastKnown()` that usually answers
    /// instantly; CoreLocation hands over the same cached position the
    /// moment updates start, and `judge` applies the same two-minute rule
    /// to it — so the fast path exists on both sides, spelled differently.
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

    private let manager: any LocationHardware
    private var waiters: [CheckedContinuation<Fix, Error>] = []
    private var isRunning = false

    /// Continuations parked on the system prompt.
    ///
    /// Deliberately a SECOND list rather than a reuse of `waiters`: the two
    /// waits are ended by different things — this one by the delegate's
    /// authorization callback, that one by a delivery or the deadline —
    /// and #41 is what came of letting one clock end both. Nothing here is
    /// ever ended by a timeout: somebody reading a privacy alert is not a
    /// hung request, and there is no honest number of seconds to give
    /// them.
    private var promptWaiters: [CheckedContinuation<Permission, Never>] = []

    init(manager: any LocationHardware = CLLocationManager()) {
        self.manager = manager
        super.init()
        self.manager.delegate = self
        self.manager.desiredAccuracy = kCLLocationAccuracyHundredMeters
    }

    /// Settle permission, raising the system prompt if it has never been
    /// answered, and waiting for the ANSWER — never for a clock.
    ///
    /// Call this BEFORE entering any state that says the composer is busy,
    /// and call `currentFix()` only on `.allowed`. That split is the whole
    /// of the #41 fix, and it is Android's shape: `ChatScreen.kt` asks for
    /// the permission itself and only calls the view model once the grant
    /// is in, so its composer stays fully usable while the dialog is up.
    /// Both iOS composers used to hold `.preparing` / `isSending` across
    /// the prompt, which disabled the attach menu, the paste door and — on
    /// the Mac — Send itself, for as long as the alert stood.
    ///
    /// `CLLocationManager` has no async authorization API: the answer
    /// arrives on `locationManagerDidChangeAuthorization`, so this parks a
    /// continuation and `authorizationDidChange(to:)` is the one place
    /// that resumes it.
    func requestPermission() async -> Permission {
        switch manager.authorizationStatus {
        case .authorizedAlways, .authorizedWhenInUse:
            return .allowed
        case .denied, .restricted:
            return .denied
        default:
            // `.notDetermined`, and whatever a later OS adds: ask.
            break
        }

        // A wait still parked on an EARLIER prompt is over the moment a new
        // one goes up: it belongs to an alert that is no longer on screen,
        // and leaving both parked would turn one answer into two shares.
        // It also bounds the parked waits to one per provider, which is
        // what keeps an unanswered prompt from accumulating.
        settlePrompt(.unanswered)

        return await withCheckedContinuation { continuation in
            // Park BEFORE asking, never after. The delegate is free to
            // answer on this very turn — a test double does — and a
            // continuation appended afterwards would be resumed by nobody:
            // the caller would hang with the manager idle. This is the
            // ordering hazard `currentFix()` warns about below, made real.
            promptWaiters.append(continuation)
            // Asked every time the status is still `.notDetermined`, which
            // is exactly when the system shows the alert again: a lapsed
            // "Allow Once" and an alert dismissed by backgrounding both
            // land here, and both deserve a fresh prompt rather than a
            // silent no-op. macOS has no "when in use" variant — asking is
            // itself the prompt there — so the one call serves both.
            manager.requestWhenInUseAuthorization()
        }
    }

    /// The app came back from the background: whatever alert was up when
    /// it left is gone, and iOS does not put it back.
    ///
    /// Called by the view that owns this provider (iOS only — see
    /// `ConversationView`), because that lifecycle belongs to the platform
    /// rather than to CoreLocation, and it keeps UIKit out of this file.
    /// The Mac deliberately does NOT call it: its alert survives a
    /// cmd-tab, so "became active" there would declare silence over an
    /// alert still on screen and then drop the Allow that followed — #41's
    /// harm, rebuilt.
    ///
    /// The status is re-read from the manager rather than remembered:
    /// somebody who left to switch the permission on in Settings comes
    /// back ALLOWED, and the live read sees that even if the delegate's
    /// callback is still a hop away. Reporting "you did not answer" over a
    /// grant that has already landed is the bug this issue is about, with
    /// a different fuse.
    func promptWasAbandoned() {
        guard manager.authorizationStatus == .notDetermined else { return }
        settlePrompt(.unanswered)
    }

    /// Resume everybody parked on the prompt — the ONE way that wait ends.
    /// Idempotent for the reason `finish` is: an empty list resumes
    /// nobody, so a spurious foreground or a second answer costs nothing.
    private func settlePrompt(_ permission: Permission) {
        let parked = promptWaiters
        promptWaiters = []
        for continuation in parked { continuation.resume(returning: permission) }
    }

    /// Ask for one fix from a device that is ALREADY allowed to give one.
    ///
    /// Authorization is not requested here any more (#41). Arming the
    /// deadline in the same breath as the prompt spent the fix budget on
    /// somebody reading a privacy alert — and an answer that arrived after
    /// the deadline found `waiters` already emptied, so the grant started
    /// nothing at all. `requestPermission()` comes first now, outside
    /// everything that gets disabled.
    func currentFix() async throws -> Fix {
        switch manager.authorizationStatus {
        case .authorizedAlways, .authorizedWhenInUse:
            break
        default:
            // Denied or restricted, or — because an "Allow Once" grant
            // lapses on the way to the background — `.notDetermined` again
            // between the settling and the hunt. All three say the same
            // thing to the reader: the switch is off, and Settings is
            // where it lives. Nothing here prompts, so no wait of ours can
            // ever be spent on an alert again.
            throw Failure.denied
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
        //
        // Created here and cancelled by the `defer` below, so its life is
        // exactly this call's: there is no clock that outlives the wait it
        // was armed for, and nothing to cancel across calls. It now
        // measures only the hunt, which is what Android's
        // `withTimeoutOrNull(TIMEOUT_MS)` has always measured.
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
        // Note the order, which #41 asked for a comment on: the clock
        // exists before `awaitFix()` registers its continuation. Both are
        // `@MainActor` and the `Task.sleep` above makes the gap
        // unreachable today, so it is not a bug — but a suspension
        // introduced between these two lines would let the deadline fire
        // into an empty `waiters`, resume nobody, and park this call for
        // ever with the manager left sampling. `requestPermission()` parks
        // BEFORE it asks for precisely this reason, where the gap is real.
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

    /// Apply one authorization change.
    ///
    /// Separate from the delegate method for the reason `deliver` is: the
    /// delegate's parameter is a concrete `CLLocationManager`, which a
    /// test that stubs the hardware cannot produce — and tests that made a
    /// real one are precisely what CI failed on.
    func authorizationDidChange(to status: CLAuthorizationStatus) {
        switch status {
        case .authorizedAlways, .authorizedWhenInUse:
            settlePrompt(.allowed)
        case .denied, .restricted:
            settlePrompt(.denied)
            // And end a hunt already in flight: the switch can be thrown in
            // Settings while the manager is sampling.
            finish(with: .failure(Failure.denied))
        default:
            // `.notDetermined` arrives for reasons that are not answers:
            // once when the delegate is first set, and again when an
            // "Allow Once" grant lapses. Resuming anybody on it is how a
            // stale callback would end a live wait.
            //
            // The branch that used to start the manager here is gone: it
            // existed because `currentFix()` raised the prompt itself, and
            // its `!waiters.isEmpty` gate was the second half of #41 — a
            // grant that arrived after the deadline had emptied the list
            // started nothing and the reader was told their location could
            // not be found. Nothing waits on a fix before it is allowed to
            // hunt now.
            break
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
        Task { @MainActor in self.authorizationDidChange(to: status) }
    }
}
