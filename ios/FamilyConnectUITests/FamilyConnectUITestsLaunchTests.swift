//
//  FamilyConnectUITestsLaunchTests.swift
//  FamilyConnectUITests
//
//  Standard launch screenshot capture.
//

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
