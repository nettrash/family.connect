//
//  StoreScreenshotUITests.swift
//  FamilyConnectUITests
//
//  Captures the App Store screenshot set. Not a regression test — it
//  asserts only enough to fail loudly rather than photograph the wrong
//  screen, and its real output is the attachments.
//
//  Run against the seeded fixture, once per device class:
//
//    server/scripts/seed-store-screenshots.sh
//    cd ios && DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer \
//      TEST_RUNNER_FC_UITEST_SERVER=http://127.0.0.1:8091 \
//      xcodebuild test -project FamilyConnect.xcodeproj -scheme FamilyConnect \
//        -destination 'id=<SIMULATOR UDID>' \
//        -only-testing:FamilyConnectUITests/StoreScreenshotUITests \
//        -resultBundlePath /tmp/shots.xcresult
//    xcrun xcresulttool export attachments \
//      --path /tmp/shots.xcresult --output-path /tmp/shots
//
//  ADDRESS SIMULATORS BY UDID, never by name+OS: two runtimes are
//  installed twice on this machine (26.1 and 26.4 each appear under two
//  builds), so `name=…,OS=26.1` resolves ambiguously.
//
//    6.9" iPhone  iPhone 17 Pro Max      1320x2868  C6681051-89A8-4D26-BD29-D9CB20EF4D8B
//    13"  iPad    iPad Pro 13-inch (M5)  2064x2752  B5839D57-056A-4C14-AB0C-A9F40EC50872
//
//  Both live on iOS 26.5, so one runtime covers both required slots.
//

import XCTest

final class StoreScreenshotUITests: XCTestCase {

