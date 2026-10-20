//
//  EmojiKeyboardUITests.swift
//  FamilyConnectUITests
//
//  Emoji as you type (#101) through the ON-SCREEN keyboard, key by key.
//
//  EmojiTypingTests drives the field with `insertText`, and passed for a
//  build in which no phone ever saw an emoji: a field that the real
//  keyboard is typing into ignores a value stored in its binding from
//  outside, while one written by `insertText` does not (2026-10-20). Only
//  real keys tell the two apart, so this taps them.
//
//  Needs a running server with user "junior" in a family whose chat is
//  "The Smiths" (server/scripts/seed-scroll-uitest.sh seeds it), and the
//  session handed in, so it skips unless the runner passes both:
//
//    TEST_RUNNER_FC_UITEST_SERVER=http://127.0.0.1:8091 \
//    TEST_RUNNER_FC_UITEST_TOKEN=<junior's token> xcodebuild test … \
//      -only-testing:FamilyConnectUITests/EmojiKeyboardUITests
//
//  A simulator with Simulator.app's hardware keyboard connected shows no
//  on-screen keyboard; the test says so rather than typing elsewhere.
//

import XCTest

final class EmojiKeyboardUITests: XCTestCase {

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    @MainActor
    func testTheComposerTurnsTypedEmoticonsAndNamesIntoEmoji() throws {
        guard let server = ProcessInfo.processInfo.environment["FC_UITEST_SERVER"],
              let token = ProcessInfo.processInfo.environment["FC_UITEST_TOKEN"]
        else {
            throw XCTSkip("set TEST_RUNNER_FC_UITEST_SERVER and TEST_RUNNER_FC_UITEST_TOKEN")
        }

        let app = XCUIApplication()
        app.launchArguments = ["-v1.serverURL", server, "-v1.sessionToken", token]
        app.launch()
        // The notification prompt belongs to SpringBoard and eats taps.
        for host in [app, XCUIApplication(bundleIdentifier: "com.apple.springboard")] {
            let allow = host.buttons["Allow"]
            if allow.waitForExistence(timeout: 6) {
                allow.tap()
                break
            }
        }

        let row = app.collectionViews.staticTexts["The Smiths"].firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 15), "family chat should be listed")
        row.tap()
        // A multi-line SwiftUI field is a text view to UIKit but may read
        // as a text field to the accessibility snapshot; either will do.
        let composer = app.descendants(matching: .any).matching(NSPredicate(
            format: "placeholderValue == 'Message' AND (elementType == %d OR elementType == %d)",
            XCUIElement.ElementType.textView.rawValue, XCUIElement.ElementType.textField.rawValue)).firstMatch
        XCTAssertTrue(composer.waitForExistence(timeout: 10), "the message field")
        composer.tap()
        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.waitForExistence(timeout: 5), "no on-screen keyboard (hardware keyboard connected?)")

        // The field's label turns into its value once it holds text, so it
        // is found by focus from here on.
        let field = app.descendants(matching: .any)
            .matching(NSPredicate(format: "hasKeyboardFocus == true")).firstMatch
        func type(_ keys: [String]) {
            for key in keys { tap(key, on: keyboard) }
        }
        func expect(_ text: String, _ step: String, line: UInt = #line) {
            let shown = NSPredicate(format: "value == %@", text)
            let met = XCTNSPredicateExpectation(predicate: shown, object: field)
            XCTAssertEqual(XCTWaiter.wait(for: [met], timeout: 3), .completed,
                           "\(step): the field shows \(String(describing: field.value))", line: line)
        }

        type(["H", "i", "space", ":", ")"])
        expect("Hi 😊", "an emoticon")
        type(["delete"])
        expect("Hi :)", "Backspace right behind it takes it back")
        type(["space", ":", "f", "i", "r", "e", ":"])
        expect("Hi :) 🔥", "a name on its closing colon")
    }

    /// Taps one key, turning to the numbers or symbols plane for it if it
    /// is not on the one showing.
    @MainActor
    private func tap(_ label: String, on keyboard: XCUIElement) {
        for _ in 0..<3 {
            let key = keyboard.keys[label]
            if key.exists && key.isHittable {
                key.tap()
                usleep(300_000)
                return
            }
            for plane in ["more", "numbers", "symbols"] {
                let toggle = keyboard.keys[plane]
                if toggle.exists && toggle.isHittable {
                    toggle.tap()
                    usleep(300_000)
                    break
                }
            }
        }
        XCTFail("no key \"\(label)\" on the keyboard")
    }
}
