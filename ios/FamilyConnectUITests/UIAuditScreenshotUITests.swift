//
//  UIAuditScreenshotUITests.swift
//  FamilyConnectUITests
//
//  A UX/UI AUDIT walk, not a regression test: logs into the seeded
//  fixture (server/scripts/seed-store-screenshots.sh) and photographs
//  every screen the app has, in portrait and landscape, so a reviewer
//  can look at the whole product on one device class in one sitting.
//  Nothing here asserts a layout; the pictures are the deliverable.
//
//    cd ios && DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer \
//      TEST_RUNNER_FC_UITEST_SERVER=http://127.0.0.1:8091 \
//      xcodebuild test-without-building -project FamilyConnect.xcodeproj \
//        -scheme FamilyConnect -destination 'id=<UDID>' \
//        -only-testing:FamilyConnectUITests/UIAuditScreenshotUITests \
//        -resultBundlePath /tmp/audit.xcresult
//    ios/scripts/export-screenshots.sh /tmp/audit.xcresult /tmp/audit
//
//  Every capture is named NN-… so export-screenshots.sh keeps it.
//  Soft everywhere: a screen that cannot be reached is logged and
//  skipped, never a failure that hides the rest of the set.
//

// iOS only: the walk rotates the device and the Mac has no orientation.
#if os(iOS)

import XCTest

final class UIAuditScreenshotUITests: XCTestCase {

    private let familyName = ProcessInfo.processInfo.environment["FC_AUDIT_FAMILY"] ?? "The Harpers"
    private let username = ProcessInfo.processInfo.environment["FC_AUDIT_USER"] ?? "nora"
    private let password = "password123"

    override func setUpWithError() throws {
        continueAfterFailure = true
    }

