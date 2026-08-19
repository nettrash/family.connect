//
//  FamilyConnectUITests.swift
//  FamilyConnectUITests
//
//  Launch smoke: a fresh install (the --uitest-reset argument wipes
//  defaults + keychain in the app's init) must land on the server setup
//  screen — the first stop of the phase machine.
//

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
