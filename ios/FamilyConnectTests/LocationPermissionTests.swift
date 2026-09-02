//
//  LocationPermissionTests.swift
//  FamilyConnectTests
//
//  The permission half of "share my location", which used to be inside the
//  fix budget (#41).
//
//  The defect was an ordering one: `currentFix()` raised the system alert
//  and armed its twenty-second deadline in the same breath, so a person
//  reading a privacy alert spent the budget meant for finding a satellite —
//  and an answer that arrived after the deadline found `waiters` already
//  emptied, so the grant started nothing and the reader was told their
//  location could not be found. Both composers made it worse by holding a
//  busy state across the whole call: the phone's `.preparing` closes the
//  attach menu and the paste door, and the Mac's `isSending` also disables
//  Send.
//
//  So these tests are mostly about things that must NOT happen — no alert
//  from a hunt, no hunt from an alert — which is why the double counts.
//  NOTHING here waits on a wall clock: every ordering is arranged by acting
//  from inside the hardware's own hooks, at the one moment a wait is
//  provably parked. The suite runs concurrently on one main actor, and a
//  test that slept would be timing its neighbours' work as well as its own
//  (#42).
//

import CoreLocation
import Foundation
import Testing
@testable import FamilyConnect

@MainActor
struct LocationPermissionTests {

    /// The double stays where #42 put it. Moving it here would rewrite that
    /// file's diff for no behavioural gain, and it is still the same
    /// hardware being stubbed.
    private typealias Hardware = LocationFreshnessTests.InertLocationHardware

    /// Somewhere to hand a second wait back out of a hook, since the hook is
    /// the only place that runs while the first wait is parked.
    @MainActor
    private final class WaitBox {
        var task: Task<LocationProvider.Permission, Never>?
    }

    // MARK: - answers that need no alert

    @Test("permission already held is settled without an alert")
    func allowedNeedsNoPrompt() async {
        let hardware = Hardware()
        let provider = LocationProvider(manager: hardware)

        let outcome = await provider.requestPermission()

        #expect(outcome == .allowed)
        // The alert is a once-per-decision thing; asking a member who has
        // already said yes would be noise, and on iOS it is a no-op anyway.
        #expect(hardware.authorizationRequests == 0)
        // And settling permission is not a hunt. This is the whole reason
        // the two are separable.
        #expect(hardware.startCount == 0)
    }

    @Test("a refusal is reported without asking again")
    func deniedNeedsNoPrompt() async {
        for status in [CLAuthorizationStatus.denied, .restricted] {
            let hardware = Hardware()
            hardware.authorizationStatus = status
            let provider = LocationProvider(manager: hardware)

            let outcome = await provider.requestPermission()

            #expect(outcome == .denied)
            #expect(hardware.authorizationRequests == 0)
        }
    }

    // MARK: - the alert, and the answer to it

    @Test("the answer arrives on the delegate's hop and resumes the wait")
    func grantResumesTheWait() async {
        let hardware = Hardware()
        hardware.authorizationStatus = .notDetermined
        let provider = LocationProvider(manager: hardware)
        hardware.onRequestAuthorization = { [weak hardware, weak provider] in
            // The person taps Allow. CoreLocation updates the status and
            // calls the delegate on a LATER turn of the main actor, which
            // the Task models — and which is the ordering that matters: the
            // continuation is parked before the ask, so an answer on this
            // turn or any later one lands on somebody.
            hardware?.authorizationStatus = grantedLocationAuthorization
            Task { @MainActor in provider?.authorizationDidChange(to: grantedLocationAuthorization) }
        }

        let outcome = await provider.requestPermission()

        #expect(outcome == .allowed)
        // Exactly one alert. Not two, which is what a design that asked
        // again on the way into the hunt would produce.
        #expect(hardware.authorizationRequests == 1)
        // And still no hunt: the caller starts that itself, once it has an
        // answer and has said the composer is busy.
        #expect(hardware.startCount == 0)
    }

    @Test("a refusal at the alert is an answer, not a failure to find a fix")
    func refusalAtThePromptIsDenied() async {
        let hardware = Hardware()
        hardware.authorizationStatus = .notDetermined
        let provider = LocationProvider(manager: hardware)
        hardware.onRequestAuthorization = { [weak hardware, weak provider] in
            hardware?.authorizationStatus = .denied
            Task { @MainActor in provider?.authorizationDidChange(to: .denied) }
        }

        let outcome = await provider.requestPermission()

        #expect(outcome == .denied)
        #expect(hardware.startCount == 0)
    }

    // MARK: - the alert nobody answers

    @Test("an alert torn down by backgrounding ends the wait, and no clock does")
    func abandonedPromptSettles() async {
        let hardware = Hardware()
        hardware.authorizationStatus = .notDetermined
        let provider = LocationProvider(manager: hardware)
        hardware.onRequestAuthorization = { [weak provider] in
            // The app goes to the background with the alert up and comes
            // back. iOS threw the alert away and will not put it back;
            // ConversationView's scenePhase hook says so. Note there is no
            // timeout in this file to reach — an unanswered alert is ended
            // by a lifecycle event or by the next ask, never by a fuse.
            Task { @MainActor in provider?.promptWasAbandoned() }
        }

        let outcome = await provider.requestPermission()

        #expect(outcome == .unanswered)
        #expect(hardware.startCount == 0)
        // Nothing was left running, so nothing had to be stopped either.
        #expect(hardware.stopCount == 0)
    }