    @MainActor
    func testAuditWalk() throws {
        guard let server = ProcessInfo.processInfo.environment["FC_UITEST_SERVER"] else {
            throw XCTSkip("set TEST_RUNNER_FC_UITEST_SERVER to a seeded server URL")
        }
        let app = XCUIApplication()
        XCUIDevice.shared.orientation = .portrait

        // 1 — before login: server setup is skipped by the launch
        //     argument, so the first screen is the auth form.
        launch(app, server: server, reset: true)
        _ = app.textFields["Username"].waitForExistence(timeout: 20)
        sleep(1)
        capture("10-auth-portrait")
        rotate(.landscapeLeft); capture("11-auth-landscape"); rotate(.portrait)
        // Sign-up mode of the same form.
        if tapFirst(app, ["Sign Up", "Create Account", "Join the Family"]) {
            sleep(1); capture("12-signup-portrait")
            _ = tapFirst(app, ["Log In"])
        }

        logIn(app)
        relaunch(app, server: server)

        // 2 — the chat list (sidebar) with nothing selected.
        _ = familyRow(app).waitForExistence(timeout: 20)
        sleep(2)
        capture("20-chats-portrait")
        rotate(.landscapeLeft); sleep(1); capture("21-chats-landscape")

        // 3 — the family thread, landscape then portrait.
        if familyRow(app).exists { familyRow(app).tap() }
        _ = composer(app).waitForExistence(timeout: 20)
        sleep(3)
        capture("22-family-chat-landscape")
        rotate(.portrait); sleep(2)
        capture("23-family-chat-portrait")
        // The sidebar over the thread, the way a portrait iPad reveals it.
        // iPad only: on a phone "Chats" is the Back button and would pop
        // the thread, and every step after this one would then miss.
        if UIDevice.current.userInterfaceIdiom == .pad,
           tapFirst(app, ["Show Sidebar", "ToggleSidebar", "Back", "Chats"]) {
            sleep(1); capture("24-family-chat-portrait-sidebar")
            // Close it again by tapping the thread.
            app.coordinate(withNormalizedOffset: CGVector(dx: 0.8, dy: 0.5)).tap()
            sleep(1)
        }
        // Scroll to the start of the thread for the older bubbles.
        for _ in 0..<3 { drag(app, from: 0.3, to: 0.8) }
        sleep(2)
        capture("25-family-chat-portrait-top")
        for _ in 0..<3 { drag(app, from: 0.8, to: 0.3) }
        sleep(1)

        // 4 — a bubble's long-press menu and the reaction picker.
        let bubble = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH 'Wouldn'")).firstMatch
        let anyBubble = bubble.exists ? bubble : app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH 'We''re by the ducks'")).firstMatch
        if anyBubble.waitForExistence(timeout: 5) {
            anyBubble.press(forDuration: 1.0)
            sleep(1)
            capture("26-bubble-menu-portrait")
            relaunch(app, server: server)
            _ = familyRow(app).waitForExistence(timeout: 20)
            familyRow(app).tap()
            _ = composer(app).waitForExistence(timeout: 20)
            sleep(2)
        }

        // 5 — the composer with text, and the attach menu.
        let field = app.textViews.firstMatch
        if field.waitForExistence(timeout: 5) {
            field.tap()
            field.typeText("A draft nobody sends, long enough to wrap onto a second line on a narrow column")
            sleep(1)
            capture("27-composer-typing-portrait")
            rotate(.landscapeLeft); sleep(1); capture("28-composer-typing-landscape"); rotate(.portrait)
            // Reply: long-press then Reply.
            if anyBubble.exists {
                anyBubble.press(forDuration: 1.0)
                if tapFirst(app, ["Reply"]) {
                    sleep(1); capture("29-composer-reply-portrait")
                }
            }
        }
        relaunch(app, server: server)
        _ = familyRow(app).waitForExistence(timeout: 20)
        familyRow(app).tap()
        _ = composer(app).waitForExistence(timeout: 20)
        sleep(2)
        composer(app).tap()
        sleep(1)
        capture("30-attach-menu-portrait")
        relaunch(app, server: server)
        _ = familyRow(app).waitForExistence(timeout: 20)
        familyRow(app).tap()
        _ = composer(app).waitForExistence(timeout: 20)
        sleep(2)

        // 6 — the album viewer.
        let album = app.descendants(matching: .any)
            .matching(NSPredicate(format: "label BEGINSWITH 'Album'")).firstMatch
        if album.waitForExistence(timeout: 10) {
            album.tap()
            sleep(3)
            capture("31-viewer-portrait")
            rotate(.landscapeLeft); sleep(1); capture("32-viewer-landscape"); rotate(.portrait)
            _ = tapFirst(app, ["Close"])
            sleep(1)
        }

        // 7 — the direct chat.
        relaunch(app, server: server)
        _ = familyRow(app).waitForExistence(timeout: 20)
        let ellie = app.collectionViews.staticTexts["Ellie"].firstMatch
        if ellie.waitForExistence(timeout: 5) {
            ellie.tap()
            _ = composer(app).waitForExistence(timeout: 20)
            sleep(3)
            capture("33-direct-chat-portrait")
            rotate(.landscapeLeft); sleep(1); capture("34-direct-chat-landscape"); rotate(.portrait)
        }

        // 8 — New Chat.
        relaunch(app, server: server)
        _ = familyRow(app).waitForExistence(timeout: 20)
        if tapFirst(app, ["New Chat"]) {
            sleep(2); capture("35-new-chat-portrait")
            rotate(.landscapeLeft); sleep(1); capture("36-new-chat-landscape"); rotate(.portrait)
        }

        // 9 — the board and a note editor.
        relaunch(app, server: server)
        _ = familyRow(app).waitForExistence(timeout: 20)
        if tapFirst(app, ["Board"]) {
            _ = app.staticTexts["Bins go out Tuesday"].waitForExistence(timeout: 10)
            sleep(2); capture("40-board-portrait")
            rotate(.landscapeLeft); sleep(1); capture("41-board-landscape"); rotate(.portrait)
            sleep(1)
            let note = app.staticTexts["Bins go out Tuesday"].firstMatch
            if note.exists {
                note.tap()
                sleep(2)
                capture("42-note-editor-portrait")
            }
            // A new note.
            relaunch(app, server: server)
            _ = familyRow(app).waitForExistence(timeout: 20)
            if tapFirst(app, ["Board"]) {
                _ = app.staticTexts["Bins go out Tuesday"].waitForExistence(timeout: 10)
                if tapFirst(app, ["New Note", "Add Note", "Add"]) {
                    sleep(2); capture("43-new-note-portrait")
                }
            }
        }

        // 10 — Settings and everything under it.
        relaunch(app, server: server)
        _ = familyRow(app).waitForExistence(timeout: 20)
        if openSettings(app) {
            sleep(2); capture("50-settings-portrait")
            rotate(.landscapeLeft); sleep(1); capture("51-settings-landscape"); rotate(.portrait)
            sleep(1)
            for _ in 0..<3 { drag(app, from: 0.62, to: 0.34) }
            sleep(1); capture("52-settings-bottom-portrait")
        }

        // Manage Family + Reports + Assistant.
        relaunch(app, server: server)
        _ = familyRow(app).waitForExistence(timeout: 20)
        if openSettings(app) {
            var roster = rosterRow(app)
            for _ in 0..<4 where !roster.exists {
                drag(app, from: 0.62, to: 0.34); sleep(1); roster = rosterRow(app)
            }
            if roster.waitForExistence(timeout: 5) {
                roster.tap()
                sleep(3)
                capture("53-family-portrait")
                rotate(.landscapeLeft); sleep(1); capture("54-family-landscape"); rotate(.portrait)
                for _ in 0..<3 { drag(app, from: 0.62, to: 0.34) }
                sleep(1); capture("55-family-bottom-portrait")
                if tapFirst(app, ["Reports"]) {
                    sleep(2); capture("56-reports-portrait")
                    back(app)
                }
                if tapFirst(app, ["Assistant", "Family Assistant"]) {
                    sleep(2); capture("57-assistant-settings-portrait")
                    back(app)
                }
                // A member row's menu.
                let member = app.staticTexts["Dan"].firstMatch
                if member.exists {
                    member.tap(); sleep(1); capture("58-member-menu-portrait")
                }
            }
        }

        // Statistics.
        relaunch(app, server: server)
        _ = familyRow(app).waitForExistence(timeout: 20)
        if openSettings(app) {
            var stats = app.buttons["Statistics"].firstMatch
            for _ in 0..<4 where !stats.exists {
                drag(app, from: 0.62, to: 0.34); sleep(1); stats = app.buttons["Statistics"].firstMatch
            }
            if stats.waitForExistence(timeout: 5) {
                stats.tap(); sleep(3); capture("60-statistics-portrait")
                rotate(.landscapeLeft); sleep(1); capture("61-statistics-landscape"); rotate(.portrait)
            }
        }

        // Change password / birthday / delete account.
        relaunch(app, server: server)
        _ = familyRow(app).waitForExistence(timeout: 20)
        if openSettings(app) {
            if tapFirst(app, ["Change Password"]) {
                sleep(2); capture("62-change-password-portrait")
                _ = tapFirst(app, ["Cancel"])
            }
            let birthday = app.buttons.matching(NSPredicate(format: "label CONTAINS[c] 'birthday'")).firstMatch
            if birthday.waitForExistence(timeout: 3) {
                birthday.tap(); sleep(2); capture("63-birthday-portrait")
                _ = tapFirst(app, ["Cancel"])
            }
        }
        relaunch(app, server: server)
        _ = familyRow(app).waitForExistence(timeout: 20)
        if openSettings(app) {
            var del = app.buttons["Delete Account"].firstMatch
            for _ in 0..<5 where !del.exists {
                drag(app, from: 0.62, to: 0.34); sleep(1); del = app.buttons["Delete Account"].firstMatch
            }
            if del.waitForExistence(timeout: 5) {
                del.tap(); sleep(2); capture("64-delete-account-portrait")
            }
        }

        // 11 — the assistant's private thread, if the server has one.
        relaunch(app, server: server)
        _ = familyRow(app).waitForExistence(timeout: 20)
        let assistant = app.collectionViews.staticTexts.matching(NSPredicate(format: "label CONTAINS[c] 'assistant'")).firstMatch
        if assistant.waitForExistence(timeout: 3) {
            assistant.tap(); sleep(3); capture("70-assistant-thread-portrait")
        }

        // 12 — logged out: the family gate is not reachable from a joined
        //      account, so the walk ends on the auth screen after a reset.
        launch(app, server: server, reset: true)
        _ = app.textFields["Username"].waitForExistence(timeout: 20)
        // The server-setup screen: clear the address by launching with
        // a reset and no server argument.
        app.terminate()
        app.launchArguments = ["--uitest-reset", "-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launch()
        sleep(2)
        capture("80-server-setup-portrait")
        rotate(.landscapeLeft); sleep(1); capture("81-server-setup-landscape"); rotate(.portrait)
    }

    // MARK: - Helpers

    @MainActor
    private func launch(_ app: XCUIApplication, server: String, reset: Bool) {
        app.terminate()
        app.launchArguments = (reset ? ["--uitest-reset"] : []) + [
            "-v1.serverURL", server,
            "-AppleLanguages", "(en)",
            "-AppleLocale", "en_US",
        ]
        app.launch()
    }

    @MainActor
    private func relaunch(_ app: XCUIApplication, server: String) {
        XCUIDevice.shared.orientation = .portrait
        launch(app, server: server, reset: false)
    }

    @MainActor
    private func rotate(_ orientation: UIDeviceOrientation) {
        XCUIDevice.shared.orientation = orientation
        sleep(1)
    }

    @MainActor
    private func logIn(_ app: XCUIApplication) {
        let field = app.textFields["Username"]
        guard field.waitForExistence(timeout: 20) else { return }
        field.tap()
        field.typeText(username)
        let secure = app.secureTextFields["Password"]
        guard secure.waitForExistence(timeout: 5) else { return }
        secure.tap()
        secure.typeText(password)
        sleep(1)
        capture("13-auth-filled-portrait")
        let submits = app.buttons.matching(NSPredicate(format: "label == 'Log In'")).allElementsBoundByIndex
        submits.max(by: { $0.frame.minY < $1.frame.minY })?.tap()
        for host in [app, XCUIApplication(bundleIdentifier: "com.apple.springboard")] {
            let notNow = host.buttons["Not Now"]
            if notNow.waitForExistence(timeout: 4) { notNow.tap(); break }
        }
        _ = familyRow(app).waitForExistence(timeout: 20)
    }

    @MainActor
    private func openSettings(_ app: XCUIApplication) -> Bool {
        func isOpen() -> Bool {
            app.navigationBars["Settings"].firstMatch.exists || app.staticTexts["Privacy"].firstMatch.exists
        }
        for _ in 0..<3 {
            if isOpen() { return true }
            sleep(2)
            let button = app.buttons["Settings"].firstMatch
            guard button.waitForExistence(timeout: 5) else { continue }
            button.tap()
            for _ in 0..<6 {
                if isOpen() { return true }
                usleep(500_000)
            }
        }
        return false
    }

    @MainActor
    private func rosterRow(_ app: XCUIApplication) -> XCUIElement {
        let owner = app.buttons["Manage Family"].firstMatch
        return owner.exists ? owner : app.buttons["Family Members"].firstMatch
    }

    @MainActor
    private func familyRow(_ app: XCUIApplication) -> XCUIElement {
        app.collectionViews.staticTexts[familyName].firstMatch
    }

    @MainActor
    private func composer(_ app: XCUIApplication) -> XCUIElement {
        app.buttons["Attach a photo, video or file"].firstMatch
    }

    @MainActor
    private func back(_ app: XCUIApplication) {
        let back = app.navigationBars.buttons.firstMatch
        if back.exists { back.tap() }
        sleep(1)
    }

    @MainActor
    private func drag(_ app: XCUIApplication, from: CGFloat, to: CGFloat) {
        let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: from))
        let end = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: to))
        start.press(forDuration: 0.05, thenDragTo: end)
    }

    @MainActor
    private func tapFirst(_ app: XCUIApplication, _ labels: [String]) -> Bool {
        for label in labels {
            for element in [app.buttons[label].firstMatch, app.staticTexts[label].firstMatch] {
                if element.waitForExistence(timeout: 2), element.isHittable {
                    element.tap()
                    return true
                }
            }
        }
        XCTContext.runActivity(named: "none of \(labels) was reachable") { _ in }
        return false
    }

    @MainActor
    private func capture(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}

#endif
