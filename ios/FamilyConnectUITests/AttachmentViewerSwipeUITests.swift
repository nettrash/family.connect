//
//  AttachmentViewerSwipeUITests.swift
//  FamilyConnectUITests
//
//  Manual regression for the album viewer's paging: a swipe ON the photo
//  (the centre of the screen, not the letterbox around it) must turn the
//  page; a drag while zoomed must pan instead of paging; and zooming back
//  out must hand the swipe back to the pager.
//
//  Needs a running server seeded with the fixture this asserts against
//  (server/scripts/seed-album-uitest.sh: user "junior"/"password123" in a
//  family "The Smiths" whose chat holds ONE album message of three
//  photos), so it skips unless the runner passes the server URL:
//
//    server/scripts/seed-album-uitest.sh
//    TEST_RUNNER_FC_UITEST_SERVER=http://127.0.0.1:8091 xcodebuild test \
//      -only-testing:FamilyConnectUITests/AttachmentViewerSwipeUITests …
//
//  Build SIGNED (no CODE_SIGNING_ALLOWED=NO): the app-group entitlement
//  makes the Keychain refuse an unsigned build's token write, and the
//  login step then fails with "Something went wrong".
//
//  Verified 2026-08-28 on the iPhone 16 simulator: the centre swipe FAILED
//  ("1 of 3") on the viewer before the GestureMask fix and passes after.
//

import XCTest

final class AttachmentViewerSwipeUITests: XCTestCase {

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    @MainActor
    func testSwipeOnThePhotoTurnsThePage() throws {
        guard let server = ProcessInfo.processInfo.environment["FC_UITEST_SERVER"] else {
            throw XCTSkip("set TEST_RUNNER_FC_UITEST_SERVER to a seeded server URL")
        }

        let app = XCUIApplication()
        app.launchArguments = ["--uitest-reset", "-v1.serverURL", server]
        app.launch()

        // Log in as the seeded member (mode defaults to Log In).
        let username = app.textFields["Username"]
        XCTAssertTrue(username.waitForExistence(timeout: 15), "should land on the auth screen")
        username.tap()
        username.typeText("junior")
        let password = app.secureTextFields["Password"]
        password.tap()
        password.typeText("password123")
        let submits = app.buttons.matching(NSPredicate(format: "label == 'Log In'")).allElementsBoundByIndex
        submits.max(by: { $0.frame.minY < $1.frame.minY })?.tap()
        for host in [app, XCUIApplication(bundleIdentifier: "com.apple.springboard")] {
            let notNow = host.buttons["Not Now"]
            if notNow.waitForExistence(timeout: 4) {
                notNow.tap()
                break
            }
        }

        // Into the family chat, and onto the album bubble.
        dismissSavePasswordIfPresent(app)
        let row = app.collectionViews.staticTexts["The Smiths"].firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 15), "family chat should be listed")
        if !row.isHittable { dismissSavePasswordIfPresent(app) }
        row.tap()
        sleep(2)
        let album = app.descendants(matching: .any)
            .matching(NSPredicate(format: "label BEGINSWITH 'Album'")).firstMatch
        XCTAssertTrue(album.waitForExistence(timeout: 10), "the album bubble should be on screen")
        album.tap()

        // The "N of 3" label, found by its text so the check reads the
        // same against a build with or without the identifier.
        let page = app.staticTexts.matching(NSPredicate(format: "label MATCHES %@", "^[0-9]+ of 3$")).firstMatch
        XCTAssertTrue(page.waitForExistence(timeout: 10), "the viewer should open with its page label")
        XCTAssertEqual(page.label, "1 of 3")
        // Let the full-size photo land (a blurred preview shows first).
        sleep(2)
        add(screenshot(named: "viewer-open", of: app))

        // 1. A swipe at the CENTRE of the screen — on the photo — pages.
        swipeLeftAtCentre(app)
        XCTAssertTrue(waitForLabel(page, "2 of 3"), "[centre swipe] a swipe on the photo must turn the page (got \(page.label))")
        add(screenshot(named: "after-centre-swipe", of: app))

        // 2. Zoomed in, a drag pans and does NOT page.
        centre(app).doubleTap()
        sleep(1)
        dragLeftAtCentre(app)
        sleep(1)
        XCTAssertEqual(page.label, "2 of 3", "[zoomed drag] a pan must not turn the page")
        add(screenshot(named: "zoomed-drag", of: app))

        // 3. Zoomed back out, the swipe pages again.
        centre(app).doubleTap()
        sleep(1)
        swipeLeftAtCentre(app)
        XCTAssertTrue(waitForLabel(page, "3 of 3"), "[after zoom out] the swipe must page again (got \(page.label))")
        add(screenshot(named: "after-zoom-out-swipe", of: app))

        // 4. And back the other way, still from the centre.
        swipeRightAtCentre(app)
        XCTAssertTrue(waitForLabel(page, "2 of 3"), "[centre swipe back] (got \(page.label))")
    }

    // MARK: - Gestures at the centre of the screen

    private func centre(_ app: XCUIApplication) -> XCUICoordinate {
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
    }

    /// A quick flick from the centre to the left third: what a thumb does
    /// to turn a page.
    private func swipeLeftAtCentre(_ app: XCUIApplication) {
        centre(app).press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.1, dy: 0.5)),
                          withVelocity: .fast, thenHoldForDuration: 0)
    }

    private func swipeRightAtCentre(_ app: XCUIApplication) {
        centre(app).press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.9, dy: 0.5)),
                          withVelocity: .fast, thenHoldForDuration: 0)
    }

    /// A slower, held drag: a pan across a zoomed photo.
    private func dragLeftAtCentre(_ app: XCUIApplication) {
        centre(app).press(forDuration: 0.2, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.15, dy: 0.5)),
                          withVelocity: .slow, thenHoldForDuration: 0.2)
    }

    private func waitForLabel(_ element: XCUIElement, _ label: String, timeout: TimeInterval = 4) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if element.label == label { return true }
            usleep(200_000)
        }
        return element.label == label
    }

    @MainActor
    private func dismissSavePasswordIfPresent(_ app: XCUIApplication) {
        for host in [app, XCUIApplication(bundleIdentifier: "com.apple.springboard")] {
            let notNow = host.buttons["Not Now"]
            if notNow.exists {
                notNow.tap()
                return
            }
        }
    }

    private func screenshot(named name: String, of app: XCUIApplication) -> XCTAttachment {
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        return attachment
    }
}
