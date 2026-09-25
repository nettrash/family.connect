//
//  CallEndUITests.swift
//  FamilyConnectUITests
//
//  THE CALLEE'S SCREEN WHEN THE CALLER HANGS UP. Reported from two
//  simulators: the answered device kept its call screen for about a minute
//  after the other side ended the call — which is `CallManager`'s own guard
//  clock giving up (ringTimeout 45 + guardSlack 15), not a `call_end` being
//  acted on.
//
//  `CallManagerTests` proves the state machine ends the call when the frame
//  is handed to it, and a socket probe against the live server proves the
//  frame arrives in 27 ms. What neither can see is the seam between them —
//  the coordinator's optional `callManager`, the socket's own liveness, and
//  the `fullScreenCover` bound to `calls.phase != .idle`. That is what this
//  drives, with a real app on a real simulator against a real server.
//
//  It needs a CALLER, which this test is not: run a second client to place
//  the call and hang up. The test waits a minute, so starting the caller a
//  few seconds late is fine.
//
//  WHAT IT FOUND, 2026-09-21, iPhone 17 / iOS 27.0 simulator against the
//  live server — and it is NOT the lingering screen it was written for:
//
//    12:18:28  socket connected
//    12:18:50  offer arrives; the app rings IN-APP (screenshot: "TEST_1 /
//              Incoming call", Decline and Accept) and reports the call to
//              CallKit
//    12:18:50  CallKit REFUSES the report: Code=0, and
//              `CallKitController.reportIncoming` drops the error on the
//              floor (`{ _ in }`)
//    ~12:18:53 CallKit performs an END action for the call it never
//              accepted; `systemDidEnd()` → `performHangUp` → in `.incoming`
//              that is a DECLINE. The chat list then reads "Declined voice
//              call" and the caller is told the person declined.
//
//  Nobody touched the phone. On a simulator this is every incoming call; on
//  a device it is every call CallKit refuses to report — another app owning
//  the system call, a carrier call, a provider that has just reset — and in
//  all of them the far side is told "declined" about a phone that never
//  rang for more than a second or two. So this test FAILS on the Simulator
//  today, at the Accept step or just after it, and that failure is the bug.
//
//    TEST_RUNNER_FC_UITEST_SERVER=https://fc.nettrash.me \
//    TEST_RUNNER_FC_UITEST_TOKEN=<a session token> \
//    xcodebuild test -only-testing:FamilyConnectUITests/CallEndUITests …
//

import XCTest

final class CallEndUITests: XCTestCase {

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    @MainActor
    func testTheCallScreenClearsWhenTheCallerHangsUp() throws {
        guard let server = ProcessInfo.processInfo.environment["FC_UITEST_SERVER"],
              let token = ProcessInfo.processInfo.environment["FC_UITEST_TOKEN"]
        else {
            throw XCTSkip("set TEST_RUNNER_FC_UITEST_SERVER and TEST_RUNNER_FC_UITEST_TOKEN")
        }

        let app = XCUIApplication()
        // The session is handed in rather than typed: this test is about the
        // call, and a password field on a simulator keyboard is its own
        // adventure (see ConversationScrollUITests).
        app.launchArguments = ["-v1.serverURL", server, "-v1.sessionToken", token]
        app.launch()

        // The notification prompt blocks every later tap, and it belongs to
        // SpringBoard rather than to the app.
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        for host in [app, springboard] {
            let allow = host.buttons["Allow"]
            if allow.waitForExistence(timeout: 6) {
                allow.tap()
                break
            }
        }

        // Ring. The caller is somebody else's job; wait a generous minute.
        let accept = app.buttons["Accept"]
        let rang = accept.waitForExistence(timeout: 60)
        // Attached BEFORE the assertion, or a failure here leaves nothing
        // to look at — which is exactly what happened the first time.
        attach(app.screenshot(), rang ? "1-ringing" : "1-no-call-arrived")
        if !rang {
            add({
                let dump = XCTAttachment(string: app.debugDescription)
                dump.name = "1-element-tree"
                dump.lifetime = .keepAlways
                return dump
            }())
        }
        XCTAssertTrue(rang, "no incoming call arrived — is the caller running?")

        accept.tap()

        // Answered: the call screen is up and carries Hang Up.
        let hangUp = app.buttons["Hang Up"]
        XCTAssertTrue(hangUp.waitForExistence(timeout: 20), "the answered call shows Hang Up")
        attach(app.screenshot(), "2-answered")

        // …and now the CALLER hangs up, which this test does not do. From
        // here the only question is how long the screen stays. `endedLinger`
        // is 2 s by design, so anything past a handful of seconds is the
        // defect; the guard clock would take about 60.
        let began = Date()
        let cleared = waitForCallScreenToGo(app, timeout: 90)
        let took = Date().timeIntervalSince(began)
        attach(app.screenshot(), "3-after-the-far-side-hung-up")

        print("CALL SCREEN CLEARED: \(cleared) after \(String(format: "%.1f", took))s")
        XCTAssertTrue(cleared, "the call screen never went away")
        XCTAssertLessThan(
            took, 12,
            "the call screen outlived the call by \(String(format: "%.1f", took))s — "
                + "`endedLinger` is 2 s, so this is the far side's `call_end` not being acted on")
    }

    /// Polls for the call screen going away, rather than waiting on one
    /// element: the ended screen swaps Hang Up for nothing at all, and the
    /// chat list underneath is what comes back.
    @MainActor
    private func waitForCallScreenToGo(_ app: XCUIApplication, timeout: TimeInterval) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            let onCall = app.buttons["Hang Up"].exists || app.buttons["Accept"].exists
            if !onCall, app.staticTexts["Chats"].exists {
                return true
            }
            usleep(400_000)
        }
        return false
    }

    private func attach(_ shot: XCUIScreenshot, _ name: String) {
        let attachment = XCTAttachment(screenshot: shot)
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
