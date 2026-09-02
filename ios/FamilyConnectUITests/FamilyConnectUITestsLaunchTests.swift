//
//  FamilyConnectUITestsLaunchTests.swift
//  FamilyConnectUITests
//
//  Standard launch screenshot capture.
//
//  iOS ONLY, since FamilyConnectUITests gained macosx (#31), and the
//  reason is privacy rather than platform. `app.screenshot()` on macOS
//  returns THE WHOLE DISPLAY whenever the app has no window of its own —
//  and an app with no window is precisely the launch failure the Mac
//  smoke test exists to catch, so the two coincide. The first macOS run
//  of this file attached a full-desktop photograph (mail, calendar, other
//  people's terminals) to the result bundle, which CI uploads as an
//  artifact on failure. A screenshot with no assertion behind it — this
//  test asserts nothing — is not worth that trade. MacLaunchSmokeUITests
//  covers the Mac launch and deliberately attaches nothing.
//

#if os(iOS)

import XCTest

final class FamilyConnectUITestsLaunchTests: XCTestCase {

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    @MainActor
    func testLaunch() throws {
        let app = XCUIApplication()
        app.launchArguments = ["--uitest-reset"]
        app.launch()

        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = "Launch Screen"
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}

#endif