    /// The seeded family. Kept here so a change to the seed script fails
    /// this test by name rather than by photographing an empty list.
    private let familyName = "The Harpers"

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    @MainActor
    func testCaptureStoreScreenshots() throws {
        guard let server = ProcessInfo.processInfo.environment["FC_UITEST_SERVER"] else {
            throw XCTSkip("set TEST_RUNNER_FC_UITEST_SERVER to a seeded server URL")
        }

        let app = XCUIApplication()
        app.launchArguments = [
            "--uitest-reset", "-v1.serverURL", server,
            // Force English, for two reasons. Store screenshots are
            // required only for the PRIMARY localization, so they must not
            // come out in whatever language the simulator happens to hold.
            // And the keyboard follows the language: an iPad left on a
            // Russian keyboard types Cyrillic into the username field, and
            // the only symptom is a login that never happens.
            "-AppleLanguages", "(en)",
            "-AppleLocale", "en_US",
        ]
        app.launch()

        try logIn(app, username: "nora", password: "password123")

        // 1 — the chat list.
        let family = familyRow(in: app)
        XCTAssertTrue(family.waitForExistence(timeout: 20), "the seeded family should be listed")
        capture("01-chats")

        // 2 — the family thread, which carries the album, the poll, the
        //     location and a reacted bubble.
        family.tap()
        sleep(3) // let the thread settle: resync, layout, image decode
        capture("02-family-chat")

        // 3 — scrolled up to the album and the poll.
        app.swipeDown()
        sleep(2)
        capture("03-photos-and-poll")

        back(in: app)

        // 4 — the board.
        if tapFirst(in: app, labels: ["Board"]) {
            sleep(2)
            capture("04-board")
        }

        // RELAUNCH between sections rather than unwinding by hand. A sheet
        // left half-dismissed swallows the next tap, and the symptom is a
        // tap that "succeeds" and navigates nowhere — which cost an hour
        // on iPad. Relaunching without --uitest-reset keeps the session,
        // so this is a fresh navigation stack and a logged-in app.
        app.terminate()
        app.launchArguments = [
            "-v1.serverURL", server,
            "-AppleLanguages", "(en)",
            "-AppleLocale", "en_US",
        ]
        app.launch()
        XCTAssertTrue(familyRow(in: app).waitForExistence(timeout: 20),
                      "the session should survive a relaunch")

        // 5 — the family roster, where Safety lives.
        XCTAssertTrue(openSettings(app), "Settings should be on screen")
        capture("06-settings")

        // Scroll INSIDE the sheet. On iPad Settings is a small centred
        // sheet floating over the chat list, so `app.swipeUp()` scrolls
        // the list BEHIND it and the row never arrives. A coordinate drag
        // through the middle of the screen lands on the sheet itself.
        var roster = rosterRow(in: app)
        for _ in 0..<4 where !roster.exists {
            let from = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.62))
            let to = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.34))
            from.press(forDuration: 0.05, thenDragTo: to)
            sleep(1)
            roster = rosterRow(in: app)
        }
        if roster.waitForExistence(timeout: 5) {
            roster.tap()
            sleep(2)
            capture("05-family")
        } else {
            XCTFail("the family roster row was not reachable — the set is incomplete")
        }
    }

    // MARK: - Navigation

    @MainActor
    private func logIn(_ app: XCUIApplication, username: String, password: String) throws {
        let field = app.textFields["Username"]
        XCTAssertTrue(field.waitForExistence(timeout: 20), "should land on the auth screen")
        field.tap()
        field.typeText(username)

        let secure = app.secureTextFields["Password"]
        XCTAssertTrue(secure.waitForExistence(timeout: 5), "password field should exist")
        secure.tap()
        secure.typeText(password)

        // VERIFY BEFORE SUBMITTING. AutoFill can steal focus between the
        // two taps, and the failure mode is silent: the password lands in
        // the username field, the server rejects it, and the only symptom
        // is "Something went wrong" on a screen that looks fine. Reading
        // the field back turns that into a message naming what was typed.
        XCTAssertEqual(field.value as? String, username,
                       "the username field holds something else — AutoFill or focus stole the keystrokes")

        // Two elements carry "Log In" — the mode segment and the submit
        // button. Match order is not visual order, so take the lowest.
        //
        // The keyboard's return key does NOT submit this form: tapping it
        // leaves the screen exactly as it was, which reads as a silent
        // failure three assertions later.
        let submits = app.buttons.matching(NSPredicate(format: "label == 'Log In'"))
            .allElementsBoundByIndex
        XCTAssertFalse(submits.isEmpty, "no Log In button on the auth screen")
        submits.max(by: { $0.frame.minY < $1.frame.minY })?.tap()
        dismissSavePassword(app)

        // Fail here rather than three screens later: an auth error leaves
        // the app on this screen and every later query then misses.
        let stillOnAuth = app.staticTexts["Something went wrong. Try again."]
        if stillOnAuth.waitForExistence(timeout: 3) {
            capture("00-login-failed")
            XCTFail("login was rejected — is the seed fresh and the server on \(app.launchArguments)?")
        }
    }

    /// iOS offers to save the password it just saw; the sheet blocks every
    /// later tap and may belong to the app or to SpringBoard.
    @MainActor
    private func dismissSavePassword(_ app: XCUIApplication) {
        for host in [app, XCUIApplication(bundleIdentifier: "com.apple.springboard")] {
            let notNow = host.buttons["Not Now"]
            if notNow.waitForExistence(timeout: 4) {
                notNow.tap()
                return
            }
        }
    }

    /// Open Settings, retrying the tap.
    ///
    /// The toolbar button is a real, hittable 36pt button, but a tap sent
    /// while the list is still settling after a launch is swallowed
    /// silently — XCUITest reports the tap as delivered and nothing
    /// happens. Retrying is the difference between a flaky set and a
    /// reproducible one.
    @MainActor
    private func openSettings(_ app: XCUIApplication) -> Bool {
        // Anchor on the NAVIGATION BAR, not on a control further down.
        // "Log Out" sits at the bottom of a long Form and SwiftUI does not
        // render it until it scrolls near the viewport, so waiting for it
        // reports "Settings did not open" about a Settings screen that is
        // plainly open.
        func isOpen() -> Bool {
            app.navigationBars["Settings"].firstMatch.exists
                || app.staticTexts["Privacy"].firstMatch.exists
        }
        for attempt in 0..<3 {
            if isOpen() { return true }
            sleep(2) // let the presenting screen come to rest first
            let button = app.buttons["Settings"].firstMatch
            guard button.waitForExistence(timeout: 5) else { continue }
            button.tap()
            for _ in 0..<6 {
                if isOpen() { return true }
                usleep(500_000)
            }
            XCTContext.runActivity(named: "Settings did not open (attempt \(attempt + 1))") { _ in }
        }
        return false
    }

    /// The row into the roster. Its label depends on who is looking:
    /// an owner sees "Manage Family", a plain member "Family Members".
    @MainActor
    private func rosterRow(in app: XCUIApplication) -> XCUIElement {
        let owner = app.buttons["Manage Family"].firstMatch
        return owner.exists ? owner : app.buttons["Family Members"].firstMatch
    }

    @MainActor
    private func familyRow(in app: XCUIApplication) -> XCUIElement {
        app.collectionViews.staticTexts[familyName].firstMatch
    }

    @MainActor
    private func back(in app: XCUIApplication) {
        let back = app.navigationBars.buttons.firstMatch
        if back.exists { back.tap() }
        sleep(1)
    }

    @MainActor
    private func dismissSheet(_ app: XCUIApplication) {
        for label in ["Done", "Close", "Cancel"] {
            let button = app.buttons[label]
            if button.exists && button.isHittable { button.tap(); sleep(1); return }
        }
        app.swipeDown()
        sleep(1)
    }

    /// Taps the first of several candidate labels that is actually there.
    /// The same control is a toolbar button on one device class and a row
    /// on another, and its label differs for an owner and a member.
    @MainActor
    @discardableResult
    private func tapFirst(in app: XCUIApplication, labels: [String]) -> Bool {
        for label in labels {
            for element in [app.buttons[label], app.staticTexts[label]] {
                if element.waitForExistence(timeout: 3), element.isHittable {
                    element.tap()
                    return true
                }
            }
        }
        XCTFail("none of \(labels) was reachable — the screenshot set is incomplete")
        return false
    }

    // MARK: - Capture

    /// THE WHOLE DISPLAY, not the app's frame: a store slot expects the
    /// status bar, and `app.screenshot()` crops it out.
    ///
    /// `.keepAlways` is load-bearing. The default lifetime is
    /// `.deleteOnSuccess`, so a screenshot test that PASSES throws its own
    /// attachments away and the result bundle comes back empty.
    @MainActor
    private func capture(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