    @Test("a permission granted in Settings outranks the foreground's 'no answer'")
    func lateGrantOutranksAbandonment() async {
        let hardware = Hardware()
        hardware.authorizationStatus = .notDetermined
        let provider = LocationProvider(manager: hardware)
        hardware.onRequestAuthorization = { [weak hardware, weak provider] in
            // The person leaves for Settings and switches location on
            // there. The status is the daemon's answer and is already true
            // when the app returns; the delegate's callback is still a hop
            // behind. Declaring silence on that order is #41's own bug with
            // a longer fuse, and the live status read is what prevents it.
            hardware?.authorizationStatus = grantedLocationAuthorization
            Task { @MainActor in
                provider?.promptWasAbandoned()
                provider?.authorizationDidChange(to: grantedLocationAuthorization)
            }
        }

        let outcome = await provider.requestPermission()

        #expect(outcome == .allowed)
    }

    @Test("a foreground with nothing parked settles nobody")
    func abandonmentWithoutAWaitIsHarmless() async {
        let hardware = Hardware()
        hardware.authorizationStatus = .notDetermined
        let provider = LocationProvider(manager: hardware)

        // Every return from the background calls this, whether or not an
        // alert was ever up. It has to be free.
        provider.promptWasAbandoned()

        #expect(hardware.authorizationRequests == 0)
        #expect(hardware.startCount == 0)
    }

    @Test("a second ask ends the wait the first one left parked")
    func secondAskSupersedesTheFirst() async {
        let hardware = Hardware()
        hardware.authorizationStatus = .notDetermined
        let provider = LocationProvider(manager: hardware)
        let box = WaitBox()
        hardware.onRequestAuthorization = { [weak hardware, weak provider] in
            // One-shot: this fires again for the second ask, and must not
            // start a third.
            hardware?.onRequestAuthorization = nil
            // A second tap, made while the first wait is parked — which the
            // Mac allows, since its alert is not modal. Two taps must not
            // become two shares, and the wait belonging to an alert that is
            // no longer on screen must not be resumed by an answer given to
            // a later one.
            box.task = Task { @MainActor in await provider?.requestPermission() ?? .unanswered }
        }

        let first = await provider.requestPermission()
        #expect(first == .unanswered)

        // The second wait is parked by now — it parked before resuming the
        // first — so the answer belongs to it alone.
        provider.authorizationDidChange(to: grantedLocationAuthorization)
        let second = await box.task?.value
        #expect(second == .allowed)
        #expect(hardware.authorizationRequests == 2)
    }

    // MARK: - the hunt, which no longer asks for anything

    @Test("a fix hunt never raises the alert — the budget is the hunt's")
    func currentFixNeverPrompts() async {
        let hardware = Hardware()
        hardware.authorizationStatus = .notDetermined
        let provider = LocationProvider(manager: hardware)
        // A fuse, not a wait: the correct path throws at once. If this ever
        // regresses to hunting, the test fails in a quarter of a second
        // instead of hanging the suite for twenty.
        provider.timeout = .milliseconds(250)

        await #expect(throws: LocationProvider.Failure.denied) {
            _ = try await provider.currentFix()
        }

        // The defect, pinned: the alert used to go up here, inside the same
        // call whose deadline was already ticking.
        #expect(hardware.authorizationRequests == 0)
        #expect(hardware.startCount == 0)
    }

    @Test("permission taken away mid-hunt ends the hunt and stops the hardware")
    func revocationEndsALiveHunt() async {
        let hardware = Hardware()
        let provider = LocationProvider(manager: hardware)
        // The same fuse: if the revocation stopped ending the wait, this
        // would fail as `.unavailable` rather than hang.
        provider.timeout = .milliseconds(250)
        hardware.onStartUpdating = { [weak provider] in
            // The switch is thrown in Settings while the manager is
            // sampling. This is the branch that survives in the delegate
            // now that the "start the hunt late" one is gone, so it is
            // worth proving it still ends the wait.
            Task { @MainActor in provider?.authorizationDidChange(to: .denied) }
        }

        await #expect(throws: LocationProvider.Failure.denied) {
            _ = try await provider.currentFix()
        }

        #expect(hardware.startCount == 1)
        #expect(hardware.stopCount >= 1)
    }

    // MARK: - the shape both composers now follow

    /// Not a view test — there is no harness for one here — but the rule
    /// the composer's guard is made of.
    ///
    /// `shareLocation()` used to open with `guard mediaState == .idle`,
    /// which no other attachment door does, and only the strip's Dismiss
    /// button returns `.failed` to `.idle`. So a reader who was told to
    /// turn the permission on in Settings, did, came back and tapped
    /// Location got silence. The guard is now `composerIsBusy`, the same
    /// expression the attach menu and the paste door use — and it is false
    /// for a dismissible failure, which is what makes the second tap work.
    #if os(iOS)
    @Test("a dismissible failure does not close the door on the next try")
    func failedStateAllowsARetry() {
        #expect(!ConversationView.MediaSendState.failed("Could not find your location.")
            .blocksComposer)
        // While a send genuinely in flight still does.
        #expect(ConversationView.MediaSendState.preparing.blocksComposer)
        #expect(ConversationView.MediaSendState.uploading(nil).blocksComposer)
    }
    #endif
}
