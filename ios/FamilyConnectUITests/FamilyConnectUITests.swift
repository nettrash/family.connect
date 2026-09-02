//
//  FamilyConnectUITests.swift
//  FamilyConnectUITests
//
//  Launch smoke: a fresh install (the --uitest-reset argument wipes
//  defaults + keychain in the app's init) must land on the server setup
//  screen — the first stop of the phase machine.
//
//  iOS ONLY, since FamilyConnectUITests gained macosx (#31). The assertion
//  below is a navigation bar, and a Mac does not have one: SwiftUI turns
//  `.navigationTitle` on a NavigationStack in a WindowGroup into the
//  WINDOW's title, so `app.navigationBars["Family Connect"]` matches
//  nothing on that platform however correct the screen is — it fails
//  there in 19s of waiting, which is a red build about nothing. The Mac's
//  equivalent, with the anchors that DO exist there, is
//  MacLaunchSmokeUITests.
//

#if os(iOS)

import XCTest

final class FamilyConnectUITests: XCTestCase {

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    @MainActor
    func testFreshInstallShowsServerSetup() throws {
        let app = XCUIApplication()
        app.launchArguments = ["--uitest-reset"]
        app.launch()

        XCTAssertTrue(
            app.navigationBars["Family Connect"].waitForExistence(timeout: 15),
            "fresh install should land on the server setup screen")
        XCTAssertTrue(app.buttons["Connect"].exists)
    }
}

#endif
